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
const plan = (fx, output = SIZE) =>
  scenePlan({
    output,
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

const draw = async (runtime, fx, size = SIZE) => {
  const message = await runtime.frame([directedLayer(plan(fx, size), [reference('wall', 'slot-wall')])], { size, probes: PROBES });
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

// ---- F062: a pass of a shot -------------------------------------------------------------------------

test('a pass of a shot draws the picture of the same pass of the output and the frame counts it apart', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    assert.equal((await runtime.load(sceneAsset('slot-wall', paths.wall, 'wall'))).state, 'ready');
    const onShot = (id, amount) => ({ ...pass(id, amount), scope: { kind: 'shot', shotId: 's1' } });
    const drawShot = async (fx) => {
      const base = plan(fx);
      const layer = directedLayer({ ...base, requires: [...base.requires, 'fxShotScope'], frame: { shotId: 's1' } }, [reference('wall', 'slot-wall')]);
      const message = await runtime.frame([layer], { size: SIZE, probes: PROBES });
      assert.equal(message.type, 'frame.done', JSON.stringify(message.error));
      return message;
    };
    const output = await draw(runtime, [pass('o', 0.8)]);
    assert.equal(output.source.directed_scene_fx_shot, undefined, 'a plan of the output alone reports no pass of a shot');

    const shot = await drawShot([onShot('s', 0.8)]);
    assert.deepEqual(shot.probes, output.probes, 'one pass of the shot over the picture of the scene is the picture of the output pass');
    assert.equal(shot.source.directed_scene_fx, 1);
    assert.equal(shot.source.directed_scene_fx_shot, 1);

    const both = await drawShot([onShot('s', 0.5), pass('o', 0.5)]);
    assert.equal(both.source.directed_scene_fx, 2, 'both passes are drawn');
    assert.equal(both.source.directed_scene_fx_shot, 1, 'and only the pass of the shot is counted apart');
  } finally {
    await runtime.close();
  }
});

// ---- Order, ping-pong, lifetime (the review of F016: A1, A5) ------------------------------------------

const RED = { r: 1, g: 0, b: 0 };
const BLUE = { r: 0, g: 0, b: 1 };
/** `pass` with the colour, radius and softness that make the two orders visibly different. */
const look = (id, amount, color, radius, softness = 0.3) => pass(id, amount, { color, radius, softness, intensity: amount });

/** What a pass does to a probe's colour, applied to the 8-bit result of the pass before it (each pass is rounded into a target). */
const stacked = (entries, probe, size = SIZE) => entries.reduce((rgb, entry) => expected(rgb, entry, probe, size), GREY);

test('the order of passes with distinct colours changes the picture, and both orders are what the formula says', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    assert.equal((await runtime.load(sceneAsset('slot-wall', paths.wall, 'wall'))).state, 'ready');
    const red = look('red', 1, RED, 0.2);
    const blue = look('blue', 0.8, BLUE, 0.45);
    const redThenBlue = await draw(runtime, [red, blue]);
    const blueThenRed = await draw(runtime, [blue, red]);
    assert.notEqual(redThenBlue.hash, blueThenRed.hash, 'the two orders are two pictures');
    // At least one probe differs by a margin that a rounding error cannot explain (the corners, where both passes act).
    const apart = redThenBlue.probes.filter((pixel, index) => pixel.slice(0, 3).some((value, channel) => Math.abs(value - blueThenRed.probes[index][channel]) > 8));
    assert.ok(apart.length >= 2, `the orders differ where both passes act (${apart.length} probes)`);
    for (const [label, message, order] of [['red then blue', redThenBlue, [red, blue]], ['blue then red', blueThenRed, [blue, red]]]) {
      message.probes.forEach((pixel, index) => {
        const want = stacked(order, PROBES[index]);
        assert.ok(near(pixel, want, 3), `${label} probe ${index} ${PROBES[index]}: expected ~rgb(${want}), got ${show(pixel)}`);
      });
    }
    // Repeating an order draws the very same pixels (nothing leaks from the other order's targets).
    assert.equal((await draw(runtime, [red, blue])).hash, redThenBlue.hash);
  } finally {
    await runtime.close();
  }
});

