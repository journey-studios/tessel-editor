# F055 (desempenho) — Suíte estrutural de regressão de performance dos gizmos e da abertura (BL015.21)

Complementa [F055](scene-gizmo.md). Pedido: "garantir que não perdemos performance e que validamos o mesmo
cenário de abrir o app visualmente; o que for possível por teste unitário".

> **Desempenho: NÃO validado.** Esta página descreve a suíte **estrutural**: planos por requisição, uma jornada
> única e versionada, sem relógio. Nenhuma medição de tempo vale: não há PASS de orçamento nem A/B medido. A pilha
> de tempo (runners, orçamentos, A/B, mutantes, janela nativa) **não** é descrita aqui: ver
> "Tempo e janela nativa". Regra de leitura: **implementado** (código e testes que rodam) ≠ **validado** (números
> ou provas com identidade).

O código de produção tocado é pequeno: o contador de layouts (`scene_plan::probe` e a chamada em
`ScenePlan::build_with`) e **uma mudança de comportamento do monitor** (`crates/desktop/src/workstation/scene_monitor_host.rs`:
o id da cena entra na chave de deduplicação de `ViewportHost::render_scene`, BL015.22). O custo do contador **não é
assumido nulo**: nunca foi medido (só uma comparação de tempo com baseline o mediria, e ela não foi julgada).

## Objetivo e limites

Uma **jornada única e versionada** (`docs/migration/fixtures/gizmo-perf/journey.json`, v1, 41 passos: 29 `document`,
8 `presentation`, 4 `presentation_only`) roda sobre um **corpus sintético compartilhado**
(`docs/migration/fixtures/gizmo-perf/corpus.manifest.json`, com sha256 do gerador, da jornada e de cada arquivo; nada
privado): abrir projeto, anexar, resolver recursos, exibir, primeiro quadro, selecionar, translação, rotação (local e
mundo) e escala (uniforme e por eixo), confirmar, cancelar, undo/redo, auxiliar oculto, trocar de cena (de outro bundle
e do mesmo bundle), redimensionar e diálogo.

| Caso | O que é | Nós no manifesto | Por quê |
|---|---|---|---|
| `small` | o corpus da prova de cena (`scripts/directed-scene-proof/corpus`), animado | 6 | abertura típica |
| `dense` | `small` + 12 bancos × 50 planos girados | 618 | custo que cresce com o número de nós |
| `deep` | `small` + cadeia de 24 pais girados + 12 anisotrópicos | 44 | pais transformados (matemática da direção local e da escala) |

As contagens são as de `corpus.manifest.json` (a raiz entra). O gerador é `scripts/gizmo-perf/corpus.py`
(`--check` falha se um arquivo versionado não é o que o gerador produz). **O manifesto fixa o sha256 do próprio
`corpus.py`:** qualquer edição do gerador, até de comentário, exige regenerar o manifesto
(`python3 scripts/gizmo-perf/corpus.py`, sem `--check`); senão `--check` e o teste
`the_corpus_the_journey_and_the_generator_match_their_manifest` falham.

Limites declarados, que esta suíte **não** esconde:

- **Não há caso com splat** (o corpus sintético pequeno não tem asset de splat; o pedido o inclui "onde suportado";
  lacuna (1)). **Nada exercita luz** (lacuna (36)).
- Sem GPU real nem ponteiro físico (BL015.2, BL043 e BL044 seguem pendentes). Sem alegar "custo 0".
- **O core não tem quadro**: só a janela e o host apresentam; "primeiro quadro" nunca é uma medida do core.
- Contador de trabalho, não de tempo: o workspace proíbe `unsafe_code`, então não há contador de alocações nem de CPU.

## Superfícies

A jornada roda pelos **mesmos handlers**; cada passo é `document` (equivalente: mesmo estado, mesmo undo),
`presentation` (precisa de uma janela que apresente) ou `presentation_only` (resize e diálogo).

