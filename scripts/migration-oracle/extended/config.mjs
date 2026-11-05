// Caminhos e constantes do oráculo ESTENDIDO. Reaproveita a resolução de origem/dependências do
// oráculo base (`lib/config.mjs`) e só troca onde as fixtures vivem: um subdiretório próprio, que
// NÃO é lido nem sobrescrito pelo corpus de 55 casos.

import { lstatSync, readFileSync, realpathSync, statSync } from "node:fs"
import path from "node:path"

import { harnessFileHashes, ORACLE_DIR, REPO_ROOT, resolveConfig, sha256Of } from "../lib/config.mjs"
import { EnvironmentError } from "../lib/harness.mjs"

export const EXTENDED_DIR = path.join(ORACLE_DIR, "extended")
export const BASELINE_FIXTURES_DIR = path.join(REPO_ROOT, "docs", "migration", "fixtures")
export const EXTENDED_FIXTURES_DIR = path.join(BASELINE_FIXTURES_DIR, "extended-round2")

export const EXTENDED_FORMAT_VERSION = 1
export const EXTENDED_MANIFEST_VERSION = 1

// Resolve existing ancestors too: a new output can live below a symlinked parent.
// An existing but unresolvable path (including a dangling link) is never a safe output.
const canonicalOutput = (value) => {
  let current = path.resolve(value)
  const suffix = []
  for (;;) {
    try {
      return path.join(realpathSync(current), ...suffix)
    } catch (error) {
      if (error.code !== "ENOENT") throw error
      try {
        lstatSync(current)
      } catch (missing) {
        if (missing.code !== "ENOENT") throw missing
        suffix.unshift(path.basename(current))
        const parent = path.dirname(current)
        if (parent === current) throw error
        current = parent
        continue
      }
      throw new EnvironmentError(`não é possível resolver o destino ${current}`)
    }
  }
}

// A hard-linked metadata file has another pathname but still modifies the same corpus.
const identityOf = (file) => {
  try {
    const { dev, ino } = statSync(file, { bigint: true })
    return ino === 0n ? null : `${dev}:${ino}`
  } catch (error) {
    if (error.code === "ENOENT") return null
    throw error
  }
}

export const assertExtendedOutput = (fixturesDir) => {
  const baseline = canonicalOutput(BASELINE_FIXTURES_DIR)
  const protectedOutputs = ["bundles", "expected", "cases.json", "manifest.json"]
    .map((name) => canonicalOutput(path.join(BASELINE_FIXTURES_DIR, name)))
  const outputs = [fixturesDir, ...["bundles", "expected", "cases.json", "manifest.json", "tolerances.json"]
    .map((name) => path.join(fixturesDir, name))].map(canonicalOutput)
  const protectedIdentities = new Set([baseline, ...protectedOutputs].map(identityOf).filter(Boolean))
  if (outputs.some((output) => protectedIdentities.has(identityOf(output)) || output === baseline || protectedOutputs.some(
    (protectedPath) => output === protectedPath || output.startsWith(`${protectedPath}${path.sep}`),
  ))) {
    throw new EnvironmentError(
      `o corpus estendido não pode ser gerado em ${BASELINE_FIXTURES_DIR} (é o corpus base)`,
    )
  }
}

export const resolveExtendedConfig = (flags) => {
  const base = resolveConfig({ ...flags, fixtures: undefined })
  const requested = flags.fixtures ?? process.env.ORACLE_EXTENDED_FIXTURES_DIR
  const fixturesDir =
    typeof requested === "string" && requested.length > 0
      ? path.resolve(requested)
      : EXTENDED_FIXTURES_DIR
  assertExtendedOutput(fixturesDir)
  return { ...base, fixturesDir }
}

const EXTENDED_GENERATOR_FILES = [
  "extended/config.mjs",
  "extended/core.mjs",
  "extended/corpus.mjs",
  "extended/entry.mjs",
  "extended/numeric.mjs",
  "extended/scope.json",
  "extended/tolerances.json",
  "lib/config.mjs",
  "lib/harness.mjs",
]

/** Hash de tudo que produz o corpus: o harness base (inalterado) e o estendido. */
export const extendedHarnessHashes = () => ({
  ...harnessFileHashes(),
  ...Object.fromEntries(
    EXTENDED_GENERATOR_FILES.map((relative) => [
      `scripts/migration-oracle/${relative}`,
      sha256Of(readFileSync(path.join(ORACLE_DIR, relative))),
    ]),
  ),
})
