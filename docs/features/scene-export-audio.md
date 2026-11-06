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
  Os segundos do bundle viram a fração decimal que o autor escreveu (`0.1` é `1/10`), **arredondada a 12 casas**
  (1 ps, erro ≤ `5e-13` s ≈ `2,4e-8` de amostra) sobre os próprios dígitos do texto `f64`, empate para longe de zero,
  e valem abaixo de `1e9` s (nenhum WAV de ≤ 4 GiB alcança isso: 268 435 s a 8 kHz mono); nenhum `f64` entra na
  conta. A aritmética é de frações em `i128` com cada operação verificada (`audio_window_invalid` se não couber, nunca
  valor truncado): `1/24`, `1/3` e `1/960` s como início de janela e trechos a milhares de segundos funcionam (BL016.6;
  o limite antigo de `i64` recusava `1/24` a partir de 74 s e `1/960` em qualquer duração). Um início não decimal cai
  a < `2,4e-8` de amostra do instante pretendido: a leitura é interpolada com esse resíduo (sub-LSB), não é cópia
  bit a bit. Janela e trecho são comparados **na grade de 48 kHz, em inteiros** (`round(t × 48000)`,
  empate para longe de zero): `8/24 s` é `1/3`, que nenhum decimal escreve, e o `0.3333333333333333` do autor
  são as mesmas 16 000 amostras, enquanto uma janela com 1 amostra a menos (`15999/48000`) continua recusada
  (BL016.6). Fonte em outra taxa (`44100`, `32000`, …) é lida na posição fracionária
  `tempo × rate`, com interpolação linear entre os dois frames vizinhos; em 48 kHz a cópia é exata,
  amostra a amostra.
- **Duração**: o stream tem exatamente `round(frames_saída / fps_saída × 48000)` amostras, igual ao
  vídeo (como no export de projeto, F029). Em conversão de frame rate o vídeo pode passar de meio frame
  além do trecho de edição: essa cauda é **silêncio declarado** (`silence_tail_samples`, aviso
  `audio_silence_tail`), nunca som lido fora da janela.
- **Canais**: mono vira estéreo centrado a −3 dB (`1/√2`: lei de **potência constante**, a do mixer do
  projeto e a que esta entrega escolheu; ver "Lei do mono × oráculo da S10" abaixo); estéreo passa inalterado. Amostras float não finitas viram silêncio; acima de fundo de escala
  são limitadas e contadas (`measured.clipped`). PCM16 é decodificado por `/32768` e codificado por `×32768`
  (o inverso exato): um PCM16 estéreo a 48 kHz sai **bit a bit** igual em toda a faixa, extremos `-32768`/`32767`
  incluídos (BL016.8); só uma fonte float mapeia fundo de escala para `32767`, como o mixer do projeto.
- **`audio:false`** (ou preset sem áudio) omite o mix com o aviso `audio_omitted` e `audio.declared`/`omitted`
  no resultado, como antes. Sem `audio` declarado: sem stream e aviso `no_audio_sources`.
- **Recusado, nunca aproximado**:
  - trecho fora da janela declarada → `audio_window_short` (o mix pela metade passaria por pronto);
  - segundos que a conta exata não comporta → `audio_window_invalid`;
  - arquivo que não cobre a **janela declarada**, mesmo quando cobre o trecho pedido → `audio_source_short`
    (`details.rule: "window"`); o trecho que ultrapassa o próprio arquivo é a mesma recusa com
    `details.rule: "range"` (BL016.7, política abaixo);
  - mix acima de **6 h** (`6 × 3600 × 48000` amostras, o teto do export de projeto; também é o que cabe no
    `data` de 32 bits do WAV, 4,147 GB < 4 GiB) → `audio_too_long`, **ao planejar**, antes de existir job,
    staging ou byte gravado (BL016.10); o job ainda recusa o que um chamador sem plano trouxesse;
  - formato fora de WAV PCM16/float32, 1–2 canais, 8–48 kHz (24 bits, ADPCM, surround, MP3, 96 kHz…) →
    `unsupported`, **antes** de existir job. Acima de 48 kHz não há filtro passa-baixa: recusa em vez de
    aliasing;
  - `narration` declarada com áudio pedido → `unsupported` (takes/legendas/ganho por linha ficam em BL017).
