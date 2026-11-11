// BL157.3: the texture pre-upload as the PAGE orchestrates it (`src/index.js`), with no browser and no GPU.
//
// Three layers prove this feature. `preupload.test.mjs` and `preupload-store.test.mjs` prove the queue and the store's rules; the Chrome
// tests (`preupload-browser.test.mjs`) prove a real WebGL context makes the texture and draws the same pixels. This file is the layer
// between them that none of those could reach: the real runtime (its serial queue, its messages, its spans) on a compositor that records
// instead of drawing (`orchestrator-fixtures.mjs`). It proves the page's decisions: what runs before what, what a lost context, a later
// `hello` or a dispose drops, and what the host is told. It cannot prove that the upload shortens anything: that is the A/B.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { rgbaBytes } from '../src/retention.js';
import { SIDE, fakeClock, frameMessage, openFake, stills, textureWarm } from './orchestrator-fixtures.mjs';

const ON = { retain: true, preupload_textures: true };
const BYTES = rgbaBytes(SIDE, SIDE);
const textureOf = (page, id) => page.assets.get(id).texture ?? null;

test('off by default: nothing is announced, nothing is uploaded ahead, and the frame makes its own texture as it always did', async () => {
  const page = openFake();
  const ready = page.hello();
  assert.ok(!ready.capabilities.features.includes('texture.preupload'), 'a default ready is the one it always was');
  await page.load('a');
  await page.warm(textureWarm(stills('a')));
  assert.equal(page.compositor.uploads.length, 0, 'a warm that asks for textures uploads none: the page was not asked to');
  assert.equal(page.debug.preupload().enabled, false);
  assert.equal(page.debug.preupload().requested, 0);

  const done = await page.frame(stills('a'));
  assert.equal(done.cache.texture_created, 1, 'the frame made the texture itself');
  assert.equal(page.compositor.frameUploads, 1);
  assert.equal(done.runtime_spans, undefined, 'and the host heard nothing about pre-uploads');
});

test('on: a warm uploads the texture ahead of the frame, the frame finds it, and the host is told it happened', async () => {
  const page = openFake();
  const ready = page.hello(ON);
  assert.ok(ready.capabilities.features.includes('texture.preupload'), 'announced to the host that asked');
  await page.load('a');
  await page.warm(textureWarm(stills('a')));
  assert.deepEqual(page.compositor.log, ['upload'], 'one upload, and no picture was drawn for it');
  assert.equal(page.debug.preupload().uploaded, 1);

  const done = await page.frame(stills('a'));
  assert.equal(done.cache.texture_created, 0, 'the frame had no texture to make');
  assert.equal(done.cache.texture_reused, 1, 'it found the one the pre-upload made');
  assert.equal(page.compositor.frameUploads, 0, 'so the first draw paid no first upload');
  assert.deepEqual(
    done.runtime_spans.map(({ k, outcome }) => ({ k, outcome })),
    [{ k: 'preupload', outcome: 'uploaded' }],
    'the page says, with this frame, that the manipulation happened',
  );
  assert.equal(typeof done.runtime_spans[0].ms, 'number');

  const next = await page.frame(stills('a'));
  assert.equal(next.runtime_spans, undefined, 'a span is handed over once');
});

test('a second warm of textures already made asks for nothing, and nothing is uploaded twice', async () => {
  const page = openFake();
  page.hello(ON);
  await page.loadAll('a', 'b');
  await page.warm(textureWarm(stills('a', 'b')));
  await page.warm(textureWarm(stills('a', 'b')));
  await page.warm(textureWarm(stills('a', 'b')));
  assert.equal(page.compositor.uploads.length, 2);
  assert.equal(page.debug.preupload().requested, 2, 'the later warms found both resident and had nothing to ask for');
});

test('a frame that is waiting goes before every queued upload, and the upload it finds already made is resident', async () => {
  const page = openFake();
  page.hello(ON);
  await page.loadAll('a', 'b', 'c');
  const release = page.debug.holdQueue();
  page.send(textureWarm(stills('a', 'b', 'c')));
  page.send(frameMessage(stills('a'), 1)); // arrives AFTER the warm, behind it in the queue
  release();
  await page.settle();

  assert.deepEqual(page.compositor.log, ['draw', 'upload', 'upload'], 'the frame went first; then b and c were made, a was already there');
  assert.equal(page.compositor.frameUploads, 1, 'the frame made a, which the pre-upload had not reached');
  const done = await page.frame(stills('a'));
  assert.deepEqual(
    done.runtime_spans.map(({ outcome }) => outcome),
    ['resident', 'uploaded', 'uploaded'],
    'and the host hears the three steps in the order they ran',
  );
});

