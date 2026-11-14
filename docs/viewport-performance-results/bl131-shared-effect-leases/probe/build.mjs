// usage: node build.mjs <runtime-src-dir> <out-dir> [instrument]
// esbuild, node_modules and the host page are the runtime's own, next to <runtime-src-dir> (`runtime/src` -> `runtime/`).
import { cpSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
const [src, out, instrument] = process.argv.slice(2);
const SRC = resolve(src);
const RUNTIME = resolve(SRC, '..');
const { build } = await import(pathToFileURL(join(RUNTIME, 'node_modules/esbuild/lib/main.js')).href);
const nodePaths = [join(RUNTIME, 'node_modules')];
const dist = join(resolve(out), 'dist');
mkdirSync(dist, { recursive: true });
let contents = `import '${SRC}/boot.js';`;
if (instrument) {
  const hooks = join(resolve(out), 'hooks.js');
  writeFileSync(hooks, readFileSync('/tmp/bl131/perf-hooks.js', 'utf8').replaceAll('__SRC__', SRC));
  contents = `import '${hooks}';\n${contents}`;
}
await build({ stdin: { contents, resolveDir: SRC, loader: 'js' }, outfile: join(dist, 'runtime.js'), bundle: true, format: 'iife', target: ['chrome120', 'safari17'], nodePaths, logLevel: 'warning' });
cpSync(join(RUNTIME, 'host/index.html'), join(dist, 'index.html'));
console.log('built', out);
