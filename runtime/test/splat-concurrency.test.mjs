// Concurrency contract of the splat/asset path in real headless Chrome (SwiftShader): BL091,
// docs/features/spark-concurrency.md. `splat.test.mjs` used to fail in about a third of the runs
// of the default `node --test` (ten files at once) because its "release while loading" step raced
// two `page.evaluate` calls against a load of ~2 ms. The runtime was never wrong: a release
// received before the load finishes answers `cancelled`; one received after leaves a `ready`
// asset that is released. These tests pin both outcomes deterministically and then prove the
// parts that matter under concurrency inside one browser: several pages at once, each with its
// own WebGL context, draw what one page draws alone.
//
// Budget: one Chrome, at most PAGES pages, assets of one splat. Run under the bench lock
// (`with-gpu-lock.py`) when measuring; `scripts/splat-concurrency.mjs` is the experiment bench.
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, test } from 'node:test';

import { AllowList, startAssetServer } from '../worker/assets.mjs';
import { findChrome, glFlags } from '../worker/chrome.mjs';
import { FRONT, encodePly, encodeSplat, holdFetches, openSplatPage, root, skipUnavailable } from './support.mjs';

const PAGES = 4;
const ROUNDS = 5;

const red = [{ position: [0, 0, 0], scale: [0.6, 0.6, 0.6], color: [255, 0, 0], alpha: 255 }];
const SCENE = { format: 'tessel-scene', version: 1, background: [0, 0, 0, 1], objects: [{ type: 'splat', asset: 'cloud' }] };

let shared = null;
let cleanup = [];

/** One browser, one asset server and the files, made on first use and removed after the file. */
const rig = () => {
  shared ??= (async () => {
    const { default: puppeteer } = await import('puppeteer-core');
    const directory = mkdtempSync(join(tmpdir(), 'te-r7-splat-concurrency-'));
    cleanup.push(() => rmSync(directory, { recursive: true, force: true }));
    const allowList = new AllowList();
    const write = (name, content) => {
      const path = join(directory, name);
      writeFileSync(path, content);
      allowList.register({ source: { type: 'file', path } });
      return path;
    };
    const files = {
      ply: write('red.ply', encodePly(red)),
      splat: write('red.splat', encodeSplat(red)),
      scene: write('single.json', JSON.stringify(SCENE)),
    };
    const { server, port } = await startAssetServer({ distDir: join(root, 'dist'), allowList });
    cleanup.push(() => server.close());
    const browser = await puppeteer.launch({ executablePath: findChrome(), headless: true, args: glFlags('swiftshader') });
    cleanup.push(() => browser.close());
    return { browser, port, files };
  })();
  return shared;
};

after(async () => {
  // Reverse order: pages' browser first, then the server and the files it served.
  for (const step of cleanup.reverse()) await Promise.resolve(step()).catch(() => {});
  cleanup = [];
  shared = null;
});

const newPage = async () => {
  const { browser, port } = await rig();
  const page = await openSplatPage(browser, port);
  await page.hello();
  return page;
};

const plyAsset = (path) => ({ id: 'slow', kind: 'splat', source: { type: 'file', path }, meta: { format: 'ply' } });
const settledAfter = (page, from) =>
  page.waitFor((m) => m.type === 'asset.state' && m.id === 'slow' && m.state !== 'loading', 'asset slow', from);
const loadedAssets = (page) => page.page.evaluate(() => window.TesselRuntime.debug.loadedAssets());

test('a release while the load is in flight cancels it and leaves nothing, on several pages at once', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const { files } = await rig();
  const pages = await Promise.all(Array.from({ length: PAGES }, newPage));
  try {
    await Promise.all(
      pages.map(async (page, index) => {
        // The fetch is held: the load is provably awaiting its bytes when the release arrives.
        const gate = await holdFetches(page.page, (url) => decodeURIComponent(url).includes(files.ply));
        try {
          for (let round = 0; round < ROUNDS; round += 1) {
            gate.arm();
            const from = page.messages.length;
            await page.send({ type: 'asset.load', asset: plyAsset(files.ply) });
            await gate.held();
            await page.send({ type: 'asset.release', id: 'slow' });
            await gate.open();
            const answer = await settledAfter(page, from);
            const where = `page ${index}, round ${round}: ${JSON.stringify(answer)}`;
            assert.equal(answer.state, 'failed', where);
            assert.equal(answer.error.code, 'cancelled', where);
            assert.equal((await loadedAssets(page)).includes('slow'), false, `a cancelled load leaves nothing behind (${where})`);
          }
        } finally {
          await gate.close();
        }
      }),
    );
  } finally {
    await Promise.all(pages.map((page) => page.page.close().catch(() => {})));
  }
});

