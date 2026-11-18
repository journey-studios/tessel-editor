//! F071: the player (waveform, progress bar, timer) and the caption track, through the headless
//! contract: add, edit, plan frame by frame, undo, split and trim, save, reopen, SRT in and out,
//! and the refusals that leave the project untouched.
//!
//! The audio is a synthetic WAV (a quiet second, then a loud one, repeated), so the waveform has
//! a known shape; nothing here is a real recording.
use core::prelude::v1::test;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::audio::{Pcm, encode_wav_pcm16};
use tessel_editor_core::state::timeline::TrimEdge;
use tessel_editor_core::{Command, EditorState, headless};

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn refused(state: &mut EditorState, request: Value) -> String {
    headless::execute(state, &request)
        .expect_err(&format!("{request} must be refused"))
        .code
        .to_string()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-ps-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Four seconds of mono 48 kHz: seconds 0 and 1 at amplitude 0.1, seconds 2 and 3 at 0.8.
fn write_voice(dir: &Path) -> PathBuf {
    let rate = 48_000usize;
    let samples = (0..rate * 4)
        .map(|at| {
            let level = if at / rate < 2 { 0.1 } else { 0.8 };
            level * (at as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin()
        })
        .collect();
    let path = dir.join("voice.wav");
    fs::write(
        &path,
        encode_wav_pcm16(&Pcm {
            sample_rate: rate as u32,
            channels: 1,
            samples,
        }),
    )
    .unwrap();
    path
}

/// 30 fps, 1280x720, 10 s. The voice (4 s) sits at frames 30..150; the player and the captions
/// cover frames 0..240.
fn project(dir: &Path) -> EditorState {
    let voice = write_voice(dir);
    let mut state = EditorState::demo();
    for request in [
        json!({"op":"project.create","name":"Live","rate":{"num":30,"den":1},
            "width":1280,"height":720,"length":300}),
        json!({"op":"asset.add","kind":"audio","id":"voice","name":"Voice","path":voice}),
        json!({"op":"track.add","id":"sound","name":"Sound","kind":"audio"}),
        json!({"op":"track.add","id":"overlay","name":"Overlay","kind":"video"}),
        json!({"op":"clip.add","id":"voice","track":"sound","asset":"voice","start":30,"duration":120}),
        json!({"op":"clip.add","id":"pl","track":"overlay","start":0,"duration":240,
            "player":{"audio":"voice"}}),
        json!({"op":"track.add","id":"captions","name":"Captions","kind":"video"}),
        json!({"op":"clip.add","id":"subs","track":"captions","start":0,"duration":240,
            "subtitles":{"cues":[
                {"start_ms":500,"end_ms":1500,"text":"Primeira"},
                {"start_ms":2500,"end_ms":3500,"text":"Segunda"}]}}),
    ] {
        run(&mut state, request);
    }
    state
}

fn layers(state: &mut EditorState, frame: i64) -> Vec<Value> {
    run(state, json!({"op":"plan","frame":frame}))["layers"]
        .as_array()
        .unwrap()
        .clone()
}

fn layer<'a>(layers: &'a [Value], id: &str) -> &'a Value {
    layers
        .iter()
        .find(|layer| layer["id"] == id)
        .unwrap_or_else(|| panic!("no layer {id}"))
}

fn near(value: &Value, expected: f64) {
    let value = value
        .as_f64()
        .unwrap_or_else(|| panic!("{value} is not a number"));
    assert!((value - expected).abs() < 1e-6, "{value} is not {expected}");
}

fn caption(layers: &[Value], clip: &str) -> Option<String> {
    layers
        .iter()
        .find(|layer| layer["id"] == format!("layer:{clip}:text"))
        .map(|layer| layer["text"]["content"].as_str().unwrap().to_string())
}

