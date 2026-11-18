//! F073: images, animated pictures and videos imported as media and drawn by the real runtime
//! (Node worker + headless Chrome), through the same `media.import`, `clip.add`, `render.frame`
//! and `export.start` operations every adapter uses.
//!
//! Every fixture is made with FFmpeg and carries its picture index as a colour (red, green, blue)
//! on the left half and nothing on the right half, over a yellow shape below it: "the right
//! picture" and "transparency works" are read from pixels, never inferred from a hash or a
//! timer. Skipped (loudly) without the runtime or FFmpeg; `TESSEL_REQUIRE_RUNTIME=1` makes that
//! fail. The formats and their limits are tabulated in `docs/features/media-animation.md`.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::host::{MediaHost, RendererFactory};
use tessel_editor_media::tools::Tools;
use tessel_editor_render::{RuntimeConfig, session};

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;
/// Frames per second of the sequence and of every animated fixture (100 ms per picture).
const RATE: u32 = 10;
const COLOURS: [[i64; 3]; 3] = [[255, 0, 0], [0, 255, 0], [0, 0, 255]];
const YELLOW: [i64; 3] = [255, 255, 0];
const LEFT: [u32; 2] = [8, 24];
const RIGHT: [u32; 2] = [56, 24];

fn require(reason: &str) -> bool {
    assert!(
        std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
        "required but unavailable: {reason}"
    );
    eprintln!("SKIPPED: {reason}");
    false
}

fn ffmpeg(tools: &Tools, args: &[&str]) -> bool {
    Command::new(&tools.ffmpeg)
        .args(["-nostdin", "-v", "error", "-y"])
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
}

/// The three source pictures `f0.png`..`f2.png`: a colour on the left half, fully transparent
/// right half.
fn make_pictures(tools: &Tools, dir: &Path) -> bool {
    ["0xFF0000", "0x00FF00", "0x0000FF"]
        .iter()
        .enumerate()
        .all(|(index, colour)| {
            let source = format!(
                "color=c={colour}:s={WIDTH}x{HEIGHT}:r={RATE},format=rgba,drawbox=x={}:y=0:w={}:h={HEIGHT}:color=0x00000000:t=fill:replace=1",
                WIDTH / 2,
                WIDTH / 2
            );
            let target = dir.join(format!("f{index}.png"));
            ffmpeg(
                tools,
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    &source,
                    "-frames:v",
                    "1",
                    target.to_str().unwrap(),
                ],
            )
        })
}

/// Encodes the three pictures as one animated file with `codec_args` (100 ms each).
fn make_animation(tools: &Tools, dir: &Path, name: &str, codec_args: &[&str]) -> Option<PathBuf> {
    let target = dir.join(name);
    let pattern = dir.join("f%d.png");
    let rate = RATE.to_string();
    let mut args = vec![
        "-start_number",
        "0",
        "-framerate",
        &rate,
        "-i",
        pattern.to_str().unwrap(),
    ];
    args.extend_from_slice(codec_args);
    args.push(target.to_str().unwrap());
    ffmpeg(tools, &args).then_some(target)
}

/// One picture (`f0.png`) in another format.
fn make_still(tools: &Tools, dir: &Path, name: &str, codec_args: &[&str]) -> Option<PathBuf> {
    let target = dir.join(name);
    let source = dir.join("f0.png");
    let mut args = vec!["-i", source.to_str().unwrap(), "-frames:v", "1"];
    args.extend_from_slice(codec_args);
    args.push(target.to_str().unwrap());
    ffmpeg(tools, &args).then_some(target)
}

struct Rig {
    state: EditorState,
    renderer: Box<dyn Renderer>,
    host: MediaHost,
    tools: Tools,
    dir: PathBuf,
    next_start: i64,
}

