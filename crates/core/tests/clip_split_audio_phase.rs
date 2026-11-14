//! BL081.5 (F017): the sound of a cut clip keeps its sample phase on every frame grid.
//!
//! The oracle is independent of the core: the sources are float32 ramps (`sample k = k * 2^-17`),
//! so a linear interpolation reads back its own position, and every expectation is computed from
//! the physical definition with exact `i128` fractions written in this file:
//!
//! * output sample `n` belongs to the frame boundary `S(f) = floor(f * den * 48000 / num + 1/2)`;
//! * it plays source position `(source_start + (n / 48000 - start / rate) * speed - offset) * sr`.
//!
//! Two kinds of check never use a tolerance: the plan's position of every segment equals the
//! oracle's exact fraction, and the mix after a cut (or in chunks, or after save/undo) is the mix
//! before it **bit for bit**. Sample values are compared with the oracle within 4e-7 (a float32
//! rounding of values below 1; one source sample of phase is 7.6e-6 here, so a lost phase of a
//! few hundredths of a sample is still caught, and `a_plan_that_loses_the_phase_...` proves it).
use std::fs;
use std::path::PathBuf;
use std::process::{Command as Process, Stdio};
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};
use tessel_editor_core::assets::{AudioStream, Decoded, DecodedAudio};
use tessel_editor_core::audio::{AudioBackend, OfflineMixer, Pcm};
use tessel_editor_core::state::split::{SplitAvailability, SplitBlock};
use tessel_editor_core::state::timeline::{TrackFlag, TrimEdge};
use tessel_editor_core::time::Ratio;
use tessel_editor_core::{Command, CommandError, EditorState, headless, persist};

const LENGTH: i64 = 96;
const SOURCE_SECONDS: usize = 5;
/// Source sample `k` is `k * SCALE` on the left channel and half of it on the right.
const SCALE: f64 = 1.0 / 131_072.0;
const TOLERANCE: f64 = 4e-7;
const RATES: [(u32, u32); 6] = [
    (24_000, 1001),
    (30_000, 1001),
    (24, 1),
    (25, 1),
    (30, 1),
    (60, 1),
];

// ---- exact arithmetic of the oracle ----------------------------------------------------------

