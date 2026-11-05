# Sequência de migração por fatias verticais

> Baseada nos 121 itens de [source-feature-inventory.md](source-feature-inventory.md) e na matriz [feature-migration-matrix.md](feature-migration-matrix.md). Contexto: [README](README.md). **Nada aqui autoriza implementação**; é o plano de ordem e de aceite.

## 1. Princípios

1. **Do dado para a pele**: modelo/core/compatibilidade com fixtures → lib/CLI headless → UI. Cada fatia só termina com evidência em cada superfície que ela toca; `ui op`/socket não conta como headless.
2. **Oráculo executável**: a origem gera golden (veredito de validação, pose por frame, hashes de pixel). O destino compara contra eles. A origem continua intocada.
3. **JS reutilizável ≠ port de domínio**: *desenho* (shaders, camadas WebGL, Spark) é reaproveitado no runtime JS do destino; *regra* (validação, tempo, câmera, comandos, mix) é reimplementada no core Rust. **Não se decide reescrever Spark em Rust.** A coluna *Tipo* abaixo diz qual é qual.
4. **Prioridade por fluxo real, não por preferência**: o único fluxo ponta a ponta verificado na própria origem é *abrir bundle → pré-visualizar com transporte e áudio → editar câmera/luz/clima/narração → exportar MP4 com áudio* (README do pacote; entregas `deliveries/*`: pilot, export, transport, light-01..03, weather-01..02, splat, pipeline). Itens ligados a esse fluxo são **essenciais**; os recursos de cada entrega são **seguintes**; o que só serve a uma produção específica (mão que escreve) ou a um branch não mergeado é **futuro**; ferramentas e infra do dev server são **não migrar**. O histórico (100 commits em ~4 dias, 2026-10-30 a 11-02) mostra um código jovem e volátil: re-verificar o baseline antes de cada fatia.
5. **Fora de escopo**: produtos Tessel que não pertencem ao editor (pipeline de produção por IA VGP-01…11, workers, R2/`wrangler`, `ai-service`).

## 2. Visão geral das fatias

| Fatia | Título | Pré-requisitos | Classificação | MIGs | Tipo (JS reutilizável / Rust / misto / ferramenta / UI-N/A) |
|---|---|---|---|---|---|
| S0 | Contrato de compatibilidade, fixtures e validação | — | essencial / não migrar | 8 | Port de domínio Rust 5, Misto 1, Ferramenta/oráculo 1, UI/adapter ou N/A 1 |
| S1 | Núcleo temporal e de câmera (puro, sem GPU) | S0 | essencial | 7 | Port de domínio Rust 6, Misto 1 |
| S2 | Propriedades, comandos em lote e CLI (agentes) | S0, S1 | essencial / seguinte / não migrar | 11 | Port de domínio Rust 10, UI/adapter ou N/A 1 |
| S3 | Runtime de planos Three.js e determinismo de quadro | S0, S1 | essencial / seguinte | 4 | JS reutilizável 2, Misto 1, Ferramenta/oráculo 1 |
| S4 | Transporte, áudio de preview e timeline mínima | S3 | essencial / seguinte / não migrar | 7 | Port de domínio Rust 2, Misto 1, UI/adapter ou N/A 4 |
| S5 | Câmera: faixa, overlay e inspector | S2, S4 | seguinte | 3 | UI/adapter ou N/A 3 |
| S6 | Narração: modelo, takes, legendas e mix | S2, S4 | seguinte / futuro | 9 | Port de domínio Rust 6, Misto 1, UI/adapter ou N/A 2 |
| S7 | Export e paridade de render | S3, S4 (e S6 se houver narração) | essencial | 3 | Port de domínio Rust 1, Misto 1, UI/adapter ou N/A 1 |
| S8 | Shell do workstation: seleção, camadas/outline, inspector, store, undo | S2, S5 | seguinte / futuro / não migrar | 11 | Port de domínio Rust 3, UI/adapter ou N/A 8 |
| S9 | Iluminação e sombras | S3, S2 | seguinte | 5 | JS reutilizável 2, Port de domínio Rust 1, Misto 1, UI/adapter ou N/A 1 |
| S10 | Clima | S3, S2 | seguinte | 5 | JS reutilizável 2, Port de domínio Rust 2, Misto 1 |
| S11 | Efeitos de pós-produção (3 looks) | S3, S2 | seguinte | 2 | Misto 1, UI/adapter ou N/A 1 |
| S12 | Splat (câmera, luz e clima em splat) | S3, S9 | seguinte / futuro | 4 | JS reutilizável 3, Port de domínio Rust 1 |
| S13 | Rig, paths e follow (mão que escreve) | S2, S3 | futuro | 11 | JS reutilizável 2, Port de domínio Rust 6, Misto 1, UI/adapter ou N/A 2 |
| S14 | Projetos múltiplos e servidor de preview | S8 | futuro / não migrar | 4 | UI/adapter ou N/A 4 |
| S15 | Delta do branch piloto (fx, shots, gestos) — somente após D1 | D1, S2, S8 | futuro | 7 | JS reutilizável 1, Port de domínio Rust 2, Misto 2, UI/adapter ou N/A 2 |
| S16 | Ferramentas externas e oráculos (fora do app) | — | não migrar | 6 | Ferramenta/oráculo 5, UI/adapter ou N/A 1 |
| — | Ausentes (sem fatia: nada a migrar) | — | ausente | 14 | — |
| **Total** | | | | **121** | |

Fecha com o inventário: 107 em fatias + 14 ausentes = **121** = 121 MIGs.

