// Output-scope effects of a directed-scene plan (capability `outputFx`, F016).
//
// WHY this module only draws: the core resolved everything that depends on time. A pass of the plan
// (`plan.fx[i]`) already carries its envelope (`weight`) and the `amount` that is `intensity x
// weight`, and the passes are listed in the order of composition. The runtime checks a pass against
// the catalog it implements and draws it over the finished picture of the scene, one fullscreen
// pass each; it knows no timeline, no clip and no parameter default. An effect it does not know is
// refused (typed `unsupported`), never skipped: a picture without its effect is not the plan's.
//
// Color: like every target of this runtime, the pictures are raw sRGB-encoded bytes
// (`NoColorSpace`, `ColorManagement` off). The vignette decodes to linear light, mixes with its
// colour and encodes again, which is what the core's reference (`directed_scene::fx::reference`)
// does; at `amount <= 0` the shader returns the sampled texel untouched.
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
};

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
  if (spec.kind === 'number') return finiteIn(value, pointer, spec.min, spec.max);
  if (!isObject(value)) throw invalid(pointer, 'expected an object');
  if (spec.kind === 'color') {
    return ['r', 'g', 'b'].map((channel) => finiteIn(value[channel], `${pointer}/${channel}`, 0, 1));
  }
  return ['x', 'y'].map((axis) => finiteIn(value[axis], `${pointer}/${axis}`, spec.min, spec.max));
};

/**
 * Reads `plan.fx` (absent means no effects) into the passes to draw, in the plan's order, or
 * throws the typed refusal. `requires` is the plan's own list: a plan with effect passes must
 * ask for `outputFx`, and one that does not draw them is refused before this runs.
 */
export const parseOutputFx = (raw, requires) => {
  if (raw === undefined) return [];
  if (!Array.isArray(raw)) throw invalid('/fx', 'expected an array');
  if (raw.length > 0 && !requires.includes('outputFx')) throw invalid('/requires', 'a plan with effects must require `outputFx`');
  const seen = new Set();
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
    if (!isObject(entry.scope) || entry.scope.kind !== 'output') {
      throw new RuntimeError('unsupported', 'This runtime draws effects of the output scope only', {
        unsupported: 'fx_scope',
        pointer: `${at}/scope`,
      });
    }
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
    return { effect: entry.effect, clipId: entry.clipId, weight, amount, params };
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

vec3 srgbToLinear(vec3 c) {
  return mix(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, step(c, vec3(0.04045)));
}
vec3 linearToSrgb(vec3 c) {
  c = clamp(c, 0.0, 1.0);
  return mix(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, step(c, vec3(0.0031308)));
}

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

const FRAGMENTS = { vignette: VIGNETTE_FRAGMENT };

/**
 * The material of an effect, built once and kept: the program is linked here, never again for the
 * next pass or frame. Its uniforms hold placeholders until `bindPass` writes a pass into them.
 */
export const createFxMaterial = (effect) =>
  new THREE.ShaderMaterial({
    uniforms: {
      tMap: { value: null },
      uAmount: { value: 0 },
      uRadius: { value: 0 },
      uSoftness: { value: 0 },
      uRoundness: { value: 0 },
      uAspect: { value: 1 },
      uCenter: { value: new THREE.Vector2() },
      uColor: { value: new THREE.Vector3() },
    },
    vertexShader: VERTEX,
    fragmentShader: FRAGMENTS[effect],
    depthTest: false,
    depthWrite: false,
    blending: THREE.NoBlending,
    toneMapped: false,
  });

/** Writes one pass over `texture` into the uniforms of its effect's material. `size` is the picture in pixels (the aspect of the ellipse). */
export const bindPass = (material, pass, texture, size) => {
  const { uniforms } = material;
  const { params } = pass;
  uniforms.tMap.value = texture;
  uniforms.uAmount.value = pass.amount;
  uniforms.uRadius.value = params.radius;
  uniforms.uSoftness.value = params.softness;
  uniforms.uRoundness.value = params.roundness;
  uniforms.uAspect.value = size.width / size.height;
  // The plan states the centre with y DOWN (like the rest of the project); the texture's V is up.
  uniforms.uCenter.value.set(params.center[0], 1 - params.center[1]);
  uniforms.uColor.value.set(...params.color);
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
   * Pass `i` draws into `targetOf(i % 2)`: two plain targets (no depth, no MSAA) of the picture's
   * size, ping-pong, so any number of passes needs two and a pass never reads what it writes.
   * `onMaterial(created)` reports each material lookup (the host's counters).
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
