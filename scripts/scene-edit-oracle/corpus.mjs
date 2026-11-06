// Corpus do oráculo de edição (S6 / BL004–BL005). Entradas SINTÉTICAS escritas do zero a partir do
// schema da origem (sem asset, sem dado privado). Cada caso é um bundle de um clipe de câmera e
// uma lista de atribuições por caminho, exatamente o que `setProperties` da origem recebe.
// Este arquivo só descreve ENTRADAS e a intenção de cada caso; o resultado esperado é sempre o que
// a origem responde (ver oracle.mjs), nunca uma regra reimplementada aqui.

const START = { position: { x: 2, y: 1, z: 9 }, target: { x: 0, y: 1, z: 0 }, fovDeg: 50 }
const MOVED = { x: 3.5, y: 1.5, z: 8 }
const CLIP = "cam"

const vec = (x, y, z) => ({ x, y, z })

const identity = { x: 0, y: 0, z: 0, w: 1 }
const unit = vec(1, 1, 1)

const node = (id, parentId, extra) => ({
  id,
  parentId,
  localPosition: vec(0, 0, 0),
  localRotation: identity,
  localScale: unit,
  ...extra,
})

const asset = (assetId) => ({
  assetId,
  revisionId: "rev-1",
  contentHash: `synthetic-sha256-${assetId}`,
  relativePath: `assets/${assetId}.png`,
  widthPx: 32,
  heightPx: 32,
  alphaMode: "straight",
  colorSpace: "srgb",
})

const plane = (id, order, width, height) =>
  node(id, "root", {
    type: "imagePlane",
    assetRef: {
      assetId: id,
      revisionId: "rev-1",
      contentHash: `synthetic-sha256-${id}`,
      relativePath: `assets/${id}.png`,
    },
    widthSceneUnits: width,
    heightSceneUnits: height,
    renderOrder: order,
  })

const segmentFrames = (segments) =>
  segments.reduce((sum, segment) => sum + segment.durationFrames, 0)

/** A valid one-shot bundle whose shot plays exactly the frames of `clip`. */
export const bundleWith = (clip) => {
  const frames = Math.max(segmentFrames(clip.segments), ...(clip.layers ?? []).map((layer) => segmentFrames(layer.segments)))
  const fps = { numerator: 24, denominator: 1 }
  return {
    schemaVersion: "1.0",
    bundleId: "oracle-scene-edit",
    revisionId: "rev-1",
    fps,
    editDurationFrames: frames,
    renderProfile: {
      width: 320,
      height: 180,
      fps,
      colorSpace: "srgb-display",
      alphaMode: "straight",
      premultiplyOnUpload: false,
      videoCodec: "h264-yuv420p",
      threeRevision: "0.187.0",
      browserChannel: "synthetic-oracle",
    },
    assets: [asset("back"), asset("front")],
    performanceTracks: [],
    compositionPasses: [],
    sceneDefinitions: [
      {
        id: "scene-a",
        schemaVersion: "1.0",
        rootNodeId: "root",
        nodes: [
          node("root", null, { type: "group" }),
          plane("back", 0, 8, 4.5),
          plane("front", 1, 2, 2),
        ],
      },
    ],
    sceneInstances: [
      { id: "inst-a", sceneDefinitionId: "scene-a", durationFrames: frames, performanceTrackIds: [] },
    ],
    cameraClips: [clip],
    shots: [
      {
        id: "shot-only",
        sceneInstanceId: "inst-a",
        editRange: { startFrame: 0, endFrameExclusive: frames },
        instanceStartFrame: 0,
        cameraClipId: clip.id,
        cameraInFrame: 0,
        intent: "synthetic shot",
      },
    ],
  }
}

const clipOf = (segments, layers) => ({
  id: CLIP,
  initialPosition: START.position,
  initialTarget: START.target,
  initialFovDeg: START.fovDeg,
  segments,
  ...(layers ? { layers } : {}),
})

const dolly = (recipe, frames, distance, easing = "easeInOutCubic", from = "previousEnd") => ({
  recipe, durationFrames: frames, from, distanceSceneUnits: distance, easing,
})
const zoom = (recipe, frames, delta, easing = "easeInOutCubic") => ({
  recipe, durationFrames: frames, from: "previousEnd", fovDeltaDeg: delta, easing,
})
const orbit = (frames, yaw, pitch, easing = "easeInOutCubic") => ({
  recipe: "orbit", durationFrames: frames, from: "previousEnd", yawDeg: yaw, pitchDeg: pitch, easing,
})
const hold = (frames, from = "previousEnd") => ({ recipe: "hold", durationFrames: frames, from })
const keys = (frames, list) => ({ recipe: "keyframes", durationFrames: frames, from: "previousEnd", keys: list })
const key = (frame, position, target, fovDeg, easing) => ({ frame, position, target, fovDeg, easing })

/** What follows the first movement: a relative recipe and a hold (the schema refuses `from: "initial"` mid-clip). */
const tail = () => [dolly("dollyIn", 10, 1), hold(6)]

const startPosition = (value) => ({ path: `camera/${CLIP}/start/position`, value })

const withTail = (first) => clipOf([first, ...tail()])

