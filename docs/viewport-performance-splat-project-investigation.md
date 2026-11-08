# Investigação de performance: projeto splat + vídeos

Investigação (só medição) pedida porque, com vários vídeos e cenas, a reprodução "não está super fluida" mesmo com o monitor
mostrando ~16 ms. **Nenhuma otimização foi implementada.** A instrumentação é local, não commitada
(`scripts/perf-investigation/`, `rust-instrumentation.patch`); nada neste relatório altera `docs/backlog.md`.

## Resumo executivo

1. **O número do cabeçalho (`Three.js · 960×540 · N ms`) é só o tempo de parede do JS** (`drawStarted → frame.done`).
   Não inclui a GPU depois do `flush` (aparece como espera na leitura de profundidade do Spark no quadro seguinte), a
   distância entre quadros, quadros perdidos nem o repaint da UI. Fluidez = quadros mostrados por segundo, não esse ms.
2. **Um quadro com uma cena SHARP (1,18 M gaussianas) custa ~52 ms com o padrão atual (MSAA 4×)** numa máquina quieta. O
   relógio de reprodução é 24 fps (41,7 ms), então só ~19 fps são mostrados. **Com MSAA 2× o mesmo quadro custa 14 ms e
   chega a 24 fps cheios.** O MSAA 4× sozinho responde por ~73 % do custo (medido). Os "16 ms" do usuário batem com
   MSAA ≤ 2 ou com um caso leve (um vídeo: 7 ms); veja a pergunta no fim.
3. **Crossfade (2 cenas) custa 106 ms (9,5 fps); 4 cenas, 179–234 ms (5–6 fps).** O custo escala por camada de splat. Com
   MSAA 2×: 74 ms; sem MSAA: 44 ms (21,7 fps).
4. **Em cada corte de cena há um engasgo de ~230–260 ms** (carga + decode do PLY de 66 MB no primeiro quadro que o usa;
   3 vezes no projeto). Não depende de MSAA.
5. **Vídeos (sequência de quadros PNG/JPEG) custam 4–8 ms por camada com proxy, ~17 ms sem proxy**, dominados por
   `fetch + createImageBitmap` e upload por quadro. Seis vídeos empilhados: 49 ms, 15,9 fps.
6. **UI GPUI não é o gargalo:** repaint de 3,5–4,5 ms de CPU, p95 ≤ 7 ms. A cadência de repaint cai (65 → 46 → 30 repaints/s)
   quando o monitor está pesado, mas o custo por repaint é constante.
7. **Ponte, core e apresentação são desprezíveis:** plano/serialização < 0,05 ms, trânsito da ponte 0,5–1,5 ms, apresentação
   +1–2 ms. A fila descarta pouco (14–17 quadros `dropped` em 25 s): os quadros "perdidos" são sobretudo os que o relógio avançou
   enquanto o render anterior ainda corria.

Maiores ganhos por esforço (detalhe na seção de candidatos): **(a) MSAA 2× no monitor** (config de uma linha; −73 % em um
take, −30 % em crossfade, validação visual pendente); **(b) pré-carga da próxima cena** (remove ~250 ms × 3);
**(c) não desenhar camadas totalmente cobertas** (hipótese, limite superior medido).

## Como foi medido

- Editor real (release, `target/release/tessel-editor` copiado para `/tmp/perf/bin`), HOME isolado por execução, runtime
  Three.js instrumentado por variante; só operações semânticas (`project.open`, `seek`, `playback.start/stop`,
  `set timeline.playhead`, `render.status`). A instância aberta do usuário não foi tocada.
- Reprodução usa o relógio do dispositivo de áudio (24 ticks/s). Trilhas de áudio mudas nas cópias. Quadro "perdido" =
  quadro que o relógio avançou e não foi mostrado.
- Cada quadro grava no runtime: fase de preparo, rig, `settle`, leitura de profundidade, sort (worker), draw, flush,
  `render_ms`, `interval_ms` (entre inícios de quadro). O host grava `submit_to_done`, `transit`, `submit_to_presented`,
  `done_gap`, repaints da UI (`perf_ui`). **Modo drain** (leitura 1×1 após cada camada) revela tempo de GPU; serializa CPU e
  GPU, então dá custo, não vazão.
