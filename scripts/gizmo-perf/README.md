# Suíte de regressão de performance dos gizmos (F055 / BL015.21)

Contrato, matriz de superfícies, limites e números: [docs/features/scene-gizmo-performance.md](../../docs/features/scene-gizmo-performance.md).

| Arquivo | Para quê |
|---|---|
| `corpus.py` | gera os bundles `dense`/`deep` e `corpus.manifest.json` (`--check` falha se um arquivo não é o que o gerador faz) |
| `budgets.py` | deriva `budgets.json` dos relatórios de calibração (as margens frias foram ampliadas depois de um FAIL; não são pré-registradas) |
| `harness.py` | lock `/tmp/tessel-scene-gpu.lock` (uma tentativa, `LOCK_NB`), `run_bounded` (timeout finito, grupo de processo, SIGTERM e depois SIGKILL ao grupo sempre, leitura da saída com limite), guardas (carga 1/5 min, térmica explícita, disco) lidas de novo depois da corrida, manifesto |
| `run.py` | `--phase build` (sem lock) e `--phase measure` (lock, não constrói) do tempo do core; `PASS`/`FAIL`/`INCONCLUSIVE`/`BLOCKED`/`INVALID_GUARD_AFTER_RUN`; `--calibrate` nunca julga; proveniência: sem `--exe`, valida HEAD, árvore e sha256 contra o `build.json` (recusa outro HEAD/árvore e binário ausente ou alterado); **com `--exe` a proveniência NÃO é validada** (`build.json` não é lido; o manifesto registra só `exe_given` e o sha256 do arquivo, e o rótulo padrão é `candidate`, então um executável de outra revisão pode receber `PASS` com esse rótulo: passe `--label`); sem `--allow-dirty`, recusa árvore suja (também com `--exe`) e build registrado sujo (`build.json` guarda `dirty`; ausente conta como sujo); com `--allow-dirty` aceita os dois, e o que seria `PASS` vira `DIRTY_RUN_NOT_A_PASS_FOR_HEAD` (o exit code continua o do `PASS`, 0), nunca `PASS` para o HEAD; INCONCLUSIVE só se todo teste que falhou, falhou pela nota de referência instável |
| `ab.py` | baseline = `cbe9e1b` fixada (outra `main`: o mesmo `--baseline SHA --allow-other-baseline` nas duas fases, ver o doc da feature) com produção INTACTA + lista de permissões (testes, fixtures, `probe.rs`, `pub mod probe;`), manifestos de produção antes/depois, `REFUSED_CONTAMINATION`, A/A declarado, diferença só fora do executável medido = `DIFFERENCE_OUTSIDE_THE_MEASUREMENT`; × candidato; ordem alternada, aquecimento, regra declarada antes |
| `mutate.py` | mutantes no produto (plano extra, 1 ms, memo desligada, replanejar por tick) × detectores estrutural/tempo/host; só o teste nomeado + a mensagem esperada matam; infra é `BLOCKED_OR_INFRA`; sem nenhum detector avaliado é `NO_DETECTOR_EVALUATED` (exit 1); `time` recusa um `prepare` não terminado ou um mutante que ele não rodou, só mede o executável cujo sha256 o `prepare` registrou (sem registro, ou com hash diferente, é `MISSING_EXECUTABLE`; o `prepare` esvazia `--out/exes` ao começar) e escreve só `manifest.final.json`; SIGTERM restaura; hashes e logs |
| `native.py` | `--phase build` e `--phase run`: binário do HEAD com proveniência, projeto salvo aberto por `project.open`, quadro novo correlacionado, cancelar com candidato diferente que precisa ter sido desenhado; registra `dirty` do build e recusa `--corpora` vazio; `--functional-only` não toma o lock e não mede |
| `audit.py` | folga dos orçamentos (um plano extra / 1 ms passam em tempo?) sobre relatórios retidos |
| `gizmo-perf.spec.py` | spec dos runners (sem janela, sem benchmark, sem o lock real) |

Códigos de saída: `0` ok, `1` falha/regressão, `3` BLOCKED (lock ocupado ou guarda), `4` INCONCLUSIVE. Medições de tempo
**só** com o lock e sem guarda falha; builds e testes funcionais não tomam o lock. O A/B, os mutantes de tempo e a
janela têm uma fase de build (`--phase build|prepare`) separada da medição, porque um build deixa a máquina carregada.
Nada aqui usa `CARGO_TARGET_DIR`, `--target-dir` nem `TESSEL_CARGO_LANE`.
