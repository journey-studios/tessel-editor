// Proof for the Spark 2.4.0 splat path in real headless Chrome (SwiftShader), through the same
// runtime entry the desktop WebView uses. Skipped, loudly, when no Chrome is installed or the
// bundle is not built: set TESSEL_REQUIRE_RUNTIME=1 to make that a failure.
//
// What is proven here (and not just assumed from Spark's documentation):
//  - .splat, .ply and .spz all decode and draw the same splat to the same dominant color;
//  - the sort follows the camera: the nearer of two splats wins from either side;
//  - out-of-order seeks give the same pixels as in-order ones (also across fresh pages);
//  - splats compose with opaque scene objects (a box in front hides them);
//  - several splats of one asset draw; a malformed file is decode_failed; a load that is
//    released while in flight is cancelled; a lost WebGL context is recovered from.
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';
import { test } from 'node:test';

import { findChrome } from '../worker/chrome.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

const unavailable = () => {
  if (!existsSync(join(root, 'dist/runtime.js'))) return 'dist/runtime.js is not built (pnpm run build)';
  try {
    findChrome();
  } catch (error) {
    return error.message;
  }
  return null;
};

const SH_C0 = 0.28209479177387814;
const clamp = (value, low, high) => Math.min(high, Math.max(low, value));

/** `splat`: position, scale (std dev), color 0..255 RGB, alpha 0..255. Identity rotation. */
const encodeSplat = (splats) => {
  const out = Buffer.alloc(splats.length * 32);
  splats.forEach((s, i) => {
    const base = i * 32;
    [...s.position, ...s.scale].forEach((v, k) => out.writeFloatLE(v, base + k * 4));
    out.set([...s.color, s.alpha], base + 24);
    out.set([255, 128, 128, 128], base + 28);
  });
  return out;
};

const encodePly = (splats) => {
  const names = ['x', 'y', 'z', 'f_dc_0', 'f_dc_1', 'f_dc_2', 'opacity', 'scale_0', 'scale_1', 'scale_2', 'rot_0', 'rot_1', 'rot_2', 'rot_3'];
  const header = `ply\nformat binary_little_endian 1.0\nelement vertex ${splats.length}\n${names
    .map((name) => `property float ${name}`)
    .join('\n')}\nend_header\n`;
  const body = Buffer.alloc(splats.length * names.length * 4);
  splats.forEach((s, i) => {
    const alpha = clamp(s.alpha / 255, 0.001, 0.999);
    const row = [
      ...s.position,
      ...s.color.map((c) => (c / 255 - 0.5) / SH_C0),
      Math.log(alpha / (1 - alpha)),
      ...s.scale.map(Math.log),
      1, 0, 0, 0,
    ];
    row.forEach((v, k) => body.writeFloatLE(v, (i * names.length + k) * 4));
  });
  return Buffer.concat([Buffer.from(header), body]);
};

/** SPZ v2 (gzip of: header, 24-bit fixed positions, alphas, colors, scales, rotations). */
const encodeSpz = (splats) => {
  const count = splats.length;
  const fractionalBits = 12;
  const header = Buffer.alloc(16);
  header.writeUInt32LE(0x5053474e, 0);
  header.writeUInt32LE(2, 4);
  header.writeUInt32LE(count, 8);
  header.set([0, fractionalBits, 0, 0], 12);
  const positions = Buffer.alloc(count * 9);
  const alphas = Buffer.alloc(count);
  const colors = Buffer.alloc(count * 3);
  const scales = Buffer.alloc(count * 3);
  const rotations = Buffer.alloc(count * 3, 128);
  splats.forEach((s, i) => {
    s.position.forEach((v, k) => {
      const fixed = Math.round(v * (1 << fractionalBits)) & 0xffffff;
      positions.set([fixed & 0xff, (fixed >> 8) & 0xff, (fixed >> 16) & 0xff], i * 9 + k * 3);
    });
    alphas[i] = s.alpha;
    s.color.forEach((c, k) => {
      colors[i * 3 + k] = clamp(Math.round(((c / 255 - 0.5) / SH_C0 * 0.15 + 0.5) * 255), 0, 255);
    });
    s.scale.forEach((v, k) => {
      scales[i * 3 + k] = clamp(Math.round((Math.log(v) + 10) * 16), 0, 255);
    });
  });
  return gzipSync(Buffer.concat([header, positions, alphas, colors, scales, rotations]));
};

const ENCODERS = { splat: encodeSplat, ply: encodePly, spz: encodeSpz };

