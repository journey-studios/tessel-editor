# Integração da base — frentes A–E

> Relatório histórico da rodada A–E. F/G e H–L ampliaram o estado abaixo; consulte a
> [integração atual](round-hijkl-report.md) e o [backlog](../backlog.md) antes de atribuir pendências.


Estado: biblioteca de cena dirigida integrada; adapters e render dirigido ainda pendentes.
O documento canônico e o runtime existentes continuam com seus fluxos anteriores.
A–E não equivalem à migração completa do Scene Studio.

## Entregas e fronteiras

| Frente | Entrega integrada | Limite |
|---|---|---|
| A / F031 | Import/round-trip do bundle 1.0, referências e grafo/transformações 3D | Subconjunto de validação; famílias adiadas preservadas, sem interpretação |
| B / F032 | 55 bundles sintéticos, expectativas reais da origem e runner com proveniência | Corpus não cobre todos os domínios; consumidores Rust são parciais |
| C / F021 | Testes de contexto/proxy, ocultar/reabrir, cancelamento e export preservado | Perda forçada WebGL, não reset de driver nem medida de fóton |
| D / F033 | Tempo racional, câmera/keys/easing/camadas e gates temporais | Animação de nós retorna Unsupported; sem render dirigido |
| E / F034 | 8 propriedades tipadas, IDs, lote/dry-run, rollback | Sem documento/histórico/adapters; escrita de pose inicial difere da operação antiga |

Os lotes de E agora executam o importador, `edit_join_issues()` e `temporal_issues()`
antes de retornar o candidato. Três testes de regressão falharam antes dessa ligação e
passaram depois: junção estrutural inválida, salto temporal e FOV composto inválido.
A fixture base dos testes de edição recebe um corte explícito local; a original da A
continua exercitando o salto que a origem rejeita.

## Validação conjunta

- `cargo fmt --all -- --check`: passou.
- `./scripts/cargo check --workspace --locked`: passou.
- `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings`: passou.
- `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked`: **453 passaram,
  zero falhas, 2 ignorados**, em 34 grupos de resultados. Os ignorados são ZIP64 de 4,5 GiB
  (sink e disco), não foram acionados nesta rodada.
- `./scripts/cargo build -p tessel-editor --release --locked`: passou.
- Runtime JS: suíte 61/61 revalidada na integração C; produto JS não mudou desde então.
- Janela WKWebView com o release recém-compilado: **147 PASS / 0 FAIL**; 12 frames
  de export idênticos antes/depois dos incidentes. Sem cliques por coordenada.
- Não houve inspeção de screenshots nesta revalidação: pixels por hash/código de barras,
  estado semântico e export por framemd5. Não é prova de foco/tooltip ou foto na tela.

Dados duráveis: [prova da janela](../viewport-performance-results/lifecycle/foundation-window-proof.json)
e [proveniência](../viewport-performance-results/lifecycle/foundation-PROVENANCE.json).
O manifest registra hash do binário/runtime e de todos os fontes Rust/manifests usados
no working tree anterior ao commit de integração. Não atribui ao binário um commit de
fonte antigo. Cargo foi executado com `MBX_GC_MAX_TOTAL_SIZE=64GiB`, sem clean/gc manual.

## Próximas dependências

1. Completar avaliação de canais de performance dos nós, com comparação ao oráculo.
2. Anexar cenas ao documento/estado/histórico e expor os mesmos handlers na CLI headless
   e nos comandos semânticos da janela.
3. Plano de cena avaliado e recursos transitivos para runtime/save/package/export.
4. Árvore/inspector/gestos sobre o catálogo e comandos compartilhados.

A matriz registra UI/CLI pendentes nas novas features; nenhuma operação foi habilitada
como funcionalidade de editor completa a partir de testes da biblioteca apenas.
