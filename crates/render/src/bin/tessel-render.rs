//! JSON Lines operations with a real renderer: `tessel-headless` plus `render.frame`.
//!
//! ```text
//! ./scripts/render [--demo] [--gl swiftshader|gpu] < requests.jsonl
//! ```
//! Starts from an empty project (`--demo` keeps the demonstration bootstrap). The Node worker
//! and Chrome start on the first request that needs a frame, so planning and editing work
//! even when the runtime is not installed; `render.frame` then answers `backend_unavailable`
//! with the setup command. `scene.render` and `scene.export` (a directed scene as an MP4, F042)
//! go through `SceneHost`; without the runtime `scene.export` answers `backend_unavailable`.
use std::error::Error;
use std::io::{self, BufWriter};
use std::process::{Command, ExitCode, Stdio};
use std::sync::Arc;

use serde_json::{Value, json};
use tessel_editor_core::backend::BackendError;
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{EditorState, headless};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::tools::Tools;
use tessel_editor_render::{RuntimeConfig, session};

const USAGE: &str =
    "Usage: tessel-render [--demo] [--gl swiftshader|gpu] [--check]  (JSON Lines on stdin)";

/// Uses the same tool discovery and Chrome handshake as a real export.
fn check_backend(gl: Option<String>) -> Result<Value, Box<dyn Error>> {
    let tools = Tools::discover()?;
    let probe = Command::new(&tools.ffprobe)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()?;
    if !probe.success() {
        return Err(BackendError::new(
            "tool_unavailable",
            format!(
                "ffprobe at `{}` exited with {probe}",
                tools.ffprobe.display()
            ),
        )
        .into());
    }

    let mut config = RuntimeConfig::discover().map_err(BackendError::from)?;
    config.gl = gl.or(config.gl);
    let mut renderer = session(&config);
    let capabilities = renderer.capabilities().map(|caps| caps.to_json());
    renderer.shutdown();
    Ok(json!({
        "ok": true,
        "media": tools.to_json(),
        "node": config.node,
        "runtime": config.runtime_dir,
        "renderer": capabilities?,
    }))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(exit) = tessel_editor_core::help::cli(&args, USAGE) {
        return exit;
    }
    let mut args = args.into_iter();
    let mut demo = false;
    let mut check = false;
    let mut gl = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--demo" => demo = true,
            "--check" => check = true,
            "--gl" => {
                gl = args.next();
                if gl.is_none() {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            }
            _ => {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
        }
    }
    if let Some(gl) = &gl
        && gl != "swiftshader"
        && gl != "gpu"
    {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }

    if check {
        return match check_backend(gl) {
            Ok(report) => {
                println!("{report}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("Setup check failed: {error}");
                ExitCode::FAILURE
            }
        };
    }

    let mut state = if demo {
        EditorState::demo()
    } else {
        match EditorState::empty(&ProjectSettings::default()) {
            Ok(state) => state,
            Err(error) => {
                eprintln!("Cannot create the project: {}", error.message);
                std::process::exit(1);
            }
        }
    };
    let mut output = BufWriter::new(io::stdout().lock());
    let result = match RuntimeConfig::discover() {
        Ok(mut config) => {
            config.gl = gl.or(config.gl);
            let mut session = session(&config);
            // WHY a factory: an export drives its own renderer on its own thread, so it never
            // shares GPU state or frame timing with this session's `render.frame`.
            let export_config = config.clone();
            let media = MediaHost::new().with_renderer(Arc::new(move || {
                Ok(Box::new(self::session(&export_config)))
            }));
            let mut scenes =
                tessel_editor_render::scene_host::SceneHost::new(media, config.clone());
            let result = headless::run_session_ext(
                &mut state,
                Some(&mut session),
                Some(&mut scenes),
                io::stdin().lock(),
                &mut output,
            );
            tessel_editor_core::session::Renderer::reset_project(&mut session);
            session.shutdown();
            result
        }
        // WHY: planning and editing do not need the runtime; only rendering reports the setup
        // problem, and it does so per request with the exact fix.
        Err(setup) => {
            eprintln!("Render backend unavailable: {}", setup.message());
            let mut media = MediaHost::new();
            headless::run_session_ext(
                &mut state,
                None,
                Some(&mut media),
                io::stdin().lock(),
                &mut output,
            )
        }
    };
    match result {
        Ok(false) => ExitCode::SUCCESS,
        Ok(true) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("I/O failed: {error}");
            ExitCode::FAILURE
        }
    }
}
