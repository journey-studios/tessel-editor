// Fixtures of the transition-plan reader tests (F064, P4b S2-R0). Not a test file (no `.test.` in the name).
//
// Every number below is written BY HAND with its arithmetic in a comment; nothing is computed by the code
// under test and no helper here knows a clock rule. The wire is the core's (`ScenePlan::to_json`, C0b:
// crates/core/src/scene_plan/{json,transition_json}.rs and temporal/json.rs). That the core really writes
// these shapes is proved the other way round by crates/render/tests/scene_transition_parser.rs, which hands
// plans the core built to this same reader.
//
// The edit has 100 frames at 24 fps and two shots joined by a cut at frame 40:
//   shot-a [0, 40)   instance inst-a (60 frames) entered at 10, camera clip cam-a (40 frames) entered at 5
//   shot-b [40, 100) instance inst-b (200 frames) entered at 0,  camera clip cam-b (60 frames) entered at 0
// Both scenes have nodes called `backdrop` and `figure` (the SAME ids, valid: ids are per deck), and share
// the asset `hero`; each has its own wall.
import { node, resource } from './directed-scene-fixtures.mjs';

export const ratio = (numerator, denominator) => ({ numerator, denominator });

const SHOTS = {
  a: { shot: 'shot-a', scene: 'scene-a', instance: 'inst-a', camera: 'cam-a', wall: 'wall-a' },
  b: { shot: 'shot-b', scene: 'scene-b', instance: 'inst-b', camera: 'cam-b', wall: 'wall-b' },
};

const A = { start: 0, end: 40, iStart: 10, iDur: 60, cIn: 5, cDur: 40 };
const B = { start: 40, end: 100, iStart: 0, iDur: 200, cIn: 0, cDur: 60 };

/**
 * The scenarios. `A`/`B` are the shots' timing (`sI`/`sC`: the instance and camera clock held to its clip);
 * `top` is the PRIMARY deck's legacy frame counters (RAW) and `oth` the neighbour's (the held samples);
 * `held` is [heldInstance, heldCamera] of the neighbour; `times` are the exact times of every counter.
 */
