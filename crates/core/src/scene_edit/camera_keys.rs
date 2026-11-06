//! The origin's write of a camera start: before the start moves, the first movement of the clip
//! becomes keyframes that make the same frames.
//!
//! WHY (the origin's reasoning, `properties/camera-properties.ts`): a recipe movement is
//! relative to the pose it starts on, so moving the start would carry the whole path along.
//! Turning it into keys with absolute poses makes only the beginning of the path move; the
//! movements after it start from where the converted one ends, which has not changed. This
//! module is `keysFromRecipe` / `toKeyframes` of `camera-commands.ts` for movement 0:
//!
//! * a hold becomes one linear key on its last frame;
//! * a dolly, truck or zoom becomes one key on its last frame at the end pose, keeping the
//!   movement's easing (the straight line between the ends is the same line, eased the same);
//! * an orbit follows an arc, so it becomes a linear key every two frames on the arc plus the
//!   end pose on the last frame (the straight spans between keys are chords, `ORBIT_KEY_STEP`);
//! * keys already: nothing changes.
//!
//! The pose arithmetic is the temporal evaluator's (`movement_pose_at`, `movement_end_pose`), not a
//! second copy, so the converted movement plays the frames the recipe played (to the chord error of
//! an orbit). The oracle in `docs/migration/fixtures/scene-edit` compares every frame with the
//! origin's own result.

use serde_json::{Value, json};

use crate::directed_scene::{
    CameraClip, CameraMotion, CameraPose, CameraSegment, SegmentStart, movement_end_pose,
    movement_pose_at,
};

use super::error::{CAMERA_CONVERSION_FAILED, DOCUMENT_MISMATCH, EditError};
use super::value::{PropertyValue, ValueKind};

/// How many frames apart the keys of a converted orbit are (the origin's
/// `ORBIT_KEY_STEP_FRAMES`). The straight spans between keys are chords of the arc; measured on the
/// oracle's orbit (40 degrees eased over 20 frames) the largest gap is 1.06% of the radius
/// (`scene_edit_oracle.rs`), and it grows with how fast the orbit turns.
const ORBIT_KEY_STEP: u64 = 2;

/// Every field the schema declares on a movement, whatever its recipe: the ones a conversion
/// rebuilds (`recipe`, `durationFrames`, `from`, `keys`) and the recipe-specific ones it retires
/// (`easing`, `distanceSceneUnits`, `fovDeltaDeg`, `yawDeg`, `pitchDeg`).
const SCHEMA_KEYS: [&str; 9] = [
    "recipe",
    "durationFrames",
    "from",
    "keys",
    "easing",
    "distanceSceneUnits",
    "fovDeltaDeg",
    "yawDeg",
    "pitchDeg",
];

/// A movement that was turned into keys by a write of the batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovementConversion {
    /// Position in the batch of the op that caused it.
    pub index: usize,
    pub clip: String,
    /// Always 0 today: only the first movement is converted.
    pub segment: usize,
    /// The recipe the movement had (`dollyIn`, `orbit`, ...).
    pub recipe: String,
    /// How many keys it has now.
    pub keys: usize,
}

fn mismatch(what: impl Into<String>) -> EditError {
    EditError::new(DOCUMENT_MISMATCH, what)
}

fn read_pose(work: &Value, clip: &str) -> Result<CameraPose, EditError> {
    let vec3 = |field: &str| {
        let value = work.pointer(&format!("{clip}/{field}"));
        match value.and_then(|value| PropertyValue::read_json(ValueKind::Vec3, value)) {
            Some(PropertyValue::Vec3(v)) => Ok(v),
            _ => Err(mismatch(format!(
                "`{clip}/{field}` holds no readable vector"
            ))),
        }
    };
    let fov = work
        .pointer(&format!("{clip}/initialFovDeg"))
        .and_then(Value::as_f64)
        .ok_or_else(|| mismatch(format!("`{clip}/initialFovDeg` holds no number")))?;
    Ok(CameraPose {
        position: vec3("initialPosition")?,
        target: vec3("initialTarget")?,
        fov_deg: fov,
    })
}

fn key_json(frame: u64, pose: &CameraPose, easing: &str) -> Value {
    json!({
        "frame": frame,
        "position": PropertyValue::Vec3(pose.position).to_json(),
        "target": PropertyValue::Vec3(pose.target).to_json(),
        "fovDeg": PropertyValue::Number(pose.fov_deg).to_json(),
        "easing": easing,
    })
}

