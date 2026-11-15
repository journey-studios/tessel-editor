# P4b S2 — mapa de implementação para o próximo plano

Estado: pesquisa do coordenador, não é implementação nem uma fila de execução.
Backlog único: BL082.4.4 (S2), BL082.4.10 (S2A) e pendências .19/.20/.21/.23/.25.
Leitura sobre `33323a2`, com S3a integrada à main79 mas ainda em revisão.
Nenhum build, teste, runtime ou GPU foi executado por esta pesquisa.
O plano canônico continua em [p4b-cross-dissolve-plan.md](p4b-cross-dissolve-plan.md).

## Costuras confirmadas por leitura

| Área | Arquivo / entrada | Trabalho necessário |
|---|---|---|
| Capacidade | runtime/src/directed-scene.js, SCENE_CAPABILITIES | JS13 hoje; anunciar fxTransition junto do desenho funcional, não antes |
| Compatibilidade | crates/core/tests/scene_fx_transition/compat.rs | Teste exige 13 e ausência da capacidade; atualizar explicitamente e manter runtime antigo simulado sem fxTransition |
| Leitura | directed-scene.js, parseDirectedLayer | Extrair leitura de deck e novo parseTransition, erros do other qualificados |
| Recursos por deck | directed-scene.js, resolveSceneResources | Tabelas separadas por assetId, revisão/hash/generation e erro com deck |
| Desenho | compositor.js, prepareDirectedScene | Deck A/B com passes próprios, mistura e saída uma vez; não aplicar spec.fx inteiro nos dois |
| Cor | output-fx.js, SRGB_CURVES | Compartilhar IEC decode/encode com blend em luz linear, decks 8 bits e saída opaca |
| Alvos e materiais | compositor.js, sceneTarget/sceneMaterial | Chaves qualificadas por deck e mesmo limite de entradas/bytes; hoje material usa ds:order:alpha:index |
| Cache | compositor.js | Directed scene hoje redesenha sempre; sceneResultKey é cache de outra camada, não prova cache de deck dirigido |
| Lifecycle | dropFxPipeline/dropRetained/dispose; assets.js/splat-stage.js | Invalidar e liberar ambos os decks e blend, inclusive erro/cancelamento/context loss; provar leases e continuous |

## Quatro carregadores do host

Todos ainda leem `plan.resources`: SceneResourceMap::from_root e SceneSlots::resolve em
crates/render/src/scene.rs, approved_map em scene_operation.rs e resource_map do export em
scene_export/mod.rs. Migrar para ScenePlan::resource_union(), sem misturar IDs de nós entre
decks. A união é BTreeMap por assetId e tem usos qualificados por deck/node_id.

A adaptação não é uma troca textual: resolve devolve referências de PlanResource;
picture_bytes/load_message também recebem esse tipo, enquanto UnionResource é valor próprio.
Fixar interface adequada antes de alterar esses pontos. Contar usos de splat por deck,
compartilhando bytes/decode quando o recurso for o mesmo.

T10/T18 exigem asset exclusivo do other presente/ausente em cada carregador, IDs colidentes,
conteúdo divergente sob mesmo assetId recusado e erro que identifique o deck.

## Consumidor X obrigatório

Fonte: dissolve-representable-cases.json, sete bundles, 98 quadros e 390 sondas.
Usar frames[].primary/neighbour; os papéis trocam no corte. O mapa asset_resolution
traduz assets/<basename>.png para dissolve-representable-assets/, com hash do manifesto.

Comparar os dois decks por quadro: instance/camera frame, held, tempo racional real,
luzes, câmera e marcador. Comparar cores nas sondas prescritas; centre_px/rect_px são
somente diagnóstico. Não alegar hold de câmera por pixel quando a pose já saturou.
Não transformar os casos center/controle em isolamento de luz que não demonstram.

Antes da primeira execução do engine, decidir a granularidade do viés: o canônico fala
por quadro; a extensão pré-registra agregado por cenário com pelo menos 24 sondas únicas.
Preservar máximo de 1 código/canal e viés <=0,25; não calibrar tolerância após medir.

## Pendências de core antes da leitura externa

- .19: qualificar erros do other também em ScenePlan::build_with e PlanTimeline::build_with;
  validador/leitor já fazem isso. Preservar o erro do topo e testar os dois construtores.
- .20: o wire não carrega corte C/comprimentos; decidir testemunhos suficientes para validar
  primary e held antes de aceitar planos externos. Não atribuir essa garantia à S1b.
- .21: construir plano válido de exatamente 20.000 itens sob 4 MiB ou demonstrar impossibilidade.
  O pré-check de array não comprova o plano completo aceito.
- .23: primário legado conserva contador bruto em câmera curta (23), offset usa preso (11).
  Não alterar bytes de planos sem transição incidentalmente.
- .25: reparar dica da cauda nos argumentos estruturados e três línguas; 40/60/48/6 pede +34,
  não +6. Registrar divergência explicitamente, sem regravar corpus de origem.

## Export e ativação em S2A

Scene export snapshot::collect_required_assets amostra um quadro inicial por shot e lê
plan.resources. O preflight de mídia usa snapshot.required_assets(); a negociação atual
ocorre na submissão do renderer. S2A precisa provar a união de recursos e requires do
intervalo inteiro antes do primeiro frame, incluindo export parcial dentro da janela.
Esse trabalho já pertence ao aceite de BL082.4.10; não criar outra fila/ID para o mesmo gate.

## Marcos para detalhar após a revisão S3a

1. Regressões/contrato de core (.19/.20/.21/.23/.25) e consumidor de estado dos sete X.
2. União nos quatro carregadores, erros por deck e tetos, com testes sem GPU.
3. Parser/desenho completo/blend/saída única, capacidade e teste compat no mesmo marco.
4. Cache de decks separado do cache de mistura, lifecycle, falhas, splats e continuous.
5. X completo e export bruto: inteiro versus subintervalo iniciado dentro da janela.
6. S2A separada: intervalo completo e inversão dos negativos públicos somente com todos os gates.

Todos os marcos S2 conservam Refuse público fechado. Estabilizar a revisão S3a antes
as alterações de core compartilhadas para manter ownership claro. Não foi lido integralmente
o transporte do host, assets.js, splat-stage.js ou todos os caminhos de export; o plano
executável deve completar essa leitura e fixar os testes e ownership por arquivo.
