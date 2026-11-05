// Drives the real worker and a real Chrome. Skipped, loudly, when no Chrome is installed or
// the bundle is not built: set TESSEL_REQUIRE_RUNTIME=1 to make that a failure.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { deflateSync } from 'node:zlib';
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

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buffer) => {
  let c = 0xffffffff;
  for (const byte of buffer) c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const body = Buffer.concat([Buffer.from(type), data]);
  const out = Buffer.alloc(8 + data.length + 4);
  out.writeUInt32BE(data.length, 0);
  body.copy(out, 4);
  out.writeUInt32BE(crc32(body), 8 + data.length);
  return out;
};
/** A 2x1 PNG: one red pixel, one blue pixel. */
const redBluePng = () => {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(2, 0);
  header.writeUInt32BE(1, 4);
  header.set([8, 6, 0, 0, 0], 8);
  const pixels = Buffer.from([0, 255, 0, 0, 255, 0, 0, 255, 255]);
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', deflateSync(pixels)),
    chunk('IEND', Buffer.alloc(0)),
  ]);
};

const startWorker = () => {
  const child = spawn('node', [join(root, 'worker/worker.mjs')], { stdio: ['pipe', 'pipe', 'pipe'] });
  const queue = [];
  const waiters = [];
  createInterface({ input: child.stdout }).on('line', (line) => {
    const message = JSON.parse(line);
    const waiter = waiters.shift();
    if (waiter) waiter(message);
    else queue.push(message);
  });
  return {
    send: (message) => child.stdin.write(`${JSON.stringify({ v: 1, ...message })}\n`),
    next: (timeout = 60_000) =>
      queue.length
        ? Promise.resolve(queue.shift())
        : new Promise((resolve, reject) => {
            const timer = setTimeout(() => reject(new Error('no message from the worker')), timeout);
            waiters.push((message) => {
              clearTimeout(timer);
              resolve(message);
            });
          }),
    until: async function until(type) {
      for (;;) {
        const message = await this.next();
        if (message.type === type) return message;
      }
    },
    exited: new Promise((resolve) => child.on('exit', resolve)),
    child,
  };
};

const plan = (layers) => ({
  version: 1,
  canvas: { width: 4, height: 2 },
  intent: { mode: 'sequence' },
  frame: 0,
  layers,
  requires: [],
  warnings: [],
});
const imageLayer = (extra = {}) => ({
  id: 'layer:a',
  kind: 'image',
  asset: 'img',
  opacity: 1,
  blend: 'normal',
  transform: { x: 0, y: 0, scale: 1, rotation: 0, crop: [0, 0, 0, 0] },
  ...extra,
});

test('worker: handshake, asset, frame with probes and PNG, stale cancel, version refusal', async (t) => {
  const reason = unavailable();
  if (reason) {
    if (process.env.TESSEL_REQUIRE_RUNTIME === '1') assert.fail(reason);
    t.skip(reason);
    return;
  }
  const directory = mkdtempSync(join(tmpdir(), 'tessel-worker-'));
  const image = join(directory, 'rb.png');
  writeFileSync(image, redBluePng());

  const worker = startWorker();
  try {
    worker.send({ type: 'hello' });
    const ready = await worker.until('ready');
    assert.equal(ready.runtime.name, 'tessel-three');
    assert.deepEqual(ready.capabilities.layers, ['image', 'video', 'text', 'scene', 'waveform']);

    worker.send({ type: 'asset.load', asset: { id: 'img', kind: 'still', source: { type: 'file', path: image }, meta: {} } });
    const loaded = await worker.until('asset.state').then(() => worker.until('asset.state'));
    assert.equal(loaded.state, 'ready');
    assert.deepEqual([loaded.meta.width, loaded.meta.height], [2, 1]);

    // 4x2 canvas, 2x1 image, contain fit: scale 2, so each source pixel is 2x2 output pixels.
    // Only the outer columns are probed: the middle two blend red into blue (linear filter).
    const png = join(directory, 'out', 'frame.png');
    worker.send({
      type: 'frame.render',
      id: 1,
      revision: 1,
      plan: plan([imageLayer()]),
      output: { width: 4, height: 2, png },
      probes: [[0, 0], [0, 1], [3, 0], [3, 1]],
    });
    const done = await worker.until('frame.done');
    assert.deepEqual(done.probes, [[255, 0, 0, 255], [255, 0, 0, 255], [0, 0, 255, 255], [0, 0, 255, 255]]);
    assert.equal(done.png, png);
    assert.equal(readFileSync(png).subarray(1, 4).toString(), 'PNG');
    assert.match(done.hash, /^[0-9a-f]{16}$/);
    assert.equal(done.inspected, true);

    // The monitor's interactive frames ask not to be read back: drawn and settled on the GPU,
    // answered without hash/mean/probes, with the time the GPU wait took.
    const drawOnly = { type: 'frame.render', id: 2, revision: 2, plan: plan([imageLayer()]), output: { width: 4, height: 2 }, inspect: false };
    worker.send(drawOnly);
    const light = await worker.until('frame.done');
    assert.equal(light.inspected, false);
    assert.equal(light.hash, null);
    assert.deepEqual(light.probes, []);
    assert.equal(typeof light.timings.settle_ms, 'number');
    assert.equal(light.timings.readback_ms, undefined, 'no readback happened');
    assert.deepEqual([light.width, light.height], [4, 2]);

    // Probes or a PNG are explicit requests for pixels: the flag cannot take them away.
    worker.send({ ...drawOnly, id: 3, revision: 3, probes: [[0, 0]] });
    const probed = await worker.until('frame.done');
    assert.equal(probed.inspected, true);
    assert.deepEqual(probed.probes, [[255, 0, 0, 255]]);
    worker.send({ ...drawOnly, id: 5, revision: 5, output: { width: 4, height: 2, png: join(directory, 'out', 'inspect-false.png') } });
    const exported = await worker.until('frame.done');
    assert.equal(exported.inspected, true);
    assert.match(exported.hash, /^[0-9a-f]{16}$/);
    assert.equal(readFileSync(join(directory, 'out', 'inspect-false.png')).subarray(1, 4).toString(), 'PNG');

    // An asset that does not exist fails by id with the file in the message.
    worker.send({ type: 'asset.load', asset: { id: 'gone', kind: 'still', source: { type: 'file', path: join(directory, 'gone.png') }, meta: {} } });
    const failed = await worker.until('asset.state').then(() => worker.until('asset.state'));
    assert.equal(failed.state, 'failed');
    assert.ok(['asset_unreadable'].includes(failed.error.code), JSON.stringify(failed));

    // A layer that uses a blend mode the runtime cannot draw fails the frame, not the process.
    worker.send({ type: 'frame.render', id: 4, revision: 4, plan: plan([imageLayer({ blend: 'overlay' })]), output: { width: 4, height: 2 } });
    const refused = await worker.until('frame.failed');
    assert.equal(refused.error.code, 'unsupported_format');

    worker.child.stdin.write(`${JSON.stringify({ v: 2, type: 'hello' })}\n`);
    const mismatch = await worker.until('error');
    assert.equal(mismatch.error.code, 'version_mismatch');

    worker.send({ type: 'dispose' });
    assert.equal((await worker.until('closed')).reason, 'disposed');
    assert.equal(await worker.exited, 0);
  } finally {
    worker.child.kill();
  }
});