test('many passes ping-pong between two targets, keep every pass in order and relink nothing per frame', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    assert.equal((await runtime.load(sceneAsset('slot-wall', paths.wall, 'wall'))).state, 'ready');
    // Five passes, odd count (the last lands in the first target) and every one a different look.
    const colours = [RED, BLUE, { r: 0, g: 1, b: 0 }, { r: 1, g: 1, b: 0 }, { r: 0, g: 0, b: 0 }];
    const entries = colours.map((color, index) => look(`p${index}`, 0.5, color, 0.15 + index * 0.1));
    const first = await draw(runtime, entries);
    first.probes.forEach((pixel, index) => {
      const want = stacked(entries, PROBES[index]);
      assert.ok(near(pixel, want, 4), `5 passes, probe ${index} ${PROBES[index]}: expected ~rgb(${want}), got ${show(pixel)}`);
    });
    // 1 scene target + 2 ping-pong targets, not one per pass.
    assert.equal(first.cache.target_entries, 3);
    const programs = (await runtime.page.evaluate(() => window.TesselRuntime.debug.glMemory())).programs;
    for (let frame = 0; frame < 3; frame += 1) {
      const again = await draw(runtime, entries);
      assert.equal(again.hash, first.hash, 'the same plan, the same pixels, frame after frame');
      assert.equal(again.cache.fx_pipeline_created, 0);
      assert.equal(again.cache.fx_program_created, 0, 'no material (so no program link) is made in a steady frame');
      assert.equal(again.cache.fx_program_reused, 5);
      assert.equal(again.cache.material_created, 0);
    }
    assert.equal((await runtime.page.evaluate(() => window.TesselRuntime.debug.glMemory())).programs, programs, 'no program appears or goes per frame');
    assert.equal((await runtime.retention()).fx_material_entries, 1, 'one material per effect, not per pass');

    // A larger budget than any a monitor would use stays on the same two targets.
    const many = Array.from({ length: 9 }, (_, index) => look(`m${index}`, 0.2, colours[index % colours.length], 0.3 + index * 0.05));
    const nine = await draw(runtime, many);
    assert.equal(nine.cache.target_entries <= 3, true, JSON.stringify(nine.cache));
    nine.probes.forEach((pixel, index) => {
      const want = stacked(many, PROBES[index]);
      assert.ok(near(pixel, want, 6), `9 passes, probe ${index}: expected ~rgb(${want}), got ${show(pixel)}`);
    });
  } finally {
    await runtime.close();
  }
});

test('a resize draws the new size and coming back draws the first picture again', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    assert.equal((await runtime.load(sceneAsset('slot-wall', paths.wall, 'wall'))).state, 'ready');
    const entries = [look('red', 1, RED, 0.3), look('blue', 0.7, BLUE, 0.5)];
    const small = await draw(runtime, entries);
    const wide = [200, 90];
    const resized = await draw(runtime, entries, wide);
    assert.deepEqual([resized.width, resized.height], wide);
    // The ellipse follows the aspect of the new picture: probes at its own corners and middle.
    for (const [index, probe] of PROBES.entries()) {
      const at = [Math.min(probe[0], wide[0] - 2), probe[1]];
      const pixel = resized.probes[index];
      if (at[0] !== probe[0]) continue;
      const want = stacked(entries, probe, wide);
      assert.ok(near(pixel, want, 4), `resized probe ${index}: expected ~rgb(${want}), got ${show(pixel)}`);
    }
    const back = await draw(runtime, entries);
    assert.equal(back.hash, small.hash, 'the first size draws the first picture again');
    assert.equal(back.cache.fx_pipeline_created, 0, 'a resize does not cost the pipeline');
    assert.equal(back.cache.fx_program_created, 0);
  } finally {
    await runtime.close();
  }
});

