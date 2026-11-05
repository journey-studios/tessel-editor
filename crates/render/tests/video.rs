//! Container video and proxy sequences drawn by the real runtime (Node worker + headless
//! Chrome), through the same `render.frame` / `export.start` operations every adapter uses.
//!
//! Every clip here is made with FFmpeg and carries its frame number as a 10-bit barcode in its
//! top row, so "the right picture" is read from pixels, never inferred from a hash or a timer.
//! Skipped (loudly) without the runtime or FFmpeg; `TESSEL_REQUIRE_RUNTIME=1` makes that fail.
//!
//! What is proven: a container video added by path (the native `<video>` path, B1/B2) shows the
//! frame asked for in any order, first and last included; PNG and JPEG proxy sequences are
//! accepted and drawn (B7); an unprepared WebM is a typed error, never a black frame (B3); an
//! imported clip exports with the right frame count, rate and picture identity.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::host::{MediaHost, RendererFactory};
use tessel_editor_media::tools::Tools;
use tessel_editor_render::{RuntimeConfig, session};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;
const FRAMES: i64 = 48;
const CELL: u32 = WIDTH / 20;

fn require(reason: &str) -> bool {
    assert!(
        std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
        "required but unavailable: {reason}"
    );
    eprintln!("SKIPPED: {reason}");
    false
}

/// The `drawbox` chain that writes the frame counter `n` in the top row (white cell = bit set).
fn barcode_filter() -> String {
    let mut boxes = vec![format!(
        "drawbox=x=0:y=0:w={}:h={CELL}:color=black:t=fill",
        CELL * 10
    )];
    for bit in 0..10u32 {
        boxes.push(format!(
            "drawbox=x={}:y=0:w={CELL}:h={CELL}:color=white:t=fill:enable='mod(floor(n/{}),2)'",
            bit * CELL,
            1u32 << bit
        ));
    }
    format!("noise=c0s=5:allf=t,{}", boxes.join(","))
}

fn ffmpeg(tools: &Tools, args: &[&str]) -> bool {
    Command::new(&tools.ffmpeg)
        .args(["-nostdin", "-v", "error", "-y"])
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
}

fn source_args() -> Vec<String> {
    let input = format!("testsrc2=size={WIDTH}x{HEIGHT}:rate=24:duration=100");
    vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        input,
        "-frames:v".into(),
        FRAMES.to_string(),
        "-vf".into(),
        barcode_filter(),
    ]
}

fn make_clip(tools: &Tools, target: &Path, codec: &[&str]) -> bool {
    let source = source_args();
    let mut args: Vec<&str> = source.iter().map(String::as_str).collect();
    args.extend_from_slice(codec);
    args.push(target.to_str().unwrap());
    ffmpeg(tools, &args)
}

struct Rig {
    state: EditorState,
    session: Box<dyn Renderer>,
    dir: PathBuf,
    tools: Tools,
}

impl Rig {
    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        headless::execute_with(&mut self.state, Some(self.session.as_mut()), &request)
    }

    fn ok(&mut self, request: Value) -> Value {
        self.run(request.clone())
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    /// Barcode value of timeline `frame`, read at the 10 cells of the 320x180 canvas.
    fn code_at(&mut self, frame: i64) -> i64 {
        let probes: Vec<[u32; 2]> = (0..10u32)
            .map(|bit| [(bit * 2 + 1) * 160 / 20, 8])
            .collect();
        let result = self.ok(json!({"op":"render.frame","frame":frame,"probes":probes}));
        let probes = result["frame"]["probes"].as_array().expect("probes");
        decode(probes.iter().map(|pixel| {
            let sum: i64 = (0..3).map(|c| pixel[c].as_i64().unwrap()).sum();
            sum as f64 / 3.0
        }))
    }
}

fn decode(levels: impl Iterator<Item = f64>) -> i64 {
    levels
        .enumerate()
        .map(|(bit, level)| if level > 128.0 { 1i64 << bit } else { 0 })
        .sum()
}

