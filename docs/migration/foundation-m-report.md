# Relatório da frente M — renderer da cena dirigida (BL007 / BL063)

2026-11-05. Checkout compartilhado `tessel-editor`, branch `feat/directed-scene-end-to-end`. A origem
(`tesselapp`, `three-scene-runtime.ts`) foi só **lida** para as convenções de câmera/material; nada foi
editado nela. Sem `git add`/commit/push/PR/worktree, sem subagentes. Contrato da feature:
[`docs/features/scene-renderer.md`](../features/scene-renderer.md).

Não toquei `crates/core/*`, `crates/media/*`, `crates/desktop/*`, `docs/backlog.md`, matriz nem índices.
O `scene.plan` do core é entregue como **patch** (abaixo, não aplicado).

## Arquivos

| Arquivo | Conteúdo |
|---|---|
| `runtime/src/directed-scene.js` (novo) | Consumidor do plano: `parseDirectedLayer`, `resolveSceneResources`, `buildDirectedScene`, `drawDirectedScene`, `DIRECTED_SCENE_CAPABILITIES`, `parseSceneIdentity` |
| `runtime/src/compositor.js` | `case 'directed_scene'` em `prepareLayer`; `prepareDirectedScene`, `sceneMaterial` (retido no `LruCache`) |
| `runtime/src/assets.js` | Identidade de recurso por slot: `admitScene`, `sceneRefusal`, `advanceSceneGeneration`, contagem e bytes |
| `runtime/src/retention.js` | Limites `sceneResourceEntries: 64`, `sceneResourceBytes: 768 MiB` |
| `runtime/src/index.js` | `features += directed_scene`, `ready.capabilities.directed_scene`, contadores em `cacheReport`/`debug.retention` |
| `runtime/test/directed-scene-fixtures.mjs`, `directed-scene.test.mjs`, `directed-scene-browser.test.mjs` | 8 testes puros + 10 em Chrome |
| `crates/render/src/scene.rs` (novo) | `SceneResourceMap`, `ResolvedResource`, `SceneSlots`, `Prepared`, `import_gated`, `plan_error`, `directed_layer`, `frame_render_message` |
| `crates/render/src/scene_renderer.rs` (novo) | `SceneRenderer<T: Transport>`, `FrameOptions`, `SubmittedFrame` |
| `crates/render/src/bin/tessel-scene-render.rs` (novo) | CLI headless |
| `crates/render/src/lib.rs` | exports (`scene`, `SceneRenderer`…, `HANDSHAKE_TIMEOUT`) |
| `crates/render/tests/scene_seam.rs`, `scene_frames.rs`, `scene_support/mod.rs` | 22 testes sem navegador, 7 em Chrome real, helpers do corpus |
| `docs/features/scene-renderer.md`, este relatório | docs |

`runtime/dist/runtime.js` foi reconstruído (`pnpm --dir runtime run build`; `dist` é ignorado pelo git e
compartilhado, então os outros agentes passam a ver o runtime novo).

`runtime/worker/*` **não** foi alterado (fora do meu ownership): o plano atravessa o worker reaproveitando
`asset.load` (identidade em campo aditivo) e `frame.render` (layer aditivo), o que mantém a lista de arquivos
permitidos e a saída PNG do worker.

## APIs e seams

- **Para N**: `SceneResourceMap::from_json` / `new` + `insert` aceita um mapa explícito
  `assetId → { revisionId, contentHash, path, geração }` sem esperar adaptador. Quando N materializar a raiz
  de transporte, `SceneResourceMap::from_root(generation, &plan, root)` já resolve sob ela.
- **Para O / coordenador (monitor)**: `SceneSlots::prepare` + `frame_render_message` (passos no doc da feature).
- **Para BL008 (export dirigido)**: `SceneRenderer::render` é o laço de um frame; falta o laço de export e a
  decisão de alpha de saída. Não alegado.

## Descoberta que só um teste real mostraria

Os testes JS iniciais usavam câmera como array (`[x,y,z]`) e passavam. O `ScenePlan::to_json` real escreve
`{x,y,z}`; o primeiro teste Rust em Chrome falhou com `invalid_plan: /camera/position`. Corrigi o parser e
as fixtures para a forma real do core. Lição: fixture escrita à mão não substitui o plano do core.

