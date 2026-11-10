# F058 — Diagnóstico de I/O de assets do viewport (BL157.1)

## Objetivo e limites

Experimento **diagnóstico** para separar, na latência real do monitor, o custo de ler e responder os arquivos do
projeto pelo protocolo `tessel://` da WebView. Não é uma feature de edição e **não aceita a latência**: nenhum limite
diagnóstico (62,5 ms, 100 ms) nem o orçamento de 41,7 ms a 24 fps é meta aprovada. Resultado, hipóteses e limites:
[relatório](../viewport-real-latency-ab-report.md); plano: [real-latency-ab-test-plan.md](../migration/real-latency-ab-test-plan.md).

Três peças, todas **opt-in**, com o comportamento padrão inalterado:

| Peça | Como liga | Efeito |
|---|---|---|
| Modo do I/O de assets | `TESSEL_VIEWPORT_ASSET_IO=sync\|async` (padrão `sync`; valor inválido avisa e volta a `sync`; o relatório do host diz o modo efetivo) | `sync`: o callback do protocolo lê e responde (comportamento anterior). `async`: o callback só **resolve** (autoriza e decide o arquivo) e entrega a leitura a um pool limitado |
| Log de correlação | `TESSEL_VIEWPORT_TRACE=1` | Anel limitado de eventos (tick, prefetch, sync, protocolo, I/O, spans da página) com relógio monotônico do host |
| Leitura do log | `op render.trace {since, limit}` | Devolve a página de eventos a partir de um cursor, com `lost` quando o leitor ficou atrás da janela |

## Contrato do core / biblioteca

- `core::trace::TraceHandle::event(kind, || json)` grava no anel (`EVENT_WINDOW` = 16 384; mutex próprio; custo zero com o
  traço desligado) e `events_since(since, limit)` devolve `{enabled, now_ms, next, lost, window, events}`.
  Nomes de assets nunca entram: só um rótulo FNV-1a de 32 bits (`trace_label`).
- `ViewportSession::with_trace(TraceHandle)` compartilha o mesmo traço com o transporte.
- `Op::RenderTrace { since, limit }` (`render.trace`): `limit` 1–16 384 (padrão 4 096); inválido → `invalid_request`;
  traço desligado → `trace_disabled`.
- `render::host::resolve` (autorização e decisão do arquivo, sem I/O) e `render::host::read_file` (todo o I/O bloqueante);
  `serve` = `resolve` + `read_file`, com a mesma resposta de antes.
- `render::asset_io::AssetPool`: workers fixos (2) e fila `sync_channel` de 16 com `try_send`. Fila cheia **devolve o job** e o
  host responde `503` explicitamente (a execução fica inválida; nunca é descarte silencioso nem ganho). `invalidate()` (troca de
  projeto/sessão/recarga) faz jobs na fila responderem `410` sem I/O e descarta o corpo de leitura em voo. Cada fechamento roda
  exatamente uma vez (`catch_unwind`); os workers seguram só um `Weak` e o `Drop` não espera. O pool não depende de GPUI nem de Wry.

## CLI headless

`./scripts/headless` **não** tem janela, WebView nem protocolo: `render.trace` responde `desktop_required` sem tocar o estado
(teste `the_host_trace_belongs_to_the_window_and_the_headless_session_says_so_without_touching_state`). O worker Node do headless
lê assets por `runtime/worker/assets.mjs`, não pelo pool Rust. Gap declarado, N/A por construção.

## UI e CLI da janela

Sem controle novo de produto: nada na UI. Na janela viva, `./scripts/ui op '{"op":"render.trace","since":0,"limit":512}'`
(com `TESSEL_VIEWPORT_TRACE=1`) lê o log; `render.status` traz `asset_io` (modo, workers, fila, contadores do pool). O socket da
janela **não** é headless.

## Evidências e status

- Pool e `resolve`/`read_file`: seis testes em `crates/render/src/asset_io.rs` (mesmas respostas que `serve`, fila cheia
  devolve o job e responde uma vez, `invalidate` com e sem I/O em voo, no máximo um corpo por worker, pânico no fechamento sem
  matar o worker, `Drop` sem esperar).
