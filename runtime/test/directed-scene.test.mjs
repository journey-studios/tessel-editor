// Pure checks of the directed-scene layer (no GL, no browser): what a plan must be to be drawn,
// which refusals are typed, and how a loaded resource is bound to the identity the plan asks for.
// The drawing itself is proved with real pixels in directed-scene-browser.test.mjs.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  DIRECTED_SCENE_CAPABILITIES,
  SCENE_CAPABILITIES,
  containSize,
  parseDirectedLayer,
  parseSceneIdentity,
  resolveSceneResources,
} from '../src/directed-scene.js';
import { RuntimeError } from '../src/protocol.js';
import { directedLayer, node, reference, resource, scenePlan } from './directed-scene-fixtures.mjs';

const twoPlanes = (overrides = {}) =>
  scenePlan({
    nodes: [node('back', 'wall', 0, 0, -2, 6, 3), node('front', 'hero', 0.5, 0, 0, 2, 2)],
    layers: [['back', 'wall', 'opaque'], ['front', 'hero']],
    resources: [resource('hero', 8, 8), resource('wall', 16, 9)],
    ...overrides,
  });

const references = () => [reference('hero', 'slot-0'), reference('wall', 'slot-1')];

const refusal = (callback) => {
  try {
    callback();
  } catch (error) {
    assert.ok(error instanceof RuntimeError, `a typed RuntimeError, got ${error}`);
    return error;
  }
  assert.fail('expected a refusal');
};

test('a complete plan is read as listed: paint order is the plan order, not depth or id order', () => {
  const spec = parseDirectedLayer(directedLayer(twoPlanes(), references()));
  assert.deepEqual(spec.planes.map((plane) => [plane.nodeId, plane.alpha]), [['back', 'opaque'], ['front', 'straight']]);
  // The front plane is nearer to the camera and listed last; swapping the list swaps the order,
  // whatever the depth says.
  const swapped = parseDirectedLayer(
    directedLayer(twoPlanes({ layers: [['front', 'hero'], ['back', 'wall', 'opaque']] }), references()),
  );
  assert.deepEqual(swapped.planes.map((plane) => plane.nodeId), ['front', 'back']);
  assert.deepEqual(spec.output, { width: 160, height: 90 });
  assert.deepEqual(spec.camera, { position: [0, 0, 10], target: [0, 0, 0], fov: 45 });
  assert.equal(spec.planes[1].world[12], 0.5, 'the world matrix is carried untouched');
});

test('a partial plan, an unknown version and a foreign document are refused before anything is drawn', () => {
  const partial = twoPlanes({
    extra: { complete: false, unsupported: [{ domain: 'lighting', subject: null }, { domain: 'splat', subject: 'cloud' }] },
  });
  const partialError = refusal(() => parseDirectedLayer(directedLayer(partial, references())));
  assert.equal(partialError.code, 'unsupported');
  assert.deepEqual(partialError.details.domains, [
    { domain: 'lighting', subject: null },
    { domain: 'splat', subject: 'cloud' },
  ]);
  // `complete: true` with a non-empty `unsupported` is contradictory, not complete.
  const lying = twoPlanes({ extra: { unsupported: [{ domain: 'weather', subject: null }] } });
  assert.equal(refusal(() => parseDirectedLayer(directedLayer(lying, references()))).code, 'unsupported');

  const future = twoPlanes({ extra: { planVersion: 2, camera: 'not even read' } });
  const versionError = refusal(() => parseDirectedLayer(directedLayer(future, references())));
  assert.equal(versionError.code, 'unsupported_plan_version');
  assert.deepEqual([versionError.details.found, versionError.details.supported], [2, 1]);

  assert.equal(refusal(() => parseDirectedLayer(directedLayer({ kind: 'something-else' }, []))).code, 'invalid_plan');
  assert.equal(refusal(() => parseDirectedLayer({ kind: 'directed_scene' })).code, 'invalid_plan');
});

test('a capability the runtime does not have is named; the ones it has are the core list', () => {
  const needy = twoPlanes({ requires: ['camera', 'imagePlanes', 'normalMaps', 'hdr'] });
  const error = refusal(() => parseDirectedLayer(directedLayer(needy, references())));
  assert.equal(error.code, 'unsupported');
  assert.deepEqual(error.details.missing, ['normalMaps', 'hdr']);
  // Advertised to the host in `ready`, spelled as `Capability::as_str` in the core.
  assert.deepEqual(DIRECTED_SCENE_CAPABILITIES.capabilities, SCENE_CAPABILITIES);
  assert.equal(SCENE_CAPABILITIES.length, 13);
  assert.ok(SCENE_CAPABILITIES.includes('fxNoise'));
  assert.ok(SCENE_CAPABILITIES.includes('fxShotScope'), 'F062: the 13th capability');
  assert.deepEqual(DIRECTED_SCENE_CAPABILITIES.resource_identity, ['generation', 'asset_id', 'revision_id', 'content_hash']);
});

