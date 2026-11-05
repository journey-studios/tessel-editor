// Native container video (runtime/src/native-video.js) and the failures around it, in real
// headless Chrome (SwiftShader) with clips made here by ffmpeg: every picture carries its own
// frame number as a 10-bit barcode in its top row, so "the right picture" is read from pixels,
// never inferred from a hash or a timer. Skipped, loudly, without Chrome, a built bundle or
// ffmpeg (TESSEL_REQUIRE_RUNTIME=1 turns that into a failure).
//
// What is proven: B1 (a video is not black), B2 (every frame, in any order, is the one asked
// for, and two layers of one clip show their own frames), B3 (what the engine cannot show is a
// structured error, never a black frame; `webm` is not promised), B7 (PNG and JPEG sequences
// draw the same pictures) and B8 (a context lost mid-frame is `context_lost`, with the cause).
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  imageLayer,
  makeProject,
  openRuntime,
  sceneAsset,
  sceneLayer,
  shuffled,
  skipUnavailable,
  stillAsset,
  transform,
  videoLayer,
} from './support.mjs';

const WIDTH = 640;
const HEIGHT = 360;
const FRAMES = 48;
const RATE = { num: 24, den: 1, fps: 24 };
const CELL = WIDTH / 20;

const ffmpeg = (args) => spawnSync('ffmpeg', ['-nostdin', '-v', 'error', '-y', ...args], { encoding: 'utf8' });

const barcodeFilter = () => {
  const boxes = [`drawbox=x=0:y=0:w=${CELL * 10}:h=${CELL}:color=black:t=fill`];
  for (let bit = 0; bit < 10; bit += 1) {
    boxes.push(`drawbox=x=${bit * CELL}:y=0:w=${CELL}:h=${CELL}:color=white:t=fill:enable='mod(floor(n/${2 ** bit}),2)'`);
  }
  return `noise=c0s=5:allf=t,${boxes.join(',')}`;
};

const source = ['-f', 'lavfi', '-i', `testsrc2=size=${WIDTH}x${HEIGHT}:rate=24:duration=100`, '-frames:v', String(FRAMES), '-vf', barcodeFilter()];

/** Makes the fixtures once for the whole file; a missing encoder is reported, not hidden. */
let fixtures = null;
const makeFixtures = () => {
  if (fixtures) return fixtures;
  const directory = mkdtempSync(join(tmpdir(), 'tessel-video-'));
  const clips = { directory };
  const encode = (name, codec) => {
    const path = join(directory, name);
    const result = ffmpeg([...source, ...codec, path]);
    clips[name] = result.status === 0 ? path : null;
    if (result.status !== 0) clips[`${name}:error`] = result.stderr || String(result.error);
  };
  encode('h264.mp4', ['-c:v', 'libx264', '-crf', '18', '-preset', 'veryfast', '-pix_fmt', 'yuv420p', '-g', '24', '-bf', '3', '-movflags', '+faststart']);
  encode('vp9.webm', ['-c:v', 'libvpx-vp9', '-b:v', '0', '-crf', '30', '-g', '24', '-cpu-used', '6', '-deadline', 'good', '-pix_fmt', 'yuv420p']);
  // A clip that never changes picture: what a stuck decoder looks like from the outside.
  const still = join(directory, 'still.mp4');
  const stillResult = ffmpeg(['-f', 'lavfi', '-i', `color=c=red:size=${WIDTH}x${HEIGHT}:rate=24:duration=2`, '-c:v', 'libx264', '-pix_fmt', 'yuv420p', still]);
  clips['still.mp4'] = stillResult.status === 0 ? still : null;
  writeFileSync(join(directory, 'broken.mp4'), Buffer.from('this is not a video, only bytes that claim to be one'));
  clips['broken.mp4'] = join(directory, 'broken.mp4');
  for (const [folder, quality] of [
    ['png', ['-compression_level', '1']],
    ['jpg', ['-q:v', '3']],
  ]) {
    mkdirSync(join(directory, folder));
    const result = ffmpeg([...source, ...quality, '-start_number', '0', join(directory, folder, `x%05d.${folder}`)]);
    clips[folder] = result.status === 0 ? join(directory, folder, `x%05d.${folder}`) : null;
  }
  fixtures = clips;
  return clips;
};

const videoAsset = (id, path, extra = {}) => ({
  id,
  kind: 'video',
  source: { type: 'file', path },
  meta: { rate: RATE, duration: { num: FRAMES, den: 24, seconds: FRAMES / 24 }, width: WIDTH, height: HEIGHT, ...extra },
});

const sequenceOf = (id, pattern) => ({
  id,
  kind: 'video',
  source: { type: 'frame_sequence', pattern, first: 0, count: FRAMES },
  meta: { rate: RATE },
});

