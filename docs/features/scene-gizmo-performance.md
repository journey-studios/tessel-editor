# F055 (desempenho) — Suíte de regressão de performance dos gizmos e da abertura (BL015.21)

Complementa [F055](scene-gizmo.md). Pedido: "garantir que não perdemos performance e que validamos o mesmo
cenário de abrir o app visualmente; o que for possível por teste unitário". Nada aqui muda o produto: o único
código de produção tocado é o contador de layouts (`scene_plan::probe` + a chamada em `ScenePlan::build_with`, ver
abaixo). O custo desse contador **não é assumido nulo**: só o A/B o mede (e ele ainda não foi medido).

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
| Host GPUI | `crates/desktop/src/workstation/scene_gizmo_perf_tests.rs` (4 testes; em `a97a0a7` **3 passam e o 4º FALHA** (`showing_a_scene_that_shares_its_bundle_with_the_shown_one_shows_it`): é um defeito de produto anterior à PR, BL015.22, ver "Achados e pendências") | a MESMA jornada por `Workstation::run_op` (mesmo golden), incluindo `monitor.show_scene` e resize; layouts por requisição **no host** (orçamento = o medido); janela parada (100 ticks do timer = 0 layouts; **no máximo 1 layout por sync** do mesmo estado — o do próprio monitor —, ou seja ≤ 20 para 20 syncs, que é o que o teste afirma (`<= SYNCS`); 0 pedidos do overlay, só `memo_hits`). O texto anterior ("20 syncs = no máximo 1 layout") não era o que o teste verificava; o contrato é o do teste (uma cota agregada de 1 por sync, não uma checagem sync a sync: um sync com 2 layouts compensado por outro com 0 passaria). Também a cena de **mesmo bundle** (`same.*`, BL015.22) | WebView, GPU, compositor, ponteiro físico; o diálogo (precisa do `Root` do Kit: é da janela nativa) |
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
longos (> 16,6 ms); **nenhuma amostra é descartada** e frio/quente ficam separados (frio = 60 editores novos por tipo,
primeiro quadro incluído). `INCONCLUSIVE` quando mais de 25% dos blocos de referência discordam de si mesmos em > 1,5×;
`BLOCKED` quando o lock `/tmp/tessel-scene-gpu.lock` está ocupado (uma tentativa, sem espera), a carga (1 **e** 5
min) passa de 0,5 por CPU, o disco está abaixo do piso ou há limite térmico. Em Apple silicon o `pmset` não informa
`CPU_Speed_Limit`: o relatório diz `thermal_state.verified = false` ("sem limite reportado; estrangulamento não
excluído"), nunca "silencioso". As guardas são lidas **de novo depois** da corrida: uma guarda ruim depois torna a
corrida `INVALID_GUARD_AFTER_RUN` (não é PASS). As duas "injeções" (um plano a mais; 1 ms) são **controles
negativos** do próprio detector, nunca prova de desempenho.

## CLI headless

`./scripts/headless` já executa o mesmo vocabulário JSONL (`scene.*`, `history.*`, `project.create`); o driver do core
usa `headless::execute`, o mesmo handler. A suíte **não** acrescenta comando novo (nenhum controle novo: nada em `tree.rs`
nem parser). Como rodar:

```
./scripts/cargo test -p tessel-editor-core --locked --test scene_gizmo_perf_structure     # estrutural, sempre
./scripts/cargo test -p tessel-editor-core --locked --test scene_gizmo_perf_threads       # nenhum layout escapa da thread lida (binário próprio)
./scripts/cargo test -p tessel-editor --locked scene_gizmo_perf                          # host GPUI (hoje 3/4: BL015.22)
python3 scripts/gizmo-perf/run.py --phase build --out DIR        # release do teste de tempo (sem lock)
python3 scripts/gizmo-perf/run.py --phase measure --out DIR      # tempo no core (lock, guardas, manifesto; não constrói)
python3 scripts/gizmo-perf/ab.py --phase build --out DIR         # baseline cbe9e1b + candidato (sem lock)
python3 scripts/gizmo-perf/ab.py --phase measure --out DIR       # A/B (lock, guardas)
python3 scripts/gizmo-perf/mutate.py --phase prepare --out DIR   # mutantes de produto: estrutural e host
python3 scripts/gizmo-perf/mutate.py --phase time --out DIR      # mutantes de tempo (lock, guardas)
python3 scripts/gizmo-perf/native.py --phase build --out DIR     # editor release do HEAD, com proveniência
python3 scripts/gizmo-perf/native.py --phase run --out DIR       # janela real (precisa ./scripts/runtime setup)
```

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
retido do `5cded7e`): em **22 de 60** tipos (todos os `move.*`/`up.*` de `dense`/`deep`/`small`, `cold.attach` e alguns de
`small`) **um plano extra passa em tempo**; em 17/60 a folga do p95 é ≥ 2×, em 6/60 ≥ 3×, e em 1 tipo (`dense/cold.attach`)
um atraso de 1 ms passa (convertido com o tempo de plano medido). Isto é uma propriedade do orçamento, não um defeito
corrigido: **o tempo sozinho não detecta um plano extra nesses 22 tipos**. É o detector estrutural (plano por requisição,
exato) que cobre isso, ortogonal ao tempo; os dois têm controles negativos (um plano a mais falha o estrutural; 1 ms falha o
tempo onde o orçamento é estreito). Onde a folga é larga, o tempo é só um teto grosso.

