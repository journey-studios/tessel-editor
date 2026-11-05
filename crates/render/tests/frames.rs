//! Real frames from the real runtime (Node worker + headless Chrome), no window involved.
//! A machine without Chrome or the runtime bundle skips these with the reason on stderr;
//! set `TESSEL_REQUIRE_RUNTIME=1` (CI) to turn the skip into a failure.
use std::path::PathBuf;

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{
    PROOF_CANVAS, QUADRANT_COLORS, StandardSet, proof_requests, video_frame_color,
    write_standard_set,
};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::{EditorState, headless};
use tessel_editor_render::{RuntimeConfig, session};

struct Rig {
    state: EditorState,
    session: Box<dyn Renderer>,
    set: StandardSet,
    out: PathBuf,
}

impl Rig {
    fn run(&mut self, request: Value) -> Result<Value, tessel_editor_core::CommandError> {
        headless::execute_with(&mut self.state, Some(self.session.as_mut()), &request)
    }

    fn ok(&mut self, request: Value) -> Value {
        self.run(request.clone())
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    /// Renders `frame` at the proof canvas size and returns `(frame result, png path)`.
    fn frame(&mut self, name: &str, frame: i64, probes: &[(u32, u32)]) -> (Value, PathBuf) {
        let png = self.out.join(format!("{name}.png"));
        let result = self.ok(json!({"op":"render.frame","frame":frame,"out":png,"probes":probes}));
        (result["frame"].clone(), png)
    }
}

fn rig(tag: &str, build_project: bool) -> Option<Rig> {
    let dir = std::env::temp_dir().join(format!("tessel-render-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let set = write_standard_set(&dir.join("fixtures")).unwrap();
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => return skip(&setup.message()),
    };
    let mut rig = Rig {
        state: EditorState::demo(),
        session: Box::new(session(&config)),
        set,
        out: dir.join("out"),
    };
    // The handshake starts Chrome; a missing browser shows up here as backend_unavailable.
    if let Err(error) = rig.run(json!({"op":"capabilities"})) {
        assert_eq!(error.code, "backend_unavailable", "{error:?}");
        return skip(&error.message);
    }
    if build_project {
        for request in proof_requests(&rig.set.clone()) {
            rig.ok(request);
        }
    }
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

fn near(actual: [i32; 3], expected: [u8; 3], tolerance: i32, what: &str) {
    for channel in 0..3 {
        assert!(
            (actual[channel] - i32::from(expected[channel])).abs() <= tolerance,
            "{what}: got {actual:?}, expected about {expected:?}"
        );
    }
}

/// Probe points at the center of each quadrant of the 160x90 image the proof project centers
/// in the 320x180 canvas (zoom 50 %).
const QUADRANT_PROBES: [(u32, u32); 4] = [(120, 67), (200, 67), (120, 112), (200, 112)];

#[test]
fn handshake_reports_the_runtime_and_the_layers_it_can_draw() {
    let Some(mut rig) = rig("caps", false) else {
        return;
    };
    let caps = rig.ok(json!({"op":"capabilities"}));
    assert_eq!(caps["backend"], "tessel-three");
    assert_eq!(caps["bridge_version"], 1);
    assert_eq!(
        caps["layers"],
        json!(["image", "video", "text", "scene", "waveform"])
    );
    assert!(
        caps["renderer"].as_str().unwrap().len() > 3,
        "the GL renderer is reported: {caps}"
    );
}

#[test]
fn proof_project_renders_image_video_text_and_scene_through_the_same_plan() {
    let Some(mut rig) = rig("proof", true) else {
        return;
    };

    // Frame 0: the image over the scene, with the title on top.
    let (first, first_png) = rig.frame("f00", 0, &QUADRANT_PROBES);
    for (index, color) in QUADRANT_COLORS.iter().enumerate() {
        near(
            pixel(&first, index),
            *color,
            3,
            &format!("image quadrant {index}"),
        );
    }
    assert_eq!(first["width"], PROOF_CANVAS.0);
    assert_eq!(&std::fs::read(&first_png).unwrap()[1..4], b"PNG");

    // The title is white pixels in the bottom band; the row probes must hit at least one glyph cell.
    let band: Vec<(u32, u32)> = (60..260u32)
        .step_by(2)
        .flat_map(|x| (146..170u32).step_by(3).map(move |y| (x, y)))
        .collect();
    let (titled, _) = rig.frame("f00-title", 0, &band);
    let white = (0..band.len())
        .filter(|index| pixel(&titled, *index) == [255, 255, 255])
        .count();
    assert!(white > 20, "title pixels found: {white}");

    // Frame 30: the media track has switched to the video, whose frame color encodes source frame 2.
    let (video, _) = rig.frame("f30", 30, &[(160, 90)]);
    near(
        pixel(&video, 0),
        video_frame_color(2),
        3,
        "video frame at sequence frame 30",
    );
    let (later, _) = rig.frame("f47", 47, &[(160, 90)]);
    near(
        pixel(&later, 0),
        video_frame_color(7),
        3,
        "video frame at sequence frame 47",
    );

    // Seeking is a different picture; repeating the same seek is the same picture, bit for bit.
    assert_ne!(first["hash"], video["hash"]);
    let (again, _) = rig.frame("f00-again", 0, &[]);
    assert_eq!(
        first["hash"], again["hash"],
        "software rendering is deterministic"
    );
}

#[test]
fn camera_orbit_and_splats_change_the_scene_frame_with_time() {
    let Some(mut rig) = rig("orbit", true) else {
        return;
    };
    // Isolate the scene layer, then compare two times.
    rig.ok(json!({"op":"set","target":"track:media-track","field":"hidden","value":true}));
    rig.ok(json!({"op":"set","target":"track:title-track","field":"hidden","value":true}));
    let (at_start, start_png) = rig.frame("scene-0", 0, &[(160, 90)]);
    let (at_half, _) = rig.frame("scene-24", 24, &[]);
    assert_ne!(at_start["hash"], at_half["hash"]);
    // The splat sphere sits at the origin, in front of the dark background: its center pixel is
    // colored by the splats and far from the background color (10, 13, 26).
    let center = pixel(&at_start, 0);
    assert!(
        center.iter().sum::<i32>() > 250,
        "splat sphere pixel {center:?}"
    );
    assert!(std::fs::metadata(start_png).unwrap().len() > 500);

    // Orbit 0 pins the camera: same picture at both times except for the spinning objects, so
    // the two hashes differ but the first frame is unchanged by the edit.
    rig.ok(json!({"op":"clip.camera","clip":"c-scene","position":[0,1.2,5],"target":[0,0,0],"fov":50,"orbit":0}));
    let (pinned, _) = rig.frame("scene-0-pinned", 0, &[]);
    assert_eq!(
        at_start["hash"], pinned["hash"],
        "orbit is zero at time zero either way"
    );
}

#[test]
fn source_preview_shows_one_asset_independent_of_the_sequence() {
    let Some(mut rig) = rig("source", true) else {
        return;
    };
    rig.ok(json!({"op":"preview.set","mode":"source","asset":"img"}));
    let png = rig.out.join("source.png");
    let result = rig
        .ok(json!({"op":"render.frame","out":png,"probes":[[10,10],[310,170],[80,40],[240,140]]}));
    let frame = &result["frame"];
    // Letterboxed: the 64x36 image fills the 320x180 canvas exactly (same 16:9 ratio).
    near(
        pixel(frame, 2),
        QUADRANT_COLORS[0],
        3,
        "top-left quadrant of the source",
    );
    near(
        pixel(frame, 3),
        QUADRANT_COLORS[3],
        3,
        "bottom-right quadrant of the source",
    );
    assert_eq!(result["plan"]["layers"].as_array().unwrap().len(), 1);

    // Audio source preview is a waveform layer with the playhead as a white column.
    rig.ok(json!({"op":"preview.set","mode":"source","asset":"tone"}));
    let wave = rig.ok(json!({"op":"render.frame","probes":[[0,90],[160,90],[160,2]]}));
    assert_eq!(wave["plan"]["layers"][0]["kind"], "waveform");
    assert_eq!(
        pixel(&wave["frame"], 0),
        [255, 255, 255],
        "playhead at the start of the source"
    );
    near(
        pixel(&wave["frame"], 1),
        [0x5f, 0xb0, 0xff],
        3,
        "waveform bar at the center",
    );
}

#[test]
fn failures_surface_with_the_asset_and_the_session_recovers() {
    let Some(mut rig) = rig("failures", true) else {
        return;
    };
    let before = rig.state.summary();

    // The runtime cannot draw this blend mode: the frame fails, the state does not change.
    rig.ok(json!({"op":"clip.property","clip":"c-img","property":"blend","value":"overlay"}));
    let blend = rig.run(json!({"op":"render.frame","frame":0})).unwrap_err();
    assert_eq!(blend.code, "unsupported_format", "{blend:?}");
    assert!(blend.message.contains("overlay"), "{blend:?}");
    rig.ok(json!({"op":"clip.property","clip":"c-img","property":"blend","value":"normal"}));

    // A missing file is reported by the core before the runtime is asked, with its path. The
    // session forgets loaded assets first: an asset already in the runtime is not re-read.
    rig.session.reset_project();
    std::fs::remove_file(&rig.set.image).unwrap();
    let missing = rig.run(json!({"op":"render.frame","frame":0})).unwrap_err();
    assert_eq!(missing.code, "asset_unreadable", "{missing:?}");
    assert!(missing.message.contains("quadrants.png"), "{missing:?}");

    // Putting the file back recovers without restarting anything.
    write_standard_set(&rig.set.dir).unwrap();
    let (recovered, _) = rig.frame("recovered", 0, &QUADRANT_PROBES);
    near(
        pixel(&recovered, 0),
        QUADRANT_COLORS[0],
        3,
        "recovered frame",
    );
    assert_eq!(
        before["timeline"]["clips"],
        rig.state.summary()["timeline"]["clips"]
    );

    // A corrupt image is a decode error from the runtime, attributed to the asset.
    std::fs::write(&rig.set.image, b"not a png").unwrap();
    rig.session.reset_project();
    let corrupt = rig.run(json!({"op":"render.frame","frame":0})).unwrap_err();
    assert_eq!(corrupt.code, "decode_failed", "{corrupt:?}");
    assert!(corrupt.message.contains("img"), "{corrupt:?}");
}

#[test]
fn replacing_the_project_does_not_leak_assets_between_projects() {
    let Some(mut rig) = rig("replace", true) else {
        return;
    };
    let (before, _) = rig.frame("before", 0, &[]);
    // A new project reusing the id `img` for different pixels must not show the old texture.
    rig.ok(json!({"op":"project.create","name":"Second","width":64,"height":36,"length":24}));
    let flat = rig.out.join("flat.png");
    std::fs::write(
        &flat,
        tessel_editor_core::fixtures::video_frame_png(8, 8, 3),
    )
    .unwrap();
    rig.ok(json!({"op":"asset.add","kind":"image","id":"img","name":"Flat","path":flat}));
    rig.ok(json!({"op":"track.add","id":"v","name":"V","kind":"video"}));
    rig.ok(json!({"op":"clip.add","track":"v","asset":"img","start":0,"duration":24}));
    let result = rig.ok(json!({"op":"render.frame","frame":0,"probes":[[32,18]]}));
    near(
        pixel(&result["frame"], 0),
        video_frame_color(3),
        3,
        "second project's own image",
    );
    assert_ne!(before["hash"], result["frame"]["hash"]);
}

#[test]
fn a_missing_runtime_is_reported_with_the_fix_not_simulated() {
    let config = RuntimeConfig {
        node: PathBuf::from("node"),
        runtime_dir: std::env::temp_dir().join("tessel-no-runtime-here"),
        gl: None,
    };
    let error = config.check().unwrap_err();
    assert!(
        error.message().contains("TESSEL_RUNTIME_DIR"),
        "{}",
        error.message()
    );

    let mut state = EditorState::demo();
    let mut unavailable = session(&config);
    let result = headless::execute_with(
        &mut state,
        Some(&mut unavailable),
        &json!({"op":"render.frame","frame":0,"intent":{"mode":"sequence"}}),
    )
    .unwrap_err();
    // The demonstration project has no media behind its clips; planning fails first, or the
    // backend says it is unavailable. Neither pretends to have drawn.
    assert!(
        matches!(result.code, "backend_unavailable" | "asset_unavailable"),
        "{result:?}"
    );
}
