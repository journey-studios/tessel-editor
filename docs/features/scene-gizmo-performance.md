# F055 (desempenho) — Suíte de regressão de performance dos gizmos e da abertura (BL015.21)

Complementa [F055](scene-gizmo.md). Pedido: "garantir que não perdemos performance e que validamos o mesmo
cenário de abrir o app visualmente; o que for possível por teste unitário". O código de produção tocado é o contador de
layouts (`scene_plan::probe` + a chamada em `ScenePlan::build_with`, ver abaixo) e, desde a rodada 2 de correções, **uma
mudança de comportamento do monitor** (`scene_monitor_host.rs`): o id da cena entra na chave de deduplicação de `ViewportHost::render_scene`
(BL015.22, um defeito de produto anterior à PR que a própria jornada encontrou; ver "Achados e pendências"). O custo do
contador **não é assumido nulo**: só o A/B o mede (e ele ainda não foi medido); a mudança do monitor, em `crates/desktop`,
fica **fora** do que o A/B mede (ver "Tempo").

## Objetivo e limites

Uma **jornada única e versionada** (`docs/migration/fixtures/gizmo-perf/journey.json`, v1: abrir projeto, anexar,
resolver recursos, exibir, primeiro quadro, selecionar, translação/rotação (local e mundo)/escala (uniforme e por eixo),
confirmar, cancelar, undo/redo, auxiliar oculto, trocar de cena, redimensionar, diálogo) sobre um **corpus sintético
compartilhado** (`corpus.manifest.json`, com sha256 do gerador, da jornada e de cada arquivo; nada privado):

| Caso | O que é | Por quê |
|---|---|---|
| `small` | o corpus da prova de cena (6 nós no manifesto, animados) | abertura típica |
| `dense` | + 12 bancos × 50 planos girados (618 nós no manifesto) | custo que cresce com o número de nós |
| `deep` | cadeia de 24 pais girados + 12 anisotrópicos (44 nós no manifesto) | pais transformados (matemática da direção local/escala) |

Contagens são as de `corpus.manifest.json` (a raiz entra; versões anteriores deste texto diziam 5 e 617). Gaps declarados: **não há caso com splat** (o gerador não cria splat; o pedido o inclui
"onde suportado"; BL015.23); sem GPU real nem ponteiro físico (BL015.2/BL043/BL044 seguem pendentes); sem alegar
"custo 0".

A jornada é **uma só** (`journey.json` v1, sha256 no manifesto do corpus): o core, o host GPUI e a janela nativa a
carregam do mesmo arquivo e a janela nativa recusa um corpus cujo manifesto não bate com o que está no disco. Abrir e o
primeiro quadro são fases `cold` separadas das fases `hot`; o core **não tem quadro** (só a janela e o host
apresentam), então "primeiro quadro" nunca foi uma medida do core frio — o texto anterior que dizia isso estava errado.

## Contrato do core / biblioteca

A jornada roda pelos **mesmos handlers** nas três superfícies; cada passo é `document` (equivalente: mesmo estado,
mesmo undo), `presentation` (precisa de uma janela que apresente) ou `presentation_only` (resize/diálogo):

| Superfície | Onde | O que prova | O que **não** prova |
|---|---|---|---|
| Core (lib) | `crates/core/tests/gizmo_perf_support` + `scene_gizmo_perf_structure.rs` (7 testes, **sempre** rodam, determinísticos), `scene_gizmo_perf_threads.rs` (1 teste: nenhum layout escapa da thread lida; **executado e passa em `a97a0a7`**) e `scene_gizmo_perf_budget.rs` (`#[ignore]`, opt-in controlado; **compilado** pelo `clippy --all-targets` e pelo `cargo test`, onde seus 3 testes aparecem como `ignored`, **nunca executado neste head**) | transformada/undo/redo idênticos ao golden (`journey.expected.json`), cancelar não muda nada, desfazer restaura o valor exato; **planos por requisição** (`scene_plan::probe::layouts_built`), independência do tamanho da cena, 60 moves = 60 plans, chave da memória do overlay; tempo em release | janela, GPU, página |
| Host GPUI | `crates/desktop/src/workstation/scene_gizmo_perf_tests.rs` (5 testes, **todos passam em `c3ae380`**; em `a97a0a7` eram 4 e o 4º (`showing_a_scene_that_shares_its_bundle_with_the_shown_one_shows_it`) **falhava**: defeito de produto anterior à PR, BL015.22, **corrigido na rodada 2**, ver "Achados e pendências"; o teste do ciclo de vida do monitor `scene_lifecycle_tests::a_scene_of_the_same_bundle_at_the_same_frame_is_another_scene_and_a_repeat_of_one_scene_is_not` completa a cobertura da correção) | a MESMA jornada por `Workstation::run_op` (mesmo golden), incluindo `monitor.show_scene` e resize; layouts por requisição **no host**, contra um **teto por rótulo** (`host_budget`: 0 para `project.create`; 3 para `scene.pointer.down`, `scene.pointer.up` e `monitor.show_scene`; 2 para os outros dez rótulos da jornada), que é o contrato do core mais o do próprio monitor. **O teto não é o medido de cada passo**: o comentário de `host_budget` o define como o máximo de layouts por requisição medido na árvore em que a suíte foi escrita (todo corpus, todo passo) e diz que um número menor nunca falha; onde o medido é menor que o teto, **um plano extra passa** (m7, BL015.23 item 9), então o teste só pega um excesso **acima** do teto; janela parada (100 ticks do timer = 0 layouts; **no máximo 1 layout por sync** do mesmo estado — o do próprio monitor —, ou seja, ≤ 20 para 20 syncs, que é o que o teste afirma (`<= SYNCS`); 0 pedidos do overlay, só `memo_hits`). O texto anterior ("20 syncs = no máximo 1 layout") não era o que o teste verificava; o contrato é o do teste (uma cota agregada de 1 por sync, não uma checagem sync a sync: um sync com 2 layouts compensado por outro com 0 passaria). Também a cena de **mesmo bundle** (`same.*`, BL015.22) | WebView, GPU, compositor, ponteiro físico; um plano extra numa requisição cujo medido fica abaixo do teto (m7, BL015.23); o diálogo (precisa do `Root` do Kit: é da janela nativa) |
| Janela nativa | `scripts/gizmo-perf/native.py` (processo próprio, `HOME`/socket próprios, WKWebView, só requisições semânticas) | a mesma jornada contra o mesmo golden, **pixels** (a imagem do monitor muda após um commit, volta ao hash exato após o undo, não muda após cancelar), o overlay desenhado e confirmado pela página (sondas de cor), tempos do cliente e os do próprio runtime | compositor (ver abaixo) |

`scene_plan::probe` (core, sem `unsafe`: o workspace proíbe `unsafe_code`, logo não há contador de alocações nem de
CPU; limite declarado) conta cada `ScenePlan::build_with` (o layout completo da cena), **por thread** (a suíte
estrutural lê a sua, exata num `cargo test` paralelo) e **por processo** (um `fetch_add` relaxado). É o **detector
estrutural**: um plano a mais por handler ou por tick falha o teste sem relógio.
- O ponto cego do contador por thread (um handler que planejasse em outra thread leria 0 e passaria) é coberto por
  `scene_gizmo_perf_threads.rs`: num binário de um único teste, a jornada gasta os mesmos layouts por requisição lida por
  thread e por processo, e um controle negativo roda a jornada noutra thread e exige que o contador da thread não mude e o
  do processo cresça. **Executado: passa em `a97a0a7`** (1 teste, 3,58 s). O controle negativo é o do próprio teste; nenhum
  mutante de produto foi rodado contra ele.
- `PlanTimeline::build_with` **deixou de ser contado**: o contador (`timeline_plans_built`) não tinha consumidor
  (código morto no caminho quente). O caminho da linha do tempo fica **sem detector estrutural** (BL015.23).
- O custo do próprio contador **não é assumido desprezível**. O A/B constrói a baseline *sem* a chamada
  (`ab.py` só dá à baseline o arquivo `probe.rs` e a linha `pub mod probe;`) e a candidata com ela: a diferença de
  tempo inclui o contador. Sem A/B medido, isso está **pendente**, não "zero".