```text
S0 ──> S1 ──> S2 ──┬─> S5 ──┐
           │       ├─> S6 ──┼─> S8
           └─> S3 ─┼─> S4 ──┘
                   ├─> S7 (export; usa S6 se houver narração)
                   ├─> S9 ─> S12
                   ├─> S10, S11
                   └─> S13
S14 (só se D10)   S15 (só após D1)   S16 (nunca migra)
```

Ordem recomendada de execução: **S0 → S1 → S2 → S3 → S4 → S7** (caminho essencial: abre, avalia, desenha, toca, exporta) e só então S5, S6, S8 e os recursos visuais S9–S11; S12 depois de S9; S13 e S14 por último; S15 após D1.

## 3. Detalhe das fatias

### S0 — Contrato de compatibilidade, fixtures e validação

- **Pré-requisitos:** —
- **Objetivo:** Ler o `render-bundle.json` v1.0 em Rust, validar com os mesmos códigos e abrir os fixtures sem GPU. Decide D2 e a política de campos desconhecidos/inativos.
- **Contratos e formatos:** `render-bundle.json` schemaVersion `1.0` (zod, 599 linhas); ~70 códigos de integridade; preflight (sha256, dimensões, alpha, hash do WAV); formatos png/spz/f32; recusas explícitas (premultiplied, rec709-video, dissolve).
- **Evidência de paridade exigida:** Golden de veredito: para N bundles (fixture técnico, narrado, hands, inválidos induzidos) o mesmo ok/erro e o mesmo `code` do zod; round-trip JSON sem perda; hashes dos fixtures da origem registrados.
- **Superfícies (ordem):** lib → CLI headless (`validate`, `show`) → sem UI.
- **Engine/JS reutilizável vs port Rust:** Nenhum JS no caminho; o oráculo (zod) roda na origem para gerar os golden.
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG001 | Modelo do documento `render-bundle.json` (schema 1.0) | essencial | Port de domínio Rust | Os fixtures do `generate-technical-fixture.mjs` e os bundles dos testes parseiam em Rust com o mesmo veredito (ok/erro + primeira mensagem)  |
  | MIG002 | Regras de integridade do bundle e de junções de edição | essencial | Port de domínio Rust | Para cada código, um bundle mínimo que o dispara e o mesmo código em Rust; bundle válido não gera nenhum. |
  | MIG003 | Arquivo do bundle: abrir, escrita atômica serializada, dry-run e finalize | essencial | Port de domínio Rust | Edição por `--dry-run` e por escrita produzem o mesmo bundle; kill no meio da escrita nunca deixa JSON parcial; fila serializa duas edições  |
  | MIG004 | Preflight de assets (hash, dimensões, alpha, hash PCM, confinamento de caminho) | essencial | Port de domínio Rust | Asset adulterado (byte, tamanho, alpha) é recusado com o mesmo código; caminho com `..` ou symlink fora da pasta é recusado. |
  | MIG005 | Matriz de formatos de asset e recusas explícitas | essencial | Misto | PNG straight sRGB e mapa linear de dados renderizam com os mesmos pixels do oráculo TS; premultiplied é recusado com o mesmo código. |
  | MIG010 | Scripts de fixture e importação de produções locais | essencial | Ferramenta/oráculo | Fixture técnico regenerado a partir da origem tem hash estável registrado no destino. |
  | MIG019 | Perfil de render (resolução, fps racional, cor, codec) | essencial | Port de domínio Rust | Bundle com rec709-video é recusado na importação com o mesmo código. |
  | MIG024 | Campos e tipos aceitos pelo schema sem consumidor no runtime/UI | não migrar | UI/adapter ou N/A | Bundle com esses campos abre; o campo é preservado ou recusado com código estável, e o teste prova qual. |

### S1 — Núcleo temporal e de câmera (puro, sem GPU)

- **Pré-requisitos:** S0
- **Objetivo:** Mapear frame de edição → shot → instância/câmera, performance tracks, câmera segmentada (8 receitas + keyframes + 3 camadas), easings, tempo racional.
- **Contratos e formatos:** `shots[]`, `sceneInstances`, `performanceTracks`, `cameraClips`; `resolveCameraAtClipFrame`; 3 easings; fps racional.
- **Evidência de paridade exigida:** Tabela dourada gerada na origem: (clipe × frame) → pose; (frame) → (shotId, instanceFrame, cameraClipFrame); tolerância 1e-9 em posição/target e 1e-6 em fov; casos de borda (frame fora da faixa, sem shot).
- **Superfícies (ordem):** lib → CLI headless (`plan`/`props` com `--frame`) → sem UI.
- **Engine/JS reutilizável vs port Rust:** Nenhum.
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG012 | Grafo de cena: nós, hierarquia, sockets, quaternions e ordem de desenho | essencial | Misto | Para bundles de teste, as matrizes efetivas por nó (frame 0 e N) em Rust coincidem com `EffectiveMatrices` do runtime TS dentro de 1e-6. |
  | MIG016 | Avaliação do frame de edição (shots → instância/câmera → estado dos nós) | essencial | Port de domínio Rust | Para todo frame do fixture, (shotId, instanceFrame, cameraClipFrame, poses) idênticos ao oráculo. |
  | MIG017 | Performance tracks (ciclo, evento com hold, estático) | essencial | Port de domínio Rust | Valores idênticos ao oráculo para a grade de frames do spec. |
  | MIG018 | Estrutura de shots, instâncias de cena e transições (somente leitura) | essencial | Port de domínio Rust | Bundle com N shots navega por `stepShot` nos mesmos frames; validação de junções com os mesmos códigos. |
  | MIG030 | Tempo racional, FPS e conversões | essencial | Port de domínio Rust | Tabela de (frame, fps) → segundos idêntica à origem nos 24/25/30/30000:1001/60. |
  | MIG031 | Clipe de câmera: pose inicial, segmentos (8 receitas + keyframes) e até 3 camadas | essencial | Port de domínio Rust | Para todos os clipes dos specs, pose por frame em Rust = TS (\|Δ\| ≤ 1e-9 em posição/target, 1e-6 em fov). |
  | MIG032 | Resolução da câmera por frame e easings | essencial | Port de domínio Rust | Tabela dourada de poses (clipes × frames) gerada na origem bate em Rust. |

