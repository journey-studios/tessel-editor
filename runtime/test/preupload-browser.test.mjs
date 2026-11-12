// BL157.3: the opt-in pre-upload of a still image's texture before its clip starts, in a real page (headless Chrome,
// SwiftShader) through the bridge messages the desktop uses. Every claim is counted at the GL boundary (`texImage2D` /
// `texSubImage2D` calls that carry an ImageBitmap), not inferred from the runtime's own counters, and every pixel claim
// compares exact hashes against the picture of a runtime that never pre-uploaded anything. Skipped, loudly, without Chrome or
// the bundle (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
import assert from 'node:assert/strict';
import { copyFileSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { flat, imageLayer, makeProject, openRuntime, plan, skipUnavailable, stillAsset } from './support.mjs';

const WARM = (layers) => ({ type: 'frame.warm', scenes: false, textures: true, plan: plan(layers) });
const ON = { retain: true, preupload_textures: true };

/** Counts the uploads of an ImageBitmap to a texture, at the WebGL boundary. */
const countUploads = (runtime) =>
  runtime.page.evaluate(() => {
    window.__uploads = 0;
    const prototype = WebGL2RenderingContext.prototype;
    for (const name of ['texImage2D', 'texSubImage2D']) {
      const original = prototype[name];
      prototype[name] = function (...args) {
        if (args[args.length - 1] instanceof ImageBitmap) window.__uploads += 1;
        return original.apply(this, args);
      };
    }
  });
const uploads = (runtime) => runtime.page.evaluate(() => window.__uploads);
const status = (runtime) => runtime.page.evaluate(() => window.TesselRuntime.debug.preupload());
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Waits until the pre-upload queue is empty and the page's serial queue has drained. */
const drained = async (runtime) => {
  for (let attempt = 0; attempt < 400; attempt += 1) {
    await runtime.page.evaluate(() => window.TesselRuntime.debug.idle());
    if ((await status(runtime)).pending === 0) {
      await runtime.page.evaluate(() => window.TesselRuntime.debug.idle());
      return;
    }
    await sleep(10);
  }
  assert.fail('the pre-upload queue never drained');
};

const warm = async (runtime, layers) => {
  await runtime.send(WARM(layers));
  await drained(runtime);
};

const canvasPixels = (runtime) => runtime.page.evaluate(() => document.querySelector('canvas').toDataURL('image/png'));

/** Two pictures with distinct, flat colors in a scratch directory (the page's allow list gets them). */
const pictures = () => {
  const directory = mkdtempSync(join(tmpdir(), 'tessel-preupload-'));
  const files = {};
  for (const [name, color] of Object.entries({ red: [220, 30, 30], green: [30, 200, 60], blue: [40, 60, 230] })) {
    files[name] = join(directory, `${name}.png`);
    writeFileSync(files[name], flat(48, 27, color));
  }
  files.redCopy = join(directory, 'red-copy.png');
  copyFileSync(files.red, files.redCopy);
  return files;
};

/** The picture of `layers` on a runtime that never pre-uploads (the reference every pixel claim compares to). */
const coldHash = async (project, extraFiles, assets, layers, hello = { retain: true }) => {
  const runtime = await openRuntime({ project, extraFiles });
  try {
    await runtime.hello(hello);
    for (const asset of assets) assert.equal((await runtime.load(asset)).state, 'ready');
    const done = await runtime.frame(layers);
    assert.equal(done.type, 'frame.done', JSON.stringify(done.error));
    return done.hash;
  } finally {
    await runtime.close();
  }
};

const open = async (project, files) => {
  const runtime = await openRuntime({ project, extraFiles: Object.values(files) });
  await countUploads(runtime);
  return runtime;
};

test('off by default: a warm that asks for textures uploads nothing and is not announced', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const files = pictures();
  const runtime = await open(project, files);
  try {
    const ready = await runtime.hello({ retain: true });
    assert.equal(ready.capabilities.features.includes('texture.preupload'), false, 'a default ready is the one it always was');
    assert.equal((await runtime.load(stillAsset('red', files.red))).state, 'ready');
    await warm(runtime, [imageLayer('red')]);
    assert.equal(await uploads(runtime), 0, 'nothing is uploaded before a frame asks');
    assert.equal((await status(runtime)).enabled, false);
    const first = await runtime.frame([imageLayer('red')]);
    assert.equal(first.cache.texture_created, 1, 'the frame uploads as it always did');
    assert.equal(await uploads(runtime), 1);
  } finally {
    await runtime.close();
  }
});

