// Regenerates this directory: synthetic bundles for node animation (performance channels) and what
// the origin's REAL functions answer for each. Used by `crates/core/tests/directed_scene_animation.rs`.
//
// The origin is executed through the migration oracle's harness (`scripts/migration-oracle`), which
// bundles `packages/scene-engine` of the pinned snapshot without altering it and never writes to
// it. Nothing here reimplements a rule: this script only builds inputs (from the oracle's own valid
// case `valid-graph-sockets-performance`, mutated, no private media) and records the origin's
// output for them: `renderBundleSchema` (verdict and issue paths), `finalizeBundle`, and
// `evaluateEditFrame` + `computeWorldMatrices` for the chosen edit frames.
//
//   node scripts/migration-oracle/oracle.mjs setup     # once; deps live outside the repo
//   node scripts/migration-oracle/oracle.mjs verify    # the pinned sources must be intact
//   node crates/core/tests/directed_scene_animation_fixtures/generate.mjs          # rewrite
//   node crates/core/tests/directed_scene_animation_fixtures/generate.mjs verify   # write nothing
//
// `verify` re-runs the origin and fails if any file on disk differs from what it would write
// (the Node version in `origin.json` is the one field it does not compare: it is a record, and
// the doubles do not depend on it).
//
// Provenance is checked BEFORE anything is written: the executed sources must be the blobs of the
// pinned commit. The numeric tolerances the Rust test applies are fixed in that test's header and in
// `docs/features/scene-animation.md`, not here: this file records exact doubles.

import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

const here = path.dirname(fileURLToPath(import.meta.url))
const repo = path.resolve(here, "../../../..")
const oracle = path.join(repo, "scripts", "migration-oracle", "lib")
const { buildHarness, pinnedBlobCheck, installedToolVersions } = await import(path.join(oracle, "harness.mjs"))
const { resolveConfig, PINNED_COMMIT, sha256Of } = await import(path.join(oracle, "config.mjs"))

const BASE_FILE = path.join(repo, "docs", "migration", "fixtures", "bundles", "valid-graph-sockets-performance.bundle.json")
const base = JSON.parse(readFileSync(BASE_FILE, "utf8"))
const clone = (value) => JSON.parse(JSON.stringify(value))
const v3 = (x, y, z) => ({ x, y, z })

// ---- inputs --------------------------------------------------------------------------------------

/** Unit quaternion of an XYZ Euler rotation in degrees (the textbook formula, used only to AUTHOR input). */
const quatOfEuler = (xd, yd, zd) => {
  const [x, y, z] = [xd, yd, zd].map((d) => (d * Math.PI) / 180)
  const [c1, c2, c3] = [Math.cos(x / 2), Math.cos(y / 2), Math.cos(z / 2)]
  const [s1, s2, s3] = [Math.sin(x / 2), Math.sin(y / 2), Math.sin(z / 2)]
  return {
    x: s1 * c2 * c3 + c1 * s2 * s3,
    y: c1 * s2 * c3 - s1 * c2 * s3,
    z: c1 * c2 * s3 + s1 * s2 * c3,
    w: c1 * c2 * c3 - s1 * s2 * s3,
  }
}

const track = (id, channels, instanceId = "inst-a") => ({ id, instanceId, channels })
const cycle = (channelId, nodeId, property, periodFrames, amplitude, axis, phaseOffsetFrames) => ({
  kind: "cycle", channelId, nodeId, property, periodFrames, amplitude, axis, phaseOffsetFrames,
})
const event = (channelId, nodeId, property, startInstanceFrame, durationFrames, delta) => ({
  kind: "event", channelId, nodeId, property, startInstanceFrame, durationFrames, delta, holdFinal: true,
})
const fixed = (channelId, nodeId, property, value) => ({ kind: "static", channelId, nodeId, property, value })

/** The oracle's case with its performance replaced by `tracks` (all listed by inst-a). */
const withTracks = (tracks, listed = tracks.map((t) => t.id)) => {
  const bundle = clone(base)
  bundle.performanceTracks = tracks
  bundle.sceneInstances[0].performanceTrackIds = listed
  return bundle
}

const nodeOf = (bundle, id) => bundle.sceneDefinitions[0].nodes.find((n) => n.id === id)
const range = (from, to) => Array.from({ length: to - from + 1 }, (_, i) => from + i)

