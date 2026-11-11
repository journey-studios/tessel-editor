// Effects of a directed-scene plan: of the output (capability `outputFx`, F016) and of a shot
// (capability `fxShotScope`, F062).
//
// WHY this module only draws: the core resolved everything that depends on time. A pass of the plan
// (`plan.fx[i]`) already carries its envelope (`weight`) and the `amount` that is `intensity x
// weight`, and the passes are listed in the order of composition. The runtime checks a pass against
// the catalog it implements and draws it over the finished picture of the scene, one fullscreen
// pass each; it knows no timeline, no clip and no parameter default. An effect it does not know is
// refused (typed `unsupported`), never skipped: a picture without its effect is not the plan's.
//
// The composition has two STAGES (F062): the passes of a shot (`scope: { kind: 'shot' }`, the *deck*)
// come first, over the picture of the scene, and the passes of the output after them; within a stage
// the order is the plan's. The parser checks that order and the plan keeps it, so the pipeline draws
// the list as it is given; the transitions of a later delivery compose a second deck at the first
// pass of the output. A pass of a shot is only drawn in the frame of that shot: the runtime has no
// other deck to put it on, so another frame's shot is refused (typed `unsupported`) rather than
// drawn on the wrong picture.
//
// Color: like every target of this runtime, the pictures are raw sRGB-encoded bytes
// (`NoColorSpace`, `ColorManagement` off). The vignette and the fade decode to linear light, mix
// with their colour and encode again, which is what the core's reference
// (`directed_scene::fx::reference`) does; when the total weight of a pass is 0 (`amount <= 0`, or
// the fade's own `amount` parameter 0) the shader returns the sampled texel untouched.
//
// The film grain (capability `fxNoise`, F059) is different in kind: it adds noise from the integer
// hash of `(seed, cell)` to the sRGB-ENCODED picture and mixes the result back in linear light. The
// seed arrives resolved (`pass.seed`, per frame, by the core): the runtime never derives a seed,
// never reads a key and never knows a frame, so a seek draws what playback drew.
//
// The checks are pure (no GL, no DOM) so they run under `node --test`.

import * as THREE from 'three';

import { RuntimeError } from './protocol.js';

/** What each effect of the catalog takes: the parameter ids, in the core's catalog order. */
export const OUTPUT_FX_EFFECTS = {
  vignette: {
    params: {
      radius: { kind: 'number', min: 0, max: 1.5 },
      softness: { kind: 'number', min: 0.05, max: 1 },
      intensity: { kind: 'number', min: 0, max: 1 },
      color: { kind: 'color' },
      roundness: { kind: 'number', min: 0, max: 1 },
      center: { kind: 'point', min: 0, max: 1 },
    },
  },
  // The pilot's `fade` look: the picture mixes toward `color` by `amount x intensity x weight`.
  fade: {
    params: {
      color: { kind: 'color' },
      amount: { kind: 'number', min: 0, max: 1 },
      intensity: { kind: 'number', min: 0, max: 1 },
    },
  },
  // The pilot's `filmGrain`: noise on the encoded picture. `noise` marks the effects whose passes
  // carry the `seed` the core resolved for the frame (and that need `fxNoise`).
  filmGrain: {
    noise: true,
    params: {
      amount: { kind: 'number', min: 0, max: 1 },
      size: { kind: 'number', min: 0.5, max: 4 },
      luma: { kind: 'number', min: 0, max: 1 },
      colored: { kind: 'bool' },
      intensity: { kind: 'number', min: 0, max: 1 },
    },
  },
};

/** The largest seed: a whole `uint32`. */
export const MAX_FX_SEED = 4294967295;

/** The height at which a grain `size` is declared in pixels (the pilot's `REFERENCE_HEIGHT`). */
export const GRAIN_REFERENCE_HEIGHT = 720;

/**
 * The side of one grain cell in pixels of a picture `height` pixels tall, applied ONCE:
 * `max(size x height / 720, 1)`. Computed in double precision like the core's reference
 * (`grain_cell_px`) and handed to the shader as one float, so the shader cannot scale it twice.
 */
export const grainCellPx = (size, height) => Math.max(size * (height / GRAIN_REFERENCE_HEIGHT), 1);

const invalid = (pointer, message) =>
  new RuntimeError('invalid_plan', `Scene plan ${pointer || '/'}: ${message}`, { pointer });

const isObject = (value) => typeof value === 'object' && value !== null && !Array.isArray(value);

