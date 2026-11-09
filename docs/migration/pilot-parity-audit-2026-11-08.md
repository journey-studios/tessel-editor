# Paridade do piloto no editor Rust — auditoria de 2026-11-08

## Resultado e limites da auditoria

A base de edição e render já existe, mas a camada do piloto continua parcial. O catálogo Rust
contém **1 dos 59 efeitos**: `vignette`, somente em `output`. Faltam 30 looks e 28 transições.
Esta contagem mede entradas implementadas; não mede percentual de trabalho ou paridade visual.

Referência integrada: `10683c5357c244692063099e13ccb5685385e0fa` (`origin/main`, conferida com fetch nesta auditoria).
Checkout de documentação: `2401037efd7001bf19e7507faf75c597fef50fd0`, branch
`docs/reconcile-migration-status`. O código foi lido com `git show origin/main:<arquivo>`:
a pasta principal está atrás da integração e não foi trocada de branch.

Origem funcional: [inventário do piloto](pilot-fx-shots-studio-features.md), snapshot declarado
`d40ac3f7c0`. O repositório JS não foi modificado, instalado nem executado. As capacidades da
origem são as documentadas nesse inventário; a auditoria não revalidou seu código nem seus testes.
Antes de portar cada fatia, reabrir o blob fixado da origem e extrair seus casos de equivalência.

Levantamento estático, com consulta ao Git e às PRs. Não foram executados build, testes de Rust,
render, export ou UI. Evidências antigas abaixo são referências históricas, nunca execução desta auditoria.
O backlog único continua em [docs/backlog.md](../backlog.md); a ordem abaixo é recomendação,
não atribuição ou início de implementação. Nenhuma nova feature foi autorizada por este documento.

## Evidências que determinam o diagnóstico

Caminhos desta seção se referem ao SHA integrado acima, não ao checkout antigo:

| Evidência | Leitura e consequência |
|---|---|
| `crates/core/src/directed_scene/fx/catalog.rs:201` | `CATALOG` contém somente `VIGNETTE`; não há 59 efeitos disponíveis |
| `crates/core/src/scene_edit/fx.rs:84` | `FxOp` tem AddClip, RemoveClip, SetParam, SetField e SetHidden; não equivale às 19 operações do piloto |
| `crates/core/src/scene_project/fx.rs:18` | Consulta publica escopo output e recusas para keys, shot/node e transições |
| `crates/core/src/directed_scene/fx/read.rs`, `read_scope`, `read_keys`, `read_layer` | `FX_SCOPE_NOT_YET`, `FX_KEYS_NOT_YET`, `FX_TRANSITION_NOT_YET`; ausência comprovada por recusa explícita |
| `runtime/src/output-fx.js:22`, `:203` | Tabela só vignette e OutputFxPipeline de saída; não é o compositor de dois decks do piloto |
| `crates/desktop/src/workstation/scene_fx.rs:126`, `:166`, `:514` | Rótulos de parâmetros e modelo de controles vivem no desktop; slider guarda valor e confirma ao fim; não entrega inspector neutro completo nem preview de candidato |
| `crates/core/src/scene_interaction/gesture.rs:1` | Já há candidato separado do documento, proteção de revisão e commit compartilhado para cenas; reutilizar mecanismo, implementar regras específicas de FX/shots |
| `crates/core/src/scene_edit/reference.rs` e `op.rs` | Catálogo tipado set/reset existente; não expõe o registro completo de caminhos fx/transition/shot do piloto |
| `docs/features/effects.md` e matriz F016 em `docs/features/README.md` na main | Há divergência de atualização: matriz marca Lib/JSONL V; contrato ainda conserva R1 sem compilar e provas nativas de head anterior. Não presumir aceite final da R1 |

## Matriz por capacidade

Legenda: **Parcial** = subconjunto implementado; **Pendente** = contrato do piloto ainda ausente;
**Base** = infraestrutura reutilizável, sem equivalência específica provada; **Fora** = ausente no
próprio piloto ou dependente de decisão de escopo. Cada linha reutiliza IDs do backlog.

