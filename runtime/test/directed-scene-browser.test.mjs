// Real pixels of the directed-scene layer, in headless Chrome (SwiftShader) through the same bridge
// messages the desktop sends. Skipped, loudly, without Chrome or the bundle
// (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
//
// What is proved here: the plan is drawn as listed (orientation, straight and opaque alpha, paint
// order against depth); resources are bound by identity and every invalidation (revision swap,
// project generation, release, budget, lost context, cancellation) ends in a typed answer or the
// same picture as before. What is not: pixels against the origin's runtime (BL018/BL071) and the
// desktop WebView (that needs the window; see docs/features/scene-renderer.md).
import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  GENERATION,
  SIZE,
  directedLayer,
  directedRuntime,
  flat,
  makeFiles,
  node,
  png,
  quadrants,
  reference,
  resource,
  sceneAsset,
  scenePlan,
  vec,
} from './directed-scene-fixtures.mjs';
import { frameMessage, glMemory, loseDuring, loseNow, post, restore } from './lifecycle-fixtures.mjs';
import { holdFetches, imageLayer, picture, shuffled, skipUnavailable, stillAsset } from './support.mjs';

const PROBES = [
  [69, 34], // top-left quadrant of a 4x4 plane at the origin
  [91, 34],
  [69, 56],
  [91, 56],
  [80, 45], // center
  [50, 45], // inside a 8x4.5 wall 2 units behind, outside a 2x2 plane in front
  [10, 45], // outside everything: the cleared background
  // The extent of the 4x4 plane at 10 units with a 45 degree vertical field: 43.4 px, x 58.3..101.7, y 23.3..66.7.
  [55, 40],
  [62, 40],
  [75, 21],
  [75, 25],
];

/** Two channels within `tolerance` (blending and 8-bit rounding), alpha ignored. */
const near = (pixel, expected, tolerance = 4) =>
  expected.every((value, channel) => Math.abs(pixel[channel] - value) <= tolerance);
const show = (pixel) => `rgb(${pixel.slice(0, 3).join(',')})`;
const assertPixel = (pixel, expected, label, tolerance) =>
  assert.ok(near(pixel, expected, tolerance), `${label}: expected ~rgb(${expected}), got ${show(pixel)}`);

const files = (t) =>
  makeFiles(t, {
    quad: quadrants(),
    wall: flat(16, 9, [0, 0, 255]),
    glass: png(4, 4, () => [255, 0, 0, 128]),
    stray: png(4, 4, () => [0, 255, 0, 10]),
    red: flat(4, 4, [255, 0, 0]),
    green: flat(4, 4, [0, 255, 0]),
    other: flat(8, 8, [10, 200, 90]),
    plain: picture(3),
  });

const SIZES = { quad: [8, 8], wall: [16, 9], glass: [4, 4], stray: [4, 4], red: [4, 4], green: [4, 4], other: [8, 8] };

/** Loads every picture of `paths` into its own slot (`slot-<name>`) and returns the references. */
const loadAll = async (runtime, paths, generation = GENERATION) => {
  const references = {};
  for (const [name, path] of Object.entries(paths).filter(([name]) => name in SIZES)) {
    const loaded = await runtime.load(sceneAsset(`slot-${name}`, path, name, { generation }));
    assert.equal(loaded.state, 'ready', JSON.stringify(loaded.error));
    references[name] = reference(name, `slot-${name}`, { generation });
  }
  return references;
};

const resourceOf = (name) => resource(name, ...SIZES[name]);

/** A plan with one plane per `[name, x, y, z, w, h, alpha]`, listed in the order given. */
const planOf = (planes, extra = {}) =>
  scenePlan({
    nodes: planes.map(([name, x, y, z, w, h]) => node(`n-${name}`, name, x, y, z, w, h)),
    layers: planes.map(([name, , , , , , alpha = 'straight']) => [`n-${name}`, name, alpha]),
    resources: [...new Set(planes.map(([name]) => name))].sort().map(resourceOf),
    ...extra,
  });

const draw = (runtime, plan, references, options = {}) => {
  const needed = new Set(plan.resources.map((entry) => entry.assetId));
  const layer = directedLayer(plan, Object.values(references).filter((entry) => needed.has(entry.asset_id)));
  return runtime.frame([layer], { size: SIZE, probes: PROBES, ...options });
};

