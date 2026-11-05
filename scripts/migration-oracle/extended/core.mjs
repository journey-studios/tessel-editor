// Núcleo do oráculo ESTENDIDO (K / BL018): escreve as entradas, gera as expectativas chamando as
// funções REAIS da origem e verifica tudo depois (hashes, deriva da origem, regeneração byte a
// byte, tolerâncias fixadas e intenção de cada caso).
//
// Nenhuma regra esperada é implementada aqui. A intenção declarada no corpus (`intent`) é conferida
// contra o que a origem responde: se divergir, o corpus está mal desenhado e a geração FALHA, em
// vez de ajustar a expectativa.

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
  PINNED_BRANCH,
  PINNED_COMMIT,
  SOURCE_PACKAGE_SUBDIR,
  sha256Of,
} from "../lib/config.mjs"
import {
  buildHarness,
  EnvironmentError,
  installedToolVersions,
  pinnedBlobCheck,
} from "../lib/harness.mjs"
import { selectProbes } from "../lib/probes.mjs"
import { serialize } from "../lib/serialize.mjs"
import {
  EXTENDED_DIR,
  EXTENDED_FORMAT_VERSION,
  EXTENDED_MANIFEST_VERSION,
  extendedHarnessHashes,
} from "./config.mjs"
import { buildExtendedCorpus } from "./corpus.mjs"
import {
  exactFloorFrame,
  exactRationalOfDouble,
  frameBoundaryRational,
  nextDown,
  nextUp,
  rationalToDouble,
} from "./numeric.mjs"

const BUNDLES_DIR = "bundles"
const EXPECTED_DIR = "expected"
const CASES_FILE = "cases.json"
const MANIFEST_FILE = "manifest.json"
const TOLERANCES_FILE = "tolerances.json"

const bundleFileOf = (id) => `${BUNDLES_DIR}/${id}.bundle.json`
const expectedFileOf = (id) => `${EXPECTED_DIR}/${id}.expected.json`

const readdirSafe = (dir) => (existsSync(dir) ? readdirSync(dir) : [])
const readText = (dir, relative) => readFileSync(path.join(dir, relative), "utf8")
const readJson = (dir, relative) => JSON.parse(readText(dir, relative))
const writeText = (dir, relative, text) => {
  const target = path.join(dir, relative)
  mkdirSync(path.dirname(target), { recursive: true })
  writeFileSync(target, text)
}

const scriptTolerancesPath = path.join(EXTENDED_DIR, TOLERANCES_FILE)

// --------------------------------------------------------------------------------------------
// corpus: escreve as ENTRADAS (bundles, cases.json e a cópia das tolerâncias). Sem a origem.
// --------------------------------------------------------------------------------------------

export const writeExtendedCorpus = (fixturesDir) => {
  const corpus = buildExtendedCorpus()
  for (const stale of readdirSafe(path.join(fixturesDir, BUNDLES_DIR))) {
    rmSync(path.join(fixturesDir, BUNDLES_DIR, stale))
  }
  for (const entry of corpus) {
    writeText(fixturesDir, bundleFileOf(entry.id), serialize(entry.bundle))
  }
  writeText(fixturesDir, TOLERANCES_FILE, readFileSync(scriptTolerancesPath, "utf8"))
  writeText(
    fixturesDir,
    CASES_FILE,
    serialize({
      formatVersion: EXTENDED_FORMAT_VERSION,
      note: "Entradas do oráculo ESTENDIDO (BL018/K). `intent` e `probes` são declarados antes da execução e conferidos na geração; os resultados esperados ficam em expected/ e vêm da origem.",
      tolerances: TOLERANCES_FILE,
      cases: corpus.map((entry) => ({
        id: entry.id,
        family: entry.family,
        kind: entry.kind,
        description: entry.description,
        bundle: bundleFileOf(entry.id),
        expected: expectedFileOf(entry.id),
        intent: entry.intent,
        probes: entry.probes,
      })),
    }),
  )
  return corpus.length
}

// --------------------------------------------------------------------------------------------
// Tempo: probes de segundos expandidos a partir dos frames declarados
// --------------------------------------------------------------------------------------------

/**
 * Segundos que cercam o início de cada frame: o instante exato, 1 µs antes, meio frame depois e,
 * para frames ≥ 1, os dois doubles vizinhos do instante. O valor `seconds` é o double mais próximo
 * da fração exata (divisão IEEE correta); `exact` é o que o consumidor Rust usa.
 */
