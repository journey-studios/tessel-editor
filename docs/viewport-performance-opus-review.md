# Parecer técnico: preview do viewport de ~9 ms para 1–4 ms com Three.js + Spark

Parecer independente, 2026-11-04. Nenhum código de produção foi alterado e nada foi commitado.
Revisor: Opus 5.5. Código lido às 19:5x (`compositor.js` 18:25, `session.rs` 18:34, `viewport_host.rs` 18:35).

## Veredito

1. **Os 9 ms vêm de dois desperdícios, não de objetos JS.** Em todo quadro o runtime reenvia a textura e
   destrói e recompila o programa de shader. Só com esses dois retidos, a imagem cai de 7,7 ms para 0,8 ms
   (média CPU, WKWebView real) com **pixels idênticos**.
2. **Parte da proposta inicial não se sustenta.** Reusar camera/`Scene`/geometria e guardar `setSize` não deu
   ganho mensurável isoladamente. Sync assíncrono não tem o que economizar no 2D. Não é preciso grafo de cena
   incremental nem trocar de motor.
3. **1–4 ms é plausível para imagem, texto, vídeo com cache e cena procedural (quente).** Não é plausível para
   vídeo com cache miss (~21 ms) nem para Spark com câmera em movimento (7 / 36 / 68 ms em 100k / 1M / 3M).
4. **O "~20 quadros/s, ~50 ms entre quadros" é em boa parte artefato do medidor.** O `present_gap_ms` mediano
   mede o período de amostragem da prova, não o intervalo entre quadros. Com `--poll-ms 250`, ele vira 256 ms
   enquanto a contagem de quadros quase não muda.
5. **Prioridade:** corrigir a medição → reter textura e programa → instrumentar host/GPUI em release →
   vídeo → Spark.

## Como medi e o que vale cada número

Harness isolado em `/tmp/tvb` (fora do repositório; `/tmp` pode sumir, portar para `runtime/scripts/` na fase 0).
A variante é uma **cópia** de `runtime/src` com flags. Os assets são sintéticos ou demo.

- **Máquina:** Apple M3 Pro. WKWebView real com esquema `tessel://` em janela flutuante de 320×180 sem ativação.
  Chrome com ANGLE/Metal. Buffer de 960×540.
- **Protocolo:** página nova por execução, 10 quadros de aquecimento, 300 quadros (splats: 120/90/40),
  um quadro por rAF.
- **Carga:** load average 3–9 durante as medições. Uma primeira rodada de splats pegou carga de até 43, foi
  **descartada por completo** e reexecutada. Está guardada em `w-splat-loaded.jsonl`.
- **Janela real:** `target/debug/tessel-editor` em instância própria com `HOME` isolado. A janela do usuário não
  foi tocada.

Limites:
- Build **debug** na janela real; release não foi medido.
- Uma máquina só. O relógio do WebKit tem quantum de 1 ms.
- Na ablação do Chrome a variação entre corridas chega a ±1,5 ms.
- O cenário `videomiss` usa handler Swift, não o `serve()` em Rust.
- Splats sintéticos não representam o overdraw de captura real.
- Arraste com ponteiro real e seek-to-photon **não foram medidos**.

---

## 1. Mapa dos custos reais e armadilhas de medição

```text
gesto → Command → EditorState → ViewportHost::sync: plan + to_json + key + can_coalesce (replan)   [UI]
      → evaluate_script(frame.render)                                               [UI → WebContent]
      → JSON.parse → renderQueue → Compositor.draw                                         [WebContent]
          prepareLayer: textura / canvas de texto / SceneRig + RT MSAA / settle do Spark
          render: cria e linka programa (wait síncrono), texImage (IPC), draw
      → gl.finish (não espera) → frame.done
      → ipc.postMessage → async_channel → on_ipc → dispatch_pending → cx.notify             [UI, GPUI]
      → GPU process (trabalho real, invisível para render_ms) → próximo rendering update → compositor → tela
```

### Custo medido por quadro (WKWebView, 60 quadros, contadores em cada chamada GL)

