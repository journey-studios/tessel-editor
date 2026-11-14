//! BL081.5: the schedule and the WAV actually fed to FFmpeg share the core's clock.
mod common;

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use tessel_editor_core::audio::{AudioBackend, OfflineMixer, Pcm, encode_wav_pcm16};
use tessel_editor_core::backend::{BackendError, Capabilities, FrameResult, Output};
use tessel_editor_core::jobs::{JobKind, JobRegistry};
use tessel_editor_core::plan::RenderPlan;
use tessel_editor_core::preset::Preset;
use tessel_editor_core::session::Renderer;
use tessel_editor_core::{EditorState, headless};
use tessel_editor_media::export::{ExportRequest, run, schedule};

use common::mock::{Mock, Settings};

fn preset() -> Preset {
    Preset::from_json(&json!({"id":"phase", "name":"Phase", "width":32, "height":18})).unwrap()
}

fn state() -> EditorState {
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create", "name":"Phase", "rate":{"num":30000,"den":1001}, "length":360,"width":32,"height":18})).unwrap();
    state
}

#[test]
fn a_ranged_schedule_has_the_absolute_sample_count_not_a_rounded_duration() {
    let state = state();
    let boundary = |frame: i64| (frame * 1001 * 48000 + 15000) / 30000;
    for from in 0..10 {
        for frames in [1, 2, 5, 23, 301] {
            let mut request = ExportRequest::new(preset(), "unused.mp4");
            request.range = Some((from, from + frames));
            let plan = schedule(&state, &request).unwrap();
            assert_eq!(
                plan.audio_samples,
                (boundary(from + frames) - boundary(from)) as u64,
                "range {from}..{}",
                from + frames
            );
        }
    }
    // A changed output FPS still follows the duration of the encoded frames.
    let mut request = ExportRequest::new(preset(), "unused.mp4");
    request.range = Some((1, 24));
    request.preset.fps = Some(tessel_editor_core::time::Rate::FPS_24);
    let plan = schedule(&state, &request).unwrap();
    assert_eq!(plan.audio_samples, plan.frames * 2000);
}

/// At the first video frame the export's completed audio.wav is still in its private staging
/// directory. Capture those bytes, then let the real H.264/AAC export finish normally.
struct CaptureAudio {
    renderer: Mock,
    wav: Option<Vec<u8>>,
}
impl Renderer for CaptureAudio {
    fn render_frame(
        &mut self,
        state: &EditorState,
        plan: RenderPlan,
        output: Output,
        probes: Vec<(u32, u32)>,
        timeout: Duration,
    ) -> Result<FrameResult, BackendError> {
        if self.wav.is_none() {
            let path = PathBuf::from(output.png.as_ref().unwrap())
                .parent()
                .unwrap()
                .join("audio.wav");
            self.wav = Some(fs::read(path).unwrap());
        }
        self.renderer
            .render_frame(state, plan, output, probes, timeout)
    }
    fn capabilities(&mut self) -> Result<Capabilities, BackendError> {
        self.renderer.capabilities()
    }
    fn summary(&self) -> Value {
        self.renderer.summary()
    }
    fn reset_project(&mut self) {
        self.renderer.reset_project();
    }
}

#[test]
fn an_ntsc_export_keeps_the_exact_wav_across_cuts_and_nonzero_chunked_ranges() {
    let dir = common::temp("export-ntsc-audio-phase");
    let source = dir.join("ramp.wav");
    let pcm = Pcm {
        sample_rate: 48000,
        channels: 1,
        samples: (0..48000 * 15)
            .map(|i| ((i % 4096) as f32 - 2048.0) / 8192.0)
            .collect(),
    };
    fs::write(&source, encode_wav_pcm16(&pcm)).unwrap();
    let mut state = state();
    for op in [
        json!({"op":"asset.add","kind":"audio","id":"snd","name":"Sound","path":source}),
        json!({"op":"track.add","kind":"audio","id":"a","name":"A"}),
        json!({"op":"clip.add","track":"a","asset":"snd","id":"c","start":1,"duration":358,"properties":{"pan":-100}}),
    ] {
        headless::execute(&mut state, &op).unwrap();
    }
    let mut cut = state.clone();
    headless::execute(&mut cut, &json!({"op":"clip.split","clip":"c","frame":17})).unwrap();
    let tools = common::tools();
    for (from, to) in [(1, 2), (2, 3), (1, 333)] {
        let reference = encode_wav_pcm16(
            &OfflineMixer::default()
                .mix(&state.plan_audio(from, to - from).unwrap())
                .unwrap(),
        );
        for (label, project) in [("whole", &state), ("cut", &cut)] {
            let destination = dir.join(format!("{label}-{from}-{to}.mp4"));
            let mut request = ExportRequest::new(preset(), &destination);
            request.range = Some((from, to));
            let mut renderer = CaptureAudio {
                renderer: Mock::new(Settings::default()),
                wav: None,
            };
            let registry = JobRegistry::new(1);
            let (_, result) = registry.run_inline(JobKind::VideoExport, "export", |ctx| {
                run(project, &request, &tools, &mut renderer, ctx)
            });
            let report = result.unwrap();
            assert_eq!(report["verification"]["ok"], true, "{report}");
            assert!(fs::metadata(destination).unwrap().len() > 0);
            assert_eq!(
                renderer.wav.as_ref().unwrap(),
                &reference,
                "{label}, range {from}..{to}: the actual WAV given to FFmpeg must equal the whole mix"
            );
        }
    }
}
