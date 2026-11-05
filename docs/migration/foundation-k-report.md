# Relatório da frente K — corpus estendido do oráculo (BL018 / BB10 / F032 existente)

2026-11-05. Rodada H/I/J/K ([plano](next-round-plan.md)). Checkout compartilhado `tessel-editor`.
Origem (`/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage`, snapshot
`d6adac15c61f4ef582a7ec259397b2e025d1f059`) e `tesselapp` não foram tocados. Contrato:
[`docs/features/migration-oracle.md`](../features/migration-oracle.md) (seção "Corpus estendido").

> Estado original da entrega, antes da integração: o coordenador posteriormente executou
> Cargo/clippy e os 15 testes Rust passaram (ver "Integração pelo coordenador").

**Na entrega, o código Rust desta frente não tinha sido compilado nem executado.** Não rodou Cargo, rustc nem clippy
(só `rustfmt` no próprio arquivo de teste, que confirma que a sintaxe parseia). **Rust não é declarado
verde.** Não fez `git add`/commit/push/PR, não criou branch/worktree, não tocou `core/src/lib.rs`,
`media/src/lib.rs`, `Cargo.toml/lock`, `docs/backlog.md`, matriz nem índices. Nenhum subagente, nenhum
GPU/Chrome/janela. Não instalou dependências na origem; reutilizou o cache externo
(`~/.cache/tessel-migration-oracle`, 40 KB de links; fixtures novas: 2,6 MB).

## Nenhum patch obrigatório de integração

O teste novo usa só API pública que já existe (`directed_scene::*`, `time::{Rate, Ratio}`): **não há
export novo em `core/src/lib.rs` nem em `media/src/lib.rs`**, nem módulo, nem campo, nem dependência.
O coordenador só precisa compilar e rodar:

```bash
cargo test -p tessel-editor-core --test directed_scene_oracle_extended -- --nocapture
```

(o nome do pacote segue o `Cargo.toml` do crate; `--nocapture` imprime, por família, o pior erro
numérico e a distância medida das fronteiras de junção).

## Arquivos

| Arquivo | Conteúdo |
|---|---|
| `scripts/migration-oracle/lib/harness.mjs` (**única edição em código compartilhado**) | `buildHarness(config, entry = ENTRY)` aceita outro ponto de entrada; `directImports` passa a ser a união dos imports de **todo** arquivo do oráculo. Prova de não regressão: `oracle.mjs verify` (55 casos) passa e regenerar o base num diretório temporário dá bytes idênticos |
| `scripts/migration-oracle/extended.mjs` | CLI `setup / corpus / generate / verify [--fixtures-only] / status`; recusa o diretório do corpus base |
| `scripts/migration-oracle/extended/entry.mjs` | exports do harness: `originJoinConstants`, `joinSamples`, `cameraWorldMatrices`, `evaluateInOrder`, `evaluateTime` + os do base |
| `.../extended/corpus.mjs` | `buildExtendedCorpus()` — 47 casos (válidos, inválidos, `boundary`) com `intent` e `probes` |
| `.../extended/core.mjs` | `generate`/`verify`: execução, intenção, ordem, tolerâncias, manifest, regeneração |
| `.../extended/config.mjs`, `numeric.mjs` | configuração isolada; aritmética exata (`nextUp/Down`, racional exato de um `double`, piso exato, quaternion de Euler) |
| `.../extended/tolerances.json`, `scope.json` | tolerâncias fixadas **antes** de comparar; escopo, métodos não exportados e lacunas |
| `scripts/migration-oracle/oracle-extended.spec.mjs` | 28 testes Node (`node:test`) com controles negativos |
| `docs/migration/fixtures/extended-round2/` | `bundles/` (47), `expected/` (47), `cases.json`, `tolerances.json`, `manifest.json` (**novo; o corpus de 55 casos e o manifest dele não foram escritos**) |
| `crates/core/tests/directed_scene_oracle_extended.rs` | consumidor Rust (API pública), 15 testes (9 comparações + 6 controles negativos) |
| `scripts/migration-oracle/README.md`, `docs/features/migration-oracle.md` | seção "Corpus estendido" |
| `docs/migration/foundation-k-report.md` | este relatório |

## Implementação

- **Execução real, sem reimplementar.** O gerador empacota (esbuild) os módulos reais da origem, lidos
  somente leitura, com `zod 4.3.6` e `three 0.187.0` fixos, e roda em Node. O manifest guarda sha256 e
  blob git de cada fonte executada (34 arquivos, conferidos com `git ls-tree` do commit fixado), hash
  de cada arquivo do gerador e das fixtures, versões de ferramentas e `summary`/`scope`. `verify`
  regenera tudo e compara byte a byte.
