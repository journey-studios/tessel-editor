# Latência real — experimento A/B do I/O de assets (BL157.1) e trânsito do quadro (BL157.2, §11)

**A latência continua não aceita.** Nada aqui fecha o BL157 nem declara um limite aprovado: 62,5 ms e 100 ms são gatilhos
diagnósticos e 41,7 ms (24 fps) é só a referência de orçamento. Plano: [real-latency-ab-test-plan.md](migration/real-latency-ab-test-plan.md);
investigação anterior: [viewport-real-latency-investigation.md](viewport-real-latency-investigation.md); contrato do que foi
construído: [F063](features/viewport-asset-io.md); dados: [viewport-real-latency-ab-results/](viewport-real-latency-ab-results/).

## Estado atual

Um item por linha; as evidências estão nas seções indicadas e a regra de proveniência dos dados em [viewport-real-latency-ab-results/README.md](viewport-real-latency-ab-results/README.md).

- **BL157.1** (I/O assíncrono do protocolo, §1–§10): implementado, opt-in (`TESSEL_VIEWPORT_ASSET_IO=async`), não demonstrou redução consistente da cauda das pausas. O I/O síncrono atrasa a janela (confirmado).
- **BL157.2** (trânsito do quadro, §11): localizado (thread da página parada logo após um draw); sem correção.
- **BL157.3** (pré-upload da textura PNG e warm draw, §13): opt-in, desligados; o pré-upload tira o custo local da entrada da imagem, mas não as pausas maiores; o warm draw não mostrou benefício e teve um `HANG` sem causa.
- **BL157.4** (deadlock do WebKit no decodificador, §16): mecanismo identificado; a remoção da chamada é opt-in (`TESSEL_RUNTIME_SPARK_RELEASE_LOCK_PATCH=1`) e o efeito não está demonstrado (NÃO REPRODUZIDO: 1 de 36 sessões presas sem a correção, 0 de 36 com ela).
- **BL157.6** (cadência do relógio de reprodução, §16.7): leitor entregue; agendamento `TESSEL_PLAYBACK_POLL=deadline` opt-in; o A/B nativo não rodou.
- **Latência: NÃO ACEITA**; BL157 segue aberto. A pausa do primeiro crossfade segue sem causa atribuída.

## 1. Resultado do BL157.1 em uma página

Esta seção responde só ao BL157.1 (I/O assíncrono do protocolo); os demais sub-itens estão no "Estado atual" e nas §11–§16.

| Pergunta | Resposta | Base |
|---|---|---|
| O callback do protocolo roda na thread da janela e lê o arquivo ali? | **Sim, medido** (em `sync`, 29 leituras por sessão na thread da UI; em `async`, 0) | evento `asset_cb` com `on_ui_thread` e nome da thread |
| Essa leitura bloqueia a janela durante a pré-carga? | **Sim**: callback de 33,5–85,4 ms na UI (máximo por sessão) em todas as 6 sessões `sync` válidas (leituras de ~66 MB, 13–31 ms de leitura mais a cópia para o `NSData`) e tick atrasado em até 71,5 ms; em `async` o callback na UI é ≤ 0,12 ms e o atraso máximo do tick é ≤ 14,4 ms | tabelas §4 |
| Tirar essa leitura da thread da UI reduz as pausas entre imagens? | **Não demonstrado: o modo assíncrono, implementado e opt-in, não demonstrou redução consistente da cauda (inconclusivo; tendência de nenhum efeito).** Gaps > 62,5 ms em sessões silenciosas: `sync` 28 (5 sessões, 10 plays), `async` 27 (5 sessões, 10 plays); > 100 ms: 1 contra 2. Em 4 pares comparáveis o efeito troca de sinal | §4.3 |
| A preparação (`plans_ahead`, `prefetch`, `sync`) é o custo? | **Refutado.** `plans_ahead` ≤ 0,07 ms, `prefetch` ≤ 4,3 ms (típico < 0,1 ms), `sync` ≤ 2,8 ms | §5 |
| O que sobra nas pausas com `async`? | Pausas em que a UI não estava bloqueada, o tick estava no horário e não houve leitura: o quadro espera 56–120 ms **fora do trabalho do runtime** (trânsito host→página→host), sem span que o explique; mais a entrada de clipes de imagem (primeiro desenho de 12–18 ms) | §5 |

**Decisão (regra do plano).** Os spans confirmam o mecanismo (leitura na thread da UI atrasa a janela), mas **não** mostram que ele
é a causa principal das pausas visíveis no monitor: a cauda das pausas não cai. Portanto o modo assíncrono **não é proposto como a
correção**; fica implementado como experimento opt-in (`TESSEL_VIEWPORT_ASSET_IO=async`, padrão `sync`) com testes, e o próximo componente
demonstrado é o **trânsito do quadro fora do runtime** (ainda sem causa atribuída; primeiro medir o atraso do laço de eventos da página) e a **entrada de clipes de imagem** (§7; BL157.2, proposta, nada implementado).

## 2. O que foi construído (um fator por vez)

- **Fator único:** quem lê e responde os arquivos do protocolo `tessel://`. `sync` (comportamento anterior) lê no callback;
  `async` só **resolve** (autorização e decisão do arquivo, sem I/O) no callback e entrega a leitura a 2 workers com fila de 16
  (valores fixados antes de medir; sem thread por requisição; sem `send` bloqueante na UI). Saturação responde `503` e invalida a
  execução (nunca ganho). Mesma build, mesmo runtime, mesmo ambiente, aquecimento (`warm`) ligado nos dois braços.
- **Instrumentação** (opt-in `TESSEL_VIEWPORT_TRACE=1`; relógio monotônico do host; a página só envia durações locais, nunca
  subtraídas de carimbos do host; nomes de assets viram rótulos FNV-1a): `tick` (previsto × executado), `prefetch` (`plans_ahead`),
  `sync`, `asset_cb` (thread, `recv_ms`, `callback_ms`), `asset_io` (workers, fila, leitura, resposta, bytes), `asset_load_sent`,
  `warm_sent`, `asset_state` (fetch/decode da página) e `runtime_span` (warm da página), mais submissão/aceite do quadro. Leitura por
  `render.trace` com cursor (anel de 16 384 eventos; nenhum evento perdido nas 14 sessões válidas).
- **Runner versionado:** `scripts/real-latency-ab.py` (`preflight`, `run`, `analyze`, `pixels`) recebe `--project`; nenhum caminho
  pessoal nem mídia no Git; janela/HOME/socket próprios; só requisições semânticas; só o documento é copiado para um workspace
  privado; as 23 fontes são lidas, nunca escritas. Specs puros: `scripts/real-latency-ab.spec.py` (20 testes: ordem dos pares,
  percentis, localização dos gaps, validade, classe silenciosa/carregada, pares, gráficos).

## 3. Provas antes do benchmark

| Prova | Resultado |
|---|---|
| Mesmos status/cabeçalhos/bytes nos dois modos (arquivo inteiro, Range, sufixo, inválido, vazio, ausente, negado; 13 casos) | passa (`asset_io_tests`) |
| Controle negativo: leitura retida por FIFO; a UI responde antes de soltar a leitura em `async`; **em `sync` o mesmo teste fica vermelho** | passa/vermelho como esperado |
| Fila cheia → `503` uma vez, contada, sem descarte silencioso; troca de projeto com leituras na fila e em voo → `410`, sem resposta dupla nem resultado velho; workers e fila limitados (2 workers, fila de 16; `max_in_flight` nunca acima do número de workers) | passa |
| Pixels do projeto **sintético** (5 cenas × 200 000 splats), 4 quadros, `sync`/`async` × quente/frio, `render.frame` | **idênticos byte a byte** nas 4 sessões; o monitor mostra a mesma imagem. Um `render.frame` explícito foi substituído por um pedido mais novo do próprio monitor (`cancelled`, em `sync` e em `async`) e foi repetido; a saída gerada não foi mantida no Git ([proveniência](viewport-real-latency-ab-results/README.md)). A primeira rodada reprovou o veredito por isso e foi corrigida no runner (nova tentativa só para `cancelled`), não nos dados |
| Export | N/A: o export não passa pelo protocolo da WebView |

## 4. A/B no projeto real

**Condições.** Projeto real horizontal de 25 s (600 quadros, 24 fps, 23 fontes presentes, cortes lidos da timeline em 186/268/410,
clipes de imagem entram em 18/196/282/326/370/512/520; documento sha256 `e93363aecc4c0591…`). Binário release sha256
`ad16ad3670f4a58f…` (o mesmo nas 16 sessões e nos pixels), runtime sha256 `a5bc2ad06d1bdf9e…`, `TESSEL_GL=gpu`, dois locks de
GPU, uma sessão por vez, janela própria. Cada sessão: um editor novo, **primeiro play completo e play repetido com cache residente,
analisados separadamente**. Sem purge e sem afirmação de disco frio. Ordem alternada `sync/async, async/sync, sync/async,
async/sync`, **duas rodadas** de 4 pares (a segunda foi pedida por mim depois que a primeira teve uma sessão inválida; nenhuma
sessão foi descartada, e os números por rodada não foram mantidos no Git: a tabela completa das 16 sessões está em [analysis-all-16-sessions.md](viewport-real-latency-ab-results/analysis-all-16-sessions.md)). A máquina **não estava ociosa**: carga média 3–20 e CPU
externa mediana 1,0–6,0 núcleos (processos de outras sessões); o runner classifica cada sessão com limiares fixados antes
(`quiet`: mediana ≤ 3 núcleos e p95 ≤ 6; par comparável: ambas quietas, válidas e com diferença de CPU externa ≤ 2).

**Sessões inválidas (mantidas).** Sessão 3 de cada rodada (`sync`): o editor encerrou sozinho. Rodada 1: `Connection refused` no
play repetido (após 90 quadros; o log do editor já não foi preservado, o runner foi corrigido depois para registrar isso). Rodada 2:
`Connection refused` a 11,9 s de sessão, com **código de saída 0**, sem pânico nem relatório de falha do sistema. No `main.rs` só
`on_window_closed` com zero janelas e uma falha de automação chamam `quit`; código 0 por pânico não ocorre. **Causa não
determinada** (hipótese não provada: a janela foi fechada por fora, já que o runner a traz para a frente com `window.activate`).
Sinalizo como INCONCLUSIVO: não há evidência de que o modo `sync` encerre o editor (as outras 6 sessões `sync` válidas rodaram 2 plays
completos), mas também não a excluo. Os dois pares afetados ficam fora dos comparáveis.

