//! BL081 (F017): cutting one timeline clip in two at an absolute frame, through the public
//! library (`EditorState`, `headless::execute`) and the JSONL process (`tessel-headless`), with
//! no window, no selection and no renderer.
//!
//! The strongest check is the same in every case: the plan (and the mix) of the sequence
//! before the cut and after it are the same picture and the same sound, frame by frame, apart
//! from the identity of the clip that produced them.
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command as Process, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::audio::{AudioBackend, OfflineMixer, Pcm};
use tessel_editor_core::fixtures::{proof_requests, write_standard_set};
use tessel_editor_core::state::split::{SplitAvailability, SplitBlock};
use tessel_editor_core::time::Ratio;
use tessel_editor_core::{CommandError, EditorState, headless, persist};

const LENGTH: i64 = 96;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-split-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn fail(state: &mut EditorState, request: Value) -> CommandError {
    headless::execute(state, &request).expect_err(&format!("{request} must fail"))
}

fn split(state: &mut EditorState, clip: &str, frame: i64) -> Value {
    run(state, json!({"op":"clip.split","clip":clip,"frame":frame}))
}

/// The proof project (still, frame-sequence video, tone, splat, scene, title) at `rate`, plus a
/// declared video `cv` that starts mid-asset (`source_start` 1/2 s) and plays at 150 %.
fn project(tag: &str, rate: Value) -> EditorState {
    let dir = scratch(tag);
    let set = write_standard_set(&dir).unwrap();
    let mut requests = proof_requests(&set);
    requests[0]["rate"] = rate;
    requests[0]["length"] = json!(LENGTH);
    let mut state = EditorState::demo();
    for request in requests {
        run(&mut state, request);
    }
    let fake = dir.join("fake.mp4");
    fs::write(&fake, b"not decoded by the core").unwrap();
    run(
        &mut state,
        json!({"op":"asset.add","kind":"video","id":"fakev","name":"Declared","path":fake,
            "width":64,"height":36,"rate":24,"duration":10}),
    );
    run(
        &mut state,
        json!({"op":"track.add","id":"vt","name":"Declared","kind":"video"}),
    );
    run(
        &mut state,
        json!({"op":"clip.add","id":"cv","track":"vt","asset":"fakev","start":12,"duration":48,
            "source_start":{"num":1,"den":2},"properties":{"speed":150}}),
    );
    state
}

fn project_24(tag: &str) -> EditorState {
    project(tag, json!(24))
}

fn ntsc(tag: &str) -> EditorState {
    project(tag, json!({"num":30000,"den":1001}))
}

/// The same picture: layers equal but for which clip made them (`id`, `clip`) and the wording of
/// warnings; numbers within 1e-9 (the orbit and the zoom are floats).
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(left), Value::Object(right)) => {
            let keys = |map: &serde_json::Map<String, Value>| {
                map.keys()
                    .filter(|key| !matches!(key.as_str(), "id" | "clip" | "message"))
                    .cloned()
                    .collect::<Vec<_>>()
            };
            keys(left) == keys(right)
                && keys(left)
                    .iter()
                    .all(|key| same(&left[key.as_str()], &right[key.as_str()]))
        }
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len() && left.iter().zip(right).all(|(l, r)| same(l, r))
        }
        (Value::Number(left), Value::Number(right)) => {
            (left.as_f64().unwrap() - right.as_f64().unwrap()).abs() <= 1e-9
        }
        _ => a == b,
    }
}

fn plans(state: &mut EditorState) -> Vec<Value> {
    (0..LENGTH)
        .map(|frame| run(state, json!({"op":"plan","frame":frame})))
        .collect()
}

fn assert_same_picture(before: &[Value], after: &[Value]) {
    assert_eq!(before.len(), after.len());
    for (frame, (a, b)) in before.iter().zip(after).enumerate() {
        assert!(same(a, b), "frame {frame} changed:\n{a}\n{b}");
    }
}

/// Everything a refused command must leave alone.
fn fingerprint(state: &EditorState) -> Value {
    json!({
        "document": state.to_document(None),
        "history": state.history.to_json(),
        "revision": state.project.revision,
        "selected": state.timeline.selected_clip.map(|id| id.to_string()),
        "selected_track": state.timeline.selected_track.map(|id| id.to_string()),
        "playhead": state.timeline.playhead,
        "clips": state.timeline.clips.iter().map(|clip| clip.id.to_string()).collect::<Vec<_>>(),
    })
}

