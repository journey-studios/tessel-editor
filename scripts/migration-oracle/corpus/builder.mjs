// Corpus SINTÉTICO do oráculo: bundles `render-bundle.json` (schema 1.0) pequenos, escritos aqui
// do zero a partir do SCHEMA da origem (não são cópia de nenhuma produção nem de fixture da origem).
//
// Os bundles são só ENTRADAS. Nenhum resultado esperado é escrito aqui: veredito, issues, poses e
// matrizes saem das funções reais da origem (`oracle.mjs generate`). As chaves `expect*` de cada
// caso são apenas a INTENÇÃO do caso, conferida na geração: se a origem não produzir o código que o
// caso diz exercitar, a geração falha (corpus mal desenhado), em vez de gravar um caso vazio.
//
// Os assets são só referências: não existe arquivo PNG/áudio, e hashes são strings sintéticas.

const clone = (value) => structuredClone(value)

const vec = (x, y, z) => ({ x, y, z })
const IDENTITY = { x: 0, y: 0, z: 0, w: 1 }

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

const renderProfile = () => ({
  width: 320,
  height: 180,
  fps: { numerator: 24, denominator: 1 },
  colorSpace: "srgb-display",
  alphaMode: "straight",
  premultiplyOnUpload: false,
  videoCodec: "h264-yuv420p",
  threeRevision: "0.187.0",
  browserChannel: "synthetic-oracle",
})

