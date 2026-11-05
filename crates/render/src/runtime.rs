//! Locating the runtime and building a session over the Node worker.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tessel_editor_core::backend::BackendError;
use tessel_editor_core::bridge::BridgeBackend;
use tessel_editor_core::session::ViewportSession;

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

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

impl RuntimeConfig {
    /// Reads `TESSEL_NODE`, `TESSEL_RUNTIME_DIR` and `TESSEL_GL`, then falls back to `node` on
    /// the PATH and to the `runtime/` directory of this repository.
    pub fn discover() -> Result<Self, SetupError> {
        let node = std::env::var_os("TESSEL_NODE")
            .map(PathBuf::from)
            .filter(|path| path.is_file())
            .or_else(|| find_in_path("node"))
            .ok_or(SetupError::NodeMissing)?;
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
