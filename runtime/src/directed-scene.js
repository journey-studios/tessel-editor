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

import {
  MAX_POINT_LIGHTS,
  arrayAt,
  integerAt,
  invalid,
  isObject,
  objectAt,
  parseResourceReferences,
  readDeckBody,
  stringAt,
} from './directed-deck.js';
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

// The deck reader moved to directed-deck.js (P4b S2-R0) and these names are still read from here.
export { MAX_POINT_LIGHTS, SPLAT_SIMILARITY_TOLERANCE, splatMatrixFault } from './directed-deck.js';

// The camera planes of the origin's runtime (ThreeSceneRuntime: PerspectiveCamera(50, aspect, 0.1, 1000)).
const NEAR = 0.1;
const FAR = 1000;

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

  // The shape of the deck, from the output to the lights: the reader shared with the neighbour deck of a
  // transition (`readDeckBody`). The host's references come last, as they always did.
  const deck = readDeckBody(plan, requires);

  return { ...deck, references: parseResourceReferences(layer.resources ?? []) };
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
