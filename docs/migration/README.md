# Migração do editor Three.js (Tessel Scene Studio) para o Tessel Editor (Rust + GPUI)

Auditoria e inventário de **o que existe** no editor atual, para outra pessoa/agente migrar sem redescobrir features. **Isto não é implementação**: nada foi alterado na origem nem no destino (exceto estes arquivos novos em `docs/migration/`).

## Mapa de navegação

Comece pelo [roadmap por etapas e building blocks](phased-roadmap.md): ele separa a base
compartilhada que falta das fases de produto. As fatias S0–S16 abaixo mantêm o detalhe por MIG.

| Arquivo | Para quê |
|---|---|
| [README.md](README.md) | este: escopo, snapshot, metodologia, o que foi/não foi validado, índice de superfícies, checklist e decisões |
| [source-feature-inventory.md](source-feature-inventory.md) | catálogo `MIGnnn` por fluxo/domínio com caminho handler→estado→regra, modelo, limites, API/CLI, testes e código com linhas |
| [feature-migration-matrix.md](feature-migration-matrix.md) | uma linha por `MIGnnn`: estado real, destino (Rust/runtime/GPUI), correspondência `Fxxx`, estratégia, risco e aceite |
| [migration-sequence.md](migration-sequence.md) | fatias verticais S0–S16, dependências, prioridade e evidência de paridade |
| [source-feature-inventory.json](source-feature-inventory.json) | o mesmo inventário em JSON compacto (IDs, estados, âncoras, destino) para scripts |

## O que é o editor de origem (resumo)

É o **Scene Studio**: um editor local de *cenas dirigidas 2.5D* para vídeos narrados, em React + Three.js 0.187 + Spark 2.4 (`apps/scene-studio`: 120 arquivos, 36 `.tsx`) sobre um motor (`packages/scene-engine`: 279 arquivos, 70 deles em `scripts/`). O documento é um único `render-bundle.json` (schema 1.0, zod) com camadas PNG em planos, instâncias de cena, **câmera segmentada**, shots, áudio (um mix + takes de narração), luzes, clima, efeitos de pós e, para a 'mão que escreve', rig/IK, paths e follow. Há uma CLI `scene` (14 comandos), uma API de edição em lote, um servidor local de preview e um render offline (Chrome + ffmpeg → H.264 + AAC).

**Não é um NLE genérico.** Não há biblioteca de mídia, clipes de vídeo/áudio na timeline, texto/fontes, split/trim/reordenar na UI (no baseline), blend/máscaras, gizmos de transformação, autosave nem menus de contexto — ver os 14 itens `AUS`. O destino (`tessel-editor`) já é, ao contrário, um editor de tracks/clipes com biblioteca, import, persistência, undo, jobs, playback e export. A migração é, portanto, **o domínio da cena dirigida** sobre a infraestrutura do destino, não a recriação de um NLE.

## Snapshot da origem e escolha explícita

| Item | Valor |
|---|---|
| Caminhos auditados | `/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/apps/scene-studio` e `/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine` |
| Branch / HEAD | `feat/image-coverage-r8` @ `d6adac15c61f4ef582a7ec259397b2e025d1f059` (commit de 2026-11-03T17:14:22-03:00) — **confirma o HEAD informado pelo pai** |
| Árvores git desses caminhos | `apps/scene-studio` = `4b7a1c246e4d437efdad747a6ed3bc1e500f2ee3` · `packages/scene-engine` = `d5fc98713731c6c0ae201ca1f3ce3bd0c9b17214` |
| Relação com a `main` local | `origin/main` = `0c0947578a` (2026-11-05T07:16-03:00); as **duas árvores acima são idênticas às de `origin/main`** (mesmos hashes); o HEAD está 64 commits atrás, nenhum toca esses caminhos |
| Sujeira do worktree | 893 entradas, **nenhuma** em `apps/scene-studio` nem `packages/scene-engine`: 1 modificada (`.claude/skills/placeholder-art/catalog.md`) e 892 não rastreadas (centenas de `placeholders-grey/*.webp`). Preservadas; mídias privadas **não copiadas** |
| Dependências | `node_modules` e `dist` **ausentes** na origem; nada foi instalado, compilado ou executado |
| Fingerprints de arquivos-chave (sha256, 16 hex) | `render-bundle.schema.ts` 953ddfbdfca5dc98 (599 linhas); `three-scene-runtime.ts` 787c409e999ca87b (1560 linhas); `scene.mjs` 353cb9244bb57a13 (540 linhas); `editorStore.ts` c166ed34844a1aa1 (808 linhas); `App.tsx` 7e3310424188a1eb (492 linhas); `studio-harness.mjs` 44a6dea4e5e1532e (321 linhas); `transport-controller.ts` a4cff19ec88929e1 (505 linhas); `render.ts` ae308c478c5f4578 (448 linhas) |
| Gerado em | 2026-11-05T11:06Z (UTC) |

