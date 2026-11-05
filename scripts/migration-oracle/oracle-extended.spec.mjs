// Testes do runner do oráculo ESTENDIDO (K / BL018, rodada 2). Rodar com:
//   node --test scripts/migration-oracle/oracle-extended.spec.mjs
//
// Tudo que corrompe ou regenera opera em CÓPIAS temporárias (fixtures e, para deriva, um recorte de
// `src/` da origem). O corpus estendido versionado, o corpus base de 55 casos e a origem nunca são
// alterados por estes testes (o último bloco prova isso contra o git).

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
import { BASELINE_FIXTURES_DIR, EXTENDED_FIXTURES_DIR } from "./extended/config.mjs"

const CLI = path.join(ORACLE_DIR, "extended.mjs")
const REAL = resolveConfig({})
const COMMITTED = EXTENDED_FIXTURES_DIR
const environmentReady =
  existsSync(path.join(REAL.sourceRoot, "src")) && depsInstalled(REAL.depsDir)

const scratch = mkdtempSync(path.join(os.tmpdir(), "oracle-extended-spec-"))
after(() => rmSync(scratch, { recursive: true, force: true }))

let counter = 0
const freshDir = (label) => path.join(scratch, `${label}-${(counter += 1)}`)

const copyFixtures = (label) => {
  const target = freshDir(label)
  cpSync(COMMITTED, target, { recursive: true })
  return target
}

const extended = (args, { fixtures, sourceRoot } = {}) => {
  const result = spawnSync("node", [CLI, ...args], {
    encoding: "utf8",
    env: {
      ...process.env,
      ORACLE_EXTENDED_FIXTURES_DIR: fixtures ?? COMMITTED,
      ORACLE_SOURCE_ROOT: sourceRoot ?? REAL.sourceRoot,
      ORACLE_DEPS_DIR: REAL.depsDir,
    },
  })
  return { status: result.status, out: result.stdout, err: result.stderr }
}

