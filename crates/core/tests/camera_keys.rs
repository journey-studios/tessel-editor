//! BL140: camera keyframes of a scene clip through the public library and the JSONL entry
//! (`headless::execute`, the same call `tessel-headless` makes), without a window or a renderer.
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command as Process, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{proof_requests, write_standard_set};
use tessel_editor_core::time::Ratio;
use tessel_editor_core::{EditorState, headless};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-camkeys-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The proof project with the scene clip's orbit switched off (keys replace the orbit).
fn project(tag: &str) -> EditorState {
    let set = write_standard_set(&scratch(tag)).unwrap();
    let mut state = EditorState::demo();
    for request in proof_requests(&set) {
        headless::execute(&mut state, &request).unwrap();
    }
    run(
        &mut state,
        json!({"op":"clip.camera","clip":"c-scene","position":[0,1.2,5],"target":[0,0,0],"fov":50,"orbit":0}),
    );
    state
}

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn fail(state: &mut EditorState, request: Value) -> String {
    headless::execute(state, &request)
        .expect_err(&format!("{request} must fail"))
        .code
        .to_string()
}

fn key(time: i64, z: f64, fov: f64, easing: &str) -> Value {
    json!({"op":"clip.camera_key","clip":"c-scene","time":time,
        "position":[0,1.0,z],"target":[0,0,0],"fov":fov,"easing":easing})
}

fn camera_at(state: &mut EditorState, frame: i64) -> Value {
    run(state, json!({"op":"plan","frame":frame}))["layers"][0]["camera"].clone()
}

fn z(camera: &Value) -> f64 {
    camera["position"][2].as_f64().unwrap()
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}

#[test]
fn keys_set_by_ops_drive_the_planned_camera_with_hold_and_interpolation() {
    let mut state = project("drive");
    run(&mut state, key(0, 10.0, 60.0, "linear"));
    run(&mut state, key(2, 4.0, 40.0, "linear"));
    // 24 fps: frame 0 = 0 s, frame 24 = 1 s, frame 47 ≈ 1.958 s.
    approx(z(&camera_at(&mut state, 0)), 10.0);
    let middle = camera_at(&mut state, 24);
    approx(z(&middle), 7.0);
    approx(middle["fov"].as_f64().unwrap(), 50.0);
    approx(z(&camera_at(&mut state, 36)), 5.5);
    // After the last key the camera holds it.
    run(
        &mut state,
        json!({"op":"clip.camera_keys","clip":"c-scene","keys":[
        {"time":0,"position":[0,1,10],"target":[0,0,0],"fov":60},
        {"time":1,"position":[0,1,4],"target":[0,0,0],"fov":40}]}),
    );
    approx(z(&camera_at(&mut state, 47)), 4.0);
}

#[test]
fn easing_shapes_the_segment_like_the_directed_evaluator() {
    let mut state = project("easing");
    run(&mut state, key(0, 10.0, 50.0, "easeOutCubic"));
    run(&mut state, key(2, 2.0, 50.0, "linear"));
    let eased = 1.0 - 0.5_f64.powi(3);
    approx(z(&camera_at(&mut state, 24)), 10.0 + (2.0 - 10.0) * eased);
    approx(
        tessel_editor_core::directed_scene::apply_easing(0.5, "easeOutCubic").unwrap(),
        eased,
    );
}

