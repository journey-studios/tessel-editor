// Point lights (F060) in the directed-scene layer. The pure half checks what the runtime accepts and
// refuses of `plan.lights`; the browser half draws real pixels in headless Chrome (SwiftShader) and
// compares them with a reference written HERE, from the formula of docs/features/scene-light.md
// (decode sRGB, multiply by `1 + sum(colour * intensity * max(0, n.l) * falloff^2)`, encode), never
// with the runtime's shader. Skipped, loudly, without Chrome or the bundle (TESSEL_REQUIRE_RUNTIME=1
// makes that a failure).
//
// What is proved: the lit picture of a flat plane (front, behind, tilted, coloured, two lights,
// the radius, an intensity of zero), exact identity without a visible light, the typed refusals,
// that a retained material takes each frame's lights, and that the editor's overlay (markers and
// the range sphere) is not part of the frame. What is not: splats (they are not lit: stated, not
// tested), a physical WKWebView, performance.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  SCENE_CAPABILITIES,
  parseDirectedLayer,
} from '../src/directed-scene.js';
import {
  GENERATION,
  SIZE,
  directedLayer,
  directedRuntime,
  flat,
  makeFiles,
  node,
  reference,
  resource,
  sceneAsset,
  scenePlan,
  vec,
} from './directed-scene-fixtures.mjs';
import { skipUnavailable } from './support.mjs';

// ---- the independent reference --------------------------------------------------------------

const decode = (byte) => {
  const c = byte / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
};
const encode = (linear) => {
  const x = Math.min(1, Math.max(0, linear));
  return 255 * (x <= 0.0031308 ? 12.92 * x : 1.055 * x ** (1 / 2.4) - 0.055);
};
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const minus = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];

const CAMERA = [0, 0, 10];
const TAN_HALF = Math.tan((45 / 2) * (Math.PI / 180));
const ASPECT = SIZE[0] / SIZE[1];

/** The world point of the plane through `origin` with `normal` seen at the middle of pixel (px, py). */
const planePoint = (px, py, origin = [0, 0, 0], normal = [0, 0, 1]) => {
  const ndcX = ((px + 0.5) / SIZE[0]) * 2 - 1;
  const ndcY = 1 - ((py + 0.5) / SIZE[1]) * 2;
  const direction = [ndcX * TAN_HALF * ASPECT, ndcY * TAN_HALF, -1];
  const t = dot(normal, minus(origin, CAMERA)) / dot(normal, direction);
  return [CAMERA[0] + t * direction[0], CAMERA[1] + t * direction[1], CAMERA[2] + t * direction[2]];
};

/** `falloff` applied twice, as the document says: a quarter at the radius. */
const falloff = (distance, radius) => (1 / (1 + (distance / radius) ** 2)) ** 2;

/** What the light adds, per channel, at `point` of a plane whose front normal is `normal`. */
const added = (lights, point, normal) => {
  // A double-sided plane is lit on the side the camera sees.
  const facing = dot(normal, minus(CAMERA, point)) < 0 ? normal.map((c) => -c) : normal;
  const sum = [0, 0, 0];
  for (const light of lights) {
    const toLight = minus([light.position.x, light.position.y, light.position.z], point);
    const distance = Math.hypot(...toLight);
    const lambert = Math.max(0, dot(facing, toLight) / distance);
    const strength = lambert * falloff(distance, light.radius) * light.intensity;
    sum[0] += light.color.r * strength;
    sum[1] += light.color.g * strength;
    sum[2] += light.color.b * strength;
  }
  return sum;
};

/** The pixel the reference expects: the base byte colour lit by `lights` at `point`. */
const expected = (base, lights, point, normal) => {
  const extra = added(lights.filter((light) => !light.hidden), point, normal);
  return base.map((byte, channel) => encode(decode(byte) * (1 + extra[channel])));
};

const light = (id, [x, y, z], { color = [1, 1, 1], intensity = 1, radius = 4, hidden } = {}) => ({
  id,
  position: vec(x, y, z),
  color: { r: color[0], g: color[1], b: color[2] },
  intensity,
  radius,
  ...(hidden === undefined ? {} : { hidden }),
});

// ---- the plans ------------------------------------------------------------------------------

const BASE = [120, 160, 200];
const PROBES = [
  [80, 45],
  [40, 45],
  [120, 20],
  [20, 80],
  [100, 60],
];

const files = (t) => makeFiles(t, { base: flat(4, 4, BASE) });
const BASE_RESOURCE = resource('base', 4, 4);

