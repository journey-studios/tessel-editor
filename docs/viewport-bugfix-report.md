# Correção dos bugs B1–B8 do viewport

Entrega de 2026-11-05 sobre [viewport-performance-opus-validation.md](viewport-performance-opus-validation.md) (a
tabela original fica como histórico; este documento diz o estado atual). Nada foi commitado nem publicado. Fontes e mídia
privada ficaram intactas; todos os vídeos e sequências são **fixtures próprias** geradas com FFmpeg.

## Resumo

| # | Estado | Em uma frase |
|---|---|---|
| **B1** | **Corrigido, provado** | `<video>` não é mais textura direta: o runtime lê um quadro por `createImageBitmap(video)`; o preto vinha do tamanho de textura zero do Three r187 |
| **B2** | **Corrigido, provado** | Contrato de tempo explícito (intervalo de PTS, leitura no **meio** do intervalo) e espera pelo quadro decodificado; 0 quadros errados nas duas engines |
| **B3** | **Corrigido para o que se pode provar; o resto é erro estruturado** | `webm` saiu das promessas; cada vídeo passa por prova de leitura no carregamento; VP8/VP9 no WebKit e AV1 no WebKit viram `unsupported_format`, nunca preto |
| **B4** | **Diagnosticado e eliminado** | A parada de ≈ 1 s é a primeira leitura **mista** de um mesmo `<video>`; com um único leitor p99 foi 24,7 ms (WebKit, 1080p). O ≈ 1 s no WebKit com janela oculta (primeiro upload) ficou **explicado, não removido** |
| **B5** | Já corrigido; **mantido e testado** | Não houve RT único; teste de duas cenas e de camadas preservado (`retention-browser.test.mjs`) |
| **B6** | **Reproduzido com entrada injetada do GPUI e corrigido**; uso real **não validado** | `on_drag_move` do ruler tratava qualquer movimento durante um drag sem `MouseUp`; agora um movimento sem botão encerra o drag |
| **B7** | **Corrigido** | Sequências PNG **e JPEG** aceitas com padrão `%0Nd`/`%d`, formato único, tamanho único e quadros presentes; PNG 540p medido |
| **B8** | **Corrigido, provado em WebKit e Chrome** | Perda de contexto no quadro vira `context_lost` (com a causa em `details.cause`); antes um quadro em contexto morto devolvia `frame.done` |

## Contrato de tempo (B2) e o que a engine consegue (B3)

Código: `runtime/src/native-video.js` (cabeçalho = contrato), `runtime/src/assets.js` (`loadVideoFile`,
`videoFrameTexture`), `runtime/src/compositor.js` (camada `video`).

- Um clipe tem taxa **constante** `r = num/den` e a primeira imagem em **PTS 0**. O quadro `f` ocupa `[f/r, (f+1)/r)`.
- O quadro `f` é lido com *seek* ao **meio** do intervalo, `(2f+1)·den / (2·num)` (racional, vale para 24, 30,
  24000/1001…). Não é um *offset* mágico: o alvo deriva da taxa. Em cima da fronteira exata, o demuxer arredondava e
  o motor devolvia o quadro **anterior** (B2: Chrome 15/48, WebKit 32–40/48 errados).
- Espera: `seeked` **e** `readyState ≥ 2`; leitura única por `createImageBitmap(video)`. Seeks do mesmo `<video>`
  são serializados (um só *playhead*); a espera aceita `aborted()` (uma busca ultrapassada não ocupa a fila). O quadro vira
  *snapshot* no cache de frames da sequência (16 por asset, LRU, textura no orçamento de 256 MiB): duas camadas do mesmo
  clipe em tempos diferentes mostram **cada uma o seu quadro** (antes: o último que o único `<video>` tinha visto).
