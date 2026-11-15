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
| Tipos das ops e das opções, `ShotOp::from_json`, `parse_shot_ops`, limites (`MAX_EDIT_FRAMES` 216000, lote de 256 ops, quadros até 2^52) | `crates/core/src/scene_edit/shots/mod.rs` |
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
  efeito** até a S2. `SHOTS_LAYER_NOT_YET` existe como código (T16), sem caso na lista de hoje.
- `limit_frame` segue a #33: o quadro mais próximo em que a regra não recusa, no mesmo relógio de `asked_frame` (o fim do plano,
  para uma duração). `EDIT_TOO_LONG` também carrega `max_frames`.
- `Refusal` é um `Box` por dentro, para o `Err` das funções do planejador ficar pequeno (`clippy::result_large_err`).

## Corpus: 121 dos 167 reproduzidos

O teste lê `docs/migration/fixtures/shot-trim/cases.json` sem Python nem rede, projeta cada documento mínimo para um
bundle 1.0 que o importador aceita (importado com `TransitionAdmission::Accept`, a porta escondida usada pela P4b),
executa `parse_shot_ops` + `apply_shots`, lê o resultado de volta ao formato mínimo e compara documento, notas e `changed`; nas recusas
compara o código, o índice da op (menos em `CANDIDATE_REJECTED`) e `limit_frame` com `limitFrame`. Também confere que a cena de
entrada não mudou.

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

### Os 46 pendentes e o porquê

Nenhum é uma divergência de regra: em todos o oráculo move uma camada (ou, no `explain`, produz uma leitura) que a S1 não
entrega por escopo. O sintoma da S1 é a recusa do portão do importador (sem reparo, sem esconder), o documento sem a camada
movida, ou, num caso, aceitar o que o oráculo recusa por causa de uma camada.

| Motivo | Quem resolve | Casos |
|---|---|---|
| Clipe `fx` tem de andar com o plano ou pela emenda (D09) — **22** | S2 (BL082.5.3) | |
| &nbsp;&nbsp;recusado pelo portão (`FX_SCOPE_MISMATCH`) | | `t11-head-cut-moves-clips`, `t11-end-shrink-cuts-tail`, `t11-roll-later-cuts-head-clips`, `t11-later-shot-clips-move-with-plan` |
| &nbsp;&nbsp;recusado pelo portão (`FX_CLIP_OUT_OF_RANGE`) | | `t12-shrink-removes-and-cuts`, `t12-head-cut-clip`, `t13-head-cut-shifts-and-prunes`, `t13-tail-cut-prunes`, `t13-fade-gone` |
| &nbsp;&nbsp;documento difere (o clipe não andou, nem as keys ou fades) | | `t11-head-earlier-anchor-moves-clips`, `t11-roll-follow-slides-clips`, `t11-continuation-clips-follow`, `t12-grow-stretches-crossing`, `t12-roll-follow-chain`, `t12-head-earlier-chain`, `t13-moves-whole-keys-intact`, `t13-stretched-keeps-keys`, `t13-fade-reduced`, `t13-fade-out-reduced`, `t13-all-keys-pruned`, `batch-with-output-clips` |
| &nbsp;&nbsp;aceito, o oráculo recusa (`FX_CLIP_OVERLAP`: um clipe de plano que a S2 moveria) | | `t12-keep-absolute-overlap` |
| Transição presa ao corte (D10) — **9**: a S1 recusa com `CANDIDATE_REJECTED` carregando `TRANSITION_TOO_LONG`/`TRANSITION_OUT_OF_RANGE` do importador, sem código tipado nem `limit_frame`, e sem `shrink_transitions` | S2 (BL082.5.3) | `t14-too-long`, `t14-shrink-center`, `t14-shrink-start`, `t14-shrink-end`, `t14-too-long-start-incoming`, `t14-too-long-roll`, `t14-too-long-head`, `t14-edit-duration`, `t14-two-transitions-shrink` |
| Keys de luz e `weatherKeys` têm de andar (D11, D12) — **8**: documento difere | S2 (BL082.5.3) | `t15-keys-move-after-point`, `t15-keys-dropped-in-removed`, `t15-empty-track-removed`, `t15-key-at-insertion-point-moves`, `t15-roll-follow-chain`, `t16-weather-keys-remapped`, `t16-weather-keys-dropped`, `t16-weather-keys-emptied` |
| `scene.shots.explain` (D16) — **7** | S3 (BL082.5.4) | `t19-explain-handles`, `t19-explain-locked-and-continuous`, `t19-audio-longer-than-edit`, `t19-edit-longer-than-audio`, `t19-audio-within-one-frame`, `t19-no-audio`, `t19-window-not-at-zero` |

### Limites do projetor (não são divergências de código)

O projetor completa o que o importador exige sem mudar nenhum campo que o oráculo modela. Dois ajustes saíram da primeira
execução (119 reproduzidos): o id de trilha `fx` do oráculo é reservado pelo importador (as trilhas ganham o prefixo `track-` na
ida e perdem na volta) e o áudio exige `sourcePcmHashSha256` não vazio (hash sintético). Os clipes `fx` do oráculo viram
`filmGrain` (`amount→amount`, `x→size`, `tint→luma`) e as transições viram `crossDissolve`; a volta lê só os campos que o oráculo modela.

## Divergências

