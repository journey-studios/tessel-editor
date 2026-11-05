# F016 — Adicionar e processar efeitos

## Status

**Pendente** em UI, CLI e biblioteca. Nenhuma API ou comando desta feature está entregue.
Estado demonstrativo e controles desabilitados não comprovam implementação funcional.

## Contrato a implementar

Começar pelo core sem GPUI: entradas e IDs próprios, consultas, comandos, resultados e erros.
Documentar unidades, pré-condições e efeitos no estado. Expor a mesma operação pela CLI headless
antes de conectá-la à UI. Para batch, não depender de seleção da interface.

## Evidências necessárias

Fluxo programático pela lib, execução em processo sem UI, erro relevante sem alteração indevida
de estado e validação da integração UI. Substituir este planejamento pelo contrato executável
na entrega; atualizar [matriz](README.md) na mesma mudança.
