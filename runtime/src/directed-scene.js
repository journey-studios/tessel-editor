// Directed-scene layer (kind `directed_scene`): draws the evaluated ScenePlan of the core
// (crates/core/src/scene_plan, F037) with Three.js. Contract: docs/features/scene-renderer.md.
//
// WHY this module does no domain work: camera, hierarchy, animation, sockets and draw order are
// resolved by the core and arrive as numbers. Here a plan is only *checked* (is it a plan this
// runtime can draw whole?) and *executed* (a perspective camera, one textured plane per layer,
// in the order the plan lists them, and the Gaussian splats of Spark painted between them where the
// plan's `afterLayers` says, each placed by its node's world matrix, which must be a similarity).
// Anything the plan does not carry (ambient or shadowed light, weather, rigs, data maps, a splat matrix
// with a non-uniform or mirrored scale) is refused with a typed error, never approximated.
//
// Point lights (F060) are the one light this runtime draws: they brighten the IMAGE PLANES by
// `lambert * falloff^2` (see `LIGHT_FRAGMENT`). Splats are not lit: Spark draws them with its own
// shader and nothing here infers a normal for a Gaussian, so a lit scene's splats keep their picture.
// That is a stated limit of this slice, not an accident.
//
// The checks are pure (no GL, no DOM), so they run under `node --test` without a browser.

import * as THREE from 'three';

import { parseOutputFx } from './output-fx.js';
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
  'splats',
  'outputFx',
  'fxNoise',
  'pointLights',
  'fxShotScope',
];

/** Advertised in `ready.capabilities.directed_scene`, so a host can check before it sends. */
export const DIRECTED_SCENE_CAPABILITIES = {
  layer_kind: 'directed_scene',
  plan_kind: SCENE_PLAN_KIND,
  plan_versions: [SCENE_PLAN_VERSION],
  capabilities: SCENE_CAPABILITIES,
  resource_identity: ['generation', 'asset_id', 'revision_id', 'content_hash'],
  resource_kinds: ['still', 'splat'],
};

/** At most this many lights, the origin's `MAX_SCENE_LIGHTS` (the core refuses more). */
export const MAX_POINT_LIGHTS = 4;
/** The origin's bounds of a light's intensity (the core refuses more). */
const MAX_LIGHT_INTENSITY = 20;

/**
 * Relative tolerance of the similarity test of a splat's world matrix; the same number as the core's
 * `SPLAT_SIMILARITY_TOLERANCE` (crates/core/src/scene_plan/validate.rs), and the guard below the
 * same rule as its `splat_matrix_fault`, which the core already applies to every plan it builds or
 * reads: this one covers a plan that reaches the runtime by another road.
 */
export const SPLAT_SIMILARITY_TOLERANCE = 1e-6;

/**
 * Why a column-major world matrix is not a similarity (rotation, one positive uniform scale,
 * translation), or `null`. Spark 2.4.0 decomposes a splat's matrix and keeps `(sx + sy + sz) / 3`
 * as its only scale, with no shear: anything else would be drawn as another picture than the plan's,
 * so it is refused instead of approximated (BL123).
 */
