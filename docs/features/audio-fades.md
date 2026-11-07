# F051 — Fade de entrada e saída em clipe de áudio (BL142)

## Objetivo e limites

`fade_in` e `fade_out` eram recusados (`not_applicable`) em clipe de áudio: no filme o fade foi aplicado direto no
WAV. Agora são **as mesmas duas propriedades** do clipe de vídeo/título (mesmo nome, mesmo comando, mesmo nó
`property:fade_*`), com efeito próprio por tipo de clipe:

| Clipe | O que o fade faz |
|---|---|
| vídeo, título, cena | rampa de opacidade (como antes, `plan.rs::opacity`) |
| **áudio** | **rampa linear de ganho** no mix: sai do silêncio nos primeiros `fade_in` quadros do clipe e vai ao silêncio nos últimos `fade_out` |
| vídeo (o **som** dele) | **não** segue o fade: o fade de um vídeo é visual (regra mantida; teste `the_visual_fades_of_a_video_clip_never_shape_its_sound`) |

Não é o motor antigo nem uma curva configurável: só rampa linear, em quadros da sequência. Não há cross-fade entre
clipes, fade de pan/volume, envelope por keyframe nem fade em mídia de vídeo com som embutido (limites abaixo).

## Contrato do core / biblioteca

- **Unidade e extremos.** Quadros da sequência (slider `0..=48`, passo 1, `fr`; 48 quadros = 2 s a 24 fps). Fora da
  faixa é *clamped* (`clamped: true`, o mesmo comportamento de qualquer slider): `500 -> 48`, `-3 -> 0`. Texto que não
  é número é `invalid_value`. `0` (padrão) = sem fade e o mix é **idêntico, amostra a amostra**, ao de antes (o mixer
  nem toca o ganho).
- **Relativo ao clipe, não ao intervalo.** `AudioSegment` ganhou `clip_offset` (amostras de saída do início do clipe até
  o início do segmento) e `clip_frames` (amostras do clipe inteiro), além de `fade_in`/`fade_out` já em amostras de
  saída (48 kHz). O ganho de cada amostra é `AudioSegment::fade_gain(index)`: `at = clip_offset + index`; entrada
  `at / fade_in` enquanto `at < fade_in`; saída `restante / fade_out` quando `restante = clip_frames - at <= fade_out`.
  Por isso **um mix em pedaços é o mix inteiro**: antes um fade que cruzava um corte do intervalo (o export mixa em
  blocos de 10 s) voltava a ganho cheio ou recomeçava no corte; hoje cada pedaço sabe onde está dentro do clipe
  (teste `a_mix_in_chunks_is_the_mix_in_one_piece_even_when_a_fade_crosses_the_cuts`, com mutação).
- **Trim e mover.** Os fades ficam nas bordas **atuais** do clipe: cortar o início/fim ou mover recalcula
  `clip_offset`/`clip_frames` a partir de `clip.start`/`clip.end()` (teste `trimming_and_moving_the_clip_move_its_fades_with_its_edges`).
- **Sobreposição** (`audio::resolve_fades`). Se `fade_in + fade_out` cabe no clipe, nada muda. Se passa, os dois são
  **encurtados na proporção dos pedidos** até preencher exatamente o clipe (`40+40` quadros em um clipe de 48 viram
  `24+24`): as rampas se encontram no ganho cheio em uma amostra, formando um triângulo, nunca se multiplicam em um
  vale. Um fade sozinho maior que o clipe é cortado no tamanho do clipe. Regra pura, testada (`resolve_fades`).
- **Fade fora do trecho audível.** Um clipe que passa do fim da sequência tem o fade de saída depois do fim do mix
  (não é ouvido); é o mesmo critério da opacidade do vídeo.
- **Onde aparece.** Seção `fades` ("Fades", aba **Áudio**) só para clipe de áudio; a seção `timing` ("Tempo", aba
  Vídeo) continua só para vídeo/título/cena e mantém as mesmas propriedades. Um nó por propriedade: o pai do
  `property:fade_in` é `inspector.fades` para áudio e `inspector.timing` para os demais (`inspector::property_section`).
  `zoom`, `speed`, `opacity` continuam `not_applicable` em clipe de áudio, sem alterar nada.
- **Desfazer.** Um fade é uma edição (`Set property`) como qualquer outra; arrastar o slider é uma só entrada
  (gesto, BL062). O valor persiste no documento (`properties`) e volta ao reabrir.