- Último quadro: `frames = round(duração·r)`; `source_frame` além do fim segura o último. **Não** há leitura além dele.
- VFR conformado (`meta.vfr_conformed`), taxa/duração ausentes e início deslocado visível ao motor →
  `unsupported_timing` (no CLI, `unsupported_format` com `details.backend_code = unsupported_timing`). Vídeo
  **importado** passa pelo FFmpeg (PNG numerados, VFR conformado por timestamp) e nunca chega a esse caminho.
- **Prova de leitura no carregamento** (`proveVideo`): lê o primeiro, o do meio e o último quadro; falha se uma leitura
  é bloqueada ou se os três são idênticos (decoder preso, leitura preta). Esse é o **único** critério de capacidade:
  nada de `canPlayType` ou extensão. Custo: 3 seeks no `asset.load`.

Capacidades reais (fixtures com barcode; 0 quadros errados = lido o código do quadro pedido nos pixels):

| Clipe | Chrome (SwiftShader/GPU) | WebKit (WKWebView real) |
|---|---|---|
| H.264 (GOP 24, intra, B-frames, 24000/1001, 30 fps), HEVC | exato (0/48 errados × 5 ordens) | exato (0/48 errados × 5 ordens) |
| VP9, VP8 (WebM) | exato | `unsupported_format` (`SecurityError`: a mídia WebM é tratada como *cross-origin* mesmo no esquema `tessel://`; CORS/blob não resolvem) |
| AV1 (MP4) | exato | `unsupported_format` (decoder preso no 1º quadro após seek) |
| H.264 com início em 1 s | lido errado (o Chrome segura o 1º quadro): **limite do contrato**, só detectável com PTS de container | recusado (acidente da prova de leitura) |
| H.264 VFR **declarado como CFR** | 3/48 errados | 3/48 errados |

Os dois últimos casos são violação do contrato por quem declarou a mídia (`asset.add path=…` sem probe): o core só
conhece o que o chamador declara. O import com FFmpeg detecta os dois. **Não prometido:** vídeo por caminho sem import.

Revisão de aceite: os 5 testes de `crates/render/tests/video.rs` foram reexecutados pelo pai com
`TESSEL_REQUIRE_RUNTIME=1` e passaram. A prova de três imagens diferentes é uma heurística,
não uma validação universal de decoder: um vídeo legitimamente estático, ou com imagens iguais
nessas três posições, também pode ser recusado na leitura nativa direta. Usar o fluxo de import
e preparação por FFmpeg nesses casos. Os testes positivos de vídeos com barcode não cobrem
essa limitação, nem validam metadados incorretamente declarados pelo chamador.

`FORMATS.video = ['frame_sequence', 'mp4']`, `capabilities.native_video = { codecs: ['h264'], timing:
'constant_rate_from_pts_zero', proof: 'load_time_read' }` (campo aditivo; bridge v1, `RUNTIME_VERSION` 0.1.0
inalterado). Outros codecs só valem se a prova de leitura passa naquela engine.

## B1 — causa e prova

- Causa: `new THREE.Texture(video)` + `WebGLTextures` do r187 aloca com `image.width/height`; num `HTMLVideoElement`
  isso é o atributo de conteúdo (0). Erro do GL: `glTexStorage2D: One or more image dimensions are not positive`.
- Antes: todos os quadros pretos (Chrome SwiftShader e WebKit), `asset.add kind=video path=…` + `render.frame` →
  `mean=[0,0,0]`. Depois: `crates/render/tests/video.rs::a_container_video_added_by_path_…` passa; com o bundle antigo
  falha com `[(47, 0), (1, 0), (46, 0), (24, 0), (23, 0), (25, 0)]` (código lido = 0 = preto).

## B4 — diagnóstico

Medido com o harness `runtime/scripts/video` (WKWebView real, janela visível, release do runtime = mesmo bundle):

