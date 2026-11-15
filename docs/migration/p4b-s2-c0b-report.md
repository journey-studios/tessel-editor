# P4b S2-C0b — relógios do plano com transição, teto exato e consumidor X de estado

Entrega de BL082.4.4 (parte core), .17, .20, .21 e .23 sobre a base `c9c5fa0`
(`main`), branch `feat/p4b-s2-c0b-plan-clocks`. Plano e contrato final:
[p4b-s2-c0b-plan.md](p4b-s2-c0b-plan.md). **Estado: implementado, a validar** — falta a revisão Windows
independente e o Mac do coordenador. Nada de runtime, renderer, host loader, mídia, UI, export, `Domain` ou capability JS
novo; `Refuse` continua fechado nas entradas públicas e F064 segue produto **P**. Sem GPU, sem lock de desempenho,
sem origem executada.

## 1. O que muda

- **Modelo (A).** `PlanTransition` ganha `alignment` e `timing: TransitionTiming` (`editDurationFrames` e, por shot,
  `shotStartFrame`, `shotEndFrameExclusive`, `instanceStartFrame`, `instanceDurationFrames`, `cameraInFrame`,
  `cameraClipDurationFrames`, `sampledInstanceFrame`, `sampledCameraClipFrame`). Obrigatórios no wire de um plano
  **com** transição; um plano do wire anterior é recusado em `/transition/alignment`. `PLAN_VERSION` continua 1 e
  planos sem transição mantêm os bytes (baseline M0 exato em debug, release e preserve_order).
- **Regra única de relógio.** `directed_scene::{ClockSample, ClockSource, DeckClocks}` (em `shot_offset.rs`) e
  `frame_start_time` (em `frame.rs`): `bruto = início + (f − inícioDoShot)`, `amostrado = clamp(bruto, 0, L−1)`,
  `preso = L == 0 ∨ amostrado ≠ bruto`. Alimenta o avaliador de deslocamento, o montador
  (`scene_plan/temporal/assemble.rs`, usado por `build.rs` e `timeline.rs`) e o validador
  (`scene_plan/temporal/validate.rs`). `PlanTransition::clocks(side)` devolve bruto/amostrado/preso dos **dois** decks.
  O comprimento da câmera é o da maior faixa (principal ou camada). O primário guarda o quadro bruto; o vizinho, o amostrado.
- **Validação.** Mesma porta para `validate()`, `from_json(Value)` e `from_json_str`, e para os dois construtores; ordem
  e ponteiros em [p4b-s2-c0b-plan.md](p4b-s2-c0b-plan.md) («Contrato final»). Aritmética de quadros e tempos verificada (overflow é erro). Mede coerência, não procedência.
- **Teto (B).** Plano externo **válido** de exatamente 20.000 itens e de exatamente 4.194.304 bytes; um item, uma luz ou
  um byte a mais é recusado em `/transition`. **Não** implica que um bundle o produza: `MAX_FX_TRACKS = 64` e as cotas do
  documento o limitam muito antes e **não** foram alterados.
- **Consumidor X (C).** `pilot_dissolve_representable_state.rs` lê o fixture representável do PR65 (7 cenários, 98
  quadros) e compara **estados** do plano real; nenhum pixel é desenhado ou comparado.
- Pastas novas/arquivos tocados: ver `git diff c9c5fa0 --stat`; as contagens do diff incluem também os documentos de fechamento. Nenhum
  arquivo acima de 800 linhas é novo e nenhum arquivo acima de 1000 linhas cresceu (`build.rs` 743 → 782 é o maior).
  Sem mudança em `Cargo.lock`, dependências, desktop, media, runtime, renderer ou na origem.

## 2. Contrato final (resumo; o normativo é o plano)

```
"alignment": "center|start|end",
"timing": { "editDurationFrames": N,
  "from": { 8 inteiros u64 }, "to": { 8 inteiros u64 } }
```

