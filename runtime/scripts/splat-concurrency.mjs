#!/usr/bin/env node
// Bench for BL091: why `splat.test.mjs` failed "intermittently" under the default `node --test`
// concurrency, and proof that the cause is the test's own race, not the runtime.
//
// The failing step released an asset "while its load is in flight" with two separate
// `page.evaluate` calls (`asset.load`, then `asset.release`). The release only cancels a load
// that has not finished; a one-splat load over loopback takes milliseconds, and the gap between
// the two CDP round trips grows with machine load. This script measures exactly that, on the
// real built runtime in headless Chrome (SwiftShader), with concurrency made INSIDE this
// process: `--browsers N` Chrome instances running at once (the default `node --test` runs ten
// test files, each with its own Chrome). It must be run under the bench lock:
//
//   python3 scripts/directed-scene-proof/with-gpu-lock.py \
//     node runtime/scripts/splat-concurrency.mjs --browsers 8 --iterations 40
//
// Modes (each in its own page, in a per-browser order shuffled by `--seed`):
//   racy       the old test: two `page.evaluate` calls, load then release (the bug as written);
//   same-task  both messages in ONE `page.evaluate` (no CDP gap): the control for the harness;
//   gated      the page's fetch of the asset is held (support.holdFetches) so the load provably
//              is in flight when the release arrives: the deterministic form the test now uses.
//
// Every iteration is classified from a log taken INSIDE the page (what `receive` got and what
// `__tesselPost` sent, in event order, with performance.now() for durations), so the verdict does
// not depend on how late the Node side noticed:
//   cancelled      the release reached the runtime first and the load answered `cancelled` (correct);
//   harness-race   `ready` was posted BEFORE the release was received: the load had finished, a
//                  release after that is legitimate and leaves nothing behind (not a runtime fault);
//   runtime-fault  `ready` was posted AFTER the release was received, or something stayed loaded:
//                  the runtime broke its contract (this is what a real bug would look like);
//   other:<code>   any other answer (context loss, resource limit, decode...), kept apart as a
//                  limit or an environment failure rather than counted as either of the above.
//
// Budget (stated, enforced): at most MAX_BROWSERS Chrome at once, one-splat assets, temp files
// only in a te-r7-* directory under the OS tmpdir (a few KiB), removed in `finally`; every
// browser and server this script opened is closed in `finally`. Nothing else is touched.
import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { encodePly, holdFetches, shuffled } from '../test/support.mjs';
import { AllowList, startAssetServer } from '../worker/assets.mjs';
import { findChrome, glFlags } from '../worker/chrome.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const MAX_BROWSERS = 12;
const MODES = ['racy', 'same-task', 'gated'];

const parseArgs = (argv) => {
  const options = { browsers: 4, iterations: 30, modes: MODES, seed: 1, out: null, quiet: false };
  for (let i = 0; i < argv.length; i += 1) {
    const [flag, inline] = argv[i].split('=');
    const value = () => inline ?? argv[++i];
    if (flag === '--browsers') options.browsers = Number(value());
    else if (flag === '--iterations') options.iterations = Number(value());
    else if (flag === '--modes') options.modes = value().split(',');
    else if (flag === '--seed') options.seed = Number(value());
    else if (flag === '--out') options.out = value();
    else if (flag === '--quiet') options.quiet = true;
    else throw new Error(`unknown option ${flag}`);
  }
  if (!Number.isInteger(options.browsers) || options.browsers < 1 || options.browsers > MAX_BROWSERS) {
    throw new Error(`--browsers must be 1..${MAX_BROWSERS} (memory budget)`);
  }
  if (!Number.isInteger(options.iterations) || options.iterations < 1 || options.iterations > 500) {
    throw new Error('--iterations must be 1..500');
  }
  for (const mode of options.modes) if (!MODES.includes(mode)) throw new Error(`unknown mode ${mode}`);
  return options;
};

const quantile = (sorted, q) => (sorted.length === 0 ? null : sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))]);
const summarize = (values) => {
  const sorted = [...values].sort((a, b) => a - b);
  const round = (v) => (v === null ? null : Math.round(v * 100) / 100);
  return { n: sorted.length, min: round(sorted[0] ?? null), p50: round(quantile(sorted, 0.5)), p95: round(quantile(sorted, 0.95)), max: round(sorted.at(-1) ?? null) };
};

