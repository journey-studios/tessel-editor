//! S3a (BL082.4.5, BL082.4.15): the five transition ops of `scene.fx.edit`, through the library
//! (`apply_fx` over a scene read by the hidden admitting importer). The gate is untouched: these
//! scenes are the tests' own (`support::admit`), and the public doors are proved to stay shut in
//! `public`. Every expected number is written by hand from the contract (F064).

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::{self, Alignment, FxValue, Handles};
use tessel_editor_core::directed_scene::{DirectedScene, Limits};
use tessel_editor_core::scene_edit::{
    CANDIDATE_REJECTED, EditError, FxOp, FxOutcome, INVALID_OP, MAX_FX_BATCH_OPS, TransitionOp,
    apply_fx, apply_fx_measured, dry_run_fx, parse_fx_batch,
};

use crate::support::*;

fn run(scene: &DirectedScene, ops: Value) -> Result<FxOutcome, EditError> {
    apply_fx(scene, &parse_fx_batch(&ops).unwrap())
}

fn run_ok(scene: &DirectedScene, ops: Value) -> FxOutcome {
    run(scene, ops.clone()).unwrap_or_else(|error| panic!("{ops}: refused: {error:?}"))
}

fn transitions(scene: &DirectedScene) -> Vec<Value> {
    scene
        .document()
        .pointer("/fx/transitions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// The importer's codes of a rejected candidate.
fn rejected_codes(error: &EditError) -> Vec<&'static str> {
    assert_eq!(error.code, CANDIDATE_REJECTED, "{error:?}");
    error
        .issues()
        .iter()
        .map(|found| found.issue.code)
        .collect()
}

fn set(extra: Value) -> Value {
    let mut op = json!({
        "op": "set_transition", "fromShotId": "s1", "toShotId": "s2", "effect": "crossDissolve"
    });
    for (key, value) in extra.as_object().unwrap() {
        op[key] = value.clone();
    }
    op
}

fn empty() -> DirectedScene {
    spec_scene(vec![])
}

// ---- E01: a new transition takes the contract's defaults -------------------------------------

#[test]
fn a_new_transition_is_written_whole_with_the_contract_defaults() {
    let outcome = run_ok(&empty(), json!([set(json!({}))]));
    assert!(outcome.summary.modified);
    assert_eq!(
        transitions(&outcome.candidate),
        [json!({
            "id": "tr-1", "effect": "crossDissolve", "fromShotId": "s1", "toShotId": "s2",
            "durationFrames": 24, "alignment": "center", "handles": "hold",
            "easing": "easeInOutCubic",
            "params": {"curve": "smooth", "intensity": 1.0},
        })]
    );
    let change = &outcome.summary.changes[0];
    assert_eq!(change.transition.as_deref(), Some("tr-1"));
    assert_eq!(change.action, "set_transition");
    assert!(!change.noop);
}

#[test]
fn the_default_length_follows_what_the_two_shots_hold_at_three_lengths_and_alignments() {
    // 48-frame shots: the typical 24 fits every alignment.
    // 4-frame shots: a centred window holds at most 8, a start or end one 4.
    let cases = [
        (Edit::spec(), "center", 24),
        (Edit::spec(), "start", 24),
        (Edit::spec(), "end", 24),
        (Edit::many_shots(3), "center", 8),
        (Edit::many_shots(3), "start", 4),
        (Edit::many_shots(3), "end", 4),
    ];
    for (edit, alignment, expected) in cases {
        let scene = scene_of(with_transitions(edit.bundle(), vec![]));
        let from = &scene.bundle().shots[0].id;
        let to = &scene.bundle().shots[1].id;
        let outcome = run_ok(
            &scene,
            json!([{
                "op": "set_transition", "fromShotId": from, "toShotId": to,
                "effect": "crossDissolve", "alignment": alignment
            }]),
        );
        let written = &transitions(&outcome.candidate)[0];
        assert_eq!(written["durationFrames"], expected, "{alignment}");
        assert_eq!(written["alignment"], alignment);
    }
}

#[test]
fn an_explicit_value_is_never_cut_and_is_judged_as_it_is() {
    // 60 frames at a 48-frame cut: refused by the importer, not shortened to fit.
    let error = run(&empty(), json!([set(json!({"durationFrames": 200}))])).unwrap_err();
    assert!(
        rejected_codes(&error).contains(&"TRANSITION_OUT_OF_RANGE")
            || rejected_codes(&error).contains(&"TRANSITION_TOO_LONG"),
        "{:?}",
        rejected_codes(&error)
    );
    let outcome = run_ok(&empty(), json!([set(json!({"durationFrames": 40}))]));
    assert_eq!(transitions(&outcome.candidate)[0]["durationFrames"], 40);
}

#[test]
fn a_length_the_shots_cannot_hold_is_refused_not_trimmed() {
    // Shots of 4 frames hold at most 4 frames of a window that starts at the cut.
    let ops = parse_fx_batch(&json!([{
        "op": "set_transition", "fromShotId": "s0", "toShotId": "s1",
        "effect": "crossDissolve", "alignment": "start", "durationFrames": 5
    }]))
    .unwrap();
    let scene = scene_of(with_transitions(Edit::many_shots(3).bundle(), vec![]));
    let error = apply_fx(&scene, &ops).unwrap_err();
    assert_eq!(rejected_codes(&error), ["TRANSITION_TOO_LONG"]);
}

// ---- E02: the upsert is by cut and keeps what it does not name --------------------------------

fn authored() -> DirectedScene {
    spec_scene(vec![tr(json!({
        "id": "keep", "durationFrames": 12,
        "params": {"curve": "smooth", "intensity": 1},
        "note": "mine", "seed": 7,
    }))])
}

#[test]
fn the_upsert_keeps_the_id_and_every_member_the_op_does_not_name() {
    let scene = authored();
    let outcome = run_ok(&scene, json!([set(json!({"params": {"curve": "linear"}}))]));
    let written = &transitions(&outcome.candidate)[0];
    assert_eq!(written["id"], "keep");
    assert_eq!(written["durationFrames"], 12);
    assert_eq!(written["note"], "mine");
    assert_eq!(written["seed"], 7);
    assert_eq!(written["params"]["curve"], "linear");
    // The authored spelling of the untouched parameter stays: an integer 1, not 1.0.
    assert_eq!(written["params"]["intensity"], json!(1));
    assert_eq!(transitions(&outcome.candidate).len(), 1);
}

#[test]
fn the_upsert_changes_only_the_fields_it_names() {
    let outcome = run_ok(&authored(), json!([set(json!({"durationFrames": 16}))]));
    let mut expected = transitions(&authored())[0].clone();
    expected["durationFrames"] = json!(16);
    assert_eq!(transitions(&outcome.candidate), [expected]);
}

#[test]
fn an_explicit_id_that_is_not_the_one_on_the_cut_is_refused_not_renamed() {
    let scene = authored();
    let error = run(&scene, json!([set(json!({"id": "other"}))])).unwrap_err();
    assert_eq!(error.code, fx::TRANSITION_ID_MISMATCH);
    assert_eq!(error.index, Some(0));
    assert_eq!(error.path.as_deref(), Some("id"));
    let details = error.details().unwrap();
    assert_eq!(details["args"]["existing"], "keep");
    assert!(details["hint"].as_str().unwrap().contains("keep"));
    // The right id is accepted and changes nothing: the same transition.
    let outcome = run_ok(&scene, json!([set(json!({"id": "keep"}))]));
    assert!(!outcome.summary.modified);
    assert!(outcome.summary.changes[0].noop);
}

#[test]
fn a_look_where_a_transition_is_expected_is_refused_with_the_family() {
    let error = run(&empty(), json!([set(json!({"effect": "vignette"}))])).unwrap_err();
    assert_eq!(error.code, fx::WRONG_FAMILY);
    assert_eq!(error.index, Some(0));
    let error = run(&empty(), json!([set(json!({"effect": "wipe"}))])).unwrap_err();
    assert_eq!(error.code, fx::UNKNOWN_EFFECT);
}

// ---- E03: resize, parameters, easing -----------------------------------------------------------

#[test]
fn resize_changes_the_window_and_only_the_window() {
    let scene = authored();
    let outcome = run_ok(
        &scene,
        json!([{"op": "resize_transition", "id": "keep", "durationFrames": 20,
                "alignment": "start"}]),
    );
    let written = &transitions(&outcome.candidate)[0];
    assert_eq!(written["durationFrames"], 20);
    assert_eq!(written["alignment"], "start");
    assert_eq!(written["params"], transitions(&scene)[0]["params"]);
    assert_eq!(written["note"], "mine");
    // Without an alignment, the one on the transition stays.
    let outcome = run_ok(
        &scene,
        json!([{"op": "resize_transition", "id": "keep", "durationFrames": 30}]),
    );
    assert_eq!(transitions(&outcome.candidate)[0]["alignment"], "center");
}

#[test]
fn a_resize_that_does_not_fit_is_refused_by_the_importer_and_nothing_is_cut() {
    let error = run(
        &authored(),
        json!([{"op": "resize_transition", "id": "keep", "durationFrames": 500}]),
    )
    .unwrap_err();
    assert!(rejected_codes(&error).contains(&"TRANSITION_OUT_OF_RANGE"));
}

#[test]
fn the_parameters_are_typed_and_judged_against_the_catalog() {
    let scene = authored();
    for (param, value) in [
        ("intensity", json!(0)),
        ("intensity", json!(0.5)),
        ("curve", json!("linear")),
    ] {
        let outcome = run_ok(
            &scene,
            json!([{"op": "set_transition_param", "id": "keep", "param": param, "value": value}]),
        );
        let written = &transitions(&outcome.candidate)[0];
        // A number is written as the catalog types it (0 becomes 0.0), a choice as its name.
        match value.as_f64() {
            Some(number) => assert_eq!(written["params"][param].as_f64(), Some(number), "{param}"),
            None => assert_eq!(written["params"][param], value, "{param}"),
        }
    }
    for (param, value, code) in [
        ("intensity", json!(1.5), fx::PARAM_OUT_OF_RANGE),
        ("intensity", json!(-0.1), fx::PARAM_OUT_OF_RANGE),
        ("intensity", json!("x"), fx::PARAM_TYPE),
        ("curve", json!("cubic"), fx::PARAM_OUT_OF_RANGE),
        ("curve", json!(1), fx::PARAM_TYPE),
        ("grain", json!(1), fx::UNKNOWN_PARAM),
    ] {
        let error = run(
            &scene,
            json!([{"op": "set_transition_param", "id": "keep", "param": param, "value": value}]),
        )
        .unwrap_err();
        assert_eq!(error.code, code, "{param} {value}");
        assert_eq!(error.index, Some(0));
        assert_eq!(error.id.as_deref(), Some("keep"));
    }
}

#[test]
fn a_linear_curve_keeps_the_intensity_it_had() {
    let scene = spec_scene(vec![tr(
        json!({"params": {"curve": "smooth", "intensity": 0.4}}),
    )]);
    let outcome = run_ok(
        &scene,
        json!([{"op": "set_transition_param", "id": "tr", "param": "curve", "value": "linear"}]),
    );
    let written = &outcome.candidate.bundle().fx.transitions[0];
    let spec = fx::transition_effect("crossDissolve").unwrap();
    let typed = |name: &str| written.params[spec.param(name).unwrap().0];
    assert_eq!(typed("curve"), FxValue::Choice("linear"));
    assert_eq!(typed("intensity"), FxValue::Number(0.4));
    assert_eq!(
        transitions(&outcome.candidate)[0]["params"]["intensity"],
        0.4
    );
}

#[test]
fn an_easing_that_is_not_known_is_refused_with_the_list() {
    let error = run(&empty(), json!([set(json!({"easing": "bounce"}))])).unwrap_err();
    assert_eq!(error.code, fx::UNKNOWN_KEYFRAME_EASING);
    assert!(
        error.details().unwrap()["hint"]
            .as_str()
            .unwrap()
            .contains("linear")
    );
    let outcome = run_ok(&empty(), json!([set(json!({"easing": "linear"}))]));
    assert_eq!(transitions(&outcome.candidate)[0]["easing"], "linear");
}

#[test]
fn a_parameter_the_catalog_does_not_have_is_never_written() {
    // Unknown members of `params` are refused by the importer, so they cannot be preserved: the
    // op refuses first and the document keeps what it had.
    let scene = authored();
    let error = run(&scene, json!([set(json!({"params": {"grain": 1}}))])).unwrap_err();
    assert_eq!(error.code, fx::UNKNOWN_PARAM);
    assert_eq!(
        transitions(&scene)[0]["params"],
        json!({"curve": "smooth", "intensity": 1})
    );
}

// ---- E04: hidden and remove --------------------------------------------------------------------

#[test]
fn hidden_true_is_written_and_hidden_false_leaves_the_member_out() {
    let scene = authored();
    let hidden = run_ok(
        &scene,
        json!([{"op": "set_transition_hidden", "id": "keep", "hidden": true}]),
    );
    assert_eq!(transitions(&hidden.candidate)[0]["hidden"], true);
    assert!(hidden.candidate.bundle().fx.transitions[0].hidden);
    let shown = run_ok(
        &hidden.candidate,
        json!([{"op": "set_transition_hidden", "id": "keep", "hidden": false}]),
    );
    assert!(transitions(&shown.candidate)[0].get("hidden").is_none());
    assert_eq!(
        shown.candidate.document(),
        scene.document(),
        "back to the original, byte for byte"
    );
    // `hidden: false` on a new transition is the same: no member.
    let outcome = run_ok(&empty(), json!([set(json!({"hidden": false}))]));
    assert!(transitions(&outcome.candidate)[0].get("hidden").is_none());
    let outcome = run_ok(&empty(), json!([set(json!({"hidden": true}))]));
    assert_eq!(transitions(&outcome.candidate)[0]["hidden"], true);
}

#[test]
fn removing_the_last_transition_keeps_the_looks_and_the_unknown_fields_of_the_layer() {
    let mut bundle = with_transitions(Edit::spec().bundle(), vec![tr(json!({}))]);
    bundle["fx"]["tracks"] = json!([output_clip("t1", "c1", "vignette", 0, 20)]);
    bundle["fx"]["x-note"] = json!("kept");
    let scene = scene_of(bundle);
    let outcome = run_ok(&scene, json!([{"op": "remove_transition", "id": "tr"}]));
    let layer = outcome.candidate.document().pointer("/fx").unwrap();
    assert_eq!(layer["transitions"], json!([]));
    assert_eq!(layer["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(layer["x-note"], "kept");
    assert!(outcome.candidate.bundle().fx.transitions.is_empty());
}

#[test]
fn removing_the_only_transition_of_a_bare_layer_gives_back_a_document_without_fx() {
    let base = scene_of(Edit::spec().bundle());
    assert!(base.document().get("fx").is_none());
    let added = run_ok(&base, json!([set(json!({}))]));
    assert!(added.candidate.document().get("fx").is_some());
    let removed = run_ok(
        &added.candidate,
        json!([{"op": "remove_transition", "id": "tr-1"}]),
    );
    assert_eq!(removed.candidate.document(), base.document());
}

#[test]
fn removing_one_transition_keeps_the_order_of_the_others() {
    let scene = spec_scene(vec![
        tr(json!({"id": "a", "durationFrames": 8})),
        tr(json!({"id": "b", "fromShotId": "s2", "toShotId": "s3", "durationFrames": 8})),
    ]);
    let outcome = run_ok(&scene, json!([{"op": "remove_transition", "id": "a"}]));
    let ids: Vec<Value> = transitions(&outcome.candidate)
        .iter()
        .map(|t| t["id"].clone())
        .collect();
    assert_eq!(ids, [json!("b")]);
}

#[test]
fn a_transition_that_is_not_there_is_named_with_the_ones_that_are() {
    let scene = authored();
    for op in [
        json!({"op": "remove_transition", "id": "ghost"}),
        json!({"op": "set_transition_hidden", "id": "ghost", "hidden": true}),
        json!({"op": "resize_transition", "id": "ghost", "durationFrames": 4}),
        json!({"op": "set_transition_param", "id": "ghost", "param": "curve", "value": "linear"}),
    ] {
        let error = run(&scene, json!([op])).unwrap_err();
        assert_eq!(error.code, fx::UNKNOWN_TRANSITION, "{op}");
        assert_eq!(error.id.as_deref(), Some("ghost"));
        let details = error.details().unwrap();
        assert_eq!(details["args"]["kind"], "some");
        assert_eq!(details["args"]["known"], json!(["keep"]));
        assert!(details["hint"].as_str().unwrap().contains("keep"));
    }
    let error = run(
        &empty(),
        json!([{"op": "remove_transition", "id": "ghost"}]),
    )
    .unwrap_err();
    assert_eq!(error.details().unwrap()["args"]["kind"], "none");
}

// ---- E05: no-ops, empty batches and a net round trip -------------------------------------------

#[test]
fn a_no_op_is_judged_on_typed_values_and_changes_nothing() {
    let scene = authored();
    for op in [
        set(
            json!({"durationFrames": 12, "alignment": "center", "handles": "hold",
                   "easing": "easeInOutCubic", "params": {"intensity": 1.0, "curve": "smooth"}}),
        ),
        json!({"op": "resize_transition", "id": "keep", "durationFrames": 12}),
        json!({"op": "set_transition_param", "id": "keep", "param": "intensity", "value": 1.0}),
        json!({"op": "set_transition_hidden", "id": "keep", "hidden": false}),
    ] {
        let outcome = run_ok(&scene, json!([op]));
        assert!(!outcome.summary.modified, "{op}");
        assert!(outcome.summary.changes[0].noop, "{op}");
        assert_eq!(outcome.summary.changed_count(), 0);
        assert_eq!(outcome.candidate.document(), scene.document(), "{op}");
    }
}

#[test]
fn an_empty_batch_and_a_batch_that_undoes_itself_are_not_modifications() {
    let scene = authored();
    let outcome = run_ok(&scene, json!([]));
    assert!(!outcome.summary.modified);
    assert_eq!(outcome.candidate.document(), scene.document());
    let outcome = run_ok(
        &scene,
        json!([
            {"op": "set_transition_hidden", "id": "keep", "hidden": true},
            {"op": "set_transition_hidden", "id": "keep", "hidden": false},
            set(json!({"durationFrames": 20})),
            set(json!({"durationFrames": 12})),
        ]),
    );
    assert!(!outcome.summary.modified, "net no change");
    assert_eq!(outcome.candidate.document(), scene.document());
    assert_eq!(
        outcome.summary.changed_count(),
        4,
        "each op did change something at its turn"
    );
}

#[test]
fn adding_a_transition_and_removing_it_again_is_a_net_no_op() {
    let base = scene_of(Edit::spec().bundle());
    let outcome = run_ok(
        &base,
        json!([set(json!({})), {"op": "remove_transition", "id": "tr-1"}]),
    );
    assert!(!outcome.summary.modified);
    assert_eq!(outcome.candidate.document(), base.document());
}

#[test]
fn numeric_round_trips_preserve_authorship_even_with_another_change() {
    for initial in [json!(1), json!(1.0)] {
        let mut raw = authored().document().clone();
        raw["fx"]["transitions"][0]["params"]["intensity"] = initial.clone();
        let scene = admit(raw).unwrap();
        let ops = json!([
            {"op":"set_transition_param","id":"keep","param":"intensity","value":0.5},
            {"op":"set_transition_param","id":"keep","param":"intensity","value":1}
        ]);
        let result = run_ok(&scene, ops.clone());
        assert!(!result.summary.modified);
        assert_eq!(result.candidate.document(), scene.document());
        let mut mixed = ops.as_array().unwrap().clone();
        mixed.push(json!({"op":"set_transition_hidden","id":"keep","hidden":true}));
        let result = run_ok(&scene, Value::Array(mixed));
        assert!(result.summary.modified);
        let mut expected = scene.document().clone();
        expected["fx"]["transitions"][0]["hidden"] = json!(true);
        assert_eq!(result.candidate.document(), &expected);
    }
}

#[test]
fn a_round_trip_that_writes_an_omitted_default_remains_a_real_edit() {
    let mut raw = authored().document().clone();
    raw["fx"]["transitions"][0]["params"]
        .as_object_mut()
        .unwrap()
        .remove("intensity");
    let scene = admit(raw).unwrap();
    let result = run_ok(
        &scene,
        json!([
            {"op":"set_transition_param","id":"keep","param":"intensity","value":0.5},
            {"op":"set_transition_param","id":"keep","param":"intensity","value":1}
        ]),
    );
    assert!(result.summary.modified, "an authored field was added");
    let mut expected = scene.document().clone();
    expected["fx"]["transitions"][0]["params"]["intensity"] = json!(1.0);
    assert_eq!(result.candidate.document(), &expected);
}

#[test]
fn a_reused_id_does_not_attribute_transition_faults_to_a_removed_look() {
    let scene = run_ok(
        &empty(),
        json!([{
            "op":"add_clip", "id":"shared", "effect":"vignette",
            "start_frame":0, "duration_frames":10
        }]),
    )
    .candidate;
    let before = scene.document().clone();
    let error = run(
        &scene,
        json!([
            {"op":"remove_clip","clip":"shared"},
            set(json!({"id":"shared","durationFrames":500}))
        ]),
    )
    .unwrap_err();
    assert_eq!(error.code, CANDIDATE_REJECTED);
    assert_eq!(error.index, Some(1));
    assert!(!error.issues().is_empty());
    for issue in error.issues() {
        assert_eq!(issue.ops, vec![1]);
    }
    assert_eq!(scene.document(), &before);
}

#[test]
fn a_dry_run_gives_the_summary_and_the_refusals_of_the_real_run() {
    let scene = empty();
    let good = parse_fx_batch(&json!([set(json!({}))])).unwrap();
    let summary = dry_run_fx(&scene, &good).unwrap();
    assert_eq!(summary, apply_fx(&scene, &good).unwrap().summary);
    assert!(transitions(&scene).is_empty(), "the scene was not touched");
    let bad = parse_fx_batch(&json!([set(json!({"durationFrames": 500}))])).unwrap();
    let dry = dry_run_fx(&scene, &bad).unwrap_err();
    let real = apply_fx(&scene, &bad).unwrap_err();
    assert_eq!(dry, real);
}

// ---- E06: the batch is atomic ------------------------------------------------------------------

#[test]
fn a_batch_of_looks_and_transitions_lands_whole_or_not_at_all() {
    let scene = empty();
    let mixed = json!([
        {"op": "add_clip", "effect": "vignette", "start_frame": 0, "duration_frames": 20, "id": "v1"},
        set(json!({"durationFrames": 10})),
        {"op": "set_transition_hidden", "id": "tr-1", "hidden": true},
    ]);
    let outcome = run_ok(&scene, mixed.clone());
    assert_eq!(transitions(&outcome.candidate).len(), 1);
    assert_eq!(outcome.candidate.bundle().fx.tracks.len(), 1);
    assert_eq!(outcome.summary.changes.len(), 3);

    // The last op fails: nothing of the first two exists.
    let broken = json!([
        {"op": "add_clip", "effect": "vignette", "start_frame": 0, "duration_frames": 20, "id": "v1"},
        set(json!({})),
        {"op": "remove_transition", "id": "ghost"},
    ]);
    let before = scene.document().clone();
    let error = run(&scene, broken).unwrap_err();
    assert_eq!(error.index, Some(2));
    assert_eq!(error.code, fx::UNKNOWN_TRANSITION);
    assert_eq!(scene.document(), &before);
    assert!(transitions(&scene).is_empty());
}

#[test]
fn a_candidate_the_importer_refuses_leaves_the_scene_untouched() {
    let scene = authored();
    let before = scene.document().clone();
    let error = run(
        &scene,
        json!([
            set(json!({"durationFrames": 14})),
            {"op": "resize_transition", "id": "keep", "durationFrames": 900},
        ]),
    )
    .unwrap_err();
    assert!(rejected_codes(&error).contains(&"TRANSITION_OUT_OF_RANGE"));
    assert_eq!(scene.document(), &before);
}

// ---- E09 (BL082.4.15): one namespace of ids -----------------------------------------------------

fn crowded() -> DirectedScene {
    let mut bundle = with_transitions(
        Edit::spec().bundle(),
        vec![
            tr(json!({"id": "fx-1", "durationFrames": 8})),
            tr(
                json!({"id": "fx-track-1", "fromShotId": "s2", "toShotId": "s3",
                      "durationFrames": 8}),
            ),
        ],
    );
    bundle["fx"]["tracks"] = json!([output_clip("t1", "tr-1", "vignette", 0, 10)]);
    scene_of(bundle)
}

#[test]
fn a_generated_id_never_collides_with_a_track_a_clip_or_a_transition() {
    let scene = crowded();
    // A clip is called `tr-1`, so the new transition is `tr-2`.
    let hidden_on_s1 = run_ok(
        &spec_scene_with_tracks(
            vec![],
            json!([output_clip("t1", "tr-1", "vignette", 0, 10)]),
        ),
        json!([set(json!({}))]),
    );
    assert_eq!(transitions(&hidden_on_s1.candidate)[0]["id"], "tr-2");

    // A transition is called `fx-1` and another `fx-track-1`: a new clip and its track skip both.
    let outcome = run_ok(
        &scene,
        // Overlapping the clip of `t1`, so a second track is needed.
        json!([{"op": "add_clip", "effect": "vignette", "start_frame": 5, "duration_frames": 10}]),
    );
    let layer = outcome
        .candidate
        .document()
        .pointer("/fx/tracks")
        .unwrap()
        .clone();
    let ids: Vec<&str> = layer
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| {
            std::iter::once(track["id"].as_str().unwrap()).chain(
                track["clips"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|clip| clip["id"].as_str().unwrap()),
            )
        })
        .collect();
    assert!(ids.contains(&"fx-track-2"), "{ids:?}");
    assert!(ids.contains(&"fx-2"), "{ids:?}");
}

#[test]
fn an_explicit_id_cannot_reuse_one_that_exists_or_a_reserved_word() {
    let scene = crowded();
    for (id, code) in [
        ("fx-1", "DUPLICATE_ID"),
        ("fx-track-1", "DUPLICATE_ID"),
        ("tr-1", "DUPLICATE_ID"),
        ("t1", "DUPLICATE_ID"),
        ("fx", "RESERVED_ID"),
        ("TRACK", "RESERVED_ID"),
        ("Transition", "RESERVED_ID"),
    ] {
        let spare = json!({"op": "set_transition", "fromShotId": "s1", "toShotId": "s3",
                           "effect": "crossDissolve", "id": id, "durationFrames": 4});
        let error = run(&scene, json!([spare])).unwrap_err();
        let expected = if code == "DUPLICATE_ID" {
            fx::DUPLICATE_ID
        } else {
            fx::RESERVED_ID
        };
        assert_eq!(error.code, expected, "{id}");
        assert_eq!(error.index, Some(0));
    }
}

#[test]
fn an_id_differing_only_by_case_is_another_id() {
    // The comparison is exact (the reserved words are the only case-insensitive names).
    let scene = authored();
    let outcome = run_ok(
        &scene,
        json!([{"op": "set_transition", "fromShotId": "s2", "toShotId": "s3",
                "effect": "crossDissolve", "id": "KEEP", "durationFrames": 8}]),
    );
    assert_eq!(transitions(&outcome.candidate).len(), 2);
}

#[test]
fn two_new_transitions_in_one_batch_get_two_ids() {
    let outcome = run_ok(
        &empty(),
        json!([
            set(json!({"durationFrames": 8})),
            {"op": "set_transition", "fromShotId": "s2", "toShotId": "s3",
             "effect": "crossDissolve", "durationFrames": 8},
        ]),
    );
    let ids: Vec<Value> = transitions(&outcome.candidate)
        .iter()
        .map(|t| t["id"].clone())
        .collect();
    assert_eq!(ids, [json!("tr-1"), json!("tr-2")]);
}

// ---- E10: quotas -------------------------------------------------------------------------------

#[test]
fn a_batch_past_the_ceiling_is_refused_before_anything_is_parsed_or_run() {
    assert_eq!(MAX_FX_BATCH_OPS, 1024);
    let long = Value::Array(vec![json!({"op": "remove_transition", "id": "x"}); 1025]);
    let error = parse_fx_batch(&long).unwrap_err();
    assert_eq!(error.code, fx::FX_LIMIT);
    assert_eq!(error.path.as_deref(), Some("ops"));
    let typed: Vec<FxOp> = (0..1025)
        .map(|_| FxOp::Transition(TransitionOp::Remove { id: "x".into() }))
        .collect();
    let (outcome, work) = apply_fx_measured(&authored(), &typed, &Limits::default());
    assert_eq!(outcome.unwrap_err().code, fx::FX_LIMIT);
    assert_eq!(work.ops_run, 0, "stopped before the first op");
    // Exactly the ceiling is a batch.
    let at_ceiling = Value::Array(vec![
        json!({"op": "set_transition_hidden", "id": "keep", "hidden": false});
        1024
    ]);
    let outcome = run_ok(&authored(), at_ceiling);
    assert_eq!(outcome.summary.work.ops_run, 1024);
    assert!(!outcome.summary.modified);
}

#[test]
fn the_count_limit_is_the_importers_and_a_removal_frees_the_room() {
    let scene = scene_of(with_transitions(Edit::many_shots(5).bundle(), vec![]));
    let all_cuts: Vec<Value> = (0..4)
        .map(|i| {
            json!({"op": "set_transition", "fromShotId": format!("s{i}"),
                   "toShotId": format!("s{}", i + 1), "effect": "crossDissolve",
                   "durationFrames": 2})
        })
        .collect();
    let full = run_ok(&scene, Value::Array(all_cuts));
    assert_eq!(transitions(&full.candidate).len(), 4);
    // A transition on a pair that is not a cut: not at cut, and one too many.
    let far = json!([{"op": "set_transition", "fromShotId": "s0", "toShotId": "s2",
                      "effect": "crossDissolve", "durationFrames": 2}]);
    let error = run(&full.candidate, far.clone()).unwrap_err();
    let codes = rejected_codes(&error);
    assert_eq!(codes, ["FX_LIMIT"], "the count is checked before the pair");
    // Replacing the one on a cut does not use a new slot.
    let replaced = run_ok(
        &full.candidate,
        json!([{"op": "set_transition", "fromShotId": "s0", "toShotId": "s1",
                "effect": "crossDissolve", "durationFrames": 1}]),
    );
    assert_eq!(transitions(&replaced.candidate).len(), 4);
    // Removing one frees the room: the same op now only says it is not a cut.
    let freed = run(
        &full.candidate,
        json!([{"op": "remove_transition", "id": "tr-1"}, far[0].clone()]),
    )
    .unwrap_err();
    let codes = rejected_codes(&freed);
    assert!(!codes.contains(&"FX_LIMIT"), "{codes:?}");
    assert!(codes.contains(&"TRANSITION_NOT_AT_CUT"), "{codes:?}");
}

// ---- E11: the rules of the importer, in front of an edit ------------------------------------------

#[test]
fn every_rule_of_the_window_is_the_importers_and_answers_a_batch() {
    let scene = authored();
    // Overlap: s1->s2 starting at the cut ([48, 96)) and s2->s3 centred at 96 ([90, 102)).
    let overlapping = json!([
        set(json!({"alignment": "start", "durationFrames": 48})),
        {"op": "set_transition", "fromShotId": "s2", "toShotId": "s3",
         "effect": "crossDissolve", "durationFrames": 12},
    ]);
    assert_eq!(
        rejected_codes(&run(&scene, overlapping).unwrap_err()),
        ["TRANSITION_OVERLAP"]
    );

    // A window that leaves the edit.
    let outside = json!([{"op": "resize_transition", "id": "keep", "durationFrames": 200}]);
    assert!(
        rejected_codes(&run(&scene, outside).unwrap_err()).contains(&"TRANSITION_OUT_OF_RANGE")
    );

    // A window longer than a shot can hold (handles hold, so it is the length that fails).
    let too_long = json!([{"op": "resize_transition", "id": "keep", "durationFrames": 100,
                           "alignment": "start"}]);
    assert!(rejected_codes(&run(&scene, too_long).unwrap_err()).contains(&"TRANSITION_TOO_LONG"));

    // A shot that does not exist, and two shots that are not neighbours.
    let ghost = run(&empty(), json!([set(json!({"toShotId": "zz"}))])).unwrap_err();
    assert_eq!(ghost.code, fx::UNKNOWN_SHOT);
    assert_eq!(
        ghost.details().unwrap()["args"]["shots"],
        json!(["s1", "s2", "s3"])
    );
    let apart = json!([{"op": "set_transition", "fromShotId": "s1", "toShotId": "s3",
                        "effect": "crossDissolve", "durationFrames": 8}]);
    assert_eq!(
        rejected_codes(&run(&empty(), apart).unwrap_err()),
        ["TRANSITION_NOT_AT_CUT"]
    );
    // The reverse order of neighbours is not a cut either.
    let reversed = json!([{"op": "set_transition", "fromShotId": "s2", "toShotId": "s1",
                           "effect": "crossDissolve", "durationFrames": 8}]);
    assert_eq!(
        rejected_codes(&run(&empty(), reversed).unwrap_err()),
        ["TRANSITION_NOT_AT_CUT"]
    );
}

#[test]
fn the_ops_are_strict_about_what_they_read() {
    for bad in [
        set(json!({"durationFrames": 12.5})),
        set(json!({"durationFrames": 12.0})),
        set(json!({"durationFrames": "12"})),
        set(json!({"durationFrames": true})),
        set(json!({"alignment": "middle"})),
        set(json!({"handles": "keep"})),
        set(json!({"hidden": "yes"})),
        set(json!({"params": 3})),
        set(json!({"from_shot_id": "s1"})),
        set(json!({"id": 4})),
        json!({"op": "set_transition", "toShotId": "s2", "effect": "crossDissolve"}),
        json!({"op": "resize_transition", "id": "tr"}),
        json!({"op": "resize_transition", "id": "tr", "durationFrames": 4.0}),
        json!({"op": "set_transition_param", "id": "tr", "param": "curve"}),
        json!({"op": "set_transition_hidden", "id": "tr", "hidden": 1}),
        json!({"op": "remove_transition", "id": "tr", "extra": 1}),
        json!({"op": "remove_transition"}),
    ] {
        let error = parse_fx_batch(&json!([bad])).unwrap_err();
        assert_eq!(error.code, INVALID_OP, "{bad}");
        assert_eq!(error.index, Some(0), "{bad}");
    }
    // The error of a bad op names where it failed in a longer batch.
    let error = parse_fx_batch(&json!([set(json!({})), {"op": "teleport"}])).unwrap_err();
    assert_eq!(error.index, Some(1));
}

#[test]
fn the_typed_ops_round_trip_through_their_json() {
    let op = FxOp::from_json(&set(json!({
        "id": "x", "durationFrames": 10, "alignment": "end", "handles": "use",
        "easing": "linear", "params": {"curve": "linear"}, "hidden": true
    })))
    .unwrap();
    let FxOp::Transition(TransitionOp::Set(set)) = &op else {
        panic!("{op:?}");
    };
    assert_eq!(set.id.as_deref(), Some("x"));
    assert_eq!(set.alignment, Some(Alignment::End));
    assert_eq!(set.handles, Some(Handles::Use));
    assert_eq!(set.hidden, Some(true));
    assert_eq!(set.duration_frames, Some(10));
    assert_eq!(op.clip_id(), None);
}

// ---- E19: the shot a transition names is not repaired -------------------------------------------

#[test]
fn a_shot_that_goes_away_or_shrinks_is_refused_by_the_importer_and_nothing_is_fixed() {
    let with = |edit: Edit| with_transitions(edit.bundle(), vec![tr(json!({}))]);
    // `s2` removed from the edit: the transition names a shot that is not there.
    let mut edit = Edit::spec();
    edit.shots.remove(1);
    edit.shots[1].start = 48;
    edit.shots[1].end = 96;
    edit.duration = 96;
    assert!(admit_codes(with(edit)).contains(&"UNKNOWN_SHOT"));
    // `s2` shortened to 4 frames: a centred 12-frame window needs 6 inside it.
    let edit = Edit::spec()
        .shot("s2", |shot| shot.end = 52)
        .shot("s3", |shot| shot.start = 52);
    assert!(admit_codes(with(edit)).contains(&"TRANSITION_TOO_LONG"));
    // `s2` and `s3` swapped: the pair is no longer a cut.
    let mut edit = Edit::spec();
    edit.shots.swap(1, 2);
    edit.shots[1].start = 48;
    edit.shots[1].end = 96;
    edit.shots[2].start = 96;
    edit.shots[2].end = 144;
    assert!(admit_codes(with(edit)).contains(&"TRANSITION_NOT_AT_CUT"));
}
