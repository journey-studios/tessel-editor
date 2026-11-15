# P4b S2 — pré-requisitos do core e ordem de implementação

Estado: plano do coordenador para Cursor Sonnet 5.5 no Windows, sem opções extras.
Base mínima: main `9073e775e0014b3927dfde7ff0a76d8eb4ead2ce` (PR66/S3a aceita).
Backlog: BL082.4.4 (S2), com .19/.20/.21/.23/.25/.29 e cobertura restante .26–.28.
Contrato canônico: p4b-cross-dissolve-plan.md; F064. Pesquisa: p4b-s2-implementation-notes.md.
Este documento detalha execução; status e atribuições continuam exclusivamente no backlog.

## Objetivo e fronteira

Preparar o core para a futura leitura/desenho de dois decks e tornar os diagnósticos acionáveis.
A primeira entrega é **S2-C0a**, abaixo. Não implementar as outras partes nessa PR.
Todas as entradas públicas continuam Refuse; F064 produto P. Sem runtime, capability JS,
shader, render real, UI, export ou alteração da origem. Nenhum Domain novo.

S3a foi aceita no escopo interno: Windows R2 no código b097979/head512685a; fechamento
9158c4f só docs; squash9073e77. Mac1673/0/7 e Windows175/10/5/1 são provas separadas.
Não declarar workspace Windows verde: os dois gizmos da base permanecem vermelhos.

## Sequência da S2

| Fatia | Conteúdo | Pré-requisito | Resultado revisável |
|---|---|---|---|
| S2-C0a | Diagnósticos de construtor (.19), reparo de cauda (.25), identidade de alvo (.29), cobertura .26–.28 | S3a aceita | PR de core, zero ativação pública |
| S2-C0b | Wire coerente (.20/.23), plano válido de 20000 itens (.21), consumidor de estado dos sete X (.17) | C0a | Contrato fixado antes do parser externo, bytes sem transição mantidos |
| S2-H | União nos quatro carregadores e erros por deck | Wire C0b | Host com recursos exclusivos do other resolvidos/recusados corretamente |
| S2-R | Parser, desenho, blend, capacidade, cache/lifecycle/splats e pixels/export bruto | Wire C0b; H para aceite renderer/export | JS e renderer completos ainda atrás de Refuse |
| S2A | Preflight de intervalo e inversão dos negativos públicos | S2 inteira + S3a aceitas | PR própria de ativação, sem pressupor aceite de UI |

H e partes JS de R podem ser paralelas com ownership separado depois de C0b. Seu aceite
integrado não é substituído pelos testes de um lado. Não anunciar fxTransition no JS antes
de desenhá-la; compat.rs do core hoje exige JS13/ausência e precisa mudar junto do runtime.

## S2-C0a — decisões fixadas antes do código

1. Regras, reparos e atribuição pertencem ao core. Adapters não montam diagnósticos.
2. Continuar o mesmo pipeline FxOp/apply_fx, um candidato, uma reimportação, atomicidade,
   no-op autorado, revisão e undo. Não criar um editor paralelo para as correções.
3. Não adicionar Accept em produção, flag de ambiente ou porta de teste em binário público.
4. Não regravar nenhum fixture/oráculo existente, Python, PNG, manifesto ou snapshot da origem.
5. Problemas do vizinho são identificados como other; problemas do topo conservam contrato.
6. Dica de reparo e números estruturados devem concordar. Aplicar o reparo em um fixture
   deve tornar a admissão válida; conservar apenas uma mensagem bonita não fecha o aceite.
7. Uma entidade removida e outra criada com o mesmo ID são identidades distintas, mesmo
   dentro do mesmo tipo. Não confundir isso com rastreamento causal por propriedade.
8. Testes defensivos de helpers são identificados como tal. Não convertê-los em prova de
   operação pública impossível hoje. Continuam sem gesto/UI de transição.
9. Decisões de API e divergências concretas são documentadas antes do código correspondente;
   escolhas rotineiras dentro destas regras não exigem nova confirmação do usuário.

### Marco A — cobertura permanente das ressalvas S3a

- .28: casos de cameraEnd presente com string, negativo, fração e null preservam mensagem/hint
  originais e marcam untranslated; ausente e zero presente permanecem válidos. Cobrir também
  contagem obrigatória com 3.0. Os casos instance/camera da S3a não podem ser enfraquecidos.
- .26: provar que a restauração de autoria não atravessa fromShotId/toShotId/effect distintos.
  Um caso real pode remover X num corte e recriar X em outro corte com intensidade tipada
  igual: a entidade nova conserva sua nova representação. Effect diferente é defensivo,
  pois só há uma transição suportada; usar teste de helper claramente identificado.
- .27/E13: após oito looks + criação de transição, provar frame dentro da janela e respostas
  de begin/update/status/inspect de gesto de look com orçamento 9/over_budget, sem cortar
  passes. Fora da janela o frame tem oito passes. A forma da resposta de frame é diferente
  da resposta budget; comparar fatos públicos, não inventar campo que não existe.
- Manter add/remove/dry-run/undo/list/inspect já cobertos e os dois mutantes R1 mortos.

