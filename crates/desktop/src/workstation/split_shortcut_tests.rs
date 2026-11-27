//! BL081.1.1 (F017): Cmd+B / Ctrl+B cuts the selected clip at the playhead.
//!
//! The key goes through GPUI's real dispatcher (`simulate_keystrokes`, `simulate_event`), not a
//! call to the handler. That is **not** a physical keyboard, and the macOS binding is only
//! covered through the platform policy (`decide` with `Platform::Mac`); no Mac run is claimed.
use serde_json::{Value, json};
use tessel_editor_core::Id;
use tessel_editor_core::audio::{AudioBackend, OfflineMixer};
use tessel_editor_core::commands::Parsed;
use tessel_editor_core::plan::{PlanRequest, PlanTime, PreviewIntent};
use tessel_editor_core::state::split::{SplitAvailability, SplitBlock};

use super::input_tests::fixture;
use super::split_shortcut::{
    Platform, Shortcut, Verdict, current_shortcut, decide, focus_is_in_field, is_split_keystroke,
    shortcut, with_shortcut,
};
use super::*;
use core::prelude::v1::test;

const CLIP: &str = "basalt-cliffs";

// ---- plumbing ---------------------------------------------------------------------------------

fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn run(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    command: Command,
) -> Result<Value, CommandError> {
    let outcome = view.update(cx, |view, cx| view.execute(command, Source::Cli, cx));
    settle(cx);
    outcome
}

