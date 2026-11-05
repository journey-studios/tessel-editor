# Validação do parecer do viewport: provas novas e limites

Rodada de validação de [viewport-performance-opus-review.md](viewport-performance-opus-review.md), 2026-11-04, 20:02–22:25 (-03).
Revisor: Opus 5.5. Produção, core e scheduler não foram alterados nem commitados. O parecer original também não foi alterado.

> **Atualização 2026-11-05:** B1–B8 foram tratados em [viewport-bugfix-report.md](viewport-bugfix-report.md). A tabela de bugs abaixo e o texto são **histórico** (estado de 2026-11-04) e não foram reescritos.

**Leia assim:** "retido" é um **protótipo** construído nesta validação. Ele **não está em produção** e os resultados dele não substituem a aceitação do build final ([§ Checks no build final](#checks-no-build-final)).

## Veredito

1. **Os ~25 ms sem dono foram refutados em release.** Eles vêm do draw do GPUI em **debug**: 36 ms por render, ~80% da thread de UI. Em release o draw cai para ~4 ms. Com imagem retida, o caminho entrega 239 quadros/s para 240 Hz de entrada, ciclo de 4,2 ms.
2. **Passar de 60 quadros/s não chega à tela.** A 120 Hz o WKWebView aceitou 237 quadros, mas só houve 122 rAF distintos em 2 s. O rAF é um proxy, mas "120 fps" nos contadores não é imagem a 120 Hz.
3. **Bug novo, reproduzido pela CLI do produto:** o caminho `<video>` desenha **preto** no baseline, nos dois motores e no Chrome com SwiftShader. Corrigindo isso aparecem mais dois problemas: quadro anterior em até ~2/3 das posições e VP8/VP9 preto no WKWebView.
4. **MP4 importado não passa por esse caminho.** O `media.prepare` decodifica para PNG (`derive.rs:369`) e o preview lê a sequência de frames, que deu 0 divergências de imagem. O risco fica em WebM ou vídeo sem decode.
5. **Prefetch não é ganho garantido.** Ajuda quando a entrada cabe na vazão de decode e piora quando não cabe: 1080p a 60 Hz caiu de 34 para 21 quadros/s. Um proxy de 540p venceu, sem prefetch e sem memória extra.
6. **"7/36/68 ms" do Spark não vale como número geral.** O mesmo 3M vai de 34 ms a 263 ms só mudando câmera e overdraw. Em movimento, 1–4 ms segue fora de alcance. Parado com cache de RT, a média fica em 0,4–0,7 ms.
7. **Retenção segura no que foi testado.** Export completo saiu **byte a byte idêntico** ao baseline em 1080p e 4K, SwiftShader e GPU. Perda de contexto em cena e splat, troca de asset, reload e troca de projeto também bateram. **Exceção:** o desenho de RT único do protótipo anterior **gera imagem errada** com duas camadas de cena e não deve ser copiado.

## Matriz

| # | Pergunta | Estado | Prova | Limite |
|---|---|---|---|---|
| 1 | ~25 ms têm dono? | **Refutado em release; atribuído em debug** | Spans por quadro em janela real | Só imagem demo; atribuição ao GPU continua sem prova |
| 1 | Bridge limita? | **Refutado até 240 Hz** com imagem retida | 239 quadros/s, ciclo 4,2 ms | Uma resolução de janela; carga concorrente |
| 1 | `notify` pesa? | **Parcial** | 118 renders/s com entrada de 60 Hz no baseline | O experimento de `notify` seletivo não rodou |
| 2 | Custo de miss de vídeo | **Provado, leitura anterior corrigida** | 13 / 26–28 / 92 ms (720p / 1080p / 4K) | Sintético, uma máquina |
| 2 | Imagem no instante certo | **Provado para sequência; refutado para `<video>`** | Barcode por quadro | AV1 sem diagnóstico |
| 2 | Prefetch | **Refutado como ganho geral** | Trace com ritmo | Protótipo simples |
| 2 | Proxy | **Provado em JPEG/H.264 540p** | 60 quadros/s, 9–12 ms | **PNG 540p, único formato aceito pelo core, não medido** |
| 3 | Spark em movimento | **Medido; 1–4 ms refutado** | CPU e GPU drenada separadas | Sintético; `readPixels` é limite superior |
| 3 | Cache de RT parado | **Provado no protótipo** | 0,4–0,7 ms, 27 passos iguais ao baseline | Pixels só em 100k |
| 4 | Ciclo de vida | **Provado no protótipo** | Scripts em 2 motores; 18 passos no host real | Sem orçamento em bytes; timeout de seek não testado |
| 5 | Hash/pixels | **Provado onde rodou** | 76 grupos idênticos | 3M e 4K em SwiftShader **não rodaram** |
| 6 | Export completo | **Provado** | SHA-256 idêntico, ffprobe, tons | 3M em 4K **não rodou** |
| 7 | Arraste real do GPUI | **Provado sem ponteiro físico** | `dispatch_event` na janela real | Do `NSEvent` até o GPUI não coberto |
| 7 | Seek-to-capture | **Parcial** | ScreenCaptureKit, 18 de 156 tentativas válidas | **Não é fóton** |
| 7 | Seek-to-photon | **Não validado** | — | Exige câmera; [procedimento](#7-arraste-e-seek-até-a-tela) |

## Proveniência

- **Baseline:** snapshot `6cd4c0aa97c1fb54`, feito às 20:04. O repositório **não tem commits**. Manifesto com SHA-256 de 150 arquivos em `scripts/viewport-validation/results/snapshot-6cd4c0aa97c1fb54.MANIFEST.sha256`; hashes de binários e variantes em `results/PROVENANCE.json`.
- **Produção em trânsito:** ao fim, `compositor.js`, `index.js`, `assets.js`, `session.rs` e `viewport_host.rs` já diferiam do snapshot, com a edição do outro agente em curso. **Nada aqui mede esse código.**
- **Variantes** (cópia do snapshot; `build-runtimes.mjs`):

| Variante | O que é |
|---|---|
| `baseline` | Build puro. Difere do `dist` do snapshot só em comentários de caminho (diff vazio sem eles) |
| `baseline-trace` | Baseline com spans, sem mudar comportamento |
| `retained`, `retained-trace` | Textura, material, texto (LRU 32), rig e RT por camada, com geração em `AssetStore` |
| `retained-rtc` | Retido com cache de quadro de cena |
| `retained-singlert` | RT único, como o protótipo anterior, para expor o bug |
| `baseline-videofix`, `baseline-videofix-rvfc` | Hipóteses do bug de `<video>` |

- **Binários instrumentados:** debug e release do `crates/desktop` copiado, em `patch-host.py`. A cópia ignora movimentos sem botão pressionado, para o ponteiro real da máquina não contaminar o gesto ([B6](#bugs-achados)).
- **Máquina:** Mac15,6, M3 Pro, 18 GiB, Darwin 25.6, WebKit do sistema, Chrome 156 (ANGLE/Metal e SwiftShader), Three r187, Spark 2.4.0, Rust 1.99.1, FFmpeg 7.1.1.
- **Carga:** outro agente ativo durante toda a rodada. Em 232 amostras, load de 1 min p50 5,4, p95 15,5, máx 17,8. As linhas têm a carga no bruto. Trate diferenças pequenas como ruído.
- **Relógios:** `performance.now` tem quantum de 1 ms no WebKit, 0,1 ms no Chrome. Spans entre processos (epoch) têm erro de ~±1 ms: valores como `eval→recv = -0,7` são esse erro, não causalidade invertida.

## Resultados por item

### 1. Atribuição dos ~25 ms

Gesto de 2 s sobre 400 quadros, janela real de 1440×876, imagem demo. Medianas de 2–3 repetições quentes, sem observar durante o gesto. Tudo vem de eventos por quadro lidos uma vez, depois do gesto.

| Build | Runtime | Entrada | Quadros/s | Ciclo p50/p95 (ms) | Draw JS (ms) | Draw GPUI p50 (ms) | Renders/s |
|---|---|---|---:|---|---:|---:|---:|
| debug | baseline | socket 30 Hz | 26,9 | 44,9 / 49,9 | 7,5 | 35,0 | 21 |
| debug | baseline | socket 120 Hz | 22,2 | 48,7 / 51,2 | 8,0 | 35,9 | 34 |
| debug | retido | socket 120 Hz | 42,0 | 9,8 / 45,2 | 0,0 | 36,0 | 41 |
| release | baseline | socket 30 Hz | 30,0 | 33,3 / 36,8 | 8,5 | 4,0 | 53 |
| release | baseline | socket 120 Hz | 84,6 | 11,6 / 16,5 | 8,0 | 4,1 | 114 |
| release | baseline | socket 240 Hz | 82,9 | 10,1 / 16,6 | 9,0 | 4,4 | 122 |
| release | retido | socket 60 Hz | 59,9 | 16,6 / 20,1 | 0,0 | 3,8 | 73 |
| release | retido | socket 120 Hz | 119,9 | 8,3 / 10,0 | 0,0 | 3,8 | 121 |
| release | retido | socket 240 Hz | 239,2 | 4,2 / 8,8 | 0,0 | 4,3 | 113 |
| release | retido | GPUI 120 Hz | 118,9 | 8,3 / 9,2 | 0,0 | 4,2 | 122 |
| release | retido | GPUI 240 Hz | 236,9 | 4,2 / 9,4 | 0,0 | 4,3 | 112 |

Spans em release (p50, ms): `plan` <0,01, chave <0,02, pedido 0,02, `evaluate_script` na UI 0,02–0,05, IPC 1–1,8 (p95 2–6), `on_ipc` 0,01–0,05, despacho de input GPUI 0,08–0,15, canal IPC→UI ~0.

- **Debug:** a thread de UI fica ~80% em draw do GPUI. O `post→IPC` sobe para ~32 ms (p50) com entrada alta porque a resposta espera o draw terminar. **Esse é o dono da sobra que o parecer deixou em aberto.** Build debug não serve para decidir desempenho.
- **Release:** o core e a bridge não aparecem como gargalo com imagem. O limite do baseline é o draw JS de 8–9 ms. O do retido é a própria entrada, até o que testei.
- **Observador:** poll de 10 ms ou 250 ms quase não muda os quadros/s (a 120 Hz: baseline 84,6 sem poll, 93,8 e 91,7 com poll; retido 119,9 nos três). **Mas muda a cadência do GPUI no baseline:** 114 renders/s sem poll, 31 com poll. Causa não investigada. Por isso os números de decisão são os sem observador.
- **rAF não atrapalhou:** com a marca de rAF desligada, o resultado ficou igual (por exemplo 118,95 contra 118,93).
- **`sample` em release, retido, GPUI 120 Hz:** 1447 de 2016 amostras da thread principal em `metal_frame`/draw do GPUI. Layout (taffy) e prepaint dominam dentro dele. `evaluateJavaScript` e mensagem de IPC somaram ~1,4%. O `sample` interfere, então vale como proporção, não como tempo. A amostra do baseline não cobriu o gesto e foi descartada.
- **Teto de tela:** `live-raf`, 2 s: 237 quadros aceitos e 122 rAF distintos a 120 Hz; 120 e 120 a 60 Hz. O rAF do WKWebView mede ~16,7 ms, enquanto o GPUI redesenha a cada 8,3 ms. Metade do trabalho acima de 60 Hz não tem como ser apresentada.

**Menor experimento para o restante, sem otimização especulativa:**
- **`notify` seletivo:** tirar `cx.notify()` de `viewport_flush` quando o texto do cabeçalho não mudou, em cópia, e comparar renders/s, `gpui_draw` e latência. Há sinal: 118 renders/s com 60 Hz de entrada no baseline, duas redesenhadas por tick.
- **Gate por rAF:** atrasar `frame.done` interativo até o próximo rAF e medir quadros aceitos contra rAF distintos.
- **GPU:** rodar Metal System Trace no GPU process. Sem ele, nenhum número aqui atribui tempo à GPU. Não tentei, porque o disco estava no limite.

### 2. Vídeo sem cache

**Capacidades reais (`results/caps-*.json`).**

| Recurso | WKWebView | Chrome 156 |
|---|---|---|
| `canPlayType` H.264, HEVC, AV1, VP8, VP9 | `probably` | `probably` |
| WebCodecs `VideoDecoder` (1080p) | H.264, HEVC, VP8, VP9, AV1 | H.264, HEVC, VP9, AV1; **sem VP8** |
| `requestVideoFrameCallback` | sim | sim |
| `fastSeek` | sim | não |
| `ImageDecoder` | **não** | sim |
| Timer query WebGL | **não** | sim |

`canPlayType` não garante textura: VP8/VP9 dizem `probably` e chegam pretos no WebGL do WKWebView. WebCodecs não foi medido em decode, só em `isConfigSupported`.

**Sequência de frames, miss (WKWebView, 960×540, 100 quadros por trace).** Caminho do MP4 importado.

| Fonte | Wall p50/p95 (ms) | Fetch | Decode | Upload+draw | Hit (ms) |
|---|---|---:|---:|---:|---:|
| PNG 720p | 13 / 14 | 1 | 6 | 6 | 2 |
| JPEG 720p | 14 / 14 | 1 | 5 | 7 | 2 |
| PNG 1080p | 28 / 32 | 3–4 | 13–14 | 11 | 3–5 |
| JPEG 1080p | 26 / 28 | 3 | 11 | 12 | 2–4 |
| JPEG 4K | 92 / 102 | 10 | 40 | 43 | 2–10 |
| JPEG 540p | 9 / 10 | 1 | 3 | 5 | 2 |

- Os "~21 ms" do parecer são um ponto dessa curva, não uma constante.
- O WKWebView gasta ~2× o Chrome. Em 1080p PNG, Chrome fica em 13,3 ms (1,6 + 8,5 + 2,8). A maior diferença é o upload.
- **O preview é 960×540:** decodificar 1080p ou 4K é o desperdício real.

**`<video>` com a correção de B1 (WKWebView), timing de 100 quadros.**

| Fonte | Forward | Random | Scrub |
|---|---:|---:|---:|
| H.264 GOP 24, 1080p | 18 | 26 (p95 32) | 19 (p95 32) |
| H.264 intra, 1080p | 18 | 19 | 18 |
| HEVC GOP 24, 1080p | 14 | 20 | 16 |
| H.264 intra, 540p | 7 | 7 | 8 |
| H.264 GOP 24, 4K | 54 | 81 (p95 114) | 59 (p95 104) |

Sem a correção os tempos eram semelhantes, mas de imagem preta (`results/video-formats-unfixed.txt`, preservado).

**Imagem no instante correto (barcode de 10 bits em cada quadro, lido em probes).**

| Fonte | Tempo | WKWebView (errados) | Chrome (errados) |
|---|---|---:|---:|
| Sequência PNG/JPEG, todas | índice | **0/60** | **0/60** |
| Sequência com prefetch | índice | **0/100** nos 3 traces | não rodou |
| `<video>` sem correção | `f/24` | **59–60/60, preto** | **59–60/60, preto** |
| H.264 ou HEVC, com correção | `f/24` | **38–44/60**, sempre o anterior | H.264 GOP 24: 18–24/60; intra: 0/60 |
| H.264 ou HEVC, com correção | `(f+0,5)/24` | **0/60** | **0/60** |
| VP9 ou VP8, com correção | qualquer | **preto, 40/40** | VP9 `(f+0,5)/24`: 0/60 |
| AV1 1080p, com correção | qualquer | **37–40/40**, parte preta | 19–25/40, saltos grandes, sem diagnóstico |

`requestVideoFrameCallback` antes do upload não resolveu nenhum caso e custou 100+ ms.

**Trace com ritmo (entrada fixa, um quadro em voo, o mais novo vence; WKWebView).**

| Fonte | 60 Hz: quadros/s | 60 Hz: latência p50/p95 | 30 Hz: latência p50/p95 | Footprint web/GPU (MB) |
|---|---:|---|---|---|
| JPEG 1080p | 34,3 | 37 / 45 | 28 / 31 | 327 / 260 |
| JPEG 1080p + cache de 64 | 33,9 | 66 / 78 | 30 / 60 | 1090 / 641 |
| JPEG 1080p + prefetch | **20,8** | **93 / 113** | 17 / 28 | 1007 / 351 |
| JPEG 720p | 60,1 | 16,7 / 19 | 16,7 / 18 | 189 / 181 |
| JPEG 720p + prefetch | 59,7 | 11 / 15 | 11 / 13 | 600 / 375 |
| **JPEG 540p** | **60,2** | **11,3 / 14** | 11,7 / 15 | **137 / 151** |
| JPEG 540p + prefetch | 60,2 | 8,3 / 11 | 7,7 / 10 | 335 / 247 |
| H.264 intra 1080p (com correção) | 58,8 | 25 / 39 | 18 / 22 | 95 / 134 |
| H.264 intra 540p (com correção) | 60,2 | 9,3 / 13 | 8,7 / 11 | 75 / 121 |

Linhas de forward. Scrub e random estão em `results/video-paced.txt`.

- **Prefetch em scrub ou random** piora a cauda: 1080p scrub a 30 Hz foi de p95 30 para 102 ms; 720p random a 60 Hz de 60 para 50 quadros/s.
- **O orçamento de 512 MB não limitou a memória real:** web ~1,1 GiB mais GPU ~0,6 GiB. O limite conta bitmaps, não o que o motor retém.
- **Conclusão:** proxy de 540p é o candidato. Prefetch só depois, e só com regra para não competir com o quadro atual.
- **Restrição de produção:** `asset.add` recusa sequência que não seja PNG (`invalid_asset: Not a PNG file`), e o decode gera PNG. Portanto o **JPEG 540p medido não é aplicável hoje sem mudar o core**, e PNG 540p ficou sem medir. Pela leitura de `backend.rs:146`, o descritor do preview usa os frames decodificados; não vi `media.proxy` ligado a ele.

### 3. Spark

Média em ms, WKWebView, 960×540. O modo CPU deixa a GPU para depois. O modo drenado faz `readPixels` 1×1 depois de cada quadro, um limite superior que inclui IPC. `render` no modo CPU paga GPU do quadro anterior, por isso os dois não somam de forma simples.

| Caso | Base CPU | Retido CPU | Retido só CPU (drenado) | GPU drenada p50 | Sort p50 (drenado) |
|---|---:|---:|---:|---:|---:|
| 100k, órbita longe | 11,6 | 8,4 | 8,6 | 7 | 7 |
| 100k, dentro (`room`) | 9,6 | 7,7 | 7,7 | 4 | 7 |
| 1M, órbita longe | 49,9 | 50,8 | 11,8 | 52 | 11 |
| 1M, câmera perto | 89,2 | 79,7 | 12,3 | 61 | 11 |
| 1M, dentro (`room`) | 20,1 | 16,7 | 12,2 | 7 | 11 |
| 1M, denso e perto | 170,4 | 155,9 | 12,4 | 162 | 11 |
| 3M, órbita longe | 131,7 | 139,8 | 22,4 | 136 | 22 |
| 3M, câmera perto | 272,0 | 263,1 | 21,8 | 246 | 21 |
| 3M, dentro (`room`) | 35,9 | 33,9 | 20,3 | 13 | 20 |
| 1M órbita, 1080p (baseline) | — | — | 17,1 (base) | 86 | 12 |
| 1M órbita, 4K (baseline) | — | — | 24,1 (base) | 236 | 11 |

- **Refuta a leitura anterior:** "7/36/68" era um asset e uma câmera. Overdraw manda: com 3M, câmera perto custa ~8× a câmera dentro de `room`.
- **Parte CPU real:** sort de ~7 / 11 / 21 ms em 100k / 1M / 3M. Retenção economiza ~3–4 ms de composição e nada de sort. **Nenhum Spark em movimento fica em 1–4 ms.**
- **Parado sem cache** repete o sort: 1M 47,6 ms, 3M 130,2 ms.
- **Parado com cache de RT (`retained-rtc`):** média 0,4 / 0,6 / 0,7 ms em 100k / 1M / 3M, p99 ≤ 2 ms, drenado 1–2 ms, 60 quadros/s.
- **Memória (footprint web/GPU, MB):** 145/216, 350/494, 860/502. Carga 20–36 / ~100 / ~280 ms. Frio 40 / 50 / 85 ms.
- **Não medido:** MSAA de novo, alocação por quadro (só footprint), Metal System Trace, captura real. Assets sintéticos: `room` tenta imitar scan, sem garantia.

### 4. Ciclo de vida

Scripts idênticos em `baseline-trace`, `retained-trace` e `retained-rtc`, comparados passo a passo em WKWebView e Chrome/Metal (`results/lifecycle.txt`).

| Script | Passos | Cobre | Retido e RTC vs baseline |
|---|---:|---|---|
| `image` | 11 | Perda de contexto, troca com mesmo id, release, reload | Iguais |
| `box` | 10 | **Cena**: perda (2×), troca de cena, tempo, câmera | Iguais |
| `splat` 100k e 1M | 17 cada | **Splat**: perda (2×), troca com mesmo id, release de cena e de splat | Iguais |
| `rtc` | 27 | Câmera, fov, opacidade, blend, transform, crop, tempo com e sem animação, tamanho, geração, perda, PNG | Iguais; 9 acertos de cache |
| `two` | 6 | Duas camadas de cena, mesma cena duas vezes, dois splats | Iguais; **RT único: 4 de 6 diferem** |
| `cancel` | 18 | Cancelar, superseder durante sort, abort entre camadas, release durante frame e load, load duplo, perda durante frame | Iguais |
| `limits` | 9 | 1200 textos, 240 vídeos, 300 cenas, blends, tamanhos, churn | Sem crescimento ilimitado |

**Host real** (release, `TESSEL_VIEWPORT_INSPECT=1`, projeto com cena, splat, imagem, vídeo e texto, `results/live-lifecycle.jsonl`): 18 passos com baseline e retido **iguais**. Perda de contexto (2×) com redesenho automático, hash igual ao de `render.frame` explícito, edição e volta de câmera, `monitor.reload`, troca de projeto e volta. O status fica `failed` enquanto o contexto está perdido e volta sozinho.

**Limites medidos no protótipo.**
- Texto para em 32 texturas após 1200 distintos. Programas ficam em 2. RT em 1 após 60 tamanhos diferentes.
- 16 stills 4K retidos (só WKWebView; o passo falhou no Chrome com o erro de fetch descrito em Limites): web 1525 MB e GPU 830 MB, contra 930 e 1133 no baseline.
- Depois do release, a web segue em 1175–1455 MB, contra 677 no baseline. Isso pode ser GC tardio; não provei se volta.

**Falta.** O protótipo **não tem orçamento em bytes**, só o limite de 16 assets da sessão: o risco que o parecer apontou segue aberto. Timeout de seek de `<video>` não foi exercitado.

### 5. Hash e pixels

Quadros **fora de ordem**, com repetições, `inspect` e probes ligados, `baseline` contra `retained` (`results/hash.txt`).

| Stack | Grupos | Idênticos | Diferentes | Cobertura |
|---|---:|---:|---:|---|
| WKWebView | 30 | 30 | 0 | Gauss e `room`, 1M e 3M, em 960×540, 1080p, UHD; 2D e cena |
| Chrome/Metal | 30 | 30 | 0 | Mesma |
| Chrome/SwiftShader | 16 | 16 | 0 | 1M em 960×540 e 1080p; 2D e cena até 1080p |

- **Orçamento em SwiftShader:** 1M custou ~11 s por quadro. Por isso **3M e 4K em SwiftShader não rodaram**. Não há conclusão sobre esses casos.
- **Repetição estável:** o mesmo quadro em posições diferentes da lista deu o mesmo hash em todos os grupos.
- **PNG:** 42 pares baseline~retido com diferença zero.
- **`retained-rtc`** só teve pixels comparados em 100k e 960×540 (script `rtc`).

**Entre stacks (`results/cross-stack.json`).** Hash igual entre GPUs **não é exigido**.

| Par | n | PSNR mín | Erro abs. médio máx (0–255) |
|---|---:|---:|---:|
| WKWebView ~ Chrome/Metal (mesma GPU) | 32 | 89,3 dB | 0,0 |
| WKWebView ~ SwiftShader | 12 | 43,2 dB | 0,141 |
| Chrome/Metal ~ SwiftShader | 12 | 43,2 dB | 0,141 |

Tolerância proposta entre stacks: ≥ 40 dB e erro médio ≤ 1, abaixo do que medi com folga e acima dos 26 dB de `measure-render.py`. O motivo é que arredondamento de filtro muda poucos níveis, e conteúdo errado cairia bem abaixo disso. **Só vale para esta GPU e o software rasterizer**; outra GPU física não foi testada.

### 6. Export completo

Projeto de 48 quadros a 24 fps com cena, splat, imagem, vídeo de frames, texto, tom e MP4 com som próprio. Sessão nova por execução, via `tessel-render` do snapshot (`results/export-*.json`).

| Stack | Preset | Baseline (s) | Retido (s) | SHA-256 do arquivo |
|---|---|---|---|---|
| SwiftShader | 1080p | 32,3 / 53,2 | 22,2 / 29,4 | `f74c8dbefc…` em **todos os 4** |
| SwiftShader | 4K | 75,3 / 82,6 | 68,5 / 64,1 | `db943893e6…` em **todos os 4** |
| GPU | 1080p | 8,8 / 8,5 | 8,3 / 8,3 | `ec468763fe…` em **todos os 4** |
| GPU | 4K | 27,8 / 29,1 | 30,5 / 28,9 | `82b0cc87ed…` em **todos os 4** |
| GPU, **splat 3M** | 1080p | 25,3 | 24,3 | `20f405b38e…` nos 2 |

- **Ffprobe em todos:** H.264, `yuv420p`, 24/1, 48 quadros, vídeo e AAC com 2,000 s, PTS monotônicos de 0 a 1,958333.
- **Áudio em todos:** 330 Hz só na segunda metade (0 → 0,0882) e 440 Hz nas duas (0,353 / 0,354). É prova espectral, não audição.
- **Tempos** não sustentam ganho de retenção no export: carga variou muito.
- **Cancelar e recuperar (baseline e retido, nas 3 baterias):** cancelado no quadro 3, sem arquivo nem sobra. Reexportar no mesmo destino deu 48/48 quadros iguais.
- **Snapshot imutável:** editar logo após iniciar deu 48/48 iguais ao original, e a exportação seguinte diferiu nos 48. Isso prova que o teste enxerga a edição.
- **Não rodou:** 3M em 4K, 3M em SwiftShader, sequência longa para memória.

### 7. Arraste e seek até a tela

**O que cada camada prova, sem misturar:**

| Camada | Como | Prova | Não prova |
|---|---|---|---|
| Entrada GPUI real | `vv_drag` na cópia: `Window::dispatch_event` em timer, sem clique do SO | Hit test, drag, `on_drag_move`, `Command` e WebView reais | `NSEvent` → GPUI, mão humana |
| Spans | Marcas Rust e JS, epoch | Onde o tempo passa na CPU | GPU, tela |
| rAF | `requestAnimationFrame` após `frame.done` | Houve rendering update depois | Que aquela imagem saiu |
| Captura | ScreenCaptureKit, barcode, `displayTime` | O compositor entregou aquele código à captura | Scanout e panel |
| **Fóton** | Câmera | **Nada aqui** | — |

Resultado da captura, entrada GPUI, PNG 720p, release. **`state` é a execução do comando no host**:

| Runtime | Caso | Corridas | Amostras | state→post | state→captura p50 (faixa) | p95 | rAF→captura |
|---|---|---:|---:|---:|---|---:|---:|
| baseline | Hit, 30 Hz | 3 | 43 | 2,0 | 14,6 (10,6–16,1) | 24,1 | 11,8 |
| baseline | Hit, 60 Hz | 5 | 70 | 1,8 | 12,0 (10,2–12,7) | 18,3 | 9,5 |
| baseline | Miss, 60 Hz | 5 | 476 | 18,6 | 29,9 (23,7–39,5) | 44,6 | 11,1 |
| retido | Hit, 30 Hz | 1 | 14 | ~0 | 15,1 | 24,1 | 12,2 |
| retido | Hit, 60 Hz | 1 | 14 | ~0 | 8,1 | 18,4 | 8,0 |
| retido | Miss, 60 Hz | 3 | 292 | 13,1 | 25,8 (21,9–26,1) | 36,3 | 11,9 |

- **Limite forte:** o ScreenCaptureKit entregou **zero quadros em 138 de 156 tentativas**, em ambas as variantes, sem relação clara com elas. Só 18 corridas são válidas. Com isso, **não comparo baseline e retido em hit**.
- **Leitura útil:** do rAF até o `displayTime` da captura há ~8–12 ms. Essa etapa não aparece em `render_ms` e não é fóton: pode incluir atraso da própria captura.

**Procedimento para seek-to-photon real (não executado, exige hardware e mão humana):**
1. Build release final, HOME isolado, projeto com faixa codificada por quadro (`make-assets.py`), PNG 720p e hit/miss separados.
2. Câmera de ≥ 240 fps mirando painel e mouse, com brilho travado e ProMotion registrado. Sem ProMotion, usar 60 Hz fixo.
3. Pessoa arrasta o ruler com o mouse real. Sem cliques por script.
4. Marcar o início do movimento no vídeo e ler o código na tela quadro a quadro, ≥ 30 gestos por caso.
5. Reportar movimento→fóton com resolução de 1/fps (~4 ms a 240 fps) e cruzar com os spans. Câmera a 120 fps não resolve a diferença entre 8 e 12 ms.

## Hipóteses anteriores, revisadas

| Leitura anterior | Agora |
|---|---|
| ~25 ms sem dono | **Refutada em release**; em debug é draw do GPUI |
| Imagem WK 7,7 → ~0,8 ms | **Confirmada em direção**: 8–9 → 0,2 ms em janela real |
| `gl.finish` pode não sincronizar | Não reexaminada |
| Hash igual em 10 quadros, 3 motores | **Ampliada**: 1M, 3M, 1080p, UHD (limites em §5) |
| Perda de contexto só em imagem | **Ampliada**: cena, splat e host real |
| Vídeo miss ~21 ms | **Refinada**: depende de resolução |
| Prefetch deve recuperar a maior parte | **Refutada como regra** |
| Spark 7/36/68 ms | **Refutada como número geral** |

## Bugs achados

| # | Onde | Sintoma | Reprodução | Estado |
|---|---|---|---|---|
| **B1** | `runtime/src/assets.js`, `videoTexture`, com Three r187 | `<video>` desenha **preto**, em WKWebView, Chrome/Metal e SwiftShader. Causa provável: dimensão lida de `video.width` (0). Definir `width`/`height` a partir de `videoWidth`/`videoHeight` faz a imagem aparecer | `tessel-render`: `asset.add kind=video path=x.webm width height rate frames duration`, `render.frame` → `mean=[0,0,0]`; com `baseline-videofix` → mean ~120–128 | **Reproduzido na CLI** |
| **B2** | Tempo do `<video>` | Depois de B1, `seeked` em `f/24` mostra o quadro **anterior**: ~2/3 no WebKit | Run `vf:*:verify`; `(f+0,5)/24` zera | Confirmado com correção aplicada |
| **B3** | `FORMATS` em `index.js` declara `webm` | VP8/VP9 **preto** no WKWebView mesmo com correção; AV1 parcial. `requestVideoFrameCallback` não ajuda | Run `fixq` | Confirmado |
| **B4** | `<video>` H.264 no WebKit | Parada de ~1 s (latência p99 ~1016 ms; 91/120 quadros) em trace forward | `p:h264g24_1080:forward:30` | **Causa não diagnosticada** |
| **B5** | Protótipo anterior e `retained-singlert` | RT único por compositor: duas camadas de cena dão imagem **errada** | Script `two` | **Só protótipo.** Não copiar |
| B6 | `on_drag_move` do ruler | Movimento sem botão pressionado altera o playhead durante drag ativo. Observado com ponteiro real durante injeção | Primeira execução de `vv_drag`: playhead salta para ~527 | Possível só sob injeção; **não prova bug em uso real** |
| B7 | `core::ops`, `asset.add` | Sequência aceita só PNG, embora o runtime decodifique JPEG | `asset.add ... pattern=...jpg` → `Not a PNG file` | Restrição real para proxy |
| B8 | `renderFrame` com perda durante quadro | Falha com `render_failed`, não `context_lost` | Passo `lose_during_frame` | Menor; recupera depois |

## Prioridade recomendada

1. **Medir sempre em release** e parar de usar debug como referência.
2. **Corrigir B1 e B2 ou retirar `webm` de `FORMATS`.** Hoje o produto promete formato que sai preto. Decidir B3 junto.
3. **Terminar retenção com RT por camada (B5) e orçamento em bytes.** Exigir os checks abaixo.
4. **Limitar a 60 quadros/s por rAF** (experimento em §1), para não gastar trabalho que não chega à tela.
5. **Proxy de 540p no preview**, medindo primeiro PNG 540p. Se o formato importar, decidir B7. **Antes de prefetch.**
6. **Cache de RT parado** com chave de geração. Em movimento, MSAA/LOD é decisão de produto.
7. **Seek-to-photon com câmera**, só depois de 1–5, para medir o que vai ficar.

## Checks no build final

Protótipo não aceita produção. No build do outro agente, com snapshot próprio, rodar:

```sh
cd scripts/viewport-validation
./snapshot.sh                                     # novo fingerprint e manifesto
python3 patch-host.py <cópia>                     # adaptar se os spans do outro agente já existirem
node build-runtimes.mjs <snap> $TEV/rt            # baseline e trace do NOVO código; retirar patches de retenção se já embutida
python3 make-assets.py $TEV/assets
python3 runs.py && ./bench.sh wk $TEV/runs/lifecycle.json out.jsonl
```

| Check | Critério |
|---|---|
| `run-live.py matrix --bins release` | Sem regressão de ciclo e `state→post` contra este baseline, com o relatório declarando a carga |
| `lifecycle` em WK e Chrome, inclusive `two` | Passos iguais ao baseline do mesmo snapshot; **duas camadas corretas** |
| `run-live.py lifecycle` | Perda, reload e troca de projeto iguais; status `ready` |
| `hash` em WK, Metal e SwiftShader | 0 diferentes; declarar o que não rodou |
| `run-export.py` | SHA-256 igual ao do baseline por stack, ffprobe, tons, cancelar e recuperar, snapshot imutável |
| `limits` | Textura limitada **em bytes**; footprint volta depois do release |
| Vídeo `verify` | 0 divergências no que o produto anunciar |
| Testes do repositório | `./scripts/runtime test`, `./scripts/cargo test --workspace --locked` |
| Câmera | Procedimento de §7 |

Antes de builds, conferir disco e usar `MBX_GC_MAX_TOTAL_SIZE=64GiB`.

## Limites e incidentes desta rodada

- **Testes do repositório não rodaram aqui:** só harnesses novos sobre o snapshot.
- **Uma máquina, assets sintéticos, um tamanho de janela, carga concorrente.** Sem GPU de outro fabricante.
- **Janelas próprias** abriram e podem ter tomado foco durante as medições.
- **Incidente de disco.** No primeiro build release o disco caiu para **1,9 GiB livres** e o GC do mbx despejou **1,4 GiB de objetos do cache compartilhado**. Registrou "abaixo do mínimo de 4 GiB", apesar de eu ter passado 1 GiB para esse limite. O `target` do repositório ficou intacto (conferido). O script de guarda de disco que escrevi **não foi iniciado** nessa hora. O cache compartilhado do mbx foi de 6,4 para 10 GiB no período: ~2,2 GiB registrados nos meus builds, o resto sem atribuição, pois outro agente também compilava. Não apaguei esses objetos e o total fica acima do teto padrão de 8 GiB do wrapper. **Quem rodar `./scripts/cargo` sem o teto de 64 GiB pode disparar GC.**
- **Fetch no Chrome:** hashes de `image4k`/`image2` falharam com `Failed to fetch` em duas passadas, com 2,0–2,4 GiB livres, e passaram com 5,6 GiB. A relação com o disco é só correlação.
- **Harness corrigido durante a rodada:** gesto de 100 quadros limitava a taxa a 50/s (descartado, `live-matrix-span100-discarded.jsonl`, fora do git); `locate` da captura; parsing do ffprobe. Nenhum número desta página vem dessas execuções.
- **Limpeza:** apaguei só o meu: assets, exports, HOMEs, PNGs e meu target gerenciado. Ficam em `/tmp/tev`: JSONL brutos, snapshot, variantes e binários instrumentados. O durável está em `scripts/viewport-validation/results/`. `/tmp/tvb` não foi tocado.

## Reproduzir

```sh
cd scripts/viewport-validation; export TEV=/tmp/tev
./snapshot.sh                                               # imprime o diretório do snapshot
node build-runtimes.mjs "$TEV/snap/<fp>" "$TEV/rt"
python3 make-assets.py "$TEV/assets"; python3 runs.py       # ~1 GiB; conferir o disco antes
swiftc -O vv-wk.swift -o "$TEV/bin-bench/vv-wk"; swiftc -O capture-latency.swift -o "$TEV/bin-bench/capture-latency"
# host instrumentado: cópia em $TEV/inst, patch-host.py, build debug e release, binários em $TEV/bin-inst/{debug,release}
python3 run-live.py matrix --out raw.jsonl --span 400 --reps 3
python3 run-live.py lifecycle --out life.jsonl
./bench.sh wk "$TEV/runs/video-formats.json" v.jsonl        # também: metal, sw; spark, lifecycle, hash, video-paced
python3 run-export.py --out e.jsonl --work "$TEV/exp" --gl swiftshader
python3 run-capture.py --out c.jsonl
python3 summarize-live.py raw.jsonl; python3 summarize-bench.py video v.jsonl
```

`/tmp/tev/fx` (fixtures do proof) vem de `$TEV/snap/<fp>/bin/tessel-fixtures /tmp/tev/fx`, usando o binário copiado para o snapshot.