### Divergências encontradas e decisão

Nota de escopo: decidir incluir as funcionalidades do piloto na migração **não exige**
mesclar esse branch na `main` da origem. São decisões separadas; não há autorização de
merge neste inventário. A documentação detalhada do piloto ainda precisa ser aprofundada
se ele for escolhido, pois o delta abaixo foi examinado apenas em nível de catálogo.

1. **Existe um branch mais novo e maior: `t3/scene-engine-pilot`** (`d40ac3f7c0`, 2026-11-04T10:32-03:00, worktree limpo em `/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f`). Não é ancestral do HEAD nem da `main` local; está **50 commits à frente** nesses caminhos e **2 atrás**; o diff de `apps/scene-studio` + `packages/scene-engine` é de 603 arquivos (505 adicionados, 95 modificados, 3 renomeados). Traz a camada de efeitos/transições (59 efeitos), edição de shots na timeline (aparar, mover corte, ripple/roll), gestos de timeline como funções puras e novos verbos da CLI (`fx`, `shots`, `keys`, `follow`, `inspect`).
2. **Escolha:** o inventário **principal é o snapshot confirmado** (equivalente à `main`), porque é o que o pai indicou e porque um branch de experimento **não** é produção. O piloto entra **apenas como delta rotulado** (`EXP`, MIG100–MIG105) e foi lido **só em nível de catálogo** (estrutura, comandos, assuntos de commit). Quem precisar de split/trim de shots, transições ou fx deve reabrir o piloto depois de D1.
3. Existe um worktree antigo (`/Users/wallyssonnunes/.t3/worktrees/tesselapp/scene-preview`, `90590e3d32`, 2026-11-02T15:27, detached): anterior ao baseline, **ignorado**.
4. **Docs desatualizados na própria origem:** `docs/video-production/studio-rig-projects-plan.md` diz 'Estado: plano, sem código no editor ainda', mas rig, follow e projetos múltiplos **existem** no código do baseline (MIG006, MIG069–MIG080). O inventário segue o código.
5. O repositório pai `tesselapp` (cwd da tarefa) **não** contém essas pastas nesta branch; `packages/editor` ali é o editor de notas e não é alvo. Não há `graphify-out` na origem (o grafo do cwd é de outro repositório e não foi usado).

## Estado do destino no momento da leitura (somente leitura)

`/Users/wallyssonnunes/www/tessel-editor`: Rust + GPUI Kit, **sem commit inicial** (tudo não rastreado), com trabalho concorrente em runtime/core/docs (OPT01–OPT10). Registros para evitar alegar estado final: sha256(16)/mtime — `AGENTS.md` 86222b67af2ef274 (2026-11-04T17:12); `docs/features/README.md` e2247306bd871fd1 (2026-11-05T02:07); `docs/README.md` a726af06acdeb837 (2026-11-05T07:21); `docs/core-architecture.md` bd0adea83aed5e34 (2026-11-04T18:00); `docs/features/viewport.md` bfe9228c12a6b545; `docs/features/spark.md` 6ba43ce65226bf16; `docs/features/video-export.md` 63a515a91716ed61; `crates/core/src/content.rs` b92855fb339f9378; `runtime/src/scene.js` 5d6fa1f684c7e572. Leituras: `AGENTS.md`, skill `tessel-feature-contract`, `docs/README.md`, `docs/features/README.md` (F001–F030), `core-architecture.md`, `migration-scope.md`, o relatório de mídia/export (cabeçalho), e os contratos F015, F021, F022, F024, F029, F030. **Nenhum arquivo do destino foi alterado** (só criei `docs/migration/`).