#[test]
fn a_player_follows_its_audio_clip_frame_by_frame() {
    let dir = scratch("clock");
    let mut state = project(&dir);

    // Before the audio starts the player is held at zero; the fill is not drawn at all.
    let before = layers(&mut state, 0);
    assert!(before.iter().all(|layer| layer["id"] != "layer:pl:fill"));
    assert_eq!(
        layer(&before, "layer:pl:time")["text"]["content"],
        "0:00 / 0:04"
    );

    // Half way through the 4 s voice: half of the 60%-wide track is filled.
    let half = layers(&mut state, 90);
    near(&layer(&half, "layer:pl:fill")["shape"]["width"], 30.0);
    near(&layer(&half, "layer:pl:track")["shape"]["width"], 60.0);
    assert_eq!(
        layer(&half, "layer:pl:time")["text"]["content"],
        "0:02 / 0:04"
    );

    // After the voice the player is held at the end, however far the timeline goes.
    for frame in [150, 200, 239] {
        let held = layers(&mut state, frame);
        near(&layer(&held, "layer:pl:fill")["shape"]["width"], 60.0);
        assert_eq!(
            layer(&held, "layer:pl:time")["text"]["content"],
            "0:04 / 0:04"
        );
    }

    // 48 bars, the played ones in the wave colour and the rest in the idle one.
    let bars: Vec<&Value> = half
        .iter()
        .filter(|layer| layer["id"].as_str().unwrap().starts_with("layer:pl:bar-"))
        .collect();
    assert_eq!(bars.len(), 48);
    assert_eq!(bars[0]["shape"]["fill"], "#ffffff");
    assert_eq!(bars[23]["shape"]["fill"], "#ffffff");
    assert_eq!(bars[24]["shape"]["fill"], "#5d6b82");
    assert_eq!(bars[47]["shape"]["fill"], "#5d6b82");
}

#[test]
fn the_waveform_bars_are_the_peaks_of_the_real_audio() {
    let dir = scratch("peaks");
    let mut state = project(&dir);
    let frame = layers(&mut state, 90);
    let height = |index: usize| {
        layer(&frame, &format!("layer:pl:bar-{index}"))["shape"]["height"]
            .as_f64()
            .unwrap()
    };
    // The first half of the voice is eight times quieter than the second.
    assert!(
        height(40) > height(5) * 4.0,
        "loud {} vs quiet {}",
        height(40),
        height(5)
    );
    // The loudest bar is the full height of the player (14% of the canvas).
    near(&json!(height(40)), 14.0);
    // Cached or not, the same frame resolves to the same layers.
    assert_eq!(layers(&mut state, 90), frame);
}

#[test]
fn a_player_without_audio_draws_flat_bars_and_still_runs_its_clock() {
    let dir = scratch("flat");
    let mut state = project(&dir);
    run(
        &mut state,
        json!({"op":"player.set","clip":"pl","field":"audio","value":""}),
    );
    // Following its own clip: 240 frames at 30 fps is 8 s.
    let frame = layers(&mut state, 120);
    assert_eq!(
        layer(&frame, "layer:pl:time")["text"]["content"],
        "0:04 / 0:08"
    );
    near(&layer(&frame, "layer:pl:fill")["shape"]["width"], 30.0);
    let flat = layer(&frame, "layer:pl:bar-3")["shape"]["height"]
        .as_f64()
        .unwrap();
    let other = layer(&frame, "layer:pl:bar-30")["shape"]["height"]
        .as_f64()
        .unwrap();
    assert_eq!(flat, other, "no audio, no peaks: every bar is the same");
}

#[test]
fn player_edits_are_one_undo_entry_each_and_refusals_change_nothing() {
    let dir = scratch("edits");
    let mut state = project(&dir);
    let start = state.to_document(None);

    run(
        &mut state,
        json!({"op":"player.set","clip":"pl","field":"bars","value":24}),
    );
    let after = state.to_document(None);
    assert_eq!(
        layers(&mut state, 90)
            .iter()
            .filter(|layer| layer["id"].as_str().unwrap().starts_with("layer:pl:bar-"))
            .count(),
        24
    );
    // The same value again is not an edit.
    let again = run(
        &mut state,
        json!({"op":"player.set","clip":"pl","field":"bars","value":24}),
    );
    assert_eq!(again["changed"], false);

    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(state.to_document(None), start);
    run(&mut state, json!({"op":"history.redo"}));
    assert_eq!(state.to_document(None), after);

    for request in [
        json!({"op":"player.set","clip":"pl","field":"bars","value":500}),
        json!({"op":"player.set","clip":"pl","field":"fill_color","value":"red"}),
        json!({"op":"player.set","clip":"pl","field":"nope","value":1}),
        json!({"op":"player.set","clip":"subs","field":"bars","value":10}),
    ] {
        let before = state.to_document(None);
        refused(&mut state, request);
        assert_eq!(state.to_document(None), before);
    }
    // A link to a clip that is not audio is refused, with the field named.
    let error = headless::execute(
        &mut state,
        &json!({"op":"player.set","clip":"pl","field":"audio","value":"subs"}),
    )
    .unwrap_err();
    assert_eq!(error.code, "invalid_link");
}

