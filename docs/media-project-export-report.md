# Relatório — Spark, mídia, projetos portáteis e exportação (P01–P12)

Data: 2026-11-04. Plano: [media-project-export-plan.md](media-project-export-plan.md). Setup: [media-setup.md](media-setup.md).
Não há commit/push (repositório sem commit inicial; tudo não rastreado).

**Revisão de aceite:** a presença de código nas etapas P01–P12 não significa que todos os
critérios do plano foram atendidos. A entrega permanece **parcial**: áudio de vídeo não
entra no mix, a saída de áudio não fornece relógio/latência de dispositivo, operações
pesadas ainda bloqueiam a UI e falta seletor visual de sequências. P09 e P12, em particular,
não estão concluídos. A compatibilidade ZIP64 acima de 4 GiB em arquivo real e a comparação
preview/export também continuam pendentes. A tabela abaixo descreve a execução delegada;
estes limites prevalecem sobre seus rótulos de conclusão até a rodada corretiva.

**Resumo honesto:** todas as etapas P01–P12 têm código real, testes e prova ponta a ponta na janela macOS real
(`scripts/live-flow-proof.py`: **105/105** checagens com `--uhd`, 99 sem). **Não** foi provado: áudio *ouvido*, sincronismo A/V
auditivo, ZIP64 >4 GiB gravado em disco e extraído por ferramenta externa, memória do Node+Chrome, GPU real (as provas de frame
usam SwiftShader), plataformas além de macOS arm64. Lacunas completas na seção 7.

## 1. Ambiente da evidência

macOS 26.6.2, Apple M3 Pro, Rust 1.99.1, Node 22.23.3, FFmpeg 7.1.1 (libx264, aac), Chrome headless com SwiftShader
(`TESSEL_GL=swiftshader`, determinístico), WKWebView real (`gpui-pre`), Spark 2.4.0 + Three.js 0.187.0, build **debug**.
Tempos são de build debug em CPU: servem de ordem de grandeza, não de capacidade do produto.

## 2. Status por etapa

| Etapa | Entrega | Contrato | Status | Prova principal |
|---|---|---|---|---|
| P01 | Schema canônico, IDs, múltiplas sequências | [F022](features/project-document.md), [F023](features/sequences.md) | Implementado e validado (lib/CLI/janela); UI sem seletor de sequências | `core/tests/project.rs`, round-trip canônico, documento inválido não altera a sessão |
| P02 | Save/load, autosave, recuperação, undo/redo | F022, [F024](features/history.md) | Implementado e validado nas três superfícies | fluxo na janela: save-as, undo/redo por botões, autosave, SIGKILL e recuperação |
| P03 | Jobs, staging, cancelamento | [F025](features/jobs.md) | Implementado e validado | testes de cancelamento/disco cheio/staging; lista de Tarefas na janela |
| P04 | Spark 2.4.0 no runtime compartilhado | [F030](features/spark.md) | Implementado e validado (Chrome + WKWebView) | `runtime/test/splat.test.mjs`, `crates/render/tests/frames.rs`, `live-proof.py` 56/56 |
| P05 | Import: probe, copy/reference, dedup, relink | [F019](features/media-io.md) | Implementado e validado | `media/tests/import.rs`; diálogo Importar na janela (mp4, png, ply, wav) |
| P06 | Thumbnails, waveforms, proxies, decode | F019 | Implementado e validado (lib/CLI); sem UI própria | `media/tests/export.rs`, cache invalida por hash/versão |
| P07 | Pacote/restauração | [F026](features/package.md) | Implementado e validado; ZIP64 >4 GiB só em `io::sink` | `media/tests/package.rs`; `.tesselpack` aberto por `zipfile`; restaurado com originais indisponíveis |
| P08 | Intercâmbio entre projetos | [F027](features/interchange.md) | Implementado e validado | `media/tests/interchange.rs`; importação com IDs colidentes na janela |
| P09 | Playback, áudio em dispositivo, gates da WebView | [F018](features/playback.md) | Implementado; **áudio não ouvido** | relógio ±3 frames, `afplay` real, gate de overlay/resize por comandos semânticos |
| P10 | Export 1080p com áudio via FFmpeg | [F029](features/video-export.md) | Implementado e validado | ffprobe lê o arquivo de volta e compara com o plano |
| P11 | Presets, vertical/quadrado, UHD 4K, cancelamento, disco cheio | [F028](features/presets.md), F029 | Implementado e validado | UHD 3840×2160 real na janela; cancelamento 4K; teste de disco cheio |
| P12 | Prova completa e regressão | este relatório | Concluído com as lacunas da seção 7 | `scripts/live-flow-proof.py` |

## 3. Decisões

- **Core sem GPUI/wry** (`cargo tree -p tessel-editor-core` limpo). Heavy work em `crates/media`, injetado pelo hook `Extension`.
- **Documento canônico** em JSON de chaves ordenadas; caminhos relativos; ID de string, tempo/FPS racionais; presentation opcional.
- **Undo por snapshot** (200 passos): nunca apaga bytes de mídia.
- **Jobs em duas fases** (preparar arquivos fora do projeto, aplicar numa transação): cancelar não deixa estado parcial.
- **ZIP64 próprio** (sem compressão, sem dependência): suporta >4 GiB e é lido por `unzip`/`zipfile`.
- **Export em renderer headless próprio** (nunca captura o monitor) a partir de um snapshot imutável; o mesmo plano do core decide pixels, tempo e áudio.
- **Spark**: um `SparkRenderer` por contexto, settle por câmera, malha emprestada à primeira cena (ver F030).
- **Playback**: relógio → `SetPlayhead`; frames atrasados são descartados; áudio é o mix offline tocado por `afplay`.
- **Diálogos** como `Root` dialogs com corpo em `Entity` própria; WebView escondida enquanto há diálogo.
- Mudança de contrato "só endurece com compatibilidade": operações novas só **adicionam** (`persistence` no `summary`, ops novas).

