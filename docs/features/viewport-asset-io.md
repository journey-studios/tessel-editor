# F060 — Diagnóstico de I/O de assets do viewport (BL157.1)

ID atual **F060** após integração com `main`: F058/F059 pertencem às chaves de FX e ao grão. Referências antigas a F058 nesta PR eram do diagnóstico.

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
  `gl_call`): naquele snapshot o dado apontava para pré-upload ainda não testado; o experimento posterior é BL157.3, sem aceite da cauda.
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

**Provas (Chrome real, `./scripts/runtime test`, sem SKIP):** `preupload-browser.test.mjs` (11) e `warm-draw-browser.test.mjs` (10) mais 4 puros de
`preupload.test.mjs`: desligado por padrão, anúncio por `hello`, hash **exato** igual com o fator ligado/desligado (quente, frio, exato, retenção
desligada), alvo/texturas/geometrias iguais ao runtime sem o fator, alvo de rascunho liberado (`renderer.properties`), quadro em espera vai
primeiro, perda de contexto e "perda recém-ocorrida com o estágio ainda conhecido", cena liberada, orçamento, idade. Core: 4 testes de sessão e 6 de
bridge; host: 2 de GPUI.

**Mutação do warm draw** (`/tmp/pr41-preupload/mutations-warmdraw.log`, fora do Git; `dist/runtime.js` reconstruído a cada mutante e restaurado,
fontes e `dist` conferidos depois): sem draw, quadros não primeiro, alvo mantido, sem idade, draw no canvas: **mortos**. Sem a verificação de
takeover e sem as de contexto, **sozinhas sobrevivem** porque as defesas se sobrepõem (`warmOwned` e `gone`/`lost`): desligando as **duas** camadas
juntas os mutantes morrem (`M3b`; `M5b` depois de o teste "contexto recém-perdido, estágio ainda conhecido" ser adicionado). O limite: cada camada
isolada não tem teste próprio; a prova é a da combinação.

### Runner e regras pré-fixadas (`scripts/first-use-ab.py`, 29 testes puros em `first-use-ab.spec.py`)

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
  `warm_draw`, binário final `4e334135…`, HOME/socket próprios, só pedidos semânticos: **PASS**, 4 quadros × (controle/ligado) × (quente/frio) com `render.frame` idêntico byte a byte, quadros de cortes distintos diferentes, leitura de todos e o monitor mostrando o mesmo quadro depois da pausa ([saída](../viewport-real-latency-ab-results/first-use/pixels-warm-draw.txt)); uma resposta `cancelled` (pedido substituído pelo refino do monitor) foi repetida, como no `real-latency-ab.py pixels`. Limite: esta execução não gravava o desfecho `drawn` do warm draw nessas sessões (o interruptor estava ligado; o draw em si é provado em Chrome); a prova passou a gravá-lo e a reprovar sem ele (ver "BL157.3 — o que os testes unitários provam", abaixo). Refinamento e export usam o mesmo caminho do quadro exato (retenção desligada e `exact` nos testes); não há prova
  separada de export na janela.
- **Limites:** "imagem aceita" não é latência de display; os relógios JS e Rust não são subtraídos; o `start_ms` antigo inclui o ocioso antes do Play;
  carga externa não controlada (a coordenação não deu máquina quieta); só 4 pares por fator; GPU pendente consistente, não provada.
  **Latência NÃO ACEITA; BL157 segue aberto** (as pausas grandes, 100–180 ms no corte e `prefetch:268`, ficam).

## Residuais da coordenação após `5adf5b9`

- Os tempos de fetch/decode pertencem ao resultado de cada carga admitida (identidade do objeto de metadados, em `WeakMap`),
  não ao ID reutilizável do asset. Cargas antigas recusadas/canceladas não publicam tempos nem apagam os da substituta.
  O campo bridge `asset.state.timings` e os metadados não mudam. Teste puro do runtime exercita a corrida e leitura fora de ordem.
- `redact()` remove também nomes finais com espaços e arquivos diretamente sob a home/temp. Relatórios publicados nunca devem conter esses nomes.
- `ab-report` só reconstrói os runs do lote salvo em `ab.json` quando **todos** os brutos esperados existem. Arquivo de outro lote é ignorado;
  arquivo ausente preserva todos os resumos, recalcula apenas o veredito e registra a limitação.
- **Lacuna de correlação:** para `frame_sequence`, `asset_load_sent.path` é o hash do padrão e `asset_cb`/`asset_io` usam o caminho do quadro concreto;
  esses rótulos não são uma chave de junção. As sete entradas PNG medidas em BL157.3 são imagens paradas (`still`), não sequências.
  Nenhum resultado sobre sequências é afirmado. Ligar cada fetch expandido à carga de sequência continua pendente em BL157.3.
- O A/B de `5adf5b9` tem **zero pares comparáveis** por fator. Os números locais são observações sob carga, não estimativa causal confiável
  da melhora total. O warm draw não demonstrou benefício; a amostra não refuta seu efeito em condições controladas.