| Superfície | Onde | O que prova | O que **não** prova |
|---|---|---|---|
| Core (lib) | `crates/core/tests/gizmo_perf_support` e `crates/core/tests/scene_gizmo_perf_{structure,threads}.rs` | transformada, undo e redo idênticos ao golden (`journey.expected.json`); planos por requisição; independência do tamanho da cena | janela, GPU, página, tempo |
| Host GPUI | `crates/desktop/src/workstation/scene_gizmo_perf_tests.rs` (5 testes) | a mesma jornada por `Workstation::run_op`, layouts por requisição no host, janela parada, cena de mesmo bundle | WebView, GPU, compositor, ponteiro físico; o diálogo (precisa do `Root` do Kit) |
| Janela nativa | `scripts/gizmo-perf/native.py` (pilha de tempo, [contrato de tempo](scene-gizmo-performance-timing.md)) | a mesma jornada numa janela real, só como observação funcional: **a única corrida completa deu `FAIL`** | desempenho, baseline, compositor, ponteiro físico |

## Contrato do core / biblioteca

`scene_plan::probe` (`crates/core/src/scene_plan/probe.rs`, 43 linhas, só `std`) conta cada `ScenePlan::build_with`
(o layout completo da cena) de duas formas: **por thread** (`layouts_built()`, exata mesmo num `cargo test` paralelo) e
**por processo** (`layouts_built_in_process()`, um `fetch_add` relaxado). É o **detector estrutural**: um plano a mais
por handler ou por tick falha o teste sem ler relógio. A chamada fica em `build_with`; nenhum `Cargo.toml` mudou.

**`scene_gizmo_perf_structure.rs`** (7 testes, rodam sempre, determinísticos):

| Teste | O que afirma |
|---|---|
| `the_corpus_the_journey_and_the_generator_match_their_manifest` | corpus, jornada e gerador são os que o manifesto hasheia |
| `the_journey_leaves_the_transform_and_the_history_the_golden_trace_says` | transformada e profundidades de undo/redo do golden; commit, cancel, undo e redo |
| `a_core_run_skips_only_what_needs_a_window_and_says_why` | o core roda só os passos `document` (29 de 41) e diz por que pulou o resto |
| `every_request_plans_the_scene_at_most_as_often_as_its_contract_says_on_every_corpus` | teto de layouts por rótulo, nos três corpos (abaixo) |
| `the_layouts_of_a_request_do_not_depend_on_the_size_of_the_scene` | mesma contagem em 6 e em 618 nós |
| `a_long_drag_costs_one_layout_per_move_and_nothing_else_grows` | 60 moves = 60 planos |
| `the_overlay_memo_key_is_free_stable_and_changes_with_everything_that_moves_the_picture` | a chave de memória do overlay não faz layout |

O teto do core (`layout_budget`) é por rótulo: `scene.gizmo.state`, `scene.pointer.move`, `scene.pointer.up` e
`scene.pointer.cancel` custam 1; `scene.pointer.down` custa 2 (acha o que está sob o ponteiro e planeja o candidato);
`scene.gizmo.tool`, `scene.select`, `scene.attach`, `project.create`, `history.undo`, `history.redo` e `seek` custam 0.
É um **limite superior**: uma otimização pode baixar o número sem falhar.

**`scene_gizmo_perf_threads.rs`** (1 teste, binário próprio com um único `#[test]`):
`no_layout_escapes_the_thread_the_suite_reads_and_the_detector_sees_one_that_does`. A jornada gasta os mesmos layouts
por requisição lida por thread e por processo, e um **controle negativo** roda a jornada noutra thread e exige que o
contador da thread não mude e o do processo cresça. Cobre o ponto cego do contador por thread (um handler que
planejasse noutra thread leria 0). Nenhum mutante de produto foi rodado contra esse controle.

**O que o detector estrutural NÃO vê:**

