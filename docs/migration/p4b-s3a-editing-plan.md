# P4b S3a — edição de transições no core com admissão fechada

Estado: plano do coordenador para execução Windows com Cursor Sonnet 5.5, sem opções adicionais.
Base mínima integrada: main `90609da002871c90d9c950d02df725307456ac18` (S1b/PR63).
Backlog: BL082.4.5 (S3a), incluindo BL082.4.15 (IDs) e diagnóstico remanescente BL082.4.24.
Contrato canônico: `docs/migration/p4b-cross-dissolve-plan.md`, §§5–9,12–14; F064.
O link da delegação, a base exata e o ownership ficam em `docs/backlog.md` pelo coordenador.

## Resultado desta entrega

Editar `crossDissolve` pela biblioteca e pelo executor compartilhado, com lote atômico,
validação única do core, dry-run, no-op, revisão, undo/redo, problemas localizados e
predicado puro para os overlays futuros. O documento candidato retém a política de admissão.

A entrada pública CONTINUA recusando transições até S2A. O processo `tessel-headless` real
prova a recusa das novas operações; o positivo interno é exercitado na biblioteca e no
executor em memória com cenas `Accept`. Isso não é prova positiva do processo público.
Não há controle de janela habilitado, preview real, runtime, export ou efeito novo nesta fase.
F064 permanece P no produto. Não implementar S2A, S3b, S4, P5 ou P6 nesta entrega.

## Decisões fixadas

1. As operações usam o mesmo `scene.fx.edit`/`FxOp`/`apply_fx` e o mesmo candidato reimportado.
   Não existe editor paralelo, estado de CLI separado ou validação de transição no desktop.
2. `Refuse` não é uma barreira de segurança; é o portão de compatibilidade. Não adicionar
   flag de ambiente, argumento de processo ou caminho de projeto que o contorne.
3. A cena `Accept` dos testes fica interna como na S1a; nenhuma ocorrência nova de `Accept`
   em produção fora das portas já autorizadas do importador. Não abrir o catálogo público.
4. Lotes mistos de looks e transições são legais pela edição direta; falha em qualquer op
   recusa o lote todo. Gesto existente de LOOK não pode aceitar uma op de transição como
   atalho: recusa de intenção/alvo, preservando candidato, último legal e revisão.
5. No-op usa resultado líquido e valores tipados. Não normalizar JSON autorado ou apagar
   membros desconhecidos por uma edição que não os solicita. `ops:[]` é identidade exata.
6. `hidden:false` é uma operação válida, mas no documento significa OMITIR `hidden`, porque
   o importador aceita somente ausência ou `true`. Não serializar um `false` inválido.
7. IDs de tracks, clips e transições dividem o namespace FX; IDs de nós/assets não entram
   nesse namespace. O alocador considera transições antes de gerar IDs (BL082.4.15).
8. Preservar IDs existentes no upsert por corte. Se um `id` explícito contradiz o ID de uma
   transição existente, recusar de forma tipada; não renomear nem ignorar em silêncio.
9. Orçamento é aviso, nunca recusa e nunca remove passes. Todas as respostas usam a regra
   `pass_budget_in(shots, edit_duration)` já unificada na S1b.
10. As regras do importador, incluindo janela, família, quotas, tipos e handles, são a fonte
    única. Não mudar os fixtures/goldens S0 nem editar a origem para tornar testes verdes.
11. O problema da dica em cameraIn além do fim é desta entrega (.24): não basta recusar;
    `details.lacks` e a dica devem conduzir a uma correção suficiente e explicar os limites.
12. As decisões de API concretas e diferenças em relação ao plano canônico são registradas
    ANTES do código correspondente; decisões rotineiras não exigem confirmação do usuário.

## API e semântica

Envelope existente: `scene.fx.edit {scene, ops, dry_run?, base_revision?}`.
O parser compartilhado continua no core; biblioteca recebe as mesmas operações tipadas.
Os campos abaixo seguem o plano canônico. Não criar aliases silenciosos camel/snake.

