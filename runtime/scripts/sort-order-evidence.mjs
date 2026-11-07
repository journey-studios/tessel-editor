#!/usr/bin/env node
// BL130 evidence: what a moving playhead's reused Spark order does to the picture, on real splat scenes.
//
//   TESSEL_GL=gpu node runtime/scripts/sort-order-evidence.mjs --plans plans.json --assets assets.json --out DIR
//        [--size 960x540] [--pace-ms 41.67[,16]] [--preview-samples 4] [--sheet 4]
//
// `plans.json` is an array of render plans of consecutive frames (the `plan` op of `tessel-headless`
// returns them: `{"op":"plan","frame":N}` -> `result`), `assets.json` the `asset.load` descriptors the
// plans need (`{id, kind, source:{type:'file',path}, meta}`), splats before the scenes that use them.
// Both are inputs of the person running it: nothing of a project is kept in this repository.
//
// It drives the real worker (bridge v1, a real browser; `TESSEL_GL=gpu` is the machine's GPU, the
// default here, `swiftshader` is software) and, per pace:
//   1. draws every plan at the final quality (the *exact* pictures; the PNG of each is kept);
//   2. settles on the first plan, then plays the plans in order, each as a `preview` frame (what the monitor
//      asks for while the playhead moves; the order of the last finished sort is reused);
//   3. compares each moving picture with the exact one of the same frame, over the whole picture (largest and mean
//      channel difference, share of pixels past 8/255, PSNR) and over each 32x32 block (the worst block's mean
//      difference: a whole-picture mean is diluted by every block the splats never reach), and the way the picture
//      changes from one frame to the next in both sequences, per block as well;
//   4. stops on the last plan and draws it at the final quality: it must be the exact picture, pixel
//      for pixel (the refine the monitor does when the playhead stops).
// MSAA is kept at `--preview-samples` (4 by default = no reduction) so that the only difference between
// a moving picture and the exact one is the order the splats were painted in (BL129 measures MSAA).
//
// What this is not. Every frame is read back (the PNG), which waits for the GPU and for the sort, so a moving frame here
// has far less sort lag than the monitor's: `--pace-ms` only spaces the requests and does not make this a stress
// test (paces of 41.67, 16 and 10 ms gave the same data). It measures what reusing an order does to a picture given
// the lag of this harness; it does not reproduce a real playback's lag, and it is not evidence that a picture does not
// pop. Plans taken every 4, 8 or 16 frames (`s4`/`s8`/`s16`) approximate a larger camera movement between frames; they
// are not a measured playback.
//
// The only failures are the invariants: the exact pictures changing with the order they are visited in, or the final
// frame differing from the exact one (`exact.same_pictures_in_reverse_order`, `final_equals_exact`); the rest is measurement,
// with no threshold of acceptance. Usage errors (a missing or unusable input) exit with status 2 before anything runs.
import { spawn } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { inflateSync } from 'node:zlib';
import { parseArgs } from 'node:util';