### S2 — Propriedades, comandos em lote e CLI (agentes)

- **Pré-requisitos:** S0, S1
- **Objetivo:** Sistema de property paths, `editBundle` atômico, comandos de câmera (15), narração (10), outline (6), keyframes e a CLI `scene` (14 comandos).
- **Contratos e formatos:** Caminhos `scene/<s>/…`, `camera/<c>/…`; formas `{set}`, `{camera}`, `{narration}`, `{outline}`; erros tipados {index, layer, code, message, hint}; saída JSON e códigos de saída 0/1/2.
- **Evidência de paridade exigida:** Corpus de lotes extraído dos specs (camera-commands, outline, narration, properties): mesmo bundle resultante ou mesmo código; lote com falha não muta; `props` do fixture lista os mesmos paths/kinds/limites.
- **Superfícies (ordem):** lib → CLI headless → (UI só na fatia seguinte). O headless do destino não tem transação de lote hoje: pré-requisito.
- **Engine/JS reutilizável vs port Rust:** Nenhum.
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG033 | Comandos de câmera (15 ops) com erros tipados e dica | seguinte | Port de domínio Rust | Corpus de lotes de comando (do spec) produz o mesmo clipe resultante ou o mesmo código de erro; rejeição não muta. |
  | MIG034 | Propriedades de câmera por caminho (`camera/<clip>/…`) | seguinte | Port de domínio Rust | Os caminhos listados por `props` na origem aparecem com os mesmos tipos/limites. |
  | MIG035 | Análise de clipe de câmera | seguinte | Port de domínio Rust | Mesmo JSON de análise para os clipes de teste. |
  | MIG042 | Outline: grupos de camadas, luzes e áudio com ocultar/silenciar/recolher | seguinte | Port de domínio Rust | Corpus de comandos do spec dá a mesma árvore ou o mesmo erro. |
  | MIG043 | Comandos de outline (6 ops) | seguinte | Port de domínio Rust | Mesmos resultados/erros do spec. |
  | MIG046 | Sistema de property paths (listar, ler, escrever, erros tipados) | seguinte | Port de domínio Rust | `scene props` do fixture lista os mesmos paths/kinds/limites; `set` inválido devolve o mesmo código; escrita rejeitada não muta. |
  | MIG047 | Keyframes de valores (resolver, regras, adicionar/remover no playhead) | seguinte | Port de domínio Rust | Tabela dourada de interpolações e de refusais. |
  | MIG093 | CLI `scene` (14 comandos, JSON em stdout, exit 0/1/2) | essencial | Port de domínio Rust | Cada um dos 14 comandos tem equivalente no headless com o mesmo corpo JSON para o fixture. |
  | MIG094 | API de edição em lote `editBundle` (6 formas de edit) | essencial | Port de domínio Rust | Lote com erro no item 3 não altera nada e devolve `index: 2`. |
  | MIG095 | Consultas `show` e `summarize` do bundle (8 camadas) | seguinte | Port de domínio Rust | Mesmo JSON. |
  | MIG096 | Exports programáticos do pacote (core, node, runtime, transport, testing) | não migrar | UI/adapter ou N/A | Índice de símbolos coberto pelas features migradas. |

### S3 — Runtime de planos Three.js e determinismo de quadro

- **Pré-requisitos:** S0, S1
- **Objetivo:** Desenhar planos PNG com alpha, grafo de cena, auditoria de projeção e a API do runtime dentro do runtime/bridge v1 do destino; prova de pixel.
- **Contratos e formatos:** Grafo de cena (nós, sockets, quaternions, `renderOrder`); fila de pintura sem depth write; espaço de cor sRGB; `frameAudit`; setters do runtime viram campos do plano (bridge v1).
- **Evidência de paridade exigida:** Hash de pixels por frame do fixture técnico igual ao oráculo no mesmo ambiente (SwiftShader); seek = scrub = export; matrizes efetivas por nó dentro de 1e-6.
- **Superfícies (ordem):** runtime JS + lib → CLI `render.frame` → monitor da janela (reaproveita o monitor único).
- **Engine/JS reutilizável vs port Rust:** Reuso direto do `ThreeSceneRuntime` (planos, material, auditoria), adaptado ao runtime do destino (decisão D3/D4).
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG013 | Camadas de plano de imagem (PNG com alpha, fila de pintura única) | essencial | JS reutilizável | Bundle técnico renderiza os mesmos pixels (hash por frame) que o oráculo no mesmo ambiente de GPU (SwiftShader). |
  | MIG020 | Auditoria de projeção e `frameAudit` por frame | seguinte | Misto | Mesmo veredicto e mesma mensagem para quadros do fixture, incluindo falhas induzidas. |
  | MIG022 | Determinismo de quadro e prova de pixel | essencial | Ferramenta/oráculo | Quadro N pelo seek direto = quadro N após percurso; comparação com o oráculo no mesmo ambiente. |
  | MIG023 | API do runtime Three.js (setters de cena, captura, descarte) | essencial | JS reutilizável | Cada setter vira campo do plano; mesmo quadro com setter em memória e com plano equivalente. |

### S4 — Transporte, áudio de preview e timeline mínima

