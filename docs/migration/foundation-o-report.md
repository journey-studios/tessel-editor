# Relatório da frente O — árvore de cena, seleção e inspector GPUI (BL014 / BL021)

2026-11-05. Checkout compartilhado `tessel-editor`, branch `feat/directed-scene-end-to-end`. A origem
`tesselapp` não foi tocada. Sem commit, push, PR, branch ou worktree; sem subagente. Contrato e
evidência por superfície: [`docs/features/scene-inspector.md`](../features/scene-inspector.md).

**Não editado:** `core`, `media`, `render`, `runtime`, `Cargo.toml/lock`, `docs/backlog.md`, matriz,
índices, `docs/ui-automation.md`. **Nenhum patch é obrigatório para compilar**: tudo o que a frente usa
já era público no core.

## Resultado

Aba **Cena** no inspector (ao lado de **Clipe**): seletor de cena anexada, árvore de entidades (Kit
`Tree`), seleção por ID, e as oito propriedades tipadas do catálogo com slider por componente, valor
vivo/candidato, reset quando há padrão, erro tipado na tela e controles desabilitados com o motivo.
Soltar o slider = um undo; Escape cancela; teclado equivale ao ponteiro. A mesma operação pelo socket
usa o mesmo executor e há nós semânticos para tudo. A cena anexada é selecionável e editável **sem
renderer**; o monitor aparece como indisponível, com um ponto de integração explícito e nenhum mock.

## Arquivos

Todos sob `crates/desktop/src/` (propriedade desta frente) e os dois documentos.

| Arquivo | Conteúdo |
|---|---|
| `workstation/scene_model.rs` (novo) | modelo de leitura sem GPUI: `TreeEntry`, `PropertyView`, `Snapshot`, `SnapshotCache`, `scrub`, componentes/Euler; 13 testes |
| `workstation/scene_gesture.rs` (novo) | adaptador slider/teclado → `scene.gesture.begin/update/commit/cancel` (`SliderKey`, `SceneSliderGesture`) |
| `workstation/scene_ui.rs` (novo) | `SceneUi` (estado de janela), seleção/reset/escolha de cena, sincronia do Kit `Tree` e do seletor, nós semânticos, `scene_invoke`, `scene_set` |
| `workstation/scene_panel.rs` (novo) | `render_inspector` (barra Clipe\|Cena) e a aba Cena |
| `workstation/scene_monitor.rs` (novo) | seam `SceneMonitor::Unavailable`, nó `monitor.scene`, patch futuro documentado no cabeçalho |
| `workstation/scene_tests.rs` (novo) | 16 testes GPUI com entrada injetada |
| `workstation/mod.rs`, `inspector_panel.rs`, `monitor.rs`, `viewport.rs`, `automation/server.rs` | fiação mínima: módulos, campo `scene_ui`, Escape e perda de foco, nós na árvore, selo do monitor, reset ao trocar de projeto, ganchos `set`/`invoke` |

`inspector_panel.rs`: o corpo antigo virou `render_clip_inspector` e a sonda `region:inspector` subiu para
o novo `render_inspector`; o modo padrão continua **Clipe**, então a UI de clipe não mudou.

## APIs e seams

- `SnapshotCache::get(&EditorState, scene_id) -> Option<Rc<Snapshot>>`: leitura por biblioteca (mesmas funções
  de `scene.tree`/`scene.catalog`/`scene.get`), teste de igualdade com a resposta do comando.
- `Workstation::scene_set_component(path, index, value, source, cx)`: begin → update → commit; é o `set`
  semântico e a prova de que socket e slider compartilham o executor.
- `Workstation::scene_monitor() -> SceneMonitor`: **o ponto de integração do renderer** (frente M). Hoje só
  `Unavailable { reason }`.
- `Workstation::scene_invoke` / `scene_set` (públicos, chamados por `automation/server.rs`).
- Nós novos: `inspector.mode`, `scene.panel`, `scene.picker`, `scene.tree`, `scene.entity:<path>`,
  `scene.property:<path>`, `monitor.scene`.

## Comandos usados (nenhum setter ou regra duplicada)

