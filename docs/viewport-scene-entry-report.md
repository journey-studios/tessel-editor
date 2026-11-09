# Entrada de cena: evidência da PR #34 (BL157, BL158)

Plano e achados: [viewport-scene-entry-plan.md](viewport-scene-entry-plan.md). Contrato: [features/viewport.md](features/viewport.md).
Status e atribuição: só em [backlog.md](backlog.md). Dados brutos: [viewport-scene-entry-results/](viewport-scene-entry-results/)
(`ab.json` com cada quadro aceito de cada execução, `ab.log` com as checagens).

**Nenhum número de `f54d42e` (a primeira versão da PR) é reutilizado aqui.** Aquelas medidas vinham de um harness local que nunca foi
versionado (`play_tour.py`, `mkrt.py`, `phase-runtime.py`), de um runtime reconstruído à mão e de código que a revisão provou errado em
quatro pontos; nada disso vale para o código corrigido. Tudo abaixo foi refeito com código e harness do repositório.

## Estado

| Item | Estado | Por quê |
|---|---|---|
| BL157, correção (orçamento, linhagem, donos, cancelamento, contexto perdido) | validado em Chrome real (SwiftShader) | 6 dos 6 testes novos reprovam o runtime anterior; suíte do runtime completa sem SKIP |
| BL157, pixel igual com e sem aquecimento | validado na janela real (GPU) | 4 frames × (com/sem) × (quente/frio), `render.frame` idêntico byte a byte; o monitor mostra a mesma imagem depois da pausa |
| BL157, efeito na entrada de cena | medido num projeto **sintético** (5 cenas de 300 000 splats), mesma build, com e sem `frame.warm` | entrada mediana 32,9 → 2,2 ms (16 entradas por lado) |
| BL157, aceite "entrada < 60 ms no projeto de amostra" | **A validar** | o projeto de amostra tem mídia privada e não foi repetido; no sintético os dois lados ficam abaixo de 60 ms (frio: máx 57,5), então o limite não discrimina: o ganho é relativo |
| BL158, nova tentativa e desistência da carga | implementado e testado (core e janela, testes de GPUI) | a **causa** de a espera de mídia expirar segue aberta e não é reproduzida aqui |

## Bases e condições (leia antes dos números)

- **Código**: runtime em `512479b` e Rust em `7c42065` (a PR `t3/perf-scene-entry`; os dois têm o mesmo conteúdo dos commits `49f63d3` e
  `01b9b73` que existiram localmente antes de a mensagem ser reescrita, sem nenhuma diferença em `crates/` e `runtime/`).
- **Binário** `tessel-editor` release, **um só** para os dois lados: SHA-256 `db9f01a5fdb6b24c7de5fdda18605c1cb2034431d1550751415a10edf2729366`,
  copiado do cache antes de o `mbx` limpar o alvo. **Runtime** carregado (`TESSEL_RUNTIME_DIR` fixado pelo harness):
  `runtime/dist/runtime.js` SHA-256 `a703245a76d61189ff0b728bea1fddaf03f3e351e907c3f34c2905321c923b49`, reconstruído da fonte desta PR.
- **Harness**: `scripts/scene-warm-proof.py` no commit `e6bf44d` (o campo `git` do `ab.json`); 24 testes puros em
  `scripts/scene-warm-proof.spec.py`. Os commits posteriores só mexem em documentação.
- **Lado**: com aquecimento = padrão; sem = `TESSEL_VIEWPORT_WARM=0` (o host nunca envia `frame.warm`). O **runtime é o mesmo**.
  Uma execução "com" que não aqueceu (`stats.warmed == 0`) ou uma "sem" que aqueceu seria **inválida**: as 8 foram válidas
  (com: `warmed` 4 em cada uma; sem: 0).
- **Máquina**: Apple M3 Pro, 11 CPUs, 18 GiB, macOS 26.6.2, GPU real (WKWebView + Metal), uma janela própria por execução, os dois
  locks de GPU do banco (`/tmp/tessel-scene-gpu.lock` e `/tmp/tessel-video-bl140-bl144-review-gpu.lock`) tomados durante todo o harness.
  **Mas a máquina não estava ociosa**: a carga de 1 min subiu de 20,8 para 35,8 ao longo das 8 execuções (outros agentes e processos
  do usuário). A ordem alternada (com, sem, sem, com, com, sem, sem, com) espalha essa deriva pelos dois lados; ela não a remove.