fn select(view: &Entity<Workstation>, cx: &mut VisualTestContext, clip: &str) {
    run(
        view,
        cx,
        Command::SelectClip {
            clip: clip.to_string(),
        },
    )
    .unwrap();
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

fn span(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> (i64, i64) {
    view.read_with(cx, |view, _| {
        let clip = view.state.timeline.clip(id).unwrap();
        (clip.start, clip.end())
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

/// Everything a refused or ignored key must leave alone: the document, its revision, the history,
/// the selection and the playhead.
fn domain(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    view.read_with(cx, |view, _| {
        json!({
            "document": view.state.to_document(None),
            "revision": view.state.project.revision,
            "history": view.state.history.to_json(),
            "selected": format!("{:?}", view.state.timeline.selected_clip),
            "playhead": view.state.timeline.playhead,
        })
    })
}

/// The workstation takes the keyboard focus, as `EditorShell` does when the window opens.
fn focus_workstation(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    view.update_in(cx, |view, window, cx| view.focus_root(window, cx));
    settle(cx);
}

fn press(cx: &mut VisualTestContext, keystrokes: &str) {
    cx.simulate_keystrokes(keystrokes);
    settle(cx);
}

fn press_shortcut(cx: &mut VisualTestContext) {
    press(cx, current_shortcut().keystroke);
}

/// `Keystroke::parse` wants a `'static` string; a few leaked test strings are harmless.
fn stroke(text: &str) -> Keystroke {
    Keystroke::parse(Box::leak(text.to_owned().into_boxed_str())).unwrap()
}

fn key_event(keystroke: &str, is_held: bool) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: stroke(keystroke),
        is_held,
        prefer_character_input: false,
    }
}

fn node(view: &Entity<Workstation>, cx: &mut VisualTestContext, id: &str) -> Value {
    view.read_with(cx, |view, cx| view.tree_node(id, cx))
        .unwrap_or_else(|| panic!("{id} is not in the tree"))
}

/// A selected demo clip with the playhead `offset` frames into it, focus on the workstation.
fn ready(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    offset: i64,
) -> ((i64, i64), u64, usize) {
    select(view, cx, CLIP);
    let (start, end) = span(view, cx, CLIP);
    playhead(view, cx, start + offset);
    focus_workstation(view, cx);
    ((start, end), undo_depth(view, cx), clips(view, cx))
}

fn ready_availability() -> SplitAvailability {
    SplitAvailability::Ready {
        clip: Id::new("clip-a").unwrap(),
        frame: 12,
    }
}

// ---- policy: platform row, matching, availability (no window) ---------------------------------

#[test]
fn each_platform_has_one_row_that_the_keystroke_and_the_label_share() {
    let mac = shortcut(Platform::Mac);
    let other = shortcut(Platform::Other);
    assert_eq!(
        mac,
        Shortcut {
            keystroke: "cmd-b",
            label: "⌘B"
        }
    );
    assert_eq!(
        other,
        Shortcut {
            keystroke: "ctrl-b",
            label: "Ctrl+B"
        }
    );
    assert_eq!(
        current_shortcut(),
        if cfg!(target_os = "macos") {
            mac
        } else {
            other
        }
    );
    assert_eq!(
        with_shortcut("Cortar clipe no playhead", Platform::Mac),
        "Cortar clipe no playhead (⌘B)"
    );
    assert_eq!(
        with_shortcut("Cortar clipe no playhead", Platform::Other),
        "Cortar clipe no playhead (Ctrl+B)"
    );
}

#[test]
fn a_platform_matches_only_its_own_exact_keystroke() {
    let accepts = |platform, keystroke: &str| is_split_keystroke(platform, &stroke(keystroke));
    assert!(accepts(Platform::Mac, "cmd-b"));
    assert!(accepts(Platform::Other, "ctrl-b"));
    // The other platform's modifier is not this platform's shortcut.
    assert!(!accepts(Platform::Mac, "ctrl-b"));
    assert!(!accepts(Platform::Other, "cmd-b"));
    for (platform, own) in [(Platform::Mac, "cmd"), (Platform::Other, "ctrl")] {
        for wrong in [
            "b".to_string(),
            format!("shift-{own}-b"),
            format!("alt-{own}-b"),
            format!("ctrl-alt-{own}-b"),
            "alt-b".to_string(),
            "shift-b".to_string(),
            format!("{own}-v"),
            format!("{own}-n"),
            format!("{own}-enter"),
        ] {
            assert!(!accepts(platform, &wrong), "{platform:?} accepted {wrong}");
        }
    }
}

#[test]
fn the_decision_cuts_only_a_fresh_press_with_a_free_field_and_no_dialog() {
    for platform in [Platform::Mac, Platform::Other] {
        let own = shortcut(platform).keystroke;
        assert_eq!(
            decide(
                platform,
                &key_event(own, false),
                false,
                false,
                ready_availability()
            ),
            Verdict::Cut {
                clip: Id::new("clip-a").unwrap(),
                frame: 12
            },
            "{platform:?}"
        );
        // A held key is consumed and never cuts.
        assert_eq!(
            decide(
                platform,
                &key_event(own, true),
                false,
                false,
                ready_availability()
            ),
            Verdict::SwallowRepeat
        );
        // A Kit field with the focus, or a dialog: left alone, held or not.
        for held in [false, true] {
            assert_eq!(
                decide(
                    platform,
                    &key_event(own, held),
                    true,
                    false,
                    ready_availability()
                ),
                Verdict::Ignore
            );
            assert_eq!(
                decide(
                    platform,
                    &key_event(own, held),
                    false,
                    true,
                    ready_availability()
                ),
                Verdict::Ignore
            );
        }
        // Character input (AltGr on Windows) is not a shortcut.
        let mut typed = key_event(own, false);
        typed.prefer_character_input = true;
        assert_eq!(
            decide(platform, &typed, false, false, ready_availability()),
            Verdict::Ignore
        );
        // The core says no: nothing to run, whatever the reason.
        for block in [
            SplitBlock::NoSelection,
            SplitBlock::PlayheadOutside,
            SplitBlock::Refused,
        ] {
            assert_eq!(
                decide(
                    platform,
                    &key_event(own, false),
                    false,
                    false,
                    SplitAvailability::Blocked(block)
                ),
                Verdict::Ignore
            );
        }
    }
    // Another platform's key never cuts.
    assert_eq!(
        decide(
            Platform::Mac,
            &key_event("ctrl-b", false),
            false,
            false,
            ready_availability()
        ),
        Verdict::Ignore
    );
}

fn shortcut_binding_exists(cx: &App, platform: Platform) -> bool {
    let stroke = Keystroke::parse(shortcut(platform).keystroke).unwrap();
    cx.key_bindings().borrow().bindings().any(|binding| {
        binding
            .match_keystrokes(std::slice::from_ref(&stroke))
            .is_some()
    })
}

#[gpui_kit::test]
fn the_keymap_has_no_binding_that_the_shortcut_would_collide_with(cx: &mut TestAppContext) {
    let (_view, cx) = fixture(cx);
    cx.update(|_, cx| {
        for platform in [Platform::Mac, Platform::Other] {
            assert!(
                !shortcut_binding_exists(cx, platform),
                "{platform:?} {} is already bound, including as a chord prefix",
                shortcut(platform).keystroke
            );
        }
        // Control: the complete-input lookup misses a prefix, but our collision guard must not.
        cx.bind_keys([KeyBinding::new(
            "cmd-b x",
            super::perf_meter::TogglePerfMeter,
            Some("Workstation"),
        )]);
        assert!(cx.all_bindings_for_input(&[stroke("cmd-b")]).is_empty());
        assert!(shortcut_binding_exists(cx, Platform::Mac));
    });
}

// ---- the real dispatcher ----------------------------------------------------------------------

#[gpui_kit::test]
fn the_key_cuts_the_selected_clip_at_the_playhead_as_one_undoable_edit(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let ((start, end), depth, count) = ready(&view, cx, 5);
    assert_eq!(node(&view, cx, "timeline.split")["enabled"], true);

    press_shortcut(cx);

    let right = format!("{CLIP}-split-1");
    assert_eq!(clips(&view, cx), count + 1);
    assert_eq!(span(&view, cx, CLIP), (start, start + 5));
    assert_eq!(span(&view, cx, &right), (start + 5, end));
    assert_eq!(undo_depth(&view, cx), depth + 1, "one key, one undo entry");
    assert_eq!(
        node(&view, cx, &format!("clip:{right}"))["state"]["selected"],
        true,
        "the right half is the selection, as for the button"
    );
    let (failed, text) = view.read_with(cx, |view, _| {
        let activity = view.activity.as_ref().unwrap();
        (activity.error, activity.text.clone())
    });
    assert!(!failed, "{text}");

    // Undo and redo are the shared history.
    run(&view, cx, Command::Undo).unwrap();
    assert_eq!(clips(&view, cx), count);
    assert_eq!(span(&view, cx, CLIP), (start, end));
    assert_eq!(undo_depth(&view, cx), depth);
    run(&view, cx, Command::Redo).unwrap();
    assert_eq!(clips(&view, cx), count + 1);
    assert_eq!(span(&view, cx, &right), (start + 5, end));
    assert_eq!(undo_depth(&view, cx), depth + 1);
}

#[gpui_kit::test]
fn selection_and_playhead_are_read_when_the_key_is_pressed(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let ((start, end), depth, _) = ready(&view, cx, 5);
    // The window was drawn with this state; the state then changes without a repaint before
    // the key (what a playback tick or the CLI does).
    view.update(cx, |view, _| view.state.timeline.playhead = start + 9);
    press_shortcut(cx);
    assert_eq!(
        span(&view, cx, CLIP),
        (start, start + 9),
        "the live playhead"
    );

    // The right half is the selection now; a later press cuts *it*, at the new playhead.
    view.update(cx, |view, _| view.state.timeline.playhead = start + 20);
    press_shortcut(cx);
    let first = format!("{CLIP}-split-1");
    assert_eq!(span(&view, cx, &first), (start + 9, start + 20));
    assert_eq!(
        span(&view, cx, &format!("{first}-split-1")),
        (start + 20, end)
    );
    assert_eq!(undo_depth(&view, cx), depth + 2, "two presses, two entries");
}

#[gpui_kit::test]
fn the_node_publishes_the_binding_the_window_really_listens_for(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    ready(&view, cx, 5);
    let cut = node(&view, cx, "timeline.split");
    let row = current_shortcut();
    assert_eq!(cut["shortcut"]["keystroke"], row.keystroke);
    assert_eq!(cut["shortcut"]["label"], row.label);
    assert_eq!(cut["shortcut"]["scope"], "Workstation");
    assert_eq!(cut["shortcut"]["repeat"], "ignored");
    // The core's own node is untouched by the metadata: enabled/reason still come from it.
    assert_eq!(cut["enabled"], true);
    assert!(
        view.read_with(cx, |view, cx| view.tree_nodes(cx))
            .iter()
            .filter(|node| node.get("shortcut").is_some())
            .all(|node| node["id"] == "timeline.split"),
        "only the cut carries the metadata"
    );
}

// ---- negatives: nothing in the domain changes -------------------------------------------------

#[gpui_kit::test]
fn without_a_selection_or_a_cuttable_playhead_the_key_changes_nothing(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let ((start, end), ..) = ready(&view, cx, 5);

    for frame in [start, end, start - 1, end + 25] {
        playhead(&view, cx, frame);
        assert_eq!(node(&view, cx, "timeline.split")["enabled"], false);
        let before = domain(&view, cx);
        press_shortcut(cx);
        assert_eq!(domain(&view, cx), before, "playhead {frame}");
    }

    playhead(&view, cx, start + 5);
    view.update(cx, |view, _| view.state.timeline.selected_clip = None);
    settle(cx);
    assert_eq!(node(&view, cx, "timeline.split")["enabled"], false);
    let before = domain(&view, cx);
    press_shortcut(cx);
    assert_eq!(domain(&view, cx), before, "no selection");
}

#[gpui_kit::test]
fn another_key_or_other_modifiers_do_not_cut(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    ready(&view, cx, 5);
    let own = current_shortcut().keystroke;
    let before = domain(&view, cx);
    let (modifier, other_modifier) = if cfg!(target_os = "macos") {
        ("cmd", "ctrl")
    } else {
        ("ctrl", "cmd")
    };
    assert!(own.starts_with(modifier));
    for keys in [
        "b".to_string(),
        "enter".to_string(),
        format!("{modifier}-v"),
        format!("{modifier}-n"),
        format!("shift-{modifier}-b"),
        format!("alt-{modifier}-b"),
        format!("{other_modifier}-b"),
        "alt-b".to_string(),
        "shift-b".to_string(),
    ] {
        press(cx, &keys);
        assert_eq!(domain(&view, cx), before, "{keys}");
    }
    // Typing the letter is not a cut either.
    cx.simulate_input("b");
    settle(cx);
    assert_eq!(domain(&view, cx), before, "typed b");
}

#[gpui_kit::test]
fn a_held_key_never_cuts_again_even_when_the_playhead_moves(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let ((start, _), depth, count) = ready(&view, cx, 5);

    // Held from before the press reached the workstation (focus arrived mid-hold): no cut.
    cx.simulate_event(key_event(current_shortcut().keystroke, true));
    settle(cx);
    assert_eq!((clips(&view, cx), undo_depth(&view, cx)), (count, depth));

    press_shortcut(cx);
    assert_eq!(
        (clips(&view, cx), undo_depth(&view, cx)),
        (count + 1, depth + 1)
    );

    // Playback moves the playhead inside the selected right half while the key is held: the
    // cut would be available, the repeat still does nothing.
    playhead(&view, cx, start + 20);
    assert_eq!(node(&view, cx, "timeline.split")["enabled"], true);
    let before = domain(&view, cx);
    for _ in 0..3 {
        cx.simulate_event(key_event(current_shortcut().keystroke, true));
        settle(cx);
    }
    assert_eq!(domain(&view, cx), before, "key repeat");

    // A new press (is_held false) is a new cut.
    press_shortcut(cx);
    assert_eq!(undo_depth(&view, cx), depth + 2);
}

#[gpui_kit::test]
fn a_dialog_blocks_the_key_and_closing_it_restores_it(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let (_, depth, count) = ready(&view, cx, 5);
    view.update(cx, |view, _| view.workflow.pretend_dialog_open(true));
    settle(cx);
    let before = domain(&view, cx);
    press_shortcut(cx);
    assert_eq!(domain(&view, cx), before);

    view.update(cx, |view, _| view.workflow.pretend_dialog_open(false));
    settle(cx);
    press_shortcut(cx);
    assert_eq!(
        (clips(&view, cx), undo_depth(&view, cx)),
        (count + 1, depth + 1)
    );
}

#[gpui_kit::test]
fn a_focused_kit_input_keeps_its_text_and_its_own_shortcuts(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    ready(&view, cx, 5);
    let search = view.read_with(cx, |view, _| view.search.clone());
    cx.update(|window, app| {
        search.update(app, |input, cx| input.focus(window, cx));
    });
    settle(cx);
    assert!(
        cx.update(|window, _| focus_is_in_field(&window.context_stack())),
        "the precondition: the focus path is inside the Kit Input"
    );

    cx.simulate_input("ab");
    settle(cx);
    assert_eq!(
        search.read_with(cx, |input, _| input.value().to_string()),
        "ab"
    );
    let before = domain(&view, cx);

    press_shortcut(cx);
    assert_eq!(domain(&view, cx), before, "the Input swallowed no cut");
    assert_eq!(
        search.read_with(cx, |input, _| input.value().to_string()),
        "ab",
        "the text is preserved"
    );

    // The Input's own editing shortcut is not stolen: select-all then typing replaces the text.
    press(
        cx,
        if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        },
    );
    cx.simulate_input("z");
    settle(cx);
    assert_eq!(
        search.read_with(cx, |input, _| input.value().to_string()),
        "z"
    );
    // Held or not, still no cut while the field has the focus.
    let before = domain(&view, cx);
    cx.simulate_event(key_event(current_shortcut().keystroke, true));
    settle(cx);
    assert_eq!(domain(&view, cx), before);
}

#[gpui_kit::test]
fn a_focused_sequence_select_blocks_the_shortcut_until_focus_leaves(cx: &mut TestAppContext) {
    let (view, cx) = fixture(cx);
    let (_, depth, count) = ready(&view, cx, 5);
    let select = view.read_with(cx, |view, _| view.sequence_select.clone());
    cx.update(|window, app| select.update(app, |select, cx| select.focus(window, cx)));
    settle(cx);
    assert!(cx.update(|window, _| {
        window
            .context_stack()
            .iter()
            .any(|context| context.contains("Select"))
    }));
    assert_eq!(node(&view, cx, "timeline.split")["enabled"], true);
    let before = domain(&view, cx);
    press_shortcut(cx);
    cx.simulate_event(key_event(current_shortcut().keystroke, true));
    settle(cx);
    assert_eq!(domain(&view, cx), before);

    focus_workstation(&view, cx);
    press_shortcut(cx);
    assert_eq!(
        (clips(&view, cx), undo_depth(&view, cx)),
        (count + 1, depth + 1)
    );
}

#[gpui_kit::test]
fn the_production_root_and_shell_route_a_descendant_panel_key_to_the_workstation(
    cx: &mut TestAppContext,
) {
    boot(cx);
    let mut shell = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| crate::shell::EditorShell::new(window, cx));
        shell = Some(view.clone());
        gpui_kit::base::Root::new(view, window, cx)
    });
    settle(cx);
    let view = shell
        .unwrap()
        .read_with(cx, |shell, _| shell.workstation.clone());
    let (_, depth, count) = ready(&view, cx, 5);
    let panel = view.read_with(cx, |view, _| view.dock.views()[0].clone());
    let root_focus = view.read_with(cx, |view, _| view.focus.clone());
    cx.update(|window, app| panel.read(app).focus_handle(app).focus(window, app));
    settle(cx);
    assert!(cx.update(|window, _| {
        window
            .context_stack()
            .iter()
            .any(|context| context.contains("Workstation"))
    }));
    assert!(cx.update(|window, app| panel.read(app).focus_handle(app).is_focused(window)));
    assert!(cx.update(|window, _| !root_focus.is_focused(window)));
    press_shortcut(cx);
    assert_eq!(
        (clips(&view, cx), undo_depth(&view, cx)),
        (count + 1, depth + 1)
    );
}

