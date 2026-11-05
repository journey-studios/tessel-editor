# Relatório — fundação da viewport multimídia

Escopo: [plano](viewport-foundation-plan.md), F015 ([projetos](features/project-creation.md)) e
F021 ([viewport](features/viewport.md)). Nada do motor ou dos assets do editor original foi
importado; todo conteúdo de prova é procedural (`./scripts/fixtures`). Sem commit nem push.

## Resultado em uma frase

Um projeto criado só por API (IDs próprios, assets tipados, tracks, clipes, câmera) é
avaliado por um plano racional no core, renderizado em frame real por **Node + Chrome +
Three.js sem janela do editor** e, pelo mesmo pedido, desenhado numa **WebView persistente**
no monitor único; os pixels conferem nos dois caminhos. O que não existe (playback, áudio
ouvido, overlays sobre a WebView, `gpui-fast` em produção) está em [Lacunas](#lacunas).

## Arquivos

Core (`crates/core/src`, sem GPUI/Wry/DOM): `id.rs`, `time.rs` (`Ratio`/`Rate`), `assets.rs`,
`content.rs` (texto, câmera), `plan.rs` (`RenderPlan`, `PreviewIntent`), `audio.rs`
(`AudioPlan`, `OfflineMixer`), `backend.rs` (`RenderBackend`, capacidades), `bridge.rs` +
`session.rs` (bridge v1, `ViewportSession`), `ops.rs` (vocabulário JSON), `headless.rs`
(executor compartilhado), `fixtures.rs`, `commands.rs` (`monitor.preview.*`, `monitor.reload`),
`state/tree.rs` (nós do monitor). Testes: `tests/proof.rs`, `tests/headless.rs`, módulos `tests`.

Render (`crates/render`): `runtime.rs` (descoberta, `RuntimeConfig`, `discover_page`),
`process.rs` (`ProcessTransport`), `host.rs` (servidor de assets da WebView), binários
`tessel-render` e `tessel-fixtures`, `tests/frames.rs`.

Runtime JS (`runtime/`): `src/` (compositor, cena, splat próprio, fonte 5×7, hash, assets,
protocolo, boot), `worker/` (Node + `puppeteer-core`), `host/index.html`, `test/` (13 testes).
Three.js 0.187.0 empacotado com esbuild; sem CDN.

Desktop (`crates/desktop/src`): `workstation/viewport_host.rs` (lógica sem tipos GPUI),
`viewport.rs` (WebView), `ops.rs` (`run_op`), `monitor.rs` (modos Sequência/Fonte, status),
`automation/{server,client}.rs` (`ui op`, resposta diferida), `*_tests.rs`.

Scripts: `scripts/{render,fixtures,runtime,live-proof.py}`. Docs: `docs/features/*`,
`core-architecture.md`, `ui-automation.md`, este relatório.

## APIs e comandos

- Rust: `EditorState::{empty,demo,plan,plan_audio}`, `ops::parse_op`, `headless::execute*`,
  `ViewportSession`, `RenderBackend`/`Transport`, `render::{session,discover_page}`.
- JSON: `project.create`, `asset.add`, `track.add`, `clip.add|text|camera|property`,
  `sequence.length`, `preview.set`, `seek`, `plan`, `render.frame`, `render.audio`,
  `capabilities`, `render.status`, `status`.
- Janela: `./scripts/ui op`, `ui invoke monitor.preview.sequence|source|reload`,
  `ui get monitor.viewport`.
- Setup: `./scripts/runtime setup|build|test`; `TESSEL_CHROME`, `TESSEL_NODE`,
  `TESSEL_RUNTIME_DIR`, `TESSEL_GL`, `TESSEL_REQUIRE_RUNTIME`.

## Validação (esta entrega, macOS, Rust 1.99.1)

| Comando | Resultado |
|---|---|
| `cargo fmt --all -- --check` | ok |
| `./scripts/cargo check --workspace --locked` | ok |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | ok |
| `./scripts/cargo test --workspace --locked` | desktop 66, core 100, `headless` 2, `proof` 8, render 4, `frames` 7: todos passam |
| `./scripts/cargo build -p tessel-editor --locked` | ok |
| `./scripts/runtime test` | 13/13 |
| `cargo tree -p tessel-editor-core` | sem `gpui`/`wry` |
| `python3 scripts/live-proof.py … ` (app aberto) | 57/57 |

Linha de base preservada: 38 desktop + 61 core + 2 integração continuam (os desktop viraram
39 com `op_wraps_one_json_object`; core e desktop cresceram com os novos testes). Os testes de
`frames.rs` pulam sem Chrome (`TESSEL_REQUIRE_RUNTIME=1` os torna obrigatórios); aqui rodaram.

Testes novos que exercitam diretamente o contrato: `monitor_controls_map_to_preview_commands…`,
`source_preview_without_a_previewable_selection_is_refused` (parser), `tree::tests` (nós,
estado `selected`, desabilitado com motivo), `viewport_tests` (nó `monitor.viewport`, projeto
por `run_op`, rejeição sem mutação, `backend_unavailable` sem runtime, ocultar o monitor),
`viewport_host_tests` (ciclo de vida da página).

## Fluxo de aceitação provado

criar projeto → assets (imagem, frames, WAV, splat, cena, texto) → 3 tracks de vídeo + 1 de
áudio → clipes (+ câmera da cena) → `seek` → `plan` → `render.frame` headless → **mesma
sequência** na janela por `ui op`. Rejeições (track inexistente, projeto inválido, frame sem
runtime, overlay não suportado) não mudam o estado (`summary()` idêntico antes/depois).

### Resultado por tipo (pixels conferidos nos dois caminhos)

| Tipo | Headless (`frames.rs`) | Janela (`live-proof.py`) |
|---|---|---|
| Imagem | quadrantes (220,40,40) (40,200,60) (40,80,220) (230,210,40) | idem |
| Vídeo (frames) | cor do frame de fonte nos quadros 30 e 47 | idem; screenshot `03-frame-30-video.png` mostra o retângulo verde |
| Texto | pixels brancos na posição do título | idem |
| Cena / splat | centro muda com tempo e câmera | idem (`scene-0` vs `scene-24`, câmera movida) |
| Fonte isolada | imagem e waveform independem da sequência | `07-source-image`, `08-source-audio` |
| Áudio | `render.audio`: WAV estéreo 48 kHz, `kind:"processed"`, pico 0.354 | idem; **não ouvido** |
| Falha de frame | `unsupported_format` (blend `overlay`), sessão se recupera | cabeçalho "Falha: unsupported_format" e recuperação |
| Ciclo de vida | troca de projeto não vaza assets | ocultar/mostrar monitor, `monitor.reload`, rajada de seeks |

## Métricas observadas

Janela real (WKWebView, renderer "Apple GPU", 320×180 de sequência, preview 960×540):

| Medida | Valor |
|---|---|
| Frame 0 | render 26 ms, parede 38 ms |
| Frames explícitos 30 / 47 | parede 50 / 40 ms (render 45 / 37 ms) |
| Seek até exibido | ~29 ms (outras corridas: 28–42 ms) |
| Rajada de 14 seeks | 14 pedidos, 1 aceito, 13 cancelados, 8 respostas velhas descartadas (histórico: com o agendador de preview, ver [seek-performance-report](seek-performance-report.md), a mesma rajada dá 2 pedidos, 2 aceitos, 0 cancelados) |
| RSS do editor | ~170 MB antes, ~132 MB depois (outra corrida: 155 → 130) |
| Processos WebKit | ~135 MB antes, ~120 MB depois (teto: pode incluir outros apps) |

Headless (SwiftShader, 320×180): primeiro frame 517 ms (inclui aquecimento do Chrome),
seguinte 234 ms. Memória do worker headless não foi medida. São medidas de uma máquina e de
uma execução: não são orçamento nem promessa.

`gpui-fast` (cópia temporária): build 1m24s, `live-proof.py` 57/57, frame 0 render 27 ms,
seek→exibido ~32 ms, rajada igual, RSS 170 → 157 MB. Diferenças dentro do ruído.

## Observado visualmente

Capturas `screencapture -l` da janela (Read de `/tmp/shots/final/*.png`): monitor mostrando
cena com esfera, 4 quadrantes de imagem e título "TESSEL" em fonte bitmap; no frame 30 o
quadro de vídeo verde substitui a imagem; após a falha provocada o cabeçalho mostra
"Falha: unsupported_format" (o texto encosta na borda direita mesmo truncado), e a imagem
anterior permanece; com o painel fechado a área do monitor fica vazia (a view nativa é
ocultada); reabrir restaura a imagem. Com `gpui-fast`, depois de `dock resize left|bottom`
a WebView acompanhou a área do monitor, sem deslocamento visível.

## Lacunas

Revisão após a entrega delegada: reexecutado `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test
--workspace --locked`, com **187 testes passando**, incluindo os 7 de frames reais sem
janela; `./scripts/runtime test` passou com 13 testes e nenhum skip. Isso confirma o caminho
headless nesta máquina. As provas nativas 57/57 e capturas acima são da execução delegada;
não foram repetidas nesta revisão. A base não deve ser considerada pronta para toda a
migração enquanto os gaps relevantes abaixo não forem resolvidos.

1. **Áudio**: só mix offline processado; sem dispositivo, sem A/V sync, sem playback.
2. **Overlays e foco sobre a WebView**: no `gpui-pre` a view nativa cobre overlays e não
   recorta (mitigado ocultando a view). Com `gpui-fast` não foi possível provar overlay/foco
   (nenhum comando semântico abre um overlay sobre o monitor; clique por coordenada ficou
   fora). O padrão **não** foi trocado; ativar exige `gpui-fast` em `gpui-kit` **e**
   `gpui-webview`, e repetir esta prova com tooltip/menu.
3. **Determinismo**: hashes de frame só são estáveis sob SwiftShader; a janela usa a GPU e dá
   hash diferente. Comparações usam tolerância por pixel (ver testes).
4. **`webm`** depende do decoder da engine (WKWebView vs Chrome); o formato inicial garantido
   é sequência de frames. *(Atualizado: [viewport-bugfix-report](viewport-bugfix-report.md) — `webm` não é mais prometido; falha tipada no WebKit.)* Sem invalidação quando o arquivo muda em disco; arquivos inteiros são
   lidos em memória quando não há range.
5. **IDs internados** nunca são liberados (limite = IDs distintos por processo).
6. **Render**: texto minúsculo sai em maiúsculas (fonte 5×7), blend `overlay` não suportado,
   alpha do fundo de cena ignorado, ordenação de splat O(N) na CPU, `viewport.resize` é no-op
   na página (letterbox por CSS).
7. **GPUI**: janela totalmente coberta não pinta; o bookkeeping de `on_screen` atrasa até
   ativar a janela (`ui invoke window.activate`). Cabeçalho de falha pode encostar na borda.
8. **Janela**: `ui op` aceita só o vocabulário de projeto (não `set/invoke/move`); barra de
   status ainda diz "Demonstração" após `project.create`; miniaturas de assets gerados são
   placeholders; sem diálogo de criação ou import por UI.
9. **Plataforma**: validação nativa só em macOS arm64 com Chrome do sistema; Linux/Windows,
   outras GPUs e HiDPI diferentes não foram exercitados. A WebView é experimental (`gpui-webview`).
10. **Memória** foi medida grosseiramente (RSS por `ps`), sem perfil de GPU nem vazamento a longo prazo.

## Como reproduzir

```sh
./scripts/runtime setup && ./scripts/runtime test
./scripts/fixtures /tmp/tessel-proof
./scripts/render < /tmp/tessel-proof/proof.jsonl          # headless, sem janela
./scripts/cargo run --locked -p tessel-editor &           # janela
./scripts/ui invoke window.activate
python3 scripts/live-proof.py /tmp/tessel-proof --shots /tmp/shots --report /tmp/report.json
```
