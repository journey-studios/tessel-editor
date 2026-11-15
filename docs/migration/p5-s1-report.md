# P5 / S1 — relatório (BL082.5.2)

Entrega da fatia S1 da [P5](p5-shot-duration-plan.md): o planejador e a aplicação das quatro ops de plano
(`trim_shot`, `move_cut`, `set_shot_duration`, `set_edit_duration`) em `crates/core`, sem GPUI, janela nem runtime.
Status do produto: **lib parcial** (planos, edição e instâncias; as camadas são da S2, `SceneCommand` e JSONL são da S3,
a UI é da S4). O contrato é o [F065](../features/shot-duration.md); o oráculo e o corpus são os da
[S0](p5-s0-report.md).

Branch `feat/p5-s1-shot-planner`, sobre `docs/p5-shot-duration-plan`. Sem push e sem PR (pedido do coordenador).

## O que entrou

| Entrega | Arquivo |
|---|---|
| Códigos de recusa, `Refusal` (com os fatos da D19: `shot`, `edge`, `asked_frame`, `limit_frame`, extras) | `crates/core/src/scene_edit/shots/error.rs` |
| Tipos das ops e das opções, `ShotOp::from_json`, `parse_shot_ops`, limites (`MAX_EDIT_FRAMES` 216000, lote de 256 ops, quadros até 2^53−1, o `MAX_SAFE_INT` do importador) | `crates/core/src/scene_edit/shots/mod.rs` |
| Planejador puro `plan_op` → `ShotPlan` (novos intervalos, `instanceStartFrame`/`cameraInFrame`, instâncias que crescem, edição, bordas movidas, `content_shift`, cadeia de emendas) | `crates/core/src/scene_edit/shots/plan.rs` |
| Aplicação ao JSON clonado, op após op (D02), e o portão do candidato | `crates/core/src/scene_edit/shots/apply.rs` |
| `pub mod shots;` | `crates/core/src/scene_edit/mod.rs` |
| Projetor do corpus (documento mínimo do oráculo → bundle 1.0 e de volta) | `crates/core/tests/shot_trim_support/mod.rs` |
| Corpus de 167 casos lido pelo Rust, com lista de pendentes explícita | `crates/core/tests/shot_trim_corpus.rs` |
| Matriz T01–T10, T16, T17, T21, T22, o lote e o planejador com reserva (44 testes) | `crates/core/tests/scene_shots.rs` e `crates/core/tests/scene_shots/{support,ops,joins,batch,property}.rs` |

API pública (no molde de `apply_fx`): `apply_shots(&DirectedScene, &[ShotOp]) -> Result<ShotsOutcome, EditError>`
(`candidate`, `notes`, `changed`) e `apply_shots_with(.., &Limits)`; `parse_shot_ops(&Value)` aceita as formas do plano §6.2.
`DirectedScene`, `fx.rs`, `batch.rs` e o restante da lista de arquivos proibidos **não foram tocados**.

### Como o planejador decide

- O planejador não conhece JSON: lê planos ordenados por início e devolve um plano ou uma `Refusal`. A aplicação escreve o plano
  no clone, reimporta (`DirectedScene::reimport`, com a admissão da própria cena) e passa o candidato por
  `edit_join_issues()` e `temporal_issues()`; qualquer achado vira `CANDIDATE_REJECTED` ligado à op pelo ponteiro JSON
  (`batch::rejected`). Sem `candidate_gate` compartilhado, por decisão do dono (ver "Dívidas").
- Ordem das recusas (a do oráculo): opção, ids, adjacência, plano curto demais, junção contínua, quadros de cabeça
  (`NO_HEAD_FRAMES` antes de `EDIT_TOO_LONG`), edição longa demais, `LOCKED`, e por fim o portão.
