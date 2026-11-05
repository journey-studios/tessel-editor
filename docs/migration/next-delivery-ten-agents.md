# Próxima entrega — dez frentes Cursor Sonnet 5.5

Autorizada pelo usuário em 2026-11-05. Base: main 914c830 (PR #3 mergeada), branch feat/directed-scene-next-delivery.
Fonte única de estado/pendências: docs/backlog.md. Este arquivo define apenas contratos, ownership e aceite.
Checkout compartilhado por limite do projeto T3 pai; não criar worktrees/clones nem mudar o projeto pai. Só o coordenador integra e faz Git write.

## Contratos congelados entre frentes

1. ScenePlan e ResourceManifest existentes continuam compatíveis. Core define domínio, preset/tempo/limites; mídia faz I/O/FFmpeg/jobs; renderer executa; UI/CLI traduzem intenção. Nenhum GPUI no core.
2. Recurso é assetId dentro da cena, com geração/revisão/hash verificados pelo MediaHost. Não introduzir cache global por assetId nem confiar em caminho arbitrário.
3. Export dirigido reutiliza plano completo e original, mapa aprovado e presets existentes. Snapshot imutável no início, intervalos [from,from+frames), tempo racional, job/progresso/cancelamento e staging/publicação reutilizáveis. Comando alvo: scene.export {scene,preset,destination,from,frames,wait?}; campos adicionais só aditivos e documentados. Não presumir áudio dirigido implementado: transportar/mixar só quando real, caso contrário erro/limite explícito, sem descarte silencioso.
4. UI de anexar usa scene.attach e scene.resources.adopt/resolve; escolher arquivo é só entrada, mesma operação programática. Diálogo passa por Workflow::has_dialog para ocultar WebView. Export UI usa o contrato acima quando integrado pelo coordenador.
5. Otimização conserva saída/erros/capabilities. Medir antes; não refazer NodeTimeline por frame se o experimento provar ganho. Preservar APIs atuais e propor novo PlanTimeline aditivo para R1; nenhum consumidor pode depender de otimização ainda não entregue.
6. Interação mantém um undo por gesto, preview isolado, commit/cancel e invalidação. IDs antigos não atingem sessão/projeto novo. Não adicionar seleções/tema ao documento sem necessidade.
7. Erros/cancelamentos só removem arquivos próprios. Pacote anterior sobrevive à falha de replace. Lease continua cooperativo; sem alegar defesa contra escritor hostil.
8. R10 usa expectativas independentes e formatos reais; operação ausente bloqueia, nunca passa. Inspeção de canvas/socket não é prova de ponteiro físico/fóton/áudio ouvido.

## Ownership exclusivo

| Frente | Itens | Arquivos que pode editar | Documento da feature |
|---|---|---|---|
| R1 Export dirigido | BL008 | novos core/src/scene_export/, render/src/scene_export/, media/src/scene_encoder/ se necessário; render/src/scene_host.rs e bin/tessel-render.rs; testes próprios scene_export* | features/scene-export.md (F042) |
| R2 Anexar pela UI | BL086 | desktop/workstation/scene_import.rs novo, flows.rs, workflow.rs, workflow_tests.rs, desktop/header.rs se preciso, testes scene_import* | features/scene-import-ui.md (F043) |
| R3 Sessão/gestos | BL066–BL069 | core/scene_interaction/* e scene_project/interaction.rs (exceto substituir scene_plan: patch coordenador), testes scene_session* | features/scene-interaction.md (F039) |
| R4 Replace de pacote | BL088 | media/package.rs, testes package* e novo package_atomic* | features/package.md (F026) |
| R5 Limpeza de jobs | BL089 | media/scene_resources/import.rs e adopt.rs, novo módulo cleanup se necessário, testes scene_import_cleanup* | features/scene-resource-integration.md (F040) |
| R6 Plano/performance | BL064 | core/scene_plan/* e directed_scene/animation ou temporal se medido/necessário, testes scene_plan_performance*, novo benchmark | features/scene-plan.md (F037) |
| R7 Spark concorrente | BL091 | runtime/test/splat.test.mjs, test/support.mjs, novos test/splat-concurrency* e scripts/splat-concurrency*, src/splat-stage.js se causa comprovada | features/spark-concurrency.md (subcontrato F030/F021) |
| R8 Lifecycle nativo | BL092 | desktop/scene_monitor_host.rs, viewport_host.rs, viewport.rs, scene_monitor.rs, novos testes scene_lifecycle* e scripts/scene-lifecycle* | features/scene-renderer.md (F041) |
| R9 Layout/inspector | BL087 | desktop/scene_panel.rs, scene_model.rs, scene_gesture.rs, scene_tests.rs e novos testes scene_visual*; scripts/scene-visual* | features/scene-inspector.md (F039-UI) |
| R10 Prova independente | BL019/BL008 | scripts/directed-scene-export-proof/* e novos testes render/tests/scene_export_proof* | migration/scene-export-proof.md |

Caminhos abreviados: crates/{core,media,render}/src, crates/desktop/src/workstation e respectivas tests/.
Cada agente escreve relatório próprio docs/migration/next-rN-report.md, sem editar relatórios alheios.
Coordenador é dono de Cargo.*, core/ops.rs, core/headless.rs, desktop/ops.rs, scene_ui.rs, matrix/backlog/índices e harness compartilhado scripts/directed-scene-proof/. Exceções exclusivas para testes possíveis: R1 pode registrar seus novos módulos nos lib.rs de core/render/media e é dono de media/host.rs nesta rodada; R2 pode acrescentar somente a declaração mod scene_import no desktop/mod.rs; R3 é dono de core/state/{mod,history}.rs e core/document.rs se precisar de wiring de sessão/summary. Nenhum outro agente edita esses arquivos.
Qualquer wiring nesses arquivos é entregue como patch preciso ao coordenador. R2 não edita scene_panel (R9); R8 não edita workflow (R2); R6 não edita scene_project/interaction (R3); R5 não edita media/host: entrega patch ao coordenador para aplicar depois de R1. Não corrigir falha de compilação de arquivo alheio.

## Dependências e aceite

R1 funciona com a API atual; pode consumir PlanTimeline de R6 somente após integração. R2/R3/R4/R5/R7/R8/R9 têm trabalho independente. R10 começa por corpus/expectativas/controles negativos e finaliza export real após R1; R2 recebe o botão/diálogo de export via coordenador depois de R1.
Meta: anexar/carregar pela UI, editar, transportar/restaurar sem origem e exportar vídeo curto em 1080p/4K, também por lib e CLI. Não declarar todo motor migrado nem suportar domínios recusados sem implementá-los.

## Execução e recursos

- Ler AGENTS.md, skill tessel-feature-contract, docs/README, matriz, arquitetura e backlog. Não subdelegar. Sem Git write/push/PR pelos filhos; coordenador mantém uma PR.
- Cargo só por python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py. Helper usa scripts/cargo; se ausente, avisar coordenador. Sem HOME alterado em wrappers, clean/gc, reinstalação ou alteração de dependências sem coordenação.
- GPU/native/Chrome: python3 scripts/directed-scene-proof/with-gpu-lock.py COMMAND...; adquirir GPU antes de Cargo quando ambos forem necessários. Nunca executar quatro gates completos por agente; testes proporcionais, gate global pelo coordenador.
- Instâncias nativas próprias, HOME/socket curtos, binário direto; não operar/reiniciar janela/projeto do usuário. Entrada semântica, sem cliques por coordenadas. Captura nativa permitida; falha de permissão fica como limite, nunca imagem simulada.
- Temporários próprios prefixados te-rN, no máximo 2 GiB por frente e piso de 10 GiB livre. Só apagar seus próprios artefatos e encerrar seus próprios processos.
- Entregar implementação, comandos, testes executados/não executados, limites, seams e patches para matriz/backlog. Estado de execução só no backlog, não criar fila paralela.