/** Two instances of the same scene, three shots that jump around in both (cuts, offsets). */
const twoInstances = () => {
  const bundle = clone(base)
  bundle.editDurationFrames = 70
  bundle.sceneInstances = [
    { id: "inst-a", sceneDefinitionId: "scene-rig", durationFrames: 60, performanceTrackIds: ["perf-a"] },
    { id: "inst-b", sceneDefinitionId: "scene-rig", durationFrames: 60, performanceTrackIds: ["perf-b"] },
  ]
  bundle.performanceTracks = [
    track("perf-a", [
      cycle("a-arm", "arm", "localRotation", 24, 20, v3(0, 0, 1), 3),
      event("a-stage", "stage", "localPosition", 30, 20, v3(0, 0.5, 0)),
    ], "inst-a"),
    track("perf-b", [
      cycle("b-arm", "arm", "localRotation", 10, 35, v3(1, 0, 1), 0),
      fixed("b-pen", "pen", "localScale", v3(2, 0.5, 1)),
    ], "inst-b"),
  ]
  bundle.cameraClips = [{
    id: "cam-rig", initialPosition: v3(0, 1, 7), initialTarget: v3(0, 1, 0), initialFovDeg: 40,
    segments: [{ recipe: "hold", durationFrames: 80, from: "initial" }],
  }]
  bundle.shots = [
    { id: "shot-1", sceneInstanceId: "inst-a", editRange: { startFrame: 0, endFrameExclusive: 20 }, instanceStartFrame: 10, cameraClipId: "cam-rig", cameraInFrame: 0, intent: "first, mid-instance" },
    { id: "shot-2", sceneInstanceId: "inst-b", editRange: { startFrame: 20, endFrameExclusive: 50 }, instanceStartFrame: 25, cameraClipId: "cam-rig", cameraInFrame: 4, intent: "other instance", transitionInto: { type: "cut", reason: "to the other instance" } },
    { id: "shot-3", sceneInstanceId: "inst-a", editRange: { startFrame: 50, endFrameExclusive: 70 }, instanceStartFrame: 40, cameraClipId: "cam-rig", cameraInFrame: 50, intent: "back, later in the instance", transitionInto: { type: "cut", reason: "back to the first instance" } },
  ]
  return bundle
}

const gimbal = () => {
  const bundle = withTracks([track("perf-1", [
    cycle("arm-swing", "arm", "localRotation", 16, 25, v3(0, 1, 0), 0),
    fixed("stage-turn", "stage", "localRotation", v3(0, 20, 0)),
  ])])
  // Authored rotations at and around the gimbal-lock threshold of the XYZ Euler decomposition
  // (|m13| >= 0.9999999 is ~89.974 degrees), a half turn about X and a mixed one.
  nodeOf(bundle, "stage").localRotation = quatOfEuler(10, 89.9, -20)
  nodeOf(bundle, "arm").localRotation = quatOfEuler(30, -89.99, 15)
  nodeOf(bundle, "pen").localRotation = { x: 1, y: 0, z: 0, w: 0 }
  nodeOf(bundle, "backdrop").localRotation = quatOfEuler(170, 10, -170)
  return bundle
}