const done = (message) => {
  assert.equal(message.type, 'frame.done', JSON.stringify(message.error));
  return message;
};

test('the plan is drawn as listed: orientation, straight and opaque alpha, paint order over depth', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    const ready = await runtime.hello();
    assert.ok(ready.capabilities.features.includes('directed_scene'));
    assert.equal(ready.capabilities.directed_scene.plan_kind, 'tessel-scene-plan');
    assert.deepEqual(ready.capabilities.directed_scene.capabilities.length, 12);
    const refs = await loadAll(runtime, paths);

    // Orientation: image top is plane top (the V axis is flipped on purpose), left is left.
    const upright = done(await draw(runtime, planOf([['quad', 0, 0, 0, 4, 4]]), refs));
    assertPixel(upright.probes[0], [255, 0, 0], 'top-left quadrant is red');
    assertPixel(upright.probes[1], [0, 255, 0], 'top-right quadrant is green');
    assertPixel(upright.probes[2], [0, 0, 255], 'bottom-left quadrant is blue');
    assertPixel(upright.probes[3], [255, 255, 0], 'bottom-right quadrant is yellow');
    assertPixel(upright.probes[6], [0, 0, 0], 'nothing there: opaque black');
    // The edges land where the vertical field of view puts them (a horizontal one would not).
    assertPixel(upright.probes[7], [0, 0, 0], 'just left of the plane');
    assertPixel(upright.probes[8], [255, 0, 0], 'just inside its left edge');
    assertPixel(upright.probes[9], [0, 0, 0], 'just above the plane');
    assertPixel(upright.probes[10], [255, 0, 0], 'just inside its top edge');
    assert.equal(upright.source.directed_scene_layers, 1);

    // Straight alpha: half-transparent red over the blue wall, wall untouched elsewhere.
    const blended = done(
      await draw(runtime, planOf([['wall', 0, 0, -2, 8, 4.5, 'opaque'], ['glass', 0, 0, 0, 2, 2]]), refs),
    );
    assertPixel(blended.probes[4], [128, 0, 127], 'red at 50% over blue', 3);
    assertPixel(blended.probes[5], [0, 0, 255], 'the wall beside the glass');
    assertPixel(blended.probes[6], [0, 0, 0], 'outside the wall');

    // A picture that is `opaque` in the plan shows its color whatever stray alpha its file has;
    // as `straight` the same file is nearly transparent over the wall.
    const wall = ['wall', 0, 0, -2, 8, 4.5, 'opaque'];
    const opaque = done(await draw(runtime, planOf([wall, ['stray', 0, 0, 0, 2, 2, 'opaque']]), refs));
    assertPixel(opaque.probes[4], [0, 255, 0], 'opaque layer ignores the alpha of its file');
    const faint = done(await draw(runtime, planOf([wall, ['stray', 0, 0, 0, 2, 2, 'straight']]), refs));
    assertPixel(faint.probes[4], [0, 10, 245], 'the same file as straight alpha: 4% green over blue', 4);

    // Paint order is the plan's: the nearer plane drawn last is on top, and when the plan lists it
    // first the farther one is on top, whatever the depth says.
    const near1 = ['green', 0, 0, 1, 4, 4];
    const far0 = ['red', 0, 0, 0, 4, 4];
    assertPixel(done(await draw(runtime, planOf([far0, near1]), refs)).probes[4], [0, 255, 0], 'listed last, nearer: on top');
    assertPixel(done(await draw(runtime, planOf([near1, far0]), refs)).probes[4], [255, 0, 0], 'listed last, farther: still on top');
  } finally {
    await runtime.close();
  }
});

