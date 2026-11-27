//! BL081.2.2: what happens to the ramp that was not edited when the half is trimmed, moved,
//! reset, undone, or edited through a slider gesture (cancelled or committed).
use super::ramp_support::*;
use super::*;

const RATE: (u32, u32) = (24, 1);
const FRAME: usize = 2000; // sample frames of the mix per frame at 24 fps

/// `main` 3..63 with `fade_out` 30 and no fade-in, cut at 40 and the right half's fade-in set
/// to 5: the reported state.
fn edited_right() -> (EditorState, Pcm, Segment) {
    let reference = whole(RATE, 48_000, true, 0, 30);
    let original = mix(&reference);
    let ref_seg = segment(&reference, "main");
    let mut state = cut(&reference, "main", 40);
    set(&mut state, "main-split-1", "fade_in", json!(5));
    (state, original, ref_seg)
}

fn trim(state: &mut EditorState, clip: &str, edge: TrimEdge, frame: i64) {
    state
        .execute(Command::TrimClip {
            clip: clip.into(),
            edge,
            frame,
        })
        .unwrap();
}

fn gesture(state: &mut EditorState, command: GestureCommand) {
    state.execute(Command::PropertyGesture(command)).unwrap();
}

fn begin(state: &mut EditorState, clip: &str, property: &str) {
    gesture(
        state,
        GestureCommand::Begin {
            clip: Some(clip.into()),
            property: property.into(),
        },
    );
}

fn update(state: &mut EditorState, value: f64) {
    gesture(
        state,
        GestureCommand::Update {
            gesture: None,
            value: Raw::Number(value),
        },
    );
}

#[test]
fn trimming_and_moving_the_half_leave_the_inherited_ramp_where_the_whole_had_it() {
    let (mut state, original, ref_seg) = edited_right();
    state.timeline.snap = false;
    let end = at(RATE, LENGTH);

    // Head inward: the new fade-in is measured from the new edge; the fade-out is the whole's.
    trim(&mut state, "main-split-1", TrimEdge::Start, 43);
    let seg = segment(&state, "main-split-1");
    assert_eq!(
        (seg.fade_out, seg.out_anchor()),
        (ref_seg.fade_out, ref_seg.out_anchor())
    );
    assert_eq!(seg.in_anchor(), 43 * FRAME as u64);
    assert_same(
        &original,
        &mix(&state),
        seg.in_end() as usize,
        end,
        "head trimmed",
    );

    // This ramp was anchored on its own edge (tail=0). Trimming that edge must
    // follow the fresh edge rule, just like an uncut/native ramp (review F1).
    let before_tail_trim = mix(&state);
    trim(&mut state, "main-split-1", TrimEdge::End, 59);
    let seg = segment(&state, "main-split-1");
    assert!(window_of(&state, "main-split-1").tail.is_none());
    assert_eq!(seg.out_anchor(), (59 * FRAME) as u64);
    assert_eq!(
        seg.fade_out,
        tessel_editor_core::audio::resolve_fades(
            5 * FRAME as u64,
            30 * FRAME as u64,
            16 * FRAME as u64
        )
        .1
    );
    let trimmed = mix(&state);
    assert!(differs(&before_tail_trim, &trimmed, 48 * FRAME, 59 * FRAME));
    let trimmed_fade_out = seg.fade_out;

    // Moved 10 frames later: the ramp travels with the half, sample for sample.
    state
        .execute(Command::MoveClip {
            clip: "main-split-1".into(),
            track: None,
            frame: Some(53),
            snap: Some(false),
        })
        .unwrap();
    let moved = mix(&state);
    let shift = 10 * FRAME;
    let seg = segment(&state, "main-split-1");
    assert_eq!(
        seg.fade_out, trimmed_fade_out,
        "the ramp is not touched by a move"
    );
    for n in (seg.in_end() as usize * 2)..(69 * FRAME * 2) {
        assert_eq!(
            moved.samples[n].to_bits(),
            trimmed.samples[n - shift * 2].to_bits(),
            "sample {n}"
        );
    }
    // Moving back and extending the head to the cut again changes nothing it should not.
    state
        .execute(Command::MoveClip {
            clip: "main-split-1".into(),
            track: None,
            frame: Some(43),
            snap: Some(false),
        })
        .unwrap();
    assert_eq!(mix(&state).samples, trimmed.samples, "back where it was");
}

#[test]
fn reset_undo_and_redo_restore_both_ramps_exactly() {
    let (mut state, original, _) = edited_right();
    let edited = mix(&state);
    assert!(differs(&original, &edited, 40 * FRAME, 45 * FRAME));

    // Resetting the fade-in removes only the edited ramp: the sound is the whole's again.
    state
        .execute(Command::ResetProperty {
            property: "fade_in".into(),
        })
        .unwrap();
    same_bits(&original, &mix(&state), "reset");
    run(&mut state, json!({"op":"history.undo"}));
    same_bits(&edited, &mix(&state), "undo reset");
    run(&mut state, json!({"op":"history.undo"}));
    same_bits(&original, &mix(&state), "undo edit: the cut");
    run(&mut state, json!({"op":"history.redo"}));
    same_bits(&edited, &mix(&state), "redo edit");
    run(&mut state, json!({"op":"history.redo"}));
    same_bits(&original, &mix(&state), "redo reset");

    // Zero is a reset too.
    run(&mut state, json!({"op":"history.undo"}));
    run(&mut state, json!({"op":"history.undo"}));
    set(&mut state, "main-split-1", "fade_in", json!(0));
    same_bits(&original, &mix(&state), "zero");
}

