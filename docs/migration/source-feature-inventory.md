# Inventário de features da origem (editor Three.js do Tessel)

> Gerado em 2026-11-05 a partir do snapshot `d6adac15c6` (baseline) e, só para o delta MIG100–MIG105, do snapshot `d40ac3f7c0` (branch piloto). Contexto, metodologia e fingerprints: [README](README.md). Matriz de destino: [feature-migration-matrix.md](feature-migration-matrix.md). Sequência: [migration-sequence.md](migration-sequence.md).

**Como ler.** Cada feature tem ID estável `MIGnnn`. O estado é o da **origem**, por leitura de código: *achar código não é feature testada em runtime*. Os blocos de evidência usam três rótulos:

- **Só código**: li o código; nenhum teste associado encontrado.
- **Teste já existente, não executado**: existe spec (contagem de `it/test` entre parênteses); **não rodei** (a origem não tem `node_modules`/`dist` e a tarefa proibia instalar/compilar).
- **Prova executada agora**: nenhuma execução funcional foi feita nesta auditoria (só `git`, `rg`, `shasum`, contagens). Portanto *nenhuma* linha abaixo afirma comportamento visto em runtime.

**Classes de estado**

- `UI` — Visível / UI funcional: 48
- `LIB` — Implementado em lib/API/CLI, sem UI: 47
- `PARC` — Parcial / placeholder: 2
- `EXP` — Experimental (branch piloto ou isolado): 7
- `INAT` — Inativo / sem consumidor: 1
- `PLAN` — Planejado apenas: 2
- `AUS` — Ausente (investigado, não existe): 14

Total: **121** features (baseline: 115; branch piloto: 6).

## Índice