/** @type {{ id: string, family: string, intent: string, clip: object, assignments: object[] }[]} */
export const cases = [
  { id: "first-hold", family: "convert", intent: "a hold becomes one linear key on its last frame", clip: withTail(hold(8, "initial")), assignments: [startPosition(MOVED)] },
  { id: "first-dolly-in", family: "convert", intent: "a dolly keeps its easing on one key at the end pose", clip: withTail(dolly("dollyIn", 12, 2, "easeOutCubic", "initial")), assignments: [startPosition(MOVED)] },
  { id: "first-dolly-out", family: "convert", intent: "dolly out, linear", clip: withTail(dolly("dollyOut", 12, 1.5, "linear")), assignments: [startPosition(MOVED)] },
  { id: "first-truck-left", family: "convert", intent: "truck left", clip: withTail(dolly("truckLeft", 10, 1)), assignments: [startPosition(MOVED)] },
  { id: "first-truck-right", family: "convert", intent: "truck right", clip: withTail(dolly("truckRight", 10, 1, "linear")), assignments: [startPosition(MOVED)] },
  { id: "first-zoom-in", family: "convert", intent: "zoom in changes only the field of view", clip: withTail(zoom("zoomIn", 9, 12, "easeOutCubic")), assignments: [startPosition(MOVED)] },
  { id: "first-zoom-out", family: "convert", intent: "zoom out", clip: withTail(zoom("zoomOut", 9, 8, "linear")), assignments: [startPosition(MOVED)] },
  { id: "first-orbit", family: "convert", intent: "an orbit becomes a key every two frames on the arc, all linear", clip: withTail(orbit(20, 40, 10)), assignments: [startPosition(MOVED)] },
  { id: "first-orbit-odd", family: "convert", intent: "odd duration: keys at 2 and 4, last at 6", clip: withTail(orbit(7, 30, -5, "linear")), assignments: [startPosition(MOVED)] },
  { id: "first-orbit-three", family: "convert", intent: "three frames: no intermediate key", clip: withTail(orbit(3, 30, 0)), assignments: [startPosition(MOVED)] },
  { id: "first-orbit-two", family: "convert", intent: "two frames: one key on frame 1", clip: withTail(orbit(2, 30, 0)), assignments: [startPosition(MOVED)] },
  { id: "refused-one-frame-conversion", family: "error", intent: "one frame: the converted key would land on frame 0 and the origin refuses the clip", clip: clipOf([hold(1), ...tail()]), assignments: [startPosition(MOVED)] },
  {
    id: "first-keyframes",
    family: "convert",
    intent: "a movement already made of keys is not converted; only the start moves",
    clip: clipOf([
      keys(10, [key(5, vec(1, 1, 8), vec(0, 1, 0), 48, "linear"), key(9, vec(0, 2, 7), vec(0, 1, 0), 46, "easeInOutCubic")]),
      ...tail(),
    ]),
    assignments: [startPosition(MOVED)],
  },
  {
    id: "single-movement",
    family: "convert",
    intent: "a clip with one movement",
    clip: clipOf([dolly("dollyIn", 12, 2)]),
    assignments: [startPosition(MOVED)],
  },
  {
    id: "with-layer",
    family: "convert",
    intent: "a layer on top of the main track is untouched by the conversion",
    clip: clipOf([dolly("dollyIn", 12, 2), hold(8)], [{ segments: [dolly("truckRight", 20, 0.5, "linear")] }]),
    assignments: [startPosition(MOVED)],
  },
  { id: "noop-same-start", family: "noop", intent: "writing the start it already has: the origin still converts, this editor leaves the document alone (poses equal)", clip: withTail(dolly("dollyIn", 12, 2)), assignments: [startPosition(START.position)] },
  { id: "target-only", family: "command", intent: "start/target is a plain setStart: no conversion, the path follows", clip: withTail(dolly("dollyIn", 12, 2)), assignments: [{ path: `camera/${CLIP}/start/target`, value: vec(0.5, 1.2, 0) }] },
  { id: "fov-only", family: "command", intent: "start/fov is a plain setStart", clip: withTail(zoom("zoomIn", 9, 12)), assignments: [{ path: `camera/${CLIP}/start/fov`, value: 60 }] },
  {
    id: "batch-position-then-target",
    family: "batch",
    intent: "the position write converts with the old target, then the target moves",
    clip: withTail(orbit(12, 40, 10)),
    assignments: [startPosition(MOVED), { path: `camera/${CLIP}/start/target`, value: vec(0.5, 1.2, 0) }],
  },
  {
    id: "batch-target-then-position",
    family: "batch",
    intent: "the target moves first, so the conversion sees the new target",
    clip: withTail(orbit(12, 40, 10)),
    assignments: [{ path: `camera/${CLIP}/start/target`, value: vec(0.5, 1.2, 0) }, startPosition(MOVED)],
  },
  {
    id: "batch-two-position-writes",
    family: "batch",
    intent: "the second write finds keys already: it converts nothing",
    clip: withTail(dolly("dollyOut", 12, 2)),
    assignments: [startPosition(MOVED), startPosition(vec(-1, 2, 7))],
  },
  { id: "refused-degenerate", family: "error", intent: "position on the target: the origin refuses the camera", clip: withTail(dolly("dollyIn", 12, 2)), assignments: [startPosition(vec(0, 1, 0))] },
  { id: "refused-fov", family: "error", intent: "180 degrees is not a field of view", clip: withTail(dolly("dollyIn", 12, 2)), assignments: [{ path: `camera/${CLIP}/start/fov`, value: 180 }] },
  {
    id: "refused-batch-rolls-back",
    family: "error",
    intent: "a later failure leaves nothing written",
    clip: withTail(dolly("dollyIn", 12, 2)),
    assignments: [startPosition(MOVED), { path: `camera/${CLIP}/start/fov`, value: 0 }],
  },
]

/** Frames sampled on the resulting clip: every frame, and two past its end (the hold). */
export const sampleFrames = (clip) => {
  const total = Math.max(segmentFrames(clip.segments), ...(clip.layers ?? []).map((layer) => segmentFrames(layer.segments)))
  return Array.from({ length: total + 2 }, (_, frame) => frame)
}
