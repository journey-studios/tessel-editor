// BL157.2: the accounting of the page's own timing (`telemetry.timing`), as the PAGE keeps it (`src/index.js`), with no browser and no GPU.
//
// `page-timing.test.mjs` proves the shape of the account in a real page. This file proves what that file cannot reach without a draw
// that can be held at an await: WHEN a request stops counting as work ahead of the next one (when it is answered, not when it starts),
// and from which moment a span of background work is placed (the hand-over of the answer, after the PNG encode). The numbers the host's
// "what held the picture" tables are made of are only as good as these two.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { deferred, fakeClock, frameMessage, openFake, sceneWarm, until } from './orchestrator-fixtures.mjs';

const TIMING = { retain: true, telemetry: { timing: true } };

/** A page with `telemetry.timing` on; `body` runs on it and the page is disposed after (it stops the timer of the loop beat). */
const withTiming = async (body) => {
  const page = openFake();
  page.hello(TIMING);
  try {
    await body(page);
  } finally {
    page.send({ type: 'dispose' });
  }
};
const answer = (page, id) => page.messages.findLast((message) => message.type === 'frame.done' && message.id === id);

test('a request that arrives while the frame before it is still being drawn counts that frame as ahead of it', () =>
  withTiming(async (page) => {
    const gate = deferred();
    page.compositor.drawGate = gate.promise;
    page.send(frameMessage([], 1));
    await until(() => page.compositor.log.includes('draw'), 'frame 1 is inside its draw');
    page.compositor.drawGate = null;
    page.send(frameMessage([], 2));
    gate.resolve();
    await page.settle();

    assert.deepEqual(answer(page, 2).page.ahead, { frames: 1, warms: 0, loads: 0 }, 'the frame at its await is in the way of the next one');
    assert.deepEqual(answer(page, 1).page.ahead, { frames: 0, warms: 0, loads: 0 });
    const third = await page.frame([]);
    assert.equal(third.page.ahead.frames, 0, 'and once both are answered nothing is left counted');
  }));

test('a frame that fails still stops counting as ahead', () =>
  withTiming(async (page) => {
    const refused = Promise.reject(new Error('the draw failed'));
    refused.catch(() => {});
    page.compositor.drawGate = refused;
    page.send(frameMessage([], 1));
    await page.settle();
    assert.equal(page.messages.findLast((message) => message.id === 1).type, 'frame.failed');

    page.compositor.drawGate = null;
    await page.frame([], 2);
    assert.equal(answer(page, 2).page.ahead.frames, 0, 'the failed frame left no count behind');
  }));

test('a request that arrives while a warm is still running counts that warm as ahead of it', () =>
  withTiming(async (page) => {
    const gate = deferred();
    page.compositor.warmGate = gate.promise;
    page.send(sceneWarm([]));
    await until(() => page.compositor.log.includes('warm_scenes'), 'the warm is inside its work');
    page.compositor.warmGate = null;
    page.send(frameMessage([], 2));
    gate.resolve();
    await page.settle();

    assert.deepEqual(answer(page, 2).page.ahead, { frames: 0, warms: 1, loads: 0 }, 'a warm at its await is in the way of the frame behind it');
    await page.frame([], 3);
    assert.equal(answer(page, 3).page.ahead.warms, 0, 'and once it ended nothing is left counted');
  }));

test('a span is placed back from the hand-over of the answer: the time the PNG encode took counts', async (t) => {
  const clock = fakeClock();
  t.after(clock.restore);
  await withTiming(async (page) => {
    await page.warm(sceneWarm([])); // a span of background work, noted now
    page.canvas.toDataURL = () => {
      clock.advance(40);
      return 'data:image/png;base64,';
    };
    page.send({ ...frameMessage([], 5), output: { width: 160, height: 90, png: true } });
    await page.settle();

    const span = answer(page, 5).runtime_spans.find((candidate) => candidate.k === 'warm');
    assert.ok(span, 'the warm is reported with this frame');
    assert.ok(span.ago_ms >= 40, `the encode (40 ms) happened after the span ended and before the hand-over: ago_ms ${span.ago_ms}`);
    assert.equal('at' in span, false, 'the page stamp itself never leaves the page');
  });
});
