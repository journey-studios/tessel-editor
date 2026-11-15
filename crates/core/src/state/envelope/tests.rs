use core::prelude::v1::test;

use super::*;

fn input(audio: bool, before: i64, after: i64, fi: f64, fo: f64) -> CutInput {
    CutInput {
        audio,
        before,
        after,
        lead: None,
        tail: None,
        raw_in: fi,
        raw_out: fo,
    }
}

fn picture(frames: i64, fade: f64) -> Reach {
    Reach {
        frames,
        fade,
        norm: None,
    }
}

fn sound(frames: i64, fade: f64, far: i64, peer: i64) -> Reach {
    Reach {
        frames,
        fade,
        norm: Some(Norm { far, peer }),
    }
}

#[test]
fn a_cut_outside_the_ramps_zeroes_them_and_needs_no_window() {
    let cut = split_fades(input(false, 20, 28, 6.0, 12.0)).unwrap();
    assert!(cut.right.zero_in && cut.left.zero_out);
    assert!(!cut.left.zero_in && !cut.right.zero_out);
    assert!(cut.left.window.is_empty() && cut.right.window.is_empty());
}

#[test]
fn a_cut_inside_the_ramps_keeps_them_and_anchors_each_half() {
    let cut = split_fades(input(false, 4, 20, 12.0, 24.0)).unwrap();
    assert!(!cut.right.zero_in);
    assert!(!cut.left.zero_out);
    let right = cut.right.window;
    assert_eq!(right.lead.unwrap().frames, 4);
    assert_eq!(right.lead.unwrap().fade, 12.0);
    assert!(right.tail.is_none());
    let left = cut.left.window;
    assert!(left.lead.is_none());
    assert_eq!(left.tail.unwrap().frames, 20);
    assert!(left.tail.unwrap().norm.is_none(), "a picture has no norm");
}

#[test]
fn a_cut_exactly_at_the_end_of_a_ramp_is_not_inside_it() {
    let at = split_fades(input(false, 12, 36, 12.0, 0.0)).unwrap();
    assert!(at.right.zero_in && at.right.window.is_empty());
    let inside = split_fades(input(false, 11, 37, 12.0, 0.0)).unwrap();
    assert!(!inside.right.zero_in);
}

#[test]
fn overlapping_audio_ramps_stay_on_both_halves_each_with_its_own_group() {
    let cut = split_fades(input(true, 30, 18, 30.0, 30.0)).unwrap();
    assert!(!cut.right.zero_in && !cut.left.zero_out);
    let out = cut.left.window.tail.unwrap();
    assert_eq!((out.frames, out.norm.unwrap().far), (18, 0));
    assert_eq!(out.norm.unwrap().peer, 30, "the request of the other ramp");
    let lead = cut.right.window.lead.unwrap();
    assert_eq!((lead.frames, lead.norm.unwrap().far), (30, 0));
}

#[test]
fn an_audio_ramp_longer_than_its_half_is_pinned_with_the_span_of_the_whole() {
    // 3..63, fade-out 30, cut at 40: the half has 23 frames, `resolve_fades` would cut the ramp
    // to 23. Pinned: anchored on the end (0 frames), normalized on the 60 frames of the whole.
    let cut = split_fades(input(true, 37, 23, 0.0, 30.0)).unwrap();
    let pin = cut.right.window.tail.expect("pinned");
    assert_eq!(pin.frames, 0);
    assert_eq!(pin.norm, Some(Norm { far: 37, peer: 0 }));
    assert_eq!(pin.fade, 30.0);
    assert!(
        cut.right.window.lead.is_none(),
        "no fade-in, nothing to pin"
    );
    // The left half holds only the first 37 frames of a ramp that starts at 33: it keeps the ramp.
    let left = cut
        .left
        .window
        .tail
        .expect("anchored 23 frames past the end");
    assert_eq!(left.frames, 23);
    // And the symmetric case.
    let cut = split_fades(input(true, 17, 43, 30.0, 0.0)).unwrap();
    let pin = cut.left.window.lead.expect("pinned");
    assert_eq!(pin.frames, 0);
    assert_eq!(pin.norm, Some(Norm { far: 43, peer: 0 }));
}

#[test]
fn an_audio_ramp_nothing_can_shorten_needs_no_pin() {
    // A half so long that no fade the sliders allow (48) can overlap the ramp: it is a plain
    // ramp of the half and nothing has to be remembered.
    let plain = split_fades(input(true, 80, 18, 12.0, 0.0)).unwrap();
    assert!(plain.left.window.is_empty());
    assert!(plain.right.zero_in);
    // 30 frames: editing the other fade to 48 would make the mixer shorten the 12-frame ramp.
    let short = split_fades(input(true, 30, 18, 12.0, 0.0)).unwrap();
    let pin = short.left.window.lead.expect("pinned");
    assert_eq!((pin.frames, pin.norm), (0, Some(Norm { far: 18, peer: 0 })));
    // 40 on the 30-frame left half would be cut to the half by the mixer: it needs the span.
    let longer = split_fades(input(true, 30, 18, 40.0, 0.0)).unwrap();
    assert!(!longer.left.window.is_empty());
}