const expandSecondsProbes = (fps, frames) => {
  const probes = []
  for (const frame of frames) {
    const boundary = frameBoundaryRational(frame, fps)
    const boundarySeconds = rationalToDouble(boundary)
    probes.push({ label: "boundary", frame, exact: boundary, seconds: boundarySeconds })
    const minus = {
      num: String(BigInt(boundary.num) * 1000000n - BigInt(boundary.den)),
      den: String(BigInt(boundary.den) * 1000000n),
    }
    probes.push({ label: "minus-1us", frame, exact: minus, seconds: rationalToDouble(minus) })
    const mid = {
      num: String(BigInt(2 * frame + 1) * BigInt(fps.denominator)),
      den: String(2n * BigInt(fps.numerator)),
    }
    probes.push({ label: "mid-frame", frame, exact: mid, seconds: rationalToDouble(mid) })
    if (frame >= 1) {
      for (const [label, seconds] of [
        ["ulp-below", nextDown(boundarySeconds)],
        ["ulp-above", nextUp(boundarySeconds)],
      ]) {
        probes.push({ label, frame, exact: exactRationalOfDouble(seconds), seconds })
      }
    }
  }
  return probes
}

const timeBlock = (harness, parsed, spec) => {
  const probes = expandSecondsProbes(parsed.fps, spec.frames)
  const evaluated = harness.evaluateTime({
    fps: parsed.fps,
    editDurationFrames: parsed.editDurationFrames,
    frames: spec.frames,
    seconds: probes,
  })
  const seconds = evaluated.seconds.map((probe) => {
    const exactFrame = exactFloorFrame(probe.exact, parsed.fps)
    return {
      ...probe,
      // Piso exato do instante: definição matemática (BigInt), NÃO uma função da origem. Só
      // classifica se a resposta em ponto flutuante da origem é o piso do instante.
      exactFloorFrame: exactFrame,
      originMatchesExact: probe.floorFrame === exactFrame,
    }
  })
  return {
    fps: parsed.fps,
    editDurationFrames: parsed.editDurationFrames,
    ...evaluated,
    seconds,
    originFloatArtifacts: seconds.filter((probe) => !probe.originMatchesExact).length,
  }
}

// --------------------------------------------------------------------------------------------
// Execução de um caso contra a origem
// --------------------------------------------------------------------------------------------

const orderProblems = (parsed, evaluation, orders, harness, caseId) => {
  const canonical = new Map(
    evaluation.editFrames.map((frame) => [frame.editFrame, serialize(frame)]),
  )
  const problems = []
  const records = orders.map((order) => {
    const result = harness.evaluateInOrder(parsed, order.frames)
    order.frames.forEach((frame, index) => {
      if (serialize(result.editFrames[index]) !== canonical.get(frame)) {
        problems.push(`ORDER_DEPENDENCE ${caseId}: ${order.name} difere da avaliação canônica no frame ${frame}`)
      }
    })
    if (result.bundleMutated) {
      problems.push(`BUNDLE_MUTATED ${caseId}: a avaliação em ${order.name} alterou o bundle`)
    }
    return {
      name: order.name,
      frames: order.frames,
      resultsSha256: sha256Of(serialize(result.editFrames)),
    }
  })
  return { records, problems }
}

const runCase = (harness, entry, bundleText) => {
  const raw = JSON.parse(bundleText)
  const { parsed, validation } = harness.inspectBundle(raw)
  const base = {
    formatVersion: EXTENDED_FORMAT_VERSION,
    case: entry.id,
    family: entry.family,
    kind: entry.kind,
    bundle: entry.bundle,
    bundleSha256: sha256Of(bundleText),
    validation,
  }
  const empty = { probes: null, evaluation: null, joins: null, cameraWorld: null, orders: null, time: null }
  if (!parsed) {
    return { expected: { ...base, ...empty }, problems: [] }
  }
  const spec = entry.probes ?? {}
  const orderFrames = (spec.orders ?? []).flatMap((order) => order.frames)
  const probes = selectProbes(parsed, [...(spec.extraEditFrames ?? []), ...orderFrames])
  const evaluation = harness.evaluateBundle(parsed, probes)
  const problems = []
  let orders = null
  if (spec.orders) {
    const result = orderProblems(parsed, evaluation, spec.orders, harness, entry.id)
    orders = { identicalToCanonical: result.problems.length === 0, results: result.records }
    problems.push(...result.problems)
  }
  return {
    expected: {
      ...base,
      probes,
      evaluation,
      joins: spec.joinSamples ? harness.joinSamples(parsed) : null,
      cameraWorld: harness.cameraWorldMatrices(evaluation),
      orders,
      time: spec.time ? timeBlock(harness, parsed, spec.time) : null,
    },
    problems,
  }
}