- **O custo de cada layout.** Ele conta chamadas, não tempo: um planejador mais lento, com o mesmo número de chamadas,
  passa. Só uma comparação de tempo com baseline independente veria isso, e ela não foi validada (lacuna (6)).
- **O caminho de `PlanTimeline`.** `PlanTimeline::build_with` **não é contado**: o contador antigo não tinha
  consumidor (código morto em caminho quente) e foi removido (lacuna (2)).
- **Trabalho dentro do layout** (nós visitados, multiplicações de matriz): um contador determinístico disso é
  previsto e não existe (lacuna (7)); mediria trabalho, não tempo.

## Host GPUI

`scene_gizmo_perf_tests.rs` roda a **mesma** jornada (mesmo `journey.json`, mesmo driver, mesmas asserções sobre a
transformada, o histórico e as igualdades de commit, cancel, undo e redo) por `Workstation::run_op`, o handler que o
socket da janela e a CLI compartilham, numa janela de teste do GPUI. O que isso acrescenta ao core: o trabalho do
próprio host em volta de cada comando (o plano do monitor, a memória do overlay, a adoção do overlay que um comando
devolveu), contado por requisição, e os passos de janela que uma janela de teste consegue rodar (`monitor.show_scene`,
redimensionar). Os 5 testes:

| Teste | O que afirma |
|---|---|
| `an_idle_window_plans_nothing_per_tick_and_the_overlay_stays_memoised` | 100 ticks do timer = 0 layouts; 20 syncs de um estado igual = no máximo 1 layout por sync (o do próprio monitor), em cota agregada; o overlay não é pedido de novo (só `memo_hits` cresce) |
| `showing_a_scene_that_shares_its_bundle_with_the_shown_one_shows_it` | BL015.22 no nível da jornada (`same.*`) |
| `monitor_show_scene_follows_the_request_between_scenes_of_one_bundle_and_of_another` | a sequência E1–E6 que reproduziu o defeito |
| `the_host_runs_the_shared_journey_to_the_same_transforms_and_history_as_the_core` | mesmo golden que o core |
| `every_host_request_plans_the_scene_at_most_as_often_as_its_contract_says` | teto por rótulo no host (`host_budget`, abaixo) |

`host_budget`: 0 para `project.create`; 3 para `scene.pointer.down`, `scene.pointer.up` e `monitor.show_scene`
(contrato do core mais o do próprio monitor); 2 para os outros dez rótulos da jornada. **O teto não é o medido de cada
passo.** É o máximo medido do rótulo, e o comentário da função diz que um número menor nunca falha. **Limite
conhecido:** onde o medido é menor que o teto, **um plano extra passa**; o teste só pega excesso **acima** do teto
(lacuna (9), BL015.30). O controle de "outra thread" só cobre o core. A cota de layouts da janela parada é agregada por
sync, não checada sync a sync: um sync com 2 layouts compensado por outro com 0 passaria.

## BL015.22 — defeito de produto do monitor (corrigido)

Defeito **anterior** a esta suíte, que a jornada encontrou: `monitor.show_scene` de uma cena anexada **do mesmo arquivo
de bundle** que a já exibida (mesmo `bundleId`, mesmo plano, mesmo mapa de recursos) respondia `ok`, mas a cena
dirigida continuava a primeira. Causa: o dedupe de `ViewportHost::render_scene` comparava plano JSON, mapa de recursos
e opções **sem o id da cena** e retornava antes de atribuir `directed.scene`; o overlay e o ponteiro da página escolhem
a cena por `directed_scene()` (lido no código), então seguiriam a cena antiga.

- **Correção** (`bab9290`, `scene_monitor_host.rs`): a chave passa a ser `format!("{scene:?}|{}|{:?}|{:?}", plano,
  mapa, opções)`; o `{:?}` aspeia o id, que não colide com o JSON do plano. Repetir a mesma cena segue sem reenviar
  nada; uma troca de cena (do mesmo bundle inclusive) registra `SceneChanged` uma vez. Nenhum `Command`, nó de
  `tree.rs` ou parser novo; o core não mudou.