- **Fronteiras de junção** (31 casos): para cada limite `JOIN_*` há abaixo / igual / acima segundo a
  comparação real (`>` estrito). Dolly de 4 unidades em 17 frames dá passo **exato** de 0,25 (igual ao
  limite, não é salto); `0.24375` e `0.25625` ficam dos lados. Cobre posição+alvo, só posição, alvo, FOV,
  velocidade, velocidade de FOV, diagonal, 2ª junção isolada, shots listados ao contrário e FPS NTSC.
- **FPS** (11): 24000/1001, 30000/1001, 60000/1001, 25, 30, 60, 48/2 não reduzido, 2000/1 e 3
  recusas. 459 sondas de tempo, incluindo o instante exato de cada frame (num/den como string) e
  ±1 ulp ao redor, comparadas ao piso exato.
- **Fora de ordem**: 7 ordens (crescente, decrescente, embaralhadas com semente fixa, repetidas,
  bordas); o harness exige que as 7 deem o resultado do caso canônico e grava o hash por ordem.
- **Matrizes**: gimbal (`|m13| ≥ 0.9999999`, onde o `z` é zerado), escalas extremas, cadeia profunda,
  sockets encadeados com canais `cycle`/`event`/`static`, e 380 matrizes mundo da câmera.
- **Controle de intenção**: cada caso declara o que deveria produzir (aceito/recusado, códigos de junção,
  código de origem). Se a origem responde outra coisa, `generate` **falha** (corpus mal desenhado), não
  grava.
- **Rust**: o teste repete as tolerâncias em constantes e falha se `tolerances.json` ou os `JOIN_*`
  divergirem. Cada comparação empurra uma divergência legível (rótulo, valor do Rust, valor da origem,
  distância, tolerância) para um `Report`; o teste mostra **todas** (até 40), não só a primeira.

### Tolerâncias (fixadas antes das comparações)

| Grandeza | Regra |
|---|---|
| ids, frames, contagens, `ok`, aceito, códigos de junção | exato |
| `worldMatrix` de nós | `abs ≤ 1e-10 + 1e-12·abs(esperado)` |
| local de nós (posição, Euler em graus, escala), canais | `abs ≤ 1e-9 + 1e-12·abs(esperado)` |
| câmera (posição, alvo, fov, velocidades) | `abs ≤ 1e-9 + 1e-12·abs(esperado)` |
| matriz mundo da câmera | `abs ≤ 1e-9 + 1e-12·abs(esperado)` |
| segundos | `abs ≤ 1e-12 + 1e-12·abs(esperado)` |

São as mesmas do corpus base (BB02/BB03); nenhuma foi afrouxada para o corpus novo. Os `JOIN_*` não
são tolerâncias de comparação, são regras de domínio da origem.

## Testes: escritos × executados

| Teste | Escrito | Executado |
|---|---|---|
| `oracle-extended.spec.mjs` (Node, 28 testes) | sim | **sim — 28/28** |
| `oracle.spec.mjs` (Node, base, 21 testes), regressão | já existia | **sim — 21/21** |
| `node scripts/migration-oracle/oracle.mjs verify` (55 casos) | — | **sim — ok, byte a byte** |
| `node scripts/migration-oracle/extended.mjs verify` (47 casos) | — | **sim — ok, contra a origem** |
| `crates/core/tests/directed_scene_oracle_extended.rs` (Rust, 15 testes) | sim | **sim — 15/15 pelo coordenador** (`cargo test --locked`) |

Observação sobre o comando: no Node 22 usado aqui, `node --test scripts/migration-oracle/` (diretório)
não carrega; passe os arquivos: `node --test scripts/migration-oracle/oracle.spec.mjs
scripts/migration-oracle/oracle-extended.spec.mjs` (49/49).

**Controles negativos executados (Node)** — cada um precisa falhar com exit `1` e a categoria certa:
número adulterado (hash **e** divergência); tempo adulterado com manifest "consertado" (só a
regeneração pega; `--fixtures-only` passa e se declara limitado); veredito de junção invertido;
`originMatchesExact` trocado; hash de uma ordem adulterado; matriz da câmera adulterada; bundle
alterado; `tolerances.json` com deriva; arquivo ausente / não listado; manifest ausente; limite de
junção `0.25 → 0.26` numa **cópia** da origem (`SOURCE_DRIFT` + `TOLERANCES_JOIN_CONSTANT` +
`EXPECTATION_DIVERGENCE` em `join-position-above`); mudança só de comentário (só hash, sem
divergência); origem ausente (exit 2); `generate` recusando origem não provável; `generate` com
`intent.accepted` trocado (falha e **nada** é regravado); `generate`/`verify`/`corpus` recusando o
diretório do corpus base; regeneração idempotente; corpus base inalterado segundo o `git status`.