**Tempo.** A medição de tempo é a razão entre a requisição e o `ScenePlan::build_with` da própria cena, medido em blocos
de 5 antes de cada janela de amostras; isso cancela a velocidade e a carga da máquina. Orçamentos por caso e tipo em
`budgets.json` (regra e margens em `scripts/gizmo-perf/budgets.py`; o arquivo guarda o sha256 dos relatórios de
calibração). Os orçamentos **não** foram pré-registrados: veja "Histórico dos orçamentos". Métricas: mediana, p95, p99, máximo, amostras perdidas, erros, ticks
longos (> 16,6 ms); **nenhuma amostra é descartada** e frio/quente ficam separados. O teste frio repete **60 vezes**, com editores
novos, a medida de `scene.attach` (`cold.attach`) e a do primeiro `scene.gizmo.state` da cena (`cold.state`, em outro editor
novo, porque o bloco de referência já montou o plano no primeiro), pelos handlers públicos do core; as 60 amostras de cada
ficam e "a primeira incluída" quer dizer a primeira **execução** (o aquecimento do processo é declarado no relatório, não
escondido). O core **não tem quadro**: nenhum tempo de primeiro quadro ou de apresentação é atribuído a
ele; isso só existe no host (sem WebView) e na janela nativa. `INCONCLUSIVE` quando mais de 25% dos blocos de referência
discordam de si mesmos em > 1,5×;
`BLOCKED` quando o lock `/tmp/tessel-scene-gpu.lock` está ocupado (uma tentativa, sem espera), a carga (1 **e** 5
min) passa de 0,5 por CPU, o disco está abaixo do piso ou há limite térmico. Em Apple silicon o `pmset` não informa
`CPU_Speed_Limit`: o relatório diz `thermal_state.verified = false` ("sem limite reportado; estrangulamento não
excluído"), nunca "silencioso". As guardas são lidas **de novo depois** da corrida: uma guarda ruim depois torna a
corrida `INVALID_GUARD_AFTER_RUN` (não é PASS). As duas "injeções" (um plano a mais; 1 ms) são **controles
negativos** do próprio detector, nunca prova de desempenho.

**O que o tempo permanente e o detector estrutural NÃO veem (M2 da revisão R).** O orçamento de tempo é uma razão entre a
requisição e o `ScenePlan::build_with` **da própria cena, medido no mesmo processo**. Um planejador 2× mais lento dobra a
parte da requisição que é layout **e** a referência, e a razão **cai**: com uma chamada de layout por requisição, r' = (r + 1)/2 (com k chamadas,
r' = (r + k)/2; sempre menor que r; com r = 8 e k = 1 a razão passa a 4,5). Logo ele passaria nos orçamentos permanentes, que
só limitam a razão por cima (derivação do desenho do teste, **não executada**: não existe mutante de planejador mais lento). O detector estrutural conta **chamadas** de layout por requisição, não o custo de cada uma, e
também passaria. O **A/B contra a `main`** é a única comparação **absoluta** com uma baseline independente (p50 e p95 da
requisição por tipo, rodadas alternadas na mesma máquina), mas **só enxerga o que passa dos seus limiares sobre o TOTAL da
requisição**: `ab.py` não compara o tempo absoluto da referência de layout (a referência só entra como nota de
estabilidade: INCONCLUSIVE se instável; as razões por rodada são registradas, não julgadas). Ele reprova se o p50 passa de
1,10× ou o p95 de 1,25× o da baseline (`MEDIAN_TOLERANCE`, `P95_TOLERANCE`), cada um **e** com o candidato mais lento em
maioria estrita dos pares de rodadas, ou se perde amostras ou tem tick longo a mais. Logo ele **pode não ver** um planejador
2× mais lento quando o layout é pequena parte da requisição: no exemplo da revisão (aritmética, **não medida**), uma requisição
que vai de 13 ms a 14 ms porque o layout vai de 1 ms a 2 ms sobe 7,7%, abaixo do limiar de 10% do p50. **Requisito de
processo** (BL015.23), **no escopo que o A/B realmente mede**: uma PR que toque o **core** (`crates/core/src`, o `build.rs` e o
`Cargo.toml` do core, em particular `scene_plan/**`, e as entradas de build do workspace: `Cargo.toml`, `Cargo.lock`,
`rust-toolchain.toml`, `.cargo`, `vendor`; é o `is_measured` de `ab.py`, porque o A/B só roda `cargo test -p
tessel-editor-core --test scene_gizmo_perf_budget`) precisa do A/B contra a `main` (`ab.py`, build e measure, com lock e
guardas; ver "A/B contra a `main` de agora") antes de qualquer afirmação de que o **core** "não perdeu desempenho", **mas ele
é necessário, não suficiente**: um A/B sem regressão só diz que nada passou dos limiares sobre a requisição inteira, neste
cenário, configuração e máquina, e nenhum teste permanente fecha o que ele não vê (o A/B também **nunca foi executado**).
**O A/B não vê o host, o renderer nem a WebView.** Uma PR que mude só `crates/desktop` ou `crates/render` recebe
`DIFFERENCE_OUTSIDE_THE_MEASUREMENT` (exit 4; os arquivos ficam em `product_files_outside_the_measurement`), nunca um "sem
regressão" (o JS do `runtime/` nem entra na lista de produto do runner, `is_prod`: uma PR só nele recebe
`A_A_NO_PRODUCT_DIFFERENCE`); uma PR que mude o core **e** o desktop recebe o veredito da parte do core, e o
`NO_REGRESSION_IN_SCENARIO` dela não diz nada sobre a parte do desktop. Vale para esta própria PR: a mudança de comportamento
do monitor (`scene_monitor_host.rs`, BL015.22) cairia em `product_files_outside_the_measurement` e o A/B **não a mediria**. O
host, o renderer e a WebView precisam de **validação própria**: hoje o teste estrutural do host (`scene_gizmo_perf_tests.rs`,
**executado**) e o `native.py` (**nunca executado**, sem baseline); **não existe comparação baseline × candidato para o host
nem para a janela** (BL015.23 (14), aberto; o desenho dela **não foi feito**). Os mutantes que existem são os de plano a mais
e de 1 ms por chamada. O único detector sem relógio **previsto** é um contador determinístico de trabalho dentro do `probe`
(nós visitados, multiplicações de matriz por layout, por exemplo), **aberto** em BL015.23 (M2(b)); **ele mede trabalho, não
tempo**: um layout que fique mais lento com as mesmas contagens de nós visitados e de multiplicações passaria nele, então no
máximo **estreita** a lacuna e não a fecha. Esse limite é **separado** do limite do A/B acima (o A/B só vê o que passa dos
limiares sobre o TOTAL da requisição); um não substitui o outro. **Nenhuma instrumentação nova foi feita nesta rodada.**

**A/B contra a `main` de agora (e não só contra a `cbe9e1b`).** Os comandos de "CLI headless" usam a baseline **fixada** no
código (`BASELINE` em `ab.py` = `cbe9e1b393861febf073633fbbaa0da9857780b6`, a `main` em que esta PR se baseia; na data desta
rodada `git ls-remote origin refs/heads/main` ainda devolvia esse commit). Se a `main` andou, por exemplo ficou mais rápida,
uma regressão **em relação a ela** pode passar dentro das tolerâncias de uma comparação com a `cbe9e1b`. Para comparar com a
`main` atual (lido em `ab.py` e **não executado**: a spec não passa `--baseline` nem `--allow-other-baseline` por `main`; o
que ela exercita com repositórios git temporários é o `make_baseline`):

```
OUT=/tmp/gizmo-perf-out     # o diretório do A/B é "$OUT/ab", o mesmo nas duas fases; os outros runners têm o seu (ver "CLI headless")
unset BASE                  # um BASE de uma sessão anterior não pode sobreviver a um fetch que falhou
git fetch origin main && BASE=$(git rev-parse --verify FETCH_HEAD) && git log -1 --format='%H %cs %s' "$BASE"  # confira o commit impresso
python3 scripts/gizmo-perf/ab.py --phase build   --baseline "${BASE:?o fetch falhou}" --allow-other-baseline --out "$OUT/ab" \
  && python3 scripts/gizmo-perf/ab.py --phase measure --baseline "$BASE" --allow-other-baseline --out "$OUT/ab"
```

O `&&` liga o fetch à resolução: se o `git fetch` falhar, `BASE` não é definido, o `git log` não roda e a fase de build recusa
(`${BASE:?}`); nada roda contra uma referência antiga. `BASE` vem do `FETCH_HEAD` **do fetch que acabou de dar certo**, não de
`origin/main`: um `git fetch origin main` só atualiza `origin/main` se o `remote.origin.fetch` do clone mapear `main`, mas
sempre escreve `FETCH_HEAD`. O SHA é copiado **já**, para a variável, porque o próximo `git fetch` reescreve `FETCH_HEAD`
(copiado para um shell novo, vai como o SHA literal). O runner não busca nada: o fetch é de quem roda.

- `--baseline REF` (padrão: a `cbe9e1b…` fixada): em **cada** invocação, nas duas fases, o runner faz `git rev-parse REF` e
  guarda o commit resolvido em `baseline_commit`. `--allow-other-baseline`: sem ela, um commit resolvido diferente do fixado
  é `REFUSED` (`the baseline is pinned to …`) antes de construir ou medir; com ela é aceito e o manifesto registra
  `baseline_is_pinned: false`. Os dois valem para **as duas fases**: `measure` também resolve o REF e também recusa sem a
  flag. Enquanto a `main` for a `cbe9e1b…`, a flag é dispensável (`baseline_is_pinned: true`).