Inteiros sem float/bool/string/null/negativo; chaves extras recusadas no próprio ponteiro. `C` = `to.shotStartFrame`.
Ordem de validação: representação `i64` e intervalos → corte único → `primary` pelo lado de `C` → quadro dentro do shot
primário → janela `(C, D, alignment, edit)` e `startFrame` → ids compartilhados com um comprimento → instância do primário
cobre o shot → amostras → quadros legados (primário bruto, vizinho amostrado) → `held*` → `Ratio` exatos. Pointers:
`/transition/alignment`, `/transition/timing[/from|/to]/<campo>`, `/transition/primary`, `/transition/startFrame`,
`/frame/{instanceFrame,cameraClipFrame,editTime,instanceTime}`, `/transition/other/frame/…`, `/transition/held{Instance,Camera}`.

## 3. Dados (fixtures, baseline, goldens): antes/depois

80 arquivos hasheados antes do primeiro código e de novo no fim (SHA-256): **0 diferenças**. Entre eles:

| Arquivo | SHA-256 (antes = depois) |
|---|---|
| `docs/migration/fixtures/pilot-fx/dissolve-representable-cases.json` | `7de67565061aaa5ec5d87e1dfb60ea536eec52f4b8a89d6d9091e41500c6a276` |
| `docs/migration/fixtures/pilot-fx/dissolve-representable-manifest.json` | `cb80d2ef88803e6b14b4825474639a523680c1d434415e07e79886dde536cebe` |
| `docs/migration/fixtures/pilot-fx/dissolve-manifest.json` | `77c4d7bc5f7b95adc37d2ec1560adffb7772c3aa6ee805079ffdb9f0a392ce42` |
| `crates/core/tests/scene_plan_transition_baseline/no-transition-plans.json` | `2ee6ea7380ec09536ca095126de48884bf992cae536cf62cb686e75ad2695608` |
| `crates/core/tests/scene_plan_performance_golden/digests.json` | `89aacd146314cf74aeb79281297cf56ddb52ed88e10c07e4efa52d892717b9a3` |

Os 12 PNG representáveis (`dissolve-representable-assets/`), os manifestos fade/grain/fx e os demais da lista de 80 entradas
(sem Python rodado, sem golden/manifest/PNG regenerado, sem novo golden).

## 4. Testes novos

| Arquivo | Testes | Cobre |
|---|---|---|
| `scene_fx_transition/timing.rs` | 13 | timing literal nos quadros 44/50, `clocks()` dos dois decks, C−1/C, último quadro real sem hold, câmera `cameraIn` além do fim antes de C, **legado bruto 23 / amostrado 11**, comprimento = maior faixa (main/layers diferentes), primário com câmera curta, D1/D ímpar/start/end com quadros fora da janela = plano puro, wire e bytes, sem transição = mesmo byte e versão 1, wire anterior recusado, lâmpada com keys no quadro real, plano compartilhado, cold vs timeline em ordem crescente/reversa/repetida |
| `scene_fx_transition/timing_refusals.rs` | 4 | cada número do timing × 9 tipos errados (float, 2.0, negativo, bool, string, null, array, objeto, > u64), campos extras, `alignment` errado, 32 planos tipados incoerentes nas **três portas** com o ponteiro do campo (janela, `primary` cedo/tarde, corte, extremos, instância curta, ids compartilhados com comprimentos diferentes, amostras sem clamp, quadros brutos/legados, `held=false` adulterado, FPS/`Ratio`) |
| `scene_fx_transition/ceiling.rs` | 5 | 20.000 itens válidos (`[3,2,0,9993,2,0]` por deck, contados à mão), +1 passe em cada deck, caso assimétrico, luzes e vizinho no mesmo contador (`(1,0)` aceito, `(2,0)` e `(1,1)` recusados), contador tipado e de arrays, 4.194.304 bytes e +1 (inclusive chave com escape) |
| `pilot_dissolve_representable_state.rs` + `/reference.rs` | 5 | leitura inteira (7/98/390/84 em janela), 84 quadros em janela comparados em estado, 14 fora da janela = plano puro, avaliador de deslocamento nos 98, independência de ordem (`PlanTimeline` crescente/reverso/repetido) |
| `pilot_dissolve_representable_state/isolation.rs` | 3 | 11 classes de falsificação recusadas pela comparação (primário no corte, held da instância, held da câmera, instância marcada presa na borda, instância e câmera sem clamp, relógios trocados, relógio do primário no vizinho, fps nominal, lâmpada no relógio da instância e noutro quadro; cada classe exercida por ≥1 quadro), estado estático do hold e comprimento de câmera com layers |

