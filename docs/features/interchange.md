# F027 — Importar conteúdo de outro projeto (P08)

## Status

Lib **V** (`crates/core` + `crates/media/tests/interchange.rs`), `scripts/media` **V**, UI **V** (diálogo
"Importar de projeto"; provado com IDs colidentes em `scripts/live-flow-proof.py`).

## Contrato

É um **merge**, nunca um open: o destino mantém suas entidades; tudo que vem da origem recebe um ID livre no
destino (o da origem quando está livre, uma variante numerada quando colide) e **todas** as referências são
reescritas com o mesmo mapa (clipe → asset, cena → splat, overrides). A origem é só lida. O merge inteiro é uma
transação — falha deixa o destino intacto, sucesso é **um passo de undo**.

| `op` | Campos | Notas |
|---|---|---|
| `project.import` | `source` (documento ou pacote), `sequences?`, `dry_run?`, `wait?` | `dry_run` devolve o plano `{sequences, assets, clips}` de pares origem→destino e avisos, sem mudar nada |

- Cada sequência importada mantém seu FPS/canvas numa sequência própria do destino. Não há mesclagem de tracks
  numa sequência existente nesta versão, nem arredondamento de tempo.
- Bytes: copiados para o workspace do destino, deduplicados por hash (`copied`/`reused` no resultado).
- Fonte em pacote: verificada e extraída para uma pasta temporária dentro do workspace de destino, removida ao fim.
- Erros: `unportable_media` (asset sem mídia utilizável), `no_workspace` (salve o destino antes), `invalid_package`,
  `integrity_error`, `unknown_element` (sequência inexistente na origem).

## Limites

Só entre projetos Tessel (não Premiere/Resolve/FCP). Sem seletor de tracks/clipes individuais: a unidade é a
sequência.
