# Migração em etapas: base compartilhada e fases funcionais

2026-11-05. Plano de arquitetura e execução futura, **não implementação iniciada**.
Este documento é a entrada para decidir a ordem da migração; mantém a rastreabilidade
com [inventário](source-feature-inventory.md), [matriz](feature-migration-matrix.md) e
[fatias detalhadas S0–S16](migration-sequence.md).

## 1. Estratégia

Dois níveis: **building blocks compartilhados** e **fases de produto**. Câmera, luz,
clima, efeitos e rig não devem criar cada um seu formato de animação, seu sistema de
propriedades, seu undo, seu relógio ou sua lógica de seleção.

Também não devemos construir um framework inteiro antes do primeiro resultado. A base
é validada por um fluxo pequeno de cena/câmera, e cada fase posterior amplia os módulos
existentes. Não criar um novo crate para cada building block sem necessidade concreta.

O core monta/avalia o domínio; runtime JS executa desenho Three.js/Spark; adapters fazem
I/O e jobs. UI e CLI atravessam a mesma interface pública. Não portar renderizadores,
shaders ou Spark para Rust por associação com a migração de regras.

## 2. Base que já existe e deve ser reaproveitada

| Base atual | Reaproveitamento | O que ainda falta para a migração |
|---|---|---|
| Core público, JSON ops, CLI headless e bridge v1 | Entrada compartilhada e comandos sem janela | Tipos e operações da cena dirigida; introspecção por propriedade |
| `time.rs`, sequências, timeline e planos por frame | FPS racional, tempos e composição | Shots/instâncias/tempo local, câmera segmentada e animação de propriedades |
| `document.rs`, persistência, package e interchange | Save/load, versões, dependências, transporte | Compatibilidade com bundles antigos e entidades de cena/dependências novas |
| `state/history.rs` e `EditorState::transaction` | Undo/redo e rollback já existem | Expor lote atômico público de edição e agrupar um gesto em uma transação |
| Assets, jobs, media e cache | Import, proxy, relink, cancelamento | Mapas de profundidade/sombra, recursos de cenas, validação da origem e invalidação transitiva |
| Runtime Three.js/Spark e viewport persistente | Backends desktop/headless e lifecycle | Planos 2.5D, câmera resolvida, luz/clima/pós e auditoria de projeção |
| Inspector, árvore, timeline, Dock e componentes Kit | Estrutura visual e controles existentes | Editor de entidades de cena, propriedades animáveis e overlays semânticos |
| Áudio, playback e export | Adapters e infraestrutura já implementados | Regras de mix/narração da origem e paridade de tempo/cor/render |
| Harness de frames e performance | Comparações reais e instrumentação | Corpus de equivalência entre origem e destino para cada domínio |

Verificação pontual no destino: há transações internas em `state/history.rs`, mas isso
não equivale ao `editBundle` público da origem. A câmera atual em `content.rs` é
position/target/fov/orbit; não há ainda o modelo completo de segmentos/keys da origem.
`plan::Transform` é transformação 2D de composição, não substitui o grafo 3D de nós.
O código está em evolução por OPT01–OPT10; reconferir este mapa antes de cada entrega.

## 3. Building blocks que faltam ou precisam ser ampliados

| ID | Módulo e interface proposta | Lacuna e responsabilidade compartilhada | Consumidores | Saída verificável |
|---|---|---|---|---|
| BB01 | Compatibilidade e documento de cena | Ler/validar bundle v1.0, preservar desconhecidos conforme política, converter para documento próprio sem perder referências | Todas as migrações, save/package/interchange | Corpus válido/inválido com mesmos vereditos/códigos e round-trip definido |
| BB02 | Modelo de entidades e grafo de cena | IDs, hierarquia, nós, instâncias, transformações 3D, referências, ordem e espaço de coordenadas | Câmera, luzes, planos, splats, rig, paths | Matrizes efetivas e resolução de referências iguais ao oráculo; ciclos/referências quebradas rejeitados |
| BB03 | Avaliação temporal e animação | Mapear sequência→shot→instância→tempo local; tracks de propriedades, keys/easing/segmentos e avaliação pura | Câmera, luz, clima, efeitos, rig, legendas | Mesmo estado avaliado em frames de borda, seek aleatório e FPS racional; não depende de playback |
| BB04 | Catálogo de propriedades e edição atômica | Paths estáveis, tipos, unidades, limites, defaults, aplicabilidade, mutações por ID, lote/dry-run e erros por operação | Inspector, CLI/agentes, animação e overlays | Mesmo comando via lib/CLI/UI; lote inválido não muda projeto; undo/redo de um gesto/lote é único |
| BB05 | Plano de cena e contrato com o runtime | Core resolve o frame e runtime recebe estado explícito; versão/capabilities, dependências, geração, cancelamento, determinismo | Preview, render offline, export, novas famílias visuais | Frame final equivalente em seek/preview/export; runtime não passa a decidir regras de edição |
| BB06 | Recursos/dependências e jobs de cena | Estender assets/preflight/cache/package a texturas, mapas, splats, fontes e recursos de efeitos; ownership e geração transitiva | Todos os módulos visuais e transporte de projetos | Pacote abre sem paths da máquina de origem; troca/release de asset invalida dependentes sem vazamento |
| BB07 | Estado de interação e ferramentas do editor | Seleção por entidade, árvore de cena, tool state, comandos de gesto, preview/commit/cancel e agrupamento no histórico | Câmera, luzes, rig, paths e manipulação de nós | Escape cancela; arraste e comando semântico têm mesmo resultado; seleção não é pré-condição para batch |
| BB08 | Interface de inspector e overlays | Controles ligados ao catálogo e overlays com eventos semânticos; foco, hit testing e coordenadas | Câmera/luz/rig/path e efeitos | UI mostra propriedades corretas; handles e comandos CLI usam BB04; testes nativos de foco e resize |
| BB09 | Transporte e áudio de cena | Ampliar transporte/mix atuais com shots, janela de edição, narração/takes, relógio e monitor gain | Reprodução, scrub, narração e export | Frame e amostra correspondentes após play/seek/pause/scrub; avaliação offline independente do dispositivo |
| BB10 | Oráculo e gates de equivalência | Fixture/corpus, snapshots da origem, plano/pose/erro/pixels/áudio, tolerâncias e performance | Todas as etapas | Cada feature tem prova do código final e trilha MIG/F; testes lidos não contam como execução |