- **Pré-requisitos:** S3
- **Objetivo:** Comandos faltantes (`jump_seconds`, `step_shot`, scrub com retomada), waveform, atalhos, campo de frame, timecode.
- **Contratos e formatos:** Fases do transporte, `seekToFrame` com invalidação de operações em voo, janela de áudio do bundle, ganho do monitor.
- **Evidência de paridade exigida:** Sequência play→seek→pause→step→jump→end gera os mesmos `editFrame`/fase do spec; amostra tocada no frame N = plano de áudio (±1 ms).
- **Superfícies (ordem):** lib → CLI → UI (GPUI timeline/transport existente).
- **Engine/JS reutilizável vs port Rust:** Nenhum (usa `crates/audio` do destino).
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG021 | Coordenador de preview (latest-wins, descarte de quadro velho, métricas) | não migrar | UI/adapter ou N/A | N/A; manter as métricas equivalentes no painel de debug se o painel for migrado. |
  | MIG025 | Controlador de transporte (play, pause, seek, passo, salto, shots, scrub) | essencial | Misto | Sequência play→seek→pause→step→jump→end produz o mesmo editFrame/fase que o spec TS; scrub devolve ao estado anterior. |
  | MIG026 | Porta de áudio do navegador (relógio de áudio, ganho do monitor, janela de edição) | essencial | Port de domínio Rust | Para o fixture com áudio, a amostra tocada no quadro N coincide com o plano de áudio do oráculo (tolerância de 1 ms). |
  | MIG027 | Waveform da timeline com scrub e seek | seguinte | Port de domínio Rust | Pico por coluna idêntico ao oráculo para o WAV do fixture. |
  | MIG028 | Atalhos de teclado e regras de foco | seguinte | UI/adapter ou N/A | Cada atalho dispara a mesma ação semântica que o botão, sem agir com foco em campo de texto. |
  | MIG029 | Barra de transporte, campo de frame, timecode e controle de volume | seguinte | UI/adapter ou N/A | Mesmo timecode do oráculo para uma grade de frames em 24, 25 e 30000/1001. |
  | MIG052 | Leitura de debug do transporte | não migrar | UI/adapter ou N/A | N/A. |

### S5 — Câmera: faixa, overlay e inspector

- **Pré-requisitos:** S2, S4
- **Objetivo:** UI de câmera sobre os comandos de S2: barras na timeline, handles do caminho, mini-mapa e inspector.
- **Contratos e formatos:** Comandos de câmera; `propertyPath` dos handles; recusas com `hint`.
- **Evidência de paridade exigida:** Cada gesto produz o mesmo comando que a CLI; undo em um passo; recusa mostra o mesmo hint.
- **Superfícies (ordem):** UI (a lib e a CLI já vêm de S2).
- **Engine/JS reutilizável vs port Rust:** Overlay: decisão D11 (desenhar no runtime JS ou numa camada GPUI).
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG036 | Faixa de câmera na timeline (barras, arrasto, redimensionar, inserir, camadas) | seguinte | UI/adapter ou N/A | Arrastar uma barra produz o mesmo `moveSegment` que o comando; recusa mostra o mesmo hint. |
  | MIG037 | Overlay do caminho 3D (handles arrastáveis) e mini-mapa CameraPlan | seguinte | UI/adapter ou N/A | Arrastar um handle gera exatamente o `moveKey` esperado e desfaz em um passo. |
  | MIG038 | Inspector de câmera | seguinte | UI/adapter ou N/A | Controle→comando 1:1 com a CLI. |

### S6 — Narração: modelo, takes, legendas e mix

- **Pré-requisitos:** S2, S4
- **Objetivo:** Faixas/linhas, plano de mix (a voz nunca soa duas vezes), takes de TTS (adapter injetado, D5), cortar do mix, SRT e legendas.
- **Contratos e formatos:** `narration` do bundle; `audioMixPlan`; API `POST /audio/speech` (OpenAI-compatível) e `/voices`; `X-Timings`; tipografia da legenda.
- **Evidência de paridade exigida:** Amostras do mix = oráculo em janelas de silêncio; legenda no export = preview (tolerância de fonte declarada); com TTS falso a linha recebe WAV+hash.
- **Superfícies (ordem):** lib → CLI → UI.
- **Engine/JS reutilizável vs port Rust:** Legenda: uma só implementação de pintura (JS) para preview e export.
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG081 | Modelo de narração (faixas, linhas, palavras, entrega, fromMix, legenda) | seguinte | Port de domínio Rust | Corpus de validação do spec. |
  | MIG082 | Comandos de narração (10 ops) | seguinte | Port de domínio Rust | Mesmos resultados/erros. |
  | MIG083 | Plano de mix: o que soa, onde a voz do mix cala porque há take | seguinte | Port de domínio Rust | Amostras do mix = oráculo em janelas de silêncio. |
  | MIG084 | Gerar takes de TTS (cliente OpenAI-compatível) e gravar no bundle | seguinte | Port de domínio Rust | Com servidor TTS fake, a linha recebe WAV + hash e `setClipAudio`; sem servidor, erro tipado. |
  | MIG085 | Cortar o take da própria voz do mix | seguinte | Port de domínio Rust | Recorte tem a mesma janela de amostras. |
  | MIG086 | Importar legendas SRT como linhas de narração | futuro | Port de domínio Rust | Mesmos comandos gerados para os SRTs do spec. |
  | MIG087 | Legendas: revelação por palavra, layout, overlay no Studio e pintura no export | seguinte | Misto | Quadro com legenda no export = preview por pixel (tolerância de fonte declarada). |
  | MIG088 | Reprodução dos takes de narração no Studio | seguinte | UI/adapter ou N/A | Take toca no frame correto (±1 amostra do plano). |
  | MIG089 | UI de narração (faixas, linhas, inspector de linha com voz/estilo/entrega) | seguinte | UI/adapter ou N/A | Toda ação com equivalente na CLI. |