test('a frame that nothing changed reuses what the retention layer kept, and draws the same pixels', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    const refs = await loadAll(runtime, paths);
    const plan = planOf([['wall', 0, 0, -2, 8, 4.5, 'opaque'], ['glass', 0, 0, 0, 2, 2]]);
    const first = done(await draw(runtime, plan, refs));
    assert.equal(first.cache.texture_created, 2);
    // Two layer materials plus the one of the quad that composites the scene's picture.
    assert.equal(first.cache.material_created, 3);
    const second = done(await draw(runtime, plan, refs));
    assert.equal(second.hash, first.hash);
    assert.equal(second.cache.texture_created, 0, 'the pictures stay on the GPU');
    assert.equal(second.cache.texture_reused, 2);
    assert.equal(second.cache.material_created, 0, 'the shader programs stay linked');
    assert.equal(second.cache.scene_resources, Object.keys(SIZES).length, 'the whole project stays loaded, whatever a frame uses');
    assert.equal(second.cache.scene_generation, GENERATION);

    // Retention off is the build-and-free path: the same pixels, nothing kept.
    await runtime.send({ type: 'hello', retain: false });
    await runtime.waitFor((m) => m.type === 'ready');
    const cold = done(await draw(runtime, plan, refs));
    assert.equal(cold.hash, first.hash, 'with and without retention the picture is the same');
    assert.equal(cold.cache.material_entries, 0);
  } finally {
    await runtime.close();
  }
});

test('an ordinary layer drawn with a directed scene is the picture it always was', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    const refs = await loadAll(runtime, paths);
    assert.equal((await runtime.load(stillAsset('plain', paths.plain))).state, 'ready');
    const alone = done(await runtime.frame([imageLayer('plain')], { size: SIZE }));
    const plan = planOf([['quad', 0, 0, 0, 4, 4]]);
    const scene = directedLayer(plan, [refs.quad]);
    const sceneAlone = done(await runtime.frame([scene], { size: SIZE }));
    // Both in one frame: the ordinary layer on top of the scene (one target slot each, no sharing).
    const both = done(await runtime.frame([scene, imageLayer('plain', { opacity: 0.5 })], { size: SIZE }));
    assert.notEqual(both.hash, alone.hash);
    assert.notEqual(both.hash, sceneAlone.hash);
    // What an ordinary layer draws does not depend on a directed scene having been drawn before it.
    assert.equal(done(await runtime.frame([imageLayer('plain')], { size: SIZE })).hash, alone.hash);
    assert.equal(done(await runtime.frame([scene], { size: SIZE })).hash, sceneAlone.hash);
  } finally {
    await runtime.close();
  }
});

test('partial plans, missing capabilities and bad versions fail typed, and the runtime keeps drawing', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    const refs = await loadAll(runtime, paths);
    const good = planOf([['quad', 0, 0, 0, 4, 4]]);
    const baseline = done(await draw(runtime, good, refs));

    const partial = await draw(
      runtime,
      planOf([['quad', 0, 0, 0, 4, 4]], { extra: { complete: false, unsupported: [{ domain: 'lighting', subject: null }] } }),
      refs,
    );
    assert.equal(partial.type, 'frame.failed');
    assert.equal(partial.error.code, 'unsupported');
    assert.deepEqual(partial.error.details.domains, [{ domain: 'lighting', subject: null }]);

    const needy = await draw(runtime, planOf([['quad', 0, 0, 0, 4, 4]], { requires: ['camera', 'relief'] }), refs);
    assert.equal(needy.error.code, 'unsupported');
    assert.deepEqual(needy.error.details.missing, ['relief']);

    const future = await draw(runtime, planOf([['quad', 0, 0, 0, 4, 4]], { extra: { planVersion: 9 } }), refs);
    assert.equal(future.error.code, 'unsupported_plan_version');

    const broken = planOf([['quad', 0, 0, 0, 4, 4]]);
    broken.camera.fovDeg = 400;
    assert.equal((await draw(runtime, broken, refs)).error.code, 'invalid_plan');

    const after = done(await draw(runtime, good, refs));
    assert.equal(after.hash, baseline.hash, 'a refused plan leaves nothing behind in the next frame');
  } finally {
    await runtime.close();
  }
});

