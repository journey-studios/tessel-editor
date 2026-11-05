// Orchestrates the performance matrix.
//
//   node scripts/perf/run.mjs --engine webkit|chrome-gpu|chrome-swiftshader --suite bench|hash|life|probe|bridge
//        [--variants current,legacy,baseline] [--baseline-bundle PATH] [--passes 3] [--first-pass 1] [--frames 300]
//        [--cases image,text] [--drain read1x1|fence] [--count] [--out DIR] [--assets DIR]
//
// Variants (same host page, same assets, only the runtime differs):
//   current   dist/runtime.js with retention on (the default)
//   legacy    dist/runtime.js with `hello.retain=false`: the per-frame create/dispose path
//   baseline  a runtime built from the sources before this work (--baseline-bundle)
// Runs of a suite are interleaved per case and the variant order flips every pass (A/B/A), so
// a drift in machine load shows up as disagreement between passes instead of as a "win".
// Raw results go to <out>/<engine>-<suite>.jsonl, one line per run, discarded runs included.
import { mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

import { shuffled } from '../../test/support.mjs';
import { DEFAULT_DIR, ensureAssets } from './assets.mjs';
import { runChrome } from './chrome.mjs';
import { runWebKit } from './webkit.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const { values: opts } = parseArgs({
  options: {
    engine: { type: 'string', default: 'chrome-swiftshader' },
    suite: { type: 'string', default: 'bench' },
    variants: { type: 'string', default: 'current,legacy' },
    'baseline-bundle': { type: 'string' },
    passes: { type: 'string', default: '3' },
    // Resume a matrix that stopped: pass N keeps its variant order (A/B/A parity) whatever the start.
    'first-pass': { type: 'string', default: '1' },
    frames: { type: 'string' },
    warmup: { type: 'string', default: '10' },
    cases: { type: 'string' },
    drain: { type: 'string' },
    count: { type: 'boolean', default: false },
    pacing: { type: 'string', default: 'raf' },
    out: { type: 'string', default: join(tmpdir(), 'tessel-perf-results') },
    assets: { type: 'string', default: DEFAULT_DIR },
    heavy: { type: 'boolean', default: false },
    seed: { type: 'string', default: '1' },
  },
});

const SUITES = {
  bench: ['image', 'image4k', 'image-2layers', 'text', 'text-changing', 'image+text', 'video-hit', 'video-miss', 'scene', 'spark-100k', 'spark-100k-orbit', 'spark-1m', 'spark-1m-orbit'],
  hash: ['image', 'image+text', 'text-changing', 'video-miss', 'scene', 'spark-100k-orbit'],
  life: ['image', 'image-2layers', 'image+text', 'video-hit', 'scene'],
  bridge: ['image', 'scene'],
  probe: ['image'],
  // OPT01/OPT06: scrubbing 1080p decoded frames, original against the 540p JPEG/PNG proxy.
  proxy: ['video1080-fwd', 'video1080-fwd-jpg', 'video1080-fwd-png', 'video1080-rand', 'video1080-rand-jpg', 'video1080-rand-png'],
  // OPT02: a splat scene that does not change, with and without the result cache.
  spark: ['spark-100k', 'spark-1m'],
  // OPT07: splat scenes in motion at 4, 2 and 0 MSAA samples.
  msaa: ['spark-100k-orbit', 'spark-1m-orbit'],
};
const VARIANTS = {
  current: { bundle: 'current', retain: true },
  legacy: { bundle: 'current', retain: false },
  baseline: { bundle: 'baseline' },
  // The same bundle with the splat result cache off: the A/B of OPT02.
  nocache: { bundle: 'current', retain: true, result_cache: false },
  // OPT07 experiment: fewer MSAA samples on scene targets (a visible quality trade).
  msaa2: { bundle: 'current', retain: true, scene_samples: 2 },
  msaa0: { bundle: 'current', retain: true, scene_samples: 0 },
};

const names = opts.variants.split(',');
const passes = Number(opts.passes);
const cases = opts.cases ? opts.cases.split(',') : SUITES[opts.suite];
if (opts.heavy && opts.suite === 'bench') cases.push('spark-3m', 'spark-3m-orbit');
const assets = resolve(opts.assets);
ensureAssets({ dir: assets, heavy: opts.heavy });

const bundles = { current: join(root, 'dist/runtime.js') };
if (opts['baseline-bundle']) bundles.baseline = resolve(opts['baseline-bundle']);
for (const name of names) {
  if (VARIANTS[name].bundle === 'baseline' && !bundles.baseline) throw new Error('--baseline-bundle is required for the baseline variant');
}

if (opts.suite === 'bridge' && opts.engine !== 'webkit') throw new Error('the bridge suite needs --engine webkit (native round trip)');

const runs = [];
for (let pass = Number(opts['first-pass']); pass <= passes; pass += 1) {
  const order = pass % 2 === 1 ? names : [...names].reverse();
  for (const name of cases) {
    for (const variant of order) {
      const base = {
        name: `${name}/${variant}`,
        case: name,
        variant,
        pass,
        mode: ['bench', 'proxy', 'spark', 'msaa'].includes(opts.suite) ? 'bench' : opts.suite === 'bridge' ? 'bridge' : opts.suite,
        ...VARIANTS[variant],
        warmup: Number(opts.warmup),
        pacing: opts.pacing,
        timeout_s: name.startsWith('spark-') ? 400 : 240,
      };
      if (opts.frames) base.frames = Number(opts.frames);
      if (opts.drain) base.drain = opts.drain;
      if (opts.count) base.count = true;
      if (opts.suite === 'bridge') base.bridge = true;
      // A different shuffle per pass and case, the same one for every variant of that pass:
      // variants must agree with each other under one order, and with themselves across orders.
      if (opts.suite === 'hash') base.order = shuffled([0, 1, 2, 3, 4, 5], Number(opts.seed) * 1000 + pass * 17 + name.length);
      runs.push(base);
    }
  }
}

mkdirSync(opts.out, { recursive: true });
const out = join(opts.out, `${opts.engine}-${opts.suite}.jsonl`);
const plan = { assets, bundles, runs };
console.log(`${opts.engine} ${opts.suite}: ${runs.length} runs -> ${out}`);

if (opts.engine === 'webkit') await runWebKit({ plan, out });
else await runChrome({ plan, out, gl: opts.engine.replace('chrome-', '') });