fn span(state: &EditorState, id: &str) -> (i64, i64) {
    let clip = state
        .timeline
        .clip(id)
        .unwrap_or_else(|| panic!("no clip {id}"));
    (clip.start, clip.end())
}

fn overrides_of(state: &EditorState, clip: &str) -> Vec<(&'static str, Value)> {
    state
        .inspector
        .overrides_sorted()
        .into_iter()
        .filter(|(owner, _, _)| owner == clip)
        .map(|(_, property, value)| (property, value.to_json()))
        .collect()
}

fn number(state: &EditorState, clip: &str, property: &str) -> f64 {
    let property = tessel_editor_core::state::inspector::property_def(property).unwrap();
    match state.inspector.value(clip, property) {
        tessel_editor_core::state::inspector::Value::Number(number) => number,
        other => panic!("{other:?} is not a number"),
    }
}

fn mix(state: &EditorState) -> Pcm {
    let plan = state.plan_audio(0, LENGTH).unwrap();
    OfflineMixer::default().mix(&plan).unwrap()
}

fn assert_same_sound(before: &Pcm, after: &Pcm) {
    assert_eq!(before.samples.len(), after.samples.len());
    let worst = before
        .samples
        .iter()
        .zip(&after.samples)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(worst < 1e-6, "the mix moved by {worst}");
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

// ---- the cut itself -----------------------------------------------------------------------

#[test]
fn a_cut_in_the_middle_covers_the_clip_exactly_and_keeps_the_picture() {
    let mut state = project_24("middle");
    let before = plans(&mut state);
    let original = state.timeline.clip("cv").unwrap().clone();

    let outcome = split(&mut state, "cv", 30);
    assert_eq!(outcome["action"], "split");
    assert_eq!(outcome["new_id"], "cv-split-1");
    assert_eq!(outcome["left"]["end"], 30);
    assert_eq!(outcome["right"]["start"], 30);
    assert_eq!(outcome["selected"], "clip:cv-split-1");

    let left = state.timeline.clip("cv").unwrap().clone();
    let right = state.timeline.clip("cv-split-1").unwrap().clone();
    // Two positive intervals that cover the original with no gap and no overlap.
    assert_eq!(span(&state, "cv"), (12, 30));
    assert_eq!(span(&state, "cv-split-1"), (30, 60));
    assert_eq!(left.duration + right.duration, original.duration);
    // Same track, asset, kind and name; the right half sits right after the left in the order.
    assert_eq!(
        (left.track, left.media, left.kind),
        (original.track, original.media, original.kind)
    );
    assert_eq!(
        (right.track, right.media, right.kind),
        (original.track, original.media, original.kind)
    );
    assert_eq!(left.name, original.name);
    assert_eq!(right.name, original.name);
    let order: Vec<&str> = state
        .timeline
        .clips
        .iter()
        .map(|clip| clip.id.as_str())
        .collect();
    let at = order.iter().position(|id| *id == "cv").unwrap();
    assert_eq!(order[at + 1], "cv-split-1");
    // The left half keeps the head of the source untouched.
    assert_eq!(left.source_start, original.source_start);
    // 18 frames at 24 fps and 150 % = 1.125 s of source after the 1/2 s it already skipped.
    assert_eq!(
        right.source_start,
        Ratio::new(1, 2)
            .unwrap()
            .add(Ratio::new(18 * 3, 24 * 2).unwrap())
            .unwrap()
    );
    assert_eq!(right.source_start, Ratio::new(13, 8).unwrap());
    // Per-clip properties travel with each half.
    assert_eq!(
        overrides_of(&state, "cv"),
        overrides_of(&state, "cv-split-1")
    );
    assert_eq!(overrides_of(&state, "cv")[0].0, "speed");

    assert_same_picture(&before, &plans(&mut state));
}

#[test]
fn halves_of_one_frame_are_valid_at_either_end() {
    for (tag, frame, halves) in [("head", 13, (1, 47)), ("tail", 59, (47, 1))] {
        let mut state = project_24(tag);
        let before = plans(&mut state);
        let right = split(&mut state, "cv", frame)["new_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            (
                state.timeline.clip("cv").unwrap().duration,
                state.timeline.clip(&right).unwrap().duration
            ),
            halves
        );
        assert_same_picture(&before, &plans(&mut state));
    }
}

