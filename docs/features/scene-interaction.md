# F039 — Interação com cenas dirigidas: seleção, árvore, avaliação de frame e gestos

## Status

| Superfície | Estado | Evidência |
|---|---|---|
| Lib (`tessel_editor_core::scene_interaction`, `SceneCommand::{Tree,Evaluate,Select,Selection,Gesture}`) | **V (lib)** — 21 testes públicos e testes unitários do módulo passaram no gate direcionado do coordenador | `crates/core/tests/scene_interaction.rs` (21/21 executados) e testes unitários em `scene_interaction/{address,selection,tree}.rs` |
| CLI headless (JSON Lines) | **V (headless)** — mesmo parser/executor (`ops::parse_op`, `headless::execute`); sessão JSONL de 19 linhas executada em processo; o executável `tessel-headless` passou uma sessão real de 13 requests | `a_json_lines_session_runs_the_whole_interaction_flow_in_process` |
| Socket / `ui op` da janela | **V no corpus** — comandos reais na janela e seleção por nó semântico | `scripts/directed-scene-proof/proof.py`, F039-UI |
| UI (árvore, inspector e Escape) | **Parcial** — controles e 16 testes de input GPUI; aparência, ponteiro físico e handles não provados | [Inspector](scene-inspector.md) |

Esta feature é **semântica**: não existe gesto físico de ponteiro aqui. O "gesto" é um ciclo de
comandos (begin → update* → commit | cancel) que a UI futura deve apenas chamar.

## Objetivo e limites

Dar a quem automatiza e à futura UI, sobre uma cena dirigida já anexada (F036):

1. **Seleção por ID** de entidades (nó, definição, instância, track de performance, clipe de câmera, shot, asset, pass);
2. uma **árvore consultável** das entidades, com os caminhos editáveis de cada uma;
3. **avaliação de frame completa** (câmera, estado de cada nó e o que não é avaliado);
4. um **ciclo de gesto** com candidato de preview separado do documento, que no commit vira
   **um lote** na pilha de undo existente.

Não acrescenta regra a `directed_scene` nem a `scene_edit`: o catálogo continua com as **8
propriedades**, o lote continua passando por importador + gates estrutural e temporal, e o histórico
continua sendo o snapshot de `EditorState`. Fora, de propósito: render/preview visual, handles e
hit-testing (BB08), inspector (BL014), a árvore de automação da UI (`state/tree.rs`), sockets do
desktop, composição de dois tipos de gesto (esta, de cena, e a de propriedades da frente L, que o
coordenador integra).

## Contrato do core / biblioteca

```rust
use tessel_editor_core::scene_interaction::GestureCommand;
use tessel_editor_core::scene_project::SceneCommand;
use tessel_editor_core::{Command, EditorState};

// `state` já tem a cena "street" anexada (F036).
let begun = state.execute(Command::Scene(SceneCommand::Gesture(GestureCommand::Begin {
    scene: "street".into(),
    targets: vec!["scene/scene-a/node/figure".into()],
    label: None,
})))?;
let gesture = begun["gesture"].as_str().unwrap().to_string();   // "gesture-1"

state.execute(Command::Scene(SceneCommand::Gesture(GestureCommand::Update {
    gesture: gesture.clone(),
    ops, // Vec<scene_edit::EditOp>; repetido a cada tick, cada update parte da base do begin
})))?;                                    // o documento e o histórico NÃO mudam

state.execute(Command::Scene(SceneCommand::Gesture(GestureCommand::Commit { gesture })))?;
// 1 entrada de histórico: "Edit scene"; `state.undo()` desfaz o gesto inteiro.
```

`state.scene_interaction: SceneInteraction` guarda o estado **efêmero** (`selection()`, `gesture()`).
Ele **não** está em `Snapshot` nem no documento: não é desfeito, não é salvo, é zerado ao abrir/criar
projeto.

### Endereço de entidade

O endereço é o **prefixo** do caminho de propriedade da F034 (`scene/<def>/node/<node>/<prop>`,
`camera/<clip>/start/<prop>`), com segmentos codificados como `encodeURIComponent`:

| Entidade | Endereço |
|---|---|
| nó | `scene/<definição>/node/<nó>` |
| definição de cena | `scene/<definição>` |
| instância | `instance/<id>` |
| track de performance | `track/<id>` |
| clipe de câmera | `camera/<clipe>` |
| shot | `shot/<id>` |
| asset | `asset/<id>` |
| pass | `pass/<id>` |

