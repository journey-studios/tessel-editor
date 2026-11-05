// Corpus SINTÉTICO ESTENDIDO (K / BL018, rodada 2): bundles `render-bundle.json` 1.0 escritos do
// zero a partir do SCHEMA da origem, nunca de fixture da origem nem de produção. Assets são só
// referências (sem PNG; hashes `synthetic-sha256-*`).
//
// Aqui só existem ENTRADAS e a INTENÇÃO de cada caso, declaradas ANTES de qualquer execução. Nenhum
// resultado esperado é escrito: veredito, poses, matrizes e tempos saem das funções reais da
// origem (`extended.mjs generate`), que também CONFERE a intenção (falha se divergir; uma intenção
// errada é corpus mal desenhado, não motivo para mudar a expectativa).
//
// Intenção (`intent`):
//   accepted      true | false  o veredito final da origem (`finalizeBundle`)
//   joinCodes     lista EXATA de códigos de `validateEditJoins` (ordenada), quando previsível
//   originDecides true se o resultado depende de aritmética de ponto flutuante na fronteira exata:
//                 nada é afirmado, a origem decide e o resultado fica gravado
//   schemaOk      true (padrão) | false
//   sourceCodes   códigos que devem aparecer nas issues do schema ou das junções

import { nextDown, nextUp, quaternionFromEulerXyzDegrees, seededShuffle } from "./numeric.mjs"

const clone = (value) => structuredClone(value)
const vec = (x, y, z) => ({ x, y, z })
const IDENTITY = { x: 0, y: 0, z: 0, w: 1 }
const FPS_24 = { numerator: 24, denominator: 1 }

const asset = (assetId, widthPx, heightPx, alphaMode) => ({
  assetId,
  revisionId: "rev-1",
  contentHash: `synthetic-sha256-${assetId}`,
  relativePath: `assets/${assetId}.png`,
  widthPx,
  heightPx,
  alphaMode,
  colorSpace: "srgb",
})

const assetRef = ({ assetId, revisionId, contentHash, relativePath }) => ({
  assetId,
  revisionId,
  contentHash,
  relativePath,
})

const BACKDROP = asset("backdrop", 64, 36, "opaque")
const FIGURE = asset("figure", 32, 32, "straight")

const groupNode = (id, parentId, overrides = {}) => ({
  id,
  parentId,
  type: "group",
  localPosition: vec(0, 0, 0),
  localRotation: IDENTITY,
  localScale: vec(1, 1, 1),
  ...overrides,
})

const planeNode = (id, parentId, record, overrides = {}) => ({
  id,
  parentId,
  type: "imagePlane",
  assetRef: assetRef(record),
  widthSceneUnits: 4,
  heightSceneUnits: 2.25,
  renderOrder: 0,
  localPosition: vec(0, 0, 0),
  localRotation: IDENTITY,
  localScale: vec(1, 1, 1),
  ...overrides,
})

const bundleShell = (bundleId, editDurationFrames, fps = FPS_24, profileFps = fps) => ({
  schemaVersion: "1.0",
  bundleId,
  revisionId: "rev-1",
  fps: clone(fps),
  editDurationFrames,
  renderProfile: {
    width: 320,
    height: 180,
    fps: clone(profileFps),
    colorSpace: "srgb-display",
    alphaMode: "straight",
    premultiplyOnUpload: false,
    videoCodec: "h264-yuv420p",
    threeRevision: "0.187.0",
    browserChannel: "synthetic-oracle",
  },
  assets: [BACKDROP, FIGURE].map(clone),
  performanceTracks: [],
  compositionPasses: [],
})

const simpleScene = (id) => ({
  id,
  schemaVersion: "1.0",
  rootNodeId: "root",
  nodes: [
    groupNode("root", null),
    planeNode("backdrop", "root", BACKDROP, {
      renderOrder: 0,
      localPosition: vec(0, 0, -2),
      widthSceneUnits: 8,
      heightSceneUnits: 4.5,
    }),
    planeNode("figure", "root", FIGURE, {
      renderOrder: 1,
      widthSceneUnits: 2,
      heightSceneUnits: 2,
      localPosition: vec(0.5, 0, 0),
    }),
  ],
})

const instance = (id, sceneDefinitionId, durationFrames, performanceTrackIds = []) => ({
  id,
  sceneDefinitionId,
  durationFrames,
  performanceTrackIds,
})

const shot = (id, sceneInstanceId, range, instanceStartFrame, cameraClipId, cameraInFrame, extra = {}) => ({
  id,
  sceneInstanceId,
  editRange: { startFrame: range[0], endFrameExclusive: range[1] },
  instanceStartFrame,
  cameraClipId,
  cameraInFrame,
  intent: `synthetic shot ${id}`,
  ...extra,
})

const CONTINUOUS = { transitionInto: { type: "continuous" } }
const cut = (reason) => ({ transitionInto: { type: "cut", reason } })

const dollyIn = (durationFrames, distanceSceneUnits, easing = "linear", from = "previousEnd") => ({
  recipe: "dollyIn",
  durationFrames,
  from,
  distanceSceneUnits,
  easing,
})

