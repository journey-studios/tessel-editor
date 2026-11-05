# F014 — Dock, tema e janela

## Objetivo e limites

Apresentação desktop: organizar/redimensionar painéis, alternar tema e controlar a janela.
Não são operações de edição de mídia. Não requerem equivalentes falsos de janela na lib.

## Biblioteca e CLI headless

`state::layout` contém tipos e validação pura; a execução do Dock depende do DockArea vivo.
O core não armazena uma segunda cópia do dock. Execução headless de alvos de layout/tema/janela
é N/A e retorna `desktop_required`. A validação pura de layout é reutilizável pela lib.

## UI e CLI da janela

```sh
./scripts/ui dock
./scripts/ui dock move inspector --to bottom
./scripts/ui dock resize left 300
./scripts/ui theme dark
./scripts/ui get window
```

UI: cabeçalhos e divisórias do Dock, tema no toolbar e botões nativos de janela.

## Evidências

`crates/desktop/src/workstation/dock_tests.rs`, `crates/core/src/state/layout.rs` e
`docs/gpui-kit-migration-report.md`. Drag manual dos painéis continua sem validação nesta entrega.
