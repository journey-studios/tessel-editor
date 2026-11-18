//! F068 (BL169.6 step 1B): the styled shapes and the rounded image mask drawn by the real runtime
//! (Node worker + headless Chrome). One project holds a card (rounded rectangle, linear gradient,
//! contour, hard shadow), a halo (ellipse, radial gradient) and a cropped cover (a still with a
//! rounded mask), each on its own frames over a flat backdrop. The monitor frame (`render.frame`,
//! PNG) and the exported H.264 carry the same pixels, an edit changes the picture and undo brings
//! it back byte for byte, and a reopened project draws the same bytes. Skipped without the runtime
//! unless `TESSEL_REQUIRE_RUNTIME=1`; FFmpeg reads the pixels back (`docs/media-setup.md`).
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{QUADRANT_COLORS, quadrants_png};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::{EditorState, headless};
use tessel_editor_media::host::{MediaHost, RendererFactory};
use tessel_editor_media::tools::Tools;
use tessel_editor_render::{RuntimeConfig, session};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
/// The flat backdrop every shape sits on, so a shadow, a cut corner or a fading edge shows as a
/// colour that is not black.
const BACKDROP: [u8; 3] = [30, 48, 80];
/// One frame inside each clip's range: the card, the halo and the cover.
const CARD_FRAME: i64 = 3;
const HALO_FRAME: i64 = 9;
const COVER_FRAME: i64 = 15;

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
    let dir = std::env::temp_dir().join(format!("tessel-style-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cover = dir.join("cover.png");
    std::fs::write(&cover, quadrants_png(64, 36)).unwrap();
    let mut rig = Rig {
        state: EditorState::demo(),
        session: Box::new(session(&config)),
        tools,
        dir,
    };
    for request in [
        json!({"op":"project.create","name":"Styles","sequence":"main","rate":24,
            "width":WIDTH,"height":HEIGHT,"length":18}),
        json!({"op":"track.add","id":"back","name":"Back","kind":"video"}),
        json!({"op":"track.add","id":"front","name":"Front","kind":"video"}),
        // The backdrop: the whole canvas, one flat colour.
        json!({"op":"clip.add","id":"backdrop","track":"back","start":0,"duration":18,
            "shape":{"kind":"rect","width":100,"height":100,"fill":"#1e3050","radius":0}}),
        // 60% x 50% is 192x90 centred: x 64..256, y 45..135. The radius is 10% of the height
        // (18px), the contour 5% (9px, centred on the edge), the shadow is 16px right, 18px down.
        json!({"op":"clip.add","id":"card","track":"front","start":0,"duration":6,
            "shape":{"kind":"rect","width":60,"height":50,"radius":10,
                "fill":"#ff0000","fill_mode":"linear","fill_to":"#0000ff","gradient_angle":0,
                "stroke":"#ffffff","stroke_width":5,
                "shadow":"#000000","shadow_blur":0,"shadow_x":5,"shadow_y":10}}),
        // 40% x 40% is 128x72: x 96..224, y 54..126.
        json!({"op":"clip.add","id":"halo","track":"front","start":6,"duration":6,
            "shape":{"kind":"ellipse","width":40,"height":40,"fill":"#ffd166",
                "fill_mode":"radial","fill_to":"#ffd16600"}}),
    ] {
        rig.ok(request);
    }
    rig.ok(json!({"op":"asset.add","kind":"still","id":"cover","name":"Cover","path":cover}));
    rig.ok(
        json!({"op":"clip.add","id":"cover-clip","track":"front","asset":"cover",
        "start":12,"duration":6}),
    );
    // A quarter off each side leaves the central half of the picture: x 80..240, full height.
    for (property, value) in [
        ("crop_left", json!(25)),
        ("crop_right", json!(25)),
        ("mask_shape", json!("rounded")),
        ("mask_radius", json!(20)),
    ] {
        rig.ok(json!({"op":"clip.property","clip":"cover-clip","property":property,"value":value}));
    }
    Some(rig)
}

impl Rig {
    fn ok(&mut self, request: Value) -> Value {
        headless::execute_with(&mut self.state, Some(self.session.as_mut()), &request)
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    fn shape(&mut self, clip: &str, field: &str, value: Value) {
        self.ok(
            json!({"op":"set","target":format!("clip_shape:{clip}"),"field":field,"value":value}),
        );
    }

    /// The monitor frame as RGB, read back from the PNG `render.frame` wrote.
    fn frame(&mut self, name: &str, frame: i64) -> Vec<u8> {
        let png = self.dir.join(format!("{name}.png"));
        self.ok(json!({"op":"render.frame","frame":frame,"out":png}));
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

/// What the three pictures must hold, to `tolerance` levels per channel (a video round trip moves
/// a flat colour by a few levels and a gradient or an edge by more).
fn assert_card(rgb: &[u8], tolerance: u8, label: &str) {
    assert_near(
        at(rgb, 20, 20),
        BACKDROP,
        tolerance,
        &format!("{label} backdrop"),
    );
    // Left to right, red to blue, over the declared size (x 64..256).
    let left = at(rgb, 80, 90);
    assert!(
        left[0] > 190 && left[2] < 60,
        "{label} card left is red: {left:?}"
    );
    let middle = at(rgb, 160, 90);
    assert_near(
        middle,
        [127, 0, 128],
        tolerance + 12,
        &format!("{label} card middle"),
    );
    let right = at(rgb, 240, 90);
    assert!(
        right[2] > 190 && right[0] < 60,
        "{label} card right is blue: {right:?}"
    );
    // The contour is white and centred on the edge: half of it lies outside the declared size.
    assert_near(
        at(rgb, 61, 90),
        [255, 255, 255],
        tolerance + 8,
        &format!("{label} contour, outside half"),
    );
    assert_near(
        at(rgb, 67, 90),
        [255, 255, 255],
        tolerance + 8,
        &format!("{label} contour, inside half"),
    );
    assert_near(
        at(rgb, 57, 90),
        BACKDROP,
        tolerance + 8,
        &format!("{label} left of the contour: no shadow there"),
    );
    // The corner is rounded: the pixel outside the arc is the backdrop, not the contour.
    assert_near(
        at(rgb, 62, 43),
        BACKDROP,
        tolerance + 12,
        &format!("{label} rounded corner"),
    );
    // The hard black shadow follows the silhouette 16px right and 18px down.
    let right_shadow = at(rgb, 270, 100);
    assert!(
        right_shadow.iter().all(|c| *c < 12 + tolerance),
        "{label} shadow right: {right_shadow:?}"
    );
    let bottom_shadow = at(rgb, 160, 150);
    assert!(
        bottom_shadow.iter().all(|c| *c < 12 + tolerance),
        "{label} shadow below: {bottom_shadow:?}"
    );
    assert_near(
        at(rgb, 300, 100),
        BACKDROP,
        tolerance,
        &format!("{label} past the shadow"),
    );
    assert_near(
        at(rgb, 160, 172),
        BACKDROP,
        tolerance,
        &format!("{label} under the shadow"),
    );
}

fn assert_halo(rgb: &[u8], tolerance: u8, label: &str) {
    assert_near(
        at(rgb, 160, 90),
        [255, 209, 102],
        tolerance + 6,
        &format!("{label} halo centre"),
    );
    let half = at(rgb, 125, 90);
    assert!(
        half[0] > 100 && half[0] < 190 && half[1] > 80 && half[1] < 160,
        "{label} half way out the halo is a blend of the glow and the backdrop: {half:?}"
    );
    assert_near(
        at(rgb, 99, 90),
        BACKDROP,
        tolerance + 24,
        &format!("{label} halo edge fades out"),
    );
    assert_near(
        at(rgb, 98, 56),
        BACKDROP,
        tolerance,
        &format!("{label} box corner is outside the ellipse"),
    );
    assert_near(
        at(rgb, 60, 90),
        BACKDROP,
        tolerance,
        &format!("{label} outside the halo"),
    );
}

fn assert_cover(rgb: &[u8], tolerance: u8, label: &str) {
    // The crop keeps x 80..240: the four quadrants are the picture's, around the canvas centre.
    assert_near(
        at(rgb, 120, 45),
        QUADRANT_COLORS[0],
        tolerance,
        &format!("{label} top left"),
    );
    assert_near(
        at(rgb, 200, 45),
        QUADRANT_COLORS[1],
        tolerance,
        &format!("{label} top right"),
    );
    assert_near(
        at(rgb, 120, 135),
        QUADRANT_COLORS[2],
        tolerance,
        &format!("{label} bottom left"),
    );
    assert_near(
        at(rgb, 200, 135),
        QUADRANT_COLORS[3],
        tolerance,
        &format!("{label} bottom right"),
    );
    // Outside the crop nothing of the picture shows.
    assert_near(
        at(rgb, 60, 90),
        BACKDROP,
        tolerance,
        &format!("{label} left of the crop"),
    );
    assert_near(
        at(rgb, 255, 90),
        BACKDROP,
        tolerance,
        &format!("{label} right of the crop"),
    );
    // The mask rounds the CROPPED rectangle (20% of its shorter side, 160px: 32px): the corner of
    // the crop is the backdrop, the pixel inside the arc is the picture.
    assert_near(
        at(rgb, 81, 2),
        BACKDROP,
        tolerance + 12,
        &format!("{label} cut corner"),
    );
    assert_near(
        at(rgb, 95, 15),
        QUADRANT_COLORS[0],
        tolerance,
        &format!("{label} inside the arc"),
    );
    assert_near(
        at(rgb, 120, 2),
        QUADRANT_COLORS[0],
        tolerance,
        &format!("{label} straight top edge"),
    );
}

#[test]
fn the_monitor_frames_draw_the_card_the_halo_and_the_cropped_cover() {
    let Some(mut rig) = rig("frames") else {
        return;
    };
    let card = rig.frame("card", CARD_FRAME);
    assert_card(&card, 1, "png");
    let halo = rig.frame("halo", HALO_FRAME);
    assert_halo(&halo, 1, "png");
    let cover = rig.frame("cover", COVER_FRAME);
    assert_cover(&cover, 2, "png");
    assert_ne!(card, halo);
    assert_ne!(halo, cover);
}

#[test]
fn an_edit_changes_the_picture_and_undo_brings_it_back_byte_for_byte() {
    let Some(mut rig) = rig("undo") else {
        return;
    };
    let card = rig.frame("card", CARD_FRAME);
    let halo = rig.frame("halo", HALO_FRAME);
    let cover = rig.frame("cover", COVER_FRAME);

    rig.shape("card", "shadow_x", json!(-5));
    let shifted = rig.frame("shifted", CARD_FRAME);
    assert_near(
        at(&shifted, 270, 100),
        BACKDROP,
        1,
        "the shadow left the right side",
    );
    assert!(
        at(&shifted, 50, 100).iter().all(|c| *c < 12),
        "and fell on the left: {:?}",
        at(&shifted, 50, 100)
    );

    rig.shape("halo", "fill_mode", json!("solid"));
    assert_near(
        at(&rig.frame("solid", HALO_FRAME), 99, 90),
        [255, 209, 102],
        2,
        "no gradient: the edge keeps the colour",
    );

    rig.ok(
        json!({"op":"clip.property","clip":"cover-clip","property":"mask_shape","value":"ellipse"}),
    );
    let ellipse = rig.frame("ellipse", COVER_FRAME);
    assert_ne!(ellipse, cover, "an ellipse mask is another picture");
    assert_near(
        at(&ellipse, 100, 20),
        BACKDROP,
        2,
        "the ellipse cuts further in than the rounded corner",
    );

    rig.ok(
        json!({"op":"clip.property","clip":"cover-clip","property":"mask_shape","value":"none"}),
    );
    let plain = rig.frame("plain", COVER_FRAME);
    assert_near(
        at(&plain, 81, 2),
        QUADRANT_COLORS[0],
        2,
        "no mask: the crop is a plain rectangle",
    );

    for _ in 0..4 {
        rig.ok(json!({"op":"history.undo"}));
    }
    assert_eq!(
        rig.frame("card-undone", CARD_FRAME),
        card,
        "the card, byte for byte"
    );
    assert_eq!(
        rig.frame("halo-undone", HALO_FRAME),
        halo,
        "the halo, byte for byte"
    );
    assert_eq!(
        rig.frame("cover-undone", COVER_FRAME),
        cover,
        "the cover, byte for byte"
    );
}

#[test]
fn a_reopened_project_draws_the_same_pixels() {
    let Some(mut rig) = rig("reopen") else {
        return;
    };
    let before: Vec<Vec<u8>> = [CARD_FRAME, HALO_FRAME, COVER_FRAME]
        .iter()
        .map(|frame| rig.frame(&format!("before-{frame}"), *frame))
        .collect();
    let path = rig.dir.join("project/styles.tessel.json");
    rig.ok(json!({"op":"project.save","path":path}));
    let document = rig.state.to_document(None);
    // The document keeps every style field the project moved off its default (the angle 0 of the
    // card is the default, so it is not written), so reading it back is exact.
    let text = document.to_string();
    for key in [
        "fill_mode",
        "fill_to",
        "stroke_width",
        "shadow_y",
        "mask_shape",
        "mask_radius",
    ] {
        assert!(text.contains(key), "{key} is saved");
    }
    rig.state = EditorState::from_document(&document, None).unwrap();
    for (index, frame) in [CARD_FRAME, HALO_FRAME, COVER_FRAME].iter().enumerate() {
        assert_eq!(
            rig.frame(&format!("after-{frame}"), *frame),
            before[index],
            "frame {frame}"
        );
    }
}

#[test]
fn the_exported_video_carries_the_card_the_halo_and_the_cover() {
    let Some(mut rig) = rig("export") else {
        return;
    };
    rig.ok(json!({"op":"project.save","path":rig.dir.join("project/styles.tessel.json")}));

    let config = RuntimeConfig::discover().unwrap();
    let factory: Arc<RendererFactory> = Arc::new(move || Ok(Box::new(session(&config))));
    let mut host = MediaHost::new().with_renderer(factory);
    let target = rig.dir.join("out/styles.mp4");
    let report = headless::execute_ext(
        &mut rig.state,
        None,
        Some(&mut host),
        &json!({"op":"export.start","preset":{"id":"p","name":"P","width":WIDTH,"height":HEIGHT},
            "destination":target,"range":[0, 18]}),
    )
    .unwrap_or_else(|error| panic!("export: {error:?}"));
    assert_eq!(report["verification"]["ok"], true, "{report}");

    let video = |frame: i64| {
        decode(
            &rig.tools,
            &target,
            &[
                "-vf",
                &format!("select=eq(n\\,{frame}),scale=in_range=tv:out_range=pc"),
                "-frames:v",
                "1",
            ],
        )
    };
    // H.264 4:2:0 moves flat colours by a few levels and edges, gradients and chroma by more.
    assert_card(&video(CARD_FRAME), 14, "mp4");
    assert_halo(&video(HALO_FRAME), 14, "mp4");
    assert_cover(&video(COVER_FRAME), 16, "mp4");
}
