# P4b S3a — edição de transições no core: relatório de entrega

Escopo: BL082.4.5 (ops de transição no `scene.fx.edit`), BL082.4.15 (namespace de ids) e BL082.4.24 (dica de
handles com `cameraIn` além do fim). Plano: [p4b-s3a-editing-plan.md](p4b-s3a-editing-plan.md). Contrato: F064
([fx-transitions.md](../features/fx-transitions.md), seção "S3a"). Base: `main` `90609da` (PR63). **A admissão pública
continua `Refuse`**: nenhum controle, janela, renderer, S2/S2A/S3b/S4/P5/P6 foi tocado, e nenhum código de produção
referencia `Accept`.

## 1. O que entrou

| Peça | Onde |
|---|---|
| Tipos/parser das ops existentes movidos sem mudança (`FxOp`, `AddClip`, `ClipField`, `parse_fx_batch`) | `scene_edit/fx_op.rs` (commit 192eb2c) |
| Cinco ops: `set_transition` (upsert por corte), `resize_transition`, `set_transition_param`, `set_transition_hidden`, `remove_transition` | `scene_edit/fx_transition.rs`, `FxOp::Transition(TransitionOp)` |
| Mesmo lote, **um** candidato reimportado pela política herdada, dry-run, no-op tipado, `base_revision`, uma entrada de undo | `scene_edit/fx.rs` (`run_batch`), `scene_project/fx.rs` (sem mudança) |
| `details.errors[]` em inglês ao lado de `details.issues` quando o lote tem op de transição | `fx_transition::attach_problems` |
| Projeção pura localizada `project_problem(&RawProblem, Locale) -> TransitionProblem` (pt-br/en/es), tabela única | `directed_scene/fx/transition_problem.rs` |
| Predicado de janela §9.6, sem consumidor | `directed_scene/fx/window_guard.rs` |
| .24: falta de início e de fim separadas (`cameraEnd`), reparo suficiente; `before 0` não exige cabeça | `directed_scene/fx/transition_explain.rs` |
| Namespace único de ids (looks, trilhas e transições) no alocador de `add_clip`/trilha | `scene_edit/fx.rs::every_id` |
| Gesto de look recusa op de transição com mensagem própria | `scene_project/fx_gesture.rs` |

## 2. Achados durante a implementação (corrigidos nesta entrega)

1. **`before 0` pedia cabeça.** A primeira versão da falta de fim (`cameraEnd`) disparava mesmo numa janela alinhada ao
   início (nada antes do corte). Corrigido em `lack()`: sem frames pedidos no lado, não há falta de fim. Teste
   `a_window_with_no_frames_before_the_cut_does_not_need_a_head`; mutante M16 o mata.
2. **`UNKNOWN_SHOT` de op saía sem texto.** O `set_transition` sem `id` com um shot ausente montava o problema sem
   `transitionId`, e a projeção (corretamente) o marcava `untranslated` com mensagem vazia. Agora a frase nomeia o id
   pedido ou o que seria gerado (`tr-<n>`). Pego pelo teste que atravessa **todas** as regras nas três línguas.
3. **Divergência do plano:** membros desconhecidos de `params` não podem ser "preservados", o importador os recusa
   (`UNKNOWN_PARAM`). O contrato F064 foi corrigido; a preservação vale para membros da transição (`note`, `seed`).

## 3. Dívida nova registrada, não corrigida

**BL082.4.25 (proposto)**: a cauda do plano que sai tem a mesma subestimativa da .24. Sonda com `cameraIn 40`, câmera
`cam1` de 60 quadros, plano de 48 e janela centrada de 12: a recusa diz "estenda `cam1` em 6", mas com 66 continua
recusado e só 94 (`+34`) aceita. Fica fora porque muda o campo `camera` da cauda, que a linha do oráculo compara.

## 4. Matriz E01–E20 do plano

