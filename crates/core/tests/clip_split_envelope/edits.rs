use super::*;

#[test]
fn editing_the_fade_of_a_half_hands_the_ramp_to_that_half_and_never_brings_the_old_one_back() {
    let mut state = halves();
    // 80 % opacity, fade-in over the original 12 frames from frame 6: frame 16 is 10/12.
    assert_eq!(opacity_at(&mut state, 16), 0.8 * (10.0 / 12.0));
    set(&mut state, "t-split-1", "fade_in", json!(8));
    assert!(
        window_of(&state, "t-split-1").is_empty(),
        "the edit takes the ramp over"
    );
    // The new ramp starts at the half's own first frame (10): frame 14 is 4/8 of it.
    assert_eq!(opacity_at(&mut state, 14), 0.8 * 0.5);
    // Editing back to the old number is a fresh 12-frame ramp, not the inherited one.
    set(&mut state, "t-split-1", "fade_in", json!(12));
    assert!(window_of(&state, "t-split-1").is_empty());
    assert_eq!(opacity_at(&mut state, 16), 0.8 * (6.0 / 12.0));
    // Undo restores the inherited ramp (the snapshot has the window); so does a second undo.
    run(&mut state, json!({"op":"history.undo"}));
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(window_of(&state, "t-split-1").lead.unwrap().frames, 4);
    assert_eq!(opacity_at(&mut state, 16), 0.8 * (10.0 / 12.0));
}

#[test]
fn zeroing_or_resetting_the_fade_of_a_half_removes_its_inherited_ramp_only() {
    let mut state = halves();
    set(&mut state, "t-split-1", "fade_in", json!(0));
    assert!(window_of(&state, "t-split-1").is_empty());
    assert_eq!(
        opacity_at(&mut state, 10),
        0.8,
        "no ramp on the half any more"
    );
    // The left half keeps its own ramp.
    assert_eq!(opacity_at(&mut state, 8), 0.8 * (2.0 / 12.0));

    let mut state = halves();
    state
        .execute(Command::ResetProperty {
            property: "fade_in".into(),
        })
        .unwrap();
    assert!(window_of(&state, "t-split-1").is_empty());
    assert_eq!(opacity_at(&mut state, 10), 0.8);
    // The fade-out of the right half is untouched by the edit of the fade-in.
    assert_eq!(opacity_at(&mut state, 65), 0.8 * (1.0 / 24.0));
}

#[test]
fn a_slider_gesture_on_a_half_keeps_the_inherited_ramp_until_it_commits() {
    let mut state = halves();
    let begin = |state: &mut EditorState| {
        state
            .execute(Command::PropertyGesture(GestureCommand::Begin {
                clip: Some("t-split-1".into()),
                property: "fade_in".into(),
            }))
            .unwrap();
    };
    let update = |state: &mut EditorState, value: f64| {
        state
            .execute(Command::PropertyGesture(GestureCommand::Update {
                gesture: None,
                value: Raw::Number(value),
            }))
            .unwrap();
    };
    // The preview shows the new value on the half's own edge at once ...
    begin(&mut state);
    update(&mut state, 8.0);
    assert_eq!(opacity_at(&mut state, 14), 0.8 * 0.5);
    // ... a selection change does not end the gesture, nor sweep the window ...
    run(
        &mut state,
        json!({"op":"invoke","target":"clip:t","action":"select"}),
    );
    assert!(!window_of(&state, "t-split-1").is_empty());
    // ... and cancelling puts the inherited ramp back exactly.
    state
        .execute(Command::PropertyGesture(GestureCommand::Cancel {
            gesture: None,
        }))
        .unwrap();
    assert_eq!(opacity_at(&mut state, 16), 0.8 * (10.0 / 12.0));
    assert_eq!(window_of(&state, "t-split-1").lead.unwrap().frames, 4);

    // Committing a different value is an edit: the window is gone and undo brings it back.
    begin(&mut state);
    update(&mut state, 8.0);
    state
        .execute(Command::PropertyGesture(GestureCommand::Commit {
            gesture: None,
        }))
        .unwrap();
    assert!(window_of(&state, "t-split-1").is_empty());
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(window_of(&state, "t-split-1").lead.unwrap().frames, 4);
    assert_eq!(opacity_at(&mut state, 16), 0.8 * (10.0 / 12.0));

    // A gesture that comes back to the original value is not an edit and keeps the window.
    begin(&mut state);
    update(&mut state, 8.0);
    update(&mut state, 12.0);
    state
        .execute(Command::PropertyGesture(GestureCommand::Commit {
            gesture: None,
        }))
        .unwrap();
    assert_eq!(window_of(&state, "t-split-1").lead.unwrap().frames, 4);
}