const valid = {
  // The oracle's own case, unchanged: the baseline every other variant is a mutation of.
  "baseline-oracle-case": { bundle: clone(base), frames: [-1, ...range(0, 47), 48] },

  "cycle-edges": {
    bundle: withTracks([track("perf-1", [
      cycle("c-period-1", "arm", "localRotation", 1, 20, v3(0, 0, 1), 0),
      cycle("c-period-2", "stage", "localPosition", 2, -0.75, v3(1, 0.5, -1), 1),
      cycle("c-offset-over-period", "pen", "localRotation", 7, 33.3, v3(1, 1, 1), 100),
      cycle("c-zero-amplitude", "backdrop", "localPosition", 48, 0, v3(1, 2, 3), 5),
      cycle("c-position-on-arm", "arm", "localPosition", 5, 0.3, v3(0, 1, 0), 4),
      cycle("c-whole-instance", "root", "localRotation", 48, 90, v3(0, 1, 0), 0),
    ])]),
    frames: [-1, 0, 1, 2, 3, 4, 5, 6, 7, 13, 23, 24, 25, 46, 47, 48],
  },

  "event-edges": {
    bundle: withTracks([track("perf-1", [
      event("e-one-frame", "stage", "localPosition", 0, 1, v3(0, 0.5, 0)),
      event("e-middle", "arm", "localRotation", 10, 20, v3(-15, 30, 45)),
      event("e-starts-on-last-frame", "pen", "localPosition", 47, 5, v3(1, 1, 1)),
      event("e-never-starts", "pen", "localRotation", 60, 4, v3(9, 9, 9)),
      event("e-very-long", "arm", "localPosition", 3, 1000, v3(100, 0, -50)),
      event("e-negative-delta", "backdrop", "localPosition", 20, 4, v3(-2, -4, -8)),
    ])]),
    frames: [-1, 0, 1, 2, 3, 9, 10, 11, 19, 20, 21, 23, 24, 29, 30, 31, 46, 47, 48],
  },

  "static-and-combinations": {
    bundle: (() => {
      const bundle = withTracks([track("perf-1", [
        fixed("s-arm-rotation", "arm", "localRotation", v3(10, -20, 30)),
        fixed("s-stage-position", "stage", "localPosition", v3(0.25, -0.5, 2)),
        fixed("s-stage-scale", "stage", "localScale", v3(0.5, 2, -1)),
        fixed("s-arm-scale-zero", "arm", "localScale", v3(0, 0, 0)),
        fixed("s-backdrop-rotation", "backdrop", "localRotation", v3(200, -250, 360)),
        // The three properties of one node, three kinds of channel.
        cycle("p-rotation", "pen", "localRotation", 12, 15, v3(0, 0, 1), 2),
        event("p-position", "pen", "localPosition", 6, 12, v3(0.5, 0, 0.25)),
        fixed("p-scale", "pen", "localScale", v3(1, 1, 1)),
      ])])
      return bundle
    })(),
    frames: [-1, 0, 1, 6, 11, 17, 18, 47, 48],
  },

  "tracks-allowlist-and-order": {
    // Both tracks are listed, in the opposite order to the bundle: the origin follows the bundle's
    // track order. `perf-unlisted` belongs to the instance but is not listed, so it is ignored
    // even though it writes the same property as a listed channel.
    bundle: (() => {
      const bundle = withTracks([
        track("perf-a", [cycle("a-arm", "arm", "localRotation", 24, 20, v3(0, 0, 1), 3)]),
        track("perf-b", [event("b-stage", "stage", "localPosition", 12, 24, v3(0, 0.5, 0))]),
        track("perf-unlisted", [fixed("u-arm", "arm", "localRotation", v3(99, 99, 99)), fixed("u-stage", "stage", "localPosition", v3(7, 7, 7))]),
      ], ["perf-b", "perf-a"])
      return bundle
    })(),
    frames: [0, 5, 12, 24, 36, 47],
  },

  "tracks-all-unlisted": {
    bundle: withTracks([track("perf-1", [cycle("arm-swing", "arm", "localRotation", 24, 20, v3(0, 0, 1), 3)])], []),
    frames: [0, 10, 47],
  },

  "writer-conflict-same-track": {
    bundle: withTracks([track("perf-1", [
      cycle("arm-swing", "arm", "localRotation", 24, 20, v3(0, 0, 1), 0),
      fixed("arm-fixed", "arm", "localRotation", v3(5, 5, 5)),
    ])]),
    frames: [-1, 0, 1, 47, 48],
  },

  "writer-conflict-two-tracks": {
    bundle: withTracks([
      track("perf-a", [event("a-stage", "stage", "localPosition", 0, 10, v3(0, 1, 0))]),
      track("perf-b", [event("b-stage", "stage", "localPosition", 5, 10, v3(1, 0, 0))]),
    ]),
    frames: [0, 20],
  },

  // The same node and property are written in two different instances: no conflict.
  "two-instances-offsets-seek": {
    bundle: twoInstances(),
    // Out of order, repeated, around each cut and past both ends: every answer is independent.
    frames: [69, 0, 19, 20, 21, 49, 50, 35, 10, -1, 70, 33, 69, 0, 51, 20, 34],
  },

  "gimbal-and-large-rotations": { bundle: gimbal(), frames: [-1, 0, 4, 8, 12, 16, 31, 47, 48] },

  // Keys that the origin's schema does not declare are dropped by zod; the evaluation must not move.
  "unknown-fields-on-channels": {
    bundle: (() => {
      const bundle = clone(base)
      const [swing, lift, scale] = bundle.performanceTracks[0].channels
      swing.label = "swing, note kept"
      swing.delta = v3(9, 9, 9) // a field of the OTHER kind: ignored by a cycle
      lift.value = v3(5, 5, 5)
      scale.holdFinal = true
      bundle.performanceTracks[0].comment = "extra field on a track"
      return bundle
    })(),
    frames: [0, 6, 12, 30, 47],
  },
}

