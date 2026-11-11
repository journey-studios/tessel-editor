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

## BL157.3 — pré-upload da textura PNG e warm draw (dois fatores opt-in, **desligados por padrão**, nunca combinados)

Duas hipóteses de custo de primeiro uso, cada uma atrás do seu próprio interruptor e do seu próprio anúncio de recurso, testadas **uma de cada vez**
no mesmo binário/runtime, com I/O `async` e warm do sort ligados nos dois braços e `gpu_finish=false`. Sem o interruptor, `hello` e respostas são
byte a byte os de antes (testes `core::bridge` e `viewport_host_tests`). Nada muda qualidade, resolução, MSAA ou contagem de splats.

| Fator | Interruptor / `hello` | Recurso anunciado | Relato em `render.status` | O que faz |
|---|---|---|---|---|
| A pré-upload | `TESSEL_VIEWPORT_PREUPLOAD=1` / `preupload_textures` | `texture.preupload` | `session.preupload {enabled, supported, sent}` | `frame.warm {scenes:false, textures:true}`: o host pede a subida de uma imagem **já carregada** de um corte futuro; o runtime a sobe com `renderer.initTexture`, um por passo, atrás de quadros em espera |
| B warm draw | `TESSEL_VIEWPORT_WARM_DRAW=1` / `warm_draw` | `scene.warm_draw` | `session.warm_draw {enabled, supported}` | depois do sort do aquecimento de uma linhagem, um draw numa `WebGLRenderTarget` de 16 px (sem MSAA, só profundidade), descartada sempre |

Regras (código em `runtime/src/preupload.js`, `assets.js` `preuploadStill`, `compositor.js` `uploadTexture`/`warmDraw`, `index.js`):

- **A:** fila de no máximo 8, idade máxima 3000 ms; resultados `uploaded`/`resident`/`budget`/`stale`/`missing`/`expired`; respeita o orçamento de
  texturas (nunca expulsa, nunca toca a recência: um quadro atual não perde lugar), cede a qualquer quadro em espera (`framesQueued`), é descartada
  em perda de contexto, retenção desligada, novo `hello` e `dispose`; recarregar o asset sobe de novo normalmente.
- **B:** uma vez por linhagem aquecida, idade máxima 3000 ms, passo na fila atrás de quadros em espera; resultados `drawn`/`lost`/`gone`/`taken`/
  `no_order`/`failed`/`dropped`/`expired`. Não desenha se o estágio foi substituído, falhou, foi assumido por um quadro (`warmOwned`), não tem a
  ordem do sort ou o contexto foi perdido; reserva os donos de malha e libera o alvo e o rig em `finally`. O canvas visível e os alvos retidos não
  são tocados.

**Provas (Chrome real, `./scripts/runtime test`, sem SKIP):** `preupload-browser.test.mjs` (11) e `warm-draw-browser.test.mjs` (10) mais 3 puros de
`preupload.test.mjs`: desligado por padrão, anúncio por `hello`, hash **exato** igual com o fator ligado/desligado (quente, frio, exato, retenção
desligada), alvo/texturas/geometrias iguais ao runtime sem o fator, alvo de rascunho liberado (`renderer.properties`), quadro em espera vai
primeiro, perda de contexto e "perda recém-ocorrida com o estágio ainda conhecido", cena liberada, orçamento, idade. Core: 4 testes de sessão e 6 de
bridge; host: 2 de GPUI.

**Mutação do warm draw** (`/tmp/pr41-preupload/mutations-warmdraw.log`, fora do Git; `dist/runtime.js` reconstruído a cada mutante e restaurado,
fontes e `dist` conferidos depois): sem draw, quadros não primeiro, alvo mantido, sem idade, draw no canvas: **mortos**. Sem a verificação de
takeover e sem as de contexto, **sozinhas sobrevivem** porque as defesas se sobrepõem (`warmOwned` e `gone`/`lost`): desligando as **duas** camadas
juntas os mutantes morrem (`M3b`; `M5b` depois de o teste "contexto recém-perdido, estágio ainda conhecido" ser adicionado). O limite: cada camada
isolada não tem teste próprio; a prova é a da combinação.

### Runner e regras pré-fixadas (`scripts/first-use-ab.py`, 19 testes puros em `first-use-ab.spec.py`)

`narrow` (janela 114–210) e `full` (projeto real completo; `RL.run_session`) com `--factor preupload|warm_draw`; `pixels` (janela real, sintético,
só `warm_draw`: o projeto sintético não tem imagens; os pixels do pré-upload são os testes de Chrome); `analyze` regenera o relato. Cada braço
declara os dois interruptores explicitamente (o outro fator em 0). Ordem alternada controle/ligado, ligado/controle…. Cada sessão faz a primeira
reprodução e uma repetição, **separadas** nas tabelas. Entradas: janela de 2 quadros na imagem PNG, 6 no corte, maior intervalo da janela;
**excesso pré-corte** = tempo a mais nos 2 s antes do corte (detecta deslocar a pausa). Par comparável = os dois válidos e quietos (mediana ≤ 3
núcleos externos, p95 ≤ 6, medianas a ≤ 2 núcleos). Benefício local = intervalo de entrada ≥ 10 ms menor em ≥ 3 de 4 primeiras reproduções sem
excesso pré-corte > 20 ms maior; "deslocado" = entrada menor com excesso > 20 ms maior; "fechado" = < 62,5 ms em toda primeira reprodução válida.
Um play com < 90 % dos quadros ou espera > 2 s é `HANG`: coluna própria, contado por braço, **nunca descartado**. 41,7/60/62,5 ms são só
referências diagnósticas. Nenhum veredito é aceite.

