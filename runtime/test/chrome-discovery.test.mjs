// Chrome/Chromium discovery (BL167.2). The environment, the platform and the file system are
// injected, so every rule runs on any machine and nothing reads or changes `process.env`.
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { ChromeNotFound, findChrome, locateChrome, windowsInstallCandidates } from '../worker/chrome.mjs';

const PF = 'C:\\Program Files';
const PF86 = 'C:\\Program Files (x86)';
const LOCAL = 'C:\\Users\\Ana Maria\\AppData\\Local';

/** A machine with exactly these files; directories are simply not in the set. */
const machine = (files, platform = 'win32') => ({ platform, fileExists: (path) => files.includes(path) });
const at = (env, files, platform) => locateChrome(env, machine(files, platform));

test('windows: chrome.exe is found on a PATH folder with spaces, and the origin says PATH', () => {
  const exe = 'C:\\Tools\\My Browser\\chrome.exe';
  const found = at({ Path: 'C:\\Windows;C:\\Tools\\My Browser' }, [exe]);
  assert.deepEqual(found, { path: exe, origin: 'PATH' });
});

test('windows: a quoted PATH entry and empty entries are handled', () => {
  const exe = 'C:\\Tools\\Quoted Dir\\chrome.exe';
  assert.equal(at({ PATH: ';;"C:\\Tools\\Quoted Dir";' }, [exe]).path, exe);
  // An empty entry is not the current directory.
  assert.throws(() => at({ PATH: ';' }, ['chrome.exe']), ChromeNotFound);
});

test('windows: the bare name is never a match', () => {
  assert.throws(() => at({ PATH: 'C:\\Tools' }, ['C:\\Tools\\chrome', 'C:\\Tools\\google-chrome']), ChromeNotFound);
});

test('windows: PATH order and PATHEXT order decide, case-insensitively', () => {
  const [a, b] = ['C:\\a\\chrome.exe', 'C:\\b\\chrome.exe'];
  assert.equal(at({ PATH: 'C:\\a;C:\\b' }, [b, a]).path, a);
  const both = ['C:\\a\\chrome.cmd', 'C:\\a\\chrome.exe'];
  assert.equal(at({ PATH: 'C:\\a' }, both).path, 'C:\\a\\chrome.exe', 'default PATHEXT: .EXE before .CMD');
  assert.equal(at({ PATH: 'C:\\a', pathext: '.CMD;.EXE' }, both).path, 'C:\\a\\chrome.exe', 'Node launches the browser without a shell');
  assert.throws(() => at({ PATH: 'C:\\a', PATHEXT: '.BAT;.CMD' }, ['C:\\a\\chrome.cmd', 'C:\\a\\chrome.bat']), ChromeNotFound);
  assert.equal(at({ PATH: 'C:\\a', PATHEXT: '.EXE;.COM' }, ['C:\\a\\chrome.com', 'C:\\a\\chrome.exe']).path, 'C:\\a\\chrome.exe');
  assert.equal(at({ PATH: 'C:\\a', PATHEXT: '.COM;.EXE' }, ['C:\\a\\chrome.com', 'C:\\a\\chrome.exe']).path, 'C:\\a\\chrome.com');
  assert.equal(at({ PATH: 'C:\\a', PATHEXT: '.VBS;.JS' }, both).path, 'C:\\a\\chrome.exe', 'script hosts are skipped');
});

test('windows: the usual install places are found without PATH, per-machine before per-user', () => {
  const chrome = (root) => `${root}\\Google\\Chrome\\Application\\chrome.exe`;
  assert.deepEqual(at({ ProgramFiles: PF }, [chrome(PF)]), { path: chrome(PF), origin: 'install location' });
  assert.equal(at({ 'ProgramFiles(x86)': PF86 }, [chrome(PF86)]).path, chrome(PF86));
  assert.equal(at({ LocalAppData: LOCAL }, [chrome(LOCAL)]).path, chrome(LOCAL), 'variable names ignore case');
  const env = { ProgramFiles: PF, 'ProgramFiles(x86)': PF86, LOCALAPPDATA: LOCAL };
  assert.equal(at(env, [chrome(LOCAL), chrome(PF86), chrome(PF)]).path, chrome(PF));
  assert.equal(at(env, [chrome(LOCAL), chrome(PF86)]).path, chrome(PF86));
});

test('windows: Chromium and Canary installs are found, Chrome stable first', () => {
  const env = { ProgramFiles: PF, LOCALAPPDATA: LOCAL };
  const chromium = `${LOCAL}\\Chromium\\Application\\chrome.exe`;
  const canary = `${LOCAL}\\Google\\Chrome SxS\\Application\\chrome.exe`;
  assert.equal(at(env, [chromium]).path, chromium);
  assert.equal(at(env, [canary]).path, canary);
  assert.equal(at(env, [canary, chromium]).path, chromium);
  assert.deepEqual(windowsInstallCandidates(env).slice(0, 2), [
    `${PF}\\Google\\Chrome\\Application\\chrome.exe`,
    `${LOCAL}\\Google\\Chrome\\Application\\chrome.exe`,
  ]);
});

