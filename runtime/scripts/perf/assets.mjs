// Synthetic, deterministic assets of the performance matrix. Nothing here is a user file or a
// copy of the product's demo images: every byte comes from a seeded generator, so any machine
// builds the same files and a result can be reproduced from the numbers in the report.
//
//   node scripts/perf/assets.mjs [--dir DIR] [--heavy]
//
// `--heavy` also writes the 3M-splat file (96 MB). The directory is outside the repository
// (default: <tmp>/tessel-perf-assets) and is rebuilt only when ASSETS_VERSION changes.
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { png } from './png.mjs';

/** Bump when a generator changes: old directories are rebuilt, never silently reused. */
export const ASSETS_VERSION = 2;
export const DEFAULT_DIR = join(tmpdir(), 'tessel-perf-assets');

const lcg = (seed) => {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 4294967296;
  };
};
const gaussian = (random) => Math.sqrt(-2 * Math.log(random() + 1e-9)) * Math.cos(2 * Math.PI * random());

/** A photograph-like picture: smooth gradients, soft shapes and a little noise, so PNG size is realistic. */
const photo = (width, height, seed) => {
  const random = lcg(seed);
  return png(
    width,
    height,
    (x, y) => {
      const u = x / width;
      const v = y / height;
      const wave = Math.sin(u * 9 + seed) * Math.cos(v * 7 - seed) * 0.5 + 0.5;
      const noise = random() * 14;
      return [
        Math.min(255, 40 + 160 * u + noise),
        Math.min(255, 30 + 170 * v * wave + noise),
        Math.min(255, 60 + 150 * (1 - u) * (0.4 + wave * 0.6) + noise),
        255,
      ];
    },
    1,
  );
};

const splatFile = (count, scale, seed) => {
  const random = lcg(seed);
  const bytes = Buffer.alloc(count * 32);
  for (let i = 0; i < count; i += 1) {
    const o = i * 32;
    bytes.writeFloatLE(gaussian(random) * 0.8, o);
    bytes.writeFloatLE(gaussian(random) * 0.8, o + 4);
    bytes.writeFloatLE(gaussian(random) * 0.8, o + 8);
    bytes.writeFloatLE(scale, o + 12);
    bytes.writeFloatLE(scale, o + 16);
    bytes.writeFloatLE(scale, o + 20);
    bytes[o + 24] = 40 + random() * 215;
    bytes[o + 25] = 40 + random() * 215;
    bytes[o + 26] = 40 + random() * 215;
    bytes[o + 27] = 200;
    bytes.set([255, 128, 128, 128], o + 28);
  }
  return bytes;
};

const sceneJson = (objects, background = [0, 0, 0, 1]) =>
  JSON.stringify({ format: 'tessel-scene', version: 1, background, objects });

/** Splat counts and the scene that draws each (asset id `splat`). */
export const SPLATS = [
  { name: 's100k', count: 100_000, scale: 0.02 },
  { name: 's1m', count: 1_000_000, scale: 0.008 },
  { name: 's3m', count: 3_000_000, scale: 0.005, heavy: true },
];

/** Full-size frames of a 1080p "decoded video" and the preview proxies derived from them (OPT01/OPT06). */
export const FRAMES_1080 = 60;
export const PROXY_HEIGHT = 540;

/**
 * Proxies are made with FFmpeg like the product's derivative does (scale the decoded frames, same
 * numbering). Skipped, loudly, when FFmpeg is missing: the proxy cases then do not run.
 */
