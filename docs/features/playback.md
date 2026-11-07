# F018 — Playback e relógio de transporte (P09)

## Status

UI **V** na janela real (`monitor.play`, `playback.*` por `ui op`; `scripts/live-flow-proof.py`), CLI da janela **V**, Lib **V**
(motor de áudio sem dispositivo: `crates/audio`, 10 testes; plano de áudio: `crates/core/tests/video_sound.rs`).
CLI headless **N/A¹**: o relógio de tempo real precisa de janela, WebView e dispositivo.

¹ Quem toca é o adapter. A regra de tempo e de **o que soa** é do core: o relógio só emite `Command::SetPlayhead` (o mesmo
comando do clique na régua) e o áudio sai do mesmo plano (`plan_audio`) que o export usa.

## Contrato

- **Áudio**: `crates/audio` (`tessel-editor-audio`, `cpal` → CoreAudio no macOS). O mix da **sequência inteira** é feito do
  frame 0 (`mix_sequence`; teto de 30 min → `audio_range_too_long`) por uma tarefa em segundo plano, inclusive o **som embutido
  dos vídeos**; o dispositivo toca a partir do frame pedido. `Output::start` nunca bloqueia: uma thread `tessel-audio` abre o
  dispositivo padrão, escolhe estéreo `f32`/`i16` na taxa da fonte se ele oferecer (senão reamostra linearmente) e informa
  `Opening → Playing(dispositivo) | Failed(audio_output_unavailable)`. Fechar também não bloqueia.
- **Relógio**: enquanto há dispositivo, quem manda é o **relógio de amostras** (`Shared::position()`: posição *ouvida* =
  último callback + extrapolação − latência que o host informa; `clock: "audio_device"`). O playhead segue
  `floor(posição / taxa × fps)`; o monitor renderiza o frame mais novo e **descarta os atrasados**. Sem dispositivo (ou
  enquanto abre) o desenho segue o relógio de parede (`clock: "wall"`/`"starting"`) **e o motivo aparece** em
  `audio.error` e na barra de status — nunca fica mudo em silêncio.
- **Transporte**: pausa mantém a posição ouvida e rebobina o cursor até ela; `seek` salta imagem e som; retomar continua de
  onde parou. Um `seek` vale como posição **na hora**, antes mesmo do próximo callback (senão uma leitura nessa janela via a posição antiga e o
  playhead piscava de volta — corrigido depois de aparecer ao vivo sob carga).
- Fim: a fonte acabou e a última amostra foi ouvida (latência incluída) → o playhead para no fim e o estado vira `stopped` com
  o relatório. Abrir/criar/restaurar projeto para a reprodução.

| `op` | Campos | Resultado |
|---|---|---|
| `playback.start` | `from?` (frame; padrão: playhead, ou 0 se está no fim) | `preparing` → `playing`; `already_playing`, `out_of_bounds`, `audio_range_too_long` |
| `playback.pause` / `playback.resume` | — | relatório; `not_playing` |
| `playback.seek` | `frame` | move imagem e som; `out_of_bounds`, `not_playing` |
| `playback.stop` | — | relatório final (`{action, report}`) |
| `playback.status` | — | `state`, `frame`, `fps`, `clock`, `elapsed_ms`, `lag_frames`, `ticks`, `skipped_frames`, `max_gap_frames`, `audio{output{name,host,sample_rate,channels,sample_format,buffer}, open_ms, latency_ms, buffer_frames, callbacks, underruns, late_callbacks, device_underruns, max_callback_gap_ms, position_frames, paused, finished, messages, heard:false}` ou `audio{output:null, error{code,message}}` |

`underruns` = `late_callbacks` (callback que chegou a mais de 1,75 período do anterior: jitter de agendamento) +
`device_underruns` (o que o host declarou como `BufferUnderrun`).

