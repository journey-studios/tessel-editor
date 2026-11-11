// BL157.3 (second, separate experiment): the opt-in warm DRAW. After the warm sort of a splat scene a hello that asked for it
// (`warm_draw: true`) gets one draw of that scene into a scratch target, so the GPU's first-use work happens before the cut.
// Real headless Chrome (SwiftShader), through the bridge messages the desktop uses. Every claim is counted where it happens
// (the `renderer.render` calls and their target, the GL memory three.js believes alive) and every pixel claim compares exact
// hashes with a runtime that never drew ahead. Skipped, loudly, without Chrome or the bundle (TESSEL_REQUIRE_RUNTIME=1 fails).
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { PREVIEW, SAME_MSAA, at, draw, kit, open, reference as exactHash, withRuntime } from './splat-kit.mjs';
import { plan, skipUnavailable } from './support.mjs';

const SIZE = [320, 180];
const ON = { ...SAME_MSAA, warm_draw: true };
const SCRATCH_WIDTH = 16;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Records the width of the target of every `renderer.render` (0: the canvas). */
const watchRenders = (runtime) =>
  runtime.page.evaluate(() => {
    const renderer = window.TesselRuntime.debug.compositor().renderer;
    window.__renders = [];
    const original = renderer.render.bind(renderer);
    renderer.render = (scene, camera) => {
      const target = renderer.getRenderTarget();
      window.__renders.push(target?.width ?? 0);
      if (target?.width === 16) window.__scratch = target;
      return original(scene, camera);
    };
  });
const scratchDraws = (runtime) => runtime.page.evaluate((width) => window.__renders.filter((w) => w === width).length, SCRATCH_WIDTH);
/** The scratch target of the last warm draw is freed: three.js holds nothing for its texture or its framebuffer. */
const scratchFreed = (runtime) =>
  runtime.page.evaluate(() => {
    const renderer = window.TesselRuntime.debug.compositor().renderer;
    const target = window.__scratch;
    return Boolean(target) && !renderer.properties.has(target.texture) && !renderer.properties.has(target);
  });
const status = (runtime) => runtime.page.evaluate(() => window.TesselRuntime.debug.warmDraw());
const memory = (runtime) => runtime.page.evaluate(() => window.TesselRuntime.debug.glMemory());
const canvasPixels = (runtime) => runtime.page.evaluate(() => document.querySelector('canvas').toDataURL('image/png'));

const sendWarm = async (runtime, layers) => {
  await runtime.send({ type: 'frame.warm', plan: plan(layers, SIZE) });
  await runtime.page.evaluate(() => window.TesselRuntime.debug.idle());
};

/** Waits until the sorts have finished and every warm draw asked for has an outcome. */
const settled = async (runtime) => {
  for (let attempt = 0; attempt < 400; attempt += 1) {
    await runtime.page.evaluate(() => window.TesselRuntime.debug.idle());
    const warming = await runtime.page.evaluate(() => window.TesselRuntime.debug.warming());
    const s = await status(runtime);
    if (warming === 0 && s.requested === s.drawn + s.skipped + s.expired) return s;
    await sleep(25);
  }
  return assert.fail('the warm and its draw never finished');
};

const warm = async (runtime, layers) => {
  await sendWarm(runtime, layers);
  return settled(runtime);
};

/** Makes the first sort of the stage a warm creates for `scene` wait for `release()` (the warm is then in flight as long as a test needs). */
const holdWarmSort = (runtime, scene) =>
  runtime.page.evaluate((name) => {
    const compositor = window.TesselRuntime.debug.compositor();
    let release;
    window.__gate = { promise: new Promise((resolve) => (release = resolve)), release };
    const stageOf = compositor.stageOf.bind(compositor);
    compositor.stageOf = (lineage) => {
      const fresh = lineage !== null && !compositor.stages.entries.has(lineage);
      const stage = stageOf(lineage);
      if (fresh && lineage.startsWith(`${name}\u0000`)) {
        const original = stage.spark.readbackDepth.bind(stage.spark);
        stage.spark.readbackDepth = (...args) => window.__gate.promise.then(() => original(...args));
      }
      return stage;
    };
  }, scene);
const releaseGate = (runtime) => runtime.page.evaluate(() => window.__gate.release());
const holdQueue = (runtime) =>
  runtime.page.evaluate(() => {
    window.__release = window.TesselRuntime.debug.holdQueue();
  });
const releaseQueue = (runtime) => runtime.page.evaluate(() => window.__release());
const stageCount = (runtime) => runtime.page.evaluate(() => window.TesselRuntime.debug.splatStages().length);
const poll = async (read, done, label) => {
  for (let attempt = 0; attempt < 400; attempt += 1) {
    const value = await read();
    if (done(value)) return value;
    await sleep(25);
  }
  return assert.fail(`${label} never happened`);
};