### Marco B — .29, identidade ao atribuir recusa do candidato

O código atual retém tipo clip/transição, mas resolve pelo ID final. remove_transition X
seguido de set_transition id=X inválido ainda atribui ops[0,1] e index0. O correto é op1.

- Representar identidade por tipo/ID e geração dentro do lote, ou outra solução equivalente
  explicitamente justificada. Manter a implementação privada ao pipeline de edição.
- Operações sobre a entidade antiga não contribuem para erro da entidade nova. Preservar
  contribuição das múltiplas ops sobre a mesma entidade que continua existindo.
- Não basta excluir somente a op remove: set_param antigo → remove → recria também precisa
  descartar a contribuição do set_param antigo. Não basta olhar apenas a última op do lote.
- No-op não deve ganhar contribuição. Não alterar códigos de erro nem o formato público
  de changes[]/details.issues/errors[] sem uma necessidade documentada.
- Testar transição→transição, clip→transição e transição→clip, com erro posterior que seja
  realmente do candidato. Se alguma falha de look só puder ser julgada na própria op, testar
  esse caminho como tal e justificar N/A para um candidato impossível; não fabricar acesso.
- Provar documento, revisão e histórico intactos na recusa e preservação do index correto.

### Marco C — .19, erros do vizinho no construtor e timeline

Entradas: ScenePlan::build_with em scene_plan/build.rs e PlanTimeline::build_with/plan_of.
Validador e leitor já qualificam other por within_other; os construtores ainda usam ?.

- Reutilizar uma única regra de qualificação, sem copiar mensagens em cada chamador.
- Qualificar falhas de avaliação/preparação, domínio e splat que realmente nasçam do other.
  Preservar pointer quando já existe e prefixar /transition/other uma única vez.
- O erro do topo mantém código e forma. Para erro do other, alinhar categoria/pointer ao
  contrato já existente do leitor/validador e documentar a diferença do construtor antigo.
  Não inventar um Domain novo para obter um erro intermediário.
- Provar ScenePlan e PlanTimeline com nó espelhado só no vizinho e IDs de nó colidentes.
  Um mesmo erro no topo não pode receber deck=other. Atingir cada construtor por sua API.
- Não abrir a admissão nem fazer bypass no renderer para produzir as cenas de teste.

### Marco D — .25, reparo suficiente da câmera de saída

Caso discriminante: cameraIn40, câmera60, shot48, after6. Estender6 continua recusado;
94 quadros de câmera (+34) aceita. Fórmula do reparo no fim:
max(0, cameraIn + shotDuration + after - cameraFrames), somente se after>0.
Usar aritmética verificada e regras únicas de alcance; sem saturar um overflow em sucesso.

- Distinguir quantidade de quadros pedidos ausentes da extensão física necessária. O campo
  legado camera pode continuar representando falta de janela, mas o reparo deve ter um
  número estruturado explícito e nome inequívoco, consumido pelo hint nas três línguas.
- Não reutilizar cameraEnd da cabeça com outro significado silencioso. Fixar o contrato de
  qualquer novo campo em F064 antes de implementá-lo.
- Cobrir cameraIn+shotDuration menor/igual/maior que total, after0, câmera curta, ambos os
  defeitos de cabeça e cauda, handles use/refuse e hold, e valores extremos recusados.
- Aplicar o reparo indicado e reimportar: +34 aceita e +33 recusa no vetor principal.
- O corpus tem linhas que pinam o diagnóstico antigo. Registrar ANTES uma lista exata dos
  casos afetados e seus vetores esperados do editor, com a frase antiga exigida literalmente.
  O consumidor pode aplicar uma divergência explícita de diagnóstico; todos os outros campos
  e casos continuam comparados. Não usar replace/regex genérico que enfraqueça o oráculo.
- Não editar os JSON/Python de origem para esconder a divergência. A fórmula de pixels,
  janela, tolerâncias e provenance origin-formula permanecem intocadas.

## Ownership desta primeira PR (C0a)

