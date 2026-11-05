// Helpers of the directed-scene tests. Not a test file (no `.test.` in the name).
//
// The plans here are written by hand in the core's wire form (crates/core/src/scene_plan/json.rs)
// so the runtime can be tested without Rust. That they match what the core really produces is
// proved the other way round, by crates/render/tests/scene_frames.rs, which draws plans the core
// built from a bundle.
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { png } from '../scripts/perf/png.mjs';
import { flat, openRuntime, transform } from './support.mjs';

export const SIZE = [160, 90];
export const GENERATION = 1;

/** Column-major translation, the shape of a plan node's `worldMatrix` for an unrotated, unscaled node. */
export const translation = (x, y, z) => [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, x, y, z, 1];

/** A 8x8 picture whose quadrants say which way is up: red, green over blue, yellow. */
export const quadrants = () =>
  png(8, 8, (x, y) => {
    if (y < 4) return x < 4 ? [255, 0, 0, 255] : [0, 255, 0, 255];
    return x < 4 ? [0, 0, 255, 255] : [255, 255, 0, 255];
  });

/** Project files (`name -> PNG bytes`), removed after the test. Returns `name -> absolute path`. */
export const makeFiles = (t, pictures) => {
  const directory = mkdtempSync(join(tmpdir(), 'tessel-directed-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const paths = {};
  for (const [name, bytes] of Object.entries(pictures)) {
    paths[name] = join(directory, `${name}.png`);
    writeFileSync(paths[name], bytes);
  }
  return paths;
};

export const resource = (assetId, widthPx, heightPx, extra = {}) => ({
  assetId,
  revisionId: `${assetId}-r1`,
  contentHash: `hash-${assetId}`,
  widthPx,
  heightPx,
  ...extra,
});

export const node = (id, assetId, x, y, z, width, height, extra = {}) => ({
  id,
  kind: 'imagePlane',
  parent: null,
  assetId,
  widthSceneUnits: width,
  heightSceneUnits: height,
  worldMatrix: translation(x, y, z),
  localMatrix: translation(x, y, z),
  renderOrder: 0,
  ...extra,
});

/** A vector in the core's wire form (`Vec3::to_json`). */
export const vec = (x, y, z) => ({ x, y, z });

/** A complete plan in the core's wire form. `planes` are listed back to front, as the plan would. */
export const scenePlan = ({ nodes, layers, resources, camera = {}, output = SIZE, requires, extra = {} }) => ({
  kind: 'tessel-scene-plan',
  planVersion: 1,
  complete: true,
  requires: requires ?? ['camera', 'nodeTransforms', 'imagePlanes', 'straightAlpha', 'opaqueAlpha'],
  source: { bundleId: 'b', revisionId: 'r', sceneDefinitionId: 's', sceneInstanceId: 'i' },
  output: { width: output[0], height: output[1], fps: { numerator: 24, denominator: 1 }, colorSpace: 'srgb-display', alphaMode: 'straight' },
  camera: { position: vec(0, 0, 10), target: vec(0, 0, 0), fovDeg: 45, velocity: vec(0, 0, 0), fovVelocityDegPerFrame: 0, ...camera },
  nodes,
  layers: layers.map(([nodeId, assetId, alpha = 'straight']) => ({ nodeId, assetId, alpha, renderOrder: 0, passOrder: null })),
  resources,
  unsupported: [],
  uninterpreted: [],
  ...extra,
});

/** What the host sends for `plan`: the layer, with where each resource was loaded and as what. */
export const directedLayer = (plan, references, extra = {}) => ({
  id: 'layer:directed',
  kind: 'directed_scene',
  opacity: 1,
  blend: 'normal',
  transform: transform(),
  scene_plan: plan,
  resources: references,
  ...extra,
});

export const reference = (assetId, slot, extra = {}) => ({
  asset_id: assetId,
  slot,
  revision_id: `${assetId}-r1`,
  content_hash: `hash-${assetId}`,
  generation: GENERATION,
  ...extra,
});

/** The `asset.load` descriptor of a scene resource: the slot is the runtime id, `scene` the identity. */
export const sceneAsset = (slot, path, assetId, extra = {}) => ({
  id: slot,
  kind: 'still',
  source: { type: 'file', path },
  meta: {},
  scene: { asset_id: assetId, revision_id: `${assetId}-r1`, content_hash: `hash-${assetId}`, generation: GENERATION, ...extra },
});

/** A runtime with `paths` allowed. */
export const directedRuntime = async (paths, hello = {}) => {
  const runtime = await openRuntime({ extraFiles: Object.values(paths), hello });
  try {
    await runtime.hello();
    return runtime;
  } catch (error) {
    await runtime.close();
    throw error;
  }
};

export { flat, png };
