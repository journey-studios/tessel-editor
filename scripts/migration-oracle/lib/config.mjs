// Resolução de caminhos e constantes do oráculo. Tudo que muda entre máquinas (origem, fixtures,
// dependências) pode ser sobrescrito por flag/variável, para que os testes operem em cópias
// temporárias sem tocar no checkout compartilhado.

import { createHash } from "node:crypto"
import { readFileSync } from "node:fs"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

export const ORACLE_DIR = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
)
export const REPO_ROOT = path.resolve(ORACLE_DIR, "..", "..")

/** Snapshot do inventário `docs/migration/README.md`: HEAD da origem auditada. */
export const PINNED_COMMIT = "d6adac15c61f4ef582a7ec259397b2e025d1f059"
export const PINNED_BRANCH = "feat/image-coverage-r8"
/** Pacote da origem de onde vêm as funções puras, relativo à raiz do repositório da origem. */
export const SOURCE_PACKAGE_SUBDIR = "packages/scene-engine"
const DEFAULT_SOURCE_ROOT =
  "/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine"

export const MANIFEST_VERSION = 1
export const EXPECTED_FORMAT_VERSION = 1

export const sha256Of = (bytes) =>
  createHash("sha256").update(bytes).digest("hex")

/** Mesmo id que `git hash-object`, para cruzar com `git ls-tree` do commit fixado. */
export const gitBlobIdOf = (bytes) =>
  createHash("sha1")
    .update(`blob ${bytes.length}\0`)
    .update(bytes)
    .digest("hex")

const fileSha256 = (file) => sha256Of(readFileSync(file))

/**
 * As dependências ficam FORA do repositório (o checkout é compartilhado e não ignora
 * `node_modules` aqui). O diretório é endereçado pelo conteúdo de package.json + lockfile.
 */
export const defaultDepsDir = () => {
  const key = createHash("sha256")
    .update(readFileSync(path.join(ORACLE_DIR, "package.json")))
    .update(readFileSync(path.join(ORACLE_DIR, "pnpm-lock.yaml")))
    .digest("hex")
    .slice(0, 12)
  return path.join(os.homedir(), ".cache", "tessel-migration-oracle", key)
}

/**
 * @param {Record<string, string | boolean>} flags
 */
export const resolveConfig = (flags) => {
  const pick = (flag, envName, fallback) => {
    const value = flags[flag] ?? process.env[envName]
    return typeof value === "string" && value.length > 0
      ? path.resolve(value)
      : fallback
  }
  return {
    fixturesDir: pick(
      "fixtures",
      "ORACLE_FIXTURES_DIR",
      path.join(REPO_ROOT, "docs", "migration", "fixtures"),
    ),
    sourceRoot: pick("source-root", "ORACLE_SOURCE_ROOT", DEFAULT_SOURCE_ROOT),
    depsDir: pick("deps-dir", "ORACLE_DEPS_DIR", defaultDepsDir()),
  }
}

export const harnessFileHashes = () => {
  const files = [
    "harness/entry.mjs",
    "harness/scope.json",
    "lib/probes.mjs",
    "lib/serialize.mjs",
    "package.json",
    "pnpm-lock.yaml",
  ]
  return Object.fromEntries(
    files.map((relative) => [
      `scripts/migration-oracle/${relative}`,
      fileSha256(path.join(ORACLE_DIR, relative)),
    ]),
  )
}
