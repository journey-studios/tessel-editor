// BL131: two scene layers of a crossfade that read the same effect splat. Real headless Chrome
// (SwiftShader) through the runtime entry the WebView uses; skipped, loudly, without Chrome or
// the bundle (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
//
// What is proven:
//  - after the load, no frame decodes a splat file again, however many scenes or objects read it
//    (the report measured ~7 ms per crossfade frame in the second scene's decode);
//  - the Spark meshes lent for a frame go back to the asset and serve the next frame, so a crossfade
//    does not build (and dispose) generators per frame either;
//  - every use keeps its own transform: two scenes (and several objects of one scene) read one asset
//    at other positions, scales and spins, and a mesh lent again carries nothing of its last use:
//    a warm page and a cold one draw the same bits for every frame, in any visiting order;
//  - releasing one use never touches the data of another, a released asset takes its meshes with
//    it, and a lost context is rebuilt once (one decode) instead of once per lease.
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { findChrome } from '../worker/chrome.mjs';
import { SPLAT_ENCODERS, openSplatPage, plan, root, sceneLayer, skipUnavailable } from './support.mjs';

const blob = (position, color, scale = 0.5) => ({ position, scale: [scale, scale, scale], color, alpha: 255 });
const FRONT = { position: [0, 0, 5], target: [0, 0, 0], fov: 50 };
const asset = (id, kind, path, format) => ({ id, kind, source: { type: 'file', path }, meta: format ? { format } : {} });
const luminance = ([r, g, b]) => r + g + b;

// One effect splat (a yellow blob with an orange one off its centre, so a spin changes the picture) read by three scenes at other places, scales and spins.
const SCENES = {
  a: [
    { type: 'splat', asset: 'bg-a', position: [0, 0, 0] },
    { type: 'splat', asset: 'fx', position: [-2.4, 0.6, 0], scale: 1, spin: [0, 30, 0] },
    { type: 'splat', asset: 'fx', position: [-2.4, -0.8, 0], scale: 0.5 },
  ],
  b: [
    { type: 'splat', asset: 'bg-b', position: [0, 0, 0] },
    { type: 'splat', asset: 'fx', position: [2.4, 0.6, 0], scale: 1.6, spin: [0, -40, 0] },
  ],
  c: [{ type: 'splat', asset: 'fx', position: [0, 0, 0], scale: 1 }],
};
// Screen position (160x90, camera FRONT) of each use, and of a place nothing draws on.
const PROBES = { fxA1: [34, 33], fxA2: [34, 60], fxB: [126, 33], empty: [126, 70] };

const crossfade = (time, opacity = 0.5) =>
  plan([sceneLayer('scene-a', time), { ...sceneLayer('scene-b', time), opacity }].map((layer) => ({ ...layer, camera: FRONT })));
const single = (id, time = 0) => plan([{ ...sceneLayer(id, time), camera: FRONT }]);

