# Medição da suíte de performance dos gizmos em `12bc8da` (rodada 5)

Primeira execução **real** dos quatro runners (`native.py`, `run.py`, `ab.py`, `mutate.py`) no head
`12bc8da6fa564b3d740fa51da29dd203e63392ae` (árvore `85cfb1038626dd68c2ef2252e14e5d276f0596ac`; base da PR
`cbe9e1b393861febf073633fbbaa0da9857780b6`), com as guardas do `harness.py`, em 11/11/2026 (UTC), das 08:33Z às 10:21Z.
Nenhum runner, spec, fixture, `budgets.json`, margem, limiar, rodada ou regra foi alterado para obter estes números, e o
código não mudou: nada foi editado no worktree durante a medição (só `docs/**`, depois da última fase). O `git status --porcelain` está
**registrado vazio** no `record.txt` no preflight, depois de cada um dos dois `prepare` do `mutate.py` (que muta e restaura a árvore), ao decidir a
3ª tentativa do A/B e antes da coleta; nas outras fases a árvore limpa só é atestada pelos manifestos retidos, que trazem `head` e `tree` não
vazios e `dirty: ''` (conferido por script em todos eles; o `dirty` do runner só olha `crates`, `Cargo.*`, `scripts` e as fixtures, não o
`git status` inteiro). Os estados julgados (`FAIL` do `run.py` e do `native.py`, os resultados do `mutate.py` que o runner classificou) **não foram repetidos para trocar o
resultado** (o `prepare` rodou duas vezes só porque o `time` apagou `exes/`, e deu os mesmos resultados); só se repetiu o que o runner marcou como não julgado (`INVALID_GUARD_AFTER_RUN`, `BLOCKED_OR_INFRA`) ou que morreu por infraestrutura, como o protocolo permite.

**Como ler.** Os estados ficam com o nome que o runner escreveu (`FAIL`, `INVALID_GUARD_AFTER_RUN`, `BLOCKED`,
`UNEXPECTED_FAILURE`, `KILLED`, `SURVIVED`). **Implementado não é validado.** Este diretório não afirma "validado", "sem
regressão" nem "desempenho OK":

- `run.py` **PASS** significaria só "dentro dos orçamentos de `budgets.json` (calibrados na mesma árvore; margens frias ampliadas
  depois de um FAIL, BL015.7/BL015.12), **não** prova de ausência de regressão". Aqui o estado é **FAIL**.
- A/B **`NO_REGRESSION_IN_SCENARIO`** significaria só "nenhum p50 > 1,10× nem p95 > 1,25× com maioria estrita dos pares de
  rodadas contra a baseline fixada `cbe9e1b`, sem amostra perdida nem tick longo a mais, sobre o TOTAL da requisição do teste
  de orçamento do core, neste cenário, nesta configuração e nesta máquina, sob esta carga": o A/B **não vê host, renderer nem
  WebView** (BL015.23 (14)), um planejador mais lento que baixa a razão requisição/plano pode ficar invisível (M2), e nunca
  prova ausência de regressão abaixo dos limiares. A mudança de código desta PR no core é **uma linha** (o gancho do
  `probe`), então o A/B mostraria sobretudo que o runner funciona de ponta a ponta e que o contador não custa o bastante
  para aparecer. Aqui o A/B terminou **INVALID_GUARD_AFTER_RUN nas duas tentativas: nada foi julgado.**
- `mutate.py`: o que segue diz qual mutante foi morto (`KILLED`) por qual detector e com qual mensagem, qual sobreviveu
  (`SURVIVED`), qual ficou `BLOCKED`/`INVALID_GUARD_AFTER_RUN` e qual deu `UNEXPECTED_FAILURE`.
- `native.py`: **não existe baseline para a janela** (BL015.23 (14)). Valem as verificações funcionais e os tempos brutos com a
  carga ao lado, **sem comparação e sem afirmação de "sem regressão"**.

## Máquina e carga (contexto de todas as linhas)

- 11 CPUs (Apple silicon), macOS (Darwin 25.6.0). Térmico: `no_limit_reported` (o `pmset` não relata `CPU_Speed_Limit`:
  **não verificado**, estrangulamento não excluído). Disco livre durante a sessão: 17,08 a 19,70 GiB em 121 leituras (piso da
  suíte 12 GiB; a corrida nativa exige ≥ 16 GiB no início).
- Carga de base de **processos que não são meus**: quatro `Google Chrome Helper` do usuário (`Aperitif` e três `Aperitif Renderer`; pids 34613,
  34642, 34645 e 49969 pelas minhas anotações; o quarto ficou quente às ~08:41Z, antes só três) com 83 a 103% de CPU cada em todas as leituras
  de `ps`, mais `WindowServer` 10–35%, `OrbStack Helper` 6–44% e picos eventuais de `T3 Code (Beta)` (6–126%, 7 leituras) e de `esbuild`/`node`
  do usuário. Nenhum foi tocado, sinalizado ou repriorizado. O limite das guardas é 5,5 (0,5 × 11 CPUs) nas médias de 1 **e** de 5 min.
- Nas 93 sondagens somente-leitura da guarda feitas durante as esperas (`guard[wait-*]` do `record.txt`; algumas logo depois
  de uma corrida minha, que ainda pesa nas médias), a média de 1 min teve mediana 5,50 (mínimo 4,84, máximo 7,77) e a de 5 min
  mediana 5,67: **a carga de base fica no limite da guarda**, e só 18 das 93 sondagens passaram nas duas
  médias; **nenhuma** teve a média de 1 min ≤ 4,6. Esperas até a primeira janela limpa: N2 23 min; A2 14,6 min; R2 4 min; M2 (tentativa 1)
  **25 min e 23 sondagens sem nenhuma janela limpa (não invocado)**; A2 tentativa 2 5 min; M2 tentativa 2 0,8 min; M2 retry 9,3 min.
