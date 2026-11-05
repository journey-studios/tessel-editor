//! F042 (BL008), lib surface: the domain half of the directed-scene export, through the public
//! API only (no GPUI, no FFmpeg, no browser, no files besides the corpus it reads).
//!
//! Proves: the request shape; the exact rational schedule (same rate, 24→30, 24→24000/1001);
//! that every export frame is the **same complete `ScenePlan`** `scene.plan` answers; range and
//! audio and unsupported-domain refusals that leave the project and its history untouched; that a
//! live gesture never leaks into an export and that a later edit cannot change a frozen one; and
//! that a scene is never a clip of the timeline.
//!
//! Does not prove: pixels, the encoder, jobs, resources (`crates/media/tests/scene_export.rs`,
//! `crates/render/tests/scene_export.rs`).

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::scene_export::{
    AudioChoice, AudioPlan, DirectedAudioMix, SceneExportSnapshot, SceneExportSpec,
};
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_core::time::Ratio;
use tessel_editor_core::{CommandError, EditorState, headless};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/directed-scene-proof/corpus")
}

fn bundle() -> Value {
    serde_json::from_str(&fs::read_to_string(corpus().join("bundle.json")).unwrap()).unwrap()
}

fn run(state: &mut EditorState, request: Value) -> Result<Value, CommandError> {
    headless::execute(state, &request)
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    run(state, request.clone()).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn project_with(bundle: Value) -> EditorState {
    let mut state = EditorState::demo();
    ok(&mut state, json!({"op":"project.create","name":"Export"}));
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle}),
    );
    state
}

fn project() -> EditorState {
    project_with(bundle())
}

fn spec(state: &EditorState, request: Value) -> Result<SceneExportSpec, CommandError> {
    let mut request = request;
    request["op"] = json!("scene.export");
    SceneExportSpec::from_json(state, &request)
}

fn capture(state: &EditorState, request: Value) -> Result<SceneExportSnapshot, CommandError> {
    SceneExportSnapshot::capture(state, &spec(state, request)?)
}

#[test]
fn a_request_has_one_preset_whole_frames_and_an_explicit_audio_choice() {
    let state = project();
    let parsed = spec(
        &state,
        json!({"scene":"street","preset":"fullhd","from":4,"frames":8}),
    )
    .unwrap();
    assert_eq!((parsed.from, parsed.frames), (4, Some(8)));
    assert_eq!(parsed.preset.id, "fullhd");
    assert_eq!(parsed.audio, AudioChoice::Preset);

    let defaults = spec(&state, json!({"scene":"street","preset":"uhd4k"})).unwrap();
    assert_eq!(
        (defaults.from, defaults.frames),
        (0, None),
        "the whole edit"
    );
    let video_only = spec(
        &state,
        json!({"scene":"street","preset":"fullhd","audio":false}),
    )
    .unwrap();
    assert_eq!(video_only.audio, AudioChoice::Omitted);

    for (request, needle) in [
        (json!({"preset":"fullhd"}), "scene"),
        (json!({"scene":"street"}), "preset"),
        (json!({"scene":"street","preset":"nope"}), "nope"),
        (
            json!({"scene":"street","preset":"fullhd","from":1.5}),
            "from",
        ),
        (
            json!({"scene":"street","preset":"fullhd","frames":"8"}),
            "frames",
        ),
        (
            json!({"scene":"street","preset":"fullhd","audio":"yes"}),
            "audio",
        ),
    ] {
        let error = spec(&state, request.clone()).unwrap_err();
        assert!(
            error.message.contains(needle),
            "{request}: {}",
            error.message
        );
    }
}