#[test]
fn the_semantic_set_and_invoke_reach_the_same_edits() {
    let dir = scratch("semantic");
    let mut state = project(&dir);
    run(
        &mut state,
        json!({"op":"set","target":"clip_player:pl","field":"time","value":"false"}),
    );
    assert!(
        layers(&mut state, 90)
            .iter()
            .all(|layer| layer["id"] != "layer:pl:time"),
        "the timer is off"
    );
    run(
        &mut state,
        json!({"op":"set","target":"subtitle_cue:subs/0","field":"text","value":"Mudou"}),
    );
    assert_eq!(
        caption(&layers(&mut state, 20), "subs").as_deref(),
        Some("Mudou")
    );
    run(
        &mut state,
        json!({"op":"set","target":"clip_subtitles:subs","field":"burn","value":"false"}),
    );
    let document = state.to_document(None);
    assert!(document["sequences"][0]["clips"].as_array().unwrap().len() >= 3);

    // `add_cue` puts a cue at the playhead of the caption clip's own clock.
    run(
        &mut state,
        json!({"op":"set","target":"timeline.playhead","field":"frame","value":150}),
    );
    let added = run(
        &mut state,
        json!({"op":"invoke","target":"clip_subtitles:subs","action":"add_cue"}),
    );
    assert_eq!(added["cue"], 2);
    assert_eq!(
        caption(&layers(&mut state, 155), "subs").as_deref(),
        Some("Nova legenda")
    );
}

#[test]
fn captions_show_the_cue_the_content_clock_is_on_and_nothing_between() {
    let dir = scratch("captions");
    let mut state = project(&dir);
    // 30 fps: 500..1500 ms is frames 15..45, 2500..3500 ms is frames 75..105.
    assert_eq!(caption(&layers(&mut state, 14), "subs"), None);
    assert_eq!(
        caption(&layers(&mut state, 15), "subs").as_deref(),
        Some("Primeira")
    );
    assert_eq!(
        caption(&layers(&mut state, 44), "subs").as_deref(),
        Some("Primeira")
    );
    assert_eq!(caption(&layers(&mut state, 45), "subs"), None);
    assert_eq!(
        caption(&layers(&mut state, 75), "subs").as_deref(),
        Some("Segunda")
    );
    assert_eq!(caption(&layers(&mut state, 105), "subs"), None);
    // The box sits behind the text.
    let frame = layers(&mut state, 20);
    let order: Vec<&str> = frame
        .iter()
        .map(|layer| layer["id"].as_str().unwrap())
        .filter(|id| id.starts_with("layer:subs:"))
        .collect();
    assert_eq!(order, ["layer:subs:box", "layer:subs:text"]);
}

#[test]
fn splitting_and_trimming_a_caption_clip_keep_every_cue_on_its_frame() {
    let dir = scratch("split");
    let mut state = project(&dir);
    let wanted: Vec<(i64, Option<String>)> = [20, 80, 100, 110, 200]
        .into_iter()
        .map(|frame| (frame, caption(&layers(&mut state, frame), "subs")))
        .collect();

    run(
        &mut state,
        json!({"op":"clip.split","clip":"subs","frame":60,"new_id":"subs-r"}),
    );
    for (frame, expected) in &wanted {
        let now = layers(&mut state, *frame);
        let shown = caption(&now, "subs").or_else(|| caption(&now, "subs-r"));
        assert_eq!(&shown, expected, "after the split, frame {frame}");
    }

    // Cutting the head of the right half moves where it starts reading, not the cues.
    state
        .execute(Command::TrimClip {
            clip: "subs-r".into(),
            edge: TrimEdge::Start,
            frame: 90,
        })
        .unwrap();
    assert_eq!(
        caption(&layers(&mut state, 100), "subs-r").as_deref(),
        Some("Segunda")
    );
    assert_eq!(
        caption(&layers(&mut state, 80), "subs-r"),
        None,
        "trimmed away"
    );
}