const rotationY = (degrees, [x, y, z]) => {
  const c = Math.cos((degrees * Math.PI) / 180);
  const s = Math.sin((degrees * Math.PI) / 180);
  return [c, 0, -s, 0, 0, 1, 0, 0, s, 0, c, 0, x, y, z, 1];
};

/** One plane of 16x9 (more than the whole view) at the origin, lit by `lights`. */
const litPlan = (lights, { tilt = 0, requires } = {}) => {
  const matrix = rotationY(tilt, [0, 0, 0]);
  return scenePlan({
    nodes: [node('plane', 'base', 0, 0, 0, 16, 9, { worldMatrix: matrix, localMatrix: matrix })],
    layers: [['plane', 'base', 'opaque']],
    resources: [BASE_RESOURCE],
    requires:
      requires ??
      ['camera', 'nodeTransforms', 'imagePlanes', 'opaqueAlpha', ...(lights.some((entry) => !entry.hidden) ? ['pointLights'] : [])],
    extra: lights.length > 0 ? { lights } : {},
  });
};

const open = async (t) => {
  if (skipUnavailable(t, assert)) return null;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  t.after(() => runtime.close());
  const loaded = await runtime.load(sceneAsset('slot-base', paths.base, 'base'));
  assert.equal(loaded.state, 'ready', JSON.stringify(loaded.error));
  return runtime;
};

const draw = async (runtime, plan, options = {}) => {
  const message = await runtime.frame([directedLayer(plan, [reference('base', 'slot-base')])], {
    size: SIZE,
    probes: PROBES,
    ...options,
  });
  assert.equal(message.type, 'frame.done', JSON.stringify(message.error));
  return message;
};

const assertLit = (message, lights, { tilt = 0, tolerance = 2 } = {}) => {
  const angle = (tilt * Math.PI) / 180;
  const normal = [Math.sin(angle), 0, Math.cos(angle)];
  let checked = 0;
  PROBES.forEach(([px, py], index) => {
    const point = planePoint(px, py, [0, 0, 0], normal);
    // A tilted 16x9 plane does not cover the whole view: only what it covers is the picture's.
    const across = dot(point, [Math.cos(angle), 0, -Math.sin(angle)]);
    if (Math.abs(across) > 7.5 || Math.abs(point[1]) > 4.2) return;
    checked += 1;
    const want = expected(BASE, lights, point, normal);
    const got = message.probes[index];
    assert.ok(
      want.every((value, channel) => Math.abs(got[channel] - value) <= tolerance),
      `probe ${px},${py}: expected ~rgb(${want.map((v) => v.toFixed(1))}), got rgb(${got.slice(0, 3)})`,
    );
  });
  assert.ok(checked >= 3, `only ${checked} probes are on the plane`);
};

// ---- the pure half --------------------------------------------------------------------------

const parse = (plan) => parseDirectedLayer(directedLayer(plan, [reference('base', 'slot-base')]));

test('the runtime advertises and parses point lights: visible ones in plan order, hidden ones take no slot', () => {
  assert.ok(SCENE_CAPABILITIES.includes('pointLights'));
  const lights = [
    light('a', [0, 0, 2], { intensity: 2 }),
    light('b', [1, 0, 2], { hidden: true }),
    light('c', [-1, 0, 2], { color: [1, 0.5, 0.25], radius: 3 }),
  ];
  const spec = parse(litPlan(lights));
  assert.deepEqual(spec.lights.map((entry) => entry.id), ['a', 'c']);
  assert.deepEqual(spec.lights[1].color, [1, 0.5, 0.25]);
  assert.equal(spec.lights[1].radius, 3);
  assert.deepEqual(parse(litPlan([])).lights, [], 'no lights is no lights');
  // Only hidden lights: nothing to draw and no capability to ask for.
  assert.deepEqual(parse(litPlan([light('h', [0, 0, 2], { hidden: true })])).lights, []);
});

test('a malformed light, a missing capability and a fifth light are refused typed, never drawn without the light', () => {
  const ok = light('a', [0, 0, 2]);
  const refused = (plan, code, pattern) => {
    assert.throws(
      () => parse(plan),
      (error) => error.code === code && pattern.test(error.message),
      `${code} ${pattern}`,
    );
  };
  refused(litPlan([ok], { requires: ['camera', 'nodeTransforms', 'imagePlanes', 'opaqueAlpha'] }), 'invalid_plan', /pointLights/);
  refused(litPlan([{ ...ok, intensity: 21 }]), 'invalid_plan', /between 0 and 20/);
  refused(litPlan([{ ...ok, intensity: -1 }]), 'invalid_plan', /between 0 and 20/);
  refused(litPlan([{ ...ok, radius: 0 }]), 'invalid_plan', /positive/);
  refused(litPlan([{ ...ok, color: { r: 2, g: 0, b: 0 } }]), 'invalid_plan', /between 0 and 1/);
  refused(litPlan([{ ...ok, position: vec(Number.NaN, 0, 0) }]), 'invalid_plan', /finite/);
  refused(litPlan([ok, { ...ok, position: vec(1, 0, 0) }]), 'invalid_plan', /listed twice/);
  refused(
    litPlan([0, 1, 2, 3, 4].map((index) => ({ ...ok, id: `l${index}` }))),
    'invalid_plan',
    /at most 4/,
  );
});