test('a frame that arrives in the middle of the queue waits for the step in flight, not for the queue', async () => {
  const page = openFake();
  page.hello(ON);
  await page.loadAll('a', 'b', 'c', 'd', 'e');
  page.compositor.onUpload = () => {
    page.compositor.onUpload = null;
    page.send(frameMessage(stills('e'), 7)); // the host asks for a picture while the first texture is being made
  };
  await page.warm(textureWarm(stills('a', 'b', 'c', 'd', 'e')));

  assert.deepEqual(
    page.compositor.log,
    ['upload', 'draw', 'upload', 'upload', 'upload'],
    'the picture was drawn right after the one upload in flight; the other four steps came behind it (e was already made by the frame)',
  );
  assert.equal(page.debug.preupload().uploaded, 4);
  assert.equal(page.debug.preupload().resident, 1);
});

test('an asset reloaded before its upload is stale and its replacement is not touched; one released is missing', async () => {
  const page = openFake();
  page.hello(ON);
  await page.loadAll('a', 'b');
  const release = page.debug.holdQueue();
  page.send(textureWarm(stills('a', 'b')));
  await page.load('a'); // a replacement: the entry now holds another serial
  page.release('b'); // released, never replaced
  release();
  await page.settle();

  assert.equal(page.compositor.uploads.length, 0, 'neither was uploaded');
  assert.equal(textureOf(page, 'a'), null, 'the replacement has no texture made for the entry it replaced');
  const done = await page.frame(stills('a'));
  assert.equal(done.cache.texture_created, 1, 'it makes its own on its own frame');
  assert.deepEqual(done.runtime_spans.map(({ outcome }) => outcome), ['stale', 'missing']);
});

test('an upload the GPU refuses is `failed`, leaves no texture behind, and the queue goes on', async () => {
  const page = openFake();
  page.hello(ON);
  await page.loadAll('a', 'b');
  page.compositor.failUploads = 1;
  await page.warm(textureWarm(stills('a', 'b')));

  assert.equal(textureOf(page, 'a'), null, 'no half-made texture is kept for a');
  assert.notEqual(textureOf(page, 'b'), null, 'b was made: one failure does not stop the queue');
  const done = await page.frame(stills('a', 'b'));
  assert.deepEqual(done.runtime_spans.map(({ outcome }) => outcome), ['failed', 'uploaded']);
  assert.equal(done.cache.texture_created, 1, 'a is made by its own frame, as it always was');
  assert.equal(done.cache.texture_reused, 1, 'b was found');
  assert.equal(page.debug.preupload().failed, 1);
});

test('with retention off nothing is queued and nothing is uploaded, whatever the warm asks', async () => {
  const page = openFake();
  const ready = page.hello({ retain: false, preupload_textures: true });
  assert.ok(ready.capabilities.features.includes('texture.preupload'), 'the host still asked for it');
  await page.load('a');
  await page.warm(textureWarm(stills('a')));
  assert.equal(page.debug.preupload().requested, 0, 'there is no retained texture to make ahead');
  assert.equal(page.compositor.uploads.length, 0);
});

for (const how of ['a later hello that does not ask', 'a dispose']) {
  test(`${how} drops what was queued`, async () => {
    const page = openFake();
    page.hello(ON);
    await page.loadAll('a', 'b');
    const release = page.debug.holdQueue();
    page.send(textureWarm(stills('a', 'b')));
    if (how === 'a dispose') page.send({ type: 'dispose' });
    else page.hello({ retain: true }); // every hello says it whole: absent is off
    assert.equal(page.debug.preupload().pending, 0, 'forgotten when it was said, not when the step got to it');
    release();
    await page.settle();

    assert.equal(page.compositor.uploads.length, 0, 'nothing was uploaded');
    assert.equal(page.debug.preupload().pending, 0);
    assert.equal(page.debug.preupload().cleared, 2, 'both were forgotten, not run');
  });
}

