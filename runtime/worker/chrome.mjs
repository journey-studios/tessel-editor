// Finds a Chrome/Chromium to drive and the flags that pick the GL backend.
import { existsSync } from 'node:fs';
import { delimiter, join } from 'node:path';

const MAC = [
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  '/Applications/Chromium.app/Contents/MacOS/Chromium',
  '/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary',
];
const NAMES = ['google-chrome', 'google-chrome-stable', 'chromium', 'chromium-browser', 'chrome'];

export class ChromeNotFound extends Error {
  constructor(tried) {
    super(
      `No Chrome or Chromium found (looked in: ${tried.join(', ')}). Install Chrome or set TESSEL_CHROME to its executable.`,
    );
    this.code = 'backend_unavailable';
  }
}

export const findChrome = (env = process.env) => {
  const tried = [];
  const candidates = [env.TESSEL_CHROME, ...MAC];
  for (const directory of (env.PATH ?? '').split(delimiter)) {
    for (const name of NAMES) candidates.push(join(directory, name));
  }
  for (const candidate of candidates.filter(Boolean)) {
    tried.push(candidate);
    if (existsSync(candidate)) return candidate;
  }
  throw new ChromeNotFound(tried.slice(0, 6));
};

/**
 * `swiftshader` (default) is the software rasterizer: slower, but the same on every machine,
 * so frame hashes are reproducible. `gpu` uses the platform GPU path (Metal on macOS) and is
 * what a measurement of real speed should use.
 */
export const glFlags = (mode = 'swiftshader', platform = process.platform) => {
  if (mode === 'gpu') return platform === 'darwin' ? ['--use-angle=metal', '--enable-gpu'] : ['--enable-gpu'];
  if (mode === 'swiftshader') return ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'];
  throw new Error(`TESSEL_GL must be \`swiftshader\` or \`gpu\`, got \`${mode}\``);
};
