# F042A — Mix offline dirigido no export de cena (BL016)

## Objetivo e limites

`scene.export` ([F042](scene-export.md)) passa a gravar no MP4 **o mix WAV que o bundle declara** em
`audio.{mixRelativePath, editStartSeconds, editEndSecondsExclusive, sourcePcmHashSha256}`, no trecho
pedido, com tempo racional exato. É **processamento real e offline**: o core calcula qual instante do
arquivo cada amostra toca, a mídia lê o WAV aprovado, escreve um stream estéreo 48 kHz e o FFmpeg o
codifica em AAC com a mesma duração do vídeo. Nada é simulado.

- **Semântica dos campos** (a mesma do transporte da origem, `audio-window.ts`): o tempo de edição `t` da
  cena toca o arquivo em `editStartSeconds + t`; nada toca em `editEndSecondsExclusive` ou depois. O trecho
  `[from, from+frames)` lê, portanto, o arquivo a partir de `editStartSeconds + from/scene_rate`.
- **Tempo**: a amostra `n` (48 kHz) da saída lê o instante `editStartSeconds + from/scene_rate + n/48000`.
  Os segundos do bundle viram a fração decimal exata que o autor escreveu (`0.1` é `1/10`); nenhum `f64`
  entra na conta. Fonte em outra taxa (`44100`, `32000`, …) é lida na posição fracionária
  `tempo × rate`, com interpolação linear entre os dois frames vizinhos; em 48 kHz a cópia é exata,
  amostra a amostra.
- **Duração**: o stream tem exatamente `round(frames_saída / fps_saída × 48000)` amostras, igual ao
  vídeo (como no export de projeto, F029). Em conversão de frame rate o vídeo pode passar de meio frame
  além do trecho de edição: essa cauda é **silêncio declarado** (`silence_tail_samples`, aviso
  `audio_silence_tail`), nunca som lido fora da janela.
- **Canais**: mono vira estéreo centrado a −3 dB (`1/√2`, o mesmo do FFmpeg da origem e do mixer do
  projeto); estéreo passa inalterado. Amostras float não finitas viram silêncio; acima de fundo de escala
  são limitadas e contadas (`measured.clipped`).
- **`audio:false`** (ou preset sem áudio) omite o mix com o aviso `audio_omitted` e `audio.declared`/`omitted`
  no resultado, como antes. Sem `audio` declarado: sem stream e aviso `no_audio_sources`.
- **Recusado, nunca aproximado**:
  - trecho fora da janela declarada → `audio_window_short` (o mix pela metade passaria por pronto);
  - segundos que a conta exata não comporta → `audio_window_invalid`;
  - arquivo curto para o trecho → `audio_source_short`;
  - formato fora de WAV PCM16/float32, 1–2 canais, 8–48 kHz (24 bits, ADPCM, surround, MP3, 96 kHz…) →
    `unsupported`, **antes** de existir job. Acima de 48 kHz não há filtro passa-baixa: recusa em vez de
    aliasing;
  - `narration` declarada com áudio pedido → `unsupported` (takes/legendas/ganho por linha ficam em BL017).
- **Identidade**: o arquivo é o aprovado no mapa de recursos. Antes de ler uma amostra o job confere
  tamanho e SHA-256 de novo; bytes trocados depois da aprovação falham com `resource_changed` **antes de
  qualquer frame**. (Cópia imutável durante o job é o escopo de BL109/S3; esta entrega apenas verifica.)
- **Não faz**: tocar som, dispositivo, TTS, mix de narração, ganho de monitor, `mixMuted`, formatos
  comprimidos, sample rate > 48 kHz, ZIP/pacote do áudio (o pacote de recursos já o carrega, F040).

## Contrato do core / biblioteca

`tessel_editor_core::scene_export` (sem gpui/wry; árvore de dependências inalterada):