const zoomIn = (durationFrames, fovDeltaDeg, easing = "linear", from = "previousEnd") => ({
  recipe: "zoomIn",
  durationFrames,
  from,
  fovDeltaDeg,
  easing,
})

const hold = (durationFrames, from = "previousEnd") => ({
  recipe: "hold",
  durationFrames,
  from,
})

const keysSegment = (durationFrames, keys) => ({
  recipe: "keyframes",
  durationFrames,
  from: "previousEnd",
  keys: keys.map(([frame, position, target, fovDeg, easing = "linear"]) => ({
    frame,
    position,
    target,
    fovDeg,
    easing,
  })),
})

const clipOf = (id, initialPosition, initialTarget, initialFovDeg, segments, layers) => ({
  id,
  initialPosition,
  initialTarget,
  initialFovDeg,
  segments,
  ...(layers ? { layers } : {}),
})

// --------------------------------------------------------------------------------------------
// Família 1 — fronteiras de JOIN
//
// Dois shots de 8 frames, contínuos, no MESMO clipe e na mesma instância. A junção mede o frame 7
// do clipe contra o frame 8: um passo de 1 frame. Com `linear` e N frames de movimento, o passo é
// distância / (N - 1) (a origem usa t = frame / (N - 1)), então a distância escolhe o passo:
//
//   passo = D / 16 com N = 17  → D = 4 dá EXATAMENTE 0,25 (potências de 2: sem arredondamento)
//
// O limite da origem é `>` estrito: 0,25 exato NÃO é salto. As variantes "ulp" usam o double
// vizinho de D; o resultado depende do arredondamento e quem decide é a origem.
// --------------------------------------------------------------------------------------------

const joinBundle = ({ id, clip, fps = FPS_24, shotOrder = "ordered", shotLength = 8 }) => {
  const total = shotLength * 2
  const shots = [
    shot("shot-a", "inst-a", [0, shotLength], 0, clip.id, 0),
    shot("shot-b", "inst-a", [shotLength, total], shotLength, clip.id, shotLength, CONTINUOUS),
  ]
  if (shotOrder === "reversed") {
    shots.reverse()
  }
  return {
    ...bundleShell(id, total, fps),
    sceneDefinitions: [simpleScene("scene-a")],
    sceneInstances: [instance("inst-a", "scene-a", total)],
    cameraClips: [clip],
    shots,
  }
}

const AXIS_START = { position: vec(0, 1.5, 10), target: vec(0, 1.5, 0) }

/** Dolly em linha reta no eixo -Z: passo = distância / (N - 1), exato quando N - 1 é potência de 2. */
const dollyClip = (distance, frames = 17) =>
  clipOf("cam-join", AXIS_START.position, AXIS_START.target, 45, [
    dollyIn(frames, distance, "linear", "initial"),
  ])

/** Dolly numa diagonal (1,1,1)/√3: o passo passa por hypot e por √, sem valor exato garantido. */
const diagonalDollyClip = (distance) =>
  clipOf("cam-join", vec(10, 11.5, 10), vec(0, 1.5, 0), 45, [
    dollyIn(17, distance, "linear", "initial"),
  ])

/** Só a posição se move (alvo parado): isola CONTINUOUS_CAMERA_JUMP. */
const positionOnlyClip = (distance) =>
  clipOf("cam-join", AXIS_START.position, AXIS_START.target, 45, [
    keysSegment(17, [[16, vec(0, 1.5, 10 - distance), AXIS_START.target, 45]]),
  ])

/** Só o alvo se move (posição parada): isola CONTINUOUS_TARGET_JUMP. */
const targetOnlyClip = (distance) =>
  clipOf("cam-join", AXIS_START.position, AXIS_START.target, 45, [
    keysSegment(17, [[16, AXIS_START.position, vec(0, 1.5, -distance), 45]]),
  ])

/** Só o FOV muda (2 graus/frame a 32): isola CONTINUOUS_FOV_JUMP. */
const fovClip = (deltaDeg) =>
  clipOf("cam-join", AXIS_START.position, AXIS_START.target, 50, [
    zoomIn(17, deltaDeg, "linear", "initial"),
  ])

/**
 * Parado nos 8 primeiros frames (velocidade 0 no último frame do `hold`) e depois um dolly de N
 * frames: a junção cai na emenda, com pose igual dos dois lados e só a VELOCIDADE saltando de 0
 * para distância / (N - 1). 0,08 é a tolerância; D = 2 e N = 26 dá 2/25.
 */
const holdThenDollyClip = (distance) =>
  clipOf("cam-join", AXIS_START.position, AXIS_START.target, 45, [
    hold(8, "initial"),
    dollyIn(26, distance, "linear"),
  ])

const holdThenZoomClip = (deltaDeg) =>
  clipOf("cam-join", AXIS_START.position, AXIS_START.target, 50, [
    hold(8, "initial"),
    zoomIn(26, deltaDeg, "linear"),
  ])

const cases = []

const addCase = (entry) => {
  cases.push({ probes: {}, ...entry })
}

const joinProbes = { joinSamples: true, extraEditFrames: [6, 7, 8, 9] }

