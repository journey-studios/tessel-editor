# Evidência da suíte de performance dos gizmos (BL015.21)

Arquivos pequenos (sem binários, sem imagens) com o que foi **executado**, o head e o estado de cada um. Nenhuma **medição** daqui vale
para o head final da PR (os gates funcionais do head e o que não pôde rodar estão listados depois da tabela): veja a coluna "vale para".

| Arquivo | O que é | Head / árvore | Estado | Vale para |
|---|---|---|---|---|
| `gizmo-perf-calib{1,2,3}.tgz` | calibração (sem julgar) do primeiro esquema (uma referência única) | árvore suja, antes do 1º commit | INCONCLUSIVE (a referência única derivava 2×) | histórico: motivou os blocos de referência |
| `gizmo-perf-calibB{1,2,3}.tgz` | calibração com blocos de referência, 25 execuções frias | árvore suja | só calibração | histórico |
| `gizmo-perf-calibC{1,2,3}.tgz` | calibração final (60 frias), **origem de `budgets.json`** | árvore suja, antes do 1º commit | só calibração | os orçamentos foram derivados desses três relatórios; as margens frias foram **ampliadas depois do `try2` falhar** (não são pré-registradas); uma calibração da mesma árvore não prova ausência de regressão |
| `gizmo-perf-try1.tgz`, `try2.tgz` | execuções julgadas **que falharam/ficaram inconclusivas** (try2: `dense/cold.state` p95 2,61× > 1,45; `cold.attach` p95 7,21 > 7,1) | árvore suja | FAIL / INCONCLUSIVE | motivaram 60 execuções frias e as margens frias 1,25/2,0 (registradas em `budgets.py`; limite p95 de `dense/cold.state` de 1,45 para 5,6); **não apagadas** |
| `gizmo-perf-final1-5cded7e.tgz` | execução julgada (3 testes, incluindo os 2 controles negativos), todas as amostras | `5cded7e`, árvore limpa (`dirty: ''`), carga 5,15–7,0 (limite 5,5 por CPU×0,5) | **PASS** | só o `5cded7e` (antes de a jornada trocar de bundle e de o suporte do core ganhar `bundle_b`); **não foi repetida no head final** (disco abaixo do piso) |
| `mutants-9186e2a-prepare-and-blocked-time.tgz` | mutantes de produto: detectores estrutural e do host (**mistura manifestos de `9186e2a` e `b898a6e`**) | `9186e2a`, limpo | `extra_plan_per_handler` morto pelo estrutural; `delay_1ms_per_handler` **não** visto pelo estrutural (esperado); `memo_off` e `replan_per_tick` mortos pelo teste do host; **tempo: BLOCKED** (carga 5,68 > 5,5 e 8,95 > 5,5; nenhum mutante de tempo foi medido) | só os detectores estrutural/host; os de tempo seguem **não provados** |
| `ab-build-037eda5.manifest.json`, `ab-measure-037eda5-BLOCKED.manifest.json` | A/B `cbe9e1b` × candidato: build executado (sha256 dos dois executáveis), medição **BLOCKED** (carga 82) | `037eda5` | sem comparação; **o `ab.py` daquela época copiava `scene_plan` do candidato para a baseline (era A/A)**: o build retido não vale nem como baseline | nada: **não há A/B medido** |
| `final2-9cc2335-BLOCKED.manifest.json`, `native-9cc2335-BLOCKED.manifest.json` | tentativa única de tempo do core e da janela no head final | `9cc2335` | BLOCKED (disco 8,39 GiB < piso 12) | registra que não houve medição |
| `native-functional-only-small.tgz` | jornada na janela real, só o corpus `small`, **sem lock e sem guarda (não é medição)**: 160 verificações funcionais, 0 falhas (transformada/undo = golden, pixels, overlay confirmado pela página) | build release de `9cc2335`-pré (jornada ainda com `scene_b` = mesmo bundle: o relatório mostra o efeito) | FUNCTIONAL_ONLY_NOT_A_MEASUREMENT | só funcional, só `small`, revisão anterior do `native.py` (a de `last_frame` foi só compilada) |
| `workspace-test-5cded7e-2294-1-16.log.gz` | `TESSEL_REQUIRE_RUNTIME=1 cargo test --workspace --no-fail-fast --locked` | `5cded7e` + `native.py` não rastreado | 2294 passaram, **1 falhou** (`the_journey_leaves_the_transform…`: ordem das chaves com `preserve_order` unificado no workspace; **corrigido** no `9cc2335`), 16 `ignored` | nunca foi repetido por inteiro depois da correção (disco) |
| `budget-audit-final1-5cded7e.json` | auditoria de folga dos orçamentos sobre o relatório retido do `5cded7e` (`scripts/gizmo-perf/audit.py`): 22/60 tipos em que um plano extra passa em tempo; 17/60 com folga p95 ≥ 2×; 6/60 ≥ 3×; 1 tipo (`dense/cold.attach`) em que 1 ms passa | `5cded7e` (análise offline, sem medir nada) | propriedade do orçamento, não prova de desempenho | só os orçamentos de `budgets.json` |

Gates do head `9cc2335` (um head ANTERIOR ao desta entrega): `fmt --check`, `git diff --check`, `clippy --workspace
--all-targets -D warnings`, testes estruturais do core 7/7 e do host 3/3. Para o head desta entrega só rodaram os
testes Python dos runners (70) e `rustfmt --check` + `git diff --check`. **Não** repetidos: `cargo tree -p
tessel-editor-core` (a alegação "sem gpui/wry" não tem log final), workspace inteiro (o do `5cded7e` teve 1 falha e não
é um PASS final), builds dos binários, provas de processo da escala `0ee2d14` (seguem pendentes) e tudo o que compila o
Rust novo desta entrega (`scene_gizmo_perf_threads.rs`, o teste do host de mesmo bundle, o `within()` que não mascara
FAIL e o contador por processo) — BLOCKED: disco 15,09 GiB < 16 GiB e carga 15,9 em 11 CPUs.
