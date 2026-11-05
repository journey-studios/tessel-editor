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

## Corpus estendido (K / BL018, rodada 2)

Corpus isolado de 47 casos em `docs/migration/fixtures/extended-round2/` (o de 55 casos segue
intocado), gerado pela mesma regra: funções reais da origem `d6adac15…`, sem reimplementar. Famílias:
fronteiras de junção (abaixo/igual/acima de `JOIN_POSITION`, `JOIN_FOV`, `JOIN_VELOCITY`,
`JOIN_FOV_VELOCITY`, diagonal, shots fora de ordem, FPS NTSC), FPS 24000/1001, 30000/1001,
60000/1001, 25, 30, 60, 48/2 e 2000/1, avaliação fora de ordem (7 ordens) e matrizes mundo (gimbal,
escalas extremas, sockets encadeados, poses `lookAt` da câmera). Comandos, formato e limites:
[`scripts/migration-oracle/README.md`](../../scripts/migration-oracle/README.md#corpus-estendido-k--bl018-rodada-2).

- **Consumidor Rust**: `crates/core/tests/directed_scene_oracle_extended.rs` (API pública apenas).
  Compara veredito e códigos por junção (com as distâncias da origem contra os limites, nas mensagens),
  poses nos dois lados de cada junção, 468 quadros (estado, nós e `worldMatrices`), as 7 ordens de seek
  (e que o Rust responde igual a cada vez), `CameraPose::to_world_matrix` (pose da origem e pose do
  Rust) e `Rate`/`Ratio`/`frame_at_time` contra segundos→frame. Tolerâncias repetidas em constantes e
  conferidas com `tolerances.json`. Inclui 6 testes de controle negativo (tolerância exata, veredito invertido
  com distâncias na mensagem, comportamento de câmera alterado, matriz mundo perturbada e minúscula,
  matriz transposta, taxa errada, regra do artefato não esconde frame errado).
- **Divergência documentada**: fps acima de 1000 é aceito pela origem; o importador Rust recusa com
  `FPS_OUT_OF_RANGE` (asserido, não ignorado).
- **Artefatos da origem**: 14 sondas em que `Math.floor(s·rate)` não é o piso exato; o Rust (racional
  exato) deve igualar o piso exato nelas e a origem no resto. O número é fixado no teste.
- **Matriz mundo da câmera**: a origem a calcula no método privado `ThreeSceneRuntime.drawEditFrame`
  (precisa de WebGL); a fixture usa `PerspectiveCamera.lookAt` do `three` fixo sobre a pose avaliada
  pela origem (`source: "three-library"`).
- **Evidência executada (Node, sem Cargo)**: `extended.mjs verify` passa (47 casos, a origem a
  `d6adac15…`); `oracle-extended.spec.mjs` 24/24 e `oracle.spec.mjs` 21/21 (45/45); o `verify` do corpus
  base segue passando byte a byte. **O teste Rust foi escrito, mas não foi compilado nem executado.**

Linha da matriz (F032 existente, a consolidar pelo coordenador): Oráculo Origem=Sim; Lib=Parcial → A
validar com a execução do coordenador; tempo racional e fronteiras de junção agora comparados.
Relatório: [`docs/migration/foundation-k-report.md`](../migration/foundation-k-report.md).

Validação da integração K pelo coordenador: `node --test scripts/migration-oracle/oracle.spec.mjs scripts/migration-oracle/oracle-extended.spec.mjs` 45/45; ambos os verify passaram contra a origem. `directed_scene_oracle_extended` passou 15/15 via Cargo com `--locked`. Mantidas as divergências declaradas e a distinção Three isolado/runtime.
