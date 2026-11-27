# F070 — Digitação, revelação por linha e preset de chat (BL169.6, etapa 3)

Entrega local da etapa 3 do [handoff](../migration/native-video-authoring-handoff.md). Reaproveita os
[grupos](groups-keyframes.md) (F069) e o texto (F066). Nada aqui é commitado nem publicado.

## Modelo

- **Revelação do texto** (`Reveal`), por filho de texto de um grupo:
  - `reveal`: `none` (padrão), `type` (caractere a caractere) ou `lines` (linha a linha).
  - `step_ms`: duração de cada passo, entre `MIN_STEP_MS` e `MAX_STEP_MS`.
  - O relógio é o do clipe (`local_ms`), o mesmo na prévia, no quadro e no export.
- **Agenda do filho**: `after` (id de um filho **anterior**; vazio = pelo escalonamento) e `delay_ms`.
  - Início = fim de `after` (ou `índice × stagger_ms`) + `delay_ms`.
  - Fim = início + o maior entre o tempo da revelação e o das trilhas de chaves.
  - `after` que aponta para frente, para si ou para um id inexistente é recusado; remover um filho que
    outro espera é recusado (`in_use`). O documento e o histórico ficam intactos nas recusas.
- **Caixa de texto** (`width` do estilo de texto, % da largura do quadro; 0 = automática, 90% da área
  segura). Um texto com revelação sempre tem caixa.
- **Preset de chat** (`chat.add`): clipe de 8 s com os filhos `panel`, `campaign`, `question` (`type`,
  atraso 400 ms), `answer` (`lines`, depois da pergunta, atraso 300 ms), `source` (`type`, depois da
  resposta, atraso 200 ms) e `source-mark` (depois da fonte, fade, cor `CHAT_MARK_COLOR`). Estilos
  `chat-campaign`, `chat-question`, `chat-answer` e `chat-source` ficam no projeto e são editáveis.
  Campanha, pergunta, resposta e fonte são os textos do clipe (`clip.group_text`).
- Reeditar a pergunta reagenda resposta, fonte e marca sozinhas por causa do `after`.

## Quebra estável

O core quebra as linhas (`text::lines_of`): bitmap por `layout_in`, sans por `layout_sans` com os avanços
medidos da Inter (`inter_widths.rs`, gerados por `runtime/scripts/measure-inter.mjs`). O plano leva o
layout **completo**; o runtime dimensiona o canvas por ele e desenha só os primeiros `visible`
caracteres de cada linha, então nada se desloca durante a digitação.

Protocolo do plano (`text` em cada camada, campos omitidos nos padrões):

| Campo | Significado |
|---|---|
| `visible` | Quantidade de caracteres (pontos de código) já mostrados; ausente = todos |
| `lines` | Linhas decididas pelo core, para sans com caixa e bitmap multilinha |
| `width` | Largura da caixa em % |

`line_starts` (Rust, `text.rs`) e `lineStarts` (JS, `runtime/src/text-reveal.js`) percorrem o conteúdo da
mesma forma e têm as mesmas fixtures.

## Superfícies

| Superfície | Como |
|---|---|
| Timeline | Botão `timeline-add-chat` (`Command::AddChat`, um undo remove o clipe) |
| Inspetor | Seção Grupo: linhas "Depois de", "Atraso", "Revelar", "Passo" (só com revelação) e "Caixa" no estilo |
| CLI/JSONL | `chat.add`; `invoke timeline.add_chat`; `set group_child:<g>/<c> reveal\|step_ms\|after\|delay_ms`; `set text_style:<id> width`; `clip.group_text` para os textos |
| Árvore | Nó `timeline.add_chat` e os campos novos nos nós de filho e de estilo |

## Persistência

Campos novos são omitidos nos padrões: documentos antigos continuam byte a byte iguais e o esquema 2
só é usado quando há grupos. Não foi testado abrir um projeto com revelação em um editor da etapa 2.

## Evidências

- Core, `crates/core/tests/chat_typing.rs` (8): inserção e undo do chat; sequência pergunta → resposta por
  linha → fonte → marca a partir das agendas; reagendamento por texto do clipe; `set`/undo byte a byte;
  campos da árvore; recusas sem alterar documento nem histórico; salvar e reabrir com plano idêntico
  (documentos forjados recusados); grupos antigos sem as chaves novas. Mais testes de unidade em `text.rs`
  (percurso das linhas, caixa estável) e `group.rs` (passos da digitação, linhas, faixa de `step_ms`).
- GPUI, `group_clip_tests.rs` (3): botão do chat e um undo; edição de revelação e agenda no inspetor com
  undo até os campos; `after` para frente recusado e o campo volta ao modelo.
- Runtime: `text-reveal.test.mjs` e dois testes Chromium em `typographic-text-browser.test.mjs`
  (bitmap e sans digitam sem deslocar o que já apareceu).
- Quadro e export reais, `crates/render/tests/chat_frames.rs` (4, 640×360 a 24 fps, `TESSEL_REQUIRE_RUNTIME=1`):
  pixels por região (pergunta, resposta, fonte, marca `[255,176,32]`) no PNG de `render.frame`; edição,
  reagendamento e undo byte a byte; reabertura idêntica; MP4 exportado com os quadros decodificados pelo
  ffmpeg mostrando a mesma sequência.
- Cópia privada de demonstração (ignorada pelo git, sem conteúdo no repo): variante do projeto da
  apresentação com o cartão nativo no lugar da camada de PNGs nos quadros 1470–1698 (228 quadros),
  MP4 de 7,6 s, verificação ok. A montagem principal e as outras cópias não foram alteradas.

## Limites

- Campos como texto; sem edição da revelação no monitor nem marcadores na timeline.
- `layout_sans` não usa kerning e não encolhe pela altura (conservador: pode quebrar um pouco antes).
- A digitação conta pontos de código; o texto é normalizado em NFC.
- Texto bitmap sem `lines` (caixa automática de linha única) não tem revelação por linha.
- A demonstração privada foi exportada com as trilhas de mundo 3D (`world-a`, `world-b`) ocultas **só
  na sessão**, porque o export com os splats sob SwiftShader não terminou em 24 min; o texto da demo é
  genérico, não o conteúdo do chat original, e o cartão não foi posicionado contra os títulos da
  montagem. Converter o vídeo inteiro é a etapa 6.
- Sem ponteiro físico nem captura da janela real.

## Falhas fora desta entrega

- Runtime `pnpm test`: 5 testes de orquestrador (`orchestrator-contract`, `-loads`, `-preupload`, `-timing`,
  `-warm-draw`) falham com `ERR_UNKNOWN_FILE_EXTENSION .woff2` no Node; a causa é o import do
  `typographic-text.js` pelo compositor desde a F068. 499 de 505 passam.
- `cargo test --workspace`: alvos de cena e mídia que não toco (`scene_gizmo_rotation`,
  `scene_gizmo_scale`, `directed_resources_transport`, `scene_resource_integration`,
  `scene_resource_publication`; `scene_export` de mídia e de render falham por `shasum` ausente no
  Windows; `scene_claims_journal` tem um teste de temporização, `a_window_killed_right_after_its_tick…`,
  que falhou ao reexecutar sozinho). Sem comparação com HEAD limpo nesta rodada.