const open = async (t) => {
  const { AllowList, startAssetServer } = await import('../worker/assets.mjs');
  const directory = mkdtempSync(join(tmpdir(), 'tessel-shared-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const allowList = new AllowList();
  const write = (name, content) => {
    const path = join(directory, name);
    writeFileSync(path, content);
    allowList.register({ source: { type: 'file', path } });
    return path;
  };
  const files = {
    fx: write('fx.ply', SPLAT_ENCODERS.ply([blob([0, 0, 0], [255, 220, 0], 0.3), blob([0.9, 0, 0], [255, 90, 0], 0.2)])),
    'bg-a': write('bg-a.splat', SPLAT_ENCODERS.splat([blob([0, 0, 0], [0, 0, 255], 0.5)])),
    'bg-b': write('bg-b.splat', SPLAT_ENCODERS.splat([blob([0, 0, 0], [0, 255, 0], 0.5)])),
  };
  const scenes = Object.fromEntries(
    Object.entries(SCENES).map(([name, objects]) => [
      name,
      write(`scene-${name}.json`, JSON.stringify({ format: 'tessel-scene', version: 1, background: [0, 0, 0, 1], objects })),
    ]),
  );
  const { server, port } = await startAssetServer({ distDir: join(root, 'dist'), allowList });
  t.after(() => server.close());
  const { default: puppeteer } = await import('puppeteer-core');
  const browser = await puppeteer.launch({
    executablePath: findChrome(),
    headless: true,
    args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
  });
  t.after(() => browser.close());
  return { port, browser, files, scenes };
};

/** A page with every asset loaded and a frame helper over bridge v1. */
const openRig = async ({ port, browser, files, scenes }, hello = {}) => {
  const rig = await openSplatPage(browser, port);
  // No result cache: every frame is drawn, so a hash compares what the meshes drew, not a kept picture.
  await rig.send({ type: 'hello', result_cache: false, ...hello });
  await rig.waitFor((m) => m.type === 'ready');
  const formats = { fx: 'ply', 'bg-a': 'splat', 'bg-b': 'splat' };
  for (const id of Object.keys(files)) {
    const loaded = await rig.load(asset(id, 'splat', files[id], formats[id]));
    assert.equal(loaded.state, 'ready', JSON.stringify(loaded));
  }
  for (const name of Object.keys(scenes)) {
    const loaded = await rig.load(asset(`scene-${name}`, 'scene', scenes[name]));
    assert.equal(loaded.state, 'ready', JSON.stringify(loaded));
  }
  let id = 1000;
  const probes = Object.values(PROBES);
  const frame = async (framePlan) => {
    id += 1;
    await rig.send({ type: 'frame.render', id, revision: id, plan: framePlan, output: { width: 160, height: 90 }, probes });
    const done = await rig.waitFor((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === id, `frame ${id}`);
    assert.equal(done.type, 'frame.done', JSON.stringify(done));
    return done;
  };
  return { ...rig, frame };
};

/** Records every lease of the asset store: which mesh, over which packed splats; counts decodes. */
const watch = (rig) =>
  rig.page.evaluate(() => {
    const store = window.TesselRuntime.debug.assets;
    const lend = store.splatLease.bind(store);
    const log = { leases: [], decodes: 0, meshes: new Set(), packed: new Set(), seenBefore: [] };
    window.__lease = log;
    store.splatLease = async (assetId) => {
      const lease = await lend(assetId);
      const fresh = !log.meshes.has(lease.mesh);
      log.meshes.add(lease.mesh);
      log.packed.add(lease.mesh.packedSplats);
      log.leases.push({ assetId, fresh });
      return lease;
    };
    // A decode is a `reinitialize` of PackedSplats from bytes: the prototype is reachable from any mesh.
    const proto = Object.getPrototypeOf(store.entries.get('fx').mesh.packedSplats);
    const reinitialize = proto.reinitialize;
    proto.reinitialize = function (options) {
      if (options?.fileBytes || options?.url) log.decodes += 1;
      return reinitialize.call(this, options);
    };
  });

/** What the leases since the last call did; resets the window. */
const leased = (rig) =>
  rig.page.evaluate(() => {
    const log = window.__lease;
    const out = {
      leases: log.leases.length,
      fx: log.leases.filter((lease) => lease.assetId === 'fx').length,
      newMeshes: log.leases.filter((lease) => lease.fresh).length,
      decodes: log.decodes,
    };
    log.leases = [];
    log.decodes = 0;
    return out;
  });

const hashes = async (rig, frames) => {
  const out = [];
  for (const [name, framePlan] of frames) out.push([name, (await rig.frame(framePlan)).hash]);
  return Object.fromEntries(out);
};

const SEQUENCE = [
  ['x0', crossfade(0)],
  ['x1', crossfade(1)],
  ['x2.5', crossfade(2.5, 0.3)],
  ['a1', single('scene-a', 1)],
  ['b1', single('scene-b', 1)],
  ['c', single('scene-c')],
  ['x1b', crossfade(1)],
];

test('BL131: a crossfade reading one effect splat in two scenes decodes nothing and builds no mesh after the first frame', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const rig = await openRig(await open(t));
  await watch(rig);

  const first = await rig.frame(crossfade(0));
  const initial = await leased(rig);
  // Three uses of `fx` (two in scene A, one in B) and the three backgrounds' two: five leases per frame.
  assert.deepEqual([initial.leases, initial.fx], [5, 3], 'a crossfade frame leases every splat object of both scenes');
  assert.equal(initial.decodes, 0, 'the first frame found the decoded assets and decoded nothing');

  for (const time of [0.25, 0.5, 1, 1.5, 2, 3]) {
    await rig.frame(crossfade(time));
    const stats = await leased(rig);
    assert.deepEqual([stats.leases, stats.fx], [5, 3], `time ${time}: same leases`);
    assert.equal(stats.decodes, 0, `time ${time}: no splat file was decoded again`);
    assert.equal(stats.newMeshes, 0, `time ${time}: no Spark mesh was built; the ones lent before served it`);
  }
  // The same frame again is the same picture (the repeat is a new frame, not a result-cache hit: scenes spin).
  assert.equal((await rig.frame(crossfade(0))).hash, first.hash);
  const meshes = await rig.page.evaluate(() => {
    const log = window.__lease;
    return { meshes: log.meshes.size, packed: log.packed.size };
  });
  // One packed-splat set per asset (3), however many meshes read it; meshes: 3 for fx + 1 + 1.
  assert.deepEqual(meshes, { meshes: 5, packed: 3 }, 'one decode per file, one mesh per concurrent use');
});

test('BL131: each use keeps its own transform; a warm page draws what a cold page draws, in any order', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const env = await open(t);
  const warm = await openRig(env);
  await watch(warm);

  const done = await warm.frame(crossfade(0, 0.5));
  const [a1, a2, b, empty] = done.probes;
  assert.ok(luminance(a1) > 150, `scene A, first use (scale 1, spinning): ${a1}`);
  assert.ok(luminance(a2) > 100, `scene A, second use (scale 0.5): ${a2}`);
  assert.ok(luminance(b) > 60, `scene B at half opacity, its own position: ${b}`);
  assert.ok(luminance(empty) < 20, `nothing is drawn where no use is: ${empty}`);
  // B's use is not where A's are: the same asset, other places.
  const onlyA = (await warm.frame(single('scene-a'))).probes;
  const onlyB = (await warm.frame(single('scene-b'))).probes;
  assert.ok(luminance(onlyA[0]) > 150 && luminance(onlyA[2]) < 20, `A alone: left only: ${onlyA[0]} ${onlyA[2]}`);
  assert.ok(luminance(onlyB[2]) > 120 && luminance(onlyB[0]) < 20, `B alone: right only: ${onlyB[0]} ${onlyB[2]}`);

  const forward = await hashes(warm, SEQUENCE);
  const backward = await hashes(warm, [...SEQUENCE].reverse());
  assert.deepEqual(backward, forward, 'the order the frames are visited in changes no bit');
  assert.equal(forward.x1, forward.x1b, 'a frame drawn again after other uses of the same meshes is the same picture');
  assert.equal(new Set(Object.values(forward)).size, SEQUENCE.length - 1, `every other frame is its own picture ${JSON.stringify(forward)}`);

  const cold = await openRig(env);
  assert.deepEqual(await hashes(cold, [...SEQUENCE].reverse()), forward, 'a cold page, visiting the other way round, draws the same bits');
  assert.deepEqual(await hashes(cold, SEQUENCE), forward);
});

test('BL131: releasing one use leaves the others intact, and a released asset takes its meshes with it', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const rig = await openRig(await open(t));
  await rig.frame(crossfade(1));

  const result = await rig.page.evaluate(async () => {
    const store = window.TesselRuntime.debug.assets;
    const owner = await store.splatLease('fx');
    const shared = await store.splatLease('fx');
    const other = await store.splatLease('fx');
    const sameData = owner.mesh.packedSplats === shared.mesh.packedSplats && shared.mesh.packedSplats === other.mesh.packedSplats;
    const array = owner.mesh.packedSplats.packedArray;
    // Each borrower leaves its own pose on the mesh (a world matrix set by hand, a spin, a scale).
    [owner, shared, other].forEach(({ mesh }, index) => {
      mesh.position.set(5 + index, 1, 2);
      mesh.rotation.set(0.3, 0.4, 0.5);
      mesh.scale.setScalar(3);
      mesh.matrixAutoUpdate = index === 0;
      mesh.matrix.makeScale(2, 2, 2);
    });
    shared.release();
    other.release();
    const afterShared = {
      sameData,
      ownerHasData: owner.mesh.packedSplats.packedArray === array && array !== null,
      numSplats: owner.mesh.packedSplats.numSplats,
    };
    owner.release();
    const again = await store.splatLease('fx');
    const reused = [owner.mesh, shared.mesh, other.mesh].includes(again.mesh);
    const pose = {
      position: again.mesh.position.toArray(),
      scale: again.mesh.scale.toArray(),
      quaternion: again.mesh.quaternion.toArray(),
      auto: again.mesh.matrixAutoUpdate,
      matrix: again.mesh.matrix.elements.every((value, index) => value === (index % 5 === 0 ? 1 : 0)),
      parent: again.mesh.parent,
    };
    again.release();
    // A lease still out when the asset is released: its data goes with the asset, and handing it back is harmless.
    const orphan = await store.splatLease('fx');
    const orphanShared = await store.splatLease('fx');
    store.release('fx');
    orphanShared.release();
    orphan.release();
    return {
      afterShared,
      reused,
      pose,
      gone: !store.has('fx'),
      meshesDead: [owner, shared, other, orphan, orphanShared].map((lease) => !lease.mesh.packedSplats || lease.mesh.packedSplats.packedArray === null),
    };
  });
  assert.equal(result.afterShared.sameData, true);
  assert.equal(result.afterShared.ownerHasData, true, 'disposing a shared mesh did not dispose the data the others read');
  assert.equal(result.afterShared.numSplats, 2);
  assert.equal(result.reused, true, 'a mesh handed back serves the next lease');
  assert.deepEqual(result.pose, { position: [0, 0, 0], scale: [1, 1, 1], quaternion: [0, 0, 0, 1], auto: true, matrix: true, parent: null }, 'it comes back with none of the last use\'s pose');
  assert.equal(result.gone, true);
  assert.deepEqual(result.meshesDead, [true, true, true, true, true], 'no mesh keeps the data of a released asset');

  // The scenes that need the released asset fail honestly; the rest of the page is fine.
  const failed = await (async () => {
    const id = 5000;
    await rig.send({ type: 'frame.render', id, revision: id, plan: crossfade(1), output: { width: 160, height: 90 }, probes: [] });
    return rig.waitFor((m) => m.id === id && (m.type === 'frame.done' || m.type === 'frame.failed'), 'frame without fx');
  })();
  assert.equal(failed.type, 'frame.failed');
  const files = await rig.page.evaluate(() => window.TesselRuntime.debug.loadedAssets());
  assert.equal(files.includes('fx'), false);
});

