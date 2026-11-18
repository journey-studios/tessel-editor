//! F071: what export owns about captions set not to burn: the `.srt` beside the video, its names,
//! the refusal of a name clash before any frame is rendered, and a snapshot that leaves the
//! project alone. The frames come from the deterministic renderer double (`common::mock`); the
//! pixels of burned and unburned captions are proven with the real runtime in
//! `crates/render/tests/player_frames.rs`.

mod common;

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{proof_requests, write_standard_set};
use tessel_editor_core::jobs::{JobError, JobKind, JobRegistry};
use tessel_editor_core::preset::Preset;
use tessel_editor_core::{EditorState, headless};
use tessel_editor_media::export::{ExportRequest, run};

use common::mock::{Mock, Settings};
use common::{temp, tools};

/// The proof project (2 s at 24 fps) with `tracks` caption clips, none of them burned.
fn project(dir: &Path, tracks: &[&str]) -> EditorState {
    let set = write_standard_set(&dir.join("fx")).unwrap();
    let mut state = EditorState::demo();
    for request in proof_requests(&set) {
        headless::execute(&mut state, &request).unwrap();
    }
    for id in tracks {
        for request in [
            json!({"op":"track.add","id":format!("track-{id}"),"name":*id,"kind":"video"}),
            json!({"op":"clip.add","id":*id,"track":format!("track-{id}"),"start":0,"duration":48,
                "subtitles":{"burn":false,"cues":[
                    {"start_ms":500,"end_ms":1500,"text":format!("Fala {id}")}]}}),
        ] {
            headless::execute(&mut state, &request).unwrap();
        }
    }
    state
}

fn preset() -> Preset {
    Preset::from_json(&json!({ "id": "t320", "name": "t320", "width": 320, "height": 180 }))
        .unwrap()
}

fn export(
    state: &EditorState,
    request: &ExportRequest,
    renderer: &mut Mock,
) -> Result<Value, JobError> {
    let registry = JobRegistry::new(1);
    let tools = tools();
    let (_, outcome) = registry.run_inline(JobKind::VideoExport, "export", |ctx| {
        run(state, request, &tools, renderer, ctx)
    });
    outcome
}

#[test]
fn one_unburned_track_is_written_next_to_the_video_with_the_name_of_the_video() {
    let dir = temp("captions-one");
    let state = project(&dir, &["subs"]);
    let before = state.to_document(None);
    let target = dir.join("out/film.mp4");
    let request = ExportRequest::new(preset(), &target);
    let mut renderer = Mock::new(Settings::default());

    let report = export(&state, &request, &mut renderer).unwrap();

    assert_eq!(report["verification"]["ok"], true, "{report}");
    assert_eq!(
        fs::read_to_string(dir.join("out/film.srt")).unwrap(),
        "1\n00:00:00,500 --> 00:00:01,500\nFala subs\n"
    );
    assert_eq!(report["captions"][0]["cues"], 1);
    assert_eq!(report["captions"][0]["clip"], "subs");
    assert_eq!(
        state.to_document(None),
        before,
        "export works on a snapshot"
    );
    // No staging leftovers beside the outputs.
    let names: Vec<String> = fs::read_dir(dir.join("out"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
}

#[test]
fn several_unburned_tracks_each_get_their_own_file() {
    let dir = temp("captions-many");
    let state = project(&dir, &["one", "two"]);
    let target = dir.join("out/film.mp4");
    let request = ExportRequest::new(preset(), &target);
    let mut renderer = Mock::new(Settings::default());

    let report = export(&state, &request, &mut renderer).unwrap();

    assert_eq!(report["captions"].as_array().unwrap().len(), 2);
    for id in ["one", "two"] {
        let text = fs::read_to_string(dir.join(format!("out/film.{id}.srt"))).unwrap();
        assert_eq!(
            text,
            format!("1\n00:00:00,500 --> 00:00:01,500\nFala {id}\n")
        );
    }
    assert!(
        !dir.join("out/film.srt").exists(),
        "no ambiguous single name"
    );
}

#[test]
fn an_existing_srt_refuses_the_export_before_any_frame_is_rendered() {
    let dir = temp("captions-clash");
    let state = project(&dir, &["subs"]);
    fs::create_dir_all(dir.join("out")).unwrap();
    fs::write(dir.join("out/film.srt"), "mine\n").unwrap();
    let target = dir.join("out/film.mp4");
    let request = ExportRequest::new(preset(), &target);
    let mut renderer = Mock::new(Settings::default());

    let error = export(&state, &request, &mut renderer).unwrap_err();

    assert_eq!(error.code, "destination_exists");
    assert!(
        renderer.seen.lock().unwrap().is_empty(),
        "nothing was rendered"
    );
    assert!(!target.exists());
    assert_eq!(
        fs::read_to_string(dir.join("out/film.srt")).unwrap(),
        "mine\n"
    );

    // Replacing is a choice the request makes.
    let mut replace = ExportRequest::new(preset(), &target);
    replace.replace = true;
    export(&state, &replace, &mut renderer).unwrap();
    assert!(
        fs::read_to_string(dir.join("out/film.srt"))
            .unwrap()
            .contains("Fala subs")
    );
}

#[test]
fn a_range_of_the_export_moves_and_cuts_the_cues_to_the_video_clock() {
    let dir = temp("captions-range");
    let state = project(&dir, &["subs"]);
    // Frames 12..36 are 0.5 s..1.5 s of the sequence: the cue (0.5 s..1.5 s) fills the video.
    let target = dir.join("out/part.mp4");
    let mut request = ExportRequest::new(preset(), &target);
    request.range = Some((12, 36));
    let mut renderer = Mock::new(Settings::default());

    export(&state, &request, &mut renderer).unwrap();
    assert_eq!(
        fs::read_to_string(dir.join("out/part.srt")).unwrap(),
        "1\n00:00:00,000 --> 00:00:01,000\nFala subs\n"
    );

    // Frames 24..48: the cue ends half a second into the video.
    let target = dir.join("out/tail.mp4");
    let mut request = ExportRequest::new(preset(), &target);
    request.range = Some((24, 48));
    export(&state, &request, &mut renderer).unwrap();
    assert_eq!(
        fs::read_to_string(dir.join("out/tail.srt")).unwrap(),
        "1\n00:00:00,000 --> 00:00:00,500\nFala subs\n"
    );
}