const blob = (position, color, alpha = 255, scale = 0.6) => ({
  position,
  scale: [scale, scale, scale],
  color,
  alpha,
});

const dominant = ([r, g, b]) => (r > g + 60 && r > b + 60 ? 'red' : g > r + 60 && g > b + 60 ? 'green' : b > r + 60 && b > g + 60 ? 'blue' : 'other');

const scenePlan = (camera, time = 0) => ({
  version: 1,
  canvas: { width: 160, height: 90 },
  intent: { mode: 'sequence' },
  frame: 0,
  layers: [
    {
      id: 'layer:scene',
      kind: 'scene',
      asset: 'scene',
      opacity: 1,
      blend: 'normal',
      transform: { x: 0, y: 0, scale: 1, rotation: 0, crop: [0, 0, 0, 0] },
      camera,
      scene_time: { seconds: time },
    },
  ],
  requires: [],
  warnings: [],
});

const FRONT = { position: [0, 0, 5], target: [0, 0, 0], fov: 50 };
const BACK = { position: [0, 0, -5], target: [0, 0, 0], fov: 50 };
const CENTER = [[80, 45]];

const openPage = async (browser, port) => {
  const page = await browser.newPage();
  const messages = [];
  await page.exposeFunction('__tesselPost', (raw) => messages.push(JSON.parse(raw)));
  await page.goto(`http://127.0.0.1:${port}/`);
  const waitFor = async (predicate, label = 'message', from = 0) => {
    for (let attempt = 0; attempt < 600; attempt += 1) {
      const found = messages.slice(from).find(predicate);
      if (found) return found;
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    throw new Error(`${label} not seen; got ${messages.map((m) => `${m.type}:${m.state ?? m.error?.code ?? ''}`).join(',')}`);
  };
  const send = (message) => page.evaluate((payload) => window.TesselRuntime.receive(payload), { v: 1, ...message });
  let frameId = 100;
  const api = {
    page,
    messages,
    waitFor,
    send,
    hello: async () => {
      await send({ type: 'hello' });
      return waitFor((m) => m.type === 'ready');
    },
    load: async (asset) => {
      // Only messages after this request count: an id is reloaded many times in one session.
      const from = messages.length;
      await send({ type: 'asset.load', asset });
      return waitFor((m) => m.type === 'asset.state' && m.id === asset.id && (m.state === 'ready' || m.state === 'failed'), `asset ${asset.id}`, from);
    },
    frame: async (camera, probes = CENTER, time = 0) => {
      frameId += 1;
      const id = frameId;
      await send({
        type: 'frame.render',
        id,
        revision: id,
        plan: scenePlan(camera, time),
        output: { width: 160, height: 90 },
        probes,
      });
      return waitFor((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === id, `frame ${id}`);
    },
  };
  return api;
};

test('spark: splat formats, sort, composition, failures and context loss in real Chrome', async (t) => {
  const reason = unavailable();
  if (reason) {
    if (process.env.TESSEL_REQUIRE_RUNTIME === '1') assert.fail(reason);
    t.skip(reason);
    return;
  }
  const { default: puppeteer } = await import('puppeteer-core');
  const { AllowList, startAssetServer } = await import('../worker/assets.mjs');
  const directory = mkdtempSync(join(tmpdir(), 'tessel-splat-'));
  const allowList = new AllowList();
  const write = (name, content) => {
    const path = join(directory, name);
    writeFileSync(path, content);
    allowList.register({ source: { type: 'file', path } });
    return path;
  };
  const asset = (id, kind, path, format) => ({ id, kind, source: { type: 'file', path }, meta: format ? { format } : {} });

  // One red splat at the origin, in each container.
  const red = [blob([0, 0, 0], [255, 0, 0])];
  const formatPaths = Object.fromEntries(
    Object.entries(ENCODERS).map(([format, encode]) => [format, write(`red.${format}`, encode(red))]),
  );
  // Red near the +z camera, green behind it, same screen position: order decides the color.
  const stacked = write('stacked.splat', encodeSplat([blob([0, 0, 1], [255, 0, 0]), blob([0, 0, -1], [0, 255, 0])]));
  // A splat behind an opaque box that sits between it and the camera.
  const hidden = write('hidden.splat', encodeSplat([blob([0, 0, 0], [255, 0, 0])]));
  const scenes = {
    single: write('single.json', JSON.stringify({ format: 'tessel-scene', version: 1, background: [0, 0, 0, 1], objects: [{ type: 'splat', asset: 'cloud' }] })),
    boxed: write(
      'boxed.json',
      JSON.stringify({
        format: 'tessel-scene',
        version: 1,
        background: [0, 0, 0, 1],
        objects: [
          { type: 'splat', asset: 'cloud' },
          { type: 'box', size: [3, 3, 0.2], position: [0, 0, 2], color: '#0000ff' },
        ],
      }),
    ),
    twin: write(
      'twin.json',
      JSON.stringify({
        format: 'tessel-scene',
        version: 1,
        background: [0, 0, 0, 1],
        objects: [
          { type: 'splat', asset: 'cloud', position: [-1.6, 0, 0], scale: 0.7 },
          { type: 'splat', asset: 'cloud', position: [1.6, 0, 0], scale: 0.7 },
        ],
      }),
    ),
  };
  // Translucent red in front of translucent green: the blend is order dependent, and it spins.
  const veil = write('veil.splat', encodeSplat([blob([0, 0, 1], [255, 0, 0], 140), blob([0, 0, -1], [0, 255, 0], 140)]));
  scenes.spinning = write(
    'spinning.json',
    JSON.stringify({ format: 'tessel-scene', version: 1, background: [0, 0, 0, 1], objects: [{ type: 'splat', asset: 'cloud', spin: [0, 50, 0] }] }),
  );
  const broken = write('broken.splat', Buffer.alloc(33));
  const sceneAsset = (name) => asset('scene', 'scene', scenes[name]);

  const { server, port } = await startAssetServer({ distDir: join(root, 'dist'), allowList });
  const browser = await puppeteer.launch({
    executablePath: findChrome(),
    headless: true,
    args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
  });
  try {
    const rig = await openPage(browser, port);
    const ready = await rig.hello();
    assert.equal(ready.runtime.spark, '2.4.0');
    assert.deepEqual(ready.capabilities.formats.splat, ['splat', 'ply', 'spz']);

    // 1. Every container draws the same red blob (and the corner stays background black).
    const centers = {};
    for (const [format, path] of Object.entries(formatPaths)) {
      const loaded = await rig.load(asset('cloud', 'splat', path, format));
      assert.equal(loaded.state, 'ready', JSON.stringify(loaded));
      assert.equal(loaded.meta.splat_count, 1, `${format} splat count`);
      assert.equal((await rig.load(sceneAsset('single'))).state, 'ready');
      const done = await rig.frame(FRONT, [...CENTER, [2, 2]]);
      assert.equal(done.type, 'frame.done', JSON.stringify(done));
      centers[format] = done.probes[0];
      assert.equal(dominant(done.probes[0]), 'red', `${format} center ${done.probes[0]}`);
      assert.ok(done.probes[0][0] > 200, `${format} is nearly opaque red: ${done.probes[0]}`);
      assert.deepEqual(done.probes[1].slice(0, 3), [0, 0, 0], `${format} leaves the background alone`);
    }
    const spread = Math.max(...Object.values(centers).map((c) => c[0])) - Math.min(...Object.values(centers).map((c) => c[0]));
    assert.ok(spread < 40, `formats agree on the red level: ${JSON.stringify(centers)}`);

    // 2. Sort follows the camera: the nearer splat wins from each side, in any visiting order.
    assert.equal((await rig.load(asset('cloud', 'splat', stacked, 'splat'))).state, 'ready');
    assert.equal((await rig.load(sceneAsset('single'))).state, 'ready');
    const hashes = { front: [], back: [] };
    for (const [name, camera] of [['front', FRONT], ['back', BACK], ['front', FRONT], ['back', BACK], ['front', FRONT]]) {
      const done = await rig.frame(camera);
      assert.equal(done.type, 'frame.done', JSON.stringify(done));
      assert.equal(dominant(done.probes[0]), name === 'front' ? 'red' : 'green', `${name} sees ${done.probes[0]}`);
      hashes[name].push(done.hash);
    }
    assert.equal(new Set(hashes.front).size, 1, 'the same camera is the same picture however it was reached');
    assert.equal(new Set(hashes.back).size, 1);
    // A fresh page asked for BACK first, then FRONT (reverse order), must match bit for bit.
    const fresh = await openPage(browser, port);
    await fresh.hello();
    await fresh.load(asset('cloud', 'splat', stacked, 'splat'));
    await fresh.load(sceneAsset('single'));
    assert.equal((await fresh.frame(BACK)).hash, hashes.back[0]);
    assert.equal((await fresh.frame(FRONT)).hash, hashes.front[0]);
    await fresh.page.close();

    // Transparency: translucent splats blend back to front, so each side leans to its near color.
    assert.equal((await rig.load(asset('cloud', 'splat', veil, 'splat'))).state, 'ready');
    const veiledFront = await rig.frame(FRONT);
    const veiledBack = await rig.frame(BACK);
    const [fr, fg] = veiledFront.probes[0];
    const [br, bg] = veiledBack.probes[0];
    assert.ok(fr > fg && fg > 20, `translucent: red over green leans red but shows green ${veiledFront.probes[0]}`);
    assert.ok(bg > br && br > 20, `translucent: green over red leans green but shows red ${veiledBack.probes[0]}`);
    assert.ok(fr < 250, 'a translucent splat is not drawn opaque');
    // Time-driven motion: seeking 0 -> 2s -> 1s -> 0 returns the t=0 frame exactly.
    assert.equal((await rig.load(sceneAsset('spinning'))).state, 'ready');
    const seen = {};
    for (const time of [0, 2, 1, 0, 2, 1]) {
      const done = await rig.frame(FRONT, CENTER, time);
      assert.equal(done.type, 'frame.done', JSON.stringify(done));
      assert.ok(seen[time] === undefined || seen[time] === done.hash, `t=${time}s repeats bit for bit`);
      seen[time] = done.hash;
    }
    assert.equal(new Set(Object.values(seen)).size, 3, 'different times are different pictures');
    assert.equal((await rig.load(sceneAsset('single'))).state, 'ready');

    // 3. Opaque scene objects occlude splats behind them; the same splat is seen without the box.
    assert.equal((await rig.load(asset('cloud', 'splat', hidden, 'splat'))).state, 'ready');
    assert.equal((await rig.load(sceneAsset('boxed'))).state, 'ready');
    const boxed = await rig.frame(FRONT);
    assert.equal(dominant(boxed.probes[0]), 'blue', `box in front hides the splat: ${boxed.probes[0]}`);
    const behind = await rig.frame(BACK);
    assert.equal(dominant(behind.probes[0]), 'red', `the splat is seen from the box's far side: ${behind.probes[0]}`);

    // 4. One asset drawn twice in one scene: both copies are on screen (split screen probes).
    assert.equal((await rig.load(sceneAsset('twin'))).state, 'ready');
    const twin = await rig.frame(FRONT, [[52, 45], [108, 45], [80, 4]]);
    assert.equal(dominant(twin.probes[0]), 'red', `left copy ${twin.probes[0]}`);
    assert.equal(dominant(twin.probes[1]), 'red', `right copy ${twin.probes[1]}`);
    assert.deepEqual(twin.probes[2].slice(0, 3), [0, 0, 0]);

    // 5. A malformed file is decode_failed, and the session keeps working.
    const malformed = await rig.load(asset('bad', 'splat', broken, 'splat'));
    assert.equal(malformed.state, 'failed');
    assert.equal(malformed.error.code, 'decode_failed', JSON.stringify(malformed.error));

    // 6. Releasing an asset while its load is in flight cancels the load.
    await rig.send({ type: 'asset.load', asset: asset('slow', 'splat', formatPaths.ply, 'ply') });
    await rig.send({ type: 'asset.release', id: 'slow' });
    const cancelled = await rig.waitFor((m) => m.type === 'asset.state' && m.id === 'slow' && m.state !== 'loading');
    assert.equal(cancelled.state, 'failed');
    assert.equal(cancelled.error.code, 'cancelled', JSON.stringify(cancelled.error));
    const loadedIds = await rig.page.evaluate(() => window.TesselRuntime.debug.loadedAssets());
    assert.equal(loadedIds.includes('slow'), false, 'a cancelled load leaves nothing behind');

    // 7. A lost WebGL context is recovered from: the same frame, bit for bit, after restore.
    assert.equal((await rig.load(asset('cloud', 'splat', stacked, 'splat'))).state, 'ready');
    assert.equal((await rig.load(sceneAsset('single'))).state, 'ready');
    const before = await rig.frame(FRONT);
    await rig.page.evaluate(() => window.TesselRuntime.debug.loseContext());
    await rig.waitFor((m) => m.type === 'context.lost');
    const lostAt = rig.messages.length;
    const refused = await rig.frame(FRONT);
    assert.equal(refused.error?.code, 'context_lost');
    await rig.page.evaluate(() => window.TesselRuntime.debug.restoreContext());
    await rig.waitFor((m) => m.type === 'context.restored', 'restore', lostAt);
    const after = await rig.frame(FRONT);
    assert.equal(after.type, 'frame.done', JSON.stringify(after));
    assert.equal(after.hash, before.hash, 'the splat frame is identical after the context came back');
  } finally {
    await browser.close();
    server.close();
  }
});
