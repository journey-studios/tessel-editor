//! BL081.2 (F017, F051): cutting a clip inside a fade ramp keeps the envelope of the whole clip.
//!
//! Two checks carry the proof and neither uses a tolerance:
//!
//! * **picture**: the plan of every frame (opacity included) before and after the cut is the same
//!   JSON, number for number, apart from the identity of the clip that produced the layer;
//! * **sound**: the mix before and after the cut is the same PCM, bit for bit, whole or in blocks.
//!
//! Every accepting test has a control that drops the fade window (the model that loses the ramp
//! of the original clip) and must fail the same check, so the check is known to see an envelope
//! that restarts at the cut. The behaviour of edits after a cut (move, trim, speed, editing or
//! zeroing a fade, undo, save/reopen) is pinned by the contract in
//! `docs/migration/clip-split-fades-report.md`.
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command as Process, Stdio};
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};
use tessel_editor_core::audio::{AudioBackend, OfflineMixer, Pcm, decode_wav};
use tessel_editor_core::fixtures::{proof_requests, write_standard_set};
use tessel_editor_core::state::envelope::FadeWindow;
use tessel_editor_core::state::property_gesture::GestureCommand;
use tessel_editor_core::state::timeline::TrimEdge;
use tessel_editor_core::{Command, CommandError, EditorState, Raw, headless, persist};

const LENGTH: i64 = 96;
const SOURCE_SECONDS: usize = 5;
const SCALE: f32 = 1.0 / 131_072.0;
const RATES: [(u32, u32); 6] = [
    (24_000, 1001),
    (30_000, 1001),
    (24, 1),
    (25, 1),
    (30, 1),
    (60, 1),
];

// ---- plumbing -----------------------------------------------------------------------------

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn fail(state: &mut EditorState, request: Value) -> CommandError {
    headless::execute(state, &request).expect_err(&format!("{request} must fail"))
}

fn split(state: &mut EditorState, clip: &str, frame: i64) -> Value {
    run(state, json!({"op":"clip.split","clip":clip,"frame":frame}))
}

fn set(state: &mut EditorState, clip: &str, property: &str, value: Value) {
    run(
        state,
        json!({"op":"clip.property","clip":clip,"property":property,"value":value}),
    );
}

fn rate_json(rate: (u32, u32)) -> Value {
    json!({"num": rate.0, "den": rate.1})
}

fn shared_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("tessel-bl0812-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    })
}

/// A float32 WAV of the ramp (left `k * 2^-17`, right half of it): every sample is distinct, so a
/// lost phase or a wrong gain cannot hide.
fn ramp(sample_rate: u32, stereo: bool) -> PathBuf {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = shared_dir().join(format!(
        "ramp-{sample_rate}-{}.wav",
        if stereo { 2 } else { 1 }
    ));
    if !path.exists() {
        let channels: u16 = if stereo { 2 } else { 1 };
        let frames = SOURCE_SECONDS * sample_rate as usize;
        let data = frames * usize::from(channels) * 4;
        let mut bytes = Vec::with_capacity(44 + data);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&3u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * u32::from(channels) * 4).to_le_bytes());
        bytes.extend_from_slice(&(channels * 4).to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data as u32).to_le_bytes());
        for k in 0..frames {
            bytes.extend_from_slice(&(k as f32 * SCALE + 1.0 / 64.0).to_le_bytes());
            if stereo {
                bytes.extend_from_slice(&((k as f32 * SCALE + 1.0 / 64.0) / 2.0).to_le_bytes());
            }
        }
        let temp = path.with_extension("tmp");
        fs::write(&temp, bytes).unwrap();
        fs::rename(&temp, &path).unwrap();
    }
    path
}

fn create(rate: (u32, u32)) -> EditorState {
    let mut state = EditorState::demo();
    run(
        &mut state,
        json!({"op":"project.create","name":"P","sequence":"main","rate":rate_json(rate),
            "width":64,"height":36,"length":LENGTH}),
    );
    state
}

/// One audio clip `main` (starts at 3, 60 frames, reads from 1/4 s) on track `a1`.
fn audio_project(rate: (u32, u32), sample_rate: u32, stereo: bool) -> EditorState {
    let mut state = create(rate);
    run(
        &mut state,
        json!({"op":"asset.add","kind":"audio","id":"ramp","name":"Ramp",
            "path":ramp(sample_rate, stereo)}),
    );
    run(
        &mut state,
        json!({"op":"track.add","id":"a1","name":"a1","kind":"audio"}),
    );
    run(
        &mut state,
        json!({"op":"clip.add","id":"main","track":"a1","asset":"ramp","start":3,"duration":60,
            "source_start":{"num":1,"den":4},"properties":{"pan":-35}}),
    );
    state
}

/// One title `t` (starts at 6, 60 frames) on video track `vt`, a second video track `ut`.
fn visual_project(rate: (u32, u32)) -> EditorState {
    let mut state = create(rate);
    run(
        &mut state,
        json!({"op":"asset.add","kind":"text","id":"label","name":"Label",
            "text":{"content":"TESSEL","size":14,"color":"#ffffff"}}),
    );
    for track in ["vt", "ut"] {
        run(
            &mut state,
            json!({"op":"track.add","id":track,"name":track,"kind":"video"}),
        );
    }
    run(
        &mut state,
        json!({"op":"clip.add","id":"t","track":"vt","asset":"label","start":6,"duration":60,
            "properties":{"opacity":80}}),
    );
    state
}