export const splatMatrixFault = (m) => {
  if (m[3] !== 0 || m[7] !== 0 || m[11] !== 0 || m[15] !== 1) return 'the matrix is not affine';
  const axes = [
    [m[0], m[1], m[2]],
    [m[4], m[5], m[6]],
    [m[8], m[9], m[10]],
  ];
  const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  const lengths = axes.map((axis) => Math.sqrt(dot(axis, axis)));
  const mean = (lengths[0] + lengths[1] + lengths[2]) / 3;
  if (!Number.isFinite(mean) || lengths.some((length) => !(length > 1e-12))) return 'the scale is degenerate';
  if (lengths.some((length) => Math.abs(length - mean) > SPLAT_SIMILARITY_TOLERANCE * mean)) return 'the scale is not uniform';
  for (const [a, b] of [[0, 1], [0, 2], [1, 2]]) {
    if (Math.abs(dot(axes[a], axes[b])) > SPLAT_SIMILARITY_TOLERANCE * mean * mean) return 'the axes are sheared';
  }
  const [x, y, z] = axes;
  const cross = [y[1] * z[2] - y[2] * z[1], y[2] * z[0] - y[0] * z[2], y[0] * z[1] - y[1] * z[0]];
  if (dot(x, cross) < 0) return 'the transform is mirrored';
  return null;
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
      {
        domains: unsupported.map((entry) => {
          const domain = { domain: entry?.domain ?? null, subject: entry?.subject ?? null };
          // Additive (F060): why a partly supported domain is refused, when the plan says.
          if (typeof entry?.reason === 'string') domain.reason = entry.reason;
          return domain;
        }),
      },
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
    // The core always writes `format`; a hand-written plan without it is a picture, as before.
    const format = resource.format ?? 'png';
    if (format !== 'png' && format !== 'spz') throw invalid(`${at}/format`, 'expected `png` or `spz`');
    resources.set(assetId, {
      assetId,
      format,
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
    if (resources.get(assetId).format !== 'png') throw invalid(`${at}/assetId`, 'an image plane reads a png');
    return {
      nodeId,
      assetId,
      alpha,
      world: matrixAt(found.node.worldMatrix, `${nodeAt}/worldMatrix`),
      width: positiveAt(found.node.widthSceneUnits, `${nodeAt}/widthSceneUnits`),
      height: positiveAt(found.node.heightSceneUnits, `${nodeAt}/heightSceneUnits`),
    };
  });

  // The splat nodes. Each one is the evaluated node (its world matrix), the `spz` it reads and the
  // slot the core gave it among the planes (`afterLayers`: how many planes are painted before it);
  // a splat node the plan does not list would silently not be drawn, so it is refused.
  const listed = new Set();
  const splats = arrayAt(plan.splats ?? [], '/splats').map((raw, index) => {
    const at = `/splats/${index}`;
    const entry = objectAt(raw, at);
    const nodeId = stringAt(entry.nodeId, `${at}/nodeId`);
    if (listed.has(nodeId)) throw invalid(`${at}/nodeId`, `splat node \`${nodeId}\` is listed twice`);
    listed.add(nodeId);
    const assetId = stringAt(entry.assetId, `${at}/assetId`);
    const found = nodes.get(nodeId);
    if (!found) throw invalid(`${at}/nodeId`, `node \`${nodeId}\` is not in the plan`);
    const nodeAt = `/nodes/${found.index}`;
    if (found.node.kind !== 'splat') throw invalid(`${nodeAt}/kind`, 'only splat nodes are splats');
    if (found.node.assetId !== assetId) throw invalid(`${at}/assetId`, `node \`${nodeId}\` uses another asset`);
    const held = resources.get(assetId);
    if (!held) throw invalid(`${at}/assetId`, `\`${assetId}\` is not in the plan's resources`);
    if (held.format !== 'spz') throw invalid(`${at}/assetId`, 'a splat reads an spz');
    const after = integerAt(entry.afterLayers, `${at}/afterLayers`);
    if (after > planes.length) throw invalid(`${at}/afterLayers`, `expected at most ${planes.length} (the number of layers)`);
    const world = matrixAt(found.node.worldMatrix, `${nodeAt}/worldMatrix`);
    const fault = splatMatrixFault(world);
    if (fault) {
      throw new RuntimeError(
        'unsupported',
        `The splat \`${nodeId}\` cannot be drawn as planned: ${fault} (a splat takes a rotation, one positive uniform scale and a translation)`,
        { unsupported: 'splat_transform', node: nodeId, reason: fault, pointer: `${nodeAt}/worldMatrix` },
      );
    }
    return { nodeId, assetId, after, world };
  });
  for (const [nodeId, { node }] of nodes) {
    if (node.kind === 'splat' && !listed.has(nodeId)) throw invalid('/splats', `splat node \`${nodeId}\` is not listed`);
  }
  if (splats.length > 0 && !requires.includes('splats')) throw invalid('/requires', 'a plan with splats must require `splats`');

  // F016: the resolved effect passes drawn over the finished picture (absent means none). F062: the
  // passes of a shot come first, and are compared with the shot of the plan's own frame.
  const fx = parseOutputFx(plan.fx, requires, plan.frame);

  // F060: the evaluated point lights (absent means none). A hidden light is kept in the plan for the
  // editor's marker and takes no slot here; any visible one needs the capability, as in the core.
  const lights = parsePointLights(plan.lights, requires);

  return {
    output: { width, height },
    fx,
    camera: {
      position: vec3At(camera.position, '/camera/position'),
      target: vec3At(camera.target, '/camera/target'),
      fov,
    },
    planes,
    splats,
    lights,
    resources,
    references: parseResourceReferences(layer.resources ?? []),
  };
};