test('off by default: the hello that does not ask announces nothing, a warm draws nothing ahead and records nothing', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b']);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await watchRenders(runtime);
    await warm(runtime, [at('scene-a', -2)]);
    assert.equal(await stageCount(runtime), 1, 'the warm sort itself still runs');
    assert.equal(await scratchDraws(runtime), 0);
    const s = await status(runtime);
    assert.deepEqual([s.enabled, s.requested, s.drawn], [false, 0, 0]);
  });
  const empty = await open(files, SAME_MSAA);
  try {
    // The announcement is a function of the hello alone.
    const ready = await empty.hello(SAME_MSAA);
    assert.ok(!ready.capabilities.features.includes('scene.warm_draw'));
    const asked = await empty.hello(ON);
    assert.ok(asked.capabilities.features.includes('scene.warm_draw'));
    const again = await empty.hello(SAME_MSAA);
    assert.ok(!again.capabilities.features.includes('scene.warm_draw'), 'a later hello that does not ask turns it off');
  } finally {
    await empty.close();
  }
});

test('on: one draw into a scratch target after the sort, none repeated, and the stage, meshes and order are the warm\'s own', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b']);
  const exact = await exactHash(files, [at('scene-a', 2)]);
  // The GL objects of the same warm and first frame without the draw: what the draw must not add to.
  const withoutDraw = await withRuntime(files, SAME_MSAA, async (runtime) => {
    await warm(runtime, [at('scene-a', -2)]);
    await draw(runtime, [at('scene-a', -1)], PREVIEW);
    return memory(runtime);
  });
  await withRuntime(files, ON, async (runtime) => {
    await watchRenders(runtime);
    const s = await warm(runtime, [at('scene-a', -2)]);
    assert.deepEqual([s.enabled, s.requested, s.drawn, s.skipped], [true, 1, 1, 0]);
    assert.deepEqual(s.outcomes, { drawn: 1 });
    assert.equal(await scratchDraws(runtime), 1, 'exactly one draw, into the 16 px target');
    assert.equal(await stageCount(runtime), 1);
    assert.ok(await scratchFreed(runtime), 'the scratch target and its texture are freed');

    await warm(runtime, [at('scene-a', -2)]);
    assert.equal(await scratchDraws(runtime), 1, 'asking again for a lineage that has its stage draws nothing');

    // The frame at the cut finds the warm's stage and meshes, as it does without the draw.
    const first = await draw(runtime, [at('scene-a', -1)], PREVIEW);
    assert.equal(first.cache.stage_created, 0);
    assert.equal(first.cache.splat_mesh_created, 0);
    assert.equal(first.source.stale_sort_layers, 1, 'drawn at once with the order the warm sorted');
    // What the draw made lazily (Spark's first-use resources) is what the frame makes for itself without it: the same count.
    const now = await memory(runtime);
    assert.equal(now.textures, withoutDraw.textures);
    assert.equal(now.geometries, withoutDraw.geometries);
    const final = await draw(runtime, [at('scene-a', 2)]);
    assert.equal(final.hash, exact, 'the exact frame is the picture of a runtime that never drew ahead');
  });
});

test('on and off draw the same pictures: hot (preview) frame, exact frame and a cold scene, hash for hash', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b']);
  const hashes = async (hello) =>
    withRuntime(files, hello, async (runtime) => {
      await warm(runtime, [at('scene-a', -2)]);
      const hot = await draw(runtime, [at('scene-a', -1)], PREVIEW);
      const exact = await draw(runtime, [at('scene-a', 2)]);
      const cold = await draw(runtime, [at('scene-b', 1)]);
      return [hot.hash, exact.hash, cold.hash];
    });
  const off = await hashes(SAME_MSAA);
  const on = await hashes(ON);
  assert.deepEqual(on, off);
  assert.notEqual(off[0], off[1], 'the hot and the exact frame are different pictures here: the comparison can tell them apart');
});

test('the canvas and the retained targets are not touched by the draw', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b']);
  await withRuntime(files, ON, async (runtime) => {
    await draw(runtime, [at('scene-b', 1)]);
    const pixels = await canvasPixels(runtime);
    const retained = await runtime.retention();
    await watchRenders(runtime);
    await warm(runtime, [at('scene-a', -2)]);
    assert.equal(await scratchDraws(runtime), 1);
    assert.equal(await canvasPixels(runtime), pixels, 'the picture on the canvas is the one before the warm');
    const now = await runtime.retention();
    for (const key of ['target_entries', 'target_bytes', 'rig_entries', 'material_entries']) {
      if (key in retained) assert.equal(now[key], retained[key], key);
    }
    assert.equal(now.counters.target_created, retained.counters.target_created, 'no retained target was made for it');
    assert.equal(now.counters.target_evicted, retained.counters.target_evicted);
  });
});