- Consequência medida: toda corrida de ≥ 1 CPU por dezenas de segundos levou a média de 1 min de ~5,1 para 5,6–6,5 (N2
  5,11 → 5,79; A2 5,07 → 6,49 e 5,08 → 5,99, que são as leituras do manifesto do próprio runner (o meu instantâneo antes da tentativa 2 deu 5,18);
  R2 5,42 → 5,88; `extra_plan` do M2 tentativa 2 5,04 → 5,59). A guarda **depois** da corrida
  (1 min ≤ 5,5) **recusou em 5 das 8 execuções guardadas** (N2 5,79; R2 5,88; A2 6,49 e 5,99; `extra_plan` do M2 tentativa 2 5,59) e
  **segurou nas 3 mais curtas**, os mutantes de tempo de ~19 s (`delay_1ms`, duas vezes) e ~10 s (`extra_plan` do retry, 5,39). É o item (11) de
  BL015.23 ("guarda pós-corrida que pode se autodisparar") agora com números. **Efeito no estado**: `native.py` (N2) e `run.py` (R2) ficaram `FAIL`
  **mesmo com `guard_after` recusando**; no A/B o `guard_after` recusando virou `INVALID_GUARD_AFTER_RUN` e **nada foi julgado**, mesmo com
  tipos `REGRESSED` na linha. Um PASS ou `NO_REGRESSION_IN_SCENARIO` com `guard_after` recusando **não teria saído** em nenhuma das duas.

## Passos (gerado de `record.txt`: nada retipado)

Legenda dos nomes: `S0` = `./scripts/runtime setup`; `N1` e `N2` = `native.py` build e run; `A1` e `A2` = `ab.py` build e measure; `R1` e
`R2` = `run.py` build e measure; `M1` e `M2` = `mutate.py` prepare e time. Sufixos: `A1-ab-build-retry1` é o retry do build do A/B, com `MBX_GC_MAX_TOTAL_SIZE=24GiB` no ambiente (o `step.py` não grava variáveis de ambiente:
veja a nota `A1-retry1` do `record.txt`); `A2b` é a
2ª tentativa do A/B; `M2b` é a 2ª tentativa do `time`; `M1b` é o `prepare` repetido porque o `time` apagou `exes/` (coordenador, item 4) e
`M2c` o `time` seguinte (o retry). A tentativa 1 do `time` **não foi invocada** (nenhuma janela limpa em 25 min), então não há bloco dela. Na tabela de sondagens, `A2c` são as sondagens oportunistas feitas depois do `M2c`, à procura de uma janela estrita para uma 3ª tentativa do A/B (que não veio).
`CPU alta` = os processos de maior CPU lidos por `ps -Ao pcpu,comm -r` antes e depois do passo (só o nome do executável e a %, nunca a
linha de comando). `$OUT` = `/tmp/pr44-measure-12bc8da` (fora do git). Os tempos são UTC de 11/11/2026.

| Passo | Comando | UTC (início–fim) | Exit | Carga 1/5/15 min antes → depois | Disco livre (GiB) | CPU alta antes → depois |
|---|---|---|---|---|---|---|
| S0-runtime-setup | `./scripts/runtime setup` | 08:33:57–08:33:59Z | 0 | 4,39/4,52/4,96 → 4,68/4,57/4,97 | 18,83 → 18,83 | 3x Chrome Helper 98-102% + WindowServer 22% + zsh 18% → 3x Chrome Helper 93-102% + OrbStack Helper 39% + fseventsd 24% |
| N1-native-build | `scripts/gizmo-perf/native.py --phase build --out $OUT/native` | 08:34:08–08:36:37Z | 0 | 4,63/4,56/4,97 → 20,31/10,92/7,46 | 18,84 → 17,13 | 3x Chrome Helper 99-101% + WindowServer 21% + zsh 18% → 3x Chrome Helper 96-101% + WindowServer 22% + esbuild 14% |
| N2-native-run-a1 | `scripts/gizmo-perf/native.py --phase run --out $OUT/native` | 08:59:41–09:07:29Z | 1 | 5,11/5,46/5,83 → 5,79/5,79/5,82 | 18,31 → 18,32 | 4x Chrome Helper 99-101% + WindowServer 20% + zsh 19% → 4x Chrome Helper 83-102% + loginwindow 36% + WindowServer 35% |
| A1-ab-build | `scripts/gizmo-perf/ab.py --phase build --out $OUT/ab` | 09:08:57–09:09:35Z | 1 | 5,75/5,81/5,82 → 10,91/6,98/6,24 | 18,31 → 19,7 | 4x Chrome Helper 97-102% + zsh 18% + esbuild 17% → 4x Chrome Helper 91-103% + WindowServer 22% + OrbStack Helper 9% |
| R1-run-build | `scripts/gizmo-perf/run.py --phase build --out $OUT/core` | 09:09:35–09:09:37Z | 0 | 10,91/6,98/6,24 → 10,44/6,95/6,23 | 19,7 → 19,68 | 4x Chrome Helper 93-101% + fseventsd 33% + WindowServer 22% → 4x Chrome Helper 99-101% + WindowServer 21% + OrbStack Helper 21% |
| A1-ab-build-retry1 | `scripts/gizmo-perf/ab.py --phase build --out $OUT/ab` | 09:11:18–09:11:20Z | 0 | 5,89/6,35/6,08 → 5,89/6,35/6,08 | 19,69 → 19,65 | 4x Chrome Helper 98-101% + WindowServer 23% + zsh 18% → 4x Chrome Helper 98-102% + WindowServer 22% + XprotectService 14% |
| A2-ab-measure | `scripts/gizmo-perf/ab.py --phase measure --out $OUT/ab` | 09:26:28–09:27:45Z | 3 | 5,07/5,45/5,72 → 6,49/5,86/5,85 | 19,63 → 19,62 | 4x Chrome Helper 98-101% + WindowServer 21% + zsh 13% → 4x Chrome Helper 96-101% + OrbStack Helper 23% + WindowServer 18% |
| R2-core-measure | `scripts/gizmo-perf/run.py --phase measure --out $OUT/core` | 09:32:47–09:33:15Z | 1 | 5,42/5,5/5,67 → 5,88/5,59/5,7 | 19,62 → 19,62 | 4x Chrome Helper 98-102% + zsh 18% + WindowServer 18% → 4x Chrome Helper 100-102% + WindowServer 20% + OrbStack Helper 19% |
| M1-mutate-prepare | `scripts/gizmo-perf/mutate.py --phase prepare --out $OUT/mutants` | 09:33:53–09:35:32Z | 0 | 5,45/5,52/5,67 → 9,27/7,01/6,24 | 19,61 → 18,75 | 4x Chrome Helper 99-101% + WindowServer 20% + T3 Code (Beta) 8% → 4x Chrome Helper 99-101% + WindowServer 23% + OrbStack Helper 12% |
| A2b-ab-measure-attempt2 | `scripts/gizmo-perf/ab.py --phase measure --out $OUT/ab` | 10:07:07–10:08:24Z | 3 | 5,18/5,48/5,63 → 5,99/5,65/5,68 | 18,82 → 18,82 | 4x Chrome Helper 99-102% + zsh 19% + WindowServer 19% → 4x Chrome Helper 99-102% + WindowServer 20% + opendirectoryd 4% |
| M2b-mutate-time-attempt2 | `scripts/gizmo-perf/mutate.py --phase time --out $OUT/mutants` | 10:09:35–10:10:05Z | 3 | 5,04/5,43/5,59 → 5,59/5,51/5,61 | 18,82 → 18,83 | 4x Chrome Helper 99-100% + WindowServer 20% + T3 Code (Beta) 6% → 4x Chrome Helper 99-101% + WindowServer 21% + loginwindow 13% |
| M1b-mutate-prepare-retry | `scripts/gizmo-perf/mutate.py --phase prepare --out $OUT/mutants` | 10:10:42–10:11:31Z | 0 | 5,27/5,44/5,58 → 8,75/6,34/5,9 | 18,83 → 19,53 | 4x Chrome Helper 98-101% + WindowServer 20% + zsh 18% → 4x Chrome Helper 99-102% + WindowServer 19% + OrbStack Helper 6% |
| M2c-mutate-time-retry | `scripts/gizmo-perf/mutate.py --phase time --out $OUT/mutants` | 10:21:13–10:21:42Z | 1 | 5,26/5,5/5,63 → 5,39/5,5/5,63 | 19,55 → 19,56 | 4x Chrome Helper 99-101% + WindowServer 18% + zsh 18% → 4x Chrome Helper 99-101% + WindowServer 21% + esbuild 4% |

