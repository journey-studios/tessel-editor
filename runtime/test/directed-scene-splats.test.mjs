// Gaussian splats in the directed-scene layer (S7, BL025 subset): the plan carries the splat nodes
// (world matrix, `spz` resource, `splats` capability) and the runtime only draws them with Spark
// behind the image planes. The first half is pure (no GL); the second draws real pixels in
// headless Chrome (SwiftShader). Skipped, loudly, without Chrome or the bundle
// (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
//
// The pixel oracle is the pinhole projection written out below, independent of the runtime: the
// splat of node N must appear where the plan's own camera and N's world matrix put it.
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  DIRECTED_SCENE_CAPABILITIES,
  SCENE_CAPABILITIES,
  parseDirectedLayer,
  resolveSceneResources,
} from '../src/directed-scene.js';
import { RuntimeError } from '../src/protocol.js';
import {
  GENERATION,
  SIZE,
  directedLayer,
  directedRuntime,
  flat,
  node,
  reference,
  resource,
  sceneAsset,
  scenePlan,
  translation,
  vec,
} from './directed-scene-fixtures.mjs';
import { loseNow, restore } from './lifecycle-fixtures.mjs';
import { SPLAT_ENCODERS, encodePly, skipUnavailable } from './support.mjs';

// ---- helpers ------------------------------------------------------------------------------

const BLOB = (position, color, scale = 0.5) => ({ position, scale: [scale, scale, scale], color, alpha: 255 });
const RED = [255, 0, 0];

const splatNode = (id, assetId, x, y, z) => ({
  id,
  kind: 'splat',
  parent: null,
  assetId,
  widthSceneUnits: null,
  heightSceneUnits: null,
  worldMatrix: translation(x, y, z),
  localMatrix: translation(x, y, z),
  renderOrder: 0,
});

const splatResource = (assetId, extra = {}) => resource(assetId, 1, 1, { format: 'spz', ...extra });
const pictureResource = (assetId, width, height) => resource(assetId, width, height, { format: 'png' });

/** A complete plan with splats behind planes, in the core's wire form. */
// The core's rule, written out here independently: a splat is painted after the planes that are
// not nearer to the camera (looking down -z, a larger z is nearer) up to the first one that is.
const slotOf = (z, planes) => {
  const first = planes.findIndex((plane) => plane[4] > z);
  return first < 0 ? planes.length : first;
};

const splatPlan = ({ splats, planes = [], resources, camera, extra = {} }) =>
  scenePlan({
    nodes: [...splats.map(([id, asset, x, y, z]) => splatNode(id, asset, x, y, z)), ...planes.map(([id, asset, x, y, z, w, h]) => node(id, asset, x, y, z, w, h))],
    layers: planes.map(([id, asset, , , , , , alpha = 'straight']) => [id, asset, alpha]),
    resources,
    camera,
    requires: ['camera', 'nodeTransforms', ...(planes.length ? ['imagePlanes', 'straightAlpha'] : []), 'splats'],
    extra: { splats: splats.map(([id, asset, , , z]) => ({ nodeId: id, assetId: asset, afterLayers: slotOf(z, planes) })), ...extra },
  });

const refusal = (callback) => {
  try {
    callback();
  } catch (error) {
    assert.ok(error instanceof RuntimeError, `a typed RuntimeError, got ${error}`);
    return error;
  }
  assert.fail('expected a refusal');
};

// ---- pure ---------------------------------------------------------------------------------