- **A máquina não estava ociosa durante a maior parte do trabalho** (load average 20–50, builds de outros agentes). As tabelas
  usam as execuções `Q-*`, `V-*`, `F-*` feitas com load 3–17; as de load alto só são citadas como A/B pareado. Cada linha
  mostra o load. Ruído esperado: ±10 %.
- Projetos de amostra em `/tmp/perf/proj` (cópias; os originais e `scripts/build_project.py` não foram tocados). Variantes
  de sobreposição: `x2`/`x3`/`x4` (2/3/4 cenas sobrepostas), `vid1..6` (1–6 vídeos H.264 de 8 s, 1280×720, cortados do
  preview do usuário), `hmix1..4` (take A + vídeos), `v` (vertical).
- Reproduzir: `python3 scripts/perf-investigation/run.py {take|stack|scrub|load|idle_ui} --project ... --samples N`;
  `exp.py TAG...` imprime p50/p95/p99.

## Tabelas por cenário

p50/p95/p99 em ms. "fps" = quadros distintos mostrados por segundo de reprodução (24 = perfeito).

### 1. Reprodução contínua, load 3–14 (máquina quase quieta)

| Cenário | MSAA | load | render JS | intervalo | fps | settle (Spark) | leitura GPU | sort | decode | rig |
|---|---|---|---|---|---|---|---|---|---|---|
| Take único (A, 1 cena 1,18 M) | 4 (padrão) | 3 | 52/65/69 | 54/67/71 | 17–22 (≈19) | 51/64/68 | 43/56/60 | 6/6/9 | – | 0 |
| idem | 2 | 4 | 14/24/28 | 45/47/50 | 24,2 | 14/23/28 | 6/16/20 | 6/6/7 | – | 0 |
| idem | 0 | 7 | 14/16/18 | 45/49/51 | 24,2 | 14/16/17 | 6/8/9 | 6/6/8 | – | 0 |
| Crossfade sustentado (2 cenas, `x2`) | 4 | 4 | 106/123/127 | 108/124/134 | 9,5 | 98/115/119 | 83/99/102 | 11/13/15 | 7/8/10 | 7/8/11 |
| idem | 2 | 5 | 74/84/88 | 77/87/93 | 13,5 | 65/75/79 | 48/57/61 | 12/14/16 | 7/8/11 | 7/9/11 |
| idem | 0 | 14 | 44/50/53 | 46/52/54 | 22,0 | 37/43/45 | 22/27/28 | 11/12/13 | 7/7/8 | 7/8/8 |
| 4 cenas sobrepostas (`x4`) | 4 | 10 | 179/248/251 | 192/251/252 | 5,2–6,7 | 169/231/233 | 144/197/199 | 17/24/27 | 8/17/19 | 8/17/19 |
| idem | 0 | 9 | 76/105/115 | 80/110/116 | 12,3 | 67/89/99 | 43/55/66 | 17/24/28 | 8/15/17 | 8/16/18 |

### 2. Projeto inteiro (0–25 s, 600 quadros), load 5–6

| MSAA | quadros mostrados | fps | intervalos > 62 ms | render (1 cena) | render (2 cenas, 14 qd) | submit→apresentado p50/p99 |
|---|---|---|---|---|---|---|
| 4 | 458 de 597 (77 %) | 18,4 | 66 | 48/65/71 | 101/159/159 | 51/106 |
| 2 | 563 (94 %) | 22,7 | 20 | 19/46/50 | 78/96/96 | 22/83 |
| 0 | 580 (97 %) | 23,3 | 6 | 15/18/25 | 52/59/63 | 16/58 |

Os poucos intervalos de 228–259 ms que sobram com MSAA 0 são os cortes de cena (item "Carga", abaixo).

### 3. Vídeos (quadros PNG/JPEG decodificados; 960×540; MSAA 4), load 9–18

