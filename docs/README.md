# Documentação do Tessel Editor

Comece pela [matriz de features](features/README.md): ela registra o contrato e o estado de
cada acesso pela UI, CLI com UI, CLI headless e biblioteca Rust.

- [Integração da rodada R1–R10](migration/round-r1-r10-report.md): export dirigido, inspector, planos e robustez; evidências e limites do gate final.
- [Próxima entrega — dez frentes](migration/next-delivery-ten-agents.md): contratos e ownership; status e delegações no backlog comum.
- [Integração M/N/O/P](migration/round-mnop-report.md): renderer, recursos portáteis, inspector e provas do fluxo dirigido.
- [Keyframes da câmera de clipe de cena (BL140)](features/camera-keys.md): chaves de posição/alvo/FOV com easing, tempo de cena, lib/JSONL/UI e export igual ao monitor.
- [PLY do SHARP direto, com a câmera da captura (BL143)](features/sharp-import.md): eixos OpenCV, FOV das intrínsecas, recusa de metadado ambíguo, câmera restaurável, pixels reais na janela e no export.
- [Renderer de cenas dirigidas](features/scene-renderer.md): scene.plan/scene.render, lib/JSONL e monitor com mapas verificados.
- [Integração H/I/J/K + sliders](migration/round-hijkl-report.md): 718 testes Rust, provas por superfície e limites atuais da base.
- [Inspector e árvore de cena](features/scene-inspector.md): controles recebidos, input GPUI e socket reportados; layout R9 inspecionado, captura final do cabeçalho em validação.
- [Integração de recursos de cena](features/scene-resource-integration.md): host, geração, pacote/restauração e intercâmbio; 22 testes direcionados reconferidos, renderer integrado; UI dedicada de recursos parcial.
- [Spark sob concorrência](features/spark-concurrency.md): diagnóstico da corrida do harness e gate determinístico de cancelamento; runtime intocado.
- [Backlog compartilhado](backlog.md): registro único de pendências, responsáveis, dependências e aceite; obrigatório para todos os agentes.
- [Inventário de migração do Scene Studio](migration/README.md): 121 itens classificados,
  evidências na origem, diferenças do branch piloto, matriz de migração e fatias sugeridas.
- [Recursos da cena dirigida](features/scene-resources.md): manifesto, preflight e materialização inicial; identidade por assetId, transporte parcial e pacote ainda protegido pelas guardas.
- [Interação de cenas dirigidas](features/scene-interaction.md): seleção, árvore, avaliação e gestos via comandos; gate local da rodada passou; inspector parcial em F039-UI.
- [Gizmo de translação no canvas](features/scene-gizmo.md) (F055, S8/BL015/BL042): overlay e arrasto no core, canvas dentro da WebView, `scene.translate` equivalente; implementado e a validar: workspace, clippy, Chrome e runtime executados no head de código `db02923`; build, prova nativa+headless com `final_evidence` e as mutações Rust executados depois, no head `fb268bb` (mesmas fontes de produção, um teste a mais; ver BL015.6), sem ponteiro físico.
- [Plano avaliado de cena dirigida](features/scene-plan.md): contrato core → runtime e recursos por assetId; gate local da rodada passou; consumido pelo renderer F041.
- [Cenas no projeto e CLI](features/scene-project.md): attach, lotes, histórico, save/reopen
  e JSONL sem janela; inspector e render dirigido integrados; diálogo de anexar pendente.
- [Animação de nós](features/scene-animation.md): performance por frame, hierarquia e matrizes
  animadas comparadas à origem; renderer/adapters ainda pendentes.
- [Propriedades e lotes de cenas](features/scene-edit.md): catálogo tipado, operações por ID,
  dry-run e validação atômica compartilhada; F036 liga projeto/histórico/headless; UI de cena pendente.
- [Tempo e câmera de cenas dirigidas](features/scene-temporal.md): avaliação pura, mapeamento racional
  e gate temporal; F035 avalia nós e F039 expõe avaliação headless; runtime/UI de cena pendentes.
- [Oráculo de migração](../scripts/migration-oracle/README.md): 55 bundles sintéticos, expectativas
  executadas na origem fixada e runner de integridade/equivalência; consumidor Rust parcial.
- [Integração da base A–E](migration/foundation-integration-report.md): resultados conjuntos,
  release e fronteiras do que ainda não está migrado.
- [Rodada paralela da base](migration/parallel-implementation-plan.md): ownership, dependências e gates.
- [Migração por building blocks e fases](migration/phased-roadmap.md): base atual reaproveitável,
  módulos compartilhados que faltam, etapas E0–E3 e fases funcionais M1–M8.

- [Arquitetura do core](core-architecture.md): dependências, execução programática e limites atuais.
- [Plano de mídia, Spark, projetos e export](media-project-export-plan.md): plano aprovado (P01–P12).
- [Relatório de mídia, projetos e export](media-project-export-report.md): status de P01–P12, arquivos,
  decisões, provas, métricas e lacunas. Setup de FFmpeg/Node/Chrome: [media-setup](media-setup.md).
- [Viewport multimídia e composição](features/viewport.md): contrato, CLI e evidências para imagem,
  vídeo, áudio (processado), texto e Three.js/splat.
- [Relatório da fundação da viewport](viewport-foundation-report.md): arquivos, APIs, comandos, métricas e lacunas.
- [Plano da viewport](viewport-foundation-plan.md): fatias e critérios (histórico).
- [Ciclo de vida do preview](viewport-lifecycle-report.md): testes de contexto perdido com proxy,
  ocultar/reabrir, cancelamento, troca de projeto e preservação do export.
- [Desempenho do preview: relatório de implementação](viewport-performance-implementation-report.md): retenção de GPU,
  spans do host e harness; comandos, arquivos, riscos, provas, lacunas e builds. Parecer e validação independentes:
  [revisão](viewport-performance-opus-review.md) e [validação](viewport-performance-opus-validation.md) (do Opus, sobre
  o snapshot anterior, não sobre este build). Números duráveis e proveniência:
  [viewport-performance-results](viewport-performance-results/). Erro de ritmo (~20 fps) do scrub:
  [seek-performance-report](seek-performance-report.md).
- [Pendências de timing e apresentação](viewport-performance-backlog.md): backlog consolidado,
  prioridades, critérios de aceite e ideias descartadas pelas medições.
- [Plano das próximas otimizações](viewport-performance-next-plan.md): execução OPT01–OPT10,
  dependências, políticas de preview, medições e limites operacionais.
- [Relatório da fundação](core-foundation-report.md): verificações e limites da extração.
- [Template por feature](features/_template.md): obrigatório para comportamento novo ou alterado.
- [Skill de implementação](../.agents/skills/tessel-feature-contract/SKILL.md): workflow obrigatório.
- [Automação da UI](ui-automation.md): socket e comandos da janela aberta.
- [Cargo e worktrees](cargo-worktrees.md): wrapper e cache.
- [Migração GPUI Kit](gpui-kit-migration-report.md): evidências históricas, não prova automática de features futuras.

Toda entrega de feature atualiza seu documento e a matriz na mesma mudança. Documentos de
plano/relatório continuam como histórico; o contrato atual fica em `features/`.