// Payloads the origin's schema refuses. Each changes one thing of a sound channel.
const broken = (mutate) => {
  const bundle = clone(base)
  mutate(bundle.performanceTracks[0].channels)
  return bundle
}
const invalid = {
  "invalid-cycle-period-missing": broken(([c]) => delete c.periodFrames),
  "invalid-cycle-period-zero": broken(([c]) => { c.periodFrames = 0 }),
  "invalid-cycle-period-fractional": broken(([c]) => { c.periodFrames = 24.5 }),
  "invalid-cycle-phase-negative": broken(([c]) => { c.phaseOffsetFrames = -1 }),
  "invalid-cycle-property-scale": broken(([c]) => { c.property = "localScale" }),
  "invalid-cycle-amplitude-string": broken(([c]) => { c.amplitude = "20" }),
  "invalid-cycle-axis-incomplete": broken(([c]) => { delete c.axis.z }),
  "invalid-event-hold-final-false": broken(([, e]) => { e.holdFinal = false }),
  "invalid-event-hold-final-missing": broken(([, e]) => delete e.holdFinal),
  "invalid-event-duration-zero": broken(([, e]) => { e.durationFrames = 0 }),
  "invalid-event-start-negative": broken(([, e]) => { e.startInstanceFrame = -3 }),
  "invalid-event-property-scale": broken(([, e]) => { e.property = "localScale" }),
  "invalid-event-delta-array": broken(([, e]) => { e.delta = [0, 0.5, 0] }),
  "invalid-static-value-missing": broken(([, , s]) => delete s.value),
  "invalid-static-property-unknown": broken(([, , s]) => { s.property = "localSkew" }),
  "invalid-channel-kind-unknown": broken(([c]) => { c.kind = "wobble" }),
  "invalid-channel-id-empty": broken(([c]) => { c.channelId = "" }),
  "invalid-channel-not-object": broken((channels) => { channels[0] = "cycle" }),
  "invalid-track-without-channels": broken((channels) => { channels.length = 0 }),
  // Two broken channels in one bundle: the origin reports both, so must the importer.
  "invalid-two-channels-broken": broken(([c, e]) => { c.periodFrames = 0; e.durationFrames = -5 }),
}

// ---- run the origin ------------------------------------------------------------------------------

const config = resolveConfig({})
const { module, sources, sourceRoot } = await buildHarness(config)
const provenance = pinnedBlobCheck(sourceRoot, PINNED_COMMIT, sources)
if (!provenance.available || provenance.mismatches.length !== 0) {
  throw new Error(`Cannot generate animation fixtures: source does not match the pinned commit (${JSON.stringify(provenance)})`)
}

const codes = (issues) => (issues ?? []).map((issue) => issue.code).sort()
const nonFinite = (_key, value) => (typeof value === "number" && !Number.isFinite(value) ? { $nonFinite: String(value) } : value)

const inspect = (name, bundle, frames) => {
  const { parsed, validation } = module.inspectBundle(bundle)
  const entry = {
    schemaAccepted: validation.schema.success,
    schemaIssues: validation.schema.issues,
    accepted: validation.accepted,
    finalizeMessage: validation.finalize?.ok === false ? validation.finalize.message : null,
    editJoinCodes: codes(validation.editJoins),
    integrityCodes: codes(validation.integrityOnParsed),
  }
  if (parsed && frames) {
    const evaluation = module.evaluateBundle(parsed, {
      editFrames: frames,
      clipFrames: Object.fromEntries(parsed.cameraClips.map((clip) => [clip.id, []])),
    })
    entry.editFrames = evaluation.editFrames
  }
  return entry
}

const VERIFY = process.argv[2] === "verify"
if (process.argv[2] !== undefined && !VERIFY) throw new Error(`unknown argument ${process.argv[2]}; use \`verify\` or nothing`)
const dir = (...parts) => path.join(here, ...parts)
const divergences = []
/** Writes `text`, or in verify mode only records whether the file on disk is exactly it. */
const emit = (relative, text) => {
  if (!VERIFY) {
    writeFileSync(dir(relative), text)
    return
  }
  let onDisk = null
  try {
    onDisk = readFileSync(dir(relative), "utf8")
  } catch {}
  if (onDisk !== text) divergences.push(relative)
}
if (!VERIFY) {
  rmSync(dir("bundles"), { recursive: true, force: true })
  mkdirSync(dir("bundles"), { recursive: true })
}

const variants = {}
const bundleHashes = {}
const write = (name, bundle) => {
  const text = `${JSON.stringify(bundle, null, 2)}\n`
  emit(`bundles/${name}.bundle.json`, text)
  bundleHashes[`bundles/${name}.bundle.json`] = sha256Of(text)
}
for (const [name, { bundle, frames }] of Object.entries(valid)) {
  write(name, bundle)
  variants[name] = { kind: "valid", frames, ...inspect(name, bundle, frames) }
}
for (const [name, bundle] of Object.entries(invalid)) {
  write(name, bundle)
  variants[name] = { kind: "invalid", ...inspect(name, bundle, null) }
}