#[test]
fn trim_and_move_keep_each_key_on_its_picture() {
    let mut state = project("trim");
    run(&mut state, key(0, 10.0, 50.0, "linear"));
    run(&mut state, key(2, 4.0, 50.0, "linear"));
    // Trim the start to frame 12 (0.5 s): the first frame now shows scene time 0.5 s.
    run(
        &mut state,
        json!({"op":"set","target":"trim:c-scene:start","field":"frame","value":12}),
    );
    let clip = state.timeline.clip("c-scene").unwrap().clone();
    assert_eq!(clip.start, 12);
    approx(z(&camera_at(&mut state, 12)), 8.5);
    approx(z(&camera_at(&mut state, 36)), 5.5);
    // Move the clip: the same moment of the scene keeps the same camera.
    run(
        &mut state,
        json!({"op":"move","target":"clip:c-scene","args":{"frame":0}}),
    );
    assert_eq!(state.timeline.clip("c-scene").unwrap().start, 0);
    approx(z(&camera_at(&mut state, 0)), 8.5);
    approx(z(&camera_at(&mut state, 24)), 5.5);
    // Trim the end: later keys are simply not reached.
    run(
        &mut state,
        json!({"op":"set","target":"trim:c-scene:end","field":"frame","value":24}),
    );
    assert_eq!(state.timeline.clip("c-scene").unwrap().duration, 24);
    assert_eq!(
        state.timeline.clip("c-scene").unwrap().camera_keys.len(),
        2,
        "trimming never deletes keys"
    );
}

#[test]
fn clips_without_keys_keep_the_orbit_behaviour() {
    let set = write_standard_set(&scratch("legacy")).unwrap();
    let mut state = EditorState::demo();
    for request in proof_requests(&set) {
        headless::execute(&mut state, &request).unwrap();
    }
    let first = camera_at(&mut state, 0)["position"].clone();
    let later = camera_at(&mut state, 24)["position"].clone();
    assert_eq!(first, json!([0.0, 1.2, 5.0]));
    assert_ne!(first, later, "the orbit still moves a clip without keys");
}

#[test]
fn undo_and_redo_restore_the_keys_and_revision_moves() {
    let mut state = project("undo");
    let before = state.project.revision;
    run(&mut state, key(0, 10.0, 50.0, "linear"));
    run(&mut state, key(2, 4.0, 50.0, "linear"));
    assert!(state.project.revision > before);
    assert_eq!(state.timeline.clip("c-scene").unwrap().camera_keys.len(), 2);
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(state.timeline.clip("c-scene").unwrap().camera_keys.len(), 1);
    run(&mut state, json!({"op":"history.undo"}));
    assert!(
        state
            .timeline
            .clip("c-scene")
            .unwrap()
            .camera_keys
            .is_empty()
    );
    run(&mut state, json!({"op":"history.redo"}));
    run(&mut state, json!({"op":"history.redo"}));
    assert_eq!(state.timeline.clip("c-scene").unwrap().camera_keys.len(), 2);
    // The edit appears in the undo list under its own name.
    let status = run(&mut state, json!({"op":"history.status"}));
    assert!(status.to_string().contains("Edit camera keys"), "{status}");
}

