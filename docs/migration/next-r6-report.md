# R6 — Custo e tamanho do plano para muitos frames (BL064)

Contrato e evidência resumida: [features/scene-plan.md](../features/scene-plan.md) (F037, "Muitos frames" e "BL064 — medição").
Estado de execução fica só no backlog; este relatório entrega o resultado e os patches para o coordenador.

## Resultado

**Medido primeiro; ganho comprovado no plano, pequeno no laço de exportação; otimização implementada de forma aditiva.**

1. **Onde o custo está** (colunas do benchmark, release): `build` = avaliação do frame (`evaluate_frame`, 9–18% do `build` em cenas com nós) + montagem independente do frame + `validate` (~30–40% do `build`). Refazer `NodeTimeline::new`/`CameraTimeline::new` por frame — a hipótese original de BL064 — é só ~3% do `build`, exceto em câmera pesada (~90–97%). O JSON (`to_json_string`) custa 4–5× o `build` e domina o laço de exportação.
2. **`PlanTimeline`** (novo, aditivo) prepara por *shot* o que não depende do frame (instância, clipe de câmera, grafo, molde validado) e, por frame, só reavalia câmera, estado dos nós e matrizes. Mesmo plano, mesmos erros, mesmas capacidades.
3. **Ganho do plano**: 3,0–5,1× (28× em câmera pesada). **Ganho no laço plano + JSON: 1,09–1,24× (2,7× em câmera pesada)** — é o JSON que manda.
4. **Descartado explicitamente**: um índice mais leve em `NodeFrame` (≈10% do caminho quente; cirurgia em struct pública, sem o ganho que justifique). **Não implementado**: escritor JSON direto byte a byte igual (proposta de BL abaixo) e redução de bytes do formato (exige `PLAN_VERSION`).

## Medição (release, mediana, árvore pristina `914c830` contra árvore com `PlanTimeline`)

Parâmetros: `cargo test --release`, **3 repetições de processo × 5 passadas × 120 frames amostrados por cena**, memória com **20 000 frames** seguidos; sintéticas sem mídia privada. Tempo em µs por frame. Reprodução: `python3 scripts/scene-plan-performance/run.py --tree . --base <árvore pristina> --repeats 3 --reps 5 --frames 120 --long 20000 --out result.json`. Ruído da máquina observado: ±10%, com pontos isolados de +40% (um deles, `medium` `evaluate`, não se repetiu em nova medição de 5 repetições: 29,7 → 24,4 µs).

### Baseline (main pristina)

| cena | nós | frames da edição | build | evaluate | setup da timeline % | validate | to_json | JSON KiB/frame | JSON MiB/edição |
|---|---|---|---|---|---|---|---|---|---|
| small | 11 | 240 | 18,1 | 3,0 | 2,8 | 5,7 | 86,2 | 8,3 | 2,0 |
| medium | 111 | 1440 | 147,6 | 25,9 | 2,8 | 58,2 | 753,5 | 65,4 | 92,0 |
| large | 1051 | 7200 | 1394,0 | 234,9 | 2,9 | 549,7 | 6892,3 | 598,0 | 4205 |
| camera-heavy | 6 | 10 000 | 100,7 | 92,0 | 89,4 | 3,0 | 49,0 | 4,9 | 47,9 |
| channel-heavy | 25 | 2400 | 43,7 | 9,9 | 5,6 | 12,8 | 193,2 | 20,7 | 48,4 |
| wide-flat | 4001 | 1200 | 5560,5 | 476,1 | 0,8 | 2069,2 | 27646,6 | 2223,0 | 2605 |

### Depois (`PlanTimeline`; `build` é o caminho direto, que não regrediu)

| cena | build direto | quente | frio (1ª pergunta do shot) | ganho quente | plano + JSON (laço) |
|---|---|---|---|---|---|
| small | 19,2 | 5,4 | 5,7 | 3,6× | 1,14× |
| medium | 148,2 | 42,5 | 45,8 | 3,5× | 1,17× |
| large | 1502,0 | 373,8 | 430,8 | 4,0× | 1,09× |
| camera-heavy | 104,0 | 3,7 | 4,6 | 28,3× | 2,74× |
| channel-heavy | 45,3 | 15,0 | 15,2 | 3,0× | 1,24× |
| wide-flat | 5626,2 | 1107,7 | 1273,8 | 5,1× | 1,14× |

"Direto depois" ficou dentro das faixas de repetição do baseline (por exemplo, large: baseline 1290–1645, depois 1296–1578): diferença é ruído, não regressão. As faixas por cena estão em `result.json`.

### Memória (RSS por `ps`, 20 000 frames, um processo por modo)

