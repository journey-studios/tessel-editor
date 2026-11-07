// BL130: a Spark sort that fails is a failure the frame hears, never a silent state. Real headless Chrome
// (SwiftShader) through the bridge messages the desktop uses; skipped, loudly, without Chrome or the bundle
// (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
//
// Spark 2.4.0's `driveSort` sets `sorting = true` and awaits the depth readback and the sort worker with no
// cleanup: a rejection of either leaves `sorting` true for good. The runtime does not touch that flag; it records
// the failure on the stage, the next request for the stage gets it at once (an exact one never waits out its
// 60 s budget for an error that already happened), and the stage is thrown away so the request after that sorts
// from a new SparkRenderer. The failures are injected where the reviewer injected them: `readbackDepth` of the
// SparkRenderer in the page.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { PREVIEW, SAME_MSAA, at, draw, kit, reference, withRuntime } from './splat-kit.mjs';
import { plan, skipUnavailable } from './support.mjs';

const SIZE = [320, 180];

/** Counts unhandled promise rejections of the page from now on. */
const watchRejections = (runtime) =>
  runtime.page.evaluate(() => {
    window.__unhandled = [];
    window.addEventListener('unhandledrejection', (event) => window.__unhandled.push(String(event.reason?.message ?? event.reason)));
  });
const rejections = (runtime) => runtime.page.evaluate(() => window.__unhandled);

/**
 * Makes the depth readback of the SparkRenderers (`only`: the last one = the shared stage; default all) fail after
 * `allowed` healthy calls: it rejects, or with `hang` never answers.
 */
const breakReadback = (runtime, { allowed = 0, hang = false, only = 'all', maxWaitMs } = {}) =>
  runtime.page.evaluate(
    (options) => {
      const stages = window.TesselRuntime.debug.splatStages();
      for (const stage of options.only === 'shared' ? stages.slice(-1) : stages) {
        const original = stage.spark.readbackDepth.bind(stage.spark);
        let calls = 0;
        stage.spark.readbackDepth = (...args) => {
          calls += 1;
          if (calls <= options.allowed) return original(...args);
          if (options.hang) return new Promise(() => {});
          return Promise.reject(new Error('injected readback failure'));
        };
        if (options.maxWaitMs) stage.maxWaitMs = options.maxWaitMs;
      }
    },
    { allowed, hang, only, maxWaitMs },
  );

/** Resolves once a stage has recorded a failure (the sort Spark started behind a frame is asynchronous). */
const failureRecorded = (runtime) =>
  runtime.page.waitForFunction(() => window.TesselRuntime.debug.splatStages().some((stage) => stage.failed), { timeout: 30_000 });

const failed = (message, text = 'injected readback failure') => {
  assert.equal(message.type, 'frame.failed', `a failed sort is a failed frame, not a frame drawn from it: ${JSON.stringify(message)}`);
  assert.equal(message.error.code, 'render_failed');
  assert.ok(message.error.message.includes(text), message.error.message);
};

test('a sort that rejects behind a moving frame fails the next request, and the one after sorts from a new stage', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const exact = await reference(files, [at('scene-a', 2)]);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await watchRejections(runtime);
    assert.equal((await draw(runtime, [at('scene-a', -2)], PREVIEW)).source.stale_sort_layers, 0);
    assert.equal((await draw(runtime, [at('scene-a', -1.2)], PREVIEW)).source.stale_sort_layers, 1, 'two healthy frames');
    // From here every depth readback rejects (the one Spark already started finishes): the moving frames below
    // cannot tell, because they draw with the order they have and the sort runs behind them.
    await breakReadback(runtime);
    await draw(runtime, [at('scene-a', -0.4)], PREVIEW).catch(() => {});
    await failureRecorded(runtime);
    // The next request hears it, instead of drawing yet another frame with the order of a sort that can never finish.
    const next = await runtime.frame([at('scene-a', 0.4)], { size: SIZE, ...PREVIEW });
    failed(next);
    const afterFailure = await runtime.retention();
    assert.equal(afterFailure.counters.stage_discarded, 1, 'the broken stage is thrown away');
    assert.equal(afterFailure.stage_entries, 0, 'and not kept for the frame after');
    // Recovery: a new stage, sorted for its own camera first (nothing of the broken one is reused)...
    const recovered = await draw(runtime, [at('scene-a', 0.4)], PREVIEW);
    assert.equal(recovered.source.stale_sort_layers, 0);
    assert.equal(recovered.cache.stage_created, 1);
    // ...and then moving frames reuse its order again.
    assert.equal((await draw(runtime, [at('scene-a', 1.2)], PREVIEW)).source.stale_sort_layers, 1);
    const final = await draw(runtime, [at('scene-a', 2)]);
    assert.equal(final.hash, exact, 'the exact frame after the failure is the one a runtime that never failed draws');
    assert.deepEqual(await rejections(runtime), [], 'no unhandled rejection from the failed sort');
  });
});

