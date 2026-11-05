// Writes the provenance of a measurement: what code produced the numbers, on what machine.
//
//   node scripts/perf/provenance.mjs --out DIR [--baseline-bundle PATH] [--raw DIR[,DIR...]]
//
// The repository has no commits yet, so hashes of the files are the only identity a result can
// carry. A number in a report is only as good as this file next to it: the same hashes mean the
// same code. `--raw` adds the browsers and the load average that the raw JSONL recorded.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { cpus, loadavg, totalmem } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const { values: opts } = parseArgs({
  options: { out: { type: 'string' }, 'baseline-bundle': { type: 'string' }, raw: { type: 'string' } },
});
if (!opts.out) throw new Error('--out DIR is required');

const sha256 = (file) => createHash('sha256').update(readFileSync(file)).digest('hex');
const sh = (command, args) => {
  try {
    return execFileSync(command, args, { encoding: 'utf8' }).trim();
  } catch {
    return null;
  }
};
const filesIn = (directory, extension) =>
  Object.fromEntries(
    readdirSync(directory)
      .filter((name) => name.endsWith(extension))
      .sort()
      .map((name) => [name, sha256(join(directory, name))]),
  );

const packageJson = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
const rawRuns = (opts.raw?.split(',') ?? []).flatMap((directory) =>
  readdirSync(directory)
    .filter((name) => name.endsWith('.jsonl'))
    .flatMap((name) => readFileSync(join(directory, name), 'utf8').split('\n').filter(Boolean).map((line) => JSON.parse(line))),
);
const loads = rawRuns.map((run) => run.loadavg?.[0]).filter(Number.isFinite).sort((a, b) => a - b);
const quantile = (share) => (loads.length ? Number(loads[Math.min(loads.length - 1, Math.floor(share * loads.length))].toFixed(2)) : null);

const provenance = {
  written_at: new Date().toISOString(),
  runtime_sources: filesIn(join(root, 'src'), '.js'),
  perf_harness: {
    ...Object.fromEntries(Object.entries(filesIn(join(root, 'scripts/perf'), '.mjs')).map(([name, hash]) => [name, hash])),
    'wk-host.swift': sha256(join(root, 'scripts/perf/wk-host.swift')),
    ...Object.fromEntries(Object.entries(filesIn(join(root, 'scripts/perf/page'), '.js')).map(([name, hash]) => [`page/${name}`, hash])),
  },
  bundles: {
    current: sha256(join(root, 'dist/runtime.js')),
    ...(opts['baseline-bundle'] ? { baseline: sha256(resolve(opts['baseline-bundle'])) } : {}),
  },
  dependencies: { three: packageJson.dependencies.three, spark: packageJson.dependencies['@sparkjsdev/spark'] },
  machine: {
    model: sh('sysctl', ['-n', 'hw.model']),
    cpu: cpus()[0]?.model ?? null,
    cores: cpus().length,
    memory_gib: Math.round(totalmem() / 2 ** 30),
    os: sh('sw_vers', ['-productVersion']),
    node: process.version,
    load_now: loadavg().map((value) => Number(value.toFixed(2))),
  },
  raw_runs: {
    count: rawRuns.length,
    browsers: [...new Set(rawRuns.map((run) => run.browser).filter(Boolean))],
    renderers: [...new Set(rawRuns.map((run) => run.renderer).filter(Boolean))],
    load_1m: { p50: quantile(0.5), p95: quantile(0.95), max: quantile(1) },
  },
};
mkdirSync(opts.out, { recursive: true });
const target = join(opts.out, 'PROVENANCE.json');
writeFileSync(target, `${JSON.stringify(provenance, null, 1)}\n`);
console.log(`wrote ${existsSync(target) ? target : '?'}`);