| Experimento (WebKit) | Resultado |
|---|---|
| `createImageBitmap(video)` **sozinho**, 60 seeks aleatórios, 1080p H.264 e HEVC | p50 1–2 ms, **p99 1–2 ms**, 0 errados |
| `drawImage` + `getImageData` e **depois** `createImageBitmap` do mesmo quadro | o primeiro `createImageBitmap` leva **1001–1005 ms** (índice 0 em 3/3 clipes); depois 1–2 ms |
| Aquecer (`createImageBitmap` antes e/ou com seek) | **não** remove: o 1 s aparece na primeira leitura mista após o aquecimento |
| `texImage2D(video)` estável | 1–2 ms (p99 ≤ 5 ms); com janela **oculta**, o primeiro upload levou ≈ 1025 ms |
| `requestVideoFrameCallback` | ≈ 50 % de *timeout* em 400 ms mesmo com o vídeo no DOM: não serve de barreira |
| Seek (`seeked`) | 5–21 ms (1080p), nunca perto de 1 s |

Conclusão: não é decode, GOP nem alvo de seek. A parada é a **primeira leitura mista** de quadros do mesmo `<video>`
(provavelmente uma espera de ≈ 1 s do WebKit por um novo quadro apresentado); a correção é **ter um só caminho de
leitura**, que é a arquitetura do B1/B2 (nada mais toca o elemento). Prefetch cego continua refutado e não foi usado.
**Não confirmado:** a causa interna do WebKit (inferida só do comportamento), e o primeiro upload de ≈ 1 s com a WebView
oculta (o runtime agora nem usa `texImage2D(video)`; a leitura por bitmap com janela oculta **não foi medida**).

Antes/depois no mesmo critério do Opus (entrada a 30 Hz, um quadro em voo, o mais novo vence, H.264 1080p GOP 24):

| Cenário | Antes (Opus) | Depois |
|---|---|---|
| WebKit, forward, 120 entradas | p99 ≈ **1016 ms**, 91/120 apresentados | 120/120, latência p50 20 / p95 23,7 / **p99 24,7** ms, 0 acima de 100 ms |
| WebKit, aleatório 1080p (frio) | — | p50 29,7 / p95 37 / p99 40,3 ms, 0 acima de 100 ms |
| WebKit, forward 540p | — | p99 13 ms |
| Chrome GPU 1080p forward / aleatório | — | p99 26,9 / 26,6 ms |
| **Janela real, binário release** (`seek` → monitor desenhado, `scripts/video-live-proof.py`) | — | 540p (n=48) p50 16,1 / p95 24,3 / p99 24,5 ms; 1080p aleatório (n=120) p50 33,7 / p95 45,6 / **p99 47,1** ms |

Eles são **frios** (cada quadro é novo; o cache de frames só ajuda no quente). Relógios de origens diferentes não foram
comparados: o "antes" é o número do Opus, o "depois" vem do harness e da janela com os mesmos 120 estímulos.
Tempos do runtime: `frame.done.timings` agora traz `video_seek_ms` e `video_read_ms`.

## B5

O desenho de RT único era do protótipo. Produção mantém RT **por camada** (`retained()` + `sceneTarget(order…)`), coberto
por `runtime/test/retention-browser.test.mjs` (duas cenas, orçamento, perda de contexto). Nenhuma mudança.

## B6 — reproduzido sem ponteiro do SO

`crates/desktop/src/workstation/input_tests.rs` (entrada injetada no GPUI, **sem** clique por coordenada do SO):

- `dragging_on_the_ruler_follows_the_pressed_pointer`: o scrub legítimo (botão pressionado) continua funcionando.
- `moves_without_a_pressed_button_do_not_scrub_after_a_lost_mouse_up`: **falhava** antes (playhead 40 → **90** com um
  movimento sem botão); passa depois.

