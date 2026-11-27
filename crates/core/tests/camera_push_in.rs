use serde_json::{Value, json};
use tessel_editor_core::fixtures::{proof_requests, write_standard_set};
use tessel_editor_core::state::camera_push_in::PushInOptions;
use tessel_editor_core::{Command, EditorState, headless};

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn scene(tag: &str) -> EditorState {
    let dir = std::env::temp_dir().join(format!("tessel-push-in-{tag}-{}", std::process::id()));
    let fixtures = write_standard_set(&dir).unwrap();
    let mut state = EditorState::demo();
    for request in proof_requests(&fixtures) {
        run(&mut state, request);
    }
    run(&mut state, json!({"op":"sequence.length","frames":180}));
    run(
        &mut state,
        json!({
            "op":"clip.add", "id":"splat", "track":"scene-track", "asset":"cloud",
            "start":72, "duration":72, "source_start":2
        }),
    );
    state
}

fn camera(state: &EditorState, frame: i64) -> tessel_editor_core::content::Camera {
    state.clip_camera_at(state.timeline.clip("splat").unwrap(), frame)
}

#[test]
fn motion_uses_the_orbit_and_target_at_each_frame_and_is_scale_independent() {
    let mut state = scene("orbit");
    for scale in [0.002, 2400.0] {
        run(
            &mut state,
            json!({
                "op":"clip.camera", "clip":"splat", "position":[5.0*scale,4.0*scale,-6.0*scale],
                "target":[4.0*scale,2.0*scale,-scale], "fov":40, "orbit":24
            }),
        );
        let baseline: Vec<_> = (72..144).map(|frame| camera(&state, frame)).collect();
        let result = state
            .execute(Command::SetCameraPushIn {
                clip: "splat".into(),
                options: Some(PushInOptions {
                    amount: Some(0.35),
                    frames: Some(21),
                }),
            })
            .unwrap();
        assert_eq!(result["range"], json!([123, 144]));
        for (frame, base) in (72..144).zip(baseline) {
            let moved = camera(&state, frame);
            if frame <= 123 {
                assert_eq!(moved, base);
            } else {
                let progress = (frame - 123) as f64 / 20.0;
                let eased = tessel_editor_core::camera_keys::Easing::EaseInOutCubic.apply(progress);
                for axis in 0..3 {
                    let remaining = (moved.position[axis] - base.target[axis])
                        / (base.position[axis] - base.target[axis]);
                    assert!((remaining - (1.0 - 0.35 * eased)).abs() < 1e-10);
                }
                assert_eq!(moved.target, base.target);
                assert_eq!(moved.fov, base.fov);
            }
        }
        run(
            &mut state,
            json!({"op":"clip.camera.push_in.clear","clip":"splat"}),
        );
    }
}

#[test]
fn preserves_keys_with_retime_persists_and_undoes_as_one_edit() {
    let mut state = scene("keyed");
    run(
        &mut state,
        json!({"op":"clip.property","clip":"splat","property":"speed","value":200}),
    );
    for (time, z) in [(2, 8), (8, 15)] {
        run(
            &mut state,
            json!({
                "op":"clip.camera_key", "clip":"splat", "time":time, "position":[0,1,z],
                "target":[0,0,0], "easing":"easeInOutCubic"
            }),
        );
    }
    let before = state.to_document(None);
    let base_end = camera(&state, 143);
    let result = run(
        &mut state,
        json!({"op":"clip.camera.push_in","clip":"splat"}),
    );
    assert_eq!(result["range"], json!([114, 144]));
    let expected_end = base_end.position[2] * 0.8;
    assert!((camera(&state, 143).position[2] - expected_end).abs() < 1e-10);
    assert_eq!(
        result["value"]["from"],
        json!({"num":11,"den":2,"seconds":5.5})
    );
    let doc = state.to_document(None);
    assert_eq!(doc["schema_version"], 3);
    let reopened = EditorState::from_document(&doc, None).unwrap();
    assert_eq!(
        reopened.camera_push_in_preview_range("splat").unwrap(),
        [114, 144]
    );
    assert_eq!(camera(&reopened, 143), camera(&state, 143));
    let mut wrong_schema = doc.clone();
    wrong_schema["schema_version"] = json!(2);
    assert!(EditorState::from_document(&wrong_schema, None).is_err());
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(state.to_document(None), before);
    assert_eq!(camera(&state, 143), base_end);
    run(&mut state, json!({"op":"history.redo"}));
    assert_eq!(state.to_document(None), doc);
}

#[test]
fn rejected_edits_are_atomic_and_replacing_reports_the_whole_changed_range() {
    let mut state = scene("replace");
    run(
        &mut state,
        json!({"op":"clip.camera.push_in","clip":"splat","frames":48}),
    );
    let doc = state.to_document(None);
    for invalid in [
        json!({"op":"clip.camera.push_in","clip":"splat","amount":1}),
        json!({"op":"clip.camera.push_in","clip":"splat","frames":73}),
        json!({"op":"clip.camera.push_in","clip":"splat","frames":1}),
        json!({"op":"clip.camera.push_in","clip":"c-img"}),
    ] {
        assert!(
            headless::execute(&mut state, &invalid).is_err(),
            "{invalid}"
        );
        assert_eq!(state.to_document(None), doc);
    }
    let next = run(
        &mut state,
        json!({"op":"clip.camera.push_in","clip":"splat","frames":10}),
    );
    assert_eq!(next["range"], json!([96, 144]));
    assert_eq!(
        state.camera_push_in_preview_range("splat").unwrap(),
        [134, 144]
    );
    assert_eq!(
        run(
            &mut state,
            json!({"op":"clip.camera.push_in.clear","clip":"splat"})
        )["range"],
        json!([134, 144])
    );
    assert!(state.camera_push_in_preview_range("splat").is_err());
}