- **Identidade e bytes congelados (BL109, S3)**: o arquivo é o aprovado no mapa de recursos e o job o trata como
  qualquer outro recurso exigido: `ResourceKey::AudioMix` entra nas chaves da cópia privada
  (`ResourceSnapshot::take`), que lê o WAV **uma vez**, calcula o SHA-256 na mesma passada que grava a cópia
  `0444` no staging e compara com o hash e o tamanho aprovados. A mistura lê **somente essa cópia**
  (`render_audio_private`); o original nunca é relido durante o job. Consequências:
  - troca de mesmo tamanho depois do `resolve` → `resource_changed` (`details.resource: "audio-mix"`, `expected` e
    `got` com SHA-256 e bytes) **antes de qualquer frame** e antes de qualquer amostra;
  - troca **depois do pedido aceito** que mude o cabeçalho (outro formato, canais, taxa, duração, nem WAV, outro
    tamanho) → o preflight do job compara o cabeçalho com o que o pedido aceitou (`freeze`) e falha com o mesmo
    `resource_changed` (`details.resource: "audio-mix"`, `blockers[0]` com o motivo), nunca com o veredito do
    cabeçalho novo (`unsupported`, `audio_source_short`); antes do pedido, o veredito do cabeçalho vale e o pedido é
    recusado com ele (`unsupported`/`audio_source_short`), sem job (BL016.9);
  - alteração do original **durante** o job fica isolada: o MP4 reflete os bytes aprovados;
  - o job confere a cópia (`verify_intact`: tamanho e mtime) depois de misturar e antes de finalizar;
  - sucesso, falha e cancelamento (no meio da cópia, logo depois dela e no meio do mix) removem só o staging do próprio
    job (cópia e `mix.wav` incluídos); staging alheio e o original nunca são tocados. Cada ponto é provado com o job
    **retido** nele (ver "Pontos de observação" abaixo), não por tempo.
- **Não faz**: tocar som, dispositivo, TTS, mix de narração, ganho de monitor, `mixMuted`, formatos
  comprimidos, sample rate > 48 kHz, ZIP/pacote do áudio (o pacote de recursos já o carrega, F040).

## Contrato do core / biblioteca

`tessel_editor_core::scene_export` (sem gpui/wry; árvore de dependências inalterada):

| Tipo | Papel |
|---|---|
| `AudioPlan::Mix(DirectedAudioMix)` | terceira saída do plano de áudio do snapshot (`NoSources`, `Omitted`, `Mix`); `AudioPlan::mix()` |
| `DirectedAudioMix::plan(&AudioMix, &SceneExportSchedule)` | posiciona o mix no export: `audio_window_invalid`, `audio_window_short` |
| `output_samples / mix_samples / silence_tail_samples / origin()` | tamanho do stream, parte lida do arquivo, cauda de silêncio e segundo do arquivo da amostra 0 (`Option<Ratio>`: `None` só se a fração reduzida passar de `i64`; o cursor e o JSON a trazem inteira) |
| JSON do tempo exato (`num`, `den`, `seconds`) | `num`/`den` são números JSON até `i64` e **texto decimal** (string) acima disso, por exemplo início `1/3` a 29,97 fps depois de ~10 000 s; `seconds` é `f64`. Nenhum consumidor deste repositório lê esses campos e a API Rust não muda; quem os ler de fora deve aceitar número ou string |
| `cursor(rate) -> SourceCursor`, `SourceCursor::at(n) -> SourcePosition{base, fraction}` | posição exata (inteiros de 128 bits) da amostra `n` em um arquivo de `rate` Hz; `fraction == 0.0` ⇒ cópia exata |
| `check_source(&SourceFacts) -> SourceSpan` | regras do arquivo: canais, taxa, cobertura da janela declarada (tolerância de 1 ms da origem) e do trecho (`unsupported`, `audio_source_short`, `details.rule`) |
| `MAX_MIX_SECONDS` | teto de 6 h do mix; `plan` recusa acima com `audio_too_long` |

`SceneExportSnapshot::capture` chama `DirectedAudioMix::plan` depois do cronograma; `summary()["audio"]` traz
`{included:true, declared:["audio"], mix:{path, sha256, window, source_start, samples, mixed_samples,
silence_tail_samples, sample_rate:48000, channels:2}}`. `SceneExportSpec`/`AudioChoice` não mudam (aditivo).

Mídia (`tessel_editor_media::scene_encoder`): `inspect(path) -> WavLayout` (só cabeçalho, em streaming),
`check_layout(&DirectedAudioMix, path)`, `render_audio(&mix, &ApprovedResource, out.wav, &JobContext) -> MixReport`
(1 s de saída por passo, memória constante; identidade → fatos do cabeçalho → regras do core → amostras) e
`encode_args_with_audio` (`encode_args` é o mesmo comando sem áudio). `SceneExport::freeze` recusa antes do
job o que o cabeçalho já mostra impossível; `preflight` acrescenta bloqueios `encoder_unavailable` (sem `aac`),
`resource_unresolved|resource_missing|resource_changed` (`asset:"audio-mix"`), `unsupported`/`audio_source_short`
e inclui na estimativa de disco o WAV misturado + AAC (`estimate.audio_mix_bytes`) **e** a cópia privada do arquivo
(`estimate.resource_copy_bytes`, parte de `staging_bytes`). O preflight só faz `stat` e lê o cabeçalho; a cópia e a
mistura acontecem no job (thread própria, estágios `snapshot` e `audio` com progresso, canceláveis), nunca na UI.
O job copia, **depois** mistura a cópia (falha cedo, antes dos frames) em `mix.wav` dentro do staging, que some em
sucesso, falha ou cancelamento. `render_audio` (identidade por SHA-256 e leitura do caminho recebido) segue público;
`render_audio_private` é o mesmo stream para o arquivo que já é a cópia do job.

