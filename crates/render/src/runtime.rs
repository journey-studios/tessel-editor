//! Locating the runtime and building a session over the Node worker.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tessel_editor_core::backend::BackendError;
use tessel_editor_core::bridge::BridgeBackend;
use tessel_editor_core::session::ViewportSession;
use tessel_editor_media::tools::locate::{Host, System, Tool, locate};

use crate::process::ProcessTransport;

/// Chrome plus the first frame can take a while on a cold start.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(45);

/// Setup problems, each with the command that fixes it.
#[derive(Debug, Clone, PartialEq)]
pub enum SetupError {
    NodeMissing,
    RuntimeMissing(PathBuf),
    DependenciesMissing(PathBuf),
    BundleMissing(PathBuf),
}

impl SetupError {
    pub fn message(&self) -> String {
        match self {
            Self::NodeMissing => {
                "Node.js was not found. Install Node 20+ or set TESSEL_NODE to its path.".into()
            }
            Self::RuntimeMissing(path) => format!(
                "The runtime directory `{}` does not exist. Set TESSEL_RUNTIME_DIR to the repository's `runtime/` directory.",
                path.display()
            ),
            Self::DependenciesMissing(path) => format!(
                "The runtime dependencies are not installed in `{}`. Run: pnpm --dir {} install",
                path.display(),
                path.display()
            ),
            Self::BundleMissing(path) => format!(
                "The runtime bundle `{}` is not built. Run: pnpm --dir {} run build",
                path.display(),
                path.parent()
                    .and_then(Path::parent)
                    .unwrap_or(path)
                    .display()
            ),
        }
    }
}

impl From<SetupError> for BackendError {
    fn from(error: SetupError) -> Self {
        Self::new("backend_unavailable", error.message())
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub node: PathBuf,
    pub runtime_dir: PathBuf,
    /// `swiftshader` (deterministic, default) or `gpu`; passed to the worker as `TESSEL_GL`.
    pub gl: Option<String>,
}

/// `TESSEL_NODE` when it is a file, else `node` (`node.exe` on Windows) along the `PATH`, else
/// the usual install places. The rules live in the media adapter, shared with FFmpeg.
fn find_node(host: &dyn Host) -> Option<PathBuf> {
    locate(host, Tool::Node).map(|found| found.path)
}

impl RuntimeConfig {
    /// Reads `TESSEL_NODE`, `TESSEL_RUNTIME_DIR` and `TESSEL_GL`, then falls back to `node` on
    /// the PATH (and, on Windows, to Program Files / WinGet) and to the `runtime/` directory of
    /// this repository.
    pub fn discover() -> Result<Self, SetupError> {
        let node = find_node(&System).ok_or(SetupError::NodeMissing)?;
        let runtime_dir = std::env::var_os("TESSEL_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime"));
        let config = Self {
            node,
            runtime_dir,
            gl: std::env::var("TESSEL_GL").ok(),
        };
        config.check()?;
        Ok(config)
    }

    /// Directory with the built page (`index.html`, `runtime.js`) both hosts serve.
    pub fn dist_dir(&self) -> PathBuf {
        self.runtime_dir.join("dist")
    }

    pub fn worker_script(&self) -> PathBuf {
        self.runtime_dir.join("worker/worker.mjs")
    }

    /// Verifies the pieces exist before spawning, so the error names the missing one.
    pub fn check(&self) -> Result<(), SetupError> {
        if !self.worker_script().is_file() {
            return Err(SetupError::RuntimeMissing(self.runtime_dir.clone()));
        }
        if !self
            .runtime_dir
            .join("node_modules/puppeteer-core")
            .is_dir()
        {
            return Err(SetupError::DependenciesMissing(self.runtime_dir.clone()));
        }
        let bundle = self.runtime_dir.join("dist/runtime.js");
        if !bundle.is_file() {
            return Err(SetupError::BundleMissing(bundle));
        }
        Ok(())
    }
}

/// Where the bundled demonstration images live (resolved against this repository).
pub fn demo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/demo")
}

/// The built page for a host that loads it itself (the desktop WebView). Needs neither Node
/// nor Chrome: only `runtime/dist` from `pnpm --dir runtime run build`.
pub fn discover_page() -> Result<PathBuf, SetupError> {
    let runtime_dir = std::env::var_os("TESSEL_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime"));
    let dist = runtime_dir.join("dist");
    for file in ["index.html", "runtime.js"] {
        if !dist.join(file).is_file() {
            return Err(SetupError::BundleMissing(dist.join(file)));
        }
    }
    Ok(dist)
}