#[test]
fn a_cut_at_a_fractional_rate_keeps_the_source_time_exact() {
    let mut state = ntsc("ntsc");
    let before = plans(&mut state);
    split(&mut state, "cv", 31);
    let right = state.timeline.clip("cv-split-1").unwrap().clone();
    // 19 frames of 1001/30000 s at 150 %, on top of the 1/2 s already skipped: exact, no float.
    let shift = Ratio::new(19 * 1001 * 3, 30_000 * 2).unwrap();
    assert_eq!(
        right.source_start,
        Ratio::new(1, 2).unwrap().add(shift).unwrap()
    );
    assert_same_picture(&before, &plans(&mut state));
    // The sequence is still described by the same rate.
    assert_eq!(state.timeline.rate.num(), 30_000);
}

#[test]
fn a_title_keeps_its_text_and_every_property_on_both_halves() {
    let mut state = project_24("title");
    run(
        &mut state,
        json!({"op":"clip.property","clip":"c-title","property":"opacity","value":70}),
    );
    run(
        &mut state,
        json!({"op":"clip.property","clip":"c-title","property":"blend","value":"multiply"}),
    );
    let before = plans(&mut state);
    let text = state.timeline.clip("c-title").unwrap().text.clone();
    assert!(text.is_some());
    split(&mut state, "c-title", 20);
    assert_eq!(state.timeline.clip("c-title").unwrap().text, text);
    assert_eq!(state.timeline.clip("c-title-split-1").unwrap().text, text);
    let kept = overrides_of(&state, "c-title");
    assert!(kept.iter().any(|(property, _)| *property == "opacity"));
    assert_eq!(kept, overrides_of(&state, "c-title-split-1"));
    assert_same_picture(&before, &plans(&mut state));
}

#[test]
fn an_orbiting_camera_continues_where_the_orbit_already_is() {
    let mut state = project_24("orbit");
    let before = plans(&mut state);
    split(&mut state, "c-scene", 17);
    let left = state.timeline.clip("c-scene").unwrap().camera.unwrap();
    let right = state
        .timeline
        .clip("c-scene-split-1")
        .unwrap()
        .camera
        .unwrap();
    assert_eq!(left.orbit, right.orbit);
    assert_ne!(
        left.position, right.position,
        "the orbit has moved by the cut"
    );
    assert_same_picture(&before, &plans(&mut state));
}

#[test]
fn camera_keys_stay_anchored_in_scene_time_on_both_halves() {
    let mut state = project_24("keys");
    run(
        &mut state,
        json!({"op":"clip.camera","clip":"c-scene","position":[0,1.2,5],"target":[0,0,0],"fov":50,"orbit":0}),
    );
    for (time, z) in [(0, 10.0), (2, 4.0)] {
        run(
            &mut state,
            json!({"op":"clip.camera_key","clip":"c-scene","time":time,
                "position":[0,1.0,z],"target":[0,0,0],"fov":50,"easing":"linear"}),
        );
    }
    let before = plans(&mut state);
    split(&mut state, "c-scene", 13);
    let keys = |state: &EditorState, id: &str| state.timeline.clip(id).unwrap().camera_keys.clone();
    assert_eq!(keys(&state, "c-scene").len(), 2);
    assert_eq!(keys(&state, "c-scene"), keys(&state, "c-scene-split-1"));
    assert_same_picture(&before, &plans(&mut state));
}

#[test]
fn a_still_keeps_its_ken_burns_drift_and_without_it_nothing_is_rewritten() {
    let mut state = project_24("kenburns");
    // A new still has Ken Burns off; turning it on is the animation the cut must continue.
    run(
        &mut state,
        json!({"op":"clip.property","clip":"c-img","property":"ken_burns","value":true}),
    );
    let before = plans(&mut state);
    let outcome = split(&mut state, "c-img", 10);
    assert_eq!(outcome["adjusted"].as_array().unwrap().len(), 2);
    // 50 % -> 106 % over 23 steps: the halves meet at the zoom the whole clip had there.
    let (start, end) = (50.0, 106.0);
    let at_cut = start + (end - start) * 10.0 / 23.0;
    assert!((number(&state, "c-img-split-1", "zoom") - at_cut).abs() < 1e-12);
    assert!(
        (number(&state, "c-img", "end_zoom") - (start + (end - start) * 9.0 / 23.0)).abs() < 1e-12
    );
    assert_eq!(number(&state, "c-img-split-1", "end_zoom"), 106.0);
    assert_same_picture(&before, &plans(&mut state));

    // The default still (Ken Burns off) has no drift to continue: nothing is rewritten.
    let mut still = project_24("nokenburns");
    let outcome = split(&mut still, "c-img", 10);
    assert_eq!(outcome["adjusted"], json!([]));
    assert_eq!(number(&still, "c-img", "zoom"), 50.0);
    assert_eq!(number(&still, "c-img-split-1", "zoom"), 50.0);
}