#[test]
fn the_project_preset_is_the_default_and_an_inline_preset_is_validated() {
    let mut state = project();
    ok(&mut state, json!({"op":"preset.select","id":"square"}));
    let selected = spec(&state, json!({"scene":"street"})).unwrap();
    assert_eq!(selected.preset.id, "square");

    let inline = spec(
        &state,
        json!({"scene":"street","preset":{"id":"mine","name":"Mine","width":640,"height":360,"fps":30}}),
    )
    .unwrap();
    assert_eq!((inline.preset.width, inline.preset.height), (640, 360));
    let broken = spec(
        &state,
        json!({"scene":"street","preset":{"id":"odd","name":"Odd","width":641,"height":360}}),
    )
    .unwrap_err();
    assert!(broken.message.contains("even"), "{}", broken.message);
}

#[test]
fn same_rate_maps_output_frame_i_to_edit_frame_from_plus_i() {
    let state = project();
    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","from":10,"frames":6}),
    )
    .unwrap();
    let schedule = snapshot.schedule();
    assert_eq!((schedule.frames, schedule.edit_frames), (6, 6));
    assert_eq!(
        schedule.geometry.render,
        (1920, 1080),
        "16:9 renders at the output size"
    );
    assert_eq!(schedule.to(), 16);
    assert_eq!(snapshot.audio(), &AudioPlan::NoSources);
    let frames: Vec<u64> = (0..6)
        .map(|index| snapshot.plan(index).unwrap().frame.edit_frame)
        .collect();
    assert_eq!(frames, [10, 11, 12, 13, 14, 15]);
    let past = snapshot.plan(6).unwrap_err();
    assert_eq!(
        past.code, "invalid_range",
        "[from, from+frames) is half-open"
    );
}

#[test]
fn every_export_frame_is_the_complete_plan_scene_plan_answers() {
    let mut state = project();
    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","from":40,"frames":16}),
    )
    .unwrap();
    for index in 0..16u64 {
        let exported = snapshot.plan(index).unwrap();
        let edit_frame = 40 + index;
        let answer = ok(
            &mut state,
            json!({"op":"scene.plan","scene":"street","frame":edit_frame}),
        );
        assert_eq!(answer["complete"], true);
        let queried = ScenePlan::from_json(&answer["plan"]).unwrap();
        assert!(exported.is_complete());
        assert_eq!(
            exported, queried,
            "frame {edit_frame} (crosses the shot cut at 48)"
        );
    }
}

#[test]
fn a_new_rate_samples_exact_instants_and_repeats_or_drops_without_blending() {
    let state = project();
    // 24 edit frames at 24 fps = 1 s; 30 fps gives 30 output frames: edit = floor(i * 24 / 30).
    let thirty = json!({"id":"p30","name":"30","width":1920,"height":1080,"fps":30});
    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":thirty,"from":0,"frames":24}),
    )
    .unwrap();
    assert_eq!(snapshot.schedule().frames, 30);
    let mapped: Vec<u64> = (0..30)
        .map(|index| snapshot.plan(index).unwrap().frame.edit_frame)
        .collect();
    let expected: Vec<u64> = (0..30u64).map(|i| i * 24 / 30).collect();
    assert_eq!(mapped, expected);
    assert_eq!(snapshot.warnings()[0]["code"], "frame_rate_conversion");

    // 24000/1001 against 24: 24 edit frames last exactly 1 s, 23.976 fps gives round(23.976) = 24.
    let ntsc =
        json!({"id":"n","name":"n","width":1920,"height":1080,"fps":{"num":24000,"den":1001}});
    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":ntsc,"from":0,"frames":96}),
    )
    .unwrap();
    // 96/24 = 4 s at 24000/1001 → 95.904 → 96 frames; the last one samples before 4 s.
    assert_eq!(snapshot.schedule().frames, 96);
    let last = snapshot.plan(95).unwrap().frame.edit_frame;
    assert!(
        last <= 95,
        "the sampled frame never leaves the range, got {last}"
    );
    let exact = snapshot.schedule().sample_time(95).unwrap();
    assert_eq!(
        Some(exact),
        Ratio::new(95 * 1001, 24000),
        "no float: 95 / (24000/1001) seconds exactly"
    );

    // Slower than the scene: 12 fps drops every other edit frame.
    let twelve = json!({"id":"p12","name":"12","width":1920,"height":1080,"fps":12});
    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":twelve,"from":6,"frames":24}),
    )
    .unwrap();
    assert_eq!(snapshot.schedule().frames, 12);
    let mapped: Vec<u64> = (0..12)
        .map(|index| snapshot.plan(index).unwrap().frame.edit_frame)
        .collect();
    assert_eq!(mapped, (0..12u64).map(|i| 6 + 2 * i).collect::<Vec<_>>());
}