Endereço malformado → `invalid_entity_path`; bem formado mas ausente na cena → `unknown_entity`.
O `scene` (ID de anexação) acompanha a seleção; o endereço não o repete.

### Comandos (`op` JSON ⇄ `SceneCommand`)

| `op` | Campos | Efeito | Histórico |
|---|---|---|---|
| `scene.tree` | `scene`, `root?` (endereço), `depth?` (≥ 1, teto 64) | árvore: cada entrada tem `path`, `kind`, `id`, `scope?`, `selected`, `children_count`, `children` ou `truncated`, `editable[]` (caminhos do catálogo) e detalhes do tipo | — |
| `scene.evaluate` | `scene`, **um de** `frame` · `time` (`{"num","den"}`), `gesture?` | `Bundle::evaluate_frame(..).to_json()` (`editFrame`, `instanceFrame`, `cameraClipFrame`, `camera`, `nodes[]` com `path`, `unevaluated`); com `gesture` avalia o **candidato** (`source:"gesture"`, `has_candidate`) | — |
| `scene.select` | `mode?` (`replace` padrão · `add` · `remove` · `toggle` · `clear`), `scene` (exceto `clear`), `entities[]` | atômico; até 4096 entidades; `changed` | — |
| `scene.selection` | — | seleção atual; remove o que a cena não tem mais e informa em `dropped[]` (`scene_detached`, `entity_missing`) | — |
| `scene.gesture.begin` | `scene`, `targets[]?`, `label?` | abre um gesto (`gesture-<n>`, nunca reutilizado na sessão) | — |
| `scene.gesture.update` | `gesture`, `ops[]` (lote da F034; `[]` volta ao início) | calcula o candidato a partir da **base do begin**; devolve `changes`, `would_change`, `changed_ops` | — |
| `scene.gesture.commit` | `gesture` | aplica os `ops` do último update bom como **um** `scene.edit` | "Edit scene" (uma entrada, só se mudou) |
| `scene.gesture.cancel` | `gesture` | descarta o candidato | — |
| `scene.gesture.status` | — | gesto aberto/invalidado ou `active:false` | — |

As respostas trazem `changed` (o **estado de sessão** mudou) e `document_changed` (o projeto mudou;
só o commit pode torná-lo verdadeiro). `scene.capabilities` lista os novos comandos.

### Garantias

- **Preview separado.** O candidato vive no gesto; `scene.evaluate` com `gesture` o lê; o documento, o
  `SceneBook`, o histórico e `project.revision` ficam idênticos até o commit.
- **Commit = um lote.** Passa por `edit_scene`, a mesma função de `scene.edit`: catálogo, importador,
  gates, orçamento de bytes. Equivale a enviar o mesmo lote como `scene.edit` (teste de equivalência).
- **Cancelar e rejeitar não deixam rastro.** Cancel descarta; um update rejeitado preserva o último
  candidato bom; um commit rejeitado devolve o gesto ao slot.
- **Invalidação tipada, nunca aplicar candidato velho.** O gesto lembra o `Arc` da cena e a
  `project.revision` do begin. Se a cena foi editada, desfeita/refeita, substituída ou removida, ou se
  qualquer outra edição subiu a revisão, `update`/`commit`/`evaluate` falham com `gesture_invalidated`
  e `details.reason` ∈ `scene_detached | scene_changed | project_revision`. `cancel` e `status` sempre
  funcionam; um novo `begin` substitui um gesto invalidado (`replaced_stale`) e recusa um vivo
  (`gesture_active`).
- **Seleção nunca é pré-condição.** `scene.edit`/dry-run/commit não leem a seleção.
- **Alvos validados.** `targets` e `entities` são checados contra a cena; um `update` fora dos alvos
  declarados falha com `gesture_target_mismatch` (`details.index`). Sem `targets`, qualquer
  propriedade do catálogo é aceita.
- **Seleção que perde a entidade** é podada ao desanexar a cena (resposta com `selection_dropped`) e,
  de forma preguiçosa, em `scene.selection` (por exemplo depois de um undo).
- **Consultas, seleção e dry-run não criam undo** (sem `history_label`).

### Erros tipados

