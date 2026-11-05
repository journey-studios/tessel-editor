// Instalação isolada das dependências fixadas, FORA do repositório do editor e da origem.

import { spawnSync } from "node:child_process"
import { copyFileSync, mkdirSync } from "node:fs"
import path from "node:path"

import { ORACLE_DIR } from "./config.mjs"
import { depsInstalled, EnvironmentError } from "./harness.mjs"

export const setupDeps = (depsDir) => {
  mkdirSync(depsDir, { recursive: true })
  for (const file of ["package.json", "pnpm-lock.yaml"]) {
    copyFileSync(path.join(ORACLE_DIR, file), path.join(depsDir, file))
  }
  // `--ignore-scripts`: nenhum script de pós-instalação roda; esbuild funciona pelo binário
  // opcional da plataforma. `--frozen-lockfile`: o lockfile versionado é a autoridade.
  const result = spawnSync(
    "pnpm",
    ["install", "--frozen-lockfile", "--ignore-scripts"],
    { cwd: depsDir, stdio: "inherit" },
  )
  if (result.status !== 0) {
    throw new EnvironmentError(`pnpm install falhou em ${depsDir}`)
  }
  if (!depsInstalled(depsDir)) {
    throw new EnvironmentError(`instalação incompleta em ${depsDir}`)
  }
}
