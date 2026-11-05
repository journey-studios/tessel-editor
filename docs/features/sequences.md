# F023 — Múltiplas sequências (P01)

## Status

Lib **V** (`crates/core/tests/project.rs`, `state/tree.rs`), headless **V**, janela via `ui op` **V**, UI **V no desktop
testado** (`workflow_tests.rs`): a timeline tem um **seletor de sequências** (`Select` do Kit, nó semântico
`timeline.sequence`, rótulo acessível «Sequência ativa») que lista todas as sequências e executa `ActivateSequence`; o
trabalho de cada sequência se mantém ao alternar. Provado por nó semântico (`ui get timeline.sequence`, `set
timeline.sequence value=<id|nome>`), **não** por clique de coordenada; o Select aberto como popover não foi inspecionado
visualmente.

## Contrato

O projeto tem N sequências, cada uma com FPS racional, canvas, comprimento, tracks e clipes. Uma é a **ativa**:
`EditorState.timeline` guarda a ativa e `sequences` as inativas (ativar troca uma pela outra); todo plano, edição e
o monitor leem a ativa, sem um segundo caminho de código.

| `op` | Campos | Erros |
|---|---|---|
| `sequence.add` | `id?`, `name?`, `rate?`, `width?`, `height?`, `length?`, `activate?` | `duplicate_id`, `invalid_argument` |
| `sequence.activate` | `sequence` | `unknown_element` |
| `sequence.remove` | `sequence` | `sequence_active` (ative outra antes), `unknown_element` |
| `sequence.rename` | `sequence`, `name` | `unknown_element` |
| `sequence.length` | `frames` | `invalid_duration` |

Nó `timeline.sequence`: `state.value` (id ativo), `state.options`, `state.sequences[{id,name,width,height,length,fps,active}]`.
Quando dois nomes colidem o rótulo ganha o id entre parênteses.

Os comandos são desfazíveis ([F024](history.md)). Exportar uma sequência específica: `export.start` com
`sequence`. Importar sequências de outro projeto: [F027](interchange.md).
