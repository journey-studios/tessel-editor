//! F061 (G4.1, BL082.35): the vignette handles against the real runtime (Node worker + headless
//! Chrome, SwiftShader) and a real export (FFmpeg), over the synthetic corpus of
//! `scripts/directed-scene-proof`.
//!
//! What it proves:
//!
//! - **The handles sit where the picture changes.** The positions come from the core
//!   (`scene.fx.handles`); the pixels from the runtime. Every probe is compared with the core's
//!   reference of the pixel math (`apply_pass_to_srgb`, written independently of the runtime's
//!   GLSL), and the darkening profile along the row of the centre is read at the handle points: the
//!   picture is untouched at and inside the `radius` handle, fully shaded at and beyond the
//!   `softness` handle, and in between it only gets darker. A control with the handles moved
//!   (a drag, the same document as `scene.fx.edit`) moves the profile.
//! - **The handles are not in the picture.** The export of the document, `scene.render` and the
//!   export taken **while the clip is targeted, the overlay is described and a handle drag is open**
//!   (its candidate differs from the document) have the same frame hashes; only the release, which
//!   commits, changes the export (the control that makes the comparison discriminating).
//!
//! What it does not prove: what the page draws (`runtime/test/fx-handles-overlay.test.mjs`), the
//! window, WKWebView, a physical pointer, performance.
//!
//! Skipped (reason on stderr) without Chrome, the runtime bundle or FFmpeg; set
//! `TESSEL_REQUIRE_RUNTIME=1` to turn the skip into a failure. Run under `with-gpu-lock.py`.

mod scene_support;

use std::fs;
use std::path::PathBuf;

use scene_support::{corpus, plan, resources};
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::apply_pass_to_srgb;
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::tools::Tools;
use tessel_editor_render::scene_host::SceneHost;
use tessel_editor_render::{
    FrameOptions, HANDSHAKE_TIMEOUT, ProcessTransport, RuntimeConfig, SceneRenderer,
};

/// 8-bit rounding in the GPU target and in the sRGB curves.
const TOLERANCE: i32 = 3;
/// The edit frame every picture is taken at, inside the clip (0..48).
const FRAME: i64 = 12;

fn skip(reason: &str) {
    assert!(
        std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
        "the runtime is required but unavailable: {reason}"
    );
    eprintln!("SKIPPED (runtime unavailable): {reason}");
}

struct Session {
    state: EditorState,
    host: SceneHost,
    work: PathBuf,
}

impl Session {
    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        headless::execute_ext(&mut self.state, None, Some(&mut self.host), &request)
    }

    fn ok(&mut self, request: Value) -> Value {
        self.run(request.clone())
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    fn handles(&mut self, frame: i64) -> Value {
        self.ok(json!({"op":"scene.fx.handles","scene":"street","clip":"v1","frame":frame}))
    }

    fn export(&mut self, name: &str, from: i64, frames: i64) -> Vec<String> {
        let destination = self.work.join(name);
        let result = self.ok(
            json!({"op":"scene.export","scene":"street","preset":"fullhd",
            "from":from,"frames":frames,"destination":destination}),
        );
        assert_eq!(result["verification"]["ok"], true);
        result["frame_hashes"]["list"]
            .as_array()
            .expect("a short export lists every frame hash")
            .iter()
            .map(|hash| hash.as_str().unwrap().to_string())
            .collect()
    }

    fn rendered_hash(&mut self, frame: i64) -> String {
        self.ok(json!({"op":"scene.render","scene":"street","frame":frame,
            "width":1920,"height":1080}))["frame"]["hash"]
            .as_str()
            .unwrap()
            .to_string()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.work);
    }
}

fn session(name: &str, params: Value) -> Option<Session> {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => {
            skip(&setup.message());
            return None;
        }
    };
    if let Err(error) = Tools::discover() {
        skip(&error.message);
        return None;
    }
    let work = std::env::temp_dir().join(format!("te-fxh-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let mut session = Session {
        state: EditorState::empty(&ProjectSettings::default()).unwrap(),
        host: SceneHost::new(MediaHost::new(), config),
        work,
    };
    session.ok(json!({"op":"scene.attach","id":"street","path":corpus().join("bundle.json")}));
    session.ok(json!({"op":"scene.resources.resolve","scene":"street","base":corpus()}));
    session.ok(json!({"op":"scene.fx.edit","scene":"street","ops":[
        {"op":"add_clip","effect":"vignette","start_frame":0,"duration_frames":48,
         "id":"v1","params":params}]}));
    Some(session)
}