// ---- same cut from the button, the invoke and the key; also the library -----------------------

#[derive(Debug, PartialEq)]
struct Outcome {
    spans: Vec<(String, i64, i64)>,
    plans: Vec<Value>,
    pcm: Vec<u32>,
}

fn outcome(state: &EditorState) -> Outcome {
    let mut spans: Vec<_> = state
        .timeline
        .clips
        .iter()
        .map(|clip| (clip.id.to_string(), clip.start, clip.end()))
        .collect();
    spans.sort();
    let plans = (0..96)
        .map(|frame| {
            state
                .plan(&PlanRequest {
                    intent: PreviewIntent::Sequence,
                    at: PlanTime::Frame(frame),
                })
                .map(|plan| plan.to_json())
                .unwrap_or_else(|error| json!({"error": error.code}))
        })
        .collect();
    let pcm = OfflineMixer::default()
        .mix(&state.plan_audio(0, 96).unwrap())
        .unwrap()
        .samples
        .iter()
        .map(|sample| sample.to_bits())
        .collect();
    Outcome { spans, plans, pcm }
}

fn assert_same(label: &str, left: &Outcome, right: &Outcome) {
    assert_eq!(left.spans, right.spans, "{label}: spans");
    assert_eq!(left.plans, right.plans, "{label}: plans");
    assert_eq!(left.pcm.len(), right.pcm.len(), "{label}: PCM length");
    let first = left.pcm.iter().zip(&right.pcm).position(|(a, b)| a != b);
    assert_eq!(first, None, "{label}: first PCM bit difference");
}