| Op | Campos | Comportamento |
|---|---|---|
| set_transition | fromShotId,toShotId,effect; id?,durationFrames?,alignment?,handles?,easing?,params?,hidden? | Upsert por corte; novo aplica padrões; existente mantém id e campos omitidos |
| resize_transition | id,durationFrames; alignment? | Altera a janela; reimportação julga vizinhança, edição, sobreposição e handles |
| set_transition_param | id,param,value | Apenas parâmetro do catálogo da família; curve e intensity tipados |
| set_transition_hidden | id,hidden | Bool real; false remove membro, true grava true |
| remove_transition | id | Ausente é UNKNOWN_TRANSITION, não sucesso silencioso |

No upsert existente, `params` faz merge SOMENTE das chaves fornecidas: escrever
`{curve:"linear"}` preserva `intensity` e membros desconhecidos já autorados. Ausência de
`params` preserva o objeto inteiro. Não substituir o objeto por um pacote de defaults.

Padrões novos: center,hold,easeInOutCubic,smooth,intensity=1; duração típica24 limitada a
`maxTransitionFrames(alignment, fromLen, toLen)`. Máximo0 recusa com TRANSITION_TOO_LONG.
Não duplicar fórmula nem fazer clamp de um valor explícito ilegal para aceitar a edição.
Com curve=linear, intensity é gravado mas não altera a mistura; não inventar outra fórmula.
Só existe crossDissolve; troca entre dois efeitos é N/A até outro efeito existir.
Keys de transição continuam recusadas e node scope não entra nesta entrega.

`base_revision` obsoleta continua stale_revision antes de gravar. Edição real com alteração
cria UMA entrada de undo; dry-run e no-op não incrementam revisão nem histórico. Undo/redo
restaura documento, transição, orçamento e planos internos. Guardas de tamanho continuam.

## Primeiro passo: conter os arquivos grandes

Na base S1b, `scene_edit/fx.rs` tem cerca de1235 linhas e `ops.rs` cerca de1217.
Não adicionar novos blocos a esses arquivos sem EXTRAÇÃO mecânica primeiro, com delta líquido
menor ou igual a zero nos arquivos grandes. `commands.rs`, `state/mod.rs`, inspector e
scene_fx desktop também são grandes e não são alvo de novas regras nesta entrega.

Extrair parser/tipos ou outra unidade coesa de `scene_edit/fx.rs` para módulos irmãos
(ex.: fx_parse.rs/fx_types.rs), mantendo reexports e sem alterar comportamento. Escolher
pela dependência real do código, não por um número arbitrário de linhas. Um commit separado
permite verificar a extração contra os testes existentes antes de acrescentar as operações.
Não criar crate nem dependência nova sem necessidade concreta. Nenhum módulo novo >1000linhas.

## Ownership

| Área | Permitido | Limite |
|---|---|---|
| Domínio de edição | scene_edit/fx_transition.rs e novos módulos coesos; fx.rs após extração; scene_edit/mod.rs | Um pipeline/candidato por lote, sem reimportar ou clonar todo o documento por op |
| Parsing | Parser de FxOp extraído e ganchos existentes; ops.rs só se inevitável com delta<=0 | Outer scene.fx.edit já existe; não inventar outro comando |
| Projeto | scene_project/fx.rs; admissão/erros e leitura interna necessários | Reimport preserva política; catálogo público e planos sem transição ficam iguais |
| Gesto de look | scene_project/fx_gesture.rs e scene_edit/fx_intent.rs | Guarda pequeno de classificação para recusar op de transição, preservando intenção e candidato; sem novo gesto de transição |
| Regras | directed_scene/fx/window_guard.rs; transition_explain.rs para diagnóstico .24; hooks pequenos | Sem runtime, sem ativação do portão, sem duplicar matemática de janela |
| Testes | scene_fx_transition/edit*.rs, testes in-crate de executor/admissão, processo headless negativo | Fixtures próprias pequenas; sem regravar o acervo de origem |
| Docs | plano e relatório S3a próprios, fx-transitions.md | Backlog, matriz e índice são patch para coordenador; não editar comuns no filho |

Extensão mecânica de ownership para match exaustivo é permitida se inevitável, registrada
no relatório e sem regra de negócio nova no adapter. Não tocar renderer, runtime, mídia,
gizmo/alças reais, UI de transição, origem JS, outras branches ou artefatos alheios.

