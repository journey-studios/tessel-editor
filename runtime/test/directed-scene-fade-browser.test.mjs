// Real pixels of the `fade` look (F016, P2), in headless Chrome (SwiftShader) through the same bridge
// messages the desktop sends. Skipped, loudly, without Chrome or the bundle (TESSEL_REQUIRE_RUNTIME=1
// makes that a failure).
//
// The expectation is a JS transcription of the PILOT's shader line (`runtime/fx/shaders/fade.ts`,
// snapshot d40ac3f7c0): `gl_FragColor = vec4(mix(sourceAt(vUv), uColor, uFade * uAmount), 1.0)`, in
// linear light, with `uAmount = clamp(intensity * weight)` and `uFade` the `amount` parameter. It is
// NOT the core's Rust reference and not this runtime's GLSL: a third derivation of the origin line.
//
// What is proved: a fade of amount 0 (any of the three factors) is the baseline byte for byte; the
// colour and the quantity are what the formula says everywhere on the picture; the fade is the SAME
// everywhere (no spatial shape); a fade stacked with a vignette differs by order and each order equals
// the formula; the runtime draws the plan's resolved values and nothing else.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { directedLayer, directedRuntime, flat, makeFiles, node, reference, resource, sceneAsset, scenePlan, SIZE } from './directed-scene-fixtures.mjs';
import { skipUnavailable } from './support.mjs';

const GREY = [200, 160, 80];
const PROBES = [
  [80, 45],
  [1, 1],
  [158, 88],
  [80, 1],
  [1, 45],
];

const files = (t) => makeFiles(t, { wall: flat(16, 9, GREY) });

const plan = (fx) =>
  scenePlan({
    output: SIZE,
    nodes: [node('wall-node', 'wall', 0, 0, -2, 60, 34)],
    layers: [['wall-node', 'wall', 'opaque']],
    resources: [resource('wall', 16, 9)],
    requires: ['camera', 'nodeTransforms', 'imagePlanes', 'opaqueAlpha', ...(fx.length > 0 ? ['outputFx'] : [])],
    extra: fx.length > 0 ? { fx } : {},
  });

/** A resolved fade pass as the core's plan lists it: `amount` is intensity x weight. */
const fade = (id, { amount = 1, intensity = 1, weight = 1, color = { r: 0, g: 0, b: 0 } } = {}) => ({
  trackId: 't',
  clipId: id,
  effect: 'fade',
  frameInClip: 0,
  weight,
  amount: Math.min(1, Math.max(0, intensity * weight)),
  params: { color, amount, intensity },
  scope: { kind: 'output' },
});

const vignette = (id, amount, color, radius, softness = 0.3) => ({
  trackId: 't',
  clipId: id,
  effect: 'vignette',
  frameInClip: 0,
  weight: 1,
  amount,
  params: { radius, softness, intensity: amount, color, roundness: 0.6, center: { x: 0.5, y: 0.5 } },
  scope: { kind: 'output' },
});

// The origin's curves (chunks/color.ts), transcribed.
const toLinear = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const toSrgb = (l) => {
  const c = Math.min(1, Math.max(0, l));
  return c < 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055;
};
const smooth = (a, b, x) => {
  const t = Math.min(1, Math.max(0, (x - a) / (b - a)));
  return t * t * (3 - 2 * t);
};

/** `mix(source, color, fadeAmount * uAmount)` per channel, in linear light, back to 8 bits. */
const fadeOf = (rgb, entry) => {
  const mixAmount = entry.params.amount * entry.amount;
  const target = [entry.params.color.r, entry.params.color.g, entry.params.color.b];
  return rgb.map((channel, index) => {
    const linear = toLinear(channel / 255);
    return Math.round(toSrgb(linear + (target[index] - linear) * mixAmount) * 255);
  });
};

/** The pilot's vignette (shaders/vignette.ts) at a probe, over an 8-bit colour. */
const vignetteOf = (rgb, entry, [x, y], [w, h]) => {
  const { radius, softness, roundness, center, color } = entry.params;
  const stretch = 1 + (w / h - 1) * roundness;
  const dx = ((x + 0.5) / w - center.x) * 2 * stretch;
  const dy = (1 - (y + 0.5) / h - (1 - center.y)) * 2;
  const distance = Math.hypot(dx, dy) / Math.max(Math.hypot(stretch, 1), 1);
  const weight = smooth(radius, radius + softness, distance) * entry.amount;
  const target = [color.r, color.g, color.b];
  return rgb.map((channel, index) => {
    const linear = toLinear(channel / 255);
    return Math.round(toSrgb(linear + (target[index] - linear) * weight) * 255);
  });
};

const near = (pixel, want, tolerance = 2) => want.every((value, channel) => Math.abs(pixel[channel] - value) <= tolerance);
const show = (pixel) => `rgb(${pixel.slice(0, 3).join(',')})`;

const draw = async (runtime, fx) => {
  const message = await runtime.frame([directedLayer(plan(fx), [reference('wall', 'slot-wall')])], { size: SIZE, probes: PROBES });
  assert.equal(message.type, 'frame.done', JSON.stringify(message.error));
  return message;
};

const withWall = async (t, body) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    const loaded = await runtime.load(sceneAsset('slot-wall', paths.wall, 'wall'));
    assert.equal(loaded.state, 'ready', JSON.stringify(loaded.error));
    await body(runtime);
  } finally {
    await runtime.close();
  }
};

