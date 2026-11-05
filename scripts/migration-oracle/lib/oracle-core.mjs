// Núcleo do oráculo: gera as expectativas com as funções REAIS da origem e as verifica depois.
// Nenhuma regra esperada é implementada aqui; só se orquestra, serializa e compara.

import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import path from "node:path"

import {
  EXPECTED_FORMAT_VERSION,
  harnessFileHashes,
  MANIFEST_VERSION,
  ORACLE_DIR,
  PINNED_BRANCH,
  PINNED_COMMIT,
  SOURCE_PACKAGE_SUBDIR,
  sha256Of,
} from "./config.mjs"
import {
  buildHarness,
  EnvironmentError,
  installedToolVersions,
  pinnedBlobCheck,
} from "./harness.mjs"
import { selectProbes } from "./probes.mjs"
import { serialize } from "./serialize.mjs"
import { buildCorpus } from "../corpus/builder.mjs"

const BUNDLES_DIR = "bundles"
const EXPECTED_DIR = "expected"
const CASES_FILE = "cases.json"
const MANIFEST_FILE = "manifest.json"

const bundleFileOf = (id) => `${BUNDLES_DIR}/${id}.bundle.json`
const expectedFileOf = (id) => `${EXPECTED_DIR}/${id}.expected.json`

const readText = (fixturesDir, relative) =>
  readFileSync(path.join(fixturesDir, relative), "utf8")

const writeText = (fixturesDir, relative, text) => {
  const target = path.join(fixturesDir, relative)
  mkdirSync(path.dirname(target), { recursive: true })
  writeFileSync(target, text)
}

const readJson = (fixturesDir, relative) => JSON.parse(readText(fixturesDir, relative))

// --------------------------------------------------------------------------------------------
// corpus: escreve as ENTRADAS (bundles + índice). Não precisa da origem.
// --------------------------------------------------------------------------------------------

export const writeCorpus = (fixturesDir) => {
  const corpus = buildCorpus()
  for (const entry of readdirSafe(path.join(fixturesDir, BUNDLES_DIR))) {
    rmSync(path.join(fixturesDir, BUNDLES_DIR, entry))
  }
  for (const entry of corpus) {
    writeText(fixturesDir, bundleFileOf(entry.id), serialize(entry.bundle))
  }
  const index = {
    formatVersion: EXPECTED_FORMAT_VERSION,
    note: "Índice das entradas do oráculo. `intent` é só a intenção do caso, conferida na geração; os resultados esperados ficam em expected/ e vêm da origem.",
    cases: corpus.map((entry) => ({
      id: entry.id,
      kind: entry.kind,
      description: entry.description,
      bundle: bundleFileOf(entry.id),
      expected: expectedFileOf(entry.id),
      intent: entry.intent,
    })),
  }
  writeText(fixturesDir, CASES_FILE, serialize(index))
  return corpus.length
}

const readdirSafe = (dir) => (existsSync(dir) ? readdirSync(dir) : [])

// --------------------------------------------------------------------------------------------
// Execução de um caso contra a origem
// --------------------------------------------------------------------------------------------

const runCase = (harness, entry, bundleText) => {
  const raw = JSON.parse(bundleText)
  const { parsed, validation } = harness.inspectBundle(raw)
  const base = {
    formatVersion: EXPECTED_FORMAT_VERSION,
    case: entry.id,
    kind: entry.kind,
    bundle: entry.bundle,
    bundleSha256: sha256Of(bundleText),
    validation,
  }
  if (!(parsed && validation.accepted)) {
    return { ...base, probes: null, evaluation: null }
  }
  const probes = selectProbes(parsed, entry.intent?.extraEditFrames ?? [])
  return { ...base, probes, evaluation: harness.evaluateBundle(parsed, probes) }
}

/** O caso diz o que exercita; se a origem não confirmar, o corpus está mal desenhado. */
const intentProblems = (entry, expected) => {
  const { validation } = expected
  const issues = validation.schema.issues
  const joins = validation.editJoins ?? []
  const problems = []
  if (entry.kind === "valid" && !validation.accepted) {
    problems.push("caso válido foi recusado pela origem")
  }
  if (entry.kind === "invalid" && validation.accepted) {
    problems.push("caso inválido foi aceito pela origem")
  }
  const producedCodes = new Set([
    ...issues.map((issue) => issue.sourceCode).filter(Boolean),
    ...joins.map((join) => join.code),
  ])
  for (const code of entry.intent?.sourceCodes ?? []) {
    if (!producedCodes.has(code)) {
      problems.push(`código ${code} não foi produzido pela origem`)
    }
  }
  for (const zodPath of entry.intent?.zodPaths ?? []) {
    if (!issues.some((issue) => issue.path === zodPath)) {
      problems.push(`nenhum erro do schema no caminho ${zodPath}`)
    }
  }
  for (const text of entry.intent?.messageIncludes ?? []) {
    if (!issues.some((issue) => issue.message.includes(text))) {
      problems.push(`nenhuma mensagem contém "${text}"`)
    }
  }
  return problems
}

