# F019 — Importar e processar mídia real (P05/P06)

## Status

Lib **V** (`crates/media/tests/import.rs`, `export.rs`, `flow.rs`), `scripts/media` **V**, janela via `ui op` **V**, UI **V**
(diálogo Importar com seletor nativo de arquivos, biblioteca com estado dos assets; provado em
`scripts/live-flow-proof.py`: mp4, png, ply, wav). Setup de FFmpeg: [media-setup](../media-setup.md).

**Descoberta do FFmpeg/FFprobe (BL167.2):** `TESSEL_FFMPEG`/`TESSEL_FFPROBE` válido vence e um inválido **falha** (sem fallback);
sem eles, o `PATH` na ordem e depois os lugares usuais da plataforma. No Windows: `ffmpeg.exe` (e as extensões de `PATHEXT`
que o sistema inicia), pastas com espaços, WinGet `Links` e o layout `Packages\Gyan.FFmpeg*\ffmpeg-*\bin` (dois níveis, sem
varredura recursiva); um diretório é rejeitado. macOS/Linux: o `PATH` e `/opt/homebrew/bin`, `/usr/local/bin`, como antes.
Resolvedor `tessel_editor_media::tools::locate` (adapter, sem regra de domínio); testado com ambiente, plataforma e arquivos
injetados. Prova real no Windows: [relatório](../migration/windows-tool-discovery-report.md). Não há prova no macOS/Linux
desta entrega (só os testes unitários da plataforma `Other`).

## Importar (P05)

Dois tempos: `prepare` (probe, hash, cópia) só toca o filesystem e roda como **job cancelável**; `commit` aplica tudo ao
projeto numa transação (**um undo**). A falha de um arquivo é resultado **por item**; os outros importam
(sucesso parcial documentado). A identificação é **pelo conteúdo** (bytes + FFprobe), nunca pela extensão.

| `op` | Campos | Notas |
|---|---|---|
| `media.import` | `paths[]`, `mode?` `copy` (padrão) \| `reference`, `folder?`, `prepare?` (padrão `true`), `wait?` | `copy` vai para `<workspace>/media/<hash>/` (exige workspace: `no_workspace` → salve antes); `reference` só registra o caminho. Por item: `ready` / `duplicate` (mesmo hash, `of` aponta o asset) / `failed` (`unsupported_format`, `io_error`, `decode_failed`) |
| `media.status` | — | por asset de arquivo: `ok`, `missing` ou `changed` (tamanho/hash), `storage`, `path`, `hash` |
| `media.relink` | `asset`, `path`, `accept_changed?`, `wait?` | confere tamanho, SHA-256, tipo e duração — **nome igual não basta**; com `accept_changed` aceita outro arquivo compatível e marca `content_changed`. Clipes e overrides preservados |
| `media.search` | `folder`, `apply?` | procura arquivos movidos (não segue links simbólicos); `apply` religa os que conferem |
| `media.prepare` | `assets?` | decodifica vídeo em contêiner e áudio comprimido para o cache. Na janela é um job; **não é mais necessário rodar à mão**: o host agenda o decode de assets sem `decoded` ao abrir/importar |

Formatos garantidos: PNG/JPEG/WebP/BMP/TIFF (imagem), WAV e áudio que o FFmpeg decodifique, vídeo H.264/AAC (MP4/MOV) e o que
o FFprobe da máquina decodificar, cenas `tessel-scene@1`, splats (`splat|ply|spz`, [F030](spark.md)), sequências de frames
numerados **PNG ou JPEG** (um formato, um tamanho, todos os quadros presentes; padrão `%0Nd` ou `%d`). `webm` **não é
garantido**: só o que o monitor lê de verdade (prova no carregamento; no WebKit VP8/VP9/AV1 viram `unsupported_format`);
o caminho garantido para qualquer contêiner é o import (FFmpeg → quadros). Vídeo por `asset.add path=…` usa a taxa e a
duração **declaradas** (CFR, PTS 0): ver [relatório](../viewport-bugfix-report.md). Unidades: tempo e FPS racionais.

**VFR:** a detecção é pelos **timestamps dos pacotes** (`classify_timing`: arredondados a ms, um intervalo constante é CFR; frame
descartado ou esticão é VFR), não pelo FPS declarado. Um vídeo VFR é **conformado** a uma grade constante (`fps=…:round=near`:
repete ou descarta o frame mais próximo, nunca mistura) no decode; o import avisa `variable_frame_rate`, o asset ganha
`meta.vfr_conformed` e o `export.preflight` repete o aviso. Provado com um vídeo gerado com frames faltando
(`crates/media/tests/video_audio.rs`: o frame na posição certa tem a cor certa; CFR não é marcado). Limite: o conforme é por
asset; sequências com FPS diferente do asset seguem a regra de repetir/descartar do export (`frame_rate_conversion`).

**Áudio do vídeo:** `Meta.audio_streams[]` lista os streams (índice, codec, canais, taxa); `decode` extrai cada um para PCM no
cache (`Decoded.audio`). Propriedades do clipe de vídeo: `audio_muted` e `audio_stream`. O som entra no mesmo plano de
áudio do playback e do export ([F029](video-export.md)).

## Derivados (P06)

Cache descartável em `<workspace>/.tessel/cache/<hash>/…` com `cache.json` que registra de onde veio: hash/tamanho da
fonte, parâmetros, versão do FFmpeg e do schema; qualquer mudança invalida e reconstrói em staging.

| `op` | Campos | Resultado |
|---|---|---|
| `media.thumbnail` | `asset`, `width?` (320), `at?` (s) | caminho do PNG |
| `media.waveform` | `asset`, `buckets?` (96) | picos 0..1 |
| `media.proxy` | `asset`, `height?` (360) | `{path, preview_only:true}` |

Um proxy é **só preview**: não muda o tempo e nunca substitui o original no export (`Derived::for_export`).

**Proxy de scrub (OPT01).** Além do `media.proxy` (arquivo de vídeo 360p), o vídeo container decodificado ganha uma
**sequência JPEG/PNG** com a mesma numeração e contagem dos frames decodificados e altura exata (padrão 540, largura par
calculada do IHDR do primeiro frame). O job automático "Build preview proxy" roda depois do decode (`proxy_failed` evita
repetir uma falha); `media.preview_proxy` `{assets?}` o pede à mão e responde `policy_original` quando a política é
`original`. Cache em `<hash>/preview-<altura>-<ext>`, impressão digital com frames/contagem/formato/qualidade, contagem
validada; recusa (`unsupported_format`) quando a fonte não é mais alta que o proxy. Aplicar é transação com hash e **não é
edição**. Contrato de uso no monitor e no export: [viewport](viewport.md#proxy-automático-de-preview-opt01).

## Limites (honestos)

Original nunca é movido nem apagado. Fixtures determinísticos: `./scripts/fixtures <dir>`. O servidor de assets da WebView só entrega
arquivos registrados por `asset.load` (403 caso contrário; ranges 200/206/416). Caminhos `decoded` não são persistidos no documento, mas o cache em disco é reaproveitado pelo hash: abrir um projeto
agenda o decode (cache hit é rápido) e `render.frame`/export esperam por ele. Não há transcode com recorte/handles.