`shot_offset.rs`: unitários da regra única (2 na base, 4 agora). Três testes de `plan.rs` foram atualizados porque o wire agora
tem 17 chaves e a regra de «cabeça presa no quadro 0» foi **substituída** pela de amostra: um vizinho com
`instanceFrame = 3` é recusado em `/transition/other/frame/instanceFrame`, não em `/transition/heldInstance`.

Mapeamento X → plano (`reference.rs`): `primary/neighbour`, `instance_wanted/camera_wanted` (bruto), `instance_frame/
camera_clip_frame` (amostrado, também em `timing.sampled*`), `held_*`, `shots.*` (offsets, comprimentos, limites),
`edit_seconds.rational`, `fps` (inclui 30000/1001), `window`, `cut`, `k`, `fx_edge`, `lights[].effective_intensity` (2e-9),
`marker.world_centre` (matriz mundial, 1e-9), câmera (posição ao longo do único segmento, 1e-9),
`transition.progress/amount/shaped` (a suavização é recalculada pela definição). **Só diagnóstico, nunca comparado como
estado:** `scene_seconds`, `probes[]` (390), `centre_px`, `shot_clip_frame`, `matrix`, `skipped_probes`, PNG.
O quadro-limite da câmera do vizinho (querido = primeiro/último real) **não aparece** nos dados de referência (só o da
instância, uma vez); a borda da câmera é provada em `timing.rs` com câmera sintética.

## 5. Gates por head

Código idêntico em `c2959ce` e `f0eb375` (este só muda docs); `c233307` é o primeiro commit de código. O relatório é o
último commit, que só altera Markdown. Execução em Windows, `./scripts/cargo` pelo Git Bash.

| Gate | Resultado (head de código `c2959ce`) | Compilado / executado |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | — |
| `check --workspace --all-targets --locked` | 0 | compilado |
| `clippy --workspace --all-targets --locked -D warnings` | 0, sem avisos | compilado |
| `test --workspace --locked --no-run` | 0 | **só compilado** (workspace inteiro não executado) |
| `build -p tessel-editor --locked` | 0 | compilado |
| `test -p tessel-editor-core --locked --no-fail-fast` | **1728 passam, 2 falham, 7 ignorados**, 66 resultados (lib 617) | executado |
| isolado (core, `EXPECT=0`) transições + corpus + X import + X estado + baseline | 0 falhas (contagens abaixo) | executado |
| `--features serde_json/preserve_order`, `EXPECT=1`, mesmo conjunto | 0 falhas | executado |
| workspace unificado, `EXPECT=1`, mesmo conjunto | 0 falhas | executado |
| baseline em `--release` | 1 passa | executado |
| desktop FX proporcional (`-p tessel-editor --bins fx`) | 80/80 | executado |

Conjunto isolado (por binário, nos três perfis): `pilot_dissolve_corpus` 10, `pilot_dissolve_representable_import` 5,
`pilot_dissolve_representable_state` 8, `scene_fx_transition` **214** (192 na base), `scene_plan_transition_baseline` 1.

**Falhas registradas, ambas da base Windows, não puladas nem relaxadas:**
`scene_gizmo_rotation::the_rotate_overlay_has_three_closed_rings_and_the_metrics_to_draw_them` e
`scene_gizmo_scale::four_parents_of_anisotropic_scale_with_a_rotation_between_them_refuse_the_world_shear`. Por isso o core
completo e o workspace global **não** são declarados verdes. Ignorados (7): harnesses de desempenho/medição
(`measure_attach_cost`, `paired_measurement…`, três de orçamento do gizmo, `report_memory…`, `report_plan_cost…`); nenhum
é desta entrega e nenhum desempenho é declarado (sem lock de GPU).
Primeira execução completa, no tree equivalente a `c233307`: core 1727/2/7, mesmos 2 vermelhos; os demais gates iguais.

## 6. Mutantes

Um por vez, restaurado por bytes e conferido por SHA-256, `mtime` tocado antes de cada execução e **controle limpo verde**
no fim de cada rodada. Conjunto: `--lib` + `scene_fx_transition` + `pilot_dissolve_*` + baseline (o `cargo` para no primeiro
binário vermelho, então «mortos por N» conta só esse binário). **29 mutantes; 0 sobreviventes finais, 0 equivalentes, 0 não
terminados.** Primeira passada: 26 mortos, **2 sobreviventes e 1 erro de compilação**, tratados com teste, não com ajuste:

