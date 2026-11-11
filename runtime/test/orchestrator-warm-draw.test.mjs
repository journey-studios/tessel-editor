// BL157.3 (second experiment): the scene warm draw as the PAGE orchestrates it (`src/index.js`), with no browser and no GPU.
//
// `warm-draw-browser.test.mjs` proves on a real WebGL context that the draw leaves the picture and the retained targets alone. This file
// proves the page's side with the real runtime on a compositor that records (`orchestrator-fixtures.mjs`): when a draw is scheduled
// (after ITS sort, as a task of the serial queue), what it yields to (a waiting frame), what drops it (a lost context, a later `hello`, a
// dispose, age), and what the host is told. The draw itself is a recorded call here: that it costs less at the cut is for the A/B.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { fakeClock, frameMessage, openFake, sceneWarm, scenes } from './orchestrator-fixtures.mjs';

const ON = { retain: true, warm_draw: true };

/** The page after a warm of one scene whose sort has not ended: one warm draw is waiting for it. */
const warming = async (hello = ON) => {
  const page = openFake();
  page.hello(hello);
  await page.warm(sceneWarm(scenes('s')));
  return page;
};

/** Ends the sort of the first warmed scene and lets the page's queue run what that released. */
const sorted = async (page) => {
  page.compositor.sorts[0].resolve();
  await page.settle();
};

test('off by default: nothing is announced, no draw is scheduled, and a sort that ends later changes nothing', async () => {
  const page = openFake();
  const ready = page.hello();
  assert.ok(!ready.capabilities.features.includes('scene.warm_draw'), 'a default ready is the one it always was');
  await page.warm(sceneWarm(scenes('s')));
  assert.deepEqual(page.compositor.log, ['warm_scenes'], 'the warm of scenes it always was');
  assert.equal(page.debug.warmDraw().enabled, false);
  assert.equal(page.debug.warmDraw().requested, 0);
  await sorted(page);
  assert.deepEqual(page.compositor.log, ['warm_scenes'], 'no draw followed the sort');
});

test('on: the draw follows ITS sort, once, and the host is told it happened', async () => {
  const page = await warming();
  assert.ok(page.messages.findLast((message) => message.type === 'ready').capabilities.features.includes('scene.warm_draw'));
  assert.equal(page.debug.warmDraw().requested, 1, 'one draw is waiting for the sort');
  assert.deepEqual(page.compositor.log, ['warm_scenes'], 'and nothing is drawn before the sort ends');

  await sorted(page);
  assert.deepEqual(page.compositor.log, ['warm_scenes', 'warm_draw']);
  const report = page.debug.warmDraw();
  assert.equal(report.drawn, 1);
  assert.deepEqual(report.outcomes, { drawn: 1 });

  const done = await page.frame([]);
  assert.deepEqual(
    done.runtime_spans.map(({ k, outcome }) => ({ k, ...(outcome ? { outcome } : {}) })),
    [{ k: 'warm' }, { k: 'warm_draw', outcome: 'drawn' }],
    'the warm, then the draw: the page says, with this frame, that the manipulation happened',
  );
  assert.equal(typeof done.runtime_spans[1].ms, 'number');
});

test('a frame that is waiting goes before the draw, even if it arrived after the draw was queued', async () => {
  const page = await warming();
  const release = page.debug.holdQueue();
  page.compositor.sorts[0].resolve();
  await new Promise((resolve) => setImmediate(resolve)); // the sort's continuation queued the draw behind the gate
  page.send(frameMessage([], 1));
  release();
  await page.settle();

  assert.deepEqual(page.compositor.log, ['warm_scenes', 'draw', 'warm_draw'], 'the picture first; the draw behind it');
});

test('a context lost while the sort ran drops the draw instead of drawing on a context that is not the one asked', async () => {
  const page = await warming();
  page.loseContext();
  await sorted(page);

  assert.ok(!page.compositor.log.includes('warm_draw'), 'nothing was drawn');
  const report = page.debug.warmDraw();
  assert.equal(report.drawn, 0);
  assert.deepEqual(report.outcomes, { dropped: 1 });
});

for (const how of ['a later hello that does not ask', 'a dispose']) {
  test(`${how} drops the draw that was waiting for its sort`, async () => {
    const page = await warming();
    if (how === 'a dispose') page.send({ type: 'dispose' });
    else page.hello({ retain: true }); // every hello says it whole: absent is off
    await sorted(page);

    assert.ok(!page.compositor.log.includes('warm_draw'));
    assert.deepEqual(page.debug.warmDraw().outcomes, { dropped: 1 });
  });
}

test('a draw whose sort ended past its age is dropped unrun: the cut it was for has come or gone', async (t) => {
  const clock = fakeClock();
  t.after(clock.restore);
  for (const [waited, outcomes] of [
    [2000, { drawn: 1 }],
    [3500, { expired: 1 }],
  ]) {
    const page = await warming();
    clock.advance(waited);
    await sorted(page);
    assert.deepEqual(page.debug.warmDraw().outcomes, outcomes, `waited ${waited} ms`);
    assert.equal(page.compositor.log.includes('warm_draw'), 'drawn' in outcomes);
  }
});

for (const outcome of ['gone', 'taken', 'no_order', 'lost', 'failed']) {
  test(`a draw the compositor reports as \`${outcome}\` is told to the host as that, and is not counted as a draw`, async () => {
    const page = await warming();
    page.compositor.warmDrawOutcome = outcome;
    await sorted(page);

    const report = page.debug.warmDraw();
    assert.equal(report.drawn, 0);
    assert.equal(report.skipped, 1);
    assert.deepEqual(report.outcomes, { [outcome]: 1 });
    const done = await page.frame([]);
    assert.deepEqual(
      done.runtime_spans.filter((span) => span.k === 'warm_draw').map((span) => span.outcome),
      [outcome],
      'the host can tell a draw that happened from one that did not',
    );
  });
}

test('a warm that fails is swallowed and the queue goes on: the frame behind it still draws', async () => {
  const page = openFake();
  page.hello(ON);
  page.compositor.warmScenesError = new Error('the scene could not be warmed');
  await page.warm(sceneWarm(scenes('s')));
  assert.equal(page.debug.warmDraw().requested, 0, 'no draw was scheduled for a warm that did not start');

  const done = await page.frame([]);
  assert.equal(done.type, 'frame.done', 'a frame that needs the scene reports what is wrong with it; the warm never takes the queue down');
  assert.equal(done.runtime_spans, undefined, 'and no span claims a warm that did not happen');
});
