// Finds a Chrome/Chromium to drive and the flags that pick the GL backend.
import { statSync } from 'node:fs';
import { posix, win32 } from 'node:path';

const MAC = [
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  '/Applications/Chromium.app/Contents/MacOS/Chromium',
  '/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary',
];
const NAMES = ['google-chrome', 'google-chrome-stable', 'chromium', 'chromium-browser', 'chrome'];

// Windows: `chrome.exe` is what the installers write and what `PATHEXT` finds for `chrome`.
const WINDOWS_NAMES = ['chrome', 'google-chrome', 'google-chrome-stable', 'chromium', 'chromium-browser'];
// Puppeteer launches without a shell: batch wrappers cannot be started directly by Node.
const DEFAULT_PATHEXT = '.COM;.EXE';
const LAUNCHABLE = ['.com', '.exe'];
// Relative to `ProgramFiles`, `ProgramFiles(x86)` and `LOCALAPPDATA`: the usual per-machine and
// per-user installs. Edge is not listed (it is not Chrome or Chromium); `TESSEL_CHROME` can name it.
const WINDOWS_APPS = [
  ['Google', 'Chrome', 'Application', 'chrome.exe'],
  ['Chromium', 'Application', 'chrome.exe'],
];
const WINDOWS_CANARY = ['Google', 'Chrome SxS', 'Application', 'chrome.exe'];

export class ChromeNotFound extends Error {
  constructor(tried) {
    super(
      `No Chrome or Chromium found (looked in: ${tried.join(', ')}). Install Chrome or set TESSEL_CHROME to its executable.`,
    );
    this.code = 'backend_unavailable';
  }
}

/** A file, not a directory: `existsSync` is true for both. */
const isFile = (path) => {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
};

/** Environment variables are case-insensitive on Windows (`Path`, `PATHEXT`, `LocalAppData`). */
const variable = (env, name, platform) => {
  if (name in env) return env[name];
  if (platform !== 'win32') return undefined;
  const key = Object.keys(env).find((candidate) => candidate.toLowerCase() === name.toLowerCase());
  return key === undefined ? undefined : env[key];
};

const extensions = (env) => {
  const configured = (variable(env, 'PATHEXT', 'win32') ?? '').trim() || DEFAULT_PATHEXT;
  const seen = [];
  for (const extension of configured.split(';').map((part) => part.trim().toLowerCase())) {
    if (LAUNCHABLE.includes(extension) && !seen.includes(extension)) seen.push(extension);
  }
  if (!seen.includes('.exe')) seen.push('.exe');
  return seen;
};

/** The Windows places a Chrome or Chromium installer writes to. No scan of the disk. */
export const windowsInstallCandidates = (env = process.env) => {
  const roots = ['ProgramFiles', 'ProgramFiles(x86)', 'LOCALAPPDATA']
    .map((name) => variable(env, name, 'win32'))
    .filter(Boolean);
  const local = variable(env, 'LOCALAPPDATA', 'win32');
  return [
    ...WINDOWS_APPS.flatMap((app) => roots.map((root) => win32.join(root, ...app))),
    ...(local ? [win32.join(local, ...WINDOWS_CANARY)] : []),
  ];
};

/** Executable candidates in the order they are tried, each with where it comes from. */
const candidatesFor = (env, platform) => {
  const found = [];
  const override = variable(env, 'TESSEL_CHROME', platform);
  if (override) found.push({ path: override, origin: 'override TESSEL_CHROME' });
  const directories = (variable(env, 'PATH', platform) ?? '').split(platform === 'win32' ? win32.delimiter : posix.delimiter);
  if (platform === 'win32') {
    const suffixes = extensions(env);
    for (const raw of directories) {
      const directory = raw.trim().replace(/^"(.*)"$/, '$1');
      if (!directory) continue;
      for (const name of WINDOWS_NAMES) {
        for (const suffix of suffixes) found.push({ path: win32.join(directory, name + suffix), origin: 'PATH' });
      }
    }
    for (const path of windowsInstallCandidates(env)) found.push({ path, origin: 'install location' });
    return found;
  }
  for (const path of MAC) found.push({ path, origin: 'install location' });
  for (const directory of directories) {
    for (const name of NAMES) found.push({ path: posix.join(directory, name), origin: 'PATH' });
  }
  return found;
};

/**
 * `{ path, origin }` of the first candidate that is a file. `TESSEL_CHROME` first (when it is not a
 * file it is skipped, as it always was), then on Windows the `PATH` and the usual install places,
 * elsewhere the macOS apps and the `PATH`. `platform` and `fileExists` are injectable for tests.
 */
export const locateChrome = (env = process.env, { platform = process.platform, fileExists = isFile } = {}) => {
  const candidates = candidatesFor(env, platform);
  for (const candidate of candidates) {
    if (fileExists(candidate.path)) return candidate;
  }
  const override = candidates.filter((candidate) => candidate.origin === 'override TESSEL_CHROME');
  const shown = platform === 'win32'
    ? [...override, ...candidates.filter((candidate) => candidate.origin === 'install location'), 'PATH (chrome.exe)']
    : candidates.slice(0, 6);
  throw new ChromeNotFound(shown.map((entry) => entry.path ?? entry));
};

export const findChrome = (env = process.env, options) => locateChrome(env, options).path;

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