| Cenário | render médio | Chamadas por quadro | Onde vai o tempo |
|---|---|---|---|
| Imagem 1672×941, atual | 7,6 ms | 1 `createProgram`, 2 `compileShader`, 1 `linkProgram`, 1 `deleteProgram`, 1 `createTexture`, 1 `texStorage2D`, 1 `texSubImage2D`, 3 `bufferData` | `texSubImage2D` 5,4 ms; `getProgramInfoLog` 1,2 ms |
| Imagem, tudo retido | 0,32 ms | 1 `drawElements`, 1 `finish` | — |
| Texto estático, atual | 5,0 ms | idem | `getProgramInfoLog` 1,8; `texSubImage2D` 1,5 |
| Vídeo com cache, atual | 2,2 ms | programa recriado, sem upload | `getProgramInfoLog` 1,3 |
| Cena caixa+esfera, atual | 5,9 ms | 2 programas, 4 compile, 2 RBO MSAA, 2 FBO, 11 `bufferData` | `getProgramInfoLog` 3,7 |

Trechos responsáveis:
- Upload: `runtime/src/compositor.js:230` (`new THREE.Texture(entry.bitmap)` com `needsUpdate`).
- Destruição do programa: `compositor.js:287` e `:206`, somados a `three/src/renderers/webgl/WebGLPrograms.js:663-676`.
  Ao descartar o último material, `usedTimes` chega a 0 e o programa é destruído.

### Armadilhas

| Armadilha | Evidência | Regra |
|---|---|---|
| **`present_gap_ms` mede o amostrador** | Sempre ~50 ms com poll de 10 ms e ~256 ms com poll de 250 ms, em runtime atual e retido, a 30 e 120 Hz. A contagem no mesmo experimento dá 26–30 quadros/s | Não usar. Timestamp por quadro, lido uma vez depois do gesto (`scripts/seek-scrub-proof.py:17`, `:223`) |
| A prova limita o resultado | Entrada de 30 Hz dá teto de 30 quadros/s; cada `get`/`set` passa pela thread de UI | Separar carga de observação; variar a entrada |
| `render_ms` é tempo de CPU e mistura quadros | 1M splats: `render` 36 ms sem sync; com `readPixels` 1×1 após cada quadro, `render` 12 + `gpu` 24. A soma é igual: o `render` do quadro N paga a GPU do N−1 | Nunca ler como tempo de GPU |
| `render_ms` esconde GPU no 2D | Imagem atual: no modo com `readPixels` 1×1 por quadro, `render` médio 11,9 ms mais 3,9 ms de leitura | Medir CPU e "GPU drenada" em execuções separadas |
| Relógio de 1 ms no WebKit | `clock_quantum_ms` = 1,0; `crossOriginIsolated` falso mesmo com COOP/COEP no esquema customizado | p50 abaixo de 1 ms não significa nada; usar média de N ≥ 300 e relógio nativo em volta |
| `frame.done` e `session.presented` não significam "na tela" | Saem antes do rendering update; `presented` só registra aceitação | Renomear conceitualmente ou adicionar sinal de rAF |
| `settle_ms ≈ 0` parece GPU pronta | Ver §5 | Remover o campo ou documentar que não prova nada |
| Média sem frios | Primeiro quadro: 18 ms (1,6 MP), 50 ms (4K), 59 ms (1M) | Reportar frio e quente separados |
| Tamanho do asset define o número | 4K atual 33,9 ms; 2 camadas 36 ms. Os "9 ms" valem para ~1,6 MP | Matriz fixa de assets |
| A contagem de chamadas perturba o tempo | Wrappers em cada chamada GL | Contar e cronometrar em execuções distintas (feito aqui) |

---

## 2. Hipóteses ranqueadas

Valores de WKWebView com média CPU, duas passadas intercaladas. "Provável" é **estimativa**; "medido" é deste
experimento, em protótipo.

