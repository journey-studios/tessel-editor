// Runs the native-video proofs in the engines the product uses.
//
//   node scripts/video/run.mjs --engine webkit|chrome-gpu|chrome-swiftshader --suite verify|paced|caps|<file.json>
//        --fx DIR [--variants before,current] [--clips a.mp4,b.webm] [--out FILE] [--hz 30] [--order forward]
//
// `--fx` is a directory made by make-fixtures.py. A variant is a runtime bundle: `--rt DIR` holds
// `<variant>/dist/runtime.js` (default `/tmp/tvb2/rt`), and `current` is always this checkout's
// `runtime/dist/runtime.js`. WebKit runs in the real WKWebView host (scripts/viewport-validation/
// vv-wk.swift: `tessel://` scheme with Range, like the editor's asset server).
import { execFileSync, spawnSync } from 'node:child_process';
import { appendFileSync, createReadStream, existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, extname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

import puppeteer from 'puppeteer-core';

import { findChrome, glFlags } from '../../worker/chrome.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '../..');
const { values: opts } = parseArgs({
  options: {
    engine: { type: 'string', default: 'chrome-swiftshader' },
    suite: { type: 'string', default: 'verify' },
    fx: { type: 'string', default: '/tmp/tvb2/fx' },
    rt: { type: 'string', default: '/tmp/tvb2/rt' },
    variants: { type: 'string', default: 'current' },
    clips: { type: 'string' },
    out: { type: 'string', default: join(tmpdir(), 'tessel-video-proof.jsonl') },
    hz: { type: 'string', default: '30' },
    order: { type: 'string', default: 'forward' },
    n: { type: 'string' },
    shift: { type: 'string', default: '0' },
    offset: { type: 'string', default: '0.5' },
    rvfc: { type: 'boolean', default: false },
    blob: { type: 'boolean', default: false },
    warmup: { type: 'boolean', default: false },
    attach: { type: 'boolean', default: false },
    only: { type: 'string' },
    timeout: { type: 'string', default: '120' },
  },
});

const fx = resolve(opts.fx);
const catalog = JSON.parse(readFileSync(join(fx, 'clips.json'), 'utf8'));
const parseRate = (text) => {
  const [num, den = 1] = String(text).split('/').map(Number);
  return { num, den };
};
const clipOf = (name) => {
  const info = catalog[name];
  if (!info) throw new Error(`unknown clip ${name}; known: ${Object.keys(catalog).join(', ')}`);
  return { name, width: info.width, height: info.height, rate: parseRate(info.rate), frames: info.frames, ...(info.pattern ? { pattern: info.pattern } : { path: join(fx, name) }) };
};
const names = opts.clips ? opts.clips.split(',') : Object.keys(catalog).filter((name) => !name.includes('1080'));

const SUITES = {
  verify: () => names.flatMap((name) => ['edges', 'forward', 'reverse', 'random', 'scrub'].map((order) => ({ mode: 'verify', clip: clipOf(name), order, n: order === 'edges' ? undefined : Number(opts.n ?? catalog[name].frames), shift: Number(opts.shift) }))),
  paced: () => names.map((name) => ({ mode: 'paced', clip: clipOf(name), order: opts.order, hz: Number(opts.hz), n: Number(opts.n ?? 120) })),
  raw: () => names.map((name) => ({ mode: 'raw', clip: clipOf(name), order: opts.order === 'forward' ? 'random' : opts.order, n: Number(opts.n ?? catalog[name].frames), offset: Number(opts.offset), rvfc: opts.rvfc, blob: opts.blob, warmup: opts.warmup, attach: opts.attach, only: opts.only })),
  caps: () => [{ mode: 'caps', clip: clipOf(names[0]) }],
  loss: () => {
    // Files the loss cases read, written next to the clips (they are tiny).
    const files = { scene: join(fx, 'loss-box.json'), splat: join(fx, 'loss.splat'), splatScene: join(fx, 'loss-splat.json') };
    writeFileSync(files.scene, JSON.stringify({ format: 'tessel-scene', version: 1, background: [0.05, 0.05, 0.1, 1], objects: [{ type: 'box', size: [1, 1, 1], color: '#c04040', position: [0, 0, 0], spin: [0, 40, 0] }] }));
    const splat = Buffer.alloc(32);
    [0, 0, 0, 0.6, 0.6, 0.6].forEach((value, index) => splat.writeFloatLE(value, index * 4));
    splat.set([255, 0, 0, 255, 255, 128, 128, 128], 24);
    writeFileSync(files.splat, splat);
    writeFileSync(files.splatScene, JSON.stringify({ format: 'tessel-scene', version: 1, background: [0, 0, 0, 1], objects: [{ type: 'splat', asset: 'cloud' }] }));
    return (opts.clips ? opts.clips.split(',') : ['image', 'scene', 'splat']).map((kind) => ({ mode: 'loss', clip: clipOf('seq-png'), kind, files, canvas: [160, 90] }));
  },
};
const base = SUITES[opts.suite] ? SUITES[opts.suite]() : JSON.parse(readFileSync(opts.suite, 'utf8'));
const runs = [];
for (const variant of opts.variants.split(',')) {
  for (const run of base) runs.push({ ...run, variant, name: `${run.clip?.name}:${run.mode}:${run.order ?? ''}:${variant}`, timeoutSec: Number(opts.timeout) });
}

// Bundles by variant: `<rt>/<variant>/dist/runtime.js`, `current` is the checkout's own.
const rt = resolve(opts.rt);
mkdirSync(join(rt, 'current/dist'), { recursive: true });
const currentBundle = join(rt, 'current/dist/runtime.js');
writeFileSync(currentBundle, readFileSync(join(root, 'dist/runtime.js')));
const pageDir = join(here, 'page');
mkdirSync(dirname(opts.out), { recursive: true });
console.log(`${opts.engine} ${opts.suite}: ${runs.length} runs -> ${opts.out}`);

const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.png': 'image/png', '.jpg': 'image/jpeg', '.json': 'application/json', '.mp4': 'video/mp4', '.webm': 'video/webm' };

if (opts.engine === 'webkit') {
  // The binary's own name is the directory WebKit keeps its caches in: a directory, not a file
  const binaryDir = join(tmpdir(), 'tessel-video-proof-bin');
  mkdirSync(binaryDir, { recursive: true });
  const binary = join(binaryDir, 'vv-wk');
  const source = join(root, '../scripts/viewport-validation/vv-wk.swift');
  if (!existsSync(binary) || statSync(binary).mtimeMs < statSync(source).mtimeMs) execFileSync('swiftc', ['-O', source, '-o', binary], { stdio: 'inherit' });
  const runsFile = join(tmpdir(), `tessel-video-runs-${process.pid}.json`);
  writeFileSync(runsFile, JSON.stringify(runs));
  const pngDir = join(tmpdir(), 'tessel-video-png');
  mkdirSync(pngDir, { recursive: true });
  const result = spawnSync(binary, [runsFile, opts.out, pageDir, rt, fx, pngDir], { stdio: 'inherit' });
  process.exit(result.status ?? 1);
}

const server = createServer((request, response) => {
  const url = new URL(request.url, 'http://x');
  let file;
  if (url.pathname === '/asset') file = url.searchParams.get('p');
  else if (url.pathname.startsWith('/rt/')) file = join(rt, url.pathname.split('/')[2], 'dist/runtime.js');
  else if (url.pathname.startsWith('/page/')) file = join(pageDir, url.pathname.slice(6));
  else file = join(pageDir, 'index.html');
  if (!existsSync(file)) {
    response.writeHead(404);
    response.end();
    return;
  }
  const size = statSync(file).size;
  const headers = { 'Content-Type': TYPES[extname(file)] ?? 'application/octet-stream', 'Accept-Ranges': 'bytes', 'Cache-Control': 'no-store' };
  const range = request.headers.range;
  if (range?.startsWith('bytes=')) {
    const [from, to] = range.slice(6).split('-');
    const start = Number(from || 0);
    const end = to ? Math.min(Number(to), size - 1) : size - 1;
    response.writeHead(206, { ...headers, 'Content-Range': `bytes ${start}-${end}/${size}`, 'Content-Length': end - start + 1 });
    createReadStream(file, { start, end }).pipe(response);
    return;
  }
  response.writeHead(200, { ...headers, 'Content-Length': size });
  createReadStream(file).pipe(response);
});
await new Promise((done) => server.listen(0, '127.0.0.1', done));
const { port } = server.address();
const gl = opts.engine.replace('chrome-', '');
const browser = await puppeteer.launch({
  executablePath: findChrome(),
  headless: true,
  protocolTimeout: 900000,
  args: [...glFlags(gl), '--autoplay-policy=no-user-gesture-required', '--disable-background-timer-throttling', '--disable-renderer-backgrounding', '--no-first-run', '--window-size=640,360'],
});
for (const run of runs) {
  const page = await browser.newPage();
  await new Promise((done) => {
    const timer = setTimeout(() => {
      appendFileSync(opts.out, `${JSON.stringify({ engine: opts.engine, cfg: run, fatal: 'timeout' })}\n`);
      done();
    }, run.timeoutSec * 1000);
    page
      .exposeFunction('__vvFinish', (record) => {
        clearTimeout(timer);
        appendFileSync(opts.out, `${JSON.stringify({ ...record, engine: opts.engine })}\n`);
        done();
      })
      .then(() => page.goto(`http://127.0.0.1:${port}/index.html?cfg=${encodeURIComponent(Buffer.from(JSON.stringify({ assetsDir: fx, ...run })).toString('base64'))}`));
  });
  await page.close().catch(() => {});
}
await browser.close();
server.close();