const bundleShell = (bundleId, editDurationFrames) => ({
  schemaVersion: "1.0",
  bundleId,
  revisionId: "rev-1",
  fps: { numerator: 24, denominator: 1 },
  editDurationFrames,
  renderProfile: renderProfile(),
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

const dollyIn = (durationFrames, distanceSceneUnits, easing, from = "previousEnd") => ({
  recipe: "dollyIn",
  durationFrames,
  from,
  distanceSceneUnits,
  easing,
})

const hold = (durationFrames, from = "previousEnd") => ({
  recipe: "hold",
  durationFrames,
  from,
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

/** BASE 1: dois planos, dois shots contínuos no mesmo clipe de câmera (dolly, hold, truck). */
const minimalBundle = () => ({
  ...bundleShell("oracle-minimal", 96),
  sceneDefinitions: [simpleScene("scene-a")],
  sceneInstances: [instance("inst-a", "scene-a", 96)],
  cameraClips: [
    {
      id: "cam-main",
      initialPosition: vec(0, 1.5, 8),
      initialTarget: vec(0, 1, 0),
      initialFovDeg: 45,
      segments: [
        dollyIn(32, 1.5, "easeInOutCubic", "initial"),
        hold(16),
        {
          recipe: "truckRight",
          durationFrames: 48,
          from: "previousEnd",
          distanceSceneUnits: 1,
          easing: "easeOutCubic",
        },
      ],
    },
  ],
  shots: [
    shot("shot-1", "inst-a", [0, 48], 0, "cam-main", 0),
    shot("shot-2", "inst-a", [48, 96], 48, "cam-main", 48, {
      transitionInto: { type: "continuous" },
    }),
  ],
})

const keyframesSegment = (durationFrames, keys) => ({
  recipe: "keyframes",
  durationFrames,
  from: "previousEnd",
  keys: keys.map(([frame, position, target, fovDeg, easing]) => ({
    frame,
    position,
    target,
    fovDeg,
    easing,
  })),
})

const CAMERA_KEYS = [
  [10, vec(2.5, 1.2, 7), vec(0, 1, 0), 40, "linear"],
  [25, vec(1, 2, 6), vec(0.2, 1, 0), 38, "easeInOutCubic"],
  [39, vec(-1, 1.5, 6.5), vec(0, 1, 0), 42, "easeOutCubic"],
]

/** BASE 2: um clipe de 120 frames com todas as famílias de movimento, keyframes e 2 camadas. */
const cameraRichBundle = () => ({
  ...bundleShell("oracle-camera-rich", 120),
  sceneDefinitions: [simpleScene("scene-a")],
  sceneInstances: [instance("inst-a", "scene-a", 120)],
  cameraClips: [
    {
      id: "cam-rich",
      initialPosition: vec(2, 1, 9),
      initialTarget: vec(0, 1, 0),
      initialFovDeg: 50,
      segments: [
        {
          recipe: "zoomIn",
          durationFrames: 20,
          from: "initial",
          fovDeltaDeg: 10,
          easing: "easeOutCubic",
        },
        {
          recipe: "orbit",
          durationFrames: 30,
          from: "previousEnd",
          yawDeg: 40,
          pitchDeg: 10,
          easing: "easeInOutCubic",
        },
        keyframesSegment(40, CAMERA_KEYS),
        hold(30),
      ],
      layers: [
        {
          segments: [
            dollyIn(60, 1, "linear"),
            hold(20),
          ],
        },
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
    },
  ],
  shots: [shot("shot-only", "inst-a", [0, 120], 0, "cam-rich", 0)],
})

/** BASE 3: hierarquia com rotação/escala, socket, e canais de performance (cycle, event, static). */
const graphPerformanceBundle = () => {
  const sceneDefinition = {
    id: "scene-rig",
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
      // 90 graus em torno de Y: componentes exatos de sin(45°) e cos(45°).
      groupNode("stage", "root", {
        localPosition: vec(1, 0, -1),
        localRotation: { x: 0, y: 0.7071067811865476, z: 0, w: 0.7071067811865476 },
        localScale: vec(2, 2, 2),
      }),
      // 30 graus em torno de Z.
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
      {
        nodeId: "arm",
        name: "hand",
        localPosition: vec(0, 0.5, 0),
        localRotation: IDENTITY,
      },
    ],
  }
  return {
    ...bundleShell("oracle-graph-performance", 48),
    sceneDefinitions: [sceneDefinition],
    sceneInstances: [instance("inst-a", "scene-rig", 48, ["perf-1"])],
    performanceTracks: [
      {
        id: "perf-1",
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
    ],
    cameraClips: [
      {
        id: "cam-rig",
        initialPosition: vec(0, 1, 7),
        initialTarget: vec(0, 1, 0),
        initialFovDeg: 40,
        segments: [hold(48, "initial")],
      },
    ],
    shots: [shot("shot-rig", "inst-a", [0, 48], 0, "cam-rig", 0)],
  }
}

/** BASE 4: duas instâncias, offsets de instância/câmera diferentes de zero e um corte com motivo. */
const cutBundle = () => ({
  ...bundleShell("oracle-cut-offsets", 100),
  sceneDefinitions: [simpleScene("scene-a"), simpleScene("scene-b")],
  sceneInstances: [instance("inst-a", "scene-a", 60), instance("inst-b", "scene-b", 60)],
  cameraClips: [
    {
      id: "cam-a",
      initialPosition: vec(0, 1, 6),
      initialTarget: vec(0, 1, 0),
      initialFovDeg: 50,
      segments: [dollyIn(40, 1, "linear", "initial")],
    },
    {
      id: "cam-b",
      initialPosition: vec(-2, 1, 5),
      initialTarget: vec(0, 1, 0),
      initialFovDeg: 35,
      segments: [
        {
          recipe: "zoomOut",
          durationFrames: 30,
          from: "initial",
          fovDeltaDeg: 15,
          easing: "easeInOutCubic",
        },
      ],
    },
  ],
  shots: [
    shot("shot-a", "inst-a", [0, 40], 10, "cam-a", 5),
    shot("shot-b", "inst-b", [40, 100], 0, "cam-b", 0, {
      transitionInto: { type: "cut", reason: "synthetic hard cut to the second scene" },
    }),
  ],
})

const cases = []

const addCase = (id, kind, description, bundle, intent = {}) => {
  cases.push({ id, kind, description, intent, bundle })
}

const addValid = (id, description, bundle, extraEditFrames = []) =>
  addCase(id, "valid", description, bundle, { extraEditFrames })

/**
 * `mutate` recebe um clone da base e o altera. `intent` descreve o que o caso exercita:
 * `sourceCodes` (códigos estáveis da origem esperados entre as issues do schema ou das junções),
 * `zodPaths` (caminhos de erro estrutural do zod) e `messageIncludes`.
 */
const addInvalid = (id, description, base, mutate, intent) => {
  const bundle = clone(base())
  mutate(bundle)
  addCase(id, "invalid", description, bundle, intent)
}

// --------------------------------------------------------------------------------------------
// Válidos
// --------------------------------------------------------------------------------------------

addValid(
  "valid-minimal-continuous",
  "Dois planos, dois shots contínuos no mesmo clipe: dolly, hold e truck com 2 easings.",
  minimalBundle(),
)
addValid(
  "valid-camera-recipes-layers",
  "Zoom, orbit, keyframes e hold na trilha principal, mais 2 camadas aditivas (dolly+hold, truck).",
  cameraRichBundle(),
)
addValid(
  "valid-graph-sockets-performance",
  "Hierarquia com quaternions/escala, socket e canais cycle/event/static; amostra o meio dos canais.",
  graphPerformanceBundle(),
  [6, 12, 18, 24, 30, 36],
)
addValid(
  "valid-cut-instance-offsets",
  "Duas instâncias, instanceStartFrame/cameraInFrame não nulos e corte com motivo entre shots.",
  cutBundle(),
)

// --------------------------------------------------------------------------------------------
// Inválidos: schema (estrutura)
// --------------------------------------------------------------------------------------------

addInvalid(
  "invalid-schema-version",
  "schemaVersion diferente de 1.0 (literal).",
  minimalBundle,
  (b) => {
    b.schemaVersion = "2.0"
  },
  { zodPaths: ["schemaVersion"] },
)
addInvalid(
  "invalid-missing-bundle-id",
  "Campo obrigatório ausente.",
  minimalBundle,
  (b) => {
    delete b.bundleId
  },
  { zodPaths: ["bundleId"] },
)
addInvalid(
  "invalid-fps-zero-numerator",
  "FPS racional com numerador 0 (deve ser inteiro positivo).",
  minimalBundle,
  (b) => {
    b.fps.numerator = 0
  },
  { zodPaths: ["fps.numerator"] },
)
addInvalid(
  "invalid-unknown-recipe",
  "Receita de câmera fora das 9 conhecidas (união discriminada).",
  minimalBundle,
  (b) => {
    b.cameraClips[0].segments[0].recipe = "crane"
  },
  { zodPaths: ["cameraClips.0.segments.0.recipe"] },
)
addInvalid(
  "invalid-dissolve-transition",
  "Transição dissolve: o schema aceita o formato, o refinamento da origem recusa.",
  minimalBundle,
  (b) => {
    b.shots[1].transitionInto = { type: "dissolve", durationFrames: 6 }
  },
  { messageIncludes: ["Dissolve transitions are not supported"] },
)

// --------------------------------------------------------------------------------------------
// Inválidos: perfil de render e assets
// --------------------------------------------------------------------------------------------

addInvalid(
  "invalid-fps-mismatch",
  "renderProfile.fps diferente de fps.",
  minimalBundle,
  (b) => {
    b.renderProfile.fps = { numerator: 30, denominator: 1 }
  },
  { sourceCodes: ["FPS_MISMATCH"] },
)
addInvalid(
  "invalid-premultiply-on-upload",
  "premultiplyOnUpload declarado e não implementado.",
  minimalBundle,
  (b) => {
    b.renderProfile.premultiplyOnUpload = true
  },
  { sourceCodes: ["PREMULTIPLY_UNSUPPORTED"] },
)
addInvalid(
  "invalid-rec709-video",
  "colorSpace rec709-video aceito pelo schema, recusado pela integridade.",
  minimalBundle,
  (b) => {
    b.renderProfile.colorSpace = "rec709-video"
  },
  { sourceCodes: ["REC709_VIDEO_UNSUPPORTED"] },
)
addInvalid(
  "invalid-asset-premultiplied",
  "Asset com alpha premultiplicado.",
  minimalBundle,
  (b) => {
    b.assets[1].alphaMode = "premultiplied"
  },
  { sourceCodes: ["ASSET_PREMULTIPLIED_UNSUPPORTED"] },
)
addInvalid(
  "invalid-duplicate-asset-id",
  "assetId repetido em assets[].",
  minimalBundle,
  (b) => {
    b.assets.push(clone(b.assets[0]))
  },
  { sourceCodes: ["DUPLICATE_ASSET_ID"] },
)
addInvalid(
  "invalid-asset-ref-unknown",
  "assetRef de nó aponta para asset inexistente.",
  minimalBundle,
  (b) => {
    b.sceneDefinitions[0].nodes[2].assetRef.assetId = "ghost-asset"
  },
  { sourceCodes: ["ASSET_REF_UNKNOWN"] },
)
addInvalid(
  "invalid-asset-ref-mismatch",
  "assetRef com contentHash diferente do registro em assets[].",
  minimalBundle,
  (b) => {
    b.sceneDefinitions[0].nodes[2].assetRef.contentHash = "synthetic-other-hash"
  },
  { sourceCodes: ["ASSET_REF_MISMATCH"] },
)
addInvalid(
  "invalid-image-plane-missing-asset",
  "imagePlane sem assetRef.",
  minimalBundle,
  (b) => {
    delete b.sceneDefinitions[0].nodes[2].assetRef
  },
  { sourceCodes: ["IMAGE_PLANE_MISSING_ASSET"] },
)

// --------------------------------------------------------------------------------------------
// Inválidos: grafo de cena, referências e transformações
// --------------------------------------------------------------------------------------------

addInvalid(
  "invalid-node-missing-parent",
  "parentId referencia nó inexistente.",
  minimalBundle,
  (b) => {
    b.sceneDefinitions[0].nodes[2].parentId = "nowhere"
  },
  { sourceCodes: ["MISSING_PARENT"] },
)
addInvalid(
  "invalid-node-parent-cycle",
  "Ciclo de pais entre dois planos.",
  minimalBundle,
  (b) => {
    b.sceneDefinitions[0].nodes[1].parentId = "figure"
    b.sceneDefinitions[0].nodes[2].parentId = "backdrop"
  },
  { sourceCodes: ["PARENT_CYCLE"] },
)
addInvalid(
  "invalid-missing-root-node",
  "rootNodeId não existe em nodes.",
  minimalBundle,
  (b) => {
    b.sceneDefinitions[0].rootNodeId = "absent-root"
  },
  { sourceCodes: ["MISSING_ROOT_NODE"] },
)
addInvalid(
  "invalid-duplicate-node-id",
  "Dois nós com o mesmo id.",
  minimalBundle,
  (b) => {
    b.sceneDefinitions[0].nodes.push(clone(b.sceneDefinitions[0].nodes[2]))
  },
  { sourceCodes: ["DUPLICATE_NODE_ID"] },
)
addInvalid(
  "invalid-orphan-node",
  "Nó não-raiz com parentId nulo e sem socket.",
  minimalBundle,
  (b) => {
    b.sceneDefinitions[0].nodes[2].parentId = null
  },
  { sourceCodes: ["ORPHAN_NODE"] },
)
addInvalid(
  "invalid-unsupported-node-type",
  "Tipo de nó aceito pelo schema (overlay) e recusado pela estrutura do grafo.",
  minimalBundle,
  (b) => {
    b.sceneDefinitions[0].nodes[2].type = "overlay"
  },
  { sourceCodes: ["UNSUPPORTED_NODE_TYPE"] },
)
addInvalid(
  "invalid-quaternion-zero",
  "Quaternion de comprimento zero.",
  minimalBundle,
  (b) => {
    b.sceneDefinitions[0].nodes[2].localRotation = { x: 0, y: 0, z: 0, w: 0 }
  },
  { sourceCodes: ["DEGENERATE_QUATERNION"] },
)
addInvalid(
  "invalid-quaternion-not-normalized",
  "Quaternion com comprimento 2.",
  minimalBundle,
  (b) => {
    b.sceneDefinitions[0].nodes[2].localRotation = { x: 0, y: 0, z: 0, w: 2 }
  },
  { sourceCodes: ["QUATERNION_NOT_NORMALIZED"] },
)
addInvalid(
  "invalid-socket-missing",
  "attachToSocket com nome de socket inexistente.",
  graphPerformanceBundle,
  (b) => {
    b.sceneDefinitions[0].nodes[4].attachToSocket.socketName = "ghost-socket"
  },
  { sourceCodes: ["MISSING_SOCKET"] },
)
addInvalid(
  "invalid-socket-and-parent-conflict",
  "Nó com parentId (não raiz) e attachToSocket ao mesmo tempo.",
  graphPerformanceBundle,
  (b) => {
    b.sceneDefinitions[0].nodes[4].parentId = "stage"
  },
  { sourceCodes: ["SOCKET_AND_PARENT_CONFLICT"] },
)

// --------------------------------------------------------------------------------------------
// Inválidos: instâncias, performance e shots
// --------------------------------------------------------------------------------------------

addInvalid(
  "invalid-instance-unknown-definition",
  "Instância referencia definição de cena inexistente.",
  minimalBundle,
  (b) => {
    b.sceneInstances[0].sceneDefinitionId = "no-such-scene"
  },
  { sourceCodes: ["UNKNOWN_SCENE_DEFINITION"] },
)
addInvalid(
  "invalid-performance-track-missing",
  "Instância lista uma performanceTrack inexistente.",
  graphPerformanceBundle,
  (b) => {
    b.sceneInstances[0].performanceTrackIds = ["ghost-track"]
  },
  { sourceCodes: ["PERFORMANCE_TRACK_REF_MISSING"] },
)
addInvalid(
  "invalid-performance-node-missing",
  "Canal de performance mira um nó inexistente.",
  graphPerformanceBundle,
  (b) => {
    b.performanceTracks[0].channels[0].nodeId = "ghost-node"
  },
  { sourceCodes: ["PERFORMANCE_NODE_MISSING"] },
)
addInvalid(
  "invalid-performance-duplicate-channel-id",
  "channelId repetido na mesma trilha (mensagem sem código estável).",
  graphPerformanceBundle,
  (b) => {
    b.performanceTracks[0].channels[1].channelId = "arm-swing"
  },
  { messageIncludes: ["Duplicate performance channelId"] },
)
addInvalid(
  "invalid-shot-unknown-instance",
  "Shot referencia instância inexistente.",
  minimalBundle,
  (b) => {
    b.shots[0].sceneInstanceId = "no-such-instance"
  },
  { sourceCodes: ["SHOT_UNKNOWN_INSTANCE"] },
)
addInvalid(
  "invalid-shot-unknown-camera-clip",
  "Shot referencia clipe de câmera inexistente.",
  minimalBundle,
  (b) => {
    b.shots[0].cameraClipId = "no-such-clip"
  },
  { sourceCodes: ["SHOT_UNKNOWN_CAMERA_CLIP"] },
)
addInvalid(
  "invalid-shot-exceeds-instance",
  "Mapeamento do shot passa da duração da instância.",
  minimalBundle,
  (b) => {
    b.sceneInstances[0].durationFrames = 80
  },
  { sourceCodes: ["SHOT_EXCEEDS_INSTANCE_DURATION"] },
)
addInvalid(
  "invalid-edit-range-start",
  "Primeiro shot não começa no frame 0.",
  minimalBundle,
  (b) => {
    b.shots[0].editRange.startFrame = 4
  },
  { sourceCodes: ["EDIT_RANGE_START"] },
)
addInvalid(
  "invalid-edit-gap",
  "Buraco de 4 frames entre os dois shots.",
  minimalBundle,
  (b) => {
    b.shots[1].editRange.startFrame = 52
  },
  { sourceCodes: ["EDIT_GAP_OR_OVERLAP"] },
)
addInvalid(
  "invalid-edit-duration-mismatch",
  "Shots cobrem 96 frames mas editDurationFrames diz 100.",
  minimalBundle,
  (b) => {
    b.editDurationFrames = 100
  },
  { sourceCodes: ["EDIT_DURATION_MISMATCH"] },
)
addInvalid(
  "invalid-duplicate-shot-id",
  "Dois shots com o mesmo id.",
  minimalBundle,
  (b) => {
    b.shots[1].id = "shot-1"
  },
  { sourceCodes: ["DUPLICATE_ID"] },
)
addInvalid(
  "invalid-composition-node-missing",
  "compositionPasses referencia nó inexistente.",
  minimalBundle,
  (b) => {
    b.compositionPasses = [
      { fragmentId: "frag-1", nodeId: "ghost-node", stage: "prePost", order: 0 },
    ]
  },
  { sourceCodes: ["COMPOSITION_NODE_MISSING"] },
)
addInvalid(
  "invalid-audio-hash-required",
  "audio.mixRelativePath sem sourcePcmHashSha256.",
  minimalBundle,
  (b) => {
    b.audio = {
      mixRelativePath: "audio/mix.wav",
      editStartSeconds: 0,
      editEndSecondsExclusive: 4,
    }
  },
  { sourceCodes: ["AUDIO_HASH_REQUIRED"] },
)
addInvalid(
  "invalid-light-parent-unknown",
  "Luz segue um nó que não existe na cena.",
  minimalBundle,
  (b) => {
    b.sceneDefinitions[0].lighting = {
      ambient: 1,
      lights: [
        {
          id: "lamp-1",
          position: vec(1, 2, 3),
          color: { r: 1, g: 0.8, b: 0.5 },
          intensity: 2,
          radius: 3,
          flicker: 0,
          parent: "ghost-node",
        },
      ],
    }
  },
  { sourceCodes: ["LIGHT_PARENT_UNKNOWN"] },
)

// --------------------------------------------------------------------------------------------
// Inválidos: câmera
// --------------------------------------------------------------------------------------------

addInvalid(
  "invalid-camera-fov-out-of-range",
  "FOV inicial fora de (0, 180).",
  minimalBundle,
  (b) => {
    b.cameraClips[0].initialFovDeg = 190
  },
  { sourceCodes: ["FOV_OUT_OF_RANGE"] },
)
addInvalid(
  "invalid-camera-direction-degenerate",
  "Alvo igual à posição: câmera sem direção.",
  minimalBundle,
  (b) => {
    b.cameraClips[0].initialTarget = clone(b.cameraClips[0].initialPosition)
  },
  { sourceCodes: ["CAMERA_DIRECTION_DEGENERATE"] },
)
addInvalid(
  "invalid-camera-unknown-easing",
  "Easing que a origem não conhece.",
  minimalBundle,
  (b) => {
    b.cameraClips[0].segments[0].easing = "bounce"
  },
  { sourceCodes: ["UNKNOWN_CAMERA_EASING"] },
)
addInvalid(
  "invalid-camera-segment-too-short",
  "Movimento (não hold) de 1 frame.",
  minimalBundle,
  (b) => {
    b.cameraClips[0].segments[0].durationFrames = 1
  },
  { sourceCodes: ["CAMERA_SEGMENT_TOO_SHORT"] },
)
addInvalid(
  "invalid-camera-key-frame-order",
  "Keyframes fora de ordem crescente.",
  cameraRichBundle,
  (b) => {
    const keys = b.cameraClips[0].segments[2].keys
    keys[0].frame = 25
    keys[1].frame = 10
  },
  { sourceCodes: ["CAMERA_KEY_FRAME_ORDER"] },
)
addInvalid(
  "invalid-camera-keys-end-short",
  "Última key antes do último frame do movimento.",
  cameraRichBundle,
  (b) => {
    b.cameraClips[0].segments[2].keys[2].frame = 30
  },
  { sourceCodes: ["CAMERA_KEYS_END_SHORT"] },
)
addInvalid(
  "invalid-camera-layer-keyframes",
  "Camada aditiva contendo keyframes (proibido).",
  cameraRichBundle,
  (b) => {
    b.cameraClips[0].layers[1].segments.push(
      keyframesSegment(10, [[9, vec(0, 1, 5), vec(0, 1, 0), 45, "linear"]]),
    )
  },
  { sourceCodes: ["CAMERA_LAYER_KEYFRAMES"] },
)
addInvalid(
  "invalid-camera-layer-from-initial",
  "Camada aditiva com movimento partindo de initial.",
  cameraRichBundle,
  (b) => {
    b.cameraClips[0].layers[1].segments[0].from = "initial"
  },
  { sourceCodes: ["CAMERA_LAYER_FROM_INITIAL"] },
)

// --------------------------------------------------------------------------------------------
// Inválidos: junções de edição (o schema aceita; `validateEditJoins` recusa)
// --------------------------------------------------------------------------------------------

addInvalid(
  "invalid-join-missing-transition",
  "Segundo shot sem transitionInto.",
  minimalBundle,
  (b) => {
    delete b.shots[1].transitionInto
  },
  { sourceCodes: ["MISSING_TRANSITION"] },
)
addInvalid(
  "invalid-join-cut-without-reason",
  "Corte cujo motivo é só espaço em branco.",
  cutBundle,
  (b) => {
    b.shots[1].transitionInto = { type: "cut", reason: "   " }
  },
  { sourceCodes: ["CUT_WITHOUT_REASON"] },
)
addInvalid(
  "invalid-join-instance-frame-discontinuity",
  "Junção contínua em que instanceStartFrame não continua o shot anterior.",
  minimalBundle,
  (b) => {
    b.shots[1].instanceStartFrame = 40
  },
  { sourceCodes: ["INSTANCE_FRAME_DISCONTINUITY"] },
)
addInvalid(
  "invalid-join-camera-clip-offset-and-jump",
  "Junção contínua que volta ao frame 0 do clipe: quebra de offset e saltos de pose.",
  minimalBundle,
  (b) => {
    b.shots[1].cameraInFrame = 0
  },
  { sourceCodes: ["CAMERA_CLIP_OFFSET_BREAK", "CONTINUOUS_CAMERA_JUMP"] },
)
addInvalid(
  "invalid-join-continuous-across-instances",
  "Junção contínua entre instâncias e clipes diferentes.",
  cutBundle,
  (b) => {
    b.shots[1].transitionInto = { type: "continuous" }
  },
  {
    sourceCodes: [
      "CONTINUOUS_INSTANCE_MISMATCH",
      "CONTINUOUS_CROSS_CLIP_UNSUPPORTED",
    ],
  },
)
addInvalid(
  "invalid-join-segment-reset",
  "Segmento no meio do clipe voltando para a pose initial.",
  minimalBundle,
  (b) => {
    b.cameraClips[0].segments[1].from = "initial"
  },
  { sourceCodes: ["CAMERA_SEGMENT_RESET"] },
)

export const buildCorpus = () => cases.map((entry) => clone(entry))