#[test]
fn editing_the_fade_out_of_the_half_hands_over_only_that_ramp_and_never_resurrects_the_old_one() {
    let (mut state, original, ref_seg) = edited_right();
    let in_edit = mix(&state);
    // The fade-out is the half's now: 7 frames from its end, next to the 5-frame fade-in.
    set(&mut state, "main-split-1", "fade_out", json!(7));
    let seg = segment(&state, "main-split-1");
    assert_eq!(
        (seg.fade_in, seg.fade_out),
        (5 * FRAME as u64, 7 * FRAME as u64)
    );
    assert!(
        window_of(&state, "main-split-1").is_empty(),
        "both ramps are the half's own"
    );
    // The fade-in is still the edited one (its length and place), only the fade-out went.
    assert_eq!(seg.in_anchor(), 40 * FRAME as u64);
    assert_eq!(seg.out_anchor(), at(RATE, 63) as u64);
    // Editing back to the old number is a fresh ramp of the half, not the inherited one: on
    // the half alone it is shortened against the fade-in, which the whole's ramp never was.
    set(&mut state, "main-split-1", "fade_out", json!(30));
    assert!(window_of(&state, "main-split-1").is_empty());
    let fresh = segment(&state, "main-split-1");
    assert_ne!(
        fresh.fade_out, ref_seg.fade_out,
        "a new ramp, resolved on the half"
    );
    assert!(differs(
        &original,
        &mix(&state),
        50 * FRAME,
        at(RATE, LENGTH)
    ));
    // Undo brings the inherited ramp back (the history holds the pinned half).
    run(&mut state, json!({"op":"history.undo"}));
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(segment(&state, "main-split-1").fade_out, ref_seg.fade_out);
    same_bits(&in_edit, &mix(&state), "undone to the fade-in edit");
}

#[test]
fn a_slider_gesture_on_a_sound_half_cancels_and_commits_without_touching_the_other_ramp() {
    let reference = whole(RATE, 48_000, true, 0, 30);
    let original = mix(&reference);
    let end = at(RATE, LENGTH);
    let mut state = cut(&reference, "main", 40);
    let cut_only = mix(&state);

    // The preview shows the new ramp from the edge at once; the end of the sound is the whole's.
    begin(&mut state, "main-split-1", "fade_in");
    update(&mut state, 5.0);
    let preview = mix(&state);
    assert_same(&original, &preview, 45 * FRAME, end, "preview");
    assert_eq!(preview.sample(40 * FRAME, 0), 0.0);
    update(&mut state, 12.0);
    assert_same(&original, &mix(&state), 52 * FRAME, end, "preview moved on");
    // Cancel: nothing of the preview is left.
    gesture(&mut state, GestureCommand::Cancel { gesture: None });
    same_bits(&cut_only, &mix(&state), "cancelled");

    // Commit: the edit stands, the other ramp is the whole's, undo and redo are exact.
    begin(&mut state, "main-split-1", "fade_in");
    update(&mut state, 5.0);
    gesture(&mut state, GestureCommand::Commit { gesture: None });
    let committed = mix(&state);
    assert_same(&original, &committed, 45 * FRAME, end, "committed");
    // Control: a model whose edit drops the window moves the end of the sound.
    let old = mix(&without_windows(&state));
    assert!(
        differs(&original, &old, 45 * FRAME, end),
        "the control sees it"
    );
    assert_eq!(segment(&state, "main-split-1").fade_out, 30 * FRAME as u64);
    run(&mut state, json!({"op":"history.undo"}));
    same_bits(&cut_only, &mix(&state), "undo of the gesture");
    run(&mut state, json!({"op":"history.redo"}));
    same_bits(&committed, &mix(&state), "redo of the gesture");

    // The symmetric half: the fade-out of the left half through the slider.
    let reference = whole(RATE, 44_100, false, 30, 0);
    let original = mix(&reference);
    let mut state = cut(&reference, "main", 20);
    let cut_only = mix(&state);
    begin(&mut state, "main", "fade_out");
    update(&mut state, 5.0);
    assert_same(&original, &mix(&state), 0, 15 * FRAME, "left preview");
    gesture(&mut state, GestureCommand::Cancel { gesture: None });
    same_bits(&cut_only, &mix(&state), "left cancelled");
    begin(&mut state, "main", "fade_out");
    update(&mut state, 5.0);
    gesture(&mut state, GestureCommand::Commit { gesture: None });
    assert_same(&original, &mix(&state), 0, 15 * FRAME, "left committed");
    assert_eq!(segment(&state, "main").fade_in, 30 * FRAME as u64);
}

