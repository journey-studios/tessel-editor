# Próxima fase: cena dirigida de ponta a ponta

Rodada autorizada pelo usuário: quatro agentes Cursor Sonnet 5.5. Base: main 192e766.
Status de execução e pendências somente em docs/backlog.md. Este arquivo define contratos e ownership.
Checkout compartilhado por limitação do projeto T3 pai; nenhum agente cria checkout ou faz Git write.
Coordenador integra exports, dependências, matriz, backlog e arquivos de wiring compartilhados.

## Contrato comum

- ScenePlan versionado existente é a autoridade de câmera, tempo, matrizes e ordem. Não reavaliar regras em JS.
- ResourceManifest identifica assets por assetId, nunca por caminho. Resolver em adapter de mídia/render, sem I/O no core.
- Mapa de recursos do runtime vincula assetId a URL local aprovada + identidade de conteúdo/revisão e geração do projeto. Nunca usar apenas assetId como chave global de cache.
- Rejeitar plano incompleto/capability ausente e recurso não resolvido com erro tipado. Não habilitar domínio recusado sem implementar.
- Cancelar, trocar projeto, trocar asset, reload e perda de contexto invalidam recursos/frames antigos. Ownership explícito e release; orçamento de memória limitado.
- Caminhos relativos pertencem à base de origem explicitamente registrada; pacote restaurado usa a própria raiz. Nunca depender de cwd nem copiar dados privados.
- Comandos scene.* existentes são reutilizados. Novo estado de domínio fica no core; configuração de preview é apresentação. Compatibilidade aditiva com bridge/projeto atual.
- Manifesto e plano mantêm APIs existentes. Alteração inevitável de contrato é proposta ao coordenador antes de quebrar consumidores.

## Ownership

| Frente | Escopo | Arquivos exclusivos | Não editar |
|---|---|---|---|
| M renderer | BL007/BL063, preview/headless da cena dirigida suportada | runtime/src, crates/render/src, novos testes runtime/render; docs/features/scene-renderer.md; relatório M | media, desktop, core compartilhado |
| N recursos | BL010/BL011, carregamento e transporte real | crates/media/src e testes media; novos módulos core/scene_resource_session se necessários; docs/features/scene-resource-integration.md; relatório N | render, desktop, runtime, directed_scene/scene_plan |
| O inspector | BL014/BL021, árvore/seleção/controles reais | crates/desktop/src e testes desktop; docs/features/scene-inspector.md; relatório O | core, media, render, runtime |
| P provas | BL019, fixtures e harness de integração | scripts/directed-scene-proof/, novos testes de integração com prefixo scene_flow; docs/migration/directed-scene-proof.md | fontes de produção, fixtures do oráculo existentes |

Todos entregam patch sugerido para arquivos compartilhados: core/lib.rs, core/ops.rs, scene_project, document, Cargo.*, matriz, backlog, índices. O coordenador aplica, evitando escrita concorrente.
M entrega seam de monitor documentado para O/coordenador. N documenta seam de recursos para M/coordenador. P começa com contratos existentes e deve declarar bloqueios reais, nunca simular sucesso de renderer inexistente.

## Aceite

Cena sintética com imagens em camadas e câmera/animação: anexar, resolver recursos, consultar frame, render headless e monitor, selecionar/editar com undo único, salvar/reabrir, empacotar/restaurar sem origem. Domínios ausentes continuam unsupported.
Export dirigido BL008 é passo dependente após renderer e recursos; nesta rodada preparar seam e teste, não afirmar entregue antes de rodar o fluxo.
Pixels/provas headless, socket nativo, UI semântica e ponteiro físico têm evidências distintas.

## Execução

Ler AGENTS e skill tessel-feature-contract. Sem subdelegação, Git write, push, PR ou alteração da origem tesselapp. Não encerrar nem reiniciar app do usuário.
Cargo somente via /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py, que serializa ./scripts/cargo com cache; nunca trocar HOME em wrapper. Se helper ausente, entregar bloqueio ao coordenador, não compilar em paralelo.
Cada agente pode rodar testes proporcionais serializados; não rodar quatro gates completos. Node/Chrome e provas GPU uma de cada vez usando o mesmo lock de bancada acordado pelo coordenador; não matar processos alheios.
Temporários próprios até 2 GiB e piso de 6 GiB livres; sem clean/gc manual. Relatar exatamente testes executados, não executados e comandos de integração.
