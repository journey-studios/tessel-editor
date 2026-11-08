// MSAA of scene targets: a scene made only of splats may use the host's `splat_samples` (alpha
// blended splats have no edge for multisampling to smooth), while a scene with a box keeps
// `scene_samples`. Real headless Chrome (SwiftShader); skipped, loudly, without Chrome or the
// bundle (TESSEL_REQUIRE_RUNTIME=1 fails).
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { findChrome } from '../worker/chrome.mjs';
import { FRONT, encodeSplat, openSplatPage, root, unavailable } from './support.mjs';

const WIDTH = 160;
const HEIGHT = 90;
/** `targetBytes`: samples x (color + depth) plus the resolve texture, four bytes a pixel. */
const bytes = (samples) => WIDTH * HEIGHT * 4 * (samples * 2 + 1);

test('splat-only scenes use splat_samples, scenes with geometry keep scene_samples', async (t) => {
  const reason = unavailable();
  if (reason) {
    if (process.env.TESSEL_REQUIRE_RUNTIME === '1') assert.fail(reason);
    t.skip(reason);
    return;
  }
  const { default: puppeteer } = await import('puppeteer-core');
  const { AllowList, startAssetServer } = await import('../worker/assets.mjs');
  const directory = mkdtempSync(join(tmpdir(), 'tessel-samples-'));
  const allowList = new AllowList();
  const write = (name, content) => {
    const path = join(directory, name);
    writeFileSync(path, content);
    allowList.register({ source: { type: 'file', path } });
    return path;
  };
  const cloud = write('cloud.splat', encodeSplat([{ position: [0, 0, 0], scale: [0.6, 0.6, 0.6], color: [255, 0, 0], alpha: 255 }]));
  const scene = (objects) => JSON.stringify({ format: 'tessel-scene', version: 1, background: [0, 0, 0, 1], objects });
  const files = {
    splats: write('splats.json', scene([{ type: 'splat', asset: 'cloud' }])),
    boxed: write('boxed.json', scene([{ type: 'splat', asset: 'cloud' }, { type: 'box', size: [3, 3, 0.2], position: [0, 0, 2], color: '#0000ff' }])),
  };
  const { server, port } = await startAssetServer({ distDir: join(root, 'dist'), allowList });
  const browser = await puppeteer.launch({
    executablePath: findChrome(),
    headless: true,
    args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
  });
  try {
    const targetBytes = async (hello, name) => {
      const rig = await openSplatPage(browser, port);
      try {
        await rig.hello(hello);
        assert.equal((await rig.load({ id: 'cloud', kind: 'splat', source: { type: 'file', path: cloud }, meta: { format: 'splat' } })).state, 'ready');
        assert.equal((await rig.load({ id: 'scene', kind: 'scene', source: { type: 'file', path: files[name] }, meta: {} })).state, 'ready');
        const done = await rig.frame(FRONT);
        assert.equal(done.type, 'frame.done', JSON.stringify(done.error));
        return { bytes: done.cache.target_bytes, red: done.probes[0][0] };
      } finally {
        await rig.page.close();
      }
    };

    // The host asks for 4 on scenes with geometry and 0 on splat-only scenes.
    const asked = { scene_samples: 4, splat_samples: 0 };
    const splatOnly = await targetBytes(asked, 'splats');
    const boxed = await targetBytes(asked, 'boxed');
    assert.equal(splatOnly.bytes, bytes(0), 'a splat-only scene renders without multisampling');
    assert.equal(boxed.bytes, bytes(4), 'a scene with a box keeps the scene samples');
    assert.ok(splatOnly.red > 200, `the splat still draws: ${splatOnly.red}`);

    // Without the option nothing changes: export and headless renders keep one count for every scene.
    assert.equal((await targetBytes({ scene_samples: 4 }, 'splats')).bytes, bytes(4));
    assert.equal((await targetBytes({}, 'splats')).bytes, bytes(4), 'the runtime default is 4');
    assert.equal((await targetBytes({ splat_samples: 2 }, 'splats')).bytes, bytes(2));
  } finally {
    await browser.close();
    server.close();
    rmSync(directory, { recursive: true, force: true });
  }
});