| Cenário | render JS | intervalo | fps | preparo | draw |
|---|---|---|---|---|---|
| 1 vídeo (proxy auto) | 7/16/33 | 43/45/70 | 22,9 | 3/10/16 | 4/7/12 |
| 2 vídeos | 14/23/40 | 44/46/121 | 22,5 | – | – |
| 4 vídeos | 31/60/93 | 44/64/164 | 21,6 | 15/40/68 | 15/28/38 |
| 6 vídeos | 49/109/200 | 51/114/277 | 15,9 | 25/75/171 | 24/45/67 |
| 4 vídeos, política `original` (sem proxy) | 105/142/372 | 107/146/377 | 8,7 | 67/82/322 | 38/59/121 |
| 4 vídeos, MSAA 0 | 39/63/91 | 44/68/95 | 20,8 | – | – |
| Take A + 1 vídeo | 67/85/118 | 69/111/296 | 13,2 | – | – |
| Take A + 2 vídeos | 71/108/135 | 74/119/288 | 12,5 | 62/95/110 | 8/18/25 |
| Take A + 4 vídeos | 84/132/171 | 90/137/326 | 10,3 | – | – |
| Take A + 2 vídeos, MSAA 0 | 43/74/95 | 47/84/262 | 19,1 | – | – |

O MSAA não afeta vídeo (4 vídeos: 31 → 39, ruído); afeta só camadas de cena. O proxy dá ~3,4× (31 vs 105 ms), em linha com OPT01.

### 4. Scrub (arrasto de 2 s a 60 Hz, 120 intenções), load 22–23

| Cenário | MSAA | quadros mostrados | submit→apresentado p50/p95 | soltar → imagem final |
|---|---|---|---|---|
| Take A, 120 quadros de arrasto | 4 | 35–41 | 51–57 / 68–78 | 77–127 ms |
| idem | 0 | 73–81 | 26–28 / 32–47 | 28–71 ms |
| Crossfade (`x2`) | 4 | 20–21 | 115–128 / 140–148 | 191–224 ms |

### 5. Carga inicial e memória

| Passo | Resultado |
|---|---|
| Abrir projeto → primeiro quadro | 1,0 s (load 36) |
| Cada PLY de 66 MB (1.179.648 splats) | fetch 40–120 ms + decode 150–260 ms (load baixo: ~40 + ~165) |
| Mudar para uma cena fria (seek) | ~1,3 s de parede até ficar ocioso |
| Pegada WebContent | 438 MB (aberto) → 482 → 729 → 845 → 945 MB após as 4 cenas (≈ +100–250 MB por cena) |
| Pegada GPU process | ~500–520 MB, estável |
| 4 cenas sobrepostas | WebContent 1117 MB |
| Editor (RSS/pegada) | 340 MB ao abrir → 550 MB após 25 s de reprodução (crescimento não investigado) |
| Retenção `RETAINED_ASSETS=16` | Sem liberação durante a reprodução do projeto inteiro (só 4 splats + efeitos + stills; as liberações aparecem todas no instante do stop). Não há thrash neste projeto. |
| Meshes de splat vivos | até 6 (`assets_with_mesh`) |

### 6. Outros

| Item | Resultado |
|---|---|
| Cache de resultado desligado vs ligado (take A) | 48/63/71 vs 49/60/68: **sem diferença** na reprodução (cena gira/câmera orbita: nunca acerta). Ajuda só parado. |
| Preview 640×360 vs 960×540 vs 1280×720 (take A, MSAA 4, load 40) | 50 / 48 / 58 ms: **resolução quase não muda o custo** (−0 % em 640, +20 % em 1280). |
| Vertical (540×960, take, MSAA 4, load 21) | render 19/38/42 ms, 24,0 fps; MSAA 0: 18/27/37. O mesmo número de pixels custou 2,7× menos que o take A horizontal: **o custo depende da cena/câmera**. |
| UI ociosa | 4 repaints/s, 3,6 ms cada |
| UI durante reprodução | 3,3–4,6 ms de CPU por repaint (p95 ≤ 7); 65 repaints/s (take, MSAA ≤ 2), 46/s (MSAA 4), 30/s (4 camadas) |

