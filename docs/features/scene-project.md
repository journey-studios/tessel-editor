# F036 — Cenas dirigidas no projeto: documento, histórico e acesso programático

## Status

| Superfície | Estado | Evidência |
|---|---|---|
| Lib (`tessel_editor_core::scene_project`, `EditorState::scenes`, `Command::Scene`) | **V** — validada pelo coordenador via Cargo | `crates/core/tests/scene_project.rs` (19 testes) e 11 testes unitários em `scene_project/{mod,scope,store}.rs` |
| CLI headless (`tessel-headless`, JSON Lines) | **V** — mesmo parser/executor, sessão JSONL e executável headless testados | `a_json_lines_session_runs_the_whole_flow_in_process`, `the_headless_executable_answers_exactly_like_the_library` |
| Socket / `ui op` da janela | **P** — roteia por `Op::Run(Command::Scene)` sem tocar o desktop; **não provado** (exige a janela). Um socket não prova a CLI headless, nem o contrário | — |
| UI visual (árvore, inspetor, gesto) | **P** — sem controle, sem nó em `tree.rs`, sem apresentação | — |
| Package / import de projeto | **Guardas** em `crates/media` (ver abaixo); 4 testes executados, incluindo pacote parcial em pasta e ZIP | `crates/media/tests/scene_project_guards.rs` |

Nada aqui habilita o bundle legado como runtime: uma cena anexada é **dado de interchange** guardado
no projeto. Não é um asset `tessel-scene@1` (F030), não vira `MediaKind::Scene`, não é renderizada,
reproduzida nem exportada.

## Objetivo e limites

Dar a quem automatiza (agentes, scripts, a CLI) e à futura UI uma porta única para: criar um projeto,
**anexar um `render-bundle` 1.0 por ID**, listar e consultar catálogo, propriedades e câmera,
**editar em lote com dry-run**, **desfazer/refazer** e **salvar/reabrir** sem perder dados. Esta feature é o
adaptador entre três peças que já existem e **não acrescenta regras a nenhuma delas**:

| Peça | Dona de |
|---|---|
| [`directed_scene`](directed-scene.md) (F031/F033) | validar e avaliar o bundle; `evaluate_camera`, `camera_at_clip_frame`, `frame_at_time` |
| [`scene_edit`](scene-edit.md) (F034) | catálogo, IDs, lote atômico, importador + gates estrutural/temporal sobre cada candidato |
| `EditorState` | histórico por snapshot, revisão, documento |

Fora, de propósito: render/preview/export/playback de cena dirigida; recursos que o bundle nomeia por
caminho relativo (imagens, mix de áudio) — **não são resolvidos, copiados nem empacotados** até BB05/BB06;
avaliação de animação de nós por esta feature (a câmera é consultável; o estado dos nós não); UI;
paginação de `scene.properties`.

## Contrato do core / biblioteca

```rust
use tessel_editor_core::scene_project::{SceneCommand, SceneSource};
use tessel_editor_core::{Command, EditorState};
use tessel_editor_core::state::ProjectSettings;

let mut state = EditorState::empty(&ProjectSettings::default())?;
state.execute(Command::Scene(SceneCommand::Attach {
    id: Some("street".into()),          // None: o core gera `scene-<n>`
    name: None,                         // None: usa o `bundleId`
    source: SceneSource::Path("render-bundle.json".into()),
}))?;                                   // 1 entrada de histórico: "Attach scene"
state.undo()?;                          // a cena some; `redo()` a devolve
```

`state.scenes: SceneBook` guarda `Vec<AttachedScene { id, name, scene: Arc<DirectedScene>, .. }>` em
ordem de anexação. Cada `scene.*` é um `SceneCommand` dentro de `Command::Scene`; tudo passa por
`EditorState::execute`, a mesma porta das demais edições.

### Comandos (`op` JSON ⇄ `SceneCommand`)

| `op` | Campos | Efeito | Histórico |
|---|---|---|---|
| `scene.attach` | `id?`, `name?`, **um de** `bundle` (objeto) · `text` (string JSON) · `path` (arquivo) | importa, roda os gates estrutural e temporal, anexa | "Attach scene" |
| `scene.detach` | `scene` | remove | "Remove scene" |
| `scene.edit` | `scene`, `ops[]` (`{"set","value"}`/`{"reset"}` da F034), `dry_run?` | lote atômico via `scene_edit::apply`/`dry_run` | "Edit scene" (uma entrada por lote) |
| `scene.list` | — | `id`, `name`, `bundle_id`, `revision_id`, contagens | — |
| `scene.describe` | `scene` | fps, duração, definições/nós, instâncias, shots, clipes, notices, famílias adiadas em uso, chaves de entrada desconhecidas | — |
| `scene.catalog` | — | as 8 propriedades (`path`, `kind`, `unit`, `range`, `default`, `applies_to`, `origin`) | — |
| `scene.properties` | `scene`, `prefix?` | cada propriedade aplicável com o valor atual | — |
| `scene.get` | `scene`, `property` (caminho canônico) | uma propriedade | — |
| `scene.camera` | `scene`, **um de** `frame` · `time` (`{"num","den"}` exato) · `clip`+`clip_frame` | câmera via `evaluate_camera` / `camera_at_clip_frame` (mapeamento shot→instância→clipe incluso) | — |
| `scene.capabilities` | — | comandos, limites e o `scope` | — |

