//! F016 (BL028/BL036/BL082): the vignette effect, output scope, through the library, the plan and
//! the JSON executor `tessel-headless` and `ui op` share (no window, no GPU).
//!
//! What it proves: the document model and its refusals, the deterministic per-frame evaluation,
//! the atomic edit (one undo entry, a refusal or a no-op leaves nothing), persistence and reopen,
//! that the plan carries the resolved passes (and a runtime without `outputFx` refuses it), and,
//! against the reference of the pixel math, that no effect and intensity 0 keep the baseline
//! while intensity above 0 changes the expected region. The pixels of the real monitor and
//! export are `crates/render/tests/scene_fx.rs`, `scene_fx_export.rs` and
//! `scripts/scene-fx-ui-proof.py` (the real window).

use std::fs;
use std::io::Write;
use std::process::{Command as Process, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::directed_scene::fx::{self, FxValue};
use tessel_editor_core::scene_plan::{Capability, PlanTimeline, ScenePlan};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless, persist};

/// Two shots over 96 frames, a complete plan at every frame (no deferred domain in use).
const MINIMAL: &str =
    include_str!("../../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json");

fn bundle() -> Value {
    serde_json::from_str(MINIMAL).unwrap()
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn err(state: &mut EditorState, request: Value) -> CommandError {
    match headless::execute(state, &request) {
        Ok(answer) => panic!("{request} should have failed, got {answer}"),
        Err(error) => error,
    }
}

fn session() -> EditorState {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle()}),
    );
    state
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

fn document(state: &EditorState) -> Value {
    state.scenes.get("street").unwrap().scene.document().clone()
}

fn edit(state: &mut EditorState, ops: Value) -> Value {
    ok(
        state,
        json!({"op":"scene.fx.edit","scene":"street","ops":ops}),
    )
}

fn edit_err(state: &mut EditorState, ops: Value) -> CommandError {
    err(
        state,
        json!({"op":"scene.fx.edit","scene":"street","ops":ops}),
    )
}

fn add(start: i64, duration: i64, params: Value) -> Value {
    json!({"op":"add_clip","effect":"vignette","start_frame":start,"duration_frames":duration,
        "id":"v1","params":params})
}

fn passes(state: &mut EditorState, frame: i64) -> Vec<Value> {
    ok(
        state,
        json!({"op":"scene.fx.frame","scene":"street","frame":frame}),
    )["passes"]
        .as_array()
        .unwrap()
        .clone()
}

fn plan_json(state: &mut EditorState, frame: i64) -> Value {
    ok(
        state,
        json!({"op":"scene.plan","scene":"street","frame":frame}),
    )["plan"]
        .clone()
}

fn scene_of(state: &EditorState) -> DirectedScene {
    DirectedScene::import_value(document(state)).unwrap()
}

// ---- the catalog and the baseline ---------------------------------------------------------

#[test]
fn the_catalog_lists_the_one_effect_this_slice_draws_and_what_it_refuses() {
    let mut state = session();
    let catalog = ok(&mut state, json!({"op":"scene.fx.catalog"}));
    assert_eq!(catalog["catalog_version"], 1);
    let effects = catalog["effects"].as_array().unwrap();
    assert_eq!(effects.len(), 1, "only the vignette is entered");
    assert_eq!(effects[0]["id"], "vignette");
    let names: Vec<_> = effects[0]["params"]
        .as_array()
        .unwrap()
        .iter()
        .map(|param| param["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "radius",
            "softness",
            "intensity",
            "color",
            "roundness",
            "center"
        ]
    );
    assert_eq!(catalog["scope"]["scope"], json!(["output"]));
    assert!(
        catalog["scope"]["refused"]
            .as_object()
            .unwrap()
            .contains_key("transitions")
    );
    let capabilities = ok(&mut state, json!({"op":"scene.capabilities"}));
    let commands = capabilities["commands"].as_array().unwrap();
    for command in [
        "scene.fx.edit",
        "scene.fx.list",
        "scene.fx.catalog",
        "scene.fx.frame",
    ] {
        assert!(commands.contains(&json!(command)), "{command}");
    }
}

