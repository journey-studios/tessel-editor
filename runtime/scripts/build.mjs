// Bundles the runtime into dist/: one script (Three.js included, no CDN at runtime) plus the
// host page. The Node worker and the desktop WebView both serve these files.
import { cpSync, mkdirSync, readFileSync, realpathSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { build } from 'esbuild';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const dist = join(root, 'dist');
mkdirSync(dist, { recursive: true });

/**
 * The call Spark's decoder worker makes after the last chunk of a file (BL157.4). WebKit can deadlock a worker inside
 * `ReadableStreamDefaultReader.releaseLock()`: the call allocates, the garbage collector runs in that very thread, and the
 * collector waits for a lock of the same reader that `releaseLock` still holds (a sample of the stuck thread shows
 * releaseLock -> allocation -> collection -> visitOutputConstraints -> lock -> parked for good). The decode never answers, so
 * the asset stays `loading` forever, and a collection the main thread then asks for can freeze the whole page. In the lab's
 * fresh WKWebView it hit 0.8% to 6.7% of the loads of the real project (the machine's load decides), only ever one of the
 * small effect clouds (79 KB to 2.9 MB). After the last chunk the stream is closed and the reader goes with it; there is
 * nothing to release.
 */
export const SPARK_RELEASE_LOCK = 'reader.releaseLock();';
/** The patch, as a function of the source so a test can run it on a string. Throws when Spark no longer has exactly one such call. */
export const withoutReleaseLock = (source) => {
  const count = source.split(SPARK_RELEASE_LOCK).length - 1;
  if (count !== 1) {
    throw new Error(`spark: expected one \`${SPARK_RELEASE_LOCK}\` in the decoder worker, found ${count}; check the release-lock patch of scripts/build.mjs against the new version`);
  }
  return source.replace(SPARK_RELEASE_LOCK, 'void 0;');
};

const sparkReleaseLockPatch = {
  name: 'spark-release-lock',
  setup(esbuild) {
    // The ES module Spark's `exports.import` points at: the file that carries the worker's source as a string.
    esbuild.onLoad({ filter: /@sparkjsdev[\\/]spark[\\/]dist[\\/]spark\.module\.js$/ }, (args) => ({
      contents: withoutReleaseLock(readFileSync(args.path, 'utf8')),
      loader: 'js',
    }));
  },
};

// Run as a script by `pnpm build`; importing this file (the test of the patch) builds nothing.
if (process.argv[1] && fileURLToPath(import.meta.url) === realpathSync(process.argv[1])) {
  await build({
    entryPoints: [join(root, 'src/boot.js')],
    outfile: join(dist, 'runtime.js'),
    bundle: true,
    format: 'iife',
    target: ['chrome120', 'safari17'],
    minify: process.env.TESSEL_RUNTIME_MINIFY === '1',
    sourcemap: false,
    legalComments: 'none',
    logLevel: 'info',
    plugins: [sparkReleaseLockPatch],
  });
  cpSync(join(root, 'host/index.html'), join(dist, 'index.html'));
  console.log(`runtime.js ${(statSync(join(dist, 'runtime.js')).size / 1024).toFixed(0)} KiB`);
}
