# Latência real — experimento A/B do I/O de assets (BL157.1)

**A latência continua não aceita.** Nada aqui fecha o BL157 nem declara um limite aprovado: 62,5 ms e 100 ms são gatilhos
diagnósticos e 41,7 ms (24 fps) é só a referência de orçamento. Plano: [real-latency-ab-test-plan.md](migration/real-latency-ab-test-plan.md);
investigação anterior: [viewport-real-latency-investigation.md](viewport-real-latency-investigation.md); contrato do que foi
construído: [F058](features/viewport-asset-io.md); dados: [viewport-real-latency-ab-results/](viewport-real-latency-ab-results/).

## 1. Resultado em uma página

| Pergunta | Resposta | Base |
|---|---|---|
| O callback do protocolo roda na thread da janela e lê o arquivo ali? | **Sim, medido** (em `sync`, 29 leituras por sessão na thread da UI; em `async`, 0) | evento `asset_cb` com `on_ui_thread` e nome da thread |
| Essa leitura bloqueia a janela durante a pré-carga? | **Sim**: callback de 24,9–85,4 ms na UI (máximo por sessão) em todas as 6 sessões `sync` válidas (leituras de ~66 MB, 13–31 ms de leitura mais a cópia para o `NSData`) e tick atrasado em até 71,5 ms; em `async` o callback na UI é ≤ 0,12 ms e o atraso máximo do tick é ≤ 14,4 ms | tabelas §4 |
| Tirar essa leitura da thread da UI reduz as pausas entre imagens? | **Não demonstrado: inconclusivo, com tendência de nenhum efeito na cauda.** Gaps > 62,5 ms em sessões silenciosas: `sync` 28 (5 sessões, 10 plays), `async` 27 (5 sessões, 10 plays); > 100 ms: 1 contra 2. Em 4 pares comparáveis o efeito troca de sinal | §4.3 |
| A preparação (`plans_ahead`, `prefetch`, `sync`) é o custo? | **Refutado.** `plans_ahead` ≤ 0,07 ms, `prefetch` ≤ 4,3 ms (típico < 0,1 ms), `sync` ≤ 2,8 ms | §5 |
| O que sobra nas pausas com `async`? | Pausas em que a UI não estava bloqueada, o tick estava no horário e não houve leitura: o quadro espera 56–120 ms **fora do trabalho do runtime** (trânsito host→página→host), sem span que o explique; mais a entrada de clipes de imagem (primeiro desenho de 12–18 ms) | §5 |

**Decisão (regra do plano).** Os spans confirmam o mecanismo (leitura na thread da UI atrasa a janela), mas **não** mostram que ele
é a causa principal das pausas visíveis no monitor: a cauda das pausas não cai. Portanto o modo assíncrono **não é proposto como a
correção**; fica como experimento opt-in (`TESSEL_VIEWPORT_ASSET_IO=async`, padrão `sync`) com testes, e o próximo componente
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
| Fila cheia → `503` uma vez, contada, sem descarte silencioso; troca de projeto com leituras na fila e em voo → `410`, sem resposta dupla nem resultado velho; workers/fila/memória limitados (≤ 1 corpo por worker) | passa |
| Pixels do projeto **sintético** (5 cenas × 200 000 splats), 4 quadros, `sync`/`async` × quente/frio, `render.frame` | **idênticos byte a byte** nas 4 sessões; o monitor mostra a mesma imagem. Um `render.frame` explícito foi substituído por um pedido mais novo do próprio monitor (`cancelled`, em `sync` e em `async`) e foi repetido; o registro está em [pixels-synthetic.txt](viewport-real-latency-ab-results/pixels-synthetic.txt). A primeira rodada reprovou o veredito por isso e foi corrigida no runner (nova tentativa só para `cancelled`), não nos dados |
| Export | N/A: o export não passa pelo protocolo da WebView |

## 4. A/B no projeto real

