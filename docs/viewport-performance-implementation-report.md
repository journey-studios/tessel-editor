# Desempenho do preview: relatório de implementação

Entrega: retenção segura de GPU no runtime, telemetria do host e harness durável para o preview do Tessel Editor.
Retoma uma execução anterior que parou por falha de conexão; nada foi recomeçado nem apagado: o código existente foi
auditado, o que faltava foi fechado e tudo foi validado de novo no build final (hashes na seção [Build](#build-e-proveniência)).

**Leitura em uma frase.** Com o quadro quente, a CPU do runtime cai de ~2–40 ms para ≲ 0,5 ms em imagem, texto e cena sem
splat, e o quadro com a GPU drenada fica em ≈ 1–1,7 ms (limite superior, inclui IPC), com pixels idênticos aos do
caminho anterior. Isso **não** vale para o primeiro quadro, para vídeo com *cache miss* nem para Spark em movimento, e
nenhum desses números é tempo de GPU nem foto na tela.

## Escopo

Dentro: caches de preview (textura de imagem/frame, texto, material/programa, rig de cena, alvo por camada), telemetria
(`TESSEL_VIEWPORT_TRACE`), harness em `runtime/scripts/perf`, testes e docs. Fora: áudio/jobs/UI (outro agente), grafo
incremental, troca de Three.js/Spark, qualquer correção de WebM (registrada como limite). O protótipo `/tmp/tvb` não foi
copiado: o `sceneCache` dele não tinha geração e o cache de texto era ilimitado; aqui ambos têm dono, limite e geração.

## Previsto pelo Opus × medido agora

O Opus mediu um snapshot anterior (`6cd4c0aa97c1fb54`) e um protótipo; os números abaixo são **desta** implementação.

| Afirmação do Opus | Medido aqui (build final) | Leitura |
|---|---|---|
| 2D quente, cache de texto/programa: 1–4 ms de CPU | CPU 0,2–0,6 ms (imagem, texto, imagem+texto, cena); com GPU drenada 0,95–1,7 ms (Chrome GPU) e 1,1–1,7 ms (WebKit) | Melhor que o previsto na CPU; a faixa 1–4 ms vale como **drenado**, não como tempo de GPU |
| Chrome: imagem 2,4 → 0,19–0,28; texto 2,8 → 0,25; cena 5,0 → 0,47 (protótipo) | Chrome GPU: imagem 2,65 → 0,20; texto 2,24 → 0,21; cena 5,17 → 0,22 (CPU, ms) | Compatível; a cena aqui é um pouco menor por haver RT por camada com chave e geração |
| Spark: sem garantia de 1–4 ms | 100k: 9,4 → 7,6 (Chrome GPU), 9,8 → 7,2 (WebKit); 1M: 15,1 → 13,2 / 14,9 → 12,0 (CPU). Drenado 1M: 37–41 ms | **Confirmado**: ganho de ≈ 20 %, longe de 1–4 ms; rig com splat nunca é retido |
| Vídeo com *miss* continua lento | `video-miss`: 13,9 → 12,5 (WebKit), 9,9 → 8,6 (Chrome) | **Confirmado**, dominado por decodificar |
| GPUI debug 36 ms × release ~4 ms | Janela real: debug `submit→done` 31–33 ms (transit 5–20 ms); release 10–12 ms (transit 0,3–3,4 ms) | **Consistente**: o extra de debug está no `transit` (host), não no runtime; o `render` do GPUI em si não foi medido |
| Ponte sem gargalo, `accepted` ≠ mostrado | `bridge` WebKit: ida e volta média 0,18–0,35 ms (p95 ≤ 0,9) | Confirmado; ver "o que `accepted` não prova" |
| PNG 540p core compat sem medida | **Não medido** | Lacuna (G7) |

## Resultados do harness (`runtime/scripts/perf`)

Todos com N ≥ 300 quadros por execução, 3 passes, ordem A/B/A alternada entre `current` (retenção ligada), `legacy`
(`retain:false`, mesmo bundle) e `baseline` (bundle construído do código anterior). Tabelas completas, com
média/p50/p95/p99, quantum do relógio e carga: [`viewport-performance-results`](viewport-performance-results/).

**Quatro medidas distintas.** `render` = CPU do runtime no relógio da página (passo de 1 ms no WebKit). `wall` = o mesmo
mais o que ocorre antes do *rAF*. `wall+drain` = o quadro seguido de leitura de 1×1 pixel: limite superior de "a GPU
terminou", inclui IPC. *rAF*/foto: não medidos nesta camada (ver `frame.presented` e G5). **Frio** = primeiro quadro.

WebKit (WKWebView, o motor do editor), `--drain read1x1`, ms, médias (frio é um quadro só):

| Caso | `render` atual | `render` `legacy` | `render` baseline | `wall+drain` atual | `wall+drain` `legacy` | Frio atual / `legacy` |
|---|---|---|---|---|---|---|
| imagem | 0,32 | 7,87 | 8,44 | 1,10 | 10,7 | 18 / 18 |
| imagem 4K | 0,41 | 30,7 | 30,8 | 1,46 | 41,1 | 58 / 62 |
| 2 imagens | 0,37 | 37,2 | 36,2 | 1,46 | 47,8 | 91 / 68 |
| texto | 0,58 | 4,88 | 4,98 | 1,61 | 6,02 | 11 / 10 |
| texto mudando | 1,35 | 3,64 | 3,19 | 1,74 | 4,37 | 11 / 10 |
| imagem+texto | 0,57 | 8,44 | 8,60 | 1,66 | 9,87 | 20 / 21 |
| vídeo, quadro em cache | 0,34 | 2,18 | 2,23 | 1,16 | 3,33 | 12 / 12 |
| vídeo, *miss* | 12,5 | 15,3 | 13,9 | 13,6 | 16,8 | 13 / 12 |
| cena (sem splat) | 0,41 | 8,25 | 6,36 | 1,61 | 10,96 | 17 / 16 |
| Spark 100k | 7,17 | 9,68 | 9,82 | 15,8 | 19,1 | 40 / 40 |
| Spark 1M | 12,0 | 15,1 | 14,9 | 36,9 | 40,9 | 54 / 54 |

Leituras honestas:

- **Quente × frio.** O frio não melhorou (criar programa/textura/alvo custa o mesmo; 2 imagens ficou 91 × 68, dentro do
  ruído de um quadro só). Quem sente a retenção é o segundo quadro em diante.
- **CPU × drenado.** O drenado é 0,7–1,3 ms maior que a CPU; ele inclui a leitura e o IPC, não é um tempo de GPU, e o
  `legacy` mostra que o ganho de CPU também aparece no drenado (10,7 → 1,10 ms na imagem).
- **Cena `legacy` 8,25 × baseline 6,36** nesse lote: o `legacy` não é o baseline (ele roda o código novo com a retenção
  desligada). No lote final o mesmo par deu 5,50 × 5,74: é variação entre lotes, não efeito.
- **Chrome GPU (ANGLE/Metal)** repete o padrão (imagem 2,58 → 0,20; 4K 7,43 → 0,34; 2 imagens 8,1 → 0,35; cena 4,96 →
  0,22; texto 2,28 → 0,21). **SwiftShader** (GPU em software) mostra o efeito maior no drenado: imagem 42,7 → 2,35 ms, cena 102,8 → 4,9 ms
  (a CPU também cai: 1,8 → 0,23 ms na imagem), porque o `legacy` refaz o trabalho caro a cada quadro.
- **Conferência com o driver final.** O `driver.js` mudou depois do lote principal (só ganhou o *watchdog* de *rAF* e
  `raf_fallbacks`, que não mexem no que é medido). Um lote curto com o driver final, WebKit, 5 casos × 3 variantes ×
  3 passes (45 execuções), em [`final-driver-spotcheck`](viewport-performance-results/final-driver-spotcheck/),
  repete os números: imagem 0,45 / 8,18 / 7,68; 2 imagens 0,34 / 34,99 / 34,48; texto 0,35 / 4,08 / 4,26; cena 0,40 /
  5,50 / 5,74; vídeo quente 0,33 / 2,31 / 2,46 (atual / `legacy` / baseline, `render` em ms). A diferença submilimétrica
  entre lotes (imagem atual 0,32 × 0,61 × 0,45) é o passo de 1 ms do relógio e a carga (1 min de carga p50 6,4, p95 15,9,
  máx 22,7 durante os lotes), por isso só médias de N ≥ 300 aparecem como medida.
- **Carga da máquina** estava alta em todo o período (Mac15,6, M3 Pro, 11 núcleos; carga mediana 6,4). Números
  absolutos são desta máquina com esta carga; a razão `atual`/`legacy` é o que se compara.

## Janela real (WKWebView), build debug × release

Instâncias com `HOME` isolado, sem clique por coordenada, só `scripts/live-proof.py` e `scripts/seek-scrub-proof.py`
(relógio do **host**, `TESSEL_VIEWPORT_TRACE=1`). Fixture: o projeto da `live-proof` (cena, imagem, título e vídeo de
sequência de quadros, 320×180). `live-proof.py`: **56/56** em debug e em release, com a retenção ligada e desligada
(57/57 com `--shots`, que só acrescenta a checagem do PNG gravado; não foi repetido no build final: o PNG é coberto por
`frames.rs` e pelos testes de export).

Arrasto de 48 posições; "janela" = duração do arrasto; mediana (p95) em ms; release, sem leitura de pixels
(3 repetições por linha nas janelas rápidas):

| Arrasto | Retenção | Quadros mostrados | Intervalo entre quadros | `runtime_render` | `submit→done` |
|---|---|---|---|---|---|
| 2 s, 30 Hz | ligada | 48 | 36,2 (70) | 11 | 11,6 |
| 2 s, 30 Hz | desligada | 48 | 37,7 (68) | 16 | 16,2 |
| 0,5 s, 120 Hz | ligada | 39–45 | 10,2–11,9 (14–17) | 10 | 10,1–11,1 |
| 0,5 s, 120 Hz | desligada | 30–32 | 15,5–16,6 (17) | 13–14 | 15,1–16,5 |
| 0,25 s, 240 Hz | ligada | 19–22 | 10,3–11,8 (13–15) | 10–11 | 10,3–12,1 |
| 0,25 s, 240 Hz | desligada | 14–15 | 16,5–16,7 (17) | 13–14 | 16,2–16,6 |

- **O "~20 fps" era a prova, não o viewport.** O arrasto de 2 s só tem 48 posições distintas (24 por segundo): o
  intervalo de ≈ 36–42 ms entre quadros em 2 s é a demanda do gesto. Pressionado (0,25–0,5 s), o release mostra um quadro
  novo a cada ≈ 10–12 ms com a retenção e a cada ≈ 16 ms sem ela, com `coalesced` 40–47 (o agendador funde o que não dá
  tempo) e convergência na última posição em ≤ 15 ms depois de soltar. Isso é **taxa aceita e concluída no host**; o que
  o painel mostra vai no ritmo do *rAF* (≈ 60 Hz nesta tela) e não foi medido (G5).
- **Debug** (com leitura de pixel, `inspect` ligado, A/B pela variável): `runtime_render` 23–24 ms ligada × 26 ms
  desligada; intervalo entre quadros 33–34 ms nos dois; `transit` 5–20 ms (contra 0,3–3,4 ms em release: o GPUI em debug custa
  muito mais). A retenção não muda o ritmo em debug, o host domina.
- **`inspect` custa ≈ 13 ms** em release (`runtime_render` 24 ms): com ele ligado o ritmo cai para ≈ 25 ms e o arrasto
  curto fica em 9–19 quadros. É só para prova de pixel, nunca para medir latência.
- **Prova de pixel na janela** (`--pixel-proof`, passou em release e em debug; números do release): 50 imagens vistas
  durante o arrasto, 10 hashes distintos, e o hash do que estava na tela ao fim é igual ao do quadro pedido de forma explícita (`84d3a925378a4b25`).

## O que foi verificado (checks do Opus no build final)

| Check | Prova |
|---|---|
| Pixels em ordem embaralhada: imagem, texto, vídeo, cena, Spark | suíte `hash`: `image`, `image+text`, `text-changing`, `video-miss`, `scene`, `spark-100k-orbit` × `current`/`legacy`/`baseline` × 3 passes com ordens diferentes, nos 3 motores (WebKit, Chrome GPU, SwiftShader): **idênticos**, 0 falhas |
| Câmeras; várias cenas | `retention-browser.test.mjs`: câmeras diferentes sobre a mesma cena e **duas camadas de cena** (alvo por camada, `target_entries === 2`; o teste falha se o alvo voltar a ser único) |
| Mesmo ID com outra fonte | suíte `life` (WebKit, Chrome GPU, SwiftShader) e teste "the same id with another source never draws the old picture" |
| Perda de contexto, cena e Spark | `life` (cena e imagem, 3 motores, pixels exatos após restaurar), teste de contexto da retenção e `splat.test.mjs` (Spark em Chrome). Perda de contexto de **splat** no WKWebView nativo não está na `life` (G9) |
| Reload, release, troca de projeto | testes de release do asset, `viewport_host_tests` (reload, troca libera assets, limite de 16) e `live-proof` (reload) |
| Cancelar, timeout, resposta velha | `viewport_host_tests` (cancelamento, resposta descartada, boot/handshake/frame) e teste "asset replaced while a frame is being built cancels that frame" |
| Evicção quente e fria | teste "every cache stays inside its bound and a frame never loses what it draws" e `LruCache` (bytes, entradas, o que o último quadro usou) |
| PNG, hash, probes, export | `crates/render/tests/frames.rs` (7), `export.rs` (10), `inspect:false`/`true` no runtime; `live-proof` e prova de pixel acima |
| Telemetria sem custo quando desligada | `trace.rs` (4 testes), `hello_defaults_stay_byte_identical…` e sessão (24) |
| Ponte aditiva | campos opcionais; host antigo recebe a mesma mensagem de antes; `frame.presented` só vai a quem pediu |

Resultados da suíte: runtime **32/32**; `cargo test --workspace --locked` verde (core 133 unit + 8 `proof` + 2 headless +
7 `video_sound`; desktop 91, dos quais `viewport_host_tests` 32 e `viewport_tests` 7; export 10; media 14); `cargo clippy
--workspace --all-targets --locked -- -D warnings` limpo; `cargo fmt --all -- --check` limpo.

## Build e proveniência

| Artefato | Estado |
|---|---|
| `target/debug/tessel-editor` | construído e usado nas provas de janela (sha256 em `live-window-binaries.txt`) |
| `target/release/tessel-editor` | construído (30 s, 619 acertos de cache) e usado nas provas de janela |
| `runtime/dist/runtime.js` | bundle `c63a8092f0ef7a7f5739397cab9965864af0e8718f97c7c778f10beaca242558` |
| Baseline do harness | bundle `a8bb9ce2…` construído do código anterior (o repo não tem commits: o identificador é o sha256 dos arquivos) |

Hashes dos fontes do runtime, do harness, versões (Three.js 0.187.0, Spark 2.4.0), máquina, navegadores, renderizadores
e carga: [`PROVENANCE.json`](viewport-performance-results/PROVENANCE.json) (1095 execuções brutas). Disco: construir o
release custou ≈ 0,1 GiB com o cache; o espaço livre variou de 4,6 a 23 GiB durante o trabalho por causa do cache
compartilhado, não por artefatos desta entrega.

## Comandos

```sh
# Runtime
./scripts/runtime test                                   # 32 testes, Chrome
cd runtime && node scripts/perf/run.mjs --engine webkit --suite bench --passes 3 \
  --variants current,legacy,baseline --baseline-bundle=<bundle antigo> --drain read1x1 --out <dir>
node scripts/perf/run.mjs --engine <webkit|chrome-gpu|chrome-swiftshader> --suite <hash|life|bridge|bench> ...
node scripts/perf/report.mjs --in <dir>[,<dir>...] [--out <dir>]
node scripts/perf/provenance.mjs --out ../docs/viewport-performance-results --baseline-bundle <bundle> --raw <dirs>

# Rust (um Cargo por vez; GC do cache com teto)
MBX_GC_MAX_TOTAL_SIZE=64GiB ./scripts/cargo test --workspace --locked
MBX_GC_MAX_TOTAL_SIZE=64GiB ./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings
MBX_GC_MAX_TOTAL_SIZE=64GiB ./scripts/cargo build [--release] -p tessel-editor --locked

# Janela real, HOME isolado (ver ui-automation.md; nunca ./scripts/ui com outro HOME)
HOME=/tmp/x TESSEL_VIEWPORT_TRACE=1 [TESSEL_VIEWPORT_INSPECT=1] [TESSEL_VIEWPORT_RETAIN=0] ./target/release/tessel-editor
HOME=/tmp/x python3 scripts/live-proof.py /tmp/tessel-proof
HOME=/tmp/x python3 scripts/seek-scrub-proof.py --seconds 0.5 --hz 120 --span 120 [--pixel-proof]
```

## Arquivos

Runtime: `runtime/src/{retention,assets,compositor,index}.js`; testes `runtime/test/{retention,retention-browser,support}`
(o de duas cenas por camada foi acrescentado e verificado contra a mutação de RT único). Harness:
`runtime/scripts/perf/{run,report,provenance,assets,chrome,webkit,png}.mjs`, `wk-host.swift`, `page/{driver,preamble}.js`
(nesta retomada: *watchdog* de *rAF* com contagem `raf_fallbacks`, `--first-pass`, `--in` com várias pastas e
`provenance.mjs`). Rust: `crates/core/src/{trace,bridge,session}.rs` e `session/tests.rs`,
`crates/desktop/src/workstation/{viewport_host,viewport_host_tests}.rs` (formatação corrigida). Provas:
`scripts/seek-scrub-proof.py`. Docs: este relatório, `seek-performance-report.md`, `features/{viewport,seek,README}.md`,
`ui-automation.md`, `README.md` e `viewport-performance-results/` (≈ 660 KB).

## Riscos

- **Memória de GPU.** Os limites (256 MiB de texturas, 32 MiB de texto, 24 materiais, 4 rigs, 2 alvos / 192 MiB
  estimados) são estimativas; a contagem real vem do `cache` de `frame.done`. Um ambiente com GPU pequena pode precisar de
  limites menores (`debug.setLimits` existe para isso nos testes).
- **Internos do Three.js 0.187.** Descartar na perda de contexto antes de restaurar depende de como o Three recria
  `WebGLTextures`; uma atualização do Three exige rodar `life` e o teste de contexto de novo.
- **Spark.** Por segurança, cena com splat não usa rig retido; reter exigiria provar o *settle* do Spark, o que não foi
  feito e não é necessário para os casos plausíveis.
- **Relógios.** WebKit tem passo de 1 ms; qualquer p50 submilimétrico é artefato. `gl.finish()` não sincroniza no
  WebKit/Chromium, por isso foi trocado por `gl.flush()` no caminho sem leitura.
- **Retenção como escape.** `TESSEL_VIEWPORT_RETAIN=0` e `hello.retain:false` voltam ao caminho anterior (A/B e rollback).

## Lacunas e limites (nada aqui é "feito")

Próximos passos e status atualizado consolidados no [backlog de timing](viewport-performance-backlog.md).
Este relatório conserva as medições e lacunas da entrega original; correções posteriores
estão no relatório de bugs e no backlog, sem reescrever a evidência histórica.

- **G1 Primeiro quadro (frio):** sem ganho; só o quente melhora.
- **G2 Spark em movimento:** ≈ 20 %, longe de 1–4 ms; sem cache de alvo para Spark parado.
- **G3 Vídeo com *miss*:** sem ganho relevante; prefetch 1080p pode piorar e não foi tentado.
- **G4 WebM:** *(histórico; **resolvido** em [viewport-bugfix-report](viewport-bugfix-report.md))* `FORMATS` declarava `webm`.
  B1 (`<video>` preto), B2 (quadro anterior após seek), B3 (VP8/VP9 pretos no WebKit), B4 (parada de ≈ 1 s no H.264), B7
  (sequência só PNG) e B8 (`render_failed` em vez de `context_lost`) estão corrigidos ou tipados lá; B6 reproduzido e
  corrigido. B5 (RT único) era do protótipo e está coberto por teste.
- **G5 Foto na tela:** `seek→foto` não foi medido (sem captura de tela por quadro); `frame.done` e `session.presented`
  não provam o painel, e `frame.presented` só indica que a página pôde desenhar. A cadência real depende do *rAF* da tela
  (≈ 60 Hz aqui); nada foi fixado em 60 fps.
- **G6 Spark 3M em 4K no SwiftShader:** **não rodou** (custo alto). 3M/4K no GPU real também não.
- **G7 PNG 540p do core:** *(histórico)* medido depois em [viewport-bugfix-report#b7](viewport-bugfix-report.md#b7): PNG 540p
  p99 23,7 ms (WebKit, misto) contra 21 ms em JPEG; o core agora aceita PNG e JPEG. Nenhum proxy é gerado automaticamente.
- **G8 Seek de `<video>` com *timeout*:** o *timeout* de 10 s existe no código e não tem teste.
- **G9 Perda de contexto de splat no WKWebView:** só testada no Chrome (`splat.test.mjs`); a `life` nativa cobre imagem,
  texto e cena sem splat.
- **G10 Janela:** build release medido só em uma tela (≈ 60 Hz) e com um projeto; a janela não mede o ritmo na tela.
  A comparação ligada × desligada na janela vem de 3 repetições por linha; é consistente (10–12 ms × 16 ms), mas não
  é uma série longa.
- **G11 Ruído:** carga alta e passo de 1 ms; ver a nota do lote final.
- Incidente de processo, para transparência: rodar `./scripts/ui` com `HOME` trocado fez o wrapper reinstalar o toolchain
  num `HOME` temporário e reapontar o `target/` do repositório para o cache dele; o `target/` foi recriado (cache normal) e
  o `HOME` temporário, que era meu, foi removido. Nada do usuário foi apagado. A regra ficou em `ui-automation.md`.
