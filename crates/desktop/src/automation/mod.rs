mod client;
mod commands;
mod model;
mod server;

pub use client::run_cli;
pub use server::start;

use std::path::PathBuf;

// One transport on every platform: a local stream socket at a per-user path. Windows has native
// AF_UNIX sockets (Windows 10 1803+), which `uds_windows` exposes with the `std` API.
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};

/// `~/.cache/tessel-editor/ui.sock`, or `%LOCALAPPDATA%\tessel-editor\ui.sock` on Windows. Both
/// directories belong to the current user.
fn socket_path() -> Result<PathBuf, String> {
    #[cfg(unix)]
    {
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
        Ok(PathBuf::from(home).join(".cache/tessel-editor/ui.sock"))
    }
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?;
        Ok(PathBuf::from(base).join("tessel-editor").join("ui.sock"))
    }
}
