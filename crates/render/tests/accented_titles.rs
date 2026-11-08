//! BL141: accented titles drawn by the real runtime (Node worker + headless Chrome), as a monitor
//! frame (`render.frame`, PNG) and as an exported H.264 video, from one project. The title is white
//! on black, so lit pixels are the glyphs. Skipped without the runtime unless
//! `TESSEL_REQUIRE_RUNTIME=1`; FFmpeg is required to read the pixels back (`docs/media-setup.md`).
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
    let tools = Tools::discover().expect("FFmpeg is required for the title tests");
    let dir = std::env::temp_dir().join(format!("tessel-titles-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut rig = Rig {
        state: EditorState::demo(),
        session: Box::new(session(&config)),
        tools,
        dir,
    };
    for request in [
        json!({"op":"project.create","name":"Titles","sequence":"main","rate":24,
            "width":WIDTH,"height":HEIGHT,"length":48}),
        json!({"op":"asset.add","kind":"text","id":"label","name":"Label",
            "text":{"content":"ACAO","size":14,"color":"#ffffff"}}),
        json!({"op":"track.add","id":"title-track","name":"Titles","kind":"video"}),
        json!({"op":"clip.add","id":"c-title","track":"title-track","asset":"label",
            "start":0,"duration":48}),
    ] {
        rig.ok(request);
    }
    Some(rig)
}

impl Rig {
    fn run(&mut self, request: Value) -> Result<Value, tessel_editor_core::CommandError> {
        headless::execute_with(&mut self.state, Some(self.session.as_mut()), &request)
    }

