# Oráculo de migração (BB10 / E0)

Gera e verifica **expectativas de equivalência** para a migração do Scene Studio, executando
**funções puras REAIS** de `packages/scene-engine` (origem, somente leitura) sobre um corpus
sintético de bundles `render-bundle.json` (schema 1.0). O que o editor Rust vier a fazer com esses
bundles deve coincidir com o que está em [`docs/migration/fixtures/expected/`](../../docs/migration/fixtures/expected/).

Contrato e limites: [`docs/features/migration-oracle.md`](../../docs/features/migration-oracle.md).
Relatório da frente: [`docs/migration/foundation-b-report.md`](../../docs/migration/foundation-b-report.md).

## O que é e o que NÃO é

- **É:** veredito de validação (schema + integridade + junções de edição), resolução de referências
  e hierarquia do grafo, matrizes mundo, avaliação do frame de edição (shot → instância/câmera →
  estado dos nós, performance) e pose de câmera nos frames de borda, **todos produzidos pelo código
  da origem**, empacotado sem alteração.
- **Não é:** a engine completa, o runtime Three.js, render, áudio, GPU, nem uma especificação.
  Nenhuma regra esperada foi reimplementada aqui (nem se deve duplicá-la em Rust a partir deste
  diretório: o Rust implementa a regra, o oráculo só diz qual resultado a origem dá).
- A lista exata do que roda, do que só é carregado e do que **não** roda está em
  [`harness/scope.json`](harness/scope.json) (copiada para o manifest).

## Layout

```text
scripts/migration-oracle/
  oracle.mjs              CLI (setup | corpus | generate | verify | status)
  oracle.spec.mjs         testes do runner (node:test, 21 casos)
  harness/entry.mjs       chama a origem; único arquivo "lógico" do harness
  harness/scope.json      o que é executado / carregado / não executado / lacunas
  corpus/builder.mjs      entradas sintéticas (bases + mutações) com a INTENÇÃO de cada caso
  lib/                    config, build do harness (esbuild), geração/verificação, probes, serialize
  package.json, pnpm-lock.yaml   dependências fixadas (zod 4.3.6, three 0.187.0, esbuild 0.25.12)
docs/migration/fixtures/
  cases.json              índice: id, tipo, descrição, intenção, caminhos
  bundles/<id>.bundle.json        entradas (bundle puro, pronto para um parser Rust)
  expected/<id>.expected.json     saída das funções reais da origem
  manifest.json           proveniência: commit, hashes das fontes executadas, versões, regeneração
```

## Pré-requisitos e instalação isolada

Node ≥ 22.12 e `pnpm` (nunca `npm`). As dependências ficam **fora** do repositório e da origem:

```sh
node scripts/migration-oracle/oracle.mjs setup
# instala em ~/.cache/tessel-migration-oracle/<hash de package.json+lock>/ com
# `pnpm install --frozen-lockfile --ignore-scripts`; nada é instalado na origem.
```

As três integridades (zod, three, esbuild) são idênticas às do `pnpm-lock.yaml` da origem no commit
fixado. A origem padrão é
`/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine`
(sobrescreva com `--source-root`/`ORACLE_SOURCE_ROOT`). Nada é copiado, escrito ou instalado nela;
o runner só lê arquivos e roda `git rev-parse|ls-tree|cat-file`.

## Comandos

```sh
node scripts/migration-oracle/oracle.mjs verify              # uso normal / gate
node scripts/migration-oracle/oracle.mjs verify --fixtures-only   # só integridade (NÃO compara com a origem)
node scripts/migration-oracle/oracle.mjs generate            # regenera expected/ e manifest.json
node scripts/migration-oracle/oracle.mjs corpus              # reescreve bundles/ e cases.json do builder
node --test scripts/migration-oracle/oracle.spec.mjs         # testes do runner
```

Códigos de saída: `0` ok · `1` corrupção, deriva da origem ou divergência · `2` ambiente/uso
(origem ou dependências ausentes, argumento inválido). Origem ausente **nunca** vira sucesso.

### O que `verify` acusa