## O que o "ms" do cabeçalho mede e omite

`render_ms` = `performance.now()` no fim do `renderFrame` menos o início do `draw`. Inclui o preparo de camadas, o rig,
`await spark.update` (gerar + leitura de profundidade da GPU + sort no worker + upload) e termina em `gl.flush()`.
Não inclui o trabalho de raster que ainda corre na GPU (ele reaparece como espera da leitura de profundidade no quadro
seguinte, por isso o valor de steady-state ainda reflete a GPU em quase tudo), o intervalo entre quadros, quadros que o
relógio avançou e não foram mostrados, a fila/ponte (0,5–1,5 ms), a apresentação (+1–2 ms) e o repaint da UI. O `performance.now`
do WebKit tem quantização de 1 ms. Para ver fluidez, é preciso `fps`/quadros perdidos, não `render_ms`.

## Gargalo dominante por cenário (com evidência)

| Cenário | Dominante | Evidência |
|---|---|---|
| Take único | **Raster na GPU com MSAA 4×**, observado como a espera da leitura de profundidade (43 de 52 ms) | MSAA 4→2→0: 52→14→14 ms; drain: `layer_gpu` 49/59/61 ms vs `frame_gpu` 1 ms; sort 6 ms; rig/draw/flush ≈ 0 |
| Crossfade / camadas empilhadas | Mesmo raster, **multiplicado por camada** (~50 ms por 1 M a MSAA 4); soma o decode por quadro dos splats de efeito (7 ms) e o rig (7 ms) | 1→2→4 camadas: 52→106→179 ms; drain `layer_gpu` por camada 70–73 ms em 2 camadas |
| Piso com MSAA ≤ 2 | **Sort + leitura de profundidade do Spark** (~12–14 ms por camada, serial) | MSAA 0: 14 ms = 6 (leitura) + 6 (sort) + 2 |
| Vídeos | **Preparo por quadro e camada** (`fetch` + `createImageBitmap` + upload), não GPU | 4 vídeos: preparo 15 de 31 ms; sem proxy 67 de 105 ms; MSAA não muda |
| Corte de cena | **Carga/decode do PLY no primeiro quadro que o usa** | intervalo de 228–259 ms com `render_ms` de 38–54 ms; `loads`: fetch ~40 + decode ~165 ms |
| Scrub | Mesmo custo por quadro × fila de uma só posição (o último pedido vence) | só 35–41 de 120 intenções mostradas com MSAA 4; 73–81 com MSAA 0 |
| UI | Não é gargalo | 3,3–4,6 ms de CPU por repaint; a cadência segue a carga do monitor |

### As duas suspeitas do pedido

- **`SceneRig` com splats é reconstruído por quadro:** verdadeiro, mas barato. `rig_build_ms` é 0 (1 camada) e 7 ms (2 camadas);
  nesses 7 ms vai quase tudo para o decode por quadro de splats de efeito compartilhados (`splatLease`: só o primeiro
  levador usa o mesh do asset; os demais decodificam de novo e descartam). Custo medido: **7 ms em 106 (~7 %) por camada
  extra**. Compartilhar o `PackedSplats` removeu o decode (nenhum `decode` registrado), mas o intervalo não melhorou de forma
  visível sob load 43 (123 vs 115 ms): o ganho está dentro do ruído e limitado a ~7 %.
- **`settle` espera o sort a cada quadro:** verdadeiro, em 1 tentativa por camada em regime estável (`settle_attempts` 1–2).
  O sort em si custa ~6 ms (1 camada) a 11 ms (2 camadas); o que o `settle` realmente serializa é a **espera da GPU**
  pela leitura de profundidade (43 de 52 ms). Modo `relaxed` (disparar e usar o sort anterior): o `render_ms` cai para 1 ms
  e o intervalo mediano para 44 ms, mas o p95 piora (115 vs 68 ms): só esconde a GPU atrasada e a fila cresce. Com duas
  camadas, `relaxed` é inválido (um só `SparkRenderer`). Sem prova de apresentação, não é um ganho demonstrado.

