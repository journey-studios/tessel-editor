//! P08: importing sequences between projects with colliding IDs.

mod common;

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{StandardSet, proof_requests, write_standard_set};
use tessel_editor_core::interchange::apply_import;
use tessel_editor_core::jobs::{JobKind, JobRegistry};
use tessel_editor_core::plan::{PlanRequest, PlanTime, PreviewIntent};
use tessel_editor_core::{EditorState, headless, persist};
use tessel_editor_media::interchange::{PreparedProjectImport, load_source, prepare};
use tessel_editor_media::package::{self, PackageFormat, PackageOptions};

use common::temp;

fn project(set: &StandardSet, document: &Path) -> EditorState {
    let mut state = EditorState::demo();
    for request in proof_requests(set) {
        headless::execute(&mut state, &request).unwrap();
    }
    persist::save(&mut state, document).unwrap();
    state
}

fn prepare_job(
    destination: &EditorState,
    source: &Path,
    workspace: &Path,
    sequences: &[String],
) -> Result<(PreparedProjectImport, ()), tessel_editor_core::jobs::JobError> {
    let registry = JobRegistry::new(1);
    let mut prepared = None;
    let (_, outcome) = registry.run_inline(JobKind::ProjectImport, "import", |ctx| {
        let loaded = load_source(source, workspace, ctx)?;
        let result = prepare(destination, loaded.state.clone(), workspace, sequences, ctx);
        loaded.cleanup();
        prepared = Some(result?);
        Ok(json!(null))
    });
    outcome?;
    Ok((prepared.unwrap(), ()))
}

fn plan_json(state: &EditorState, sequence: &str, frame: i64) -> Value {
    let mut state = state.clone();
    headless::execute(
        &mut state,
        &json!({"op":"sequence.activate","sequence":sequence}),
    )
    .unwrap();
    state
        .plan(&PlanRequest {
            intent: PreviewIntent::Sequence,
            at: PlanTime::Frame(frame),
        })
        .unwrap()
        .to_json()
}

