# F039 — Interação com cenas dirigidas: seleção, árvore, avaliação de frame e gestos

## Status

| Superfície | Estado | Evidência |
|---|---|---|
| Lib (`tessel_editor_core::scene_interaction`, `SceneCommand::{Tree,Evaluate,Select,Selection,Gesture}`) | **V (lib)** — 21 testes públicos e testes unitários do módulo passaram no gate direcionado do coordenador | `crates/core/tests/scene_interaction.rs` (21/21 executados) e testes unitários em `scene_interaction/{address,selection,tree}.rs` |
| CLI headless (JSON Lines) | **V (headless)** — mesmo parser/executor (`ops::parse_op`, `headless::execute`); sessão JSONL de 19 linhas executada em processo; o executável `tessel-headless` passou uma sessão real de 13 requests | `a_json_lines_session_runs_the_whole_interaction_flow_in_process` |
| Socket / `ui op` da janela | **V no corpus** — comandos reais na janela e seleção por nó semântico | `scripts/directed-scene-proof/proof.py`, F039-UI |
| UI (árvore, inspector e Escape) | **Parcial** — controles e 16 testes de input GPUI; aparência, ponteiro físico e handles não provados | [Inspector](scene-inspector.md) |

**Rodada R3 (BL066–BL069):** IDs de gesto por sessão, seleção/gesto no summary, poda imediata no undo e prova do
orçamento de bytes. Implementado e testado em lib e no executável JSONL público
(`crates/core/tests/scene_session.rs`); socket/UI não foram reexecutados pela frente — ver [relatório](../migration/next-r3-report.md).

Esta feature é **semântica**: não existe gesto físico de ponteiro aqui. O "gesto" é um ciclo de
comandos (begin → update* → commit | cancel) que a UI futura deve apenas chamar.

## Objetivo e limites

Dar a quem automatiza e à futura UI, sobre uma cena dirigida já anexada (F036):

1. **Seleção por ID** de entidades (nó, definição, instância, track de performance, clipe de câmera, shot, asset, pass);
2. uma **árvore consultável** das entidades, com os caminhos editáveis de cada uma;
3. **avaliação de frame completa** (câmera, estado de cada nó e o que não é avaliado);
4. um **ciclo de gesto** com candidato de preview separado do documento, que no commit vira
   **um lote** na pilha de undo existente.

Não acrescenta regra a `directed_scene` nem a `scene_edit`: o catálogo é o de [F034](scene-edit.md) (nesta fatia eram **8
propriedades**; a ampliação S6 para 21 entradas não muda este contrato), o lote continua passando por importador + gates estrutural e temporal, e o histórico
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
let gesture = begun["gesture"].as_str().unwrap().to_string();   // opaco: `gesture-<sessão>-<n>`

state.execute(Command::Scene(SceneCommand::Gesture(GestureCommand::Update {
    gesture: gesture.clone(),
    ops, // Vec<scene_edit::EditOp>; repetido a cada tick, cada update parte da base do begin
})))?;                                    // o documento e o histórico NÃO mudam