- `auto`: `anchored = max(0, min(d, cabeça − reserva))` (`AUTO_RESERVE`). A reserva vem de `fx.transitions` com
  `toShotId == plano` e `handles != hold`; em produção é 0, porque a admissão pública recusa transições. Num `continuous` de
  entrada o `auto` ancora tudo e nunca segue (#30). `AUTO_FELL_TO_FOLLOW` só sai com `content == auto` e `followed > 0`.
- Cada instância cresce para `max(instanceStartFrame + quadros)` dos seus planos e nunca encolhe; a nota é `INSTANCE_GREW`
  (`instance`, `from`, `to`).
- `LOCKED`: planos de uma instância `editLocked` que estavam em passo (mesmo `instanceStartFrame − startFrame`) têm de
  continuar em passo. Sob `auto`, a tentativa que quebraria o passo cai para a seguinte; se nenhuma serve, `LOCKED`.
- Uma op que não muda nada devolve a cena de entrada com `changed: false` e **não** roda o portão.
- `with_narration` é `SHOTS_OPTION_NOT_YET` (D13). `keep_absolute` e `shrink_transitions` são lidos e carregados e **não têm
  efeito** até a S2. `SHOTS_LAYER_NOT_YET` é a recusa provisória da S1: sai quando a op exigiria mover uma camada que a S1 não move (ver "Revisão R1").
- `limit_frame` segue a #33: o quadro mais próximo em que a regra não recusa, no mesmo relógio de `asked_frame` (o fim do plano,
  para uma duração). `EDIT_TOO_LONG` também carrega `max_frames`.
- `Refusal` é um `Box` por dentro, para o `Err` das funções do planejador ficar pequeno (`clippy::result_large_err`).

## Corpus: 121 dos 167 reproduzidos

O teste lê `docs/migration/fixtures/shot-trim/cases.json` sem Python nem rede, projeta cada documento mínimo para um
bundle 1.0 que o importador aceita (importado com `TransitionAdmission::Accept`, a porta escondida usada pela P4b),
executa `parse_shot_ops` + `apply_shots`, lê o resultado de volta ao formato mínimo e compara documento, notas e `changed`; nas recusas
compara o código, o índice da op (menos em `CANDIDATE_REJECTED`) e `limit_frame` com `limitFrame`. Nas recusas, os códigos dos achados do portão (`CANDIDATE_REJECTED`) têm de conter cada `refusal.findings[].code` do oráculo.

| | Casos | Reproduzidos | Pendentes |
|---|---:|---:|---:|
| Ops sem camadas | 98 | 98 | 0 |
| Ops com camadas (clipes `fx`, transições, keys de luz, `weatherKeys`) | 62 | 23 | 39 |
| `explain` | 7 | 0 | 7 |
| **Total** | **167** | **121** | **46** |

Por forma: 87 aceitos e 34 recusas reproduzidos; os 17 gêmeos `@limit` reproduzem. Dos 23 casos com camadas que já
reproduzem, a recusa vem antes das camadas, a op não as toca ou o documento não precisa delas mexidas.

O teste `s1_reproduces_every_case_it_does_not_name_pending` falha nos dois sentidos: um caso fora da lista que não reproduz, e um
caso da lista que passa a reproduzir sem sair dela. Também confere que o motivo declarado corresponde a uma camada que o
documento tem. Nada é pulado em silêncio.

### Os 46 pendentes e o sintoma de cada um

Nenhum é uma divergência de regra: em todos o oráculo move uma camada (ou, no `explain`, produz uma leitura) que a S1 não
entrega por escopo. Desde a R1 cada caso tem o **sintoma fixado** no teste (`Symptom`): se a S1 mudar de comportamento num deles,
o teste falha. Nenhum caso resta no estado "documento difere" (a S1 recusa em vez de devolver o documento sem a camada movida).