Sondagens da guarda (somente leitura, sem lock) antes de cada invocação:

| Espera | Leituras | Primeira → última (UTC) | Carga 1 min (mín–máx) | Carga 5 min (mín–máx) | Última leitura limpa? |
|---|---|---|---|---|---|
| N2 | 12 | 08:39:05Z → 08:59:35Z | 5,03–6,12 | 5,46–8,22 | sim, às 08:59:35Z |
| A2 | 11 | 09:11:45Z → 09:26:22Z | 4,95–6,25 | 5,45–6,19 | sim, às 09:26:22Z |
| R2 | 4 | 09:28:43Z → 09:32:43Z | 5,19–5,49 | 5,45–5,67 | sim, às 09:32:43Z |
| M2 | 23 | 09:36:26Z → 10:01:36Z | 4,98–6,81 | 5,50–6,70 | não |
| A2b | 6 | 10:02:11Z → 10:07:02Z | 4,84–5,88 | 5,43–5,76 | sim, às 10:07:02Z |
| M2b | 2 | 10:08:44Z → 10:09:30Z | 5,05–5,42 | 5,43–5,54 | sim, às 10:09:30Z |
| M2c | 12 | 10:11:45Z → 10:21:01Z | 5,00–7,77 | 5,48–6,25 | sim, às 10:21:01Z |
| A2c | 23 | 10:22:28Z → 10:54:01Z | 4,94–7,09 | 5,23–5,76 | sim, às 10:41:35Z |

Todas as 93 sondagens: média de 1 min 4,84–7,77; média de 5 min 5,23–8,22.

## Verificações pedidas pelo coordenador

| # | Verificação | Resultado (arquivo) |
|---|---|---|
| 1 | A/B: executável do candidato × da baseline antes de gastar uma janela | **Diferentes** (`AB_EXECUTABLES_COLLIDE` **não** ocorreu): candidato `…/t3-4e3aaee2/target/release/deps/scene_gizmo_perf_budget-c6520d81ec54922a`, sha256 `9f81ee1f4571d8fd5008ccb67b8501c1e9fac3de916e99c3d2f9f0d08fd05f38`; baseline `…/ab/baseline-src/target/release/deps/scene_gizmo_perf_budget-c6520d81ec54922a`, sha256 `f8002313ed2601c77a125a7db79a05568359836ff78f390afcd1101cf3a0b3d6` (`ab/manifest.build.json`; o sha256 do candidato é o mesmo do `run.py --phase build`, `core/build.json`). `CARGO_TARGET_DIR` e `TESSEL_CARGO_LANE` estavam **não definidos** no ambiente que construiu (N1, A1, R1, M1 e todos os retries). |
| 2 | native: `overlay_drawn`, `cleanup_errors`, `session`, processo | `overlay_drawn`: 4 entradas por corpus (`state.translate`, `state.rotate.local`, `state.scale.local`, `switch.state`), **nenhuma com `drawn` nulo**; porém o runner **não checa** essas entradas (item (18) abaixo): a de `small/state.scale.local` é o overlay **anterior** (`tool: rotate`, `space: world`, `draws: 28`) enquanto `dense` e `deep` trazem `scale`/`local` com `draws: 29`. `cleanup_errors`: `[]` nos três corpos. `session`: o relatório guarda só `trace` (de `render.status`.`session.trace`: `enabled: true` nos três, logo não era um objeto de erro) e `session_error`, que é `null` nos três (a verificação "the session reported no error" passou); o objeto `session` inteiro não é guardado. `frames_total: 0` (por desenho: quadros de cena dirigida não são carimbados pela sessão). `ps` depois da corrida: nenhum editor meu restou (o único `tessel-editor*` listado, pid 58073 em `/private/tmp/pr41-preupload/bin/`, não é meu). |
| 3 | `run.py`: `run.env` sem `TESSEL_GIZMO_PERF_CALIBRATE` | Confirmado: `run.env` = `{TESSEL_GIZMO_PERF_OUT: …/core/report.json}`; `calibrate: false` (`core/manifest.json`). |
| 4 | `mutate.py --phase time` e `exes/` | Confirmado: depois da tentativa 2 do `time` (`BLOCKED_OR_INFRA`, com `INVALID_GUARD_AFTER_RUN` no tempo de `extra_plan_per_handler`) o runner **apagou `mutants/exes/`**; foi preciso `--phase prepare` de novo (`M1b`) e um `time` de novo (`M2c`): **um só retry**, sem `--only`. O runner também apaga `exes/` depois de um `time` completo. |
| 5 | Todo manifesto retido mostra `head`/`tree` e `dirty` vazio | Confirmado em todos os manifestos deste diretório (`head` `12bc8da6…`, `tree` `85cfb103…`, `dirty: ''`; no A/B e nos mutantes também `candidate`/`prepare_identity`). |

