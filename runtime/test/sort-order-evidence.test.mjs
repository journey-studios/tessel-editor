// BL130: the evidence script refuses inputs that cannot measure anything, with a usage error (status 2) and before it
// renders a frame, starts the worker or writes its output directory. No browser needed: every case here stops at
// the argument check.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const script = join(dirname(fileURLToPath(import.meta.url)), '../scripts/sort-order-evidence.mjs');
const planOf = (frame) => ({ frame, layers: [] });

const run = (t, { plans, assets = [], extra = [], omit = [] }) => {
  const directory = mkdtempSync(join(tmpdir(), 'tessel-evidence-args-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const files = { plans: join(directory, 'plans.json'), assets: join(directory, 'assets.json'), out: join(directory, 'out') };
  writeFileSync(files.plans, typeof plans === 'string' ? plans : JSON.stringify(plans));
  writeFileSync(files.assets, typeof assets === 'string' ? assets : JSON.stringify(assets));
  const args = ['plans', 'assets', 'out'].filter((name) => !omit.includes(name)).flatMap((name) => [`--${name}`, files[name]]);
  // A worker that would start is told to look for a runtime that is not there: reaching it would be a different failure.
  const result = spawnSync(process.execPath, [script, ...args, ...extra], { encoding: 'utf8', timeout: 20_000, env: { ...process.env, TESSEL_RUNTIME_DIST: join(directory, 'none') } });
  return { ...result, out: files.out };
};

const refused = (result, pattern) => {
  assert.equal(result.status, 2, `a usage error, not a crash or a run: status ${result.status}\n${result.stderr}`);
  assert.match(result.stderr, pattern);
  assert.match(result.stderr, /usage: node runtime\/scripts\/sort-order-evidence\.mjs/);
  assert.equal(existsSync(result.out), false, 'nothing was rendered or written');
  assert.equal(result.stdout, '', 'no report');
};

test('fewer than two plans measure nothing: refused before any render, with the usage', (t) => {
  refused(run(t, { plans: [planOf(1)] }), /at least 2 render plans.*got 1/);
  refused(run(t, { plans: [] }), /at least 2 render plans.*got 0/);
});

test('plans that are not an array of render plans are refused', (t) => {
  refused(run(t, { plans: { frame: 1 } }), /array of at least 2 render plans.*got object/);
  refused(run(t, { plans: [planOf(1), null] }), /--plans\[1\] is not a render plan/);
  refused(run(t, { plans: [planOf(1), { frame: 2 }] }), /--plans\[1\] is not a render plan/);
  refused(run(t, { plans: [planOf(1), { frame: '2', layers: [] }] }), /--plans\[1\] is not a render plan/);
  refused(run(t, { plans: '{not json' }), /--plans .* is not readable JSON/);
});

test('assets, size, paces, samples and sheet that cannot be used are refused', (t) => {
  const plans = [planOf(1), planOf(2)];
  refused(run(t, { plans, assets: { id: 'a' } }), /--assets must be an array/);
  refused(run(t, { plans, assets: [{ kind: 'splat' }] }), /--assets\[0\] is not an asset\.load descriptor/);
  refused(run(t, { plans, extra: ['--size', '960'] }), /--size must be WIDTHxHEIGHT/);
  refused(run(t, { plans, extra: ['--pace-ms', 'fast'] }), /--pace-ms must be numbers/);
  refused(run(t, { plans, extra: ['--preview-samples', '3'] }), /--preview-samples must be 0, 2 or 4/);
  refused(run(t, { plans, extra: ['--sheet', '0'] }), /--sheet must be a count/);
  refused(run(t, { plans, extra: ['--sheet=-1'] }), /--sheet must be a count/);
  refused(run(t, { plans, extra: ['--nope', '1'] }), /nope/);
});

test('a missing required option is a usage error as well', (t) => {
  for (const name of ['plans', 'assets', 'out']) {
    const result = run(t, { plans: [planOf(1), planOf(2)], omit: [name] });
    assert.equal(result.status, 2);
    assert.match(result.stderr, new RegExp(`--${name} is required`));
  }
});

test('an unavailable runtime worker fails the evidence run instead of leaving waiters pending', (t) => {
  const result = run(t, { plans: [planOf(1), planOf(2)] });
  assert.equal(result.error, undefined, 'must finish before the process timeout');
  assert.equal(result.status, 1, result.stderr);
  assert.match(result.stderr, /evidence worker (error|closed|exited)/);
});

test('empty or duplicate paces and empty sample values are refused before artifacts exist', (t) => {
  const plans = [planOf(1), planOf(2)];
  for (const value of ['', '41.67,', '41.67,,16', ' ,16']) {
    refused(run(t, { plans, extra: ['--pace-ms', value] }), /--pace-ms must not contain empty entries/);
  }
  for (const value of ['16,16', '16,16.0', '0,0']) {
    refused(run(t, { plans, extra: ['--pace-ms', value] }), /--pace-ms must not repeat a pace/);
  }
  for (const value of ['', ' ', '04']) {
    refused(run(t, { plans, extra: ['--preview-samples', value] }), /--preview-samples must be 0, 2 or 4/);
  }
});
