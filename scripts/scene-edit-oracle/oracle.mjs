#!/usr/bin/env node
// Oráculo de edição (S6 / BL004–BL005): executa `setProperties` REAL da origem sobre um corpus
// sintético de escritas em `camera/<clip>/start/*` e grava o que ela responde (clipe resultante e
// pose em cada frame). O consumidor Rust (`crates/core/tests/scene_edit_oracle.rs`) compara o que
// o editor faz com esses arquivos. Reaproveita, sem alterar, o empacotador e a configuração do
// oráculo de migração (`scripts/migration-oracle/lib`).
//
//   node scripts/scene-edit-oracle/oracle.mjs generate   # grava cases/expected/manifest
//   node scripts/scene-edit-oracle/oracle.mjs verify     # integridade + deriva da origem + regeneração byte a byte
//   node scripts/scene-edit-oracle/oracle.mjs verify --fixtures-only   # só integridade (NÃO conferido na origem)
//
// Saída: 0 ok · 1 corrupção/deriva/divergência · 2 ambiente (origem ou deps ausentes).

import { createHash } from "node:crypto"
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

import { PINNED_COMMIT, REPO_ROOT, resolveConfig, sha256Of } from "../migration-oracle/lib/config.mjs"
import { EnvironmentError, buildHarness, installedToolVersions, pinnedBlobCheck } from "../migration-oracle/lib/harness.mjs"
import { bundleWith, cases, sampleFrames } from "./corpus.mjs"

const HERE = path.dirname(fileURLToPath(import.meta.url))
const FIXTURES = path.join(REPO_ROOT, "docs", "migration", "fixtures", "scene-edit")
const FORMAT_VERSION = 1

const pretty = (value) => `${JSON.stringify(value, null, 2)}\n`

const casesFile = () => ({
  formatVersion: FORMAT_VERSION,
  note: "Entradas sintéticas; o resultado esperado é o que a origem responde (expected.json).",
  cases: cases.map(({ id, family, intent, clip, assignments }) => ({
    id, family, intent, assignments, bundle: bundleWith(clip),
  })),
})

const pose = (state) => ({
  position: state.position,
  target: state.target,
  fovDeg: state.fovDeg,
})

const errorOf = (error) => ({
  code: error.code,
  path: error.path,
  message: error.message,
  cameraCodes: (error.errors ?? []).map((entry) => entry.code),
})

/** `camera/<clip>/start/target|fov`: the origin has only the `setStart` command for them. */
const COMMAND_FIELDS = { target: "target", fov: "fovDeg" }

/**
 * One write, as the origin does it: by property path where it has one, by `setStart` where it does
 * not. Returns the new bundle or the origin's errors; each write is checked on the whole bundle
 * (`finalizeBundle`), like `setProperties` does for each of its assignments.
 */
const writeOnce = (harness, bundle, assignment) => {
  const [, clipId, group, field] = assignment.path.split("/")
  const commandField = group === "start" ? COMMAND_FIELDS[field] : undefined
  if (commandField === undefined) {
    const result = harness.setProperties(bundle, [assignment])
    return result.ok ? { ok: true, bundle: result.bundle } : result
  }
  const answered = harness.applyCameraCommands(bundle, clipId, [{ op: "setStart", [commandField]: assignment.value }])
  if (!answered.ok) {
    return { ok: false, errors: [{ code: "CAMERA_REFUSED", path: assignment.path, message: answered.errors.map((entry) => `${entry.code}: ${entry.message}`).join("; "), errors: answered.errors }] }
  }
  return harness.applyPropertyChanges(bundle, [{ domain: "camera", clip: answered.clip }], assignment.path)
}

const expectedFor = (harness, entry) => {
  const parsed = harness.parseBundle(entry.bundle)
  if (!parsed.success) {
    throw new Error(`INTENT ${entry.id}: o bundle de entrada deve ser válido para a origem`)
  }
  let current = parsed.data
  for (const assignment of entry.assignments) {
    const written = writeOnce(harness, current, assignment)
    if (!written.ok) {
      return { id: entry.id, ok: false, errors: written.errors.map(errorOf) }
    }
    current = written.bundle
  }
  const clip = current.cameraClips.find((candidate) => candidate.id === "cam")
  return {
    id: entry.id,
    ok: true,
    changes: entry.assignments.length,
    clip,
    poses: sampleFrames(clip).map((frame) => ({
      frame,
      ...pose(harness.resolveCameraAtClipFrame(current, clip.id, frame)),
    })),
  }
}

const INTENT_OK = { error: false }
const checkIntent = (entry, expected) => {
  const wantsError = entry.family === "error"
  if (expected.ok === wantsError) {
    throw new Error(`INTENT ${entry.id}: família ${entry.family}, mas a origem respondeu ok=${expected.ok}`)
  }
  return INTENT_OK
}