- **Testes** (três): os dois do host acima (`showing_a_scene_that_shares…` e a sequência E1–E6) e
  `scene_lifecycle_tests::a_scene_of_the_same_bundle_at_the_same_frame_is_another_scene_and_a_repeat_of_one_scene_is_not`.
  O `showing_a_scene_that_shares…` **falhava em `a97a0a7`** (`left: Some("proof-b")`, `right: Some("proof-c")`), antes
  da correção. Não se afrouxou teste nem corpus.
- **Verificado no host** (árvore limpa de `c3ae380`): os 5 testes `scene_gizmo_perf` passam e o crate `tessel-editor`
  inteiro dá 418 passaram, 0 falharam; no head final `c1c8fda`, com a `main` integrada, são 424 e 0 (ver "Estado desta entrega").
- **Controle negativo, só o que o log mostra.** Com a chave **antiga** recolocada
  (`format!("{}|{:?}|{:?}", plan.to_json(), map, options)`), uma execução filtrada rodou 8 testes: **3 falharam** (os
  três acima) e 5 passaram (os outros 3 do host e 2 rascunhos de investigação que não fazem parte da árvore). Essa
  execução foi sobre o head `9a72e03` com a **árvore suja**: as mudanças sob teste ainda não estavam commitadas. O log
  **saiu do git** por conter dados da máquina de desenvolvimento (caminhos e diretório de trabalho). Para repetir:
  recoloque a linha antiga da chave em `scene_monitor_host.rs` e rode `./scripts/cargo test -p tessel-editor --locked --
  scene_gizmo_perf a_scene_of_the_same_bundle`; o esperado são os três testes acima falhando.
- **NÃO provado:** o efeito na **janela nativa**. Uma corrida da janela real em `12bc8da` (pilha de tempo) passou
  os 15 checks `same.*` e `switch.*` de cada corpo, mas a corrida inteira deu `FAIL` por outros motivos e **não há
  controle com a chave antiga na janela**. Um relatório nativo antigo mostrou o monitor indo à cena pedida numa corrida
  em que o defeito não apareceu, e a diferença para o host com a chave antiga **não está explicada**.
- Biblioteca e CLI headless: **N/A justificado** (o monitor é um adaptador de apresentação do GPUI; o core não tem
  monitor, cena dirigida nem chave de dedupe).

## CLI headless

`./scripts/headless` já executa o mesmo vocabulário JSONL (`scene.*`, `history.*`, `project.create`); o driver do core
usa `headless::execute`, o mesmo handler. A suíte **não** acrescenta comando novo (nenhum controle novo: nada em
`tree.rs` nem parser). Como rodar, sem janela:

```
./scripts/cargo test -p tessel-editor-core --locked --test scene_gizmo_perf_structure   # estrutural, 7 testes
./scripts/cargo test -p tessel-editor-core --locked --test scene_gizmo_perf_threads     # nenhum layout escapa da thread lida, 1 teste
./scripts/cargo test -p tessel-editor --locked -- scene_gizmo_perf                      # host GPUI, 5 testes
python3 scripts/gizmo-perf/corpus.py --check                                            # corpus, manifesto e gerador conferem
python3 scripts/gizmo-perf/hygiene.spec.py                                              # nenhum dado da máquina em arquivo versionado
```

O host GPUI é uma janela de teste do GPUI, **não** é execução headless do core. Biblioteca e CLI sem janela cobrem só
as duas primeiras linhas.

## Tempo e janela nativa

