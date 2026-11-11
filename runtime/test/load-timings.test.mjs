// BL157.1: timings belong to the admitted load result. Failed/released loads cannot wipe a replacement's unread result.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { AssetStore } from '../src/assets.js';
import { RetentionCounters } from '../src/retention.js';

const descriptor = { id: 'a', kind: 'still', source: { type: 'file', path: 'x.png' } };
const loaded = () => ({ entry: { bitmap: { close() {} }, width: 1, height: 1 }, meta: { width: 1, height: 1 } });
const store = () => new AssetStore({ assetUrl: (path) => path, counters: new RetentionCounters() });

test('a load released while it ran leaves no timings for the next load of that id', async () => {
  const assets = store();
  assets.loadStill = async ({ id }) => {
    assets.release(id); // what `asset.release` does while the bytes are still arriving
    return loaded();
  };
  await assert.rejects(assets.load(descriptor), (error) => error.code === 'cancelled');
  // The next load times nothing (a loader that ends without noting): it must not be given the cancelled load's figures.
  assets.loadStill = async () => loaded();
  const untimed = await assets.load(descriptor);
  assert.deepEqual(assets.takeLoadTimings(untimed), {}, 'nothing of the load that was released');
  assets.loadStill = async () => ({ ...loaded(), timings: { fetch_ms: 2, decode_ms: 3 } });
  const timed = await assets.load(descriptor);
  assert.deepEqual(assets.takeLoadTimings(timed), { fetch_ms: 2, decode_ms: 3 });
  assert.deepEqual(assets.takeLoadTimings(timed), {}, 'taken once');
});

test('a load that failed after timing its decode leaves nothing behind', async () => {
  const assets = store();
  assets.loadStill = async ({ id }) => {
    throw new Error('late failure');
  };
  await assert.rejects(assets.load(descriptor), /late failure/);
  assert.deepEqual(assets.takeLoadTimings({}), {});
});

for (const outcome of ['failed', 'cancelled']) {
  test(`an older ${outcome} load cannot erase the replacement's unread timings`, async () => {
    const assets = store();
    let finishOld;
    assets.loadStill = async ({ id, source }) => {
      if (source.path === 'old.png') {
        await new Promise((resolve) => { finishOld = resolve; });
        if (outcome === 'failed') throw new Error('late failure');
        return { ...loaded(), timings: { fetch_ms: 111 } };
      }
      return { ...loaded(), timings: { fetch_ms: 2, decode_ms: 3 } };
    };
    const old = assets.load({ ...descriptor, source: { type: 'file', path: 'old.png' } });
    const refused = assert.rejects(old, outcome === 'failed' ? /late failure/ : (error) => error.code === 'cancelled');
    const replacement = await assets.load(descriptor);
    finishOld();
    await refused;
    assert.deepEqual(assets.takeLoadTimings(replacement), { fetch_ms: 2, decode_ms: 3 });
  });
}

test('two successful replacements keep their own unread timings, even when read out of order', async () => {
  const assets = store();
  let n = 0;
  assets.loadStill = async () => ({ ...loaded(), timings: { fetch_ms: ++n } });
  const first = await assets.load(descriptor);
  const second = await assets.load(descriptor);
  assert.deepEqual(first, second, 'public metadata is unchanged');
  assert.notEqual(first, second, 'each admitted result owns its timing identity');
  assert.deepEqual(assets.takeLoadTimings(second), { fetch_ms: 2 });
  assert.deepEqual(assets.takeLoadTimings(first), { fetch_ms: 1 });
  assert.deepEqual(assets.takeLoadTimings(second), {});
});
