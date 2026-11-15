# P4b S2-R1a — extrair o desenho atual sem alterar comportamento

Contrato pré-código, 2026-11-15. Base main `7d88545320fd8466812c649a1877c6f0362493ae`
(PR75/R0 integrada). Backlog único BL082.4.4; F041/F064. Atribuição/status ficam no
backlog. Este plano autoriza apenas R1a, não R1b/c, desenho de transição ou S2A.

Salvo e commitado antes de qualquer mudança de código. Resultado e evidências por SHA:
[p4b-s2-r1a-draw-extraction-report.md](p4b-s2-r1a-draw-extraction-report.md).

## Objetivo e fronteira

Reduzir o módulo compositor antes de acrescentar o desenho de dois decks. Extrair
`prepareDirectedScene` e `sceneMaterial` do runtime atual para um módulo coeso novo,
sem alterar suas operações nem a ordem em que acontecem. O caminho produtivo de
planos simples passa pelo helper extraído e continua desenhando a mesma imagem.
Planos com transição continuam recusados. Não entregar código de transição dormente.

R0 foi aceita internamente: produção a3c357b, Windows R2 e Mac separados; fechamento
cd7d18b só docs/testes, squash7d88545 idêntico por árvore. JS13/Rust14 e Refuse público
permanecem, F064 produto P. Os limites JS/core e de objetos ativos da R0 continuam
registrados; esta extração não deve consertar nem ampliar esse contrato.

## Decisões fixadas antes do código

1. Novo `runtime/src/directed-deck-draw.js` (nome ajustável por clareza antes do código),
   com funções que recebem o compositor dono como argumento (`host`) e os parâmetros
   originais. A classe mantém os métodos e assinaturas `prepareDirectedScene(layer,
   frame)` e `sceneMaterial(order, alpha, index, texture, keep)` como wrappers finos.
   Preservar o método async e a propagação de sucesso/recusa; não expor bridge/debug/API.
2. Corpos movidos mecanicamente, com substituição de `this` por `host` onde necessária,
   mantendo callbacks arrow, captura de variáveis, awaits, retornos e try/catch. Nada de
   limpeza semântica, nova abstração genérica, reordenação de preflight ou otimização.
   Nomes locais/imports podem mudar somente por causa da extração, registrados no relatório.
3. Imports diretos de `imageTexture` (assets.js), dos helpers dirigidos já existentes
   e RuntimeError (protocol.js); não importar Compositor de volta no helper nem criar
   ciclo de dependências. Usar a mesma instância ESM dos módulos, sem estado global novo.
4. Preservar parseDirectedLayer e resolveSceneResources, registro `used`, aborts,
   containSize, targets, buildDirectedScene, frameSplats, leases, stage/draw, FX e
   telemetry exatamente na ordem atual, inclusive a recusa antes/depois de cada ação.
   Os stacks podem apontar ao novo módulo; code/message/details/primeira recusa não mudam.
5. Preservar chaves `ds:<order>`, `ds:<order>:<alpha>:<index>` e
   `dsfx:<order>:<slot>`, tamanhos, MSAA, depth, contadores/cache, formato da textura
   devolvida e ausência/presença dos campos de sources. Não introduzir cache de deck.
6. Preservar a posse e o momento do descarte. `frame.keep` continua guardando geometria,
   materiais/texturas temporárias e leases até o fim do frame, em sucesso/erro/cancelamento.
   Não liberar lease ao terminar um deck nem acrescentar um finally novo por suposição.
   `discardFailedStage`, dropFxPipeline e erros continuam iguais. A proposta futura de
   liberar entre decks não faz parte desta extração e ainda exige prova de continuous.
7. `sceneMaterial` mantém a regra de retenção atual, reuso/criação, atribuição de map e
   chaves. Não qualificar por from/to ainda; isso pertence ao desenho novo e aos seus testes.
8. Nenhuma mudança em shader, cor, SRGB_CURVES, catálogo, limites, hello/capacidades,
   router, parser interno de transição, desenho/cache/working set, assets/splats, transporte,
   export ou documento. Em especial SCENE_CAPABILITIES continua13 e não anuncia fxTransition.
9. Comparar os corpos antigos/novos de forma verificável, normalizando apenas as diferenças
   mecânicas declaradas (assinatura/indentação/this→host). Registrar base/hash/contagem de
   linhas e o resultado; isso não substitui testes de imagem já existentes. Não comparar
   stacks como se a localização de fonte pudesse continuar idêntica após mover o código.
10. Não escrever testes que apenas espelham o helper novo. Usar os controles existentes
    pelo compositor/runtime real, mais uma prova de extração se ela discriminar mudança
    de ordem/telemetria/lifetime. Não regenerar golden, fixture ou baseline no candidato.

## Ownership

- Novos `runtime/src/directed-deck-draw.js` e relatório/plano R1a próprios.
- `runtime/src/compositor.js`: imports e wrappers mecânicos; delta de linhas <=0 obrigatório
  porque já tem mais de1000linhas. O módulo novo deve ficar bem abaixo de1000linhas.
- Testes runtime de desenho dirigido somente se necessários para um controle novo relevante;
  preferir executar os atuais. Não relaxar expectativas/counters/pixels/guardas existentes.
- Documentos `docs/features/scene-renderer.md` e `fx-transitions.md`: nota de implementação
  e evidência da extração, sem mudança de status de produto. Este plano e relatório novo.
- Backlog/matriz/índice são do coordenador: fornecer patches literais para BL082.4.4,
  F041/F064 e docs/README.md, sem editar arquivos comuns nem criar IDs.
