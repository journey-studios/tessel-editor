# Entrada de cena: plano de correção e prova (BL157, BL158)

Plano da PR #34 (`t3/perf-scene-entry`). Evidência e resultado: [viewport-scene-entry-report.md](viewport-scene-entry-report.md).
Contrato: [features/viewport.md](features/viewport.md) (F021). Status e atribuição: só em [backlog.md](backlog.md) (BL157, BL158; dependem de
BL146 e BL129–BL131; nenhum ID novo).

## O que a PR entrega

1. **BL157 — aquecimento da cena.** `frame.warm {plan}` (feature aditiva `scene.warm`): o runtime cria o estágio Spark da linhagem de um
   corte futuro e inicia a primeira ordenação antes do quadro que o desenha.
2. **BL158 — nova tentativa do quadro com `timeout`.** O host repete o pedido (1 s, 3 s); a partir da segunda falha a sessão desiste das
   cargas que o quadro esperava (`cancel_timed_out`) e as pede de novo.
3. **Chave de A/B.** `TESSEL_VIEWPORT_WARM=0` (host) desliga o envio de `frame.warm` sem tocar no runtime: o **mesmo binário** mede os
   dois lados.

Fora desta PR: a causa de a espera de mídia expirar (BL158 segue aberto nisso), BL155 (LoD), memória de GPU, o harness antigo
(`play_tour.py`, `mkrt.py`, `phase-runtime.py`), que nunca foi versionado e **não é reutilizado**: o harness desta PR é novo e está no
repositório (`scripts/scene-warm-proof.py`).

## Achados das 15 conversas da PR e onde cada um é fechado

| # | Achado (origem) | Veredito | Correção | Prova que o reprovaria |
|---|---|---|---|---|
| 1 | `makeRoom(keep)`/`trim()` passam de `maxEntries` e liberam o estágio protegido (OCR) | válido | `LruCache.makeRoom(keep)` devolve se a entrada cabe, `trim(keep)` só tira o que não está protegido; `stageOf` falha com `resource_budget` em vez de passar do limite | `retention.test.mjs` ("never frees a kept entry…") e `scene-warm.test.mjs` ("keeps the stage budget and the warm stage") falham no runtime anterior |
| 2 | `warmScenes` e `planLineages` numeram linhagens diferente (OCR) | válido | uma só função, `splatUses`, numera `(asset, nth)` sobre **todas** as camadas de cena com o asset, cobertas ou não; só as desenhadas pedem estágio | `scene-warm.test.mjs` "two layers of one scene are numbered alike by the warm and by the frame, covered or not" |
| 3 | aquecimento empresta malha sobressalente sem reservar dono (OCR) | válido | `reserveSplatOwners([...drawnOwners, ...warmedOwners()])` em volta da montagem do rig e no início de cada quadro | `scene-warm.test.mjs` "a warm takes no spare mesh from the scene on screen…" |
| 4 | quadro que espera a ordenação ignora cancelamento (OCR) | válido | `awaitWarm(lineage, aborted)` consulta o cancelamento a cada 25 ms e limita a 1 s (`WARM_WAIT_MS`) | `scene-warm.test.mjs` "a frame that waits on a warm sort stops waiting when a newer frame replaces it" |
| 5 | `rig_with_scene_cut` divide diretório entre testes em paralelo (OCR) | válido | um diretório por chamada (contador atômico) | `crates/core/src/session/tests.rs`, 63 testes de sessão verdes em paralelo |
| 6 | cena dirigida ignora a contabilidade de tentativas; `retry_due` velho reagenda erro que não é `timeout` (OCR) | válido | `note_change` (a janela diz que o usuário mudou algo), `clear_failure`/`delivered`/`show_failure` únicos; o caminho dirigido usa os mesmos | `scene_lifecycle_tests.rs` (2 testes dirigidos), `viewport_host_tests.rs` (3 testes) |
| 7 | CodeRabbit major: o reinício de tentativas não chega à cena dirigida | válido (o mesmo de 6) | idem | idem |
| 8 | `prefetch_ahead` esconde o aquecimento atrás de `plans.is_empty()` (CodeRabbit) | válido | sempre chama `session.warm(&plans)` (limpa o que saiu da janela) | `viewport_host_tests.rs` |
| 9 | contrato do helper de `viewport_host` na doc e estado/gates anteriores à integração (CodeRabbit, backlog) | válido | `features/viewport.md` e backlog atualizados; gates refeitos no head | este plano e o relatório |
| 10 | harness `play_tour.py` não versionado (CodeRabbit) | válido | `scripts/scene-warm-proof.py` + `.spec.py` | `python3 scripts/scene-warm-proof.spec.py` (24 testes) |
| 11 | contagem de execuções (3 declaradas, 2 mostradas) (CodeRabbit) | válido | a seção antiga é substituída por uma tabela com **todas** as execuções, de uma única rodada nova | relatório |
| 12 | `flush` é CPU, não GPU (CodeRabbit) | válido | os campos são chamados de CPU em toda parte | relatório |
| 13 | média de 22 ms e 3 de 5 entradas não provam < 60 ms (CodeRabbit) | válido | latência **por entrada**, todas as entradas, todas as execuções; aceite continua **a validar** enquanto os dados não o mostrarem | relatório |
| 14 | mediana caiu, pico máximo subiu (CodeRabbit) | válido | mediana e pico reportados por execução; nenhuma afirmação de que o pico não cresce | relatório |
| 15 | interferência de CPU/GPU sem comparação controlada (CodeRabbit) | válido | causa **inconclusiva**, registrada assim | relatório |