O parser (`ops::scene_command`) só traduz forma; nenhuma regra mora nele. As respostas de câmera usam
as chaves de `FrameEvaluation::to_json` (`editFrame`, `instanceFrame`, `cameraClipFrame`, `camera.{position,
target, fovDeg, velocity, fovVelocityDegPerFrame}`).

### Garantias

- **Rejeição, dry-run e no-op não mudam nada.** `scene.edit` só troca a cena depois de `apply` aceitar e
  de `summary.modified` ser verdadeiro. Dry-run devolve `changed:false`; no-op (valor igual, ou
  setar e restaurar no mesmo lote, ou lote vazio) devolve `changed:false, noop:true`. `EditorState::execute`
  só grava histórico e sobe `project.revision` quando `changed != false`.
- **Códigos preservados.** `unknown_node`, `unknown_path`, `invalid_bundle` etc. vêm da camada que achou
  o problema; `details.index`, `details.path` e `details.issues[]` (com `code` do importador, `path`
  JSON Pointer e `ops`) atravessam sem renomear. Um lote inválido tardio devolve o `index` do op culpado.
- **Uma pilha só.** Não há segunda pilha de undo: `Snapshot` ganhou `scenes` (cada bundle é `Arc`, o
  snapshot custa ponteiros). Vários comandos em `EditorState::transaction` viram uma entrada; uma
  transação que falha desfaz tudo.
- **IDs estáveis.** `scene_id` escolhido por quem chama (ou `scene-<n>`, o primeiro livre), com o alfabeto
  de `Id` (`A-Za-z0-9-_.`, ≤ 64 bytes), validado por `check_scene_id`. Não dependem de seleção, janela
  ou `bundleId`. São `String`s: **nunca** passam por `Id::new`, que internaliza para sempre.
- **Gates na entrada.** `attach` e a abertura do documento executam `admit`: importador (já feito pela
  importação), `edit_join_issues()` e `temporal_issues()`. Um bundle que importa mas quebra um gate
  (ex.: `MISSING_TRANSITION`, `CONTINUOUS_VELOCITY_JUMP`) não entra.
- **Limites.** 64 cenas, 128 MiB de JSON compacto em attach/open/edições. Save, autosave e
  pacote limitam o JSON realmente serializado a 256 MiB, antes da escrita, incluindo indentação.

- **Declaração de escopo.** `attach` e `scene.capabilities` devolvem `scope`: `validated`, `deferred_fields`
  e `deferred_rules` (as listas do próprio importador), `not_evaluated`, `not_enabled`, `transport`; e
  `attach`/`describe` devolvem `deferred_in_use` (quantas vezes o bundle usa luz, clima, rig, narração…
  — dados guardados verbatim que ninguém valida nem avalia).

## Documento `.tessel.json`

Bloco **aditivo** `scenes`, ausente quando o projeto não tem cenas:

```json
"scenes": {
  "version": 1,
  "items": [ { "id": "street", "name": "Street", "bundle": { "...render-bundle 1.0..." } } ]
}
```

- Projetos com bloco de cenas usam `schema_version: 2`; projetos sem dados de cena continuam em 1.
- Abrir: `from_document` valida cada item como um `attach` (ID, importador, gates). Qualquer item ruim
  falha a abertura inteira (`invalid_bundle` com as `issues`, `target = scene:<id>`; `unsupported_schema`
  para `scenes.version` maior que 1; `invalid_document` para forma errada). O projeto aberto não muda.
- O bundle é guardado como a árvore JSON importada: **campos desconhecidos do bundle, de cada item e do
  bloco voltam byte-iguais** (salvar de novo escreve os mesmos bytes).
- Leitores anteriores, limitados ao schema 1, recusam schema 2 e não podem apagar cenas
  silenciosamente. Projetos comuns continuam no schema 1; a compatibilidade de leitura
  dos documentos antigos é preservada.

## CLI headless

