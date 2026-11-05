// Testes do runner do oráculo (node:test). Rodar com:
//   node --test scripts/migration-oracle/
//
// Tudo que corrompe ou regenera opera em CÓPIAS temporárias (fixtures e, para deriva, um recorte
// de `src/` da origem). As fixtures versionadas e a origem nunca são alteradas por estes testes.

import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import {
  cpSync,
  existsSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import os from "node:os"
import path from "node:path"
import { after, describe, test } from "node:test"

import { ORACLE_DIR, REPO_ROOT, resolveConfig, sha256Of } from "./lib/config.mjs"
import { depsInstalled } from "./lib/harness.mjs"

const CLI = path.join(ORACLE_DIR, "oracle.mjs")
const REAL = resolveConfig({})
const COMMITTED = REAL.fixturesDir
const environmentReady =
  existsSync(path.join(REAL.sourceRoot, "src")) && depsInstalled(REAL.depsDir)

const scratch = mkdtempSync(path.join(os.tmpdir(), "oracle-spec-"))
after(() => rmSync(scratch, { recursive: true, force: true }))

let counter = 0
const freshDir = (label) => path.join(scratch, `${label}-${(counter += 1)}`)

const copyFixtures = (label) => {
  const target = freshDir(label)
  cpSync(COMMITTED, target, { recursive: true })
  return target
}

const oracle = (args, { fixtures, sourceRoot } = {}) => {
  const result = spawnSync("node", [CLI, ...args], {
    encoding: "utf8",
    env: {
      ...process.env,
      ORACLE_FIXTURES_DIR: fixtures ?? COMMITTED,
      ORACLE_SOURCE_ROOT: sourceRoot ?? REAL.sourceRoot,
      ORACLE_DEPS_DIR: REAL.depsDir,
    },
  })
  return { status: result.status, out: result.stdout, err: result.stderr }
}

const readJson = (file) => JSON.parse(readFileSync(file, "utf8"))
const writeJson = (file, value) => writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`)

/** Troca o hash de um arquivo no manifest, simulando quem "arruma" o manifest depois de editar. */
const rehashInManifest = (fixtures, relative) => {
  const manifestPath = path.join(fixtures, "manifest.json")
  const manifest = readJson(manifestPath)
  const record = manifest.fixtureFiles.find((file) => file.path === relative)
  record.sha256 = sha256Of(readFileSync(path.join(fixtures, relative)))
  writeJson(manifestPath, manifest)
}

const snapshotDir = (dir) =>
  Object.fromEntries(
    ["bundles", "expected"]
      .flatMap((sub) => readdirSync(path.join(dir, sub)).map((name) => `${sub}/${name}`))
      .concat("cases.json", "manifest.json")
      .sort()
      .map((relative) => [relative, sha256Of(readFileSync(path.join(dir, relative)))]),
  )

const env = { skip: environmentReady ? false : "origem ou dependências ausentes (rode `setup`)" }

describe("fixtures versionadas", env, () => {
  test("verify passa: hashes da origem e regeneração byte a byte", () => {
    const result = oracle(["verify"])
    assert.equal(result.status, 0, result.err)
    assert.match(result.out, /contra a origem/)
  })

  test("o manifest fixa o commit e o hash de cada arquivo executado", () => {
    const manifest = readJson(path.join(COMMITTED, "manifest.json"))
    assert.match(manifest.source.commit, /^[0-9a-f]{40}$/)
    assert.equal(manifest.source.workingTreeMatchesCommit, true)
    assert.ok(manifest.executedSources.length > 20)
    for (const source of manifest.executedSources) {
      assert.match(source.sha256, /^[0-9a-f]{64}$/)
      assert.match(source.gitBlob, /^[0-9a-f]{40}$/)
    }
    assert.equal(manifest.tools.zod, "4.3.6")
    assert.equal(manifest.tools.three, "0.187.0")
  })

  test("cada caso tem bundle e expectativa, e há válidos e inválidos", () => {
    const { cases } = readJson(path.join(COMMITTED, "cases.json"))
    assert.ok(cases.some((entry) => entry.kind === "valid"))
    assert.ok(cases.some((entry) => entry.kind === "invalid"))
    for (const entry of cases) {
      assert.ok(existsSync(path.join(COMMITTED, entry.bundle)), entry.bundle)
      assert.ok(existsSync(path.join(COMMITTED, entry.expected)), entry.expected)
    }
  })

  test("o corpus versionado é exatamente o que o builder produz", () => {
    const rebuilt = freshDir("rebuilt")
    const result = oracle(["corpus"], { fixtures: rebuilt })
    assert.equal(result.status, 0, result.err)
    for (const name of readdirSync(path.join(COMMITTED, "bundles"))) {
      assert.equal(
        readFileSync(path.join(rebuilt, "bundles", name), "utf8"),
        readFileSync(path.join(COMMITTED, "bundles", name), "utf8"),
        name,
      )
    }
    assert.equal(
      readFileSync(path.join(rebuilt, "cases.json"), "utf8"),
      readFileSync(path.join(COMMITTED, "cases.json"), "utf8"),
    )
  })

  test("a expectativa de rejeição traz o código estável da origem", () => {
    const expected = readJson(path.join(COMMITTED, "expected", "invalid-edit-gap.expected.json"))
    assert.equal(expected.validation.accepted, false)
    assert.ok(
      expected.validation.schema.issues.some((issue) => issue.sourceCode === "EDIT_GAP_OR_OVERLAP"),
    )
    assert.equal(expected.evaluation, null)
  })
})

describe("verify detecta corrupção e divergência (exit != 0)", env, () => {
  test("expectativa numérica adulterada: hash e divergência acusam o caso", () => {
    const fixtures = copyFixtures("corrupt-number")
    const file = path.join(fixtures, "expected", "valid-minimal-continuous.expected.json")
    const expected = readJson(file)
    expected.evaluation.editFrames[2].state.camera.position.z += 0.001
    writeJson(file, expected)
    const result = oracle(["verify"], { fixtures })
    assert.equal(result.status, 1)
    assert.match(result.err, /FIXTURE_HASH_MISMATCH: expected\/valid-minimal-continuous/)
    assert.match(result.err, /EXPECTATION_DIVERGENCE: valid-minimal-continuous .*position\.z/)
  })

  test("expectativa adulterada COM manifest 'consertado': a regeneração ainda pega", () => {
    const fixtures = copyFixtures("corrupt-rehashed")
    const relative = "expected/valid-cut-instance-offsets.expected.json"
    const file = path.join(fixtures, relative)
    const expected = readJson(file)
    expected.evaluation.editFrames[3].state.instanceFrame += 1
    writeJson(file, expected)
    rehashInManifest(fixtures, relative)
    const result = oracle(["verify"], { fixtures })
    assert.equal(result.status, 1)
    assert.doesNotMatch(result.err, /FIXTURE_HASH_MISMATCH/)
    assert.match(result.err, /EXPECTATION_DIVERGENCE: valid-cut-instance-offsets/)
    // a variante só de integridade NÃO enxerga esse caso: é por isso que é opt-in e se declara limitada
    const integrityOnly = oracle(["verify", "--fixtures-only"], { fixtures })
    assert.equal(integrityOnly.status, 0)
    assert.match(integrityOnly.out, /NÃO conferido contra a origem/)
  })

  test("veredito invertido de um caso negativo é detectado", () => {
    const fixtures = copyFixtures("flip-verdict")
    const relative = "expected/invalid-orphan-node.expected.json"
    const file = path.join(fixtures, relative)
    const expected = readJson(file)
    expected.validation.accepted = true
    expected.validation.schema.success = true
    expected.validation.schema.issues = []
    writeJson(file, expected)
    rehashInManifest(fixtures, relative)
    const result = oracle(["verify"], { fixtures })
    assert.equal(result.status, 1)
    assert.match(result.err, /EXPECTATION_DIVERGENCE: invalid-orphan-node/)
  })

  test("bundle de entrada alterado: hash do manifest acusa", () => {
    const fixtures = copyFixtures("corrupt-bundle")
    const file = path.join(fixtures, "bundles", "valid-minimal-continuous.bundle.json")
    const bundle = readJson(file)
    bundle.cameraClips[0].initialFovDeg = 46
    writeJson(file, bundle)
    const result = oracle(["verify"], { fixtures })
    assert.equal(result.status, 1)
    assert.match(result.err, /FIXTURE_HASH_MISMATCH: bundles\/valid-minimal-continuous/)
  })

  test("arquivo ausente e arquivo não listado são acusados", () => {
    const fixtures = copyFixtures("missing-unlisted")
    rmSync(path.join(fixtures, "expected", "invalid-edit-gap.expected.json"))
    writeFileSync(path.join(fixtures, "bundles", "stray.bundle.json"), "{}\n")
    const result = oracle(["verify", "--fixtures-only"], { fixtures })
    assert.equal(result.status, 1)
    assert.match(result.err, /FIXTURE_MISSING: expected\/invalid-edit-gap/)
    assert.match(result.err, /FIXTURE_UNLISTED: bundles\/stray/)
  })

  test("manifest ausente falha em vez de passar", () => {
    const fixtures = copyFixtures("no-manifest")
    rmSync(path.join(fixtures, "manifest.json"))
    const result = oracle(["verify"], { fixtures })
    assert.equal(result.status, 1)
    assert.match(result.err, /MANIFEST_MISSING/)
  })
})

describe("verify detecta deriva da origem", env, () => {
  // Recorte de src/ (1,6 MB) numa pasta temporária: a origem real nunca é escrita.
  const copySourceRoot = (label) => {
    const root = freshDir(label)
    cpSync(path.join(REAL.sourceRoot, "src"), path.join(root, "src"), { recursive: true })
    return root
  }

  test("mudança de comportamento: deriva de hash E divergência das expectativas", () => {
    const sourceRoot = copySourceRoot("drift-behaviour")
    const joins = path.join(sourceRoot, "src", "validate", "edit-joins.ts")
    const text = readFileSync(joins, "utf8")
    const needle = "JOIN_VELOCITY_TOLERANCE_SCENE_UNITS_PER_FRAME = 0.08"
    assert.ok(text.includes(needle))
    writeFileSync(joins, text.replace(needle, "JOIN_VELOCITY_TOLERANCE_SCENE_UNITS_PER_FRAME = 0.001"))
    const result = oracle(["verify"], { sourceRoot })
    assert.equal(result.status, 1)
    assert.match(result.err, /SOURCE_DRIFT: src\/validate\/edit-joins\.ts/)
    assert.match(result.err, /EXPECTATION_DIVERGENCE: valid-minimal-continuous/)
  })

  test("mudança só de comentário: deriva de hash sem divergência", () => {
    const sourceRoot = copySourceRoot("drift-comment")
    const easing = path.join(sourceRoot, "src", "camera", "easing.ts")
    writeFileSync(easing, `// edição local\n${readFileSync(easing, "utf8")}`)
    const result = oracle(["verify"], { sourceRoot })
    assert.equal(result.status, 1)
    assert.match(result.err, /SOURCE_DRIFT: src\/camera\/easing\.ts/)
    assert.doesNotMatch(result.err, /EXPECTATION_DIVERGENCE/)
  })

  test("origem ausente é erro de ambiente (2), nunca sucesso", () => {
    const result = oracle(["verify"], { sourceRoot: freshDir("no-source") })
    assert.equal(result.status, 2)
    assert.match(result.err, /Origem indisponível/)
  })

  test("generate recusa uma origem que não dá para provar (sem git/commit)", () => {
    const fixtures = copyFixtures("generate-unprovable")
    const sourceRoot = copySourceRoot("generate-unprovable-source")
    const result = oracle(["generate"], { fixtures, sourceRoot })
    assert.equal(result.status, 2)
    assert.match(result.err, /Não é possível provar o commit fixado/)
    assert.deepEqual(snapshotDir(fixtures), snapshotDir(COMMITTED))
  })

  test("generate recusa gravar proveniência de arquivo diferente do commit fixado", (t) => {
    // Clone --shared SOMENTE LEITURA da origem (usa os objetos dela por alternates, sem escrevê-los),
    // com só `src/` em checkout; depois um arquivo é editado na cópia.
    const clone = freshDir("pinned-clone")
    const subdir = "packages/scene-engine"
    const origin = spawnSync("git", ["-C", REAL.sourceRoot, "rev-parse", "--show-toplevel"], {
      encoding: "utf8",
    })
    if (origin.status !== 0) {
      t.skip("origem sem repositório git")
      return
    }
    const commit = readJson(path.join(COMMITTED, "manifest.json")).source.commit
    const git = (cwd, args) => spawnSync("git", args, { cwd, encoding: "utf8" })
    assert.equal(git(scratch, ["clone", "--shared", "--no-checkout", "--quiet", origin.stdout.trim(), clone]).status, 0)
    assert.equal(git(clone, ["checkout", commit, "--", `${subdir}/src`]).status, 0)
    const sourceRoot = path.join(clone, subdir)

    const fixtures = copyFixtures("generate-mismatch")
    const pristine = oracle(["verify"], { fixtures, sourceRoot })
    assert.equal(pristine.status, 0, `um clone sem edição deve verificar: ${pristine.err}`)

    const easing = path.join(sourceRoot, "src", "camera", "easing.ts")
    writeFileSync(easing, `// edição local\n${readFileSync(easing, "utf8")}`)
    const result = oracle(["generate"], { fixtures, sourceRoot })
    assert.equal(result.status, 1)
    assert.match(result.err, /SOURCE_NOT_AT_PINNED_COMMIT src\/camera\/easing\.ts/)
    assert.deepEqual(snapshotDir(fixtures), snapshotDir(COMMITTED))
  })
})