/** Where the 10 barcode cells land in a full-canvas 160x90 render. */
const probesAt = ({ left = 0, size = 160 } = {}) => {
  const cell = size / 20;
  const row = Math.round((90 - (size * HEIGHT) / WIDTH) / 2 + cell / 2);
  return Array.from({ length: 10 }, (_, bit) => [Math.round(left + (bit + 0.5) * cell), row]);
};
const decode = (probes) => probes.reduce((code, pixel, bit) => code + ((pixel[0] + pixel[1] + pixel[2]) / 3 > 128 ? 1 << bit : 0), 0);

const draw = async (runtime, layers, probes) => {
  const id = (draw.next = (draw.next ?? 1000) + 1);
  await runtime.send({
    type: 'frame.render',
    id,
    revision: id,
    plan: { version: 1, canvas: { width: 160, height: 90 }, intent: { mode: 'sequence' }, frame: 0, layers, requires: [], warnings: [] },
    output: { width: 160, height: 90 },
    probes,
  });
  return runtime.waitFor((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === id, `frame ${id}`);
};

const readCode = async (runtime, asset, frame) => {
  const done = await draw(runtime, [videoLayer(asset, frame)], probesAt());
  assert.equal(done.type, 'frame.done', JSON.stringify(done.error));
  return decode(done.probes);
};

const needs = (t, key) => {
  if (skipUnavailable(t, assert)) return null;
  const clips = makeFixtures();
  if (!clips[key]) {
    if (process.env.TESSEL_REQUIRE_RUNTIME === '1') assert.fail(`ffmpeg could not make ${key}: ${clips[`${key}:error`] ?? 'ffmpeg missing'}`);
    t.skip(`ffmpeg could not make ${key}`);
    return null;
  }
  return clips;
};

test('video B1+B2: every frame of an H.264 clip is the frame asked for, in any order', async (t) => {
  const clips = needs(t, 'h264.mp4');
  if (!clips) return;
  const runtime = await openRuntime({ extraFiles: [clips['h264.mp4']] });
  try {
    await runtime.hello();
    const loaded = await runtime.load(videoAsset('clip', clips['h264.mp4']));
    assert.equal(loaded.state, 'ready', JSON.stringify(loaded));
    assert.equal(loaded.meta.frames, FRAMES);

    const first = await draw(runtime, [videoLayer('clip', 5)], probesAt());
    assert.ok(first.mean[0] + first.mean[1] + first.mean[2] > 3, 'the picture is not black (B1)');
    assert.ok(first.timings.video_seek_ms >= 0 && first.timings.video_read_ms >= 0, 'seek and read are measured spans');

    const orders = {
      edges: [0, FRAMES - 1, 0, 1, FRAMES - 2, 24, 23, 25],
      forward: Array.from({ length: FRAMES }, (_, frame) => frame),
      reverse: Array.from({ length: FRAMES }, (_, frame) => FRAMES - 1 - frame),
      random: shuffled(Array.from({ length: FRAMES }, (_, frame) => frame), 7),
    };
    for (const [name, order] of Object.entries(orders)) {
      // A fresh asset per order: the frame cache would otherwise answer without seeking.
      await runtime.load(videoAsset('clip', clips['h264.mp4']));
      const wrong = [];
      for (const frame of order) {
        const code = await readCode(runtime, 'clip', frame);
        if (code !== frame) wrong.push([frame, code]);
      }
      assert.deepEqual(wrong, [], `${name}: asked frame vs shown frame`);
    }
    // Past the end holds the last frame; the contract never reads beyond it.
    assert.equal(await readCode(runtime, 'clip', FRAMES + 10), FRAMES - 1);
  } finally {
    await runtime.close();
  }
});

test('video B2: two layers of one clip show their own frames in the same plan', async (t) => {
  const clips = needs(t, 'h264.mp4');
  if (!clips) return;
  const runtime = await openRuntime({ extraFiles: [clips['h264.mp4']] });
  try {
    await runtime.hello();
    assert.equal((await runtime.load(videoAsset('clip', clips['h264.mp4']))).state, 'ready');
    // The upper layer is the same clip at half size in the middle: the lower layer's barcode row
    // stays visible above it, and the upper one's is read inside it.
    const inset = { ...videoLayer('clip', 31), id: 'layer:inset', transform: transform({ scale: 0.5 }) };
    const done = await draw(runtime, [videoLayer('clip', 3), inset], [...probesAt(), ...probesAt({ left: 40, size: 80 })]);
    assert.equal(done.type, 'frame.done', JSON.stringify(done.error));
    assert.equal(decode(done.probes.slice(0, 10)), 3, 'the lower layer shows its own frame');
    assert.equal(decode(done.probes.slice(10)), 31, 'the upper layer shows its own frame');
  } finally {
    await runtime.close();
  }
});

test('video B2: a frame superseded while it seeks is cancelled and the next one is exact', async (t) => {
  const clips = needs(t, 'h264.mp4');
  if (!clips) return;
  const runtime = await openRuntime({ extraFiles: [clips['h264.mp4']] });
  try {
    await runtime.hello();
    assert.equal((await runtime.load(videoAsset('clip', clips['h264.mp4']))).state, 'ready');
    const [first, second] = [9001, 9002];
    const frameMessage = (id, frame) => ({
      type: 'frame.render',
      id,
      revision: id,
      plan: { version: 1, canvas: { width: 160, height: 90 }, intent: { mode: 'sequence' }, frame: 0, layers: [videoLayer('clip', frame)], requires: [], warnings: [] },
      output: { width: 160, height: 90 },
      probes: probesAt(),
    });
    await runtime.send(frameMessage(first, 40));
    await runtime.send(frameMessage(second, 17));
    const stale = await runtime.waitFor((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === first);
    assert.equal(stale.type, 'frame.failed');
    assert.equal(stale.error.code, 'cancelled');
    const done = await runtime.waitFor((m) => m.type === 'frame.done' && m.id === second);
    assert.equal(decode(done.probes), 17);
    // The cancelled seek did not leave the playhead somewhere that corrupts the next read.
    assert.equal(await readCode(runtime, 'clip', 40), 40);
  } finally {
    await runtime.close();
  }
});

test('video B3: capabilities promise only what is proven; failures are structured, never black', async (t) => {
  const clips = needs(t, 'h264.mp4');
  if (!clips) return;
  const runtime = await openRuntime({ extraFiles: Object.values(clips).filter((value) => typeof value === 'string' && value.endsWith('4') || String(value).endsWith('webm')) });
  try {
    const ready = await runtime.hello();
    assert.ok(!ready.capabilities.formats.video.includes('webm'), 'webm is not promised');
    assert.ok(ready.capabilities.formats.video.includes('frame_sequence'));
    assert.deepEqual(ready.capabilities.native_video.codecs, ['h264']);

    const broken = await runtime.load(videoAsset('broken', clips['broken.mp4']));
    assert.equal(broken.state, 'failed');
    assert.ok(['decode_failed', 'unsupported_format'].includes(broken.error.code), broken.error.code);

    const vfr = await runtime.load(videoAsset('vfr', clips['h264.mp4'], { vfr_conformed: true }));
    assert.equal(vfr.state, 'failed');
    assert.equal(vfr.error.code, 'unsupported_timing');

    const noRate = await runtime.load({ ...videoAsset('norate', clips['h264.mp4']), meta: {} });
    assert.equal(noRate.state, 'failed');
    assert.equal(noRate.error.code, 'unsupported_timing');

    if (clips['still.mp4']) {
      const stuck = await runtime.load({ ...videoAsset('still', clips['still.mp4']), meta: { ...videoAsset('x', '').meta, duration: { num: 2, den: 1, seconds: 2 } } });
      assert.equal(stuck.state, 'failed', 'a picture that never changes is refused, not drawn as if it played');
      assert.equal(stuck.error.code, 'unsupported_format');
    }
    assert.deepEqual(await runtime.page.evaluate(() => window.TesselRuntime.debug.loadedAssets()), []);

    // A frame of an asset that failed to load fails by name; it is not an empty picture.
    const missing = await draw(runtime, [videoLayer('broken', 0)], []);
    assert.equal(missing.type, 'frame.failed');
    assert.equal(missing.error.code, 'asset_unreadable');
  } finally {
    await runtime.close();
  }
});

test('video B3: VP9 in Chrome is proven by reading pictures, and is exact when it is', async (t) => {
  const clips = needs(t, 'vp9.webm');
  if (!clips) return;
  const runtime = await openRuntime({ extraFiles: [clips['vp9.webm']] });
  try {
    await runtime.hello();
    const loaded = await runtime.load(videoAsset('clip', clips['vp9.webm']));
    assert.equal(loaded.state, 'ready', JSON.stringify(loaded));
    for (const frame of shuffled(Array.from({ length: FRAMES }, (_, index) => index), 3).slice(0, 16)) {
      assert.equal(await readCode(runtime, 'clip', frame), frame);
    }
  } finally {
    await runtime.close();
  }
});

test('video B7: PNG and JPEG proxy sequences draw the frame asked for', async (t) => {
  const clips = needs(t, 'png');
  if (!clips) return;
  if (!clips.jpg) return t.skip('ffmpeg could not make the JPEG sequence');
  const runtime = await openRuntime({ extraPatterns: [clips.png, clips.jpg] });
  try {
    await runtime.hello();
    for (const [id, pattern] of [['png', clips.png], ['jpg', clips.jpg]]) {
      assert.equal((await runtime.load(sequenceOf(id, pattern))).state, 'ready', id);
      for (const frame of shuffled(Array.from({ length: FRAMES }, (_, index) => index), 11)) {
        assert.equal(await readCode(runtime, id, frame), frame, `${id} frame ${frame}`);
      }
    }
  } finally {
    await runtime.close();
  }
});

/** Loses the context while a frame is being drawn: right after it was queued, or after `delay` ms. */
const loseDuring = async (runtime, layers, delay) => {
  const id = (loseDuring.next = (loseDuring.next ?? 5000) + 1);
  const message = {
    v: 1,
    type: 'frame.render',
    id,
    revision: id,
    plan: { version: 1, canvas: { width: 160, height: 90 }, intent: { mode: 'sequence' }, frame: 0, layers, requires: [], warnings: [] },
    output: { width: 160, height: 90 },
  };
  await runtime.page.evaluate(
    (payload, wait) =>
      new Promise((resolve) => {
        window.TesselRuntime.receive(payload);
        if (wait === 0) {
          window.TesselRuntime.debug.loseContext();
          resolve();
          return;
        }
        setTimeout(() => {
          window.TesselRuntime.debug.loseContext();
          resolve();
        }, wait);
      }),
    message,
    delay,
  );
  const failed = await runtime.waitFor((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === id, `frame ${id}`);
  await runtime.waitFor((m) => m.type === 'context.lost');
  return failed;
};

/** One tiny red Gaussian splat (the 32-byte `.splat` layout), enough to reach the Spark path. */
const splatFile = () => {
  const out = Buffer.alloc(32);
  [0, 0, 0, 0.6, 0.6, 0.6].forEach((value, index) => out.writeFloatLE(value, index * 4));
  out.set([255, 0, 0, 255, 255, 128, 128, 128], 24);
  return out;
};

test('video B8: a context lost mid-frame is context_lost, for image, scene, splat and video', async (t) => {
  const clips = needs(t, 'h264.mp4');
  if (!clips) return;
  const project = makeProject();
  const directory = mkdtempSync(join(tmpdir(), 'tessel-loss-'));
  const splatPath = join(directory, 'one.splat');
  writeFileSync(splatPath, splatFile());
  const splatSceneAsset = join(directory, 'splat.json');
  writeFileSync(splatSceneAsset, JSON.stringify({ format: 'tessel-scene', version: 1, background: [0, 0, 0, 1], objects: [{ type: 'splat', asset: 'cloud' }] }));
  const cases = [
    { name: 'image', layers: [imageLayer('a')], delay: 0 },
    { name: 'scene', layers: [sceneLayer('box', 1)], delay: 0 },
    { name: 'splat', layers: [sceneLayer('cloudscene', 0)], delay: 0 },
    // An uncached frame: the loss lands while the frame awaits the seek.
    { name: 'video', layers: [videoLayer('clip', 33)], warm: [videoLayer('clip', 2)], delay: 1 },
  ];
  for (const { name, layers, warm = layers, delay } of cases) {
    const runtime = await openRuntime({ project, extraFiles: [clips['h264.mp4'], splatPath, splatSceneAsset] });
    try {
      await runtime.hello();
      assert.equal((await runtime.load(stillAsset('a', project.files.a))).state, 'ready');
      assert.equal((await runtime.load(sceneAsset('box', project.files.scene))).state, 'ready');
      assert.equal((await runtime.load({ id: 'cloud', kind: 'splat', source: { type: 'file', path: splatPath }, meta: { format: 'splat' } })).state, 'ready');
      assert.equal((await runtime.load({ ...sceneAsset('cloudscene', splatSceneAsset), meta: {} })).state, 'ready');
      assert.equal((await runtime.load(videoAsset('clip', clips['h264.mp4']))).state, 'ready');
      // Warm the caches so the loss has retained objects to take with it.
      assert.equal((await draw(runtime, warm, [])).type, 'frame.done', `${name}: warm-up frame`);
      const failed = await loseDuring(runtime, layers, delay);
      assert.equal(failed.type, 'frame.failed', `${name}: the frame did not report success on a dead context`);
      assert.equal(failed.error.code, 'context_lost', `${name}: ${JSON.stringify(failed.error)}`);
      await runtime.page.evaluate(() => window.TesselRuntime.debug.restoreContext());
      await runtime.waitFor((m) => m.type === 'context.restored');
      const after = await draw(runtime, layers, []);
      assert.equal(after.type, 'frame.done', `${name}: draws again after the restore (${JSON.stringify(after.error)})`);
    } finally {
      await runtime.close();
    }
  }
});