export const SCENARIOS = {
  // f = 36, window [34, 46) (center, D 12, cut 40), frameInWindow 2, primary shot-a.
  // shot-a: instance 10 + 36 = 46 (< 60); camera 5 + 36 = 41 past its 40 frames: sample 39, held (raw 41 / sample 39).
  // shot-b: delta 36 - 40 = -4: instance -4 -> 0 held, camera -4 -> 0 held.
  // easeInOutCubic at t = 2.5 / 12 = 5/24: 4 t^3 = 4 * 125 / 13824 = 125/3456.
  base: {
    fps: [24, 1], edit: 100, f: 36, primary: 'from', D: 12, alignment: 'center', startFrame: 34, inWindow: 2,
    easing: 'easeInOutCubic', curve: 'smooth', intensity: 1, progress: 125 / 3456, amount: 1,
    A: { ...A, sI: 46, sC: 39 }, B: { ...B, sI: 0, sC: 0 },
    top: { i: 46, c: 41 }, oth: { i: 0, c: 0 }, held: [true, true],
    // 36/24 = 3/2; 46/24 = 23/12; 41/24; 0.
    times: { edit: [3, 2], top: { i: [23, 12], c: [41, 24] }, oth: { i: [0, 1], c: [0, 1] } },
  },
  // f = 42, frameInWindow 8, primary shot-b (the roles swap at the cut), intensity 1/2 (amount 1/2, not 1/2 * progress).
  // shot-b: delta 2: instance 2, camera 2, both inside. shot-a (neighbour): delta 42: instance 10 + 42 = 52 (< 60, NOT
  // held), camera 5 + 42 = 47 -> 39 held. easeInOutCubic at t = 8.5 / 12 = 17/24: p = 2 - 34/24 = 7/12;
  // 1 - p^3 / 2 = 1 - 343/3456 = 3113/3456.
  afterCut: {
    fps: [24, 1], edit: 100, f: 42, primary: 'to', D: 12, alignment: 'center', startFrame: 34, inWindow: 8,
    easing: 'easeInOutCubic', curve: 'linear', intensity: 0.5, progress: 3113 / 3456, amount: 0.5,
    A: { ...A, sI: 52, sC: 39 }, B: { ...B, sI: 2, sC: 2 },
    top: { i: 2, c: 2 }, oth: { i: 52, c: 39 }, held: [false, true],
    // 42/24 = 7/4; 2/24 = 1/12; 52/24 = 13/6; 39/24 = 13/8.
    times: { edit: [7, 4], top: { i: [1, 12], c: [1, 12] }, oth: { i: [13, 6], c: [13, 8] } },
  },
  // f = 39 = C - 1: still shot-a. frameInWindow 5, linear: 5.5 / 12 = 11/24.
  // shot-a: instance 10 + 39 = 49, camera 5 + 39 = 44 -> 39 held. shot-b: delta -1 -> both 0, held.
  beforeCut: {
    fps: [24, 1], edit: 100, f: 39, primary: 'from', D: 12, alignment: 'center', startFrame: 34, inWindow: 5,
    easing: 'linear', curve: 'smooth', intensity: 1, progress: 11 / 24, amount: 1,
    A: { ...A, sI: 49, sC: 39 }, B: { ...B, sI: 0, sC: 0 },
    top: { i: 49, c: 44 }, oth: { i: 0, c: 0 }, held: [true, true],
    // 39/24 = 13/8; 49/24; 44/24 = 11/6; 0.
    times: { edit: [13, 8], top: { i: [49, 24], c: [11, 6] }, oth: { i: [0, 1], c: [0, 1] } },
  },
  // f = 40 = C: shot-b is the primary, frameInWindow 6, linear: 6.5 / 12 = 13/24.
  // shot-b: delta 0 -> instance 0, camera 0 (the first real frames, not held). shot-a: delta 40: instance 50 (< 60),
  // camera 5 + 40 = 45 -> 39 held.
  atCut: {
    fps: [24, 1], edit: 100, f: 40, primary: 'to', D: 12, alignment: 'center', startFrame: 34, inWindow: 6,
    easing: 'linear', curve: 'smooth', intensity: 1, progress: 13 / 24, amount: 1,
    A: { ...A, sI: 50, sC: 39 }, B: { ...B, sI: 0, sC: 0 },
    top: { i: 0, c: 0 }, oth: { i: 50, c: 39 }, held: [false, true],
    // 40/24 = 5/3; 0; 50/24 = 25/12; 39/24 = 13/8.
    times: { edit: [5, 3], top: { i: [0, 1], c: [0, 1] }, oth: { i: [25, 12], c: [13, 8] } },
  },
  // Odd duration, center: D = 5: start = 40 - floor(5/2) = 38, end 43 (2 frames before the cut, 3 from it).
  // f = 40: frameInWindow 2; easeOutCubic at t = 2.5 / 5 = 1/2: 1 - (1/2)^3 = 7/8. Clocks as atCut.
  oddCenter: {
    fps: [24, 1], edit: 100, f: 40, primary: 'to', D: 5, alignment: 'center', startFrame: 38, inWindow: 2,
    easing: 'easeOutCubic', curve: 'smooth', intensity: 1, progress: 7 / 8, amount: 1,
    A: { ...A, sI: 50, sC: 39 }, B: { ...B, sI: 0, sC: 0 },
    top: { i: 0, c: 0 }, oth: { i: 50, c: 39 }, held: [false, true],
    times: { edit: [5, 3], top: { i: [0, 1], c: [0, 1] }, oth: { i: [25, 12], c: [13, 8] } },
  },
  // `start`: [C, C + D) = [40, 52). f = 40: frameInWindow 0, linear: 0.5 / 12 = 1/24. Clocks as atCut.
  startAligned: {
    fps: [24, 1], edit: 100, f: 40, primary: 'to', D: 12, alignment: 'start', startFrame: 40, inWindow: 0,
    easing: 'linear', curve: 'smooth', intensity: 1, progress: 1 / 24, amount: 1,
    A: { ...A, sI: 50, sC: 39 }, B: { ...B, sI: 0, sC: 0 },
    top: { i: 0, c: 0 }, oth: { i: 50, c: 39 }, held: [false, true],
    times: { edit: [5, 3], top: { i: [0, 1], c: [0, 1] }, oth: { i: [25, 12], c: [13, 8] } },
  },
  // `end`: [C - D, C) = [28, 40). f = 39 is its last frame: frameInWindow 11, linear: 11.5 / 12 = 23/24. Clocks as beforeCut.
  endAligned: {
    fps: [24, 1], edit: 100, f: 39, primary: 'from', D: 12, alignment: 'end', startFrame: 28, inWindow: 11,
    easing: 'linear', curve: 'smooth', intensity: 1, progress: 23 / 24, amount: 1,
    A: { ...A, sI: 49, sC: 39 }, B: { ...B, sI: 0, sC: 0 },
    top: { i: 49, c: 44 }, oth: { i: 0, c: 0 }, held: [true, true],
    times: { edit: [13, 8], top: { i: [49, 24], c: [11, 6] }, oth: { i: [0, 1], c: [0, 1] } },
  },
  // A one-frame window: center, D 1: start = 40 - floor(1/2) = 40. frameInWindow 0, easeInOutCubic at t = 1/2:
  // p = 2 - 1 = 1, 1 - 1/2 = 1/2. Clocks as atCut.
  oneFrame: {
    fps: [24, 1], edit: 100, f: 40, primary: 'to', D: 1, alignment: 'center', startFrame: 40, inWindow: 0,
    easing: 'easeInOutCubic', curve: 'smooth', intensity: 1, progress: 0.5, amount: 1,
    A: { ...A, sI: 50, sC: 39 }, B: { ...B, sI: 0, sC: 0 },
    top: { i: 0, c: 0 }, oth: { i: 50, c: 39 }, held: [false, true],
    times: { edit: [5, 3], top: { i: [0, 1], c: [0, 1] }, oth: { i: [25, 12], c: [13, 8] } },
  },
  // The base frame at 30000/1001 fps: time(n) = n * 1001 / 30000, reduced.
  // 36 -> 36036/30000 = 3003/2500; 46 -> 46046/30000 = 23023/15000; 41 -> 41041/30000 (no common factor); 0 -> 0/1.
  ntsc: {
    fps: [30000, 1001], edit: 100, f: 36, primary: 'from', D: 12, alignment: 'center', startFrame: 34, inWindow: 2,
    easing: 'easeInOutCubic', curve: 'smooth', intensity: 1, progress: 125 / 3456, amount: 1,
    A: { ...A, sI: 46, sC: 39 }, B: { ...B, sI: 0, sC: 0 },
    top: { i: 46, c: 41 }, oth: { i: 0, c: 0 }, held: [true, true],
    times: { edit: [3003, 2500], top: { i: [23023, 15000], c: [41041, 30000] }, oth: { i: [0, 1], c: [0, 1] } },
  },
  // f = 39 as beforeCut, but shot-b's camera clip has NO frames (length 0) and is entered at 1: raw 1 - 1 = 0, which is
  // the sample 0, and still held: there is no real frame to be on. (With 60 frames the same raw 0 is NOT held: see
  // `neighbourCameraInside` in the tests.)
  emptyCamera: {
    fps: [24, 1], edit: 100, f: 39, primary: 'from', D: 12, alignment: 'center', startFrame: 34, inWindow: 5,
    easing: 'linear', curve: 'smooth', intensity: 1, progress: 11 / 24, amount: 1,
    A: { ...A, sI: 49, sC: 39 }, B: { ...B, cIn: 1, cDur: 0, sI: 0, sC: 0 },
    top: { i: 49, c: 44 }, oth: { i: 0, c: 0 }, held: [true, true],
    times: { edit: [13, 8], top: { i: [49, 24], c: [11, 6] }, oth: { i: [0, 1], c: [0, 1] } },
  },
  // f = 40 as atCut, but shot-a's camera has 46 frames: the neighbour's camera raw is 5 + 40 = 45 = 46 - 1, the LAST
  // real frame, so it is not held (sample 45, heldCamera false). Its time is 45/24 = 15/8.
  lastRealFrame: {
    fps: [24, 1], edit: 100, f: 40, primary: 'to', D: 12, alignment: 'center', startFrame: 34, inWindow: 6,
    easing: 'linear', curve: 'smooth', intensity: 1, progress: 13 / 24, amount: 1,
    A: { ...A, cDur: 46, sI: 50, sC: 45 }, B: { ...B, sI: 0, sC: 0 },
    top: { i: 0, c: 0 }, oth: { i: 50, c: 45 }, held: [false, false],
    times: { edit: [5, 3], top: { i: [0, 1], c: [0, 1] }, oth: { i: [25, 12], c: [15, 8] } },
  },
};