| # | Hipótese | Onde | Ganho | Risco | Status / experimento |
|---|---|---|---|---|---|
| 1 | Reenvio da textura a cada quadro | `compositor.js:228-238` | **Medido:** 7,7 → 3,15 ms (1,6 MP); com tudo retido, 33,9 → 0,23 ms (4K) | Memória de GPU; ciclo de vida | **Confirmada.** Refutaria se o tempo não caísse com a textura retida |
| 2 | Programa destruído e recriado | `compositor.js:206,287`; `scene.js:60,124` | **Medido:** 7,7 → 5,5–6,2 ms sozinho; 1+2 → 0,74–0,86 ms. Chrome: 2,4 → 0,19 ms | Baixo; pool invalidado na perda de contexto | **Confirmada** |
| 3 | Rig e RT MSAA recriados (cena) | `compositor.js:259-262` | **Medido:** caixa 5,8 → 0,35 ms; só RT+material: 5,09 ms (o programa Lambert domina) | Rig preso a asset trocado | **Confirmada** |
| 4 | Cache de texto | `compositor.js:248-252` | **Medido:** 4,4 → 0,37 ms estático; mudando a cada quadro, 4,7 → 2,6 ms (p99 18) | Cache sem limite (LRU) | **Confirmada** |
| 5 | Vídeo com cache miss | `assets.js:243-278`; limite de 16 quadros (`:11`) | Retenção só dá 22,2 → 20,8 ms. **Provável:** a maior parte volta com prefetch ou proxy em scrub local (especulação até medir) | Memória; trabalho desperdiçado | Dominado por fetch + decode + upload. Experimento: taxa de acerto e tempo por mídia real, com prefetch ±N direcional e `media.proxy` |
| 6 | Host/GPUI no caminho do scrub | `viewport_host.rs:287-320`; `viewport.rs:95-106` (`cx.notify` a cada mensagem); `session.rs:263-283` | **Não atribuído.** Piso WebKit sem GPUI: 10,4 ms de ida e volta (runtime atual) e 0,12 ms (retido). Janela real debug com ≈90 Hz de entrada: ~25 ms por quadro mesmo retido | Médio | **Sobra 25 ms sem dono.** Fazer spans e medir release; testar `notify` só quando o cabeçalho mudar |
| 7 | Arquivo lido por inteiro na thread de UI | `host.rs:262-265` (`read_to_end`) via `ProtocolHandle` com `Rc<RefCell>` (`viewport_host.rs:655-663`) | Não medido. **Provável:** picos de UI em splats grandes e em cada cache miss de vídeo | Médio, handler assíncrono | A leitura do código indica thread de UI (`Rc` não é `Send`); confirmar com span e tamanho do arquivo |
| 8 | Spark ressorteia com câmera parada | `splat-stage.js:54-67`; `spark.module.js:12512-12571` (`autoUpdate:false` força `generate()` e sort) | **Medido:** 1M parado 39,6 ms ≈ orbitando 40,6 ms. Cache do RT com chave correta: 0,2–0,4 ms | **Alto** se a chave errar | **Confirmada** só para cena sem nada animado |
| 9 | MSAA 4× no RT de splat | `compositor.js:261` | **Medido:** 1M 35,6 → 21,4 ms; 3M 68,1 → 45,0 ms; 100k sem ganho | Muda pixels; só durante o gesto, com quadro final completo | Sem decisão até comparar visualmente. Fere "sem queda de qualidade" se adotado sem aprovação |
| 10 | Timer de 4 ms na leitura assíncrona | `three/src/utils.js:386-400`, chamado pelo sort do Spark | Não medido. **Especulação:** parte do piso de ~7 ms em 100k | Alto (interno de dependência) | Medir fases do sort em uma cópia |

### Refutadas ou sem ganho medido

| Proposta | Resultado | Recomendação |
|---|---|---|
| Reusar camera e `Scene` | 7,36–7,59 ms vs base 7,67–7,80 ms | Não vale como otimização isolada |
| Reusar geometria | 7,41–7,52 ms vs base | Adiar. O conjunto (com textura, material, cena, geometria e guarda) fica 0,4 ms abaixo de 1+2 no WebKit; no Chrome não se distingue do ruído |
| Guarda em `setSize` | WebKit e Chrome: ~0 µs por atribuição (300 repetições, abaixo da resolução) e o pixel **não** é limpo | Sem ganho medido. Só por clareza |
| Grafo de cena incremental | O custo não está em objetos JS | **Não fazer** |
| `preserveDrawingBuffer:false` | Sem diferença de CPU (0,21–0,74 ms vs 0,37–0,43 ms). Custo de composição não medido | Não mexer agora; envolve `toDataURL` e o front buffer |
| Remover `gl.finish` por desempenho | 0,29–0,50 ms vs 0,37–0,43 ms | Remover pela **verdade do código**, não pelo tempo (§5) |
| Sync assíncrono no 2D | A espera síncrona vinha do link do programa, que some ao retê-lo | Sem ganho. Serve para backpressure, não para reduzir `render_ms` |
| Trocar para threers | 2D abaixo de 1 ms em Three; Spark domina no resto | **Sem evidência a favor.** Reavaliar só se Spark for trocado por outro motivo |

---

## 3. 1–4 ms é plausível?

