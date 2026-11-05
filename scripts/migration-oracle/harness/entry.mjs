// Harness do oráculo. Este arquivo é empacotado (esbuild) JUNTO com as fontes REAIS da origem,
// sem copiar nem alterar nenhuma delas: `@origin/*` aponta para `<sourceRoot>/src/*` e `zod`/`three`
// resolvem para as versões fixadas no lockfile deste diretório.
//
// Regra do projeto: aqui só se CHAMA a origem e se serializa o resultado. Nenhuma regra esperada
// é reimplementada (ver docs/features/migration-oracle.md).

import { finalizeBundle } from "@origin/edit/finalize-bundle"
import { evaluateEditFrame } from "@origin/evaluator/evaluate-edit-frame"
import {
  getCameraClipDurationFrames,
  mainTrackPoses,
  resolveCameraAtClipFrame,
} from "@origin/camera/resolve-camera-clip"
import {
  computeWorldMatrices,
  resolveParentLinks,
  validateSceneGraphStructure,
} from "@origin/scene/scene-graph"
import { renderBundleSchema } from "@origin/schema/render-bundle.schema"
import { validateBundleIntegrity } from "@origin/validate/bundle-integrity"
import { validateEditJoins } from "@origin/validate/edit-joins"

const messageOf = (error) => (error instanceof Error ? error.message : String(error))

/** Executa `fn` e grava ou o resultado ou a mensagem de erro REAL lançada pela origem. */
const attempt = (fn) => {
  try {
    return { ok: true, value: fn() }
  } catch (error) {
    return { ok: false, error: messageOf(error) }
  }
}

// O schema da origem embute o código estável na mensagem ("EDIT_GAP_OR_OVERLAP: Shot ..."). O
// campo `sourceCode` só separa esse prefixo para consulta; a mensagem original segue intacta.
const SOURCE_CODE_PREFIX = /^([A-Z][A-Z0-9_]*): /

const describeZodIssue = (issue) => {
  const prefix = issue.code === "custom" ? SOURCE_CODE_PREFIX.exec(issue.message) : null
  return {
    path: issue.path.map(String).join("."),
    zodCode: issue.code,
    sourceCode: prefix ? prefix[1] : null,
    message: issue.message,
  }
}

const isRecord = (value) => typeof value === "object" && value !== null

/** `validateSceneGraphStructure` chamada direto no JSON cru, sem passar pelo schema. */
const sceneGraphStructure = (raw) => {
  if (!(isRecord(raw) && Array.isArray(raw.sceneDefinitions))) {
    return []
  }
  return raw.sceneDefinitions.map((definition) => {
    const definitionId = isRecord(definition) && typeof definition.id === "string" ? definition.id : null
    if (!(isRecord(definition) && Array.isArray(definition.nodes))) {
      return { definitionId, skipped: "nodes is not an array" }
    }
    const result = attempt(() => validateSceneGraphStructure(definition))
    if (!result.ok) {
      return { definitionId, threw: result.error }
    }
    return { definitionId, issues: result.value }
  })
}

/**
 * Veredito da origem sobre um bundle cru. Devolve também o bundle parseado (não serializável) para
 * `evaluateBundle`; ele é `null` quando o schema recusa.
 */
export const inspectBundle = (raw) => {
  const parsedResult = renderBundleSchema.safeParse(raw)
  const schema = {
    success: parsedResult.success,
    issues: parsedResult.success ? [] : parsedResult.error.issues.map(describeZodIssue),
  }
  const structure = sceneGraphStructure(raw)
  if (!parsedResult.success) {
    return {
      parsed: null,
      validation: {
        schema,
        sceneGraphStructure: structure,
        integrityOnParsed: null,
        editJoins: null,
        finalize: null,
        accepted: false,
      },
    }
  }
  const parsed = parsedResult.data
  const finalized = finalizeBundle(parsed)
  const finalize = finalized.ok ? { ok: true } : { ok: false, message: finalized.message }
  return {
    parsed,
    validation: {
      schema,
      sceneGraphStructure: structure,
      integrityOnParsed: validateBundleIntegrity(parsed),
      editJoins: validateEditJoins(parsed),
      finalize,
      accepted: finalized.ok,
    },
  }
}

const definitionOfInstance = (bundle, instanceId) => {
  const instance = bundle.sceneInstances.find((candidate) => candidate.id === instanceId)
  return bundle.sceneDefinitions.find(
    (definition) => definition.id === instance?.sceneDefinitionId,
  )
}

const evaluateEditFrameWithWorld = (bundle, editFrame) => {
  const result = attempt(() => {
    const state = evaluateEditFrame(bundle, editFrame)
    const definition = definitionOfInstance(bundle, state.instanceId)
    return { state, worldMatrices: computeWorldMatrices(definition, state.nodes) }
  })
  if (!result.ok) {
    return { editFrame, ok: false, error: result.error }
  }
  return { editFrame, ok: true, ...result.value }
}

const linksByDefinition = (bundle) =>
  Object.fromEntries(
    bundle.sceneDefinitions.map((definition) => [
      definition.id,
      Object.fromEntries(resolveParentLinks(definition)),
    ]),
  )

const evaluateClip = (bundle, clip, clipFrames) => ({
  clipId: clip.id,
  durationFrames: getCameraClipDurationFrames(bundle, clip.id),
  mainTrackPoses: mainTrackPoses(clip),
  frames: clipFrames.map((clipFrame) => {
    const result = attempt(() => resolveCameraAtClipFrame(bundle, clip.id, clipFrame))
    if (!result.ok) {
      return { clipFrame, ok: false, error: result.error }
    }
    return { clipFrame, ok: true, camera: result.value }
  }),
})

/** Avalia um bundle ACEITO nos frames escolhidos por `lib/probes.mjs`. */
export const evaluateBundle = (bundle, probes) => ({
  parentLinks: linksByDefinition(bundle),
  editFrames: probes.editFrames.map((editFrame) =>
    evaluateEditFrameWithWorld(bundle, editFrame),
  ),
  cameraClips: bundle.cameraClips.map((clip) =>
    evaluateClip(bundle, clip, probes.clipFrames[clip.id] ?? []),
  ),
})
