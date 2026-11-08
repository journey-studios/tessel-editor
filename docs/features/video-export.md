# F029 — Exportar vídeo com áudio (P10/P11)

## Status

Lib **V** (`crates/media/tests/export.rs`, `video_audio.rs`, `flow.rs`; `crates/core/tests/video_sound.rs`), `scripts/render`
**V** (`scripts/headless-flow-proof.py`: 1080p e 4K com o som do próprio vídeo), UI **V** (diálogo Exportar com
relatório de preflight assíncrono, job, progresso, cancelar; `scripts/live-flow-proof.py` mede o som exportado). Exportação ocorre em renderer **headless próprio** (Node+Chrome,
thread própria), nunca captura o monitor.

## Pipeline

`snapshot imutável → preflight → prepare (decode) → mix WAV em blocos de 10 s → frame PNG a frame → FFmpeg
(image2pipe) → ffprobe verifica → publica por rename`. O **mesmo plano do core** decide pixels, tempo e amostras;
o tamanho de render é o real (4K renderiza em 3840×2160, não amplia preview). Memória limitada: um frame por vez
cruza o pipe, o áudio vai direto para arquivo.

| `op` | Campos | Notas |
|---|---|---|
| `export.preflight` | `preset?`, `destination`, `sequence?`, `range?` | `{ok, blockers[], warnings[], summary}` (`summary`: faixa, tamanhos de render/saída, áudio, `estimate` de bytes de saída/staging/memória, espaço livre, encoder, versão do FFmpeg, capacidades do renderer) |
| `export.start` | idem + `replace?`, `wait?` | job `video_export`; `range` = `[início, fim)` em frames |

Blockers: `invalid_preset`, `invalid_destination`, `destination_exists`, `disk_full`, `encoder_unavailable`,
`asset_missing`, `asset_changed`, `no_workspace` (mídia a decodificar sem workspace), `render_too_large`,
`audio_too_large`, `audio_too_long`. Erros do plano de áudio, que falham o job antes de publicar qualquer arquivo:
`audio_stream_missing` (o clipe pede um stream que o arquivo não tem) e `asset_not_decoded` (o host prepara antes; só aparece
para quem chama o core direto). Avisos:
`frame_rate_conversion` (repete/descarta, nunca mistura), `aspect_mismatch` (barras ou corte central conforme `fit`),
`no_audio_sources`, `variable_frame_rate` (vídeo VFR conformado a uma grade constante, [F019](media-io.md)).
`video_audio_not_mixed` **não existe mais**: o som do vídeo entra no mix.

## Som do vídeo no mix

Uma só regra do core (`EditorState::audible_clips`/`audio_sources`/`plan_audio`) decide o que soa, e vale para o export **e**
para o playback: clipes de tracks de áudio e o **som embutido dos clipes de vídeo** (`Meta.audio_streams`/`Decoded.audio`).
Por clipe: `audio_muted` (silencia só o som; esconder a track de vídeo **não** silencia, é um comando separado de mute) e
`audio_stream` (qual stream; padrão o primeiro). Tempo do som: `trim`, `start` e o `offset` do stream (relógio do asset,
inclusive negativo), e `speed` é varispeed (muda altura). Um vídeo sem áudio não soa e não é erro; pedir um stream que não existe
é `audio_stream_missing`. Provas: amostras exatas com rampa WAV no core (`video_sound.rs`), tom na posição certa depois do
export (`video_audio.rs`: trim, offset, speed, mute, stream ausente) e análise espectral dos arquivos 1080p e 4K gerados
pelas três superfícies.
`export.start` roda o preflight e falha com os mesmos blockers.

Verificação final (ffprobe lê o arquivo de volta e o compara com o plano): codec h264, resolução, `yuv420p`, taxa de frames, nº de frames e duração do vídeo e do áudio; qualquer divergência falha o job e nada é publicado (exit 0 do encoder não basta). Cancelar remove o staging e não deixa arquivo no destino; falha de disco idem.

## Limites (honestos)

- Proxies nunca entram no export (usa originais). Latência de cancelamento = por frame (4K em SwiftShader: segundos).
- Mix offline ≠ áudio ouvido: a análise espectral prova conteúdo de sinal, não audição.
- Resampler linear para taxas diferentes de 48 kHz (sem filtro anti-aliasing): ok para o conteúdo testado, não medido em música.
- Medidas de diferença preview×export e de memória do Node+Chrome: seção abaixo.
- Não reaproveita export interrompido; sem HDR/HEVC/alpha.

## Posse do renderer (BL144.10)

Um export (e seu preflight) pede o renderer pela política única de [F056](renderer-ownership.md): com outro export (ou preflight) no renderer, responde `renderer_busy`/`retryable` e nada é enfileirado; com um job de miniaturas no renderer, o job é parado no próximo quadro e o export é recusado até a nova tentativa (o renderer fica reservado por 30 s). Escrito, não executado (disco).