### 4.1 Sessões (resumo; tabela completa em [analysis-all-16-sessions.md](viewport-real-latency-ab-results/analysis-all-16-sessions.md))

| Modo | Sessões válidas (quietas/carregadas) | Callback máx. na UI (ms) | Atraso máx. do tick (ms) | RSS editor mediana/pico (MiB) | RSS helpers mediana/pico (MiB) |
|---|---|---|---|---|---|
| `sync` | 6 (5/1) | 33,5–85,4 | 15,1–71,5 | 164–258 / 204–261 | 507–1125 / 866–1695 |
| `async` | 8 (5/3) | 0,02–0,12 | 7,6–14,4 | 126–252 / 256–263 | 461–977 / 688–1475 |

RSS é o residente de cada processo (editor; processos WebKit novos desde a linha de base), **não** soma de memória física nem
memória de GPU; a dispersão entre execuções supera a diferença entre os modos. Pool `async`: no máximo 2 jobs na fila e 2 em voo,
no máximo ~66 MB retidos, 0 saturações, espera máxima na fila 12 ms. GPU: **não medido**.

### 4.2 Distribuições por play (todas as sessões válidas, intervalos entre imagens aceitas)

Mediana dos intervalos 43,2–44,4 ms nos dois modos (relógio de 24 fps). p95 por play: 44,5–48,6 ms (`sync`) e 44,5–49,3 ms (`async`).
Máximos por play e contagens > 62,5 / > 100 ms estão por sessão no arquivo completo; os resumos por grupo:

| Grupo | Sessões | Plays | Imagens | Gaps > 62,5 | > 100 | Pior intervalo por sessão (ms) |
|---|---|---|---|---|---|---|
| `sync` silencioso | 5 | 10 | 5 922 | 28 | 1 | 81,5; 65,9; 116,5; 93,8; 84,7 |
| `async` silencioso | 5 | 10 | 5 942 | 27 | 2 | 57,6; 67,5; 165,8; 66,4; 93,8 |
| `sync` carregado | 1 | 2 | 1 187 | 11 | 3 | 118,4 |
| `async` carregado | 3 | 6 | 3 582 | 24 | 4 | 111,2; 90,3; 139,2 |

Os quadros de um play são correlacionados; estas são contagens descritivas, não amostras independentes. Imagem após o comando de Play
no primeiro play: 142–170 ms (`sync`) e 145–170 ms (`async`); no play repetido há saídas de 1,4–1,7 s nos dois modos.

### 4.3 Pares (4 pares comparáveis; é um começo de engenharia, não prova de produção)

| Par | Play | Gaps > 62,5 `sync`→`async` | Maior intervalo `sync`→`async` (ms) |
|---|---|---|---|
| 0 | primeiro | 0→0 | 59,0→57,3 |
| 0 | repetido | 3→0 | 81,5→57,6 |
| 4 | primeiro | 1→9 | 66,1→136,2 |
| 4 | repetido | 9→8 | 116,5→165,8 |
| 6 | primeiro | 4→1 | 93,8→66,4 |
| 6 | repetido | 3→2 | 86,6→66,4 |
| 7 | primeiro | 1→2 | 63,9→93,8 |
| 7 | repetido | 4→4 | 84,7→86,1 |

O efeito muda de sinal: na contagem de gaps > 62,5 ms `async` é melhor em 4 dos 8 plays, pior em 2 e igual em 2; no maior intervalo,
melhor em 4 e pior em 4. Os pares não comparáveis (carga alta, diferença de CPU externa ou sessão inválida) estão no arquivo completo e não entram nesta
tabela. Os gráficos por par (intervalo entre imagens contra o quadro; sombreado = 2 s antes do corte; verde = corte) e a linha do tempo de spans
de uma sessão `sync` e uma `async` são gerados pelo runner (`analyze --report-dir`) e não foram mantidos no Git ([proveniência](viewport-real-latency-ab-results/README.md)).

## 5. O que cada pausa estava fazendo (correlação, não causa)

Cada gap > 62,5 ms foi cruzado com os spans que o sobrepõem (sessões válidas, todos os braços):

| Grupo | Gaps | com callback na UI ≥ 20 ms | com decode/fetch/warm da página ≥ 10 ms | com trânsito do quadro ≥ 30 ms | sem parada de UI **nem** trabalho da página | em janela de pré-carga | em corte | em entrada de clipe de imagem | outros |
|---|---|---|---|---|---|---|---|---|---|
| `sync` silencioso | 28 | **10** | 12 | 5 | 16 | 12 | 1 | 12 | 3 |
| `async` silencioso | 27 | **0** | 5 | 6 | 22 | 9 | 3 | 11 | 4 |
| `sync` carregado | 11 | 2 | 2 | 3 | 9 | 3 | 2 | 5 | 1 |
| `async` carregado | 24 | 0 | 5 | 8 | 19 | 6 | 4 | 12 | 2 |

- **Confirmado:** em `sync`, 10 dos 28 gaps silenciosos coincidem com um callback de 20–85 ms na UI, todos em janelas de pré-carga,
  com tick atrasado nas sessões `sync` (máximo por sessão de 15,1–71,5 ms; ver tabela de memória e paradas). `async` zera esse subconjunto (0 em 51).
- **Mas o total não cai**, porque o resto das pausas é de outra origem: no `async`, 22 de 27 não têm parada de UI nem trabalho da
  página sobreposto e o tick estava no horário (≤ 14 ms). Os maiores (166, 139, 136 ms; `video:370`, `cut:410`, `cut:410`) têm
  `runtime_total` de 1–24 ms e **trânsito** (envio→aceite menos o total do runtime) de 92–120 ms, sem leitura, sem decode/fetch/warm da
  página e sem parada de UI sobrepostos nos dois `cut:410`. O quadro esperou **fora** do trabalho do runtime e **nenhum span meu
  explica onde**: pode ser a thread da página ocupada com trabalho não medido (GC, ordenação do Spark), a entrega do `evaluate` ou o
  processamento do IPC pela UI, ou o escalonamento dos processos WebKit sob a carga externa. **Causa não atribuída.**
- **Entradas de clipe de imagem** (quadros 18/196/282/326/370/512/520) concentram 11–12 gaps por grupo nos três grupos com 5 sessões ou mais, e 5 no grupo de 1 sessão, nos dois modos, nos mesmos
  quadros, sem leitura sobreposta: o asset já está residente e o primeiro desenho custa 12–18 ms de `draw` (mediana por entrada, as duas
  rodadas). Isso explica 20–37 ms a mais no intervalo, não os 90–110 ms dos piores; **não** é I/O de protocolo.
- `plans_ahead`, `prefetch` e `sync` do host: máximos 0,07 / 4,3 / 2,8 ms (medianas < 0,1 ms). **Refutados** como custo.

## 6. Hipóteses

| # | Hipótese | Estado |
|---|---|---|
| H1 | O callback do protocolo roda na thread da janela | **Confirmada** (medido por requisição) |
| H2 | A leitura síncrona bloqueia a janela e atrasa o tick durante a pré-carga | **Confirmada** (callbacks de 24–85 ms; tick de até 71 ms; zerados em `async`) |
| H3 | Tirar a leitura da UI reduz as pausas de imagem | **Não sustentada / inconclusiva**: contagem total igual, pares com sinais mistos; só o subconjunto "com parada de UI" desaparece |
| H4 | O custo está na preparação do plano (`plans_ahead`) fora do tick | **Refutada** |
| H5 | `async` desloca o custo para antes do corte (fila/espera) | **Não observada**: espera na fila ≤ 12 ms, 0 saturações; gaps em janela de pré-carga 9 (`async`) contra 12 (`sync`) nas sessões silenciosas |
| H6 | O decode do splat ou o warm na thread da página explicam as pausas restantes | **Parcial**: spans da página se sobrepõem a 5–12 gaps por grupo, mas os maiores gaps do `async` (59–120 ms de trânsito) **não** têm span sobreposto; sem atribuição |
| H6b | Entrar num clipe de imagem custa o primeiro desenho | **Confirmada como custo de 12–18 ms por entrada**, insuficiente para os piores gaps |
| H7 | O modo assíncrono é a correção da latência | **Não proposta como correção**; opt-in |

## 7. Próximo componente (proposta de BL157.1; investigada em BL157.2, §11)

BL157.2, um fator por vez, dentro do escopo protegido, **em ordem de evidência**: (1) **atribuir o trânsito**: instrumentar (só
durações locais, nunca subtraindo relógios) o atraso do laço de eventos da página (batimento/`longtask`), a duração do tratamento do
IPC pela UI e a fila do `evaluate`, para separar "thread da página ocupada" de "entrega"; sem isso qualquer correção seria aposta;
(2) **entrada de clipes de imagem**: pré-subir/pré-desenhar a textura do PNG antes de o clipe entrar (12–18 ms por entrada, mesmos
quadros nos dois modos); (3) **decode do splat em fatias/Worker** (~158 ms por asset de 66 MB), só se (1) o apontar. Cada fator ganha
seu A/B no mesmo runner (`--project`), com o modo de leitura fixo, e a proteção do quadro atual não pode virar uma pausa de 2 s antes
do corte.

## 8. Limites

- Duas rodadas de 4 pares, máquina compartilhada (carga 3–20): é um começo de engenharia, não prova de produção; o par 4 (`async`
  pior, ambas quietas) e o par 6 (`async` melhor) mostram que o ruído entre sessões é do tamanho do efeito na cauda.
- Dois pares perderam uma sessão (editor encerrou sozinho, causa não determinada).
- No modo `sync` o custo do `NSData` do Wry entra no callback (inline); em `async` ele corre no worker. `async` mantém um corpo de até
  66 MB por worker em memória.
- Sem GPU medida, sem apresentação no fóton (o aceite é a imagem aceita pelo host), sem disco frio, sem ponteiro físico.
- O headless (processo JSONL) não passa pelo protocolo: `render.trace` responde `desktop_required`.

## 9. Gates

Os gates do A/B de BL157.1 (binário `ad16ad36…`) foram do head `1454413`; dois deles ficaram **BLOCKED** por lock de GPU. Os gates abaixo são
os do código final de BL157.2 (commit `a01090b` sobre `origin/main` integrado; depois dele só docs e a correção de revisão de `runtime/src/assets.js`, ver a linha do runtime), com saída e
código gravados em log local, não versionado (sem pipe mascarando o código).

