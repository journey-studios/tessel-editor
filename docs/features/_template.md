# FXXX — Nome da feature

## Objetivo e limites

Descreva o comportamento observável e se há apenas estado demonstrativo ou processamento real.

## Contrato do core / biblioteca

Tipos públicos, comando(s), consultas, IDs, pré-condições, unidades e resultado esperado.
Mostre um fluxo Rust sem UI, erros relevantes e o que permanece inalterado em caso de falha.

## CLI headless

Comando executável e JSONL de entrada/saída esperado, sem processo desktop. Declare gaps.

## UI e CLI da janela

Controle/gesto e equivalente semântico `./scripts/ui ...`. Registre apresentação N/A com motivo.

## Evidências e status

Teste/arquivo/comando e resultado observado por superfície. Diferencie implementado de
validado e aponte limitações. Atualize a linha correspondente em `docs/features/README.md`.