test('BL131: a lost context is rebuilt from the kept bytes with one decode per asset, and the frame is the same', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const env = await open(t);
  const rig = await openRig(env);
  const baseline = await hashes(rig, SEQUENCE);
  await watch(rig);
  await rig.frame(crossfade(1));
  const warm = await leased(rig);
  assert.equal(warm.decodes, 0);

  await rig.page.evaluate(() => window.TesselRuntime.debug.loseContext());
  await rig.waitFor((m) => m.type === 'context.lost');
  const lostAt = rig.messages.length;
  await rig.page.evaluate(() => window.TesselRuntime.debug.restoreContext());
  await rig.waitFor((m) => m.type === 'context.restored', 'restore', lostAt);

  const first = await rig.frame(crossfade(1));
  const rebuilt = await leased(rig);
  assert.equal(rebuilt.decodes, 3, 'one decode per asset (fx, bg-a, bg-b) rebuilds the lost GPU data; leases beyond the first share it');
  assert.equal(rebuilt.fx, 3);
  assert.equal(rebuilt.newMeshes, 5, 'the meshes of the lost context were not served again');
  assert.equal(first.hash, baseline.x1, 'the frame after the restore is the frame before the loss');

  await rig.frame(crossfade(2));
  const settled = await leased(rig);
  assert.deepEqual([settled.decodes, settled.newMeshes], [0, 0], 'the next frame is served by what the restore built');
  assert.deepEqual(await hashes(rig, SEQUENCE), baseline, 'every frame of the sequence is the same after the restore');
});