## Achados e pendências (não são regressões provadas)

- `pointer.move`/`up`: 6–8 planos observados no release (denso ≈ 4 ms) **contando um único layout por requisição** (o
  teste estrutural prova 1 layout por move): o resto é do gesto (candidato, planos de escala/rotação). Não há trabalho
  duplicado demonstrado e **nenhuma otimização foi feita**; sem A/B mostrando regressão contra a `main`, nada a corrigir.
  Desempenho: pendente.
- BL015.22 — `monitor.show_scene` entre duas cenas do **mesmo `bundleId`**: **reproduzido no host, de forma
  determinística, e é um defeito de produto anterior a esta PR** (a hipótese anterior, "não reproduzida", era do relatório
  nativo único; ela deixa de valer). A jornada inclui o caso (`same.attach`/`same.resources`/`same.show`: `proof-c` do mesmo
  arquivo de `proof-b`, com `expect.shown_scene = proof-c`). O teste do host
  `showing_a_scene_that_shares_its_bundle_with_the_shown_one_shows_it` **FALHA em `a97a0a7`**, no grupo e isolado
  (`left: Some("proof-b")`, `right: Some("proof-c")`). Não se evitou o caso nem se afrouxou o teste (sem `#[ignore]`, sem
  mudar a expectativa nem o corpus), e **o produto não foi alterado nesta PR**.
  - *Causa raiz (executada, com reprodutor mínimo; [`bl01522-same-bundle-repro.log`](../migration/gizmo-perf-evidence/bl01522-same-bundle-repro.log)
    e [`.rs.txt`](../migration/gizmo-perf-evidence/bl01522-same-bundle-repro.rs.txt))*: `ViewportHost::render_scene`
    (`scene_monitor_host.rs`, dedupe de 6a8b030, não tocado pela PR) compara a chave `plano JSON | mapa de recursos | opções`
    com a do último pedido e, se igual, **retorna antes** de atribuir `directed.scene`. O id da cena não está na chave: duas
    cenas anexadas do mesmo arquivo têm plano, mapa e opções idênticos (medido: os três iguais). Resultado: `monitor.show_scene
    proof-c` responde `ok` (`{"action":"monitor.show_scene","scene":"proof-c"}`) e o host segue reportando `proof-b` como cena
    dirigida **logo depois do pedido** (não é efeito dos redimensionamentos que vêm depois). Controle: mostrar uma cena de
    plano diferente e depois `proof-c` funciona (`proof` → `proof-c`); mostrar `proof-b` logo depois de `proof-c` não troca
    (a regra é simétrica e por conteúdo). O que isso **não** prova: o que a página desenha (o host do teste não tem WebView;
    como plano, mapa e opções são idênticos, o quadro pedido ao runtime deveria ser o mesmo, mas isso não foi observado) nem o
    efeito no gizmo da janela real, que só foi **lido no código** (`overlay_refresh`/`overlay_pointer` escolhem a cena por
    `directed_scene()`, ou seja, o overlay e o ponteiro da página seguiriam a cena antiga): **não executado**.
  - *Mudança de produto candidata, verificada só como diagnóstico local e **descartada** (nunca commitada)*: incluir o id da
    cena na chave. Com ela, o teste acima passa sem alteração, os outros 3 do host continuam passando (inclusive o dos
    layouts por requisição) e o reprodutor mínimo mostra `proof-c`/`proof-b` conforme o pedido. **A decisão é do
    coordenador/dono do produto** (id na chave, recusa tipada ou dedupe legítimo com `directed.scene` atualizada); enquanto
    isso o teste do host fica vermelho de propósito e o resultado nativo segue pendente.
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

## Estado desta entrega e evidência

Arquivos e identidade de cada execução: [gizmo-perf-evidence/README.md](../migration/gizmo-perf-evidence/README.md).
Regra de leitura: **implementado** (código e testes que rodam) ≠ **validado** (números/provas com identidade).