**A medição de tempo tem contrato próprio:** [scene-gizmo-performance-timing.md](scene-gizmo-performance-timing.md) (runners
em Python, orçamentos, comparação com baseline, mutantes de tempo e a janela nativa com WKWebView). Motivo da separação: essa
pilha **nunca produziu um PASS válido nem uma comparação medida**. Os quatro runners rodaram pela primeira vez em
`12bc8da`, sob carga de base no limite da guarda de carga: o runner de tempo do core deu `FAIL` (uma execução), o A/B
deu `INVALID_GUARD_AFTER_RUN` nas duas tentativas (nada julgado), o runner de mutantes terminou `UNEXPECTED` e a janela
nativa deu `FAIL` funcional (uma execução; 20 de 639 verificações, todas em `drag.*`). **Nada disso valida
desempenho.** Os logs dessa execução **saíram do git**
por conter dados da máquina de desenvolvimento; reexecutar é um comando do contrato de tempo, em máquina silenciosa.

## Estado desta entrega

Regra: **implementado** ≠ **validado**. Cada linha diz em que head foi executada; o que não tem head fica `a validar`.

| Item | Estado | Head |
|---|---|---|
| Estrutural do core (7 testes) | passa (7/7) | `c1c8fda` (merge da `main` `ca06345`, árvore limpa) |
| Threads do core (1 teste, com controle negativo) | passa (1/1) | `c1c8fda` |
| Host GPUI `scene_gizmo_perf` (5 testes) e o teste de mesmo bundle de `scene_lifecycle_tests` (1) | passam (5/5 e 1/1); crate `tessel-editor`: 424 passaram, 0 falharam | `c1c8fda` |
| Teste de mesmo bundle (BL015.22) | **falhava**, passa depois da correção | falha em `a97a0a7`; passa em `c3ae380` e em `c1c8fda` |
| Controle negativo do BL015.22 (chave antiga) | 3 falham, 5 passam | `9a72e03`, árvore suja; log fora do git |
| `fmt --all -- --check`, `check --workspace`, `clippy --workspace --all-targets -D warnings`, `build -p tessel-editor`, `cargo tree -p tessel-editor-core` sem gpui/wry | exit 0 | `c1c8fda` |
| `test --workspace --locked --no-fail-fast` com `TESSEL_REQUIRE_RUNTIME=1` | 2340 passaram, **1 falhou**, 13 ignorados. A falha é `a_full_disk_ends_the_export_with_disk_full_and_leaves_the_volume_clean` (`hdiutil: attach failed - Permissão negada`): com o `TMPDIR` da corrida num volume externo o `hdiutil attach` foi negado (duas vezes); com o `TMPDIR` padrão o mesmo teste passou (1/1). Causa não investigada; nada em `crates/media` mudou nesta PR | `c1c8fda` |
| `corpus.py --check`, `hygiene.spec.py` (10 testes) e as outras 18 specs de `scripts/*.spec.py` (313 testes) | exit 0 (19 specs, 323 testes, nenhuma falhou) | `c1c8fda` |
| Efeito do BL015.22 na janela nativa | **não provado** | — |
| Desempenho (tempo) | **NÃO validado** | — |

`c1c8fda` é o merge da `main` no branch da PR #44. O commit seguinte, que escreve esta tabela, só altera
documentação: o código, os scripts e os fixtures do head final são os de `c1c8fda`. A máquina estava ocupada por outros
trabalhos (média de carga de 1 minuto entre 9 e 74 durante as corridas); nenhum teste desta PR mede tempo.

Os mutantes sintéticos que provaram que o detector discrimina (um plano a mais por handler morto pelo estrutural; o
replanejar por tick morto pelo host) foram rodados pelo runner de mutantes (`mutate.py`, [contrato de tempo](scene-gizmo-performance-timing.md)), uma vez, em
`12bc8da`. O runner mudou depois e **não foi rodado de novo**: a discriminação do detector estrutural só é reproduzível por
ele, a validar, fora o controle negativo do teste de threads.

## Lacunas

