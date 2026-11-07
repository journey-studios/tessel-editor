//! BL143: a SHARP PLY drawn by the real runtime (Node worker + headless Chrome, SwiftShader + Spark).
//!
//! The oracle does not come from the renderer. The synthetic file puts five Gaussians over five
//! known pixels of its 160x120 source image; the pixel each one must take in a 320x180 frame is
//! computed from the file's own intrinsics (`Capture::project` is the pinhole of the primary source,
//! `project_in_editor` the Three.js perspective model after the axis turn). The frame must show each
//! colour there, which a missing axis turn (everything upside down and behind the camera), a wrong
//! field of view or a camera that is not the capture's would all fail.
//!
//! A machine without Chrome or the runtime bundle skips these with the reason on stderr; set
//! `TESSEL_REQUIRE_RUNTIME=1` to turn the skip into a failure. Run them under the GPU lock.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{SHARP_FIXTURE_PIXELS, SharpSpec, write_sharp_ply};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::sharp::{self, Capture, Choice};
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::host::{MediaHost, RendererFactory};
use tessel_editor_media::tools::Tools;
use tessel_editor_render::{RuntimeConfig, session};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
/// What each point of the fixture looks like on a black frame, as the channel that must dominate.
const DOMINANT: [[bool; 3]; 5] = [
    [true, false, false],
    [false, true, false],
    [false, false, true],
    [true, true, false],
    [true, true, true],
];

struct Rig {
    state: EditorState,
    session: Box<dyn Renderer>,
    dir: PathBuf,
}

impl Rig {
    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        headless::execute_with(&mut self.state, Some(self.session.as_mut()), &request)
    }

    fn ok(&mut self, request: Value) -> Value {
        self.run(request.clone())
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }
}