## Candidatos a otimização (ganho ÷ esforço)

Medido = número do harness; hipótese = não medido.

| # | Candidato | Esforço | Ganho estimado | Como foi estimado | Risco |
|---|---|---|---|---|---|
| 1 | **MSAA padrão 2× no monitor** (export segue em 4×; `scene_samples` já existe no `hello`) | Muito baixo | Take 52→14 ms (−73 %, **24 fps cheios**); crossfade 106→74 ms (−30 %); projeto inteiro 77 %→94 % dos quadros | **Medido** (`Q-A-*`, `Q-S2-*`, `F-full-*`, load 3–14) | Qualidade de borda não foi comparada (sem PSNR/visual); precisa de validação visual. Splats têm borda suave; planos/texto em cenas podem mostrar serrilhado |
| 2 | **Pré-carga da próxima cena** (fetch + decode do PLY antes do corte, p.ex. no seek/lookahead do plano) | Baixo–médio | Remove um engasgo de ~230–260 ms por corte (3 no projeto): `>62 ms` de 66→~0 (limite) | **Medido** o tamanho do engasgo; a remoção é hipótese | Memória: +100–250 MB por cena pré-carregada (retenção por bytes) |
| 3 | **Não desenhar nem preparar camadas totalmente cobertas** (opacas, `normal`, cobrindo o quadro) | Médio | 4 vídeos empilhados: até 31→7 ms (≈ −75 %); 6: 49→7 ms. Só se as camadas de cima cobrirem as de baixo | **Limite superior medido** (1 vídeo = 7 ms); ganho real depende da timeline | Detecção de cobertura precisa de regra no core (opacidade, bounds, transform) |
| 4 | **Garantir/indicar o proxy antes de reproduzir vídeo** (política `auto`) | Baixo | Sem proxy, 4 vídeos custam 105 ms; com proxy 31 ms (3,4×) | **Medido** (`V-vid4-orig` vs `V-vid4-auto`) | Disco/tempo de geração |
| 5 | **Cache de bitmaps/pre-fetch de quadros de vídeo** (decodificar à frente) | Médio | Parte dos 4–8 ms/camada de preparo, e as caudas p99 (93–200 ms) | **Hipótese** (preparo medido: 15 de 31 ms) | Memória |
| 6 | **Compartilhar `PackedSplats` entre camadas** (efeitos repetidos) | Baixo | ≤ 7 ms por camada extra (~7 % em 2 camadas) | **Medido** o custo do decode; o A/B de ganho ficou no ruído | Baixo |
| 7 | **`maxStdDev` 2 (Spark)** | Baixo | −6 % (quiet, load 10: 52→49) a −15 % (load 23, 48→41) | **Medido**, ruidoso | Muda a aparência (caudas das gaussianas) |
| 8 | **Poda/decimação de splats só para o monitor (LOD ou PLY reduzido)** | Alto | Hipótese: custo ~ linear no número de splats; não medido | **Hipótese.** Tentativa de `lod:true`: com a 1,4 k-splats `vfx-sparks` falhou com `decode_failed` ("holds no splats"); limitada às cenas grandes, o carregamento não terminou em 5+ min. Não foi viável medir (BL031) | Qualidade, tempo de construção da árvore |
| 9 | **Pipeline do sort/`settle` (relaxed)** | Alto | Tira ~12–14 ms do piso por camada com MSAA ≤ 2; só vale quando o piso importa (2+ camadas, > 24 fps) | **Hipótese** (1 camada: `render_ms` 1 ms mas p95 pior) | Imagem com sort desatualizado; precisa provar apresentação |
| 10 | **Mostrar fps/quadros perdidos no cabeçalho** | Baixo | Nenhum no desempenho; evita a leitura enganosa de "16 ms" | – | – |

Medidos sem ganho (descartar): resolução do preview 640 (≈ 0 %), `minAlpha` (≈ 0 %, ruído), `enableLod:false` (≈ −5 %, ruído),
cache de resultado na reprodução (0 %).

## O que não foi possível medir

