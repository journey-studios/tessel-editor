# Relatório da frente J — seleção, árvore, avaliação de frame e gestos de cena dirigida (BL012 / BL013 / BL020 / BB07 inicial / F039)

2026-11-05. Rodada H/I/J/K ([plano](next-round-plan.md)). Checkout compartilhado `tessel-editor`.
Origem e `tesselapp` não foram tocados. Contrato da feature:
[`docs/features/scene-interaction.md`](../features/scene-interaction.md) (F039).

**Esta frente não compilou nada.** Não rodou Cargo, rustc nem clippy; só `rustfmt` nos próprios
arquivos (confirma que a sintaxe parseia; `--check` passa). **Nenhum teste foi executado; Rust não é
declarado verde.** Não fez `git add`/commit/push/PR, não criou branch/worktree, não tocou `core/src/lib.rs`,
`media/src/lib.rs`, `Cargo.toml/lock`, `document.rs`, `state/history.rs` (reservado à frente L),
`docs/backlog.md`, matriz nem índices. Nenhum subagente.

## PATCHES OBRIGATÓRIOS DO COORDENADOR (sem eles o core não compila)

1. `crates/core/src/lib.rs`: exportar o módulo.

   ```rust
   pub mod scene_interaction;
   ```

2. `crates/core/src/document.rs`, no literal `EditorState { .. }` de `from_document` (logo depois de
   `scenes,`, ~linha 417): o campo novo de `EditorState` é obrigatório.

   ```rust
   scene_interaction: Default::default(),
   ```

   (Os dois outros literais, `demo()` e `empty()`, já foram atualizados em `state/mod.rs`. O campo da
   frente L, `property_gesture`, é dela.)

## Arquivos (ownership exclusivo)

| Arquivo | Conteúdo |
|---|---|
| `crates/core/src/scene_interaction/mod.rs` | docs do módulo, `SceneInteraction { selection, gesture }`, reexports |
| `.../address.rs` | gramática de endereço de entidade (prefixo do caminho de propriedade da F034), `parse_entity`, `entity_path`, `exists`/`ensure_exists`, `entity_of_property` |
| `.../command.rs` | `FrameAt`, `SelectMode`, `GestureCommand` |
| `.../selection.rs` | `Selection` (atômica, teto 4096), `Selected`, `Dropped`, poda |
| `.../gesture.rs` | `Gesture` (base `Arc`, revisão, candidato), `GestureSlot`, `Invalidation` |
| `.../tree.rs` | `tree_json` (profundidade ≤ 64), `editable` por entidade |
| `.../frame.rs` | `frame_json` = `Bundle::evaluate_frame(..).to_json()` + `path` por nó |
| `crates/core/src/scene_project/interaction.rs` | cola com `EditorState`: tree/evaluate/select/selection/gesture |
| `crates/core/src/scene_project/{command,query,run,mod,scope}.rs` | variantes `Tree`, `Evaluate`, `Select`, `Selection`, `Gesture`; `history_label` só no `Commit`; capacidades; roteamento; poda no `detach` |
| `crates/core/src/ops.rs` | parser dos 9 `op` novos (único parser, compartilhado por headless e `ui op`) |
| `crates/core/src/state/mod.rs` | **apenas** o campo `scene_interaction`, seu doc e os 2 inicializadores |
| `crates/core/tests/scene_interaction.rs` | testes públicos (escritos, não executados) |
| `docs/features/scene-interaction.md`, este relatório | docs |

## Implementação

- **Sem segundo executor nem segunda pilha.** O commit chama `edit_scene`, a função de `scene.edit`:
  catálogo, importador, gates estrutural/temporal e orçamento de bytes. `EditorState::execute` grava
  **uma** entrada ("Edit scene") quando a resposta tem `changed != false`. `begin/update/cancel/status`,
  `tree`, `evaluate`, `select`, `selection` não têm `history_label`: não criam undo, não sobem
  `project.revision`, não tocam o `SceneBook`.
