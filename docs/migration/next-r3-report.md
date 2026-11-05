# R3 — sessão e gestos de cena (BL066–BL069)

Frente R3 da [próxima entrega](next-delivery-ten-agents.md). Contrato da feature: [F039](../features/scene-interaction.md).
Estado de execução fica só em [backlog](../backlog.md); este relatório entrega os patches de linha ao coordenador.
Nada foi commitado; nenhum arquivo fora do ownership de R3 foi editado.

## O que mudou

| Item | Decisão | Código |
|---|---|---|
| BL066 | **Bug real, corrigido.** O ID era `gesture-<n>` com contador por `EditorState`. Criar/abrir projeto cria um `EditorState` novo (contador em 1, revisão em 0); um pedido guardado do projeto antigo com `gesture-1` achava o gesto novo e o atualizava, porque a invalidação compara `Arc` da cena e revisão **do gesto novo**. Agora o ID é opaco, `gesture-<sal>-<época>-<n>`: época = contador do processo (nunca repete entre sessões), sal = 32 bits aleatórios por processo (`RandomState`, sem dependência nova). Pedido de outra sessão → `unknown_gesture`/`no_gesture` com `details.reason = "other_session"` e nada é tocado. | `scene_interaction/gesture.rs` (`SessionTag`, `GestureSlot::default/missing`) |
| BL067 | **Provado.** Update acima do orçamento: erro `scenes_too_large` preservando candidato bom, documento, histórico e revisão (teste público, bundle real de 128 MiB). Commit acima do orçamento: o gesto volta ao slot inteiro (teste unitário). O ramo do commit **não é alcançável pela API pública** (toda operação que muda o tamanho do livro sobe a revisão e invalida o gesto antes), então o teste unitário faz o livro crescer sem subir a revisão; isso fica dito no teste e na feature. Nenhuma mudança de código de produção. | `tests/scene_session.rs`, `scene_project/interaction.rs` (`mod tests`) |
| BL068 | **Implementado.** `summary()` ganha `scene_interaction` (seleção + gesto) só quando há algo; leitura não poda, não encerra gesto, não cria histórico. Lista limitada a `SUMMARY_LIMIT` (64) com `count` total e `truncated`; entradas sem correspondência saem com `stale`; gesto mostra `state: open|invalidated` e `invalidation`. | `scene_interaction/{mod,selection}.rs`, `state/mod.rs` (`summary`) |
| BL069 | **Hipótese confirmada por medição**, então implementada. A poda era preguiçosa (`scene.selection`), logo o resultado dependia de alguém consultar: selecionar, `undo` da anexação, `redo` deixava a entidade selecionada **de novo** sem consulta no meio e **fora** com consulta no meio. Agora `restore` (undo, redo e rollback de transação) poda na hora; undo/redo respondem `selection_dropped[]` só quando algo saiu; redo não restaura a seleção. O **gesto não é podado no undo**: fica invalidado de forma preguiçosa para a UI ver `invalidated` e cancelar (documentado em `gesture.rs` e na feature). | `state/history.rs` (`restore`, `note_dropped`) |

Fora de escopo e intocado: assinatura de `scene_plan`/`SceneCommand::Plan` (R1/R6), `core/ops.rs`, `headless.rs`, `document.rs` (o `Default` do slot já gera sessão nova em `from_document`/`empty`/`demo`, então nenhum wiring foi necessário).

## Reprodução antes da correção

Com o código novo dos testes e a correção desligada temporariamente (ID constante; poda no `restore` desligada), 6 dos 14 testes de sessão falharam:

- `an_id_kept_from_the_previous_project_cannot_touch_the_next_gesture` e `reopening_the_same_saved_project_starts_a_new_session`: `gesture-0-0-1 == gesture-0-0-1` (mesmo ID em dois projetos);
- `a_gesture_id_is_issued_once_across_many_sessions`: ID emitido duas vezes;
- `undo_prunes_a_selection_it_orphans_at_once…` e `the_outcome_does_not_depend_on_whether_someone_queried_the_selection_between`: seleção `[]` com consulta e `[figure]` sem consulta (o não determinismo de BL069);
- teste do executável: `not_current` no lugar de `other_session`.

As duas edições temporárias foram revertidas na mesma execução (restauradas a partir de cópias) e o diff final só tem a correção.

## APIs e seams

- `GestureSlot` não deriva mais `Default`: `GestureSlot::default()` é "sessão nova" (é o que `EditorState::{demo,empty}` e `from_document` usam). `Clone` mantém a sessão (um clone é a mesma sessão; é o que o host faz ao aplicar jobs).
- `Selection::summary_json(&SceneBook)`, `SceneInteraction::summary_json(&SceneBook, revision) -> Option<Value>`, `scene_interaction::SUMMARY_LIMIT`.
- `EditorState::restore` devolve `Vec<Dropped>`; `history.undo|redo` acrescentam `selection_dropped` (mesmo formato de `scene.detach`).
- Erros: `unknown_gesture`/`no_gesture` ganharam `details {gesture, reason: other_session|not_current, current}`. Aditivo.
- **Quebra consciente**: o formato `gesture-1` deixou de existir. Nenhum script do repositório o grava (conferido em `scripts/`); `tests/scene_interaction.rs` foi ajustado (ver abaixo). Consumidores devem ler o ID da resposta de `begin`.
- `tests/scene_interaction.rs` (arquivo da própria F039, fora da lista literal do plano): `observable()` passa a ignorar só o bloco `scene_interaction` do summary (selecionar/`begin` agora o alteram por contrato) e o teste da sessão JSONL fixa `$gesture` e alimenta `run_session` linha a linha; o teste de poda reflete a poda imediata (`changed: false` na leitura posterior). Nenhuma asserção foi removida.