| Sintoma na S1 | Quem resolve | Casos |
|---|---|---|
| `SHOTS_LAYER_NOT_YET`, `layer = fx_clip_shot` — **8** | S2 (BL082.5.3) | `t11-head-cut-moves-clips`, `t11-end-shrink-cuts-tail`, `t11-head-earlier-anchor-moves-clips`, `t11-roll-follow-slides-clips`, `t11-roll-later-cuts-head-clips`, `t11-continuation-clips-follow`, `t11-later-shot-clips-move-with-plan`, `t12-keep-absolute-overlap` |
| `SHOTS_LAYER_NOT_YET`, `layer = fx_clip_output` — **14** | S2 | `t12-shrink-removes-and-cuts`, `t12-grow-stretches-crossing`, `t12-head-cut-clip`, `t12-roll-follow-chain`, `t12-head-earlier-chain`, `t13-moves-whole-keys-intact`, `t13-head-cut-shifts-and-prunes`, `t13-tail-cut-prunes`, `t13-stretched-keeps-keys`, `t13-fade-reduced`, `t13-fade-out-reduced`, `t13-fade-gone`, `t13-all-keys-pruned`, `batch-with-output-clips` |
| `SHOTS_LAYER_NOT_YET`, `layer = transition` — **4** | S2 | `t14-shrink-center`, `t14-shrink-start`, `t14-shrink-end`, `t14-two-transitions-shrink` |
| `SHOTS_LAYER_NOT_YET`, `layer = light_keys` — **5** | S2 | `t15-keys-move-after-point`, `t15-keys-dropped-in-removed`, `t15-empty-track-removed`, `t15-key-at-insertion-point-moves`, `t15-roll-follow-chain` |
| `SHOTS_LAYER_NOT_YET`, `layer = weather_keys` — **3** | S2 | `t16-weather-keys-remapped`, `t16-weather-keys-dropped`, `t16-weather-keys-emptied` |
| `CANDIDATE_REJECTED` com `TRANSITION_TOO_LONG` do importador — **3** (sem `shrink_transitions` e sem código tipado nem `limit_frame`) | S2 | `t14-too-long`, `t14-too-long-roll`, `t14-too-long-head` |
| `CANDIDATE_REJECTED` com `TRANSITION_OUT_OF_RANGE` do importador — **2** | S2 | `t14-too-long-start-incoming`, `t14-edit-duration` |
| `scene.shots.explain` (D16) — **7** | S3 (BL082.5.4) | `t19-explain-handles`, `t19-explain-locked-and-continuous`, `t19-audio-longer-than-edit`, `t19-edit-longer-than-audio`, `t19-audio-within-one-frame`, `t19-no-audio`, `t19-window-not-at-zero` |

Um sintoma `Layer(..)` exige que a S1 recuse com esse código **e** essa camada; `Gate(..)` exige `CANDIDATE_REJECTED` com exatamente
esses achados; `Explain` exige que o caso seja uma leitura e não uma op. O sintoma `LayersOnly` (aceita; o que a S1 controla —
planos, instâncias, edição, sem `fx` nem `sceneDefinitions` — tem de ser igual ao do oráculo) existe no teste e hoje não tem caso.
### Limites do projetor (não são divergências de código)

O projetor completa o que o importador exige sem mudar nenhum campo que o oráculo modela. Dois ajustes saíram da primeira
execução (119 reproduzidos): o id de trilha `fx` do oráculo é reservado pelo importador (as trilhas ganham o prefixo `track-` na
ida e perdem na volta) e o áudio exige `sourcePcmHashSha256` não vazio (hash sintético). Os clipes `fx` do oráculo viram
`filmGrain` (`amount→amount`, `x→size`, `tint→luma`) e as transições viram `crossDissolve`; a volta lê só os campos que o oráculo modela.

## Divergências

**Nenhuma entre o Rust e o oráculo nos 121 casos reproduzidos**, e nenhuma regra do oráculo foi alterada nem enfraquecida. O
que merece o olhar do coordenador:

1. **Transições na S1 (9 casos `t14-*`).** O oráculo recusa com `TRANSITION_TOO_LONG` e `limitFrame`, ou aceita encolhendo
   (`shrink_transitions`). A S1 devolve `CANDIDATE_REJECTED` com o achado do importador (5 casos) ou `SHOTS_LAYER_NOT_YET`
   (`transition`, 4 casos com `shrink_transitions`); a S2 troca por código tipado e `limit_frame`.
2. **`t12-keep-absolute-overlap`.** Antes da R1 a S1 aceitava um documento que o oráculo recusa com `FX_CLIP_OVERLAP`. Agora recusa com
   `SHOTS_LAYER_NOT_YET` (`fx_clip_shot`), porque o clipe de plano que o oráculo move não é movido na S1.
3. **T06 e o portão temporal.** Numa câmera que faz dolly a 0,2 unidades por quadro por 60 quadros e depois segura, o corte da
   junção `continuous` no quadro 59 (o último do dolly) é recusado com `CONTINUOUS_VELOCITY_JUMP`; o corte em 45 e o em 60
   passam. Isto é o comportamento do portão temporal do importador (compara o último quadro do plano anterior com o primeiro do
   seguinte), documentado no teste; a S1 não repara.

## Mutantes

Sete mutantes manuais (a tabela original; a R1 refez a execução e acrescentou seis, abaixo) em `shots/plan.rs`, cada um aplicado, executado contra `scene_shots` e `shot_trim_corpus` e revertido (a árvore
ficou limpa depois de cada um; nenhum permanece no código).

