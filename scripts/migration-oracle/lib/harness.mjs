// Empacota o harness com as fontes REAIS da origem (somente leitura) e o carrega em memória.
// Nada é escrito na origem; o bundle gerado fica no diretório de dependências, fora do repositório.

import { spawnSync } from "node:child_process"
import { existsSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs"
import { createRequire } from "node:module"
import path from "node:path"
import { pathToFileURL } from "node:url"

import { gitBlobIdOf, ORACLE_DIR, sha256Of } from "./config.mjs"

const ENTRY = realpathSync(path.join(ORACLE_DIR, "harness", "entry.mjs"))
// Pacotes que as fontes importam por nome: resolvidos SEMPRE das versões fixadas deste diretório,
// nunca de um node_modules que por acaso exista perto da origem.
const PINNED_PACKAGES = ["zod", "three"]

export class EnvironmentError extends Error {}

export const depsInstalled = (depsDir) =>
  PINNED_PACKAGES.concat("esbuild").every((name) =>
    existsSync(path.join(depsDir, "node_modules", name, "package.json")),
  )

const requireFromDeps = (depsDir) =>
  createRequire(path.join(depsDir, "package.json"))

const readPackageVersion = (depsDir, name) =>
  JSON.parse(
    readFileSync(path.join(depsDir, "node_modules", name, "package.json"), "utf8"),
  ).version

export const installedToolVersions = (depsDir) => ({
  esbuild: readPackageVersion(depsDir, "esbuild"),
  three: readPackageVersion(depsDir, "three"),
  zod: readPackageVersion(depsDir, "zod"),
})

const originPlugin = (sourceRoot, depsDir) => ({
  name: "oracle-origin",
  setup(build) {
    const srcDir = path.join(sourceRoot, "src")
    build.onResolve({ filter: /^@origin\// }, (args) =>
      build.resolve(`./${args.path.slice("@origin/".length)}`, {
        kind: args.kind,
        resolveDir: srcDir,
      }),
    )
    // `build.resolve` volta a passar por este mesmo onResolve; `pluginData` corta a recursão
    // (sem isso o esbuild reentra para sempre e estoura a memória).
    build.onResolve({ filter: /^(zod|three)$/ }, (args) => {
      if (args.pluginData?.oracleResolved) {
        return undefined
      }
      return build.resolve(args.path, {
        kind: args.kind,
        resolveDir: depsDir,
        pluginData: { oracleResolved: true },
      })
    })
  },
})

/**
 * @returns {Promise<{ module: any, sources: SourceFile[], sourceRoot: string }>}
 */
export const buildHarness = async (config) => {
  if (!existsSync(path.join(config.sourceRoot, "src"))) {
    throw new EnvironmentError(
      `Origem indisponível: ${path.join(config.sourceRoot, "src")} não existe (use --source-root ou ORACLE_SOURCE_ROOT)`,
    )
  }
  if (!depsInstalled(config.depsDir)) {
    throw new EnvironmentError(
      `Dependências ausentes em ${config.depsDir}. Rode: node scripts/migration-oracle/oracle.mjs setup`,
    )
  }
  // O esbuild devolve caminhos reais (sem symlinks, ex.: /var -> /private/var no macOS); os
  // prefixos usados para separar "fonte da origem" de "dependência" precisam estar no mesmo formato.
  const sourceRoot = realpathSync(config.sourceRoot)
  const depsDir = realpathSync(config.depsDir)
  const esbuild = requireFromDeps(depsDir)("esbuild")
  const result = await esbuild.build({
    entryPoints: [ENTRY],
    absWorkingDir: depsDir,
    bundle: true,
    write: false,
    metafile: true,
    format: "esm",
    platform: "node",
    target: "node22",
    legalComments: "none",
    logLevel: "silent",
    plugins: [originPlugin(sourceRoot, depsDir)],
  })
  const output = result.outputFiles[0].text
  const outDir = path.join(depsDir, "build")
  mkdirSync(outDir, { recursive: true })
  const bundleSha256 = sha256Of(output)
  const outFile = path.join(outDir, `harness-${bundleSha256.slice(0, 16)}.mjs`)
  writeFileSync(outFile, output)
  const module = await import(pathToFileURL(outFile).href)
  // O módulo já está em memória; não deixa um bundle de ~1 MB por execução no diretório de deps.
  rmSync(outFile)
  const sources = collectSources(result.metafile, sourceRoot, depsDir)
  if (sources.length === 0) {
    // Defesa: um prefixo de caminho errado já fez o runner "verificar" zero arquivos da origem.
    throw new Error(`nenhuma fonte de ${sourceRoot} foi empacotada; o harness não está lendo a origem`)
  }
  return { module, sources, sourceRoot }
}

/**
 * @typedef {{ path: string, sha256: string, gitBlob: string, bytes: number, bytesInOutput: number, role: "called-directly" | "loaded-transitively" }} SourceFile
 */
const collectSources = (metafile, sourceRoot, depsDir) => {
  const entryKey = Object.keys(metafile.inputs).find(
    (key) => path.resolve(depsDir, key) === ENTRY,
  )
  const directImports = new Set(
    (metafile.inputs[entryKey]?.imports ?? []).map((imported) =>
      path.resolve(depsDir, imported.path),
    ),
  )
  const outputInputs = Object.values(metafile.outputs)[0].inputs
  const prefix = `${sourceRoot}${path.sep}`
  return Object.keys(metafile.inputs)
    .map((key) => path.resolve(depsDir, key))
    .filter((absolute) => absolute.startsWith(prefix))
    .sort()
    .map((absolute) => {
      const bytes = readFileSync(absolute)
      const outputKey = Object.keys(outputInputs).find(
        (key) => path.resolve(depsDir, key) === absolute,
      )
      return {
        path: path.relative(sourceRoot, absolute).split(path.sep).join("/"),
        sha256: sha256Of(bytes),
        gitBlob: gitBlobIdOf(bytes),
        bytes: bytes.length,
        bytesInOutput: outputInputs[outputKey]?.bytesInOutput ?? 0,
        role: directImports.has(absolute) ? "called-directly" : "loaded-transitively",
      }
    })
}

const git = (cwd, args) =>
  spawnSync("git", args, { cwd, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 })

/**
 * Compara os arquivos efetivamente empacotados com os blobs do commit fixado (somente leitura:
 * `git rev-parse`, `git ls-tree`). Sem repositório git ou sem o commit, devolve `available: false`.
 */
export const pinnedBlobCheck = (sourceRoot, commit, sources) => {
  const top = git(sourceRoot, ["rev-parse", "--show-toplevel"])
  if (top.status !== 0) {
    return { available: false, reason: "origem não está num repositório git" }
  }
  const repoRoot = top.stdout.trim()
  const exists = git(repoRoot, ["cat-file", "-e", `${commit}^{commit}`])
  if (exists.status !== 0) {
    return { available: false, reason: `commit ${commit} ausente no repositório da origem` }
  }
  const relativeRoot = path.relative(repoRoot, sourceRoot).split(path.sep).join("/")
  const tree = git(repoRoot, ["ls-tree", "-r", commit, "--", relativeRoot])
  const blobs = new Map()
  for (const line of tree.stdout.split("\n").filter(Boolean)) {
    const [meta, file] = line.split("\t")
    blobs.set(file, meta.split(" ")[2])
  }
  const mismatches = sources.flatMap((source) => {
    const pinned = blobs.get(`${relativeRoot}/${source.path}`)
    return pinned === source.gitBlob
      ? []
      : [{ path: source.path, workingTreeBlob: source.gitBlob, pinnedBlob: pinned ?? null }]
  })
  const subtree = git(repoRoot, ["rev-parse", `${commit}:${relativeRoot}`])
  return {
    available: true,
    repoHead: git(repoRoot, ["rev-parse", "HEAD"]).stdout.trim(),
    packageTreeAtCommit: subtree.stdout.trim(),
    mismatches,
  }
}