const readCases = (fixturesDir) => {
  if (!existsSync(path.join(fixturesDir, CASES_FILE))) {
    throw new EnvironmentError(
      `${CASES_FILE} ausente em ${fixturesDir}. Rode: node scripts/migration-oracle/oracle.mjs corpus`,
    )
  }
  return readJson(fixturesDir, CASES_FILE).cases
}

// --------------------------------------------------------------------------------------------
// generate: expected/*.json + manifest.json
// --------------------------------------------------------------------------------------------

export const generate = async (config) => {
  const entries = readCases(config.fixturesDir)
  const built = await buildHarness(config)
  const blobCheck = pinnedBlobCheck(built.sourceRoot, PINNED_COMMIT, built.sources)
  if (!blobCheck.available) {
    throw new EnvironmentError(
      `Não é possível provar o commit fixado: ${blobCheck.reason}. A geração é recusada para não gravar proveniência sem prova.`,
    )
  }
  if (blobCheck.mismatches.length > 0) {
    return {
      ok: false,
      errors: blobCheck.mismatches.map(
        (m) => `SOURCE_NOT_AT_PINNED_COMMIT ${m.path}: working tree ${m.workingTreeBlob}, commit ${m.pinnedBlob}`,
      ),
    }
  }

  const outputs = []
  const errors = []
  for (const entry of entries) {
    const expected = runCase(built.module, entry, readText(config.fixturesDir, entry.bundle))
    const problems = intentProblems(entry, expected)
    errors.push(...problems.map((problem) => `INTENT ${entry.id}: ${problem}`))
    outputs.push({ entry, text: serialize(expected) })
  }
  if (errors.length > 0) {
    return { ok: false, errors }
  }

  for (const stale of readdirSafe(path.join(config.fixturesDir, EXPECTED_DIR))) {
    rmSync(path.join(config.fixturesDir, EXPECTED_DIR, stale))
  }
  for (const { entry, text } of outputs) {
    writeText(config.fixturesDir, entry.expected, text)
  }
  writeText(
    config.fixturesDir,
    MANIFEST_FILE,
    serialize(buildManifest(config, entries, built.sources, blobCheck)),
  )
  return { ok: true, cases: entries.length, sourceFiles: built.sources.length }
}

const fixtureFileRecords = (fixturesDir, entries) =>
  [
    CASES_FILE,
    ...entries.flatMap((entry) => [entry.bundle, entry.expected]),
  ]
    .sort()
    .map((relative) => {
      const bytes = readFileSync(path.join(fixturesDir, relative))
      return { path: relative, sha256: sha256Of(bytes), bytes: bytes.length }
    })

const buildManifest = (config, entries, sources, blobCheck) => ({
  manifestVersion: MANIFEST_VERSION,
  purpose:
    "Proveniência do oráculo BB10/E0: as expectativas em expected/ foram geradas executando as fontes REAIS listadas abaixo, sem alterá-las.",
  source: {
    repository: "tesselapp-image-coverage (somente leitura)",
    branch: PINNED_BRANCH,
    commit: PINNED_COMMIT,
    package: SOURCE_PACKAGE_SUBDIR,
    packageTreeAtCommit: blobCheck.packageTreeAtCommit,
    workingTreeMatchesCommit: true,
    note: "executedSources[].gitBlob foi cruzado com `git ls-tree` do commit fixado na geração; verify confere pelo sha256 do conteúdo.",
  },
  tools: {
    node: process.version,
    ...installedToolVersions(config.depsDir),
    pinnedBy: "scripts/migration-oracle/package.json + pnpm-lock.yaml (integridades iguais às do lockfile da origem)",
  },
  harness: harnessFileHashes(),
  executedSources: sources,
  fixtureFiles: fixtureFileRecords(config.fixturesDir, entries),
  scope: JSON.parse(readFileSync(path.join(ORACLE_DIR, "harness", "scope.json"), "utf8")),
  regeneration: [
    "node scripts/migration-oracle/oracle.mjs setup",
    "node scripts/migration-oracle/oracle.mjs corpus   # só se o corpus mudar",
    "node scripts/migration-oracle/oracle.mjs generate",
    "node scripts/migration-oracle/oracle.mjs verify",
  ],
})

// --------------------------------------------------------------------------------------------
// verify: integridade das fixtures + deriva da origem + divergência das expectativas
// --------------------------------------------------------------------------------------------

const describeDifference = (left, right, where = "$") => {
  if (Object.is(left, right)) {
    return null
  }
  const bothContainers =
    typeof left === "object" && left !== null && typeof right === "object" && right !== null
  if (!bothContainers || Array.isArray(left) !== Array.isArray(right)) {
    return `${where}: committed ${JSON.stringify(left)} != regenerated ${JSON.stringify(right)}`
  }
  const keys = new Set([...Object.keys(left), ...Object.keys(right)])
  for (const key of keys) {
    const found = describeDifference(left[key], right[key], `${where}.${key}`)
    if (found) {
      return found
    }
  }
  return null
}