Interfaces acima são desenho proposto, não novos símbolos já entregues. BB01–BB06/BB09
ampliam módulos existentes; BB07/BB08 reaproveitam as entidades/controles GPUI atuais.
O catálogo começa com propriedades de câmera e nós; luz/clima/efeitos o estendem depois.
Não criar mecanismo genérico de plugins dinâmicos ou replicar schemas em três adapters.

### Dependências entre os building blocks

```text
BB10: provas e corpus acompanham todas as etapas
BB01 documento ──> BB02 grafo ──> BB03 avaliação
                       │              │
                       └──> BB04 edição/props ──> BB07 interação ──> BB08 inspector/overlays
BB06 recursos ─────────────────> BB05 plano/runtime
BB02 + BB03 ───────────────────> BB05 plano/runtime ──> preview/export
BB03 + áudio atual ────────────> BB09 transporte/mix
```

BB04 lê os tipos/regras de BB02/BB03; os setters não podem implantar uma segunda avaliação.
BB10 começa com dados sintéticos pequenos e cresce por domínio. Não é necessário migrar
toda a câmera para testar documento, nem todo o clima para provar o contrato de animação.

## 4. Etapas da base — executar antes de ampliar famílias de features

| Etapa | Building blocks | Entrega concreta | Gate |
|---|---|---|---|
| E0 — Contrato e baseline | BB01/BB10 | Fixar snapshots, escolhas de formato/tipos, corpus mínimo e política de compatibilidade | Decisões aplicáveis registradas; validar bundles sem GPU; oráculo reproduzível |
| E1 — Domínio mínimo compartilhado | BB02/BB03/BB04 | Uma cena com dois planos, um shot e câmera com propriedade animada, editável por lote atômico | Plan/pose determinísticos, validação e rollback pela lib e CLI sem janela |
| E2 — Render e recursos integrados | BB05/BB06 | Render da cena E1 pelo runtime existente, save/package/reopen e export curto | Mesmo conteúdo preservado após transporte; frames/áudio/cor conforme contrato |
| E3 — Superfícies compartilhadas | BB07/BB08/BB09 | Seleção, inspector, um handle de câmera, scrub e histórico conectados ao mesmo core | Mesmo fluxo pela UI/CLI/lib; undo/cancel; nenhuma regra exclusiva da UI |

**Primeiro marco:** abrir um bundle mínimo → avaliar a câmera em um frame → alterar a
propriedade por ID → undo/redo → salvar → renderizar o mesmo frame por CLI e monitor →
exportar trecho curto. Isso prova a base; não requer migrar antecipadamente luz/clima/rig.

E2/E3 já são fatias visíveis pequenas, não meses de estrutura invisível. Só aprovar um
building block com pelo menos um consumidor real e teste pela interface pública.

## 5. Fases funcionais de migração