## O que cada runner deu

### `native.py` (janela real, corpus `small`, `dense` e `deep`)

- `N1` build: `BUILT`; binário `tessel-editor` sha256 `fbc7a9c97ac892f105863f55f6c8c7f517a2c32a3f91015bcdeddb26910b5dbd`,
  `runtime/dist` `636c0d9fc3af852f299af3419f5a1f4d1f7f1b1c7d70c9b2afeb2cdc6d1c5d9e`, jornada `7f69fa96…`, manifesto do corpus
  `1e7ca4b9…` (`native/build.json`).
- `N2` run (tentativa **1 de 3**, a única): `FAIL`, exit 1, 7 min 48 s, 639 verificações, **20 falharam** (`small` 5, `dense` 10,
  `deep` 5), todas em passos `drag.*`; `guard_after`: "load average (1 min) 5.79 over 5.5" (reportado junto com o `FAIL`).
  - **15 falhas "the committed picture is a NEW picture"** (5 arrastos que confirmam por corpus: `drag.translate.x`,
    `drag.rotate.local.y`, `drag.rotate.world.z`, `drag.scale.local.uniform`, `drag.scale.local.x`). O runner tira a marca
    `gesture_mark` (o id do quadro mais novo **antes** da soltura, que é o quadro do próprio candidato quando ele foi desenhado) e
    exige um quadro com `request > gesture_mark` depois de `scene.pointer.up` (`native.py`, linhas 674 a 681). Nos 15 casos o
    monitor continuou com **o quadro do candidato** (mesmo `request` e mesmo hash) depois de um `up` que respondeu
    `document_changed: true`; os caminhos de cancelar desenharam quadros novos (as verificações "the cancel drew a NEW
    picture" passaram). **Não está decidido** se é o produto (o host não redesenha um commit idêntico ao candidato; **hipótese
    não testada**, coerente com os `sync_unchanged` e `plan_key` do trace do `dense`) ou a expectativa do runner (item (16)).
  - **5 falhas só no `dense`, passos `scale.*`**: a imagem do candidato, a do commit e a anterior têm o **mesmo hash**
    (`a5cee15db5c0b400`): `drag.scale.local.uniform` e `drag.scale.local.x` (candidato igual à base; commit igual ao anterior; mais a falha
    do quadro novo contada acima) e `drag.scale.cancel` (candidato igual à base). Escala não muda um pixel do corpus `dense`
    nesta câmera/nó: **não decidido** se é o corpus, a verificação por pixel ou o produto (item (17)).
  - As outras 619 verificações passaram: transformada/undo/redo iguais ao golden, cancelar restaura a imagem-base, `project.open`
    real, `same.*` e `switch.*` (**15 de 15** por corpus nos três).
- **BL015.22 na janela real**: `same.show: proof-c drew a NEW picture (newer than request …)` passou em `small`, `dense` e `deep`,
  e `switch.show: proof-b drew a NEW picture`, `switch.state` e `same.attach`/`same.resources` também. Isso diz o que o
  **head corrigido** faz na janela; **não há controle negativo na janela** (nenhuma corrida com a chave antiga), a corrida inteira é
  `FAIL` por outros motivos e o `PASS_FUNCTIONAL_OBSERVATION_ONLY` não foi alcançado.
- **Tempos brutos do cliente (ms, com a carga acima, sem baseline, nada comparado)**: `launch_to_socket_ms` 309,7 / 538,2 / 288,8
  (`small` / `dense` / `deep`); `project.open` 12,6 / 9,3 / 47,0; `scene.pointer.move` (n = 26) p50 1,5 / **21,5** / 4,6 e p95 5,7 / **35,4** / 6,0;
  `scene.pointer.down` p50 0,8 / 16,7 / 1,5; `scene.pointer.up` p50 3,0 / 18,3 / 1,9; `runtime_total_ms` das imagens p50 14 / 25 / 13, p95 20 / 30 / 19. Em `dense` o
  `pointer.move` passa do quadro de 60 Hz (16,6 ms) no p50 na janela, enquanto o core sozinho dá ~3,8 ms (`move.translate` p50 5,95× a referência de 0,635 ms; `move.rotate` 6,36×, ~4,0 ms);
  isso é uma **observação** (a diferença é do caminho do host/IPC/WebView, que o A/B não vê, BL015.23 (14)), não uma regressão.
- Spans do trace: `small` e `deep` só têm `evaluate`/`ipc`/`serve`, o `dense` tem 11 tipos (e `dropped: 4`): **diferença não
  explicada**, só registrada. Três capturas PNG (`*.open.png`, 0,45–0,65 MB cada, observacionais) ficam **fora do git**; sha256 abaixo.

### `run.py` (core, release)

- `R1` build: `BUILT` em 1,63 s (restauração do cache do mbx: 25 hits, 0 misses), executável sha256
  `9f81ee1f4571d8fd5008ccb67b8501c1e9fac3de916e99c3d2f9f0d08fd05f38`. O `stderr` do build traz
  `mbx[savings]: 1.4 TiB logical of abandoned target directories removed`: o mbx **podou** diretórios de target abandonados
  durante a sessão (relevante para o achado (15)).