| Tipo | Papel |
|---|---|
| `AudioPlan::Mix(DirectedAudioMix)` | terceira saída do plano de áudio do snapshot (`NoSources`, `Omitted`, `Mix`); `AudioPlan::mix()` |
| `DirectedAudioMix::plan(&AudioMix, &SceneExportSchedule)` | posiciona o mix no export: `audio_window_invalid`, `audio_window_short` |
| `output_samples / mix_samples / silence_tail_samples / origin()` | tamanho do stream, parte lida do arquivo, cauda de silêncio e segundo do arquivo da amostra 0 (`Ratio`) |
| `cursor(rate) -> SourceCursor`, `SourceCursor::at(n) -> SourcePosition{base, fraction}` | posição exata (inteiros de 128 bits) da amostra `n` em um arquivo de `rate` Hz; `fraction == 0.0` ⇒ cópia exata |
| `check_source(&SourceFacts) -> SourceSpan` | regras do arquivo: canais, taxa, quadros suficientes (`unsupported`, `audio_source_short`) |

`SceneExportSnapshot::capture` chama `DirectedAudioMix::plan` depois do cronograma; `summary()["audio"]` traz
`{included:true, declared:["audio"], mix:{path, sha256, window, source_start, samples, mixed_samples,
silence_tail_samples, sample_rate:48000, channels:2}}`. `SceneExportSpec`/`AudioChoice` não mudam (aditivo).

Mídia (`tessel_editor_media::scene_encoder`): `inspect(path) -> WavLayout` (só cabeçalho, em streaming),
`check_layout(&DirectedAudioMix, path)`, `render_audio(&mix, &ApprovedResource, out.wav, &JobContext) -> MixReport`
(1 s de saída por passo, memória constante; identidade → fatos do cabeçalho → regras do core → amostras) e
`encode_args_with_audio` (`encode_args` é o mesmo comando sem áudio). `SceneExport::freeze` recusa antes do
job o que o cabeçalho já mostra impossível; `preflight` acrescenta bloqueios `encoder_unavailable` (sem `aac`),
`resource_unresolved|resource_missing|resource_changed` (`asset:"audio-mix"`), `unsupported`/`audio_source_short`
e inclui WAV + AAC na estimativa de disco. O job renderiza o mix **antes** dos frames (falha cedo) em
`mix.wav` dentro do staging, que some em sucesso, falha ou cancelamento.

### Fluxo Rust sem UI

```rust
let result = headless::execute_ext(&mut state, None, Some(&mut host), &json!({
    "op":"scene.export","scene":"street","preset":"fullhd","from":36,"frames":24,
    "destination":"/tmp/street.mp4","resources_root":"corpus"}))?;
// result["audio"] = {included:true, declared:["audio"], mix:{samples:48000,…},
//                    source:{sha256, sample_rate, channels, converted_rate, upmixed_mono},
//                    measured:{samples, peak, rms, clipped}}
```

### Resultado

`audio` ganha `source` e `measured` quando o mix foi gravado; `warnings` pode trazer `audio_silence_tail`;
`verification` (ffprobe) confere `aac`, 48 kHz, 2 canais, duração e início iguais aos do vídeo.
Um job que falha reporta o código pelo registro de jobs: códigos sem nome estático aparecem como
`job_failed` com `details.job_code` (ex.: `resource_changed`).

### Erros

| Código | Quando | Efeito |
|---|---|---|
| `audio_window_short` | trecho além de `editEndSecondsExclusive − editStartSeconds` | recusado antes do job; nada muda |
| `audio_window_invalid` | tempo do bundle que a conta exata não comporta | idem |
| `audio_source_short` | o arquivo não tem quadros para o trecho | idem (cabeçalho) ou bloqueio no preflight |
| `unsupported` | formato/canais/taxa fora do suportado; narração com áudio pedido | idem |
| `resource_unresolved`, `resource_missing`, `resource_changed` | mix fora do mapa, sumido ou trocado | bloqueio no preflight; `resource_changed` também falha o job antes dos frames |
| `encoder_unavailable` | FFmpeg sem `aac` | bloqueio no preflight |

