# Suíte de regressão de performance dos gizmos (F055 / BL015.21)

Contratos, limites e o estado da medição (nada de tempo está validado):
[estrutural](../../docs/features/scene-gizmo-performance.md) e [tempo](../../docs/features/scene-gizmo-performance-timing.md).

| Arquivo | Para quê |
|---|---|
| `corpus.py` | gera os bundles `dense`/`deep` e `corpus.manifest.json`; `--check` falha se um arquivo não é o que o gerador faz |
| `hygiene.spec.py` | reprova dado de máquina de desenvolvimento (caminho absoluto, usuário, lista de processos) nos arquivos da feature |
| `harness.py` | o que os runners dividem: lock de GPU `/tmp/tessel-scene-gpu.lock` (uma tentativa, nunca apaga o arquivo), guardas de carga/térmica/disco, processos limitados, identidade do git, posse de `--out`, gravação **redigida** (`redact`) |
| `run.py` | tempo do core em release: `--phase build`, `--phase measure` (`--calibrate` nunca julga) |
| `ab.py` | baseline × candidato, o mesmo teste de orçamento construído duas vezes; cada gate (p50, p95) tem a sua maioria de rodadas |
| `mutate.py` | defeitos reais no produto × detectores estrutural/tempo/host (`--phase prepare`, `--phase time`); edita o checkout (BL015.33) |
| `native.py` | a jornada numa janela real, só por requisições semânticas (`--phase build`, `--phase run`, `--functional-only`) |
| `audit.py`, `budgets.py` | folga dos orçamentos em planos; derivação de `budgets.json` a partir de calibrações |
| `gizmo-perf.spec.py`, `harness.spec.py`, `ab.spec.py` | specs dos runners: dublês, sem janela, sem benchmark, sem o lock real; **não** provam tempo |
| `fixtures/recorded-native-report.json.gz` | o relatório da única corrida nativa real, varrido de caminhos, que a spec reproduz |

Sem medir nada: `for s in gizmo-perf harness ab hygiene; do python3 scripts/gizmo-perf/$s.spec.py; done` e `python3 scripts/gizmo-perf/corpus.py --check`.

Os runners medem **uma vez, em máquina quieta**, e só no checkout limpo. Um `--out` por runner, o mesmo em todas as fases dele; o
`measure`, o `time` e o `run` acham ali o que o `build`/`prepare` deixaram. O lock de performance nunca é esperado: ocupado ou guarda
ruim é `BLOCKED`. Não envolva um runner em `scripts/directed-scene-proof/with-gpu-lock.py` (BL015.27). Nenhum runner define
`CARGO_TARGET_DIR` nem `TESSEL_CARGO_LANE`.

Saídas: `0` ok, `1` falha, regressão ou recusa, `2` argumento inválido, `3` `BLOCKED` ou `INVALID_GUARD_AFTER_RUN`, `4` `INCONCLUSIVE`.
Nenhum arquivo gravado por um runner contém caminho absoluto da máquina, usuário ou lista de processos; os comandos exatos estão no doc de tempo.