const generateInMemory = async (config) => {
  const harness = await buildHarness(config, path.join(HERE, "entry.mjs"))
  const input = casesFile()
  const expected = input.cases.map((entry) => {
    const answer = expectedFor(harness.module, entry)
    checkIntent(entry, answer)
    return answer
  })
  const blobs = pinnedBlobCheck(harness.sourceRoot, PINNED_COMMIT, harness.sources)
  if (!blobs.available || blobs.mismatches.length > 0) {
    throw new EnvironmentError(
      `a origem não é o snapshot ${PINNED_COMMIT}: ${blobs.available ? JSON.stringify(blobs.mismatches) : blobs.reason}`,
    )
  }
  const expectedFile = { formatVersion: FORMAT_VERSION, cases: expected }
  const harnessFiles = ["entry.mjs", "corpus.mjs", "oracle.mjs"].map((name) => [
    `scripts/scene-edit-oracle/${name}`,
    sha256Of(readFileSync(path.join(HERE, name))),
  ])
  const manifest = {
    formatVersion: FORMAT_VERSION,
    claim: "Expectativas geradas por setProperties/resolveCameraAtClipFrame REAIS de packages/scene-engine, empacotados sem alteração (esbuild + zod + three fixados). Não é a engine completa.",
    originCommit: PINNED_COMMIT,
    packageTreeAtCommit: blobs.packageTreeAtCommit,
    tools: { node: process.version, ...installedToolVersions(config.depsDir) },
    harness: Object.fromEntries(harnessFiles),
    sources: harness.sources.map(({ path: file, sha256, gitBlob }) => ({ path: file, sha256, gitBlob })),
    fixtures: {
      "cases.json": sha256Of(pretty(input)),
      "expected.json": sha256Of(pretty(expectedFile)),
    },
  }
  return { input, expectedFile, manifest }
}

const generate = async (config) => {
  const { input, expectedFile, manifest } = await generateInMemory(config)
  mkdirSync(FIXTURES, { recursive: true })
  writeFileSync(path.join(FIXTURES, "cases.json"), pretty(input))
  writeFileSync(path.join(FIXTURES, "expected.json"), pretty(expectedFile))
  writeFileSync(path.join(FIXTURES, "manifest.json"), pretty(manifest))
  console.log(`ok: ${expectedFile.cases.length} casos gravados em ${path.relative(REPO_ROOT, FIXTURES)}`)
  return 0
}

const read = (name) => readFileSync(path.join(FIXTURES, name), "utf8")

const verify = async (config, fixturesOnly) => {
  const errors = []
  for (const name of ["cases.json", "expected.json", "manifest.json"]) {
    if (!existsSync(path.join(FIXTURES, name))) {
      errors.push(`FIXTURE_MISSING ${name}`)
    }
  }
  if (errors.length > 0) {
    errors.forEach((error) => console.error(`FAIL ${error}`))
    return 1
  }
  const manifest = JSON.parse(read("manifest.json"))
  for (const [name, hash] of Object.entries(manifest.fixtures)) {
    if (sha256Of(read(name)) !== hash) {
      errors.push(`FIXTURE_HASH_MISMATCH ${name}`)
    }
  }
  if (sha256Of(pretty(casesFile())) !== manifest.fixtures["cases.json"]) {
    errors.push("CORPUS_DRIFT cases.json não é o que corpus.mjs produz")
  }
  if (!fixturesOnly && errors.length === 0) {
    const fresh = await generateInMemory(config)
    if (pretty(fresh.expectedFile) !== read("expected.json")) {
      errors.push("EXPECTATION_DIVERGENCE regenerar com a origem não reproduz expected.json byte a byte")
    }
    if (pretty(fresh.manifest.sources) !== pretty(manifest.sources)) {
      errors.push("SOURCE_DRIFT as fontes empacotadas da origem mudaram")
    }
  }
  errors.forEach((error) => console.error(`FAIL ${error}`))
  if (errors.length > 0) {
    return 1
  }
  const scope = fixturesOnly ? "SOMENTE integridade (NÃO conferido na origem)" : "contra a origem (regeneração byte a byte)"
  console.log(`ok: ${manifest.fixtures ? Object.keys(manifest.fixtures).length : 0} arquivos verificados ${scope}`)
  return 0
}

const main = async () => {
  const [command, ...rest] = process.argv.slice(2)
  const flags = {}
  for (let index = 0; index < rest.length; index += 1) {
    if (rest[index] === "--fixtures-only") {
      flags["fixtures-only"] = true
    } else if (rest[index].startsWith("--") && rest[index + 1]) {
      flags[rest[index].slice(2)] = rest[index + 1]
      index += 1
    } else {
      throw new EnvironmentError(`argumento inesperado: ${rest[index]}`)
    }
  }
  const config = resolveConfig(flags)
  if (command === "generate") {
    return generate(config)
  }
  if (command === "verify") {
    return verify(config, flags["fixtures-only"] === true)
  }
  console.error("Uso: oracle.mjs generate | verify [--fixtures-only] [--source-root DIR] [--deps-dir DIR]")
  return 2
}

main().then(
  (code) => { process.exitCode = code },
  (error) => {
    console.error(error instanceof EnvironmentError ? `ERRO DE AMBIENTE: ${error.message}` : error)
    process.exitCode = error instanceof EnvironmentError ? 2 : 1
  },
)
