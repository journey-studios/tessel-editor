//! The cpal adapter: opens the default output device on its own thread and runs an
//! [`Engine`] in the data callback.
//!
//! Opening a device, starting a stream and dropping it can each block for tens to hundreds of
//! milliseconds (CoreAudio), so none of them happens on the caller's thread: [`Output::start`]
//! returns at once with [`Status::Opening`], and the caller reads [`Output::status`] on its next
//! tick. When there is no device, or it cannot be configured, the status carries an explicit
//! `audio_output_unavailable` error; nothing falls back silently to another clock.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig, StreamError};
use serde_json::{Value, json};

use crate::engine::{CallbackInfo, Engine, Shared, Source};

/// Messages a device error callback may leave for the report; the rest is only counted.
const KEPT_MESSAGES: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct AudioError {
    pub code: &'static str,
    pub message: String,
}

impl AudioError {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: "audio_output_unavailable",
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// What the stream was opened with.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub name: String,
    pub host: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
    /// `default` or the fixed size asked of the device.
    pub buffer: String,
}

impl DeviceInfo {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name, "host": self.host, "sample_rate": self.sample_rate,
            "channels": self.channels, "sample_format": self.sample_format, "buffer": self.buffer,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Opening,
    Playing(DeviceInfo),
    Failed(AudioError),
}

const OPENING: u8 = 0;

pub struct Output {
    shared: Arc<Shared>,
    status: Arc<Mutex<Status>>,
    messages: Arc<Mutex<Vec<String>>>,
    close: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
    /// Mirrors `status` for a lock-free "is it open yet" read.
    phase: Arc<AtomicU8>,
}

impl Output {
    /// Starts playing `source` from `start_frame` on the default output device. Never blocks.
    pub fn start(source: Source, start_frame: u64) -> Self {
        let shared = Arc::new(Shared::new(&source, start_frame));
        let status = Arc::new(Mutex::new(Status::Opening));
        let messages = Arc::new(Mutex::new(Vec::new()));
        let phase = Arc::new(AtomicU8::new(OPENING));
        let (close, closed) = mpsc::channel();
        let thread = {
            let (shared, status, messages) = (shared.clone(), status.clone(), messages.clone());
            let phase = phase.clone();
            std::thread::Builder::new()
                .name("tessel-audio".into())
                .spawn(move || {
                    let outcome = open(source, &shared, &messages);
                    let (stream, next) = match outcome {
                        Ok((stream, info)) => (Some(stream), Status::Playing(info)),
                        Err(error) => (None, Status::Failed(error)),
                    };
                    set(&status, &phase, next);
                    if stream.is_some() {
                        // Holds the stream (and so the callback) alive until asked to close or
                        // until the `Output` is gone; dropping it happens here, off every caller.
                        let _ = closed.recv();
                    }
                    drop(stream);
                })
        };
        let thread = match thread {
            Ok(handle) => Some(handle),
            Err(error) => {
                set(
                    &status,
                    &phase,
                    Status::Failed(AudioError::unavailable(format!(
                        "Cannot start the audio thread: {error}"
                    ))),
                );
                None
            }
        };
        Self {
            shared,
            status,
            messages,
            close: Some(close),
            thread,
            phase,
        }
    }

    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// `true` once the device stream is running.
    pub fn is_playing(&self) -> bool {
        self.phase.load(Ordering::SeqCst) == PLAYING
    }

