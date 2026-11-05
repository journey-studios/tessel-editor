//! Media adapters of the editor: everything that touches files, FFmpeg or long-running work.
//!
//! The core owns the rules (commands, IDs, plans); this crate does the I/O it must never do on a
//! UI thread and reports through jobs. No GPUI, no window.

pub mod derive;
pub mod export;
pub mod fsx;
pub mod host;
pub mod import;
pub mod interchange;
pub mod package;
pub mod probe;
pub mod tools;
pub mod zip;
