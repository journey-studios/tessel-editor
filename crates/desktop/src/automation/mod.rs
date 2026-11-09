#[cfg(unix)]
mod client;
#[cfg(unix)]
mod commands;
#[cfg(unix)]
mod model;
#[cfg(unix)]
mod server;

#[cfg(unix)]
pub use client::run_cli;
#[cfg(unix)]
pub use server::start;

#[cfg(unix)]
use std::path::PathBuf;

#[cfg(unix)]
fn socket_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home).join(".cache/tessel-editor/ui.sock"))
}

/// The automation transport is a Unix domain socket with owner-only permissions. Other
/// platforms open the window without it until an equivalent local transport is validated
/// (BL054); the editor stays usable by hand.
#[cfg(not(unix))]
mod unsupported {
    use gpui_kit::*;

    use crate::shell::EditorShell;

    pub fn start(
        _window: AnyWindowHandle,
        _shell: Entity<EditorShell>,
        _cx: &mut App,
    ) -> Result<(), String> {
        eprintln!("UI automation is unavailable on this platform (Unix socket transport only)");
        Ok(())
    }

    pub fn run_cli(_args: &[String]) -> Result<(), String> {
        Err("`ui` needs the Unix socket transport, which this platform does not have".into())
    }
}

#[cfg(not(unix))]
pub use unsupported::{run_cli, start};
