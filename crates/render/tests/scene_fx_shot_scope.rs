//! F062: effects with the `shot` scope, drawn by the real runtime (Node worker + headless Chrome,
//! SwiftShader) over the two migration fixtures with two shots:
//!
//! - `valid-minimal-continuous`: `shot-1` is `[0, 48)` and `shot-2` is `[48, 96)`;
//! - `valid-cut-instance-offsets`: `shot-a` is `[0, 40)` and `shot-b` is `[40, 100)`, joined by a cut,
//!   each shot on its own instance with its own offsets.
//!
//! The fixtures carry synthetic hashes. The test lays each one out in a temporary folder with the
//! corpus pictures of `scripts/directed-scene-proof` standing in for its assets (same sizes, same
//! alpha modes) and the real hashes written over the synthetic ones; nothing private is used.
//!
//! What it proves:
//!
//! - a `shot` clip darkens the frames of its shot only (every probe of a grid against the core's
//!   reference of the pixel math, which is written independently of the runtime's GLSL), and the
//!   frames of the other shot are the baseline's, hash for hash, with one frame on each side of the
//!   boundary;
//! - without other passes a `shot` clip draws the very picture of an `output` clip over the same
//!   frames (equal hashes);
//! - the two stages do not commute and the picture follows the stage order. The expectation is
//!   written HERE: the deck pass (the `shot` fade) first and the output pass (the vignette) after it,
//!   whatever the track order is; the passes are only looked up by effect, never by the order the
//!   plan lists them. The same two effects, both `output`, in the opposite track order give the
//!   other picture, and the reference agrees with each;
//! - a plan with a `shot` pass asks for `fxShotScope` and a runtime without it is refused by name
//!   before any asset or frame is sent; a plan with only `output` passes does not ask for it;
//! - an export carries the same pictures `scene.render` draws for the same edit frames, with two
//!   frames on each side of the boundary, and `set_scope` between `shot` and `output` over the same
//!   frames does not change a pixel of the export.
//!
//! A machine without Chrome, the runtime bundle or FFmpeg skips what needs them with the reason on
//! stderr; set `TESSEL_REQUIRE_RUNTIME=1` (CI) to turn the skip into a failure.

mod scene_support;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use scene_support::corpus;
use serde_json::{Value, json};
use tessel_editor_core::backend::{BackendError, FrameResult};
use tessel_editor_core::bridge::{BridgeBackend, Transport};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::directed_scene::fx::{FxPass, FxScope, apply_pass_to_srgb};
use tessel_editor_core::scene_edit::{FxOp, apply_fx};
use tessel_editor_core::scene_plan::{Capability, ScenePlan};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::tools::Tools;
use tessel_editor_render::scene::import_gated;
use tessel_editor_render::scene_host::SceneHost;
use tessel_editor_render::{
    FrameOptions, HANDSHAKE_TIMEOUT, ProcessTransport, RuntimeConfig, SceneRenderer,
    SceneResourceMap,
};

/// 8-bit rounding in the GPU target and in the sRGB curves: a channel may move a unit or two.
const TOLERANCE: i32 = 3;

// ---- the fixtures, laid out on disk ---------------------------------------------------------------