### Pontos de observação do job (testes determinísticos)

`SceneFrameRenderer` tem três ganchos com implementação vazia por padrão (mais o `max_size`, que já existia); o job os
chama em pontos exatos e **nada do que um gancho faz alcança os bytes que o job lê** (o renderer real não os implementa):

| Gancho | Quando | Uso nos testes |
|---|---|---|
| `resources_frozen()` | a cópia privada terminou e nada foi lido dela | reescrever o original; reter o job antes do 1º passo do mix |
| `copy_progressed(bytes)` | depois de cada pedaço (1 MiB) gravado na cópia, estágio `snapshot`; as imagens do corpus (4–9 KiB) chegam inteiras de uma vez, só o WAV reporta `bytes == 1 MiB` | reter o job dentro da cópia **do WAV** (primeiro pedaço cheio) |
| `mix_stepped(done, steps)` | depois de cada passo de 1 s do mix gravado e contado, estágio `audio` | reter o job entre dois passos |
| `max_size()` (já existia) | primeira conversa do job com o renderer, antes do preflight dele | trocar o arquivo depois do pedido aceito |

Reter o job é determinístico: enquanto parado, `job.get` é fato (`stage`, `done`/`total`) e o staging tem o que o ponto
implica; o teste então cancela e libera. Um job cancelado termina sempre com `stage == "cancelled"` (o registro o
escreve), então o estágio não prova **onde** parou: o teste compara `done`/`total` finais com os lidos na barreira
(`finish` não os altera ao cancelar), e qualquer cópia, passo de mix ou estágio seguinte depois do cancelamento os mudaria.
`ParkCopyChunk` retém só em `bytes == 1 MiB`, portanto dentro do WAV (`done == imagens + 1 MiB`, `total == imagens +
WAV`) e com 1 MiB do WAV em disco. Ganchos que **falham** ou **entram em pânico** (`resources_frozen`, `mix_stepped`)
terminam o job tipado (`render_failed`) ou como `job_failed` (o `catch_unwind` do registro) e deixam só o staging alheio. Substitui a versão anterior, que consultava `stage == "snapshot"` com 46 MiB e
dependia da velocidade do disco (o cancelamento podia chegar depois da cópia). O que isto **não** prova: nenhum
cancelamento durante um único `read`/`write` do SO nem com o disco lento; prova que cada estágio observa o cancelamento
entre as unidades e que o `Staging` limpa só o que é do job, qualquer que seja o ponto.

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
`snapshot {files, bytes, verified}` inclui o WAV. Um job que falha reporta o código **tipado** (BL016.4):
`From<JobError> for CommandError` nomeia `resource_changed`, `resource_missing`, `unsupported`, `audio_source_short`,
`audio_window_invalid`, `audio_window_short`, `audio_too_long` e os erros do resolvedor
(`scene_resources_not_loaded`, `resource_unknown`, `resource_not_carried`, `stale_generation`,
`resource_budget_exceeded`); só o que não tem nome vira `job_failed` + `details.job_code`.

### Erros

| Código | Quando | Efeito |
|---|---|---|
| `audio_window_short` | trecho além de `editEndSecondsExclusive − editStartSeconds` | recusado antes do job; nada muda |
| `audio_window_invalid` | tempo do bundle que a conta exata não comporta | idem |
| `audio_source_short` | o arquivo é menor que `editEndSecondsExclusive` (a origem tolera 1 ms) ou não tem quadros para o trecho | idem (cabeçalho) ou bloqueio no preflight |
| `audio_too_long` | mix de mais de 6 h | recusado ao planejar; nada muda, nenhum byte gravado |
| `unsupported` | formato/canais/taxa fora do suportado; narração com áudio pedido | idem |
| `resource_unresolved`, `resource_missing`, `resource_changed` | mix fora do mapa, sumido ou trocado | bloqueio no preflight (mix sumido: job `preflight_failed` com `blockers[0].code: resource_missing`); `resource_changed` falha o job antes dos frames com o código tipado: com as duas identidades quando a troca é de mesmo tamanho e mesmo cabeçalho (estágio `snapshot`), com `blockers[0]` quando o tamanho ou o cabeçalho mudou desde o pedido (preflight) |
| `disk_full` | staging sem espaço para vídeo + WAV + AAC + cópias | bloqueio no preflight, antes de escrever |
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

O diálogo `flow.export_scene` tem o campo `audio` (`yes|no`): `yes`/omitido inclui o mix, `no` o omite com aviso.
A mudança de UI é só o rótulo do campo e a tradução do pedido (teste de unidade em `flows.rs`); o pedido é o
mesmo `scene.export` dos outros adaptadores. A prova na janela real está em
[`scripts/scene-export-audio-ui-proof.py`](../../scripts/scene-export-audio-ui-proof.py): instância própria (binário
direto, HOME curto, socket próprio), somente `get`/`set`/`invoke`/`op` (sem ponteiro nem coordenadas), sob o lock de GPU.

