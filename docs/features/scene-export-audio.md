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
- **Identidade e bytes congelados (BL109, S3)**: o arquivo é o aprovado no mapa de recursos e o job o trata como
  qualquer outro recurso exigido: `ResourceKey::AudioMix` entra nas chaves da cópia privada
  (`ResourceSnapshot::take`), que lê o WAV **uma vez**, calcula o SHA-256 na mesma passada que grava a cópia
  `0444` no staging e compara com o hash e o tamanho aprovados. A mistura lê **somente essa cópia**
  (`render_audio_private`); o original nunca é relido durante o job. Consequências:
  - troca de mesmo tamanho depois do `resolve` → `resource_changed` (`details.resource: "audio-mix"`, `expected` e
    `got` com SHA-256 e bytes) **antes de qualquer frame** e antes de qualquer amostra;
  - alteração do original **durante** o job fica isolada: o MP4 reflete os bytes aprovados;
  - o job confere a cópia (`verify_intact`: tamanho e mtime) depois de misturar e antes de finalizar;
  - sucesso, falha e cancelamento removem só o staging do próprio job (cópia e `mix.wav` incluídos); staging alheio
    e o original nunca são tocados.
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
e inclui na estimativa de disco o WAV misturado + AAC (`estimate.audio_mix_bytes`) **e** a cópia privada do arquivo
(`estimate.resource_copy_bytes`, parte de `staging_bytes`). O preflight só faz `stat` e lê o cabeçalho; a cópia e a
mistura acontecem no job (thread própria, estágios `snapshot` e `audio` com progresso, canceláveis), nunca na UI.
O job copia, **depois** mistura a cópia (falha cedo, antes dos frames) em `mix.wav` dentro do staging, que some em
sucesso, falha ou cancelamento. `render_audio` (identidade por SHA-256 e leitura do caminho recebido) segue público;
`render_audio_private` é o mesmo stream para o arquivo que já é a cópia do job.

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
| `audio_source_short` | o arquivo não tem quadros para o trecho | idem (cabeçalho) ou bloqueio no preflight |
| `unsupported` | formato/canais/taxa fora do suportado; narração com áudio pedido | idem |
| `resource_unresolved`, `resource_missing`, `resource_changed` | mix fora do mapa, sumido ou trocado | bloqueio no preflight; `resource_changed` (troca de mesmo tamanho incluída) falha o job antes dos frames, com as duas identidades |
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

Status: **a validar na integração** (implementado, com as provas abaixo, incluindo a janela real; sem som ouvido).

| Superfície | Evidência |
|---|---|
| Lib (core) | `crates/core/tests/scene_export.rs` (20) e unidade `scene_export::audio` (4): plano padrão com mix, `audio:false`/preset mudo, offset de janela (`10–14 s`, `from=24`), taxa 24→30 (48 000 amostras), cauda de 800 amostras (2 frames → 3), NTSC `30000/1001` (48 048), janela curta com números, decimal exato, narração recusada |
| Lib (mídia, FFmpeg real, sem browser) | `crates/media/tests/scene_export_audio.rs` (20, contando os de bytes congelados abaixo): amostras **exatas** em 48 kHz (cópia = fatia da fonte), offset, 44,1 kHz mono (interpolação calculada no teste, ±1 LSB), cauda silenciosa, NaN/∞/clipping, bytes trocados (`resource_changed`), 8 arquivos/formatos recusados (`unsupported`) mais curto e ausente com código próprio; MP4 pelo host com espectro 440→880 Hz medido no AAC decodificado nos lugares certos, 24→30 + 44,1 kHz, offset de janela, `audio:false`, recusas antes de job/frame, troca de bytes falha com 0 frames desenhados, preflight de mix sumido |
| CLI headless real (Chrome) | `crates/render/tests/scene_export.rs`: `the_headless_binary_mixes_the_declared_wav_at_the_requested_range` (JSONL, frames reais do runtime, 440→880 Hz medido) e o teste de política de áudio atualizado |
| Oráculo independente da R10 | `scripts/directed-scene-export-proof/proof.py --phases export_audio_policy`: 17/17 (uma stream de áudio, metade 440 Hz e metade 880 Hz, duração do áudio = vídeo ± 50 ms), resultado `outcome: mixed` |
| Bytes congelados do mix (BL109) | `crates/media/tests/scene_export_audio.rs` (20): troca de mesmo tamanho → `resource_changed` **tipado** com as duas identidades e 0 frames; original reescrito no frame 0 não muda o AAC (440→880 Hz medidos, `source.sha256` = aprovado, `snapshot.bytes` ≥ WAV); `render_audio_private` sobre a cópia dá as amostras exatas depois de reescrever o original (e `render_audio` ainda recusa o original); mix sumido → `preflight_failed`/`resource_missing` e só o staging próprio some; cancelamento no meio dos frames limpa só o próprio staging; estimativa: `resource_copy_bytes` e `audio_mix_bytes` somam o WAV, e `jobs.rs` testa o mapeamento tipado |
| UI (janela real, instância própria) | `scripts/scene-export-audio-ui-proof.py` **45/45**: árvore expõe `flow.export_scene`; `audio` oferece yes/no com padrão yes; `dialog.submit` com `audio=yes` cria 1 job; MP4 com AAC 48 kHz estéreo, duração do áudio = vídeo (±60 ms), 440 Hz em 0,05–0,40 s e 880 Hz em 0,60–0,95 s no decodificado; `snapshot` inclui o WAV; `audio=no` exporta sem stream e com `audio_omitted`; troca de mesmo tamanho (WAV válido) → job `failed` com `resource_changed`, `details.resource=audio-mix`, sem MP4 e sem staging, visível em `get jobs`; mix que deixou de ser WAV → submit recusado (`unsupported`) com diálogo aberto, sem job nem arquivo; `audio=no` ainda exporta com o mix inutilizável |

