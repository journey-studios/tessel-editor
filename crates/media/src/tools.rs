//! FFmpeg/FFprobe as versioned adapters with a capability report.
//!
//! Nothing here assumes the machine has a codec: `Tools::discover` records the version and the
//! encoders/decoders the binary really lists, and callers (preflight, export) decide from that.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::jobs::JobError;

pub mod locate;

#[derive(Debug, Clone)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    /// First line of `ffmpeg -version`, kept verbatim for diagnostics.
    pub version: String,
    pub encoders: Vec<String>,
    pub decoders: Vec<String>,
    pub muxers: Vec<String>,
}

/// Override, `PATH`, then the install places of the platform (see [`locate`]).
fn find(tool: locate::Tool) -> Option<PathBuf> {
    locate::locate(&locate::System, tool).map(|found| found.path)
}

fn run_text(binary: &Path, args: &[&str]) -> Result<String, JobError> {
    let output = Command::new(binary)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            JobError::new(
                "tool_unavailable",
                format!("Cannot run `{}`: {error}", binary.display()),
            )
        })?;
    if !output.status.success() {
        return Err(JobError::new(
            "tool_unavailable",
            format!("`{}` exited with {}", binary.display(), output.status),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Names from a `-encoders`/`-decoders`/`-muxers` listing (the table below the `--`/`------` line: muxer listings use the short one).
fn listing_names(listing: &str) -> Vec<String> {
    listing
        .lines()
        .skip_while(|line| !line.trim_start().starts_with("--"))
        .skip(1)
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(str::to_string)
        .collect()
}

impl Tools {
    pub fn discover() -> Result<Self, JobError> {
        let missing = |tool: locate::Tool| {
            JobError::new(
                "tool_unavailable",
                format!(
                    "`{}` was not found. Install FFmpeg or set {} to its path.",
                    tool.binary(),
                    tool.override_var()
                ),
            )
        };
        let ffmpeg = find(locate::Tool::Ffmpeg).ok_or_else(|| missing(locate::Tool::Ffmpeg))?;
        let ffprobe = find(locate::Tool::Ffprobe).ok_or_else(|| missing(locate::Tool::Ffprobe))?;
        let version = run_text(&ffmpeg, &["-hide_banner", "-version"])?
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        Ok(Self {
            encoders: listing_names(&run_text(&ffmpeg, &["-hide_banner", "-encoders"])?),
            decoders: listing_names(&run_text(&ffmpeg, &["-hide_banner", "-decoders"])?),
            muxers: listing_names(&run_text(&ffmpeg, &["-hide_banner", "-muxers"])?),
            ffmpeg,
            ffprobe,
            version,
        })
    }

    pub fn has_encoder(&self, name: &str) -> bool {
        self.encoders.iter().any(|encoder| encoder == name)
    }

    pub fn has_decoder(&self, name: &str) -> bool {
        self.decoders.iter().any(|decoder| decoder == name)
    }

    pub fn has_muxer(&self, name: &str) -> bool {
        self.muxers.iter().any(|muxer| muxer == name)
    }

    /// Software H.264 is the reference encoder: a hardware one is an optimisation with its own
    /// rate control, so it never changes what "H.264 export" means.
    pub fn h264_encoder(&self) -> Option<&'static str> {
        ["libx264", "h264_videotoolbox"]
            .into_iter()
            .find(|name| self.has_encoder(name))
    }

    pub fn to_json(&self) -> Value {
        let decoders: Vec<&str> = [
            "h264",
            "hevc",
            "aac",
            "mp3",
            "pcm_s16le",
            "mjpeg",
            "png",
            "prores",
            "vp9",
        ]
        .into_iter()
        .filter(|name| self.has_decoder(name))
        .collect();
        json!({
            "ffmpeg": self.ffmpeg, "ffprobe": self.ffprobe, "version": self.version,
            "h264_encoder": self.h264_encoder(),
            "aac": self.has_encoder("aac"),
            "mp4": self.has_muxer("mp4"),
            "decoders": decoders,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listings_are_parsed_below_the_separator() {
        let listing =
            " Encoders:\n V..... = Video\n ------\n V....D libx264   H.264\n A....D aac   AAC\n";
        assert_eq!(listing_names(listing), ["libx264", "aac"]);
    }
}
