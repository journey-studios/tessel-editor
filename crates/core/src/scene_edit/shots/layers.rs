//! The layers in edit frames that S1 does not move, and the refusal that says so (F065, D09-D12).
//!
//! The planner makes a chain of splices and a `content_shift` per shot; S2 will apply them to the
//! effect clips, the light keys and `weatherKeys`. Until then an op that would change any of them
//! must not be accepted with the layer left where it was (the document would pass the gate and be
//! wrong): [`first_unmoved`] looks for the first item the plan would move, and the caller refuses
//! the op with `SHOTS_LAYER_NOT_YET`.
//!
//! The mapping below is the reference's (`scripts/shot-trim-reference.py`: `map_point`,
//! `map_clip`, `reframe_shot_clip`, `_judge_transitions`), written again from its text, because the
//! point is to ask whether the reference would have changed the item. The result is judged
//! against the item as it is: an item the mapping leaves exactly where it is does not refuse.
//!
//! Scan order (the first item found is the one reported): transitions, effect clips by track,
//! light keys by definition, `weatherKeys` by definition.

use std::collections::HashMap;

use serde_json::Value;

use crate::directed_scene::fx::{Alignment, transition_window};

use super::error::{Refusal, SHOTS_LAYER_NOT_YET};
use super::plan::{Placement, ShotPlan, Splice};
use super::{ShotOptions, lenient_int};

pub(super) const FX_CLIP_OUTPUT: &str = "fx_clip_output";
pub(super) const FX_CLIP_SHOT: &str = "fx_clip_shot";
pub(super) const TRANSITION: &str = "transition";
pub(super) const LIGHT_KEYS: &str = "light_keys";
pub(super) const WEATHER_KEYS: &str = "weather_keys";

/// The first layer item the plan would move: its JSON pointer and its layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Unmoved {
    pub layer: &'static str,
    pub path: String,
}

/// A point of the edit through the chain of splices (#31); `None` when it falls in a removed stretch.
fn map_point(frame: i64, chain: &[Splice]) -> Option<i64> {
    let mut frame = frame;
    for splice in chain {
        match *splice {
            Splice::Insert { at, frames } => {
                if frame >= at {
                    frame += frames;
                }
            }
            Splice::Remove { from, to } => {
                if (from..to).contains(&frame) {
                    return None;
                }
                if frame >= to {
                    frame -= to - from;
                }
            }
        }
    }
    Some(frame)
}

/// A clip `[start, end)` through the chain: `(start, end, head)` where `head` is how many of its
/// own head frames were cut, or `None` when it ends up inside a removed stretch.
fn map_clip(start: i64, end: i64, chain: &[Splice]) -> Option<(i64, i64, i64)> {
    let (mut start, mut end, mut head) = (start, end, 0);
    for splice in chain {
        match *splice {
            Splice::Insert { at, frames } => {
                if start >= at {
                    start += frames;
                    end += frames;
                } else if end > at {
                    end += frames;
                }
            }
            Splice::Remove { from, to } => {
                let removed = to - from;
                if end <= from {
                    continue;
                }
                if start >= to {
                    start -= removed;
                    end -= removed;
                    continue;
                }
                if start >= from && end <= to {
                    return None;
                }
                if start < from && end <= to {
                    end = from;
                    continue;
                }
                if start < from && end > to {
                    end -= removed;
                    continue;
                }
                // `from <= start < to < end`: the head is cut up to `to`.
                head += to - start;
                start = from;
                end -= removed;
            }
        }
    }
    Some((start, end, head))
}

/// A clip of a shot moves with the picture by `delta` (the change of `startFrame -
/// instanceStartFrame` of its shot), then is cut to the shot's new range.
fn reframe_shot_clip(
    start: i64,
    end: i64,
    delta: i64,
    shot: &Placement,
) -> Option<(i64, i64, i64)> {
    let (moved_start, moved_end) = (start + delta, end + delta);
    let (new_start, new_end) = (moved_start.max(shot.start), moved_end.min(shot.end));
    (new_end > new_start).then_some((new_start, new_end, new_start - moved_start))
}

/// Whether the plan moves a transition tied to a touched shot: the transition no longer fits
/// between its two shots and the op asked to shrink it (`shrink_transitions`), which is S2's.
/// A transition that fits stays as the reference leaves it; one that does not fit and was not to
/// be shrunk is the gate's own finding.
fn transition_would_shrink(work: &Value, plan: &ShotPlan) -> Option<Unmoved> {
    let transitions = work.pointer("/fx/transitions")?.as_array()?;
    let placements = plan.placements_by_id();
    for (index, transition) in transitions.iter().enumerate() {
        let (Some(from), Some(to)) = (
            transition.get("fromShotId").and_then(Value::as_str),
            transition.get("toShotId").and_then(Value::as_str),
        ) else {
            continue;
        };
        if !plan.touched.iter().any(|id| id == from || id == to) {
            continue;
        }
        let (Some(source), Some(target)) = (placements.get(from), placements.get(to)) else {
            continue;
        };
        let (Some(duration), Some(alignment)) = (
            transition.get("durationFrames").and_then(Value::as_i64),
            transition
                .get("alignment")
                .and_then(Value::as_str)
                .and_then(Alignment::parse),
        ) else {
            continue;
        };
        // `before` does not depend on where the cut is: any cut far from frame 0 will do.
        let Ok(window) = transition_window(duration, duration, alignment, i64::MAX) else {
            continue;
        };
        let after = duration - window.before;
        if source.frames() < window.before || target.frames() < after {
            return Some(Unmoved {
                layer: TRANSITION,
                path: format!("/fx/transitions/{index}"),
            });
        }
    }
    None
}

