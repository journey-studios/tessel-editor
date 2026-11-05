// Two optimizations of the next-performance plan, proven in a real page (headless Chrome,
// SwiftShader) through the same bridge messages the desktop uses:
//
//   OPT01  preview proxies: a `preview` frame of a frame sequence reads the attached proxy, an
//          original frame never does, the framing is the original's, and a proxy that cannot be
//          read is reported, not hidden;
//   OPT02  the result cache of a static splat scene: a repeated frame is served without sorting,
//          and every kind of change (camera, asset, size, time of an animated scene, context)
//          forces a real draw whose pixels equal the ones drawn with the cache off.
//
// Skipped, loudly, without Chrome or the bundle (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { boxScene, flat, openRuntime, sceneAsset, sceneLayer, sequenceAsset, shuffled, skipUnavailable, videoLayer } from './support.mjs';

const COUNT = 6;
const pad = (index) => String(index).padStart(5, '0');

/**
 * A 4:3 sequence (48x36) on a 16:9 canvas leaves black bars, so the framing is observable: the
 * proxy (24x18, flat green) must fill the same rectangle as the original (flat red).
 */
const makeSequences = () => {
  const directory = mkdtempSync(join(tmpdir(), 'tessel-proxy-'));
  const original = join(directory, 'o%05d.png');
  const proxy = join(directory, 'p%05d.png');
  for (let frame = 0; frame < COUNT; frame += 1) {
    writeFileSync(join(directory, `o${pad(frame)}.png`), flat(48, 36, [200, 20 + frame * 10, 20]));
    writeFileSync(join(directory, `p${pad(frame)}.png`), flat(24, 18, [20, 200, 20 + frame * 10]));
  }
  return { directory, original, proxy };
};

const PROBES = [
  [10, 45], // left bar
  [30, 45], // inside the picture
  [130, 45], // inside the picture
  [150, 45], // right bar
];

const dominantColor = ([r, g, b]) => (r > 150 ? 'red' : g > 150 ? 'green' : b > 150 ? 'blue' : 'black');

const proxyRuntime = async (sequences) => {
  const runtime = await openRuntime({ extraPatterns: [sequences.original, sequences.proxy] });
  await runtime.hello();
  const loaded = await runtime.load(sequenceAsset('seq', sequences.original, COUNT));
  assert.equal(loaded.state, 'ready');
  return runtime;
};
const proxyOf = (sequences, extra = {}) => ({ pattern: sequences.proxy, first: 0, count: COUNT, ...extra });

test('a preview frame reads the proxy, an original frame never does, and the framing is the same', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const sequences = makeSequences();
  const runtime = await proxyRuntime(sequences);
  try {
    const before = await runtime.frame([videoLayer('seq', 2)], { probes: PROBES });
    assert.equal(before.source.quality, 'original');
    assert.deepEqual(
      before.probes.map(dominantColor),
      ['black', 'red', 'red', 'black'],
      'the 4:3 original leaves bars on a 16:9 canvas',
    );

    assert.equal((await runtime.attachProxy('seq', proxyOf(sequences))).state, 'attached');

    const preview = await runtime.frame([videoLayer('seq', 2)], { quality: 'preview', probes: PROBES });
    assert.equal(preview.type, 'frame.done', JSON.stringify(preview.error));
    assert.deepEqual(
      preview.source,
      { quality: 'preview', proxy_layers: 1, original_layers: 0, result_hits: 0 },
      'the layer was drawn from the proxy',
    );
    assert.deepEqual(
      preview.probes.map(dominantColor),
      ['black', 'green', 'green', 'black'],
      'proxy pixels, the original rectangle: same framing and aspect',
    );
    assert.notEqual(preview.hash, before.hash);

    // The request with no `quality` (explicit render, export, PNG) and `quality: original` ignore the proxy.
    for (const quality of [undefined, 'original']) {
      const final = await runtime.frame([videoLayer('seq', 2)], { quality });
      assert.equal(final.hash, before.hash, `quality ${quality}: the original's pixels, byte for byte`);
      assert.equal(final.source.proxy_layers, 0);
    }

    // Back to preview then to the final frame of the same position: no proxy picture stays behind.
    const again = await runtime.frame([videoLayer('seq', 2)], { quality: 'preview' });
    assert.equal(again.hash, preview.hash, 'the same proxy frame draws the same pixels');
    const settled = await runtime.frame([videoLayer('seq', 2)]);
    assert.equal(settled.hash, before.hash, 'the refine frame is the original again');
  } finally {
    await runtime.close();
  }
});