- `R2` measure (tentativa **1 de 3**, a única; `FAIL` é final): **`FAIL`**, exit 1 (cargo exit 101), 27,31 s, 09:32:47Z a 09:33:15Z.
  - Os **2 controles negativos passaram** (`the_budget_detector_rejects_one_extra_plan_of_work_per_handler_call`,
    `the_budget_detector_rejects_a_millisecond_of_delay_per_handler_call`); o teste de orçamento
    (`the_gizmo_handlers_stay_within_their_budget_cold_and_hot_on_every_corpus`) **falhou**: "over budget" em **6 tipos, todos no `deep`, todos p95**:

    | Tipo (`deep`) | p95 / plano | orçamento | p50 / plano | n |
    |---|---|---|---|---|
    | `down.rotate` | 6,10× | 5,4 | 4,04× | 40 |
    | `down.translate` | 3,26× | 3,15 | 2,41× | 40 |
    | `move.rotate` | 10,96× | 9,05 | 6,64× | 240 |
    | `read.rotate` | 5,98× | 4,15 | 2,89× | 200 |
    | `state.rotate` | 5,30× | 3,95 | 2,96× | 40 |
    | `up.rotate` | 17,41× | 10,05 | 7,28× | 40 |

  - `small` e `dense`: nada acima do orçamento; **0 ticks longos** nos três corpos; blocos de referência instáveis 11/259 (4,25%) em
    `small`, 18/259 (6,95%) em `dense`, 28/259 (10,81%) em `deep` (limite 25%: o `deep` foi **julgado**, não `INCONCLUSIVE`).
    Referência (`p50` do plano) 9,375 µs / 635,084 µs / 47,209 µs.
  - `run.env` sem `TESSEL_GIZMO_PERF_CALIBRATE` (item 3). Carga antes 5,42/5,50/5,67, depois 5,88/5,59/5,70 (`guard_after` recusando; o `FAIL` vale mesmo assim).
  - **Campo enganoso do manifesto**: `core/manifest.json` traz `tests_passed` com os **três** nomes, inclusive o teste de orçamento que **falhou**
    (`run.py` linha 219 só procura `test NAME ...` no `stdout`, não `ok`). Quem passou ou falhou está em `core/stdout.txt.gz` (`test result: FAILED. 2 passed; 1 failed`);
    o estado `FAIL` e o exit 1 não dependem desse campo (item (22)).
  - **O que o FAIL não mostra**: não separa ruído de máquina carregada (quatro helpers a ~100%, carga de base 5,4+) de regressão. O PASS do
    `5cded7e` (`gizmo-perf-final1-5cded7e.tgz`) é de outro head e de outra guarda. A comparação independente na mesma máquina seria o A/B, mas ele
    ficou INVALID (observação sem veredito: razão mediana candidato/baseline 0,99–1,03 nas duas tentativas). Item (20).

### `ab.py` (A/B `cbe9e1b` × candidato)

- `A1` build, tentativa 1: **crash de infraestrutura**, exit 1, `FileNotFoundError` em `ab.py` linha 434
  (`shutil.copy(c_exe, exes['candidate'])`): o executável do candidato que o cargo imprimiu
  (`…/target/release/deps/scene_gizmo_perf_budget-c6520d81ec54922a`) **não existia mais** depois do build da baseline; nenhum
  manifesto foi escrito (item (15)). Um retry (permitido: morte de infraestrutura, não veredito) com **só o ambiente** mudado,
  `MBX_GC_MAX_TOTAL_SIZE=24GiB` (a variável documentada em `docs/cargo-worktrees.md`; nenhum código, nenhum `mbx gc`/`clean`, alvo
  padrão): `BUILT` em 2 s (os dois builds vieram do cache). **Não se sabe** se foi a variável ou o cache quente que fez a
  diferença (um só retry).
- `ab/manifest.build.json`: `candidate` = head `12bc8da6…`/tree `85cfb103…`/`dirty: ''`; baseline `cbe9e1b393861febf073633fbbaa0da9857780b6`
  (fixada); árvores de produção: antes do overlay `54283977…` (= árvore de `cbe9e1b`), depois do overlay `0578616b…`, candidato
  `6b4ee280…`; `overlay_changed` = `crates/core/src/scene_plan/mod.rs` e `crates/core/src/scene_plan/probe.rs`; `contaminated: []`;
  **`product_files_that_differ`** = `crates/core/src/scene_plan/build.rs` (só ele); **`product_files_outside_the_measurement`** =
  `crates/desktop/src/workstation/mod.rs`, `scene_gizmo_perf_tests.rs`, `scene_lifecycle_tests.rs` e `scene_monitor_host.rs`
  (**quatro** arquivos, não dois: o `is_prod` conta tudo sob `crates/*/src`, inclusive os módulos de teste do desktop).
- `A2` measure, tentativas **1 e 2 de 3**: `INVALID_GUARD_AFTER_RUN`, exit 3, **nada julgado**.

  | | início–fim (UTC) | 1 min antes → depois (manifesto do runner) | `verdict_counts` (observação) | tipos `REGRESSED` na linha (observação) | mediana cand/base | p95 cand/base | spread da baseline |
  |---|---|---|---|---|---|---|---|
  | 1 | 09:26:28–09:27:45 | 5,07 → 6,49 | 56 `NO_REGRESSION_IN_SCENARIO` + 4 `REGRESSED` | `deep/cold.attach` (p95 389667 > 291875 × 1,25, 4/4 pares), `deep/down.translate` (3/4), `deep/move.translate` (3/4), `small/state.scale` (3/4) | 0,9913–1,0305 | 0,4830–1,5000 | máx 0,0931 (`dense/undo`) |
  | 2 | 10:07:07–10:08:24 | 5,08 → 5,99 | 59 + 1 | `small/read.scale` (p95 47833 > 31500 × 1,25, 4/4) | 0,9895–1,0298 | 0,5503–1,5185 | máx 0,0756 (`dense/undo`) |

  Em cada tentativa: 10 execuções (2 de aquecimento + 8 mantidas, ordem B C C B B C C B), ~7,6 s cada, nenhum tick longo, nenhuma
  amostra perdida. **Nenhum tipo `REGRESSED` se repete entre as duas tentativas** (e nenhum dos da 1 reaparece na 2): é o padrão de
  ruído, mas isso é uma leitura, não um veredito. As linhas por tipo **não** são um veredito do A/B (o estado é INVALID).
  O manifesto da tentativa 1 está retido (`ab/attempt1.manifest.json`); os `verdicts.json` e os relatórios por rodada dela
  foram **apagados pelo próprio runner** no início da tentativa 2 e eu não os arquivei antes (erro de ordem meu; item (21)); os números dela
  são os do manifesto e os transcritos em `record.txt`. A tentativa 2 está completa (`ab/attempt2.*`).