test('splats are part of what this runtime advertises and of what it reads from a plan', () => {
  assert.ok(SCENE_CAPABILITIES.includes('splats'));
  assert.equal(SCENE_CAPABILITIES.length, 9);
  assert.deepEqual(DIRECTED_SCENE_CAPABILITIES.resource_kinds, ['still', 'splat']);

  const plan = splatPlan({
    splats: [['cloud-node', 'cloud', 1, 0, -2]],
    planes: [['front', 'hero', 0, 0, 0, 2, 2]],
    resources: [splatResource('cloud'), pictureResource('hero', 8, 8)],
  });
  const spec = parseDirectedLayer(directedLayer(plan, []));
  assert.equal(spec.splats.length, 1);
  assert.equal(spec.splats[0].nodeId, 'cloud-node');
  assert.deepEqual(spec.splats[0].world, translation(1, 0, -2), 'the node matrix is the core one, untouched');
  assert.equal(spec.planes.length, 1, 'a splat is not a plane');
  assert.equal(spec.resources.get('cloud').format, 'spz');
  assert.equal(spec.resources.get('hero').format, 'png');

  // A plan without splats (the key is absent on the wire) is what it always was.
  const plain = scenePlan({
    nodes: [node('front', 'hero', 0, 0, 0, 2, 2)],
    layers: [['front', 'hero']],
    resources: [resource('hero', 8, 8)],
  });
  assert.deepEqual(parseDirectedLayer(directedLayer(plain, [])).splats, []);
});

test('a plan that hides, mislabels or under-declares its splats is refused with the pointer', () => {
  const base = () =>
    splatPlan({ splats: [['cloud-node', 'cloud', 0, 0, 0]], resources: [splatResource('cloud')] });
  const invalid = (edit, pointer) => {
    const plan = base();
    edit(plan);
    const error = refusal(() => parseDirectedLayer(directedLayer(plan, [])));
    assert.equal(error.code, 'invalid_plan');
    assert.equal(error.details.pointer, pointer);
  };
  invalid((plan) => { plan.splats = []; }, '/splats'); // a splat node nobody lists would not be drawn
  invalid((plan) => { plan.splats[0].nodeId = 'nobody'; }, '/splats/0/nodeId');
  invalid((plan) => { plan.splats[0].afterLayers = 1; }, '/splats/0/afterLayers'); // there is no layer to come after
  invalid((plan) => { delete plan.splats[0].afterLayers; }, '/splats/0/afterLayers');
  invalid((plan) => { plan.nodes[0].kind = 'imagePlane'; }, '/nodes/0/kind');
  invalid((plan) => { plan.splats[0].assetId = 'other'; }, '/splats/0/assetId');
  invalid((plan) => { plan.resources[0].format = 'png'; }, '/splats/0/assetId'); // a splat reads an spz
  invalid((plan) => { plan.resources[0].format = 'ply'; }, '/resources/0/format');
  invalid((plan) => { plan.requires = ['camera', 'nodeTransforms']; }, '/requires');
  invalid((plan) => { plan.nodes[0].worldMatrix = plan.nodes[0].worldMatrix.slice(1); }, '/nodes/0/worldMatrix');

  // An image plane cannot read an spz either.
  const plane = splatPlan({
    splats: [],
    planes: [['front', 'cloud', 0, 0, 0, 2, 2]],
    resources: [splatResource('cloud')],
  });
  plane.requires = ['camera', 'nodeTransforms', 'imagePlanes', 'straightAlpha'];
  const error = refusal(() => parseDirectedLayer(directedLayer(plane, [])));
  assert.deepEqual([error.code, error.details.pointer], ['invalid_plan', '/layers/0/assetId']);
});

test('light, weather and rigs still make the plan unsupported even when splats are drawable', () => {
  const plan = splatPlan({ splats: [['cloud-node', 'cloud', 0, 0, 0]], resources: [splatResource('cloud')] });
  plan.complete = false;
  plan.unsupported = [{ domain: 'lighting', subject: null }, { domain: 'rigs', subject: null }];
  const error = refusal(() => parseDirectedLayer(directedLayer(plan, [])));
  assert.equal(error.code, 'unsupported');
  assert.deepEqual(error.details.domains.map((entry) => entry.domain), ['lighting', 'rigs']);
});