## Evidências e status

Status: **a validar na integração** (implementado, sem som ouvido). A rodada de correção da revisão independente
(F1–F7 da primeira, N1–N5 e F7 da segunda; BL016.6–.11) foi **executada** neste head: os testes e as mutações abaixo
rodaram (`TESSEL_REQUIRE_RUNTIME=1`); os gates de workspace e a prova nativa constam em BL016 e na PR.

| Rodada de correção (BL016.6–.11) | Executado |
|---|---|
| Core, unidade `scene_export::audio` (11) | `a_window_is_compared_on_the_48k_grid_not_as_float_seconds` (1–2000 quadros a 24 fps escritos como `f64`, 1 amostra a menos); `a_window_that_starts_on_a_non_decimal_instant_plans_whatever_its_length` (início `1/24`, `1/12`, `1/6`, `1+1/24`, `1/3`, `1/960` com 1 s a 6 h; 1 amostra a menos recusada); `a_non_decimal_start_and_a_late_range_place_the_first_sample_exactly` (`1/3` com `from` > 307 s, 29,97 fps com início `1/3` a 10 000 s, NTSC, `1e30` recusado tipado); `a_decimal_keeps_twelve_places_and_rounds_the_rest_on_its_digits` (1e-25 → 0, empate 5e-13, teto de 1e9 s); `exact_arithmetic_reports_overflow_instead_of_wrapping`; `a_source_shorter_than_the_window_is_refused_even_when_the_range_fits`; `six_hours_of_audio_plan_and_one_frame_more_is_refused_before_any_write`; e os 4 anteriores |
| Core, integração `scene_export` (21) | `a_window_written_as_the_float_of_the_range_is_that_range_and_one_sample_less_is_not`; `origin()` agora `Option<Ratio>` |
| Mídia, FFmpeg `scene_export_audio` (29) | N1: `cancel_parked` compara `stage == "cancelled"` e `done`/`total` finais com os da barreira; N3: `a_cancel_in_the_middle_of_the_copy_…` retém no primeiro pedaço de 1 MiB **do WAV** (`done == imagens + 1 MiB`, `total == imagens + WAV`, 1 MiB do WAV em disco); `a_cancel_right_after_the_copy_…` (cópia completa, sem `mix.wav`); `a_cancel_in_the_middle_of_the_mix_…` (2 de 4); N5: `a_hook_that_fails_or_panics_ends_the_job_typed_and_leaves_nothing_but_the_foreign_staging` (4 casos: `Err` e pânico em `resources_frozen` e em `mix_stepped`; código, estágio/`done` do registro, só o staging alheio, original intacto, 0 frames, nada publicado); mais os 24 anteriores |
| Mutações (aplicadas, executadas e revertidas) | M1 sem `token.check()` em `copy_hashed` → o teste do meio da cópia falha; M2 sem o `ctx.check()` inicial do mix → o teste pós-cópia falha; M3 sem `ctx.check()` no laço do mix → o teste do meio do mix falha; M4 erro de `resources_frozen` ignorado e M5 erro de `mix_stepped` ignorado → o teste de ganchos falha; M6 `Exact` limitado a `i64` → 2 testes falham; M7 arredondamento `>` em vez de `>=` e M8 sem quantização → o teste de 12 casas falha; M9 (M6 + M8, o comportamento anterior) → 4 testes de unidade falham. Antes da correção, os dois testes de início não decimal falhavam no código anterior (`audio_window_invalid`, 1/24 e `from` 7371) |

Mutações **não** rodadas nesta rodada (da lista da rodada anterior): escala `32767` no PCM16 e `render_private` lendo o original
(F3/F4, cobertas pelos testes do head anterior, não repetidas aqui); remover o `catch_unwind` do registro não foi tentado
(o job ficaria `running`; o teste de pânico prova o caminho com o `catch_unwind` presente).

