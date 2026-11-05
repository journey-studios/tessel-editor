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
- **Gesto de slider (BL062):** o arrasto de um slider de propriedade é **uma** entrada (valor de antes do
  arrasto → valor ao soltar). O preview aplica o valor ao vivo sem gravar nem apagar o redo; só o `commit`
  grava, e nada é gravado se o valor não mudou ou voltou ao original. Um gesto aberto é confirmado antes de
  qualquer outra edição, `history.undo|redo`, lote ou troca de sequência, então o preview não atravessa outro
  commit. Aparece em `history.status` como `history.gesture`. Um preview move a revisão do projeto, o que
  invalida um gesto de cena (`scene.gesture.*`) aberto (`gesture_invalidated`). Ver
  [F005](properties.md#gesto-de-slider-bl062-preview-ao-vivo-uma-entrada-no-undo).
- Abrir ou criar um projeto zera a história; ela não é persistida no documento.

BL062: agrupamento de sliders passou em 28 testes core, 13 testes GPUI e executável JSONL. Preview não aumenta undo; commit gera 1; undo/redo foram conferidos. Prova física de ponteiro permanece separada.
