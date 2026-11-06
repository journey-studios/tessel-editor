//! F042A (BL016), the audio of a directed-scene export, without a browser: real FFmpeg, real WAV files,
//! real core snapshots and the real job registry; the frame source alone is a stand-in
//! (`StillRenderer`, one real PNG per frame), as in `scene_export.rs`.
//!
//! Two layers:
//!
//! * **Exact samples** (`render_audio` called directly): the stream the encoder is handed, compared
//!   sample by sample with a value computed in the test from the declared window, the range and the
//!   source rate (copy, offset, 44.1 kHz interpolation, mono centring, declared tail silence, NaN,
//!   clipping, identity of the approved file, unsupported shapes).
//! * **Through the host** (`scene.export`): the MP4 carries an AAC stream whose spectrum, decoded
//!   back, holds the tone of the declared window at the requested place; refusals happen before a
//!   job, swapped bytes fail before a frame is drawn, preflight reports a vanished mix.
//!
//! Nothing here is heard: the samples are measured. A machine without FFmpeg skips;
//! `TESSEL_REQUIRE_RUNTIME=1` turns the skip into a failure.

use std::f64::consts::PI;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tessel_editor_core::jobs::{CancelToken, JobError, JobKind, JobRegistry};
use tessel_editor_core::scene_export::{SceneExportSnapshot, SceneExportSpec};
use tessel_editor_core::scene_resources::{ResourceKey, ResourceKind};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::scene_encoder::{
    FrameJob, InspectError, MixReport, RenderedFrame, SceneFrameRenderer, SceneRendererFactory,
    check_layout, render_audio, render_audio_private,
};
use tessel_editor_media::scene_resources::{
    ApprovedResource, Generation, ResolveError, ResourceResolver, ResourceSnapshot, snapshot_bytes,
};
use tessel_editor_media::tools::Tools;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/directed-scene-proof/corpus")
}

// ---------------------------------------------------------------------------------------------
// WAV files written here, with known content.
// ---------------------------------------------------------------------------------------------