/// NTSC (30000/1001: a fractional sample grid), a tone and a title, both with fades that cross
/// the cut at frame 10: the audio clip `snd` (1..48, fades 20/20) and the title `t` (6..66,
/// fades 12/24).
fn requests(wav: &std::path::Path) -> Vec<Value> {
    vec![
        json!({"op":"project.create","name":"Ntsc","sequence":"main",
            "rate":{"num":30000,"den":1001},"width":64,"height":36,"length":96}),
        json!({"op":"asset.add","kind":"audio","id":"tone","name":"Tone","path":wav}),
        json!({"op":"asset.add","kind":"text","id":"label","name":"Label",
            "text":{"content":"TESSEL","size":14,"color":"#ffffff"}}),
        json!({"op":"track.add","id":"a1","name":"A1","kind":"audio"}),
        json!({"op":"track.add","id":"vt","name":"VT","kind":"video"}),
        json!({"op":"clip.add","id":"snd","track":"a1","asset":"tone","start":1,"duration":47}),
        json!({"op":"clip.add","id":"t","track":"vt","asset":"label","start":6,"duration":60,
            "properties":{"opacity":80}}),
        json!({"op":"clip.property","clip":"snd","property":"fade_in","value":20}),
        json!({"op":"clip.property","clip":"snd","property":"fade_out","value":20}),
        json!({"op":"clip.property","clip":"t","property":"fade_in","value":12}),
        json!({"op":"clip.property","clip":"t","property":"fade_out","value":24}),
    ]
}