| Capacidade do piloto | Rust integrado / lacuna concreta | Estado | Backlog e aceite necessário |
|---|---|---|---|
| Fx opcional, tracks, clips, limites, fades e ordem | Modelo output, parâmetros fixos e vinheta; validação/resolve/persistência já ligados | Parcial | BL082/BL001: equivalência de normalização, erros e limites com corpus fixado |
| 19 comandos de FX | 5 variantes de FxOp; faltam operações de trilha, transferência/reordenação, chaves, seed e transições | Parcial | BL004/BL028/BL082: comparar operação por operação; lote atômico, no-op e undo único |
| 31 looks | 1 entrada Rust/runtime; 30 ausentes, discriminadas no catálogo abaixo | Parcial | BL082.1: um efeito por fatia com referência independente e pixels monitor/export |
| 28 transições e dissolve legado | `transitions` não vazio é recusado; falta normalizar legado, janelas e handles | Pendente | BL082.4: cortes pares/ímpares, alinhamentos, duração, vizinhos e erros equivalentes |
| Parâmetros boolean/enum/ref e unidades | Catálogo atual oferece number/color/point; não há variedade de controles do piloto | Parcial | BL004/BL082.1: tipo/default/faixa/unidade e referências validadas no core |
| Chaves, easing e seed | Chaves recusadas; vinheta não prova ruído temporal ou seed determinístico | Pendente | BL082.2: avaliação pura por frame, mesmo seed/frame = mesmo estado/pixel |
| Escopo shot | Só output; falta aplicar antes da transição no deck correto | Pendente | BL082.3: provar efeito local e preservação dos outros planos |
| Escopo node | Rust recusa; piloto também recusa NODE_SCOPE_NOT_YET | Fora da paridade | BL082.3/BL036: separar decisão futura de requisito já existente na origem |
| 4 comandos shots | Trim de clipe da sequência não implementa trimShot/moveCut/setShotDuration/setEditDuration | Pendente | BL082.5/BL022: core + lib/JSONL/UI, comparação do bundle completo |
| Ripple/roll, anchor/follow/auto | Falta propagação coordenada a FX, luz/clima, rig, outline e transições | Pendente | BL082.5: invariantes e clamp com limitFrame, inclusive limites e recusas atômicas |
| Imagem e áudio independentes | Existe áudio de projeto/export dirigido, mas falta regra de retime de shots/takes | Parcial | BL016/BL017/BL082.5: padrão não move áudio; withNarration explícito; aviso NARRATION_PAST_EDIT |
| Gestos FX/shot/transition/key | Há mecanismo genérico de candidato; faltam gestos específicos e seus limites | Base | BL028/BL082.5/.6: candidato não altera documento, cancel limpa, soltar produz 1 undo |
| Inspector neutro e paths | Controles da vinheta no desktop; não é describeFxInspector do core | Parcial | BL004/BL014/BL082.1: modelo consultável por JSONL e UI, labels e reset/key por campo |
| i18n de parâmetros | Efeito tem pt-br/en; rótulos de parâmetro locais em português, falta cobertura pt-br/en/es | Parcial | BL029/BL040: tabela do core e teste de cobertura dos IDs do catálogo |
| Compositor linear, dois decks e transição | Pipeline de saída existe; faltam decks/looks por shot, passagem/adoption do secundário e transformação final equivalente | Parcial | BL015/BL082.3/.4: cor, ordem, identidade amount=0, resize e perda de contexto |
| Timeline de FX e transições | Seção Efeitos no inspector não equivale a trilhas com clips/handles/delete | Pendente | BL028/BL014: cada gesto acessível por comando e prova da janela |
| Timeline de shots e chaves | Timeline NLE existe; não prova ripple/roll, keys ordenadas, retime follow ou Alt | Base | BL022/BL027/BL028: UI consome gesto puro e mantém último candidato legal |
| Preview de FX durante slider (estado da auditoria anterior a P0+P1; entregue no branch `t3/fx-candidate-preview`) | Buffer confirma ao soltar; sem candidato vivo, inserção no playhead/teclado incompletos | Pendente | BL082.6: prova semântica e visual, undo único e cancelamento |
| Aviso do orçamento de 8 passes (estado da auditoria anterior a P0+P1; entregue no branch `t3/fx-candidate-preview`) | Core/JSONL avisam; exibição na janela ainda é gap registrado | Parcial | BL082.16: nó inspecionável e aviso visível, nunca truncar passes |
| CLI do piloto | scene.fx.catalog/list/frame/edit existe; falta cobertura funcional de catalog filters/describe/inspect/shots/keys/follow | Parcial | BL028/BL004/BL027: mapear semântica para JSONL; não exigir copiar sintaxe Node |
| Undo por domínio e abas de projeto | Undo/projeto/sequências existem; não demonstram múltiplos projetos abertos nem domínios do piloto | Base | BL011/BL028/BL029: decidir semântica e provar independência antes de anunciar paridade |
| Luz/clima/referências e looks de cena | Famílias ainda adiadas; efeitos ancorados em luz não podem entrar como shader isolado sem alvo válido | Pendente | BL023/BL024/BL003: avaliação e referência no core, desenho e pixels por família |
| Rig/IK/path/follow | Não há migração funcional completa dessas regras | Pendente | BL027: escolher solver, estado/frame da origem, gestos e integração visual |
| Narração/takes/legendas | Mix offline dirigido não entrega edição de takes nem transporte dirigido completo | Parcial | BL017/BL026/BL016.1: conteúdo/tempo/mix persistentes e preview/export coerentes |
| Oráculo e goldens do delta | Oráculo atual de migração é base reaproveitável; falta corpus piloto de FX/shots/gestos | Parcial | BL018/BL019/BL036: fixar versão, hashes, tolerâncias e controles negativos |
| Produzir splat a partir de imagem | Importar PLY SHARP (F052) não equivale a executar o gerador de imagem→splat | Fora do editor | BL036/MIG105: decidir escopo e recursos antes de migrar ferramentas de produção |