test('an original frame that was never loaded still draws while a proxy is attached (export, explicit frame)', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const sequences = makeSequences();
  const runtime = await proxyRuntime(sequences);
  try {
    assert.equal((await runtime.attachProxy('seq', proxyOf(sequences))).state, 'attached');
    // Regression: the fetch of an original frame compared the attached proxy with "no proxy" and
    // answered `cancelled` ("released while loading") for every frame that was not cached yet, which
    // failed an export of a video the editor had a proxy for at its second frame.
    for (const index of [3, 4, 5, 1]) {
      const frame = await runtime.frame([videoLayer('seq', index)], { probes: PROBES });
      assert.equal(frame.type, 'frame.done', JSON.stringify(frame.error));
      assert.deepEqual(frame.probes.map(dominantColor), ['black', 'red', 'red', 'black']);
      assert.equal(frame.source.proxy_layers, 0);
    }
  } finally {
    await runtime.close();
  }
});

test('the proxy follows the frame index: out-of-order seeks show the frame asked for, never a neighbour', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const sequences = makeSequences();
  const runtime = await proxyRuntime(sequences);
  try {
    await runtime.attachProxy('seq', proxyOf(sequences));
    const reference = [];
    for (let frame = 0; frame < COUNT; frame += 1) reference.push((await runtime.frame([videoLayer('seq', frame)], { quality: 'preview' })).hash);
    assert.equal(new Set(reference).size, COUNT, 'every proxy frame is a different picture');
    for (const frame of shuffled([...Array(COUNT).keys(), ...Array(COUNT).keys()], 7)) {
      const done = await runtime.frame([videoLayer('seq', frame)], { quality: 'preview' });
      assert.equal(done.hash, reference[frame], `preview frame ${frame}`);
    }
    // Past the end holds the last frame, in the proxy as in the original.
    const past = await runtime.frame([videoLayer('seq', COUNT + 4)], { quality: 'preview' });
    assert.equal(past.hash, reference[COUNT - 1]);
  } finally {
    await runtime.close();
  }
});

test('a proxy that cannot be read is dropped and reported, and the original is drawn', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const sequences = makeSequences();
  const runtime = await proxyRuntime(sequences);
  try {
    const original = await runtime.frame([videoLayer('seq', 1)]);
    const ghost = join(sequences.directory, 'missing%05d.png');
    // The runtime cannot tell a missing file at attach time (it reads lazily): the first preview finds out.
    assert.equal((await runtime.attachProxy('seq', { pattern: ghost, first: 0, count: COUNT })).state, 'attached');
    const failed = await runtime.frame([videoLayer('seq', 1)], { quality: 'preview' });
    assert.equal(failed.type, 'frame.done', 'the picture is still shown');
    assert.equal(failed.hash, original.hash, 'and it is the original');
    assert.equal(failed.source.proxy_layers, 0);
    assert.equal(failed.source.original_layers, 1);
    assert.match(failed.source.proxy_error.code, /asset_unreadable/);
    assert.equal(failed.cache.proxy_failed, 1, 'the failure is counted, not silent');
    const next = await runtime.frame([videoLayer('seq', 1)], { quality: 'preview' });
    assert.equal(next.source.proxy_error, undefined, 'a dropped proxy is not retried every frame');
    assert.equal(next.source.original_layers, 1);
  } finally {
    await runtime.close();
  }
});