fn failed(error: impl std::fmt::Debug) -> EditError {
    EditError::new(
        CAMERA_CONVERSION_FAILED,
        format!("cannot turn the first movement into keyframes: {error:?}"),
    )
}

/// The keys that reproduce `segment` when it starts on `start`.
fn keys_from(start: &CameraPose, segment: &CameraSegment) -> Result<Vec<Value>, EditError> {
    let last = segment.duration_frames.saturating_sub(1);
    let end = movement_end_pose(start, segment).map_err(failed)?;
    match &segment.motion {
        CameraMotion::Keyframes { .. } => Err(mismatch("the movement is already keyframes")),
        CameraMotion::Hold => Ok(vec![key_json(last, &end, "linear")]),
        CameraMotion::Move { easing, .. } | CameraMotion::Zoom { easing, .. } => {
            Ok(vec![key_json(last, &end, easing)])
        }
        CameraMotion::Orbit { .. } => {
            let mut keys = Vec::new();
            let mut frame = ORBIT_KEY_STEP;
            while frame < last {
                let pose = movement_pose_at(start, segment, frame).map_err(failed)?;
                keys.push(key_json(frame, &pose, "linear"));
                frame += ORBIT_KEY_STEP;
            }
            keys.push(key_json(last, &end, "linear"));
            Ok(keys)
        }
    }
}

fn recipe_name(motion: &CameraMotion) -> &'static str {
    use crate::directed_scene::{MoveRecipe, ZoomRecipe};
    match motion {
        CameraMotion::Hold => "hold",
        CameraMotion::Move { recipe, .. } => match recipe {
            MoveRecipe::DollyIn => "dollyIn",
            MoveRecipe::DollyOut => "dollyOut",
            MoveRecipe::TruckLeft => "truckLeft",
            MoveRecipe::TruckRight => "truckRight",
        },
        CameraMotion::Zoom { recipe, .. } => match recipe {
            ZoomRecipe::In => "zoomIn",
            ZoomRecipe::Out => "zoomOut",
        },
        CameraMotion::Orbit { .. } => "orbit",
        CameraMotion::Keyframes { .. } => "keyframes",
    }
}

/// Turns movement 0 of the clip at `clip` (a JSON Pointer into `work`) into keyframes, from the
/// start pose `work` holds *now* (earlier ops of the batch included). `Ok(None)`: it already is.
///
/// `base` is the clip as the input scene typed it. Movements change only through this function, so
/// the typed first movement is the one in `work` until the first conversion; after it `work` says
/// `keyframes` and the typed one is not consulted.
pub(super) fn convert_first_movement(
    work: &mut Value,
    clip: &str,
    base: &CameraClip,
    index: usize,
) -> Result<Option<MovementConversion>, EditError> {
    let pointer = format!("{clip}/segments/0");
    let recipe = work
        .pointer(&format!("{pointer}/recipe"))
        .and_then(Value::as_str)
        .ok_or_else(|| mismatch(format!("`{pointer}` is not a movement")))?;
    if recipe == "keyframes" {
        return Ok(None);
    }
    let Some(segment) = base.segments.first() else {
        return Err(mismatch(format!("clip `{}` has no movements", base.id)));
    };
    let start = read_pose(work, clip)?;
    let keys = keys_from(&start, segment)?;
    let count = keys.len();
    let from = match segment.from {
        SegmentStart::Initial => "initial",
        SegmentStart::PreviousEnd => "previousEnd",
    };
    let mut converted = json!({
        "recipe": "keyframes",
        "durationFrames": segment.duration_frames,
        "from": from,
        "keys": keys,
    });
    let slot = work
        .pointer_mut(&pointer)
        .ok_or_else(|| mismatch(format!("`{pointer}` vanished")))?;
    // Fields the schema does not declare stay on the movement: the document is the canonical data
    // and the importer keeps and reports unknown fields (F031). The origin drops them, but it
    // never sees them (its zod parse strips them before any command runs).
    if let (Some(old), Some(new)) = (slot.as_object(), converted.as_object_mut()) {
        for (key, value) in old {
            if !SCHEMA_KEYS.contains(&key.as_str()) {
                new.insert(key.clone(), value.clone());
            }
        }
    }
    *slot = converted;
    Ok(Some(MovementConversion {
        index,
        clip: base.id.clone(),
        segment: 0,
        recipe: recipe_name(&segment.motion).to_string(),
        keys: count,
    }))
}