| Mutante | Resultado |
|---|---|
| M01 `items >` → `>=` (validador tipado) | morto por 5 (`ceiling`) |
| M02 `items >` → `>=` (pré-checagem dos arrays) | morto por 6 (`limits`, `ceiling`) |
| M03 / M04 bytes `>` → `>=` (`precheck_text` / `check_text`) | mortos por 2 cada (`limits`, `ceiling`) |
| M05 vizinho fora do contador tipado | morto por 1 (`the_typed_plan_is_judged_by_the_same_count`) |
| **M06 luzes fora do contador tipado** | **sobreviveu** → teste tipado com luzes → morto por 1 |
| **M07 luzes fora do contador de arrays** | **erro de compilação** → refeito (chave `"resources"` duplicada) → morto por 6 |
| M08 vizinho fora do contador de arrays | morto por 2 (`limits`, `review`) |
| M09 sem teto superior no clamp | morto por 4 (unitários); sozinho no X estado, por 6 |
| M10 clipe vazio não preso em 0 | morto por 1 (unitário) |
| M11 / M12 último / primeiro quadro real marcado preso | mortos por 2 cada (unitários) |
| M13 comprimento da câmera só pela faixa principal | morto por 1 (`isolation::the_camera_length_counts_the_layers…`) |
| M14 lâmpada no relógio da instância | morto por 1 (corpus); sozinho no X estado, por 4 |
| **M15 lâmpada no quadro da instância (não no real)** | **sobreviveu** (as luzes do fixture não têm keys) → `a_keyed_lamp_of_either_deck_is_read_on_the_real_edit_frame` → morto por 1 |
| M16 primário vira em C+1 | morto por 4 (corpus); sozinho no X estado, por 4 |
| M17 início da janela sem checagem | morto por 2 (`timing_refusals`) |
| M18 / M19 `heldInstance` / `heldCamera` sem checagem | mortos por 1 cada (`timing_refusals`) |
| M20 / M21 amostra da instância / quadro bruto do primário sem checagem | mortos por 1 cada |
| M22 amostra do vizinho sem checagem | morto por 2 (`plan`, `timing_refusals`) |
| M23 tempos sem checagem | morto por 1 |
| M24 regra de id compartilhado invertida | morto por 1 (corpus) |
| M25 / M26 encaixe da instância do primário afrouxado / apertado | M25 morto por 1 (`timing_refusals`), M26 por 1 (corpus) |
| M27 / M28 `primary` / corte sem checagem | mortos por 1 cada (`timing_refusals`) |
| M29 held do vizinho lido do deck do primário | morto por 8 (corpus); sozinho no X estado, por 5 |

Rodada extra só contra `pilot_dissolve_representable_state` (M09, M14, M16, M29): os quatro foram mortos por esse binário sozinho (6, 4, 4 e 5 testes), ou seja, o consumidor X não depende dos outros binários.

Não mutados: o ramo `DurationAboveEdit` de `check_timing_window` (nenhum teste desta entrega o exercita sozinho: a checagem de `/transition/progress` recusa antes num plano tipado; fica registrado como braço sem prova própria) e a aritmética de `Ratio` do `frame_start_time` (coberta por testes de valor exato, 30000/1001, não por mutação).


## 7. Limites e pendências

- Windows independente e Mac: pendentes (coordenador). Nada aqui é aceite de produto, render ou desempenho.
- Core completo: 2 vermelhos da base; workspace global não executado (só compilado).
- O plano externo de 20.000 itens/4 MiB é construído em memória (nada commitado); um bundle real não chega perto.
- A comparação X é de **estado**. Pixels (390 sondas, 12 PNG) seguem para o renderer da S2; nenhuma tolerância nova.
- `scene_seconds` e as sondas são diagnósticos; a suavização `shaped` é a definição do runtime recalculada no teste.
- O primeiro/último quadro real da **câmera** do vizinho não existe no fixture (veja §4); a prova é sintética.

## 8. Patches literais para o coordenador

Números de linha são os de `docs/backlog.md`, `docs/features/README.md` e `docs/README.md` na base `c9c5fa0`. Nenhum
desses arquivos foi editado nesta entrega. `<PR>` = esta PR.