`scene.select` (replace), `scene.gesture.begin|update|commit|cancel`, `scene.edit` com `EditOp::reset`.
Leituras: `tree_json`, `catalog`, `read_property`, `Gesture::preview_scene` (as mesmas funções dos
comandos de leitura). Nenhum comando novo no core.

## Evidência

| Item | Resultado |
|---|---|
| `cargo test -p tessel-editor --locked` (pacote inteiro) | **137 passaram, 0 falharam** (108 pré-existentes + 13 `scene_model` + 16 `scene_tests`) |
| `cargo clippy -p tessel-editor --locked --all-targets -- -D warnings` | limpo |
| `cargo fmt -p tessel-editor` | aplicado |
| Socket em instância própria | passou (roteiro e saídas em [scene-inspector.md](../features/scene-inspector.md)) |

## Não executado / limites

- **Sem captura de tela**: aparência e layout (barra de modos, altura da árvore, linhas de slider) não foram
  vistos; só estado, estrutura e entrada injetada.
- **Ponteiro físico não provado** (BL043); os testes injetam eventos no `VisualTestContext`.
- Não rodei `cargo test` do workspace inteiro nem `core`/`media`: só `tessel-editor` (desktop). `pnpm`/`graphify`
  não se aplicam a este repositório.
- Trocar de modo é estado da janela, não `Command` (ver proposta 2).
- Sem diálogo "Anexar cena" na janela (ver proposta 3).
- Overlays/handles (BL015) **não** migrados.

## Propostas ao coordenador (core/apresentação — não aplicadas)

1. **Comando de apresentação `monitor.show_scene { scene }`** (quando houver renderer): em
   `crates/core/src/state/` um `Command::ShowSceneInMonitor { scene }` que só registra `preview` =
   cena; `Preview`/`PreviewIntent` ganha a variante correspondente. O botão "Ver no monitor" de
   `scene_panel.rs` passa a enviá-lo e `scene_monitor.rs` ganha o braço `Available`.
2. **`Command::SetInspectorMode { mode: clip|scene }`** em `state/inspector.rs` ao lado de `SetInspectorTab`, para
   que a troca de modo seja comando, entre no `ui tree` do core e persista com o layout. Hoje é
   estado local (`SceneUi.mode`), testado pelo nó `inspector.mode`.
3. **Diálogo "Anexar cena"** reutilizando `UploadSheet`-equivalente do workflow (`workflow.rs`) para
   `scene.attach` por arquivo; hoje só pelo socket/headless.

## Linhas sugeridas

`docs/backlog.md`:

- BL014: `Concluído (UI + socket)` — "Inspector da aba Cena: oito propriedades tipadas, vivo/candidato, reset, erro tipado e indisponibilidade com motivo; 29 testes (13 lib + 16 GPUI) e socket em instância própria; sem pixels nem ponteiro físico — [relatório](foundation-o-report.md)". Evidência: `scene-inspector.md`.
- BL021: `Socket: concluído · UI visual: A validar` — "Socket em instância própria prova `scene.select`/`gesture`/`edit` e os nós semânticos; resta captura/inspeção visual da aba Cena e ponteiro físico (BL043)".
- BL015: sem mudança; adicionar dependência "BL014 concluído; falta renderer (M)".
- Novos (IDs do coordenador): "Diálogo Anexar cena na janela" (proposta 3); "`monitor.show_scene`" (proposta 1);
  "Modo do inspector como Command" (proposta 2); "Revisar visualmente a aba Cena (layout, 1440×874)".
- BL074/BL075: acrescentar "vale também para sliders da aba Cena".
- BL066 (colisão de IDs de gesto entre projetos): `scene_project_changed` já zera o adaptador na troca de projeto,
  mas o ID de gesto segue do core; sem mudança.

`docs/migration/feature-migration-matrix.md`: linha da UI de F039 — "V (UI, input injetado) + V (socket)";
ponteiro físico e visual: P.

`docs/ui-automation.md` (nota): listar os nós `inspector.mode`, `scene.*`, `monitor.scene` e que
`set scene.property:<path> <campo> <n>` é um gesto completo (uma entrada de undo).
