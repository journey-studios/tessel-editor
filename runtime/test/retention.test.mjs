// Lifecycle rules of the retained GPU objects, without a browser: the LRU bounds and the
// AssetStore's texture ownership (three.js textures need no WebGL until they are drawn).
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { AssetStore } from '../src/assets.js';
import { LIMITS, LruCache, RetentionCounters, rgbaBytes, targetBytes } from '../src/retention.js';

const bitmap = (width, height) => ({
  width,
  height,
  closed: false,
  close() {
    this.closed = true;
  },
});

/** A store with `ids` still images of 100x100 (40 000 bytes of RGBA each), already "loaded". */
const storeWith = (ids, limits = LIMITS) => {
  const store = new AssetStore({ assetUrl: (path) => path, counters: new RetentionCounters(), limits });
  for (const id of ids) {
    store.entries.set(id, { id, kind: 'still', bitmap: bitmap(100, 100), width: 100, height: 100, texture: null });
  }
  return store;
};

const watchDispose = (texture) => {
  const state = { disposed: 0 };
  texture.addEventListener('dispose', () => {
    state.disposed += 1;
  });
  return state;
};

test('LruCache frees the oldest entry first and never one the last frame used', () => {
  const freed = [];
  const cache = new LruCache({ maxEntries: 2, dispose: (value) => freed.push(value) });
  cache.set('a', 'A');
  cache.set('b', 'B');
  cache.set('c', 'C');
  cache.beginFrame();
  cache.get('c');
  cache.trim();
  assert.deepEqual(freed, ['A'], 'only the oldest unused entry goes, down to the bound');
  assert.equal(cache.size, 2);

  cache.beginFrame();
  cache.get('b');
  cache.get('c');
  cache.set('d', 'D');
  cache.trim();
  assert.equal(cache.size, 3, 'a frame that used everything may exceed the bound until the next one');
  cache.beginFrame();
  cache.get('d');
  cache.trim();
  assert.deepEqual(freed, ['A', 'B'], 'the next frame frees what was not used, oldest first');
});

test('LruCache counts bytes, replaces without leaking and clears everything', () => {
  const freed = [];
  const cache = new LruCache({ maxEntries: 10, maxBytes: 100, dispose: (value) => freed.push(value) });
  cache.set('a', 'A', 60);
  cache.set('b', 'B', 60);
  assert.equal(cache.bytes, 120);
  cache.beginFrame();
  cache.get('b');
  cache.trim();
  assert.deepEqual(freed, ['A']);
  assert.equal(cache.bytes, 60);

  cache.set('b', 'B2', 10);
  assert.deepEqual(freed, ['A', 'B'], 'replacing an entry frees the old value');
  assert.equal(cache.bytes, 10);
  cache.deleteWhere((key) => key === 'b');
  assert.equal(cache.size, 0);
  cache.set('x', 'X', 5);
  cache.clear();
  assert.deepEqual(freed, ['A', 'B', 'B2', 'X']);
  assert.equal(cache.bytes, 0);
});

test('a still image keeps one GPU texture across frames and counts reuse', () => {
  const store = storeWith(['a']);
  store.beginFrame();
  const first = store.stillTexture('a');
  store.endFrame();
  store.beginFrame();
  const second = store.stillTexture('a');
  store.endFrame();
  assert.equal(first, second, 'the same texture object is drawn again, not uploaded again');
  assert.equal(store.counters.values.texture_created, 1);
  assert.equal(store.counters.values.texture_reused, 1);
  assert.equal(store.textureBytes(), rgbaBytes(100, 100));
});

test('the byte budget frees the least recently used texture, never one the frame pinned', () => {
  const store = storeWith(['a', 'b', 'c'], { ...LIMITS, imageTextureBytes: rgbaBytes(100, 100) * 2 });
  store.beginFrame();
  const a = store.stillTexture('a');
  const b = store.stillTexture('b');
  store.endFrame();
  const watchA = watchDispose(a);
  assert.equal(store.textureCount(), 2, 'two textures fit');

  store.beginFrame();
  store.stillTexture('c');
  store.stillTexture('b');
  store.endFrame();
  assert.equal(watchA.disposed, 1, 'a was the least recently used');
  assert.equal(store.entries.get('a').texture, null);
  assert.equal(store.textureCount(), 2);
  assert.equal(store.counters.values.texture_evicted, 1);

  // Over budget inside one frame: all pinned, nothing is freed under the frame that draws them.
  store.beginFrame();
  store.stillTexture('a');
  store.stillTexture('b');
  store.stillTexture('c');
  store.endFrame();
  assert.equal(store.textureCount(), 3);
  store.beginFrame();
  store.stillTexture('c');
  store.endFrame();
  assert.equal(store.textureCount(), 2, 'the next frame brings it back under budget');
  // An evicted image is uploaded again from its bitmap, as a new texture.
  assert.equal(store.entries.get('a').bitmap.closed, false);
});

