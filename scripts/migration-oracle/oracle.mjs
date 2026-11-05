#!/usr/bin/env node
// CLI do oráculo de migração (BB10/E0). Ver scripts/migration-oracle/README.md.
//
// Códigos de saída: 0 = ok; 1 = corrupção, deriva da origem ou divergência detectada;
// 2 = ambiente/uso (origem ou dependências ausentes, argumento inválido).

import { resolveConfig } from "./lib/config.mjs"
import { setupDeps } from "./lib/deps.mjs"
import { EnvironmentError } from "./lib/harness.mjs"
import { generate, verify, writeCorpus } from "./lib/oracle-core.mjs"

const USAGE = `Uso: node scripts/migration-oracle/oracle.mjs <comando> [opções]

Comandos:
  setup     instala as dependências fixadas FORA do repositório (pnpm, --frozen-lockfile)
  corpus    reescreve bundles e cases.json a partir de corpus/builder.mjs (não usa a origem)
  generate  executa as funções REAIS da origem e grava expected/ e manifest.json
  verify    confere fixtures, deriva da origem e divergência das expectativas
            --fixtures-only  só integridade das fixtures; NÃO compara com a origem
  status    mostra caminhos resolvidos

Opções: --fixtures DIR  --source-root DIR  --deps-dir DIR
(equivalentes: ORACLE_FIXTURES_DIR, ORACLE_SOURCE_ROOT, ORACLE_DEPS_DIR)`

const parseArgs = (argv) => {
  const [command, ...rest] = argv
  const flags = {}
  for (let index = 0; index < rest.length; index += 1) {
    const arg = rest[index]
    if (!arg.startsWith("--")) {
      throw new EnvironmentError(`argumento inesperado: ${arg}`)
    }
    const name = arg.slice(2)
    if (name === "fixtures-only") {
      flags[name] = true
      continue
    }
    const value = rest[index + 1]
    if (value === undefined || value.startsWith("--")) {
      throw new EnvironmentError(`--${name} exige um valor`)
    }
    flags[name] = value
    index += 1
  }
  return { command, flags }
}

const printErrors = (errors) => {
  for (const error of errors) {
    console.error(`FAIL ${error.category}: ${error.detail}`)
  }
}

const run = async () => {
  const { command, flags } = parseArgs(process.argv.slice(2))
  const config = resolveConfig(flags)
  if (command === "status") {
    console.log(JSON.stringify(config, null, 2))
    return 0
  }
  if (command === "setup") {
    setupDeps(config.depsDir)
    console.log(`ok: dependências em ${config.depsDir}`)
    return 0
  }
  if (command === "corpus") {
    console.log(`ok: ${writeCorpus(config.fixturesDir)} bundles escritos em ${config.fixturesDir}`)
    return 0
  }
  if (command === "generate") {
    const result = await generate(config)
    if (!result.ok) {
      printErrors(result.errors.map((detail) => ({ category: "GENERATE", detail })))
      return 1
    }
    console.log(`ok: ${result.cases} casos gerados a partir de ${result.sourceFiles} arquivos da origem`)
    return 0
  }
  if (command === "verify") {
    const result = await verify(config, { fixturesOnly: flags["fixtures-only"] === true })
    for (const warning of result.warnings) {
      console.error(`WARN ${warning}`)
    }
    if (result.errors.length > 0) {
      printErrors(result.errors)
      console.error(`verify FALHOU: ${result.errors.length} problema(s) em ${result.cases} casos`)
      return 1
    }
    const scope = result.checkedAgainstSource
      ? "contra a origem (hashes + regeneração byte a byte)"
      : "SOMENTE integridade das fixtures (NÃO conferido contra a origem)"
    console.log(`ok: ${result.cases} casos verificados ${scope}`)
    return 0
  }
  console.error(USAGE)
  return 2
}

run().then(
  (code) => {
    process.exitCode = code
  },
  (error) => {
    console.error(error instanceof EnvironmentError ? `ERRO DE AMBIENTE: ${error.message}` : error)
    process.exitCode = 2
  },
)