**Qualidade durante a reprodução (BL129):** os quadros do monitor enquanto o playhead anda são `Preview` (proxy de vídeo e, em
cena com splat, MSAA 0 em vez de 4); ao pausar ou terminar, o quadro parado se refina sozinho para o final, sem ação do usuário.
`render.frame` e export nunca usam o preview. Detalhes em [viewport.md](viewport.md#msaa-por-movimento-bl129).
**Ordem dos splats durante a reprodução (BL130):** o quadro de uma cena com splat não espera mais a própria ordenação enquanto o
playhead anda; usa a ordem da última ordenação terminada e a exata volta sozinha ao parar (cabeçalho " · sort"); o quadro de movimento não é o exato (o MSAA reduzido e os proxies seguem a política). Contrato em
[viewport.md](viewport.md#ordem-dos-splats-por-movimento-bl130).

Gate de WebView: o WebView nativo pinta acima de qualquer elemento GPUI, então **qualquer diálogo aberto o esconde**
(`monitor.viewport.state.on_screen=false`) e ele volta ao fechar; provado por requisições semânticas (sem coordenadas),
junto com render após diálogo e após resize da janela. **Limite:** isso prova que a WebView sai de cima do diálogo e volta;
**não** prova foco de teclado nem atalhos sobre a WebView (não há comando semântico para isso).

## Evidência

**Dispositivo real** (macOS 26.6.2, M3 Pro, saída: *AirPods Pro* via CoreAudio, 48 kHz estéreo `f32`, buffer de 512 frames), medido
por `crates/audio/src/bin/audio-probe.rs` (`cargo run -p tessel-editor-audio --bin audio-probe`, JSON):

| Medida | Valor |
|---|---|
| `Output::start` retorna em | 0,32 ms (abertura do dispositivo ≈ 125 ms, em segundo plano) |
| Latência informada pelo host | 10,67 ms (**estimativa do host**, ver limites) |
| Deriva dispositivo × relógio do sistema (regressão em 4 s) | −374 ppm; razão de taxa da posição 1,000005 |
| Posição andou para trás | 0 vezes em 350 leituras |
| Underruns / maior intervalo entre callbacks | 0 / 10,7 ms |
| Pausa | posição congelada; retomar: erro −17 ms vs. relógio de parede |
| Seek de 300 ms | erro −18 ms vs. relógio de parede (≈ buffer + latência) |
| Fim da fonte | detectado 1,1 ms após o instante esperado |

**Janela real** (`scripts/live-flow-proof.py`, sequência de 2 s a 24 fps): relógio `audio_device`, playhead a ≤ 2 frames da posição
do dispositivo, `lag_frames` ≤ 2, 0 frames pulados, 83 ticks, ≥ 10 pedidos de frame ao runtime durante a reprodução, pausa que não
move o playhead, seek e retomada, recusa de seek fora da sequência, fim detectado pelo relógio de amostras, `stop` limpo. Os números
da última execução estão no [relatório](../media-project-export-report.md) (seção 6).

**Testes sem dispositivo:** `Engine::fill` com timestamps sintéticos (resample, estéreo/mono/canais extras, seek, pausa, fim,
callback atrasado, início no meio); desktop (`workflow_tests.rs`): playback segue o relógio de áudio ou diz por que não pode
(pausa/seek/retomada/parada).

## O que NÃO está provado (explícito)

- **Ninguém ouviu o resultado.** `heard:false` sempre. "Sincronismo" aqui é concordância entre o relógio de amostras do
  dispositivo e o frame, não A/V ouvido.
- **A latência é a estimativa do host.** A saída de teste foi um fone **Bluetooth**; o atraso real de codec/rádio/DAC não é
  observável pelo CoreAudio e costuma ser muito maior que os 10,7 ms informados. O contrato só afirma o que o host reporta.
- `late_callbacks` pode subir com a máquina sob carga (uma execução da prova, com outros builds em paralelo, registrou 1 callback atrasado de 253 (gap de 25,9 ms) e o host
  declarou 0 underruns; a execução final registrou 0 de 246, gap máximo 10,7 ms): glitch audível não foi avaliado.
- Offline mix ≠ playback ouvido. Foco/atalhos de teclado não têm comando semântico (só `dialog.*`). Só macOS; outras plataformas dependem do `cpal`, não testadas.