### Resultados no projeto real (binário `4e334135…`, runtime `f721cd65…`, documento `e93363ae…`; `fb0036f` com `b5ad3b8` de mesma árvore)

Dados e tabelas por execução: [first-use/](../viewport-real-latency-ab-results/first-use/) (`analysis.md` + `runs.sha256`; os JSON brutos, com o projeto
privado, ficam fora do Git). 4 pares × 2 sessões por fator; **todas as 16 sessões válidas**; carga: 14 de 16 sessões "carregada" (carga média
9–57 de outras tarefas); **0 de 4 pares comparáveis** em cada fator.

| Fator | Entrada PNG 196 (p50/p95/max, 1ª reprodução, n=28) controle → ligado | Entrada do corte 186 (n=12) | Intervalos > 62,5 ms por run | Excesso pré-corte por run (ms) | Veredito mecânico |
|---|---|---|---|---|---|
| A pré-upload | 58,4 / 72,4 / 110,9 → **44,2 / 51,6 / 55,5**; repetições 56,0/76,4/93,5 → 44,1/48,9/49,0 | 49,4/68,7 → 50,4/59,8 (sem efeito) | 3,9,4,1 → 3,1,1,3 | 320/439/358/389 → 374/338/308/393 | `SHIFTED` pela regra (1 de 4 pares com +53,6 ms); o agregado está dentro da dispersão entre runs |
| B warm draw | 57,9 / 71,6 / 76,2 → 64,1 / 113,6 / 129,2 | 50,0/73,6 → 47,2/142,7 | 1,3,11,5 → 6,4,12,9 | 349/357/537/374 → 500/436/416/710 | **sem benefício local**: entrada menor em 2 de 4, excesso pré-corte maior em 3 de 4 |

- **A (pré-upload):** benefício **local e real** na entrada da imagem (os 7 pontos do projeto), todos os 4 pares: o `texSubImage2D` de 13–27 ms sai da
  entrada e vai para o passo de pré-upload (14 `uploaded` por sessão, 244–310 ms no total, máx. 28–48 ms por passo). **Não resolve as pausas
  maiores:** as entradas de corte ficam iguais (p50 49,4 → 50,4) e as pausas de 63–79 ms em `prefetch:268` (quadros 220→221, 70–79 ms de trabalho
  da página) persistem nos dois braços. O aumento do excesso pré-corte de um par está dentro da dispersão dos braços; a leitura honesta é "sem
  deslocamento demonstrado", não "demonstrado sem deslocamento". Memória: mediana das medianas 865 (controle) contra 958 MiB (ligado), picos até 1569
  MiB: **sem afirmação**. O pré-upload fica opt-in, **não** é padrão.
- **B (warm draw):** a hipótese de custo de primeiro uso **do draw do corte** **não foi sustentada**: o braço ligado não reduziu a entrada do corte
  e teve mais intervalos > 62,5 ms (6/4/12/9 contra 1/3/11/5 nas primeiras reproduções) e mais excesso pré-corte. Com carga externa alta e desigual
  (mediana externa 4,2–8,6 núcleos; pico 57), isto não prova que o warm draw **piora**; prova que **não melhora** o suficiente para sair do ruído.
  Um run ligado (repetição, par 2, pico de carga 56,7) travou **11,1 s** entre os quadros 266→277 (`loop_stall` de 4,7 s, `clientWaitSync` de 349
  ms; 165 de 600 imagens): marcado `HANG`, mantido nas tabelas, **causa não determinada** (a página para sob carga também em `ab1`–`ab3` sem o fator,
  5/28 runs); não é atribuível ao fator nem o exonera. Rejeitado como candidato; não é padrão.
- **Pixels:** Chrome real prova hashes exatos iguais (quente/frio/exato/retenção desligada) para os dois fatores. Janela real, quadros sintéticos,
  `warm_draw`, binário final `4e334135…`, HOME/socket próprios, só pedidos semânticos: **PASS**, 4 quadros × (controle/ligado) × (quente/frio) com `render.frame` idêntico byte a byte, quadros de cortes distintos diferentes, leitura de todos e o monitor mostrando o mesmo quadro depois da pausa ([saída](../viewport-real-latency-ab-results/first-use/pixels-warm-draw.txt)); uma resposta `cancelled` (pedido substituído pelo refino do monitor) foi repetida, como no `real-latency-ab.py pixels`. Limite: o relato não grava o desfecho `drawn` do warm draw nessas sessões (o interruptor estava ligado; o draw em si é provado em Chrome). Refinamento e export usam o mesmo caminho do quadro exato (retenção desligada e `exact` nos testes); não há prova
  separada de export na janela.
- **Limites:** "imagem aceita" não é latência de display; os relógios JS e Rust não são subtraídos; o `start_ms` antigo inclui o ocioso antes do Play;
  carga externa não controlada (a coordenação não deu máquina quieta); só 4 pares por fator; GPU pendente consistente, não provada.
  **Latência NÃO ACEITA; BL157 segue aberto** (as pausas grandes, 100–180 ms no corte e `prefetch:268`, ficam).
