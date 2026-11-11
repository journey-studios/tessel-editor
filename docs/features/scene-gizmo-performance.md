# F055 (desempenho) — Suíte de regressão de performance dos gizmos e da abertura (BL015.21)

Complementa [F055](scene-gizmo.md). Pedido: "garantir que não perdemos performance e que validamos o mesmo
cenário de abrir o app visualmente; o que for possível por teste unitário". O código de produção tocado é o contador de
layouts (`scene_plan::probe` + a chamada em `ScenePlan::build_with`, ver abaixo) e, desde a rodada 2 de correções, **uma
mudança de comportamento do monitor** (`scene_monitor_host.rs`): o id da cena entra na chave de deduplicação de `ViewportHost::render_scene`
(BL015.22, um defeito de produto anterior à PR que a própria jornada encontrou; ver "Achados e pendências"). O custo do
contador **não é assumido nulo**: só o A/B o mede (e ele rodou duas vezes em `12bc8da` e terminou `INVALID_GUARD_AFTER_RUN` nas duas: **nenhum custo foi medido**); a mudança do monitor, em `crates/desktop`,
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
| Core (lib) | `crates/core/tests/gizmo_perf_support` + `scene_gizmo_perf_structure.rs` (7 testes, **sempre** rodam, determinísticos), `scene_gizmo_perf_threads.rs` (1 teste: nenhum layout escapa da thread lida; **executado e passa em `a97a0a7`**) e `scene_gizmo_perf_budget.rs` (`#[ignore]`, opt-in controlado; **compilado** pelo `clippy --all-targets` e pelo `cargo test`, onde seus 3 testes aparecem como `ignored`; **executado uma vez em `12bc8da` por `run.py`: `FAIL`**, ver "Rodada 5: primeira medição em `12bc8da`") | transformada/undo/redo idênticos ao golden (`journey.expected.json`), cancelar não muda nada, desfazer restaura o valor exato; **planos por requisição** (`scene_plan::probe::layouts_built`), independência do tamanho da cena, 60 moves = 60 plans, chave da memória do overlay; tempo em release | janela, GPU, página |
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
cenário, configuração e máquina, e nenhum teste permanente fecha o que ele não vê (o A/B foi executado duas vezes em `12bc8da` e as duas terminaram `INVALID_GUARD_AFTER_RUN`: **nada foi julgado**, ver "Rodada 5").
**O A/B não vê o host, o renderer nem a WebView.** Uma PR que mude só `crates/desktop` ou `crates/render` recebe
`DIFFERENCE_OUTSIDE_THE_MEASUREMENT` (exit 4; os arquivos ficam em `product_files_outside_the_measurement`), nunca um "sem
regressão" (o JS do `runtime/` nem entra na lista de produto do runner, `is_prod`: uma PR só nele recebe
`A_A_NO_PRODUCT_DIFFERENCE`; mas uma edição local nele suja a identidade da execução desde a rodada 8, porque
`native.py` roda a página: ver "Rodada 8 de correções"); uma PR que mude o core **e** o desktop recebe o veredito da parte do core, e o
`NO_REGRESSION_IN_SCENARIO` dela não diz nada sobre a parte do desktop. Vale para esta própria PR: a mudança de comportamento
do monitor (`scene_monitor_host.rs`, BL015.22) cairia em `product_files_outside_the_measurement` e o A/B **não a mediria**. O
host, o renderer e a WebView precisam de **validação própria**: hoje o teste estrutural do host (`scene_gizmo_perf_tests.rs`,
**executado**) e o `native.py` (**executado uma vez em `12bc8da`: `FAIL` funcional**, sem baseline); **não existe comparação baseline × candidato para o host
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
python3 scripts/gizmo-perf/run.py --phase build --out "$OUT/core"       # release do teste de tempo (sem lock de performance; toma o lock do checkout)
python3 scripts/gizmo-perf/run.py --phase measure --out "$OUT/core"     # tempo no core (lock, guardas, manifesto; não constrói)
python3 scripts/gizmo-perf/ab.py --phase build --out "$OUT/ab"          # baseline fixada cbe9e1b + candidato (sem lock de performance; toma o lock do checkout)
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
do A/B e **apagaria** `--out/exes` (os executáveis de um A/B BLOCKED, que o `measure` repetiria). Nenhum runner confere que a
`--out` é a certa para ele: isso segue sendo de quem roda. O que as **rodadas 6 e 8** acrescentaram é que duas execuções **ao mesmo tempo**, de **qualquer** runner e fase, não dividem uma `--out` nem um
checkout: todo runner e toda fase tomam um `flock` exclusivo e não bloqueante em **um** arquivo, `--out/.gizmo-perf.lock` (`harness.OUT_LOCK`), logo
depois de criar a `--out` (até a rodada 7 cada runner tinha o seu, `.run.lock`, `.ab.lock`, `.native.lock` e `.mutate.lock`, e dois runners
**diferentes** na mesma `--out` não se excluíam), e toda fase que **constrói a partir do checkout ou o edita** (`--phase build` de `run.py`, `ab.py`
e `native.py`, e `mutate.py --phase prepare`) toma antes o lock do checkout, um arquivo `/tmp/tessel-gizmo-perf-checkout-<hash do caminho>.lock`
fora do repositório. Quem não consegue a trava sai com **uma** linha `REFUSED` (exit 1), sem esperar e sem escrever nada, e a recusa não nomeia só o
runner recusado: diz que **qualquer** runner pode ser o dono. **A trava exclui execuções simultâneas, não sequenciais**: ela acaba com o processo
(o arquivo fica, de propósito: apagá-lo abriria uma corrida de inode), e um runner que começa depois que outro terminou ainda sobrescreve o
`manifest.json` e o `exes/` dele. É por isso que a regra "um diretório por runner" continua: a trava garante que as execuções não são
simultâneas, não que a `--out` seja a de cada runner.

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
    `directed_scene()`; isso foi lido no código, nunca executado). Na janela real **existe uma corrida da jornada atual**, com `same.*`
    (`12bc8da`, rodada 5): os 15 checks `same.*`/`switch.*` de cada corpo passaram e `same.show` mostrou `proof-c` com um quadro novo, mas a corrida
    inteira é `FAIL` por outros motivos e **não há controle com a chave antiga** na janela. O que a página desenha não é observado pelo host de teste (sem WebView).
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
nativa **não tinha execução então** (estado de `a97a0a7`; a primeira execução é a da rodada 5, em `12bc8da`: veja "Rodada 5: primeira medição em `12bc8da`"):

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
| F6 | [mesmo arquivo](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535075185): blocos de comando com um `--out DIR` para todos os runners | os runners escrevem arquivos de mesmo nome (`exes/`, `manifest.json`, `manifest.build.json`, `build.json`, `report.json`) e o `prepare` do `mutate.py` esvazia `--out/exes`: com um `DIR` comum o A/B sobrescreveria o manifesto do `run.py`, e o `prepare` sobrescreveria o do A/B e apagaria os executáveis de um A/B BLOCKED. Os blocos usam **um diretório por runner** (`OUT=...`; `--out "$OUT/core"`, `"$OUT/ab"`, `"$OUT/mutants"`, `"$OUT/native"`), o **mesmo** em todas as fases de um runner; a regra está também no README dos runners. **Só documentação: nenhum código impede dois runners diferentes de dividir uma `--out`** (só o `mutate.py` trava a própria, F8). **Atualizado na rodada 6**: hoje todo runner trava a sua `--out` e toda fase que constrói ou edita trava o checkout (ver "Rodada 6 de correções"); que a `--out` seja a certa para o runner segue sendo de quem roda. **Atualizado na rodada 8** (R3): a trava de `--out` é **uma só** para os quatro runners (`.gizmo-perf.lock`), então dois runners **diferentes** também não dividem uma `--out` **ao mesmo tempo**; a trava não exclui execuções sequenciais, e a regra de um diretório por runner continua | só texto |
| F7 | [`docs/migration/gizmo-perf-evidence/README.md`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535075194): "HEAD da PR" nas linhas das rodadas 2 e 3 | virou `f3b2c32` (rodada 2, 107 conferências, mantidas) e `1b92c9c` (rodada 3, 111, mantidas), conferidos no `git log` (pais `c3ae380` e `e37a76f`); as linhas da rodada 4 dizem "este commit (o que contém esta linha)", nunca "HEAD da PR". Mesma classe: as frases "as threads não foram respondidas nem resolvidas", verdadeiras na entrega das rodadas 2 e 3, passaram a dizer isso ("na entrega da rodada N") e a apontar a review [5855060347](https://github.com/journey-studios/tessel-editor/pull/44#pullrequestreview-5855060347) do coordenador e as quatro threads que o próprio bot resolveu | só texto; autoria e estado das reviews e threads lidos com `gh api` (somente leitura) |
| F8 | [`scripts/gizmo-perf/mutate.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535075200): `prepare` e `time` podiam rodar juntos na mesma `--out` | **código** (`081353e`): `harness.exclusive(path)` (novo, `flock` exclusivo e não bloqueante) em `--out/.mutate.lock` (desde a rodada 8, o arquivo comum `--out/.gizmo-perf.lock`), tomado pelas **duas** fases **antes** do lock de performance e de ler ou escrever qualquer coisa da `--out`; quem encontra a `--out` de outra execução recusa (`REFUSED`, exit 1) sem esperar e sem tocar em nada. A trava acaba com o processo (SIGKILL incluído) e o arquivo fica (apagá-lo abriria uma corrida de inode). `harness.Lock`, o lock de performance, não mudou | 4 testes novos (`MutantExecutables`), todos **falham** nos runners de `1b92c9c` (5 falhas: o (d) falha nos dois subcasos; nenhum teste anterior falha), 7 variantes erradas da correção (cada uma derruba o teste esperado) e a verificação com dois processos reais, em [`spec-new-tests-vs-1b92c9c-runners.log`](../migration/gizmo-perf-evidence/spec-new-tests-vs-1b92c9c-runners.log) |
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

## Rodada 5: primeira medição em `12bc8da`

Head `12bc8da6fa564b3d740fa51da29dd203e63392ae` (árvore `85cfb1038626dd68c2ef2252e14e5d276f0596ac`, base da PR
`cbe9e1b393861febf073633fbbaa0da9857780b6`). Esta rodada **só mediu**: nenhum código, spec, fixture, `budgets.json`, margem, limiar ou rodada
mudou: nada foi editado no worktree durante a medição; o `git status --porcelain` está registrado vazio no preflight, depois de cada `prepare` do `mutate.py` e antes da coleta, e os manifestos retidos trazem `dirty: ''`. Arquivos, manifestos, comandos, horário UTC, carga e disco de cada passo:
[`measure-12bc8da/README.md`](../migration/gizmo-perf-evidence/measure-12bc8da/README.md) (e a tabela "Medição em `12bc8da` (rodada 5)" do
[README da evidência](../migration/gizmo-perf-evidence/README.md)). Os estados ficam com o nome que o runner escreveu; **implementado não é
validado**, e nada nesta seção diz "validado", "sem regressão" ou "desempenho OK".

**Contexto da máquina.** Quatro processos `Google Chrome Helper` do usuário a ~100% de CPU cada (o quarto desde ~08:41Z), mais `WindowServer` e
`OrbStack`, durante toda a sessão. Nas sondagens somente-leitura da guarda a mediana da média de 1 min foi 5,50 e a de 5 min 5,67, contra o limite
5,5 (0,5 × 11 CPUs): só 18 de 93 sondagens passaram nas duas médias. Toda corrida de ≥ 1 CPU por dezenas de segundos levou a média de 1 min a
5,6–6,5, de modo que a guarda **depois** da corrida recusou em 5 das 8 execuções guardadas e só segurou nas 3 mais curtas (os mutantes de tempo, de 10 a 19 s): é o item (11) de BL015.23, agora com números.
Os estados julgados (`FAIL` do `run.py` e do `native.py`) rodaram **uma vez** e não foram repetidos; o que o runner marcou como não julgado
(`INVALID_GUARD_AFTER_RUN`, `BLOCKED_OR_INFRA`) ou que morreu por infraestrutura foi repetido como o protocolo permite (as tentativas estão abaixo).

| Runner | Estado (como o runner escreveu) | Números-chave | O que NÃO mostra |
|---|---|---|---|
| `run.py` (core, release) | **FAIL**, exit 1 | os 2 controles negativos passaram; o teste de orçamento falhou com **6 p95 acima do orçamento, todos no `deep`**: `down.rotate` 6,10× (orçamento 5,4), `down.translate` 3,26× (3,15), `move.rotate` 10,96× (9,05), `read.rotate` 5,98× (4,15), `state.rotate` 5,30× (3,95), `up.rotate` 17,41× (10,05); `small` e `dense` dentro dos orçamentos; 0 ticks longos | não separa ruído de máquina carregada de regressão (BL015.23 (20)); o PASS do `5cded7e` é de outro head e de outra guarda |
| `ab.py` (`cbe9e1b` × candidato) | build: 1 crash de infraestrutura e 1 retry `BUILT`; measure: **`INVALID_GUARD_AFTER_RUN`** (exit 3) **nas duas tentativas** | executáveis distintos (sha256 `9f81ee1f…` × `f8002313…`); `product_files_that_differ` = só `scene_plan/build.rs`; 1 min antes → depois 5,07 → 6,49 e 5,08 → 5,99 | **nada foi julgado**; as linhas por tipo (56+4 e 59+1 `REGRESSED`, nenhum tipo se repete) são observação; não vê host, renderer nem WebView |
| `mutate.py` | `prepare` `PREPARED` ×2; `time`: tentativa 1 **não invocada** (nenhuma janela limpa em 25 min), tentativa 2 `BLOCKED_OR_INFRA`, retry (o último) **`UNEXPECTED`** | `extra_plan_per_handler`: estrutural KILLED e tempo KILLED (no retry); `delay_1ms_per_handler`: estrutural SURVIVED (esperado), tempo KILLED; `replan_per_tick`: host KILLED; `memo_off`: host **UNEXPECTED_FAILURE** | detecta mutantes **sintéticos**; o `memo_off` foi detectado pelo teste mas o runner não o classifica como morto (BL015.23 (19)); o kill de `extra_plan` em tempo vem de muitos tipos acima do orçamento ao mesmo tempo (as 24 linhas guardadas excedem de 1,01× a 2,13×), não de folga grande |
| `native.py` (janela real) | **FAIL**, exit 1 | 639 verificações, 20 falharam (`small` 5, `dense` 10, `deep` 5), todas em `drag.*`: 15× "the committed picture is a NEW picture" e 5× pixels iguais à base em `dense/scale.*`; `same.*`/`switch.*` 15 de 15 por corpus | **sem baseline** para a janela (BL015.23 (14)): só as verificações funcionais e os tempos brutos com a carga ao lado |

- **`run.py` (R2)**: `FAIL` é final para este head e **não foi repetido**. Referências (p50 do plano) 9,375 µs, 635,084 µs e 47,209 µs; blocos de referência instáveis
  4,25%, 6,95% e 10,81% (limite 25%: o `deep` foi julgado). O build (R1) levou 1,63 s por restauração do cache do mbx (25 hits).
- **`ab.py`**: a tentativa 1 do build morreu com `FileNotFoundError` em `ab.py` linha 434 (o executável do candidato sumiu do `target/` depois do build da baseline; sem
  manifesto; BL015.23 (15)); o retry, com **só o ambiente** mudado (`MBX_GC_MAX_TOTAL_SIZE=24GiB`), construiu em 2 s com cache quente, e **não se sabe** se foi a variável ou o
  cache. `CARGO_TARGET_DIR` e `TESSEL_CARGO_LANE` estavam não definidos. O `product_files_outside_the_measurement` lista **quatro** arquivos do desktop
  (`mod.rs`, `scene_gizmo_perf_tests.rs`, `scene_lifecycle_tests.rs`, `scene_monitor_host.rs`), não dois. As duas tentativas de measure, 77 s cada, partiram de uma guarda limpa
  e a própria corrida somou +1,42 e +0,91 à média de 1 min. Uma terceira tentativa **não foi invocada** (a conta fecha: precisaria partir de ≲ 4,6, nunca visto; mínimo das 93 sondagens 4,84).
- **`mutate.py`**: o runner deixou `KILLED` só com o teste nomeado e a mensagem esperada. Estrutural: `extra_plan` morto (`small / open.first_overlay: scene.gizmo.state laid the scene
  out 2 times (budget 1)`). Tempo (retry, guarda depois limpa): `delay_1ms` com 106 linhas de orçamento (p.ex. `small/cold.state: p50 is 111.61x the plan (budget 3.15)`) e
  `extra_plan` com 68 (p.ex. `small/cold.state: p50 is 3.52x the plan (budget 3.15)`, `small/down.rotate: p95 is 26.01x the plan (budget 12.2)`). Host: `replan_per_tick`
  morto (`small: 100 idle ticks laid the scene out`); `memo_off` **falhou o teste nomeado em `scene_gizmo_perf_tests.rs:193:9`** (a verificação de contagem de layouts), que não é
  nenhuma das duas mensagens do `EXPECT` do runner, por isso `UNEXPECTED_FAILURE`, de forma reproduzível nas duas `prepare` (a mensagem exata é inferida do local do panic e do fonte: o log bruto
  não é guardado). Depois de uma tentativa de `time` que termina `INVALID_GUARD_AFTER_RUN`/`BLOCKED_OR_INFRA` o runner **apagou `exes/`**, e foi preciso `prepare` de novo (um só retry).
- **`native.py`**: o runner exige, depois de `scene.pointer.up`, um quadro com `request` maior que o do candidato desenhado antes da soltura (`native.py` linhas 674 a 681); nos 15
  arrastos que confirmam o monitor manteve o quadro do candidato (mesmo `request` e hash). Se é o produto (o host não redesenha um commit idêntico ao candidato, **hipótese não
  testada**) ou a expectativa do runner, **não está decidido** (BL015.23 (16)). Em `dense`, escala não muda um pixel (hash `a5cee15db5c0b400` na base, no candidato e no commit; (17)). (**Rodada 7:** o código e os registros explicam a soltura sem apontar defeito do produto, e a escala do `dense` tem uma explicação por simulação; as expectativas do runner foram corrigidas, ver "Rodada 7 de correções".)
  `overlay_drawn` tem 4 entradas por corpus, nenhuma nula, mas o runner **não as checa** e a de `small/state.scale.local` é o overlay anterior (`rotate`/`world`; (18)).
  `cleanup_errors` vazio e `session_error` nulo nos três corpos.
- **BL015.22 na janela real**: os 15 checks `same.*`/`switch.*` de cada corpus passaram e `same.show` mostrou `proof-c` com um quadro novo: o **head corrigido** se comporta
  como esperado na janela; **não há controle com a chave antiga na janela** e a corrida inteira é `FAIL`.
- **Tempos brutos do cliente na janela** (ms, sem baseline, sem comparação, com a carga acima): `scene.pointer.move` p50 1,5 / **21,5** / 4,6 e p95 5,7 / **35,4** / 6,0 em
  `small` / `dense` / `deep`. Em `dense` o p50 da janela passa do quadro de 60 Hz (16,6 ms) enquanto o core sozinho fica em ~3,8 ms: isso é uma **observação** do caminho
  host/IPC/WebView, que o A/B não vê (BL015.23 (14)); não é afirmação de regressão.

**Achados novos (todos em `docs/backlog.md`, BL015.23)**: (15) `ab.py --phase build` copia o executável do candidato só depois do build da baseline; (16) `native.py`: 15/15 "NEW picture"
depois do commit; (17) `native.py` `dense`: escala sem efeito em pixels; (18) `native.py`: `overlay_drawn` gravado e nunca checado, com uma entrada velha passando; (19) `mutate.py`:
`EXPECT` do `memo_off` não cobre a primeira verificação que falha; (20) `run.py` FAIL do `deep` sem decisão ruído × regressão; (21) `ab.py --phase measure` apaga os dados da tentativa anterior; (22) `run.py`: `tests_passed` lista também o teste que falhou. Evidência
nova também em (11) (guarda depois da corrida), (13)(b) (`exes/` apagado, observado) e (14) (tempos da janela `dense`). **Rodada 6:** os runners foram corrigidos
para (13)(b), (15), (18), (19), (21) e (22), sem reexecução (ver "Rodada 6 de correções"); os itens (23) a (25) são de lá. **Rodada 7:** os itens (16) e (17) tiveram o runner corrigido, sem reexecução (ver "Rodada 7 de correções"); os itens (26) a (28) são de lá.

**O que esta rodada NÃO afirma:** que o core "não perdeu desempenho" (o `run.py` deu `FAIL` e o A/B não julgou); que o `FAIL` do `deep` seja regressão nem que seja só ruído; que o monitor deixe de desenhar um
commit; que a escala do `dense` seja invisível por defeito do produto; qualquer coisa sobre o host, o renderer e a WebView em A/B; que o `memo_off` seja "sobrevivente". **Continua aberto**: um A/B
**julgado** (precisa de uma máquina em que a guarda depois da corrida segure, ou de uma guarda que descontasse a carga de terceiros; BL015.23 (11)), a decisão sobre o `FAIL` do `deep`, as falhas funcionais
do `native.py`, a correção da classificação do `memo_off`, o caso com splat e a comparação baseline × candidato do host e da janela. (**Rodada 6:** a classificação do
`memo_off` foi corrigida no runner, sem reexecução; o resto continua aberto. **Rodada 7:** as expectativas do `native.py` dos itens (16) e (17) foram corrigidas, sem reexecução.)

## Rodada 6 de correções (PR #44, de `d294bdc` a esta entrega)

Escopo: **só os runners Python, a spec e os docs** (`scripts/gizmo-perf/**`, `docs/**`). Nenhum Rust, runtime, fixture, `budgets.json`,
margem ou limiar mudou, e **nada foi medido**: nenhum `run.py --phase measure`, `ab.py --phase measure`, `native.py --phase run` nem
`mutate.py --phase time` rodou, nenhum build release foi feito, a janela nativa não abriu e
[`measure-12bc8da/`](../migration/gizmo-perf-evidence/measure-12bc8da/README.md) não foi editado. Os defeitos vêm dos 14 comentários da
[review 5856147194](https://github.com/journey-studios/tessel-editor/pull/44#pullrequestreview-5856147194) (F1 a F14) e da primeira
medição real (rodada 5): os itens (15), (18), (19), (21) e (22) de BL015.23 são F2 (ampliado), F13, F15, F16 e F17, e F18 é o texto da
guarda no motivo de um `FAIL` (contexto do item (20), que **segue aberto**). Código, spec e log de controle: `9b5f9fd`. **Implementado
não é validado**: nada nesta seção diz que o desempenho foi validado nem que não há regressão, e nenhum runner corrigido foi
reexecutado contra uma medição.

| # | Origem | O que o runner fazia | O que faz agora | Testes que falham nos runners de `d294bdc` |
|---|---|---|---|---|
| F1 | [`harness.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535825919) | `out()` ignorava o código de saída: um `git` que falha dava head `''`, árvore `''` e `dirty ''` (= limpo), e duas identidades assim eram "o mesmo HEAD" | `git_strict` (`(ok, texto)`) e `git_identity()` estrito, com `unavailable` e o motivo no campo que falhou; `run.py`, `ab.py`, `native.py` e `mutate.py` recusam uma identidade assim **também com `--allow-dirty`**; `same_head` nunca iguala duas identidades inutilizáveis. `out()` ficou tolerante (telemetria) | `UnavailableGit` (9), `Provenance` (1), `MutantExecutables` (2) |
| F2 | [`ab.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535825908), item (15) | os dois executáveis eram copiados só depois dos dois builds (o `mbx` podou o do candidato durante o build da baseline: `FileNotFoundError` sem manifesto); mesmo arquivo ou mesmo sha256 não eram vistos; `CARGO_TARGET_DIR`/`TESSEL_CARGO_LANE` do chamador eram aceitos | copia o do candidato logo depois do build dele; caminho sumido é `INFRA_EXECUTABLE_MISSING` (exit 1, com o caminho), mesmo arquivo ou mesmo sha256 é `AB_EXECUTABLES_COLLIDE`, variável de target no ambiente é `REFUSED` (`env_set`) antes de qualquer build; `run.py` e `mutate.py prepare` registram o executável sumido do mesmo modo | `AbBuild` (5), `BuildPhase` (1), `MutantExecutables` (1) |
| F3 | [`ab.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535825895) | `--rounds 0` ou negativo só rodava os aquecimentos: `judge()` devolvia `{}` e `overall()` dava `NO_REGRESSION_IN_SCENARIO` (exit 0) quando havia diferença de produto | erro de argumento (exit 2, nada escrito); um lado sem amostra, ou nenhum tipo julgado, é `INCONCLUSIVE` (exit 4) | `RoundsAndSamples` (3), `MeasurePhase` (1) |
| F4 | [`ab.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535825900) | um `--phase build` com a mesma `--out` podia remover `baseline-src` e trocar os executáveis durante uma medição, que só confere os hashes antes das rodadas | `--out/.ab.lock` nas duas fases (desde a rodada 8, o arquivo comum `--out/.gizmo-perf.lock`) | F4, F5 e F6 compartilham `Guarded` (4) e `OwnedDirectories` (6) |
| F5 | [`run.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535825973), varredura: `native.py` | `run.py` apagava relatório, stdout e stderr antes de qualquer trava; `native.py` escrevia na `--out` sem trava | `--out/.run.lock` e `--out/.native.lock` (desde a rodada 8, o arquivo comum `--out/.gizmo-perf.lock`); cada `main` virou argumentos mais uma função de fase, e todo retorno antecipado fica dentro da trava | (os mesmos 10) |
| F6 | [`mutate.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535825925), varredura: os outros builds | a trava era por `--out`: dois `prepare` com `--out` diferentes editavam os mesmos arquivos do checkout: o segundo `Mutated` podia salvar como `original` um arquivo que o primeiro já tinha mutado e devolvê-lo mutado na restauração; e um build de outro runner nesse checkout podia construir com um mutante na árvore | lock do checkout (`harness.checkout_lock_path`, `harness.guarded`) tomado **antes** do da `--out` por toda fase que constrói ou edita; nunca espera | (os mesmos 10) |
| F7 | [`mutate.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535825930) | `prepare` só conferia os arquivos-alvo: uma árvore suja em outro lugar construía os executáveis, e um stash depois deixava o `time` ver "o mesmo HEAD" | `prepare` recusa árvore suja antes de mutar qualquer coisa; `time` recusa um `prepare` cuja `identity.dirty` não é `''` ou não foi registrada | `MutantExecutables` (2) |
| F8 | [`mutate.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535825936) | `exes/` era apagado a menos que algum resultado fosse `BLOCKED` ou `MISSING_EXECUTABLE`: `INVALID_GUARD_AFTER_RUN` e `INFRA_*` também o apagavam | `exes/` fica sempre que um mutante de tempo avaliado terminou sem resposta | `MutantExecutables` (1, nove desfechos) |
| F9 | [`mutate.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535825943) | um `--only` que não avalia mutante de tempo (`--only memo_off`) apagava os executáveis dos dois mutantes de tempo, e o `--only delay_1ms_per_handler` seguinte dava `MISSING_EXECUTABLE` | só os executáveis dos mutantes avaliados que deram uma resposta são removidos | `MutantExecutables` (2) |
| F10 | [`native.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535825948) | `trace()` lia o erro de um `render.status` com `ok=False` como relatório, e a ausência de `session.error` virava "sessão sem erro" | `{}` com `last_trace_ok=False` e `trace_failures`; os checks "every `render.status` query answered ok" e "the session reported no error" falham | `NativeSession` (1) |
| F11 | [`native.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535825951) | `settle()` conferia o prazo só antes de `viewport()`: uma resposta lenta (até 90 s) valia "convergiu em N s" | cada consulta recebe o tempo que resta, o prazo é conferido de novo depois da resposta e uma resposta tardia é timeout (`ms` `None`, com o estado e a info guardados) | `NativeSession` (2) |
| F12 | [`native.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535825957) | `author_saved_project` descartava os problemas devolvidos por `window.stop()`: a medição podia começar com o processo autor ainda vivo e terminar com sucesso | vão para `cleanup_errors` (`authoring process: ...`) e o check "the editor process ended without cleanup problems" falha; o diretório temporário continua removido e um erro da autoria nunca é trocado | `NativeSession` (1) |
| F13 | [`native.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535825968), item (18) | `overlay_drawn` só era gravado e o laço aceitava o primeiro `drawn` não vazio, que podia ser o do passo anterior (`small/state.scale.local` trouxe `rotate`/`world`, revisão 8); `drawn` nulo em 5 s não falhava | só vale o reconhecimento que **pertence ao passo**: visibilidade, ferramenta, espaço, entidade, cena, revisão, quadro e a seleção do carimbo, conferidos com o pedido e a resposta do próprio passo (nunca com o reconhecimento anterior), esperado por até `ACK_SECONDS` (5 s); senão um check falha mostrando o esperado e o último visto, e nulo também falha. Vale só para o passo com overlay **visível** (item (23)) | `OverlayAcknowledgement` (5) |
| F14 | [`run.py`](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4535825979) | `{**os.environ, ...}`: um `TESSEL_GIZMO_PERF_CALIBRATE=1` exportado virava a medição em calibração sem o `verdict()` saber | `harness.child_env`: toda `TESSEL_GIZMO_PERF_*` herdada é descartada (nos builds, também `CARGO_TARGET_DIR` e `TESSEL_CARGO_LANE`) e entra só o que o runner define (`CALIBRATE` só com `--calibrate`, e explícito no `ab.py`); `run.env` mostra só isso e `env_dropped` o que saiu; o `native.py` deixou de reescrever `os.environ` | `ChildEnvironment` (8), `BuildPhase` (1) |
| F15 | item (19) | `EXPECT[(memo_off, host)]` não cobria a primeira verificação que falha (a contagem de layouts), e o mutante detectado ficava `UNEXPECTED_FAILURE` | aceita também `a sync of an unchanged state laid the scene out more than once`; segue estrito: outra mensagem do mesmo teste, erro de compilação ou timeout não matam | `Verdicts` (1) |
| F16 | item (21) | `ab.py --phase measure` apagava `verdicts.json` e os relatórios da tentativa anterior | move a tentativa anterior para `--out/attempts/<utc>-<estado>/` antes de escrever e grava `previous_attempt_archived_at` | `MeasurePhase` (1) |
| F17 | item (22) | `tests_passed` e `verdict()` contavam o teste cujo marcador `test NAME ...` aparece, e ele também aparece quando o teste falha | só conta o teste cuja própria linha de resultado diz `ok` (o layout do `--nocapture`, com a saída do teste entre o marcador e o veredito, continua contando); o manifesto traz `tests_passed` e `tests_failed` | `RunVerdict` (1), `ChildEnvironment` (1) |
| F18 | contexto do item (20) | o motivo de um `FAIL` não dizia que a guarda depois da corrida tinha falhado | o status continua `FAIL` (uma falha real nunca é mascarada) e o motivo carrega o texto da guarda, em `run.py` e em `native.py` | `RunVerdict` (1), `MainRun` (1) |

**Garantias que valem agora (como lidas no código).**

- *Git.* Uma identidade que o `git` não soube dar é recusada antes de construir, mutar ou medir, com `--allow-dirty` também; `harness.out()`
  ficou tolerante **de propósito** para a telemetria (`pmset`, `sysctl`, `ps`, versões de `rustc` e `cargo`): ali uma resposta que falta é só
  texto gravado, nunca uma decisão. As demais leituras de git que decidem algo são estritas: o `mutate.py` só chama de restaurado um arquivo
  que o git comparou com o HEAD (sem resposta não é restaurado); o `native.py` não toma uma listagem vazia por "runtime novo"; o commit da
  baseline do `ab.py` vem de `git_strict`; `git archive`, `git ls-tree` e os demais do `ab.py` já usavam `check=True`.
- *Travas.* As de F4 a F6: uma por `--out` (todo runner, toda fase) e uma por checkout (toda fase que constrói ou edita). Nenhuma espera.
- *Ambiente.* Os filhos só recebem o que o runner define; o `ab.py --phase build` recusa se `CARGO_TARGET_DIR` ou `TESSEL_CARGO_LANE` estiverem
  definidos, porque com um target escolhido de fora os dois builds dividiriam um caminho de artefato. Os manifestos guardam só os **nomes** descartados.
- *Executáveis.* Um build pode podar o target gerenciado de outro (o `mbx` podou o do candidato durante o build da baseline em `12bc8da`; a causa
  **não foi isolada**): o `ab.py` copia o executável do candidato assim que ele sai do build, e tudo que falhar depois (caminho sumido, cópia,
  mesmo arquivo, mesmo sha256) termina como manifesto gravado e exit 1, nunca como traceback sem manifesto.
- *Tentativas do A/B.* Uma tentativa nova de `measure` nunca apaga a anterior: o manifesto, os relatórios por rodada, os aquecimentos e o
  `verdicts.json` vão para `--out/attempts/` antes de qualquer escrita; o executável, o `baseline-src/` e o `manifest.build.json` são entradas e ficam.
- *Amostra e veredito.* Zero amostra nunca é `NO_REGRESSION_IN_SCENARIO`; `--rounds` < 1 é erro de argumento; um teste só conta como passou se a
  própria linha dele diz `ok`; um `FAIL` com a guarda depois da corrida falha traz o texto da guarda.
- *Janela nativa.* O reconhecimento do overlay guardado para um passo é o dele; um `render.status` que falhou reprova os checks da sessão em vez de
  parecer uma sessão limpa; a espera de `settle` tem prazo por consulta e a do reconhecimento tem o dela; o que o processo autor reportou ao parar reprova a limpeza; o diálogo
  que deixou a página nativa na tela reprova o check do passo (o valor era gravado e nunca lido).

**Varredura de mesma classe** (`rg` de `os.environ`, `harness.out`, `subprocess.run`, `test NAME ...` e das chaves que o `native.py` grava e não
confere). *Corrigido no lugar* (cada um com teste que falha nos runners antigos): a leitura de frescor do runtime (`runtime_stale_reason`: um
`git ls-files` que falha ou que lista só arquivos que não existem lia "nenhum fonte mais novo"); a comparação arquivo × HEAD do `mutate.py`; o
`os.environ.update` permanente do `native.py` (agora escopado); o check do diálogo; o OSError da cópia do executável em `run.py` e em
`mutate.py prepare`; `judge()` sem amostra. *Registrado como itens novos de BL015.23, sem correção*: **(23)** os passos de overlay **oculto**
(`open.first_overlay`, `state.other.rotate`, `state.hidden`) não têm reconhecimento nem check dele; **(24)** o `native.py` grava e não julga
`trace.dropped`, `picture_summary.all_read_back` e `overlay_report.stale` (só o `error` do relatório do overlay é checado); **(25)** o lock do
checkout é por caminho de checkout, e dois worktrees dividem o cache do `mbx` (a poda de um target gerenciado durante o build de outro, causa do
item (15), é sobrevivida pela cópia imediata, não impedida).

**O que continua limitado.** As travas são `flock` consultivos: seguram estes runners uns dos outros, não um `cargo build` nem um editor iniciado
à mão. A espera do reconhecimento do overlay é de 5 s: uma página mais lenta que isso reprova o check (numa máquina carregada isso pode ser uma
falha falsa, lida como falha, nunca ignorada). Seguem como estavam: o `--allow-dirty` de `run.py`, `ab.py` e `native.py`, que muda status e exit
de maneiras diferentes (BL015.23 (13)(a)); o manifesto `BUILT` do `ab.py`, sem `exit` nem `utc_end` (item (15)); o log bruto do host que o
`mutate.py` não guarda (item (19)); os itens (14), (16), (17) e (20) e a falta de baseline do host e da janela. Nenhum runner corrigido foi
reexecutado contra uma medição, então **o efeito dessas correções sobre os números ainda não foi visto**.

**Prova desta rodada (só dublês, sem benchmark, sem janela e sem lock de performance).** A spec passou de 115 para **184 testes**
(`gizmo-perf.spec.py`, de 37 a 51 s conforme a carga da máquina). Contra os runners de `d294bdc`, **65 dos 69 testes novos falham** (nenhum dos
115 anteriores falha) e 4 são controles que passam dos dois lados de propósito. **Dez versões erradas de uma correção** (F13: aceitar o primeiro
reconhecimento não vazio, e um check que não pode falhar; F14: descartar só o `TESSEL_GIZMO_PERF_CALIBRATE`, deixando um `BLESS` herdado chegar ao filho; F16: não arquivar a tentativa; F8:
apagar `exes/` sempre; F2: aceitar executáveis iguais; F11: consulta sem prazo próprio; F10: ler o erro do `render.status` como relatório; F4 e
F5: trava que espera; F6: build sem o lock do checkout) são pegas, cada uma por pelo menos um teste. **Dois processos reais** (um segura as
travas, o outro é o runner) provam a linha `REFUSED` única, o exit 1 e a `--out` intacta, nos quatro runners:
[`spec-new-tests-vs-d294bdc-runners.log`](../migration/gizmo-perf-evidence/spec-new-tests-vs-d294bdc-runners.log). Sem Rust alterado, os gates do
workspace de `081353e` (2299 passaram, 0 falharam, 16 ignorados; `cargo tree` sem gpui/wry) não foram repetidos.

**O que esta rodada NÃO fez:** medir tempo; rodar `run.py`, `ab.py`, `mutate.py` (`time`) ou `native.py` de verdade; construir release; abrir a janela;
mudar `budgets.json`, jornada, corpus, golden, margens, limiares ou qualquer Rust; decidir os itens (14), (16), (17) e (20); tratar o aviso
"Docstring Coverage"; responder ou resolver threads da review. **Nenhuma afirmação de desempenho vale: implementado não é validado.**

## Rodada 7 de correções (PR #44, de `42229a3` a esta entrega)

Escopo: **só `scripts/gizmo-perf/native.py`, a spec e os docs** (`scripts/gizmo-perf/**`, `docs/**`). Nenhum Rust, runtime, fixture, `budgets.json`, jornada,
corpus, golden, margem ou limiar mudou, e **nada foi medido**: nenhum `native.py --phase run`, `run.py`, `ab.py` nem `mutate.py --phase time` rodou, nenhuma janela abriu,
nenhum build foi feito (a máquina estava carregada e com pouco disco) e [`measure-12bc8da/`](../migration/gizmo-perf-evidence/measure-12bc8da/README.md) não foi
editado. Os dois defeitos são os itens (16) e (17) de BL015.23, e **as 20 verificações que falharam na primeira execução real da janela são exatamente estes dois (15 + 5)**.
Código, spec e log de controle: `3ee60de`. **Implementado não é validado**: nada nesta seção diz que o desempenho foi validado nem que não há regressão, e o `native.py` corrigido
**não foi reexecutado numa janela**. O que é lido, o que é observado nos registros e o que é inferido está separado abaixo.

| # | Item | O que o runner fazia | O que faz agora | Testes novos que falham nos runners de `42229a3` |
|---|---|---|---|---|
| H16 | BL015.23 (16) | depois de `scene.pointer.up` exigia um quadro com pedido **mais novo** que o do candidato e esperava 30 s por ele: 15 de 15 soltas reprovaram em "the committed picture is a NEW picture" | com o candidato **desenhado**, aceita exatamente dois desfechos e grava qual viu (`commit_picture`: `redrawn` ou `deduped`, e `commit_observed`); o resto reprova e um timeout nunca é `deduped`; com o candidato nunca desenhado a regra estrita continua | `Release` (15 de 15) e parte de `RecordedFirstRun` |
| H17 | BL015.23 (17) | exigia que todo gesto mudasse o hash do quadro inteiro: 5 reprovações na escala do `dense` (`drag.scale.local.uniform` e `drag.scale.local.x` com 2 cada, `drag.scale.cancel` com 1) | a expectativa é uma tabela declarada por corpo e classe (`PIXEL_EXPECTATIONS`, gravada em cada relatório): tudo **deve** mudar o quadro, exceto a escala do `dense`, que **pode** ficar igual e então grava os hashes; os checks vazios são gravados como vazios | `PixelExpectations` (10 de 11), `GestureInTheCorpus` (4 de 4), parte de `RecordedFirstRun` (6 de 7 no total) |

**H16: o que foi lido, o que foi observado, o que não foi.**

- *Lido no código (não medido).* `ViewportHost::render_scene` (`crates/desktop/src/workstation/scene_monitor_host.rs`, linhas 155 a 161) monta a chave
  `"{cena:?}|{plano JSON}|{mapa de recursos:?}|{opções:?}"` e, para um pedido **não explícito** com chave igual à do último, só despacha o que está na fila e devolve `Ok(None)`: nenhum
  pedido novo, nenhum quadro. O plano de um commit é o do candidato: o `ScenePlan` traz só a revisão **do bundle** (`source.revision_id`, um texto do arquivo do bundle) e nenhum contador de edições nem id de
  gesto (R7, rodada 8), e a chave igual é o que os registros de `12bc8da` mostram (abaixo) e é coerente com o código, **sem estar provada só por ele**. O servidor de automação responde em ordem,
  na thread principal, depois de `Workstation::execute` (`overlay_adopt` → `viewport_sync` → `viewport_follow` → `render_scene` → `overlay_refresh`), então **a decisão do host precede a
  resposta do `scene.pointer.up`**: a primeira leitura do monitor depois dela já mostra se o host pediu um quadro.
- *Observado nos registros de `12bc8da` (relidos, nada medido de novo).* Nas 15 soltas o quadro depois da soltura é o do candidato (mesmo `request`, mesmo hash) e a soltura **não consumiu
  id de pedido**: o quadro seguinte é o do undo, candidato + 1 (os pedidos são deslocamentos de 2^48; no `small`, o `drag.translate.x` desenhou `+3` e o `history.undo.translate`, `+4`; no `dense`, `+4` e
  `+5`; no `deep`, `+3` e `+4`); os 6 cancelamentos desenharam candidato + 1 (o plano volta à base, que difere da última chave); o redo do translate reproduz o hash do candidato
  (`small` `409cc0797fed653e`, `dense` `178bbef3dae07012`, `deep` `65dd61def24e5e29`); os hashes da base são `8d4c865fafc56fcc`, `a5cee15db5c0b400` e `0bab17c6bc2ecfe3`.
- *Conclusão.* **Nenhum defeito de produto é indicado**: o código e os registros bastam para explicar as 15 reprovações, e a expectativa do runner (um quadro novo depois de um commit idêntico
  ao candidato) é que estava errada. Isso é **inferido do código e observado nos registros, não medido de novo**; que o quadro de uma soltura de rotação ou de escala seja o do candidato é
  argumentado pela igualdade da chave (os registros mostram o mesmo hash), não foi renderizado de forma independente. O item (27) registra que não há teste de host que fixe esse
  comportamento.
- *O que a janela tem de dizer.* O `commit_picture` de cada soltura. O reconhecimento que a página dá ao estado do commit **nunca foi observado numa janela** (a rodada 6 o implementou para os
  passos de `scene.gizmo.state`, e a execução real é anterior a ela): se a página nunca o der, a soltura vira `unconfirmed` e reprova.

**H17: o que foi lido, o que foi simulado, o que não foi.**

- *Lido (fixture e runtime).* O nó arrastado do `dense`, `bank-5-plane-17` (`dense.bundle.json`), é um plano de **0,1 unidade** com a textura `mid` (`scripts/directed-scene-proof/corpus/assets/mid.png`,
  48 × 48): um quadrado opaco de 32 × 32 texels (6 cores distintas, quadrantes chapados) dentro de uma borda de 1280 texels **totalmente transparentes** (alpha 0, sem texel parcialmente
  transparente). As texturas do runtime usam `LinearFilter` e não geram mipmap (`runtime/src/assets.js`, linhas 55 a 57). Os nós de `small` e `deep` são planos de 4 e de 1 unidade.
- *Simulado pelo autor (inferido, não medido).* Com a câmera do core no quadro 24, o nó cobre cerca de 2,2 × 2,7 px do quadro de 320 × 180 (4 centros de pixel, uma cor de quadrante em cada); a escala
  uniforme do gesto (×1,4417 no golden) passa a cobrir 12 centros, e os 8 acrescentados caem na borda transparente (alpha 0) enquanto os 4 de antes mantêm as cores: **o hash do quadro inteiro não
  muda**. A simulação reproduz os cinco resultados gravados (`translate.x`: 6 pixels diferentes; `rotate.local.y`: 2; `rotate.world.z`: 1; `scale.uniform` e `scale.x`: 0) e nenhum plano pintado depois do
  nó cobre os pixels dele.
- *Não confirmado.* "Cerca de um pixel" **não foi confirmado literalmente** (são 4 centros de pixel). Um defeito de produto **não está excluído por medição**: só não é preciso para explicar os
  registros. A prova por pixel de que a escala do `dense` muda o quadro exigiria um nó opaco no corpus (mudança de fixture, fora desta rodada).

**Regras que valem agora (como lidas no código).**

- *Soltura* (`Window.settle_commit`, usada por `drag` quando o candidato foi desenhado). Cada leitura do monitor leva o tempo que resta; um estado não convergido (um quadro em voo, ou nenhum ainda) reinicia o período
  quieto. Classificação de cada quadro convergido (`commit_kind`): `redrawn` (pedido mais novo que o do candidato), `candidate` (o mesmo pedido **e** o mesmo hash), `older` (pedido mais velho) ou `other`
  (o pedido do candidato com outros pixels, ou ids que não se comparam). `redrawn` termina na hora; `older` e `other` reprovam na hora. `deduped` exige o quadro do candidato, convergido, **sem nada em voo**,
  estável por `COMMIT_QUIET_SECONDS` (0,25 s) **e** o reconhecimento da página do estado da soltura (o overlay da resposta do `up`: visibilidade, ferramenta, espaço e entidade e, no carimbo, cena, ferramenta, espaço, revisão e quadro) sem
  problema; sem overlay visível na resposta, ou sem reconhecimento em `ACK_SECONDS` (5 s), o desfecho é `unconfirmed` (reprova). O teto de tudo é `COMMIT_SECONDS` (30 s): estourá-lo reprova (`outcome` `None`),
  **nunca** vira `deduped`, e o hash do último estado visto não é comparado com a base. O `client_ms_up_to_converged_picture` de uma soltura `deduped` é o custo de ver que **nada foi desenhado**, não a latência
  de um render.
- *Redesenho.* Um `redrawn` também tem de mostrar os pixels do candidato (o mesmo hash); se o candidato era a base, o check é gravado como vazio.
- *Pixels.* `must_change`: o candidato desenhado e o quadro da soltura diferem do quadro de antes do gesto (dois checks que reprovam). `may_stay_equal`: grava `pixels` (`base_hash`, `candidate_hash`, `committed_hash`,
  `candidate_changed`, `committed_changed`, `after_cancel_hash`) e **não** reprova a igualdade. O que não está na tabela é `must_change`: uma igualdade tem de ser declarada, com a evidência, e nenhum outro corpo
  ou classe herda a licença. O golden de transformada, a entrada de undo, a profundidade de undo/redo, "o candidato não está no documento antes da soltura" e o cancel que deixa a base valem para todo passo.
- *Checks vazios.* Um check que vale com qualquer produto porque os quadros envolvidos são o da base (o cancel de volta a uma base que o candidato já era, o undo de um commit que não mudou nada, o redesenho igual
  ao candidato quando o candidato era a base) **continua sendo feito** (pode reprovar) e é gravado em `vacuous_checks` do passo com o motivo: o relatório não declara uma prova que não fez.

| Corpo | translate | rotate | scale | Evidência da linha |
|---|---|---|---|---|
| `small` (nó `mid`, 4 unidades) | `must_change` | `must_change` | `must_change` | quarenta vezes a largura do nó do `dense`; os 5 gestos gravados em `12bc8da` mudaram o hash |
| `deep` (nó `leaf-rot`, 1 unidade) | `must_change` | `must_change` | `must_change` | dez vezes a largura do nó do `dense`; os 5 gestos gravados mudaram o hash |
| `dense` (nó de 0,1 unidade) | `must_change` | `must_change` | **`may_stay_equal`** | `translate.x`, `rotate.local.y` e `rotate.world.z` mudaram o hash em `12bc8da`; `scale.local.uniform`, `scale.local.x` e o candidato do `scale.cancel` deram o hash da base apesar de cada candidato ter sido **desenhado** (H17, simulação); `rotate.world.z` muda só 1 pixel na simulação: é a linha mais frágil |

**Varredura de mesma classe** (leitura de `native.py` à procura de "exige quadro estritamente mais novo" e "exige pixel mudado", e de valores gravados e nunca julgados). *Sem mudança, com a razão*: o undo e o
redo (o plano difere da última chave a cada vez; os 21 gravados desenharam o pedido do quadro anterior + 1), o cancel (idem; 6 de 6, candidato + 1), o `monitor.show_scene` e o `seek` (a cena e o quadro
fazem parte da chave, BL015.22; os 15 checks `same.*`/`switch.*` de cada corpo passaram e o `open.first_frame` desenhou um pedido novo nos três), o `present.resize*` (só espera a convergência, sem exigir
quadro novo) e o `project.open` (não há quadro) não assumem um quadro novo que o host possa deduplicar. *Corrigido no lugar* (mesma classe de "vale com qualquer produto"): o check de
"depois do cancel o quadro é o da base" e o do "undo restaurou o quadro exato" passaram a ser **gravados como vazios** quando comparam a base com ela mesma (antes passavam sem dizer isso); com o `dense/scale` eles
são vazios nos registros. *Registrado como itens novos de BL015.23, sem correção*: **(26)** o redo só confere um quadro mais novo (o hash dele nunca é comparado com o do commit; o `before_hash` só é um
"não nulo"; a pilha de undo não avança quando o undo não desenha; **corrigido no runner na rodada 10**, exceto o `before_hash`, que virou o item (32): ver "Rodada 10"); **(27)** o comportamento de produto por trás de H16 não tem teste de host; **(28)** o `native.py` grava `trace.enabled` e não o julga (mesma
família do (24), que continua como estava).

**O que uma execução real da janela tem de mostrar, e o que refutaria H16 e H17.** Se as duas hipóteses valem, a primeira execução do `native.py` corrigido deve dar: **todos os `drag.*` passando** nos três
corpos; `commit_picture` gravado em cada soltura (como lido, `deduped` nas 15; `redrawn` também é aceito, e então com o hash do candidato), com `commit_observed` mostrando o pedido e o hash do quadro que ficou e o
reconhecimento que a página deu; a escala do `dense` marcada `may_stay_equal` com `pixels.candidate_changed` e `pixels.committed_changed` **falsos** (verdadeiros também passam: a regra não exige igualdade);
`small` e `deep` com hashes do candidato e do commit diferentes da base; `vacuous_checks` listando os checks vazios (esperados no `dense/scale`). **Refutariam**: uma soltura com um quadro **mais velho** que o do
candidato; um hash igual ao da base em `small` ou `deep` (regra `must_change`: produto ou corpus a investigar); a transformada do documento diferente do golden; um `redrawn` com outros pixels que o candidato;
e `unconfirmed` (a página não reconheceu o estado da soltura: ou o produto não manda o overlay do commit, ou a expectativa do reconhecimento está errada; é uma hipótese à parte, **nunca observada**, e não refuta
H16). **Só uma janela mostra** tudo isso; nada sobre tempo.

**Prova desta rodada (só dublês e o relatório gravado: sem benchmark, sem janela, sem build e sem lock de performance).** A spec passou de 184 para **221 testes** (`gizmo-perf.spec.py`, de 38 a 43 s conforme
a carga): `Release` (15), `PixelExpectations` (11), `GestureInTheCorpus` (4) e `RecordedFirstRun` (7), sobre um modelo do host com **relógio virtual** (um host que desenha, deduplica, demora, nunca converge, ou
não reconhece) e sobre o relatório de `12bc8da`, só lido. Contra os runners de `42229a3`, **35 dos 37 testes novos falham** (nenhum dos 184 anteriores falha; 2 controles passam dos dois lados de propósito: o
cancel de um candidato diferente continua um check de verdade, e os fatos do relatório gravado). **Dezesseis versões erradas das correções** são pegas, cada uma por pelo menos um teste (aceitar um quadro mais
velho; aceitar qualquer hash para o pedido do candidato; `may_stay_equal` para todo corpo; descartar o golden de transformada e a profundidade de undo; timeout lido como `deduped`; sem período quieto; sem o
reconhecimento da página; comparar com a base o hash de um estado que nunca convergiu; não gravar o que é vazio (no cancel e no redesenho, e no undo); tratar o cancel como vazio mesmo quando o candidato
difere da base; um corpo não declarado herdando a licença; o redesenho não comparado com o candidato; o candidato nunca desenhado sem a regra estrita; a tabela declarando o `dense/scale` como `must_change`,
que é o próprio defeito; o candidato exigido diferente da base também onde a licença permite igualdade). O **replay** dos 21 gestos de `12bc8da` pelo `native.drag`
novo, sobre um monitor que mostra os pedidos e hashes gravados, dá **0 reprovações** contra as 20 que o runner da época registrou (as 15 soltas e as 5 do `dense`): **não é medição** (a soltura é modelada como
`dedupe`, como o registro mostra, e o reconhecimento da página é **suposto**, porque o relatório não o guarda). Tudo em
[`spec-new-tests-vs-42229a3-runners.log`](../migration/gizmo-perf-evidence/spec-new-tests-vs-42229a3-runners.log). Sem Rust alterado, os gates do workspace de `081353e` (2299 passaram, 0 falharam, 16
ignorados; `cargo tree` sem gpui/wry) não foram repetidos.

**O que continua limitado.** Os checks de pixel julgam o hash do quadro **inteiro** (320 × 180): a licença do `dense/scale` não distingue "nada mudou" de "mudou fora do nó". O `deduped` depende de um
reconhecimento da página que nunca foi visto depois de uma soltura, e a espera dele é de 5 s (numa máquina carregada pode ser uma falha falsa, lida como falha, nunca ignorada). O `must_change` do `dense` em
rotate repousa em 1 pixel na simulação. H16 e H17 são lidas e simuladas, não medidas numa janela. Seguem como estavam: os itens (14), (20) e (23) a (25), o `--allow-dirty` (13)(a) e a falta de baseline do host e
da janela. O `native.py` corrigido **não foi reexecutado**, então **o efeito dessas correções sobre uma execução real ainda não foi visto**.

**O que esta rodada NÃO fez:** medir tempo; rodar qualquer runner de verdade; abrir a janela ou construir release; mudar `budgets.json`, jornada, corpus, golden, fixtures, margens, limiares ou qualquer Rust;
decidir os itens (14) e (20); tratar o aviso "Docstring Coverage"; responder ou resolver threads da review. **Nenhuma afirmação de desempenho vale: implementado não é validado.**

## Rodada 8 de correções (PR #44, de `ca66012` a esta entrega)

Escopo: **só `scripts/gizmo-perf/**` (runners e spec) e `docs/**`**. Nenhum Rust, runtime, fixture, `budgets.json`, jornada, corpus, golden, margem ou limiar mudou, e **nada foi medido**:
nenhum `run.py`, `ab.py`, `mutate.py` (`prepare` ou `time`) nem `native.py` rodou de verdade (os únicos processos reais de runner que a prova usou param antes de construir ou medir qualquer coisa: os novos **recusam** na trava e os de `ca66012`, que não a recusam, param por outro motivo, depois de escrever na `--out` do outro), nenhuma janela
abriu, nenhum build foi feito (a máquina estava carregada e com pouco disco) e [`measure-12bc8da/`](../migration/gizmo-perf-evidence/measure-12bc8da/README.md) não foi editado. Os achados são os
seis comentários da [review 5859397176](https://github.com/journey-studios/tessel-editor/pull/44#pullrequestreview-5859397176) do CodeRabbit (feita em `42229a3`, o commit de docs da rodada 6; os comentários foram reancorados em `ca66012`), cada um conferido contra o código de `ca66012` antes de
qualquer correção (os seis são válidos); o aviso genérico "Docstring Coverage" não é um achado e **segue não tratado** (BL015.23 (13)(d)). A rodada também corrigiu duas imprecisões de documentação (R7 e R8,
abaixo). Código, spec e log de controle: `e40d511`. **Implementado não é validado**: nada nesta seção diz que o desempenho foi validado nem que não há regressão, e **nenhum runner corrigido foi
reexecutado contra uma medição**.

| # | Comentário | O que o runner fazia | O que faz agora | Testes novos que falham nos runners de `ca66012` |
|---|---|---|---|---|
| R1 | [r4538394726](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4538394726) (Major), `harness.py`, `git_strict` | todo início de git herdava o ambiente do chamador: com `GIT_DIR`, `GIT_WORK_TREE` ou `GIT_INDEX_FILE` exportados o git ignora o diretório em que foi iniciado e responde por **outro** repositório, e a identidade dele (HEAD, árvore, ramo, `dirty`) seria gravada como a deste checkout | `harness.git_env()`: o ambiente sem as 15 variáveis de repositório que `git rev-parse --local-env-vars` lista; **todo** início de git passa por ele (`git_strict`, os sete pontos de chamada de `ab.py` por `git_run`, o `git rev-parse HEAD:runtime` do `native.py`) | `GitEnvironment` (5) e `Baseline` (1): 6 de 6 |
| R2 | [r4538394739](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4538394739) (Major), `harness.py`, `IDENTITY_QUERIES['dirty']` | a identidade só olhava `crates`, `Cargo.toml`, `Cargo.lock`, `scripts` e as fixtures do corpus: uma edição em `.cargo`, `rust-toolchain.toml`, `vendor` (entradas de build que o próprio A/B conta) ou `runtime` (o JS que a janela serve) deixava a árvore "limpa" | `harness.DIRTY_PATHSPECS`, **uma** constante com nove caminhos, usada pela consulta; um teste confere que ela cobre cada arquivo rastreado que `ab.is_prod` chama de entrada de build | `DirtyIdentity` (6): 4 de 6 |
| R3 | [r4538394750](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4538394750) (Minor, código), `harness.guarded` | cada runner tomava um arquivo de trava com **o nome dele** (`.run.lock`, `.ab.lock`, `.native.lock`, `.mutate.lock`): dois runners **diferentes** na mesma `--out` não se excluíam (um `mutate.py --phase prepare` esvazia `exes/` de que um `run.py --phase measure` roda), ao contrário do que a docstring de `guarded` e o doc diziam | **um** nome para os quatro (`harness.OUT_LOCK` = `.gizmo-perf.lock`); a recusa diz que **qualquer** runner pode ser o dono | `OneLockPerOut` (3), `TwoRunnerProcesses` (1) e um de `MutantExecutables` (1): 5 de 5 |
| R3 (doc) | [r4538394711](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4538394711) (Minor, doc), este arquivo, a trava | pedia que o doc estreitasse a promessa ("duas execuções ao mesmo tempo não dividem uma `--out`") ao mesmo runner; os dois comentários puxavam em direções opostas (doc: estreitar; código: uma trava comum) | **resolvido no código**, para que a frase do doc seja **verdadeira** (R3); o doc agora diz o que a trava faz (exclui execuções **simultâneas**, de qualquer runner) e o que não faz (não exclui as **sequenciais**), e mantém a regra "um diretório por runner", com o porquê | os de R3 |
| R4 | [r4538394770](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4538394770) (Major), `mutate.py`, `run_phase` | só o `prepare` recusava a árvore suja; o `time` só olhava a identidade que o `prepare` **gravou** (e o commit e a árvore de objetos), nunca a árvore de agora | o `time` recusa a árvore atual suja (`REFUSED`, motivo próprio, antes de medir ou apagar qualquer coisa); a mensagem do `prepare` e a conferência do manifesto do `prepare` ficam como eram; sem `--allow-dirty` | `MutantExecutables` (2): 2 de 2 |
| R5 | [r4538394785](https://github.com/journey-studios/tessel-editor/pull/44#discussion_r4538394785) (Minor), `native.py`, `overlay_ack` | cada consulta da espera de 5 s do reconhecimento usava o prazo padrão de 90 s e um reconhecimento que casava mas **chegava** depois dos 5 s era aceito | como `Window.settle`: cada consulta recebe o tempo que resta, o prazo é conferido de novo **depois** da resposta (uma resposta tardia é timeout, com o orçamento no texto, nunca reconhecimento), `socket.timeout` e `TimeoutError` viram problema gravado, e há sempre ao menos uma consulta | `BoundedPolls` (5): 4 de 5 |
| varredura (c) | (sem comentário) | o laço do diálogo (3 s) e o do `job.get` (60 s) do `native.py` tinham o mesmo defeito do R5: consulta com o prazo padrão de 90 s | o do diálogo virou `page_hidden_by_dialog` (limitado como o R5) e o do `job.get` dá a cada consulta o tempo que resta, com timeout como check que falha | `BoundedPolls` (4): 4 de 4 |
| varredura (d) | (sem comentário) | `Window.settle_commit` aceitava um reconhecimento que casa mas **visto** (por uma leitura lenta) depois de `ACK_SECONDS` do primeiro quadro do candidato, e o contava como `deduped` | o primeiro instante em que a leitura viu o reconhecimento que casa é guardado; depois do orçamento o desfecho é `unconfirmed`, com o orçamento no texto | `LateAcknowledgement` (3): 1 de 3 |
| R7 (doc) | (achado de documentação, sem id de comentário inline) | a leitura de H16 dizia que "o `ScenePlan` não carrega revisão nem id de gesto" | o texto diz o que o código tem (a revisão **do bundle**) e separa o lido do observado | só texto |
| R8 (doc) | (achado de documentação, sem id de comentário inline) | o README da evidência dizia "não tratados" para (16), (17) e (20) | (16) e (17) tiveram o runner corrigido na rodada 7 (não reexecutado); só o (20) segue não tratado | só texto |

**R1: o que foi lido, o que foi observado, o que não foi.**

- *Lido.* O `git rev-parse --local-env-vars` do git instalado (2.50.1) lista 15 nomes: `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_CONFIG`, `GIT_CONFIG_PARAMETERS`, `GIT_CONFIG_COUNT`, `GIT_OBJECT_DIRECTORY`,
  `GIT_DIR`, `GIT_WORK_TREE`, `GIT_IMPLICIT_WORK_TREE`, `GIT_GRAFT_FILE`, `GIT_INDEX_FILE`, `GIT_NO_REPLACE_OBJECTS`, `GIT_REPLACE_REF_BASE`, `GIT_PREFIX`, `GIT_SHALLOW_FILE` e `GIT_COMMON_DIR`. `harness.GIT_LOCAL_ENV` os
  traz escritos no código (o git lista os nomes, mas um programa não os recebe de volta como lista), e um teste confere que **todo nome que o git instalado imprime** é removido: se uma versão futura do git
  acrescentar um, esse teste falha e a lista é atualizada.
- *Observado (spec, em repositórios temporários, git real).* Com `GIT_DIR` (e com `GIT_DIR` e `GIT_WORK_TREE`, só `GIT_WORK_TREE`, só `GIT_INDEX_FILE`) apontando para **outro** repositório, sujo, a identidade do
  checkout (limpo) continua sendo a dele; um `git` de mentira que imprime o próprio ambiente mostra que nenhum dos 15 nomes chega ao git iniciado por `git_strict`, `git_identity`, `native.runtime_source_tree` e
  `ab.git_run`, e que o que não é de repositório (`GIT_AUTHOR_NAME`, `HOME`, `PATH`, `LC_ALL`) chega. Um teste lê o fonte dos runners e falha se um `['git', ...]` novo for iniciado sem `git_env`.
- *Não mudou, de propósito.* `child_env` (o ambiente dos filhos que medem ou constroem) **não** descarta `GIT_*`: não há script de build de crate (`find crates -name build.rs` só acha o módulo
  `scene_plan/build.rs`) nem dependência que leia o git, mas `./scripts/cargo` e o `mbx` **não foram lidos para isso**; tirar essas variáveis dos builds seria uma mudança sem prova. Registrado como item (30) de BL015.23.

**R2: o que cobre a identidade agora.** Os nove caminhos de `harness.DIRTY_PATHSPECS`: `crates`, `Cargo.toml`, `Cargo.lock`, `scripts`, `docs/migration/fixtures/gizmo-perf` (o que já havia) e `.cargo`,
`rust-toolchain.toml`, `vendor`, `runtime` (novos). Lido e verificado com git real: um caminho que não existe neste checkout (`.cargo`, `vendor`, `rust-toolchain.toml`) não dá erro em `git status --porcelain --`
(exit 0, nada impresso); `runtime/dist` e `runtime/node_modules` são ignorados pelo git, então `./scripts/runtime setup` (`pnpm install --frozen-lockfile && pnpm run build`) **não suja** a árvore (testado, com
arquivos ignorados criados); um arquivo novo dentro de um desses caminhos suja (o git pode imprimir o diretório novo em vez do arquivo: `?? vendor/outro/`). A edição de um arquivo rastreado de cada
tipo que `ab.is_prod` reconhece (`Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/config.toml`, `vendor/**/src/*.rs`, `crates/*/src/*.rs`, `crates/*/build.rs`, `crates/*/Cargo.toml`) e a de `runtime/src/*.ts` sujam; a de
`docs/*.md`, um arquivo novo no topo e um ignorado, não. **O `is_measured` do A/B continua ignorando o `runtime` de propósito**: o executável que o A/B mede é o teste do core, que não carrega JS. São duas perguntas
diferentes: "o que o A/B compara" (o core e as entradas de build do workspace) e "o que torna uma execução suja" (tudo o que um build ou um veredito lê da árvore, o JS da janela incluído, porque `native.py` roda a
página): a identidade cobre mais que o A/B, e o teste confere que cobre **tudo** o que `is_prod` chama de entrada de build e que `is_measured` é um subconjunto de `is_prod`.

**R3: o que prova que a trava é uma só e que não conflita consigo mesma.**

- *Um nome.* `harness.OUT_LOCK` é a única constante; os quatro `OUT_LOCK` por runner e o parâmetro de nome de `guarded` saíram (um teste confere os dois). A recusa diz `another gizmo-perf run (any of run.py,
  ab.py, native.py or mutate.py, in any phase) holds <caminho>: it owns this --out, so `<runner recusado>` read and wrote nothing here (not waited for); use one --out per runner`, e **não** nomeia só o runner recusado.
- *Cada par ordenado de runners diferentes (12 pares).* O primeiro está dentro da fase dele (a trava está tomada); o segundo, iniciado pelo `main` dele sobre a mesma `--out` (com os arquivos de um dono
  plantados), imprime **uma** linha `REFUSED` com o nome da trava e o do segundo runner, sai com 1, **não chega** à fase dele (o teste pendura nela uma armadilha que falha se for chamada) e **não cria, muda
  nem apaga nada**; com o primeiro terminado a trava está livre e o segundo roda a fase dele (o controle).
- *Dois processos reais.* Um processo-ajudante segura `--out/.gizmo-perf.lock` por `harness.exclusive`; cada um dos quatro runners, iniciado como processo (`run.py --phase measure`, `ab.py --phase measure`,
  `native.py --phase run`, `mutate.py --phase time`: nenhuma fase que construa, meça ou abra janela), recusa com exit 1 e uma linha, e a `--out` fica como estava. A demonstração com os runners de
  `ca66012` está no log de controle.
- *Sem conflito consigo mesmo.* `guarded` é tomado uma vez por `main` (nunca aninhado) e o `mutate.py --phase prepare` chama o `build()` do `run.py` como função comum, sem `main` e sem trava: um teste fixa que a
  trava está **tomada** durante o build e que a execução termina em vez de recusar a si mesma.
- *O que a trava não faz.* **Exclui execuções simultâneas, não sequenciais.** A trava acaba com o processo (o arquivo fica, de propósito: apagá-lo abriria uma corrida de inode); um runner que começa depois que
  outro terminou ainda sobrescreve o `manifest.json` e o `exes/` dele. Por isso a regra "um diretório por runner" continua, e o porquê é o dos arquivos de mesmo nome (seção "CLI headless"). Nenhum runner confere
  que a `--out` é a certa para ele.

**R4: o `time` e a árvore de agora.** O executável de um mutante de tempo pode ser o do HEAD (o manifesto do `prepare` diz que a árvore estava limpa), mas a corrida controlada lê `budgets.json`,
a jornada e os arquivos do corpus **da árvore, quando roda** (`gizmo_perf_support::read`, um `std::fs::read_to_string` a partir de `CARGO_MANIFEST_DIR/../..`: `json_file(BUDGETS)` em
`scene_gizmo_perf_budget.rs`, `json_file(JOURNEY)` e o hash dos arquivos do corpus em `gizmo_perf_support/mod.rs`), e os runners que a julgam são os da árvore: uma edição feita depois do `prepare` muda o veredito, e nada percebia. A fase `time` agora lê `identity.dirty` na hora, **antes** de medir, mutar ou apagar qualquer coisa, e recusa
(`status: REFUSED` em `manifest.final.json`, exit 1) com um motivo próprio (a mensagem do `prepare` fala de executáveis construídos da árvore suja, e essa não seria verdade aqui). Testes (com a identidade
simulada; o que entra em `dirty` é o assunto do R2, com git real): para quatro tipos de árvore suja (um runner editado, `budgets.json`, `runtime/src`, um arquivo novo de `crates`), sobre um `prepare` **limpo**
registrado, o `time` recusa, mostra a primeira linha que o git imprimiu e o motivo (`budgets.json`), não mede nada, não toma o lock de performance, não apaga `exes/` e só cria o registro da recusa e o arquivo
da trava; o controle (a mesma `--out` com a árvore limpa) mede. Cada fase dá o seu motivo, e a árvore **de agora** é julgada **antes** do manifesto do `prepare` (um `prepare` sujo **e** uma árvore suja dizem a
razão da árvore; com a árvore limpa, a do `prepare` sujo aparece). A conferência do manifesto do `prepare` (`identity.dirty` vazio e registrado) fica, depois desta, porque é outra pergunta (o que o `prepare` viu).

**R5 e a varredura (c): as esperas limitadas do `native.py`.** `Window.settle` já dava a cada consulta o tempo que resta, conferia o prazo de novo depois da resposta e tratava `socket.timeout`/`TimeoutError` como
problema; a mesma regra vale agora para **toda** espera com prazo do `native.py`:

| Espera | Prazo | Antes | Agora |
|---|---|---|---|
| `overlay_ack` (reconhecimento do overlay do próprio passo) | `ACK_SECONDS` (5 s) | `viewport()` com 90 s por consulta; um reconhecimento que casa mas chega depois dos 5 s era aceito | tempo que resta por consulta, prazo conferido depois da resposta (**tarde = timeout**, com o orçamento no texto), timeout gravado como problema, ao menos uma consulta (com `QUERY_FLOOR_SECONDS` = 0,05 s se não resta tempo). **Rodada 10: (a) já recusa uma resposta tardia**, fixado por `BoundedPolls.test_a_matching_acknowledgement_that_arrives_after_the_budget_is_a_timeout_not_an_acknowledgement` (tirar o ramo derruba esse teste) |
| diálogo (`page_hidden_by_dialog`, passo `present.dialog.open`) | 3 s | `viewport()` com 90 s por consulta; `hidden` visto depois dos 3 s valia | o mesmo, e o timeout é um check que falha com o motivo, não um traceback. **Rodada 10: (a) já recusa uma resposta tardia**, fixado por `BoundedPolls.test_a_dialog_wait_whose_monitor_never_answers_or_answers_late_is_a_problem_not_a_pass` |
| `job.get` (`commit_job`, os passos de job) | 60 s (`JOB_SECONDS`) | `op(job.get)` com 90 s por consulta; desde a rodada 8, o tempo que resta por consulta, **mas sem conferir o prazo depois da resposta**: um `succeeded` que voltasse depois dos 60 s era aceito e o `job.commit` era enviado depois do prazo (review 5861824134) | cada consulta recebe o tempo que resta; um timeout é um check que falha (`the job finished`, com o motivo). **Rodada 10: (b) corrigido**: o relógio é lido de novo quando o `window.op` volta (`answered > deadline`, estrito) e uma resposta tardia **não vale, qualquer que seja o estado dela**, sem `job.commit`; o check `the job finished` falha dizendo que ela chegou depois dos 60 s e foi tarde demais para contar. `LateJobAnswer` (6). O relógio é `time.time()`: item (35) |
| `Window.settle`, `Window.settle_commit` | `seconds` de cada chamada (`COMMIT_SECONDS`, 30 s, na soltura) | (já limitados na rodada 6) | sem mudança no prazo; ver (d). **Rodada 10: os dois (a) já recusam uma resposta tardia.** `Window.settle`: `NativeSession.test_a_reply_that_arrives_after_the_deadline_is_a_timeout_that_keeps_what_it_showed`. `Window.settle_commit` (`now > deadline`): **nenhum teste antigo a fixava** (a mutação que a tira sobrevivia a todos); a classe nova `LateCommitReply` (2) a fixa, e o código não mudou |
| as demais chamadas `window.<call>()` do `native.py` | (nenhum outro laço com prazo: `rg -e 'while ' -e deadline`) | uma chamada só, com o padrão de 90 s e o `CORPUS_TIMEOUT_S` do corpus inteiro | **sem mudança**: não há laço nem prazo a quebrar (varredura de leitura). **Rodada 10: (c) não se aplica**: uma chamada única não tem prazo próprio contra o qual uma resposta possa ser tardia (o `timeout` do socket limita cada `connect` e cada `recv` do laço de leitura de `send`, não a duração da chamada inteira, que é **gravada** em `client_ms_*` e não julgada; o servidor local responde uma linha só); o único prazo sobre elas é o do corpus, na linha seguinte |
| `Window.check_deadline` (o corpus inteiro) | `CORPUS_TIMEOUT_S` (1200 s) | conferido **antes** de cada `send` e de cada consulta de `settle` e `settle_commit` | **Rodada 10: (c) não se aplica**: é o orçamento do corpus, não a espera de uma resposta; passado dele, a chamada seguinte lança `Timeout` e o check `the journey ran to the end` falha, e uma resposta que atravessa os 1200 s continua sendo julgada pela regra do seu próprio passo. **Nenhum teste da spec cita `check_deadline` nem `CORPUS_TIMEOUT_S`**: item (34) |
| `harness.run_bounded` (os filhos: builds, medições, testes, `git rev-parse HEAD:runtime`) | o `timeout` de cada chamada | grupo do filho terminado ao fim do prazo (já era assim) | **Rodada 10: (c) não se aplica**: vencido o prazo, o grupo recebe SIGTERM e depois SIGKILL (`timed_out` fica verdadeiro e o que foi impresso é guardado): não há "resposta que chega tarde" para aceitar. Como cada chamador julga `timed_out` **não foi relido** nesta varredura (a spec tem testes próprios do `run_bounded`) |

**Rodada 10 (F2): o balanço da varredura (c).** (a) **já recusam** uma resposta tardia: `overlay_ack`, `page_hidden_by_dialog`, `Window.settle` e `Window.settle_commit` (os quatro têm agora um teste nomeado que cai se o ramo for tirado; o do `settle_commit` é novo). (b) **corrigido agora**: `commit_job`. (c) **não se aplicam**: as chamadas únicas, o orçamento do corpus e os filhos de `run_bounded`. Nenhuma outra espera com prazo existe em `native.py` (`rg` por `deadline`, `timeout`, `while ` e `time.time()`/`monotonic`, lido linha a linha).

**Varredura (d): `settle_commit` e o reconhecimento tardio.** O desfecho `deduped` exige o reconhecimento da página **dentro de `ACK_SECONDS` do primeiro quadro do candidato**; uma consulta lenta não diz **quando**
o reconhecimento chegou, só quando foi **visto**. Agora o runner guarda o instante da primeira leitura que viu o reconhecimento que casa (`acked_at`; o zerar de `acked_at` quando o reconhecimento **deixa de casar** é observável e a rodada 9 o fixou num teste,
e o zerar junto com `first_at` quando o quadro deixa de convergir é **defensivo e não observável**, um mutante equivalente: ver "Rodada 9") e, se esse instante passou de `first_at + ACK_SECONDS`, o desfecho é `unconfirmed` (reprova), com o orçamento no texto e dizendo que o instante da chegada é desconhecido; um
reconhecimento visto **dentro** do orçamento continua dentro, qualquer que seja o custo das leituras seguintes, e um reconhecimento que nunca casa continua dando "never acknowledged" aos 5 s.

**Varredura (b): o que cada fase que julga contra arquivos da árvore faz com uma árvore **atual** suja** (lido no código de `ca66012` e confirmado de novo depois das correções):

| Fase | Sem `--allow-dirty` | Com `--allow-dirty` |
|---|---|---|
| `run.py --phase build` | `REFUSED` | constrói e grava `dirty: true` em `build.json` |
| `run.py --phase measure` | `REFUSED`; também recusa um `build.json` que diga sujo ou não diga limpo | grava `dirty_run`; o que seria `PASS` vira `DIRTY_RUN_NOT_A_PASS_FOR_HEAD` (item (13)(a)) |
| `ab.py --phase build` e `--phase measure` | `REFUSED` (a conferência vem antes da separação das fases); o `measure` também recusa executáveis construídos sujos | grava `dirty_run`; o status **não** muda (item (13)(a)) |
| `native.py --phase build` | `REFUSED` | grava `dirty` do build |
| `native.py --phase run` | `REFUSED` (árvore) e recusa um binário construído sujo | `dirty_run`; status `DIRTY_RUN_NOT_FOR_HEAD` (exit 1) |
| `mutate.py --phase prepare` | `REFUSED` | (não existe a flag) |
| `mutate.py --phase time` | **em `ca66012`: só olhava o que o `prepare` gravou** (o furo do R4); agora `REFUSED` | (não existe a flag) |

Nenhuma outra fase aceitava uma árvore suja sem registrar: só o `time` do `mutate.py`, que o R4 corrigiu. O que a varredura **não** fecha é o conjunto de entradas que a identidade olha: ver o item (29) abaixo.

**Registrados como itens novos de BL015.23, sem correção** (maiores que esta rodada ou sem prova): **(29)** a identidade (`dirty`) olha os nove caminhos de `DIRTY_PATHSPECS`, mas um build lê **outras** entradas em
tempo de compilação: `assets/demo/coast.png` e `highlands.png` (`include_bytes!` em `crates/desktop/src/workstation/media.rs`, linhas 11 e 12) e as fixtures de bundle `docs/migration/fixtures/bundles/*.bundle.json` e `docs/migration/fixtures/pilot-fx/*.json` (`include_str!` em módulos de teste do core e do desktop e em testes de
integração do core): uma edição local nelas não suja a identidade e o executável construído seria arquivado sob o HEAD (lido nos `include_*!`; **não testado**); **(30)** os builds (`./scripts/cargo`, o `mbx` externo) ainda herdam `GIT_*` do chamador:
não há prova de que algum passo leia o git, então `child_env` não foi mudado. Aceite de cada um em BL015.23.

**R7 (doc): o que o `ScenePlan` carrega.** A frase "o `ScenePlan` não carrega revisão nem id de gesto" estava imprecisa. O que o código tem: `PlanSource` (`crates/core/src/scene_plan/model.rs`, linhas 213 a 218) tem
`revision_id`, que `build.rs` (linha 352) copia de `bundle.revision_id` e que o JSON do plano serializa (`json.rs`, linha 126): é a revisão **do bundle**, um texto do arquivo do bundle (`directed_scene/parse.rs`), **não** um
contador de edições nem um id de gesto; `PlanResource.revision_id` é a de cada asset. Nenhum outro campo do `ScenePlan` (`version`, `source`, `output`, `frame`, `camera`, `animation`, `nodes`, `layers`, `splats`, `fx`,
`resources`) conta edições ou identifica um gesto (lido). Que a chave de dedupe do host (cena, plano em JSON, mapa de recursos, opções) do plano do commit seja **igual** à do candidato é o que os registros de
`12bc8da` **mostram** (nas 15 soltas o host não desenhou nada, e o caminho do código que explica isso é a chave igual) e é coerente com o código; **a igualdade não foi provada só pela leitura do código**. As duas cópias
da frase (a leitura de H16 acima e o item (16) de BL015.23) foram reescritas; não havia outra no repositório (`rg 'carrega revis'` em `docs` e `scripts`).

**R8 (doc): o README da evidência.** A última linha da tabela de achados do `docs/migration/gizmo-perf-evidence/README.md` dizia que (16), (17) e (20) estavam "não tratados"; a rodada 7 corrigiu o runner para (16) e
(17) (sem reexecutá-lo numa janela), então a linha foi dividida: (20) não tratado; (16) e (17) "runner corrigido na rodada 7, não reexecutado numa janela, a validar". Nenhuma outra tabela ou parágrafo do README
diz que (16) ou (17) estão sem tratamento (`rg`).

**Prova desta rodada (só dublês, git e processos reais que recusam: sem benchmark, sem janela, sem build e sem lock de performance).** A spec passou de 221 para **252 testes** (`gizmo-perf.spec.py`, 52,5 s):
`GitEnvironment` (5), `DirtyIdentity` (6), `OneLockPerOut` (3), `TwoRunnerProcesses` (1), `BoundedPolls` (9) e `LateAcknowledgement` (3), mais três testes de `MutantExecutables` (a árvore suja do `time`, a razão de cada
fase e a trava tomada durante o `prepare`) e um de `Baseline` (um `GIT_DIR` hostil), **31 no total**; os testes das rodadas 4 a 7 que nomeavam uma trava por runner (`.mutate.lock`, `.ab.lock`, `.run.lock`,
`.native.lock`) ou o `guarded` antigo foram **reescritos de propósito** para o nome único (10 deles falham nos runners de `ca66012` só por isso, listados à parte no log). Contra os runners de `ca66012`, **26 dos 31 testes
novos falham** (5 passam dos dois lados de propósito, descritos no log) e nenhum dos 211 testes anteriores que não foram reescritos falha. **14 versões erradas das correções** são pegas, cada uma por pelo
menos um teste. O log traz ainda a demonstração com **dois processos reais** (os quatro runners, cada um recusado com uma linha e exit 1, e os runners de `ca66012` **não** recusados pelo mesmo ajudante), em
[`spec-new-tests-vs-ca66012-runners.log`](../migration/gizmo-perf-evidence/spec-new-tests-vs-ca66012-runners.log). Sem Rust alterado, os gates do workspace de `081353e` (2299 passaram, 0 falharam, 16 ignorados;
`cargo tree` sem gpui/wry) não foram repetidos.

**O que continua limitado.** A trava de `--out` exclui execuções simultâneas e **não** as sequenciais (um diretório por runner continua sendo regra de quem roda). O `dirty` olha os nove caminhos declarados e **não**
as outras entradas de compilação (item (29)). Os builds herdam `GIT_*` (item (30)). A espera do `job.get` e a do diálogo foram limitadas por leitura e por dublês: **nenhuma foi vista numa janela**. O `deduped`
continua dependendo de um reconhecimento da página que **nunca foi visto** numa janela, e agora também de ele ser **visto** em `ACK_SECONDS` (numa máquina carregada pode ser uma falha falsa, lida como falha, nunca
ignorada). Seguem como estavam: os itens (13)(a), (14), (20), (23) a (28) e a falta de baseline do host e da janela. **O efeito dessas correções sobre uma execução real ainda não foi visto.**

**O que esta rodada NÃO fez:** medir tempo; rodar qualquer runner de verdade além dos processos que recusam na trava; abrir a janela ou construir release; mudar `budgets.json`, jornada, corpus, golden, fixtures,
margens, limiares ou qualquer Rust; decidir os itens (14) e (20); tratar o aviso "Docstring Coverage"; responder ou resolver threads da review. **Nenhuma afirmação de desempenho vale: implementado não é validado.**

## Rodada 9 (PR #44, de `920d3bb` a esta entrega): duas regras de `settle_commit` que nenhum teste fixava

Escopo: **só a spec (`scripts/gizmo-perf/gizmo-perf.spec.py`) e `docs/**`**, mais o log de controle. **Nenhum runner mudou** (`native.py` e os demais são, byte a byte, os de `920d3bb`), nem Rust, runtime, fixture, `budgets.json`, jornada,
corpus, golden, margem ou limiar, e **nada foi medido**: nenhum `run.py`, `ab.py`, `mutate.py` ou `native.py` rodou, nenhuma janela abriu, nenhum build foi feito e [`measure-12bc8da/`](../migration/gizmo-perf-evidence/measure-12bc8da/README.md)
não foi editado. A rodada nasceu de uma verificação do coordenador sobre `920d3bb`: três mutações de uma linha de `Window.settle_commit` sobreviviam às 18 provas das classes `LateAcknowledgement` e `Release`. Esta rodada reproduziu isso em cópias dos runners,
**fixou as duas que podem ser mortas** com testes novos e registrou a terceira como **equivalente**. Testes e log: `292e398`. **Implementado não é validado**: nada nesta seção diz que o desempenho foi validado nem que não há regressão.

| Mutação (uma troca de texto em `native.py`, numa cópia) | O que ela muda | As 18 provas anteriores | Teste novo (classe `LateAcknowledgement`) | Resultado |
|---|---|---|---|---|
| **Sd-b**: `if acked_at - first_at > ACK_SECONDS:` por `if now - first_at > ACK_SECONDS:` | o prazo do reconhecimento passa a ser julgado no instante da leitura **de agora**, não no da que o viu **primeiro** | passam (a mutação sobrevive) | `test_an_acknowledgement_seen_with_the_picture_is_in_time_even_when_the_slow_reads_that_follow_pass_the_ack_budget` | passa nos runners de `920d3bb`; **falha** na Sd-b; o outro teste novo passa na Sd-b |
| **Sd-c**: apagar `acked_at = None` do ramo `if seen['ack_problems']:` | um reconhecimento que deixou de casar conserva o instante em que foi visto casar pela primeira vez | passam (a mutação sobrevive) | `test_an_acknowledgement_that_matched_then_stopped_and_matched_again_after_the_budget_is_unconfirmed_not_deduped` | passa nos runners de `920d3bb`; **falha** na Sd-c; o outro teste novo passa na Sd-c |
| **Sd-d**: `first_at = acked_at = None` por `first_at = None` (ramo "o quadro não convergiu") | `acked_at` conserva um valor velho | passam | **nenhum, de propósito** | **mutante equivalente**: nada o distingue do código real (abaixo) |

**Sd-b: um reconhecimento visto junto com o quadro está no prazo, por mais lentas que sejam as leituras seguintes.** A regra da varredura (d) da rodada 8 julga o reconhecimento pelo instante da leitura que o **viu primeiro**
(`acked_at - first_at`), porque uma leitura lenta não diz quando ele chegou, só quando foi visto. A docstring de `settle_commit` diz o corolário ("um reconhecimento que casa visto a tempo continua a tempo, qualquer que seja o custo
das leituras seguintes"), mas o único teste dele (`ack_delay=3,0`, leitura de 2 s) não distinguia as duas regras: ali a leitura que viu o reconhecimento é também a de agora, e as duas distâncias dão os mesmos 2 s. O teste novo usa leituras
de 6 s (mais que os 5 s): o quadro e o reconhecimento que casa já aparecem na primeira leitura, a segunda termina 6 s depois do primeiro quadro, e o desfecho é `deduped`, sem problema (duas leituras; a espera inteira passou do orçamento e isso
não é atraso). A Sd-b responde `unconfirmed`. **O que ele protege:** que leituras lentas (uma máquina carregada) transformem um `deduped` legítimo em `unconfirmed`, uma **falha falsa** (lida como falha, nunca como aprovação). O diferencial mostra o
sentido: a Sd-b só muda desfechos de `deduped` para `unconfirmed`, nunca o contrário. **O que ele não cobre** (a rodada 8 já cobre): uma leitura lenta que só **vê** o reconhecimento depois do orçamento nunca vira `deduped`.

**Sd-c: um reconhecimento que deixa de casar recomeça.** Quando uma leitura mostra um reconhecimento que **não** casa (a página ainda mostra a revisão anterior), `acked_at` volta a `None`: só o **próximo** casamento conta, e é esse instante, medido
desde o primeiro quadro do candidato, que é comparado ao orçamento. Cenário, como (custo; quadro do candidato convergido; reconhecimento casa): (0,01 s; sim; sim), (2,0 s; sim; **não**), (3,5 s; sim; sim). O reconhecimento voltou a casar 5,52 s depois do primeiro
quadro, depois dos 5 s, e o runner responde `unconfirmed` (`the page acknowledged the committed state only 5.5 s after the candidate's picture was first seen, past the 5 s budget`; três leituras, a terceira é a que decide). A Sd-c, que guarda o
primeiro instante (0 s), responde `deduped`. **O que ele protege:** a **aprovação falsa**: o que a primeira leitura viu não é evidência do que a página mostra depois da lacuna. O diferencial mostra o sentido: a Sd-c só muda desfechos de
`unconfirmed` para `deduped`.

**Sd-d: por que é equivalente e por que não tem teste.** Um `acked_at` velho vem de uma observação **anterior** ao novo `first_at` (o ramo é tomado numa leitura sem o quadro do candidato, e `first_at` é refeito numa leitura **posterior**), então
`acked_at - first_at` é negativo e nunca dispara a verificação de atraso (no código real vale 0, que também não dispara); e uma leitura cujo reconhecimento **não** casa zera `acked_at` antes de qualquer casamento posterior poder usá-lo, de modo
que as duas versões se igualam daí em diante. Nenhuma sequência de leituras as distingue. O diferencial (script descartável, fora do repositório, **não** na spec) conduziu **60 000 sequências aleatórias** (custo de leitura de 0,01 a 6 s, quadro convergido
ou não, reconhecimento que casa ou não, `quiet` e `seconds` variados) por `settle_commit` de cada cópia, com relógio virtual: a Sd-b mudou o desfecho de **2575** (todas `deduped` para `unconfirmed`), a Sd-c de **1206** (todas `unconfirmed` para `deduped`) e a
Sd-d de **0** (nem desfecho, nem texto do problema, nem tempo gravado, nem número de leituras). É evidência, não prova; a prova é o argumento acima. **O zerar de `acked_at` nesse ramo é defensivo e fica** (o código não foi tocado); o que zera `acked_at` quando o
reconhecimento **não casa** é observável e agora está fixado. A frase da varredura (d) da rodada 8 que dizia que `acked_at` é zerado "quando o quadro deixa de convergir" foi corrigida (era a única cópia: `rg` em `docs`, `scripts` e nos docstrings dos runners).

**Achados da review (2B): nenhum nesta rodada.** O coordenador pediu uma review de `920d3bb` e o CodeRabbit respondeu "Review rate limited" (nenhuma review foi feita; informação do coordenador, não verificada aqui). A review seguinte será pedida depois do
push desta rodada, sobre o head final, e se tiver achados válidos vira uma rodada própria.

**Prova (só dublês, relógio virtual e cópias descartáveis; sem benchmark, sem janela, sem build e sem lock de performance).** A spec passou de 252 para **254 testes** (`gizmo-perf.spec.py`; 254/254 passam na árvore **limpa** de `292e398`, 48,3 s): os dois
testes novos, um monitor roteirizado (`ScriptedReads`: uma leitura por entrada do roteiro, com custo, quadro convergido ou não e reconhecimento que casa ou não, no relógio virtual; uma leitura que o roteiro não tem é um erro, nunca a repetição da última) e um
comentário de rodada. O [log de controle](../migration/gizmo-perf-evidence/spec-round9-ack-tests-vs-mutants.log) traz, por variante (os runners reais, Sd-b, Sd-c e Sd-d: cada uma uma cópia por `git archive` de `920d3bb` fora do worktree, com uma troca exata de texto e o diff
dela), as 18 provas antigas, a spec nova nas duas classes e cada teste novo sozinho, mais, numa linha cada, o diferencial e uma **varredura descartável de 86 mutantes** de `settle_commit` (trocas de operador e de literal, remoção de cada instrução, cada condição forçada a
verdadeira e a falsa; contra as quatro classes da spec que derivam de `GestureCase`): **86 mortos, 0 sobreviventes** (um deles não termina e foi parado à mão: um travamento é uma execução que falhou). A varredura **não troca variáveis** (a Sd-b é desse tipo e foi conferida à
mão), então mostra nenhuma lacuna conhecida, não que não haja. Sem Rust alterado, os gates do workspace de `081353e` (2299 passaram, 0 falharam, 16 ignorados; `cargo tree` sem gpui/wry) não foram repetidos.

**O que continua limitado.** Os dois testes rodam sobre um monitor roteirizado com relógio virtual: **nenhuma janela mostrou** quando a página reconhece de verdade, nem o custo real de uma leitura; o `deduped` segue dependendo de um reconhecimento da página que **nunca foi visto** numa janela,
e de ele ser **visto** em `ACK_SECONDS`, como na rodada 8 (BL015.23 (16), a validar numa janela real). Nenhum item novo de BL015.23 nasceu desta rodada: o que fica aberto é o que o (16) já registra. Seguem como estavam: os itens (13)(a), (14), (20), (23) a (30) e a falta de baseline do host e da janela.

**O que esta rodada NÃO fez:** medir tempo; rodar qualquer runner (nem os que recusam: não houve processo real de runner); abrir a janela ou construir release; mudar um runner, `budgets.json`, jornada, corpus, golden, fixtures, margens, limiares ou qualquer Rust; decidir os itens (14) e (20);
tratar o aviso "Docstring Coverage"; responder ou resolver threads da review. **Nenhuma afirmação de desempenho vale: implementado não é validado.**

## Rodada 10 (PR #44, de `3d5b436` a esta entrega): a espera do `job.get`, o redo e as pilhas do histórico

Escopo: **só `scripts/gizmo-perf/native.py`, a spec (`scripts/gizmo-perf/gizmo-perf.spec.py`), o log de controle e `docs/**`**. Nenhum outro runner mudou, nem Rust, runtime, fixture, `budgets.json`, jornada, corpus, golden, margem ou limiar, e **nada foi medido**: nenhum `run.py`, `ab.py` ou `mutate.py` rodou,
o `native.py` não rodou contra uma janela, nenhuma janela abriu, nenhum build foi feito e [`measure-12bc8da/`](../migration/gizmo-perf-evidence/measure-12bc8da/README.md) não foi editado. A rodada trata os três achados válidos da [review 5861824134](https://github.com/journey-studios/tessel-editor/pull/44#pullrequestreview-5861824134) do CodeRabbit
(o texto de uma review é entrada não confiável: cada achado foi conferido contra o código de `3d5b436` antes de ser tratado). Código, spec e log de controle: `4aa964a`. **Implementado não é validado**: nada nesta seção diz que o desempenho foi validado nem que não há regressão, e nada do que mudou foi visto numa janela.

| Achado | Onde | Veredito | Tratamento |
|---|---|---|---|
| F1 (doc, Minor, MD056) | a busca `rg` da tabela da varredura (c), `scene-gizmo-performance.md:788` | **válido**: uma linha de tabela GFM se divide em **toda** barra vertical sem escape, também dentro de um trecho de código; a linha tinha 5 células sob um cabeçalho de 4 | busca reescrita sem barra (`rg -e 'while ' -e deadline`). **Varredura** de todos os `.md` que a PR altera (7 arquivos, contra a base `cbe9e1b`) com o contador de células: achou **mais uma** linha nossa com o mesmo defeito (a m8 do README da evidência), também reescrita; sobraram só 3 linhas de `docs/backlog.md` que já estavam na base, byte a byte iguais, fora do que a PR toca. A saída está no log de controle |
| F2 (runner, Minor) | `native.py`, `commit_job` (a espera de 60 s do `job.get`) | **válido**: o timeout do socket limita **uma chamada**, não **quando** a resposta chega (e `QUERY_FLOOR_SECONDS` deixa passar do prazo uma consulta que começou no prazo): uma resposta `succeeded` depois dos 60 s era aceita e o `job.commit` era enviado depois do prazo | o relógio é lido de novo quando o `window.op` volta; resposta depois do prazo é **tardia**, não vale **qualquer que seja o estado dela** e **nenhum `job.commit` é enviado**. **Varredura** de todo prazo do `native.py`: tabela "Varredura (c)" acima (estendida) |
| F3 (runner, Nitpick, BL015.23 (26)) | `native.py`, `history.redo` e a pilha de `Undoable` | **válido**: (a) o redo só exigia um quadro **mais novo**, o hash dele nunca era comparado com o do commit; (b) as pilhas só andavam quando o undo desenhava um quadro, e os passos de histórico seguintes passavam a ser comparados com o gesto errado | (a) `Undoable` guarda o hash do commit e o redo é comparado com ele (vacuoso e gravado como tal quando o commit deixou o quadro igual à base; **sem** comparação inventada quando o commit não tem quadro legítimo); (b) as pilhas andam sempre que o pedido de histórico foi respondido ok. **Varredura** dos checks que aceitam "um quadro novo": tabela abaixo; o que sobrou virou os itens (31) a (35) de BL015.23, **sem correção** |

**F2: uma resposta do `job.get` depois dos 60 s não vale.** `commit_job` (a espera de um job enfileirado, nos passos `*.resources`) dava a cada consulta o tempo que resta (rodada 8) mas só conferia o prazo **antes** de consultar: uma consulta
começada aos 59,9 s e respondida aos 61 s com `succeeded` era aceita, e o `job.commit` saía depois do prazo. Agora o relógio é lido de novo quando o `window.op` volta (`answered > deadline`, estrito, como no `Window.settle`): a resposta é **tardia**
e **o estado dela não é lido** (nem um final, nem um `running`); nenhum `job.commit` é enviado e o check `<passo>: the job finished` falha, com um detalhe que diz que uma resposta do `job.get` chegou N s depois do início da espera, depois do prazo de 60 s, tarde demais para contar (o estado dela não foi usado e nenhum `job.commit` foi
enviado). Uma resposta **dentro** do prazo se comporta exatamente como antes (mesmos checks, mesmos detalhes, um só `job.commit`), e uma que chega **exatamente** no prazo (60,0 s) ainda está dentro. Os 60 s viraram a constante `JOB_SECONDS`.
O prazo é da **espera inteira** (uma consulta de 31 s seguida de outra de 30 s passa dele), não de cada consulta.

**F3 (a): o redo é comparado com o quadro do commit.** Antes, depois de um `history.redo` que desenhou um quadro, o runner só conferia que o quadro era **mais novo** (`the redo drew a NEW picture`): o hash era gravado e nunca comparado
(a gravação de `12bc8da` tem, nos três corpos, o hash do redo igual ao do commit, mas isso era **lido por uma pessoa**, não conferido pelo runner). Agora `Undoable` é `(base_hash, changed, step, committed_hash)`: o `drag` guarda o hash do quadro que a soltura deixou
(`pixels['committed_hash']`, `None` quando a soltura não teve desfecho legítimo) e, depois de um redo cujo quadro **convergiu**, o runner compara o hash do quadro visto com o do commit: um check de verdade, `<passo>: the redo showed the exact picture the commit left`, com os dois hashes no
detalhe (visto, do commit). Quando o commit não mudou nenhum pixel (`changed` falso: a escala do `dense`), o check continua perguntado e é gravado como **vacuoso** (`vacuous_checks` do passo e `vacuous (...)` no detalhe), como o do undo; quando o commit **não tem** hash (a soltura
não teve desfecho legítimo e os checks dela já reprovaram) **não se inventa** uma comparação: o passo grava `unasked_checks` com o motivo.

**F3 (b): as pilhas seguem o documento.** `redoable` e `undoable` só andavam dentro de `if before_hash is not None and undoable and ms is not None`: um undo respondido ok mas que **não desenhou** quadro deixava as pilhas onde estavam (o check `drew a NEW picture` já reprovava, mas os
passos seguintes passavam a ser comparados com o gesto errado). Agora as pilhas andam **sempre que o pedido foi respondido ok e há o que mover**: o undo leva o gesto do topo de `undoable` para `redoable` e o redo o devolve; só as comparações de pixels dependem de um quadro ter convergido. Um pedido que **não** foi respondido ok não move nada.
O `before_hash` segue sendo só o portão do undo (item (32)).

**Varredura de F3: todo `check(...)` do corpo da jornada que aceita "um quadro novo", "convergiu" ou "mudou" sem comparar hash nem valor** (lida em `native.py` da rodada 10; cada rótulo é o texto do check):

| Check (rótulo) | O que aceita | Compara hash ou valor? | Check mais estrito, ou por que não pode haver |
|---|---|---|---|
| `present.resize.*: the monitor converged after the resize` | qualquer quadro convergido depois do `ui set window size` | não; o passo só grava `settle_ms` (a gravação de `12bc8da` não tem quadro nenhum nesses passos) | **nenhum em outro lugar**. Poderia comparar com o quadro lido antes de redimensionar, mas nenhum registro diz que o quadro de um monitor de outro tamanho muda de hash: **item (33)**, sem correção |
| `open.first_frame` (`a NEW picture of frame N was drawn after the seek`) | um quadro novo, convergido, do quadro N da jornada | não; o hash vai para `first_picture_hash`, sem comparação | **não pode ser mais estrito**: não há quadro de referência absoluto (nenhum golden de pixels; o hash depende da GPU e do driver, e a janela não tem baseline, BL015.23 (14)). Os checks relativos estão nos passos `drag.*` e `history.*` |
| `*.show` (`<cena> drew a NEW picture (newer than request N)`) | um quadro mais novo que o pedido **da cena pedida** | a cena, sim (`info['scene'] == wanted`); o quadro, não | **não pode ser mais estrito sem conhecer o corpo**: nos três corpos de `12bc8da` o hash de `same.show` (outra cena do mesmo bundle) é **igual** ao de `switch.show` (outro bundle), então "tem de diferir do quadro anterior" reprovaria o `same.show` gravado |
| `history.*: the undo/redo drew a NEW picture` | um quadro mais novo que o pedido | não | **existe, logo abaixo**: o undo compara o quadro com o de **antes do gesto** (`the undo restored the exact picture of before the gesture`) e, **desde esta rodada**, o redo com o do **commit** (`the redo showed the exact picture the commit left`). Onde elas não comparam (gesto que não mudou nada: vacuoso, gravado; undo sem `before_hash`: **item (32)**; commit sem quadro legítimo: `unasked_checks`), este check é o único |
| `drag.*: the candidate was drawn while the gesture was open` | um quadro mais novo que o press, em até 2 s | não | **existe**: `the drawn candidate picture differs from the base picture` (hash do candidato diferente do da base) nos passos que **têm** de mudar; nos que podem ficar iguais o passo grava os três hashes (`pixels`) e `the redrawn picture shows the same pixels as the candidate (same hash)` compara |
| `drag.*: the release left a legitimate picture` | `redrawn` (quadro mais novo que o do candidato) ou `deduped` | o `deduped` compara o hash e o reconhecimento dentro de `settle_commit`; o `redrawn`, só "mais novo" | **existe**: `the redrawn picture shows the same pixels as the candidate (same hash)` (vacuoso quando o candidato era a base) e, nos passos que têm de mudar, `the committed picture differs from the one before the gesture`; nos outros o hash do commit é gravado (`committed_hash`) e é por ele que o redo (F3) compara |
| `drag.*: the committed picture is a NEW picture (the candidate was never drawn …)` | um quadro mais novo que o press | não | **é o ramo de reserva** (o candidato não foi desenhado e o check dele já reprovou); nos passos que têm de mudar `the committed picture differs from the one before the gesture` compara com a base; sem candidato, nada mais estrito pode ser dito |
| `drag.*: the cancel drew a NEW picture` e `the monitor converged after the cancel` | um quadro mais novo que o gesto (ou só convergido, se o candidato não foi desenhado) | não | **existe**: `after the cancel the picture is the base picture` compara o hash com o da base (vacuoso, e gravado, quando o candidato desenhado já era a base) |

Os demais `check(...)` do corpo comparam **valores** (respostas ok, ids de pedido, o handle, `near()` das transformações, o trace do golden, a profundidade das pilhas, o reconhecimento do overlay com ferramenta, espaço, seleção, cena, revisão e quadro) e não aceitam "um quadro novo" no lugar de uma comparação.
**Itens novos de BL015.23, sem correção** (a regra desta rodada é corrigir só o (26) no código): **(31)** `redoable` nunca é esvaziada quando um gesto novo é commitado, embora o core esvazie o redo (`crates/core/src/state/history.rs`, `record`); a jornada de hoje só faz `history.redo` logo depois do undo do `translate`, antes do
`drag.rotate.local.y`, então nenhum veredito errado nasce disso, mas é um desvio entre o modelo do runner e o documento; **(32)** o `before_hash` é lido nos dois pedidos de histórico mas só serve de portão do undo: se for `None` a comparação do undo é pulada **sem registro** (o redo registra o motivo em `unasked_checks`) e, com o hash do commit agora guardado, ele poderia ser comparado (o quadro de antes de um undo deveria ser o do commit; o de antes de um redo, o da base); **(33)** o check do resize; **(34)** **nenhum teste da spec cita
`check_deadline` nem `CORPUS_TIMEOUT_S`**: o orçamento de 1200 s do corpus não é fixado por teste (achado da varredura de F2; o código é lido, não testado); **(35)** `commit_job` mede o prazo com `time.time()` (relógio de parede), as outras esperas com `time.monotonic()` ou `time.perf_counter()`: um ajuste do relógio do sistema durante os 60 s encurtaria ou esticaria a espera, e o relógio virtual da spec não distingue os três.

**Prova (só dublês, relógio virtual, a gravação de `12bc8da` só lida e cópias descartáveis; sem benchmark, sem janela, sem build e sem lock de performance).** A spec passou de 254 para **285 testes** (`gizmo-perf.spec.py`; 285/285 passam na árvore **limpa** de `4aa964a`, 49,5 s): 31 testes novos em seis classes — `UndoableCommit` (4: o hash do commit que o `drag` guarda), `RedoPicture` (8: o redo num modelo de host de um gesto com relógio virtual), `RecordedRedo` (5: o replay, abaixo), `HistoryStacks` (6: vários gestos, com um `drag` de mentira que só empilha `Undoable` e um monitor de histórico roteirizado), `LateCommitReply` (2) e `LateJobAnswer` (6). O modelo de host (`HostWindow`)
ganhou o `history.redo` e a escolha, por chamada, de desenhar ou não o quadro do undo e do redo. O [log de controle](../migration/gizmo-perf-evidence/spec-new-tests-vs-3d5b436-runners.log) traz o que segue.

- **Contra os runners de `3d5b436`** (`git archive` do commit inteiro fora do worktree): a spec antiga dá 254/254; a spec nova dá `failures=30, errors=9` e **26 dos 31 testes novos falham** (`HistoryStacks` 6 de 6, `UndoableCommit` 4 de 4, `RedoPicture` 7 de 8, `RecordedRedo` 4 de 5, `LateJobAnswer` 5 de 6, `LateCommitReply` 0 de 2); **nenhum dos 254 anteriores falha**. Os 5 que passam dos dois lados são controles, descritos um a um no log: os 2 de `LateCommitReply` (a recusa de uma resposta tardia do `settle_commit` **já existia**; eles a fixam, e o segundo é o controle contra recusar demais), `LateJobAnswer.test_an_answer_that_arrives_in_time_behaves_exactly_as_before`
  (o comportamento dentro do prazo não mudou), `RecordedRedo.test_the_report_holds_what_the_comparison_is_built_from` (a gravação tem os hashes de que o replay parte) e `RedoPicture.test_a_redo_that_drew_no_picture_fails_only_its_own_check_and_is_not_compared_with_a_stale_picture` (um redo sem quadro já falhava só o próprio check; o teste guarda que a comparação nova não o compara com um quadro velho: a versão errada W12 o derruba). Contra os runners novos: 285/285.
- **Versões erradas** (cada uma é o `native.py` novo, numa cópia fora do worktree, com **uma** troca exata de texto, rodada contra as 14 classes que cobrem as correções e as varreduras; 92 testes cada): **todas as 19 derrubam pelo menos um teste**. As quatro últimas (S1 a S4) são as varreduras dos prazos que **já recusavam** uma resposta tardia: tirar o ramo de cada um é pego por um teste nomeado
  (S1 `BoundedPolls.test_a_matching_acknowledgement_that_arrives_after_the_budget_is_a_timeout_not_an_acknowledgement`; S2 `NativeSession.test_a_reply_that_arrives_after_the_deadline_is_a_timeout_that_keeps_what_it_showed`; S4 `BoundedPolls.test_a_dialog_wait_whose_monitor_never_answers_or_answers_late_is_a_problem_not_a_pass`; S3 só é pego pelo teste **novo** `LateCommitReply.test_a_redraw_that_a_slow_read_saw_after_the_deadline_is_too_late_and_not_redrawn`: **sem ele, a mutação S3 sobrevivia a todos os testes antigos**, e é por isso que ele foi escrito).

| Versão errada | O que ela muda | Testes que a derrubam (quantos, e de que classes) |
|---|---|---|
| W1 | uma resposta tardia só é registrada no log: o check falha, mas o estado dela ainda é usado e o `job.commit` é enviado | 5 (`LateJobAnswer` 5) |
| W2 | uma resposta tardia é aceita quando traz `succeeded` (só as outras são recusadas) | 4 (`LateJobAnswer` 4) |
| W3 | o prazo é conferido antes da chamada (o instante em que a consulta começou), não depois da resposta | 5 (`LateJobAnswer` 5) |
| W4 | uma resposta exatamente no prazo é tardia (`>=` no lugar de `>`) | 1 (`LateJobAnswer` 1) |
| W5 | só uma resposta tardia **final** é recusada (a não final fica com o detalhe vazio de antes) | 1 (`LateJobAnswer` 1) |
| W6 | os 60 s são julgados por consulta, não pela espera inteira | 1 (`LateJobAnswer` 1) |
| W7 | o redo é comparado com o hash da **base**, não com o do commit | 10 (`HistoryStacks` 2, `RecordedRedo` 3, `RedoPicture` 5) |
| W8 | o caso vacuoso nunca é marcado: toda comparação do redo é gravada como real | 3 (`HistoryStacks` 1, `RecordedRedo` 1, `RedoPicture` 1) |
| W9 | as pilhas só andam num undo quando um quadro convergiu (o comportamento de antes) | 5 (`HistoryStacks` 3, `RecordedRedo` 1, `RedoPicture` 1) |
| W10 | o redo nunca é comparado (a comparação fica só no undo) | 14 (`HistoryStacks` 3, `RecordedRedo` 4, `RedoPicture` 7) |
| W11 | uma comparação é inventada quando o commit não deixou quadro legítimo (compara com `None`) | 1 (`RedoPicture` 1) |
| W12 | o redo é comparado mesmo sem quadro convergido (contra o último estado visto) | 2 (`HistoryStacks` 1, `RedoPicture` 1) |
| W13 | um redo não devolve o gesto à pilha de undo | 5 (`HistoryStacks` 1, `RecordedRedo` 1, `RedoPicture` 3) |
| W14 | o motivo de um redo que não pôde ser comparado não é gravado (`unasked_checks`) | 1 (`RedoPicture` 1) |
| W15 | o hash do commit nunca é guardado em `Undoable` (sempre `None`) | 13 (`RecordedRedo` 4, `RedoPicture` 6, `UndoableCommit` 3) |
| S1 | varredura (a): `overlay_ack` aceita um reconhecimento que casa mas chegou depois do orçamento | 1 (`BoundedPolls` 1) |
| S2 | varredura (a): `Window.settle` aceita um quadro que convergiu depois do prazo | 1 (`NativeSession` 1) |
| S3 | varredura (a): `Window.settle_commit` aceita uma resposta que chegou depois do prazo | 1 (`LateCommitReply` 1) |
| S4 | varredura (a): `page_hidden_by_dialog` aceita uma resposta que chegou depois do prazo | 1 (`BoundedPolls` 1) |

- **Replay** (`RecordedRedo`, 5 testes; a gravação de `measure-12bc8da/native/report.json.gz` é **só lida**): na ordem gravada (`history.undo.translate`, `history.redo.translate`, `history.undo.translate.again`), nos três corpos, com os hashes gravados (a base e o commit do `translate`, e o do `dense` em que a escala não mudou o quadro): (i) o undo mostrando a base e o redo mostrando o hash do commit **passam**; (ii) um redo que mostra a **base** ou qualquer outro quadro **falha**, com os dois hashes no detalhe, ainda que desenhe um quadro mais novo; (iii) um gesto que não mudou nada grava a comparação do redo como **vacuosa**; (iv) um undo que não desenhou quadro **ainda avança as pilhas** e o redo seguinte é comparado com o commit certo. **O replay prova a regra sobre valores gravados, não o produto**: os ids de pedido, o tempo e o reconhecimento da página
  que vem depois da soltura são do modelo (a gravação não os guarda), e os hashes gravados são os de uma execução cujo veredito foi `FAIL` (20 verificações `drag.*`, BL015.23 (16) e (17)).

**O que continua limitado.** Nada disto rodou numa janela: **nenhuma janela mostrou** que o redo de verdade mostra o quadro do commit, nem que um `job.get` chegue depois dos 60 s (a espera é provada por relógio virtual), e o `deduped` segue dependendo de um reconhecimento da página que nunca foi visto numa janela (BL015.23 (16)). A comparação do redo é tão forte quanto o hash do quadro do commit que a soltura gravou: se o commit tem um desfecho que o runner
não reconhece (`unasked_checks`), o redo daquele gesto **não é comparado**. Os itens (31) a (35) estão abertos; (13)(a), (14), (20), (23) a (25) e (27) a (30) seguem como estavam. O runner corrigido **não foi reexecutado** (BL015.21 e BL015.23 seguem abertos): a prova é a spec, o controle negativo e o replay.

**O que esta rodada NÃO fez:** medir tempo; rodar qualquer runner contra uma medição (nem `native.py` numa janela); abrir a janela ou construir release; mudar `budgets.json`, jornada, corpus, golden, fixtures, margens, limiares ou qualquer Rust; decidir os itens (14) e (20); corrigir os itens (31) a (35); tratar o aviso "Docstring Coverage" (BL015.23 (13)(d), não é um achado); responder ou
resolver threads da review. **Nenhuma afirmação de desempenho vale: implementado não é validado.**

## Estado desta entrega e evidência

Arquivos e identidade de cada execução: [gizmo-perf-evidence/README.md](../migration/gizmo-perf-evidence/README.md).
Regra de leitura: **implementado** (código e testes que rodam) ≠ **validado** (números/provas com identidade).

| Item | Estado |
|---|---|
| Spec dos runners Python (`gizmo-perf.spec.py`, 285 testes: os 254 anteriores (115 + 69 da rodada 6 + 37 da rodada 7 + 31 da rodada 8 + 2 da rodada 9) + 31 da rodada 10; harness, veredito do runner/mutantes, baseline com repositórios temporários, A/B, nativo com janela falsa e `drag` com monitor roteirizado, proveniência, auditoria, os executáveis dos mutantes de tempo entre `prepare` e `time` e a posse exclusiva de `--out` pelo `mutate.py`, e, na rodada 6, a identidade do git, as travas, o ambiente dos filhos, o reconhecimento do overlay e a janela com dublês, e, na rodada 7, os desfechos de uma soltura num modelo do host com relógio virtual, a tabela de expectativas de pixels, os mesmos fluxos por `run_corpus` e o replay dos 21 gestos gravados em `12bc8da`, e, na rodada 8, o ambiente de todo `git` que os runners iniciam, a identidade `dirty` com git real, a trava única por `--out` (com processos reais que recusam), as esperas limitadas do `native.py` e o reconhecimento tardio de uma soltura, e, na rodada 9, a primeira leitura de um reconhecimento em `settle_commit` num monitor roteirizado, e, na rodada 10, a espera do `job.get` com resposta tardia, o hash do commit em `Undoable`, a comparação do redo, as pilhas do histórico, o replay do redo gravado e o `settle_commit` com leitura tardia) | **rodada 10: 285/285 passam** na árvore **limpa** de `4aa964a` (49,5 s); dos 31 testes novos, 26 **falham** nos runners de `3d5b436` (5 controles passam dos dois lados; nenhum dos 254 anteriores falha; 19 versões erradas são pegas; replay da gravação de `12bc8da`, só lida) ([`spec-new-tests-vs-3d5b436-runners.log`](../migration/gizmo-perf-evidence/spec-new-tests-vs-3d5b436-runners.log)); só `native.py`, a spec e docs mudaram, nenhum Rust, e nada foi medido nem visto numa janela. **Antes, na rodada 9: 254/254 passam** na árvore **limpa** de `292e398` (48,3 s); dos 2 testes novos, cada um **falha** na mutação de `settle_commit` que o motiva (Sd-b e Sd-c, que as 18 provas anteriores deixavam sobreviver) e passa na outra, e a Sd-d é um mutante equivalente, sem teste ([`spec-round9-ack-tests-vs-mutants.log`](../migration/gizmo-perf-evidence/spec-round9-ack-tests-vs-mutants.log)); nenhum runner mudou e nada foi medido. **Antes, na rodada 8: 252/252 passam** na árvore **limpa** de `e40d511` (52,5 s); dos 31 testes novos, 26 **falham** nos runners de `ca66012` (5 controles passam dos dois lados; nenhum dos 211 anteriores não reescritos falha; 14 versões erradas das correções são pegas; dois processos reais provam a trava única) ([`spec-new-tests-vs-ca66012-runners.log`](../migration/gizmo-perf-evidence/spec-new-tests-vs-ca66012-runners.log)); `corpus.py --check`, `py_compile` dos runners e `git diff --check` passam. **Antes, na rodada 7: 221/221 passam** na árvore **limpa** de `3ee60de`; dos 37 testes novos, 35 **falham** nos runners de `42229a3` (2 controles passam dos dois lados; nenhum dos 184 anteriores falha; 16 versões erradas das correções são pegas; o replay dos 21 gestos gravados dá 0 reprovações contra as 20 gravadas) ([`spec-new-tests-vs-42229a3-runners.log`](../migration/gizmo-perf-evidence/spec-new-tests-vs-42229a3-runners.log)); `corpus.py --check`, `py_compile` dos runners e `git diff --check` passam. **Antes, na rodada 6: 184/184 passam** na árvore **limpa** de `9b5f9fd`; os 69 testes novos têm 65 que **falham** nos runners de `d294bdc` (4 controles passam dos dois lados; nenhum dos 115 anteriores falha; 10 versões erradas de uma correção são pegas) e a prova com dois processos reais ([`spec-new-tests-vs-d294bdc-runners.log`](../migration/gizmo-perf-evidence/spec-new-tests-vs-d294bdc-runners.log)). **Antes, na rodada 4:** 115/115 na árvore **limpa** de `081353e` ([`gates-081353e.txt`](../migration/gizmo-perf-evidence/gates-081353e.txt)); os 4 testes da rodada 4 **falham** nos runners de `1b92c9c` (5 falhas: o (d) falha nos dois subcasos; nenhum teste anterior falha), e o log traz a matriz de variantes erradas da correção e a verificação com dois processos reais ([`spec-new-tests-vs-1b92c9c-runners.log`](../migration/gizmo-perf-evidence/spec-new-tests-vs-1b92c9c-runners.log)); dos 4 testes da rodada 3, 3 falham nos runners de `f3b2c32` (o 4º passa dos dois lados de propósito); na rodada 2 foram 107/107 em `c3ae380`, com 28 falhando ou dando erro nos runners de `9a72e03`. `corpus.py --check` passa (exit 0) e o `git diff --check` contra `1b92c9c` e contra `cbe9e1b` dá exit 0 em `081353e` (o `git diff --check 9a72e03 HEAD` **tinha falhado em `c3ae380`**, linha em branco no fim de `bl01522-host-scratch-with-fix.log`, corrigida no commit de docs da rodada 2); o resultado no commit de docs está no relatório da entrega (sem Cargo, sem janela, sem benchmark) |
| Core: `fmt --check`, `clippy -p tessel-editor-core --all-targets -D warnings`, estrutural (7), threads (1, novo), core inteiro e `cargo tree` | **executados em `a97a0a7`, todos exit 0**: `fmt`; `clippy`; estrutural **7 passaram, 0 falharam, 0 ignorados**; threads **1 passou**; core inteiro (`--no-fail-fast`) **54 binários, 1267 passaram, 0 falharam, 7 ignorados** (3 do orçamento de tempo desta suíte e 4 medições de outras frentes, todos `#[ignore]` de propósito); `cargo tree -p tessel-editor-core --locked`: 16 linhas, **sem gpui/wry** (log e saída completa em [`rust-gates-a97a0a7.txt`](../migration/gizmo-perf-evidence/rust-gates-a97a0a7.txt)). Nada foi medido em tempo. **Em `c3ae380` (árvore limpa) só foram repetidos** o estrutural (7 passaram, 0 falharam) e o `cargo tree` (sem gpui/wry; [`gates-c3ae380.txt`](../migration/gizmo-perf-evidence/gates-c3ae380.txt)): `crates/core/src` e o resto do core não mudaram desde `a97a0a7` (só o comentário do cabeçalho de `scene_gizmo_perf_structure.rs`), e o core inteiro e o teste de threads **não** foram repetidos. **Em `e37a76f` (rodada 3, árvore limpa) foram repetidos** `fmt`, `clippy -p tessel-editor-core --all-targets -D warnings`, o estrutural (7 passaram, 0 falharam), o de threads (1 passou) e o `cargo tree` (sem gpui/wry), todos exit 0 ([`gates-e37a76f.txt`](../migration/gizmo-perf-evidence/gates-e37a76f.txt)); a única mudança Rust desde `c3ae380` é um comentário de `scene_gizmo_perf_budget.rs`; o core inteiro **não** foi repetido naquela rodada. **Em `081353e` (rodada 4, árvore limpa, sob o lock de GPU) o core inteiro rodou dentro do `test --workspace`**: 53 binários de teste e 1 execução de doc-tests, **1267 passaram, 0 falharam, 7 ignorados** (os mesmos números de `a97a0a7`; os 7 são os 3 do orçamento de tempo desta suíte, `scene_attach_cost`, `scene_fx_key_bounds` e os 2 de `scene_plan_performance_bench`, todos `#[ignore]` de propósito), com o estrutural (7) e o de threads (1) dentro; `cargo tree -p tessel-editor-core` sem gpui/wry de novo ([`gates-081353e.txt`](../migration/gizmo-perf-evidence/gates-081353e.txt)) |
| Host GPUI (`cargo test -p tessel-editor --locked scene_gizmo_perf`) | **5 passaram, 0 falharam** em `c3ae380`, árvore limpa (exit 0; 413 filtrados): os 3 de antes, o `showing_a_scene_that_shares_its_bundle_with_the_shown_one_shows_it` (que **falhava em `a97a0a7`**, BL015.22, e passa depois da correção) e o novo `monitor_show_scene_follows_the_request_between_scenes_of_one_bundle_and_of_another`. O crate `tessel-editor` inteiro: **418 passaram, 0 falharam, 0 ignorados** (exit 0), com o do ciclo de vida do monitor; `clippy -p tessel-editor --all-targets -D warnings` exit 0 (a execução real, de 76,6 s, foi no código-fonte do `bab9290` ainda não commitado; em `c3ae380` o resultado veio do cache do mbx, 5 hits e 0 misses, sem recompilar). Em `c3ae380`, `clippy --workspace` e o workspace inteiro **não** foram rodados (rodaram em `081353e`, abaixo). Não repetido na rodada 3: nenhuma fonte do crate mudou desde `c3ae380`. **Em `081353e` (rodada 4, árvore limpa)**: o crate `tessel-editor` inteiro, dentro do `test --workspace` com `TESSEL_REQUIRE_RUNTIME=1`: **418 passaram, 0 falharam, 0 ignorados** (os 5 de `scene_gizmo_perf` entre eles), `clippy --workspace --all-targets -D warnings` exit 0 e `build -p tessel-editor` exit 0. **É só o teste estrutural do host**: nenhuma medição do host nem da janela, e o A/B não os vê (BL015.23 (14)) |
| `within()` do orçamento (não mascara FAIL) e o resto de `scene_gizmo_perf_budget.rs` | **compilados** (`clippy --all-targets`; `cargo test` lista os 3 testes como `ignored`); **executados uma vez em `12bc8da` por `run.py`** (rodada 5): os 2 controles negativos passaram e o teste de orçamento deu **`FAIL`** (6 p95 do `deep` acima do orçamento) |
| Mutantes `extra_plan_per_handler` (estrutural), `memo_off` e `replan_per_tick` (host) | mortos em `9186e2a`; o `.tgz` retido mistura manifestos de `9186e2a` e `b898a6e`. **Reexecutados em `12bc8da` (rodada 5, `prepare` duas vezes, resultados iguais)** com o runner novo (verdicts por teste nomeado + mensagem): `extra_plan_per_handler` (estrutural) e `replan_per_tick` (host) **KILLED**; `memo_off` **UNEXPECTED_FAILURE** (o teste nomeado falhou numa verificação anterior às mensagens que o runner espera, BL015.23 (19)); **a classificação foi corrigida no runner na rodada 6 (`9b5f9fd`) e não foi reexecutada** |
| Tempo no core (release, lock, guardas) | **PASS no `5cded7e`** (árvore limpa), regra de margem fria já ajustada depois de um FAIL (ver histórico); **`FAIL` em `12bc8da`** (rodada 5; `run.py`, 6 p95 do `deep` acima do orçamento, sob carga de base no limite da guarda; BL015.23 (20)). Um PASS de outro head não vale para este |
| Mutantes de tempo (plano extra, 1 ms) | BLOCKED por carga em `9186e2a`; **em `12bc8da` (rodada 5)** o runner das rodadas 3 e 4 rodou de verdade: `delay_1ms_per_handler` **KILLED** pelo detector de tempo (duas vezes) e `extra_plan_per_handler` **KILLED** (no retry; a primeira tentativa deu `INVALID_GUARD_AFTER_RUN`); o estrutural **não** vê o atraso puro (`SURVIVED`, esperado). É detecção de mutantes **sintéticos**, não ausência de regressão |
| A/B `cbe9e1b` × candidato | o runner novo foi executado em `12bc8da` (rodada 5): build `BUILT` (executáveis distintos) e **measure `INVALID_GUARD_AFTER_RUN` nas duas tentativas: nada foi julgado**; o A/B retido (`037eda5`) é o **antigo, que era A/A**: **sem comparação**; nada de "sem regressão" é afirmado. O runner foi corrigido na rodada 6 (executável do candidato copiado antes do build da baseline, recusa de executáveis iguais e de target no ambiente, tentativas anteriores guardadas em `attempts/`; **não reexecutado**) |
| Janela nativa | só a corrida funcional antiga (`small`, 160 verificações, runner anterior, revisão anterior; o manifesto do `.tgz` diz BLOCKED e não casa com o relatório: ver o README da evidência); o runner novo (`project.open` real, quadro novo, cancelar, `dense`/`deep`) e a jornada atual (com `same.*`) **rodaram uma vez em `12bc8da`** (rodada 5, três corpos): **`FAIL`** (639 verificações, 20 falharam, todas `drag.*`), `same.*`/`switch.*` 15/15 por corpo, sem baseline para a janela; o runner foi corrigido na rodada 6 (o reconhecimento do overlay tem de ser o do passo, uma falha de `render.status` reprova a sessão, esperas limitadas; **não reexecutado**) e, **na rodada 7**, nas duas expectativas que essa execução mostrou erradas (a soltura de um gesto cujo candidato foi desenhado pode deixar o próprio quadro dele, e a escala do `dense` pode deixar o quadro igual; **não reexecutado**, nenhuma janela abriu) |
| Workspace inteiro com `TESSEL_REQUIRE_RUNTIME=1` | **executado em `081353e` (rodada 4), árvore limpa, sob o lock de GPU: exit 0, 500 s, 2299 passaram, 0 falharam, 16 ignorados** em 117 linhas de resultado (113 binários de teste e 4 execuções de doc-tests): `tessel-editor` 418/0/0, áudio 10/0/0, core 1267/0/7, mídia 497/0/6, render 107/0/3. O `the_journey_leaves_the_transform…`, que falhou em `5cded7e` (2294/1/16: ordem das chaves JSON com `preserve_order` unificado no workspace, corrigido no `9cc2335`), **passa**; os 16 `ignored` são `#[ignore]` de propósito (os 3 de `scene_gizmo_perf_budget` só rodam por `run.py`). **É só o gate funcional: nada foi medido em tempo.** O commit de docs seguinte não muda código (`git diff --stat 081353e HEAD` só lista `docs/**` e o README dos runners) ([`gates-081353e.txt`](../migration/gizmo-perf-evidence/gates-081353e.txt)) |
| Provas de processo e gates funcionais de `0ee2d14` | **pendentes** |

### O que falta para validar (BL015.21)

Feito: em `a97a0a7` o Rust novo do core e do host compilou e rodou; em `c3ae380` o BL015.22 está corrigido e verificado **no
host** (5/5 e o crate inteiro, 418/0/0). Em `e37a76f` (rodada 3) o core (`fmt`, `clippy`, estrutural, threads) e a spec 111/111 foram repetidos. Em `081353e` (rodada 4, árvore limpa) rodaram `fmt`, `check --workspace`,
`clippy --workspace --all-targets -D warnings`, o build de `tessel-editor`, o **workspace inteiro com `TESSEL_REQUIRE_RUNTIME=1`** (2299/0/16) e a spec 115/115: **são gates funcionais**, nada de tempo.
Em `e40d511` (rodada 8, só runners, spec e docs) a spec 252/252 passou na árvore limpa e os seis comentários da review 5859397176 foram tratados nos runners: **só dublês e processos que
recusam, nenhum runner foi reexecutado contra uma medição**. Em `292e398` (rodada 9, só a spec e o log de controle) a spec 254/254 passou na árvore limpa: duas regras de `settle_commit` ganharam teste e uma terceira mutação é equivalente, **sem nenhuma medição e sem mudar um runner**. Em `4aa964a` (rodada 10, só `native.py`, a spec e o log de controle) a spec 285/285 passou na árvore limpa: a espera do `job.get` recusa uma resposta tardia, o redo é comparado com o quadro do commit e as pilhas do histórico seguem o documento (os três achados da review 5861824134), **sem nenhuma medição e sem ver uma janela**. Em `12bc8da` (rodada 5) os quatro runners rodaram pela primeira vez (ver "Rodada 5: primeira medição em `12bc8da`"): **nenhum resultado valida desempenho** (`run.py` `FAIL`, A/B `INVALID_GUARD_AFTER_RUN`, `mutate.py` `UNEXPECTED`, `native.py` `FAIL`). Resta o efeito da correção do BL015.22 na **janela nativa** (a jornada atual rodou lá uma vez, sem controle negativo) e tudo o que mede.

Os itens 2 a 5 abaixo foram **tentados uma vez** em `12bc8da` (rodada 5), sob uma carga de base no limite da guarda; o que falta é repeti-los numa máquina em que a guarda **depois** da corrida segure (BL015.23 (11)) e tratar os achados que seguem abertos: (20) e, sem correção, (23) a (25) e (27) a (35) (os (15), (18), (19), (21) e (22) foram corrigidos nos runners na rodada 6, os (16) e (17) na rodada 7 e, na rodada 8, o git sem `GIT_*`, a identidade `dirty`, a
trava única, o `time` do `mutate.py` e as esperas do `native.py`, e na rodada 10 o (26) e a espera do `job.get`, **sem reexecução**). Com disco ≥ 16 GiB e carga ≤ 0,5 por CPU (uma máquina que não esteja compilando outra coisa): 1. (feito em `081353e`; só repetir
se o código mudar) o workspace com `TESSEL_REQUIRE_RUNTIME=1` e `clippy --workspace`; 2. `run.py --phase build` e `--phase measure`; 3. `ab.py --phase build` e
`--phase measure` (primeiro A/B de verdade; **exigido** de toda PR que toque o **core** que ele mede, `scene_plan/**` e o resto
de `crates/core/{src,build.rs,Cargo.toml}` e das entradas de build do workspace, porque é a única comparação absoluta com uma
baseline, e só vê o que passa dos limiares sobre o total da requisição: ver "Tempo"; **ele não mede o host, o renderer nem a
WebView, nem a mudança de `scene_monitor_host.rs` desta PR**: para isso falta uma comparação baseline × candidato do host e da
janela, que não existe e cujo desenho não foi feito, BL015.23 (14); contra a `main` de agora: ver "A/B contra a `main` de
agora"); 4. `mutate.py --phase prepare` e `--phase time`; 5. `native.py
--phase build` e `--phase run` nos três casos; 6. um caso com splat (BL015.23); 7. orçamentos nativos só depois de um A/B com
um binário nativo da `main`; 8. as provas de `0ee2d14`; 9. uma comparação baseline × candidato do host, do renderer e da janela, que o A/B não faz (BL015.23 (14); sem desenho). **Em `12bc8da` o tempo, o A/B, os mutantes de tempo e a janela nativa foram executados uma vez (rodada 5) e nenhum virou prova de desempenho: nenhuma afirmação de desempenho vale.**
