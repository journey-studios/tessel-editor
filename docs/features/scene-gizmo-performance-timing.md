# F055 (tempo) — Pilha de medição de tempo da suíte de performance dos gizmos (BL015.21)

Complementa [F055](scene-gizmo.md) e o [contrato estrutural](scene-gizmo-performance.md) (contador de layouts, jornada
versionada, corpus, testes sem relógio). Esta página é **só a pilha de tempo**: os runners Python de `scripts/gizmo-perf/`, os
orçamentos, o teste de orçamento do core e a janela nativa. Backlog: [BL015.21](../backlog.md) e as lacunas filhas de BL015.23.

## Estado honesto (leia primeiro)

**Nada de desempenho está validado.** Não existe um `PASS` do `run.py`, um A/B medido, um mutante de tempo julgado limpo nem uma
janela nativa medida com baseline. Os quatro runners rodaram de verdade **uma vez**, no head `12bc8da`, numa máquina com carga de
base no limite da guarda (quatro processos de terceiros a ~100% de CPU) e com 11 CPUs (limite de carga 0,5 × 11 = 5,5):

| Runner | O que aconteceu em `12bc8da` | O que isso não mostra |
|---|---|---|
| `run.py` (core, release) | **`FAIL`**: os 2 controles negativos passaram e o teste de orçamento falhou com **6 p95 acima do orçamento, todos no `deep`** (`down.rotate`, `down.translate`, `move.rotate`, `read.rotate`, `state.rotate`, `up.rotate`); carga de 1 min depois da corrida 5,88 > 5,5. `small` e `dense` ficaram dentro dos orçamentos; 0 ticks longos | não separa ruído de máquina carregada de regressão. O `PASS` de um head anterior (`5cded7e`) é de outro head, de outra guarda e de orçamentos já ajustados (ver "Orçamentos") |
| `ab.py` (A/B) | build `BUILT` (executáveis distintos); measure **`INVALID_GUARD_AFTER_RUN`** (exit 3) **nas duas tentativas**: a própria corrida levou a carga de 1 min de ~5,1 a 6,0–6,5 e a guarda depois dela recusou. **O A/B nunca mediu**: o resultado retido de um head anterior era um A/A (as duas pontas eram o mesmo produto), e nada foi julgado | as linhas por tipo desse measure são observação, não veredito; não vê host, renderer nem WebView |
| `mutate.py` (fase `time`) | `prepare` `PREPARED` duas vezes; `time`: uma tentativa não invocada (nenhuma janela de guarda limpa), uma **`BLOCKED_OR_INFRA`** e um retry que terminou **`UNEXPECTED`** (o `memo_off` foi detectado, mas o runner de então não o classificava como morto; corrigido depois, sem reexecução). No retry os dois mutantes sintéticos de tempo foram mortos, sob a mesma carga | detecta mutantes **sintéticos** (plano extra, 1 ms), não é ausência de regressão |
| `native.py` (janela real) | `--phase run` completo nos três corpos: **`FAIL`** (639 verificações, 20 falharam, todas em `drag.*`); `same.*` e `switch.*` 15 de 15 por corpo. As expectativas de pixel e de soltura que causaram essas falhas foram corrigidas no runner depois, **sem reexecução** (nenhuma janela abriu desde então). As corridas antigas com `--functional-only` eram só do corpus `small` | **sem baseline** para a janela: os tempos do cliente (p.ex. `scene.pointer.move` p50 de 21,5 ms no `dense`, contra ~3,8 ms do core) são observação com carga ao lado, não afirmação de regressão |

Consequências: **sem `PASS`, sem A/B medido, sem número de desempenho citável**. Os arquivos brutos dessa corrida não ficam no
repositório (traziam caminhos e a lista de processos da máquina); o único que sobrevive é o relatório nativo, **varrido**, em
`scripts/gizmo-perf/fixtures/recorded-native-report.json.gz`, que a spec reproduz gesto a gesto (21 gestos gravados). Nenhuma parte
deste ramo foi medida depois de `12bc8da`: tudo que mudou desde então foi validado só com dublês (`gizmo-perf.spec.py`,
`harness.spec.py`, `ab.spec.py`) e compilação.

## O que existe

