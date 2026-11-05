// Regenerates this directory: synthetic bundle variants (written from `../directed_scene_fixtures/
// layered_scene.json`, no private media) and what the origin's REAL functions answer for each.
//
// The origin is executed through the migration oracle's harness (`scripts/migration-oracle`), which
// bundles `packages/scene-engine` of the pinned snapshot without altering it and never writes to
// it. Nothing here reimplements a rule: the script builds inputs and records the origin's output.
//
//   node scripts/migration-oracle/oracle.mjs setup            # once; deps live outside the repo
//   node crates/core/tests/directed_scene_temporal_fixtures/generate.mjs
//
// Used by `crates/core/tests/directed_scene_temporal.rs` (test `variants_match_the_origin_*`).

import { mkdirSync, readFileSync, writeFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

const here = path.dirname(fileURLToPath(import.meta.url))
const repo = path.resolve(here, "../../../..")
const oracle = path.join(repo, "scripts", "migration-oracle", "lib")
const { buildHarness, pinnedBlobCheck, installedToolVersions } = await import(path.join(oracle, "harness.mjs"))
const { resolveConfig, PINNED_COMMIT, sha256Of } = await import(path.join(oracle, "config.mjs"))

const base = JSON.parse(readFileSync(path.join(here, "..", "directed_scene_fixtures", "layered_scene.json"), "utf8"))
const clone = (value) => JSON.parse(JSON.stringify(value))

const dolly = (frames, distance, from) => ({
  recipe: "dollyIn", durationFrames: frames, from, distanceSceneUnits: distance, easing: "linear",
})
const zoomIn = (frames, delta, from) => ({
  recipe: "zoomIn", durationFrames: frames, from, fovDeltaDeg: delta, easing: "linear",
})
const hold = (frames, from) => ({ recipe: "hold", durationFrames: frames, from })

/** The layered fixture with one layer-less clip made of `segments`. */
const withClip = (segments) => {
  const bundle = clone(base)
  bundle.cameraClips[0].segments = segments
  delete bundle.cameraClips[0].layers
  return bundle
}

const variants = {
  // The fixture as it is: structurally sound, rejected by the origin for a velocity jump.
  "layered-as-is": clone(base),
  "smooth-single-dolly": withClip([dolly(48, 6, "initial")]),
  "resting-hold": withClip([hold(48, "initial")]),
  "velocity-jump": withClip([dolly(24, 5, "initial"), dolly(24, 5, "previousEnd")]),
  "fov-velocity-jump": withClip([zoomIn(24, 20, "initial"), zoomIn(24, 20, "previousEnd")]),
  "reset-position-target-jump": withClip([dolly(24, 5, "initial"), hold(24, "initial")]),
  "reset-fov-jump": withClip([zoomIn(24, 20, "initial"), hold(24, "initial")]),
  "stacked-fov-out-of-range": (() => {
    const bundle = clone(base)
    bundle.cameraClips[0].segments = [
      { recipe: "zoomOut", durationFrames: 24, from: "initial", fovDeltaDeg: 120, easing: "linear" },
    ]
    bundle.cameraClips[0].layers = [{ segments: [
      { recipe: "zoomOut", durationFrames: 48, from: "previousEnd", fovDeltaDeg: 20, easing: "linear" },
    ] }]
    return bundle
  })(),
  "cut-after-jumping-clip": (() => {
    const bundle = withClip([dolly(24, 5, "initial"), hold(24, "initial")])
    bundle.shots[1].transitionInto = { type: "cut", reason: "new angle" }
    return bundle
  })(),
}

const config = resolveConfig({})
const { module, sources, sourceRoot } = await buildHarness(config)
const provenance = pinnedBlobCheck(sourceRoot, PINNED_COMMIT, sources)
if (!provenance.available || provenance.mismatches.length !== 0) {
  throw new Error("Cannot generate temporal fixtures: source does not match the pinned commit")
}

const editFrames = [-1, ...Array.from({ length: 48 }, (_, i) => i), 48]
const clipFrames = [-1, ...Array.from({ length: 52 }, (_, i) => i)]

const codes = (issues) => (issues ?? []).map((issue) => issue.code).sort()
const origin = {}
for (const [name, bundle] of Object.entries(variants)) {
  writeFileSync(path.join(here, `${name}.bundle.json`), `${JSON.stringify(bundle, null, 2)}\n`)
  const { parsed, validation } = module.inspectBundle(bundle)
  const entry = {
    schemaAccepted: validation.schema.success,
    // The origin runs its integrity rules inside the schema refinement too; their codes land here.
    schemaIssueCodes: validation.schema.issues.map((issue) => issue.sourceCode).filter(Boolean).sort(),
    accepted: validation.accepted,
    editJoinCodes: codes(validation.editJoins),
    integrityCodes: codes(validation.integrityOnParsed),
    finalizeMessage: validation.finalize?.ok === false ? validation.finalize.message : null,
  }
  if (name === "layered-as-is" && parsed) {
    // The camera of every edit frame, to compare the layered clip (24000/1001 bundle) with the origin.
    const evaluation = module.evaluateBundle(parsed, {
      editFrames,
      clipFrames: Object.fromEntries(parsed.cameraClips.map((clip) => [clip.id, clipFrames])),
    })
    entry.editFrames = evaluation.editFrames.map((frame) =>
      frame.ok
        ? { editFrame: frame.editFrame, ok: true, state: frame.state }
        : { editFrame: frame.editFrame, ok: false, error: frame.error },
    )
  }
  origin[name] = entry
}

const node = process.version
const sourceCommit = provenance.repoHead
const result = {
  formatVersion: 1,
  note: "Answers of the origin's real functions (finalizeBundle, validateEditJoins, validateBundleIntegrity, evaluateEditFrame) for the synthetic variants in this directory. Regenerate with generate.mjs.",
  originCommit: sourceCommit,
  pinnedCommit: PINNED_COMMIT,
  node,
  executedSources: sources.length,
  sourceFiles: sources,
  packageTreeAtPinnedCommit: provenance.packageTreeAtCommit,
  toolVersions: installedToolVersions(config.depsDir),
  generatorSha256: sha256Of(readFileSync(fileURLToPath(import.meta.url))),
  baseFixtureSha256: sha256Of(readFileSync(path.join(here, "..", "directed_scene_fixtures", "layered_scene.json"))),
  bundleHashes: Object.fromEntries(Object.keys(variants).map((name) => [
    `${name}.bundle.json`, sha256Of(readFileSync(path.join(here, `${name}.bundle.json`))),
  ])),
  variants: origin,
}
mkdirSync(here, { recursive: true })
writeFileSync(path.join(here, "origin.json"), `${JSON.stringify(result, null, 2)}\n`)
console.log(`wrote ${Object.keys(variants).length} variants; origin ${sourceCommit}, ${sources.length} source files, node ${node}`)