state.execute(Command::Scene(SceneCommand::Gesture(GestureCommand::Commit { gesture })))?;
// 1 entrada de histórico: "Edit scene"; `state.undo()` desfaz o gesto inteiro.
```

`state.scene_interaction: SceneInteraction` guarda o estado **efêmero** (`selection()`, `gesture()`).
Ele **não** está em `Snapshot` nem no documento: não é desfeito, não é salvo, é zerado ao abrir/criar
projeto. Fica visível no `summary()` compartilhado (`status`/`ui status`), ver "Summary" abaixo.

### IDs de gesto são opacos e pertencem à sessão (BL066)

`begin` devolve `gesture-<sal>-<época>-<n>`. O cliente **só repassa** o ID que recebeu; não o monta nem o
interpreta. Cada `EditorState` novo (projeto criado, aberto ou recuperado) é uma **sessão nova**: a
época é um contador do processo e o sal é aleatório por processo, então nenhum ID emitido por uma
sessão é emitido por outra, nem com o mesmo arquivo reaberto, a mesma cena e a mesma revisão.
Um pedido guardado de uma sessão antiga (`update`, `commit`, `cancel`, `evaluate`/`plan` com `gesture`)
falha com `unknown_gesture` (ou `no_gesture`) e `details.reason = "other_session"`, sem tocar o gesto
novo, o documento, o histórico nem a revisão. Um ID desta sessão que não é o aberto (cancelado, digitado
errado) tem `details.reason = "not_current"`; `details.current` traz o ID aberto (ou `null`).
Scripts que gravam `gesture-1` à mão deixam de funcionar: leia o ID da resposta de `begin`.

**Slot compartilhado com o gesto de efeito (BL082.6).** O gesto de propriedades e o gesto de efeito
(`scene.fx.gesture.*`, [F016](effects.md)) disputam o **mesmo** slot: há um gesto aberto por vez e o segundo
`begin` devolve `gesture_active` (um gesto já invalidado é substituído e dito na resposta). Cada gesto tem um `kind`
(`properties` | `fx`) e `scene.gesture.*` recusa o ID de um gesto de efeito com `gesture_kind_mismatch`; a
invalidação (cena trocada ou destacada, undo, edição externa, outro projeto) é a mesma para os dois tipos.

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

`history.undo`/`history.redo` acrescentam `selection_dropped[]` à resposta (mesmo formato de `scene.detach`) quando
a restauração tirou algo da seleção; sem nada podado, a resposta é a de sempre.

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
- **Seleção que perde a entidade** é podada **na hora** ao desanexar a cena e em todo `undo`/`redo`
  (resposta com `selection_dropped`; BL069). `scene.selection` continua podando por garantia e lista
  tudo o que saiu (`dropped[]`). A seleção é apresentação: o undo **só poda**, nunca restaura, e o redo
  não traz de volta a entidade podada.
- **Gesto aberto não é tocado pelo undo/redo**: continua invalidado de forma preguiçosa
  (`gesture_invalidated`), de modo que a UI vê `state: "invalidated"` e pode cancelá-lo. Nunca reabilita:
  undo seguido de redo não revive o gesto.
- **Orçamento de bytes** (`MAX_SCENE_BYTES`, 128 MiB de JSON compacto somando as cenas): `update`
  acima do orçamento é recusado com `scenes_too_large` e preserva o último candidato bom, o documento, o
  histórico e a revisão; o mesmo lote como `scene.edit` dá o mesmo erro. O commit repete a checagem
  (`edit_scene`): se falhar, o gesto **volta ao slot inteiro** (candidato, ops, contagem). Esse ramo
  não é alcançável pela API pública, porque qualquer operação que muda o tamanho do livro também sobe
  a revisão e invalida o gesto antes; a prova dele é um teste unitário que cresce o livro sem subir a revisão.
- **Consultas, seleção e dry-run não criam undo** (sem `history_label`).

### Erros tipados

`unknown_entity`, `invalid_entity_path`, `selection_limit`, `no_gesture`, `unknown_gesture`,
`gesture_active`, `gesture_invalidated`, `gesture_target_mismatch`, `gesture_scene_mismatch`; mais os
já existentes: `unknown_element` (cena), `invalid_value`, `invalid_request`, `unknown_node`,
`unknown_path`, `frame_out_of_range`, `invalid_bundle`.

## Summary compartilhado (BL068)

`EditorState::summary()` (op `status`, `ui status`, `tessel-headless`) ganha o bloco `scene_interaction`
**somente** quando há algo a mostrar (seleção, entidades podadas ou gesto); um projeto que nunca usou
as ferramentas tem o summary de sempre. Ler o summary não poda, não encerra gesto e não cria histórico.

```json
"scene_interaction": {
  "selection": { "count": 1, "truncated": false, "dropped_count": 0, "dropped": [],
                 "entities": [{ "path": "scene/scene-a/node/figure", "scene": "street", "kind": "node", "id": "figure" }] },
  "gesture": { "id": "gesture-…", "scene": "street", "label": null, "targets": ["scene/scene-a/node/figure"],
               "state": "open", "invalidation": null, "updates": 1, "has_candidate": true,
               "would_change": true, "changed_ops": 1, "revision": 1 }
}
```

- `entities` lista até `SUMMARY_LIMIT` (64) entradas; `count` é sempre o total; `truncated` avisa o corte.
- Uma entrada cuja cena/entidade sumiu sem poda (não ocorre pelos caminhos atuais; defesa) aparece com
  `stale: "scene_detached" | "entity_missing"`.
- `gesture.state` é `open` ou `invalidated` (com `invalidation` = `scene_detached | scene_changed |
  project_revision`); `gesture` é `null` sem gesto.

## CLI headless

O ID do gesto só existe depois do `begin`, então a sessão é conduzida linha a linha (um `printf` fixo
não consegue citá-lo). Exemplo executado contra `target/debug/tessel-headless` (`./scripts/cargo build -p
tessel-editor-core --bin tessel-headless`):

```python
import json, subprocess

hl = subprocess.Popen(["target/debug/tessel-headless", "--empty"], text=True,
                      stdin=subprocess.PIPE, stdout=subprocess.PIPE)

def ask(request):
    hl.stdin.write(json.dumps(request) + "\n"); hl.stdin.flush()
    return json.loads(hl.stdout.readline())