test('resources are bound by identity: unresolved, mismatched and stale slots are typed, never the picture of another revision', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    const refs = await loadAll(runtime, paths);
    const plan = planOf([['red', 0, 0, 0, 4, 4]]);
    const red = done(await draw(runtime, plan, refs));
    assertPixel(red.probes[4], [255, 0, 0], 'the red picture');

    // The frame names no slot for what the plan requires.
    const unnamed = await runtime.frame([directedLayer(plan, [])], { size: SIZE });
    assert.deepEqual([unnamed.error.code, unnamed.error.details.asset_id], ['resource_unresolved', 'red']);

    // The host loaded another revision than the one the plan asks for.
    const revised = await runtime.frame([directedLayer(plan, [reference('red', 'slot-red', { revision_id: 'red-r2' })])], { size: SIZE });
    assert.equal(revised.error.code, 'resource_mismatch');
    const rehashed = await runtime.frame([directedLayer(plan, [reference('red', 'slot-red', { content_hash: 'other' })])], { size: SIZE });
    assert.equal(rehashed.error.code, 'resource_mismatch');

    // Same asset id, new revision, loaded into the same slot: the old identity is gone from the
    // runtime, so a frame still asking for it is stale and the new revision draws its own picture.
    const swap = await runtime.load(sceneAsset('slot-red', paths.other, 'red', { revision_id: 'red-r2', content_hash: 'hash-red-2' }));
    assert.equal(swap.state, 'ready');
    const stale = await draw(runtime, plan, refs);
    assert.equal(stale.error.code, 'resource_stale');
    assert.equal(stale.error.details.held.revision_id, 'red-r2');
    const revisedPlan = planOf([['red', 0, 0, 0, 4, 4]]);
    revisedPlan.resources[0] = { ...resourceOf('red'), revisionId: 'red-r2', contentHash: 'hash-red-2', widthPx: 8, heightPx: 8 };
    const fresh = done(
      await runtime.frame(
        [directedLayer(revisedPlan, [reference('red', 'slot-red', { revision_id: 'red-r2', content_hash: 'hash-red-2' })])],
        { size: SIZE, probes: PROBES },
      ),
    );
    assertPixel(fresh.probes[4], [10, 200, 90], 'the new revision');
    assert.notEqual(fresh.hash, red.hash);

    // A file that is not the picture the plan declares (size) is refused, not stretched.
    const wrongSize = planOf([['red', 0, 0, 0, 4, 4]]);
    wrongSize.resources[0] = { ...resourceOf('red'), revisionId: 'red-r2', contentHash: 'hash-red-2' };
    const sized = await runtime.frame(
      [directedLayer(wrongSize, [reference('red', 'slot-red', { revision_id: 'red-r2', content_hash: 'hash-red-2' })])],
      { size: SIZE },
    );
    assert.equal(sized.error.code, 'resource_mismatch');

    // Released slots are unresolved and free their GPU texture.
    await runtime.release('slot-red');
    const gone = await draw(runtime, plan, refs);
    assert.deepEqual([gone.error.code, gone.error.details.slot], ['resource_unresolved', 'slot-red']);
  } finally {
    await runtime.close();
  }
});

test('a newer project generation replaces the older one: its resources are released and cannot be loaded again', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    const first = await loadAll(runtime, { red: paths.red, wall: paths.wall }, 1);
    const plan = planOf([['red', 0, 0, 0, 4, 4]]);
    const before = done(await draw(runtime, plan, first));
    assert.equal((await runtime.retention()).scene_resources, 2);
    const { textures } = await glMemory(runtime);

    // The host replaces the project: one resource of generation 2 is enough for the runtime to
    // drop everything of generation 1, even if the host released nothing.
    const second = await runtime.load(sceneAsset('slot-next', paths.green, 'red', { generation: 2 }));
    assert.equal(second.state, 'ready');
    const held = await runtime.retention();
    assert.deepEqual([held.scene_resources, held.scene_generation, held.texture_entries], [1, 2, 0]);
    assert.deepEqual(await runtime.page.evaluate(() => window.TesselRuntime.debug.loadedAssets()), ['slot-next']);
    assert.ok((await glMemory(runtime)).textures < textures, 'the old world left no picture on the GPU');

    // A late frame of the old project names slots that no longer exist; it never shows the new picture.
    const late = await draw(runtime, plan, first);
    assert.equal(late.error.code, 'resource_unresolved');
    // A late load of the old generation is refused and changes nothing.
    const lateLoad = await runtime.load(sceneAsset('slot-late', paths.red, 'red', { generation: 1 }));
    assert.equal(lateLoad.state, 'failed');
    assert.equal(lateLoad.error.code, 'resource_stale');
    assert.equal((await runtime.retention()).scene_resources, 1);

    // The same asset id under the new generation draws the new project's picture.
    const next = done(await draw(runtime, plan, { red: reference('red', 'slot-next', { generation: 2 }) }));
    assertPixel(next.probes[4], [0, 255, 0], 'the new project');
    assert.notEqual(next.hash, before.hash);
  } finally {
    await runtime.close();
  }
});

