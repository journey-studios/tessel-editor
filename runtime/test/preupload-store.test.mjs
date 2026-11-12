// BL157.3: the texture pre-upload against the REAL AssetStore, with no browser and no GPU.
//
// `preupload.test.mjs` proves the queue with a store that is a stub, and `preupload-browser.test.mjs` proves the whole thing on a real
// WebGL context. This file is the layer between them: the real `AssetStore.preuploadStill` (the budget, the serial, the recency clock,
// the disposal of a failed upload) and the real `stillsToUpload`/`TexturePreupload`, driven by what a host does (load, reload,
// release, lose the context). `upload` is a spy standing for `renderer.initTexture`; three.js textures need no WebGL until drawn.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { AssetStore } from '../src/assets.js';
import { TexturePreupload, stillsToUpload } from '../src/preupload.js';
import { LIMITS, RetentionCounters, rgbaBytes } from '../src/retention.js';
import { imageLayer, plan } from './support.mjs';

const SIDE = 100;
const BYTES = rgbaBytes(SIDE, SIDE);

const bitmap = () => ({
  width: SIDE,
  height: SIDE,
  closed: false,
  close() {
    this.closed = true;
  },
});
const descriptor = (id) => ({ id, kind: 'still', source: { type: 'file', path: `${id}.png` }, meta: {} });

/** A store that has loaded `ids` through `load()` (so each entry has the serial a real load gives it); every still decodes to 100x100. */
const loadedStore = async (ids, limits = LIMITS) => {
  const assets = new AssetStore({ assetUrl: (path) => path, counters: new RetentionCounters(), limits });
  assets.loadStill = async () => ({ entry: { bitmap: bitmap(), width: SIDE, height: SIDE }, meta: { width: SIDE, height: SIDE } });
  for (const id of ids) await assets.load(descriptor(id));
  return assets;
};

const serialOf = (assets, id) => assets.get(id).serial;
/** The retained texture of `id`, `null` when there is none (a loaded entry starts without the field, a freed one has null). */
const textureOf = (assets, id) => assets.get(id).texture ?? null;
const count = (assets, name) => assets.counters.values[name] ?? 0;

/** Stands for `renderer.initTexture`: remembers every texture it was given. */
const gpu = () => {
  const made = [];
  const upload = (texture) => {
    made.push(texture);
  };
  return Object.assign(upload, { made });
};

const watchDispose = (texture) => {
  const state = { disposed: 0 };
  texture.addEventListener('dispose', () => {
    state.disposed += 1;
  });
  return state;
};

test('uploaded: the texture is made once, ahead of the frame, and the frame that draws it finds it', async () => {
  const assets = await loadedStore(['a']);
  const upload = gpu();
  assert.equal(assets.preuploadStill('a', serialOf(assets, 'a'), upload), 'uploaded');
  assert.equal(upload.made.length, 1, 'one upload');
  assert.equal(assets.get('a').texture, upload.made[0], 'the entry holds the texture that was uploaded');
  assert.equal(assets.textureBytes(), BYTES);
  assert.equal(assets.pinned.size, 0, 'nothing is pinned: no frame is drawing');

  assets.beginFrame();
  const drawn = assets.stillTexture('a');
  assets.endFrame();
  assert.equal(drawn, upload.made[0], 'the frame draws the very texture the pre-upload made');
  assert.equal(count(assets, 'texture_created'), 0, 'the frame created nothing');
  assert.equal(count(assets, 'texture_reused'), 1);
  assert.equal(upload.made.length, 1, 'and uploaded nothing');
});

test('resident: a texture that already exists is never made or uploaded again, by a frame or by a second ask', async () => {
  const assets = await loadedStore(['a', 'b']);
  const upload = gpu();
  const serial = serialOf(assets, 'a');
  assert.equal(assets.preuploadStill('a', serial, upload), 'uploaded');
  assert.equal(assets.preuploadStill('a', serial, upload), 'resident');
  assets.beginFrame();
  assets.stillTexture('b'); // made by a frame, as it always was
  assets.endFrame();
  assert.equal(assets.preuploadStill('b', serialOf(assets, 'b'), upload), 'resident', 'a texture a frame made is found, not duplicated');
  assert.equal(upload.made.length, 1, 'the only upload is the first');
});

