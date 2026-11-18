# F071 — Player de áudio e faixa de legendas SRT (BL169.6, etapa 4)

Entrega local da etapa 4 do [handoff](../migration/native-video-authoring-handoff.md). Reaproveita
os picos e o relógio exato do core (`audio.rs`, `Rate`), as formas (F067/F068), o texto sans (F066) e o
mecanismo de camadas dos [grupos](groups-keyframes.md) (F069). Nada aqui é commitado nem publicado.

## Modelo

Dois tipos novos de clipe, `player` e `subtitles`. Como o grupo, cada um **vira camadas comuns de forma e
texto no plano** (`plan/live_layers.rs`); o compositor não conhece nenhum dos dois e não há mudança no JS.
Os dois leem só o quadro que está sendo desenhado: prévia, `render.frame`, seek e export resolvem o mesmo
quadro nas mesmas camadas.

### Player (`player.rs`)

- Campos (`PlayerSpec::FIELDS`): `audio` (id de um clipe de áudio da sequência ativa, vazio = segue o próprio
  clipe), `width`, `height`, `bars` (8 a 96), `gap`, `wave`, `bar`, `time` (liga ou desliga cada parte),
  `wave_color`, `idle_color`, `track_color`, `fill_color`, `text_color`, `bar_height`, `text_size`, `spacing`.
- **Relógio**: `frame_to_time(quadro − início)` sobre o trecho do clipe de áudio seguido (início e duração
  dele, já aparados), ou do próprio clipe do player. Antes do trecho o player fica em 0, depois fica no fim.
- **Onda**: uma forma retangular por barra. A altura vem dos picos reais do trecho do arquivo que o clipe
  toca (`audio::waveform_window`, janela em segundos do arquivo, normalizada pela barra mais alta). Barras
  já tocadas usam `wave_color`, as demais `idle_color`. Cache de picos global, limitado a 16 janelas, com
  chave por caminho, tamanho, mtime, janela e número de barras: um export não relê o arquivo a cada quadro.
- **Progresso**: um trilho (`track_color`) e um preenchimento (`fill_color`) de largura `width × progresso`.
  O preenchimento só existe a partir de 0,01% da largura.
- **Cronômetro**: texto sans `m:ss / m:ss`. O decorrido conta só segundos completos; no fim (ou depois dele) mostra
  o total, que é arredondado.
- Um `audio` que aponta para um clipe que não é de áudio é recusado na edição (`invalid_link`). Se o clipe
  some depois (outra edição), o plano avisa `player_audio_missing` e o player segue o próprio clipe; se o
  som não pode ser lido, avisa `player_audio_unavailable` e as barras ficam planas, com progresso e
  cronômetro funcionando.

### Legendas (`subtitles.rs`)

- Uma faixa por clipe: cues (`start_ms`, `end_ms`, `text`) e estilo (`size`, `color`, `width`, `y`, `box`,
  `pad`, `radius`, `burn`). O texto é sans, centralizado, com caixa opcional (`box` = `#rrggbb[aa]` ou `none`).
- O tempo do cue é o **relógio de conteúdo** do clipe: `source_start + decorrido`, como um vídeo lê o
  arquivo. Aparar a cabeça ou dividir o clipe mantém cada legenda no quadro em que estava, sem reescrever cue.
- Cues ordenados por início, sem sobreposição (`invalid_duration` e `cue_overlap` recusados), no máximo 2000, tempo
  até 24 h.
- **SRT**: `subtitles.import` (texto `srt` ou arquivo `path`, até 2 MiB) troca todos os cues e alonga o clipe
  até o fim do último cue, até onde a sequência e o próximo clipe da faixa deixam (nunca é erro por falta de
  espaço). Marcação (`<i>`, `{\an8}`) é removida e a pontuação tipográfica vira simples na entrada.
  `subtitles.export` devolve o SRT e, com `out`, grava o arquivo (`HH:MM:SS,mmm`, numerado, `\n`).
- **Queimar ou não (`burn`)**: ligado (padrão), o export desenha as legendas no vídeo. Desligado, o export
  tira a faixa do snapshot, não desenha nada dela e grava `<vídeo>.srt` ao lado (`<vídeo>.<clipe>.srt` quando há
  mais de uma faixa), com os tempos no relógio do **vídeo exportado** (zero no primeiro quadro, cortados pela
  faixa de export e pelo clipe). A prévia mostra as legendas nos dois casos. O projeto não muda: o export usa
  uma cópia. Sem `replace`, um `.srt` que já existe recusa o export **antes** de renderizar.

## Superfícies

| Superfície | Como |
|---|---|
| Timeline | Botões `timeline-add-player` (cobre o clipe de áudio que segue, ou o cursor se não houver áudio) e `timeline-add-subtitles` (um cue inicial); um undo remove cada um |
| Inspetor | Seção **Player** (16 campos com amostra de cor) e **Legendas**: campo "Importar" (Enter lê o `.srt`), editor de um cue por vez (anterior, próxima, adicionar no cursor, remover; início, fim, texto) e os 8 campos de estilo. Enter ou sair do campo faz **uma** edição, a mesma de `set` |
| CLI/JSONL | `player.add`, `subtitles.add`, `clip.add` com `player` ou `subtitles`, `player.set`, `subtitles.set`, `subtitles.import`, `subtitles.cue.add\|set\|remove`, `subtitles.export`; `set clip_player:<id> <campo>`, `set clip_subtitles:<id> <campo>`, `set subtitle_cue:<id>/<n> start_ms\|end_ms\|text`; `invoke clip_subtitles:<id> add_cue`; `invoke timeline.add_player` e `timeline.add_subtitles` |
| Árvore | Nós `clip_player:<id>`, `clip_subtitles:<id>`, `subtitle_cue:<id>/<n>` e os botões da timeline |

