//! Device audio output for playback.
//!
//! [`engine`] is the device-independent rule set (a sample clock, pause, seek, resampling, the end);
//! [`output`] is the adapter that runs it on the system's default output device through CoreAudio
//! (macOS), WASAPI or ALSA via `cpal`. Everything that can block (opening the device, starting and
//! dropping the stream) happens on the adapter's own thread; callers only read atomics.

pub mod engine;
pub mod output;

pub use engine::{Source, Stats};
pub use output::{AudioError, DeviceInfo, Output, Status};