| Superfície | Evidência |
|---|---|
| Lib (core) | `crates/core/tests/scene_export.rs` (21) e unidade `scene_export::audio` (11): plano padrão com mix, `audio:false`/preset mudo, offset de janela (`10–14 s`, `from=24`), taxa 24→30 (48 000 amostras), cauda de 800 amostras (2 frames → 3), NTSC `30000/1001` (48 048), janela curta com números, decimal exato, narração recusada |
| Lib (mídia, FFmpeg real, sem browser) | `crates/media/tests/scene_export_audio.rs` (29, contando os de bytes congelados abaixo): amostras **exatas** em 48 kHz (cópia = fatia da fonte), offset, 44,1 kHz mono (interpolação calculada no teste, ±1 LSB), cauda silenciosa, NaN/∞/clipping, bytes trocados (`resource_changed`), 8 arquivos/formatos recusados (`unsupported`) mais curto e ausente com código próprio; MP4 pelo host com espectro 440→880 Hz medido no AAC decodificado nos lugares certos, 24→30 + 44,1 kHz, offset de janela, `audio:false`, recusas antes de job/frame, troca de bytes falha com 0 frames desenhados, preflight de mix sumido |
| CLI headless real (Chrome) | `crates/render/tests/scene_export.rs`: `the_headless_binary_mixes_the_declared_wav_at_the_requested_range` (JSONL, frames reais do runtime, 440→880 Hz medido) e o teste de política de áudio atualizado |
| Oráculo independente da R10 | `scripts/directed-scene-export-proof/proof.py --phases export_audio_policy`: 17/17 (uma stream de áudio, metade 440 Hz e metade 880 Hz, duração do áudio = vídeo ± 50 ms), resultado `outcome: mixed` |
| Bytes congelados do mix (BL109) | `crates/media/tests/scene_export_audio.rs` (subconjunto BL109 dos 29): troca de mesmo tamanho → `resource_changed` **tipado** com as duas identidades e 0 frames; original reescrito no frame 0 não muda o AAC (440→880 Hz medidos, `source.sha256` = aprovado, `snapshot.bytes` ≥ WAV); `render_audio_private` sobre a cópia dá as amostras exatas depois de reescrever o original (e `render_audio` ainda recusa o original); mix sumido → `preflight_failed`/`resource_missing` e só o staging próprio some; cancelamento no meio dos frames limpa só o próprio staging; estimativa: `resource_copy_bytes` e `audio_mix_bytes` somam o WAV, e `jobs.rs` testa o mapeamento tipado |
| UI (janela real, instância própria) | `scripts/scene-export-audio-ui-proof.py` **45/45**: árvore expõe `flow.export_scene`; `audio` oferece yes/no com padrão yes; `dialog.submit` com `audio=yes` cria 1 job; MP4 com AAC 48 kHz estéreo, duração do áudio = vídeo (±60 ms), 440 Hz em 0,05–0,40 s e 880 Hz em 0,60–0,95 s no decodificado; `snapshot` inclui o WAV; `audio=no` exporta sem stream e com `audio_omitted`; troca de mesmo tamanho (WAV válido) → job `failed` com `resource_changed`, `details.resource=audio-mix`, sem MP4 e sem staging, visível em `get jobs`; mix que deixou de ser WAV → submit recusado (`unsupported`) com diálogo aberto, sem job nem arquivo; `audio=no` ainda exporta com o mix inutilizável |

### Limites declarados

- AAC é com perda: o espectro e a duração são medidos, a amostra exata é provada no stream que entra no
  encoder (`mix.wav`), não no MP4.
- Conversão de taxa é interpolação linear (sem passa-baixa); fonte > 48 kHz é recusada.
- Mono com −3 dB: ver "Lei do mono" abaixo. A origem usa `atrim` sem teto em `editEnd` no render Node,
  enquanto o transporte web para nele — aqui o teto é regra (recusa trecho fora da janela).
- **Política da fonte curta (BL016.7, decidida pela origem).** `node/render.ts` (`muxAudio`) recusa com `AUDIO_SHORT`
  quando `duration(fonte) + 0.001 < editEndSecondsExclusive`, e o `renderBundle` da origem só renderiza o bundle
  **inteiro** (`RenderOptions` não tem `from`/`frames`). O trecho parcial é uma extensão deste editor: ele **herda a
  mesma recusa** em vez de aceitar sub-trechos de um bundle que a origem recusaria. A regra é exata em inteiros
  (a tolerância de 1 ms vale como no original, não é epsilon de float). Adicionalmente, o trecho nunca lê além do
  arquivo (`audio_source_short`, `details.rule: "range"`): amostras não são inventadas dentro da tolerância.
  Consequência para o corpus da S10 (WAV de 8 s, janela `[1, 9)`): recusado, como a origem o recusaria.
- **Divergência declarada com a origem na duração da janela (BL016.11; decisão do coordenador, sem paridade completa).**
  No pin `d6adac15`, `node/render.ts` (`muxAudio`) recusa com `AUDIO_VIDEO_DURATION` quando
  `|(editEndSecondsExclusive − editStartSeconds) − duração da edição| > 0,001` (tolerância de 1 ms nos dois sentidos), e
  `createStudioSession.ts` só recusa a janela **mais curta** que a edição além de 1 ms. Esta entrega mantém o contrato
  de 1 amostra exata e **não** implementa essa regra:
  - janela até 1 ms mais curta que a edição — `[0, 0.333)` com 8 frames a 24 fps (edição de `0,3333…` s, 0,33 ms a
    menos): a origem aceita dentro da tolerância; este editor recusa com `audio_window_short` o trecho que chega ao fim
    da edição (15 984 amostras contra 16 000);
  - janela mais curta que a edição por mais de 1 ms — `[0, 2)` com 96 frames a 24 fps (edição de 4 s): o `render.ts` da
    origem recusa com `AUDIO_VIDEO_DURATION`; este editor exporta qualquer trecho que caiba na janela (`fits.mp4` nos
    testes) e recusa só o que a ultrapassa;
  - janela mais longa que a edição por mais de 1 ms: a origem recusa no render Node (a sessão do estúdio a aceita); este
    editor exporta a edição inteira, pois o trecho cabe na janela;
  - a origem julga o bundle inteiro, este editor julga o trecho pedido.
  Não é epsilon de `f64` e não é "paridade completa": é o contrato atual, escolhido, e registrado em BL016.11.