test('on: the texture is uploaded once, before the frame that draws it, and the frame uploads nothing', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const files = pictures();
  const reference = await coldHash(project, Object.values(files), [stillAsset('red', files.red)], [imageLayer('red')]);
  const runtime = await open(project, files);
  try {
    const ready = await runtime.hello(ON);
    assert.ok(ready.capabilities.features.includes('texture.preupload'));
    assert.equal((await runtime.load(stillAsset('red', files.red))).state, 'ready');
    assert.equal(await uploads(runtime), 0, 'loading a picture does not upload it');

    await warm(runtime, [imageLayer('red')]);
    assert.equal(await uploads(runtime), 1, 'uploaded before any frame');
    assert.equal((await runtime.retention()).texture_entries, 1);
    const before = await canvasPixels(runtime);

    const first = await runtime.frame([imageLayer('red')]);
    assert.equal(first.type, 'frame.done');
    assert.equal(first.cache.texture_created, 0, 'the frame found the texture');
    assert.equal(first.cache.texture_reused, 1);
    assert.equal(await uploads(runtime), 1, 'and uploaded nothing');
    assert.equal(first.hash, reference, 'the pixels are the pixels of a runtime that never pre-uploaded');

    // The same warm again, and more frames: still the one upload.
    await warm(runtime, [imageLayer('red')]);
    await runtime.frame([imageLayer('red')]);
    await runtime.frame([imageLayer('red')]);
    assert.equal(await uploads(runtime), 1, 'never uploaded again');
    assert.deepEqual(
      { uploaded: (await status(runtime)).uploaded, requested: (await status(runtime)).requested },
      { uploaded: 1, requested: 1 },
      'the second warm found the texture resident and asked for nothing',
    );
    assert.notEqual(before, await canvasPixels(runtime), 'sanity: the frame did change the canvas (the upload alone did not)');
  } finally {
    await runtime.close();
  }
});

test('a pre-upload draws nothing: the canvas of the frame on screen is untouched', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const files = pictures();
  const runtime = await open(project, files);
  try {
    await runtime.hello(ON);
    for (const [id, file] of [['red', files.red], ['green', files.green]]) {
      assert.equal((await runtime.load(stillAsset(id, file))).state, 'ready');
    }
    const shown = await runtime.frame([imageLayer('red')]);
    const onScreen = await canvasPixels(runtime);
    await warm(runtime, [imageLayer('green')]);
    assert.equal(await uploads(runtime), 2, 'the next picture was uploaded');
    assert.equal(await canvasPixels(runtime), onScreen, 'not one pixel of the picture on screen changed');
    const again = await runtime.frame([imageLayer('red')]);
    assert.equal(again.hash, shown.hash);
  } finally {
    await runtime.close();
  }
});

test('hot, cold and retention-off draws agree exactly, and two entries with equal bitmaps each get their own correct texture', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const files = pictures();
  const assets = [stillAsset('red', files.red), stillAsset('twin', files.redCopy), stillAsset('green', files.green)];
  const layersOf = (id) => [imageLayer(id)];
  const cold = {};
  const unretained = {};
  for (const id of ['red', 'twin', 'green']) {
    cold[id] = await coldHash(project, Object.values(files), assets, layersOf(id));
    unretained[id] = await coldHash(project, Object.values(files), assets, layersOf(id), { retain: false });
  }
  assert.equal(cold.red, cold.twin, 'the two entries really hold equal bitmaps');
  assert.notEqual(cold.red, cold.green);
  assert.deepEqual(unretained, cold, 'retention off is the reference: same pixels');

  const runtime = await open(project, files);
  try {
    await runtime.hello(ON);
    for (const asset of assets) assert.equal((await runtime.load(asset)).state, 'ready');
    await warm(runtime, [imageLayer('red'), imageLayer('twin'), imageLayer('green')]);
    assert.equal(await uploads(runtime), 3, 'one texture per entry, equal bitmaps or not: nothing is shared by pixels');
    for (const id of ['green', 'twin', 'red', 'green']) {
      const done = await runtime.frame(layersOf(id));
      assert.equal(done.cache.texture_created, 0, `${id}: found its own texture`);
      assert.equal(done.hash, cold[id], `${id}: hot picture is the cold picture`);
    }
    assert.equal(await uploads(runtime), 3);
    // An explicit frame (inspected, original quality) and a preview frame are the same picture for a still.
    const preview = await runtime.frame(layersOf('twin'), { quality: 'preview' });
    assert.equal(preview.hash, cold.twin);
  } finally {
    await runtime.close();
  }
});