## 4. Arquivos principais

- `crates/core/src/`: `document.rs`, `persist.rs`, `jobs.rs`, `preset.rs`, `interchange.rs`, `splat.rs`, `fixtures.rs`, `state/history.rs`, `state/project.rs` (sequências, presets).
- `crates/media/` (novo): `host.rs`, `import.rs`, `probe.rs`, `derive.rs`, `package.rs`, `zip.rs`, `interchange.rs`, `export.rs`, `fsx.rs`, `tools.rs`, binário `tessel-media`, testes `import|package|interchange|export|flow`.
- `crates/desktop/src/workstation/`: `flows.rs` (campos/requests dos diálogos), `workflow.rs` (diálogos, tarefas, autosave, nós semânticos), `playback.rs`, `workflow_tests.rs`; ajustes em `mod.rs`, `ops.rs`, `viewport.rs`, `monitor.rs`, `status_bar.rs`, `header.rs`, `automation/server.rs`.
- `runtime/src/`: `splat-stage.js`, `splat-format.js`, `scene.js`, `assets.js`, `compositor.js`; testes `splat.test.mjs`, `pure.test.mjs`.
- `scripts/`: `live-flow-proof.py` (novo), `media` (novo), `fixtures`.

## 5. Verificação final (esta rodada)

| Comando | Resultado |
|---|---|
| `cargo fmt --all -- --check` | ok |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | ok |
| `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked` | ok: desktop 76, core 112 + headless 2 + project 8 + proof 8, media 10 + export 10 + flow 2 + import 4 + interchange 2 + package 4, render 4 + export 2 + frames 7 |
| `./scripts/cargo build -p tessel-editor --locked` | ok |
| `./scripts/runtime test` | 12/12 (Chrome real) |
| `cargo tree -p tessel-editor-core` | sem gpui/wry |
| teste ZIP64 `--ignored` (4,5 GiB para `io::sink`) | ok, 163 s (debug) |
| `python3 scripts/live-proof.py` (viewport, janela real) | 56/56 |
| `python3 scripts/live-flow-proof.py … --uhd` | 105/105 |

## 6. Métricas medidas (build debug, SwiftShader, M3 Pro)

- Import de 4 arquivos (mp4, png, ply, wav) copiando: **0,6–0,9 s**. Pacote `.tesselpack` do projeto de prova: **469 417 bytes**.
- Export de 2 s a 24 fps (48 frames) com áudio: **Full HD 17–21 s**, vertical **11–14 s**, quadrado **10–14 s**, **UHD 4K (3840×2160, tamanho real) 52–57 s**. O custo é do render por frame em CPU, não do FFmpeg.
- Cancelamento de export UHD: reconhecido e sem arquivo parcial; latência = no máximo um frame (segundos em 4K/SwiftShader).
- Playback (2 s, 24 fps): 61 ticks, **0 frames pulados**, gap máximo 1 frame, `lag_frames` ≤ 2; `afplay` rodou **2,95–3,18 s** para um mix de 2,0 s (sobrecarga **~1,0–1,2 s**).
- Teste de 4,5 GiB (CRC+SHA em stream, sem escrever): 163 s em debug.
- **Não medidos:** memória de CPU/GPU do Node+Chrome, tempo de sort do Spark por frame com nuvens grandes, diferença numérica preview×export, espaço temporário real além do estimado no preflight.

## 7. Lacunas explícitas

1. **Áudio ouvido:** nada foi ouvido por uma pessoa; "A/V sync" é concordância relógio/frame. O `afplay` tem ~1 s de sobrecarga, então o som pode entrar atrasado na prática. Falta adapter de baixa latência (CoreAudio) e medição de latência de dispositivo.
2. **ZIP64 >4 GiB em disco** e extração por ferramenta externa: não executado (disco com ~7 GiB livres); provado só o layout com `io::sink` e a compatibilidade externa em pacotes pequenos.
3. **Áudio embutido em vídeo não é mixado** no export (aviso `video_audio_not_mixed`); só clipes em tracks de áudio.
4. **Proxies são só de preview** e `decoded` é runtime-only: após abrir um projeto é preciso `media.prepare` antes de render/export de vídeo comprimido.
5. **VFR/FPS misto** sem tratamento dedicado.
6. **Operações síncronas na thread da UI** no host interativo: `media.prepare`, `project.verify`, `project.restore`, sonda de capacidades de `export.preflight` (constroem renderer).
7. Cancelamento de export tem granularidade de **um frame**.
8. **Sem evidência visual por screenshot** nesta rodada (o id da janela não foi obtido pelo `screencapture -l`); a prova é por pixels/estado via socket e por ffprobe, não por inspeção humana do vídeo exportado.
9. **Sem seletor visual de sequências**; a troca é por `sequence.activate`.
10. Foco por teclado e atalhos sobre a WebView não têm comando semântico (só `dialog.*` e `invoke`); o gate de overlay cobre diálogos do app.
11. Só macOS arm64, só H.264/AAC/MP4 SDR; sem HEVC/HDR/alpha, sem retomada de export, sem política de fontes redistribuíveis, sem empacotar/licenciar o FFmpeg.
12. Presets: sem migração de versão de preset/documento (só v1).
13. Fontes de texto: bitmap 5×7 do runtime (herdado da fundação).