/** Três variantes (abaixo, igual, acima) do mesmo limite. */
const addBoundaryTriple = ({ stem, what, make, values, codesAbove, equalDecidedByOrigin = false }) => {
  const [below, equal, above] = values
  const intents = [
    ["below", below, { accepted: true, joinCodes: [] }],
    [
      "equal",
      equal,
      equalDecidedByOrigin ? { originDecides: true } : { accepted: true, joinCodes: [] },
    ],
    ["above", above, { accepted: false, joinCodes: [...codesAbove].sort() }],
  ]
  for (const [position, value, intent] of intents) {
    addCase({
      id: `join-${stem}-${position}`,
      family: "join-boundary",
      description: `${what}: ${position} do limite (valor ${value}).`,
      bundle: joinBundle({ id: `join-${stem}-${position}`, clip: make(value) }),
      intent: { schemaOk: true, ...intent },
      probes: joinProbes,
    })
  }
}

addBoundaryTriple({
  stem: "position",
  what: "Passo de posição por frame (dolly): 0,24375 / 0,25 exato / 0,25625 contra JOIN_POSITION_TOLERANCE_SCENE_UNITS = 0,25 (posição e alvo andam juntos)",
  make: dollyClip,
  values: [3.9, 4, 4.1],
  codesAbove: ["CONTINUOUS_CAMERA_JUMP", "CONTINUOUS_TARGET_JUMP"],
})

for (const [position, value] of [
  ["below", nextDown(4)],
  ["above", nextUp(4)],
]) {
  addCase({
    id: `join-position-ulp-${position}`,
    family: "join-boundary",
    description: `Dolly com distância ${position === "below" ? "um double abaixo" : "um double acima"} de 4 (${value}): passo a 1 ulp de 0,25; a aritmética da origem decide.`,
    bundle: joinBundle({ id: `join-position-ulp-${position}`, clip: dollyClip(value) }),
    intent: { schemaOk: true, originDecides: true },
    probes: joinProbes,
  })
}

addBoundaryTriple({
  stem: "target",
  what: "Só o alvo anda 0,24375 / 0,25 / 0,25625 por frame (posição parada)",
  make: targetOnlyClip,
  values: [3.9, 4, 4.1],
  codesAbove: ["CONTINUOUS_TARGET_JUMP"],
})

addBoundaryTriple({
  stem: "position-only",
  what: "Só a posição anda 0,24375 / 0,25 / 0,25625 por frame (alvo parado)",
  make: positionOnlyClip,
  values: [3.9, 4, 4.1],
  codesAbove: ["CONTINUOUS_CAMERA_JUMP"],
})

addBoundaryTriple({
  stem: "fov",
  what: "FOV muda 1,9375 / 2 / 2,0625 graus por frame contra JOIN_FOV_TOLERANCE_DEG = 2",
  make: fovClip,
  values: [31, 32, 33],
  codesAbove: ["CONTINUOUS_FOV_JUMP"],
})

addCase({
  id: "join-fov-ulp-above",
  family: "join-boundary",
  description: `FOV com delta um double acima de 32 (${nextUp(32)}): 1 ulp acima do limite de 2 graus por frame.`,
  bundle: joinBundle({ id: "join-fov-ulp-above", clip: fovClip(nextUp(32)) }),
  intent: { schemaOk: true, originDecides: true },
  probes: joinProbes,
})

addBoundaryTriple({
  stem: "velocity",
  what: "Salto de velocidade na emenda hold→dolly: 0,076 / 0,08 / 0,084 contra JOIN_VELOCITY_TOLERANCE_SCENE_UNITS_PER_FRAME = 0,08",
  make: holdThenDollyClip,
  values: [1.9, 2, 2.1],
  codesAbove: ["CONTINUOUS_VELOCITY_JUMP"],
  equalDecidedByOrigin: true,
})

addBoundaryTriple({
  stem: "fov-velocity",
  what: "Salto de velocidade de FOV na emenda hold→zoom: 0,48 / 0,5 / 0,52 contra JOIN_FOV_VELOCITY_TOLERANCE_DEG_PER_FRAME = 0,5",
  make: holdThenZoomClip,
  values: [12, 12.5, 13],
  codesAbove: ["CONTINUOUS_FOV_VELOCITY_JUMP"],
  equalDecidedByOrigin: true,
})

addBoundaryTriple({
  stem: "diagonal",
  what: "Dolly na diagonal (1,1,1)/√3: o passo é D/16 só em teoria; a igualdade passa por hypot e raiz",
  make: diagonalDollyClip,
  values: [3.9, 4, 4.1],
  codesAbove: ["CONTINUOUS_CAMERA_JUMP", "CONTINUOUS_TARGET_JUMP"],
  equalDecidedByOrigin: true,
})