fn clips(work: &Value, plan: &ShotPlan, options: &ShotOptions) -> Option<Unmoved> {
    let chain = &plan.splices;
    let placements = plan.placements_by_id();
    let shifts: HashMap<&str, i64> = plan
        .content_shift
        .iter()
        .map(|(id, shift)| (id.as_str(), *shift))
        .collect();
    let tracks = work.pointer("/fx/tracks")?.as_array()?;
    for (track_index, track) in tracks.iter().enumerate() {
        let Some(list) = track.get("clips").and_then(Value::as_array) else {
            continue;
        };
        for (clip_index, clip) in list.iter().enumerate() {
            let (Some(start), Some(duration)) = (
                clip.get("startFrame").and_then(lenient_int),
                clip.get("durationFrames").and_then(lenient_int),
            ) else {
                continue;
            };
            let end = start + duration;
            let path = format!("/fx/tracks/{track_index}/clips/{clip_index}");
            let scope = clip.get("scope");
            let shot_scope = scope
                .filter(|scope| scope.get("kind").and_then(Value::as_str) == Some("shot"))
                .and_then(|scope| scope.get("shotId").and_then(Value::as_str));
            if let Some(id) = shot_scope {
                let Some(shot) = placements.get(id) else {
                    // A clip of a shot the edit does not have: the gate's finding, not a move.
                    continue;
                };
                let delta = shifts.get(id).copied().unwrap_or(0);
                if reframe_shot_clip(start, end, delta, shot) != Some((start, end, 0)) {
                    return Some(Unmoved {
                        layer: FX_CLIP_SHOT,
                        path,
                    });
                }
            } else if !options.keep_absolute && map_clip(start, end, chain) != Some((start, end, 0))
            {
                return Some(Unmoved {
                    layer: FX_CLIP_OUTPUT,
                    path,
                });
            }
        }
    }
    None
}

/// The first key of `keys` (`[{frame, ..}]`) the chain moves or drops, as its index.
fn first_moved_key(keys: &Value, chain: &[Splice]) -> Option<usize> {
    keys.as_array()?.iter().position(|key| {
        key.get("frame")
            .and_then(lenient_int)
            .is_some_and(|frame| map_point(frame, chain) != Some(frame))
    })
}

fn keys(work: &Value, plan: &ShotPlan) -> Option<Unmoved> {
    let definitions = work.get("sceneDefinitions")?.as_array()?;
    let chain = &plan.splices;
    for (index, definition) in definitions.iter().enumerate() {
        let lights = definition
            .pointer("/lighting/lights")
            .and_then(Value::as_array);
        for (light_index, light) in lights.into_iter().flatten().enumerate() {
            let Some(channels) = light.get("keys").and_then(Value::as_object) else {
                continue;
            };
            for (channel, list) in channels {
                if let Some(key) = first_moved_key(list, chain) {
                    return Some(Unmoved {
                        layer: LIGHT_KEYS,
                        path: format!(
                            "/sceneDefinitions/{index}/lighting/lights/{light_index}/keys/{channel}/{key}"
                        ),
                    });
                }
            }
        }
    }
    for (index, definition) in definitions.iter().enumerate() {
        if let Some(key) = definition
            .get("weatherKeys")
            .and_then(|list| first_moved_key(list, chain))
        {
            return Some(Unmoved {
                layer: WEATHER_KEYS,
                path: format!("/sceneDefinitions/{index}/weatherKeys/{key}"),
            });
        }
    }
    None
}

/// The first item of a layer the plan would move that S1 does not (see the module docs).
pub(super) fn first_unmoved(
    work: &Value,
    plan: &ShotPlan,
    options: &ShotOptions,
) -> Option<Unmoved> {
    if options.shrink_transitions
        && let Some(found) = transition_would_shrink(work, plan)
    {
        return Some(found);
    }
    if let Some(found) = clips(work, plan, options) {
        return Some(found);
    }
    if options.keep_absolute {
        return None;
    }
    keys(work, plan)
}

/// The refusal of an op whose plan would move `unmoved`, in the op's name.
pub(super) fn refusal(plan: &ShotPlan, unmoved: &Unmoved) -> Refusal {
    let hint = if matches!(unmoved.layer, FX_CLIP_OUTPUT | LIGHT_KEYS | WEATHER_KEYS) {
        "; `keep_absolute` leaves it where it is"
    } else {
        ""
    };
    let refusal = Refusal::new(
        SHOTS_LAYER_NOT_YET,
        format!(
            "the op would move `{}` ({}), and this phase does not move that layer yet{hint}",
            unmoved.path, unmoved.layer
        ),
    )
    .with("path", unmoved.path.as_str().into())
    .with("layer", unmoved.layer.into());
    refusal.in_op(
        plan.target.shot.as_deref(),
        plan.target.edge,
        plan.target.asked_frame,
    )
}
