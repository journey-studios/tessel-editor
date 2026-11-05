//! The FFmpeg command line of a scene export: PNG frames on a pipe in, H.264 MP4 out.

use std::path::Path;

use tessel_editor_core::preset::{Geometry, Preset};
use tessel_editor_core::time::Rate;

use crate::export::filter_chain;

fn rate_arg(rate: Rate) -> String {
    format!("{}/{}", rate.num(), rate.den())
}

/// Video only: a directed scene has no mix to carry (`SceneExportSnapshot` refuses or omits
/// declared audio explicitly), so there is no `-map 1:a` branch to get wrong.
///
/// WHY a second builder next to `export::encode_args`: that one is private to the project export
/// and interleaves the audio input; the picture half (`filter_chain`, codec, color tags, CFR,
/// faststart) is the same and the tests verify the tags with ffprobe (`export::verify`) so the two
/// cannot drift unnoticed. Merging them is a one-line visibility change in `export.rs`, which
/// this delivery does not own (see `docs/migration/next-r1-report.md`).
pub fn encode_args(
    preset: &Preset,
    geometry: &Geometry,
    output_rate: Rate,
    output: &Path,
) -> Vec<String> {
    let rate = rate_arg(output_rate);
    let mut args: Vec<String> = [
        "-v",
        "error",
        "-nostdin",
        "-y",
        "-f",
        "image2pipe",
        "-framerate",
    ]
    .map(String::from)
    .into();
    args.push(rate.clone());
    args.extend(["-c:v", "png", "-i", "-"].map(String::from));
    args.extend(["-vf".to_string(), filter_chain(geometry)]);
    args.extend(["-map", "0:v:0"].map(String::from));
    args.extend(
        [
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-profile:v",
            "high",
            "-pix_fmt",
            "yuv420p",
        ]
        .map(String::from),
    );
    args.extend(["-crf".to_string(), preset.crf.to_string()]);
    args.extend(
        [
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-color_range",
            "tv",
        ]
        .map(String::from),
    );
    args.extend(["-r".to_string(), rate, "-fps_mode".into(), "cfr".into()]);
    args.extend(
        [
            "-map_metadata",
            "-1",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ]
        .map(String::from),
    );
    args.push(output.to_string_lossy().into_owned());
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use tessel_editor_core::preset::builtins;

    #[test]
    fn the_command_is_video_only_and_tags_bt709_tv_range() {
        let preset = builtins().remove(0);
        let geometry = preset.geometry(320, 180);
        let args = encode_args(
            &preset,
            &geometry,
            Rate::new(24, 1).unwrap(),
            Path::new("out.mp4"),
        );
        let joined = args.join(" ");
        assert!(joined.contains("-framerate 24/1"), "{joined}");
        assert!(joined.contains("-fps_mode cfr"), "{joined}");
        assert!(joined.contains("-color_range tv"), "{joined}");
        assert!(
            !joined.contains("1:a") && !joined.contains("aac"),
            "{joined}"
        );
        assert_eq!(args.last().map(String::as_str), Some("out.mp4"));
    }
}