| # | Mutante | Mortos por |
|---|---|---|
| M1 | O ripple do fim não move os planos seguintes (`shift_later` removido) | 14 de `scene_shots` (`t01_a_longer_shot_moves_the_following_ones_and_grows_the_edit`, `t05_growing_the_start_of_a_take_slides_what_carries_on`, `a_batch_applies_op_after_op_and_is_atomic`, …) e o corpus |
| M2 | `limit_frame` de `SHOT_TOO_SHORT` um quadro além (`start + 2`) | `t01_a_shot_shorter_than_one_frame_is_refused_with_the_legal_end`, `t04_an_edit_that_leaves_no_frame_to_the_last_shot_is_refused_with_the_limit` e o corpus |
| M3 | O `auto` ignora a reserva da transição (`min(d, head)`) | `the_reserve_of_a_transition_comes_off_what_auto_anchors` e o corpus (T14b) |
| M4 | `LOCKED` desligado (`lock_problem` não é chamado) | quatro `t07_*` e o corpus |
| M5 | A instância "cresce" também ao encolher (`needed != frames`) | 16 de `scene_shots` (entre eles `t08_an_instance_grows_to_fit_with_a_note_and_never_shrinks`, `t21_random_sequences_…`) e o corpus |
| M6 | A continuação do take não desliza (`slide_continuation` removido) | `t05_growing_the_start_…`, `t05_shrinking_the_start_…`, `t06_a_cut_that_breaks_the_camera_pose_…` e o corpus |
| M7 | `limit_frame` de `SHOT_TOO_SHORT` frouxo demais (`start`) | `t01_…legal_end`, `t04_…limit` e a propriedade `t21_random_sequences_pass_the_gate_refusals_change_nothing_and_the_limit_does_not_refuse` |

Achado do exercício: a propriedade T21 pega um limite **frouxo** (M7, o gêmeo no limite volta a recusar), mas não um limite
**conservador** demais (M2): nesse caso a op no limite passa. Quem fixa o valor exato do limite são os testes de mão
(`t01`, `t02`, `t03`, `t04`, `t09`, `t10`).

## Gates

Windows, Rust 1.99.1, `./scripts/cargo` no sh do Git. Contagens são de binários de teste (suites) e testes.

| Comando | Saída | Resultado |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | limpo |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 | limpo |
| `./scripts/cargo test -p tessel-editor-core --locked --no-fail-fast` | 101 | 67 suites, **1761 passam, 2 falham, 7 ignorados** (valores após a R1; eram 1742 na entrega); as 2 falhas são as conhecidas do Windows (`scene_gizmo_rotation::the_rotate_overlay_has_three_closed_rings_and_the_metrics_to_draw_them`, `scene_gizmo_scale::four_parents_of_anisotropic_scale_with_a_rotation_between_them_refuse_the_world_shear`); nenhuma outra |
| `./scripts/cargo tree -p tessel-editor-core` | 0 | 17 linhas: `flate2`, `serde`, `serde_json` e, como dev, `proc-macro2`; **nenhum `gpui` nem `wry`** |
| `scripts/gizmo-perf/hygiene.spec.py` (Python 3.13) | 0 | 13 testes ok |
| `scripts/backlog-tables.spec.py` (Python 3.13) | 0 | 19 testes ok |
| `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked --no-fail-fast` | 101 | 136 suites, 2814 passam, **88 falham**, 15 ignorados. **Não comparado com a `main`** |

Sobre as 88 falhas do workspace com `TESSEL_REQUIRE_RUNTIME=1`: além das 2 do core, são 86 em `tessel-editor-render` e
`tessel-editor-media`. Amostras lidas: o runtime não está instalado neste checkout (`The runtime dependencies are not
installed … Run: pnpm --dir …/runtime install`, e `TESSEL_REQUIRE_RUNTIME=1` transforma o "pular" em falha) e os testes de
mídia assumem caminho POSIX (`the workspace /p/ws is not an absolute path`) e o travamento da raiz no Windows. A S1 só
acrescenta um módulo e testes a `crates/core`; `render` e `media` não os usam. Para provar de fato que a diferença é nula,
rodar o mesmo comando na `main` depois de `pnpm --dir runtime install`; não foi feito aqui (custo de um segundo build do
workspace). Nenhuma contenção de WebGL foi encontrada: o runtime nem sobe.

## Dívidas e o que não foi feito