fn rig(tag: &str) -> Option<Rig> {
    let dir = std::env::temp_dir().join(format!("tessel-sharp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => return skip(&setup.message()),
    };
    let mut rig = Rig {
        state: EditorState::demo(),
        session: Box::new(session(&config)),
        dir,
    };
    if let Err(error) = rig.run(json!({"op":"capabilities"})) {
        assert_eq!(error.code, "backend_unavailable", "{error:?}");
        return skip(&error.message);
    }
    rig.ok(
        json!({"op":"project.create","name":"SHARP","sequence":"main","rate":24,
        "width":WIDTH,"height":HEIGHT,"length":48}),
    );
    Some(rig)
}

fn skip(reason: &str) -> Option<Rig> {
    assert!(
        std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
        "the runtime is required but unavailable: {reason}"
    );
    eprintln!("SKIPPED (runtime unavailable): {reason}");
    None
}

fn pixel(frame: &Value, index: usize) -> [i32; 3] {
    let probe = &frame["probes"][index];
    [0, 1, 2].map(|channel| probe[channel].as_i64().unwrap() as i32)
}

/// The probe (frame pixel) of each fixture point, from the file's own capture.
fn probes_of(path: &Path, spec: &SharpSpec) -> Vec<(u32, u32)> {
    let capture = sharp::read(path, Choice::Auto).unwrap().unwrap();
    spec.points
        .iter()
        .map(|point| {
            let at = point.position.map(f64::from);
            let source = capture.project(at).unwrap();
            let frame = capture.project_in_editor(at, [WIDTH, HEIGHT]).unwrap();
            // The independent pinhole must place the five points over the five known pixels.
            assert!(
                SHARP_FIXTURE_PIXELS
                    .iter()
                    .any(|pixel| (pixel[0] - source[0]).abs() < 0.01
                        && (pixel[1] - source[1]).abs() < 0.01)
            );
            (frame[0].round() as u32, frame[1].round() as u32)
        })
        .collect()
}

fn matches(color: [i32; 3], dominant: [bool; 3]) -> bool {
    // On black: a lit channel is bright, an unlit one stays dark.
    (0..3).all(|channel| {
        if dominant[channel] {
            color[channel] > 150
        } else {
            color[channel] < 90
        }
    })
}

fn add_splat_clip(rig: &mut Rig, id: &str, track: &str, path: &Path, capture: &str) {
    rig.ok(
        json!({"op":"asset.add","kind":"splat","id":id,"name":id,"path":path,"capture":capture}),
    );
    rig.ok(json!({"op":"track.add","id":track,"name":track,"kind":"video"}));
    rig.ok(
        json!({"op":"clip.add","id":format!("clip-{id}"),"track":track,"asset":id,
        "start":0,"duration":48}),
    );
}

#[test]
fn a_sharp_splat_lands_on_the_pixels_its_own_intrinsics_predict() {
    let Some(mut rig) = rig("pixels") else {
        return;
    };
    let spec = SharpSpec::default();
    let path = rig.dir.join("shot.ply");
    write_sharp_ply(&path, &spec).unwrap();
    let probes = probes_of(&path, &spec);
    add_splat_clip(&mut rig, "shot", "t-shot", &path, "auto");

    // The clip was created with the capture camera: nothing was typed.
    let capture = sharp::read(&path, Choice::Auto).unwrap().unwrap();
    assert_eq!(
        rig.state.timeline.clip("clip-shot").unwrap().camera,
        Some(capture.camera())
    );
    let png = rig.dir.join("shot.png");
    let result = rig.ok(json!({"op":"render.frame","frame":0,"out":png,"probes":probes}));
    assert_eq!(result["plan"]["layers"][0]["kind"], "scene");
    let layer = &rig.ok(json!({"op":"plan","frame":0}))["layers"][0].clone();
    assert_eq!(layer["orientation"], json!([180.0, 0.0, 0.0]));
    assert!((layer["camera"]["fov"].as_f64().unwrap() - 50.0).abs() < 1e-3);
    assert_eq!(layer["camera"]["position"], json!([0.0, 0.0, 0.0]));
    for (index, dominant) in DOMINANT.iter().enumerate() {
        let color = pixel(&result["frame"], index);
        assert!(
            matches(color, *dominant),
            "point {index} at {:?}: {color:?}",
            probes[index]
        );
    }
    assert!(std::fs::metadata(&png).unwrap().len() > 500);

    // A frame later is the same picture: the capture camera is still.
    let later = rig.ok(json!({"op":"render.frame","frame":24}));
    assert_eq!(result["frame"]["hash"], later["frame"]["hash"]);

    // The source preview of the asset uses the same camera and axes.
    rig.ok(json!({"op":"preview.set","mode":"source","asset":"shot"}));
    let source = rig.ok(json!({"op":"render.frame","probes":probes}));
    let planned = rig.ok(json!({"op":"plan"}));
    assert_eq!(
        planned["layers"][0]["orientation"],
        json!([180.0, 0.0, 0.0])
    );
    assert_eq!(
        planned["layers"][0]["camera"]["position"],
        json!([0.0, 0.0, 0.0])
    );
    for (index, dominant) in DOMINANT.iter().enumerate() {
        assert!(
            matches(pixel(&source["frame"], index), *dominant),
            "source point {index}"
        );
    }
}

#[test]
fn ignoring_the_capture_or_losing_the_axes_does_not_land_on_those_pixels() {
    let Some(mut rig) = rig("negative") else {
        return;
    };
    let spec = SharpSpec::default();
    let path = rig.dir.join("shot.ply");
    write_sharp_ply(&path, &spec).unwrap();
    let probes = probes_of(&path, &spec);
    add_splat_clip(&mut rig, "plain", "t-plain", &path, "ignore");
    // Imported as a plain splat: no capture camera, no axis turn.
    let result = rig.ok(json!({"op":"render.frame","frame":0,"probes":probes}));
    let planned = rig.ok(json!({"op":"plan","frame":0}));
    let layer = &planned["layers"][0];
    assert!(layer.get("orientation").is_none(), "{layer}");
    assert_ne!(layer["camera"]["position"], json!([0.0, 0.0, 0.0]));
    let hits = DOMINANT
        .iter()
        .enumerate()
        .filter(|(index, dominant)| matches(pixel(&result["frame"], *index), **dominant))
        .count();
    assert!(
        hits < 5,
        "a plain splat seen from the default camera matched {hits} of 5 points"
    );

    // Mutation: the capture camera but no axis turn (the file's y-down, z-forward axes taken as the
    // editor's) puts the whole cloud behind the camera: the frame is the empty background.
    rig.ok(json!({"op":"asset.add","kind":"splat","id":"keyed","name":"keyed","path":path}));
    rig.ok(json!({"op":"track.add","id":"t-keyed","name":"keyed","kind":"video"}));
    let capture: Capture = sharp::read(&path, Choice::Auto).unwrap().unwrap();
    rig.ok(
        json!({"op":"clip.add","id":"clip-keyed","track":"t-keyed","asset":"keyed","start":0,
        "duration":48,"camera":{"position":[0,0,0],"target":[0,0,1],"fov":capture.vfov,"orbit":0}}),
    );
    rig.ok(json!({"op":"set","target":"track:t-plain","field":"hidden","value":true}));
    let behind = rig.ok(json!({"op":"render.frame","frame":0,"probes":probes_of(&path, &spec)}));
    // The clip carries the axis turn from the plan; looking down +z at the turned cloud sees nothing.
    let lit = (0..5)
        .filter(|index| pixel(&behind["frame"], *index).iter().sum::<i32>() > 120)
        .count();
    assert_eq!(
        lit, 0,
        "a camera turned away from the cloud saw {lit} points"
    );
}

/// Decodes frame `index` of `file` to RGB24 (BT.709 limited range, as the export writes it).
fn exported_frame(tools: &Tools, file: &Path, index: u64, size: [u32; 2]) -> Vec<u8> {
    let filter = format!(
        "select=eq(n\\,{index}),scale=in_color_matrix=bt709:in_range=tv:out_range=pc,format=rgb24"
    );
    let output = Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(file)
        .args(["-vf", &filter, "-frames:v", "1", "-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert_eq!(output.stdout.len(), (size[0] * size[1] * 3) as usize);
    output.stdout
}

#[test]
fn the_exported_video_keeps_the_axes_and_the_capture_camera() {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => {
            skip(&setup.message());
            return;
        }
    };
    let tools = Tools::discover().expect("FFmpeg is required for the export tests");
    let dir = std::env::temp_dir().join(format!("tessel-sharp-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let spec = SharpSpec::default();
    let path = dir.join("shot.ply");
    write_sharp_ply(&path, &spec).unwrap();
    let capture = sharp::read(&path, Choice::Auto).unwrap().unwrap();

    let factory: Arc<RendererFactory> = Arc::new(move || Ok(Box::new(session(&config))));
    let mut host = MediaHost::new().with_renderer(factory);
    let mut state = EditorState::demo();
    for request in [
        json!({"op":"project.create","name":"SHARP","sequence":"main","rate":24,
            "width":WIDTH,"height":HEIGHT,"length":24}),
        json!({"op":"asset.add","kind":"splat","id":"shot","name":"shot","path":path}),
        json!({"op":"track.add","id":"t","name":"t","kind":"video"}),
        json!({"op":"clip.add","id":"c","track":"t","asset":"shot","start":0,"duration":24}),
        json!({"op":"project.save","path":dir.join("project/shot.tessel.json")}),
    ] {
        headless::execute(&mut state, &request).unwrap();
    }
    let target = dir.join("out/shot.mp4");
    let size = [640u32, 360u32];
    let report = headless::execute_ext(
        &mut state,
        None,
        Some(&mut host),
        &json!({"op":"export.start","preset":{"id":"p","name":"p","width":size[0],"height":size[1]},
            "destination":target,"range":[0, 3]}),
    )
    .unwrap();
    assert_eq!(report["verification"]["ok"], true, "{report}");
    let rgb = exported_frame(&tools, &target, 1, size);
    for (index, point) in spec.points.iter().enumerate() {
        let at = capture
            .project_in_editor(point.position.map(f64::from), size)
            .unwrap();
        let (x, y) = (at[0].round() as u32, at[1].round() as u32);
        let offset = ((y * size[0] + x) * 3) as usize;
        let color = [0, 1, 2].map(|channel| i32::from(rgb[offset + channel]));
        assert!(
            matches(color, DOMINANT[index]),
            "exported point {index} at ({x}, {y}): {color:?}"
        );
    }
}
