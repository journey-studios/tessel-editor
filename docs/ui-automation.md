# UI automation

Control the running editor through a local CLI, without mouse coordinates. Start the
app (`./scripts/cargo run -p tessel-editor --locked`), then from the repository:

```sh
./scripts/ui status                       # window, theme, live editor summary
./scripts/ui tree                         # every live component, value, option, limit
./scripts/ui get property:zoom            # one node: state, fields, actions
./scripts/ui actions                      # operations currently available
./scripts/ui set <id> <field> <value…>
./scripts/ui move clip:<id> [--track T] [--frame N] [--snap true|false]
./scripts/ui move track:<id> (--before T | --after T)
./scripts/ui invoke <id> [action]
./scripts/ui theme dark|light
./scripts/ui invoke window.minimize       # also activate, maximize, fullscreen, close
```

`./scripts/ui` uses Cargo to keep the binary current; `target/debug/tessel-editor ui …`
is faster once built. Commands connect to the running app; they never start another one.

## Project operations and the viewport

`./scripts/ui op '<json|@file>'` sends one project operation (`project.create`, `asset.add`,
`track.add`, `clip.add`, `seek`, `plan`, `render.frame`, `render.audio`, …: see
[F015](features/project-creation.md)) to the live state. `render.frame` is answered only after
the monitor's WebView has drawn the frame (client timeout 45 s; 5 s for other commands).
`monitor.preview.sequence`, `monitor.preview.source` and `monitor.reload` are normal
`invoke` targets; `ui get monitor.viewport` reports phase, headline, `on_screen`, bridge
capabilities, session statistics (`coalesced`, `pending`, `presented`: the sequence frame on screen) and the last frame
(`render_ms`, size; `hash`/`mean` only for explicit frames or an editor started with `TESSEL_VIEWPORT_INSPECT=1`).
`scripts/seek-scrub-proof.py` drives a playhead drag over this socket and checks that the monitor keeps updating.
`scripts/video-live-proof.py` starts the release binary with an isolated `HOME` and checks, with semantic requests only,
that container video and PNG/JPEG sequences show the frame asked for (barcoded fixtures) and measures `seek` → displayed
latency ([viewport-bugfix-report](viewport-bugfix-report.md)). A scrub with no pressed button never moves the playhead:
GPUI tests inject the input (`input_tests.rs`), no OS pointer.
Started with `TESSEL_VIEWPORT_TRACE=1`, `get monitor.viewport` also carries `session.trace` (spans of the host side of a
frame, the last 8 frames; `op render.status` carries the last 128). The proof reads the editor's own
timestamps from it instead of polling during the drag. `TESSEL_VIEWPORT_RETAIN=0` starts the editor on the runtime's
per-frame create/dispose path (A/B and rollback). Neither adds a control: both are read-only opt-ins in the environment.
With the same trace, `ui op '{"op":"render.trace","since":0,"limit":4096}'` returns the correlation events (tick, prefetch, protocol
requests, asset reads, page spans) after a cursor, with `lost` when the reader fell behind the 16 384-event window;
`TESSEL_VIEWPORT_ASSET_IO=sync|async` (default `sync`) picks who reads project files for the `tessel://` protocol and
`render.status` reports `asset_io`. `./scripts/headless` has no window, so it answers `desktop_required` ([F058](features/viewport-asset-io.md)).