- **`candidate_gate` compartilhado.** Não extraído (decisão do dono para a S1): `apply.rs` chama `reimport` +
  `edit_join_issues` + `temporal_issues` direto, como `fx::apply_fx` e `batch`. Há três cópias da mesma sequência; extrair é uma
  entrega própria (ver proposta abaixo).
- **Códigos no comando.** `EditError` com os códigos novos vira o código de comando `invalid_edit` até a S3 ligá-los.
- **Camadas e `explain`:** S2 e S3 (lista acima). Até a S2 a S1 recusa com `SHOTS_LAYER_NOT_YET` o que exigiria mover uma camada; `keep_absolute` e `shrink_transitions` só decidem se a recusa acontece.
- **Texto localizado dos códigos novos:** S3 (T20).
- **Transições com admissão aberta:** a reserva do `auto` só é exercida de ponta a ponta com a S2A da P4b (BL082.5.8); aqui a
  prova é o planejador com `reserve` explícito e o corpus (`t14b-*`) sob `Accept`.
- **Lote longo demais** usa `INVALID_OP` (256 ops), e quadros fora de ±(2^53−1) em op ou documento também (inclusive `i64::MIN`); o plano não nomeia um código
  próprio para isso.

Nota de processo: o ambiente acrescentou um trailer de assistente ao `git commit`. Os commits foram feitos com
`git commit-tree` + `git reset --soft` a partir de arquivo de mensagem, e `git log -1 --format=%B` confere sem trailer.

## Revisão R1

Correções dos achados da revisão independente R1, no branch `feat/p5-s1-shot-planner-r1` (sobre `1cf7081`), um commit por assunto:
`c5ea21d` (P1-2), `afe3743` (P1-1, P2-3, P3), `617de18` (P2-1), `97671c7` (P2-4, P2-2), `d1b28bd` (T06) e o commit de documentação.
O oráculo (`scripts/shot-trim-reference.py`) e o corpus (`cases.json`) **não foram alterados**; `--check` reproduz os 2 arquivos byte a byte.

| Achado | Correção | Teste |
|---|---|---|
| P1-1 `i64::MIN` entra em pânico em `abs()` | `in_bound` testa o intervalo `-FRAME_BOUND..=FRAME_BOUND`; `plan_op` revalida op e documento no topo com `INVALID_OP` (op tipada não passa pelo parser) | `scene_shots/robust.rs`: `i64::MIN`/`i64::MAX` por JSON e por op tipada, nas quatro ops; documento tipado com quadro fora do limite |
| P1-2 T21 aceitava um planejador que dispara o portão | a propriedade exige `code != CANDIDATE_REJECTED` (e `!= SHOTS_LAYER_NOT_YET`) em toda recusa | `scene_shots/property.rs`; mutante M5b (abaixo) |
| P2-1 achados do oráculo não comparados | o corpus exige que os códigos dos achados do `CANDIDATE_REJECTED` contenham cada `refusal.findings[].code` | `shot_trim_corpus.rs`: `t12-keep-absolute-out-of-range`, três `t14b-*`, `batch-gate-refusal` (5 casos) |
| P2-2 pendentes sem sintoma | cada `PENDING` carrega o `Symptom` (`Layer`, `Gate`, `LayersOnly`, `Explain`); se aceita, compara o que a S1 controla (sem `fx` nem `sceneDefinitions`); se recusa, exige o código fixado | `s1_reproduces_every_case_it_does_not_name_pending` |
| P2-3 quadros lidos de forma estrita, `set_int` reescrevia valores iguais | `lenient_int` replica o `as_integer` do importador (`30.0` aceito, até 2^53−1); `set_int` compara numericamente; limite alinhado ao `MAX_SAFE_INT` | `robust.rs`: documento com `.0` editado por uma op que muda um plano, o resto igual byte a byte |
| P2-4 op que deixaria uma camada para trás passava | `shots/layers.rs`: `SHOTS_LAYER_NOT_YET` com `details.path` (primeiro item) e `details.layer` (`fx_clip_output`, `fx_clip_shot`, `transition`, `light_keys`, `weather_keys`). Com `keep_absolute` clipes de saída, keys de luz e `weatherKeys` não recusam; clipes de plano e transições ainda recusam | `scene_shots/layers.rs` e o t16 renomeado `t16_an_op_that_would_move_an_output_clip_is_refused_and_one_that_leaves_it_alone_is_not` |
| P3 `details` incoerentes | `shot`/`edge` são o alvo da op; `affected_shot` é extra; `NO_HEAD_FRAMES` do `move_cut` carrega o plano que entra | `ops.rs`/`joins.rs` |
| P3 exportações | `INSTANCE_GREW`, `AUTO_FELL_TO_FOLLOW`, `FollowFallback`, `Facts` e `PlanTarget` exportados | compilação dos testes |
| P3 `REFUSED_LAYERS` | lista vazia, na posição da ordem de recusa do oráculo (opção → ids → camadas recusadas → regras do planejador) | `apply.rs` |
| P3 custos quadráticos | `lock_problem` agrupa por passo antigo (mesmo primeiro par do oráculo); `placement` por mapa de ids; `reserve_of` por mapa de `toShotId` | corpus e `scene_shots` iguais |
| P3 comentário de `MAX_SHOT_BATCH_OPS` | um clone para o lote | — |
| P3 asserts vácuos "cena de entrada mudou" | removidos | — |
| P3 campos desconhecidos | teste de preservação em plano, `editRange`, instância e raiz | `robust.rs` |
| P3 T06 | corte em 60 e `edit_join_issues()` vazio para o candidato do quadro 59 (só o portão temporal recusa) | `t06_the_rejected_cut_meets_every_structural_rule_and_only_the_temporal_gate_says_no` |