fn gcd(a: i128, b: i128) -> i128 {
    if b == 0 { a.abs() } else { gcd(b, a % b) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Frac {
    n: i128,
    d: i128,
}

impl Frac {
    fn new(n: i128, d: i128) -> Self {
        let g = gcd(n, d).max(1) * if d < 0 { -1 } else { 1 };
        Self { n: n / g, d: d / g }
    }
    fn int(n: i128) -> Self {
        Self { n, d: 1 }
    }
    fn add(self, o: Self) -> Self {
        Self::new(self.n * o.d + o.n * self.d, self.d * o.d)
    }
    fn sub(self, o: Self) -> Self {
        Self::new(self.n * o.d - o.n * self.d, self.d * o.d)
    }
    fn mul(self, o: Self) -> Self {
        Self::new(self.n * o.n, self.d * o.d)
    }
    fn floor(self) -> i128 {
        self.n.div_euclid(self.d)
    }
    fn f64(self) -> f64 {
        self.n as f64 / self.d as f64
    }
}

fn frac(ratio: Ratio) -> Frac {
    Frac::new(i128::from(ratio.num()), i128::from(ratio.den()))
}

/// One sound as the oracle defines it, from values the test chose (never read from the state).
#[derive(Clone, Copy, Debug)]
struct Spec {
    rate: (i128, i128),
    start: i64,
    duration: i64,
    ss: Frac,
    /// Percent.
    speed: i128,
    offset: Frac,
    sr: i128,
    stereo: bool,
    fade_in: i64,
    fade_out: i64,
    audio_clip: bool,
}

impl Spec {
    fn audio(rate: (u32, u32), sr: u32, stereo: bool, start: i64, duration: i64, ss: Frac) -> Self {
        Self {
            rate: (i128::from(rate.0), i128::from(rate.1)),
            start,
            duration,
            ss,
            speed: 100,
            offset: Frac::int(0),
            sr: i128::from(sr),
            stereo,
            fade_in: 0,
            fade_out: 0,
            audio_clip: true,
        }
    }
    fn end(&self) -> i64 {
        self.start + self.duration
    }
    /// Absolute output sample at which `frame` starts.
    fn boundary(&self, frame: i64) -> i128 {
        let n = i128::from(frame) * self.rate.1 * 48_000;
        (2 * n + self.rate.0) / (2 * self.rate.0)
    }
    fn position(&self, n: i128) -> Frac {
        let clip_time = Frac::new(i128::from(self.start) * self.rate.1, self.rate.0);
        let elapsed = Frac::new(n, 48_000).sub(clip_time);
        self.ss
            .add(elapsed.mul(Frac::new(self.speed, 100)))
            .sub(self.offset)
            .mul(Frac::int(self.sr))
    }
    /// The part `a..b` of this sound, as an independent clip: same sound, new start.
    fn piece(&self, a: i64, b: i64) -> Self {
        let advance = Frac::new(i128::from(a - self.start) * self.rate.1, self.rate.0)
            .mul(Frac::new(self.speed, 100));
        Self {
            start: a,
            duration: b - a,
            ss: self.ss.add(advance),
            fade_in: if a == self.start { self.fade_in } else { 0 },
            fade_out: if b == self.end() { self.fade_out } else { 0 },
            ..*self
        }
    }
    fn gain(&self, n: i128) -> f64 {
        if !self.audio_clip {
            return 1.0;
        }
        let (first, last) = (self.boundary(self.start), self.boundary(self.end()));
        let at = n - first;
        let mut gain = 1.0;
        let ramp_in = self.boundary(self.start + self.fade_in) - first;
        if self.fade_in > 0 && at < ramp_in {
            gain *= at as f64 / ramp_in as f64;
        }
        let remaining = last - n;
        let ramp_out = last - self.boundary(self.end() - self.fade_out);
        if self.fade_out > 0 && remaining <= ramp_out {
            gain *= remaining as f64 / ramp_out as f64;
        }
        gain
    }
}

/// What the oracle says the mix of `specs` is for the frames `from..from + frames`, against `pcm`.
fn worst_error(pcm: &Pcm, specs: &[&Spec], from: i64, frames: i64) -> f64 {
    let any = specs.first().expect("a spec to take the clock from");
    let (origin, end) = (any.boundary(from), any.boundary(from + frames));
    assert_eq!(pcm.frames() as i128, end - origin, "the block length");
    let mut worst = 0.0f64;
    for (index, n) in (origin..end).enumerate() {
        let (mut left, mut right) = (0.0, 0.0);
        for spec in specs {
            if n < spec.boundary(spec.start) || n >= spec.boundary(spec.end()) {
                continue;
            }
            let position = spec.position(n);
            if position.n < 0 || position.floor() >= (SOURCE_SECONDS as i128) * spec.sr {
                continue;
            }
            let value = position.f64() * SCALE * spec.gain(n);
            left += value;
            right += if spec.stereo { value / 2.0 } else { 0.0 };
        }
        worst = worst
            .max((f64::from(pcm.sample(index, 0)) - left).abs())
            .max((f64::from(pcm.sample(index, 1)) - right).abs());
    }
    worst
}

// ---- sources and projects --------------------------------------------------------------------

fn shared_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("tessel-bl0815-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    })
}

/// A float32 WAV of the ramp: left `k * 2^-17`, right `k * 2^-18`, exact in float32.
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
            bytes.extend_from_slice(&(k as f32 * (SCALE as f32)).to_le_bytes());
            if stereo {
                bytes.extend_from_slice(&(k as f32 * (SCALE as f32) / 2.0).to_le_bytes());
            }
        }
        let temp = path.with_extension("tmp");
        fs::write(&temp, bytes).unwrap();
        fs::rename(&temp, &path).unwrap();
    }
    path
}

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn fail(state: &mut EditorState, request: Value) -> CommandError {
    headless::execute(state, &request).expect_err(&format!("{request} must fail"))
}

fn rate_json(rate: (u32, u32)) -> Value {
    json!({"num": rate.0, "den": rate.1})
}

fn project(rate: (u32, u32)) -> EditorState {
    let mut state = EditorState::demo();
    run(
        &mut state,
        json!({"op":"project.create","name":"P","sequence":"main","rate":rate_json(rate),
            "width":64,"height":36,"length":LENGTH}),
    );
    state
}

/// A project with the ramp as an audio asset and one audio track per name.
fn audio_project(rate: (u32, u32), sr: u32, stereo: bool, tracks: &[&str]) -> EditorState {
    let mut state = project(rate);
    run(
        &mut state,
        json!({"op":"asset.add","kind":"audio","id":"ramp","name":"Ramp","path":ramp(sr, stereo)}),
    );
    for track in tracks {
        run(
            &mut state,
            json!({"op":"track.add","id":track,"name":track,"kind":"audio"}),
        );
    }
    state
}

fn add_audio(state: &mut EditorState, id: &str, track: &str, spec: &Spec, pan: Option<i32>) {
    let mut request = json!({"op":"clip.add","id":id,"track":track,"asset":"ramp",
        "start":spec.start,"duration":spec.duration,
        "source_start":{"num":spec.ss.n as i64,"den":spec.ss.d as i64}});
    if let Some(pan) = pan {
        request["properties"] = json!({"pan": pan});
    }
    run(state, request);
}

