//! Colour correction (inspector tab "Correção de cor": `brightness`, `contrast`, `saturation`)
//! drawn by the real runtime (Node worker + headless Chrome): the monitor frame (`render.frame`,
//! PNG) and the exported H.264 video both change as `tessel_editor_core::plan::Grading` documents,
//! and a clip back at 0 is the ungraded picture byte for byte. The subject is a solid-colour title
//! on black, so its lit pixels carry exactly one colour. Skipped without the runtime unless
//! `TESSEL_REQUIRE_RUNTIME=1`; FFmpeg reads the pixels back (`docs/media-setup.md`).
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::{EditorState, headless};
use tessel_editor_media::host::{MediaHost, RendererFactory};
use tessel_editor_media::tools::Tools;
use tessel_editor_render::{RuntimeConfig, session};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const COLOR: [u8; 3] = [200, 100, 50];

struct Rig {
    state: EditorState,
    session: Box<dyn Renderer>,
    tools: Tools,
    dir: PathBuf,
}

fn rig(tag: &str) -> Option<Rig> {
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
    let tools = Tools::discover().expect("FFmpeg is required for the grading tests");
    let dir = std::env::temp_dir().join(format!("tessel-grading-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut rig = Rig {
        state: EditorState::demo(),
        session: Box::new(session(&config)),
        tools,
        dir,
    };
    for request in [
        json!({"op":"project.create","name":"Grading","sequence":"main","rate":24,
            "width":WIDTH,"height":HEIGHT,"length":48}),
        json!({"op":"asset.add","kind":"text","id":"label","name":"Label",
            "text":{"content":"HHH","size":40,"color":"#c86432"}}),
        json!({"op":"track.add","id":"title-track","name":"Titles","kind":"video"}),
        json!({"op":"clip.add","id":"c-title","track":"title-track","asset":"label",
            "start":0,"duration":48}),
    ] {
        rig.ok(request);
    }
    Some(rig)
}

impl Rig {
    fn ok(&mut self, request: Value) -> Value {
        headless::execute_with(&mut self.state, Some(self.session.as_mut()), &request)
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    fn grade(&mut self, brightness: f64, contrast: f64, saturation: f64) {
        for (property, value) in [
            ("brightness", brightness),
            ("contrast", contrast),
            ("saturation", saturation),
        ] {
            self.ok(
                json!({"op":"clip.property","clip":"c-title","property":property,"value":value}),
            );
        }
    }

    /// The monitor frame as RGB, read back from the PNG `render.frame` wrote.
    fn frame(&mut self, name: &str) -> Vec<u8> {
        let png = self.dir.join(format!("{name}.png"));
        self.ok(json!({"op":"render.frame","frame":0,"out":png}));
        decode(&self.tools, &png, &["-frames:v", "1"])
    }
}

fn decode(tools: &Tools, file: &Path, extra: &[&str]) -> Vec<u8> {
    let output = Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(file)
        .args(extra)
        .args(["-pix_fmt", "rgb24", "-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert_eq!(
        output.stdout.len(),
        (WIDTH * HEIGHT * 3) as usize,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// The most frequent colour among the lit (not near-black) pixels: the title's colour.
fn title_color(rgb: &[u8]) -> [u8; 3] {
    let mut counts: HashMap<[u8; 3], usize> = HashMap::new();
    for pixel in rgb.chunks(3) {
        if pixel.iter().any(|channel| *channel > 24) {
            *counts.entry([pixel[0], pixel[1], pixel[2]]).or_default() += 1;
        }
    }
    let (color, count) = counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .expect("the title drew nothing");
    assert!(
        count > 500,
        "the title is a large solid area: {count} pixels"
    );
    color
}

/// The documented formula (`Grading` in `crates/core/src/plan.rs`), on 8-bit channels.
fn expected(brightness: f64, contrast: f64, saturation: f64) -> [u8; 3] {
    let step = |color: [f64; 3], f: &dyn Fn(f64) -> f64| color.map(|c| f(c).clamp(0.0, 1.0));
    let mut color = COLOR.map(|channel| f64::from(channel) / 255.0);
    color = step(color, &|c| c + brightness / 200.0);
    color = step(color, &|c| (c - 0.5) * (1.0 + contrast / 100.0) + 0.5);
    let luma = 0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2];
    color = step(color, &|c| luma + (c - luma) * (1.0 + saturation / 100.0));
    color.map(|c| (c * 255.0).round() as u8)
}

fn assert_near(actual: [u8; 3], wanted: [u8; 3], tolerance: u8, label: &str) {
    assert!(
        actual
            .iter()
            .zip(wanted)
            .all(|(a, w)| a.abs_diff(w) <= tolerance),
        "{label}: got {actual:?}, want {wanted:?} (±{tolerance})"
    );
}

#[test]
fn the_monitor_frame_is_graded_and_back_at_zero_is_the_ungraded_picture() {
    let Some(mut rig) = rig("frame") else {
        return;
    };
    let plain = rig.frame("plain");
    assert_near(title_color(&plain), COLOR, 1, "ungraded");
    for (brightness, contrast, saturation) in [
        (40.0, 0.0, 0.0),
        (-60.0, 0.0, 0.0),
        (0.0, 50.0, 0.0),
        (0.0, -100.0, 0.0),
        (0.0, 0.0, -100.0),
        (0.0, 0.0, 80.0),
        (20.0, 30.0, -40.0),
    ] {
        rig.grade(brightness, contrast, saturation);
        let graded = rig.frame("graded");
        let label =
            format!("brightness {brightness}, contrast {contrast}, saturation {saturation}");
        assert_near(
            title_color(&graded),
            expected(brightness, contrast, saturation),
            1,
            &label,
        );
        assert_ne!(graded, plain, "{label} changes the picture");
    }
    rig.grade(0.0, 0.0, 0.0);
    assert_eq!(
        rig.frame("reset"),
        plain,
        "all three back at 0 is the ungraded picture, byte for byte"
    );
}

#[test]
fn the_exported_video_carries_the_grading() {
    let Some(mut rig) = rig("export") else {
        return;
    };
    let (brightness, contrast, saturation) = (-40.0, 20.0, -100.0);
    rig.grade(brightness, contrast, saturation);
    rig.ok(json!({"op":"project.save","path":rig.dir.join("project/grading.tessel.json")}));

    let config = RuntimeConfig::discover().unwrap();
    let factory: Arc<RendererFactory> = Arc::new(move || Ok(Box::new(session(&config))));
    let mut host = MediaHost::new().with_renderer(factory);
    let target = rig.dir.join("out/graded.mp4");
    let report = headless::execute_ext(
        &mut rig.state,
        None,
        Some(&mut host),
        &json!({"op":"export.start","preset":{"id":"p","name":"P","width":WIDTH,"height":HEIGHT},
            "destination":target,"range":[0, 4]}),
    )
    .unwrap_or_else(|error| panic!("export: {error:?}"));
    assert_eq!(report["verification"]["ok"], true, "{report}");

    let video = decode(
        &rig.tools,
        &target,
        &[
            "-vf",
            "select=eq(n\\,2),scale=in_range=tv:out_range=pc",
            "-frames:v",
            "1",
        ],
    );
    let color = title_color(&video);
    let wanted = expected(brightness, contrast, saturation);
    // H.264 4:2:0 and the limited-range round trip move a solid colour by a few levels.
    assert_near(color, wanted, 6, "exported grey");
    assert!(
        COLOR.iter().zip(color).any(|(c, v)| c.abs_diff(v) > 40),
        "the export is not the ungraded colour: {color:?}"
    );
}