// Três shots: só a SEGUNDA junção salta, e o shotId reportado é o do shot seguinte (shot-c).
addCase({
  id: "join-second-join-only-jumps",
  family: "join-boundary",
  description: "Três shots contínuos; o clipe tem dois dollies (passo 0,2 e depois 0,35). A junção A|B é suave e B|C salta; o código aponta shot-c.",
  bundle: (() => {
    const clip = clipOf("cam-join", AXIS_START.position, AXIS_START.target, 45, [
      dollyIn(13, 2.4, "linear", "initial"),
      dollyIn(13, 4.2, "linear"),
    ])
    return {
      ...bundleShell("join-second-join-only-jumps", 30),
      sceneDefinitions: [simpleScene("scene-a")],
      sceneInstances: [instance("inst-a", "scene-a", 30)],
      cameraClips: [clip],
      shots: [
        shot("shot-a", "inst-a", [0, 10], 0, "cam-join", 0),
        shot("shot-b", "inst-a", [10, 20], 10, "cam-join", 10, CONTINUOUS),
        shot("shot-c", "inst-a", [20, 30], 20, "cam-join", 20, CONTINUOUS),
      ],
    }
  })(),
  intent: {
    schemaOk: true,
    accepted: false,
    joinCodes: ["CONTINUOUS_CAMERA_JUMP", "CONTINUOUS_TARGET_JUMP"],
  },
  probes: { joinSamples: true, extraEditFrames: [9, 10, 19, 20] },
})

// A origem ordena os shots por `startFrame` antes de medir junções: a ordem do array não conta.
for (const [position, distance, accepted, joinCodes] of [
  ["below", 3.9, true, []],
  ["above", 4.1, false, ["CONTINUOUS_CAMERA_JUMP", "CONTINUOUS_TARGET_JUMP"]],
]) {
  addCase({
    id: `join-shots-listed-in-reverse-${position}`,
    family: "join-boundary",
    description: `Mesmo caso de join-position-${position}, mas com o array de shots em ordem inversa: a origem ordena por startFrame.`,
    bundle: joinBundle({
      id: `join-shots-listed-in-reverse-${position}`,
      clip: dollyClip(distance),
      shotOrder: "reversed",
    }),
    intent: { schemaOk: true, accepted, joinCodes },
    probes: joinProbes,
  })
}

// As junções não dependem do FPS (são quadros): mesmo veredito a 24000/1001 e a 30000/1001.
for (const [stem, fps] of [
  ["24000-1001", { numerator: 24000, denominator: 1001 }],
  ["30000-1001", { numerator: 30000, denominator: 1001 }],
]) {
  addCase({
    id: `join-position-above-${stem}`,
    family: "join-boundary",
    description: `Salto de posição de 0,25625 por frame a ${fps.numerator}/${fps.denominator} fps: a regra é por frame, não por segundo.`,
    bundle: joinBundle({ id: `join-position-above-${stem}`, clip: dollyClip(4.1), fps }),
    intent: {
      schemaOk: true,
      accepted: false,
      joinCodes: ["CONTINUOUS_CAMERA_JUMP", "CONTINUOUS_TARGET_JUMP"],
    },
    probes: joinProbes,
  })
  addCase({
    id: `join-position-below-${stem}`,
    family: "join-boundary",
    description: `Passo de 0,24375 por frame a ${fps.numerator}/${fps.denominator} fps: aceito.`,
    bundle: joinBundle({ id: `join-position-below-${stem}`, clip: dollyClip(3.9), fps }),
    intent: { schemaOk: true, accepted: true, joinCodes: [] },
    probes: joinProbes,
  })
}

// --------------------------------------------------------------------------------------------
// Família 2 — FPS racionais e conversão de tempo
// --------------------------------------------------------------------------------------------

const FPS_CASES = [
  { stem: "24000-1001", fps: { numerator: 24000, denominator: 1001 }, nominal: 24 },
  { stem: "30000-1001", fps: { numerator: 30000, denominator: 1001 }, nominal: 30 },
  { stem: "60000-1001", fps: { numerator: 60000, denominator: 1001 }, nominal: 60 },
  { stem: "25", fps: { numerator: 25, denominator: 1 }, nominal: 25 },
  { stem: "30", fps: { numerator: 30, denominator: 1 }, nominal: 30 },
  { stem: "60", fps: { numerator: 60, denominator: 1 }, nominal: 60 },
  { stem: "unreduced-48-2", fps: { numerator: 48, denominator: 2 }, nominal: 24 },
]

/**
 * Frames de tempo que importam: as bordas do 1º e do 2º segundo, o fim da edição e frames grandes
 * onde a deriva do ponto flutuante aparece (o frame 30000 a 30000/1001 começa em 1001 s exatos).
 */
const timeFrames = (nominal, duration) =>
  [
    ...new Set([
      0,
      1,
      2,
      nominal - 1,
      nominal,
      nominal + 1,
      2 * nominal - 1,
      2 * nominal,
      duration - 2,
      duration - 1,
      duration,
      duration + 1,
      1000,
      30000,
      100000,
    ]),
  ].sort((a, b) => a - b)