#[test]
fn srt_comes_in_and_goes_out_and_the_clip_grows_to_hold_it() {
    let dir = scratch("srt");
    let mut state = project(&dir);
    run(
        &mut state,
        json!({"op":"track.add","id":"extra","name":"Extra","kind":"video"}),
    );
    run(
        &mut state,
        json!({"op":"clip.add","id":"short","track":"extra","start":100,"duration":10,
            "subtitles":{}}),
    );
    let srt = "1\r\n00:00:00,000 --> 00:00:01,000\r\nOlá, mundo\r\n\r\n\
               2\r\n00:00:01,500 --> 00:00:03,000\r\nSegunda\r\nlinha\r\n";
    let imported = run(
        &mut state,
        json!({"op":"subtitles.import","clip":"short","srt":srt}),
    );
    assert_eq!(imported["imported"], 2);
    // The last cue ends at 3 s (90 frames): the 10-frame clip grew to hold it.
    assert_eq!(imported["fitted"], true);
    assert_eq!(state.timeline.clip("short").unwrap().duration, 90);
    let out = dir.join("out.srt");
    let exported = run(
        &mut state,
        json!({"op":"subtitles.export","clip":"short","out":out}),
    );
    assert_eq!(exported["cues"], 2);
    let written = fs::read_to_string(&out).unwrap();
    assert_eq!(written, exported["srt"].as_str().unwrap());
    assert_eq!(
        written,
        "1\n00:00:00,000 --> 00:00:01,000\nOlá, mundo\n\n2\n00:00:01,500 --> 00:00:03,000\nSegunda\nlinha\n"
    );

    // An import from a file is the same import.
    let mut other = project(&dir);
    run(
        &mut other,
        json!({"op":"subtitles.import","clip":"subs","path":out}),
    );
    assert_eq!(
        other.subtitles_of("subs").unwrap().1.cues,
        state.subtitles_of("short").unwrap().1.cues
    );

    // A refused SRT (a cue without timing) changes nothing.
    let before = state.to_document(None);
    refused(
        &mut state,
        json!({"op":"subtitles.import","clip":"short","srt":"1\nno timing here\n"}),
    );
    assert_eq!(state.to_document(None), before);
    refused(
        &mut state,
        json!({"op":"subtitles.import","clip":"short","srt":"x","path":out}),
    );
    refused(
        &mut state,
        json!({"op":"subtitles.import","clip":"short","path":dir.join("missing.srt")}),
    );
}

#[test]
fn cue_edits_keep_the_track_ordered_and_refuse_overlaps() {
    let dir = scratch("cues");
    let mut state = project(&dir);
    // Added out of order, the cue lands where its start puts it.
    let added = run(
        &mut state,
        json!({"op":"subtitles.cue.add","clip":"subs","start_ms":1600,"end_ms":2400,"text":"Meio"}),
    );
    assert_eq!(added["cue"], 1);
    let starts: Vec<i64> = state
        .subtitles_of("subs")
        .unwrap()
        .1
        .cues
        .iter()
        .map(|cue| cue.start_ms)
        .collect();
    assert_eq!(starts, [500, 1600, 2500]);

    for request in [
        json!({"op":"subtitles.cue.add","clip":"subs","start_ms":1000,"end_ms":1800,"text":"Sobrepõe"}),
        json!({"op":"subtitles.cue.add","clip":"subs","start_ms":3000,"end_ms":2000,"text":"Ao contrário"}),
        json!({"op":"subtitles.cue.add","clip":"subs","start_ms":5000,"end_ms":6000,"text":""}),
        json!({"op":"subtitles.cue.set","clip":"subs","index":0,"field":"end_ms","value":1700}),
        json!({"op":"subtitles.cue.set","clip":"subs","index":9,"field":"text","value":"x"}),
        json!({"op":"subtitles.cue.remove","clip":"subs","index":9}),
        json!({"op":"subtitles.set","clip":"subs","field":"size","value":99}),
        json!({"op":"subtitles.set","clip":"subs","field":"box","value":"blue"}),
        json!({"op":"subtitles.set","clip":"pl","field":"size","value":5}),
    ] {
        let before = state.to_document(None);
        refused(&mut state, request);
        assert_eq!(state.to_document(None), before);
    }

    run(
        &mut state,
        json!({"op":"subtitles.cue.remove","clip":"subs","index":1}),
    );
    assert_eq!(state.subtitles_of("subs").unwrap().1.cues.len(), 2);
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(state.subtitles_of("subs").unwrap().1.cues.len(), 3);
}

