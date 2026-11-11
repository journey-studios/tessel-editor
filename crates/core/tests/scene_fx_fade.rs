//! F016 / F031 (BL029, BL040, BL082.1): the second look (`fade`) and the neutral inspector model,
//! through the library and the JSON executor `tessel-headless` and `ui op` share (no window, no GPU).
//!
//! What it proves: a fade is created, edited, hidden and removed by the same ops as the vignette
//! (one undo entry each, a no-op or a refusal leaves nothing), previewed as the core's candidate,
//! committed or cancelled, saved and reopened; the plan carries both looks in the stacking order;
//! more than eight passes warn and drop nothing. `scene.fx.inspect` is derived from the catalog
//! and the state: every offered effect has every field and text in the three languages, the
//! gesture candidate and the typed refusals show through it, and a probe judges a value without
//! writing. The pixels are compared with a formula transcribed in this file (the pilot's fade:
//! `mix(source, color, amount × intensity × weight)` in linear light), not with the routine under
//! test: the baseline, intensity/amount 0, a positive colour, the stacking order, preview ==
//! commit and cancel/undo back to the baseline. The real window and the export are
//! `scripts/scene-fx-ui-proof.py` and `crates/render/tests/scene_fx*.rs`.

use std::fs;
use std::io::Write;
use std::process::{Command as Process, Stdio};

use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::directed_scene::fx::{self, FxValue};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless, persist};

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

fn footprint(state: &EditorState) -> (Value, u64, u64) {
    (document(state), undo_depth(state), state.project.revision)
}

fn edit(state: &mut EditorState, ops: Value) -> Value {
    ok(
        state,
        json!({"op":"scene.fx.edit","scene":"street","ops":ops}),
    )
}

fn add_fade(id: &str, start: i64, duration: i64, params: Value) -> Value {
    json!({"op":"add_clip","effect":"fade","start_frame":start,"duration_frames":duration,
        "id":id,"params":params})
}

fn add_vignette(id: &str, start: i64, duration: i64, params: Value) -> Value {
    json!({"op":"add_clip","effect":"vignette","start_frame":start,"duration_frames":duration,
        "id":id,"params":params})
}

fn set(clip: &str, param: &str, value: Value) -> Value {
    json!({"op":"set_param","clip":clip,"param":param,"value":value})
}

fn list(state: &mut EditorState) -> Value {
    ok(state, json!({"op":"scene.fx.list","scene":"street"}))
}