- A cópia privada dobra o espaço do WAV no staging e custa uma leitura + escrita antes do primeiro frame (como as
  imagens, BL109); `verify_intact` não refaz o hash. O original que mudou **depois** da cópia não é detectado:
  é isolado, e o vídeo reflete os bytes aprovados.
- A prova da janela é semântica (get/set/invoke): não prova que uma pessoa vê ou alcança o controle (BL043), nem som
  ouvido (BL046). O `--min-free-gib` do script existe porque a máquina de desenvolvimento é compartilhada; a prova
  escreve < 200 MiB.
- Narração, takes, `mixMuted`, ganho de monitor, reprodução em dispositivo e TTS: fora (BL017/BL046).
- Precisão do tempo declarado: 12 casas decimais (ver "Tempo"); um bundle que escreva um tempo abaixo de `1e-12` s o vê
  tornar-se zero, e tempos de `1e9` s ou mais são `audio_window_invalid`. A métrica `peak`/`rms` é de `f32` e uma fonte
  float perto de `f32::MAX` interpolada pode dar `inf` na métrica; o áudio continua limitado a fundo de escala (limite
  conhecido, sem feature nova).
- O harness da R10 (`proof.py`, fase `ui_export_flow`) ainda espera a recusa do padrão `audio=yes`; com o mix
  entregue essa fase muda de resultado e é ajuste da frente S10 (não alterado aqui).

### Aceite independente da S10 (PR #8, `proof.py` em `c03b41e`) — divergência de semântica, não de qualidade

Rodado com o binário da PR #11 antes das correções desta rodada, sem alterar verificadores nem tolerâncias, sob o lock de GPU:
`tolerances_frozen` 6/6, `corpus` 32/32, `controls_audio` 26/26, `baseline_audio` 46/46, `audio_off` 70/70 (o
`audio:false` e a política de aviso passam no corpus da S10); `audio_export` **FAIL** e `audio_package_export`
**FAIL/abortado**. Esse FAIL é o resultado registrado daquela rodada; **não** é PASS, e nenhuma execução posterior
a reverteu: o aceite vivo da S10 segue bloqueado até a correção independente do oráculo e a integração. A causa é a
**suposição A1** do oráculo da S10 (`oracle.py`: "mix[0] ↔ editStartSeconds", o arquivo começa em `editStartSeconds`
da linha do tempo e antes disso há silêncio), que contradiz a origem:

- origem (`packages/scene-engine/src/transport/audio-window.ts`): `editSecondsToSourceTime(t) = editStartSeconds + t`,
  o WAV é a **fonte inteira** e a janela escolhe um trecho dela; `node/render.ts` recusa com `AUDIO_SHORT` quando
  `duration(source) + 0.001 < editEndSecondsExclusive`. É o que esta entrega implementa.
- o `mix.wav` do corpus da S10 tem 8 s com janela `[1, 9)`: pela regra da origem ele é **curto** e o bundle
  seria recusado pela própria origem (`8 + 0.001 < 9`).

Medido de forma independente (ffmpeg decodifica o MP4 do `audio24_window_a`, janelas de 100 ms): 440 Hz de 0 a
0,25 s, silêncio até 0,75 s, 880 Hz até 2,25 s, 1320 Hz a −6 dB depois — exatamente o que a regra da origem prevê
para `from=18` (0,75 s) com `editStart=1,0` (arquivo em 1,75 s…); o oráculo da S10 espera o contrário (silêncio, 440 Hz
em 0,25 s). Essa medição é do binário anterior às correções desta rodada; com BL016.7 aquele bundle (arquivo de
8 s, janela `[1, 9)`) é recusado por inteiro com `audio_source_short`, como a origem o recusaria.