- **Seek-to-photon físico, vsync real da tela**: só há `submit_to_presented`; `rAF`/`ack` não são fóton.
- **Tempo de GPU por timer query**: o WebKit não expõe; usei drain (custo, não vazão).
- **Qualidade visual** de MSAA 2/0, `maxStdDev`, LoD: sem comparação de pixels (PSNR/visual).
- **LoD do Spark** (BL031): não chegou a carregar.
- **Máquina totalmente ociosa**: a melhor foi load 3; várias execuções A/B estão em load 20–45.
- **Vídeo H.264 nativo (`native-video.js`)**: os vídeos importados viram sequência de quadros no workspace; o caminho `mp4`
  nativo não foi exercitado. Os clipes foram cortados do preview do usuário (1280×720); vídeos 1080p/4K não foram medidos.
- **Projeto vertical**: só um take (MSAA 4 e 0), sem crossfade/scrub.
- **Crescimento do RSS do editor** (340 → 550 MB em 25 s): observado, não investigado.
- **Chrome/Windows**: só macOS/WebKit.
- **Causa do cliff do MSAA 4×** (3,7× por apenas 2× de amostras, 2→4): hipótese de estouro de tile memory da GPU Apple;
  não confirmada.
- Algumas execuções A/B pareadas (sd2, alpha, nolod, relaxed, share) estão com load 20–45; só indicam ordem de grandeza.

## Pergunta em aberto sobre o "16 ms"

Não consegui reproduzir **16 ms com o padrão atual**: o take A custa 52 ms (render) e 54 ms entre quadros, e o cabeçalho
mostraria ~52 ms. 14–16 ms aparece com MSAA ≤ 2, com cenas leves, ou com 1–2 vídeos (7–14 ms). **Pergunta ao usuário:**
em qual momento do projeto o cabeçalho mostra 16 ms (só vídeos? uma cena SHARP parada/tocando? quantas camadas)? Se
for com vídeos, o gargalo é o preparo por quadro e as caudas p99; se for com cenas, o MSAA 4×.

## Resultados após implementar os quatro primeiros candidatos

Commit `50b7e98` (PERF-1 a PERF-4, contratos em [features/viewport.md](features/viewport.md)). Mesma máquina e harness,
binário novo; os números "antes" são os das seções acima (cargas diferentes, ordem de grandeza). Backlog: BL113–BL122
em [backlog.md](backlog.md), todos **A validar**.

| Cenário | Antes | Depois | Observação |
|---|---|---|---|
| Take A, JS render p50 | 51 ms (MSAA 4), 19 fps mostrados | 17 ms (MSAA 2), 24,2 fps | load 6; também nas medidas 14 ms com load 4 |
| Projeto 0–25 s, MSAA 2: maior intervalo entre quadros | 291 ms | 95 ms | pré-carga; intervalos > 62 ms: 20 → 25 (ruído, load 7) |
| Projeto 0–25 s, MSAA 4: maior intervalo | 330 ms | 222 ms | 4× continua saturado; intervalos > 62 ms 66 → 123 |
| 6 vídeos empilhados (mp4 opacos) | 50 ms, 17 fps | 8 ms, 24,2 fps | cobertura; p99 182 → 20 ms |
| 4 vídeos empilhados | 30 ms | 11 ms, 24,2 fps | |
| 4 vídeos sobre splat (`hmix4`) | 85 ms, 11 fps | 8 ms, 24,2 fps | |
| Crossfade 2 camadas (MSAA 2) | 75 ms | 73 ms | sem mudança: duas cenas visíveis; o piso segue ~12–14 ms por camada de sort/readback |
| 4 cenas empilhadas (MSAA 2) | 172 ms (MSAA 4) | 118 ms | cenas nunca são tratadas como cobertas (letterbox) |

Limites: sem comparação visual de pixels entre MSAA 2 e 4 (só hash idêntico no teste de cobertura); a pré-carga não foi
medida isoladamente em memória; o aviso de proxy no cabeçalho só tem teste de unidade/relatório, sem captura de tela;
`render_ms` segue sendo só o tempo do JS.

