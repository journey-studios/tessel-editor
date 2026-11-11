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
import { mkdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

import { shuffled } from '../../test/support.mjs';
import { DEFAULT_DIR, ensureAssets, photo, realSceneJson } from './assets.mjs';
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
    // Output size of every frame (default 960x540, the size the matrix was always measured at). The real project's monitor is 1280x720.
    size: { type: 'string' },
    // `preview` is what the monitor asks for while the playhead moves (MSAA 0 on scene layers); the default is the case's own.
    quality: { type: 'string' },
    // The real suite: the clouds of a real project, as PATHS on the command line. They are linked into <assets>/real, never copied.
    'real-cloud-a': { type: 'string' },
    'real-cloud-b': { type: 'string' },
    'real-ring': { type: 'string' },
    'real-sparks': { type: 'string' },
    // Optional: the third and fourth scene of the chain case.
    'real-cloud-c': { type: 'string' },
    'real-cloud-d': { type: 'string' },
    // WebKit only: one host process per run, so each run is a cold launch (see webkit.mjs).
    fresh: { type: 'boolean', default: false },
    // ms to keep the page alive after the run (a sampler attached to its processes must finish before they exit).
    'end-hold': { type: 'string' },
    // ms: record every WebGL call at least this long with the frame in flight (alone: it wraps every call).
    glcalls: { type: 'string' },
    // Record the draw configurations (program, state, formats) in the order the page first uses them (alone, like glcalls).
    psokeys: { type: 'boolean', default: false },
    // Record every Worker the page makes (Spark decodes and sorts on them) and every fetch: what a load that never ends was waiting for.
    workerlog: { type: 'boolean', default: false },
    // ms an asset load may take before the run is given up (default 60000).
    'load-timeout': { type: 'string' },
    // ms the page stays idle after a `prime-open` warm (the monitor warming the first transition when the project is opened).
    'prime-wait': { type: 'string' },
    // s the host waits for a run before it gives it up from OUTSIDE the page (default 240): a page whose main thread is frozen cannot
    // time itself out.
    'run-timeout': { type: 'string' },
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
  // BL157.4: what a frame of the real project costs when it is drawn with splats: one cloud at a film's pace, two crossfading.
  xfade: ['spark-1m-slow', 'spark-1m-xfade'],
  // BL157.4: the frames around the entry of a crossfade (a second cloud, loaded and warmed, drawn for the first time).
  entry: ['spark-1m-entry'],
  // BL157.4: the same with the real clouds of a project (`--real-cloud-a/-b`, `--real-ring`, `--real-sparks`).
  real: ['real-one', 'real-entry', 'real-cut', 'real-fade-image', 'real-prefetch'],
  // The crossfade entry later in the run (see `--pacing film`).
  late: ['real-entry', 'real-entry-100', 'real-entry-186'],
  // The four scenes of the real project in sequence (needs `--real-cloud-c/-d` as well).
  chain: ['real-chain', 'real-chain-cut', 'real-chain-loop'],
};
/** Suites whose cases need only the splat clouds: their assets are written alone (a fraction of the disk of the whole matrix). */
const SPLAT_ONLY_SUITES = new Set(['xfade', 'entry']);
const VARIANTS = {
  current: { bundle: 'current', retain: true },
  legacy: { bundle: 'current', retain: false },
  baseline: { bundle: 'baseline' },
  // The bundle given by `--baseline-bundle` with the retention the monitor runs with: the A/B of a change of the runtime itself.
  prev: { bundle: 'baseline', retain: true },
  // The same bundle with the splat result cache off: the A/B of OPT02.
  nocache: { bundle: 'current', retain: true, result_cache: false },
  // OPT07 experiment: fewer MSAA samples on scene targets (a visible quality trade).
  msaa2: { bundle: 'current', retain: true, scene_samples: 2 },
  msaa0: { bundle: 'current', retain: true, scene_samples: 0 },
  // BL157.4 (entry suite): the monitor's `frame.warm` is NOT sent ahead of the entry, and the opt-in warm draw is asked for.
  nowarm: { bundle: 'current', retain: true, entry_warm: false },
  warmdraw: { bundle: 'current', retain: true, warm_draw: true },
  // The warm of the first transition is sent as soon as the assets are ready and the page then idles (the monitor warming at project
  // open, before anybody presses play); the usual warm ahead of the transition follows, unless `prime_only`. `primescene` warms the
  // scenes only, `primeopen` also draws them.
  primeopen: { bundle: 'current', retain: true, warm_draw: true, prime_open: true },
  primeonly: { bundle: 'current', retain: true, warm_draw: true, prime_open: true, prime_only: true },
  primescene: { bundle: 'current', retain: true, prime_open: true },
  // BL157.3 texture preupload: the `frame.warm` of the entry also asks for the textures of its still images.
  preupload: { bundle: 'current', retain: true, preupload_textures: true, warm_textures: true },
};