test('budget: an upload that does not fit asks nothing of the textures already there and leaves the frame to make it', async () => {
  const assets = await loadedStore(['a', 'b', 'c'], { ...LIMITS, imageTextureBytes: BYTES * 2 });
  const upload = gpu();
  assert.equal(assets.preuploadStill('a', serialOf(assets, 'a'), upload), 'uploaded');
  assert.equal(assets.preuploadStill('b', serialOf(assets, 'b'), upload), 'uploaded');
  const textures = [assets.get('a').texture, assets.get('b').texture];

  assert.equal(assets.preuploadStill('c', serialOf(assets, 'c'), upload), 'budget', 'a third texture does not fit');
  assert.equal(upload.made.length, 2, 'the refused one was never sent to the GPU');
  assert.equal(textureOf(assets, 'c'), null);
  assert.deepEqual([assets.get('a').texture, assets.get('b').texture], textures, 'nothing was freed to make room');
  assert.equal(count(assets, 'texture_evicted'), 0, 'a pre-upload evicts nothing');
  assert.ok(assets.textureBytes() <= assets.limits.imageTextureBytes);

  // The frame uploads it as it always did.
  assets.beginFrame();
  assets.stillTexture('c');
  assets.endFrame();
  assert.equal(count(assets, 'texture_created'), 1);
});

test('recency: a texture nobody drew is the first the budget gives back, and one a frame drew stays', async () => {
  const assets = await loadedStore(['a', 'b', 'c'], { ...LIMITS, imageTextureBytes: BYTES * 2 });
  const upload = gpu();
  assets.beginFrame();
  const used = assets.stillTexture('a'); // what the playhead is on
  assets.endFrame();
  assert.equal(assets.preuploadStill('b', serialOf(assets, 'b'), upload), 'uploaded');
  assert.equal(assets.get('b').lastUsed, undefined, 'the pre-upload does not touch the recency clock');

  // The frame that reaches `c` takes the store over budget: the one nobody used goes, never the one that was drawn.
  assets.beginFrame();
  assets.stillTexture('c');
  assets.endFrame();
  assert.equal(textureOf(assets, 'b'), null, 'the pre-uploaded texture nobody drew was given back first');
  assert.equal(assets.get('a').texture, used, 'the texture the playhead drew stayed');
  assert.equal(assets.get('b').bitmap.closed, false, 'only the GPU copy went: the picture can be uploaded again');
  assert.equal(count(assets, 'texture_evicted'), 1);
});

test('stale and missing: an entry that was reloaded or released meanwhile is never given a texture by the ask of the old one', async () => {
  const assets = await loadedStore(['a', 'b']);
  const upload = gpu();
  const oldA = serialOf(assets, 'a');
  const oldB = serialOf(assets, 'b');

  await assets.load(descriptor('a')); // `a` reloaded with other bytes: a new entry under the same id
  assert.notEqual(serialOf(assets, 'a'), oldA, 'a reload gives the id a serial nobody had');
  assert.equal(assets.preuploadStill('a', oldA, upload), 'stale');
  assert.equal(textureOf(assets, 'a'), null, 'the replacement has no texture from the old ask');

  assets.release('b');
  assert.equal(assets.preuploadStill('b', oldB, upload), 'missing');
  assert.equal(assets.preuploadStill('never-loaded', 1, upload), 'missing');
  // Not stills, but shaped so that only their kind can stop them: a decoded picture is there to upload.
  for (const [id, kind, serial] of [['s', 'splat', 90], ['m', 'video', 92]]) {
    assets.entries.set(id, { id, kind, serial, bitmap: bitmap(), width: SIDE, height: SIDE, texture: null });
    assert.equal(assets.preuploadStill(id, serial, upload), 'missing', `only still images are uploaded ahead, not a ${kind}`);
  }
  assets.entries.set('v', { id: 'v', kind: 'still', serial: 91, bitmap: null, width: SIDE, height: SIDE, texture: null });
  assert.equal(assets.preuploadStill('v', 91, upload), 'missing', 'a still without a decoded picture has nothing to upload');
  assert.equal(upload.made.length, 0, 'no upload was made for any of them');

  // The replacement gets its own: asked with its own serial.
  assert.equal(assets.preuploadStill('a', serialOf(assets, 'a'), upload), 'uploaded');
  assert.equal(upload.made.length, 1);
});

