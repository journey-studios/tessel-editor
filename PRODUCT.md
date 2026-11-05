# Tessel Editor

## Platform and stack

Desktop Rust + GPUI Kit (docking, controls and theme from `gpui-kit`).
Initial validation target: macOS. Other platforms remain unvalidated.

## Confirmed scope

Prepare an independent application and reproduce the existing Three.js editor UI.
The shell now includes interactive demonstration state and a public Rust core library.
The core must support complete programmable editing flows without UI; desktop and CLI
are adapters over shared rules. Every feature has a Markdown contract and surface coverage
in `docs/features/README.md`. The current headless session operates on the demo model.
Playback, scene renderer, real media processing, persistence, network services and migration
of the existing engine remain outside the current implementation.

## Source

The matching existing application is `apps/scene-studio`, inspected in
`~/www/journeystudios/tesselapp-image-coverage`. It uses Three.js, layers/lights,
a central viewport, an inspector, and camera/keyframe/audio tracks.

## Open decisions

First functional migration slice and backend/rendering strategy are undecided.
