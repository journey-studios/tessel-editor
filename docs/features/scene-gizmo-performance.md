# F055 (desempenho) — Suíte de regressão de performance dos gizmos e da abertura (BL015.21)

Complementa [F055](scene-gizmo.md). Pedido: "garantir que não perdemos performance e que validamos o mesmo
cenário de abrir o app visualmente; o que for possível por teste unitário". Nada aqui muda o produto: o único
código de produção tocado são três linhas de contagem (`scene_plan::probe`, ver abaixo).

## Objetivo e limites

Uma **jornada única e versionada** (`docs/migration/fixtures/gizmo-perf/journey.json`, v1: abrir projeto, anexar,
resolver recursos, exibir, primeiro quadro, selecionar, translação/rotação (local e mundo)/escala (uniforme e por eixo),
confirmar, cancelar, undo/redo, auxiliar oculto, trocar de cena, redimensionar, diálogo) sobre um **corpus sintético
compartilhado** (`corpus.manifest.json`, com sha256 do gerador, da jornada e de cada arquivo; nada privado):

| Caso | O que é | Por quê |
|---|---|---|
| `small` | o corpus da prova de cena (5 nós, animados) | abertura típica |
| `dense` | + 12 bancos × 50 planos girados (617 nós) | custo que cresce com o número de nós |
| `deep` | cadeia de 24 pais girados + 12 anisotrópicos | pais transformados (matemática da direção local/escala) |

Gaps declarados: **não há caso com splat** (o gerador não cria splat; o pedido o inclui "onde suportado"); sem GPU real
nem ponteiro físico (BL015.2/BL043/BL044 seguem pendentes); sem alegar "custo 0".

## Contrato do core / biblioteca

A jornada roda pelos **mesmos handlers** nas três superfícies; cada passo é `document` (equivalente: mesmo estado,
mesmo undo), `presentation` (precisa de uma janela que apresente) ou `presentation_only` (resize/diálogo):

| Superfície | Onde | O que prova | O que **não** prova |
|---|---|---|---|
| Core (lib) | `crates/core/tests/gizmo_perf_support` + `scene_gizmo_perf_structure.rs` (7 testes, **sempre** rodam, determinísticos) e `scene_gizmo_perf_budget.rs` (`#[ignore]`, opt-in controlado) | transformada/undo/redo idênticos ao golden (`journey.expected.json`), cancelar não muda nada, desfazer restaura o valor exato; **planos por requisição** (`scene_plan::probe::layouts_built`), independência do tamanho da cena, 60 moves = 60 plans, chave da memória do overlay; tempo em release | janela, GPU, página |
| Host GPUI | `crates/desktop/src/workstation/scene_gizmo_perf_tests.rs` (3 testes) | a MESMA jornada por `Workstation::run_op` (mesmo golden), incluindo `monitor.show_scene` e resize; layouts por requisição **no host** (orçamento = o medido); janela parada (100 ticks do timer = 0 layouts; 20 syncs = no máximo 1 layout e 0 pedidos do overlay, só `memo_hits`) | WebView, GPU, compositor, ponteiro físico; o diálogo (precisa do `Root` do Kit: é da janela nativa) |
| Janela nativa | `scripts/gizmo-perf/native.py` (processo próprio, `HOME`/socket próprios, WKWebView, só requisições semânticas) | a mesma jornada contra o mesmo golden, **pixels** (a imagem do monitor muda após um commit, volta ao hash exato após o undo, não muda após cancelar), o overlay desenhado e confirmado pela página (sondas de cor), tempos do cliente e os do próprio runtime | compositor (ver abaixo) |

`scene_plan::probe` (core, sem `unsafe`: o workspace proíbe `unsafe_code`, logo não há contador de alocações nem de
CPU; limite declarado) conta cada `ScenePlan::build_with` e `PlanTimeline::build_with` da thread. É o **detector
estrutural**: um plano a mais por handler ou por tick falha o teste sem relógio.