Lista numerada herdada do item BL015.23 de [`docs/backlog.md`](../backlog.md). **A numeração (1) a (36) é a das
lacunas e não deve ser confundida com os IDs BL015.24 a BL015.36**, que são dívidas separadas (registradas em
`docs/backlog.md`). `[A]` = cobertura estrutural ou do host (PR #44); `[B]` = pilha de tempo ou janela (PR #49).
"Runner corrigido, a validar" = o código mudou e nunca foi visto numa execução real.

1. [A] **Sem caso com splat.** O corpus sintético pequeno não tem asset de splat. Aberto.
2. [A] **`PlanTimeline::build_with` sem detector estrutural** (o contador morto foi removido). Aberto.
3. [B] **Compositor e `presented` não observáveis na janela.** Um quadro novo correlacionado não é prova física de pintura. Aberto.
4. [A][B] **Sem orçamentos por passo**: o host usa teto por rótulo (item 9) e a janela nativa não tem orçamento. Aberto.
5. [B] **`pointer.move`/`up`**: a razão requisição/plano de uma calibração antiga (relatório fora do git) é alta mesmo com 1 layout por requisição; o resto seria do gesto (hipótese, não medida). Sem baseline; nada foi otimizado. Aberto.
6. [B] **Requisito de processo (M2(a)).** O estrutural conta chamadas, não custo; um planejador mais lento passa nele e nos orçamentos de tempo (derivação do desenho do teste, **não executada**). A única comparação absoluta prevista é o A/B da pilha de tempo, contra uma baseline fixa no código; ele só vê o que passa dos limiares dele sobre o total da requisição (um layout de 1 para 2 ms numa requisição de 13 ms sobe 7,7% e escapa: aritmética, não medida) e **não vê host, renderer nem WebView**. Essa baseline fixa **recusa** hoje o head mergeado com a `main` atual (BL015.29), então o requisito não é executável sem `--baseline` e `--allow-other-baseline`, rota **sem teste**. Aberto.
7. [A] **Contador determinístico de trabalho dentro do `probe`** (nós visitados, multiplicações de matriz). Mede trabalho, não tempo; um layout mais lento com as mesmas contagens passaria, então só estreita a lacuna (6). Não feito: muda produção. Aberto (BL015.31 prepara o `cfg`).
8. [B] **Tick longo absoluto decide o veredito** (`long_ticks_max: 0` reprova por uma amostra > 16,6 ms): falso FAIL possível, não falso PASS. Aberto (BL015.28).
9. [A] **Tetos do host com folga** (limite m7): um plano extra passa onde o medido é menor; o controle de "outra thread" só cobre o core. Aberto (BL015.30).
10. [B] **`within()` do teste de orçamento** não exige que todo tipo orçado tenha sido medido. Compilado, executado uma vez. Aberto.
11. [B] **Nits do teste de orçamento e dos runners:** a injeção exige `--test-threads=1` (falta comentário); o runner de mutantes não roda os testes sem mutar antes de afirmar um kill; `MAX_BLOCK_SPREAD` × `budgets.json`; a guarda pós-corrida pode se autodisparar (uma corrida de ~1 CPU soma ~1 à carga de 1 min; recusou em 5 das 8 execuções guardadas de `12bc8da`). Aberto.
12. [A] **Cabeçalho de `corpus.py`** dizia 5 nós e 605/617. Corrigido nesta PR para 6/618/44, os números do manifesto (o manifesto foi regenerado junto: só o sha do gerador mudou).
13. [B] **(a)** `--allow-dirty` mudava status e exit de modos diferentes em cada runner: uma corrida suja que seria `PASS`, "sem regressão" ou sucesso funcional termina agora com status próprio e exit 1 nos três (`run.py`, `ab.py`, `native.py`), só com dublês, a validar; **(b)** o runner de mutantes apagava `exes/` depois de uma corrida inválida e tratava como resposta um teste de tempo que passou com a guarda depois ruim: runner corrigido, a validar; **(c)** a spec não cobre `--baseline` e `--allow-other-baseline` do A/B; **(d)** o aviso "Docstring Coverage" do bot não foi tratado. (c), (d) abertos.
14. [B] **Não existe comparação baseline × candidato para host, renderer nem WebView.** O A/B só roda o teste de orçamento do core; ele não mediria a mudança do monitor desta PR (BL015.22). Sem desenho. Aberto.
15. [B] **Build do A/B** morreu uma vez quando o executável do candidato sumiu após o build da baseline (poda de cache; causa não isolada). Runner corrigido (cópia imediata, estados explícitos), a validar. Aberto: manifesto `BUILT` sem `exit` nem `utc_end`.
16. [B] **Janela: 15 de 15 arrastos reprovaram "quadro novo" após a soltura.** Pelo código e pelos registros, o host deduplica um plano igual ao do candidato; nenhum defeito de produto indicado (inferido, não medido). Runner corrigido (`redrawn` ou `deduped`), a validar numa janela real.
17. [B] **Janela, `dense`: a escala não mudou um pixel.** Explicação por simulação (nó de 0,1 unidade): inferida, não medida; defeito de produto não excluído. Runner corrigido (`may_stay_equal` só nessa escala), a validar; a prova por pixel exigiria um nó opaco no corpus.
18. [B] **`overlay_drawn` gravado e não checado.** Runner corrigido em parte (reconhecimento do passo com overlay visível), a validar.
19. [B] **`memo_off` do host** morria por uma verificação que o runner de mutantes não esperava. Corrigido em parte; o log bruto do host não é guardado. A validar.
20. [B] **Runner de tempo do core: `FAIL` na única corrida real** (`12bc8da`), com carga no limite da guarda: nada separa ruído de regressão. Nenhum orçamento ou margem mudou. Aberto.
21. [B] **A/B apagava a tentativa anterior.** Corrigido (`attempts/`), a validar.
22. [B] **`tests_passed` listava teste que falhou.** Corrigido, a validar.
23. [B] **Passos de overlay oculto** não esperam nem conferem reconhecimento da página. Aberto.
24. [B] **`trace.dropped`, `picture_summary.all_read_back` e `overlay_report.stale`** são gravados e não julgados. Aberto.
25. [B] **Lock do checkout é por caminho:** dois worktrees dividem o cache do `mbx`, cuja poda é a causa provável de (15). Aberto.
26. [B] **Redo só conferia "quadro mais novo"; pilhas de histórico só andavam se o undo desenhasse.** Corrigido no runner, a validar numa janela; resta o `before_hash` (32).
27. [A] **Sem teste de host do dedupe** de um pedido não explícito cujo plano é igual ao último (o comportamento por trás de (16)): só é coberto de lado (id da cena na chave; o teste de layouts do host conta layouts, não quadros). Aberto.
28. [B] **`trace.enabled`** gravado e não julgado. Aberto.
29. [B] **Identidade `dirty` olha nove caminhos**; `include_str!` e `include_bytes!` de assets e fixtures fora deles não sujam. Aberto.
30. [B] **Os builds herdam `GIT_*`** do chamador. Aberto.
31. [B] **`redoable` do runner nunca é esvaziada** num commit novo (o core esvazia). Sem veredito errado na jornada de hoje. Aberto.
32. [B] **`before_hash`** só serve de portão do undo, sem registro quando falta. Aberto.
33. [B] **`present.resize.*`** aceita qualquer quadro convergido. Aberto.
34. [B] **`check_deadline` e `CORPUS_TIMEOUT_S`** (1200 s) sem teste. Aberto.
35. [B] **`commit_job` mede 60 s com `time.time()`** (relógio de parede). Aberto.
36. [A] **Nada exercita luz.** A luz pontual (F060, PR #45: posição e `scene.range`) está nesta árvore desde o merge da `main`, mas o corpus e a jornada não têm luz nem passo de luz: nenhum pedido de luz passa pelo detector nem tem teto de layouts. Aberto.

Dívidas da revisão desta PR (BL015.24 a BL015.36) e o que cada uma exige para fechar: `docs/backlog.md`.
