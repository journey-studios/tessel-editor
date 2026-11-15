//! BL081.2.2: repeated cuts, files and refusals of sound ramps that carry their own normalization.
use super::ramp_support::*;
use super::*;

/// Every property of a document the editor saves: it reopens as the same project.
fn assert_reopens(state: &EditorState, label: &str) -> EditorState {
    let again = reopened(state);
    assert_eq!(again.timeline.clips, state.timeline.clips, "{label}: clips");
    assert_eq!(mix(&again).samples, mix(state).samples, "{label}: sound");
    again
}

#[test]
fn several_cuts_and_edits_keep_every_inherited_ramp_across_files_and_history() {
    for (rate, sample_rate) in [
        ((24, 1), 48_000),
        ((30_000, 1001), 44_100),
        ((25, 1), 44_100),
    ] {
        let reference = whole(rate, sample_rate, true, 40, 40);
        let original = mix(&reference);
        let ref_seg = segment(&reference, "main");
        let end = at(rate, LENGTH);
        let (a, b, c) = ("main", "main-split-1", "main-split-1-split-1");

        let mut state = cut(&reference, "main", 20);
        split(&mut state, b, 45);
        let cuts = state.timeline.clips.clone();
        same_bits(&original, &mix(&state), "three pieces");
        assert_reopens(&state, "three pieces");

        // Each piece edits one fade: the other ramp is the whole's, wherever the piece is.
        set(&mut state, b, "fade_in", json!(5));
        let piece = segment(&state, b);
        assert_eq!(
            (piece.fade_out, piece.out_anchor()),
            (ref_seg.fade_out, ref_seg.out_anchor()),
            "{rate:?}: middle fade-out"
        );
        set(&mut state, c, "fade_in", json!(3));
        let piece = segment(&state, c);
        assert_eq!(
            (piece.fade_out, piece.out_anchor()),
            (ref_seg.fade_out, ref_seg.out_anchor()),
            "{rate:?}: last fade-out"
        );
        set(&mut state, a, "fade_out", json!(6));
        let piece = segment(&state, a);
        assert_eq!(
            (piece.fade_in, piece.in_anchor()),
            (ref_seg.fade_in, ref_seg.in_anchor()),
            "{rate:?}: first fade-in"
        );
        let edited = mix(&state);
        let (b_seg, c_seg, a_seg) = (segment(&state, b), segment(&state, c), segment(&state, a));
        let whole_in_end = ref_seg.in_end() as usize;
        assert_same(
            &original,
            &edited,
            b_seg.in_end().max(ref_seg.in_end()) as usize,
            at(rate, 45),
            "middle",
        );
        assert_same(
            &original,
            &edited,
            c_seg.in_end().max(ref_seg.in_end()) as usize,
            end,
            "last",
        );
        assert_same(
            &original,
            &edited,
            0,
            a_seg.out_start().min(ref_seg.out_start()) as usize,
            "first",
        );
        assert!(
            whole_in_end > at(rate, 20),
            "the whole's fade-in is over by the middle piece"
        );
        assert_reopens(&state, "three edits");
        // The previous model (windows gone) cannot pass the same checks.
        let control = mix(&without_windows(&state));
        assert!(
            differs(&original, &control, c_seg.in_end() as usize, end)
                || differs(&original, &control, b_seg.in_end() as usize, at(rate, 45)),
            "{rate:?}: the control must see the loss"
        );

        // A new cut of an edited piece keeps its ramps: the middle piece (native fade-in,
        // inherited fade-out) cut again changes nothing it should not.
        split(&mut state, b, 30);
        same_bits(&edited, &mix(&state), "cut of an edited piece");
        let tail_piece = segment(&state, "main-split-1-split-2");
        assert_eq!(
            (tail_piece.fade_out, tail_piece.out_anchor()),
            (ref_seg.fade_out, ref_seg.out_anchor())
        );
        assert_reopens(&state, "cut of an edited piece");

        // The history restores each state exactly.
        for _ in 0..4 {
            run(&mut state, json!({"op":"history.undo"}));
        }
        same_bits(&original, &mix(&state), "undone to three pieces");
        assert_eq!(state.timeline.clips, cuts);
        for _ in 0..4 {
            run(&mut state, json!({"op":"history.redo"}));
        }
        assert_eq!(mix(&state).samples.len(), edited.samples.len());
    }
}