#[test]
fn the_aspect_of_the_preset_decides_the_render_size_and_warns() {
    let state = project();
    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":"vertical","from":0,"frames":2}),
    )
    .unwrap();
    assert_eq!(snapshot.schedule().geometry.render, (1080, 608));
    assert_eq!(snapshot.schedule().geometry.output, (1080, 1920));
    assert!(
        snapshot
            .warnings()
            .iter()
            .any(|warning| warning["code"] == "aspect_mismatch")
    );
}

#[test]
fn invalid_requests_are_refused_without_touching_the_project() {
    let mut state = project();
    let before = (
        state.timeline.clips.len(),
        ok(&mut state, json!({"op":"history.status"})),
        ok(&mut state, json!({"op":"scene.describe","scene":"street"})),
    );
    for (request, code) in [
        (
            json!({"scene":"ghost","preset":"fullhd","frames":1}),
            "unknown_element",
        ),
        (
            json!({"scene":"street","preset":"fullhd","from":-1,"frames":1}),
            "invalid_range",
        ),
        (
            json!({"scene":"street","preset":"fullhd","from":96,"frames":1}),
            "invalid_range",
        ),
        (
            json!({"scene":"street","preset":"fullhd","from":0,"frames":0}),
            "invalid_range",
        ),
        (
            json!({"scene":"street","preset":"fullhd","from":90,"frames":7}),
            "invalid_range",
        ),
    ] {
        let error = capture(&state, request.clone()).unwrap_err();
        assert_eq!(error.code, code, "{request}: {}", error.message);
    }
    let after = (
        state.timeline.clips.len(),
        ok(&mut state, json!({"op":"history.status"})),
        ok(&mut state, json!({"op":"scene.describe","scene":"street"})),
    );
    assert_eq!(before, after);
    assert_eq!(after.0, 0, "a scene is never placed on the timeline");
}

fn with_audio() -> Value {
    with_window(0.0, 4.0)
}

fn with_window(start: f64, end: f64) -> Value {
    let mut raw = bundle();
    raw["audio"] = json!({
        "mixRelativePath": "audio/mix.wav",
        "editStartSeconds": start,
        "editEndSecondsExclusive": end,
        "sourcePcmHashSha256": "0".repeat(64),
    });
    raw
}

fn mix_of(snapshot: &SceneExportSnapshot) -> &DirectedAudioMix {
    snapshot
        .audio()
        .mix()
        .unwrap_or_else(|| panic!("no mix planned: {:?}", snapshot.audio()))
}

#[test]
fn a_declared_mix_is_planned_by_default_and_video_only_is_the_explicit_choice() {
    let state = project_with(with_audio());
    let planned = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","frames":4}),
    )
    .unwrap();
    let mix = mix_of(&planned);
    // 4 frames at 24/1 = 1/6 s = 8000 samples of the 48 kHz stream, all of them read from the file.
    assert_eq!(
        (
            mix.output_samples(),
            mix.mix_samples(),
            mix.silence_tail_samples()
        ),
        (8_000, 8_000, 0)
    );
    assert_eq!(mix.origin(), Ratio::ZERO);
    assert_eq!(mix.relative_path(), "audio/mix.wav");
    let audio = planned.summary()["audio"].clone();
    assert_eq!(audio["included"], true);
    assert_eq!(audio["declared"], json!(["audio"]));
    assert_eq!(audio["mix"]["samples"], 8_000);
    assert!(
        planned
            .warnings()
            .iter()
            .all(|warning| warning["code"] != "audio_omitted"),
        "a mixed export omits nothing"
    );

    let explicit = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","frames":4,"audio":false}),
    )
    .unwrap();
    assert_eq!(
        explicit.audio(),
        &AudioPlan::Omitted {
            declared: vec!["audio"]
        }
    );
    assert!(
        explicit
            .warnings()
            .iter()
            .any(|warning| warning["code"] == "audio_omitted"),
        "leaving the sound out is reported, never silent"
    );
    assert_eq!(explicit.summary()["audio"]["included"], false);

    // A preset that disables audio is the same explicit choice.
    let silent =
        json!({"id":"mute","name":"Mute","width":1920,"height":1080,"audio":{"enabled":false}});
    let by_preset = capture(&state, json!({"scene":"street","preset":silent,"frames":4})).unwrap();
    assert!(matches!(by_preset.audio(), AudioPlan::Omitted { .. }));
}