test('a proxy with another frame count is refused, and an id that was reloaded loses its proxy', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const sequences = makeSequences();
  const runtime = await proxyRuntime(sequences);
  try {
    const refused = await runtime.attachProxy('seq', proxyOf(sequences, { count: COUNT - 1 }));
    assert.equal(refused.state, 'failed');
    assert.equal(refused.error.code, 'invalid_proxy');
    assert.equal((await runtime.attachProxy('missing', proxyOf(sequences))).state, 'failed');

    await runtime.attachProxy('seq', proxyOf(sequences));
    // The same id loaded again is a new entry: what was attached to the old one is gone with it.
    assert.equal((await runtime.load(sequenceAsset('seq', sequences.original, COUNT))).state, 'ready');
    const preview = await runtime.frame([videoLayer('seq', 3)], { quality: 'preview' });
    assert.equal(preview.source.proxy_layers, 0, 'no stale proxy for a reloaded asset');
    assert.equal(preview.source.original_layers, 1);

    await runtime.attachProxy('seq', proxyOf(sequences));
    assert.equal((await runtime.attachProxy('seq', null)).state, 'detached');
    const detached = await runtime.frame([videoLayer('seq', 3)], { quality: 'preview' });
    assert.equal(detached.source.proxy_layers, 0);
    // Still images and scenes have no frames to proxy.
    const still = await runtime.attachProxy('seq', proxyOf(sequences));
    assert.equal(still.state, 'attached');
  } finally {
    await runtime.close();
  }
});

test('proxy frames stay inside their bound and are released with the asset', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const sequences = makeSequences();
  const runtime = await proxyRuntime(sequences);
  try {
    await runtime.attachProxy('seq', proxyOf(sequences));
    for (let frame = 0; frame < COUNT; frame += 1) await runtime.frame([videoLayer('seq', frame)], { quality: 'preview', inspect: false });
    const held = await runtime.page.evaluate(() => {
      const entry = window.TesselRuntime.debug.assets.entries.get('seq');
      return [...entry.frames.keys()].filter((key) => typeof key === 'string').length;
    });
    assert.equal(held, COUNT);
    await runtime.release('seq');
    const after = await runtime.retention();
    assert.equal(after.texture_entries, 0, 'releasing the asset frees the proxy textures too');
  } finally {
    await runtime.close();
  }
});

// ---- OPT02: result cache of a static splat scene ----

const writeCloud = (directory, name, tint) => {
  const bytes = Buffer.alloc(300 * 32);
  for (let index = 0; index < 300; index += 1) {
    const offset = index * 32;
    [Math.sin(index) * 0.8, Math.cos(index * 1.7) * 0.8, Math.sin(index * 0.3) * 0.8, 0.07, 0.07, 0.07].forEach((value, slot) =>
      bytes.writeFloatLE(value, offset + slot * 4),
    );
    bytes.set([tint[0] + (index % 40), tint[1] + (index % 30), tint[2] + (index % 20), 230, 255, 128, 128, 128], offset + 24);
  }
  const path = join(directory, `${name}.splat`);
  writeFileSync(path, bytes);
  return path;
};

const sceneFile = (directory, name, { spin } = {}) => {
  const path = join(directory, `${name}.json`);
  writeFileSync(
    path,
    JSON.stringify({
      format: 'tessel-scene',
      version: 1,
      background: [0, 0, 0.05, 1],
      objects: [{ type: 'splat', asset: 'splat', position: [0, 0, 0], scale: 1, ...(spin ? { spin } : {}) }],
    }),
  );
  return path;
};

const splatKit = () => {
  const directory = mkdtempSync(join(tmpdir(), 'tessel-result-cache-'));
  const files = {
    redCloud: writeCloud(directory, 'red', [200, 20, 20]),
    blueCloud: writeCloud(directory, 'blue', [20, 20, 200]),
    still: sceneFile(directory, 'still'),
    spinning: sceneFile(directory, 'spinning', { spin: [0, 60, 0] }),
  };
  const splat = (path) => ({ id: 'splat', kind: 'splat', source: { type: 'file', path }, meta: { format: 'splat' } });
  return { directory, files, splat };
};