const fpsBundle = ({ id, fps, nominal, profileFps = fps }) => {
  const duration = 2 * nominal + 1
  const clip = clipOf("cam-fps", AXIS_START.position, AXIS_START.target, 45, [
    dollyIn(duration, 1, "linear", "initial"),
  ])
  return {
    ...bundleShell(id, duration, fps, profileFps),
    sceneDefinitions: [simpleScene("scene-a")],
    sceneInstances: [instance("inst-a", "scene-a", duration, ["perf-fps"])],
    performanceTracks: [
      {
        id: "perf-fps",
        instanceId: "inst-a",
        channels: [
          {
            kind: "cycle",
            channelId: "figure-spin",
            nodeId: "figure",
            property: "localRotation",
            periodFrames: nominal,
            amplitude: 15,
            axis: vec(0, 0, 1),
            phaseOffsetFrames: 0,
          },
          {
            kind: "event",
            channelId: "backdrop-slide",
            nodeId: "backdrop",
            property: "localPosition",
            startInstanceFrame: nominal,
            durationFrames: nominal,
            delta: vec(0.5, 0, 0),
            holdFinal: true,
          },
        ],
      },
    ],
    cameraClips: [clip],
    shots: [
      shot("shot-1", "inst-a", [0, nominal], 0, "cam-fps", 0),
      shot("shot-2", "inst-a", [nominal, 2 * nominal], nominal, "cam-fps", nominal, CONTINUOUS),
      shot("shot-3", "inst-a", [2 * nominal, duration], 2 * nominal, "cam-fps", 2 * nominal, cut("synthetic cut at the second boundary")),
    ],
  }
}

for (const { stem, fps, nominal } of FPS_CASES) {
  const duration = 2 * nominal + 1
  const frames = timeFrames(nominal, duration)
  addCase({
    id: `fps-${stem}`,
    family: "fps",
    description: `Bundle a ${fps.numerator}/${fps.denominator} fps com 3 shots nas bordas dos segundos, canais cycle/event e câmera contínua; mais sondas de tempo (frame↔segundos) em torno de cada início de frame.`,
    bundle: fpsBundle({ id: `fps-${stem}`, fps, nominal }),
    intent: { schemaOk: true, accepted: true, joinCodes: [] },
    probes: {
      extraEditFrames: [nominal - 1, nominal, nominal + 1, 2 * nominal - 1, 2 * nominal],
      joinSamples: true,
      time: { frames },
    },
  })
}

addCase({
  id: "fps-above-1000",
  family: "fps",
  description: "2000/1 fps: a origem aceita (o schema só pede inteiros positivos). O importador Rust declara um teto de 1000 fps (FPS_OUT_OF_RANGE): a divergência documentada fica no consumidor, não aqui.",
  bundle: fpsBundle({ id: "fps-above-1000", fps: { numerator: 2000, denominator: 1 }, nominal: 4 }),
  intent: { schemaOk: true, accepted: true, joinCodes: [] },
  probes: { time: { frames: [0, 1, 2000, 2001] } },
})

addCase({
  id: "fps-profile-mismatch-ntsc",
  family: "fps",
  description: "fps do bundle 30000/1001 e do renderProfile 30000/1000: FPS_MISMATCH (a igualdade é por numerador e denominador, não por valor).",
  bundle: fpsBundle({
    id: "fps-profile-mismatch-ntsc",
    fps: { numerator: 30000, denominator: 1001 },
    nominal: 30,
    profileFps: { numerator: 30000, denominator: 1000 },
  }),
  intent: { schemaOk: false, accepted: false, sourceCodes: ["FPS_MISMATCH"] },
})

addCase({
  id: "fps-profile-unreduced-equivalent",
  family: "fps",
  description: "bundle.fps 24/1 e renderProfile.fps 48/2: mesmo valor, frações diferentes → FPS_MISMATCH na origem (comparação literal).",
  bundle: fpsBundle({
    id: "fps-profile-unreduced-equivalent",
    fps: { numerator: 24, denominator: 1 },
    nominal: 24,
    profileFps: { numerator: 48, denominator: 2 },
  }),
  intent: { schemaOk: false, accepted: false, sourceCodes: ["FPS_MISMATCH"] },
})

addCase({
  id: "fps-zero-denominator",
  family: "fps",
  description: "Denominador 0: o schema recusa (inteiro positivo).",
  bundle: (() => {
    const bundle = fpsBundle({ id: "fps-zero-denominator", fps: FPS_24, nominal: 24 })
    bundle.fps = { numerator: 24, denominator: 0 }
    return bundle
  })(),
  intent: { schemaOk: false, accepted: false },
})

// --------------------------------------------------------------------------------------------
// Família 3 — avaliação FORA DE ORDEM (shots, instâncias, canais de performance, camadas)
// --------------------------------------------------------------------------------------------

const rigScene = (id) => ({
  id,
  schemaVersion: "1.0",
  rootNodeId: "root",
  nodes: [
    groupNode("root", null),
    planeNode("backdrop", "root", BACKDROP, {
      renderOrder: 0,
      localPosition: vec(0, 0, -3),
      widthSceneUnits: 8,
      heightSceneUnits: 4.5,
    }),
    groupNode("stage", "root", {
      localPosition: vec(1, 0, -1),
      localRotation: { x: 0, y: 0.7071067811865476, z: 0, w: 0.7071067811865476 },
      localScale: vec(2, 2, 2),
    }),
    groupNode("arm", "stage", {
      localPosition: vec(0, 1, 0),
      localRotation: { x: 0, y: 0, z: 0.25881904510252074, w: 0.9659258262890683 },
    }),
    planeNode("pen", null, FIGURE, {
      renderOrder: 2,
      widthSceneUnits: 0.5,
      heightSceneUnits: 0.5,
      localPosition: vec(0.1, 0.2, 0.3),
      attachToSocket: { parentNodeId: "arm", socketName: "hand" },
    }),
  ],
  sockets: [
    { nodeId: "arm", name: "hand", localPosition: vec(0, 0.5, 0), localRotation: IDENTITY },
  ],
})