import { png } from './perf/png.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const USAGE = 'usage: node runtime/scripts/sort-order-evidence.mjs --plans plans.json --assets assets.json --out DIR [--size 960x540] [--pace-ms 41.67[,16]] [--preview-samples 4] [--sheet 4]';
/** A usage error: said on stderr and exit status 2, before the worker or the browser is started. */
const usage = (message) => {
  console.error(`sort-order-evidence: ${message}\n${USAGE}`);
  process.exit(2);
};
let opts;
try {
  ({ values: opts } = parseArgs({
    options: {
      plans: { type: 'string' },
      assets: { type: 'string' },
      out: { type: 'string' },
      size: { type: 'string', default: '960x540' },
      'pace-ms': { type: 'string', default: '41.67' },
      'preview-samples': { type: 'string', default: '4' },
      sheet: { type: 'string', default: '4' },
      dist: { type: 'string' },
    },
  }));
} catch (error) {
  usage(error.message);
}
for (const name of ['plans', 'assets', 'out']) {
  if (!opts[name]) usage(`--${name} is required (see the header of this file)`);
}
const readJson = (name) => {
  try {
    return JSON.parse(readFileSync(opts[name], 'utf8'));
  } catch (error) {
    return usage(`--${name} ${opts[name]} is not readable JSON: ${error.message}`);
  }
};
const plans = readJson('plans');
const assets = readJson('assets');
// A moving frame is compared with the exact one of its own frame, and its change is measured from the frame before:
// fewer than two plans measure nothing (and would only fail later, after the browser started, on an empty list).
if (!Array.isArray(plans) || plans.length < 2) usage(`--plans must be an array of at least 2 render plans (consecutive frames), got ${Array.isArray(plans) ? plans.length : typeof plans}`);
const unusablePlan = plans.findIndex((plan) => plan === null || typeof plan !== 'object' || !Number.isInteger(plan.frame) || !Array.isArray(plan.layers));
if (unusablePlan >= 0) usage(`--plans[${unusablePlan}] is not a render plan (an object with an integer \`frame\` and \`layers\`; the \`result\` of {"op":"plan"})`);
if (!Array.isArray(assets)) usage('--assets must be an array of asset.load descriptors');
const unusableAsset = assets.findIndex((asset) => asset === null || typeof asset !== 'object' || typeof asset.id !== 'string');
if (unusableAsset >= 0) usage(`--assets[${unusableAsset}] is not an asset.load descriptor (an object with a string \`id\`)`);
const sizeMatch = /^(\d+)x(\d+)$/.exec(opts.size);
if (!sizeMatch || Number(sizeMatch[1]) < 2 || Number(sizeMatch[2]) < 2) usage(`--size must be WIDTHxHEIGHT (at least 2x2), got ${JSON.stringify(opts.size)}`);
const [width, height] = [Number(sizeMatch[1]), Number(sizeMatch[2])];
const paceEntries = opts['pace-ms'].split(',');
if (paceEntries.some((entry) => entry.trim() === '')) usage('--pace-ms must not contain empty entries');
const paces = paceEntries.map(Number);
if (new Set(paces).size !== paces.length) usage('--pace-ms must not repeat a pace (artifact paths must be unique)');
if (paces.some((pace) => !Number.isFinite(pace) || pace < 0)) usage(`--pace-ms must be numbers of milliseconds, got ${JSON.stringify(opts['pace-ms'])}`);
if (!/^(0|2|4)$/.test(opts['preview-samples'])) usage(`--preview-samples must be 0, 2 or 4, got ${JSON.stringify(opts['preview-samples'])}`);
if (!/^[1-9]\d*$/.test(opts.sheet)) usage(`--sheet must be a count of frames, got ${JSON.stringify(opts.sheet)}`);
mkdirSync(opts.out, { recursive: true });

