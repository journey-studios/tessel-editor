// BL157.4: the build patch that drops the `reader.releaseLock()` of Spark's decoder worker (a WebKit deadlock, see scripts/build.mjs).
// No browser: the patch is a function of the source, and the installed Spark and the built bundle are read as files. What the patch buys
// (a load that always ends) is measured by the perf lab's `real-load` case; the deadlock itself needs a loaded WKWebView to show.
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { SPARK_RELEASE_LOCK, withoutReleaseLock } from '../scripts/build.mjs';

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

test('the built bundle does not make the call', (t) => {
  const bundle = join(root, 'dist/runtime.js');
  if (!existsSync(bundle)) {
    if (process.env.TESSEL_REQUIRE_RUNTIME === '1') assert.fail('dist/runtime.js is not built (pnpm run build)');
    t.skip('dist/runtime.js is not built (pnpm run build)');
    return;
  }
  assert.ok(!readFileSync(bundle, 'utf8').includes(SPARK_RELEASE_LOCK), 'rebuild with `pnpm build`: this bundle predates the patch');
});