- Intocados: core/src, media, desktop, render/src, runtime assets/splat-stage/retention/
  directed-scene/transition/output-fx/bridge/worker, shaders, Cargo.*, package/pnpm lock,
  dados S0/X/PNG/manifesto/golden/baseline e origem. Um import removido por não ser mais
  usado no compositor é a exceção mecânica prevista, não nova semântica.

## Gates e evidências

1. Confirmar Windows nativo, projeto preparado E:\www\tessel-p4b-s2r1-20261115,
   main7d88545, origin, status limpo, espaço>=16GiB e memória antes de trabalho pesado.
   Ler AGENTS, skill de contrato, backlog, índice/matriz/arquitetura, R0 e plano canônico.
   Salvar e commitar este plano antes de mover código. Não criar clone/worktree/checkout.
2. Commit mecânico próprio, depois gates/relatório. Não misturar desenho de transição.
   Se aparecer defeito preexistente, registrar como observação para o coordenador, sem
   transformar esta PR em correção incidental. Nunca mudar comportamento silenciosamente.
3. ./scripts/runtime setup (pnpm frozen) e build oficial antes dos testes de browser,
   evitando usar dist antigo. Node grupos dirigidos: directed-scene, light, splats,
   output-fx, fx-handles-overlay, transition (quatro arquivos). Base:195 testes no grupo
   de nove arquivos;120+1 são puros de transição, os outros incluem browser. Executar
   também os controles browser FX/fade/grain e cena dirigida pertinentes à extração.
   Usar TESSEL_REQUIRE_RUNTIME=1 quando a prova exigir runtime/Chrome, sem skip verde.
4. Comparar resultados e contadores dos planos simples nos controles existentes:
   sem FX, shot+output, light, splat/lease, erro/cancelamento e retain on/off. Não mudar
   a tolerância prévia nem dizer que uma comparação de texto comprova pixels.
5. Rust pelo ./scripts/cargo, defaultmbx: fmt/check/clippy workspace-all-targets locked
   -Dwarnings, no-run e build desktop; core216/corpus10/Ximport5/Xstate8/baseline1
   isolado EXPECT0 e preserve_order EXPECT1, interop5. Executar controles render de
   ScenePlan/FX que exercitem o runtime novo, com dependências reais e guarda obrigatória.
   Core/src intocado: não é preciso repetir core completo/ledger/stress/áudio. Workspace
   global não está declarado verde; dois gizmos Windows e shasum de BL167.3 são separados.
6. Nenhum benchmark/FPS, GPU física, áudio ouvido, UI física ou engine da origem. Chrome/
   SwiftShader funcional é evidência de browser do caminho antigo, não pixel de transição.
   Não obter lock de desempenho para teste funcional nem herdar prova Mac do coordenador.
7. Provas por SHA: extração e corpo/hash, comandos/exits/contagens, executado vs leitura,
   N/A, imports/ownership, limites e patches dos arquivos comuns. Não somar baterias de
   mutantes de outras fases; extração sem regra nova não exige inventar mutantes redundantes.
8. Sem CARGO_TARGET_DIR/target-dir/lane, clean/gc manual ou artefatos tmp>500MB; preservar
   processos e TEMP alheios. Skill commit, mensagens convencionais curtas, linhas<=100,
   sem assistentes/trailers. Sanear somente commits ainda não publicados, sem force-push.
9. PR contra main, push normal; link_pull_request como próxima ferramenta após criar/push.
   Não watch/poll/merge. Última linha PR URL, head completo e checks/reviews vistos uma vez.
   Revisão independente e Mac são do coordenador depois da entrega, não aprovação herdada.

## O que o levantamento não autorizou para R1a

- A proposta de nove alvos é por camada de transição, não limite global. targetBytes é
  uma estimativa de política, não contabilidade exata de attachments/VRAM. A proposta
  de1GiB para working set fica pendente de contrato R1b/c; não entra nesta PR. targets2/
  192MiB atuais são flexíveis, pois trim preserva usados. Não chamar isso de teto rígido.
- Cache T31 precisa reter a textura final pós-shot-FX (incluindo paridade ping-pong), não
  só o alvo MSAA. Em4K esse alvo MSAA excede192MiB e fica transitório. A política de reuso
  por elegibilidade e a memória global ainda precisam ser fixadas antes do código novo.
- readOtherDeck já recusa fxTransition e checkDepth já recusa nesting. Adicionar14 à lista
  atual sozinho não abriria desenho: guardas ainda recusam e TRANSITION_CAPABILITIES
  duplicaria o nome. Router/capabilities precisam vir com desenho funcional, não nesta extração.
- Não impor>=24sondas/canal/quadro ao X fixado. No futuro desenho, aplicar max1código/canal
  e viés absoluto<=0,25 sobre TODAS as sondas prescritas de CADA quadro; agregado por cenário
  é só adicional. Grids extras podem ser pré-registrados para outros casos, sem substituir
  nem calibrar o acervo. Nenhum pixel de transição será executado nesta PR.
- O tracer f37 está certo: p=4*(3,5/12)^3≈0,099248; smoothstep(p)≈0,027595. Progresso no
  plano e peso efetivo no shader são fatos diferentes. Não corrigir números corretos.


## Conferência de leitura do coordenador

Duas leituras independentes confirmaram R1a como extração coerente, sem executar código.
Para a etapa seguinte: resultados de camada vivem até o composite/finally externo;
texturas e dados compactados pertencentes ao AssetStore não são recursos descartáveis
de um deck. targetBytes omite attachments de depth, portanto sua soma não é limite
físico de VRAM. Mesma instância/arquivo não prova um só sort: transforms, câmera, painter
placement e mapping precisam permitir o reuso; devolver uma lease antes não demonstra
essa propriedade. Nenhuma dessas observações muda o comportamento autorizado na R1a.