| Item | Estado |
|---|---|
| Spec dos runners Python (`gizmo-perf.spec.py`, 76 testes: os 70 anteriores + 6 desta rodada; harness, veredito do runner/mutantes, baseline com repositórios temporários, A/B, nativo com janela falsa e `drag` com monitor roteirizado, proveniência, auditoria) | **executado e passa (76/76)** na árvore desta rodada (`a97a0a7` + as correções de `native.py`/`ab.py`); `corpus.py --check` e `git diff --check` também passam (sem Cargo, sem janela, sem benchmark) |
| Core: `fmt --check`, `clippy -p tessel-editor-core --all-targets -D warnings`, estrutural (7), threads (1, novo), core inteiro e `cargo tree` | **executados em `a97a0a7`, todos exit 0**: `fmt`; `clippy`; estrutural **7 passaram, 0 falharam, 0 ignorados**; threads **1 passou**; core inteiro (`--no-fail-fast`) **54 binários, 1267 passaram, 0 falharam, 7 ignorados** (3 do orçamento de tempo desta suíte e 4 medições de outras frentes, todos `#[ignore]` de propósito); `cargo tree -p tessel-editor-core --locked`: 16 linhas, **sem gpui/wry** (log e saída completa em [`rust-gates-a97a0a7.txt`](../migration/gizmo-perf-evidence/rust-gates-a97a0a7.txt)). Nada foi medido em tempo |
| Host GPUI (`cargo test -p tessel-editor --locked scene_gizmo_perf`) | **3 passaram, 1 FALHOU** (exit 101, `a97a0a7`): passam o golden da jornada, os layouts por requisição e a janela parada; **falha** `showing_a_scene_that_shares_its_bundle_with_the_shown_one_shows_it`, também isolado (determinístico): BL015.22, defeito de produto anterior à PR, causa raiz e reprodutor em "Achados e pendências"; o teste e o corpus ficam como estão. Só esses 4 testes do host rodaram (412 filtrados): `clippy` do host e do workspace e o workspace inteiro **não** foram rodados neste head |
| `within()` do orçamento (não mascara FAIL) e o resto de `scene_gizmo_perf_budget.rs` | **compilados** (`clippy --all-targets`; `cargo test` lista os 3 testes como `ignored`), **não executados**: só rodam com tempo, lock e guardas |
| Mutantes `extra_plan_per_handler` (estrutural), `memo_off` e `replan_per_tick` (host) | mortos em `9186e2a`; o `.tgz` retido mistura manifestos de `9186e2a` e `b898a6e`. **Reexecutar** com o runner novo (verdicts por teste nomeado + mensagem) |
| Tempo no core (release, lock, guardas) | **PASS no `5cded7e`** (árvore limpa), regra de margem fria já ajustada depois de um FAIL (ver histórico); **não repetido** neste head. Um PASS de outro head não vale para este |
| Mutantes de tempo (plano extra, 1 ms) | **não provados** (BLOCKED por carga em `9186e2a`; não repetidos) |
| A/B `cbe9e1b` × candidato | o runner novo **não foi executado** (BLOCKED: disco/carga); o A/B retido (`037eda5`) é o **antigo, que era A/A**: **sem comparação**; nada de "sem regressão" é afirmado |
| Janela nativa | só a corrida funcional antiga (`small`, 160 verificações, runner anterior, revisão anterior); o runner novo (`project.open` real, quadro novo, cancelar, `dense`/`deep`) **não rodou** |
| Workspace inteiro com `TESSEL_REQUIRE_RUNTIME=1` | no `5cded7e`: 2294/1/16 (**1 falha**, defeito de ordem das chaves JSON do teste `the_journey_leaves_the_transform…`, corrigido depois e verificado só com `-p tessel-editor-core`; em `a97a0a7` só rodou `-p tessel-editor-core`, 1267/0/7, que não reproduz aquela falha de features unificadas): **não é um PASS final**; o workspace inteiro não foi repetido |
| Provas de processo e gates funcionais de `0ee2d14` | **pendentes** |

### O que falta para validar (BL015.21)

Feito em `a97a0a7` (item 1 em parte): o Rust novo do core e do host compilou e rodou; resta o workspace inteiro com
`TESSEL_REQUIRE_RUNTIME=1`, o `clippy` do host/workspace, e a **decisão sobre BL015.22** (o teste do host está vermelho).

Com disco ≥ 16 GiB e carga ≤ 0,5 por CPU (uma máquina que não esteja compilando outra coisa): 1. o workspace com
`TESSEL_REQUIRE_RUNTIME=1` e `clippy --workspace`; 2. `run.py --phase build` e `--phase measure`; 3. `ab.py --phase build` e
`--phase measure` (primeiro A/B de verdade); 4. `mutate.py --phase prepare` e `--phase time`; 5. `native.py --phase build` e
`--phase run` nos três casos; 6. um caso com splat (BL015.23); 7. orçamentos nativos só depois de um A/B com um binário
nativo da `main`; 8. as provas de `0ee2d14`. **Tempo, A/B, mutantes de tempo e janela nativa seguem sem execução neste
head: nenhuma afirmação de desempenho vale.**