#[test]
fn rejected_edits_leave_the_state_untouched() {
    let mut state = project("reject");
    run(&mut state, key(0, 10.0, 50.0, "linear"));
    run(&mut state, key(2, 4.0, 50.0, "linear"));
    let keep = state.to_document(None);
    let revision = state.project.revision;
    let undo_depth = run(&mut state, json!({"op":"history.status"}));

    let cases = [
        // A straight path through the target, between two individually valid keys.
        (key(4, -10.0, 50.0, "linear"), "invalid_value"),
        // FOV, easing, time and position outside their domain.
        (key(3, 4.0, 0.2, "linear"), "invalid_value"),
        (key(3, 4.0, 50.0, "bounce"), "invalid_value"),
        (
            json!({"op":"clip.camera_key","clip":"c-scene","time":-1,"position":[0,1,5]}),
            "invalid_value",
        ),
        (
            json!({"op":"clip.camera_key","clip":"c-scene","time":100000,"position":[0,1,5]}),
            "invalid_value",
        ),
        (
            json!({"op":"clip.camera_key","clip":"c-scene","position":[0,1,5]}),
            "invalid_value",
        ),
        (
            json!({"op":"clip.camera_key","clip":"c-scene","time":3,"position":[0,5,0]}),
            "invalid_value",
        ),
        // Wrong clip kind and unknown clip.
        (
            json!({"op":"clip.camera_key","clip":"c-img","time":1,"position":[0,1,5]}),
            "not_applicable",
        ),
        (
            json!({"op":"clip.camera_key","clip":"nope","time":1,"position":[0,1,5]}),
            "unknown_element",
        ),
        // No key there to remove.
        (
            json!({"op":"clip.camera_key.remove","clip":"c-scene","time":1}),
            "unknown_element",
        ),
        // Duplicate times in a replacement.
        (
            json!({"op":"clip.camera_keys","clip":"c-scene","keys":[
                {"time":1,"position":[0,1,5]},{"time":1,"position":[0,1,6]}]}),
            "invalid_value",
        ),
        // Keys replace the orbit: asking for both is refused.
        (
            json!({"op":"clip.camera","clip":"c-scene","position":[0,1,5],"orbit":30}),
            "camera_orbit_conflict",
        ),
        // Capture outside the clip's frames.
        (
            json!({"op":"clip.camera_key.capture","clip":"c-scene","easing":"nope"}),
            "invalid_value",
        ),
    ];
    for (request, code) in cases {
        assert_eq!(fail(&mut state, request.clone()), code, "{request}");
    }
    assert_eq!(state.to_document(None), keep);
    assert_eq!(state.project.revision, revision);
    assert_eq!(
        run(&mut state, json!({"op":"history.status"})),
        undo_depth,
        "a rejected edit adds no undo entry"
    );
}

#[test]
fn keying_an_orbiting_clip_is_refused_until_the_orbit_is_cleared() {
    let set = write_standard_set(&scratch("orbit")).unwrap();
    let mut state = EditorState::demo();
    for request in proof_requests(&set) {
        headless::execute(&mut state, &request).unwrap();
    }
    assert_eq!(
        fail(&mut state, key(0, 10.0, 50.0, "linear")),
        "camera_orbit_conflict"
    );
    assert!(
        state
            .timeline
            .clip("c-scene")
            .unwrap()
            .camera_keys
            .is_empty()
    );
}

#[test]
fn capture_records_the_camera_shown_at_the_playhead() {
    let mut state = project("capture");
    run(&mut state, key(0, 10.0, 60.0, "linear"));
    run(&mut state, key(2, 4.0, 40.0, "linear"));
    run(&mut state, json!({"op":"seek","frame":24}));
    let result = run(
        &mut state,
        json!({"op":"clip.camera_key.capture","clip":"c-scene","easing":"easeInOutCubic"}),
    );
    assert_eq!(result["count"], 3);
    let keys = &state.timeline.clip("c-scene").unwrap().camera_keys;
    approx(keys[1].time.to_f64(), 1.0);
    approx(keys[1].position[2], 7.0);
    approx(keys[1].fov, 50.0);
    assert_eq!(keys[1].easing.label(), "easeInOutCubic");
    // The picture at the playhead did not change: the new key sits on the old curve.
    approx(z(&camera_at(&mut state, 24)), 7.0);
    // Capturing twice at the same moment replaces the key instead of adding one.
    let again = run(
        &mut state,
        json!({"op":"clip.camera_key.capture","clip":"c-scene"}),
    );
    assert_eq!(again["count"], 3);
    // Outside the clip there is nothing to capture.
    run(
        &mut state,
        json!({"op":"set","target":"trim:c-scene:start","field":"frame","value":30}),
    );
    run(&mut state, json!({"op":"seek","frame":3}));
    assert_eq!(
        fail(
            &mut state,
            json!({"op":"clip.camera_key.capture","clip":"c-scene"})
        ),
        "invalid_time"
    );
}

