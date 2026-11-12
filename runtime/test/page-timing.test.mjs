// BL157.2: the page's own account of a frame (`telemetry.timing` in `hello`). It exists to tell, when a picture arrives late,
// whether the page was busy (its event loop stalled), whether the request waited in the page's serial queue behind other work,
// or whether the delay was outside the page. Real headless Chrome through the bridge messages the desktop uses; skipped,
// loudly, without Chrome or the bundle (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { openRuntime, plan, skipUnavailable } from './support.mjs';

const SIZE = [64, 36];

const request = (id) => ({
  v: 1,
  type: 'frame.render',
  id,
  revision: id,
  plan: plan([], SIZE),
  output: { width: SIZE[0], height: SIZE[1] },
});

const withPage = async (hello, body) => {
  const runtime = await openRuntime();
  try {
    await runtime.hello(hello);
    return await body(runtime);
  } finally {
    await runtime.close();
  }
};

const done = (runtime, id) => runtime.waitFor((m) => m.type === 'frame.done' && m.id === id, `frame ${id}`);

test('without telemetry.timing a frame answers exactly as it did: no page account, no timers', async (t) => {
  if (skipUnavailable(t, assert)) return;
  await withPage({}, async (runtime) => {
    const frame = await runtime.frame([], { size: SIZE });
    assert.equal(frame.type, 'frame.done');
    assert.equal('page' in frame, false);
    assert.equal(frame.runtime_spans?.some((span) => 'ago_ms' in span) ?? false, false);
  });
});

test('with telemetry.timing a frame carries its queue wait, its time held, what was ahead and the loop lag', async (t) => {
  if (skipUnavailable(t, assert)) return;
  await withPage({ telemetry: { timing: true } }, async (runtime) => {
    const first = await runtime.frame([], { size: SIZE, id: 1 });
    assert.deepEqual(Object.keys(first.page).sort(), ['ahead', 'held_ms', 'loop', 'post_prev_ms', 'queue_ms']);
    assert.deepEqual(first.page.ahead, { frames: 0, warms: 0, loads: 0 });
    assert.equal(first.page.post_prev_ms, null, 'nothing was posted before the first answer');
    assert.ok(first.page.held_ms >= first.page.queue_ms && first.page.queue_ms >= 0);
    assert.deepEqual(Object.keys(first.page.loop).sort(), ['beat_ms', 'max_ms', 'n', 'over']);

    const second = await runtime.frame([], { size: SIZE, id: 2 });
    assert.equal(typeof second.page.post_prev_ms, 'number', 'the next answer says how long handing over the previous one took');
    assert.equal(second.timings.total_ms >= 0, true, 'total_ms keeps meaning the draw, queue wait excluded');
  });
});

test('a request that arrives behind another frame says so', async (t) => {
  if (skipUnavailable(t, assert)) return;
  await withPage({ telemetry: { timing: true } }, async (runtime) => {
    // Both arrive in one task: the second can only start after the first has been settled (newest wins, so the older
    // one is cancelled), and it is the second that reports what was ahead of it.
    await runtime.page.evaluate((a, b) => {
      window.TesselRuntime.receive(a);
      window.TesselRuntime.receive(b);
    }, request(10), request(11));
    const second = await done(runtime, 11);
    assert.deepEqual(second.page.ahead, { frames: 1, warms: 0, loads: 0 });
    const older = runtime.messages.find((m) => m.type === 'frame.failed' && m.id === 10);
    assert.equal(older?.error.code, 'cancelled');
  });
});

