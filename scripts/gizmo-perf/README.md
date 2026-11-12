# Suíte estrutural de performance dos gizmos (F055 / BL015.21)

Contrato, limites e lacunas: [docs/features/scene-gizmo-performance.md](../../docs/features/scene-gizmo-performance.md).
**Desempenho (tempo) NÃO validado**: nenhum PASS de tempo nem A/B medido.

| Arquivo | Para quê |
|---|---|
| `corpus.py` | gera os bundles `dense` e `deep` e `corpus.manifest.json`; `--check` falha se um arquivo versionado não é o que o gerador produz |
| `hygiene.spec.py` | falha se um arquivo versionado da suíte traz dado da máquina de desenvolvimento (caminho absoluto, nome de usuário, lista de processos) |
| `docs/migration/fixtures/gizmo-perf/` | corpus, jornada v1 e golden (`journey.expected.json`) que os testes do core e do host leem |

Como rodar, sem janela: `python3 scripts/gizmo-perf/corpus.py --check` e `python3 scripts/gizmo-perf/hygiene.spec.py`.
Os comandos dos testes Rust (estrutural, threads e host GPUI) estão em "CLI headless" no doc da feature.

A pilha de tempo (runners, orçamentos, A/B, mutantes, janela nativa) é a PR-B, em rascunho (BL015.23), com README próprio.
