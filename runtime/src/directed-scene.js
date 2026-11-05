// Directed-scene layer (kind `directed_scene`): draws the evaluated ScenePlan of the core
// (crates/core/src/scene_plan, F037) with Three.js. Contract: docs/features/scene-renderer.md.
//
// WHY this module does no domain work: camera, hierarchy, animation, sockets and draw order are
// resolved by the core and arrive as numbers. Here a plan is only *checked* (is it a plan this
// runtime can draw whole?) and *executed* (a perspective camera, one textured plane per layer,
// in the order the plan lists them). Anything the plan does not carry (light, weather, effects,
// rigs, splats, data maps) is refused with a typed error, never approximated.
//
// The checks are pure (no GL, no DOM), so they run under `node --test` without a browser.

import * as THREE from 'three';

import { RuntimeError } from './protocol.js';

export const SCENE_PLAN_KIND = 'tessel-scene-plan';
export const SCENE_PLAN_VERSION = 1;

/**
 * What this runtime executes, spelled as the core's `Capability::as_str`. A plan that requires
 * anything else is refused (`unsupported` + `details.missing`). Kept in step by a Rust test
 * (`scene_frames.rs`) that parses this list with `Capability::parse`.
 */
export const SCENE_CAPABILITIES = [
  'camera',
  'nodeTransforms',
  'sockets',
  'performanceChannels',
  'imagePlanes',
  'straightAlpha',
  'opaqueAlpha',
  'compositionPassOrder',
];

/** Advertised in `ready.capabilities.directed_scene`, so a host can check before it sends. */
export const DIRECTED_SCENE_CAPABILITIES = {
  layer_kind: 'directed_scene',
  plan_kind: SCENE_PLAN_KIND,
  plan_versions: [SCENE_PLAN_VERSION],
  capabilities: SCENE_CAPABILITIES,
  resource_identity: ['generation', 'asset_id', 'revision_id', 'content_hash'],
  resource_kinds: ['still'],
};

// The camera planes of the origin's runtime (ThreeSceneRuntime: PerspectiveCamera(50, aspect, 0.1, 1000)).
const NEAR = 0.1;
const FAR = 1000;

const isObject = (value) => typeof value === 'object' && value !== null && !Array.isArray(value);

const invalid = (pointer, message) =>
  new RuntimeError('invalid_plan', `Scene plan ${pointer || '/'}: ${message}`, { pointer });

const objectAt = (value, pointer) => {
  if (!isObject(value)) throw invalid(pointer, 'expected an object');
  return value;
};

const arrayAt = (value, pointer) => {
  if (!Array.isArray(value)) throw invalid(pointer, 'expected an array');
  return value;
};

const stringAt = (value, pointer) => {
  if (typeof value !== 'string' || value === '') throw invalid(pointer, 'expected a non-empty string');
  return value;
};

const finiteAt = (value, pointer) => {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw invalid(pointer, 'expected a finite number');
  return value;
};

const positiveAt = (value, pointer) => {
  if (finiteAt(value, pointer) <= 0) throw invalid(pointer, 'expected a positive number');
  return value;
};

// The core writes a vector as `{ x, y, z }` (`Vec3::to_json`); the matrices are the only arrays.
const vec3At = (value, pointer) => {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) throw invalid(pointer, 'expected an object {x, y, z}');
  return ['x', 'y', 'z'].map((axis) => finiteAt(value[axis], `${pointer}/${axis}`));
};

const matrixAt = (value, pointer) => {
  const items = arrayAt(value, pointer);
  if (items.length !== 16) throw invalid(pointer, 'expected 16 numbers (column-major)');
  return items.map((item, index) => finiteAt(item, `${pointer}/${index}`));
};

const integerAt = (value, pointer) => {
  if (!Number.isSafeInteger(value) || value < 0) throw invalid(pointer, 'expected a non-negative integer');
  return value;
};

/**
 * The identity a host attaches to the `asset.load` of a scene resource (`descriptor.scene`).
 * `generation` is the host's project generation: it grows when a project is opened, replaced or
 * relinked, so a resource of an older world can never be mistaken for the current one.
 */
export const parseSceneIdentity = (raw, pointer = '/scene') => {
  const scene = objectAt(raw, pointer);
  return {
    assetId: stringAt(scene.asset_id, `${pointer}/asset_id`),
    revisionId: stringAt(scene.revision_id, `${pointer}/revision_id`),
    contentHash: stringAt(scene.content_hash, `${pointer}/content_hash`),
    generation: integerAt(scene.generation, `${pointer}/generation`),
  };
};