impl Rig {
    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        headless::execute_ext(
            &mut self.state,
            Some(self.renderer.as_mut()),
            Some(&mut self.host),
            &request,
        )
    }

    fn ok(&mut self, request: Value) -> Value {
        self.run(request.clone())
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    /// Imports `file` and returns `(asset id, kind)`.
    fn import(&mut self, file: &Path) -> (String, String) {
        let (id, kind, _) = self.import_decoded(file);
        (id, kind)
    }

    /// Like `import`, and whether the import derived frames or a PNG from the file.
    fn import_decoded(&mut self, file: &Path) -> (String, String, bool) {
        let report = self.ok(json!({"op":"media.import","paths":[file]}));
        let item = &report["applied"]["items"][0];
        // Importing the same bytes again answers with the asset already in the workspace.
        let id = match item["status"].as_str() {
            Some("ready") => item["id"].as_str(),
            Some("duplicate") => item["duplicate_of"].as_str(),
            _ => None,
        }
        .unwrap_or_else(|| panic!("{file:?}: {item}"))
        .to_string();
        let decoded = report["applied"]["decoded"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|candidate| candidate == id.as_str()));
        (id, item["kind"].as_str().unwrap().to_string(), decoded)
    }

    /// Places `asset` for `duration` frames after the previous clip and returns its first frame.
    fn place(
        &mut self,
        asset: &str,
        duration: i64,
        properties: Value,
    ) -> Result<i64, CommandError> {
        let start = self.next_start;
        let clip = format!("c{start}");
        self.run(
            json!({"op":"clip.add","id":clip,"track":"top","asset":asset,"start":start,
            "duration":duration,"properties":properties}),
        )?;
        self.next_start = start + duration + 2;
        Ok(start)
    }

    fn probes(&mut self, frame: i64) -> (Vec<i64>, Vec<i64>) {
        let result = self.ok(json!({"op":"render.frame","frame":frame,"probes":[LEFT, RIGHT]}));
        let probes = result["frame"]["probes"].as_array().expect("probes");
        let rgb = |pixel: &Value| {
            (0..3)
                .map(|c| pixel[c].as_i64().unwrap())
                .collect::<Vec<_>>()
        };
        (rgb(&probes[0]), rgb(&probes[1]))
    }
}

fn near(actual: &[i64], wanted: [i64; 3], tolerance: i64, label: &str) {
    let off: Vec<i64> = actual
        .iter()
        .zip(wanted)
        .map(|(a, w)| (a - w).abs())
        .collect();
    assert!(
        off.iter().all(|difference| *difference <= tolerance),
        "{label}: drew {actual:?}, wanted {wanted:?} ± {tolerance}"
    );
}

fn mix(over: [i64; 3], under: [i64; 3], opacity: f64) -> [i64; 3] {
    let channel = |index: usize| {
        (over[index] as f64 * opacity + under[index] as f64 * (1.0 - opacity)).round() as i64
    };
    [channel(0), channel(1), channel(2)]
}

/// A deterministic shuffle (xorshift), so a failing order can be replayed.
fn shuffled(count: i64, mut seed: u64) -> Vec<i64> {
    let mut items: Vec<i64> = (0..count).collect();
    for index in (1..items.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        items.swap(index, (seed % (index as u64 + 1)) as usize);
    }
    items
}

fn rig(tag: &str, length: i64) -> Option<Rig> {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => {
            require(&setup.message());
            return None;
        }
    };
    let Ok(tools) = Tools::discover() else {
        require("FFmpeg is not installed");
        return None;
    };
    let dir =
        std::env::temp_dir().join(format!("tessel-render-media-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    if !make_pictures(&tools, &dir) {
        require("FFmpeg could not make the source pictures");
        return None;
    }
    let factory_config = RuntimeConfig::discover().ok()?;
    let factory: Arc<RendererFactory> = Arc::new(move || Ok(Box::new(session(&factory_config))));
    let mut rig = Rig {
        state: EditorState::demo(),
        renderer: Box::new(session(&config)),
        host: MediaHost::new().with_renderer(factory),
        tools,
        dir,
        next_start: 0,
    };
    if let Err(error) = rig.run(json!({"op":"capabilities"})) {
        assert_eq!(error.code, "backend_unavailable", "{error:?}");
        require(&error.message);
        return None;
    }
    let workspace = rig.dir.join("project");
    rig.ok(
        json!({"op":"project.create","name":"Media proof","sequence":"main","rate":RATE,
        "width":WIDTH,"height":HEIGHT,"length":length,"dir":workspace}),
    );
    rig.ok(json!({"op":"project.save","path":workspace.join("proof.tessel.json")}));
    rig.ok(json!({"op":"track.add","id":"below","name":"Below","kind":"video"}));
    rig.ok(json!({"op":"track.add","id":"top","name":"Top","kind":"video"}));
    // A yellow shape under everything: transparency shows as yellow, an opaque picture hides it.
    rig.ok(
        json!({"op":"clip.add","id":"yellow","track":"below","start":0,"duration":length,
        "shape":{"kind":"rect","width":100,"height":100,"fill":"#ffff00","radius":0}}),
    );
    Some(rig)
}