/** O caso declara o que exercita; se a origem não confirmar, o corpus está mal desenhado. */
const intentProblems = (entry, expected) => {
  const { validation } = expected
  const intent = entry.intent ?? {}
  const problems = []
  if (intent.schemaOk !== undefined && validation.schema.success !== intent.schemaOk) {
    problems.push(`schema ${validation.schema.success ? "aceitou" : "recusou"}, a intenção era schemaOk=${intent.schemaOk}`)
  }
  if (intent.accepted !== undefined && validation.accepted !== intent.accepted) {
    problems.push(`veredito accepted=${validation.accepted}, a intenção era ${intent.accepted}`)
  }
  const joinCodes = (validation.editJoins ?? []).map((join) => join.code).sort()
  if (intent.joinCodes !== undefined) {
    if (JSON.stringify(joinCodes) !== JSON.stringify([...intent.joinCodes].sort())) {
      problems.push(`códigos de junção ${JSON.stringify(joinCodes)}, a intenção era ${JSON.stringify([...intent.joinCodes].sort())}`)
    }
  }
  const produced = new Set([
    ...validation.schema.issues.map((issue) => issue.sourceCode).filter(Boolean),
    ...joinCodes,
  ])
  for (const code of intent.sourceCodes ?? []) {
    if (!produced.has(code)) {
      problems.push(`código ${code} não foi produzido pela origem`)
    }
  }
  if (entry.probes?.joinSamples && expected.joins !== null && expected.joins.length === 0) {
    problems.push("o caso pede amostras de junção mas não há junção contínua no mesmo clipe")
  }
  if (entry.probes?.time && expected.time === null) {
    problems.push("o caso pede sondas de tempo mas o bundle não foi aceito pelo schema")
  }
  return problems
}

const readCases = (fixturesDir) => {
  if (!existsSync(path.join(fixturesDir, CASES_FILE))) {
    throw new EnvironmentError(
      `${CASES_FILE} ausente em ${fixturesDir}. Rode: node scripts/migration-oracle/extended.mjs corpus`,
    )
  }
  return readJson(fixturesDir, CASES_FILE).cases
}

/** As constantes JOIN_* declaradas nas tolerâncias têm de ser as que o módulo real exporta. */
const toleranceProblems = (harness, fixturesDir) => {
  const problems = []
  const declared = readJson(fixturesDir, TOLERANCES_FILE)
  const actual = harness.originJoinConstants()
  for (const [name, value] of Object.entries(actual)) {
    if (declared.joinRules[name] !== value) {
      problems.push({
        category: "TOLERANCES_JOIN_CONSTANT",
        detail: `${name}: declarado ${declared.joinRules[name]}, a origem exporta ${value}`,
      })
    }
  }
  const script = readFileSync(scriptTolerancesPath, "utf8")
  if (script !== readText(fixturesDir, TOLERANCES_FILE)) {
    problems.push({
      category: "TOLERANCES_DRIFT",
      detail: `${TOLERANCES_FILE} das fixtures difere de scripts/migration-oracle/extended/${TOLERANCES_FILE}`,
    })
  }
  return problems
}

// --------------------------------------------------------------------------------------------
// generate: expected/*.json + manifest.json
// --------------------------------------------------------------------------------------------

export const generateExtended = async (config) => {
  const entries = readCases(config.fixturesDir)
  const built = await buildHarness(config, path.join(EXTENDED_DIR, "entry.mjs"))
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
  const errors = toleranceProblems(built.module, config.fixturesDir).map(
    (problem) => `${problem.category}: ${problem.detail}`,
  )
  const outputs = []
  for (const entry of entries) {
    const { expected, problems } = runCase(built.module, entry, readText(config.fixturesDir, entry.bundle))
    errors.push(...problems)
    errors.push(...intentProblems(entry, expected).map((problem) => `INTENT ${entry.id}: ${problem}`))
    outputs.push({ entry, expected, text: serialize(expected) })
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
    serialize(buildManifest(config, entries, built, blobCheck, outputs.map((o) => o.expected))),
  )
  return { ok: true, cases: entries.length, sourceFiles: built.sources.length }
}

const fixtureFileRecords = (fixturesDir, entries) =>
  [CASES_FILE, TOLERANCES_FILE, ...entries.flatMap((entry) => [entry.bundle, entry.expected])]
    .sort()
    .map((relative) => {
      const bytes = readFileSync(path.join(fixturesDir, relative))
      return { path: relative, sha256: sha256Of(bytes), bytes: bytes.length }
    })