```rust
use tessel_editor_core::audio::resolve_fades;
assert_eq!(resolve_fades(60, 60, 100), (50, 50));
```

## CLI headless

`./scripts/headless` (JSONL, sem janela):

```sh
./scripts/headless <<'JSONL'
{"op":"clip.property","clip":"c-tone","property":"fade_in","value":12}
{"op":"clip.property","clip":"c-tone","property":"fade_out","value":12}
{"op":"render.audio","from":0,"frames":48,"out":"/tmp/fades.wav","samples":[[0,0],[12000,0],[95999,0]]}
JSONL
```

`render.audio` devolve o mix **processado** (offline, não tocado por dispositivo): `peak`, `rms`, as amostras pedidas e
`plan.segments[].{fade_in,fade_out,clip_offset,clip_frames}`; o WAV escrito é decodificável por qualquer ferramenta.

## UI e CLI da janela

Inspetor, aba **Áudio**, seção **Fades** com os sliders **Entrada** e **Saída** (`fr`); equivalente semântico com a
janela aberta (um clipe de áudio selecionado):

```sh
./scripts/ui set inspector.tabs value audio
./scripts/ui set property:fade_in value 12
./scripts/ui get inspector.fades        # accordion visível, state.tab = audio
./scripts/ui get property:fade_in       # parent inspector.fades, value 12, unit fr
```

O mix para tocar (`playback.*`) é preparado fora da thread da UI (a mesma tarefa de fundo do Play) a partir do mesmo
`plan_audio`; mudar um fade só grava a propriedade e o próximo Play ouve o novo mix. Captura:
[`assets/bl142-audio-fades-window.png`](assets/bl142-audio-fades-window.png) (janela própria, `screencapture -l`).

## Evidências e status

| Superfície | Evidência | Resultado |
|---|---|---|
| Lib | `crates/core/tests/audio_fades.rs` (7): amostras exatas, RMS, blocos = inteiro, trim/mover, sobreposição, extremos/rejeição sem efeito, desfazer + salvar/reabrir; `video_sound.rs` (+1: som do vídeo sem fade); unitários de `inspector`/`tree` | passam; mutação (`clip_offset = 0`) quebra o teste de blocos |
| Mídia | `crates/media/tests/export.rs::an_audio_fade_that_crosses_the_export_chunks_is_the_fade_of_the_whole_mix` (FFmpeg real): RMS do export em blocos = RMS do mix inteiro (< 1e-6) e o AAC decodificado sobe de forma monotônica atravessando o corte de 10 s | passa |
| CLI headless | `scripts/audio-fades-proof.py`: processo `tessel-headless`, WAV de `render.audio` lido por Python (`wave`), 23 checks (rampa, RMS 1/√3, trim, sobreposição, recusas, desfazer, vídeo) | 23/23 |
| Janela (CLI) | o mesmo roteiro por `tessel-editor ui op/get/set/invoke` com `HOME` próprio + nós `inspector.fades`/`property:fade_in` + captura nativa | 28/28 |
| UI (GPUI) | `crates/desktop/src/workstation/audio_fades_tests.rs` (2): arrastar o slider real de Entrada = 1 desfazer; a árvore lista cada fade uma vez sob a seção do tipo do clipe | passam |

Limites e pendências (backlog): o slider vai a 48 quadros (2 s a 24 fps): fades musicais longos pedem uma faixa
maior por tipo de clipe — [BL142.1](../backlog.md); o fade é linear (sem curva) e o som embutido de um vídeo não tem
fade — [BL142.2](../backlog.md); o fade aparece só sob **Áudio**/Fades para clipe de áudio (um clipe de vídeo continua
vendo Entrada/Saída como opacidade em Tempo); nenhum sinal foi **ouvido** em dispositivo (`render.audio` e o export
são mix offline; a escuta em hardware continua sem prova, como em F018).

Revisão B: a prova usa o helper canônico da PR20, roda cada superfície com relatório/cleanup e marca captura indisponível comoSKIP. A checagem vazia no cruzamento zero do seno foi substituída por atenuação de uma janela com amostras nãozero. Testes de borda receberam tolerância discriminante e foram escritos casos23.976/29.97 com erro permitido baseado em uma amostra de envelope. Esses testes Rust e provas reais NÃO foram executados após a correção: disco abaixo do piso12GiB. Não há clipe subframe no modelo de timeline inteira; áudio-fonte menor que um quadro permanece um caso adicional a validar.