/** One deck: nodes `backdrop` (its wall) and `figure` (the shared `hero`), painted back to front. */
const deck = (key, scenario, role) => {
  const ids = SHOTS[key];
  const own = role === 'primary' ? scenario.top : scenario.oth;
  const time = role === 'primary' ? scenario.times.top : scenario.times.oth;
  const file = (id, width, height, path) =>
    resource(id, width, height, { format: 'png', relativePath: path, alpha: 'straight', colorSpace: 'srgb', usedBy: [] });
  return {
    kind: 'tessel-scene-plan',
    planVersion: 1,
    complete: true,
    requires: ['camera', 'nodeTransforms', 'imagePlanes', 'straightAlpha', 'opaqueAlpha'],
    source: { bundleId: 'bundle', revisionId: 'rev-1', sceneDefinitionId: ids.scene, sceneInstanceId: ids.instance },
    output: { width: 160, height: 90, fps: { numerator: scenario.fps[0], denominator: scenario.fps[1] }, colorSpace: 'srgb-display', alphaMode: 'straight' },
    frame: {
      editFrame: scenario.f,
      editTime: ratio(...scenario.times.edit),
      shotId: ids.shot,
      instanceFrame: own.i,
      instanceTime: ratio(...time.i),
      cameraClipId: ids.camera,
      cameraClipFrame: own.c,
      cameraClipTime: ratio(...time.c),
    },
    camera: {
      position: { x: 0, y: 0, z: 10 }, target: { x: 0, y: 0, z: 0 }, fovDeg: 45,
      velocity: { x: 0, y: 0, z: 0 }, fovVelocityDegPerFrame: 0,
    },
    nodes: [
      node('backdrop', ids.wall, 0, 0, -2, 6, 3),
      node('figure', 'hero', 0.5, 0, 0, 2, 2),
    ],
    layers: [
      { nodeId: 'backdrop', assetId: ids.wall, alpha: 'opaque', renderOrder: 0, passOrder: null },
      { nodeId: 'figure', assetId: 'hero', alpha: 'straight', renderOrder: 1, passOrder: null },
    ],
    resources: [file('hero', 8, 8, 'assets/hero.png'), file(ids.wall, 16, 9, `assets/${ids.wall}.png`)],
    unsupported: [],
    uninterpreted: [],
  };
};