- Desktop (`workstation/asset_io_tests.rs`): os dois modos respondem **o mesmo** status/cabeçalhos/bytes em 13 casos (arquivo
  inteiro, Range, sufixo, inválido, vazio, ausente, negado); controle negativo com leitura retida por FIFO: em `async` o pulso
  da janela segue responsivo antes de liberar a leitura, em `sync` o mesmo teste fica vermelho; fila cheia → `503` uma vez e contada;
  troca de projeto com leituras na fila e em voo → `410`, sem dupla resposta. Só `/asset` passa pelo pool: o pacote da própria
  página (`/`, `/index.html`, `/runtime.js`) é lido na thread chamadora também em `async`, para uma troca de projeto não recusar
  a página com `410`/`503` (teste `replacing_the_project_never_refuses_the_page_its_own_bundle`, verificado por mutação).
- Runtime: os spans da página (fetch/decode/warm) chegam por campos aditivos de `asset.state`/`frame.done`
  (`runtime/test/scene-warm.test.mjs`).
- Pixels: `scripts/real-latency-ab.py pixels` (projeto **sintético**; sync × async × quente/frio com `render.frame` idêntico byte
  a byte e o monitor mostrando a mesma imagem). O export não passa pelo protocolo (N/A).
- A/B no projeto real: [relatório](../viewport-real-latency-ab-report.md). Limites: o modo `sync` inclui no callback o custo da cópia
  `NSData` do Wry; em `async` ela corre no worker.

## BL157.2 — probes do trânsito do quadro (opt-in, o mesmo custo nos dois braços)

Os A/B de BL157.1 deixaram pausas de 56–120 ms em que o runtime gastou 1–24 ms e nenhum span explicava o resto. Os probes abaixo
separam, **cada um num único relógio**, de onde vem esse tempo. Ligam com `TESSEL_VIEWPORT_TRACE=1` (o mesmo gatilho do restante) e não
pedem quadro nenhum para medir; sem o traço o `hello` e as respostas são byte a byte os de antes.

| Probe | Onde | O que mede |
|---|---|---|
| `telemetry.timing` no `hello` | `core::bridge::HelloOptions::timing` (aditivo; sozinho ou com `presented`) | liga os probes da página |
| `frame.done.page` | `runtime/src/index.js` | `queue_ms` (espera na fila serial da página **antes** de o quadro começar; `total_ms` não a conta), `held_ms` (do recebimento até a entrega ao transporte), `ahead` (quadros/aquecimentos/cargas à frente), `post_prev_ms` (custo de entregar a resposta anterior), `loop` (atraso do laço de eventos: batimento de 4 ms, máximo e contagem ≥ 16 ms desde a resposta anterior) |
| span `loop_stall` e `ago_ms` | idem | atraso do laço ≥ 16 ms vira um span; com o timing, **todo** span da página (incluindo `warm`) diz há quantos ms terminou, em relação à entrega (a mensagem que o leva pode sair muito depois: antes, o span era posicionado pela chegada da mensagem seguinte) |
| evento `eval_done` | `workstation/viewport.rs` | do envio do script ao *completion handler* do WebView (no host): entrega até a página + tratamento + volta do callback |
| span/campo `ipc_wait` | idem + `frame_page` | espera entre o WebView entregar a mensagem (handler de IPC) e a tarefa da janela pegá-la |
| evento `frame_page` | `viewport_host::trace_incoming` | a conta da página de cada `frame.done`, com `ipc_wait_ms` (e `gpu_finish_ms` no braço diagnóstico) |
| span `gl_call` | `runtime/src/index.js` (protótipo do contexto WebGL, restaurado ao desligar) | chamada WebGL de 4 ms ou mais, com o **nome** da chamada (leitura, fence, upload, link, draw que força flush): diz se um atraso do laço estava dentro de uma chamada WebGL |
| evento `prefetch_asset` | `core::session::prefetch` (só com o traço) | por carga iniciada: `fs_ms` (verificações de arquivo), `send_ms` (entrega ao backend), `bytes`, `kind`; nunca o nome do asset |
| `telemetry.gpu_finish` (env `TESSEL_VIEWPORT_PROBE_GPU_FINISH=1`, só com o traço) | `HelloOptions::gpu_finish`, `index.js` | **braço diagnóstico, nunca padrão**: após cada draw a página lê 1 pixel (`finish()` sozinho voltou em 0 ms no WebKit, não esperou) e reporta `timings.gpu_finish_ms`; troca a espera do navegador na apresentação por uma espera dentro do quadro |