## Segunda rodada: onde está o custo do readback

Hipótese de partida: com 2 camadas o `readback` (6 → ~48 ms) era uma espera de sincronização. Medido com GPU drenada por
camada (variantes `layers`/`layersnd` do harness, `Q`/`L-*` em `/tmp/perf/results-new`):

| Cenário (MSAA) | GPU por camada (drain) | Leitura sem drain |
|---|---|---|
| 1 camada, 4× | 53 ms | render 64–70 ms |
| 1 camada, 2× | 32 ms | render 14 ms (GPU escondida no intervalo de 41,7 ms) |
| 2 camadas, 2× | 29 + 24 ms | render 73 ms; `readback` da camada 1: 21 ms, da camada 2: 29 ms |
| 1 camada, 0× | 11 ms | render 29 ms (com drain) |
| 2 camadas, 0× | 10 + 9 ms | render 52 ms; fps 17,6 |

Conclusão: o "readback" é onde a CPU espera a GPU da camada anterior; a causa é tempo de GPU por camada, dominado pelo
MSAA em cenas de splats (que nada suaviza). Com 2 camadas a segunda leitura fica atrás do desenho da primeira, por isso o
custo não é linear. Fora o MSAA, cada camada ainda paga ~6 ms de readback + ~6 ms de sort (em série, um `SparkRenderer`
por contexto) e ~7–10 ms de reconstrução do rig por quadro.

**Qualidade (MSAA).** `render.frame` 1280×720 do mesmo quadro (60, 230, 340, 480) com 0×, 2× e 4× e PSNR do ffmpeg:
2× vs 4× = 63,7–67,3 dB; 0× vs 4× = 62,6–66,0 dB (acima de ~60 dB a diferença é de arredondamento). Os quadros são só de
splats e texto/imagens; cenas com caixa ou esfera ainda dependem de MSAA e não foram incluídas.

**Decisão implementada (PERF-5).** Cena só de splats usa 0 amostras no monitor (`hello.splat_samples`), sem mudar export
nem cenas com geometria. A/B pareado (2 rodadas, load 12–29, MSAA 2 nas demais cenas):

| Cenário | MSAA 2 (antes) | Splat-only 0× | 
|---|---|---|
| Crossfade (`x2`) JS render p50 | 68 / 72 ms (14,5 / 13,3 fps) | 50 / 56 ms (19,1 / 17,3 fps) |
| Take A JS render p50 | 18 / 16 ms | 15 / 16 ms (p95 37/32 → 22/28) |

**Decimação (hipótese 8).** PLYs com ½ dos splats (copiados para `/tmp`, originais intactos): crossfade em MSAA 0 de 52 para
36 ms (24 fps), take A de 29 para 20 ms (drain). ¼ dos splats expirou (30 s) sob load 15–30 e não foi concluído.

**Memória da pré-carga.** Projeto inteiro, MSAA 2, uma execução cada: footprint do WebContent 357 MB (antes) vs 572 MB
(com pré-carga); GPU 439 vs 454 MB. É coerente com uma cena de 66 MB a mais carregada, mas é só uma execução.

## Terceira rodada: clones de splat e orçamento da pré-carga

O `rig_build` de 7 ms do crossfade não era o rig (uma camada: 0,1 ms): era o segundo uso do mesmo asset (`vfx-ring`,
53 k splats somados) decodificando o arquivo de novo a cada quadro (`lease_decode` 7 ms, 2 por quadro). Agora o segundo uso
é um clone que compartilha os dados e é reaproveitado.

| Crossfade `x2` (MSAA 2, splat sem MSAA), 3 rodadas pareadas | render p50 | fps mostrado | `rig_build` |
|---|---|---|---|
| Antes (decodifica a cada quadro) | 50 / 49 / 48 ms | 18,7 / 20,1 / 19,8 | 7 ms |
| Depois (clone no conjunto) | 41 / 41 / 42 ms | 23,9 / 23,4 / 23,0 | 0 ms |
| 4 camadas (`x4`) antes / depois | 72 ms / 80 ms | 12,6 / 12,6 | 8 → 0 ms (a espera passou ao readback: é GPU) |