## Persistência

`player` e `subtitles` entram no clipe e forçam o **esquema 2**; um documento esquema 1 com esses tipos é
recusado (um leitor da etapa 3 os descartaria). Um projeto sem eles continua esquema 1, byte a byte igual.
A abertura valida tudo antes de trocar o projeto; um campo inválido recusa o documento nomeando o clipe.
Cada edição é um `Command` com um snapshot de histórico (o clipe inteiro volta no undo).

## Evidências

- Core, `crates/core/tests/player_subtitles.rs` (12): relógio do player quadro a quadro (zero, metade, fim e
  depois do fim), barras tocadas e ociosas, picos do áudio sintético (a metade alta é bem mais alta que a baixa),
  player sem áudio, undo/redo por edição e recusas sem alterar o documento, `set`/`invoke` semânticos, cues
  nos quadros certos (incluindo bordas), divisão e aparagem mantendo cada cue no quadro, SRT de ida e volta
  (arquivo, recusas, alongamento do clipe), cues ordenados e sobreposição recusada, salvar/reabrir com plano
  idêntico (esquema 2, esquema 1 forjado e campo inválido recusados), botões da timeline, relógio do `.srt`.
  Mais 12 testes de unidade em `player.rs` e `subtitles.rs`.
- Mídia, `crates/media/tests/export_captions.rs` (4, renderizador de teste): um `.srt` ao lado do vídeo com o
  nome dele; vários arquivos `<vídeo>.<clipe>.srt` com mais de uma faixa; um `.srt` existente recusa o export
  antes de qualquer quadro (`destination_exists`) e `replace` o troca; faixas de export (`range`) deslocam e cortam
  os cues para o relógio do vídeo; o projeto fica intacto (snapshot).
- GPUI, `live_clip_tests.rs` (8): botões da timeline e um undo; campos do player e do estilo mostram o modelo e
  Enter faz uma edição com undo; valor recusado volta ao modelo e é reportado; evento atrasado não escreve em
  outro clipe; navegação, adição e remoção de cues; importação de `.srt` e arquivo inexistente reportado.
- Quadro e export reais, `crates/render/tests/player_frames.rs` (6, 640×360 a 30 fps, voz sintética de 4 s com
  metade baixa e metade alta, `TESSEL_REQUIRE_RUNTIME=1`):
  - PNG de `render.frame`: barras tocadas (brancas) à esquerda e ociosas à direita aos 2 s, preenchimento com
    192 ± 4 px de 384, barras baixas ≤ 12 px e altas ≥ 40 px, cronômetro diferente de 0:00 para 0:02, quadro
    parado depois do áudio; legendas ausentes antes do cue, centralizadas (±6 px) sobre a caixa, ausentes entre
    cues e depois do último.
  - Seek: frente, trás e uma sessão reaberta desenham os mesmos bytes.
  - Edição de cor do player e do texto, undo e redo byte a byte, reabertura com os mesmos pixels.
  - MP4 exportado (H.264/AAC) decodificado com ffmpeg: o mesmo player e as mesmas legendas; o áudio do arquivo
    tem a metade alta mais de 5× mais forte que a baixa; com `burn` desligado nenhuma legenda nem caixa nos
    quadros e `captions[0]` no relatório, com o `.srt` esperado (`00:00:00,300 --> 00:00:01,500` …).

## Limites

- Uma forma por barra (8 a 96): uma onda de 384 px com 96 barras não é uma curva contínua. O `path` de forma
  limita a 128 segmentos, por isso não foi usado.
- O progresso é **por tempo**: no último quadro do áudio ele ainda não chegou a 100% (119/120 → ~99%);
  só chega ao fim quando o quadro passa do trecho. O cronômetro conta segundos completos.
- A onda é a dos picos do arquivo normalizados, **sem** ganho, fade, mute de faixa nem velocidade: ela mostra o
  som, não a mixagem. O trecho respeita o início aparado e a duração do clipe de áudio.
- O `audio` liga o player a **um id**. Dividir o clipe de áudio deixa o player só na metade que mantém o id;
  substituir o clipe quebra o vínculo (aviso `player_audio_missing`). Só clipes da sequência ativa.
- Legenda: sem sobreposição de cues, sem estilo por cue (um estilo por faixa), sem quebra automática por
  tempo de leitura, sem posição por cue. Marcação do SRT é removida e a pontuação tipográfica é normalizada
  (a fonte não desenha o resto); um caractere que a fonte não desenha recusa o cue, nomeando-o.
- `burn` desligado vale para a sequência exportada: uma legenda dentro de uma sequência aninhada continua
  desenhada, porque o relógio dela não é o do vídeo.
- Campos como texto no inspetor (sem seletor de cor, sem arrastar cue na timeline nem no monitor); um cue por vez.
- O player não segue um clipe de vídeo; e `subtitles.import` lê o arquivo do disco do editor (não há importação
  pela janela com seletor de arquivos).
- Sem ponteiro físico nem captura da janela real: o GPUI foi testado por eventos de teste e geometria.
- Não foi feita demonstração privada separada nesta rodada; a montagem principal e as cópias privadas
  anteriores não foram tocadas. Converter o vídeo inteiro é a etapa 6.

## Falhas fora desta entrega

Ver as de [F070](chat-typing.md#falhas-fora-desta-entrega): continuam as mesmas e não foram comparadas com um HEAD limpo.
