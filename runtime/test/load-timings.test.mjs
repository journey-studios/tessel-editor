// BL157.1: the durations of a load wait in a side table until the host's answer takes them. A load that ends without an
// answer (released while it was running, refused, failed) must not leave them there for the next load of the same id.
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
    assets.noteLoadTimings(id, { fetch_ms: 111, decode_ms: 111 });
    assets.release(id); // what `asset.release` does while the bytes are still arriving
    return loaded();
  };
  await assert.rejects(assets.load(descriptor), (error) => error.code === 'cancelled');
  // The next load times nothing (a loader that ends without noting): it must not be given the cancelled load's figures.
  assets.loadStill = async () => loaded();
  await assets.load(descriptor);
  assert.deepEqual(assets.takeLoadTimings('a'), {}, 'nothing of the load that was released');
  assets.noteLoadTimings('a', { fetch_ms: 2, decode_ms: 3 });
  assert.deepEqual(assets.takeLoadTimings('a'), { fetch_ms: 2, decode_ms: 3 });
  assert.deepEqual(assets.takeLoadTimings('a'), {}, 'taken once');
});

test('a load that failed after timing its decode leaves nothing behind', async () => {
  const assets = store();
  assets.loadStill = async ({ id }) => {
    assets.noteLoadTimings(id, { fetch_ms: 5, decode_ms: 5 });
    throw new Error('late failure');
  };
  await assert.rejects(assets.load(descriptor), /late failure/);
  assert.deepEqual(assets.takeLoadTimings('a'), {});
});