- A prova nativa `pixels-warm-draw.txt` (histórica) não registrou `drawn`: comprovava compatibilidade da execução, não que a manipulação aconteceu
  na janela. As sessões ligadas do **A/B** nativo registraram, sim, o desfecho que funcionou em todos os runs (tabela por run de cada `analysis.md`:
  pré-upload `uploaded` 14 por run em 8/8 runs completos, warm draw `drawn` 4–6 por run em 8/8; nenhum span do fator em nenhum dos 8 runs de controle
  de cada fator), então a ativação na janela **está evidenciada para o A/B**. A prova **de pixels** passou a registrar os spans e a reprovar se a sessão
  ligada não relata `drawn` ou se o controle relata algo (`activation_verdicts`, com spec). Falta **repetir** a prova nativa com esse veredito (não executada nesta entrega: 5 GiB livres, abaixo do piso duro de 12 GiB do runner; o fio `render.trace` → spans foi conferido no código e testado com um `Lab` falso, não com a janela).
  O pré-upload não é exercitado pelo fixture nativo sem imagens: Chrome real cobre seus pixels.
- Integração de `main` e correções desta seção têm somente testes puros atuais. Gates Rust/Chrome/janela/release e A/B **não** são herdados de `5adf5b9`.
  O disco observado de 14 GiB está abaixo da reserva de 16 GiB. O caminho do experimento não vira padrão e a latência permanece **não aceita**.

## BL157.3 — o que os testes unitários provam (e o que não podem provar)

Esta seção nasceu de uma pergunta: "dá para implementar isso com unit testing?". Uma parte grande dá. Os quatro blocos de validação que
estavam pendentes (gates de Rust, Chrome real, janela nativa, A/B no projeto real) misturavam **decisões**, que são determinísticas e
testáveis sem GPU, com **medidas**, que só existem na janela real. As decisões agora têm testes; as medidas continuam abertas (última
subseção). Nada aqui muda o produto: os dois fatores seguem opt-in e desligados, a latência segue **NÃO ACEITA** e BL157 segue aberto.

### Camadas e o que cada uma garante

| Camada | Arquivo | Testes | Garante |
|---|---|---|---|
| Store (Node puro, sem WebGL) | `runtime/test/preupload-store.test.mjs` | 8 | `AssetStore.preuploadStill`: `uploaded` (a textura é feita uma vez, antes do quadro, e o quadro a encontra), `resident` (nunca refeita), `budget` (nunca expulsa para caber), `stale`/`missing` (o pedido de uma carga antiga nunca dá textura à substituta, a um asset solto, nem a splat/vídeo), `failed` (sem textura nem bytes, libera o que criou, pode ser pedido de novo), recência intacta (`lastUsed`: uma textura que ninguém desenhou é a primeira que o orçamento devolve), perda de contexto (descarta a textura, mantém a imagem) |
| Fila e tempos (Node puro) | `preupload.test.mjs` (4), `load-timings.test.mjs` (5) | 9 | fila limitada, idade, `upload_ms` só soma o upload que aconteceu; tempos presos ao resultado da carga admitida |
| Página (`index.js`) com compositor falso | `orchestrator-preupload.test.mjs` | 15 | desligado por padrão (nada anunciado nem subido); ligado: sobe antes do quadro e relata; nada em dobro; **quadro primeiro** (já em espera e chegando no meio da fila: espera o passo em voo, não a fila); recarga ou liberação antes do passo; falha do upload; retenção desligada; descarte em perda de contexto, `hello` sem o fator e `dispose`; perda ainda não entregue como evento; fila limitada; orçamento; idade |
| | `orchestrator-warm-draw.test.mjs` | 13 | desligado por padrão; o draw segue o **seu** sort e é um só; quadro em espera vai antes; contexto perdido durante o sort; descarte por `hello` sem o fator e `dispose`; idade; cada desfecho do compositor (`gone`, `taken`, `no_order`, `lost`, `failed`) relatado como ele é e **não** contado como draw; falha engolida sem travar a fila |
| | `orchestrator-loads.test.mjs` | 4 | a carga pronta devolve o pedido que responde e os tempos da **própria** carga; carga ultrapassada por recarga é cancelada com o **seu** pedido, nas duas ordens de chegada; carga liberada em voo |
| Contrato de ativação | `orchestrator-contract.test.mjs` + `runtime/test/fixtures/activation-spans.json` | 19 | para cada span do contrato (`preupload`: `uploaded`; `warm_draw`: `drawn`), cada desfecho é emitido pelo runtime como `{k, outcome, ms}` em `frame.done.runtime_spans`; varredura do código real: o que `warmDraw`, `runWarmDraw`, `preuploadStill` e `step` podem devolver é **exatamente** o vocabulário do contrato |
| Host (Rust) | `crates/desktop/src/workstation/asset_io_tests.rs` | 1 | todo desfecho do contrato chega ao `render.trace` do host como `runtime_span` com `k`/`outcome`/`ms` intactos |
| Harness (Python) | `first-use-ab.spec.py` (29), `scene-warm-proof.spec.py` (27) | 13 novos | só o braço que relata o desfecho que funcionou é "ativo"; controle contaminado, interruptor diferente do pedido ou ausência de relato **reprovam**; coleta de spans da prova de pixels |
| Chrome real (já existia) | `preupload-browser.test.mjs` (11), `warm-draw-browser.test.mjs` (10) | 21 | hashes exatos iguais, alvos/texturas/geometrias, liberação do alvo, perda de contexto real |