/** The wire of a plan with a transition, from a scenario (the keys the core writes, in its order). */
export const buildPlan = (scenario) => {
  const topKey = scenario.primary === 'from' ? 'a' : 'b';
  const otherKey = topKey === 'a' ? 'b' : 'a';
  const top = deck(topKey, scenario, 'primary');
  const other = deck(otherKey, scenario, 'other');
  const side = (shot) => ({
    shotStartFrame: shot.start,
    shotEndFrameExclusive: shot.end,
    instanceStartFrame: shot.iStart,
    instanceDurationFrames: shot.iDur,
    cameraInFrame: shot.cIn,
    cameraClipDurationFrames: shot.cDur,
    sampledInstanceFrame: shot.sI,
    sampledCameraClipFrame: shot.sC,
  });
  top.requires = [...top.requires, 'fxTransition'];
  top.transition = {
    id: 'tr',
    effect: 'crossDissolve',
    fromShotId: 'shot-a',
    toShotId: 'shot-b',
    primary: scenario.primary,
    startFrame: scenario.startFrame,
    durationFrames: scenario.D,
    alignment: scenario.alignment,
    timing: { editDurationFrames: scenario.edit, from: side(scenario.A), to: side(scenario.B) },
    frameInWindow: scenario.inWindow,
    easing: scenario.easing,
    progress: scenario.progress,
    amount: scenario.amount,
    params: [['curve', scenario.curve], ['intensity', scenario.intensity]],
    heldInstance: scenario.held[0],
    heldCamera: scenario.held[1],
    other,
  };
  return top;
};

export const basePlan = () => buildPlan(SCENARIOS.base);

/** A deep copy, so a test can break one field without touching the next test's plan. */
export const clone = (value) => structuredClone(value);

/** Sets (or, for `undefined`, deletes) the value at a JSON pointer of `plan`, and returns the plan. */
export const edit = (plan, pointer, value) => {
  const keys = pointer.split('/').slice(1);
  let parent = plan;
  for (const key of keys.slice(0, -1)) parent = parent[key];
  const last = keys[keys.length - 1];
  if (value === undefined) {
    if (Array.isArray(parent)) parent.splice(Number(last), 1);
    else delete parent[last];
  } else {
    parent[last] = value;
  }
  return plan;
};

/** A plan with every pointer of `changes` applied, from a fresh base. */
export const brokenBase = (changes) => {
  const plan = basePlan();
  for (const [pointer, value] of Object.entries(changes)) edit(plan, pointer, value);
  return plan;
};

/** Freezes a value and everything under it: a reader that writes into its input throws. */
export const deepFreeze = (value) => {
  if (value !== null && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const key of Reflect.ownKeys(value)) deepFreeze(value[key]);
  }
  return value;
};

/** A fade pass of the deck, in the core's wire form (`FxPass::to_json`). */
export const fadePass = (clipId, scope) => ({
  trackId: `track-${clipId}`,
  clipId,
  effect: 'fade',
  frameInClip: 0,
  weight: 1,
  amount: 0.5,
  params: { color: { r: 0, g: 0, b: 0 }, amount: 0.5, intensity: 1 },
  scope,
});

export const OUTPUT_SCOPE = { kind: 'output' };
export const shotScope = (shotId) => ({ kind: 'shot', shotId });