test('a context lost before the step drops the queue; a warm during the loss asks nothing and one after the restore starts again', async () => {
  const page = openFake();
  page.hello(ON);
  await page.loadAll('a', 'b');
  const release = page.debug.holdQueue();
  page.send(textureWarm(stills('a', 'b')));
  page.loseContext();
  assert.equal(page.debug.preupload().pending, 0, 'forgotten at the loss, not when the step got to it');
  release();
  await page.settle();

  assert.equal(page.compositor.uploads.length, 0, 'nothing was uploaded to a context that is gone');
  assert.equal(page.debug.preupload().pending, 0);
  assert.equal(page.debug.preupload().cleared, 2);
  assert.equal(page.messages.filter((message) => message.type === 'context.lost').length, 1, 'the host was told, as it always was');

  await page.warm(textureWarm(stills('a', 'b')));
  assert.equal(page.debug.preupload().requested, 2, 'a warm during the loss asked for nothing');

  page.restoreContext();
  await page.warm(textureWarm(stills('a', 'b')));
  assert.equal(page.compositor.uploads.length, 2, 'after the restore the textures are made again, for the new context');
  assert.equal(page.debug.preupload().uploaded, 2);
});

test('a loss whose event has not been delivered yet is still a loss: the step asks the context before touching it', async () => {
  const page = openFake();
  page.hello(ON);
  await page.loadAll('a', 'b');
  const release = page.debug.holdQueue();
  page.send(textureWarm(stills('a', 'b')));
  page.compositor.lost = true; // the GL context is gone; its `webglcontextlost` is a separate task that has not run
  release();
  await page.settle();

  assert.equal(page.compositor.uploads.length, 0);
  assert.equal(page.debug.preupload().pending, 0);
  assert.equal(page.debug.preupload().failed, 0, 'it was not even tried');
});

test('the queue is bounded: a warm that finds more is counted, and the rest is made by the next warm', async () => {
  const page = openFake();
  page.hello(ON);
  const ids = Array.from({ length: 12 }, (_, index) => `s${index}`);
  await page.loadAll(...ids);
  await page.warm(textureWarm(stills(...ids)));
  assert.equal(page.debug.preupload().requested, 12);
  assert.equal(page.debug.preupload().dropped, 4, 'no more than 8 wait at once');
  assert.equal(page.compositor.uploads.length, 8);

  await page.warm(textureWarm(stills(...ids)));
  assert.equal(page.compositor.uploads.length, 12, 'the four that did not fit are asked for again, and the eight are not');
  assert.equal(page.debug.preupload().requested, 16);
});

test('the byte budget is never exceeded to make a texture ahead: an upload that does not fit is `budget`, and nothing is evicted for it', async () => {
  const page = openFake();
  page.hello(ON);
  page.debug.setLimits({ imageTextureBytes: BYTES }); // room for exactly one
  await page.loadAll('a', 'b');
  await page.warm(textureWarm(stills('a', 'b')));

  assert.equal(page.compositor.uploads.length, 1, 'only a fit');
  assert.notEqual(textureOf(page, 'a'), null, 'and a was not evicted to make room for b');
  assert.equal(page.debug.preupload().budget, 1);
  assert.equal(page.debug.retention().counters.texture_evicted, 0);

  const done = await page.frame(stills('b'));
  assert.equal(done.cache.texture_created, 1, 'b is made by the frame that draws it, as it always was');
  assert.equal(done.cache.texture_evicted, 1, 'and the budget gives back the texture nobody used (a), not the one on screen');
  assert.equal(textureOf(page, 'a'), null);
  assert.notEqual(textureOf(page, 'b'), null);
});

test('a queued upload ages out: one within its age runs, one past it is dropped unrun', async (t) => {
  const clock = fakeClock();
  t.after(clock.restore);
  for (const [waited, outcome] of [
    [2000, 'uploaded'],
    [3500, 'expired'],
  ]) {
    const page = openFake();
    page.hello(ON);
    await page.loadAll('a');
    const release = page.debug.holdQueue();
    page.send(textureWarm(stills('a')));
    clock.advance(waited); // the playhead moved on while the page was busy
    release();
    await page.settle();
    const done = await page.frame(stills('a'));
    assert.deepEqual(done.runtime_spans.map((span) => span.outcome), [outcome], `waited ${waited} ms`);
    assert.equal(page.compositor.uploads.length, outcome === 'uploaded' ? 1 : 0);
  }
});