fn at_of(answer: &Value, id: &str) -> (f64, f64) {
    let handle = answer["handles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|handle| handle["id"] == id)
        .unwrap_or_else(|| panic!("no handle `{id}` in {answer}"));
    (
        handle["at"]["u"].as_f64().unwrap(),
        handle["at"]["v"].as_f64().unwrap(),
    )
}

/// The pixel a picture position falls on (pixel centres sit at `(i + 0.5) / size`).
fn pixel_of(at: (f64, f64), size: (usize, usize)) -> (u32, u32) {
    (
        (at.0 * size.0 as f64 - 0.5).round() as u32,
        (at.1 * size.1 as f64 - 0.5).round() as u32,
    )
}

fn renderer() -> Option<SceneRenderer<ProcessTransport>> {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => {
            skip(&setup.message());
            return None;
        }
    };
    let backend = tessel_editor_core::bridge::BridgeBackend::new(
        "three-worker",
        ProcessTransport::new(config),
        HANDSHAKE_TIMEOUT,
    );
    let mut renderer = SceneRenderer::new(backend);
    match renderer.ensure_ready() {
        Ok(_) => Some(renderer),
        Err(error) if error.code == "backend_unavailable" && !error.message.contains("Rebuild") => {
            skip(&error.message);
            None
        }
        Err(error) => panic!("{error:?}"),
    }
}

/// How much of the baseline is left at a probe: the sum of the shaded channels over the sum of
/// the baseline's (`None` where the baseline is too dark for a ratio to mean anything).
fn left(base: [u8; 4], shaded: [u8; 4]) -> Option<f64> {
    let (b, s): (i32, i32) = (
        (0..3).map(|c| i32::from(base[c])).sum(),
        (0..3).map(|c| i32::from(shaded[c])).sum(),
    );
    (b >= 240).then(|| f64::from(s) / f64::from(b))
}

/// The probes of one picture: the core-reported handle points and a profile along the centre row.
struct Reading {
    size: (usize, usize),
    centre: (u32, u32),
    radius: (u32, u32),
    softness: (u32, u32),
    profile: Vec<u32>,
}

impl Reading {
    fn of(answer: &Value, size: (usize, usize)) -> Self {
        let centre = pixel_of(at_of(answer, "center"), size);
        let radius = pixel_of(at_of(answer, "radius"), size);
        let softness = pixel_of(at_of(answer, "softness"), size);
        assert_eq!((radius.1, softness.1), (centre.1, centre.1), "one row");
        let profile = (centre.0..size.0 as u32).step_by(3).collect();
        Self {
            size,
            centre,
            radius,
            softness,
            profile,
        }
    }

    fn points(&self) -> Vec<(u32, u32)> {
        let mut points = vec![self.centre, self.radius, self.softness];
        points.extend(self.profile.iter().map(|x| (*x, self.centre.1)));
        points
    }
}

fn take(
    renderer: &mut SceneRenderer<ProcessTransport>,
    scene: &tessel_editor_core::directed_scene::DirectedScene,
    points: Vec<(u32, u32)>,
) -> (tessel_editor_core::scene_plan::ScenePlan, Vec<[u8; 4]>) {
    let plan = plan(scene, FRAME);
    let result = renderer
        .render(
            &plan,
            &resources(1, &plan),
            &FrameOptions {
                probes: points,
                ..FrameOptions::default()
            },
        )
        .unwrap();
    (plan, result.probes)
}