#[test]
fn target_style_edits_share_the_same_rules() {
    let mut state = project("targets");
    run(&mut state, key(0, 10.0, 50.0, "linear"));
    run(&mut state, key(2, 4.0, 50.0, "linear"));
    run(
        &mut state,
        json!({"op":"set","target":"camera_key:c-scene:1","field":"fov","value":35}),
    );
    run(
        &mut state,
        json!({"op":"set","target":"camera_key:c-scene:1","field":"easing","value":"easeOutCubic"}),
    );
    run(
        &mut state,
        json!({"op":"set","target":"camera_key:c-scene:0","field":"position_z","value":12}),
    );
    run(
        &mut state,
        json!({"op":"set","target":"camera_key:c-scene:1","field":"time","value":3.5}),
    );
    let keys = state.timeline.clip("c-scene").unwrap().camera_keys.clone();
    approx(keys[1].fov, 35.0);
    approx(keys[0].position[2], 12.0);
    approx(keys[1].time.to_f64(), 3.5);
    // A time that collides with another key, an out-of-range index and an unknown field fail.
    assert_eq!(
        fail(
            &mut state,
            json!({"op":"set","target":"camera_key:c-scene:1","field":"time","value":0})
        ),
        "invalid_value"
    );
    assert_eq!(
        fail(
            &mut state,
            json!({"op":"set","target":"camera_key:c-scene:9","field":"fov","value":35})
        ),
        "unknown_element"
    );
    assert_eq!(
        fail(
            &mut state,
            json!({"op":"set","target":"camera_key:c-scene:0","field":"colour","value":1})
        ),
        "unknown_field"
    );
    assert_eq!(state.timeline.clip("c-scene").unwrap().camera_keys.len(), 2);
    run(
        &mut state,
        json!({"op":"invoke","target":"camera_key:c-scene:0","action":"remove"}),
    );
    assert_eq!(state.timeline.clip("c-scene").unwrap().camera_keys.len(), 1);
}

#[test]
fn keys_survive_save_and_reopen_and_old_documents_still_open() {
    let mut state = project("persist");
    let plain = state.to_document(None);
    assert_eq!(plain["schema_version"], 1, "no keys: the oldest schema");
    run(&mut state, key(0, 10.0, 60.0, "easeInOutCubic"));
    run(&mut state, key(2, 4.0, 40.0, "linear"));
    let document = state.to_document(None);
    assert_eq!(document["schema_version"], 2, "keys protect legacy readers");
    let mut reopened = EditorState::from_document(&document, None).unwrap();
    assert_eq!(
        reopened.timeline.clip("c-scene").unwrap().camera_keys,
        state.timeline.clip("c-scene").unwrap().camera_keys
    );
    for frame in [0, 7, 24, 40, 48] {
        assert_eq!(
            camera_at(&mut reopened, frame),
            camera_at(&mut state, frame),
            "frame {frame}"
        );
    }
    // An older project (no `camera_keys`) loads with no keys.
    let old = EditorState::from_document(&plain, None).unwrap();
    assert!(old.timeline.clip("c-scene").unwrap().camera_keys.is_empty());
    // A bad document is refused as a whole, never half read.
    let mut broken = document.clone();
    let clips = broken["sequences"][0]["clips"].as_array_mut().unwrap();
    let scene = clips
        .iter_mut()
        .find(|clip| clip["id"] == "c-scene")
        .unwrap();
    scene["camera_keys"][1]["fov"] = json!(900);
    assert!(EditorState::from_document(&broken, None).is_err());
    let mut on_image = document.clone();
    let clips = on_image["sequences"][0]["clips"].as_array_mut().unwrap();
    let image = clips.iter_mut().find(|clip| clip["id"] == "c-img").unwrap();
    image["camera_keys"] = json!([{"time":0,"position":[0,1,5]}]);
    assert!(EditorState::from_document(&on_image, None).is_err());
}