### Desvio do pedido: a regra de transição é mais estreita

A recusa `SHOTS_LAYER_NOT_YET` por transição só dispara quando a transição ligada a um plano tocado não cabe mais **e** `shrink_transitions` pediu o encolhimento que a S1 não faz. Sem a opção, uma transição que não cabe é o achado do próprio portão (`TRANSITION_TOO_LONG`/`TRANSITION_OUT_OF_RANGE`, 5 casos acima), e uma que ainda cabe fica como o oráculo a deixa. É mais estreito que o texto do pedido (`transição ligada a um plano tocado`); o ganho é manter o código do importador onde ele já diz o motivo e não recusar o que o oráculo aceita sem mexer na transição. Decisão para o coordenador confirmar na S2.

### Números depois da R1

- Corpus: **121 de 167 reproduzidos**, 46 pendentes, todos com sintoma (tabela acima). Nenhum "documento difere".
- `scene_shots`: **63 testes** (eram 44); `shot_trim_corpus`: 2 testes.
- Mutantes: os sete originais foram re-executados na árvore nova e seguem mortos; seis novos foram acrescentados. Todos revertidos.

| # | Mutante | Mortos por |
|---|---|---:|
| M1 | ripple não move os planos seguintes | 27 testes |
| M2 | `limit_frame` um quadro além | 3 |
| M3 | `auto` ignora a reserva | 2 |
| M4 | `LOCKED` desligado | 6 |
| M5 | a instância encolhe (`needed != frames`) | 18 (a T21 mata pelo "a instância encolheu") |
| M6 | a continuação não desliza | 5 |
| M7 | `limit_frame` frouxo demais | 4 |
| M8 | `abs()` no parser | 1 |
| M9 | sem limite nas ops tipadas | 2 |
| M10 | guarda de camada desligada | 10 |
| M11 | `set_int` cru | 2 |
| M12 | `keep_absolute` ignorado para clipes | 3 |
| M13 | quadros estritos | 2 |
| M5b | a instância cresce um quadro a menos | morto pela nova T21 (`CANDIDATE_REJECTED`, `SHOT_EXCEEDS_INSTANCE_DURATION`); sobrevive com a propriedade antiga |

**O mutante da "instância que encolhe" (M5) é morto pela T21 agora** — e o M5b, que antes sobrevivia à propriedade por ela aceitar `CANDIDATE_REJECTED`, também.

### Gates da R1

| Comando | Saída | Resultado |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | limpo |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 | limpo |
| `./scripts/cargo test -p tessel-editor-core --locked --no-fail-fast` | 101 | 67 suites, **1761 passam, 2 falham, 7 ignorados**; só as 2 falhas conhecidas do Windows |
| `./scripts/cargo tree -p tessel-editor-core` | 0 | nenhum `gpui` nem `wry` |
| `scripts/shot-trim-reference.py --check` | 0 | 167 casos, 2 arquivos reproduzidos byte a byte |
| `scripts/gizmo-perf/hygiene.spec.py` | 0 | 13 testes ok |
| `scripts/backlog-tables.spec.py` | 0 | 19 testes ok |
## Patch para o coordenador