const at = (asset, x, time = 0) => ({
  ...sceneLayer(asset, time),
  camera: { position: [x, 0.3, 4], target: [0, 0, 0], fov: 50 },
});

const openSplatRuntime = async (kit, { resultCache = true } = {}) => {
  const runtime = await openRuntime({ extraFiles: Object.values(kit.files) });
  await runtime.hello({ result_cache: resultCache });
  assert.equal((await runtime.load(kit.splat(kit.files.redCloud))).state, 'ready');
  assert.equal((await runtime.load(sceneAsset('still', kit.files.still))).state, 'ready');
  assert.equal((await runtime.load(sceneAsset('spinning', kit.files.spinning))).state, 'ready');
  return runtime;
};

const draw = async (runtime, layers, options) => {
  const done = await runtime.frame(layers, options);
  assert.equal(done.type, 'frame.done', JSON.stringify(done.error));
  return done;
};

test('a static splat scene is served from its result: same pixels, no second sort', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const kit = splatKit();
  const cached = await openSplatRuntime(kit);
  try {
    const first = await draw(cached, [at('still', 0)]);
    assert.equal(first.cache.result_miss, 1);
    assert.equal(first.cache.result_hit, 0);
    const second = await draw(cached, [at('still', 0)]);
    assert.equal(second.cache.result_hit, 1, 'nothing changed: the picture is reused');
    assert.equal(second.source.result_hits, 1);
    assert.equal(second.hash, first.hash);
    // Another playhead time of a scene that does not move is the same picture.
    const later = await draw(cached, [at('still', 0, 9.5)]);
    assert.equal(later.cache.result_hit, 1);
    assert.equal(later.hash, first.hash);
    // An interactive frame (no readback) is served the same way.
    const interactive = await draw(cached, [at('still', 0)], { inspect: false });
    assert.equal(interactive.cache.result_hit, 1);
    // The report says how much it keeps.
    assert.equal((await cached.retention()).result_entries, 1);
  } finally {
    await cached.close();
  }
  const plain = await openSplatRuntime(kit, { resultCache: false });
  try {
    const reference = await draw(plain, [at('still', 0)]);
    const again = await draw(plain, [at('still', 0)]);
    assert.equal(again.cache.result_hit, 0, 'off means off');
    assert.equal(again.hash, reference.hash);
    assert.equal(reference.hash, (await (async () => {
      const runtime = await openSplatRuntime(kit);
      try {
        return await draw(runtime, [at('still', 0)]);
      } finally {
        await runtime.close();
      }
    })()).hash, 'the cache does not change the picture');
  } finally {
    await plain.close();
  }
});

test('every change of what the picture depends on is a real draw, equal to the uncached one', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const kit = splatKit();
  const cases = [
    { name: 'a', layers: [at('still', 0)] },
    { name: 'b', layers: [at('still', 2)] },
    { name: 'c', layers: [at('still', -2)] },
    { name: 'spin0', layers: [at('spinning', 0, 0)] },
    { name: 'spin1', layers: [at('spinning', 0, 1)] },
    { name: 'wide', layers: [at('still', 0)], size: [200, 100] },
  ];
  const run = async (resultCache, order) => {
    const runtime = await openSplatRuntime(kit, { resultCache });
    try {
      const out = new Map();
      for (const index of order) {
        const spec = cases[index];
        const done = await draw(runtime, spec.layers, { size: spec.size });
        out.set(spec.name, { hash: done.hash, hit: done.cache.result_hit });
        if (!out.has(`${spec.name}:list`)) out.set(`${spec.name}:list`, []);
        out.get(`${spec.name}:list`).push(done.hash);
      }
      return out;
    } finally {
      await runtime.close();
    }
  };
  const order = shuffled([0, 1, 2, 3, 4, 5, 0, 1, 2, 3, 4, 5, 0, 4, 3, 1], 11);
  const withCache = await run(true, order);
  const without = await run(false, order);
  for (const spec of cases) {
    assert.deepEqual(withCache.get(`${spec.name}:list`), without.get(`${spec.name}:list`), `${spec.name}: cache on and off agree in a shuffled order`);
  }
  assert.notEqual(withCache.get('a').hash, withCache.get('b').hash, 'the camera is part of the key');
  assert.notEqual(withCache.get('spin0').hash, withCache.get('spin1').hash, 'an animated scene is a new picture per time');
});