test('malformed plans name the pointer instead of drawing something', () => {
  const cases = [
    ['/camera/fovDeg', (plan) => { plan.camera.fovDeg = 0; }],
    ['/camera/position/z', (plan) => { plan.camera.position = { x: 0, y: 1 }; }],
    ['/nodes/0/worldMatrix', (plan) => { plan.nodes[0].worldMatrix = plan.nodes[0].worldMatrix.slice(0, 15); }],
    ['/nodes/1/worldMatrix/3', (plan) => { plan.nodes[1].worldMatrix[3] = null; }],
    ['/nodes/0/widthSceneUnits', (plan) => { plan.nodes[0].widthSceneUnits = -1; }],
    ['/layers/0/nodeId', (plan) => { plan.layers[0].nodeId = 'ghost'; }],
    ['/layers/0/alpha', (plan) => { plan.layers[0].alpha = 'premultiplied'; }],
    ['/layers/1/assetId', (plan) => { plan.layers[1].assetId = 'wall'; }],
    ['/output/width', (plan) => { plan.output.width = 160.5; }],
    ['/resources/1/assetId', (plan) => { plan.resources[1].assetId = 'hero'; }],
  ];
  for (const [pointer, mutate] of cases) {
    const plan = twoPlanes();
    mutate(plan);
    const error = refusal(() => parseDirectedLayer(directedLayer(plan, references())));
    assert.equal(error.code, 'invalid_plan', pointer);
    assert.equal(error.details.pointer, pointer);
  }
  // A layer that points at a group is not an image plane.
  const group = twoPlanes();
  group.nodes[0].kind = 'group';
  assert.equal(refusal(() => parseDirectedLayer(directedLayer(group, references()))).details.pointer, '/nodes/0/kind');
});

test('resource references are checked for shape, repeats and identity of the scene descriptor', () => {
  const repeated = [...references(), reference('hero', 'slot-2')];
  assert.equal(refusal(() => parseDirectedLayer(directedLayer(twoPlanes(), repeated))).details.pointer, '/resources/2/asset_id');
  const noGeneration = [reference('hero', 'slot-0', { generation: -1 }), reference('wall', 'slot-1')];
  assert.equal(refusal(() => parseDirectedLayer(directedLayer(twoPlanes(), noGeneration))).details.pointer, '/resources/0/generation');

  const identity = parseSceneIdentity({ asset_id: 'a', revision_id: 'r', content_hash: 'h', generation: 3 });
  assert.deepEqual(identity, { assetId: 'a', revisionId: 'r', contentHash: 'h', generation: 3 });
  assert.equal(refusal(() => parseSceneIdentity({ asset_id: 'a', revision_id: 'r', content_hash: 'h' })).code, 'invalid_plan');
});

/** A stand-in for the asset store: only what `resolveSceneResources` reads. */
const fakeStore = (slots) => ({
  has: (slot) => slots.has(slot),
  get: (slot) => slots.get(slot),
});
const loaded = (assetId, width, height, extra = {}) => ({
  kind: 'still',
  width,
  height,
  descriptor: {
    scene: { asset_id: assetId, revision_id: `${assetId}-r1`, content_hash: `hash-${assetId}`, generation: 1 },
    ...extra,
  },
});
const goodStore = () =>
  fakeStore(new Map([['slot-0', loaded('hero', 8, 8)], ['slot-1', loaded('wall', 16, 9)]]));

test('a resource is bound only to the entry that has exactly the identity the plan asks for', () => {
  const spec = parseDirectedLayer(directedLayer(twoPlanes(), references()));
  const bound = resolveSceneResources(spec, goodStore());
  assert.deepEqual([...bound.keys()].sort(), ['hero', 'wall']);
  assert.equal(bound.get('hero').slot, 'slot-0');
});