| Categoria | Significado |
|---|---|
| `FIXTURE_HASH_MISMATCH` / `FIXTURE_MISSING` / `FIXTURE_UNLISTED` | bundle, expectativa ou `cases.json` editado, apagado ou sobrando |
| `HARNESS_DRIFT` | `entry.mjs`, `scope.json`, `probes.mjs`, `serialize.mjs`, `package.json` ou lockfile mudaram |
| `TOOLCHAIN_DRIFT` | zod/three/esbuild instalados diferem do manifest |
| `SOURCE_DRIFT` | algum arquivo da origem empacotado tem sha256 diferente do fixado (ou entrou/saiu do grafo) |
| `EXPECTATION_DIVERGENCE` | regenerar com a origem atual não reproduz a expectativa versionada byte a byte (mostra o 1º caminho divergente) |
| `MANIFEST_MISSING` / `MANIFEST_VERSION` | proveniência ausente ou de outra versão |

Um aviso (`WARN`, sem falhar) é emitido se a versão do Node difere da usada na geração.

## Como a origem é executada

`harness/entry.mjs` importa `@origin/<módulo>`; o esbuild resolve para `<source-root>/src/<módulo>`
e empacota **os arquivos originais sem alteração** (33 arquivos `.ts`) com `zod` e `three` das versões
fixadas. Sem DOM, sem WebGL, sem rede. As funções chamadas são:
`renderBundleSchema.safeParse`, `validateBundleIntegrity`, `validateEditJoins`, `finalizeBundle`,
`validateSceneGraphStructure`, `resolveParentLinks`, `computeWorldMatrices`, `evaluateEditFrame`,
`resolveCameraAtClipFrame`, `mainTrackPoses`, `getCameraClipDurationFrames` (e, por elas,
`resolveShotAtEditFrame`, `resolvePerformanceOverrides`, easing e keyframes).
**Nenhuma função foi extraída ou copiada para um harness** — por isso não há procedimento de
extração a registrar; a prova de que o código é o da origem é o conjunto de hashes do manifest.

## Proveniência e regeneração

`manifest.json` registra: commit `d6adac15c61f4ef582a7ec259397b2e025d1f059`
(`feat/image-coverage-r8`), árvore `packages/scene-engine` no commit
(`d5fc98713731c6c0ae201ca1f3ce3bd0c9b17214`), e para cada fonte empacotada o `sha256`, o blob git
(cruzado com `git ls-tree` do commit **na geração**; `generate` recusa se algum arquivo difere ou se
o commit não pode ser provado), tamanho, bytes no bundle e se foi chamada direto ou carregada
indiretamente; versões de Node/zod/three/esbuild; hashes do harness; hashes de cada fixture.

Regenerar (idempotente: rodar duas vezes produz bytes idênticos):

```sh
node scripts/migration-oracle/oracle.mjs setup
node scripts/migration-oracle/oracle.mjs generate
node scripts/migration-oracle/oracle.mjs verify
```

Para trocar o snapshot (ex.: nova decisão de baseline/piloto): atualizar `PINNED_COMMIT`/
`PINNED_BRANCH` em `lib/config.mjs`, rodar `generate`, **revisar o diff de `expected/`** (cada
mudança é uma diferença de comportamento da origem a explicar) e só então versionar. Mudar o corpus:
editar `corpus/builder.mjs`, `corpus` e `generate`. Se `generate` falhar com `INTENT <caso>`, o caso
não exercita o código que declara (a origem não o produziu): corrija o caso, não a expectativa.

## Corpus

55 casos: 4 válidos e 51 inválidos (`cases.json`). Os bundles são escritos do zero a partir do
**schema** da origem (não são cópia de fixture nem de produção). Os assets são só referências: não
existe PNG/áudio e os hashes são strings sintéticas (`synthetic-sha256-*`). Cada inválido muda uma
coisa de uma base válida, e o nome do caso diz qual. `intent` (em `cases.json`) é só a intenção,
conferida na geração.

## Formato de `expected/<id>.expected.json`

```text
formatVersion, case, kind, bundle, bundleSha256
validation:
  schema            { success, issues[ { path, zodCode, sourceCode|null, message } ] }
  sceneGraphStructure[ { definitionId, issues[ {code,message} ] } ]   (JSON cru, mesmo se o schema recusa)
  integrityOnParsed [ {code,message,path?} ] | null                    (só se o schema aceitou)
  editJoins         [ {code,message,shotId?} ] | null
  finalize          { ok } | { ok:false, message } | null              (= veredito de `scene validate`, sem arquivos)
  accepted          bool  (= finalize.ok)
probes              { editFrames[], clipFrames{clipId:[...]} } | null     frames de borda escolhidos pelo runner
evaluation          (só se aceito)
  parentLinks       { definitionId: { nodeId: root|parent|socket } }
  editFrames[]      { editFrame, ok:true, state:ResolvedFrameState, worldMatrices{nodeId:[16]} } | { editFrame, ok:false, error }
  cameraClips[]     { clipId, durationFrames, mainTrackPoses[], frames[ {clipFrame, ok, camera|error} ] }
```

