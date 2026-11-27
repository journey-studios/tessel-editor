//! F069: reusable groups with keyframes, drawn by the real runtime (Node worker + headless Chrome).
//! A card (panel + title with staggered fades) and a step sequence (three shapes that enter one
//! after the other, with position, scale and opacity keys) are proven on the monitor frame
//! (`render.frame`, PNG) and on the exported H.264 video. An edit through the contract changes the
//! picture and undo brings it back byte for byte, and a saved and reopened project draws the same
//! pixels. Skipped without the runtime unless `TESSEL_REQUIRE_RUNTIME=1`; FFmpeg reads the pixels
//! back (`docs/media-setup.md`).
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
const BLUE: [u8; 3] = [50, 100, 200];
const BLACK: [u8; 3] = [0, 0, 0];

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
    let tools = Tools::discover().expect("FFmpeg is required for the group tests");
    let dir = std::env::temp_dir().join(format!("tessel-group-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut rig = Rig {
        state: EditorState::demo(),
        session: Box::new(session(&config)),
        tools,
        dir,
    };
    for request in [
        json!({"op":"project.create","name":"Groups","sequence":"main","rate":24,
            "width":WIDTH,"height":HEIGHT,"length":96}),
        json!({"op":"track.add","id":"overlay","name":"Overlay","kind":"video"}),
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

    /// The monitor frame as RGB, read back from the PNG `render.frame` wrote.
    fn frame(&mut self, name: &str, frame: i64) -> Vec<u8> {
        let png = self.dir.join(format!("{name}.png"));
        self.ok(json!({"op":"render.frame","frame":frame,"out":png}));
        decode(&self.tools, &png, &["-frames:v", "1"])
    }

    /// Three steps that enter one after the other, 500 ms apart. Each fades in over 100 ms; the
    /// first also grows from half to full size over 500 ms. They sit at x -30%, 0 and +30%.
    fn steps(&mut self) {
        self.steps_on("overlay");
    }

    fn steps_on(&mut self, track: &str) {
        let fade = json!({"prop":"opacity","keys":[
            {"time_ms":0,"value":0},{"time_ms":100,"value":100}]});
        let step = |id: &str, x: f64, fill: &str, extra: Vec<Value>| {
            let mut tracks = vec![fade.clone()];
            tracks.extend(extra);
            json!({"id":id,"kind":"shape","x":x,
                "shape":{"kind":"rect","width":20,"height":40,"fill":fill,"radius":0},
                "tracks":tracks})
        };
        let grow = json!({"prop":"scale","keys":[
            {"time_ms":0,"value":50},{"time_ms":500,"value":100}]});
        self.ok(
            json!({"op":"group.define","definition":{"id":"steps","name":"Steps",
            "stagger_ms":500,"children":[
                step("one", -30.0, "#c86432", vec![grow]),
                step("two", 0.0, "#32c864", vec![]),
                step("three", 30.0, "#3264c8", vec![]),
            ]}}),
        );
        self.ok(
            json!({"op":"clip.add","id":"seq","track":track,"start":0,"duration":96,
            "group":{"def":"steps"}}),
        );
    }

    /// A card: a panel and a white title that fade in one after the other, 200 ms apart.
    fn card(&mut self) {
        self.ok(json!({"op":"text_style.set",
            "style":{"id":"title","size":12,"color":"#ffffff","font":"sans"}}));
        let fade = json!([{"prop":"opacity","keys":[
            {"time_ms":0,"value":0},{"time_ms":400,"value":100}]}]);
        self.ok(
            json!({"op":"group.define","definition":{"id":"card","name":"Card",
            "stagger_ms":200,"children":[
                {"id":"panel","kind":"shape",
                 "shape":{"kind":"rect","width":60,"height":30,"fill":"#c86432","radius":0},
                 "tracks":fade},
                {"id":"title","kind":"text","content":"Welcome","style":"title","tracks":fade},
            ]}}),
        );
        self.ok(
            json!({"op":"clip.add","id":"card1","track":"overlay","start":0,"duration":96,
            "group":{"def":"card"}}),
        );
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

fn white(pixel: [u8; 3]) -> bool {
    pixel.iter().all(|channel| *channel > 220)
}

fn yellow(pixel: [u8; 3]) -> bool {
    pixel[0] > 200 && pixel[1] > 150 && pixel[2] < 80
}

/// The centres of the three steps.
const ONE: (u32, u32) = (64, 90);
const TWO: (u32, u32) = (160, 90);
const THREE: (u32, u32) = (256, 90);

#[test]
fn the_card_enters_in_order_and_its_edits_change_the_picture_and_undo_restores_it() {
    let Some(mut rig) = rig("card") else {
        return;
    };
    rig.card();

    // Frame 0: everything is still invisible.
    let hidden = rig.frame("hidden", 0);
    assert_near(at(&hidden, 160, 90), BLACK, 1, "panel before its fade");
    assert_eq!(count(&hidden, white), 0, "no title yet");

    // Frame 6 = 250 ms: the panel is 62.5% in, the title (waiting 200 ms) is only 12.5% in.
    let early = rig.frame("early", 6);
    assert_near(at(&early, 80, 70), [125, 63, 31], 3, "panel at 62.5%");
    assert_eq!(count(&early, white), 0, "the title has barely started");

    // Frame 60 = 2.5 s: both are fully in. The panel is 60% x 30% = 192x54 at x 64..256, y 63..117.
    let done = rig.frame("done", 60);
    assert_near(at(&done, 70, 66), ORANGE, 1, "panel top-left");
    assert_near(at(&done, 250, 114), ORANGE, 1, "panel bottom-right");
    assert_near(at(&done, 60, 66), BLACK, 1, "left of the panel");
    assert_near(at(&done, 160, 120), BLACK, 1, "below the panel");
    let title_pixels = count(&done, white);
    assert!(
        title_pixels > 40,
        "the title is drawn: {title_pixels} white pixels"
    );

    // One clip changes its text without touching the definition: another picture, then undo.
    rig.ok(json!({"op":"clip.group_text","clip":"card1","child":"title","text":"Hi"}));
    let override_frame = rig.frame("override", 60);
    assert_ne!(override_frame, done, "the clip's own text is drawn");
    rig.ok(json!({"op":"history.undo"}));
    assert_eq!(
        rig.frame("override-undone", 60),
        done,
        "undo restores the picture, byte for byte"
    );

    // The style is shared: one edit recolours every group that uses it.
    rig.ok(json!({"op":"set","target":"text_style:title","field":"color","value":"#ffcc00"}));
    let styled = rig.frame("styled", 60);
    assert_eq!(count(&styled, white), 0, "the title is no longer white");
    assert!(count(&styled, yellow) > 40, "the title follows its style");
    rig.ok(json!({"op":"history.undo"}));
    assert_eq!(rig.frame("style-undone", 60), done);

    // A child's field edit moves the pixels: the panel narrows from 60% to 30%.
    rig.ok(json!({"op":"set","target":"group_child:card/panel","field":"width","value":30}));
    let narrow = rig.frame("narrow", 60);
    assert_near(at(&narrow, 70, 66), BLACK, 1, "the panel left its old edge");
    assert_near(
        at(&narrow, 160, 66),
        ORANGE,
        1,
        "the panel is still centred",
    );
    rig.ok(json!({"op":"history.undo"}));
    assert_eq!(rig.frame("narrow-undone", 60), done);
}

#[test]
fn the_step_sequence_enters_one_by_one_with_position_scale_and_opacity() {
    let Some(mut rig) = rig("steps") else {
        return;
    };
    rig.steps();

    let start = rig.frame("start", 0);
    for (name, (x, y)) in [("one", ONE), ("two", TWO), ("three", THREE)] {
        assert_near(
            at(&start, x, y),
            BLACK,
            1,
            &format!("{name} before its turn"),
        );
    }

    // Frame 6 = 250 ms: the first is in and half-way through growing (75%: 48x54 around x 64);
    // the others wait for 500 ms and 1000 ms.
    let first = rig.frame("first", 6);
    assert_near(at(&first, 64, 90), ORANGE, 1, "first, opaque");
    assert_near(at(&first, 44, 90), ORANGE, 1, "inside the grown first step");
    assert_near(at(&first, 36, 90), BLACK, 1, "outside the 75% first step");
    assert_near(at(&first, 160, 90), BLACK, 1, "second waits");
    assert_near(at(&first, 256, 90), BLACK, 1, "third waits");

    // Frame 12 = 500 ms: full size, and the second starts its fade (a 100 ms fade at 0 ms).
    let grown = rig.frame("grown", 12);
    assert_near(
        at(&grown, 36, 90),
        ORANGE,
        1,
        "the first step reached full size",
    );
    assert_near(at(&grown, 160, 90), BLACK, 1, "second at its first key");

    // Frame 18 = 750 ms: the second is fully in, the third still waits.
    let second = rig.frame("second", 18);
    assert_near(at(&second, 160, 90), GREEN, 1, "second");
    assert_near(at(&second, 256, 90), BLACK, 1, "third waits");

    // Frame 36 = 1.5 s: all three, at three positions.
    let all = rig.frame("all", 36);
    assert_near(at(&all, 64, 90), ORANGE, 1, "first");
    assert_near(at(&all, 160, 90), GREEN, 1, "second");
    assert_near(at(&all, 256, 90), BLUE, 1, "third");
    assert_near(
        at(&all, 112, 90),
        BLACK,
        1,
        "gap between the first and the second",
    );

    // Editing the stagger changes when they enter, and undo is exact.
    rig.ok(json!({"op":"group.set","group":"steps","field":"stagger_ms","value":250}));
    let faster = rig.frame("faster", 12);
    assert_near(at(&faster, 160, 90), GREEN, 1, "second enters earlier");
    assert_near(
        at(&faster, 256, 90),
        BLACK,
        1,
        "third has not started at 500 ms",
    );
    rig.ok(json!({"op":"history.undo"}));
    assert_eq!(rig.frame("faster-undone", 12), grown);
}

#[test]
fn a_repeating_rotation_key_track_turns_the_bar_back_and_forth() {
    let Some(mut rig) = rig("spin") else {
        return;
    };
    rig.ok(
        json!({"op":"group.define","definition":{"id":"spin","name":"Spin","children":[
            {"id":"bar","kind":"shape",
             "shape":{"kind":"rect","width":50,"height":40,"fill":"#c86432","radius":0},
             "tracks":[{"prop":"rotation","repeat":0,"mode":"pingpong","keys":[
                 {"time_ms":0,"value":0},{"time_ms":500,"value":90}]}]},
        ]}}),
    );
    rig.ok(
        json!({"op":"clip.add","id":"spinner","track":"overlay","start":0,"duration":96,
        "group":{"def":"spin"}}),
    );

    // The bar is 160x72 at rest (x 80..240, y 54..126) and 72x160 once turned a quarter.
    let flat = rig.frame("flat", 0);
    assert_near(at(&flat, 100, 90), ORANGE, 1, "horizontal bar, left");
    assert_near(at(&flat, 160, 20), BLACK, 1, "horizontal bar, above");

    let upright = rig.frame("upright", 12);
    assert_near(at(&upright, 160, 20), ORANGE, 1, "turned, top");
    assert_near(at(&upright, 100, 90), BLACK, 1, "turned, where the end was");

    // Without repetition the bar would hold at 90 degrees: it is back at 0 after a second cycle.
    let back = rig.frame("back", 24);
    assert_eq!(back, flat, "the second cycle plays backwards to the start");
    assert_eq!(rig.frame("again", 36), upright, "and forwards again");

    // Halfway through a turn the bar is diagonal: the centre stays, the straight ends are gone.
    let diagonal = rig.frame("diagonal", 6);
    assert_near(at(&diagonal, 160, 90), ORANGE, 1, "centre");
    assert_near(
        at(&diagonal, 85, 90),
        BLACK,
        1,
        "the horizontal end left the row",
    );
}

#[test]
fn a_reopened_project_draws_the_same_pixels() {
    let Some(mut rig) = rig("reopen") else {
        return;
    };
    rig.card();
    rig.ok(json!({"op":"track.add","id":"upper","name":"Upper","kind":"video"}));
    rig.steps_on("upper");
    rig.ok(json!({"op":"clip.group_text","clip":"card1","child":"title","text":"Edited"}));
    let before = [rig.frame("a", 6), rig.frame("b", 18), rig.frame("c", 60)];
    let path = rig.dir.join("project/groups.tessel.json");
    rig.ok(json!({"op":"project.save","path":path}));
    let document = rig.state.to_document(None);
    assert!(
        document["groups"].is_object(),
        "the groups travel with the project"
    );
    rig.state = EditorState::from_document(&document, None).unwrap();
    assert_eq!(rig.frame("a2", 6), before[0]);
    assert_eq!(rig.frame("b2", 18), before[1]);
    assert_eq!(rig.frame("c2", 60), before[2]);
}

#[test]
fn the_exported_video_carries_the_step_sequence() {
    let Some(mut rig) = rig("export") else {
        return;
    };
    rig.steps();
    rig.ok(json!({"op":"project.save","path":rig.dir.join("project/groups.tessel.json")}));

    let config = RuntimeConfig::discover().unwrap();
    let factory: Arc<RendererFactory> = Arc::new(move || Ok(Box::new(session(&config))));
    let mut host = MediaHost::new().with_renderer(factory);
    let target = rig.dir.join("out/steps.mp4");
    let report = headless::execute_ext(
        &mut rig.state,
        None,
        Some(&mut host),
        &json!({"op":"export.start","preset":{"id":"p","name":"P","width":WIDTH,"height":HEIGHT},
            "destination":target,"range":[0, 40]}),
    )
    .unwrap_or_else(|error| panic!("export: {error:?}"));
    assert_eq!(report["verification"]["ok"], true, "{report}");

    let frame = |n: u32| {
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
    let first = frame(0);
    assert_near(
        at(&first, TWO.0, TWO.1),
        BLACK,
        8,
        "exported, before any step",
    );
    let second = frame(18);
    assert_near(at(&second, TWO.0, TWO.1), GREEN, 10, "exported second step");
    assert_near(
        at(&second, THREE.0, THREE.1),
        BLACK,
        8,
        "exported third waits",
    );
    let all = frame(36);
    assert_near(at(&all, ONE.0, ONE.1), ORANGE, 10, "exported first step");
    assert_near(at(&all, TWO.0, TWO.1), GREEN, 10, "exported second step");
    assert_near(at(&all, THREE.0, THREE.1), BLUE, 10, "exported third step");
}