#[test]
fn every_still_format_is_drawn_and_transparency_shows_what_is_below() {
    let Some(mut rig) = rig("stills", 120) else {
        return;
    };
    let dir = rig.dir.clone();
    let tools = rig.tools.clone();
    // (file, whether the format carries transparency)
    let mut stills: Vec<(PathBuf, bool)> = vec![(dir.join("f0.png"), true)];
    let candidates: [(&str, &[&str], bool); 5] = [
        ("still.webp", &["-c:v", "libwebp", "-lossless", "1"], true),
        ("still.tiff", &["-c:v", "tiff", "-pix_fmt", "rgba"], true),
        (
            "still.gif",
            &[
                "-vf",
                "split[a][b];[a]palettegen=reserve_transparent=1[p];[b][p]paletteuse=alpha_threshold=128",
            ],
            true,
        ),
        ("still.bmp", &["-pix_fmt", "bgr24"], false),
        ("still.jpg", &["-q:v", "2", "-pix_fmt", "yuvj444p"], false),
    ];
    for (name, args, alpha) in candidates {
        match make_still(&tools, &dir, name, args) {
            Some(path) => stills.push((path, alpha)),
            None => eprintln!("SKIPPED {name}: this FFmpeg cannot encode it"),
        }
    }
    assert!(
        stills.len() >= 5,
        "png, webp, tiff, gif and bmp must be testable"
    );

    for (file, alpha) in stills {
        let label = file.file_name().unwrap().to_string_lossy().to_string();
        let (asset, kind, decoded) = rig.import_decoded(&file);
        assert_eq!(kind, "still", "{label} is one picture");
        // A browser engine cannot read a TIFF: it is drawn from a PNG derived at import.
        assert_eq!(
            decoded,
            label.ends_with(".tiff"),
            "{label}: derived picture"
        );
        let start = rig
            .place(&asset, 3, json!({}))
            .unwrap_or_else(|e| panic!("{label}: {e:?}"));
        // Seek in any order: a still does not depend on the frame asked before.
        for frame in [start + 2, start, start + 1, start + 2] {
            let (left, right) = rig.probes(frame);
            near(&left, COLOURS[0], 8, &format!("{label} left at {frame}"));
            if alpha {
                near(
                    &right,
                    YELLOW,
                    8,
                    &format!("{label} transparent half at {frame}"),
                );
            }
        }
    }
}