test('BL131: with retention off the meshes are freed with the frame (the rollback path) and the pictures are the same', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const env = await open(t);
  const retained = await hashes(await openRig(env), SEQUENCE);
  const rig = await openRig(env, { retain: false });
  await watch(rig);
  // The first frame meets every mesh for the first time whatever the mode; count from the second.
  await rig.frame(crossfade(0));
  await leased(rig);
  const first = await rig.frame(crossfade(1));
  const frame = await leased(rig);
  assert.deepEqual([frame.leases, frame.decodes], [5, 0], 'still no decode');
  assert.equal(frame.newMeshes, 2, 'the two extra uses of `fx` are built for the frame (the asset keeps only the mesh that decoded)');
  await rig.frame(crossfade(2));
  assert.equal((await leased(rig)).newMeshes, 2, 'and built again for the next frame');
  assert.equal(first.hash, retained.x1, 'the same picture as with retention');
  assert.deepEqual(await hashes(rig, SEQUENCE), retained);

  // Switching retention on again lets the asset keep what the next frames return.
  await rig.send({ type: 'hello', result_cache: false, retain: true });
  await rig.waitFor((m) => m.type === 'ready' && rig.messages.filter((x) => x.type === 'ready').length > 1);
  await leased(rig);
  await rig.frame(crossfade(1));
  await leased(rig);
  await rig.frame(crossfade(2));
  assert.equal((await leased(rig)).newMeshes, 0, 'meshes returned after the switch serve the next frame');
});