const richClip = () =>
  clipOf(
    "cam-rich",
    vec(2, 1, 9),
    vec(0, 1, 0),
    50,
    [
      zoomIn(20, 10, "easeOutCubic", "initial"),
      {
        recipe: "orbit",
        durationFrames: 30,
        from: "previousEnd",
        yawDeg: 40,
        pitchDeg: 10,
        easing: "easeInOutCubic",
      },
      keysSegment(40, [
        [10, vec(2.5, 1.2, 7), vec(0, 1, 0), 40, "linear"],
        [25, vec(1, 2, 6), vec(0.2, 1, 0), 38, "easeInOutCubic"],
        [39, vec(-1, 1.5, 6.5), vec(0, 1, 0), 42, "easeOutCubic"],
      ]),
      hold(30),
    ],
    [
      { segments: [dollyIn(60, 1, "linear"), hold(20)] },
      {
        segments: [
          {
            recipe: "truckLeft",
            durationFrames: 40,
            from: "previousEnd",
            distanceSceneUnits: 0.5,
            easing: "easeOutCubic",
          },
        ],
      },
    ],
  )

const outOfOrderBundle = () => ({
  ...bundleShell("ooo-shots-instances-layers", 120),
  sceneDefinitions: [rigScene("scene-rig")],
  sceneInstances: [
    instance("inst-a", "scene-rig", 80, ["perf-a"]),
    instance("inst-b", "scene-rig", 60, ["perf-b"]),
  ],
  performanceTracks: [
    {
      id: "perf-a",
      instanceId: "inst-a",
      channels: [
        {
          kind: "cycle",
          channelId: "arm-swing",
          nodeId: "arm",
          property: "localRotation",
          periodFrames: 24,
          amplitude: 20,
          axis: vec(0, 0, 1),
          phaseOffsetFrames: 3,
        },
        {
          kind: "event",
          channelId: "stage-lift",
          nodeId: "stage",
          property: "localPosition",
          startInstanceFrame: 12,
          durationFrames: 24,
          delta: vec(0, 0.5, 0),
          holdFinal: true,
        },
        {
          kind: "static",
          channelId: "pen-scale",
          nodeId: "pen",
          property: "localScale",
          value: vec(1.5, 1.5, 1.5),
        },
      ],
    },
    {
      id: "perf-b",
      instanceId: "inst-b",
      channels: [
        {
          kind: "cycle",
          channelId: "arm-swing",
          nodeId: "arm",
          property: "localRotation",
          periodFrames: 17,
          amplitude: 35,
          axis: vec(1, 0, 0),
          phaseOffsetFrames: 0,
        },
        {
          kind: "event",
          channelId: "stage-lift",
          nodeId: "stage",
          property: "localPosition",
          startInstanceFrame: 5,
          durationFrames: 30,
          delta: vec(0.25, 0, 0.5),
          holdFinal: true,
        },
      ],
    },
  ],
  cameraClips: [richClip()],
  shots: [
    shot("shot-1", "inst-a", [0, 40], 0, "cam-rich", 0),
    shot("shot-2", "inst-b", [40, 80], 10, "cam-rich", 40, cut("synthetic hard cut to the second instance")),
    shot("shot-3", "inst-a", [80, 120], 40, "cam-rich", 80, cut("synthetic hard cut back to the first instance")),
  ],
})

const OOO_FRAMES = [-1, 0, 1, 2, 19, 20, 21, 38, 39, 40, 41, 42, 59, 60, 79, 80, 81, 82, 100, 118, 119, 120]
const interleaveEnds = (frames) => {
  const result = []
  let low = 0
  let high = frames.length - 1
  while (low <= high) {
    result.push(frames[low])
    if (low !== high) {
      result.push(frames[high])
    }
    low += 1
    high -= 1
  }
  return result
}

addCase({
  id: "ooo-shots-instances-layers",
  family: "out-of-order",
  description: "Três shots em duas instâncias (cortes), canais cycle/event/static com períodos diferentes, clipe com keyframes e 2 camadas. A mesma lista de frames é avaliada em 7 ordens (inclusive repetida); cada resposta tem de ser idêntica à canônica.",
  bundle: outOfOrderBundle(),
  intent: { schemaOk: true, accepted: true, joinCodes: [] },
  probes: {
    extraEditFrames: OOO_FRAMES.filter((frame) => frame >= 0 && frame < 120),
    orders: [
      { name: "forward", frames: [...OOO_FRAMES] },
      { name: "reverse", frames: [...OOO_FRAMES].reverse() },
      { name: "ends-inward", frames: interleaveEnds(OOO_FRAMES) },
      { name: "shuffle-seed-1", frames: seededShuffle(OOO_FRAMES, 1) },
      { name: "shuffle-seed-2024", frames: seededShuffle(OOO_FRAMES, 2024) },
      { name: "forward-then-reverse-repeated", frames: [...OOO_FRAMES, ...[...OOO_FRAMES].reverse()] },
      { name: "cut-hopping", frames: [80, 0, 119, 40, 39, 81, 1, 118, 41, 79, 40, 40, 80] },
    ],
  },
})

