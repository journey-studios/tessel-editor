//! JSON Lines operations with a real renderer: `tessel-headless` plus `render.frame`.
//!
//! ```text
//! ./scripts/render [--demo] [--gl swiftshader|gpu] < requests.jsonl
//! ```
//! Starts from an empty project (`--demo` keeps the demonstration bootstrap). The Node worker
//! and Chrome start on the first request that needs a frame, so planning and editing work
//! even when the runtime is not installed; `render.frame` then answers `backend_unavailable`
//! with the setup command.
use std::io::{self, BufWriter};
use std::sync::Arc;

use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{EditorState, headless};
use tessel_editor_media::host::MediaHost;
use tessel_editor_render::{RuntimeConfig, session};

const USAGE: &str = "Usage: tessel-render [--demo] [--gl swiftshader|gpu]  (JSON Lines on stdin)";

fn main() {
    let mut args = std::env::args().skip(1);
    let mut demo = false;
    let mut gl = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--demo" => demo = true,
            "--gl" => gl = args.next(),
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
        Ok(false) => {}
        Ok(true) => std::process::exit(1),
        Err(error) => {
            eprintln!("I/O failed: {error}");
            std::process::exit(1);
        }
    }
}
