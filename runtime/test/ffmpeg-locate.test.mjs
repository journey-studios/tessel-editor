// Where the runtime tests find ffmpeg/ffprobe (BL167.2.2). Environment, platform and file system
// are injected: nothing here reads or changes `process.env`, so it runs the same on any machine.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { ffToolPath, locateFfTool } from './ffmpeg-locate.mjs';

const LOCAL = 'C:\\Users\\Ana Maria\\AppData\\Local';
const PF = 'C:\\Program Files';
const PACKAGES = `${LOCAL}\\Microsoft\\WinGet\\Packages`;

/** A machine with exactly these files and directory listings. */
const machine = (files, platform = 'win32', directories = {}) => ({
  platform,
  fileExists: (path) => files.includes(path),
  list: (path) => directories[path] ?? [],
});
const find = (name, env, files, platform, directories) => locateFfTool(name, env, machine(files, platform, directories));

test('windows: ffmpeg.exe on a PATH folder with spaces is found, with its origin', () => {
  const exe = 'C:\\Tools\\My FFmpeg\\ffmpeg.exe';
  assert.deepEqual(find('ffmpeg', { Path: 'C:\\Windows;C:\\Tools\\My FFmpeg' }, [exe]), { path: exe, origin: 'PATH' });
  assert.equal(find('ffmpeg', { PATH: 'C:\\Tools' }, ['C:\\Tools\\ffmpeg']), null, 'the bare name is not a Windows executable');
});

test('windows: ffprobe has its own override and name', () => {
  const env = { PATH: 'C:\\a', TESSEL_FFPROBE: 'C:\\custom\\probe.exe' };
  assert.equal(find('ffprobe', env, ['C:\\custom\\probe.exe', 'C:\\a\\ffprobe.exe']).path, 'C:\\custom\\probe.exe');
  assert.equal(find('ffprobe', { PATH: 'C:\\a' }, ['C:\\a\\ffprobe.exe']).path, 'C:\\a\\ffprobe.exe');
});

test('an override that is not a file is not replaced by the PATH', () => {
  const env = { PATH: 'C:\\a', TESSEL_FFMPEG: 'C:\\gone\\ffmpeg.exe' };
  assert.equal(find('ffmpeg', env, ['C:\\a\\ffmpeg.exe']), null);
  assert.throws(() => ffToolPath('ffmpeg', env, machine(['C:\\a\\ffmpeg.exe'])), { code: 'ToolNotFound' });
  assert.throws(() => ffToolPath('ffmpeg', { ...env, TESSEL_FFMPEG: 'ffmpeg' }, machine(['C:\\a\\ffmpeg.exe'])), { code: 'ToolNotFound' });
});

test('windows: WinGet Links and the Gyan package layout are found without PATH, newest build first', () => {
  const links = `${LOCAL}\\Microsoft\\WinGet\\Links\\ffmpeg.exe`;
  assert.deepEqual(find('ffmpeg', { LOCALAPPDATA: LOCAL }, [links]), { path: links, origin: 'WinGet' });

  const gyan = `${PACKAGES}\\Gyan.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe`;
  const directories = {
    [PACKAGES]: ['Other.Package', 'Gyan.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe'],
    [gyan]: ['ffmpeg-8.1-full_build', 'ffmpeg-9.0.2-full_build', 'ffmpeg-9.0-full_build', 'notes.txt'],
  };
  const files = [`${gyan}\\ffmpeg-8.1-full_build\\bin\\ffmpeg.exe`, `${gyan}\\ffmpeg-9.0.2-full_build\\bin\\ffmpeg.exe`];
  assert.equal(find('ffmpeg', { LOCALAPPDATA: LOCAL }, files, 'win32', directories).path, files[1]);
  // The Links shim wins over the package when both exist.
  assert.equal(find('ffmpeg', { LOCALAPPDATA: LOCAL }, [links, ...files], 'win32', directories).path, links);
  // Per-machine packages are looked at too.
  const machineGyan = `${PF}\\WinGet\\Packages\\Gyan.FFmpeg`;
  const machineExe = `${machineGyan}\\ffmpeg-7.0-full_build\\bin\\ffmpeg.exe`;
  assert.equal(
    find('ffmpeg', { ProgramFiles: PF }, [machineExe], 'win32', {
      [`${PF}\\WinGet\\Packages`]: ['Gyan.FFmpeg'],
      [machineGyan]: ['ffmpeg-7.0-full_build'],
    }).path,
    machineExe,
  );
});

test('windows: PATH comes before the install places', () => {
  const links = `${LOCAL}\\Microsoft\\WinGet\\Links\\ffmpeg.exe`;
  assert.equal(find('ffmpeg', { LOCALAPPDATA: LOCAL, PATH: 'C:\\a' }, [links, 'C:\\a\\ffmpeg.exe']).path, 'C:\\a\\ffmpeg.exe');
});

test('posix: PATH first, then Homebrew, and nothing else is guessed', () => {
  assert.deepEqual(find('ffmpeg', { PATH: '/x:/y' }, ['/y/ffmpeg', '/opt/homebrew/bin/ffmpeg'], 'darwin'), { path: '/y/ffmpeg', origin: 'PATH' });
  assert.deepEqual(find('ffmpeg', { PATH: '/x' }, ['/opt/homebrew/bin/ffmpeg', '/usr/local/bin/ffmpeg'], 'darwin'), {
    path: '/opt/homebrew/bin/ffmpeg',
    origin: 'install location',
  });
  assert.equal(find('ffmpeg', { PATH: '/x' }, ['/usr/local/bin/ffmpeg'], 'linux').path, '/usr/local/bin/ffmpeg');
  assert.equal(find('ffmpeg', { PATH: '/x' }, ['/x/ffmpeg.exe'], 'linux'), null, 'no PATHEXT outside Windows');
});

test('windows: automatic lookup skips batch wrappers before native executables', () => {
  const env = { PATH: 'C:\\a', PATHEXT: '.CMD;.BAT;.EXE' };
  const files = ['C:\\a\\ffmpeg.cmd', 'C:\\a\\ffmpeg.bat', 'C:\\a\\ffmpeg.exe'];
  assert.equal(find('ffmpeg', env, files).path, files[2]);
  assert.equal(find('ffmpeg', env, files.slice(0, 2)), null);
});

test('an empty override is explicit and never replaced by a discovered executable', () => {
  const env = { PATH: 'C:\\a', TESSEL_FFMPEG: '' };
  const options = machine(['C:\\a\\ffmpeg.exe']);
  assert.equal(locateFfTool('ffmpeg', env, options), null);
  assert.throws(() => ffToolPath('ffmpeg', env, options), { code: 'ToolNotFound' });
});

test('unix: empty PATH entries use the current directory and spaces stay literal', () => {
  assert.equal(find('ffmpeg', { PATH: ':/x' }, ['ffmpeg', '/x/ffmpeg'], 'darwin').path, 'ffmpeg');
  assert.equal(find('ffmpeg', {}, ['ffmpeg'], 'darwin').path, 'ffmpeg');
  assert.equal(find('ffmpeg', { PATH: '/with space ' }, ['/with space /ffmpeg'], 'linux').path, '/with space /ffmpeg');
});
