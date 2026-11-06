# Matriz de features

Esta é a entrada obrigatória para implementar ou alterar comportamento. Toda linha tem um
`.md` com contrato e evidências. Uma feature de edição só está completa quando funciona
pela biblioteca, CLI sem janela e UI. Gaps históricos abaixo ficam explícitos.

**Legenda:** V = validado por evidência reproduzível da superfície; I = implementado,
sem validação específica completa; Parcial = parte do contrato entregue; P = pendente;
N/A = não se aplica, com justificativa. Um teste de regra não prova interação de UI.

Baseline após extração do core: UI e CLI da janela herdadas da migração GPUI Kit, com
limites no relatório. A CLI headless implementa o parser compartilhado e tem smoke de
sessão/erros, e os 13 exemplos documentados foram executados. O fluxo de projeto/mídia/export (F019, F022–F030)
foi provado ponta a ponta na janela real: [relatório](../media-project-export-report.md). Isso não cobre todas as variantes de cada
contrato, por isso o acesso completo permanece I. Veja [evidências](../core-foundation-report.md).
Lib V significa testes do modelo demo, não um motor real de edição.

| ID | Feature / contrato | UI | CLI da janela | CLI headless | Lib | Entrada compartilhada |
|---|---|---|---|---|---|---|
| F001 | [Pastas: expandir e selecionar](folders.md) | I | I | I | V | `SetFolderExpanded / SelectFolder` |
| F002 | [Busca e seleção de mídia](media-selection.md) | I | I | I | V | `SetMediaSearch / SelectMedia` |
| F003 | [Tabs do workspace e inspetor](tabs.md) | I | I | I | V | `SetWorkspaceTab / SetInspectorTab` |
| F004 | [Accordions do inspetor](accordions.md) | I | I | I | V | `SetSectionExpanded` |
| F005 | [Sliders, selects e switches de propriedades](properties.md) | I | I | I | V | `SetProperty`, `property.gesture.*` (28 core + 13 GPUI + 12 checks de socket/janela; mouse físico não provado) |
| F006 | [Reset de propriedades e seções](reset.md) | I | I | I | V | `ResetProperty / ResetSection` |
| F007 | [Tempo, seek e passos por frame](seek.md) | I | I | I | V | `SetPlayhead` |
| F008 | [Zoom da timeline e snapping](timeline-view.md) | I | I | I | V | `SetTimelineZoom / SetSnap` |
| F009 | [Seleção de tracks e clipes](selection.md) | I | I | I | V | `SelectTrack / SelectClip` |
| F010 | [Visibilidade e mute de tracks](track-flags.md) | I | I | I | V | `SetTrackFlag` |
| F011 | [Mover clipes entre posições/tracks](move-clips.md) | I | I | I | V | `MoveClip` |
| F012 | [Trim das bordas de clipes](trim.md) | I | I | I | V | `TrimClip` |
| F013 | [Reordenar tracks](reorder-tracks.md) | I | I | I | V | `ReorderTrack` |
| F014 | [Dock, tema e janela](desktop.md) | I | I | N/A¹ | Parcial¹ | `LayoutCommand` / adapter |
| F015 | [Criar projetos, tracks e clipes dinâmicos](project-creation.md) | Parcial³ | V | V | V | `ops::parse_op` / `Command` |
| F016 | [Adicionar e processar efeitos](effects.md) | P | P | P | P | Não entregue |
| F017 | [Dividir clipe / corte interno](split.md) | P | P | P | P | Não entregue |
| F018 | [Playback e relógio de transporte](playback.md) | V⁶ | V⁶ | N/A⁶ | V⁶ | `SetPlayhead` + `playback.*` (adapter do desktop, `crates/audio`) + plano de áudio do core |
| F019 | [Importar e processar mídia real](media-io.md) | V⁴ | V | V | V | `media.import|relink|search|status|prepare|thumbnail|waveform|proxy|preview_proxy` |
| F020 | [Persistência, undo e exportação](persistence-export.md) (índice, desmembrada em F022–F029) | — | — | — | — | Ver F022–F029 |
| F021 | [Viewport multimídia e composição](viewport.md) | V⁵ | V | V | V | `RenderPlan` / bridge v1 / `preview.policy` |
| F022 | [Documento do projeto: salvar, abrir, autosave, recuperar](project-document.md) | V | V | V | V | `project.save|open|autosave|recover|discard_recovery|describe` |
| F023 | [Múltiplas sequências](sequences.md) | V⁷ | V | V | V | `sequence.add|activate|remove|rename|length` + nó `timeline.sequence` |
| F024 | [Desfazer e refazer](history.md) | V | V | V | V | `history.undo|redo|status` |
| F025 | [Jobs, cancelamento e staging](jobs.md) | V | V | V | V | `job.list|get|wait|cancel|commit` |
| F026 | [Empacotar e restaurar](package.md) | V | V | V | V | `project.package_preflight\|package\|verify\|restore`; R4 replace: lib/CLI testados (10 reconferidos), UI inalterada; queda entre renames ainda limitada |
| F027 | [Importar de outro projeto](interchange.md) | V | V | V | V | `project.import` |
| F028 | [Presets de exportação](presets.md) | V | V | V | V | `preset.list|save|delete|select` |
| F029 | [Exportar vídeo com áudio](video-export.md) | V | V | V⁸ | V | `export.preflight|start` |
| F030 | [Gaussian splats (Spark 2.4.0)](spark.md) | V | V | V | V | cena `tessel-scene@1` + asset `splat`; [harness concorrente](spark-concurrency.md); malhas emprestadas voltam ao asset e servem o quadro seguinte (BL131: 0 geradores/programas Spark novos por quadro de crossfade, ≈ 0,5 ms de CPU, `render_ms` sem mudança mensurável, hash dos quadros idêntico, [medição](../viewport-performance-results/bl131-shared-effect-leases/README.md)); gate global da rodada pendente |
| F031 | [Cena dirigida: importar, consultar e regravar render-bundle 1.0](directed-scene.md) | P | P | P | Parcial¹⁰ | `directed_scene::DirectedScene`; inteiros extremos recusados sem panic; adapters pendentes |
| F032 | [Oráculo de equivalência da migração (BB10/E0)](migration-oracle.md) | N/A⁹ | N/A⁹ | N/A⁹ | Parcial⁹ | `oracle.mjs verify` (55) + `extended.mjs verify` (47); 49 testes Node e 15 testes Rust estendidos passaram |
| F033 | [Cena dirigida: tempo, câmera e gate temporal (BB03)](scene-temporal.md) | P | P | P | Parcial¹¹ | `Bundle::evaluate_camera`, `evaluate_frame`, `DirectedScene::temporal_issues`; orçamento agregado de FOV; adapters pendentes |
| F034 | [Catálogo de propriedades e edição atômica de cenas (BB04)](scene-edit.md) | P | P | P | Parcial¹² | `scene_edit::{catalog, apply, dry_run}`; S6 (a validar): componentes `X/Y/Z`, `renderOrder`, pose inicial com conversão do 1º movimento, `modified` por dado (pai e eixo), oráculo fixo de 31 casos com divergências medidas; prova de superfície com guarda de disco testada também em Python -O; revisão R2: preservação por receita corrigida e asserção de avanço do playback reforçada; gates executados em `cf21f97` (fmt, check, clippy, workspace 1215 passaram/0 falharam/9 ignorados com Chrome real) e prova de superfície `scripts/scene-edit-proof.py` (janela própria 44 checks + headless 32 checks passaram na rodada limpa de `d0d972b`, registrada em F034) |
| F035 | [Animação de nós e matrizes de mundo (BB03)](scene-animation.md) | P | P | P | Parcial¹³ | `Bundle::{evaluate_frame, node_timeline, evaluate_instance_nodes, performance_plan}` |
| F036 | [Cenas no projeto, histórico e CLI](scene-project.md) | P | P | V | V | `Command::Scene` / `scene.*`; schema 2 com cenas, undo compartilhado, contagem de bytes em cache, medida por `AdmittedScene::prepare` (BL100a) |
| F037 | [Plano avaliado da cena dirigida](scene-plan.md) | Parcial | V | V | V | `ScenePlan`, `scene.plan` com gates e candidato de gesto; `PlanTimeline` reutilizável com equivalência testada na lib (R6); adotada no export F042, ganho real ainda não medido; 60 checks headless do plano; renderer F041 |
| F038 | [Recursos da cena dirigida: manifesto, preflight e materialização (BB06)](scene-resources.md) | P | P | P | Parcial¹⁵ | `scene_resources::ResourceManifest`; `directed_resources::{preflight, materialize}`; integração host/pacote/CLI em F040 |
| F039 | [Interação de cenas](scene-interaction.md) e [inspector/árvore](scene-inspector.md) | Parcial | V no corpus | V | V | `scene.*` compartilhado; R3: sessão opaca, summary e poda no undo (36 testes core/JSONL reconferidos); gate da janela desta mudança pendente; aparência/ponteiro/handles pendentes |
| F040 | [Recursos da cena no editor](scene-resource-integration.md) | Parcial | V no corpus | V no corpus | Parcial | `scene.resources.*`, `project.import_scenes`, pacote/restauração; integração F041 com mapas verificados; PendingAdoption e claims/cleanup testados no host; claims persistentes sob lease e `recover` provados com dois processos reais na lib (BL111, a validar no head da PR); `scene.resources.claims`/`recover` como jobs em JSONL (`tessel-media` real) e na janela via `op` (BL111.1, a validar; sem botão); falha de gravação após a cópia, aceitar (`Provisional::accept` fallível, último passo transacional), fundir (`transfer` gravado antes) e `claims.json` sem lock fecham-se com falha fechada, e o hash do `recover` é cancelável entre blocos (BL111.2, a validar; dois processos reais e janela própria); custo UI BL112 pendente; formatos BL077 pendentes  Regressões S2: relatório parcial em cancelamento/falha tardia e cleanup de múltiplas cenas.  Falha de forget propaga relatório parcial; prova nativa repetida23/23.  Contratos de ownership documentados (BL111.5; medição do serviço pendente). |
| F041 | [Renderer da cena dirigida](scene-renderer.md) | Parcial | V no corpus | V no corpus | V no subconjunto | `scene.render`, SceneHost, SceneRenderer e SceneSlots; pixels Chrome/WKWebView; R8: host 13/13 e harness 7/7 reconferidos, agente reporta 218 checks nativos em duas execuções com perda forçada; gate global pendente ([relatório](../migration/next-r8-report.md)); S4/BL102: identidade aditiva `asset.identity` em `asset.load`/`asset.state` (runtime, core puro, host dirigido) a validar até a revisão; headless (`SceneRenderer`, BL102b) carimba e confere o eco, a validar; export dirigido F042 em validação  BL102c: teste de substituição21/22 estabilizado por fetch retido; suíte runtime completa 108/108 no head `2c7cf7c` (coordenador, atribuída) e, no head `586a671`, 108/108 numa execução e 107/108 noutra (corrida do teste de vídeo B2, BL128; ver [scene-splat.md](scene-splat.md#revalidação-no-head-586a671-executado-com-proveniência)); Opus final pendente |
| F042 | [Exportação de cena dirigida](scene-export.md) | V no corpus | V no corpus | V no corpus | V no corpus | `scene.export`/preflight; PlanTimeline, snapshot e FFmpeg; core15/media7/desktop190 reconferidos; R1 reporta render10 com 1080p/4K; CLI/socket/diálogo finais: 23 fases/570 checks; gate global passou; áudio dirigido: F042A; S3 reporta media11/render11; BL109 (a validar no gate): bytes copiados e verificados por hash no job, troca de mesmo tamanho → `resource_changed`, original reescrito durante o job isolado |
| F042A | [Mix offline dirigido no export de cena](scene-export-audio.md) | Parcial | V no corpus | V no corpus | V no corpus | `scene.export` com o WAV declarado: janela/offset/trecho em tempo racional, conversão de frame rate e de sample rate, AAC com a duração do vídeo; core 21 (integração) + 11 (unidade) e mídia 29 na rodada de correção (FFmpeg real, executados; o WAV entra na cópia de bytes congelados do job, BL109), CLI headless com Chrome e oráculo da R10 (fase de áudio 17/17); janela real 45/45 por get/set/invoke em instância própria (`scripts/scene-export-audio-ui-proof.py`); códigos de job tipados (BL016.4); correções da revisão independente F1–F7/N1–N5 de BL016.6–.11 (grade de 48 kHz e tempo em `i128` sem estouro de `i64`, fonte que cubra a janela como a origem, PCM16 bit a bit, cópia privada provada no job com `done`/`total` na barreira, ganchos que falham ou entram em pânico, cabeçalho trocado = `resource_changed`, teto de 6 h ao planejar) **executadas com mutações** (gates de workspace e prova nativa repetidos no head integrado `31e0c34`, pós-`main` `141259e`: workspace 1263/0/9, M1–M9, nativa 45/45; ver BL016 e o doc); divergência declarada com a origem na duração da janela (BL016.11); aceite da S10 diverge na semântica de `editStartSeconds` (BL016.5: fonte decidida, emenda do oráculo pendente); narração (BL017) recusada |
| F043 | [Anexar cena pela interface](scene-import-ui.md) | Parcial | Parcial (prova viva 59 checks no head `f9898a0`: BL101 no monitor real sem pasta/workspace; BL100a: sem parada > 100 ms com 24 MiB; BL113a só na lib/host/processo, não na janela) | V (`scene.attach.start` no host de mídia; BL113a no processo `tessel-media` real via JSONL) | V (core + host + desktop com host real) | `flow.attach_scene`, `flow.scene_resources`, `scene_import`, `node:26`; BL100: leitura/parse/validação do bundle em job (`scene.attach.start`, `AdmittedScene`), aplicado só no projeto/sessão/revisão da partida (BL113a); BL100a: `prepare` mede tamanho/contagens/avisos e o apply é O(1); BL101: manifesto sem arquivos carrega sem pasta; testes core 5+11, host 20, processo 3, desktop 23  Reparo assíncrono declara changed:false e falha sem ID usa rótulo legível. |
| F048 | [Splat Spark numa cena dirigida (S7)](scene-splat.md) | Validado no corpus (gate combinado `e7fd75f`: workspace 1287/0/9, runtime 108/108, nativo S7 129/129 e S9 45/45; revisão Sonnet 5805273378 confirmou código e evidências; histórico: head `586a671`, a validar até a revisão independente final: runtime reconstruído, prova nativa **129/129** com árvore limpa, binário direto de Rust equivalente, só `op`/`get`/`set` semânticos; estado do WebView, não pixels da tela nem ponteiro físico; suíte runtime 108/108 numa execução e 107/108 noutra por corrida do teste de vídeo B2 alheio à PR (por inspeção do diff, sem reprodução na main), BL128; aceite independente S10 segue BLOCKED/FAIL, BL120/BL121; head combinado `12f9d48` com S9: só fmt/check/clippy/spec/sintaxe passaram, testes Rust, runtime e provas nativas ficaram sem execução completa por disco abaixo de 12 GiB, sem gate combinado) | N/A: sem verbo novo | V no corpus | V no corpus | `ScenePlan::splats`, `Capability::Splats`, recurso `spz` e matriz/câmera do avaliador; runtime só desenha (o core fixa o lugar do splat entre os planos por profundidade, `afterLayers`, ignorando plano atrás da câmera; só semelhança de matriz, o resto recusado: BL123; decodificação compartilhada e teto de splats por quadro: BL125; cor guardada, BL019-F1, também no caminho de projeto); luz/clima/rig recusados; pacote e export provados; ver [contrato](scene-splat.md). Timeline recompõe slots e confere matrizes por frame. |

¹ Dock/tema/janela são capacidades de apresentação. A lib valida layout, mas quem o executa
é o DockArea vivo; não há execução do Dock no core. UI nativa não é requisito para fluxos
de domínio. Se uma ação de apresentação ganhar efeito de domínio, documentar esse efeito
como comando independente do core.

³ Projeto/assets/tracks/clipes por `ui op` e API estão validados; não há diálogo de criação
de projeto na UI e miniaturas de assets gerados são placeholders. (Salvar/abrir/importar/exportar têm
diálogos: F019, F022, F026, F027, F029.)

⁴ V = import por diálogo (seletor nativo opcional) e por `ui op`, provado no fluxo completo da janela real
(`scripts/live-flow-proof.py`). Import, `media.prepare`, `project.verify|restore` e a sonda do preflight são **jobs** na janela
(a UI não espera); decode automático ao abrir/importar; VFR detectado por timestamps e conformado com aviso
(`variable_frame_rate`); o som embutido do vídeo entra no mix do playback e do export ([F029](video-export.md)). Gaps: o conforme VFR
é por asset (não há resample de áudio por clipe além do varispeed) e a responsividade da janela durante um job foi provada por
tempo de resposta e testes, não por medição de quadros da UI.

⁶ Playback tem prova na janela real: o **relógio de amostras do dispositivo** (CoreAudio via `cpal`, crate
`tessel-editor-audio`) conduz o playhead, com pause/seek/resume e relatório de dispositivo, latência estimada pelo host e
underruns (números no [relatório](../media-project-export-report.md)). **Não há áudio ouvido**: a latência é a estimativa do host
(o atraso acústico/Bluetooth não é observável, `heard:false`). Sem dispositivo o playback segue mudo no relógio de parede e diz
`audio_output_unavailable`. CLI sem janela é N/A para o relógio em tempo real; a **Lib** (V) prova o motor `Engine::fill`
sem dispositivo (resample, mapeamento de canais, seek, pausa, fim, 9 testes) e o plano de áudio que o playback usa. O gate de
diálogos esconde a WebView (provado por requisições semânticas); isso **não** prova foco/teclado sobre a WebView.

⁷ O seletor de sequências da timeline (`Select` do Kit) é provado por nó semântico e por teste do desktop
(`workflow_tests.rs`: lista, troca e mantém o trabalho de cada sequência); o popover aberto não foi inspecionado visualmente.

⁸ `./scripts/render` exporta com renderer próprio (Node+Chrome); `./scripts/headless` (sem renderer) responde
`backend_unavailable`.

⁵ V = validado na janela macOS real (WKWebView, `gpui-pre`) por `scripts/live-proof.py`
(56/56 checagens de pixel e estado sem `--shots`; 57/57 com ele; refeita no build debug com a retenção ligada e desligada) e por testes de host sem GPUI; imagem final ≠ áudio
processado ≠ áudio ouvido (**este último não existe**). Overlay da WebView por diálogos foi
resolvido (o WebView se esconde enquanto há diálogo; [F018](playback.md)). Não validados: `gpui-fast`,
GPU diferente da testada, foco por teclado sobre a WebView. Detalhes no
[relatório](../viewport-foundation-report.md). Vídeo em contêiner (contrato de tempo, `webm` não prometido), sequências
PNG/JPEG, `context_lost` no meio do quadro e scrub do ruler: [relatório de correções](../viewport-bugfix-report.md),
provados por barcode em Chrome, WKWebView e na janela real (`scripts/video-live-proof.py`).

⁹ Ferramenta de desenvolvimento, não uma capacidade do editor: UI, CLI da janela e CLI headless são
N/A (o runner é Node, não é o `tessel-headless`). Lib é Parcial: o consumidor `directed_scene_oracle.rs` compara os 55 vereditos,
referências pai/socket e matrizes estáticas. O consumidor temporal F033 também compara câmera
e gates; o consumidor F035 acrescenta animação de nós e matrizes. O contrato de dados tem 55 bundles e expectativas geradas por funções reais de `packages/scene-engine@d6adac15`,
manifest com hashes e validação Node (21/21 testes, repetidos pelo coordenador). Cobre validação,
grafo, avaliação e câmera; não cobre narração, clima, luz por keys, efeitos, rig, splat, I/O,
runtime nem pixel/áudio. Ver [relatório](../migration/foundation-b-report.md).

¹⁰ Importador BB01/BB02 puro, sem adapters CLI/UI. 29 testes próprios e 3 testes do oráculo
passam (baseline desta integração: 219 testes do core incluindo doctest). Somente um subconjunto do schema
é validado; as famílias adiadas e junções temporais podem conter erros aceitos na importação.
`edit_join_issues()` verifica apenas junções estruturais. Não equivale a `finalizeBundle`
completo nem habilita render/export. [Contrato e limites](directed-scene.md).

¹¹ Avaliação pura de câmera e tempo com 19 testes públicos, comparada ao oráculo nos 4 casos
válidos e em 9 variantes geradas pelo código fixado da origem. Gates combinados conferidos
nos 55 casos. Tolerância numérica `1e-9 + 1e-12·|x|`; frames/IDs/códigos são exatos. O teste
unitário adicional cobre abaixo/no/acima dos cinco limiares de junção (não é corpus da origem).
Animação de nós está integrada na F035; UI/CLI permanecem pendentes. Não é `finalizeBundle`
completo para domínios adiados. [Contrato](scene-temporal.md).

¹² 21 propriedades tipadas de nó/câmera (as 8 originais, `renderOrder` inteiro e 12 componentes
`X/Y/Z`), edição por ID, lote atômico, reset/no-op definidos e dry-run. `apply` e `dry_run`
executam importador e gates estruturais/temporais da F033; falhas não alteram a entrada.
Escrever `start/position` converte o primeiro movimento em keys como a origem, comparado
frame a frame com um oráculo fixo de 31 casos (24 originais e 7 acrescentados, gerados pela
própria `setProperties`). Testes: 69 de lib (29+33+7), 8 de sessão e 22 de
interação headless, além dos testes do inspector. **A validar** pelo coordenador, sem aprovação formal: os gates
foram executados pelo agente de testes no head de produção `cf21f97` (fmt, check, clippy `-D warnings`; os cinco
grupos acima com 29+33+7+8+22 passando; desktop `scene_`/`playback` 93; workspace com `TESSEL_REQUIRE_RUNTIME=1`,
Chrome real e lock de GPU: 1215 passaram, 0 falharam, 9 ignorados por medição/ZIP64/export; build e árvore do core sem
gpui/wry). A produção de `0d5304a` e a 33ª regressão passaram, e a regressão reprova quando a correção é desfeita.
Gestos usam o executável headless compartilhado e a janela real recebe o mesmo cenário em
`scripts/scene-edit-proof.py` (superfícies reportadas separadamente; socket da janela não é headless; 44 + 32 checks passaram na rodada final limpa). Sem ponteiro
físico nem pixels. Sem editor geral de keyframes; não certifica render/domínios adiados.
[Contrato, conversão e oráculo](scene-edit.md).

¹³ Avaliação de `cycle`/`event`/`static`, hierarquia e matrizes animadas, com 29 testes
públicos. Comparação com 4 casos válidos, 11 variantes e 20 payloads recusados gerados na
origem (tolerância `1e-9 + 1e-12·|x|`). JSON preserva bits de binary64 com
`serde_json/float_roundtrip`, conferido por regressão. Luz/clima/rig/pós e render continuam
fora da avaliação; `unevaluated` informa as seções adiadas. [Contrato](scene-animation.md).

Ciclo de vida de F021: perda forçada de contexto com proxy, ocultar/reabrir monitor, diálogo,
reload/troca de projeto com quadro em voo e export intacto têm prova específica no
[relatório](../viewport-lifecycle-report.md) e em `runtime/test/lifecycle.test.mjs`. A evidência
de janela usa macOS, uma tela e binário release existente, sem rebuild nesta frente.
`WEBGL_lose_context` não equivale a reset do driver; métricas Three.js não medem memória
física da GPU. Esses cenários não acrescentam comandos de domínio nem mudam o contrato
headless; os testes Chrome cobrem o runtime sem janela nativa.

## Critério de conclusão

- Documento da feature com contrato, exemplos, erros e limites atualizado.
- Regras no core, sem dependência do desktop; sem segunda implementação na CLI/UI.
- Fluxo de domínio via biblioteca e CLI em processo sem janela, com evidências.
- Integração UI verificada e limites de teste claramente informados.
- Matriz atualizada na mesma entrega. Não transformar pendências em status validado por inferência.

Veja [arquitetura](../core-architecture.md) e [template](_template.md).

¹⁴ Primeira fatia BB05: câmera, nós/matrizes, ordem, recursos por assetId e capabilities sobre evaluate_frame. Domínios não avaliados são recusados (ou plano parcial explícito). 29 testes públicos e doctest executados; clippy lib limpo. Não é render nem certificação do bundle completo. Ver [contrato](scene-plan.md).

¹⁵ Manifesto por assetId e materialização suportada (PNG/SPZ/mix) com preflight/rollback. 27 testes públicos core, 39 media e integração plano/manifesto passaram. Mapas/profundidade/relevo/narração apenas listados. Raízes precisam de controle exclusivo; PNG só cabeçalho. Guarda de pacote incompleto permanece. [Contrato](scene-resources.md).

Integração R9/F039: 11 testes visuais GPUI reconferidos pelo coordenador após ligar Anexar ao diálogo F043. O agente reporta 50 capturas nativas e 188 testes desktop; captura do novo cabeçalho e gate global pendentes ([relatório](../migration/next-r9-report.md), BL104).

Rodada integrada R1–R10: [relatório](../migration/round-r1-r10-report.md) com gate local, hashes, 23 fases de export, 218 checks de lifecycle, 33 de import e 50 capturas. Limites de cleanup: processo local (BL111), custo de hash no descarte (BL112). CI/revisão/merge são etapas separadas.

Entrega R1–R10 mergeada na [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), `c099f10`. Os estados V/Parcial da matriz descrevem cobertura por superfície; o encerramento dos itens está no backlog. CodeRabbit pulou a revisão, e o usuário autorizou o merge. Limites BL094/BL111/BL112 e domínios ainda não migrados permanecem abertos.