Causa: no `gpui` 0.2.2 `on_drag_move` roda em **todo** `MouseMove` enquanto existe um valor de drag, com ou sem botão, e
só `MouseUp` o limpa. Se o `MouseUp` não chega (solto sobre outra janela, gesto injetado que para no meio), o hover
arrastava o playhead. Correção (`timeline_panel.rs`): movimento sem botão esquerdo encerra o drag (`stop_active_drag`) em
vez de dar seek. Todo movimento de um drag genuíno traz o botão. **Limite honesto:** que o `MouseUp` realmente se perca
no macOS com um usuário real **não foi provado**; o que está provado é a regra de entrada e o conserto dela. Os arrastos
de clipe/trim/trilha têm a mesma forma e **não** foram alterados (fora do relato e sem reprodução).

## B7

`crates/core/src/assets.rs`: `probe_frame_sequence` lê PNG **ou** JPEG (SOF por cabeçalho, até 1 MiB de metadados),
exige **um formato** (`frame-sequence/png` | `frame-sequence/jpeg`), **um tamanho** e **todos os quadros presentes**
(só nomes: um buraco falha ao adicionar, não no meio da reprodução); `expand_pattern` aceita `%0Nd` e `%d` (como o
runtime e o servidor de assets). Rejeitados: arquivo que não é PNG/JPEG, formatos misturados, tamanho diferente,
quadro faltando, padrão sem número. Nenhuma conversão com perda de cache original: o JPEG só existe se o chamador o
forneceu.

Medido (540p, 960×540, entrada a 30 Hz, aleatória, 1 quadro em voo; harness):

| Engine | Formato | Frio (48) p50 / p95 / p99 | Misto (240) p50 / p95 / p99 |
|---|---|---|---|
| WebKit | PNG | 11,3 / 17 / 42 ms | 13,7 / 20,7 / 23,7 ms |
| WebKit | JPEG | 9 / 14 / 15,3 ms | 10 / 17 / 21 ms |
| Chrome GPU | PNG | 5,7 / 13,3 / 18,7 ms | 6,1 / 13 / 14,8 ms |
| Chrome GPU | JPEG | 5,6 / 10,2 / 12 ms | 3,8 / 8,2 / 12,1 ms |

Todos 100 % apresentados, 0 acima de 100 ms. PNG 540p (pendente no Opus) fica ≈ 2–4 ms atrás do JPEG: a escolha de
proxy é de produto, ambos servem.

## B8

`runtime/src/index.js` (`renderFrame`): a perda de contexto durante o quadro (contador `contextLosses`, flag e
`gl.isContextLost()`) vale em **qualquer** desfecho:

- quadro que "terminaria bem" num contexto morto (three.js segue chamando no-ops; `readPixels` dá zeros) agora falha
  com `context_lost` (antes: `frame.done` com pixels vazios);
- outra falha durante a perda (`render_failed` do Spark) é reportada como `context_lost` com a original em
  `details.cause`; `cancelled` não é reclassificado.

Prova antes/depois (`runtime/scripts/video/run.mjs --suite loss`; imagem, cena, splat; perda **na mesma tarefa** do pedido):

| Caso | Antes (WebKit = Chrome) | Depois (WebKit = Chrome) |
|---|---|---|
| imagem | `frame.done` (pixels de um contexto morto) | `frame.failed` `context_lost` |
| cena | `frame.done` | `frame.failed` `context_lost` |
| splat | `frame.failed` `render_failed` | `frame.failed` `context_lost`, `cause: render_failed` |

Em todos, depois de `restoreContext` o quadro seguinte desenha com o **mesmo hash** do quadro anterior à perda.
Vídeo coberto no teste de Chrome (perda durante o *seek*). Sessão/caches: o `context.lost` já falhava o quadro ativo e
limpava assets na sessão Rust (testes existentes); `dropGpu` e o `asset.generation` não mudaram.

## Arquivos

Runtime: `native-video.js` (novo), `assets.js`, `compositor.js`, `index.js`. Core: `assets.rs`, `headless.rs`
(`unsupported_timing` → `unsupported_format`). Desktop: `timeline_panel.rs`. Harness (fica no repo, artefatos não):
`runtime/scripts/video/{make-fixtures.py,run.mjs,page/*}`, `scripts/video-live-proof.py`. Testes: `runtime/test/video.test.mjs`,
`crates/render/tests/video.rs`, `crates/core/src/assets.rs` (módulo `tests`), `crates/desktop/src/workstation/input_tests.rs`.
Docs: este arquivo, `features/viewport.md`, `features/media-io.md`, `features/README.md`, `ui-automation.md`, relatórios G4/G7.

