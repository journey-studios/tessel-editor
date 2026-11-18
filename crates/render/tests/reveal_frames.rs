//! F072 (BL169.6 step 5): the reveal of a clip and the light beam that rides its edge, drawn by the
//! real runtime (Node worker + headless Chrome). A shot is revealed over the previous one by a
//! diagonal edge (the 23 s reference of the presentation, scaled to a 320x180 canvas), and the
//! beam is nothing but an ordinary group of two rotated rectangles with a multi-stop gradient and
//! keyed positions. Proven on the monitor frame (`render.frame`, PNG) before, during and after
//! the sweep, on the exported H.264 video, with undo byte for byte, with a saved and reopened
//! project, and against a clip that never had a reveal. Skipped without the runtime unless
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
const ORANGE: [u8; 3] = [200, 100, 50];
const GREEN: [u8; 3] = [50, 200, 100];
/// The shot starts here; the sweep takes 9 frames (a 24 fps copy of the reference's 9 at 30).
const SHOT_START: i64 = 24;
const SWEEP: i64 = 9;
/// The reference's geometry at 1920x1080, in units of that canvas; the canvas here is 1/6 of it.
const REF_WIDTH: f64 = 1920.0;
const REF_FROM: f64 = -300.0;
const REF_TO: f64 = 2229.0;
const REF_LEAN: f64 = 118.0 / 540.0;

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
    let tools = Tools::discover().expect("FFmpeg is required for the reveal tests");
    let dir = std::env::temp_dir().join(format!("tessel-reveal-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut rig = Rig {
        state: EditorState::demo(),
        session: Box::new(session(&config)),
        tools,
        dir,
    };
    for request in [
        json!({"op":"project.create","name":"Reveal","sequence":"main","rate":24,
            "width":WIDTH,"height":HEIGHT,"length":72}),
        json!({"op":"track.add","id":"back","name":"Back","kind":"video"}),
        json!({"op":"track.add","id":"mid","name":"Mid","kind":"video"}),
        json!({"op":"track.add","id":"top","name":"Top","kind":"video"}),
        // The previous shot: the whole canvas, orange, for the whole sequence.
        json!({"op":"clip.add","id":"previous","track":"back","start":0,"duration":72,
            "shape":{"kind":"rect","width":100,"height":100,"fill":"#c86432","radius":0}}),
        // The incoming shot: the whole canvas, green, from frame 24.
        json!({"op":"clip.add","id":"shot","track":"mid","start":SHOT_START,"duration":48,
            "shape":{"kind":"rect","width":100,"height":100,"fill":"#32c864","radius":0}}),
    ] {
        rig.ok(request);
    }
    Some(rig)
}

fn percent_of_width(reference_x: f64) -> f64 {
    reference_x / REF_WIDTH * 100.0
}