street = "docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"
figure = "scene/scene-a/node/figure"
ask({"op": "scene.attach", "id": "street", "path": street})
ask({"op": "scene.select", "scene": "street", "entities": [figure]})
gesture = ask({"op": "scene.gesture.begin", "scene": "street", "targets": [figure]})["result"]["gesture"]
ask({"op": "scene.gesture.update", "gesture": gesture,
     "ops": [{"set": figure + "/position", "value": {"x": 2, "y": 0, "z": 0}}]})
print(ask({"op": "status"})["result"]["scene_interaction"])   # seleção + gesto aberto, histórico intocado
print(ask({"op": "scene.gesture.commit", "gesture": gesture})["result"]["changed"])   # True: uma entrada
print(ask({"op": "history.undo"})["result"]["label"])         # "Edit scene"
```

Mesmo envelope `{ok, version, result|error}` das demais. O exemplo acima foi executado pela frente R3 no
binário de debug; o teste `the_executable_keeps_gesture_ids_apart_…` automatiza o mesmo caminho.

## UI e CLI da janela

A janela executa os mesmos `scene.*` por `Op::Run(Command::Scene(..))`. Árvore/seleção/propriedades, Escape e confirmação por release estão em [F039-UI](scene-inspector.md). Socket e pixels foram exercitados no corpus da rodada M/N/O/P; testes GPUI injetam entrada, sem prova de ponteiro físico. Aparência segue pendente (BL087). O gizmo de translação (handles `x`/`y`/`z`/centro, arrasto = um gesto F039 = uma entrada de undo, `scene.translate`) está em [scene-gizmo.md](scene-gizmo.md) (F055, BL015, a validar); rotação/escala/snapping seguem pendentes (BL015.1).

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
- A invalidação do gesto continua **preguiçosa** (checada ao usar). Decisão R3/BL069: um gesto
  invalidado fica no slot de propósito, para a UI mostrar `invalidated` e cancelá-lo; podá-lo no undo
  tiraria esse estado e transformaria um erro tipado em `no_gesture`.
- IDs de gesto são válidos só na sessão que os emitiu. Um cliente não pode montá-los, reaproveitá-los
  entre projetos nem depender do formato.
- `MAX_SCENE_BYTES` conta JSON compacto do projeto inteiro; o teste público do orçamento aloca ~128 MiB
  (~3 s em build debug). A checagem repetida no commit não é alcançável pela API pública (ver "Garantias").
- O summary lista no máximo 64 entidades selecionadas; `scene.selection` tem a lista completa.
- Nós semânticos e controles em F039-UI; aparência, ponteiro físico e handles ainda não provados.

### Evidência da rodada R3 (BL066–BL069)

`crates/core/tests/scene_session.rs` (15 testes) e testes unitários em `scene_interaction/selection.rs`
e `scene_project/interaction.rs`:

| Superfície | O que prova |
|---|---|
| Lib | IDs de projeto antigo/reaberto/40 sessões não colidem e o pedido velho não toca o gesto novo; `reason` `other_session`/`not_current`; summary (bloco, limite, leitura sem efeito, gesto invalidado); undo/redo podam a seleção na hora, redo não restaura, resultado independente de consulta intermediária; gesto morto após undo+redo; orçamento (update recusado preservando candidato; commit recusado devolve o gesto inteiro); no-op, cancel e update rejeitado sem rastro |
| Executável `tessel-headless` (JSONL, processo real) | no-op commit, cancel, commit/undo/redo com gesto invalidado, troca de projeto na mesma sessão com ID antigo recusado (`other_session`) e summary do processo com seleção/gesto |
| Socket / UI | **não reexecutado por R3.** O adaptador desktop usa o mesmo core; nenhuma prova nova de janela |

Antes da correção os testes de BL066 (mesmo ID em dois projetos) e BL069 (seleção volta num redo
conforme alguém consultou ou não) **falharam** contra o código anterior; ver o relatório.

Integração pelo coordenador: testes públicos 21/21, unitários no gate core (197/197 no conjunto) e clippy lib limpo. Corrigidas duas expectativas: alvo fora do gesto é recusado antes de validar nó inexistente (`gesture_target_mismatch`). O executável headless passou uma sessão real de 13 requests (attach, árvore, seleção, evaluate, begin/update/commit, get, undo/redo). A integração posterior da janela está em F039-UI; testes e CLI sem janela não provam ponteiro físico.

A última proposta recusada de um gesto de propriedades é exposta em `gesture.refusal`, sem alterar o candidato legal ou o documento. Um update válido limpa a recusa; segue o mesmo contrato do gesto FX. Um update recusado antes de calcular candidato (gesto invalidado ou de outro tipo) não grava recusa. Evidência em processo real (`tessel-headless`) e testes em `scene_interaction.rs` no código `7deb23c`.
