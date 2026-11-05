//! Video export driven by the real runtime (Node worker + headless Chrome + Three.js), through
//! the same `export.start` operation every adapter uses. Skipped without the runtime unless
//! `TESSEL_REQUIRE_RUNTIME=1`, like `frames.rs`.
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{
    QUADRANT_COLORS, proof_requests, video_frame_color, write_standard_set,
};
use tessel_editor_core::{EditorState, headless};
use tessel_editor_media::host::{MediaHost, RendererFactory};
use tessel_editor_media::tools::Tools;
use tessel_editor_render::{RuntimeConfig, session};

fn rgb_frame(tools: &Tools, file: &Path, index: u64, width: u32, height: u32) -> Vec<u8> {
    let filter = format!(
        "select=eq(n\\,{index}),scale=in_color_matrix=bt709:in_range=tv:out_range=pc,format=rgb24"
    );
    let output = Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(file)
        .args(["-vf", &filter, "-frames:v", "1", "-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert_eq!(
        output.stdout.len(),
        (width * height * 3) as usize,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn pixel(rgb: &[u8], width: u32, x: u32, y: u32) -> [i32; 3] {
    let at = ((y * width + x) * 3) as usize;
    [
        i32::from(rgb[at]),
        i32::from(rgb[at + 1]),
        i32::from(rgb[at + 2]),
    ]
}

fn near(actual: [i32; 3], expected: [u8; 3], tolerance: i32, what: &str) {
    for channel in 0..3 {
        assert!(
            (actual[channel] - i32::from(expected[channel])).abs() <= tolerance,
            "{what}: got {actual:?}, expected about {expected:?}"
        );
    }
}

fn setup(tag: &str) -> Option<(MediaHost, EditorState, std::path::PathBuf, Tools)> {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => {
            assert!(
                std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
                "the runtime is required but unavailable: {}",
                setup.message()
            );
            eprintln!("SKIPPED (runtime unavailable): {}", setup.message());
            return None;
        }
    };
    let tools =
        Tools::discover().expect("FFmpeg is required for the export tests (docs/media-setup.md)");
    let dir =
        std::env::temp_dir().join(format!("tessel-render-export-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let set = write_standard_set(&dir.join("fixtures")).unwrap();
    let factory: Arc<RendererFactory> = Arc::new(move || Ok(Box::new(session(&config))));
    let host = MediaHost::new().with_renderer(factory);
    let mut state = EditorState::demo();
    for request in proof_requests(&set) {
        headless::execute(&mut state, &request).unwrap();
    }
    headless::execute(
        &mut state,
        &json!({"op":"project.save","path":dir.join("project/proof.tessel.json")}),
    )
    .unwrap();
    Some((host, state, dir, tools))
}

fn export(host: &mut MediaHost, state: &mut EditorState, request: Value) -> Value {
    headless::execute_ext(state, None, Some(host), &request)
        .unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

#[test]
fn real_runtime_export_matches_the_frames_the_monitor_draws() {
    let Some((mut host, mut state, dir, tools)) = setup("real") else {
        return;
    };
    let target = dir.join("out/proof.mp4");
    let preset = json!({"id":"proof640","name":"Proof","width":640,"height":360});
    // Frames 0..6: scene + still image, then the tone is mixed from the audio track.
    let report = export(
        &mut host,
        &mut state,
        json!({"op":"export.start","preset":preset,"destination":target,"range":[0, 6]}),
    );
    assert_eq!(report["verification"]["ok"], true, "{report}");
    assert_eq!(report["frames"], 6);
    assert_eq!(
        report["render"],
        json!({"width":640,"height":360}),
        "rendered at the real size"
    );
    assert!(report["audio"]["rms"].as_f64().unwrap() > 0.01);

    // The still image is centered at 50 %: the quadrant centers of the 320x180 canvas, doubled.
    let rgb = rgb_frame(&tools, &target, 3, 640, 360);
    for (index, (x, y)) in [(240u32, 134u32), (400, 134), (240, 224), (400, 224)]
        .into_iter()
        .enumerate()
    {
        near(
            pixel(&rgb, 640, x, y),
            QUADRANT_COLORS[index],
            24,
            &format!("quadrant {index} after H.264"),
        );
    }

    // Second half of the sequence shows the frame-sequence video: the time map picks its frames.
    let late = dir.join("out/late.mp4");
    let report = export(
        &mut host,
        &mut state,
        json!({"op":"export.start","preset":preset,"destination":late,"range":[24, 30]}),
    );
    assert_eq!(report["verification"]["ok"], true, "{report}");
    let rgb = rgb_frame(&tools, &late, 0, 640, 360);
    // 8 fps video on a 24 fps sequence: sequence frame 24 shows video frame 0.
    near(
        pixel(&rgb, 640, 320, 200),
        video_frame_color(0),
        24,
        "video frame 0 at sequence frame 24",
    );
    let rgb = rgb_frame(&tools, &late, 5, 640, 360);
    near(
        pixel(&rgb, 640, 320, 200),
        video_frame_color(1),
        24,
        "video frame 1 at sequence frame 29",
    );
}

#[test]
fn real_runtime_export_frames_a_vertical_output_with_black_bars() {
    let Some((mut host, mut state, dir, tools)) = setup("vertical") else {
        return;
    };
    let target = dir.join("out/vertical.mp4");
    let preset = json!({"id":"vertical360","name":"Vertical","width":360,"height":640});
    let report = export(
        &mut host,
        &mut state,
        json!({"op":"export.start","preset":preset,"destination":target,"range":[0, 4]}),
    );
    assert_eq!(report["verification"]["ok"], true, "{report}");
    assert_eq!(report["output"], json!({"width":360,"height":640}));
    let render = report["render"].clone();
    assert_eq!(
        render["width"], 360,
        "the sequence is rendered at the width of the output: {render}"
    );
    assert!(
        render["height"].as_u64().unwrap() < 640,
        "and is not stretched to the height: {render}"
    );

    let rgb = rgb_frame(&tools, &target, 1, 360, 640);
    near(pixel(&rgb, 360, 180, 10), [0, 0, 0], 4, "top bar");
    near(pixel(&rgb, 360, 180, 630), [0, 0, 0], 4, "bottom bar");
    // Image quadrants, scaled 1.125 and offset by the top bar (218 px).
    near(
        pixel(&rgb, 360, 135, 76 + 218),
        QUADRANT_COLORS[0],
        28,
        "top-left quadrant inside the picture",
    );
    near(
        pixel(&rgb, 360, 225, 127 + 218),
        QUADRANT_COLORS[3],
        28,
        "bottom-right quadrant inside the picture",
    );
}
