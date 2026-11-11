// BL157.3: the bookkeeping of the texture pre-upload queue, without a browser (the GL behaviour is in
// preupload-browser.test.mjs).
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { PREUPLOAD_MAX_AGE_MS, PREUPLOAD_PENDING_LIMIT, TexturePreupload, stillsToUpload } from '../src/preupload.js';

const entry = (kind, serial, extra = {}) => ({ kind, serial, bitmap: {}, texture: null, ...extra });
const store = (entries) => ({ entries: new Map(Object.entries(entries)) });
const layer = (asset, extra = {}) => ({ kind: 'image', asset, ...extra });

test('only loaded, uncovered still layers without a texture are wanted, once each, with the serial loaded now', () => {
  const assets = store({
    a: entry('still', 7),
    b: entry('still', 8, { texture: {} }),
    c: entry('splat', 9),
    d: entry('still', 10, { bitmap: null }),
    e: entry('still', 11),
  });
  const plan = {
    layers: [
      layer('a'),
      layer('a'),
      layer('b'),
      layer('c'),
      layer('d'),
      layer('missing'),
      layer('e', { occluded: true }),
      { kind: 'text', asset: 'a' },
    ],
  };
  assert.deepEqual(stillsToUpload(plan, assets), [{ id: 'a', serial: 7 }]);
  assert.deepEqual(stillsToUpload({}, assets), []);
  assert.deepEqual(stillsToUpload(null, assets), []);
});

test('the queue is bounded, repeats collapse and a clear is counted', () => {
  const queue = new TexturePreupload({ now: () => 0 });
  queue.enqueue([{ id: 'a', serial: 1 }, { id: 'a', serial: 1 }]);
  assert.equal(queue.pending.length, 1);
  queue.enqueue(Array.from({ length: PREUPLOAD_PENDING_LIMIT + 3 }, (_, index) => ({ id: `s${index}`, serial: index })));
  assert.equal(queue.pending.length, PREUPLOAD_PENDING_LIMIT);
  assert.equal(queue.stats.dropped, 4);
  queue.clear();
  assert.equal(queue.pending.length, 0);
  assert.equal(queue.stats.cleared, PREUPLOAD_PENDING_LIMIT);
  assert.equal(queue.step(store({}), () => {}), null);
});

test('only an upload that happened adds to the upload time', () => {
  let clock = 0;
  const queue = new TexturePreupload({ now: () => clock });
  const said = { a: 'uploaded', b: 'resident', c: 'budget', d: 'stale', e: 'missing' };
  const assets = {
    preuploadStill: (id) => {
      clock += 10; // every step costs 10 ms, whatever it ends up saying
      return said[id];
    },
  };
  queue.enqueue(Object.keys(said).map((id, serial) => ({ id, serial })));
  for (let step = 0; step < 5; step += 1) queue.step(assets, () => {});
  assert.equal(queue.stats.upload_ms, 10, 'the one upload that happened, not the four steps that did not make a texture');
  assert.equal(queue.stats.max_upload_ms, 10);
});

test('a step does one upload and reports what the store said; a throw is a failure, an old item expires', () => {
  let clock = 0;
  const queue = new TexturePreupload({ now: () => clock });
  const calls = [];
  const assets = {
    preuploadStill: (id, serial) => {
      calls.push([id, serial]);
      if (id === 'boom') throw new Error('gpu');
      return id === 'x' ? 'budget' : 'uploaded';
    },
  };
  queue.enqueue([{ id: 'a', serial: 1 }, { id: 'x', serial: 2 }, { id: 'boom', serial: 3 }]);
  assert.equal(queue.step(assets, () => {}).outcome, 'uploaded');
  assert.equal(calls.length, 1, 'one texture per step');
  assert.equal(queue.step(assets, () => {}).outcome, 'budget');
  assert.equal(queue.step(assets, () => {}).outcome, 'failed');
  assert.deepEqual(
    [queue.stats.uploaded, queue.stats.budget, queue.stats.failed],
    [1, 1, 1],
  );

  queue.enqueue([{ id: 'late', serial: 4 }]);
  clock += PREUPLOAD_MAX_AGE_MS + 1;
  assert.equal(queue.step(assets, () => {}).outcome, 'expired');
  assert.equal(calls.length, 3, 'an expired item reaches the store never');
});