fn wav() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-split-shortcut-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("tone.wav");
    std::fs::write(
        &wav,
        tessel_editor_core::fixtures::tone_wav(44_100, 2.0, 440.0, 0.5, 2),
    )
    .unwrap();
    wav
}

/// A window that holds the project above, with `clip` selected and the playhead on `frame`.
fn project_window(
    cx: &mut TestAppContext,
    wav: &std::path::Path,
    clip: &str,
    frame: i64,
) -> (Entity<Workstation>, &'static mut VisualTestContext) {
    let (view, vcx) = window(cx);
    view.update(vcx, |view, _| {
        for request in requests(wav) {
            tessel_editor_core::headless::execute(&mut view.state, &request).unwrap();
        }
    });
    settle(vcx);
    select(&view, vcx, clip);
    playhead(&view, vcx, frame);
    focus_workstation(&view, vcx);
    (view, vcx)
}

fn boot(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_perf_meter(cx);
        crate::theme::apply(cx, true);
    });
}

/// One more window with its own `Workstation`, on a context that outlives the call so several can
/// be driven at once.
fn window(cx: &mut TestAppContext) -> (Entity<Workstation>, &'static mut VisualTestContext) {
    let handle = cx.open_window(size(px(1024.), px(768.)), Workstation::new);
    let view = handle.root(cx).unwrap();
    let vcx = VisualTestContext::from_window(handle.into(), cx).into_mut();
    vcx.run_until_parked();
    (view, vcx)
}