fn mix_with(mixer: &mut OfflineMixer, state: &EditorState, from: i64, frames: i64) -> Pcm {
    let plan = state.plan_audio(from, frames).unwrap();
    mixer.mix(&plan).unwrap()
}

fn mix_range(state: &EditorState, from: i64, frames: i64) -> Pcm {
    mix_with(&mut OfflineMixer::default(), state, from, frames)
}

fn mix(state: &EditorState) -> Pcm {
    mix_range(state, 0, LENGTH)
}

fn first_difference(a: &Pcm, b: &Pcm) -> Option<usize> {
    assert_eq!(a.samples.len(), b.samples.len(), "length");
    a.samples
        .iter()
        .zip(&b.samples)
        .position(|(x, y)| x.to_bits() != y.to_bits())
}

fn same_bits(a: &Pcm, b: &Pcm, label: &str) {
    let first = first_difference(a, b);
    assert!(first.is_none(), "{label}: first differing sample {first:?}");
}

/// The plan of every frame in `frames`, without the identity of the layers (a cut changes which
/// clip makes a layer, never what the layer shows) and without the wording of warnings.
fn plans(state: &mut EditorState, frames: std::ops::Range<i64>) -> Vec<Value> {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.remove("id");
                map.remove("clip");
                map.remove("message");
                map.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    frames
        .map(|frame| {
            let mut plan = run(state, json!({"op":"plan","frame":frame}));
            strip(&mut plan);
            plan
        })
        .collect()
}

fn opacity_at(state: &mut EditorState, frame: i64) -> f64 {
    let plan = run(state, json!({"op":"plan","frame":frame}));
    plan["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|layer| layer["kind"] != "scene")
        .map_or(0.0, |layer| layer["opacity"].as_f64().unwrap())
}

fn first_plan_difference(a: &[Value], b: &[Value]) -> Option<usize> {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).position(|(x, y)| x != y)
}

fn window_of(state: &EditorState, clip: &str) -> FadeWindow {
    state.timeline.clip(clip).unwrap().fade_window
}

fn drop_windows(state: &mut EditorState) {
    for clip in &mut state.timeline.clips {
        clip.fade_window = FadeWindow::default();
    }
}

/// Everything a refused command must leave alone.
fn fingerprint(state: &EditorState) -> Value {
    json!({
        "document": state.to_document(None),
        "history": state.history.to_json(),
        "revision": state.project.revision,
        "selected": state.timeline.selected_clip.map(|id| id.to_string()),
        "playhead": state.timeline.playhead,
    })
}

fn number(state: &EditorState, clip: &str, property: &str) -> f64 {
    let property = tessel_editor_core::state::inspector::property_def(property).unwrap();
    match state.inspector.value(clip, property) {
        tessel_editor_core::state::inspector::Value::Number(number) => number,
        other => panic!("{other:?} is not a number"),
    }
}

/// Cut frames around the interesting points of a clip `start..end` with fades `fade_in` and
/// `fade_out`: both edges of each ramp, the middle and the frames next to the edges.
fn cut_frames(start: i64, end: i64, fade_in: i64, fade_out: i64) -> Vec<i64> {
    let mut frames = vec![start + 1, (start + end) / 2, end - 1];
    for ramp in [start + fade_in, end - fade_out] {
        frames.extend([ramp - 2, ramp - 1, ramp, ramp + 1]);
    }
    frames.retain(|frame| *frame > start && *frame < end);
    frames.sort_unstable();
    frames.dedup();
    frames
}

// ---- picture --------------------------------------------------------------------------------

/// Fade sets: ramp in, ramp out, both, two ramps that overlap (their product dips), one ramp
/// longer than half of the clip.
const VISUAL_FADES: [(i64, i64); 5] = [(12, 0), (0, 12), (12, 24), (40, 40), (48, 0)];

#[path = "clip_split_envelope/visual.rs"]
mod visual;

#[path = "clip_split_envelope/audio.rs"]
mod audio;

#[path = "clip_split_envelope/documents.rs"]
mod documents;

#[path = "clip_split_envelope/edits.rs"]
mod edits;

#[path = "clip_split_envelope/surfaces.rs"]
mod surfaces;

#[path = "clip_split_envelope/ramp_support.rs"]
mod ramp_support;

#[path = "clip_split_envelope/ramps.rs"]
mod ramps;

#[path = "clip_split_envelope/ramp_edits.rs"]
mod ramp_edits;

#[path = "clip_split_envelope/ramp_documents.rs"]
mod ramp_documents;

#[path = "clip_split_envelope/ramp_surfaces.rs"]
mod ramp_surfaces;

fn halves() -> EditorState {
    let mut state = visual_project((24, 1));
    set(&mut state, "t", "fade_in", json!(12));
    set(&mut state, "t", "fade_out", json!(24));
    split(&mut state, "t", 10);
    assert_eq!(window_of(&state, "t-split-1").lead.unwrap().frames, 4);
    state
}

const AUDIO_FADES: [(i64, i64); 5] = [(12, 0), (0, 12), (12, 24), (30, 30), (44, 0)];