| ID | Feature | Estado | Fonte | Domínio |
|---|---|---|---|---|
| [MIG001](#mig001) | Modelo do documento `render-bundle.json` (schema 1.0) | LIB | baseline | A |
| [MIG002](#mig002) | Regras de integridade do bundle e de junções de edição | LIB | baseline | A |
| [MIG003](#mig003) | Arquivo do bundle: abrir, escrita atômica serializada, dry-run e finalize | LIB | baseline | A |
| [MIG004](#mig004) | Preflight de assets (hash, dimensões, alpha, hash PCM, confinamento de caminho) | LIB | baseline | A |
| [MIG005](#mig005) | Matriz de formatos de asset e recusas explícitas | PARC | baseline | A |
| [MIG006](#mig006) | Registro de projetos do Studio (abrir, duplicar, renomear, fechar; trava de escrita) | UI | baseline | A |
| [MIG007](#mig007) | Abas de projeto e menu Project na UI | UI | baseline | A |
| [MIG008](#mig008) | Hibernação de runtime e stores por projeto (limite de 2 projetos ativos) | UI | baseline | A |
| [MIG009](#mig009) | Servidor de preview HTTP local (rotas do bundle, assets e salvamentos) | LIB | baseline | A |
| [MIG010](#mig010) | Scripts de fixture e importação de produções locais | LIB | baseline | A |
| [MIG011](#mig011) | Layout de painéis persistido no navegador | UI | baseline | A |
| [MIG012](#mig012) | Grafo de cena: nós, hierarquia, sockets, quaternions e ordem de desenho | LIB | baseline | B |
| [MIG013](#mig013) | Camadas de plano de imagem (PNG com alpha, fila de pintura única) | LIB | baseline | B |
| [MIG014](#mig014) | Camadas de splat Gaussiano (Spark 2.4, `.spz`) | LIB | baseline | B |
| [MIG015](#mig015) | Limites de câmera sobre splat (zonas safe/caution) | LIB | baseline | B |
| [MIG016](#mig016) | Avaliação do frame de edição (shots → instância/câmera → estado dos nós) | LIB | baseline | B |
| [MIG017](#mig017) | Performance tracks (ciclo, evento com hold, estático) | LIB | baseline | B |
| [MIG018](#mig018) | Estrutura de shots, instâncias de cena e transições (somente leitura) | PARC | baseline | B |
| [MIG019](#mig019) | Perfil de render (resolução, fps racional, cor, codec) | LIB | baseline | B |
| [MIG020](#mig020) | Auditoria de projeção e `frameAudit` por frame | LIB | baseline | B |
| [MIG021](#mig021) | Coordenador de preview (latest-wins, descarte de quadro velho, métricas) | UI | baseline | B |
| [MIG022](#mig022) | Determinismo de quadro e prova de pixel | LIB | baseline | B |
| [MIG023](#mig023) | API do runtime Three.js (setters de cena, captura, descarte) | LIB | baseline | B |
| [MIG024](#mig024) | Campos e tipos aceitos pelo schema sem consumidor no runtime/UI | INAT | baseline | B |
| [MIG025](#mig025) | Controlador de transporte (play, pause, seek, passo, salto, shots, scrub) | UI | baseline | C |
| [MIG026](#mig026) | Porta de áudio do navegador (relógio de áudio, ganho do monitor, janela de edição) | UI | baseline | C |
| [MIG027](#mig027) | Waveform da timeline com scrub e seek | UI | baseline | C |
| [MIG028](#mig028) | Atalhos de teclado e regras de foco | UI | baseline | C |
| [MIG029](#mig029) | Barra de transporte, campo de frame, timecode e controle de volume | UI | baseline | C |
| [MIG030](#mig030) | Tempo racional, FPS e conversões | LIB | baseline | C |
| [MIG031](#mig031) | Clipe de câmera: pose inicial, segmentos (8 receitas + keyframes) e até 3 camadas | LIB | baseline | D |
| [MIG032](#mig032) | Resolução da câmera por frame e easings | LIB | baseline | D |
| [MIG033](#mig033) | Comandos de câmera (15 ops) com erros tipados e dica | LIB | baseline | D |
| [MIG034](#mig034) | Propriedades de câmera por caminho (`camera/<clip>/…`) | LIB | baseline | D |
| [MIG035](#mig035) | Análise de clipe de câmera | LIB | baseline | D |
| [MIG036](#mig036) | Faixa de câmera na timeline (barras, arrasto, redimensionar, inserir, camadas) | UI | baseline | D |
| [MIG037](#mig037) | Overlay do caminho 3D (handles arrastáveis) e mini-mapa CameraPlan | UI | baseline | D |
| [MIG038](#mig038) | Inspector de câmera | UI | baseline | D |
| [MIG039](#mig039) | Modelo de seleção (12 tipos) e inspector contextual | UI | baseline | E |
| [MIG040](#mig040) | Painel de camadas (Cena, Câmera, Clima, Efeitos, árvore de nós, luzes, áudio/texto) | UI | baseline | E |
| [MIG041](#mig041) | Árvore de camadas: arrastar e soltar (mouse e teclado), renomear com F2, ações por linha | UI | baseline | E |
| [MIG042](#mig042) | Outline: grupos de camadas, luzes e áudio com ocultar/silenciar/recolher | LIB | baseline | E |
| [MIG043](#mig043) | Comandos de outline (6 ops) | LIB | baseline | E |
| [MIG044](#mig044) | Inspector de cena, nó e grupo | UI | baseline | E |
| [MIG045](#mig045) | Infra do inspector (schema declarativo sobre Leva) | UI | baseline | E |
| [MIG046](#mig046) | Sistema de property paths (listar, ler, escrever, erros tipados) | LIB | baseline | E |
| [MIG047](#mig047) | Keyframes de valores (resolver, regras, adicionar/remover no playhead) | LIB | baseline | E |
| [MIG048](#mig048) | Store do editor (mostrar, comprometer, salvar, reverter) | UI | baseline | E |
| [MIG049](#mig049) | Desfazer e refazer (200 passos, por projeto) | UI | baseline | E |
| [MIG050](#mig050) | Pipeline de salvamento por domínio e edição em lote | UI | baseline | E |
| [MIG051](#mig051) | Internacionalização pt-br/en/es (383 chaves) | UI | baseline | E |
| [MIG052](#mig052) | Leitura de debug do transporte | UI | baseline | E |
| [MIG053](#mig053) | API global de teste `window.__sceneStudio` | LIB | baseline | E |
| [MIG054](#mig054) | Modelo de iluminação da cena (ambiente + até 4 luzes pontuais) | UI | baseline | F |
| [MIG055](#mig055) | Keys de luz (intensidade, cor, posição) e luz que segue uma camada | UI | baseline | F |
| [MIG056](#mig056) | Planos iluminados: mapas de normais e relevo | LIB | baseline | F |
| [MIG057](#mig057) | Sombras de personagens: contato no chão e projetada (até 4 casters) | UI | baseline | F |
| [MIG058](#mig058) | Luz dinâmica em splats (Spark world modifier) | LIB | baseline | F |
| [MIG059](#mig059) | UI de luzes: seção, marcador arrastável no viewport e inspector | UI | baseline | F |
| [MIG060](#mig060) | Ferramentas de mapas (normais, relevo, superfície) | LIB | baseline | F |
| [MIG061](#mig061) | Gramática do clima (palavras, intensidade, erro de digitação) | UI | baseline | G |
| [MIG062](#mig062) | Renderização do clima em planos (chuva, neve, relâmpago, grade de cor) | UI | baseline | G |
| [MIG063](#mig063) | Superfícies do clima: neve que acumula, poças com reflexo e respingos | UI | baseline | G |
| [MIG064](#mig064) | Linha do chão do clima (para plano sem mapa) | UI | baseline | G |
| [MIG065](#mig065) | Keys de intensidade do clima | UI | baseline | G |
| [MIG066](#mig066) | Clima em cenas de splat (colisão por profundidade, respingos, neve, poças) | LIB | baseline | G |
| [MIG067](#mig067) | Looks de pós-produção (tinta, sonho, filme) com catálogo | UI | baseline | H |
| [MIG068](#mig068) | Inspector de efeitos (sliders por look) | UI | baseline | H |
| [MIG069](#mig069) | Rig: ossos sobre uma camada, limites, punho e pino | UI | baseline | I |
| [MIG070](#mig070) | IK (CCDIKSolver) e bake determinístico da pose | LIB | baseline | I |
| [MIG071](#mig071) | Binding de camadas ao rig (rígido ou esfolado) e RigLayer | LIB | baseline | I |
| [MIG072](#mig072) | Comandos e propriedades de rig (11 ops + caminhos `rig/<id>/…`) | LIB | baseline | I |
| [MIG073](#mig073) | Validação e diagnósticos do rig | UI | baseline | I |
| [MIG074](#mig074) | UI de rig (menu, árvore, overlay de ossos, faixa de alvos, inspector, vista pose/repouso) | UI | baseline | I |
| [MIG075](#mig075) | Paths sobre a imagem de uma camada (catmull-rom, linear, bezier, caneta levantada) | UI | baseline | I |
| [MIG076](#mig076) | Gerar o primeiro path de um texto a partir dos pixels da camada | UI | baseline | I |
| [MIG077](#mig077) | Follow: a ponta de um osso caminha pelo path (chaves de progresso, alcance) | UI | baseline | I |
| [MIG078](#mig078) | Reveal: a tinta aparece quando a caneta já passou | UI | baseline | I |
| [MIG079](#mig079) | UI de follow e path (menu, árvore, overlay de pontos, faixa de progresso) | UI | baseline | I |
| [MIG080](#mig080) | Ferramentas da mão que escreve (bundle, captura, relatório, preparação de assets) | LIB | baseline | I |
| [MIG081](#mig081) | Modelo de narração (faixas, linhas, palavras, entrega, fromMix, legenda) | UI | baseline | J |
| [MIG082](#mig082) | Comandos de narração (10 ops) | LIB | baseline | J |
| [MIG083](#mig083) | Plano de mix: o que soa, onde a voz do mix cala porque há take | LIB | baseline | J |
| [MIG084](#mig084) | Gerar takes de TTS (cliente OpenAI-compatível) e gravar no bundle | UI | baseline | J |
| [MIG085](#mig085) | Cortar o take da própria voz do mix | UI | baseline | J |
| [MIG086](#mig086) | Importar legendas SRT como linhas de narração | LIB | baseline | J |
| [MIG087](#mig087) | Legendas: revelação por palavra, layout, overlay no Studio e pintura no export | UI | baseline | J |
| [MIG088](#mig088) | Reprodução dos takes de narração no Studio | UI | baseline | J |
| [MIG089](#mig089) | UI de narração (faixas, linhas, inspector de linha com voz/estilo/entrega) | UI | baseline | J |
| [MIG090](#mig090) | Render offline: Chrome + Three.js quadro a quadro → ffmpeg H.264 | LIB | baseline | K |
| [MIG091](#mig091) | Verificação do arquivo final (ffprobe) e contrato de cor | LIB | baseline | K |
| [MIG092](#mig092) | Grafo de áudio do export (mix + takes, silêncios, AAC) | LIB | baseline | K |
| [MIG093](#mig093) | CLI `scene` (14 comandos, JSON em stdout, exit 0/1/2) | LIB | baseline | K |
| [MIG094](#mig094) | API de edição em lote `editBundle` (6 formas de edit) | LIB | baseline | K |
| [MIG095](#mig095) | Consultas `show` e `summarize` do bundle (8 camadas) | LIB | baseline | K |
| [MIG096](#mig096) | Exports programáticos do pacote (core, node, runtime, transport, testing) | LIB | baseline | K |
| [MIG097](#mig097) | Lançador do Studio e provas de QA (transporte, A/V, specs de navegador) | LIB | baseline | K |
| [MIG098](#mig098) | Pipeline imagem → cena de splat (SHARP, MoGe-3, SPZ, R2) | LIB | baseline | K |
| [MIG099](#mig099) | Experimento de modelo de mundo (RunPod, HunyuanWorld) | EXP | baseline | K |
| [MIG100](#mig100) | Camada de efeitos por clipe (faixas, clipes, parâmetros, envelope, escopo, chaves) | EXP | piloto | L |
| [MIG101](#mig101) | Catálogo de 59 efeitos (31 looks + 28 transições) com baseline de pixel | EXP | piloto | L |
| [MIG102](#mig102) | Edição de shots: aparar bordas, mover corte, duração do shot e da edição (ripple/roll) | EXP | piloto | L |
| [MIG103](#mig103) | Gestos de timeline como funções puras (arrastar shot, clipe de fx, transição, chaves) | EXP | piloto | L |
| [MIG104](#mig104) | Timeline do Studio com faixas de shots, efeitos e transições, e inspector de item | EXP | piloto | L |
| [MIG105](#mig105) | Outras novidades do piloto (render por frames/contact-sheet, normais Marigold, splat de imagem única, fumaça/fogo) | EXP | piloto | L |
| [MIG106](#mig106) | Plugins e efeitos isolados por fragmento (VGP-15) e Sprite Normals (VGP-16) | PLAN | baseline | M |
| [MIG107](#mig107) | Produção de ativos por IA e pipeline de vídeo narrado (VGP-01…11) | PLAN | baseline | M |
| [MIG108](#mig108) | Biblioteca de mídia, busca, miniaturas e import pela UI | AUS | baseline | N |
| [MIG109](#mig109) | Clipes de vídeo/áudio como camadas | AUS | baseline | N |
| [MIG110](#mig110) | Camadas de texto e fontes | AUS | baseline | N |
| [MIG111](#mig111) | Cortar/aparar/dividir/reordenar clipes, snapping, marcadores (no baseline) | AUS | baseline | N |
| [MIG112](#mig112) | Transformação de camadas e gizmos (posição/escala/rotação) | AUS | baseline | N |
| [MIG113](#mig113) | Blend modes, máscaras e composição por passe (como recurso do usuário) | AUS | baseline | N |
| [MIG114](#mig114) | Autosave, recuperação e 'salvar como' | AUS | baseline | N |
| [MIG115](#mig115) | Presets e templates (de projeto, de rig, de efeito) | AUS | baseline | N |
| [MIG116](#mig116) | Menus de contexto, multi-seleção e copiar/colar | AUS | baseline | N |
| [MIG117](#mig117) | Presets de exportação, jobs, cancelamento e progresso no editor | AUS | baseline | N |
| [MIG118](#mig118) | Integração com nuvem/IA dentro do editor (login, upload, geração) | AUS | baseline | N |
| [MIG119](#mig119) | Acessibilidade completa | AUS | baseline | N |
| [MIG120](#mig120) | Edição de splat (selecionar, apagar, cortar ou transformar splats) | AUS | baseline | N |
| [MIG121](#mig121) | Qualidade de preview, LOD, cache de assets e streaming | AUS | baseline | N |

## A. Documento, projeto e arquivos

### MIG001

**Modelo do documento `render-bundle.json` (schema 1.0)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (24 casos em 2 spec vitest)
- **O usuário faz:** Autor ou ferramenta descreve uma produção inteira num único JSON: assets, definições de cena, instâncias, performance, câmeras, shots, áudio, narração e outline.
- **Gatilho (UI/atalho/CLI):** Todo comando abre o arquivo (`scene show|validate|set…`); o Studio carrega `/p/<id>/bundle.json`.
- **Caminho handler → estado → regra/runtime:** `openBundle` → `renderBundleSchema.safeParse` (zod) → `superRefine` (narração, outline, integridade, rig, ground, performance, dissolve) → `RenderBundle` em memória; a UI repete `renderBundleSchema.parse(await res.json())`.
- **Propriedades, unidades, modelo de dados:** fps racional {numerator,denominator}; `editDurationFrames`; `renderProfile` {width,height,fps,colorSpace,alphaMode,videoCodec=h264-yuv420p,threeRevision,browserChannel}; `assets[]` {assetId,revisionId,contentHash,relativePath,format png|spz|f32,widthPx,heightPx,alphaMode,colorSpace}; `sceneDefinitions[]` (nodes group|imagePlane|supportPlane|overlay|splat, sockets, frameAudit, weather, weatherKeys, lighting, effects, weatherGround, rigs, paths); `sceneInstances`; `performanceTracks`; `cameraClips`; `shots`; `compositionPasses`; `audio`; `narration`; `outline`; `metadata`. Unidades: frames de edição, segundos de edição, unidades de cena, px da imagem da camada.
- **Limitações:** `schemaVersion` é o literal "1.0" (sem migração entre versões). `metadata` é livre e sem consumidor. Campos aceitos pelo schema nem sempre têm runtime (ver MIG024).
- **API / CLI:** Lib: `renderBundleSchema`, `RenderBundle` (`@tessel/scene-engine/core`). CLI: `scene show|validate`.
- **Testes (existentes, não executados):** [packages/scene-engine/src/schema/render-bundle.schema.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.spec.ts) (18 casos); [packages/scene-engine/src/node/bundle-file.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/bundle-file.spec.ts) (6 casos)
- **Código:** [packages/scene-engine/src/schema/render-bundle.schema.ts:481](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L481) · [packages/scene-engine/src/schema/render-bundle.schema.ts:179](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L179) · [packages/scene-engine/src/schema/render-bundle.schema.ts:231](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L231) · [packages/scene-engine/src/schema/render-bundle.schema.ts:427](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L427)

### MIG002

**Regras de integridade do bundle e de junções de edição**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (22 casos em 2 spec vitest)
- **O usuário faz:** Recusar um bundle incoerente com um código estável e mensagem (ex.: `EDIT_GAP_OR_OVERLAP`, `CAMERA_KEYS_END_SHORT`, `AUDIO_HASH_REQUIRED`).
- **Gatilho (UI/atalho/CLI):** `scene validate`; também roda em todo parse do bundle e antes de cada escrita.
- **Caminho handler → estado → regra/runtime:** `superRefine` do schema chama `validateBundleIntegrity` (+ `rigBundleIssues`, `explainNarration`, `explainOutline`); `validateEditJoins` valida junções contínuas/cortes entre shots.
- **Propriedades, unidades, modelo de dados:** ~70 códigos em 4 grupos: referências de asset (hash/tamanho/formato), mapas (surface/normal/relief/sceneDepth), câmera (FOV, direção degenerada, easings, keys, layers), grafo (quaternions, sockets, ids duplicados, planos de sombra), shots (faixa de edição contígua, duração = soma), passes de composição, janela de áudio e hash PCM obrigatório.
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** Dissolve é recusado no schema (`Dissolve transitions are not supported`). `premultiplied`, `rec709-video`, `premultiplyOnUpload` e asset `linear-srgb` de cor são recusados explicitamente.
- **API / CLI:** Lib: `validateBundleIntegrity`, `validateEditJoins`. CLI: `scene validate <bundle>` (exit 1 com a lista).
- **Testes (existentes, não executados):** [packages/scene-engine/src/validate/edit-joins.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/validate/edit-joins.spec.ts) (4 casos); [packages/scene-engine/src/schema/render-bundle.schema.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.spec.ts) (18 casos)
- **Código:** [packages/scene-engine/src/validate/bundle-integrity.ts:466](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/validate/bundle-integrity.ts#L466) · [packages/scene-engine/src/validate/edit-joins.ts:116](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/validate/edit-joins.ts#L116) · [packages/scene-engine/src/node/validate.ts:13](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/validate.ts#L13)

### MIG003

**Arquivo do bundle: abrir, escrita atômica serializada, dry-run e finalize**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (8 casos em 2 spec vitest)
- **O usuário faz:** Aplicar edições no arquivo com segurança: sem `--write` só calcula; com `--write` grava inteiro por arquivo irmão (rename).
- **Gatilho (UI/atalho/CLI):** CLI `--write`; PUT do Studio; `changeBundle`.
- **Caminho handler → estado → regra/runtime:** `openBundle` → fila por arquivo no processo (`serialized`) → `update(change)` lê, aplica `change`, `finalizeBundle`, revalida, `writeAtomically`, chama `onSaved`.
- **Propriedades, unidades, modelo de dados:** `BundleFile` {path, read, update}; erros `SceneError` com `details[]` {code,message,hint?}; `UpdateAccepted|UpdateRefused`.
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** A fila é por processo: dois processos escrevendo o mesmo arquivo não são coordenados (o Studio usa trava por projeto, ver MIG006). Sem backup/versão anterior.
- **API / CLI:** Lib: `openBundle`, `changeBundle`, `SceneError` (`@tessel/scene-engine/node`).
- **Testes (existentes, não executados):** [packages/scene-engine/src/node/bundle-file.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/bundle-file.spec.ts) (6 casos); [packages/scene-engine/src/edit/finalize-bundle.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/edit/finalize-bundle.spec.ts) (2 casos)
- **Código:** [packages/scene-engine/src/node/bundle-file.ts:129](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/bundle-file.ts#L129) · [packages/scene-engine/src/node/bundle-file.ts:195](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/bundle-file.ts#L195) · [packages/scene-engine/src/node/bundle-file.ts:115](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/bundle-file.ts#L115) · [packages/scene-engine/src/edit/finalize-bundle.ts:18](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/edit/finalize-bundle.ts#L18)

### MIG004

**Preflight de assets (hash, dimensões, alpha, hash PCM, confinamento de caminho)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (2 casos em 1 spec vitest)
- **O usuário faz:** Antes de renderizar ou abrir, provar que cada arquivo declarado existe e bate com o que o bundle diz.
- **Gatilho (UI/atalho/CLI):** Automático em `createViewerHarness` (render/preview) e `scene validate`.
- **Caminho handler → estado → regra/runtime:** `runAssetPreflight(bundle, dir)` → lê bytes, `sha256`, decodifica PNG com sharp (largura, altura, alpha), compara com o asset; splat/depth só por hash; áudio por hash do arquivo; recusa caminhos fora da pasta.
- **Propriedades, unidades, modelo de dados:** `PreflightReport` com resultado por asset (`observedHash`, dimensões, alpha).
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** `sourcePcmHashSha256` é hash do arquivo WAV inteiro (inclui cabeçalho), não do PCM. Decodificação via sharp (nativo Node).
- **API / CLI:** Lib: `runAssetPreflight`.
- **Testes (existentes, não executados):** [packages/scene-engine/src/preflight/asset-preflight.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/preflight/asset-preflight.spec.ts) (2 casos)
- **Código:** [packages/scene-engine/src/preflight/asset-preflight.ts:78](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/preflight/asset-preflight.ts#L78) · [packages/scene-engine/src/node/viewer.ts:105](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/viewer.ts#L105)

### MIG005

**Matriz de formatos de asset e recusas explícitas**

- **Estado:** `PARC` — Parcial / placeholder · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (20 casos em 2 spec vitest)
- **O usuário faz:** Saber o que o motor realmente desenha: PNG sRGB (alpha reto/opaco), splat `.spz`, grade `f32` de profundidade.
- **Gatilho (UI/atalho/CLI):** Validação do bundle e carga em `createTextureLoader`.
- **Caminho handler → estado → regra/runtime:** Schema aceita `png|spz|f32`, `alphaMode` straight|premultiplied|opaque, `colorSpace` srgb|linear-srgb; a integridade recusa premultiplied e `linear-srgb` de cor; mapas de dados (surface, normal, relief) são lidos como dados lineares.
- **Propriedades, unidades, modelo de dados:** Asset {format?, alphaMode, colorSpace}; mapas: `surface` (RGB linear = up/floor/linha), `normalMap`, `relief` (f32 + intrínsecos), `sceneDepth` (f32 + imagem).
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** O schema aceita mais do que o runtime (premultiplied, rec709-video): são recusas explícitas, não suporte. Sem vídeo, sem JPEG/WebP no bundle (apenas PNG nas camadas).
- **Testes (existentes, não executados):** [packages/scene-engine/src/preflight/asset-preflight.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/preflight/asset-preflight.spec.ts) (2 casos); [packages/scene-engine/src/schema/render-bundle.schema.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.spec.ts) (18 casos)
- **Código:** [packages/scene-engine/src/schema/render-bundle.schema.ts:53](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L53) · [packages/scene-engine/src/validate/bundle-integrity.ts:534](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/validate/bundle-integrity.ts#L534) · [packages/scene-engine/src/runtime/three-scene-runtime.ts:403](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/three-scene-runtime.ts#L403)

### MIG006

**Registro de projetos do Studio (abrir, duplicar, renomear, fechar; trava de escrita)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (10 casos em 2 spec vitest + 1 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Abrir vários bundles como projetos nomeados, duplicar um para editar uma cópia, renomear e fechar sem apagar arquivos.
- **Gatilho (UI/atalho/CLI):** Menu **Project** (abas do Studio); `POST/PATCH/DELETE /projects`.
- **Caminho handler → estado → regra/runtime:** UI `useProjects` → `projectsApi` (fetch) → `handleProjectsApi` → `createProjectRegistry` (`projects.json` em `~/www/.data/scene-engine/studio`, uma pasta por projeto, id estável `p<sha8>` do caminho). Primeiro projeto aberto no arquivo é dono da escrita; o segundo fica **somente leitura** (`BUNDLE_LOCKED`, 409).
- **Propriedades, unidades, modelo de dados:** Projeto {id,name,bundlePath,assetsDir?,workDir,createdAt,config{renderProfile,fps,defaults{weather,lighting},rigPresets[]}}; REGISTRY_VERSION 1.
- **Dependências:** MIG003 (Arquivo do bundle: abrir, escrita atômica serializada, dry-run e finalize)
- **Limitações:** Não se pode fechar o último projeto. `config.defaults` e `config.rigPresets` não têm consumidor encontrado na UI (ver MIG024). Servidor local, sem rede.
- **API / CLI:** HTTP local: `GET /projects`, `POST /projects` {path|duplicateOf}, `PATCH /projects/<id>` {name}, `DELETE /projects/<id>`.
- **Testes (existentes, não executados):** [apps/scene-studio/src/engine/projectsApi.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/projectsApi.spec.ts) (5 casos); [apps/scene-studio/src/engine/projectStores.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/projectStores.spec.ts) (5 casos); [packages/scene-engine/scripts/scene-studio-projects.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-projects.spec.mjs) (script de navegador/Chrome)
- **Código:** [packages/scene-engine/scripts/studio-projects.mjs:46](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/studio-projects.mjs#L46) · [packages/scene-engine/scripts/studio-project-api.mjs:245](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/studio-project-api.mjs#L245) · [apps/scene-studio/src/engine/projectsRegistry.ts:151](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/projectsRegistry.ts#L151) · [apps/scene-studio/src/engine/projectsApi.ts:83](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/projectsApi.ts#L83)

### MIG007

**Abas de projeto e menu Project na UI**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (6 casos em 1 spec vitest)
- **O usuário faz:** Alternar entre projetos abertos, abrir por caminho, duplicar, renomear e fechar; ver selo de somente leitura/hibernado.
- **Gatilho (UI/atalho/CLI):** Barra de abas no topo; menu **Project** com Open/Duplicate/Rename/Close; prompt de texto para caminho/nome.
- **Caminho handler → estado → regra/runtime:** `ProjectTabs` → ações de `useProjects` (open/duplicate/rename/close) → API HTTP → registro; `App` remonta `ProjectWorkspace` por `key={entry.info.id}`.
- **Propriedades, unidades, modelo de dados:** Estado por projeto: entry {info, readiness, canvas, stores, sleeping, frame}.
- **Dependências:** MIG006 (Registro de projetos do Studio (abrir, duplicar, renomear, fechar; trava de escrita))
- **Limitações:** Entrada de caminho é texto livre (sem seletor de arquivo). Sem reordenar abas.
- **Testes (existentes, não executados):** [apps/scene-studio/src/engine/hotProjects.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/hotProjects.spec.ts) (6 casos)
- **Código:** [apps/scene-studio/src/components/ProjectTabs.tsx:32](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/ProjectTabs.tsx#L32) · [apps/scene-studio/src/App.tsx:457](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/App.tsx#L457)

### MIG008

**Hibernação de runtime e stores por projeto (limite de 2 projetos ativos)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (6 casos em 1 spec vitest)
- **O usuário faz:** Manter vários projetos abertos sem estourar contextos WebGL: os mais antigos hibernam e voltam no mesmo quadro/seleção.
- **Gatilho (UI/atalho/CLI):** Automático ao trocar de aba; `window.__sceneStudio.setHotLimit`.
- **Caminho handler → estado → regra/runtime:** `projectsToHibernate(projects, activeId, limit)` escolhe quem dorme; `hibernate(id)` descarta transport+runtime, preservando store/histórico; `startSession(id)` recria com canvas novo e re-seek.
- **Propriedades, unidades, modelo de dados:** `DEFAULT_HOT_LIMIT = 2`; o ativo nunca dorme (mínimo 1).
- **Dependências:** MIG006 (Registro de projetos do Studio (abrir, duplicar, renomear, fechar; trava de escrita))
- **Limitações:** Só um projeto toca áudio por vez; estado de edição vive no store, não no runtime.
- **Testes (existentes, não executados):** [apps/scene-studio/src/engine/hotProjects.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/hotProjects.spec.ts) (6 casos)
- **Código:** [apps/scene-studio/src/engine/hotProjects.ts:17](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/hotProjects.ts#L17) · [apps/scene-studio/src/engine/projectsRegistry.ts:297](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/projectsRegistry.ts#L297)

### MIG009

**Servidor de preview HTTP local (rotas do bundle, assets e salvamentos)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (0 casos em 0 spec vitest + 2 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Servir o Studio e o bundle de cada projeto e receber os salvamentos do editor, só para a máquina local.
- **Gatilho (UI/atalho/CLI):** `scene preview <bundle> [--port]` ou `pnpm studio:preview` (porta padrão 8767).
- **Caminho handler → estado → regra/runtime:** `createStudioHarness` → rotas `/p/<id>/bundle.json`, `/bundle-assets/<rel>` (whitelist = assets + takes + mix; bloqueia `..` e symlink), PUT `/bundle-weather|-weather-keys|-effects|-ground|-lighting|-rig|-path|-edit|-camera|-camera-commands|-narration-take|-narration-from-mix`, GET `/narration-voices`, `/projects`. PUT só com `isLocalEditorRequest` (403) e método PUT (405); trava devolve 409.
- **Propriedades, unidades, modelo de dados:** Respostas JSON {error,code,errors[]} com a lista de erros do motor.
- **Dependências:** MIG003 (Arquivo do bundle: abrir, escrita atômica serializada, dry-run e finalize)
- **Limitações:** Dev server Node (Playwright/Chrome para provas). Os mesmos saves existem como `editBundle` na lib, o servidor é só casca.
- **API / CLI:** HTTP local: 12 rotas PUT (7 saves de cena + `-edit`, `-camera`, `-camera-commands`, `-narration-take`, `-narration-from-mix`) e GET `/p/<id>/bundle.json`, `/p/<id>/bundle-assets/<rel>`, `/narration-voices`, `/projects`.
- **Testes (existentes, não executados):** [packages/scene-engine/scripts/scene-studio-projects.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-projects.spec.mjs) (script de navegador/Chrome); [packages/scene-engine/scripts/scene-studio-history.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-history.spec.mjs) (script de navegador/Chrome)
- **Código:** [packages/scene-engine/scripts/studio-harness.mjs:54](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/studio-harness.mjs#L54) · [packages/scene-engine/scripts/studio-harness.mjs:44](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/studio-harness.mjs#L44) · [packages/scene-engine/scripts/studio-project-api.mjs:311](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/studio-project-api.mjs#L311) · [packages/scene-engine/scripts/studio-http.mjs:62](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/studio-http.mjs#L62)

### MIG010

**Scripts de fixture e importação de produções locais**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (3 casos em 1 spec vitest)
- **O usuário faz:** Gerar o bundle técnico procedural; importar arte/voz privadas locais; importar splat SHARP; montar o bundle da mão que escreve.
- **Gatilho (UI/atalho/CLI):** `pnpm fixture:generate`, `pilot:import-local`, `splat/import-sharp-scene.mjs`, `elements/rig/build-hands-bundle.mjs`.
- **Caminho handler → estado → regra/runtime:** Scripts Node escrevem `render-bundle.json` + assets em `output/…` (ignorado pelo git).
- **Propriedades, unidades, modelo de dados:** Fixture técnico: palma/prop/dedos coplanares, um gesto com hold, spans contínuos, inserção e retorno.
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** Entradas privadas (`layers/*.png`, `audio/voice.wav`) não vão ao repositório; scripts dependem de caminhos locais.
- **API / CLI:** Scripts Node (sem CLI unificada).
- **Testes (existentes, não executados):** [packages/scene-engine/src/node/narrated-scene.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/narrated-scene.spec.ts) (3 casos)
- **Código:** [packages/scene-engine/scripts/generate-technical-fixture.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/generate-technical-fixture.mjs) · [packages/scene-engine/scripts/import-local-production.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/import-local-production.mjs) · [packages/scene-engine/scripts/splat/import-sharp-scene.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/splat/import-sharp-scene.mjs) · [packages/scene-engine/scripts/elements/rig/build-hands-bundle.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/elements/rig/build-hands-bundle.mjs)

### MIG011

**Layout de painéis persistido no navegador**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Arrastar as divisórias entre camadas/viewport/inspetor e entre editor/timeline e o tamanho é lembrado.
- **Gatilho (UI/atalho/CLI):** Alças dos painéis redimensionáveis.
- **Caminho handler → estado → regra/runtime:** `useSavedLayout(id)` lê/grava `localStorage`; ids `studio-columns` e `studio-rows`.
- **Propriedades, unidades, modelo de dados:** Proporções por painel.
- **Limitações:** Por navegador, sem sincronizar. Sem layouts nomeados.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [apps/scene-studio/src/hooks/useSavedLayout.ts:35](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/hooks/useSavedLayout.ts#L35)

## B. Cena, viewport e runtime Three.js

### MIG012

**Grafo de cena: nós, hierarquia, sockets, quaternions e ordem de desenho**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (3 casos em 2 spec vitest)
- **O usuário faz:** Descrever camadas em 3D com pai/filho, anexar um nó a um socket de outro e fixar a ordem de pintura.
- **Gatilho (UI/atalho/CLI):** Bundle (`sceneDefinitions[].nodes/sockets`); sem edição de transformação na UI.
- **Caminho handler → estado → regra/runtime:** `scene-graph.ts` valida/resolve a árvore e converte quaternion→Euler; o runtime monta `Object3D`s e aplica `renderOrder`.
- **Propriedades, unidades, modelo de dados:** Nó {id,parentId,type,assetRef?,localPosition,localRotation(quat),localScale,widthSceneUnits,heightSceneUnits,renderOrder,socketId,attachToSocket}; unidades de cena; quaternions normalizados.
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** Convenção assumida: planos de frente para +z, câmera olhando -z (weather). Sem interpenetração/oclusão automática: todos os planos usam uma fila de pintura sem depth write.
- **Testes (existentes, não executados):** [packages/scene-engine/src/scene/scene-graph.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/scene/scene-graph.spec.ts) (2 casos); [packages/scene-engine/src/scene/quaternion-roundtrip.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/scene/quaternion-roundtrip.spec.ts) (1 casos)
- **Código:** [packages/scene-engine/src/scene/scene-graph.ts:5](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/scene/scene-graph.ts#L5) · [packages/scene-engine/src/scene/scene-graph.ts:58](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/scene/scene-graph.ts#L58) · [packages/scene-engine/src/validate/bundle-integrity.ts:649](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/validate/bundle-integrity.ts#L649)

### MIG013

**Camadas de plano de imagem (PNG com alpha, fila de pintura única)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (9 casos em 2 spec vitest)
- **O usuário faz:** Ver cada camada PNG como um plano texturizado em 2.5D, com alpha reto e ordem de pintura explícita.
- **Gatilho (UI/atalho/CLI):** Viewport do Studio e render offline.
- **Caminho handler → estado → regra/runtime:** `ensureSceneBuilt` carrega textura (`createTextureLoader`), aplica espaço de cor, cria `PlaneGeometry` + `MeshBasicMaterial` (`transparent`, `DoubleSide`, `depthWrite:false`, `premultipliedAlpha:false`), registra perfil de pés e mapas opcionais.
- **Propriedades, unidades, modelo de dados:** Plano {widthSceneUnits,heightSceneUnits,renderOrder}; textura sRGB; mapas opcionais (surface, normal, relief); perfil de pés para sombra.
- **Dependências:** MIG012 (Grafo de cena: nós, hierarquia, sockets, quaternions e ordem de desenho), MIG005 (Matriz de formatos de asset e recusas explícitas)
- **Limitações:** `imagePlane` exige asset e dimensões. `supportPlane` e `overlay` são aceitos pelo schema mas não são construídos (ver inativos).
- **Testes (existentes, não executados):** [packages/scene-engine/src/runtime/projection-audit.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/projection-audit.spec.ts) (7 casos); [packages/scene-engine/src/preflight/asset-preflight.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/preflight/asset-preflight.spec.ts) (2 casos)
- **Código:** [packages/scene-engine/src/runtime/three-scene-runtime.ts:351](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/three-scene-runtime.ts#L351) · [packages/scene-engine/src/runtime/three-scene-runtime.ts:458](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/three-scene-runtime.ts#L458) · [packages/scene-engine/src/runtime/three-scene-runtime.ts:1543](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/three-scene-runtime.ts#L1543)

### MIG014

**Camadas de splat Gaussiano (Spark 2.4, `.spz`)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Usar uma nuvem de splats (cena SHARP) como fundo, com personagens em planos à frente.
- **Gatilho (UI/atalho/CLI):** Nó `type: "splat"` no bundle.
- **Caminho handler → estado → regra/runtime:** `buildSplatNode` carrega o `.spz` via Spark; `SplatStage.render` espera o ordenamento terminar para a câmera exata (`minSortIntervalMs: 0`) antes de devolver o quadro.
- **Propriedades, unidades, modelo de dados:** Nó splat com `localRotation` {1,0,0,0} (meia volta em X: quadro OpenCV→cena), `sceneDepth` opcional; asset format `spz`.
- **Dependências:** MIG013 (Camadas de plano de imagem (PNG com alpha, fila de pintura única))
- **Limitações:** Dentro dos splats só planos opacos/alpha-test compõem corretamente; personagem vai à frente com render order posterior (README).
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [packages/scene-engine/src/runtime/splat-stage.ts:24](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/splat-stage.ts#L24) · [packages/scene-engine/src/runtime/three-scene-runtime.ts:736](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/three-scene-runtime.ts#L736)

### MIG015

**Limites de câmera sobre splat (zonas safe/caution)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (4 casos em 1 spec vitest)
- **O usuário faz:** Avisar quando a câmera sai demais da vista original do splat e a imagem quebra.
- **Gatilho (UI/atalho/CLI):** Aviso no inspector de câmera quando a cena é splat; análise de câmera.
- **Caminho handler → estado → regra/runtime:** `splat-camera-limits` mede o deslocamento (frações da profundidade pivô, órbitas em graus) contra limites medidos no laboratório (~5% seguro, ~12% cuidado).
- **Propriedades, unidades, modelo de dados:** Rig {forward,right,up,orbitYaw,orbitPitch}; limites seguros/cautela por eixo.
- **Dependências:** MIG031 (Clipe de câmera: pose inicial, segmentos (8 receitas + keyframes) e até 3 camadas), MIG014 (Camadas de splat Gaussiano (Spark 2.4, `.spz`))
- **Limitações:** Números vêm de um laboratório de reconstrução apagado do disco (roadmap VGP-17 registra o worktree perdido); dependem do modelo SHARP.
- **Testes (existentes, não executados):** [packages/scene-engine/src/camera/splat-camera-limits.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/camera/splat-camera-limits.spec.ts) (4 casos)
- **Código:** [packages/scene-engine/src/camera/splat-camera-limits.ts:11](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/camera/splat-camera-limits.ts#L11)

### MIG016

**Avaliação do frame de edição (shots → instância/câmera → estado dos nós)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (6 casos em 1 spec vitest)
- **O usuário faz:** Dado um frame de edição, saber qual shot, qual quadro da cena, qual quadro da câmera e onde cada nó está.
- **Gatilho (UI/atalho/CLI):** Chamado a cada quadro do preview e do render.
- **Caminho handler → estado → regra/runtime:** `evaluateEditFrame` → `resolveShotAtEditFrame` (faixa → instanceFrame/cameraClipFrame) → `resolveCameraAtClipFrame` + `resolvePerformanceOverrides` → `ResolvedFrameState`.
- **Propriedades, unidades, modelo de dados:** Faixas `[startFrame,endFrameExclusive)`; `instanceFrame = instanceStartFrame + offset`; `cameraClipFrame = cameraInFrame + offset`.
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0)), MIG002 (Regras de integridade do bundle e de junções de edição)
- **Limitações:** Lança erro para frame fora de `[0,editDurationFrames)` e para frame sem shot.
- **Testes (existentes, não executados):** [packages/scene-engine/src/evaluator/evaluate-edit-frame.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/evaluator/evaluate-edit-frame.spec.ts) (6 casos)
- **Código:** [packages/scene-engine/src/evaluator/evaluate-edit-frame.ts:58](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/evaluator/evaluate-edit-frame.ts#L58) · [packages/scene-engine/src/edit/resolve-shot-at-edit-frame.ts:11](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/edit/resolve-shot-at-edit-frame.ts#L11)

### MIG017

**Performance tracks (ciclo, evento com hold, estático)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (3 casos em 1 spec vitest)
- **O usuário faz:** Animar nós com ciclos senoidais, eventos de uma vez com hold e transformações estáticas, no tempo da instância (não do shot).
- **Gatilho (UI/atalho/CLI):** Bundle (`performanceTracks`, `sceneInstances[].performanceTrackIds`); sem UI.
- **Caminho handler → estado → regra/runtime:** `resolvePerformanceOverrides` → um escritor por (nó, propriedade) (lança erro se houver dois) → `mergeTransformWithOverride`.
- **Propriedades, unidades, modelo de dados:** cycle {periodFrames,amplitude,axis,phaseOffsetFrames}; event {startInstanceFrame,durationFrames,delta,holdFinal:true}; static {value}; props localRotation|localPosition|localScale.
- **Dependências:** MIG016 (Avaliação do frame de edição (shots → instância/câmera → estado dos nós))
- **Limitações:** Sem editor; criado por scripts/fixtures. Dois canais escrevendo a mesma propriedade lançam.
- **Testes (existentes, não executados):** [packages/scene-engine/src/performance/resolve-performance.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/performance/resolve-performance.spec.ts) (3 casos)
- **Código:** [packages/scene-engine/src/performance/resolve-performance.ts:32](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/performance/resolve-performance.ts#L32) · [packages/scene-engine/src/schema/render-bundle.schema.ts:297](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L297)

### MIG018

**Estrutura de shots, instâncias de cena e transições (somente leitura)**

- **Estado:** `PARC` — Parcial / placeholder · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (18 casos em 2 spec vitest)
- **O usuário faz:** O bundle declara a sequência de shots; o Studio só navega entre eles (anterior/próximo) e exibe.
- **Gatilho (UI/atalho/CLI):** Botões **Shot anterior/próximo** na barra de transporte; ao editar o bundle por JSON/CLI.
- **Caminho handler → estado → regra/runtime:** `shots[]` {editRange, instanceStartFrame, cameraClipId, cameraInFrame, transitionInto continuous|cut(reason)|dissolve(recusado)}; `stepShot` acha o início do shot vizinho.
- **Propriedades, unidades, modelo de dados:** Faixas contíguas sem lacuna/sobreposição; duração total = soma.
- **Dependências:** MIG016 (Avaliação do frame de edição (shots → instância/câmera → estado dos nós))
- **Limitações:** Sem criar/apagar/dividir/aparar/reordenar shots na UI nem na CLI do baseline (existe só no branch piloto). Dissolve rejeitado.
- **Testes (existentes, não executados):** [packages/scene-engine/src/validate/edit-joins.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/validate/edit-joins.spec.ts) (4 casos); [packages/scene-engine/src/transport/transport-controller.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/transport-controller.spec.ts) (14 casos)
- **Código:** [packages/scene-engine/src/schema/render-bundle.schema.ts:450](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L450) · [packages/scene-engine/src/transport/shot-navigation.ts:3](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/shot-navigation.ts#L3) · [packages/scene-engine/src/validate/edit-joins.ts:116](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/validate/edit-joins.ts#L116)

### MIG019

**Perfil de render (resolução, fps racional, cor, codec)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (18 casos em 1 spec vitest)
- **O usuário faz:** Fixar largura, altura, fps e as regras de cor/alpha do vídeo final.
- **Gatilho (UI/atalho/CLI):** `bundle.renderProfile` (somente leitura no Studio).
- **Caminho handler → estado → regra/runtime:** Valida `fps` igual ao do bundle; recusa `rec709-video`, alpha premultiplicado e `premultiplyOnUpload`; o canvas é sRGB de exibição.
- **Propriedades, unidades, modelo de dados:** {width,height,fps,colorSpace srgb-display,alphaMode,videoCodec h264-yuv420p,threeRevision,browserChannel}.
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** Codec e cor fixos; `threeRevision` e `browserChannel` documentam o ambiente, não selecionam nada.
- **Testes (existentes, não executados):** [packages/scene-engine/src/schema/render-bundle.schema.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.spec.ts) (18 casos)
- **Código:** [packages/scene-engine/src/schema/render-bundle.schema.ts:469](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L469) · [packages/scene-engine/src/validate/bundle-integrity.ts:476](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/validate/bundle-integrity.ts#L476)

### MIG020

**Auditoria de projeção e `frameAudit` por frame**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (7 casos em 1 spec vitest)
- **O usuário faz:** Garantir em cada frame exportado que âncoras de contato batem em pixels, planos coplanares ficam próximos, a ordem de pintura é a declarada e o fundo cobre a viewport.
- **Gatilho (UI/atalho/CLI):** Automático a cada quadro do preview (erro vira `markError`) e do render.
- **Caminho handler → estado → regra/runtime:** `auditSceneFrame(input, matrices)` no coordenador de preview e no viewer; falha lança erro com mensagem.
- **Propriedades, unidades, modelo de dados:** `frameAudit` {contacts[{anchors,maxErrorPx}], coplanarGroups, drawOrderBackToFront, backgroundNodeId, coverageMarginPx}.
- **Dependências:** MIG013 (Camadas de plano de imagem (PNG com alpha, fila de pintura única))
- **Limitações:** Valida geometria/ordem, não estética; só imagePlane.
- **Testes (existentes, não executados):** [packages/scene-engine/src/runtime/projection-audit.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/projection-audit.spec.ts) (7 casos)
- **Código:** [packages/scene-engine/src/runtime/projection-audit.ts:10](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/projection-audit.ts#L10) · [packages/scene-engine/src/schema/render-bundle.schema.ts:211](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L211)

### MIG021

**Coordenador de preview (latest-wins, descarte de quadro velho, métricas)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (4 casos em 1 spec vitest)
- **O usuário faz:** Rolar o playhead rápido sem enfileirar desenhos: só o quadro mais recente é apresentado; o resto é descartado e contado.
- **Gatilho (UI/atalho/CLI):** Qualquer seek/scrub/play.
- **Caminho handler → estado → regra/runtime:** `PreviewRenderCoordinator.requestFrame(frame, revision)` → `renderFrame(frame, shouldPresent)`; `bumpRevision` invalida; métricas requested/presented/dropped/lag.
- **Propriedades, unidades, modelo de dados:** `PreviewRenderMetrics` {requestedFrame, presentedFrame, droppedRenderRequests, presentationLagFrames}.
- **Dependências:** MIG025 (Controlador de transporte (play, pause, seek, passo, salto, shots, scrub))
- **Limitações:** Runtime recusa operações concorrentes (`frameBusy`).
- **Testes (existentes, não executados):** [packages/scene-engine/src/transport/preview-render-coordinator.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/preview-render-coordinator.spec.ts) (4 casos)
- **Código:** [packages/scene-engine/src/transport/preview-render-coordinator.ts:23](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/preview-render-coordinator.ts#L23) · [packages/scene-engine/src/runtime/three-scene-runtime.ts:1284](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/three-scene-runtime.ts#L1284)

### MIG022

**Determinismo de quadro e prova de pixel**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (0 casos em 0 spec vitest + 2 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Provar que um quadro é a mesma imagem por qualquer caminho (seek, scrub, export).
- **Gatilho (UI/atalho/CLI):** `pnpm pilot:pixel-test`, `studio:transport-test`, `studio:av-proof` (exigem Chrome/ffmpeg).
- **Caminho handler → estado → regra/runtime:** Todo estado visual é função pura de (frame de edição, câmera); clima/mão/splat são funções de tempo; scripts comparam RGBA do preview com o offline (não MP4 com perda).
- **Propriedades, unidades, modelo de dados:** Hashes SHA-256 de pixels por ambiente fixo; WebM de diagnóstico não é export.
- **Dependências:** MIG013 (Camadas de plano de imagem (PNG com alpha, fila de pintura única))
- **Limitações:** Hash vale para um ambiente de export fixo, não equivalência entre GPUs (README).
- **Testes (existentes, não executados):** [packages/scene-engine/scripts/pixel-determinism.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/pixel-determinism.mjs) (script de navegador/Chrome); [packages/scene-engine/scripts/scene-studio-transport.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-transport.spec.mjs) (script de navegador/Chrome)
- **Código:** [packages/scene-engine/scripts/pixel-determinism.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/pixel-determinism.mjs) · [packages/scene-engine/scripts/studio-av-proof.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/studio-av-proof.mjs) · [packages/scene-engine/src/runtime/three-scene-runtime.ts:1302](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/three-scene-runtime.ts#L1302)

### MIG023

**API do runtime Three.js (setters de cena, captura, descarte)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Ajustar clima, luzes, efeitos, rig, paths e outline em memória e redesenhar o quadro atual.
- **Gatilho (UI/atalho/CLI):** Chamadas do Studio (`setScene*`, `setRigTrack`, `refreshRigs`, `setOutline`) e do viewer headless.
- **Caminho handler → estado → regra/runtime:** `ThreeSceneRuntime` métodos `setSceneWeather|Effects|WeatherKeys|Lighting|Ground`, `setOutline`, `getRigTrack|setRigTrack|refreshRigs`, `renderEditFrame`, `capturePng`, `prepareForInstance`, `dispose`.
- **Propriedades, unidades, modelo de dados:** Sobrescritas em memória não gravam no bundle; o Studio grava depois.
- **Dependências:** MIG013 (Camadas de plano de imagem (PNG com alpha, fila de pintura única))
- **Limitações:** Classe de 1.560 linhas, excluída da cobertura unitária (browser checks cobrem).
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [packages/scene-engine/src/runtime/three-scene-runtime.ts:180](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/three-scene-runtime.ts#L180) · [packages/scene-engine/src/runtime/three-scene-runtime.ts:1153](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/three-scene-runtime.ts#L1153) · [packages/scene-engine/src/runtime/three-scene-runtime.ts:1514](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/three-scene-runtime.ts#L1514) · [packages/scene-engine/src/runtime/three-scene-runtime.ts:1532](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/three-scene-runtime.ts#L1532)

### MIG024

**Campos e tipos aceitos pelo schema sem consumidor no runtime/UI**

- **Estado:** `INAT` — Inativo / sem consumidor · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Nada: o bundle pode declarar esses campos, mas nenhum fluxo os usa.
- **Gatilho (UI/atalho/CLI):** —
- **Caminho handler → estado → regra/runtime:** `sceneNodeSchema.type` aceita `supportPlane` e `overlay`, mas `ensureSceneBuilt` só constrói `group`, `imagePlane` e `splat` (os demais caem em `continue`) e o inspector só tem rótulo para eles; `contactPlaneId` e `sceneInstances[].editLocked` só aparecem no schema e nos scripts de fixture; `metadata` é livre; `config.defaults` e `config.rigPresets` do registro de projetos são gravados e lidos como dados sem consumidor na UI; transição `dissolve` é aceita pelo tipo e recusada pelo `superRefine`.
- **Propriedades, unidades, modelo de dados:** Campos: nós supportPlane/overlay, contactPlaneId, editLocked, metadata, ProjectConfig.defaults/rigPresets, premultiplyOnUpload (recusado), colorSpace rec709-video (recusado).
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** Não migrar por padrão: se um bundle trouxer esses campos, o destino deve preservar (round-trip) ou recusar de forma explícita, nunca interpretar.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [packages/scene-engine/src/schema/render-bundle.schema.ts:182](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L182) · [packages/scene-engine/src/runtime/three-scene-runtime.ts:385](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/three-scene-runtime.ts#L385) · [packages/scene-engine/src/schema/render-bundle.schema.ts:246](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L246) · [packages/scene-engine/src/schema/render-bundle.schema.ts:466](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L466) · [packages/scene-engine/scripts/studio-projects.mjs:31](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/studio-projects.mjs#L31) · [packages/scene-engine/src/schema/render-bundle.schema.ts:591](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L591)

## C. Transporte, tempo e áudio

### MIG025

**Controlador de transporte (play, pause, seek, passo, salto, shots, scrub)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (14 casos em 1 spec vitest + 1 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Tocar, pausar, ir ao início/fim, avançar ±1 frame, saltar ±5 s, pular entre shots e arrastar o playhead (scrub).
- **Gatilho (UI/atalho/CLI):** Botões da barra de transporte, campo de frame, clique/arraste na waveform, atalhos (ver MIG028).
- **Caminho handler → estado → regra/runtime:** `TransportController` mantém fases loading|ready|playing|paused|seeking|ended|error; `seekToFrame` valida inteiro e faixa, invalida operações em voo, para áudio, pede quadro ao coordenador e retoma se estava tocando; `scrub*` pausa e restaura; relógio = áudio (se houver) ou `MonotonicClock`.
- **Propriedades, unidades, modelo de dados:** `TransportPublicState` {phase, editFrame, revisionToken, instanceFrame, cameraClipFrame, sourceTimeSeconds, errorMessage, metrics, wasPlayingBeforeScrub, audioPresent}.
- **Dependências:** MIG016 (Avaliação do frame de edição (shots → instância/câmera → estado dos nós))
- **Limitações:** Observado no código: `play()` só age em `phase === "paused"`; a partir de `ended` retorna sem efeito (seek explícito necessário). Não verificado em runtime. Playback segue o fim do áudio/edição declarado.
- **API / CLI:** Lib: `TransportController`, `stepShot`, `editFrameToEditSeconds` (`@tessel/scene-engine/transport`).
- **Testes (existentes, não executados):** [packages/scene-engine/src/transport/transport-controller.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/transport-controller.spec.ts) (14 casos); [packages/scene-engine/scripts/scene-studio-transport.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-transport.spec.mjs) (script de navegador/Chrome)
- **Código:** [packages/scene-engine/src/transport/transport-controller.ts:36](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/transport-controller.ts#L36) · [packages/scene-engine/src/transport/transport-controller.ts:156](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/transport-controller.ts#L156) · [packages/scene-engine/src/transport/transport-controller.ts:237](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/transport-controller.ts#L237) · [packages/scene-engine/src/transport/transport-controller.ts:329](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/transport-controller.ts#L329) · [packages/scene-engine/src/transport/monotonic-playback.ts:4](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/monotonic-playback.ts#L4)

### MIG026

**Porta de áudio do navegador (relógio de áudio, ganho do monitor, janela de edição)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (7 casos em 2 spec vitest)
- **O usuário faz:** Ouvir o mix durante o preview, alinhado ao quadro apresentado; volume e mute do monitor.
- **Gatilho (UI/atalho/CLI):** Play; controle de volume e botão de mudo na seção de áudio.
- **Caminho handler → estado → regra/runtime:** `createWebAudioTransport` decodifica o buffer, mapeia a janela do bundle (`editStartSeconds..editEndSecondsExclusive`) para a linha do tempo de edição, usa timestamps de saída do `AudioContext`; `setMonitorGain`. Boot recusa hash divergente, janela maior que o PCM ou menor que a edição.
- **Propriedades, unidades, modelo de dados:** `AudioTransportPort`; ganho linear = volume × mixGain (zerado por mute).
- **Dependências:** MIG025 (Controlador de transporte (play, pause, seek, passo, salto, shots, scrub)), MIG083 (Plano de mix: o que soa, onde a voz do mix cala porque há take)
- **Limitações:** Preview é áudio de monitor; não prova latência física de alto-falante (documentação da origem). Tarefa suspensa do `AudioContext` pausa o transporte.
- **Testes (existentes, não executados):** [packages/scene-engine/src/transport/browser/web-audio-transport.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/browser/web-audio-transport.spec.ts) (4 casos); [packages/scene-engine/src/transport/audio-window.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/audio-window.spec.ts) (3 casos)
- **Código:** [packages/scene-engine/src/transport/browser/web-audio-transport.ts:29](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/browser/web-audio-transport.ts#L29) · [packages/scene-engine/src/transport/audio-window.ts:4](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/audio-window.ts#L4) · [apps/scene-studio/src/engine/createStudioSession.ts:169](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/createStudioSession.ts#L169)

### MIG027

**Waveform da timeline com scrub e seek**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (3 casos em 1 spec vitest)
- **O usuário faz:** Ver o envelope do áudio na timeline, clicar para saltar e arrastar para fazer scrub.
- **Gatilho (UI/atalho/CLI):** Faixa da waveform no rodapé; setas dentro dela.
- **Caminho handler → estado → regra/runtime:** `peaksForEditWindow` calcula picos (512 por janela) do buffer decodificado; `TimelineWaveform` desenha, `onScrubStart/onScrub/onScrubEnd/onSeek` falam com o transporte.
- **Propriedades, unidades, modelo de dados:** `WaveformPeak[]`.
- **Dependências:** MIG026 (Porta de áudio do navegador (relógio de áudio, ganho do monitor, janela de edição))
- **Limitações:** Sem zoom/pan da timeline nem seleção de intervalo; sem waveform por linha de narração.
- **Testes (existentes, não executados):** [packages/scene-engine/src/transport/waveform-peaks.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/waveform-peaks.spec.ts) (3 casos)
- **Código:** [packages/scene-engine/src/transport/waveform-peaks.ts:43](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/transport/waveform-peaks.ts#L43) · [apps/scene-studio/src/components/TimelineWaveform.tsx:15](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/TimelineWaveform.tsx#L15)

### MIG028

**Atalhos de teclado e regras de foco**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Controlar o transporte e o histórico sem o mouse.
- **Gatilho (UI/atalho/CLI):** Espaço (play/pause); ← → (±1 frame); Shift+← → (±5 s); ⌘/Ctrl+Z, Shift+⌘Z, ⌘/Ctrl+Y (desfazer/refazer).
- **Caminho handler → estado → regra/runtime:** `keydown` global em `ProjectWorkspace`; ignora alvos de entrada (input/textarea/select, slider, switch, contenteditable); desfazer/refazer só não vale em textarea/contenteditable.
- **Propriedades, unidades, modelo de dados:** —
- **Dependências:** MIG025 (Controlador de transporte (play, pause, seek, passo, salto, shots, scrub)), MIG049 (Desfazer e refazer (200 passos, por projeto))
- **Limitações:** Sem atalho de Delete/duplicar/copiar; sem mapa de atalhos configurável; F2 renomeia na árvore de camadas.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [apps/scene-studio/src/App.tsx:152](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/App.tsx#L152) · [apps/scene-studio/src/App.tsx:46](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/App.tsx#L46) · [apps/scene-studio/src/ui/layer-tree/Row.tsx:167](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/ui/layer-tree/Row.tsx#L167)

### MIG029

**Barra de transporte, campo de frame, timecode e controle de volume**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Usar botões início/−5s/play/+5s/fim/frame anterior/próximo/shot anterior/próximo, digitar um frame e ver `mm:ss:ff`.
- **Gatilho (UI/atalho/CLI):** Rodapé do editor.
- **Caminho handler → estado → regra/runtime:** Botões chamam `transport.*`; `<input type=number>` chama `seekToFrame` se inteiro dentro de [0, N−1]; `formatTimecode`.
- **Propriedades, unidades, modelo de dados:** Timecode `mm:ss:ff` com `ff = round(frac × fps)`.
- **Dependências:** MIG025 (Controlador de transporte (play, pause, seek, passo, salto, shots, scrub))
- **Limitações:** `ff` pode chegar ao valor do fps por arredondamento (não verificado).
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [apps/scene-studio/src/App.tsx:106](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/App.tsx#L106) · [apps/scene-studio/src/engine/createStudioSession.ts:475](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/createStudioSession.ts#L475)

### MIG030

**Tempo racional, FPS e conversões**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Converter frames ↔ segundos com fps racional.
- **Gatilho (UI/atalho/CLI):** Interno.
- **Caminho handler → estado → regra/runtime:** `RationalFps`, `fpsToRate`, `editFrameToSeconds`, `secondsToEditFrame`, `editSecondsToFrame`.
- **Propriedades, unidades, modelo de dados:** fps = numerator/denominator inteiros positivos.
- **Limitações:** Conversões em ponto flutuante na origem.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [packages/scene-engine/src/types/fps.ts:1](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/types/fps.ts#L1) · [packages/scene-engine/src/types/frame-range.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/types/frame-range.ts)

## D. Câmera

### MIG031

**Clipe de câmera: pose inicial, segmentos (8 receitas + keyframes) e até 3 camadas**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (28 casos em 2 spec vitest)
- **O usuário faz:** Definir um movimento de câmera composto por receitas encadeadas (hold, dolly, zoom, truck, órbita) ou poses explícitas.
- **Gatilho (UI/atalho/CLI):** Bundle `cameraClips`; editado por comandos/propriedades e pela faixa de câmera.
- **Caminho handler → estado → regra/runtime:** Clip {initialPosition, initialTarget, initialFovDeg, segments[], layers[≤3]}; segmento de receita tem `from` initial|previousEnd e easing; `keyframes` tem keys {frame, position, target, fovDeg, easing}; camadas somam deslocamentos à trilha principal.
- **Propriedades, unidades, modelo de dados:** Receitas: hold, dollyIn/Out (distância), zoomIn/Out (Δ fov em graus), truckLeft/Right (distância), orbit (yaw/pitch graus), keyframes. `MAX_CAMERA_LAYERS = 3`. Unidades: frames, unidades de cena, graus.
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** Camadas continuam sempre do segmento anterior (sem pose própria), keyframes só na trilha principal, juntas contínuas não cruzam clipes.
- **Testes (existentes, não executados):** [packages/scene-engine/src/camera/camera-keyframes.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/camera/camera-keyframes.spec.ts) (23 casos); [packages/scene-engine/src/camera/resolve-camera-clip.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/camera/resolve-camera-clip.spec.ts) (5 casos)
- **Código:** [packages/scene-engine/src/schema/render-bundle.schema.ts:345](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L345) · [packages/scene-engine/src/schema/render-bundle.schema.ts:417](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L417) · [packages/scene-engine/src/schema/render-bundle.schema.ts:427](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L427)

### MIG032

**Resolução da câmera por frame e easings**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (28 casos em 2 spec vitest)
- **O usuário faz:** Obter a pose (posição, alvo, FOV) em qualquer frame do clipe.
- **Gatilho (UI/atalho/CLI):** Chamado a cada quadro e pelos handles do overlay.
- **Caminho handler → estado → regra/runtime:** `resolveCameraAtClipFrame(clip, frame)` percorre segmentos/camadas, aplica `applyEasing` (linear, easeInOutCubic, easeOutCubic) e soma camadas.
- **Propriedades, unidades, modelo de dados:** 3 easings nomeados; `UNKNOWN_CAMERA_EASING` na validação.
- **Dependências:** MIG031 (Clipe de câmera: pose inicial, segmentos (8 receitas + keyframes) e até 3 camadas)
- **Limitações:** Easing desconhecido lança erro em tempo de execução.
- **Testes (existentes, não executados):** [packages/scene-engine/src/camera/resolve-camera-clip.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/camera/resolve-camera-clip.spec.ts) (5 casos); [packages/scene-engine/src/camera/camera-keyframes.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/camera/camera-keyframes.spec.ts) (23 casos)
- **Código:** [packages/scene-engine/src/camera/resolve-camera-clip.ts:342](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/camera/resolve-camera-clip.ts#L342) · [packages/scene-engine/src/camera/easing.ts:10](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/camera/easing.ts#L10)

### MIG033

**Comandos de câmera (15 ops) com erros tipados e dica**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (24 casos em 1 spec vitest + 1 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Editar o clipe sem acessar a UI: início, duração, velocidade, parâmetros, easing, from, receita, adicionar/remover/mover segmento, keyframes, camadas e ajustar à duração dos shots.
- **Gatilho (UI/atalho/CLI):** CLI `scene camera <bundle> apply <clipId> '<comandos>'`; propriedades `camera/<clip>/…`; faixa de câmera na UI.
- **Caminho handler → estado → regra/runtime:** `applyCameraCommands(bundle, clipId, commands)` aplica em lote com `absorb` (quem absorve a folga), valida contra os shots que usam o clipe e devolve clipe ou erros {code,message,hint,command}.
- **Propriedades, unidades, modelo de dados:** Ops: setStart, setDuration, setSpeed, setParam, setEasing, setFrom, setRecipe, addSegment, removeSegment, toKeyframes, moveKey, addTrack, removeTrack, moveSegment, fitToShots.
- **Dependências:** MIG031 (Clipe de câmera: pose inicial, segmentos (8 receitas + keyframes) e até 3 camadas), MIG032 (Resolução da câmera por frame e easings)
- **Limitações:** Lote é atômico por chamada; códigos como CLIP_TOO_SHORT, SEGMENT_OUT_OF_RANGE, CANNOT_ABSORB.
- **API / CLI:** CLI `scene camera analyze|apply`; lib `applyCameraCommands`, `cameraCommandSchema`.
- **Testes (existentes, não executados):** [packages/scene-engine/src/camera/camera-commands.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/camera/camera-commands.spec.ts) (24 casos); [packages/scene-engine/scripts/scene-studio-camera.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-camera.spec.mjs) (script de navegador/Chrome)
- **Código:** [packages/scene-engine/src/camera/camera-commands.ts:95](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/camera/camera-commands.ts#L95) · [packages/scene-engine/src/camera/camera-commands.ts:1171](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/camera/camera-commands.ts#L1171) · [packages/scene-engine/src/camera/camera-commands.ts:34](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/camera/camera-commands.ts#L34)

### MIG034

**Propriedades de câmera por caminho (`camera/<clip>/…`)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (13 casos em 1 spec vitest)
- **O usuário faz:** Mudar a posição inicial, o ponto final de um segmento ou uma key por um caminho textual e valor JSON.
- **Gatilho (UI/atalho/CLI):** `scene set/get/props`; handles do overlay; `window.__sceneStudio.setProperty`.
- **Caminho handler → estado → regra/runtime:** `cameraProperties` expõe `start/position`, `track/<t>/segment/<s>/end/position`, `…/key/<k>/<campo>`; escrever converte a receita em keyframes e passa pelo mesmo motor de comandos.
- **Propriedades, unidades, modelo de dados:** Kind `vec3`/`number`/`easing`; erros `CAMERA_REFUSED` carregam os erros do motor.
- **Dependências:** MIG046 (Sistema de property paths (listar, ler, escrever, erros tipados)), MIG033 (Comandos de câmera (15 ops) com erros tipados e dica)
- **Limitações:** Convertem receitas em keys ao mover pontos do caminho (efeito colateral intencional).
- **Testes (existentes, não executados):** [packages/scene-engine/src/properties/camera-properties.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/camera-properties.spec.ts) (13 casos)
- **Código:** [packages/scene-engine/src/properties/camera-properties.ts:268](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/camera-properties.ts#L268) · [packages/scene-engine/src/properties/property-paths.ts:103](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/property-paths.ts#L103)

### MIG035

**Análise de clipe de câmera**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (24 casos em 1 spec vitest)
- **O usuário faz:** Pedir um relatório do clipe (segmentos, folgas, uso por shots, alertas).
- **Gatilho (UI/atalho/CLI):** `scene camera <bundle> analyze [clipId]`; inspector de câmera.
- **Caminho handler → estado → regra/runtime:** `analyzeCameraClip(bundle, id)` devolve estrutura por segmento; `null` para clipe desconhecido.
- **Propriedades, unidades, modelo de dados:** Resumo estruturado (frames, receita, pose inicial/final).
- **Dependências:** MIG031 (Clipe de câmera: pose inicial, segmentos (8 receitas + keyframes) e até 3 camadas)
- **Limitações:** Formato não documentado fora do código.
- **Testes (existentes, não executados):** [packages/scene-engine/src/camera/camera-commands.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/camera/camera-commands.spec.ts) (24 casos)
- **Código:** [packages/scene-engine/src/camera/camera-commands.ts:1307](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/camera/camera-commands.ts#L1307)

### MIG036

**Faixa de câmera na timeline (barras, arrasto, redimensionar, inserir, camadas)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (0 casos em 0 spec vitest + 1 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Ver cada movimento como barra; arrastar para mover/mudar de camada; redimensionar duração; inserir depois, remover, adicionar camada; navegar entre movimentos.
- **Gatilho (UI/atalho/CLI):** Faixa **Câmera** do rodapé; botões anterior/próximo, inserir, remover, adicionar camada.
- **Caminho handler → estado → regra/runtime:** Ponteiro → `pressBar/dragBar/dropBar/resize` → `camera.run(commands)` (`moveSegment`, `setDuration`, `addSegment`, `removeSegment`, `addTrack`) → `useCameraClip` → store → bundle; recusa mostra o `hint`.
- **Propriedades, unidades, modelo de dados:** Barras por trilha (0 principal), fatia pelo frame do clipe.
- **Dependências:** MIG033 (Comandos de câmera (15 ops) com erros tipados e dica)
- **Limitações:** Não encontrei snapping, multi-seleção nem atalho de teclado para mover barras no código lido (CameraTrack.tsx).
- **Testes (existentes, não executados):** [packages/scene-engine/scripts/scene-studio-camera.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-camera.spec.mjs) (script de navegador/Chrome)
- **Código:** [apps/scene-studio/src/components/CameraTrack.tsx:88](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/CameraTrack.tsx#L88) · [apps/scene-studio/src/hooks/useCameraClip.ts:19](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/hooks/useCameraClip.ts#L19)

### MIG037

**Overlay do caminho 3D (handles arrastáveis) e mini-mapa CameraPlan**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (6 casos em 1 spec vitest)
- **O usuário faz:** Ver o caminho da câmera sobre o viewport, arrastar pontos e ver a câmera vista de cima com camadas e luzes.
- **Gatilho (UI/atalho/CLI):** Selecionar a câmera no painel; arrastar handle; setas no handle focado.
- **Caminho handler → estado → regra/runtime:** `sampleCameraPath` → `CameraPathOverlay` (projeção com `view` do último quadro) → `onMoveHandle` → `setProperty(handle.propertyPath, position, {commit})` → converte em keyframes e salva; `CameraPlan` desenha mapa superior.
- **Propriedades, unidades, modelo de dados:** Handles com `propertyPath`.
- **Dependências:** MIG034 (Propriedades de câmera por caminho (`camera/<clip>/…`))
- **Limitações:** Depende das matrizes do último quadro apresentado; handle de camada não existe (camadas só somam).
- **Testes (existentes, não executados):** [apps/scene-studio/src/engine/cameraPath.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/cameraPath.spec.ts) (6 casos)
- **Código:** [apps/scene-studio/src/components/CameraPathOverlay.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/CameraPathOverlay.tsx) · [apps/scene-studio/src/components/CameraPlan.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/CameraPlan.tsx) · [apps/scene-studio/src/engine/cameraPath.ts:47](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/cameraPath.ts#L47)

### MIG038

**Inspector de câmera**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Editar o movimento selecionado: tipo, duração, velocidade, parâmetros, easing, ponto de partida, keys; ver análise e aviso de limite de splat.
- **Gatilho (UI/atalho/CLI):** Selecionar a câmera ou uma barra.
- **Caminho handler → estado → regra/runtime:** `describeCamera` monta o schema (slider/choice/note/action) → `LevaInspector`; mudanças viram comandos `camera.run`.
- **Propriedades, unidades, modelo de dados:** ~14 controles por seleção (campos sliders/choices/actions).
- **Dependências:** MIG033 (Comandos de câmera (15 ops) com erros tipados e dica), MIG045 (Infra do inspector (schema declarativo sobre Leva))
- **Limitações:** Controles sem rótulo i18n por tecla estão no JSON; feedback de recusa via `refuseClip`.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [apps/scene-studio/src/inspector/describeCamera.ts:525](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/inspector/describeCamera.ts#L525)

## E. Seleção, camadas, outline, inspector, propriedades e estado

### MIG039

**Modelo de seleção (12 tipos) e inspector contextual**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (18 casos em 1 spec vitest)
- **O usuário faz:** Clicar em cena, câmera, clima, efeitos, camada, luz, rig, path, follow, grupo, faixa de narração ou linha para ver e editar suas propriedades.
- **Gatilho (UI/atalho/CLI):** Clique nos itens do painel de camadas, na timeline ou nos overlays.
- **Caminho handler → estado → regra/runtime:** `useEditor.select(selection)` → `StudioEditor` escolhe o `describe*` correspondente (grupo > linha > faixa > luz > câmera > follow > path > rig > camada > clima > efeitos > cena) e mostra no `LevaInspector`; seleção inexistente cai para a cena.
- **Propriedades, unidades, modelo de dados:** `StudioSelection` = scene|weather|effects|camera{track?,segment?}|layer|light|rig{boneId?,constraintId?}|path{pointIndex?}|follow|group|narrationTrack|narrationLine.
- **Limitações:** Seleção única (sem multi-seleção); seleção de luz/camada vale na definição de cena do playhead.
- **Testes (existentes, não executados):** [apps/scene-studio/src/engine/editorStore.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/editorStore.spec.ts) (18 casos)
- **Código:** [apps/scene-studio/src/engine/editorStore.ts:55](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/editorStore.ts#L55) · [apps/scene-studio/src/components/StudioEditor.tsx:80](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/StudioEditor.tsx#L80)

### MIG040

**Painel de camadas (Cena, Câmera, Clima, Efeitos, árvore de nós, luzes, áudio/texto)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (18 casos em 1 spec vitest)
- **O usuário faz:** Navegar pelas camadas da cena, luzes e linhas de narração; mostrar/ocultar luzes e faixas; silenciar; renomear; apagar luz/linha; adicionar faixa.
- **Gatilho (UI/atalho/CLI):** Coluna esquerda do editor.
- **Caminho handler → estado → regra/runtime:** `LayersPanel` compõe `LayersSection` (Cena/Câmera/Clima/Efeitos + árvore de nós), `LightsSection`, `AudioSection` (áudio da cena + faixas + linhas), `AudioLayer` (monitor) e `RigTree`/`FollowTree`; ações chamam `lighting.setLight`, `narration.run`, `outline.run`.
- **Propriedades, unidades, modelo de dados:** Linhas de árvore (`TreeRowData`) com ícone/rótulo/ações.
- **Dependências:** MIG039 (Modelo de seleção (12 tipos) e inspector contextual), MIG042 (Outline: grupos de camadas, luzes e áudio com ocultar/silenciar/recolher)
- **Limitações:** Camadas de imagem são somente leitura (sem ocultar/solo/reordenar/apagar camada de imagem); só luzes e linhas/faixas de narração têm ações.
- **Testes (existentes, não executados):** [apps/scene-studio/src/engine/editorStore.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/editorStore.spec.ts) (18 casos)
- **Código:** [apps/scene-studio/src/components/LayersPanel.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/LayersPanel.tsx) · [apps/scene-studio/src/components/layers/LayersSection.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/layers/LayersSection.tsx) · [apps/scene-studio/src/components/layers/LightsSection.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/layers/LightsSection.tsx) · [apps/scene-studio/src/components/layers/AudioSection.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/layers/AudioSection.tsx)

### MIG041

**Árvore de camadas: arrastar e soltar (mouse e teclado), renomear com F2, ações por linha**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Reordenar e agrupar itens arrastando (antes/depois/dentro), com teclado (espaço pega, setas escolhem, espaço solta), renomear com duplo clique ou F2.
- **Gatilho (UI/atalho/CLI):** `LayerTree` no painel de camadas.
- **Caminho handler → estado → regra/runtime:** `@dnd-kit` (`useSortable`) + `dropPosition` (quartos superior/inferior de grupo = antes/depois, meio = dentro) → `outline.run([{op:'moveItem'|'moveGroup'}])`; `Row` edita nome (Enter confirma, Esc cancela).
- **Propriedades, unidades, modelo de dados:** `DropPosition` before|after|into.
- **Dependências:** MIG043 (Comandos de outline (6 ops))
- **Limitações:** Só reordena o outline (grupos/itens), não a ordem de pintura das camadas.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [apps/scene-studio/src/ui/layer-tree/LayerTree.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/ui/layer-tree/LayerTree.tsx) · [apps/scene-studio/src/ui/layer-tree/dropPosition.ts:26](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/ui/layer-tree/dropPosition.ts#L26) · [apps/scene-studio/src/ui/layer-tree/Row.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/ui/layer-tree/Row.tsx)

### MIG042

**Outline: grupos de camadas, luzes e áudio com ocultar/silenciar/recolher**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (54 casos em 2 spec vitest)
- **O usuário faz:** Organizar camadas, luzes e faixas de narração em grupos aninhados; ocultar um grupo de luzes (apaga) ou de linhas (some a legenda), silenciar um grupo de áudio.
- **Gatilho (UI/atalho/CLI):** Painel de camadas; inspector de grupo; `scene outline`.
- **Caminho handler → estado → regra/runtime:** `outline` no bundle {groups[{id,label,section,parentId,order,hidden?,muted?,collapsed?}], items[{key 'node|light|track:<id>', groupId, order}]}; `outlineTree` gera a árvore; `effectiveFlags`/`litLighting` aplicam o ocultar herdado; `explainOutline` valida.
- **Propriedades, unidades, modelo de dados:** Seções `layers|lights|audio`; `hidden` não vale para camadas; `muted` só para áudio; ids `[a-z0-9_-]{1,64}`.
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** Aplicado só a luzes (apaga) e narração (legenda/áudio); camadas de imagem não ocultam.
- **Testes (existentes, não executados):** [packages/scene-engine/src/outline/outline.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/outline/outline.spec.ts) (44 casos); [packages/scene-engine/src/outline/flags.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/outline/flags.spec.ts) (10 casos)
- **Código:** [packages/scene-engine/src/outline/outline-schema.ts:50](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/outline/outline-schema.ts#L50) · [packages/scene-engine/src/outline/outline.ts:121](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/outline/outline.ts#L121) · [packages/scene-engine/src/outline/outline.ts:452](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/outline/outline.ts#L452) · [packages/scene-engine/src/outline/outline.ts:394](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/outline/outline.ts#L394) · [packages/scene-engine/src/outline/flags.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/outline/flags.ts)

### MIG043

**Comandos de outline (6 ops)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (44 casos em 1 spec vitest)
- **O usuário faz:** Criar, renomear, remover, mover itens/grupos e alternar hidden/muted/collapsed.
- **Gatilho (UI/atalho/CLI):** CLI `scene outline <bundle> show|apply '<cmds>'`; UI da árvore.
- **Caminho handler → estado → regra/runtime:** `applyOutlineCommands` → ops addGroup, renameGroup, removeGroup, moveItem, moveGroup, setGroup; `editBundle` aceita `{outline: cmd}`.
- **Propriedades, unidades, modelo de dados:** —
- **Dependências:** MIG042 (Outline: grupos de camadas, luzes e áudio com ocultar/silenciar/recolher)
- **Limitações:** Erros com código/dica; item/grupo desconhecido recusado.
- **API / CLI:** CLI `scene outline show|apply`; lib `applyOutlineCommands`, `outlineCommandSchema`.
- **Testes (existentes, não executados):** [packages/scene-engine/src/outline/outline.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/outline/outline.spec.ts) (44 casos)
- **Código:** [packages/scene-engine/src/outline/outline.ts:570](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/outline/outline.ts#L570) · [packages/scene-engine/src/outline/outline.ts:941](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/outline/outline.ts#L941)

### MIG044

**Inspector de cena, nó e grupo**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Ver tipo da camada, alternar a sombra que a camada projeta (até 4) e, em imagePlane, vinculá-la a um rig; ver o resumo de cena (clima + luz); gerir grupo (rótulo, visível, audível, remover).
- **Gatilho (UI/atalho/CLI):** Selecionar cena, nó ou grupo.
- **Caminho handler → estado → regra/runtime:** `describeNode`/`describeScene`/`describeGroup` → `LevaInspector`; camada não tem transformação editável.
- **Propriedades, unidades, modelo de dados:** Campos note/switch/action.
- **Dependências:** MIG039 (Modelo de seleção (12 tipos) e inspector contextual)
- **Limitações:** Não há edição de posição/escala/rotação/ordem de camadas no Studio.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [apps/scene-studio/src/inspector/describeNode.ts:25](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/inspector/describeNode.ts#L25) · [apps/scene-studio/src/inspector/describeScene.ts:326](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/inspector/describeScene.ts#L326) · [apps/scene-studio/src/inspector/describeGroup.ts:26](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/inspector/describeGroup.ts#L26)

### MIG045

**Infra do inspector (schema declarativo sobre Leva)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Cada seleção declara campos (nota, slider, switch, escolha, texto, ação) e a UI os renderiza com chaves únicas e rótulos acessíveis.
- **Gatilho (UI/atalho/CLI):** Coluna direita.
- **Caminho handler → estado → regra/runtime:** `InspectorSchema` {id,title,sections[fields]} → `buildLevaSchema` → `LevaInspector key={schema.id}`.
- **Propriedades, unidades, modelo de dados:** Tipos de campo: note, slider, switch, choice, text, action.
- **Limitações:** Dependência `leva` (tema próprio) — descartável.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [apps/scene-studio/src/ui/leva/schema.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/ui/leva/schema.ts) · [apps/scene-studio/src/ui/leva/buildLevaSchema.ts:7](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/ui/leva/buildLevaSchema.ts#L7) · [apps/scene-studio/src/ui/leva/LevaInspector.tsx:42](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/ui/leva/LevaInspector.tsx#L42)

### MIG046

**Sistema de property paths (listar, ler, escrever, erros tipados)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (95 casos em 5 spec vitest)
- **O usuário faz:** Ler/escrever qualquer valor editável por um caminho textual e valor JSON, com validação, limites e erros tipados.
- **Gatilho (UI/atalho/CLI):** CLI `scene props|get|set`; `window.__sceneStudio.setProperty/getProperty/listProperties`.
- **Caminho handler → estado → regra/runtime:** `describeBundleProperties` lista `Property {path,kind,min,max,options,read,validate,write}`; `setProperties` aplica em lote com `editFrame`; ids são URI-encoded; `PropertyChange` por domínio (lighting, weather, weatherKeys, effects, ground, camera, rig, path).
- **Propriedades, unidades, modelo de dados:** Famílias: `scene/<s>/lighting/ambient`, `scene/<s>/shadows/contact`, `scene/<s>/light/<l>/{intensity,radius,flicker,hidden,color,position[/x|y|z],parent,<canal>/keys…}`, `scene/<s>/weather[/intensity|puddles|settle|covered|intensity/keys…]`, `scene/<s>/ground/top`, `scene/<s>/effects/<kind>`, `scene/<s>/path/<p>/…`, `camera/<c>/…`, `rig/<r>/…`. Kinds: number, boolean, vec3, color, weather, keys, layer, easing, vec2, choice. Erros: UNKNOWN_PATH/SCENE/LIGHT/LAYER/CLIP/RIG, RIG_REFUSED, PATH_REFUSED, CAMERA_REFUSED, INVALID_VALUE, NOT_APPLICABLE, NOT_READY.
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** O conjunto de caminhos depende do estado (luz criada passa a ter caminhos).
- **API / CLI:** CLI `scene props [--prefix] [--frame]`, `get`, `set <path>=<json> [--write]`.
- **Testes (existentes, não executados):** [packages/scene-engine/src/properties/properties.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/properties.spec.ts) (21 casos); [packages/scene-engine/src/properties/bundle-properties.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/bundle-properties.spec.ts) (24 casos); [packages/scene-engine/src/properties/camera-properties.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/camera-properties.spec.ts) (13 casos); [packages/scene-engine/src/properties/effects-properties.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/effects-properties.spec.ts) (14 casos); [packages/scene-engine/src/properties/key-properties.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/key-properties.spec.ts) (23 casos)
- **Código:** [packages/scene-engine/src/properties/bundle-properties.ts:421](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/bundle-properties.ts#L421) · [packages/scene-engine/src/properties/bundle-properties.ts:321](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/bundle-properties.ts#L321) · [packages/scene-engine/src/properties/index.ts:79](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/index.ts#L79) · [packages/scene-engine/src/properties/types.ts:131](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/types.ts#L131) · [packages/scene-engine/src/properties/property-paths.ts:35](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/property-paths.ts#L35)

### MIG047

**Keyframes de valores (resolver, regras, adicionar/remover no playhead)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (44 casos em 5 spec vitest)
- **O usuário faz:** Animar luz (intensidade/cor/posição) e intensidade do clima com chaves {frame, valor, easing}; adicionar chave no playhead, remover, mover.
- **Gatilho (UI/atalho/CLI):** Inspector (ação 'adicionar chave'), faixa **Chaves** na timeline, `window.__sceneStudio.addKey/removeKey`.
- **Caminho handler → estado → regra/runtime:** `resolveKeyframes(keys, frame, mix)` interpola entre chaves; regras `keyframe-rules` (ordem estritamente crescente, easing válido); `addKey`/`removeKey` em `key-properties` escrevem a chave no frame do playhead com o valor lido.
- **Propriedades, unidades, modelo de dados:** Key {frame ≥0 inteiro, value, easing ∈ {linear, easeInOutCubic, easeOutCubic}}; nunca vazia (vazia = propriedade ausente).
- **Dependências:** MIG046 (Sistema de property paths (listar, ler, escrever, erros tipados))
- **Limitações:** Mover chave arrastando existe só no branch piloto; no baseline a faixa **Chaves** é de leitura/seleção.
- **API / CLI:** CLI `scene set …/keys=<json>`; lib `resolveKeyframes`, `addKey`, `removeKey`.
- **Testes (existentes, não executados):** [packages/scene-engine/src/keyframes/resolve-keyframes.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/keyframes/resolve-keyframes.spec.ts) (7 casos); [packages/scene-engine/src/keyframes/keyframe-rules.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/keyframes/keyframe-rules.spec.ts) (6 casos); [packages/scene-engine/src/properties/key-properties.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/key-properties.spec.ts) (23 casos); [apps/scene-studio/src/engine/editorStoreKeys.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/editorStoreKeys.spec.ts) (6 casos); [apps/scene-studio/src/engine/keyTracks.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/keyTracks.spec.ts) (2 casos)
- **Código:** [packages/scene-engine/src/keyframes/resolve-keyframes.ts:29](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/keyframes/resolve-keyframes.ts#L29) · [packages/scene-engine/src/keyframes/keyframe-rules.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/keyframes/keyframe-rules.ts) · [packages/scene-engine/src/properties/key-properties.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/key-properties.ts) · [apps/scene-studio/src/components/KeysTrack.tsx:27](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/KeysTrack.tsx#L27)

### MIG048

**Store do editor (mostrar, comprometer, salvar, reverter)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (56 casos em 4 spec vitest)
- **O usuário faz:** Ver a mudança na hora; comprometer ao soltar o controle; salvar; se a gravação falha, voltar ao último salvo.
- **Gatilho (UI/atalho/CLI):** Todo controle do inspector e todo overlay arrastável.
- **Caminho handler → estado → regra/runtime:** `useEditor` (zustand): `setProperty(path, value, {commit})` → `applyPropertyChange` → `show` (runtime) → com `commit` grava, empilha histórico e atualiza `SaveState`; `createHistoryDomain` guarda `saved`, `committed`, `latest` para reversão correta; slices de rig e path.
- **Propriedades, unidades, modelo de dados:** Estado: session, selection, lighting/weather/weatherKeys/grounds/effects/clips por cena, `saves` {saving,saved,error}.
- **Dependências:** MIG046 (Sistema de property paths (listar, ler, escrever, erros tipados)), MIG049 (Desfazer e refazer (200 passos, por projeto))
- **Limitações:** Sem estado de salvamento unificado (save por domínio); falha de gravação reverte só se nada mais novo ocorreu.
- **Testes (existentes, não executados):** [apps/scene-studio/src/engine/editorStore.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/editorStore.spec.ts) (18 casos); [apps/scene-studio/src/engine/editorStoreKeys.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/editorStoreKeys.spec.ts) (6 casos); [apps/scene-studio/src/engine/rigStore.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/rigStore.spec.ts) (18 casos); [apps/scene-studio/src/engine/followStore.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/followStore.spec.ts) (14 casos)
- **Código:** [apps/scene-studio/src/engine/editorStore.ts:795](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/editorStore.ts#L795) · [apps/scene-studio/src/engine/editorStore.ts:200](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/editorStore.ts#L200) · [apps/scene-studio/src/engine/editorStore.ts:153](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/editorStore.ts#L153)

### MIG049

**Desfazer e refazer (200 passos, por projeto)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (18 casos em 1 spec vitest + 1 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Desfazer/refazer qualquer mudança comprometida (slider solto, interruptor, luz adicionada).
- **Gatilho (UI/atalho/CLI):** Botões na barra do cabeçalho; ⌘Z/⇧⌘Z/⌘Y.
- **Caminho handler → estado → regra/runtime:** `createHistory` (LIMIT 200); `recordChange` ignora mudança sem efeito (JSON igual); `restore` volta ao valor pelo mesmo caminho de gravação; nova mudança descarta o ramo refeito.
- **Propriedades, unidades, modelo de dados:** `HistoryEntry` {id,label,undo,redo}.
- **Limitações:** Registra só valores comprometidos (nunca movimentos de arraste); falha de gravação remove a entrada.
- **Testes (existentes, não executados):** [packages/scene-engine/scripts/scene-studio-history.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-history.spec.mjs) (script de navegador/Chrome); [apps/scene-studio/src/engine/editorStore.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/editorStore.spec.ts) (18 casos)
- **Código:** [apps/scene-studio/src/engine/history.ts:33](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/history.ts#L33) · [apps/scene-studio/src/engine/history.ts:93](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/history.ts#L93) · [apps/scene-studio/src/App.tsx:63](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/App.tsx#L63)

### MIG050

**Pipeline de salvamento por domínio e edição em lote**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (5 casos em 1 spec vitest + 3 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Cada alteração do editor é gravada no bundle imediatamente, com status de 'salvando/salvo/erro'.
- **Gatilho (UI/atalho/CLI):** Automático ao comprometer.
- **Caminho handler → estado → regra/runtime:** `createBundleSavers(base)` → PUTs `/bundle-weather|-effects|-lighting|…` (objetos inteiros por cena, validados por `parseSceneSave`) e `/bundle-edit`/`-camera-commands` (listas de edição).
- **Propriedades, unidades, modelo de dados:** `SceneSave` por tipo: weather, weatherKeys, effects, ground, lighting, rig, path.
- **Dependências:** MIG003 (Arquivo do bundle: abrir, escrita atômica serializada, dry-run e finalize)
- **Limitações:** Sem autosave/rascunho: toda alteração é gravação direta no arquivo (sem 'salvar como' nem recuperação).
- **Testes (existentes, não executados):** [apps/scene-studio/src/engine/projectsApi.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/projectsApi.spec.ts) (5 casos); [packages/scene-engine/scripts/scene-studio-weather.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-weather.spec.mjs) (script de navegador/Chrome); [packages/scene-engine/scripts/scene-studio-light.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-light.spec.mjs) (script de navegador/Chrome); [packages/scene-engine/scripts/scene-studio-effects.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-effects.spec.mjs) (script de navegador/Chrome)
- **Código:** [apps/scene-studio/src/engine/saveSceneWeather.ts:100](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/saveSceneWeather.ts#L100) · [packages/scene-engine/src/properties/scene-saves.ts:112](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/scene-saves.ts#L112) · [apps/scene-studio/src/engine/saveBundleEdit.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/saveBundleEdit.ts)

### MIG051

**Internacionalização pt-br/en/es (383 chaves)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Usar o Studio em três idiomas.
- **Gatilho (UI/atalho/CLI):** Idioma do navegador via `useTranslations`.
- **Caminho handler → estado → regra/runtime:** `useTranslations` carrega `en.json|es.json|pt-br.json` (383 chaves cada: camera 44, narration 37, rig inspector 36, follow inspector 33, layers 29, light 27, weather 22, outline 22 …).
- **Propriedades, unidades, modelo de dados:** Chaves planas aninhadas por área.
- **Limitações:** Paridade de chaves verificada entre os três idiomas (383 = 383 = 383). As descrições dos looks do motor (`effects/catalog.ts`) estão só em inglês.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [apps/scene-studio/src/i18n/useTranslations.ts:27](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/i18n/useTranslations.ts#L27) · [apps/scene-studio/src/i18n/pt-br.json](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/i18n/pt-br.json)

### MIG052

**Leitura de debug do transporte**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Ver fase, editFrame, instanceFrame, cameraClipFrame, tempo da fonte e métricas (pedido/apresentado/descartado/atraso).
- **Gatilho (UI/atalho/CLI):** Rodapé do inspector.
- **Caminho handler → estado → regra/runtime:** `StudioEditor` imprime `state.*`.
- **Propriedades, unidades, modelo de dados:** —
- **Dependências:** MIG021 (Coordenador de preview (latest-wins, descarte de quadro velho, métricas))
- **Limitações:** Texto fixo, só diagnóstico.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [apps/scene-studio/src/components/StudioEditor.tsx:580](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/StudioEditor.tsx#L580)

### MIG053

**API global de teste `window.__sceneStudio`**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (0 casos em 0 spec vitest + 3 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Scripts e provas Playwright dirigem o editor por API (transport, setProperty, rigCommand, setPath…) e leem estado.
- **Gatilho (UI/atalho/CLI):** Disponível quando o projeto ativo está pronto.
- **Caminho handler → estado → regra/runtime:** `publishGlobal` expõe a API do projeto ativo + `projects` + `hibernated()` + `setHotLimit`.
- **Propriedades, unidades, modelo de dados:** 29 métodos por projeto (transport, getState, bundle, getSceneWeather/setSceneWeather, get/setSceneLighting, set/getProperty, add/removeKey, listProperties, getRig/bakeRig/settleRigs/createRig/deleteRig/rigCommand/bindLayer, getPath/setPath/createPath/generatePath/deletePath/pathCommand, getFollow/bakeFollow/setReveal, history) + `projects`, `active`, `hibernated()`, `setHotLimit`.
- **Limitações:** Superfície só de teste; sem contrato versionado.
- **Testes (existentes, não executados):** [packages/scene-engine/scripts/scene-studio-camera.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-camera.spec.mjs) (script de navegador/Chrome); [packages/scene-engine/scripts/scene-studio-follow.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-follow.spec.mjs) (script de navegador/Chrome); [packages/scene-engine/scripts/scene-studio-rig.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-rig.spec.mjs) (script de navegador/Chrome)
- **Código:** [apps/scene-studio/src/engine/projectsRegistry.ts:165](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/projectsRegistry.ts#L165) · [apps/scene-studio/src/engine/projectsRegistry.ts:234](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/projectsRegistry.ts#L234)

## F. Iluminação e sombras

### MIG054

**Modelo de iluminação da cena (ambiente + até 4 luzes pontuais)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (13 casos em 2 spec vitest + 1 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Acrescentar luzes pontuais coloridas sobre a arte, controlar o quanto da luz da própria arte permanece e esconder uma luz sem apagá-la.
- **Gatilho (UI/atalho/CLI):** Ação **Adicionar luz** no inspector de cena; seção **Luzes**; inspector de luz.
- **Caminho handler → estado → regra/runtime:** `SceneLighting` {ambient 0–1.5, lights[≤4], shadows?} → `lightAmount`/`packLights` (uniformes de até 4 luzes) → material dos planos e dos splats; `useSceneLighting` aplica no runtime e salva em `/bundle-lighting`.
- **Propriedades, unidades, modelo de dados:** Luz {id, position (cena; offset do `parent` se houver), color rgb 0–1, intensity 0–20, radius >0 (queda a 1/4), flicker 0–1, hidden?, parent?, keys?{intensity,color,position}}.
- **Dependências:** MIG013 (Camadas de plano de imagem (PNG com alpha, fila de pintura única)), MIG046 (Sistema de property paths (listar, ler, escrever, erros tipados))
- **Limitações:** `MAX_SCENE_LIGHTS = 4`. A cor no inspector é uma escolha de presets (swatches), não seletor livre.
- **API / CLI:** Props `scene/<s>/light/<l>/…`, `scene/<s>/lighting/ambient`; CLI `scene set`.
- **Testes (existentes, não executados):** [packages/scene-engine/src/lighting/scene-lighting.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/lighting/scene-lighting.spec.ts) (7 casos); [packages/scene-engine/src/lighting/resolve-scene-lighting.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/lighting/resolve-scene-lighting.spec.ts) (6 casos); [packages/scene-engine/scripts/scene-studio-light.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-light.spec.mjs) (script de navegador/Chrome)
- **Código:** [packages/scene-engine/src/lighting/scene-lighting.ts:11](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/lighting/scene-lighting.ts#L11) · [packages/scene-engine/src/schema/render-bundle.schema.ts:154](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L154) · [apps/scene-studio/src/hooks/useSceneLighting.ts:159](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/hooks/useSceneLighting.ts#L159) · [packages/scene-engine/src/properties/light-properties.ts:167](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/light-properties.ts#L167)

### MIG055

**Keys de luz (intensidade, cor, posição) e luz que segue uma camada**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (17 casos em 2 spec vitest)
- **O usuário faz:** Animar a luz no tempo e prender uma luz a uma camada (a posição vira deslocamento).
- **Gatilho (UI/atalho/CLI):** Inspector de luz (adicionar/remover chave no playhead; escolha de camada pai); faixa **Chaves**.
- **Caminho handler → estado → regra/runtime:** `resolveLightingAtFrame` resolve keys com `resolveKeyframes` e `layerPositionsAtEditFrame` soma a posição animada da camada pai.
- **Propriedades, unidades, modelo de dados:** `keys.{intensity|color|position}` = Keyframe[].
- **Dependências:** MIG054 (Modelo de iluminação da cena (ambiente + até 4 luzes pontuais)), MIG047 (Keyframes de valores (resolver, regras, adicionar/remover no playhead))
- **Limitações:** Arrastar chaves da luz na timeline só no branch piloto.
- **Testes (existentes, não executados):** [packages/scene-engine/src/lighting/light-keys.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/lighting/light-keys.spec.ts) (11 casos); [packages/scene-engine/src/lighting/resolve-scene-lighting.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/lighting/resolve-scene-lighting.spec.ts) (6 casos)
- **Código:** [packages/scene-engine/src/lighting/light-keys.ts:15](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/lighting/light-keys.ts#L15) · [packages/scene-engine/src/lighting/resolve-scene-lighting.ts:12](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/lighting/resolve-scene-lighting.ts#L12)

### MIG056

**Planos iluminados: mapas de normais e relevo**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (6 casos em 1 spec vitest)
- **O usuário faz:** Fazer a luz atingir a pintura onde a superfície está (chão em sua profundidade, pilar à frente da parede) em vez do plano plano.
- **Gatilho (UI/atalho/CLI):** Nó com `normalMap` e/ou `relief` no bundle; sem UI.
- **Caminho handler → estado → regra/runtime:** `rebuildPlaneEffects` constrói o material de `plane-effects-material.ts` (562 linhas GLSL): normal (RGB, convenção OpenGL, fonte moge-3|sprite-normals|luminance), relevo (grade f32 + intrínsecos + planeDepth).
- **Propriedades, unidades, modelo de dados:** `normalMap {assetRef, source}`; `relief {depthAssetRef, cols, rows, intrinsics, planeDepth, source:moge-3}`.
- **Dependências:** MIG054 (Modelo de iluminação da cena (ambiente + até 4 luzes pontuais))
- **Limitações:** Depende de mapas produzidos por ferramentas externas (MoGe-3); sem geração no editor.
- **Testes (existentes, não executados):** [packages/scene-engine/src/lighting/foot-profile.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/lighting/foot-profile.spec.ts) (6 casos)
- **Código:** [packages/scene-engine/src/runtime/plane-effects-material.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/plane-effects-material.ts) · [packages/scene-engine/src/schema/render-bundle.schema.ts:106](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L106) · [packages/scene-engine/src/schema/render-bundle.schema.ts:117](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L117)

### MIG057

**Sombras de personagens: contato no chão e projetada (até 4 casters)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (13 casos em 2 spec vitest)
- **O usuário faz:** Ancorar figuras recortadas ao chão com sombra de contato e fazer figuras bloquearem a luz das lâmpadas.
- **Gatilho (UI/atalho/CLI):** Interruptor de sombra no inspector da camada; slider de **contato** no inspector de cena.
- **Caminho handler → estado → regra/runtime:** `footProfile(texture)` acha a linha dos pés; `plane-shadows.ts` desenha um quad antes de cada caster que multiplica o que está atrás pela luz restante (oito amostras por lâmpada = penumbra) sobre chão e parede traseira.
- **Propriedades, unidades, modelo de dados:** `lighting.shadows {casters[1..4 ids de imagePlane], contact 0–1}`.
- **Dependências:** MIG056 (Planos iluminados: mapas de normais e relevo)
- **Limitações:** `MAX_SHADOW_CASTERS = 4`; recusa caster que não seja imagePlane; WebGL não coberto por teste unitário.
- **Testes (existentes, não executados):** [packages/scene-engine/src/lighting/foot-profile.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/lighting/foot-profile.spec.ts) (6 casos); [packages/scene-engine/src/lighting/scene-lighting.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/lighting/scene-lighting.spec.ts) (7 casos)
- **Código:** [packages/scene-engine/src/runtime/plane-shadows.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/plane-shadows.ts) · [packages/scene-engine/src/lighting/foot-profile.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/lighting/foot-profile.ts) · [packages/scene-engine/src/lighting/scene-lighting.ts:65](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/lighting/scene-lighting.ts#L65)

### MIG058

**Luz dinâmica em splats (Spark world modifier)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Iluminar a cena de splats com as mesmas luzes pontuais e o relâmpago.
- **Gatilho (UI/atalho/CLI):** Automático quando a cena de splat tem luzes ou relâmpago.
- **Caminho handler → estado → regra/runtime:** `SplatLighting` instala um modificador (dyno) que usa a normal = eixo mais plano do splat voltada à câmera; cor = `rgb*grade*ambient + rgb*lamps`.
- **Propriedades, unidades, modelo de dados:** Mesmos parâmetros das luzes.
- **Dependências:** MIG054 (Modelo de iluminação da cena (ambiente + até 4 luzes pontuais)), MIG014 (Camadas de splat Gaussiano (Spark 2.4, `.spz`))
- **Limitações:** Só enquanto há luz ou relâmpago; sem teste unitário.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [packages/scene-engine/src/runtime/splat-lighting.ts:24](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/splat-lighting.ts#L24)

### MIG059

**UI de luzes: seção, marcador arrastável no viewport e inspector**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (0 casos em 0 spec vitest + 1 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Selecionar uma luz, arrastar seu marcador no viewport (posição) e editar cor, intensidade, raio, tremor, x/y/z, visibilidade e camada pai; remover.
- **Gatilho (UI/atalho/CLI):** Seção **Luzes**; `LightOverlay` sobre o viewport; inspector.
- **Caminho handler → estado → regra/runtime:** `LightOverlay` (ponteiro) → `lighting.setLight(id,'position',pos,commit)` (posição mundo menos âncora do pai) → store → runtime/salva.
- **Propriedades, unidades, modelo de dados:** Posição arredondada a 0,01.
- **Dependências:** MIG054 (Modelo de iluminação da cena (ambiente + até 4 luzes pontuais))
- **Limitações:** Overlay depende das matrizes do último quadro; 2D sobre a projeção.
- **Testes (existentes, não executados):** [packages/scene-engine/scripts/scene-studio-light.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-light.spec.mjs) (script de navegador/Chrome)
- **Código:** [apps/scene-studio/src/components/LightOverlay.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/LightOverlay.tsx) · [apps/scene-studio/src/components/layers/LightsSection.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/layers/LightsSection.tsx) · [apps/scene-studio/src/inspector/describeLight.ts:168](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/inspector/describeLight.ts#L168)

### MIG060

**Ferramentas de mapas (normais, relevo, superfície)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Gerar e importar para o bundle os mapas de normal, relevo e superfície (clima) a partir de uma imagem.
- **Gatilho (UI/atalho/CLI):** `scripts/lighting/*` (python + importadores .mjs), `scripts/surface-map/*`.
- **Caminho handler → estado → regra/runtime:** Python (numpy/Pillow, saída do MoGe-3 em .npz) → arquivo → `import-*-map.mjs` valida hash/tamanho e grava atomicamente no bundle.
- **Propriedades, unidades, modelo de dados:** RGB linear; intrínsecos como frações da imagem.
- **Dependências:** MIG005 (Matriz de formatos de asset e recusas explícitas)
- **Limitações:** Requer ambiente Python e saída do MoGe-3 (GPU, fora do repositório).
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [packages/scene-engine/scripts/lighting/build_normal_map.py](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/lighting/build_normal_map.py) · [packages/scene-engine/scripts/lighting/build_relief_map.py](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/lighting/build_relief_map.py) · [packages/scene-engine/scripts/surface-map/build_surface_map.py](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/surface-map/build_surface_map.py) · [packages/scene-engine/scripts/surface-map/import-surface-map.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/surface-map/import-surface-map.mjs) · [packages/scene-engine/scripts/lighting/import-normal-map.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/lighting/import-normal-map.mjs)

## G. Clima

### MIG061

**Gramática do clima (palavras, intensidade, erro de digitação)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (21 casos em 2 spec vitest)
- **O usuário faz:** Descrever o clima por um nome curto: `rain`, `snow`, `storm`, `blizzard`, `rain+night`, `snow:0.6`.
- **Gatilho (UI/atalho/CLI):** Escolha no painel de clima; `scene set scene/<s>/weather="storm:0.7"`.
- **Caminho handler → estado → regra/runtime:** `weatherNameProblem`/`parseWeather` validam ao carregar o bundle; `weather-choice` mapeia escolha ↔ nome.
- **Propriedades, unidades, modelo de dados:** Palavras: rain, snow, puddles, settle, covered, lightning, day, dusk, night, overcast, storm (= rain+puddles+lightning+night), blizzard (= snow+settle+overcast), none; intensidade 0–1,5 (padrão 1).
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** `settle` exige neve; palavras que precisam de cena em profundidade (`free`) são recusadas em planos.
- **Testes (existentes, não executados):** [packages/scene-engine/src/weather/weather-spec.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-spec.spec.ts) (14 casos); [packages/scene-engine/src/weather/weather-choice.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-choice.spec.ts) (7 casos)
- **Código:** [packages/scene-engine/src/weather/weather-spec.ts:35](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-spec.ts#L35) · [packages/scene-engine/src/weather/weather-spec.ts:45](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-spec.ts#L45) · [packages/scene-engine/src/weather/weather-choice.ts:7](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-choice.ts#L7)

### MIG062

**Renderização do clima em planos (chuva, neve, relâmpago, grade de cor)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (22 casos em 2 spec vitest)
- **O usuário faz:** Ver chuva/neve atravessando a cena (a câmera se move por dentro), relâmpago duplo/simples e a grade dia/entardecer/noite/nublado.
- **Gatilho (UI/atalho/CLI):** Escolher clima; play/seek.
- **Caminho handler → estado → regra/runtime:** `WeatherLayer`: gotas/flocos são quads instanciados numa caixa fixa no mundo, recortada por plano e com `renderOrder` próprio para ficar atrás do plano à frente; posições = função pura de (partícula, tempo de edição); `applyPlaneGrade` e `LightningFrame` no material dos planos.
- **Propriedades, unidades, modelo de dados:** Tamanho de partícula escala com o plano mais alto; relâmpago em 1,2 s e 1,34 s (duplo) e 3,9 s (simples).
- **Dependências:** MIG061 (Gramática do clima (palavras, intensidade, erro de digitação)), MIG013 (Camadas de plano de imagem (PNG com alpha, fila de pintura única))
- **Limitações:** Grade é cor sobre a arte pintada, não re-iluminação. Camadas WebGL sem teste unitário.
- **Testes (existentes, não executados):** [packages/scene-engine/src/weather/weather-model.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-model.spec.ts) (16 casos); [packages/scene-engine/src/weather/weather-volume.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-volume.spec.ts) (6 casos)
- **Código:** [packages/scene-engine/src/runtime/weather-layer.ts:107](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/weather-layer.ts#L107) · [packages/scene-engine/src/weather/weather-model.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-model.ts) · [packages/scene-engine/src/weather/weather-volume.ts:9](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-volume.ts#L9) · [packages/scene-engine/src/runtime/three-scene-runtime.ts:1086](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/three-scene-runtime.ts#L1086)

### MIG063

**Superfícies do clima: neve que acumula, poças com reflexo e respingos**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (13 casos em 1 spec vitest + 1 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Fazer neve acumular em superfícies voltadas para cima e formar poças que refletem a pintura e fazem anéis quando chove.
- **Gatilho (UI/atalho/CLI):** Interruptores **Poças**, **Neve acumulando**, **Começa coberta**; camadas com `surface` no bundle.
- **Caminho handler → estado → regra/runtime:** `surface` map (R=voltado para cima, G=chão, B=linha onde o chão encontra o objeto) + intrínsecos + plano do chão → `weather-surface-material`; reflexo = pintura espelhada em torno de B; ruído projetado em perspectiva.
- **Propriedades, unidades, modelo de dados:** `node.surface {assetRef, intrinsics, floor{normal,offset}|null}`.
- **Dependências:** MIG062 (Renderização do clima em planos (chuva, neve, relâmpago, grade de cor)), MIG060 (Ferramentas de mapas (normais, relevo, superfície))
- **Limitações:** Plano sem mapa não recebe neve (a UI avisa quais planos); só o chão tem fallback (linha reta).
- **Testes (existentes, não executados):** [packages/scene-engine/src/weather/weather-surface.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-surface.spec.ts) (13 casos); [packages/scene-engine/scripts/scene-studio-weather.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-weather.spec.mjs) (script de navegador/Chrome)
- **Código:** [packages/scene-engine/src/weather/weather-surface.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-surface.ts) · [packages/scene-engine/src/schema/render-bundle.schema.ts:65](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/schema/render-bundle.schema.ts#L65) · [apps/scene-studio/src/inspector/describeScene.ts:293](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/inspector/describeScene.ts#L293)

### MIG064

**Linha do chão do clima (para plano sem mapa)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (6 casos em 1 spec vitest)
- **O usuário faz:** Ajustar onde começa o chão quando o plano não tem surface map.
- **Gatilho (UI/atalho/CLI):** Slider **Linha do chão** (aparece quando o plano do chão não tem mapa).
- **Caminho handler → estado → regra/runtime:** `resolveGround` → `weatherGround {nodeId, top 0,2–0,95}` → `setSceneGround`; salvo em `/bundle-ground`.
- **Propriedades, unidades, modelo de dados:** `top` = fração da imagem.
- **Dependências:** MIG063 (Superfícies do clima: neve que acumula, poças com reflexo e respingos)
- **Limitações:** Aproximação: em corredor o chão é trapézio e a linha reta põe poças ao pé das paredes.
- **Testes (existentes, não executados):** [packages/scene-engine/src/weather/weather-ground.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-ground.spec.ts) (6 casos)
- **Código:** [packages/scene-engine/src/weather/weather-ground.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-ground.ts) · [packages/scene-engine/src/properties/property-paths.ts:77](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/property-paths.ts#L77)

### MIG065

**Keys de intensidade do clima**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (7 casos em 1 spec vitest)
- **O usuário faz:** Fazer a chuva/neve crescer ou parar ao longo da edição.
- **Gatilho (UI/atalho/CLI):** Inspector de clima: interruptor de animar, adicionar/remover chave.
- **Caminho handler → estado → regra/runtime:** `weatherLevelAt(keys, frame)` com `resolveKeyframes`; salvo em `/bundle-weather-keys`; ignorado sem clima.
- **Propriedades, unidades, modelo de dados:** Keyframe<number> 0–1,5.
- **Dependências:** MIG047 (Keyframes de valores (resolver, regras, adicionar/remover no playhead)), MIG061 (Gramática do clima (palavras, intensidade, erro de digitação))
- **Limitações:** Mover chaves arrastando só no piloto.
- **Testes (existentes, não executados):** [packages/scene-engine/src/weather/weather-keys.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-keys.spec.ts) (7 casos)
- **Código:** [packages/scene-engine/src/weather/weather-keys.ts:40](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/weather-keys.ts#L40)

### MIG066

**Clima em cenas de splat (colisão por profundidade, respingos, neve, poças)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (26 casos em 2 spec vitest)
- **O usuário faz:** Chuva que para no chão, neve que acumula e poças com reflexo numa cena de splat.
- **Gatilho (UI/atalho/CLI):** `scene.weather` numa cena com splat e `sceneDepth` no nó.
- **Caminho handler → estado → regra/runtime:** `splat-weather-sim.ts` (710 linhas): grade de profundidade visível (f32, NaN sem superfície) vista da câmera de origem; função pura de (tempo, câmera); `SplatWeatherLayer` desenha como splats.
- **Propriedades, unidades, modelo de dados:** `sceneDepth {depthAssetRef, imageAssetRef, cols, rows, binPx, focalPx, pivotDepth}`.
- **Dependências:** MIG062 (Renderização do clima em planos (chuva, neve, relâmpago, grade de cor)), MIG014 (Camadas de splat Gaussiano (Spark 2.4, `.spz`))
- **Limitações:** Portado de um laboratório apagado do disco; sem UI própria além do painel de clima.
- **Testes (existentes, não executados):** [packages/scene-engine/src/weather/splat/splat-weather-sim.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/splat/splat-weather-sim.spec.ts) (22 casos); [packages/scene-engine/src/weather/splat/scene-depth.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/splat/scene-depth.spec.ts) (4 casos)
- **Código:** [packages/scene-engine/src/weather/splat/splat-weather-sim.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/splat/splat-weather-sim.ts) · [packages/scene-engine/src/weather/splat/scene-depth.ts:6](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/weather/splat/scene-depth.ts#L6) · [packages/scene-engine/src/runtime/splat-weather-layer.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/splat-weather-layer.ts)

## H. Efeitos de pós-produção

### MIG067

**Looks de pós-produção (tinta, sonho, filme) com catálogo**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (47 casos em 5 spec vitest + 1 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Aplicar à cena inteira um look pictórico (Kuwahara + contornos), brilho/aberração/borda suave ou grade filme com grão; cada um de 0 a 1.
- **Gatilho (UI/atalho/CLI):** Camada **Efeitos** → sliders por look.
- **Caminho handler → estado → regra/runtime:** `SCENE_EFFECTS` (ink, dream, film; ordem = ordem do passe) → schema estrito, paths `scene/<s>/effects/<kind>`, `PostEffects` monta o shader (GLSL) por catálogo e aplica só se algum amount > 0; `scene effects` imprime o catálogo.
- **Propriedades, unidades, modelo de dados:** amount 0–1 por look; custo por pixel documentado (25–169 amostras para tinta).
- **Dependências:** MIG013 (Camadas de plano de imagem (PNG com alpha, fila de pintura única)), MIG046 (Sistema de property paths (listar, ler, escrever, erros tipados))
- **Limitações:** Tinta cintila onde a imagem se move; raios em px para 1280×720; filme escurece cenas noturnas.
- **API / CLI:** CLI `scene effects`; props `scene/<s>/effects/<kind>`.
- **Testes (existentes, não executados):** [packages/scene-engine/src/effects/catalog.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/effects/catalog.spec.ts) (5 casos); [packages/scene-engine/src/effects/scene-effects.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/effects/scene-effects.spec.ts) (7 casos); [packages/scene-engine/src/runtime/post-effects/assemble.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/post-effects/assemble.spec.ts) (20 casos); [packages/scene-engine/src/properties/effects-properties.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/effects-properties.spec.ts) (14 casos); [packages/scene-engine/scripts/scene-studio-effects.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-effects.spec.mjs) (script de navegador/Chrome); [packages/scene-engine/src/node/render-effects.e2e.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/render-effects.e2e.spec.ts) (1 casos)
- **Código:** [packages/scene-engine/src/effects/catalog.ts:26](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/effects/catalog.ts#L26) · [packages/scene-engine/src/effects/scene-effects.ts:23](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/effects/scene-effects.ts#L23) · [packages/scene-engine/src/runtime/post-effects/post-effects.ts:43](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/post-effects/post-effects.ts#L43) · [packages/scene-engine/src/runtime/post-effects/assemble.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/post-effects/assemble.ts) · [apps/scene-studio/src/inspector/describeEffects.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/inspector/describeEffects.ts)

### MIG068

**Inspector de efeitos (sliders por look)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (4 casos em 1 spec vitest)
- **O usuário faz:** Ajustar a força de cada look; ver a dica do catálogo.
- **Gatilho (UI/atalho/CLI):** Selecionar **Efeitos**.
- **Caminho handler → estado → regra/runtime:** `describeEffects` gera sliders de 0 a 1 por `kind` → `setProperty(effectPath)`; salvo em `/bundle-effects`.
- **Propriedades, unidades, modelo de dados:** —
- **Dependências:** MIG067 (Looks de pós-produção (tinta, sonho, filme) com catálogo)
- **Limitações:** Só os 3 looks.
- **Testes (existentes, não executados):** [apps/scene-studio/src/inspector/describeEffects.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/inspector/describeEffects.spec.ts) (4 casos)
- **Código:** [apps/scene-studio/src/inspector/describeEffects.ts:30](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/inspector/describeEffects.ts#L30)

## I. Rig, paths e follow (mão que escreve)

### MIG069

**Rig: ossos sobre uma camada, limites, punho e pino**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (32 casos em 3 spec vitest)
- **O usuário faz:** Desenhar uma cadeia de ossos sobre uma camada de imagem para deformar ou carregar partes da arte.
- **Gatilho (UI/atalho/CLI):** Menu **Rig** (criar, adicionar/remover osso, fixar/soltar), overlay no viewport, árvore de rig.
- **Caminho handler → estado → regra/runtime:** `rigSchema` {id,nodeId,bones[≥1],constraints[],follows?}; `createRig` (slice) → store → `/bundle-rig`; poses não são armazenadas (são assadas, ver bake).
- **Propriedades, unidades, modelo de dados:** Osso {id,parentId,rest{position?,angleDeg −360..360,length>0},limits{minDeg,maxDeg ±180},role?:wrist,pin?{nodeId,point}}; unidades px da imagem da camada (x→, y↓); filho começa onde o pai termina.
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** Convenção de ângulo: 0 ao longo de +x, 90 para baixo. Sem importação de rigs externos.
- **Testes (existentes, não executados):** [packages/scene-engine/src/rig/rig-commands.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/rig-commands.spec.ts) (11 casos); [packages/scene-engine/src/rig/rig-geometry.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/rig-geometry.spec.ts) (3 casos); [apps/scene-studio/src/engine/rigStore.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/rigStore.spec.ts) (18 casos)
- **Código:** [packages/scene-engine/src/rig/rig-schema.ts:72](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/rig-schema.ts#L72) · [packages/scene-engine/src/rig/rig-schema.ts:17](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/rig-schema.ts#L17) · [packages/scene-engine/src/rig/rig-geometry.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/rig-geometry.ts) · [apps/scene-studio/src/engine/rigSlice.ts:197](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/rigSlice.ts#L197)

### MIG070

**IK (CCDIKSolver) e bake determinístico da pose**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (22 casos em 2 spec vitest)
- **O usuário faz:** Resolver a cadeia de ossos para que a ponta alcance o alvo por frame e guardar o resultado como trilha de poses lida por seek e export.
- **Gatilho (UI/atalho/CLI):** Ação **Bake** no menu Rig; automático ao abrir a cena (`ensureRigTracks`).
- **Caminho handler → estado → regra/runtime:** `bakeRig(request)` usa `CCDIKSolver` do Three.js (`three/addons`) por frame, modo `reset` (cada frame parte do repouso) ou `warm` (só no bake sequencial); hash com `BAKE_VERSION = 1`; a trilha assada é o que seek/export leem (nunca resolvem).
- **Propriedades, unidades, modelo de dados:** Constraint {effectorBoneId,chainLength 1–16,iterations 1–500,mode,target.keys[{frame,x,y,easing}]}; `RigTrack` por frame.
- **Dependências:** MIG069 (Rig: ossos sobre uma camada, limites, punho e pino)
- **Limitações:** Depende da implementação do CCDIK do Three.js 0.187: reimplementar em Rust muda o resultado numérico. Bake em worker no Studio.
- **Testes (existentes, não executados):** [packages/scene-engine/src/rig/bake-rig.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/bake-rig.spec.ts) (17 casos); [apps/scene-studio/src/engine/rigBaker.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/rigBaker.spec.ts) (5 casos)
- **Código:** [packages/scene-engine/src/rig/bake-rig.ts:104](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/bake-rig.ts#L104) · [packages/scene-engine/src/rig/bake-rig.ts:15](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/bake-rig.ts#L15) · [apps/scene-studio/src/engine/rigBake.worker.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/rigBake.worker.ts) · [apps/scene-studio/src/engine/rigBakeClient.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/rigBakeClient.ts) · [packages/scene-engine/src/runtime/three-scene-runtime.ts:591](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/three-scene-runtime.ts#L591)

### MIG071

**Binding de camadas ao rig (rígido ou esfolado) e RigLayer**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (3 casos em 1 spec vitest)
- **O usuário faz:** Fazer uma camada de imagem seguir um osso (mão) ou deformar-se com todos (manga).
- **Gatilho (UI/atalho/CLI):** Seção de binding no inspector da camada (imagePlane).
- **Caminho handler → estado → regra/runtime:** `rigBind {rigId, mode rigid|skinned, boneId?, blendPx?}` → `RigLayer` constrói malha com pesos de até 4 ossos (`rig-skin.ts`), `blendPx` padrão 150.
- **Propriedades, unidades, modelo de dados:** Pesos por vértice (soma 1).
- **Dependências:** MIG070 (IK (CCDIKSolver) e bake determinístico da pose)
- **Limitações:** WebGL sem teste unitário; pesos por distância, não pintados.
- **Testes (existentes, não executados):** [packages/scene-engine/src/rig/rig-geometry.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/rig-geometry.spec.ts) (3 casos)
- **Código:** [packages/scene-engine/src/runtime/rig-layer.ts:45](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/rig-layer.ts#L45) · [packages/scene-engine/src/rig/rig-skin.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/rig-skin.ts) · [packages/scene-engine/src/rig/rig-schema.ts:84](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/rig-schema.ts#L84) · [apps/scene-studio/src/inspector/describeRig.ts:540](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/inspector/describeRig.ts#L540)

### MIG072

**Comandos e propriedades de rig (11 ops + caminhos `rig/<id>/…`)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (38 casos em 3 spec vitest)
- **O usuário faz:** Editar rigs sem UI: osso, pino, restrição IK, chaves de alvo e follows.
- **Gatilho (UI/atalho/CLI):** CLI `scene edit '[{"rig":{…},"rigId":…}]'`; props `rig/<id>/bone/<b>/{length,rest/angle,rest/position,rest/tip,limit/min,limit/max,role,pin/point}`, `constraint/<c>/{iterations,chain,mode}`, `target/<f>/{position,easing}`, `bind/blendPx`.
- **Caminho handler → estado → regra/runtime:** `applyRigCommand` (11 ops: addBone, removeBone, setPin, setTargetKey, removeTargetKey, addConstraint, removeConstraint, addFollow, removeFollow, setFollowKey, removeFollowKey) + `rigPropertiesOf`.
- **Propriedades, unidades, modelo de dados:** Erros tipados (RIG_PIN_NEEDS_ROOT, RIG_KEY_UNKNOWN, RIG_CHAIN_UNAVAILABLE, RIG_FOLLOW_UNKNOWN, RIG_FOLLOW_LAST_KEY…).
- **Dependências:** MIG069 (Rig: ossos sobre uma camada, limites, punho e pino), MIG046 (Sistema de property paths (listar, ler, escrever, erros tipados))
- **Limitações:** As 11 ops estão em `RIG_COMMAND_OPS`; op desconhecida é recusada com a lista.
- **API / CLI:** Lib `applyRigCommand`, `rigCommandSchema`, `RIG_COMMAND_OPS`.
- **Testes (existentes, não executados):** [packages/scene-engine/src/rig/rig-commands.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/rig-commands.spec.ts) (11 casos); [packages/scene-engine/src/rig/follow-commands.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/follow-commands.spec.ts) (6 casos); [packages/scene-engine/src/properties/properties.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/properties.spec.ts) (21 casos)
- **Código:** [packages/scene-engine/src/rig/rig-commands.ts:527](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/rig-commands.ts#L527) · [packages/scene-engine/src/rig/rig-commands.ts:65](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/rig-commands.ts#L65) · [packages/scene-engine/src/properties/rig-properties.ts:485](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/rig-properties.ts#L485)

### MIG073

**Validação e diagnósticos do rig**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (13 casos em 1 spec vitest)
- **O usuário faz:** Ver problemas (ponta longe do alvo >8 px, junta presa no limite, punho girando demais) e saltar ao frame do problema.
- **Gatilho (UI/atalho/CLI):** Menu Rig → **Validar**; painel de validação (clique busca o frame).
- **Caminho handler → estado → regra/runtime:** `rigDefinitionIssues` + `rigBundleIssues` + `rigDiagnostics` sobre a trilha assada; erro bloqueia o export do rig, aviso só aparece.
- **Propriedades, unidades, modelo de dados:** `TIP_ERROR_LIMIT_PX = 8`.
- **Dependências:** MIG070 (IK (CCDIKSolver) e bake determinístico da pose)
- **Limitações:** Regras vêm da skill 'scene-element-recipes' (documento externo).
- **Testes (existentes, não executados):** [packages/scene-engine/src/rig/rig-validation.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/rig-validation.spec.ts) (13 casos)
- **Código:** [packages/scene-engine/src/rig/rig-validation.ts:167](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/rig-validation.ts#L167) · [packages/scene-engine/src/rig/rig-diagnostics.ts:9](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/rig-diagnostics.ts#L9) · [apps/scene-studio/src/components/RigValidationPanel.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/RigValidationPanel.tsx)

### MIG074

**UI de rig (menu, árvore, overlay de ossos, faixa de alvos, inspector, vista pose/repouso)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (7 casos em 1 spec vitest + 1 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Criar rig, arrastar ossos/alvos no viewport (mouse ou setas), chavear alvo, alternar vista repouso/pose, editar limites no inspector.
- **Gatilho (UI/atalho/CLI):** Menu **Rig**; `RigOverlay`; `RigTree`; faixa de rig; `describeRig`.
- **Caminho handler → estado → regra/runtime:** `useRigEditor` → `rigCommand`/`setValue` → store (rigSlice) → `/bundle-rig`; overlay usa `layerSpaceForRig` (px da imagem ↔ mundo).
- **Propriedades, unidades, modelo de dados:** 36 chaves i18n de inspector.
- **Dependências:** MIG072 (Comandos e propriedades de rig (11 ops + caminhos `rig/<id>/…`)), MIG073 (Validação e diagnósticos do rig)
- **Limitações:** Handles dependem das matrizes do último quadro.
- **Testes (existentes, não executados):** [apps/scene-studio/src/engine/rigView.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/rigView.spec.ts) (7 casos); [packages/scene-engine/scripts/scene-studio-rig.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-rig.spec.mjs) (script de navegador/Chrome)
- **Código:** [apps/scene-studio/src/components/RigMenu.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/RigMenu.tsx) · [apps/scene-studio/src/components/RigOverlay.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/RigOverlay.tsx) · [apps/scene-studio/src/components/RigTree.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/RigTree.tsx) · [apps/scene-studio/src/components/RigTrack.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/RigTrack.tsx) · [apps/scene-studio/src/inspector/describeRig.ts:497](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/inspector/describeRig.ts#L497) · [apps/scene-studio/src/hooks/useRigEditor.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/hooks/useRigEditor.ts)

### MIG075

**Paths sobre a imagem de uma camada (catmull-rom, linear, bezier, caneta levantada)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (28 casos em 3 spec vitest)
- **O usuário faz:** Desenhar a linha que a caneta deve seguir, em px da imagem; alternar interpolação, fechar e levantar a caneta entre linhas.
- **Gatilho (UI/atalho/CLI):** Menu **Follow** (criar/gerar caminho, adicionar/remover ponto, apagar); overlay; propriedades `scene/<s>/path/<p>/…`.
- **Caminho handler → estado → regra/runtime:** `pathSchema` {id,nodeId,space:'plane',points[≥2]{x,y,penUp?,handleIn?,handleOut?},interpolation,closed}; `path-geometry` amostra a linha suave (nunca armazenada).
- **Propriedades, unidades, modelo de dados:** `closed` não combina com caneta levantada.
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** Só espaço 'plane'.
- **Testes (existentes, não executados):** [packages/scene-engine/src/path/path-geometry.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/path/path-geometry.spec.ts) (9 casos); [packages/scene-engine/src/path/path-commands.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/path/path-commands.spec.ts) (5 casos); [apps/scene-studio/src/engine/followStore.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/followStore.spec.ts) (14 casos)
- **Código:** [packages/scene-engine/src/path/path-schema.ts:28](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/path/path-schema.ts#L28) · [packages/scene-engine/src/path/path-geometry.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/path/path-geometry.ts) · [packages/scene-engine/src/path/path-commands.ts:124](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/path/path-commands.ts#L124) · [packages/scene-engine/src/properties/path-properties.ts:180](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/properties/path-properties.ts#L180)

### MIG076

**Gerar o primeiro path de um texto a partir dos pixels da camada**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (6 casos em 1 spec vitest)
- **O usuário faz:** Propor automaticamente a linha de escrita (linhas de texto, caneta que volta ao início de cada linha).
- **Gatilho (UI/atalho/CLI):** Menu Follow → **Gerar caminho**.
- **Caminho handler → estado → regra/runtime:** `generatePath(sceneId,nodeId)` lê a camada (`readLayerInk`) → `path-ink.ts` acha direção das linhas pelo histograma, vales = intervalos entre linhas, propõe pontos (padrão 48 px; linhas ≥26 px; marcas <1500 px grudam).
- **Propriedades, unidades, modelo de dados:** Função pura da máscara.
- **Dependências:** MIG075 (Paths sobre a imagem de uma camada (catmull-rom, linear, bezier, caneta levantada))
- **Limitações:** Heurística: o usuário corrige os handles.
- **Testes (existentes, não executados):** [packages/scene-engine/src/path/path-ink.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/path/path-ink.spec.ts) (6 casos)
- **Código:** [packages/scene-engine/src/path/path-ink.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/path/path-ink.ts) · [apps/scene-studio/src/engine/pathSlice.ts:139](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/pathSlice.ts#L139) · [apps/scene-studio/src/engine/layerInk.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/layerInk.ts)

### MIG077

**Follow: a ponta de um osso caminha pelo path (chaves de progresso, alcance)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (23 casos em 2 spec vitest + 1 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Fazer a caneta (ponta do osso) percorrer o path de 0 a 1 com easing; escolher se ponto inalcançável bloqueia (`report`) ou prende ao mais próximo (`clamp`).
- **Gatilho (UI/atalho/CLI):** Menu Follow → **Seguir caminho**; inspector de follow.
- **Caminho handler → estado → regra/runtime:** `followSchema` {id,boneId,pathId,progress[{frame,value 0–1,easing}],offset?,reach} → `addFollow` cria a restrição IK se faltar; `followProgressAt` interpola; `follow-validation` verifica.
- **Propriedades, unidades, modelo de dados:** O path é a fonte da verdade: nada escala o path ou afrouxa junta.
- **Dependências:** MIG075 (Paths sobre a imagem de uma camada (catmull-rom, linear, bezier, caneta levantada)), MIG072 (Comandos e propriedades de rig (11 ops + caminhos `rig/<id>/…`))
- **Limitações:** Mover/escalar chaves de progresso arrastando só no piloto.
- **Testes (existentes, não executados):** [packages/scene-engine/src/rig/follow.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/follow.spec.ts) (17 casos); [packages/scene-engine/src/rig/follow-commands.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/follow-commands.spec.ts) (6 casos); [packages/scene-engine/scripts/scene-studio-follow.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-follow.spec.mjs) (script de navegador/Chrome)
- **Código:** [packages/scene-engine/src/rig/follow-schema.ts:21](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/follow-schema.ts#L21) · [packages/scene-engine/src/rig/follow.ts:20](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/follow.ts#L20) · [packages/scene-engine/src/rig/follow-validation.ts:240](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/follow-validation.ts#L240) · [apps/scene-studio/src/inspector/describeFollow.ts:343](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/inspector/describeFollow.ts#L343)

### MIG078

**Reveal: a tinta aparece quando a caneta já passou**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (9 casos em 1 spec vitest)
- **O usuário faz:** Fazer o texto escrito surgir conforme a caneta caminha.
- **Gatilho (UI/atalho/CLI):** Menu Follow → **Revelar tinta** / **Parar de revelar**.
- **Caminho handler → estado → regra/runtime:** `revealSchema {rigId, followId}` na camada; `path-reveal` dá a cada pixel de tinta o comprimento de arco do ponto do path mais próximo; um frame é um intervalo ordenado; `RevealLayer` cópia a textura escondendo o resto.
- **Propriedades, unidades, modelo de dados:** `INK_ALPHA_MIN`; grade de ~N px por célula.
- **Dependências:** MIG077 (Follow: a ponta de um osso caminha pelo path (chaves de progresso, alcance))
- **Limitações:** A camada deve ser a mesma do path do follow.
- **Testes (existentes, não executados):** [packages/scene-engine/src/path/path-reveal.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/path/path-reveal.spec.ts) (9 casos)
- **Código:** [packages/scene-engine/src/path/path-reveal.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/path/path-reveal.ts) · [packages/scene-engine/src/runtime/reveal-layer.ts:60](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/runtime/reveal-layer.ts#L60) · [packages/scene-engine/src/rig/follow-schema.ts:41](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/rig/follow-schema.ts#L41)

### MIG079

**UI de follow e path (menu, árvore, overlay de pontos, faixa de progresso)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (14 casos em 1 spec vitest)
- **O usuário faz:** Arrastar pontos do path e alças bezier, ver chaves de progresso na timeline, escolher follow na árvore.
- **Gatilho (UI/atalho/CLI):** `FollowMenu`, `FollowTree`, `FollowOverlay`, `FollowTrack`, `describeFollow/describePath`.
- **Caminho handler → estado → regra/runtime:** `useFollowEditor` → comandos de path/rig → store.
- **Propriedades, unidades, modelo de dados:** 33 chaves i18n do inspector.
- **Dependências:** MIG077 (Follow: a ponta de um osso caminha pelo path (chaves de progresso, alcance)), MIG076 (Gerar o primeiro path de um texto a partir dos pixels da camada)
- **Limitações:** —
- **Testes (existentes, não executados):** [apps/scene-studio/src/engine/followStore.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/engine/followStore.spec.ts) (14 casos)
- **Código:** [apps/scene-studio/src/components/FollowMenu.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/FollowMenu.tsx) · [apps/scene-studio/src/components/FollowOverlay.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/FollowOverlay.tsx) · [apps/scene-studio/src/components/FollowTree.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/FollowTree.tsx) · [apps/scene-studio/src/components/FollowTrack.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/FollowTrack.tsx) · [apps/scene-studio/src/hooks/useFollowEditor.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/hooks/useFollowEditor.ts)

### MIG080

**Ferramentas da mão que escreve (bundle, captura, relatório, preparação de assets)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Montar o bundle da mão, abrir o editor nele, capturar telas e validar camadas de mão/tinta.
- **Gatilho (UI/atalho/CLI):** `scripts/elements/rig/*` (mjs + python).
- **Caminho handler → estado → regra/runtime:** Scripts Node/Python; `capture-hands-editor.mjs` usa `__sceneStudio`.
- **Propriedades, unidades, modelo de dados:** —
- **Dependências:** MIG069 (Rig: ossos sobre uma camada, limites, punho e pino)
- **Limitações:** Específico de uma produção; dependem de arte local.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [packages/scene-engine/scripts/elements/rig/build-hands-bundle.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/elements/rig/build-hands-bundle.mjs) · [packages/scene-engine/scripts/elements/rig/rig.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/elements/rig/rig.mjs) · [packages/scene-engine/scripts/elements/rig/capture-hands-editor.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/elements/rig/capture-hands-editor.mjs) · [packages/scene-engine/scripts/elements/rig/prep_rig_assets.py](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/elements/rig/prep_rig_assets.py) · [packages/scene-engine/scripts/elements/validate_hand_layer.py](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/elements/validate_hand_layer.py)

## J. Narração, legendas e mix de áudio

### MIG081

**Modelo de narração (faixas, linhas, palavras, entrega, fromMix, legenda)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (28 casos em 2 spec vitest)
- **O usuário faz:** Colocar linhas de texto narradas na linha do tempo, cada uma com um take de TTS (ou texto sobre uma voz que já está no mix), e definir o estilo da legenda.
- **Gatilho (UI/atalho/CLI):** Seção **Áudio/Texto** do painel de camadas; inspector de faixa/linha.
- **Caminho handler → estado → regra/runtime:** `narrationSchema {tracks[{id,label,clips[]}], caption, mixMuted?}`; `explainNarration` valida (sobreposição, passa do fim, ids…) com códigos.
- **Propriedades, unidades, modelo de dados:** Clip {id,text 1–600,startSeconds,durationSeconds ≤120,voice?,style?,audio{relativePath,sha256}?,words[]?,delivery{exaggeration .25–2,cfgWeight 0–1,seed int}?,fromMix{start,dur}?,hidden?,muted?}; caption {backdropOpacity 0–1, position bottom|top, fontScale .02–.1, holdSeconds 0–3}. Tempo em segundos de edição.
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** Linhas na mesma faixa não podem se sobrepor (`CLIP_OVERLAP`).
- **Testes (existentes, não executados):** [packages/scene-engine/src/narration/narration.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/narration.spec.ts) (25 casos); [packages/scene-engine/src/narration/take-rules.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/take-rules.spec.ts) (3 casos)
- **Código:** [packages/scene-engine/src/narration/narration-schema.ts:83](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/narration-schema.ts#L83) · [packages/scene-engine/src/narration/narration-schema.ts:21](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/narration-schema.ts#L21) · [packages/scene-engine/src/narration/narration.ts:625](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/narration.ts#L625)

### MIG082

**Comandos de narração (10 ops)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (43 casos em 2 spec vitest)
- **O usuário faz:** Adicionar/atualizar/remover faixas e linhas, mover linha entre faixas, definir take (`setClipAudio`), silenciar o mix e ajustar a legenda.
- **Gatilho (UI/atalho/CLI):** CLI `scene narration show|apply|import-srt`; UI de linhas.
- **Caminho handler → estado → regra/runtime:** `applyNarrationCommands` → ops addTrack, updateTrack, removeTrack, addClip, updateClip, setClipAudio, moveClip, removeClip, setMix, setCaption; duração estimada do texto quando não há take.
- **Propriedades, unidades, modelo de dados:** Erros UNKNOWN_CLIP, CLIP_OVERLAP, CLIP_PAST_END…
- **Dependências:** MIG081 (Modelo de narração (faixas, linhas, palavras, entrega, fromMix, legenda))
- **Limitações:** —
- **API / CLI:** CLI `scene narration`; lib `applyNarrationCommands`, `narrationCommandSchema`.
- **Testes (existentes, não executados):** [packages/scene-engine/src/narration/narration.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/narration.spec.ts) (25 casos); [packages/scene-engine/src/node/scene-cli.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/scene-cli.spec.ts) (18 casos)
- **Código:** [packages/scene-engine/src/narration/narration.ts:216](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/narration.ts#L216) · [packages/scene-engine/src/narration/narration.ts:683](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/narration.ts#L683)

### MIG083

**Plano de mix: o que soa, onde a voz do mix cala porque há take**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (7 casos em 1 spec vitest)
- **O usuário faz:** Garantir que a voz nunca soe duas vezes: o mix fica mudo onde uma linha toca seu próprio take, está silenciada ou oculta.
- **Gatilho (UI/atalho/CLI):** Automático no preview (agendador) e no export (grafo ffmpeg).
- **Caminho handler → estado → regra/runtime:** `audioMixPlan(bundle)` → spans do mix + takes tocáveis; `mixSilences`, `playableTakes`, `mixGainAt`; o Studio agenda e o export constrói o mesmo plano.
- **Propriedades, unidades, modelo de dados:** `AudioMixPlan {durationSeconds, mix{relativePath, sourceStartSeconds, silences[{startSeconds,endSeconds}]}|null, takes[{clipId,relativePath,startSeconds,durationSeconds}]}`.
- **Dependências:** MIG081 (Modelo de narração (faixas, linhas, palavras, entrega, fromMix, legenda))
- **Limitações:** —
- **Testes (existentes, não executados):** [packages/scene-engine/src/narration/mix-plan.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/mix-plan.spec.ts) (7 casos)
- **Código:** [packages/scene-engine/src/narration/mix-plan.ts:105](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/mix-plan.ts#L105) · [packages/scene-engine/src/narration/narration.ts:603](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/narration.ts#L603)

### MIG084

**Gerar takes de TTS (cliente OpenAI-compatível) e gravar no bundle**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (12 casos em 2 spec vitest)
- **O usuário faz:** Gerar a voz de uma linha (ou das faltantes) por um servidor TTS local e anexá-la com tempos de palavra.
- **Gatilho (UI/atalho/CLI):** Botão **Gerar take** no inspector da linha; CLI `scene takes <bundle> (--clips a,b|--missing) [--tts url] [--voice v] --write`; `GET /narration-voices`.
- **Caminho handler → estado → regra/runtime:** `makeTakes` → `createTtsClient` (`POST {base}/audio/speech`, WAV PCM; `GET {base}/voices`) → grava WAV na pasta do bundle → `setClipAudio`; `X-Timings` fornece palavras.
- **Propriedades, unidades, modelo de dados:** Env `SCENE_STUDIO_TTS_URL|MODEL|VOICE`; timeouts 180 s (síntese) e 5 s (vozes); erro `TTS_NOT_CONFIGURED`.
- **Dependências:** MIG082 (Comandos de narração (10 ops))
- **Limitações:** Depende de serviço HTTP externo (local, ex.: Chatterbox/Kokoro); sem URL configurada a feature recusa. Rede é o único ponto de integração do editor.
- **API / CLI:** CLI `scene takes`; HTTP `/bundle-narration-take`.
- **Testes (existentes, não executados):** [packages/scene-engine/src/node/tts.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/tts.spec.ts) (5 casos); [packages/scene-engine/src/node/takes.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/takes.spec.ts) (7 casos)
- **Código:** [packages/scene-engine/src/node/tts.ts:157](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/tts.ts#L157) · [packages/scene-engine/src/node/tts.ts:48](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/tts.ts#L48) · [packages/scene-engine/src/node/takes.ts:136](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/takes.ts#L136) · [apps/scene-studio/src/hooks/useNarrationVoices.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/hooks/useNarrationVoices.ts)

### MIG085

**Cortar o take da própria voz do mix**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (13 casos em 2 spec vitest)
- **O usuário faz:** Transformar uma linha que está no mix em take próprio, recortando o WAV e silenciando o mix naquele trecho.
- **Gatilho (UI/atalho/CLI):** Ação no inspector da linha; CLI `scene cut-from-mix --clips|--all --write`.
- **Caminho handler → estado → regra/runtime:** `cutTakesFromMix` fatia o WAV (`sliceWav`), grava o take, `setClipAudio` com `fromMix`.
- **Propriedades, unidades, modelo de dados:** —
- **Dependências:** MIG083 (Plano de mix: o que soa, onde a voz do mix cala porque há take)
- **Limitações:** Só funciona sobre WAV PCM.
- **Testes (existentes, não executados):** [packages/scene-engine/src/node/takes.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/takes.spec.ts) (7 casos); [packages/scene-engine/src/node/wav.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/wav.spec.ts) (6 casos)
- **Código:** [packages/scene-engine/src/node/takes.ts:218](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/takes.ts#L218) · [packages/scene-engine/src/node/wav.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/wav.ts)

### MIG086

**Importar legendas SRT como linhas de narração**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (6 casos em 1 spec vitest)
- **O usuário faz:** Converter cues de um .srt em linhas, com tempos relativos à janela de áudio do bundle.
- **Gatilho (UI/atalho/CLI):** CLI `scene narration <bundle> import-srt <file.srt> [--track id] [--write]`.
- **Caminho handler → estado → regra/runtime:** `parseSrt` + `srtNarrationCommands(bundle, cues, track)`; cues fora da edição são descartados; sem nenhum → `SRT_NO_CUES`.
- **Propriedades, unidades, modelo de dados:** —
- **Dependências:** MIG082 (Comandos de narração (10 ops))
- **Limitações:** Tempos do SRT são segundos do áudio de origem, não da edição.
- **Testes (existentes, não executados):** [packages/scene-engine/src/narration/srt.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/srt.spec.ts) (6 casos)
- **Código:** [packages/scene-engine/src/narration/srt.ts:24](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/srt.ts#L24) · [packages/scene-engine/src/narration/srt.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/srt.ts)

### MIG087

**Legendas: revelação por palavra, layout, overlay no Studio e pintura no export**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (8 casos em 2 spec vitest)
- **O usuário faz:** Mostrar a legenda em caixa cinza translúcida com revelação no ritmo da fala (ou do texto) e gravá-la nos quadros exportados.
- **Gatilho (UI/atalho/CLI):** Overlay no viewport; opção `--no-captions` no render.
- **Caminho handler → estado → regra/runtime:** `captionsAt(narration, seconds, outline)` → `captionLayout`/`placeCaptionBoxes` (geometria do motor) → Studio: HTML/CSS (`CaptionOverlay`); export: `createCaptionPainter` (sharp + Pango) pinta no PNG antes do ffmpeg.
- **Propriedades, unidades, modelo de dados:** Letras ainda por vir ficam invisíveis mas ocupam lugar.
- **Dependências:** MIG081 (Modelo de narração (faixas, linhas, palavras, entrega, fromMix, legenda))
- **Limitações:** Fonte via fontconfig (`TESSEL_SCENE_ENGINE_CAPTION_FONT`); pixel da legenda no export ≠ pixel do overlay do navegador (duas implementações de pintura).
- **Testes (existentes, não executados):** [packages/scene-engine/src/narration/caption-layout.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/caption-layout.spec.ts) (5 casos); [packages/scene-engine/src/node/captions.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/captions.spec.ts) (3 casos)
- **Código:** [packages/scene-engine/src/narration/narration.ts:160](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/narration.ts#L160) · [packages/scene-engine/src/narration/caption-layout.ts:43](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/narration/caption-layout.ts#L43) · [packages/scene-engine/src/node/captions.ts:76](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/captions.ts#L76) · [apps/scene-studio/src/components/CaptionOverlay.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/CaptionOverlay.tsx)

### MIG088

**Reprodução dos takes de narração no Studio**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Ouvir cada linha no seu início durante o play, respeitando silenciar/ocultar e o volume do monitor.
- **Gatilho (UI/atalho/CLI):** Play.
- **Caminho handler → estado → regra/runtime:** `useNarrationPlayback` agenda os takes via Web Audio a partir de `audioMixPlan`; ganho do mix = `mixGainAt`.
- **Propriedades, unidades, modelo de dados:** —
- **Dependências:** MIG083 (Plano de mix: o que soa, onde a voz do mix cala porque há take), MIG026 (Porta de áudio do navegador (relógio de áudio, ganho do monitor, janela de edição))
- **Limitações:** Depende do relógio de áudio do navegador.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [apps/scene-studio/src/hooks/useNarrationPlayback.ts:49](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/hooks/useNarrationPlayback.ts#L49)

### MIG089

**UI de narração (faixas, linhas, inspector de linha com voz/estilo/entrega)**

- **Estado:** `UI` — Visível / UI funcional · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Adicionar faixas, criar linhas, escolher voz e estilo, ajustar exagero/ritmo/semente, gerar take, cortar do mix, mover entre faixas, ocultar/silenciar, remover.
- **Gatilho (UI/atalho/CLI):** `AudioSection`, `describeNarrationTrack/Line` (570 linhas), lista de vozes.
- **Caminho handler → estado → regra/runtime:** `useNarration` (`run`/`check`) → `/bundle-edit`.
- **Propriedades, unidades, modelo de dados:** 37 chaves i18n.
- **Dependências:** MIG082 (Comandos de narração (10 ops)), MIG084 (Gerar takes de TTS (cliente OpenAI-compatível) e gravar no bundle)
- **Limitações:** —
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [apps/scene-studio/src/inspector/describeNarration.ts:113](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/inspector/describeNarration.ts#L113) · [apps/scene-studio/src/hooks/useNarration.ts:58](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/hooks/useNarration.ts#L58) · [apps/scene-studio/src/components/layers/AudioSection.tsx](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio/src/components/layers/AudioSection.tsx)

## K. Render, export, CLI e APIs programáticas

### MIG090

**Render offline: Chrome + Three.js quadro a quadro → ffmpeg H.264**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (9 casos em 3 spec vitest)
- **O usuário faz:** Exportar o bundle como MP4 com áudio, legenda opcional, relatório JSON e quadros-amostra em PNG.
- **Gatilho (UI/atalho/CLI):** CLI `scene render <bundle> --out <dir> [--label n] [--no-captions]`; `pnpm pilot:render`.
- **Caminho handler → estado → regra/runtime:** `renderBundle` → preflight → `createViewerHarness` (servidor local + Chrome via playwright-core com o mesmo runtime do Studio) → para cada frame `renderViewerFrame` (RGBA) → opcional pintura de legenda (sharp) → `ffmpeg -f rawvideo -pix_fmt rgba … libx264 yuv420p` → mux AAC com o grafo de áudio → verifica e escreve o relatório.
- **Propriedades, unidades, modelo de dados:** Saídas em `--out`: `<label>.mp4` (só vídeo), `<label>.m4a` (áudio AAC), `<label>-with-audio.mp4` (final), `<label>-report.json` e `frames/` (amostras padrão: 23 frames do piloto + o último). `label` padrão `render`, só `[a-z0-9-]` (`INVALID_LABEL`). Chrome padrão: macOS Google Chrome (`TESSEL_SCENE_ENGINE_CHROME`).
- **Dependências:** MIG013 (Camadas de plano de imagem (PNG com alpha, fila de pintura única)), MIG083 (Plano de mix: o que soa, onde a voz do mix cala porque há take)
- **Limitações:** Sem cancelamento, sem estimativa de disco, sem job/fila; `onProgress(frame,total)` apenas. Pixels dependem do ambiente (SwiftShader/GPU).
- **API / CLI:** CLI `scene render`; lib `renderBundle(path, {outDir,label,sampleFrames,captions,onProgress})`.
- **Testes (existentes, não executados):** [packages/scene-engine/src/node/render-effects.e2e.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/render-effects.e2e.spec.ts) (1 casos); [packages/scene-engine/src/node/narrated-scene.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/narrated-scene.spec.ts) (3 casos); [packages/scene-engine/src/node/audio-graph.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/audio-graph.spec.ts) (5 casos)
- **Código:** [packages/scene-engine/src/node/render.ts:298](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/render.ts#L298) · [packages/scene-engine/src/node/render.ts:148](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/render.ts#L148) · [packages/scene-engine/src/node/viewer.ts:85](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/viewer.ts#L85) · [packages/scene-engine/src/node/viewer.ts:263](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/viewer.ts#L263)

### MIG091

**Verificação do arquivo final (ffprobe) e contrato de cor**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (1 casos em 1 spec vitest)
- **O usuário faz:** Garantir que o MP4 tem a resolução, fps, duração e tags de cor do contrato e decodifica inteiro.
- **Gatilho (UI/atalho/CLI):** Final do `renderBundle`.
- **Caminho handler → estado → regra/runtime:** `probeMedia` e checagens: `VIDEO_CONTRACT_MISMATCH` (largura/altura/fps/duração ±1 ms), `COLOR_TAGS_INCORRECT` (esperado `bt709` matriz, `iec61966-2-1` transferência sRGB, primárias bt709, faixa `tv`); conversão RGB→YUV explícita BT709, faixa limitada.
- **Propriedades, unidades, modelo de dados:** H.264/yuv420p, +faststart.
- **Dependências:** MIG090 (Render offline: Chrome + Three.js quadro a quadro → ffmpeg H.264)
- **Limitações:** Transferência é sRGB, **não** a curva Rec.709 (documentado); frames comparados em RGBA sem perda, não no MP4.
- **Testes (existentes, não executados):** [packages/scene-engine/src/node/render-effects.e2e.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/render-effects.e2e.spec.ts) (1 casos)
- **Código:** [packages/scene-engine/src/node/render.ts:106](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/render.ts#L106) · [packages/scene-engine/src/node/render.ts:115](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/render.ts#L115) · [packages/scene-engine/src/node/render.ts:71](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/render.ts#L71)

### MIG092

**Grafo de áudio do export (mix + takes, silêncios, AAC)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (5 casos em 1 spec vitest)
- **O usuário faz:** Construir o áudio final: mix cortado na janela de edição, takes atrasados no instante certo, silêncio onde o take substitui o mix.
- **Gatilho (UI/atalho/CLI):** Parte do render.
- **Caminho handler → estado → regra/runtime:** `audioGraphOf(plan)` gera `filter_complex` (`atrim`, `adelay`, `volume=0:enable=between`, `amix normalize=0`, `apad`) e `audioArgsOf` (AAC).
- **Propriedades, unidades, modelo de dados:** Estéreo, `fltp`, taxa fixa `SAMPLE_RATE`.
- **Dependências:** MIG083 (Plano de mix: o que soa, onde a voz do mix cala porque há take)
- **Limitações:** O plano do mix é o mesmo do Studio (`audioMixPlan`).
- **Testes (existentes, não executados):** [packages/scene-engine/src/node/audio-graph.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/audio-graph.spec.ts) (5 casos)
- **Código:** [packages/scene-engine/src/node/audio-graph.ts:24](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/audio-graph.ts#L24) · [packages/scene-engine/src/node/audio-graph.ts:78](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/audio-graph.ts#L78)

### MIG093

**CLI `scene` (14 comandos, JSON em stdout, exit 0/1/2)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (18 casos em 1 spec vitest)
- **O usuário faz:** Operar o bundle sem navegador: catálogo de efeitos, resumo, listar/ler/escrever propriedades, edições em lote, câmera, narração, outline, takes, cortar do mix, validar, renderizar e abrir o Studio.
- **Gatilho (UI/atalho/CLI):** `scene <comando> <bundle> …` (`pnpm scene …`).
- **Caminho handler → estado → regra/runtime:** `bin/scene.mjs` só interpreta args e chama `core` (regras) e `node` (arquivos, TTS, ffmpeg, Chrome); escreve só com `--write`; 0 ok, 1 recusa (erros no JSON), 2 uso.
- **Propriedades, unidades, modelo de dados:** Comandos: effects, show, props, get, set, edit, camera (analyze|apply), narration (show|apply|import-srt), outline (show|apply), takes, cut-from-mix, validate, render, preview.
- **Dependências:** MIG094 (API de edição em lote `editBundle` (6 formas de edit)), MIG046 (Sistema de property paths (listar, ler, escrever, erros tipados))
- **Limitações:** `render` sem `--frames`/contact-sheet no baseline (existe no piloto).
- **API / CLI:** Ver `HELP` na origem; 14 comandos (13 com bundle + `effects`).
- **Testes (existentes, não executados):** [packages/scene-engine/src/node/scene-cli.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/scene-cli.spec.ts) (18 casos)
- **Código:** [packages/scene-engine/bin/scene.mjs:172](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/bin/scene.mjs#L172) · [packages/scene-engine/bin/scene.mjs:40](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/bin/scene.mjs#L40) · [packages/scene-engine/src/node/index.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/node/index.ts)

### MIG094

**API de edição em lote `editBundle` (6 formas de edit)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (20 casos em 1 spec vitest)
- **O usuário faz:** Aplicar uma lista de edições de qualquer camada num bundle, atomicamente, dizendo qual entrada falhou.
- **Gatilho (UI/atalho/CLI):** CLI `scene set|edit|camera apply|narration apply|outline apply`; HTTP `/bundle-edit`.
- **Caminho handler → estado → regra/runtime:** `editBundle(bundle, edits, {editFrame})`: entradas `{set,value}`, `{camera,clipId}`, `{narration}`, `{outline}`, `{rig,rigId}`, `{path,sceneId,pathId}` aplicadas em sequência sobre o bundle mudado até ali; `finalizeBundle`; erro {index,layer,code,message,hint}.
- **Propriedades, unidades, modelo de dados:** Camada do erro: edit|set|camera|narration|outline|rig|path|bundle.
- **Dependências:** MIG046 (Sistema de property paths (listar, ler, escrever, erros tipados)), MIG033 (Comandos de câmera (15 ops) com erros tipados e dica), MIG082 (Comandos de narração (10 ops)), MIG043 (Comandos de outline (6 ops))
- **Limitações:** Uma entrada recusada recusa o lote inteiro (nada gravado).
- **API / CLI:** Lib `editBundle`, `bundleEditSchema`, `BUNDLE_EDIT_SHAPES`.
- **Testes (existentes, não executados):** [packages/scene-engine/src/edit/edit-bundle.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/edit/edit-bundle.spec.ts) (20 casos)
- **Código:** [packages/scene-engine/src/edit/edit-bundle.ts:432](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/edit/edit-bundle.ts#L432) · [packages/scene-engine/src/edit/edit-bundle.ts:94](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/edit/edit-bundle.ts#L94)

### MIG095

**Consultas `show` e `summarize` do bundle (8 camadas)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (5 casos em 1 spec vitest)
- **O usuário faz:** Ler um resumo do bundle ou uma camada inteira (camera, narration, outline, lights, weather, effects, rig, paths).
- **Gatilho (UI/atalho/CLI):** `scene show <bundle> [--layer …]`.
- **Caminho handler → estado → regra/runtime:** `summarizeBundle`, `showLayer`, `parseShowLayer`.
- **Propriedades, unidades, modelo de dados:** —
- **Dependências:** MIG001 (Modelo do documento `render-bundle.json` (schema 1.0))
- **Limitações:** —
- **Testes (existentes, não executados):** [packages/scene-engine/src/edit/show-bundle.spec.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/edit/show-bundle.spec.ts) (5 casos)
- **Código:** [packages/scene-engine/src/edit/show-bundle.ts:7](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/edit/show-bundle.ts#L7) · [packages/scene-engine/src/edit/show-bundle.ts:73](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/edit/show-bundle.ts#L73)

### MIG096

**Exports programáticos do pacote (core, node, runtime, transport, testing)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Consumir o motor como biblioteca em scripts, no Studio e em pipelines.
- **Gatilho (UI/atalho/CLI):** `import … from "@tessel/scene-engine/(core|node|runtime|transport|transport/browser|testing)"`.
- **Caminho handler → estado → regra/runtime:** `core.ts` reexporta ~100 símbolos (schema, avaliação, comandos, propriedades, narração, outline, rig/path, clima, luzes); `node` (arquivos/TTS/ffmpeg/Chrome); `runtime` (Three.js).
- **Propriedades, unidades, modelo de dados:** Pacote ESM privado; `bin: scene`.
- **Limitações:** Pacote privado do monorepo; sem versionamento público.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [packages/scene-engine/src/core.ts](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/src/core.ts) · [packages/scene-engine/package.json:10](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/package.json#L10)

### MIG097

**Lançador do Studio e provas de QA (transporte, A/V, specs de navegador)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (0 casos em 0 spec vitest + 9 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** Subir o Studio local e rodar provas Playwright (câmera, luz, clima, efeitos, rig, follow, projetos, histórico, transporte) e a prova A/V de 15 s/120 s.
- **Gatilho (UI/atalho/CLI):** `pnpm studio:preview`, `studio:transport-test`, `studio:av-proof`, `scene-studio-*.spec.mjs`.
- **Caminho handler → estado → regra/runtime:** Scripts compilam o pacote e o app (vite), sobem o harness e dirigem `window.__sceneStudio` via Chrome.
- **Propriedades, unidades, modelo de dados:** 9 specs .mjs de navegador + provas.
- **Limitações:** Exigem Chrome, ffmpeg, pnpm; não rodam em CI padrão (`vitest` exclui e2e).
- **Testes (existentes, não executados):** [packages/scene-engine/scripts/scene-studio-camera.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-camera.spec.mjs) (script de navegador/Chrome); [packages/scene-engine/scripts/scene-studio-transport.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-transport.spec.mjs) (script de navegador/Chrome); [packages/scene-engine/scripts/scene-studio-follow.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-follow.spec.mjs) (script de navegador/Chrome); [packages/scene-engine/scripts/scene-studio-rig.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-rig.spec.mjs) (script de navegador/Chrome); [packages/scene-engine/scripts/scene-studio-light.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-light.spec.mjs) (script de navegador/Chrome); [packages/scene-engine/scripts/scene-studio-weather.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-weather.spec.mjs) (script de navegador/Chrome); [packages/scene-engine/scripts/scene-studio-effects.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-effects.spec.mjs) (script de navegador/Chrome); [packages/scene-engine/scripts/scene-studio-projects.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-projects.spec.mjs) (script de navegador/Chrome); [packages/scene-engine/scripts/scene-studio-history.spec.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-history.spec.mjs) (script de navegador/Chrome)
- **Código:** [packages/scene-engine/scripts/scene-studio-preview.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-studio-preview.mjs) · [packages/scene-engine/scripts/studio-av-proof.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/studio-av-proof.mjs) · [packages/scene-engine/scripts/studio-harness.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/studio-harness.mjs)

### MIG098

**Pipeline imagem → cena de splat (SHARP, MoGe-3, SPZ, R2)**

- **Estado:** `LIB` — Implementado em lib/API/CLI, sem UI · **Fonte:** baseline (d6adac15c6) · **Evidência:** Teste já existente, não executado (0 casos em 0 spec vitest + 1 script(s) de navegador/Chrome sem contagem)
- **O usuário faz:** A partir de uma pintura, gerar splats Gaussianos (SHARP), grade de profundidade e normais (MoGe-3), empacotar SPZ e montar um bundle.
- **Gatilho (UI/atalho/CLI):** `scripts/scene-pipeline/image-to-splat-scene.mjs` (+ python).
- **Caminho handler → estado → regra/runtime:** Roda localmente no Mac (SHARP/MoGe em MPS); `scene_gaussian.py --upload` publica no R2 (`wrangler r2 object put`) de staging/prod do Tessel; downloads de cena já publicada.
- **Propriedades, unidades, modelo de dados:** Modelos e execuções fora do repo (`SCENE_PIPELINE_HOME`).
- **Dependências:** MIG014 (Camadas de splat Gaussiano (Spark 2.4, `.spz`))
- **Limitações:** SHARP não é reprodutível bit a bit (README do pipeline); a licença dos pesos SHARP é restrita a pesquisa segundo anotação prévia do usuário (não verificada aqui). O laboratório original foi apagado e reconstruído a partir de transcrições.
- **Testes (existentes, não executados):** [packages/scene-engine/scripts/scene-pipeline/scene_gaussian.spec.py](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-pipeline/scene_gaussian.spec.py) (script de navegador/Chrome)
- **Código:** [packages/scene-engine/scripts/scene-pipeline/image-to-splat-scene.mjs](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-pipeline/image-to-splat-scene.mjs) · [packages/scene-engine/scripts/scene-pipeline/scene_gaussian.py](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-pipeline/scene_gaussian.py) · [packages/scene-engine/scripts/scene-pipeline/sharp/run-sharp.sh](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-pipeline/sharp/run-sharp.sh) · [packages/scene-engine/scripts/scene-pipeline/moge/run_moge3.py](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/scene-pipeline/moge/run_moge3.py)

### MIG099

**Experimento de modelo de mundo (RunPod, HunyuanWorld)**

- **Estado:** `EXP` — Experimental (branch piloto ou isolado) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código
- **O usuário faz:** Testar um modelo de mundo em GPU alugada, com teto de gasto.
- **Gatilho (UI/atalho/CLI):** `scripts/world-model/run-world-test.sh`.
- **Caminho handler → estado → regra/runtime:** Container + RunPod; viewer de malhas em camadas.
- **Propriedades, unidades, modelo de dados:** —
- **Limitações:** Experimento isolado; JOU-1191; gasta dinheiro; não alimenta o editor.
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [packages/scene-engine/scripts/world-model/README.md](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/world-model/README.md) · [packages/scene-engine/scripts/world-model/run-world-test.sh](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine/scripts/world-model/run-world-test.sh)

## L. Delta do branch piloto `t3/scene-engine-pilot` (experimental, fora da main)

### MIG100

**Camada de efeitos por clipe (faixas, clipes, parâmetros, envelope, escopo, chaves)**

- **Estado:** `EXP` — Experimental (branch piloto ou isolado) · **Fonte:** branch piloto (d40ac3f7c0), fora da main · **Evidência:** Teste já existente, não executado (12 casos em 1 spec vitest)
- **O usuário faz:** Colocar efeitos e transições como clipes em faixas na timeline, com parâmetros chaveáveis, envelope e escopo.
- **Gatilho (UI/atalho/CLI):** Timeline: faixas de efeitos e de transições; `scene fx <bundle> list|apply|explain|preset|transition`.
- **Caminho handler → estado → regra/runtime:** Modelo `fx` no bundle + `applyFxCommands` (19 ops: addTrack, removeTrack, renameTrack, reorderTrack, addClip, removeClip, moveClip, resizeClip, setParam, keyParam, unkeyParam, setEnvelope, setScope, duplicateClip, setTransition, resizeTransition, removeTransition, applyPreset, setHidden) + `fx-explain-*` (erros com orçamento).
- **Propriedades, unidades, modelo de dados:** Famílias look|transition; kinds pass|geometry-pass|transform|preset.
- **Dependências:** MIG067 (Looks de pós-produção (tinta, sonho, filme) com catálogo)
- **Limitações:** Branch não mergeado; `src/fx` tem 206 arquivos (inclui specs, goldens e spikes). Estende o `effects` de 3 looks da main.
- **Testes (existentes, não executados):** [packages/scene-engine/src/fx/fx-commands.spec.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/fx/fx-commands.spec.ts) (12 casos)
- **Código:** [packages/scene-engine/src/fx/fx-command-schema.ts:22](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/fx/fx-command-schema.ts#L22) · [packages/scene-engine/src/fx/fx-commands.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/fx/fx-commands.ts) · [packages/scene-engine/src/fx/fx-explain.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/fx/fx-explain.ts)

### MIG101

**Catálogo de 59 efeitos (31 looks + 28 transições) com baseline de pixel**

- **Estado:** `EXP` — Experimental (branch piloto ou isolado) · **Fonte:** branch piloto (d40ac3f7c0), fora da main · **Evidência:** Teste já existente, não executado (38 casos em 2 spec vitest)
- **O usuário faz:** Escolher entre looks (bloom, halation, filmGrain, vignette, godRays, smoke, fire, …) e transições (cut, fade, dissolves, wipes, push, iris, glitch, …).
- **Gatilho (UI/atalho/CLI):** Galeria; `scene fx catalog|show|describe|preview`.
- **Caminho handler → estado → regra/runtime:** Cada efeito = arquivo meta em `fx/catalog/*.ts` + shader de mesmo id em `runtime/fx/shaders`; spec recusa efeito sem shader e vice-versa; `__golden__` e `__baseline__/pixels.json`.
- **Propriedades, unidades, modelo de dados:** `family: look|transition`; parâmetros em `param-defs.ts`.
- **Dependências:** MIG100 (Camada de efeitos por clipe (faixas, clipes, parâmetros, envelope, escopo, chaves))
- **Limitações:** Pixels dourados são de um ambiente; sem execução aqui.
- **Testes (existentes, não executados):** [packages/scene-engine/src/fx/catalog/golden-files.spec.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/fx/catalog/golden-files.spec.ts) (1 casos); [packages/scene-engine/src/fx/catalog/index.spec.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/fx/catalog/index.spec.ts) (37 casos)
- **Código:** [packages/scene-engine/src/fx/catalog/index.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/fx/catalog/index.ts) · [packages/scene-engine/src/fx/catalog/fx-effect-meta.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/fx/catalog/fx-effect-meta.ts) · [packages/scene-engine/src/fx/__baseline__/pixels.json](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/fx/__baseline__/pixels.json)

### MIG102

**Edição de shots: aparar bordas, mover corte, duração do shot e da edição (ripple/roll)**

- **Estado:** `EXP` — Experimental (branch piloto ou isolado) · **Fonte:** branch piloto (d40ac3f7c0), fora da main · **Evidência:** Teste já existente, não executado (31 casos em 2 spec vitest)
- **O usuário faz:** Aparar o início/fim de um shot, mover o corte entre shots, mudar a duração do shot ou da edição inteira, com ripple ou roll, opcionalmente levando a narração.
- **Gatilho (UI/atalho/CLI):** Timeline (arrastar bordas/cortes); `scene shots <bundle> list|trim|move-cut|duration|edit-duration`.
- **Caminho handler → estado → regra/runtime:** `shot-commands.ts` (4 ops: trimShot, moveCut, setShotDuration, setEditDuration) → `shot-plan.ts` → camadas (`shot-layers-fx|keys|narration`); opções `withNarration`, `keepAbsolute`, `shrinkTransitions`, `content anchor|follow|auto`.
- **Propriedades, unidades, modelo de dados:** Imagem e áudio são independentes: encurtar o vídeo não mexe no mix.
- **Dependências:** MIG018 (Estrutura de shots, instâncias de cena e transições (somente leitura))
- **Limitações:** Branch não mergeado; sem split/duplicar/reordenar shots.
- **Testes (existentes, não executados):** [packages/scene-engine/src/shots/shot-commands.spec.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/shots/shot-commands.spec.ts) (19 casos); [packages/scene-engine/src/shots/shot-duration.spec.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/shots/shot-duration.spec.ts) (12 casos)
- **Código:** [packages/scene-engine/src/shots/shot-commands.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/shots/shot-commands.ts) · [packages/scene-engine/src/shots/shot-schema.ts:61](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/shots/shot-schema.ts#L61) · [packages/scene-engine/src/shots/shot-plan.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/shots/shot-plan.ts)

### MIG103

**Gestos de timeline como funções puras (arrastar shot, clipe de fx, transição, chaves)**

- **Estado:** `EXP` — Experimental (branch piloto ou isolado) · **Fonte:** branch piloto (d40ac3f7c0), fora da main · **Evidência:** Teste já existente, não executado (25 casos em 2 spec vitest)
- **O usuário faz:** Arrastar shot, clipe de efeito, transição e chaves (luz, clima, parâmetro, follow) com limites legais e recusa que diz o frame mais próximo.
- **Gatilho (UI/atalho/CLI):** Timeline; `scene keys <bundle> <path/keys> move-key|move-all|scale`; `scene follow … move-key|move-all|scale`.
- **Caminho handler → estado → regra/runtime:** `src/gestures/*` (shot-drag, fx-clip-drag, fx-transition-drag, key-frame-drag, key-retime, key-path-retime, scene-spans) → comandos; UI usa o mesmo núcleo ('decisões do editor no core').
- **Propriedades, unidades, modelo de dados:** —
- **Dependências:** MIG102 (Edição de shots: aparar bordas, mover corte, duração do shot e da edição (ripple/roll)), MIG100 (Camada de efeitos por clipe (faixas, clipes, parâmetros, envelope, escopo, chaves)), MIG047 (Keyframes de valores (resolver, regras, adicionar/remover no playhead))
- **Limitações:** Branch não mergeado; `studioCoreBoundary.spec.ts` garante a fronteira.
- **Testes (existentes, não executados):** [packages/scene-engine/src/gestures/shot-drag.spec.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/gestures/shot-drag.spec.ts) (17 casos); [packages/scene-engine/src/gestures/key-retime.spec.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/gestures/key-retime.spec.ts) (8 casos)
- **Código:** [packages/scene-engine/src/gestures/index.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/gestures/index.ts) · [packages/scene-engine/src/gestures/shot-drag.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/src/gestures/shot-drag.ts) · [apps/scene-studio/src/studioCoreBoundary.spec.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/apps/scene-studio/src/studioCoreBoundary.spec.ts)

### MIG104

**Timeline do Studio com faixas de shots, efeitos e transições, e inspector de item**

- **Estado:** `EXP` — Experimental (branch piloto ou isolado) · **Fonte:** branch piloto (d40ac3f7c0), fora da main · **Evidência:** Teste já existente, não executado (18 casos em 2 spec vitest)
- **O usuário faz:** Ver todos os shots (esmaecendo os fora do atual), efeitos e transições, selecionar um item e editar seus parâmetros; mudar o comprimento da edição.
- **Gatilho (UI/atalho/CLI):** Faixas **Shots**, **FX**, **Transições**; `EditLength`; `scene inspect <bundle> clip|transition|shot`.
- **Caminho handler → estado → regra/runtime:** Componentes `ShotsTrack`, `FxTrack`, `TransitionsTrack`, `EditLength`, hooks `useShotsEdit`, `useFxEdit`, `useTimelineInspector`; `engine/fxSlice.ts`, `shotsSlice.ts`; `timeline/*` (barras puras).
- **Propriedades, unidades, modelo de dados:** —
- **Dependências:** MIG103 (Gestos de timeline como funções puras (arrastar shot, clipe de fx, transição, chaves))
- **Limitações:** Branch não mergeado.
- **Testes (existentes, não executados):** [apps/scene-studio/src/engine/editorStoreShots.spec.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/apps/scene-studio/src/engine/editorStoreShots.spec.ts) (7 casos); [apps/scene-studio/src/engine/editorStoreFx.spec.ts](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/apps/scene-studio/src/engine/editorStoreFx.spec.ts) (11 casos)
- **Código:** [apps/scene-studio/src/components/ShotsTrack.tsx](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/apps/scene-studio/src/components/ShotsTrack.tsx) · [apps/scene-studio/src/components/FxTrack.tsx](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/apps/scene-studio/src/components/FxTrack.tsx) · [apps/scene-studio/src/components/TransitionsTrack.tsx](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/apps/scene-studio/src/components/TransitionsTrack.tsx) · [apps/scene-studio/src/components/EditLength.tsx](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/apps/scene-studio/src/components/EditLength.tsx)

### MIG105

**Outras novidades do piloto (render por frames/contact-sheet, normais Marigold, splat de imagem única, fumaça/fogo)**

- **Estado:** `EXP` — Experimental (branch piloto ou isolado) · **Fonte:** branch piloto (d40ac3f7c0), fora da main · **Evidência:** Só código
- **O usuário faz:** Renderizar frames escolhidos com contact sheet; gerar normais com Marigold-Normals; construir cena de splat só com uma imagem; efeitos de fumaça/fogo/raios/poeira.
- **Gatilho (UI/atalho/CLI):** `scene render --frames … --contact-sheet`; scripts.
- **Caminho handler → estado → regra/runtime:** Commits `render --frames`, `feat(lighting)`, `Build a splat scene from a single picture`, `smoke`, `fire`.
- **Propriedades, unidades, modelo de dados:** —
- **Dependências:** MIG100 (Camada de efeitos por clipe (faixas, clipes, parâmetros, envelope, escopo, chaves))
- **Limitações:** Branch não mergeado; detalhado só por assunto de commit e nomes de arquivo (não lido em profundidade).
- **Testes:** nenhum associado encontrado (Só código).
- **Código:** [packages/scene-engine/bin/scene.mjs:84](/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f/packages/scene-engine/bin/scene.mjs#L84)

## M. Planejado apenas (documentos, sem código no baseline)

### MIG106

**Plugins e efeitos isolados por fragmento (VGP-15) e Sprite Normals (VGP-16)**

- **Estado:** `PLAN` — Planejado apenas · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só documento
- **O usuário faz:** (Planejado) registro versionado de efeitos por fragmento com A/B; processador Sprite Normals + material de relight.
- **Gatilho (UI/atalho/CLI):** —
- **Caminho handler → estado → regra/runtime:** Roadmap `video-production-pipeline.md` (VGP-15/16) e `scene-studio-plan.md` §4–§6; no baseline só existem o catálogo de 3 looks e mapas de normais por camada via script.
- **Propriedades, unidades, modelo de dados:** —
- **Limitações:** Planos descrevem trabalho; não são implementação pronta. Parte virou código só no branch piloto.
- **Testes:** nenhum associado encontrado (Só documento).
- **Código:** [docs/video-production/scene-studio-plan.md:78](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/docs/video-production/scene-studio-plan.md#L78)

### MIG107

**Produção de ativos por IA e pipeline de vídeo narrado (VGP-01…11)**

- **Estado:** `PLAN` — Planejado apenas · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só documento
- **O usuário faz:** (Planejado) outline, elenco, roteiro, TTS, alinhamento, música, revisão, custo por minuto, orquestração durável.
- **Gatilho (UI/atalho/CLI):** —
- **Caminho handler → estado → regra/runtime:** Roadmap e specs `ai-asset-production-spec.md`; pertencem ao produto Tessel (workers/ai-service), não ao editor.
- **Propriedades, unidades, modelo de dados:** —
- **Limitações:** Fora do escopo: migração de produtos Tessel que não pertencem ao editor.
- **Testes:** nenhum associado encontrado (Só documento).
- **Código:** [docs/video-production/ai-asset-production-spec.md](/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/docs/video-production/ai-asset-production-spec.md)

## N. Ausente na origem (investigado e não encontrado)

### MIG108

**Biblioteca de mídia, busca, miniaturas e import pela UI**

- **Estado:** `AUS` — Ausente (investigado, não existe) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código (busca estática)
- **O que não existe:** Biblioteca de mídia, busca, miniaturas e import pela UI.
- **Como foi investigado:** `rg` por `thumbnail`, `media library`, `import` em `apps/scene-studio/src` e `scene-engine/src` (0 ocorrências de UI); assets entram só por script/bundle.
- **Destino hoje (somente leitura):** O destino já tem biblioteca/busca/import (F001, F002, F019). F001/F002/F019 (V).

### MIG109

**Clipes de vídeo/áudio como camadas**

- **Estado:** `AUS` — Ausente (investigado, não existe) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código (busca estática)
- **O que não existe:** Clipes de vídeo/áudio como camadas.
- **Como foi investigado:** O runtime só constrói `group`, `imagePlane` e `splat`; sem `<video>`/`VideoTexture`; áudio = 1 mix + takes de narração.
- **Destino hoje (somente leitura):** Destino tem vídeo, áudio e texto como assets/clipes. F019/F021/F029 (V).

### MIG110

**Camadas de texto e fontes**

- **Estado:** `AUS` — Ausente (investigado, não existe) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código (busca estática)
- **O que não existe:** Camadas de texto e fontes.
- **Como foi investigado:** Só legendas de narração (HTML no Studio, sharp+Pango no export); sem camada de texto no grafo nem seleção de fonte.
- **Destino hoje (somente leitura):** Destino tem texto (fonte bitmap 5×7) como clipe. F021 texto inline (V).

### MIG111

**Cortar/aparar/dividir/reordenar clipes, snapping, marcadores (no baseline)**

- **Estado:** `AUS` — Ausente (investigado, não existe) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código (busca estática)
- **O que não existe:** Cortar/aparar/dividir/reordenar clipes, snapping, marcadores (no baseline).
- **Como foi investigado:** `rg trimShot|moveCut` = 0; sem split de shot; 'marker' só aparece como `markError`; edição de shots existe só no branch piloto.
- **Destino hoje (somente leitura):** Destino tem mover/trim/reordenar/snap (F008, F011–F013); split é P (F017). F008/F011–F013 (V); F017 (P).

### MIG112

**Transformação de camadas e gizmos (posição/escala/rotação)**

- **Estado:** `AUS` — Ausente (investigado, não existe) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código (busca estática)
- **O que não existe:** Transformação de camadas e gizmos (posição/escala/rotação).
- **Como foi investigado:** `describeNode` só mostra tipo/sombra/binding; nenhum arrasto de nó; overlays existem só para luz, câmera, rig e path.
- **Destino hoje (somente leitura):** Destino tem `transform` por clipe (F021 plan). F021 (V genérico).

### MIG113

**Blend modes, máscaras e composição por passe (como recurso do usuário)**

- **Estado:** `AUS` — Ausente (investigado, não existe) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código (busca estática)
- **O que não existe:** Blend modes, máscaras e composição por passe (como recurso do usuário).
- **Como foi investigado:** `compositionPasses` só define ordem; `CustomBlending` aparece somente na sombra interna; máscara só no alfa da tinta.
- **Destino hoje (somente leitura):** Destino tem blend (`lighten` etc.) no compositor. F021 blend (V).

### MIG114

**Autosave, recuperação e 'salvar como'**

- **Estado:** `AUS` — Ausente (investigado, não existe) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código (busca estática)
- **O que não existe:** Autosave, recuperação e 'salvar como'.
- **Como foi investigado:** `rg autosave|recover` sem ocorrência funcional; toda edição regrava o arquivo imediatamente.
- **Destino hoje (somente leitura):** Destino tem salvar/abrir/autosave/recuperar (F022). F022 (V).

### MIG115

**Presets e templates (de projeto, de rig, de efeito)**

- **Estado:** `AUS` — Ausente (investigado, não existe) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código (busca estática)
- **O que não existe:** Presets e templates (de projeto, de rig, de efeito).
- **Como foi investigado:** `config.rigPresets` e `defaults` existem como dados e nunca são escritos ou lidos pela UI; sem templates.
- **Destino hoje (somente leitura):** Destino tem presets de exportação (F028). F028 (V) — outro domínio.

### MIG116

**Menus de contexto, multi-seleção e copiar/colar**

- **Estado:** `AUS` — Ausente (investigado, não existe) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código (busca estática)
- **O que não existe:** Menus de contexto, multi-seleção e copiar/colar.
- **Como foi investigado:** `rg onContextMenu|ContextMenu|clipboard` = 0; seleção é única.
- **Destino hoje (somente leitura):** Destino: seleção de tracks/clipes (F009); sem menus de contexto declarados. F009 (V).

### MIG117

**Presets de exportação, jobs, cancelamento e progresso no editor**

- **Estado:** `AUS` — Ausente (investigado, não existe) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código (busca estática)
- **O que não existe:** Presets de exportação, jobs, cancelamento e progresso no editor.
- **Como foi investigado:** `renderBundle` só tem `onProgress`; sem fila, cancelamento, estimativa de disco nem diálogo de export no Studio.
- **Destino hoje (somente leitura):** Destino tem F025/F028/F029. F025/F028/F029 (V).

### MIG118

**Integração com nuvem/IA dentro do editor (login, upload, geração)**

- **Estado:** `AUS` — Ausente (investigado, não existe) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código (busca estática)
- **O que não existe:** Integração com nuvem/IA dentro do editor (login, upload, geração).
- **Como foi investigado:** O Studio só chama o servidor local (mesma origem) e, opcionalmente, um TTS local por HTTP; sem SDK/credenciais; só o script `scene_gaussian.py --upload` usa R2 (ferramenta, não editor).
- **Destino hoje (somente leitura):** Destino proíbe serviços/credenciais nesta etapa (AGENTS.md). —

### MIG119

**Acessibilidade completa**

- **Estado:** `AUS` — Ausente (investigado, não existe) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código (busca estática)
- **O que não existe:** Acessibilidade completa.
- **Como foi investigado:** Há ~80 `aria-*` e ~32 `role`/`tabIndex` e DnD por teclado na árvore; não há auditoria nem atalhos documentados além do transporte.
- **Destino hoje (somente leitura):** Destino: GPUI Kit (a11y depende do Kit). —

### MIG120

**Edição de splat (selecionar, apagar, cortar ou transformar splats)**

- **Estado:** `AUS` — Ausente (investigado, não existe) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código (busca estática)
- **O que não existe:** Edição de splat (selecionar, apagar, cortar ou transformar splats).
- **Como foi investigado:** `rg` por `deleteSplat|splatEdit|erase|crop` sem ocorrência; splats são só carregados, ordenados e iluminados/atingidos pelo clima.
- **Destino hoje (somente leitura):** Destino carrega splats (F030) e não edita. F030 (V) só visualização.

### MIG121

**Qualidade de preview, LOD, cache de assets e streaming**

- **Estado:** `AUS` — Ausente (investigado, não existe) · **Fonte:** baseline (d6adac15c6) · **Evidência:** Só código (busca estática)
- **O que não existe:** Qualidade de preview, LOD, cache de assets e streaming.
- **Como foi investigado:** `rg pixelRatio|quality|downscale|LOD` não achou controle de qualidade; texturas ficam em `Map` por nó da cena ativa e são descartadas ao trocar de cena (sem LRU nem limite); assets são buscados inteiros por `fetch`.
- **Destino hoje (somente leitura):** Destino tem retenção de GPU com limites e LRU (256 MiB texturas), agendador de scrub e preview interativo sem leitura de pixel. F021 retenção/agendador (V).