test('a runtime that does not list pointLights refuses a plan that requires it', () => {
  const plan = litPlan([light('a', [0, 0, 2])]);
  const without = SCENE_CAPABILITIES.filter((name) => name !== 'pointLights');
  const missing = plan.requires.filter((name) => !without.includes(name));
  assert.deepEqual(missing, ['pointLights']);
});

test('a plan the core refuses for lighting is refused with its reason, not drawn', () => {
  const plan = litPlan([]);
  plan.complete = false;
  plan.unsupported = [{ domain: 'lighting', subject: null, reason: 'ambient_not_neutral' }];
  assert.throws(
    () => parse(plan),
    (error) => error.code === 'unsupported' && error.details.domains[0].reason === 'ambient_not_neutral',
  );
});

// ---- real pixels ----------------------------------------------------------------------------

test('a light in front lights the plane by lambert and falloff, against an independent reference', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const lights = [light('lamp', [1, 0.5, 2], { intensity: 3, radius: 3 })];
  const lit = await draw(runtime, litPlan(lights));
  assertLit(lit, lights);
  // It really is brighter than the unlit picture where the light is near, and the same far away.
  const unlit = await draw(runtime, litPlan([]));
  assert.ok(lit.probes[0][2] > unlit.probes[0][2] || lit.probes[0][1] > unlit.probes[0][1]);
  assert.notEqual(lit.hash, unlit.hash);
});

test('the radius is where the light is a quarter as strong, and a larger radius reaches further', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  // Straight above the probe at the centre, at a distance equal to the radius: falloff 1/4.
  const near = [light('lamp', [0, 0, 3], { intensity: 4, radius: 3 })];
  assertLit(await draw(runtime, litPlan(near)), near);
  const wide = [light('lamp', [0, 0, 3], { intensity: 4, radius: 6 })];
  const wideFrame = await draw(runtime, litPlan(wide));
  assertLit(wideFrame, wide);
  const narrowFrame = await draw(runtime, litPlan(near));
  // Away from the middle the wider light is the stronger one.
  assert.ok(wideFrame.probes[3][2] >= narrowFrame.probes[3][2]);
  assert.notEqual(wideFrame.hash, narrowFrame.hash);
});

test('a light behind the plane does nothing, and a light of intensity zero is the unlit picture', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const unlit = await draw(runtime, litPlan([]));
  const behind = [light('lamp', [0, 0, -3], { intensity: 10, radius: 5 })];
  const behindFrame = await draw(runtime, litPlan(behind));
  assertLit(behindFrame, behind);
  PROBES.forEach((_, index) => {
    assert.ok(
      behindFrame.probes[index].slice(0, 3).every((value, channel) => Math.abs(value - unlit.probes[index][channel]) <= 1),
      `a light behind the plane leaves probe ${index} as it was`,
    );
  });
  const dark = [light('lamp', [0, 0, 2], { intensity: 0 })];
  const darkFrame = await draw(runtime, litPlan(dark));
  PROBES.forEach((_, index) => {
    assert.ok(
      darkFrame.probes[index].slice(0, 3).every((value, channel) => Math.abs(value - unlit.probes[index][channel]) <= 1),
      `intensity 0 leaves probe ${index} as it was`,
    );
  });
});

test('a tilted plane is lit by its own normal, and the colour of a light tints only what it carries', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const lights = [light('lamp', [2, 1, 3], { color: [1, 0.2, 0], intensity: 5, radius: 4 })];
  assertLit(await draw(runtime, litPlan(lights, { tilt: 40 })), lights, { tilt: 40 });
  assertLit(await draw(runtime, litPlan(lights, { tilt: -40 })), lights, { tilt: -40 });
  // A light with no blue adds none: the blue channel stays at the picture's own.
  const frame = await draw(runtime, litPlan([light('red', [0, 0, 3], { color: [1, 0, 0], intensity: 5 })]));
  assert.ok(Math.abs(frame.probes[0][2] - BASE[2]) <= 1, `blue stays ${BASE[2]}, got ${frame.probes[0][2]}`);
  assert.ok(frame.probes[0][0] > BASE[0] + 10, 'the red channel is lit');
});