O sort/readback por camada (BL126) e a decimação (BL127) ficam com os números acima: o crossfade já está no limite de
41,7 ms e a espera de 3+ camadas é de GPU (~10 ms por camada com MSAA 0).

## Quarta rodada: memória

Tour de 4 cenas do projeto real (quadros 60, 230, 340, 480 e de volta ao 60; MSAA 2), pegada do processo WebContent
(`footprint`, MB) e contagem do próprio runtime (`cache.memory`):

| Quadro | 60 | 230 | 340 | 480 | 60 (volta) |
|---|---|---|---|---|---|
| Antes | 471 | 711 | 821 | 1034 | 1061 |
| Depois (3 execuções) | 402–405 | 576–582 | 626–629 | 592–673 | 602–673 |

O maior item contabilizado era o arquivo PLY bruto retido (65 MB por cena; 255 MB no fim), enquanto os splats decodificados
somavam só 18,8 MB por cena (72,8 MB no fim). O `SplatMesh` já copia `fileBytes` antes de enviá-lo ao worker, então nem a
cópia `slice(0)` era necessária. O restante da pegada é transitório (decodificação, coleta de lixo, estruturas do Spark) e
não é contabilizado. Os bytes brutos deixaram de ser retidos (BL128); a sessão passou a limitar por bytes o que mantém carregado
(BL129; o tour cabe no orçamento, então a remoção só está provada por teste) e os quadros de vídeo ganharam orçamento global
de CPU (BL130; 10 quadros de um projeto de vídeo ≈ 35 MB, longe do limite).

### Crescimento do processo do editor (BL122)

O processo do editor (Rust) crescia ~65 MB por cena nova, independentemente do runtime. `vmmap` mostrou a região
`MALLOC_LARGE (empty)` (blocos grandes liberados que o alocador guarda) indo de 88 a 280 MB no tour. Um interposer de `malloc`
(dylib local que registra alocações ≥ 1 MiB com pilha) apontou a origem: `host::serve` → `read_to_end` num `Vec` vazio, que
cresce por `realloc` dobrando (1, 2, 4, 8, 16, 32, 64 MiB para cada PLY de 66 MB), com ~190 MB de blocos de 64 MiB, 96 MB de 32 MiB etc.
no tour. `malloc_history` não serviu (só mostra alocações vivas).

Correção: capacidade exata (um bloco) e resposta de intervalo limitada a 4 MiB. Depois do conserto, o interposer só vê blocos de
4 MiB de `serve` (os mesmos reaproveitados) e `MALLOC_LARGE (empty)` fica em ≤ ~84 MB constante (era 88→280 MB, crescendo);
a pegada do editor no tour fica entre 150 e 325 MB (era 316→532 MB). O `footprint` do macOS oscila porque o sistema
comprime e devolve memória por conta própria, então a comparação usa a região do alocador, que é determinística em
tendência. Ler o PLY em intervalos de 4 MiB no JS reduziu o editor a ~5 MB nessa região, mas o WebContent subiu para ~780 MB
(os `ArrayBuffer`s das partes esperam a coleta de lixo): sem ganho líquido, não foi mantido.

## Backlog

Os itens propostos foram registrados em [backlog.md](backlog.md): BL113 (MSAA 2×), BL114 (pré-carga), BL115 (cobertura),
BL116 (aviso de proxy) e as hipóteses restantes BL117–BL122 (quadros de vídeo à frente, `PackedSplats` compartilhado,
LoD, pipeline do `settle`, fps no cabeçalho, RSS).

## Estado da instrumentação

A instrumentação (`trace.rs` com janelas maiores, `TESSEL_PERF_PREVIEW_LONG_SIDE`, `render.status` consumindo o trace,
marcas de repaint em `perf_ui.rs`) **não foi integrada**: foi revertida da árvore. O harness e o `rust-instrumentation.patch`
ficam locais em `scripts/perf-investigation/` (não versionados); dados brutos em `/tmp/perf/results` e `results-new`.