#[test]
fn the_mix_follows_the_range_start_and_the_declared_window_offset() {
    // The file plays at `editStartSeconds + t`: frame 24 of a 24 fps scene is t = 1 s.
    let state = project_with(with_window(10.0, 14.0));
    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","from":24,"frames":24}),
    )
    .unwrap();
    let mix = mix_of(&snapshot);
    assert_eq!(mix.origin(), Ratio::from_int(11));
    assert_eq!(mix.output_samples(), 48_000);
    // Sample 0 is at file second 11, sample 24 000 half a second later (48 kHz source).
    let cursor = mix.cursor(48_000).unwrap();
    assert_eq!(cursor.at(0).base, 11 * 48_000);
    assert_eq!(cursor.at(24_000).base, 11 * 48_000 + 24_000);
    // A 44.1 kHz file is read at the exact fractional frame, with no drift: 1 s of output = 44 100.
    let cursor = mix.cursor(44_100).unwrap();
    assert_eq!(cursor.at(0).base, 11 * 44_100);
    assert_eq!(cursor.at(48_000).base, 12 * 44_100);

    // A decimal the author wrote is exact: 0.1 s starts at frame 4 800, not at a float neighbour.
    let state = project_with(with_window(0.1, 4.1));
    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","from":0,"frames":24}),
    )
    .unwrap();
    assert_eq!(mix_of(&snapshot).origin(), Ratio::new(1, 10).unwrap());
    assert_eq!(mix_of(&snapshot).cursor(48_000).unwrap().at(0).base, 4_800);
}

#[test]
fn a_frame_rate_conversion_keeps_the_stream_as_long_as_the_video() {
    let state = project_with(with_audio());
    // 24 edit frames at 24 fps = 1 s; the 30 fps preset draws 30 frames: 48 000 samples exactly.
    let thirty = json!({"id":"p30","name":"P30","width":640,"height":360,"fps":30});
    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":thirty,"from":40,"frames":24}),
    )
    .unwrap();
    let mix = mix_of(&snapshot);
    assert_eq!(snapshot.schedule().frames, 30);
    assert_eq!(
        (
            mix.output_samples(),
            mix.mix_samples(),
            mix.silence_tail_samples()
        ),
        (48_000, 48_000, 0)
    );
    assert_eq!(mix.origin(), Ratio::new(5, 3).unwrap());

    // 2 edit frames (1/12 s) become 2.5 → 3 frames at 30 fps (0.1 s): the video is 800 samples
    // longer than the sound the range holds, and that tail is declared silence, not borrowed sound.
    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":thirty,"from":0,"frames":2}),
    )
    .unwrap();
    let mix = mix_of(&snapshot);
    assert_eq!(snapshot.schedule().frames, 3);
    assert_eq!(
        (
            mix.output_samples(),
            mix.mix_samples(),
            mix.silence_tail_samples()
        ),
        (4_800, 4_000, 800)
    );
    assert!(
        snapshot
            .warnings()
            .iter()
            .any(|warning| warning["code"] == "audio_silence_tail")
    );

    // NTSC output: 30000/1001 never drifts: 30 frames last exactly 1.001 s = 48 048 samples.
    let ntsc = json!({"id":"n","name":"N","width":640,"height":360,
        "fps":{"num":30000,"den":1001}});
    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":ntsc,"from":0,"frames":24}),
    )
    .unwrap();
    assert_eq!(snapshot.schedule().frames, 30);
    assert_eq!(mix_of(&snapshot).output_samples(), 48_048);
}