#[test]
fn a_scene_without_effects_plans_exactly_as_before() {
    let mut state = session();
    assert!(passes(&mut state, 10).is_empty());
    let listing = ok(&mut state, json!({"op":"scene.fx.list","scene":"street"}));
    assert_eq!(listing["clip_count"], 0);
    let plan = plan_json(&mut state, 10);
    assert!(
        plan.get("fx").is_none(),
        "the key is additive and absent without effects"
    );
    assert!(
        !plan["requires"]
            .as_array()
            .unwrap()
            .contains(&json!("outputFx"))
    );
    assert!(document(&state).get("fx").is_none());
}

// ---- editing: create, set, remove, one undo entry ------------------------------------------

#[test]
fn a_clip_is_created_edited_and_removed_one_undo_entry_each() {
    let mut state = session();
    let baseline = document(&state);
    let depth = undo_depth(&state);

    let created = edit(&mut state, json!([add(8, 24, json!({"intensity": 0.5}))]));
    assert_eq!(created["changed"], true);
    assert_eq!(created["clip_count"], 1);
    assert_eq!(undo_depth(&state), depth + 1);
    let listing = ok(&mut state, json!({"op":"scene.fx.list","scene":"street"}));
    let clip = &listing["tracks"][0]["clips"][0];
    assert_eq!(clip["id"], "v1");
    assert_eq!(clip["params"]["intensity"], 0.5);
    assert_eq!(
        clip["params"]["radius"], 0.75,
        "defaults are resolved in the answer"
    );
    assert_eq!(clip["end_frame_exclusive"], 32);
    assert_eq!(document(&state)["fx"]["catalogVersion"], 1);

    // A parameter, a field, hidden: each its own command, each undoable.
    edit(
        &mut state,
        json!([{"op":"set_param","clip":"v1","param":"radius","value":0.4},
               {"op":"set_param","clip":"v1","param":"color","value":{"r":0.1,"g":0.0,"b":0.0}},
               {"op":"set_field","clip":"v1","field":"fade_in_frames","value":6}]),
    );
    assert_eq!(undo_depth(&state), depth + 2, "a batch is one entry");
    let clip = ok(&mut state, json!({"op":"scene.fx.list","scene":"street"}))["tracks"][0]["clips"]
        [0]
    .clone();
    assert_eq!(clip["params"]["radius"], 0.4);
    assert_eq!(clip["params"]["color"], json!({"r":0.1,"g":0.0,"b":0.0}));
    assert_eq!(clip["fade_in_frames"], 6);

    edit(
        &mut state,
        json!([{"op":"set_hidden","clip":"v1","hidden":true}]),
    );
    assert!(
        passes(&mut state, 12).is_empty(),
        "a hidden clip draws nothing"
    );
    edit(
        &mut state,
        json!([{"op":"set_hidden","clip":"v1","hidden":false}]),
    );
    assert_eq!(passes(&mut state, 12).len(), 1);

    let removed = edit(&mut state, json!([{"op":"remove_clip","clip":"v1"}]));
    assert_eq!(removed["clip_count"], 0);
    assert_eq!(
        document(&state),
        baseline,
        "add then remove gives the document back"
    );

    // Undo walks back one entry at a time; redo returns.
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(passes(&mut state, 12).len(), 1);
    for _ in 0..4 {
        ok(&mut state, json!({"op":"history.undo"}));
    }
    assert_eq!(document(&state), baseline);
    assert_eq!(undo_depth(&state), depth);
    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(passes(&mut state, 12).len(), 1);
}

#[test]
fn a_no_op_and_a_dry_run_leave_no_trace() {
    let mut state = session();
    edit(&mut state, json!([add(0, 12, json!({}))]));
    let depth = undo_depth(&state);
    let before = document(&state);

    let same = edit(
        &mut state,
        json!([{"op":"set_field","clip":"v1","field":"start_frame","value":0},
               {"op":"set_hidden","clip":"v1","hidden":false}]),
    );
    assert_eq!(same["changed"], false);
    assert_eq!(same["noop"], true);
    assert_eq!(undo_depth(&state), depth);
    assert_eq!(document(&state), before);

    let dry = ok(
        &mut state,
        json!({"op":"scene.fx.edit","scene":"street","dry_run":true,
               "ops":[{"op":"set_param","clip":"v1","param":"intensity","value":0.2}]}),
    );
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["would_change"], true);
    assert_eq!(dry["changed"], false);
    assert_eq!(undo_depth(&state), depth);
    assert_eq!(document(&state), before);
}