test('a splat resource is bound only to a splat loaded as spz with exactly the plan identity', () => {
  const plan = splatPlan({ splats: [['cloud-node', 'cloud', 0, 0, 0]], resources: [splatResource('cloud')] });
  const spec = parseDirectedLayer(directedLayer(plan, [reference('cloud', 'slot-0')]));
  const held = { asset_id: 'cloud', revision_id: 'cloud-r1', content_hash: 'hash-cloud', generation: GENERATION };
  const entry = (extra = {}) => ({ kind: 'splat', fileName: 'asset.spz', descriptor: { scene: held }, ...extra });
  const store = (slot) => ({ has: (id) => id === 'slot-0' && slot !== null, get: () => slot });

  const bound = resolveSceneResources(spec, store(entry()));
  assert.equal(bound.get('cloud').slot, 'slot-0');
  assert.equal(refusal(() => resolveSceneResources(spec, store(entry({ kind: 'still', width: 1, height: 1 })))).code, 'resource_mismatch');
  const ply = refusal(() => resolveSceneResources(spec, store(entry({ fileName: 'asset.ply' }))));
  assert.equal(ply.code, 'resource_mismatch', 'a ply under an spz identity is another file');
  assert.deepEqual(ply.details.expected, { kind: 'splat', format: 'spz' });
  const stale = refusal(() => resolveSceneResources(spec, store(entry({ descriptor: { scene: { ...held, generation: GENERATION + 1 } } }))));
  assert.equal(stale.code, 'resource_stale');
  assert.equal(refusal(() => resolveSceneResources(spec, { has: () => false, get: () => null })).code, 'resource_unresolved');
});

test('the budget counts a splat file by its kept bytes and packed splats, a picture by its RGBA', async () => {
  const { AssetStore } = await import('../src/assets.js');
  const store = new AssetStore({ assetUrl: (path) => path });
  assert.equal(store.sceneEntryBytes({ kind: 'splat', bytes: new ArrayBuffer(1000), count: 50 }), 1000 + 50 * 16);
  assert.equal(store.sceneEntryBytes({ kind: 'still', width: 8, height: 4 }), 8 * 4 * 4);
});

// ---- real pixels --------------------------------------------------------------------------

const [WIDTH, HEIGHT] = SIZE;
const FOV = 45;
const DISTANCE = 10;

/** Where a point at (x, y, 0) of a camera looking down -z from `DISTANCE` lands, in pixels (pinhole maths). */
const project = (x, y, z = 0, camera = { x: 0, y: 0 }) => {
  const depth = DISTANCE - z;
  const halfHeight = depth * Math.tan((FOV * Math.PI) / 360);
  const halfWidth = halfHeight * (WIDTH / HEIGHT);
  return [Math.round(WIDTH / 2 + ((x - camera.x) / halfWidth) * (WIDTH / 2)), Math.round(HEIGHT / 2 - ((y - camera.y) / halfHeight) * (HEIGHT / 2))];
};

const isRed = ([r, g, b]) => r > 150 && g < 80 && b < 80;
const isGreen = ([r, g, b]) => g > 150 && r < 80 && b < 80;
const isBlack = ([r, g, b]) => r < 12 && g < 12 && b < 12;
const show = (pixel) => `rgb(${pixel.slice(0, 3).join(',')})`;