test('seeking out of order draws the same picture for the same frame, whatever was drawn before', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    const refs = await loadAll(runtime, paths);
    // Frame k: the camera has dollied sideways by k/4 units (the core would send these numbers).
    const frameK = (k) => planOf([['wall', 0, 0, -2, 8, 4.5, 'opaque'], ['quad', 0.2, 0, 0, 2, 2]], { camera: { position: vec(k / 4, 0, 10), target: vec(k / 4, 0, 0) } });
    const inOrder = {};
    for (const k of [0, 1, 2, 3, 4, 5, 6, 7]) inOrder[k] = done(await draw(runtime, frameK(k), refs)).hash;
    assert.equal(new Set(Object.values(inOrder)).size, 8, 'every frame is a different picture');
    for (const k of shuffled([0, 1, 2, 3, 4, 5, 6, 7, 3, 3, 0, 7], 20261105)) {
      const again = done(await draw(runtime, frameK(k), refs));
      assert.equal(again.hash, inOrder[k], `frame ${k} after other frames`);
    }
    // After a context-free restart of the whole page nothing remembers: the pictures are a function of (plan, resources).
    const cold = await directedRuntime(paths);
    try {
      const coldRefs = await loadAll(cold, paths);
      assert.equal(done(await draw(cold, frameK(5), coldRefs)).hash, inOrder[5]);
    } finally {
      await cold.close();
    }
  } finally {
    await runtime.close();
  }
});

test('a newer frame supersedes the older one; an explicit cancel answers cancelled and leaves nothing half drawn', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    const refs = await loadAll(runtime, paths);
    const plan = (k) => planOf([['quad', k / 10, 0, 0, 4, 4]]);
    const layerOf = (k) => directedLayer(plan(k), [refs.quad]);
    const alone = done(await runtime.frame([layerOf(3)], { size: SIZE, probes: PROBES }));

    // A burst of seeks: only the last one is drawn, the others come back cancelled.
    const burst = [frameMessage([layerOf(1)], { id: 801 }), frameMessage([layerOf(2)], { id: 802 }), frameMessage([layerOf(3)], { id: 803, inspect: true })];
    await runtime.page.evaluate((messages) => messages.forEach((message) => window.TesselRuntime.receive({ v: 1, ...message })), burst);
    const answers = {};
    for (const id of [801, 802, 803]) {
      answers[id] = await runtime.waitFor((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === id, `frame ${id}`);
    }
    assert.deepEqual([answers[801].error?.code, answers[802].error?.code], ['cancelled', 'cancelled']);
    assert.equal(answers[803].type, 'frame.done');
    assert.equal(answers[803].hash, alone.hash, 'the surviving frame is the picture it is in isolation');

    // An explicit cancel that reaches the runtime before the frame starts.
    const cancelled = frameMessage([layerOf(4)], { id: 810 });
    await runtime.page.evaluate((message) => {
      window.TesselRuntime.receive({ v: 1, ...message });
      window.TesselRuntime.receive({ v: 1, type: 'frame.cancel', id: message.id });
    }, cancelled);
    const answer = await runtime.waitFor((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === 810, 'frame 810');
    assert.equal(answer.error.code, 'cancelled');
    const after = done(await runtime.frame([layerOf(3)], { size: SIZE, probes: PROBES }));
    assert.equal(after.hash, alone.hash, 'a cancelled frame leaves no state that shows in the next one');
  } finally {
    await runtime.close();
  }
});