**Decidido (BL016.5 — fonte).** A leitura do pin da origem `d6adac15c61f4ef582a7ec259397b2e025d1f059`
(`git show`, só leitura) confirma `packages/scene-engine/src/transport/audio-window.ts` →
`editStartSeconds + editSeconds` e `node/render.ts` `muxAudio` → `AUDIO_SHORT` quando `duration + 0.001 <
editEndSecondsExclusive`. Fica a regra da origem; o core não muda para fazer uma fixture errada passar, e esta
frente não altera o harness da S10 nem as tolerâncias. **Pendente (BL016.5 — emenda do oráculo, S10/PR #8):** corrigir
A1 e o WAV do corpus (um arquivo que cubra `[0, editEnd)` com o tom no tempo de arquivo, ou janela coerente com ele) e
repetir `audio_export`/`audio_package_export`; até lá os FAIL acima ficam registrados como estão. A prova da janela
(`scripts/scene-export-audio-ui-proof.py`) usa o `bundle.audio.json` com janela `[0, 4)` e um WAV de 4 s, onde as duas
semânticas coincidem e a fonte cobre a janela.

### Lei do mono × oráculo da S10 (aviso ao coordenador/S10; sem paridade inventada)

Esta entrega converte mono em estéreo com ganho `1/√2` (−3 dB) por canal. **Referência da origem:** o grafo de áudio
(`packages/scene-engine/src/node/audio-graph.ts`, `aformat=…:channel_layouts=stereo`) deixa o upmix ao rematrix padrão
do FFmpeg, que usa o nível de centro `1/√2`. Medido com o FFmpeg local sobre o mesmo filtro: mono
`10000, 20000, -20000, 30000` → estéreo `7071, 14142, -14142, 21213` em ambos os canais. O mixer do projeto
(`crates/core/src/audio.rs`) usa pan de potência constante, que no centro dá o mesmo ganho (`cos(π/4) = 1/√2`). Um oráculo que espere 0 dB no mono (cópia do valor nos dois canais)
descreve outra lei que não é a da origem nem a deste editor; ajustar esse oráculo (ou documentá-lo como contrato novo,
o que mudaria o core e este texto) é decisão da S10, não parte desta frente. Teste desta lei:
`a_44k_mono_source_is_interpolated_at_exact_positions_and_centred`.

**Gate da rodada de correção da revisão R2** (código no head `6e5f3f3`, árvore limpa; os commits seguintes só mudam documentação).
Com `TESSEL_REQUIRE_RUNTIME=1`, Chrome real e sob o lock de GPU, executados por esta frente (nenhum número atribuído):
`cargo fmt --all -- --check`, `check --workspace --all-targets --locked`, `clippy --workspace --all-targets --locked -- -D warnings`
(um `manual_contains` de teste da rodada anterior, nunca compilado até então, corrigido), `test --workspace --locked --no-fail-fast`
(**1210 passed, 0 failed, 9 ignored**: core lib 225, `scene_export` do core 21, mídia `scene_export` 11 e `scene_export_audio` 29, render
`scene_export` 12, desktop 201; os `ignored` são as medições longas/ZIP64 e as fases de aceite que exigem o lock explícito),
`build -p tessel-editor --locked`; `cargo tree -p tessel-editor-core` sem gpui nem wry. Não rodei `./scripts/runtime test` (o
runtime Three.js não mudou nesta rodada; o gate de integração anterior registra 93/93). Prova nativa repetida no head limpo,
sem ponteiro: `scripts/scene-export-audio-ui-proof.py` **45/45** sobre `target/debug/tessel-editor`
(SHA-256 `bb35d75ac8f905a6c73b070f5cf079071c91b9a61b3d25b9cf99ff2b845d2685`; relatório JSON, que não traz o head, SHA-256
`6521c4a5c21f05436a9a4dc9bea3c2ddbee2e96bc9b2fab87909c91c5ca13afd`, igual ao anterior porque só lista os checks). O S10 vivo
**não** foi rodado: o FAIL registrado de `audio_export`/`audio_package_export` continua FAIL (BL121).

**Gate integrado pós-`main` `141259e` (PR #12 mergeada), cabeça `31e0c34`** (`697ab0a` + `main` `141259e`; árvore
`875498bff7c630d3d02f282c9671b4febcff6ba3`, limpa em cada gate). O merge só se sobrepõe a `docs/backlog.md` e
`docs/features/README.md` (linhas S6/F034 da `main` e S9/F042A desta PR preservadas). Executado nesta rodada, sem
atribuição, sob o lock de GPU (`with-gpu-lock.py`) com Cargo seriado (`cargo-serial.py`), `TESSEL_REQUIRE_RUNTIME=1`,
Chrome real, piso de 12 GiB amostrado a cada 2 s (mínimo registrado 13,25 GiB; nenhum aborto). Cada gate tem log próprio com
cabeçalho (head, árvore, status, comando, cwd, UTC, espaço livre) e rodapé (saída, espaço mínimo, veredito) em
`/tmp/tessel-pr11-integrated-gates-resume/` (`MANIFEST.sha256`; o runner `run_gates.py` tem SHA-256
`8636e3b680025c7f16922e56063e7b919e633c579682d3a1b987383eca1220d9`). A tentativa anterior abortou antes do `fmt` por falta de disco
(`ABORTED_LOW_DISK_BEFORE_START`) e **não conta como PASS nem como FAIL**; os 1210/0/9 de `6e5f3f3` são base antiga.

| Gate | Resultado |
|---|---|
| `runtime setup` (`pnpm install --frozen-lockfile` + build, do próprio checkout) | exit 0 |
| `cargo fmt --all -- --check`; `check --workspace --all-targets --locked`; `clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| core `scene_export::audio` (unidade) / core `scene_export` | 11 / 21 passaram |
| mídia `scene_export_audio` / mídia `scene_export` / render `scene_export` (Chrome real) | 29 / 11 / 12 passaram |
| S6: `scene_edit` 29, `scene_edit_oracle` 7, `scene_edit_properties` 33, `scene_edit_session` 8, `scene_interaction` 22 | 99 passaram |
| desktop `-p tessel-editor --bins` (205 testes; era 201 antes do merge da S6) | 205 passaram, 0 falharam |
| `test --workspace --no-fail-fast --locked` | **1263 passed, 0 failed, 9 ignored** em 78 binários (core lib 225; as 9 `ignored` são medições longas, ZIP64 e fases de aceite com lock explícito, listadas no log) |
| `build -p tessel-editor`, `build -p tessel-editor-core --bin tessel-headless`, `cargo tree -p tessel-editor-core` | exit 0; a árvore do core não lista gpui nem wry |
| Mutações M1–M9, aplicadas uma a uma e revertidas (árvore limpa conferida depois de cada uma) | todas reprovaram o teste certo: M1 `a_cancel_in_the_middle_of_the_copy…`; M2 `a_cancel_right_after_the_copy…`; M3 `a_cancel_in_the_middle_of_the_mix…`; M4/M5 `a_hook_that_fails_or_panics…`; M6 2 testes (`exact_arithmetic_reports_overflow…`, `a_non_decimal_start_and_a_late_range…`); M7/M8 `a_decimal_keeps_twelve_places…`; M9 4 testes |
| Prova nativa 45/45 (`scripts/scene-export-audio-ui-proof.py`): binário próprio, HOME curto, socket próprio, só `get/set/invoke/op`, sem janela por coordenadas | 45 checks, 0 falhas |

Prova nativa, sidecar coletado na execução (`native.sidecar.json`): head `31e0c34`, árvore limpa, binário
`mbx/targets/…/debug/tessel-editor` SHA-256 `886b448fef0a63496154c96b78c37effd473cdccdd4c5df0d8b76c8e632cbc71` (igual antes e depois),
script SHA-256 `2a3a2a6b1a9b73c789847acdb8bae354dde64be5bd0476d57e846a90bbd45daf`, relatório JSON SHA-256
`6521c4a5c21f05436a9a4dc9bea3c2ddbee2e96bc9b2fab87909c91c5ca13afd` (o mesmo de antes: o JSON só lista os checks e não traz head,
por isso o head e o binário vêm do sidecar e o console está em `27-native-proof.log`).

Limites desta execução, declarados: o gate nº 11 do runner pediu `-p tessel-editor --lib`, mas o desktop é um crate binário
(`no library targets found`, FAIL de especificação, mantido no log); o mesmo gate foi repetido como `--bins` (nº 28, 205/0) no
mesmo head limpo. Não rodei `./scripts/runtime test` (o JavaScript do runtime não mudou), o S10 vivo (BL121 continua BLOCKED e o
FAIL histórico de `audio_export`/`audio_package_export` não é revertido), transporte (BL016.1), fonte > 48 kHz (BL016.3), ponteiro
físico (BL043) nem som ouvido (BL046). O harness da S10 na PR #8 segue em correção.

**Gate final da integração anterior** (cabeça `989bfe3` = `c9833bb` + merge de `origin/main` `e296b36`, sem código novo depois do merge;
conflito só nas linhas F041/F042/F042A da matriz). Com `TESSEL_REQUIRE_RUNTIME=1`, Chrome real e sob o lock de GPU:
`cargo fmt --check`, `check --workspace --all-targets`, `clippy --workspace --all-targets -D warnings` e
`test --workspace --locked` passaram (core 218, `scene_export` do core 20, desktop 194, mídia `scene_export` 11 e
`scene_export_audio` 20, render `scene_export` 12; só ficaram `ignored` as medições longas/ZIP64 e as fases de aceite
`scene_export_proof` que exigem o lock explícito); `./scripts/runtime test` 93/93; `cargo tree -p tessel-editor-core` não lista
gpui/wry; R10 `export_audio_policy` 17/17. A prova da janela real foi repetida com o piso padrão de 12 GiB (29 GiB livres):
`scripts/scene-export-audio-ui-proof.py` **45/45** sobre `target/debug/tessel-editor`
(SHA-256 `2ab2541bb8080e93bb8e059c9e7c60fe1167f587a437a33a1471a12da3485a35`; relatório JSON SHA-256
`6521c4a5c21f05436a9a4dc9bea3c2ddbee2e96bc9b2fab87909c91c5ca13afd`). Isto substitui a execução anterior com piso 8 GiB como
gate; `--min-free-gib` 8, 11.99, `nan` e `inf` são recusados com status 2 antes de iniciar app ou mídia.

Integração com main6f7a944 (claims persistentes e attach por sessão): conflito restrito à
matriz, preservando F042/F042A e F043 da base. Fmt/clippy workspace passaram, assim como
scene_export_audio20, scene_export11, attach_job/process e claims_integrity. Render
scene_export12 passou com Chrome real/runtime obrigatório sob lockGPU; build desktop passou.
Prova nativa repetida no código integrado:45/45, piso12GiB, áudio ligado/desligado e snapshot
privado preservados (`/tmp/tessel-s9-main-native.json`). Isso não conclui o aceite S10.
