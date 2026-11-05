# Próximas otimizações do viewport: relatório (OPT01–OPT10)

Execução de [viewport-performance-next-plan.md](viewport-performance-next-plan.md) sobre o índice
[viewport-performance-backlog.md](viewport-performance-backlog.md). Nada foi commitado nem publicado (o repositório não tem
commit inicial; a identidade dos números é o hash dos arquivos em
[`viewport-performance-results/next/PROVENANCE.json`](viewport-performance-results/next/PROVENANCE.json) e `binaries.txt`).

**Leitura em uma frase.** O proxy automático de scrub (OPT01) é o ganho real e mensurável na janela: um arrasto de 2 s a
60 Hz sobre um clipe H.264 1080p passa de ~67 quadros mostrados (intervalo mediano 29 ms, imagem 3 quadros atrás do
playhead) para 120 (intervalo 16,7 ms, 0 de atraso), a imagem é refinada para o original ≈ 0,33 s depois de soltar, e o
export/`render.frame` continuam usando o original (export idêntico, frame a frame, ao da política `original`).
O cache de resultado Spark (OPT02) vale **só para cena parada**. Nada aqui é "60 fps universal", nem tempo de GPU, nem foto na
tela: CPU < não é GPU pronta; o `complete` do harness é a GPU drenada por uma leitura de 1×1; `rAF`/`ack` não são pixel físico.

## Estado por item

Revisão após a entrega: o pai reexecutou `TESSEL_REQUIRE_RUNTIME=1 ./scripts/runtime test`
na árvore final: **53/53**, sem falhas nem skips. Os fingerprints dos arquivos do runtime
conferem com `next/PROVENANCE.json`. Isso não repete as medições de janela nem supre os
gaps abaixo: além dos itens físicos, faltam regressões de perda de contexto/ocultação com
proxy, determinismo 3M, o caso UHD/SwiftShader e a investigação da falha do Chrome na bancada.
LOD e otimização de sort em movimento também não foram implementados. O plano permanece
parcial nesses pontos; experimentos sem ganho são decisões documentadas, não features novas.