// WHY own-property checks everywhere a name from a plan indexes a table: `OUTPUT_FX_EFFECTS.constructor`,
// `.toString` and `.__proto__` are not undefined, so a plain lookup took them for effects (and then
// failed with a TypeError instead of the typed refusal) and `'toString' in params` called a method a
// parameter. A plan's names are data, never keys into the prototype chain.
const own = (table, name) => typeof name === 'string' && Object.hasOwn(table, name);

const finiteIn = (value, pointer, min, max) => {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw invalid(pointer, 'expected a finite number');
  if (value < min || value > max) throw invalid(pointer, `expected a number from ${min} to ${max}, got ${value}`);
  return value;
};

const parseParam = (spec, value, pointer) => {
  if (spec.kind === 'bool') {
    if (typeof value !== 'boolean') throw invalid(pointer, 'expected true or false');
    return value;
  }
  if (spec.kind === 'number') return finiteIn(value, pointer, spec.min, spec.max);
  if (!isObject(value)) throw invalid(pointer, 'expected an object');
  if (spec.kind === 'color') {
    return ['r', 'g', 'b'].map((channel) => finiteIn(value[channel], `${pointer}/${channel}`, 0, 1));
  }
  return ['x', 'y'].map((axis) => finiteIn(value[axis], `${pointer}/${axis}`, spec.min, spec.max));
};

/** The stages of the composition, in the order they are drawn. */
const FX_STAGES = ['deck', 'output'];

/**
 * The scope of a pass: where it is drawn. `output` is the finished picture; `shot` is the picture of
 * one shot, which this runtime has only for the shot of the frame it is drawing. Shape first, then
 * what the runtime can do: the capability, the frame's shot, and nothing else (`node` is refused).
 */
const parseScope = (raw, pointer, requires, frame) => {
  if (isObject(raw) && raw.kind === 'output') {
    if (Object.hasOwn(raw, 'shotId')) throw invalid(`${pointer}/shotId`, 'an output scope names no shot');
    return { stage: 'output' };
  }
  if (isObject(raw) && raw.kind === 'shot') {
    if (typeof raw.shotId !== 'string' || raw.shotId === '') throw invalid(`${pointer}/shotId`, 'expected a non-empty string');
    if (!requires.includes('fxShotScope')) throw invalid('/requires', 'a plan with a pass of a shot must require `fxShotScope`');
    const frameShot = isObject(frame) ? frame.shotId : undefined;
    if (typeof frameShot !== 'string' || frameShot === '') {
      throw invalid('/frame/shotId', 'a plan with a pass of a shot names the shot of its frame');
    }
    if (raw.shotId !== frameShot) {
      throw new RuntimeError('unsupported', `This runtime draws the effects of a shot in the frames of that shot only (\`${raw.shotId}\` is not \`${frameShot}\`)`, {
        unsupported: 'fx_scope',
        reason: 'shot_not_in_frame',
        shot: raw.shotId,
        frame_shot: frameShot,
        pointer: `${pointer}/shotId`,
      });
    }
    return { stage: 'deck', shotId: raw.shotId };
  }
  throw new RuntimeError('unsupported', 'This runtime draws effects of the output and of a shot only', {
    unsupported: 'fx_scope',
    reason: 'scope_kind',
    scope: isObject(raw) && typeof raw.kind === 'string' ? raw.kind : null,
    pointer,
  });
};

/**
 * Reads `plan.fx` (absent means no effects) into the passes to draw, in the plan's order, or
 * throws the typed refusal. `requires` is the plan's own list: a plan with effect passes must
 * ask for `outputFx` (and one whose passes carry a seed for `fxNoise`, and one with a pass of a shot
 * for `fxShotScope`), and one that does not draw them is refused before this runs. `frame` is the
 * plan's `frame`: the shot a pass of a shot is compared with. Each pass comes back with its `stage`
 * (`deck` or `output`); the stages must be listed in order, as the core lists them.
 */