#[test]
fn moving_a_half_carries_its_envelope_and_a_speed_change_does_not_touch_it() {
    let mut state = halves();
    let (a, b) = (opacity_at(&mut state, 20), opacity_at(&mut state, 63));
    state
        .execute(Command::MoveClip {
            clip: "t-split-1".into(),
            track: Some("ut".into()),
            frame: Some(20),
            snap: Some(false),
        })
        .unwrap();
    // The half now starts 10 frames later (and on another track): the envelope moved with it.
    assert_eq!(opacity_at(&mut state, 30), a);
    assert_eq!(opacity_at(&mut state, 73), b);
    assert_eq!(window_of(&state, "t-split-1").lead.unwrap().frames, 4);

    // Speed: a declared video clip's envelope is in sequence frames; the speed moves the source.
    let mut video = create((24, 1));
    let fake = shared_dir().join("fake-speed.mp4");
    fs::write(&fake, b"not decoded by the core").unwrap();
    run(
        &mut video,
        json!({"op":"asset.add","kind":"video","id":"fakev","name":"Declared","path":fake,
            "width":64,"height":36,"rate":24,"duration":10}),
    );
    run(
        &mut video,
        json!({"op":"track.add","id":"vt","name":"vt","kind":"video"}),
    );
    run(
        &mut video,
        json!({"op":"clip.add","id":"cv","track":"vt","asset":"fakev","start":6,"duration":60}),
    );
    set(&mut video, "cv", "fade_in", json!(12));
    set(&mut video, "cv", "fade_out", json!(24));
    split(&mut video, "cv", 10);
    let before = (10..66)
        .map(|frame| opacity_at(&mut video, frame))
        .collect::<Vec<_>>();
    for clip in ["cv", "cv-split-1"] {
        set(&mut video, clip, "speed", json!(250));
    }
    let after = (10..66)
        .map(|frame| opacity_at(&mut video, frame))
        .collect::<Vec<_>>();
    assert_eq!(before, after);
    assert_eq!(window_of(&video, "cv-split-1").lead.unwrap().frames, 4);
}

#[test]
fn trimming_a_half_keeps_the_ramp_in_place_until_the_edge_passes_its_origin() {
    let mut state = halves();
    state.timeline.snap = false;
    let trim = |state: &mut EditorState, clip: &str, edge, frame| {
        state
            .execute(Command::TrimClip {
                clip: clip.into(),
                edge,
                frame,
            })
            .unwrap();
    };
    let reference: Vec<f64> = (10..66).map(|f| opacity_at(&mut state, f)).collect();

    // Head trimmed 3 frames inward: the remaining frames read the same envelope; lead 4 -> 7.
    trim(&mut state, "t-split-1", TrimEdge::Start, 13);
    assert_eq!(window_of(&state, "t-split-1").lead.unwrap().frames, 7);
    for frame in 13..66 {
        assert_eq!(
            opacity_at(&mut state, frame),
            reference[(frame - 10) as usize],
            "{frame}"
        );
    }
    // Trimmed back out to the cut: 4 again; one more frame out is not possible (the left half).
    trim(&mut state, "t-split-1", TrimEdge::Start, 10);
    assert_eq!(window_of(&state, "t-split-1").lead.unwrap().frames, 4);

    // Left half out of the way, head trimmed past the origin of the ramp (frame 6): the lead is
    // gone and the fade is measured from the new first frame.
    state
        .execute(Command::MoveClip {
            clip: "t".into(),
            track: Some("ut".into()),
            frame: Some(30),
            snap: Some(false),
        })
        .unwrap();
    trim(&mut state, "t-split-1", TrimEdge::Start, 4);
    assert!(window_of(&state, "t-split-1").is_empty());
    assert_eq!(opacity_at(&mut state, 4), 0.0);
    assert_eq!(opacity_at(&mut state, 10), 0.8 * (6.0 / 12.0));

    // The tail follows the end edge the same way: the left half of a cut inside the fade-out.
    let mut state = visual_project((24, 1));
    state.timeline.snap = false;
    set(&mut state, "t", "fade_out", json!(24));
    split(&mut state, "t", 50);
    assert_eq!(window_of(&state, "t").tail.unwrap().frames, 16);
    let reference: Vec<f64> = (6..50).map(|f| opacity_at(&mut state, f)).collect();
    trim(&mut state, "t", TrimEdge::End, 45);
    assert_eq!(window_of(&state, "t").tail.unwrap().frames, 21);
    for frame in 6..45 {
        assert_eq!(
            opacity_at(&mut state, frame),
            reference[(frame - 6) as usize],
            "{frame}"
        );
    }
}