## Testes

Executados (via `cargo-serial.py`, build debug):

| Comando | Resultado |
|---|---|
| `test -p tessel-editor-core --locked` (pacote inteiro, com WIP de R1/R6 já compilando) | 206 unit + todos os testes de integração passaram, incl. `scene_session` 15/15, `scene_interaction` 21/21, `scene_project` 19/19, `scene_flow` 8/8, `headless` 2/2 |
| `test … --lib --test scene_session --test scene_interaction` (após `rustfmt`) | 206 + 15 + 21 passaram |
| `clippy -p tessel-editor-core --lib --bins --test scene_session --test scene_interaction --test headless -- -D warnings` | limpo |
| `cargo tree -p tessel-editor-core` | 0 ocorrências de gpui/wry |
| exemplo Python do doc contra `target/debug/tessel-headless` | executado, `Edit scene` no undo |

`scene_session.rs`: lib (`headless::execute`/`EditorState::execute`) e **executável público `tessel-headless`** como processo filho, uma linha por vez (o ID só existe após `begin`), cobrindo projeto trocado na mesma sessão, undo/redo, no-op, cancel, erro sem mutação, summary e poda de seleção. O teste do orçamento aloca ~128 MiB (~3 s em debug).

Bloqueio observado e resolvido sem ação minha: durante a primeira rodada o `core` não compilava por WIP de R6 (`directed_scene/temporal/frame.rs`, E0477); voltou a compilar sozinho. `clippy --all-targets` no core falha em `tests/scene_plan_performance_{support/mod,bench}.rs` (R6: `is_multiple_of`, unit value); fora do meu ownership, não corrigido. `cargo fmt --check` aponta diffs em arquivos de R1/R6, não meus (os meus foram formatados com `rustfmt --edition 2024` em arquivos individuais).

Não executados:

- **Desktop** (`crates/desktop`, testes GPUI de `scene_*`/`viewport_*`): não compilados nem rodados; as mudanças são aditivas na API usada por ele (`gesture()`, `selection()`), mas o gate de workspace é do coordenador. `scene_gesture.rs`/`scene_model.rs` (R9) não foram lidos para mudanças.
- **Socket da janela / `ui status`**: o summary é o mesmo `EditorState::summary()`, mas não foi exercitado. Nenhuma instância de UI foi aberta, nenhum GPU/Chrome usado.
- Orçamento de bytes por JSONL/executável: só pela lib (um request de ~128 MiB não é um teste útil).
- Workspace inteiro (`check`/`clippy`/`test --workspace`), `pnpm`-equivalentes: não, gate global.

## Limitações

- A invalidação do gesto continua conservadora e preguiçosa (decisão BL069, ver feature).
- Sal de 32 bits por processo: a distinção entre **processos** é probabilística (≈ 2⁻³²); entre sessões do mesmo processo é garantida pela época.
- O ramo "commit estoura o orçamento" é defesa em profundidade provada só por teste unitário que faz o livro crescer sem subir a revisão (white-box).
- `SUMMARY_LIMIT = 64` corta a lista no summary; `scene.selection` devolve tudo.

## Patches para o coordenador

Backlog (`docs/backlog.md`, linhas existentes):

```text
BL066 | Concluído (lib + executável JSONL; socket/UI não reexecutados) | R3 | IDs de gesto opacos por sessão; teste cruzando projeto novo e projeto reaberto; `details.reason=other_session` | tests/scene_session.rs, next-r3-report.md
BL067 | A validar → Concluído em lib | R3 | update recusado preserva candidato/histórico/revisão (público, 128 MiB); commit recusado devolve o gesto inteiro (unitário). Executável: não aplicável (ver relatório) | idem
BL068 | A validar (lib + executável JSONL `status`; `ui status` não reexecutado) | R3 | bloco `scene_interaction` no summary; limite 64; leitura sem efeito; gesto invalidado refletido
BL069 | Concluído em lib e executável | R3 | hipótese confirmada (resultado dependia de consulta intermediária); poda imediata em `restore`, redo não restaura, gesto fica preguiçoso por decisão documentada
```

Se o coordenador preferir manter "A validar" até a prova de janela: registrar uma pendência única reutilizando **BL014** (inspector) para "conferir `scene_interaction` no `ui status` e o ID opaco no adaptador" em vez de criar ID novo.

Matriz (`docs/features/README.md`, linha F039): adicionar à evidência "R3: `scene_session.rs` (15) lib + executável; IDs opacos por sessão, summary com seleção/gesto, poda imediata no undo; socket/UI não reexecutados" e manter UI como **Parcial**.

Arquivos que R3 alterou: `crates/core/src/scene_interaction/{gesture,mod,selection}.rs`, `crates/core/src/scene_project/interaction.rs`, `crates/core/src/state/{mod,history}.rs`, `crates/core/tests/{scene_session,scene_interaction}.rs`, `docs/features/scene-interaction.md` e este relatório. Temporários `te-r3-*`: os diretórios de teste são removidos por `Drop`; nada restou em `/tmp`.