| Peça | Papel |
|---|---|
| `scene_gizmo_perf_budget.rs` (core, 3 testes `#[ignore]` e 4 de unidade de `within`, sem relógio) | mede a jornada em release; julga contra `budgets.json` (um orçamento que nenhuma amostra alcançou reprova); os dois outros testes são controles negativos (um plano a mais e 1 ms por chamada, injetados no próprio teste) |
| `run.py` | corrida controlada do teste de orçamento: fases `build` e `measure` |
| `ab.py` | baseline × candidato com o mesmo executável de medição, construído duas vezes |
| `mutate.py` | defeitos reais no produto × detectores estrutural/tempo/host |
| `native.py` | a mesma jornada numa janela real do editor (pixels, overlay, tempos do cliente) |
| `audit.py`, `budgets.py` | folga dos orçamentos em "planos"; derivação de `budgets.json` |
| `harness.py` | lock de GPU, processos limitados, guardas, identidade do git, posse de `--out`, gravação redigida |

O tempo é uma **razão** entre a requisição e o `ScenePlan::build_with` da própria cena, medido no mesmo processo em blocos de 5
antes de cada janela de amostras: a razão cancela a velocidade e a carga do momento. O teste repete 60 vezes, com editores novos, as
medidas frias (`cold.attach`, `cold.state`) e roda 8 ciclos de aquecimento, 40 quentes, 200 leituras e 6 movimentos por arrasto;
nenhuma amostra é descartada e frio e quente ficam separados. O core **não tem quadro**: nenhum tempo de primeiro quadro ou de
apresentação é atribuído a ele (só o host e a janela têm). `INCONCLUSIVE` quando mais de 25% dos blocos de referência discordam de
si mesmos em mais de 1,5×.

## Contrato de cada runner

Em todos: uma execução por `--out` e, nas fases que constroem, uma por checkout (ver "Posse e trava"); identidade do git que não
responde é `REFUSED`, `--allow-dirty` incluído; árvore suja é recusada (com `--allow-dirty` a corrida roda e nunca é um sucesso para o HEAD, exit 1: `DIRTY_RUN_NOT_A_PASS_FOR_HEAD`
no `run.py`, `DIRTY_RUN_NOT_FOR_HEAD` no `ab.py` e no `native.py`; o `mutate.py` não tem a flag); filhos com
prazo finito e grupo de processo próprio; nenhum `TESSEL_GIZMO_PERF_*` herdado chega a um filho (`harness.child_env`). Saídas:
`0` ok, `1` falha, regressão ou recusa, `2` argumento inválido, `3` `BLOCKED`/`INVALID_GUARD_AFTER_RUN`, `4` `INCONCLUSIVE`.

### `run.py`

- `--phase build`: recusa árvore suja, constrói `scene_gizmo_perf_budget` em release pelo wrapper de cargo (sem lock de
  performance: um build não é medição), copia o executável para `--out/exes` e grava HEAD, árvore, se estava suja e o sha256 em
  `build.json` (`exe` é relativo a `--out`).
- `--phase measure`: só roda o executável que o `build` registrou para este HEAD e esta árvore (ou `--exe`, a baseline de um A/B:
  sem HEAD, árvore nem hash a conferir, o caminho é resolvido uma vez e a corrida nunca é `PASS`, `EXE_GIVEN_NOT_A_PASS_FOR_HEAD`, exit 1).
  Apaga o relatório anterior, lê as guardas (carga de 1 **e** 5 min ≤ 0,5 por CPU, limite térmico, disco), toma o lock de GPU **uma
  vez, sem esperar**, roda com `--ignored --test-threads=1` e lê as guardas **de novo depois**. Lock ocupado ou guarda ruim é
  `BLOCKED` (exit 3); guarda ruim **depois** de uma corrida que passaria é `INVALID_GUARD_AFTER_RUN` (os números ficam, não é
  `PASS`). Uma falha real nunca é mascarada: orçamento estourado ou teste que falhou é `FAIL`; `INCONCLUSIVE` só quando **todo**
  teste que falhou foi pela nota de referência instável. `--calibrate` não julga (`CALIBRATION_ONLY`) e nunca é `PASS`.
- Em Apple silicon o `pmset` não informa `CPU_Speed_Limit`: o manifesto diz `thermal_state.verified = false` (estrangulamento não
  excluído), nunca "silencioso".

### `ab.py`

