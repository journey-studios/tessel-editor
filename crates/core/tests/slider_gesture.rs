//! BL062 (F005/F024): a slider drag is ONE undoable edit, through the public library API and
//! the JSON executor (no window).
//!
//! The contract under test: while a gesture is open every value is applied at once (live
//! preview) but nothing is recorded; the commit records one entry from the original value to
//! the final one; cancel and "released where it started" record nothing; redo survives until
//! a commit actually records. The pointer part (a real Kit slider drag) is covered by
//! `crates/desktop/src/workstation/slider_gesture_tests.rs`.

use serde_json::{Value, json};
use tessel_editor_core::state::inspector::{
    PROPERTIES, PropertyKind, Value as PropertyValue, property_def,
};
use tessel_editor_core::state::property_gesture::GestureCommand;
use tessel_editor_core::{Command, CommandError, EditorState, Raw, headless};

const CLIP: &str = "atlantic-coast";

fn gesture(state: &mut EditorState, command: GestureCommand) -> Result<Value, CommandError> {
    state.execute(Command::PropertyGesture(command))
}

/// Opens a gesture on the selected clip and returns its ID.
fn begin(state: &mut EditorState, property: &str) -> u64 {
    let outcome = gesture(
        state,
        GestureCommand::Begin {
            clip: None,
            property: property.to_string(),
        },
    )
    .unwrap_or_else(|error| panic!("begin {property}: {error:?}"));
    outcome["gesture"].as_u64().expect("begin returns the ID")
}

fn update(state: &mut EditorState, id: u64, value: f64) -> Value {
    gesture(
        state,
        GestureCommand::Update {
            gesture: Some(id),
            value: Raw::Number(value),
        },
    )
    .unwrap_or_else(|error| panic!("update {value}: {error:?}"))
}

fn commit(state: &mut EditorState) -> Value {
    gesture(state, GestureCommand::Commit { gesture: None }).unwrap()
}

fn cancel(state: &mut EditorState) -> Value {
    gesture(state, GestureCommand::Cancel { gesture: None }).unwrap()
}

fn set(state: &mut EditorState, property: &str, value: f64) {
    state
        .execute(Command::SetProperty {
            property: property.to_string(),
            value: Raw::Number(value),
        })
        .unwrap();
}

fn number(state: &EditorState, clip: &str, property: &str) -> f64 {
    let definition = property_def(property).unwrap();
    match state.inspector.value(clip, definition) {
        PropertyValue::Number(number) => number,
        other => panic!("`{property}` is not numeric: {other:?}"),
    }
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

fn redo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["redo_depth"].as_u64().unwrap()
}

fn undo(state: &mut EditorState) -> Value {
    state.execute(Command::Undo).unwrap()
}

fn redo(state: &mut EditorState) -> Value {
    state.execute(Command::Redo).unwrap()
}

/// Baseline of the reported bug: without a gesture, N slider values are N undo entries.
#[test]
fn slider_gesture_baseline_one_entry_per_discrete_value() {
    let mut state = EditorState::demo();
    for value in 101..=120 {
        set(&mut state, "zoom", f64::from(value));
    }
    assert_eq!(undo_depth(&state), 20, "this is the bug a gesture removes");
}

#[test]
fn slider_gesture_many_updates_record_one_undo_entry() {
    let mut state = EditorState::demo();
    let id = begin(&mut state, "zoom");
    for value in 101..=120 {
        update(&mut state, id, f64::from(value));
    }
    let outcome = commit(&mut state);

    assert_eq!(outcome["recorded"], true);
    assert_eq!(undo_depth(&state), 1, "20 values, one entry");
    assert_eq!(
        state.history.undo_label(),
        Some("Set property"),
        "a drag looks like a discrete set in the undo list"
    );
    assert_eq!(number(&state, CLIP, "zoom"), 120.0);

    undo(&mut state);
    assert_eq!(
        number(&state, CLIP, "zoom"),
        100.0,
        "undo returns to the start"
    );
    assert_eq!(undo_depth(&state), 0);
    redo(&mut state);
    assert_eq!(
        number(&state, CLIP, "zoom"),
        120.0,
        "redo returns to the end"
    );
}

