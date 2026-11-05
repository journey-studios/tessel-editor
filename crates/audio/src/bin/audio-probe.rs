//! Measures the output path on the machine's real default device and prints one JSON report.
//!
//! What it measures is *technical*: the device sample clock against the system clock, callback
//! timing and underruns, the latency the host reports, and whether pause, resume, seek and the end
//! land where the sample clock says. It plays a quiet tone, but nobody is listening to it here:
//! this is not evidence that anything was heard, and the reported latency is the host's estimate
//! (buffer based on CoreAudio), not an acoustic measurement.
//!
//! `audio-probe [seconds]` (default 6). Exit status 2 with `audio_output_unavailable` in the report
//! when there is no usable device.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tessel_editor_audio::{Output, Source, Status};

const RATE: u32 = 48_000;

fn tone(seconds: f64) -> Source {
    let frames = (seconds * f64::from(RATE)) as usize;
    let mut samples = Vec::with_capacity(frames * 2);
    for n in 0..frames {
        let value = (n as f64 / f64::from(RATE) * 440.0 * std::f64::consts::TAU).sin() * 0.1;
        samples.push(value as f32);
        samples.push(value as f32);
    }
    Source {
        samples: samples.into(),
        sample_rate: RATE,
    }
}

fn wait(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    done()
}

fn slope(points: &[(f64, f64)]) -> f64 {
    let n = points.len() as f64;
    let (sx, sy) = points
        .iter()
        .fold((0.0, 0.0), |(x, y), p| (x + p.0, y + p.1));
    let (mx, my) = (sx / n, sy / n);
    let covariance: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let variance: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
    covariance / variance
}

fn main() {
    let seconds: f64 = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(6.0);
    let source = tone(seconds);
    let opened = Instant::now();
    let mut output = Output::start(source, 0);
    let returned = opened.elapsed();
    wait(Duration::from_secs(10), || {
        output.status() != Status::Opening
    });
    let open_ms = opened.elapsed().as_secs_f64() * 1000.0;
    let device = match output.status() {
        Status::Playing(info) => info,
        Status::Failed(error) => {
            println!(
                "{}",
                json!({ "ok": false, "error": { "code": error.code, "message": error.message },
                    "start_returned_ms": returned.as_secs_f64() * 1000.0 })
            );
            std::process::exit(2);
        }
        Status::Opening => {
            println!(
                "{}",
                json!({ "ok": false, "error": { "code": "audio_output_unavailable",
                    "message": "The device did not open within 10 s" } })
            );
            std::process::exit(2);
        }
    };
    let shared = output.shared().clone();
    let rate = f64::from(RATE);

    // 1. Free-running: sample the position against the wall for 2 s.
    wait(Duration::from_secs(2), || shared.stats().callbacks > 3);
    let first_callback = Instant::now();
    let mut samples: Vec<(f64, f64)> = Vec::new();
    let mut produced: Vec<(f64, f64)> = Vec::new();
    let mut backwards = 0u32;
    let mut last = -1.0_f64;
    let run_until = Instant::now() + Duration::from_millis(4_000);
    while Instant::now() < run_until {
        let position = shared.position();
        if position < last {
            backwards += 1;
        }
        last = position;
        let now = first_callback.elapsed().as_secs_f64();
        samples.push((now, position));
        produced.push((now, shared.stats().frames_out as f64 / rate));
        std::thread::sleep(Duration::from_millis(10));
    }
    // Position advance per wall second between the first and last sample (1.0 = in step).
    let (t0, p0) = samples[0];
    let (t1, p1) = samples[samples.len() - 1];
    let rate_ratio = ((p1 - p0) / rate) / (t1 - t0);
    // The device consumes whole buffers, so a single pair of readings is off by up to one buffer
    // period; the slope of a least-squares line through all of them is not.
    let drift_ppm = (slope(&produced) - 1.0) * 1e6;

    // 2. Pause: the position freezes and stays frozen; resume continues from it.
    shared.pause();
    wait(Duration::from_secs(1), || {
        shared.stats().callbacks > 0 && shared.is_paused()
    });
    std::thread::sleep(Duration::from_millis(150));
    let held = shared.position();
    std::thread::sleep(Duration::from_millis(500));
    let still = shared.position();
    let resume_call = Instant::now();
    shared.resume();
    std::thread::sleep(Duration::from_millis(250));
    let resumed = shared.position();
    let resume_elapsed = resume_call.elapsed().as_secs_f64() * 1000.0;

    // 3. Seek to 4 s (or the middle of a short probe).
    let target = ((seconds * 0.66) * rate) as u64;
    let seek_call = Instant::now();
    shared.seek(target);
    std::thread::sleep(Duration::from_millis(300));
    let after_seek = shared.position();
    let seek_elapsed = seek_call.elapsed().as_secs_f64() * 1000.0;

    // 4. Run to the end: when the sample clock says finished, against what the length predicts.
    let remaining = (seconds * rate - after_seek) / rate;
    let ended_started = Instant::now();
    let ended = wait(Duration::from_secs_f64(remaining + 3.0), || {
        shared.finished()
    });
    let waited = ended_started.elapsed().as_secs_f64();
    let stats = shared.stats();
    let messages = output.messages();
    output.close_and_wait();

    let report: Value = json!({
        "ok": true,
        "device": device.to_json(),
        "start_returned_ms": returned.as_secs_f64() * 1000.0,
        "open_ms": open_ms,
        "latency_reported_ms": stats.latency.as_secs_f64() * 1000.0,
        "latency_note": "the host's estimate (buffer based on CoreAudio); DAC, Bluetooth and speaker delay are not observable here",
        "buffer_frames": stats.buffer_frames,
        "callbacks": stats.callbacks,
        "underruns": { "late_callbacks": stats.late_callbacks, "device": stats.device_underruns,
            "max_callback_gap_ms": stats.max_callback_gap.as_secs_f64() * 1000.0 },
        "clock": {
            "device_vs_system_drift_ppm": drift_ppm,
            "position_rate_ratio": rate_ratio,
            "position_went_backwards": backwards,
            "samples": samples.len(),
        },
        "pause": {
            "held_frames": held, "after_500ms_frames": still,
            "frozen": (held - still).abs() < 1.0,
            "resumed_advanced_ms": (resumed - still) / rate * 1000.0,
            "resumed_wall_ms": resume_elapsed,
            "resume_error_ms": (resumed - still) / rate * 1000.0 - resume_elapsed,
        },
        "seek": {
            "target_frames": target, "heard_after_300ms_frames": after_seek,
            "advanced_ms": (after_seek - target as f64) / rate * 1000.0,
            "wall_ms": seek_elapsed,
            // 0 means the position landed on the target and advanced exactly with the wall clock.
            "error_ms": (after_seek - target as f64) / rate * 1000.0 - seek_elapsed,
        },
        "end": {
            "finished": ended, "waited_s": waited, "expected_s": remaining,
            "error_ms": (waited - remaining) * 1000.0,
        },
        "device_messages": messages,
        "heard": false,
        "heard_note": "technical measurement only: nobody listened",
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