`scripts/proxy-live-proof.py` (OPT01/OPT04) starts the release binary with an isolated `HOME` per variant and checks, with
semantic requests only: the media job builds the preview proxy (`op preview.policy` → `proxy_ready`), a scrub frame is drawn
from it (`monitor.viewport` → `last_frame.source.proxy_layers`), the refine replaces it with the original, an explicit
`render.frame` and an export use the original (export identical, frame by frame, to the `original` policy's), and with
`TESSEL_VIEWPORT_INSPECT=1` the picture on screen after the drag is byte-identical to an explicit render. It also runs
`seek-scrub-proof.py` per variant. `scripts/proxy-soak.py` repeats drags (and a policy flip) for `--seconds` and samples the
editor's resident memory. Read-only opt-ins in the environment: `TESSEL_VIEWPORT_RESULT_CACHE=0` (splat result cache off),
`TESSEL_VIEWPORT_RETAIN_MIB=<MiB>` (source-file budget of retained stills/scenes/splats before the least recently used heavy asset is released, default 256; BL151), `TESSEL_VIEWPORT_SCENE_SAMPLES=0|2|4` (final MSAA of scene targets), `TESSEL_VIEWPORT_PREVIEW_SCENE_SAMPLES=0|2|4` (MSAA of a splat
scene while the playhead moves; default 0, 4 turns the reduction off; BL129), `TESSEL_VIEWPORT_PREVIEW_SORT=latest|exact` (whether a splat
frame of a moving playhead is drawn with the order of the last finished sort, default `latest`, or waits for its own, `exact`, which only changes the sort, not the preview MSAA or proxies; an unreadable value is `exact` with a warning on
stderr; BL130; the scrub/playback harness takes it through `--env`, and `--inspect` also compares the last frame the runtime returned
after every scrub release (not the screen pixels) with an explicit `render.frame`; a release that never settles within the timeout is
recorded as `passed: false` and the harness exits 1). `./scripts/ui invoke monitor.preview.quality` toggles the policy
(proxy and lighter MSAA while playing/scrubbing, or the original always);
`op preview.policy` reads/sets it; `op media.preview_proxy` builds the proxy by hand; `op render.status` carries `ui`
(`repaints`/`skipped`, OPT04).

To run a second, isolated editor for a proof, start the built binary with `HOME=<scratch dir>` and point the proof
scripts at the same `HOME` (they resolve the socket under `~/.cache/tessel-editor/`). Do not use `./scripts/ui` or
`./scripts/cargo` with that `HOME`: the wrapper treats it as a first build on a new machine, reinstalls the toolchain
under the scratch `HOME` (several GiB) and repoints `target/` at that cache. Use `HOME=… ./target/debug/tessel-editor ui …`
for the CLI and delete a stale `ui.sock` of the scratch `HOME` after killing its editor.

macOS does not paint a window that is fully covered, and the monitor's on-screen bookkeeping
runs in `render`: run `./scripts/ui invoke window.activate` before observing or capturing
(`screencapture -l <window id>` is the native capture used for evidence). `scripts/live-proof.py`
drives the full flow and asserts pixels over this socket.

`scripts/scene-edit-proof.py` (S6, [F034](features/scene-edit.md)) starts the direct binary with its own short `HOME` and drives it
only with `ui op|get|set` of that binary (`scene.edit`, `scene.camera`, `history.undo`, `project.save`, the inspector's
`set scene.property:<path>`); it runs the same scenario against `tessel-headless` (JSONL, no window) and reports the two
surfaces separately. Run it under `scripts/directed-scene-proof/with-gpu-lock.py`; it refuses to start with fewer than 12 GiB
free or with tracked changes (unless `--allow-dirty`, never for final evidence).

## Flows and dialogs

No coordinates and no native picker are needed: `invoke flow.import` opens the dialog, `set dialog.field.paths
value '/a.mp4; /b.wav'` and `set dialog.field.mode value reference` fill it, `invoke dialog.verify` refreshes a
read-only report (packages, export preflight), `invoke dialog.submit` runs the same operation `ui op` would
(`media.import`, `project.save|open|recover`, `project.package`, `project.import`, `export.start`) and
`get jobs` follows it. The native file picker is only a convenience that fills the same field. Dialog errors
stay in the dialog and are returned as a structured error (`missing_argument` names the field's label).

While any dialog is open the monitor's WebView is hidden (`get monitor.viewport` reports `on_screen:false`) because a
native view would paint over it; it returns on close. Playback: `./scripts/ui op '{"op":"playback.start"}'`,
`playback.status`, `playback.pause`, `playback.resume`, `playback.seek` (`frame`), `playback.stop` (or `invoke monitor.play`);
`playback.status` informa o relógio em uso (`clock`: `audio_device` ou `wall`), o dispositivo e a latência estimada pelo host
([F018](features/playback.md)). Operações de mídia pesadas (`media.import|prepare|relink`, `project.verify|restore|import`,
`export.preflight`) respondem **na hora** com um job (`{job, state, kind}`); o projeto muda quando o editor aplica o resultado
(siga por `get jobs`, `job.get`/`job.wait`). Enquanto o decode automático de um projeto recém-aberto não termina,
`render.frame`/`render.audio` respondem `media_preparing` (tente de novo). O seletor de sequências é o nó `timeline.sequence`
(`set timeline.sequence value <id|nome>`).

`scripts/live-flow-proof.py` drives the whole plan flow (create, import, edit, undo/redo, save, package, restore,
interchange, export 1080p/vertical/square[/UHD] com o som do próprio vídeo medido espectralmente, cancel, SIGKILL + recover,
playback no relógio do dispositivo com pause/seek/resume, overlay/resize gates) with semantic requests only. A etapa do crash
mata **somente** o editor que o próprio script abriu (nunca um `pgrep` por nome) e o script aceita `HOME=<dir>` diferente para
isolar o socket de outra instância do editor. `scripts/headless-flow-proof.py` faz o mesmo fluxo de mídia sem janela;
`scripts/measure-render.py` mede preview/export e memória.

## Medidor de desempenho (F049)

`./scripts/ui get toolbar.perf_meter` lê o estado do medidor flutuante (`visible`, `shortcut`,
`placement` e, só quando visível, o `readout`); `./scripts/ui invoke toolbar.perf_meter toggle` e
`./scripts/ui set toolbar.perf_meter visible true|false` chamam o mesmo handler do atalho
`⌘⇧D` (`Ctrl+Shift+D` fora do macOS) e do botão do cabeçalho. Contrato e limites em
[perf-meter.md](features/perf-meter.md); a prova na janela real é `scripts/perf-meter-proof.py`.

## Histórico de desempenho (F049, BL145.2)

`./scripts/ui get perf.history` lê o estado do histórico de 60 s (`recording`, `visible`, `frozen`,
`selected_ms`, `has_data` e, havendo dados, `view`: janela, picos por métrica, eventos recentes, segmentos,
contadores e contexto). `./scripts/ui set perf.history recording true|false`, `visible true|false`,
`frozen true|false`, `selected_ms <ms>`, `export_to <caminho absoluto>` (inicia a escrita de um arquivo **novo** em
segundo plano e responde `state: writing` com `export_id`; o fim é lido em `get perf.history` →
`export`: `idle|writing|done|failed` com o erro tipado; um destino existente falha como
`perf_history_export_exists`, um temporário alheio como `perf_history_export_temporary_busy` e uma
segunda exportação durante a escrita é recusada com `perf_history_export_in_progress`) e `export_destination <caminho>` (preenche o campo
"Destino do JSON" do painel, sem escrever), e `./scripts/ui invoke perf.history
record_start|record_stop|show|hide|freeze|resume|clear|snapshot|clear_selection|export` (o botão Exportar JSON: grava no
destino do campo; vazio é `perf_history_export_path_missing`) chamam o mesmo handler dos
botões do painel e do cabeçalho. Gravar e mostrar são independentes: `show` abre um diálogo (a WebView do monitor
fica escondida, `get monitor.viewport` diz `on_screen:false`) e `hide` o fecha sem parar a gravação. Tempos são
milissegundos no relógio monotônico do histórico (`view.window` publica o intervalo). Contrato e limites em
[perf-history.md](features/perf-history.md); a prova na janela real é `scripts/perf-history-proof.py`. O alvo é
da janela: `tessel-headless` responde `desktop_required`.

## Headless access

This page documents the **running window** adapter. For editing without any desktop process,
use `./scripts/headless` or `tessel_editor_core` directly; see [core architecture](core-architecture.md).
Per-feature coverage and gaps are in [the matrix](features/README.md).

## One shared core

State lives in `tessel_editor_core::state::EditorState` (no GPUI). Every gesture builds a `state::Command`;
the CLI parses its arguments into the **same** `Command` and calls
`Workstation::execute`, which mutates the state, notifies the view and records the
last activity in the status bar. The tree is built from that state on every request,
so `tree`/`get` never describe something the window does not show.

The header is a Kit `Toolbar`; its buttons are listed in `tree` as `node:<N>` entries (parented to
`region:header`): Abrir `node:18`, Salvar `node:19`, Importar `node:14`, Desfazer `node:9`, Refazer `node:11`,
Empacotar `node:20`, Exportar `node:16`, Tarefas `node:21`. Each is enabled and runs the real operation (see
[Flows and dialogs](#flows-and-dialogs)); a button of an absent feature is disabled with a reason.
The dock layout is not part of `EditorState`: `tree`, `get dock` and `dock` read the live `DockArea`.

## IDs

| ID | Fields / actions |
|---|---|
| `region:header|toolbar|library|monitor|timeline|inspector|status` | `set region:<panel> visible true|false` (a dock panel) |
| `toolbar.panel.library`, `toolbar.panel.inspector` | `invoke` toggles the panel |
| `toolbar.theme` | `invoke` toggles dark/light |
| `folder:all`, `folder:<id>` | `select`; `set folder:<id> expanded true|false` (folders with children) |
| `media.search` | `set media.search value <text>` |
| `media:<id>` | `select` (feeds monitor and inspector) |
| `workspace.tabs`, `inspector.tabs` | `set … value <option>` (options listed by `get`) |
| `inspector.<section>` | `set … expanded true|false`; `invoke … reset` |
| `property:<id>` | `set … value <v>`; `invoke … reset` |
| `timeline.zoom`, `timeline.snap`, `timeline.playhead` | `value` / `value` / `frame` |
| `track:<id>` | `select`; `set … hidden|muted <bool>`; `move --before/--after` |
| `clip:<id>` | `select`; `move --track --frame --snap` |
| `monitor.play` | `invoke`: play/pause the sequence from the playhead ([F018](features/playback.md)) |
| `flow.open|save_as|recover|import|import_project|package|export|jobs` | `invoke` opens the dialog |
| `dialog.field.<name>` | `set … value <text>`: fills a field of the open dialog |
| `dialog.submit|cancel|verify` | `invoke`: run, close, or refresh the report of the open dialog |
| `jobs` | `get`: `active` and `recent` jobs (id, kind, state, stage, progress, error) |
| other `monitor.*`, `media.insert`, `timeline.tool.*` | disabled, with `disabled_reason` |

Track IDs are stable (`titles`, `picture`, `score`, `ambience`); labels V2/V1/A1/A2
come from the current order.

## Policies

- Slider, zoom and playhead values outside the range are **clamped** and the response
  says `clamped: true` with the stored value. Non-numeric or non-finite → `invalid_value`.
- Slider values are rounded to the property step. Selects accept only listed options.
- `end_zoom` requires `ken_burns`; otherwise `disabled_element`.
- Clips can only live in 0..696 frames (29 s at 24 fps). Out of range → `out_of_bounds`
  (rejected, nothing changes). Overlap on the destination track → `overlap` (rejected,
  names the conflicting clip). Audio on video tracks and titles on audio → `incompatible_track`.
- Snap (default on, threshold 6 frames) pulls the start/end to 0, sequence end, playhead
  and other clips' edges, but only when the snapped placement is valid. `--snap false`
  disables it for one move; the response reports `snapped_from`.
- Reordering tracks must keep video tracks above audio → `invalid_track_order`.
- A cancelled drag (Escape or release outside) leaves the state untouched: a drag only
  produces a preview, the model changes on drop through the same `Command`.

## Contract

Responses: `{"ok":true,"version":1,"data":…}` or
`{"ok":false,"version":1,"error":{"code","message","target?","details?"}}`; failures exit
nonzero. Error codes: `unknown_element`, `unknown_field`, `unknown_action`,
`disabled_element`, `invalid_value`, `missing_argument`, `out_of_bounds`, `overlap`,
`incompatible_track`, `invalid_track_order`, `not_applicable`, `not_expandable`, `invalid_duration`.
The CLI validates against the live tree before executing, so unknown and disabled
elements are rejected uniformly. `dispatched: true` on window actions means the operation
was issued; OS animation may finish later.

## Local transport

Unix socket `~/.cache/tessel-editor/ui.sock`, directory mode 700, socket 600. On Windows the same
protocol runs over a native AF_UNIX socket (`uds_windows`) at `%LOCALAPPDATA%\tessel-editor\ui.sock`;
there are no mode bits, the per-user directory ACL is the restriction, and the CLI is
`target\debug\tessel-editor.exe ui <command>` (`./scripts/ui` is a `sh` wrapper). One instance
owns the endpoint; a stale socket is removed at startup and a live one is never replaced.
No HTTP listener. Newline-delimited JSON, 64 KiB maximum, read/write timeouts:

```json
{"version":1,"command":"set","target":"property:zoom","field":"value","value":"110"}
{"version":1,"command":"move","target":"clip:first-light","args":{"track":"picture","frame":"12"}}
```

Only registered actions run, never shell commands. Closing is acknowledged before exit.

## Docking

The four panels (`panel:library`, `panel:monitor`, `panel:timeline`, `panel:inspector`) live in
the Kit `DockArea`. Gestures (dragging a panel header, dropping on a tab group or an edge,
dragging a divider, the collapse buttons) and the CLI end in the same area; `dock` reads it
back, so the CLI never reports a layout the window does not show.

```sh
./scripts/ui dock                              # docks, groups, splits, panels, hidden panels
./scripts/ui dock show|hide|activate <panel>
./scripts/ui dock move <panel> --to left|right|bottom|center
./scripts/ui dock move <panel> --group group:N|panel:P [--index N]    # join a tab group
./scripts/ui dock move <panel> --beside group:N|panel:P --side left|right|top|bottom
./scripts/ui dock resize left|right|bottom <px>
./scripts/ui dock open|close left|right|bottom
./scripts/ui dock split split:N 120,300        # divider sizes, px
./scripts/ui dock load <json | @file>          # restore a `layout.dump` from `dock`
./scripts/ui dock reset                        # default layout
```

Equivalent spellings: `invoke panel:<id> hide|show|activate`, `set panel:<id> visible false`,
`set dock:left size 300`, `set dock:bottom open false`, `set split:N sizes 120,300`,
`move panel:<id> --to bottom`, `invoke dock reset`, `invoke toolbar.panel.<id> toggle`.

- **Stable IDs:** `panel:<kind>` and `dock:center|left|right|bottom` never change. `group:N` and
  `split:N` are numbered by the live area and are renumbered after a reset or load: read them
  from `dock` before using them.
- **Structured errors, nothing changes on error:** unknown panel/group/split → `unknown_element`
  (`details` lists what exists); sizes outside the dock's bounds → `out_of_bounds` (`details` has
  `min`/`max`); malformed layout JSON or unknown panel names → `invalid_value`. The tests compare
  the whole layout before and after a rejected command.
- An emptied side dock collapses (`open: false`) and reopens when a panel moves back in.
- There is only ever one monitor panel. The centre dock itself cannot be collapsed or resized as a
  region (`not_applicable`); the monitor panel can be hidden, tabbed or moved like any other.
- `layout.dump` is the Kit's own `DockAreaState`; nothing is written to disk automatically.

## Boundaries

The CLI automates semantic state; it is not OS accessibility, pixel geometry or a visual
check. Drag *gestures* themselves (pointer paths) are covered by unit tests of the pure
geometry (`workstation/drag.rs`) and of `plan_move`/`plan_reorder`, not by automated pointer
input. Every new control must follow the checklist in `AGENTS.md`.

## Native window sizing

```sh
./scripts/ui set window size 960x640
./scripts/ui get window
./scripts/ui set window size 1440x900
```

Requests require integer dimensions from 960×640 through 8192×8192; invalid
requests fail without resizing. The `window` node is included in `tree`.
Resize acknowledgement records dispatch; query status after the native resize.

## Timeline trim handles

Each clip exposes two handles: `trim:<clip-id>:start` and `trim:<clip-id>:end`.
Their `frame` field is the absolute sequence frame. For example:

```sh
./scripts/ui get trim:first-light:end
./scripts/ui set trim:first-light:end frame 120
./scripts/ui set trim:first-light:start frame 24
```

The opposite edge stays fixed. At least one frame is required; negative frames,
sequence overflow and overlap are rejected atomically. Snap follows timeline.snap.
Pointer dragging uses the same TrimClip command: previews do not mutate state,
drop commits and Escape/outside release cancels. This changes demonstration
sequence timing only, without trimming source files or rendering media.

For regression tests of the gesture routing itself (rather than direct state
commands), run `./scripts/cargo test workstation::input_tests --locked` (and `workstation::dock_tests` for the dock). These tests send
mouse down/movement/release and Escape through GPUI's test platform. The test
locates handles from measured geometry, without controlling the OS mouse.

## Property slider gestures

Property sliders preview every update but commit one undo entry on release (BL062). Lib,
headless JSONL and the window use the same `Command::PropertyGesture` handlers. A discrete
`set` remains one edit. Timeline zoom is presentation state and stays outside undo.

```json
{"op":"property.gesture.begin","property":"zoom"}
{"op":"property.gesture.update","value":120}
{"op":"property.gesture.update","value":140}
{"op":"property.gesture.commit"}
```

`begin` locks the selected clip (or explicit `clip`) and returns a numeric `gesture` ID.
Pass that ID on subsequent requests when driving asynchronous input; omitting it uses the
current gesture. `property.gesture.cancel` restores the original without an undo entry;
no change or returning to the original produces no entry. `history.status` exposes the
open gesture. Native pointer evidence and command evidence remain distinct; see
[properties](features/properties.md) and the [report](slider-gesture-history-report.md).

## Cena dirigida: inspector e monitor

Nós da janela: `inspector.mode` (clip/scene), `scene.panel`, `scene.picker` (quando houver várias cenas), `scene.tree`, `scene.entity:<path>`, `scene.property:<path>` e `monitor.scene`. Expandir a árvore expõe descendentes; eles não são listados como visíveis quando o pai está recolhido.
Um set de componente de propriedade usa begin/update/commit compartilhados (um undo).
`ui op '{"op":"scene.resources.resolve","scene":"id","base":"/raiz/absoluta"}'` cria job; conferir job.get e aplicação antes de exibir. `monitor.show_scene {scene}` seleciona a apresentação; `scene.render` responde depois do frame real. `scene.plan` é a consulta core/headless. Inspeção de pixels não prova ponteiro ou aparência.

### Gizmo de translação (F055)

Nó `monitor.gizmo` (só com cena anexada): `get monitor.gizmo` devolve `selection`, `dragging`, `handle`, `gesture`; o overlay que a página recebeu e o que ela desenhou (com pixels amostrados por handle) estão em `get monitor.viewport` → `state.overlay`. Não há botão para clicar: `ui op '{"op":"scene.pointer.down","scene":"id","frame":N,"point":{"u":0.6,"v":0.5}}'` seguido de `scene.pointer.move`/`scene.pointer.up` (ou `scene.pointer.cancel` com `reason`) chega ao mesmo handler que as mensagens `overlay.pointer` da página, e `scene.translate` dá o mesmo resultado sem ponteiro. Coordenadas são da **imagem** (u, v em 0..1), não da tela: isto não é um ponteiro físico (BL043). `state.overlay` também traz os contadores de custo e identidade (`computed`, `adopted`, `memo_hits`, `held`, `epoch`, `stale`, `sets`): sem o feature negociado ou com o overlay desligado, `computed` não sobe; durante a reprodução só `held` sobe (uma vez). `state.overlay.error` guarda o último `overlay.error` da página (desenho que falhou ou payload recusado, com `request`): é consumido pelo adaptador fora da sessão de render, então o monitor continua disponível e as capacidades não se perdem; só o `error` genérico do protocolo derruba a sessão. Contrato em [scene-gizmo.md](features/scene-gizmo.md); `scripts/scene-gizmo-proof.py` usa só estas requisições. O relatório do script é **atômico e sempre reescrito**: um `ok:true` antigo nunca sobrevive a uma execução que falha ou é interrompida (um stub `ok:false` é gravado antes de tudo), só `ok:true` com os **dois processos** que o script lança (`headless`, o binário `tessel-headless` em JSONL, e `native`, a janela real pelo socket) — `SURFACES` tem exatamente esses dois, `final_blockers` exige os dois — o manifesto de checagens por superfície completo e a árvore limpa conta como final; `--surfaces` parcial ou `--allow-dirty` nunca são finais; o contrato do projeto tem **três superfícies** (biblioteca, CLI headless, UI), mas a biblioteca não é um terceiro processo desta prova: ela é provada pelos testes `cargo test` do core (`crates/core/tests/scene_gizmo.rs`), e a prova do script compara o arrasto entre os dois processos (`surfaces_agree_on_the_drag`); o diretório de trabalho de uma superfície que falha é mantido (`retained`) com `headless.stderr.log` e a cauda do log.

### Importação visual de cena (F043, R2)

Botão do cabeçalho: `node:26` (ícone `node:27`). Operações `flow.attach_scene` e `flow.scene_resources` abrem os diálogos; `dialog.field.*`/submit/cancel compartilham os handlers da UI. Nó `scene_import` expõe passo/job/done/error/presentation_error. A cadeia aguarda job concluído **e aplicado** antes de seguir para o monitor. A seleção nativa de arquivo ainda não tem prova; campos por caminho têm evidência semântica.

### Exportação de cena dirigida (F042)

`flow.export_scene` abre o diálogo com campos `scene`, `preset`, `from`, `frames`, `audio` (`yes`/`no`) e `destination`. `dialog.submit` envia `scene.export`, com a mesma validação do core. O botão Exportar (`node:16`) abre esse fluxo quando o inspector está na aba Cena. `scene.export.preflight` não cria export; a operação de export retorna job na janela e `job.get`/`job.cancel` acompanham e cancelam. A integração compila e passa nos testes desktop; a prova nativa de export desta integração fica a cargo da R10.
