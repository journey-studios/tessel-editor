//! BL081 (F017): "cut at the playhead" in the live window. The Kit button in the timeline toolbar,
//! the semantic tree (`timeline.split`), `ui invoke` and the `clip.split` op are four entrances to
//! one command; the cutting rules themselves are tested without a window in
//! `crates/core/tests/clip_split.rs`.
use serde_json::Value;
use tessel_editor_core::commands::Parsed;

use super::OpReply;
use super::input_tests::fixture;
use super::*;
use core::prelude::v1::test;

const CLIP: &str = "basalt-cliffs";

fn run(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    command: Command,
) -> Result<Value, CommandError> {
    let outcome = view.update(cx, |view, cx| view.execute(command, Source::Cli, cx));
    settle(cx);
    outcome
}

/// Paints the new state so the probes of the toolbar are the ones of this state.
fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn node(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> Value {
    view.read_with(cx, |view, cx| view.tree_node(id, cx))
        .unwrap_or_else(|| panic!("{id} is not in the tree"))
}

fn invoke(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    target: &str,
) -> Result<Value, CommandError> {
    let parsed = view.read_with(cx, |view, cx| {
        let layout = view.layout_snapshot(cx);
        tessel_editor_core::commands::parse_invoke(&view.state, &layout, target, "invoke")
    })?;
    let Parsed::Run(command) = parsed else {
        panic!("the cut is a state command");
    };
    run(view, cx, command)
}

fn op(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    request: Value,
) -> Result<Value, CommandError> {
    let reply = view.update(cx, |view, cx| view.run_op(&request, cx));
    settle(cx);
    match reply? {
        OpReply::Done(value) => Ok(value),
        OpReply::Pending(_) => panic!("{request} should not wait for a frame"),
    }
}

fn button_center(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Point<Pixels> {
    view.read_with(cx, |view, _| {
        view.geometry
            .get("control:timeline-split")
            .expect("the toolbar paints the cut button")
            .center()
    })
}

fn clips(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |view, _| view.state.timeline.clips.len())
}

fn undo_depth(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |view, _| {
        view.state.history.to_json()["undo_depth"].as_u64().unwrap()
    })
}

