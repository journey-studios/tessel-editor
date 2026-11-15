// The reader of ONE deck of a directed-scene plan (what `parseDirectedLayer` has always checked
// before it draws), kept apart from the negotiation of capabilities so that a second deck can be
// read by the very same code (F064, P4b S2-R0). Contract: docs/features/scene-renderer.md.
//
// WHY this module exists: a plan with a transition carries two decks, the primary one and the
// neighbour under `transition.other`; both are plans like any other and must be judged by the same
// rules, not by a copy of them. `readDeckBody` is the part of the old `parseDirectedLayer` that
// starts at `/output`: it needs the plan's `requires` already negotiated and answers what the
// drawing needs. The checks are pure (no GL, no DOM), so they run under `node --test`.

import { parseOutputFx } from './output-fx.js';
import { RuntimeError } from './protocol.js';

export const SCENE_PLAN_KIND = 'tessel-scene-plan';
export const SCENE_PLAN_VERSION = 1;

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

export const isObject = (value) => typeof value === 'object' && value !== null && !Array.isArray(value);

export const invalid = (pointer, message) =>
  new RuntimeError('invalid_plan', `Scene plan ${pointer || '/'}: ${message}`, { pointer });

export const objectAt = (value, pointer) => {
  if (!isObject(value)) throw invalid(pointer, 'expected an object');
  return value;
};

export const arrayAt = (value, pointer) => {
  if (!Array.isArray(value)) throw invalid(pointer, 'expected an array');
  return value;
};

export const stringAt = (value, pointer) => {
  if (typeof value !== 'string' || value === '') throw invalid(pointer, 'expected a non-empty string');
  return value;
};

export const finiteAt = (value, pointer) => {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw invalid(pointer, 'expected a finite number');
  return value;
};

export const positiveAt = (value, pointer) => {
  if (finiteAt(value, pointer) <= 0) throw invalid(pointer, 'expected a positive number');
  return value;
};

// The core writes a vector as `{ x, y, z }` (`Vec3::to_json`); the matrices are the only arrays.
export const vec3At = (value, pointer) => {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) throw invalid(pointer, 'expected an object {x, y, z}');
  return ['x', 'y', 'z'].map((axis) => finiteAt(value[axis], `${pointer}/${axis}`));
};

const matrixAt = (value, pointer) => {
  const items = arrayAt(value, pointer);
  if (items.length !== 16) throw invalid(pointer, 'expected 16 numbers (column-major)');
  return items.map((item, index) => finiteAt(item, `${pointer}/${index}`));
};

export const integerAt = (value, pointer) => {
  if (!Number.isSafeInteger(value) || value < 0) throw invalid(pointer, 'expected a non-negative integer');
  return value;
};

/**
 * The kind and the version of a plan, before any other field: a plan of a later version may have other
 * fields, and must be refused as such, not as a malformed one (`ScenePlan::from_json`).
 */
export const assertPlanKind = (plan) => {
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
};

/** A partial plan is the scene without its light, weather or effects: not the picture. */
export const assertComplete = (plan) => {
  const unsupported = arrayAt(plan.unsupported, '/unsupported');
  if (plan.complete !== true || unsupported.length > 0) {
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
};

/**
 * The plan's `requires`, each one a non-empty string, and refused (`unsupported` + `details.missing`)
 * when it names a capability that is not in `capabilities`. The caller chooses the list: the
 * negotiation is not the deck reader's business.
 */
export const readRequires = (plan, capabilities) => {
  const requires = arrayAt(plan.requires, '/requires').map((item, index) => stringAt(item, `/requires/${index}`));
  const missing = requires.filter((capability) => !capabilities.includes(capability));
  if (missing.length > 0) {
    throw new RuntimeError('unsupported', `The plan requires capabilities this runtime does not have: ${missing.join(', ')}`, {
      missing,
    });
  }
  return requires;
};

/**
 * What one deck of a plan asks the drawing for: output size, camera, the planes in plan order, the
 * splats, the effect passes, the visible point lights and the resources. `requires` is the plan's
 * own list, already negotiated by the caller (a plan with splats must ask for `splats`, and so on).
 *
 * Order of refusals follows the core (`ScenePlan::from_json`): the shape of what is drawn, field by
 * field, in the order the old `parseDirectedLayer` read it.
 */
export const readDeckBody = (plan, requires) => {
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
export const parseResourceReferences = (raw) => {
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
