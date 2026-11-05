//! One JSON request per stdin line; state lasts for the process lifetime.
//!
//! `--empty` starts from an empty project instead of the demonstration bootstrap.
//! Rendering needs a backend: this binary has none, so `render.frame` answers
//! `backend_unavailable`; `tessel-render` (Node + Chrome runtime) provides one.
use std::io::{self, BufWriter};

use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{EditorState, headless};

fn run(empty: bool) -> io::Result<bool> {
    let mut state = if empty {
        EditorState::empty(&ProjectSettings::default())
            .map_err(|error| io::Error::other(error.to_string()))?
    } else {
        EditorState::demo()
    };
    let mut output = BufWriter::new(io::stdout().lock());
    headless::run_session(&mut state, None, io::stdin().lock(), &mut output)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg != "--empty") {
        eprintln!("Usage: tessel-headless [--empty]  (JSON Lines on stdin)");
        std::process::exit(2);
    }
    match run(!args.is_empty()) {
        Ok(false) => {}
        Ok(true) => std::process::exit(1),
        Err(error) => {
            eprintln!("Headless I/O failed: {error}");
            std::process::exit(1);
        }
    }
}
