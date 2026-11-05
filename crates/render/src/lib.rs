//! Headless viewport host: runs the Three.js runtime in a real browser behind a Node worker.
//!
//! The core knows nothing about processes. This crate provides the [`ProcessTransport`] that
//! carries bridge v1 JSON Lines to `runtime/worker/worker.mjs` and the discovery of what the
//! worker needs (Node, the built runtime, `puppeteer-core`, Chrome). Anything missing is a
//! `backend_unavailable` error that says how to fix it; nothing here simulates rendering.

pub mod host;
mod process;
mod runtime;

pub use process::ProcessTransport;
pub use runtime::{RuntimeConfig, SetupError, demo_dir, discover_page, session, worker_backend};