const spzFiles = (t, clouds) => {
  const directory = mkdtempSync(join(tmpdir(), 'tessel-directed-splat-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const write = (name, bytes) => {
    const path = join(directory, name);
    writeFileSync(path, bytes);
    return path;
  };
  const paths = Object.fromEntries(Object.entries(clouds).map(([name, blobs]) => [name, write(`${name}.spz`, SPLAT_ENCODERS.spz(blobs))]));
  paths.plyCloud = write('cloud.ply', encodePly(clouds.cloud ?? Object.values(clouds)[0]));
  paths.green = write('green.png', flat(4, 4, [0, 255, 0]));
  paths.stored = write('stored.png', flat(4, 4, [230, 49, 49]));
  return paths;
};

const splatAsset = (slot, path, assetId, extra = {}) => ({ ...sceneAsset(slot, path, assetId, extra), kind: 'splat', meta: { format: 'spz' } });

const loadScene = async (runtime, paths) => {
  const cloud = await runtime.load(splatAsset('slot-cloud', paths.cloud, 'cloud'));
  assert.equal(cloud.state, 'ready', JSON.stringify(cloud.error));
  assert.equal(cloud.meta.splat_count, 1);
  const green = await runtime.load(sceneAsset('slot-green', paths.green, 'green'));
  assert.equal(green.state, 'ready', JSON.stringify(green.error));
  return { cloud: reference('cloud', 'slot-cloud'), green: reference('green', 'slot-green') };
};

const draw = (runtime, plan, refs, probes) => {
  const needed = new Set(plan.resources.map((entry) => entry.assetId));
  const layer = directedLayer(plan, Object.values(refs).filter((entry) => needed.has(entry.asset_id)));
  return runtime.frame([layer], { size: SIZE, probes });
};

const done = (message) => {
  assert.equal(message.type, 'frame.done', JSON.stringify(message.error));
  return message;
};

test('a splat is drawn where its node matrix and the plan camera put it, at its slot among the planes', async (t) => {
  if (skipUnavailable(t, assert)) return;
  // `wide` (sigma 1) reaches past the 2x2 plane that will cover its center; `cloud` (sigma 0.5) does not reach 2 units.
  const paths = spzFiles(t, { cloud: [BLOB([0, 0, 0], RED)], wide: [BLOB([0, 0, 0], RED, 1)] });
  const runtime = await directedRuntime(paths);
  try {
    const ready = await runtime.hello();
    assert.deepEqual(ready.capabilities.directed_scene.resource_kinds, ['still', 'splat']);
    assert.ok(ready.capabilities.directed_scene.capabilities.includes('splats'));
    assert.equal(ready.capabilities.directed_scene.capabilities.length, 9);
    const refs = await loadScene(runtime, paths);
    const wide = await runtime.load(splatAsset('slot-wide', paths.wide, 'wide'));
    assert.equal(wide.state, 'ready', JSON.stringify(wide.error));
    refs.wide = reference('wide', 'slot-wide');

    const at = (x) => project(x, 0);
    const origin = at(0);
    const right = at(2);
    const left = at(-2);
    const far = [WIDTH - 4, 4]; // a corner the splat never reaches
    const probes = [origin, right, left, far];

    // The same file, the same camera; only the node's matrix moves: the splat follows it.
    const centered = done(await draw(runtime, splatPlan({ splats: [['cloud-node', 'cloud', 0, 0, 0]], resources: [splatResource('cloud')] }), refs, probes));
    assert.ok(isRed(centered.probes[0]), `centered: ${show(centered.probes[0])}`);
    assert.ok(isBlack(centered.probes[1]) && isBlack(centered.probes[2]), 'two units to the sides: nothing');
    assert.ok(isBlack(centered.probes[3]), 'the background stays opaque black');
    assert.equal(centered.source.directed_scene_splats, 1);

    const moved = done(await draw(runtime, splatPlan({ splats: [['cloud-node', 'cloud', 2, 0, 0]], resources: [splatResource('cloud')] }), refs, probes));
    assert.ok(isRed(moved.probes[1]), `at +2: ${show(moved.probes[1])}`);
    assert.ok(isBlack(moved.probes[0]) && isBlack(moved.probes[2]), 'it left the center');
    const mirrored = done(await draw(runtime, splatPlan({ splats: [['cloud-node', 'cloud', -2, 0, 0]], resources: [splatResource('cloud')] }), refs, probes));
    assert.ok(isRed(mirrored.probes[2]), `at -2: ${show(mirrored.probes[2])}`);
    assert.ok(isBlack(mirrored.probes[1]), 'and not on the other side');
    assert.notEqual(moved.hash, mirrored.hash);

    // The camera is the plan's: dollying it by +2 puts a splat at the origin where a splat at -2 was.
    const dollied = done(
      await draw(
        runtime,
        splatPlan({
          splats: [['cloud-node', 'cloud', 0, 0, 0]],
          resources: [splatResource('cloud')],
          camera: { position: vec(2, 0, DISTANCE), target: vec(2, 0, 0) },
        }),
        refs,
        probes,
      ),
    );
    const shifted = project(0, 0, 0, { x: 2, y: 0 });
    assert.deepEqual(shifted, left, 'the oracle agrees: camera +2 is node -2');
    assert.equal(dollied.hash, mirrored.hash, 'a camera move and the opposite node move are the same picture');

    // In front of or behind a plane by depth, as the core's slot says (BL019 seam with the S10 corpus,
    // where the background plane is far behind the splats): a plane farther than the splat is under it...
    const behind = done(
      await draw(
        runtime,
        splatPlan({
          splats: [['cloud-node', 'wide', 0, 0, 0]],
          planes: [['wall', 'green', 0, 0, -2, 2, 2]],
          resources: [splatResource('wide'), pictureResource('green', 4, 4)],
        }),
        refs,
        [project(0, 0, -2)],
      ),
    );
    assert.ok(isRed(behind.probes[0]), `the splat is over a plane that is farther: ${show(behind.probes[0])}`);
    assert.equal(behind.source.directed_scene_layers, 1);
    // ...and a plane nearer than the splat covers it where it reaches (the plane is 2 units wide at depth 8,
    // the blob wider): the splat still shows around it.
    const covered = done(
      await draw(
        runtime,
        splatPlan({
          splats: [['cloud-node', 'wide', 0, 0, 0]],
          planes: [['wall', 'green', 0, 0, 2, 2, 2]],
          resources: [splatResource('wide'), pictureResource('green', 4, 4)],
        }),
        refs,
        [project(0, 0, 2), project(1.6, 0)],
      ),
    );
    assert.ok(isGreen(covered.probes[0]), `a nearer plane covers the splat: ${show(covered.probes[0])}`);
    const beside = covered.probes[1];
    assert.ok(beside[0] > 60 && beside[0] > beside[1] + 40, `the splat shows beside the plane: ${show(beside)}`);
    assert.notEqual(behind.hash, covered.hash);

    // A splat between two planes: the far plane under it, the near plane over it.
    const between = done(
      await draw(
        runtime,
        splatPlan({
          splats: [['cloud-node', 'wide', 0, 0, 0]],
          planes: [
            ['far', 'green', 0, 0, -3, 40, 40],
            ['near', 'green', 4, 0, 3, 1, 1],
          ],
          resources: [splatResource('wide'), pictureResource('green', 4, 4)],
        }),
        refs,
        [project(0, 0), project(4, 0, 3), project(-4, 3)],
      ),
    );
    assert.ok(isRed(between.probes[0]), `the splat is over the far plane: ${show(between.probes[0])}`);
    assert.ok(isGreen(between.probes[1]), `the near plane is drawn: ${show(between.probes[1])}`);
    assert.ok(isGreen(between.probes[2]), `the far plane fills the rest: ${show(between.probes[2])}`);
  } finally {
    await runtime.close();
  }
});

test('BL019-F1: a splat shows its stored colour, and a picture keeps its own, in the same frame', async (t) => {
  if (skipUnavailable(t, assert)) return;
  // (230,49,49) is the colour the acceptance harness found drawn as (203,7,7): sRGB->linear with
  // no re-encode. Pure red (255,0,0) is invariant under that conversion, so the other tests could not see it.
  const STORED = [230, 49, 49];
  const paths = spzFiles(t, { cloud: [BLOB([0, 0, 0], STORED)] });
  const runtime = await directedRuntime(paths);
  try {
    const refs = await loadScene(runtime, paths);
    const plane = await runtime.load(sceneAsset('slot-stored', paths.stored, 'stored'));
    assert.equal(plane.state, 'ready', JSON.stringify(plane.error));
    refs.stored = reference('stored', 'slot-stored');
    const plan = splatPlan({
      splats: [['cloud-node', 'cloud', -2, 0, 0]],
      planes: [['wall', 'stored', 3, 0, 0, 2, 2]],
      resources: [splatResource('cloud'), pictureResource('stored', 4, 4)],
    });
    const frame = done(await draw(runtime, plan, refs, [project(-2, 0), project(3, 0)]));
    const [splat, image] = frame.probes;
    const delta = (pixel) => Math.max(...STORED.map((value, channel) => Math.abs(pixel[channel] - value)));
    // The centre of an opaque gaussian is the stored colour; the tolerance is the harness's own centre colour delta (24), far tighter in practice.
    assert.ok(delta(splat) <= 6, `the splat centre is the stored colour: ${show(splat)} (stored ${show(STORED)})`);
    assert.ok(delta(image) <= 1, `the picture is untouched by the splat colour handling: ${show(image)}`);
    assert.ok(Math.abs(splat[0] - 203) > 15, 'not the sRGB->linear value (203,7,7)');
  } finally {
    await runtime.close();
  }
});

test('the picture is a function of the plan: out of order, repeated and after a cold start', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = spzFiles(t, { cloud: [BLOB([0, 0, 0], RED)] });
  const runtime = await directedRuntime(paths);
  try {
    const refs = await loadScene(runtime, paths);
    const frame = (k) =>
      splatPlan({
        splats: [['cloud-node', 'cloud', 0.25 * k, 0, 0]],
        planes: [['wall', 'green', 4, 3, -2, 1, 1]],
        resources: [splatResource('cloud'), pictureResource('green', 4, 4)],
        camera: { position: vec(k / 8, 0, DISTANCE), target: vec(k / 8, 0, 0) },
      });
    const ordered = {};
    for (const k of [0, 1, 2, 3, 4, 5]) ordered[k] = done(await draw(runtime, frame(k), refs, [project(0, 0)])).hash;
    assert.equal(new Set(Object.values(ordered)).size, 6, 'six different pictures');
    for (const k of [4, 4, 0, 5, 2, 1, 3, 0, 5]) {
      assert.equal(done(await draw(runtime, frame(k), refs, [project(0, 0)])).hash, ordered[k], `frame ${k} after other frames`);
    }
    const cold = await directedRuntime(paths);
    try {
      const coldRefs = await loadScene(cold, paths);
      assert.equal(done(await draw(cold, frame(3), coldRefs, [project(0, 0)])).hash, ordered[3], 'a cold page draws the same picture');
    } finally {
      await cold.close();
    }
  } finally {
    await runtime.close();
  }
});

