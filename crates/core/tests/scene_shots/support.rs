//! The builders of the F065 matrix: real render bundles from the corpus projector and the helpers
//! that read a result back. Every number in the tests is written by hand, never read from here.

use serde_json::{Map, Value, json};
use tessel_editor_core::directed_scene::fx::TransitionAdmission;
use tessel_editor_core::directed_scene::{DirectedScene, Limits};
use tessel_editor_core::scene_edit::shots::{ShotOp, ShotsOutcome, apply_shots, parse_shot_ops};
use tessel_editor_core::scene_edit::{CANDIDATE_REJECTED, EditError};

use crate::shot_trim_support::project;
// ---- builders --------------------------------------------------------------------------------

#[derive(Clone)]
pub struct Sh {
    pub id: &'static str,
    pub instance: &'static str,
    pub start: i64,
    pub end: i64,
    pub instance_start: i64,
    pub camera_in: i64,
    pub continuous: bool,
}

pub fn sh(
    id: &'static str,
    instance: &'static str,
    start: i64,
    end: i64,
    instance_start: i64,
    camera_in: i64,
) -> Sh {
    Sh {
        id,
        instance,
        start,
        end,
        instance_start,
        camera_in,
        continuous: false,
    }
}

impl Sh {
    pub fn carries_on(mut self) -> Self {
        self.continuous = true;
        self
    }
}

/// A render bundle: `edit` frames of `shots`, `instances` as `(id, frames, locked)`, one 400 frame
/// camera clip `cam` that holds its pose, and the oracle-shaped `extra` fields (`fx`, `narration`...).
pub fn bundle(edit: i64, shots: &[Sh], instances: &[(&str, i64, bool)], extra: Value) -> Value {
    let shots: Vec<Value> = shots
        .iter()
        .enumerate()
        .map(|(index, shot)| {
            let mut out = json!({
                "id": shot.id, "intent": format!("shot {}", shot.id),
                "sceneInstanceId": shot.instance, "cameraClipId": "cam",
                "cameraInFrame": shot.camera_in, "instanceStartFrame": shot.instance_start,
                "editRange": {"startFrame": shot.start, "endFrameExclusive": shot.end},
            });
            if index > 0 {
                out["transitionInto"] = if shot.continuous {
                    json!({"type": "continuous"})
                } else {
                    json!({"type": "cut", "reason": "scene change"})
                };
            }
            out
        })
        .collect();
    let instances: Vec<Value> = instances
        .iter()
        .map(|(id, frames, locked)| {
            let mut out = json!({
                "id": id, "sceneDefinitionId": format!("def-{id}"), "durationFrames": frames,
            });
            if *locked {
                out["editLocked"] = json!(true);
            }
            out
        })
        .collect();
    let mut minimal = json!({
        "editDurationFrames": edit, "fps": {"numerator": 24, "denominator": 1},
        "sceneDefinitions": [], "sceneInstances": instances,
        "cameraClips": [{"id": "cam", "durationFrames": 400}], "shots": shots,
    });
    for (key, value) in extra.as_object().unwrap_or(&Map::new()) {
        minimal[key] = value.clone();
    }
    project(&minimal)
}

pub fn scene_of(document: Value) -> DirectedScene {
    DirectedScene::import_value(document).unwrap_or_else(|error| panic!("the fixture: {error:?}"))
}

/// A scene admitted under `TransitionAdmission::Accept`, the door the corpus uses: the public
/// admission refuses `fx.transitions` today.
pub fn scene_admitting(document: Value) -> DirectedScene {
    DirectedScene::import_value_admitting(document, &Limits::default(), TransitionAdmission::Accept)
        .unwrap_or_else(|error| panic!("the fixture: {error:?}"))
}

pub fn build(edit: i64, shots: &[Sh], instances: &[(&str, i64, bool)]) -> DirectedScene {
    scene_of(bundle(edit, shots, instances, json!({})))
}

/// Three cut shots: `a` `[0,30)`, `b` `[30,60)` with `head` frames of scene and camera before it,
/// `c` `[60,90)`; every instance holds 60 frames.
pub fn three(head: i64) -> DirectedScene {
    build(
        90,
        &[
            sh("a", "ia", 0, 30, 0, 0),
            sh("b", "ib", 30, 60, head, head),
            sh("c", "ic", 60, 90, 0, 0),
        ],
        &[("ia", 60, false), ("ib", 60, false), ("ic", 60, false)],
    )
}

/// `a` and `b` are one take on `ia` (a continuous join); `c` is a cut on `ic`.
pub fn chain() -> DirectedScene {
    build(
        90,
        &[
            sh("a", "ia", 0, 30, 0, 0),
            sh("b", "ia", 30, 60, 30, 30).carries_on(),
            sh("c", "ic", 60, 90, 0, 0),
        ],
        &[("ia", 60, false), ("ic", 60, false)],
    )
}

pub fn op(value: Value) -> Vec<ShotOp> {
    parse_shot_ops(&json!([value])).unwrap_or_else(|error| panic!("the op did not parse: {error}"))
}

/// `apply_shots` takes the scene by shared reference, so the type system already says the input
/// never changes; the tests that care compare documents after a refusal where it is not trivial.
pub fn run(scene: &DirectedScene, value: Value) -> Result<ShotsOutcome, EditError> {
    apply_shots(scene, &op(value))
}

#[track_caller]
pub fn done(scene: &DirectedScene, value: Value) -> ShotsOutcome {
    run(scene, value.clone()).unwrap_or_else(|error| panic!("{value} was refused: {error}"))
}

#[track_caller]
pub fn refused(scene: &DirectedScene, value: Value) -> EditError {
    match run(scene, value.clone()) {
        Ok(_) => panic!("{value} was accepted"),
        Err(error) => error,
    }
}

/// `id [start,end) i<instanceStart> c<cameraIn>`, in the order of the document.
pub fn layout(scene: &DirectedScene) -> Vec<String> {
    scene.document()["shots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|shot| {
            format!(
                "{} [{},{}) i{} c{}",
                shot["id"].as_str().unwrap(),
                shot["editRange"]["startFrame"],
                shot["editRange"]["endFrameExclusive"],
                shot["instanceStartFrame"],
                shot["cameraInFrame"],
            )
        })
        .collect()
}

pub fn edit_frames(scene: &DirectedScene) -> i64 {
    scene.document()["editDurationFrames"].as_i64().unwrap()
}

pub fn instance_frames(scene: &DirectedScene, id: &str) -> i64 {
    scene.document()["sceneInstances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|instance| instance["id"] == id)
        .unwrap()["durationFrames"]
        .as_i64()
        .unwrap()
}

pub fn notes(outcome: &ShotsOutcome) -> Vec<Value> {
    outcome.notes.iter().map(|note| note.to_json()).collect()
}

pub fn detail(error: &EditError, key: &str) -> Value {
    error.details().expect("details")[key].clone()
}

#[track_caller]
pub fn finding(error: &EditError, code: &str) {
    assert_eq!(error.code, CANDIDATE_REJECTED, "{error}");
    assert!(
        error.issues().iter().any(|found| found.issue.code == code),
        "no `{code}` among {:?}",
        error.issues()
    );
}