test('a lost context mid frame answers context_lost; after the restore the same plan draws the same picture without reloading', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    const refs = await loadAll(runtime, paths);
    const plan = planOf([['wall', 0, 0, -2, 8, 4.5, 'opaque'], ['quad', 0.2, 0, 0, 2, 2]]);
    const layer = directedLayer(plan, [refs.wall, refs.quad]);
    const before = done(await runtime.frame([layer], { size: SIZE, probes: PROBES }));

    const failed = await loseDuring(runtime, [layer]);
    assert.equal(failed.type, 'frame.failed', 'a frame on a dead context never reports success');
    assert.equal(failed.error.code, 'context_lost', JSON.stringify(failed.error));
    assert.equal((await glMemory(runtime)).textures, 0, 'nothing of the lost context stays alive');
    // The pictures are CPU-side data of the store: they survive, so the redraw does not fetch again.
    assert.equal((await runtime.retention()).scene_resources, Object.keys(SIZES).length);

    await restore(runtime);
    const after = done(await runtime.frame([layer], { size: SIZE, probes: PROBES }));
    assert.equal(after.hash, before.hash, 'the restored context draws the same picture');
    assert.equal(after.cache.texture_created, 2, 'textures are made again from the bitmaps');
    assert.equal(after.cache.gl_textures, before.cache.gl_textures, 'the same GL objects as before the loss: nothing leaked');

    // Lost between frames, then a frame before the restore: typed, not a black picture.
    await loseNow(runtime);
    const dead = await runtime.frame([layer], { size: SIZE });
    assert.equal(dead.error.code, 'context_lost');
    await restore(runtime);
    assert.equal(done(await runtime.frame([layer], { size: SIZE, probes: PROBES })).hash, before.hash);
  } finally {
    await runtime.close();
  }
});

test('the resource budget is a limit the runtime enforces, with the pictures it already holds left alone', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    await runtime.page.evaluate(() => window.TesselRuntime.debug.setLimits({ sceneResourceEntries: 2 }));
    assert.equal((await runtime.load(sceneAsset('s-1', paths.red, 'red'))).state, 'ready');
    assert.equal((await runtime.load(sceneAsset('s-2', paths.green, 'green'))).state, 'ready');
    const third = await runtime.load(sceneAsset('s-3', paths.wall, 'wall'));
    assert.equal(third.state, 'failed');
    assert.equal(third.error.code, 'resource_budget');
    // Reloading a slot that is already held replaces it; it is not a third resource.
    assert.equal((await runtime.load(sceneAsset('s-2', paths.other, 'green'))).state, 'ready');
    assert.equal((await runtime.retention()).scene_resources, 2);

    // Bytes: decoded RGBA of everything held may not pass the limit.
    await runtime.page.evaluate(() => window.TesselRuntime.debug.setLimits({ sceneResourceEntries: 8, sceneResourceBytes: 4 * 4 * 4 + 8 * 8 * 4 }));
    await runtime.release('s-2');
    assert.equal((await runtime.load(sceneAsset('s-4', paths.other, 'other'))).state, 'ready');
    const over = await runtime.load(sceneAsset('s-5', paths.quad, 'quad'));
    assert.equal(over.error.code, 'resource_budget');
    assert.equal((await runtime.retention()).scene_resources, 2, 'a refused load adds nothing');

    // A resource of a kind that is not a still is not a scene resource.
    const notStill = await runtime.load({ ...sceneAsset('s-6', paths.red, 'red'), kind: 'audio' });
    assert.equal(notStill.error.code, 'unsupported_format');
  } finally {
    await runtime.close();
  }
});

