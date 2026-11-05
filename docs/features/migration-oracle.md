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

Não há API Rust nesta entrega (**Lib: pendente**). O que existe é um contrato de dados estável em
caminho, consumível por testes Rust futuros:

- `docs/migration/fixtures/bundles/<id>.bundle.json` — entrada.
- `docs/migration/fixtures/expected/<id>.expected.json` — veredito (`validation.accepted`, issues com
  `sourceCode`, `editJoins[].code`), `evaluation.parentLinks`, `worldMatrices`, `editFrames[].state`,
  `cameraClips[]` com poses e velocidades.
- `docs/migration/fixtures/cases.json` — índice (`kind`, `bundle`, `expected`, `intent`).
- `docs/migration/fixtures/manifest.json` — proveniência (commit, hashes, versões).

Pré-condições de consumo e política numérica (nenhuma tolerância é definida pelo oráculo; as únicas
são constantes da origem) estão no [README do runner](../../scripts/migration-oracle/README.md).
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
- Não executado: Cargo, Chrome, GPU. Não há consumidor Rust ainda, por isso a coluna Lib é pendente.

Linha da matriz (a ser consolidada pelo coordenador) e relatório:
[`docs/migration/foundation-b-report.md`](../migration/foundation-b-report.md).