#[test]
fn keys_do_not_cost_the_frames_that_have_no_keyed_camera() {
    // The still camera of a keyless clip is bit-identical to what an older editor planned.
    let mut state = project("still");
    let still = camera_at(&mut state, 24);
    run(&mut state, key(0, 5.0, 50.0, "linear"));
    run(
        &mut state,
        json!({"op":"clip.camera_keys","clip":"c-scene","keys":[]}),
    );
    assert_eq!(camera_at(&mut state, 24), still);
}

#[test]
fn the_headless_process_runs_the_same_edit_flow() {
    let set = write_standard_set(&scratch("process")).unwrap();
    let mut lines: Vec<Value> = proof_requests(&set);
    lines.extend([
        json!({"op":"clip.camera","clip":"c-scene","position":[0,1.2,5],"target":[0,0,0],"fov":50,"orbit":0}),
        key(0, 10.0, 60.0, "linear"),
        key(2, 4.0, 40.0, "linear"),
        // Rejected: the session goes on.
        key(3, -10.0, 40.0, "linear"),
        json!({"op":"plan","frame":24}),
        json!({"op":"history.undo"}),
        json!({"op":"plan","frame":24}),
    ]);
    let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for line in &lines {
        writeln!(stdin, "{line}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "the rejected key fails the session"
    );
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), lines.len());
    let n = replies.len();
    assert_eq!(replies[n - 4]["ok"], false);
    assert_eq!(replies[n - 4]["error"]["code"], "invalid_value");
    let planned = |reply: &Value| z(&reply["result"]["layers"][0]["camera"]);
    approx(planned(&replies[n - 3]), 7.0);
    // The undo removed the second key: the camera holds the first one.
    approx(planned(&replies[n - 1]), 10.0);
}

