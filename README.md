# Tessel Editor

Open-source desktop editor by [Journey Studios](https://github.com/journey-studios/tessel-editor),
licensed under [MIT](LICENSE). Third-party components retain their
[own notices](THIRD_PARTY_NOTICES.md).

Independent Rust desktop editor workstation built on [GPUI Kit](https://gpui-kit.com)
0.9.0 (GPUI snapshot `gpui-pre` 0.3.11). Library, single monitor, timeline and
inspector are panels of the Kit's real `DockArea`: headers drag, tabs and splits
form, dividers resize, and the CLI edits the same live area. The state is an
demonstration by default, but the editor can now create, save, open, undo, package and exchange
projects, import real media, preview it with Spark splats, play it with a clock and export H.264/AAC MP4
up to UHD 4K. See [docs/media-project-export-report.md](docs/media-project-export-report.md) for what is
proven and what is not (notably: nothing was ever *heard* by a person, and the player adapter has a
large start-up latency). The original scene-engine migration is in progress; see [the capability matrix](docs/features/README.md)
for the implemented surfaces and remaining work.

## Run

Use the cached Cargo wrapper (see [docs/cargo-worktrees.md](docs/cargo-worktrees.md)):

```sh
./scripts/cargo run -p tessel-editor --locked
```

macOS needs full Xcode and Metal Toolchain. If necessary:

```sh
xcodebuild -downloadComponent MetalToolchain
```

Rust 1.99.1 is pinned in `rust-toolchain.toml`: `gpui-pre` 0.3.11 uses
`std::hint::cold_path`, which does not compile on the previous 1.94.1 pin (E0658).
Only `gpui-kit` is declared; GPUI comes through it (`gpui_kit::*`), so there is a
single GPUI family in `Cargo.lock`. The default backend is `gpui-pre` with a persistent native WebView; `gpui-fast` is experimental
(see the viewport reports). Linux has not been validated. On Windows the editor builds and opens with
plain `cargo build -p tessel-editor --locked` (MSVC Build Tools, `pnpm install --frozen-lockfile && pnpm run build` in
`runtime/`). Use `--release` to judge speed: debug builds are several times slower. The `ui` CLI works over a
native Windows AF_UNIX socket (`target\debug\tessel-editor.exe ui status`); see BL054 in
[docs/backlog.md](docs/backlog.md).

## Validate

```sh
cargo fmt --all -- --check
./scripts/cargo check --workspace --locked
./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings
./scripts/cargo test --workspace --locked
./scripts/cargo build -p tessel-editor --locked
```

## Structure

- `crates/core`: public GPUI-free library, headless executable, the `Command` enum and the
  pure `Layout` model the dock CLI validates against. The single source of truth for
  editing; the dock layout itself lives in the `DockArea`.
- `crates/desktop/src/workstation`: Kit views. `dock.rs` hosts the four panels in
  `DockArea`/`DockSkin`; `controls.rs` wraps Kit `Button`, `TabBar`, `Slider`, `Select`,
  `Input`, `Tree`; `viewport.rs`/`viewport_host.rs`/`ops.rs` host the persistent WebView viewport and the `ui op` bridge (GPUI-free logic in `viewport_host.rs`); the timeline
  is custom drawing; `*_tests.rs` hold GPUI input and dock tests.
- `crates/media`: file-system, FFmpeg and archive adapters injected into the core's `Extension` hook
  (import, derived media, packages, project interchange, video export) plus the `tessel-media` JSON Lines
  binary (`./scripts/media`). Setup: [docs/media-setup.md](docs/media-setup.md).
- `crates/render`: the real render backends (Node + headless Chrome worker, WebView asset
  server) and the `tessel-render`/`tessel-fixtures` binaries. `runtime/`: the local Three.js
  runtime (JS, built with esbuild by `./scripts/runtime setup`, always pnpm). See
  [docs/features/viewport.md](docs/features/viewport.md). Scrubbing a container video draws a derived
  540-row preview proxy (`op preview.policy`, node `monitor.preview.quality`; the picture is refined to the
  original when the scrub settles, and explicit frames and exports always use the original). Proofs:
  `scripts/proxy-live-proof.py`, `scripts/proxy-soak.py`; numbers in
  [docs/viewport-performance-next-report.md](docs/viewport-performance-next-report.md).
- `crates/desktop/src/header.rs`: unified header (Kit `Toolbar`/`Button`).
- `crates/desktop/src/theme.rs`: dark/light through the Kit `Theme`.
- `crates/desktop/src/automation`: local socket server, CLI client, argument parsing.
- `assets/demo`: demonstration stills, see [docs/proui-reference.md](docs/proui-reference.md).
- `docs/gpui-kit-migration-report.md`: versions, components, dock commands, evidence, limits.
- `docs/archive/editor.tailor.txt`: the retired Tailor header design, kept for history only.

References: [GPUI Kit](https://gpui-kit.com/docs/installation/),
[ProUI Splice](https://pro-ui.dev/examples/video-editor).

## Control the running UI

```sh
./scripts/ui tree
./scripts/ui set property:zoom value 110
./scripts/ui move clip:river-detail --track picture --frame 360
./scripts/ui dock                                  # live dock layout
./scripts/ui dock move inspector --to bottom
./scripts/ui dock resize left 300
./scripts/ui dock reset
./scripts/ui theme light
```

See [docs/ui-automation.md](docs/ui-automation.md) for commands, IDs, value policies,
the dock commands and the local IPC contract. Playback and header file actions use the shared workflows. Unimplemented controls stay
disabled and report why.

## Core and feature documentation

Start at [docs/README.md](docs/README.md) and the [feature matrix](docs/features/README.md).
Every implemented or changed feature must update its own Markdown contract and matrix row.
UI and CLI consume `tessel-editor-core`; no domain rule requires a window.

```sh
./scripts/headless <<'JSONL'
{"op":"set","target":"timeline.playhead","field":"frame","value":120}
{"op":"status"}
JSONL
```

This runs without GPUI or a desktop socket. See [the core API and remaining limits](docs/core-architecture.md)
and the project skill [tessel-feature-contract](.agents/skills/tessel-feature-contract/SKILL.md).
The default project is demonstrative; dynamic project/track creation and media processing
are implemented as listed in the feature matrix. Directed-scene rendering and its visual
editing surface remain pending; see [the shared backlog](docs/backlog.md).
