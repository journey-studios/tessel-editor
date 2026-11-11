# Evidência da suíte de performance dos gizmos (BL015.21)

Arquivos pequenos (sem binários, sem imagens) com o que foi **executado**, o head e o estado de cada um. Nenhum número
aqui vale para o head final da PR: veja a coluna "vale para".

| Arquivo | O que é | Head / árvore | Estado | Vale para |
|---|---|---|---|---|
| `gizmo-perf-calib{1,2,3}.tgz` | calibração (sem julgar) do primeiro esquema (uma referência única) | árvore suja, antes do 1º commit | INCONCLUSIVE (a referência única derivava 2×) | histórico: motivou os blocos de referência |
| `gizmo-perf-calibB{1,2,3}.tgz` | calibração com blocos de referência, 25 execuções frias | árvore suja | só calibração | histórico |
| `gizmo-perf-calibC{1,2,3}.tgz` | calibração final (60 frias), **origem de `budgets.json`** | árvore suja, antes do 1º commit | só calibração | os orçamentos foram derivados desses três relatórios e **antes** de qualquer execução julgada aceita |
| `gizmo-perf-try1.tgz`, `try2.tgz` | execuções julgadas **que falharam/ficaram inconclusivas** (try2: `dense/cold.state` p95 2,61× > 1,45; `cold.attach` p95 7,21 > 7,1) | árvore suja | FAIL / INCONCLUSIVE | motivaram 60 execuções frias e as margens frias 1,25/2,0 (registradas em `budgets.py`); **não apagadas** |
| `gizmo-perf-final1-5cded7e.tgz` | execução julgada (3 testes, incluindo os 2 controles negativos), todas as amostras | `5cded7e`, árvore limpa (`dirty: ''`), carga 5,15–7,0 (limite 5,5 por CPU×0,5) | **PASS** | só o `5cded7e` (antes de a jornada trocar de bundle e de o suporte do core ganhar `bundle_b`); **não foi repetida no head final** (disco abaixo do piso) |
| `mutants-9186e2a-prepare-and-blocked-time.tgz` | mutantes de produto: detectores estrutural e do host | `9186e2a`, limpo | `extra_plan_per_handler` morto pelo estrutural; `delay_1ms_per_handler` **não** visto pelo estrutural (esperado); `memo_off` e `replan_per_tick` mortos pelo teste do host; **tempo: BLOCKED** (carga 5,68 > 5,5 e 8,95 > 5,5; nenhum mutante de tempo foi medido) | só os detectores estrutural/host; os de tempo seguem **não provados** |
| `ab-build-037eda5.manifest.json`, `ab-measure-037eda5-BLOCKED.manifest.json` | A/B `cbe9e1b` × candidato: build executado (sha256 dos dois executáveis), medição **BLOCKED** (carga 82) | `037eda5` | sem comparação | nada: **não há A/B medido** |
| `final2-9cc2335-BLOCKED.manifest.json`, `native-9cc2335-BLOCKED.manifest.json` | tentativa única de tempo do core e da janela no head final | `9cc2335` | BLOCKED (disco 8,39 GiB < piso 12) | registra que não houve medição |
| `native-functional-only-small.tgz` | jornada na janela real, só o corpus `small`, **sem lock e sem guarda (não é medição)**: 160 verificações funcionais, 0 falhas (transformada/undo = golden, pixels, overlay confirmado pela página) | build release de `9cc2335`-pré (jornada ainda com `scene_b` = mesmo bundle: o relatório mostra o efeito) | FUNCTIONAL_ONLY_NOT_A_MEASUREMENT | só funcional, só `small`, revisão anterior do `native.py` (a de `last_frame` foi só compilada) |
| `workspace-test-5cded7e-2294-1-16.log.gz` | `TESSEL_REQUIRE_RUNTIME=1 cargo test --workspace --no-fail-fast --locked` | `5cded7e` + `native.py` não rastreado | 2294 passaram, **1 falhou** (`the_journey_leaves_the_transform…`: ordem das chaves com `preserve_order` unificado no workspace; **corrigido** no `9cc2335`), 16 `ignored` | nunca foi repetido por inteiro depois da correção (disco) |

Gates repetidos no head final (`9cc2335`): `fmt --check`, `git diff --check`, `clippy --workspace --all-targets -D warnings`,
testes estruturais do core 7/7 e do host 3/3, spec dos runners 15/15. **Não** repetidos: workspace inteiro, builds dos
binários, `cargo tree`, provas de processo da escala `0ee2d14` (os gates funcionais pendentes de `0ee2d14` seguem pendentes).
