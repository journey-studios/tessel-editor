// Runs the same plan in WKWebView (the engine the desktop editor embeds) through wk-host.swift.
// The host is compiled once into a temp directory and recompiled only when the source changes.
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdirSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const source = join(here, 'wk-host.swift');
const cacheDir = join(tmpdir(), 'tessel-perf-host');

const compile = () => {
  mkdirSync(cacheDir, { recursive: true });
  const binary = join(cacheDir, 'wk-host');
  if (existsSync(binary) && statSync(binary).mtimeMs > statSync(source).mtimeMs) return binary;
  execFileSync('swiftc', ['-O', source, '-o', binary], { stdio: 'inherit' });
  return binary;
};

/**
 * `fresh: true` starts one host process per run: every run then meets a WebContent and a GPU process that never drew, as an
 * editor launch does (the default shares them between the runs of a plan, so only its first run is cold).
 */
export const runWebKit = async ({ plan, out, fresh = false }) => {
  const binary = compile();
  const groups = fresh ? plan.runs.map((run) => [run]) : [plan.runs];
  for (const [index, runs] of groups.entries()) {
    await new Promise((resolve, reject) => {
      const planFile = join(cacheDir, `plan-${process.pid}-${index}.json`);
      writeFileSync(planFile, JSON.stringify({ ...plan, runs, page: join(here, 'page') }));
      const child = spawn(binary, [planFile, out], { stdio: 'inherit' });
      child.on('exit', (code) => (code === 0 ? resolve() : reject(new Error(`wk-host exited ${code}`))));
    });
  }
};
