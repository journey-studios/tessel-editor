//! Writes the deterministic fixture set and the proof project requests.
//!
//! ```text
//! ./scripts/fixtures <directory>
//! ```
//! Prints a JSON summary; `<directory>/proof.jsonl` holds one operation per line and runs
//! through `./scripts/render`, `./scripts/headless` (plan only) and `./scripts/ui op`.
use std::path::Path;

use serde_json::json;
use tessel_editor_core::fixtures::{proof_requests, write_standard_set};

fn main() {
    let Some(directory) = std::env::args().nth(1) else {
        eprintln!("Usage: tessel-fixtures <directory>");
        std::process::exit(2);
    };
    let directory = std::path::absolute(Path::new(&directory)).unwrap_or_else(|_| directory.into());
    let result = write_standard_set(&directory).and_then(|set| {
        let lines: Vec<String> = proof_requests(&set)
            .iter()
            .map(ToString::to_string)
            .collect();
        let proof = directory.join("proof.jsonl");
        std::fs::write(&proof, lines.join("\n") + "\n")?;
        Ok((set, proof))
    });
    match result {
        Ok((set, proof)) => println!(
            "{}",
            json!({ "dir": set.dir, "image": set.image, "frames": set.frames_pattern, "tone": set.tone,
                "splat": set.splat, "sharp": set.sharp, "scene": set.scene, "proof": proof })
        ),
        Err(error) => {
            eprintln!("Cannot write fixtures: {error}");
            std::process::exit(1);
        }
    }
}
