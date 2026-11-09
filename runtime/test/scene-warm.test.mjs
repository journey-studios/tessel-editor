// BL157: `frame.warm` gives the splat scenes of a plan their Spark stage and first sort before a frame draws them, so the
// frame at a cut finds a stage that holds the scene instead of waiting for its own sort. Real headless Chrome (SwiftShader)
// through the bridge messages the desktop uses; skipped, loudly, without Chrome or the bundle
// (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { PREVIEW, SAME_MSAA, at, draw, kit, open, reference as exactHash, withRuntime } from './splat-kit.mjs';
import { plan, skipUnavailable } from './support.mjs';

const SIZE = [320, 180];

/** Sends `frame.warm` for `layers` and waits until the sorts it started have finished. */
const warm = async (runtime, layers) => {
  await runtime.send({ type: 'frame.warm', plan: plan(layers, SIZE) });
  await runtime.page.evaluate(() => window.TesselRuntime.debug.idle());
  for (let attempt = 0; attempt < 400; attempt += 1) {
    if ((await runtime.page.evaluate(() => window.TesselRuntime.debug.warming())) === 0) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  assert.fail('the warm sort never finished');
};

const stages = (runtime) => runtime.page.evaluate(() => window.TesselRuntime.debug.splatStages().length);

test('the runtime announces the warm, and a warmed scene is drawn by its stage from the first frame', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b']);
  const exact = await exactHash(files, [at('scene-a', 2)]);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    const ready = await runtime.hello(SAME_MSAA);
    assert.ok(ready.capabilities.features.includes('scene.warm'));

    await warm(runtime, [at('scene-a', -2)]);
    assert.equal(await stages(runtime), 1, 'the stage exists before any frame');

    const first = await draw(runtime, [at('scene-a', -1)], PREVIEW);
    assert.equal(first.cache.stage_created, 0, 'the frame builds no stage');
    assert.equal(first.cache.splat_mesh_created, 0, 'and creates no mesh: the warm lent them back for the same owner');
    assert.equal(first.source.stale_sort_layers, 1, 'it is drawn at once with the order the warm sorted, not after its own sort');

    // The exact frame (explicit, export, refine) is the picture of a runtime that never warmed anything.
    const final = await draw(runtime, [at('scene-a', 2)]);
    assert.equal(final.source.stale_sort_layers, 0);
    assert.equal(final.hash, exact);
  });
});

test('a scene that was not warmed still waits for its own sort, and the one that was does not', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b']);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await warm(runtime, [at('scene-a', -2)]);
    const mixed = await draw(
      runtime,
      [at('scene-a', -1), at('scene-b', -1)].map((layer) => ({ ...layer, opacity: 0.5 })),
      PREVIEW,
    );
    assert.equal(mixed.cache.stage_created, 1, 'only the cold scene makes a stage');
    assert.equal(mixed.source.stale_sort_layers, 1, 'the warm one reuses its order, the cold one is exact');
  });
});

test('a warm never frees a stage for itself, never starts a load and repeating it does nothing', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b', 'c']);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await warm(runtime, [at('scene-a', 0), at('scene-b', 0), at('scene-c', 0)]);
    assert.equal(await stages(runtime), 2, 'the budget (2): the third scene is left for its own frame');
    await warm(runtime, [at('scene-a', 0), at('scene-b', 0), at('scene-c', 0)]);
    assert.equal(await stages(runtime), 2, 'asking again changes nothing');

    // A scene whose asset is not loaded is skipped without a message or a load.
    const empty = await open(kit(['a']));
    try {
      await empty.release('splat-a');
      await warm(empty, [at('scene-a', 0)]);
      assert.equal(await stages(empty), 0);
      assert.equal(empty.messages.filter((message) => message.type === 'error').length, 0);
    } finally {
      await empty.close();
    }
  });
});

test('a warm takes the slot of a stage the last frame did not draw, and never of one it did', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b', 'c']);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    // Scene a ended, scene b is on screen: the cut to c finds a's idle stage in the way.
    await draw(runtime, [at('scene-a', 0)], PREVIEW);
    await draw(runtime, [at('scene-b', 0)], PREVIEW);
    assert.equal(await stages(runtime), 2);
    await warm(runtime, [at('scene-c', -1)]);
    assert.equal(await stages(runtime), 2, 'a was freed for c');
    const cut = await draw(runtime, [at('scene-b', 1), at('scene-c', 1)].map((layer) => ({ ...layer, opacity: 0.5 })), PREVIEW);
    assert.equal(cut.cache.stage_created, 0, 'b kept its stage and c arrives warm');
    assert.ok(cut.source.stale_sort_layers >= 1, 'c is drawn at once with the order the warm sorted');

    // Both stages drawn by the last frame: nothing is freed, the scene is left for its own frame.
    await warm(runtime, [at('scene-a', -1)]);
    const again = await draw(runtime, [at('scene-b', 2), at('scene-c', 2)].map((layer) => ({ ...layer, opacity: 0.5 })), PREVIEW);
    assert.equal(again.cache.stage_created, 0, 'the stages on screen were not touched');
  });
});