test('failed: an upload that throws leaves no texture and no bytes, frees what it made, and can be asked again', async () => {
  const assets = await loadedStore(['a']);
  const serial = serialOf(assets, 'a');
  let watch = null;
  assert.throws(
    () =>
      assets.preuploadStill('a', serial, (texture) => {
        watch = watchDispose(texture);
        throw new Error('gpu said no');
      }),
    /gpu said no/,
  );
  assert.equal(watch.disposed, 1, 'the texture of the failed upload was disposed');
  assert.equal(textureOf(assets, 'a'), null);
  assert.equal(assets.textureBytes(), 0);
  assert.equal(count(assets, 'texture_created'), 0);

  const upload = gpu();
  assert.equal(assets.preuploadStill('a', serial, upload), 'uploaded', 'a later ask succeeds');
  assert.equal(upload.made.length, 1);
});

test('a lost context drops the texture and keeps the picture: the same entry is uploaded again, as a new texture', async () => {
  const assets = await loadedStore(['a']);
  const upload = gpu();
  const serial = serialOf(assets, 'a');
  assert.equal(assets.preuploadStill('a', serial, upload), 'uploaded');
  const first = upload.made[0];
  const watch = watchDispose(first);

  assets.dropGpu(); // what the runtime does on `webglcontextlost`
  assert.equal(watch.disposed, 1);
  assert.equal(textureOf(assets, 'a'), null);
  assert.equal(assets.get('a').bitmap.closed, false, 'the decoded image survives the context');
  assert.equal(assets.preuploadStill('a', serial, upload), 'uploaded', 'same entry, same serial, new context');
  assert.notEqual(upload.made[1], first, 'a disposed texture is never reused');

  assets.dropTextures(); // retention switched off
  assert.equal(assets.textureBytes(), 0);
  assert.equal(assets.preuploadStill('a', serial, upload), 'uploaded', 'and again after the retention is back');
  assert.equal(upload.made.length, 3);
});

test('the queue and the store together: a warm asks once, a reload or release before the step is dropped, and a done texture is not asked again', async () => {
  const assets = await loadedStore(['a', 'b', 'c']);
  const queue = new TexturePreupload({ now: () => 0 });
  const upload = gpu();
  const layers = plan([imageLayer('a'), imageLayer('b'), imageLayer('c')]);

  // The host warms the cut: three loaded, uncovered stills are wanted, once each, with the serial loaded now.
  assert.deepEqual(
    stillsToUpload(layers, assets),
    ['a', 'b', 'c'].map((id) => ({ id, serial: serialOf(assets, id) })),
  );
  queue.enqueue(stillsToUpload(layers, assets));
  queue.enqueue(stillsToUpload(layers, assets)); // the same warm sent again while it waits
  assert.equal(queue.pending.length, 3, 'a repeated warm collapses');

  // Before the first step: `a` is replaced by a reload and `b` released.
  await assets.load(descriptor('a'));
  assets.release('b');
  const outcomes = [];
  while (queue.pending.length > 0) outcomes.push(queue.step(assets, upload).outcome);
  assert.deepEqual(outcomes, ['stale', 'missing', 'uploaded']);
  assert.equal(upload.made.length, 1, 'only `c`, the entry that was still the one asked for, reached the GPU');
  assert.deepEqual(
    { uploaded: queue.stats.uploaded, stale: queue.stats.stale, missing: queue.stats.missing, requested: queue.stats.requested },
    { uploaded: 1, stale: 1, missing: 1, requested: 6 },
  );

  // The next warm wants the new `a` (its own serial) and nothing for `c`, whose texture is there.
  assert.deepEqual(stillsToUpload(layers, assets), [{ id: 'a', serial: serialOf(assets, 'a') }]);
  queue.enqueue(stillsToUpload(layers, assets));
  assert.equal(queue.step(assets, upload).outcome, 'uploaded');
  assert.equal(upload.made.length, 2);
  assert.deepEqual(stillsToUpload(layers, assets), [], 'every still of the plan has its texture: a warm asks for nothing');
});
