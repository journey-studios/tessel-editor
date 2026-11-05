# Fixtures do oráculo de migração (BB10 / E0)

Corpus **sintético** e expectativas **geradas pela origem**. Não editar à mão: qualquer edição
é detectada (`node scripts/migration-oracle/oracle.mjs verify`).

| Caminho | Conteúdo |
|---|---|
| `cases.json` | índice dos 55 casos (4 válidos, 51 inválidos) com a intenção de cada um |
| `bundles/<id>.bundle.json` | entrada: um `render-bundle.json` (schema 1.0) sintético |
| `expected/<id>.expected.json` | saída das funções reais de `packages/scene-engine` para o bundle |
| `manifest.json` | proveniência: commit da origem, hashes das fontes executadas, versões, regeneração |

Sem mídia: os `assets[]` são referências sintéticas (nenhum PNG/áudio existe, hashes são
strings `synthetic-sha256-*`).

Como regenerar, formato dos arquivos, política numérica e consumo em testes Rust:
[`scripts/migration-oracle/README.md`](../../../scripts/migration-oracle/README.md).
Contrato e limites: [`docs/features/migration-oracle.md`](../../features/migration-oracle.md).