const ensureProxies = (dir, log) => {
  const marker = join(dir, 'VERSION-proxy');
  const wanted = `2:${FRAMES_1080}:${PROXY_HEIGHT}`;
  if (existsSync(marker) && readFileSync(marker, 'utf8').trim() === wanted) return;
  const probe = spawnSync('ffmpeg', ['-version'], { encoding: 'utf8' });
  if (probe.status !== 0) {
    log('ffmpeg is not available: the proxy cases will not run');
    return;
  }
  mkdirSync(join(dir, 'proxy-jpg'), { recursive: true });
  mkdirSync(join(dir, 'proxy-png'), { recursive: true });
  const width = Math.round((1920 * PROXY_HEIGHT) / 1080);
  const scale = `scale=${width}:${PROXY_HEIGHT}:flags=bicubic`;
  const input = ['-v', 'error', '-nostdin', '-y', '-start_number', '0', '-i', join(dir, 'frames1080', 'f%05d.png')];
  log('deriving the JPEG and PNG proxies');
  const jpg = spawnSync('ffmpeg', [...input, '-vf', scale, '-q:v', '3', '-start_number', '0', join(dir, 'proxy-jpg', 'p%05d.jpg')], { encoding: 'utf8' });
  const pngOut = spawnSync('ffmpeg', [...input, '-vf', scale, '-compression_level', '1', '-start_number', '0', join(dir, 'proxy-png', 'p%05d.png')], { encoding: 'utf8' });
  if (jpg.status !== 0 || pngOut.status !== 0) throw new Error(`proxy derivation failed: ${jpg.stderr}${pngOut.stderr}`);
  const count = (folder) => readdirSync(join(dir, folder)).length;
  if (count('proxy-jpg') !== FRAMES_1080 || count('proxy-png') !== FRAMES_1080) throw new Error('the proxies do not have one file per frame');
  writeFileSync(marker, `${wanted}\n`);
};

export const ensureAssets = ({ dir = DEFAULT_DIR, heavy = false, log = () => {} } = {}) => {
  mkdirSync(join(dir, 'frames'), { recursive: true });
  const marker = join(dir, 'VERSION');
  const current = existsSync(marker) ? readFileSync(marker, 'utf8').trim() : '';
  const stale = current !== String(ASSETS_VERSION);
  const write = (name, make) => {
    const path = join(dir, name);
    if (!stale && existsSync(path)) return;
    log(`writing ${name}`);
    writeFileSync(path, make());
  };

  write('image.png', () => photo(1672, 941, 1));
  write('image-alt.png', () => photo(1672, 941, 2));
  write('image4k.png', () => photo(3840, 2160, 3));
  for (let frame = 0; frame < 120; frame += 1) {
    write(`frames/f${String(frame).padStart(5, '0')}.png`, () => photo(960, 540, 100 + frame));
  }
  mkdirSync(join(dir, 'frames1080'), { recursive: true });
  for (let frame = 0; frame < FRAMES_1080; frame += 1) {
    write(`frames1080/f${String(frame).padStart(5, '0')}.png`, () => photo(1920, 1080, 300 + frame));
  }
  write(
    'box.json',
    () =>
      sceneJson(
        [
          { type: 'box', size: [1, 1, 1], color: '#c04040', position: [-1, 0, 0], spin: [0, 40, 0] },
          { type: 'sphere', radius: 0.6, color: '#40a0c0', position: [1, 0, 0], spin: [20, 0, 0] },
        ],
        [0.05, 0.05, 0.1, 1],
      ),
  );
  write(
    'box-alt.json',
    () => sceneJson([{ type: 'box', size: [1.6, 0.6, 1], color: '#40c070', position: [0, 0, 0], spin: [10, 50, 0] }], [0.1, 0.05, 0.05, 1]),
  );
  for (const { name, count, scale, heavy: isHeavy } of SPLATS) {
    if (isHeavy && !heavy) continue;
    write(`${name}.splat`, () => splatFile(count, scale, 7 + count));
    write(`${name}.json`, () => sceneJson([{ type: 'splat', asset: 'splat', position: [0, 0, 0], scale: 1 }]));
  }
  writeFileSync(marker, `${ASSETS_VERSION}\n`);
  ensureProxies(dir, log);
  return dir;
};

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const index = process.argv.indexOf('--dir');
  const dir = index === -1 ? DEFAULT_DIR : process.argv[index + 1];
  ensureAssets({ dir, heavy: process.argv.includes('--heavy'), log: (line) => process.stderr.write(`${line}\n`) });
  process.stdout.write(`${dir}\n`);
}