// The intent of every variant is checked here, so a case that stopped exercising what its name says
// fails the generation instead of silently becoming a different test.
const expectAccepted = (name, accepted) => {
  if (variants[name].schemaAccepted !== accepted) {
    throw new Error(`INTENT ${name}: schemaAccepted=${variants[name].schemaAccepted}, expected ${accepted}`)
  }
}
for (const name of Object.keys(valid)) expectAccepted(name, true)
for (const name of Object.keys(invalid)) expectAccepted(name, false)
for (const name of ["writer-conflict-same-track", "writer-conflict-two-tracks"]) {
  const failing = variants[name].editFrames.filter((frame) => !frame.ok && frame.error.startsWith("Multiple performance channels write"))
  if (failing.length === 0) throw new Error(`INTENT ${name}: the origin raised no writer conflict`)
}
for (const name of ["baseline-oracle-case", "cycle-edges", "event-edges", "static-and-combinations", "tracks-allowlist-and-order", "two-instances-offsets-seek", "gimbal-and-large-rotations", "unknown-fields-on-channels"]) {
  if (!variants[name].editFrames.some((frame) => frame.ok)) throw new Error(`INTENT ${name}: no frame evaluated`)
}
if (variants["tracks-allowlist-and-order"].editFrames.some((frame) => frame.ok && frame.state.performanceOverrides.some((o) => o.value.x === 99 || o.value.x === 7))) {
  throw new Error("INTENT tracks-allowlist-and-order: an unlisted track was applied by the origin")
}

const result = {
  formatVersion: 1,
  note: "Answers of the origin's real functions (renderBundleSchema, finalizeBundle, evaluateEditFrame, computeWorldMatrices) for the synthetic variants in bundles/. Regenerate with generate.mjs.",
  originCommit: provenance.repoHead,
  pinnedCommit: PINNED_COMMIT,
  node: process.version,
  executedSources: sources.length,
  sourceFiles: sources,
  packageTreeAtPinnedCommit: provenance.packageTreeAtCommit,
  toolVersions: installedToolVersions(config.depsDir),
  generatorSha256: sha256Of(readFileSync(fileURLToPath(import.meta.url))),
  baseCase: "valid-graph-sockets-performance",
  baseBundleSha256: sha256Of(readFileSync(BASE_FILE)),
  bundleHashes,
  variants,
}

/** Pretty down to `depth`, compact below it: exact doubles, but one line per evaluated frame. */
const render = (value, depth, indent = "") => {
  if (depth === 0 || typeof value !== "object" || value === null) {
    return JSON.stringify(value, nonFinite)
  }
  const inner = `${indent}  `
  if (Array.isArray(value)) {
    if (value.length === 0) return "[]"
    return `[\n${value.map((item) => `${inner}${render(item, depth - 1, inner)}`).join(",\n")}\n${indent}]`
  }
  const entries = Object.entries(value)
  if (entries.length === 0) return "{}"
  return `{\n${entries.map(([key, item]) => `${inner}${JSON.stringify(key)}: ${render(item, depth - 1, inner)}`).join(",\n")}\n${indent}}`
}
// result (4) > variants (3) > one variant (2) > editFrames (1) > a frame (0): a frame stays on one line.
if (VERIFY) {
  // The recorded Node version is not an answer of the origin: compare everything else.
  let recorded = ""
  try {
    recorded = readFileSync(dir("origin.json"), "utf8")
  } catch {}
  const withoutNode = (text) => text.replace(/^ {2}"node": "[^"]*",\n/m, "")
  emit("origin.json", withoutNode(recorded) === withoutNode(`${render(result, 4)}\n`) ? recorded : `${render(result, 4)}\n`)
  if (divergences.length > 0) {
    console.error(`verify FAILED: ${divergences.length} file(s) differ from what the origin produces now:\n  ${divergences.join("\n  ")}`)
    process.exit(1)
  }
  console.log(`verify ok: ${Object.keys(variants).length} variants, ${sources.length} source files, origin ${provenance.repoHead}`)
  process.exit(0)
}
emit("origin.json", `${render(result, 4)}\n`)
console.log(`wrote ${Object.keys(variants).length} variants; origin ${provenance.repoHead}, ${sources.length} source files, node ${process.version}`)