test('releasing an asset disposes its texture, closes its bitmap and tells the listeners', () => {
  const store = storeWith(['a', 'b']);
  const heard = [];
  store.releaseListeners.add((id) => heard.push(id));
  store.beginFrame();
  const texture = store.stillTexture('a');
  store.endFrame();
  const watch = watchDispose(texture);
  const before = store.generation;
  const entry = store.entries.get('a');

  store.release('a');
  assert.equal(watch.disposed, 1);
  assert.equal(entry.bitmap.closed, true);
  assert.equal(store.has('a'), false);
  assert.ok(store.generation > before, 'the generation moves so anything built from the old world can tell');
  assert.deepEqual(heard, ['a']);
  assert.equal(store.isCurrent('a', entry), false, 'a frame that read the old entry must not draw it');
});

test('a lost context drops textures but keeps the bitmaps so the next frame uploads again', () => {
  const store = storeWith(['a']);
  store.beginFrame();
  const texture = store.stillTexture('a');
  store.endFrame();
  const watch = watchDispose(texture);
  const before = store.generation;

  store.dropGpu();
  assert.equal(watch.disposed, 1);
  assert.equal(store.textureCount(), 0);
  assert.equal(store.entries.get('a').bitmap.closed, false, 'the decoded image survives the context');
  assert.ok(store.generation > before);

  store.beginFrame();
  const again = store.stillTexture('a');
  store.endFrame();
  assert.notEqual(again, texture, 'a disposed texture is never reused');
  assert.equal(store.counters.values.texture_created, 2);
});

test('sequence frames: the count bound frees the oldest frame but not one the frame in flight pinned', async () => {
  const store = new AssetStore({ assetUrl: (path) => path, counters: new RetentionCounters() });
  const frames = new Map();
  const entry = {
    id: 'seq',
    kind: 'video',
    sequence: { count: 40, first: 0, pattern: 'f%d.png' },
    frames,
    width: 10,
    height: 10,
  };
  store.entries.set('seq', entry);
  let fetched = 0;
  store.fetchFrame = async () => {
    fetched += 1;
    return bitmap(10, 10);
  };

  const textures = [];
  for (let index = 0; index < 20; index += 1) {
    store.beginFrame();
    textures.push(await store.frameTexture('seq', index));
    store.endFrame();
  }
  assert.equal(frames.size, 16, 'at most 16 decoded frames are kept');
  assert.equal(fetched, 20);
  assert.equal(frames.has(0), false, 'the oldest went first');
  assert.equal(frames.has(19), true);

  // One frame that draws a cached old frame and fetches new ones: the pinned frame survives.
  store.beginFrame();
  const oldest = frames.keys().next().value;
  const kept = await store.frameTexture('seq', oldest);
  const watch = watchDispose(kept);
  for (let index = 20; index < 24; index += 1) await store.frameTexture('seq', index);
  store.endFrame();
  assert.equal(watch.disposed, 0, 'a texture the frame in flight draws is never disposed under it');
  assert.equal(frames.has(oldest), true);
  assert.equal(frames.size, 16);

  // A hit makes no fetch and returns the same texture.
  store.beginFrame();
  const hit = await store.frameTexture('seq', 19);
  store.endFrame();
  assert.equal(hit, textures[19]);
  assert.equal(fetched, 24);

  // Evicting only the GPU copy keeps the bitmap: the frame is not fetched again.
  store.disposeTextures(entry);
  store.beginFrame();
  const rebuilt = await store.frameTexture('seq', 19);
  store.endFrame();
  assert.notEqual(rebuilt, textures[19]);
  assert.equal(fetched, 24);
});

test('a frame fetched for an entry that was replaced meanwhile is discarded, not cached', async () => {
  const store = new AssetStore({ assetUrl: (path) => path, counters: new RetentionCounters() });
  const original = {
    id: 'seq',
    kind: 'video',
    sequence: { count: 4, first: 0, pattern: 'f%d.png' },
    frames: new Map(),
    width: 10,
    height: 10,
  };
  store.entries.set('seq', original);
  const late = bitmap(10, 10);
  store.fetchFrame = async () => {
    // The same id is loaded again from another source while this frame is still arriving.
    store.entries.set('seq', { ...original, frames: new Map() });
    return late;
  };
  await assert.rejects(store.frameTexture('seq', 1), (error) => error.code === 'cancelled');
  assert.equal(late.closed, true, 'the bitmap of the old source is closed');
  assert.equal(original.frames.size, 0);
});

test('render targets are estimated with their MSAA storage, so export size is never retained', () => {
  assert.ok(targetBytes(960, 540, 4) < LIMITS.targetBytes, 'the monitor size fits');
  assert.ok(targetBytes(3840, 2160, 4) > LIMITS.targetBytes, 'a 4K scene target is rebuilt per frame instead');
});