| cena | modo | RSS antes → depois (KiB) |
|---|---|---|
| medium | direto | 4800 → 4832 |
| medium | `PlanTimeline` | 4448 → 5056 |
| camera-heavy | direto | 6688 → 6864 |
| camera-heavy | `PlanTimeline` | 6640 → 6768 |

Sem crescimento por frame; a linha do tempo guarda no máximo `instâncias + clipes + shots` preparos (teste `what_a_timeline_keeps_is_bounded_by_the_bundle_not_by_the_frames_asked`). Nenhum plano é guardado.

### Tamanho

O JSON do plano é o custo dominante: ~8 KiB (11 nós) a ~2,2 MiB (4001 nós) por frame; uma edição inteira de `large` são ~4,1 GiB. As matrizes dos nós são ~19–27% do JSON (cada nó leva `local` e `world` de 16 números). Nada foi mudado no formato (`PLAN_VERSION` 1 intacto).

## Arquivos (todos sob o ownership de R6)

| Arquivo | Mudança |
|---|---|
| `crates/core/src/scene_plan/timeline.rs` | novo: `PlanTimeline` (`new`, `scene`, `build`, `build_with`, `prepared_layouts`), cache preguiçoso `OnceLock` por instância/clipe/shot, `Sync`; 1 unit test |
| `crates/core/src/scene_plan/build.rs` | `build_with` dividido em `Prepared` (`new`, `refuse_unsupported`, `assemble`) sem mudar a saída; `ScenePlan::at_frame` (`pub(super)`: reescreve só o que varia e erra tipado se a lista de nós não alinha) |
| `crates/core/src/scene_plan/mod.rs` | `mod timeline`, `pub use PlanTimeline`, doc |
| `crates/core/src/directed_scene/temporal/frame.rs` | `evaluate_frame` delega ao novo `evaluate_frame_with` (`pub(crate)`, aceita timelines emprestadas via `Cow`); `camera_of` vira função associada. Sem mudança de API pública |
| `crates/core/src/directed_scene/temporal/camera.rs` | `#[derive(Clone)]` em `LayerTrack`, `MainTrack`, `CameraTimeline` (para `Cow`) |
| `crates/core/tests/scene_plan_performance.rs` + `scene_plan_performance_support/mod.rs` | 10 testes de equivalência/geração/erro/threads/memória + geradores sintéticos e carregadores do corpus |
| `crates/core/tests/scene_plan_performance_golden.rs` + `scene_plan_performance_golden/digests.json` | saída do planejador fixada na main pristina (56 cenas: 10 oracle + 43 extended + 3 sintéticas), por perfil |
| `crates/core/tests/scene_plan_performance_bench.rs` | medições `#[ignore]` (custo, tamanho, memória) |
| `scripts/scene-plan-performance/run.py`, `run.spec.py` | runner (build release pelo wrapper serial, `wait4` para RSS, agrega medianas/faixas, renderiza markdown) e 4 testes dele |
| `docs/features/scene-plan.md` | contrato `PlanTimeline` + evidência BL064; limite antigo ("refaz `NodeTimeline::new` por frame") substituído pela medição |
| `docs/migration/next-r6-report.md` | este relatório |

Nenhum arquivo de R1/R3 (`scene_operation`, `scene_export`, `scene_project/interaction.rs`, host/render) foi tocado; `Cargo.toml` intacto (só `serde`, `serde_json`, `flate2`; `cargo tree -p tessel-editor-core` sem gpui/wry).

## APIs / seams

- `PlanTimeline<'a>`: `new(&'a DirectedScene)`, `build(frame)`, `build_with(frame, PlanOptions)`, `scene()`, `prepared_layouts()`. Empresta a cena (nova geração = nova linha do tempo); `Sync`.
- `ScenePlan::build`/`build_with`: assinatura e saída intactas.
- `Bundle::evaluate_frame_with` (`pub(crate)`): seam para quem tiver timelines já preparadas; não é API pública.

## Patch de adoção para o coordenador (R1/monitor) — depois de integrar R6

`SceneExportSnapshot` guarda `Arc<DirectedScene>`; `PlanTimeline` empresta, então a linha do tempo vive **no job de export** (uma por snapshot imutável), não dentro do snapshot (evita struct autorreferente). Em `crates/core/src/scene_export/snapshot.rs` (R1):

```rust
// novo, ao lado de `plan`
pub fn plan_with(&self, timeline: &PlanTimeline<'_>, index: u64) -> Result<ScenePlan, CommandError> {
    let frame = self.schedule.edit_frame(self.scene.bundle(), index)?;
    Ok(timeline.build(frame)?)
}
// no laço do job (render/src/scene_export ou onde hoje chama `snapshot.plan(i)`):
let timeline = PlanTimeline::new(snapshot.scene());
for index in 0..snapshot.schedule().frames {
    let plan = snapshot.plan_with(&timeline, index)?;
    ...
}
```