const readJson = (file) => JSON.parse(readFileSync(file, "utf8"))
const writeJson = (file, value) => writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`)

const expectedPath = (fixtures, id) => path.join(fixtures, "expected", `${id}.expected.json`)

/** Simula quem "arruma" o manifest depois de editar uma expectativa à mão. */
const rehashInManifest = (fixtures, relative) => {
  const manifestPath = path.join(fixtures, "manifest.json")
  const manifest = readJson(manifestPath)
  const record = manifest.fixtureFiles.find((file) => file.path === relative)
  record.sha256 = sha256Of(readFileSync(path.join(fixtures, relative)))
  writeJson(manifestPath, manifest)
}

/** Edita uma expectativa, 'conserta' o manifest e devolve o resultado do verify completo. */
const verifyAfterEditing = (label, id, edit) => {
  const fixtures = copyFixtures(label)
  const file = expectedPath(fixtures, id)
  const expected = readJson(file)
  edit(expected)
  writeJson(file, expected)
  rehashInManifest(fixtures, `expected/${id}.expected.json`)
  return { fixtures, result: extended(["verify"], { fixtures }) }
}

const snapshotDir = (dir) =>
  Object.fromEntries(
    ["bundles", "expected"]
      .flatMap((sub) => readdirSync(path.join(dir, sub)).map((name) => `${sub}/${name}`))
      .concat(["cases.json", "manifest.json", "tolerances.json"].filter((name) => existsSync(path.join(dir, name))))
      .sort()
      .map((relative) => [relative, sha256Of(readFileSync(path.join(dir, relative)))]),
  )

const env = { skip: environmentReady ? false : "origem ou dependências ausentes (rode `setup`)" }

describe("corpus estendido versionado", env, () => {
  test("verify passa: hashes da origem, tolerâncias, intenção e regeneração byte a byte", () => {
    const result = extended(["verify"])
    assert.equal(result.status, 0, result.err)
    assert.match(result.out, /contra a origem/)
  })

  test("o manifest fixa commit, hashes das fontes executadas, do gerador e das fixtures", () => {
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
    assert.ok(
      Object.keys(manifest.harness).some((file) => file.endsWith("extended/corpus.mjs")),
      "o hash do gerador do corpus estendido está no manifest",
    )
    // 47 bundles + 47 expectativas + cases.json + tolerances.json
    assert.equal(manifest.fixtureFiles.length, 47 * 2 + 2)
    assert.ok(manifest.fixtureFiles.every((file) => /^[0-9a-f]{64}$/.test(file.sha256)))
  })

  test("o resumo e o escopo declaram o que é decidido pela origem e o que não é exportado", () => {
    const manifest = readJson(path.join(COMMITTED, "manifest.json"))
    assert.equal(manifest.summary.cases, 47)
    assert.equal(manifest.summary.originFloatArtifacts, 14)
    assert.equal(manifest.summary.originDecidedBoundaries.length, 6)
    assert.ok(manifest.scope.dependsOnNonExportedMethod.length >= 3)
    assert.equal(manifest.originJoinConstants.JOIN_POSITION_TOLERANCE_SCENE_UNITS, 0.25)
  })

  test("cobre as quatro famílias e os fps pedidos", () => {
    const { cases } = readJson(path.join(COMMITTED, "cases.json"))
    for (const family of ["join-boundary", "fps", "out-of-order", "world-matrix"]) {
      assert.ok(cases.some((entry) => entry.family === family), family)
    }
    for (const id of [
      "fps-24000-1001",
      "fps-30000-1001",
      "fps-60000-1001",
      "fps-25",
      "fps-30",
      "fps-60",
    ]) {
      assert.ok(cases.some((entry) => entry.id === id), id)
    }
    for (const entry of cases) {
      assert.ok(existsSync(path.join(COMMITTED, entry.bundle)), entry.bundle)
      assert.ok(existsSync(path.join(COMMITTED, entry.expected)), entry.expected)
    }
  })

  test("o corpus versionado é exatamente o que o builder produz", () => {
    const rebuilt = freshDir("rebuilt")
    const result = extended(["corpus"], { fixtures: rebuilt })
    assert.equal(result.status, 0, result.err)
    for (const name of readdirSync(path.join(COMMITTED, "bundles"))) {
      assert.equal(
        readFileSync(path.join(rebuilt, "bundles", name), "utf8"),
        readFileSync(path.join(COMMITTED, "bundles", name), "utf8"),
        name,
      )
    }
    for (const name of ["cases.json", "tolerances.json"]) {
      assert.equal(
        readFileSync(path.join(rebuilt, name), "utf8"),
        readFileSync(path.join(COMMITTED, name), "utf8"),
        name,
      )
    }
  })

  test("a fronteira de junção é estritamente maior: igual ao limite não é salto", () => {
    const equal = readJson(expectedPath(COMMITTED, "join-position-equal"))
    const above = readJson(expectedPath(COMMITTED, "join-position-above"))
    assert.equal(equal.validation.accepted, true)
    assert.equal(above.validation.accepted, false)
    assert.ok(above.validation.editJoins.some((join) => join.code === "CONTINUOUS_CAMERA_JUMP"))
  })
})

describe("verify detecta corrupção e divergência (exit != 0)", env, () => {
  test("expectativa numérica adulterada: hash e divergência acusam o caso", () => {
    const fixtures = copyFixtures("corrupt-number")
    const file = expectedPath(fixtures, "join-position-below")
    const expected = readJson(file)
    const frame = expected.evaluation.editFrames.find((candidate) => candidate.ok)
    frame.state.camera.position.z += 0.001
    writeJson(file, expected)
    const result = extended(["verify"], { fixtures })
    assert.equal(result.status, 1)
    assert.match(result.err, /FIXTURE_HASH_MISMATCH: expected\/join-position-below/)
    assert.match(result.err, /EXPECTATION_DIVERGENCE: join-position-below .*position\.z/)
  })

  test("expectativa de tempo adulterada COM manifest 'consertado': a regeneração pega", () => {
    const { fixtures, result } = verifyAfterEditing("time-rehashed", "fps-30000-1001", (expected) => {
      expected.time.frames[1].seconds += 1e-9
    })
    assert.equal(result.status, 1)
    assert.doesNotMatch(result.err, /FIXTURE_HASH_MISMATCH/)
    assert.match(result.err, /EXPECTATION_DIVERGENCE: fps-30000-1001/)
    // a variante só de integridade NÃO enxerga: é por isso que é opt-in e se declara limitada
    const integrityOnly = extended(["verify", "--fixtures-only"], { fixtures })
    assert.equal(integrityOnly.status, 0)
    assert.match(integrityOnly.out, /NÃO conferido contra a origem/)
  })

  test("veredito de junção invertido é detectado", () => {
    const { result } = verifyAfterEditing("flip-join", "join-position-above", (expected) => {
      expected.validation.accepted = true
      expected.validation.editJoins = []
      expected.validation.finalize = { ok: true }
    })
    assert.equal(result.status, 1)
    assert.match(result.err, /EXPECTATION_DIVERGENCE: join-position-above/)
  })

  test("marcar um artefato de ponto flutuante como exato é detectado", () => {
    const { result } = verifyAfterEditing("artifact-flag", "fps-30000-1001", (expected) => {
      const probe = expected.time.seconds.find((candidate) => candidate.originMatchesExact === false)
      assert.ok(probe, "o caso tem um artefato do floor em ponto flutuante")
      probe.originMatchesExact = true
    })
    assert.equal(result.status, 1)
    assert.match(result.err, /EXPECTATION_DIVERGENCE: fps-30000-1001/)
  })

  test("resultado de avaliação fora de ordem adulterado é detectado", () => {
    const { result } = verifyAfterEditing("order-hash", "ooo-shots-instances-layers", (expected) => {
      expected.orders.results[0].resultsSha256 = "0".repeat(64)
    })
    assert.equal(result.status, 1)
    assert.match(result.err, /EXPECTATION_DIVERGENCE: ooo-shots-instances-layers/)
  })

  test("matriz de câmera adulterada é detectada", () => {
    const { result } = verifyAfterEditing("camera-matrix", "world-camera-lookat-poses", (expected) => {
      const entry = expected.cameraWorld.find((candidate) => candidate.ok)
      entry.matrix[12] += 1e-6
    })
    assert.equal(result.status, 1)
    assert.match(result.err, /EXPECTATION_DIVERGENCE: world-camera-lookat-poses .*cameraWorld/)
  })

  test("bundle de entrada alterado: hash do manifest acusa", () => {
    const fixtures = copyFixtures("corrupt-bundle")
    const file = path.join(fixtures, "bundles", "join-position-equal.bundle.json")
    const bundle = readJson(file)
    bundle.cameraClips[0].initialFovDeg += 1
    writeJson(file, bundle)
    const result = extended(["verify"], { fixtures })
    assert.equal(result.status, 1)
    assert.match(result.err, /FIXTURE_HASH_MISMATCH: bundles\/join-position-equal/)
  })

  test("tolerâncias das fixtures diferentes das do gerador: deriva acusada", () => {
    const fixtures = copyFixtures("tolerances-drift")
    const file = path.join(fixtures, "tolerances.json")
    const tolerances = readJson(file)
    tolerances.numeric.worldMatrix.absolute = 1e-6
    writeJson(file, tolerances)
    rehashInManifest(fixtures, "tolerances.json")
    const result = extended(["verify"], { fixtures })
    assert.equal(result.status, 1)
    assert.match(result.err, /TOLERANCES_DRIFT/)
  })

  test("arquivo ausente e arquivo não listado são acusados", () => {
    const fixtures = copyFixtures("missing-unlisted")
    rmSync(expectedPath(fixtures, "fps-25"))
    writeFileSync(path.join(fixtures, "bundles", "stray.bundle.json"), "{}\n")
    const result = extended(["verify", "--fixtures-only"], { fixtures })
    assert.equal(result.status, 1)
    assert.match(result.err, /FIXTURE_MISSING: expected\/fps-25/)
    assert.match(result.err, /FIXTURE_UNLISTED: bundles\/stray/)
  })

  test("manifest ausente falha em vez de passar", () => {
    const fixtures = copyFixtures("no-manifest")
    rmSync(path.join(fixtures, "manifest.json"))
    const result = extended(["verify"], { fixtures })
    assert.equal(result.status, 1)
    assert.match(result.err, /MANIFEST_MISSING/)
  })
})

describe("verify detecta deriva da origem", env, () => {
  // Recorte de src/ numa pasta temporária: a origem real nunca é escrita.
  const copySourceRoot = (label) => {
    const root = freshDir(label)
    cpSync(path.join(REAL.sourceRoot, "src"), path.join(root, "src"), { recursive: true })
    return root
  }

  test("mudança de comportamento (limite de junção 0.25 → 0.26): deriva E divergência", () => {
    const sourceRoot = copySourceRoot("drift-join")
    const joins = path.join(sourceRoot, "src", "validate", "edit-joins.ts")
    const text = readFileSync(joins, "utf8")
    const needle = "JOIN_POSITION_TOLERANCE_SCENE_UNITS = 0.25"
    assert.ok(text.includes(needle))
    writeFileSync(joins, text.replace(needle, "JOIN_POSITION_TOLERANCE_SCENE_UNITS = 0.26"))
    const result = extended(["verify"], { sourceRoot })
    assert.equal(result.status, 1)
    assert.match(result.err, /SOURCE_DRIFT: src\/validate\/edit-joins\.ts/)
    assert.match(result.err, /TOLERANCES_JOIN_CONSTANT: JOIN_POSITION_TOLERANCE_SCENE_UNITS/)
    // 0.2563 deixa de ser salto com o limite em 0.26: o caso "above" muda de veredito
    assert.match(result.err, /EXPECTATION_DIVERGENCE: join-position-above/)
  })

  test("mudança só de comentário: deriva de hash sem divergência", () => {
    const sourceRoot = copySourceRoot("drift-comment")
    const easing = path.join(sourceRoot, "src", "camera", "easing.ts")
    writeFileSync(easing, `// edição local\n${readFileSync(easing, "utf8")}`)
    const result = extended(["verify"], { sourceRoot })
    assert.equal(result.status, 1)
    assert.match(result.err, /SOURCE_DRIFT: src\/camera\/easing\.ts/)
    assert.doesNotMatch(result.err, /EXPECTATION_DIVERGENCE/)
  })

  test("origem ausente é erro de ambiente (2), nunca sucesso", () => {
    const result = extended(["verify"], { sourceRoot: freshDir("no-source") })
    assert.equal(result.status, 2)
    assert.match(result.err, /Origem indisponível/)
  })

  test("generate recusa uma origem que não dá para provar (sem git/commit)", () => {
    const fixtures = copyFixtures("generate-unprovable")
    const sourceRoot = copySourceRoot("generate-unprovable-source")
    const result = extended(["generate"], { fixtures, sourceRoot })
    assert.equal(result.status, 2)
    assert.match(result.err, /Não é possível provar o commit fixado/)
    assert.deepEqual(snapshotDir(fixtures), snapshotDir(COMMITTED))
  })
})