describe("regeneração e restauração", env, () => {
  test("apagar expectativas e manifest e regenerar restaura bytes idênticos", () => {
    const fixtures = copyFixtures("restore")
    const before = snapshotDir(fixtures)
    rmSync(path.join(fixtures, "expected"), { recursive: true })
    rmSync(path.join(fixtures, "manifest.json"))
    assert.equal(oracle(["verify"], { fixtures }).status, 1)
    const generated = oracle(["generate"], { fixtures })
    assert.equal(generated.status, 0, generated.err)
    assert.deepEqual(snapshotDir(fixtures), before)
    assert.equal(oracle(["verify"], { fixtures }).status, 0)
  })

  test("regenerar sobre uma expectativa corrompida a restaura", () => {
    const fixtures = copyFixtures("restore-corrupt")
    const before = snapshotDir(fixtures)
    const file = path.join(fixtures, "expected", "valid-camera-recipes-layers.expected.json")
    writeFileSync(file, readFileSync(file, "utf8").replace("fovDeg", "fovDegX"))
    assert.equal(oracle(["verify"], { fixtures }).status, 1)
    assert.equal(oracle(["generate"], { fixtures }).status, 0)
    assert.deepEqual(snapshotDir(fixtures), before)
  })

  test("generate falha se um caso não exercita o que diz (corpus mal desenhado)", () => {
    const fixtures = copyFixtures("bad-intent")
    const casesPath = path.join(fixtures, "cases.json")
    const index = readJson(casesPath)
    const entry = index.cases.find((candidate) => candidate.id === "invalid-edit-gap")
    entry.intent.sourceCodes = ["CODIGO_QUE_A_ORIGEM_NAO_PRODUZ"]
    writeJson(casesPath, index)
    const before = snapshotDir(COMMITTED)
    const result = oracle(["generate"], { fixtures })
    assert.equal(result.status, 1)
    assert.match(result.err, /INTENT invalid-edit-gap: código CODIGO_QUE_A_ORIGEM_NAO_PRODUZ/)
    // nada foi regravado: as expectativas seguem as de antes
    for (const name of readdirSync(path.join(fixtures, "expected"))) {
      assert.equal(
        sha256Of(readFileSync(path.join(fixtures, "expected", name))),
        before[`expected/${name}`],
      )
    }
  })
})

describe("uso da CLI", () => {
  test("comando desconhecido devolve 2 e mostra o uso", () => {
    const result = oracle(["nao-existe"])
    assert.equal(result.status, 2)
    assert.match(result.err, /Uso:/)
  })

  test("o repositório do editor não ganha artefatos do oráculo", () => {
    assert.ok(!existsSync(path.join(ORACLE_DIR, "node_modules")))
    assert.ok(!existsSync(path.join(REPO_ROOT, "node_modules")))
  })
})