**Controles negativos Rust executados pelo coordenador**: tolerância aplicada exatamente (dentro
passa, fora falha, NaN falha); veredito invertido com as distâncias na mensagem; bundle com
comportamento de câmera alterado (dolly 3,9 → 4,1 e 4,1 → 3,9) muda o veredito; matriz mundo de nó
perturbada por `1e-6` falha e por `1e-13` passa, e o quadro 16 não passa por quadro 8; matriz da câmera
transposta falha; taxa 30/1 contra a expectativa de 30000/1001 falha; um "artefato" com piso exato
trocado ainda falha.

## O que o teste Rust compara (e o que difere de propósito)

| Teste | Compara |
|---|---|
| `declared_tolerances_are_the_ones_fixed_before_the_comparison` | constantes do teste == `tolerances.json`; `JOIN_*` do Rust == os exportados pela origem |
| `corpus_has_the_declared_shape` | 47 casos, 4 famílias, `case`/`formatVersion` |
| `verdicts_and_join_findings_match_the_origin_on_every_case` | `import_value` + `edit_join_issues` + `temporal_issues`: veredito e `(código, shot)` por junção; código de origem quando a recusa é só por regra; as 6 fronteiras `boundary` decididas pela origem; fps > 1000 (divergência documentada) |
| `camera_poses_on_both_sides_of_every_continuous_join_match_the_origin` | `camera_at_clip_frame` nos dois frames de medida das 39 junções |
| `edit_frames_and_world_matrices_match_the_origin_on_every_probed_frame` | 468 quadros: ids, câmera, nós (local + `worldMatrix`), `performanceOverrides`, e o texto do erro de quadro fora do intervalo |
| `seeking_in_any_order_gives_the_origin_answer_and_the_same_answer_every_time` | 7 ordens, import novo por ordem; resposta == origem **e** == a primeira resposta do próprio Rust |
| `camera_to_world_matrix_matches_the_three_look_at_of_the_origin_poses` | `to_world_matrix` da pose da origem (isola o método) e da pose que o Rust avalia (ponta a ponta), 380 matrizes |
| `rational_frame_rates_convert_time_like_the_origin` | `Rate`, `frame_to_time`, `time_to_frame_floor`, `Bundle::frame_at_time`, recorte; 14 artefatos fixados |
| `ntsc_frames_start_on_the_exact_rational_second` | 30000/1001: o quadro 30000 começa em exatamente 1001 s; 1000,999999 s ainda é o quadro 29999 |

Divergências **esperadas** (afirmadas, nunca puladas em silêncio):

1. `fps-above-1000`: a origem aceita qualquer taxa racional positiva; o importador Rust recusa com
   `FPS_OUT_OF_RANGE` (teto de 1000 fps documentado no próprio `Rate`).
2. Segundos→frame em limites NTSC exatos: a origem usa `Math.floor(segundos·taxa)` em `double`; em
   **14 sondas** (24000/1001: 3; 30000/1001: 4; 60000/1001: 6; 48/2 não reduzido: 1) isso fica um
   quadro abaixo do piso exato. O Rust usa racionais e deve igualar o **piso exato** nessas, e a origem
   em todas as outras. O número 14 está fixado no teste e no manifest: se mudar, é para re-triar.

## Limites

- **Fronteiras decididas pela origem** (6): `join-velocity-equal`, `join-fov-velocity-equal`,
  `join-diagonal-equal`, `join-position-ulp-below/above`, `join-fov-ulp-above`. Velocidade 0,08 e
  velocidade de FOV 0,5 não são exatas em binário, então o oráculo grava a resposta da origem (aceita ou
  recusa) em vez de afirmar um valor.
- **Dependem de métodos não exportados** (`scope.json`): (a) a matriz mundo da câmera é calculada no
  método **privado** `ThreeSceneRuntime.drawEditFrame`, que exige WebGL; a fixture usa
  `PerspectiveCamera.lookAt` do `three` fixo sobre a pose que a origem avaliou (`source:
  "three-library"`): prova a convenção da biblioteca, não o método da origem, e não percebe se o runtime
  mudar a sequência; (b) `compareContinuousPose`/`cameraStateAtJoinEnd/Start` não são exportadas: o
  oráculo grava códigos e as poses reais dos dois lados, e as distâncias saem das poses; (c) mensagens
  `toFixed(4)` são diagnóstico: gravadas, não comparadas.
- **Lacunas do corpus** (`scope.json`): sem caso "exatamente igual" para easings não lineares; seek
  aleatório cobre a **ordem** de um caso, não é fuzz de bundles; tempo racional amostrado em 9 frames por
  FPS (até o frame 100000), não em todos; sem áudio, narração, pixel, GPU, performance nem branch piloto.
