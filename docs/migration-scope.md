# Current scope

An interactive demonstration workstation. **Functional migration has not begun.**

Implemented, in memory only: folder tree, media search and selection, inspector tabs,
accordions, sliders, selects, workspace tabs, timeline zoom/snap/playhead, track and
clip selection, clip moves (including across compatible tracks) and track reordering,
all reachable from gestures and from the CLI through shared commands. Panels live in the
GPUI Kit `DockArea` (move, tab, split, resize, hide/show, reset), also reachable from the CLI.

Not implemented and not claimed: scene renderer (Three.js or Rust), playback, export,
audio or video processing, project loading, persistence, undo, backend, network.
Changing an inspector value does not render or export anything.

The source editor was located at `apps/scene-studio` in Tessel worktrees:
`~/www/journeystudios/tesselapp-image-coverage/apps/scene-studio`.
Its manifest declares Three.js 0.187.0. The source checkout was read only; no original
engine logic, private media or data was migrated.

Before functional migration, decide rendering strategy, document compatibility,
platform targets and acceptance criteria. Only macOS (Metal) has been built and run.