- **Estado efêmero fora do documento.** `SceneInteraction` é um campo de `EditorState`, fora de
  `Snapshot` e do documento: não é salvo, não é desfeito, é zerado ao abrir/criar projeto.
- **Invalidação sem gancho de undo.** Como `history.rs` é da frente L, o gesto guarda o `Arc` da cena
  e a `project.revision` do begin e se compara na hora de usar: `gesture_invalidated` com
  `details.reason` ∈ `scene_detached | scene_changed | project_revision`. Cobre edit, undo, redo,
  detach/re-attach e edição alheia. Nunca aplica candidato velho. `cancel`/`status` sempre funcionam;
  novo `begin` substitui gesto invalidado (`replaced_stale`) e recusa gesto vivo (`gesture_active`).
- **Seleção por endereço** (`scene/<def>/node/<n>`, `camera/<c>`, `shot/<s>` ...), validada contra a
  cena; atômica; podada no `detach` (resposta com `selection_dropped`) e na leitura (`scene.selection`,
  com `dropped[]` e motivo). Nenhum comando de edição lê a seleção.
- **`scene.evaluate`** devolve exatamente `FrameEvaluation::to_json` (com `path` por nó), por frame ou
  tempo exato; com `gesture`, avalia o candidato. **`scene.tree`** lista entidades e os caminhos
  editáveis do catálogo; o catálogo continua com **8** propriedades.
- Erros tipados novos: `unknown_entity`, `invalid_entity_path`, `selection_limit`, `no_gesture`,
  `unknown_gesture`, `gesture_active`, `gesture_invalidated`, `gesture_target_mismatch`,
  `gesture_scene_mismatch`.

## Testes: escritos × executados

| | |
|---|---|
| **Escritos** | `crates/core/tests/scene_interaction.rs` (árvore e catálogo de 8; seleção por endereço, atômica, sem undo, não é pré-condição; poda por detach/undo; estado efêmero não salvo; `scene.evaluate` igual a `Bundle::evaluate_frame` nos fixtures `MINIMAL`/`RIG`/`OFFSETS` e igual a `scene.camera`; nós animados e tempo exato; erros tipados; gesto: preview não altera nada, commit = uma entrada de undo, equivalência com `scene.edit`, cancel, no-op, update rejeitado preserva o candidato, alvos; invalidação por edit/undo/redo/edição alheia/detach; ciclo pela API tipada; sessão JSONL de 19 linhas) + unitários em `scene_interaction/{address,selection,tree}.rs` |
| **Executados** | **Nenhum.** Sem Cargo nesta rodada |
| **Revisão manual** | Passagem por tipos, empréstimos e fixtures (campos `localScale`/`localPosition` existem no figure; os 3 bundles são casos válidos do corpus); pode restar erro de compilação |

Comandos do gate (coordenador):

```bash
./scripts/cargo test -p tessel-editor-core --lib scene_interaction
./scripts/cargo test -p tessel-editor-core --test scene_interaction --test scene_project --test headless
./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings
```

Expectativas de fixture que, se falharem, são do teste e não do código (verificar antes de mudar
código): `OFFSETS` frame 55 no shot-b com `instanceFrame` 15; `RIG` frame 12; o nó `arm` em
`scene-rig`; o id do primeiro gesto `gesture-1` (projeto novo); `undo_depth` contando o attach.

## Limites

- **Sem UI, sem socket, sem gesto físico de ponteiro.** Escape/soltar/handles (BL015) são só o
  equivalente semântico `cancel`/`commit`. Nenhum `tree.rs` (árvore de automação) alterado.