### Como a página é testada sem WebGL

`createRuntime` ganhou um parâmetro `createCompositor` (padrão: o `Compositor` real; duas linhas, sem mudança de comportamento). Os testes
injetam um `FakeCompositor` (`runtime/test/orchestrator-fixtures.mjs`) que usa o **`AssetStore` real** (contadores, fixação e séries
reais) e modela só o que a GPU decidiria: se um upload acontece, falha ou encontra o contexto perdido, e o desfecho do warm draw. A
ordem e as corridas são arranjadas sem `sleep`: `debug.holdQueue()` segura a fila da página, o gancho `onUpload` faz um quadro chegar no meio
da fila, um relógio falso cobre as idades de 3000 ms (2000 ms ainda corre, 3500 ms expira) e `debug.idle()` assenta a página.

### Cadeia de custódia da ativação

Ligar o interruptor não prova que a manipulação aconteceu. Quem diz o que **aconteceu** é a página, em spans `{k, outcome, ms}` dentro de
`frame.done.runtime_spans`; `render.status` só diz o que o host **enviou**. O contrato `activation-spans.json` (um arquivo, como
`fx-catalog.json`) liga as três pontas:

1. **Página:** `orchestrator-contract.test.mjs` provoca cada desfecho do contrato pelo `index.js` real e exige o span; a varredura do código
   real impede que um desfecho novo ou renomeado fique fora do contrato (ou o contrato fique com um desfecho que o código não emite).
2. **Host:** `every_outcome_the_page_can_report_for_the_two_experiments_reaches_the_trace_intact` envia todos os spans do contrato num
   `frame.done` ao `ViewportHost` real e confere um `runtime_span` por span no `render.trace`.
3. **Harness:** `first-use-ab.py` lê o mesmo contrato (`FACTORS[...]["worked"]`, spec `ActivationContractTests`): só `uploaded`/`drawn` é
   evidência de que o fator rodou; qualquer desfecho do contrato num braço de controle o contamina. A prova de pixels
   (`first-use-ab.py pixels`) agora também reprova (`activation_verdicts`) se a sessão ligada que tocou até o corte não relata o desfecho
   que funcionou, se alguma sessão de controle relata um span do fator, ou se o `enabled` do host difere do braço.

### Mutação (os testes novos não são vácuos)

Cada regra foi quebrada, uma por vez, numa **cópia** de `runtime/` (a árvore e o `dist` não são tocados) e o conjunto novo precisa reprovar:
37 mutantes, **37 mortos, 0 sobreviventes** no código final:

| Onde | Mutantes | Regras quebradas (cada uma reprova pelo menos um teste novo) |
|---|---|---|
| Página (`index.js`) | 24 | passo que não cede a quadro em espera (pré-upload e warm draw); contexto não consultado; fila com retenção desligada, durante perda, sem esquecer em perda/`hello`/`dispose`; fator ligado "a menos que recusado"; recurso anunciado a todo host; nome e campos dos spans; draw que ignora perda de contexto, `hello` que desliga e idade; todo desfecho contado como draw; quadro em espera não contado; tempos lidos pelo ID reutilizável (a leitura anterior a `b53775e`); pedido da carga não devolvido; pré-upload que nunca avisa o host |
| Store (`assets.js`) | 7 | entrada pedida não conferida; série repetida; orçamento ignorado; recência tocada; textura recusada não liberada; segunda textura sobre a residente; entrada que não é imagem parada aceita |
| Fila (`preupload.js`) | 5 | limite +1; idade ignorada; pedido repetido; nome do desfecho `failed`; `upload_ms` somando o que não subiu |
| Compositor (`compositor.js`) | 1 | desfecho do warm draw renomeado (a varredura do código real o pega) |

A primeira rodada (35 de 37 mortos) deixou dois sobreviventes que viraram testes: splat/vídeo só eram barrados por `kind` (agora a entrada
fabricada tem bitmap, então só `kind` a impede) e `upload_ms` contava uploads que não aconteceram; a limpeza síncrona da fila (equivalente no
resultado final) passou a ser conferida no instante do evento. No host Rust, renomear o evento `runtime_span` ou
descartar `outcome` do span encaminhado (dois mutantes) reprova o teste do host. No harness Python, 13 mutantes (veredito de ativação e coleta de
spans) reprovam os specs.

### O que um teste unitário não prova