- **A2 tentativa 3: não invocada.** Cada corrida de 77 s somou +0,91 e +1,42 à média de 1 min (5,08 → 5,99 e 5,07 → 6,49), então a guarda depois (≤ 5,5) exigiria partir de ≲ 4,6 (e a de 5 min ≤ 5,45 para o início não recusar). Nenhuma das 93 sondagens teve a média de 1 min ≤ 4,6 (mínimo 4,84, às 10:07:02Z), então uma terceira corrida só repetiria `INVALID_GUARD_AFTER_RUN` sem julgar nada. O teto de 3 tentativas é um máximo, não uma meta. **Sem veredito do A/B nesta rodada.**

### `mutate.py`

| Mutante | Detector | Estado | Detalhe (do manifesto) |
|---|---|---|---|
| `extra_plan_per_handler` | estrutural | **KILLED** (nas duas `prepare`) | `small / open.first_overlay: scene.gizmo.state laid the scene out 2 times (budget 1)`; falharam os dois testes do detector: `every_request_plans_the_scene_at_most_as_often_as_its_contract_says_on_every_corpus` (o nomeado, com a mensagem esperada) e `a_long_drag_costs_one_layout_per_move_and_nothing_else_grows` |
| `extra_plan_per_handler` | tempo | tentativa 2: **INVALID_GUARD_AFTER_RUN** (guarda depois 5,59; **não é resposta**, embora tenha visto 64 linhas); **retry: KILLED** | retry: 68 linhas fora do orçamento (o manifesto guarda as 24 primeiras: de 1,01× a 2,13× o orçamento, mediana 1,35×; a menor folga é `small/down.scale: p95 is 7.33x the plan (budget 7.25)`); as três primeiras: `small/cold.state: p50 is 3.52x the plan (budget 3.15)`, `small/down.rotate: p50 is 11.25x the plan (budget 10.45)`, `small/down.rotate: p95 is 26.01x the plan (budget 12.2)`. O kill vem de **muitos tipos acima do orçamento ao mesmo tempo**, não de folga grande em cada um; guarda depois limpa (5,39/5,50/5,63) |
| `delay_1ms_per_handler` | estrutural | **SURVIVED** (esperado: o estrutural não deve ver atraso puro; os 2 testes nomeados rodaram e passaram) | nas duas `prepare` |
| `delay_1ms_per_handler` | tempo | **KILLED** (tentativa 2 **e** retry; guarda depois limpa nas duas) | tentativa 2: 107 linhas (as 24 guardadas: de 11,05× a 55,84× o orçamento), `small/cold.state: p50 is 77.30x the plan (budget 3.15)`, `small/cold.state: p95 is 170.85x the plan (budget 6.05)`, `small/down.rotate: p50 is 166.82x the plan (budget 10.45)`; retry: 106 linhas (as 24 guardadas: de 11,02× a 94,61×), `small/cold.state: p50 is 111.61x the plan (budget 3.15)`, `small/cold.state: p95 is 175.06x the plan (budget 6.05)`, `small/down.rotate: p50 is 167.80x the plan (budget 10.45)` |
| `replan_per_tick` | host | **KILLED** (nas duas `prepare`) | `assertion left == right failed: small: 100 idle ticks laid the scene out` |
| `memo_off` | host | **UNEXPECTED_FAILURE** (nas duas `prepare`; **reproduzível**) | "the named test failed without its own message (0 of 1 expected lines)": o teste nomeado (`an_idle_window_plans_nothing_per_tick_and_the_overlay_stays_memoised`) **falhou** (exit 101) em `scene_gizmo_perf_tests.rs:193:9`, que é a verificação de **contagem de layouts** ("a sync of an unchanged state laid the scene out more than once"), uma verificação **anterior** às duas mensagens que o `EXPECT` do runner aceita (linhas 200 e 204: "the overlay was asked for again", "every sync of an unchanged state is a memo hit"). Pelas regras do próprio runner o `memo_off` **não conta como morto**. O log bruto não é guardado pela fase `prepare` (só o sha256: `cf4264aa…` e `d0488544…`): a mensagem exata é **inferida** do local do panic + o fonte, não lida (item (19)). |

- `M1` (`prepare`, duas vezes, 99 s e ~50 s): `PREPARED`, resultados idênticos; árvore **restaurada** depois de cada uma (`restore.restored: true` por mutante;
  `git status --porcelain` vazio; HEAD e árvore iguais). `M2` (`time`): **tentativa 1 não invocada** (25 min de espera, 23 sondagens, nenhuma
  limpa: pelas regras seria BLOCKED, mas o runner nem rodou, então não há manifesto); tentativa 2: exit 3, status do runner `BLOCKED_OR_INFRA` (o `extra_plan` ficou
  `INVALID_GUARD_AFTER_RUN`; o `delay_1ms` foi `KILLED`); **retry (o único)**: exit 1, status final **`UNEXPECTED`** (só por causa do `memo_off`, que o `prepare` já tinha dado).
- **Resumo do que o runner deixa dizer**: 3 dos 4 mutantes de produto foram mortos pelo detector esperado (`extra_plan` pelo estrutural **e** pelo de tempo,
  `delay_1ms` pelo de tempo, e o estrutural corretamente **não** o vê, `replan_per_tick` pelo host); o `memo_off` foi detectado pelo host mas **não** classificado como morto (item (19)).
  Isso é detecção de **mutantes sintéticos**; não é prova de ausência de regressão.

## Achados e onde estão registrados (todos em `docs/backlog.md`)