#[test]
fn animated_pictures_play_every_picture_in_any_order_with_transparency() {
    let Some(mut rig) = rig("animated", 120) else {
        return;
    };
    let (dir, tools) = (rig.dir.clone(), rig.tools.clone());
    let formats: [(&str, &[&str]); 3] = [
        (
            "anim.gif",
            &[
                "-vf",
                "split[a][b];[a]palettegen=reserve_transparent=1[p];[b][p]paletteuse=alpha_threshold=128",
                "-loop",
                "0",
            ],
        ),
        (
            "anim.apng",
            &["-plays", "0", "-f", "apng", "-pix_fmt", "rgba"],
        ),
        (
            "anim.webp",
            &["-c:v", "libwebp_anim", "-lossless", "1", "-loop", "0"],
        ),
    ];
    let mut tested = 0;
    for (name, args) in formats {
        let Some(file) = make_animation(&tools, &dir, name, args) else {
            eprintln!("SKIPPED {name}: this FFmpeg cannot encode it");
            continue;
        };
        let (asset, kind) = rig.import(&file);
        assert_eq!(
            kind, "video",
            "{name} is an animation, not its first picture"
        );
        let start = rig
            .place(&asset, 3, json!({}))
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        for frame in [2, 0, 1, 2, 2, 0, 1, 0] {
            let (left, right) = rig.probes(start + frame);
            near(
                &left,
                COLOURS[frame as usize],
                10,
                &format!("{name} picture {frame}"),
            );
            near(
                &right,
                YELLOW,
                10,
                &format!("{name} transparent half at {frame}"),
            );
        }
        tested += 1;
    }
    assert!(tested >= 1, "at least the GIF must be encodable");
}

#[test]
fn videos_with_and_without_an_alpha_channel_show_the_right_picture_in_any_order() {
    let Some(mut rig) = rig("videos", 200) else {
        return;
    };
    let (dir, tools) = (rig.dir.clone(), rig.tools.clone());
    // (file, args, keeps transparency)
    let formats: [(&str, &[&str], bool); 4] = [
        (
            "anim_qtrle.mov",
            &["-c:v", "qtrle", "-pix_fmt", "argb"],
            true,
        ),
        (
            "anim_prores.mov",
            &[
                "-c:v",
                "prores_ks",
                "-profile:v",
                "4444",
                "-pix_fmt",
                "yuva444p10le",
            ],
            true,
        ),
        (
            "anim.mp4",
            &[
                "-c:v", "libx264", "-crf", "12", "-pix_fmt", "yuv420p", "-g", "1",
            ],
            false,
        ),
        (
            "anim.webm",
            &[
                "-c:v",
                "libvpx-vp9",
                "-crf",
                "20",
                "-b:v",
                "0",
                "-pix_fmt",
                "yuv420p",
            ],
            false,
        ),
    ];
    for (name, args, alpha) in formats {
        let Some(file) = make_animation(&tools, &dir, name, args) else {
            eprintln!("SKIPPED {name}: this FFmpeg cannot encode it");
            continue;
        };
        let (asset, kind) = rig.import(&file);
        assert_eq!(kind, "video", "{name}");
        let start = rig
            .place(&asset, 3, json!({}))
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        for frame in shuffled(3, 0x9e37_79b9).into_iter().chain([0, 2, 1]) {
            let (left, right) = rig.probes(start + frame);
            near(
                &left,
                COLOURS[frame as usize],
                24,
                &format!("{name} picture {frame}"),
            );
            if alpha {
                near(
                    &right,
                    YELLOW,
                    24,
                    &format!("{name} transparent half at {frame}"),
                );
            }
        }
    }
}

#[test]
fn a_loop_repeats_a_short_animation_and_without_it_the_clip_must_fit_the_source() {
    let Some(mut rig) = rig("loop", 120) else {
        return;
    };
    let (dir, tools) = (rig.dir.clone(), rig.tools.clone());
    let Some(gif) = make_animation(
        &tools,
        &dir,
        "anim.gif",
        &[
            "-vf",
            "split[a][b];[a]palettegen=reserve_transparent=1[p];[b][p]paletteuse=alpha_threshold=128",
            "-loop",
            "0",
        ],
    ) else {
        require("FFmpeg cannot encode GIF");
        return;
    };
    let (asset, _) = rig.import(&gif);
    // Three pictures cannot fill nine frames by themselves: the clip is refused, not stretched.
    let error = rig.place(&asset, 9, json!({})).unwrap_err();
    assert_eq!(error.code, "source_out_of_range", "{error:?}");
    // With `loop` the same animation fills them, picture n at frame n modulo 3.
    let start = rig.place(&asset, 9, json!({"loop": true})).unwrap();
    for frame in shuffled(9, 0x51ed_270b) {
        let (left, right) = rig.probes(start + frame);
        near(
            &left,
            COLOURS[(frame % 3) as usize],
            10,
            &format!("loop frame {frame}"),
        );
        near(
            &right,
            YELLOW,
            10,
            &format!("loop transparent half at {frame}"),
        );
    }
    // The inspector switch is the same property: turning it off holds the last picture instead.
    rig.ok(
        json!({"op":"clip.property","clip":format!("c{start}"),"property":"loop","value":false}),
    );
    let (left, _) = rig.probes(start + 7);
    near(
        &left,
        COLOURS[2],
        10,
        "without loop the last picture is held past the source",
    );
}