test('an exact frame after a failed sort gets the failure at once, not after waiting out the sort budget', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const exact = await reference(files, [at('scene-a', 2)]);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await watchRejections(runtime);
    await draw(runtime, [at('scene-a', -2)], PREVIEW);
    await breakReadback(runtime);
    await draw(runtime, [at('scene-a', -1)], PREVIEW).catch(() => {});
    await failureRecorded(runtime);
    const started = performance.now();
    // An explicit frame (render.frame, export, the refine): no `quality`.
    failed(await runtime.frame([at('scene-a', 2)], { size: SIZE }));
    // 60 s is what it waited for a sort that could not finish; the error has already happened.
    assert.ok(performance.now() - started < 30_000, `${Math.round(performance.now() - started)} ms`);
    assert.equal((await draw(runtime, [at('scene-a', 2)])).hash, exact, 'recovered, exact');
    assert.deepEqual(await rejections(runtime), []);
  });
});

test('an exact frame whose own sort rejects fails with that error and leaves nothing broken behind', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const exact = await reference(files, [at('scene-a', 1)]);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await watchRejections(runtime);
    await draw(runtime, [at('scene-a', -1)]);
    await breakReadback(runtime);
    failed(await runtime.frame([at('scene-a', 1)], { size: SIZE }));
    assert.equal((await runtime.retention()).stage_entries, 0);
    assert.equal((await draw(runtime, [at('scene-a', 1)])).hash, exact);
    assert.deepEqual(await rejections(runtime), []);
  });
});

test('a failure Spark reports on its own (the sort it chains or starts from a timer) is recorded, not rethrown unhandled', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await watchRejections(runtime);
    await draw(runtime, [at('scene-a', -1)]);
    // `dropErrorIfDisposed` is where Spark 2.4.0 sends the error of a sort nobody awaits; it rethrows into a promise
    // nobody holds. Called here as Spark calls it.
    const reported = await runtime.page.evaluate(() => {
      const [stage] = window.TesselRuntime.debug.splatStages();
      stage.spark.dropErrorIfDisposed(new Error('late sort failure'));
      stage.spark.dropErrorIfDisposed(new Error('a second one'));
      return { failed: stage.failed, first: stage.failure.message };
    });
    assert.deepEqual(reported, { failed: true, first: 'late sort failure' }, 'the first cause is the one kept');
    failed(await runtime.frame([at('scene-a', 1)], { size: SIZE }), 'late sort failure');
    assert.equal((await draw(runtime, [at('scene-a', 1)])).source.stale_sort_layers, 0, 'a new stage');
    assert.deepEqual(await rejections(runtime), []);
  });
});

test('a sort that never answers: a superseded frame stops waiting, the budget ends the next, nothing is unhandled', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const exact = await reference(files, [at('scene-a', 2)]);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await watchRejections(runtime);
    await draw(runtime, [at('scene-a', -2)]);
    await breakReadback(runtime, { hang: true, maxWaitMs: 1500 });
    const send = (id, x) =>
      runtime.send({ type: 'frame.render', id, revision: id, plan: plan([at('scene-a', x)], SIZE), output: { width: SIZE[0], height: SIZE[1] } });
    const answer = (id) => runtime.waitFor((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === id, `frame ${id}`);

    // Frame 101 is waiting for a sort that never ends...
    await send(101, 0.5);
    await runtime.page.waitForFunction(() => window.TesselRuntime.debug.splatStages().some((stage) => stage.spark.sorting), { timeout: 30_000 });
    // ...a newer one supersedes it: it stops waiting (before, it would hold the queue until the budget ran out).
    await send(102, 1);
    const superseded = await answer(101);
    assert.equal(superseded.type, 'frame.failed');
    assert.equal(superseded.error.code, 'cancelled');
    // The newer frame has the same stuck sort in front of it: the time budget ends it, with an error that says so.
    const started = performance.now();
    const timedOut = await answer(102);
    assert.equal(timedOut.type, 'frame.failed');
    assert.equal(timedOut.error.code, 'render_failed');
    assert.ok(/did not (finish|settle)/.test(timedOut.error.message), timedOut.error.message);
    assert.ok(performance.now() - started < 20_000);
    // The stage that timed out is retired, not waited on again.
    assert.equal((await runtime.retention()).counters.stage_discarded, 1);
    assert.equal((await draw(runtime, [at('scene-a', 2)])).hash, exact);
    assert.deepEqual(await rejections(runtime), []);
  });
});

test('the shared stage is replaced after a failed sort, and the scenes that keep stages keep them', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b', 'c']);
  const layers = (x) => ['a', 'b', 'c'].map((name) => ({ ...at(`scene-${name}`, x), opacity: 1 / 3 }));
  const exact = await reference(files, layers(2));
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await watchRejections(runtime);
    await draw(runtime, layers(-2), PREVIEW);
    assert.equal((await runtime.retention()).shared_stage_entries, 1, 'the third scene is drawn by the shared stage');
    await breakReadback(runtime, { only: 'shared' });
    failed(await runtime.frame(layers(-1), { size: SIZE, ...PREVIEW }));
    const after = await runtime.retention();
    assert.deepEqual([after.counters.stage_discarded, after.shared_stage_entries, after.stage_entries], [1, 0, 2], 'only the shared stage is thrown away');
    const recovered = await draw(runtime, layers(-0.5), PREVIEW);
    assert.equal(recovered.source.stale_sort_layers, 2, 'the two scenes with stages never lost their orders');
    assert.equal(recovered.cache.shared_stage_entries, 1);
    assert.equal((await draw(runtime, layers(2))).hash, exact);
    assert.deepEqual(await rejections(runtime), []);
  });
});