// ---- fades --------------------------------------------------------------------------------

#[test]
fn visual_fades_keep_their_outer_edges_and_the_inner_edges_are_plain_cuts() {
    let mut state = project_24("fades");
    for (property, value) in [("fade_in", 6), ("fade_out", 12)] {
        run(
            &mut state,
            json!({"op":"clip.property","clip":"c-title","property":property,"value":value}),
        );
    }
    let before = plans(&mut state);
    let outcome = split(&mut state, "c-title", 25);
    assert_eq!(number(&state, "c-title", "fade_in"), 6.0);
    assert_eq!(number(&state, "c-title", "fade_out"), 0.0);
    assert_eq!(number(&state, "c-title-split-1", "fade_in"), 0.0);
    assert_eq!(number(&state, "c-title-split-1", "fade_out"), 12.0);
    assert_eq!(outcome["adjusted"].as_array().unwrap().len(), 2);
    assert_same_picture(&before, &plans(&mut state));
}

#[test]
fn a_cut_inside_a_visual_fade_is_refused_and_leaves_everything_as_it_was() {
    let mut state = project_24("fade-refused");
    for (property, value) in [("fade_in", 12), ("fade_out", 24)] {
        run(
            &mut state,
            json!({"op":"clip.property","clip":"c-title","property":property,"value":value}),
        );
    }
    let untouched = fingerprint(&state);
    // Frame 11 is the last still-ramping frame of the fade-in; frame 26 leaves 22 frames, but the
    // fade-out has 24: the last frame of the left half would still be fading.
    for (frame, reason) in [(11, "fade_in_crosses_cut"), (26, "fade_out_crosses_cut")] {
        let error = fail(
            &mut state,
            json!({"op":"clip.split","clip":"c-title","frame":frame}),
        );
        assert_eq!(error.code, "split_unsupported", "frame {frame}");
        assert_eq!(error.details.as_ref().unwrap()["reason"], reason);
        assert_eq!(
            fingerprint(&state),
            untouched,
            "frame {frame} changed state"
        );
    }
    // The boundary frames on the good side are accepted.
    split(&mut state, "c-title", 12);
    split(&mut state, "c-title-split-1", 25);
}

#[test]
fn loaded_fractional_visual_fades_use_the_renderers_exact_boundaries() {
    for (property, fade, refused, accepted) in [
        ("fade_in", 1.4, Some(1), 2),
        ("fade_out", 2.4, Some(47), 46),
        ("fade_in", 0.4, None, 1),
    ] {
        let mut doc = project_24(&format!("fractional-{property}-{fade}")).to_document(None);
        doc["clip_properties"]["c-title"]["fade_in"] = json!(0.0);
        doc["clip_properties"]["c-title"]["fade_out"] = json!(0.0);
        doc["clip_properties"]["c-title"][property] = json!(fade);
        let mut state = EditorState::from_document(&doc, None).unwrap();
        assert_eq!(number(&state, "c-title", property), fade);

        if let Some(frame) = refused {
            run(
                &mut state,
                json!({"op":"invoke","target":"clip:c-title","action":"select"}),
            );
            run(
                &mut state,
                json!({"op":"set","target":"timeline.playhead","field":"frame","value":frame}),
            );
            assert!(matches!(
                state.split_availability(),
                SplitAvailability::Blocked(_)
            ));
            let untouched = fingerprint(&state);
            let error = fail(
                &mut state,
                json!({"op":"clip.split","clip":"c-title","frame":frame}),
            );
            assert_eq!(error.code, "split_unsupported");
            assert_eq!(
                error.details.as_ref().unwrap()["reason"],
                format!("{property}_crosses_cut")
            );
            assert_eq!(fingerprint(&state), untouched);
        }

        let before = plans(&mut state);
        split(&mut state, "c-title", accepted);
        let inner = if property == "fade_in" {
            "c-title-split-1"
        } else {
            "c-title"
        };
        assert_eq!(number(&state, inner, property), 0.0);
        assert_same_picture(&before, &plans(&mut state));
    }

    // Sound keeps the mixer's rounded duration: a loaded 1.4-frame ramp ends at frame 1.
    let mut doc = project_24("fractional-audio-control").to_document(None);
    doc["clip_properties"]["c-tone"]["fade_in"] = json!(1.4);
    doc["clip_properties"]["c-tone"]["fade_out"] = json!(0.0);
    let mut state = EditorState::from_document(&doc, None).unwrap();
    let before = mix(&state);
    split(&mut state, "c-tone", 1);
    assert_eq!(before.samples, mix(&state).samples);
}

