// Shared helpers of the browser tests: synthetic PNGs and a puppeteer page running the built
// runtime against the real asset server. Not a test file (no `.test.` in the name).
import { existsSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';

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

// ---- Splat containers -------------------------------------------------------------------
// Encoders for the three Gaussian splat containers, shared by splat.test.mjs, the concurrency
// tests and scripts/splat-concurrency.mjs so every one of them draws the same bytes.

const SH_C0 = 0.28209479177387814;
const clamp = (value, low, high) => Math.min(high, Math.max(low, value));

/** `splat`: position, scale (std dev), color 0..255 RGB, alpha 0..255. Identity rotation. */
export const encodeSplat = (splats) => {
  const out = Buffer.alloc(splats.length * 32);
  splats.forEach((s, i) => {
    const base = i * 32;
    [...s.position, ...s.scale].forEach((v, k) => out.writeFloatLE(v, base + k * 4));
    out.set([...s.color, s.alpha], base + 24);
    out.set([255, 128, 128, 128], base + 28);
  });
  return out;
};

export const encodePly = (splats) => {
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
export const encodeSpz = (splats) => {
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

export const SPLAT_ENCODERS = { splat: encodeSplat, ply: encodePly, spz: encodeSpz };

// ---- Holding a fetch in flight ----------------------------------------------------------

/**
 * Holds the page's fetches of the files `match(url)` accepts, so a test can prove a load is in
 * flight instead of betting that it has not finished yet. A load of one splat over loopback
 * finishes in milliseconds; two separate `page.evaluate` calls (one CDP round trip each) take
 * longer than that as soon as the machine is busy, which is how a "release while loading" test
 * that only raced the two calls turned into "release after ready" (BL091). Everything else the
 * page requests goes through untouched. Armed on creation; `open()` lets the held fetches (and
 * later ones) through, `arm()` holds the next matching fetch again.
 */
export const holdFetches = async (page, match) => {
  await page.setRequestInterception(true);
  const held = [];
  let holding = true;
  let wake = () => {};
  const onRequest = (request) => {
    if (!holding || !match(request.url())) {
      request.continue().catch(() => {});
      return;
    }
    held.push(request);
    wake();
  };
  page.on('request', onRequest);
  return {
    /** Resolves once a matching fetch is being held (so the runtime is awaiting its bytes). */
    held: async (timeoutMs = 15000) => {
      const deadline = Date.now() + timeoutMs;
      while (held.length === 0) {
        if (Date.now() > deadline) throw new Error('no fetch was held: the load never reached its fetch');
        await new Promise((resolve) => {
          wake = resolve;
          setTimeout(resolve, 25);
        });
      }
      return held.length;
    },
    open: async () => {
      holding = false;
      await Promise.all(held.splice(0).map((request) => request.continue().catch(() => {})));
    },
    arm: () => {
      holding = true;
    },
    /** Opens, then stops intercepting this page's requests altogether. */
    close: async () => {
      holding = false;
      await Promise.all(held.splice(0).map((request) => request.continue().catch(() => {})));
      page.off('request', onRequest);
      await page.setRequestInterception(false);
    },
  };
};

// ---- Holding a seek where it starts -----------------------------------------------------

/**
 * Keeps back the next assignment to `currentTime` of any media element, so a test can prove a
 * frame is superseded *while it waits for its seek* instead of betting that the seek has not
 * finished yet. A seek of a short clip takes tens of milliseconds, and two separate `runtime.send`
 * calls (one CDP round trip each) take longer than that as soon as the machine is busy: the first
 * frame then ended as `frame.done` before the second one replaced it (BL128). While the
 * assignment is held no `seeked` can arrive, so the only way out of the wait is the runtime's own
 * poll of "was I superseded?". The next assignment applies the held one first, in order, so the
 * engine sees the two seeks in a row that it would have seen without the hold. Armed on creation;
 * `close()` applies a seek still held and puts the engine's own accessor back.
 */
export const holdSeek = async (page) => {
  await page.evaluate(() => {
    const proto = HTMLMediaElement.prototype;
    const engine = Object.getOwnPropertyDescriptor(proto, 'currentTime');
    const gate = { armed: true, held: null, engine };
    window.__seekGate = gate;
    Object.defineProperty(proto, 'currentTime', {
      configurable: true,
      enumerable: engine.enumerable,
      get() {
        return engine.get.call(this);
      },
      set(value) {
        if (gate.armed) {
          gate.armed = false;
          gate.held = { element: this, value };
          return;
        }
        if (gate.held) {
          const { element, value: earlier } = gate.held;
          gate.held = null;
          engine.set.call(element, earlier);
        }
        engine.set.call(this, value);
      },
    });
  });
  return {
    /** Resolves once a seek is being held (so the runtime waits for a `seeked` that cannot come). */
    held: async (timeoutMs = 15000) => {
      const deadline = Date.now() + timeoutMs;
      while (!(await page.evaluate(() => Boolean(window.__seekGate?.held)))) {
        if (Date.now() > deadline) throw new Error('no seek was held: the frame never reached its seek');
        await new Promise((resolve) => setTimeout(resolve, 10));
      }
    },
    close: async () => {
      await page.evaluate(() => {
        const gate = window.__seekGate;
        if (!gate) return;
        Object.defineProperty(HTMLMediaElement.prototype, 'currentTime', gate.engine);
        if (gate.held) {
          gate.engine.set.call(gate.held.element, gate.held.value);
          gate.held = null;
        }
        delete window.__seekGate;
      });
    },
  };
};

// ---- A page for splat scenes ------------------------------------------------------------

export const splatScenePlan = (camera, time = 0) => ({
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

export const FRONT = { position: [0, 0, 5], target: [0, 0, 0], fov: 50 };
export const BACK = { position: [0, 0, -5], target: [0, 0, 0], fov: 50 };
export const CENTER = [[80, 45]];

/**
 * One page of `browser` running the built runtime against the asset server on `port`, with
 * helpers to talk bridge v1 to it. Messages are kept in `messages` (never consumed), so
 * `waitFor(predicate, label, from)` can look from an index and an id may be reloaded many times.
 */
export const openSplatPage = async (browser, port) => {
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
  return {
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
        plan: splatScenePlan(camera, time),
        output: { width: 160, height: 90 },
        probes,
      });
      return waitFor((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === id, `frame ${id}`);
    },
  };
};

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
export const openRuntime = async ({ project, hello = {}, extraFiles = [], extraPatterns = [], overlay = false } = {}) => {
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
  // `overlay: true` is the desktop page: a page driven through `__tesselPost` (the headless worker)
  // has no gizmo overlay unless it asks for one with `?overlay=gizmo` (F055).
  await page.goto(`http://127.0.0.1:${port}/${overlay ? '?overlay=gizmo' : ''}`);
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