/** One page with in-page logging of what the runtime received and posted. */
const openRig = async (browser, port) => {
  const page = await browser.newPage();
  const messages = [];
  await page.exposeFunction('__tesselPost', (raw) => messages.push(JSON.parse(raw)));
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.evaluate(() => {
    window.__log = [];
    const post = window.__tesselPost;
    window.__tesselPost = (raw) => {
      const m = JSON.parse(raw);
      window.__log.push({ t: performance.now(), dir: 'out', type: m.type, id: m.id, state: m.state, code: m.error?.code });
      return post(raw);
    };
    const receive = window.TesselRuntime.receive;
    window.TesselRuntime.receive = (m) => {
      window.__log.push({ t: performance.now(), dir: 'in', type: m.type, id: m.asset?.id ?? m.id });
      return receive(m);
    };
  });
  const send = (message) => page.evaluate((payload) => window.TesselRuntime.receive(payload), { v: 1, ...message });
  const settled = async (id, from) => {
    for (let attempt = 0; attempt < 1200; attempt += 1) {
      const found = messages.slice(from).find((m) => m.type === 'asset.state' && m.id === id && m.state !== 'loading');
      if (found) return found;
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
    throw new Error(`asset ${id} never settled`);
  };
  return { page, messages, send, settled };
};

/**
 * Order is the position in the page's log, which is appended in event order; the timestamps only
 * measure durations. `performance.now()` is coarse, and two events in one tick share a value.
 */
const classify = (log, final, loaded, id) => {
  const at = (predicate) => log.findIndex(predicate);
  const loadAt = at((e) => e.dir === 'in' && e.type === 'asset.load' && e.id === id);
  const releaseAt = at((e) => e.dir === 'in' && e.type === 'asset.release' && e.id === id);
  const answerAt = at((e) => e.dir === 'out' && e.type === 'asset.state' && e.id === id && e.state !== 'loading');
  if (loadAt < 0 || releaseAt < 0 || answerAt < 0) {
    return { verdict: 'runtime-fault', reason: `incomplete page log (load ${loadAt}, release ${releaseAt}, answer ${answerAt})`, gap: null, loadMs: null };
  }
  const gap = log[releaseAt].t - log[loadAt].t;
  const loadMs = log[answerAt].t - log[loadAt].t;
  const released = releaseAt < answerAt;
  if (loaded.includes(id)) return { verdict: 'runtime-fault', reason: 'asset still loaded after release', gap, loadMs };
  if (final.state === 'failed' && final.error?.code === 'cancelled') {
    if (!released) return { verdict: 'runtime-fault', reason: 'cancelled although no release had been received', gap, loadMs };
    return { verdict: 'cancelled', gap, loadMs };
  }
  if (final.state === 'ready') {
    if (!released) return { verdict: 'harness-race', gap, loadMs };
    return { verdict: 'runtime-fault', reason: 'ready posted after the release was received', gap, loadMs };
  }
  return { verdict: `other:${final.error?.code ?? final.state}`, detail: final.error?.message, gap, loadMs };
};

const runMode = async ({ mode, browser, port, plyPath, iterations, gateFor }) => {
  const rig = await openRig(browser, port);
  const results = [];
  try {
    await rig.send({ type: 'hello' });
    await rig.page.waitForFunction(() => window.__log.some((e) => e.dir === 'out' && e.type === 'ready'));
    const gate = mode === 'gated' ? await holdFetches(rig.page, gateFor(plyPath)) : null;
    const asset = { id: 'slow', kind: 'splat', source: { type: 'file', path: plyPath }, meta: { format: 'ply' } };
    for (let i = 0; i < iterations; i += 1) {
      await rig.page.evaluate(() => {
        window.__log.length = 0;
      });
      const from = rig.messages.length;
      if (mode === 'racy') {
        await rig.send({ type: 'asset.load', asset });
        await rig.send({ type: 'asset.release', id: 'slow' });
      } else if (mode === 'same-task') {
        await rig.page.evaluate((payload) => {
          window.TesselRuntime.receive({ v: 1, type: 'asset.load', asset: payload });
          window.TesselRuntime.receive({ v: 1, type: 'asset.release', id: 'slow' });
        }, asset);
      } else {
        gate.arm();
        await rig.send({ type: 'asset.load', asset });
        await gate.held();
        await rig.send({ type: 'asset.release', id: 'slow' });
        await gate.open();
      }
      const final = await rig.settled('slow', from);
      // The release was received before we looked: cancelled or released-after-ready, the asset
      // must be gone either way.
      const [log, loaded] = await Promise.all([
        rig.page.evaluate(() => window.__log.slice()),
        rig.page.evaluate(() => window.TesselRuntime.debug.loadedAssets()),
      ]);
      results.push({ mode, i, ...classify(log, final, loaded, 'slow') });
    }
  } finally {
    await rig.page.close().catch(() => {});
  }
  return results;
};

const main = async () => {
  const options = parseArgs(process.argv.slice(2));
  const directory = mkdtempSync(join(os.tmpdir(), 'te-r7-splat-'));
  const browsers = [];
  let server = null;
  try {
    const plyPath = join(directory, 'one.ply');
    writeFileSync(plyPath, encodePly([{ position: [0, 0, 0], scale: [0.6, 0.6, 0.6], color: [255, 0, 0], alpha: 255 }]));
    const allowList = new AllowList();
    allowList.register({ source: { type: 'file', path: plyPath } });
    const started = await startAssetServer({ distDir: join(root, 'dist'), allowList });
    server = started.server;
    const { port } = started;
    // The asset URL the runtime fetches: `/asset?p=<path>`.
    const gateFor = (path) => (url) => url.includes('/asset?') && decodeURIComponent(url).includes(path);

    const context = {
      node: process.version,
      cpus: os.cpus().length,
      loadavg: os.loadavg().map((v) => Math.round(v * 100) / 100),
      freememMiB: Math.round(os.freemem() / 2 ** 20),
      distSha256: createHash('sha256').update(readFileSync(join(root, 'dist/runtime.js'))).digest('hex').slice(0, 16),
      options,
    };

    for (let b = 0; b < options.browsers; b += 1) {
      browsers.push(await (await import('puppeteer-core')).default.launch({ executablePath: findChrome(), headless: true, args: glFlags('swiftshader') }));
    }
    context.chrome = await browsers[0].version();

    const all = (
      await Promise.all(
        browsers.map(async (browser, b) => {
          const order = shuffled(options.modes, options.seed + b);
          const out = [];
          for (const mode of order) {
            const rows = await runMode({ mode, browser, port, plyPath, iterations: options.iterations, gateFor });
            out.push(...rows.map((row) => ({ browser: b, ...row })));
          }
          return out;
        }),
      )
    ).flat();

    const report = { context: { ...context, loadavgAfter: os.loadavg().map((v) => Math.round(v * 100) / 100) }, modes: {}, failures: [] };
    for (const mode of options.modes) {
      const rows = all.filter((r) => r.mode === mode);
      const verdicts = {};
      for (const row of rows) verdicts[row.verdict] = (verdicts[row.verdict] ?? 0) + 1;
      report.modes[mode] = {
        iterations: rows.length,
        verdicts,
        gapMs: summarize(rows.map((r) => r.gap).filter((v) => v != null)),
        loadMs: summarize(rows.map((r) => r.loadMs).filter((v) => v != null)),
      };
    }
    report.failures = all.filter((r) => r.verdict === 'runtime-fault' || r.verdict.startsWith('other:') || (r.mode !== 'racy' && r.verdict !== 'cancelled'));
    if (options.out) {
      mkdirSync(dirname(options.out), { recursive: true });
      writeFileSync(options.out, `${JSON.stringify({ ...report, rows: all }, null, 2)}\n`);
    }
    if (!options.quiet) console.log(JSON.stringify(report, null, 2));
    process.exitCode = report.failures.length === 0 ? 0 : 1;
  } finally {
    await Promise.all(browsers.map((browser) => browser.close().catch(() => {})));
    server?.close();
    rmSync(directory, { recursive: true, force: true });
  }
};

main().catch((error) => {
  console.error(error);
  process.exitCode = 2;
});