test('two nodes may share one file; the file is decoded for each and released with the frame', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = spzFiles(t, { cloud: [BLOB([0, 0, 0], RED)] });
  const runtime = await directedRuntime(paths);
  try {
    const refs = await loadScene(runtime, paths);
    const plan = splatPlan({
      splats: [['left', 'cloud', -2, 0, 0], ['right', 'cloud', 2, 0, 0]],
      resources: [splatResource('cloud', { usedBy: ['left', 'right'] })],
    });
    const first = done(await draw(runtime, plan, refs, [project(-2, 0), project(2, 0), project(0, 0)]));
    assert.ok(isRed(first.probes[0]) && isRed(first.probes[1]), `both nodes draw: ${show(first.probes[0])} ${show(first.probes[1])}`);
    assert.ok(isBlack(first.probes[2]));
    assert.equal(first.source.directed_scene_splats, 2);
    // A second frame finds the asset's own mesh free again (the lease was given back).
    assert.equal(done(await draw(runtime, plan, refs, [project(-2, 0)])).hash, first.hash);
  } finally {
    await runtime.close();
  }
});

test('refusals are typed and leave the runtime drawing: unresolved, mismatched, stale and unlisted splats', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = spzFiles(t, { cloud: [BLOB([0, 0, 0], RED)] });
  const runtime = await directedRuntime(paths);
  try {
    const refs = await loadScene(runtime, paths);
    const plan = splatPlan({ splats: [['cloud-node', 'cloud', 0, 0, 0]], resources: [splatResource('cloud')] });
    const good = done(await draw(runtime, plan, refs, [project(0, 0)]));

    // A plan that needs splats the page was never told where to find.
    const none = await runtime.frame([directedLayer(plan, [])], { size: SIZE });
    assert.equal(none.error.code, 'resource_unresolved');

    // Loaded as a splat, but another revision than the plan requires.
    const other = await draw(runtime, plan, { cloud: reference('cloud', 'slot-cloud', { revision_id: 'cloud-r2' }) }, []);
    assert.equal(other.error.code, 'resource_mismatch');

    // The slot holds a picture instead: right identity, wrong kind.
    const still = await runtime.load(sceneAsset('slot-still', paths.green, 'cloud'));
    assert.equal(still.state, 'ready');
    const picture = await draw(runtime, plan, { cloud: reference('cloud', 'slot-still') }, []);
    assert.equal(picture.error.code, 'resource_mismatch');

    // The slot holds a ply under the spz identity: not the declared file.
    const ply = await runtime.load({ ...splatAsset('slot-ply', paths.plyCloud, 'cloud'), meta: { format: 'ply' } });
    assert.equal(ply.state, 'ready', JSON.stringify(ply.error));
    const mislabelled = await draw(runtime, plan, { cloud: reference('cloud', 'slot-ply') }, []);
    assert.equal(mislabelled.error.code, 'resource_mismatch');

    // A newer project generation released the old resource: the same references are stale.
    assert.equal((await runtime.load(sceneAsset('slot-next', paths.green, 'green', { generation: GENERATION + 1 }))).state, 'ready');
    const stale = await draw(runtime, plan, refs, []);
    assert.equal(stale.error.code, 'resource_unresolved', 'the old generation was released with its slot');

    // A splat node nobody listed, a plan that hides the capability, a partial plan: all refused before drawing.
    const bad = await runtime.frame([directedLayer({ ...plan, splats: [] }, [refs.cloud])], { size: SIZE });
    assert.equal(bad.error.code, 'invalid_plan');
    const quiet = await runtime.frame([directedLayer({ ...plan, requires: ['camera', 'nodeTransforms'] }, [refs.cloud])], { size: SIZE });
    assert.equal(quiet.error.code, 'invalid_plan');
    const partial = await runtime.frame(
      [directedLayer({ ...plan, complete: false, unsupported: [{ domain: 'lighting', subject: null }] }, [refs.cloud])],
      { size: SIZE },
    );
    assert.equal(partial.error.code, 'unsupported');
    assert.ok(good.hash, 'the first frame was a picture');
  } finally {
    await runtime.close();
  }
});

