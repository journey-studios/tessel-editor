# Continuidade de áudio ao cortar clipes em grade fracionária (F017 / BL081.5) — relatório de entrega

Contrato: [split.md](../features/split.md) ("Áudio em grade fracionária"), [audio-fades.md](../features/audio-fades.md)
("Relógio absoluto"), [core-architecture.md](../core-architecture.md). Este relatório registra o modelo, as invariantes, a
compatibilidade e as evidências executadas. Status e atribuição vivem só em [backlog.md](../backlog.md) (não editado aqui:
propostas no fim). Nenhum commit, push, PR ou merge; tudo a validar pela revisão do coordenador (incluindo o Mac).

**Estado atual:** BL081.5 validado localmente Windows/Mac, código `46506ac`; sem publicação/PR/merge.
Limites restantes: desempenho não medido (BL081.7) e fade atravessado fora desta fatia (BL081.2).
As rodadas e tentativas abaixo preservam a proveniência; o fechamento está ao final.

## Base e ambiente

- Windows (10.0.26200), PowerShell, Rust 1.99.1, **só** `./scripts/cargo` (Git Bash/mbx, alvo padrão; sem
  `CARGO_TARGET_DIR`, `--target-dir`, `TESSEL_CARGO_LANE`, `clean` ou `gc`). `HEAD` `ead9ca2` com o overlay não commitado
  B1+B2+BL081. Os 15 arquivos-fonte do manifesto tinham os SHA-256 esperados antes de começar.
  `docs/computador-readiness-2026-11-12.md` (18579 bytes, SHA-256 `AF30904C…19486659`) fica fora do diff e não foi tocado; o
  índice real do Git não foi usado para staging; os arquivos de B1/B2 não foram editados.
- Snapshot da base montado por **índice temporário** (`GIT_INDEX_FILE`, 37 caminhos explícitos do `git status` inicial, sem o
  readiness): árvore `48dc8abfbb04988dd187642a3301721e624ae1ff`. O diff do BL081.5 é contra essa árvore.