**Nenhuma entre o Rust e o oráculo nos 121 casos reproduzidos**, e nenhuma regra do oráculo foi alterada nem enfraquecida. O
que merece o olhar do coordenador:

1. **Transições na S1 (9 casos `t14-*`).** O oráculo recusa com `TRANSITION_TOO_LONG` e `limitFrame`, ou aceita encolhendo
   (`shrink_transitions`). A S1 devolve `CANDIDATE_REJECTED` com o achado do importador. É o comportamento previsto pelo escopo, e a
   S2 troca por código tipado e `limit_frame`.
2. **`t12-keep-absolute-overlap`.** A S1 aceita um documento que o oráculo recusa com `FX_CLIP_OVERLAP`: o clipe de plano que o
   oráculo move não é movido na S1, então a sobreposição não existe. Some quando a S2 mover o clipe.
3. **T06 e o portão temporal.** Numa câmera que faz dolly a 0,2 unidades por quadro por 60 quadros e depois segura, o corte da
   junção `continuous` no quadro 59 (o último do dolly) é recusado com `CONTINUOUS_VELOCITY_JUMP`; o corte em 45 e o em 60
   passam. Isto é o comportamento do portão temporal do importador (compara o último quadro do plano anterior com o primeiro do
   seguinte), documentado no teste; a S1 não repara.

## Mutantes

Sete mutantes manuais em `shots/plan.rs`, cada um aplicado, executado contra `scene_shots` e `shot_trim_corpus` e revertido (a árvore
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
| `./scripts/cargo test -p tessel-editor-core --locked --no-fail-fast` | 101 | 67 suites, **1742 passam, 2 falham, 7 ignorados**; as 2 falhas são as conhecidas do Windows (`scene_gizmo_rotation::the_rotate_overlay_has_three_closed_rings_and_the_metrics_to_draw_them`, `scene_gizmo_scale::four_parents_of_anisotropic_scale_with_a_rotation_between_them_refuse_the_world_shear`); nenhuma outra |
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
- **Camadas e `explain`:** S2 e S3 (lista acima). `keep_absolute` e `shrink_transitions` só têm efeito na S2.
- **Texto localizado dos códigos novos:** S3 (T20).
- **Transições com admissão aberta:** a reserva do `auto` só é exercida de ponta a ponta com a S2A da P4b (BL082.5.8); aqui a
  prova é o planejador com `reserve` explícito e o corpus (`t14b-*`) sob `Accept`.
- **Lote longo demais** usa `INVALID_OP` (256 ops), e quadros além de 2^52 em op ou documento também; o plano não nomeia um código
  próprio para isso.

Nota de processo: o ambiente acrescentou um trailer de assistente ao `git commit`. Os commits foram feitos com
`git commit-tree` + `git reset --soft` a partir de arquivo de mensagem, e `git log -1 --format=%B` confere sem trailer.

## Patch para o coordenador

Não editei `docs/backlog.md` nem `docs/features/README.md`.

### `docs/backlog.md` — tabela P5

Substituir a linha BL082.5.2:

```
| BL082.5.2 | P5/S1 | Planejador e aplicação das quatro ops em planos, edição e instâncias; `content`, junções contínuas, `LOCKED`, limites; recusa D13 (`SHOTS_OPTION_NOT_YET`; `SHOTS_LAYER_NOT_YET` fica só como código, sem caso na lista de hoje). Sem `candidate_gate` extraído (decisão do dono): o portão é `reimport` + `edit_join_issues` + `temporal_issues` direto | A validar — entregue em `feat/p5-s1-shot-planner` (`a8738ca`, `bda03fc`): módulo `scene_edit::shots`, corpus **121 de 167** reproduzidos (98/98 sem camadas; 46 pendentes nomeados: 22 clipes `fx`, 9 transições, 8 keys e 7 `explain`, para S2 e S3), 44 testes de mão (T01–T10, T16, T17, T21, T22), 7 mutantes mortos, fmt/clippy do workspace limpos, `test -p tessel-editor-core` 1742 ok e só as 2 falhas conhecidas do Windows; sem push nem PR; [relatório](migration/p5-s1-report.md) | BL082.5.1 | Cursor Sonnet 5.5 / coordenador | Corpus da S0 reproduzido salvo os pendentes de S2/S3 (lista no teste); propriedade T21; T01–T10, T16, T17, T22 em L; revisão independente antes de integrar |
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
| F065 | [Duração e cortes dos planos da cena dirigida (P5, BL082.5)](shot-duration.md) | P | P | P | Parcial | `trim_shot`, `move_cut`, `set_shot_duration`, `set_edit_duration` (opções `mode`, `content`, `keep_absolute`, `shrink_transitions`; `with_narration` recusado) e `scene.shots.explain`. Lib: S1 entregue em `scene_edit::shots` (planejador puro e aplicação ao JSON, sem camadas): corpus 121/167 reproduzidos (pendentes nomeados para S2/S3), T01–T10, T16, T17, T21, T22 em L, 7 mutantes mortos; sem `SceneCommand`, JSONL nem janela. `keep_absolute` e `shrink_transitions` são lidos e sem efeito até a S2. S0: contrato, auditoria dos 17 campos adiados, D07, referência Python e corpus de 167 casos; `ORIGIN_NOT_EXECUTED`; `AUTO_RESERVE` (#29) decidido pela leitura do oráculo. |
```

Se o coordenador preferir manter a coluna Lib como `P` até a revisão independente, o restante da linha vale igual.