## Metodologia

1. Verifiquei o snapshot (`git rev-parse`, status, relação com `origin/main`, worktrees) e busquei implementações mais novas (achado: branch piloto).
2. Instruções aplicáveis: `AGENTS.md` da raiz da origem (241 linhas, regras gerais do monorepo; li o começo e fiz `rg` por scene/video/three: **nenhuma regra específica desta área**; não há `AGENTS.md` nas pastas auditadas) e o `AGENTS.md` do destino (106 linhas, lido inteiro) + a skill `tessel-feature-contract`. Também li o `README` do pacote, o início da skill `.claude/skills/scene-cli` (232 linhas; confere com o `HELP` da CLI) e, só para **estado/planejado**, `docs/video-production/*` (títulos, status, roadmap VGP). As skills `scene-effects` e `scene-element-recipes` foram apenas listadas.
3. Fui do documento (schema) para cada superfície: entry points (`main.tsx`, `App.tsx`, `StudioEditor.tsx`), store, registro de projetos, hooks, componentes, `describe*` do inspector, comandos (câmera, narração, outline, rig, path, edição em lote), propriedades por caminho, runtime, transporte, Node (render, TTS, takes), CLI e servidor.
4. Depois da leitura dirigida, usei `rg`/`git grep` para fechar lacunas e para **provar ausências** (busca registrada em cada item `AUS`). Âncoras `arquivo:linha` foram **resolvidas por script direto do blob do commit** (falharia se uma âncora não existisse).
5. Contei casos de teste por regex (`it/test`) nos specs referenciados; **não executei testes** (sem dependências, e a tarefa proibia instalar/compilar). Um import de função não foi tratado como prova de API alcançável: cada alcançabilidade (UI/CLI/HTTP) foi seguida até o gatilho.

## O que foi e o que não foi validado

| Rótulo | Significado aqui | Onde se aplica |
|---|---|---|
| **Só código** | li o código, nenhum teste associado | 21 MIGs |
| **Teste já existente, não executado** | spec presente; não rodei | 84 MIGs; 1361 casos contados (pode haver spec citado por mais de um MIG) |
| **Só documento** / **busca estática** | planejado em docs; ausência provada por `rg` | MIGs `PLAN` e `AUS` |
| **Prova executada agora** | **nenhuma execução funcional** (nem UI, nem testes, nem render). Só `git`, `rg`, `shasum`, contagem e geração dos documentos | 0 MIGs |

Totais da origem (baseline): 85 specs `.spec.ts` com 781 casos (`apps/scene-studio` 105 + `packages/scene-engine` 676), 9 specs de navegador `.spec.mjs` (Chrome) e 1 spec Python; `three-scene-runtime.ts`, `weather-layer.ts`, `runtime/index.ts` e `web-audio-transport.ts` são **excluídos da cobertura** (`vitest.config.ts`) e os demais módulos WebGL (`plane-*`, `splat-*`, `rig-layer`, `reveal-layer`) não têm spec: dependem das provas de navegador. Pixel hashes valem para um ambiente de export fixo.

## Contagens (fecham entre inventário, matriz e fatias)

- **MIGs: 121** no inventário = 121 linhas da matriz = 107 em fatias + 14 ausentes.
- Por estado: **UI** 48, **LIB** 47, **PARC** 2, **EXP** 7, **INAT** 1, **PLAN** 2, **AUS** 14 (soma 121).
- Por prioridade: ausente (sem migração) 14, essencial 24, futuro 23, não migrar 16, seguinte 44.
- Por tipo: JS reutilizável (runtime/shared) 12, Port de domínio Rust 45, Misto (regra Rust + desenho JS) 12, Ferramenta/oráculo (fora do app) 7, UI/adapter ou N/A 45.
- Por fonte: baseline 115, piloto 6.
- Por domínio: A 11; B 13; C 6; D 8; E 15; F 7; G 6; H 2; I 12; J 9; K 10; L 6; M 2; N 14.

