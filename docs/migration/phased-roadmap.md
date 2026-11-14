# Migração em etapas: base compartilhada e fases funcionais

**Status em 2026-11-14.** A implementação **começou e está parcialmente entregue**; este arquivo continua sendo
o mapa de dependências E0–E3 / BB01–BB10 / M1–M8, **não** uma fila: a fila única é [docs/backlog.md](../backlog.md).
A reconciliação datada contra contratos, backlog e código da `main` `2b54ac1` (PR #57 integrada; PRs #39, #42, #48 e
#51–#57) está na [seção 9](#9-reconciliação-em-2026-11-14). As seções 1–8 abaixo são o texto do plano de
**2026-11-05**, preservado como histórico de decisão e arquitetura (a arquitetura não mudou); a frase original
"não implementação iniciada" descrevia o dia 05/11 e deixou de valer.

2026-11-05 (original). Plano de arquitetura e execução futura. Este documento é a entrada para decidir a ordem da
migração; mantém a rastreabilidade com [inventário](source-feature-inventory.md), [matriz](feature-migration-matrix.md)
e [fatias detalhadas S0–S16](migration-sequence.md).

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

> Texto de 05/11. Estado de cada BB em 14/11/2026: [seção 9.2](#92-building-blocks-bb01bb10).

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

> Texto de 05/11. Estado de E0–E3 e do primeiro marco em 14/11/2026: [seção 9.1](#91-etapas-e0e3).

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

> Texto de 05/11. Estado de M1–M8 em 14/11/2026: [seção 9.3](#93-fases-m1m8).

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

## 9. Reconciliação em 2026-11-14

Base lida: `origin/main` `2b54ac1` (PR #57, "Add clip splitting with exact audio phase and Windows support", integrada em
2026-11-14T12:30:55Z), os contratos de [docs/features/](../features/README.md), [docs/backlog.md](../backlog.md) e o
código, com as PRs #39 (P2), #42 (P3/`filmGrain`), #48 (P4a, F062) e #51–#57 (reconciliação S1–S10, testes, higiene do
backlog e PR #57). **Nada foi reexecutado para escrever esta seção**: cada afirmação cita o estado que o backlog ou o
contrato registra, com o head que o próprio registro nomeia. BL163 registra o harness S10 **24/24** em
`057b7db` e uma reconfirmação posterior de fmt/check/clippy/build, workspace (2559 passaram, 16 ignorados e
uma falha de ambiente BL082.47) e runtime384/384 em `bc0c3ea`, depois das PRs #52 e #53. Após `057b7db`
entraram #51–#57; #52 e #57 alteram produção. A PR57 muda áudio do core, export de mídia e descoberta do runtime;
seus gates dirigidos não repetem S10. Isto **não** declara a `main` `2b54ac1` verde.

### 9.0 As quatro colunas (não se misturam)

| Coluna | Significa | Fonte |
|---|---|---|
| **Implementado** | Código presente na `main` `2b54ac1` com contrato em `docs/features/` | Matriz e backlog; leitura de código por amostragem |
| **Provado** | Execução registrada **por superfície** (lib, JSONL em processo real, GPUI sintético, janela real semântica, pixels), no head que o registro cita. "A validar" no backlog continua **sem** fechamento | Linhas do backlog e relatórios |
| **Aceite humano** | Alguém olhou/ouviu a saída e aceitou. **Nenhum registro encontrado nos documentos lidos** para E0–E3 nem M1–M8: BL058 (revisão visual/áudio dos exports finais) está "A validar"; BL043 (ponteiro físico), BL046 (áudio ouvido) e BL082.42 (prova de alças/ponteiro e WebView; foco/popover em BL044) seguem abertos | BL058, BL043, BL046 |
| **Paridade** | Comparação com a origem: **executada** na origem fixada (`d6adac15`, câmera/edição/vereditos do bundle) ou **por fórmula transcrita** por um script Python independente (piloto, sem golden). Nenhuma paridade com o golden da origem é afirmada: `ORIGIN_GOLDEN_NOT_COMPARABLE` | `docs/migration/fixtures/`, F016, F044 |

A prova no Windows se limita ao que BL167 registra (desktop 502/502, runtime 384/384, export 11/11 e flow 2/2 em
`ead9ca2`, com o workspace **não verde**); toda prova de efeitos e gizmos citada abaixo é de macOS/SwiftShader ou do
Chrome do runtime, salvo onde a linha diz Windows.

### 9.1 Etapas E0–E3

| Etapa | Implementado | Provado | Aceite humano | Paridade | Lacunas (backlog) |
|---|---|---|---|---|---|
| E0 — Contrato e baseline (BB01/BB10) | Leitor/validador do bundle 1.0 (`directed_scene`), corpus de bundles válidos/inválidos, oráculo Node na origem fixada | Corpus ampliado: 47 casos estendidos + 55 base conferidos contra a origem; 49 testes Node e 15 Rust (BL018, F032) | N/A (não é visual) | **Executada** na origem fixada para vereditos do bundle 1.0. Exceção: `invalid-dissolve-transition` vem de um pin anterior ao piloto (ver [p4b-cross-dissolve-plan.md](p4b-cross-dissolve-plan.md), D04) | BL001 `DEFERRED_RULES/FIELDS` (Parcial); pin do piloto `d40ac3f7c0` ausente no clone Windows da origem |
| E1 — Domínio mínimo (BB02/BB03/BB04) | Grafo e entidades, avaliação temporal, câmera segmentada, `scene.edit` em lote atômico com dry-run/undo (F034), luz pontual (F060) | BL004/BL005/BL004a–d/BL005a–c **A validar** (31 casos do oráculo de edição, `oracle.mjs verify` saiu 0; lib e JSONL); sem ponteiro físico | Nenhum registro encontrado nos documentos lidos | **Executada** (pose/edição contra o oráculo fixo) | BL001–BL004 Parcial: keys/segmentos gerais, clima, rig, paths, bind/reveal |
| E2 — Render e recursos (BB05/BB06) | Plano `ScenePlan` v1 (13 capacidades), renderer Three.js no monitor e headless, export de cena (BL008), manifesto/closure/pacote (BL009–BL011), mix offline de áudio (F042A) | BL006 e BL008 **Concluído** (PR #3/#4, 1080p/4K, CLI/socket/diálogo); S10 em `057b7db` (24 fases; não repetida após #57); reconfirmação de gates em `bc0c3ea` | Nenhum registro encontrado nos documentos lidos (BL058 A validar) | Oráculo **independente** do harness da S10 ([F044](../features/scene-acceptance.md)), não execução da origem | BL007 contexto/reload nativos dirigidos; BL009–BL011 mapas/fontes/formatos (BL077) Parcial |
| E3 — Superfícies (BB07/BB08/BB09) | Seleção/gesto (F039), inspector ligado (BL014), gizmos de translação, rotação e escala (F055; PRs #40/#43), luz com alcance (PR #45), alças 2D da vinheta (PR #46) | BL014 **Concluído**; BL015 e F055/F060/F061 **A validar** (lib, JSONL, GPUI, janela real semântica); sem ponteiro físico (BL043) nem a prova correspondente de WebView/alças (BL082.42; foco/popover BL044) | Nenhum registro encontrado nos documentos lidos | Não aplicável por gizmo (comportamento do editor); sem golden | **BL016.1** transporte dirigido (play/seek/pause) Pendente; BL017 narração/takes Pendente |

**Primeiro marco** (cena mínima → plano → monitor/headless → pacote sem origem → export curto): **atingido como fatia
integrada** (BL006, BL008, BL163), com as lacunas acima; não é aceite humano nem prova no Windows.

### 9.2 Building blocks BB01–BB10

Para todas as linhas, **Aceite humano** (nenhum registro) e **Windows** (só BL167) valem como na seção 9.0 e não se repetem.

| BB | Backlog (estado em 14/11) | Implementado | Provado | Paridade | Falta |
|---|---|---|---|---|---|
| BB01 documento | BL001 Parcial | Bundle 1.0 lossless, extensão `fx` do editor | Corpus e testes de ida e volta | Executada (origem fixada) | Regras deferidas por domínio; dissolve legado |
| BB02 grafo | BL002 Parcial | Entidades, hierarquia, `EntityKind::Light` (G2.1) | Matrizes efetivas contra o oráculo; luz a validar | Executada | Rig, paths |
| BB03 avaliação | BL003 Parcial | Tempo sequência→shot→instância, câmera, luz pontual por quadro | Frames de borda, seek aleatório e FPS racional no corpus | Executada | Clima, rig, paths, bind/reveal |
| BB04 catálogo e edição | BL004 Parcial; BL004a–d, BL005, BL005a–c **A validar** | Catálogo (21 entradas em F034), componentes `X/Y/Z`, `renderOrder`, lote/dry-run/undo | Lib, JSONL, GPUI; 31 casos do oráculo | Executada | Keys/segmentos gerais e domínios novos |
| BB05 plano e runtime | BL006 **Concluído**; BL007 Parcial; BL008 **Concluído** | `ScenePlan`, 13 capacidades, runtime só desenha, export pelo mesmo plano | PR #3/#4; BL163 | Oráculo independente (F044) | Contexto/reload nativos dirigidos (BL007/BL092) |
| BB06 recursos e jobs | BL009/BL010/BL011 Parcial | Manifesto/closure/preflight, `MediaHost`, pacote Directory/Archive | F038/F040 (core 27 + media 39; restauração sem origem) | N/A | Mapas, fontes, narração; UI e formatos (BL077) |
| BB07 interação | BL012/BL013 Parcial | Seleção por entidade, gestos preview/commit/cancel com histórico agrupado | Executável headless real (13 requests), GPUI e socket | N/A | Ponteiro físico e handles (BL043/BL015) |
| BB08 inspector e overlays | BL014 **Concluído**; BL015 **A validar** | Inspector ligado ao catálogo, gizmos, alças da vinheta | Janela real semântica; mutantes mortos | N/A | WKWebView/ponteiro físico, desempenho dos gizmos |
| BB09 transporte e áudio | BL016 **A validar**; BL016.1 **Pendente**; BL017 Pendente | Mix offline do WAV declarado (F042A), tempo racional frames/amostras | 270/270 de `audio_export` e harness S10 (áudio medido, **não ouvido**) | Regra da origem (`editStartSeconds`) decidida; divergência BL016.11 declarada | Transporte dirigido com relógio de amostras; narração/takes |
| BB10 oráculo e gates | BL018/BL019 Parcial; BL163 **Concluído** (12/11) | Corpus, `pilot-fx` (Python independente), harness S10, higiene (BL164–BL166, PRs #54–#56) | BL163: S10 em `057b7db`, gates em `bc0c3ea`; spec das tabelas do backlog (BL165) | Ver 9.0 | Provas por domínio novo; desempenho; atestação reprodutível (BL019-F11) |

### 9.3 Fases M1–M8

| Fase | Backlog | Estado em 14/11 | Implementado / provado | Paridade | Próximo passo (sem criar fila) |
|---|---|---|---|---|---|
| M1 cenas e câmera | BL022 | Parcial | Câmera, shots e receitas editáveis em lib/CLI/UI (E1–E3); ripple/roll e `trimShot` ausentes (BL082.5/P5). A divisão de clipes da PR #57 é da timeline de vídeo (BL081.5), **não** de shots dirigidos | Executada (câmera) | P5 depois de P3/P4 |
| M2 luzes | BL023 (+.1–.6) | Parcial — luz pontual (F060) **A validar** | Posição/alcance, gizmo, plano `lights` (G2.1); splats não iluminados | Por oráculo independente do renderer | BL023.1–.6: ambiente, sombras, mapas, splats, prova física, custo |
| M3 clima e pós | BL024 | **Pendente** (clima, partículas, grade). Os três *looks* já entregues são rastreados em BL082 (M7), não aqui | — | — | Depende de iluminação aplicável |
| M4 splat | BL025 | Parcial — subconjunto S7 validado (PR #13) | `spz` aprovado, matriz de semelhança, cor | Oráculo independente | Luz/clima em splats |
| M5 narração e legendas | BL026, BL017 | **Pendente** | — | — | TTS só se autorizado (BL037) |
| M6 rig, IK, paths | BL027 | **Pendente** | — | — | Solver decidido explicitamente |
| M7 delta do piloto | BL028, BL082.* | Parcial — **3 de 59 efeitos** (`vignette`, `fade`, `filmGrain`), chaves/seed (F058/F059), alças da vinheta (F061), escopo `shot` (F062, PR #48): **A validar**, macOS/SwiftShader | P0–P4a em lib, JSONL real, GPUI e janela real semântica; sem Windows, ponteiro físico, GPU real nem desempenho (BL082.42/.43) | **Por fórmula** (`pilot-fx`, Python); golden da origem não comparado | **P4b** (BL082.4, [plano](p4b-cross-dissolve-plan.md)); escopo `node` (BL082.40); P5/P6 |
| M8 complementos | BL029, BL030 | Parcial / Pendente — rótulos de efeitos em pt-br/en/es no core; acessibilidade, atalhos e demais telas pendentes | Teste sobre os IDs oferecidos | — | Fechamento: matriz MIG/F reconciliada (BL030) |

Sobreposição a corrigir no desenho: M3 ("looks existentes") e M7 ("efeitos/transições do piloto") citam as mesmas famílias
de efeito. O backlog resolveu rastreando **todos** os efeitos em BL082/BL028; esta tabela mantém M3 só para clima,
partículas e grade.

### 9.4 Rodada Windows de 14/11/2026

| Item | Estado registrado | O que este documento afirma |
|---|---|---|
| BL167 (agregador) | Parcial — setup e prova inicial em `ead9ca2`; workspace Windows **não verde** | Nada além do backlog |
| BL167.1 (compilação dos testes de mídia) e BL167.2 (descoberta de ferramentas) | **Integrados na `main` pela PR #57** (`2b54ac1`); as linhas do backlog ainda dizem "Concluído localmente… integração/publicação pendentes" e "Sem PR/merge" | **Linhas obsoletas**; patch proposto ao coordenador (abaixo). Windows continua com falhas de execução fora desses itens |
| BL167.3 (hash portátil, dois gizmos divergentes) e BL167.7 (separador do helper `tree()`) | **Pendente** no backlog; **frente W1 iniciada** pelo coordenador nesta rodada | **Nenhum resultado, commit ou PR foi recebido por esta frente.** Sucesso **não** alegado |
| BL167.6 (teste do ledger que trava) | **Pendente**; **frente W2 iniciada** | Idem |
| BL167.4 (mbx sem *hits*, volumes sem *hard link*, wrapper dependente de Git Bash) e BL167.5 (Python, release, provas vivas) | **Pendente** | Ordem do coordenador: depois de W1/W2 e da P4b |

Patch proposto às linhas obsoletas (o coordenador integra; esta frente não edita `docs/backlog.md`):

- **BL167.1** — Estado: `Integrado na main pela PR #57 (2b54ac1, 2026-11-14); validação Windows/Mac conforme o relatório; workspace Windows
  ainda não verde (BL167.3/.6/.7)`; remover "integração/publicação pendentes".
- **BL167.2** — Estado: `Integrado na main pela PR #57 (2b54ac1)`; trocar "Sem PR/merge" por "PR #57"; manter BL167.2.3 (Linux) e BL167.2.2
  (script de desempenho) como pendências.

### 9.5 Próximas fatias (a ordem é do coordenador; o backlog é a fila)

1. **Confiabilidade Windows:** W1 (BL167.3/.7) e W2 (BL167.6), em andamento; BL167.4/.5 depois.
2. **Plano P4b** (este roadmap + [p4b-cross-dissolve-plan.md](p4b-cross-dissolve-plan.md)): revisão independente, depois
   integração **só com prova**; a implementação começa S0 → S1a → S1b → {S2 ∥ S3} → S4 → S5 → S6.
3. **Escopo `node` (BL082.40), P5 (BL082.5) e P6** não bloqueiam a P4b por associação; P5 herda da P4b o contrato de recusar
   o que invalida uma transição (BL082.44 para clipes `shot`).
4. **BL016.1** (transporte dirigido) e demais famílias (M2–M6) seguem como estão no backlog; nenhuma é habilitada por este plano.

### 9.6 Limites desta reconciliação

Sem execução nova, sem prova de Windows, sem aceite humano, sem golden da origem e sem leitura do pin `d40ac3f7c0`
(ausente no clone Windows da origem; as leituras de P4b vêm da ponta `3655790`, somente leitura). As decisões D1–D11 do
inventário continuam **sem** aprovação implícita (seção 6). Qualquer divergência entre esta tabela e `docs/backlog.md` se
resolve a favor do backlog.