// --------------------------------------------------------------------------------------------
// Família 4 — matrizes mundo: gimbal, escalas extremas, sockets encadeados, câmera lookAt
// --------------------------------------------------------------------------------------------

const singleShotBundle = ({ id, definition, performanceTracks = [], clip, duration = 24 }) => ({
  ...bundleShell(id, duration),
  sceneDefinitions: [definition],
  sceneInstances: [
    instance("inst-a", definition.id, duration, performanceTracks.map((track) => track.id)),
  ],
  performanceTracks,
  cameraClips: [clip],
  shots: [shot("shot-only", "inst-a", [0, duration], 0, clip.id, 0)],
})

const stillClip = (id = "cam-still", duration = 24) =>
  clipOf(id, vec(0, 1, 7), vec(0, 1, 0), 40, [hold(duration, "initial")])

// Euler XYZ em graus para quaternion. O Three.js reconverte quaternion → Euler e, perto de
// |m13| >= 0,9999999 (y ≈ ±89,9744°), zera z: a rotação deixa de ser a original. O corpus cerca
// esse limiar dos dois lados.
const GIMBAL_Y_DEGREES = [89.9, 89.973, 89.974, 89.975, 89.98, 90, -89.98, -90]

const gimbalDefinition = () => {
  const nodes = [groupNode("root", null)]
  GIMBAL_Y_DEGREES.forEach((yDeg, index) => {
    const id = `g${index}`
    nodes.push(
      groupNode(id, "root", {
        localPosition: vec(index * 2, 0, 0),
        localRotation: quaternionFromEulerXyzDegrees(20, yDeg, 30),
      }),
      groupNode(`${id}-child`, id, { localPosition: vec(1, 2, 3) }),
    )
  })
  return { id: "scene-gimbal", schemaVersion: "1.0", rootNodeId: "root", nodes }
}

addCase({
  id: "world-gimbal-threshold",
  family: "world-matrix",
  description: "Oito nós com Euler XYZ (20°, y, 30°) para y de 89,9° a 90° e negativos, cercando o limiar de gimbal do Three.js; um canal cycle de rotação soma graus de Euler em um deles.",
  bundle: singleShotBundle({
    id: "world-gimbal-threshold",
    definition: gimbalDefinition(),
    performanceTracks: [
      {
        id: "perf-gimbal",
        instanceId: "inst-a",
        channels: [
          {
            kind: "cycle",
            channelId: "g4-wobble",
            nodeId: "g4",
            property: "localRotation",
            periodFrames: 12,
            amplitude: 0.01,
            axis: vec(0, 1, 0),
            phaseOffsetFrames: 0,
          },
        ],
      },
    ],
    clip: stillClip(),
  }),
  intent: { schemaOk: true, accepted: true, joinCodes: [] },
  probes: { extraEditFrames: [0, 3, 6, 9, 12, 18, 23] },
})

const extremeScaleDefinition = () => {
  const nodes = [groupNode("root", null)]
  // Cadeia de 12 níveis: cada um gira 15° em torno de um eixo alternado e escala 1,1.
  let parent = "root"
  for (let level = 1; level <= 12; level += 1) {
    const axis = ["x", "y", "z"][level % 3]
    const half = (15 * Math.PI) / 360
    const rotation = { x: 0, y: 0, z: 0, w: Math.cos(half) }
    rotation[axis] = Math.sin(half)
    const id = `chain-${level}`
    nodes.push(
      groupNode(id, parent, {
        localPosition: vec(0.5, 0.25 * level, -0.1 * level),
        localRotation: rotation,
        localScale: vec(1.1, 1.1, 1.1),
      }),
    )
    parent = id
  }
  nodes.push(
    groupNode("anisotropic", "root", { localScale: vec(2, 0.5, 3), localPosition: vec(-3, 0, 0) }),
    groupNode("anisotropic-child", "anisotropic", {
      localPosition: vec(1, 1, 1),
      localRotation: quaternionFromEulerXyzDegrees(10, 40, 70),
    }),
    groupNode("mirrored-x", "root", { localScale: vec(-1, 1, 1), localPosition: vec(0, -2, 0) }),
    groupNode("mirrored-child", "mirrored-x", {
      localPosition: vec(1, 0, 0),
      localRotation: quaternionFromEulerXyzDegrees(0, 0, 45),
    }),
    groupNode("far", "root", { localPosition: vec(1e6, -2e6, 3e6) }),
    groupNode("far-child", "far", { localPosition: vec(1, 1, 1), localScale: vec(1e3, 1e-3, 1) }),
    groupNode("collapsed", "root", { localScale: vec(0, 1, 1) }),
    groupNode("collapsed-child", "collapsed", { localPosition: vec(1, 1, 1) }),
  )
  return { id: "scene-extreme", schemaVersion: "1.0", rootNodeId: "root", nodes }
}

