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
  troca de projeto com leituras na fila e em voo → `410`, sem dupla resposta.
- Runtime: os spans da página (fetch/decode/warm) chegam por campos aditivos de `asset.state`/`frame.done`
  (`runtime/test/scene-warm.test.mjs`).
- Pixels: `scripts/real-latency-ab.py pixels` (projeto **sintético**; sync × async × quente/frio com `render.frame` idêntico byte
  a byte e o monitor mostrando a mesma imagem). O export não passa pelo protocolo (N/A).
- A/B no projeto real: [relatório](../viewport-real-latency-ab-report.md). Limites: o modo `sync` inclui no callback o custo da cópia
  `NSData` do Wry; em `async` ela corre no worker.