/**
 * Reads a `directed_scene` layer into what the drawing needs, or throws the typed refusal.
 *
 * Order of refusals follows the core (`ScenePlan::from_json`): kind and version first, before any
 * other field; then partial plans and capabilities; then the shape of what is drawn.
 */
export const parseDirectedLayer = (layer) => {
  const plan = objectAt(layer?.scene_plan, '/scene_plan');
  if (plan.kind !== SCENE_PLAN_KIND) {
    throw invalid('/kind', `expected \`${SCENE_PLAN_KIND}\``);
  }
  if (plan.planVersion !== SCENE_PLAN_VERSION) {
    throw new RuntimeError(
      'unsupported_plan_version',
      `Scene plan version ${JSON.stringify(plan.planVersion)} is not supported (this runtime draws ${SCENE_PLAN_VERSION})`,
      { found: plan.planVersion, supported: SCENE_PLAN_VERSION },
    );
  }

  const unsupported = arrayAt(plan.unsupported, '/unsupported');
  if (plan.complete !== true || unsupported.length > 0) {
    // A partial plan is the scene without its light, weather or effects: not the picture.
    throw new RuntimeError(
      'unsupported',
      `The plan declares ${unsupported.length} domain(s) it does not carry; this runtime draws complete plans only`,
      { domains: unsupported.map((entry) => ({ domain: entry?.domain ?? null, subject: entry?.subject ?? null })) },
    );
  }
  const requires = arrayAt(plan.requires, '/requires').map((item, index) => stringAt(item, `/requires/${index}`));
  const missing = requires.filter((capability) => !SCENE_CAPABILITIES.includes(capability));
  if (missing.length > 0) {
    throw new RuntimeError('unsupported', `The plan requires capabilities this runtime does not have: ${missing.join(', ')}`, {
      missing,
    });
  }

  const output = objectAt(plan.output, '/output');
  const wholePixels = (value, pointer) => {
    const pixels = positiveAt(value, pointer);
    if (!Number.isInteger(pixels)) throw invalid(pointer, 'expected whole pixels');
    return pixels;
  };
  const width = wholePixels(output.width, '/output/width');
  const height = wholePixels(output.height, '/output/height');

  const camera = objectAt(plan.camera, '/camera');
  const fov = finiteAt(camera.fovDeg, '/camera/fovDeg');
  if (!(fov > 0 && fov < 180)) throw invalid('/camera/fovDeg', 'expected a field of view between 0 and 180 degrees');

  const nodes = new Map();
  arrayAt(plan.nodes, '/nodes').forEach((raw, index) => {
    const node = objectAt(raw, `/nodes/${index}`);
    nodes.set(stringAt(node.id, `/nodes/${index}/id`), { node, index });
  });

  const resources = new Map();
  arrayAt(plan.resources, '/resources').forEach((raw, index) => {
    const at = `/resources/${index}`;
    const resource = objectAt(raw, at);
    const assetId = stringAt(resource.assetId, `${at}/assetId`);
    if (resources.has(assetId)) throw invalid(`${at}/assetId`, `\`${assetId}\` is listed twice`);
    resources.set(assetId, {
      assetId,
      revisionId: stringAt(resource.revisionId, `${at}/revisionId`),
      contentHash: stringAt(resource.contentHash, `${at}/contentHash`),
      widthPx: positiveAt(resource.widthPx, `${at}/widthPx`),
      heightPx: positiveAt(resource.heightPx, `${at}/heightPx`),
    });
  });

  // `layers` is the paint order, back to front: it is drawn as listed, never re-sorted.
  const planes = arrayAt(plan.layers, '/layers').map((raw, index) => {
    const at = `/layers/${index}`;
    const entry = objectAt(raw, at);
    const nodeId = stringAt(entry.nodeId, `${at}/nodeId`);
    const assetId = stringAt(entry.assetId, `${at}/assetId`);
    const alpha = entry.alpha;
    if (alpha !== 'straight' && alpha !== 'opaque') throw invalid(`${at}/alpha`, 'expected `straight` or `opaque`');
    const found = nodes.get(nodeId);
    if (!found) throw invalid(`${at}/nodeId`, `node \`${nodeId}\` is not in the plan`);
    const nodeAt = `/nodes/${found.index}`;
    if (found.node.kind !== 'imagePlane') throw invalid(`${nodeAt}/kind`, 'only image planes are drawn');
    if (found.node.assetId !== assetId) throw invalid(`${at}/assetId`, `node \`${nodeId}\` uses another asset`);
    if (!resources.has(assetId)) throw invalid(`${at}/assetId`, `\`${assetId}\` is not in the plan's resources`);
    return {
      nodeId,
      assetId,
      alpha,
      world: matrixAt(found.node.worldMatrix, `${nodeAt}/worldMatrix`),
      width: positiveAt(found.node.widthSceneUnits, `${nodeAt}/widthSceneUnits`),
      height: positiveAt(found.node.heightSceneUnits, `${nodeAt}/heightSceneUnits`),
    };
  });

  return {
    output: { width, height },
    camera: {
      position: vec3At(camera.position, '/camera/position'),
      target: vec3At(camera.target, '/camera/target'),
      fov,
    },
    planes,
    resources,
    references: parseResourceReferences(layer.resources ?? []),
  };
};