### S7 — Export e paridade de render

- **Pré-requisitos:** S3, S4 (e S6 se houver narração)
- **Objetivo:** Exportar o fixture pelo caminho do destino (F029) e provar equivalência com o `scene render` da origem, incluindo marcação de cor.
- **Contratos e formatos:** H.264 yuv420p, matriz BT709, transferência sRGB (`iec61966-2-1`), faixa `tv`, +faststart; verificação ffprobe de contrato; amostras PNG.
- **Evidência de paridade exigida:** RGBA por frame do export = oráculo; tags de cor presentes; duração/fps/áudio verificados; falha de verificação não publica arquivo.
- **Superfícies (ordem):** lib → CLI `export.start` → UI (diálogo Exportar existente).
- **Engine/JS reutilizável vs port Rust:** Reuso do runtime do destino (`crates/render`); nada a portar do `render.ts`.
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG090 | Render offline: Chrome + Three.js quadro a quadro → ffmpeg H.264 | essencial | Misto | Export do fixture no destino = oráculo (pixels RGBA por frame, tags de cor, duração, áudio) com o mesmo hash por ambiente. |
  | MIG091 | Verificação do arquivo final (ffprobe) e contrato de cor | essencial | Port de domínio Rust | Arquivo exportado no destino passa a mesma verificação, incluindo `COLOR_TAGS_INCORRECT`. |
  | MIG092 | Grafo de áudio do export (mix + takes, silêncios, AAC) | essencial | UI/adapter ou N/A | Mix do fixture com o mesmo envelope/offset de take. |

### S8 — Shell do workstation: seleção, camadas/outline, inspector, store, undo

- **Pré-requisitos:** S2, S5
- **Objetivo:** Modelo de seleção, painel de camadas/árvore com DnD (mouse e teclado), outline, inspector declarativo, pipeline de commit/preview transitório e unidades de undo.
- **Contratos e formatos:** `StudioSelection` (12 tipos); `outline` (grupos layers/lights/audio); `PropertyChange` por domínio; granularidade de undo (um passo por arraste).
- **Evidência de paridade exigida:** Cada ação da árvore/inspector tem comando semântico equivalente; arrasto → N previews → 1 transação; falha de gravação restaura.
- **Superfícies (ordem):** UI + CLI da janela; lib já existe de S2.
- **Engine/JS reutilizável vs port Rust:** Nenhum (GPUI).
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG011 | Layout de painéis persistido no navegador | não migrar | UI/adapter ou N/A | N/A. |
  | MIG039 | Modelo de seleção (12 tipos) e inspector contextual | seguinte | Port de domínio Rust | Cada tipo de seleção existe como nó semântico (`tree.rs`) com comando de seleção. |
  | MIG040 | Painel de camadas (Cena, Câmera, Clima, Efeitos, árvore de nós, luzes, áudio/texto) | seguinte | UI/adapter ou N/A | Cada ação da árvore existe como comando semântico equivalente na CLI. |
  | MIG041 | Árvore de camadas: arrastar e soltar (mouse e teclado), renomear com F2, ações por linha | seguinte | UI/adapter ou N/A | Dropar 'dentro' de grupo = `moveItem {groupId}`; o teclado alcança todas as posições. |
  | MIG044 | Inspector de cena, nó e grupo | seguinte | UI/adapter ou N/A | Mesmos campos e estados desabilitados. |
  | MIG045 | Infra do inspector (schema declarativo sobre Leva) | não migrar | UI/adapter ou N/A | N/A. |
  | MIG048 | Store do editor (mostrar, comprometer, salvar, reverter) | seguinte | Port de domínio Rust | Arrasto → N previews → 1 transação; falha de gravação restaura o último estado salvo. |
  | MIG049 | Desfazer e refazer (200 passos, por projeto) | seguinte | Port de domínio Rust | Mesmas unidades de undo (um passo por arraste/comprometimento). |
  | MIG050 | Pipeline de salvamento por domínio e edição em lote | não migrar | UI/adapter ou N/A | N/A. |
  | MIG051 | Internacionalização pt-br/en/es (383 chaves) | futuro | UI/adapter ou N/A | Paridade de chaves entre idiomas. |
  | MIG053 | API global de teste `window.__sceneStudio` | não migrar | UI/adapter ou N/A | N/A. |

### S9 — Iluminação e sombras

- **Pré-requisitos:** S3, S2
- **Objetivo:** Ambiente + até 4 luzes pontuais, keys, mapas de normal/relevo, sombras de contato e projetada, luz em splat (S12).
- **Contratos e formatos:** `lighting` (ambient 0–1,5; luz: cor 0–1, intensidade 0–20, raio, flicker 0–1, hidden, parent, keys); `normalMap`, `relief`; `shadows.casters ≤ 4`.
- **Evidência de paridade exigida:** Pixel igual ao oráculo com 1–4 luzes, luz oculta/em grupo oculto, parent e keys.
- **Superfícies (ordem):** lib → CLI (`set …/light/…`) → UI (seção, overlay, inspector).
- **Engine/JS reutilizável vs port Rust:** Reuso dos shaders `plane-effects-material.ts` e `plane-shadows.ts` (562 + 348 linhas, sem teste unitário).
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG054 | Modelo de iluminação da cena (ambiente + até 4 luzes pontuais) | seguinte | Misto | Cena com 1–4 luzes em hash de pixel igual ao oráculo; luz `hidden` ou em grupo oculto não ilumina. |
  | MIG055 | Keys de luz (intensidade, cor, posição) e luz que segue uma camada | seguinte | Port de domínio Rust | Luz com chaves e parent coincide com o oráculo por frame. |
  | MIG056 | Planos iluminados: mapas de normais e relevo | seguinte | JS reutilizável | Pixel igual ao oráculo com luzes e mapas do fixture de luz. |
  | MIG057 | Sombras de personagens: contato no chão e projetada (até 4 casters) | seguinte | JS reutilizável | Pixel igual ao oráculo em cena de luzes + casters. |
  | MIG059 | UI de luzes: seção, marcador arrastável no viewport e inspector | seguinte | UI/adapter ou N/A | Arrastar o marcador = `set …/light/<l>/position`; undo em 1 passo. |