| ID | O que | Onde está a prova |
|---|---|---|
| BL015.23 (15) | `ab.py --phase build` copia o executável do candidato só **depois** do build da baseline; se ele sumir do `target/` entre os dois (causa não isolada; o `mbx` podou targets abandonados na mesma sessão), `FileNotFoundError` sem manifesto; o manifesto de build não tem `exit` nem `utc_end` | `record.txt` (`A1-ab-build`, `A1-result`), `ab/manifest.build.json` (retry), `core/manifest.build.json` (linha `mbx[savings]`) |
| BL015.23 (16) | `native.py`: 15 de 15 arrastos que confirmam falham em "a committed picture is a NEW picture": expectativa do runner × produto, **não decidido** | `native/report.json.gz`, `native/manifest.json` |
| BL015.23 (17) | `native.py` `dense`: arrastos de `scale.*` não mudam um pixel (mesmo hash `a5cee15db5c0b400`): 5 falhas por pixel | idem |
| BL015.23 (18) | `native.py`: `overlay_drawn` só é gravado, nunca checado, e o laço aceita o **primeiro `drawn` não vazio** (que pode ser o overlay do passo anterior; sem `drawn` em 5 s grava `null` sem falhar): uma entrada velha (`small/state.scale.local`, rotate/world) passou em silêncio | idem (`native.py` linhas 553 a 560) |
| BL015.23 (19) | `mutate.py`: `EXPECT[(memo_off, host)]` não inclui a primeira verificação que falha (linha 193): `UNEXPECTED_FAILURE` com o mutante detectado | `mutants/prepare1.manifest.json`, `mutants/prepare2.manifest.json` |
| BL015.23 (20) | `run.py` `FAIL`: 6 p95 do `deep` acima do orçamento com carga de base ~5,5: decisão (ruído × regressão × recalibrar sob a guarda nova) aberta | `core/stderr.txt`, `core/report.json.gz` |
| BL015.23 (21) | `ab.py --phase measure` apaga o `verdicts.json` e os relatórios da tentativa anterior no início da seguinte (evidência perdida se não for arquivada antes) | `record.txt` (`A2b-result`) |
| BL015.23 (22) | `run.py` grava `tests_passed` com o que **apareceu** em `stdout` (`test NAME ...`), não com o que passou: um `FAIL` lista os 3 testes como "passed" (`run.py` linha 219) | `core/manifest.json` (`tests_passed` com os 3 nomes) × `core/stdout.txt.gz` ("2 passed; 1 failed") |
| (11), (13)(b), (14) e BL015.22 | evidência nova: guarda pós-corrida com números; `exes/` apagado depois de `INVALID` (observado); tempos da janela `dense` sem baseline; `same.*` na janela real | seções acima |

## O que isto NÃO mostra

- Nenhum "sem regressão", nenhum "validado", nenhum "desempenho OK": `run.py` deu `FAIL`; o A/B não julgou; `native.py` não tem baseline.
- Que o `FAIL` do `deep` seja regressão do produto (a máquina estava no limite da guarda e o A/B, que usaria a mesma carga, ficou INVALID), nem que seja só ruído.
- Que o monitor deixe de desenhar um commit (item (16)) ou que a escala do `dense` seja invisível por defeito do produto (item (17)).
- O host, o renderer e a WebView em A/B (BL015.23 (14)); o caso com splat (BL015.23 (1)); orçamentos nativos; as provas de processo de `0ee2d14`.
- Que o `memo_off` seja "sobrevivente": ele foi **detectado**; é a classificação do runner que precisa corrigir (item (19)).
- Reprodutibilidade: cada estado julgado (`FAIL` do `run.py` e do `native.py`, os resultados do `mutate.py`) rodou **uma vez**, com uma carga de base de ~5,5 e quatro processos de terceiros a ~100% de CPU; nada foi repetido para conferir.

## Arquivos (gerado: caminho, tamanho, sha256; `.gz`/`.tgz` são o arquivo cru comprimido sem alteração)

`SHA256SUMS.txt` traz o sha256 de cada arquivo **cru** (inclusive o de cada membro dos `.tgz` e o do conteúdo descomprimido dos `.gz`); a tabela abaixo mostra o sha256 do arquivo cru (nos `.gz`, o do conteúdo descomprimido) e, nos `.tgz`, o do próprio `.tgz`.