/**
 * `layer.resources`: for each asset id, the runtime slot the host loaded it into and the
 * identity it loaded. The runtime does not look anything up by `assetId` alone.
 */
const parseResourceReferences = (raw) => {
  const references = new Map();
  arrayAt(raw, '/resources').forEach((item, index) => {
    const at = `/resources/${index}`;
    const reference = objectAt(item, at);
    const assetId = stringAt(reference.asset_id, `${at}/asset_id`);
    if (references.has(assetId)) throw invalid(`${at}/asset_id`, `\`${assetId}\` is referenced twice`);
    references.set(assetId, {
      assetId,
      slot: stringAt(reference.slot, `${at}/slot`),
      revisionId: stringAt(reference.revision_id, `${at}/revision_id`),
      contentHash: stringAt(reference.content_hash, `${at}/content_hash`),
      generation: integerAt(reference.generation, `${at}/generation`),
    });
  });
  return references;
};

/**
 * Binds each resource the plan requires to the loaded entry that has exactly its identity.
 * Returns `assetId -> { slot, entry }`. Refusals:
 *   resource_unresolved  the host named no slot, or the slot holds nothing;
 *   resource_mismatch    the host's revision/hash differs from the plan's, or the file is not the
 *                        declared picture (size) or not a still;
 *   resource_stale       the slot was reloaded for another identity or generation since.
 * The asset id is never a cache key by itself: revision, content hash and generation must all agree.
 */
export const resolveSceneResources = (spec, assets) => {
  const bound = new Map();
  for (const resource of spec.resources.values()) {
    const { assetId, revisionId, contentHash } = resource;
    const reference = spec.references.get(assetId);
    if (!reference) {
      throw new RuntimeError('resource_unresolved', `The frame does not say where \`${assetId}\` was loaded`, { asset_id: assetId });
    }
    if (reference.revisionId !== revisionId || reference.contentHash !== contentHash) {
      throw new RuntimeError('resource_mismatch', `\`${assetId}\` was loaded for another revision or content than the plan requires`, {
        asset_id: assetId,
        expected: { revision_id: revisionId, content_hash: contentHash },
        got: { revision_id: reference.revisionId, content_hash: reference.contentHash },
      });
    }
    if (!assets.has(reference.slot)) {
      throw new RuntimeError('resource_unresolved', `\`${assetId}\` is not loaded (slot \`${reference.slot}\`)`, {
        asset_id: assetId,
        slot: reference.slot,
      });
    }
    const entry = assets.get(reference.slot);
    const held = entry.descriptor?.scene;
    const matches =
      isObject(held) &&
      held.asset_id === assetId &&
      held.revision_id === revisionId &&
      held.content_hash === contentHash &&
      held.generation === reference.generation;
    if (!matches) {
      throw new RuntimeError('resource_stale', `Slot \`${reference.slot}\` no longer holds \`${assetId}\` of this revision and generation`, {
        asset_id: assetId,
        slot: reference.slot,
        wanted: { revision_id: revisionId, content_hash: contentHash, generation: reference.generation },
        held: isObject(held) ? held : null,
      });
    }
    if (entry.kind !== 'still') {
      throw new RuntimeError('resource_mismatch', `\`${assetId}\` is a ${entry.kind}, the plan draws still images`, { asset_id: assetId });
    }
    if (entry.width !== resource.widthPx || entry.height !== resource.heightPx) {
      throw new RuntimeError('resource_mismatch', `\`${assetId}\` is ${entry.width}x${entry.height}, the plan declares ${resource.widthPx}x${resource.heightPx}`, {
        asset_id: assetId,
        expected: { width_px: resource.widthPx, height_px: resource.heightPx },
        got: { width_px: entry.width, height_px: entry.height },
      });
    }
    bound.set(assetId, { slot: reference.slot, entry });
  }
  return bound;
};