/**
 * `plan.lights`: the lights at this frame, world position, linear colour, the intensity already
 * multiplied by its flicker, and the radius (the distance at which a light has fallen to a quarter).
 * Returns the visible ones, in plan order.
 */
const parsePointLights = (raw, requires) => {
  const items = arrayAt(raw ?? [], '/lights');
  if (items.length > MAX_POINT_LIGHTS) throw invalid('/lights', `expected at most ${MAX_POINT_LIGHTS} lights`);
  const seen = new Set();
  const lights = items.map((item, index) => {
    const at = `/lights/${index}`;
    const light = objectAt(item, at);
    const id = stringAt(light.id, `${at}/id`);
    if (seen.has(id)) throw invalid(`${at}/id`, `light \`${id}\` is listed twice`);
    seen.add(id);
    const color = objectAt(light.color, `${at}/color`);
    const channels = ['r', 'g', 'b'].map((channel) => {
      const value = finiteAt(color[channel], `${at}/color/${channel}`);
      if (value < 0 || value > 1) throw invalid(`${at}/color/${channel}`, 'expected a number between 0 and 1');
      return value;
    });
    const intensity = finiteAt(light.intensity, `${at}/intensity`);
    if (intensity < 0 || intensity > MAX_LIGHT_INTENSITY) throw invalid(`${at}/intensity`, `expected a number between 0 and ${MAX_LIGHT_INTENSITY}`);
    if (light.hidden !== undefined && typeof light.hidden !== 'boolean') throw invalid(`${at}/hidden`, 'expected true or false');
    return {
      id,
      position: vec3At(light.position, `${at}/position`),
      color: channels,
      intensity,
      radius: positiveAt(light.radius, `${at}/radius`),
      hidden: light.hidden === true,
    };
  });
  const visible = items.length === 0 ? [] : lights.filter((light) => !light.hidden);
  if (visible.length > 0 && !requires.includes('pointLights')) throw invalid('/requires', 'a plan with visible lights must require `pointLights`');
  return visible;
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
    if (resource.format === 'spz') {
      // A splat has no declared size to compare: it must be a splat the host loaded as spz.
      if (entry.kind !== 'splat' || !entry.fileName?.endsWith('.spz')) {
        throw new RuntimeError('resource_mismatch', `\`${assetId}\` is not an spz splat file (${entry.kind}), the plan draws it as splats`, {
          asset_id: assetId,
          expected: { kind: 'splat', format: 'spz' },
          got: { kind: entry.kind, file: entry.fileName ?? null },
        });
      }
      bound.set(assetId, { slot: reference.slot, entry });
      continue;
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

// ---- point lights (F060) -----------------------------------------------------------------------
//
// What a light does to an image plane, in one place. The renderer has colour management off and
// the pictures are sRGB bytes drawn as they are, so the light is applied in LINEAR space: decode
// the plane's colour, multiply it by `1 + L`, encode it back. `L` is the sum over the lights of
// `colour * intensity * max(0, n . l) * falloff(d)^2`, with `falloff = 1 / (1 + (d / radius)^2)`
// applied twice (so a light is a quarter as strong at its radius) and `n` the plane's normal facing
// the camera (a double-sided plane is lit on the side it is seen from). Without a light the
// factor is exactly 1: the branch is skipped, the picture is the unlit one bit for bit.
// The reference the tests compare with is written independently in `directed-scene-light.test.mjs`.

const LIGHT_VERTEX_DECLARATIONS = `
varying vec3 vTesselWorld;
varying vec3 vTesselNormal;
`;

const LIGHT_VERTEX_BODY = `
  vTesselWorld = (modelMatrix * vec4(transformed, 1.0)).xyz;
  vTesselNormal = transpose(inverse(mat3(modelMatrix))) * normal;
`;

const LIGHT_FRAGMENT_DECLARATIONS = `
varying vec3 vTesselWorld;
varying vec3 vTesselNormal;
uniform int uTesselLightCount;
uniform vec3 uTesselLightPosition[${MAX_POINT_LIGHTS}];
uniform vec3 uTesselLightColor[${MAX_POINT_LIGHTS}];
uniform float uTesselLightRadius[${MAX_POINT_LIGHTS}];
vec3 tesselDecode(vec3 c) {
  return mix(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, vec3(lessThanEqual(c, vec3(0.04045))));
}
vec3 tesselEncode(vec3 c) {
  return mix(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, vec3(lessThanEqual(c, vec3(0.0031308))));
}
`;

const LIGHT_FRAGMENT_BODY = `
  if (uTesselLightCount > 0) {
    vec3 tesselNormal = normalize(vTesselNormal);
    if (dot(tesselNormal, cameraPosition - vTesselWorld) < 0.0) tesselNormal = -tesselNormal;
    vec3 tesselLit = vec3(0.0);
    for (int i = 0; i < ${MAX_POINT_LIGHTS}; i++) {
      if (i >= uTesselLightCount) break;
      vec3 toLight = uTesselLightPosition[i] - vTesselWorld;
      float distanceToLight = length(toLight);
      if (distanceToLight < 1e-9) continue;
      float once = 1.0 / (1.0 + (distanceToLight * distanceToLight) / (uTesselLightRadius[i] * uTesselLightRadius[i]));
      float facing = max(0.0, dot(tesselNormal, toLight / distanceToLight));
      tesselLit += uTesselLightColor[i] * (facing * once * once);
    }
    outgoingLight = tesselEncode(clamp(tesselDecode(outgoingLight) * (vec3(1.0) + tesselLit), 0.0, 1.0));
  }
`;

/** The uniforms a lit material shares with its compiled program: updated per frame, never recompiled. */
const lightUniforms = () => ({
  uTesselLightCount: { value: 0 },
  uTesselLightPosition: { value: Array.from({ length: MAX_POINT_LIGHTS }, () => new THREE.Vector3()) },
  uTesselLightColor: { value: Array.from({ length: MAX_POINT_LIGHTS }, () => new THREE.Vector3()) },
  uTesselLightRadius: { value: new Array(MAX_POINT_LIGHTS).fill(1) },
});

const LIGHT_CACHE_KEY = 'tessel-point-lights-v1';

/** Makes a layer material light-aware: one program, with the lights as uniforms (count 0 is the unlit picture). */
const makeLightAware = (material) => {
  const uniforms = lightUniforms();
  material.userData.tesselLights = uniforms;
  material.onBeforeCompile = (shader) => {
    Object.assign(shader.uniforms, uniforms);
    shader.vertexShader = shader.vertexShader
      .replace('#include <common>', `#include <common>\n${LIGHT_VERTEX_DECLARATIONS}`)
      .replace('#include <begin_vertex>', `#include <begin_vertex>\n${LIGHT_VERTEX_BODY}`);
    shader.fragmentShader = shader.fragmentShader
      .replace('#include <common>', `#include <common>\n${LIGHT_FRAGMENT_DECLARATIONS}`)
      .replace('#include <opaque_fragment>', `${LIGHT_FRAGMENT_BODY}\n#include <opaque_fragment>`);
  };
  material.customProgramCacheKey = () => LIGHT_CACHE_KEY;
  return material;
};

/**
 * Sets the lights a material draws this frame (the visible ones of `parseDirectedLayer`). A retained
 * material is the same object across frames, so this runs for every plane of every frame; it only
 * writes numbers.
 */
export const applyPointLights = (material, lights) => {
  const uniforms = material.userData.tesselLights;
  if (!uniforms) return;
  uniforms.uTesselLightCount.value = lights.length;
  lights.forEach((light, index) => {
    uniforms.uTesselLightPosition.value[index].set(...light.position);
    // The intensity multiplies the colour here, so the shader needs one vector per light.
    uniforms.uTesselLightColor.value[index].set(
      light.color[0] * light.intensity,
      light.color[1] * light.intensity,
      light.color[2] * light.intensity,
    );
    uniforms.uTesselLightRadius.value[index] = light.radius;
  });
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
  if (alpha === 'straight') return makeLightAware(new THREE.MeshBasicMaterial({ ...common, blending: THREE.NormalBlending }));
  return makeLightAware(new THREE.MeshBasicMaterial({
    ...common,
    blending: THREE.CustomBlending,
    blendEquation: THREE.AddEquation,
    blendSrc: THREE.OneFactor,
    blendDst: THREE.ZeroFactor,
    blendEquationAlpha: THREE.AddEquation,
    blendSrcAlpha: THREE.ZeroFactor,
    blendDstAlpha: THREE.OneFactor,
  }));
};

/**
 * The three.js scene and camera of the plan. `textureOf(assetId)` gives the texture of a layer's
 * picture, `materialOf(alpha, index)` its material (the compositor retains these between frames),
 * `keep` registers what must be disposed when the frame ends.
 */
export const buildDirectedScene = (spec, { textureOf, materialOf, keep }) => {
  const scene = new THREE.Scene();
  spec.planes.forEach((plane, index) => {
    const material = materialOf(plane.alpha, index, textureOf(plane.assetId));
    // The lights of this frame (F060). A retained material carries the previous frame's: always set.
    applyPointLights(material, spec.lights);
    const mesh = new THREE.Mesh(keep(planeGeometry(plane.width, plane.height)), material);
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

/**
 * The scenes of the splats, one per slot among the planes (`meshes[i]` is the Spark mesh of
 * `spec.splats[i]`), each mesh placed by the node's world matrix (the core's: ancestors, sockets and
 * performance channels already applied) and seen by the same camera as the planes. Ascending by
 * `after`; empty when the plan has no splat. The meshes are lent by the asset store:
 * `releaseSplatMesh` gives the transform state back before they are returned.
 */
export const buildSplatScenes = (spec, meshes) => {
  const slots = new Map();
  spec.splats.forEach((splat, index) => {
    const mesh = meshes[index];
    mesh.matrixAutoUpdate = false;
    mesh.matrix.fromArray(splat.world);
    mesh.matrixWorldNeedsUpdate = true;
    if (!slots.has(splat.after)) slots.set(splat.after, new THREE.Scene());
    slots.get(splat.after).add(mesh);
  });
  return [...slots.entries()]
    .sort(([left], [right]) => left - right)
    .map(([after, scene]) => {
      scene.updateMatrixWorld(true);
      return { after, scene };
    });
};

/** Undoes `buildSplatScenes` on a lent mesh, so the next borrower finds the transform state it expects. */
export const releaseSplatMesh = (mesh) => {
  mesh.removeFromParent();
  mesh.matrixAutoUpdate = true;
  mesh.matrix.identity();
  mesh.position.set(0, 0, 0);
  mesh.quaternion.identity();
  mesh.scale.set(1, 1, 1);
};

/**
 * Draws into `target`: opaque black first (alpha 1), then the plan's painter's order: the planes
 * in plan order with each slot of splats (Spark, sorted for exactly this camera) painted right
 * after the `after` planes the core said come before it. Several passes and not one scene: the
 * plan decides what is in front of what, and that must not depend on how Spark orders its own
 * mesh or on a depth buffer. A plan without splats is one pass over every plane, as it always was.
 * `stage` is the compositor's SplatStage; `aborted` stops the wait for the sort. A frame that
 * cannot settle its sort is refused, never drawn from another viewpoint.
 */
export const drawDirectedScene = async (renderer, target, { scene, camera, splatScenes = [] }, { stage, aborted }) => {
  const planes = scene.children;
  const paint = (from, to) => {
    if (to <= from) return;
    planes.forEach((plane, index) => {
      plane.visible = index >= from && index < to;
    });
    renderer.render(scene, camera);
  };
  try {
    renderer.setRenderTarget(target);
    renderer.setClearColor(0x000000, 1);
    renderer.clear(true, true, true);
    let painted = 0;
    for (const group of splatScenes) {
      paint(painted, group.after);
      painted = Math.max(painted, group.after);
      stage.attach(group.scene);
      try {
        await stage.settle(group.scene, camera, aborted);
        renderer.render(group.scene, camera);
      } finally {
        stage.detach(group.scene);
      }
    }
    paint(painted, planes.length);
  } finally {
    planes.forEach((plane) => {
      plane.visible = true;
    });
    renderer.setRenderTarget(null);
  }
};