fn clip_of(listing: &Value, id: &str) -> Value {
    listing["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap())
        .find(|clip| clip["id"] == id)
        .unwrap_or_else(|| panic!("no clip {id} in {listing}"))
        .clone()
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

fn scene_of(state: &EditorState) -> DirectedScene {
    DirectedScene::import_value(document(state)).unwrap()
}

// ---- the lifecycle of a fade -----------------------------------------------------------------

#[test]
fn a_fade_is_created_edited_hidden_and_removed_by_the_same_ops_one_undo_entry_each() {
    let mut state = session();
    let baseline = document(&state);
    let depth = undo_depth(&state);

    let created = edit(&mut state, json!([add_fade("f1", 8, 24, json!({}))]));
    assert_eq!(created["changed"], true);
    assert_eq!(undo_depth(&state), depth + 1);
    let clip = clip_of(&list(&mut state), "f1");
    assert_eq!(clip["effect"], "fade");
    assert_eq!(clip["params"]["amount"], 1.0, "defaults are resolved");
    assert_eq!(clip["params"]["intensity"], 1.0);
    assert_eq!(clip["params"]["color"], json!({"r":0.0,"g":0.0,"b":0.0}));

    edit(
        &mut state,
        json!([
            set("f1", "amount", json!(0.4)),
            set("f1", "color", json!({"r":1.0,"g":0.5,"b":0.0}))
        ]),
    );
    assert_eq!(undo_depth(&state), depth + 2, "a batch is one entry");
    let clip = clip_of(&list(&mut state), "f1");
    assert_eq!(clip["params"]["amount"], 0.4);
    assert_eq!(clip["params"]["color"], json!({"r":1.0,"g":0.5,"b":0.0}));

    // Hidden draws nothing; shown draws one pass whose amount is intensity × weight.
    edit(
        &mut state,
        json!([{"op":"set_hidden","clip":"f1","hidden":true}]),
    );
    assert!(passes(&mut state, 12).is_empty());
    edit(
        &mut state,
        json!([{"op":"set_hidden","clip":"f1","hidden":false}]),
    );
    let drawn = passes(&mut state, 12);
    assert_eq!(drawn.len(), 1);
    assert_eq!(drawn[0]["effect"], "fade");
    assert_eq!(drawn[0]["amount"], 1.0);
    assert_eq!(drawn[0]["params"]["amount"], 0.4);

    // The same value again is no entry.
    let depth = undo_depth(&state);
    let same = edit(&mut state, json!([set("f1", "amount", json!(0.4))]));
    assert_eq!(same["changed"], false);
    assert_eq!(same["noop"], true);
    assert_eq!(undo_depth(&state), depth);

    edit(&mut state, json!([{"op":"remove_clip","clip":"f1"}]));
    assert_eq!(
        document(&state),
        baseline,
        "add then remove is the baseline"
    );
    for _ in 0..5 {
        ok(&mut state, json!({"op":"history.undo"}));
    }
    assert_eq!(document(&state), baseline);
    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(clip_of(&list(&mut state), "f1")["params"]["amount"], 1.0);
}

#[test]
fn a_refused_fade_edit_is_atomic_and_names_the_op() {
    let mut state = session();
    edit(&mut state, json!([add_fade("f1", 0, 24, json!({}))]));
    let before = footprint(&state);

    // A good op first and a bad one second: nothing of the batch lands.
    for bad in [
        set("f1", "amount", json!(1.5)),
        set("f1", "amount", json!(-0.1)),
        set("f1", "color", json!({"r":2.0,"g":0.0,"b":0.0})),
        set("f1", "radius", json!(0.2)),
    ] {
        let error = err(
            &mut state,
            json!({"op":"scene.fx.edit","scene":"street",
                   "ops":[set("f1","intensity",json!(0.3)), bad.clone()]}),
        );
        assert!(
            ["param_out_of_range", "unknown_param", "invalid_value"].contains(&error.code),
            "{bad}: {error:?}"
        );
        assert_eq!(footprint(&state), before, "{bad}: nothing changed");
    }
}

// ---- the plan: both looks, in order, with the budget ------------------------------------------

#[test]
fn the_plan_carries_both_looks_in_track_order_and_asks_for_the_capability() {
    let mut state = session();
    edit(
        &mut state,
        json!([
            add_fade("f1", 0, 40, json!({"amount": 0.5, "intensity": 0.5})),
            add_vignette("v1", 0, 40, json!({"intensity": 0.8}))
        ]),
    );
    let plan = ok(
        &mut state,
        json!({"op":"scene.plan","scene":"street","frame":10}),
    )["plan"]
        .clone();
    assert_eq!(
        plan["requires"].as_array().unwrap().last(),
        Some(&json!("outputFx"))
    );
    let effects: Vec<&str> = plan["fx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pass| pass["effect"].as_str().unwrap())
        .collect();
    assert_eq!(effects, ["fade", "vignette"], "the plan is in track order");
    assert_eq!(plan["fx"][0]["amount"], 0.5, "intensity × weight");
    assert_eq!(plan["fx"][0]["params"]["amount"], 0.5);

    // A plan whose amount is not intensity × weight is refused for a fade too.
    let mut wire = plan.clone();
    wire["fx"][0]["amount"] = json!(0.9);
    let error = tessel_editor_core::scene_plan::ScenePlan::from_json(&wire).unwrap_err();
    assert!(error.to_string().contains("/fx/0/amount"), "{error}");
    let mut wire = plan.clone();
    wire["fx"][0]["params"]["amount"] = json!(3.0);
    assert!(tessel_editor_core::scene_plan::ScenePlan::from_json(&wire).is_err());
    assert!(tessel_editor_core::scene_plan::ScenePlan::from_json(&plan).is_ok());
}

#[test]
fn the_ninth_pass_warns_with_a_fade_among_them_and_no_pass_is_dropped() {
    let mut state = session();
    let mut ops = vec![add_fade("f0", 0, 20, json!({}))];
    for n in 0..8 {
        ops.push(add_vignette(&format!("v{n}"), 0, 20, json!({})));
    }
    edit(&mut state, Value::Array(ops));
    let listing = list(&mut state);
    assert_eq!(listing["budget"]["peak_passes"], 9);
    assert_eq!(listing["budget"]["over_budget"], true);
    assert!(!listing["warnings"].as_array().unwrap().is_empty());
    assert_eq!(passes(&mut state, 5).len(), 9, "the warning drops nothing");
    // Eight is the budget: hiding the fade clears the warning.
    edit(
        &mut state,
        json!([{"op":"set_hidden","clip":"f0","hidden":true}]),
    );
    assert_eq!(list(&mut state)["budget"]["over_budget"], false);
}

// ---- gesture: preview, commit, cancel, refusal ------------------------------------------------------

fn begin(state: &mut EditorState, clip: &str) -> String {
    ok(
        state,
        json!({"op":"scene.fx.gesture.begin","scene":"street","clip":clip}),
    )["gesture"]
        .as_str()
        .unwrap()
        .to_string()
}

fn update(state: &mut EditorState, gesture: &str, ops: Value) -> Result<Value, CommandError> {
    headless::execute(
        state,
        &json!({"op":"scene.fx.gesture.update","gesture":gesture,"ops":ops}),
    )
}

#[test]
fn a_fade_gesture_previews_the_candidate_commits_once_and_cancel_leaves_nothing() {
    let mut state = session();
    edit(&mut state, json!([add_fade("f1", 0, 40, json!({}))]));
    let before = footprint(&state);

    let gesture = begin(&mut state, "f1");
    update(
        &mut state,
        &gesture,
        json!([set("f1", "amount", json!(0.3))]),
    )
    .unwrap();
    // A refusal keeps the last legal candidate.
    let refused = update(
        &mut state,
        &gesture,
        json!([set("f1", "amount", json!(9.0))]),
    );
    assert!(refused.is_err());
    let shown = ok(
        &mut state,
        json!({"op":"scene.fx.list","scene":"street","gesture":gesture}),
    );
    assert_eq!(shown["source"], "gesture");
    assert_eq!(clip_of(&shown, "f1")["params"]["amount"], 0.3);
    assert_eq!(footprint(&state), before, "the preview writes nothing");

    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.cancel","gesture":gesture}),
    );
    assert_eq!(footprint(&state), before, "a cancel leaves nothing");

    let gesture = begin(&mut state, "f1");
    for value in [0.2, 0.5, 0.7] {
        update(
            &mut state,
            &gesture,
            json!([set("f1", "amount", json!(value))]),
        )
        .unwrap();
    }
    let committed = ok(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    assert_eq!(committed["changed"], true);
    assert_eq!(
        undo_depth(&state),
        before.1 + 1,
        "the whole drag is one entry"
    );
    assert_eq!(clip_of(&list(&mut state), "f1")["params"]["amount"], 0.7);
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(document(&state), before.0);
}

// ---- persistence ---------------------------------------------------------------------------------

#[test]
fn both_looks_survive_save_and_reopen_and_undo_and_redo_from_there() {
    let mut state = session();
    edit(
        &mut state,
        json!([add_fade("f1", 4, 30, json!({"amount": 0.6, "color": {"r":1.0,"g":1.0,"b":1.0}})),
               add_vignette("v1", 4, 30, json!({"intensity": 0.4})),
               {"op":"set_field","clip":"f1","field":"fade_in_frames","value":6}]),
    );
    let scratch = std::env::temp_dir().join(format!("tessel-fade-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    let path = scratch.join("fade.tessel.json");
    persist::save(&mut state, &path).unwrap();

    let mut reopened = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(&mut reopened, json!({"op":"project.open","path":path}));
    assert_eq!(document(&reopened), document(&state));
    for frame in [0, 4, 7, 20, 33, 34] {
        assert_eq!(
            passes(&mut reopened, frame),
            passes(&mut state, frame),
            "frame {frame}"
        );
    }
    let depth = undo_depth(&reopened);
    edit_clip(&mut reopened);
    assert_eq!(undo_depth(&reopened), depth + 1);
    ok(&mut reopened, json!({"op":"history.undo"}));
    assert_eq!(clip_of(&list(&mut reopened), "f1")["params"]["amount"], 0.6);
    ok(&mut reopened, json!({"op":"history.redo"}));
    assert_eq!(clip_of(&list(&mut reopened), "f1")["params"]["amount"], 0.1);
    fs::remove_dir_all(&scratch).unwrap();
}

fn edit_clip(state: &mut EditorState) {
    edit(state, json!([set("f1", "amount", json!(0.1))]));
}

// ---- pixels: a formula transcribed here, against the reference of the core ----------------------------------

const WIDTH: usize = 64;
const HEIGHT: usize = 36;

fn picture() -> Vec<u8> {
    let mut rgba = Vec::with_capacity(WIDTH * HEIGHT * 4);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let base = 60 + ((x * 120) / WIDTH) as u8;
            rgba.extend_from_slice(&[base, base.saturating_add((y * 50 / HEIGHT) as u8), 150, 255]);
        }
    }
    rgba
}

fn to_linear(byte: u8) -> f64 {
    let c = f64::from(byte) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_byte(linear: f64) -> f64 {
    let l = linear.clamp(0.0, 1.0);
    let c = if l <= 0.0031308 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round()
}

/// The pilot's fade, written out: `mix(source, colour, amount)` in linear light, uniform over the
/// picture, with the colour as the linear value the plan carries.
fn pilot_fade(rgba: &[u8], colour: [f64; 3], weight: f64) -> Vec<u8> {
    let mut out = rgba.to_vec();
    if weight <= 0.0 {
        return out;
    }
    for pixel in out.as_chunks_mut::<4>().0.iter_mut() {
        for channel in 0..3 {
            let source = to_linear(pixel[channel]);
            pixel[channel] = to_byte(source + (colour[channel] - source) * weight) as u8;
        }
    }
    out
}

fn render(scene: &DirectedScene, frame: i64, base: &[u8]) -> Vec<u8> {
    let mut rgba = base.to_vec();
    for pass in scene.bundle().fx_passes(frame) {
        assert!(fx::apply_pass_to_rgba8(&pass, &mut rgba, WIDTH, HEIGHT));
    }
    rgba
}

fn within(left: &[u8], right: &[u8], tolerance: u8) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| a.abs_diff(*b) <= tolerance)
}

#[test]
fn a_fade_in_the_reference_matches_the_pilot_formula_and_zero_is_the_exact_baseline() {
    let base = picture();
    let mut state = session();

    // No clip, intensity 0 and amount 0 are the very bytes of the baseline.
    assert_eq!(render(&scene_of(&state), 3, &base), base);
    edit(
        &mut state,
        json!([add_fade(
            "f1",
            0,
            40,
            json!({"intensity": 0.0, "color": {"r":1.0,"g":1.0,"b":1.0}})
        )]),
    );
    assert_eq!(render(&scene_of(&state), 10, &base), base, "intensity 0");
    edit(
        &mut state,
        json!([
            set("f1", "intensity", json!(1.0)),
            set("f1", "amount", json!(0.0))
        ]),
    );
    assert_eq!(render(&scene_of(&state), 10, &base), base, "amount 0");

    // Positive amount towards white and towards a colour, uniform over the picture.
    edit(&mut state, json!([set("f1", "amount", json!(0.5))]));
    let half_white = render(&scene_of(&state), 10, &base);
    assert_ne!(half_white, base);
    assert!(within(
        &half_white,
        &pilot_fade(&base, [1.0, 1.0, 1.0], 0.5),
        1
    ));
    assert!(
        base.as_chunks::<4>()
            .0
            .iter()
            .zip(half_white.as_chunks::<4>().0.iter())
            .all(|(before, after)| after[3] == 255
                && after[..3].iter().zip(&before[..3]).all(|(a, b)| a >= b)),
        "towards white never darkens, alpha is untouched"
    );
    edit(
        &mut state,
        json!([
            set("f1", "color", json!({"r":1.0,"g":0.0,"b":0.0})),
            set("f1", "amount", json!(1.0)),
            set("f1", "intensity", json!(0.25))
        ]),
    );
    let tinted = render(&scene_of(&state), 10, &base);
    assert!(within(
        &tinted,
        &pilot_fade(&base, [1.0, 0.0, 0.0], 0.25),
        1
    ));
    // Full weight is the colour exactly.
    edit(&mut state, json!([set("f1", "intensity", json!(1.0))]));
    let solid = render(&scene_of(&state), 10, &base);
    assert!(
        solid
            .as_chunks::<4>()
            .0
            .iter()
            .all(|px| *px == [255, 0, 0, 255])
    );
}

#[test]
fn stacking_order_is_discriminating_and_the_plan_order_is_the_drawing_order() {
    let base = picture();
    let render_order = |first_fade: bool| {
        let mut state = session();
        let fade = add_fade(
            "f1",
            0,
            40,
            json!({"amount": 0.7, "color": {"r":1.0,"g":1.0,"b":1.0}}),
        );
        let vignette = add_vignette(
            "v1",
            0,
            40,
            json!({"intensity": 1.0, "color": {"r":0.0,"g":0.0,"b":0.0}}),
        );
        let ops = if first_fade {
            vec![fade, vignette]
        } else {
            vec![vignette, fade]
        };
        edit(&mut state, Value::Array(ops));
        let scene = scene_of(&state);
        let order: Vec<String> = scene
            .bundle()
            .fx_passes(10)
            .iter()
            .map(|pass| pass.effect.clone())
            .collect();
        (render(&scene, 10, &base), order)
    };
    let (fade_then_vignette, order_a) = render_order(true);
    let (vignette_then_fade, order_b) = render_order(false);
    assert_eq!(order_a, ["fade", "vignette"]);
    assert_eq!(order_b, ["vignette", "fade"]);
    assert_ne!(
        fade_then_vignette, vignette_then_fade,
        "the order changes the pixels"
    );
    // At the corner the vignette is strong: drawn after the fade it darkens the lifted picture,
    // drawn before it the fade lifts what is left. The two orders are far apart there.
    let corner = |picture: &[u8]| i32::from(picture[0]);
    assert!(
        corner(&vignette_then_fade) > corner(&fade_then_vignette) + 40,
        "{} vs {}",
        corner(&vignette_then_fade),
        corner(&fade_then_vignette)
    );
}

#[test]
fn the_preview_is_the_commit_and_cancel_and_undo_go_back_to_the_baseline_pixels() {
    let base = picture();
    let mut state = session();
    edit(
        &mut state,
        json!([add_fade(
            "f1",
            0,
            40,
            json!({"amount": 0.2, "color": {"r":0.0,"g":0.0,"b":1.0}})
        )]),
    );
    let at_rest = render(&scene_of(&state), 10, &base);

    let gesture = begin(&mut state, "f1");
    update(
        &mut state,
        &gesture,
        json!([set("f1", "amount", json!(0.8))]),
    )
    .unwrap();
    // The candidate's pixels: from the plan of the gesture.
    let shown = ok(
        &mut state,
        json!({"op":"scene.fx.frame","scene":"street","frame":10,"gesture":gesture}),
    );
    assert_eq!(shown["source"], "gesture");
    assert_eq!(shown["passes"][0]["params"]["amount"], 0.8);
    assert_eq!(
        render(&scene_of(&state), 10, &base),
        at_rest,
        "the document did not move"
    );

    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.commit","gesture":gesture}),
    );
    let committed = render(&scene_of(&state), 10, &base);
    assert_ne!(committed, at_rest);
    assert!(
        within(&committed, &pilot_fade(&base, [0.0, 0.0, 1.0], 0.8), 1),
        "commit == the formula at the previewed value"
    );

    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(
        render(&scene_of(&state), 10, &base),
        at_rest,
        "undo is the pixels before"
    );
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(
        render(&scene_of(&state), 10, &base),
        base,
        "and before the clip, the baseline"
    );
}

// ---- the inspector -------------------------------------------------------------------------------------------------

fn inspect(state: &mut EditorState, extra: Value) -> Value {
    let mut request = json!({"op":"scene.fx.inspect","scene":"street"});
    for (key, value) in extra.as_object().unwrap() {
        request[key] = value.clone();
    }
    ok(state, request)
}

#[test]
fn every_offered_effect_is_inspected_with_every_field_and_text_in_three_languages() {
    let mut state = session();
    let offered: Vec<String> = inspect(&mut state, json!({}))["offers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|offer| offer["effect"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        offered,
        ["vignette", "fade", "filmGrain"],
        "the offers are the catalog"
    );

    // One clip of every offered effect, then the inspector answers for each in every language.
    let mut ops = Vec::new();
    for (n, effect) in offered.iter().enumerate() {
        ops.push(
            json!({"op":"add_clip","effect":effect,"start_frame":0,"duration_frames":40,
            "id":format!("c{n}")}),
        );
    }
    edit(&mut state, Value::Array(ops));
    for locale in ["pt-br", "en", "es"] {
        let answer = inspect(&mut state, json!({"locale": locale}));
        assert_eq!(answer["clip_count"], offered.len());
        for offer in answer["offers"].as_array().unwrap() {
            assert!(
                !offer["label"].as_str().unwrap().is_empty(),
                "{locale}: {offer}"
            );
        }
        for clip in answer["clips"].as_array().unwrap() {
            let fields = clip["fields"].as_array().unwrap();
            let ids: Vec<&str> = fields
                .iter()
                .map(|field| field["id"].as_str().unwrap())
                .collect();
            assert_eq!(ids.first(), Some(&"intensity"), "{locale}: intensity first");
            for field in fields {
                assert!(
                    !field["label"].as_str().unwrap().is_empty(),
                    "{locale}: {field}"
                );
                // A number sits in a range; a switch is true or false and has no range to show.
                let keys: &[&str] = if field["type"] == "boolean" {
                    &["value", "default"]
                } else {
                    &["value", "min", "max", "step"]
                };
                for key in keys {
                    assert!(field.get(key).is_some(), "{locale}: {key} in {field}");
                }
                assert!(field.get("applicable").is_some() && field.get("at_default").is_some());
                assert!(
                    field["labels"]["pt-br"].is_string()
                        && field["labels"]["en"].is_string()
                        && field["labels"]["es"].is_string(),
                    "{field}"
                );
            }
        }
    }
    // The languages differ where they should: the fade's name and the amount parameter.
    let label_in = |state: &mut EditorState, locale: &str| {
        let answer = inspect(state, json!({"locale": locale, "clip": "c1"}));
        answer["clips"][0]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["id"] == "amount")
            .unwrap()["label"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(label_in(&mut state, "pt-br"), "Quantidade");
    assert_eq!(label_in(&mut state, "en"), "Amount");
    assert_eq!(label_in(&mut state, "es"), "Cantidad");
}

#[test]
fn the_inspector_says_what_is_at_default_what_is_not_drawn_and_never_writes() {
    let mut state = session();
    edit(
        &mut state,
        json!([add_fade("f1", 0, 40, json!({"amount": 0.25}))]),
    );
    let before = footprint(&state);
    let answer = inspect(&mut state, json!({}));
    assert_eq!(answer["source"], "document");
    let scopes = answer["scopes"].as_array().unwrap();
    let applicable = |kind: &str| {
        scopes.iter().find(|scope| scope["kind"] == kind).unwrap()["applicable"].clone()
    };
    // The output and (F062) a shot are drawn; a node target does not exist yet.
    assert_eq!(applicable("output"), true);
    assert_eq!(applicable("shot"), true);
    assert_eq!(applicable("node"), false, "node is not drawn");
    let node = scopes.iter().find(|scope| scope["kind"] == "node").unwrap();
    assert!(node["reason"].is_string(), "node carries its reason");
    let clip = &answer["clips"][0];
    let amount = clip["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["id"] == "amount")
        .unwrap();
    assert_eq!(amount["value"], 0.25);
    assert_eq!(amount["default"], 1.0);
    assert_eq!(amount["at_default"], false);
    assert_eq!(
        amount["reset"],
        json!({"needed": true, "to": 1.0, "op": {"op":"set_param","clip":"f1","param":"amount","value":1.0}})
    );
    assert_eq!(footprint(&state), before, "inspecting writes nothing");
    // The ops a script sends to reset the look are in the answer; sending them resets it.
    assert_eq!(
        clip["reset_ops"],
        json!([{"op":"set_param","clip":"f1","param":"amount","value":1.0}])
    );
    edit(&mut state, clip["reset_ops"].clone());
    assert_eq!(
        inspect(&mut state, json!({}))["clips"][0]["reset_ops"],
        json!([])
    );

    let unknown = err(
        &mut state,
        json!({"op":"scene.fx.inspect","scene":"street","clip":"ghost"}),
    );
    assert_eq!(unknown.code, "unknown_element");
    let bad = err(
        &mut state,
        json!({"op":"scene.fx.inspect","scene":"street","locale":"fr"}),
    );
    assert!(
        bad.code.contains("invalid") || bad.code.contains("unknown"),
        "{bad:?}"
    );
}

#[test]
fn a_probe_judges_a_value_through_the_core_without_writing_it() {
    let mut state = session();
    edit(&mut state, json!([add_fade("f1", 0, 40, json!({}))]));
    let probe = |state: &mut EditorState, field: &str, value: f64, quantize: bool| {
        inspect(
            state,
            json!({"probe": {"clip":"f1","field":field,"value":value,"quantize":quantize}}),
        )["probe"]
            .clone()
    };
    let before = footprint(&state);
    let legal = probe(&mut state, "amount", 0.4, false);
    assert_eq!(legal["ok"], true, "{legal}");
    assert_eq!(legal["would_change"], true);
    // Writing a default the clip leaves out is a change of the document; writing what it holds is not.
    let explicit_default = probe(&mut state, "amount", 1.0, false);
    assert_eq!(explicit_default["would_change"], true);
    assert_eq!(footprint(&state), before, "the probes so far wrote nothing");
    edit(&mut state, json!([set("f1", "amount", json!(0.4))]));
    let same = probe(&mut state, "amount", 0.4, false);
    assert_eq!(same["ok"], true);
    assert_eq!(same["would_change"], false);
    let before = footprint(&state);
    let out_of_range = probe(&mut state, "amount", 4.0, false);
    assert_eq!(out_of_range["ok"], false);
    assert_eq!(out_of_range["error"]["code"], "param_out_of_range");
    let fraction = probe(&mut state, "start_frame", 1.5, false);
    assert_eq!(fraction["ok"], false, "a fractional frame is refused");
    let nothing = probe(&mut state, "nope", 1.0, false);
    assert_eq!(nothing["error"]["code"], "unknown_field");
    // Quantization is the core's: 0.4137 lands on the 0.01 step.
    let snapped = probe(&mut state, "amount", 0.4137, true);
    assert_eq!(snapped["value"], 0.41);
    assert_eq!(footprint(&state), before, "a probe is a read");
}

#[test]
fn the_inspector_follows_the_gesture_candidate_and_its_last_refusal() {
    let mut state = session();
    edit(&mut state, json!([add_fade("f1", 0, 40, json!({}))]));
    let before = footprint(&state);
    let gesture = begin(&mut state, "f1");
    update(
        &mut state,
        &gesture,
        json!([set("f1", "amount", json!(0.6))]),
    )
    .unwrap();
    assert!(
        update(
            &mut state,
            &gesture,
            json!([set("f1", "amount", json!(7.0))])
        )
        .is_err()
    );

    let shown = inspect(&mut state, json!({"gesture": gesture}));
    assert_eq!(shown["source"], "gesture");
    assert_eq!(shown["gesture"]["clip"], "f1");
    assert_eq!(shown["gesture"]["refusal"]["code"], "param_out_of_range");
    let amount = shown["clips"][0]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["id"] == "amount")
        .unwrap()
        .clone();
    assert_eq!(amount["value"], 0.6, "the last legal candidate");
    assert_eq!(inspect(&mut state, json!({}))["source"], "document");
    assert_eq!(footprint(&state), before);

    ok(
        &mut state,
        json!({"op":"scene.fx.gesture.cancel","gesture":gesture}),
    );
    let error = err(
        &mut state,
        json!({"op":"scene.fx.inspect","scene":"street","gesture":gesture}),
    );
    assert!(
        !error.code.is_empty(),
        "a closed gesture is no longer readable"
    );
}

#[test]
fn the_real_headless_process_inspects_and_edits_a_fade_like_the_library() {
    let requests = [
        json!({"op":"scene.attach","id":"street","bundle":bundle()}).to_string(),
        json!({"op":"scene.fx.edit","scene":"street","ops":[add_fade("f1", 0, 40, json!({"amount":0.5}))]}).to_string(),
        json!({"op":"scene.fx.inspect","scene":"street","locale":"en"}).to_string(),
        json!({"op":"scene.fx.inspect","scene":"street","clip":"f1",
               "probe":{"clip":"f1","field":"amount","value":3.0}}).to_string(),
        json!({"op":"scene.fx.frame","scene":"street","frame":10}).to_string(),
        json!({"op":"history.undo"}).to_string(),
        json!({"op":"scene.fx.inspect","scene":"street"}).to_string(),
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
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
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
    assert_eq!(answers[2]["result"]["locale"], "en");
    assert_eq!(answers[2]["result"]["clips"][0]["effect"], "fade");
    assert_eq!(answers[3]["result"]["probe"]["ok"], false);
    assert_eq!(
        answers[3]["result"]["probe"]["error"]["code"],
        "param_out_of_range"
    );
    assert_eq!(answers[4]["result"]["passes"][0]["effect"], "fade");
    assert_eq!(
        answers[6]["result"]["clip_count"], 0,
        "undo removed the clip"
    );

    // A fade the catalog does not have is refused by the same process with a typed code.
    let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
        .arg("--empty")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let refused = [
        json!({"op":"scene.attach","id":"street","bundle":bundle()}).to_string(),
        json!({"op":"scene.fx.edit","scene":"street","ops":[
            {"op":"add_clip","effect":"dissolve","start_frame":0,"duration_frames":8}]})
        .to_string(),
    ]
    .join("\n");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(refused.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let answers = lines(&output.stdout);
    assert_eq!(answers[1]["ok"], false);
}

#[test]
fn a_fade_param_reads_back_as_the_typed_value() {
    let mut state = session();
    edit(
        &mut state,
        json!([add_fade("f1", 0, 40, json!({"amount": 0.3}))]),
    );
    let scene = scene_of(&state);
    let pass = &scene.bundle().fx_passes(10)[0];
    assert_eq!(pass.param("amount"), Some(FxValue::Number(0.3)));
    assert_eq!(
        pass.param("color"),
        Some(FxValue::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0
        })
    );
}

#[test]
fn a_filtered_inspector_probes_the_complete_layer_and_reports_the_quantized_value() {
    let mut state = session();
    edit(
        &mut state,
        json!([
            add_fade("f1", 0, 40, json!({})),
            add_vignette("v1", 0, 40, json!({}))
        ]),
    );
    edit(
        &mut state,
        json!([{"op":"set_field","clip":"v1","field":"fade_out_frames","value":10}]),
    );
    let before = footprint(&state);
    let answer = inspect(
        &mut state,
        json!({"clip":"f1", "probe":{"clip":"v1","field":"intensity","value":0.4}}),
    );
    assert_eq!(answer["clip_count"], 1);
    assert_eq!(answer["clips"][0]["clip"], "f1");
    assert_eq!(answer["probe"]["ok"], true, "{answer}");
    let answer = inspect(
        &mut state,
        json!({"clip":"f1", "probe":{"clip":"v1","field":"fade_in_frames","value":35.4,"quantize":true}}),
    );
    assert_eq!(answer["probe"]["ok"], false);
    assert_eq!(answer["probe"]["value"], 35.0);
    assert_eq!(answer["probe"]["requested_value"], 35.4);
    assert_eq!(answer["probe"]["error"]["code"], "invalid_bundle");
    assert_eq!(footprint(&state), before, "both probes are read-only");
}

#[test]
fn field_reset_ops_are_executable_in_the_real_jsonl_process_including_components_and_time() {
    let attach = json!({"op":"scene.attach","id":"street","bundle":bundle()});
    let add = json!({"op":"scene.fx.edit","scene":"street","ops":[add_fade("f1",0,40,json!({"color":{"r":0.2,"g":0.7,"b":0.8}}))]});
    for (field, expected_op) in [
        (
            "color_g",
            json!({"op":"set_param","clip":"f1","param":"color","value":{"r":0.2,"g":0.0,"b":0.8}}),
        ),
        (
            "duration_frames",
            json!({"op":"set_field","clip":"f1","field":"duration_frames","value":24}),
        ),
    ] {
        let mut state = session();
        ok(&mut state, add.clone());
        let before = footprint(&state);
        let info = inspect(&mut state, json!({"clip":"f1"}));
        let described = info["clips"][0]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"] == field)
            .unwrap();
        let op = described["reset"]["op"].clone();
        assert_eq!(op, expected_op);
        assert_eq!(footprint(&state), before);
        edit(&mut state, json!([op.clone()]));
        assert_eq!(undo_depth(&state), before.1 + 1);
        if field == "color_g" {
            assert_eq!(
                clip_of(&list(&mut state), "f1")["params"]["color"],
                json!({"r":0.2,"g":0.0,"b":0.8})
            );
            assert_eq!(clip_of(&list(&mut state), "f1")["duration_frames"], 40);
        }
        let requests = [
            attach.clone(),
            add.clone(),
            json!({"op":"scene.fx.inspect","scene":"street","clip":"f1"}),
            json!({"op":"scene.fx.edit","scene":"street","ops":[op]}),
            json!({"op":"scene.fx.inspect","scene":"street","clip":"f1"}),
            json!({"op":"history.undo"}),
            json!({"op":"scene.fx.inspect","scene":"street","clip":"f1"}),
        ];
        let input = requests
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let mut library_state = EditorState::empty(&ProjectSettings::default()).unwrap();
        let mut library = Vec::new();
        headless::run_session(&mut library_state, None, input.as_bytes(), &mut library).unwrap();
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
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let parse = |bytes: &[u8]| {
            String::from_utf8_lossy(bytes)
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>()
        };
        let results = parse(&output.stdout);
        assert_eq!(results, parse(&library));
        let value_of = |index: usize| {
            results[index]["result"]["clips"][0]["fields"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["id"] == field)
                .unwrap()["value"]
                .clone()
        };
        assert_eq!(
            value_of(4),
            if field == "color_g" {
                json!(0.0)
            } else {
                json!(24.0)
            }
        );
        assert_eq!(value_of(6), value_of(2), "undo restores the field");
    }
}
