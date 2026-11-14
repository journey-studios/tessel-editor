// Where the runtime tests find `ffmpeg`/`ffprobe`: the same order as the Rust adapter
// (`crates/media/src/tools/locate.rs`) so a test never needs a refreshed `PATH` that the app does
// not need either. `TESSEL_FFMPEG`/`TESSEL_FFPROBE` first (strict: when set, it is the answer),
// then `PATH` (with `PATHEXT` on Windows), then the install places: Homebrew on macOS, WinGet on
// Windows (`Links` and `Packages\Gyan.FFmpeg*\ffmpeg-*\bin`, two levels, no disk scan).
// `platform`, `fileExists` and `listDir` are injectable, so the rules are testable on any machine.
import { readdirSync, statSync } from 'node:fs';
import { posix, win32 } from 'node:path';

// spawnSync without a shell can launch native executables, not batch wrappers.
const LAUNCHABLE = ['.com', '.exe'];
const DEFAULT_PATHEXT = '.COM;.EXE';

const isFile = (path) => {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
};

const listDir = (path) => {
  try {
    return readdirSync(path);
  } catch {
    return [];
  }
};

/** Environment variables are case-insensitive on Windows. */
const variable = (env, name, platform) => {
  if (name in env) return env[name];
  if (platform !== 'win32') return undefined;
  const key = Object.keys(env).find((candidate) => candidate.toLowerCase() === name.toLowerCase());
  return key === undefined ? undefined : env[key];
};

const suffixes = (env) => {
  const configured = (variable(env, 'PATHEXT', 'win32') ?? '').trim() || DEFAULT_PATHEXT;
  const seen = [];
  for (const extension of configured.split(';').map((part) => part.trim().toLowerCase())) {
    if (LAUNCHABLE.includes(extension) && !seen.includes(extension)) seen.push(extension);
  }
  if (!seen.includes('.exe')) seen.push('.exe');
  return seen;
};

/** `9.0.2-full_build` -> `[9, 0, 2]`. */
const versionKey = (name) =>
  (name.match(/^[\d.]*/)?.[0] ?? '')
    .split('.')
    .filter(Boolean)
    .map(Number);

const compareVersions = (a, b) => {
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    const difference = (b[index] ?? 0) - (a[index] ?? 0);
    if (difference !== 0) return difference;
  }
  return 0;
};

/** `Packages/Gyan.FFmpeg*\/ffmpeg-*\/bin`, newest build first. */
const gyanBinDirectories = (packages, list) => {
  const builds = [];
  for (const package_ of list(packages)) {
    if (!package_.toLowerCase().startsWith('gyan.ffmpeg')) continue;
    const root = win32.join(packages, package_);
    for (const build of list(root)) {
      if (build.toLowerCase().startsWith('ffmpeg-')) {
        builds.push({ key: versionKey(build.slice('ffmpeg-'.length)), directory: win32.join(root, build, 'bin') });
      }
    }
  }
  return builds.sort((a, b) => compareVersions(a.key, b.key) || a.directory.localeCompare(b.directory)).map((build) => build.directory);
};

const windowsInstallDirectories = (env, list) => {
  const local = variable(env, 'LOCALAPPDATA', 'win32');
  const programFiles = variable(env, 'ProgramFiles', 'win32');
  const directories = [];
  if (local) directories.push(win32.join(local, 'Microsoft', 'WinGet', 'Links'));
  if (programFiles) directories.push(win32.join(programFiles, 'WinGet', 'Links'));
  for (const packages of [
    local && win32.join(local, 'Microsoft', 'WinGet', 'Packages'),
    programFiles && win32.join(programFiles, 'WinGet', 'Packages'),
  ].filter(Boolean)) {
    directories.push(...gyanBinDirectories(packages, list));
  }
  return directories;
};

/**
 * `{ path, origin }` of `ffmpeg` or `ffprobe`, or `null`. A set `TESSEL_FFMPEG`/`TESSEL_FFPROBE`
 * that is not a file is `null`: an override is never silently replaced.
 */
export const locateFfTool = (name, env = process.env, { platform = process.platform, fileExists = isFile, list = listDir } = {}) => {
  const overrideName = name === 'ffprobe' ? 'TESSEL_FFPROBE' : 'TESSEL_FFMPEG';
  const override = variable(env, overrideName, platform);
  if (override !== undefined) return fileExists(override) ? { path: override, origin: `override ${overrideName}` } : null;
  const windows = platform === 'win32';
  const path = windows ? win32 : posix;
  const names = windows ? suffixes(env).map((suffix) => name + suffix) : [name];
  const found = (directory, origin) => {
    for (const file of names) {
      const candidate = path.join(directory, file);
      if (fileExists(candidate)) return { path: candidate, origin };
    }
    return null;
  };
  for (const raw of (variable(env, 'PATH', platform) ?? '').split(path.delimiter)) {
    const directory = windows ? raw.trim().replace(/^"(.*)"$/, '$1') : raw;
    if (windows && !directory) continue;
    const hit = found(directory, 'PATH');
    if (hit) return hit;
  }
  const installs = windows ? windowsInstallDirectories(env, list) : ['/opt/homebrew/bin', '/usr/local/bin'];
  for (const directory of installs) {
    const hit = found(directory, windows ? 'WinGet' : 'install location');
    if (hit) return hit;
  }
  return null;
};

/** An invalid explicit override fails before spawn can look up a bare name on PATH. */
export const ffToolPath = (name, env = process.env, options = {}) => {
  const found = locateFfTool(name, env, options);
  if (found) return found.path;
  const overrideName = name === 'ffprobe' ? 'TESSEL_FFPROBE' : 'TESSEL_FFMPEG';
  if (variable(env, overrideName, options.platform ?? process.platform) !== undefined) {
    const error = new Error(`${overrideName} does not name an executable file`);
    error.code = 'ToolNotFound';
    throw error;
  }
  return name;
};