export const parseOutputFx = (raw, requires, frame) => {
  if (raw === undefined) return [];
  if (!Array.isArray(raw)) throw invalid('/fx', 'expected an array');
  if (raw.length > 0 && !requires.includes('outputFx')) throw invalid('/requires', 'a plan with effects must require `outputFx`');
  const seen = new Set();
  let lastPosition = 0;
  return raw.map((entry, index) => {
    const at = `/fx/${index}`;
    if (!isObject(entry)) throw invalid(at, 'expected an object');
    if (!own(OUTPUT_FX_EFFECTS, entry.effect)) {
      const named = typeof entry.effect === 'string' ? entry.effect : null;
      throw new RuntimeError('unsupported', `This runtime does not draw the effect ${named === null ? 'of that kind' : `\`${named}\``}`, {
        unsupported: 'fx_effect',
        effect: named,
        pointer: `${at}/effect`,
      });
    }
    const known = OUTPUT_FX_EFFECTS[entry.effect];
    const scope = parseScope(entry.scope, `${at}/scope`, requires, frame);
    const position = FX_STAGES.indexOf(scope.stage);
    if (position < lastPosition) throw invalid(`${at}/scope`, 'the passes of the shot are listed before the passes of the output');
    lastPosition = position;
    if (typeof entry.clipId !== 'string' || entry.clipId === '') throw invalid(`${at}/clipId`, 'expected a non-empty string');
    if (seen.has(entry.clipId)) throw invalid(`${at}/clipId`, `clip \`${entry.clipId}\` appears twice`);
    seen.add(entry.clipId);
    const weight = finiteIn(entry.weight, `${at}/weight`, 0, 1);
    const amount = finiteIn(entry.amount, `${at}/amount`, 0, 1);
    if (!isObject(entry.params)) throw invalid(`${at}/params`, 'expected an object');
    const params = {};
    for (const [name, spec] of Object.entries(known.params)) {
      if (!own(entry.params, name)) throw invalid(`${at}/params/${name}`, 'missing parameter');
      params[name] = parseParam(spec, entry.params[name], `${at}/params/${name}`);
    }
    const extra = Object.keys(entry.params).find((name) => !own(known.params, name));
    if (extra !== undefined) throw invalid(`${at}/params/${extra}`, 'not a parameter of the effect');
    const pass = { effect: entry.effect, clipId: entry.clipId, weight, amount, params, stage: scope.stage };
    if (scope.shotId !== undefined) pass.shotId = scope.shotId;
    if (known.noise === true) {
      if (!requires.includes('fxNoise')) throw invalid('/requires', 'a plan with noise effects must require `fxNoise`');
      const { seed } = entry;
      if (!Number.isInteger(seed) || seed < 0 || seed > MAX_FX_SEED) {
        throw invalid(`${at}/seed`, `expected a whole number from 0 to ${MAX_FX_SEED}`);
      }
      pass.seed = seed;
    } else if (Object.hasOwn(entry, 'seed')) {
      throw invalid(`${at}/seed`, 'only an effect that draws noise carries a seed');
    }
    return pass;
  });
};

const VERTEX = /* glsl */ `
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}`;

// The sRGB curves are the IEC 61966-2-1 pair the core's reference uses, written out (not the
// `pow(c, 2.2)` shortcut), so the two agree to the last bit of an 8-bit picture.
const SRGB_CURVES = /* glsl */ `
vec3 srgbToLinear(vec3 c) {
  return mix(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, step(c, vec3(0.04045)));
}
vec3 linearToSrgb(vec3 c) {
  c = clamp(c, 0.0, 1.0);
  return mix(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, step(c, vec3(0.0031308)));
}`;

const VIGNETTE_FRAGMENT = /* glsl */ `
precision highp float;
uniform sampler2D tMap;
uniform float uAmount;
uniform float uRadius;
uniform float uSoftness;
uniform float uRoundness;
uniform float uAspect;
uniform vec2 uCenter;
uniform vec3 uColor;
varying vec2 vUv;
${SRGB_CURVES}

void main() {
  vec4 source = texture2D(tMap, vUv);
  if (uAmount <= 0.0) {
    gl_FragColor = source;
    return;
  }
  float stretch = mix(1.0, uAspect, uRoundness);
  vec2 shape = vec2(stretch, 1.0);
  vec2 offset = (vUv - uCenter) * 2.0 * shape;
  float distance = length(offset) / max(length(shape), 1.0);
  float shade = smoothstep(uRadius, uRadius + uSoftness, distance);
  vec3 mixed = mix(srgbToLinear(source.rgb), uColor, shade * uAmount);
  gl_FragColor = vec4(linearToSrgb(mixed), source.a);
}`;

// The fade (pilot `shaders/fade.ts`: `mix(source, uColor, uFade * uAmount)`): one number for the
// whole picture. `uAmount` is the pass's `intensity x weight`, `uFade` the `amount` parameter;
// exactly the identity when either is 0.
const FADE_FRAGMENT = /* glsl */ `
precision highp float;
uniform sampler2D tMap;
uniform float uAmount;
uniform float uFade;
uniform vec3 uColor;
varying vec2 vUv;
${SRGB_CURVES}

void main() {
  vec4 source = texture2D(tMap, vUv);
  float weight = uFade * uAmount;
  if (weight <= 0.0) {
    gl_FragColor = source;
    return;
  }
  vec3 mixed = mix(srgbToLinear(source.rgb), uColor, weight);
  gl_FragColor = vec4(linearToSrgb(mixed), source.a);
}`;