type Shots = [(&'static str, i64, i64); 2];

/// `valid-minimal-continuous`: two shots of one instance over 96 frames.
const MINIMAL: Shots = [("shot-1", 0, 48), ("shot-2", 48, 96)];
/// `valid-cut-instance-offsets`: two shots of two instances, joined by a cut, over 100 frames.
const CUT: Shots = [("shot-a", 0, 40), ("shot-b", 40, 100)];

/// One migration fixture in a temporary folder: `bundle.json` with the real hashes and
/// `assets/backdrop.png`, `assets/figure.png` next to it. The folder goes away with the value.
struct Fixture {
    dir: PathBuf,
    text: String,
    shots: Shots,
}

impl Fixture {
    fn new(name: &str, tag: &str, shots: Shots) -> Self {
        let dir = std::env::temp_dir().join(format!("te-shot-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("assets")).unwrap();
        let corpus_bundle: Value =
            serde_json::from_str(&fs::read_to_string(corpus().join("bundle.json")).unwrap())
                .unwrap();
        let hash_of = |asset: &str| -> String {
            corpus_bundle["assets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["assetId"] == asset)
                .and_then(|entry| entry["contentHash"].as_str())
                .unwrap_or_else(|| panic!("the corpus has no asset `{asset}`"))
                .to_string()
        };
        // The corpus pictures stand in for the fixtures' synthetic assets: `sky` is 64 x 36 and
        // opaque like `backdrop`, `hero` is 32 x 32 and straight like `figure`.
        for (from, to) in [("sky", "backdrop"), ("hero", "figure")] {
            fs::copy(
                corpus().join(format!("assets/{from}.png")),
                dir.join(format!("assets/{to}.png")),
            )
            .unwrap();
        }
        let source = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/migration/fixtures/bundles")
                .join(format!("{name}.bundle.json")),
        )
        .unwrap();
        let text = source
            .replace("synthetic-sha256-backdrop", &hash_of("sky"))
            .replace("synthetic-sha256-figure", &hash_of("hero"));
        fs::write(dir.join("bundle.json"), &text).unwrap();
        Fixture { dir, text, shots }
    }

    fn minimal(tag: &str) -> Self {
        Self::new("valid-minimal-continuous", tag, MINIMAL)
    }

    fn cut(tag: &str) -> Self {
        Self::new("valid-cut-instance-offsets", tag, CUT)
    }

    /// The fixture through the renderer's own import gates.
    fn scene(&self) -> DirectedScene {
        import_gated(&self.text).unwrap()
    }

    fn bundle(&self) -> PathBuf {
        self.dir.join("bundle.json")
    }

    fn shot(&self, index: usize) -> (&'static str, i64, i64) {
        self.shots[index]
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

// ---- the real runtime -----------------------------------------------------------------------------

fn skip<T>(reason: &str) -> Option<T> {
    assert!(
        std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
        "the runtime is required but unavailable: {reason}"
    );
    eprintln!("SKIPPED (runtime unavailable): {reason}");
    None
}

fn renderer() -> Option<SceneRenderer<ProcessTransport>> {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => return skip(&setup.message()),
    };
    let backend = BridgeBackend::new(
        "three-worker",
        ProcessTransport::new(config),
        HANDSHAKE_TIMEOUT,
    );
    let mut renderer = SceneRenderer::new(backend);
    match renderer.ensure_ready() {
        Ok(_) => Some(renderer),
        Err(error) if error.code == "backend_unavailable" && !error.message.contains("Rebuild") => {
            skip(&error.message)
        }
        Err(error) => panic!("{error:?}"),
    }
}

fn grid(width: u32, height: u32) -> Vec<(u32, u32)> {
    let mut points = Vec::new();
    for row in 0..6 {
        for column in 0..9 {
            points.push(((column * (width - 1)) / 8, (row * (height - 1)) / 5));
        }
    }
    points
}

fn draw(
    renderer: &mut SceneRenderer<ProcessTransport>,
    fixture: &Fixture,
    scene: &DirectedScene,
    edit_frame: i64,
) -> (ScenePlan, FrameResult) {
    let plan = ScenePlan::build(scene, edit_frame).unwrap();
    let resources = SceneResourceMap::from_root(1, &plan, &fixture.dir).unwrap();
    let points = grid(plan.output.width as u32, plan.output.height as u32);
    let result = renderer
        .render(
            &plan,
            &resources,
            &FrameOptions {
                probes: points,
                ..FrameOptions::default()
            },
        )
        .unwrap();
    (plan, result)
}

// ---- the edits ------------------------------------------------------------------------------------

/// The fixture with `ops` applied through the core's atomic edit, as `scene.fx.edit` does.
fn edited(base: &DirectedScene, ops: &[Value]) -> DirectedScene {
    let ops: Vec<FxOp> = ops
        .iter()
        .map(|op| FxOp::from_json(op).unwrap_or_else(|error| panic!("{op}: {error:?}")))
        .collect();
    apply_fx(base, &ops).unwrap().candidate
}

fn clip(effect: &str, id: &str, scope: &str, start: i64, end: i64, params: Value) -> Value {
    json!({
        "op": "add_clip", "effect": effect, "id": id, "scope": scope,
        "start_frame": start, "duration_frames": end - start, "params": params,
    })
}

/// The vignette parameters of the proofs. The fixtures draw a picture that does not reach the edges
/// of the frame (a plane 8 x 4.5 seen from 8 units away), so the default radius would leave every
/// pixel it could darken at black already and some frames would not change at all. With radius 0
/// and softness 0.5 the shade grows from the centre and is full from half way out: the picture
/// darkens at every frame, whatever the camera does.
fn strong_vignette() -> Value {
    json!({ "intensity": 1.0, "radius": 0.0, "softness": 0.5 })
}

fn vignette(id: &str, scope: &str, start: i64, end: i64) -> Value {
    clip("vignette", id, scope, start, end, strong_vignette())
}

fn white_fade(id: &str, scope: &str, start: i64, end: i64, amount: f64) -> Value {
    clip(
        "fade",
        id,
        scope,
        start,
        end,
        json!({ "color": { "r": 1.0, "g": 1.0, "b": 1.0 }, "amount": amount }),
    )
}

// ---- the reference --------------------------------------------------------------------------------

/// The passes of `plan` in the order the TEST names, looked up by effect: never the order the plan
/// (that is, `Fx::resolve`) lists them in.
fn in_order<'a>(plan: &'a ScenePlan, effects: &[&str]) -> Vec<&'a FxPass> {
    effects
        .iter()
        .map(|effect| {
            let mut found = plan.fx.iter().filter(|pass| pass.effect == *effect);
            let pass = found
                .next()
                .unwrap_or_else(|| panic!("the plan has no `{effect}` pass: {:?}", plan.fx));
            assert!(found.next().is_none(), "two `{effect}` passes in one plan");
            pass
        })
        .collect()
}

/// The effects of `plan.fx` in the order the plan lists them. Only for asserting what the plan says;
/// the expectations of the pixels never take their order from here.
fn listed(plan: &ScenePlan) -> Vec<&str> {
    plan.fx.iter().map(|pass| pass.effect.as_str()).collect()
}

/// What the passes, applied in the order given, make of one baseline probe. The GPU target holds 8
/// bits between passes, so the reference rounds the same way.
fn expected(
    stages: &[&FxPass],
    base: [u8; 4],
    point: (u32, u32),
    size: (usize, usize),
) -> [i32; 3] {
    let mut want = [
        f64::from(base[0]) / 255.0,
        f64::from(base[1]) / 255.0,
        f64::from(base[2]) / 255.0,
    ];
    for pass in stages {
        want = apply_pass_to_srgb(
            pass,
            want,
            point.0 as usize,
            point.1 as usize,
            size.0,
            size.1,
        )
        .unwrap();
        want = want.map(|channel| (channel * 255.0).round() / 255.0);
    }
    want.map(|channel| (channel * 255.0).round() as i32)
}

/// Every probe of `read` against the reference applied to `baseline` through `stages` (in the
/// caller's order); returns the worst difference and the strongest change from the baseline.
fn assert_matches(
    plan: &ScenePlan,
    baseline: &FrameResult,
    read: &FrameResult,
    stages: &[&FxPass],
    tolerance: i32,
    what: &str,
) -> (i32, i32) {
    let size = (plan.output.width as usize, plan.output.height as usize);
    let points = grid(size.0 as u32, size.1 as u32);
    let (mut worst, mut strongest) = (0, 0);
    for ((point, base), got) in points.iter().zip(&baseline.probes).zip(&read.probes) {
        let want = expected(stages, *base, *point, size);
        for channel in 0..3 {
            let delta = (i32::from(got[channel]) - want[channel]).abs();
            worst = worst.max(delta);
            strongest = strongest.max((i32::from(got[channel]) - i32::from(base[channel])).abs());
            assert!(
                delta <= tolerance,
                "{what}: probe {point:?} channel {channel} read {got:?}, the reference says {want:?} (baseline {base:?})"
            );
        }
    }
    (worst, strongest)
}

fn brightest_gain(baseline: &FrameResult, read: &FrameResult) -> i32 {
    baseline
        .probes
        .iter()
        .zip(&read.probes)
        .flat_map(|(base, got)| (0..3).map(|c| i32::from(got[c]) - i32::from(base[c])))
        .max()
        .unwrap()
}

/// One frame from the middle of each shot plus the last frame of the first and the first of the
/// second: one frame on each side of the boundary.
fn frames_of(shots: &Shots) -> Vec<i64> {
    let (_, first_start, boundary) = shots[0];
    let (_, _, last_end) = shots[1];
    vec![
        first_start,
        (first_start + boundary) / 2,
        boundary - 1,
        boundary,
        (boundary + last_end) / 2,
        last_end - 1,
    ]
}

// ---- capability -----------------------------------------------------------------------------------

#[test]
fn the_runtime_advertises_the_shot_scope_and_only_a_shot_pass_asks_for_it() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let fixture = Fixture::minimal("capability");
    let capabilities = renderer.ensure_ready().unwrap().clone();
    let advertised = capabilities.raw["capabilities"]["directed_scene"]["capabilities"].to_string();
    assert!(
        advertised.contains(Capability::FxShotScope.as_str()),
        "{advertised}"
    );
    assert_eq!(Capability::FxShotScope.as_str(), "fxShotScope");

    let base = fixture.scene();
    let (plain, _) = draw(&mut renderer, &fixture, &base, 12);
    assert!(plain.fx.is_empty());
    assert!(!plain.requires.contains(&Capability::FxShotScope));

    // An `output` pass asks for `outputFx` and nothing about scopes.
    let output = edited(&base, &[vignette("v1", "output", 0, 48)]);
    let (output_plan, _) = draw(&mut renderer, &fixture, &output, 12);
    assert_eq!(output_plan.fx[0].scope, FxScope::Output);
    assert!(output_plan.requires.contains(&Capability::OutputFx));
    assert!(!output_plan.requires.contains(&Capability::FxShotScope));

    // A `shot` pass asks for both; the plan says which shot it belongs to.
    let shot = edited(&base, &[vignette("v1", "shot:shot-1", 0, 48)]);
    let (shot_plan, _) = draw(&mut renderer, &fixture, &shot, 12);
    assert_eq!(shot_plan.fx[0].scope, FxScope::Shot("shot-1".into()));
    assert!(shot_plan.requires.contains(&Capability::OutputFx));
    assert!(shot_plan.requires.contains(&Capability::FxShotScope));
    let without: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|capability| *capability != Capability::FxShotScope)
        .collect();
    assert_eq!(
        shot_plan.missing_capabilities(&without),
        [Capability::FxShotScope]
    );
    assert!(output_plan.missing_capabilities(&without).is_empty());
    renderer.shutdown();
}

// ---- a shot clip draws in its shot only -----------------------------------------------------------

/// Over `fixture`, for each shot in turn: a vignette clip as long as the shot, once with the `shot`
/// scope and once with `output`; the frames of the shot against the reference, the frames of the
/// other shot against the baseline, and the two scopes against each other.
fn prove_shot_clip(fixture: &Fixture) {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let base = fixture.scene();
    let frames = frames_of(&fixture.shots);
    let baselines: Vec<(ScenePlan, FrameResult)> = frames
        .iter()
        .map(|frame| draw(&mut renderer, fixture, &base, *frame))
        .collect();
    assert!(baselines.iter().all(|(plan, _)| plan.fx.is_empty()));

    let (mut worst, mut strongest) = (0, 0);
    for target in 0..2 {
        let (id, start, end) = fixture.shot(target);
        let shot_scene = edited(&base, &[vignette("v1", &format!("shot:{id}"), start, end)]);
        // The same clip over the same frames, but on the output: only as long as the shot, so the
        // frames of the other shot have no pass either way.
        let output_scene = edited(&base, &[vignette("v1", "output", start, end)]);
        for (frame, (baseline_plan, baseline)) in frames.iter().zip(&baselines) {
            let inside = (start..end).contains(frame);
            let (plan, shaded) = draw(&mut renderer, fixture, &shot_scene, *frame);
            let (output_plan, via_output) = draw(&mut renderer, fixture, &output_scene, *frame);
            let what = format!("{id} clip at edit frame {frame}");
            if inside {
                assert_eq!(plan.fx.len(), 1, "{what}");
                assert_eq!(plan.fx[0].scope, FxScope::Shot(id.into()), "{what}");
                assert_eq!(plan.frame.shot_id, id, "{what}: the frame is in the shot");
                assert!(plan.requires.contains(&Capability::FxShotScope), "{what}");
                let (w, s) = assert_matches(
                    &plan,
                    baseline,
                    &shaded,
                    &in_order(&plan, &["vignette"]),
                    TOLERANCE,
                    &what,
                );
                (worst, strongest) = (worst.max(w), strongest.max(s));
                assert_ne!(shaded.hash, baseline.hash, "{what}: the picture changed");
                assert!(
                    brightest_gain(baseline, &shaded) <= TOLERANCE,
                    "{what}: a vignette never brightens"
                );
                // `shot` is `output` when nothing else is drawn: the very same picture.
                assert_eq!(output_plan.fx.len(), 1, "{what}");
                assert_eq!(output_plan.fx[0].scope, FxScope::Output, "{what}");
                assert_eq!(via_output.hash, shaded.hash, "{what}: shot == output");
                assert_eq!(via_output.probes, shaded.probes, "{what}: shot == output");
            } else {
                // The other shot: no pass at all, the baseline's picture byte for byte.
                assert!(plan.fx.is_empty(), "{what}: {:?}", plan.fx);
                assert!(!plan.requires.contains(&Capability::FxShotScope), "{what}");
                assert_eq!(plan.frame.shot_id, baseline_plan.frame.shot_id, "{what}");
                assert_eq!(shaded.hash, baseline.hash, "{what}: outside the shot");
                assert_eq!(shaded.probes, baseline.probes, "{what}: outside the shot");
                assert_eq!(via_output.hash, baseline.hash, "{what}: outside the clip");
            }
        }
    }
    assert!(
        strongest >= 20,
        "the comparison must discriminate: the vignette changed a probe by {strongest} at most"
    );
    eprintln!(
        "EVIDENCE shot scope (vignette): worst delta to the reference {worst}, strongest change from the baseline {strongest}"
    );
    renderer.shutdown();
}

#[test]
fn a_shot_clip_darkens_only_its_shot_on_the_minimal_corpus() {
    prove_shot_clip(&Fixture::minimal("darken-minimal"));
}

#[test]
fn a_shot_clip_darkens_only_its_shot_on_the_cut_corpus() {
    prove_shot_clip(&Fixture::cut("darken-cut"));
}

// ---- the stages do not commute ----------------------------------------------------------------------

/// A light fade over `shot:<id>` on a LATER track than an `output` vignette, against the same two
/// effects both on `output` in the same track order. The staged scene is drawn fade first (deck,
/// then output); the all-output scene is drawn in track order, vignette first.
fn prove_stage_order(fixture: &Fixture, target: usize, frame: i64) {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let base = fixture.scene();
    let (id, start, end) = fixture.shot(target);
    let (_, baseline) = draw(&mut renderer, fixture, &base, frame);

    // The vignette is added first, so it is on the first track; the overlapping fade goes to a
    // second one. In the staged scene the second track draws first.
    let shot_fade = format!("shot:{id}");
    let staged = edited(
        &base,
        &[
            vignette("v1", "output", start, end),
            white_fade("f1", &shot_fade, start, end, 0.7),
        ],
    );
    let all_output = edited(
        &base,
        &[
            vignette("v1", "output", start, end),
            white_fade("f1", "output", start, end, 0.7),
        ],
    );
    let (staged_plan, staged_read) = draw(&mut renderer, fixture, &staged, frame);
    let (flat_plan, flat_read) = draw(&mut renderer, fixture, &all_output, frame);
    // What the plan says: the shot's fade first although its track is the later one.
    assert_eq!(listed(&staged_plan), ["fade", "vignette"]);
    assert_eq!(listed(&flat_plan), ["vignette", "fade"]);
    assert!(staged_plan.requires.contains(&Capability::FxShotScope));
    assert!(!flat_plan.requires.contains(&Capability::FxShotScope));

    // The expectations, written here. Staged: the deck stage first (the shot's fade), then the
    // output stage (the vignette). All-output: the tracks in order (vignette, then fade).
    let staged_stages = in_order(&staged_plan, &["fade", "vignette"]);
    let flat_stages = in_order(&flat_plan, &["vignette", "fade"]);
    let size = (
        staged_plan.output.width as usize,
        staged_plan.output.height as usize,
    );
    let points = grid(size.0 as u32, size.1 as u32);

    // The comparison discriminates: the two orders disagree on some probe by far more than the
    // rounding, so a runtime that ignored the stages could not pass both checks.
    let apart = points
        .iter()
        .zip(&baseline.probes)
        .map(|(point, base)| {
            let a = expected(&staged_stages, *base, *point, size);
            let b = expected(&flat_stages, *base, *point, size);
            (0..3).map(|c| (a[c] - b[c]).abs()).max().unwrap()
        })
        .max()
        .unwrap();
    assert!(
        apart >= 4 * TOLERANCE,
        "the two orders must be told apart by the reference: {apart}"
    );

    let (staged_worst, _) = assert_matches(
        &staged_plan,
        &baseline,
        &staged_read,
        &staged_stages,
        TOLERANCE * 2,
        "staged: shot fade, then output vignette",
    );
    let (flat_worst, _) = assert_matches(
        &flat_plan,
        &baseline,
        &flat_read,
        &flat_stages,
        TOLERANCE * 2,
        "all output: vignette, then fade",
    );
    assert_ne!(
        staged_read.hash, flat_read.hash,
        "the order changes the pixels"
    );
    assert_ne!(staged_read.probes, flat_read.probes);

    // The staged picture is a picture of the deck-then-output order and not of the track order.
    let wrong_order: Vec<[i32; 3]> = points
        .iter()
        .zip(&baseline.probes)
        .map(|(point, base)| expected(&flat_stages, *base, *point, size))
        .collect();
    assert!(
        staged_read
            .probes
            .iter()
            .zip(&wrong_order)
            .any(|(got, want)| (0..3).any(|c| (i32::from(got[c]) - want[c]).abs() > 2 * TOLERANCE)),
        "the staged picture must not match the track order"
    );

    // Seek and replay: the same frame, drawn after jumping away, is the same picture.
    let other = if frame + 1 < end {
        frame + 1
    } else {
        frame - 1
    };
    let _ = draw(&mut renderer, fixture, &staged, other);
    let (_, again) = draw(&mut renderer, fixture, &staged, frame);
    assert_eq!(
        again.hash, staged_read.hash,
        "a seek draws what playback drew"
    );
    eprintln!(
        "EVIDENCE stage order ({id}, frame {frame}): worst delta to the reference {staged_worst} / {flat_worst}, the two orders differ by {apart}"
    );
    renderer.shutdown();
}

#[test]
fn the_stages_do_not_commute_and_the_picture_follows_them_on_the_minimal_corpus() {
    prove_stage_order(&Fixture::minimal("order-minimal"), 0, 12);
}

#[test]
fn the_stages_do_not_commute_and_the_picture_follows_them_on_the_cut_corpus() {
    prove_stage_order(&Fixture::cut("order-cut"), 1, 60);
}

// ---- a runtime without the capability ---------------------------------------------------------------

type Sent = Rc<RefCell<Vec<Value>>>;

/// A runtime that answers every message with `respond(message) -> replies`, in order.
struct Scripted {
    sent: Sent,
    queue: VecDeque<Value>,
    respond: fn(&Value) -> Vec<Value>,
}

impl Transport for Scripted {
    fn send(&mut self, message: &Value) -> Result<(), BackendError> {
        self.sent.borrow_mut().push(message.clone());
        self.queue.extend((self.respond)(message));
        Ok(())
    }

    fn recv(&mut self, _: Duration) -> Option<Value> {
        self.queue.pop_front()
    }

    fn close(&mut self) {}
}

/// A runtime that lists the feature and every capability but `fxShotScope`.
fn without_the_shot_scope(message: &Value) -> Vec<Value> {
    match message["type"].as_str() {
        Some("hello") => {
            let capabilities: Vec<&str> = Capability::ALL
                .iter()
                .filter(|capability| **capability != Capability::FxShotScope)
                .map(|capability| capability.as_str())
                .collect();
            vec![json!({
                "v": 1, "type": "ready",
                "runtime": {"name": "scripted", "version": "0", "renderer": "none"},
                "capabilities": {
                    "layers": ["directed_scene"], "formats": {}, "features": ["directed_scene"],
                    "directed_scene": {"layer_kind": "directed_scene", "capabilities": capabilities},
                },
            })]
        }
        Some("asset.load") => vec![json!({
            "v": 1, "type": "asset.state", "id": message["asset"]["id"], "state": "ready", "meta": {},
        })],
        Some("frame.render") => vec![json!({
            "v": 1, "type": "frame.done", "id": message["id"], "revision": message["id"],
            "width": 320, "height": 180, "hash": "abcd", "mean": [1, 2, 3, 255], "probes": [],
            "png": null, "timings": {"render_ms": 1.0, "total_ms": 2.0},
        })],
        _ => vec![],
    }
}

fn scripted() -> (SceneRenderer<Scripted>, Sent) {
    let sent: Sent = Rc::default();
    let transport = Scripted {
        sent: Rc::clone(&sent),
        queue: VecDeque::new(),
        respond: without_the_shot_scope,
    };
    let backend = BridgeBackend::new("scripted", transport, Duration::from_secs(1));
    (
        SceneRenderer::new(backend).with_timeout(Duration::from_secs(1)),
        sent,
    )
}

fn types(sent: &Sent) -> Vec<String> {
    sent.borrow()
        .iter()
        .map(|message| message["type"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_runtime_without_the_shot_scope_is_refused_by_name_before_anything_is_sent() {
    let fixture = Fixture::minimal("scripted");
    let base = fixture.scene();
    let plan_of = |scene: &DirectedScene| {
        let plan = ScenePlan::build(scene, 12).unwrap();
        let resources = SceneResourceMap::from_root(1, &plan, &fixture.dir).unwrap();
        (plan, resources)
    };

    // A shot pass: refused by the name of the missing capability, with nothing else sent.
    let shot = edited(&base, &[vignette("v1", "shot:shot-1", 0, 48)]);
    let (plan, resources) = plan_of(&shot);
    assert!(plan.requires.contains(&Capability::FxShotScope));
    let (mut renderer, sent) = scripted();
    let error = renderer
        .render(&plan, &resources, &FrameOptions::default())
        .unwrap_err();
    assert_eq!(error.code, "unsupported");
    let details = error.details.unwrap();
    assert_eq!(details["unsupported"], "scene_plan_capabilities");
    assert!(details.to_string().contains("fxShotScope"), "{details}");
    assert_eq!(types(&sent), ["hello"], "nothing else was sent to it");

    // The same runtime serves a plan that has no shot pass: an `output` pass, and no pass at all.
    for scene in [
        edited(&base, &[vignette("v1", "output", 0, 48)]),
        base.clone(),
    ] {
        let (plan, resources) = plan_of(&scene);
        assert!(!plan.requires.contains(&Capability::FxShotScope));
        let (mut renderer, sent) = scripted();
        renderer
            .render(&plan, &resources, &FrameOptions::default())
            .expect("a plan without a shot pass is not refused for the shot scope");
        assert_eq!(
            types(&sent).last().map(String::as_str),
            Some("frame.render")
        );
    }
}

// ---- the export -------------------------------------------------------------------------------------

const WIDTH: i64 = 1920;
const HEIGHT: i64 = 1080;

struct Session {
    state: EditorState,
    host: SceneHost,
    fixture: Fixture,
}

impl Session {
    fn new(fixture: Fixture) -> Option<Session> {
        let config = match RuntimeConfig::discover() {
            Ok(config) => config,
            Err(setup) => return skip(&setup.message()),
        };
        if let Err(error) = Tools::discover() {
            return skip(&error.message);
        }
        let mut session = Session {
            state: EditorState::empty(&ProjectSettings::default()).unwrap(),
            host: SceneHost::new(MediaHost::new(), config),
            fixture,
        };
        let path = session.fixture.bundle();
        let base = session.fixture.dir.clone();
        session.ok(json!({"op":"scene.attach","id":"street","path":path}));
        session.ok(json!({"op":"scene.resources.resolve","scene":"street","base":base}));
        Some(session)
    }

    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        headless::execute_ext(&mut self.state, None, Some(&mut self.host), &request)
    }

    fn ok(&mut self, request: Value) -> Value {
        self.run(request.clone())
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    fn export(&mut self, name: &str, from: i64, frames: i64) -> Vec<String> {
        let destination = self.fixture.dir.join(name);
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
            "width":WIDTH,"height":HEIGHT}))["frame"]["hash"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn edit(&mut self, ops: Value) -> Value {
        self.ok(json!({"op":"scene.fx.edit","scene":"street","ops":ops}))
    }
}

/// Exports `frames` edit frames from `from` with a vignette clip over `target`'s whole shot:
/// the frames of the clip's shot differ from the baseline export, the others are its hashes, every
/// frame is what `scene.render` draws for the same edit frame, and flipping the clip to `output`
/// and back changes nothing.
fn prove_export(fixture: Fixture, target: usize, from: i64, frames: i64) {
    let (id, start, end) = fixture.shot(target);
    let Some(mut session) = Session::new(fixture) else {
        return;
    };
    let baseline = session.export("base.mp4", from, frames);

    let edited = session.edit(json!([{
        "op": "add_clip", "effect": "vignette", "id": "v1", "scope": format!("shot:{id}"),
        "start_frame": start, "duration_frames": end - start, "params": strong_vignette(),
    }]));
    assert_eq!(edited["changed"], true);
    let shot_hashes = session.export("shot.mp4", from, frames);

    let (mut inside, mut outside) = (0, 0);
    for index in 0..frames as usize {
        let edit_frame = from + index as i64;
        if (start..end).contains(&edit_frame) {
            inside += 1;
            assert_ne!(
                shot_hashes[index], baseline[index],
                "edit frame {edit_frame} is in shot {id}"
            );
        } else {
            outside += 1;
            assert_eq!(
                shot_hashes[index], baseline[index],
                "edit frame {edit_frame} is outside shot {id}"
            );
        }
        assert_eq!(
            shot_hashes[index],
            session.rendered_hash(edit_frame),
            "export frame {index} is not what scene.render draws for edit frame {edit_frame}"
        );
    }
    assert!(
        inside >= 2 && outside >= 2,
        "two frames on each side of the boundary ({inside} inside, {outside} outside)"
    );

    // `set_scope` to `output` over the same frames: the export does not change by a pixel; back to
    // `shot` and the same again (one atomic edit each time).
    let flipped = session.edit(json!([{"op":"set_scope","clip":"v1","scope":"output"}]));
    assert_eq!(flipped["changed"], true);
    assert_eq!(
        session.export("output.mp4", from, frames),
        shot_hashes,
        "shot == output in the export"
    );
    session.edit(json!([{"op":"set_scope","clip":"v1","scope":format!("shot:{id}")}]));
    assert_eq!(session.export("shot2.mp4", from, frames), shot_hashes);

    // One undo per edit: back to the `output` clip, then to the `shot` clip, then no clip.
    session.ok(json!({"op":"history.undo"}));
    session.ok(json!({"op":"history.undo"}));
    session.ok(json!({"op":"history.undo"}));
    assert_eq!(
        session.export("base2.mp4", from, frames),
        baseline,
        "undo restores the baseline pictures"
    );
    eprintln!(
        "EVIDENCE shot scope export ({id}): {inside} frames inside, {outside} outside, every hash equal to scene.render"
    );
}

#[test]
fn an_export_on_the_minimal_corpus_is_what_scene_render_draws_on_both_sides_of_the_cut_of_shots() {
    // The clip covers `shot-1` ([0, 48)); frames 46..50 are 46, 47 inside and 48, 49 outside.
    prove_export(Fixture::minimal("export-minimal"), 0, 46, 4);
}

#[test]
fn an_export_on_the_cut_corpus_is_what_scene_render_draws_on_both_sides_of_the_cut() {
    // The clip covers `shot-b` ([40, 100)); frames 38..42 are 38, 39 outside and 40, 41 inside.
    prove_export(Fixture::cut("export-cut"), 1, 38, 4);
}
