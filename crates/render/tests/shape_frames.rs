//! F067: the rectangle clip drawn by the real runtime (Node worker + headless Chrome): the monitor
//! frame (`render.frame`, PNG) and the exported H.264 video carry the shape's size, fill and corner
//! radius, an edit through the contract changes the picture and undo brings it back byte for byte,
//! and a saved and reopened project draws the same pixels. Skipped without the runtime unless
//! `TESSEL_REQUIRE_RUNTIME=1`; FFmpeg reads the pixels back (`docs/media-setup.md`).
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
const FILL: [u8; 3] = [200, 100, 50];

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
    let tools = Tools::discover().expect("FFmpeg is required for the shape tests");
    let dir = std::env::temp_dir().join(format!("tessel-shape-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut rig = Rig {
        state: EditorState::demo(),
        session: Box::new(session(&config)),
        tools,
        dir,
    };
    // 50% x 40% of 320x180 is 160x72, centred on whole pixels: x 80..240, y 54..126.
    for request in [
        json!({"op":"project.create","name":"Shapes","sequence":"main","rate":24,
            "width":WIDTH,"height":HEIGHT,"length":48}),
        json!({"op":"track.add","id":"overlay","name":"Overlay","kind":"video"}),
        json!({"op":"clip.add","id":"box","track":"overlay","start":0,"duration":48,
            "shape":{"kind":"rect","width":50,"height":40,"fill":"#c86432","radius":0}}),
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

    fn set(&mut self, field: &str, value: Value) {
        self.ok(json!({"op":"set","target":"clip_shape:box","field":field,"value":value}));
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

fn at(rgb: &[u8], x: u32, y: u32) -> [u8; 3] {
    let offset = ((y * WIDTH + x) * 3) as usize;
    [rgb[offset], rgb[offset + 1], rgb[offset + 2]]
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

const BLACK: [u8; 3] = [0, 0, 0];

#[test]
fn the_monitor_frame_draws_the_rectangle_and_an_edit_and_undo_change_it() {
    let Some(mut rig) = rig("frame") else {
        return;
    };
    let square = rig.frame("square");
    assert_near(at(&square, 160, 90), FILL, 1, "centre");
    assert_near(at(&square, 80, 54), FILL, 1, "first pixel of the rectangle");
    assert_near(
        at(&square, 239, 125),
        FILL,
        1,
        "last pixel of the rectangle",
    );
    assert_near(at(&square, 79, 90), BLACK, 1, "left of the rectangle");
    assert_near(at(&square, 240, 90), BLACK, 1, "right of the rectangle");
    assert_near(at(&square, 160, 53), BLACK, 1, "above the rectangle");
    assert_near(at(&square, 160, 126), BLACK, 1, "below the rectangle");

    // 10% of the canvas height is an 18px radius: the very corner is cut, the edge middle is not.
    rig.set("radius", json!(10));
    let rounded = rig.frame("rounded");
    assert_near(at(&rounded, 80, 54), BLACK, 1, "rounded corner");
    assert_near(at(&rounded, 160, 54), FILL, 1, "straight edge");
    assert_near(at(&rounded, 160, 90), FILL, 1, "centre");
    assert_ne!(rounded, square);

    rig.set("fill", json!("#2864c8"));
    assert_near(
        at(&rig.frame("blue"), 160, 90),
        [40, 100, 200],
        1,
        "new fill",
    );

    // Position and size come from the common properties and the spec.
    rig.set("width", json!(25));
    rig.ok(json!({"op":"clip.property","clip":"box","property":"position_x","value":50}));
    let moved = rig.frame("moved");
    // 25% of 320 is 80px wide, shifted right by half the canvas: x 240..320 (clipped to 280).
    assert_near(at(&moved, 280, 90), [40, 100, 200], 1, "moved shape");
    assert_near(at(&moved, 160, 90), BLACK, 1, "where it used to be");

    for _ in 0..4 {
        rig.ok(json!({"op":"history.undo"}));
    }
    assert_eq!(
        rig.frame("undone"),
        square,
        "undo restores the first picture, byte for byte"
    );
}

#[test]
fn a_reopened_project_draws_the_same_pixels() {
    let Some(mut rig) = rig("reopen") else {
        return;
    };
    rig.set("radius", json!(12.5));
    let before = rig.frame("before");
    let path = rig.dir.join("project/shapes.tessel.json");
    rig.ok(json!({"op":"project.save","path":path}));
    let document = rig.state.to_document(None);
    rig.state = EditorState::from_document(&document, None).unwrap();
    assert_eq!(rig.frame("after"), before);
}

#[test]
fn the_exported_video_carries_the_shape() {
    let Some(mut rig) = rig("export") else {
        return;
    };
    rig.set("radius", json!(10));
    rig.ok(json!({"op":"project.save","path":rig.dir.join("project/shapes.tessel.json")}));

    let config = RuntimeConfig::discover().unwrap();
    let factory: Arc<RendererFactory> = Arc::new(move || Ok(Box::new(session(&config))));
    let mut host = MediaHost::new().with_renderer(factory);
    let target = rig.dir.join("out/shape.mp4");
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
    // H.264 4:2:0 and the limited-range round trip move a solid colour by a few levels.
    assert_near(at(&video, 160, 90), FILL, 8, "exported fill");
    assert_near(at(&video, 20, 20), BLACK, 8, "exported background");
    assert_near(
        at(&video, 160, 40),
        BLACK,
        8,
        "above the exported rectangle",
    );
    assert_near(at(&video, 82, 56), BLACK, 40, "exported rounded corner");
}
