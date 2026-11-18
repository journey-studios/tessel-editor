//! Animated pictures (GIF, APNG, animated WebP): what they are made of and the constant frame
//! grid the editor plays them on (F073).
//!
//! These files hold several pictures, each with its **own** display time. The editor draws
//! container video from numbered frames at one constant rate, so an animated picture becomes a
//! video asset whose rate is the coarsest grid every frame time is a multiple of: the greatest
//! common divisor of the display times. A GIF made of 40 ms and 100 ms frames plays on a 20 ms
//! grid (50 fps) and the 100 ms frame is held for five grid steps, so no frame is dropped and
//! none is shown for the wrong time. Display times that share no usable divisor (an animated
//! WebP with 33/34 ms frames) fall back to their mean rate, rounded, and the decode picks the
//! nearest frame per step (at most half a step of timing error, never a blend); the import says so.
//!
//! The picture itself is decoded by FFmpeg into RGBA PNG frames (`Cache::decode_video`), which is
//! what keeps transparency and the disposal rules of the format exact. A file with a single
//! picture is a still (`probe` decides).

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use tessel_editor_core::assets::{MediaKind, Meta};
use tessel_editor_core::jobs::JobError;
use tessel_editor_core::time::{Rate, Ratio};

use crate::probe::Probe;
use crate::tools::Tools;

/// A grid finer than this is not worth it: the PNG sequence would be several times the size of
/// the animation for a timing difference nobody can see.
const MAX_GRID_FPS: i64 = 50;
/// Mean rate used when no divisor grid is usable, at most (the editor's own cap on a rate).
const MAX_FALLBACK_FPS: i64 = 60;

/// Whether FFprobe's codec and container name an animated picture format.
pub fn is_animated_format(codec: &str, container: &str) -> bool {
    matches!(codec, "gif" | "apng" | "webp_anim")
        || container.contains("webp_anim")
        || container == "apng"
        || container.contains("gif")
}

/// The label the asset keeps as its `format`.
fn format_label(codec: &str, container: &str) -> &'static str {
    if codec == "gif" || container.contains("gif") {
        "gif"
    } else if codec == "apng" || container == "apng" {
        "apng"
    } else {
        "webp"
    }
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a.abs() } else { gcd(b, a % b) }
}

/// The grid the display times `durations_ms` play on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub rate: Rate,
    /// Every display time is a whole number of grid steps.
    pub exact: bool,
    pub total_ms: i64,
}

/// Picks the grid of a list of display times (milliseconds, each at least 1). `None` for fewer
/// than two pictures (a still) or an unusable list.
pub fn grid_of(durations_ms: &[i64]) -> Option<Grid> {
    if durations_ms.len() < 2 || durations_ms.iter().any(|ms| *ms <= 0) {
        return None;
    }
    let total_ms: i64 = durations_ms.iter().sum();
    let divisor = durations_ms.iter().fold(0, |acc, ms| gcd(acc, *ms));
    // `1000 / divisor` frames per second; the cap is on that, not on the divisor.
    if divisor * MAX_GRID_FPS >= 1000 {
        return Some(Grid {
            rate: Rate::new(1000, u32::try_from(divisor).ok()?)?,
            exact: true,
            total_ms,
        });
    }
    let mean = (durations_ms.len() as f64 * 1000.0 / total_ms as f64).round() as i64;
    Some(Grid {
        rate: Rate::new(u32::try_from(mean.clamp(1, MAX_FALLBACK_FPS)).ok()?, 1)?,
        exact: false,
        total_ms,
    })
}