test('a frame that is waiting goes first, takes the lineage over, and the draw then does nothing', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a']);
  await withRuntime(files, ON, async (runtime) => {
    await watchRenders(runtime);
    await holdWarmSort(runtime, 'scene-a');
    await sendWarm(runtime, [at('scene-a', -2)]);
    await holdQueue(runtime);
    await releaseGate(runtime);
    await poll(() => runtime.page.evaluate(() => window.TesselRuntime.debug.warming()), (n) => n === 0, 'the sort');
    // The sort is done and the draw is queued behind the held queue; a frame arrives before it can run.
    const frame = draw(runtime, [at('scene-a', -1)], PREVIEW);
    await sleep(100);
    await releaseQueue(runtime);
    const done = await frame;
    assert.equal(done.cache.stage_created, 0);
    assert.equal(done.cache.splat_mesh_created, 0);
    const s = await settled(runtime);
    assert.deepEqual(s.outcomes, { taken: 1 }, 'the frame took the lineage over: the draw is a no-op');
    assert.equal(await scratchDraws(runtime), 0);
  });
});

test('a lost context drops a queued draw; the next frame is the picture of a runtime that never lost anything', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a']);
  const exact = await exactHash(files, [at('scene-a', 2)]);
  await withRuntime(files, ON, async (runtime) => {
    await watchRenders(runtime);
    await holdWarmSort(runtime, 'scene-a');
    await sendWarm(runtime, [at('scene-a', -2)]);
    await holdQueue(runtime);
    await releaseGate(runtime);
    await poll(() => runtime.page.evaluate(() => window.TesselRuntime.debug.warming()), (n) => n === 0, 'the sort');
    await runtime.page.evaluate(() => window.TesselRuntime.debug.loseContext());
    await runtime.waitFor((m) => m.type === 'context.lost', 'context.lost');
    await releaseQueue(runtime);
    const s = await settled(runtime);
    assert.equal(s.drawn, 0);
    assert.equal(await scratchDraws(runtime), 0, 'nothing was drawn on a lost context');
    assert.equal(runtime.messages.filter((m) => m.type === 'error').length, 0);
    await runtime.page.evaluate(() => window.TesselRuntime.debug.restoreContext());
    await runtime.waitFor((m) => m.type === 'context.restored', 'context.restored');
    const done = await draw(runtime, [at('scene-a', 2)]);
    assert.equal(done.hash, exact);
  });
});

test('a scene released before its draw is skipped without an error and leaves nothing alive', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a']);
  const withoutDraw = await withRuntime(files, SAME_MSAA, async (runtime) => {
    await warm(runtime, [at('scene-a', -2)]);
    await runtime.release('scene-a');
    await runtime.release('splat-a');
    await runtime.page.evaluate(() => window.TesselRuntime.debug.idle());
    return memory(runtime);
  });
  await withRuntime(files, ON, async (runtime) => {
    await watchRenders(runtime);
    await holdWarmSort(runtime, 'scene-a');
    await sendWarm(runtime, [at('scene-a', -2)]);
    await holdQueue(runtime);
    await releaseGate(runtime);
    await poll(() => runtime.page.evaluate(() => window.TesselRuntime.debug.warming()), (n) => n === 0, 'the sort');
    await runtime.release('scene-a');
    await runtime.release('splat-a');
    await releaseQueue(runtime);
    const s = await settled(runtime);
    assert.equal(s.drawn, 0);
    assert.equal(await scratchDraws(runtime), 0);
    assert.equal(runtime.messages.filter((m) => m.type === 'error').length, 0);
    assert.equal(await stageCount(runtime), 0, 'the release took the stage with it');
    const after = await memory(runtime);
    assert.equal(after.textures, withoutDraw.textures, 'the same GL objects as the same release without a draw');
  });
});

test('the stage budget is the warm\'s: a third scene gets no stage and so no draw', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b', 'c']);
  await withRuntime(files, ON, async (runtime) => {
    await watchRenders(runtime);
    const s = await warm(runtime, [at('scene-a', 0), at('scene-b', 0), at('scene-c', 0)]);
    assert.equal(await stageCount(runtime), 2);
    assert.equal(s.requested, 2);
    assert.equal(await scratchDraws(runtime), 2);
  });
});

test('a draw that waited past its age is dropped unrun', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a']);
  await withRuntime(files, ON, async (runtime) => {
    await watchRenders(runtime);
    await holdWarmSort(runtime, 'scene-a');
    await sendWarm(runtime, [at('scene-a', -2)]);
    await holdQueue(runtime);
    await releaseGate(runtime);
    await poll(() => runtime.page.evaluate(() => window.TesselRuntime.debug.warming()), (n) => n === 0, 'the sort');
    await sleep(3200);
    await releaseQueue(runtime);
    const s = await settled(runtime);
    assert.deepEqual(s.outcomes, { expired: 1 });
    assert.equal(await scratchDraws(runtime), 0);
  });
});