// ---- What a warm in flight must not disturb (BL157 review) ----------------------------------------------------------------

const STACKED = (layers) => layers.map((layer) => ({ ...layer, opacity: 0.5 }));

/**
 * Makes the first sort of the stage a warm creates for `scene` wait for `release()`: the warm is then in flight for as
 * long as a test needs. The wait is in the depth readback Spark's sort awaits, so it is the sort that is slow, as a big
 * scene's is, and nothing about how the runtime starts or tracks it is replaced. Returns the gate's number (a second
 * hold, after a lost context, gets its own).
 */
const holdWarmSort = (runtime, scene, gate = 1) =>
  runtime.page.evaluate(
    ({ scene: name, gate: number }) => {
      const compositor = window.TesselRuntime.debug.compositor();
      window.__gates ??= {};
      let release;
      window.__gates[number] = { promise: new Promise((resolve) => (release = resolve)), release };
      const stageOf = compositor.stageOf.bind(compositor);
      compositor.stageOf = (lineage) => {
        const fresh = lineage !== null && !compositor.stages.entries.has(lineage);
        const stage = stageOf(lineage);
        if (fresh && lineage.startsWith(`${name}\u0000`)) {
          const original = stage.spark.readbackDepth.bind(stage.spark);
          stage.spark.readbackDepth = (...args) => window.__gates[number].promise.then(() => original(...args));
        }
        return stage;
      };
    },
    { scene, gate },
  );
const releaseGate = (runtime, gate = 1) => runtime.page.evaluate((number) => window.__gates[number].release(), gate);
const sendWarm = async (runtime, layers) => {
  await runtime.send({ type: 'frame.warm', plan: plan(layers, SIZE) });
  await runtime.page.evaluate(() => window.TesselRuntime.debug.idle());
};
const warming = (runtime) => runtime.page.evaluate(() => window.TesselRuntime.debug.warming());
const settled = async (runtime) => {
  for (let attempt = 0; attempt < 400; attempt += 1) {
    if ((await warming(runtime)) === 0) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  assert.fail('the warm sort never finished');
};

test('a frame that needs a new scene while a warm is sorting keeps the stage budget and the warm stage', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b', 'c']);
  const crossfade = STACKED([at('scene-a', 1), at('scene-b', 1)]);
  const exact = await exactHash(files, crossfade);
  const exactC = await exactHash(files, [at('scene-c', 2)]);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await draw(runtime, [at('scene-a', 0)], PREVIEW);
    await holdWarmSort(runtime, 'scene-c');
    await sendWarm(runtime, [at('scene-c', -1)]);
    assert.equal(await warming(runtime), 1, 'the warm of c is sorting');
    const before = await runtime.retention();
    assert.equal(before.stage_entries, 2, 'a (on screen) and c (warming) fill the budget');

    // a -> b with b new: b may not take a third stage and may not cost c its slot.
    const cut = await draw(runtime, crossfade, PREVIEW);
    const during = await runtime.retention();
    assert.equal(during.stage_entries, 2, 'the bound holds: b took the shared stage, not a third one');
    assert.equal(during.counters.stage_evicted, before.counters.stage_evicted, 'c was not freed under its sort');
    assert.equal(await warming(runtime), 1, 'and its sort is still running');
    assert.ok(cut.source.stale_sort_layers <= 1);

    await releaseGate(runtime);
    await settled(runtime);
    // c's warm survived the frame: the cut into c finds its stage, and the exact pictures are the cold ones.
    const intoC = await draw(runtime, [at('scene-c', 0.5)], PREVIEW);
    assert.equal(intoC.cache.stage_created, 0, 'the warmed stage was still there');
    assert.equal(intoC.source.stale_sort_layers, 1, 'drawn at once with the order the warm sorted');
    assert.equal((await draw(runtime, crossfade)).hash, exact);
    assert.equal((await draw(runtime, [at('scene-c', 2)])).hash, exactC);
  });
});

test('a frame that waits on a warm sort stops waiting when a newer frame replaces it', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'c']);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await holdWarmSort(runtime, 'scene-c');
    await sendWarm(runtime, [at('scene-c', -1)]);
    assert.equal(await warming(runtime), 1);
    await runtime.page.evaluate(() => {
      const compositor = window.TesselRuntime.debug.compositor();
      const awaitWarm = compositor.awaitWarm.bind(compositor);
      window.__waiting = false;
      compositor.awaitWarm = (...args) => {
        window.__waiting = true;
        return awaitWarm(...args);
      };
    });
    // Frame 100 needs c and waits for its warm; the gate stays shut, so only a cancel can end the wait early.
    await runtime.send({ type: 'frame.render', id: 100, revision: 100, plan: plan([at('scene-c', 0)], SIZE), output: { width: SIZE[0], height: SIZE[1] }, quality: 'preview' });
    await runtime.page.waitForFunction(() => window.__waiting === true, { timeout: 10_000 });
    const sent = Date.now();
    const newer = await draw(runtime, [at('scene-a', 0)], PREVIEW);
    const waited = Date.now() - sent;
    assert.ok(newer.type === 'frame.done');
    const old = await runtime.waitFor((message) => message.id === 100, 'the superseded frame');
    assert.equal(old.type, 'frame.failed');
    assert.equal(old.error.code, 'cancelled', 'the frame a newer one replaced is reported as cancelled');
    assert.ok(waited < 600, `the newer frame was held ${waited} ms behind a wait that a cancel must end (the bound is 1000 ms)`);
    assert.equal(await warming(runtime), 1, 'the warm itself is untouched');
    await releaseGate(runtime);
    await settled(runtime);
  });
});

