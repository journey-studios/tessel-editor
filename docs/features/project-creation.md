# F015 — Criar projetos, tracks e clipes dinâmicos

## Objetivo e limites

Criar um projeto vazio e configurável, cadastrar assets tipados, criar tracks e clipes com
IDs escolhidos pelo chamador, configurar câmera e propriedades, e avaliar o resultado, sem
seleção de UI e sem o estado demonstrativo. O demo continua existindo, mas é um *bootstrap
explícito* (`EditorState::demo()`; `--demo` nas CLIs).

Fora do escopo desta entrega: editar clipes existentes por vocabulário de projeto além de texto,
câmera e propriedades. Persistência/undo/sequências/import foram entregues depois em
[F022](project-document.md), [F024](history.md), [F023](sequences.md) e [F019](media-io.md); a barra de status
mostra agora o estado salvo/não salvo e as tarefas em andamento.

## Contrato do core / biblioteca

- `EditorState::empty(&ProjectSettings) -> Result<EditorState, CommandError>`; `ProjectSettings
  { name, sequence: SequenceSettings { id, rate: Rate, length, width, height } }`.
- `Id` (`tessel_editor_core::Id`): validado (1–64 bytes; letras, dígitos, `-`, `_`, `.`),
  `Copy`. **Trade-off:** é internado e vive até o fim do processo; o limite de memória é o
  número de IDs *distintos* já criados, não de entidades vivas.
- Comandos: `AddAsset`, `CreateTrack`, `AddClip`, `SetClipText`, `SetClipCamera`,
  `SetClipProperty`, `SetSequenceLength`, `SetPreview`, `SetPlayhead`. IDs ausentes são
  gerados; IDs repetidos ou desconhecidos são rejeitados sem mutar o estado.
- Tipos de asset: `image`, `video`, `audio`, `text`, `scene`, `splat`; `Asset::probe` lê o
  cabeçalho (png, wav, splat) e rejeita arquivo ilegível/inválido com erro tipado.
- Vocabulário JSON (`ops::parse_op`, igual na lib, CLI headless e janela):

| `op` | Campos principais |
|---|---|
| `project.create` | `name`, `rate` (int ou `{num,den}`), `length` (frames) **ou** `seconds`, `width`, `height`, `sequence` |
| `asset.add` | `kind`, `name`, `id?`, `path` ou `pattern`+`frames`+`rate` (frames numerados `%05d`), `text`, `folder?` |
| `track.add` | `kind` (`video`/`audio`), `name`, `id?` |
| `clip.add` | `track`, `asset?`, `start`, `duration`, `id?`, `source_start?`, `text?`, `camera?`, `properties?` |
| `clip.text` / `clip.camera` / `clip.property` | `clip`, e `text` (`content` em NFC ou NFD, ASCII + acentos do português; [F050](accented-titles.md)) / `position,target,fov,orbit` / `property`+`value` |
| `sequence.length`, `preview.set`, `seek` | `frames`; `intent`; `frame` ou `time` |
| `plan`, `render.frame`, `render.audio`, `capabilities`, `render.status`, `status` | ver [F021](viewport.md) |

```sh
printf '%s\n' \
 '{"op":"project.create","name":"Demo","width":320,"height":180,"length":48}' \
 '{"op":"asset.add","kind":"text","id":"label","name":"Label","text":{"content":"HI","color":"#ffffff","size":14}}' \
 '{"op":"track.add","id":"titles","name":"Titles","kind":"video"}' \
 '{"op":"clip.add","id":"hi","track":"titles","asset":"label","start":0,"duration":24}' \
 '{"op":"clip.add","id":"x","track":"nope","asset":"label","start":0,"duration":5}' \
 '{"op":"plan","frame":0}' | ./scripts/headless
```

A quinta linha responde `{"error":{"code":"unknown_element","message":"No track with ID \`nope\`"…}}`
e o plano seguinte contém só o clipe `hi`. Erros de forma: `invalid_request` (campo ausente ou
tipo errado), `invalid_id`, `invalid_value` com as opções válidas.

## CLI headless

`./scripts/headless` (sem renderer) e `./scripts/render` (com renderer) leem JSON Lines e
mantêm um único `EditorState` por sessão. Sem `--demo` começam vazios. Teste:
`crates/core/tests/proof.rs` (8) e `tests/headless.rs` (2).

## UI e CLI da janela

`./scripts/ui op '<json>'` aplica o mesmo vocabulário no estado vivo; o timeline, a
biblioteca e o monitor refletem o projeto criado (tamanho/FPS/duração vêm de
`state.timeline`, não de constantes). Evidência: `scripts/live-proof.py` constrói o projeto
da prova por `ui op` (passo "project built") e confere o mesmo resultado do headless.
Miniaturas da biblioteca de assets gerados ainda são placeholders. Não há UI de diálogo
para criar projeto.

## Evidências e status

Lib V, CLI headless V (os 8 testes do `proof.rs` incluem erro sem mutação), janela V por
`live-proof.py` (macOS). Limite: Id internado; o vocabulário de projeto não edita
`set/invoke/move` (esses seguem `ui set|invoke|move`).

## Continuous property edits

For a slider-like stream, use shared `property.gesture.begin/update/commit/cancel` instead of discrete sets per sample. This preserves live preview while recording one undo at commit. Contract and JSONL example: [properties](properties.md#gesto-de-slider-bl062-preview-ao-vivo-uma-entrada-no-undo) and [UI automation](../ui-automation.md#property-slider-gestures).