**Tempo.** A medição de tempo é a razão entre a requisição e o `ScenePlan::build_with` da própria cena, medido em blocos
de 5 antes de cada janela de amostras; isso cancela a velocidade e a carga da máquina. Orçamentos por caso e tipo em
`budgets.json` (regra e margens em `scripts/gizmo-perf/budgets.py`, fixadas **antes** de julgar o candidato; o arquivo
guarda o sha256 dos relatórios de calibração). Métricas: mediana, p95, p99, máximo, amostras perdidas, erros, ticks
longos (> 16,6 ms); **nenhuma amostra é descartada** e frio/quente ficam separados (frio = 60 editores novos por tipo,
primeiro quadro incluído). `INCONCLUSIVE` quando mais de 25% dos blocos de referência discordam de si mesmos em > 1,5×;
`BLOCKED` quando o lock `/tmp/tessel-scene-gpu.lock` está ocupado (uma tentativa, sem espera), a carga passa de
0,5 por CPU, há limite térmico ou o disco está abaixo do piso. As duas "injeções" (um plano a mais; 1 ms) são **controles
negativos** do próprio detector, nunca prova de desempenho.

## CLI headless

`./scripts/headless` já executa o mesmo vocabulário JSONL (`scene.*`, `history.*`, `project.create`); o driver do core
usa `headless::execute`, o mesmo handler. A suíte **não** acrescenta comando novo (nenhum controle novo: nada em `tree.rs`
nem parser). Como rodar:

```
./scripts/cargo test -p tessel-editor-core --locked --test scene_gizmo_perf_structure     # estrutural, sempre
./scripts/cargo test -p tessel-editor --locked scene_gizmo_perf                          # host GPUI
python3 scripts/gizmo-perf/run.py --out DIR            # tempo no core (release, lock, guardas, manifesto)
python3 scripts/gizmo-perf/ab.py --phase build|measure # main (cbe9e1b) × candidato, mesma fonte de medição
python3 scripts/gizmo-perf/mutate.py --phase prepare|time
python3 scripts/gizmo-perf/native.py --binary target/release/tessel-editor   # janela real (precisa ./scripts/runtime setup)
```

## UI e CLI da janela

`native.py` usa o socket (`op`/`get`/`set`/`invoke`), nunca coordenadas nem teclas. Captura nativa
(`screencapture -l`) é só observacional (uma por caso). Resize e diálogo são `ui set window size` e `ui invoke flow.jobs`.
Apresentação: o relatório do runtime traz `render_ms`/`total_ms` e o hash lido de volta de cada quadro (`last_frame`) e os
vãos do host (`plan`, `evaluate`, `ipc`, `serve`); os quadros de cena dirigida **não** recebem o carimbo de apresentação
da sessão (`frames_total` = 0), então `presented_after_done_ms` não existe para eles e o último salto (compositor) **não é
medido**. Uma resposta de socket ou o `frame.done` da página nunca contam como prova de pintura.

## Evidências e status

Preenchido com os números da entrega em [Evidência](#evidência-desta-entrega). Regra de leitura: **implementado** (código
e testes que rodam) ≠ **validado** (números/provas com identidade); o que é autorrelato fica marcado como tal.

### Achados (não são regressões provadas)

- `pointer.move`/`up` custam ~6–8× um plano no release (denso ≈ 4 ms) contando um único layout: o resto é do gesto
  (candidato, planos de escala/rotação). Sem A/B mostrando regressão contra a `main`, **não** foi otimizado.
- `monitor.show_scene` entre duas cenas **com o mesmo `bundleId`** (duas cenas anexadas do mesmo arquivo) responde `ok`
  mas o monitor continua em `directed.scene` da primeira (nenhum quadro novo é pedido); com bundles diferentes troca.
  Observado uma vez na janela real (build release do head); a causa não foi investigada e nada foi alterado. A jornada usa bundles diferentes na troca; o caso fica em BL015.22.