#[test]
fn slider_gesture_commit_matches_a_discrete_set_in_the_document() {
    let mut dragged = EditorState::demo();
    let id = begin(&mut dragged, "opacity");
    for value in [90.0, 70.0, 45.0] {
        update(&mut dragged, id, value);
    }
    commit(&mut dragged);

    let mut typed = EditorState::demo();
    set(&mut typed, "opacity", 45.0);

    assert_eq!(
        dragged.to_document(None)["clip_properties"],
        typed.to_document(None)["clip_properties"]
    );
    assert_eq!(undo_depth(&dragged), undo_depth(&typed));
    assert_eq!(dragged.history.undo_label(), typed.history.undo_label());
}

#[test]
fn slider_gesture_preview_is_live_but_not_in_the_history() {
    let mut state = EditorState::demo();
    state.project.saved_revision = Some(state.project.revision);
    let id = begin(&mut state, "zoom");
    assert!(!state.project.is_dirty(), "beginning changes nothing");

    update(&mut state, id, 140.0);
    assert_eq!(
        number(&state, CLIP, "zoom"),
        140.0,
        "preview is applied at once"
    );
    update(&mut state, id, 160.0);
    assert_eq!(number(&state, CLIP, "zoom"), 160.0);
    assert_eq!(undo_depth(&state), 0, "no entry while previewing");
    assert!(state.project.is_dirty(), "the preview is an unsaved change");
    assert_eq!(
        state.history.to_json()["gesture"]["property"],
        "property:zoom"
    );
    assert_eq!(state.open_property_gesture().unwrap().id, id);

    commit(&mut state);
    assert!(state.open_property_gesture().is_none());
    assert_eq!(state.history.to_json()["gesture"], Value::Null);
}

#[test]
fn slider_gesture_preview_keeps_redo_until_a_commit_records() {
    let mut state = EditorState::demo();
    set(&mut state, "zoom", 150.0);
    undo(&mut state);
    assert_eq!(redo_depth(&state), 1);

    let id = begin(&mut state, "opacity");
    update(&mut state, id, 60.0);
    assert_eq!(redo_depth(&state), 1, "redo survives the preview");
    cancel(&mut state);
    assert_eq!(redo_depth(&state), 1, "and a cancel");

    let id = begin(&mut state, "opacity");
    update(&mut state, id, 60.0);
    commit(&mut state);
    assert_eq!(
        redo_depth(&state),
        0,
        "a recorded edit clears redo, like any edit"
    );
}

#[test]
fn slider_gesture_cancel_restores_the_original_without_an_entry() {
    let mut state = EditorState::demo();
    set(&mut state, "zoom", 150.0);
    let depth = undo_depth(&state);

    let id = begin(&mut state, "zoom");
    update(&mut state, id, 220.0);
    update(&mut state, id, 310.0);
    let outcome = cancel(&mut state);

    assert_eq!(outcome["changed"], true);
    assert_eq!(outcome["recorded"], false);
    assert_eq!(
        number(&state, CLIP, "zoom"),
        150.0,
        "back to the value at begin"
    );
    assert_eq!(undo_depth(&state), depth);
    assert!(state.open_property_gesture().is_none());
}

#[test]
fn slider_gesture_cancel_of_a_default_value_drops_the_override() {
    let mut state = EditorState::demo();
    let before = state.to_document(None)["clip_properties"].clone();
    let id = begin(&mut state, "rotation");
    update(&mut state, id, 33.0);
    cancel(&mut state);
    assert_eq!(state.to_document(None)["clip_properties"], before);
    assert_eq!(undo_depth(&state), 0);
}