/// Display time of every picture, in milliseconds, in presentation order. `None` when FFprobe
/// cannot list the packets.
fn display_times(tools: &Tools, path: &Path) -> Option<Vec<i64>> {
    let output = Command::new(&tools.ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time,duration_time",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut rows: Vec<(f64, Option<f64>)> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut cells = line.trim().split(',');
            let pts = cells.next()?.trim().parse::<f64>().ok()?;
            let duration = cells
                .next()
                .and_then(|cell| cell.trim().parse::<f64>().ok())
                .filter(|seconds| *seconds > 0.0);
            Some((pts, duration))
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut times = Vec::with_capacity(rows.len());
    for (index, (pts, duration)) in rows.iter().enumerate() {
        // The packet's own duration when the demuxer states one, else the gap to the next
        // picture (the last one repeats the gap before it).
        let seconds = duration
            .or_else(|| rows.get(index + 1).map(|next| next.0 - pts))
            .or_else(|| times.last().map(|ms: &i64| *ms as f64 / 1000.0))?;
        times.push(((seconds * 1000.0).round() as i64).max(1));
    }
    Some(times)
}

/// What a file of an animated format is: `None` when it holds a single picture (the caller makes
/// it a still), else a video asset on the grid of its display times.
pub fn probe_animated(
    tools: &Tools,
    path: &Path,
    size: u64,
    (width, height): (u32, u32),
    codec: &str,
    container: &str,
    summary: Value,
) -> Result<Option<Probe>, JobError> {
    let times = display_times(tools, path).ok_or_else(|| {
        JobError::new(
            "unsupported_format",
            format!(
                "`{}`: the picture times of the animation could not be read",
                path.display()
            ),
        )
    })?;
    let Some(grid) = grid_of(&times) else {
        return Ok(None);
    };
    let mut warnings = Vec::new();
    if !grid.exact {
        warnings.push(format!(
            "animation_timing_conformed: the display times share no common step; frames are conformed to a constant {:.3} fps grid (at most half a step of timing error), never blended",
            grid.rate.as_f64()
        ));
    }
    let label = format_label(codec, container);
    Ok(Some(Probe {
        kind: MediaKind::Video,
        meta: Meta {
            width: Some(width),
            height: Some(height),
            rate: Some(grid.rate),
            duration: Ratio::new(grid.total_ms, 1000),
            format: Some(label.into()),
            ..Meta::default()
        },
        size,
        has_audio: false,
        has_video: true,
        warnings,
        summary: json!({ "streams": summary, "video_codec": codec, "animated": true,
            "frames": times.len(), "color": { "pix_fmt": "rgba" } }),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_constant_delay_is_its_own_grid() {
        let grid = grid_of(&[40, 40, 40]).unwrap();
        assert_eq!(grid.rate, Rate::new(25, 1).unwrap());
        assert!(grid.exact);
        assert_eq!(grid.total_ms, 120);
    }

    #[test]
    fn mixed_delays_play_on_their_greatest_common_step() {
        // 40 and 100 ms share 20 ms: a 50 fps grid, the 100 ms picture held for five steps.
        let grid = grid_of(&[40, 40, 100]).unwrap();
        assert_eq!(grid.rate, Rate::new(50, 1).unwrap());
        assert!(grid.exact);
        // 30 and 40 ms share 10 ms: 100 fps is finer than the cap, so the mean rate is used.
        let grid = grid_of(&[30, 40, 30, 40]).unwrap();
        assert!(!grid.exact);
        assert_eq!(grid.rate, Rate::new(29, 1).unwrap());
    }

    #[test]
    fn fractional_grids_stay_exact() {
        // 3 pictures of 66 ms and 99 ms: gcd 33 ms = 1000/33 fps.
        let grid = grid_of(&[66, 99, 66]).unwrap();
        assert_eq!(grid.rate, Rate::new(1000, 33).unwrap());
        assert!(grid.exact);
    }

    #[test]
    fn one_picture_or_a_bad_list_is_no_animation() {
        assert!(grid_of(&[100]).is_none());
        assert!(grid_of(&[]).is_none());
        assert!(grid_of(&[40, 0]).is_none());
    }

    #[test]
    fn formats_are_recognised_by_codec_or_container() {
        assert!(is_animated_format("gif", "gif"));
        assert!(is_animated_format("apng", "apng"));
        assert!(is_animated_format("webp_anim", "webp_anim"));
        assert!(!is_animated_format("png", "png_pipe"));
        assert!(!is_animated_format("webp", "webp_pipe"));
        assert!(!is_animated_format("h264", "mov,mp4,m4a,3gp,3g2,mj2"));
    }
}