## Testes: escritos × executados

Todos via Cargo serial (`cargo-serial.py`) e lock `fcntl` em `/tmp/tessel-scene-gpu.lock`.

| Comando | Resultado |
|---|---|
| `node --test test/directed-scene.test.mjs` (de `runtime/`) | 8/8 |
| `TESSEL_REQUIRE_RUNTIME=1 … node --test test/directed-scene-browser.test.mjs` (lock) | 10/10 (Chrome, SwiftShader), antes da correção de câmera; reexecutados dentro da suíte abaixo |
| `… node --test --test-concurrency=1 test/*.test.mjs` (lock) | **79/79** |
| `… node --test test/*.test.mjs` (lock, concorrência padrão) | 78/79: `splat.test.mjs` falhou **uma vez**; sozinho passa; causa não investigada |
| `cargo clippy -p tessel-editor-render --all-targets --locked -- -D warnings` | limpo |
| `cargo test -p tessel-editor-render --locked -- --test-threads=1` (lock, `TESSEL_REQUIRE_RUNTIME=1`) | lib 4, `export` 2, `frames` 7, `scene_flow_resources` 3 (+1 ignorado de P), **`scene_frames` 7**, **`scene_seam` 22**, `video` 5: todos passam |
| `rustfmt --edition 2024` nos arquivos próprios | aplicado |

Evidência por superfície:

- **Runtime (Chrome real)**: orientação e bordas (fov vertical); alpha straight/opaque; ordem do plano vence
  profundidade; retenção (frame repetido: `texture_created` 0; mesmo hash com retenção desligada); convivência
  com camada comum; recusas tipadas; identidade (não resolvido/divergente/obsoleto/tamanho/liberado); varredura
  de geração; seek fora de ordem (mesmo hash, runtime frio igual); rajada (2 cancelados) e cancelamento
  explícito; perda de contexto no meio do frame e entre frames (mesmo hash após restaurar); orçamentos.
- **Lib Rust**: identidade/slots (22 testes), incluindo recusa que não altera o livro, geração, revisão, LRU,
  bytes, ids de slot limitados, e `SceneRenderer` contra um runtime roteirizado: ordem das mensagens, picture
  recusada e recarregada, reenvio em `context_lost` (até 2×), `cancelled` nunca reenviado, runtime que morre,
  timeout, runtime sem a feature ou sem capacidade.
- **Headless (Chrome real)**: `scene_frames` contra o **oráculo independente** do corpus de P
  (`scripts/directed-scene-proof/corpus`, 320×180, 12 sondas por frame): 12 visitas em ordem que pula e repete
  (95, 0, 48, 12, 47, 36, 11, 60, 20, 12, 31, 0), 10 imagens distintas, **diferença máxima de canal 0**
  (tolerância 6), PNG escrito com o tamanho certo; runtime frio = aquecido; troca de recurso (hero rev-1 →
  rev-2, geração 2) bate as sondas de `swapFrames` e muda o hash; geração antiga é `resource_stale`; volta ao
  primeiro projeto (geração 3) reproduz o hash inicial; rajada mostra o frame mais novo igual ao isolado; CLI.
- **Socket / UI**: **não executados** (nenhuma superfície ligada).

### Não executado / não provado

Desktop WebView com `directed_scene`; UI e monitor; socket; export dirigido (BL008); pixels contra o runtime
da origem (BL018/BL071); GPU real (só SwiftShader); medição de custo/tamanho (BL064); troca de recurso no meio
do frame sem teste determinístico; perda de contexto provocada pelo lado Rust (não há mensagem de bridge para
isso); verificação do hash do conteúdo do arquivo (é do adaptador).

## Patch para o coordenador (não aplicado, **não compilado**)

Alinha com o teste bloqueado `scene_plan_is_available_as_an_operation_and_matches_the_model`
(`crates/core/tests/scene_flow.rs`), que espera `answer["plan"]["nodes"][].worldMatrix`.

`crates/core/src/ops.rs`, ao lado de `"scene.evaluate"`:

```rust
"scene.plan" => SceneCommand::Plan {
    scene: scene()?,
    at: frame_at(request)?,
    // WHY opt-in: a partial plan is a diagnostic; a renderer must only be fed complete plans.
    partial: optional_bool(request, "partial")?.unwrap_or(false),
},
```