- **Fixture** (gerada pelo harness, apagada no fim; nada privado): 5 cenas de 5 s (72 quadros a 24 fps; 360 no total), cada uma com
  sua nuvem de **300 000 splats** aleatórios semeados (9,6 MB, bola de raio 1,6, 960×540), câmera em órbita diferente por cena.
  Cada execução é um editor novo (cache frio igual nos dois lados), toca o projeto inteiro uma vez. O traço do próprio host
  (`TESSEL_VIEWPORT_TRACE=1`) dá o relógio de cada imagem aceita; o harness lê o anel a cada 4 s (cabe 256 quadros, cerca de 10 s)
  e junta por id. A leitura periódica é o mesmo custo nos dois lados.

## 1. Provas de correção (o que reprovaria o código errado)

| Propriedade | Teste | Resultado |
|---|---|---|
| orçamento: `makeRoom(keep)`/`trim(keep)` nunca passam do limite nem soltam o protegido | `retention.test.mjs`, `scene-warm.test.mjs` | verde; reprova o runtime anterior |
| linhagem: aquecimento e quadro numeram igual, com camada coberta que divide o asset | `scene-warm.test.mjs` | verde; reprova o anterior |
| dono: o aquecimento não leva a malha da cena na tela, que mantém a ordem | `scene-warm.test.mjs` | verde; reprova o anterior |
| cancelamento: o quadro que espera a ordenação é substituído sem esperar 1 s | `scene-warm.test.mjs` | verde; reprova o anterior |
| contexto perdido durante a ordenação: nada fica para trás e a ordenação antiga não desfaz um aquecimento novo | `scene-warm.test.mjs` | verde; reprova o anterior |
| pixel: hash exato igual a um runtime que nunca aqueceu | `scene-warm.test.mjs` (antes e depois) | verde |
| sessão: `cancel_timed_out`, `cancelled` ignorado, `set_warm(false)`, cortes sem aquecimento | `crates/core/src/session/tests.rs` | verde (63 testes de sessão) |
| retry: um dono, dirigido e comum, sem retry órfão | `viewport_host_tests.rs`, `scene_lifecycle_tests.rs` | verde (82 testes do host) |

**Discriminação.** Com `compositor.js` e `retention.js` do head original da PR (f54d42e), reconstruindo `runtime/dist` e rodando
`retention.test.mjs` + `scene-warm.test.mjs`: 14 passam e 6 reprovam (o teste de `LruCache` com `keep` e os testes 16 a 20 de
`scene-warm`: orçamento, cancelamento, linhagem, dono e contexto perdido). Os 4 testes que passam nas duas versões (12 a 15) cobrem
comportamento já correto antes (anúncio do aquecimento, cena não aquecida, aquecimento que não libera nem carrega, slot do quadro
anterior); eles protegem contra regressão e não provam correção nova. O runtime atual foi restaurado e reconstruído (hash do bundle
`a703245a76d6…`, o mesmo de antes do teste).

**Suítes no head de código (e6bf44d; os commits seguintes só mudam docs e um limite de teste).**

| Suíte | Resultado |
|---|---|
| `./scripts/runtime test` com `TESSEL_REQUIRE_RUNTIME=1`, Chrome real (SwiftShader) | 212 testes, 212 passam, 0 falham, 0 SKIP (carga da máquina 41; um teste de cancelamento usava limite absoluto de 600 ms e falhou sob carga; passou a comparar com uma linha de base medida na própria execução) |
| `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked --no-fail-fast` | 2018 passam, 0 falham, 12 ignorados |
| clippy workspace/all-targets `-D warnings`, `cargo fmt --check`, `cargo check --workspace --locked` | limpos |
| build release do desktop | gerado e usado no A/B (binário `4aaf7f8`) |
| `cargo tree -p tessel-editor-core` | sem gpui nem wry |
| harness `scripts/scene-warm-proof.spec.py` | 24 testes passam |

Limite: o build debug do desktop não foi repetido (o disco livre estava em 18 GiB, abaixo da reserva de 16 GiB para carga pesada nova
somada ao piso de 12 GiB); os testes de crate já compilam o desktop em perfil de teste.

## 2. Custo da entrada de cena, mesma build, com e sem o aquecimento

Latência da primeira imagem de cada corte (`submit_to_done_ms` do traço do host: da admissão do pedido à aceitação da imagem, em ms;
inclui a espera da ordenação, a leitura de volta no runtime e a fila; **não** é fóton nem GPU). Entrada 0 não é uma entrada do play
(é a abertura do projeto) e não entra.