## Catálogo individual: 59 entradas

Fonte dos IDs e famílias: tabelas §2.1/§2.2 do inventário do piloto. Conferência estática
contra `CATALOG` Rust e `OUTPUT_FX_EFFECTS`. Arquivo estruturado:
[pilot-parity-catalog.json](pilot-parity-catalog.json). Vignette permanece parcial porque há
escopos/keys/gestos do piloto não entregues; as outras 58 entradas estão pendentes.

| Família | Efeito | Estado | Backlog |
|---|---|---|---|
| look | `bleachBypass` | pendente | BL082.1 |
| look | `bloom` | pendente | BL082.1 |
| look | `breathingZoom` | pendente | BL082.1 |
| look | `cameraShake` | pendente | BL082.1 |
| look | `candlelight` | pendente | BL082.1 |
| look | `chromaticAberration` | pendente | BL082.1 |
| look | `colorGrade` | pendente | BL082.1 |
| look | `dream` | pendente | BL082.1 |
| look | `dust` | pendente | BL082.1 |
| look | `fade` | pendente | BL082.1 |
| look | `film` | pendente | BL082.1 |
| look | `filmGrain` | pendente | BL082.1 |
| look | `fire` | pendente | BL082.1 |
| look | `flameLife` | pendente | BL082.1 |
| look | `flicker` | pendente | BL082.1 |
| look | `godRays` | pendente | BL082.1 |
| look | `halation` | pendente | BL082.1 |
| look | `heatHaze` | pendente | BL082.1 |
| look | `ink` | pendente | BL082.1 |
| look | `inkPrint` | pendente | BL082.1 |
| look | `lensBlur` | pendente | BL082.1 |
| look | `letterbox` | pendente | BL082.1 |
| look | `lightLeak` | pendente | BL082.1 |
| look | `mist` | pendente | BL082.1 |
| look | `moonlight` | pendente | BL082.1 |
| look | `scalePunch` | pendente | BL082.1 |
| look | `scanlines` | pendente | BL082.1 |
| look | `smoke` | pendente | BL082.1 |
| look | `smokeBillow` | pendente | BL082.1 |
| look | `smokeWisp` | pendente | BL082.1 |
| look | `vignette` | parcial | BL082 |
| transition | `additiveDissolve` | pendente | BL082.4 |
| transition | `blinds` | pendente | BL082.4 |
| transition | `burn` | pendente | BL082.4 |
| transition | `crossDissolve` | pendente | BL082.4 |
| transition | `cut` | pendente | BL082.4 |
| transition | `defocusDissolve` | pendente | BL082.4 |
| transition | `dipToBlack` | pendente | BL082.4 |
| transition | `dipToColor` | pendente | BL082.4 |
| transition | `dipToWhite` | pendente | BL082.4 |
| transition | `eyeBlink` | pendente | BL082.4 |
| transition | `fadeThroughVignette` | pendente | BL082.4 |
| transition | `filmDissolve` | pendente | BL082.4 |
| transition | `glitch` | pendente | BL082.4 |
| transition | `heartbeatCut` | pendente | BL082.4 |
| transition | `inkBleed` | pendente | BL082.4 |
| transition | `iris` | pendente | BL082.4 |
| transition | `lightningFlash` | pendente | BL082.4 |
| transition | `lumaWipe` | pendente | BL082.4 |
| transition | `push` | pendente | BL082.4 |
| transition | `shadowSweep` | pendente | BL082.4 |
| transition | `shockwave` | pendente | BL082.4 |
| transition | `slide` | pendente | BL082.4 |
| transition | `smokeFog` | pendente | BL082.4 |
| transition | `staticNoise` | pendente | BL082.4 |
| transition | `whipPan` | pendente | BL082.4 |
| transition | `wipeLinear` | pendente | BL082.4 |
| transition | `wipeRadial` | pendente | BL082.4 |
| transition | `zoomCross` | pendente | BL082.4 |