fn span(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> (i64, i64) {
    view.read_with(cx, |view, _| {
        let clip = view.state.timeline.clip(id).unwrap();
        (clip.start, clip.end())
    })
}

fn playhead(view: &Entity<Workstation>, cx: &mut VisualTestContext, frame: i64) {
    run(
        view,
        cx,
        Command::SetPlayhead {
            frame: Raw::Number(frame as f64),
        },
    )
    .unwrap();
}

#[gpui_kit::test]
fn the_button_node_and_invoke_agree_on_when_a_cut_is_possible(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    run(
        &view,
        cx,
        Command::SelectClip {
            clip: CLIP.to_string(),
        },
    )
    .unwrap();
    let (start, end) = span(&view, cx, CLIP);
    let depth = undo_depth(&view, cx);
    let count = clips(&view, cx);

    // On an edge a half would be empty: the node, the tooltip and `invoke` all say no, with the
    // reason the core gives.
    for edge in [start, end] {
        playhead(&view, cx, edge);
        let cut = node(&view, cx, "timeline.split");
        assert_eq!(cut["enabled"], false, "frame {edge}");
        assert_eq!(cut["parent"], "region:timeline");
        let reason = cut["disabled_reason"].as_str().unwrap().to_string();
        let refused = invoke(&view, cx, "timeline.split").unwrap_err();
        assert_eq!(refused.code, "disabled_element");
        assert_eq!(
            refused.message, reason,
            "invoke answers with the node's reason"
        );
        // A click on the painted, disabled button runs nothing.
        let at = button_center(&view, cx);
        cx.simulate_click(at, Default::default());
        settle(cx);
        assert_eq!((clips(&view, cx), undo_depth(&view, cx)), (count, depth));
    }

    // With nothing selected there is nothing to cut.
    view.update(cx, |view, _| view.state.timeline.selected_clip = None);
    settle(cx);
    assert_eq!(node(&view, cx, "timeline.split")["enabled"], false);
    assert_eq!(
        invoke(&view, cx, "timeline.split").unwrap_err().code,
        "disabled_element"
    );
}

#[gpui_kit::test]
fn a_click_cuts_the_selected_clip_at_the_playhead_as_one_undoable_edit(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    run(
        &view,
        cx,
        Command::SelectClip {
            clip: CLIP.to_string(),
        },
    )
    .unwrap();
    let (start, end) = span(&view, cx, CLIP);
    let (depth, count) = (undo_depth(&view, cx), clips(&view, cx));
    playhead(&view, cx, start + 5);
    assert_eq!(node(&view, cx, "timeline.split")["enabled"], true);

    let at = button_center(&view, cx);
    cx.simulate_click(at, Default::default());
    settle(cx);
    assert_eq!(clips(&view, cx), count + 1);
    assert_eq!(
        undo_depth(&view, cx),
        depth + 1,
        "one click, one undo entry"
    );
    let right = format!("{CLIP}-split-1");
    assert_eq!(span(&view, cx, CLIP), (start, start + 5));
    assert_eq!(span(&view, cx, &right), (start + 5, end));
    assert_eq!(
        node(&view, cx, &format!("clip:{right}"))["state"]["selected"],
        true,
        "the right half is the selection"
    );
    // The playhead sits on the new edge, so the button is off again, with a reason.
    let cut = node(&view, cx, "timeline.split");
    assert_eq!(cut["enabled"], false);
    assert!(cut["disabled_reason"].is_string());
    let (failed, text) = view.read_with(cx, |view, _| {
        let activity = view.activity.as_ref().unwrap();
        (activity.error, activity.text.clone())
    });
    assert!(!failed, "{text}");

    // Undo and redo are the shared history: the click is one entry in both directions.
    run(&view, cx, Command::Undo).unwrap();
    assert_eq!(clips(&view, cx), count);
    assert_eq!(span(&view, cx, CLIP), (start, end));
    run(&view, cx, Command::Redo).unwrap();
    assert_eq!(clips(&view, cx), count + 1);
    assert_eq!(span(&view, cx, &right), (start + 5, end));
}

#[gpui_kit::test]
fn invoke_and_the_op_run_the_same_cut_and_a_refusal_changes_nothing(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    run(
        &view,
        cx,
        Command::SelectClip {
            clip: CLIP.to_string(),
        },
    )
    .unwrap();
    let (start, end) = span(&view, cx, CLIP);
    let count = clips(&view, cx);

    // `ui invoke timeline.split`: the selected clip at the playhead.
    playhead(&view, cx, start + 3);
    let outcome = invoke(&view, cx, "timeline.split").unwrap();
    assert_eq!(outcome["action"], "split");
    assert_eq!(span(&view, cx, CLIP), (start, start + 3));

    // `ui op clip.split`: an explicit clip and frame, whatever is selected, with a chosen ID.
    let outcome = op(
        &view,
        cx,
        json!({"op":"clip.split","clip":format!("{CLIP}-split-1"),"frame":start + 8,
            "new_id":"third-part"}),
    )
    .unwrap();
    assert_eq!(outcome["new_id"], "third-part");
    assert_eq!(span(&view, cx, "third-part"), (start + 8, end));
    assert_eq!(clips(&view, cx), count + 2);

    // Refused ops are typed and leave the window's state exactly as it was.
    let depth = undo_depth(&view, cx);
    let revision = view.read_with(cx, |view, _| view.state.project.revision);
    for (request, code) in [
        (
            json!({"op":"clip.split","clip":"nope","frame":10}),
            "unknown_element",
        ),
        (
            json!({"op":"clip.split","clip":CLIP,"frame":start}),
            "invalid_duration",
        ),
        (
            json!({"op":"clip.split","clip":CLIP,"frame":end + 100}),
            "out_of_bounds",
        ),
        (
            json!({"op":"clip.split","clip":CLIP,"frame":start as f64 + 1.5}),
            "invalid_request",
        ),
        (
            json!({"op":"clip.split","clip":CLIP,"frame":start + 1,"new_id":"third-part"}),
            "duplicate_id",
        ),
    ] {
        let error = op(&view, cx, request.clone()).unwrap_err();
        assert_eq!(error.code, code, "{request}");
    }
    assert_eq!(undo_depth(&view, cx), depth);
    assert_eq!(
        view.read_with(cx, |view, _| view.state.project.revision),
        revision
    );
    assert_eq!(clips(&view, cx), count + 2);
}

#[gpui_kit::test]
fn audio_on_a_fractional_grid_is_offered_cut_and_keeps_the_sound(cx: &mut TestAppContext) {
    // BL081.5: this cut used to be disabled (`SplitBlock::AudioSampleGrid`). The window asks the
    // core, and the core now carries the sample phase of each half.
    use tessel_editor_core::audio::{AudioBackend, OfflineMixer};
    let (view, cx) = fixture(cx);
    let dir = std::env::temp_dir().join(format!("tessel-split-ui-audio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("tone.wav");
    std::fs::write(
        &wav,
        tessel_editor_core::fixtures::tone_wav(44_100, 2.0, 440.0, 0.5, 2),
    )
    .unwrap();
    view.update(cx, |view, _| {
        for request in [
            json!({"op":"project.create","name":"Ntsc","sequence":"main",
                "rate":{"num":30000,"den":1001},"width":64,"height":36,"length":96}),
            json!({"op":"asset.add","kind":"audio","id":"tone","name":"Tone","path":wav}),
            json!({"op":"track.add","id":"a1","name":"A1","kind":"audio"}),
            json!({"op":"clip.add","id":"snd","track":"a1","asset":"tone","start":1,"duration":47}),
        ] {
            tessel_editor_core::headless::execute(&mut view.state, &request).unwrap();
        }
    });
    settle(cx);
    let mix = |view: &Entity<Workstation>, cx: &mut VisualTestContext| {
        view.read_with(cx, |view, _| {
            let plan = view.state.plan_audio(0, 96).unwrap();
            OfflineMixer::default().mix(&plan).unwrap().samples
        })
    };
    let before = mix(&view, cx);
    assert!(before.iter().any(|sample| sample.abs() > 0.01));
    run(
        &view,
        cx,
        Command::SelectClip {
            clip: "snd".to_string(),
        },
    )
    .unwrap();
    playhead(&view, cx, 2);
    assert_eq!(
        node(&view, cx, "timeline.split")["enabled"],
        true,
        "{}",
        node(&view, cx, "timeline.split")
    );
    let at = button_center(&view, cx);
    cx.simulate_click(at, Default::default());
    settle(cx);
    assert_eq!(span(&view, cx, "snd"), (1, 2));
    assert_eq!(span(&view, cx, "snd-split-1"), (2, 48));
    let after = mix(&view, cx);
    assert_eq!(
        before.len(),
        after.len(),
        "the click preserves every sample"
    );
    assert!(
        before
            .iter()
            .zip(&after)
            .all(|(a, b)| a.to_bits() == b.to_bits()),
        "the click did not change one sample"
    );
}