#[test]
fn a_gesture_that_returns_to_the_cut_value_is_the_cut_and_one_that_ends_elsewhere_is_an_edit() {
    // 40 + 40 on 60 frames: both ramps are pinned on the right half; dragging the fade-in away
    // and back lands exactly on the inherited ramp, committing elsewhere hands it over.
    let reference = whole(RATE, 48_000, true, 40, 40);
    let original = mix(&reference);
    let mut state = cut(&reference, "main", 40);
    begin(&mut state, "main-split-1", "fade_in");
    update(&mut state, 5.0);
    assert!(
        differs(&original, &mix(&state), 40 * FRAME, 45 * FRAME),
        "the preview"
    );
    update(&mut state, 40.0);
    same_bits(&original, &mix(&state), "back on the inherited value");
    gesture(&mut state, GestureCommand::Commit { gesture: None });
    same_bits(&original, &mix(&state), "committed unchanged");
    assert!(
        window_of(&state, "main-split-1").lead.is_some(),
        "not an edit: the ramp is still the inherited one"
    );
    begin(&mut state, "main-split-1", "fade_in");
    update(&mut state, 5.0);
    gesture(&mut state, GestureCommand::Commit { gesture: None });
    let right = window_of(&state, "main-split-1");
    assert!(right.lead.is_none() && right.tail.is_some());
    let seg = segment(&state, "main-split-1");
    let ref_seg = segment(&reference, "main");
    assert_eq!(
        (seg.fade_out, seg.out_anchor()),
        (ref_seg.fade_out, ref_seg.out_anchor())
    );
}

#[test]
fn fractional_fades_from_the_document_are_rounded_once_and_kept_across_edits_and_files() {
    // The document says 12.4 and 29.6; the mixer rounds them (12, 30) and so does every half.
    let mut base = whole(RATE, 44_100, true, 12, 30);
    let mut doc = base.to_document(None);
    doc["clip_properties"]["main"]["fade_in"] = json!(12.4);
    doc["clip_properties"]["main"]["fade_out"] = json!(29.6);
    let fractional = EditorState::from_document(&doc, None).unwrap();
    assert_eq!(number(&fractional, "main", "fade_out"), 29.6);
    let original = mix(&fractional);
    same_bits(&mix(&base), &original, "rounded like the integers");
    base.timeline.playhead = 0;

    for cut_at in [20, 40, 57] {
        let mut state = cut(&fractional, "main", cut_at);
        same_bits(&original, &mix(&state), &format!("cut {cut_at}"));
        // The guard keeps the fractional value exactly: a file brings it back unchanged.
        assert_eq!(
            mix(&reopened(&state)).samples,
            mix(&state).samples,
            "file {cut_at}"
        );
        set(&mut state, "main-split-1", "fade_in", json!(5));
        let right = segment(&state, "main-split-1");
        let ref_seg = segment(&fractional, "main");
        assert_eq!(right.fade_out, ref_seg.fade_out, "cut {cut_at}");
        assert_eq!(right.out_anchor(), ref_seg.out_anchor(), "cut {cut_at}");
        let from = (right.in_end().max(ref_seg.in_end()) as usize).min(at(RATE, LENGTH));
        assert_same(
            &original,
            &mix(&state),
            from,
            at(RATE, LENGTH),
            &format!("edit {cut_at}"),
        );
        assert_eq!(
            mix(&reopened(&state)).samples,
            mix(&state).samples,
            "edited file {cut_at}"
        );
        let mut left = cut(&fractional, "main", cut_at);
        set(&mut left, "main", "fade_out", json!(5));
        let seg = segment(&left, "main");
        assert_eq!(
            seg.fade_in,
            segment(&fractional, "main").fade_in,
            "cut {cut_at}"
        );
    }
}

#[test]
fn a_pinned_sound_ramp_on_the_edge_follows_that_edge_when_trimmed() {
    for rate in RATES {
        let reference = whole(rate, 48_000, true, 40, 40);
        let mut left = cut(&reference, "main", 20);
        assert_eq!(window_of(&left, "main").lead.unwrap().frames, 0);
        let before = fingerprint(&left);
        trim(&mut left, "main", TrimEdge::Start, 8);
        assert!(window_of(&left, "main").lead.is_none());
        assert_eq!(segment(&left, "main").in_anchor(), at(rate, 8) as u64);
        assert_eq!(mix(&left).sample(at(rate, 8), 0), 0.0);
        let after = fingerprint(&left);
        run(&mut left, json!({"op":"history.undo"}));
        assert_eq!(fingerprint(&left)["document"], before["document"]);
        run(&mut left, json!({"op":"history.redo"}));
        assert_eq!(fingerprint(&left)["document"], after["document"]);
        assert!(EditorState::from_document(&left.to_document(None), None).is_ok());

        let mut right = cut(&reference, "main", 40);
        assert_eq!(window_of(&right, "main-split-1").tail.unwrap().frames, 0);
        trim(&mut right, "main-split-1", TrimEdge::End, 58);
        assert!(window_of(&right, "main-split-1").tail.is_none());
        assert_eq!(
            segment(&right, "main-split-1").out_anchor(),
            at(rate, 58) as u64
        );
    }
}