### S10 — Clima

- **Pré-requisitos:** S3, S2
- **Objetivo:** Gramática do clima, chuva/neve por fatias de plano, relâmpago, grade de cor, superfícies (poças, neve acumulada), chão, keys.
- **Contratos e formatos:** `weather` (nome), `weatherKeys`, `weatherGround`, `node.surface`; funções puras de (partícula, tempo de edição).
- **Evidência de paridade exigida:** Quadro N idêntico por seek/scrub/export; pixel = oráculo nos 3 frames do fixture de clima.
- **Superfícies (ordem):** lib → CLI → UI.
- **Engine/JS reutilizável vs port Rust:** Reuso de `weather-layer.ts`, `weather-model.ts`, `weather-volume.ts` e do shader de superfície; gramática em Rust.
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG061 | Gramática do clima (palavras, intensidade, erro de digitação) | seguinte | Port de domínio Rust | Tabela de nomes aceitos/recusados idêntica. |
  | MIG062 | Renderização do clima em planos (chuva, neve, relâmpago, grade de cor) | seguinte | JS reutilizável | Quadro N idêntico por seek/scrub/export; pixel = oráculo no fixture. |
  | MIG063 | Superfícies do clima: neve que acumula, poças com reflexo e respingos | seguinte | JS reutilizável | Fixture de clima com surface map igual ao oráculo em 3 frames. |
  | MIG064 | Linha do chão do clima (para plano sem mapa) | seguinte | Misto | Mesmo `top` produz a mesma linha. |
  | MIG065 | Keys de intensidade do clima | seguinte | Port de domínio Rust | Mesmo nível por frame. |

### S11 — Efeitos de pós-produção (3 looks)

- **Pré-requisitos:** S3, S2
- **Objetivo:** Catálogo ink/dream/film com amounts 0–1 e passe de pós.
- **Contratos e formatos:** `effects` estrito (strictObject); ordem do catálogo = ordem do passe; custo e limites documentados.
- **Evidência de paridade exigida:** Cada look em 0,5 e 1,0 = oráculo; `effects` lista o mesmo catálogo.
- **Superfícies (ordem):** lib → CLI → UI (sliders).
- **Engine/JS reutilizável vs port Rust:** Reuso dos GLSL (`ink`, `dream`, `film`, `common`).
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG067 | Looks de pós-produção (tinta, sonho, filme) com catálogo | seguinte | Misto | Os 3 looks em amount 0,5 e 1 igualam o oráculo; `effects` lista o mesmo catálogo. |
  | MIG068 | Inspector de efeitos (sliders por look) | seguinte | UI/adapter ou N/A | Mesmos controles e limites. |

### S12 — Splat (câmera, luz e clima em splat)

- **Pré-requisitos:** S3, S9
- **Objetivo:** Nó splat dentro de cena de planos, limites de câmera, luz e clima em splat. O Spark 2.4.0 já está no destino (F030): **não** se reescreve Spark em Rust.
- **Contratos e formatos:** Nó `splat` (rotação {1,0,0,0}), `sceneDepth` f32, limites safe/caution.
- **Evidência de paridade exigida:** Cena SHARP do fixture = oráculo; seek e export iguais.
- **Superfícies (ordem):** lib → CLI → UI.
- **Engine/JS reutilizável vs port Rust:** Reuso do estágio de splat do destino + modificadores (`splat-lighting`, `splat-weather-*`).
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG014 | Camadas de splat Gaussiano (Spark 2.4, `.spz`) | seguinte | JS reutilizável | Cena SHARP do fixture renderiza o mesmo hash que o oráculo; seek e export produzem a mesma imagem. |
  | MIG015 | Limites de câmera sobre splat (zonas safe/caution) | seguinte | Port de domínio Rust | Mesmos veredictos safe/caution/unsafe para a malha de poses do spec. |
  | MIG058 | Luz dinâmica em splats (Spark world modifier) | seguinte | JS reutilizável | Pixel igual ao oráculo na cena SHARP com 1 luz. |
  | MIG066 | Clima em cenas de splat (colisão por profundidade, respingos, neve, poças) | futuro | JS reutilizável | Quadro de chuva em splat = oráculo. |

### S13 — Rig, paths e follow (mão que escreve)