test('a pass that fails discards the pipeline, leaves no program behind and the next frame draws as before', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    assert.equal((await runtime.load(sceneAsset('slot-wall', paths.wall, 'wall'))).state, 'ready');
    const entries = [look('red', 1, RED, 0.2), look('blue', 0.8, BLUE, 0.45), look('green', 0.6, { r: 0, g: 1, b: 0 }, 0.3)];
    const before = await draw(runtime, entries);
    const glBefore = await runtime.page.evaluate(() => window.TesselRuntime.debug.glMemory());

    // The second pass of the next frame throws, as a GPU error in the middle of the chain would.
    await runtime.page.evaluate(() => {
      const { renderer } = window.TesselRuntime.debug.compositor();
      const original = renderer.render.bind(renderer);
      let passes = 0;
      renderer.render = (scene, camera) => {
        if (scene.children[0]?.material?.uniforms?.uAmount) {
          passes += 1;
          if (passes === 2) {
            renderer.render = original;
            throw new Error('fx pass failed on purpose');
          }
        }
        return original(scene, camera);
      };
    });
    const failed = await draw(runtime, entries).catch((error) => error);
    assert.ok(failed instanceof Error && /fx pass failed on purpose|frame\.failed/.test(failed.message), String(failed));
    const held = await runtime.retention();
    assert.equal(held.fx_pipeline_entries, 0, 'the pipeline of the failed frame is gone');
    assert.equal(held.counters.fx_pipeline_discarded, 1);
    assert.equal((await runtime.page.evaluate(() => window.TesselRuntime.debug.glMemory())).programs, glBefore.programs - 1, 'its program was released');

    const after = await draw(runtime, entries);
    assert.equal(after.hash, before.hash, 'the frame after the failure is the frame before it');
    assert.equal(after.cache.fx_pipeline_created, 1, 'a new pipeline');
    assert.equal(after.cache.fx_program_created, 1);
    assert.equal((await runtime.page.evaluate(() => window.TesselRuntime.debug.glMemory())).programs, glBefore.programs, 'no program leaked by the failure');
    const steady = await draw(runtime, entries);
    assert.equal(steady.cache.fx_program_created, 0);
  } finally {
    await runtime.close();
  }
});

test('a lost context drops the pipeline and the restored one draws the same pixels; without retention nothing is kept', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    assert.equal((await runtime.load(sceneAsset('slot-wall', paths.wall, 'wall'))).state, 'ready');
    const entries = [look('red', 1, RED, 0.2), look('blue', 0.8, BLUE, 0.45)];
    const before = await draw(runtime, entries);
    assert.equal((await runtime.retention()).fx_pipeline_entries, 1);

    await runtime.page.evaluate(() => window.TesselRuntime.debug.loseContext());
    await runtime.waitFor((m) => m.type === 'context.lost');
    assert.equal((await runtime.retention()).fx_pipeline_entries, 0, 'its GPU state went with the context');
    await runtime.page.evaluate(() => window.TesselRuntime.debug.restoreContext());
    await runtime.waitFor((m) => m.type === 'context.restored');
    const after = await draw(runtime, entries);
    assert.equal(after.hash, before.hash);
    assert.equal(after.cache.fx_pipeline_created, 1);
  } finally {
    await runtime.close();
  }

  // Retention off is the original build-and-free path: the same pixels, a pipeline per frame, none kept.
  const cold = await directedRuntime(paths, { retain: false });
  try {
    assert.equal((await cold.load(sceneAsset('slot-wall', paths.wall, 'wall'))).state, 'ready');
    const entries = [look('red', 1, RED, 0.2), look('blue', 0.8, BLUE, 0.45)];
    const frame = await draw(cold, entries);
    const held = await cold.retention();
    assert.equal(held.fx_pipeline_entries ?? 0, 0);
    assert.equal(held.counters.fx_pipeline_created, 0);
    const reference = await directedRuntime(paths);
    try {
      assert.equal((await reference.load(sceneAsset('slot-wall', paths.wall, 'wall'))).state, 'ready');
      assert.equal((await draw(reference, entries)).hash, frame.hash, 'retained and unretained draw the same picture');
    } finally {
      await reference.close();
    }
  } finally {
    await cold.close();
  }
});