**`docs/backlog.md`, linha 1883 (BL082.4.4):**

```
| BL082.4.4 | P4b/S2 | Runtime/renderer/mistura e lifecycle/cache | Em andamento — C0a aceita PR69; C0b (relógios/timing do plano, teto 20000, consumidor X de estado) implementada em <PR>, a validar (Windows independente e Mac pendentes); H/R pendentes | BL082.4.3; S2A também BL082.4.5/S3a | Não atribuído | Cor8bits/alpha declarados, capability, dois splats/leases/contextloss/dispose e pixelsindependentes. C0b: `p4b-s2-c0b-report.md`; nenhum pixel/runtime/renderer. |
```

**Linha 2198 (BL082.4.17, tabela de pendências) e linha 2291 (mesma linha na tabela de revisão):**

```
| BL082.4.17 | PR63 / referência X | Três lit-held não importam por instância menor que shot, comparações só parcialmente preservadas | A validar — referência aceita Windows9bafc43/Mac; consumidor de estado implementado na S2-C0b (`pilot_dissolve_representable_state.rs`, 7 cenários/98 quadros, sem pixel), a validar na revisão independente; pixels/sondas de cor pendentes para o renderer | BL082.4.1/.4.3 | Coordenador / Cursor Sonnet5.5 | JulgarT06/T10/T16 e necessidade de fixtures representáveis sem enfraquecerimportador ouregravarorigem; não promover todosos estados/sondasporassociação. Autor declara reconstrução e limites. S2-C0b: papéis, relógios bruto/amostrado/preso dos dois decks, offsets/ids/limites, Ratio (30000/1001), câmera, marcador e luzes no relógio real comparados ao plano real; fora da janela = plano puro do primário; ordem independente; 11 classes de falsificação recusadas (`p4b-s2-c0b-report.md` §4). |
```
```
| BL082.4.17 | P4b/S0-X representável | Casos válidos de hold do vizinho e luz no quadro real, com bundles importáveis | A validar — referência aceita Windows9bafc43/Mac; consumidor de estado na S2-C0b (`pilot_dissolve_representable_state.rs`), a validar; pixels pendentes | BL082.4.1/.4.2 | Coordenador / Cursor Sonnet5.5 | 7cenários/98quadros/390sondas,37specs normal/-O e5Rust no Windows/Mac da entrega inicial. Nits: papéis só por quadro, Python3.9 sem metadata3.10, janela Rust e mapa de assets; relatório próprio. S2-C0b lê o JSON (sem Python/pixel) e compara estados do plano real; hold da câmera e UNH por estado estático provados; sondas de cor (390) seguem diagnósticas. Viés agregado por cenário vs texto canônico por quadro a decidir antes do engine. Origem não executada/golden não comparável. |
```

**Linha 2236 (BL082.4.20):**

```
| BL082.4.20 | S2 / contrato do plano | Wire sem C e comprimentos não permite validar toda a escolha temporal de primary/held | Implementado na S2-C0b — a validar (Windows independente e Mac pendentes) | BL082.4.3/.4.4 | S2-C0b / Cursor Sonnet5.5 | `alignment` e `timing` obrigatórios no wire com transição (C, intervalos, offsets, comprimentos, amostras), regra única de relógio, validador em três portas com ponteiro por campo, mutantes mortos; sem mudar `PLAN_VERSION` nem bytes sem transição. Coerência, não procedência. `p4b-s2-c0b-report.md` §2/§6. |
```

**Linha 2237 (BL082.4.21):**

```
| BL082.4.21 | Validação do teto de itens | Não há plano válido aceito de exatamente 20000 itens sob o teto de bytes | Implementado na S2-C0b — a validar (Windows independente e Mac pendentes) | BL082.4.3 | S2-C0b / Cursor Sonnet5.5 | Plano válido de 20000 itens ([3,2,0,9993,2,0] por deck) aceito em validate/Value/text; texto de exatamente 4194304 bytes aceito e +1 byte recusado; +1 passe/luz recusado em `/transition`; lights e vizinho contados; mutantes `>`/`>=`, vizinho e luzes mortos. Um bundle com MAX_FX_TRACKS=64 não produz esse plano: cotas do documento inalteradas. |
```