- **Pré-requisitos:** S2, S3
- **Objetivo:** Ossos, IK, bake determinístico, binding rígido/esfolado, paths, follow, reveal e UI correspondente.
- **Contratos e formatos:** `rigs[]`, `paths[]`, `rigBind`, `reveal`; 11 ops de rig, 2 ops de path; `BAKE_VERSION`; limites ±180°; reach report/clamp.
- **Evidência de paridade exigida:** Trilha assada igual (tolerância declarada) para os rigs do fixture 'hands'; pixel igual na cena com manga esfolada; D7 decide solver em JS ou Rust.
- **Superfícies (ordem):** lib → CLI → UI.
- **Engine/JS reutilizável vs port Rust:** IK: `CCDIKSolver` do Three.js 0.187 (reusar em worker) ou port com tolerância (D7).
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG069 | Rig: ossos sobre uma camada, limites, punho e pino | futuro | Port de domínio Rust | Rigs do fixture 'hands' carregam, validam e produzem a mesma lista de ossos/limites. |
  | MIG070 | IK (CCDIKSolver) e bake determinístico da pose | futuro | JS reutilizável | Mesma trilha (\|Δ ângulo\| ≤ tolerância declarada) para os rigs do fixture; hash de bake estável. |
  | MIG071 | Binding de camadas ao rig (rígido ou esfolado) e RigLayer | futuro | JS reutilizável | Pixel = oráculo com a manga esfolada em 3 poses. |
  | MIG072 | Comandos e propriedades de rig (11 ops + caminhos `rig/<id>/…`) | futuro | Port de domínio Rust | Corpus de comandos do spec. |
  | MIG073 | Validação e diagnósticos do rig | futuro | Port de domínio Rust | Mesmas issues/códigos nos rigs de teste. |
  | MIG074 | UI de rig (menu, árvore, overlay de ossos, faixa de alvos, inspector, vista pose/repouso) | futuro | UI/adapter ou N/A | Todo gesto tem comando semântico. |
  | MIG075 | Paths sobre a imagem de uma camada (catmull-rom, linear, bezier, caneta levantada) | futuro | Port de domínio Rust | Amostragem idêntica por interpolação. |
  | MIG076 | Gerar o primeiro path de um texto a partir dos pixels da camada | futuro | Port de domínio Rust | Mesmos pontos para as máscaras de teste. |
  | MIG077 | Follow: a ponta de um osso caminha pelo path (chaves de progresso, alcance) | futuro | Port de domínio Rust | Progresso por frame e issues idênticos. |
  | MIG078 | Reveal: a tinta aparece quando a caneta já passou | futuro | Misto | Frame N revela os mesmos pixels do oráculo. |
  | MIG079 | UI de follow e path (menu, árvore, overlay de pontos, faixa de progresso) | futuro | UI/adapter ou N/A | Todo gesto tem comando. |

### S14 — Projetos múltiplos e servidor de preview

- **Pré-requisitos:** S8
- **Objetivo:** Só se D10 mantiver multi-projeto: abas, trava de escrita, duplicar. O servidor HTTP e a hibernação de runtime não migram.
- **Contratos e formatos:** `projects.json`, `BUNDLE_LOCKED`.
- **Evidência de paridade exigida:** Abrir o mesmo arquivo duas vezes → um escritor + um somente leitura.
- **Superfícies (ordem):** UI.
- **Engine/JS reutilizável vs port Rust:** Nenhum.
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG006 | Registro de projetos do Studio (abrir, duplicar, renomear, fechar; trava de escrita) | futuro | UI/adapter ou N/A | Abrir o mesmo arquivo duas vezes resulta em um escritor e um somente leitura; fechar não apaga arquivos; duplicar copia config. |
  | MIG007 | Abas de projeto e menu Project na UI | futuro | UI/adapter ou N/A | Se migrado: troca de aba preserva seleção e playhead do projeto (comportamento atual). |
  | MIG008 | Hibernação de runtime e stores por projeto (limite de 2 projetos ativos) | não migrar | UI/adapter ou N/A | N/A. |
  | MIG009 | Servidor de preview HTTP local (rotas do bundle, assets e salvamentos) | não migrar | UI/adapter ou N/A | N/A; critério: nenhuma regra de domínio fica só na casca (já é o caso na origem). |

### S15 — Delta do branch piloto (fx, shots, gestos) — somente após D1

- **Pré-requisitos:** D1, S2, S8
- **Objetivo:** Reanalisar o delta quando congelar (50 commits de 2026-11-02 a 11-04) e então decidir. Hoje catalogado só em nível de catálogo.
- **Contratos e formatos:** `fx` (19 ops), 59 efeitos (31 looks + 28 transições), `shots` (4 ops), gestos puros, CLI `fx|shots|keys|follow|inspect`.
- **Evidência de paridade exigida:** A definir após D1; mínimo: golden de pixels `fx/__baseline__/pixels.json`.
- **Superfícies (ordem):** —
- **Engine/JS reutilizável vs port Rust:** Shaders do catálogo reutilizáveis; compositor de passes a avaliar.
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG100 | Camada de efeitos por clipe (faixas, clipes, parâmetros, envelope, escopo, chaves) | futuro | Misto | Definido só após D1; no mínimo, golden pixels do `__baseline__/pixels.json`. |
  | MIG101 | Catálogo de 59 efeitos (31 looks + 28 transições) com baseline de pixel | futuro | JS reutilizável | Cada efeito reproduz seu pixel dourado. |
  | MIG102 | Edição de shots: aparar bordas, mover corte, duração do shot e da edição (ripple/roll) | futuro | Port de domínio Rust | Comando de aparar produz o mesmo bundle (shots, fx, keys, narração) do oráculo. |
  | MIG103 | Gestos de timeline como funções puras (arrastar shot, clipe de fx, transição, chaves) | futuro | Port de domínio Rust | Mesmo frame legal para a grade de entradas do spec. |
  | MIG104 | Timeline do Studio com faixas de shots, efeitos e transições, e inspector de item | futuro | UI/adapter ou N/A | Todo gesto tem comando. |
  | MIG105 | Outras novidades do piloto (render por frames/contact-sheet, normais Marigold, splat de imagem única, fumaça/fogo) | futuro | Misto | Definido após D1. |
  | MIG106 | Plugins e efeitos isolados por fragmento (VGP-15) e Sprite Normals (VGP-16) | futuro | UI/adapter ou N/A | N/A. |

### S16 — Ferramentas externas e oráculos (fora do app)

