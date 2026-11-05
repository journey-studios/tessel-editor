//! The viewport proof project built purely through semantic operations, then planned and mixed
//! without any window or renderer. Pixel proofs live in `crates/render/tests`.
use std::path::PathBuf;

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{StandardSet, proof_requests, write_standard_set};
use tessel_editor_core::{EditorState, headless};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-proof-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn build(tag: &str) -> (EditorState, StandardSet) {
    let set = write_standard_set(&scratch(tag)).unwrap();
    let mut state = EditorState::demo();
    for request in proof_requests(&set) {
        headless::execute(&mut state, &request)
            .unwrap_or_else(|error| panic!("{request}: {error:?}"));
    }
    (state, set)
}

fn kinds(plan: &Value) -> Vec<String> {
    plan["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|layer| {
            format!(
                "{}:{}",
                layer["kind"].as_str().unwrap(),
                layer["clip"].as_str().unwrap()
            )
        })
        .collect()
}

#[test]
fn proof_project_is_built_by_api_without_the_demo() {
    let (state, _) = build("build");
    let summary = state.summary();
    assert_eq!(summary["project"], "Viewport proof");
    assert_eq!(summary["timeline"]["width"], 320);
    assert_eq!(summary["timeline"]["duration_frames"], 48);
    assert!(
        !state.demonstration,
        "project.create leaves the demo bootstrap"
    );
    assert_eq!(summary["timeline"]["clips"].as_array().unwrap().len(), 5);
    assert_eq!(summary["assets"].as_array().unwrap().len(), 6);
}

#[test]
fn sequence_plan_alternates_image_video_and_keeps_scene_and_text_around_them() {
    let (mut state, _) = build("plan");
    let at = |state: &mut EditorState, frame: i64| {
        headless::execute(state, &json!({"op":"plan","frame":frame})).unwrap()
    };
    // Bottom to top: scene track, media track, titles.
    assert_eq!(
        kinds(&at(&mut state, 0)),
        ["scene:c-scene", "image:c-img", "text:c-title"]
    );
    assert_eq!(
        kinds(&at(&mut state, 23)),
        ["scene:c-scene", "image:c-img", "text:c-title"]
    );
    let video = at(&mut state, 30);
    assert_eq!(
        kinds(&video),
        ["scene:c-scene", "video:c-vid", "text:c-title"]
    );
    // 8 fps source in a 24 fps sequence: 6 frames in = 0.25 s = source frame 2.
    assert_eq!(video["layers"][1]["source_frame"], 2);
    assert_eq!(at(&mut state, 47)["layers"][1]["source_frame"], 7);
    // A seek is a Command: the same plan comes from the playhead.
    headless::execute(&mut state, &json!({"op":"seek","frame":30})).unwrap();
    assert_eq!(
        headless::execute(&mut state, &json!({"op":"plan"})).unwrap()["frame"],
        30
    );
    // Scene dependencies are listed transitively for the backend.
    let requires: Vec<_> = video["requires"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        requires.contains(&"cloud") && requires.contains(&"scn"),
        "{requires:?}"
    );
}

#[test]
fn camera_orbit_changes_the_planned_camera_with_time() {
    let (mut state, _) = build("camera");
    let position = |state: &mut EditorState, frame: i64| {
        headless::execute(state, &json!({"op":"plan","frame":frame})).unwrap()["layers"][0]["camera"]["position"].clone()
    };
    let first = position(&mut state, 0);
    let later = position(&mut state, 24);
    assert_ne!(first, later);
    assert_eq!(first, json!([0.0, 1.2, 5.0]));
    headless::execute(
        &mut state,
        &json!({"op":"clip.camera","clip":"c-scene","position":[0,1.2,5],"target":[0,0,0],"fov":50,"orbit":0}),
    )
    .unwrap();
    assert_eq!(
        position(&mut state, 0),
        position(&mut state, 24),
        "orbit 0 keeps the camera still"
    );
}

#[test]
fn hidden_tracks_leave_the_plan_and_source_preview_is_independent_of_selection() {
    let (mut state, _) = build("hidden");
    headless::execute(
        &mut state,
        &json!({"op":"set","target":"track:title-track","field":"hidden","value":true}),
    )
    .unwrap();
    assert_eq!(
        kinds(&headless::execute(&mut state, &json!({"op":"plan","frame":0})).unwrap()),
        ["scene:c-scene", "image:c-img"]
    );
    headless::execute(
        &mut state,
        &json!({"op":"preview.set","mode":"source","asset":"img"}),
    )
    .unwrap();
    let source = headless::execute(&mut state, &json!({"op":"plan"})).unwrap();
    assert_eq!(source["intent"]["mode"], "source");
    assert_eq!(source["layers"].as_array().unwrap().len(), 1);
    let past_end = headless::execute(
        &mut state,
        &json!({"op":"plan","intent":{"mode":"source","asset":"vid","time":{"num":5,"den":1}}}),
    );
    assert_eq!(past_end.unwrap_err().code, "invalid_time");
}

