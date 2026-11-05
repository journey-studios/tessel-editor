# Rodada paralela da base de migração

Status: execução em andamento após o primeiro commit `59d618a`. Frente B entregue e
verificada pelo coordenador: 55 casos contra a origem e 21 testes do runner; consumidor Rust
integrado parcialmente na frente A (55 vereditos, grafo e matrizes estáticas). A entregue e
revisada, com validação temporal/adapters pendentes. C entregue: suíte runtime 61/61
e prova de janela real 147/147, repetidas pelo coordenador contra release existente.
Referência: [building blocks e fases](phased-roadmap.md).

## Contrato de integração

- O documento canônico continua `.tessel.json`; bundle legado é intercâmbio versionado,
  com adaptação explícita, sem substituir o modelo atual silenciosamente.
- Domínio e validação no core Rust sem GPUI. UI e CLI reutilizam o mesmo contrato.
- O formato da origem é lido no snapshot do inventário; não inventar campos ou copiar mídia privada.
- Começar pequeno: planos em camadas, hierarquia e referências, transformações 3D, shot/câmera.
  Tipos ainda não suportados devem ser preservados ou rejeitados de forma explícita.
- Avaliação temporal, catálogo de propriedades e render de cena começam após integrar os tipos.
- Nenhum teste lido é prova de execução; cada superfície tem evidência e status próprios.

## Frentes independentes

| Frente | Entrega | Arquivos sob responsabilidade | Limite |
|---|---|---|---|
| A — Documento e grafo (BB01/BB02) | API pública mínima de import/validação/round-trip de bundle e grafo com IDs, referências e transformações; testes sem GPU | `crates/core/src/directed_scene/`, export em `crates/core/src/lib.rs`, testes próprios, `docs/features/directed-scene.md`, relatório A | Não alterar runtime, comandos ou documento canônico; adapters/UI ficam pendentes explícitos |
| B — Oráculo (BB10/E0) | Corpus sintético independente derivado da origem, casos válidos/inválidos e saída esperada de funções puras, proveniência e runner reproduzível | `scripts/migration-oracle/`, `docs/migration/fixtures/`, relatório B | Não alterar core/runtime nem instalar na origem; manter corpus independente de A |
| C — Lifecycle de preview | Provas e correções para perda de contexto com proxy, ocultar/reabrir monitor/janela e recuperação/cancelamento | runtime, testes de viewport existentes e harness dedicado; `docs/features/viewport.md`, relatório C | Não alterar modelo de cena nem inventar métricas de GPU/fóton |

A matriz `docs/features/README.md` e índices gerais serão consolidados pelo coordenador
na integração; cada frente fornece a linha exata em seu relatório, com status honesto.
O T3 desta conversa está vinculado ao tesselapp e recusou `target.projectId` local
para worktrees de outro projeto (`invalid_request`). Nesta rodada, os agentes trabalham
no checkout existente do tessel-editor, com ownership exclusivo dos arquivos acima.
Somente o coordenador faz staging/commits; agentes não alteram branches, index ou remotes.
Cada frente informa arquivos, comandos, resultados, limitações e dependências.
Sem push, PR ou merge pelos agentes.

## Recursos e coordenação

- A e B fazem validação sem GPU; C tem exclusividade de benchmark GPU nesta rodada.
- Usar `./scripts/cargo` e cache mbx compartilhado; serializar Cargo no checkout compartilhado.
  A é dona da compilação Rust enquanto C começa pelos testes JS; C deixa checks Rust
  para o coordenador integrar, evitando contenção com A.
  Em builds usar `MBX_GC_MAX_TOTAL_SIZE=64GiB`; não executar clean/gc.
- Manter pelo menos 6 GiB livres, limitar artefatos temporários a 2 GiB por frente e
  limpar apenas os próprios temporários. Não reinstalar toolchain por trocar HOME.
- Não tocar no editor/projeto aberto do usuário; instâncias isoladas preservam
  `RUSTUP_HOME` e `CARGO_HOME` reais e não executam wrappers sob HOME alternativo.
- Coordenador revisa diffs, integra commits, resolve exports/matriz/índices e executa
  verificações conjuntas antes de liberar a segunda rodada BB03/BB04/BB05.

## Aceite da rodada