## O que não deve virar requisito de paridade

O próprio piloto ainda não entrega node scope, flashlightReveal, presets preenchidos, galeria F6,
prefetch de bytes completo, split/delete/reorder de shots e slip. Preservar como decisões futuras
(BL036/BL028), sem tratá-los como regressões do port. F017/BL081 continua sendo uma feature do
NLE Rust, independente de split de shot no piloto. Não portar o servidor Node do Studio como arquitetura.

## Frentes concorrentes e dependências operacionais

Consulta GitHub nesta auditoria: única PR aberta no repositório é
[#31](https://github.com/journey-studios/tessel-editor/pull/31), câmera da captura SHARP nas
miniaturas (BL144.2). Coordenação export/miniaturas
[#30](https://github.com/journey-studios/tessel-editor/pull/30) e revalidação de performance
[#32](https://github.com/journey-studios/tessel-editor/pull/32) já constam no histórico integrado.
Essas entregas não fecham o catálogo/gestos do piloto. Estados de PR são um snapshot e devem ser
reconsultados antes de iniciar implementação. Não tomar suas branches nem modificar planos comuns.

O trabalho local prévio em `docs/backlog.md` e os dois planos não rastreados foi preservado;
a auditoria somente acrescenta uma nota própria ao final do backlog e um documento separado.
Não iniciar dois renderers em paralelo para provar FX: usar ownership do renderer já integrado
(BL144.10), scripts de lock de GPU e instância própria da janela.

## Sequência recomendada, vinculada ao backlog único

1. **Fechar a especificação de equivalência (BL036/BL018/BL019).** Selecionar o delta exato por
   capacidade, reabrir blobs da origem, fixar fixtures/goldens de FX, shots e gestos. Reconciliar
   matriz F016 e contrato effects.md com evidência por SHA (BL082.17). Não repetir gates pesados
   nesta tarefa documental. Saída: corpus reproduzível e tabela de critérios por fatia.
2. **Completar o uso da vinheta (BL082.6/.16).** Candidato vivo no slider, inserção no playhead,
   teclado e aviso de orçamento. Escopo pequeno sobre modelo já integrado; prova em lib, JSONL,
   janela e pixels. Esta é a próxima implementação recomendada após a especificação.
3. **Inspector no core e um segundo look (BL004/BL014/BL082.1).** Modelo neutro, labels e controles
   suficientes para `fade` ou outro look simples escolhido; cada entrada exige aceites próprios.
   Não cadastrar 58 nomes com implementação vazia. Tipos adicionais entram quando usados.
4. **Chaves e seed (BL082.2).** Easing, limites e avaliação determinística; depois ampliar os looks
   temporais, por exemplo filmGrain. Golden de estado e de pixel por efeito e frame.
5. **Escopo shot e primeira transição (BL082.3/.4).** Preparar dois decks, janelas/handles e
   normalização; começar por crossDissolve. Provar ausência/zero, troca de cena, resize, contexto
   perdido, cor e paridade monitor/export. Separar node scope como decisão futura.
6. **Edição de shots (BL082.5/BL022).** Contrato inicial sobre domínios suportados; recusar de forma
   explícita retime de domínio ainda não migrado. Ripple/roll completo só é aceito quando todos
   os dados dependentes migram corretamente. Áudio independente por padrão e atomicidade obrigatórios.
7. **Timeline e gestos por item (BL028/BL014/BL027).** Adapters sobre os contratos anteriores,
   sem regras na GPUI. Depois ampliar luz/clima/rig/narração e efeitos que dependem deles.
8. **Aceite por fatia (BL019/BL043/BL046/BL084).** Lib + processo JSONL real + UI semântica;
   monitor/export com referência independente. Registrar separadamente ponteiro/foco, áudio ouvido,
   GPU nativa, CI e merge. Um check verde de revisão não substitui nenhum desses resultados.

## Conferência documental desta entrega

- Catálogo extraído automaticamente: 31 looks + 28 transições, 59 IDs únicos.
- 1 ID presente nas tabelas Rust/runtime, 58 ausentes; não inferir custo pela contagem.
- Âncoras de código conferidas no SHA integrado; IDs reutilizados do backlog existente.
- Links locais dos novos documentos e diff conferidos; sem alteração de código ou dados privados.
- Nenhuma execução funcional nova e nenhuma declaração de aceite visual nova.