`crates/core/src/scene_project/command.rs`, na enum `SceneCommand`:

```rust
/// BL063: the evaluated plan of one frame (`ScenePlan`), the contract between the core and a runtime.
Plan { scene: String, at: FrameAt, partial: bool },
```

`crates/core/src/scene_project/run.rs`, no `match` de `apply_scene`:

```rust
SceneCommand::Plan { scene, at, partial } => self.scene_plan(&scene, &at, partial),
```

`crates/core/src/scene_project/interaction.rs`, ao lado de `scene_evaluate` (imports:
`crate::scene_plan::{PlanOptions, ScenePlan}`):

```rust
pub(super) fn scene_plan(
    &self,
    scene: &str,
    at: &FrameAt,
    partial: bool,
) -> Result<Value, CommandError> {
    let entry = self.attached_scene(scene)?;
    let frame = match at {
        FrameAt::Frame(frame) => *frame,
        FrameAt::Time(time) => entry.scene.bundle().frame_at_time(*time)?,
    };
    // `PlanError -> CommandError` keeps the code and `details` (unsupported domains, capabilities).
    let plan = ScenePlan::build_with(&entry.scene, frame, PlanOptions { allow_partial: partial })?;
    Ok(json!({
        "action": "scene.plan", "scene": scene, "complete": plan.is_complete(),
        "plan": plan.to_json(),
    }))
}
```

`crates/core/src/scene_project/query.rs`, no braço que recusa comandos de leitura que não serve
(`SceneCommand::Evaluate { .. } | …`): acrescentar `| SceneCommand::Plan { .. }`. Procurar também
`scene.evaluate` na tabela de operações em `docs/features/scene-interaction.md` e no catálogo de capacidades.

Depois do patch, remover o `#[ignore]` de `scene_plan_is_available_as_an_operation…`, e o desktop/CLI podem
ligar `scene.plan` → `SceneResourceMap` → `SceneRenderer::render`.

**Nota sobre `Content::DirectedScene`.** A rota tipada `RenderPlan` (`plan.rs`) precisará de um conteúdo que
carregue o layer quando o export dirigido (BL008) e `session` passarem a pedir cenas pelo mesmo caminho das
imagens. Não escrevi esse patch: depende de todos os `match` sobre `Content` e de decidir se
`PLAN_LAYER_KINDS` passa a incluir `directed_scene` (o runtime hoje **não** o lista em
`capabilities.layers`; o gate é `features`). Fica como decisão de integração.

## Sugestões de linhas para backlog / matriz

`docs/backlog.md` (o coordenador reconcilia; não criei outra fila):

- **BL007** → `Parcial`. Evidência: runtime 79/79 (serializado), `scene_seam` 22, `scene_frames` 7 em Chrome real com oráculo independente (diferença máxima 0). Falta: monitor desktop (O), export (BL008), GPU real, medição.
- **BL063** → `Parcial`. Entregue: consumo do plano pelo runtime, `SceneResourceMap`/`SceneSlots`, CLI `tessel-scene-render`. Falta: operação `scene.plan` (patch acima), ligação socket/UI.
- **BL064** → `Pendente` (sem mudança): sem cache de resultado de frame dirigido; medir custo/tamanho antes.
- **Novo (pendência da M)**: alpha de saída do frame dirigido (hoje fundo preto opaco) para BL008.
- **Novo**: mipmaps/qualidade de minificação das texturas versus origem (BL018/BL071).
- **Novo**: investigar a falha única de `splat.test.mjs` com `node --test` em concorrência padrão.
- **Novo**: verificação de hash de conteúdo do arquivo na materialização (N/`media`), pois o renderer confia no mapa.
- **Novo**: prova do `directed_scene` no WebView desktop (`host.rs`) e no monitor O.

Matriz de features (nova linha sugerida, ID a atribuir): `Renderer de cena dirigida` — UI **P**, CLI da
janela **P**, CLI headless **V** (`tessel-scene-render`), Lib **V**, entrada compartilhada: `SceneSlots::prepare` +
`frame_render_message` (lib); `scene.plan` pendente.