fn layer(state: &mut EditorState, frame: i64, clip: &str) -> Value {
    run(state, json!({"op":"plan","frame":frame}))["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|layer| layer["clip"] == clip)
        .unwrap_or_else(|| panic!("no layer for `{clip}` at frame {frame}"))
        .clone()
}

fn scene_z(state: &mut EditorState, frame: i64) -> f64 {
    layer(state, frame, "c-scene")["camera"]["position"][2]
        .as_f64()
        .unwrap()
}

fn history(state: &mut EditorState) -> Value {
    run(state, json!({"op":"history.status"}))
}

// ---- BL140.2: trimming the head at a speed other than 100 % -------------------------------

#[test]
fn head_trim_move_and_end_trim_keep_the_camera_and_scene_time_at_every_speed() {
    for speed in [50, 100, 200] {
        let mut state = project(&format!("speed-{speed}"));
        run(
            &mut state,
            json!({"op":"clip.property","clip":"c-scene","property":"speed","value":speed}),
        );
        run(
            &mut state,
            json!({"op":"clip.camera_keys","clip":"c-scene","keys":[
                {"time":0,"position":[0,1,10],"target":[0,0,0],"fov":50},
                {"time":8,"position":[0,1,2],"target":[0,0,0],"fov":50}]}),
        );
        // Independent oracle: z falls 1 unit per second of scene time, and scene time runs at
        // `speed` % of the 24 fps timeline from the clip start.
        let expected =
            |frame_in_clip: i64| 10.0 - frame_in_clip as f64 / 24.0 * speed as f64 / 100.0;
        let frames = [12, 24, 36];
        let mut times = Vec::new();
        for frame in frames {
            approx(scene_z(&mut state, frame), expected(frame));
            times.push(layer(&mut state, frame, "c-scene")["scene_time"].clone());
        }
        run(
            &mut state,
            json!({"op":"set","target":"trim:c-scene:start","field":"frame","value":12}),
        );
        assert_eq!(state.timeline.clip("c-scene").unwrap().start, 12);
        for (frame, time) in frames.iter().zip(&times) {
            assert_eq!(
                &layer(&mut state, *frame, "c-scene")["scene_time"],
                time,
                "speed {speed}%: scene time of frame {frame} moved after the head trim"
            );
            approx(scene_z(&mut state, *frame), expected(*frame));
        }
        // Moving the clip keeps its first picture: new frame 0 is old frame 12.
        run(
            &mut state,
            json!({"op":"move","target":"clip:c-scene","args":{"frame":0}}),
        );
        assert_eq!(layer(&mut state, 0, "c-scene")["scene_time"], times[0]);
        approx(scene_z(&mut state, 0), expected(12));
        // Trimming the end changes no earlier frame.
        run(
            &mut state,
            json!({"op":"set","target":"trim:c-scene:end","field":"frame","value":24}),
        );
        assert_eq!(layer(&mut state, 12, "c-scene")["scene_time"], times[1]);
        approx(scene_z(&mut state, 12), expected(24));
        assert_eq!(state.timeline.clip("c-scene").unwrap().camera_keys.len(), 2);
    }
}

#[test]
fn head_trim_keeps_video_and_audio_on_the_source_time_they_read() {
    for speed in [50, 100, 200] {
        let mut state = project(&format!("av-{speed}"));
        run(
            &mut state,
            json!({"op":"clip.property","clip":"c-vid","property":"speed","value":speed}),
        );
        let frames = [36, 40, 47];
        let before: Vec<Value> = frames
            .iter()
            .map(|frame| layer(&mut state, *frame, "c-vid")["source_time"].clone())
            .collect();
        run(
            &mut state,
            json!({"op":"set","target":"trim:c-vid:start","field":"frame","value":36}),
        );
        assert_eq!(state.timeline.clip("c-vid").unwrap().start, 36);
        for (frame, time) in frames.iter().zip(&before) {
            assert_eq!(
                &layer(&mut state, *frame, "c-vid")["source_time"],
                time,
                "speed {speed}%: video frame {frame} reads a different source time after the trim"
            );
        }
        // 12 frames at 24 fps are 0.5 s of timeline = speed/200 s of source; the unscaled shift
        // (1/2 s) is wrong for any speed but 100 %.
        assert_eq!(
            state.timeline.clip("c-vid").unwrap().source_start,
            Ratio::new(speed, 200).unwrap()
        );
    }
    // The audio clip has no speed control: it keeps reading the same source time too.
    let mut state = project("audio");
    let tone = state.timeline.clip("c-tone").unwrap().clone();
    let before = state.clip_source_time(&tone, 30);
    run(
        &mut state,
        json!({"op":"set","target":"trim:c-tone:start","field":"frame","value":12}),
    );
    let trimmed = state.timeline.clip("c-tone").unwrap().clone();
    assert_eq!(trimmed.start, 12);
    assert_eq!(state.clip_source_time(&trimmed, 30), before);
}

// ---- BL140.3: omitting the easing is not choosing linear ----------------------------------

#[test]
fn upsert_and_capture_keep_the_easing_of_an_existing_key_unless_one_is_given() {
    let mut state = project("easing-keep");
    run(&mut state, key(0, 10.0, 50.0, "easeInOutCubic"));
    run(&mut state, key(2, 4.0, 50.0, "linear"));
    // Frame 12 is a quarter of the segment: easeInOutCubic gives 4 * 0.25^3 = 0.0625, linear 0.25.
    approx(z(&camera_at(&mut state, 12)), 10.0 + (4.0 - 10.0) * 0.0625);

    // Same time, no easing said: new pose, same easing; still two keys.
    let moved = json!({"op":"clip.camera_key","clip":"c-scene","time":0,
        "position":[0,1.0,12],"target":[0,0,0],"fov":50});
    let result = run(&mut state, moved.clone());
    assert_eq!(result["count"], 2);
    let keys = state.timeline.clip("c-scene").unwrap().camera_keys.clone();
    assert_eq!(keys[0].easing.label(), "easeInOutCubic");
    approx(z(&camera_at(&mut state, 12)), 12.0 + (4.0 - 12.0) * 0.0625);
    // Undo brings back the previous pose together with its easing.
    run(&mut state, json!({"op":"history.undo"}));
    let keys = state.timeline.clip("c-scene").unwrap().camera_keys.clone();
    approx(keys[0].position[2], 10.0);
    assert_eq!(keys[0].easing.label(), "easeInOutCubic");

    // An explicit choice is respected, including `linear` over a cubic key.
    let mut explicit = moved.clone();
    explicit["easing"] = json!("linear");
    run(&mut state, explicit);
    let keys = state.timeline.clip("c-scene").unwrap().camera_keys.clone();
    assert_eq!(keys[0].easing.label(), "linear");
    approx(z(&camera_at(&mut state, 12)), 12.0 + (4.0 - 12.0) * 0.25);

    // A new key without an easing is linear.
    run(
        &mut state,
        json!({"op":"clip.camera_key","clip":"c-scene","time":1,"position":[0,1,6]}),
    );
    let keys = state.timeline.clip("c-scene").unwrap().camera_keys.clone();
    assert_eq!(keys.len(), 3);
    assert_eq!(keys[1].easing.label(), "linear");
}

#[test]
fn capture_by_op_and_by_invoke_does_not_reset_the_easing_of_the_key_it_lands_on() {
    let mut state = project("capture-easing");
    run(&mut state, key(0, 10.0, 50.0, "easeOutCubic"));
    run(&mut state, key(2, 4.0, 50.0, "linear"));
    run(&mut state, json!({"op":"seek","frame":0}));
    let depth = history(&mut state);
    // The camera at frame 0 is exactly the key at 0 s: capturing again changes nothing and,
    // above all, does not turn the cubic easing into linear (the UI button and `invoke` send
    // no easing).
    run(
        &mut state,
        json!({"op":"invoke","target":"clip:c-scene","action":"select"}),
    );
    let result = run(
        &mut state,
        json!({"op":"invoke","target":"camera_keys.capture","action":"invoke"}),
    );
    assert_eq!(result["changed"], false, "{result}");
    let result = run(
        &mut state,
        json!({"op":"clip.camera_key.capture","clip":"c-scene"}),
    );
    assert_eq!(result["changed"], false, "{result}");
    let keys = state.timeline.clip("c-scene").unwrap().camera_keys.clone();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].easing.label(), "easeOutCubic");
    assert_eq!(
        history(&mut state),
        depth,
        "a no-op capture adds no undo entry"
    );
    // The curve is still the cubic one: a quarter along gives 1 - 0.75^3.
    approx(
        z(&camera_at(&mut state, 12)),
        10.0 + (4.0 - 10.0) * (1.0 - 0.75_f64.powi(3)),
    );
    // A capture on a new moment is linear unless told otherwise; an explicit easing sticks.
    run(&mut state, json!({"op":"seek","frame":12}));
    run(
        &mut state,
        json!({"op":"invoke","target":"camera_keys.capture","action":"invoke"}),
    );
    let keys = state.timeline.clip("c-scene").unwrap().camera_keys.clone();
    assert_eq!(keys.len(), 3);
    assert_eq!(keys[1].easing.label(), "linear");
    run(
        &mut state,
        json!({"op":"clip.camera_key.capture","clip":"c-scene","easing":"easeInOutCubic"}),
    );
    let keys = state.timeline.clip("c-scene").unwrap().camera_keys.clone();
    assert_eq!(keys.len(), 3);
    assert_eq!(keys[1].easing.label(), "easeInOutCubic");
}