test('a warm only makes the textures of loaded stills the plan draws: covered, unloaded and non-still layers are left alone', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const files = pictures();
  const runtime = await open(project, files);
  try {
    await runtime.hello(ON);
    assert.equal((await runtime.load(stillAsset('red', files.red))).state, 'ready');
    await warm(runtime, [
      imageLayer('red', { id: 'covered', occluded: true }),
      imageLayer('never-loaded'),
      { ...imageLayer('red'), kind: 'text', text: { content: 'T', size: 12, color: '#fff' } },
    ]);
    assert.equal(await uploads(runtime), 0);
    assert.equal((await status(runtime)).requested, 0);
    assert.equal((await runtime.retention()).texture_entries, 0);
  } finally {
    await runtime.close();
  }
});

test('a release before the upload leaves nothing behind, and a replacement is never touched by the warm of the old entry', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const files = pictures();
  const runtime = await open(project, files);
  try {
    await runtime.hello(ON);
    assert.equal((await runtime.load(stillAsset('slot', files.red))).state, 'ready');

    // 1. Released while the warm waits in the queue.
    await runtime.page.evaluate(() => {
      window.__release = window.TesselRuntime.debug.holdQueue();
    });
    await runtime.send(WARM([imageLayer('slot')]));
    await runtime.release('slot');
    await runtime.page.evaluate(() => window.__release());
    await drained(runtime);
    assert.equal(await uploads(runtime), 0);
    assert.equal((await status(runtime)).missing, 1);
    assert.equal((await runtime.retention()).texture_entries, 0);

    // 2. Replaced (same id, other bytes) while the warm waits: the warm was for the old entry and does not touch the new one.
    assert.equal((await runtime.load(stillAsset('slot', files.red))).state, 'ready');
    await runtime.page.evaluate(() => {
      window.__release = window.TesselRuntime.debug.holdQueue();
    });
    await runtime.send(WARM([imageLayer('slot')]));
    assert.equal((await runtime.load(stillAsset('slot', files.green))).state, 'ready');
    await runtime.page.evaluate(() => window.__release());
    await drained(runtime);
    const after = await status(runtime);
    assert.equal(after.stale, 1);
    assert.equal(await uploads(runtime), 0, 'the new entry was not uploaded by the old warm');
    assert.equal((await runtime.retention()).texture_entries, 0);

    // The frame that draws the new entry draws its own picture, from its own upload.
    const done = await runtime.frame([imageLayer('slot')]);
    assert.equal(done.cache.texture_created, 1);
    const green = await coldHash(project, Object.values(files), [stillAsset('slot', files.green)], [imageLayer('slot')]);
    assert.equal(done.hash, green, 'never the picture of the old entry');
  } finally {
    await runtime.close();
  }
});

test('a reload after the upload frees the old texture and the next warm uploads the new one, once', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const files = pictures();
  const runtime = await open(project, files);
  try {
    await runtime.hello(ON);
    assert.equal((await runtime.load(stillAsset('slot', files.red))).state, 'ready');
    await warm(runtime, [imageLayer('slot')]);
    assert.equal(await uploads(runtime), 1);
    const glBefore = await runtime.page.evaluate(() => window.TesselRuntime.debug.glMemory().textures);

    assert.equal((await runtime.load(stillAsset('slot', files.blue))).state, 'ready');
    assert.equal((await runtime.retention()).texture_entries, 0, 'the old texture went with its entry');
    assert.equal(await runtime.page.evaluate(() => window.TesselRuntime.debug.glMemory().textures), glBefore - 1);
    await warm(runtime, [imageLayer('slot')]);
    assert.equal(await uploads(runtime), 2);
    const done = await runtime.frame([imageLayer('slot')]);
    assert.equal(done.cache.texture_created, 0);
    assert.equal(await uploads(runtime), 2);
    assert.equal(done.hash, await coldHash(project, Object.values(files), [stillAsset('slot', files.blue)], [imageLayer('slot')]));
  } finally {
    await runtime.close();
  }
});