test('two layers of one scene are numbered alike by the warm and by the frame, covered or not', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a']);
  // The scene twice, the lower copy covered by a layer above: the visible one is the second lineage of the scene.
  const layers = [{ ...at('scene-a', -2), occluded: true }, at('scene-a', -1)];
  const exact = await exactHash(files, layers);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await warm(runtime, layers);
    assert.equal(await stages(runtime), 1, 'one stage, for the layer that is drawn');
    const first = await draw(runtime, layers, PREVIEW);
    assert.equal(first.cache.stage_created, 0, 'the frame draws with the lineage the warm sorted');
    assert.equal(first.source.stale_sort_layers, 1, 'at once, with the warmed order, not after a sort of its own');
    assert.equal(first.source.occluded_layers, 1);
    assert.equal((await draw(runtime, layers)).hash, exact);
  });
});

test('a warm takes no spare mesh from the scene on screen and the scene keeps its order', async (t) => {
  if (skipUnavailable(t, assert)) return;
  // Scenes a and c both draw the effect cloud `splat-fx`: a warm for c finds a's spare mesh of it in the pool.
  const files = kit(['a', 'c']);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await draw(runtime, [at('scene-a', -2)], PREVIEW);
    const owners = () =>
      runtime.page.evaluate(() => {
        const entry = window.TesselRuntime.debug.assets.entries.get('splat-fx');
        return entry.pool.free.map((mesh) => mesh.leaseOwner);
      });
    assert.deepEqual(await owners(), ['scene-a\u00000#1'], 'a holds the only spare mesh of the effect cloud');
    await warm(runtime, [at('scene-c', -1)]);
    assert.ok((await owners()).includes('scene-a\u00000#1'), 'the warm of c built its own mesh instead of taking a\'s');
    const next = await draw(runtime, [at('scene-a', -1.2)], PREVIEW);
    assert.equal(next.source.stale_sort_layers, 1, 'a still draws with the order it sorted: its meshes are the ones it sorted');
  });
});

test('a context lost while a warm sorts leaves nothing behind, and the old sort cannot undo a newer warm', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'c']);
  const exactC = await exactHash(files, [at('scene-c', 2)]);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await runtime.page.evaluate(() => {
      window.__unhandled = [];
      window.addEventListener('unhandledrejection', (event) => window.__unhandled.push(String(event.reason?.message ?? event.reason)));
    });
    await draw(runtime, [at('scene-a', 0)], PREVIEW);
    await holdWarmSort(runtime, 'scene-c', 1);
    await sendWarm(runtime, [at('scene-c', -1)]);
    assert.equal(await warming(runtime), 1);

    await runtime.page.evaluate(() => window.TesselRuntime.debug.loseContext());
    await runtime.waitFor((message) => message.type === 'context.lost', 'context.lost');
    await runtime.page.evaluate(() => window.TesselRuntime.debug.restoreContext());
    await runtime.waitFor((message) => message.type === 'context.restored', 'context.restored');
    assert.equal(await warming(runtime), 0, 'the lost context cleared the warm book');
    assert.equal((await runtime.retention()).stage_entries, 0, 'and its stages');

    // The scene is warmed again on the new context; the sort of the old one then ends, and must not remove the new warm.
    await draw(runtime, [at('scene-a', 0)], PREVIEW);
    await holdWarmSort(runtime, 'scene-c', 2);
    await sendWarm(runtime, [at('scene-c', -1)]);
    assert.equal(await warming(runtime), 1, 'the new warm is sorting');
    await releaseGate(runtime, 1);
    await new Promise((resolve) => setTimeout(resolve, 300));
    assert.equal(await warming(runtime), 1, 'the end of the old sort did not delete the new warm');
    assert.equal((await runtime.retention()).stage_entries, 2, 'nor its stage');
    await releaseGate(runtime, 2);
    await settled(runtime);

    const cut = await draw(runtime, [at('scene-c', 0.5)], PREVIEW);
    assert.equal(cut.cache.stage_created, 0, 'the new warm served the cut');
    assert.equal((await draw(runtime, [at('scene-c', 2)])).hash, exactC, 'the exact picture is the one of a runtime that never warmed');
    assert.deepEqual(await runtime.page.evaluate(() => window.__unhandled), [], 'no rejection was left unhandled');
  });
});