- A baseline é `BASELINE = cbe9e1b…` (o merge da PR #43), com o código de **produção intacto** mais uma lista de permissões: os
  testes do orçamento, o suporte, as fixtures, o arquivo `scene_plan/probe.rs` e a linha `pub mod probe;`. A baseline **não** recebe
  as chamadas do contador: o custo dele entra no que o A/B mede. Qualquer outra diferença fora da lista é `REFUSED_CONTAMINATION`.
- Se nenhum arquivo do que o executável medido enxerga difere, é A/A (`A_A_NO_PRODUCT_DIFFERENCE`, exit 4), nunca "sem regressão";
  se só diferem arquivos de outros crates (o desktop, o render) é `DIFFERENCE_OUTSIDE_THE_MEASUREMENT` (exit 4). **O A/B não vê o
  host, o renderer, a WebView nem `scene_monitor_host.rs`.**
- As fases: `build` (os dois builds em sequência, o executável do candidato copiado **antes** do build da baseline, recusa com
  `CARGO_TARGET_DIR`/`TESSEL_CARGO_LANE` definidos, recusa executáveis iguais) e `measure` (guarda, lock, uma corrida de
  aquecimento por lado, descartada e declarada, depois `--rounds` rodadas alternando a ordem; padrão 4). Uma tentativa anterior em `--out` é movida
  para `attempts/` antes de escrever.
- Regras declaradas antes de ler um número: um tipo **regride** quando a mediana agrupada do candidato passa de 1,10× a da baseline
  **e** o candidato é mais lento na maioria dos pares de rodadas **das medianas**; **ou** quando o p95 agrupado passa de 1,25× **e** o
  candidato tem p95 maior na maioria dos pares de rodadas **do p95 de cada rodada**; ou quando perde amostras ou tem tick longo
  (> 16,6 ms) a mais. p99 e máximo são registrados, não julgam. Baseline cuja mediana varia mais de 15% entre rodadas, ou corpus cujos
  blocos de referência instáveis passam de `reference.max_unstable_blocks`, é `INCONCLUSIVE`. Só vale para este cenário, esta
  configuração, esta máquina e este backend (não para a janela nativa).
- Veredito: `REGRESSED` (exit 1), `INCONCLUSIVE`/A/A/fora da medição (exit 4), `NO_REGRESSION_IN_SCENARIO` (exit 0), guarda ruim
  depois da corrida `INVALID_GUARD_AFTER_RUN` (exit 3; nada é julgado). Uma corrida de árvore suja (`--allow-dirty`) que seria
  `NO_REGRESSION_IN_SCENARIO` é `DIRTY_RUN_NOT_FOR_HEAD` (exit 1): as mudanças locais decidiram o que foi medido.
- **A/B contra a `main` de agora.** Se a `main` andou em `crates/core/src/scene_plan/mod.rs`, o A/B com a `BASELINE` fixa **recusa**:
  `make_baseline`, enquanto a baseline não tem `probe.rs`, só aceita que `scene_plan/mod.rs` do candidato difira do commit da
  baseline por exatamente a linha `pub mod probe;` (`git diff --numstat` `1 0`), e um head que contém essa `main` mostra mais que isso (num ensaio de merge local com a
  `main` de `ca06345` o numstat foi `4 3` e o resultado, `REFUSED_CONTAMINATION`). Comparar com a `main` atual exige **as duas
  flags**, na fase de build **e** na de measure, com o SHA literal (o mesmo nas duas): `--baseline <SHA da main> --allow-other-baseline`.
  Com elas o numstat volta a `1 0` e a checagem de overlay passa, mas **essa rota não tem teste**: a spec fixa `cbe9e1b` e exercita
  `make_baseline` só com repositórios git temporários, nunca com `--baseline` apontando para uma `main` real. É BL015.29: não foi
  corrigido nem coberto aqui. `cbe9e1b` é só a baseline **fixada** no runner (o merge da PR #43); não é a `main` de agora.

### `mutate.py`

- Quatro mutantes no **produto**: `extra_plan_per_handler` (todo plano construído duas vezes: estrutural **e** tempo devem falhar),
  `delay_1ms_per_handler` (`apply_gizmo` dorme 1 ms: o estrutural deve **passar**, o tempo deve falhar), `memo_off` e
  `replan_per_tick` (o teste de janela parada do host deve falhar). Só conta como "morto" quando o processo saiu com erro, o teste
  **nomeado** falhou **e** a mensagem esperada apareceu (e, no tempo, a guarda depois estava limpa e a corrida não foi
  `INCONCLUSIVE`; um `over budget:` de corpus julgável vale mesmo com a nota de outro corpus instável, como no `run.py`); compilação quebrada, prazo, executável ausente, disco cheio, lock ocupado e guarda ruim não são mortes. Pela
  mesma regra, um teste de tempo que **passa** com a guarda depois ruim é `INVALID_GUARD_AFTER_RUN`, não `SURVIVED`: não é resposta
  e os executáveis ficam para repetir.
- `--phase prepare` constrói, `--phase time` mede (um só lock, sem esperar). `time` recusa um `prepare` de outro HEAD, sujo ou
  inacabado, e só mede o executável cujo hash o `prepare` registrou. Uma execução em que algum mutante ficou sem resposta mantém todos
  os executáveis para repetir; `--only NOME...` restringe os mutantes.
- **Limite:** o `mutate.py` edita o **produto no checkout** e restaura os bytes (lock por checkout, recusa de árvore suja,
  restauração em `finally`). Um `kill -9` no meio deixaria o produto mutado em disco. Rodar os mutantes numa cópia (como o `ab.py` faz
  com `git archive`) é BL015.33.

### `native.py`

- Abre o editor release do HEAD num processo próprio (`HOME` e socket próprios), pelo socket semântico (`op`/`get`/`set`/`invoke`),
  nunca por coordenadas. Projeto salvo e aberto por `project.open`; cada gesto exige um quadro **novo** correlacionado (pedido mais
  novo que o do comando; uma soltura cujo plano é o do candidato já desenhado pode legitimamente manter o quadro do candidato, e a
  tabela `PIXEL_EXPECTATIONS` declara, por corpo e ferramenta, quando a escala pode deixar o quadro igual).
- Prova: transformada contra o golden, um só undo por gesto, undo/redo, o candidato fora do documento, o cancelar sem efeito,
  pixels, overlay desenhado e confirmado pela página, e a cena de mesmo bundle (`same.*`, BL015.22). A captura do SO é observacional.
- `--phase build` (binário com proveniência: HEAD, árvore, sha256 do binário e do `runtime/dist`; `binary` é relativo ao
  `build.json`) e `--phase run`. Precisa de `./scripts/runtime setup`. Mesmo uma corrida completa que passa termina em
  `PASS_FUNCTIONAL_OBSERVATION_ONLY`: é observação funcional, não `PASS` de desempenho. `--functional-only` roda a jornada sem lock
  nem guarda de carga e o estado diz `FUNCTIONAL_ONLY_NOT_A_MEASUREMENT`.
- **Limites declarados** (ficam em `LIMITS` no manifesto): o compositor/GPU não é observável (`presented` é `not_observable`; o
  hash do quadro é a leitura de volta do runtime); sem ponteiro, teclado nem foco físico (BL015.2, BL043, BL044); sem caso com splat;
  `client_ms` inclui o socket e o processo cliente, não é o custo do host; nada é orçado e não existe A/B contra um binário nativo.

### `audit.py` e `budgets.py`

- `audit.py` lê `budgets.json` e relatórios retidos e diz, por tipo, quantos "planos" o orçamento deixaria passar. Um plano extra soma 1
  à razão. 1 ms **soma-se a cada chamada** (`samples_ns`) **e divide-se pela referência da janela dela** (`reference_ns`), como o detector
  faz, e p50 e p95 (posto mais próximo) são recalculados; sem os pares (ou com listas que não casam) vale o tempo de plano do corpus
  (`reference.p50_ns`), só correto se toda janela tivesse essa referência, e `delay_1ms_basis` diz qual base cada relatório usou. Razão de
  um relatório nunca se soma ao plano de outro; diz em quantos relatórios a detecção vale e lista os tipos vistos só em alguns. É propriedade
  do orçamento, não defeito: onde a folga é larga o tempo é só um teto grosso, e é o detector estrutural que cobre o plano extra.
- `budgets.py` deriva `budgets.json`: `p50 = teto_0,05(máx. das calibrações × 1,15)` (piso 0,25), `p95 = teto_0,05(máx. × 1,25)`
  (piso 0,5), tipos `cold.*` com margens 1,25 e 2,0. **Recusa**, sem gravar nada: nenhum relatório, `--out` igual a um relatório, perfil
  que não é release e calibração cujos blocos de referência instáveis passam de 25% (ou que não diz) — o máximo afrouxaria o orçamento.

## Orçamentos (o que foi e o que não foi pré-registrado)

Os orçamentos vêm de calibrações **desta mesma árvore**, em três esquemas sucessivos. A primeira corrida julgada com o esquema final
ficou `INCONCLUSIVE` e a seguinte **falhou**; só **depois** disso as margens frias foram ampliadas (o p95 de `dense/cold.state` foi de
1,45 para 5,6). Logo: **as margens frias foram escolhidas depois de ver um `FAIL`**; o `PASS` de `5cded7e` é o de uma regra já
ajustada, e uma calibração da mesma árvore não prova ausência de regressão. O máximo de três calibrações não é estimativa de
variância. `budgets.json` aponta (`calibrations[].dir`: `gizmo-perf-calibC1` a `C3`) para calibrações que **não** estão no
repositório; o campo é rótulo e não muda (BL015.34). A recusa de calibração instável do `budgets.py` é posterior a C1–C3: não há como
saber se elas passariam. Em medição de tempo vale o A/B contra a baseline, não a calibração.

## Como rodar

Um `--out` **por runner**, o mesmo em todas as fases dele (o `prepare` do `mutate.py` esvazia `--out/exes`). Só em máquina quieta
(carga de 1 e de 5 min ≤ 0,5 por CPU, disco livre ≥ 12 GiB, e 16 GiB para abrir a janela) e **sem** envolver o runner em
`with-gpu-lock.py` (BL015.27).

```
OUT=/tmp/gizmo-perf-out
python3 scripts/gizmo-perf/run.py    --phase build   --out "$OUT/core"       # release; sem lock de performance
python3 scripts/gizmo-perf/run.py    --phase measure --out "$OUT/core"       # lock, guardas; [--calibrate] [--exe PATH --label baseline]
python3 scripts/gizmo-perf/ab.py     --phase build   --out "$OUT/ab"         # baseline fixada + candidato
python3 scripts/gizmo-perf/ab.py     --phase measure --out "$OUT/ab"         # [--rounds N]; contra outra main: ver "ab.py"
python3 scripts/gizmo-perf/mutate.py --phase prepare --out "$OUT/mutants"    # [--only NOME ...]
python3 scripts/gizmo-perf/mutate.py --phase time    --out "$OUT/mutants"
python3 scripts/gizmo-perf/native.py --phase build   --out "$OUT/native"     # precisa de ./scripts/runtime setup
python3 scripts/gizmo-perf/native.py --phase run     --out "$OUT/native"     # [--corpora small dense deep] [--functional-only]
python3 scripts/gizmo-perf/audit.py --budgets docs/migration/fixtures/gizmo-perf/budgets.json "$OUT/core/report.json"
python3 scripts/gizmo-perf/budgets.py "$OUT/cal1/report.json" "$OUT/cal2/report.json" "$OUT/cal3/report.json" --out budgets.json
```

O `budgets.py` só deriva (não mede): as calibrações vêm de `run.py --calibrate`. `--help` de cada runner lista as flags.

Sem janela, sem benchmark e sem o lock real (só dublês e compilação):

```
python3 scripts/gizmo-perf/gizmo-perf.spec.py     # os runners (288 testes)
python3 scripts/gizmo-perf/harness.spec.py        # o que os runners gravam: sem caminho, sem processos
python3 scripts/gizmo-perf/ab.spec.py             # o veredito do A/B por gate
python3 scripts/gizmo-perf/corpus.py --check      # o corpus é o que o gerador faz
./scripts/cargo test -p tessel-editor-core --locked --test scene_gizmo_perf_budget --no-run   # compila o teste de orçamento
```

## Posse e trava

O lock de performance é `/tmp/tessel-scene-gpu.lock`: um `flock` exclusivo **não bloqueante**, tomado **uma vez**; se estiver
ocupado o runner grava `BLOCKED` e para (sem laço, sem `sleep`, sem desvio). O runner **nunca apaga o arquivo do lock e nunca mata um
processo alheio**: só encerra o grupo de processos do próprio filho (SIGTERM e depois SIGKILL). Builds e testes funcionais não
tomam esse lock. Cada `--out` tem `.gizmo-perf.lock` (um nome para os quatro runners e todas as fases) e as fases que constroem ou
editam o checkout tomam antes o lock do checkout (um arquivo em `/tmp`, por hash do caminho). Quem não consegue sai com **uma** linha
`REFUSED` (exit 1), sem esperar e sem escrever. A trava exclui execuções **simultâneas**, não sequenciais: um runner que vem depois
ainda sobrescreve o `manifest.json` e o `exes/` do anterior.

## O que os runners gravam (e o que não gravam)

O repositório é público: **nenhum arquivo que um runner grava contém caminho absoluto da máquina, nome de usuário ou lista de
processos.** `harness.machine()` registra só o modelo e a contagem de CPU, memória, carga, disco livre, estado térmico, energia e versão do SO.
Os runners continuam gravando `cwd`, `argv`, o comando do build e o fim da saída do compilador, mas toda gravação passa por **um** ponto:
`harness.write` (JSON), `harness.write_text` (o stdout e o stderr de um filho e o `--out` do `audit.py`) e a linha que `harness.die` imprime chamam
`harness.redact`, que troca o checkout por `<checkout>`, o home por `~`, os diretórios temporários por `<tmp>` e qualquer outro
caminho absoluto por `<abs>/<nome-base>`. Um campo que uma fase **relê** como caminho não é deixado à redação: é gravado relativo a
`--out` (`exe` do `run.py`, `binary` do `native.py`). `harness.spec.py` testa o `machine()`, o `redact` e, com dublês, o que
`run.py`, `ab.py`, `mutate.py`, `native.py` e `audit.py` gravam; um teste estático reprova um `write_text(` de arquivo fora do `harness`.
**Limite:** só se tira o que tem forma de caminho absoluto; um nome de usuário solto, fora de um caminho, não é reconhecido, nem
`file:///caminho`. O que um runner **imprime** (`stdout`) e as capturas PNG da janela não passam por aqui: não cole a saída
num arquivo versionado e não versione captura (BL015.37).

## Limitações conhecidas

| ID | Limitação |
|---|---|
| BL015.23 | lacunas gerais: sem caso com splat; sem baseline × candidato do host, do renderer e da janela (o A/B não os vê); `PlanTimeline` sem detector estrutural; o contador mede chamadas de layout, não o custo de cada uma, e um planejador mais lento passa nos orçamentos permanentes (a razão cai) e pode passar no A/B; nenhum detector de trabalho dentro do laço de nós |
| BL015.24 | `gizmo-perf.spec.py` (cerca de 5 040 linhas, o nome com hífen obriga a carregar os runners por `importlib`) e `native.py` (1 362, que mistura proveniência do runtime, helpers puros de pixel e reconhecimento, o cliente da janela e a orquestração) passam de 1000 linhas; a divisão por módulo não foi feita (os novos `harness.spec.py` e `ab.spec.py` já seguem o padrão por módulo). O revisor automático também não lê a spec por inteiro (limite de tamanho por arquivo): o que muda nela só passa pelo OpenQodex e pelas portas locais |
| BL015.25 | esta página (menos de 250 linhas) e o README dos scripts (27) são a versão enxuta do contrato de tempo; falta o ponteiro para esta página na linha F055 do índice de features |
| BL015.26 | o percentil tem duas semânticas: `ab.py` usa o índice por piso (`ordered[int(q·n)]`), `native.py` e o teste Rust o posto mais próximo (para n = 100 e q = 0,95: índices 95 e 94), então o p95 do veredito do A/B não é o p95 do `report.json` das mesmas amostras; e a decisão "guarda depois da corrida ruim ⇒ `INVALID_GUARD_AFTER_RUN`" e o corpo de `run_phase` estão escritos quatro vezes (um por runner) |
| BL015.27 | o lock de `harness.Lock` é um `flock` não bloqueante e `with-gpu-lock.py` (a convenção do repositório, usada por ~12 scripts) segura o mesmo lock de forma bloqueante: um runner chamado dentro do wrapper recebe `BLOCKED` contra o próprio pai. **Não envolva os runners no wrapper**; um `--gpu-lock-held` não existe |
| BL015.28 | `long_ticks_max: 0` julga tempo **absoluto**: uma só amostra quente > 16,6 ms reprova o teste de orçamento (falso `FAIL` possível em máquina lenta ou ocupada; a única corrida real foi com carga 5,88), em tensão com "p99/máximo não julgam"; e a tolerância de 1,10× da mediana é menor que o espalhamento de 15% que ainda chama a baseline de estável (só a maioria de pares segura o falso positivo) |
| BL015.29 | a `BASELINE` fixa (`cbe9e1b…`) envelhece a cada merge na `main`: contra a `main` de agora o A/B exige `--baseline <SHA> --allow-other-baseline` nas duas fases e essa rota **não tem teste**; um head que contém a `main` atual é recusado com a baseline fixa (ver "`ab.py`"). O ensaio de merge local passou a spec (285) e `corpus.py --check`, mas não rodou nenhum A/B |
| BL015.33 | `mutate.py` edita o produto no checkout (restauração por `finally`; `kill -9` deixa o produto mutado) |
| BL015.34 | os orçamentos nascem do máximo de três calibrações com margens frias ampliadas depois de um `FAIL` e apontam rótulos que não estão no repositório; os testes `#[ignore]` podem ser chamados por `cargo test --release -- --ignored` fora do `run.py`, sem lock nem guardas |
| BL015.37 | resíduos da redação (R1) e do veredito (R2): o `stdout` dos runners (`print(json.dumps({status, reason}))` e `GIZMO_PERF_SUMMARY`) não passa pelo redator e um `reason` pode nomear um caminho; as capturas PNG da janela não são varridas nem redigíveis; o redator não reconhece `file:///caminho` nem um nome de usuário solto; em `ab.py`, uma rodada sem amostras de um tipo dá `pct` `None` e `judge` compara `None` com número |

Há ainda um mutante **equivalente** conhecido em `native.py` (`settle_commit`): zerar `acked_at` junto com `first_at` quando o
quadro deixa de convergir é defensivo e nenhuma sequência de leituras o distingue de `first_at = None` sozinho, então nenhum teste o
mata (documentado no comentário da spec).

## O que mudou neste ramo e por quê

1. **R1 na fonte.** `harness.machine()` deixou de gravar a lista de processos (`ps`); a gravação passou a ser redigida num ponto só
   (`redact`/`write`/`write_text`/`die`), cobrindo `cwd`, `argv`, comandos de build, o fim da saída do compilador e o stdout/stderr dos
   filhos; os campos relidos viraram relativos a `--out`. Antes, todo manifesto novo vazaria de novo caminhos e processos.
2. **Evidência removida.** A pasta de evidência bruta saiu do repositório; só o relatório nativo, varrido de caminhos, ficou como
   fixture de spec (`scripts/gizmo-perf/fixtures/recorded-native-report.json.gz`, gravado de forma determinística).
3. **R2 no `ab.py`.** O gate de p95 usava a maioria de pares das **medianas**, a mesma do gate de p50. Numa regressão só de cauda
   (p95 +50%, p50 igual) essa maioria é uma moeda: com 4 rodadas, ≥ 3 de 4 ao acaso é 5/16 ≈ 31%, logo o veredito seria
   `NO_REGRESSION_IN_SCENARIO` em ~69% das vezes (conta da revisão, **não medida**: o A/B nunca mediu). Agora o p95 de cada rodada
   é guardado (`p95s`) e cada gate julga com os seus próprios pares (`slower` das medianas, `slower95` dos p95 por rodada); a
   semântica do percentil não mudou (BL015.26).
4. **Testes novos em arquivos novos:** `harness.spec.py` (14) e `ab.spec.py` (6); contra os runners anteriores a este ramo
   falham os 14 e 4 dos 6 (os 2 restantes são controles que passam dos dois lados).
5. **Este documento** é o contrato da parte de tempo; os comentários que apontavam para o doc da feature por conta do histórico
   de orçamentos (`budgets.py`, `scene_gizmo_perf_budget.rs`, a spec) apontam para cá.
6. **Três regras que o código não cumpria** (achados da revisão completa deste ramo, todos com teste que falha no código
   anterior): o `ab.py` devolvia `NO_REGRESSION_IN_SCENARIO` com exit 0 numa corrida de árvore suja; o `run.py` já a chamava
   `DIRTY_RUN_NOT_A_PASS_FOR_HEAD`, mas saía com 0 e nenhum teste chegava a essa linha; e o `mutate.py` chamava `SURVIVED` um teste de
   tempo que passou com a guarda depois ruim (então `UNEXPECTED`, com os executáveis apagados), enquanto uma morte na mesma
   condição já era `INVALID_GUARD_AFTER_RUN`. Só dublês: nenhuma janela nem benchmark real rodou depois da mudança.