**Linha 2242 (BL082.4.23) e linha 2292 (mesma linha na tabela de revisão):**

```
| BL082.4.23 | S1b / relógio de câmera | Plano primário conserva contador bruto além da câmera curta; offset/oráculo usa contador preso | Implementado na S2-C0b — a validar (Windows independente e Mac pendentes) | BL082.4.17/.4.3 | S2-C0b / Cursor Sonnet5.5 | Contrato: o primário guarda o quadro **bruto** (bytes sem transição inalterados) e `timing.sampled*` + `PlanTransition::clocks` dão o amostrado e o preso de cada deck pela mesma regra do offset. Caso legado raw23/sample11 (câmera de 12 quadros) é teste explícito; o último quadro real não é preso. Pose é hold no kit; isto não é prova de pixels reais. |
```

**`docs/features/README.md`, linha 81 (F064):** substituir o trecho final «Wire/consistência(.20), plano20000(.21), contador(.23), consumidor X de estado/pixels e host/runtime/render/S2A/UI ainda pendentes.» por:

```
S2-C0b em <PR> (a validar, Windows independente/Mac pendentes): `alignment`/`timing` no plano com transição e regra única de relógio (.20/.23), plano externo válido de 20000 itens/4 MiB exatos (.21) e consumidor X de estado sem pixel (.17), core 1728/2/7 com os 2 gizmos da base vermelhos. Pixels e host/runtime/render/S2A/UI ainda pendentes.
```

**`docs/README.md`, inserir depois da linha 20:**

```
- [P4b S2-C0b — relógios do plano com transição](migration/p4b-s2-c0b-report.md): `alignment`/`timing`, regra única de relógio, teto exato de 20000 itens/4 MiB e leitura da referência representável como estado; admissão pública fechada.
```

## 9. Conferência do coordenador e reforço da câmera

No head4ee9370, o Mac nativo concluiu fmt/check/clippy/no-run/build/tree com saída0;
core1731/0/7 em66resultados (lib618), preserve e workspaceunificado214/10/5/8/1,
baselinerelease1 e desktopFX80. Logs comHEAD/command em
~/.cache/tessel-pr71-mac-20261115/summary.json. Workspace inteiro foi compilado,
não executado. Não há prova de Windows nesses números, nem runtime/pixel/GPU.

A comparação X original conferia posição da câmera, mas não target nem FOV. O
coordenador acrescenta os três eixos do alvo e o FOV em ambos os decks, usando
a definição analítica dos kits: truck desloca posição e alvo juntos; hold mantém
a pose; todos os clipes commitados têm initialFovDeg50 e nenhum segmento alteraFOV.
A leitura do fixture fixa essa pré-condição de FOV sem derivá-la do plano produzido.
Oito controles novos alteram isoladamente cada eixo de target e o FOV de cada deck;
eles se somam às11classes anteriores. Nenhum fixture/limiar/código de produção mudou.
Esta comparação continua sintética e de estado: não cobre pixels ou textura.

A prova vermelha no Mac rodou o consumidor original com os controles novos:7passam/1falha,
por aceitar `primary camera target x` incorreto. Com a comparação completa,8/8 passaram
no isolado e8/8 com preserve_order; fmt0. Logs `camera-controls-red.log`, `camera-green.log`
e `camera-preserve.log`, com hashes dos4arquivos da árvore candidata em
`camera-candidate-summary.json`, no diretório de logs acima. O clone voltou limpo a4ee9370;
nenhum resultado foi atribuído silenciosamente a outrohead. No commit publicado f3f1c8cc2eac5a4126017eac1bf20115f1dabc99, o Mac reexecutou
fmt0, Xestado8/8 isolado e8/8preserve, clippycorealltargets-Dwarnings0. Logs
`final-summary.json` e `final-*.log` no mesmo diretório, clone limpo.
A revisãoWindows independente do reforço continua pendente. Nenhum arquivo de
produção mudou entre4ee9370 e f3f1c8c; a única mudança Rust é em testes/comentário.

Os patches comuns foram integrados pelo coordenador na PR71 com status A validar.
O teto de bytes só é julgado na porta textual; as portas Value e tipada não recebem
texto. A contagem20000 é julgada nas três portas. Revisão Windows independente
em execução; Refuse fechado e F064produtoP.