impl Rig {
    fn ok(&mut self, request: Value) -> Value {
        headless::execute_with(&mut self.state, Some(self.session.as_mut()), &request)
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    fn property(&mut self, clip: &str, property: &str, value: Value) {
        self.ok(json!({"op":"clip.property","clip":clip,"property":property,"value":value}));
    }

    /// The reveal of the reference on the shot: the edge leans atan(118 / 540) and runs from
    /// x = -300 to x = 2229 (1920 wide) over 9 frames with smoothStep.
    fn reveal(&mut self) {
        for (property, value) in [
            ("reveal_mode", json!("wipe")),
            ("reveal_from", json!(percent_of_width(REF_FROM))),
            ("reveal_to", json!(percent_of_width(REF_TO))),
            ("reveal_angle", json!(REF_LEAN.atan().to_degrees())),
            ("reveal_duration", json!(SWEEP)),
        ] {
            self.property("shot", property, value);
        }
    }

    /// The beam of the reference as an editable composition of common elements: a soft strip and
    /// a thin white line, both rotated by the lean of the edge, both keyed from the same start
    /// to the same end with the same easing as the reveal, so they ride the edge.
    fn beam(&mut self) {
        let cosine = REF_LEAN.atan().cos();
        // At mid height the sheared strip of the reference spans [x - 210, x + 1] and its line
        // [x + 2, x + 5]: their centres follow the edge at x - 104.5 and x + 3.5.
        let ride = |offset: f64| {
            let sweep_ms = SWEEP * 1000 / 24;
            json!([{"prop":"x","keys":[
                {"time_ms":0,"value":percent_of_width(REF_FROM + offset - REF_WIDTH / 2.0),
                 "easing":"smoothStep"},
                {"time_ms":sweep_ms,"value":percent_of_width(REF_TO + offset - REF_WIDTH / 2.0)},
            ]}])
        };
        let lean = REF_LEAN.atan().to_degrees();
        let glow_width = percent_of_width(211.0 * cosine);
        let line_width = percent_of_width(3.0 * cosine);
        self.ok(json!({"op":"group.define","definition":{"id":"beam","name":"Beam","children":[
            {"id":"glow","kind":"shape","rotation":lean,"tracks":ride(-104.5),
             "shape":{"kind":"rect","width":glow_width,"height":130,"radius":0,
                "fill":"#c6a7ff00","fill_to":"#c6a7ff00","fill_mode":"linear","gradient_angle":0,
                "stops":"0.58 #c6a7ff60 0.92 #c6a7fffa"}},
            {"id":"line","kind":"shape","rotation":lean,"tracks":ride(3.5),
             "shape":{"kind":"rect","width":line_width,"height":130,"radius":0,
                "fill":"#ffffff"}},
        ]}}));
        self.ok(
            json!({"op":"clip.add","id":"beam-clip","track":"top","start":SHOT_START,
            "duration":SWEEP + 1,"group":{"def":"beam"}}),
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

/// Pixels of the frame for which `test` holds.
fn count(rgb: &[u8], test: impl Fn([u8; 3]) -> bool) -> usize {
    rgb.as_chunks::<3>()
        .0
        .iter()
        .filter(|pixel| test(**pixel))
        .count()
}

/// Where the reference's edge is at mid height on frame `local` of the sweep, on this canvas.
fn edge_at(local: i64) -> f64 {
    let t = local as f64 / SWEEP as f64;
    let eased = t * t * (3.0 - 2.0 * t);
    (REF_FROM + (REF_TO - REF_FROM) * eased) * f64::from(WIDTH) / REF_WIDTH
}

/// The edge on pixel row `y`: it leans, the top leading.
fn edge_on_row(local: i64, y: u32) -> f64 {
    edge_at(local) + REF_LEAN * (f64::from(HEIGHT) / 2.0 - f64::from(y))
}

fn is_orange(pixel: [u8; 3]) -> bool {
    pixel.iter().zip(ORANGE).all(|(a, w)| a.abs_diff(w) <= 2)
}

fn is_green(pixel: [u8; 3]) -> bool {
    pixel.iter().zip(GREEN).all(|(a, w)| a.abs_diff(w) <= 2)
}

#[test]
fn the_shot_is_uncovered_by_the_edge_before_during_and_after_the_sweep() {
    let Some(mut rig) = rig("sweep") else {
        return;
    };
    rig.reveal();
    rig.beam();

    // Before the shot there is only the previous one.
    let before = rig.frame("before", SHOT_START - 4);
    assert_eq!(count(&before, is_orange), (WIDTH * HEIGHT) as usize);

    // The first frame of the shot: the edge is still off the left, so nothing of it shows and
    // neither does the beam: the picture is the previous shot's, byte for byte.
    let first = rig.frame("first", SHOT_START);
    assert_eq!(first, before, "nothing is revealed yet");

    // During: frame 4 of 9. Green behind the edge, orange ahead of it, on a leaning line.
    let local = 4;
    let during = rig.frame("during", SHOT_START + local);
    for y in [20_u32, 90, 160] {
        let edge = edge_on_row(local, y);
        // The beam trails the edge by 35 px (211 of the reference's 1920).
        let behind = edge - 40.0;
        let ahead = edge + 6.0;
        assert_near(
            at(&during, behind as u32, y),
            GREEN,
            2,
            &format!("behind the beam, row {y}"),
        );
        assert_near(
            at(&during, ahead as u32, y),
            ORANGE,
            2,
            &format!("ahead of the edge, row {y}"),
        );
    }
    // The beam is lilac light on the green, brightest next to the edge, and only there.
    let edge = edge_on_row(local, 90);
    let lit = at(&during, (edge - 4.0) as u32, 90);
    assert!(
        lit[0] > 120 && lit[2] > 150,
        "the beam lightens the green towards lilac: {lit:?}"
    );
    assert_ne!(lit, GREEN);
    assert!(
        !is_green(at(&during, (edge - 20.0) as u32, 90)),
        "the glow fades in gradually behind the edge"
    );

    // After: the whole canvas is the shot and the beam is gone.
    let after = rig.frame("after", SHOT_START + SWEEP + 3);
    assert_eq!(count(&after, is_green), (WIDTH * HEIGHT) as usize);
}

#[test]
fn a_finished_reveal_is_the_picture_of_a_clip_that_never_had_one_and_undo_is_exact() {
    let Some(mut rig) = rig("compat") else {
        return;
    };
    let plain_before = rig.frame("plain-before", SHOT_START - 2);
    let plain_during = rig.frame("plain-during", SHOT_START + 4);
    let plain_after = rig.frame("plain-after", SHOT_START + 14);
    assert_eq!(count(&plain_during, is_green), (WIDTH * HEIGHT) as usize);

    rig.reveal();
    let during = rig.frame("during", SHOT_START + 4);
    assert_ne!(during, plain_during, "the edge changes the picture");
    assert_eq!(
        rig.frame("after", SHOT_START + 14),
        plain_after,
        "a finished reveal draws exactly the plain clip"
    );
    assert_eq!(rig.frame("before", SHOT_START - 2), plain_before);

    // An edit through the contract changes the picture; undo brings it back byte for byte.
    rig.property("shot", "reveal_angle", json!(30));
    let steeper = rig.frame("steeper", SHOT_START + 4);
    assert_ne!(steeper, during, "the lean moved the edge");
    rig.ok(json!({"op":"history.undo"}));
    assert_eq!(rig.frame("steeper-undone", SHOT_START + 4), during);

    // Turning it off is the plain picture again, in the middle of the sweep too.
    rig.property("shot", "reveal_mode", json!("none"));
    assert_eq!(rig.frame("off", SHOT_START + 4), plain_during);
    rig.ok(json!({"op":"history.undo"}));
    assert_eq!(rig.frame("on-again", SHOT_START + 4), during);

    // Easing and delay are choices of the same section: another curve is another picture.
    let smooth = rig.frame("smooth", SHOT_START + 2);
    rig.property("shot", "reveal_easing", json!("linear"));
    let linear = rig.frame("linear", SHOT_START + 2);
    assert!(
        linear != smooth,
        "frame 2 of 9 is further along with a linear curve than with smoothStep"
    );
}

#[test]
fn a_reopened_project_draws_the_same_pixels() {
    let Some(mut rig) = rig("reopen") else {
        return;
    };
    rig.reveal();
    rig.beam();
    let frames = [
        SHOT_START - 2,
        SHOT_START,
        SHOT_START + 2,
        SHOT_START + 5,
        SHOT_START + 14,
    ];
    let before: Vec<Vec<u8>> = frames
        .iter()
        .map(|frame| rig.frame(&format!("before-{frame}"), *frame))
        .collect();
    let path = rig.dir.join("project/reveal.tessel.json");
    rig.ok(json!({"op":"project.save","path":path}));
    let document = rig.state.to_document(None);
    let text = document.to_string();
    for key in [
        "reveal_mode",
        "reveal_from",
        "reveal_to",
        "reveal_angle",
        "stops",
    ] {
        assert!(text.contains(key), "{key} is saved");
    }
    rig.state = EditorState::from_document(&document, None).unwrap();
    for (index, frame) in frames.iter().enumerate() {
        assert_eq!(
            rig.frame(&format!("after-{frame}"), *frame),
            before[index],
            "frame {frame}"
        );
    }
}

#[test]
fn the_exported_video_carries_the_edge_and_the_beam() {
    let Some(mut rig) = rig("export") else {
        return;
    };
    rig.reveal();
    rig.beam();
    rig.ok(json!({"op":"project.save","path":rig.dir.join("project/reveal.tessel.json")}));

    let config = RuntimeConfig::discover().unwrap();
    let factory: Arc<RendererFactory> = Arc::new(move || Ok(Box::new(session(&config))));
    let mut host = MediaHost::new().with_renderer(factory);
    let target = rig.dir.join("out/reveal.mp4");
    let report = headless::execute_ext(
        &mut rig.state,
        None,
        Some(&mut host),
        &json!({"op":"export.start","preset":{"id":"p","name":"P","width":WIDTH,"height":HEIGHT},
            "destination":target,"range":[0, 48]}),
    )
    .unwrap_or_else(|error| panic!("export: {error:?}"));
    assert_eq!(report["verification"]["ok"], true, "{report}");

    let frame = |n: i64| {
        decode(
            &rig.tools,
            &target,
            &[
                "-vf",
                &format!("select=eq(n\\,{n}),scale=in_range=tv:out_range=pc"),
                "-frames:v",
                "1",
            ],
        )
    };
    // H.264 4:2:0 and the limited-range round trip move a solid colour by a few levels.
    let before = frame(SHOT_START - 2);
    assert_near(
        at(&before, 160, 90),
        ORANGE,
        10,
        "exported, before the shot",
    );

    let local = 4;
    let during = frame(SHOT_START + local);
    for y in [20_u32, 160] {
        let edge = edge_on_row(local, y);
        assert_near(
            at(&during, (edge - 42.0) as u32, y),
            GREEN,
            12,
            &format!("exported, behind the beam, row {y}"),
        );
        assert_near(
            at(&during, (edge + 8.0) as u32, y),
            ORANGE,
            12,
            &format!("exported, ahead of the edge, row {y}"),
        );
    }
    let after = frame(SHOT_START + SWEEP + 3);
    assert_near(at(&after, 160, 90), GREEN, 10, "exported, after the sweep");
    assert_near(
        at(&after, 4, 4),
        GREEN,
        10,
        "exported, corner after the sweep",
    );
    assert_near(
        at(&after, 315, 175),
        GREEN,
        10,
        "exported, far corner after the sweep",
    );
}