| Arquivo | Bytes | sha256 (do arquivo cru; nos `.tgz`, do próprio `.tgz`) | O que é |
|---|---|---|---|
| `record.txt` | 64348 | `7e819144307223f6738f823f09616a328ad1206f5311417b9b7a902c6dfae002` | registro cru: cada passo (comando, UTC, exit, carga e disco antes e depois, CPU alta), as sondagens da guarda e as anotações da sessão |
| `native/build.json` | 772 | `b8bd1cb32777636d1103bcc660efe73195dea7f49c4de82cdb1234c890a0c754` | saída do `native.py --phase build` (sha256 do binário, do `runtime/dist` e da jornada) |
| `native/manifest.build.json` | 4273 | `ff06854fbfe69c4dcc126e3ace40b557edc1b16fec276d51af4e3a39f255b0b0` | manifesto do build nativo (identidade, guardas, toolchain) |
| `native/manifest.json` | 10273 | `cd855db76d7b066bfbe2511769bc32d1043ab668b1c5ac92342a8dd2df2a13df` | manifesto da corrida nativa (identidade, guardas, verificações que falharam, hashes das capturas) |
| `native/report.json.gz` | 35138 | `76e6a0b2a0ee7054802f13b06606537063d015627d3228710ea6826ba8374645` | relatório completo da corrida nativa (639 verificações, amostras de cliente, imagens, `overlay_drawn`, trace) |
| `core/build.json` | 242 | `b29dff899e0f2916df6fbfac890b2b710804b0c142104584dfe80f20c0a1fabf` | saída do `run.py --phase build` |
| `core/manifest.build.json` | 1953 | `eb91772de2e0a4e6ed03b372309740ef7f1ce190cba6e6bca89fdb9d2ac9daca` | manifesto do build do `run.py` (sha256 do executável, `stderr` do mbx) |
| `core/manifest.json` | 7494 | `e0fc424f3e0bdc2c7ae1607e0c8c3aed3ca7b06634c324b2396c6f3757b93742` | manifesto da medição do `run.py` (`run.env`, guardas, `tests_passed`) |
| `core/stdout.txt.gz` | 902 | `0af561b3808a363edc775d9870beae03f28301fba3bcea72814b2d4399d33f59` | `stdout` do teste (`2 passed; 1 failed`, os `GIZMO_PERF_SUMMARY` por corpus); comprimido só porque o original termina com uma linha em branco, que o `git diff --check` recusa (o sha256 é o `stdout_sha256` do `core/manifest.json`) |
| `core/stderr.txt` | 578 | `5c92adb6866cc92546cd5c957abdd1bb6611b148c18c67c1720bdfa1b8d1746b` | `stderr` do teste (a lista `over budget:`) |
| `core/report.json.gz` | 39562 | `ef2b7ea82c0d78ef4a53bbc099f7dde41930a6b18cef4e16ad201732d752c7db` | relatório do `run.py` (todas as amostras por corpus e tipo) |
| `ab/manifest.build.json` | 3709 | `63130ae41807f12092beca66292dc51fd1d421094a9bab0031e3ee4bc922116d` | manifesto do build do A/B (executáveis e seus sha256, árvores de produção, `product_files_*`) |
| `ab/attempt1.manifest.json` | 12870 | `264a6d727d2820bf01867cd2350c766f4c5d1a087840d6f920b2d7af11218249` | manifesto da tentativa 1 do A/B (`INVALID_GUARD_AFTER_RUN`); `verdicts.json` e rodadas dela foram apagados pelo runner (item (21)) |
| `ab/attempt2.manifest.json` | 12986 | `2b50e23c1cc88b69419ac84d2c3a5b7241d4597742926b1da781217abffe1201` | manifesto da tentativa 2 do A/B (`INVALID_GUARD_AFTER_RUN`) |
| `ab/attempt2.verdicts.json` | 62765 | `6bae3e5134bd44244e8881bf1fef7d3c1c2da233ab3b2525b24d3acb5939bc8c` | linhas por tipo da tentativa 2 (observação; **não** é veredito) |
| `ab/attempt2.rounds.tgz` | 400194 | `b25712263ea9b28bca3ef18bee2175ddc78be01ae85667d85819458a70e46304` | os 10 relatórios por rodada da tentativa 2 (2 de aquecimento + 8 mantidas) (contém: round00-baseline.json, round00-baseline.stdout.txt, round01-candidate.json, round01-candidate.stdout.txt, round02-candidate.json, round02-candidate.stdout.txt, round03-baseline.json, round03-baseline.stdout.txt, round04-baseline.json, round04-baseline.stdout.txt, round05-candidate.json, round05-candidate.stdout.txt, round06-candidate.json, round06-candidate.stdout.txt, round07-baseline.json, round07-baseline.stdout.txt, warmup-baseline.json, warmup-candidate.json) |
| `mutants/prepare1.manifest.json` | 10416 | `d99f0c460de1e77e996cf51e3ba56b76aa3dcca490e302b249efbca3b21bcd31` | `prepare` 1 (estrutural e host de cada mutante; restauração da árvore) |
| `mutants/time1.manifest.final.json` | 20148 | `491c168644c7377751cfbd66081b010db9e7c45ad4129fc76a92535c762584d1` | `time` tentativa 2 (a 1ª invocada): `BLOCKED_OR_INFRA` |
| `mutants/time1.reports.tgz` | 89602 | `df54564fd575ce0f0b5ea10c28a07aebbadec48c9f7f2b580ee7a9b6fe42e985` | relatórios, `stdout` e `stderr` do `time` da tentativa 2 (os dois mutantes de tempo) (contém: delay_1ms_per_handler.report.json, delay_1ms_per_handler.time.stderr.txt, delay_1ms_per_handler.time.stdout.txt, extra_plan_per_handler.report.json, extra_plan_per_handler.time.stderr.txt, extra_plan_per_handler.time.stdout.txt) |
| `mutants/prepare2.manifest.json` | 10836 | `6743ccae65d5810bcf7a47017d12990e4914cf240206b3230c43de1a824b2b35` | `prepare` 2 (repetido porque o `time` apagou `exes/`; mesmos resultados) |
| `mutants/time2.manifest.final.json` | 20606 | `ea0f1e1b5f94b0b8d480c05b89daa7fff115fdb3c00dc71ba4b219f72151718d` | `time` retry (o único): `UNEXPECTED` |
| `mutants/time2.reports.tgz` | 90204 | `32f14f4da16574425a560f924a05d826df908f7020df593c235a391f12ba3d21` | relatórios, `stdout` e `stderr` do `time` do retry (contém: delay_1ms_per_handler.report.json, delay_1ms_per_handler.time.stderr.txt, delay_1ms_per_handler.time.stdout.txt, extra_plan_per_handler.report.json, extra_plan_per_handler.time.stderr.txt, extra_plan_per_handler.time.stdout.txt) |

Fora do git (o motivo está na coluna): os executáveis copiados (`native/exes/tessel-editor` 38 MB, `core/exes/candidate.exe`,
`ab/exes/*`, `mutants/exes/*`; apagados no fim, mantidos os JSONs) e as três capturas PNG da abertura:

| Arquivo | Tamanho | sha256 | Motivo |
|---|---|---|---|
| `native/small.open.png` | 453396 B | `31135e55158c8aabc88eb5e46c982fc5076a0dd1150d76a500b53969a014a7ea` | captura observacional do primeiro quadro da janela (PNG); o hash também está em `native/manifest.json`; fora do git por tamanho |
| `native/dense.open.png` | 648014 B | `41182b885271dc9a8771e6c2358effb4ae7d8a7f089c692d2cbf4fd3de5d19b3` | captura observacional do primeiro quadro da janela (PNG); o hash também está em `native/manifest.json`; fora do git por tamanho |
| `native/deep.open.png` | 460745 B | `a70142a1c71697c81e32b2fc2dcc5b4c637fc961d34a3133a45f969fc783511e` | captura observacional do primeiro quadro da janela (PNG); o hash também está em `native/manifest.json`; fora do git por tamanho |
| `native/exes/tessel-editor` | 38795136 B | `fbc7a9c97ac892f105863f55f6c8c7f517a2c32a3f91015bcdeddb26910b5dbd` | executável copiado para a corrida; binário, não vai para o git; removido no fim |
| `core/exes/candidate.exe` | 4090576 B | `9f81ee1f4571d8fd5008ccb67b8501c1e9fac3de916e99c3d2f9f0d08fd05f38` | executável copiado para a corrida; binário, não vai para o git; removido no fim |
| `ab/exes/baseline.exe` | 4073520 B | `f8002313ed2601c77a125a7db79a05568359836ff78f390afcd1101cf3a0b3d6` | executável do A/B (candidato ou baseline); binário, não vai para o git; removido no fim |
| `ab/exes/candidate.exe` | 4090576 B | `9f81ee1f4571d8fd5008ccb67b8501c1e9fac3de916e99c3d2f9f0d08fd05f38` | executável do A/B (candidato ou baseline); binário, não vai para o git; removido no fim |
