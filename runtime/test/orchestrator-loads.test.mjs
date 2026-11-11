// BL157.1: what the page says about a load (`asset.state`), through the real runtime handler with no browser and no GPU.
//
// `load-timings.test.mjs` proves the STORE binds a load's timings to the result it admitted, so an older load that fails or is
// cancelled cannot wipe a replacement's unread timings (the race fixed in b53775e). This file proves the other half, in `index.js`: the
// message the host receives reads the timings by that result (not by the reusable asset id), and every state of a load repeats the
// identity of the request that caused it, in whichever order a load and the one that overtook it end. A decoder is a stub here; the
// bytes, the fetch and the decode are the Chrome tests'.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { SIDE, openFake, stillDescriptor, until } from './orchestrator-fixtures.mjs';

const OLD = { fetch_ms: 111, decode_ms: 112 };
const NEW = { fetch_ms: 2, decode_ms: 3 };
const timingsByPath = (path) => (path === 'old.png' ? OLD : NEW);
const answers = (page, id, request) =>
  page.messages.filter((message) => message.type === 'asset.state' && message.id === id && message.request === request);
const states = (page, id, request) => answers(page, id, request).map((message) => message.state);

test('a ready load says which request it answers and carries the timings of its own load', async () => {
  const page = openFake();
  page.hello();
  page.timings = timingsByPath;
  const ready = await page.load('a', { request: 7, path: 'new.png' });

  assert.equal(ready.state, 'ready');
  assert.equal(ready.request, 7, 'the identity of the request that caused it');
  assert.deepEqual(ready.meta, { width: SIDE, height: SIDE });
  assert.equal(ready.timings.fetch_ms, NEW.fetch_ms, 'the figures of this load, read through its own result');
  assert.equal(ready.timings.decode_ms, NEW.decode_ms);
  assert.equal(typeof ready.timings.load_ms, 'number', 'and the page-clock duration of the whole load');
});

for (const order of ['the old load ends first', 'the replacement ends first']) {
  test(`a load a reload overtook is cancelled with ITS request, and the replacement keeps its own timings: ${order}`, async () => {
    const page = openFake();
    page.hello();
    page.timings = timingsByPath;
    const finishOld = page.hold('old.png');
    const finishNew = page.hold('new.png');
    page.send({ type: 'asset.load', asset: stillDescriptor('a', 'old.png', 1) });
    page.send({ type: 'asset.load', asset: stillDescriptor('a', 'new.png', 2) });

    const ends = {
      old: async () => {
        finishOld();
        await until(() => states(page, 'a', 1).includes('failed'), 'the old load ended');
      },
      replacement: async () => {
        finishNew();
        await until(() => states(page, 'a', 2).includes('ready'), 'the replacement was ready');
      },
    };
    if (order === 'the old load ends first') {
      await ends.old();
      await ends.replacement();
    } else {
      await ends.replacement();
      await ends.old();
    }

    assert.deepEqual(states(page, 'a', 1), ['loading', 'failed'], 'the old request never became ready');
    assert.equal(answers(page, 'a', 1)[1].error.code, 'cancelled', 'it says it was overtaken, with its own request');
    assert.deepEqual(states(page, 'a', 2), ['loading', 'ready']);
    const ready = answers(page, 'a', 2)[1];
    assert.equal(ready.timings.fetch_ms, NEW.fetch_ms, 'the replacement reports its own figures, never the older load\'s');
    assert.equal(ready.timings.decode_ms, NEW.decode_ms);
    assert.equal(page.assets.get('a').descriptor.source.path, 'new.png', 'and the store holds the replacement');
  });
}

test('a load released while it ran is cancelled with its own request, and the next load of the id reports only its own timings', async () => {
  const page = openFake();
  page.hello();
  page.timings = timingsByPath;
  const finishOld = page.hold('old.png');
  page.send({ type: 'asset.load', asset: stillDescriptor('a', 'old.png', 1) });
  page.release('a'); // the host gave the slot up while the bytes were arriving
  finishOld();
  await until(() => states(page, 'a', 1).includes('failed'), 'the released load ended');
  assert.equal(answers(page, 'a', 1)[1].error.code, 'cancelled');

  const ready = await page.load('a', { request: 2, path: 'new.png' });
  assert.equal(ready.request, 2);
  assert.equal(ready.timings.fetch_ms, NEW.fetch_ms, 'nothing of the released load reached this one');
  assert.deepEqual(states(page, 'a', 1), ['loading', 'failed'], 'and the released request stayed what it was');
});