test('a lost context drops the queue and the texture, ignores a warm while lost and warms again after the restore', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const files = pictures();
  const runtime = await open(project, files);
  try {
    await runtime.hello(ON);
    assert.equal((await runtime.load(stillAsset('red', files.red))).state, 'ready');
    const reference = (await runtime.frame([imageLayer('red')])).hash;
    await runtime.release('red');
    assert.equal((await runtime.load(stillAsset('red', files.red))).state, 'ready');
    await warm(runtime, [imageLayer('red')]);
    const uploaded = await uploads(runtime);

    // Lost with an upload waiting: the queue is dropped, nothing is uploaded on a context that is not the one asked.
    await runtime.release('red');
    assert.equal((await runtime.load(stillAsset('red', files.red))).state, 'ready');
    await runtime.page.evaluate(() => {
      window.__release = window.TesselRuntime.debug.holdQueue();
    });
    await runtime.send(WARM([imageLayer('red')]));
    await runtime.page.evaluate(() => window.TesselRuntime.debug.loseContext());
    await runtime.waitFor((m) => m.type === 'context.lost');
    await runtime.page.evaluate(() => window.__release());
    await drained(runtime);
    assert.equal(await uploads(runtime), uploaded, 'no upload on the lost context');
    assert.equal((await status(runtime)).pending, 0);
    assert.ok((await status(runtime)).cleared >= 1, 'the waiting upload was dropped');
    assert.equal((await runtime.retention()).texture_entries, 0);

    // A warm that arrives while lost is not queued.
    const requested = (await status(runtime)).requested;
    await warm(runtime, [imageLayer('red')]);
    assert.equal((await status(runtime)).requested, requested);

    await runtime.page.evaluate(() => window.TesselRuntime.debug.restoreContext());
    await runtime.waitFor((m) => m.type === 'context.restored');
    // The host loads everything again after a loss (the session forgets its assets); the warm then works as the first time.
    assert.equal((await runtime.load(stillAsset('red', files.red))).state, 'ready');
    await warm(runtime, [imageLayer('red')]);
    assert.equal(await uploads(runtime), uploaded + 1, 're-warmed once on the new context');
    const done = await runtime.frame([imageLayer('red')]);
    assert.equal(done.cache.texture_created, 0);
    assert.equal(done.hash, reference, 'the picture after the loss is the picture before it');
  } finally {
    await runtime.close();
  }
});