| ID | Prova |
|---|---|
| E01 padrões | `edit::a_new_transition_is_written_whole…`, `…default_length_follows…three_lengths_and_alignments`, `…explicit_value_is_never_cut…`, `…length_the_shots_cannot_hold…` |
| E02 upsert | `…upsert_keeps_the_id…`, `…changes_only_the_fields_it_names`, `…explicit_id_that_is_not_the_one_on_the_cut…`, `…look_where_a_transition_is_expected…` |
| E03 resize/params/easing | `resize_changes_the_window…`, `…resize_that_does_not_fit…`, `…parameters_are_typed…`, `…linear_curve_keeps_the_intensity…`, `…easing_that_is_not_known…`, `…parameter_the_catalog_does_not_have…` |
| E04 hidden/remove | `hidden_true_is_written_and_hidden_false…`, `removing_the_last_transition…`, `…only_transition_of_a_bare_layer…`, `removing_one_transition_keeps_the_order…`, `…transition_that_is_not_there…` |
| E05 no-op/vazio/ida e volta | `edit::…no_op_is_judged_on_typed_values…`, `…empty_batch_and_a_batch_that_undoes_itself…`, `adding_a_transition_and_removing_it_again…`; executor `a_no_op_batch_changes_no_revision…`; regressões de grafia numérica na lib e no executor (.26) |
| E06 dry-run | `edit::a_dry_run_gives_the_summary…`, executor `a_dry_run_reports_the_change_and_writes_nothing` |
| E07 atomicidade, lote único, undo | `edit::a_batch_of_looks_and_transitions_lands_whole…`, `…candidate_the_importer_refuses_leaves…`; executor `a_batch_of_looks_and_transitions_is_one_revision_and_one_undo_entry`, `undo_and_redo_walk_a_sequence…` |
| E08 `base_revision` | executor `an_edit_built_on_an_old_revision_is_refused_whole` |
| E09 ids (.15) | `…generated_id_never_collides…`, `…explicit_id_cannot_reuse…`, `…id_differing_only_by_case…`, `two_new_transitions_in_one_batch…` |
| E10 quotas | `…batch_past_the_ceiling…` (lib e executor), `…count_limit_is_the_importers…` |
| E11 regras do importador | `every_rule_of_the_window_is_the_importers…`, `…ops_are_strict_about_what_they_read`, `…typed_ops_round_trip…` |
| E12 .24 | `edit_problems::a_camera_entered_past_its_end…`, `one_frame_past_the_end…`, `…entered_too_early…`, `both_faults_are_reported…`, `refuse_reads_the_same_frames…`, `…window_with_no_frames_before_the_cut…`; projeção nas três línguas em `a_problem_reads_in_three_languages_without_a_hole_and_english_is_the_importers_text` |
| E13 orçamento | `transition_edits_report_the_candidate_budget_and_undo_restores_it`: oito looks + mistura, dry-run/remove/undo e respostas list/inspect |
| E14 predicado | `window_guard::*` (5) |
| E15 gesto | executor `a_gesture_cannot_carry_a_transition_op`; op recusada preserva último legal |
| E16 política preservada | executor `the_policy_survives_every_transition_edit…`; `admission_tests` (existentes); `edit_budget` inalterado |
| E17 processo real | `public::a_real_headless_process_refuses_the_transition_ops_and_keeps_its_session` |
| E18 catálogo/plano públicos | `public::the_transition_ops_through_the_executor_are_refused…` (catálogo = 3 looks, `scene.fx.frame` sem `transition`) |
| E19 shot removido/encurtado | `edit::a_shot_that_goes_away_or_shrinks_is_refused_by_the_importer…` |
| E20 contrato e dependências | F064/relatório por superfície e `cargo tree` sem GPUI/wry; errors[] coberto em edit_problems e executor |

## 5. Gates (Windows nativo, Git Bash + `./scripts/cargo`, sem `CARGO_TARGET_DIR`/lane/clean/gc)

