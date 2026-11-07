//! Headless editor rules shared by desktop, CLI and Rust callers.
//!
//! The crate owns the project model (IDs, typed assets, tracks, clips, exact time), the
//! composition plan and the contracts of injected render/audio backends. It has no GPUI,
//! Wry or DOM dependency. See `docs/core-architecture.md` for the public contract and limits
//! and `docs/features/viewport.md` for the viewport pipeline.
pub mod assets;
pub mod audio;
pub mod backend;
pub mod bridge;
pub mod commands;
pub mod content;
pub mod directed_scene;
pub mod document;
pub mod fixtures;
pub mod header;
pub mod headless;
pub mod id;
pub mod interchange;
pub mod jobs;
pub mod ops;
pub mod perf;
pub mod persist;
pub mod plan;
pub mod preset;
pub mod preview;
pub mod scene_edit;
pub mod scene_export;
pub mod scene_interaction;
pub mod scene_plan;
pub mod scene_project;
pub mod scene_resources;
pub mod session;
pub mod splat;
pub mod state;
pub mod thumbnails;
pub mod time;
pub mod trace;

pub use id::Id;
pub use state::{Command, CommandError, EditorState, Raw};