`unknown_entity`, `invalid_entity_path`, `selection_limit`, `no_gesture`, `unknown_gesture`,
`gesture_active`, `gesture_invalidated`, `gesture_target_mismatch`, `gesture_scene_mismatch`; mais os
já existentes: `unknown_element` (cena), `invalid_value`, `invalid_request`, `unknown_node`,
`unknown_path`, `frame_out_of_range`, `invalid_bundle`.

## CLI headless

```bash
printf '%s\n' \
  '{"op":"scene.attach","id":"street","path":"docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"}' \
  '{"op":"scene.tree","scene":"street","depth":2}' \
  '{"op":"scene.select","scene":"street","entities":["scene/scene-a/node/figure"]}' \
  '{"op":"scene.evaluate","scene":"street","frame":10}' \
  '{"op":"scene.gesture.begin","scene":"street","targets":["scene/scene-a/node/figure"]}' \
  '{"op":"scene.gesture.update","gesture":"gesture-1","ops":[{"set":"scene/scene-a/node/figure/position","value":{"x":2,"y":0,"z":0}}]}' \
  '{"op":"scene.evaluate","scene":"street","frame":10,"gesture":"gesture-1"}' \
  '{"op":"scene.gesture.commit","gesture":"gesture-1"}' \
  '{"op":"history.undo"}' \
  | ./scripts/cargo run -q -p tessel-editor-core --bin tessel-headless -- --empty
```

Mesmo envelope `{ok, version, result|error}` das demais. O coordenador executou uma sessão equivalente
no executável `tessel-headless` (ver a evidência de integração ao final).

## UI e CLI da janela

`./scripts/ui op '{"op":"scene.tree","scene":"street"}'` chega a `Op::Run(Command::Scene(..))`, que o
desktop já executa por `EditorState::execute`; nenhuma variante nova de `Op`, nenhum arquivo do
desktop alterado. **Não exercitado.** Apresentação visual **P**: sem controle (árvore, handle,
Escape), portanto sem equivalente `ui click`. Quando existir, Escape = `scene.gesture.cancel` e o
soltar do ponteiro = `scene.gesture.commit`; handle e comando devem dar o mesmo resultado (BL015).

## Evidências e status

Executados pelo coordenador (21/21): `crates/core/tests/scene_interaction.rs`
(árvore e catálogo de 8; seleção atômica, sem undo e sem ser pré-condição; poda por detach/undo; estado
não salvo; `scene.evaluate` igual a `Bundle::evaluate_frame` nos três fixtures válidos e igual a
`scene.camera`; nós animados; erros; preview/commit/cancel; um único undo; equivalência com
`scene.edit`; no-op; update rejeitado; alvos; invalidação por edit/undo/redo/edição alheia/detach;
ciclo pela API tipada; sessão JSONL) e testes unitários de endereço, seleção e árvore.

Comparação com origem: só existe para câmera (F033, oráculo); a avaliação de nós é a função da
biblioteca, não equivalência com a origem. Para o coordenador, depois do gate:

```bash
./scripts/cargo test -p tessel-editor-core --lib scene_interaction
./scripts/cargo test -p tessel-editor-core --test scene_interaction --test scene_project --test headless
./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings
```

Limitações conhecidas:

- A invalidação é **conservadora**: qualquer edição ou undo do projeto (não só da cena) invalida o gesto.
- Sem gancho em `history.rs` (reservado à frente L), a seleção é podada **na leitura**, não no undo.
- IDs de gesto reiniciam quando o projeto é aberto/criado; um cliente que guardou `gesture-1` de antes
  poderia colidir com o do novo projeto.
- Rejeição do commit por orçamento de bytes não tem teste (o orçamento é inalcançável com fixtures).
- Sem nó em `state/tree.rs`, sem prova de socket, sem UI.
- `summary()` não expõe seleção nem gesto.

Integração pelo coordenador: testes públicos 21/21, unitários no gate core (197/197 no conjunto) e clippy lib limpo. Corrigidas duas expectativas: alvo fora do gesto é recusado antes de validar nó inexistente (`gesture_target_mismatch`). O executável headless passou uma sessão real de 13 requests (attach, árvore, seleção, evaluate, begin/update/commit, get, undo/redo). A integração posterior da janela está em F039-UI; testes e CLI sem janela não provam ponteiro físico.