// --- a PNG decoder for what the worker writes (8-bit RGB/RGBA, no interlace) -------------------------
const decodePng = (buffer) => {
  let offset = 8;
  let header = null;
  const data = [];
  while (offset < buffer.length) {
    const length = buffer.readUInt32BE(offset);
    const type = buffer.toString('latin1', offset + 4, offset + 8);
    const body = buffer.subarray(offset + 8, offset + 8 + length);
    if (type === 'IHDR') header = { width: body.readUInt32BE(0), height: body.readUInt32BE(4), depth: body[8], color: body[9], interlace: body[12] };
    if (type === 'IDAT') data.push(body);
    offset += 12 + length;
  }
  if (!header || header.depth !== 8 || header.interlace !== 0 || ![2, 6].includes(header.color)) {
    throw new Error(`unsupported PNG ${JSON.stringify(header)}`);
  }
  const channels = header.color === 6 ? 4 : 3;
  const stride = header.width * channels;
  const raw = inflateSync(Buffer.concat(data));
  const pixels = Buffer.alloc(header.width * header.height * 4, 255);
  let previous = Buffer.alloc(stride);
  for (let row = 0; row < header.height; row += 1) {
    const filter = raw[row * (stride + 1)];
    const line = Buffer.from(raw.subarray(row * (stride + 1) + 1, (row + 1) * (stride + 1)));
    for (let i = 0; i < stride; i += 1) {
      const a = i >= channels ? line[i - channels] : 0;
      const b = previous[i];
      const c = i >= channels ? previous[i - channels] : 0;
      let predictor = 0;
      if (filter === 1) predictor = a;
      else if (filter === 2) predictor = b;
      else if (filter === 3) predictor = (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c;
        const pa = Math.abs(p - a);
        const pb = Math.abs(p - b);
        const pc = Math.abs(p - c);
        predictor = pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      line[i] = (line[i] + predictor) & 0xff;
    }
    for (let x = 0; x < header.width; x += 1) {
      const target = (row * header.width + x) * 4;
      pixels[target] = line[x * channels];
      pixels[target + 1] = line[x * channels + 1];
      pixels[target + 2] = line[x * channels + 2];
    }
    previous = line;
  }
  return { width: header.width, height: header.height, pixels };
};

/** Side of the blocks of the local metric, in pixels. */
const BLOCK = 32;

/**
 * Mean channel difference (0-255, alpha ignored) of every BLOCKxBLOCK block of two pictures of the same size, row by
 * row. The splats of a scene fill a small part of the picture; a mean over all of it says little about the block
 * where the order changed, which is what this keeps.
 */
const blockMeans = (a, b) => {
  const columns = Math.ceil(a.width / BLOCK);
  const rows = Math.ceil(a.height / BLOCK);
  const sums = new Float64Array(columns * rows);
  const counts = new Uint32Array(columns * rows);
  for (let y = 0; y < a.height; y += 1) {
    for (let x = 0; x < a.width; x += 1) {
      const block = Math.floor(y / BLOCK) * columns + Math.floor(x / BLOCK);
      const at = (y * a.width + x) * 4;
      sums[block] += Math.abs(a.pixels[at] - b.pixels[at]) + Math.abs(a.pixels[at + 1] - b.pixels[at + 1]) + Math.abs(a.pixels[at + 2] - b.pixels[at + 2]);
      counts[block] += 3;
    }
  }
  return { columns, means: Array.from(sums, (sum, block) => sum / counts[block]) };
};

/** The worst block of a `blockMeans` (or of the difference of two), where it is and how many are past 2/255. */
const worstBlock = ({ columns, means }) => {
  let worst = 0;
  means.forEach((value, block) => {
    if (Math.abs(value) > Math.abs(means[worst])) worst = block;
  });
  return {
    block_px: BLOCK,
    worst: Number(means[worst].toFixed(3)),
    at_px: [(worst % columns) * BLOCK, Math.floor(worst / columns) * BLOCK],
    blocks_past_2: means.filter((value) => Math.abs(value) > 2).length,
    blocks: means.length,
  };
};

/** Channel differences of two pictures of the same size (alpha ignored). */
const compare = (a, b) => {
  const count = a.width * a.height;
  let max = 0;
  let sum = 0;
  let squares = 0;
  let past = 0;
  const histogram = new Uint32Array(256);
  for (let i = 0; i < count; i += 1) {
    let pixelMax = 0;
    for (let channel = 0; channel < 3; channel += 1) {
      const delta = Math.abs(a.pixels[i * 4 + channel] - b.pixels[i * 4 + channel]);
      sum += delta;
      squares += delta * delta;
      if (delta > pixelMax) pixelMax = delta;
    }
    histogram[pixelMax] += 1;
    if (pixelMax > max) max = pixelMax;
    if (pixelMax > 8) past += 1;
  }
  let acc = 0;
  let p99 = 0;
  for (let level = 0; level < 256; level += 1) {
    acc += histogram[level];
    if (acc >= count * 0.99) {
      p99 = level;
      break;
    }
  }
  const mse = squares / (count * 3);
  return {
    max,
    mean: Number((sum / (count * 3)).toFixed(4)),
    p99,
    past_8_pct: Number(((100 * past) / count).toFixed(3)),
    psnr: mse === 0 ? null : Number((10 * Math.log10((255 * 255) / mse)).toFixed(2)),
    identical: sum === 0,
    local: worstBlock(blockMeans(a, b)),
  };
};

// --- the worker ------------------------------------------------------------------------------------------
const worker = spawn('node', [join(root, 'worker/worker.mjs')], {
  stdio: ['pipe', 'pipe', 'inherit'],
  env: { ...process.env, TESSEL_GL: process.env.TESSEL_GL ?? 'gpu', ...(opts.dist ? { TESSEL_RUNTIME_DIST: opts.dist } : {}) },
});
const queue = [];
const waiters = [];
let workerFailure = null;
const failWorker = (error) => {
  workerFailure ??= error instanceof Error ? error : new Error(JSON.stringify(error));
  for (const waiter of waiters.splice(0)) waiter.reject(workerFailure);
};
worker.on('error', failWorker);
worker.on('exit', (code, signal) => failWorker(new Error(`evidence worker exited (${code ?? signal})`)));
worker.stdin.on('error', failWorker);
process.once('exit', () => worker.kill());
createInterface({ input: worker.stdout }).on('line', (line) => {
  let message;
  try { message = JSON.parse(line); } catch (error) { failWorker(error); return; }
  if (message.type === 'error' || message.type === 'closed') {
    failWorker(new Error(`evidence worker ${message.type}: ${JSON.stringify(message)}`));
    return;
  }
  const index = waiters.findIndex(({ predicate }) => predicate(message));
  if (index >= 0) waiters.splice(index, 1)[0].resolve(message);
  else queue.push(message);
});
const send = (message) => worker.stdin.write(`${JSON.stringify({ v: 1, ...message })}\n`);
const waitFor = (predicate) => {
  if (workerFailure) return Promise.reject(workerFailure);
  const index = queue.findIndex(predicate);
  if (index >= 0) return Promise.resolve(queue.splice(index, 1)[0]);
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => failWorker(new Error('evidence worker reply timed out after 60 s')), 60_000);
    waiters.push({ predicate,
      resolve: (value) => { clearTimeout(timer); resolve(value); },
      reject: (error) => { clearTimeout(timer); reject(error); },
    });
  });
};
let nextId = 1;
const frame = async (plan, png_path, quality) => {
  const id = nextId++;
  send({
    type: 'frame.render',
    id,
    revision: id,
    plan,
    output: { width, height, png: png_path },
    ...(quality ? { quality } : {}),
  });
  const done = await waitFor((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === id);
  if (done.type === 'frame.failed') throw new Error(`frame failed: ${JSON.stringify(done.error)}`);
  return done;
};
const pad = (n) => String(n).padStart(4, '0');
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

send({ type: 'hello', host: 'sort-order-evidence', preview_scene_samples: Number(opts['preview-samples']) });
const ready = await waitFor((m) => m.type === 'ready');
for (const asset of assets) {
  send({ type: 'asset.load', asset });
  const state = await waitFor((m) => m.type === 'asset.state' && m.id === asset.id && m.state !== 'loading');
  if (state.state !== 'ready') throw new Error(`asset ${asset.id}: ${JSON.stringify(state.error)}`);
}

const report = {
  renderer: ready.runtime.renderer,
  runtime: ready.runtime,
  size: [width, height],
  preview_scene_samples: Number(opts['preview-samples']),
  frames: plans.map((plan) => plan.frame),
  exact: {},
  runs: [],
};

// 1. The exact pictures, in order and again (a second pass proves they do not depend on what came before).
const exactPictures = [];
for (const [index, plan] of plans.entries()) {
  const path = join(opts.out, `exact-${pad(plan.frame)}.png`);
  const done = await frame(plan, path);
  exactPictures.push({ hash: done.hash, picture: decodePng(readFileSync(path)), path });
  if (done.source.stale_sort_layers) throw new Error(`an exact frame reported stale_sort_layers ${done.source.stale_sort_layers}`);
  if (index === 0) report.exact.first_frame_source = done.source;
}
const reversed = [];
for (const index of [...plans.keys()].reverse()) {
  const done = await frame(plans[index], join(opts.out, 'scratch.png'));
  reversed.push(done.hash === exactPictures[index].hash);
}
report.exact.same_pictures_in_reverse_order = reversed.every(Boolean);

// 2-4. A moving playhead per pace.
for (const pace of paces) {
  await frame(plans[0], join(opts.out, 'scratch.png'));
  const moving = [];
  for (let index = 1; index < plans.length; index += 1) {
    const started = performance.now();
    const path = join(opts.out, `moving-p${pace}-${pad(plans[index].frame)}.png`);
    const done = await frame(plans[index], path, 'preview');
    moving.push({
      index,
      frame: plans[index].frame,
      stale: done.source.stale_sort_layers,
      reduced: done.source.reduced_scene_layers,
      render_ms: done.timings.render_ms,
      picture: decodePng(readFileSync(path)),
      path,
    });
    const wait = pace - (performance.now() - started);
    if (wait > 0) await sleep(wait);
  }
  const stats = moving.map((m) => ({ frame: m.frame, stale_sort_layers: m.stale, render_ms: Number(m.render_ms.toFixed(1)), ...compare(m.picture, exactPictures[m.index].picture) }));
  // How much the picture changes from frame to frame, in the exact sequence and in the moving one.
  const temporal = moving.map((m, i) => {
    const previousMoving = i === 0 ? { picture: exactPictures[0].picture } : moving[i - 1];
    // Per block too: how much more a block changes in the moving sequence than in the exact one (a block that
    // jumps while the picture as a whole barely changes is what the whole-picture mean hides).
    const exactChange = blockMeans(exactPictures[m.index].picture, exactPictures[m.index - 1].picture);
    const movingChange = blockMeans(m.picture, previousMoving.picture);
    return {
      frame: m.frame,
      exact: compare(exactPictures[m.index].picture, exactPictures[m.index - 1].picture).mean,
      moving: compare(m.picture, previousMoving.picture).mean,
      excess_local: worstBlock({ columns: exactChange.columns, means: movingChange.means.map((value, block) => value - exactChange.means[block]) }),
    };
  });
  const popping = temporal.map((t) => Number((t.moving - t.exact).toFixed(4)));
  // The refine: stop on the last plan and draw the final quality.
  const last = plans.length - 1;
  const refined = await frame(plans[last], join(opts.out, `refined-p${pace}-${pad(plans[last].frame)}.png`));
  report.runs.push({
    pace_ms: pace,
    frames_moving: moving.length,
    stale_frames: moving.filter((m) => m.stale > 0).length,
    render_ms: { median: Number(stats.map((s) => s.render_ms).sort((a, b) => a - b)[Math.floor(stats.length / 2)].toFixed(1)) },
    versus_exact: {
      max_channel_delta: Math.max(...stats.map((s) => s.max)),
      mean_of_means: Number((stats.reduce((sum, s) => sum + s.mean, 0) / stats.length).toFixed(4)),
      worst_mean: Math.max(...stats.map((s) => s.mean)),
      worst_past_8_pct: Math.max(...stats.map((s) => s.past_8_pct)),
      min_psnr: Math.min(...stats.map((s) => s.psnr ?? Infinity)),
      identical_frames: stats.filter((s) => s.identical).length,
      // The worst 32x32 block of any frame: where the order made the picture differ most, not diluted by the rest of it.
      worst_block_mean: Math.max(...stats.map((s) => s.local.worst)),
      max_blocks_past_2: Math.max(...stats.map((s) => s.local.blocks_past_2)),
    },
    popping: {
      note: 'whole-picture mean change from the previous frame, moving minus exact (a mean over blocks the splats never reach is diluted: read worst_block_excess; this harness reads every frame back, so it is not evidence about a real playback)',
      max: Math.max(...popping),
      min: Math.min(...popping),
      mean_abs: Number((popping.reduce((sum, v) => sum + Math.abs(v), 0) / popping.length).toFixed(4)),
      exact_mean_change: Number((temporal.reduce((sum, t) => sum + t.exact, 0) / temporal.length).toFixed(4)),
      // The largest excess change of one 32x32 block between two frames, moving minus exact (0-255).
      worst_block_excess: Math.max(...temporal.map((t) => Math.abs(t.excess_local.worst))),
    },
    final_equals_exact: refined.hash === exactPictures[last].hash,
    final_source: refined.source,
    per_frame: stats.map((s, i) => ({ ...s, popping: popping[i], popping_local: temporal[i].excess_local })),
  });

  // A contact sheet of the worst frames: exact | moving | difference x8, at half size.
  const picks = [...stats.keys()].sort((a, b) => stats[b].mean - stats[a].mean).slice(0, Number(opts.sheet)).sort((a, b) => a - b);
  const half = (picture, x, y) => {
    const sx = Math.min(picture.width - 1, x * 2);
    const sy = Math.min(picture.height - 1, y * 2);
    const at = (sy * picture.width + sx) * 4;
    return [picture.pixels[at], picture.pixels[at + 1], picture.pixels[at + 2]];
  };
  const cell = [Math.floor(width / 2), Math.floor(height / 2)];
  const sheet = png(cell[0] * 3, cell[1] * picks.length, (x, y) => {
    const row = Math.floor(y / cell[1]);
    const column = Math.floor(x / cell[0]);
    const pick = picks[row];
    const lx = x % cell[0];
    const ly = y % cell[1];
    const exact = half(exactPictures[moving[pick].index].picture, lx, ly);
    const shown = half(moving[pick].picture, lx, ly);
    if (column === 0) return [...exact, 255];
    if (column === 1) return [...shown, 255];
    return [...exact.map((v, c) => Math.min(255, Math.abs(v - shown[c]) * 8)), 255];
  });
  const sheetPath = join(opts.out, `sheet-p${pace}.png`);
  writeFileSync(sheetPath, sheet);
  report.runs.at(-1).sheet = { path: sheetPath, frames: picks.map((p) => moving[p].frame), columns: 'exact | moving | |difference| x8' };
}

send({ type: 'dispose' });
writeFileSync(join(opts.out, 'report.json'), JSON.stringify(report, null, 1));
const failed = !report.exact.same_pictures_in_reverse_order || report.runs.some((run) => !run.final_equals_exact);
console.log(JSON.stringify({ ...report, runs: report.runs.map(({ per_frame, ...rest }) => rest) }, null, 1));
process.exit(failed ? 1 : 0);