#[test]
fn audio_fades_cut_in_the_plateau_mix_the_same_sound() {
    let mut state = project_24("audio-fades");
    for (property, value) in [("fade_in", 12), ("fade_out", 24)] {
        run(
            &mut state,
            json!({"op":"clip.property","clip":"c-tone","property":property,"value":value}),
        );
    }
    let before = mix(&state);
    assert!(before.samples.iter().any(|sample| sample.abs() > 0.01));
    split(&mut state, "c-tone", 24);
    assert_eq!(number(&state, "c-tone", "fade_in"), 12.0);
    assert_eq!(number(&state, "c-tone", "fade_out"), 0.0);
    assert_eq!(number(&state, "c-tone-split-1", "fade_in"), 0.0);
    assert_eq!(number(&state, "c-tone-split-1", "fade_out"), 24.0);
    assert_same_sound(&before, &mix(&state));
}

#[test]
fn an_audio_cut_with_a_fade_across_it_is_refused() {
    let mut state = project_24("audio-refused");
    for (property, value) in [("fade_in", 12), ("fade_out", 24)] {
        run(
            &mut state,
            json!({"op":"clip.property","clip":"c-tone","property":property,"value":value}),
        );
    }
    let untouched = fingerprint(&state);
    // Sound fades by sample: with a 24-frame fade-out only a cut 24 frames before the end (frame
    // 24) leaves the left half entirely at full gain; frame 25 is already inside the ramp.
    for frame in [11, 25] {
        let error = fail(
            &mut state,
            json!({"op":"clip.split","clip":"c-tone","frame":frame}),
        );
        assert_eq!(error.code, "split_unsupported", "frame {frame}");
        assert_eq!(fingerprint(&state), untouched);
    }
    // Fades that already overlap are shortened by the mixer in proportion: not splittable.
    run(
        &mut state,
        json!({"op":"clip.property","clip":"c-tone","property":"fade_in","value":30}),
    );
    run(
        &mut state,
        json!({"op":"clip.property","clip":"c-tone","property":"fade_out","value":30}),
    );
    let error = fail(
        &mut state,
        json!({"op":"clip.split","clip":"c-tone","frame":30}),
    );
    assert_eq!(
        error.details.as_ref().unwrap()["reason"],
        "fades_longer_than_clip"
    );
    run(
        &mut state,
        json!({"op":"invoke","target":"clip:c-tone","action":"select"}),
    );
    for frame in [1, 24, 47] {
        run(
            &mut state,
            json!({"op":"set","target":"timeline.playhead","field":"frame","value":frame}),
        );
        assert_eq!(
            state.split_availability(),
            SplitAvailability::Blocked(SplitBlock::FadesLongerThanClip)
        );
        let error = fail(
            &mut state,
            json!({"op":"invoke","target":"timeline.split","action":"invoke"}),
        );
        assert_eq!(error.message, SplitBlock::FadesLongerThanClip.reason());
        assert!(!error.message.contains("move the playhead"));
    }
}

#[test]
fn audio_without_fades_mixes_the_same_sound_at_every_cut() {
    for frame in [1, 7, 24, 47] {
        let mut state = project_24(&format!("audio-{frame}"));
        let before = mix(&state);
        split(&mut state, "c-tone", frame);
        assert_same_sound(&before, &mix(&state));
    }
}

// ---- IDs and selection --------------------------------------------------------------------

#[test]
fn the_default_id_is_deterministic_and_an_explicit_one_is_used_as_given() {
    let mut state = project_24("ids");
    assert_eq!(
        split(&mut state, "c-title", 30)["new_id"],
        "c-title-split-1"
    );
    // The next free number, never a reused one.
    assert_eq!(
        split(&mut state, "c-title", 15)["new_id"],
        "c-title-split-2"
    );
    assert_eq!(
        split(&mut state, "c-title-split-1", 40)["new_id"],
        "c-title-split-1-split-1"
    );
    let outcome = run(
        &mut state,
        json!({"op":"clip.split","clip":"c-img","frame":12,"new_id":"second-half"}),
    );
    assert_eq!(outcome["new_id"], "second-half");
    assert_eq!(span(&state, "second-half"), (12, 24));
    assert_eq!(state.timeline.selected_clip.unwrap(), "second-half");
}

