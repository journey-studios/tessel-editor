// Caminhos e constantes do oráculo ESTENDIDO. Reaproveita a resolução de origem/dependências do
// oráculo base (`lib/config.mjs`) e só troca onde as fixtures vivem: um subdiretório próprio, que
// NÃO é lido nem sobrescrito pelo corpus de 55 casos.

import { readFileSync } from "node:fs"
import path from "node:path"

import { harnessFileHashes, ORACLE_DIR, REPO_ROOT, resolveConfig, sha256Of } from "../lib/config.mjs"
import { EnvironmentError } from "../lib/harness.mjs"

export const EXTENDED_DIR = path.join(ORACLE_DIR, "extended")
export const BASELINE_FIXTURES_DIR = path.join(REPO_ROOT, "docs", "migration", "fixtures")
export const EXTENDED_FIXTURES_DIR = path.join(BASELINE_FIXTURES_DIR, "extended-round2")

export const EXTENDED_FORMAT_VERSION = 1
export const EXTENDED_MANIFEST_VERSION = 1

export const resolveExtendedConfig = (flags) => {
  const base = resolveConfig({ ...flags, fixtures: undefined })
  const requested = flags.fixtures ?? process.env.ORACLE_EXTENDED_FIXTURES_DIR
  const fixturesDir =
    typeof requested === "string" && requested.length > 0
      ? path.resolve(requested)
      : EXTENDED_FIXTURES_DIR
  // O corpus de 55 casos e o manifest dele são intocáveis por esta ferramenta.
  if (fixturesDir === BASELINE_FIXTURES_DIR) {
    throw new EnvironmentError(
      `o corpus estendido não pode ser gerado em ${BASELINE_FIXTURES_DIR} (é o corpus base)`,
    )
  }
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