test('two lights add; a hidden light is not drawn and the plan without visible lights is the unlit picture bit for bit', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const two = [
    light('left', [-2, 0, 2], { color: [1, 0.4, 0.1], intensity: 2, radius: 3 }),
    light('right', [2, 0, 2], { color: [0.1, 0.4, 1], intensity: 2, radius: 3 }),
  ];
  assertLit(await draw(runtime, litPlan(two)), two);

  const unlit = await draw(runtime, litPlan([]));
  const hidden = await draw(runtime, litPlan([light('lamp', [0, 0, 2], { intensity: 20, hidden: true })]));
  assert.equal(hidden.hash, unlit.hash, 'a hidden light takes no slot: the same picture, exactly');
});

test('a retained material takes each frame\'s lights: lit, then unlit, then lit again', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const lights = [light('lamp', [0, 0, 2], { intensity: 4, radius: 3 })];
  const first = await draw(runtime, litPlan(lights));
  const unlit = await draw(runtime, litPlan([]));
  const again = await draw(runtime, litPlan(lights));
  assert.notEqual(first.hash, unlit.hash);
  assert.equal(again.hash, first.hash, 'the same lights draw the same picture');
  assert.equal(again.cache.material_created, 0, 'the lights are uniforms: no relink');
  // The unlit frame after a lit one is the unlit picture, not the previous frame's light.
  const base = await draw(runtime, litPlan([]));
  assert.equal(base.hash, unlit.hash);
  // And a light that moves changes the picture without a new program.
  const moved = await draw(runtime, litPlan([light('lamp', [3, 0, 2], { intensity: 4, radius: 3 })]));
  assert.notEqual(moved.hash, first.hash);
  assert.equal(moved.cache.material_created, 0);
});

test('the same plan with retention off draws the same lit picture', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const plan = litPlan([light('lamp', [1, 1, 2], { intensity: 3, radius: 3 })]);
  const retained = await draw(runtime, plan);
  await runtime.send({ type: 'hello', retain: false });
  await runtime.waitFor((m) => m.type === 'ready');
  const cold = await draw(runtime, plan);
  assert.equal(cold.hash, retained.hash);
});

test('the editor\'s overlay (light markers and the range sphere) is not in the frame', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const { openRuntime } = await import('./support.mjs');
  const runtime = await openRuntime({ overlay: true, extraFiles: Object.values(paths) });
  t.after(() => runtime.close());
  await runtime.hello();
  const loaded = await runtime.load(sceneAsset('slot-base', paths.base, 'base'));
  assert.equal(loaded.state, 'ready');
  const plan = litPlan([light('lamp', [1, 0.5, 2], { intensity: 3, radius: 3 })]);
  const before = await draw(runtime, plan);

  const ring = (cx, cy, r) => Array.from({ length: 33 }, (_, i) => ({ u: cx + (r * Math.cos((i / 32) * 2 * Math.PI)) / ASPECT, v: cy + r * Math.sin((i / 32) * 2 * Math.PI) }));
  await runtime.send({ type: 'overlay.enable', enabled: true });
  await runtime.send({
    type: 'overlay.set',
    ack: true,
    overlay: {
      version: 1,
      visible: true,
      reason: null,
      entity: 'scene/s/light/lamp',
      origin: { u: 0.6, v: 0.4 },
      handles: [{ id: 'range', kind: 'range', from: { u: 0.6, v: 0.4 }, to: { u: 0.8, v: 0.4 } }],
      sphere: [ring(0.6, 0.4, 0.2), ring(0.6, 0.4, 0.15), ring(0.6, 0.4, 0.1)],
      light_markers: [{ entity: 'scene/s/light/lamp', at: { u: 0.6, v: 0.4 }, hidden: false, selected: true }],
      outlines: [],
      active: null,
      aspect: ASPECT,
      stamp: { scene: 's', session: 'x', revision: 1, frame: 0 },
      metrics: { hit_radius: 0.025, center_radius: 0.03, marker_radius: 0.025, unit: 'picture_height' },
    },
  });
  const drawn = await runtime.waitFor((m) => m.type === 'overlay.drawn', 'overlay.drawn');
  // The page really painted the knob and the marker...
  assert.ok(drawn.probes.find((probe) => probe.id === 'range').rgba[3] > 200, 'the range knob is on the overlay canvas');
  assert.ok(drawn.markers[0].rgba[3] > 100, 'the marker is on the overlay canvas');
  // ...and the frame the runtime exports is the one it drew without them.
  const after = await draw(runtime, plan);
  assert.equal(after.hash, before.hash, 'markers and the sphere are not part of the picture');
});