**Condições.** Projeto real horizontal de 25 s (600 quadros, 24 fps, 23 fontes presentes, cortes lidos da timeline em 186/268/410,
clipes de imagem entram em 18/196/282/326/370/512/520; documento sha256 `e93363aecc4c0591…`). Binário release sha256
`ad16ad3670f4a58f…` (o mesmo nas 16 sessões e nos pixels), runtime sha256 `a5bc2ad06d1bdf9e…`, `TESSEL_GL=gpu`, dois locks de
GPU, uma sessão por vez, janela própria. Cada sessão: um editor novo, **primeiro play completo e play repetido com cache residente,
analisados separadamente**. Sem purge e sem afirmação de disco frio. Ordem alternada `sync/async, async/sync, sync/async,
async/sync`, **duas rodadas** de 4 pares (a segunda foi pedida por mim depois que a primeira teve uma sessão inválida; nenhuma
sessão foi descartada, e os números por rodada estão em [analysis-batch1.md](viewport-real-latency-ab-results/analysis-batch1.md) e
[analysis-batch2.md](viewport-real-latency-ab-results/analysis-batch2.md)). A máquina **não estava ociosa**: carga média 3–20 e CPU
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
| `sync` | 6 (5/1) | 24,9–85,4 | 15,1–71,5 | 164–258 / 204–261 | 507–1125 / 866–1695 |
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
tabela. Gráficos por par (intervalo entre imagens contra o quadro; sombreado = 2 s antes do corte; verde = corte) em
[charts/](viewport-real-latency-ab-results/charts/) (`gaps-pair*.svg`) e linha do tempo de spans de uma sessão `sync` e uma `async`
(`spans-session8-…`, `spans-session9-…`).

## 5. O que cada pausa estava fazendo (correlação, não causa)

Cada gap > 62,5 ms foi cruzado com os spans que o sobrepõem (sessões válidas, todos os braços):

| Grupo | Gaps | com callback na UI ≥ 20 ms | com decode/fetch/warm da página ≥ 10 ms | com trânsito do quadro ≥ 30 ms | sem parada de UI **nem** trabalho da página | em janela de pré-carga | em corte | em entrada de clipe de imagem | outros |
|---|---|---|---|---|---|---|---|---|---|
| `sync` silencioso | 28 | **10** | 12 | 5 | 16 | 12 | 1 | 12 | 3 |
| `async` silencioso | 27 | **0** | 5 | 6 | 22 | 9 | 3 | 11 | 4 |
| `sync` carregado | 11 | 2 | 2 | 3 | 9 | 3 | 2 | 5 | 1 |
| `async` carregado | 24 | 0 | 5 | 8 | 19 | 6 | 4 | 12 | 2 |

- **Confirmado:** em `sync`, 10 dos 28 gaps silenciosos coincidem com um callback de 20–85 ms na UI, todos em janelas de pré-carga,
  junto a tick atrasado de 14–71 ms. `async` zera esse subconjunto (0 em 51).
- **Mas o total não cai**, porque o resto das pausas é de outra origem: no `async`, 22 de 27 não têm parada de UI nem trabalho da
  página sobreposto e o tick estava no horário (≤ 14 ms). Os maiores (166, 139, 136 ms; `video:370`, `cut:410`, `cut:410`) têm
  `runtime_total` de 1–24 ms e **trânsito** (envio→aceite menos o total do runtime) de 92–120 ms, sem leitura, sem decode/fetch/warm da
  página e sem parada de UI sobrepostos nos dois `cut:410`. O quadro esperou **fora** do trabalho do runtime e **nenhum span meu
  explica onde**: pode ser a thread da página ocupada com trabalho não medido (GC, ordenação do Spark), a entrega do `evaluate` ou o
  processamento do IPC pela UI, ou o escalonamento dos processos WebKit sob a carga externa. **Causa não atribuída.**
- **Entradas de clipe de imagem** (quadros 18/196/282/326/370/512/520) concentram 11–12 gaps por grupo, nos dois modos, nos mesmos
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

## 7. Próximo componente (proposta; **não implementada**)

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

## 9. Reprodução

```text
./scripts/runtime setup && ./scripts/cargo build -p tessel-editor --locked --release
python3 scripts/real-latency-ab.py preflight --project <documento .tessel.json>
python3 scripts/real-latency-ab.py pixels  --binary target/release/tessel-editor --out <dir>
python3 scripts/real-latency-ab.py run     --binary target/release/tessel-editor --project <documento> --out <dir> --pairs 4
python3 scripts/real-latency-ab.py analyze --out <dir> --project <documento>
python3 scripts/real-latency-ab.spec.py
```

As sessões brutas (1,5 MB cada) ficam fora do Git; seus sha256 estão em
[sessions.sha256](viewport-real-latency-ab-results/sessions.sha256) (rodada 1 `ab/`, rodada 2 `ab2/`).