`collect_required_assets` pode usar o mesmo `PlanTimeline` no laço por shot (ganho pequeno: um frame por shot). `scene.plan` (um frame por chamada, `scene_project/interaction.rs`) **não** deve adotar: uma pergunta por cena não amortiza o preparo e o monitor a cada gesto cria cena nova.
Expectativa honesta: no export, plano + JSON ~1,1–1,2× mais rápido em cenas com nós (o JSON domina); só cenas de câmera pesada chegam a ~2,7×. Se o export não serializa o plano em JSON por frame (consome `ScenePlan` direto no render), o ganho é o do plano: 3–5×.

## Testes

Executados (repo compartilhado, wrapper serial, debug salvo indicação):

| Comando | Resultado |
|---|---|
| `test -p tessel-editor-core --test scene_plan_performance` | 10/10 |
| `... --test scene_plan_performance_golden` (debug) e `test --release ... scene_plan_performance_golden` | 1/1 e 1/1 |
| `... --lib scene_plan` (unit test de `timeline.rs`) | 1/1 |
| `... --test scene_plan` | 29/29 |
| `... --test scene_plan_resources` | 1/1 |
| `... --test scene_flow` | 8/8 |
| `... --test directed_scene_temporal` / `directed_scene_animation` | 20/20 / 29/29 |
| `... --test directed_scene_oracle` / `directed_scene_oracle_extended` | 3/3 / 15/15 |
| `... --test scene_export` (R1, só para provar que não conflita) | 13/13 |
| `... --doc scene_plan` | 1/1 |
| `fmt -p tessel-editor-core -- --check`, `clippy -p tessel-editor-core --all-targets -- -D warnings` | limpos |
| `python3 scripts/scene-plan-performance/run.spec.py` | 4/4 |
| Benchmark oficial (release, números acima) e rechecagem de `medium`/`small` com 5 repetições | executado |

Não executados:

- `cargo test --workspace`, `pnpm`/runtime e gates globais (do coordenador); crates `desktop`, `media`, `render` (alheios; não consomem `PlanTimeline` ainda).
- Nenhuma execução GPU/nativa (nada de R6 usa GPU, sem lock GPU).
- Nenhum consumo real de `PlanTimeline` por export/monitor: só o patch acima, sem aplicar. **O ganho no export real não foi medido** (depende de R1).
- Benchmark em máquina sem ruído: as medidas foram feitas num Mac com outras frentes compilando; por isso faixas e ±10% de ruído estão declarados.

## Limitações

- A main pristina já difere em 1 ulp (pior caso 8,9e-16 absoluto) entre `debug` e `release` em alguns frames; por isso o golden fixa um digest por perfil. A **tolerância não foi afrouxada**: as comparações plano-timeline vs. plano-direto são exatas (valor e JSON canônico) no mesmo perfil.
- `PlanTimeline` não acelera a pergunta única nem a primeira de cada shot (custo do preparo ≈ um `build`).
- Cenas com `instances`/`clips` de id repetido: o primeiro vence, como na avaliação direta (teste de instância quebrada cobre a falha compartilhada).
- Memória medida por RSS grosso (`ps`), sem alocador contador (`unsafe_code = "forbid"` no workspace).
- Falha de compilação alheia vista no início (E0521 em `scene_interaction/gesture.rs`, R3 WIP) não foi tocada; desapareceu no checkout compartilhado atual.

## Patches de backlog / matriz (coordenador)

1. **BL064 → "A validar"**. Evidência: este relatório, F037, `scene_plan_performance*`. Remover "Em andamento"; criterio "mesma saída e budget explícito no export curto" cumprido para a saída; o budget do export curto depende da adoção em R1.
2. **Novo BL (sugerido)** — *escritor JSON direto do plano, byte a byte igual a `to_json_string`*: `to_json_string` custa 4–5× o `build` (80% do laço de exportação medido) e passa pelo `Value`; mesma saída, sem mudar `PLAN_VERSION`. Dono: `scene_plan/json.rs`; teste: comparar texto com o escritor atual no corpus e o golden. Dependências: BL064.
3. **Novo BL (sugerido)** — *tamanho do plano na exportação*: fragmentos estáticos (matrizes `local`, recursos, `requires`) enviados uma vez por shot ou matrizes derivadas no runtime; exige decisão de `PLAN_VERSION` e do runtime. Só vale se o JSON por frame continuar no caminho do export.
4. **Matriz F037**: adicionar a linha "linha do tempo reutilizável: V na lib (equivalência no corpus, 4 ordens × 2 modos, geração, threads, memória); consumidor real: pendente de R1".
5. **Adoção**: patch acima em R1/`scene_export` após integrar R6 (o plano de dez frentes já prevê "R1 pode consumir PlanTimeline somente após integração").