- Entre as fases, `measure` lê o `manifest.build.json` de `--out` e recusa (`REFUSED`) se: o HEAD ou a árvore do candidato
  não são os do build; o build foi de árvore suja (salvo `--allow-dirty`); o `baseline_commit` resolvido agora não é o do
  build (`the executables were built against another baseline`); `--out/baseline-src` sumiu; o sha256 de um dos dois
  executáveis não é o registrado. A comparação é entre commits **resolvidos**, não entre o texto do REF: com um REF que anda
  (`origin/main`, `main`) nas duas fases, um `git fetch` entre elas faria o `measure` ser recusado; o mesmo SHA literal nas
  duas (o `$BASE` copiado do `FETCH_HEAD`, acima) elimina a dúvida. O runner **não sabe qual commit se queria**: que `$BASE`
  é a `main` desejada é conferência de quem roda (o `git log -1` acima); o manifesto guarda `baseline` (o REF dado),
  `baseline_commit` e `baseline_is_pinned` para quem lê depois.
- Contra outro commit, o `build` mede o que difere **no que o executável medido enxerga** (o core e as entradas de build do
  workspace, `is_measured`): `make_baseline` copia do **candidato** só os testes do orçamento, o suporte e as fixtures; o
  `probe.rs` e a linha `pub mod probe;` só entram se o commit ainda não tiver `probe.rs` (e `REFUSED_CONTAMINATION` se
  `scene_plan/mod.rs` do candidato diferir do commit por mais que essa linha); o `build` imprime `product_files_that_differ`
  (os arquivos do que é medido) e `product_files_outside_the_measurement` (os outros arquivos de produto que diferem, como
  `crates/desktop/src`). Se o commit escolhido tem mudanças **no core** que o candidato não tem, elas aparecem em
  `product_files_that_differ` e **entram no número**: o runner não isola a mudança da PR, e para isso o candidato precisa
  conter o commit da baseline (rebase ou merge antes do `build`). **As diferenças em `crates/desktop/src` (e no render) NÃO
  entram no número**: ficam só em `product_files_outside_the_measurement`, e um resultado sem regressão **não permite
  concluir nada sobre elas, nem quando o core também difere**.

## CLI headless

`./scripts/headless` já executa o mesmo vocabulário JSONL (`scene.*`, `history.*`, `project.create`); o driver do core
usa `headless::execute`, o mesmo handler. A suíte **não** acrescenta comando novo (nenhum controle novo: nada em `tree.rs`
nem parser). Como rodar:

```
./scripts/cargo test -p tessel-editor-core --locked --test scene_gizmo_perf_structure     # estrutural, sempre
./scripts/cargo test -p tessel-editor-core --locked --test scene_gizmo_perf_threads       # nenhum layout escapa da thread lida (binário próprio)
./scripts/cargo test -p tessel-editor --locked scene_gizmo_perf                          # host GPUI (5 testes)
OUT=/tmp/gizmo-perf-out                                              # UM subdiretório por runner (abaixo); nunca o mesmo para runners diferentes
python3 scripts/gizmo-perf/run.py --phase build --out "$OUT/core"       # release do teste de tempo (sem lock)
python3 scripts/gizmo-perf/run.py --phase measure --out "$OUT/core"     # tempo no core (lock, guardas, manifesto; não constrói)
python3 scripts/gizmo-perf/ab.py --phase build --out "$OUT/ab"          # baseline fixada cbe9e1b + candidato (sem lock)
python3 scripts/gizmo-perf/ab.py --phase measure --out "$OUT/ab"        # A/B (lock, guardas); outra main: ver "A/B contra a `main` de agora"
python3 scripts/gizmo-perf/mutate.py --phase prepare --out "$OUT/mutants"  # mutantes de produto: estrutural e host
python3 scripts/gizmo-perf/mutate.py --phase time --out "$OUT/mutants"     # mutantes de tempo (lock, guardas)
python3 scripts/gizmo-perf/native.py --phase build --out "$OUT/native"  # editor release do HEAD, com proveniência
python3 scripts/gizmo-perf/native.py --phase run --out "$OUT/native"    # janela real (precisa ./scripts/runtime setup)
```

**Um diretório por runner, o mesmo em todas as fases do mesmo runner** (`"$OUT/core"`, `"$OUT/ab"`, `"$OUT/mutants"`,
`"$OUT/native"`): é nele que o `measure`, o `time` e o `run` acham o que o `build` e o `prepare` deixaram. Os runners escrevem
arquivos de mesmo nome com sentidos diferentes (`manifest.json`, `manifest.build.json`, `build.json`, `report.json`,
`exes/`): com um `--out` só, o A/B sobrescreveria o `manifest.json` do `run.py`, e `mutate.py --phase prepare` sobrescreveria o
do A/B e **apagaria** `--out/exes` (os executáveis de um A/B BLOCKED, que o `measure` repetiria). Nenhum runner confere isso
(não há trava no código): é de quem roda. A única trava é a do `mutate.py` contra duas execuções **simultâneas** na mesma
`--out` (`prepare` e `time`): a segunda sai com `REFUSED` (exit 1) sem escrever nada, por causa de `--out/.mutate.lock`.

## UI e CLI da janela

`native.py` usa o socket (`op`/`get`/`set`/`invoke`), nunca coordenadas nem teclas. Captura nativa
(`screencapture -l`) é só observacional (uma por caso). Resize e diálogo são `ui set window size` e `ui invoke flow.jobs`.
Apresentação: o relatório do runtime traz `render_ms`/`total_ms` e o hash lido de volta de cada quadro (`last_frame`) e os
vãos do host (`plan`, `evaluate`, `ipc`, `serve`); os quadros de cena dirigida **não** recebem o carimbo de apresentação
da sessão (`frames_total` = 0), então `presented_after_done_ms` não existe para eles e o último salto (compositor) **não é
medido**. Uma resposta de socket ou o `frame.done` da página nunca contam como prova de pintura.

## Evidências e status

Preenchido com os números da entrega em [Estado desta entrega e evidência](#estado-desta-entrega-e-evidência). Regra de leitura: **implementado** (código
e testes que rodam) ≠ **validado** (números/provas com identidade); o que é autorrelato fica marcado como tal.

### Histórico dos orçamentos (o que foi e o que não foi pré-registrado)

Os orçamentos de `budgets.json` vêm de calibrações sobre **esta mesma árvore** (nenhuma base anterior), em três
esquemas sucessivos (referência única → INCONCLUSIVE porque derivava 2×; blocos de referência, 25 execuções frias;
blocos, 60 execuções frias: `calib1..3`, `calibB1..3`, `calibC1..3`). A primeira corrida julgada com o esquema final
(`try1`) ficou INCONCLUSIVE e a segunda (`try2`) **FALHOU** (`dense/cold.state` p95 2,61× > 1,45; `cold.attach` p95 7,21 >
7,1). Só **depois** disso as margens frias foram ampliadas (1,25/2,0 em `budgets.py`) e as execuções frias passaram a 60: o
limite p95 de `dense/cold.state` foi de **1,45 para 5,6**. Logo: **as margens frias foram escolhidas depois de ver um FAIL; não são
"pré-registradas" nem "fixadas antes de julgar"**; o `PASS` do `5cded7e` é o de uma regra já ajustada, e uma calibração
sobre a mesma árvore **não prova ausência de regressão** (ela só descreve o que a árvore faz).

**Critério registrado ANTES do próximo julgamento** (qualquer mudança é um commit novo e vale para a corrida seguinte):

1. Os orçamentos existentes ficam como estão; **nenhuma margem muda para fazer algo passar**. O que passa a valer como
   prova de regressão é o **A/B contra a baseline independente** (`cbe9e1b` com o código de produção intacto) e o
   contrato estrutural, não a calibração.
2. Métricas por tipo de requisição: p50, p95, p99, máximo, amostras perdidas e ticks longos. p50 e p95 julgam (A/B: > 10%
   e > 25% sobre a baseline, com maioria dos pares de rodadas); p99 e máximo são **registrados, não julgam** (um soluço do
   agendador numa máquina compartilhada decidiria o veredito); amostra perdida ou tick longo que a baseline não teve
   reprova.
3. Ruído: baseline que varia > 15% entre rodadas, ou > 25% dos blocos de referência discordando de si em > 1,5×, torna o
   tipo/corpus INCONCLUSIVE. Nada de melhor-de-N, nenhuma amostra descartada.
4. Um tipo só tem orçamento de tempo "adequado" se uma injeção de 1 ms **realmente medida** o reprova; um relógio falso
   não prova o produto.

**Auditoria da folga** (`scripts/gizmo-perf/audit.py`, saída em
[`budget-audit-final1-5cded7e.json`](../migration/gizmo-perf-evidence/budget-audit-final1-5cded7e.json), sobre o relatório
retido do `5cded7e`): em **22 de 60** tipos **um plano extra passa em tempo**: 16 dos 18 tipos `move.*`/`up.*` (só
`deep/move.translate` e `deep/up.translate` o veem), os três `cold.attach` e `small/down.rotate`, `small/read.rotate` e
`small/state.rotate`; em 17/60 a folga do p95 é ≥ 2×, em 6/60 ≥ 3×, e em 1 tipo (`dense/cold.attach`)
um atraso de 1 ms passa (convertido com o tempo de plano medido). Isto é uma propriedade do orçamento, não um defeito
corrigido: **o tempo sozinho não detecta um plano extra nesses 22 tipos**. É o detector estrutural (plano por requisição,
exato) que cobre isso, ortogonal ao tempo; os dois têm controles negativos (um plano a mais falha o estrutural; 1 ms falha o
tempo onde o orçamento é estreito). Onde a folga é larga, o tempo é só um teto grosso.