const summarize = (expectedList) => {
  const families = {}
  let timeProbes = 0
  let originFloatArtifacts = 0
  const artifactsByCase = {}
  let joinSamples = 0
  let frames = 0
  let originDecidedBoundaries = []
  for (const expected of expectedList) {
    families[expected.family] = (families[expected.family] ?? 0) + 1
    frames += expected.evaluation?.editFrames.length ?? 0
    joinSamples += expected.joins?.length ?? 0
    if (expected.time) {
      timeProbes += expected.time.seconds.length
      originFloatArtifacts += expected.time.originFloatArtifacts
      if (expected.time.originFloatArtifacts > 0) {
        artifactsByCase[expected.case] = expected.time.originFloatArtifacts
      }
    }
    if (expected.kind === "boundary") {
      originDecidedBoundaries = [
        ...originDecidedBoundaries,
        {
          case: expected.case,
          accepted: expected.validation.accepted,
          joinCodes: (expected.validation.editJoins ?? []).map((join) => join.code).sort(),
        },
      ]
    }
  }
  return {
    cases: expectedList.length,
    families,
    editFramesEvaluated: frames,
    joinSamples,
    timeProbes,
    originFloatArtifacts,
    originFloatArtifactsByCase: artifactsByCase,
    originDecidedBoundaries,
  }
}

const buildManifest = (config, entries, built, blobCheck, expectedList) => ({
  manifestVersion: EXTENDED_MANIFEST_VERSION,
  purpose:
    "Proveniência do oráculo ESTENDIDO (BL018/K): as expectativas em expected/ foram geradas executando as fontes REAIS listadas abaixo, sem alterá-las. Separado do manifest dos 55 casos, que não é lido nem escrito aqui.",
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
  harness: extendedHarnessHashes(),
  originJoinConstants: built.module.originJoinConstants(),
  executedSources: built.sources,
  fixtureFiles: fixtureFileRecords(config.fixturesDir, entries),
  summary: summarize(expectedList),
  scope: JSON.parse(readFileSync(path.join(EXTENDED_DIR, "scope.json"), "utf8")),
  regeneration: [
    "node scripts/migration-oracle/oracle.mjs setup",
    "node scripts/migration-oracle/extended.mjs corpus   # só se o corpus mudar",
    "node scripts/migration-oracle/extended.mjs generate",
    "node scripts/migration-oracle/extended.mjs verify",
  ],
})

// --------------------------------------------------------------------------------------------
// verify
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
  for (const key of new Set([...Object.keys(left), ...Object.keys(right)])) {
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
    } else if (sha256Of(readFileSync(absolute)) !== file.sha256) {
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
  const current = extendedHarnessHashes()
  const errors = Object.entries(manifest.harness).flatMap(([file, hash]) =>
    current[file] === hash ? [] : [{ category: "HARNESS_DRIFT", detail: file }],
  )
  for (const file of Object.keys(current)) {
    if (!(file in manifest.harness)) {
      errors.push({ category: "HARNESS_DRIFT", detail: `${file}: arquivo novo fora do manifest` })
    }
  }
  return errors
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
    const { expected, problems } = runCase(harness, entry, readText(config.fixturesDir, entry.bundle))
    const errors = [
      ...problems.map((detail) => ({ category: "ORDER_INDEPENDENCE", detail })),
      ...intentProblems(entry, expected).map((detail) => ({
        category: "INTENT_VIOLATION",
        detail: `${entry.id}: ${detail}`,
      })),
    ]
    const regenerated = serialize(expected)
    const committed = readFileSync(committedPath, "utf8")
    if (committed !== regenerated) {
      const where =
        describeDifference(JSON.parse(committed), JSON.parse(regenerated)) ?? "bytes diferentes (formatação)"
      errors.push({ category: "EXPECTATION_DIVERGENCE", detail: `${entry.id} ${where}` })
    }
    return errors
  })

/**
 * @param {{ fixturesOnly?: boolean }} options `fixturesOnly` NÃO confere nada contra a origem.
 */
export const verifyExtended = async (config, { fixturesOnly = false } = {}) => {
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
  if (manifest.manifestVersion !== EXTENDED_MANIFEST_VERSION) {
    errors.push({ category: "MANIFEST_VERSION", detail: `${manifest.manifestVersion} != ${EXTENDED_MANIFEST_VERSION}` })
  }
  const entries = existsSync(path.join(config.fixturesDir, CASES_FILE))
    ? readCases(config.fixturesDir)
    : []
  if (fixturesOnly) {
    return { errors, warnings, checkedAgainstSource: false, cases: entries.length }
  }
  errors.push(...verifyHarnessFiles(manifest), ...verifyToolchain(config, manifest))
  const built = await buildHarness(config, path.join(EXTENDED_DIR, "entry.mjs"))
  errors.push(
    ...verifySourceFiles(manifest, built.sources),
    ...toleranceProblems(built.module, config.fixturesDir),
  )
  if (manifest.tools.node !== process.version) {
    warnings.push(`Node ${process.version} difere do usado na geração (${manifest.tools.node}); só avisa, o resultado numérico é o que decide.`)
  }
  errors.push(...verifyExpectations(built.module, config, entries))
  return { errors, warnings, checkedAgainstSource: true, cases: entries.length }
}
