// Shared helpers of the lifecycle tests (lifecycle.test.mjs). Not a test file (no `.test.` in the name).
//
// Everything here drives the real runtime page through the same bridge messages the desktop
// sends; the context is lost and restored with the page's own `debug` hooks, which call the
// browser's WEBGL_lose_context extension. That is a *forced* loss of a real WebGL context, not a
// GPU reset and not a mock: the runtime sees genuine `webglcontextlost`/`webglcontextrestored`.
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { flat, openRuntime, plan, sequenceAsset, videoLayer } from './support.mjs';

export const COUNT = 6;
const pad = (index) => String(index).padStart(5, '0');

/**
 * A 4:3 sequence (48x36) on a 16:9 canvas, as in proxy-cache.test.mjs: the original is flat red
 * and the proxy flat green, so the probes say which source a picture came from. `tone` picks
 * another family of colors, which is how "another project under the same asset id" is told apart.
 */
export const makeSequences = (t, { tone = 'warm' } = {}) => {
  const directory = mkdtempSync(join(tmpdir(), 'tessel-lifecycle-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const original = join(directory, 'o%05d.png');
  const proxy = join(directory, 'p%05d.png');
  for (let frame = 0; frame < COUNT; frame += 1) {
    const originalColor = tone === 'warm' ? [200, 20 + frame * 10, 20] : [20, 20 + frame * 10, 200];
    const proxyColor = tone === 'warm' ? [20, 200, 20 + frame * 10] : [200, 200, 20 + frame * 10];
    writeFileSync(join(directory, `o${pad(frame)}.png`), flat(48, 36, originalColor));
    writeFileSync(join(directory, `p${pad(frame)}.png`), flat(24, 18, proxyColor));
  }
  return { directory, original, proxy };
};

export const PROBES = [
  [10, 45], // left bar
  [30, 45], // inside the picture
  [130, 45], // inside the picture
  [150, 45], // right bar
];

export const dominantColor = ([r, g, b]) => (r > 150 && g > 150 ? 'yellow' : r > 150 ? 'red' : g > 150 ? 'green' : b > 150 ? 'blue' : 'black');

export const proxyOf = (sequences, extra = {}) => ({ pattern: sequences.proxy, first: 0, count: COUNT, ...extra });

/** A runtime with `sequences` allowed and the original loaded as `seq` (no proxy attached yet). */
export const lifecycleRuntime = async (sequences, extra = {}) => {
  const runtime = await openRuntime({
    extraPatterns: [sequences.original, sequences.proxy, ...(extra.patterns ?? [])],
    extraFiles: extra.files ?? [],
  });
  try {
    await runtime.hello();
    const loaded = await runtime.load(sequenceAsset('seq', sequences.original, COUNT));
    if (loaded.state !== 'ready') throw new Error(`seq did not load: ${JSON.stringify(loaded)}`);
    return runtime;
  } catch (error) {
    await runtime.close();
    throw error;
  }
};

/** Sends a bridge message without waiting for any answer. */
export const post = (runtime, message) =>
  runtime.page.evaluate((payload) => window.TesselRuntime.receive(payload), { v: 1, ...message });

let nextId = 9000;
export const frameMessage = (layers, { quality, id = (nextId += 1), size = [160, 90], inspect = true } = {}) => ({
  type: 'frame.render',
  id,
  revision: id,
  plan: plan(layers, size),
  output: { width: size[0], height: size[1] },
  ...(quality ? { quality } : {}),
  ...(inspect ? {} : { inspect: false }),
});

const answered = (id) => (message) => (message.type === 'frame.done' || message.type === 'frame.failed') && message.id === id;

/**
 * Loses the context while a frame is being drawn: right after the frame was queued (`delay` 0) or
 * `delay` ms later. Returns the frame's answer and has consumed the `context.lost` message too.
 */
export const loseDuring = async (runtime, layers, { delay = 0, quality } = {}) => {
  const message = frameMessage(layers, { quality });
  await runtime.page.evaluate(
    (payload, wait) =>
      new Promise((resolve) => {
        window.TesselRuntime.receive(payload);
        const lose = () => {
          window.TesselRuntime.debug.loseContext();
          resolve();
        };
        if (wait === 0) lose();
        else setTimeout(lose, wait);
      }),
    { v: 1, ...message },
    delay,
  );
  const result = await runtime.waitFor(answered(message.id), `frame ${message.id}`);
  await runtime.waitFor((m) => m.type === 'context.lost', 'context.lost');
  return result;
};

export const loseNow = async (runtime) => {
  await runtime.page.evaluate(() => window.TesselRuntime.debug.loseContext());
  await runtime.waitFor((m) => m.type === 'context.lost', 'context.lost');
};

export const restore = async (runtime) => {
  await runtime.page.evaluate(() => window.TesselRuntime.debug.restoreContext());
  await runtime.waitFor((m) => m.type === 'context.restored', 'context.restored');
};

/** What the asset store holds for `seq`: proxy attached, and the frame keys per source. */
export const entryOf = (runtime, id = 'seq') =>
  runtime.page.evaluate((assetId) => {
    const entry = window.TesselRuntime.debug.assets.entries.get(assetId);
    if (!entry) return null;
    const keys = [...(entry.frames?.keys() ?? [])];
    return {
      proxy: entry.proxy ?? null,
      originals: keys.filter((key) => typeof key === 'number').length,
      proxies: keys.filter((key) => typeof key === 'string').length,
      textured: [...(entry.frames?.values() ?? [])].filter((frame) => frame.texture).length,
    };
  }, id);

/** GL objects three.js counts as alive on the current context (null when there is no compositor). */
export const glMemory = (runtime) => runtime.page.evaluate(() => window.TesselRuntime.debug.glMemory());

export const preview = (frame) => [videoLayer('seq', frame)];

export { flat, plan, sequenceAsset, videoLayer };
