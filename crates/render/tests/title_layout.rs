//! BL141.1: a title too wide for one line is drawn wrapped, centred and inside the frame by the
//! real runtime (Node worker + headless Chrome), as a monitor frame (`render.frame`, PNG) and in
//! the exported H.264 video, in landscape and vertical sequences; an explicit `\n` starts a line.
//! White on black, so lit pixels are the glyphs. Skipped without the runtime unless
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

const LONG: &str = "Ação, Câmera e Edição: São Paulo";

struct Rig {
    state: EditorState,
    session: Box<dyn Renderer>,
    tools: Tools,
    dir: PathBuf,
    width: u32,
    height: u32,
}

fn rig(tag: &str, width: u32, height: u32, content: &str, size: f64) -> Option<Rig> {
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
    let dir = std::env::temp_dir().join(format!("tessel-layout-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut rig = Rig {
        state: EditorState::demo(),
        session: Box::new(session(&config)),
        tools,
        dir,
        width,
        height,
    };
    for request in [
        json!({"op":"project.create","name":"Titles","sequence":"main","rate":24,
            "width":width,"height":height,"length":48}),
        json!({"op":"asset.add","kind":"text","id":"label","name":"Label",
            "text":{"content":content,"size":size,"color":"#ffffff"}}),
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

    fn frame(&mut self, name: &str) -> Picture {
        let png = self.dir.join(format!("{name}.png"));
        self.ok(json!({"op":"render.frame","frame":0,"out":png}));
        Picture::new(
            decode(
                &self.tools,
                &png,
                &["-frames:v", "1"],
                self.width,
                self.height,
            ),
            self.width,
            self.height,
        )
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

/// The lit pixels of a white-on-black frame.
struct Picture {
    lit: Vec<bool>,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy)]
struct Bounds {
    left: u32,
    right: u32,
    top: u32,
    bottom: u32,
}

impl Picture {
    fn new(rgb: Vec<u8>, width: u32, height: u32) -> Self {
        let lit = rgb.chunks(3).map(|pixel| pixel[0] > 96).collect();
        Self { lit, width, height }
    }

    fn row_lit(&self, y: u32) -> bool {
        (0..self.width).any(|x| self.lit[(y * self.width + x) as usize])
    }

    fn bounds_in(&self, rows: std::ops::RangeInclusive<u32>) -> Bounds {
        let mut bounds = Bounds {
            left: self.width,
            right: 0,
            top: self.height,
            bottom: 0,
        };
        for y in rows {
            for x in 0..self.width {
                if self.lit[(y * self.width + x) as usize] {
                    bounds.left = bounds.left.min(x);
                    bounds.right = bounds.right.max(x);
                    bounds.top = bounds.top.min(y);
                    bounds.bottom = bounds.bottom.max(y);
                }
            }
        }
        assert!(bounds.left <= bounds.right, "nothing drawn there");
        bounds
    }

    fn bounds(&self) -> Bounds {
        self.bounds_in(0..=self.height - 1)
    }

    /// Bands of lit rows separated by at least `gap` blank rows: one per line of text (accents
    /// sit 1 row from their letter, lines `LINE_PITCH_ROWS - body` rows apart).
    fn line_bands(&self, gap: u32) -> Vec<(u32, u32)> {
        let mut bands: Vec<(u32, u32)> = Vec::new();
        let mut blank = u32::MAX;
        for y in 0..self.height {
            if self.row_lit(y) {
                match bands.last_mut() {
                    Some(band) if blank < gap => band.1 = y,
                    _ => bands.push((y, y)),
                }
                blank = 0;
            } else {
                blank = blank.saturating_add(1);
            }
        }
        bands
    }

    /// Asserts the lit block is inside the title-safe area (90 %), up to one pixel of rounding.
    fn assert_inside_safe_area(&self, label: &str) -> Bounds {
        let bounds = self.bounds();
        let margin_x = (f64::from(self.width) * 0.05).floor() as u32;
        let margin_y = (f64::from(self.height) * 0.05).floor() as u32;
        assert!(
            bounds.left + 1 >= margin_x
                && bounds.right <= self.width - margin_x
                && bounds.top + 1 >= margin_y
                && bounds.bottom <= self.height - margin_y,
            "{label}: {bounds:?} leaves the safe area of {}x{}",
            self.width,
            self.height
        );
        bounds
    }
}

fn centre(a: u32, b: u32) -> f64 {
    (f64::from(a) + f64::from(b)) / 2.0
}

#[test]
fn a_long_title_is_wrapped_centred_and_kept_inside_the_frame() {
    for (width, height) in [(320, 180), (180, 320)] {
        let Some(mut rig) = rig("wrap", width, height, LONG, 14.0) else {
            return;
        };
        let picture = rig.frame(&format!("long-{width}x{height}"));
        let label = format!("{width}x{height}");
        let bounds = picture.assert_inside_safe_area(&label);
        let bands = picture.line_bands(3);
        assert!(bands.len() >= 2, "{label}: one band per line: {bands:?}");
        // The block is centred, and so is every line in it: each line's cells are, and its lit
        // pixels may sit off them by what a glyph leaves unlit at its sides (a ':' or ',' lights
        // only its middle columns): at most 2 of the 5 columns at one end, so one cell.
        let cell = 0.14 * f64::from(height) / 7.0;
        let frame_centre = f64::from(width - 1) / 2.0;
        assert!(
            (centre(bounds.left, bounds.right) - frame_centre).abs() <= cell + 1.0,
            "{label}: {bounds:?}"
        );
        for band in &bands {
            let line = picture.bounds_in(band.0..=band.1);
            assert!(
                (centre(line.left, line.right) - frame_centre).abs() <= cell + 1.0,
                "{label}: line {band:?} at {line:?}"
            );
        }
        // The box is centred with the accent padding of the whole title above the first line
        // and below the last; a last line without a cedilla leaves that bottom padding unlit, so
        // the lit block may sit up to the padding (2 cells) above the centre, never more.
        let offset = f64::from(height - 1) / 2.0 - centre(bounds.top, bounds.bottom);
        assert!(
            (-1.0..=2.0 * cell + 1.0).contains(&offset),
            "{label}: the block is centred vertically: {bounds:?}, offset {offset}"
        );
    }
}

#[test]
fn an_explicit_break_draws_two_lines_and_a_title_that_fits_stays_one() {
    let Some(mut rig) = rig("break", 320, 180, "UM\nDOIS", 14.0) else {
        return;
    };
    let two = rig.frame("two");
    assert_eq!(two.line_bands(3).len(), 2, "{:?}", two.line_bands(3));
    rig.ok(json!({"op":"set","target":"clip_text:c-title","field":"content","value":"UM DOIS"}));
    let one = rig.frame("one");
    assert_eq!(one.line_bands(3).len(), 1, "it fits: one line, as before");
    // Same size: the single line is as tall as one of the two (±1 px of resampling phase).
    let (first, single) = (two.line_bands(3)[0], one.line_bands(3)[0]);
    assert!(
        (first.1 - first.0).abs_diff(single.1 - single.0) <= 1,
        "{first:?} vs {single:?}"
    );
}

#[test]
fn the_exported_video_shows_the_wrapped_title_the_monitor_shows() {
    let Some(mut rig) = rig("export", 180, 320, LONG, 14.0) else {
        return;
    };
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
        &json!({"op":"export.start","preset":{"id":"p","name":"P","width":180,"height":320},
            "destination":target,"range":[0, 4]}),
    )
    .unwrap_or_else(|error| panic!("export: {error:?}"));
    assert_eq!(report["verification"]["ok"], true, "{report}");
    let video = Picture::new(
        decode(
            &rig.tools,
            &target,
            &[
                "-vf",
                "select=eq(n\\,2),scale=in_range=tv:out_range=pc",
                "-frames:v",
                "1",
            ],
            180,
            320,
        ),
        180,
        320,
    );
    let (a, b) = (monitor.bounds(), video.bounds());
    for (m, v, side) in [
        (a.left, b.left, "left"),
        (a.right, b.right, "right"),
        (a.top, b.top, "top"),
        (a.bottom, b.bottom, "bottom"),
    ] {
        assert!(m.abs_diff(v) <= 2, "{side}: monitor {m}, video {v}");
    }
    assert_eq!(
        monitor.line_bands(3).len(),
        video.line_bands(3).len(),
        "same number of lines"
    );
    video.assert_inside_safe_area("export");
}

/// A wrapped title placed low (the lower third) grows upwards: every line is drawn inside the frame,
/// where centring the block on the position would push the last one past the bottom edge. The same
/// title on one line stays exactly where the position puts it.
#[test]
fn a_wrapped_title_in_the_lower_third_keeps_every_line_in_the_frame() {
    let Some(mut rig) = rig("anchor", 320, 180, LONG, 14.0) else {
        return;
    };
    rig.ok(json!({"op":"clip.property","clip":"c-title","property":"position_y","value":38}));
    let low = rig.frame("low");
    let bands = low.line_bands(3);
    let plan = rig.ok(json!({"op":"plan","frame":0}));
    let lines = plan["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|layer| layer["kind"] == "text")
        .unwrap()["text"]["lines"]
        .as_array()
        .map_or(1, Vec::len);
    assert!(lines >= 3, "{plan}");
    assert_eq!(bands.len(), lines, "every planned line is drawn: {bands:?}");
    let bounds = low.bounds();
    assert!(
        bounds.top > 0 && bounds.bottom < 179,
        "nothing touches the frame's edges: {bounds:?}"
    );
    // One line at the same position: drawn where it was, low in the frame.
    rig.ok(json!({"op":"set","target":"clip_text:c-title","field":"content","value":"CURTO"}));
    let short = rig.frame("short").bounds();
    let centre = (f64::from(short.top) + f64::from(short.bottom)) / 2.0;
    let expected = 89.5 + 0.38 * 180.0;
    assert!(
        (centre - expected).abs() <= 3.0,
        "a single line is centred on the position: {centre} vs {expected}"
    );
}
