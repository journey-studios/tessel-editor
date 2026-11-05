# F024 — Histórico: desfazer e refazer (P02)

## Status

Lib **V**, headless **V**, janela **V** (`history.*` por `ui op`; botões Desfazer/Refazer = `node:9`/`node:11`;
provado em `scripts/live-flow-proof.py`).

## Contrato

Undo/redo atua sobre **transações de domínio** por snapshot do modelo persistente (biblioteca, todas as
sequências, propriedades de clipe). Snapshot em vez de comando inverso: um inverso esquecido corromperia a cadeia
em silêncio. O custo é memória proporcional aos metadados × `HISTORY_LIMIT` (200 por direção); bytes de mídia
nunca entram num snapshot, então **desfazer não apaga arquivos** (um import desfeito remove só a referência;
originais e cópias gerenciadas ficam).

- Importação em lote, `project.import` e restauração são **uma unidade de undo**.
- Não são desfeitos: playhead, zoom, snap, seleção, busca e abas (não são edições).
- `history.undo`, `history.redo`, `history.status`. Pilha vazia → `nothing_to_undo` / `nothing_to_redo`, sem
  alterar o estado.
- Abrir ou criar um projeto zera a história; ela não é persistida no documento.
