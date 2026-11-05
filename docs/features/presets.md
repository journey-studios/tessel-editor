# F028 — Presets de exportação (P11)

## Status

Lib **V** (`crates/core/src/preset.rs` + testes), headless **V**, UI **V** (seletor de preset no diálogo Exportar).

## Contrato

Preset versionado (`version: 1`) que diz **o que** produzir; o adapter decide **como** com o FFmpeg que
encontrar e reporta blocker se não puder. Containers/codecs desconhecidos são **rejeitados**, não ignorados.

```json
{"version":1,"id":"fullhd","name":"Full HD 1080p","builtin":true,"width":1920,"height":1080,
 "fps":null,"fit":"contain","container":"mp4",
 "video":{"codec":"h264","crf":18,"pix_fmt":"yuv420p"},
 "audio":{"enabled":true,"codec":"aac","bitrate_kbps":192,"sample_rate":48000},
 "color":{"space":"rec709","range":"tv"}}
```

Embutidos (somente leitura): `fullhd` 1920×1080, `uhd4k` 3840×2160, `vertical` 1080×1920, `square` 1080×1080.
Personalizado: `preset.save` com o objeto acima (`id` 1–48 `[A-Za-z0-9_-]`, nome ≤ 80, lados **pares** de 16 a 8192,
`crf` 1–51, `audio_kbps` 32–512, `fps` ≤ 240 ou `null` = usa o da sequência; `fit` `contain|cover`).

| `op` | Campos | Erros |
|---|---|---|
| `preset.list` | — | embutidos + do usuário |
| `preset.save` | `preset` | `invalid_preset`, `invalid_value` (com `details` min/max), `unsupported_schema` |
| `preset.delete` | `id` | `unknown_element` (inclui os embutidos: só existem presets do usuário no documento); limpa a seleção se era o removido |
| `preset.select` | `id` | `unknown_element` |

Presets do usuário e o selecionado vivem no documento (F022) e são **desfazíveis**.

## Limites

Só H.264/AAC/MP4 SDR Rec.709 sem alpha. HEVC, HDR e ProRes ficam para depois. CRF 0 (lossless) não é aceito
(exigiria 4:4:4).