| # | lado | carga (1 min) | entrada 72 | 144 | 216 | 288 |
|---|---|---|---|---|---|---|
| 0 | com | 20,8 | 2,1 | 1,8 | 1,7 | 3,1 |
| 1 | sem | 26,4 | 20,3 | 57,5 | 16,1 | 30,2 |
| 2 | sem | 29,7 | 19,5 | 34,6 | 32,7 | 33,2 |
| 3 | com | 32,2 | 2,3 | 1,7 | 1,9 | 4,2 |
| 4 | com | 33,5 | 2,2 | 1,9 | 5,7 | 2,0 |
| 5 | sem | 34,6 | 34,7 | 37,5 | 39,4 | 19,0 |
| 6 | sem | 35,2 | 17,1 | 17,6 | 33,1 | 41,3 |
| 7 | com | 35,8 | 2,4 | 9,2 | 5,9 | 1,7 |

Por lado, 16 entradas cada: **com** mediana 2,2 ms, máximo 9,2 ms; **sem** mediana 32,9 ms, máximo 57,5 ms. Nenhuma entrada chegou a
60 ms em nenhum lado, então o aceite do backlog (< 60 ms) **não é o que separa os lados neste fixture**; o que separa é a distância:
a pior entrada com aquecimento (9,2) é menor que a melhor sem (16,1). Cada execução tem todas as entradas listadas; nada foi
promediado entre cortes.

Quadros ao longo do play (mesmas 8 execuções, intervalos entre imagens aceitas, limite 62,5 ms = 1,5 quadro a 24 fps):

| # | lado | intervalos > 62,5 ms | maior (ms) | excesso acima de 41,7 ms (soma, ms) | quadros pulados / maior salto |
|---|---|---|---|---|---|
| 0 | com | 0 | 52,9 | 0 | 0 / 1 |
| 3 | com | 0 | 56,6 | 0 | 0 / 1 |
| 4 | com | 0 | 57,0 | 0 | 0 / 1 |
| 7 | com | 1 | 64,4 | 22,7 | 0 / 1 |
| 1 | sem | 2 | 100,2 | 91,3 | 0 / 1 |
| 2 | sem | 4 | 77,6 | 122,5 | 0 / 1 |
| 5 | sem | 4 | 82,1 | 137,9 | 0 / 1 |
| 6 | sem | 2 | 81,9 | 74,5 | 0 / 1 |

Os intervalos longos sem aquecimento caem **nos quadros de corte** (143→144, 215→216, 287→288 em quase todas as execuções); o único
com aquecimento está em 279→280 (64,4 ms), fora de corte. A **partida** do relógio de reprodução (do último quadro da abertura ao
primeiro tique) levou 1,5–2,8 s nos dois lados (com 1,5/1,7/2,5/2,8 s; sem 1,6/1,7/2,0/2,1 s): não é efeito do aquecimento e não é
uma entrada.

## 3. Memória: mediana e pico, sem promessa sobre o pico

RSS do editor mais os processos do WebKit que não existiam antes de ele subir, uma vez por segundo (não é a pegada física, não é
memória de GPU, e o WebKit de outro programa que nascesse durante a execução seria contado; os locks mantêm o banco sozinho, o
navegador do usuário não é controlado).

| # | lado | mediana (MiB) | pico (MiB) |
|---|---|---|---|
| 0 | com | 463,2 | 508,3 |
| 1 | sem | 425,8 | 472,6 |
| 2 | sem | 427,0 | 523,8 |
| 3 | com | 387,2 | 434,2 |
| 4 | com | 423,8 | 458,5 |
| 5 | sem | 430,0 | 467,3 |
| 6 | sem | 422,3 | 469,9 |
| 7 | com | 395,2 | 427,9 |

Medianas por execução: com 387–463 (mediana 410), sem 422–430 (mediana 427); picos: com 428–508 (mediana 446), sem 467–524 (mediana
471). **A dispersão entre execuções do mesmo lado (76 MiB nas medianas com aquecimento) é maior que a diferença entre os lados**, e o
maior pico de cada lado (508 com, 524 sem) está dentro dela. Conclusão que os dados sustentam: neste fixture não se observa aumento
de memória com o aquecimento; **não se afirma que o pico não cresce** (o orçamento de dois estágios é a garantia de projeto, não
esta medida) e não se afirma queda.

## 4. Pixels: o aquecimento não muda a imagem

