//! Writing a shot plan into the document and judging the candidate (F065, D01, D02, §5.2, §5.4).
//!
//! The batch works on a clone of the scene's JSON, never on the typed `Bundle` (that view is
//! rebuilt on every import, so it cannot see what the previous op of the batch wrote). Each op
//! reads the shots from the candidate the previous one left, plans, writes. After the last op the
//! candidate goes through the one gate every edit uses: `DirectedScene::reimport` (the importer,
//! under the scene's own transition policy), then the structural and the temporal join rules. A
//! finding is `CANDIDATE_REJECTED`, tied to the op that touched the entity the finding points into
//! by JSON pointer, exactly as `scene_edit::fx` does it.
//!
//! The shared `candidate_gate` of the plan (§4) is not extracted here: the importer paths of the
//! other edits are in another slice's files, so this module calls the same three pieces itself.
//! An op that changes nothing ends the batch without the gate: there is no candidate to judge.

use serde_json::{Map, Value, json};

use crate::directed_scene::fx::{Alignment, Handles, transition_window};
use crate::directed_scene::{DirectedScene, ImportError, Limits};
use crate::scene_edit::batch::rejected;
use crate::scene_edit::error::{DOCUMENT_MISMATCH, EditError};

use super::plan::{Placement, PlanInput, PlanInstance, PlanShot, ShotPlan, plan_op};
use super::{FRAME_BOUND, ShotOp, check_batch_len};

/// The instance grew so the shot fits (#6).
pub const INSTANCE_GREW: &str = "INSTANCE_GREW";
/// `auto` anchored the head frames it had and followed the rest (#28).
pub const AUTO_FELL_TO_FOLLOW: &str = "AUTO_FELL_TO_FOLLOW";

/// What an op did that its author may not have meant. The edit still happened.
#[derive(Debug, Clone, PartialEq)]
pub struct ShotNote {
    pub code: &'static str,
    /// The facts of the note (`instance`, `from`, `to`; `shot`, `anchored`, `followed`).
    pub fields: Map<String, Value>,
}

impl ShotNote {
    fn new(code: &'static str, facts: Value) -> Self {
        let Value::Object(fields) = facts else {
            unreachable!("a note carries an object");
        };
        Self { code, fields }
    }

    /// `{"code": ..., ...facts}`.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("code".into(), json!(self.code));
        out.extend(self.fields.clone());
        Value::Object(out)
    }
}

/// A batch that was accepted: the candidate scene (the input one when nothing changed), the notes
/// of every op in order, and whether the document differs from what went in.
#[derive(Debug, Clone, PartialEq)]
pub struct ShotsOutcome {
    pub candidate: DirectedScene,
    pub notes: Vec<ShotNote>,
    pub changed: bool,
}

/// [`apply_shots_with`] under the importer's default `Limits`.
pub fn apply_shots(scene: &DirectedScene, ops: &[ShotOp]) -> Result<ShotsOutcome, EditError> {
    apply_shots_with(scene, ops, &Limits::default())
}

/// Runs the ops in order on a copy of the document and judges the result once. The first op that
/// refuses ends the batch with its position (`index`); the input scene never changes.
pub fn apply_shots_with(
    scene: &DirectedScene,
    ops: &[ShotOp],
    limits: &Limits,
) -> Result<ShotsOutcome, EditError> {
    check_batch_len(ops.len())?;
    let mut work = scene.document().clone();
    let mut notes = Vec::new();
    let mut pointers: Vec<(usize, String)> = Vec::new();
    for (index, op) in ops.iter().enumerate() {
        let input = read_input(&work).map_err(|error| error.at_op(index))?;
        let planned =
            plan_op(&input, op).map_err(|refusal| refusal.into_edit_error().at_op(index))?;
        let Some(plan) = planned else {
            continue;
        };
        let written =
            write_plan(&mut work, &plan, &mut notes).map_err(|error| error.at_op(index))?;
        pointers.extend(written.into_iter().map(|pointer| (index, pointer)));
    }
    if work == *scene.document() {
        return Ok(ShotsOutcome {
            candidate: scene.clone(),
            notes,
            changed: false,
        });
    }
    let candidate = scene
        .reimport(work, limits)
        .map_err(|error| rejected(error, &pointers))?;
    let mut issues = candidate.edit_join_issues();
    issues.extend(candidate.temporal_issues());
    if !issues.is_empty() {
        return Err(rejected(ImportError::Invalid { issues }, &pointers));
    }
    Ok(ShotsOutcome {
        candidate,
        notes,
        changed: true,
    })
}

fn mismatch(what: impl Into<String>) -> EditError {
    EditError::new(DOCUMENT_MISMATCH, what)
}

fn int(value: &Value, what: &str) -> Result<i64, EditError> {
    value
        .as_i64()
        .filter(|frame| frame.abs() <= FRAME_BOUND)
        .ok_or_else(|| mismatch(format!("`{what}` is not an integer frame")))
}

fn text<'a>(value: &'a Value, key: &str, what: &str) -> Result<&'a str, EditError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| mismatch(format!("`{what}/{key}` is not a string")))
}

fn number(value: &Value, key: &str, what: &str) -> Result<i64, EditError> {
    let found = value
        .get(key)
        .ok_or_else(|| mismatch(format!("`{what}/{key}` is missing")))?;
    int(found, &format!("{what}/{key}"))
}

fn list<'a>(work: &'a Value, key: &str) -> Result<&'a Vec<Value>, EditError> {
    work.get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| mismatch(format!("`/{key}` is not an array")))
}