#[test]
fn a_range_outside_the_declared_window_is_refused_with_its_numbers() {
    // The window holds 1 s of edit time; the range reaches 2 s.
    let state = project_with(with_window(0.0, 1.0));
    let refused = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","from":0,"frames":48}),
    )
    .unwrap_err();
    assert_eq!(refused.code, "audio_window_short");
    let details = refused.details.as_ref().unwrap();
    assert_eq!(details["range_end"]["num"], 2);
    assert_eq!(details["hint"], "audio: false");
    // The same range fits when it stays inside the window, and video only never needed it.
    capture(
        &state,
        json!({"scene":"street","preset":"fullhd","from":0,"frames":24}),
    )
    .unwrap();
    capture(
        &state,
        json!({"scene":"street","preset":"fullhd","from":0,"frames":48,"audio":false}),
    )
    .unwrap();
    // A start past the end of the window is the same refusal.
    let refused = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","from":30,"frames":2}),
    )
    .unwrap_err();
    assert_eq!(refused.code, "audio_window_short");
}

#[test]
fn a_window_that_exact_time_cannot_hold_is_refused() {
    let state = project_with(with_window(0.0, 1e30));
    let refused = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","frames":4}),
    )
    .unwrap_err();
    assert_eq!(refused.code, "audio_window_invalid");
}

#[test]
fn declared_narration_is_still_refused_and_never_stands_in_for_the_mix() {
    let mut raw = with_audio();
    raw["narration"] = json!({"tracks": [], "caption": {}});
    let state = project_with(raw);
    let refused = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","frames":4}),
    )
    .unwrap_err();
    assert_eq!(refused.code, "unsupported");
    assert_eq!(
        refused.details.as_ref().unwrap()["domains"],
        json!(["narration"])
    );
    let video_only = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","frames":4,"audio":false}),
    )
    .unwrap();
    assert_eq!(
        video_only.audio(),
        &AudioPlan::Omitted {
            declared: vec!["audio", "narration"]
        }
    );
}

#[test]
fn a_scene_without_audio_exports_with_no_audio_stream_and_says_so() {
    let state = project();
    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","frames":4}),
    )
    .unwrap();
    assert_eq!(snapshot.audio(), &AudioPlan::NoSources);
    assert!(
        snapshot
            .warnings()
            .iter()
            .any(|warning| warning["code"] == "no_audio_sources")
    );
}

#[test]
fn a_declared_domain_nobody_evaluates_refuses_the_export_before_any_job() {
    let mut raw = bundle();
    raw["sceneDefinitions"][0]["lighting"] = json!({"ambient": {"intensity": 0.4}});
    let state = project_with(raw);
    let error = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","frames":4}),
    )
    .unwrap_err();
    assert_eq!(error.code, "unsupported");
    assert!(
        error.details.as_ref().unwrap()["domains"]
            .to_string()
            .contains("lighting"),
        "{error:?}"
    );
}

#[test]
fn required_assets_are_what_the_plans_of_the_range_draw() {
    let mut state = project();
    let planned = ok(
        &mut state,
        json!({"op":"scene.plan","scene":"street","frame":0}),
    );
    let mut expected: Vec<String> = ScenePlan::from_json(&planned["plan"])
        .unwrap()
        .required_asset_ids()
        .into_iter()
        .map(str::to_string)
        .collect();
    expected.sort();
    assert!(!expected.is_empty(), "the corpus draws pictures");
    let all = capture(&state, json!({"scene":"street","preset":"fullhd"})).unwrap();
    assert_eq!(all.required_assets(), expected.as_slice());
    let one_shot = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","from":50,"frames":2}),
    )
    .unwrap();
    assert_eq!(one_shot.required_assets(), expected.as_slice());
    assert_eq!(all.summary()["assets"], json!(expected));
}