#[test]
fn a_pinned_ramp_keeps_its_group_when_the_cut_is_repeated() {
    let first = split_fades(input(true, 37, 23, 0.0, 30.0)).unwrap();
    // Cut the right half (23 frames) again at 10 / 13, as the clip would carry the pin.
    let second = split_fades(CutInput {
        audio: true,
        before: 10,
        after: 13,
        lead: None,
        tail: first.right.window.tail,
        raw_in: 0.0,
        raw_out: 30.0,
    })
    .unwrap();
    let left = second.left.window.tail.unwrap();
    assert_eq!(
        (left.frames, left.norm),
        (13, Some(Norm { far: 37, peer: 0 }))
    );
    let right = second.right.window.tail.unwrap();
    assert_eq!(
        (right.frames, right.norm),
        (0, Some(Norm { far: 47, peer: 0 }))
    );
}

#[test]
fn a_native_ramp_next_to_a_pinned_one_sees_no_peer() {
    // Right half of a 3..63 clip with fade-in 40 / out 40 (30 + 30 resolved) cut at 40: the
    // fade-out is pinned with the request of the fade-in (40) as its peer.
    let cut = split_fades(input(true, 37, 23, 40.0, 40.0)).unwrap();
    let out = cut.right.window.tail.unwrap();
    assert_eq!(out.norm, Some(Norm { far: 37, peer: 40 }));
    let state = audio_ramps(None, Some(out), 5.0, 40.0);
    // The edited fade-in is resolved alone; the pinned fade-out keeps what the whole gave it.
    assert_eq!(state.fade_in.peer, 0);
    assert_eq!(
        (
            state.fade_out.anchor,
            state.fade_out.far,
            state.fade_out.peer
        ),
        (0, 37, 40)
    );
}