#[test]
fn save_and_reopen_reproduce_both_clips_and_their_plan() {
    let dir = scratch("reopen");
    let mut state = project(&dir);
    run(
        &mut state,
        json!({"op":"player.set","clip":"pl","field":"bars","value":32}),
    );
    run(
        &mut state,
        json!({"op":"subtitles.set","clip":"subs","field":"burn","value":"false"}),
    );
    run(
        &mut state,
        json!({"op":"subtitles.set","clip":"subs","field":"box","value":"none"}),
    );

    let document = state.to_document(None);
    assert_eq!(document["schema_version"], 2);
    let mut reopened = EditorState::from_document(&document, None).unwrap();
    assert_eq!(reopened.to_document(None), document);
    for frame in [0, 20, 90, 100, 200] {
        assert_eq!(
            layers(&mut reopened, frame),
            layers(&mut state, frame),
            "frame {frame}"
        );
    }
    assert!(!reopened.subtitles_of("subs").unwrap().1.burn);

    // A schema 1 reader would drop both kinds: a document that carries them cannot claim it.
    let mut forged = document.clone();
    forged["schema_version"] = json!(1);
    assert!(EditorState::from_document(&forged, None).is_err());

    // A project without them is still schema 1.
    let plain = EditorState::demo().to_document(None);
    assert_eq!(plain["schema_version"], 1);

    // A damaged spec refuses the whole document, with the clip named.
    let mut broken = document.clone();
    let clips = broken["sequences"][0]["clips"].as_array_mut().unwrap();
    let player = clips.iter_mut().find(|clip| clip["id"] == "pl").unwrap();
    player["player"]["bars"] = json!(1000);
    let error = EditorState::from_document(&broken, None).unwrap_err();
    assert!(format!("{error:?}").contains("pl"), "{error:?}");
}

#[test]
fn the_toolbar_actions_add_one_undoable_clip_each() {
    let mut state = EditorState::demo();
    for action in ["timeline.add_player", "timeline.add_subtitles"] {
        // The playhead is not part of an undo (it stays where the add put it), so the clips
        // and tracks are what must come back.
        let before = state.to_document(None)["sequences"].clone();
        run(
            &mut state,
            json!({"op":"invoke","target":action,"action":"invoke"}),
        );
        assert_ne!(state.to_document(None)["sequences"], before);
        run(&mut state, json!({"op":"history.undo"}));
        assert_eq!(
            state.to_document(None)["sequences"],
            before,
            "{action}: one gesture, one undo"
        );
    }
}

#[test]
fn the_export_snapshot_sets_unburned_captions_aside_on_the_videos_clock() {
    let dir = scratch("sidecar");
    let mut state = project(&dir);
    run(
        &mut state,
        json!({"op":"subtitles.set","clip":"subs","field":"burn","value":"false"}),
    );
    let before = state.to_document(None);

    // Export frames 30..150: the cues are on the clip's clock (500..1500 ms and 2500..3500 ms),
    // the video's clock starts one second in, so the first cue is cut to its last half second
    // and the second one starts at 1.5 s.
    let mut snapshot = state.clone();
    let sidecars = snapshot.set_aside_unburned_captions(30, 150);
    assert_eq!(sidecars.len(), 1);
    assert_eq!(sidecars[0].cues, 2);
    assert_eq!(
        sidecars[0].srt,
        "1\n00:00:00,000 --> 00:00:00,500\nPrimeira\n\n2\n00:00:01,500 --> 00:00:02,500\nSegunda\n"
    );
    // Nothing is drawn for them in the snapshot, and the project itself is untouched.
    assert!(snapshot.timeline.clip("subs").is_none());
    assert_eq!(state.to_document(None), before);
    assert!(
        caption(&layers(&mut state, 20), "subs").is_some(),
        "the monitor still shows them"
    );

    // Burned captions are not set aside.
    run(
        &mut state,
        json!({"op":"subtitles.set","clip":"subs","field":"burn","value":"true"}),
    );
    assert!(state.clone().set_aside_unburned_captions(0, 240).is_empty());
}