#[test]
fn a_cut_does_not_depend_on_the_selection_and_then_selects_the_right_half() {
    let mut state = project_24("selection");
    state.timeline.selected_clip = None;
    split(&mut state, "c-title", 10);
    assert_eq!(state.timeline.selected_clip.unwrap(), "c-title-split-1");
    run(
        &mut state,
        json!({"op":"invoke","target":"clip:c-img","action":"select"}),
    );
    split(&mut state, "c-title", 5);
    assert_eq!(
        state.timeline.selected_clip.unwrap(),
        "c-title-split-2",
        "selecting another clip first changes nothing about which clip is cut"
    );
}

#[test]
fn the_control_cuts_the_selected_clip_at_the_playhead_through_the_same_command() {
    let mut state = project_24("control");
    state.timeline.selected_clip = None;
    // Nothing selected, then a selection with the playhead outside it: refused with the reason.
    let error = fail(
        &mut state,
        json!({"op":"invoke","target":"timeline.split","action":"invoke"}),
    );
    assert_eq!(error.code, "disabled_element");
    assert_eq!(error.message, SplitBlock::NoSelection.reason());
    run(
        &mut state,
        json!({"op":"invoke","target":"clip:c-title","action":"select"}),
    );
    run(
        &mut state,
        json!({"op":"set","target":"timeline.playhead","field":"frame","value":0}),
    );
    assert_eq!(
        state.split_availability(),
        SplitAvailability::Blocked(SplitBlock::PlayheadOutside),
        "the first frame of a clip is an edge"
    );
    let untouched = fingerprint(&state);
    let error = fail(
        &mut state,
        json!({"op":"invoke","target":"timeline.split","action":"invoke"}),
    );
    assert_eq!(error.code, "disabled_element");
    assert_eq!(fingerprint(&state), untouched);

    run(
        &mut state,
        json!({"op":"set","target":"timeline.playhead","field":"frame","value":20}),
    );
    assert!(matches!(
        state.split_availability(),
        SplitAvailability::Ready { frame: 20, .. }
    ));
    let depth = undo_depth(&state);
    let outcome = run(
        &mut state,
        json!({"op":"invoke","target":"timeline.split","action":"invoke"}),
    );
    assert_eq!(outcome["action"], "split");
    assert_eq!(span(&state, "c-title"), (0, 20));
    assert_eq!(span(&state, "c-title-split-1"), (20, 48));
    assert_eq!(undo_depth(&state), depth + 1, "one entry for the click");
}

// ---- atomic refusals ----------------------------------------------------------------------

#[test]
fn every_refusal_is_typed_and_changes_nothing() {
    let mut state = project_24("refusals");
    // A clip of another sequence, to prove IDs are unique across sequences.
    run(
        &mut state,
        json!({"op":"sequence.add","id":"alt","activate":true}),
    );
    run(
        &mut state,
        json!({"op":"track.add","id":"alt-track","name":"Alt","kind":"video"}),
    );
    run(
        &mut state,
        json!({"op":"clip.add","id":"alt-clip","track":"alt-track","asset":"img","start":0,"duration":10}),
    );
    run(
        &mut state,
        json!({"op":"sequence.activate","sequence":"main"}),
    );
    let untouched = fingerprint(&state);
    let long = "x".repeat(65);
    let cases = [
        (json!({"clip":"nope","frame":10}), "unknown_element"),
        (json!({"clip":"c-title","frame":-5}), "out_of_bounds"),
        (json!({"clip":"c-title","frame":49}), "out_of_bounds"),
        (json!({"clip":"c-title","frame":i64::MAX}), "out_of_bounds"),
        (json!({"clip":"c-title","frame":i64::MIN}), "out_of_bounds"),
        (json!({"clip":"c-title","frame":0}), "invalid_duration"),
        (json!({"clip":"c-title","frame":48}), "invalid_duration"),
        (
            json!({"clip":"c-title","frame":10,"new_id":""}),
            "invalid_id",
        ),
        (
            json!({"clip":"c-title","frame":10,"new_id":"a b"}),
            "invalid_id",
        ),
        (
            json!({"clip":"c-title","frame":10,"new_id":"a:b"}),
            "invalid_id",
        ),
        (
            json!({"clip":"c-title","frame":10,"new_id":long}),
            "invalid_id",
        ),
        (
            json!({"clip":"c-title","frame":10,"new_id":"c-title"}),
            "duplicate_id",
        ),
        (
            json!({"clip":"c-title","frame":10,"new_id":"c-img"}),
            "duplicate_id",
        ),
        (
            json!({"clip":"c-title","frame":10,"new_id":"title-track"}),
            "duplicate_id",
        ),
        (
            json!({"clip":"c-title","frame":10,"new_id":"alt-clip"}),
            "duplicate_id",
        ),
        (json!({"clip":"c-title","frame":12.5}), "invalid_request"),
        (json!({"clip":"c-title","frame":12.0}), "invalid_request"),
        (json!({"clip":"c-title","frame":"12"}), "invalid_request"),
        (json!({"clip":"c-title"}), "invalid_request"),
        (json!({"frame":12}), "invalid_request"),
        (
            json!({"clip":"c-title","frame":10,"new_id":7}),
            "invalid_request",
        ),
    ];
    for (mut request, code) in cases {
        request["op"] = json!("clip.split");
        let error = fail(&mut state, request.clone());
        assert_eq!(error.code, code, "{request}");
        assert_eq!(
            fingerprint(&state),
            untouched,
            "{request} changed the state"
        );
    }
    // The refusals above consumed nothing: the default ID is still the first one.
    assert_eq!(
        split(&mut state, "c-title", 10)["new_id"],
        "c-title-split-1"
    );
}

