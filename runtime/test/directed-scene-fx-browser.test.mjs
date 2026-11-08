// Real pixels of the output-scope effects (F016), in headless Chrome (SwiftShader) through the same
// bridge messages the desktop sends. Skipped, loudly, without Chrome or the bundle
// (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
//
// What is proved: a plan without effects and a pass of amount 0 draw the very same picture; a
// vignette of amount > 0 leaves the middle alone and darkens the edges where the math says
// (against a JS transcription of the formula, and discriminating: the centre moves the dark
// side); passes stack in the plan's order; an effect the runtime does not draw is a typed
// refusal. The comparison against the core's own reference is crates/render/tests/scene_fx.rs.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { directedLayer, directedRuntime, flat, makeFiles, node, reference, resource, sceneAsset, scenePlan, SIZE } from './directed-scene-fixtures.mjs';
import { skipUnavailable } from './support.mjs';

const GREY = [200, 160, 80];
const PROBES = [
  [80, 45], // centre
  [1, 1], // top-left corner
  [158, 88], // bottom-right corner
  [80, 1], // top middle
  [1, 45], // left middle
];

const files = (t) => makeFiles(t, { wall: flat(16, 9, GREY) });

// A wall much larger than the frame at 12 units from the camera: every pixel is the flat colour.
const plan = (fx) =>
  scenePlan({
    nodes: [node('wall-node', 'wall', 0, 0, -2, 60, 34)],
    layers: [['wall-node', 'wall', 'opaque']],
    resources: [resource('wall', 16, 9)],
    requires: ['camera', 'nodeTransforms', 'imagePlanes', 'opaqueAlpha', ...(fx.length > 0 ? ['outputFx'] : [])],
    extra: fx.length > 0 ? { fx } : {},
  });

const pass = (id, amount, params = {}) => ({
  trackId: 't',
  clipId: id,
  effect: 'vignette',
  frameInClip: 0,
  weight: 1,
  amount,
  params: {
    radius: 0.75,
    softness: 0.45,
    intensity: amount,
    color: { r: 0, g: 0, b: 0 },
    roundness: 0.6,
    center: { x: 0.5, y: 0.5 },
    ...params,
  },
  scope: { kind: 'output' },
});

const toLinear = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const toSrgb = (l) => {
  const c = Math.min(1, Math.max(0, l));
  return c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055;
};
const smooth = (a, b, x) => {
  const t = Math.min(1, Math.max(0, (x - a) / (b - a)));
  return t * t * (3 - 2 * t);
};

/** The vignette at pixel (x, y) of a `w` x `h` picture of colour `rgb` (0..255), as the core's reference states it. */
const expected = (rgb, entry, [x, y], [w, h]) => {
  const { radius, softness, roundness, center, color } = entry.params;
  const u = (x + 0.5) / w;
  const v = (y + 0.5) / h;
  const stretch = 1 + (w / h - 1) * roundness;
  const dx = (u - center.x) * 2 * stretch;
  const dy = (1 - v - (1 - center.y)) * 2;
  const distance = Math.hypot(dx, dy) / Math.max(Math.hypot(stretch, 1), 1);
  const weight = smooth(radius, radius + softness, distance) * entry.amount;
  return rgb.map((channel, index) => {
    const linear = toLinear(channel / 255);
    const target = [color.r, color.g, color.b][index];
    return Math.round(toSrgb(linear + (target - linear) * weight) * 255);
  });
};

const near = (pixel, want, tolerance = 2) => want.every((value, channel) => Math.abs(pixel[channel] - value) <= tolerance);
const show = (pixel) => `rgb(${pixel.slice(0, 3).join(',')})`;

const draw = async (runtime, fx) => {
  const message = await runtime.frame([directedLayer(plan(fx), [reference('wall', 'slot-wall')])], { size: SIZE, probes: PROBES });
  assert.equal(message.type, 'frame.done', JSON.stringify(message.error));
  return message;
};

test('no effect and amount 0 keep the baseline; amount above 0 darkens the edges where the math says', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    const loaded = await runtime.load(sceneAsset('slot-wall', paths.wall, 'wall'));
    assert.equal(loaded.state, 'ready', JSON.stringify(loaded.error));

    const baseline = await draw(runtime, []);
    baseline.probes.forEach((pixel, index) => assert.ok(near(pixel, GREY, 1), `baseline ${index}: ${show(pixel)}`));
    assert.equal(baseline.source.directed_scene_fx, undefined);

    const zero = await draw(runtime, [pass('v', 0)]);
    assert.deepEqual(zero.probes, baseline.probes, 'a pass of amount 0 is the identity, byte for byte');
    assert.equal(zero.source.directed_scene_fx, 1);

    const full = pass('v', 1);
    const shaded = await draw(runtime, [full]);
    shaded.probes.forEach((pixel, index) => {
      const want = expected(GREY, full, PROBES[index], SIZE);
      assert.ok(near(pixel, want), `probe ${index} ${PROBES[index]}: expected ~rgb(${want}), got ${show(pixel)}`);
    });
    assert.ok(near(shaded.probes[0], GREY, 1), 'the middle is untouched');
    assert.ok(shaded.probes[1][0] + 40 < baseline.probes[1][0], 'the corner darkens');
    assert.ok(shaded.probes[2][0] + 40 < baseline.probes[2][0]);

    // Discriminating: moving the centre to the bottom right clears that corner and darkens the other.
    const moved = pass('v', 1, { center: { x: 1, y: 1 } });
    const shifted = await draw(runtime, [moved]);
    shifted.probes.forEach((pixel, index) => {
      const want = expected(GREY, moved, PROBES[index], SIZE);
      assert.ok(near(pixel, want), `moved probe ${index}: expected ~rgb(${want}), got ${show(pixel)}`);
    });
    assert.ok(shifted.probes[2][0] > shaded.probes[2][0] + 40, 'the bottom right is now clear');
    assert.ok(shifted.probes[1][0] < shaded.probes[1][0], 'and the top left is darker than before');

    // A colour other than black mixes in linear light.
    const tinted = pass('v', 0.6, { color: { r: 0, g: 0, b: 1 } });
    const blue = await draw(runtime, [tinted]);
    blue.probes.forEach((pixel, index) => {
      const want = expected(GREY, tinted, PROBES[index], SIZE);
      assert.ok(near(pixel, want), `tinted probe ${index}: expected ~rgb(${want}), got ${show(pixel)}`);
    });
  } finally {
    await runtime.close();
  }
});

test('passes stack in the plan order and an unknown effect is refused, never skipped', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    const loaded = await runtime.load(sceneAsset('slot-wall', paths.wall, 'wall'));
    assert.equal(loaded.state, 'ready', JSON.stringify(loaded.error));
    const one = await draw(runtime, [pass('a', 0.5)]);
    const two = await draw(runtime, [pass('a', 0.5), pass('b', 0.5)]);
    assert.equal(two.source.directed_scene_fx, 2);
    assert.ok(two.probes[1][0] < one.probes[1][0], 'the second pass darkens what the first left');
    assert.ok(near(two.probes[0], GREY, 1), 'the middle stays untouched by both');

    const refused = await runtime.frame([directedLayer(plan([{ ...pass('a', 0.5), effect: 'glitch' }]), [reference('wall', 'slot-wall')])], { size: SIZE, probes: PROBES });
    assert.equal(refused.type, 'frame.failed', JSON.stringify(refused));
    assert.equal(refused.error.code, 'unsupported');
    assert.equal(refused.error.details.effect, 'glitch');
  } finally {
    await runtime.close();
  }
});