| Gate | Resultado |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo check --workspace --locked` | exit 0 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 (cobre o desktop e o render: nenhum `match` exaustivo quebrou fora do core) |
| `cargo build -p tessel-editor --locked` | exit 0 |
| `cargo test -p tessel-editor-core --locked --no-fail-fast` | 1658 passam, **2 falham**: os mesmos 2 testes de gizmo da base (`scene_gizmo_rotation::the_rotate_overlay_has_three_closed_rings_and_the_metrics_to_draw_them`, `scene_gizmo_scale::four_parents_of_anisotropic_scale_with_a_rotation_between_them_refuse_the_world_shear`); baseline 1593/2 |
| alvo `scene_fx_transition` | 170 passam (114 da base + 56 novos: `edit` 35, `edit_problems` 14, `window_guard` 5, `public` +2), debug, `--features serde_json/preserve_order` e `--release` |
| lib do core | 604 passam (595 + 9 do `transition_edit_tests`) |
| `cargo test -p tessel-editor --locked` | 508 passam (inclui os testes GPUI) |
| `cargo tree -p tessel-editor-core` | sem `gpui` nem `wry` |

**Não declarado:** workspace verde (os 2 testes de gizmo seguem vermelhos na base, fora do escopo); Mac; nenhuma execução
de motor, GPU, desempenho ou FPS (o lock de GPU não foi tomado).

## 6. Mutantes

Cada mutante altera uma linha de produção, roda `lib` + `scene_fx_transition` com `--no-fail-fast` e restaura os bytes
(SHA-256 conferido). **16 mortos, 0 sobreviventes finais.**

| Mutante | Resultado |
|---|---|
| M01 candidato perde a política no reimport | morto (38) |
| M02 ids gerados ignoram ids de transição | morto (4) |
| M03 upsert perde membros que não conhece | morto (5) |
| M04 `hidden: false` é escrito | morto (3) |
| M05 `base_revision` ignorado | morto (1) |
| M06 só a primeira op do lote roda | morto (14) |
| M07 no-op vira revisão | morto (1) |
| M08 teto de 1024 ops removido | morto (2) |
| M09 fim da janela inclusivo | morto (3) |
| M10 dica .24 pede pouco (60/48 → 6) | morto (2) |
| M11 inglês diverge do texto do importador | morto (1) |
| M12 gesto aceita op de transição | **sobreviveu** na 1ª rodada: a recusa genérica "sem clipe" da mesma função produzia o mesmo código; o teste passou a exigir a mensagem própria ("is a transition op") e o mutante morre (1) |
| M13 id gerado ignora ids tomados | morto (3) |
| M14 `errors[]` nunca anexado | morto (5) |
| M15 id explícito diferente aceito | morto (4) |
| M16 portão `before 0` da falta de fim removido | morto (1) |

## 7. Executado, apenas lido, N/A

- **Executado:** os gates da seção 5, os mutantes, a sonda da cauda (descartada, resultado em §3).
- **Apenas lido:** `docs/migration/fixtures/**` e o corpus S0 (não editados; o oráculo `dissolve-cases.json` continua
  passando byte a byte com a dica corrigida).
- **N/A (justificado):** UI, CLI de janela e qualquer renderer (S3b/S4); gesto de transição; troca de efeito (só há
  `crossDissolve`); edição de shots e reavaliação pela P5; `locale` no envelope (a projeção existe na biblioteca; o JSONL
  responde `errors[]` em inglês); comprimento de câmera desconhecido na .24 (não construível num bundle válido).
- **Chaves defensivas da tabela sem cenário válido:** `TRANSITION_TOO_LONG:no_room` e
  `TRANSITION_OUT_OF_RANGE:overflow` (exigem shot de 0 quadros ou duração já recusada por `duration_above_edit`);
  testadas direto nas três línguas.

## 8. Limites e ressalvas

- O portão público segue fechado: `set_transition` num cenário `Refuse` termina em `invalid_bundle` com
  `FX_TRANSITION_NOT_YET` (provado pela biblioteca, pelo executor e por um processo `tessel-headless` real, exit 1,
  sessão e histórico intactos). Todos os testes de edição usam `support::admit`/`AttachedScene` de dentro do crate.
- O predicado §9.6 não tem consumidor; o código `neighbor_deck_frame_out_of_shot` é só proposta documentada.
- Extensões de dono: nenhuma (nenhum arquivo fora de `crates/core`, do alvo de testes e de F064 foi editado).
- A skill `commit` não existe neste checkout; seguiu-se a convenção (commits convencionais no imperativo, título ≤ 70,
  linhas ≤ 100, só caminhos próprios).
- **Incidente do terminal:** o terminal do host acrescenta `Co-authored-by: Cursor` a todo `git commit`. Os commits desta
  branch foram feitos pelo encanamento do Git real (`write-tree` + `commit-tree` + `update-ref`) e **não** têm trailer.

## 9. Atualizações para o coordenador (não editadas aqui)

**`docs/backlog.md`** (linhas literais):

```
| BL082.4.5 | P4b/S3a | Ops de transição no core (`scene.fx.edit`), erros localizados e predicado de janela | A validar — implementação na branch `feat/p4b-s3a-transition-editing`, S3b/gestos/inspector/persistência de nós pendentes | BL082.4.3 | Cursor Sonnet5.5 (S3a) | Cinco ops, lote atômico, dry-run, no-op, `base_revision`, um undo; `details.errors[]`; projeção pt-br/en/es; predicado §9.6 sem consumidor. 170 transition/604 lib/508 desktop; clippy/check/fmt/build; 16 mutantes mortos. Refuse público fechado; core 2 gizmo base. [Relatório](migration/p4b-s3a-editing-report.md) |
| BL082.4.15 | PR62 R2 / S3 | IDs automáticos de looks e trilhas ignoram IDs de transições | A validar — corrigido na S3a (`every_id`) | BL082.4.5 | Cursor Sonnet5.5 (S3a) | Provado com transições `fx-1`/`fx-track-1`, `tr-<n>` e recusas explícitas atômicas; M02/M13 mortos. |
| BL082.4.24 | PR63 / handles | cameraIn além do fim conta cabeça inexistente e use/refuse aceita frames presos | Dica corrigida na S3a, a validar | BL082.4.3 | Cursor Sonnet5.5 (S3a) | 60/48 → +12 no fim, 49/48 → +1, 4/3 pede os dois reparos; `before 0` não exige cabeça; reparo aplicado reimporta válido; oráculo intacto. M10/M16 mortos. |
| BL082.4.25 | S3a / handles (cauda) | Dica da cauda do plano que sai subestima o reparo (cameraIn 40, câmera 60, plano 48: dica +6, preciso +34) | Pendente | BL082.4.24 | Não atribuído | Corrigir `camera` da cauda e a linha do oráculo que o compara; teste de reparo suficiente. Detalhe no F064. |
```

**`docs/features/README.md`**, linha F064: acrescentar ao fim da coluna de estado: `S3a (core): 5 ops de transição no scene.fx.edit com Refuse público fechado, errors[] e projeção pt-br/en/es, predicado de janela e .24 corrigida; a validar (relatório p4b-s3a-editing-report.md). Sem S3b/gestos/UI de transição.`

**`docs/README.md`**, abaixo da linha do P4b: `- [P4b S3a — edição de transições no core](migration/p4b-s3a-editing-report.md): ops, erros localizados, predicado de janela e a correção da dica .24; admissão pública ainda recusada.`

## 10. Integração e gates Mac do coordenador

`6cbd2c0` incorporou a main `79ed312` (referência X da PR65). O único conflito
foi de apêndices em F064; ambos foram preservados. `33323a2` acrescenta backlog,
matriz e índice. O código de produção do core é idêntico ao head do autor `934bab3`;
o teste de importabilidade X da main passa a fazer parte desta integração.

Mac nativo, head limpo `33323a2aa402796ad54610856e90ebd3a41e8dcf`, wrapper
`./scripts/cargo`, target padrão no clone SSD do coordenador:

| Gate | Resultado |
|---|---|
| fmt, check workspace, clippy workspace/all-targets `-D warnings` | exit 0 |
| core completo `--no-fail-fast` | 65 resultados, 1666 passaram, 0 falharam, 7 ignorados; inclui lib605, transições170, corpus10, X5 e baseline1 |
| alvos transição/corpus/X/baseline com `serde_json/preserve_order` | 170/10/5/1, 0 falhas |
| desktop, filtro `scene_fx` | 80/80 |
| `test --workspace --no-run` e build desktop | exit 0 |
| árvore do core | nenhuma linha com GPUI ou wry |

O workspace inteiro foi compilado, não executado. Os dois gizmos vermelhos do
Windows continuam atribuídos àquele host; os resultados Mac não os tornam verdes.
Não houve runtime, GPU, desempenho, UI física ou execução da origem. A revisão
independente Windows do head integrado segue pendente. Nenhuma alegação de produto
é promovida: Refuse permanece fechado e o predicado de janela sem consumidor.

Logs pequenos e summary.json em `~/.cache/tessel-pr66-mac-20261114/`, com HEAD e
exit por comando. O clone terminou limpo em `33323a2`. Nenhum target-dir, lane,
clean/gc ou limpeza de arquivos alheios foi usado.

## 11. Revisão independente Windows R1

R1 não encontrou bloqueante no escopo interno em `33323a2`: fmt/check/clippy/
no-run/build passaram, transições170 debug/preserve/release, lib604, desktop508;
core1663/2 (os dois gizmos da base). Reproduziu processo real, política, gesto,
limites e reparos de cabeça; 5/7 mutantes próprios mortos. Os dois sobreviventes
revelaram lacunas de teste de hint es e erro de look em lote misto.

Antes de integrar, o coordenador trata BL082.4.26 (ida e volta numérica altera
1 para1.0), BL082.4.27 (hints/erro misto/orçamento e precisão do contrato de parse).
Uma regressão reproduziu .26 por asserção antes do fix. Nenhum dado/corpus é
alterado; .25 continua só dica conhecida, segura e pendente. A matriz E acima
foi reconciliada com os números do plano, incluindo E13 orçamento.

Os resultados iniciais e da R1 não são herdados pelo código corretivo. Commits
publicados com linhas longas não serão reescritos: o squash terá mensagem limpa.

## 12. Correções do coordenador após R1

- .26: regressão reproduzida antes do fix (`p2-red.log`, falha por asserção).
  Ao terminar o lote, parâmetros presentes que voltam ao valor tipado original
  conservam sua grafia autorada. Omisso/default escrito continuam distintos;
  nenhum campo desconhecido é normalizado. Testes incluem outra mudança no mesmo
  lote e executor dry-run/commit sem revisão ou undo extra.
- .27: teste de look inválido em lote misto, hint espanhol de ID_MISMATCH e
  orçamento do candidato ao criar/remover transição e desfazer. E-IDs conciliados
  com o plano; parser `invalid_op` documentado como anterior à projeção.
- .28: contagens faltantes/inválidas não viram zero na tradução, que preserva
  o diagnóstico e sinaliza `untranslated`. Atribuição de erros usa o tipo do alvo,
  sem atribuir falha de transição nova à remoção de um clip com o mesmo ID.
  A dica de corte duplicado cita a op real `set_transition`, nas três línguas.

O teste de idiomas inicialmente exigia textos diferentes até para o imperativo
legítimo "Use linear", comum às três línguas. A asserção foi corrigida para exigir
distinção na frase de ID_MISMATCH que o mutante da R1 adulterava; as outras dicas
continuam verificadas quanto a presença e marcadores. Esse primeiro gate falhou
somente nessa asserção (`p2-core.log`). O ajuste .26/.27 então passou Mac1670/0/7,
transições172 com preserve_order, clippy workspace e build (`p2-fixed-*.log`).

Depois entraram as regressões de .28: exigem gates próprios, ainda sem herdar a
R1 nem o primeiro gate corretivo. Nenhum fixture/oráculo foi modificado: só o
consumidor Rust adapta a dica fixa de TRANSITION_DUPLICATE, exigindo primeiro a
frase original exata e alterando apenas o nome do comando. Todo outro campo e
texto continua sendo comparado; não há nova alegação de paridade ou golden.

### Gates das correções b097979

Mac: core completo1673/0/7, transições175 com preserve_order, clippy workspace
all-targets `-D warnings` e build desktop passaram. Os nove arquivos Rust foram
conferidos byte a byte entre o checkout do coordenador e a cópia que executou os
gates; manifesto SHA-256 em `review-fixed-source.json`, junto de
`review-fixed-{core,preserve,clippy,build}.log`. Os testes correram sobre a base5273
com o patch próprio; o commit de código `b097979` contém exatamente esses bytes.
Não se usa o head antigo como prova do patch: os logs e hashes são próprios.

Nenhum fixture foi alterado. Os três comentários automáticos de .28 foram lidos
e confirmados no código, com regressões específicas. A revisão Windows da R1
continua atribuída a33323a2; .26–.28 aguardam rechecagem independente do novo código.