// The pilot's `filmGrain` (`shaders/film-grain.ts`): the hashes are `uint` arithmetic only (no
// float hash, no `sin`), bit for bit the core's `fx/hash.rs`. `gl_FragCoord` counts from the
// bottom-left and its pixel centre is `+0.5`, which the core's reference converts explicitly.
const FILM_GRAIN_FRAGMENT = /* glsl */ `
precision highp float;
precision highp int;
uniform sampler2D tMap;
uniform float uAmount;
uniform float uGrain;
uniform float uLuma;
uniform float uColored;
uniform float uCell;
uniform uint uSeed;
varying vec2 vUv;
${SRGB_CURVES}

uint lowbias32(uint x) {
  x ^= x >> 16u;
  x *= 0x7feb352du;
  x ^= x >> 15u;
  x *= 0x846ca68bu;
  x ^= x >> 16u;
  return x;
}
uint hashPixel(uint seed, uint x, uint y) {
  return lowbias32(lowbias32(seed ^ 0x9e3779b9u) + x + (y << 16u));
}
float unitFloat(uint h) {
  return float(h >> 8u) / 16777216.0;
}
float triangular(uint h) {
  return unitFloat(h) + unitFloat(lowbias32(h ^ 0x68bc21ebu)) - 1.0;
}

void main() {
  vec4 source = texture2D(tMap, vUv);
  if (uGrain * uAmount <= 0.0) {
    gl_FragColor = source;
    return;
  }
  uint cellX = uint(floor(gl_FragCoord.x / uCell));
  uint cellY = uint(floor(gl_FragCoord.y / uCell));
  uint h = hashPixel(uSeed, cellX, cellY);
  vec3 noise = vec3(triangular(h));
  if (uColored > 0.5) {
    noise = vec3(triangular(h), triangular(lowbias32(h + 0x632be5abu)), triangular(lowbias32(h + 0x85157af5u)));
  }
  vec3 encoded = clamp(source.rgb, 0.0, 1.0);
  float luma = dot(encoded, vec3(0.2126, 0.7152, 0.0722));
  float response = mix(1.0, 1.0 - smoothstep(0.05, 0.95, luma), uLuma);
  vec3 grained = clamp(encoded + noise * (uGrain * 0.25 * response), 0.0, 1.0);
  vec3 mixed = mix(srgbToLinear(encoded), srgbToLinear(grained), uAmount);
  gl_FragColor = vec4(linearToSrgb(mixed), source.a);
}`;

/**
 * How each effect of the catalog is drawn: its fragment shader, the uniforms it owns (the shared
 * `tMap` and `uAmount` are added for all) and how a pass writes its resolved values into them.
 * A name from a plan reaches this table only after `parseOutputFx` accepted it, and by own-property
 * lookup (`drawOf`).
 */
const EFFECT_DRAWS = {
  vignette: {
    fragment: VIGNETTE_FRAGMENT,
    uniforms: () => ({
      uRadius: { value: 0 },
      uSoftness: { value: 0 },
      uRoundness: { value: 0 },
      uAspect: { value: 1 },
      uCenter: { value: new THREE.Vector2() },
      uColor: { value: new THREE.Vector3() },
    }),
    bind: (uniforms, params, size) => {
      uniforms.uRadius.value = params.radius;
      uniforms.uSoftness.value = params.softness;
      uniforms.uRoundness.value = params.roundness;
      uniforms.uAspect.value = size.width / size.height;
      // The plan states the centre with y DOWN (like the rest of the project); the texture's V is up.
      uniforms.uCenter.value.set(params.center[0], 1 - params.center[1]);
      uniforms.uColor.value.set(...params.color);
    },
  },
  fade: {
    fragment: FADE_FRAGMENT,
    uniforms: () => ({
      uFade: { value: 0 },
      uColor: { value: new THREE.Vector3() },
    }),
    bind: (uniforms, params) => {
      uniforms.uFade.value = params.amount;
      uniforms.uColor.value.set(...params.color);
    },
  },
  filmGrain: {
    fragment: FILM_GRAIN_FRAGMENT,
    uniforms: () => ({
      uGrain: { value: 0 },
      uLuma: { value: 0 },
      uColored: { value: 0 },
      uCell: { value: 1 },
      uSeed: { value: 0 },
    }),
    bind: (uniforms, params, size, pass) => {
      uniforms.uGrain.value = params.amount;
      uniforms.uLuma.value = params.luma;
      uniforms.uColored.value = params.colored ? 1 : 0;
      uniforms.uCell.value = grainCellPx(params.size, size.height);
      uniforms.uSeed.value = pass.seed;
    },
  },
};

