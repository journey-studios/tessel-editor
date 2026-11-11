# Suíte de regressão de performance dos gizmos (F055 / BL015.21)

Contrato, matriz de superfícies, limites e números: [docs/features/scene-gizmo-performance.md](../../docs/features/scene-gizmo-performance.md).

| Arquivo | Para quê |
|---|---|
| `corpus.py` | gera os bundles `dense`/`deep` e `corpus.manifest.json` (`--check` falha se um arquivo não é o que o gerador faz) |
| `budgets.py` | deriva `budgets.json` dos relatórios de calibração (regra fixada antes do candidato) |
| `harness.py` | lock `/tmp/tessel-scene-gpu.lock` (uma tentativa, `LOCK_NB`), guardas (carga, térmica, disco), manifesto |
| `run.py` | tempo do core em release sob o lock; `PASS`/`FAIL`/`INCONCLUSIVE`/`BLOCKED`; `--calibrate` nunca julga |
| `ab.py` | `main` (`cbe9e1b`, `git archive`, alvo MBX padrão) × candidato; ordem alternada, aquecimento, regra pré-declarada |
| `mutate.py` | mutantes no produto (plano extra, 1 ms, memo desligada, replanejar por tick) × detectores estrutural/tempo/host |
| `native.py` | a mesma jornada na janela real (processo próprio); `--functional-only` não toma o lock e não mede |
| `gizmo-perf.spec.py` | spec dos runners (sem janela, sem benchmark, sem o lock real) |

Códigos de saída: `0` ok, `1` falha/regressão, `3` BLOCKED (lock ocupado ou guarda), `4` INCONCLUSIVE. Medições de tempo
**só** com o lock e sem guarda falha; builds e testes funcionais não tomam o lock. O A/B, os mutantes de tempo e a
janela têm uma fase de build (`--phase build|prepare`) separada da medição, porque um build deixa a máquina carregada.
Nada aqui usa `CARGO_TARGET_DIR`, `--target-dir` nem `TESSEL_CARGO_LANE`.
