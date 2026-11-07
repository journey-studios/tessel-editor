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
use tessel_editor_core::fixtures::write_standard_set;
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
        _probes: Vec<(u32, u32)>,
        timeout: Duration,
    ) -> Result<FrameResult, BackendError> {
        let PreviewIntent::Source { asset, .. } = plan.intent.clone() else {
            panic!("a thumbnail is a source plan");
        };
        let result = self
            .inner
            .render_frame(state, plan, output, self.probes.clone(), timeout)?;
        self.seen
            .lock()
            .unwrap()
            .push((asset.to_string(), result.clone()));
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
    let probes = vec![box_at, sphere_at, cloud_at, (2, 2), (width - 3, height - 3)];
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
    let (scene_frame, splat_frame) = (of(&scene), of(&format!("thumb-{splat}")));

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

    // The bare splat: the cloud and nothing else (no box, no sphere), on the same background.
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
        distance(pixel(splat_frame, 2), BACKGROUND) > 40,
        "the cloud is at the center: {:?}",
        pixel(splat_frame, 2)
    );
    for corner in [3, 4] {
        assert!(
            distance(pixel(splat_frame, corner), BACKGROUND) <= 12,
            "corner {corner}"
        );
    }
    assert_ne!(scene_frame.hash, splat_frame.hash, "two different pictures");
}
