mod client;
mod commands;
mod model;
mod server;

pub use client::{HELP as CLI_USAGE, run_cli};
pub use server::start;

use std::ffi::OsString;
use std::path::PathBuf;

// One transport on every platform: a local stream socket at a per-user path. Windows has native
// AF_UNIX sockets (Windows 10 1803+), which `uds_windows` exposes with the `std` API.
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};

/// Environment variable that moves the socket of one window (and of the `ui` client talking to
/// it) to another absolute path. A proof uses it to run its own window next to the user's: on
/// Unix it also changes `HOME`, but on Windows the default path is under `%LOCALAPPDATA%`, where
/// the FFmpeg that WinGet installed is found too (BL167.2), so changing that would hide it.
pub const SOCKET_ENV: &str = "TESSEL_UI_SOCKET";

/// `TESSEL_UI_SOCKET` when set, else `~/.cache/tessel-editor/ui.sock`, or
/// `%LOCALAPPDATA%\tessel-editor\ui.sock` on Windows. The default directories belong to the
/// current user; the server makes the socket's directory private the same way for an override,
/// so an override should name a directory of its own.
fn socket_path() -> Result<PathBuf, String> {
    let base = if cfg!(windows) {
        "LOCALAPPDATA"
    } else {
        "HOME"
    };
    socket_path_from(std::env::var_os(SOCKET_ENV), std::env::var_os(base))
}

/// [`socket_path`] from the values of `TESSEL_UI_SOCKET` and of the platform's base directory.
fn socket_path_from(socket: Option<OsString>, base: Option<OsString>) -> Result<PathBuf, String> {
    if let Some(socket) = socket.filter(|socket| !socket.is_empty()) {
        let socket = PathBuf::from(socket);
        if !socket.is_absolute() {
            return Err(format!("{SOCKET_ENV} must be an absolute path"));
        }
        return Ok(socket);
    }
    #[cfg(unix)]
    {
        let home = base.ok_or("HOME is not set")?;
        Ok(PathBuf::from(home).join(".cache/tessel-editor/ui.sock"))
    }
    #[cfg(windows)]
    {
        let base = base.ok_or("LOCALAPPDATA is not set")?;
        Ok(PathBuf::from(base).join("tessel-editor").join("ui.sock"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    fn absolute(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn the_default_socket_is_under_the_users_own_directory() {
        let base = absolute("user");
        let path = socket_path_from(None, Some(base.clone().into())).unwrap();
        assert!(path.starts_with(&base), "{}", path.display());
        assert!(path.ends_with("ui.sock"));
        // An empty override is no override.
        assert_eq!(
            socket_path_from(Some(OsString::new()), Some(base.into())).unwrap(),
            path
        );
    }

    #[test]
    fn an_absolute_override_wins_and_a_relative_one_is_refused() {
        let own = absolute("proof").join("ui.sock");
        assert_eq!(
            socket_path_from(Some(own.clone().into()), Some(absolute("user").into())).unwrap(),
            own,
            "the base directory is not even needed"
        );
        assert_eq!(
            socket_path_from(Some(own.clone().into()), None).unwrap(),
            own
        );
        let error =
            socket_path_from(Some("ui.sock".into()), Some(absolute("user").into())).unwrap_err();
        assert!(error.contains(SOCKET_ENV), "{error}");
    }

    #[test]
    fn without_override_and_base_the_error_names_what_is_missing() {
        let error = socket_path_from(None, None).unwrap_err();
        assert!(error.contains(if cfg!(windows) {
            "LOCALAPPDATA"
        } else {
            "HOME"
        }));
    }
}