- **Pré-requisitos:** —
- **Objetivo:** Não migram: manter na origem como gerador de fixtures/mapas (SHARP, MoGe-3, mapas, mão, QA). Servem de oráculo e roteiro de aceite.
- **Contratos e formatos:** Formatos de saída (`.spz`, mapas f32/RGB).
- **Evidência de paridade exigida:** Os artefatos que geram carregam no destino.
- **Superfícies (ordem):** —
- **Engine/JS reutilizável vs port Rust:** —
- **MIGs:**

  | ID | Feature | Prioridade | Tipo | Aceite (resumo) |
  |---|---|---|---|---|
  | MIG060 | Ferramentas de mapas (normais, relevo, superfície) | não migrar | Ferramenta/oráculo | Mapas existentes carregam e validam no destino. |
  | MIG080 | Ferramentas da mão que escreve (bundle, captura, relatório, preparação de assets) | não migrar | Ferramenta/oráculo | N/A. |
  | MIG097 | Lançador do Studio e provas de QA (transporte, A/V, specs de navegador) | não migrar | Ferramenta/oráculo | Cada cenário listado tem um equivalente na prova do destino. |
  | MIG098 | Pipeline imagem → cena de splat (SHARP, MoGe-3, SPZ, R2) | não migrar | Ferramenta/oráculo | N/A; o destino só consome `.spz`. |
  | MIG099 | Experimento de modelo de mundo (RunPod, HunyuanWorld) | não migrar | Ferramenta/oráculo | N/A. |
  | MIG107 | Produção de ativos por IA e pipeline de vídeo narrado (VGP-01…11) | não migrar | UI/adapter ou N/A | N/A. |

## 4. Classificação essencial / seguinte / futuro

| Prioridade | Critério | MIGs |
|---|---|---|
| essencial (24) | Necessário para o fluxo verificado de ponta a ponta (abrir → avaliar → desenhar → tocar → exportar) ou contrato de dados/compat de que tudo depende. | MIG001, MIG002, MIG003, MIG004, MIG005, MIG010, MIG012, MIG013, MIG016, MIG017, MIG018, MIG019, MIG022, MIG023, MIG025, MIG026, MIG030, MIG031, MIG032, MIG090, MIG091, MIG092, MIG093, MIG094 |
| seguinte (44) | Recursos entregues e documentados em `deliveries/*` (câmera, luz, clima, efeitos, narração, outline, propriedades) mas que não bloqueiam o fluxo mínimo. | MIG014, MIG015, MIG020, MIG027, MIG028, MIG029, MIG033, MIG034, MIG035, MIG036, MIG037, MIG038, MIG039, MIG040, MIG041, MIG042, MIG043, MIG044, MIG046, MIG047, MIG048, MIG049, MIG054, MIG055, MIG056, MIG057, MIG058, MIG059, MIG061, MIG062, MIG063, MIG064, MIG065, MIG067, MIG068, MIG081, MIG082, MIG083, MIG084, MIG085, MIG087, MIG088, MIG089, MIG095 |
| futuro (23) | Uso restrito (mão que escreve: rig/path/follow/reveal), multi-projeto, i18n, importação SRT, clima em splat, ou branch piloto não mergeado. | MIG006, MIG007, MIG051, MIG066, MIG069, MIG070, MIG071, MIG072, MIG073, MIG074, MIG075, MIG076, MIG077, MIG078, MIG079, MIG086, MIG100, MIG101, MIG102, MIG103, MIG104, MIG105, MIG106 |
| não migrar (16) | Casca de dev server, hibernação de runtime, Leva, store em zustand, ferramentas externas, experimentos e itens inativos. | MIG008, MIG009, MIG011, MIG021, MIG024, MIG045, MIG050, MIG052, MIG053, MIG060, MIG080, MIG096, MIG097, MIG098, MIG099, MIG107 |
| ausente (14) | Investigado e inexistente na origem; onde o destino já tem algo, está na coluna de correspondência. | MIG108, MIG109, MIG110, MIG111, MIG112, MIG113, MIG114, MIG115, MIG116, MIG117, MIG118, MIG119, MIG120, MIG121 |

## 5. O que cada superfície precisa provar (por fatia)

| Superfície | Prova aceita | Não aceita |
|---|---|---|
| Lib (core) | teste Rust sem GPUI contra golden gerado pela origem; `cargo tree -p tessel-editor-core` sem gpui/wry | teste que importa o bundle TS |
| CLI headless | `./scripts/headless` com JSONL, incluindo erro relevante e invariância de estado após rejeição | socket de janela aberta |
| UI | nó em `state/tree.rs` + comando semântico + teste; observação visual nativa permitida (`screencapture`) | cliques de coordenada; `ui op` como substituto de fluxo visual |
| Runtime JS | prova de pixel com hash no ambiente fixo (SwiftShader) e `live-proof` na janela real | comparação entre GPUs diferentes sem tolerância |

## 6. Riscos transversais

- **Campo desconhecido no round-trip**: o parser da origem (`z.object`) descarta chaves desconhecidas; o destino precisa decidir preservar ou recusar (MIG001, MIG024).
- **Runtime sem teste unitário**: `three-scene-runtime.ts`, `weather-layer.ts`, `plane-effects-material.ts`, `plane-shadows.ts`, `splat-*` ficam fora da cobertura da origem; a única garantia são as provas de navegador (não executadas aqui). O aceite precisa ser por pixel.
- **Duas pinturas de legenda** (HTML no Studio, sharp+Pango no export) divergem por construção (MIG087).
- **Contrato de cor**: a origem marca BT709/sRGB/`tv`; o destino não marca (MIG091).
- **Dependência de ambiente** (Chrome, ffmpeg, Python/GPU para mapas): isolar em adapters injetados, como o destino já faz com FFmpeg.
- **Origem muda**: 100 commits em 4 dias e um branch com +50 commits; fixar os hashes de golden por snapshot e registrar a versão do oráculo.