fn wav(tag: u16, bits: u16, channels: u16, rate: u32, data: &[u8]) -> Vec<u8> {
    let block = channels * bits / 8;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
    out.extend_from_slice(&block.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    out
}

fn pcm16(channels: u16, rate: u32, samples: &[i16]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    wav(1, 16, channels, rate, &data)
}

fn float32(channels: u16, rate: u32, samples: &[f32]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    wav(3, 32, channels, rate, &data)
}

/// A mono tone that is `first` Hz for the first half and `second` Hz after: the same shape as the
/// export proof's mix, so "which half did the export take" is a measurement.
fn two_tone(rate: u32, seconds: u32, first: f64, second: f64) -> Vec<u8> {
    let total = rate * seconds;
    let samples: Vec<i16> = (0..total)
        .map(|index| {
            let hz = if index < total / 2 { first } else { second };
            (0.5 * 32767.0 * (2.0 * PI * hz * f64::from(index) / f64::from(rate)).sin()).round()
                as i16
        })
        .collect();
    pcm16(1, rate, &samples)
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Squared magnitude of `hz` in `samples` (Goertzel), normalised by length.
fn power(samples: &[f64], rate: f64, hz: f64) -> f64 {
    let coeff = 2.0 * (2.0 * PI * hz / rate).cos();
    let (mut s1, mut s2) = (0.0, 0.0);
    for sample in samples {
        let s0 = sample + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2) / (samples.len() as f64).powi(2)
}

/// Stereo PCM16 samples of a WAV this crate wrote.
fn read_stereo(path: &Path) -> Vec<i16> {
    let bytes = fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2, "stereo");
    assert_eq!(
        u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
        48_000
    );
    assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 16);
    let declared = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as usize;
    assert_eq!(declared, bytes.len() - 44, "the data size was patched");
    bytes[44..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| i16::from_le_bytes(*pair))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// A project with the corpus scene and a declared mix.
// ---------------------------------------------------------------------------------------------

struct Work(PathBuf);

impl Work {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("te-s9-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The corpus bundle declaring `audio/mix.wav` with `window`; its hash is the file's.
fn bundle_with_mix(mix: &[u8], window: (f64, f64)) -> Value {
    let mut raw: Value =
        serde_json::from_str(&fs::read_to_string(corpus().join("bundle.json")).unwrap()).unwrap();
    raw["audio"] = json!({
        "mixRelativePath": "audio/mix.wav",
        "editStartSeconds": window.0,
        "editEndSecondsExclusive": window.1,
        "sourcePcmHashSha256": sha(mix),
    });
    raw
}

/// A base folder with the corpus images and `audio/mix.wav`.
fn base_with_mix(work: &Work, mix: &[u8]) -> PathBuf {
    let base = work.path("base");
    for entry in ["assets/sky.png", "assets/mid.png", "assets/hero.png"] {
        let target = base.join(entry);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(corpus().join(entry), target).unwrap();
    }
    fs::create_dir_all(base.join("audio")).unwrap();
    fs::write(base.join("audio/mix.wav"), mix).unwrap();
    base
}

fn demo_state(bundle: &Value) -> EditorState {
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    headless::execute(
        &mut state,
        &json!({"op":"scene.attach","id":"street","bundle":bundle}),
    )
    .unwrap();
    state
}

fn snapshot(state: &EditorState, request: Value) -> Result<SceneExportSnapshot, CommandError> {
    let mut request = request;
    request["op"] = json!("scene.export");
    let spec = SceneExportSpec::from_json(state, &request)?;
    SceneExportSnapshot::capture(state, &spec)
}

fn approved(path: &Path, bytes: &[u8]) -> ApprovedResource {
    ApprovedResource {
        scene_id: "street".into(),
        key: ResourceKey::AudioMix,
        kind: ResourceKind::Audio,
        path: path.to_path_buf(),
        content_hash: sha(bytes),
        revision_id: None,
        bytes: bytes.len() as u64,
        generation: Generation(0),
        cache_key: String::new(),
        node_ids: Vec::new(),
    }
}

/// Runs `render_audio` as a job body (it needs a real `JobContext`).
fn render(
    snapshot: &SceneExportSnapshot,
    approved: &ApprovedResource,
    out: &Path,
) -> Result<MixReport, JobError> {
    let registry = JobRegistry::new(1);
    let (sender, receiver) = mpsc::channel();
    let (mix, approved, out) = (
        snapshot.audio().mix().expect("a mix is planned").clone(),
        approved.clone(),
        out.to_path_buf(),
    );
    registry.spawn(JobKind::VideoExport, "audio", move |ctx| {
        let _ = sender.send(render_audio(&mix, &approved, &out, ctx));
        Ok(json!({}))
    });
    receiver.recv_timeout(Duration::from_secs(60)).unwrap()
}

fn tiny() -> Value {
    json!({"id":"tiny","name":"Tiny","width":320,"height":180,"fps":24})
}

// ---------------------------------------------------------------------------------------------
// Exact samples.
// ---------------------------------------------------------------------------------------------

fn ramp_stereo(frames: usize) -> Vec<i16> {
    (0..frames)
        .flat_map(|index| {
            let left = (index % 20_000) as i32 - 10_000;
            [left as i16, (-left) as i16]
        })
        .collect()
}

/// Stereo PCM16 that sweeps the whole 16-bit range, extremes included (`-32768`, `32767`), the
/// left channel up and the right channel down: every value above 16384 is where `/32768` then
/// `*32767` loses a step (`20000` → `19999`).
fn full_scale_stereo(frames: usize) -> Vec<i16> {
    (0..frames)
        .flat_map(|index| {
            let left = -32768 + (index * 65_535 / (frames - 1)) as i32;
            [left as i16, (-1 - left) as i16]
        })
        .collect()
}

#[test]
fn a_48k_pcm16_stereo_source_is_copied_bit_for_bit_across_the_whole_16_bit_range() {
    // BL016.8: the contract is "exact copy", so no value may move, above −6 dBFS least of all.
    let work = Work::new("fullscale");
    let frames = 4 * 48_000;
    let source = full_scale_stereo(frames);
    assert_eq!((source[0], source[1]), (-32768, 32767));
    assert_eq!(
        (source[2 * frames - 2], source[2 * frames - 1]),
        (32767, -32768)
    );
    assert!(source.iter().any(|sample| *sample == 20_000));
    let mix = pcm16(2, 48_000, &source);
    let state = demo_state(&bundle_with_mix(&mix, (0.0, 4.0)));
    let file = work.path("mix.wav");
    fs::write(&file, &mix).unwrap();
    // The whole file, then the same file read from the middle of the window.
    for (from, frames_out, first_frame) in [(0, 96, 0usize), (48, 48, 96_000)] {
        let plan = snapshot(
            &state,
            json!({"scene":"street","preset":tiny(),"from":from,"frames":frames_out}),
        )
        .unwrap();
        let out = work.path(&format!("out-{from}.wav"));
        let report = render(&plan, &approved(&file, &mix), &out).unwrap();
        let samples = read_stereo(&out);
        let wanted = &source[2 * first_frame..2 * first_frame + samples.len()];
        let moved = samples.iter().zip(wanted).filter(|(a, b)| a != b).count();
        assert_eq!(moved, 0, "{moved} of {} samples moved", samples.len());
        assert_eq!(report.clipped, 0);
    }
}

#[test]
fn a_48k_stereo_source_is_copied_sample_for_sample_from_the_window_offset() {
    let work = Work::new("copy");
    let mix = pcm16(2, 48_000, &ramp_stereo(4 * 48_000));
    let state = demo_state(&bundle_with_mix(&mix, (0.0, 4.0)));
    // Frame 24 at 24 fps is second 1: output sample n is source frame 48 000 + n.
    let plan = snapshot(
        &state,
        json!({"scene":"street","preset":tiny(),"from":24,"frames":24}),
    )
    .unwrap();
    let file = work.path("mix.wav");
    fs::write(&file, &mix).unwrap();
    let out = work.path("out.wav");
    let report = render(&plan, &approved(&file, &mix), &out).unwrap();
    let samples = read_stereo(&out);
    assert_eq!(report.samples, 48_000);
    assert_eq!(samples.len(), 2 * 48_000);
    let source = ramp_stereo(4 * 48_000);
    assert_eq!(&samples[..], &source[2 * 48_000..2 * 96_000]);
    assert_eq!(report.clipped, 0);
    assert!(report.layout.facts.sample_rate == 48_000);
}

#[test]
fn a_declared_window_offset_moves_where_the_file_is_read() {
    let work = Work::new("offset");
    // Window 2.0–6.0: edit second 1 is file second 3.
    let mix = pcm16(2, 48_000, &ramp_stereo(8 * 48_000));
    let state = demo_state(&bundle_with_mix(&mix, (2.0, 6.0)));
    let plan = snapshot(
        &state,
        json!({"scene":"street","preset":tiny(),"from":24,"frames":12}),
    )
    .unwrap();
    let file = work.path("mix.wav");
    fs::write(&file, &mix).unwrap();
    let out = work.path("out.wav");
    render(&plan, &approved(&file, &mix), &out).unwrap();
    let samples = read_stereo(&out);
    let source = ramp_stereo(8 * 48_000);
    let first = 3 * 48_000;
    assert_eq!(samples.len(), 2 * 24_000);
    assert_eq!(&samples[..], &source[2 * first..2 * (first + 24_000)]);
}

#[test]
fn a_44k_mono_source_is_interpolated_at_exact_positions_and_centred() {
    let work = Work::new("rate");
    let source: Vec<i16> = (0..4 * 44_100)
        .map(|index| ((index * 7) % 20_000) as i16 - 10_000)
        .collect();
    let mix = pcm16(1, 44_100, &source);
    let state = demo_state(&bundle_with_mix(&mix, (0.0, 4.0)));
    let plan = snapshot(
        &state,
        json!({"scene":"street","preset":tiny(),"from":24,"frames":24}),
    )
    .unwrap();
    let file = work.path("mix.wav");
    fs::write(&file, &mix).unwrap();
    let out = work.path("out.wav");
    let report = render(&plan, &approved(&file, &mix), &out).unwrap();
    let samples = read_stereo(&out);
    assert_eq!(samples.len(), 2 * 48_000);
    let at = |frame: usize| f64::from(source[frame]) / 32768.0;
    for n in [0usize, 1, 2, 47, 1_000, 24_000, 47_999] {
        // Position in source frames: (1 s + n/48000 s) × 44100, linear between its neighbours.
        let position = (48_000 + n) as f64 * 44_100.0 / 48_000.0;
        let (base, fraction) = (position.floor() as usize, position.fract());
        let value = at(base) + (at(base + 1) - at(base)) * fraction;
        let expected = (value * std::f64::consts::FRAC_1_SQRT_2 * 32768.0).round();
        for channel in 0..2 {
            let actual = f64::from(samples[2 * n + channel]);
            assert!(
                (actual - expected).abs() <= 1.0,
                "sample {n} ch {channel}: {actual} vs {expected}"
            );
        }
    }
    assert!(report.layout.facts.sample_rate == 44_100 && report.layout.facts.channels == 1);
}

#[test]
fn the_tail_a_rate_conversion_adds_is_declared_silence() {
    let work = Work::new("tail");
    let mix = pcm16(2, 48_000, &ramp_stereo(4 * 48_000));
    let state = demo_state(&bundle_with_mix(&mix, (0.0, 4.0)));
    // 2 edit frames (4000 samples) become 3 frames at 30 fps (4800 samples).
    let thirty = json!({"id":"t","name":"T","width":320,"height":180,"fps":30});
    let plan = snapshot(
        &state,
        json!({"scene":"street","preset":thirty,"from":0,"frames":2}),
    )
    .unwrap();
    let file = work.path("mix.wav");
    fs::write(&file, &mix).unwrap();
    let out = work.path("out.wav");
    render(&plan, &approved(&file, &mix), &out).unwrap();
    let samples = read_stereo(&out);
    assert_eq!(samples.len(), 2 * 4_800);
    assert_eq!(&samples[..2 * 4_000], &ramp_stereo(4_000)[..]);
    assert!(samples[2 * 4_000..].iter().all(|sample| *sample == 0));
}

#[test]
fn float_sources_are_clamped_and_never_let_a_nan_through() {
    let work = Work::new("float");
    let mut data = vec![0.25f32; 2 * 48_000];
    data[0] = f32::NAN;
    data[1] = 2.5;
    data[2] = -3.0;
    data[3] = f32::INFINITY;
    let mix = float32(2, 48_000, &data);
    let state = demo_state(&bundle_with_mix(&mix, (0.0, 1.0)));
    let plan = snapshot(
        &state,
        json!({"scene":"street","preset":tiny(),"from":0,"frames":24}),
    )
    .unwrap();
    let file = work.path("mix.wav");
    fs::write(&file, &mix).unwrap();
    let out = work.path("out.wav");
    let report = render(&plan, &approved(&file, &mix), &out).unwrap();
    let samples = read_stereo(&out);
    assert_eq!(samples[0], 0, "NaN is silence");
    assert_eq!(samples[1], 32767, "above full scale is clamped");
    assert_eq!(samples[2], -32767);
    assert_eq!(samples[3], 0, "infinity is silence");
    assert_eq!(samples[4], (0.25f32 * 32767.0).round() as i16);
    assert_eq!(report.clipped, 2, "the two finite overs are counted");
}

#[test]
fn bytes_that_changed_after_approval_are_never_mixed() {
    let work = Work::new("swap");
    let mix = pcm16(2, 48_000, &ramp_stereo(2 * 48_000));
    let state = demo_state(&bundle_with_mix(&mix, (0.0, 2.0)));
    let plan = snapshot(
        &state,
        json!({"scene":"street","preset":tiny(),"from":0,"frames":24}),
    )
    .unwrap();
    let file = work.path("mix.wav");
    fs::write(&file, &mix).unwrap();
    let approval = approved(&file, &mix);

    // Same size, one byte different.
    let mut swapped = mix.clone();
    *swapped.last_mut().unwrap() ^= 0x55;
    fs::write(&file, &swapped).unwrap();
    let error = render(&plan, &approval, &work.path("a.wav")).unwrap_err();
    assert_eq!(error.code, "resource_changed", "{}", error.message);

    // A different size.
    fs::write(&file, &mix[..mix.len() - 4]).unwrap();
    let error = render(&plan, &approval, &work.path("b.wav")).unwrap_err();
    assert_eq!(error.code, "resource_changed", "{}", error.message);

    // Gone.
    fs::remove_file(&file).unwrap();
    let error = render(&plan, &approval, &work.path("c.wav")).unwrap_err();
    assert!(error.code == "asset_unreadable", "{}", error.code);
}

#[test]
fn shapes_no_mix_can_use_are_refused_with_a_typed_reason() {
    let work = Work::new("shapes");
    let mix = pcm16(2, 48_000, &ramp_stereo(2 * 48_000));
    let state = demo_state(&bundle_with_mix(&mix, (0.0, 2.0)));
    let plan = snapshot(
        &state,
        json!({"scene":"street","preset":tiny(),"from":0,"frames":24}),
    )
    .unwrap();
    let mix_plan = plan.audio().mix().unwrap();
    let judge = |name: &str, bytes: &[u8]| {
        let path = work.path(name);
        fs::write(&path, bytes).unwrap();
        match check_layout(mix_plan, &path) {
            Ok(_) => "ok".to_string(),
            Err(InspectError::Unsupported(error)) => error.code.to_string(),
            Err(InspectError::Unreadable(error)) => error.code,
        }
    };
    let frames = 2 * 48_000usize;
    assert_eq!(judge("ok.wav", &mix), "ok");
    assert_eq!(
        judge("text.wav", b"definitely not audio at all"),
        "unsupported"
    );
    assert_eq!(
        judge("mp3.wav", b"ID3\x04\x00\x00\x00\x00\x00\x00 not a riff"),
        "unsupported"
    );
    let int24 = wav(1, 24, 2, 48_000, &vec![0u8; frames * 6]);
    assert_eq!(judge("int24.wav", &int24), "unsupported");
    let int8 = wav(1, 8, 1, 48_000, &vec![128u8; frames]);
    assert_eq!(judge("int8.wav", &int8), "unsupported");
    let adpcm = wav(2, 4, 1, 48_000, &vec![0u8; frames]);
    assert_eq!(judge("adpcm.wav", &adpcm), "unsupported");
    let surround = pcm16(6, 48_000, &vec![0i16; frames * 6]);
    assert_eq!(judge("surround.wav", &surround), "unsupported");
    let hi_res = pcm16(2, 96_000, &vec![0i16; frames * 4]);
    assert_eq!(judge("hires.wav", &hi_res), "unsupported");
    let short = pcm16(2, 48_000, &ramp_stereo(24_000));
    assert_eq!(judge("short.wav", &short), "audio_source_short");
    assert_eq!(judge("empty.wav", b""), "unsupported");
    assert_eq!(
        judge_missing(mix_plan, &work.path("missing.wav")),
        "asset_unreadable"
    );
}

fn judge_missing(mix: &tessel_editor_core::scene_export::DirectedAudioMix, path: &Path) -> String {
    match check_layout(mix, path) {
        Err(InspectError::Unreadable(error)) => error.code,
        other => format!("unexpected {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Through the host: scene.export with a stand-in frame source.
// ---------------------------------------------------------------------------------------------

/// What the stand-in frame source does besides drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    None,
    /// BL109: rewrite the approved original mix, same size and other bytes, once the first frame
    /// is asked for (the mix was already read by then; the picture is still being drawn).
    TamperMix,
    /// BL109/BL016.9: rewrite the original the moment the job's private copy exists, *before* the
    /// mix reads anything. Only a mix drawn from the copy survives it.
    TamperAfterCopy,
    /// Stop the job right after the copy (before the mix) until the test releases it, so a
    /// cancel lands exactly at the start of the `audio` step.
    ParkAfterCopy,
    /// Stop the job after the first chunk of the private copy was written, until the test
    /// releases it: the cancel lands in the middle of the copy, whatever the disk's speed.
    ParkCopyChunk,
    /// Stop the job once the mix has written this many one-second steps, until the test
    /// releases it: the cancel lands between two known steps of the mix.
    ParkMixStep(u64),
    /// BL016.9: replace the mix with `swap` when the job first talks to the renderer, which is
    /// after the request was accepted (`freeze` read the header) and before the job's preflight.
    SwapAtStart,
    /// Slow frames, so a test can cancel the job while it renders.
    Slow,
}

/// The two ends of a park: the job says it arrived, the test says go.
struct Gate {
    parked: mpsc::Sender<()>,
    release: Arc<Mutex<mpsc::Receiver<()>>>,
}

struct StillRenderer {
    png: PathBuf,
    drawn: Arc<AtomicU64>,
    fault: Fault,
    victim: PathBuf,
    gate: Arc<Gate>,
    swap: Arc<Mutex<Option<Vec<u8>>>>,
    /// `ParkCopyChunk` parks at the first chunk only.
    parked_in_copy: bool,
}

impl StillRenderer {
    /// Tells the test the job is here and holds it until the test releases it.
    fn park(&self) {
        let _ = self.gate.parked.send(());
        self.gate
            .release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(60))
            .expect("the test releases the parked job");
    }
}

impl SceneFrameRenderer for StillRenderer {
    fn max_size(&mut self) -> Result<Option<u32>, JobError> {
        if self.fault == Fault::SwapAtStart
            && let Some(bytes) = self.swap.lock().unwrap().take()
        {
            fs::write(&self.victim, bytes).unwrap();
        }
        Ok(None)
    }

    fn resources_frozen(&mut self) -> Result<(), JobError> {
        match self.fault {
            Fault::TamperAfterCopy => same_size_garbage(&self.victim),
            Fault::ParkAfterCopy => self.park(),
            _ => {}
        }
        Ok(())
    }

    fn copy_progressed(&mut self, _bytes: u64) {
        if self.fault == Fault::ParkCopyChunk && !self.parked_in_copy {
            self.parked_in_copy = true;
            self.park();
        }
    }

    fn mix_stepped(&mut self, done: u64, _steps: u64) -> Result<(), JobError> {
        if self.fault == Fault::ParkMixStep(done) {
            self.park();
        }
        Ok(())
    }

    fn render(&mut self, job: &FrameJob) -> Result<RenderedFrame, JobError> {
        let index = self.drawn.fetch_add(1, Ordering::SeqCst);
        match self.fault {
            Fault::TamperMix if index == 0 => same_size_garbage(&self.victim),
            Fault::Slow => std::thread::sleep(Duration::from_millis(25)),
            _ => {}
        }
        fs::copy(&self.png, job.png)
            .map_err(|error| JobError::new("render_failed", error.to_string()))?;
        Ok(RenderedFrame {
            hash: format!("still-{index}"),
            render_ms: 1.0,
        })
    }
}

struct Lab {
    state: EditorState,
    host: MediaHost,
    work: Work,
    base: PathBuf,
    drawn: Arc<AtomicU64>,
    tools: Tools,
    /// Arrives when a `ParkAfterCopy` job is parked; `release` lets it go on.
    parked: mpsc::Receiver<()>,
    release: mpsc::Sender<()>,
    /// What a `SwapAtStart` job writes over the mix when it starts.
    swap: Arc<Mutex<Option<Vec<u8>>>>,
}

impl Lab {
    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        headless::execute_ext(&mut self.state, None, Some(&mut self.host), &request)
    }

    fn ok(&mut self, request: Value) -> Value {
        self.run(request.clone())
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    fn export(
        &mut self,
        name: &str,
        preset: Value,
        from: i64,
        frames: i64,
    ) -> Result<Value, CommandError> {
        let destination = self.work.path(name);
        self.run(json!({"op":"scene.export","scene":"street","preset":preset,
            "from":from,"frames":frames,"destination":destination,"wait":true}))
    }

    fn leftovers(&self) -> Vec<String> {
        fs::read_dir(&self.work.0)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".tessel-"))
            .collect()
    }

    /// The first audio stream of `path` decoded to mono 48 kHz s16, as floats.
    fn decoded_audio(&self, path: &Path) -> Vec<f64> {
        let output = Command::new(&self.tools.ffmpeg)
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(path)
            .args([
                "-map", "0:a:0", "-f", "s16le", "-ac", "1", "-ar", "48000", "-",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
            .stdout
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| f64::from(i16::from_le_bytes(*pair)) / 32768.0)
            .collect()
    }

    fn probe(&self, path: &Path) -> Value {
        let output = Command::new(&self.tools.ffprobe)
            .args(["-v", "error", "-show_streams", "-of", "json"])
            .arg(path)
            .output()
            .unwrap();
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

fn same_size_garbage(path: &Path) {
    let size = fs::metadata(path).unwrap().len();
    fs::write(path, vec![0x55; size as usize]).unwrap();
}

fn lab(name: &str, mix: &[u8], window: (f64, f64)) -> Option<Lab> {
    lab_with(name, mix, window, Fault::None)
}

fn lab_with(name: &str, mix: &[u8], window: (f64, f64), fault: Fault) -> Option<Lab> {
    let tools = match Tools::discover() {
        Ok(tools) => tools,
        Err(error) => {
            assert!(
                std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
                "FFmpeg is required: {}",
                error.message
            );
            eprintln!("SKIPPED (FFmpeg unavailable): {}", error.message);
            return None;
        }
    };
    if !tools.has_encoder("aac") {
        eprintln!("SKIPPED (no aac encoder)");
        return None;
    }
    let work = Work::new(name);
    let frame = work.path("frame.png");
    let made = Command::new(&tools.ffmpeg)
        .args([
            "-v",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x180:rate=1",
        ])
        .args(["-frames:v", "1"])
        .arg(&frame)
        .status()
        .unwrap();
    assert!(made.success());
    let base = base_with_mix(&work, mix);
    let drawn = Arc::new(AtomicU64::new(0));
    let counter = drawn.clone();
    let victim = base.join("audio/mix.wav");
    let (parked, parked_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let gate = Arc::new(Gate {
        parked,
        release: Arc::new(Mutex::new(release_rx)),
    });
    let swap: Arc<Mutex<Option<Vec<u8>>>> = Arc::default();
    let swap_for_renderer = swap.clone();
    let factory: Arc<SceneRendererFactory> = Arc::new(move || {
        Ok(Box::new(StillRenderer {
            png: frame.clone(),
            drawn: counter.clone(),
            fault,
            victim: victim.clone(),
            gate: gate.clone(),
            swap: swap_for_renderer.clone(),
            parked_in_copy: false,
        }))
    });
    let mut lab = Lab {
        state: EditorState::empty(&ProjectSettings::default()).unwrap(),
        host: MediaHost::new().with_scene_renderer(factory),
        work,
        base,
        drawn,
        tools,
        parked: parked_rx,
        release,
        swap,
    };
    lab.ok(json!({"op":"scene.attach","id":"street","bundle":bundle_with_mix(mix, window)}));
    let base = lab.base.clone();
    lab.ok(json!({"op":"scene.resources.resolve","scene":"street","base":base}));
    Some(lab)
}

fn assert_tone(samples: &[f64], from_seconds: f64, to_seconds: f64, hz: f64, other: f64) {
    let slice = &samples[(from_seconds * 48_000.0) as usize..(to_seconds * 48_000.0) as usize];
    let (wanted, unwanted) = (power(slice, 48_000.0, hz), power(slice, 48_000.0, other));
    assert!(
        wanted > 50.0 * unwanted && wanted > 1e-3,
        "{from_seconds}-{to_seconds} s: {hz} Hz {wanted:e} vs {other} Hz {unwanted:e}"
    );
}

#[test]
fn the_declared_mix_is_in_the_file_for_the_requested_range() {
    let mix = two_tone(48_000, 4, 440.0, 880.0);
    let Some(mut lab) = lab("e2e", &mix, (0.0, 4.0)) else {
        return;
    };
    // Frames 36..60 are seconds 1.5–2.5 of the scene; the mix switches from 440 to 880 Hz at 2.0 s.
    let result = lab.export("mixed.mp4", tiny(), 36, 24).unwrap();
    assert_eq!(result["verification"]["ok"], true, "{result}");
    assert_eq!(result["audio"]["included"], true);
    assert_eq!(result["audio"]["declared"], json!(["audio"]));
    assert_eq!(result["audio"]["mix"]["samples"], 48_000);
    assert_eq!(result["audio"]["source"]["sha256"], sha(&mix));
    assert_eq!(result["audio"]["source"]["converted_rate"], false);
    assert!(
        result["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|warning| warning["code"] != "audio_omitted")
    );

    let destination = lab.work.path("mixed.mp4");
    let probe = lab.probe(&destination);
    let audio = probe["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stream| stream["codec_type"] == "audio")
        .expect("an audio stream");
    assert_eq!(audio["codec_name"], "aac");
    assert_eq!(audio["sample_rate"], "48000");
    assert_eq!(audio["channels"], 2);

    let samples = lab.decoded_audio(&destination);
    assert!(
        (samples.len() as i64 - 48_000).abs() <= 2_048,
        "{} samples decoded",
        samples.len()
    );
    // Half a second of 440 Hz, then half a second of 880 Hz, measured away from the switch.
    assert_tone(&samples, 0.05, 0.40, 440.0, 880.0);
    assert_tone(&samples, 0.60, 0.95, 880.0, 440.0);
    assert_eq!(lab.drawn.load(Ordering::SeqCst), 24);
    assert!(lab.leftovers().is_empty(), "{:?}", lab.leftovers());
}

#[test]
fn a_frame_rate_conversion_and_a_44k_source_land_on_the_same_instants() {
    // 44.1 kHz source, 24 → 30 fps: 24 edit frames (1 s) become 30 frames; the sound must still be
    // the 440→880 switch half way through the second, and as long as the picture.
    let mix = two_tone(44_100, 4, 440.0, 880.0);
    let Some(mut lab) = lab("e2e-rate", &mix, (0.0, 4.0)) else {
        return;
    };
    let thirty = json!({"id":"t","name":"T","width":320,"height":180,"fps":30});
    let result = lab.export("rate.mp4", thirty, 36, 24).unwrap();
    assert_eq!(result["frames"], 30);
    assert_eq!(result["verification"]["ok"], true, "{result}");
    assert_eq!(result["audio"]["source"]["converted_rate"], true);
    assert_eq!(result["audio"]["source"]["sample_rate"], 44_100);
    assert_eq!(result["audio"]["mix"]["samples"], 48_000);

    let destination = lab.work.path("rate.mp4");
    let samples = lab.decoded_audio(&destination);
    assert!(
        (samples.len() as i64 - 48_000).abs() <= 2_048,
        "{}",
        samples.len()
    );
    assert_tone(&samples, 0.05, 0.40, 440.0, 880.0);
    assert_tone(&samples, 0.60, 0.95, 880.0, 440.0);
    let probe = lab.probe(&destination);
    let seconds = |kind: &str| -> f64 {
        probe["streams"]
            .as_array()
            .unwrap()
            .iter()
            .find(|stream| stream["codec_type"] == kind)
            .and_then(|stream| stream["duration"].as_str()?.parse().ok())
            .unwrap()
    };
    assert!((seconds("video") - seconds("audio")).abs() < 0.06);
}

#[test]
fn a_window_offset_selects_the_part_of_the_file_that_plays() {
    // The mix is 8 s: 440 Hz for 4 s then 880 Hz. The window starts at 3.5 s, so edit second 0 is file
    // second 3.5 and the switch (file second 4) lands at edit second 0.5.
    let mix = two_tone(48_000, 8, 440.0, 880.0);
    let Some(mut lab) = lab("e2e-offset", &mix, (3.5, 7.5)) else {
        return;
    };
    let result = lab.export("offset.mp4", tiny(), 0, 24).unwrap();
    assert_eq!(result["verification"]["ok"], true, "{result}");
    let samples = lab.decoded_audio(&lab.work.path("offset.mp4"));
    assert_tone(&samples, 0.05, 0.40, 440.0, 880.0);
    assert_tone(&samples, 0.60, 0.95, 880.0, 440.0);
}

#[test]
fn audio_false_exports_the_picture_only_and_says_what_was_left_out() {
    let mix = two_tone(48_000, 4, 440.0, 880.0);
    let Some(mut lab) = lab("e2e-silent", &mix, (0.0, 4.0)) else {
        return;
    };
    let destination = lab.work.path("video-only.mp4");
    let result = lab.ok(json!({"op":"scene.export","scene":"street","preset":tiny(),
        "from":0,"frames":8,"audio":false,"destination":destination,"wait":true}));
    assert_eq!(result["audio"]["included"], false);
    assert_eq!(result["audio"]["declared"], json!(["audio"]));
    assert!(
        result["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["code"] == "audio_omitted")
    );
    assert!(
        lab.probe(&destination)["streams"]
            .as_array()
            .unwrap()
            .iter()
            .all(|stream| stream["codec_type"] != "audio")
    );
}

#[test]
fn a_mix_no_export_can_use_is_refused_before_any_job_or_frame() {
    // 24-bit, outside the window, and too short are all refused by the request itself.
    let int24 = wav(1, 24, 2, 48_000, &vec![0u8; 4 * 48_000 * 6]);
    let Some(mut lab) = lab("e2e-refuse", &int24, (0.0, 4.0)) else {
        return;
    };
    let jobs = lab.ok(json!({"op":"job.list"}));
    let error = lab.export("a.mp4", tiny(), 0, 8).unwrap_err();
    assert_eq!(error.code, "unsupported", "{}", error.message);
    assert!(error.message.contains("24 bits"), "{}", error.message);
    assert_eq!(lab.ok(json!({"op":"job.list"})), jobs);
    assert_eq!(lab.drawn.load(Ordering::SeqCst), 0);
    assert!(!lab.work.path("a.mp4").exists());
    assert!(lab.leftovers().is_empty());

    let preflight = lab
        .run(
            json!({"op":"scene.export.preflight","scene":"street","preset":tiny(),
            "from":0,"frames":8,"destination":lab.work.path("b.mp4")}),
        )
        .unwrap_err();
    assert_eq!(preflight.code, "unsupported");

    // `audio:false` still exports: the unusable mix is simply not used.
    let destination = lab.work.path("c.mp4");
    let result = lab.ok(json!({"op":"scene.export","scene":"street","preset":tiny(),
        "from":0,"frames":8,"audio":false,"destination":destination,"wait":true}));
    assert_eq!(result["audio"]["included"], false);
}

#[test]
fn a_source_shorter_than_its_window_is_refused_for_every_range_the_origin_would_refuse() {
    // BL016.7, the origin's rule (`node/render.ts`, `AUDIO_SHORT`): the file must cover the whole
    // declared window, `duration + 1 ms >= editEndSecondsExclusive`, whatever range is exported.
    // The file holds 2 s and the bundle declares a 4 s window: no range of it exports, not even
    // the first second, which the file does cover.
    let mix = two_tone(48_000, 2, 440.0, 880.0);
    let Some(mut lab) = lab("e2e-short", &mix, (0.0, 4.0)) else {
        return;
    };
    for (name, from, frames) in [("first.mp4", 0, 24), ("late.mp4", 40, 24)] {
        let error = lab.export(name, tiny(), from, frames).unwrap_err();
        assert_eq!(error.code, "audio_source_short", "{}", error.message);
        assert_eq!(error.details.as_ref().unwrap()["rule"], "window");
        assert!(!lab.work.path(name).exists());
    }
    assert!(lab.leftovers().is_empty());
    assert_eq!(lab.drawn.load(Ordering::SeqCst), 0, "no frame was drawn");
    // The same refusal from the preflight operation, and `audio:false` never needed the file.
    let preflight = lab
        .run(
            json!({"op":"scene.export.preflight","scene":"street","preset":tiny(),
            "from":0,"frames":8,"destination":lab.work.path("p.mp4")}),
        )
        .unwrap_err();
    assert_eq!(preflight.code, "audio_source_short");
    let destination = lab.work.path("silent.mp4");
    lab.ok(
        json!({"op":"scene.export","scene":"street","preset":tiny(),"from":0,"frames":8,
        "audio":false,"destination":destination,"wait":true}),
    );
}

#[test]
fn the_window_the_file_covers_exports_and_a_range_past_it_is_refused_by_the_window() {
    // A file that is exactly its 2 s window exports its range, and a range that leaves the window
    // is `audio_window_short`, not a half-mixed file.
    let mix = two_tone(48_000, 2, 440.0, 880.0);
    let Some(mut lab) = lab("e2e-window", &mix, (0.0, 2.0)) else {
        return;
    };
    lab.export("fits.mp4", tiny(), 0, 24).unwrap();
    let error = lab.export("late.mp4", tiny(), 40, 24).unwrap_err();
    assert_eq!(error.code, "audio_window_short", "{}", error.message);
    assert!(!lab.work.path("late.mp4").exists());
    assert!(lab.leftovers().is_empty());
    assert_eq!(
        lab.drawn.load(Ordering::SeqCst),
        24,
        "only the accepted export drew frames"
    );
}

#[test]
fn an_offset_window_longer_than_the_file_is_refused_like_the_origin_does() {
    // The shape the S10 corpus has: 8 s of file and a window [1, 9). The origin refuses it
    // (`8 + 0.001 < 9`), however small the range, and so does this export.
    let mix = two_tone(48_000, 8, 440.0, 880.0);
    let Some(mut lab) = lab("e2e-offset-short", &mix, (1.0, 9.0)) else {
        return;
    };
    let error = lab.export("a.mp4", tiny(), 0, 24).unwrap_err();
    assert_eq!(error.code, "audio_source_short", "{}", error.message);
    assert_eq!(lab.drawn.load(Ordering::SeqCst), 0);
    assert!(!lab.work.path("a.mp4").exists());
}

#[test]
fn bytes_swapped_after_approval_fail_the_job_before_a_frame_is_drawn() {
    let mix = two_tone(48_000, 4, 440.0, 880.0);
    let Some(mut lab) = lab("e2e-swap", &mix, (0.0, 4.0)) else {
        return;
    };
    // Same size, different content: the approval's stat check passes, the identity check does not.
    let mut other = mix.clone();
    let last = other.len() - 1;
    other[last] ^= 0x7f;
    fs::write(lab.base.join("audio/mix.wav"), &other).unwrap();
    let error = lab.export("swapped.mp4", tiny(), 0, 24).unwrap_err();
    // The code is typed all the way out (never `job_failed`), with both identities named.
    assert_eq!(error.code, "resource_changed", "{}", error.message);
    let details = error.details.as_ref().expect("both identities are named");
    assert_eq!(details["resource"], "audio-mix");
    assert_ne!(details["got"]["sha256"], details["expected"]["sha256"]);
    assert_eq!(details["got"]["bytes"], details["expected"]["bytes"]);
    assert_eq!(
        lab.drawn.load(Ordering::SeqCst),
        0,
        "failed before any frame"
    );
    assert!(!lab.work.path("swapped.mp4").exists());
    assert!(lab.leftovers().is_empty(), "{:?}", lab.leftovers());
}

#[test]
fn preflight_names_a_vanished_mix_and_a_clear_one() {
    let mix = two_tone(48_000, 4, 440.0, 880.0);
    let Some(mut lab) = lab("e2e-preflight", &mix, (0.0, 4.0)) else {
        return;
    };
    let ask = |lab: &mut Lab, name: &str| {
        let destination = lab.work.path(name);
        lab.ok(
            json!({"op":"scene.export.preflight","scene":"street","preset":tiny(),
            "from":0,"frames":24,"destination":destination}),
        )
    };
    let clear = ask(&mut lab, "p.mp4");
    assert_eq!(clear["ok"], true, "{clear}");
    assert_eq!(clear["summary"]["audio"]["included"], true);
    assert_eq!(clear["summary"]["audio"]["mix"]["samples"], 48_000);

    fs::remove_file(lab.base.join("audio/mix.wav")).unwrap();
    let blocked = ask(&mut lab, "q.mp4");
    assert_eq!(blocked["ok"], false);
    let blocker = &blocked["blockers"][0];
    assert_eq!(blocker["code"], "resource_missing", "{blocked}");
    assert_eq!(blocker["asset"], "audio-mix");
    assert!(!lab.work.path("q.mp4").exists());
    assert_eq!(lab.drawn.load(Ordering::SeqCst), 0);
}

/// A finished or failed export leaves no staging directory of its own beside the destination.
fn assert_clean(lab: &Lab) {
    assert!(lab.leftovers().is_empty(), "{:?}", lab.leftovers());
}

#[test]
fn the_private_copy_of_the_mix_is_what_the_export_mixes_even_when_the_original_is_rewritten_mid_job()
 {
    let mix = two_tone(48_000, 4, 440.0, 880.0);
    let Some(mut lab) = lab_with("e2e-during", &mix, (0.0, 4.0), Fault::TamperMix) else {
        return;
    };
    let original = lab.base.join("audio/mix.wav");
    let result = lab.export("during.mp4", tiny(), 36, 24).unwrap();
    assert_eq!(result["verification"]["ok"], true, "{result}");
    assert_ne!(
        fs::read(&original).unwrap(),
        mix,
        "the fault really rewrote the original while the job ran"
    );
    // The file the sound came from is the approved one, and the job's private copy holds it.
    assert_eq!(result["audio"]["source"]["sha256"], sha(&mix));
    let snapshot = &result["snapshot"];
    assert_eq!(snapshot["verified"], true);
    assert!(
        snapshot["bytes"].as_u64().unwrap() >= mix.len() as u64,
        "the copy counts the mix: {snapshot}"
    );
    let samples = lab.decoded_audio(&lab.work.path("during.mp4"));
    assert_tone(&samples, 0.05, 0.40, 440.0, 880.0);
    assert_tone(&samples, 0.60, 0.95, 880.0, 440.0);
    assert_clean(&lab);
}

#[test]
fn rendering_reads_only_the_snapshot_and_a_swap_after_the_copy_cannot_reach_a_sample() {
    // The media-level proof that the mix never re-reads the original: take the copy, rewrite the
    // original (same size), mix from the copy, and compare with the samples of the approved bytes.
    let work = Work::new("private");
    let mix = pcm16(2, 48_000, &ramp_stereo(4 * 48_000));
    let state = demo_state(&bundle_with_mix(&mix, (0.0, 4.0)));
    let plan = snapshot(
        &state,
        json!({"scene":"street","preset":tiny(),"from":24,"frames":24}),
    )
    .unwrap();
    let original = work.path("mix.wav");
    fs::write(&original, &mix).unwrap();
    let approved = approved(&original, &mix);
    let resolver = OnlyMix(approved.clone());

    assert_eq!(
        snapshot_bytes(&resolver, "street", &[ResourceKey::AudioMix]).unwrap(),
        mix.len() as u64
    );
    let frozen = ResourceSnapshot::take(
        &resolver,
        "street",
        &[ResourceKey::AudioMix],
        &work.path("resources"),
        &CancelToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!((frozen.files(), frozen.bytes()), (1, mix.len() as u64));
    let private = frozen.resolve("street", &ResourceKey::AudioMix).unwrap();
    assert_ne!(private.path, original);
    assert!(private.path.starts_with(work.path("resources")));
    assert_eq!(private.content_hash, approved.content_hash);

    same_size_garbage(&original);
    let out = work.path("out.wav");
    let mix_plan = plan.audio().mix().expect("a mix is planned").clone();
    let from_copy = run_job(move |ctx| render_audio_private(&mix_plan, &private, &out, ctx));
    from_copy.unwrap();
    let samples = read_stereo(&work.path("out.wav"));
    let source = ramp_stereo(4 * 48_000);
    assert_eq!(&samples[..], &source[2 * 48_000..2 * 96_000]);
    frozen.verify_intact().unwrap();

    // The identity-checking entry point still refuses the original it was asked to read.
    let error = render(&plan, &approved, &work.path("again.wav")).unwrap_err();
    assert_eq!(error.code, "resource_changed", "{}", error.message);
}

struct OnlyMix(ApprovedResource);

impl ResourceResolver for OnlyMix {
    fn resolve(&self, _: &str, key: &ResourceKey) -> Result<ApprovedResource, ResolveError> {
        if key == &ResourceKey::AudioMix {
            Ok(self.0.clone())
        } else {
            Err(ResolveError::UnknownAsset {
                scene: "street".into(),
                resource: key.label(),
            })
        }
    }
}

fn run_job<T: Send + 'static>(
    body: impl FnOnce(&tessel_editor_core::jobs::JobContext) -> T + Send + 'static,
) -> T {
    let registry = JobRegistry::new(1);
    let (sender, receiver) = mpsc::channel();
    registry.spawn(JobKind::VideoExport, "audio", move |ctx| {
        let _ = sender.send(body(ctx));
        Ok(json!({}))
    });
    receiver.recv_timeout(Duration::from_secs(60)).unwrap()
}

#[test]
fn a_vanished_mix_fails_typed_before_any_frame_and_only_its_own_staging_goes() {
    let mix = two_tone(48_000, 4, 440.0, 880.0);
    let Some(mut lab) = lab("e2e-missing", &mix, (0.0, 4.0)) else {
        return;
    };
    // Another export's staging that happens to sit next to the destination is not ours to clean.
    let foreign = lab.work.path(".tessel-staging-scene-export-foreign-0");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("keep"), b"theirs").unwrap();
    // Removed between approval and the job: the preflight of the request is the one to say so.
    fs::remove_file(lab.base.join("audio/mix.wav")).unwrap();
    let error = lab.export("gone.mp4", tiny(), 0, 24).unwrap_err();
    assert_eq!(error.code, "preflight_failed", "{}", error.message);
    let blockers = &error.details.as_ref().expect("the report")["blockers"];
    assert_eq!(blockers[0]["code"], "resource_missing", "{blockers}");
    assert_eq!(blockers[0]["asset"], "audio-mix");
    assert_eq!(lab.drawn.load(Ordering::SeqCst), 0);
    assert!(!lab.work.path("gone.mp4").exists());
    assert_eq!(lab.leftovers(), [".tessel-staging-scene-export-foreign-0"]);
    assert_eq!(fs::read(foreign.join("keep")).unwrap(), b"theirs");
}

#[test]
fn a_cancelled_export_with_audio_removes_its_own_staging_and_nothing_else() {
    let mix = two_tone(48_000, 4, 440.0, 880.0);
    let Some(mut lab) = lab_with("e2e-cancel", &mix, (0.0, 4.0), Fault::Slow) else {
        return;
    };
    let foreign = lab.work.path(".tessel-staging-scene-export-foreign-1");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("keep"), b"theirs").unwrap();
    let destination = lab.work.path("cancel.mp4");
    let started = lab.ok(json!({"op":"scene.export","scene":"street","preset":tiny(),
        "from":0,"frames":96,"destination":destination,"wait":false}));
    let job = started["job"].as_str().unwrap().to_string();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let info = lab.ok(json!({"op":"job.get","job":job}));
        if info["stage"] == "render" && info["done"].as_u64().unwrap_or(0) >= 2 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "never rendered: {info}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    lab.ok(json!({"op":"job.cancel","job":job}));
    let info = lab.ok(json!({"op":"job.wait","job":job,"timeout_ms":60000}));
    assert_eq!(info["state"], "cancelled", "{info}");
    assert!(!destination.exists());
    assert_eq!(lab.leftovers(), [".tessel-staging-scene-export-foreign-1"]);
    assert_eq!(fs::read(foreign.join("keep")).unwrap(), b"theirs");
    assert!(
        lab.base.join("audio/mix.wav").is_file(),
        "the original is never removed"
    );
}

#[test]
fn preflight_counts_the_mix_and_its_private_copy_without_touching_the_destination() {
    let mix = two_tone(48_000, 4, 440.0, 880.0);
    let Some(mut lab) = lab("e2e-estimate", &mix, (0.0, 4.0)) else {
        return;
    };
    let ask = |lab: &mut Lab, name: &str, audio: bool| {
        let destination = lab.work.path(name);
        lab.ok(
            json!({"op":"scene.export.preflight","scene":"street","preset":tiny(),
            "from":0,"frames":24,"audio":audio,"destination":destination}),
        )
    };
    let with = ask(&mut lab, "with.mp4", true);
    let without = ask(&mut lab, "without.mp4", false);
    let (with, without) = (&with["estimate"], &without["estimate"]);
    assert_eq!(
        with["resource_copy_bytes"].as_u64().unwrap()
            - without["resource_copy_bytes"].as_u64().unwrap(),
        mix.len() as u64,
        "the copy of the mix is part of the copy estimate"
    );
    // One second of 48 kHz stereo s16 for the mixed stream, plus the AAC encoding.
    assert!(with["audio_mix_bytes"].as_u64().unwrap() >= 48_000 * 4);
    assert_eq!(without["audio_mix_bytes"], 0);
    assert!(
        with["staging_bytes"].as_u64().unwrap()
            >= without["staging_bytes"].as_u64().unwrap()
                + mix.len() as u64
                + with["audio_mix_bytes"].as_u64().unwrap()
    );
    assert!(with["output_bytes"].as_u64().unwrap() > without["output_bytes"].as_u64().unwrap());
    assert_eq!(lab.drawn.load(Ordering::SeqCst), 0);
    assert!(!lab.work.path("with.mp4").exists());
    assert_clean(&lab);
}

// ---------------------------------------------------------------------------------------------
// BL016.9: the original rewritten *between* the copy and the mix; cancellation around the copy;
// a header that changed between the request and the job.
// ---------------------------------------------------------------------------------------------

#[test]
fn rewriting_the_original_between_the_copy_and_the_mix_cannot_reach_a_sample() {
    // The sound comes from the job's private copy. The original is rewritten (same size, other bytes)
    // after the copy exists and before the mix reads a single sample: a mix that read the original
    // again would carry the garbage, and the tones below would not be where they must be.
    let mix = two_tone(48_000, 4, 440.0, 880.0);
    let Some(mut lab) = lab_with("e2e-after-copy", &mix, (0.0, 4.0), Fault::TamperAfterCopy) else {
        return;
    };
    let original = lab.base.join("audio/mix.wav");
    let result = lab.export("after-copy.mp4", tiny(), 36, 24).unwrap();
    assert_eq!(result["verification"]["ok"], true, "{result}");
    assert_ne!(
        fs::read(&original).unwrap(),
        mix,
        "the original really was rewritten before the mix"
    );
    assert_eq!(result["audio"]["source"]["sha256"], sha(&mix));
    assert_eq!(result["audio"]["measured"]["clipped"], 0);
    let samples = lab.decoded_audio(&lab.work.path("after-copy.mp4"));
    assert_tone(&samples, 0.05, 0.40, 440.0, 880.0);
    assert_tone(&samples, 0.60, 0.95, 880.0, 440.0);
    assert_clean(&lab);
}

/// The destination folder holds the other export's staging and the original mix; a cancelled job
/// must leave exactly those.
fn assert_only_the_foreign_staging_remains(lab: &Lab, foreign: &Path, mix: &[u8], name: &str) {
    assert!(!lab.work.path(name).exists(), "nothing was published");
    assert_eq!(
        lab.leftovers(),
        [foreign.file_name().unwrap().to_string_lossy().into_owned()]
    );
    assert_eq!(fs::read(foreign.join("keep")).unwrap(), b"theirs");
    assert_eq!(fs::read(lab.base.join("audio/mix.wav")).unwrap(), mix);
    assert_eq!(lab.drawn.load(Ordering::SeqCst), 0, "no frame was drawn");
}

/// Starts a scene export that parks (see `Fault`), waits until it is parked and returns the job.
/// While parked the job is at a known point, so what `job.get` says is a fact, not a race.
fn start_parked(lab: &mut Lab, name: &str, frames: u32) -> (String, PathBuf) {
    let destination = lab.work.path(name);
    let started = lab.ok(json!({"op":"scene.export","scene":"street","preset":tiny(),
        "from":0,"frames":frames,"destination":destination,"wait":false}));
    let job = started["job"].as_str().unwrap().to_string();
    lab.parked
        .recv_timeout(Duration::from_secs(60))
        .expect("the job reached the point it parks at");
    (job, destination)
}

/// Cancels the parked job, lets it go on and returns what the job ended as.
fn cancel_parked(lab: &mut Lab, job: &str) -> Value {
    lab.ok(json!({"op":"job.cancel","job":job}));
    lab.release.send(()).unwrap();
    let info = lab.ok(json!({"op":"job.wait","job":job,"timeout_ms":60000}));
    assert_eq!(info["state"], "cancelled", "{info}");
    info
}

#[test]
fn a_cancel_in_the_middle_of_the_copy_removes_only_its_own_staging() {
    // BL016.9. 12 s of stereo is more than one 1 MiB copy chunk, and the job is parked after the
    // first chunk is written: the copy is provably unfinished, with no dependence on timing.
    let mix = pcm16(2, 48_000, &ramp_stereo(12 * 48_000));
    assert!(mix.len() > 2 << 20);
    let Some(mut lab) = lab_with("e2e-cancel-copy", &mix, (0.0, 4.0), Fault::ParkCopyChunk) else {
        return;
    };
    let foreign = lab.work.path(".tessel-staging-scene-export-foreign-2");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("keep"), b"theirs").unwrap();
    let (job, destination) = start_parked(&mut lab, "cancel-copy.mp4", 24);
    let parked = lab.ok(json!({"op":"job.get","job":job}));
    assert_eq!(parked["stage"], "snapshot", "{parked}");
    let (done, total) = (parked["done"].as_u64().unwrap(), parked["total"].as_u64());
    assert!(done > 0 && Some(done) < total, "a partial copy: {parked}");
    let own: Vec<String> = lab
        .leftovers()
        .into_iter()
        .filter(|name| !name.contains("foreign"))
        .collect();
    assert_eq!(own.len(), 1, "the job's own staging exists: {own:?}");
    let info = cancel_parked(&mut lab, &job);
    assert_eq!(info["stage"], "snapshot", "it never left the copy: {info}");
    assert!(!destination.exists());
    assert_only_the_foreign_staging_remains(&lab, &foreign, &mix, "cancel-copy.mp4");
}

#[test]
fn a_cancel_right_after_the_copy_and_before_the_mix_removes_only_its_own_staging() {
    // The job is parked once its private copy exists: staging holds the copy, no sample was read.
    let mix = two_tone(48_000, 4, 440.0, 880.0);
    let Some(mut lab) = lab_with("e2e-cancel-audio", &mix, (0.0, 4.0), Fault::ParkAfterCopy) else {
        return;
    };
    let foreign = lab.work.path(".tessel-staging-scene-export-foreign-3");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("keep"), b"theirs").unwrap();
    let (job, _) = start_parked(&mut lab, "cancel-audio.mp4", 24);
    let own: Vec<String> = lab
        .leftovers()
        .into_iter()
        .filter(|name| !name.contains("foreign"))
        .collect();
    assert_eq!(
        own.len(),
        1,
        "the job's own staging exists while parked: {own:?}"
    );
    cancel_parked(&mut lab, &job);
    assert_only_the_foreign_staging_remains(&lab, &foreign, &mix, "cancel-audio.mp4");
}

#[test]
fn a_cancel_in_the_middle_of_the_mix_removes_only_its_own_staging() {
    // BL016.9. 4 s of output are four one-second steps; the job is parked after the second is
    // written (and counted), so half of the mix is on disk when the cancel arrives.
    let mix = two_tone(48_000, 4, 440.0, 880.0);
    let Some(mut lab) = lab_with("e2e-cancel-mix", &mix, (0.0, 4.0), Fault::ParkMixStep(2)) else {
        return;
    };
    let foreign = lab.work.path(".tessel-staging-scene-export-foreign-4");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("keep"), b"theirs").unwrap();
    let (job, destination) = start_parked(&mut lab, "cancel-mix.mp4", 96);
    let parked = lab.ok(json!({"op":"job.get","job":job}));
    assert_eq!(parked["stage"], "audio", "{parked}");
    assert_eq!(
        (parked["done"].as_u64(), parked["total"].as_u64()),
        (Some(2), Some(4))
    );
    let own: Vec<String> = lab
        .leftovers()
        .into_iter()
        .filter(|name| !name.contains("foreign"))
        .collect();
    assert_eq!(own.len(), 1, "the job's own staging exists: {own:?}");
    let staging = lab.work.path(&own[0]);
    let half = fs::metadata(staging.join("mix.wav")).unwrap().len();
    assert!(
        half >= 44 + 48_000 * 4,
        "the steps already written are on disk: {half} bytes"
    );
    let info = cancel_parked(&mut lab, &job);
    assert_eq!(info["stage"], "audio", "it never left the mix: {info}");
    assert!(!destination.exists());
    assert!(
        !staging.exists(),
        "the half-written mix went with its staging"
    );
    assert_only_the_foreign_staging_remains(&lab, &foreign, &mix, "cancel-mix.mp4");
}

#[test]
fn a_header_that_changed_after_the_request_is_resource_changed_whatever_it_became() {
    // `freeze` accepted this file's header when the request came in. A different file with the same
    // size, put there before the job starts, must say what happened (the file changed), not what
    // the new header would have been on its own (`unsupported`, `audio_source_short`).
    let mix = two_tone(48_000, 4, 440.0, 880.0);
    let Some(mut lab) = lab_with("e2e-header", &mix, (0.0, 4.0), Fault::SwapAtStart) else {
        return;
    };
    let data = mix.len() - 44;
    let swaps: [(&str, Vec<u8>); 5] = [
        // Valid, but 2 s of stereo: the header alone would be `audio_source_short`.
        ("stereo-2s", pcm16(2, 48_000, &vec![0i16; data / 2])),
        // 24-bit: `unsupported` on its own.
        ("int24", wav(1, 24, 1, 48_000, &vec![0u8; data])),
        // Not a WAV at all.
        ("not-a-wav", vec![b'x'; mix.len()]),
        // Another valid mix that still covers the window: only the header differs.
        ("mono-24k", pcm16(1, 24_000, &vec![0i16; data / 2])),
        // Another size.
        ("truncated", mix[..mix.len() - 4].to_vec()),
    ];
    for (name, bytes) in swaps {
        if name != "truncated" {
            assert_eq!(bytes.len(), mix.len(), "{name} has the same size");
        }
        fs::write(lab.base.join("audio/mix.wav"), &mix).unwrap();
        *lab.swap.lock().unwrap() = Some(bytes);
        let out = format!("{name}.mp4");
        let error = lab.export(&out, tiny(), 0, 24).unwrap_err();
        assert_eq!(error.code, "resource_changed", "{name}: {}", error.message);
        let details = error.details.as_ref().expect("the report");
        assert_eq!(details["resource"], "audio-mix", "{name}");
        assert_eq!(details["blockers"][0]["code"], "resource_changed", "{name}");
        assert_eq!(details["blockers"][0]["asset"], "audio-mix");
        assert_eq!(
            lab.drawn.load(Ordering::SeqCst),
            0,
            "{name}: before any frame"
        );
        assert!(!lab.work.path(&out).exists(), "{name}");
        assert_clean(&lab);
    }
}