// ---- BL140.4: a reopened project cannot hold what editing refuses -------------------------

#[test]
fn the_loader_refuses_an_orbit_with_keys_like_editing_does_and_keeps_the_state() {
    let mut state = project("loader-orbit");
    run(&mut state, key(0, 10.0, 50.0, "linear"));
    run(&mut state, key(2, 4.0, 50.0, "linear"));
    let edit = fail(
        &mut state,
        json!({"op":"clip.camera","clip":"c-scene","position":[0,1,5],"orbit":30}),
    );
    let before = state.to_document(None);

    let mut document = before.clone();
    let clips = document["sequences"][0]["clips"].as_array_mut().unwrap();
    let scene = clips
        .iter_mut()
        .find(|clip| clip["id"] == "c-scene")
        .unwrap();
    scene["camera"]["orbit"] = json!(30);
    let Err(error) = EditorState::from_document(&document, None) else {
        panic!("orbit + keys must not load");
    };
    assert_eq!(error.code, edit, "the loader and the edit give one code");
    assert_eq!(error.code, "camera_orbit_conflict");
    // The open project was not touched by the failed load.
    assert_eq!(state.to_document(None), before);

    // Keys in a schema 1 document would be dropped by a legacy reader: refused.
    let mut legacy = before.clone();
    legacy["schema_version"] = json!(1);
    assert!(EditorState::from_document(&legacy, None).is_err());
}