#[test]
fn arithmetic_that_overflows_is_refused_not_wrapped() {
    let mut state = project_24("overflow");
    // A clip whose end does not fit a frame count.
    {
        let clip = state
            .timeline
            .clips
            .iter_mut()
            .find(|clip| clip.id == "c-title")
            .unwrap();
        clip.start = i64::MAX - 5;
        clip.duration = 10;
    }
    let error = fail(
        &mut state,
        json!({"op":"clip.split","clip":"c-title","frame":i64::MAX - 2}),
    );
    assert_eq!(error.code, "invalid_time");

    // A source time that cannot take the elapsed span on top of it.
    let mut state = project_24("overflow-source");
    state
        .timeline
        .clips
        .iter_mut()
        .find(|clip| clip.id == "cv")
        .unwrap()
        .source_start = Ratio::from_int(i64::MAX - 1);
    let before = fingerprint(&state);
    let error = fail(
        &mut state,
        json!({"op":"clip.split","clip":"cv","frame":30}),
    );
    assert_eq!(error.code, "invalid_time");
    assert_eq!(fingerprint(&state), before);
}

// ---- history, save and reopen -------------------------------------------------------------

#[test]
fn a_cut_is_one_undo_entry_and_redo_brings_the_same_cut_back() {
    let mut state = project_24("history");
    run(
        &mut state,
        json!({"op":"clip.property","clip":"c-title","property":"fade_in","value":6}),
    );
    run(
        &mut state,
        json!({"op":"clip.property","clip":"c-title","property":"fade_out","value":6}),
    );
    let before = fingerprint(&state);
    let picture = plans(&mut state);
    let depth = undo_depth(&state);

    split(&mut state, "c-title", 20);
    let after = fingerprint(&state);
    assert_eq!(undo_depth(&state), depth + 1, "one entry, not one per half");
    assert_eq!(state.history.to_json()["undo_label"], "Split clip");
    assert!(after["revision"].as_u64() > before["revision"].as_u64());

    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(state.timeline.clip("c-title-split-1"), None);
    assert_eq!(span(&state, "c-title"), (0, 48));
    assert_eq!(
        number(&state, "c-title", "fade_out"),
        6.0,
        "the ramp is back"
    );
    assert_eq!(
        state.to_document(None),
        before["document"],
        "undo restores the document"
    );
    assert_eq!(state.timeline.selected_clip, None, "and the selection");
    assert_same_picture(&picture, &plans(&mut state));

    run(&mut state, json!({"op":"history.redo"}));
    assert_eq!(state.to_document(None), after["document"]);
    // The selection is session state, not a history entry: redo does not select anything.
    assert_eq!(state.timeline.selected_clip, None);
    assert_same_picture(&picture, &plans(&mut state));
    // A cut after the undo reuses the freed default ID.
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(
        split(&mut state, "c-title", 30)["new_id"],
        "c-title-split-1"
    );
}

#[test]
fn a_saved_cut_reopens_identical() {
    let mut state = project_24("persist");
    run(
        &mut state,
        json!({"op":"clip.property","clip":"c-title","property":"fade_in","value":6}),
    );
    split(&mut state, "c-title", 20);
    split(&mut state, "c-img", 10);
    let picture = plans(&mut state);
    let dir = scratch("persist-file");
    let path = dir.join("cut.tessel.json");
    persist::save(&mut state, &path).unwrap();
    let mut reopened = persist::load(&path).unwrap();
    for id in ["c-title", "c-title-split-1", "c-img", "c-img-split-1"] {
        assert_eq!(
            reopened.timeline.clip(id),
            state.timeline.clip(id),
            "{id} differs after reopening"
        );
        assert_eq!(
            overrides_of(&reopened, id),
            overrides_of(&state, id),
            "{id}"
        );
    }
    assert_same_picture(&picture, &plans(&mut reopened));
    // Zoom values the split derived are stored exactly: they survive the JSON file bit for bit.
    assert_eq!(
        number(&reopened, "c-img-split-1", "zoom"),
        number(&state, "c-img-split-1", "zoom")
    );
}

