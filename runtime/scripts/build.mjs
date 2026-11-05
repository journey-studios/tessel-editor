// Bundles the runtime into dist/: one script (Three.js included, no CDN at runtime) plus the
// host page. The Node worker and the desktop WebView both serve these files.
import { cpSync, mkdirSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { build } from 'esbuild';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const dist = join(root, 'dist');
mkdirSync(dist, { recursive: true });

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
});
cpSync(join(root, 'host/index.html'), join(dist, 'index.html'));
console.log(`runtime.js ${(statSync(join(dist, 'runtime.js')).size / 1024).toFixed(0)} KiB`);