fn button_center(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Point<Pixels> {
    view.read_with(cx, |view, _| {
        view.geometry
            .get("control:timeline-split")
            .expect("the toolbar paints the cut button")
            .center()
    })
}

fn invoke(view: &Entity<Workstation>, cx: &mut VisualTestContext) {
    let parsed = view
        .read_with(cx, |view, cx| {
            let layout = view.layout_snapshot(cx);
            tessel_editor_core::commands::parse_invoke(
                &view.state,
                &layout,
                "timeline.split",
                "invoke",
            )
        })
        .unwrap();
    let Parsed::Run(command) = parsed else {
        panic!("the cut is a state command");
    };
    run(view, cx, command).unwrap();
}

#[gpui_kit::test]
fn button_invoke_key_and_library_make_the_same_cut_inside_fades_on_a_fractional_grid(
    cx: &mut TestAppContext,
) {
    boot(cx);
    let wav = wav();
    for clip in ["snd", "t"] {
        // The library: the explicit op, no window and no selection.
        let mut library = EditorState::demo();
        for request in requests(&wav) {
            tessel_editor_core::headless::execute(&mut library, &request).unwrap();
        }
        let uncut = outcome(&library);
        tessel_editor_core::headless::execute(
            &mut library,
            &json!({"op":"clip.split","clip":clip,"frame":10}),
        )
        .unwrap();
        let by_library = outcome(&library);

        let mut windows = Vec::new();
        for path in ["button", "invoke", "key"] {
            let (view, vcx) = project_window(cx, &wav, clip, 10);
            assert_eq!(node(&view, vcx, "timeline.split")["enabled"], true);
            let depth = undo_depth(&view, vcx);
            match path {
                "button" => {
                    let at = button_center(&view, vcx);
                    vcx.simulate_click(at, Default::default());
                    settle(vcx);
                }
                "invoke" => invoke(&view, vcx),
                _ => press_shortcut(vcx),
            }
            assert_eq!(undo_depth(&view, vcx), depth + 1, "{clip} by {path}");
            assert_eq!(span(&view, vcx, clip).1, 10, "{clip} by {path}");
            windows.push((path, view, vcx));
        }

        let results: Vec<_> = windows
            .iter()
            .map(|(path, view, vcx)| {
                let state = view.read_with(*vcx, |view, _| outcome(&view.state));
                let mut document = view.read_with(*vcx, |view, _| view.state.to_document(None));
                // The project identity is minted per project (`p-<random>`); nothing else may differ.
                document["project"]["id"] = Value::Null;
                (*path, state, document)
            })
            .collect();
        for (path, state, document) in &results {
            assert_same(&format!("{clip}: {path} vs library"), state, &by_library);
            assert_eq!(
                document, &results[0].2,
                "{clip}: the saved document of {path} differs from the button's"
            );
            // The cut itself changed no sample (BL081.2/BL081.5), through every entrance.
            assert_eq!(state.pcm, uncut.pcm, "{clip}: {path} changed the PCM");
        }
    }
}

// ---- scope: two editors, only the right one receives the key ----------------------------------

#[gpui_kit::test]
fn only_the_focused_editor_receives_the_key_with_two_windows(cx: &mut TestAppContext) {
    boot(cx);
    let (first, first_cx) = window(cx);
    let (second, second_cx) = window(cx);
    let (_, first_depth, first_count) = ready(&first, first_cx, 5);
    let (_, second_depth, second_count) = ready(&second, second_cx, 7);

    // A key typed in the first window cuts the first workstation only.
    press_shortcut(first_cx);
    assert_eq!(
        (clips(&first, first_cx), undo_depth(&first, first_cx)),
        (first_count + 1, first_depth + 1)
    );
    assert_eq!(
        (clips(&second, second_cx), undo_depth(&second, second_cx)),
        (second_count, second_depth),
        "the other editor saw nothing"
    );

    // The second window without the keyboard focus on its workstation does not take it either.
    second_cx.update(|window, cx| window.blur(cx));
    settle(second_cx);
    press_shortcut(second_cx);
    assert_eq!(
        (clips(&second, second_cx), undo_depth(&second, second_cx)),
        (second_count, second_depth),
        "a window whose workstation has no focus is outside the scope"
    );

    // Given the focus, it cuts once, and the first one is still where it was.
    focus_workstation(&second, second_cx);
    press_shortcut(second_cx);
    assert_eq!(
        (clips(&second, second_cx), undo_depth(&second, second_cx)),
        (second_count + 1, second_depth + 1)
    );
    assert_eq!(
        (clips(&first, first_cx), undo_depth(&first, first_cx)),
        (first_count + 1, first_depth + 1)
    );
}
