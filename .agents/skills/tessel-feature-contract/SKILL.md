---
name: tessel-feature-contract
description: Implementar ou alterar features do Tessel Editor com regras no core Rust, paridade entre biblioteca, CLI headless e UI, documentação por feature e evidências na matriz de capacidades.
---

# Contrato de features do Tessel Editor

Aplica-se ao projeto `tessel-editor`. Leia `docs/README.md`, `docs/features/README.md` e
`docs/core-architecture.md` a partir da raiz do repositório antes de alterar comportamento.

- Leia [docs/backlog.md](../../../docs/backlog.md) antes de começar. Toda pendência descoberta,
  inclusive prova ausente ou decisão, entra nesse registro comum com ID, status, responsável,
  dependências, aceite e referência. Não crie outra fila de execução; preserve relatórios como histórico.
- Antes de encerrar, atualize o item com evidência por superfície, commit/PR e limites restantes.
  Em paralelo, envie ao coordenador o patch das linhas sob seu ownership para integração segura.
- Toda feature implementada ou alterada exige um `.md` em `docs/features/` e uma linha na
  matriz. Preserve IDs estáveis. Mudanças de comportamento exigem atualizar ambos na mesma entrega.
- Comece pelo contrato público da biblioteca `crates/core`: comandos, consultas, resultados,
  erros, invariantes e sequência de uso sem janela. Regras e processamento não pertencem a
  GPUI, handlers de clique, IPC ou parsing de argumentos. A biblioteca não depende do desktop.
- UI e CLI traduzem entradas e apresentam resultados da mesma biblioteca. Uma feature de edição
  só está completa quando funciona pela lib, pela CLI sem UI e pela UI. Não use um socket de uma
  janela aberta como evidência de execução headless. Não duplique validações de domínio nos adapters.
- Gestos têm equivalentes semânticos: arrastar vira mover/reordenar/trim; scrub vira seek;
  um slider vira definir propriedade. Não exija ponteiro ou contexto gráfico no core.
- Separe estado do projeto de apresentação. Fechar janela, foco, tema e docking são capacidades
  do adapter desktop: marque N/A nas superfícies inaplicáveis com motivo; não invente operações
  de domínio para simular uma janela headless. Registre exceções e gaps reais na matriz.
- Para cada feature documente objetivo, limites, API lib, comando CLI headless, acesso pela UI,
  erros, efeitos no estado, pré-condições e evidências reproduzíveis. Use o template
  `docs/features/_template.md`. Não declare processamento real quando só existe estado demonstrativo.
- Valide um fluxo pela API pública sem GPUI e seu caminho CLI; cubra um erro relevante e a
  invariância de estado quando houver rejeição. Verifique o adapter UI conforme a mudança;
  teste de core não comprova drag nativo, layout ou menu aberto. Reaproveite evidências pertinentes.
- Na matriz, use implementado/validado/parcial/pendente/N/A conforme a legenda. Implementação
  sem evidência de execução não é validação. Não marque planos como entregues.
- Execute verificações proporcionais via `./scripts/cargo`. Para a biblioteca, rode
  `./scripts/cargo test -p tessel-editor-core --locked` e confira que sua árvore de dependências
  não traz GPUI. Quando afetar adapters, verifique também o workspace.

Esta skill não autoriza publicação, migração do motor de mídia nem implementação de funcionalidades
fora da tarefa. Registre trabalho futuro explicitamente, sem bloquear correções por gaps históricos.
