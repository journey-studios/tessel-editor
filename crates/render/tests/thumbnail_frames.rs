//! BL144: library thumbnails drawn by the real runtime (Node worker + headless Chrome, SwiftShader),
//! through the same `media.thumbnails` job the window schedules. No window involved.
//!
//! The oracle is independent of the renderer: where the scene's box and sphere must land in the
//! thumbnail is projected here with a plain pinhole from the default scene camera, and the pixels
//! under those points are read from the very frame that became the PNG (a spy renderer asks the
//! real session for probes). The scene thumbnail must show the box (orange) and the sphere
//! (blue) there; the thumbnail of the bare splat must show neither, and both must show the cloud
//! at the center and the background in the corners. Not proved: a real GPU (SwiftShader).
//!
//! A machine without Chrome or the runtime bundle skips these with the reason on stderr; set
//! `TESSEL_REQUIRE_RUNTIME=1` to turn the skip into a failure. Run them under the GPU lock.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tessel_editor_core::backend::{BackendError, Capabilities, FrameResult, Output};
use tessel_editor_core::fixtures::{SHARP_FIXTURE_PIXELS, write_standard_set};
use tessel_editor_core::plan::{PreviewIntent, RenderPlan};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::thumbnails::size_for;
use tessel_editor_core::{EditorState, headless};
use tessel_editor_media::host::{MediaHost, RendererFactory};
use tessel_editor_render::{RuntimeConfig, session};

/// The scene file's background (`[0.04, 0.05, 0.1]`) as 8-bit channels.
const BACKGROUND: [i32; 3] = [10, 13, 26];

/// Forwards to the real session, asking it for `probes` of every frame it draws.
struct Spy {
    inner: Box<dyn Renderer>,
    probes: Vec<(u32, u32)>,
    seen: Arc<Mutex<Vec<(String, FrameResult)>>>,
}

impl Renderer for Spy {
    fn render_frame(
        &mut self,
        state: &EditorState,
        plan: RenderPlan,
        output: Output,
        probes: Vec<(u32, u32)>,
        timeout: Duration,
    ) -> Result<FrameResult, BackendError> {
        let PreviewIntent::Source { asset, .. } = plan.intent.clone() else {
            panic!("a thumbnail is a source plan");
        };
        // The adapter's own probes (what tells an empty picture) come first and go back to it;
        // the spy's come after them and are kept for the assertions, indexed from zero.
        let asked = probes.len();
        let mut every = probes;
        every.extend(self.probes.iter().copied());
        let mut result = self
            .inner
            .render_frame(state, plan, output, every, timeout)?;
        let mut kept = result.clone();
        kept.probes = result.probes.get(asked..).unwrap_or_default().to_vec();
        result.probes.truncate(asked);
        self.seen.lock().unwrap().push((asset.to_string(), kept));
        Ok(result)
    }

    fn capabilities(&mut self) -> Result<Capabilities, BackendError> {
        self.inner.capabilities()
    }

    fn summary(&self) -> Value {
        self.inner.summary()
    }

    fn reset_project(&mut self) {
        self.inner.reset_project();
    }
}