`sourceCode` é o prefixo `CODIGO:` que a origem embute na mensagem do refinamento (a mensagem
completa segue intacta); é `null` em erros estruturais do zod e em mensagens sem código. Os frames de
borda (`probes`) são gravados no arquivo: o teste Rust usa a lista gravada.

Convenções: números são os doubles exatos do JS (`JSON.stringify`); `-0` aparece como `0`; valor não
finito apareceria como `{ "$nonFinite": "NaN" }`. O consumidor Rust já adota `abs <= 1e-10 + 1e-12 * abs(expected)` para matrizes estáticas
([directed_scene_oracle.rs](../../crates/core/tests/directed_scene_oracle.rs)) e
`abs <= 1e-9 + 1e-12 * abs(expected)` para câmera e animação
(`directed_scene_temporal.rs`, `directed_scene_animation.rs`). IDs, frames e códigos são exatos.
As constantes `JOIN_*` da origem são regras de domínio distintas dessas tolerâncias de comparação.
Textos de erro estrutural do zod (`zodCode`/`message` sem `sourceCode`) são específicos do zod 4.3.6:
para eles, compare veredito e `path`, não o texto.

## Consumo em testes Rust futuros

Entregas concretas, estáveis em caminho:

- `docs/migration/fixtures/bundles/*.bundle.json` — entradas para o parser/validador Rust (BB01).
- `docs/migration/fixtures/expected/*.expected.json` — veredito e códigos (BB01), `parentLinks` e
  `worldMatrices` (BB02), `editFrames[].state` e `cameraClips[]` (BB03/E1).
- `docs/migration/fixtures/cases.json` — índice para iterar casos (`kind`, `bundle`, `expected`).

Esboço (caminho a partir do crate de teste; sem depender de nenhuma API existente):

```rust
let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/migration/fixtures");
let index: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("cases.json"))?)?;
for case in index["cases"].as_array().unwrap() {
    let bundle = std::fs::read_to_string(dir.join(case["bundle"].as_str().unwrap()))?;
    let expected: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(case["expected"].as_str().unwrap()))?)?;
    // comparar o veredito do Rust com expected["validation"]["accepted"] e os códigos com
    // issues[].sourceCode / editJoins[].code; pose e matrizes com expected["evaluation"].
}
```

## Limites

Ver `harness/scope.json` (`loadedButNotExercised`, `notExecuted`, `corpusGaps`) e o relatório.
Em resumo: sem narração, outline, clima, luz por keys, efeitos, rig/path, splat, FPS não inteiro,
preflight de assets, I/O de arquivo, runtime/pixel/áudio e nada do branch piloto.

## Corpus estendido (K / BL018, rodada 2)

Segundo corpus, **isolado** do de 55 casos: fixtures em `docs/migration/fixtures/extended-round2/`,
CLI `extended.mjs`, código em `extended/`. Mesma origem (`d6adac15…`), mesmos `zod 4.3.6` / `three
0.187.0`, mesma regra "executar o módulo real empacotado, nunca reimplementar". O corpus base, o
manifest dele e o `harness/` não são tocados (`extended.mjs` **recusa** gerar em `docs/migration/fixtures/`
e `oracle-extended.spec.mjs` prova com `git status` que o base não mudou). Única mudança no código
compartilhado: `lib/harness.mjs` aceita outro ponto de entrada e passa a registrar como fonte direta os
imports de qualquer arquivo do oráculo, e o `verify` do base continua byte a byte igual.

```bash
node scripts/migration-oracle/extended.mjs setup      # mesmas dependências fixas, fora do repo
node scripts/migration-oracle/extended.mjs corpus     # reescreve bundles/, cases.json, tolerances.json (não usa a origem)
node scripts/migration-oracle/extended.mjs generate   # executa a origem e grava expected/ + manifest.json
node scripts/migration-oracle/extended.mjs verify     # hashes, deriva da origem, tolerâncias, intenção, regeneração byte a byte
node scripts/migration-oracle/extended.mjs verify --fixtures-only   # só integridade; NÃO compara com a origem
node --test scripts/migration-oracle/oracle-extended.spec.mjs       # 24 testes, incl. controles negativos
```