const drawOf = (effect) => {
  if (!own(EFFECT_DRAWS, effect)) throw new Error(`No draw is defined for the effect ${String(effect)}`);
  return EFFECT_DRAWS[effect];
};

/**
 * The material of an effect, built once and kept: the program is linked here, never again for the
 * next pass or frame. Its uniforms hold placeholders until `bindPass` writes a pass into them.
 */
export const createFxMaterial = (effect) => {
  const draw = drawOf(effect);
  return new THREE.ShaderMaterial({
    uniforms: { tMap: { value: null }, uAmount: { value: 0 }, ...draw.uniforms() },
    vertexShader: VERTEX,
    fragmentShader: draw.fragment,
    depthTest: false,
    depthWrite: false,
    blending: THREE.NoBlending,
    toneMapped: false,
  });
};

/** Writes one pass over `texture` into the uniforms of its effect's material. `size` is the picture in pixels (the aspect of the ellipse). */
export const bindPass = (material, pass, texture, size) => {
  const { uniforms } = material;
  uniforms.tMap.value = texture;
  uniforms.uAmount.value = pass.amount;
  drawOf(pass.effect).bind(uniforms, pass.params, size, pass);
};

/**
 * What draws the passes of a plan, kept between frames (A1): the fullscreen quad, its scene and
 * camera, and one material per effect of the catalog (so one linked program per effect, however
 * many passes use it, and whatever the frame). Three.js uploads uniforms at each `render`, so one
 * material serves the passes of a frame in turn; what a pass changes lives in the uniforms.
 *
 * Lifetime: the compositor owns it (`Compositor.fx`) and calls `dispose` when the context is lost,
 * when retention is turned off, after a frame that failed in a pass (the next frame builds a new
 * one: nothing half-written survives an error) and when the compositor goes. A host that does not
 * retain builds one per frame and disposes it with the frame. It has no size of its own: the
 * targets it draws into come from the caller, so a resize cannot leave it stale.
 */
export class OutputFxPipeline {
  constructor() {
    this.scene = new THREE.Scene();
    this.camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
    this.geometry = new THREE.PlaneGeometry(2, 2);
    this.quad = new THREE.Mesh(this.geometry);
    this.quad.frustumCulled = false;
    this.scene.add(this.quad);
    this.materials = new Map();
    this.disposed = false;
  }

  /** The kept material of an effect; `created` says whether this call built (and so will link) it. */
  materialOf(effect) {
    let material = this.materials.get(effect);
    const created = material === undefined;
    if (created) {
      material = createFxMaterial(effect);
      this.materials.set(effect, material);
    }
    return { material, created };
  }

  /**
   * Draws `passes` over `source` (a render target holding the finished picture), each pass reading
   * the one before, and returns the texture of the last one (`source.texture` when there are none).
   * The plan lists the passes of the shot before those of the output (the parser checks it), so the
   * list is drawn as it is. Pass `i` draws into `targetOf(i % 2)`: two plain targets (no depth, no
   * MSAA) of the picture's size, ping-pong, so any number of passes needs two and a pass never reads
   * what it writes. `onMaterial(created)` reports each material lookup (the host's counters).
   */
  draw(renderer, source, passes, { targetOf, size, onMaterial = () => {} }) {
    if (this.disposed) throw new Error('The output-fx pipeline was disposed');
    if (passes.length === 0) return source.texture;
    let texture = source.texture;
    try {
      passes.forEach((pass, index) => {
        const target = targetOf(index % 2);
        const { material, created } = this.materialOf(pass.effect);
        onMaterial(created);
        bindPass(material, pass, texture, size);
        this.quad.material = material;
        renderer.setRenderTarget(target);
        // No clear: the quad covers every pixel with NoBlending and writes alpha too.
        renderer.render(this.scene, this.camera);
        texture = target.texture;
      });
    } finally {
      renderer.setRenderTarget(null);
      // Do not keep a reference to a texture of a target the cache may free after this frame.
      for (const material of this.materials.values()) material.uniforms.tMap.value = null;
    }
    return texture;
  }

  dispose() {
    this.disposed = true;
    for (const material of this.materials.values()) material.dispose();
    this.materials.clear();
    this.geometry.dispose();
  }
}