## Índice de superfícies examinadas

| Superfície | Quantidade | Examinado | Não analisado a fundo e por quê |
|---|---|---|---|
| Entry points do app | 3 (`index.html`, `src/main.tsx`, `src/App.tsx`) | lidos | — |
| Painéis do editor | 4 (camadas, viewport, inspector, timeline) + cabeçalho/abas | `App.tsx`, `StudioEditor.tsx` lidos inteiros | — |
| Componentes React | 20 em `components/` + 10 em `components/layers/` + 15 em `ui/` | interações por `rg` (ponteiro/teclado/menu) e leitura dos principais | interior de `CameraTrack.tsx` (668 linhas), `RigOverlay`, `FollowOverlay`: só handlers e rótulos |
| Menus | 3 dropdowns: Project 4 itens, Rig 9 itens, Follow 9 itens; **0 menus de contexto** | todos os itens enumerados | — |
| Atalhos | 7 combinações (Espaço, ←, →, Shift+←/→, ⌘/Ctrl+Z, ⇧⌘Z, ⌘/Ctrl+Y) + F2 na árvore + DnD por teclado | lidos | — |
| Actions do store (`useEditor`) | 11 do `EditorState` + 12 do slice de rig + 7 do slice de path; 29 métodos em `window.__sceneStudio` | tipos e API lidos | corpos de `rigSlice.ts` (657) e `pathSlice.ts` (459) |
| Controles de inspector (`describe*`) | 10 descritores (câmera, luz, cena/clima, efeitos, nó, grupo, narração faixa/linha, rig, follow/path) | chaves e tipos de campo enumerados por `rg` | texto das 383 chaves i18n não lido chave a chave (paridade pt/en/es verificada: 383 = 383 = 383) |
| Comandos/ops de domínio | câmera 15 + narração 10 + outline 6 + rig 11 + path 2 = 44; edição em lote 6 formas | schemas lidos | regras internas de `camera-commands.ts` (1.342 linhas): ops e erros, não cada ramo |
| Property paths | famílias `scene/…`, `camera/…`, `rig/…` (ver MIG046) | construtores de path e provedores lidos | lista exaustiva de paths por bundle depende de estado (não gerada) |
| Schemas zod | 30 `*Schema` exportados + ~15 locais no schema do bundle | schema do bundle lido inteiro; demais por `rg` | — |
| Códigos de integridade | ~70 | listados por `rg` | lógica de cada regra (912 linhas) não revisada ramo a ramo |
| CLI `scene` | 14 comandos | `bin/scene.mjs` lido inteiro | — |
| Rotas HTTP do servidor | 12 PUT + 4 GET + `/projects` (GET/POST/PATCH/DELETE) | `studio-harness.mjs` lido inteiro; `studio-project-api.mjs` (trecho da API) | `studio-http.mjs`, `viewer.ts` por assinatura |
| Runtime Three.js | `three-scene-runtime.ts` (1.560) + 12 módulos | estrutura, métodos, construção de planos/splats, desenho | GLSL de `plane-effects-material.ts` (562), `splat-weather-sim.ts` (710), shaders de pós: cabeçalhos e catálogo, não o código de shader |
| Transporte/áudio | controller 505, web-audio 243, coordenador 156 | controller lido nas partes centrais | ramos de erro do áudio |
| Node (render/TTS/takes/captions) | 10 módulos | render, tts, takes, audio-graph por trechos e assinaturas | pintura de legenda (sharp/Pango) só pela API |
| Scripts | 70 arquivos (mjs, py, sh) | listados; cabeçalhos/READMEs de `scene-pipeline`, `world-model`, `elements/rig` | corpo dos scripts Python e dos `*.spec.mjs` de navegador (9) não lido; não executados |
| Docs de planejamento e skills | `docs/video-production/*` (≈20 docs) e `.claude/skills/scene-*` (3) | títulos/estado, roadmap VGP e início de `scene-cli` | conteúdo integral; `scene-effects` e `scene-element-recipes` não lidas |
| Branch piloto | 505 arquivos novos | estrutura, comandos (`fx` 19 ops, `shots` 4 ops), catálogo (59 efeitos), CLI help, commits | implementação (shaders, compositor, gestos): **não lida** |
| i18n / estilos | 3 JSON + `globals.css` | contagens e paridade | — |

