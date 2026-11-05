# Component implementation report

> **Histórico.** Escrito antes da migração para GPUI Kit (ver `gpui-kit-migration-report.md`). Menções a Guise, Tailor, GPUI 0.2.2 e a controles manuais descrevem o estado anterior e não valem mais.

Plan: [component-implementation-plan.md](component-implementation-plan.md). State: demonstration only,
in memory, macOS (Metal) only. Evidence key — **T** unit/behaviour test (`cargo test`),
**CLI** exercised against a running instance (`target/debug/tessel-editor ui …`, shared
`Workstation::execute`), **Code** implemented but verified only by reading/compiling.
At the delegated handoff, shell screen capture failed and no visual review had been
performed. The parent verification below supersedes that limitation for the listed captures.
There is no claim of exact pixel fidelity to Splice.

| ID | Status | Evidence |
|---|---|---|
| C01 Tokens | Done | `workstation/tokens.rs` from Guise palette, dark/light. CLI `theme light/dark` returned the real scheme; Code for appearance. |
| C02 Panel shell | Done (layout unreviewed visually) | `shell.rs`, `workstation/mod.rs`; CLI `status` reports window 1440×874, panels visible; `invoke toolbar.panel.library` toggled twice. Smaller window not inspected. |
| C03 IconButton/ToolbarGroup | Done | `controls.rs`; disabled controls (`monitor.play`, `media.insert`, header buttons) return `disabled_element` with reason (CLI). Tooltips/focus: Code. |
| C04 Folder tree | Done | `state/library.rs` T (counts 7/4/1/2, expand/collapse, select); CLI `set folder:north-expedition expanded false`, `invoke folder:audio select`, `folder:all`. |
| C05 Media library | Done | T search/results; CLI `set media.search value river` → 1 result, empty → 7; `invoke media:ao03 select` updated monitor. |
| C06 Tabs | Done | T; CLI `set inspector.tabs/workspace.tabs`; unimplemented workspace modes report `implemented:false` and show a notice (Code). |
| C07 Accordion | Done | T `state/inspector.rs`; CLI `set inspector.cropping expanded true`. Keyboard: Code. |
| C08 PropertyRow | Done | `controls.rs property_row`; CLI `get property:<id>` lists type/limits/unit; invalid rejected (`abc`). |
| C09 Slider | Done | T quantize/value_at/value_after_key, drag geometry; CLI `set property:zoom value 110`, 999 → 400 `clamped:true`. Pointer drag not automated. |
| C10 Select | Done | T `adjacent_option`; CLI `set property:blend value multiply`, `bogus` → `invalid_value` listing options. Popup open/close: Code. |
| C11 Inspector + thumbnail | Done | Per-clip overrides and reset T; CLI zoom set on selected clip; `end_zoom` disabled when Ken Burns off. Changing values does not render anything. |
| C12 Single monitor | Done | One monitor region; transport disabled with reason (CLI); still from selected media. |
| C13 TimelineToolbar | Done | CLI `set timeline.zoom value 60` (1 → 8 clamped), `timeline.snap` set/toggle. Razor/delete tools disabled. |
| C14 Ruler + playhead | Done | T ruler/scrub math; CLI `set timeline.playhead frame 72` (5000 → 696, -3 → 0, `NaN` rejected) with timecode and fps. No playback. |
| C15 Track header/lane | Done | T; CLI `invoke track:score select`, `set track:titles hidden true`; labels V2/V1/A1/A2 from order. |
| C16 ClipBlock | Done | CLI `invoke clip:river-detail select`, `get` returns track/start/duration/end. Thumbnails/amber border: Code. |
| C17 Drag and drop | Done (gestures not automated) | T `plan_move`/`plan_reorder`/drag geometry (bounds, snap, overlap, incompatible, reorder rules, cancel). CLI `move clip …` success, `out_of_bounds` (9999, -5), `overlap`, `incompatible_track`, `unknown_element`; `move track … --before/--after`, `invalid_track_order`. Mouse drag, drop markers and Escape cancel are wired to the same commands but were not exercised by pointer input. |
| C18 Waveform | Done | `waveform.rs`, deterministic `waveform_amplitude` T; clip `get` reports the waveform as demonstrative. Not audio. |
| C19 StatusBar | Done | `status.summary` from `EditorState::status_summary` T; CLI `get status.summary` and `status` → `editor.summary`. |
| C20 Semantic automation | Done | `state/tree.rs`, `automation/*`. T parsing/model; CLI tree (97 nodes), get/actions/status/set/move/invoke/theme, structured errors with nonzero exit. |