const verifyFixtureFiles = (fixturesDir, manifest) => {
  const errors = []
  const listed = new Set(manifest.fixtureFiles.map((file) => file.path))
  for (const file of manifest.fixtureFiles) {
    const absolute = path.join(fixturesDir, file.path)
    if (!existsSync(absolute)) {
      errors.push({ category: "FIXTURE_MISSING", detail: file.path })
      continue
    }
    if (sha256Of(readFileSync(absolute)) !== file.sha256) {
      errors.push({ category: "FIXTURE_HASH_MISMATCH", detail: file.path })
    }
  }
  for (const dir of [BUNDLES_DIR, EXPECTED_DIR]) {
    for (const name of readdirSafe(path.join(fixturesDir, dir))) {
      if (!listed.has(`${dir}/${name}`)) {
        errors.push({ category: "FIXTURE_UNLISTED", detail: `${dir}/${name}` })
      }
    }
  }
  return errors
}

const verifyHarnessFiles = (manifest) => {
  const current = harnessFileHashes()
  return Object.entries(manifest.harness).flatMap(([file, hash]) =>
    current[file] === hash ? [] : [{ category: "HARNESS_DRIFT", detail: file }],
  )
}

const verifyToolchain = (config, manifest) => {
  const installed = installedToolVersions(config.depsDir)
  return ["esbuild", "three", "zod"].flatMap((name) =>
    installed[name] === manifest.tools[name]
      ? []
      : [{ category: "TOOLCHAIN_DRIFT", detail: `${name}: manifest ${manifest.tools[name]}, installed ${installed[name]}` }],
  )
}

const verifySourceFiles = (manifest, sources) => {
  const pinned = new Map(manifest.executedSources.map((file) => [file.path, file]))
  const current = new Map(sources.map((file) => [file.path, file]))
  const errors = []
  for (const [file, record] of pinned) {
    const now = current.get(file)
    if (!now) {
      errors.push({ category: "SOURCE_DRIFT", detail: `${file}: deixou de ser carregado pelo harness` })
    } else if (now.sha256 !== record.sha256) {
      errors.push({ category: "SOURCE_DRIFT", detail: `${file}: sha256 ${now.sha256.slice(0, 12)} != fixado ${record.sha256.slice(0, 12)}` })
    }
  }
  for (const file of current.keys()) {
    if (!pinned.has(file)) {
      errors.push({ category: "SOURCE_DRIFT", detail: `${file}: novo arquivo carregado, ausente do manifest` })
    }
  }
  return errors
}

const verifyExpectations = (harness, config, entries) =>
  entries.flatMap((entry) => {
    const committedPath = path.join(config.fixturesDir, entry.expected)
    if (!existsSync(committedPath)) {
      return [{ category: "EXPECTATION_MISSING", detail: entry.expected }]
    }
    const regenerated = serialize(
      runCase(harness, entry, readText(config.fixturesDir, entry.bundle)),
    )
    const committed = readFileSync(committedPath, "utf8")
    if (committed === regenerated) {
      return []
    }
    const where =
      describeDifference(JSON.parse(committed), JSON.parse(regenerated)) ?? "bytes diferentes (formatação)"
    return [{ category: "EXPECTATION_DIVERGENCE", detail: `${entry.id} ${where}` }]
  })

/**
 * @param {{ fixturesOnly?: boolean }} options `fixturesOnly` NÃO confere nada contra a origem:
 *   só detecta corrupção/edição das fixtures versionadas. Precisa ser pedido explicitamente.
 */
export const verify = async (config, { fixturesOnly = false } = {}) => {
  const manifestPath = path.join(config.fixturesDir, MANIFEST_FILE)
  if (!existsSync(manifestPath)) {
    return {
      errors: [{ category: "MANIFEST_MISSING", detail: manifestPath }],
      warnings: [],
      checkedAgainstSource: false,
      cases: 0,
    }
  }
  const manifest = readJson(config.fixturesDir, MANIFEST_FILE)
  const errors = [...verifyFixtureFiles(config.fixturesDir, manifest)]
  const warnings = []
  if (manifest.manifestVersion !== MANIFEST_VERSION) {
    errors.push({ category: "MANIFEST_VERSION", detail: `${manifest.manifestVersion} != ${MANIFEST_VERSION}` })
  }
  const entries = existsSync(path.join(config.fixturesDir, CASES_FILE))
    ? readCases(config.fixturesDir)
    : []
  if (fixturesOnly) {
    return { errors, warnings, checkedAgainstSource: false, cases: entries.length }
  }

  errors.push(...verifyHarnessFiles(manifest), ...verifyToolchain(config, manifest))
  const built = await buildHarness(config)
  errors.push(...verifySourceFiles(manifest, built.sources))
  if (manifest.tools.node !== process.version) {
    warnings.push(`Node ${process.version} difere do usado na geração (${manifest.tools.node}); só avisa, o resultado numérico é o que decide.`)
  }
  errors.push(...verifyExpectations(built.module, config, entries))
  return { errors, warnings, checkedAgainstSource: true, cases: entries.length }
}