## Diagnóstico e localização

Produzir problemas do core com code,message,hint,transitionId?,shotId?,details.
Distinguir código do envelope CommandError de código de Issue; testar e documentar ambos.
Usar uma representação única para biblioteca, dry-run e futuro inspector, com textos dos
códigos novos em pt-br,en,es. Não montar texto no desktop nem usar braço padrão silencioso.
Fixar uma função pura do core que recebe `Locale` e projeta esses problemas; testar as três
línguas e documentar sua assinatura. O envelope `scene.fx.edit` não ganha `locale` nesta fase:
o executor usa inglês, como hoje; S3b poderá consumir a mesma projeção com seu locale.
No erro de transição do executor, acrescentar `details.errors[]` com essa forma localizada
em inglês, preservando `details.issues` e todos os códigos/formatos existentes de looks.
Sucesso e no-op não ganham campos de erro vazios; dry-run recusado continua sendo recusa,
sem transformar erro em sucesso. Documentar os dois níveis de código e o caminho exato.
Essa decisão de envelope, que concretiza o `errors[]` esquemático do plano canônico, deve
constar do contrato F064 antes do código. Não alegar seleção pública de idioma no JSONL.

Para .24, considerar limites inferior E superior da janela real, sem confundir cabeça
contígua com quantidade de frames a acrescentar. Testes mínimos:

- cameraIn60,cam48,before6: sugestão de mais6 é insuficiente; a correção indicada precisa
  permitir todos os frames54..59. Verificar aplicando a sugestão ao fixture e reimportando.
- cameraIn49,cam48,before6: 1 frame no fim resolve esse limite superior; não chamá-lo de
 6 frames ausentes sem explicar que é disponibilidade contígua, não custo de reparo.
- cameraIn48,cam48: há cabeça real; não recusar igualdade do end exclusivo.
- cameraIn menor que before: faltam frames antes do zero; estender só o fim não resolve.
- before=0 não exige cabeça; comprimento desconhecido mantém a política existente.

Não enfraquecer a admissão para simplificar a dica. Se a representação atual de `lacks` não
separa as duas faltas, ampliá-la de forma explícita e testada, com compatibilidade documentada.
Não basta usar max(before-head,cameraIn-total) e afirmar que é o reparo mínimo de todos os casos.

## Predicado puro e shot apagado

Implementar o predicado único do §9.6 do plano, reaproveitando a mesma escolha de janela
usada por resolve_frame. Testar início/fim meio-abertos, fora da janela, transição hidden,
ordem de consulta e mudanças de frame sem estado interno. Nomes finais na API do core.
Nenhum consumidor de overlay real é ligado nesta fase: S4a aplicará essa decisão.

Verificar como o dono de uma entidade e o frame explícito são resolvidos hoje. Uma propriedade
estática sem chave não depende do frame; não inventar gravação de chave fora do shot do dono.
Registrar a regra que S4a/S3b usará para alvo do vizinho; não implementar controles antecipados.

Não existe editor de shots/P5 aqui. Provar pelo importador do candidato que remover um shot
referenciado, encurtá-lo ou quebrar vizinhança recusa o documento sem cortar transições.
Testar substituição/anexo pelo handler existente quando aplicável. N/A explícito para comando
de shot inexistente; não falsificar uma edição P5 para completar a matriz.

## Matriz de aceite

