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
| F005 | [Sliders, selects e switches de propriedades](properties.md) | I | I | I | V | `SetProperty` |
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
| F026 | [Empacotar e restaurar](package.md) | V | V | V | V | `project.package_preflight|package|verify|restore` |
| F027 | [Importar de outro projeto](interchange.md) | V | V | V | V | `project.import` |
| F028 | [Presets de exportação](presets.md) | V | V | V | V | `preset.list|save|delete|select` |
| F029 | [Exportar vídeo com áudio](video-export.md) | V | V | V⁸ | V | `export.preflight|start` |
| F030 | [Gaussian splats (Spark 2.4.0)](spark.md) | V | V | V | V | cena `tessel-scene@1` + asset `splat` |
| F031 | [Cena dirigida: importar, consultar e regravar render-bundle 1.0](directed-scene.md) | P | P | P | Parcial¹⁰ | `directed_scene::DirectedScene`; inteiros extremos recusados sem panic; adapters pendentes |
| F032 | [Oráculo de equivalência da migração (BB10/E0)](migration-oracle.md) | N/A⁹ | N/A⁹ | N/A⁹ | Parcial⁹ | `oracle.mjs verify` (55) + `extended.mjs verify` (47); 45 testes Node e 15 testes Rust estendidos passaram |
| F033 | [Cena dirigida: tempo, câmera e gate temporal (BB03)](scene-temporal.md) | P | P | P | Parcial¹¹ | `Bundle::evaluate_camera`, `evaluate_frame`, `DirectedScene::temporal_issues`; orçamento agregado de FOV; adapters pendentes |
| F034 | [Catálogo de propriedades e edição atômica de cenas (BB04)](scene-edit.md) | P | P | P | Parcial¹² | `scene_edit::{catalog, apply, dry_run}`; importação e gates compartilhados |
| F035 | [Animação de nós e matrizes de mundo (BB03)](scene-animation.md) | P | P | P | Parcial¹³ | `Bundle::{evaluate_frame, node_timeline, evaluate_instance_nodes, performance_plan}` |
| F036 | [Cenas no projeto, histórico e CLI](scene-project.md) | P | P | V | V | `Command::Scene` / `scene.*`; schema 2 com cenas, undo compartilhado, contagem de bytes em cache |
| F037 | [Plano avaliado de frame de cena dirigida (BB05)](scene-plan.md) | P | P | P | I¹⁴ | `scene_plan::ScenePlan`; plano tipado e serializável, gate Cargo pendente; sem renderer |
| F039 | [Interação com cenas dirigidas: seleção, árvore, frame e gestos](scene-interaction.md) | P | P | I | I | `scene.tree`, `scene.evaluate`, `scene.select`, `scene.selection`, `scene.gesture.*`; gate Cargo pendente |

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

¹² 8 propriedades tipadas de nó/câmera, edição por ID, lote atômico e dry-run, validados
por 29 testes públicos e 1 doctest via Cargo. `apply` e `dry_run` executam importador e
gates estruturais/temporais da F033; falhas não alteram a entrada. Ainda sem integração
com histórico/documento, CLI ou UI; não certifica render/domínios adiados.
[Contrato e divergências de escrita da câmera](scene-edit.md).

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

¹⁴ Primeira fatia BB05: câmera, nós/matrizes, ordem, recursos por assetId e capabilities sobre evaluate_frame. Domínios não avaliados são recusados (ou plano parcial explícito). 29 testes escritos, ainda não executados; não é render nem certificação do bundle completo. Ver [contrato](scene-plan.md).