test('worker: losing the WebGL context is reported and rendering resumes after restore', async (t) => {
  const reason = unavailable();
  if (reason) {
    if (process.env.TESSEL_REQUIRE_RUNTIME === '1') assert.fail(reason);
    t.skip(reason);
    return;
  }
  const { default: puppeteer } = await import('puppeteer-core');
  const { AllowList, startAssetServer } = await import('../worker/assets.mjs');
  const directory = mkdtempSync(join(tmpdir(), 'tessel-context-'));
  const image = join(directory, 'rb.png');
  writeFileSync(image, redBluePng());
  const allowList = new AllowList();
  allowList.register({ source: { type: 'file', path: image } });
  const { server, port } = await startAssetServer({ distDir: join(root, 'dist'), allowList });
  const browser = await puppeteer.launch({
    executablePath: findChrome(),
    headless: true,
    args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
  });
  try {
    const page = await browser.newPage();
    const messages = [];
    await page.exposeFunction('__tesselPost', (raw) => messages.push(JSON.parse(raw)));
    await page.goto(`http://127.0.0.1:${port}/`);
    const send = (message) => page.evaluate((payload) => window.TesselRuntime.receive(payload), { v: 1, ...message });
    const waitFor = async (predicate) => {
      for (let attempt = 0; attempt < 300; attempt += 1) {
        const found = messages.find(predicate);
        if (found) return found;
        await new Promise((resolve) => setTimeout(resolve, 50));
      }
      throw new Error(`message not seen; got ${messages.map((m) => m.type).join(',')}`);
    };
    await send({ type: 'hello' });
    await waitFor((m) => m.type === 'ready');
    await send({ type: 'asset.load', asset: { id: 'img', kind: 'still', source: { type: 'file', path: image }, meta: {} } });
    await waitFor((m) => m.type === 'asset.state' && m.state === 'ready');

    // Two requests delivered in the same tick: the first is overtaken before it starts.
    await page.evaluate((frames) => {
      frames.forEach((frame) => window.TesselRuntime.receive(frame));
    }, [10, 11].map((id) => ({ v: 1, type: 'frame.render', id, revision: id, plan: plan([imageLayer()]), output: { width: 4, height: 2 } })));
    const overtaken = await waitFor((m) => m.type === 'frame.failed' && m.id === 10);
    assert.equal(overtaken.error.code, 'cancelled');
    await waitFor((m) => m.type === 'frame.done' && m.id === 11);

    await page.evaluate(() => window.TesselRuntime.debug.loseContext());
    await waitFor((m) => m.type === 'context.lost');
    await send({ type: 'frame.render', id: 1, revision: 1, plan: plan([imageLayer()]), output: { width: 4, height: 2 } });
    const refused = await waitFor((m) => m.type === 'frame.failed' && m.id === 1);
    assert.equal(refused.error.code, 'context_lost');

    await page.evaluate(() => window.TesselRuntime.debug.restoreContext());
    await waitFor((m) => m.type === 'context.restored');
    await send({ type: 'frame.render', id: 2, revision: 2, plan: plan([imageLayer()]), output: { width: 4, height: 2 }, probes: [[0, 0], [3, 1]] });
    const done = await waitFor((m) => m.type === 'frame.done' && m.id === 2);
    assert.deepEqual(done.probes, [[255, 0, 0, 255], [0, 0, 255, 255]]);
  } finally {
    await browser.close();
    server.close();
  }
});