test('unresolved, mismatched and stale resources fail with their own typed code and say which asset', () => {
  const layerWith = (refs, plan = twoPlanes()) => parseDirectedLayer(directedLayer(plan, refs));

  const missing = refusal(() => resolveSceneResources(layerWith([reference('hero', 'slot-0')]), goodStore()));
  assert.deepEqual([missing.code, missing.details.asset_id], ['resource_unresolved', 'wall']);

  const notLoaded = refusal(() => resolveSceneResources(layerWith(references()), fakeStore(new Map([['slot-0', loaded('hero', 8, 8)]]))));
  assert.deepEqual([notLoaded.code, notLoaded.details.slot], ['resource_unresolved', 'slot-1']);

  // The host loaded another revision than the plan requires: not the same picture, whatever the id.
  const otherRevision = [reference('hero', 'slot-0', { revision_id: 'hero-r2' }), reference('wall', 'slot-1')];
  const mismatch = refusal(() => resolveSceneResources(layerWith(otherRevision), goodStore()));
  assert.equal(mismatch.code, 'resource_mismatch');
  assert.equal(mismatch.details.expected.revision_id, 'hero-r1');
  assert.equal(mismatch.details.got.revision_id, 'hero-r2');
  const otherHash = [reference('hero', 'slot-0', { content_hash: 'tampered' }), reference('wall', 'slot-1')];
  assert.equal(refusal(() => resolveSceneResources(layerWith(otherHash), goodStore())).code, 'resource_mismatch');

  // The slot was reloaded for another asset / revision / generation since the frame was prepared.
  for (const held of [
    { asset_id: 'other', revision_id: 'hero-r1', content_hash: 'hash-hero', generation: 1 },
    { asset_id: 'hero', revision_id: 'hero-r2', content_hash: 'hash-hero', generation: 1 },
    { asset_id: 'hero', revision_id: 'hero-r1', content_hash: 'hash-other', generation: 1 },
    { asset_id: 'hero', revision_id: 'hero-r1', content_hash: 'hash-hero', generation: 2 },
  ]) {
    const store = fakeStore(new Map([['slot-0', { ...loaded('hero', 8, 8), descriptor: { scene: held } }], ['slot-1', loaded('wall', 16, 9)]]));
    const stale = refusal(() => resolveSceneResources(layerWith(references()), store));
    assert.equal(stale.code, 'resource_stale', JSON.stringify(held));
    assert.equal(stale.details.asset_id, 'hero');
  }
  // An entry that was never a directed-scene resource has no identity to compare.
  const plain = fakeStore(new Map([['slot-0', { kind: 'still', width: 8, height: 8, descriptor: {} }], ['slot-1', loaded('wall', 16, 9)]]));
  assert.equal(refusal(() => resolveSceneResources(layerWith(references()), plain)).code, 'resource_stale');

  // Right identity, wrong file: not a still, or not the declared size.
  const video = fakeStore(new Map([['slot-0', { ...loaded('hero', 8, 8), kind: 'video' }], ['slot-1', loaded('wall', 16, 9)]]));
  assert.equal(refusal(() => resolveSceneResources(layerWith(references()), video)).code, 'resource_mismatch');
  const wrongSize = fakeStore(new Map([['slot-0', loaded('hero', 9, 8)], ['slot-1', loaded('wall', 16, 9)]]));
  const sized = refusal(() => resolveSceneResources(layerWith(references()), wrongSize));
  assert.deepEqual([sized.code, sized.details.expected, sized.details.got], ['resource_mismatch', { width_px: 8, height_px: 8 }, { width_px: 9, height_px: 8 }]);
});

test('the picture of a plan fits the canvas like any layer and is exact at the plan size', () => {
  const output = { width: 320, height: 180 };
  assert.deepEqual(containSize(output, 320, 180), { width: 320, height: 180 });
  assert.deepEqual(containSize(output, 160, 90), { width: 160, height: 90 });
  assert.deepEqual(containSize(output, 160, 160), { width: 160, height: 90 }, 'letterboxed, never stretched');
  assert.deepEqual(containSize(output, 640, 180), { width: 320, height: 180 }, 'pillarboxed');
  assert.deepEqual(containSize({ width: 1, height: 1 }, 1, 1), { width: 1, height: 1 });
});

test('concurrent scene resource loads cannot exceed the entry budget at registration', async () => {
  const { AssetStore } = await import('../src/assets.js');
  const { LIMITS } = await import('../src/retention.js');
  const store = new AssetStore({ assetUrl: () => '', limits: { ...LIMITS, sceneResourceEntries: 1 } });
  let finishA, finishB;
  const closes = [];
  store.loadStill = (descriptor) => new Promise((resolve) => {
    const finish = () => resolve({ entry: { width: 1, height: 1, bitmap: { close: () => closes.push(descriptor.id) } }, meta: {} });
    if (descriptor.id === 'a') finishA = finish; else finishB = finish;
  });
  const descriptor = (id) => ({ id, kind: 'still', source: { type: 'file', path: id },
    scene: reference(id, id) });
  const a = store.load(descriptor('a'));
  const b = store.load(descriptor('b'));
  finishA();
  await a;
  finishB();
  await assert.rejects(b, (error) => error.code === 'resource_budget');
  assert.equal(store.sceneEntries().length, 1);
  assert.deepEqual(closes, ['b'], 'the refused decoded bitmap is released');
  store.release('a');
});