- **Latência.** Nenhum teste unitário tem a GPU, o disco, a carga ou o relógio da janela real. Se a antecipação do `texSubImage2D` ou o
  draw de aquecimento reduzem a pausa no projeto real é pergunta do A/B, que foi **inconclusivo** (zero pares comparáveis por fator).
- **O compositor real.** O compositor falso devolve o desfecho que o teste manda. Que o `Compositor` real devolve `drawn`/`lost`/`gone` num
  contexto WebGL de verdade é o que os testes de Chrome (SwiftShader) provam; Chrome não é o WKWebView/Metal da janela. A varredura
  do código real só garante que o vocabulário não diverge.
- **Pixels no WKWebView.** Só a prova de pixels na janela real vê o monitor nativo. A prova de pixels da janela real (`first-use-ab.py pixels --factor warm_draw`) agora registra os spans e reprova sem `drawn` no braço ligado, mas **não foi repetida** nesta entrega (o runner exige 12 GiB livres e havia 5 GiB): o último resultado nativo continua sendo o histórico `pixels-warm-draw.txt`, sem registro de ativação. O pré-upload é N/A nessa prova (o projeto
  sintético não tem imagens paradas); seus pixels são os testes de Chrome.
- **Ainda sem causa e sem teste:** a parada de 60–160 ms da página depois do draw do corte, o `HANG` de 11,1 s sob carga (que **recuperou**: não é
  atribuído ao deadlock do decodificador de [BL157.4](#bl1574--o-deadlock-do-webkit-no-decodificador-o-laboratório-e-a-prova-nativa), cujos runs presos nunca se recuperam),
  a correlação dos fetches expandidos de `frame_sequence` e um A/B com pares comparáveis. A **página que para de vez** tem causa e correção em BL157.4.

### Reprodução

```sh
cd runtime && node --test test/preupload-store.test.mjs test/preupload.test.mjs test/load-timings.test.mjs test/orchestrator-*.test.mjs  # puros, segundos
cd runtime && TESSEL_REQUIRE_RUNTIME=1 pnpm test                                                                                       # tudo, com Chrome real
python3 scripts/first-use-ab.spec.py && python3 scripts/scene-warm-proof.spec.py
./scripts/cargo test -p tessel-editor --locked asset_io
```

## BL157.4 — o deadlock do WebKit no decodificador, o laboratório e a prova nativa

Rodada de otimização do BL157 (PR #41, após `4c16656`): atribuir antes de mudar, um fator por vez, sem reduzir qualidade. **A única mudança de produto é no build do
`runtime.js`.** Nenhum controle, `Command`, nó de árvore, op ou variável de ambiente novos, e nenhum interruptor: a remoção está **ligada por padrão** porque só tira uma chamada que pode travar, mas **o efeito dela na taxa de páginas
presas não está demonstrado** (laboratório sugestivo, réplica e janela real NÃO REPRODUZIDO: abaixo); manter ou reverter até haver prova com poder é uma decisão em aberto.
`warm_draw` e pré-upload seguem **opt-in e desligados**; a latência segue **NÃO ACEITA** e BL157 segue aberto. Números e leitura:
[relatório §16](../viewport-real-latency-ab-report.md#16-bl1574-deadlock-do-webkit-no-decodificador-laboratório-de-webkit-real-e-prova-nativa-pr-41-após-4c16656).

| Superfície (contrato de features) | Estado |
|---|---|
| Biblioteca / core | N/A: o patch está no bundle de página (JS); nenhuma regra de domínio muda; `cargo tree -p tessel-editor-core` não muda |
| CLI headless | O worker serve o mesmo `dist/runtime.js` ao Chrome headless; o defeito é do WebKit, então o efeito em Chrome é N/A (a suíte de Chrome real passa com o bundle corrigido) |
| UI (janela) | O `WKWebView` serve o mesmo `runtime.js` (basta reconstruí-lo, sem recompilar o Rust): prova pelo A/B nativo, abaixo |

### O defeito e a correção

O worker de decodificação do Spark (`jsContent` inline em `spark.module.js`) chama `reader.releaseLock();` depois do último pedaço. No WebKit, `releaseLock()` roda um builtin que aloca (um `TypeError`);
se a alocação dispara uma coleta nessa thread, a marcação visita o mesmo leitor e espera em `lockSlow` um lock que a thread parece já segurar. O decode nunca responde (o asset fica `loading`) e uma
parada geral pendente (`JITWorklist::suspendAllThreads`) pode congelar a página. Evidência: pilhas `sample` de 3 páginas paradas, **a mesma cadeia** (2 do laboratório e 1 da janela real, que teve outro gatilho da
coleta e a thread principal ociosa); só decodes pequenos foram afetados (arquivos de 2,9 MB e 79 KB, nunca uma nuvem grande), em 0% a 6,7% dos runs do laboratório conforme o lote (o lote de 6,7% rodou sem controle e sob carga muito maior, `load1` mediana 35; a diferença entre os lotes intercalados e a réplica
**não** se explica pela carga medida: 5,4–10,1 contra 9,7). O mecanismo é a leitura da pilha, não foi confirmado alterando o WebKit, e **o efeito da correção na taxa não está demonstrado** (abaixo).

`runtime/scripts/build.mjs` troca, no build (plugin `onLoad` do esbuild), a **única** `reader.releaseLock();` do worker por `void 0;` (o bundle encolhe 14 bytes) e **recusa o build** se o Spark não tiver
exatamente uma (`SPARK_RELEASE_LOCK`, `withoutReleaseLock`). O `releaseLock` da thread principal (`readStream`) só roda com `stream`, que o runtime nunca passa (ele passa `fileBytes`): fica. O script só
constrói quando é o ponto de entrada (`realpathSync(process.argv[1])`), então os testes importam as funções sem construir nada.

### O que prova o quê

| Camada | Onde | Garante |
|---|---|---|
| Node puro | `runtime/test/spark-release-lock.test.mjs` (5) | só a chamada do worker some e a de `readStream` fica; Spark com 0 ou 2 chamadas faz o build falhar; o Spark instalado tem exatamente 1; o `dist` construído não a tem (exige `TESSEL_REQUIRE_RUNTIME=1`; sem ela, pulado) |
| Mutação (script fora do repositório) | — | 6 de 6 mutantes mortos: patch que não faz nada; patch que também solta o leitor da thread principal; sem checagem de contagem; "pelo menos uma"; patch que apaga a linha seguinte; `dist` sem a correção |
| Suíte do runtime com Chrome real | `TESSEL_REQUIRE_RUNTIME=1 pnpm test` | 336 de 336 com o bundle corrigido |
| Laboratório (WKWebView real, sem editor) | `runtime/scripts/perf`, caso `real-load` | Intercalado: **4 de 193 runs presos sem a correção, 0 de 193 com ela** (p exato unilateral 0,0615). **Réplica pré-registrada (300 por braço): 0 e 0 → NÃO REPRODUZIDO.** Total intercalado: **4 de 493 contra 0 de 493** (p 0,062: sugestivo, não conclusivo). Mais 8 de 120 em lote sem pareamento e sob carga ≈ 35, fora da conta |
| Janela real, mesma build | `scripts/runtime-bundle-ab.py` | 36 pares: **1 sessão presa sem a correção, 0 com ela → NÃO REPRODUZIDO** (p 0,5). A sessão presa tem a cadeia do deadlock em 2 amostras: o defeito existe no editor real, mas **a rodada não tem poder para julgar a correção** |

### O laboratório de WebKit real (`runtime/scripts/perf`)

Ferramenta de diagnóstico: um `WKWebView` mínimo (`wk-host.swift`, via `webkit.mjs`) carrega o `runtime.js` real sem o editor. Os resultados dependem da máquina e da carga e **não são o editor**:
servem para atribuir hipóteses, não para aceitar latência. Cada run vai para o JSONL com a carga (`loadavg`); runs com erro ou presos **nunca são descartados**. Novidades desta rodada:

| Opção / caso / variante | Para quê |
|---|---|
| `--fresh` | uma página nova por run (o primeiro crossfade de um processo é o que se mede) |
| `--real-cloud-a/-b/-c/-d`, `--real-ring`, `--real-sparks` | as nuvens do projeto real como **caminhos** locais: viram links numa pasta temporária, nunca cópias, e **não são versionadas** |
| `--size`, `--quality preview`, `--pacing film`, `--drain read1x1` | quadro 1280×720, qualidade do monitor com o playhead andando, 24 fps, "quadro completo" medido com a GPU |
| `--glcalls MS`, `--psokeys` | chamadas WebGL que levam ≥ MS com o quadro em voo; chaves de configuração de draw na ordem do primeiro uso (cada uma sozinha) |
| `--workerlog` | todo `Worker` e todo `fetch` da página: o que uma carga que não termina esperava |
| `--load-timeout`, `--run-timeout` | limite da carga de assets; limite de fora (a página com a thread principal congelada não consegue se dar timeout) |
| `--prime-wait` | espera ociosa da página depois do aviso enviado na abertura |
| casos `real-load`, `real-load-many` | carregam e decodificam os assets do projeto real; foi aqui que o deadlock apareceu |
| variantes `current`, `nowarm`, `warmdraw`, `prev` | o runtime como está; sem o aviso `frame.warm`; com `scene.warm_draw`; o bundle sem a correção (`--baseline-bundle`) |
| variantes `primeopen`, `primeonly`, `primescene` | aquecer as cenas na abertura (com draw e com o aviso normal depois; só na abertura; sem draw) |
| suítes `real`, `late`, `chain` | cenas com as nuvens reais (uma nuvem, entrada de crossfade, corte, fade de imagem, prefetch); a entrada do crossfade mais tarde no run; as quatro cenas em sequência (crossfades e cortes) |
| suítes `xfade`, `entry` | nuvens **sintéticas** de 1 M de splats: uma nuvem ao ritmo de filme contra duas em crossfade; os quadros em volta da entrada |

O monitor de atrasos de timer do driver (`BUSY_LATE_MS`) passou de 12 para 30 ms: a 12 ms a lista continha quase todos os quadros de um run a 24 fps (13–27 ms de atraso, o trabalho do próprio quadro).
Resultados do laboratório (onde o primeiro crossfade paga, aquecer na abertura não resolve): [§16.2 e §16.3](../viewport-real-latency-ab-report.md#16-bl1574-deadlock-do-webkit-no-decodificador-laboratório-de-webkit-real-e-prova-nativa-pr-41-após-4c16656).

### Prova nativa: duas builds de `runtime.js`, um editor

`python3 scripts/runtime-bundle-ab.py` pergunta ao editor real, com a janela e o projeto reais, o que o laboratório respondeu sem eles. Mesmo binário nos dois braços; os braços diferem **só** no `runtime.js`
que a página carrega: o `TESSEL_RUNTIME_DIR` de cada sessão é uma pasta temporária com o bundle do braço e o `index.html` do repositório. O resto é o controle de `first-use-ab.py` (I/O assíncrono, traço ligado, aquecimento de
cena ligado, os dois fatores de primeiro uso explicitamente desligados). Cada sessão é um editor novo (HOME e socket próprios) que toca o projeto inteiro duas vezes (a primeira e uma repetição no mesmo processo),
na ordem desligado/ligado, ligado/desligado… O bundle sem a correção é um argumento (`--unpatched-bundle`, construído dos mesmos fontes com o `build.mjs` anterior, fora do repositório); antes da primeira sessão cada bundle
é lido: o sem correção precisa ter a chamada do worker uma vez, o corrigido nenhuma. Depois, cada sessão é conferida contra o registro `asset_io` **do próprio host** do `runtime.js` que serviu:
dentro de um braço todas receberam o mesmo número de bytes (o do arquivo do braço) e o bundle sem correção é exatamente 14 bytes maior; senão o run é `INVALID ARMS` e não diz nada sobre a correção.

Um vigia lê a tabela de quadros: com o editor tocando e nenhum quadro novo por 10 s, ele roda `/usr/bin/sample` no processo de WebContent novo (no máximo 2 por sessão) e guarda o arquivo. A amostra só é **deadlock** se tiver
`ReadableStreamDefaultReader::releaseLock`, a marcação do coletor (`visitOutputConstraints` ou `isReachableFromOpaqueRoots`) e `lockSlow`. Só um helper cuja amostra foi deadlock e que sobreviveu à sessão é morto; os demais
helpers restantes são apenas relatados.

Regras de decisão **fixadas antes de qualquer medida** (diagnósticas; nada aqui aceita a latência de BL157):

| Termo | Regra |
|---|---|
| play preso | menos de 90% dos quadros do projeto, ou mais de 2000 ms entre duas imagens (a regra de `first-use-ab.py`) |
| sessão presa | ao menos um play preso; sessão que dá erro ou em que o editor morre sem play preso é `failed`, nunca presa |
| NÃO REPRODUZIDO | o braço sem a correção tem menos de 3 sessões presas: o run não diz nada sobre a correção |
| CONFIRMADO | o braço sem a correção tem ao menos 3 sessões presas e o corrigido nenhuma |
| REDUZIDO | o corrigido tem algumas, menos que o outro: o travamento tem outra causa; a correção não é chamada de conserto |
| SEM EFEITO | o corrigido tem tantas sessões presas quanto o outro, ou mais |

O p exato unilateral (hipergeométrico) sai só como diagnóstico. A carga é registrada, nunca filtrada; nenhuma sessão é descartada. Pré-requisitos (`BLOCKED`, saída 2): arquivos, bundles que não são o que o braço diz,
≥ 16 GiB livres, os dois locks de GPU. Saídas em `--out`: `manifest.json`, `run-NN-braço.json`, `hang-*.txt` (amostras), `analysis.json`, `analysis.md`, `runs.sha256`; `--report-dir` publica as três últimas.
O relatório traz também **como foram os plays de cada braço** (maior espera entre duas imagens por play, gaps lentos de 62,5 e 100 ms, em quantos plays cada local do projeto teve um gap lento): é o estado do editor na
rodada, não uma comparação que este experimento decide (as sessões dividem uma máquina, não um relógio).
`scripts/runtime-bundle-ab.spec.py` tem 51 testes puros (ordem dos braços, achados do bundle, regra de play preso, assinatura do deadlock com texto realista, hipergeométrico, veredito, linhas de sessão sintéticas, o vigia, a
pasta temporária, o bundle que o host serviu, os resumos dos gaps por play); mutação: 39 de 39 mutantes mortos (script fora do repositório).

**Resultado** (72 sessões em 36 pares, 80 min; binário `4e334135…`, documento `0b456aec…`, bundles `c7d27bd2…` sem a correção e `78a0c8f4…` com ela; o host serviu o bundle certo em 72 de 72 sessões;
dados e hashes em [`viewport-real-latency-ab-results/bundle-ab/`](../viewport-real-latency-ab-results/bundle-ab/)):

| Braço | Sessões | Presas | Falhas | Inválidas | Amostras com a assinatura do deadlock |
|---|---|---|---|---|---|
| sem a correção | 36 | 1 (a 24) | 0 | 1 (a mesma) | 2 |
| com a correção | 36 | 0 | 0 | 0 | 0 |

**Veredito mecânico: NÃO REPRODUZIDO** (p exato unilateral 0,5, só diagnóstico). Na sessão 24 (sem a correção) o asset de 78.760 B ficou em `loading` desde 1,37 s, o editor seguiu vivo e a página aceitou 3 dos 1.202 quadros
pedidos; as duas amostras do vigia (13,7 s e 24,5 s) têm a cadeia completa do deadlock, a mesma do laboratório (gatilho da coleta diferente; a thread principal da página, ociosa). Isso mostra que **o defeito acontece no
editor real**; **não** mostra que a correção o evita: a 1 em 36, 36 sessões por braço dão 3 ou mais presas em ≈ 8% das vezes (um teste com 90% de poder pediria ≈ 190 sessões por braço, ≈ 7 h).
Registrado, não filtrado: a sessão 11 deixou 2 helpers de GPU vivos depois do editor (relatados, nunca mortos; já tinham saído na análise), por isso `analyze` sai com 1 (`all_ran: false`); a sessão 24 é inválida
(o aquecimento da cena nunca rodou) e conta como presa; o runner ganhou código de análise durante a rodada (a revisão que mediu é `6b1cab41…`; regras, ordem e coleta não mudaram). Mais em
[relatório §16.5](../viewport-real-latency-ab-report.md#165-prova-nativa-o-editor-real-a-mesma-build-dois-runtimejs).

### Limites e pendências de BL157.4

- **A correção atinge uma classe de falha** (página que não termina), não a pausa de 60–160 ms do corte nem a do primeiro crossfade. O `HANG` de 11,1 s do §13 recuperou e não é atribuído a ela.
- **O efeito da correção não está demonstrado.** Laboratório intercalado 4 de 493 contra 0 de 493 (p 0,062), réplica pré-registrada 0 de 300 contra 0 de 300 e janela real 1 de 36 contra 0 de 36: as duas réplicas deram
  NÃO REPRODUZIDO. Decisão pendente: manter por padrão (mecanismo comum a 3 pilhas e custo praticamente nulo) ou reverter até haver prova com poder. Para provar com a taxa da janela (1 em 36) seriam ≈ 190 sessões por braço
  (≈ 7 h); com a do laboratório somado (0,8%), ≈ 655 (≈ 24 h); uma alternativa mais barata, não iniciada, é um reprodutor sintético no laboratório (worker que lê um fluxo até o fim e chama `releaseLock()` com alocação em laço).
- A causa da espera de GPU na primeira entrada de crossfade segue **sem atribuição**; `warm_draw` desloca o custo (do quadro de entrada para o ponto do aviso), então fica opt-in. Promover a padrão exige A/B nativo pareado em
  máquina quieta (`first-use-ab.py full --factor warm_draw --pairs N`) e uma decisão deliberada: não feito aqui.
- Continuam pendentes: a prova de pixels nativa do `warm_draw` com o veredito de ativação (`first-use-ab.py pixels --factor warm_draw`) e a correlação dos fetches expandidos de `frame_sequence`.
- Nada foi feito em Rust nesta rodada. Os gates Rust foram reexecutados no head final: fmt, check, `clippy -D warnings` e build com saída 0; `test --workspace` 2.239 passaram, 0 falharam, 13 ignorados;
  runtime com Chrome real 336 de 336 (ver o relatório §16.6). Isso fecha a pendência de "gates Rust da árvore final" do §15.
- A cadência do relógio de reprodução (BL157.6, abaixo) foi **medida e explicada**; nenhuma mudança de produto.

### Reprodução de BL157.4

```sh
cd runtime && pnpm test                                      # 336 testes; TESSEL_REQUIRE_RUNTIME=1 inclui o bundle e o Chrome real
python3 scripts/runtime-bundle-ab.spec.py                    # 51 testes puros do runner nativo
python3 scripts/playback-cadence.spec.py                     # 38 testes puros do leitor de cadência (BL157.6)
# laboratório (precisa de macOS com WKWebView; as nuvens são caminhos locais seus):
node runtime/scripts/perf/run.mjs --engine webkit --suite real --cases real-entry --variants current,warmdraw --passes 30 --fresh \
  --pacing film --warmup 10 --size 1280x720 --quality preview --drain read1x1 --out <dir> \
  --real-cloud-a <nuvem-a.ply> --real-cloud-b <nuvem-b.ply> --real-ring <anel.ply> --real-sparks <faiscas.ply>
python3 scripts/runtime-bundle-ab.py run --binary <tessel-editor> --project <projeto.tessel.json> --unpatched-bundle <runtime-sem-correcao.js> \
  --out <dir> --pairs 36 --stress-cpu 6                      # janela real; trava os locks de GPU, ≥ 16 GiB livres
python3 scripts/runtime-bundle-ab.py analyze --out <dir> --project <projeto.tessel.json> [--report-dir <pasta>]   # refaz a análise dos registros salvos
python3 scripts/playback-cadence.py <dir> --group auto --markdown cadence.md   # BL157.6: lê o traço do host de cada sessão (só lê)
# réplica do laboratório (`prev` é o bundle sem a correção; 300 passes intercalados, uma página nova por run):
node runtime/scripts/perf/run.mjs --engine webkit --suite real --cases real-load --variants prev,current --baseline-bundle <runtime-sem-correcao.js> \
  --passes 300 --fresh --warmup 1 --size 1280x720 --quality preview --workerlog --load-timeout 10000 --out <dir> \
  --real-cloud-a <nuvem-a.ply> --real-cloud-b <nuvem-b.ply> --real-ring <anel.ply> --real-sparks <faiscas.ply>
```

## BL157.6 — a cadência do relógio de reprodução (leitor; nenhuma mudança de produto)

Achado lateral da análise de BL157.4: o intervalo entre imagens entregues ao monitor tem mediana 44,0 ms (24 fps pedem 41,7) e 13% das entregas chegam menos de 30 ms depois da anterior. **Não é uma pausa e este
leitor não aceita nada.** `scripts/playback-cadence.py` só **lê** o traço do host (`tick` e `play_clock_started`, com `TESSEL_VIEWPORT_TRACE=1`) dos registros de sessão que `runtime-bundle-ab.py`,
`real-latency-ab.py` e `first-use-ab.py` gravam, e imprime por grupo (braço do A/B, senão o modo, ou todas as sessões) quantas consultas do relógio cada troca de quadro levou, o intervalo real da consulta, o que o
monitor entregou e os quadros que o relógio pulou, ao lado da previsão do modelo (um quadro dura `período do quadro / intervalo da consulta` consultas). Saída em Markdown e em JSON.

| Superfície (contrato de features) | Estado |
|---|---|
| Biblioteca / core | N/A: instrumento de diagnóstico em Python que lê registros; nenhuma regra de domínio muda |
| CLI headless | N/A: o headless não tem relógio de reprodução nem host; o leitor trabalha sobre registros da janela |
| UI (janela) | O que se lê é o traço do host da janela real (`playback.rs`); nenhum controle novo |

**Resultado nas 72 sessões da prova nativa** (86.098 trocas de quadro, 24 fps; [relatório §16.7](../viewport-real-latency-ab-report.md#167-bl1576-a-cadência-do-relógio-de-reprodução-achado-lateral-nada-foi-alterado),
saídas em [`bundle-ab/`](../viewport-real-latency-ab-results/bundle-ab/)): `playback.rs` consulta o relógio `TICKS_PER_FRAME` = 2 vezes por quadro (a cada 20,83 ms) e rearma o timer **depois** de cada consulta,
que chega em média 1,4 ms atrasada: o intervalo real é 22,29 ms e um quadro de 41,67 ms vale 1,869 consultas. **13,1% das trocas de quadro são vistas na 1ª consulta (≈ 22 ms) e 86,9% na 2ª (≈ 44 ms)**: é o que o modelo
prevê (13,1% e 86,9%), nos dois braços, então a causa é o agendamento e não o bundle. É um micro-tranco regular (um quadro curto a cada ≈ 8) com a média certa (41,6 ms entregues em média); 28 quadros pulados no total.

`scripts/playback-cadence.spec.py` tem 38 testes puros (janelas de play, consultas por troca de quadro, quadros pulados, entregas, o modelo com a fronteira de ponto flutuante, percentis, grupos, leitura de arquivo e pasta, saída em
Markdown e JSON, códigos de saída 0 e 2); mutação: 39 de 39 mutantes mortos, com o controle sem mutação passando (script fora do repositório).

**Proposta, não implementada** (BL157.6 aberto, sem responsável): armar a próxima consulta para o instante em que o relógio cruza a próxima fronteira de quadro, com folga para o atraso do timer e limites mínimo e máximo,
no lugar de um intervalo fixo depois da consulta anterior; a conta do próximo instante numa função pura testada, mudança em `playback.rs` (Rust), opt-in primeiro e A/B nativo pareado. Riscos: sem a segunda consulta,
um quadro que atrasa deixa de ser absorvido; a defasagem áudio-imagem muda em até meia consulta (≈ 11 ms); o ganho é de **suavidade** e **não** reduz as pausas de 60–160 ms, que são o que não foi aceito. **Aceite proposto, a fixar
antes de medir:** entregas < 30 ms em ≤ 5% (hoje 13%), sem aumento de quadros pulados nem de esperas > 62,5 ms contra o controle.