- Logs duráveis: `C:\Users\wally\tessel-readiness-logs\2026-11-14-bl081-5\` (cada `*.log` traz o `head`, o comando e `exit=`;
  os `final-*.log` são a rodada final depois da última edição de código).
- Sem lock de GPU: nenhuma medição de desempenho foi feita ou declarada (o laço do mixer agora soma inteiros `i128`; o custo
  **não foi medido**).

## O problema

Em 30000/1001 fps, um áudio movido ao frame 1 e cortado no 2 alterava o PCM. `plan_audio` arredondava `AudioSegment.source_start`
a `i64` na amostra de fonte mais próxima do **quadro**, e o mixer lia `source_start + índice × razão` em `f64`. Cada metade
reiniciava a fase; `source_start` racional em segundos não bastava porque a quantização também estava no relógio de **saída**
(o quadro 1 de 30000/1001 começa em 1601,6 amostras de 48 kHz). O guard de `state/split.rs` recusava o som quando as amostras
por quadro (saída ou fonte com velocidade) não eram inteiras: 30000/1001, 24000/1001, 44,1 kHz a 24 fps e os cortes em múltiplos de 5
do NTSC.

## O modelo (`crates/core/src/audio_clock.rs`, novo)

1. **Relógio de saída absoluto.** `S(f) = round(f · den · 48000 / num)` (`i128` checado, meio para longe de zero) é a amostra em que
   o quadro `f` começa, contada do quadro 0 da sequência. Um clipe soa em `S(start) .. S(end)`. Um trecho do mix
   (`plan_audio(from, frames)`) é `S(from) .. S(from+frames)`, com `output_start`/`frames` como diferenças de `S`.
   Antes, o relógio era relativo a `from`: cada trecho arredondava por conta própria (até 1 amostra por trecho).
2. **Posição de fonte exata.** A amostra de saída `n` lê o stream em
   `(source_start + (n/48000 − start/taxa) · velocidade − offset_do_stream) · sr`, fração racional sem arredondar
   (`source_position`). `AudioSegment` agora carrega `source_start: i64` (piso), `source_phase: Ratio` (fração em `[0,1)`),
   `speed: Ratio` (exato, antes `f64`); o mixer deriva `passo = sr · velocidade / 48000` em `Ratio`.
3. **Mixer sem acumulador de float.** `SourceCursor` anda `início + n · passo` com denominador comum em `i128`, soma inteira e
   vai-um; devolve `(piso, fração)` por amostra. Plano que não cabe → `BackendError invalid_audio_plan`.
4. **Fades no mesmo relógio.** `fade_in = S(start+fade) − S(start)`, `fade_out = S(end) − S(end−fade)`,
   `clip_frames = S(end) − S(start)`, `clip_offset = S(início do segmento) − S(start)`. A rampa externa de uma metade é a do clipe
   inteiro. Estouro vira `invalid_time`, nunca zero silencioso.
5. **Split.** O corte continua gravando `source_start` racional (`source_start + decorrido × velocidade`). Não há metadado novo no
   clipe, no documento nem no histórico: a fórmula só usa `start`, `source_start`, velocidade e offset, que toda edição já mantém
   exatos. Por isso não existe âncora que possa ficar velha ao mover, aparar, mudar a velocidade ou cortar de novo, e documentos
   antigos leem igual (nenhuma migração). O guard `audio_sample_grid` e `SplitBlock::AudioSampleGrid` foram removidos; o
   split só recusa o som se a posição de fonte da metade direita não cabe nos inteiros (`invalid_time`, atômico, testado com um
   slider aberto).

### Invariantes (todas com teste)

- Plano: para cada segmento, `source_start + source_phase` é **exatamente** a fração do oráculo na amostra inicial do segmento
  (igualdade de frações, sem tolerância).
- Metades × inteiro: o mix depois do corte é o mix antes, **bit a bit** (`f32::to_bits`), também com clipes em outras faixas
  que cruzam, começam antes/depois e não alinham com o corte.
- Blocos × inteiro: o mix em pedaços, com bordas fora do corte, é o inteiro, bit a bit.
- Undo/redo e salvar/reabrir mantêm estado e PCM; recusa e no-op não mudam o estado, o histórico nem um gesto aberto.
- Mudo/desmudo (clipe de vídeo e faixa) são equivalentes; um clipe NTSC mudo é cortável como qualquer outro.

## Compatibilidade, antes e depois

- **Grade inteira** (24/25/30/60 fps a 48 kHz; velocidades 50/150/200 com passo exato em `f64`): PCM **idêntico bit a bit** ao do
  mixer antigo, provado por emulação do mixer antigo escrita no teste (`integer_grids_keep_the_legacy_pcm_bit_for_bit`).
- **Grade fracionária**: o PCM do clipe **inteiro** mudou, de propósito. O modelo antigo começava cada segmento na amostra de
  fonte mais próxima do *quadro* (não do instante da amostra de saída); a diferença depende da taxa de fonte, velocidade, offset e da fase da fronteira de saída. Não se promete
  identidade com o comportamento antigo; o contrato novo é provado contra o oráculo independente.
- Caso relatado (30000/1001, 48 kHz, clipe movido ao frame 1, corte no 2; rampa float32, 1 amostra de fonte = 7,63e-6):

| Medida | Antes | Depois |
|---|---|---|
| metades × inteiro | 7,63e-6 (= 1 amostra de fonte) | **0** (bit a bit) |
| inteiro × oráculo | 3,05e-6 (≈ 0,4 amostra) | < 4e-7 (arredondamento de float32) |

  (`the_old_halves_disagreed_on_a_fractional_grid_and_the_new_ones_do_not`, rodado com `--nocapture`: a linha
  `BL081.5 before/after …` está em `test-phase.log`.) O sinal é uma rampa, para que cada amostra diga a posição lida.
- `plan_audio` em 24000/1001 e 30000/1001: `output_start`, `frames` e os fades dos trechos podem mudar de 1 amostra em relação ao
  arredondamento por trecho antigo (o export em blocos agora concatena exatamente). Em 24 fps a 48 kHz nada muda.
- JSON do plano: `source_start` segue inteiro; novos `source_phase` e `speed_exact`; `speed` segue número.

## Oráculo independente

`crates/core/tests/clip_split_audio_phase.rs`: fontes float32 com a amostra `k` valendo `k · 2^-17` (esquerda) e metade disso
(direita), exatas em float32, de modo que a interpolação linear devolve a própria posição. O oráculo usa frações `i128` próprias
do arquivo (não `Ratio` nem as funções do core) e a definição física acima. Há duas classes de checagem. As **sem tolerância**:
posição de cada segmento igual à fração do oráculo, e PCM depois igual ao PCM antes, bit a bit. A comparação do **valor** de
cada amostra com o oráculo usa tolerância 4e-7 (arredondamento de float32); uma amostra de fase vale 7,6e-6, então perder uma fração
pequena de amostra já reprova (controle negativo: erro > 1e-6).

| Teste (14) | O que prova |
|---|---|
| `every_cut_keeps_the_phase_on_every_grid_for_audio_clips` | 24000/1001, 30000/1001, 24, 25, 30, 60 fps × 44,1/48 kHz × mono/estéreo × cortes nos quadros 4, 11, 14 e 26 de um clipe em `start` 3, duração 24 e `source_start` prévio de 1/4 s; mais três clipes (`pre`, `cross`, `post`) que terminam antes, cruzam e começam depois do corte, com fases diferentes; undo após cada corte (96 casos) |
| `the_special_grids_that_the_old_guard_refused_are_cut_at_every_frame` | NTSC a 48 kHz e 24000/1001 a 44,1 kHz nos quadros 5, 10, …, 35; 44,1 kHz a 24 fps em todos os quadros 1–39 |
| `video_sound_keeps_the_phase_with_speed_offset_and_two_streams` | som próprio de vídeo em 24000/1001, 30000/1001 e 25 fps × velocidade 50/100/150/200 × offset do stream 0 de 0 e 7 ms × stream 0 (44,1 kHz estéreo) e stream 1 (48 kHz mono, 20 ms de offset) × cortes nos quadros 6, 17 e 28 (144 casos) |
| `repeated_cuts_and_later_edits_leave_no_stale_anchor` | quatro cortes (um deles dentro de uma metade antiga), mover a última peça para o quadro 40, aparar a cabeça de uma peça do meio (sem snap), `source_start` das peças igual ao do oráculo em cada passo; seis undos voltam ao clipe único, bit a bit |
| `a_speed_change_after_the_cut_reads_from_the_halfs_own_anchor` | som de vídeo em 24000/1001: velocidade 150 na metade direita depois do corte |
| `chunks_are_the_whole_mix_and_external_fades_keep_their_envelope` | 30000/1001, 24000/1001 e 25 fps: `fade_in` 4 e `fade_out` 6 quadros, cortes no platô (quadros 7, 8, 14, 21), blocos com bordas fora do corte, cada bloco contra o oráculo; concatenação = inteiro, bit a bit |
| `mute_and_unmute_are_equivalent_and_no_longer_a_bypass_of_the_cut` | `audio_muted` de um clipe de vídeo NTSC (corte oferecido e feito) e `muted` de uma faixa de áudio, mudo antes e desmudo depois do corte |
| `save_reopen_undo_and_redo_keep_the_pcm` | `persist::save/load`, undo e redo |
| `a_source_position_that_overflows_is_refused_atomically_with_a_slider_open` | `source_start` enorme: `clip.split` e `plan_audio` dão `invalid_time`, com um gesto de slider aberto; estado, histórico e gesto intactos |
| `a_cut_inside_an_audio_fade_is_still_refused_without_changing_the_sound` | cortes nos quadros 5 e 24 de um clipe com fades de 6 quadros: `split_unsupported` (`FadeCrossesCut`), estado e som intactos (BL081.2 segue recusa explícita) |
| `a_plan_that_loses_the_phase_fails_the_oracle` | controle negativo: plano com a fase zerada (piso) ou arredondada reprova no oráculo (erro > 1e-6) |
| `integer_grids_keep_the_legacy_pcm_bit_for_bit` | grade inteira = mixer antigo, bit a bit |
| `the_old_halves_disagreed_on_a_fractional_grid_and_the_new_ones_do_not` | antes/depois acima |
| `the_headless_process_cuts_the_case_that_was_refused_and_the_wav_is_identical` | processo `tessel-headless`: o WAV depois do corte é o de antes |

Testes de unidade novos em `audio_clock` (3). Testes antigos ajustados: `clip_split.rs` (o teste do guard virou um teste
positivo; o arquivo segue com 22 testes), `video_sound.rs` (`speed` é `Ratio`), `audio_fades.rs` (o caso fracionário foi
**apertado** de "dentro de uma amostra do mix inteiro" para igualdade bit a bit).

### Mutações (não vacuidade)

- Arredondar a fase em `plan_audio` (`round`, fase 0): **11 de 14** testes do arquivo novo falham
  (`mutation-round-phase.log`).
- Relógio de saída relativo a `from` (o arredondamento antigo por trecho): `fractional_frame_rates_keep_chunk_fades_identical_to_the_whole_mix`
  e `chunks_are_the_whole_mix_and_external_fades_keep_their_envelope` falham (`mutation-relative-clock.log`).
- `plan.rs` restaurado e conferido por hash depois de cada mutação.

## Arquivos alterados (somente BL081.5, contra o snapshot pré-tarefa)

| Camada | Arquivo | Motivo |
|---|---|---|
| core | `src/audio_clock.rs` (novo), `src/lib.rs` | relógio absoluto, posição exata, `SourceCursor` |
| core | `src/audio.rs` | `AudioSegment` com fase e `speed` exato; mixer pelo cursor |
| core | `src/plan.rs` | `plan_audio` no relógio absoluto, fase e fades; remove `Resolver::speed` (`f64`) |
| core | `src/state/split.rs` | remove o guard e `SplitBlock::AudioSampleGrid`; valida só a posição da metade direita |
| core (testes) | `tests/clip_split_audio_phase.rs` (novo), `tests/clip_split.rs`, `tests/audio_fades.rs`, `tests/video_sound.rs` | provas e ajustes de contrato |
| media | `src/export.rs` | **necessário para consumir o contrato**: o início esperado de cada bloco vinha de um arredondamento relativo a `schedule.from`; com o relógio absoluto o plano já concatena, então passa a usar `audio_clock::sample_at` (diferença de dois `S`). Sem regra duplicada no adapter; para `schedule.from = 0` o resultado é igual |
| desktop | `workstation/timeline_panel.rs` | remove o texto do motivo extinto; `workstation/split_tests.rs` ganha um teste (clique em áudio NTSC) |
| docs | `features/split.md`, `features/audio-fades.md`, `features/README.md` (F017, F051), `core-architecture.md`, este relatório | |

Sem dependência nova, sem mudança em `Cargo.lock`, sem alteração em `crates/audio`, `runtime/`, `render/`, `vendor/` ou infra.

## Validações executadas (rodada final, todas por `./scripts/cargo`)

| Comando | Resultado |
|---|---|
| `fmt --all -- --check` | 0 (`final-fmt.log`) |
| `check --workspace --locked --all-targets` | 0 (`final-check-ws.log`) |
| `clippy --workspace --all-targets --locked -- -D warnings` | 0 (`final-clippy.log`) |
| `test --workspace --locked --no-run` | 0 (`final-norun.log`) |
| `build -p tessel-editor --locked` | 0 (`final-build.log`) |
| `test -p tessel-editor-core --locked --no-fail-fast` | lib 580/0; 58 alvos `ok`, incluindo `clip_split_audio_phase` 14/14, `clip_split` 22/22, `video_sound` 8/8 e `audio_fades` 8/8; **2 alvos falham**, as falhas conhecidas de ponto flutuante do gizmo no Windows (`scene_gizmo_rotation::the_rotate_overlay_has_three_closed_rings_and_the_metrics_to_draw_them`, `scene_gizmo_scale::four_parents_of_anisotropic_scale_with_a_rotation_between_them_refuse_the_world_shear`). Não são verde (`final-core-all.log`, `exit=101`) |
| `test -p tessel-editor --bin tessel-editor --locked split_tests` | 4/4: 3 de BL081 e 1 de BL081.5 (`final-desktop-split.log`) |
| `test -p tessel-editor-media --locked --test export` | 11/11, inclui `an_audio_fade_that_crosses_the_export_chunks_is_the_fade_of_the_whole_mix` (`final-media-export.log`) |
| `test --workspace` (execução completa) | **não executado** (testes de mídia podem passar de 30 min e há 2 falhas conhecidas); só `--no-run` do workspace, o core inteiro, `split_tests` e `media export` |
| `scripts/audio-fades-proof.py` | **não reexecutado**; o `audio_fades.rs` equivalente passa |
| `cargo tree -p tessel-editor-core -e normal` | 0 linhas com gpui ou wry (`final-tree.log`) |

(`final-core-audio.log` falhou por um nome de alvo inexistente que eu passei na linha de comando; é um erro da chamada, não do código,
e foi substituído pela rodada do core inteiro acima.)

## Provas reais (não só testes escritos)

- **Processo headless** (`./scripts/headless`, JSONL real, `headless/session.jsonl`, `exit=0`): projeto 30000/1001, WAV 44,1 kHz mono
  PCM16 (dente de serra), clipe `snd` no frame 1 com 47 quadros e `pan` −100 → `render.audio` (`whole.wav`) → `clip.split` no
  frame 2 (caso antes recusado) → `render.audio` (`cut.wav`) → `clip.split` de `snd-split-1` nos frames 15 e 5 →
  `render.audio` (`cut3.wav`). SHA-256 dos três WAV **iguais** (`8D14590A…CA4E0`).
- **Janela própria** (`target\debug\tessel-editor.exe`, uma só, só `tessel-editor ui …`, sem coordenadas): `ui op` cria o mesmo projeto
  e `render.audio` (`window/whole.wav`) → `ui invoke clip:snd select`, `ui set timeline.playhead frame 2` → `ui get timeline.split`
  (botão habilitado) → `ui invoke timeline.split invoke` → `render.audio` (`cut.wav`) → playhead 9, seleção de `snd-split-1`,
  `ui get timeline.split`, `ui invoke timeline.split invoke` → `render.audio` (`cut2.wav`) → dois `history.undo` (`undone.wav`) e dois
  `history.redo` (`redone.wav`) → `ui invoke window.close`. Todos os comandos com `exit=0` (`ui.log`); os cinco WAV têm o **mesmo
  SHA-256** do headless; `editor.err.log` e `editor.out.log` vazios; nenhum processo `tessel*` restante ao final.
- **Limites**: o PCM prova **processamento**, não áudio ouvido; sem ponteiro físico; sem pixels; só Windows; sem desempenho aceito.

## Limites e riscos conhecidos

- O PCM de um clipe **inteiro** em grade fracionária mudou (de propósito, ver Compatibilidade). Projetos já exportados em
  30000/1001 ou 24000/1001 não saem bit a bit iguais aos anteriores; a diferença de amostras depende da fonte e da velocidade; seu efeito audível não foi avaliado.
- O mixer agora usa soma inteira `i128` por amostra; custo por amostra não foi medido (nenhum desempenho aceito; sem lock de GPU).
- Cortar **dentro** de uma rampa de fade de áudio segue recusado (BL081.2).
- O áudio da cena (`scene_export/audio.rs`) é outra rota de plano e não foi alterado.
- `crates/audio` (reprodução) não precisou mudar: consome o mesmo `plan_audio` pelo `OfflineMixer`.
- `docs/migration/clip-split-report.md` e o texto "±1 amostra" do MIG088 não foram tocados e ficam defasados em relação a este relatório.
- Validação no Mac e execução completa do workspace ficam para o coordenador.

## Registro de pendências

Status e atribuições são mantidos em `docs/backlog.md`. A proposta original que
usava BL081.6 para desempenho foi reconciliada: esse ID já era da atomicidade
do slider. O custo do mixer está em BL081.7; prova de fades em BL081.8; export
NTSC em BL081.9; coerência documental em BL081.10. Achados novos da revisão
estão em BL081.11/.12. Não há fila independente neste relatório.

## Recebimento e revisão da coordenação (Mac, 2026-11-14)

Artefatos recebidos e conferidos: diff120666B SHA256
f654486e86b9c057cd8c8dd523aa5eeffaf16e4f7f3a7bd02876487229742998;
gzip35891B SHA256
8ed984f62030814be9022425d4e6bec1df8e88b41ea425eb653f1d86bc3d8dee.
Aplicado sobreab2e8fa; apenas os contextos dos contratos F017/matriz foram
reconciliados separadamente, preservando as provas anteriores. Bytes originais
em /Volumes/Extreme Pro/tessel-bl0815-coordination/received/. Docs atuais
modificados pela coordenação não têm o mesmo hash dos docs originais Windows.

A [revisão independente](t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3A77122fb9-fe19-4ca6-b856-7beca247f3a3%3Adelegate-task%3Abl0815-audio-math-review-mac-20261114)
conferiu fórmula, cursor, bordas, overflow, fades e compatibilidade. Não apontou
bloqueio no modelo de fase, mas achou erro de export (BL081.11) e lacunas de teste.
É revisão de leitura, sem teste/build, e não substitui as provas abaixo.

### Correções da revisão

- BL081.11: schedule usava S(to-from), diferente de S(to)-S(from). O controle NTSC
  1..2 falha no código recebido (1602 contra1601). Com FPS preservado, o alvo
  agora usa a diferença absoluta com erro tipado em overflow; com FPS convertido
  preserva a política de duração dos frames codificados. Comentário obsoleto
  corrigido. Novo teste mede50faixas e outro executa seis exports H.264/AAC reais
  com FFmpeg, capturando o WAV de staging antes do encoder. Faixas1..2,2..3 e
  1..333 (mais de10s, atravessa bloco), antes/depois de cortar o clipe: WAVs
  idênticos ao mix inteiro byte a byte, sem trim/pad de uma amostra na cauda.
- BL081.12: novos testes exercitam fase negativa em[-1,0), o último sample
  segurado durante interpolação, silêncio depois da fonte, invalid_audio_plan
  de plano manual e carry exato. Foram mantidos os controles originais; não se
  relaxou a tolerância PCM nem se chamou igualdade do plano de igualdade do som.
- Os14testes de fase foram divididos em cinco módulos coesos (raiz429linhas,
  módulos103–222linhas, antes1107), mantendo o mesmo alvo de integração.
  Comentário colado em split.rs e contrato de SourceCursor::new esclarecidos.
- Referências históricas de uma amostra nos contratos de fades/MIG088 foram
  atualizadas; relatório BL081 anterior aponta este seguimento. História das
  medições anteriores preservada, sem promover aceites antigos ao código novo.

### Validação Mac no código corrigido

Todos os comandos por scripts/cargo/mbx, alvo padrão; sem os quatro overrides
de ferramentas. Logs com comando/exit/hash em
/Volumes/Extreme Pro/tessel-bl0815-coordination/mac/results.jsonl.

| Check | Resultado |
|---|---|
| fmt/check workspace alltargets/clippy alltargets -D warnings |exit0 |
| core completo |1455 passam,7 ignorados existentes,0 falhas;61 resultados incluindo doctests |
| split_tests desktop; audio_fades_tests desktop |4/4 e2/2 |
| media export e novo export_audio_phase |11/11 e2/2, FFmpeg real |
| workspace no-run; build desktop+headless; cargo tree core |exit0; sem GPUI/Wry |
| audio-fades-proof.py |headless23/23, janela28/28,0skips, captura nativa obtida |
| fase fracionária headless+janela própria |seis WAVs idênticos (inteiro/corte/undo/redo), botão habilitado,15requisições semânticas |

A prova de fase usa30000/1001fps, fonte44,1kHz, clipe no frame1 cortado no2,
HOME/socket próprios e fechamento por window.close. Hashes e identidade dos
binários em mac/phase-proof/result.json; saídas headless/ui e WAVs preservados.
Fades: mac/fades-proof.json e mac/fades-proof/. Nenhuma prova é áudio ouvido,
ponteiro físico, igualdade de AAC ou medição de desempenho.

A [rechecagem Windows final](t3-thread://v2/33595e42-547d-42b7-92a7-cb4acb430dd7/mcp:launch:3a303a871c3f3de78e2a39e4311e4f10948394ad13ee6cedbc2cdb0d735dfb8e)
foi iniciada por nova delegate_task, Cursor Sonnet5.5, com patch de revisão72257B
SHA256 3b9f54c772e451e0c6ed44198af4e43365fb70a8346ea25767e6401b7cddbed4.
Retorno ainda pendente. As provas originais Windows não são gates herdados
para a correção de export e os testes acrescentados depois. Sem commit/push/PR
pelo executor; publicação da coordenação também não autorizada neste item.


### Rechecagem Windows: correção do transporte

A primeira rechecagem do patch de revisão terminou sem conseguir reconstruir o
base64 retranscrito. Nada aplicado, nenhum gate executado; o status/HEAD/índice e
o readiness permaneceram iguais. Não se interpreta isso como reprovação do código
nem como prova Windows dos novos testes. Os artefatos ABORTED.txt e probes foram
preservados no logdir da tentativa.

A coordenação iniciou nova delegate_task com arquivo binário baixado diretamente
por transferência temporária na rede local. Só o patch exato é servido; servidor
encerra após download ou prazo de10min. O executor verifica os72257B/SHA256 antes
de aplicar; nenhum blob será reparado por tentativa e erro. Execução atual:
[patch em arquivo](t3-thread://v2/33595e42-547d-42b7-92a7-cb4acb430dd7/mcp:launch:9cdcc3990116d459cbc3fcbf68ccd73d6640d8735588357a43f4e08491bd25cd).
Retorno ainda pendente, sem polling. Código46506ac e provas Mac inalterados.


## Fechamento local após rechecagem Windows (2026-11-14)

Status funcional: **validado no Windows e Mac**, código46506ac. A rechecagem
por arquivo obteve os72257B/SHA256exatos e aplicou o patch sem conflito. Gates
fmt/checkworkspacealltargets/clippyalltargets-Dwarnings/workspacenorun/build exit0.
Os77testes direcionados passaram sem falhas/skips: fase14,bordas2,split22,fades8,
video_sound8,audio_clock4,desktop4+2,mediaexport11+novoexport2. FFmpeg real nos exports.
O core completo e workspace inteiro não foram repetidos nesta rodada Windows.

Logs novos direct-* em C:\Users\wally\tessel-readiness-logs\2026-11-14-bl081-5-review\,
com direct-SUMMARY.txt, direct-sources-after.txt e direct-media-export-phase.log.
Índice real/readiness e18fontes B1/B2 preservados;Windows continua main@ead9ca2 com
overlay autorizado. Nenhum commit/push/PR pelo executor. A transferência temporária
serviu um únicoGET200 e o servidor encerrou;nenhum serviço ficou em execução.

### Auditoria da fonte desktop divergente

O manifesto bruto conferiu18/19fontes. split_tests.rs Windows tem11275B ehash
0c4ab12901ad787c96e17c65d273eb856531ca266a04f9f9f1fec3ceaa47743e;
a cópiaGit/Mac tem11274B ehash
1dbacb59cabf1682f8f6957771d1e978abfc334510c361e706377d781b1687f7.
Uma tarefa somente de transporte leu ReadAllBytes no Windows sem alterar arquivo.
Bytes recuperados pelo transcript conferiram com seu hash; a comparação mostrou
somenteCRLF na última linha no Windows, contraLF noMac. Removido esseCR, os bytes
são idênticos. Não se promove esse fato a igualdade de hashes brutos; a prova dos
4testesDesktop é das versões respectivas com conteúdo igual. Evidência durável:
split_tests.windows-exact.rs e split-tests-line-ending-audit.json no diretórioSSD.

A janela Windows não foi repetida após a correção de export; o modelo/handler de
produção do core não mudou semanticamente em relação à provaWindows original.
O export modificado recebeu teste real novo; a janelaMac final e as seisWAVs
idênticas estão registradas acima. Sem áudio ouvido, ponteiro físico ou desempenho
aceito. BL081.7 (custo do mixer) e BL081.2 (fade atravessado) permanecem abertos.
A revisão, as validações e o transporte estão concluídos; nenhum executor ativo.
Sem push, PR ou merge. O backlog centraliza os status eIDs finais.


## Revisão GitHub da PR #57 — BL081.14

O oráculo rejeita PCM não finito nos dois canais antes de acumular o maior erro.
Controle novo injeta NaN e infinitos em cada canal e confirma rejeição; a prova
desktop exige o mesmo comprimento antes da comparação bit a bit. O diagnóstico
de fades de áudio somados maiores que o clipe recomenda encurtar o fade, sem
sugerir mover o playhead (nenhuma posição pode cortar esse caso).

Mac: fase15/15, split23/23, fades8/8, desktop split4/4; export11/11.
fmt/checkworkspacealltargets/clippyalltargets-Dwarnings/no-run/build desktop exit0.
O core completo1456/7 ignorados anteriormente registrado é do bf2f16b; não se
reatribui esse número ao ajuste novo. Provas anteriores Windows permanecem no
código e nos hashes originais; sem nova janela, execução Windows ou desempenho.
Logs no SSD de coordenação, github-review-final/.
[PR #57](https://github.com/journey-studios/tessel-editor/pull/57).
