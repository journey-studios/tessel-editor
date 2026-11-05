# Editor workstation

Mode: Operate. Compact workstation in the style of the Splice reference, on GPUI Kit.

- Unified 40 px header (Kit `Toolbar` + compact `Button`s) with native macOS controls; 80 px
  reserved on the left (window configuration in `main.rs`). Window starts at 1440 × 900,
  minimum 960 × 640. The header and the status bar are fixed; they are not dock panels.
- Between them sits one `DockArea`. Default layout: library left (232 px), the **single**
  monitor in the centre, timeline below (296 px), inspector right (272 px). Panel headers are
  draggable, panels can join tab groups or split beside one another, dividers resize, and an
  emptied side dock collapses (it reopens when a panel returns).
- Timeline: 24 px ruler, 128 px track-header column, continuous lanes (titles 36 px, picture
  72 px, audio 40 px). Clips are rectangles proportional to duration with repeated thumbnails
  (video) or illustrative waveform bars (audio). Custom drawing on purpose: the Kit has no
  multitrack timeline.
- Density: 1 px borders, 11 px text, small radius, no large cards. The dark/light palette is the
  Kit's active theme (`ActiveTheme`); `workstation/tokens.rs` only derives workstation roles
  (audio green/teal, selection amber) from it, never ad hoc hex. The selected workspace pill and
  primary accents follow the Kit's primary colour (neutral in the default theme).
- Icons come from the Kit (`IconName`, Lucide). Icon-only buttons carry a tooltip that is also
  their accessible name. The Kit `Tab` draws an icon or a label, never both, so the workspace
  mode bar uses labels.
- Reduced chrome on purpose: no motion beyond what the Kit controls do themselves.

## Shared core

`crates/core` owns domain state and rules without GPUI. The desktop consumes that public
library; gestures and both CLIs translate to shared commands. Presentation-only DockArea
state remains in the desktop. See `docs/core-architecture.md` and `docs/features/README.md`.

## Boundary between the Kit and our code

Kit components are used as they are (`Button`, `TabBar`, `Slider`, `Select`, `Input`, `Tree`,
`Accordion`, `Switch`, scroll, `DockArea`). Small wrappers in `workstation/controls.rs` bind
them to `Command`s; widgets never keep a second copy of a value, they are re-synchronised from
`EditorState` on every render. Reactive regions read `EditorState`; the automation tree is built
from that state and from the live `DockArea`.

## Viewport seam

`workstation/viewport.rs::render_viewport` is the only place that paints the monitor picture.
A future WebView/Three.js viewport replaces that function; nothing else (dock, state, CLI) knows
how the picture is drawn. See the report for the unvalidated POC plan.

## Demonstration data

`assets/demo/{highlands,coast}.png` are used as stills and thumbnails. Waveforms are
deterministic illustrative bars, not audio. The monitor shows the still of the selected
media; there is no playback.

ProUI Splice is an additional visual reference only. No ProUI dependency or kit is used.
The Tailor header design was retired; `docs/archive/editor.tailor.txt` is history.
