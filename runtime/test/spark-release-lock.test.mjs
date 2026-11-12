// BL157.4: the OPT-IN build patch that drops the `reader.releaseLock()` of Spark's decoder worker (a WebKit deadlock, see scripts/build.mjs).
// No browser: the patch is a function of the source, the bundle is built in memory (never written, so `dist/` is not touched) and the
// installed Spark is read as a file. Off by default, on only with `TESSEL_RUNTIME_SPARK_RELEASE_LOCK_PATCH=1`. What the patch buys (a
// load that always ends) is measured by the perf lab's `real-load` case; the deadlock itself needs a loaded WKWebView to show.
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { SPARK_RELEASE_LOCK, SPARK_RELEASE_LOCK_PATCH_ENV, bundleRuntime, runtimePlugins, withoutReleaseLock } from '../scripts/build.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

test('drops the call the worker makes after the last chunk, and nothing around it', () => {
  const source = ['while (true) {', '  const { done, value } = await reader.read();', '  if (done) {', `    ${SPARK_RELEASE_LOCK}`, '    break;', '  }', '}'].join('\n');
  const patched = withoutReleaseLock(source);
  assert.ok(!patched.includes('releaseLock'));
  assert.equal(patched, source.replace(SPARK_RELEASE_LOCK, 'void 0;'));
  assert.ok(patched.includes('await reader.read();'), 'the read loop is untouched');
  assert.ok(patched.includes('break;'), 'the loop still ends where it did');
});

test('leaves the other stream the library has (the main thread one) alone', () => {
  const source = `readStream.releaseLock();\n${SPARK_RELEASE_LOCK}`;
  assert.equal(withoutReleaseLock(source), 'readStream.releaseLock();\nvoid 0;');
});

test('refuses a Spark that has no such call, or more than one, and says how many', () => {
  assert.throws(() => withoutReleaseLock('const nothing = 1;'), /found 0/);
  assert.throws(() => withoutReleaseLock(`${SPARK_RELEASE_LOCK}\n${SPARK_RELEASE_LOCK}`), /found 2/);
});

test('the installed Spark has exactly the one call the patch removes', () => {
  const file = join(root, 'node_modules/@sparkjsdev/spark/dist/spark.module.js');
  assert.ok(existsSync(file), 'Spark is installed (pnpm install)');
  const patched = withoutReleaseLock(readFileSync(file, 'utf8'));
  assert.ok(!patched.includes(SPARK_RELEASE_LOCK));
});

const ON = { [SPARK_RELEASE_LOCK_PATCH_ENV]: '1' };

/** What each plugin of a build for `env` registers as loader hooks (the build runs `setup` once per plugin). */
const loaderHooks = (env) => {
  const hooks = [];
  for (const plugin of runtimePlugins(env)) plugin.setup({ onLoad: (options, callback) => hooks.push({ filter: options.filter, callback }) });
  return hooks;
};
/** The bundle esbuild makes for `env`, as text. */
const bundleText = async (env) => {
  const { outputFiles } = await bundleRuntime(env, { write: false, outfile: 'runtime.js', logLevel: 'silent' });
  return outputFiles[0].text;
};
const calls = (text) => text.split(SPARK_RELEASE_LOCK).length - 1;

test('off by default: the build has no plugin, so no loader hook is registered on Spark', () => {
  assert.deepEqual(runtimePlugins({}), []);
  assert.deepEqual(loaderHooks({}), []);
});

test('only the exact value 1 turns the patch on', () => {
  for (const value of ['0', '', 'true', 'yes', '1 ', 'on']) {
    assert.deepEqual(runtimePlugins({ [SPARK_RELEASE_LOCK_PATCH_ENV]: value }), [], `${JSON.stringify(value)} is not 1`);
  }
  assert.equal(runtimePlugins(ON).length, 1);
});

test('with the variable the patch registers one hook, on the module Spark exports and on nothing else', () => {
  const hooks = loaderHooks(ON);
  assert.equal(hooks.length, 1);
  assert.ok(hooks[0].filter.test('/app/node_modules/@sparkjsdev/spark/dist/spark.module.js'));
  assert.ok(hooks[0].filter.test('C:\\app\\node_modules\\@sparkjsdev\\spark\\dist\\spark.module.js'));
  assert.ok(!hooks[0].filter.test('/app/node_modules/three/build/three.module.js'));
  assert.ok(!hooks[0].filter.test('/app/runtime/src/compositor.js'));
});

test('the default bundle still makes the call, and the one built with the variable does not, and is shorter by what the patch removes', async () => {
  const [plain, patched] = await Promise.all([bundleText({}), bundleText(ON)]);
  assert.equal(calls(plain), 1, 'Spark ships as it is');
  assert.equal(calls(patched), 0, 'the call is gone');
  assert.equal(plain.length - patched.length, SPARK_RELEASE_LOCK.length - 'void 0;'.length, 'and nothing else changed');
});