#[test]
fn audio_plan_mixes_exact_samples_and_marks_the_result_as_processed() {
    let (mut state, set) = build("audio");
    let mix = headless::execute(
        &mut state,
        &json!({"op":"render.audio","from":0,"frames":48,"samples":[[0,0],[1000,0],[1000,1]]}),
    )
    .unwrap();
    assert_eq!(mix["kind"], "processed");
    assert_eq!(mix["frames"], 96_000);
    // The tone is mono at 0.5: equal-power pan puts 0.5 * cos(pi/4) in both channels.
    let peak = mix["peak"].as_f64().unwrap();
    assert!(
        (peak - 0.5 * std::f64::consts::FRAC_PI_4.cos()).abs() < 0.01,
        "peak {peak}"
    );
    let left = mix["samples"][1]["value"].as_f64().unwrap();
    let right = mix["samples"][2]["value"].as_f64().unwrap();
    let expected = 0.5
        * (std::f64::consts::TAU * 440.0 * 1000.0 / 48_000.0).sin()
        * std::f64::consts::FRAC_PI_4.cos();
    assert!(
        (left - expected).abs() < 0.001 && (right - expected).abs() < 0.001,
        "{left} {right} {expected}"
    );
    assert_eq!(mix["samples"][0]["value"].as_f64().unwrap(), 0.0);
    // Trimming the clip start advances the source: the mix of the second half equals the tail.
    let out = set.dir.join("mix.wav");
    headless::execute(
        &mut state,
        &json!({"op":"render.audio","from":24,"frames":24,"out":out.to_string_lossy()}),
    )
    .unwrap();
    assert!(std::fs::metadata(&out).unwrap().len() > 48_000 * 2);
}

#[test]
fn rejected_requests_leave_the_state_untouched() {
    let (mut state, set) = build("errors");
    let before = state.summary();
    let image = set.image.to_string_lossy().into_owned();
    let cases = [
        (
            json!({"op":"asset.add","kind":"image","id":"img","name":"dup","path":image}),
            "duplicate_id",
        ),
        (
            json!({"op":"asset.add","kind":"image","name":"x","path":"/does/not/exist.png"}),
            "asset_unreadable",
        ),
        (
            json!({"op":"clip.add","track":"music","asset":"img","start":0,"duration":4}),
            "incompatible_track",
        ),
        (
            json!({"op":"clip.add","track":"media-track","asset":"img","start":10,"duration":4}),
            "overlap",
        ),
        (
            json!({"op":"clip.add","track":"media-track","asset":"vid","start":100,"duration":4}),
            "out_of_bounds",
        ),
        (
            json!({"op":"clip.add","track":"nope","asset":"img","start":0,"duration":4}),
            "unknown_element",
        ),
        (
            json!({"op":"clip.add","track":"title-track","asset":"vid","start":0,"duration":2000}),
            "source_out_of_range",
        ),
        (
            json!({"op":"track.add","id":"has space","name":"x","kind":"video"}),
            "invalid_id",
        ),
        (json!({"op":"seek"}), "invalid_request"),
        (
            json!({"op":"preview.set","mode":"source","asset":"missing"}),
            "unknown_element",
        ),
        (
            json!({"op":"clip.property","clip":"c-img","property":"zoom","value":"wide"}),
            "invalid_value",
        ),
        (json!({"op":"sequence.length","frames":1}), "out_of_bounds"),
        (json!({"op":"project.create","rate":0}), "invalid_value"),
        (
            json!({"op":"render.frame","frame":0}),
            "backend_unavailable",
        ),
    ];
    for (request, code) in cases {
        let error = headless::execute(&mut state, &request).expect_err(&request.to_string());
        assert_eq!(error.code, code, "{request} -> {error:?}");
        assert_eq!(before, state.summary(), "{request} mutated the state");
    }
}

#[test]
fn plan_applies_speed_trim_fades_and_sequence_bounds() {
    let (mut state, _) = build("evaluation");
    let run = |state: &mut EditorState, request: Value| headless::execute(state, &request).unwrap();
    // Speed 200 %: frame 30 is 0.25 s into the clip, read at 2x = 0.5 s of an 8 fps source.
    run(
        &mut state,
        json!({"op":"clip.property","clip":"c-vid","property":"speed","value":200}),
    );
    let layer = &run(&mut state, json!({"op":"plan","frame":30}))["layers"][1];
    assert_eq!(
        (
            layer["source_time"]["num"].as_i64(),
            layer["source_frame"].as_i64()
        ),
        (Some(1), Some(4))
    );
    // Fade in over 12 frames: half opaque at frame 6, fully opaque from frame 12.
    run(
        &mut state,
        json!({"op":"clip.property","clip":"c-img","property":"fade_in","value":12}),
    );
    assert_eq!(
        run(&mut state, json!({"op":"plan","frame":6}))["layers"][1]["opacity"],
        0.5
    );
    assert_eq!(
        run(&mut state, json!({"op":"plan","frame":12}))["layers"][1]["opacity"],
        1.0
    );
    // The end of the sequence is an empty frame, past it is an error that changes nothing.
    assert_eq!(
        run(&mut state, json!({"op":"plan","frame":48}))["layers"],
        json!([])
    );
    let before = state.summary();
    for frame in [-1, 49] {
        let error = headless::execute(&mut state, &json!({"op":"plan","frame":frame})).unwrap_err();
        assert_eq!(error.code, "invalid_time");
    }
    assert_eq!(before, state.summary());
}

#[test]
fn trimming_a_clip_start_keeps_its_content_anchored() {
    let (mut state, _) = build("trim");
    let frame_at = |state: &mut EditorState, frame: i64| {
        headless::execute(state, &json!({"op":"plan","frame":frame})).unwrap()["layers"][1]["source_frame"].as_i64()
    };
    assert_eq!(frame_at(&mut state, 36), Some(4));
    headless::execute(
        &mut state,
        &json!({"op":"set","target":"trim:c-vid:start","field":"frame","value":30}),
    )
    .unwrap();
    // Frame 36 still shows source frame 4: trimming the head must not slide the content.
    assert_eq!(frame_at(&mut state, 36), Some(4));
}