### Limites declarados

- AAC é com perda: o espectro e a duração são medidos, a amostra exata é provada no stream que entra no
  encoder (`mix.wav`), não no MP4.
- Conversão de taxa é interpolação linear (sem passa-baixa); fonte > 48 kHz é recusada.
- Mono com −3 dB (convenção da origem/FFmpeg); a origem usa `atrim` sem teto em `editEnd` no render Node,
  enquanto o transporte web para nele — aqui o teto é regra (recusa trecho fora da janela).
- A cópia privada dobra o espaço do WAV no staging e custa uma leitura + escrita antes do primeiro frame (como as
  imagens, BL109); `verify_intact` não refaz o hash. O original que mudou **depois** da cópia não é detectado:
  é isolado, e o vídeo reflete os bytes aprovados.
- A prova da janela é semântica (get/set/invoke): não prova que uma pessoa vê ou alcança o controle (BL043), nem som
  ouvido (BL046). O `--min-free-gib` do script existe porque a máquina de desenvolvimento é compartilhada; a prova
  escreve < 200 MiB.
- Narração, takes, `mixMuted`, ganho de monitor, reprodução em dispositivo e TTS: fora (BL017/BL046).
- O harness da R10 (`proof.py`, fase `ui_export_flow`) ainda espera a recusa do padrão `audio=yes`; com o mix
  entregue essa fase muda de resultado e é ajuste da frente S10 (não alterado aqui).

### Aceite independente da S10 (PR #8, `proof.py` em `c03b41e`) — divergência de semântica, não de qualidade

Rodado com o binário desta branch, sem alterar verificadores nem tolerâncias, sob o lock de GPU:
`tolerances_frozen` 6/6, `corpus` 32/32, `controls_audio` 26/26, `baseline_audio` 46/46, `audio_off` 70/70 (o
`audio:false` e a política de aviso passam no corpus da S10); `audio_export` **FAIL** e `audio_package_export`
**FAIL/abortado**. A causa é a **suposição A1** do oráculo da S10 (`oracle.py`: "mix[0] ↔ editStartSeconds", o
arquivo começa em `editStartSeconds` da linha do tempo e antes disso há silêncio), que contradiz a origem:

- origem (`packages/scene-engine/src/transport/audio-window.ts`): `editSecondsToSourceTime(t) = editStartSeconds + t`,
  o WAV é a **fonte inteira** e a janela escolhe um trecho dela; `node/render.ts` recusa com `AUDIO_SHORT` quando
  `duration(source) < editEndSecondsExclusive`. É o que esta entrega implementa.
- o `mix.wav` do corpus da S10 tem 8 s com janela `[1, 9)`: pela regra da origem ele é **curto** (`audio_source_short`,
  `file_frames 352800`, `needed_through 396899` no caso `audio24_window_b`) e o bundle seria recusado pela própria
  origem.

Medido de forma independente (ffmpeg decodifica o MP4 do `audio24_window_a`, janelas de 100 ms): 440 Hz de 0 a
0,25 s, silêncio até 0,75 s, 880 Hz até 2,25 s, 1320 Hz a −6 dB depois — exatamente o que a regra da origem prevê
para `from=18` (0,75 s) com `editStart=1,0` (arquivo em 1,75 s…); o oráculo da S10 espera o contrário (silêncio, 440 Hz
em 0,25 s). Nenhum dos dois lados foi ajustado para passar: a decisão é do coordenador (BL016.5). Para fechar o aceite
vivo sem mudar o contrato da origem a S10 precisa de um WAV que cubra `[0, editEnd)` com o tom no tempo de arquivo
(ou de assumir A1 como contrato novo, o que muda o core e esta doc); as fases `audio_export` e `audio_package_export`
só viram PASS quando o oráculo e a origem concordarem.

**Decisão do coordenador (BL016.5).** A leitura do pin da origem `d6adac15c61f4ef582a7ec259397b2e025d1f059`
(`git show`, só leitura) confirma `packages/scene-engine/src/transport/audio-window.ts` →
`editStartSeconds + editSeconds` e `node/render.ts:246-248` → `AUDIO_SHORT` quando a duração do arquivo é menor que
`editEndSecondsExclusive`. Fica a regra da origem; o oráculo da S10 (A1) é corrigido por **outra tarefa** já iniciada,
e o core não muda para fazer uma fixture errada passar. Enquanto o oráculo não for corrigido, os FAIL de
`audio_export` e `audio_package_export` acima permanecem registrados como estão. Esta entrega não altera o harness
da S10 nem as tolerâncias. A prova da janela (`scripts/scene-export-audio-ui-proof.py`) usa o `bundle.audio.json`
com janela `[0, 4)`, onde as duas semânticas coincidem.

Nota de execução do coordenador: a prova nativa 45/45 da integração local foi rodada com `--min-free-gib 8`, abaixo do piso autorizado de 12 GiB. O resultado técnico foi preservado, mas não satisfaz o gate operacional e deve ser repetido no head final com >=12 GiB. O script agora recusa piso menor que 12 ou não finito; duas sondas de CLI confirmam a recusa antes de iniciar app/mídia.

**Gate final da integração** (cabeça `989bfe3` = `c9833bb` + merge de `origin/main` `e296b36`, sem código novo depois do merge;
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