#[test]
fn a_window_is_validated_and_an_unknown_one_is_refused() {
    let picture = |value: Value| FadeWindow::from_json(&value, 10, false);
    assert!(picture(json!({"lead": {"frames": 3, "fade": 6.5}})).is_ok());
    for bad in [
        json!({}),
        json!([]),
        json!({"lead": {"frames": 0, "fade": 1}}),
        json!({"lead": {"frames": -2, "fade": 1}}),
        json!({"lead": {"frames": 1.5, "fade": 1}}),
        json!({"lead": {"frames": 1, "fade": -1}}),
        json!({"lead": {"frames": 1}}),
        json!({"tail": {"frames": 1, "fade": 1, "extra": 1}}),
        json!({"middle": {"frames": 1, "fade": 1}}),
        json!({"lead": {"frames": 1_000_000, "fade": 1}}),
        json!({"lead": {"frames": 1, "fade": 1, "norm": {"far": 0, "peer": 0}}}),
    ] {
        assert!(picture(bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn a_sound_window_needs_a_valid_norm() {
    let sound = |value: Value| FadeWindow::from_json(&value, 10, true);
    let good = json!({"lead": {"frames": 0, "fade": 12, "norm": {"far": 40, "peer": 0}}});
    assert!(sound(good).is_ok());
    assert!(
        sound(json!({"tail": {"frames": 3, "fade": 6, "norm": {"far": -4, "peer": 7}}})).is_ok()
    );
    for bad in [
        json!({"lead": {"frames": 3, "fade": 12}}),
        json!({"lead": {"frames": -1, "fade": 12, "norm": {"far": 0, "peer": 0}}}),
        json!({"lead": {"frames": 3, "fade": 12, "norm": {"far": 0}}}),
        json!({"lead": {"frames": 3, "fade": 12, "norm": {"peer": 0}}}),
        json!({"lead": {"frames": 3, "fade": 12, "norm": {"far": 0.5, "peer": 0}}}),
        json!({"lead": {"frames": 3, "fade": 12, "norm": {"far": 0, "peer": -1}}}),
        json!({"lead": {"frames": 3, "fade": 12, "norm": {"far": 0, "peer": 2_000_000}}}),
        json!({"lead": {"frames": 3, "fade": 12, "norm": {"far": 0, "peer": 0, "x": 1}}}),
        // The span (3 + 10 - 40) is empty or negative: not a ramp group.
        json!({"lead": {"frames": 3, "fade": 12, "norm": {"far": -40, "peer": 0}}}),
        // Longer than the limit.
        json!({"lead": {"frames": 3, "fade": 12, "norm": {"far": 1_000_000, "peer": 0}}}),
        json!({"lead": {"frames": 3, "fade": 12, "norm": {"far": i64::MIN, "peer": 0}}}),
        json!({"lead": {"frames": 3, "fade": 12, "norm": "x"}}),
    ] {
        assert!(sound(bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn a_trim_keeps_the_ramps_in_place_and_drops_an_anchor_that_passes_its_origin() {
    let window = FadeWindow {
        lead: Some(picture(5, 8.0)),
        tail: Some(picture(7, 4.0)),
    };
    let trimmed = window.trimmed(2, -3).unwrap();
    assert_eq!(trimmed.lead.unwrap().frames, 7);
    assert_eq!(trimmed.tail.unwrap().frames, 4);
    let gone = window.trimmed(-5, -7).unwrap();
    assert!(gone.is_empty());
    assert!(window.trimmed(i64::MAX, 0).is_none());
}

#[test]
fn a_sound_trim_moves_the_anchor_and_the_span_and_never_changes_the_other_ramp() {
    let window = FadeWindow {
        lead: Some(sound(5, 8.0, 20, 30)),
        tail: Some(sound(6, 30.0, 5, 8)),
    };
    // A clip of 30 frames: head trimmed 3 inward, end 4 outward (31 frames now).
    let trimmed = window.trimmed(3, -4).unwrap();
    let (lead, tail) = (trimmed.lead.unwrap(), trimmed.tail.unwrap());
    assert_eq!((lead.frames, lead.norm.unwrap().far), (8, 16));
    assert_eq!((tail.frames, tail.norm.unwrap().far), (2, 8));
    assert_eq!(
        (lead.fade, tail.fade),
        (8.0, 30.0),
        "guards are never touched"
    );
    // The span of each group is where it was on the sequence.
    let span = |reach: Reach, duration: i64| reach.frames + duration + reach.norm.unwrap().far;
    assert_eq!(span(window.lead.unwrap(), 30), span(lead, 31));
    assert_eq!(span(window.tail.unwrap(), 30), span(tail, 31));
    assert_eq!(
        trimmed.trimmed(-3, 4).unwrap(),
        window,
        "a trim is exactly undone"
    );
    // The fade-out anchor passes its end: that ramp goes; the fade-in keeps its place and the
    // span of the group the fade-out was part of (the far end is now inside the clip).
    let gone = window.trimmed(0, -7).unwrap();
    assert!(gone.tail.is_none());
    let lead = gone.lead.unwrap();
    assert_eq!(lead.norm.unwrap().far, 13);
    // An anchor exactly on the origin is still a ramp (a sound pin can sit on the edge).
    assert_eq!(window.trimmed(-5, -6).unwrap().lead.unwrap().frames, 0);
    assert!(window.trimmed(i64::MAX, 0).is_none());
}

#[test]
fn only_a_ramp_whose_guard_matches_is_live() {
    let window = FadeWindow {
        lead: Some(picture(5, 8.0)),
        tail: Some(picture(7, 4.0)),
    };
    assert_eq!(window.live(8.0, 4.0), Extent { lead: 5, tail: 7 });
    assert_eq!(window.live(9.0, 4.0), Extent { lead: 0, tail: 7 });
    assert_eq!(window.live(8.0, 0.0), Extent { lead: 5, tail: 0 });
    let (lead, tail) = window.live_reach(8.0, 3.0);
    assert!(lead.is_some() && tail.is_none());
}

#[test]
fn a_window_that_does_not_fit_the_frame_arithmetic_refuses_the_cut() {
    let mut huge = input(false, 4, 20, 12.0, 24.0);
    huge.lead = Some(picture(MAX_LENGTH_FRAMES, 12.0));
    assert!(split_fades(huge).is_none());
    huge.lead = Some(picture(i64::MAX, 12.0));
    huge.tail = Some(picture(1, 24.0));
    assert!(split_fades(huge).is_none());
    let mut huge = input(true, 4, 20, 12.0, 24.0);
    huge.lead = Some(sound(MAX_LENGTH_FRAMES, 12.0, 0, 24));
    assert!(split_fades(huge).is_none());
    huge.lead = Some(sound(i64::MAX, 12.0, 0, 24));
    assert!(split_fades(huge).is_none());
    huge.lead = Some(sound(0, 12.0, i64::MAX, 24));
    assert!(split_fades(huge).is_none());
}

#[test]
fn the_pinned_ramps_of_a_sound_group_as_the_mixer_reads_them() {
    // Plain ramps are one pair.
    let plain = audio_ramps(None, None, 12.0, 30.0);
    assert_eq!(
        plain.fade_in,
        RampGroup {
            own: 12,
            anchor: 0,
            far: 0,
            peer: 30
        }
    );
    assert_eq!(
        plain.fade_out,
        RampGroup {
            own: 30,
            anchor: 0,
            far: 0,
            peer: 12
        }
    );
    // Fractional values are rounded as the mixer rounds them.
    assert_eq!(audio_ramps(None, None, 12.4, 29.6).fade_out.peer, 12);
    assert_eq!(audio_ramps(None, None, 12.4, 29.6).fade_in.peer, 30);
    assert_eq!(audio_ramps(None, None, -3.0, f64::NAN).fade_in.own, 0);
    // A pinned ramp reads its own record.
    let pinned = audio_ramps(Some(sound(4, 12.0, 9, 30)), None, 12.0, 30.0);
    assert_eq!(
        pinned.fade_in,
        RampGroup {
            own: 12,
            anchor: 4,
            far: 9,
            peer: 30
        }
    );
    assert_eq!(
        pinned.fade_out.peer, 0,
        "the fade-in is frozen: no peer for the fade-out"
    );
}