Quatro sessões da mesma build com `TESSEL_VIEWPORT_INSPECT=1`, uma por (lado, ordem): **quente** toca a cena anterior até o corte (o
prefetch carrega, o aquecimento ordena, o relógio cruza o corte) e pausa; **frio** salta direto ao quadro. Em cada uma, os quadros
77, 112, 149 e 293 (corte 72 + 5 e + 40, corte 144 + 5, corte 288 + 5) são pedidos por `render.frame` com o tamanho do monitor:

| quadro | hash (todas as 4 sessões) | média RGB |
|---|---|---|
| 77 | `3630406963a018b7` | 55,2 / 55,2 / 63,8 |
| 112 | `9456438e2ba5f60b` | 55,2 / 55,2 / 63,8 |
| 149 | `e08356f1f781000f` | 58,1 / 55,2 / 64,4 |
| 293 | `51d6dbeba8cf06c6` | 65,7 / 55,5 / 64,0 |

Os quatro hashes são **idênticos nas quatro sessões**; são diferentes entre si (controle: uma imagem travada ou em branco não
passaria); todos foram lidos de volta (`inspected`); depois da pausa o monitor mostra o hash do render explícito. Sessão com
aquecimento e quente: `stats.warmed` 5; as outras três: 0 (o lado "sem" nunca aqueceu, o "com" frio nunca teve um corte a aquecer).
**O que isto não prova**: que a imagem do fixture é sensível à ordem das splats (a sensibilidade é o que o teste de Chrome real prova,
com nuvens translúcidas grandes, e a nuvem do fixture aqui é densa e aleatória); e a imagem do *quadro do corte durante o play* (que
usa a ordem do aquecimento, desatualizada por desenho) não foi capturada, só a final depois da pausa.

## 5. Lacunas e limites (registradas, nenhuma fechada sem prova)

- **Aceite do backlog no projeto de amostra**: a mídia é privada; o harness antigo e as 100–290 ms de entrada daquele projeto não
  foram reproduzidos. O fixture sintético mostra o mecanismo e a direção, não o valor do projeto real. BL157 fica **A validar**.
- **Máquina com carga de 21–36 por outros processos** nas duas posições da comparação; não há comparação com a máquina quieta. A
  interferência de CPU/GPU entre a ordenação em segundo plano do aquecimento e os quadros (nove atrasos em seis execuções com
  aquecimento contra três em cinco sem, na primeira versão) **segue inconclusiva**: neste fixture não houve atraso por interferência
  (o pior intervalo com aquecimento é 64,4 ms, sem aquecimento 100,2 ms), mas quatro pares com carga variável não a excluem.
- **Quatro pares** são pouco para uma estimativa de variância; os valores por execução estão acima e no JSON, nenhuma média foi usada
  como veredito.
- **`flush`/`render_ms` são CPU**: nenhuma afirmação sobre tempo de GPU é feita; `submit_to_done_ms` não é latência até o fóton.
- **Cancelamento e perda de contexto na janela real**: provados em Chrome real (SwiftShader), não induzidos na janela nativa (não existe
  operação semântica para isso sem alterar o produto). O `context_loss_reloads_assets_and_asks_for_a_redraw` cobre a sessão.
- **Headless**: `frame.warm` e a nova tentativa são do monitor nativo; N/A justificado. Não se declara paridade entre o SwiftShader do
  headless e a GPU da janela.
- **BL158**: a nova tentativa e a desistência da carga existem e têm teste; **não** se mostrou que elas resolvem o `timeout` real de
  abertura (a causa, "12 s contra `LOAD_TIMEOUT_MS` de 30 s", segue aberta).
- **Memória**: RSS, não pegada nem GPU; ver a seção 3.

## 6. Como reproduzir

```sh
./scripts/runtime setup                    # runtime/dist da fonte, Chrome para os testes
./scripts/cargo build -p tessel-editor --locked --release   # copie target/release/tessel-editor antes do mbx limpar o alvo
python3 scripts/scene-warm-proof.spec.py   # lógica do harness, sem janela
python3 scripts/scene-warm-proof.py --binary /caminho/tessel-editor --pairs 4 --splats 300000 --cuts 5 --cut-frames 72 --out ab.json
```

O harness toma os dois locks (espera no máximo `--lock-wait` segundos e sai com 2), exige 16 GiB livres, usa um `HOME` isolado por
execução (nunca o editor do usuário), só requisições semânticas pelo socket da janela (`op`, `get`, `set`, `invoke`) e sai com 1 se
uma execução for inválida, uma checagem falhar ou um corte ficar sem imagem.