    fn ok(&mut self, request: Value) -> Value {
        self.run(request.clone())
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    fn set_title(&mut self, content: &str) {
        self.ok(json!({"op":"clip.text","clip":"c-title",
            "text":{"content":content,"size":14,"color":"#ffffff"}}));
    }

    /// The monitor frame of the title as RGB, read back from the PNG `render.frame` wrote.
    fn frame(&mut self, name: &str) -> Mask {
        let png = self.dir.join(format!("{name}.png"));
        self.ok(json!({"op":"render.frame","frame":0,"out":png}));
        let rgb = decode(&self.tools, &png, &["-frames:v", "1"], WIDTH, HEIGHT);
        Mask::new(rgb)
    }
}

fn decode(tools: &Tools, file: &Path, extra: &[&str], width: u32, height: u32) -> Vec<u8> {
    let output = Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(file)
        .args(extra)
        .args(["-pix_fmt", "rgb24", "-f", "rawvideo", "-"])
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

/// Lit (glyph) pixels of a white-on-black picture.
struct Mask {
    rgb: Vec<u8>,
    lit: Vec<bool>,
}

#[derive(Debug, PartialEq)]
struct Bounds {
    left: u32,
    right: u32,
    top: u32,
    bottom: u32,
}

impl Mask {
    fn new(rgb: Vec<u8>) -> Self {
        let lit = rgb.chunks(3).map(|pixel| pixel[0] > 64).collect();
        Self { rgb, lit }
    }

    fn count(&self) -> usize {
        self.lit.iter().filter(|lit| **lit).count()
    }

    fn bounds(&self) -> Bounds {
        let mut bounds = Bounds {
            left: WIDTH,
            right: 0,
            top: HEIGHT,
            bottom: 0,
        };
        for (index, lit) in self.lit.iter().enumerate() {
            if *lit {
                let (x, y) = (index as u32 % WIDTH, index as u32 / WIDTH);
                bounds.left = bounds.left.min(x);
                bounds.right = bounds.right.max(x);
                bounds.top = bounds.top.min(y);
                bounds.bottom = bounds.bottom.max(y);
            }
        }
        assert!(bounds.left <= bounds.right, "the title drew nothing");
        bounds
    }
}

/// The accent rows add lit pixels above the capital and the cedilla adds them below, while the
/// body of the letters stays where plain text puts it and the width does not change.
#[test]
fn accents_are_drawn_around_the_same_body_in_the_monitor_frame() {
    let Some(mut rig) = rig("frame") else {
        return;
    };
    rig.set_title("ACAO");
    let plain = rig.frame("plain");
    rig.set_title("AÇÃO");
    let accented = rig.frame("accented");
    let (plain_box, accent_box) = (plain.bounds(), accented.bounds());

    assert_eq!(
        (plain_box.left, plain_box.right),
        (accent_box.left, accent_box.right),
        "four glyphs, four cells: the width is the same"
    );
    let above = plain_box.top - accent_box.top;
    let below = accent_box.bottom - plain_box.bottom;
    assert!(above >= 4, "the tilde sits above the capital: {above} px");
    assert!(below >= 4, "the cedilla hangs below the body: {below} px");
    assert!(
        above.abs_diff(below) <= 2,
        "the body keeps its place: {above} px above, {below} px below"
    );
    assert!(
        accented.count() > plain.count() + 20,
        "the marks are lit pixels: {} against {}",
        accented.count(),
        plain.count()
    );

    // A different accent is a different picture; lowercase is still drawn as capitals.
    rig.set_title("AÇÁO");
    let acute = rig.frame("acute");
    assert_ne!(acute.rgb, accented.rgb, "acute is not tilde");
    rig.set_title("ação");
    assert_eq!(
        rig.frame("lowercase").rgb,
        accented.rgb,
        "lowercase letters draw as their capitals"
    );
}

/// What the library accepts: the same picture whichever way the text was spelled, and a character
/// the font does not have is refused before any frame is drawn (never an outlined box).
#[test]
fn composed_and_decomposed_titles_are_one_picture_and_unsupported_text_never_draws() {
    let Some(mut rig) = rig("nfc") else {
        return;
    };
    rig.set_title("Não é só açaí — ÀÉÔ");
    let composed = rig.frame("composed");
    assert!(composed.count() > 100);
    rig.set_title(
        "Na\u{0303}o e\u{0301} so\u{0301} ac\u{0327}ai\u{0301} — A\u{0300}E\u{0301}O\u{0302}",
    );
    let decomposed = rig.frame("decomposed");
    assert_eq!(composed.rgb, decomposed.rgb);

    let error = rig
        .run(json!({"op":"clip.text","clip":"c-title",
            "text":{"content":"preço €5","size":14,"color":"#ffffff"}}))
        .unwrap_err();
    assert_eq!(error.code, "unsupported_text");
    assert_eq!(
        rig.frame("after-refusal").rgb,
        composed.rgb,
        "the refused edit changed nothing on screen"
    );
}

/// The exported video draws the same title as the monitor frame: same bounds, and pixels that
/// differ only by the H.264 round trip.
#[test]
fn the_exported_video_shows_the_accented_title_the_monitor_shows() {
    let Some(mut rig) = rig("export") else {
        return;
    };
    rig.set_title("AÇÃO");
    let monitor = rig.frame("monitor");
    rig.ok(json!({"op":"project.save","path":rig.dir.join("project/titles.tessel.json")}));

    let config = RuntimeConfig::discover().unwrap();
    let factory: Arc<RendererFactory> = Arc::new(move || Ok(Box::new(session(&config))));
    let mut host = MediaHost::new().with_renderer(factory);
    let target = rig.dir.join("out/title.mp4");
    let report = headless::execute_ext(
        &mut rig.state,
        None,
        Some(&mut host),
        &json!({"op":"export.start","preset":{"id":"p","name":"P","width":WIDTH,"height":HEIGHT},
            "destination":target,"range":[0, 4]}),
    )
    .unwrap_or_else(|error| panic!("export: {error:?}"));
    assert_eq!(report["verification"]["ok"], true, "{report}");

    let video = Mask::new(decode(
        &rig.tools,
        &target,
        &[
            "-vf",
            "select=eq(n\\,2),scale=in_range=tv:out_range=pc",
            "-frames:v",
            "1",
        ],
        WIDTH,
        HEIGHT,
    ));
    let (monitor_box, video_box) = (monitor.bounds(), video.bounds());
    for (a, b, side) in [
        (monitor_box.left, video_box.left, "left"),
        (monitor_box.right, video_box.right, "right"),
        (monitor_box.top, video_box.top, "top"),
        (monitor_box.bottom, video_box.bottom, "bottom"),
    ] {
        assert!(a.abs_diff(b) <= 2, "{side}: monitor {a}, video {b}");
    }
    let union = monitor
        .lit
        .iter()
        .zip(&video.lit)
        .filter(|(a, b)| **a || **b)
        .count();
    let changed = monitor
        .lit
        .iter()
        .zip(&video.lit)
        .filter(|(a, b)| a != b)
        .count();
    let worst_row = monitor
        .lit
        .chunks(WIDTH as usize)
        .zip(video.lit.chunks(WIDTH as usize))
        .map(|(a, b)| {
            let union = a.iter().zip(b).filter(|(a, b)| **a || **b).count();
            let changed = a.iter().zip(b).filter(|(a, b)| a != b).count();
            if union == 0 {
                0.0
            } else {
                changed as f64 / union as f64
            }
        })
        .fold(0.0_f64, f64::max);
    eprintln!("glyph comparison changed={changed} union={union} worst_row_fraction={worst_row}");
    assert!(
        glyph_masks_agree(&monitor, &video),
        "export differs on the glyph foreground or an accent row"
    );
    rig.set_title("AÇÁO");
    let wrong_accent = rig.frame("wrong-accent");
    assert!(
        !glyph_masks_agree(&monitor, &wrong_accent),
        "the comparison must reject a changed accent"
    );
    eprintln!(
        "title export: monitor lit {}, video lit {}, boxes {monitor_box:?} / {video_box:?}",
        monitor.count(),
        video.count()
    );
}

/// Compare the foreground and each row so the black background cannot hide a wrong accent.
fn glyph_masks_agree(a: &Mask, b: &Mask) -> bool {
    let differences = |left: &[bool], right: &[bool]| {
        let union = left.iter().zip(right).filter(|(x, y)| **x || **y).count();
        let changed = left.iter().zip(right).filter(|(x, y)| x != y).count();
        (union, changed)
    };
    let (union, changed) = differences(&a.lit, &b.lit);
    union > 0
        && changed * 100 <= union * 2
        && a.lit
            .chunks(WIDTH as usize)
            .zip(b.lit.chunks(WIDTH as usize))
            .all(|(left, right)| {
                let (union, changed) = differences(left, right);
                changed <= 1 || changed * 100 <= union * 15
            })
}