describe("generate: intenção, isolamento do corpus base e idempotência", env, () => {
  test("generate falha se um caso não exercita o que diz (corpus mal desenhado)", () => {
    const fixtures = copyFixtures("bad-intent")
    const casesPath = path.join(fixtures, "cases.json")
    const index = readJson(casesPath)
    const entry = index.cases.find((candidate) => candidate.id === "join-position-above")
    entry.intent.accepted = true
    writeJson(casesPath, index)
    const before = snapshotDir(COMMITTED)
    const result = extended(["generate"], { fixtures })
    assert.equal(result.status, 1)
    assert.match(result.err, /INTENT join-position-above: veredito accepted=false/)
    for (const name of readdirSync(path.join(fixtures, "expected"))) {
      assert.equal(
        sha256Of(readFileSync(path.join(fixtures, "expected", name))),
        before[`expected/${name}`],
        `${name} foi regravado apesar do erro`,
      )
    }
  })

  test("generate e verify recusam o diretório do corpus base de 55 casos", () => {
    const before = snapshotDir(BASELINE_FIXTURES_DIR)
    for (const command of ["generate", "verify", "corpus"]) {
      const result = spawnSync("node", [CLI, command, "--fixtures", BASELINE_FIXTURES_DIR], {
        encoding: "utf8",
        env: { ...process.env, ORACLE_SOURCE_ROOT: REAL.sourceRoot, ORACLE_DEPS_DIR: REAL.depsDir },
      })
      assert.equal(result.status, 2, `${command}: ${result.stderr}`)
      assert.match(result.stderr, /não pode ser gerado em .* \(é o corpus base\)/)
    }
    assert.deepEqual(snapshotDir(BASELINE_FIXTURES_DIR), before)
  })

  test("apagar expectativas e manifest e regenerar restaura bytes idênticos (idempotente)", () => {
    const fixtures = copyFixtures("restore")
    const before = snapshotDir(fixtures)
    rmSync(path.join(fixtures, "expected"), { recursive: true })
    rmSync(path.join(fixtures, "manifest.json"))
    assert.equal(extended(["verify"], { fixtures }).status, 1)
    const generated = extended(["generate"], { fixtures })
    assert.equal(generated.status, 0, generated.err)
    assert.deepEqual(snapshotDir(fixtures), before)
    const again = extended(["generate"], { fixtures })
    assert.equal(again.status, 0, again.err)
    assert.deepEqual(snapshotDir(fixtures), before)
    assert.equal(extended(["verify"], { fixtures }).status, 0)
  })
})

describe("o corpus base de 55 casos segue intocado", () => {
  test("nenhum arquivo do corpus base mudou em relação ao git", () => {
    const base = path.relative(REPO_ROOT, BASELINE_FIXTURES_DIR)
    const paths = ["bundles", "expected", "cases.json", "manifest.json"].map((name) => path.join(base, name))
    const result = spawnSync("git", ["-C", REPO_ROOT, "status", "--porcelain", "--", ...paths], {
      encoding: "utf8",
    })
    if (result.status !== 0) {
      return
    }
    assert.equal(result.stdout.trim(), "", `o corpus base foi alterado:\n${result.stdout}`)
  })
})