```bash
printf '%s\n' \
  '{"op":"scene.attach","id":"street","path":"docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"}' \
  '{"op":"scene.camera","scene":"street","frame":10}' \
  '{"op":"scene.edit","scene":"street","dry_run":true,"ops":[{"set":"camera/cam-main/start/fov","value":55}]}' \
  '{"op":"scene.edit","scene":"street","ops":[{"set":"camera/cam-main/start/fov","value":55}]}' \
  '{"op":"history.undo"}' \
  | ./scripts/cargo run -q -p tessel-editor-core --bin tessel-headless -- --empty
```

Cada linha de saída é o envelope `{ok, version, result|error}` de sempre. Um erro tem `error.code`,
`error.target` e `error.details`. O processo sai com 1 se alguma linha falhou. O executável e a
biblioteca respondem **linha a linha igual** (teste de subprocesso).

## UI e CLI da janela

`./scripts/ui op '{"op":"scene.list"}'` chega a `Op::Run(Command::Scene(..))`, que o desktop já executa
por `EditorState::execute` — nenhuma variante nova de `Op`, nenhum arquivo do desktop alterado. **Não foi
exercitado** (sem janela nesta frente). Apresentação visual: **P**; não há controle nem gesto, então não há
equivalente `ui click`. Quando existir, deve chamar os mesmos comandos.

## Package e importação de projeto (guardas em `crates/media`)

Só compatibilidade; codificação/importação/render intocados.

| Operação | Comportamento |
|---|---|
| `package.preflight` / `package.create` | As cenas viajam no documento do pacote. Se algum bundle nomeia arquivos (`assets[].relativePath`, mix de áudio), cada cena entra em `missing` como `{asset:"scene:<id>", paths, state:"not_packaged"}`: o pacote é **incompleto** (`complete:false`) e, sem `allow_partial`, bloqueia com `unsupported` (preflight: blocker `unsupported`; create: `JobError unsupported`). Com `allow_partial` o pacote preserva o documento e as cenas e diz que está incompleto. Todo bundle válido exige ao menos um asset com caminho; portanto, atualmente todo projeto com cena anexada tem pacote incompleto. O teste de pacote completo cobre projeto sem cenas. |
| `project.import` (sequências de outro projeto) | Cenas pertencem ao projeto, não à sequência: nunca são importadas, e o plano ganha um aviso (`directed scene(s) … not imported`). A origem só é lida. |
| `project.restore` | Restaura o documento com as cenas; `complete:false` quando o manifesto diz que faltam recursos. |

## Evidências e status

Executado pela frente G (sem Cargo, por determinação): oráculo `node scripts/migration-oracle/oracle.mjs
verify` (hashes fixados) e `rustfmt --check` (apenas sintaxe/formatação). **Nenhum teste Cargo foi
executado.** Para o coordenador:

```bash
./scripts/cargo test -p tessel-editor-core --lib scene_project
./scripts/cargo test -p tessel-editor-core --test scene_project --test headless --test project
./scripts/cargo test -p tessel-editor-media --test scene_project_guards --test package --test interchange
./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings
```

A comparação com a origem existe só para a **câmera** (corpus do oráculo, tolerância `1e-9 +
1e-12·|x|`, igual à F033); o restante é comportamento desta feature, não equivalência.
Limitações: sem paginação em `scene.properties`; sem render; recursos do bundle não resolvidos;
socket e janela ainda não exercitados; teto de bytes aplicado também em edições e serialização.

## Revisão do coordenador

O coordenador executou os testes oficiais, introduziu proteção de versão 2 somente para
projetos com cenas e teto de serialização real em save/autosave/pacote. O limite agregado
é validado também em edição/dry-run; falha não troca a cena. O aviso de intercâmbio
foi movido para o plano no core, compartilhado por biblioteca e adapter de mídia.
UI visual e render dirigido continuam pendentes; recursos não são empacotados como completos.

As guardas de mídia passaram 4/4: pacote completo é recusado enquanto faltam recursos
de cena, pacote parcial preserva o documento e declara a lacuna, e importação de
sequências declara que não leva cenas. A revisão corrigiu a criação da pasta de
conteúdo de pacote parcial quando não há arquivos de mídia para copiar.

Validação oficial final do coordenador: workspace com **522 testes passando,
zero falhas e 2 ignorados** (ZIP64 de 4,5 GiB); clippy com warnings negados e
formatação passaram. Os 19 testes públicos de cena incluem executável headless real;
as 4 guardas de mídia também passaram. UI visual/socket continuam sem prova específica.

O tamanho compacto de cada bundle é contado ao inserir/substituir e mantido em cache no `SceneBook`; consultas de orçamento não reserializam cenas inalteradas. Clone, remoção, substituição e rejeição preservam os contadores (testes de `scene_project::edit_budget_tests`).