#[test]
fn slider_gesture_released_without_change_records_nothing() {
    let mut state = EditorState::demo();
    begin(&mut state, "zoom");
    let outcome = commit(&mut state);
    assert_eq!(outcome["changed"], false);
    assert_eq!(outcome["recorded"], false);
    assert_eq!(undo_depth(&state), 0, "no empty step");
}

#[test]
fn slider_gesture_returning_to_the_original_records_nothing() {
    let mut state = EditorState::demo();
    set(&mut state, "zoom", 130.0);
    let depth = undo_depth(&state);
    let id = begin(&mut state, "zoom");
    update(&mut state, id, 200.0);
    update(&mut state, id, 130.0);
    let outcome = commit(&mut state);
    assert_eq!(outcome["recorded"], false);
    assert_eq!(undo_depth(&state), depth);
    assert_eq!(number(&state, CLIP, "zoom"), 130.0);
}

#[test]
fn slider_gesture_keeps_the_validation_of_set_property() {
    let mut state = EditorState::demo();
    let id = begin(&mut state, "zoom");

    let clamped = update(&mut state, id, 9999.0);
    assert_eq!(clamped["value"], 400.0, "out of range is clamped");
    assert_eq!(clamped["clamped"], true);

    let error = gesture(
        &mut state,
        GestureCommand::Update {
            gesture: Some(id),
            value: Raw::Text("wide".into()),
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "invalid_value");
    assert_eq!(
        number(&state, CLIP, "zoom"),
        400.0,
        "a rejected value changes nothing"
    );
    assert!(
        state.open_property_gesture().is_some(),
        "and does not end the gesture"
    );

    update(&mut state, id, 120.04);
    assert_eq!(
        number(&state, CLIP, "zoom"),
        120.0,
        "rounded to the property's decimals"
    );
    commit(&mut state);
    assert_eq!(undo_depth(&state), 1);
}

#[test]
fn slider_gesture_begin_is_rejected_for_what_set_property_would_reject() {
    let mut state = EditorState::demo();
    let reject = |state: &mut EditorState, clip: Option<&str>, property: &str| {
        gesture(
            state,
            GestureCommand::Begin {
                clip: clip.map(str::to_string),
                property: property.to_string(),
            },
        )
        .unwrap_err()
    };
    assert_eq!(reject(&mut state, None, "blend").code, "not_applicable");
    assert_eq!(reject(&mut state, None, "ken_burns").code, "not_applicable");
    assert_eq!(reject(&mut state, None, "nope").code, "unknown_element");
    assert_eq!(
        reject(&mut state, Some("ghost"), "zoom").code,
        "unknown_element"
    );
    assert_eq!(
        reject(&mut state, Some("score-clip"), "zoom").code,
        "not_applicable",
        "an audio clip has no zoom"
    );
    // `ken_burns` is a toggle: it takes a boolean, set the way the UI switch does.
    state
        .execute(Command::SetProperty {
            property: "ken_burns".into(),
            value: Raw::Bool(false),
        })
        .unwrap();
    assert_eq!(
        reject(&mut state, None, "end_zoom").code,
        "disabled_element",
        "end_zoom requires Ken Burns"
    );
    state.timeline.selected_clip = None;
    assert_eq!(reject(&mut state, None, "zoom").code, "no_selection");
    assert!(state.open_property_gesture().is_none());
}

#[test]
fn slider_gesture_a_rejected_begin_leaves_the_open_gesture_alone() {
    let mut state = EditorState::demo();
    let id = begin(&mut state, "zoom");
    update(&mut state, id, 150.0);
    let error = gesture(
        &mut state,
        GestureCommand::Begin {
            clip: None,
            property: "blend".into(),
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "not_applicable");
    assert_eq!(state.open_property_gesture().unwrap().id, id);
    assert_eq!(undo_depth(&state), 0);
}

#[test]
fn slider_gesture_stays_on_its_clip_when_the_selection_moves() {
    let mut state = EditorState::demo();
    let id = begin(&mut state, "zoom");
    update(&mut state, id, 130.0);

    state
        .execute(Command::SelectClip {
            clip: "river-detail".into(),
        })
        .unwrap();
    assert!(
        state.open_property_gesture().is_some(),
        "selection is presentation: it does not end the gesture"
    );
    update(&mut state, id, 170.0);

    assert_eq!(
        number(&state, CLIP, "zoom"),
        170.0,
        "the locked clip got the value"
    );
    assert_eq!(
        number(&state, "river-detail", "zoom"),
        100.0,
        "the newly selected clip is never touched"
    );
    commit(&mut state);
    undo(&mut state);
    assert_eq!(number(&state, CLIP, "zoom"), 100.0);
}

#[test]
fn slider_gesture_another_edit_settles_it_first() {
    let mut state = EditorState::demo();
    let id = begin(&mut state, "zoom");
    update(&mut state, id, 120.0);

    state
        .execute(Command::SetClipProperty {
            clip: "river-detail".into(),
            property: "opacity".into(),
            value: Raw::Number(50.0),
        })
        .unwrap();

    assert!(state.open_property_gesture().is_none());
    assert_eq!(undo_depth(&state), 2, "the gesture, then the edit");
    undo(&mut state);
    assert_eq!(number(&state, "river-detail", "opacity"), 100.0);
    assert_eq!(
        number(&state, CLIP, "zoom"),
        120.0,
        "the gesture kept its value"
    );
    undo(&mut state);
    assert_eq!(number(&state, CLIP, "zoom"), 100.0);
}

#[test]
fn slider_gesture_undo_and_redo_mid_drag_settle_it() {
    let mut state = EditorState::demo();
    set(&mut state, "opacity", 40.0);
    undo(&mut state);
    assert_eq!(redo_depth(&state), 1);

    let id = begin(&mut state, "zoom");
    update(&mut state, id, 180.0);
    // Redo mid-drag: the settled gesture is a new edit, so there is nothing to redo.
    let error = state.execute(Command::Redo).unwrap_err();
    assert_eq!(error.code, "nothing_to_redo");
    assert!(state.open_property_gesture().is_none());
    assert_eq!(undo_depth(&state), 1);

    let id = begin(&mut state, "zoom");
    update(&mut state, id, 220.0);
    let undone = undo(&mut state);
    assert_eq!(undone["action"], "history.undo");
    assert_eq!(
        number(&state, CLIP, "zoom"),
        180.0,
        "undo mid-drag reverts exactly the drag that was previewing"
    );
    assert_eq!(redo_depth(&state), 1);
}

#[test]
fn slider_gesture_batch_transaction_settles_it_first() {
    let mut state = EditorState::demo();
    let id = begin(&mut state, "zoom");
    update(&mut state, id, 120.0);
    state
        .transaction("Batch", |state| {
            state
                .execute(Command::SetClipProperty {
                    clip: "river-detail".into(),
                    property: "opacity".into(),
                    value: Raw::Number(50.0),
                })
                .map(|_| ())
        })
        .unwrap();
    assert_eq!(
        undo_depth(&state),
        2,
        "the gesture and the batch are separate entries"
    );
    assert_eq!(state.history.undo_label(), Some("Batch"));
}

#[test]
fn slider_gesture_switching_sequence_settles_it() {
    let mut state = EditorState::demo();
    state
        .execute(Command::AddSequence {
            id: Some("alt".into()),
            name: None,
            settings: None,
            activate: false,
        })
        .unwrap();
    let id = begin(&mut state, "zoom");
    update(&mut state, id, 120.0);
    state
        .execute(Command::ActivateSequence {
            sequence: "alt".into(),
        })
        .unwrap();
    assert!(state.open_property_gesture().is_none());
    assert_eq!(undo_depth(&state), 2, "add sequence, then the gesture");
    assert_eq!(number(&state, CLIP, "zoom"), 120.0);
}

#[test]
fn slider_gesture_a_new_begin_settles_the_previous_one() {
    let mut state = EditorState::demo();
    let first = begin(&mut state, "zoom");
    update(&mut state, first, 150.0);
    let second = begin(&mut state, "opacity");
    assert!(second > first, "IDs only grow");
    assert_eq!(undo_depth(&state), 1, "the first gesture was settled");
    update(&mut state, second, 40.0);
    commit(&mut state);
    assert_eq!(undo_depth(&state), 2);
    assert_eq!(number(&state, CLIP, "zoom"), 150.0);
    assert_eq!(number(&state, CLIP, "opacity"), 40.0);
}

#[test]
fn slider_gesture_late_and_foreign_events_are_typed_errors() {
    let mut state = EditorState::demo();
    let update_to = |state: &mut EditorState, gesture_id: Option<u64>| {
        gesture(
            state,
            GestureCommand::Update {
                gesture: gesture_id,
                value: Raw::Number(120.0),
            },
        )
    };
    assert_eq!(update_to(&mut state, None).unwrap_err().code, "no_gesture");
    assert_eq!(
        gesture(&mut state, GestureCommand::Commit { gesture: None })
            .unwrap_err()
            .code,
        "no_gesture"
    );
    assert_eq!(
        gesture(&mut state, GestureCommand::Cancel { gesture: None })
            .unwrap_err()
            .code,
        "no_gesture"
    );

    let id = begin(&mut state, "zoom");
    let error = update_to(&mut state, Some(id + 7)).unwrap_err();
    assert_eq!(error.code, "stale_gesture", "an event of another gesture");
    assert!(
        state.open_property_gesture().is_some(),
        "the open one is untouched"
    );

    // The gesture ends by another edit; a late event of the old drag must not revive it.
    set(&mut state, "opacity", 10.0);
    assert_eq!(
        update_to(&mut state, Some(id)).unwrap_err().code,
        "no_gesture"
    );
    assert_eq!(number(&state, CLIP, "zoom"), 100.0);
}

#[test]
fn slider_gesture_a_document_changed_behind_it_is_dropped_as_stale() {
    let mut state = EditorState::demo();
    let id = begin(&mut state, "zoom");
    update(&mut state, id, 120.0);

    // Something bypassed the settle rule and moved the revision.
    state.project.revision += 1;
    let error = gesture(
        &mut state,
        GestureCommand::Update {
            gesture: Some(id),
            value: Raw::Number(130.0),
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "stale_gesture");
    assert_eq!(error.details.unwrap()["reason"], "document_changed");
    assert!(state.open_property_gesture().is_none());
    assert_eq!(undo_depth(&state), 0, "a stale gesture records nothing");
    assert_eq!(
        number(&state, CLIP, "zoom"),
        120.0,
        "the model is left alone when we cannot tell whose value it is"
    );
}

#[test]
fn slider_gesture_a_vanished_target_rolls_its_own_preview_back() {
    let mut state = EditorState::demo();
    let id = begin(&mut state, "zoom");
    update(&mut state, id, 120.0);

    state.timeline.clips.retain(|clip| clip.id != CLIP);
    let error = gesture(&mut state, GestureCommand::Commit { gesture: Some(id) }).unwrap_err();
    assert_eq!(error.code, "stale_gesture");
    assert_eq!(error.details.unwrap()["reason"], "target_changed");
    assert!(state.open_property_gesture().is_none());
    assert_eq!(undo_depth(&state), 0);
    assert_eq!(
        number(&state, CLIP, "zoom"),
        100.0,
        "only our own preview changed the document, so it is undone"
    );
}

#[test]
fn slider_gesture_clearing_the_history_ends_it_and_ids_keep_growing() {
    let mut state = EditorState::demo();
    let first = begin(&mut state, "zoom");
    update(&mut state, first, 120.0);
    state.clear_history();
    assert!(state.open_property_gesture().is_none());
    assert_eq!(undo_depth(&state), 0, "a loaded project starts clean");
    let second = begin(&mut state, "zoom");
    assert!(second > first);
}

#[test]
fn slider_gesture_every_property_slider_makes_one_entry() {
    for property in PROPERTIES {
        let PropertyKind::Slider { min, max, .. } = property.kind else {
            continue;
        };
        let mut state = EditorState::demo();
        let clip = if property.section == "levels" {
            "score-clip"
        } else {
            CLIP
        };
        state
            .execute(Command::SelectClip { clip: clip.into() })
            .unwrap();
        let default = number(&state, clip, property.id);
        let target = if max != default { max } else { min };

        let id = begin(&mut state, property.id);
        update(&mut state, id, (min + max) / 2.0);
        update(&mut state, id, target);
        commit(&mut state);

        assert_eq!(undo_depth(&state), 1, "{}", property.id);
        assert_eq!(number(&state, clip, property.id), target, "{}", property.id);
        undo(&mut state);
        assert_eq!(
            number(&state, clip, property.id),
            default,
            "{}",
            property.id
        );
    }
}

#[test]
fn slider_gesture_discrete_sets_still_record_one_entry_each() {
    let mut state = EditorState::demo();
    set(&mut state, "zoom", 120.0);
    assert_eq!(undo_depth(&state), 1);
    state
        .execute(Command::SetClipProperty {
            clip: "river-detail".into(),
            property: "zoom".into(),
            value: Raw::Number(130.0),
        })
        .unwrap();
    assert_eq!(undo_depth(&state), 2);
    headless::execute(
        &mut state,
        &json!({"op":"clip.property","clip":"river-detail","property":"zoom","value":140}),
    )
    .unwrap();
    assert_eq!(undo_depth(&state), 3);
}

/// The timeline zoom slider is presentation (not saved in the document): it never records
/// history, with or without a gesture open, and does not end the gesture.
#[test]
fn slider_gesture_timeline_zoom_is_presentation_and_never_enters_the_history() {
    let mut state = EditorState::demo();
    state
        .execute(Command::SetTimelineZoom {
            zoom: Raw::Number(2.0),
        })
        .unwrap();
    assert_eq!(undo_depth(&state), 0);

    let id = begin(&mut state, "zoom");
    update(&mut state, id, 150.0);
    state
        .execute(Command::SetTimelineZoom {
            zoom: Raw::Number(3.0),
        })
        .unwrap();
    assert!(state.open_property_gesture().is_some());
    commit(&mut state);
    assert_eq!(undo_depth(&state), 1);
}

#[test]
fn slider_gesture_json_operations_share_the_library_rules() {
    let mut state = EditorState::demo();
    let mut run = |request: Value| {
        headless::execute(&mut state, &request)
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    };
    let begun = run(json!({"op":"property.gesture.begin","property":"zoom","clip":CLIP}));
    assert_eq!(begun["action"], "gesture.begin");
    let id = begun["gesture"].as_u64().unwrap();
    for value in [110, 130, 150] {
        let updated = run(json!({"op":"property.gesture.update","gesture":id,"value":value}));
        assert_eq!(updated["value"], f64::from(value));
    }
    let status = run(json!({"op":"history.status"}));
    assert_eq!(status["history"]["undo_depth"], 0);
    assert_eq!(status["history"]["gesture"]["id"], id);
    let committed = run(json!({"op":"property.gesture.commit"}));
    assert_eq!(committed["recorded"], true);
    assert_eq!(committed["history"]["undo_depth"], 1);
    let undone = run(json!({"op":"history.undo"}));
    assert_eq!(undone["history"]["undo_depth"], 0);

    let begun = run(json!({"op":"property.gesture.begin","property":"zoom"}));
    assert!(begun["gesture"].as_u64().unwrap() > id);
    run(json!({"op":"property.gesture.update","value":220}));
    let cancelled = run(json!({"op":"property.gesture.cancel"}));
    assert_eq!(cancelled["recorded"], false);
    assert_eq!(cancelled["history"]["undo_depth"], 0);
    assert_eq!(number(&state, CLIP, "zoom"), 100.0);
}

// ---- coexistence with the scene gesture (`scene.gesture.*`, F039) ----
//
// Policy under test: a property gesture's preview IS in the document, a scene gesture's preview
// is not. So (1) any scene edit or scene commit settles the property preview before it runs,
// and (2) a property preview or commit that moves the project revision invalidates an open
// scene gesture (typed `gesture_invalidated`), so no old preview crosses another commit.

const SCENE_FIGURE: &str = "scene/scene-a/node/figure";
const SCENE_FIGURE_POSITION: &str = "scene/scene-a/node/figure/position";

fn with_scene() -> EditorState {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json");
    let bundle: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut state = EditorState::demo();
    headless::execute(
        &mut state,
        &json!({"op":"scene.attach","id":"street","bundle":bundle}),
    )
    .unwrap();
    state
}

fn scene_op(state: &mut EditorState, request: Value) -> Result<Value, CommandError> {
    headless::execute(state, &request)
}

fn undo_label(state: &EditorState) -> Option<String> {
    state.history.undo_label().map(str::to_string)
}

#[test]
fn slider_gesture_a_property_preview_invalidates_an_open_scene_gesture() {
    let mut state = with_scene();
    let attached = undo_depth(&state);
    let scene_gesture = scene_op(
        &mut state,
        json!({"op":"scene.gesture.begin","scene":"street","targets":[SCENE_FIGURE]}),
    )
    .unwrap()["gesture"]
        .as_str()
        .unwrap()
        .to_string();
    scene_op(
        &mut state,
        json!({"op":"scene.gesture.update","gesture":scene_gesture,
            "ops":[{"set":SCENE_FIGURE_POSITION,"value":{"x":3,"y":0,"z":0}}]}),
    )
    .unwrap();

    // The slider preview moves the project revision: the older scene preview must not be
    // committable on top of it.
    let id = begin(&mut state, "zoom");
    update(&mut state, id, 170.0);
    let error = scene_op(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":scene_gesture}),
    )
    .unwrap_err();
    assert_eq!(error.code, "gesture_invalidated");

    // The commit attempt settled the slider preview as ONE entry; nothing of the scene moved.
    assert!(state.open_property_gesture().is_none());
    assert_eq!(undo_depth(&state), attached + 1);
    assert_eq!(undo_label(&state).as_deref(), Some("Set property"));
    assert_eq!(number(&state, CLIP, "zoom"), 170.0);
}

#[test]
fn slider_gesture_a_valid_scene_commit_never_swallows_or_reorders_the_property_entry() {
    let mut state = with_scene();
    let attached = undo_depth(&state);
    // Property gesture first, scene gesture after it: both are live at the same revision.
    let id = begin(&mut state, "zoom");
    update(&mut state, id, 150.0);
    let scene_gesture = scene_op(
        &mut state,
        json!({"op":"scene.gesture.begin","scene":"street","targets":[SCENE_FIGURE]}),
    )
    .unwrap()["gesture"]
        .as_str()
        .unwrap()
        .to_string();
    scene_op(
        &mut state,
        json!({"op":"scene.gesture.update","gesture":scene_gesture,
            "ops":[{"set":SCENE_FIGURE_POSITION,"value":{"x":3,"y":0,"z":0}}]}),
    )
    .unwrap();
    assert!(
        state.open_property_gesture().is_some(),
        "reads and previews keep it open"
    );

    scene_op(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":scene_gesture}),
    )
    .unwrap();
    assert!(state.open_property_gesture().is_none());
    assert_eq!(undo_depth(&state), attached + 2);
    assert_eq!(undo_label(&state).as_deref(), Some("Edit scene"));
    // Undoing the scene edit leaves the property entry, with its committed value.
    undo(&mut state);
    assert_eq!(undo_label(&state).as_deref(), Some("Set property"));
    assert_eq!(number(&state, CLIP, "zoom"), 150.0);
    undo(&mut state);
    assert_eq!(number(&state, CLIP, "zoom"), 100.0);
}
