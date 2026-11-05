import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, before, test } from 'node:test';

import { AllowList, startAssetServer } from '../worker/assets.mjs';
import { glFlags } from '../worker/chrome.mjs';

let directory;
let server;
let base;

before(async () => {
  directory = mkdtempSync(join(tmpdir(), 'tessel-server-'));
  writeFileSync(join(directory, 'clip.webm'), Buffer.from('0123456789'));
  writeFileSync(join(directory, 'secret.txt'), 'nope');
  writeFileSync(join(directory, 'f00001.png'), 'png');
  const dist = join(directory, 'dist');
  const allowList = new AllowList();
  allowList.register({ source: { type: 'file', path: join(directory, 'clip.webm') } });
  allowList.register({ source: { type: 'frame_sequence', pattern: join(directory, 'f%05d.png') } });
  ({ server } = await startAssetServer({ distDir: dist, allowList }).then((started) => {
    base = `http://127.0.0.1:${started.port}`;
    return started;
  }));
});

after(() => new Promise((done) => server.close(done)));

const asset = (path) => `${base}/asset?p=${encodeURIComponent(path)}`;

test('registered files are served, with byte ranges for video seeking', async () => {
  const file = join(directory, 'clip.webm');
  assert.equal(await (await fetch(asset(file))).text(), '0123456789');
  const partial = await fetch(asset(file), { headers: { Range: 'bytes=2-5' } });
  assert.equal(partial.status, 206);
  assert.equal(partial.headers.get('content-range'), 'bytes 2-5/10');
  assert.equal(await partial.text(), '2345');
  const tail = await fetch(asset(file), { headers: { Range: 'bytes=-3' } });
  assert.equal(await tail.text(), '789');
  assert.equal((await fetch(asset(file), { headers: { Range: 'bytes=50-60' } })).status, 416);
});

test('files nobody registered are refused, including path tricks', async () => {
  assert.equal((await fetch(asset(join(directory, 'secret.txt')))).status, 403);
  assert.equal((await fetch(asset(join(directory, 'sub', '..', 'secret.txt')))).status, 403);
  assert.equal((await fetch(asset('/etc/passwd'))).status, 403);
  assert.equal((await fetch(`${base}/asset`)).status, 403);
});

test('a frame pattern allows its numbered frames and nothing else', async () => {
  assert.equal((await fetch(asset(join(directory, 'f00001.png')))).status, 200);
  assert.equal((await fetch(asset(join(directory, 'f00001.png.bak')))).status, 403);
});

test('GL modes map to Chrome flags and unknown modes are refused', () => {
  assert.ok(glFlags('swiftshader').includes('--use-angle=swiftshader'));
  assert.ok(glFlags('gpu', 'darwin').includes('--use-angle=metal'));
  assert.throws(() => glFlags('vulkan'), /swiftshader/);
});