#[test]
fn a_sound_window_the_editor_could_not_have_written_is_refused_whole() {
    let mut state = audio_project((24, 1), 48_000, true);
    set(&mut state, "main", "fade_out", json!(30));
    split(&mut state, "main", 40);
    let doc = state.to_document(None);
    assert_eq!(doc["schema_version"], 3);
    let clips = doc["sequences"][0]["clips"].as_array().unwrap();
    let index = clips
        .iter()
        .position(|clip| clip["id"] == "main-split-1")
        .unwrap();
    assert_eq!(
        clips[index]["fade_window"],
        json!({"tail": {"frames": 0, "fade": 30.0, "norm": {"far": 37, "peer": 0}}}),
        "the pinned fade-out of the reported right half"
    );
    assert!(EditorState::from_document(&doc, None).is_ok());

    let open = audio_project((24, 1), 48_000, true);
    let untouched = fingerprint(&open);
    let bad = |window: Value| {
        let mut bad = doc.clone();
        bad["sequences"][0]["clips"][index]["fade_window"] = window;
        bad
    };
    let tail = |norm: Value| json!({"tail": {"frames": 0, "fade": 30, "norm": norm}});
    let mut refused = vec![
        // A sound ramp always has its normalization; a picture's never does.
        bad(json!({"tail": {"frames": 3, "fade": 30}})),
        bad(tail(json!({"far": 37}))),
        bad(tail(json!({"peer": 0}))),
        bad(tail(json!({"far": 37, "peer": -1}))),
        bad(tail(json!({"far": 37.5, "peer": 0}))),
        bad(tail(json!({"far": i64::MAX, "peer": 0}))),
        bad(tail(json!({"far": i64::MIN, "peer": 0}))),
        bad(tail(json!({"far": -23, "peer": 0}))),
        bad(tail(json!({"far": 999_999, "peer": 0}))),
        bad(tail(json!({"far": 37, "peer": 1_000_001}))),
        bad(tail(json!({"far": 37, "peer": 0, "extra": 1}))),
        bad(json!({"tail": {"frames": -1, "fade": 30, "norm": {"far": 37, "peer": 0}}})),
        bad(json!({"tail": {"frames": 1_000_000, "fade": 30, "norm": {"far": 0, "peer": 0}}})),
    ];
    for version in [1, 2] {
        let mut old = doc.clone();
        old["schema_version"] = json!(version);
        refused.push(old);
    }
    // A picture with a `norm` is refused too.
    let mut visual = visual_project((24, 1));
    set(&mut visual, "t", "fade_in", json!(30));
    split(&mut visual, "t", 20);
    let mut picture = visual.to_document(None);
    let clip = picture["sequences"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .position(|clip| clip["id"] == "t-split-1")
        .unwrap();
    assert!(EditorState::from_document(&picture, None).is_ok());
    picture["sequences"][0]["clips"][clip]["fade_window"] =
        json!({"lead": {"frames": 4, "fade": 30, "norm": {"far": 0, "peer": 0}}});
    refused.push(picture);
    for doc in refused {
        let error = EditorState::from_document(&doc, None).expect_err(&format!("{doc}"));
        assert_eq!(error.code, "invalid_document", "{doc}");
        assert_eq!(fingerprint(&open), untouched, "opening is a candidate swap");
    }
}

#[test]
fn a_sound_window_is_dropped_on_open_when_the_stored_fade_no_longer_matches_it() {
    let mut state = audio_project((24, 1), 48_000, true);
    set(&mut state, "main", "fade_out", json!(30));
    split(&mut state, "main", 40);
    let mut doc = state.to_document(None);
    doc["clip_properties"]["main-split-1"]["fade_out"] = json!(31);
    let reopened = EditorState::from_document(&doc, None).unwrap();
    assert!(
        window_of(&reopened, "main-split-1").is_empty(),
        "the guard is stale: not resurrected"
    );
    assert_eq!(
        segment(&reopened, "main-split-1").fade_out,
        23 * 2000,
        "an ordinary 31-frame ramp, cut to the 23-frame half"
    );
}

#[test]
fn a_trim_at_the_limit_of_a_sound_is_refused_whole_or_leaves_a_reopenable_document() {
    let mut state = audio_project((24, 1), 48_000, true);
    run(
        &mut state,
        json!({"op":"track.add","id":"a2","name":"a2","kind":"audio"}),
    );
    state.timeline.length = 1_000_000;
    let clip = state
        .timeline
        .clips
        .iter_mut()
        .find(|clip| clip.id == "main")
        .unwrap();
    clip.start = 0;
    clip.duration = 999_000;
    set(&mut state, "main", "fade_in", json!(48));
    set(&mut state, "main", "fade_out", json!(48));
    split(&mut state, "main", 10);
    split(&mut state, "main-split-1", 998_990);
    state
        .execute(Command::MoveClip {
            clip: "main-split-1-split-1".into(),
            track: Some("a2".into()),
            frame: Some(1020),
            snap: Some(false),
        })
        .unwrap();
    assert_reopens_document(&state);
    for (edge, frame) in [
        (TrimEdge::Start, 0),
        (TrimEdge::End, 1_000_000),
        (TrimEdge::Start, 1025),
        (TrimEdge::End, 1029),
    ] {
        let before = fingerprint(&state);
        let result = state.execute(Command::TrimClip {
            clip: "main-split-1-split-1".into(),
            edge,
            frame,
        });
        match result {
            Ok(_) => {}
            Err(error) => {
                assert_eq!(error.code, "invalid_time", "{edge:?} {frame}");
                assert_eq!(fingerprint(&state), before, "{edge:?} {frame}: atomic");
            }
        }
        assert_reopens_document(&state);
    }
}

fn assert_reopens_document(state: &EditorState) {
    let again = EditorState::from_document(&state.to_document(None), None)
        .expect("a document the editor saves reopens");
    assert_eq!(again.timeline.clips, state.timeline.clips);
}

#[test]
fn a_short_half_can_require_schema_three_to_keep_a_future_fade_edit_independent() {
    let mut state = whole((24, 1), 48_000, false, 5, 0);
    assert_eq!(state.document_schema_version(), 1);
    split(&mut state, "main", 33);
    let pinned = window_of(&state, "main").lead.unwrap();
    assert_eq!(pinned.frames, 0);
    assert!(pinned.norm.is_some());
    assert_eq!(state.document_schema_version(), 3);
}