#[test]
fn clips_without_a_track_go_to_the_first_track_where_they_fit() {
    let mut state = session();
    edit(
        &mut state,
        json!([{"op":"add_clip","effect":"vignette","start_frame":0,"duration_frames":20,"id":"a"},
               {"op":"add_clip","effect":"vignette","start_frame":10,"duration_frames":20,"id":"b"},
               {"op":"add_clip","effect":"vignette","start_frame":20,"duration_frames":10,"id":"c"}]),
    );
    let listing = ok(&mut state, json!({"op":"scene.fx.list","scene":"street"}));
    assert_eq!(listing["track_count"], 2, "b overlaps a: a second track");
    let on = |track: usize| -> Vec<&str> {
        listing["tracks"][track]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .map(|clip| clip["id"].as_str().unwrap())
            .collect()
    };
    assert_eq!(on(0), ["a", "c"], "c fits after a on the first track");
    assert_eq!(on(1), ["b"]);
    // Both tracks draw at frame 12, in track order (the order of composition).
    let at12 = passes(&mut state, 12);
    assert_eq!(
        at12.iter()
            .map(|pass| pass["clipId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    // Removing the last clip of a track removes the track.
    edit(&mut state, json!([{"op":"remove_clip","clip":"b"}]));
    assert_eq!(
        ok(&mut state, json!({"op":"scene.fx.list","scene":"street"}))["track_count"],
        1
    );
}

// ---- refusals are typed and atomic -----------------------------------------------------------

#[test]
fn a_refused_batch_changes_nothing_and_names_the_op() {
    let mut state = session();
    edit(&mut state, json!([add(8, 24, json!({}))]));
    let depth = undo_depth(&state);
    let before = document(&state);

    let cases = [
        (
            json!([{"op":"set_param","clip":"v1","param":"intensity","value":0.9},
                   {"op":"set_param","clip":"v1","param":"intensity","value":1.5}]),
            "param_out_of_range",
            Some(1),
        ),
        (
            json!([{"op":"set_param","clip":"v1","param":"radius","value":"wide"}]),
            "param_type",
            Some(0),
        ),
        (
            json!([{"op":"set_param","clip":"v1","param":"glow","value":1}]),
            "unknown_param",
            Some(0),
        ),
        (
            json!([{"op":"add_clip","effect":"blur","start_frame":0}]),
            "unknown_effect",
            Some(0),
        ),
        (
            json!([{"op":"remove_clip","clip":"nope"}]),
            "unknown_fx_clip",
            Some(0),
        ),
        (
            json!([{"op":"add_clip","effect":"vignette","start_frame":90,"duration_frames":20}]),
            "invalid_bundle",
            Some(0),
        ),
        (
            json!([{"op":"add_clip","effect":"vignette","start_frame":120}]),
            "fx_clip_out_of_range",
            Some(0),
        ),
        (
            json!([{"op":"add_clip","effect":"vignette","start_frame":0,"duration_frames":0}]),
            "fx_clip_empty",
            Some(0),
        ),
        (
            json!([{"op":"add_clip","effect":"vignette","start_frame":0,"id":"v1"}]),
            "duplicate_id",
            Some(0),
        ),
        (
            json!([{"op":"add_clip","effect":"vignette","start_frame":0,"id":"track"}]),
            "reserved_id",
            Some(0),
        ),
        (
            json!([{"op":"add_clip","effect":"vignette","start_frame":10,"duration_frames":4,
                    "track":"fx-track-9"}]),
            "unknown_fx_track",
            Some(0),
        ),
        (
            json!([{"op":"set_field","clip":"v1","field":"fade_in_frames","value":30}]),
            "invalid_bundle",
            Some(0),
        ),
        (json!([{"op":"explode"}]), "invalid_op", Some(0)),
    ];
    for (ops, code, index) in cases {
        let error = edit_err(&mut state, ops.clone());
        assert_eq!(error.code, code, "{ops}: {error:?}");
        if let Some(index) = index {
            assert_eq!(
                error
                    .details
                    .as_ref()
                    .and_then(|details| details["index"].as_u64()),
                Some(index),
                "{ops}: {error:?}"
            );
        }
        assert_eq!(document(&state), before, "{ops}");
        assert_eq!(undo_depth(&state), depth, "{ops}");
    }
}

#[test]
fn an_overlap_on_one_track_is_refused_with_the_importer_code_and_the_clip() {
    let mut state = session();
    edit(&mut state, json!([add(8, 24, json!({}))]));
    let listing = ok(&mut state, json!({"op":"scene.fx.list","scene":"street"}));
    let track = listing["tracks"][0]["id"].as_str().unwrap().to_string();
    let error = edit_err(
        &mut state,
        json!([{"op":"add_clip","effect":"vignette","start_frame":20,"duration_frames":10,
                "id":"w","track":track}]),
    );
    assert_eq!(error.code, "invalid_bundle", "{error:?}");
    let issues = error.details.as_ref().unwrap()["issues"]
        .as_array()
        .unwrap();
    assert_eq!(issues[0]["code"], "FX_CLIP_OVERLAP", "{issues:?}");
    assert_eq!(issues[0]["ops"], json!([0]));
}

#[test]
fn a_bundle_that_asks_for_what_is_not_implemented_is_refused_at_import() {
    let try_with = |change: &dyn Fn(&mut Value)| -> Vec<String> {
        let mut document = bundle();
        document["fx"] = json!({"catalogVersion":1,"tracks":[{"id":"t","label":"FX","clips":[
            {"id":"c","effect":"vignette","startFrame":0,"durationFrames":10}]}],"transitions":[]});
        change(&mut document);
        match DirectedScene::import_value(document) {
            Ok(_) => Vec::new(),
            Err(error) => error
                .issues()
                .iter()
                .map(|issue| issue.code.to_string())
                .collect(),
        }
    };
    assert!(try_with(&|_| {}).is_empty(), "the base layer is valid");
    assert_eq!(
        try_with(&|d| d["fx"]["tracks"][0]["clips"][0]["keys"] = json!({"radius":[]})),
        ["FX_KEYS_NOT_YET"]
    );
    assert_eq!(
        try_with(
            &|d| d["fx"]["tracks"][0]["clips"][0]["scope"] = json!({"kind":"shot","shotId":"s1"})
        ),
        ["FX_SCOPE_NOT_YET"]
    );
    assert_eq!(
        try_with(&|d| d["fx"]["transitions"] = json!([{"id":"x","effect":"crossDissolve"}])),
        ["FX_TRANSITION_NOT_YET"]
    );
    assert_eq!(
        try_with(&|d| d["fx"]["tracks"][0]["clips"][0]["effect"] = json!("glitch")),
        ["UNKNOWN_EFFECT"]
    );
    assert_eq!(
        try_with(&|d| d["fx"]["tracks"][0]["clips"][0]["params"] = json!({"radius": 9})),
        ["PARAM_OUT_OF_RANGE"]
    );
    assert_eq!(
        try_with(&|d| d["fx"]["catalogVersion"] = json!(2)),
        ["CATALOG_TOO_NEW"]
    );
    assert_eq!(
        try_with(&|d| d["fx"]["tracks"][0]["clips"][0]["startFrame"] = json!(-1)),
        ["FX_CLIP_OUT_OF_RANGE"]
    );
    assert_eq!(
        try_with(&|d| {
            let clip = d["fx"]["tracks"][0]["clips"][0].clone();
            let mut other = clip;
            other["id"] = json!("d");
            other["startFrame"] = json!(5);
            d["fx"]["tracks"][0]["clips"]
                .as_array_mut()
                .unwrap()
                .push(other);
        }),
        ["FX_CLIP_OVERLAP"]
    );
    assert_eq!(
        try_with(&|d| {
            d["fx"]["tracks"][0]["clips"][0]["fadeInFrames"] = json!(8);
            d["fx"]["tracks"][0]["clips"][0]["fadeOutFrames"] = json!(8);
        }),
        ["ENVELOPE_TOO_LONG"]
    );
}

// ---- evaluation: deterministic, per frame --------------------------------------------------

#[test]
fn the_envelope_is_sampled_at_the_middle_of_the_frame() {
    let mut state = session();
    edit(
        &mut state,
        json!([add(8, 24, json!({"intensity": 0.5})),
               {"op":"set_field","clip":"v1","field":"fade_in_frames","value":6},
               {"op":"set_field","clip":"v1","field":"fade_out_frames","value":6}]),
    );
    let amount = |state: &mut EditorState, frame: i64| -> Option<f64> {
        passes(state, frame)
            .first()
            .map(|pass| pass["amount"].as_f64().unwrap())
    };
    assert_eq!(amount(&mut state, 7), None, "before the clip");
    assert_eq!(amount(&mut state, 32), None, "the end is exclusive");
    let first = amount(&mut state, 8).unwrap();
    assert!((first - 0.5 * (0.5 / 6.0)).abs() < 1e-12, "{first}");
    let middle = amount(&mut state, 20).unwrap();
    assert!((middle - 0.5).abs() < 1e-12, "{middle}");
    let last = amount(&mut state, 31).unwrap();
    assert!((last - 0.5 * (0.5 / 6.0)).abs() < 1e-12, "{last}");
    // A function of the frame alone: any order, any repetition.
    let in_order: Vec<_> = (0..96).map(|frame| amount(&mut state, frame)).collect();
    let mut shuffled: Vec<_> = (0..96)
        .rev()
        .map(|frame| (frame, amount(&mut state, frame)))
        .collect();
    shuffled.sort_by_key(|(frame, _)| *frame);
    assert_eq!(
        in_order,
        shuffled
            .into_iter()
            .map(|(_, amount)| amount)
            .collect::<Vec<_>>()
    );
}

// ---- persistence and reopen -------------------------------------------------------------------

#[test]
fn the_effects_survive_save_and_reopen_and_evaluate_the_same() {
    let mut state = session();
    edit(
        &mut state,
        json!([add(8, 24, json!({"intensity": 0.8, "center": {"x": 0.3, "y": 0.4}})),
               {"op":"set_field","clip":"v1","field":"fade_in_frames","value":4}]),
    );
    let scratch = std::env::temp_dir().join(format!("tessel-fx-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    let path = scratch.join("fx.tessel.json");
    persist::save(&mut state, &path).unwrap();

    let mut reopened = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(&mut reopened, json!({"op":"project.open","path":path}));
    assert_eq!(document(&reopened), document(&state));
    for frame in [0, 8, 10, 20, 31, 32] {
        assert_eq!(
            passes(&mut reopened, frame),
            passes(&mut state, frame),
            "frame {frame}"
        );
    }
    // The reopened project undoes and redoes its own edits from there.
    edit_on(&mut reopened);
    fs::remove_dir_all(&scratch).unwrap();
}

fn edit_on(state: &mut EditorState) {
    let depth = undo_depth(state);
    edit(
        state,
        json!([{"op":"set_param","clip":"v1","param":"intensity","value":0.1}]),
    );
    assert_eq!(undo_depth(state), depth + 1);
    ok(state, json!({"op":"history.undo"}));
    let clip =
        ok(state, json!({"op":"scene.fx.list","scene":"street"}))["tracks"][0]["clips"][0].clone();
    assert_eq!(clip["params"]["intensity"], 0.8);
}

// ---- the plan carries the resolved passes ---------------------------------------------------------

#[test]
fn the_plan_carries_the_resolved_passes_and_asks_for_the_capability() {
    let mut state = session();
    edit(
        &mut state,
        json!([add(8, 24, json!({"intensity": 0.5, "color": {"r": 0.0, "g": 0.0, "b": 0.2}})),
               {"op":"set_field","clip":"v1","field":"fade_in_frames","value":6}]),
    );
    let plan = plan_json(&mut state, 8);
    assert_eq!(
        plan["requires"].as_array().unwrap().last(),
        Some(&json!("outputFx"))
    );
    let pass = &plan["fx"][0];
    assert_eq!(pass["effect"], "vignette");
    assert_eq!(pass["clipId"], "v1");
    assert_eq!(pass["frameInClip"], 0);
    assert_eq!(pass["scope"], json!({"kind":"output"}));
    assert_eq!(pass["params"]["color"], json!({"r":0.0,"g":0.0,"b":0.2}));
    // The very same pass the executor reports for the frame.
    assert_eq!(passes_of(&plan), passes(&mut state, 8));
    // Outside the clip the plan has no key at all.
    assert!(plan_json(&mut state, 3).get("fx").is_none());

    let scene = scene_of(&state);
    let built = ScenePlan::build(&scene, 8).unwrap();
    assert_eq!(built.fx.len(), 1);
    assert_eq!(
        ScenePlan::from_json(&built.to_json()).unwrap(),
        built,
        "the wire round-trips"
    );
    // A runtime without the capability is refused by name, never drawn without the effect.
    let without: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|capability| *capability != Capability::OutputFx)
        .collect();
    assert_eq!(built.missing_capabilities(&without), [Capability::OutputFx]);
    let outside = ScenePlan::build(&scene, 3).unwrap();
    assert_eq!(outside.require_capabilities(&without), Ok(()));
}

fn passes_of(plan: &Value) -> Vec<Value> {
    plan["fx"].as_array().cloned().unwrap_or_default()
}

#[test]
fn the_timeline_gives_the_plan_of_a_build_for_every_frame() {
    let mut state = session();
    edit(
        &mut state,
        json!([add(8, 24, json!({"intensity": 0.7})),
               {"op":"set_field","clip":"v1","field":"fade_out_frames","value":5},
               {"op":"add_clip","effect":"vignette","start_frame":20,"duration_frames":16,"id":"v2",
                "params":{"radius": 0.5}}]),
    );
    let scene = scene_of(&state);
    let timeline = PlanTimeline::new(&scene);
    // Out of order on purpose: the template of a shot is made by whichever frame comes first.
    for frame in (0..96).rev().chain(0..96) {
        assert_eq!(
            timeline.build(frame),
            ScenePlan::build(&scene, frame),
            "frame {frame}"
        );
    }
    let during = timeline.build(24).unwrap();
    assert_eq!(during.fx.len(), 2);
    assert!(during.requires.contains(&Capability::OutputFx));
    let before = timeline.build(2).unwrap();
    assert!(before.fx.is_empty());
    assert!(
        !before.requires.contains(&Capability::OutputFx),
        "requires follows the frame"
    );
}

#[test]
fn a_plan_whose_amount_is_not_intensity_times_weight_is_refused() {
    let mut state = session();
    edit(&mut state, json!([add(8, 24, json!({"intensity": 0.5}))]));
    let plan = plan_json(&mut state, 12);
    let mut wire = plan.clone();
    wire["fx"][0]["amount"] = json!(0.9);
    let error = ScenePlan::from_json(&wire).unwrap_err();
    assert!(error.to_string().contains("/fx/0/amount"), "{error}");
    let mut wire = plan.clone();
    wire["fx"][0]["params"]["radius"] = json!(7.0);
    assert!(ScenePlan::from_json(&wire).is_err());
    let mut wire = plan.clone();
    wire["fx"][0]["effect"] = json!("glitch");
    assert!(ScenePlan::from_json(&wire).is_err());
    let mut wire = plan.clone();
    wire["requires"] = json!([
        "camera",
        "nodeTransforms",
        "sockets",
        "performanceChannels",
        "imagePlanes",
        "straightAlpha",
        "opaqueAlpha",
        "compositionPassOrder"
    ]);
    assert!(
        ScenePlan::from_json(&wire).is_err(),
        "fx without outputFx in requires"
    );
    assert!(ScenePlan::from_json(&plan).is_ok());
}

// ---- pixels: the reference of the math, against a baseline ---------------------------------

fn picture(width: usize, height: usize) -> Vec<u8> {
    // A mid grey with a gradient so a wrong pixel cannot hide in a flat field.
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let base = 120 + ((x * 40) / width) as u8;
            rgba.extend_from_slice(&[base, base.saturating_add((y * 30 / height) as u8), 150, 255]);
        }
    }
    rgba
}

fn corner_and_centre(rgba: &[u8], width: usize, height: usize) -> (u8, u8) {
    let at = |x: usize, y: usize| rgba[(y * width + x) * 4];
    (at(0, 0), at(width / 2, height / 2))
}

#[test]
fn no_effect_and_intensity_zero_keep_the_baseline_and_intensity_changes_the_edges() {
    let (width, height) = (64, 36);
    let baseline = picture(width, height);

    let mut state = session();
    edit(&mut state, json!([add(8, 24, json!({"intensity": 0.0}))]));
    let scene = scene_of(&state);
    let apply = |scene: &DirectedScene, frame: i64| {
        let mut rgba = baseline.clone();
        for pass in scene.bundle().fx.resolve(frame) {
            assert!(fx::apply_pass_to_rgba8(&pass, &mut rgba, width, height));
        }
        rgba
    };
    // Outside any clip and inside a clip of intensity 0: the very bytes of the baseline.
    assert_eq!(apply(&scene, 2), baseline, "no effect at this frame");
    assert_eq!(apply(&scene, 12), baseline, "intensity 0 is the identity");

    edit(
        &mut state,
        json!([{"op":"set_param","clip":"v1","param":"intensity","value":1.0}]),
    );
    let scene = scene_of(&state);
    let shaded = apply(&scene, 12);
    assert_ne!(shaded, baseline);
    let (base_corner, base_centre) = corner_and_centre(&baseline, width, height);
    let (corner, centre) = corner_and_centre(&shaded, width, height);
    assert_eq!(
        centre, base_centre,
        "the middle of the picture is untouched"
    );
    assert!(
        corner + 20 < base_corner,
        "the corner darkens: {corner} < {base_corner}"
    );
    // Alpha is untouched everywhere.
    assert!(shaded.iter().skip(3).step_by(4).all(|alpha| *alpha == 255));

    // A discriminating comparison: moving the centre moves the dark region with it.
    edit(
        &mut state,
        json!([{"op":"set_param","clip":"v1","param":"center","value":{"x":1.0,"y":1.0}}]),
    );
    let scene = scene_of(&state);
    let moved = apply(&scene, 12);
    let at = |picture: &[u8], x: usize, y: usize| picture[(y * width + x) * 4];
    assert!(
        at(&moved, 0, 0) < at(&moved, width - 1, height - 1),
        "now the bottom right is clear"
    );
    assert_ne!(moved, shaded);
    // And the reference agrees with the plan's own number for number: the resolved param.
    let pass = &scene.bundle().fx.resolve(12)[0];
    assert_eq!(
        pass.param("center"),
        Some(FxValue::Point { x: 1.0, y: 1.0 })
    );
}

#[test]
fn the_core_stays_free_of_windows_and_webviews() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in ["gpui", "wry"] {
        assert!(!manifest.contains(forbidden), "{forbidden}");
    }
}

#[test]
fn the_headless_executable_edits_effects_like_the_library_and_refuses_the_same_way() {
    let bundle_line = json!({"op":"scene.attach","id":"street","bundle":bundle()}).to_string();
    let requests = [
        bundle_line,
        json!({"op":"scene.fx.catalog"}).to_string(),
        edit_request(json!([add(10, 20, json!({"intensity": 0.5}))])),
        json!({"op":"scene.fx.frame","scene":"street","frame":15}).to_string(),
        json!({"op":"scene.fx.frame","scene":"street","frame":40}).to_string(),
        edit_request(json!([{"op":"set_param","clip":"v1","param":"intensity","value":2.0}])),
        edit_request(json!([{"op":"remove_clip","clip":"v1"}])),
        json!({"op":"scene.fx.list","scene":"street"}).to_string(),
        json!({"op":"history.undo"}).to_string(),
        json!({"op":"scene.fx.list","scene":"street"}).to_string(),
    ];
    let input = requests.join("\n");
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let mut library = Vec::new();
    headless::run_session(&mut state, None, input.as_bytes(), &mut library).unwrap();

    let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
        .arg("--empty")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    // A session with a refused request exits 1 (the executable's contract), after answering all.
    assert_eq!(output.status.code(), Some(1));
    let lines = |bytes: &[u8]| -> Vec<Value> {
        String::from_utf8_lossy(bytes)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };
    let answers = lines(&output.stdout);
    assert_eq!(
        answers,
        lines(&library),
        "the process answers like the library"
    );

    assert_eq!(answers[1]["result"]["effects"][0]["id"], "vignette");
    assert_eq!(answers[2]["result"]["changed"], true);
    assert_eq!(
        answers[3]["result"]["count"], 1,
        "frame 15 is inside the clip"
    );
    assert_eq!(answers[3]["result"]["passes"][0]["amount"], 0.5);
    assert_eq!(answers[4]["result"]["count"], 0, "frame 40 is outside it");
    assert_eq!(answers[5]["ok"], false, "{:?}", answers[5]);
    assert_eq!(answers[5]["error"]["code"], "param_out_of_range");
    assert_eq!(answers[6]["result"]["changed"], true);
    assert_eq!(answers[7]["result"]["clip_count"], 0);
    assert_eq!(
        answers[9]["result"]["clip_count"], 1,
        "undo brought the clip back"
    );
}

fn edit_request(ops: Value) -> String {
    json!({"op":"scene.fx.edit","scene":"street","ops":ops}).to_string()
}