/**
 * Size of the picture the plan's output takes inside a `width` x `height` canvas ("contain").
 * Equal to the plan's output when the canvas is that size, so an export-size frame is pixel exact.
 */
export const containSize = (output, width, height) => {
  const fit = Math.min(width / output.width, height / output.height);
  return {
    width: Math.max(1, Math.round(output.width * fit)),
    height: Math.max(1, Math.round(output.height * fit)),
  };
};

/** A plane whose V axis is flipped: images are uploaded top row first (`flipY` off). */
const planeGeometry = (width, height) => {
  const geometry = new THREE.PlaneGeometry(width, height);
  const uv = geometry.getAttribute('uv');
  for (let index = 0; index < uv.count; index += 1) uv.setY(index, 1 - uv.getY(index));
  return geometry;
};

/**
 * The material of one layer slot. All layers are `transparent` on purpose: three.js draws opaque
 * objects before transparent ones, which would break the painter's order the plan fixed.
 *   straight  source-over with straight alpha (what the origin's planes use);
 *   opaque    replaces the color and leaves the destination alpha alone, so a picture with a
 *             stray alpha channel cannot show what is behind it.
 */
export const createSceneMaterial = (alpha, map) => {
  const common = {
    // The map is given at creation: three.js picks the shader variant (USE_MAP) when it first sees the material.
    map,
    transparent: true,
    side: THREE.DoubleSide,
    depthTest: false,
    depthWrite: false,
    premultipliedAlpha: false,
  };
  if (alpha === 'straight') return new THREE.MeshBasicMaterial({ ...common, blending: THREE.NormalBlending });
  return new THREE.MeshBasicMaterial({
    ...common,
    blending: THREE.CustomBlending,
    blendEquation: THREE.AddEquation,
    blendSrc: THREE.OneFactor,
    blendDst: THREE.ZeroFactor,
    blendEquationAlpha: THREE.AddEquation,
    blendSrcAlpha: THREE.ZeroFactor,
    blendDstAlpha: THREE.OneFactor,
  });
};

/**
 * The three.js scene and camera of the plan. `textureOf(assetId)` gives the texture of a layer's
 * picture, `materialOf(alpha, index)` its material (the compositor retains these between frames),
 * `keep` registers what must be disposed when the frame ends.
 */
export const buildDirectedScene = (spec, { textureOf, materialOf, keep }) => {
  const scene = new THREE.Scene();
  spec.planes.forEach((plane, index) => {
    const mesh = new THREE.Mesh(keep(planeGeometry(plane.width, plane.height)), materialOf(plane.alpha, index, textureOf(plane.assetId)));
    // The world matrix is the core's: ancestors, sockets and performance channels already applied.
    mesh.matrixAutoUpdate = false;
    mesh.matrix.fromArray(plane.world);
    // Position in the painter's queue = position in the plan. No depth sort decides anything.
    mesh.renderOrder = index;
    mesh.frustumCulled = false;
    scene.add(mesh);
  });
  scene.updateMatrixWorld(true);

  const camera = new THREE.PerspectiveCamera(spec.camera.fov, spec.output.width / spec.output.height, NEAR, FAR);
  camera.position.set(...spec.camera.position);
  camera.up.set(0, 1, 0);
  camera.lookAt(...spec.camera.target);
  camera.updateMatrixWorld(true);
  camera.updateProjectionMatrix();
  return { scene, camera };
};

/** Draws into `target`: opaque black first (alpha 1), then the planes in plan order. */
export const drawDirectedScene = (renderer, target, { scene, camera }) => {
  renderer.setRenderTarget(target);
  renderer.setClearColor(0x000000, 1);
  renderer.clear(true, true, true);
  renderer.render(scene, camera);
  renderer.setRenderTarget(null);
};
