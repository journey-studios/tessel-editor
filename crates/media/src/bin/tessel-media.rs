//! JSON Lines with the media operations and no renderer: import, relink, packages,
//! interchange and job control run without Node, Chrome or a window.
//! (`export.start` needs a renderer: use `tessel-render` for it.)
//!
//! ```text
//! ./scripts/media [--empty] < requests.jsonl
//! ```
use std::io::{self, BufWriter};

use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{EditorState, headless};
use tessel_editor_media::host::MediaHost;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg != "--empty") {
        eprintln!("Usage: tessel-media [--empty]  (JSON Lines on stdin)");
        std::process::exit(2);
    }
    let mut state = if args.is_empty() {
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
    let mut media = MediaHost::new();
    let mut output = BufWriter::new(io::stdout().lock());
    match headless::run_session_ext(
        &mut state,
        None,
        Some(&mut media),
        io::stdin().lock(),
        &mut output,
    ) {
        Ok(false) => {}
        Ok(true) => std::process::exit(1),
        Err(error) => {
            eprintln!("I/O failed: {error}");
            std::process::exit(1);
        }
    }
}