Mesma resolução (960×540) e GPU, WKWebView, quente. "Atual" e "retido" são médias de CPU com relógio de 1 ms;
"GPU drenada" é o `readPixels` 1×1 medido depois do quadro, um limite superior que inclui IPC.

| Conteúdo | Atual | Retido | GPU drenada (retido) | Frio | 1–4 ms? |
|---|---|---|---|---|---|
| Imagem 1,6 MP | 7,7 | 0,4 | 0,85 | 15–19 | **Sim** |
| Imagem 4K | 33,9 | 0,23 | 1,07 | ~50 | **Sim** quente; 1º quadro não |
| 2 camadas (1,6 MP + 4K) | 36,0 | 0,52 | não medido | ~55 | **Sim** quente |
| Texto estático | 4,4 | 0,37 | não medido | 10–13 | **Sim** |
| Texto mudando por quadro | 4,7 | 2,6 (p99 18) | não medido | 12–13 | **Provavelmente na média**; p99 não |
| Vídeo com cache | 3,3 | 0,5 | não medido | 13 | **Sim** |
| Vídeo miss (720p PNG) | 22,2 | 20,8 | não medido | 13–24 | **Não** sem prefetch ou proxy |
| Cena procedural | 5,8 | 0,35 | não medido | 18 | **Sim em CPU**; GPU do MSAA não medida |
| Spark 100k, câmera parada, sem animação | 14,8 | 0,19 (cache) | não medido | 42 | **Só com cache de RT** |
| Spark 100k, câmera em movimento | 15,5 | 7,3 | 8,0 (`render` 7,0) | — | **Não**; ~15 ms até drenar |
| Spark 1M, em movimento | 40,6 | 35,6 (21,4 sem MSAA) | 23,6 (`render` 12,3) | 59 | **Não** |
| Spark 3M, em movimento | 70,9 | 68,1 (45,0 sem MSAA) | 47,9 (`render` 20,9) | 92 | **Não** |

- **Chrome:** imagem atual 2,4 ms → 0,19–0,28 ms; texto 2,8 → 0,25 ms; cena 5,0 → 0,47 ms; Spark 1M 32,9 → 29,3 ms.
  A diferença entre os motores está principalmente no upload: `texSubImage2D` 5,4 ms (WebKit) contra 0,6 ms (Chrome).
- **Spark grande:** a meta útil é não travar a UI nem atrasar o gesto, não 1–4 ms. Com a regra atual "nunca desenhar
  sort de outro ponto de vista", o quadro novo fica longe disso. Relaxar essa regra só durante o gesto é decisão de produto.
- **Especulação:** LOD do Spark ou resolução adaptativa no gesto podem baixar bastante, mas não medi.

---

## 4. Plano incremental e benchmarks

### Plano

| Fase | Entrega | Critério de saída |
|---|---|---|
| **0. Medição** (antes de otimizar) | Portar o harness para `runtime/scripts/`; trocar o gap amostrado por anel de timestamps por quadro (`sent`, `done`, `accepted`), lido só após o gesto; `frame.presented` aditivo via rAF; corrigir a leitura de ~20 fps em `seek-performance-report.md` | Baseline reproduzido em A/B/A |
| **1. Retenção mínima** | Textura de imagem no `AssetStore`; material por (blend, slot); texto com LRU; rig e RT por cena | Metas abaixo; hashes idênticos; testes de perda de contexto e troca de asset |
| **2. Host** | Spans em `sync`, `on_ipc`, `viewport_flush`, render GPUI e `serve`; **build release**; `notify` seletivo; leitura de asset fora da thread de UI, se confirmada | Os ~25 ms têm dono; scrub com arraste real medido |
| **3. Vídeo** | Prefetch direcional, orçamento em bytes, proxy no gesto; medir `webm`/`<video>` | Taxa de acerto e p95 em mídia real |
| **4. Spark** | Cache de RT com chave segura; fases do sort; decisão de produto sobre MSAA/LOD no gesto | Prova de determinismo; quadro final sempre completo |
| **5. Backpressure** (só se necessário) | Gate por rAF ou fence para quadros interativos | Experimento mostra fila crescendo |

Metas propostas para o quente, a validar na fase 0. **São metas, não promessas:**
- Imagem, texto e vídeo com cache: média de CPU ≤ 2 ms, p99 ≤ 4 ms.
- `first_present` e atraso em frames sem regressão.
- Sem meta de 1–4 ms para vídeo miss nem Spark em movimento.