test('a hit needs the very same camera, time of an animated scene, size and asset', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const kit = splatKit();
  const runtime = await openSplatRuntime(kit);
  try {
    const hits = async (layers, options) => (await draw(runtime, layers, options)).cache.result_hit;
    assert.equal(await hits([at('still', 0)]), 0);
    assert.equal(await hits([at('still', 0)]), 1);
    assert.equal(await hits([at('still', 0.001)]), 0, 'a camera that moved a hair is another picture');
    assert.equal(await hits([at('still', 0.001)]), 1);
    assert.equal(await hits([at('still', 0.001)], { size: [128, 72] }), 0, 'the output size is part of the key');
    assert.equal(await hits([at('spinning', 0, 1)]), 0);
    assert.equal(await hits([at('spinning', 0, 1)]), 1, 'same time of an animated scene');
    assert.equal(await hits([at('spinning', 0, 1.01)]), 0, 'another time of an animated scene');
  } finally {
    await runtime.close();
  }
});

test('the same id with another splat file never serves the old picture', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const kit = splatKit();
  const runtime = await openSplatRuntime(kit);
  try {
    const red = await draw(runtime, [at('still', 0)]);
    assert.equal((await draw(runtime, [at('still', 0)])).cache.result_hit, 1);
    // Same asset ids, other bytes: the splat is reloaded, the scene asset is not.
    assert.equal((await runtime.load(kit.splat(kit.files.blueCloud))).state, 'ready');
    const blue = await draw(runtime, [at('still', 0)]);
    assert.equal(blue.cache.result_hit, 0, 'the asset changed under the same id');
    assert.notEqual(blue.hash, red.hash);
    assert.equal((await draw(runtime, [at('still', 0)])).cache.result_hit, 1, 'and the new picture is cached in turn');
    // Back to the first file: a real draw again, with the first pixels.
    assert.equal((await runtime.load(kit.splat(kit.files.redCloud))).state, 'ready');
    const back = await draw(runtime, [at('still', 0)]);
    assert.equal(back.cache.result_hit, 0);
    assert.equal(back.hash, red.hash);
    // Releasing an asset the picture was built from leaves the next frame failing, not served.
    await runtime.release('splat');
    const gone = await runtime.frame([at('still', 0)]);
    assert.equal(gone.type, 'frame.failed', 'a released asset is an error, never a cached picture');
  } finally {
    await runtime.close();
  }
});