/// A project with a declared container video whose sound has the given streams
/// `(sample rate, stereo, offset)`, decoded to the ramp.
fn video_project(rate: (u32, u32), streams: &[(u32, bool, Ratio)]) -> EditorState {
    let mut state = project(rate);
    let video = shared_dir().join("fake.mp4");
    fs::write(&video, b"not decoded by the core").unwrap();
    run(
        &mut state,
        json!({"op":"asset.add","kind":"video","id":"vid","name":"Clip","path":video,
            "width":64,"height":36,"rate":24,"duration":4}),
    );
    let item = state
        .library
        .items
        .iter_mut()
        .find(|item| item.id == "vid")
        .unwrap();
    let asset = item.asset.as_mut().unwrap();
    asset.meta.audio_streams = streams
        .iter()
        .enumerate()
        .map(|(index, (sample_rate, stereo, offset))| AudioStream {
            index: index as u32,
            sample_rate: *sample_rate,
            channels: if *stereo { 2 } else { 1 },
            offset: *offset,
        })
        .collect();
    asset.meta.sample_rate = Some(streams[0].0);
    asset.decoded = Some(Decoded {
        frames: None,
        audio: streams
            .iter()
            .enumerate()
            .map(|(index, (sample_rate, stereo, _))| DecodedAudio {
                stream: index as u32,
                path: ramp(*sample_rate, *stereo).to_string_lossy().into_owned(),
            })
            .collect(),
        proxy: None,
    });
    run(
        &mut state,
        json!({"op":"track.add","id":"v","name":"V","kind":"video"}),
    );
    state
}

fn add_video(state: &mut EditorState, id: &str, spec: &Spec, stream: u32, pan: i32) {
    run(
        state,
        json!({"op":"clip.add","id":id,"track":"v","asset":"vid","start":spec.start,
            "duration":spec.duration,"source_start":{"num":spec.ss.n as i64,"den":spec.ss.d as i64},
            "properties":{"speed":spec.speed as i64,"audio_stream":stream,"pan":pan}}),
    );
}

fn mix_range(state: &EditorState, from: i64, frames: i64) -> Pcm {
    let plan = state.plan_audio(from, frames).unwrap();
    OfflineMixer::default().mix(&plan).unwrap()
}

fn mix(state: &EditorState) -> Pcm {
    mix_range(state, 0, LENGTH)
}

/// Every segment sits at the oracle's exact position and the mix is the oracle's, for a block.
fn check(state: &EditorState, from: i64, frames: i64, specs: &[(&str, Spec)], label: &str) -> Pcm {
    let plan = state.plan_audio(from, frames).unwrap();
    let to = from + frames;
    let live: Vec<&(&str, Spec)> = specs
        .iter()
        .filter(|(_, spec)| spec.start < to && spec.end() > from)
        .collect();
    assert_eq!(plan.segments.len(), live.len(), "{label}: segments");
    for segment in &plan.segments {
        let (_, spec) = live
            .iter()
            .find(|(id, _)| segment.clip == *id)
            .unwrap_or_else(|| panic!("{label}: unexpected segment {}", segment.clip));
        let first = spec.boundary(spec.start.max(from));
        let expected = spec.position(first);
        let planned = Frac::int(i128::from(segment.source_start)).add(frac(segment.source_phase));
        assert_eq!(planned, expected, "{label}: {} position", segment.clip);
        assert_eq!(
            segment.frames as i128,
            spec.boundary(spec.end().min(to)) - first,
            "{label}: {} length",
            segment.clip
        );
    }
    let pcm = OfflineMixer::default().mix(&plan).unwrap();
    let all: Vec<&Spec> = specs.iter().map(|(_, spec)| spec).collect();
    let worst = worst_error(&pcm, &all, from, frames);
    assert!(
        worst < TOLERANCE,
        "{label}: the mix is {worst} from the oracle"
    );
    pcm
}

fn same_bits(a: &Pcm, b: &Pcm, label: &str) {
    assert_eq!(a.samples.len(), b.samples.len(), "{label}: length");
    let first = a
        .samples
        .iter()
        .zip(&b.samples)
        .position(|(x, y)| x.to_bits() != y.to_bits());
    assert!(first.is_none(), "{label}: first differing sample {first:?}");
}

fn split(state: &mut EditorState, clip: &str, frame: i64) -> Value {
    run(state, json!({"op":"clip.split","clip":clip,"frame":frame}))
}

fn fingerprint(state: &EditorState) -> Value {
    json!({
        "document": state.to_document(None),
        "history": state.history.to_json(),
        "revision": state.project.revision,
        "selected": state.timeline.selected_clip.map(|id| id.to_string()),
        "playhead": state.timeline.playhead,
    })
}

fn ss_of(state: &EditorState, clip: &str) -> Frac {
    frac(state.timeline.clip(clip).unwrap().source_start)
}

#[path = "clip_split_audio_phase/controls.rs"]
mod controls;
#[path = "clip_split_audio_phase/cuts.rs"]
mod cuts;
#[path = "clip_split_audio_phase/edits.rs"]
mod edits;
#[path = "clip_split_audio_phase/history.rs"]
mod history;
#[path = "clip_split_audio_phase/mixing.rs"]
mod mixing;