## Achados e pendências (não são regressões provadas)

- `pointer.move`/`up`: a razão p50 requisição/plano observada no release (relatório retido do `5cded7e`) é de 5,3–7,2 em
  `dense` e `deep` (denso ≈ 4 ms por requisição) e de 8,7–13,9 em `small`, **contando um único layout por requisição** (o
  teste estrutural prova 1 layout por move): o resto seria do gesto (candidato, planos de escala/rotação; hipótese, não
  medida). Não há trabalho duplicado demonstrado e **nenhuma otimização foi feita**; sem A/B mostrando regressão contra a
  `main`, nada a corrigir. Desempenho: pendente.
- BL015.22 — `monitor.show_scene` entre duas cenas do **mesmo `bundleId`**: **defeito de produto anterior a esta PR,
  reproduzido no host de forma determinística e CORRIGIDO na rodada 2** (`bab9290`, `fix(viewport)`; a hipótese antiga,
  "não reproduzida", era do relatório nativo único e deixou de valer). A jornada inclui o caso
  (`same.attach`/`same.resources`/`same.show`: `proof-c` do mesmo arquivo de `proof-b`, com `expect.shown_scene = proof-c`).
  O teste do host `showing_a_scene_that_shares_its_bundle_with_the_shown_one_shows_it` **FALHAVA em `a97a0a7`**, no grupo e
  isolado (`left: Some("proof-b")`, `right: Some("proof-c")`), e **passa em `c3ae380`**. Não se evitou o caso nem se afrouxou
  o teste (sem `#[ignore]`, sem mudar a expectativa nem o corpus; só o comentário do teste foi reescrito).
  - *Correção*: o id da cena entra na chave de deduplicação (`format!("{scene:?}|{}|{:?}|{:?}", plano, mapa, opções)`). O ramo
    de dedupe segue chamando `dispatch_directed()` e retornando sem reenviar nada; `note_session(SceneChanged)` roda uma vez por
    troca de cena (também entre cenas do mesmo bundle) e nenhuma para a repetição da mesma cena. **Nenhum `Command`, nó de
    `tree.rs` ou parser novo; o core não mudou** (`cargo tree -p tessel-editor-core` sem gpui/wry). Biblioteca e CLI headless:
    **N/A justificado**: o monitor é um adaptador de apresentação do GPUI (o core não tem monitor, cena dirigida nem chave de
    dedupe); a superfície da janela é a `monitor.show_scene` que já existia.
  - *O que foi executado* (árvore limpa de `c3ae380`): os 5 testes `scene_gizmo_perf` do host passam e o crate
    `tessel-editor` inteiro passa (418 testes, 0 falhas). Testes novos: o do ciclo de vida do monitor
    (`scene_lifecycle_tests`: uma troca de cena por segmento, nenhum envio para a repetição da mesma cena) e a sequência E1–E6
    por `Workstation::run_op` em `monitor_show_scene_follows_the_request_between_scenes_of_one_bundle_and_of_another`.
    **Controle negativo**: com a chave ANTIGA recolocada, **3 testes FALHAM** (o do ciclo de vida, o E1–E6 e o
    `showing_a_scene_that_shares...` existente) e os outros 5 passam (os 3 demais do host e 2 rascunhos):
    [`bl01522-host-tests-vs-old-key.log`](../migration/gizmo-perf-evidence/bl01522-host-tests-vs-old-key.log).
  - *O que NÃO está provado*: o efeito na **janela nativa** (o overlay e o ponteiro da página escolhem a cena por
    `directed_scene()`; isso foi lido no código, nunca executado) e a jornada atual, com `same.*`, **nunca rodou na janela
    real**. O que a página desenha não é observado pelo host de teste (sem WebView).
  - *O que o relatório nativo retido mostra, exatamente* (M1): em `native-functional-only-small.tgz` (jornada de `037eda5`,
    corpus `small`, sem lock nem guardas), a verificação `switch.show: the shown scene drew its picture` tem
    `directed_scene: "proof-b"`, com `proof-b` anexada **do mesmo arquivo** que `proof` (mesmo `bundleId`, `proof-p-corpus`,
    e a mesma pasta base, nas duas) e com `state.undo = 2, redo = 0` registrado no passo `switch.attach`. Ou seja, **naquela
    corrida o defeito não apareceu**; ela não é uma execução da jornada atual (`same.*` entrou depois) nem do código
    corrigido, e **não prova nem refuta** a causa descrita abaixo. A diferença entre esse resultado e o host com a chave
    antiga **não está explicada**. Os experimentos de rascunho do host
    ([`bl01522-scratch-experiments.rs.txt`](../migration/gizmo-perf-evidence/bl01522-scratch-experiments.rs.txt); saídas em
    [`bl01522-host-scratch-with-fix.log`](../migration/gizmo-perf-evidence/bl01522-host-scratch-with-fix.log) e
    [`bl01522-host-tests-vs-old-key.log`](../migration/gizmo-perf-evidence/bl01522-host-tests-vs-old-key.log)) mostram, no
    host: o **plano** da cena editada por um arrasto difere do da intacta (o mapa de recursos e as opções são iguais), e com
    a edição desfeita plano, mapa e opções voltam a ser iguais aos da intacta e aos de uma cena do mesmo arquivo; depois do
    prefixo da jornada (até `state.hidden`) mais o `scene.attach` de `proof-b` do mesmo arquivo e `monitor.show_scene
    proof-b`, a chave antiga deixa o monitor em `proof` e a nova o leva a `proof-b`. Uma hipótese
    **não testada** para a diferença com o relatório nativo é que o monitor nativo re-renderiza a cena depois das edições
    (chave nova) enquanto a janela de teste do host não roda o timer. Nenhuma frase desta página lê-se como "o relatório
    nativo mostra o efeito".
  - *Causa raiz (executada, com reprodutor mínimo; [`bl01522-same-bundle-repro.log`](../migration/gizmo-perf-evidence/bl01522-same-bundle-repro.log)
    e [`.rs.txt`](../migration/gizmo-perf-evidence/bl01522-same-bundle-repro.rs.txt))*: `ViewportHost::render_scene`
    (`scene_monitor_host.rs`, dedupe de 6a8b030) comparava a chave `plano JSON | mapa de recursos | opções`
    com a do último pedido e, se igual, **retornava antes** de atribuir `directed.scene`. O id da cena não estava na chave: duas
    cenas anexadas do mesmo arquivo têm plano, mapa e opções idênticos (medido: os três iguais). Resultado (antes da correção):
    `monitor.show_scene proof-c` respondia `ok` (`{"action":"monitor.show_scene","scene":"proof-c"}`) e o host seguia reportando `proof-b` como cena
    dirigida **logo depois do pedido** (não era efeito dos redimensionamentos que vêm depois). Controle: mostrar uma cena de
    plano diferente e depois `proof-c` funcionava (`proof` → `proof-c`); mostrar `proof-b` logo depois de `proof-c` não
    trocava (a regra era simétrica e por conteúdo). O que isso **não** prova: o que a página desenha (o host do teste não tem WebView;
    como plano, mapa e opções são idênticos, o quadro pedido ao runtime deveria ser o mesmo, mas isso não foi observado) nem o
    efeito no gizmo da janela real, que só foi **lido no código** (`overlay_refresh`/`overlay_pointer` escolhem a cena por
    `directed_scene()`, ou seja, o overlay e o ponteiro da página seguiriam a cena antiga): **não executado**.
  - *Decisão de produto*: o diagnóstico de `a97a0a7` (id da cena na chave; revertido na época e nunca commitado, com o
    diff em `bl01522-same-bundle-repro.log`) virou a correção por decisão do coordenador na rodada 2: corrigir nesta PR, sem
    `#[ignore]`. As outras opções que o diagnóstico listava (recusa tipada; dedupe legítimo com `directed.scene` atualizada)
    não foram adotadas. O resultado nativo segue pendente.
- A janela nativa não observa `presented` (compositor): o runner marca `presented: not_observable` e exige quadro novo
  correlacionado por request/cena/frame/revisão/hash; isso **não** é prova física de pintura. Faltam também o caso com
  splat e a cobertura estrutural de `PlanTimeline` (BL015.23), e orçamentos por passo do host e da janela.

## O que a revisão 5853504495 mudou nesta entrega

Itens da review da PR #44 (head `ef62bda`), todos implementados. O Rust novo desta rodada foi **compilado e executado pela
primeira vez em `a97a0a7`** (veja "Estado desta entrega e evidência"); o que é tempo, A/B, mutantes de tempo e janela
nativa continua **sem execução** (veja "Estado desta entrega"):