/// The planner's view of the document as it is now.
fn read_input(work: &Value) -> Result<PlanInput, EditError> {
    let edit_frames = number(work, "editDurationFrames", "")?;
    let instances = list(work, "sceneInstances")?
        .iter()
        .enumerate()
        .map(|(index, instance)| {
            let what = format!("/sceneInstances/{index}");
            Ok(PlanInstance {
                id: text(instance, "id", &what)?.to_string(),
                frames: number(instance, "durationFrames", &what)?,
                locked: instance.get("editLocked") == Some(&Value::Bool(true)),
            })
        })
        .collect::<Result<Vec<_>, EditError>>()?;
    let shots = list(work, "shots")?
        .iter()
        .enumerate()
        .map(|(index, shot)| {
            let what = format!("/shots/{index}");
            let range = shot
                .get("editRange")
                .ok_or_else(|| mismatch(format!("`{what}/editRange` is missing")))?;
            let id = text(shot, "id", &what)?.to_string();
            let reserve = reserve_of(work, &id);
            Ok(PlanShot {
                instance: text(shot, "sceneInstanceId", &what)?.to_string(),
                at: Placement {
                    start: number(range, "startFrame", &format!("{what}/editRange"))?,
                    end: number(range, "endFrameExclusive", &format!("{what}/editRange"))?,
                    instance_start: number(shot, "instanceStartFrame", &what)?,
                    camera_in: number(shot, "cameraInFrame", &what)?,
                },
                continuous_in: shot
                    .get("transitionInto")
                    .and_then(|join| join.get("type"))
                    .and_then(Value::as_str)
                    == Some("continuous"),
                reserve,
                id,
            })
        })
        .collect::<Result<Vec<_>, EditError>>()?;
    Ok(PlanInput::new(edit_frames, shots, instances))
}

/// The head frames the transition **into** `shot_id` draws before the cut (#29): none without a
/// transition or when its `handles` is `hold`. A transition document only exists when the scene
/// was admitted under `TransitionAdmission::Accept` (public admission refuses `fx.transitions`
/// today), so in production this is 0.
fn reserve_of(work: &Value, shot_id: &str) -> i64 {
    let Some(transition) = work
        .pointer("/fx/transitions")
        .and_then(Value::as_array)
        .and_then(|list| {
            list.iter()
                .find(|item| item.get("toShotId").and_then(Value::as_str) == Some(shot_id))
        })
    else {
        return 0;
    };
    let handles = transition
        .get("handles")
        .and_then(Value::as_str)
        .map_or(Some(Handles::Hold), Handles::parse);
    if handles.is_none_or(|handles| !handles.needs_frames()) {
        return 0;
    }
    let alignment = transition
        .get("alignment")
        .and_then(Value::as_str)
        .and_then(Alignment::parse);
    let duration = transition.get("durationFrames").and_then(Value::as_i64);
    // `before` does not depend on where the cut is: put it one duration in, so no window is
    // refused for starting before frame 0.
    match (alignment, duration) {
        (Some(alignment), Some(duration)) => {
            transition_window(duration, duration, alignment, i64::MAX)
                .map_or(0, |window| window.before)
        }
        _ => 0,
    }
}

/// Sets `key` of `object` to `value` when it differs. `true` when it wrote.
fn set_int(object: &mut Value, key: &str, value: i64) -> bool {
    if object.get(key).and_then(Value::as_i64) == Some(value) {
        return false;
    }
    object[key] = json!(value);
    true
}

/// Writes the plan into the candidate and appends its notes. Returns the JSON pointers of what it
/// changed, for the findings of the gate to be tied to the op.
fn write_plan(
    work: &mut Value,
    plan: &ShotPlan,
    notes: &mut Vec<ShotNote>,
) -> Result<Vec<String>, EditError> {
    let mut pointers = Vec::new();
    if let Some(follow) = &plan.follow {
        notes.push(ShotNote::new(
            AUTO_FELL_TO_FOLLOW,
            json!({
                "shot": follow.shot, "anchored": follow.anchored, "followed": follow.followed,
            }),
        ));
    }
    let shots = work
        .get_mut("shots")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| mismatch("`/shots` is not an array"))?;
    for (index, shot) in shots.iter_mut().enumerate() {
        let id = shot.get("id").and_then(Value::as_str).unwrap_or_default();
        let Some(at) = plan.placement(id).copied() else {
            continue;
        };
        let mut changed = set_int(shot, "instanceStartFrame", at.instance_start);
        changed |= set_int(shot, "cameraInFrame", at.camera_in);
        let range = shot
            .get_mut("editRange")
            .ok_or_else(|| mismatch(format!("`/shots/{index}/editRange` is missing")))?;
        changed |= set_int(range, "startFrame", at.start);
        changed |= set_int(range, "endFrameExclusive", at.end);
        if changed {
            pointers.push(format!("/shots/{index}"));
        }
    }
    if set_int(work, "editDurationFrames", plan.edit_frames) {
        pointers.push("/editDurationFrames".to_string());
    }
    let instances = work
        .get_mut("sceneInstances")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| mismatch("`/sceneInstances` is not an array"))?;
    for growth in &plan.growth {
        let Some((index, instance)) = instances.iter_mut().enumerate().find(|(_, instance)| {
            instance.get("id").and_then(Value::as_str) == Some(&growth.instance)
        }) else {
            return Err(mismatch(format!("no instance `{}`", growth.instance)));
        };
        set_int(instance, "durationFrames", growth.to);
        pointers.push(format!("/sceneInstances/{index}"));
        notes.push(ShotNote::new(
            INSTANCE_GREW,
            json!({"instance": growth.instance, "from": growth.from, "to": growth.to}),
        ));
    }
    Ok(pointers)
}