- Cobertura por linha **não foi medida**; "executado" = chamado pelo harness ou exercitado por essas
  chamadas.
- Na entrega inicial, o teste Rust foi escrito a partir das assinaturas. O coordenador depois
  compilou e executou os 15 testes e o clippy; não há primeira execução pendente. Os limites de
  contagem (47 casos, 14 artefatos e fronteiras numéricas) continuam explícitos no consumidor.
- Nenhum bug de produção do avaliador foi encontrado nesta frente. A revisão do PR #2 encontrou
  um bypass por aliases no gerador de corpus; ele foi corrigido e recebeu quatro regressões,
  sempre em cópias temporárias. Status e atribuições continuam no backlog comum.

## Propostas históricas (já consolidadas no backlog)

Status e atribuições atuais: [docs/backlog.md](../backlog.md). As linhas abaixo registram
a proposta original, não constituem uma fila de execução independente.

### `docs/backlog.md`

BL018: **Em andamento → A validar** (depois que o coordenador compilar e rodar o teste Rust), com a
evidência: *"Corpus estendido de 47 casos gerado e verificado contra a origem `d6adac15…` (Node:
`extended.mjs verify`, 28/28 + 21/21); consumidor Rust
`crates/core/tests/directed_scene_oracle_extended.rs` escrito, a rodar. Divergências documentadas: fps >
1000 e 14 artefatos de ponto flutuante da origem."*

Novas linhas (numeração sugerida a partir de BL070; o coordenador renumera):

| ID | Bloco | Pendência | Status | Depende de | Dono | Critério de pronto |
|---|---|---|---|---|---|---|
| BL070 | BB10/tempo | Decidir se segundos→frame do destino segue o racional exato (hoje) ou reproduz o `Math.floor` em ponto flutuante da origem nos 14 limites NTSC exatos do corpus | Pendente | BL018 | — | Decisão registrada; o teste `rational_frame_rates_convert_time_like_the_origin` passa a afirmar um só comportamento e o número `EXPECTED_ORIGIN_FLOAT_ARTIFACTS` é removido ou zerado |
| BL071 | BB10/câmera | Matriz mundo da câmera vinda do `ThreeSceneRuntime.drawEditFrame` (método privado, WebGL) em vez de `PerspectiveCamera.lookAt` isolado | Pendente | BL018, runtime em Chrome | — | Matriz medida no runtime da origem em ambiente com GPU; fixtures `source: "runtime"` substituem `three-library` |
| BL072 | BB10/fps | Divergência documentada: origem aceita fps > 1000, `Rate::new` recusa (`FPS_OUT_OF_RANGE`) | Pendente | BL018 | — | Decidir manter o teto (e documentar na matriz) ou ampliá-lo; ajustar `DOCUMENTED_RUST_LIMITS` |
| BL073 | BB10/junção | Fronteiras de velocidade (0,08), velocidade de FOV (0,5), diagonal e ±1 ulp são decididas pela origem; ampliar com easings não lineares e frames de medida ajustados | Pendente | BL018 | — | Casos "igual" por tentativa de distância com resposta registrada; `summary.originDecidedBoundaries` revisado |

### Matriz (F032 existente, sem reservar F040)

> F032 — Oráculo de equivalência (origem executada). **Origem** Sim (47 casos novos + 55, funções reais
> de `d6adac15…`). **Lib** Parcial → A validar: Rust compara fronteiras de junção (`JOIN_*` estrito), FPS
> NTSC e racional (`Rate`/`Ratio`), avaliação fora de ordem, matrizes mundo de nós e da câmera.
> **Limites**: matriz da câmera pela biblioteca `three` (método da origem é privado), 14 artefatos de
> ponto flutuante da origem e fps > 1000 declarados como divergências; 6 fronteiras decididas pela
> origem. **Evidência**: `extended.mjs verify`, `oracle-extended.spec.mjs` (28/28); Rust escrito, não
> executado.

## Disco e orçamento

Fixtures novas: 2,6 MB (96 arquivos). Artefatos temporários dos testes Node ficam em `os.tmpdir()` e
são apagados ao final (`after`). Nada de `target/`, `mbx` ou `HOME`/`CARGO_HOME`/`RUSTUP_HOME` foi
tocado; muito abaixo do teto de 2 GiB de artefatos.

## Integração pelo coordenador

45 testes Node passaram; verify de 55+47 casos passou contra origem. Consumidor Rust estendido compilou e passou 15/15 com Cargo --locked. BL018 fica parcial pelos limites explícitos; BL070–BL073 registrados sem alterar expectativas. Matriz F032 permanece parcial, sem alegar render/pixel/áudio.