fn rig(tag: &str) -> Option<Rig> {
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
        std::env::temp_dir().join(format!("tessel-render-video-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut rig = Rig {
        state: EditorState::demo(),
        session: Box::new(session(&config)),
        dir,
        tools,
    };
    if let Err(error) = rig.run(json!({"op":"capabilities"})) {
        assert_eq!(error.code, "backend_unavailable", "{error:?}");
        require(&error.message);
        return None;
    }
    rig.ok(
        json!({"op":"project.create","name":"Video proof","sequence":"main","rate":24,
        "width":320,"height":180,"length":FRAMES}),
    );
    Some(rig)
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

#[test]
fn a_container_video_added_by_path_shows_the_frame_asked_for_in_any_order() {
    let Some(mut rig) = rig("native") else {
        return;
    };
    let clip = rig.dir.join("clip.mp4");
    if !make_clip(
        &rig.tools,
        &clip,
        &[
            "-c:v",
            "libx264",
            "-crf",
            "18",
            "-preset",
            "veryfast",
            "-pix_fmt",
            "yuv420p",
            "-g",
            "24",
            "-bf",
            "3",
            "-movflags",
            "+faststart",
        ],
    ) {
        require("FFmpeg could not make the H.264 clip");
        return;
    }
    // Added by path with a declared rate and duration and no workspace: nothing prepares frames,
    // so this is the native `<video>` path.
    rig.ok(
        json!({"op":"asset.add","kind":"video","id":"vid","name":"Barcoded","path":clip,
        "rate":24,"width":WIDTH,"height":HEIGHT,"duration":2}),
    );
    rig.ok(json!({"op":"track.add","id":"v","name":"V","kind":"video"}));
    rig.ok(json!({"op":"clip.add","id":"c","track":"v","asset":"vid","start":0,"duration":FRAMES}));

    let mut orders = vec![
        vec![0, FRAMES - 1, 0, 1, FRAMES - 2, 24, 23, 25],
        (0..FRAMES).collect::<Vec<_>>(),
        (0..FRAMES).rev().collect::<Vec<_>>(),
        shuffled(FRAMES, 0x9e37_79b9),
    ];
    for order in orders.drain(..) {
        let wrong: Vec<(i64, i64)> = order
            .into_iter()
            .map(|frame| (frame, rig.code_at(frame)))
            .filter(|(asked, shown)| asked != shown)
            .collect();
        assert!(wrong.is_empty(), "asked frame vs shown frame: {wrong:?}");
    }
}

#[test]
fn a_webm_the_engine_cannot_read_is_a_typed_error_or_exact_never_a_black_frame() {
    let Some(mut rig) = rig("webm") else {
        return;
    };
    let clip = rig.dir.join("clip.webm");
    if !make_clip(
        &rig.tools,
        &clip,
        &[
            "-c:v",
            "libvpx-vp9",
            "-b:v",
            "0",
            "-crf",
            "30",
            "-g",
            "24",
            "-cpu-used",
            "6",
            "-deadline",
            "good",
            "-pix_fmt",
            "yuv420p",
        ],
    ) {
        require("FFmpeg has no VP9 encoder");
        return;
    }
    rig.ok(
        json!({"op":"asset.add","kind":"video","id":"vid","name":"WebM","path":clip,
        "rate":24,"width":WIDTH,"height":HEIGHT,"duration":2}),
    );
    rig.ok(json!({"op":"track.add","id":"v","name":"V","kind":"video"}));
    rig.ok(json!({"op":"clip.add","id":"c","track":"v","asset":"vid","start":0,"duration":FRAMES}));
    // Headless Chrome decodes VP9; the claim is "exact when shown, typed error otherwise".
    match rig.run(json!({"op":"render.frame","frame":10})) {
        Ok(_) => assert_eq!(rig.code_at(10), 10),
        Err(error) => assert!(
            error.code == "unsupported_format" || error.code == "decode_failed",
            "{error:?}"
        ),
    }
}

#[test]
fn png_and_jpeg_proxy_sequences_are_accepted_and_draw_the_frame_asked_for() {
    let Some(mut rig) = rig("sequences") else {
        return;
    };
    for (folder, extension, quality) in [
        ("png", "png", ["-compression_level", "1"]),
        ("jpg", "jpg", ["-q:v", "3"]),
    ] {
        let directory = rig.dir.join(folder);
        std::fs::create_dir_all(&directory).unwrap();
        let pattern = directory.join(format!("x%05d.{extension}"));
        let source = source_args();
        let mut args: Vec<&str> = source.iter().map(String::as_str).collect();
        args.extend_from_slice(&quality);
        args.extend_from_slice(&["-start_number", "0", pattern.to_str().unwrap()]);
        assert!(ffmpeg(&rig.tools, &args), "FFmpeg made the {folder} frames");

        let id = format!("seq-{folder}");
        rig.ok(json!({"op":"asset.add","kind":"video","id":id,"name":id,
            "pattern":pattern,"rate":24,"frames":FRAMES}));
        rig.ok(json!({"op":"track.add","id":format!("t-{folder}"),"name":"T","kind":"video"}));
        rig.ok(
            json!({"op":"clip.add","id":format!("c-{folder}"),"track":format!("t-{folder}"),
            "asset":id,"start":0,"duration":FRAMES}),
        );
        let wrong: Vec<(i64, i64)> = shuffled(FRAMES, 0x51ed_270b)
            .into_iter()
            .map(|frame| (frame, rig.code_at(frame)))
            .filter(|(asked, shown)| asked != shown)
            .collect();
        assert!(wrong.is_empty(), "{folder}: {wrong:?}");
        // The topmost track wins, so the next sequence is drawn on top of this one.
    }
}

#[test]
fn invalid_proxy_sequences_are_refused_when_added() {
    let Some(mut rig) = rig("invalid") else {
        return;
    };
    let directory = rig.dir.join("bad");
    std::fs::create_dir_all(&directory).unwrap();
    for index in 0..3 {
        std::fs::write(directory.join(format!("f{index:05}.png")), b"not a picture").unwrap();
    }
    let pattern = directory.join("f%05d.png");
    let error = rig
        .run(
            json!({"op":"asset.add","kind":"video","id":"bad","name":"Bad","pattern":pattern,
            "rate":24,"frames":3}),
        )
        .unwrap_err();
    assert_eq!(error.code, "invalid_asset", "{error:?}");
}

fn ffprobe(tools: &Tools, file: &Path, entries: &str) -> String {
    let output = Command::new(&tools.ffprobe)
        .args(["-v", "error", "-count_frames", "-select_streams", "v:0"])
        .args(["-show_entries", entries, "-of", "default=nw=1"])
        .arg(file)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// RGB bytes of exported frame `index`, color-converted like the viewer would show it.
fn exported_code(tools: &Tools, file: &Path, index: u64) -> i64 {
    let filter = format!(
        "select=eq(n\\,{index}),scale=in_color_matrix=bt709:in_range=tv:out_range=pc,format=rgb24"
    );
    let output = Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(file)
        .args(["-vf", &filter, "-frames:v", "1", "-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert_eq!(output.stdout.len(), (WIDTH * HEIGHT * 3) as usize);
    decode((0..10u32).map(|bit| {
        let x = bit * CELL + CELL / 2;
        let y = CELL / 2;
        let at = ((y * WIDTH + x) * 3) as usize;
        output.stdout[at..at + 3]
            .iter()
            .map(|byte| f64::from(*byte))
            .sum::<f64>()
            / 3.0
    }))
}

#[test]
fn an_imported_clip_exports_with_the_right_frames_rate_and_pictures() {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => {
            require(&setup.message());
            return;
        }
    };
    let Ok(tools) = Tools::discover() else {
        require("FFmpeg is not installed");
        return;
    };
    let dir =
        std::env::temp_dir().join(format!("tessel-render-video-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let clip = dir.join("clip.mp4");
    if !make_clip(
        &tools,
        &clip,
        &[
            "-c:v",
            "libx264",
            "-crf",
            "14",
            "-preset",
            "veryfast",
            "-pix_fmt",
            "yuv420p",
            "-g",
            "24",
            "-movflags",
            "+faststart",
        ],
    ) {
        require("FFmpeg could not make the H.264 clip");
        return;
    }
    let factory: Arc<RendererFactory> = Arc::new(move || Ok(Box::new(session(&config))));
    let mut host = MediaHost::new().with_renderer(factory);
    let mut state = EditorState::demo();
    let workspace = dir.join("project");
    let run = |state: &mut EditorState, host: &mut MediaHost, request: Value| {
        headless::execute_ext(state, None, Some(host), &request)
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    };
    run(
        &mut state,
        &mut host,
        json!({"op":"project.create","name":"Export proof","sequence":"main","rate":24,
        "width":640,"height":360,"length":FRAMES,"dir":workspace}),
    );
    run(
        &mut state,
        &mut host,
        json!({"op":"project.save","path":workspace.join("proof.tessel.json")}),
    );
    let report = run(
        &mut state,
        &mut host,
        json!({"op":"media.import","paths":[clip]}),
    );
    let id = report["applied"]["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut state,
        &mut host,
        json!({"op":"track.add","id":"v","name":"V","kind":"video"}),
    );
    run(
        &mut state,
        &mut host,
        json!({"op":"clip.add","id":"c","track":"v","asset":id,"start":0,"duration":FRAMES}),
    );

    // 16 frames from timeline frame 4: exported picture i is source frame 4 + i.
    let target = dir.join("out/proof.mp4");
    let preset = json!({"id":"proof640","name":"Proof","width":640,"height":360});
    let exported = run(
        &mut state,
        &mut host,
        json!({"op":"export.start","preset":preset,"destination":target,"range":[4, 20]}),
    );
    assert_eq!(exported["verification"]["ok"], true, "{exported}");
    assert_eq!(exported["frames"], 16);

    let probe = ffprobe(
        &tools,
        &target,
        "stream=nb_read_frames,avg_frame_rate,width,height",
    );
    assert!(probe.contains("nb_read_frames=16"), "{probe}");
    assert!(probe.contains("avg_frame_rate=24/1"), "{probe}");
    assert!(
        probe.contains("width=640") && probe.contains("height=360"),
        "{probe}"
    );
    for index in [0u64, 7, 15] {
        assert_eq!(
            exported_code(&tools, &target, index),
            4 + index as i64,
            "exported frame {index} carries source frame {}",
            4 + index
        );
    }
}