| Gate | Resultado |
|---|---|
| `cargo fmt --all -- --check` | 0 |
| `./scripts/cargo check --workspace --locked` | 0 |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 (o head recuperado `be56bfd` falhava em `collapsible_if` em `asset_io.rs`; corrigido com let-chain) |
| `cargo tree -p tessel-editor-core` sem `gpui`/`wry` | 0, nenhuma ocorrência |
| Build debug e release de `tessel-editor` | 0 e 0; sha256 do release `ac023ebadf863d1d` (reconstruído depois de o mbx recolher o artefato: mesmo sha) |
| `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked --no-fail-fast` | 0; 2083 passaram, 0 falharam |
| `./scripts/runtime setup` (do zero) e `./scripts/runtime test` (Chrome, sem SKIP) | 0 e 0 no código medido (223/223, `runtime.js` `1fed1ef9…`). Depois da correção de revisão em `assets.js` (timings de carga cancelada/recusada, +2 testes): 225 testes, **uma execução teve 1 falha** (`video B2: a frame superseded while it seeks…`, corrida de tempo com carga 31 na máquina; 3 de 3 passaram isolado) e a **repetição passou 225/225, 0 skipped** (`runtime.js` `fcde5a38…`); `loop5` (3 editores novos) roda com esse runtime |
| 16 specs Python `scripts/*.spec.py` (inclui `viewport-transit-probe`, 18 testes) | todos 0 |
| Headless JSONL (`./scripts/headless`) | `render.trace` → `desktop_required` (sem tocar o estado), `status` ok; sai com 1 pelo erro pedido |
| Janela real, binário release `ac023eba…` (HOME/socket próprios, só pedidos semânticos de `./scripts/ui`/`Lab`, sem coordenadas) | `real-latency-ab.py pixels`: PASS (imagem do `render.frame` idêntica byte a byte em quente/frio × cache quente/seek frio, controles negativos: cortes distintos não compartilham imagem e todo quadro foi lido); `loop4` (4 editores novos, `render.status.asset_io.mode == async` verificado por run) e `ab4` (16 runs válidos) |