- Invalidação **conservadora**: qualquer edição ou undo do projeto invalida o gesto.
- Poda da seleção **preguiçosa** na leitura (sem gancho no undo).
- IDs de gesto reiniciam ao abrir/criar projeto: um cliente com `gesture-1` antigo poderia colidir.
- Rejeição do commit por orçamento de bytes sem teste (inalcançável com os fixtures).
- `summary()` não expõe seleção nem gesto.
- A avaliação de nós é a função da biblioteca (F031/F033), não equivalência com a origem; só a câmera
  tem oráculo.
- `docs/features/scene-project.md` ainda diz "só câmera" para consultas (arquivo da F036, não meu).

## Contratos para integrar

- Os dois tipos de gesto são distintos: `Command::PropertyGesture` (frente L, propriedades do
  projeto) e `SceneCommand::Gesture` (esta, cena dirigida). Nenhum depende do outro; a integração
  comum (por exemplo um `Escape` que cancela o que estiver aberto) é do coordenador.
- Proposta para o dono de `history.rs` (L), **opcional**: um gancho `restore()` que invalide
  `SceneInteraction` de forma ansiosa tornaria a poda/invalidação imediata e dispensaria a
  comparação por `Arc`/revisão. Não é necessário para o contrato atual.
- Chamadores de `scene.*` devem tratar `gesture_invalidated` como "recomece com `begin`".

## Patches propostos: matriz e backlog (não aplicados)

`docs/features/README.md` — nova linha:

```text
| F039 | [Interação com cenas dirigidas: seleção, árvore, frame e gestos](scene-interaction.md) | P | P | I | I | `scene.tree|evaluate|select|selection|gesture.*` |
```

`docs/backlog.md` — mover para **A validar** (implementado, não executado) e acrescentar linhas comuns:

```text
| BL012 | BB07 | Seleção por entidade, árvore de cena e tool state | A validar | BB02, BL004 | J | Lib/JSONL escritos (F039), não executados; UI pendente (BL014) |
| BL013 | BB07 | Gestos com preview/commit/cancel e histórico agrupado | A validar | BL012 | J | Ciclo semântico em lib/JSONL (commit = 1 undo); Escape/ponteiro dependem de UI (BL015) |
| BL020 | Adapters | Avaliação completa de nós por frame no acesso programático de cena | A validar | BB03 | J | `scene.evaluate` = `Bundle::evaluate_frame`; executar o teste no gate |
| BL021 | Adapters | Provar `scene.*` pelo socket da janela e depois pela UI visual | A validar | BL012/BL014 | — | Inclui os 9 comandos novos; headless escrito, não executado |
```

Novas pendências (linhas para o backlog comum, sem fila paralela):

```text
| BL0NN | BB07 | Nós de automação (`state/tree.rs`) e controles da árvore/seleção de cena | Pendente | BL012 | — | UI/CLI da janela equivalem aos comandos F039 |
| BL0NN | BB07 | Prova de `scene.tree|evaluate|select|gesture.*` pelo socket/`ui op` | Pendente | BL021 | — | Evidência separada da headless |
| BL0NN | BB07 | Executar `scene_interaction` + `tessel-headless` com o exemplo JSONL da F039 | Pendente | gate | — | Depois do gate do coordenador |
| BL0NN | BB07 | Gancho de undo para invalidação/poda ansiosas e IDs de gesto únicos entre projetos | Pendente | L | — | Opcional; ver "Contratos" |
| BL0NN | BB07 | Teste de commit rejeitado por orçamento e exposição de seleção/gesto em `summary()` | Pendente | — | — | — |
| BL0NN | BB07 | Atualizar `docs/features/scene-project.md` ("câmera apenas") para apontar F039 | Pendente | — | — | Arquivo da F036 |
```

## Recebimento pelo coordenador

Export scene_interaction e inicialização no loader adicionados. F039 fica I em lib/headless; BL012/BL013/BL020 A validar. Socket continua A validar/sem prova e UI P. BL066–BL069 registram os limites novos sem duplicar BL012/BL014/BL021; documentação F036 reconciliada. Cargo aguarda os módulos compartilhados de L/I/K terminarem.