test('a release after the load finished is a plain release: ready, then nothing is loaded', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const { files } = await rig();
  const page = await newPage();
  try {
    // This is the outcome the old step 6 got whenever its load beat its second evaluate call
    // (`'ready' !== 'failed'`): legitimate, and not a bug of the runtime.
    for (let round = 0; round < ROUNDS; round += 1) {
      const loaded = await page.load(plyAsset(files.ply));
      assert.equal(loaded.state, 'ready', JSON.stringify(loaded));
      assert.ok((await loadedAssets(page)).includes('slow'));
      const from = page.messages.length;
      await page.send({ type: 'asset.release', id: 'slow' });
      assert.equal((await loadedAssets(page)).includes('slow'), false, 'the release freed the asset');
      // Nothing answers a release of a loaded asset: no `cancelled`, no second state.
      await new Promise((resolve) => setTimeout(resolve, 100));
      assert.deepEqual(
        page.messages.slice(from).filter((m) => m.type === 'asset.state'),
        [],
        'a release after ready posts no asset state',
      );
    }
  } finally {
    await page.page.close().catch(() => {});
  }
});

test('control: with the held fetch let through first, the same sequence ends ready (the gate decides)', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const { files } = await rig();
  const page = await newPage();
  const gate = await holdFetches(page.page, (url) => decodeURIComponent(url).includes(files.ply));
  try {
    const from = page.messages.length;
    await page.send({ type: 'asset.load', asset: plyAsset(files.ply) });
    await gate.held();
    // Same messages as the cancelling test, but the bytes arrive BEFORE the release is sent.
    await gate.open();
    const answer = await settledAfter(page, from);
    assert.equal(answer.state, 'ready', JSON.stringify(answer));
    await page.send({ type: 'asset.release', id: 'slow' });
    assert.equal((await loadedAssets(page)).includes('slow'), false);
  } finally {
    await gate.close();
    await page.page.close().catch(() => {});
  }
});

test('load and release delivered in one task are cancelled, whatever the machine is doing', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const { files } = await rig();
  const pages = await Promise.all(Array.from({ length: PAGES }, newPage));
  try {
    await Promise.all(
      pages.map(async (page, index) => {
        for (let round = 0; round < ROUNDS * 2; round += 1) {
          const from = page.messages.length;
          // One evaluate: the runtime cannot finish a load between the two messages.
          await page.page.evaluate((asset) => {
            window.TesselRuntime.receive({ v: 1, type: 'asset.load', asset });
            window.TesselRuntime.receive({ v: 1, type: 'asset.release', id: asset.id });
          }, plyAsset(files.ply));
          const answer = await settledAfter(page, from);
          const where = `page ${index}, round ${round}: ${JSON.stringify(answer)}`;
          assert.equal(answer.error?.code, 'cancelled', where);
          assert.equal((await loadedAssets(page)).includes('slow'), false, where);
        }
      }),
    );
  } finally {
    await Promise.all(pages.map((page) => page.page.close().catch(() => {})));
  }
});

test('pages drawing a splat at the same time draw what one page draws alone', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const { files } = await rig();
  const draw = async (page) => {
    const cloud = await page.load({ id: 'cloud', kind: 'splat', source: { type: 'file', path: files.splat }, meta: { format: 'splat' } });
    assert.equal(cloud.state, 'ready', JSON.stringify(cloud));
    const scene = await page.load({ id: 'scene', kind: 'scene', source: { type: 'file', path: files.scene }, meta: {} });
    assert.equal(scene.state, 'ready', JSON.stringify(scene));
    const done = await page.frame(FRONT);
    assert.equal(done.type, 'frame.done', JSON.stringify(done));
    assert.ok(done.probes[0][0] > 200 && done.probes[0][1] < 60, `red at the center: ${done.probes[0]}`);
    return done.hash;
  };
  const alone = await newPage();
  let solo;
  try {
    solo = await draw(alone);
  } finally {
    await alone.page.close().catch(() => {});
  }
  const pages = await Promise.all(Array.from({ length: PAGES }, newPage));
  try {
    const hashes = await Promise.all(pages.map(draw));
    hashes.forEach((hash, index) => assert.equal(hash, solo, `page ${index} drew other pixels than a page alone`));
  } finally {
    await Promise.all(pages.map((page) => page.page.close().catch(() => {})));
  }
});
