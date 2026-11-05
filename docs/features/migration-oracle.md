# Oráculo de equivalência da migração (BB10 / E0)

## Objetivo e limites

Ferramenta de **desenvolvimento** (não é uma capacidade do editor): fornece, para a migração do
Scene Studio, um corpus sintético de bundles `render-bundle.json` válidos e inválidos e, para cada
um, o resultado produzido pelas **funções puras reais** de `packages/scene-engine` (validação,
referências e grafo de cena, matrizes mundo, avaliação do frame de edição e pose de câmera nos
frames de borda). Serve de gate de equivalência para BB01–BB03 (documento, grafo, avaliação) sem
GPU e sem depender de nenhuma API Rust.

Limites honestos:

- Executa **parte** da origem (33 arquivos `.ts`, empacotados sem alteração com zod 4.3.6 e three
  0.187.0). Não é a engine completa nem o runtime; não há pixel, áudio, preflight de arquivo nem
  I/O. Lista de executado / só carregado / não executado / lacunas do corpus:
  [`harness/scope.json`](../../scripts/migration-oracle/harness/scope.json).
- O snapshot é o baseline fixado no inventário (`d6adac15…`), não o branch piloto.
- Cobertura por linha **não** foi medida; "executado" = chamado pelo harness ou exercitado por
  essas chamadas nas entradas do corpus.
- Nenhuma regra esperada foi reescrita aqui; o Rust implementa a regra e o oráculo só informa o
  que a origem responde.

## Contrato do core / biblioteca

O consumidor Rust `crates/core/tests/directed_scene_oracle.rs` está integrado (**Lib: Parcial**):
55 vereditos de importação + junções estruturais, referências pai/socket dos casos válidos,
round-trip e matrizes estáticas. `directed_scene_temporal.rs` acrescenta a comparação de câmera,
mapeamento de frames e gates temporais (F033); a F035 compara também performance dos nós e matrizes animadas. O contrato de dados é:

- `docs/migration/fixtures/bundles/<id>.bundle.json` — entrada.
- `docs/migration/fixtures/expected/<id>.expected.json` — veredito (`validation.accepted`, issues com
  `sourceCode`, `editJoins[].code`), `evaluation.parentLinks`, `worldMatrices`, `editFrames[].state`,
  `cameraClips[]` com poses e velocidades.
- `docs/migration/fixtures/cases.json` — índice (`kind`, `bundle`, `expected`, `intent`).
- `docs/migration/fixtures/manifest.json` — proveniência (commit, hashes, versões).

Pré-condições de consumo e política numérica (o oráculo grava doubles exatos; o consumidor Rust compara
matrizes com `abs <= 1e-10 + 1e-12 * abs(expected)`) estão no [README do runner](../../scripts/migration-oracle/README.md) e no consumidor
[directed_scene_oracle.rs](../../crates/core/tests/directed_scene_oracle.rs). Câmera e animação usam
`1e-9 + 1e-12 * abs(expected)`; IDs, frames e códigos são exatos.
Rejeição: um bundle recusado tem `evaluation: null`; um Rust equivalente não deve avaliá-lo.

## CLI headless

Não se aplica ao `tessel-headless`: o runner é Node, fora do editor.

```sh
node scripts/migration-oracle/oracle.mjs setup      # deps fixadas, fora do repo (pnpm)
node scripts/migration-oracle/oracle.mjs verify     # gate: hashes + regeneração byte a byte
node scripts/migration-oracle/oracle.mjs generate   # regenera expected/ e manifest.json
node --test scripts/migration-oracle/oracle.spec.mjs
```

`verify` sai com `1` em corrupção/deriva/divergência e `2` em ambiente (origem ou deps ausentes).

## UI e CLI da janela

N/A: sem superfície de editor. Nenhum controle, comando `ui`/`headless` ou nó de `tree.rs`.

## Evidências e status

- `node --test scripts/migration-oracle/oracle.spec.mjs`: **21/21** — verify nas fixtures
  versionadas; manifest com commit e hashes; corpus versionado == builder; expectativa adulterada
  (com e sem "consertar" o manifest); veredito invertido; bundle adulterado; arquivo ausente/não
  listado; manifest ausente; deriva da origem por mudança de comportamento (hash **e** divergência) e
  por comentário (só hash); origem ausente (exit 2); `generate` recusando origem não provável e
  arquivo diferente do commit (clone `--shared` somente leitura); regeneração restaurando bytes
  idênticos (apagando expectativas+manifest e corrompendo uma); caso com intenção não produzida pela
  origem; CLI e ausência de `node_modules` no repo.
- Idempotência: `generate` duas vezes produz hash idêntico de `expected/` + `manifest.json`.
- Origem: working tree dos 33 arquivos executados == blobs de `d6adac15…`; árvore
  `packages/scene-engine` == `d5fc9871…` (a do inventário).
- Não executado: Cargo, Chrome, GPU. O consumidor Rust integrado passou nos 3 testes adicionais; a coluna Lib é parcial
  porque os demais domínios ainda não são comparados. Performance/matrizes animadas são
  comparadas na F035; a câmera/gates temporais
  têm comparação adicional na F033 (19 testes públicos).

Linha da matriz (a ser consolidada pelo coordenador) e relatório:
[`docs/migration/foundation-b-report.md`](../migration/foundation-b-report.md).