// ---- the windowless process ---------------------------------------------------------------

#[test]
fn the_headless_process_cuts_undoes_and_reports_refusals() {
    let set = write_standard_set(&scratch("process")).unwrap();
    let mut lines: Vec<Value> = proof_requests(&set);
    lines.extend([
        json!({"op":"plan","frame":30}),
        json!({"op":"clip.split","clip":"c-title","frame":20}),
        // Rejected (the left half now ends at 20): the session goes on.
        json!({"op":"clip.split","clip":"c-title","frame":20}),
        json!({"op":"clip.split","clip":"c-title","frame":12.5}),
        json!({"op":"plan","frame":30}),
        json!({"op":"history.undo"}),
        json!({"op":"plan","frame":30}),
        json!({"op":"history.status"}),
    ]);
    let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for line in &lines {
        writeln!(stdin, "{line}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "a refused cut fails the session"
    );
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), lines.len(), "every line is answered");
    let n = replies.len();
    let (planned, cut, again, fractional) = (
        &replies[n - 8],
        &replies[n - 7],
        &replies[n - 6],
        &replies[n - 5],
    );
    assert_eq!(cut["ok"], true);
    assert_eq!(cut["result"]["new_id"], "c-title-split-1");
    assert_eq!(again["ok"], false);
    assert_eq!(again["error"]["code"], "invalid_duration");
    assert_eq!(fractional["error"]["code"], "invalid_request");
    assert!(
        same(&planned["result"], &replies[n - 4]["result"]),
        "the cut changed no picture"
    );
    assert!(
        same(&planned["result"], &replies[n - 2]["result"]),
        "undo is the original"
    );
    assert_eq!(replies[n - 1]["result"]["history"]["can_redo"], true);
}

#[test]
fn audio_on_a_fractional_sample_grid_is_cut_and_keeps_its_sound() {
    // BL081.5: this cut was refused (`audio_sample_grid`) because the mixer rounded each new
    // segment to a whole source sample. The phase is now carried exactly, so it is accepted;
    // `clip_split_audio_phase.rs` proves the sound sample by sample against an independent oracle.
    let mut state = ntsc("audio-ntsc-speed");
    state
        .execute(tessel_editor_core::Command::MoveClip {
            clip: "c-tone".into(),
            track: None,
            frame: Some(1),
            snap: Some(false),
        })
        .unwrap();
    let before = mix(&state);
    let tone = state.timeline.clip("c-tone").unwrap().id;
    state.timeline.selected_clip = Some(tone);
    state.timeline.playhead = 2;
    assert_eq!(
        state.split_availability(),
        SplitAvailability::Ready {
            clip: tone,
            frame: 2
        }
    );
    split(&mut state, "c-tone", 2);
    assert_eq!(span(&state, "c-tone"), (1, 2));
    assert_eq!(span(&state, "c-tone-split-1"), (2, 49));
    assert_eq!(mix(&state).samples, before.samples, "bit for bit");
}

#[test]
fn a_refused_cut_leaves_an_open_slider_gesture_and_its_history_untouched() {
    use tessel_editor_core::state::property_gesture::GestureCommand;
    use tessel_editor_core::{Command, Raw};
    let mut state = project_24("refusal-live-slider");
    state
        .execute(Command::PropertyGesture(GestureCommand::Begin {
            clip: Some("c-img".into()),
            property: "zoom".into(),
        }))
        .unwrap();
    state
        .execute(Command::PropertyGesture(GestureCommand::Update {
            gesture: None,
            value: Raw::Number(120.0),
        }))
        .unwrap();
    let before = fingerprint(&state);
    let gesture = state.open_property_gesture();
    let edge = state.timeline.clip("c-title").unwrap().start;
    let error = fail(
        &mut state,
        json!({"op":"clip.split","clip":"c-title","frame":edge}),
    );
    assert_eq!(error.code, "invalid_duration");
    assert_eq!(state.open_property_gesture(), gesture);
    assert_eq!(fingerprint(&state), before);
}