#[test]
fn moving_and_trimming_a_cut_sound_keeps_its_samples() {
    // 24 fps, 48 kHz: 2000 samples a frame, so a move by whole frames is an exact shift.
    let mut reference = audio_project((24, 1), 48_000, true);
    set(&mut reference, "main", "fade_in", json!(20));
    set(&mut reference, "main", "fade_out", json!(20));
    let whole = mix(&reference);
    let frame = 2000usize * 2;
    let mut state = reference.clone();
    split(&mut state, "main", 12);
    state.timeline.snap = false;

    // Right half 10 frames later: its samples are the whole's, shifted.
    state
        .execute(Command::MoveClip {
            clip: "main-split-1".into(),
            track: None,
            frame: Some(22),
            snap: Some(false),
        })
        .unwrap();
    let moved = mix(&state);
    for n in 0..(12 * frame) {
        assert_eq!(
            moved.samples[n].to_bits(),
            whole.samples[n].to_bits(),
            "left {n}"
        );
    }
    for n in (12 * frame)..(22 * frame) {
        assert_eq!(moved.samples[n], 0.0, "the gap is silent at {n}");
    }
    for n in (12 * frame)..(63 * frame) {
        assert_eq!(
            moved.samples[n + 10 * frame].to_bits(),
            whole.samples[n].to_bits(),
            "right {n}"
        );
    }
    run(&mut state, json!({"op":"history.undo"}));

    // Head of the right half trimmed 3 frames: the rest is the whole's, bit for bit.
    state
        .execute(Command::TrimClip {
            clip: "main-split-1".into(),
            edge: TrimEdge::Start,
            frame: 15,
        })
        .unwrap();
    let trimmed = mix(&state);
    for n in 0..(12 * frame) {
        assert_eq!(
            trimmed.samples[n].to_bits(),
            whole.samples[n].to_bits(),
            "left {n}"
        );
    }
    for n in (12 * frame)..(15 * frame) {
        assert_eq!(trimmed.samples[n], 0.0, "trimmed away at {n}");
    }
    for n in (15 * frame)..(63 * frame) {
        assert_eq!(
            trimmed.samples[n].to_bits(),
            whole.samples[n].to_bits(),
            "right {n}"
        );
    }
}

#[test]
fn editing_a_fade_of_a_cut_sound_follows_the_ordinary_rule_for_that_half() {
    let mut state = audio_project((24, 1), 48_000, true);
    set(&mut state, "main", "fade_in", json!(20));
    split(&mut state, "main", 10);
    let plan = |state: &EditorState| state.plan_audio(0, LENGTH).unwrap().to_json();
    let right = |plan: &Value| {
        plan["segments"]
            .as_array()
            .unwrap()
            .iter()
            .find(|segment| segment["clip"] == "main-split-1")
            .cloned()
            .unwrap()
    };
    // The half carries the ramp that began 7 frames before it: 13 frames (26 000 samples) left.
    let inherited = right(&plan(&state));
    assert_eq!(inherited["fade_in"], 20 * 2000);
    assert_eq!(inherited["clip_offset"], 7 * 2000);
    // Editing the fade makes it the half's own: measured from its own first sample.
    set(&mut state, "main-split-1", "fade_in", json!(5));
    let own = right(&plan(&state));
    assert_eq!(own["fade_in"], 5 * 2000);
    assert_eq!(own["clip_offset"], 0);
    assert_eq!(own["clip_frames"], 53 * 2000);
}

// ---- refusal, UI availability, process ------------------------------------------------------