test('a lost context keeps the splat bytes: after the restore the same plan draws the same picture', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = spzFiles(t, { cloud: [BLOB([0, 0, 0], RED)] });
  const runtime = await directedRuntime(paths);
  try {
    const refs = await loadScene(runtime, paths);
    const plan = splatPlan({
      splats: [['cloud-node', 'cloud', 1, 0, 0]],
      planes: [['wall', 'green', 4, 3, -2, 1, 1]],
      resources: [splatResource('cloud'), pictureResource('green', 4, 4)],
    });
    const before = done(await draw(runtime, plan, refs, [project(1, 0)]));
    assert.ok(isRed(before.probes[0]));
    await loseNow(runtime);
    const dead = await draw(runtime, plan, refs, []);
    assert.equal(dead.error.code, 'context_lost');
    await restore(runtime);
    const after = done(await draw(runtime, plan, refs, [project(1, 0)]));
    assert.equal(after.hash, before.hash, 'the splat is rebuilt from its bytes on the new context');
  } finally {
    await runtime.close();
  }
});

test('a splat counts as a scene resource and any other kind is refused before it loads', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const paths = spzFiles(t, { cloud: [BLOB([0, 0, 0], RED)] });
  const runtime = await directedRuntime(paths);
  try {
    const refs = await loadScene(runtime, paths);
    const retention = await runtime.retention();
    assert.equal(retention.scene_resources, 2, 'a splat and a picture are both scene resources');
    // A kind that is neither is refused before it loads.
    const video = await runtime.load({ ...sceneAsset('slot-video', paths.green, 'video'), kind: 'video' });
    assert.equal(video.state, 'failed');
    assert.equal(video.error.code, 'unsupported_format');
    assert.ok(done(await draw(runtime, splatPlan({ splats: [['cloud-node', 'cloud', 0, 0, 0]], resources: [splatResource('cloud')] }), refs, [project(0, 0)])).hash);
  } finally {
    await runtime.close();
  }
});
