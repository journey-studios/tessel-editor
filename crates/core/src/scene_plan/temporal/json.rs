//! The wire form of `alignment` and `timing` (written and read by hand, like the rest of the plan).
//!
//! Every number is an unsigned integer: a float, a bool, a string, a `null`, a negative number, a
//! missing field and an unknown key are refused where they are, with the pointer of the field.
//! No default and no fallback: a plan with a transition that has none of this is the old
//! experimental wire, and is refused at `/transition/alignment`.

use serde_json::{Map, Value, json};

use crate::directed_scene::fx::Alignment;

use super::super::PlanError;
use super::super::json::Obj;
use super::model::{DeckTiming, TransitionTiming};

fn deck_json(deck: &DeckTiming) -> Value {
    Value::Object(
        deck.fields()
            .into_iter()
            .map(|(key, value)| (key.to_string(), json!(value)))
            .collect::<Map<String, Value>>(),
    )
}

pub(in crate::scene_plan) fn timing_json(timing: &TransitionTiming) -> Value {
    json!({
        "editDurationFrames": timing.edit_duration_frames,
        "from": deck_json(&timing.from),
        "to": deck_json(&timing.to),
    })
}

pub(in crate::scene_plan) fn read_alignment(transition: &Obj<'_>) -> Result<Alignment, PlanError> {
    Alignment::parse(transition.text_ref("alignment")?)
        .ok_or_else(|| transition.fail("alignment", "expected `center`, `start` or `end`"))
}

fn read_deck(deck: &Obj<'_>) -> Result<DeckTiming, PlanError> {
    deck.only(&[
        "shotStartFrame",
        "shotEndFrameExclusive",
        "instanceStartFrame",
        "instanceDurationFrames",
        "cameraInFrame",
        "cameraClipDurationFrames",
        "sampledInstanceFrame",
        "sampledCameraClipFrame",
    ])?;
    Ok(DeckTiming {
        shot_start_frame: deck.unsigned("shotStartFrame")?,
        shot_end_frame_exclusive: deck.unsigned("shotEndFrameExclusive")?,
        instance_start_frame: deck.unsigned("instanceStartFrame")?,
        instance_duration_frames: deck.unsigned("instanceDurationFrames")?,
        camera_in_frame: deck.unsigned("cameraInFrame")?,
        camera_clip_duration_frames: deck.unsigned("cameraClipDurationFrames")?,
        sampled_instance_frame: deck.unsigned("sampledInstanceFrame")?,
        sampled_camera_clip_frame: deck.unsigned("sampledCameraClipFrame")?,
    })
}

pub(in crate::scene_plan) fn read_timing(timing: &Obj<'_>) -> Result<TransitionTiming, PlanError> {
    timing.only(&["editDurationFrames", "from", "to"])?;
    Ok(TransitionTiming {
        edit_duration_frames: timing.unsigned("editDurationFrames")?,
        from: read_deck(&timing.object("from")?)?,
        to: read_deck(&timing.object("to")?)?,
    })
}