### Protocolo de benchmark

- **Sem cherry-pick:** toda execução fica em JSONL bruto, inclusive descartadas, com motivo e load average. Ao menos
  3 passadas intercaladas A/B/A; comparar medianas das passadas; declarar ruído. Registrar energia, temperatura e carga.
- **Quente e frio:** frio = página nova, 1º e 2º quadro, 1º uso de um blend, primeiro uso de um asset. Quente = após 10
  quadros, N ≥ 300 (splats ≥ 40, declarados). Reportar p50/p95/p99/média/máx.
- **Matriz fixa:** imagem 1,6 MP e 4K; 2+ camadas; texto fixo e variável; vídeo hit e miss (PNG, JPEG, webm); cena
  procedural; Spark 100k/1M/3M parado e em movimento; 960×540 e tamanho de export. Assets versionados e sintéticos.
- **Build:** debug e release rotulados; o número de decisão é o de release.

| Tempo | Como medir | Observação |
|---|---|---|
| CPU JS | `timings` por fase, em execução sem wrappers | Quantum de 1 ms no WebKit; usar média |
| GPU, WebKit | Instruments *Metal System Trace* (`xctrace`) no GPU process; `readPixels` 1×1 como limite superior | WKWebView não expõe timer query (medido). Trace ainda não testado aqui |
| GPU, Chrome | `EXT_disjoint_timer_query_webgl2` (exposta, medido) | Não vale como número do WebKit |
| Bridge | Relógio nativo em `evaluate_script` e no handler IPC | Sem GPUI, duas corridas: atual 10,4 / 11,6–11,8 / 12–14 ms; retido 0,11–0,12 / 0,30–0,34 / 6–19 ms (p50/p95/p99) |
| Thread de UI | Spans em Rust | Faltam; é a principal lacuna |
| Apresentação | rAF após `frame.done` como proxy | Proxy, não é fóton |
| Seek-to-photon | Projeto sintético com faixa codificada por cor + ScreenCaptureKit em instância isolada, ou câmera de alta velocidade | Nunca assets do usuário. Não medido |
| Taxa de scrub | Contagem de quadros aceitos por segundo + timestamps + atraso em frames | Observador fora do caminho durante o gesto |

---

## 5. Reaproveitamento seguro, `gl.finish`, fences, timers e fallbacks

### Invalidação: identidade primeiro, sem chaves de conteúdo para assets

| Recurso | Dono | Chave | Invalidação | Limite |
|---|---|---|---|---|
| Textura de imagem | `AssetStore` (a entrada; `release` já faz `entry.texture?.dispose()`, `assets.js:123`) | a entrada | `asset.load` do mesmo id (já chama `release`), `asset.release`, `reset_project`, `dispose` | **Orçamento em bytes com LRU.** Hoje `RETAINED_ASSETS = 16` conta assets: 16×4K ≈ 530 MB de GPU |
| Material | `Compositor` | (blend, slot) | perda de contexto, `dispose`; `map`, `opacity` e `color` definidos em todo quadro | Slots por número de camadas |
| Texto | `Compositor` | valor (`content`, `size`, `color`, altura) | perda de contexto; LRU | Pequeno (o protótipo não tem limite: **não copiar**) |
| Rig de cena | entrada do asset de cena | a entrada + geração | troca do asset de cena **ou dos splats**, `dropGpu`, perda de contexto | Por cena visível |
| RT | `Compositor` | (w, h, samples) | tamanho, perda de contexto | 1 por camada de cena em uso |
| Quadro cacheado de cena | `Compositor` | camera, tempo **só se houver animação**, tamanho, identidade da cena + `assets.generation` | qualquer `asset.load`/`release`/`dropGpu`, perda de contexto | 1 |

Regras:
- **Mesh pode continuar por quadro** e a cena limpa a cada desenho: menos estado, sem custo mensurável.
- **Geração em `AssetStore`:** o protótipo não tem essa chave em `sceneCache`, então **não é seguro** copiá-lo.
- **Perda de contexto:** chamar `compositor.dropRetained()` junto de `dropSplatStage()` em `index.js:49-59`. Testei imagem
  com retenção: `context.restored` e hash igual ao de antes, nos dois motores. **Não testei** rig/splat retidos; `dropGpu`
  descarta meshes, então o rig precisa cair junto.