## Checklist de cobertura pedida

| Tema pedido | Resultado | MIGs |
|---|---|---|
| Criação/abertura/save/formatos, assets, import, export, transcode, presets | Abrir/gravar bundle e projetos existem; **import de mídia pela UI, transcode de assets e presets de export não existem** (só `spz-transcode.mjs` em ferramenta e ffmpeg no render) | MIG001–MIG011, MIG090, MIG098, AUS |
| Viewport Three/Spark, cenas, múltiplos objetos | Existe (planos, splats, grupos); sem vídeo nem texto como objetos | MIG012–MIG015, MIG023 |
| Imagens/vídeo/áudio/texto/fontes | Imagem PNG ✔; áudio = 1 mix + takes; **vídeo, camada de texto e fontes: ausentes**; legendas existem | MIG013, MIG026, MIG087, MIG109, MIG110 |
| Timeline/tracks/trims/split/reorder/snapping/markers | Só faixas de câmera/rig/follow/chaves/waveform e navegação de shots; **trim/split/reorder/snapping/markers de shots: ausentes no baseline, existem no piloto** | MIG018, MIG036, MIG047, MIG111, MIG100–MIG105 |
| Transport/seek/reprodução/áudio/duração e FPS | Existe | MIG025–MIG030 |
| Câmeras/caminhos/keyframes/interpolação/easing | Existe (câmera segmentada); 3 easings | MIG031–MIG038, MIG047 |
| Camadas/grupos/pastas/seleção/inspector/gizmos/transform | Seleção, camadas, outline, inspector ✔; **gizmos e transformação de camada: ausentes**; 'pastas' = grupos do outline | MIG039–MIG045, MIG112 |
| Luzes/sombras/markers/áreas | Luzes pontuais (≤4), sombras, mapas ✔; marcadores = handles de overlay; **luzes de área/direcionais: ausentes** | MIG054–MIG059 |
| Efeitos/cor/filtros/pós/composição/blend/máscaras | 3 looks de pós + grade de cor do clima; blend/máscaras de usuário ausentes; 59 efeitos só no piloto | MIG067, MIG068, MIG019, MIG113, MIG101 |
| Splat edit/dynamics/lighting/weather/partículas | Splat visualização, luz e clima ✔; **edição de splat ausente**; partículas = só chuva/neve | MIG014, MIG058, MIG066, MIG120 |
| Presets/templates | Ausentes (dados `rigPresets` sem uso) | MIG115, MIG024 |
| Undo/redo/autosave/recovery | Undo/redo ✔ (200); autosave/recovery ausentes | MIG049, MIG048, MIG050, MIG114 |
| Atalhos/menus de contexto/acessibilidade | Atalhos ✔; **menus de contexto ausentes**; a11y parcial (aria + DnD por teclado) | MIG028, MIG041, MIG116, MIG119 |
| Media library/search/thumbnails | Ausentes | MIG108 |
| Determinismo, qualidade de preview, cache, stream, cancelamento, jobs | Determinismo ✔ (por design e provas de navegador); **qualidade/cache/stream/cancelar/jobs ausentes** | MIG022, MIG021, MIG121, MIG117 |
| CLI batch / APIs programáticas | CLI `scene` (14), `editBundle`, `show`, exports do pacote, `window.__sceneStudio` | MIG093–MIG096, MIG053 |
| Integrações cloud/IA realmente conectadas | Só um TTS HTTP **local** opcional; sem cloud no editor. O script de splat usa `wrangler r2 object put` (ferramenta, produto Tessel) | MIG084, MIG098, MIG118 |

Código experimental isolado (`world-model`, branch piloto) **não** entrou como necessidade de migração por acidente: está marcado `EXP` e priorizado como futuro/não migrar.

