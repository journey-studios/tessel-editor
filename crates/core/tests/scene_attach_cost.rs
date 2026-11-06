//! BL100a measurement harness (ignored by default; it prints, it does not gate).
//!
//! ```text
//! ./scripts/cargo test -p tessel-editor-core --test scene_attach_cost --locked -- --ignored --nocapture
//! ```
//!
//! For each bundle shape it runs a bounded number of replicas of the two halves of an attach:
//! `prepare` (what a job thread does: read the file, parse, gates, facts) and the apply (what the
//! window's thread does: `Command::Scene(Attach)` of the prepared scene), plus the synchronous
//! attach of the same file as a reference, and prints a fingerprint of everything the attach
//! produced (answer, stored document, stored byte count). It uses the public API only, so the
//! same file measured on the code before and after BL100a must print the same fingerprints.
//!
//! This is CPU time of the core, not window responsiveness: the UI stall is measured on the real
//! window by `scripts/scene-import-proof.py` (`large_bundle.stalls_over_100ms`).

use core::prelude::v1::test;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::{Value, json};
use tessel_editor_core::jobs::CancelToken;
use tessel_editor_core::scene_project::{AdmittedScene, SceneCommand, SceneSource};
use tessel_editor_core::{Command, EditorState, headless};

const REPLICAS: usize = 3;

struct Fnv(u64);

impl Write for Fnv {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn fingerprint(value: &Value) -> String {
    let mut hasher = Fnv(0xcbf2_9ce4_8422_2325);
    serde_json::to_writer(&mut hasher, value).unwrap();
    format!("{:016x}", hasher.0)
}

fn ms(started: Instant) -> f64 {
    (started.elapsed().as_secs_f64() * 1000.0 * 10.0).round() / 10.0
}

fn project() -> EditorState {
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":"cost"})).unwrap();
    state
}

fn base() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

/// 24 MiB of an ignored extension string: the shape the live proof uses.
fn padding() -> Value {
    let mut bundle = base();
    bundle["x-padding"] = json!("0".repeat(24 * 1024 * 1024));
    bundle
}

/// ~30 MB of nodes (90 000, each with an unknown field, so one notice each): the shape where the
/// attach used to scan and rebuild per-node data as well.
fn nodes() -> Value {
    let mut bundle = base();
    let list = bundle["sceneDefinitions"][0]["nodes"]
        .as_array_mut()
        .unwrap();
    for n in 0..90_000 {
        list.push(json!({"id": format!("n{n}"), "parentId": "root", "type": "group",
            "x-note": "n".repeat(100),
            "localPosition": {"x": 0, "y": 0, "z": 0}, "localRotation": {"x": 0, "y": 0, "z": 0, "w": 1},
            "localScale": {"x": 1, "y": 1, "z": 1}}));
    }
    bundle["x-padding"] = json!("0".repeat(6 * 1024 * 1024));
    bundle
}

fn write(name: &str, bundle: &Value) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-attach-cost-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, serde_json::to_vec(bundle).unwrap()).unwrap();
    path
}

fn attach(state: &mut EditorState, source: SceneSource) -> Value {
    state
        .execute(Command::Scene(SceneCommand::Attach {
            id: Some("big".into()),
            name: None,
            source,
        }))
        .unwrap()
}

#[test]
#[ignore = "measurement harness: prints timings and fingerprints"]
fn measure_attach_cost() {
    for (shape, bundle) in [("padding-24MiB", padding()), ("nodes-90k", nodes())] {
        let path = write(&format!("{shape}.json"), &bundle);
        let bytes = fs::metadata(&path).unwrap().len();
        drop(bundle);
        let mut rows = Vec::new();
        for replica in 0..REPLICAS {
            let started = Instant::now();
            let admitted =
                AdmittedScene::prepare(SceneSource::Path(path.clone()), &CancelToken::new())
                    .unwrap();
            let prepare = ms(started);

            let mut state = project();
            let started = Instant::now();
            let report = attach(&mut state, SceneSource::Admitted(admitted));
            let apply = ms(started);

            let mut reference = project();
            let started = Instant::now();
            let sync_report = attach(&mut reference, SceneSource::Path(path.clone()));
            let sync = ms(started);

            let document = state.to_document(None);
            rows.push(json!({
                "shape": shape, "bytes": bytes, "replica": replica,
                "prepare_ms": prepare, "apply_ms": apply, "sync_attach_ms": sync,
                "report": fingerprint(&report), "document": fingerprint(&document["scenes"]),
                "sync_report": fingerprint(&sync_report),
                "sync_document": fingerprint(&reference.to_document(None)["scenes"]),
                "undo_depth": headless::execute(&mut state, &json!({"op":"history.status"})).unwrap()["history"]["undo_depth"],
            }));
        }
        for row in rows {
            println!("COST {row}");
        }
    }
}