test('windows: PATH wins over the install places', () => {
  const onPath = 'C:\\Tools\\chrome.exe';
  const installed = `${PF}\\Google\\Chrome\\Application\\chrome.exe`;
  assert.equal(at({ PATH: 'C:\\Tools', ProgramFiles: PF }, [installed, onPath]).path, onPath);
});

test('windows: a valid TESSEL_CHROME wins; one that is not a file falls back, as it always did', () => {
  const custom = 'D:\\Browsers\\Edge Dev\\msedge.exe';
  const installed = `${PF}\\Google\\Chrome\\Application\\chrome.exe`;
  const env = { TESSEL_CHROME: custom, ProgramFiles: PF };
  assert.deepEqual(at(env, [custom, installed]), { path: custom, origin: 'override TESSEL_CHROME' });
  assert.equal(at(env, [installed]).path, installed, 'missing override');
  // A directory is in no file set: it is rejected the same way, with a real directory below.
  assert.equal(at({ TESSEL_CHROME: 'D:\\Browsers', ProgramFiles: PF }, [installed]).path, installed);
});

test('a TESSEL_CHROME that is a real directory is rejected on the real file system', () => {
  const root = mkdtempSync(join(tmpdir(), 'tessel-chrome-'));
  try {
    const directory = join(root, 'chrome.exe');
    mkdirSync(directory);
    const real = join(root, 'real');
    writeFileSync(real, '');
    assert.equal(findChrome({ TESSEL_CHROME: real }), real, 'a real file is used');
    // The directory is skipped: either another Chrome of this machine answers, or none does.
    let outcome;
    try {
      outcome = locateChrome({ TESSEL_CHROME: directory, PATH: '' }).path;
    } catch (error) {
      assert.ok(error instanceof ChromeNotFound, String(error));
    }
    assert.notEqual(outcome, directory);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('windows: nothing found says where it looked and how to fix it', () => {
  const env = { TESSEL_CHROME: 'D:\\gone\\chrome.exe', ProgramFiles: PF, LOCALAPPDATA: LOCAL };
  assert.throws(
    () => at(env, []),
    (error) => {
      assert.ok(error instanceof ChromeNotFound);
      assert.equal(error.code, 'backend_unavailable');
      assert.match(error.message, /D:\\gone\\chrome\.exe/);
      assert.match(error.message, /Program Files\\Google\\Chrome\\Application\\chrome\.exe/);
      assert.match(error.message, /set TESSEL_CHROME/);
      return true;
    },
  );
  assert.throws(() => at({}, []), ChromeNotFound, 'no variables at all');
});

test('not windows: the macOS apps first, then the PATH names, exactly as before', () => {
  const mac = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
  const onPath = '/usr/bin/google-chrome';
  assert.deepEqual(at({ PATH: '/usr/bin' }, [onPath, mac], 'darwin'), { path: mac, origin: 'install location' });
  assert.deepEqual(at({ PATH: '/usr/bin' }, [onPath], 'linux'), { path: onPath, origin: 'PATH' });
  assert.equal(at({ PATH: '/opt/x:/usr/bin' }, ['/usr/bin/chromium', '/opt/x/chromium-browser'], 'linux').path, '/opt/x/chromium-browser');
  const canary = '/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary';
  assert.equal(at({}, [canary], 'darwin').path, canary);
  const override = '/custom/chrome';
  assert.equal(at({ TESSEL_CHROME: override, PATH: '/usr/bin' }, [override, mac, onPath], 'darwin').path, override);
  assert.equal(at({ TESSEL_CHROME: '/gone', PATH: '/usr/bin' }, [mac], 'darwin').path, mac);
});

test('not windows: no extensions and none of the Windows places', () => {
  const env = { PATH: '/usr/bin', PATHEXT: '.EXE', ProgramFiles: PF, LOCALAPPDATA: LOCAL };
  const files = ['/usr/bin/chrome.exe', `${PF}\\Google\\Chrome\\Application\\chrome.exe`];
  assert.throws(() => at(env, files, 'linux'), ChromeNotFound);
  assert.throws(() => at(env, files, 'darwin'), ChromeNotFound);
  assert.equal(at({ PATH: '/usr/bin' }, ['/usr/bin/chrome'], 'linux').path, '/usr/bin/chrome');
});

test('findChrome still returns just the path', () => {
  const exe = 'C:\\Tools\\chrome.exe';
  assert.equal(findChrome({ PATH: 'C:\\Tools' }, machine([exe])), exe);
});