test('a fade of amount, intensity or weight 0 is the baseline byte for byte; a positive one is what the pilot line says', async (t) => {
  await withWall(t, async (runtime) => {
    const baseline = await draw(runtime, []);
    for (const [label, entry] of [
      ['amount 0', fade('f', { amount: 0 })],
      ['intensity 0', fade('f', { intensity: 0 })],
      ['weight 0', fade('f', { weight: 0 })],
    ]) {
      const zero = await draw(runtime, [entry]);
      assert.deepEqual(zero.probes, baseline.probes, `${label} is the identity`);
      assert.equal(zero.hash, baseline.hash, `${label}: the whole picture is the baseline's`);
      assert.equal(zero.source.directed_scene_fx, 1, 'the pass is still drawn (and is the identity)');
    }

    // Full black: every pixel is the colour, everywhere (no spatial shape).
    const black = await draw(runtime, [fade('f')]);
    black.probes.forEach((pixel, index) => assert.ok(near(pixel, [0, 0, 0], 1), `black probe ${index}: ${show(pixel)}`));

    // Half white: linear-light mix, the same at every probe, and clearly brighter than the grey.
    const half = fade('f', { amount: 0.5, color: { r: 1, g: 1, b: 1 } });
    const lifted = await draw(runtime, [half]);
    const want = fadeOf(GREY, half);
    lifted.probes.forEach((pixel, index) => {
      assert.ok(near(pixel, want), `half white probe ${index}: expected ~rgb(${want}), got ${show(pixel)}`);
      assert.ok(pixel[0] > GREY[0] + 20, 'it brightens');
    });
    // A linear-light mix is not the mix of the encoded bytes: the transcription discriminates.
    const naive = GREY.map((channel) => Math.round(channel * 0.5 + 255 * 0.5));
    assert.ok(!near(want, naive, 6), `the linear mix ${want} differs from the encoded mix ${naive}`);
  });
});

test('colour, amount, intensity and the envelope each move the picture where the formula says', async (t) => {
  await withWall(t, async (runtime) => {
    const cases = [
      fade('red', { amount: 0.7, color: { r: 1, g: 0, b: 0 } }),
      fade('weighted', { amount: 1, intensity: 0.8, weight: 0.5, color: { r: 0, g: 0, b: 1 } }),
      fade('quarter', { amount: 0.25, color: { r: 0, g: 0, b: 0 } }),
    ];
    const seen = new Set();
    for (const entry of cases) {
      const message = await draw(runtime, [entry]);
      const want = fadeOf(GREY, entry);
      message.probes.forEach((pixel, index) => assert.ok(near(pixel, want), `${entry.clipId} probe ${index}: expected ~rgb(${want}), got ${show(pixel)}`));
      seen.add(message.hash);
    }
    assert.equal(seen.size, cases.length, 'three different fades are three different pictures');
  });
});

test('a fade and a vignette stack in the plan order, and the two orders are different pictures that both equal the formula', async (t) => {
  await withWall(t, async (runtime) => {
    const white = fade('white', { amount: 0.6, color: { r: 1, g: 1, b: 1 } });
    const dark = vignette('dark', 1, { r: 0, g: 0, b: 0 }, 0.2);
    const fadeFirst = await draw(runtime, [white, dark]);
    const vignetteFirst = await draw(runtime, [dark, white]);
    assert.notEqual(fadeFirst.hash, vignetteFirst.hash, 'the order is part of the picture');
    const apart = fadeFirst.probes.filter((pixel, index) => pixel.slice(0, 3).some((value, channel) => Math.abs(value - vignetteFirst.probes[index][channel]) > 8));
    assert.ok(apart.length >= 2, `the orders differ where the vignette acts (${apart.length} probes)`);
    const after = (order, probe) =>
      order.reduce((rgb, entry) => (entry.effect === 'fade' ? fadeOf(rgb, entry) : vignetteOf(rgb, entry, probe, SIZE)), GREY);
    for (const [label, message, order] of [
      ['fade then vignette', fadeFirst, [white, dark]],
      ['vignette then fade', vignetteFirst, [dark, white]],
    ]) {
      message.probes.forEach((pixel, index) => {
        const want = after(order, PROBES[index]);
        assert.ok(near(pixel, want, 3), `${label} probe ${index}: expected ~rgb(${want}), got ${show(pixel)}`);
      });
    }
    // One material per effect, not per pass: the two looks are two programs.
    assert.equal((await runtime.retention()).fx_material_entries, 2);
  });
});

test('a fade is drawn only from the plan: a missing or extra parameter is a typed plan refusal', async (t) => {
  await withWall(t, async (runtime) => {
    const entry = fade('f', { amount: 0.5 });
    const { amount, ...rest } = entry.params;
    const missing = await runtime.frame([directedLayer(plan([{ ...entry, params: rest }]), [reference('wall', 'slot-wall')])], { size: SIZE, probes: PROBES });
    assert.equal(missing.type, 'frame.failed', JSON.stringify(missing));
    assert.equal(missing.error.code, 'invalid_plan');
    assert.ok(amount === 0.5);
    const extra = await runtime.frame([directedLayer(plan([{ ...entry, params: { ...entry.params, radius: 0.5 } }]), [reference('wall', 'slot-wall')])], { size: SIZE, probes: PROBES });
    assert.equal(extra.type, 'frame.failed', JSON.stringify(extra));
    assert.equal(extra.error.code, 'invalid_plan');
    const high = await runtime.frame([directedLayer(plan([{ ...entry, params: { ...entry.params, amount: 1.5 } }]), [reference('wall', 'slot-wall')])], { size: SIZE, probes: PROBES });
    assert.equal(high.type, 'frame.failed', JSON.stringify(high));
    assert.equal(high.error.code, 'invalid_plan');
  });
});