- **Comentário a corrigir:** `assets.js:2-3` afirma que a textura é criada por quadro. Deixa de ser verdade.
- **Mudança de asset durante quadro:** o risco de `bitmap.close()` em quadro em curso já existe hoje; `aborted()` antes do
  draw cobre o caso cancelado. Manter o teste.

### `gl.finish`, fences e readiness

**`gl.finish` não comprova GPU pronta. Evidências:**
- **Medição, WebKit:** 3M splats com `finish` 47,9 ms e sem `finish` 47,1 ms no `readPixels` 1×1 seguinte; 1M, 23,6 vs 23,4 ms.
- **Medição, Chrome:** 51,1 vs 50,4 ms (3M); 24,4 vs 24,4 ms (1M).
- **Fontes WebKit:** `Finish()` não é `Synchronous` em
  [RemoteGraphicsContextGL.messages.in](https://raw.githubusercontent.com/WebKit/WebKit/main/Source/WebKit/GPUProcess/graphics/RemoteGraphicsContextGL.messages.in),
  enquanto `ReadPixels*` e `ClientWaitSync` são. O [bug 257644](https://www2.webkit.org/show_bug.cgi?id=257644) tem o título
  "GPUP WebGL flush() and finish() are asynchronous" e segue aberto, então pode mudar.
- **Chromium:** no mesmo bug, Kenneth Russell diz que a implementação de WebGL só chama o Flush subjacente. Não vi o código-fonte.

**Recomendação:** tirar `gl.finish` do preview e o comentário "after the GPU finished it" (`compositor.js:162-163`, `:196-197`).
Usar `gl.flush()` se quisermos dispatch imediato. Não aposentar a prova PNG/hash: ela é a prova de imagem.

**Fences:**
- Pela [especificação do WebGL 2](https://registry.khronos.org/webgl/specs/latest/2.0/), um sync só pode sinalizar quando o event
  loop não está executando uma tarefa, e o timeout máximo pode ser zero. Então não há fence "síncrona" útil.
- Medido no WebKit, com polling por `MessageChannel`: 2 / 8 / 45 ms (p50/p95/p99). Mede bem o suficiente para backpressure,
  **não** para `render_ms`.
- Spark já sincroniza pela própria leitura assíncrona.

**Backpressure:**
- O risco é real em tese. Sem GPUI, o quadro retido chegou a 0,12 ms de ida e volta, sem nada que limite a fila de GPU.
- Na prática o scrub é limitado pela entrada e por um quadro em voo: não observei fila crescendo, mas não testei GPU saturada.
- Testar antes de construir. Se preciso, atrasar `frame.done` interativo para o próximo rAF, com fallback por timer, porque
  **WebView oculta não recebe rAF** (o monitor é escondido em diálogos). O core não muda.

### Fallbacks

- **`TESSEL_VIEWPORT_RETAIN=0`** (ou campo aditivo na bridge) durante o rollout, para A/B e recuo rápido; remover ao estabilizar.
- **Sem GPU timers no WebKit:** Metal System Trace e 1×1, sem fallback silencioso.
- **`checkShaderErrors`:** manter ligado. Com o programa retido, ele só custa no frio.
- **Orçamento de memória estourado:** liberar por LRU e fazer novo upload; nunca falhar o quadro.
- **Frio:** opcional aquecer programas via `renderer.compile` depois de `ready`, se o 1º quadro incomodar.

---

## 6. Export, cancelamento, provas e núcleo

### Hash: equivalência medida no protótipo

Dez quadros **fora de ordem**, `inspect` ligado, atual contra retido. Em WKWebView, Chrome/Metal e Chrome/SwiftShader,
**todos idênticos**:
imagem, 2 camadas, texto, texto variável, imagem+texto, vídeo hit/miss, caixa, Spark 100k orbitando e parado (incluindo
`sceneCache`), e `edits` (troca de blend, opacidade, crop e asset entre quadros).

Trocar o arquivo de um id devolveu a imagem nova e voltar ao arquivo original restaurou o hash. Isso **não** cobre
1M/3M, tamanho de export, `<video>` nem export completo.

### Regras

- **Mesmo caminho no preview e no export.** Não criar "modo preview" dentro do compositor, para não divergir. A retenção não
  altera o conteúdo. Fica de fora só `sceneCache`, que começa desligado em pedido explícito.
- **`frame.render` explícito, `probes` e PNG** continuam lendo pixels (`index.js:104`, `session.rs:351-352`). Nada muda.
- **Cancelamento:** `renderQueue` serializa quadros (`index.js:146`), o que torna a retenção segura sem lock. `aborted()` entre
  camadas e antes do draw permanece; quadro abortado só deixa meshes na cena, limpos no próximo. `request_frame` mantém a supersessão estrita.
- **Export:** rodar `scripts/measure-render.py` e os testes de `crates/render`. Testar sequência longa para memória: um quadro por vez
  era garantia do export (`video-export.md`) e o orçamento precisa valer ali também.
- **Testes a adicionar em `runtime/test`:** equivalência com retenção on/off, troca de asset, perda de contexto com cena e splat,
  LRU, abort no meio de múltiplas camadas.
- **Núcleo:** nada disso entra em `crates/core`, que continua sem depender de GPUI/Wry. Bridge v1 só recebe campos aditivos, nas duas
  direções, para host e runtime de versões diferentes.
- **Documentação da entrega:** atualizar F021/F030 e a matriz. Comportamento de edição não muda; superfícies de apresentação
  ficam N/A com justificativa. A nova telemetria deve ficar disponível por lib, CLI e `monitor.viewport`, sem estado paralelo.

---

## 7. Se reduzir 9 ms não resolver a latência: prioridade

**Janela real**, `seek-scrub-proof.py`, build debug, 3 corridas por célula, runtime atual contra protótipo retido:

| Entrada (poll 10 ms) | Métrica | Atual | Retido |
|---|---|---|---|
| 30 Hz | Quadros em 2 s | 52 / 53 / 59 | 58 / 59 / 59 |
| 30 Hz | Primeiro quadro (ms, amostrado) | 55–63 | 12–14 |
| 30 Hz | Atraso em frames (mediana) | 2 | 0 |
| 30 Hz | Soltar → final (ms) | 47–115 | 41–48 |
| 30 Hz | `last_frame.render_ms` | 7–8 | 0–1 |
| 120 Hz pedidos (≈80–98 reais) | Quadros em 2 s | 39 / 40 / 41 | 73 / 74 / 79 |
| 120 Hz pedidos | Atraso em frames | 3–4 | 1 |

Com poll de 250 ms: 30 Hz deu 47–55 (atual) e 54–56 (retido); 120 Hz deu 38–52 e 67–82. O observador pesa pouco na contagem.

**Leitura:**
- A taxa com entrada de 30 Hz **já estava perto do teto da própria entrada**. A retenção melhora latência e atraso, mais que a contagem.
- Com entrada alta, retido quase dobra os quadros. Ainda há ~25 ms por quadro **sem atribuição**, em debug e por socket.
- Portanto reduzir 9 ms **vale a pena**: é barato, seguro e melhora o resultado real. Só não garante scrub fluido em todo tipo de mídia.

**Ordem recomendada:**
1. Fase 0: sem isso, decidiremos por um número errado.
2. Retenção mínima (hipóteses 1–4): maior ganho por linha alterada.
3. Host em release com spans: é onde deve estar o restante.
4. Vídeo: para quem edita vídeo, é aí que o scrub real pesa. **Pode subir para o 2º lugar** se o uso principal for vídeo com cache miss.
5. Spark: cache primeiro; MSAA/LOD no gesto só com decisão explícita de qualidade.

**Não medido, portanto especulação:** quanto o release reduz os 25 ms; quanto o `cx.notify` pesa; comportamento com ponteiro real; seek-to-photon.

---

## Reproduzir

```sh
cd /tmp/tvb
node make-assets.mjs && node build.mjs       # splats/cenas e variante; PNGs vieram de assets/demo via sips
swiftc -O wk.swift -o wk
./wk runs-count.json out.jsonl               # contadores GL, WKWebView real
./wk runs-2d.json out.jsonl                  # ablação 2D
./wk runs-splat.json out.jsonl
./wk runs-bridge.json out.jsonl              # ida e volta nativa, sem GPUI
./wk runs-hash.json out.jsonl                # equivalência de pixels
./wk runs-life.json out.jsonl                # troca de asset e perda de contexto
node chrome.mjs runs-hash.json out.jsonl     # SW=1 para SwiftShader
python3 live.py                              # janela real em HOME isolado (abre uma janela própria)
python3 report.py out.jsonl
```

Resultados brutos: `/tmp/tvb/{w,c}-*.jsonl`, `/tmp/tvb/live.jsonl` e `/tmp/tvb/load.log`.
