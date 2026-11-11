# Suíte de regressão de performance dos gizmos (F055 / BL015.21)

Contrato, matriz de superfícies, limites e números: [docs/features/scene-gizmo-performance.md](../../docs/features/scene-gizmo-performance.md).

| Arquivo | Para quê |
|---|---|
| `corpus.py` | gera os bundles `dense`/`deep` e `corpus.manifest.json` (`--check` falha se um arquivo não é o que o gerador faz) |
| `budgets.py` | deriva `budgets.json` dos relatórios de calibração (as margens frias foram ampliadas depois de um FAIL; não são pré-registradas) |
| `harness.py` | lock `/tmp/tessel-scene-gpu.lock` (uma tentativa, `LOCK_NB`), `run_bounded` (timeout finito, grupo de processo, SIGTERM→SIGKILL), guardas (carga 1/5 min, térmica explícita, disco) lidas de novo depois da corrida, manifesto |
| `run.py` | `--phase build` (sem lock) e `--phase measure` (lock, não constrói) do tempo do core; `PASS`/`FAIL`/`INCONCLUSIVE`/`BLOCKED`/`INVALID_GUARD_AFTER_RUN`; `--calibrate` nunca julga; recusa árvore suja |
| `ab.py` | baseline = `cbe9e1b` com produção INTACTA + lista de permissões (testes, fixtures, `probe.rs`, `pub mod probe;`), manifestos de produção antes/depois, `REFUSED_CONTAMINATION`, A/A declarado; × candidato; ordem alternada, aquecimento, regra declarada antes |
| `mutate.py` | mutantes no produto (plano extra, 1 ms, memo desligada, replanejar por tick) × detectores estrutural/tempo/host; só o teste nomeado + a mensagem esperada matam; infra é `BLOCKED_OR_INFRA`; SIGTERM restaura; hashes e logs |
| `native.py` | `--phase build` e `--phase run`: binário do HEAD com proveniência, projeto salvo aberto por `project.open`, quadro novo correlacionado, cancelar com candidato diferente; `--functional-only` não toma o lock e não mede |
| `audit.py` | folga dos orçamentos (um plano extra / 1 ms passam em tempo?) sobre relatórios retidos |
| `gizmo-perf.spec.py` | spec dos runners (sem janela, sem benchmark, sem o lock real) |

Códigos de saída: `0` ok, `1` falha/regressão, `3` BLOCKED (lock ocupado ou guarda), `4` INCONCLUSIVE. Medições de tempo
**só** com o lock e sem guarda falha; builds e testes funcionais não tomam o lock. O A/B, os mutantes de tempo e a
janela têm uma fase de build (`--phase build|prepare`) separada da medição, porque um build deixa a máquina carregada.
Nada aqui usa `CARGO_TARGET_DIR`, `--target-dir` nem `TESSEL_CARGO_LANE`.