## CLI headless

`./scripts/render` (`tessel-render`) com a mesma operação, sem flag nova:

```text
{"op":"scene.attach","id":"sound","path":"corpus/bundle.audio.json"}
{"op":"scene.export","scene":"sound","preset":"fullhd","from":36,"frames":24,
 "destination":"/tmp/out.mp4","resources_root":"corpus"}
{"ok":true,"result":{"audio":{"included":true,"mix":{"samples":48000,…}},"verification":{"ok":true},…}}
```

## UI e CLI da janela

O diálogo `flow.export_scene` já tinha o campo `audio` (`yes|no`): `yes`/omitido agora inclui o mix, `no` o omite
com aviso. A mudança de UI é só o rótulo do campo e a tradução do pedido (teste de unidade em `flows.rs`); o
pedido é o mesmo `scene.export` dos outros adaptadores. **Sem prova viva na janela** com mix (ver limites).

## Evidências e status

Status: **a validar na integração** (implementado, com as provas abaixo; sem prova viva da janela).

| Superfície | Evidência |
|---|---|
| Lib (core) | `crates/core/tests/scene_export.rs` (20) e unidade `scene_export::audio` (4): plano padrão com mix, `audio:false`/preset mudo, offset de janela (`10–14 s`, `from=24`), taxa 24→30 (48 000 amostras), cauda de 800 amostras (2 frames → 3), NTSC `30000/1001` (48 048), janela curta com números, decimal exato, narração recusada |
| Lib (mídia, FFmpeg real, sem browser) | `crates/media/tests/scene_export_audio.rs` (15): amostras **exatas** em 48 kHz (cópia = fatia da fonte), offset, 44,1 kHz mono (interpolação calculada no teste, ±1 LSB), cauda silenciosa, NaN/∞/clipping, bytes trocados (`resource_changed`), 8 arquivos/formatos recusados (`unsupported`) mais curto e ausente com código próprio; MP4 pelo host com espectro 440→880 Hz medido no AAC decodificado nos lugares certos, 24→30 + 44,1 kHz, offset de janela, `audio:false`, recusas antes de job/frame, troca de bytes falha com 0 frames desenhados, preflight de mix sumido |
| CLI headless real (Chrome) | `crates/render/tests/scene_export.rs`: `the_headless_binary_mixes_the_declared_wav_at_the_requested_range` (JSONL, frames reais do runtime, 440→880 Hz medido) e o teste de política de áudio atualizado |
| Oráculo independente da R10 | `scripts/directed-scene-export-proof/proof.py --phases export_audio_policy`: 17/17 (uma stream de áudio, metade 440 Hz e metade 880 Hz, duração do áudio = vídeo ± 50 ms), resultado `outcome: mixed` |
| UI | só rótulo + `flows.rs` (unidade); janela real com mix **não provada** |

### Limites declarados

- AAC é com perda: o espectro e a duração são medidos, a amostra exata é provada no stream que entra no
  encoder (`mix.wav`), não no MP4.
- Conversão de taxa é interpolação linear (sem passa-baixa); fonte > 48 kHz é recusada.
- Mono com −3 dB (convenção da origem/FFmpeg); a origem usa `atrim` sem teto em `editEnd` no render Node,
  enquanto o transporte web para nele — aqui o teto é regra (recusa trecho fora da janela).
- A identidade é checada por SHA-256 no início do job, não por cópia imutável: uma troca *durante* o job
  continua em BL109 (S3).
- Narração, takes, `mixMuted`, ganho de monitor, reprodução em dispositivo e TTS: fora (BL017/BL046).
- Códigos novos de job aparecem como `job_failed` + `details.job_code` (mapeamento estático em `jobs.rs`, fora
  desta entrega).
- O harness da R10 (`proof.py`, fase `ui_export_flow`) ainda espera a recusa do padrão `audio=yes`; com o mix
  entregue essa fase muda de resultado e é ajuste da frente S10.