| Área | Permitido | Limite |
|---|---|---|
| Proveniência de ops | scene_edit/fx.rs e novo módulo privado coeso se necessário | FxOp/FxChange públicos preservados; sem uma cópia por op |
| Autoria de parâmetros | scene_edit/fx_transition.rs e módulo de testes irmão | Somente costura de testes defensivos se a regra já estiver correta; conservar o limite de1000linhas |
| Diagnósticos | directed_scene/fx/transition_explain.rs e transition_problem.rs | Uma regra e projeção Locale; não ultrapassar1000linhas por módulo novo |
| Contexto do other | scene_plan/build.rs, timeline.rs, transition_validate.rs e helper novo | Não mudar wire nem o plano primário válido nesta PR |
| Testes | scene_fx_transition/*, scene_project/transition_edit_tests.rs e testes de plano | Fixtures sintéticos próprios; corpus fixado intocado |
| Docs | este plano, relatório C0a próprio, fx-transitions.md e scene-plan.md | Backlog/matriz/índice são patch literal para coordenador |

Extensão mecânica para match exaustivo é permitida e deve constar do relatório. Não adicionar
regra no desktop. Arquivos existentes acima de1000linhas exigem extração prévia/delta<=0.
Não tocar state/mod.rs, commands.rs, runtime, renderer, mídia, origem, UI de transição ou P5/P6.

## Aceite da C0a

| Caso | Prova |
|---|---|
| A01 | Contagens opcionais/obrigatórias: presente inválido, ausente e zero com fallback correto |
| A02 | Autoria não atravessa corte/effect/entidade recriada; identidade nova preservada |
| A03 | E13 frame e gesto após editar transição, orçamento uniforme, sem descartar passes |
| B01 | Remove/recria mesmo tipo: erro só da geração final; set antigo também excluído |
| B02 | Entre tipos permanece correto; no-op não contribui; recusa atômica e index estável |
| C01 | Erro de other em ScenePlan e PlanTimeline qualificado, inclusive IDs colidentes |
| C02 | Erro do topo conserva código/forma; não duplicar prefixo; não criar Domain |
| D01 | 40/60/48/6 pede+34: reparo aceita, um frame a menos recusa |
| D02 | Bordas, after0, use/refuse/hold e overflow tipado, sem regressão de cabeça .24 |
| D03 | Hints e números de reparo iguais nas três línguas; corpus antigo e manifesto intactos |
| G01 | Refuse público/proc headless real permanece; nenhuma nova porta Accept de produção |
| G02 | Bytes/digests sem transição e baseline existentes permanecem idênticos |
| G03 | Dependências do core sem GPUI/wry, contratos/report e matriz com escopo honesto |

Mutantes mínimos: opcional inválido→zero, apagar guarda de corte da restauração, atribuir
por ID final sem geração, qualificação other omitida ou aplicada ao topo, reparo da cauda
reduzido para after, after0 exigindo frames e hint numérico divergente. Restaurar bytes/hash
e tocar mtime; controlar que o binário do mutante não é herdado (incidente real na R1/R2).

## Gates e entrega

1. Confirmar Windows nativo, origin, main9073e77, branch própria, status limpo e espaço.
   Ler AGENTS/skill/contratos/backlog. Não criar outro clone/checkout/worktree.
2. Salvar este plano em docs/migration/p4b-s2-core-prerequisites-plan.md antes do código.
   Registrar números/casos de divergência de diagnóstico antes de alterar a produção.
3. Commits revisáveis: cobertura restante; proveniência .29; contexto .19; reparo .25;
   gates e relatório. Extrações mecânicas em commit próprio quando necessárias.
4. Git Bash REAL C:\Program Files\Git\bin\bash.exe; ./scripts/cargo; targetmbxpadrão.
   Nunca CARGO_TARGET_DIR/target-dir/lane/clean/gc; não limpar TEMP ou processos alheios.
5. fmt, check workspace, clippy workspace/all-targets-Dwarnings, no-run e build desktop;
   coreFX/transições/corpus/X/baseline em debug e preserve_order, lib e desktopFX;
   repetir release baseline conforme os helpers numéricos afetados. Teste core completo
   pode evidenciar os dois gizmos Windows; não rotular workspace global como verde.
6. Sem runtime/GPU/performance nesta PR; não repetir stress de ledger nem criar artefato
   próprio >500MB emtmp. Registrar todos os exits, contagens e limites por SHA.
7. Relatório p4b-s2-c0a-report.md com API, diferenças, testes, mutantes, execução vs leitura,
   N/A, ownership e patch literal .4/.19/.25/.26–.29/F064/index para o coordenador.
8. Commit skill se disponível; mensagens convencionais, título<=70, linhas<=100, sem
   assistentes/trailers. Se o terminal injetar mensagem, sanear só commits não publicados;
   não force-push nem reescrever branch remota. Staging só dos arquivos próprios.
9. PR contra main; link_pull_request como próxima ferramenta após criar/push. Não fazer
   watch/merge, não esperar bots. Última linha PRURL/head/checksreviews. Não parar só no plano.

## Contrato das fatias seguintes, ainda não atribuídas

C0b fixará metadados suficientes de corte/intervalos/clocks antes do parser externo. Não
tratar um witness como autenticação do bundle: a validação prova consistência estrutural
entre campos; a construção usa o bundle validado. O contador legado do primário não pode
ser normalizado incidentalmente (.23); distinguir contador autorado e amostra presa.
Provar plano realmente válido no limite de20000itens e consumidor de estado dos seteX.

H migrará from_root/SceneSlots::resolve/approved_map/resource_map para resource_union(),
com adaptação de PlanResource/UnionResource e testes de asset exclusivo do other presente/
ausente. R fixará cache de deck distinto do blend, contexto/tamanho/leases e saída única.

Antes do primeiro engine run, fixar o viés: canônico por quadro, X agregado por cenário.
Não ajustar tolerâncias após medir. centre_px/rect_px são diagnósticos, hold de câmera se
prova por estado. S2A precisa resources E requires do intervalo antes do primeiro frame;
snapshot::collect_required_assets hoje amostra só o começo de cada shot. Esses gates já
pertencem a BL082.4.10 e não são automaticamente fechados pelo novo parser ou shader.
