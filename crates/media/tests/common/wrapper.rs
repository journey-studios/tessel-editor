//! A stand-in for `ffmpeg`/`ffprobe` that the tests can make slow or broken on demand, and that
//! runs on every host the media tests run on (BL167.1).
//!
//! The wrapper is the real tool until a flag file exists, then a tool that is gone (it prints
//! `forced failure` and exits 1). It may also wait `delay` seconds before every run, which is how a
//! test keeps a job in flight on purpose. What is executable differs per host, the contract does
//! not:
//!
//! - Unix: a `#!/bin/sh` script with the executable bit.
//! - Windows: a `.cmd` file. `Command::new` runs it through `cmd.exe`, so a `TESSEL_FFMPEG` that
//!   points to it works for the `tessel-media` child process as well. There is no shell script
//!   and no `Permissions::from_mode` to port: a Windows file is executable by its extension.
//!
//! Arguments reach the real tool through `%*` (Windows) or `"$@"` (Unix), unchanged.

use std::fs;
use std::path::{Path, PathBuf};

/// Writes a wrapper called `name` into `dir` and returns the path to execute (on Windows it is
/// `name` plus `.cmd`). `real` is the tool it forwards to while `flag` does not exist.
pub fn forwarding_tool(dir: &Path, name: &str, real: &Path, flag: &Path, delay: f64) -> PathBuf {
    let path = dir.join(wrapper_file_name(name));
    fs::write(&path, script(real, flag, delay)).unwrap();
    make_executable(&path);
    path
}

#[cfg(unix)]
fn wrapper_file_name(name: &str) -> String {
    name.to_string()
}

#[cfg(windows)]
fn wrapper_file_name(name: &str) -> String {
    format!("{name}.cmd")
}

#[cfg(unix)]
fn script(real: &Path, flag: &Path, delay: f64) -> String {
    let wait = if delay > 0.0 {
        format!("sleep {delay}\n")
    } else {
        String::new()
    };
    let quote = |path: &Path| format!("'{}'", path.display().to_string().replace('\'', "'\"'\"'"));
    format!(
        "#!/bin/sh\n{wait}if [ -e {} ]; then echo 'forced failure' >&2; exit 1; fi\nexec {} \"$@\"\n",
        quote(flag),
        quote(real)
    )
}

#[cfg(windows)]
fn script(real: &Path, flag: &Path, delay: f64) -> String {
    // `cmd.exe` expands `%` inside a batch file and `"` would close the quoting: refuse a path the
    // wrapper cannot carry instead of writing one that forwards to the wrong place.
    for path in [real, flag] {
        let text = path.display().to_string();
        assert!(
            !text.contains(['%', '"', '\r', '\n']),
            "the wrapper cannot carry the path {text:?}"
        );
    }
    // `timeout` refuses redirected input and `ping` has a one second grain: PowerShell waits in
    // milliseconds, and only the tests that ask for a delay pay for starting it.
    let wait = if delay > 0.0 {
        format!(
            "powershell -NoProfile -NonInteractive -Command \"Start-Sleep -Milliseconds {}\"\r\n",
            (delay * 1000.0).round() as u64
        )
    } else {
        String::new()
    };
    [
        "@echo off".to_string(),
        wait.trim_end().to_string(),
        format!(
            "if exist \"{}\" (echo forced failure 1>&2 & exit /b 1)",
            flag.display()
        ),
        format!("\"{}\" %*", real.display()),
        "exit /b %ERRORLEVEL%".to_string(),
    ]
    .into_iter()
    .filter(|line| !line.is_empty())
    .map(|line| line + "\r\n")
    .collect()
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Nothing to do: Windows decides by extension, not by a mode bit.
#[cfg(windows)]
fn make_executable(_path: &Path) {}