## Decisões que precisam ser tomadas

| # | Tema | Recomendação e pergunta | Afeta |
|---|---|---|---|
| D1 | Baseline vs. branch piloto | Usar como baseline o snapshot confirmado (idêntico à `main` local nesses caminhos) para S0–S14 e tratar o piloto (`t3/scene-engine-pilot`, 50 commits à frente, fora da main) como delta S15, reanalisado só quando congelar. **Decisão de escopo:** quais funcionalidades do piloto entram na migração e com qual snapshot? A integração desse branch na main da origem é uma decisão separada; até essa escolha, o delta permanece rotulado experimental. | MIG100–MIG105, S15 |
| D2 | Compatibilidade de formato | Recomendo ler/escrever `render-bundle.json` v1.0 como formato de intercâmbio (como F027 'importar de outro projeto') sem substituir o `.tessel.json`, com política explícita para campos desconhecidos (preservar). **Decisão:** o destino precisa abrir e regravar bundles existentes, ou só importá-los? | MIG001, MIG024, S0 |
| D3 | Modelo de cena | A origem é planos 2.5D + câmera segmentada + shots; o destino é tracks/clipes e `tessel-scene@1` (caixas, esferas, splats, ambiente+sol fixos). Recomendo um tipo novo de cena em camadas (planos texturizados com alpha) em vez de simulá-lo com clipes 2D, porque a paralaxe depende da câmera 3D. | MIG012, MIG013, MIG018, MIG081 |
| D4 | Onde mora a simulação visual | Recomendo: desenho (clima, sombras, pós, camada de rig/reveal) permanece em JS no runtime do destino; regras e parâmetros (gramática, keys, validação) ficam no core Rust. Contraria a regra do destino de 'JS só desenha' apenas se o JS passar a calcular regra. | MIG023, MIG056–MIG068, MIG070–MIG071, MIG078 |
| D5 | TTS | O destino proíbe serviços nesta etapa. Recomendo adapter injetado em `crates/media` (como FFmpeg), desligado por padrão. **Decisão:** permitir chamada HTTP local a um servidor TTS OpenAI-compatível? | MIG084, S6 |
| D6 | Pipelines de ML (SHARP, MoGe-3, mundo) | Confirmar que ficam fora do app (ferramentas que geram `.spz`/mapas); só os formatos de saída são contrato. | MIG098 (pipeline de splat), MIG099 (modelo de mundo) |
| D7 | Solver de IK | `CCDIKSolver` do Three.js decide a pose; portar para Rust muda os números. Recomendo reusar em worker JS até haver necessidade de bake headless sem Chrome. | MIG070, S13 |
| D8 | Contrato de cor do export | A origem marca BT709/sRGB/`tv` e verifica; o destino não marca. Recomendo adotar a marcação e a verificação no F029. | MIG091, S7 |
| D9 | Idiomas | A origem tem pt-br/en/es (383 chaves). O destino não declara i18n. **Decisão:** haverá i18n? | MIG051 |
| D10 | Multi-projeto | A origem abre vários bundles em abas com trava de escrita. O destino tem projeto único + sequências. Recomendo **não** migrar abas; duplicar/abrir já existe. | MIG006–MIG009, S14 |
| D11 | Overlays sobre o monitor | Handles de câmera/luz/rig/path vivem sobre o viewport; no destino a WebView pinta acima de GPUI. Recomendo desenhar os handles no runtime JS (dentro da WebView) e mandar eventos semânticos ao core. | MIG037, MIG059, MIG074, MIG079 |

## Restrições respeitadas

Origem somente leitura (nenhum arquivo, `AGENTS`, grafo ou dependência alterado; sem instalar, compilar, transcodificar, gerar mídia, commit/push/deploy ou ação externa); sem subagentes; sem abrir apps nem cliques; destino: só novos `docs/migration/*.md` e o JSON; os arquivos do outro agente (OPT01–OPT10) não foram tocados; nenhuma mídia privada copiada; nenhum trecho extenso de código copiado (apenas links com linhas).