/// Pixel of a scene-space point seen from the default scene camera (`[0,1,5]` looking at the
/// origin, 50 degrees vertical) on a `width` × `height` picture.
fn project(point: [f64; 3], width: u32, height: u32) -> (u32, u32) {
    let eye: [f64; 3] = [0.0, 1.0, 5.0];
    let forward = {
        let to = [-eye[0], -eye[1], -eye[2]];
        let length = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
        [to[0] / length, to[1] / length, to[2] / length]
    };
    // right = forward x up(0,1,0); up' = right x forward.
    let right = {
        let r: [f64; 3] = [-forward[2], 0.0, forward[0]];
        let length = (r[0] * r[0] + r[2] * r[2]).sqrt();
        [r[0] / length, 0.0, r[2] / length]
    };
    let up = [
        right[1] * forward[2] - right[2] * forward[1],
        right[2] * forward[0] - right[0] * forward[2],
        right[0] * forward[1] - right[1] * forward[0],
    ];
    let offset = [point[0] - eye[0], point[1] - eye[1], point[2] - eye[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let depth = dot(offset, forward);
    let focal = f64::from(height) / 2.0 / (25f64.to_radians()).tan();
    let x = f64::from(width) / 2.0 + focal * dot(offset, right) / depth;
    let y = f64::from(height) / 2.0 - focal * dot(offset, up) / depth;
    (x.round() as u32, y.round() as u32)
}

fn png_size(path: &str) -> (u32, u32) {
    let bytes = fs::read(path).unwrap();
    assert_eq!(&bytes[1..4], b"PNG");
    let word = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    (word(16), word(20))
}

fn distance(pixel: [u8; 4], reference: [i32; 3]) -> i32 {
    (0..3)
        .map(|c| (i32::from(pixel[c]) - reference[c]).abs())
        .max()
        .unwrap()
}

#[test]
fn the_real_runtime_draws_a_scene_and_a_bare_splat_into_their_thumbnails() {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => {
            assert!(
                std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
                "the runtime is required but unavailable: {}",
                setup.message()
            );
            eprintln!("SKIPPED (runtime unavailable): {}", setup.message());
            return;
        }
    };
    let dir: PathBuf =
        std::env::temp_dir().join(format!("tessel-thumb-frames-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let set = write_standard_set(&dir.join("set")).unwrap();
    let mut state = EditorState::empty(&ProjectSettings {
        name: "Thumbnails".into(),
        workspace: Some(dir.join("project")),
        ..ProjectSettings::default()
    })
    .unwrap();
    // The default sequence is the canvas the thumbnails take their aspect from.
    assert_eq!((state.timeline.width, state.timeline.height), (1920, 1080));
    let (width, height) = size_for(state.timeline.width, state.timeline.height);

    let mut importer = MediaHost::new();
    let imported = headless::execute_ext(
        &mut state,
        None,
        Some(&mut importer),
        &json!({"op":"media.import","paths":[set.splat, set.scene]}),
    )
    .unwrap();
    assert_eq!(imported["applied"]["imported"], 2, "{imported}");
    let splat = state
        .library
        .items
        .iter()
        .find(|item| item.kind.label() == "splat")
        .unwrap()
        .id
        .to_string();
    let scene = state
        .library
        .items
        .iter()
        .find(|item| item.kind.label() == "scene")
        .unwrap()
        .id
        .to_string();

    // Probes: the box and the sphere of the scene file, the cloud at the origin, two corners.
    let box_at = project([-2.2, 0.0, 0.0], width, height);
    let sphere_at = project([2.2, 0.0, 0.0], width, height);
    let cloud_at = project([0.0, 0.0, 0.0], width, height);
    let middle = (width / 2, height / 2);
    let probes = vec![
        box_at,
        sphere_at,
        cloud_at,
        (2, 2),
        (width - 3, height - 3),
        middle,
    ];
    let seen = Arc::new(Mutex::new(Vec::new()));
    let factory: Arc<RendererFactory> = {
        let (seen, probes, config) = (seen.clone(), probes.clone(), config.clone());
        Arc::new(move || {
            Ok(Box::new(Spy {
                inner: Box::new(session(&config)),
                probes: probes.clone(),
                seen: seen.clone(),
            }))
        })
    };
    let mut host = MediaHost::new().with_renderer(factory);
    let report = headless::execute_ext(
        &mut state,
        None,
        Some(&mut host),
        &json!({"op":"media.thumbnails"}),
    )
    .unwrap();
    assert_eq!(report["ready"], 2, "{report}");

    let frames = seen.lock().unwrap().clone();
    let of = |asset: &str| -> &FrameResult {
        &frames
            .iter()
            .find(|(name, _)| name == asset)
            .unwrap_or_else(|| panic!("{asset} in {frames:?}"))
            .1
    };
    assert_eq!(frames.len(), 2, "one scene and one stand-in: {frames:?}");
    let scene_frame = of(&scene);
    let splat_frame = &frames.iter().find(|(name, _)| name != &scene).unwrap().1;

    // What reached the library are the files of those frames, at the size the project asks for.
    for (id, frame) in [(&scene, scene_frame), (&splat, splat_frame)] {
        let thumbnail = state
            .thumbnail(id)
            .unwrap_or_else(|| panic!("a thumbnail for {id}"));
        assert_eq!((thumbnail.width, thumbnail.height), (width, height));
        assert_eq!(png_size(&thumbnail.path), (width, height), "{id}");
        assert_eq!((frame.width, frame.height), (width, height));
    }

    // The scene: orange box on the left, blue sphere on the right, the cloud in the middle.
    let pixel = |frame: &FrameResult, index: usize| frame.probes[index];
    let (b, s, c) = (
        pixel(scene_frame, 0),
        pixel(scene_frame, 1),
        pixel(scene_frame, 2),
    );
    assert!(
        i32::from(b[0]) > i32::from(b[2]) + 60,
        "the box is orange at {box_at:?}: {b:?}"
    );
    assert!(
        i32::from(s[2]) > i32::from(s[0]) + 60,
        "the sphere is blue at {sphere_at:?}: {s:?}"
    );
    assert!(
        distance(c, BACKGROUND) > 40,
        "the cloud is at the center {cloud_at:?}: {c:?}"
    );
    for corner in [3, 4] {
        assert!(
            distance(pixel(scene_frame, corner), BACKGROUND) <= 12,
            "corner {corner}: {:?}",
            pixel(scene_frame, corner)
        );
    }

    // The bare splat: the cloud and nothing else (no box, no sphere), on the same background. It
    // is framed from its own file, so the cloud is in the middle of the picture, not where the
    // scenes' default camera sees the origin.
    assert!(
        distance(pixel(splat_frame, 0), BACKGROUND) <= 12,
        "no box in the splat thumbnail: {:?}",
        pixel(splat_frame, 0)
    );
    assert!(
        distance(pixel(splat_frame, 1), BACKGROUND) <= 12,
        "no sphere in the splat thumbnail: {:?}",
        pixel(splat_frame, 1)
    );
    assert!(
        distance(pixel(splat_frame, 5), BACKGROUND) > 40,
        "the cloud is at the middle of the picture {middle:?}: {:?}",
        pixel(splat_frame, 5)
    );
    for corner in [3, 4] {
        assert!(
            distance(pixel(splat_frame, corner), BACKGROUND) <= 12,
            "corner {corner}"
        );
    }
    assert_ne!(scene_frame.hash, splat_frame.hash, "two different pictures");
}

/// A dense, bright ball of `count` splat records of `radius` around `center` (the 32-byte layout).
fn ball(center: [f32; 3], radius: f32, count: usize) -> Vec<u8> {
    let mut seed = 0x2545_f491_u32;
    let mut next = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
    };
    let mut bytes = Vec::new();
    let mut made = 0;
    while made < count {
        let offset = [next(), next(), next()];
        if offset.iter().map(|v| v * v).sum::<f32>() > 1.0 {
            continue;
        }
        made += 1;
        for axis in 0..3 {
            bytes.extend_from_slice(&(center[axis] + offset[axis] * radius).to_le_bytes());
        }
        for _ in 0..3 {
            bytes.extend_from_slice(&(radius * 0.05).to_le_bytes());
        }
        bytes.extend_from_slice(&[230, 180, 90, 255]);
        bytes.extend_from_slice(&[255, 128, 128, 128]);
    }
    bytes
}

/// A cloud that is nowhere near the origin, in a different size, is still in the picture: the
/// frame is framed from the file, not from where scene thumbnails happen to look.
#[test]
fn a_displaced_cloud_of_any_size_is_in_the_middle_of_its_thumbnail() {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => {
            assert!(
                std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
                "the runtime is required but unavailable: {}",
                setup.message()
            );
            eprintln!("SKIPPED (runtime unavailable): {}", setup.message());
            return;
        }
    };
    let dir: PathBuf =
        std::env::temp_dir().join(format!("tessel-thumb-displaced-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("set")).unwrap();
    // Spark stores a splat's centre as three float16 values: that is the runtime's limit, not the
    // thumbnail's. `capture` is the biggest cloud it can hold (tens of thousands of units from the
    // origin, every coordinate inside +-65504); `beyond` is past that and cannot be drawn.
    let cases = [
        ("far", [1000.0, -200.0, 300.0], 5.0),
        ("tiny", [0.2, 0.1, -0.3], 0.01),
        ("capture", [-30_000.0, 8_000.0, 40_000.0], 3_000.0),
    ];
    let beyond = ("beyond", [-80_000.0, 20_000.0, 90_000.0], 3_000.0);
    let mut paths = Vec::new();
    for (name, center, radius) in cases.into_iter().chain([beyond]) {
        let path = dir.join("set").join(format!("{name}.splat"));
        fs::write(&path, ball(center, radius, 3000)).unwrap();
        paths.push(path);
    }
    let mut state = EditorState::empty(&ProjectSettings {
        name: "Displaced".into(),
        workspace: Some(dir.join("project")),
        ..ProjectSettings::default()
    })
    .unwrap();
    let (width, height) = size_for(state.timeline.width, state.timeline.height);
    let mut importer = MediaHost::new();
    let imported = headless::execute_ext(
        &mut state,
        None,
        Some(&mut importer),
        &json!({"op":"media.import","paths":paths}),
    )
    .unwrap();
    assert_eq!(imported["applied"]["imported"], 4, "{imported}");

    let probes = vec![(width / 2, height / 2), (2, 2), (width - 3, height - 3)];
    let seen = Arc::new(Mutex::new(Vec::new()));
    let factory: Arc<RendererFactory> = {
        let (seen, probes, config) = (seen.clone(), probes.clone(), config.clone());
        Arc::new(move || {
            Ok(Box::new(Spy {
                inner: Box::new(session(&config)),
                probes: probes.clone(),
                seen: seen.clone(),
            }))
        })
    };
    let mut host = MediaHost::new().with_renderer(factory);
    let report = headless::execute_ext(
        &mut state,
        None,
        Some(&mut host),
        &json!({"op":"media.thumbnails"}),
    )
    .unwrap();
    assert_eq!(report["ready"], 3, "{report}");
    // What the runtime cannot hold comes out as the background only: named, never offered as a picture.
    let lost = state.thumbnail_report(beyond.0);
    assert_eq!(lost["state"], "failed", "{lost}");
    assert_eq!(lost["code"], "empty_picture", "{lost}");
    assert!(state.thumbnail(beyond.0).is_none());

    let frames = seen.lock().unwrap().clone();
    for (name, _, _) in cases {
        let id = state.thumbnail_stand_in_id(name);
        let frame = &frames
            .iter()
            .find(|(asset, _)| *asset == id)
            .unwrap_or_else(|| panic!("{id} in {frames:?}"))
            .1;
        assert!(
            distance(frame.probes[0], BACKGROUND) > 40,
            "{name}: the cloud is in the middle: {:?}",
            frame.probes[0]
        );
        for corner in [1, 2] {
            assert!(
                distance(frame.probes[corner], BACKGROUND) <= 12,
                "{name}: corner {corner} is the background: {:?}",
                frame.probes[corner]
            );
        }
        let thumbnail = state
            .thumbnail(name)
            .unwrap_or_else(|| panic!("a thumbnail for {name}"));
        assert_eq!(png_size(&thumbnail.path), (width, height), "{name}");
    }
    let _ = fs::remove_dir_all(&dir);
}

/// What each of the five SHARP fixture points looks like on the dark background, as the channels
/// that must be lit (red, green, blue, yellow, white).
const DOMINANT: [[bool; 3]; 5] = [
    [true, false, false],
    [false, true, false],
    [false, false, true],
    [true, true, false],
    [true, true, true],
];

fn lit_as(color: [u8; 4], dominant: [bool; 3]) -> bool {
    (0..3).all(|channel| {
        if dominant[channel] {
            color[channel] > 150
        } else {
            color[channel] < 90
        }
    })
}

/// Draws the thumbnail of the SHARP fixture imported with `capture` and returns the pixels under
/// the five points' expected positions, in the order of `SHARP_FIXTURE_PIXELS`.
fn sharp_probes(config: &RuntimeConfig, dir: &std::path::Path, capture: &str) -> Vec<[u8; 4]> {
    let set = write_standard_set(&dir.join(format!("set-{capture}"))).unwrap();
    let mut state = EditorState::empty(&ProjectSettings {
        name: format!("Sharp {capture}"),
        workspace: Some(dir.join(format!("project-{capture}"))),
        ..ProjectSettings::default()
    })
    .unwrap();
    let (width, height) = size_for(state.timeline.width, state.timeline.height);
    let mut importer = MediaHost::new();
    let imported = headless::execute_ext(
        &mut state,
        None,
        Some(&mut importer),
        &json!({"op":"media.import","paths":[set.sharp],"capture":capture}),
    )
    .unwrap();
    assert_eq!(imported["applied"]["imported"], 1, "{imported}");

    // The oracle is the source picture, not the renderer and not the core's camera: the fixture puts
    // each Gaussian over a known pixel of its 160 x 120 image; the thumbnail keeps the lens (the
    // vertical field of view), so that pixel is where the same height-scaled, centred picture has it.
    let scale = f64::from(height) / 120.0;
    let probes: Vec<(u32, u32)> = SHARP_FIXTURE_PIXELS
        .iter()
        .map(|pixel| {
            (
                (f64::from(width) / 2.0 + (pixel[0] - 80.0) * scale).round() as u32,
                (f64::from(height) / 2.0 + (pixel[1] - 60.0) * scale).round() as u32,
            )
        })
        .collect();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let factory: Arc<RendererFactory> = {
        let (seen, probes, config) = (seen.clone(), probes.clone(), config.clone());
        Arc::new(move || {
            Ok(Box::new(Spy {
                inner: Box::new(session(&config)),
                probes: probes.clone(),
                seen: seen.clone(),
            }))
        })
    };
    let mut host = MediaHost::new().with_renderer(factory);
    let report = headless::execute_ext(
        &mut state,
        None,
        Some(&mut host),
        &json!({"op":"media.thumbnails"}),
    )
    .unwrap();
    let thumbnail = state.thumbnail_report("sharp");
    // A plain reading may draw or fail as an empty picture; either way this is what it says.
    if capture == "auto" {
        assert_eq!(report["ready"], 1, "{report} {thumbnail}");
        let made = state.thumbnail("sharp").expect("a thumbnail");
        assert_eq!(png_size(&made.path), (width, height));
    }
    let id = state.thumbnail_stand_in_id("sharp");
    let frames = seen.lock().unwrap().clone();
    frames
        .iter()
        .find(|(asset, _)| *asset == id)
        .map(|(_, frame)| frame.probes.clone())
        .unwrap_or_default()
}

/// BL144.2: the real runtime draws a SHARP splat through its capture, upright, over the pixels the
/// source image had; read as a plain splat (`capture: ignore`) the same file does not.
#[test]
fn a_sharp_splat_thumbnail_is_the_source_picture_upright_and_a_plain_reading_is_not() {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => {
            assert!(
                std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
                "the runtime is required but unavailable: {}",
                setup.message()
            );
            eprintln!("SKIPPED (runtime unavailable): {}", setup.message());
            return;
        }
    };
    let dir: PathBuf =
        std::env::temp_dir().join(format!("tessel-thumb-sharp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let seen = sharp_probes(&config, &dir, "auto");
    assert_eq!(seen.len(), 5);
    for (index, dominant) in DOMINANT.iter().enumerate() {
        assert!(
            lit_as(seen[index], *dominant),
            "point {index} ({:?}) over source pixel {:?}: {:?}",
            dominant,
            SHARP_FIXTURE_PIXELS[index],
            seen[index]
        );
    }

    // Mutation: the same file framed as any splat (no capture) is not that picture.
    let plain = sharp_probes(&config, &dir, "ignore");
    let hits = DOMINANT
        .iter()
        .enumerate()
        .filter(|(index, dominant)| plain.get(*index).is_some_and(|c| lit_as(*c, **dominant)))
        .count();
    assert!(
        hits < 5,
        "a plain reading matched {hits} of 5 source pixels: {plain:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}
