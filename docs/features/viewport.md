# F021 — Viewport multimídia e composição

## Objetivo e limites

Um único monitor mostra a **sequência composta** ou **uma mídia isolada**. O mesmo plano de
render alimenta três consumidores: o worker headless (Node + Chrome), a WebView persistente do
monitor e o mixer de áudio offline. O core decide tempo, camadas, câmera e trims; o runtime
JS só desenha o que o plano manda e não repete regras de edição.

**Entregue e validado** (ver [relatório](../viewport-foundation-report.md) para números):
imagem, vídeo por sequência de frames, texto (fonte bitmap 5×7), cena Three.js procedural e
Gaussian splat via **Spark 2.4.0** ([F030](spark.md)), compostos em camadas por track; waveform de áudio no preview de
fonte; mix de áudio **processado offline**.

**Entregue depois da fundação** (contratos próprios): import por UI e CLI ([F019](media-io.md)), playback com
relógio ([F018](playback.md)), export de vídeo com áudio ([F029](video-export.md)). **Não entregue**: sincronismo A/V
*ouvido* e garantia de decode de qualquer codec em qualquer engine. O áudio é *processado* (amostras
verificáveis num WAV) ou *tocado por um processo de player*, nunca declarado como *ouvido*.

| Conteúdo | Preview de fonte | Papel na composição | Evidência |
|---|---|---|---|
| Imagem (png/jpeg/webp/gif/bmp) | Imagem com fit | Camada 2D com transform, opacidade, blend | Headless + viewport: 4 quadrantes por pixel |
| Vídeo (`frame_sequence` PNG/JPEG; contêiner `mp4` H.264 por `<video>` com [contrato de tempo](../viewport-bugfix-report.md#contrato-de-tempo-b2-e-o-que-a-engine-consegue-b3)) | Quadro no tempo da fonte | Frame mapeado com trim/speed pelo core | Headless + viewport: cor do frame 30 e 47; barcode por quadro em qualquer ordem ([relatório](../viewport-bugfix-report.md)) |
| Áudio (`wav` PCM) | Waveform | Só no mix (`render.audio`), sem pixels | Amostras do WAV mixado; **não** ouvido |
| Texto (inline) | Texto com estilo | Camada 2D | Pixels brancos na posição esperada |
| Cena `tessel-scene@1` | Cena e câmera | Camada 3D com câmera do clipe | Centro da cena muda com tempo e câmera |
| Splat (`.splat`, PLY Gaussian, `.spz`; Spark 2.4.0) | Nuvem | Dentro de cena | Mesmo frame nos três formatos; ver [F030](spark.md) |

Suportar uma categoria não é suportar todos os codecs: `capabilities` lista o que o backend
conectado aceita e o resto volta como erro tipado (`unsupported_format`).

## Contrato do core / biblioteca

Tudo abaixo existe e é testado sem GPUI (`crates/core`):

- `PreviewIntent::{Sequence, Source { asset, time }}` em `EditorState::preview`; comando
  `Command::SetPreview`. Trocar a seleção **não** muda a intenção.
- `EditorState::plan(&PlanRequest) -> RenderPlan`: camadas ordenadas por track, transform, opacidade, blend, tempo local da fonte (`Ratio`), câmera da cena,
  avisos (`Warning`). Independe de seleção; aceita `frame` ou `time` racional.
  `EditorState::plan_audio(from, frames) -> AudioPlan` + `OfflineMixer` (processado).
- `Ratio`/`Rate` racionais (ex.: 24000/1001); sem acumulo de float.
- `trait RenderBackend` (capabilities, load/release asset, request/cancel frame, resize,
  eventos) com `UnavailableBackend`; `trait Transport` + `BridgeBackend` falam a bridge v1.
- `ViewportSession<B>`: revisão ativa, `request_frame` (pedido **explícito**: supersede o que está em curso),
  `request_preview` + `dispatch_pending` (pedido **interativo** do monitor, ver
  [agendador](#agendador-de-preview-scrub)), `cancel`, `pump`, descarte de resposta velha
  (`stats.dropped_stale`), `release_except`, `reset_project`, `shutdown`.
  `render_blocking` é para CLI; **a UI nunca bloqueia** (`known_capabilities` + `pump`).
- `ops::parse_op` traduz o vocabulário JSON de projeto (ver [F015](project-creation.md)).

```rust
use tessel_editor_core::{EditorState, headless};
use tessel_editor_core::state::ProjectSettings;
let mut state = EditorState::empty(&ProjectSettings::default())?;
let plan = headless::execute(&mut state, &serde_json::json!({"op":"plan","frame":0}))?;
```

Erros (`CommandError.code`) mais comuns: `unknown_element`, `invalid_request`,
`invalid_value`, `invalid_id`, `invalid_time`, `invalid_preview`, `no_selection`,
`backend_unavailable`, `unsupported_format`, `asset_unreadable`, `decode_failed`,
`render_failed`, `context_lost` (também quando a perda ocorre no meio do quadro, com a causa em `details.cause`),
`timeout`, `cancelled`. `unsupported_timing` do runtime (VFR/sem taxa) chega ao CLI como `unsupported_format` com
`details.backend_code`. Uma rejeição não altera o estado
(`failed_commands_do_not_change_state`, `a_rejected_operation_changes_nothing`).

### Agendador de preview (scrub)

O monitor pede quadros por `ViewportSession::request_preview`; CLI/provas/export pedem por `request_frame`.

- Com um quadro **interativo já enviado ao runtime**, uma intenção nova que só move o tempo
  (o projeto, replanejado no tempo do quadro em curso, ainda produz exatamente o plano em curso; mesmo
  assunto — sequência ou a mesma mídia —; mesmo tamanho) **não cancela** o quadro: vai para um slot único
  (`pending`, a mais nova vence; `stats.coalesced` conta). Quando o quadro em curso é aceito
  (`accepted`), `dispatch_pending` envia a posição pendente. O quadro em curso é apresentado: é um
  quadro intermediário real, não uma resposta velha aceita por engano.
- **Continuam invalidando** (cancelam o quadro em curso, e a resposta dele é descartada como `Stale`):
  qualquer edição que mude o plano (trilha oculta, texto, clipe…), troca de assunto/tamanho, pedido
  explícito (`render.frame`), `reset_project`/reload, perda de contexto, falha do quadro, timeout (30 s) e
  quadro ainda esperando assets. Um quadro explícito em curso nunca recebe fusão: um seek o cancela, como antes.
- Falha de um quadro: o erro é mostrado e a posição pendente é descartada (nada recomeça sozinho);
  o próximo movimento tenta de novo.
- `session.presented` (`request`, `frame`) diz qual posição está na tela; é o que uma prova compara com o playhead.
- Leitura de pixels: o quadro **interativo** vai com `inspect:false` e o runtime só desenha (não faz
  `readPixels` nem hash). Quadro explícito, com `probes` ou `out` PNG nunca pula a leitura. Para medir/provar
  imagem com o monitor, inicie o editor com `TESSEL_VIEWPORT_INSPECT=1` (`session.inspect_previews`
  vira `true` e `last_frame` volta a trazer `hash`/`mean`). `last_frame.inspected` e `hash == ""` indicam
  "não medido" (nunca "preto").

### Proxy automático de preview (OPT01)

Política (`EditorState.preview_policy`, comando `SetPreviewPolicy`, **não é edição**: sem histórico, sem revisão):
`mode` `auto` (padrão) ou `original`, `max_height` (padrão 540, 144–4320) e `format` `jpeg` (padrão) ou `png`.
`op preview.policy` sem campos devolve o relatório (`crates/core/src/preview.rs`), com campos aplica; o nó
`monitor.preview.quality` (toggle "Fast preview (proxy and lighter MSAA while playing or scrubbing)") alterna `auto`/`original`:

```sh
./scripts/ui op '{"op":"preview.policy"}'                       # o que está valendo e por quê, por asset
./scripts/ui op '{"op":"preview.policy","mode":"original"}'     # sem proxy: o monitor volta ao original
./scripts/ui op '{"op":"preview.policy","max_height":720,"format":"png"}'
```

O relatório diz `interactive_quality`, `export:"original"`, `explicit_frames:"original"` e, por asset, `reason`:
`proxy_ready` (com `proxy.{pattern,first,count,width,height,format}`), `proxy_missing` (ainda não gerado), `proxy_stale`
(o proxy é de outros frames), `small_enough` (o original já não é maior que o proxy), `no_frames` (ainda não decodificado),
`not_video`, `policy_original`. **A qualidade nunca fica escondida**: o cabeçalho do monitor ganha " · proxy" quando a
imagem veio de um proxy (e " · MSAA n" quando uma cena com splat foi desenhada com menos amostras, BL129) e `last_frame.source` traz
`{quality, proxy_layers, original_layers, result_hits, proxy_error?, scene_samples?, reduced_scene_layers?}`.

Regras (as mesmas em lib/CLI/UI, `Quality` decidida na sessão):

- Só o **quadro do scrub do monitor sem PNG nem probes** usa `Preview`. `render.frame`, export, PNG, probes e o quadro de
  refino usam `Original`. O export nunca vê o proxy (`Derived::for_export`; provado frame a frame, ver o relatório).
- **Refino:** depois de uma imagem **reduzida** (de proxy, ou de cena com splat com MSAA menor; `FrameDetail::is_reduced`) ser aceita, a sessão guarda o plano; após `REFINE_DELAY` (200 ms) sem intenção
  nova um quadro `Original` não fundível o substitui (no desktop um timer de 220 ms, não o tick de 500 ms). Qualquer
  intenção, edição ou pedido explícito descarta o refino pendente (`session.refine_pending`, `stats.refined`).
- **Geração do proxy:** o `MediaHost` agenda o job "Build preview proxy" depois do decode (`Cache::preview_frames`, FFmpeg
  `scale` + `-start_number` do decode, mesma numeração e contagem, altura exata e largura par; chave = padrão dos frames
  decodificados) e a op `media.preview_proxy` o pede à mão (`policy_original` se a política é `original`). Aplicar é uma
  transação com hash (`Pending::Proxies` → `state.set_preview_proxy`, que recusa proxy de outros frames); falha
  não se repete sozinha. Só vídeo container com frames decodificados; sequências PNG/JPEG não ganham proxy.
- **Bridge (aditivo, v1):** `asset.proxy {id, proxy|null}` → `{state: attached|detached|failed}`, só enviado se
  `capabilities.features` contém `asset.proxy`; `frame.render.quality:"preview"` (ausente = original, mensagem antiga
  idêntica); `frame.done.source`. O runtime cai para o original **por camada** se o proxy falhar e o relata em
  `source.proxy_error` (`session.proxy_error` guarda o último).
- **Troca de fonte:** um asset desenhado antes do decode terminar (container nativo) é recarregado a partir dos frames
  quando o decode chega (`Session::loaded_from`); sem isso o runtime não aceitaria proxy (`unsupported_format`) e um
  `asset.load` novo com o mesmo ID era o único caminho.

### Cache do resultado de cena Spark parada (OPT02)

O runtime guarda o **resultado** (render target) de uma cena com splat que não mudou: chave = cena+serial, seriais e
parâmetros dos splats, câmera, tamanho, tempo da cena só se animada, amostras MSAA e `contextGeneration`. Só cenas com
splat entram (`hello.result_cache:false`/`TESSEL_VIEWPORT_RESULT_CACHE=0` desliga). Perda de contexto, troca de asset
com o mesmo ID, tamanho, câmera ou tempo animado são desenho real, com pixels iguais aos do caminho sem cache (teste
no runtime). **Só vale para cena parada**; em movimento nada muda (ver relatório: 1M parado 12,5 → 1,0 ms de CPU).

### Repaint seletivo e ritmo (OPT03/OPT04)

- **OPT04, entregue:** `viewport_ipc` só chama `cx.notify()` quando o que o cabeçalho/erro do monitor mostra mudou
  (`ViewportHost::ui_signature`); `frame.presented`, `asset.proxy`/`asset.state` sem efeito visível e quadros com o mesmo
  texto não repintam o GPUI. A imagem em si é o canvas da WebView, que a página desenha. Contadores em
  `render.status.ui` (`repaints`, `skipped`). Não é FPS.
- **OPT03, medido e removido:** segurar o próximo quadro do scrub até `frame.presented` (rAF) não deu ganho (tabela no
  relatório); o código foi retirado. `frame.presented` continua sendo só telemetria do `TESSEL_VIEWPORT_TRACE=1`.
- **OPT07, opt-in:** `hello.scene_samples` / `TESSEL_VIEWPORT_SCENE_SAMPLES=0|2|4` troca MSAA **final** das cenas (padrão 4).
  Com 1M em órbita o quadro drenado vai de 35 → 27 (2) → 21 ms (0): como opção do processo inteiro piora o quadro final;
  por isso o padrão não mudou e o BL129 (abaixo) reduz só enquanto o playhead anda.

### MSAA por movimento (BL129)

Cena **com splat** desenhada num quadro `Preview` (playback e scrub do monitor, sem PNG nem probes) usa
`hello.preview_scene_samples` amostras (**padrão 0**; `TESSEL_VIEWPORT_PREVIEW_SCENE_SAMPLES=0|2|4`; 4 desliga a redução; nunca
passa de `scene_samples`). Pausa, fim do scrub, `render.frame`, export, PNG, probes e o quadro de refino são `Original` e usam
`scene_samples` (padrão 4): nada disso mudou. Cena sem splat (geometria procedural, barata) não é reduzida.

- **Sem controle novo:** a política é automática e é a de `monitor.preview.quality` (`original` = MSAA final também no
  playback). A qualidade nunca fica escondida: cabeçalho " · MSAA n" e `last_frame.source.scene_samples` /
  `reduced_scene_layers` (campos aditivos do `frame.done`; runtime antigo não os envia e o quadro é tratado como final).
- **Nunca preso em baixa qualidade:** a sessão guarda o plano de um quadro interativo reduzido (`FrameDetail::is_reduced`) e,
  200 ms sem intenção (timer de 220 ms no desktop), pede o quadro `Original`; nova intenção, edição, pedido explícito ou
  troca de política descartam/substituem o refino. O refino acontece sozinho ao pausar e ao soltar o scrub.
- **O cache de resultado não serve quadro reduzido como final:** a chave do resultado da cena e o alvo MSAA retido trazem as
  amostras do quadro; um alvo com outra contagem é recriado (testes em `runtime/test/preview-samples.test.mjs`).
- Cenas dirigidas (F048) seguem sempre com `scene_samples`.

Evidência BL129 (release, M3 Pro, GPU compartilhada, cópia em `/tmp` do projeto 1280×720/24 fps com 4 cenas SHARP de 1.179.648
gaussians; repetições de 3 corridas por sessão, média (mín–máx); `scripts/preview-quality-proof.py`, relógio do próprio editor com
`TESSEL_VIEWPORT_TRACE=1`). "Antes" = `origin/main` em `01c0f9a` (2 sessões); "MSAA 0" = este PR sobre ele (3 sessões; a
sessão 2 rodou com a CPU disputada por outros builds, load average ≈ 15 contra ≈ 3, e concentra todo o pior caso);
"MSAA 2" = `TESSEL_VIEWPORT_PREVIEW_SCENE_SAMPLES=2`, medido antes da fusão do BL131 (2 sessões):

| Medida | Antes | MSAA 2 | MSAA 0 (padrão) |
|---|---|---|---|
| Take A a 24 fps, quadros sem imagem | 22,6 % (13,8–31,2) | 1,3 % (0–7,6) | **0 %** (0–0) |
| Crossfade (14 quadros), quadros sem imagem | 57,1 % (57,1–57,1) | 39,3 % (28,6–50,0) | 4,8 % (0–28,6); 0 % fora da sessão disputada |
| Desenho no runtime, take A (p50) | 50,0 ms (45–59) | 19,0 ms (14–34) | 14,8 ms (14–17) |
| Scrub do take A, imagens por segundo | 19,2 (17,5–20,0) | 28,2 (27,0–29,5) | **46,2** (39,0–49,5) |
| Scrub do crossfade, imagens por segundo | 19,2 (17,5–21,0) | 26,2 (24,0–27,5) | **42,4** (39,0–46,5) |
| Atraso do scrub (quadros, mediana) | 3,0 | 2,0 | 0,6 |
| Parou → quadro final de volta | — (sem refino de MSAA) | 294 ms (211–420) | 342 ms (220–430) |

Na sessão sem disputa o scrub fica em 41–49,5 imagens/s e o crossfade não perde quadro; sob load 15 o scrub cai para 39–42,5.

Escolha 0 em vez de 2: 2 amostras não fecham o aceite (scrub < 40 imagens/s e quadros perdidos no take A e no crossfade),
e 0 é indistinguível de 4 neste conteúdo: diferença máxima por canal de 3–8/255, nenhum pixel acima de 16/255, PSNR ≈ 66 dB
em relação ao MSAA 4 (2 amostras: 66,5–66,8 dB). Gaussians translúcidas não têm silhueta dura, então a redução só custa
o raster. Imagens em `/Users/wallyssonnunes/www/tessel-video-splat/previews/perf-bl129/`, fora do repositório.

- `render.frame` explícito e export idênticos: hash dos quadros 5, 60, 100, 193, 300 e 450 igual antes e depois, e 6 quadros
  de crossfade exportados (190–195, `fullhd`, medido antes da fusão do BL131) com o mesmo MD5 decodificado antes e depois.
- Depois de parar, o quadro na tela (`last_frame.source`: `scene_samples` 4, `reduced_scene_layers` 0) tem o mesmo hash que o
  `render.frame` do mesmo quadro (take A e crossfade).
- Não provado: outros conteúdos (cena com muita geometria opaca fora de splat não é reduzida por regra; um splat com silhueta
  dura pode mostrar serrilhado enquanto o playhead anda e se refina ao parar), outras máquinas/GPUs, e áudio ouvido (o projeto de teste
  rodou com as trilhas de áudio mudas, relógio de parede). O intervalo médio entre imagens no take A foi 41,6 ms (24,05/s).

### Retenção de GPU no runtime

O runtime não recria mais textura, material, programa e alvo de render a cada quadro. Tudo que é mantido tem dono,
limite e ciclo de vida (`runtime/src/retention.js`, `assets.js`, `compositor.js`):

| Objeto | Dono / chave | Limite | Morre quando |
|---|---|---|---|
| Textura de imagem e de frame de sequência | `AssetStore`, por asset (e frame) | 256 MiB (LRU por último uso; o que o quadro em curso usa fica fixo) | `asset.release`, troca do mesmo ID (nova fonte/hash), perda de contexto, estouro do limite |
| Textura de texto | `Compositor`, chave `[conteúdo, tamanho, cor, altura]` | 32 entradas / 32 MiB (LRU) | estouro, troca de contexto, `retain:false` |
| Material de camada (e, portanto, o programa) | pool `blend:slot` | 24 | perda de contexto, `retain:false` |
| Rig de cena (**sem** splats) | por asset + identidade da entrada + geração do contexto | 4 | `asset.release`, troca do asset, perda de contexto |
| Alvo MSAA de cena | por slot e tamanho | 2 / 192 MiB estimados (`w·h·4·(amostras·2+1)`) | estouro (tamanho de export nunca é retido), perda de contexto |

Regras que a implementação mantém (testadas em `runtime/test/retention*.test.mjs`): o `asset.generation` sobe a cada
carga/liberação/`dropGpu` e um quadro que vê a entrada do asset mudar no meio do desenho termina em `cancelled`
(nunca mistura fonte velha com nova); objetos retidos são **descartados na perda de contexto, antes da restauração**
(o Three.js 0.187 criaria `WebGLTextures` novos e os `dispose` antigos apontariam para o contexto errado); depois do
descarte nasce um objeto novo, nunca um `Texture` já descartado reaproveitado; cena com splat **nunca** usa rig retido,
então o Spark ordena e acomoda a cada quadro (export e headless não pulam o *settle* por cache); o pixel é o mesmo com a
retenção ligada, desligada (`hello.retain:false`) e no runtime anterior (provas por hash em ordem embaralhada).

**O que os números significam.** `render_ms` é tempo de **CPU** no runtime (`prepare_ms` resolve assets e monta o que
desenha, `draw_ms` grava as chamadas, `flush_ms` entrega os comandos ao processo de GPU com `gl.flush()`). `gl.finish()`
foi trocado por `gl.flush()` no caminho sem leitura: no WebKit e no Chromium `finish()` não espera a GPU, só parecia
esperar por demorar ≈0 ms, então nunca provou que o quadro terminou. **Nada disso é tempo de GPU.** A única medida de
"a GPU terminou" é a leitura de 1×1 pixel após o quadro (`wall+drain` no harness), um **limite superior** que inclui a
ida e volta do IPC. O relógio do WebKit tem passo de 1 ms: médias sobre N ≥ 300 quadros, nunca um p50 de submilissegundo
lido como medida.

### Limites da retenção e bugs registrados

A retenção tira o custo **repetido** (textura, programa, alvo, texto). Não muda o que segue abaixo; cada item tem a
medida em [viewport-performance-implementation-report](../viewport-performance-implementation-report.md):

- **Primeiro quadro (frio)** continua igual ao anterior: criar programa/textura/alvo custa o mesmo. Só o quente muda.
- **Splat em movimento** (Spark): sem rig retido, ordenação e *settle* a cada quadro; ganho pequeno (≈ 20–25 %), nunca
  1–4 ms. Não há cache de alvo para cena de Spark parada.
- **Vídeo com *cache miss*** (decodificar o quadro): sem ganho relevante; dominado pela decodificação.
- **Vídeo em contêiner** (`<video>`): **corrigido** em [viewport-bugfix-report](../viewport-bugfix-report.md) (histórico
  B1–B4 na [validação do Opus](../viewport-performance-opus-validation.md#bugs-achados)). Contrato: taxa constante,
  primeira imagem em PTS 0, quadro `f` = intervalo `[f/r,(f+1)/r)` lido no **meio**; um só leitor
  (`createImageBitmap`), snapshot no cache de frames; prova de leitura no `asset.load` (H.264/HEVC exatos no WebKit e no
  Chrome; VP8/VP9/AV1 exatos só no Chrome; no WebKit viram `unsupported_format`). `webm` **não é prometido**
  (`FORMATS.video = frame_sequence, mp4`; `capabilities.native_video`). O vídeo **importado** continua indo por PNG do
  FFmpeg. Os casos `video-hit`/`video-miss` do harness medem só o custo de cache. B5 (RT único) era do protótipo e é
  coberto pelo teste de duas cenas; B6 (scrub sem botão), B7 (sequência PNG/JPEG) e B8 (`context_lost` no meio do quadro)
  também estão no relatório.
- **Display e cadência**: o *rAF* da tela de teste é ≈ 60 Hz e o ritmo aceito (`accepted`) não é o ritmo mostrado.
  `frame.presented` é só um sinal de que a página pôde desenhar.

### Spans do host (opt-in)

`TESSEL_VIEWPORT_TRACE=1` liga `TraceHandle` (`crates/core/src/trace.rs`) e pede ao runtime `frame.presented`.
Desligado, cada ponto de medição custa uma leitura atômica relaxada e nenhum relógio. Ligado: uma leitura de relógio por
span e um mutex sem contenção por registro. Spans (todos no relógio monotônico do host, em ms):

| Span | O que é |
|---|---|
| `plan` | `EditorState::plan` do `sync` (e `plan_key`: serializar o plano para a chave de "nada mudou") |
| `sync`, `sync_unchanged` | o `sync` inteiro; o segundo é o caso "nada mudou, nenhum quadro pedido" |
| `prepare` | sessão: checagens de asset e `load_asset` antes de enviar (não inclui a espera por assets) |
| `serialize`, `dispatch` | montar a mensagem `frame.render`; entregar ao transporte (para a WebView: texto + fila) |
| `evaluate` | a janela entregando o script à WebView (retorna antes de a página rodar) |
| `transit` | **estimativa**: (envio → resposta aceita) − `total_ms` do runtime: fila, avaliação, volta e a thread de UI pegando a resposta |
| `runtime_total`, `runtime_render` | o que o runtime reportou (relógio da página) |
| `accept` | a sessão tratando a resposta |
| `ipc` | `on_ipc` inteiro (JSON, sessão, `dispatch_pending`, novo `sync`) |
| `serve` | `tessel://` servindo página ou asset |
| `presented_after_done`, `submit_to_presented` | só com `frame.presented`: *rAF* após `frame.done` (relógio da página) e o total derivado |

Não medido: o `render` do GPUI em volta da WebView e o que o compositor do macOS faz depois do *rAF*. `frame.presented`
é um sinal de "a página teve a chance de desenhar", **não** prova de que o pixel chegou à tela (não dispara com a vista
oculta). Cada quadro aceito guarda `submitted_ms`/`done_ms` (relógio do host desde a criação do trace) e `shown_frame`:
é daí que a prova do scrub lê o ritmo real, sem sondar o editor de fora. Leitura: `session.trace` em `render.status`
(últimos 128 quadros) e no nó `monitor.viewport` (últimos 8). Janelas: 512 amostras por span, 256 quadros.

### Bridge Rust↔JS v1

JSON por mensagem, campo `v: 1`. Host→página: `hello`, `asset.load`, `asset.release`,
`frame.render`, `frame.cancel`, `viewport.resize`, `dispose`. Página→host: `page.ready`,
`ready`, `asset.state`, `frame.done` (PNG base64 só se `output.png`), `frame.failed`,
`context.lost`, `context.restored`, `closed`, `error`. Campo aditivo (compatível, v1):
`frame.render.inspect:false` (ausente = ler e descrever, o que todo host antigo faz); `frame.done` ganha
`inspected` e `timings.{settle_ms|readback_ms, describe_ms}`; sem leitura, `hash`/`mean` vêm `null`.
Mais campos aditivos (BL129, ainda v1): `hello.preview_scene_samples` e `frame.done.source.scene_samples|reduced_scene_layers`. Mais campos aditivos (OPT01/02/07, ainda v1): `asset.proxy`, `frame.render.quality`, `frame.done.source`, `hello.result_cache`, `hello.scene_samples` e `capabilities.features` `asset.proxy`/`frame.quality`. Mais campos aditivos (entrega anterior, ainda v1): `hello.retain:false` (volta ao caminho de criar/descartar por quadro; ausente =
retenção ligada) e `hello.telemetry.presented:true` (pede `frame.presented`, que **só** é enviada a quem pediu: o
`decode_event` de um host antigo rejeita tipo desconhecido); `ready.capabilities.features`
(`frame.presented`, `cache.telemetry`); `frame.done` ganha `timings.{prepare_ms, draw_ms, flush_ms}` e `cache`
(textura/texto/material/rig/alvo × criado/reaproveitado/expulso e o que está retido). Campos antigos e `settle_ms`
continuam. `HelloOptions` padrão gera a mesma mensagem de antes, byte a byte (`hello_defaults_stay_byte_identical…`).
Mais um campo aditivo (BL102, ainda v1): `asset.load.asset.request` (inteiro) e o eco `request`/`scene` em todo `asset.state`, negociados por `capabilities.features` `asset.identity`; só é enviado a runtime que o anuncia, e o runtime antigo ignora. Contrato em [scene-renderer.md](scene-renderer.md#identidade-de-carga-em-assetloadassetstate-bl102).
Versão incompatível vira `backend_unavailable` mencionando a bridge. O worker Node e a WebView falam o mesmo protocolo.

## CLI headless (sem janela do editor)

Headless = sem janela do editor, **com** runtime real: Node + Chrome do sistema + Three.js.

```sh
./scripts/runtime setup                 # pnpm install --frozen-lockfile && build do bundle
./scripts/fixtures /tmp/tessel-proof    # imagem, frames, WAV, splat, cena + proof.jsonl
{ cat /tmp/tessel-proof/proof.jsonl
  echo '{"op":"render.frame","frame":0,"out":"/tmp/tessel-proof/out/f00.png","probes":[[160,90]]}'
  echo '{"op":"seek","frame":30}'
  echo '{"op":"render.frame","out":"/tmp/tessel-proof/out/f30.png"}'
  echo '{"op":"render.audio","from":0,"frames":48,"out":"/tmp/tessel-proof/out/mix.wav"}'
} | ./scripts/render
```

Saída observada (resumida): `render.frame` → `{"ok":true,"result":{"action":"render.frame",
"frame":{"hash":"…","png":"…/f00.png","probes":[[149,165,82,255]],"render_ms":…},"plan":{…}}}`;
`render.audio` → `"kind":"processed","note":"offline mix; not played through an output device"`.

Variáveis: `TESSEL_CHROME`, `TESSEL_NODE`, `TESSEL_RUNTIME_DIR`, `TESSEL_GL`
(`swiftshader` padrão e determinístico, ou `gpu`; `--gl` na CLI), `TESSEL_REQUIRE_RUNTIME=1`
(testes passam a falhar em vez de pular). **Sem runtime** o frame responde
`backend_unavailable` com o comando de setup; planejar, editar e `render.audio` seguem
funcionando. Nada é simulado por screenshot ou pelo socket da janela. `tessel-headless`
(sem renderer) faz tudo exceto `render.frame`.

## UI e CLI da janela

Monitor único com uma `WebView` persistente (criada uma vez em `Workstation::new`, página
servida em `tessel://localhost/index.html` por protocolo próprio, sem CDN, navegação externa
negada). O tamanho de preview segue a razão da sequência com lado maior ≤ 960; o resto é
letterbox em CSS (redimensionar painel não re-renderiza).

Controles, todos `Command`/`Parsed` no core e nós em `ui tree`:

| Nó | Efeito |
|---|---|
| `monitor.preview.sequence` | `SetPreview { Sequence }` |
| `monitor.preview.source` | `SetPreview { Source }` da mídia selecionada com arquivo; desabilitado com motivo se não houver |
| `monitor.preview.quality` | alterna a política de preview `auto`/`original` (proxy no scrub); estado `selected`/`policy` |
| `monitor.reload` | recria a página (janela) e re-sincroniza o plano |
| `monitor.viewport` | leitura: `phase`, `headline`, `on_screen`, `capabilities`, `session` (`stats` com `coalesced`, `pending`, `presented`, `inspect_previews`, `trace`), `last_frame` (com `prepare_ms`, `draw_ms`, `flush_ms`, `cache`, `source`), `ui` (`repaints`/`skipped`), `session.preview_policy|refine_pending|proxy_error` |

```sh
./scripts/ui invoke monitor.preview.source
./scripts/ui get monitor.viewport
./scripts/ui op '{"op":"render.frame","frame":30,"out":"/tmp/f30.png","probes":[[60,90]]}'
```

`ui op` roda o vocabulário de projeto na janela viva; `render.frame` só responde depois que
a WebView desenhou (resposta diferida, timeout do cliente 45 s). Falha de frame aparece no
cabeçalho do monitor (`Falha: <código>`) e o próximo estado válido se recupera sozinho.

Ciclo de vida tratado e testado (`viewport_host_tests`, 34 testes): boot 15 s, handshake 15 s,
frame 30 s, seek em rajada **funde** posições (o quadro em curso termina; ver o agendador), edição/pedido
explícito cancelam e a resposta velha é descartada, perda e
restauração de contexto, bundle ausente (placeholder com a correção), runtime fechado,
reload, versão incompatível, asset com falha, troca de projeto libera assets (limite 16
retidos). No `gpui-pre` a view nativa **cobre overlays e não recorta**: ela é ocultada
explicitamente quando o monitor fecha ou não é a aba ativa (`update_viewport_visibility`).
O GPUI não pinta janela totalmente coberta; use `ui invoke window.activate` antes de observar.

### Ciclo de vida do preview sob incidente

O que o preview (proxy + refino) promete quando o ambiente falha, e como é provado. Contrato: **a última posição pedida
converge para a imagem original correta; o render/export original nunca muda; recursos da geração abandonada são liberados
segundo os contadores do runtime/Three.js (não é prova de liberação física no driver)**.

| Incidente | Resultado esperado | Prova |
|---|---|---|
| Perda de contexto no meio de um quadro de scrub com proxy / de refino / com o timer de refino pendente / com o playhead andando / com o monitor oculto / 6 vezes seguidas | `context_lost` tipado (`details.cause`), o proxy volta a ser aplicado na restauração (nunca é derrubado pela perda), o refino converge ao original byte a byte, `gl_textures` não cresce por ciclo | `runtime/test/lifecycle.test.mjs` (Chrome) e `scripts/viewport-lifecycle-proof.py` (janela real) |
| Contexto **nunca** restaurado | falha visível (`Falha: context_lost`), intenções novas respondem sem travar o host, `monitor.reload` recupera | harness |
| Perda e, antes da restauração, troca de projeto / reload | nenhum asset, proxy ou falha do projeto antigo aparece; o novo projeto desenha os próprios pixels | harness + teste do runtime (asset trocado durante a perda) |
| Ocultar/mostrar o monitor (8 alternâncias com o playhead andando), diálogo sobre o monitor (`flow.import` → `dialog.cancel`), janela minimizada/ativada | o monitor volta `on_screen`, sem falha, e a posição final converge ao hash de um `render.frame` original | harness |
| `monitor.reload` / `project.open` com quadro em voo e `render.frame` explícito esperando | o explícito responde (`cancelled` tipado ou um quadro de UM projeto, nunca pendurado) e a posição converge | harness (o reload é repetido até a corrida acontecer de fato) |
| Quadro cancelado em voo, `dispose` com quadros de proxy em voo | nada fica retido (`debug.glMemory()` zera), o próximo quadro é exato | `lifecycle.test.mjs` |
| Export original antes e depois de todos os incidentes | `framemd5` idêntico (12 quadros) | harness (export do próprio editor, worker Node+Chrome servindo o `dist` real) |

"Convergiu" significa, ao mesmo tempo: `session.status ready`, nada pendente nem refino, `presented.frame` == playhead,
`last_frame.revision` == `session.revision`; o hash da imagem na tela (`TESSEL_VIEWPORT_INSPECT=1`) == o de um
`render.frame` original do mesmo quadro e tamanho; o código de barras do quadro decodifica certo (+100 no segundo projeto);
e `last_frame.cache.context_generation` conta exatamente as perdas provocadas.

Limites: a perda é **forçada** com `WEBGL_lose_context` (eventos reais `webglcontextlost/restored`, via
`TesselRuntime.debug`, num `index.html` de rascunho que só instala o gatilho; o `runtime.js` é o mesmo), não é reset de GPU nem
queda de driver; não há timing de foto; o RSS dos processos auxiliares do WebKit não é medido (só o do editor); minimizar a
janela não muda `on_screen` (não é observável como ocultar). A contagem de texturas GL de quadros originais cresce com os
quadros distintos visitados até o teto de bytes (256 MiB) — é a retenção desenhada, não vazamento. Sem Rust alterado:
observações em [viewport-lifecycle-report](../viewport-lifecycle-report.md#observações-do-rust-sem-correção).

### `gpui-fast`

POC feita numa cópia temporária com `gpui-fast` ligado em `gpui-kit` **e** `gpui-webview`:
compila, boot/handshake ok, `live-proof.py` 57/57, resize de docks acompanhado pela view e
ocultar/mostrar o monitor. **Não validado**: overlay/popover/tooltip sobre a WebView e foco
(não há comando semântico que abra um overlay por cima do monitor, e clique por coordenada
está fora do escopo). Portanto o backend padrão **continua `gpui-pre`**.

## Evidências e status

| Superfície | Status | Como |
|---|---|---|
| Lib (plano, tempo, sessão, bridge, mixer, trace) | V | `./scripts/cargo test -p tessel-editor-core` (158 unit + 8 `proof` + 2 headless; 36 testes de sessão, 4 de `trace`) |
| Headless com runtime real | V | `crates/render/tests/frames.rs` (7) + `./scripts/render`; pixels conferidos por tipo |
| Runtime JS | V | `./scripts/runtime test` (61 testes, Chrome; inclui `inspect:false`, a retenção, o alvo por camada com duas cenas e 8 de ciclo de vida: perda com proxy, repetida, asset trocado, cancelamento, dispose) |
| Host da WebView sem GPUI | V | `viewport_host_tests` (34, incluindo scrub contra página lenta e os spans) + `viewport_tests` (7, janela de teste sem WebView) |
| Scrub na janela real | V (macOS) | `scripts/seek-scrub-proof.py` (+ `--pixel-proof`), números em [seek-performance-report](../seek-performance-report.md) |
| Janela real (WKWebView) | V (macOS) | `scripts/live-proof.py` 56/56 (57/57 com `--shots`, que acrescenta a checagem do PNG gravado), build debug, HOME isolado, com e sem `TESSEL_VIEWPORT_RETAIN=0` |
| Retenção: pixel igual e ganho de CPU | V (WebKit, Chrome GPU, SwiftShader) | harness `runtime/scripts/perf` (hash/vida/bench); tabelas e proveniência em [viewport-performance-results](../viewport-performance-results/); leitura em [viewport-performance-implementation-report](../viewport-performance-implementation-report.md) |
| Proxy automático + refino + export original | V (macOS, release) | `scripts/proxy-live-proof.py` (janela real, HOME isolado, só pedidos semânticos): proxy gerado pelo job, quadro do scrub vem do proxy, refino converge ao pixel do `render.frame` original, export idêntico ao da política `original`; testes de sessão/host/media/runtime |
| Ritmo do preview na janela, build release | V (macOS, uma tela) | [relatório OPT01–OPT10](../viewport-performance-next-report.md); soak de 480 s em `scripts/proxy-soak.py`; outros displays e foto na tela: P |
| Ciclo de vida sob incidente (perda de contexto com proxy, ocultar/mostrar, diálogo, reload/troca de projeto com quadro em voo, export intacto) | V (macOS, release, uma tela; perda **forçada**) | `scripts/viewport-lifecycle-proof.py` (147 checagens, janela real, HOME isolado, só pedidos semânticos) + `runtime/test/lifecycle.test.mjs`; [relatório](../viewport-lifecycle-report.md) |
| Áudio ouvido / A/V sync | P | não há saída de dispositivo |
| Overlays/foco sobre a WebView | P | ver `gpui-fast` (o diálogo que esconde a WebView é coberto pelo harness de ciclo de vida; popover/tooltip e foco continuam P) |

Limites conhecidos estão no [relatório](../viewport-foundation-report.md#lacunas).
