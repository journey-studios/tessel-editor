//! Procedural media fixtures built with FFmpeg's own generators (no binary files in the repo).
#![allow(dead_code)]

pub mod mock;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tessel_editor_media::tools::Tools;

pub fn tools() -> Tools {
    Tools::discover()
        .expect("FFmpeg and FFprobe are required for the media tests (see docs/media-setup.md)")
}

pub fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-media-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn ffmpeg(tools: &Tools, args: &[&str]) {
    let output = Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-y"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "ffmpeg {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 2 s, 24 fps, 320x180 H.264 + AAC (440 Hz tone).
pub fn video_with_audio(tools: &Tools, dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    ffmpeg(
        tools,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=24:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2:sample_rate=48000",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            path.to_str().unwrap(),
        ],
    );
    path
}

pub fn jpeg(tools: &Tools, dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    ffmpeg(
        tools,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=1",
            "-frames:v",
            "1",
            path.to_str().unwrap(),
        ],
    );
    path
}

/// 1 s of 880 Hz as MP3 (a codec the core mixer cannot read directly).
pub fn mp3(tools: &Tools, dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    ffmpeg(
        tools,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:duration=1:sample_rate=44100",
            "-c:a",
            "libmp3lame",
            path.to_str().unwrap(),
        ],
    );
    path
}
