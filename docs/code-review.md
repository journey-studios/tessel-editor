# Revisão de código

O CodeRabbit está instalado no repositório da organização. `.coderabbit.yaml` habilita
revisão automática, incremental em cada push e também para PRs em draft.

Antes de mergear, conferir os comentários/achados e o resultado efetivo da revisão.
Um check ignorado não significa revisão executada. Revisão do bot não substitui os
testes, as provas de runtime nem o contrato UI/CLI/lib da matriz de features.

Para solicitar revisão completa: comentar `@coderabbitai full review` no PR.
Referência oficial: https://docs.coderabbit.ai/reference/configuration

O plano atual limita a revisão a 100 arquivos. Os filtros excluem somente fixtures
JSON geradas, resultados de performance e textos de licença copiados. Testes, geradores,
implementação e documentação de contrato continuam incluídos. Os dados gerados possuem
provas de hash/regeneração próprias. Nenhum plano ou crédito pago foi alterado.