## Reproduzir

```sh
./scripts/runtime build
(cd runtime && TESSEL_REQUIRE_RUNTIME=1 pnpm test)                         # 39 testes (32 + 7 novos), Chrome
TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --locked --workspace           # inclui crates/render/tests/video.rs
./scripts/cargo build --locked --workspace --release && python3 scripts/video-live-proof.py   # janela real, HOME isolado

python3 runtime/scripts/video/make-fixtures.py /tmp/tvb2/fx --big           # fixtures com barcode (fora do git)
mkdir -p /tmp/tvb2/rt/before/dist && cp <runtime.js anterior> /tmp/tvb2/rt/before/dist/runtime.js
(cd runtime && node scripts/video/run.mjs --engine webkit --suite verify --variants before,current)
(cd runtime && node scripts/video/run.mjs --engine webkit --suite paced --variants current --clips h264-g24-1080-240.mp4 --hz 30)
(cd runtime && node scripts/video/run.mjs --engine webkit --suite loss --variants before,current)
(cd runtime && node scripts/video/run.mjs --engine webkit --suite raw --only bitmap --clips hevc-g24.mp4)   # diagnóstico B4
```

`--engine` aceita `webkit` (WKWebView real: `scripts/viewport-validation/vv-wk.swift`), `chrome-gpu` e
`chrome-swiftshader`. O bundle `before` é o `runtime/dist/runtime.js` anterior a esta entrega (a correção está em
`runtime/src`); esta entrega não guarda esse arquivo.

## Resultados

- Runtime: **39/39** (32 anteriores + 7 novos em `video.test.mjs`: B1+B2 em 4 ordens, duas camadas, quadro
  ultrapassado, capacidades/erros, VP9, PNG×JPEG, B8 em 4 tipos). O teste B8 **falha** com o bundle antigo.
- Rust: workspace **333 passaram, 0 falharam, 2 ignorados** (novos: 5 em `core::assets`, 5 em `render/tests/video.rs`,
  2 em `desktop::input_tests`). `crates/render/tests/video.rs::a_container_video…` falha com o bundle antigo; o de
  scrub do ruler falhava antes da correção. `cargo fmt --check`, `clippy -D warnings`, `build --locked` (debug e release) limpos.
- Janela real (release, HOME isolado, só comandos semânticos): 20 checagens de quadro exato (H.264, PNG, JPEG × 4 ordens) e
  WebM = `unsupported_format` tipado, 0 falhas.
- Export: `an_imported_clip_exports_with_the_right_frames_rate_and_pictures` (clipe importado → PNGs do FFmpeg, 16 quadros
  de `[4,20)`, `ffprobe`: `nb_read_frames=16`, `24/1`, 640×360; barcode dos quadros 0/7/15 = fontes 4/11/19). A exportação
  determinística não foi tocada.

## Incertezas e dependências

- Causa interna do WebKit no B4 é inferida do comportamento; o caso de janela **oculta** não foi remedido com o leitor novo.
- Medidas de latência são de um Mac, janela visível, vídeo sintético; o valor absoluto depende da máquina.
- Vídeo por `asset.add path=…` **confia** na taxa/duração declaradas; VFR ou início deslocado não declarados passam sem
  aviso no Chrome (o WebKit recusou o deslocado por acidente). O caminho importado é o garantido.
- AV1/VP8/VP9 no WebKit dependem da política de origem do WebKit; a prova de leitura é o que decide, não esta tabela.
- B6 em uso real (perda de `MouseUp`) não foi provado; *seek→photon* e áudio **ouvido** continuam sem prova e
  não foram marcados.