test('the texture budget is never exceeded and no texture in use is freed to make room', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const files = pictures();
  const runtime = await open(project, files);
  try {
    await runtime.hello(ON);
    for (const [id, file] of [['red', files.red], ['green', files.green], ['blue', files.blue]]) {
      assert.equal((await runtime.load(stillAsset(id, file))).state, 'ready');
    }
    // Room for one 48x27 picture.
    await runtime.page.evaluate(() => window.TesselRuntime.debug.setLimits({ imageTextureBytes: 48 * 27 * 4 + 16 }));
    const inUse = await runtime.frame([imageLayer('red')]);
    assert.equal(inUse.cache.texture_created, 1);
    const evictedBefore = (await runtime.retention()).counters.texture_evicted;

    await warm(runtime, [imageLayer('green'), imageLayer('blue')]);
    assert.equal((await status(runtime)).budget, 2, 'neither fits beside the picture in use: asked for no eviction');
    assert.equal((await runtime.retention()).counters.texture_evicted, evictedBefore, 'nothing was freed for them');
    assert.equal(await uploads(runtime), 1, 'only the frame uploaded');
    assert.ok((await runtime.retention()).texture_bytes <= 48 * 27 * 4 + 16);
    const again = await runtime.frame([imageLayer('red')]);
    assert.equal(again.cache.texture_created, 0, 'the texture in use was still there');

    // Room for two now: a pre-uploaded picture nobody used is the first the budget gives back, never the one in use.
    await runtime.page.evaluate(() => window.TesselRuntime.debug.setLimits({ imageTextureBytes: 2 * 48 * 27 * 4 + 16 }));
    await warm(runtime, [imageLayer('green')]);
    assert.equal((await status(runtime)).uploaded, 1, 'green fits beside red and was uploaded');
    const crowded = await runtime.frame([imageLayer('blue')]);
    assert.equal(crowded.cache.texture_created, 1);
    const kept = await runtime.frame([imageLayer('red')]);
    assert.equal(kept.cache.texture_created, 0, 'the budget gave back the picture nobody used (green), not the one in use (red)');
    await runtime.page.evaluate(() => window.TesselRuntime.debug.setLimits({ imageTextureBytes: 48 * 27 * 4 + 16 }));

    // The frame that does need green uploads it as it always did and is correct.
    const green = await runtime.frame([imageLayer('green')]);
    assert.equal(green.type, 'frame.done');
    assert.equal(green.cache.texture_created, 1);
    assert.equal(green.hash, await coldHash(project, Object.values(files), [stillAsset('green', files.green)], [imageLayer('green')]));
  } finally {
    await runtime.close();
  }
});

test('the frame the host is waiting for goes first, and the queue is bounded', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const files = pictures();
  const runtime = await open(project, files);
  try {
    await runtime.hello(ON);
    assert.equal((await runtime.load(stillAsset('red', files.red))).state, 'ready');

    // Queue: [gate, warm upload, frame]. The upload yields to the frame behind it: the frame uploads its own texture.
    await runtime.page.evaluate(() => {
      window.__release = window.TesselRuntime.debug.holdQueue();
    });
    await runtime.send(WARM([imageLayer('red')]));
    await runtime.send({
      type: 'frame.render',
      id: 700,
      revision: 700,
      plan: plan([imageLayer('red')]),
      output: { width: 160, height: 90 },
    });
    await runtime.page.evaluate(() => window.__release());
    const done = await runtime.waitFor((m) => m.type === 'frame.done' && m.id === 700, 'frame 700');
    assert.equal(done.cache.texture_created, 1, 'the frame was not made to wait behind an upload for its own picture');
    await drained(runtime);
    assert.equal(await uploads(runtime), 1, 'the upload that had yielded found the texture resident');
    assert.equal((await status(runtime)).resident, 1);

    // Bounded: ten pictures asked at once leave eight pending, the rest are counted.
    for (let index = 0; index < 10; index += 1) {
      assert.equal((await runtime.load(stillAsset(`g${index}`, files.green))).state, 'ready');
    }
    await runtime.page.evaluate(() => {
      window.__release = window.TesselRuntime.debug.holdQueue();
    });
    await runtime.send(WARM(Array.from({ length: 10 }, (_, index) => imageLayer(`g${index}`))));
    const queued = await status(runtime);
    assert.equal(queued.pending, 8);
    assert.equal(queued.dropped, 2);
    await runtime.page.evaluate(() => window.__release());
    await drained(runtime);
    assert.equal(await uploads(runtime), 1 + 8);
  } finally {
    await runtime.close();
  }
});

test('turning it off in a later hello forgets what was queued and uploads nothing', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const files = pictures();
  const runtime = await open(project, files);
  try {
    await runtime.hello(ON);
    assert.equal((await runtime.load(stillAsset('red', files.red))).state, 'ready');
    await runtime.page.evaluate(() => {
      window.__release = window.TesselRuntime.debug.holdQueue();
    });
    await runtime.send(WARM([imageLayer('red')]));
    const off = await runtime.hello({ retain: true });
    assert.equal(off.capabilities.features.includes('texture.preupload'), false);
    await runtime.page.evaluate(() => window.__release());
    await drained(runtime);
    assert.equal(await uploads(runtime), 0);
    assert.equal((await status(runtime)).enabled, false);
  } finally {
    await runtime.close();
  }
});