#[test]
fn keys_animate_an_image_a_gif_and_a_video_by_the_clock_in_any_order() {
    let Some(mut rig) = rig("keys", 200) else {
        return;
    };
    let (dir, tools) = (rig.dir.clone(), rig.tools.clone());
    let gif = make_animation(
        &tools,
        &dir,
        "anim.gif",
        &[
            "-vf",
            "split[a][b];[a]palettegen=reserve_transparent=1[p];[b][p]paletteuse=alpha_threshold=128",
            "-loop",
            "0",
        ],
    );
    let mp4 = make_animation(
        &tools,
        &dir,
        "anim.mp4",
        &[
            "-c:v", "libx264", "-crf", "12", "-pix_fmt", "yuv420p", "-g", "1",
        ],
    );
    let (Some(gif), Some(mp4)) = (gif, mp4) else {
        require("FFmpeg cannot encode GIF and H.264");
        return;
    };
    let png = dir.join("f0.png");

    // Opacity: 0 % at 0 ms to 100 % at 300 ms of media time, on three kinds of media.
    for (file, picture) in [(png.clone(), None), (gif, Some(())), (mp4, Some(()))] {
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        let (asset, _) = rig.import(&file);
        let duration = if picture.is_some() { 3 } else { 10 };
        let start = rig.place(&asset, duration, json!({})).unwrap();
        let clip = format!("c{start}");
        rig.ok(json!({"op":"clip.key","clip":clip,"prop":"opacity","time_ms":0,"value":0}));
        rig.ok(json!({"op":"clip.key","clip":clip,"prop":"opacity","time_ms":300,"value":100}));
        for frame in shuffled(duration.min(4), 0xdead_beef) {
            let (left, right) = rig.probes(start + frame);
            // A picture starts at 100 ms per frame; the png is the same pose by construction.
            let opacity = (frame as f64 / 3.0).min(1.0);
            let colour = if picture.is_some() {
                COLOURS[frame as usize % 3]
            } else {
                COLOURS[0]
            };
            near(
                &left,
                mix(colour, YELLOW, opacity),
                14,
                &format!("{name} opacity key at {frame}"),
            );
            // H.264 has no alpha: its "transparent" half is opaque black and fades with the key.
            let right_expected = if name.ends_with(".mp4") {
                mix([0, 0, 0], YELLOW, opacity)
            } else {
                YELLOW
            };
            near(
                &right,
                right_expected,
                14,
                &format!("{name} right half at {frame}"),
            );
        }
    }

    // Position: the image slides right by half the canvas over one second.
    let (asset, _) = rig.import(&png);
    let start = rig.place(&asset, 12, json!({})).unwrap();
    let clip = format!("c{start}");
    rig.ok(json!({"op":"clip.key","clip":clip,"prop":"position_x","time_ms":0,"value":0}));
    rig.ok(json!({"op":"clip.key","clip":clip,"prop":"position_x","time_ms":1000,"value":50}));
    let (left, _) = rig.probes(start);
    near(
        &left,
        COLOURS[0],
        8,
        "position key at 0 ms leaves the image in place",
    );
    let (left, _) = rig.probes(start + 10);
    near(
        &left,
        YELLOW,
        8,
        "after 1 s the red half has slid off the left probe",
    );
    let result = rig.ok(json!({"op":"render.frame","frame":start + 10,"probes":[[48, 24]]}));
    near(
        &(0..3)
            .map(|c| result["frame"]["probes"][0][c].as_i64().unwrap())
            .collect::<Vec<_>>(),
        COLOURS[0],
        8,
        "after 1 s the red half is at the right probe",
    );
    // Seeking back is exact: nothing remembers the frame drawn before.
    let (left, _) = rig.probes(start);
    near(&left, COLOURS[0], 8, "seek back to 0 ms");
}