test('two scene layers keep separate results, and the byte bound evicts without breaking pixels', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const kit = splatKit();
  const two = [at('still', 0), { ...at('still', 3), id: 'layer:second', transform: { x: 20, y: 0, scale: 0.5, rotation: 0, crop: [0, 0, 0, 0] } }];
  const reference = await (async () => {
    const runtime = await openSplatRuntime(kit, { resultCache: false });
    try {
      return await draw(runtime, two);
    } finally {
      await runtime.close();
    }
  })();
  const runtime = await openSplatRuntime(kit);
  try {
    const first = await draw(runtime, two);
    assert.equal(first.hash, reference.hash);
    const second = await draw(runtime, two);
    assert.equal(second.cache.result_hit, 2, 'each layer has its own render target and result');
    assert.equal(second.hash, reference.hash);
    // One target only: the layers evict each other every frame, and every frame is still right.
    await runtime.page.evaluate(() => window.TesselRuntime.debug.setLimits({ targets: 1 }));
    for (let round = 0; round < 3; round += 1) {
      const done = await draw(runtime, two);
      assert.equal(done.hash, reference.hash, `round ${round} under a one-target bound`);
    }
    // A frame never loses a target it draws, so both survive the two-layer frames; the bound is
    // enforced on the first frame that needs fewer.
    await draw(runtime, [at('still', 0)]);
    const kept = await runtime.retention();
    assert.ok(kept.target_entries <= 1, `target_entries ${kept.target_entries}`);
    assert.ok(kept.result_entries <= kept.target_entries, 'a result lives only as long as its target');
  } finally {
    await runtime.close();
  }
});

test('a lost context drops the results; the restored one draws and caches again', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const kit = splatKit();
  const runtime = await openSplatRuntime(kit);
  try {
    const before = await draw(runtime, [at('still', 0)]);
    assert.equal((await draw(runtime, [at('still', 0)])).cache.result_hit, 1);
    await runtime.page.evaluate(() => window.TesselRuntime.debug.loseContext());
    await runtime.waitFor((m) => m.type === 'context.lost', 'context.lost');
    assert.equal((await runtime.retention()).result_entries, 0, 'nothing of the lost context is kept');
    const lost = await runtime.frame([at('still', 0)]);
    assert.equal(lost.type, 'frame.failed');
    assert.equal(lost.error.code, 'context_lost');
    await runtime.page.evaluate(() => window.TesselRuntime.debug.restoreContext());
    await runtime.waitFor((m) => m.type === 'context.restored', 'context.restored');
    const restored = await draw(runtime, [at('still', 0)]);
    assert.equal(restored.cache.result_hit, 0, 'the restored context has no picture to reuse');
    assert.equal(restored.hash, before.hash);
    assert.equal((await draw(runtime, [at('still', 0)])).cache.result_hit, 1);
  } finally {
    await runtime.close();
  }
});

test('a frame cancelled before its draw leaves nothing a later frame could mistake for its picture', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const kit = splatKit();
  const runtime = await openSplatRuntime(kit);
  try {
    const reference = await draw(runtime, [at('still', 2)]);
    // A newer request overtakes the older one the moment the runtime sees both: the older is cancelled.
    const older = runtime.frame([at('still', -2)], { id: 900 });
    const newer = runtime.frame([at('still', 2)], { id: 901 });
    const [olderDone, newerDone] = await Promise.all([older, newer]);
    assert.equal(newerDone.hash, reference.hash);
    const afterCancel = await draw(runtime, [at('still', -2)]);
    const clean = await (async () => {
      const fresh = await openSplatRuntime(kit, { resultCache: false });
      try {
        return await draw(fresh, [at('still', -2)]);
      } finally {
        await fresh.close();
      }
    })();
    assert.equal(afterCancel.hash, clean.hash, `the cancelled frame (${olderDone.type}) did not leave a result behind`);
  } finally {
    await runtime.close();
  }
});

test('a plain scene without splats is never keyed: the cache is only for Spark', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const kit = splatKit();
  const boxes = join(kit.directory, 'boxes.json');
  writeFileSync(boxes, JSON.stringify(boxScene()));
  const runtime = await openRuntime({ extraFiles: [boxes] });
  try {
    await runtime.hello();
    assert.equal((await runtime.load(sceneAsset('boxes', boxes))).state, 'ready');
    for (let round = 0; round < 3; round += 1) {
      const done = await draw(runtime, [sceneLayer('boxes', 0)]);
      assert.equal(done.cache.result_hit + done.cache.result_miss, 0, 'no sort to skip, so no result is kept');
    }
    assert.equal((await runtime.retention()).result_entries, 0);
  } finally {
    await runtime.close();
  }
});