// ---- BL140.7: inert edits and silent drops ------------------------------------------------

#[test]
fn clip_camera_on_a_keyed_clip_is_refused_instead_of_a_silent_undo_entry() {
    let mut state = project("inert");
    run(&mut state, key(0, 10.0, 50.0, "linear"));
    run(&mut state, key(2, 4.0, 50.0, "linear"));
    let before = state.to_document(None);
    let depth = history(&mut state);
    assert_eq!(
        fail(
            &mut state,
            json!({"op":"clip.camera","clip":"c-scene","position":[0,2,7],"fov":40,"orbit":0})
        ),
        "camera_keys_override"
    );
    assert_eq!(state.to_document(None), before);
    assert_eq!(history(&mut state), depth);
    // The identical camera is a plain no-op, not an error and not an undo entry.
    let same = run(
        &mut state,
        json!({"op":"clip.camera","clip":"c-scene","position":[0,1.2,5],"target":[0,0,0],"fov":50,"orbit":0}),
    );
    assert_eq!(same["changed"], false);
    assert_eq!(history(&mut state), depth);
    // Without keys the camera edit works again.
    run(
        &mut state,
        json!({"op":"clip.camera_keys","clip":"c-scene","keys":[]}),
    );
    let changed = run(
        &mut state,
        json!({"op":"clip.camera","clip":"c-scene","position":[0,2,7],"fov":40,"orbit":0}),
    );
    assert_eq!(changed["changed"], true);
}

#[test]
fn a_key_with_an_orbit_is_refused_instead_of_losing_the_field() {
    let mut state = project("key-orbit");
    let before = state.to_document(None);
    assert_eq!(
        fail(
            &mut state,
            json!({"op":"clip.camera_key","clip":"c-scene","time":1,"position":[0,1,5],"orbit":30})
        ),
        "camera_orbit_conflict"
    );
    assert_eq!(
        fail(
            &mut state,
            json!({"op":"clip.camera_keys","clip":"c-scene","keys":[
                {"time":1,"position":[0,1,5],"orbit":30}]})
        ),
        "camera_orbit_conflict"
    );
    assert_eq!(state.to_document(None), before);
    // `orbit: 0` says nothing a key does not already mean.
    run(
        &mut state,
        json!({"op":"clip.camera_key","clip":"c-scene","time":1,"position":[0,1,5],"orbit":0}),
    );
    assert_eq!(state.timeline.clip("c-scene").unwrap().camera_keys.len(), 1);
}

#[test]
fn the_fov_range_is_one_up_to_but_excluding_170() {
    let mut state = project("fov");
    assert_eq!(
        fail(&mut state, key(1, 5.0, 170.0, "linear")),
        "invalid_value"
    );
    run(&mut state, key(1, 5.0, 169.9, "linear"));
    run(&mut state, key(2, 5.0, 1.0, "linear"));
    assert_eq!(
        fail(&mut state, key(3, 5.0, 0.9, "linear")),
        "invalid_value"
    );
}