| Fase | O que o usuário passa a fazer | Reutiliza a base | Referência às fatias/MIGs existentes |
|---|---|---|---|
| M1 — Cenas e direção de câmera completas | Abrir produções em camadas, navegar shots, editar segmentos/receitas/keyframes/easing e usar controles de câmera | BB01–BB10 | S0–S5/S7/S8; MIG012–MIG018, MIG025–MIG045, MIG047 e contratos de documento/render |
| M2 — Iluminação e sombras | Criar/editar luzes, animação, marcadores e mapas/sombras com resultado no preview/export | BB02–BB08/BB10 | S9; MIG054–MIG059 |
| M3 — Clima e pós-produção | Usar clima/chuva/neve e looks existentes, animando parâmetros pelo mesmo catálogo | BB02–BB06/BB08/BB10 | S10/S11; MIG060–MIG068 |
| M4 — Splat com recursos da cena | Aplicar a integração da origem entre splats, luz e clima; preservar câmera/composição/determinismo | BB02–BB06/BB10 + M2/M3 aplicáveis | S12; não remigrar Spark básico já entregue |
| M5 — Narração e legendas | Editar takes, tempos, texto temporizado e mix; conferir saída final | BB01/BB03/BB04/BB06/BB08–BB10 | S6; TTS é adapter opcional sujeito à decisão de escopo |
| M6 — Rig, paths e follow | Usar mão/rig/IK e trajetórias/follow das produções que dependem disso | BB02–BB08/BB10 | S13; MIG069–MIG080; solver pode permanecer no runtime JS |
| M7 — Delta do piloto | Migrar as funcionalidades selecionadas de efeitos/transições e edição de shots/gestos | BB01–BB10 + famílias escolhidas | S15; MIG100–MIG105, após aprofundar o catálogo do piloto |
| M8 — Complementos de produto e fechamento | i18n, acessibilidade, atalhos e organização visual faltantes; compatibilidade e regressão completas | Base e fases entregues | MIGs correspondentes na matriz; multi-projeto apenas se escolhido |

Faixas MIG nas fases são referências de domínio, não nova partição exaustiva: a matriz
continua sendo a fonte por feature. MIGs AUS/PLAN não viram obrigação de migração; ferramentas
externas e implementação do servidor de preview não migram por padrão.

M2 e M5 podem evoluir independentemente depois da base. M3 depende de iluminação somente
nos recursos cujo desenho realmente use luz. M4 não bloqueia a validação de Spark básico
nem a migração de câmera. M6/M7 são posteriores ao caminho principal. M8 tem itens que devem
acompanhar as fases desde o início (labels, foco, atalhos), não justificar uma UI inacessível.

## 6. Decisões de arquitetura antes da etapa afetada

- **Antes de E0:** compatibilidade com `render-bundle.json`: proposta é ler/gravar como
  intercâmbio, preservando o documento próprio. Definir perda/round-trip e campos desconhecidos.
- **Antes de E1/E2:** representação de cena dirigida: proposta de tipo versionado com
  grafo/planos 2.5D/shot/câmera, não simular paralaxe como clipes 2D independentes.
- **Antes de E3:** protocolo de overlays: desenho/hit testing no runtime JS quando necessário,
  intenções por IDs para o core; não permitir que o handle edite estado só no navegador.
- **Antes de M5:** fontes/texto temporizado e política de TTS local. Serviços novos não
  são autorizados automaticamente por este planejamento; preservar adapters opcionais.
- **Antes de M6:** manter solver IK JS inicialmente é a proposta; paridade numérica antes
  de qualquer port para Rust. Persistir parâmetros/versões necessários para repetir resultado.
- **Antes de M7:** escolher features e snapshot do piloto; não é necessário merge na origem.
- **Multi-projeto:** não confundir múltiplas sequências atuais com projetos em abas. Só migrar
  abas/lock/hibernação se o fluxo for escolhido; isso não bloqueia a base.

As propostas acima precisam de decisão de escopo registrada antes da implementação
dependente. Este plano não assume aprovação implícita das recomendações D1–D11 do inventário.

## 7. Gates obrigatórios de toda entrega

1. Comando/consulta de domínio por ID pela lib e CLI headless; UI consome a mesma interface.
2. Documento da feature e linha na matriz com status por superfície e referência MIG/BB/fase.
3. Mesmo veredito/plano/pose/tempo que a origem nos casos aceitos; diferenças intencionais
   documentadas. Códigos/tolerâncias não são inventados depois de ver o resultado.
4. Pixel/áudio/render reais quando aplicável, incluindo frames fora de ordem, export e
   save/package/reopen. Sucesso de teste unitário não prova imagem na janela.
5. Rollback/undo/cancelamento/invalidação/contexto e memória limitados; sem bloquear a UI
   ou depender de janela aberta para completar o fluxo programático.
6. Benchmark em release, quente/frio, mesma fixture/qualidade, sem regressão das otimizações
   atuais. Separar CPU/GPU drenada/accepted/rAF/photon e declarar provas físicas ausentes.

## 8. Relação com o trabalho em andamento

OPT01–OPT10 otimiza a infraestrutura já existente. Pode seguir em paralelo ao desenho
E0, mas não deve competir com alterações simultâneas no mesmo runtime/core durante E1/E2.
Antes de implementar cada etapa, conferir contratos e relatório final das otimizações.

Não reabrir a migração inteira do zero nem exigir concluir toda otimização de Spark para
começar E0/E1. Ordem prática: **fechar contratos → base mínima com prova completa → câmera
e cenas completas → famílias visuais/narração → rig e piloto escolhidos**.