### O que cobre (47 casos)

| Família | Casos | Pergunta |
|---|---|---|
| `join-boundary` (31) | posição, alvo, FOV, velocidade, velocidade de FOV, diagonal, segunda junção, shots fora de ordem, FPS 24000/1001 e 30000/1001: abaixo / igual / acima | O gate aceita/recusa exatamente onde a origem (`>` estrito) recusa? As poses nos dois lados são as da origem? |
| `fps` (11) | 24000/1001, 30000/1001, 60000/1001, 25, 30, 60, 48/2 não reduzido, 2000/1, e 3 recusas (perfil divergente, denominador 0) | `fpsToRate`, `editFrameToSeconds`, `secondsToEditFrame`, `editSecondsToFrame` |
| `out-of-order` (1) | 7 ordens: crescente, decrescente, embaralhada (semente fixa), repetida, nas bordas | Mesma resposta em qualquer ordem (origem e Rust) |
| `world-matrix` (4) | gimbal (`|m13| ≥ 0.9999999`), escalas extremas e cadeia profunda, sockets encadeados com canais, poses `lookAt` da câmera | Matrizes mundo e matriz da câmera |

### Formato de `expected/<id>.expected.json`

`{ formatVersion, case, family, kind, bundle, bundleSha256, validation, probes, evaluation, joins,
cameraWorld, orders, time }`:

- `validation`: o mesmo bloco do corpus base (`schema`, `sceneGraphStructure`, `integrityOnParsed`,
  `editJoins`, `finalize`, `accepted`).
- `joins[]`: por junção contínua no mesmo clipe, a pose da câmera (`resolveCameraAtClipFrame`) no último
  frame do shot anterior (`end`) e no primeiro do seguinte (`start`), com `clipFrame`.
- `cameraWorld[]`: `{ editFrame, ok, source: "three-library", matrix[16] }`.
- `orders`: `identicalToCanonical` e, por ordem, `resultsSha256` dos resultados.
- `time`: `rate`, durações, `frames[]` (segundos de cada frame) e `seconds[]` — sondas de segundos→frame
  com o instante **exato** (`exact.num/den`, strings), o `floorFrame` da origem (ponto flutuante), o
  `exactFloorFrame` (racional exato), `originMatchesExact` e o recorte (`clampedFrame`).

### Tolerâncias (fixadas antes de comparar; `tolerances.json`)

As mesmas do corpus base, nenhuma afrouxada: matrizes de nós `1e-10 + 1e-12·|e|`; local/câmera/matriz
da câmera `1e-9 + 1e-12·|e|`; segundos `1e-12 + 1e-12·|e|`; ids, frames, códigos exatos. Os `JOIN_*` são
regras de domínio da origem (comparação estrita `>`), conferidos contra o módulo real a cada
`generate`/`verify`. O consumidor Rust repete as constantes e confere `tolerances.json`.

### Limites declarados

- **Fronteiras decididas pela origem** (`kind: "boundary"`, 6 casos): velocidade 0,08, velocidade de FOV
  0,5, diagonal e variações de 1 ulp não são exatas em binário; o oráculo grava o que a origem responde
  (`summary.originDecidedBoundaries`) e não afirma um valor a priori.
- **Artefatos de ponto flutuante** (14 sondas): `secondsToEditFrame` da origem é `Math.floor(s·rate)`
  em `double` e erra um frame em alguns limites NTSC exatos. A fixture marca `originMatchesExact=false`;
  o Rust (racionais exatos) deve igualar o piso exato ali e a origem em todo o resto.
- **Dependem de método não exportado** (`scope.json`): a matriz mundo da câmera (método privado do
  runtime, precisa de WebGL) é gravada com `PerspectiveCamera.lookAt` do `three` fixo sobre a pose que a
  origem avaliou (`source: "three-library"`); `compareContinuousPose` e as mensagens `toFixed(4)` não
  são exportadas (o oráculo grava códigos e poses; as distâncias saem das poses).
- Sem GPU, áudio, pixel, narração; tempo racional amostrado em 9 frames por FPS, não em todos.