| ID | Estado | Evidência (o que prova e o que não prova) |
|---|---|---|
| OPT01 proxy automático | **Entregue e provado** (macOS/WebKit, release) | Seções [OPT01](#opt01-proxy-automático-de-previewscrub) e [provas na janela](#provas-na-janela-real-release) |
| OPT02 cache do resultado Spark parado | **Entregue e provado** (cena parada) | [OPT02](#opt02-resultado-de-cena-spark-parada); em movimento nada mudou |
| OPT03 backpressure por rAF | **Medido, sem ganho, removido** | [OPT03](#opt03-backpressure-pela-apresentação-medido-e-removido) |
| OPT04 notify seletivo | **Entregue** (contadores; sem alegação de FPS) | [OPT04](#opt04-repaint-seletivo) |
| OPT05 primeiro frame / recursos frios | **Medido; sem experimento que se pague, nada mantido** | [OPT05](#opt05-primeiro-frame-e-recursos-frios) |
| OPT06 decode / cache miss de vídeo | **Resolvido pelo proxy no scrub; prefetch medido e removido** | [OPT06](#opt06-decode-e-cache-miss) |
| OPT07 Spark em movimento (MSAA/LOD) | **Gancho opt-in medido; padrão inalterado** | [OPT07](#opt07-spark-em-movimento) |
| OPT08 seek-to-photon | **Pendente (hardware)**; método publicado | [OPT08](#opt08-seek-to-photon-pendente) |
| OPT09 3M/4K, longa duração | **Amostra com orçamento declarada**; SwiftShader/UHD export não rodou | [OPT09](#opt09-3m4k-e-longa-duração) |
| OPT10 outros displays/cargas | **Pendente** (uma tela só); método publicado | [OPT10](#opt10-outros-displays-e-cargas-pendente) |

## Baseline preservado

Antes: 39 testes JS do runtime + 333 de Rust (2 ignorados), builds debug/release, B1/B2/B3/B4/B7/B8, import com FFmpeg,
áudio/jobs/persistência/interchange/export. Depois, na árvore final (`TESSEL_REQUIRE_RUNTIME=1`):

| Verificação | Resultado |
|---|---|
| `./scripts/cargo fmt --check`, `check --workspace --all-targets --locked`, `clippy … -D warnings` | limpos |
| `./scripts/cargo test --workspace --locked` | **360 passaram, 0 falharam, 2 ignorados** (333 → 360) |
| `pnpm --dir runtime test` / `build` | **53/53** (39 → 53); bundle 4060 KiB |
| `./scripts/cargo build --locked` e `--release --locked` | ok (hashes em `binaries.txt`) |
| `scripts/live-flow-proof.py` (janela real, debug, HOME isolado: import, edição, undo, save, package, restore, interchange, export 1080p/vertical/square com áudio medido, cancel, SIGKILL + recover) | **115 PASS, `failed: []`** |
| `scripts/video-live-proof.py` (release: frame mostrado == frame pedido em H.264/PNG/JPEG, WebM com erro tipado, seek p99 < 500 ms) | todos PASS |
| `scripts/proxy-live-proof.py` (release) | 32/32 checagens |

Um teste do runtime (`splat.test.mjs`, passo 6: "soltar um asset durante o load cancela") falhou **uma vez** em uma rodada
com a máquina carregada (`'ready' !== 'failed'`: o load terminou antes do release) e passou nas demais (53/53 em três
execuções seguidas). É uma corrida do próprio teste, anterior a esta entrega; fica registrada, não corrigida.

## Dois bugs achados pelas provas na janela (corrigidos, com regressão)

Ambos só apareceram no build real, não na bancada do runtime:

1. **Um quadro original nunca carregado falhava (`cancelled`, "released while loading") enquanto um proxy estava anexado.**
   `frameTexture` comparava o proxy anexado com "sem proxy" também no caminho do original. Quebrava **todo export** de um
   vídeo que o editor já tinha com proxy (quadro 1 do export) e qualquer `render.frame` num quadro novo. Correção em
   `runtime/src/assets.js` (só a variante proxy é amarrada ao proxy anexado); regressão em
   `runtime/test/proxy-cache.test.mjs` ("an original frame that was never loaded still draws while a proxy is attached").
2. **Um vídeo desenhado antes do decode terminar nunca passava a usar os frames decodificados** (o runtime seguia com o
   container nativo e respondia `unsupported_format` ao `asset.proxy`; `session.proxy_error` agora guarda o motivo).
   `Session::loaded_from` recarrega o asset quando a origem (frames decodificados) muda; regressão em
   `a_video_drawn_before_it_was_decoded_is_reloaded_from_its_frames_and_takes_the_proxy`.

## OPT01: proxy automático de preview/scrub

Contrato completo em [viewport.md](features/viewport.md#proxy-automático-de-preview-opt01) e
[media-io.md](features/media-io.md). Resumo: política `auto`/`original` (+ altura 540 e formato JPEG/PNG) inspecionável por
`op preview.policy` e pelo nó `monitor.preview.quality`; só o quadro de scrub do monitor sem PNG/probes é `Preview`;
`render.frame`, export, PNG, probes e o quadro de refino são `Original`; o refino troca o proxy pelo original
200 ms (+ timer de 220 ms no desktop) depois da última intenção; o cabeçalho diz " · proxy"; o runtime cai para o original
por camada e relata `source.proxy_error`. O job "Build preview proxy" gera a sequência derivada depois do decode, sem
originais apagados e sem mudar codec.

**Bancada (WebKit, 1080p, por quadro, GPU drenada = `complete`)** — `viewport-performance-results/next/tables.md`:

| Caso | render (média) | complete p50 / p95 / p99 |
|---|---|---|
| frames originais PNG 1080p, sequencial | 35,8 ms | 39 / 40 / 41 |
| frames originais PNG 1080p, aleatório | 36,0 ms | 39 / 41 / 42 |
| proxy JPEG 540p, sequencial | 7,9 ms | 9 / 11 / 11 |
| proxy JPEG 540p, aleatório | 7,9 ms | 9 / 11 / 12 |
| proxy PNG 540p, sequencial / aleatório | 10,6 / 10,7 ms | 12 / 13 / 13 |

**Na janela real** (release, HOME isolado, só pedidos semânticos; clipe H.264 1080p, 240 quadros; arrasto de 2 s a 60 Hz
por `seek-scrub-proof.py`, relógio do próprio editor via `TESSEL_VIEWPORT_TRACE=1`; 5 arrastos por variante;
`live/proxy-live-proof.json`):

| Variante | quadros mostrados em 2 s | intervalo entre quadros (mediana / p95) | atraso da imagem vs playhead (mediana) |
|---|---|---|---|
| política `original` | 64 / 67 / 68 / 67 / 68 | 29,1 / 33,3 ms | 3 quadros |
| política `auto` (proxy) | 120 em todos | 16,7 / 17,1 ms | 0 quadros |

Cuidado com a leitura: 120 é o número de intenções enviadas (60 Hz × 2 s): com proxy **nenhuma foi fundida**
(`coalesced` ≈ 0), sem proxy cerca de 60 % foram fundidas. O intervalo de 16,7 ms é o do próprio arrasto, não um teto nem
um "60 fps" do produto; sem proxy o editor não acompanha o arrasto.

**Refino** (5 arrastos, sondagem de ~10 ms ⇒ limite superior): imagem de proxy 2 ms após soltar; imagem original
327–345 ms após soltar (200 ms de repouso + timer do desktop + render original).

**Original/final sem imagem velha e export com original:**

- `explicit render.frame` decodifica o código de barras do quadro pedido e reporta `quality: original`, `proxy_layers: 0`
  (também com proxy anexado).
- Com `TESSEL_VIEWPORT_INSPECT=1`, a imagem na tela após o arrasto tem **o mesmo hash** que o `render.frame` explícito do
  último quadro (2 execuções).
- Export de 24 quadros 1080p do editor com proxy vs do editor em política `original` (que **não tem proxy algum**):
  `framemd5` dos 24 quadros **idêntico**; 1920×1080, 24 fps. O PSNR contra a fonte (25,09 dB nos dois) é só informativo: o
  clipe é ruído sintético e o PSNR não distingue uma cópia 540p de um re-encode; a prova é a identidade dos quadros.
- Testes: política/regras em `crates/core/src/preview.rs`, sessão (11+1 testes de proxy/refino/recarga), media
  (`crates/media/tests/preview_proxy.rs`, 5), host, runtime (cache/pixels, proxy falho, proxy de outra contagem, id
  recarregado, limites).

Casos negativos: mesmo ID com geração de asset nova (proxy descartado e recarga, teste de runtime e de sessão), proxy de
outros frames recusado (`set_preview_proxy`, `proxy_stale`), arquivo de proxy ausente (sem anexar), proxy recusado pelo
runtime (contado, não reenviado), troca de política no meio de um quadro (a fusão exige a mesma qualidade), `reset_project`/
reload reanexa, runtime sem a feature `asset.proxy` (nada é enviado), quadro explícito/edição descartam o refino.
**Não cobertos como teste automatizado:** perda de contexto da GPU *com proxy* e ocultar/reabrir a janela com proxy
(a perda de contexto sem proxy e o reload têm testes; o recarregamento por perda reutiliza o mesmo caminho do reload).
*Atualização:* os dois casos agora têm teste e prova na janela real, ver o [relatório de ciclo de vida](viewport-lifecycle-report.md).

## OPT02: resultado de cena Spark parada

WebKit, 100k/1M splats parados (`complete` = GPU drenada): 100k 18 → 3 ms (p50), 1M 35 → 2 ms; CPU do runtime 1M 12,5 → 1,0 ms.
3M parado (OPT09): 68 → 3 ms. Pixels iguais ao caminho sem cache (hash), inclusive duas cenas, mudança de câmera/tamanho/
asset com o mesmo ID, tempo de cena animada, perda de contexto e quadro cancelado antes do desenho (`runtime/test/proxy-cache.test.mjs`).
**Em movimento (órbita) nada muda**; não há promessa de 1–4 ms.

## OPT03: backpressure pela apresentação (medido e removido)

Experimento: segurar o próximo quadro do scrub até o `frame.presented` da página (rAF), com timeout de 100 ms, atrás de
`TESSEL_VIEWPORT_PRESENT_GATE=1`. Medido na janela real, release, mesmos arrastos
(`live/present-gate-experiment-removed.json`, 3 arrastos por variante):

| Variante | quadros em 2 s | intervalo mediano / p95 | atraso (mediana) |
|---|---|---|---|
| `original` sem gate | 63 / 67 / 67 | 29,3 / 34,5 ms | 3 |
| `original` com gate | 68 / 70 / 70 | 28,0 / 32,0 ms | 3 |
| `auto` sem gate | 120 | 16,6 / 24,8 ms | 0 |
| `auto` com gate | 120 | 16,6 / 20,7 ms | 0 |

O gate não mudou o atraso nem o intervalo mediano e a diferença nos demais números está no ruído de 3 amostras; não paga a
complexidade (timeout, janela oculta, estado extra). **Removido** (código, env e testes). `frame.presented` segue como
telemetria do trace; `rAF`/ack não é pixel físico.

## OPT04: repaint seletivo

`viewport_ipc` só repinta o GPUI quando muda o que o monitor mostra (`ViewportHost::ui_signature`: cabeçalho + estado +
erro). Mensagens como `frame.presented`, `asset.proxy`/`asset.state` sem efeito visível e quadros com o mesmo texto
(`… 12 ms`) não repintam. Contadores em `render.status.ui`: no `proxy-live-proof` (com trace, que gera um
`frame.presented` por quadro) a variante `auto` fez 670 repaints e pulou 671 (≈ 50 % das mensagens); a `original`, 24 e 694;
no soak de 480 s, 13 043 e 1 897. **Não** é um ganho de FPS medido: a imagem é o canvas da WebView; o ganho possível é de
CPU/UI-thread e o contador só mostra repaints evitados. Teste: `a_presentation_report_changes_nothing_the_window_shows_but_a_new_picture_does`.

## OPT05: primeiro frame e recursos frios

O trace da janela separa os spans (`prepare`, `serialize`, `dispatch`, `transit`, `runtime_render`). Um pico único de ≈ 400 ms
aparece no **primeiro `render.frame` explícito em 1080p** (decode do PNG original + recriação do alvo ao tamanho do
projeto); `gl_programs: 1` e `material_created: 0` nesse quadro mostram que o programa já estava compilado pelos quadros
do monitor, então uma pré-compilação não teria o que ganhar ali. O que sobra é decode/upload do original, que depende do
asset e só se evita sabendo qual quadro vem (prefetch às cegas: ver OPT06). **Nenhum experimento foi mantido**; o span
já existe para o diagnóstico. Não medido: o primeiro quadro de uma cena Spark em GPU real (só WebKit/SwiftShader).

## OPT06: decode e cache miss

No scrub o miss caro (36 ms por quadro 1080p) deixa de estar no caminho porque o proxy tem 8–11 ms. Prefetch (quadro seguinte,
em ordem) foi implementado como experimento e **removido**: sequencial JPEG 8,1 → 5,3 ms, mas em aleatório e em PNG original
não houve ganho (`raw/webkit-prefetch-removed-experiment.jsonl`), e competir com o quadro corrente é o risco já apontado no
backlog. Sem prefetch cego no produto.

## OPT07: Spark em movimento

Gancho opt-in (`hello.scene_samples`, `TESSEL_VIEWPORT_SCENE_SAMPLES=0|2|4`, padrão 4; frame final/export mantêm o
configurado). Órbita, `complete` p50 (WebKit, drenado): 1M 35 → 27 (2×) → 21 ms (0×); 100k 16 → 13 → 12 ms; 3M 68 → 44 ms
(0×). O custo é qualidade de borda visível, **por isso não é padrão**. LOD e a ordenação (sort) em movimento não foram
implementados: sem hipótese medida que os justificasse nesta janela de execução; permanecem como pendência do backlog.

## OPT08: seek-to-photon (pendente)

Sem câmera de alta velocidade nem pessoa para o procedimento. **Não medido.** Método para quem tiver o hardware: cena
sintética com marcador por quadro (código de barras do `video-live-proof.py`), `ui op seek` por um script semântico, câmera
≥ 240 fps apontada para a tela e para um LED/relógio sincronizados ao envio; publicar resolução temporal (1/fps),
amostras e a taxa de atualização da tela; `ack`, `rAF` e captura de tela não contam como pixel físico.

## OPT09: 3M/4K e longa duração

Amostra **com orçamento declarado** (WebKit nativo do macOS, GPU Apple M3 Pro, 1 passada de 60 quadros, `--drain read1x1`,
asset 3M de 96 MB em `/tmp`, disco guardado a 6 GiB; `raw/webkit-heavy-*.jsonl`):

| Caso (complete p50 / p95 / p99, ms) | atual | sem cache | MSAA 0 |
|---|---|---|---|
| imagem 4K | 4 / 5 / 5 | 4 / 5 / 5 | 4 / 4 / 5 |
| Spark 3M parado | 3 / 4 / 4 | 68 / 75 / 109 | 3 / 4 / 5 |
| Spark 3M órbita | 68 / 75 / 114 | 69 / 83 / 112 | 44 / 46 / 66 |

3M em órbita fica em ≈ 15 quadros/s **drenados** (68 ms): não há promessa de 60. **Não rodaram:** SwiftShader/UHD 4K
(export 4K por `--uhd` do `live-flow-proof.py`), 3M em Chrome GPU, e determinismo de pixel em 3M (o hash 3M não foi comparado;
a igualdade de pixels do cache foi provada em 100k/1M e no runtime).

Longa duração (`scripts/proxy-soak.py`, release, 1080p, **480 s**, 131 arrastos de 2 s com a política alternando para `original`
a cada 5º arrasto): 14 879 quadros aceitos, 0 cancelados, 0 descartados, `proxy_failed 0`, 237 refinos, **131/131 arrastos
convergiram**, memória residente do editor 174,7 → 175,3 MB (pico 175,3), fase `ready` no fim. O RSS dos processos auxiliares
do WebKit não foi capturado (não são filhos do editor: 0,0 é "não medido", não "zero"). Isto é uma amostra de 8 minutos, não de horas.

## OPT10: outros displays e cargas (pendente)

Só uma tela (a do Mac usado), sem carga sintética paralela documentada, sem 120/240 Hz externo. **Não medido.** Método:
repetir `scripts/proxy-live-proof.py --runs 5` e `proxy-soak.py` em cada display/taxa, anotar `load average` (o harness já
grava) e comparar p50/p95/p99 do intervalo e a convergência; a sondagem do observador nunca é FPS.

## Arquivos e comandos novos

`crates/core/src/preview.rs`; `scripts/proxy-live-proof.py`, `scripts/proxy-soak.py`, `scripts/disk-guard.sh`;
`seek-scrub-proof.py` ganhou `release_to_picture_ms`, `release_to_original_polled_ms` e a convergência pós-refino (um arrasto
que termina num quadro já mostrado em original não desenha nada: não use esses dois campos sozinhos; a latência de refino
medida de forma robusta está em `proxy-live-proof.py`). Ops: `preview.policy`, `media.preview_proxy`; nó `monitor.preview.quality`;
bridge: `asset.proxy`, `frame.render.quality`, `frame.done.source`, `hello.result_cache`, `hello.scene_samples`; env:
`TESSEL_VIEWPORT_RESULT_CACHE=0`, `TESSEL_VIEWPORT_SCENE_SAMPLES`. Dados compactos e proveniência (hashes do código, do binário,
da máquina, dos bundles, baseline `03b1d6cd…`: o bundle do runtime no começo desta execução, guardado compactado em `baseline-runtime.js.gz`) em `viewport-performance-results/next/`.

## Limites que continuam valendo

- CPU baixa **não** é GPU pronta; `complete` é um limite superior com IPC; nada daqui é o instante do fóton.
- Só macOS/WebKit/uma tela. Chrome GPU foi usado antes (relatórios anteriores); o run do Chrome nesta janela quebrou na
  bancada e a causa não foi investigada, então nenhum número novo é de Chrome.
- Proxy só para vídeo container decodificado; sequências PNG/JPEG e imagens não ganham proxy.
- O proxy muda a imagem do scrub (menos pixels): a qualidade é visível (" · proxy") e a política é `original` por um clique.