#[test]
fn importing_a_sequence_remaps_every_colliding_id_and_leaves_the_source_alone() {
    let dir = temp("interchange");
    let set = write_standard_set(&dir.join("fx")).unwrap();
    let source_doc = dir.join("source/source.tessel.json");
    let source = project(&set, &source_doc);
    let source_bytes = fs::read(&source_doc).unwrap();
    let destination_doc = dir.join("dest/dest.tessel.json");
    let mut destination = project(&set, &destination_doc);
    let workspace = dir.join("dest");
    let before = destination.to_document(Some(&workspace));
    let undo_depth = destination.history.to_json()["undo_depth"]
        .as_u64()
        .unwrap();

    let (prepared, ()) = prepare_job(&destination, &source_doc, &workspace, &[]).unwrap();
    let plan = &prepared.plan;
    // Every ID of the source collides with the destination: everything is renamed.
    assert!(plan.assets.iter().all(|(from, to)| from != to), "{plan:?}");
    assert_eq!(plan.sequences, [("main".to_string(), "main-2".to_string())]);
    assert!(plan.clips.iter().all(|(from, to)| from != to));

    let report = apply_import(
        &mut destination,
        &prepared.source,
        &prepared.plan,
        &prepared.replacements,
    )
    .unwrap();
    assert_eq!(report["changed"], true);
    let ids: Vec<_> = destination
        .all_sequences()
        .map(|t| t.id.as_str().to_string())
        .collect();
    assert_eq!(ids, ["main", "main-2"]);
    // The original sequence of the destination is bit-for-bit what it was.
    assert_eq!(
        destination.to_document(Some(&workspace))["sequences"][0],
        before["sequences"][0]
    );
    // The imported one plays the same: same layers/content kinds at every sampled frame.
    for frame in [0, 11, 23, 40] {
        let strip = |value: Value| {
            value["layers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| (l["kind"].clone(), l["opacity"].clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            strip(plan_json(&destination, "main-2", frame)),
            strip(plan_json(&source, "main", frame)),
            "frame {frame}"
        );
    }
    // The scene file was rewritten to name the renamed splat, not the destination's own.
    let (_, scene_new) = plan.assets.iter().find(|(from, _)| from == "scn").unwrap();
    let (_, splat_new) = plan
        .assets
        .iter()
        .find(|(from, _)| from == "cloud")
        .unwrap();
    let tessel_editor_core::assets::Source::File { path } = &destination
        .library
        .item(scene_new)
        .unwrap()
        .asset
        .as_ref()
        .unwrap()
        .source
    else {
        panic!()
    };
    let scene: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert!(
        scene["objects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["type"] == "splat" && o["asset"] == splat_new.as_str()),
        "{scene}"
    );
    assert!(
        destination
            .library
            .item(scene_new)
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .meta
            .requires
            .iter()
            .any(|id| id == splat_new.as_str())
    );
    // Bytes were copied into the destination workspace, deduplicated with what was there.
    for (_, new) in &plan.assets {
        let asset = destination
            .library
            .item(new)
            .unwrap()
            .asset
            .as_ref()
            .unwrap();
        for file in tessel_editor_core::document::asset_files(asset) {
            assert!(file.starts_with(&workspace), "{}", file.display());
        }
    }
    // Overrides moved with the clips.
    let (_, new_img_clip) = plan.clips.iter().find(|(from, _)| from == "c-img").unwrap();
    assert_eq!(
        destination
            .inspector
            .overrides_snapshot()
            .contains_key(new_img_clip.as_str()),
        source.inspector.overrides_snapshot().contains_key("c-img")
    );

    // One undo step restores the destination exactly; redo brings it back.
    assert_eq!(
        destination.history.to_json()["undo_depth"]
            .as_u64()
            .unwrap(),
        undo_depth + 1
    );
    headless::execute(&mut destination, &json!({"op":"history.undo"})).unwrap();
    assert_eq!(
        destination.to_document(Some(&workspace))["sequences"],
        before["sequences"]
    );
    assert_eq!(
        destination.to_document(Some(&workspace))["library"],
        before["library"]
    );
    headless::execute(&mut destination, &json!({"op":"history.redo"})).unwrap();
    assert_eq!(destination.all_sequences().count(), 2);
    // Importing never wrote to the source.
    assert_eq!(fs::read(&source_doc).unwrap(), source_bytes);
    assert_eq!(source.all_sequences().count(), 1);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn a_package_is_a_valid_source_and_a_missing_file_fails_atomically() {
    let dir = temp("interchange-pack");
    let set = write_standard_set(&dir.join("fx")).unwrap();
    let source = project(&set, &dir.join("source/source.tessel.json"));
    let registry = JobRegistry::new(1);
    let pack = dir.join("source.tesselpack");
    let options = PackageOptions {
        format: PackageFormat::Archive,
        sequences: None,
        allow_partial: false,
        replace: false,
    };
    registry
        .run_inline(JobKind::Package, "package", |ctx| {
            package::create(&source, &pack, &options, ctx)
        })
        .1
        .unwrap();

    let workspace = dir.join("dest");
    let mut destination = project(&set, &workspace.join("dest.tessel.json"));
    let before = destination.to_document(Some(&workspace));
    let (prepared, ()) =
        prepare_job(&destination, &pack, &workspace, &["main".to_string()]).unwrap();
    apply_import(
        &mut destination,
        &prepared.source,
        &prepared.plan,
        &prepared.replacements,
    )
    .unwrap();
    assert_eq!(destination.all_sequences().count(), 2);
    assert!(!workspace.join(".tessel/import-scratch/src-0").exists());
    assert!(
        fs::read_dir(workspace.join(".tessel/import-scratch"))
            .map_or(true, |mut d| d.next().is_none()),
        "scratch extraction is removed"
    );

    // A source whose media is gone: reported with the asset list, destination untouched.
    let broken_doc = dir.join("broken/broken.tessel.json");
    let _broken = project(&set, &broken_doc);
    fs::remove_file(&set.tone).unwrap();
    let mut other = project_without_files(&set, &workspace);
    let state_before = other.to_document(Some(&workspace));
    let error = prepare_job(&other, &broken_doc, &workspace, &[])
        .err()
        .unwrap();
    assert_eq!(error.code, "missing_media");
    assert_eq!(other.to_document(Some(&workspace)), state_before);
    let _ = (&mut other, before);
    fs::remove_dir_all(dir).ok();
}

fn project_without_files(_set: &StandardSet, workspace: &Path) -> EditorState {
    let mut state = EditorState::demo();
    headless::execute(
        &mut state,
        &json!({"op":"project.create","name":"Empty","length":24,"dir":workspace}),
    )
    .unwrap();
    state
}