fn exported_rgb(tools: &Tools, file: &Path, index: u64, at: [u32; 2]) -> Vec<i64> {
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
        (WIDTH * HEIGHT * 3) as usize,
        "decoded export frame"
    );
    let offset = ((at[1] * WIDTH + at[0]) * 3) as usize;
    output.stdout[offset..offset + 3]
        .iter()
        .map(|byte| i64::from(*byte))
        .collect()
}

#[test]
fn a_looped_keyed_gif_exports_and_reopens_with_the_pictures_the_monitor_shows() {
    let Some(mut rig) = rig("export", 40) else {
        return;
    };
    let (dir, tools) = (rig.dir.clone(), rig.tools.clone());
    let Some(gif) = make_animation(
        &tools,
        &dir,
        "anim.gif",
        &[
            "-vf",
            "split[a][b];[a]palettegen=reserve_transparent=1[p];[b][p]paletteuse=alpha_threshold=128",
            "-loop",
            "0",
        ],
    ) else {
        require("FFmpeg cannot encode GIF");
        return;
    };
    let (asset, _) = rig.import(&gif);
    let start = rig.place(&asset, 9, json!({"loop": true})).unwrap();
    assert_eq!(start, 0);
    // Opacity 40 % to 100 % over the three pictures: the keys repeat with each pass of the loop.
    rig.ok(json!({"op":"clip.key","clip":"c0","prop":"opacity","time_ms":0,"value":40}));
    rig.ok(json!({"op":"clip.key","clip":"c0","prop":"opacity","time_ms":200,"value":100}));

    let expected = |frame: i64| {
        let opacity = 0.4 + 0.6 * ((frame % 3) as f64 / 2.0);
        mix(COLOURS[(frame % 3) as usize], YELLOW, opacity)
    };
    for frame in shuffled(9, 0x2545_f491) {
        let (left, _) = rig.probes(frame);
        near(
            &left,
            expected(frame),
            14,
            &format!("monitor frame {frame}"),
        );
    }

    // The export is the same clock: nine frames at 10 fps, picture n modulo 3, same opacity.
    let target = dir.join("out").join("proof.mp4");
    let preset = json!({"id":"proof64","name":"Proof","width":WIDTH,"height":HEIGHT});
    let exported =
        rig.ok(json!({"op":"export.start","preset":preset,"destination":target,"range":[0, 9]}));
    assert_eq!(exported["verification"]["ok"], true, "{exported}");
    assert_eq!(exported["frames"], 9);
    for frame in [8u64, 0, 4, 1, 7, 2, 5, 3, 6] {
        let drawn = exported_rgb(&tools, &target, frame, LEFT);
        near(
            &drawn,
            expected(frame as i64),
            34,
            &format!("exported frame {frame}"),
        );
    }

    // Save, reopen in a fresh state: the clip, its loop and its keys draw the same pictures.
    rig.ok(json!({"op":"project.save"}));
    let document = rig.dir.join("project").join("proof.tessel.json");
    let before: Vec<Vec<i64>> = (0..9).map(|frame| rig.probes(frame).0).collect();
    let mut reopened = EditorState::demo();
    headless::execute_ext(
        &mut reopened,
        Some(rig.renderer.as_mut()),
        Some(&mut rig.host),
        &json!({"op":"project.open","path":document}),
    )
    .expect("reopen");
    rig.state = reopened;
    let clip = rig
        .state
        .timeline
        .clip("c0")
        .expect("the clip survives a reopen")
        .clone();
    assert_eq!(clip.keys.len(), 1, "the opacity track survives a reopen");
    for frame in 0..9i64 {
        let (left, _) = rig.probes(frame);
        assert_eq!(left, before[frame as usize], "reopened frame {frame}");
    }
}
