// usage: node build.mjs <runtime-src-dir> <out-dir> [instrument]
import { build } from '/Users/wallyssonnunes/.t3/worktrees/tessel-editor/t3-b251b22c/runtime/node_modules/esbuild/lib/main.js';
import { cpSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
const [src, out, instrument] = process.argv.slice(2);
const SRC = resolve(src);
const nodePaths = ['/Users/wallyssonnunes/.t3/worktrees/tessel-editor/t3-b251b22c/runtime/node_modules'];
const dist = join(resolve(out), 'dist');
mkdirSync(dist, { recursive: true });
let contents = `import '${SRC}/boot.js';`;
if (instrument) {
  const hooks = join(resolve(out), 'hooks.js');
  writeFileSync(hooks, readFileSync('/tmp/bl131/perf-hooks.js', 'utf8').replaceAll('__SRC__', SRC));
  contents = `import '${hooks}';\n${contents}`;
}
await build({ stdin: { contents, resolveDir: SRC, loader: 'js' }, outfile: join(dist, 'runtime.js'), bundle: true, format: 'iife', target: ['chrome120', 'safari17'], nodePaths, logLevel: 'warning' });
cpSync('/Users/wallyssonnunes/.t3/worktrees/tessel-editor/t3-b251b22c/runtime/host/index.html', join(dist, 'index.html'));
console.log('built', out);