    /// Device errors the stream reported (first few), besides the underrun counter.
    pub fn messages(&self) -> Vec<String> {
        self.messages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Asks the audio thread to stop and drop the stream; returns at once.
    pub fn close(&mut self) {
        self.close.take();
        self.thread.take();
    }

    /// Like [`Output::close`] but waits for the device to be released (tests and the probe).
    pub fn close_and_wait(&mut self) {
        self.close.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.close();
    }
}

const PLAYING: u8 = 1;
const FAILED: u8 = 2;

fn set(status: &Mutex<Status>, phase: &AtomicU8, next: Status) {
    let code = match &next {
        Status::Opening => OPENING,
        Status::Playing(_) => PLAYING,
        Status::Failed(_) => FAILED,
    };
    *status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = next;
    phase.store(code, Ordering::SeqCst);
}

fn open(
    source: Source,
    shared: &Arc<Shared>,
    messages: &Arc<Mutex<Vec<String>>>,
) -> Result<(Stream, DeviceInfo), AudioError> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| AudioError::unavailable("There is no default audio output device"))?;
    let name = device
        .description()
        .map(|description| description.name().to_string())
        .unwrap_or_else(|_| "unknown device".to_string());
    let config = choose(&device, source.sample_rate)
        .map_err(|message| AudioError::unavailable(format!("{name}: {message}")))?;
    let format = config.sample_format();
    let stream_config: StreamConfig = config.config();
    let engine = Engine::new(
        source,
        shared.clone(),
        stream_config.sample_rate,
        usize::from(stream_config.channels),
    );
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, &stream_config, engine, messages),
        SampleFormat::I16 => build::<i16>(&device, &stream_config, engine, messages),
        other => {
            return Err(AudioError::unavailable(format!(
                "{name} wants {other:?} samples; only f32 and i16 outputs are supported"
            )));
        }
    }
    .map_err(|error| AudioError::unavailable(format!("{name}: {error}")))?;
    stream
        .play()
        .map_err(|error| AudioError::unavailable(format!("{name}: cannot start: {error}")))?;
    Ok((
        stream,
        DeviceInfo {
            name,
            host: host.id().name().to_string(),
            sample_rate: stream_config.sample_rate,
            channels: stream_config.channels,
            sample_format: format!("{format:?}"),
            buffer: format!("{:?}", stream_config.buffer_size).to_lowercase(),
        },
    ))
}

/// A stereo f32 config at the source's own rate when the device offers one (no resampling at
/// all); otherwise the device's default.
fn choose(device: &cpal::Device, rate: u32) -> Result<cpal::SupportedStreamConfig, String> {
    if let Ok(configs) = device.supported_output_configs() {
        for range in configs {
            let fits = range.channels() >= 2
                && range.min_sample_rate() <= rate
                && rate <= range.max_sample_rate()
                && matches!(range.sample_format(), SampleFormat::F32 | SampleFormat::I16);
            if fits {
                return Ok(range.with_sample_rate(rate));
            }
        }
    }
    device
        .default_output_config()
        .map_err(|error| format!("no usable output configuration: {error}"))
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut engine: Engine,
    messages: &Arc<Mutex<Vec<String>>>,
) -> Result<Stream, cpal::BuildStreamError>
where
    T: SizedSample + FromSample<f32>,
{
    let shared = engine.shared().clone();
    let errors = shared.clone();
    let messages = messages.clone();
    // Allocated once; a larger buffer than this grows it (rare, and only on the first such call).
    let mut scratch: Vec<f32> = vec![0.0; 8192 * usize::from(config.channels)];
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
            if scratch.len() < data.len() {
                scratch.resize(data.len(), 0.0);
            }
            let timestamp = info.timestamp();
            let latency = timestamp
                .playback
                .duration_since(&timestamp.callback)
                .unwrap_or_default();
            let at = shared.elapsed();
            let window = &mut scratch[..data.len()];
            engine.fill(window, CallbackInfo { at, latency });
            for (out, sample) in data.iter_mut().zip(window.iter()) {
                *out = T::from_sample(*sample);
            }
        },
        move |error: StreamError| {
            if matches!(error, StreamError::BufferUnderrun) {
                errors.device_underrun();
                return;
            }
            let mut kept = messages
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if kept.len() < KEPT_MESSAGES {
                kept.push(error.to_string());
            }
        },
        None,
    )
}
