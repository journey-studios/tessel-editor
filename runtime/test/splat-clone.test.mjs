// A splat asset drawn by two objects of one scene (or by two layers of a frame) shares its decoded
// splats: the second taker is a clone kept on the entry and reused by the next frame. The pixels
// must be the ones of two meshes that each decoded the file, frame after frame, and releasing
// the asset must leave nothing behind. Real headless Chrome (SwiftShader); skipped, loudly,
// without Chrome or the bundle (TESSEL_REQUIRE_RUNTIME=1 fails).
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { findChrome } from '../worker/chrome.mjs';
import { FRONT, encodeSplat, openSplatPage, root, unavailable } from './support.mjs';

test('splat clones draw the same pixels as separately decoded meshes and are reused', async (t) => {
  const reason = unavailable();
  if (reason) {
    if (process.env.TESSEL_REQUIRE_RUNTIME === '1') assert.fail(reason);
    t.skip(reason);
    return;
  }
  const { default: puppeteer } = await import('puppeteer-core');
  const { AllowList, startAssetServer } = await import('../worker/assets.mjs');
  const directory = mkdtempSync(join(tmpdir(), 'tessel-clone-'));
  const allowList = new AllowList();
  const write = (name, content) => {
    const path = join(directory, name);
    writeFileSync(path, content);
    allowList.register({ source: { type: 'file', path } });
    return path;
  };
  const splats = encodeSplat([
    { position: [0, 0, 0], scale: [0.5, 0.5, 0.5], color: [255, 0, 0], alpha: 255 },
    { position: [0.4, 0.2, -0.5], scale: [0.4, 0.4, 0.4], color: [0, 255, 0], alpha: 200 },
  ]);
  // The same bytes under two ids: each is decoded once and owns its mesh (no clone involved).
  const files = { one: write('cloud.splat', splats), other: write('cloud-copy.splat', splats) };
  const scene = (objects) => JSON.stringify({ format: 'tessel-scene', version: 1, background: [0, 0, 0, 1], objects });
  const placed = (asset, x, spin) => ({ type: 'splat', asset, position: [x, 0, 0], scale: 0.7, ...(spin ? { spin } : {}) });
  const scenes = {
    // one asset, two takers: the second is a clone
    twin: write('twin.json', scene([placed('cloud', -1.6, [0, 40, 0]), placed('cloud', 1.6, [0, -25, 0])])),
    // the same picture from two assets that each decoded the file
    apart: write('apart.json', scene([placed('cloud', -1.6, [0, 40, 0]), placed('copy', 1.6, [0, -25, 0])])),
  };
  const { server, port } = await startAssetServer({ distDir: join(root, 'dist'), allowList });
  const browser = await puppeteer.launch({
    executablePath: findChrome(),
    headless: true,
    args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
  });
  try {
    const rig = await openSplatPage(browser, port);
    await rig.hello();
    const load = async (asset) => assert.equal((await rig.load(asset)).state, 'ready', asset.id);
    await load({ id: 'cloud', kind: 'splat', source: { type: 'file', path: files.one }, meta: { format: 'splat' } });
    await load({ id: 'copy', kind: 'splat', source: { type: 'file', path: files.other }, meta: { format: 'splat' } });
    const times = [0, 0.5, 1, 0, 1, 0.5, 0];

    const draw = async (name) => {
      await load({ id: 'scene', kind: 'scene', source: { type: 'file', path: scenes[name] }, meta: {} });
      const hashes = [];
      for (const time of times) {
        const done = await rig.frame(FRONT, [[26, 45], [134, 45]], time);
        assert.equal(done.type, 'frame.done', JSON.stringify(done.error));
        hashes.push(done.hash);
      }
      return hashes;
    };
    const apart = await draw('apart');
    const twin = await draw('twin');
    assert.deepEqual(twin, apart, 'a clone draws what a mesh of its own decode draws, in every frame');
    assert.equal(new Set(twin).size, 3, 'the spin makes three distinct pictures, so the comparison means something');

    // Releasing the asset frees the clone with it; loading it again works and draws the same.
    await rig.send({ type: 'asset.release', id: 'cloud' });
    await load({ id: 'cloud', kind: 'splat', source: { type: 'file', path: files.one }, meta: { format: 'splat' } });
    assert.deepEqual(await draw('twin'), twin, 'a reloaded asset draws the same pictures');
  } finally {
    await browser.close();
    server.close();
    rmSync(directory, { recursive: true, force: true });
  }
});