const names = opts.variants.split(',');
const passes = Number(opts.passes);
const cases = opts.cases ? opts.cases.split(',') : SUITES[opts.suite];
if (opts.heavy && opts.suite === 'bench') cases.push('spark-3m', 'spark-3m-orbit');
const assets = resolve(opts.assets);
/** Links the real clouds under <assets>/real (a link is not a copy) and writes the two scenes that use them. */
const linkReal = (dir) => {
  const target = join(dir, 'real');
  mkdirSync(target, { recursive: true });
  const links = { 'cloud-a.ply': 'real-cloud-a', 'cloud-b.ply': 'real-cloud-b', 'ring.ply': 'real-ring', 'sparks.ply': 'real-sparks' };
  for (const [name, option] of Object.entries(links)) {
    if (!opts[option]) throw new Error(`--${option} PATH is required for the real suite`);
    rmSync(join(target, name), { force: true });
    symlinkSync(resolve(opts[option]), join(target, name));
  }
  for (const [name, option] of [['cloud-c.ply', 'real-cloud-c'], ['cloud-d.ply', 'real-cloud-d']]) {
    rmSync(join(target, name), { force: true });
    if (opts[option]) symlinkSync(resolve(opts[option]), join(target, name));
  }
  // A still for the cases that fade something other than a scene in (what an entry costs when no splat stage is involved).
  writeFileSync(join(target, 'image.png'), photo(1280, 720, 3));
  writeFileSync(join(target, 'scene-a.json'), realSceneJson('cloud-a'));
  writeFileSync(join(target, 'scene-b.json'), realSceneJson('cloud-b'));
  writeFileSync(join(target, 'scene-c.json'), realSceneJson('cloud-c'));
  writeFileSync(join(target, 'scene-d.json'), realSceneJson('cloud-d'));
};
if (['real', 'chain', 'late'].includes(opts.suite)) {
  mkdirSync(assets, { recursive: true });
  linkReal(assets);
} else {
  ensureAssets({ dir: assets, heavy: opts.heavy, only: SPLAT_ONLY_SUITES.has(opts.suite) ? 'splats' : null });
}
const [sizeWidth, sizeHeight] = opts.size ? opts.size.split('x').map(Number) : [undefined, undefined];
if (opts.size && !(sizeWidth > 0 && sizeHeight > 0)) throw new Error('--size must look like 1280x720');

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
        mode: ['bench', 'proxy', 'spark', 'msaa', 'xfade', 'entry', 'real', 'chain', 'late'].includes(opts.suite) ? 'bench' : opts.suite === 'bridge' ? 'bridge' : opts.suite,
        ...VARIANTS[variant],
        warmup: Number(opts.warmup),
        pacing: opts.pacing,
        timeout_s: name.startsWith('spark-') ? 400 : 240,
      };
      if (opts.frames) base.frames = Number(opts.frames);
      if (sizeWidth) {
        base.width = sizeWidth;
        base.height = sizeHeight;
      }
      if (opts.quality) base.quality = opts.quality;
      if (opts.drain) base.drain = opts.drain;
      if (opts.count) base.count = true;
      if (opts.glcalls) base.glcalls = Number(opts.glcalls);
      if (opts.psokeys) base.psokeys = true;
      if (opts.workerlog) base.workerlog = true;
      if (opts['load-timeout']) base.load_timeout_ms = Number(opts['load-timeout']);
      if (opts['prime-wait']) base.prime_wait_ms = Number(opts['prime-wait']);
      if (opts['run-timeout']) base.timeout_s = Number(opts['run-timeout']);
      if (opts['end-hold']) base.end_hold_ms = Number(opts['end-hold']);
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

if (opts.engine === 'webkit') await runWebKit({ plan, out, fresh: opts.fresh });
else await runChrome({ plan, out, gl: opts.engine.replace('chrome-', '') });