test('a page whose event loop is blocked says how long: in the frame that waited and in the loop lag that follows', async (t) => {
  if (skipUnavailable(t, assert)) return;
  await withPage({ telemetry: { timing: true } }, async (runtime) => {
    await runtime.frame([], { size: SIZE, id: 20 });
    const BLOCK_MS = 150;
    // The request is handed over, then the page's thread is held for BLOCK_MS: what a long task does to a queued frame.
    await runtime.page.evaluate(
      (message, block) => {
        window.TesselRuntime.receive(message);
        const until = performance.now() + block;
        while (performance.now() < until);
      },
      request(21),
      BLOCK_MS,
    );
    const blocked = await done(runtime, 21);
    assert.ok(blocked.page.queue_ms >= BLOCK_MS - 5, `the frame waited out the block (queue_ms ${blocked.page.queue_ms})`);
    // total_ms is the draw alone: queue wait plus draw fits inside the time held (counting the wait in total_ms would not).
    // A relation on the page's own clock, so it holds however loaded the machine is.
    assert.ok(
      blocked.page.queue_ms + blocked.timings.total_ms <= blocked.page.held_ms + 1,
      `total_ms ${blocked.timings.total_ms} left out the wait (queue_ms ${blocked.page.queue_ms}, held_ms ${blocked.page.held_ms})`,
    );

    // The timer that was due during the block fires late afterwards; the next answer reports it, and as a span placed by
    // a duration back from the hand-over (never a page timestamp the host would subtract from its own).
    const next = await runtime.frame([], { size: SIZE, id: 22 });
    assert.ok(next.page.loop.max_ms >= BLOCK_MS - 20, `the next frame reports the lag (max_ms ${next.page.loop.max_ms})`);
    assert.ok(next.page.loop.over >= 1);
    const stall = (next.runtime_spans ?? []).find((span) => span.k === 'loop_stall');
    assert.ok(stall, 'the stall is a span');
    assert.ok(stall.ms >= BLOCK_MS - 20);
    assert.equal(typeof stall.ago_ms, 'number');
    assert.equal('at' in stall, false, 'the page stamp itself never leaves the page');
  });
});

test('with telemetry.timing a WebGL call that makes the page wait is a span with its name, and the probe goes off with the timing', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    // A stand-in for a call the GPU process made the page wait on.
    await runtime.page.evaluate(() => {
      const slow = WebGL2RenderingContext.prototype.finish;
      window.__nativeFinish = slow;
      WebGL2RenderingContext.prototype.finish = function (...args) {
        const until = performance.now() + 12;
        while (performance.now() < until);
        return slow.apply(this, args);
      };
    });
    await runtime.hello({ telemetry: { timing: true } });
    await runtime.page.evaluate(() => {
      document.createElement('canvas').getContext('webgl2').finish();
    });
    const frame = await runtime.frame([], { size: SIZE, id: 31 });
    const span = (frame.runtime_spans ?? []).find((s) => s.k === 'gl_call' && s.name === 'finish');
    assert.ok(span, 'the slow call is a span');
    assert.ok(span.ms >= 12);
    await runtime.hello({});
    await runtime.page.evaluate(() => {
      document.createElement('canvas').getContext('webgl2').finish();
    });
    const after = await runtime.frame([], { size: SIZE, id: 32 });
    assert.equal((after.runtime_spans ?? []).some((s) => s.k === 'gl_call'), false, 'the probe is off again');
  } finally {
    await runtime.close();
  }
});

test('the GPU wait of the diagnostic arm is reported only when asked, and only with the timing', async (t) => {
  if (skipUnavailable(t, assert)) return;
  await withPage({ telemetry: { timing: true } }, async (runtime) => {
    const plain = await runtime.frame([], { size: SIZE, id: 41 });
    assert.equal('gpu_finish_ms' in plain.timings, false);
    await runtime.hello({ telemetry: { timing: true, gpu_finish: true } });
    const waited = await runtime.frame([], { size: SIZE, id: 42 });
    assert.equal(typeof waited.timings.gpu_finish_ms, 'number');
    await runtime.hello({ telemetry: { gpu_finish: true } });
    const alone = await runtime.frame([], { size: SIZE, id: 43 });
    assert.equal('gpu_finish_ms' in alone.timings, false, 'without timing the arm stays off');
  });
});
