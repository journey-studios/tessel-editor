// Harness ESTENDIDO do oráculo (K / BL018). Empacotado (esbuild) junto com as fontes REAIS da
// origem, sem copiar nem alterar nenhuma. Reexporta o harness base (`inspectBundle`,
// `evaluateBundle`, byte a byte o mesmo arquivo do corpus de 55 casos) e acrescenta só CHAMADAS a
// funções adicionais da origem:
//
//   - JOIN_*                      constantes reais de `validate/edit-joins.ts`
//   - resolveCameraAtClipFrame    poses dos dois lados de cada junção contínua
//   - fpsToRate, secondsToEditFrame, editFrameToSeconds                (`types/fps.ts`)
//   - editSecondsToFrame, editFrameToEditSeconds, maxEditSeconds,
//     editTimelineDurationSeconds, clampEditSeconds                    (`transport/audio-window.ts`)
//   - PerspectiveCamera.lookAt    do Three.js 0.187.0 FIXADO, não da origem (ver abaixo)
//
// Regra do projeto: aqui só se CHAMA e se serializa. Nenhuma regra esperada é reimplementada.
//
// LIMITE DECLARADO — matriz mundo da câmera: a origem a constrói dentro de
// `ThreeSceneRuntime.drawEditFrame` (método PRIVADO de uma classe que exige `WebGLRenderer`), então
// não há função exportada e pura para chamar sem GPU. O oráculo aplica a MESMA sequência que o
// runtime (position.set → lookAt(Vector3 alvo) → updateMatrixWorld) com a biblioteca Three.js
// fixada, sobre a pose que a origem avaliou. A prova é "a biblioteca real", não "o método da
// origem": está marcada como `source: "three-library"` nas expectativas.

import { PerspectiveCamera, Vector3 } from "three"

import { resolveCameraAtClipFrame } from "@origin/camera/resolve-camera-clip"
import {
  clampEditSeconds,
  editFrameToEditSeconds,
  editSecondsToFrame,
  editTimelineDurationSeconds,
  maxEditSeconds,
} from "@origin/transport/audio-window"
import {
  editFrameToSeconds,
  fpsToRate,
  secondsToEditFrame,
} from "@origin/types/fps"
import {
  JOIN_FOV_TOLERANCE_DEG,
  JOIN_FOV_VELOCITY_TOLERANCE_DEG_PER_FRAME,
  JOIN_POSITION_TOLERANCE_SCENE_UNITS,
  JOIN_VELOCITY_TOLERANCE_SCENE_UNITS_PER_FRAME,
} from "@origin/validate/edit-joins"

import { evaluateBundle, inspectBundle } from "../harness/entry.mjs"

export { evaluateBundle, inspectBundle }

const messageOf = (error) => (error instanceof Error ? error.message : String(error))

const attempt = (fn) => {
  try {
    return { ok: true, value: fn() }
  } catch (error) {
    return { ok: false, error: messageOf(error) }
  }
}

/** Constantes JOIN_* exatamente como o módulo real as exporta. */
export const originJoinConstants = () => ({
  JOIN_POSITION_TOLERANCE_SCENE_UNITS,
  JOIN_FOV_TOLERANCE_DEG,
  JOIN_VELOCITY_TOLERANCE_SCENE_UNITS_PER_FRAME,
  JOIN_FOV_VELOCITY_TOLERANCE_DEG_PER_FRAME,
})

/**
 * Poses da câmera dos dois lados de cada junção contínua no mesmo clipe, ordenadas como a origem
 * ordena (por `startFrame`). A escolha dos frames espelha onde `validateEditJoins` mede (último
 * frame do shot anterior e primeiro do seguinte): é seleção de PONTOS DE MEDIDA, como `probes.mjs`;
 * as poses são as que `resolveCameraAtClipFrame` devolve.
 */
export const joinSamples = (bundle) => {
  const sorted = [...bundle.shots].sort(
    (a, b) => a.editRange.startFrame - b.editRange.startFrame,
  )
  const samples = []
  for (let index = 1; index < sorted.length; index += 1) {
    const previous = sorted[index - 1]
    const next = sorted[index]
    if (next.transitionInto?.type !== "continuous") {
      continue
    }
    if (previous.cameraClipId !== next.cameraClipId) {
      continue
    }
    const length = previous.editRange.endFrameExclusive - previous.editRange.startFrame
    const endClipFrame = previous.cameraInFrame + length - 1
    const startClipFrame = next.cameraInFrame
    const sample = (clipFrame) => {
      const result = attempt(() =>
        resolveCameraAtClipFrame(bundle, previous.cameraClipId, clipFrame),
      )
      return result.ok
        ? { clipFrame, ok: true, camera: result.value }
        : { clipFrame, ok: false, error: result.error }
    }
    samples.push({
      previousShotId: previous.id,
      nextShotId: next.id,
      cameraClipId: previous.cameraClipId,
      end: sample(endClipFrame),
      start: sample(startClipFrame),
    })
  }
  return samples
}

const cameraWorldMatrix = (camera) => {
  const threeCamera = new PerspectiveCamera(camera.fovDeg, 16 / 9, 0.1, 1000)
  threeCamera.position.set(camera.position.x, camera.position.y, camera.position.z)
  threeCamera.lookAt(new Vector3(camera.target.x, camera.target.y, camera.target.z))
  threeCamera.updateMatrixWorld(true)
  return threeCamera.matrixWorld.elements.slice()
}

/** Matriz mundo da câmera em cada frame de edição JÁ avaliado pela origem (`evaluateBundle`). */
export const cameraWorldMatrices = (evaluation) =>
  evaluation.editFrames.map((frame) => {
    if (!frame.ok) {
      return { editFrame: frame.editFrame, ok: false }
    }
    return {
      editFrame: frame.editFrame,
      ok: true,
      source: "three-library",
      matrix: cameraWorldMatrix(frame.state.camera),
    }
  })

/** Avalia os mesmos frames de edição em outra ordem, num clone novo do bundle. */
export const evaluateInOrder = (bundle, frames) => {
  const fresh = structuredClone(bundle)
  const evaluated = evaluateBundle(fresh, { editFrames: frames, clipFrames: {} })
  return {
    editFrames: evaluated.editFrames,
    bundleMutated: JSON.stringify(fresh) !== JSON.stringify(bundle),
  }
}

/** Funções de tempo da origem sobre os probes de segundos já expandidos pelo gerador. */
export const evaluateTime = ({ fps, editDurationFrames, frames, seconds }) => ({
  rate: fpsToRate(fps),
  timelineDurationSeconds: editTimelineDurationSeconds(editDurationFrames, fps),
  maxEditSeconds: maxEditSeconds(editDurationFrames, fps),
  frames: frames.map((frame) => ({
    frame,
    seconds: editFrameToSeconds(frame, fps),
    editSeconds: editFrameToEditSeconds(frame, fps),
  })),
  seconds: seconds.map((probe) => ({
    label: probe.label,
    frame: probe.frame,
    exact: probe.exact,
    seconds: probe.seconds,
    floorFrame: secondsToEditFrame(probe.seconds, fps),
    clampedFrame: editSecondsToFrame(probe.seconds, fps, editDurationFrames),
    clampedSeconds: clampEditSeconds(probe.seconds, editDurationFrames, fps),
  })),
})