test('asset.state repeats the identity of the load it answers, so a reused slot can tell an old answer from the new one (BL102)', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    const ready = await runtime.hello();
    assert.ok(ready.capabilities.features.includes('asset.identity'));
    assert.deepEqual(ready.capabilities.asset_identity.echo, ['request', 'scene']);

    const states = (slot) => runtime.messages.filter((m) => m.type === 'asset.state' && m.id === slot);
    const settled = (slot, request) => (m) =>
      m.type === 'asset.state' && m.id === slot && m.request === request && m.state !== 'loading';
    const identity = (name) => ({
      asset_id: name,
      revision_id: `${name}-r1`,
      content_hash: `hash-${name}`,
      generation: GENERATION,
    });

    // Every state of one load (loading and ready) repeats the request and the scene identity.
    await runtime.send({ type: 'asset.load', asset: { ...sceneAsset('slot', paths.red, 'red'), request: 11 } });
    const first = await runtime.waitFor(settled('slot', 11), 'ready of request 11');
    assert.equal(first.state, 'ready');
    assert.deepEqual(first.scene, identity('red'));
    assert.deepEqual(
      states('slot').map((m) => [m.state, m.request]),
      [['loading', 11]],
      'the loading state of the same request was echoed too (ready was consumed)',
    );
    runtime.messages.length = 0;

    // Hold request 21 at its real fetch: separate CDP calls alone do not prove it is still
    // loading, especially when the image is cached or the suite is under load (BL102c).
    const gate = await holdFetches(runtime.page, (url) => decodeURIComponent(url).includes(paths.red));
    try {
      await runtime.send({ type: 'asset.load', asset: { ...sceneAsset('slot', paths.red, 'red'), request: 21 } });
      await gate.held();
      await runtime.send({ type: 'asset.load', asset: { ...sceneAsset('slot', paths.green, 'green'), request: 22 } });
      const replacing = await runtime.waitFor(settled('slot', 22), 'ready of request 22');
      assert.equal(replacing.state, 'ready');
      assert.deepEqual(replacing.scene, identity('green'));
      await gate.open();
      const ended = await runtime.waitFor(settled('slot', 21), 'end of request 21');
      assert.deepEqual(
        [ended.state, ended.error.code, ended.scene.asset_id],
        ['failed', 'cancelled', 'red'],
        'the replaced load ends as cancelled, naming its own request and picture, never as a ready',
      );
      assert.equal(states('slot').filter((m) => m.request === 21 && m.state === 'ready').length, 0);
    } finally {
      await gate.close();
    }

    // The same picture loaded into the same slot again is a different request.
    runtime.messages.length = 0;
    await runtime.send({ type: 'asset.load', asset: { ...sceneAsset('slot', paths.green, 'green'), request: 23 } });
    assert.equal((await runtime.waitFor(settled('slot', 23), 'ready of request 23')).request, 23);

    // A failure carries the identity as well.
    runtime.messages.length = 0;
    await runtime.send({ type: 'asset.load', asset: { ...sceneAsset('gone', `${paths.red}.missing`, 'red'), request: 31 } });
    const failed = await runtime.waitFor((m) => m.type === 'asset.state' && m.id === 'gone' && m.state === 'failed', 'failed load');
    assert.equal(failed.request, 31);
    assert.deepEqual(failed.scene, identity('red'));

    // A request that is not a non-negative integer is refused as a load, with no echo to trust.
    runtime.messages.length = 0;
    await runtime.send({ type: 'asset.load', asset: { ...sceneAsset('bad', paths.red, 'red'), request: -1 } });
    const invalid = await runtime.waitFor((m) => m.type === 'asset.state' && m.id === 'bad', 'refused request');
    assert.deepEqual([invalid.state, invalid.error.code, invalid.request], ['failed', 'invalid_request', undefined]);
    assert.equal(runtime.messages.some((m) => m.type === 'asset.state' && m.id === 'bad' && m.state === 'ready'), false);
  } finally {
    await runtime.close();
  }
});

test('a load without identity gets the asset.state messages it always got (bridge v1 stays compatible)', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = files(t);
  const runtime = await directedRuntime(paths);
  try {
    runtime.messages.length = 0;
    const loaded = await runtime.load({ id: 'plain', kind: 'still', source: { type: 'file', path: paths.red }, meta: {} });
    assert.deepEqual(Object.keys(loaded).sort(), ['id', 'meta', 'state', 'type', 'v']);
    const loading = runtime.messages.find((m) => m.type === 'asset.state' && m.id === 'plain' && m.state === 'loading');
    assert.deepEqual(Object.keys(loading).sort(), ['id', 'state', 'type', 'v']);

    // A scene load of a host that never learned `request` still works, and is not echoed a request.
    const scene = await runtime.load(sceneAsset('slot', paths.green, 'green'));
    assert.equal(scene.state, 'ready');
    assert.equal(scene.request, undefined);
  } finally {
    await runtime.close();
  }
});