Não editei `docs/backlog.md` nem `docs/features/README.md`.

### `docs/backlog.md` — tabela P5

Substituir a linha BL082.5.2:

```
| BL082.5.2 | P5/S1 | Planejador e aplicação das quatro ops em planos, edição e instâncias; `content`, junções contínuas, `LOCKED`, limites; recusa D13 (`SHOTS_OPTION_NOT_YET`; `SHOTS_LAYER_NOT_YET` fica só como código, sem caso na lista de hoje). Sem `candidate_gate` extraído (decisão do dono): o portão é `reimport` + `edit_join_issues` + `temporal_issues` direto | A validar — entregue em `feat/p5-s1-shot-planner` (`a8738ca`, `bda03fc`): módulo `scene_edit::shots`, corpus **121 de 167** reproduzidos (98/98 sem camadas; 46 pendentes, cada um com o sintoma fixado: 34 recusas `SHOTS_LAYER_NOT_YET` por camada, 5 recusas do portão por transição e 7 `explain`), 63 testes de mão em `scene_shots` (T01–T10, T16, T17, T21, T22, camadas e robustez), 13 mutantes mortos, fmt/clippy do workspace limpos, `test -p tessel-editor-core` 1761 ok e só as 2 falhas conhecidas do Windows; revisão R1 em `feat/p5-s1-shot-planner-r1`; sem push nem PR; [relatório](migration/p5-s1-report.md) | BL082.5.1 | Cursor Sonnet 5.5 / coordenador | Corpus da S0 reproduzido salvo os pendentes de S2/S3 (lista no teste); propriedade T21; T01–T10, T16, T17, T22 em L; revisão independente antes de integrar |
```

Acrescentar:

```
| BL082.5.9 | P5/S1 | `candidate_gate` compartilhado: `fx::apply_fx`, `batch` e `shots::apply` repetem `reimport` + `edit_join_issues` + `temporal_issues` + `rejected(..)`; extrair sem mudar comportamento (decisão do dono de não fazer na S1) | Pendente | BL082.5.2 | — | Uma função chamada pelos três; os testes de `scene.edit`, `scene.fx.edit` e `scene_shots` seguem iguais; entrega própria, depois da S1 integrada |
| BL082.5.10 | P5/S1 | Prova do workspace com o runtime instalado: o `TESSEL_REQUIRE_RUNTIME=1 test --workspace` no Windows deste checkout falha 88 testes (runtime não instalado; caminhos e travas POSIX em `media`), sem comparação com a `main` | Pendente | BL082.5.2 | — | Rodar o mesmo comando na `main` e no branch depois de `pnpm --dir runtime install` e comparar a lista; só as 2 falhas conhecidas do core permanecem |
```

Na linha da BL082.5.3, acrescentar ao aceite: os 39 casos de camadas e os 9 de transições da lista de pendentes de
`crates/core/tests/shot_trim_corpus.rs` passam a reproduzir e saem da lista (o teste falha enquanto estiverem nela).
Na da BL082.5.4, os 7 casos `explain`.

### `docs/features/README.md` — F065

Substituir a linha F065 (a coluna Lib passa de `P` para `Parcial`):

```
| F065 | [Duração e cortes dos planos da cena dirigida (P5, BL082.5)](shot-duration.md) | P | P | P | Parcial | `trim_shot`, `move_cut`, `set_shot_duration`, `set_edit_duration` (opções `mode`, `content`, `keep_absolute`, `shrink_transitions`; `with_narration` recusado) e `scene.shots.explain`. Lib: S1 entregue em `scene_edit::shots` (planejador puro e aplicação ao JSON, sem camadas): corpus 121/167 reproduzidos (pendentes com sintoma fixado para S2/S3), T01–T10, T16, T17, T21, T22 em L, 13 mutantes mortos; sem `SceneCommand`, JSONL nem janela. Até a S2, uma op que exigiria mover uma camada que a S1 não move é recusada com `SHOTS_LAYER_NOT_YET` (`path`, `layer`); `keep_absolute` e `shrink_transitions` são lidos e só mudam essa decisão. S0: contrato, auditoria dos 17 campos adiados, D07, referência Python e corpus de 167 casos; `ORIGIN_NOT_EXECUTED`; `AUTO_RESERVE` (#29) decidido pela leitura do oráculo. |
```

Se o coordenador preferir manter a coluna Lib como `P` até a revisão independente, o restante da linha vale igual.