1. Primeiro commit e remoto privado disponíveis antes de abrir worktrees.
2. APIs públicas de A com testes positivos, erros e invariância/round-trip definido.
3. B gera expectativas a partir do código da origem, com commit e hashes verificáveis.
4. C demonstra recuperação com frames reais; limitações de hardware ficam pendentes.
5. Cada relatório distingue lib, CLI sem janela, socket e UI; não declarar a migração completa.

## Rodada 2 — após integrar A/B (`72fa715`)

A base de documento/grafo está integrada. Seguem em paralelo, sem mudar APIs existentes:

| Frente | Responsabilidade | Ownership |
|---|---|---|
| D — BB03 | Avaliação pura de tempo de shot/instância/clipe de câmera; recipes, keys, easing, camadas; gates de junção temporal; comparação com oráculo | `directed_scene/temporal/`, `directed_scene/mod.rs`, `joins.rs`; ajustes aditivos em `model.rs`/`parse.rs` somente se necessários; testes e docs próprios |
| E — BB04 | Catálogo mínimo de propriedades de nó/câmera e edição por IDs em lote/dry-run com rollback; usa importador existente, sem regras exclusivas da UI | `crates/core/src/scene_edit/`, export em `lib.rs`, testes e docs próprios |

D não altera `lib.rs`; E não altera módulos internos de `directed_scene`. Ambos consomem
a API pública existente sem a quebrar. D executa os checks Cargo; E implementa e informa
checks Rust para o coordenador executar na integração, evitando concorrência pesada.
C continua com GPU/runtime exclusivamente. Matriz/índices e commits ficam com coordenador.

Esta rodada ainda é biblioteca de base: adapters, ligação ao documento/estado do editor e
render dirigido serão etapas seguintes, com status pendente explícito. E não certifica
validação temporal antes de consumir o gate D; o coordenador integra esse gate após ambas.

### Status da rodada 2

D entregue e revisada: 19 testes públicos temporais, comparação de câmera/gates com o
oráculo, teste dos limites de tolerância e gerador com proveniência validada. E entregue
e revisada: gate estrutural/temporal integrado aos lotes e 29 testes públicos via Cargo.
UI/CLI e animação de nós continuam pendentes.

## Rodada 3 — avaliação completa dos nós e acesso programático

Base integrada `52f3175`: A–E, workspace verde (453 testes, 2 ignorados), release e
lifecycle 147/147. Sem adapters de cena dirigida ainda.

| Frente | Entrega | Ownership |
|---|---|---|
| F — BB03 nós | Validar/evaluar canais de performance, estado de nós e matrizes por frame, com oráculo; completar `evaluate_frame` para esse subconjunto | Internos de `directed_scene/` e novos testes/fixtures; docs próprias |
| G — Documento/histórico/CLI | Anexar cenas ao projeto canônico de forma aditiva; importar/consultar/editar/salvar/reabrir/undo via handlers compartilhados e JSONL headless | `document`, `state`, `ops`, `headless` e módulo adapter novo; export `lib.rs`, testes/fixtures; docs próprias |

F não altera `lib.rs`, documento, state, ops ou headless; G não altera internos de
`directed_scene` ou `scene_edit`. APIs públicas existentes permanecem compatíveis.
G usa `scene_edit::apply` com gates embutidos e não duplica regras de câmera/animação.
F tem o slot de compilação Rust; G deixa testes preparados para o coordenador, sem
compilar cópia paralela com rustc. Nenhuma frente usa GPU.

F expõe estado avaliado serializável/consultável, mas não altera runtime. G mantém
carregamento de projetos antigos e undo/redo existentes, sem segundo histórico.
Nenhuma delas habilita render de bundle legado, copia mídia privada ou implementa
famílias visuais M2–M8. Preservar recursos no documento não significa transportá-los
no pacote: dependências transitivas e ligação ao renderer ficam para BB05/BB06.

IDs reservados: F035 (avaliação dos nós), F036 (cenas no projeto e acesso programático).
Matriz/índices, commits e revisão do conjunto continuam com o coordenador.

### Status da rodada 3

F entregue e revalidada pelo coordenador: 29 testes públicos de animação, gerador verificável
e round-trip JSON de binary64 sem perda. G segue com documento/histórico/CLI em andamento.
Nenhuma dessas provas equivale ainda a render dirigido na UI.