`scripts/viewport-transit-probe.py` (`loop`/`ab`/`ab-report`/`analyze`; specs em `.spec.py`) decompõe cada imagem tardia em `host_pre_send`, `page_queue`,
`page_draw`, `ipc_wait` e `delivery` (o que sobra; ida e volta juntas) e nomeia o dominante. O `loop` é **capaz de falhar**: sai com 1 se
alguma iteração tem gap > 62,5 ms (limiar diagnóstico, não aceite), um bloqueio de UI (passo de prefetch ≥ 50 ms) ou um play faminto (quase
nenhuma imagem). Um quadro do mesmo `shown_frame` que o anterior é a imagem de assentamento depois do stop e não conta como gap de
reprodução. Nenhum relógio de página, host e Python é subtraído. `ab` alterna A (traço + timing) e B (A + `gpu_finish`) em editores novos,
um por run, com o modo de I/O fixo; um run em que a página para de responder fica **inválido e listado**; o veredito é mecânico e só diz
`CONSISTENT`/`INCONCLUSIVE` (a espera do braço B também é um stall de página, então as contagens de A e B não se comparam).

### O que os probes mostraram (projeto real, janela 114–210, I/O `async`; **não é aceite de latência**)

Dados e tabelas por run em [viewport-real-latency-ab-results/bl157-2/](../viewport-real-latency-ab-results/bl157-2/). Lotes separados (cargas externas
diferentes; não se somam): `loop1`–`loop4` e `ab1`–`ab4`; o final é `loop4`/`ab4` (binário `ac023eba…`, runtime `1fed1ef9…`, código `a01090b`).

- Descartado como causa dos gaps de 56–185 ms: fila do `evaluate` do host (`pre-send` ≤ 0,012 ms), espera da tarefa da janela pela
  mensagem (`ipc_wait` p95 ≤ 0,4 ms; máx. 5,7 ms), fila serial da página (`queue_ms` 0 nos gaps), custo de desenho do runtime nos gaps
  do corte (1–6 ms).
- Onde está o tempo nos gaps do corte (`187→188`/`188→189`, e `115→116`/`116→117` no início do play): a **thread principal da página
  parada** 60–160 ms, começando logo após um draw terminar e terminando antes de o próximo script entrar. O `eval_done` do host ≈ o trânsito inteiro;
  o `loop_stall` da página (120 ms) cobre o intervalo. Nenhuma chamada WebGL da lista passou de 4 ms nesse intervalo, e não havia leitura
  de asset em voo: o atraso está fora do JS do runtime e das chamadas WebGL medidas.
- Os gaps `195→196` de 63–83 ms são `page_draw` de 24–31 ms (um `texSubImage2D` de 13–27 ms na primeira imagem de cada sequência PNG,
  `gl_call`): aqui o dado **aponta** para pré-upload de textura, ainda não testado.
- Braço `gpu_finish` (B, 8 pares em `ab4`, todos válidos): a leitura esperou ≥ 40 ms em 8 de 8 runs (máx. por run 41–119 ms), nos quadros dos primeiros draws
  e nos de 185–187 (o corte); onde A tem o gap `187→188`, B tem `185→186`/`186→187` de ~65–70 ms (um quadro de 41,7 + a espera). Resultado
  mecânico: `CONSISTENT with GPU work still pending after a draw` (ver [ab4](../viewport-real-latency-ab-results/bl157-2/ab4.md)). O atraso **se moveu para
  dentro do quadro, não sumiu**: o dado é compatível com trabalho de GPU pendente depois do draw do corte (dois cenas splat), **não prova** que
  seja todo o stall e não é latência de display/fóton. Em `ab4` A reproduziu um stall ≥ 40 ms em 6 de 8 runs; em `ab3`, em 3 de 6.
- Falhas que não são latência e ficam registradas: em `loop1` um run teve dois passos de prefetch de ~3 s na thread da janela (sem
  causa determinada: os probes por carga só entraram depois e o evento não se repetiu nos 60 runs seguintes); em `ab1`–`ab3`, 5 de 28 runs
  (2 do braço A, 3 do B) o frame pipeline da página **parou de responder** (100 % dos quadros cancelados; a página ainda atendia
  assets). Não explicadas; os runs são inválidos e estão nas tabelas. `ab1`: `finish()` sozinho não esperou (manipulação falhou, veredito `INCONCLUSIVE`).
- Mudança causal de produto: **nenhuma**. A alavanca implicada (custo de GPU do draw do corte) mexe em resolução/MSAA/contagem de splats, que esta
  rodada não variou. O efeito de um draw de aquecimento **não foi medido** (nenhum teste separa custo de preenchimento de custo de primeiro uso).

Headless: N/A (o worker Node não passa por WebView nem pelo traço do host).