#[test]
fn a_frozen_export_does_not_follow_later_edits_or_a_detach() {
    let mut state = project();
    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","from":0,"frames":4}),
    )
    .unwrap();
    let before: Vec<ScenePlan> = (0..4).map(|index| snapshot.plan(index).unwrap()).collect();

    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set":"scene/scene-a/node/hero/position","value":{"x":2.5,"y":0.5,"z":0}},
            {"set":"camera/cam-main/start/fov","value":60}]}),
    );
    let edited = ok(
        &mut state,
        json!({"op":"scene.plan","scene":"street","frame":0}),
    );
    assert_ne!(
        ScenePlan::from_json(&edited["plan"]).unwrap(),
        before[0],
        "the edit really changed the scene"
    );
    ok(&mut state, json!({"op":"scene.detach","scene":"street"}));

    let after: Vec<ScenePlan> = (0..4).map(|index| snapshot.plan(index).unwrap()).collect();
    assert_eq!(before, after, "the snapshot keeps the scene it froze");
}

#[test]
fn a_live_gesture_is_never_part_of_an_export() {
    let mut state = project();
    let begun = ok(
        &mut state,
        json!({"op":"scene.gesture.begin","scene":"street","targets":["scene/scene-a/node/hero"]}),
    );
    let gesture = begun["gesture"].as_str().unwrap().to_string();
    ok(
        &mut state,
        json!({"op":"scene.gesture.update","gesture":gesture,"ops":[
            {"set":"scene/scene-a/node/hero/position","value":{"x":3.0,"y":0.5,"z":0}}]}),
    );
    let committed = ok(
        &mut state,
        json!({"op":"scene.plan","scene":"street","frame":0}),
    );
    let preview = ok(
        &mut state,
        json!({"op":"scene.plan","scene":"street","frame":0,"gesture":gesture}),
    );
    assert_ne!(
        committed["plan"], preview["plan"],
        "the preview differs while it is live"
    );

    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":"fullhd","from":0,"frames":1}),
    )
    .unwrap();
    assert_eq!(
        snapshot.plan(0).unwrap(),
        ScenePlan::from_json(&committed["plan"]).unwrap(),
        "the export reads the committed document"
    );
    ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":gesture}),
    );
}

#[test]
fn a_prepared_timeline_plans_exactly_what_a_direct_build_does() {
    let state = project();
    // 24 -> 30 across the cut at 48: both shots, repeated frames, and the last frame of the range.
    let snapshot = capture(
        &state,
        json!({"scene":"street","preset":{"id":"p30","name":"30","width":640,"height":360,"fps":30},
            "from":40,"frames":24}),
    )
    .unwrap();
    let timeline = snapshot.timeline();
    let frames = snapshot.schedule().frames;
    assert_eq!(frames, 30);
    for index in 0..frames {
        assert_eq!(
            snapshot.plan_with(&timeline, index).unwrap(),
            snapshot.plan(index).unwrap(),
            "output frame {index}"
        );
    }
    // The range guard is the snapshot's, not the timeline's: past the end is refused alike.
    let direct = snapshot.plan(frames).unwrap_err();
    let prepared = snapshot.plan_with(&timeline, frames).unwrap_err();
    assert_eq!(
        (prepared.code, prepared.message),
        (direct.code, direct.message)
    );
}

#[test]
fn a_timeline_from_another_snapshot_is_refused_even_for_an_identical_bundle() {
    let state = project();
    let other_state = project();
    let request = json!({"scene":"street","preset":"fullhd","frames":4});
    let snapshot = capture(&state, request.clone()).unwrap();
    let other = capture(&other_state, request).unwrap();
    let timeline = other.timeline();
    assert_eq!(
        snapshot.plan_with(&timeline, 0).unwrap_err().code,
        "stale_scene"
    );
}