1. **A/B era A/A.** `ab.py` copiava `scene_plan` do candidato sobre a baseline (apagava a diferença do caminho quente). Agora
   a baseline é a `cbe9e1b` fixada, com **só** uma lista de permissões (testes, fixtures, `probe.rs` novo e uma linha
   `pub mod probe;`); registra manifestos da árvore de produção (ids de blob do git) antes/depois do overlay e do
   candidato, **recusa** contaminação (`REFUSED_CONTAMINATION`) e declara A/A quando nenhum arquivo de produção difere.
   O código-fonte da baseline fica em `--out/baseline-src` entre as fases (o teste lê a jornada do caminho em que foi
   compilado) e só é apagado depois de uma corrida julgada; sem ele, `--phase measure` recusa (comentário da revisão do
   CodeRabbit; sem execução). Um candidato com trabalho extra em `scene_plan` continua DIFERENTE da baseline (teste do runner com repositórios git
   temporários). O contador que escapava para outra thread ganhou o detector do item "probe" acima; o contador morto
   do `PlanTimeline` foi removido.
2. **Mutantes.** `MORTO` só pelo teste nomeado **e** a mensagem de orçamento esperada; Cargo 127, binário ausente, 0
   testes, timeout, disco, lock ou erro de compilação são `BLOCKED_OR_INFRA` (exit 3), nunca um kill; timeout finito, guarda
   relida depois da corrida, SIGTERM restaura o checkout, âncora não única para antes de tocar o arquivo, hashes
   (commitado/mutado/restaurado) e logs guardados.
3. **Runner.** Todo filho tem timeout finito e grupo de processo encerrado (SIGTERM→SIGKILL); relatório fail-closed;
   `run.py --phase build|measure` separa o build da medição (sem poll/sleep de resfriamento); a medição não constrói
   sozinha.
4. **Nativo.** `native.py --phase build|run` sobe o **binário do HEAD** (proveniência: HEAD, sha256 do binário,
   `runtime/dist`, manifesto do corpus), cria e salva um projeto e o abre pelo `project.open` real; cada mostra/seek/undo
   exige o quadro **novo** (nada de ack de IPC, cache ou `frame.done` velho); o cancelar prova um candidato **diferente**
   da base ANTES e base/undo/documento corretos DEPOIS. Casos `small`/`dense`/`deep` com as mesmas expectativas.
5. **Orçamentos:** histórico e critério acima; auditoria de folga e regras de métrica registradas.
6. **Docs:** esta página, o README da evidência, o backlog.
7. **Rodada de compilação e execução (depois de `a97a0a7`).** Duas threads abertas do CodeRabbit, cada uma com teste na spec
   ([`native.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4533742510),
   [`ab.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4533742491)), e a primeira execução do
   Rust novo:
   - `native.py::drag`: a imagem **antes** do gesto precisa ter convergido (`ms is not None`) para servir de base de
     `start_hash`/`start_mark`; sem isso nada é pressionado e a verificação falha. No cancelar, a comparação com a imagem
     base exige `ms is not None` (um timeout deixa o último estado visto, que ainda pode ter o hash da base: a verificação
     passava sem quadro novo). Testes `DragBase` (4): contra os runners de `a97a0a7`, 5 dos 6 testes novos da spec falham
     (1 FAIL + 4 ERROR) e o 6º é um controle (`spec-new-tests-vs-a97a0a7-runners.log`).
   - `ab.py`: o comentário pedia a instabilidade da referência lida de `reference.unstable_fraction` de cada rodada e
     comparada com `max_unstable_blocks` de `budgets.json`. A leitura dos relatórios mantidos (em vez da mensagem
     `INCONCLUSIVE` do stdout) e o registro do limite no manifesto já estavam em `a97a0a7` (`unstable_corpora`); faltava o
     limite deixar de ser uma segunda cópia de 0,25: `MAX_UNSTABLE_FRACTION` vem agora de `budgets.json:
     reference.max_unstable_blocks` (o mesmo campo que o teste do core lê) e o runner recusa um valor que não seja fração em
     [0, 1]. Testes `Judge` (2).
   - Os gates do Rust e o achado BL015.22 estão em "Estado desta entrega e evidência".

## Rodada 2 de correções (PR #44, de `9a72e03` a esta entrega)

Commits: `bab9290` (produto, testes Rust e evidência do BL015.22), `c3ae380` (runners, spec e o log do controle negativo) e o
de docs. Cada correção de runner tem testes novos em `gizmo-perf.spec.py` que **falham ou dão erro nos runners de
`9a72e03`** (controle em [`spec-new-tests-vs-9a72e03-runners.log`](../migration/gizmo-perf-evidence/spec-new-tests-vs-9a72e03-runners.log):
107 testes; com a spec nova e os runners novos 107/107 passam; com a spec nova e os runners ANTIGOS, **28 falham ou dão erro**).
Dos 31 testes novos, 3 passam dos dois lados **de propósito** (um guarda contra corrigir demais, um é o caminho feliz e um é
o A/A que agora testa o que diz; o log os descreve). O que esta rodada **não** fez: medir tempo, rodar A/B, mutantes de
tempo ou a janela nativa; mudar `budgets.json`, margens, jornada, corpus ou golden.

| Item | O que mudou | Testes que falham no código antigo |
|---|---|---|
| BL015.22 (CodeRabbit 1) | o id da cena entra na chave de deduplicação de `ViewportHost::render_scene` (`scene_monitor_host.rs`) | host: o do ciclo de vida, o E1–E6 e o `showing_a_scene_that_shares...` (log da chave antiga) |
| 2 (doc) | o teste frio mede `scene.attach` e o primeiro `scene.gizmo.state` da cena, sem tempo de "primeiro quadro" (seção "Tempo") | só texto |
| 3 (doc) | proveniência separada por passo no README da evidência | só texto |
| 4 `ab.py` | o diretório dos executáveis e o código-fonte da baseline só são apagados **depois** de uma corrida julgada; uma corrida BLOCKED/INVALID os mantém para repetir | `MeasurePhase` (a corrida inválida mantém o que a julgada apaga) |
| 5 `harness.py` | `_kill_group` manda SIGTERM e **sempre** SIGKILL ao grupo, mesmo que o líder já tenha saído; a leitura da saída (`_drain`) tem limite e guarda a parcial | `Harness` ×2 |
| 6 `mutate.py` | uma recusa da fase `time` (inclusive a de arquivo de mutante fora do HEAD) não sobrescreve o `manifest.json` de um `prepare` terminado: vai para `manifest.final.json` | `TimePhase` (a recusa não troca o `prepare`) |
| 7 `mutate.py` | a fase `time` recusa um `prepare` não terminado e um mutante que o `prepare` não rodou (não o descarta); um veredito sem nenhum detector é `NO_DETECTOR_EVALUATED` (exit 1), nunca "como esperado" | `TimePhase` ×3 |
| 8 `native.py` | `runtime_source_tree()` devolve `None` se o `git` não inicia, falha, expira ou imprime vazio; `build_phase` e `verify_provenance` recusam | `RuntimeTree` ×4, `BuildPhase`, `Provenance` |
| 9 `native.py`, `run.py`, `ab.py` | o registro de build guarda `dirty`; ausente conta como sujo; medir ou julgar como HEAD recusa um binário de build sujo (sem `--allow-dirty`), mesmo que a árvore volte a ler limpa | `BuildPhase`, `Provenance`, `RunRecord` ×2, `MeasurePhase`, `MainRun` |
| 10 `run.py` | INCONCLUSIVE só se **todo** teste que falhou falhou pela nota de referência instável (mensagem de pânico própria de cada um); um controle que não detectou o trabalho injetado, ao lado da nota, é FAIL | `RunVerdict` ×2 |
| M3 `ab.py` | "difere" é só o que o executável medido vê (`is_measured`: o crate do core e as entradas de build do workspace); diferença só fora disso é `DIFFERENCE_OUTSIDE_THE_MEASUREMENT` (exit 4, arquivos listados à parte), nunca "sem regressão" | `Baseline` ×3 |
| m1 `mutate.py` | `Mutated.__enter__` restaura o arquivo se a entrada falha (o `with` não chama `__exit__`) | `Mutants` |
| m2 spec | o teste do A/A agora é de fato um A/A; o antigo mantém um nome verdadeiro | passam nos dois lados |
| m6 `native.py` | `--corpora` vazio é recusado (zero verificações não é sucesso); um gesto cujo candidato diferente não foi desenhado enquanto aberto falha uma verificação | `MainRun`, `DragBase` ×2 |

Itens **sem código**, registrados como abertos em BL015.23 com o motivo: m5 (um tick longo absoluto decide o veredito),
m7 (tetos do host com folga e o controle de "outra thread" só cobre o core), m9 (`within()` não exige todo tipo
orçamentado medido; só compilado), M2(b) (contador determinístico de trabalho no `probe`), e os nits (a injeção do teste
exige `--test-threads=1`, `mutate.py` sem controle de árvore limpa, `MAX_BLOCK_SPREAD` × `budgets.json`, teste do workspace
só com `-p core`, a redação "whatever" que R aponta em "corpus de referência instável pula o julgamento", não localizada
nos arquivos da suíte) e os comentários de contagem de nós do cabeçalho de `corpus.py`
(arquivo com sha256 no manifesto do corpus: corrigir exigiria regenerar o manifesto, e o corpus não se muda nesta rodada).
O m8 (trailers dos commits) não foi tocado.

## Rodada 3 de correções (PR #44, de `f3b2c32` a esta entrega)