Conversas **desatualizadas** (outdated): a de `docs/backlog.md` sobre o harness e as demais do `f54` foram relidas contra o head novo; o
veredito acima já considera cada uma. Nenhuma conversa é resolvida nem respondida sem o código/dado que a fecha.

## Provas exigidas e a superfície de cada uma

| Propriedade | Biblioteca (core) | CLI headless | Runtime em Chrome real (SwiftShader) | Janela real (GPU), `scripts/ui`/socket |
|---|---|---|---|---|
| cancelamento do quadro que espera a ordenação | — | N/A: o aquecimento é do monitor | `scene-warm.test.mjs` | — (não há op para cancelar na janela sem alterar o produto; ver limites no relatório) |
| dono da malha | — | N/A | `scene-warm.test.mjs` | pixel igual (abaixo) |
| orçamento e retenção | `session/tests.rs` (bytes), `retention.test.mjs` | N/A | `scene-warm.test.mjs` | `stats.evicted`/`warmed` no relatório |
| perda de contexto | `context_loss_reloads_assets_and_asks_for_a_redraw` | N/A | `scene-warm.test.mjs` "a context lost while a warm sorts leaves nothing behind…" | não induzida na janela (limite) |
| pixels com e sem aquecimento, ordem quente e fria | — | N/A | `scene-warm.test.mjs` (hash idêntico a um runtime que nunca aqueceu) | `scene-warm-proof.py` (4 sessões, `render.frame` com `TESSEL_VIEWPORT_INSPECT=1`) |
| retry/limpeza/cancelamento de carga | `session/tests.rs` (4 novos) | N/A | — | testes GPUI de `viewport_host`/`scene_lifecycle` |
| custo da entrada, A/B no mesmo build | — | N/A | — | `scene-warm-proof.py` (pares alternados) |

"N/A" do headless é justificado: `frame.warm` e a retentativa pertencem ao monitor nativo, e o headless não pré-carrega nem aquece. O
headless continua sendo a superfície que renderiza o mesmo projeto sem monitor (`./scripts/headless`), e a prova não declara paridade
entre os dois rasterizadores.

## Gates do head integrado

`cargo fmt --all -- --check`; `./scripts/cargo check --workspace --locked`; `clippy --workspace --all-targets --locked -- -D warnings`;
`TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked --no-fail-fast`; build debug e release de `tessel-editor`;
`./scripts/runtime setup` e os testes de Chrome real **sem SKIP**; `cargo tree -p tessel-editor-core` sem `gpui`/`wry`; prova na janela
própria. Cada log carrega o SHA do head e o hash do binário; um log de antes de uma correção não vale depois dela.
