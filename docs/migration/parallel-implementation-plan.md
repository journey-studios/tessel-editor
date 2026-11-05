# Rodada paralela da base de migração

Status: execução em andamento após o primeiro commit `59d618a`. Frente B entregue e
verificada pelo coordenador: 55 casos contra a origem e 21 testes do runner; consumidor Rust
ainda pendente. Frentes A e C seguem com ownership separado.
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