addCase({
  id: "world-extreme-scales-and-chain",
  family: "world-matrix",
  description: "Cadeia de 12 níveis (15° alternados, escala 1,1), escala anisotrópica, espelhada, translação de 1e6, escala 1e3/1e-3 e escala zero (matriz singular): acúmulo de arredondamento e valores extremos.",
  bundle: singleShotBundle({
    id: "world-extreme-scales-and-chain",
    definition: extremeScaleDefinition(),
    clip: stillClip(),
  }),
  intent: { schemaOk: true, accepted: true, joinCodes: [] },
  probes: { extraEditFrames: [0, 12, 23] },
})

const socketChainDefinition = () => ({
  id: "scene-sockets",
  schemaVersion: "1.0",
  rootNodeId: "root",
  nodes: [
    groupNode("root", null),
    groupNode("a", "root", {
      localPosition: vec(1, 0, 0),
      localRotation: quaternionFromEulerXyzDegrees(0, 0, 30),
      localScale: vec(1.5, 1.5, 1.5),
    }),
    groupNode("b", null, {
      localPosition: vec(0, 0.5, 0.2),
      localRotation: quaternionFromEulerXyzDegrees(0, 20, 0),
      attachToSocket: { parentNodeId: "a", socketName: "s1" },
    }),
    groupNode("c", null, {
      localPosition: vec(0.1, 0.1, 0.1),
      attachToSocket: { parentNodeId: "b", socketName: "s2" },
    }),
    groupNode("d", "c", { localPosition: vec(0, 0, 1), localScale: vec(0.5, 2, 0.5) }),
  ],
  sockets: [
    { nodeId: "a", name: "s1", localPosition: vec(0, 1, 0), localRotation: quaternionFromEulerXyzDegrees(45, 0, 0) },
    { nodeId: "b", name: "s2", localPosition: vec(0.3, 0, 0), localRotation: quaternionFromEulerXyzDegrees(0, 0, -35) },
  ],
})

addCase({
  id: "world-sockets-chain-animated",
  family: "world-matrix",
  description: "Dois sockets encadeados (a→b→c) com rotações de socket não triviais e um filho; canais cycle (rotação de a) e event (posição de b) deslocam a cadeia ao longo do tempo.",
  bundle: singleShotBundle({
    id: "world-sockets-chain-animated",
    definition: socketChainDefinition(),
    performanceTracks: [
      {
        id: "perf-sockets",
        instanceId: "inst-a",
        channels: [
          {
            kind: "cycle",
            channelId: "a-swing",
            nodeId: "a",
            property: "localRotation",
            periodFrames: 16,
            amplitude: 25,
            axis: vec(0, 1, 0),
            phaseOffsetFrames: 2,
          },
          {
            kind: "event",
            channelId: "b-slide",
            nodeId: "b",
            property: "localPosition",
            startInstanceFrame: 4,
            durationFrames: 12,
            delta: vec(0.4, -0.2, 0.1),
            holdFinal: true,
          },
        ],
      },
    ],
    clip: stillClip(),
  }),
  intent: { schemaOk: true, accepted: true, joinCodes: [] },
  probes: { extraEditFrames: [0, 2, 4, 8, 10, 16, 20, 23] },
})

// Poses de câmera nos frames-chave do clipe; a matriz mundo da câmera é a do `lookAt` do Three.js.
addCase({
  id: "world-camera-lookat-poses",
  family: "world-matrix",
  description: "Câmera por keyframes cobrindo pose comum, olhando reto para baixo e para cima (cross zero → Three.js empurra z em 1e-4), quase vertical (1e-7 de desvio, SEM empurrão), muito longe (1e5) e para -Z; a matriz mundo da câmera vem do lookAt do Three.js fixado.",
  bundle: singleShotBundle({
    id: "world-camera-lookat-poses",
    definition: simpleScene("scene-a"),
    clip: clipOf("cam-lookat", vec(0, 1, 8), vec(0, 1, 0), 45, [
      keysSegment(40, [
        [5, vec(3, 2, 6), vec(0.5, 1, 0), 50],
        [10, vec(0, 5, 0), vec(0, 0, 0), 40],
        [15, vec(0, -3, 0), vec(0, 2, 0), 40],
        [20, vec(1e-7, 5, 0), vec(0, 0, 0), 40],
        [25, vec(1e5, 1e5, 1e5), vec(0, 0, 0), 10],
        [30, vec(0, 0, 5), vec(0, 0, 0), 90],
        [39, vec(-4, 1, -4), vec(0, 1, 0), 60],
      ]),
    ]),
    duration: 40,
  }),
  intent: { schemaOk: true, accepted: true, joinCodes: [] },
  probes: { extraEditFrames: [0, 5, 10, 15, 20, 25, 30, 35, 39] },
})

// `kind` é só a intenção declarada: "boundary" = a fronteira exata é decidida pela origem.
const kindOf = (intent) => {
  if (intent.accepted === true) {
    return "valid"
  }
  if (intent.accepted === false) {
    return "invalid"
  }
  return "boundary"
}

export const buildExtendedCorpus = () =>
  cases.map((entry) => ({
    ...entry,
    kind: kindOf(entry.intent),
    bundle: clone(entry.bundle),
  }))