/// A bridge backend over a lazily started worker. Nothing is spawned until the first message.
pub fn worker_backend(config: &RuntimeConfig) -> BridgeBackend<ProcessTransport> {
    BridgeBackend::new(
        "three-worker",
        ProcessTransport::new(config.clone()),
        HANDSHAKE_TIMEOUT,
    )
}

/// The session the headless CLI and the tests use: demo images resolve against this repository.
pub fn session(config: &RuntimeConfig) -> ViewportSession<BridgeBackend<ProcessTransport>> {
    ViewportSession::new(worker_backend(config)).with_demo_dir(demo_dir())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use tessel_editor_media::tools::locate::Os;

    use super::*;

    /// A machine with a fixed environment and a fixed set of files; the process environment and
    /// the disk are never read, so these tests do not race the others.
    struct Machine {
        os: Os,
        vars: Vec<(&'static str, OsString)>,
        files: Vec<PathBuf>,
    }

    impl Host for Machine {
        fn os(&self) -> Os {
            self.os
        }

        fn var(&self, name: &str) -> Option<OsString> {
            self.vars
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone())
        }

        fn is_file(&self, path: &Path) -> bool {
            self.files.iter().any(|file| file == path)
        }

        fn entries(&self, _dir: &Path) -> Vec<PathBuf> {
            Vec::new()
        }
    }

    fn windows(vars: &[(&'static str, &str)], files: &[PathBuf]) -> Machine {
        Machine {
            os: Os::Windows,
            vars: vars.iter().map(|(k, v)| (*k, OsString::from(v))).collect(),
            files: files.to_vec(),
        }
    }

    fn node_dir() -> PathBuf {
        Path::new("/c/Program Files").join("nodejs")
    }

    fn path_of(dir: &Path) -> OsString {
        std::env::join_paths([dir]).unwrap()
    }

    #[test]
    fn node_exe_is_found_on_a_windows_path_with_spaces() {
        let dir = node_dir();
        let mut machine = windows(&[], &[dir.join("node.exe")]);
        machine.vars.push(("PATH", path_of(&dir)));
        assert_eq!(find_node(&machine), Some(dir.join("node.exe")));
    }

    #[test]
    fn a_valid_node_override_wins_and_an_invalid_one_falls_back_to_the_path() {
        let dir = node_dir();
        let custom = PathBuf::from("/d/node-22/node.exe");
        let files = [dir.join("node.exe"), custom.clone()];
        let mut valid = windows(&[("TESSEL_NODE", "/d/node-22/node.exe")], &files);
        valid.vars.push(("PATH", path_of(&dir)));
        assert_eq!(find_node(&valid), Some(custom));

        let mut invalid = windows(&[("TESSEL_NODE", "/d/gone/node.exe")], &files);
        invalid.vars.push(("PATH", path_of(&dir)));
        assert_eq!(find_node(&invalid), Some(dir.join("node.exe")));
    }

    #[test]
    fn nothing_found_is_node_missing() {
        // The directory rule (a directory is not a tool) is tested where it lives, in
        // `tessel_editor_media::tools::locate`.
        let dir = node_dir();
        let mut machine = windows(&[], &[]);
        machine.vars.push(("PATH", path_of(&dir)));
        assert_eq!(find_node(&machine), None);
        assert_eq!(
            SetupError::NodeMissing.message(),
            "Node.js was not found. Install Node 20+ or set TESSEL_NODE to its path."
        );
    }

    #[test]
    fn node_is_found_in_program_files_when_the_path_is_stale() {
        let node = node_dir().join("node.exe");
        let machine = windows(
            &[("ProgramFiles", "/c/Program Files")],
            std::slice::from_ref(&node),
        );
        assert_eq!(find_node(&machine), Some(node));
    }

    #[test]
    fn outside_windows_the_bare_name_is_used_as_before() {
        let dir = Path::new("/usr/bin");
        let machine = Machine {
            os: Os::Other,
            vars: vec![("PATH", path_of(dir))],
            files: vec![dir.join("node")],
        };
        assert_eq!(find_node(&machine), Some(dir.join("node")));
    }
}