| Caso | Prova exigida |
|---|---|
| E01 | Biblioteca: criar com padrões em pelo menos3comprimentos e alinhamentos; janela calculada pelo core |
| E02 | Upsert preserva ID, autoria e campos omitidos; params parcial preserva intensity/unknown; ID explícito divergente recusa |
| E03 | Resize e todos os parâmetros legais; linear preserva intensity sem mudar fórmula |
| E04 | Hidden true/false e persistência do membro omitido; remover último e manter looks/unknown fields |
| E05 | No-op tipado, lote vazio e ida-e-volta líquida sem revisão/undo |
| E06 | Dry-run válido/recusado: documento/revisão/histórico/seleção intactos; mesmos problemas do real |
| E07 | Lote misto looks/transições, um undo; um inválido no meio recusa tudo; undo/redo exatos |
| E08 | base_revision obsoleta e cena/projeto trocado não aplicam candidato antigo |
| E09 | IDs tracks/clips/transições colidentes e reservados case-insensitive; geração não colide (.15) |
| E10 | Quotas1024ops antes da cópia, demais limites existentes; substituir/remover libera quota |
| E11 | Janela fora da edição, sobreposição, shot inexistente/não vizinho, família errada, tipos estritos |
| E12 | Quantidades/dicas .24 coerentes; aplicar o reparo indicado torna o caso válido; 3línguas |
| E13 | Orçamento edit/list/frame/inspect/gesto usa uma regra e aviso, nunca corta passes |
| E14 | Predicado de janela puro, hidden e bordas; nenhuma regra duplicada no adapter |
| E15 | Gesto de look recusa op de transição sem mudar último candidato legal; `gesture_op_not_allowed` existente é válido, sem exigir outro código artificial |
| E16 | Reimport/undo/redo preservam Accept interno; público Refuse nunca vira Accept |
| E17 | tessel-headless REAL: anexar cena comum válida, tentar set_transition pelo scene.fx.edit e provar FX_TRANSITION_NOT_YET no erro, exit1 e estado/histórico intactos; não substituir isso por unknown_scene após attach recusado |
| E18 | Planos/catálogo públicos e baseline sem transição continuam iguais; sem novo controle enabled |
| E19 | Shot removido/encurtado/não vizinho recusado pelo candidato, sem consertar o documento em silêncio |
| E20 | Contrato/report por superfície, acervoS0 intocado, tree core sem gpui/wry |

Os testes positivos internos não viram prova positiva de janela/processo sem admissão.
Mutantes discriminantes: tirar gate; perder política no reimport; omitir IDs de transição;
perder campos omitidos; gravar hiddenfalse; ignorar base_revision; aplicar parte de lote;
criar undo no no-op; tirar limite1024; janela end inclusivo; dica60/48insuficiente.
Rodar mutantes proporcionais e restaurar fontes por bytes/hash e testes verdes após restauração.

## Sequência de execução e gates

1. Confirmar host Windows, origin, base, branch e status limpo, espaço/mbx. Ler os contratos e
   registrar o plano no repo antes da primeira implementação. Não tocar a origem.
2. Extração mecânica e gates focados de FX; commit independente, sem mudar API/saída.
3. Ops tipadas/parser e regra de edição; biblioteca/limites/IDs/atomicidade/diagnósticos.
4. Fiação do executor/dry-run/revisão/undo, processo negativo, predicado e contratos de dono.
5. Gates Windows: fmt; check workspace; clippy alltargets-Dwarnings; no-run/build desktop;
   coreFX/transições e baseline debug/release; testes próprios/lib/executor e GPUI de regressão;
   cargo tree sem gpui/wry. Não declarar workspace global verde: W1 ainda tem2gizmos base.
6. Não rodar engine/GPU/performance por esta entrega. Se alguma prova exigir runtime real,
   respeitar setup e lock; nunca transformar skip em aprovação. Não repetir stress do ledger.
7. Relatório com SHAs de código/docs, comandos/exits/contagens, mutantes mortos e sobreviventes,
   execução versus leitura versus N/A, limites, owner extensions e patch de backlog/matriz.
8. PR contra main; push sem force; link_pull_request como próxima ferramenta após criar/push.
   Filho não faz watch/merge nem espera bots. Última linha: PR URL, head, checks/reviews.

Usar Git Bash REAL `C:\Program Files\Git\bin\bash.exe` para `./scripts/cargo`; PATHbash pode ser WSL.
Target padrão/mbx; nunca CARGO_TARGET_DIR, target-dir, lane, clean/gc ou build externo emtmp.
Não apagar cache, staging, processos ou TEMP alheios. Artefatos próprios pequenos, <500MB emtmp.
Commits convencionais, imperativo, título<=70/linhas<=100, staging sóowned paths; sem referência
ou trailer de assistentes. Não force-push. A publicação não fecha aceite de produto ou humano.