## Policies chosen

Clamp-and-report for numeric ranges; reject (never silently shift) for clip bounds, overlap
and incompatible tracks; snap only to valid placements; tracks keep video above audio.
See [ui-automation.md](ui-automation.md).

## Tailor boundary

Tailor document: header only. The export (`scripts/export-tailor.py`) re-run produced an
identical `main_screen.rs` and `theme.rs`; `design.json` differs only by its temporary
`export_dir`. The live tree merges Tailor nodes (`node:N`) with the live state.

## Validation (last run)

- `cargo fmt --all -- --check`, `cargo check --workspace --locked`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean.
- `cargo test --workspace --locked`: 60 passed.
- CLI smoke test against the running app: ~60 commands (set/move/invoke/theme, success and error paths) as listed above.

## Limitations

- No visual verification (no screenshots) at wide or small window sizes; density and the
  960×640 minimum are unreviewed.
- Pointer drag/slider/scrub gestures, Escape cancel, select popup and keyboard handlers are
  not covered by automated input.
- No playback, export, processing, persistence, undo or Three.js migration; waveforms are
  illustrative; demo images are of unverified redistribution licence (see `proui-reference.md`).
- Header strings are Portuguese/English literals (no i18n layer in this project).

## Parent verification — 2026-11-04

- Re-ran format, check, Clippy, tests and native build. Final suite: **62 passed**.
- Observed the native app through the computer-use screenshot API without clicks;
  the delegated shell capture error (`could not create image from display`) did
  not prevent observation through the available app capture API.
- Inspected a wide window and the actual 960×640 window. The small capture exposed
  overlapping header content; shortened header labels and hid redundant badges
  in the Tailor source, regenerated and recaptured. Header overlap is resolved.
- Single monitor, library thumbnails/folders, inspector sliders/accordions/select
  and timeline lanes/thumbnails/waveforms were visible. Inspector and media use
  their existing scroll areas in the smaller viewport; long timeline content
  is horizontally scrollable. This is a composition check, not full pixel QA.
- Added native CLI `set window size WIDTHxHEIGHT`, queryable `get window`, size
  validation and two tests. Actual status confirmed 960×640. Restored 1440×900.
- CLI set `property:zoom` to 115; the rendered inspector displayed 115.0 and a
  modified marker. Toggled cropping through CLI and read the state back.
- Shortened the blend property label to `Mesclagem` to avoid clipping.
- Normalized Tailor's temporary export directory to `target/tailor-export`.
  A second export produced identical header Rust, theme, design snapshot and
  editable Tailor document (SHA-256 comparisons). Hidden header node visibility
  is now reported correctly by the automation tree.
- Pointer drag/drop, scrub, select popup input and keyboard gestures remain
  unexercised by physical input; their shared commands/geometry retain test coverage.

The app is left open. No commit, push, PR, deployment or original Tessel change.

## Trim handles — 2026-11-04

Added visible start/end grips and horizontal resize cursors to all seven clips
(video/title/audio), plus preview geometry, snap and commit-on-drop using the
same TrimClip command as the CLI. Handles are independently discoverable in
the live tree. Invalid preview uses the error border; cancellation discards it.

Validation: 65 tests passed, Clippy clean, native build passed. Three new tests
cover preserved opposite edges, atomic invalid requests, and nonmutating snapped
previews. Live CLI verified all 14 handles, both edges, duration changes,
overlap/bounds/zero-length errors, and restored original timing. Native screenshot
confirmed visible grips. Physical pointer dragging remains unexercised.

## Handle input regression — 2026-11-04

Reproduced with the GPUI test platform's real event routing: mouse down, a single
threshold-crossing movement, then release kept the clip at 192 frames instead
of the expected 120. The older model/CLI tests could not expose this routing bug.
The movement starts GPUI's drag and stops propagation; no preview existed if
release occurred before another movement. The drop now computes its trim from
the final pointer position before committing. Handle mouse-down focuses the
workstation, and Escape stops the active GPUI drag as well as clearing preview.

`cargo test workstation::input_tests --locked` drives actual GPUI mouse/key
events through the displayed components: continuous right trim, single-motion
right trim, left trim preserving the opposite edge, and Escape followed by
release over the timeline. All four pass. Test-only GPUI support is enabled
as a dev dependency; it does not change the shipped app dependency features.
