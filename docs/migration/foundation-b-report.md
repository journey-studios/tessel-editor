# Relatório da frente B — Oráculo BB10/E0

2026-11-05. Rodada paralela da base de migração ([plano](parallel-implementation-plan.md),
[roadmap](phased-roadmap.md) §4 E0). Checkout `tessel-editor` em `cb1412f` (baseline inicial
`59d618a`), origem `github.com/wallynm/tessel-editor`. Esta frente não fez `git add`/commit/push,
não alterou core/runtime/manifests Cargo e não depende da frente A.

## Entrega

Corpus sintético de **55 bundles** (4 válidos, 51 inválidos) e um runner reproduzível que gera as
expectativas **executando funções puras reais** de `packages/scene-engine` (origem em `d6adac15…`,
somente leitura, sem alterar nenhum arquivo dela). Detalhes e uso:
[README do runner](../../scripts/migration-oracle/README.md), contrato em
[`docs/features/migration-oracle.md`](../features/migration-oracle.md), fixtures em
[`docs/migration/fixtures/`](fixtures/README.md).

### Arquivos (todos novos, dentro do ownership)

- `scripts/migration-oracle/`: `oracle.mjs`, `oracle.spec.mjs`, `README.md`, `package.json`,
  `pnpm-lock.yaml`, `harness/entry.mjs`, `harness/scope.json`, `corpus/builder.mjs`,
  `lib/{config,deps,harness,oracle-core,probes,serialize}.mjs`
- `docs/migration/fixtures/`: `README.md`, `cases.json`, `manifest.json`, `bundles/*.bundle.json` (55),
  `expected/*.expected.json` (55) — ~880 KB
- `docs/features/migration-oracle.md`, `docs/migration/foundation-b-report.md`

### O que é executado da origem

Empacotamento (esbuild 0.25.12) dos arquivos **originais, sem cópia nem extração**, com zod 4.3.6 e
three 0.187.0 (integridades iguais às do lockfile da origem), instalados **fora** do repo e da origem
(`~/.cache/tessel-migration-oracle/<hash>`, `pnpm install --frozen-lockfile --ignore-scripts`).
33 arquivos `.ts` entram no bundle (todos com sha256 e blob git no manifest; o working tree bate com
os blobs de `d6adac15…`; árvore `packages/scene-engine` = `d5fc9871…`, a do inventário).

Chamadas diretas: `renderBundleSchema.safeParse`, `validateBundleIntegrity`, `validateEditJoins`,
`finalizeBundle` (veredito de `scene validate` sem checar arquivos), `validateSceneGraphStructure`
(também sobre JSON cru), `resolveParentLinks`, `computeWorldMatrices`, `evaluateEditFrame`,
`resolveCameraAtClipFrame`, `mainTrackPoses`, `getCameraClipDurationFrames`; por elas rodam
`resolveShotAtEditFrame`, `resolvePerformanceOverrides` (cycle/event/static), easing e keyframes.
Como nada foi extraído, **não há procedimento de extração/escopo reduzido a registrar** além desse
empacotamento direto; isto **não** prova a engine completa.

### Corpus e resultados gerados

- Válidos: mínimo contínuo (2 shots, dolly/hold/truck), câmera rica (zoom, orbit, keyframes, hold,
  2 camadas aditivas), grafo com quaternions/escala/socket + performance (cycle/event/static) e
  duas instâncias com offsets e corte.
- Inválidos: 4 erros estruturais do zod; 40 códigos distintos de integridade/grafo do schema; 9 códigos de
  junção (`validateEditJoins`, com schema aceito); 2 recusas sem código estável (dissolve, canal de
  performance duplicado). Cada caso muda uma coisa de uma base válida.
- Nos válidos: 38 frames de edição (8 deles -1/fim da edição, com a **mensagem real** de erro),
  110 matrizes mundo, 70 frames de clipe de câmera (inclui 5 em -1 e frames após o fim; 13 erros reais gravados no total),
  poses/velocidades por movimento nas bordas (b-1, b, b+1, meio, fim, fim+5).
- A geração confere a **intenção** de cada caso (o código declarado precisa ser produzido pela
  origem, válido precisa ser aceito, inválido recusado); todos os 55 passaram na primeira geração,
  e esse teste é coberto por um caso negativo no spec.

## Comandos executados e resultados

| Comando | Resultado |
|---|---|
| `node scripts/migration-oracle/oracle.mjs setup` | ok (zod 4.3.6, three 0.187.0, esbuild 0.25.12; deps com 40 KB em cache de symlinks, store do pnpm) |
| `oracle.mjs corpus` | 55 bundles escritos |
| `oracle.mjs generate` (2×) | 55 casos / 33 fontes; hash idêntico de `expected/`+`manifest.json` nas duas execuções |
| `oracle.mjs verify` | ok: 55 casos verificados contra a origem (hashes + regeneração byte a byte) |
| `oracle.mjs verify --fixtures-only` | ok, rotulado "NÃO conferido contra a origem" |
| `node --test scripts/migration-oracle/oracle.spec.mjs` | **21/21** passam |