Origem: a review [5854694660](https://github.com/journey-studios/tessel-editor/pull/44#pullrequestreview-5854694660) do
CodeRabbit sobre `f3b2c32` (4 comentários inline e 1 fora do diff, todos Minor, todos válidos) e um comentário enganoso que a
revisão R já apontava (BL015.23 item 11). Commits: `e37a76f` (código: `mutate.py`, a spec, o comentário do teste de orçamento e o
log do controle negativo) e o de docs (esta página, o README dos runners, o backlog, o README da evidência e
[`gates-e37a76f.txt`](../migration/gizmo-perf-evidence/gates-e37a76f.txt)).

| Item | O que mudou | Prova |
|---|---|---|
| `mutate.py` (fora do diff) | a fase `time` só mede o executável cujo hash o `prepare` registrou: sem registro (o build do mutante não deu executável) é `MISSING_EXECUTABLE`, como um arquivo que mudou; o `prepare` esvazia `--out/exes` ao começar, então nenhum `.exe` de um `prepare` anterior (outro HEAD, ou uma fase `time` BLOCKED que o manteve) vale pelo deste. Uma corrida BLOCKED continua mantendo os executáveis | 4 testes novos (`MutantExecutables`): 3 falham nos runners de `f3b2c32`; o 4º (hash registrado diferente do arquivo) passa dos dois lados de propósito, porque o código antigo já o recusava: ele guarda a condição nova e o log mostra a variante errada que ele pega ([`spec-new-tests-vs-f3b2c32-runners.log`](../migration/gizmo-perf-evidence/spec-new-tests-vs-f3b2c32-runners.log)) |
| tabela do host (inline) | o teto de `host_budget` é por rótulo (0, 2 ou 3) e é o máximo medido do rótulo, não o de cada passo: onde o medido é menor, um plano extra passa (m7); acrescentado também à coluna "não prova" | só texto; lido em `scene_gizmo_perf_tests.rs` |
| o A/B só vê o que passa dos limiares (inline) | "só o A/B vê um planejador mais lento" foi reescrito: o A/B julga o p50 e o p95 absolutos da requisição inteira (1,10× e 1,25×, com maioria estrita dos pares de rodadas), não o tempo do layout; 13 → 14 ms porque o layout vai de 1 a 2 ms é +7,7%, abaixo de 10%; "só o contador determinístico (M2(b), aberto) fecha a lacuna sem relógio" (**corrigido na rodada 4**: o contador é o único detector sem relógio previsto, mas mede trabalho, não tempo, e estreita a lacuna sem fechá-la) | só texto; lido em `ab.py` (`judge`) |
| comparar com a `main` atual (inline) | seção "A/B contra a `main` de agora": `git rev-parse origin/main` uma vez (**corrigido na rodada 4**: o SHA sai do `FETCH_HEAD` do `fetch` que deu certo, numa cadeia `&&`), conferir o commit, o mesmo `--baseline SHA --allow-other-baseline` nas duas fases, e o que as flags e o `measure` conferem entre as fases | só texto; lido em `ab.py` (`main`, `make_baseline`), **não executado**; a spec não cobre as flags |
| README dos runners, linha do `run.py` (inline) | proveniência: sem `--exe` valida HEAD, árvore e hash contra `build.json`; com `--exe` NÃO valida (rótulo padrão `candidate`); sem `--allow-dirty` recusa árvore e build sujos; com ela o que seria `PASS` vira `DIRTY_RUN_NOT_A_PASS_FOR_HEAD` (exit 0) **só numa execução de fato suja** (frase da rodada 3 corrigida na rodada 4: com árvore **e** build registrado limpos a flag não muda nada e o `PASS` continua `PASS`; com `--exe` o `build.json` não é lido e só a árvore decide) | só texto; lido em `run.py` (`dirty_run` na l.139, a do build registrado na l.182 e a troca do `PASS` na l.223) |
| comentário de `scene_gizmo_perf_budget.rs` | dizia que `budgets.json` foi "fixed BEFORE the candidate was measured"; agora diz que não foi (calibrações da mesma árvore; margens frias ampliadas depois de um FAIL) | só o comentário muda (5 linhas a mais, 2 a menos); `fmt`, `clippy --all-targets` e o estrutural passam em `e37a76f` |

**O que esta rodada NÃO fez:** medir tempo; rodar de verdade `run.py`, `ab.py`, `mutate.py` (`prepare` ou `time`) ou
`native.py` (a correção do `mutate.py` só foi exercitada pela spec, com git, máquina, lock, build e corrida falsos); build
release de runner; abrir a janela nativa; rodar o workspace inteiro com runtime ou o host GPUI (outra frente; nenhuma fonte do
crate mudou desde `c3ae380`); mudar `budgets.json`, jornada, corpus, golden, margens ou limiares. A máquina estava carregada
por outros projetos (a carga de cada passo está em `gates-e37a76f.txt`). **Nenhuma afirmação de desempenho vale: implementado
não é validado.**

## Rodada 4 de correções (PR #44, de `1b92c9c` a esta entrega)

Origem: a review [5855208747](https://github.com/journey-studios/tessel-editor/pull/44#pullrequestreview-5855208747) do CodeRabbit
sobre `1b92c9c`: 8 comentários inline e um achado só no corpo da review (o segundo comentário da linha 635 de `docs/backlog.md`,
sem thread própria), **9 achados, todos conferidos contra o código e válidos**; nenhum foi recusado. Commits: `081353e` (código:
`harness.py`, `mutate.py`, o docstring de `ab.py`, a spec e o log do controle negativo) e o de docs (esta página, o README dos
runners, o backlog, o README da evidência, a linha de F055 em `docs/features/README.md` e
[`gates-081353e.txt`](../migration/gizmo-perf-evidence/gates-081353e.txt)). Nenhum arquivo Rust, `Cargo.lock`, `budgets.json`, jornada,
corpus ou golden mudou. O aviso genérico "Docstring Coverage" dos pré-merge checks **não foi tratado** (BL015.23 (13d)).

| # | Achado | O que mudou | Prova |
|---|---|---|---|
| F1 | [`docs/backlog.md`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535075152): o contador determinístico "fecha a lacuna sem relógio" (BL015.23 (6) e (7)) | o contador conta **trabalho** (nós visitados, multiplicações de matriz), não **tempo**: um layout que fique mais lento com as mesmas contagens passa nele. BL015.23 (6) e (7), "Tempo" desta página e a linha da rodada 3 que repetia a frase agora dizem que ele é o **único** detector sem relógio **previsto**, que estreita a lacuna e **não a fecha**, e que o limite dele fica **separado** do limite do A/B. O contador continua sem existir (M2(b) segue aberto) | só texto |
| F2 | (sem thread: segundo comentário da linha 635 de `docs/backlog.md`, no corpo da review) o requisito de A/B para uma mudança só no host (`scene_monitor_host.rs`) | `ab.py` só roda `cargo test -p tessel-editor-core --test scene_gizmo_perf_budget` e só conta como diferença o que `is_measured` vê: `crates/core/{src,build.rs,Cargo.toml}` e as entradas de build do workspace (`Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo`, `vendor`). A review pede uma medição do host **ou** restringir o requisito ao que o gate mede: **feita a segunda** (BL015.23 (6), "O que falta para validar" e "Tempo"); a primeira **não** foi feita e está registrada como o item novo **(14)** de BL015.23, **sem desenho**. O A/B **não mediria** a mudança de `scene_monitor_host.rs` desta própria PR | só texto; lido em `ab.py` (`is_measured`, `is_prod`, o comando de teste) |
| F3 | [`docs/features/README.md`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535075162): linha de F055 | a linha diz o que o A/B mede (o core e as entradas de build), que uma mudança só no host ou no renderer **exige validação própria** (hoje o teste estrutural do host, executado, e `native.py`, nunca executado) e vira `DIFFERENCE_OUTSIDE_THE_MEASUREMENT`, e que um A/B positivo do core **não valida** o host nem o renderer quando os dois mudam. Mesmo texto corrigido na linha "próximo" de BL015.21, em BL015.23 (6), no bullet do A/B e em "O que falta" desta página e na linha M2 do README da evidência | só texto |
| F4 | [`docs/features/scene-gizmo-performance.md`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535075169): receita "A/B contra a `main` de agora" | a sequência é **uma** cadeia `&&` (`git fetch origin main && BASE=$(git rev-parse --verify FETCH_HEAD) && ...`): o SHA sai do `FETCH_HEAD` logo depois do `fetch` que deu certo, nada depende de `origin/main` (que um `remote.origin.fetch` sem `main` deixa ausente ou velho) e um `fetch` que falha interrompe a sequência; o `git log -1` continua como conferência; as frases "`origin/main` é o que o clone local tem" e "com `origin/main` nas duas fases" foram reescritas | só texto; **não executado** (a spec não cobre as flags, BL015.23 (13c)) |
| F5 | [mesmo arquivo](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535075177): "Contra outro commit, o build mede o que difere" | `ab.py` mede o core e as entradas de build, **não** `crates/desktop/src`: as diferenças do desktop vão para `product_files_outside_the_measurement` e **não entram no número**; um "sem regressão" não diz nada sobre elas, **mesmo quando o core também difere** (o veredito é o da parte do core). Reescrito aqui e nas cópias (BL015.23 (6), linha de F055, o docstring de `ab.py`) | só texto; o docstring de `ab.py` é comentário |
| F6 | [mesmo arquivo](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535075185): blocos de comando com um `--out DIR` para todos os runners | os runners escrevem arquivos de mesmo nome (`exes/`, `manifest.json`, `manifest.build.json`, `build.json`, `report.json`) e o `prepare` do `mutate.py` esvazia `--out/exes`: com um `DIR` comum o A/B sobrescreveria o manifesto do `run.py`, e o `prepare` sobrescreveria o do A/B e apagaria os executáveis de um A/B BLOCKED. Os blocos usam **um diretório por runner** (`OUT=...`; `--out "$OUT/core"`, `"$OUT/ab"`, `"$OUT/mutants"`, `"$OUT/native"`), o **mesmo** em todas as fases de um runner; a regra está também no README dos runners. **Só documentação: nenhum código impede dois runners diferentes de dividir uma `--out`** (só o `mutate.py` trava a própria, F8) | só texto |
| F7 | [`docs/migration/gizmo-perf-evidence/README.md`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535075194): "HEAD da PR" nas linhas das rodadas 2 e 3 | virou `f3b2c32` (rodada 2, 107 conferências, mantidas) e `1b92c9c` (rodada 3, 111, mantidas), conferidos no `git log` (pais `c3ae380` e `e37a76f`); as linhas da rodada 4 dizem "este commit (o que contém esta linha)", nunca "HEAD da PR". Mesma classe: as frases "as threads não foram respondidas nem resolvidas", verdadeiras na entrega das rodadas 2 e 3, passaram a dizer isso ("na entrega da rodada N") e a apontar a review [5855060347](https://github.com/journey-studios/tessel-editor/pull/44#pullrequestreview-5855060347) do coordenador e as quatro threads que o próprio bot resolveu | só texto; autoria e estado das reviews e threads lidos com `gh api` (somente leitura) |
| F8 | [`scripts/gizmo-perf/mutate.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535075200): `prepare` e `time` podiam rodar juntos na mesma `--out` | **código** (`081353e`): `harness.exclusive(path)` (novo, `flock` exclusivo e não bloqueante) em `--out/.mutate.lock`, tomado pelas **duas** fases **antes** do lock de performance e de ler ou escrever qualquer coisa da `--out`; quem encontra a `--out` de outra execução recusa (`REFUSED`, exit 1) sem esperar e sem tocar em nada. A trava acaba com o processo (SIGKILL incluído) e o arquivo fica (apagá-lo abriria uma corrida de inode). `harness.Lock`, o lock de performance, não mudou | 4 testes novos (`MutantExecutables`), todos **falham** nos runners de `1b92c9c` (5 falhas: o (d) falha nos dois subcasos; nenhum teste anterior falha), 7 variantes erradas da correção (cada uma derruba o teste esperado) e a verificação com dois processos reais, em [`spec-new-tests-vs-1b92c9c-runners.log`](../migration/gizmo-perf-evidence/spec-new-tests-vs-1b92c9c-runners.log) |
| F9 | [`scripts/gizmo-perf/README.md`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535075211): linha de `run.py` (`--allow-dirty`) | `--allow-dirty` reescreve o `PASS` para `DIRTY_RUN_NOT_A_PASS_FOR_HEAD` **só numa execução de fato suja**: `dirty_run` vem da árvore (`run.py`:139) ou, sem `--exe`, do build registrado sujo ou sem o campo `dirty` (:182), e a troca do `PASS` só acontece se `dirty_run` (:223). Com árvore **e** build registrado limpos a flag não muda nada e o `PASS` continua `PASS`; o exit code continua 0; com `--exe` o `build.json` não é lido e só a árvore decide. Cópias: README dos runners, a linha da rodada 3 desta página, BL015.23 (13a) e a linha do achado 5 da review anterior no README da evidência | só texto; lido em `run.py` |

Mesma classe de erro, varrida nesta rodada em `scene-gizmo-performance.md`, `docs/backlog.md`, `docs/features/README.md`, o README da
evidência, o README dos runners, os docstrings de `scripts/gizmo-perf/*.py` e os de `crates/core/tests/scene_gizmo_perf_*.rs`:
"o contador fecha a lacuna sem relógio" (backlog (6) e (7), esta página em "Tempo" e na tabela da rodada 3, README da evidência), "toda PR
que toque `scene_plan/**` ou o caminho de renderização precisa do A/B" (backlog (6) e a linha "próximo" de BL015.21, README de
features, "O que falta", README da evidência), "o build mede o que difere" (esta página, `ab.py`), "HEAD da PR" (README da evidência), "as
threads não foram respondidas" (README da evidência, duas vezes) e `--allow-dirty` (README dos runners, esta página, backlog (13a),
README da evidência). Nos docstrings Rust e Python só o de `ab.py` tratava do assunto e foi estendido no `081353e`.

**O que esta rodada fez de verdade** (árvore limpa de `081353e`, cada passo sob timeout e o `test --workspace` sob o lock de GPU; comando,
exit, carga, disco e árvore de cada passo em [`gates-081353e.txt`](../migration/gizmo-perf-evidence/gates-081353e.txt)): `fmt --check`, `check
--workspace`, `clippy --workspace --all-targets -D warnings`, `build -p tessel-editor`, `./scripts/runtime setup`, `test --workspace
--no-fail-fast` com `TESSEL_REQUIRE_RUNTIME=1` (**2299 passaram, 0 falharam, 16 ignorados**; estrutural 7/7, threads 1/1, host
`scene_gizmo_perf` 5/5 dentro dos 418 do crate `tessel-editor`; os 3 testes de tempo de `scene_gizmo_perf_budget` seguem `ignored`
de propósito), `cargo tree -p tessel-editor-core` sem gpui/wry, a spec 115/115, `corpus.py --check` e `git diff --check` contra
`1b92c9c` e contra `cbe9e1b`, todos exit 0.

**O que esta rodada NÃO fez:** medir tempo; rodar `run.py`, `ab.py`, `mutate.py` (`prepare` ou `time`: a trava só foi exercitada pela
spec, com dublês, e por dois processos reais que nunca passam dela) ou `native.py`; build release de runner; abrir a janela nativa; mudar
`budgets.json`, jornada, corpus, golden, margens, limiares ou qualquer Rust; desenhar a comparação baseline × candidato do host e
da janela (BL015.23 (14)); tratar o aviso "Docstring Coverage". A máquina estava carregada por outros projetos (a carga de cada passo
está em `gates-081353e.txt`). **Nenhuma afirmação de desempenho vale: implementado não é validado.**

## Estado desta entrega e evidência

Arquivos e identidade de cada execução: [gizmo-perf-evidence/README.md](../migration/gizmo-perf-evidence/README.md).
Regra de leitura: **implementado** (código e testes que rodam) ≠ **validado** (números/provas com identidade).

| Item | Estado |
|---|---|
| Spec dos runners Python (`gizmo-perf.spec.py`, 115 testes: os 76 anteriores + 31 da rodada 2 + 4 da rodada 3 + 4 da rodada 4; harness, veredito do runner/mutantes, baseline com repositórios temporários, A/B, nativo com janela falsa e `drag` com monitor roteirizado, proveniência, auditoria, os executáveis dos mutantes de tempo entre `prepare` e `time` e a posse exclusiva de `--out` pelo `mutate.py`) | **executado e passa (115/115)** na árvore **limpa** de `081353e` ([`gates-081353e.txt`](../migration/gizmo-perf-evidence/gates-081353e.txt)); os 4 testes da rodada 4 **falham** nos runners de `1b92c9c` (5 falhas: o (d) falha nos dois subcasos; nenhum teste anterior falha), e o log traz a matriz de variantes erradas da correção e a verificação com dois processos reais ([`spec-new-tests-vs-1b92c9c-runners.log`](../migration/gizmo-perf-evidence/spec-new-tests-vs-1b92c9c-runners.log)); dos 4 testes da rodada 3, 3 falham nos runners de `f3b2c32` (o 4º passa dos dois lados de propósito); na rodada 2 foram 107/107 em `c3ae380`, com 28 falhando ou dando erro nos runners de `9a72e03`. `corpus.py --check` passa (exit 0) e o `git diff --check` contra `1b92c9c` e contra `cbe9e1b` dá exit 0 em `081353e` (o `git diff --check 9a72e03 HEAD` **tinha falhado em `c3ae380`**, linha em branco no fim de `bl01522-host-scratch-with-fix.log`, corrigida no commit de docs da rodada 2); o resultado no commit de docs está no relatório da entrega (sem Cargo, sem janela, sem benchmark) |
| Core: `fmt --check`, `clippy -p tessel-editor-core --all-targets -D warnings`, estrutural (7), threads (1, novo), core inteiro e `cargo tree` | **executados em `a97a0a7`, todos exit 0**: `fmt`; `clippy`; estrutural **7 passaram, 0 falharam, 0 ignorados**; threads **1 passou**; core inteiro (`--no-fail-fast`) **54 binários, 1267 passaram, 0 falharam, 7 ignorados** (3 do orçamento de tempo desta suíte e 4 medições de outras frentes, todos `#[ignore]` de propósito); `cargo tree -p tessel-editor-core --locked`: 16 linhas, **sem gpui/wry** (log e saída completa em [`rust-gates-a97a0a7.txt`](../migration/gizmo-perf-evidence/rust-gates-a97a0a7.txt)). Nada foi medido em tempo. **Em `c3ae380` (árvore limpa) só foram repetidos** o estrutural (7 passaram, 0 falharam) e o `cargo tree` (sem gpui/wry; [`gates-c3ae380.txt`](../migration/gizmo-perf-evidence/gates-c3ae380.txt)): `crates/core/src` e o resto do core não mudaram desde `a97a0a7` (só o comentário do cabeçalho de `scene_gizmo_perf_structure.rs`), e o core inteiro e o teste de threads **não** foram repetidos. **Em `e37a76f` (rodada 3, árvore limpa) foram repetidos** `fmt`, `clippy -p tessel-editor-core --all-targets -D warnings`, o estrutural (7 passaram, 0 falharam), o de threads (1 passou) e o `cargo tree` (sem gpui/wry), todos exit 0 ([`gates-e37a76f.txt`](../migration/gizmo-perf-evidence/gates-e37a76f.txt)); a única mudança Rust desde `c3ae380` é um comentário de `scene_gizmo_perf_budget.rs`; o core inteiro **não** foi repetido naquela rodada. **Em `081353e` (rodada 4, árvore limpa, sob o lock de GPU) o core inteiro rodou dentro do `test --workspace`**: 53 binários de teste e 1 execução de doc-tests, **1267 passaram, 0 falharam, 7 ignorados** (os mesmos números de `a97a0a7`; os 7 são os 3 do orçamento de tempo desta suíte, `scene_attach_cost`, `scene_fx_key_bounds` e os 2 de `scene_plan_performance_bench`, todos `#[ignore]` de propósito), com o estrutural (7) e o de threads (1) dentro; `cargo tree -p tessel-editor-core` sem gpui/wry de novo ([`gates-081353e.txt`](../migration/gizmo-perf-evidence/gates-081353e.txt)) |
| Host GPUI (`cargo test -p tessel-editor --locked scene_gizmo_perf`) | **5 passaram, 0 falharam** em `c3ae380`, árvore limpa (exit 0; 413 filtrados): os 3 de antes, o `showing_a_scene_that_shares_its_bundle_with_the_shown_one_shows_it` (que **falhava em `a97a0a7`**, BL015.22, e passa depois da correção) e o novo `monitor_show_scene_follows_the_request_between_scenes_of_one_bundle_and_of_another`. O crate `tessel-editor` inteiro: **418 passaram, 0 falharam, 0 ignorados** (exit 0), com o do ciclo de vida do monitor; `clippy -p tessel-editor --all-targets -D warnings` exit 0 (a execução real, de 76,6 s, foi no código-fonte do `bab9290` ainda não commitado; em `c3ae380` o resultado veio do cache do mbx, 5 hits e 0 misses, sem recompilar). Em `c3ae380`, `clippy --workspace` e o workspace inteiro **não** foram rodados (rodaram em `081353e`, abaixo). Não repetido na rodada 3: nenhuma fonte do crate mudou desde `c3ae380`. **Em `081353e` (rodada 4, árvore limpa)**: o crate `tessel-editor` inteiro, dentro do `test --workspace` com `TESSEL_REQUIRE_RUNTIME=1`: **418 passaram, 0 falharam, 0 ignorados** (os 5 de `scene_gizmo_perf` entre eles), `clippy --workspace --all-targets -D warnings` exit 0 e `build -p tessel-editor` exit 0. **É só o teste estrutural do host**: nenhuma medição do host nem da janela, e o A/B não os vê (BL015.23 (14)) |
| `within()` do orçamento (não mascara FAIL) e o resto de `scene_gizmo_perf_budget.rs` | **compilados** (`clippy --all-targets`; `cargo test` lista os 3 testes como `ignored`), **não executados**: só rodam com tempo, lock e guardas |
| Mutantes `extra_plan_per_handler` (estrutural), `memo_off` e `replan_per_tick` (host) | mortos em `9186e2a`; o `.tgz` retido mistura manifestos de `9186e2a` e `b898a6e`. **Reexecutar** com o runner novo (verdicts por teste nomeado + mensagem) |
| Tempo no core (release, lock, guardas) | **PASS no `5cded7e`** (árvore limpa), regra de margem fria já ajustada depois de um FAIL (ver histórico); **não repetido** neste head. Um PASS de outro head não vale para este |
| Mutantes de tempo (plano extra, 1 ms) | **não provados** (BLOCKED por carga em `9186e2a`; não repetidos); o runner da rodada 3 (só mede o executável que o `prepare` registrou) nunca rodou fora da spec |
| A/B `cbe9e1b` × candidato | o runner novo **não foi executado** (BLOCKED: disco/carga); o A/B retido (`037eda5`) é o **antigo, que era A/A**: **sem comparação**; nada de "sem regressão" é afirmado |
| Janela nativa | só a corrida funcional antiga (`small`, 160 verificações, runner anterior, revisão anterior; o manifesto do `.tgz` diz BLOCKED e não casa com o relatório: ver o README da evidência); o runner novo (`project.open` real, quadro novo, cancelar, `dense`/`deep`) e a jornada atual (com `same.*`) **nunca rodaram** na janela |
| Workspace inteiro com `TESSEL_REQUIRE_RUNTIME=1` | **executado em `081353e` (rodada 4), árvore limpa, sob o lock de GPU: exit 0, 500 s, 2299 passaram, 0 falharam, 16 ignorados** em 117 linhas de resultado (113 binários de teste e 4 execuções de doc-tests): `tessel-editor` 418/0/0, áudio 10/0/0, core 1267/0/7, mídia 497/0/6, render 107/0/3. O `the_journey_leaves_the_transform…`, que falhou em `5cded7e` (2294/1/16: ordem das chaves JSON com `preserve_order` unificado no workspace, corrigido no `9cc2335`), **passa**; os 16 `ignored` são `#[ignore]` de propósito (os 3 de `scene_gizmo_perf_budget` só rodam por `run.py`). **É só o gate funcional: nada foi medido em tempo.** O commit de docs seguinte não muda código (`git diff --stat 081353e HEAD` só lista `docs/**` e o README dos runners) ([`gates-081353e.txt`](../migration/gizmo-perf-evidence/gates-081353e.txt)) |
| Provas de processo e gates funcionais de `0ee2d14` | **pendentes** |

### O que falta para validar (BL015.21)

Feito: em `a97a0a7` o Rust novo do core e do host compilou e rodou; em `c3ae380` o BL015.22 está corrigido e verificado **no
host** (5/5 e o crate inteiro, 418/0/0). Em `e37a76f` (rodada 3) o core (`fmt`, `clippy`, estrutural, threads) e a spec 111/111 foram repetidos. Em `081353e` (rodada 4, árvore limpa) rodaram `fmt`, `check --workspace`,
`clippy --workspace --all-targets -D warnings`, o build de `tessel-editor`, o **workspace inteiro com `TESSEL_REQUIRE_RUNTIME=1`** (2299/0/16) e a spec 115/115: **são gates funcionais**, nada de tempo.
Resta o efeito da correção na **janela nativa** (a jornada atual nunca rodou lá) e tudo o que mede.

Com disco ≥ 16 GiB e carga ≤ 0,5 por CPU (uma máquina que não esteja compilando outra coisa): 1. (feito em `081353e`; só repetir
se o código mudar) o workspace com `TESSEL_REQUIRE_RUNTIME=1` e `clippy --workspace`; 2. `run.py --phase build` e `--phase measure`; 3. `ab.py --phase build` e
`--phase measure` (primeiro A/B de verdade; **exigido** de toda PR que toque o **core** que ele mede, `scene_plan/**` e o resto
de `crates/core/{src,build.rs,Cargo.toml}` e das entradas de build do workspace, porque é a única comparação absoluta com uma
baseline, e só vê o que passa dos limiares sobre o total da requisição: ver "Tempo"; **ele não mede o host, o renderer nem a
WebView, nem a mudança de `scene_monitor_host.rs` desta PR**: para isso falta uma comparação baseline × candidato do host e da
janela, que não existe e cujo desenho não foi feito, BL015.23 (14); contra a `main` de agora: ver "A/B contra a `main` de
agora"); 4. `mutate.py --phase prepare` e `--phase time`; 5. `native.py
--phase build` e `--phase run` nos três casos; 6. um caso com splat (BL015.23); 7. orçamentos nativos só depois de um A/B com
um binário nativo da `main`; 8. as provas de `0ee2d14`; 9. uma comparação baseline × candidato do host, do renderer e da janela, que o A/B não faz (BL015.23 (14); sem desenho). **Tempo, A/B, mutantes de tempo e janela nativa seguem sem execução
neste head: nenhuma afirmação de desempenho vale.**
