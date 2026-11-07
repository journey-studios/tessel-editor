# Investigação de performance — projeto real com splats e vídeos (2026-11-06)

Investigação, não implementação: nenhuma otimização foi entregue. Perguntas do relato: *"Com vários
vídeos, várias coisas, a perf cai, está em 16 ms e não está super fluida. É possível otimizar?"*

**Resposta curta.** Sim. O "16 ms" do cabeçalho não mede o que torna a reprodução lenta, e o gargalo
neste projeto é a GPU rasterizando 1,18 M de gaussianas com MSAA 4×, não a CPU, a ponte, o GPUI nem
o `SceneRig`. Com vídeos, o gargalo é outro (decodificação de quadro sob demanda, sem read-ahead).
Duas alavancas medidas, ambas de preview, tiram a reprodução e o scrub do projeto de 18 fps / 10 fps
para tempo real: **MSAA 0 durante movimento** e **ordenação de splats sem espera durante movimento**. "Tempo real" aqui é só a cadência de quadros aceitos pelo host perto de 24 fps (§1); não é imagem vista na tela.

**Estado posterior (revisão da PR #14).** Este relatório é de investigação e foi medido numa base anterior a `5ebb855`.
Depois dele, a frente MSAA foi integrada (BL129, PR #17), a malha compartilhada de efeitos também (BL131, PR #15) e a ordenação
sem espera (BL130, PR #19, merge `11832a9`). As tabelas **não** foram refeitas: cada frente re-mede antes de afirmar ganho.
Escopo acrescido nesta PR (só documentação, depois do merge da PR #19, auditoria 5812503277): as alegações de BL130 em `spark.md`, `viewport.md`, no README
de `bl130-spark-settle/` e na linha BL130 do backlog foram alinhadas ao que sobrou da prova do scrub (2 execuções em `release-frame-check.json`; os hashes dos
quadros 20/150 estão retirados) e a fraqueza de proveniência dos JSON de `final-gate/` foi registrada. Nenhuma medição nova, nenhum código nem artefato histórico alterado.
Na revisão, as linhas de **um vídeo (N = 1)** foram retiradas (vinham de uma execução inválida, ver as seções 5 e 8)
e os drivers passaram a falhar de forma explícita (seção 11).

Dados brutos, drivers e patch de instrumentação (não aplicado ao código) estão em
[`viewport-performance-results/investigation-2026-11-06/`](viewport-performance-results/investigation-2026-11-06/).
Contexto anterior: [viewport-performance-backlog.md](viewport-performance-backlog.md),
[viewport-performance-next-report.md](viewport-performance-next-report.md).

## 1. Método e proveniência

- Máquina: Apple M3 Pro, 18 GiB, macOS (Darwin 25.6). Monitor 960×540 (limite do preview; projeto 1280×720@24).
- Binário medido: **release** com instrumentação local (`tessel-editor-instr`, sha256 `29eb53f7ea74939f…`),
  runtime Three.js 0.187 + Spark 2.4.0 reempacotado com a instrumentação. Um binário debug com a mesma
  instrumentação foi usado só para a comparação de UI (seção 6).
- Projeto: cópia em `/tmp` de `tessel-splat-h` (4 cenas `tessel-scene@1`, cada uma com PLY SHARP de
  1.179.648 gaussianas, 2 splats de efeito pequenos, crossfades de 14 quadros em 186–200, 268–282,
  410–424; áudio mudo na cópia). Originais intocados; nada copiado para o repositório.
- Cada execução usa `HOME` isolado (socket próprio), só operações semânticas (`op`, `get`, `set`). A instância
  do usuário (pid 74492) não foi tocada.
- **Contaminação conhecida:** a máquina estava com carga (load average ≈ 10) e a GPU era compartilhada com
  outros agentes (utilização ambiente ≈ 16 %, picos de 60 %). Mitigação: espera por GPU quieta antes de cada
  execução, 2–3 repetições, utilização de GPU registrada por execução. Os números têm ruído de ±5–10 %;
  diferenças menores que isso não são conclusões.
- Duas famílias de execução:
  - **sem drain** (`nd-*`, `scrub-*`, `ui-*`): o que o usuário vê; o runtime não força a GPU a terminar.
  - **drain** (`sw-*`): após cada quadro o runtime força a conclusão da GPU (`readPixels` 1×1). Mede o custo real
    por quadro (`render_ms` + `drain_ms`, coluna *r+d*) sem que um quadro absorva a cauda do anterior.
- "Quadros perdidos" = quadros do relógio de reprodução (24 fps) que não geraram um quadro distinto **aceito pelo host**.
  O trace do host guarda por pedido `done_ms` (relógio do host quando a resposta do runtime foi aceita) e `shown_frame` (quadro
  da sequência que a imagem aceita mostra); intervalos e perdas saem daí, p99 só quando o balde tem quadros suficientes.
  São **estimativas do host**: aceito não é exibido. Não medem o que a tela mostrou nem o scan-out (não medido, §8, BL032/BL034),
  e `presented_after_done_ms` (relógio da página, próximo *animation frame*, só quando o host opta) também não é pixel visto.
  Toda conclusão de "tempo real" ou "fluido" neste relatório vale só até esse limite; o relatório não afirma pixels.

## 2. O que o "16 ms" mede

O cabeçalho `Three.js · 960×540 · N ms` é `frame.render_ms`: relógio da página, de `performance.now()` no
início do desenho até a submissão. Inclui `prepare`, a espera da ordenação do Spark (`settle`) e o submit.
**Não inclui** a cauda de rasterização na GPU, o trânsito da ponte (≈ 1 ms), nem a apresentação.

Com a GPU ociosa (parado, primeiro quadro, ou drain) ele mostra **14–17 ms**: exatamente o "16 ms" do relato.
Decomposição de um quadro em repouso (take, MSAA 4, GPU ociosa, `settle_total` ≈ 14 ms):

| Etapa | ms (p50) |
|---|---|
| `generate()` Spark (CPU) | 0–1 |
| leitura de profundidade (`readRenderTargetPixelsAsync`) | 6 |
| ordenação no worker (1,2 M) | 6 |
| submit do desenho / composição | 0–1 |
| **cabeçalho** | **14–16** |
| **cauda de raster na GPU (drain), não aparece no cabeçalho** | **36 (p95 53)** |

Em reprodução contínua o número muda de significado: a leitura de profundidade do quadro N bloqueia até a GPU
terminar o raster do quadro N−1, então o cabeçalho passa a ser ≈ o custo total (take A: **48–55 ms**; crossfade:
**95–115 ms**). Ou seja: um "16 ms" durante reprodução com MSAA 4 neste projeto só pode vir de um quadro parado, de
um trecho mais leve, ou de vídeo com proxy (seção 5). **Pergunta em aberto:** onde exatamente o usuário viu 16 ms;
não consigo reproduzir 16 ms em reprodução de take com cenas de 1,2 M.

O cabeçalho é um bom indicador de CPU+espera, um mau indicador de fluidez. O indicador confiável é o
intervalo entre quadros entregues (host `done`), e a perda de quadros.

## 3. Reprodução contínua (sem drain, o que o usuário vê)

Take A (quadros 40–114, 1 cena de 1,18 M splats) e crossfade (quadros 186–200, 2 cenas) em 24 fps (alvo: 41,7 ms/quadro).
Valores por execução (2 repetições), faixa entre elas.

| Config | Trecho | Quadros perdidos | Intervalo p50 / p95 / p99 (ms) | Cabeçalho p50 / p95 (ms) | GPU util. média |
|---|---|---|---|---|---|
| **baseline (MSAA 4)** | take A | **24–27 %** | 57,6–58,1 / 65,6–70,1 / 70–77 | 54–55 / 62–69 | 78–81 % |
| | crossfade | **57 %** | 96–99 / 108–112 / — | 95–97 / 112–115 | 68 % |
| MSAA 2 | take A | 0 % | 41,9–42,6 / 47–48 / 48–56 | 15–18 / 30–32 | 75 % |
| | crossfade | 36 % | 65–72 / 75–81 / — | 64–69 / 70–74 | 56–59 % |
| **MSAA 0** | take A | 0–1,4 % | 43,8–45,3 / 50 / 55–76 | 14–16 / 16–25 | 36–57 % |
| | crossfade | 7 % | 46–47 / 49–54 / — | 44–45 / 49–50 | 33–35 % |
| ordem de sort velha (*stale*), MSAA 4 | take A | 0 % | 42–44 / 78–83 / — | **1** / 1 | 82–84 % |
| | crossfade | 29–50 % | 16–31 / 195–250 / — | 9 / 163–209 | 76–82 % |
| **stale + MSAA 0** | take A | **0 %** | 44,1–44,3 / 45,8–47,4 / 47–49 | 1 / 1 | 31–40 % |
| | crossfade | **0 %** | 42,6–44,1 / 48–49 / 51–66 | 8 / 9–12 | 35–44 % |

Notas de leitura:

- Nenhum p99 é apresentado para o balde `single` do trecho 181:30: ele inclui o salto 185→200 (15 quadros ≈ 630 ms) entre
  dois quadros "single" consecutivos do balde; é artefato do bucketing, não travamento.
- O cabeçalho do modo *stale* mente (1 ms): não espera a ordenação. Em MSAA 4 a GPU satura (82 %) e o intervalo
  fica instável; só com MSAA 0 o conjunto fecha o orçamento.
- O trecho 181:30 de takes B/C (fora do take A) fica em 39–45 ms/quadro com MSAA 4: o custo depende de quanto da tela
  as 1,2 M gaussianas cobrem (take A é o pior caso medido, cabeçalho 32–34 ms em B/C contra 54 em A).

### Custo real por quadro (drain, *r+d* p50) e alavancas

| Config (take A / crossfade) | r+d take A (ms) | r+d crossfade (ms) | Observação |
|---|---|---|---|
| baseline MSAA 4, 960×540 | 53–54 | 97–103 | perda 23–25 % / 57–64 % |
| MSAA 2 | 34–36 | 72–90 | |
| MSAA 0 | 22–24 | 48–50 | |
| stale, MSAA 4 | 38–39 | 75–80 | ganho ≈ 12–15 ms por camada de cena |
| stale + MSAA 0 | 9–11 | 17–19 | |
| preview 480 de lado maior | 45–46 | 98–103 | resolução pesa pouco |
| preview 640 | 52–57 | 86–91 | |
| preview 1280 | 57–67 | 123–134 | |
| MSAA 0 + preview 640 / 480 | 22–25 | 46–48 | ≈ igual a MSAA 0 em 960 |
| retenção desligada (`RETAIN=0`) | 53–71 | 103–109 | +15–30 % |
| cache de resultado desligado | 59–61 | 101–102 | dentro do ruído |
| `stdDev` 2,0 / 2,4 | 46–53 | 91–103 | sem ganho relevante |
| `minAlpha` maior / `maxPixelRadius` menor | 54–61 | 95–115 | sem ganho |

Conclusões sustentadas por medição:

1. O gargalo é a **GPU** (raster de ≈ 36–40 ms por cena de 1,2 M a MSAA 4 em 960×540, ≈ 9 ms a MSAA 0). A CPU gasta
   ≈ 14 ms por cena (6 de readback + 6 de sort), que só aparece no caminho crítico porque o `settle` serializa
   CPU↔GPU por quadro.
2. **MSAA é a maior alavanca** (≈ 2,3× em take, ≈ 2× em crossfade). Resolução do preview e parâmetros do Spark
   (`stdDev`, `minAlpha`, raios) não são alavancas relevantes para este conteúdo.
3. O crossfade custa ≈ 2× um take porque as duas cenas ordenam e rasterizam em série (a leitura da segunda espera o
   raster da primeira): readback p50 73 ms contra 40 ms, sort 11 ms contra 6 ms.
4. A retenção de assets ajuda (+15–30 % de custo sem ela). Cache de resultado não ajuda em movimento (nunca acerta).

### Hipóteses do pedido

- **`SceneRig` reconstruído por quadro:** *refutada como custo*. `rig_build` mede 0–1 ms em quadro de uma cena. Exceção
  real: nos quadros de crossfade `rig_build` ≈ 7 ms e `lease_decode` ≈ 7 ms (p50 7–8, p95 13): o asset de efeito
  (`vfx-ring`, `vfx-sparks`) é pedido pelas duas cenas, o *lease* do mesmo asset só serve a primeira, e a segunda
  redecodifica o PLY a cada quadro (+ troca de mapping do Spark). Isso são ≈ 7 de ≈ 44 ms no crossfade a MSAA 0.
- **`SplatStage.settle` esperando a ordenação por quadro:** *parcialmente confirmada*. Tentativas por quadro: p50 = 1,
  `settle_sleep` ≈ 0 (não é laço de espera), mas a serialização custa **12–15 ms por camada de cena**
  (experimento *stale*: r+d 54→38 em take, 23→10 com MSAA 0), muito menos que o raster. É o segundo maior item, não o primeiro.

## 4. Scrub (arrastar o playhead), `seek-scrub-proof.py`-equivalente, 2 s a 60 Hz, 100 quadros de curso

Apresentações = quadros novos na janela de 2 s. *Lag* = distância em quadros entre o playhead e o quadro mostrado.

| Config | Trecho | Apresentações / 2 s | Intervalo p50 / p95 / máx (ms) | Lag p50 / p95 / máx (quadros) | Largada→final (ms) |
|---|---|---|---|---|---|
| baseline MSAA 4 | take | 37–39 (≈ 19/s) | 50–54 / 64–67 / 70–76 | 3 / 3–4 / 4 | 49–101 |
| | crossfade | 35–40 | 45–49 / 90–103 / 110–119 | 2–3 / 4–5 / 5–6 | 64–81 |
| | take, 30 Hz | 33–42 | 46–62 / 56–73 / 65–93 | 2–3 / 4–5 / 4–5 | 62–82 |
| **MSAA 0** | take | **95–97 (≈ 48/s)** | **19,8 / 25 / 27–63** | **1 / 1 / 2–3** | **20–24** |
| | crossfade | 83–86 | 20 / 44–50 / 57–70 | 1 / 2 / 3–4 | 18–25 |
| stale + MSAA 0 | take | 98–100 | 17,8 / 34–37 / 40–90 | 0 / 0 / 0–3 | 2–4 |
| | crossfade | 99–100 | 18,5–19 / 34–35 / 38–48 | 0 / 1 / 1 | 5–10 |

Ressalva *stale*: a coluna "largada→final" confirma convergência de revisão, não pixel igual ao render
explícito; o experimento deixa a ordem velha, então a qualidade do último quadro **não foi provada** (isso exigiria
`TESSEL_VIEWPORT_INSPECT=1` e uma etapa de refino na pausa).

Com MSAA 4 o scrub entrega ~19 quadros/s com 3 quadros de atraso (≈ 125 ms) e leva até ≈ 100 ms para convergir; com MSAA 0, ~48 quadros/s com 1 quadro de atraso.

## 5. Vídeos H.264 sobrepostos (variante em `/tmp`, 1280×720@24)

Projeto novo (`project.create` + `media.import`) com N trilhas de vídeo opacas, tela cheia, sobrepostas, sobre 2 fontes MP4
H.264 de 6 s (offsets de fonte diferentes por trilha), 96 quadros reproduzidos, 2 repetições. `original` e `auto` (proxy) via
`op preview.policy`. GPU média 22–37 % (execução `vid-c`): **a GPU não é o gargalo aqui**.

**Fonte das linhas:** só a execução `vid-c` (2 fontes, sem op rejeitada) sustenta as linhas de N = 2 e N = 4. A execução `vid-b`
é **inválida** (todos os `clip.add` foram rejeitados e a linha de tempo estava vazia) e **abortada** por disco; `vid-c` também
terminou abortada por disco depois de N = 4. Ambas estão marcadas em `raw-run-logs.txt`.

| N vídeos | Política | Quadros perdidos | Intervalo p50 / p95 (ms) | Cabeçalho p50 / p95 (ms) |
|---|---|---|---|---|
| 1 | — | **não medido** (as linhas anteriores vinham da execução `vid-b` inválida, sem clipes na linha de tempo) | — | — |
| 2 | original | 0–10 % | 43,3–45,9 / 54–57 | 33–44 / 44–54 |
| 2 | auto | 0 % | 43–44 / 50–54 | 13–14 / 21–32 |
| 4 | original | **54–67 %** | 90–128 / 148–188 | **83–125 / 146–185** |
| 4 | auto | **4,5 %** | 42 / 81–96 | 18–20 / 61–68 |
| 8 | — | **não medido** | — | — |

- Efeito do proxy: com 4 vídeos reduz perda de 54–67 % para 4,5 % e o cabeçalho de ≈ 100 para ≈ 19 ms. Mesmo assim o p95 do intervalo
  (81–96 ms) mostra soluços: `auto` ainda não é fluido com 4 camadas.
- Por que o cabeçalho cai tanto com vídeo: aqui ele *é* o custo (decodificar + subir textura), ao contrário do caso de splats.
  Isso pode explicar "16 ms e não fluido": com proxy, cabeçalho ≈ 13–20 ms, mas p95 do intervalo ≈ 80–95 ms.
- Causa provável (leitura de código, não medida por etapa): `AssetStore.frameTexture` (`runtime/src/assets.js`) busca e
  decodifica cada quadro (`fetch` + `createImageBitmap`) **sob demanda no caminho do desenho, sem read-ahead**; cada camada paga
  4–10 ms de preparo (`layers[].ms`, p50 4) e a composição 5–9 ms. Não separei decodificação de upload por política.
- Limites: N = 1, 8 vídeos e o cenário misto (splat + vídeo) **não foram medidos** (seção 8). Sem N = 1 válido não há linha de base de
  "um vídeo" nem curva N = 1→4: o escalonamento acima só compara N = 2 e N = 4.

## 6. Fluidez da UI GPUI durante a reprodução (separada do monitor)

Release e debug com a mesma instrumentação. RTT = tempo de uma requisição semântica (`get timeline.playhead`) ao socket,
atendida pela thread principal do GPUI, medida a cada 100 ms.

| | Release | Debug |
|---|---|---|
| RTT parado p50 / p95 / p99 (ms) | 0,84 / 1,1 / 3,7 | 4,0 / 20 / 42 |
| RTT em reprodução (take) p50 / p95 / p99 | 0,85 / 4,4 / 5,1 | 13,9 / 26 / 31 |
| RTT no crossfade p50 / p95 / p99 | 0,88 / 2,4 / 4,5 | 15,4 / 30 / 32 |
| Repaints GPUI por segundo (reprodução) | 35–45 | 22–24 |
| Intervalo entre repaints p50 / p95 | 25 / 33–42 ms | 41,7 / 42–54 ms |
| `playback_tick` p50 / p99 | 0,05 / 0,15 ms | 0,21 / 0,38 ms |
| `Workstation::render()` p50 | 0,013 ms | 0,06 ms |
| CPU médio do processo (parado / reproduzindo) | 2,5 % / 6–16 % | 20 % / 25–60 % |
| Thread principal ocupada (amostra de 2 s em reprodução) | 32 % | 17 % |

- A thread principal do GPUI **não está saturada** em nenhum dos binários (≤ 32 % ocupada); a reprodução fornece um repaint
  a cada ≈ 25 ms (2 por quadro). O tick e `render()` custam microssegundos (ressalva: medem a função, não layout/paint do GPUI,
  cujo custo aparece no perfil como `taffy`/`bounds_tree`, poucas amostras).
- O monitor é a WebView nativa pintada sobre o GPUI: trânsito da ponte p50 ≈ 0,9 ms (p95 ≈ 3,7), `presented_after_done` p50 0 ms;
  plano, serialização, despacho e aceitação < 0,1 ms. Não há leitura de pixels nem JPEG no caminho interativo.
- **Binário do usuário:** `/Users/wallyssonnunes/www/tessel-video-splat/bin/tessel-editor` (141 MB) contém as strings de overflow-check
  (`attempt to add with overflow`) que o meu release (35 MB) não tem e o meu debug (135 MB) tem. Forte indício de **build debug**
  (não confirmei pelo processo, só pelo arquivo; não toquei na instância).
  Efeito medido do debug: UI 15× mais lenta a responder (RTT 14 ms contra 0,85 ms), CPU 20 % parado, metade dos repaints.
  Nas minhas execuções o debug **não** perdeu mais quadros do monitor que o release (a produção de quadros está na WebView/GPU),
  então o build debug explica "UI pesada", não "monitor a 18 fps". Recomendação: confirmar com o usuário e usar release.
- Cauda dos repaints: p99/máx do intervalo de GPUI de 90–350 ms em release durante algumas execuções (ex.: 225 ms no início de `playback.start`,
  que mixa o áudio da sequência antes de tocar); não correlacionei com o monitor.

## 7. Carga inicial e memória

Release, projeto `tessel-splat-h`, instrumentado.

| Momento | Editor RSS / footprint | WebKit GPU RSS / footprint | WebContent RSS / footprint |
|---|---|---|---|
| boot (0,85 s até a viewport pronta) | 158 / 242 MB | 50 / 189 MB | 82 / 93 MB |
| projeto aberto (3 ms; carga preguiçosa) | 210 / 310 MB | 75 / 121 MB | 428 / 436 MB |
| após visitar as 4 cenas | 394 / 525 MB | 150 / 520 MB | 832 / 1127 MB |
| após a 2ª rodada | 392 / 525 MB | 150 / 518 MB | 826 / 936 MB |
| após execuções de reprodução | 265 / 566 MB | 94 / 511 MB | 560 / 966 MB |

- Total ≈ **2,0–2,2 GB de footprint** com as 4 cenas. 11 dos 17 assets visuais carregados; Spark: máximo 1.245.184
  splats por acumulador; até 6 objetos Spark vivos ao mesmo tempo (campo `meshes.alive` do instrumento).
- Primeira visita de cada take: seek→pronto **275–345 ms** (decodificar 66 MB de PLY + primeira ordenação), ≈ 7–8 quadros de
  travada ao entrar num take ainda não carregado; segunda visita: 55–63 ms. Cabeçalho na primeira visita: 23–81 ms.
- Teto de retenção (`RETAINED_ASSETS = 16`) não foi atingido neste projeto (11 carregados, nenhum recarregado na 2ª rodada), mas o
  projeto tem 17 assets visuais: um projeto que visite > 16 assets passa a recarregar (RETAIN=0 custa +15–30 %).
- Não medi 0→600 a frio em uma única passada contínua (todas as reproduções partiram de takes pré-visitados).

## 8. O que não foi medido, e por quê

- **Vídeo com N = 1 (linha de base):** a única execução com N = 1 (`vid-b`) mediu uma linha de tempo vazia porque todo `clip.add` foi
  rejeitado (`source_start` enviado como texto); foi preservada no log bruto com marcação e **retirada** das tabelas. Precisa de
  nova execução com o driver corrigido (BL136); não foi repetida aqui porque a revisão corrigiu os drivers, não refez a medição pesada.
- **Vídeo com N = 8 e cenário misto (splat + 2–4 vídeos):** o disco do host chegou a < 0,5 GiB livres (concorrência de outros processos);
  o job de decodificação travou em `media_preparing` e a execução foi abortada. Sem esses dois, a interação splat × vídeo
  (o caso mais provável do usuário) é **hipótese**.
- **Seek-to-photon físico, outros monitores/taxas de atualização:** sem hardware (BL032/BL034).
- **Tempo de GPU isolado:** WKWebView não expõe timer queries; o "custo de GPU" é *r+d* (drain), que embute a CPU residual.
- **GPU limpa:** compartilhada com outros agentes; números têm ruído de ±5–10 %.
- **Qualidade de MSAA 0 / *stale*:** não avaliada (nenhum PSNR/hash contra MSAA 4). Pelo OPT07 as bordas pioram; para gaussianas
  mescladas por alpha o efeito deve ser pequeno, mas **é hipótese** até comparar.
- **Vertical (`tessel-splat-v`):** não executado; mesma composição esperada, resolução do projeto diferente.
- A instrumentação (patch) altera os binários medidos; o custo dela (≈ micro-segundos por etapa, um `postMessage` por quadro) foi desprezado.

## 9. Otimizações candidatas (ordenadas por ganho ÷ esforço)

Cada ganho vem marcado: **[medido]** = diferença observada nas tabelas acima; **[hipótese]** = estimativa sem medição direta.

| # | Candidata | Ganho estimado | Como foi estimado | Esforço / risco |
|---|---|---|---|---|
| 1 | **MSAA 0 (ou 2) durante reprodução e scrub**; 4× em pausa, `render.frame` explícito e export | take A: 54→23 ms r+d, perda 25 %→0–1 %; crossfade 98→48 ms, perda 57 %→7 %; scrub 19→48 quadros/s, lag 3→1 | **[medido]** (`TESSEL_VIEWPORT_SCENE_SAMPLES`, 2 reps; MSAA 2 intermediário: take 0 % perda, crossfade 36 %) | Baixo: já existe como variável de ambiente; falta qualidade por pedido + refino na pausa. Risco: borda das gaussianas (não avaliado); explícito/export devem continuar a 4×. |
| 2 | **Ordem de sort velha em movimento**, `settle` só na pausa/refino | −12–15 ms por camada de cena; com MSAA 0: take 23→10 ms, crossfade 48→18 ms, scrub com lag 0 e 48→50 quadros/s | **[medido]** como experimento (`exp-stale`); correção do quadro final **não provada** | Médio: Spark `update` sem espera, política por qualidade do pedido, pixel proof na pausa. Risco: *popping* de ordem com câmera animada rápida. |
| 3 | **Compartilhar o lease/malha de assets de efeito entre camadas do crossfade** | ≈ 7 ms (decode) + mapping a cada quadro de crossfade (≈ 15 % do crossfade a MSAA 0) | **[medido]** (`lease_decode` e `rig_build` p50 7 ms só em quadros de 2 cenas) | Baixo–médio: `splatLease`/`SceneRig`; cuidado com mutação de transform por camada. |
| 4 | **Pré-aquecer splats à frente do playhead** (e/ou carregar na abertura em segundo plano) | elimina a travada de 275–345 ms (≈ 7–8 quadros) na primeira entrada em cada take | custo **[medido]**, ganho **[hipótese]** | Médio: job de pré-carga no `Session` com prioridade abaixo do quadro corrente. |
| 5 | **Read-ahead de quadros de vídeo** (próximos 2–4 quadros por camada, descartando ao pausar/seek) | alvo: reduzir o p95 de 81–96 ms (4 vídeos com proxy) e perda em `original` | custo por camada **[medido]** (4–10 ms), efeito do read-ahead **[hipótese]** | Médio: `AssetStore.frameTexture` + trim de cache. |
| 6 | **Política de proxy mais agressiva** (≥ 3 camadas de vídeo → proxy sempre) | N = 4: perda 54–67 % → 4,5 % | **[medido]** (auto contra original) | Baixo, se já for o padrão; confirmar a política efetiva do usuário. |
| 7 | **Release no uso diário / confirmar build do usuário** | UI ≈ 15× mais responsiva (RTT 14→0,85 ms), ≈ 8× menos CPU parado | **[medido]** | Trivial; o monitor em si não mudou. |
| 8 | Elevar/ajustar `RETAINED_ASSETS` (> 16) ou contar só assets pesados | evita +15–30 % e recargas de 275–345 ms em projetos > 16 assets visuais | **[medido]** (`RETAIN=0`); efeito do teto **[hipótese]** | Baixo; memória sobe (≈ 0,15 GB/take medido). |
| 9 | Ordenar/rasterizar as duas cenas do crossfade em lote (uma leitura de profundidade) | ≈ 12–20 ms de ≈ 44 (MSAA 0) | **[hipótese]** a partir de readback 21 vs 6 ms e sort 11 vs 6 ms | Alto: Spark com múltiplos acumuladores num só passe. |
| 10 | Pular camadas com opacidade ≈ 0 nas pontas do crossfade | ≤ 1–2 quadros por transição | **[hipótese]** | Baixo, ganho pequeno. |
| 11 | Preview a 480–640 em vez de 960 | 5–15 % (single, MSAA 4); ≈ 0 com MSAA 0 | **[medido]** | Perda de nitidez; só como último recurso. |

Combinação 1 + 2 (medida em conjunto, `nd-stale-msaa0`): **0 % de perda no take e no crossfade**, intervalo 44 ms ≈ relógio de 24 fps, GPU 31–44 %.
Isso também deixa folga de GPU para vídeo + splat juntos, mas a interação não foi medida (seção 8).

Fora da lista por não ter ganho medido: cache de resultado (nunca acerta em movimento), `stdDev`/`minAlpha`/`maxPixelRadius` (sem ganho),
elevar a resolução do preview (piora), tocar a cadência de `cx.notify()` (UI não é o gargalo).

## 10. Linhas propostas para `docs/backlog.md` (IDs a definir pelo coordenador)

Proposta original, mantida como histórico. Os IDs reais estão registrados em `docs/backlog.md` (seção *Performance de preview*): BLxxx-1 → BL129,
BLxxx-2 → BL130 (filho de BL031; linha única, da PR #19), BLxxx-3 → BL131, BLxxx-4 → BL132, BLxxx-5 → BL133, BLxxx-6 → BL136, BLxxx-7 → BL135,
BLxxx-8 → BL137, BLxxx-9 → BL134, BLxxx-10 → BL136. O rótulo `PERF-N` da coluna *Origem* do backlog é o N de `BLxxx-N` **deste §10** (não o # do §9):
BL134 = PERF-9, BL137 = PERF-8, BL136 = PERF-6/PERF-10. BL138 (hipóteses #9 e #10 do §9) não vem de uma linha deste §10 e se rotula `§9 #9/#10`.
Fonte comum: este relatório.

| ID | Item | Estado proposto | Aceite |
|---|---|---|---|
| BLxxx-1 | Qualidade de preview por movimento: MSAA 0/2 em reprodução e scrub, 4× em pausa, explícito e export (cobre OPT07 parcialmente) | Pendente | Take A ≥ 24 fps sem perda; scrub ≥ 40 quadros/s; frame explícito e export idênticos ao atual; comparação visual MSAA 0 × 4 registrada |
| BLxxx-2 | Ordenação Spark sem espera durante movimento + refino exato na pausa | Pendente (experimento) | Crossfade 0 % perda a MSAA 0; hash do quadro final = render explícito; sem popping visível em câmera animada |
| BLxxx-3 | Lease/malha compartilhada de assets de efeito em crossfade | Pendente | `lease_decode` = 0 em quadros de 2 cenas com asset de efeito comum; teste de runtime |
| BLxxx-4 | Pré-carga de splats antes do playhead entrar na cena | Pendente | Entrada fria em take ≤ 1 quadro de atraso; memória e cancelamento medidos |
| BLxxx-5 | Read-ahead de quadros de vídeo na WebView | Pendente | 4 vídeos com proxy: p95 do intervalo ≤ 50 ms; perda ≤ 1 % |
| BLxxx-6 | Medir vídeo N = 8 e cenário misto splat + vídeo em disco folgado | Bloqueado (disco) | p50/p95/p99, perdas e memória por N; ver seção 8 |
| BLxxx-7 | Indicador de desempenho honesto no cabeçalho do monitor (intervalo real/quadros perdidos; marcar que `render_ms` exclui a cauda de GPU) | Decisão | Cabeçalho não passa a impressão de 16 ms quando o intervalo é 55 ms |
| BLxxx-8 | Confirmar com o usuário o build debug (`bin/tessel-editor` 141 MB) e entregar binário release | Decisão | Build release usado no dia a dia; RTT semântico < 2 ms |
| BLxxx-9 | `RETAINED_ASSETS` ≥ nº de assets pesados ou contagem por peso | Pendente | Projeto de 17+ assets visuais sem recarga na 2ª passada |
| BLxxx-10 | Medir o vertical (`tessel-splat-v`) e 0→600 a frio contínuo | A validar | Mesmas tabelas das seções 3 e 7 |

## 11. Reprodutibilidade

- `viewport-performance-results/investigation-2026-11-06/instrumentation.patch`: patch local (não aplicado): `perf_probe.rs` (log em `TESSEL_PERF_LOG`),
  `perf.js` (etapas por quadro enviadas por `ipc.postMessage`), `render`/`playback_tick` no host, e as variáveis experimentais
  (`TESSEL_PERF_PREVIEW_LONG`, modo *stale* e parâmetros do Spark via `globalThis.__TESSEL_EXP`).
  O modo *drain* é uma leitura 1×1 após o `send` (`DRAIN`).
- `drivers/`: Python de apoio (`drv.py` abre o editor com `HOME` isolado e fala com o socket; `s_sweep.py` reprodução, `s_scrub.py` scrub,
  `s_ui.py` UI, `s_video.py` vídeos, `mainthread.py` resumo da thread principal de um `sample`). A raiz do repositório vem do próprio
  arquivo (`TESSEL_REPO_ROOT` a sobrescreve); o diretório de trabalho é `/tmp/perf-inv` (descartável, não parte do produto) ou
  `TESSEL_PERF_INV_WORK`; o guarda de disco do `s_video.py` mede o volume de `WORK`. **Contrato de falha** (revisões da PR #14): op rejeitada,
  espera que estoura (import, prontidão, `settle`), `playback.start` rejeitado, reprodução que nunca toca ou que não desenha, prova de scrub
  com saída não zero ou sem arquivo novo, cabeçalho de thread principal ausente ou sem amostras: cada um vira erro registrado em `errors`
  (com a resposta do editor), **nenhuma linha medida é gravada** para o que não aconteceu, o amostrador de GPU e o editor são sempre
  parados e o arquivo parcial é salvo com `status` = `complete` | `partial` | `failed` | `empty`; o código de saída só é 0 em `complete`.
  Isso vale também para o **preparo da sessão** (`scen.boot`, `scen.seek_to`): `window.activate`, `project.open`, cada `seek` de
  aquecimento ou de origem do arrasto e o `settle` seguinte são conferidos; falha no preparo encerra a sessão como `failed` (nada medido),
  falha antes de uma execução descarta só essa execução. Uma thread de probe de RTT que morre, não amostra ou recebe erro, e um `sample` que
  falha, descartam a fase (o RTT truncado não é gravado). O arquivo de saída do scrub é apagado antes de cada prova e o
  `results/<label>.json` anterior antes de cada sessão.
  Testes sem editor, GPU nem Rust, de qualquer diretório:
  `python3 docs/viewport-performance-results/investigation-2026-11-06/drivers/drivers.spec.py` (80 casos; Python >= 3.9, também com `-O`).
  **Escopo exato da prova:** os drivers reais rodam contra um editor, relógio e subprocessos **falsos** do contrato (`ok`, campos de
  `viewport`); isso mostra que os drivers reagem às falhas, **não** como um editor real se comporta quando `project.open`, `seek` ou
  `settle` falham. A lista `MUTANTS` do spec tem 63 mutantes (28 da primeira rodada, 35 da segunda); cada um apaga uma guarda numa cópia e
  faz um teste nomeado falhar. É prova dessas 63 guardas, **não** promessa de que toda guarda ou linha tenha mutação.
  Os drivers corrigidos **não** foram usados para refazer medições; as tabelas acima vêm dos drivers anteriores.
- `raw-run-logs.txt`: saída bruta das execuções citadas acima, **com anotações** (linhas `####`) que marcam `vid-b` como inválida e abortada e
  `vid-c` como válida para N = 2 e N = 4 e abortada por disco; as linhas brutas ficam intactas.