Testes do runner (21): execução do `verify` nas fixtures versionadas; manifest com commit/hashes/
versões; corpus versionado == builder; rejeição traz o código estável; expectativa numérica
adulterada (hash + divergência com caminho); adulterada **e** manifest "consertado" (só a
regeneração pega; `--fixtures-only` não pega, por isso se declara limitado); veredito invertido de
caso negativo; bundle adulterado; arquivo ausente e não listado; manifest ausente; deriva de
comportamento da origem (SOURCE_DRIFT **e** EXPECTATION_DIVERGENCE) e só de comentário (só hash);
origem ausente → exit 2; `generate` recusa origem não provável e arquivo diferente do commit
fixado (clone `--shared` somente leitura da origem); regeneração restaura bytes idênticos (apagando
`expected/`+manifest e corrompendo um arquivo); caso com intenção não produzida; uso da CLI e
nenhum `node_modules` no repo. Todos os testes que corrompem operam em cópias temporárias.

Não executados nesta frente: Cargo, Chrome, GPU, e qualquer coisa na origem além de leitura
(`git rev-parse|ls-tree|cat-file`; `git clone --shared --no-checkout` só nos testes, escrevendo em
tmp). Nenhuma mídia privada copiada. Artefatos próprios: ~1 MB no repo (fixtures) + cache de deps
(~40 KB de symlinks; o store do pnpm já guarda ~18 MB); espaço livre medido ≥ 9 GiB.

## Funções NÃO executadas e lacunas

Lista completa e justificada em [`harness/scope.json`](../../scripts/migration-oracle/harness/scope.json)
(copiada ao manifest). Principais:

- Comandos de câmera/outline/narração, property paths e `editBundle` (BB04), plano de mix,
  legendas, rig/IK/follow/path, luz por keys, clima, efeitos, limites de splat, auditoria de
  projeção, tempo racional como API própria, I/O do bundle e preflight de assets (sharp),
  runtime Three.js/Spark, transporte/áudio, render/ffmpeg, branch piloto.
- Os módulos de narração/outline/rig/path/efeitos/clima/luz **são carregados** (o schema os importa)
  mas o corpus não tem conteúdo desses domínios; só a luz com `parent` inválido os toca.
- Corpus: sem caso de fronteira das tolerâncias `JOIN_*` (um salto isolado só aparece junto de
  `CAMERA_CLIP_OFFSET_BREAK`), FPS sempre 24/1, sem seek aleatório; sem cobertura por linha.
- Mensagens de erro estrutural do zod são específicas da versão 4.3.6; para elas o Rust deve
  comparar veredito e `path`.
- Política numérica do Rust não definida (o oráculo grava doubles exatos; `-0` vira `0` no JSON).

## Dependências e pontos para o coordenador

- Nada em `Cargo.toml`, `Cargo.lock`, `.gitignore` ou índices gerais foi tocado. O repo não ganha
  `node_modules` (deps fora dele); não há necessidade de `.gitignore`.
- Não há consumidor Rust ainda: BB01/BB02/BB03 (frente A e seguintes) podem ler
  `docs/migration/fixtures/{cases.json,bundles,expected}` por caminho relativo à raiz; o esboço
  está no README do runner. Frente A não é dependência desta frente nem o inverso.
- Gate sugerido para integração: `node scripts/migration-oracle/oracle.mjs verify` (exit 0) numa
  máquina com a origem em `…/tesselapp-image-coverage` (ou `--source-root`); sem a origem, só
  `--fixtures-only` (limitado).
- O `.spec.mjs` usa `node:test` (Node ≥ 22.12), sem nova dependência além das três fixadas. Esta
  é uma camada Node apenas de ferramental em `scripts/`, não do produto.

## Linha exata da matriz (`docs/features/README.md`)

ID proposto `F032` (A deve usar `F031` para `directed-scene`; ajuste se a numeração mudar):

```markdown
| F032 | [Oráculo de equivalência da migração (BB10/E0)](migration-oracle.md) | N/A⁹ | N/A⁹ | N/A⁹ | P⁹ | `scripts/migration-oracle/oracle.mjs verify\|generate` + `docs/migration/fixtures/` |
```

Nota de rodapé sugerida:

```markdown
⁹ Ferramenta de desenvolvimento, não uma capacidade do editor: UI, CLI da janela e CLI headless são
N/A (o runner é Node, não é o `tessel-headless`). Lib é P: não há consumidor Rust ainda; o contrato de
dados (55 bundles + expectativas geradas por funções reais de `packages/scene-engine@d6adac15`,
manifest com hashes) está entregue e validado só no lado Node (21/21 em
`scripts/migration-oracle/oracle.spec.mjs`). Cobre validação, grafo, avaliação e câmera; não cobre
narração, clima, luz por keys, efeitos, rig, splat, I/O, runtime nem pixel/áudio. Ver
[relatório](../migration/foundation-b-report.md).
```

## Revisão de integração pelo coordenador

O coordenador repetiu `node scripts/migration-oracle/oracle.mjs verify` (55 casos
contra a origem) e `node --test scripts/migration-oracle/oracle.spec.mjs` (21/21).
A matriz recebeu F032 com consumidor Rust pendente e os índices apontam para o runner.
Somente arquivos da frente B e esses índices entram neste commit; trabalho concorrente
do core e do runtime permanece separado.