#[test]
fn the_picture_darkens_from_the_radius_handle_to_the_softness_handle() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let Some(mut session) = session(
        "profile",
        json!({"intensity":1.0,"radius":0.35,"softness":0.25,"roundness":0.6,
               "center":{"x":0.3,"y":0.5}}),
    ) else {
        return;
    };
    // The same document without the clip: the baseline the shade is judged against.
    let baseline_scene = tessel_editor_core::scene_edit::apply_fx(
        &session.state.scenes.get("street").unwrap().scene,
        &[tessel_editor_core::scene_edit::FxOp::from_json(
            &json!({"op":"remove_clip","clip":"v1"}),
        )
        .unwrap()],
    )
    .unwrap()
    .candidate;

    let answer = session.handles(FRAME);
    assert_eq!(answer["visible"], true, "{answer}");
    let shaded_scene = session.state.scenes.get("street").unwrap().scene.clone();
    let probe_plan = plan(&shaded_scene, FRAME);
    let size = (
        probe_plan.output.width as usize,
        probe_plan.output.height as usize,
    );
    let reading = Reading::of(&answer, size);
    let points = reading.points();

    let (_, base) = take(&mut renderer, &baseline_scene, points.clone());
    let (plan_now, shaded) = take(&mut renderer, &shaded_scene, points.clone());

    // Every probe against the reference of the pixel math.
    for ((point, base), read) in points.iter().zip(&base).zip(&shaded) {
        let mut want = [
            f64::from(base[0]) / 255.0,
            f64::from(base[1]) / 255.0,
            f64::from(base[2]) / 255.0,
        ];
        for pass in &plan_now.fx {
            want = apply_pass_to_srgb(
                pass,
                want,
                point.0 as usize,
                point.1 as usize,
                size.0,
                size.1,
            )
            .unwrap();
        }
        for channel in 0..3 {
            let expected = (want[channel] * 255.0).round() as i32;
            assert!(
                (i32::from(read[channel]) - expected).abs() <= TOLERANCE,
                "probe {point:?} channel {channel} read {read:?}, the reference says {expected} (baseline {base:?})"
            );
        }
    }

    // The profile, read at the handle points the core reports.
    let (centre, radius, softness) = (0, 1, 2);
    let at_centre = left(base[centre], shaded[centre]).expect("a bright centre");
    assert!(
        at_centre > 0.97,
        "the centre is not touched: {at_centre} of the baseline is left"
    );
    if let Some(at_radius) = left(base[radius], shaded[radius]) {
        assert!(
            at_radius > 0.95,
            "the falloff starts at the radius handle: {at_radius} left"
        );
    }
    let mut seen = 0;
    let mut last: Option<f64> = None;
    for (slot, x) in reading.profile.iter().enumerate() {
        let index = 3 + slot;
        let Some(here) = left(base[index], shaded[index]) else {
            continue;
        };
        seen += 1;
        if let Some(before) = last {
            assert!(
                here <= before + 0.03,
                "the picture only gets darker outwards: x={x} keeps {here} after {before}"
            );
        }
        last = Some(here);
        if *x >= reading.softness.0 + 2 {
            assert!(
                here < 0.08,
                "beyond the softness handle the picture is fully shaded: x={x} keeps {here}"
            );
        }
    }
    assert!(seen >= 5, "enough of the profile is readable: {seen}");
    if let Some(at_softness) = left(base[softness], shaded[softness]) {
        assert!(
            at_softness < 0.08,
            "the falloff ends at the softness handle: {at_softness} left"
        );
    }
    assert!(
        reading.radius.0 < reading.softness.0 && reading.centre.0 < reading.radius.0,
        "{reading_size:?}: centre, radius and softness handles in order",
        reading_size = reading.size
    );

    // The control: the same document edited the way a drag edits it (`scene.fx.edit`), the radius
    // handle moved outwards, moves the profile (a reading that never changes proves nothing).
    session.ok(json!({"op":"scene.fx.edit","scene":"street","ops":[
        {"op":"set_param","clip":"v1","param":"radius","value":0.5}]}));
    let moved_scene = session.state.scenes.get("street").unwrap().scene.clone();
    let (_, moved) = take(&mut renderer, &moved_scene, points.clone());
    assert_ne!(moved, shaded, "a moved handle moves the pixels");
    let moved_at_old_radius = left(base[radius], moved[radius]);
    if let (Some(before), Some(after)) = (left(base[radius], shaded[radius]), moved_at_old_radius) {
        assert!(
            after >= before,
            "the old radius point is less shaded once the radius grew"
        );
    }
    eprintln!(
        "EVIDENCE handles {size:?}: centre {:?}, radius {:?}, softness {:?}; profile samples {seen}",
        reading.centre, reading.radius, reading.softness
    );
    renderer.shutdown();
}

#[test]
fn the_export_is_the_document_with_the_handles_on_and_a_drag_open() {
    let Some(mut session) = session(
        "export",
        json!({"intensity":1.0,"radius":0.35,"softness":0.25,"roundness":0.6,
               "center":{"x":0.5,"y":0.5}}),
    ) else {
        return;
    };
    let (from, frames) = (4, 4);
    let at = session.handles(from);
    let radius = at_of(&at, "radius");
    let depth = session.ok(json!({"op":"history.status"}))["undo_depth"].clone();

    // The document, nothing targeted: the export and what scene.render draws.
    let plain = session.export("plain.mp4", from, frames);
    for (index, hash) in plain.iter().enumerate() {
        assert_eq!(
            *hash,
            session.rendered_hash(from + index as i64),
            "export frame {index} is not what scene.render draws"
        );
    }

    // The overlay on: the clip targeted, its overlay described, a drag open on the radius handle
    // with a candidate that is not the document.
    session.ok(json!({"op":"scene.fx.select","scene":"street","clip":"v1"}));
    let overlay = session.ok(json!({"op":"scene.gizmo.state","scene":"street","frame":from}));
    assert_eq!(overlay["overlay"]["target"], "fx", "{overlay}");
    session.ok(
        json!({"op":"scene.pointer.down","scene":"street","frame":from,
        "point":{"u":radius.0,"v":radius.1}}),
    );
    session.ok(json!({"op":"scene.pointer.move","point":{"u":radius.0 + 0.08,"v":radius.1}}));
    let live = session.handles(from);
    assert_eq!(live["active"], "radius", "the drag is open: {live}");
    let during = session.export("during.mp4", from, frames);
    assert_eq!(
        during, plain,
        "an export taken with the handles on and a drag open is the document's"
    );
    for (index, hash) in during.iter().enumerate() {
        assert_eq!(*hash, session.rendered_hash(from + index as i64));
    }
    assert_eq!(
        session.ok(json!({"op":"history.status"}))["undo_depth"],
        depth,
        "nothing was committed under the open drag"
    );

    // The control: the release commits the candidate and only then does the export change.
    session.ok(json!({"op":"scene.pointer.up","point":{"u":radius.0 + 0.08,"v":radius.1}}));
    let committed = session.export("committed.mp4", from, frames);
    assert_ne!(
        committed, plain,
        "the committed radius is in the export (the comparison discriminates)"
    );
}
