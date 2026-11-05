// Shared helpers of the browser tests: synthetic PNGs and a puppeteer page running the built
// runtime against the real asset server. Not a test file (no `.test.` in the name).
import { existsSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { png } from '../scripts/perf/png.mjs';
import { AllowList, startAssetServer } from '../worker/assets.mjs';
import { findChrome, glFlags } from '../worker/chrome.mjs';

export const root = join(dirname(fileURLToPath(import.meta.url)), '..');

export const unavailable = () => {
  if (!existsSync(join(root, 'dist/runtime.js'))) return 'dist/runtime.js is not built (pnpm run build)';
  try {
    findChrome();
  } catch (error) {
    return error.message;
  }
  return null;
};

/** Skips (or, with TESSEL_REQUIRE_RUNTIME=1, fails) a test when there is no Chrome or bundle. */
export const skipUnavailable = (t, assert) => {
  const reason = unavailable();
  if (!reason) return false;
  if (process.env.TESSEL_REQUIRE_RUNTIME === '1') assert.fail(reason);
  t.skip(reason);
  return true;
};

/** Distinct, structured picture per `seed`, so two sources never draw the same pixels. */
export const picture = (seed, width = 48, height = 27) =>
  png(width, height, (x, y) => [
    (x * 5 + seed * 40) % 256,
    (y * 9 + seed * 70) % 256,
    ((x + y) * 3 + seed * 100) % 256,
    255,
  ]);

/** Deterministic shuffle (mulberry32), so a failing order can be replayed. */
export const shuffled = (items, seed) => {
  let state = seed >>> 0;
  const random = () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  const out = [...items];
  for (let i = out.length - 1; i > 0; i -= 1) {
    const j = Math.floor(random() * (i + 1));
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
};

export const transform = (extra = {}) => ({ x: 0, y: 0, scale: 1, rotation: 0, crop: [0, 0, 0, 0], ...extra });
export const plan = (layers, size = [160, 90]) => ({
  version: 1,
  canvas: { width: size[0], height: size[1] },
  intent: { mode: 'sequence' },
  frame: 0,
  layers,
  requires: [],
  warnings: [],
});
export const imageLayer = (asset, extra = {}) => ({
  id: `layer:${asset}`,
  kind: 'image',
  asset,
  opacity: 1,
  blend: 'normal',
  transform: transform(),
  ...extra,
});
export const textLayer = (content, extra = {}) => ({
  id: 'layer:text',
  kind: 'text',
  opacity: 1,
  blend: 'normal',
  transform: transform({ y: 30 }),
  text: { content, size: 12, color: '#ffffff' },
  ...extra,
});
export const videoLayer = (asset, frame) => ({
  id: `layer:${asset}`,
  kind: 'video',
  asset,
  opacity: 1,
  blend: 'normal',
  transform: transform(),
  source_frame: frame,
  source_time: { seconds: frame / 24 },
});
export const sceneLayer = (asset, time = 0) => ({
  id: `layer:${asset}`,
  kind: 'scene',
  asset,
  opacity: 1,
  blend: 'normal',
  transform: transform(),
  camera: { position: [0, 0.3, 4], target: [0, 0, 0], fov: 50 },
  scene_time: { seconds: time },
});

/** A scene file with a box and a sphere (no splats: its rig may be retained). */
export const boxScene = () => ({
  format: 'tessel-scene',
  version: 1,
  background: [0.05, 0.05, 0.1, 1],
  objects: [
    { type: 'box', size: [1, 1, 1], color: '#c04040', position: [-1, 0, 0], spin: [0, 40, 0] },
    { type: 'sphere', radius: 0.6, color: '#40a0c0', position: [1, 0, 0], spin: [20, 0, 0] },
  ],
});

export const stillAsset = (id, path) => ({ id, kind: 'still', source: { type: 'file', path }, meta: {} });
export const sceneAsset = (id, path) => ({ id, kind: 'scene', source: { type: 'file', path }, meta: {} });
export const sequenceAsset = (id, pattern, count) => ({
  id,
  kind: 'video',
  source: { type: 'frame_sequence', pattern, first: 0, count },
  meta: { rate: { num: 24, den: 1 } },
});

/** A project directory with synthetic pictures, a frame sequence and a scene file. */
/** A flat-color PNG: its pixels say at a glance which source a layer read. */
export const flat = (width, height, color) => png(width, height, () => [...color, 255]);

export const makeProject = () => {
  const directory = mkdtempSync(join(tmpdir(), 'tessel-retention-'));
  const files = {
    a: join(directory, 'a.png'),
    b: join(directory, 'b.png'),
    c: join(directory, 'c.png'),
    scene: join(directory, 'box.json'),
    pattern: join(directory, 'f%05d.png'),
  };
  writeFileSync(files.a, picture(1));
  writeFileSync(files.b, picture(2, 64, 36));
  writeFileSync(files.c, picture(3));
  writeFileSync(files.scene, JSON.stringify(boxScene()));
  for (let frame = 0; frame < 6; frame += 1) {
    writeFileSync(join(directory, `f${String(frame).padStart(5, '0')}.png`), picture(10 + frame));
  }
  return { directory, files, frames: 6 };
};

/**
 * Opens the built runtime in headless Chrome (SwiftShader: the same pixels on every machine)
 * and returns helpers to talk bridge v1 to it. `close()` frees the browser and the server.
 */
export const openRuntime = async ({ project, hello = {}, extraFiles = [], extraPatterns = [] } = {}) => {
  const { default: puppeteer } = await import('puppeteer-core');
  const allowList = new AllowList();
  if (project) {
    allowList.register({ source: { type: 'file', path: project.files.a } });
    allowList.register({ source: { type: 'file', path: project.files.b } });
    allowList.register({ source: { type: 'file', path: project.files.c } });
    allowList.register({ source: { type: 'file', path: project.files.scene } });
    allowList.register({ source: { type: 'frame_sequence', pattern: project.files.pattern } });
  }
  for (const path of extraFiles) allowList.register({ source: { type: 'file', path } });
  for (const pattern of extraPatterns) allowList.register({ source: { type: 'frame_sequence', pattern } });
  const { server, port } = await startAssetServer({ distDir: join(root, 'dist'), allowList });
  const browser = await puppeteer.launch({
    executablePath: findChrome(),
    headless: true,
    args: glFlags('swiftshader'),
  });
  const page = await browser.newPage();
  const messages = [];
  await page.exposeFunction('__tesselPost', (raw) => messages.push(JSON.parse(raw)));
  await page.goto(`http://127.0.0.1:${port}/`);
  const send = (message) => page.evaluate((payload) => window.TesselRuntime.receive(payload), { v: 1, ...message });
  const waitFor = async (predicate, label = 'message') => {
    for (let attempt = 0; attempt < 600; attempt += 1) {
      const index = messages.findIndex(predicate);
      if (index !== -1) return messages.splice(index, 1)[0];
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
    throw new Error(`${label} not seen; got ${messages.map((m) => m.type).join(',')}`);
  };
  let nextId = 1;
  const runtime = {
    page,
    messages,
    send,
    waitFor,
    close: async () => {
      await browser.close();
      server.close();
    },
    hello: async (extra = hello) => {
      await send({ type: 'hello', ...extra });
      return waitFor((m) => m.type === 'ready');
    },
    load: async (asset) => {
      await send({ type: 'asset.load', asset });
      return waitFor((m) => m.type === 'asset.state' && m.id === asset.id && m.state !== 'loading');
    },
    release: (id) => send({ type: 'asset.release', id }),
    /** Attaches (or with `null` removes) a preview proxy and returns the runtime's answer. */
    attachProxy: async (id, proxy) => {
      await send({ type: 'asset.proxy', id, proxy });
      return waitFor((m) => m.type === 'asset.proxy' && m.id === id, `asset.proxy ${id}`);
    },
    /** Draws and returns `frame.done` (or `frame.failed`). `inspect: true` by default: the hash is the proof. */
    frame: async (layers, { inspect = true, size = [160, 90], id = nextId++, quality, probes } = {}) => {
      await send({
        type: 'frame.render',
        id,
        revision: id,
        plan: plan(layers, size),
        output: { width: size[0], height: size[1] },
        ...(probes ? { probes } : {}),
        ...(quality ? { quality } : {}),
        ...(inspect ? {} : { inspect: false }),
      });
      return waitFor((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === id, `frame ${id}`);
    },
    retention: () => page.evaluate(() => window.TesselRuntime.debug.retention()),
  };
  return runtime;
};