**Segunda rodada de correções de revisão** (`2b4b197`: o pacote da página não passa mais pelo pool em `async`; `665ec4d`: `redact` e
`ab-report`, os dois apagados depois, na revisão thermo da PR #41: [F063](features/viewport-asset-io.md#núcleo-comum-dos-runners-scriptsab)). Gates refeitos no código final (log local, não versionado): fmt 0, check 0, clippy `-D warnings` 0,
`cargo tree` sem `gpui`/`wry`, build debug e release 0 (release `4add680ead4d37f9`, reconstruído depois de o mbx recolher o artefato:
mesmo sha), `cargo test --workspace --locked` 0 (2084 passaram, 0 falharam), `./scripts/runtime test` 0 (225/225), specs
`viewport-transit-probe` e `real-latency-ab` 0. **BLOCKED:** o `loop` do editor final (`4add680e…`) esperou 120 s pelos locks de GPU
(`/tmp/tessel-scene-gpu.lock`, `/tmp/tessel-video-bl140-bl144-review-gpu.lock`) e não rodou; não foi repetido. As medições de §11 e
`loop4`/`loop5`/`ab4` são do binário `ac023eba…`; a diferença para o final é só a leitura do pacote da página (uma vez por
carga da página), **sem medição própria**. Os resultados regenerados em `bl157-2/` usam a regra de validade mais estrita (run
sem imagens conta como inválido).

## 10. Reprodução (BL157.1)

```text
./scripts/runtime setup && ./scripts/cargo build -p tessel-editor --locked --release
python3 scripts/real-latency-ab.py preflight --project <documento .tessel.json>
python3 scripts/real-latency-ab.py pixels  --binary target/release/tessel-editor --out <dir>
python3 scripts/real-latency-ab.py run     --binary target/release/tessel-editor --project <documento> --out <dir> --pairs 4
python3 scripts/real-latency-ab.py analyze --out <dir> --project <documento>
python3 scripts/real-latency-ab.spec.py
```

As sessões brutas (1,5 MB cada) e a lista de seus sha256 (rodada 1 `ab/`, rodada 2 `ab2/`) ficam fora do Git ([proveniência](viewport-real-latency-ab-results/README.md)).

## 11. BL157.2: onde está o trânsito do quadro

**Latência continua NÃO ACEITA; nenhuma mudança de produto foi feita; BL157 segue aberto.** Contrato dos probes: [F063](features/viewport-asset-io.md#bl1572--probes-do-trânsito-do-quadro-opt-in-o-mesmo-custo-nos-dois-braços).
Dados do lote final em [viewport-real-latency-ab-results/bl157-2/](viewport-real-latency-ab-results/bl157-2/) (`loop4-final-analysis.md` e `ab4.md`, binário `ac023eba…`, runtime `1fed1ef9…`, código `a01090b`).
Os lotes anteriores (`loop1`–`loop3`, `loop5`, `ab1`–`ab3`) usaram binários, runtimes e documentos diferentes e **não se somam**; as análises deles não foram mantidas no Git, e o que importa deles está resumido aqui
(§11.1, §11.3) e na [proveniência](viewport-real-latency-ab-results/README.md).
Projeto real (23 assets, só o documento foi copiado), janela 114–210 (3 s antes do primeiro corte até 1 s depois), I/O `async` fixo, TRACE e page
timing ligados nos dois braços, um editor novo por run (**todos são primeiras execuções de um processo frio**; a repetição no mesmo processo
não foi medida), cargas externas registradas por run (`quiet`/`loaded`).

### 11.1 Hipóteses

| Hipótese | Resultado | Base |
|---|---|---|
| A fila do `evaluate` do host ou a preparação no host atrasam o envio | **Refutada** | `pre-send` ≤ 0,012 ms em todos os gaps lentos |
| A mensagem espera a tarefa da janela depois de chegar ao WebView (`ipc_wait`) | **Refutada** | p95 ≤ 0,4 ms; máx. 5,7 ms |
| O pedido espera na fila serial da página | **Refutada nos gaps do corte** | `queue_ms` 0 |
| O runtime demora a desenhar o quadro do corte | **Refutada** nos cortes (1–6 ms), **confirmada** em `195→196` (24–31 ms) | `page_draw` |
| O laço de eventos da página fica parado | **Confirmada** | `loop_stall` máximo de 124–174 ms nos 4 runs do `loop4`, `eval_done` ≈ trânsito, fim do stall logo antes de o script entrar |
| A parada está dentro de uma chamada WebGL medida | **Refutada** | nenhum `gl_call` ≥ 4 ms nesse intervalo (só `texSubImage2D` de 13–27 ms em outro lugar) |
| Há leitura de asset em voo na hora | **Refutada** | sem `asset_io`/`asset_cb` entre os quadros |
| A parada é GPU ainda trabalhando depois do draw | **Consistente, não provada** | `ab4` (8 pares, 16 runs válidos): a leitura de 1 pixel esperou ≥ 40 ms em 8/8 runs do braço B (máx. 41–119 ms), nos quadros dos 1ºs draws e nos do corte; onde A tem `187→188`, B tem `185→186`/`186→187` de 65–70 ms (41,7 + a espera): **o atraso se moveu para dentro do quadro**. A reproduziu um stall ≥ 40 ms em 6 de 8 runs |
| `gl.finish()` espera a GPU no WebKit | **Inconclusiva** | `ab1`: `gpu_finish_ms` 0 em todos os quadros: a manipulação falhou (a chamada voltou sem esperar), o que não mostra que `finish()` nunca espera; o braço passou a usar uma leitura de 1 pixel |
| Pré-upload da 1ª textura PNG tira os gaps `195→196` | **Testada no §13: benefício local, não resolve as pausas maiores** | o dado apontava (`texSubImage2D` 13–27 ms na 1ª imagem de cada sequência); no §13 a entrada da imagem 196 na 1ª reprodução caiu de 58,4/72,4/110,9 para 44,2/51,6/55,5 ms (p50/p95/max), mas as pausas maiores do corte seguem |

### 11.2 Números (lote final)

`loop4` (4 editores novos, `loaded`): 14 gaps > 62,5 ms; dominantes: `delivery` (8 gaps de 68–185 ms com trânsito de 27–161 ms: `115→116`, `116→117`, `187→188`, `188→189` com
63–161 ms, e dois em `139→140`/`140→141`, coincidentes com a entrada de uma carga, com 27 ms), `page_draw` 3, sem dominante 3. `ab4` (A: 8 runs, B: 8 runs, ordem alternada; A com 4 runs `quiet` e 4 `loaded`, B com 5 e 3; por run na tabela):
gap máximo por run A 50,0–101,8 ms (mediana 65,4), B 64,7–131,7 (mediana 67,1, mas B carrega a espera que impôs); gaps > 62,5 ms por run A 0–4, B 1–3. Nenhum run inválido em `ab4`.
Lotes anteriores (binários diferentes): `ab1` manipulação falhou; `ab2` 3 de 8 runs inválidos (página parou de responder) → `INCONCLUSIVE`; `ab3` 1 de 12 inválido,
veredito mecânico `CONSISTENT` com A reproduzindo o stall em 3 de 6 runs.

### 11.3 Falhas mantidas

- Em 5 de 28 runs de `ab1`–`ab3` (2 do braço A, 3 do B) o pipeline de quadros da página **parou de responder** (≈ 100 % dos quadros cancelados pelo host, a página ainda atendia
  assets). Não ocorreu em `loop2`–`loop4` nem `ab4`. Causa não determinada (suspeitas não testadas: janela oclusa/estrangulada pelo WebKit com a máquina carregada; um draw esperando uma
  promessa que não resolve). Os runs ficam como inválidos nas tabelas.
- Em `loop1`, um de 3 editores teve dois passos de prefetch de ~3 s na thread da janela (UI bloqueada, sem imagens). Os eventos por carga (`prefetch_asset`) só existem a partir do `loop2`;
  não se repetiu nos 60 runs seguintes. Causa não determinada.
- Veredito mecânico do `ab`: só `CONSISTENT`/`INCONCLUSIVE`; a regra antiga (`REFUTED` pela contagem de stalls) estava confundida (a espera do braço B também é um stall) e foi trocada
  antes de ser usada como resultado; o `analyze` reaplica a regra vigente a cada lote (o `ab-report` que fazia isso foi apagado na revisão thermo da PR #41).

### 11.4 Limites e próximo passo

Sem medida de fóton/display; `gpu_finish_ms` é o tempo de uma leitura que não volta antes de a GPU terminar o que foi desenhado, no relógio da página. Máquina compartilhada (carga 14–33
nos lotes). Não se declara causa única: a GPU pendente é **compatível**, e o braço **moveu** o atraso em vez de eliminá-lo. A alavanca implicada (custo de GPU do draw do corte: dois
cenas splat) mexe em resolução, MSAA ou contagem de splats, que não foram variados e dependem de decisão de produto. O efeito de um draw de aquecimento **não foi medido**, e nenhum teste daqui separa custo de preenchimento de custo de primeiro uso: não se conclui que ajudaria nem que não ajudaria.
Próximos fatores, um por vez e dentro da proteção do quadro atual: pré-upload da 1ª textura PNG de cada sequência (`195→196`) e investigação das páginas que param.

Reprodução: `python3 scripts/viewport-transit-probe.py loop --project <documento> --out <dir> --iterations 4` e `... ab --pairs 8`; o `analyze` regenera as tabelas de um lote **novo** (com `batch_id` no manifesto): os lotes desta seção são anteriores a ele e o `analyze` de hoje os recusa (BL157.13).


## 12. Reprodução final pela coordenação (`7850ade`)

Os locks estavam disponíveis nesta rodada. O teste pendente foi executado no código limpo
`7850adea60eff1babe2ff72153a426bbd304a384`, com release SHA-256
`4add680ead4d37f9623999b03c8cfde268f48c5a0bc9bb6519f81f6537db48f8`
e runtime `fcde5a3846b2c222bf6c27927cb88cb510ee515949a29ac84d64afc2061f8d81`.

- `viewport-transit-probe.py loop --iterations 2 --lock-wait 1`, projeto real,
  I/O async, warm ligado, janela própria, frames 114–210: duas execuções válidas,
  ambas carregadas, nenhuma UI block de 50 ms e nenhum play sem imagens.
  **Exit 1 esperado do diagnóstico RED**, não um teste verde de latência.
- Iteração 0: três gaps >62,5 ms, pior 140,598 ms; 188→189 teve 100,609 ms de delivery
  com draw de 2 ms. Iteração 1: dois gaps, pior 101,713 ms (220→222).
- Primeira imagem PNG em 195→196: gaps 67,275 e 63,463 ms, draw 28 e 16 ms.
  É um custo localizado a testar; não explica sozinho o atraso de 140,598 ms.
- `real-latency-ab.py pixels` repetido no mesmo binário/runtime final: exit 0,
  hashes exatos iguais nas quatro sessões e monitor após pausa igual ao quadro explícito;
  controles de quadros distintos e readback completos passaram. Corpus sintético,
  não prova o pixel de todos os quadros do projeto real.

A decomposição por execução desta reprodução não foi mantida no Git ([proveniência](viewport-real-latency-ab-results/README.md)).
Logs e dados brutos: log local, não versionado; nenhum asset privado no Git.
Não foi necessário repetir build/workspace por estes novos documentos. O bloqueio dos locks
registrado no §9 era daquela tentativa; a reprodução final acima agora foi executada.
Nenhuma correção de latência foi implementada nesta rodada da coordenação. BL157 segue não aceito.

## 13. BL157.3: pré-upload da textura PNG e warm draw (PR #41, `fb0036f` + docs)

Dois fatores de custo de primeiro uso, **opt-in, desligados por padrão**, medidos separadamente no mesmo binário (`4e334135…`, runtime `f721cd65…`,
documento `e93363ae…`), com `async` e warm do sort ligados nos dois braços, `gpu_finish=false`, `TESSEL_GL=gpu`. Contrato, regras, provas de Chrome e
mutação: [F063 §BL157.3](features/viewport-asset-io.md). Runner `scripts/first-use-ab.py` (+ spec, 19 testes) sobre `real-latency-ab.py` e
`viewport-transit-probe.py`. Dados: [first-use/](viewport-real-latency-ab-results/first-use/) (`analysis.md` dos dois conjuntos do projeto completo, `full-preupload` e `full-warm-draw`; a rodada estreita, os `manifest.json` e as listas `runs.sha256` não foram mantidos no Git, e os JSON
brutos ficam fora dele por conterem o projeto privado; [proveniência](viewport-real-latency-ab-results/README.md)).
Limite de proveniência: o commit `d028360` (pré-upload), já publicado, carrega um trailer de coautoria do ambiente e não foi reescrito; `fb0036f` e os posteriores não.

Execução: janela estreita 114–210 (1 par por fator, 2 sessões) e projeto real completo (4 pares alternados controle/ligado, 8 sessões por fator),
**20 sessões com coleta válida, todas mantidas**, incluindo um `HANG` de reprodução (validade da coleta não significa reprodução saudável). Classes: nas 16 sessões completas, 14 "carregadas" e 2 "quietas" (carga média de outras
tarefas 9–57); **0 de 4 pares comparáveis por fator**. Nenhuma tolerância foi relaxada.

| | A pré-upload | B warm draw |
|---|---|---|
| Entrada PNG 196, 1ª reprodução (n=28), p50/p95/max ms, controle → ligado | 58,4/72,4/110,9 → **44,2/51,6/55,5** | 57,9/71,6/76,2 → 64,1/113,6/129,2 |
| Entrada PNG, repetições | 56,0/76,4/93,5 → 44,1/48,9/49,0 | 62,6/82,9/86,5 → 56,0/114,7/122,0 |
| Entrada do corte (n=12), p50/p95/max | 49,4/68,7/68,7 → 50,4/59,8/59,8 | 50,0/73,6/73,6 → 47,2/142,7/142,7 |
| Intervalos > 62,5 ms por run (1ª reprodução) | 3,9,4,1 → 3,1,1,3 | 1,3,11,5 → 6,4,12,9 |
| > 100 ms por run | 0,1,0,0 → 0,0,0,0 | 0,0,1,0 → 2,0,3,3 |
| Excesso pré-corte por run, ms | 320/439/358/389 → 374/338/308/393 | 349/357/537/374 → 500/436/416/710 |
| Memória, mediana das medianas / picos por run (MiB) | 865 / 996–1395 → 958 / 1034–1569 | 777 / 792–1168 → 640 / 677–1412 |
| `HANG` (play com < 90 % dos quadros ou espera > 2 s) | 0 | 1 (par 2, repetição: 11,1 s, 165 de 600 imagens, pico de carga 56,7) |
| Veredito mecânico (regras pré-fixadas) | `SHIFTED` (1 de 4 pares com +53,6 ms); agregado dentro da dispersão | sem benefício local |

**Leitura:**

1. **A** mostrou uma entrada de imagem menor nesta amostra; o trabalho **local** da primeira imagem é antecipado (o `texSubImage2D` de 13–27 ms sai do corte da imagem e vai para um passo de
   pré-upload de ≤ 48 ms atrás dos quadros em espera). **Não resolve as pausas grandes** (entradas de corte iguais; `prefetch:268` 63–79 ms nos dois
   braços) e portanto **não fecha BL157**. Não há deslocamento demonstrado para antes do corte, mas o excesso de um par cresceu e a carga externa
   impede afirmar que não houve.
2. **B** não sustentou a hipótese de custo de primeiro uso do draw: não reduziu a entrada do corte e o braço ligado teve mais pausas. Com carga
   desigual isso não demonstra piora nem ausência de efeito; não houve benefício demonstrado. Não há evidência para promovê-lo a padrão.
3. O `HANG` do B é mantido e **sem causa determinada** (laço da página com `loop_stall` de 4,7 s e `clientWaitSync` de 349 ms em carga 56); a mesma
   classe de parada ocorreu sem o fator em `ab1`–`ab3` (5 de 28 runs).
4. Memória: sem diferença atribuível; picos de até 1569 MiB no A ligado (um run), nenhuma promessa.
5. Pixels: ver §13.1.

### 13.1 Gates e provas de pixels no código final

Gates no código final (`./scripts/cargo`; saídas em log local, não versionado; as fontes Rust são as de `fb0036f`, depois só
scripts e docs):

| Gate | Resultado |
|---|---|
| `cargo fmt --all -- --check` | 0 |
| `./scripts/cargo check --workspace --locked` | 0 |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 |
| `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked --no-fail-fast` | 0; 2153 passaram, 0 falharam, 12 ignorados; sem SKIP |
| Build debug e release de `tessel-editor` | 0 e 0; sha256 do release `4e334135…` (o mesmo binário das medições) |
| `./scripts/runtime setup` e `./scripts/runtime test` (Chrome real) | 0 e 0; 256/256 sem SKIP |
| Specs Python (`first-use-ab`, `real-latency-ab`, `viewport-transit-probe`, `scene-warm-proof`) | 0 (19, 22, 18 e a do `scene-warm-proof`) |
| `cargo tree -p tessel-editor-core` | sem `gpui`/`wry` |
| Headless JSONL | N/A para os dois fatores (`render.trace` e o relato da janela exigem o desktop; a regra do fator é testada no core, `bridge`/`session`) |
| Janela real, binário `4e334135…`, só pedidos semânticos, sem coordenadas | `first-use-ab.py pixels --factor warm_draw`: PASS (ver F063); pré-upload: N/A na janela (o projeto sintético não tem imagens), provado em Chrome real |
| Locks de GPU | uma tentativa de 15 s nas execuções de desempenho e de pixels: adquiridos; nenhum lock removido, nenhuma espera repetida |

A primeira tentativa de pixels foi **BLOCKED** pelo piso de disco do runner (15,3 GiB < 16); depois, com a prova escrevendo < 1 GiB, o piso inicial
foi passado explicitamente (`--min-free-gib 13`, acima do piso duro de 12 GiB) e o resultado acima é dessa execução.

Hipóteses: **componente local observado:** a primeira imagem PNG paga `texSubImage2D`; o pré-upload desloca essa chamada antes da entrada.
O A/B sob carga não estabelece uma redução causal da latência total. **Sem benefício demonstrado nesta amostra:** o draw de aquecimento;
zero pares comparáveis não permitem refutar seu efeito sob condições controladas. **Em aberto:** a parada de 60–160 ms da página depois do draw do corte (GPU pendente é consistente, não
provado); as páginas que param sob carga.

## 14. Integração e resíduos de revisão após `5adf5b9`

A coordenação integrou a `main` por merge (`08cdfd4`), preservando FX/chaves/grão e resolvendo o conflito da matriz.
Os gates de §13.1 continuam históricos de `5adf5b9`: não validam a árvore integrada nem as correções seguintes.

Reproduções puras antes dos fixes: dois testes de carga concorrente falharam (a carga antiga apagava os tempos da nova);
a redação falhou para quatro caminhos com nomes privados; a reconstrução falhou tanto com brutos parciais como com restos de outro lote.
As correções associam tempos ao resultado admitido, removem caminhos completos e respeitam estritamente o conjunto de runs salvo.
O lote `loop1` é histórico (faltam seus JSONs brutos e as regras antigas incluíam assentamento e play sem imagens); a análise dele não foi mantida no Git.

A correlação dos caminhos de sequências continua uma lacuna explícita (padrão não é caminho expandido); não é usada para concluir o teste de imagens paradas.
O `HANG` de 11,1 s está retido como falha observada de reprodução, mesmo com os requisitos de coleta satisfeitos.
Zero pares comparáveis impedem atribuir causalmente a diferença agregada. O resultado de pixels nativos sem evento `drawn`
comprova compatibilidade da execução e não ativação do warm draw.

Próximo diagnóstico de BL157.3: em uma execução com recursos e carga comparáveis, confirmar a manipulação na janela e manter o loop vermelho
para a pausa de corte; separar a espera de GPU, o trabalho de prefetch e a página que deixa de responder, uma variável por vez.
Sem redução de qualidade ou promoção dos switches a padrão. A latência continua **NÃO ACEITA**.

Verificação atual das correções (árvore com integração `08cdfd4`): `cargo fmt --all -- --check`, `node --check` dos dois módulos,
15 testes Node puros (telemetria e corpus básico), specs Python `real-latency-ab` 23, `viewport-transit-probe` 21,
`first-use-ab` 19 e `scene-warm-proof` 24, nos modos normal e `-O`, e `py_compile` passaram.
Logs: log local, não versionado. Builds/check/clippy/workspace/Chrome/janela/A/B da árvore integrada
não foram iniciados: 14 GiB livres, abaixo da reserva de 16 GiB. Nada foi limpo nem houve redução do piso para rodá-los.

## 15. BL157.3: o que os testes unitários provam, sem janela (PR #41, após `b53775e`)

A pergunta foi: "isso não dá para fazer com unit testing?". Dá para as **decisões**, não para as **medidas**. Matriz teste → garantia, cadeia de
ativação, mutação e limites: [F063](features/viewport-asset-io.md#bl1573--o-que-os-testes-unitários-provam-e-o-que-não-podem-provar).
Nenhum comportamento do produto mudou (o único toque em código de produto é o parâmetro `createCompositor` de `createRuntime`, com o `Compositor`
real como padrão) e **nenhuma medida de latência foi produzida**.

| Validação pendente | O que existe agora | Continua aberto |
|---|---|---|
| Gates Rust da árvore integrada | `b53775e` (sem esta entrega): fmt, check, clippy `-D warnings`, `test --workspace` (113 suítes, 2238 passaram, 0 falharam, 13 ignorados), build debug, `gpui`/`wry` fora do core. Depois: `fmt` limpo e os 19 specs Python do repositório verdes | `clippy --workspace --all-targets -D warnings` e `cargo test --workspace` da árvore final **não foram reexecutados**: o `disk-guard` do repositório recusou (5 GiB livres, piso de 6 GiB) e nada foi limpo nem teve o piso reduzido. O único código Rust novo é um teste do host, que compilou e passou antes da queda do disco; `rustfmt` limpo |
| Chrome real: cancelamento, retenção, perda de contexto, corrida entre cargas | 60 testes de Node puro (store 8, página 15 + 13 + 4, contrato 19, fila 1), compositor e relógio falsos; mutação 37/37; suíte completa do runtime com Chrome real e `TESSEL_REQUIRE_RUNTIME=1`: **331/331, 0 ignorados** | — |
| Janela nativa: a manipulação rodou e os pixels não mudaram | contrato único `activation-spans.json` (página → host → harness: 19 + 1 + 13 testes); a prova de pixels reprova sem `drawn` no braço ligado, com span do fator no controle ou com interruptor diferente do pedido | repetir a prova nativa com esse veredito: não executado nesta entrega (5 GiB livres, abaixo do piso duro de 12 GiB do runner) |
| A/B no projeto real com pares comparáveis | N/A: teste unitário não tem GPU, disco, carga nem o relógio da janela | **Aberto.** Zero pares comparáveis por fator; latência **NÃO ACEITA**; BL157 segue aberto |

**Correção de uma afirmação anterior (§14).** "O resultado de pixels nativos sem evento `drawn` comprova compatibilidade da execução e não
ativação" vale para a **prova sintética de pixels**. Nas sessões ligadas do A/B nativo a ativação **está registrada** (tabela por run de cada
`analysis.md`): pré-upload `uploaded` 14 por run em 8/8 runs completos, warm draw `drawn` 4–6 por run em 8/8, e nenhum span do fator em nenhum dos
8 runs de controle de cada fator. O que faltava era a prova de pixels registrar o mesmo; agora registra.

**O que isto não muda:** a leitura do A/B (§13: benefício local na entrada da imagem com o pré-upload, nenhum benefício demonstrado no warm draw,
pausas grandes que persistem, `HANG` sem causa). Os testes unitários dizem que as decisões do runtime estão corretas, seguras e observáveis; não
dizem que reduzem a latência.

## 16. BL157.4: deadlock do WebKit no decodificador, laboratório de WebKit real e prova nativa (PR #41, após `4c16656`)

Pedido: seguir da validação para a **otimização**. A regra do repositório não muda: atribuir antes de otimizar, um fator por vez, nenhuma redução
de qualidade sem decisão de produto, opt-in primeiro, padrão só depois de A/B controlado; implementação sem prova segue **a validar**.
Duas perguntas estavam abertas (§13–§15): **(a)** por que a página deixa de responder em alguns runs (5 de 28 em `ab1`–`ab3`; o `HANG` de 11,1 s do §13)
e **(b)** o que paga a pausa na primeira entrada de crossfade. Uma sessão nativa leva ~70 s e roda sob carga externa que não controlamos; para ter dezenas
de runs por pergunta, esta rodada montou um laboratório de WebKit real, sem o editor (§16.1), e leva à janela só o que ele estabelece.

| Pergunta | Resultado | Estado |
|---|---|---|
| (a) página que para | **Mecanismo encontrado:** deadlock do WebKit em `ReadableStreamDefaultReader.releaseLock()` no decodificador do Spark (§16.4), com a mesma cadeia de pilha em 2 páginas do laboratório e em 1 da janela real. A chamada é removida **no build**, quando a variável está ligada | Correção **opt-in** (`TESSEL_RUNTIME_SPARK_RELEASE_LOCK_PATCH=1`, desligada por padrão; só muda o `runtime.js`). Na época das medições a remoção era o padrão do build. **O efeito dela na taxa de páginas presas NÃO está demonstrado:** laboratório intercalado 4 de 493 runs presos sem a correção, 0 de 493 com ela (p 0,062, sugestivo); a réplica pré-registrada (0 de 300 contra 0 de 300) e a janela real (1 de 36 contra 0 de 36) deram **NÃO REPRODUZIDO** (§16.4, §16.5) |
| (b) primeiro crossfade | O custo é **espera da GPU** (77–130 ms em `readPixels`; CPU do quadro 1–5 ms), paga no **primeiro crossfade do processo**; **sem causa atribuída** (§16.2). O `warm_draw` tira o custo da entrada e o **desloca** para o ponto do aviso; aquecer na abertura não resolve e o hitch reaparece 1–2 quadros depois da entrada (§16.3) | **Nada promovido.** `warm_draw` segue opt-in e desligado; BL157 segue aberto |

**A latência continua NÃO ACEITA.** Nenhum número desta seção é latência de display; o laboratório não é o editor; nada foi medido em máquina quieta.

### 16.1 O laboratório

`node runtime/scripts/perf/run.mjs --engine webkit …` carrega o `runtime.js` **de verdade** num `WKWebView` mínimo (`wk-host.swift`), sem o editor, sem Rust e
sem projeto de usuário na árvore: as nuvens do projeto real entram só por argumentos de linha de comando locais (`--real-cloud-a/-b/-c/-d`, `--real-ring`,
`--real-sparks`), viram links numa pasta temporária (nunca cópias) e **não são versionadas**; os relatórios citam só contagens, tamanhos e tempos. Cada run:

- numa **página nova** (`--fresh`), porque o primeiro crossfade de um processo é justamente o que se mede;
- com as variantes **intercaladas** (A, B, A, B…), para que a carga da máquina caia igual nas duas; a carga (`loadavg`) vai em cada linha e **nunca filtra**;
- 1280×720, qualidade `preview` (a do monitor com o playhead andando), 24 fps com ritmo de filme (`--pacing film`) e um `readPixels` 1×1 ao fim de cada quadro
  (`--drain read1x1`): o "quadro completo" inclui a espera da GPU;
- gravado em JSONL por run; **run com erro ou travado nunca é descartado**: o host desiste de fora (`--run-timeout`), porque uma página com a thread principal
  congelada não consegue se dar timeout, e a carga de assets tem limite próprio (`--load-timeout`).

Leitura das tabelas: **entrada** = quadro em que o crossfade começa (40; o aviso `frame.warm` sai 30 quadros antes, no 10); **hitch** = quadro completo > 62,5 ms
(1,5 quadro a 24 fps); medianas de quadro completo em ms. O laboratório orienta hipóteses; ele **não** substitui a janela: outro processo de GPU, sem Wry, sem host
Rust, sem prefetch real e com tempos que mudam com a máquina.

### 16.2 Onde o primeiro crossfade paga

| Rodada (runs por variante) | Carga (load1, mediana) | Quadro de entrada, mediana: `nowarm` / `current` / `warmdraw` | Runs com hitch: `current` → `warmdraw` |
|---|---|---|---|
| `cold` (6) | 9,6 | 66,5 / 44,5 / 27,5 | — |
| `prime` (12) | 10,5 / 10,7 | — / 40,0 (p90 149) / 27,5 (p90 37) | 5 de 12 → 0 de 12 |
| `ab30` (29 e 28, intercalados) | 20,4 / 20,8 | — / 94,0 (p90 152) / 32,5 (p90 103) | 21 de 29 → 13 de 28 |

- **O custo é espera da GPU, não CPU da página.** Em todo quadro lento a diferença está no `drain` (a espera do `readPixels`: 77–130 ms), enquanto `prepare`, `render` e
  `wall` (a página) ficam em 1–5 ms; nenhuma chamada WebGL da página passou de 4 ms (`--glcalls`). Coerente com o que `gpu_finish` indicou na janela (BL157.2: consistente com GPU
  pendente, não prova).
- **É o primeiro crossfade do processo.** Quatro cenas em sequência (`real-chain`, 4 runs, carga ≈ 36): o 1º crossfade custa 86,5 ms (46–166) e o 2º e o 3º, com nuvens que o
  processo nunca desenhou, 30,0 e 29,0 (máximos 36 e 29); com `warmdraw`, 30,0 / 27,0 / 27,5. Um **corte** para nuvem nova (`real-chain-cut`, 6 runs, carga ≈ 9) custa 16,0 /
  27,0 / 25,0 mesmo na primeira vez. As duas cadeias rodaram em cargas diferentes: a diferença entre corte e crossfade é um indício, não uma medida pareada.
- **O `warm_draw` não elimina o custo: muda onde ele cai.** Dos 13 runs de `ab30` com hitch, 9 o têm **antes** da entrada, nos quadros 10–14 (o aviso sai no 10); o quadro mais
  lento por run cai de 102 para 45,5 ms (mediana). A 10 de carga (`prime`) o ponto do aviso não teve hitch (0 de 12); a 20 teve em 9 de 28: o deslocamento depende da carga.
  Coerente com §13 (nenhum benefício demonstrado na janela).
- **Causas descartadas no laboratório**, uma por vez: compilação/link de programa na entrada, chave de PSO por função de blend esquecida (`--psokeys`), alocação do alvo da
  segunda camada, qualquer chamada WebGL da página ≥ 4 ms, CPU total do processo de GPU. **A causa continua sem atribuição.**

### 16.3 Aquecer na abertura não resolve

Se o custo é pago uma vez por processo, pagá-lo ocioso na abertura deveria bastar. Três variantes, 12 runs cada, intercaladas com `current` e `warmdraw` (carga ≈ 10–11):
`primeopen` (cenas aquecidas **e desenhadas** ao abrir, 1,5 s ocioso, e depois o aviso normal), `primeonly` (só o da abertura) e `primescene` (cenas aquecidas, sem draw).

| Variante | Quadro de entrada, mediana (p90) | Runs com hitch | Onde caem os hitches (quadro: 40 = entrada) |
|---|---|---|---|
| `current` | 40,0 (149) | 5 de 12 | 5 no quadro 40 |
| `primeonly` | 28,0 (55) | 6 de 12 | 40: 1, 41: 3, 42: 2 |
| `primeopen` | 27,0 (78) | 6 de 12 | 40: 2, 41: 1, 42: 4 |
| `primescene` | 26,5 (30) | 3 de 12 | 42: 3 |
| `warmdraw` | 27,5 (37) | **0 de 12** | — |

O quadro de entrada melhora (40 → 27 ms), mas **o hitch não some: 13 dos 16 hitches das três variantes caem 1–2 quadros depois da entrada**, com o mesmo perfil (`drain` de 80–130 ms,
CPU 1–5 ms). `warmdraw` tem 0 de 12 e `primeopen` 6 de 12 (p exato unilateral ≈ 0,007; `current` contra `warmdraw`, 5 de 12 contra 0, ≈ 0,019): **aquecer mais cedo, além do aviso normal, piora
em relação a aquecer só no aviso.** Hipóteses **não testadas**:
a ordenação aquecida na abertura envelhece em ~3 s e é refeita logo depois da entrada; o trabalho de GPU adiado vai para o primeiro quadro em que a segunda camada tem opacidade > 0;
o estado de energia da GPU. Nada disso foi implementado: o host já avisa só 2 s antes (`PREFETCH_SECONDS`) e nenhum aviso na abertura é proposto.

### 16.4 A página que para: um deadlock do WebKit no decodificador do Spark

**Sintoma.** Em alguns runs do laboratório um asset fica em `loading` para sempre: o decodificador (um worker, `jsContent` inline no `spark.module.js`) nunca responde. Foi isso que o
`real-load` expôs ao carregar os quatro assets do projeto real e esperar. Só decodificações **pequenas** ficaram presas: nos 12 runs presos do laboratório, 9 eram o arquivo de
2.912.361 B e 3 o de 78.760 B; nunca uma nuvem grande. Dois `sample` de páginas paradas do laboratório mostram a mesma cadeia, na **mesma thread** do worker (a janela real deu
uma terceira página, com a mesma cadeia e outro gatilho da coleta: §16.5):

```
ReadableStreamDefaultReader::releaseLock → (builtin) globalFuncMakeTypeError → ErrorInstance::create
  → LocalAllocator::allocateSlowCase → Heap::collectInMutatorThread → … MarkingConstraintSolver
  → JSReadableStreamDefaultReaderOwner::isReachableFromOpaqueRoots   (ou JSReadableStreamDefaultReader::visitOutputConstraints)
  → WTF::LockAlgorithm::lockSlow      (espera, sem fim, um lock que a mesma thread já segura)
```

Leitura da pilha: `releaseLock()` roda um builtin que aloca (cria um `TypeError`); a alocação dispara uma coleta **na própria thread**; a marcação visita o mesmo leitor e fica esperando, em
`lockSlow`, um lock que essa thread parece já segurar (a espera está aninhada na chamada), então não termina. Em uma das amostras a thread principal estava em
`JITWorklist::suspendAllThreads` → `lockSlow`: com uma parada geral pendente, **a página inteira congela**. É um defeito do WebKit exposto pela chamada do Spark (o runtime não a faz). O WebKit
não foi alterado nem recompilado para confirmar o mecanismo. O que está estabelecido é a **cadeia** (a mesma em 3 páginas paradas, em 2 ambientes); o **efeito da correção** na taxa de páginas
presas era o que os A/B deveriam mostrar, e **eles não o mostraram** (tabela abaixo e §16.5).

**Correção (opt-in).** Com `TESSEL_RUNTIME_SPARK_RELEASE_LOCK_PATCH=1`, `runtime/scripts/build.mjs` troca, na hora do build (plugin `onLoad` do esbuild), a única `reader.releaseLock();` do worker por `void 0;` (o bundle encolhe 14 bytes) e **recusa o build**
se o Spark não tiver exatamente uma dessas chamadas; sem a variável, o Spark entra no bundle como está. Quando as provas abaixo rodaram, a troca era o padrão do build; hoje ela é opt-in. O leitor deixa de ser solto, mas o worker não lê o fluxo de novo e o decode de um arquivo termina do mesmo jeito. O `releaseLock` da thread principal (`readStream`)
só roda com `stream`, que o runtime nunca passa (ele passa `fileBytes`); não foi tocado.

| Prova | Resultado | Limite |
|---|---|---|
| Pilhas de 3 páginas paradas (a cadeia acima) | cadeia completa, na mesma thread: 2 páginas do laboratório (uma com a thread principal em `suspendAllThreads`) e 1 da janela real (§16.5; duas amostras da mesma sessão, a 13,7 s e a 24,5 s) | mostram o mecanismo, não o efeito da correção |
| Laboratório, 120 runs sem a correção (não intercalado) | 8 de 120 runs presos (6,7%) | carga muito maior que a dos outros lotes (load1 mediana 35 contra 5–10); sem braço de controle; fica fora de qualquer teste abaixo |
| Laboratório, 3 lotes **intercalados** (28 + 45 + 120 = 193 runs por braço) | **4 de 193 sem a correção, 0 de 193 com ela** (2,1% → 0); p exato unilateral **0,0615** | quatro eventos: sugestivo, não conclusivo; motivou a réplica abaixo |
| **Réplica pré-registrada** do laboratório: 300 runs por braço, intercalados, mesma linha de comando do maior lote intercalado (120 por braço); regra e contagem fixadas antes de medir | **0 de 300 sem a correção, 0 de 300 com ela → NÃO REPRODUZIDO** (menos de 3 runs presos sem a correção: a réplica não diz nada sobre a correção) | a 2,1% esperavam-se ≈ 6 presos; 0 em 300 teria chance de ≈ 0,2% nessa taxa, mas ela vem de 4 eventos (IC 95% de 0,6% a 5,2%; a 0,6%, 0 em 300 acontece em ≈ 18% das vezes). A carga (load1 mediana 9,7) ficou na faixa dos lotes intercalados anteriores (5,4–10,1), então a diferença de taxa **não se explica por essa medida de carga** |
| **Total intercalado do laboratório** (193 + 300 = 493 runs por braço) | **4 de 493 sem a correção, 0 de 493 com ela**; p exato unilateral **0,062** | a réplica não acrescentou presos a nenhum braço; continua sugestivo, não conclusivo |
| Janela real, 36 pares de sessões (§16.5) | **1 de 36 sem a correção, 0 de 36 com ela → NÃO REPRODUZIDO** (p exato 0,5) | sem poder: a 1 em 36, 36 sessões por braço dão 3 ou mais presas em ≈ 8% das vezes |
| Testes (`runtime/test/spark-release-lock.test.mjs`) | remove só a chamada do worker e deixa a de `readStream`; Spark com 0 ou 2 chamadas faz o build falhar; o Spark instalado tem exatamente 1; o padrão deixa o Spark intacto e `TESSEL_RUNTIME_SPARK_RELEASE_LOCK_PATCH=1` o remove (a checagem do `dist` exige `TESSEL_REQUIRE_RUNTIME=1`) | não prova que a janela deixa de parar |
| Mutação (script local, fora do repositório) | 6 de 6 mutantes mortos | — |
| Suíte do runtime, Chrome real | 336 de 336 com o bundle corrigido | Chrome não é WKWebView |

**O que isto não afirma.** O `HANG` de 11,1 s do §13 **recuperou** e um decode preso nunca se recupera: não é atribuído a este defeito. As 5 de 28 páginas paradas de `ab1`–`ab3` (≈ 100% dos quadros
cancelados com a página ainda atendendo pedidos) são **compatíveis** com um asset que nunca termina de carregar; aquelas sessões não guardaram pilhas, então isso não prova nada.

### 16.5 Prova nativa: o editor real, a mesma build, dois `runtime.js`

`scripts/runtime-bundle-ab.py run` (contrato e regras, fixadas antes de medir, em [F063](features/viewport-asset-io.md#bl1574--o-deadlock-do-webkit-no-decodificador-o-laboratório-e-a-prova-nativa)):
o **mesmo binário** do editor (`4e334135…`) e o mesmo projeto real (25 s, 600 quadros; `0b456aec…`), um editor novo por sessão que toca o projeto duas vezes, 36 pares alternados
(72 sessões, 80 min; a ordem dentro do par inverte a cada par), com 6 processos de CPU ocupados ao lado de cada sessão para dar carga igual aos dois braços. Os braços diferem **só** no
`runtime.js` (`TESSEL_RUNTIME_DIR` aponta para uma pasta temporária por braço): sem a correção (`c7d27bd2…`, 4.281.059 B) e com ela (`78a0c8f4…`, 4.281.045 B: o bundle que
`pnpm build` reproduzia byte a byte quando a troca era o padrão do build; hoje ela exige `TESSEL_RUNTIME_SPARK_RELEASE_LOCK_PATCH=1`). O host registrou o que serviu a cada página: **72 de 72 sessões receberam o tamanho do bundle do seu braço**.

| Braço | Sessões | Presas | Falhas | Inválidas | Amostras com a assinatura do deadlock |
|---|---|---|---|---|---|
| sem a correção | 36 | **1** | 0 | 1 (a mesma) | 2 |
| com a correção | 36 | **0** | 0 | 0 | 0 |

**Veredito mecânico (regra fixada antes): NÃO REPRODUZIDO.** O braço sem a correção teve 1 sessão presa, menos que as 3 exigidas: a rodada **não diz nada sobre o efeito da correção**
(p exato unilateral 0,5, só diagnóstico). Dados e o resumo gerado: [`bundle-ab/`](viewport-real-latency-ab-results/bundle-ab/) (`analysis.md`, com o mesmo veredito: *NOT REPRODUCED: unpatched 1 of 36 sessions hung, patched 0 of 36*; a lista de hashes e o `manifest.json` não são mantidos no Git, ver a [proveniência](viewport-real-latency-ab-results/README.md)).

**A sessão presa (24, sem a correção).** Um asset (`splat`, o arquivo de 78.760 B) foi buscado a 1,37 s e **nunca saiu de `loading`**. O editor seguiu vivo: o relógio de reprodução do host
correu (1.133 e 1.132 ticks nos dois plays, 0 quadros pulados) e o host pediu 1.202 quadros, mas a página aceitou só 3 em 57 s (1.199 pedidos cancelados). O vigia (`/usr/bin/sample` no processo da
página depois de 10 s sem imagem) coletou duas amostras, a 13,7 s e a 24,5 s, **as duas com a assinatura completa do deadlock**: a thread do worker dentro de `releaseLock`, a coleta de
lixo disparada nela mesma e a espera em `lockSlow`. A thread principal da página estava ociosa no laço de eventos (nada em `suspendAllThreads`): desta vez só o decode parou, a página não
congelou. A cadeia é a do laboratório, com outro gatilho da coleta (`Structure::materializePropertyTable`, num `get_by_id` do builtin, no lugar de `makeTypeError`):

```
ReadableStreamDefaultReader::releaseLock → InternalReadableStreamDefaultReader::releaseLock
  → invokeReadableStreamDefaultReaderFunction → JSC::call → (builtin) get_by_id_direct
  → Structure::materializePropertyTable → Heap::decrementDeferralDepthAndGCIfNeededSlow → Heap::collectInMutatorThread
  → … MarkingConstraintSolver → JSReadableStreamDefaultReader::visitOutputConstraints
  → WTF::LockAlgorithm::lockSlow      (espera, sem fim, um lock que a mesma thread já segura)
```

A cadeia completa, só com nomes de símbolos, está em `bundle-ab/deadlock-chain.txt`; as duas amostras dão a mesma. **O que isto mostra:** o defeito acontece **no editor real**, com a mesma cadeia, e não é
um artefato do laboratório. **O que não mostra:** que a correção o evita; 1 sessão presa em 36 não dá poder para isso (a 1 em 36, 36 sessões por braço dão 3 ou mais presas em ≈ 8% das vezes).

**Como foram os plays** (o estado do editor sob carga; **não é o que este experimento decide**: as sessões dividem uma máquina, não um relógio):

| Braço | Plays | Maior espera por play: p50 / p95 / máx (ms) | Esperas > 62,5 ms | > 100 ms | Plays com espera > 100 ms |
|---|---|---|---|---|---|
| sem a correção | 72 (70 medidos¹) | 68,7 / 116,8 / 334,8 | 356 | 22 | 9 |
| com a correção | 72 | 69,6 / 104,4 / 600,4 | 386 | 14 | 6 |

¹ Os 2 plays da sessão presa não têm imagens para medir; contam como presos, não como espera.

- **Todas as 72 sessões rodaram carregadas** (processos externos ocupando em mediana 8,7 núcleos por sessão, de 7,0 a 9,8; `load1` de pico: mediana 18,4, de 9,2 a 112,9). A carga subiu a 50–113 nas
  sessões 20 e 32–40, por outros processos da máquina. **Os três maiores atrasos** (601 ms com a correção, 335 ms sem, 201 ms com) caíram nessas sessões (pico de `load1` de 94–113); fora das 10 sessões
  com pico acima de 50, o maior atraso de um play é 159 ms sem a correção e 100 ms com ela.
- **Os pontos de espera são os mesmos nos dois braços**: o início de cada vídeo (`video:N`), o quadro do aviso de pré-carga (`prefetch:N`) e, em menos plays, os cortes (`cut:N`); cada braço tem os mesmos
  14 locais com esperas > 62,5 ms. Nada nesta tabela distingue os braços.
- **Anomalias registradas, não filtradas.** A sessão 11 (sem a correção) deixou 2 processos de GPU vivos depois do editor: o runner os relata e **nunca os mata**; já tinham saído quando a rodada foi
  analisada. Por isso `analyze` sai com código 1 (`all_ran: false`) embora a sessão não tenha parado nem falhado. A sessão 24 está marcada inválida (o aquecimento da cena nunca rodou, porque a página parou antes) e **conta como presa**.
- **O runner foi estendido enquanto a rodada corria.** Quem mediu foi a revisão `6b1cab41…`; depois entraram a conferência do bundle servido e o resumo das esperas por play, que são só código de **análise**
  (regras, ordem, braços e coleta não mudaram), e a análise foi refeita dos registros salvos na época (a lista de SHA-256 dos registros não é mantida no Git; o `analyze` de hoje recusa um lote sem `batch_id` e leva uma validade mais estrita, então estes números não se reproduzem com ele: BL157.13). A revisão commitada é a final. O `manifest.json` da rodada (também fora do Git) registrou o commit do código como
  `b153f257b14a` (o commit da correção, reescrito depois só na mensagem, com a mesma árvore: hoje `e4e5aa2`) com 9 arquivos sujos (diff por SHA-256 no manifesto): os scripts, os testes e as docs desta
  entrega, ainda não commitados na época.
- **Limites:** uma máquina, um projeto, um WebKit (21624.5.1.11.3); os 72 registros brutos (143 MB) não são versionados; o runner não conferiu o tamanho servido contra o dos arquivos dos braços (essa conferência não existia quando a rodada
  começou), então a comparação foi feita depois, à mão: os tamanhos servidos (4.281.059 B e 4.281.045 B) são os dos dois arquivos.

### 16.6 Limites e o que fica aberto

- **Latência NÃO ACEITA; BL157 segue aberto.** O que cai aqui é uma classe de falha (página que não termina), não a pausa de 60–160 ms do corte e do primeiro crossfade.
- **A remoção do `releaseLock` é opt-in (`TESSEL_RUNTIME_SPARK_RELEASE_LOCK_PATCH=1`, desligada por padrão) e o efeito dela não está demonstrado** (§16.4, §16.5: NÃO REPRODUZIDO, 1 de 36 sessões presas sem a correção e 0 de 36 com ela). O que a sustenta é o mecanismo (a mesma cadeia em 3 páginas paradas de 2 ambientes)
  e o custo praticamente nulo (a chamada removida roda depois do último pedaço, quando o fluxo já acabou, e o decode termina do mesmo jeito). Torná-la padrão exige uma prova com poder; até lá fica desligada. BL157.4 fica **Parcial**, não validado.
- **Como obter uma prova com poder, se o custo valer.** A taxa vista na janela (1 de 36 sessões, 2,8%) pede ≈ 190 sessões por braço (≈ 7 h) para ter 90% de chance de 3 ou mais presas sem a correção; a taxa do laboratório
  somada (4 de 493 runs, 0,8%) pede ≈ 655 por braço (≈ 24 h). Uma alternativa mais barata, **não iniciada**: um reprodutor sintético no laboratório (um worker que lê um fluxo até o fim e chama `releaseLock()`
  enquanto outro código aloca em laço, para forçar a coleta no momento certo), com a mesma regra fixada antes.
- A causa da espera de GPU na primeira entrada segue **sem atribuição**; o `warm_draw` segue **opt-in** (desloca o custo, não o elimina) e promovê-lo a padrão exige A/B nativo pareado em máquina quieta
  (`scripts/first-use-ab.py full --factor warm_draw --pairs N`, que existe; não foi repetido nesta entrega) e uma decisão deliberada.
- A prova de **pixels** nativa do `warm_draw` com o veredito de ativação (`first-use-ab.py pixels --factor warm_draw`) e a correlação dos fetches de `frame_sequence` continuam pendentes (F063).
- As três hipóteses de §16.3 **não foram testadas**; testá-las precisa de instrumentação do processo de GPU (captura de quadro Metal), que o laboratório não tem.
- A cadência do relógio de reprodução (§16.7, BL157.6): a consulta por prazo (`TESSEL_PLAYBACK_POLL=deadline`) foi entregue **opt-in**, com o padrão `fixed`; o efeito no editor real **não foi medido** (o A/B nativo pareado não rodou: falta disco) e a pausa de 60–160 ms não é o alvo dela.
- **Gates do head final desta entrega.** Nada de Rust mudou nesta rodada; o único Rust novo desde `b53775e` é o teste do host do §15, e os gates abaixo rodaram nele (fecham a pendência do §15):
  `cargo fmt --all -- --check` 0; `./scripts/cargo check --workspace --locked` 0; `clippy --workspace --all-targets --locked -- -D warnings` 0; `test --workspace --locked` 0 (114 resultados:
  **2.239 passaram, 0 falharam, 13 ignorados**); `build -p tessel-editor --locked` 0; `cargo tree -p tessel-editor-core` sem `gpui` nem `wry`; `TESSEL_REQUIRE_RUNTIME=1 pnpm test` em `runtime/`
  **336 de 336** (bundle real e Chrome real); `pnpm build` reproduzia o bundle byte a byte (`78a0c8f4…`, com a troca então padrão do build); os 21 specs Python de `scripts/` passam (inclui os 51 e os 38 novos); mutação dos dois scripts novos:
  39 de 39 mortos cada. **`pnpm tsc`: N/A** — o repositório não tem TypeScript nem `tsc`.

### 16.7 BL157.6: a cadência do relógio de reprodução e o agendamento da consulta

Ao ler as esperas das 72 sessões do §16.5 apareceu um padrão que não é pausa: a mediana do intervalo entre imagens entregues ao monitor é **44,0 ms**, não 41,7 ms (24 fps), e 13% das entregas chegam
com menos de 30 ms de intervalo. `scripts/playback-cadence.py` (só lê; BL157.6) lê o traço do host (`tick` e `play_clock_started`) dos registros salvos e explica o padrão. Não é aceite de nada:
descreve a cadência, não uma pausa.

| Grupo | Sessões | Plays | Trocas de quadro | Consultas do relógio por troca de quadro | Intervalo da consulta (ms), média / p95 / p99 | Imagens entregues (ms), p50 / p95 / máx | Entregas < 30 ms | Esperas > 62,5 ms | Quadros pulados |
|---|---|---|---|---|---|---|---|---|---|
| todas | 72 | 144 | 86.098 | 1 consulta: 13,1%; 2: 86,9% | 22,29 / 23,07 / 28,51 | 44,0 / 52,4 / 600,4 | 13,0% | 0,88% | 28 |
| sem a correção | 36 | 72 | 43.058 | 1: 13,0%; 2: 87,0% | 22,28 / 23,07 / 28,24 | 44,0 / 52,1 / 334,8 | 12,95% | 0,85% | 12 |
| com a correção | 36 | 72 | 43.040 | 1: 13,1%; 2: 86,9% | 22,30 / 23,07 / 28,79 | 44,0 / 52,7 / 600,4 | 13,0% | 0,9% | 16 |

**Mecanismo (lido do traço e reproduzido por um modelo de uma linha).** `playback.rs` consulta o relógio `TICKS_PER_FRAME` = 2 vezes por quadro (a cada `1/(fps·2)` = 20,83 ms) e rearma o timer
**depois** de cada consulta (`armed = Instant::now(); timer(interval).await; tick`); o timer chega em média 1,4 ms atrasado, então o intervalo real é 22,29 ms. Um quadro de 41,67 ms equivale a
1,869 consultas: 87% das trocas de quadro são vistas só na 2ª consulta (≈ 44 ms depois da anterior) e 13% na 1ª (≈ 22 ms). O modelo (`41,667 / 22,294`) prevê 13,1% e 86,9%, **o que o traço mostra**, e é igual nos dois braços
(logo, não é do bundle). O efeito é um **micro-tranco regular**: a cada ≈ 8 quadros, um sai a ≈ 22 ms em vez de ≈ 42 ms; a média continua certa (41,6 ms entregues em média, contra 41,67 de referência).
O traço do host tem 28 quadros pulados em 86.098 trocas (consultas em que o relógio avançou mais de um quadro).

**A mudança (opt-in; padrão `fixed`).** `TESSEL_PLAYBACK_POLL=deadline` arma a próxima consulta para a próxima fronteira de quadro **mais 1 ms de folga**, descontado o tempo desde que o relógio foi lido e nunca mais que
meio quadro (a consulta continua a acontecer ao menos 2 vezes por quadro, então um quadro que atrasa, um `seek` ou um relógio que se reestima ainda são vistos em meio quadro); sem leitura, com o relógio parado ou em pausa, ou com uma
taxa inválida, espera meio quadro como `fixed`. A conta é a função pura `playback_poll::next_poll` e o laço de `playback.rs` a usa; contrato, testes e superfícies em
[F063 §BL157.6](features/viewport-asset-io.md#bl1576--a-cadência-do-relógio-de-reprodução-e-o-agendamento-da-consulta) e [F018](features/playback.md). O **modelo** (que reproduz primeiro as medidas nativas do `fixed`) prevê para o `deadline`
a 24 fps: 0% das imagens a menos de 30 ms da anterior, atraso médio de observação de 2,46 ms (guarda de 1 ms + atraso do timer, ≈ 1,46 ms) contra 11,15 ms, e 2,00 consultas por troca contra 1,87.
**É um modelo**: ele assume um relógio perfeito e um timer com atraso constante, e não diz nada sobre o editor real.

**A regra do veredito foi fixada antes de qualquer sessão** em [`poll-ab/rule.txt`](viewport-real-latency-ab-results/poll-ab/rule.txt), e `scripts/playback-poll-ab.py run` se recusa a começar (BLOCKED, saída 2) se o arquivo não está
commitado e inalterado, gravando o hash e o commit dele no `manifest.json`. O A/B é pareado: 12 pares (24 sessões, cada uma um editor novo que toca o projeto duas vezes), o **mesmo binário** nos dois braços, que diferem só em
`TESSEL_PLAYBACK_POLL`, na ordem fixed/deadline, deadline/fixed, …; cada play é conferido contra o `play_clock_started.poll` que o próprio host gravou. Resultados possíveis, todos mecânicos: INCONCLUSIVO (menos de 10 pares
contados), CONTROLE NÃO REPRODUZIDO (menos de 8% das imagens do `fixed` a menos de 30 ms da anterior), REGRESSÃO (imagens > 62,5 ms 0,25 ponto percentual acima; imagens > 100 ms acima de 1,5× as do `fixed` + 3; quadros pulados por
play 0,15 acima; mais sessões presas), SEM GANHO (mais de 3% das imagens < 30 ms; atraso médio acima de 4 ms ou menos de 5 ms abaixo do `fixed`; consultas por troca fora de 1,9–2,15; atraso menor em menos de 90% dos pares) e ADOTAR
(nenhum dos anteriores: **permite** tornar `deadline` o padrão, **não é aceite do usuário nem de BL157**). As marcas são diagnósticas, tiradas do ruído das 72 sessões (imagens > 62,5 ms em 0,86–0,90%, erro padrão ≈ 0,11
ponto percentual para 12 sessões por braço), e não são limites aprovados por ninguém. Carga e ocupação são registradas, nunca filtradas; nenhuma sessão é descartada.

**Estado: o A/B nativo NÃO foi rodado.** O runner exige ≥ 16 GiB livres e a máquina de desenvolvimento estava abaixo disso durante todo o trabalho (5–12 GiB livres em `/tmp`, medidos entre as etapas), então nenhuma sessão nativa foi
tocada: **não há medida do `deadline` no editor real, o padrão continua `fixed` e BL157.6 segue aberto**. O runner (104 testes e mutação 150/150 na rodada; hoje 86 mais 10 da regra, depois da revisão thermo da PR #41), o leitor (74 testes, mutação 118/118), a função pura (34 testes, mutação 40/41, o
sobrevivente é equivalente) e o laço (10 testes determinísticos, mutação 19/19) têm prova; o que **não** tem prova é o efeito na janela real: se `deadline` entrega mesmo as imagens a ≈ 42 ms, se o atraso de observação cai de ≈ 11 ms para
≈ 2,5 ms e se isso não piora o resto. Para rodar: `./scripts/runtime setup && ./scripts/cargo build -p tessel-editor --locked --release`, depois
`python3 scripts/playback-poll-ab.py run --binary target/release/tessel-editor --project <projeto.tessel.json> --out <dir> --pairs 12 --report-dir docs/viewport-real-latency-ab-results/poll-ab` (≈ 35 min; janela e áudio reais).

**Fumaça informal no editor real (não é o A/B).** Quatro sessões (duas por braço) do binário **debug**, com o projeto real e a máquina com carga média de 9 a 22, tocaram o projeto até o fim nos dois modos, sem erro e com o braço confirmado pelo host: o laço funciona na janela real. Os números **não** são o efeito: o timer chegou ≈ 20 ms atrasado (no release, 1,46 ms), as consultas por troca foram 1,00–1,01 (`fixed`) e 1,20–1,36 (`deadline`) em vez de 1,87 e 2,00, e pela regra fixada esse regime daria CONTROLE NÃO REPRODUZIDO. Delas saiu um achado, **BL157.7** (aberto, custo não medido): cada consulta termina em `cx.notify()`, tenha o quadro trocado ou não. Detalhes em [F063 §BL157.6](features/viewport-asset-io.md#fumaça-no-editor-real-informal-não-é-o-ab).

**Limites desta etapa.** Mesmo com um veredito ADOTAR, o ganho é de regularidade: **não** reduz as pausas de 60–160 ms do corte e do primeiro crossfade, que são o que o usuário não aceitou (BL157 segue aberto). A defasagem
áudio-imagem muda em até meia consulta (≈ 11 ms) ao mudar o agendamento, e **ninguém ouviu** o resultado (`heard:false`, F018). As tabelas das 72 sessões não foram mantidas no Git; `scripts/playback-cadence.py` as regenera sobre os registros brutos ([proveniência](viewport-real-latency-ab-results/README.md)).

**Gates do head desta entrega (BL157.6).** `cargo fmt --all -- --check` 0; `./scripts/cargo check --workspace --locked` 0; `clippy --workspace --all-targets --locked -- -D warnings` 0; `test --workspace --locked` 0 (114 resultados: **2.283 passaram, 0 falharam, 13 ignorados**, entre eles os 34 testes de `playback_poll_tests` e os 10 de `playback_tests`); `build -p tessel-editor --locked` 0; `cargo tree -p tessel-editor-core` sem `gpui` nem `wry`; os 22 specs Python de `scripts/` passam (610 testes). `TESSEL_REQUIRE_RUNTIME=1 pnpm test` em `runtime/` (o runtime não foi tocado): **335 de 336 na primeira execução completa**; a falha é `video B2: a frame superseded while it seeks is cancelled and the next one is exact` (`runtime/test/video.test.mjs:189`, `frame.done` em vez de `frame.failed`), o **flaky pré-existente BL015.15** (mesmo teste, linha e mensagem; reproduz em `main`); rodando só esse arquivo (`node --test test/video.test.mjs`), **7 de 7**. `pnpm build` reproduz o bundle byte a byte (`78a0c8f4…`). **`pnpm tsc`: N/A** — o repositório não tem TypeScript nem `tsc`.
