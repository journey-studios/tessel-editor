//! The rules of `fx.transitions` that need the whole bundle (F064, P4b S1a): the shots named, their
//! adjacency, the window and the frames behind it. They follow the pilot's `transitionIssues` step
//! by step and, like it, **report every finding of every transition** (no step stops the next one,
//! except where the next one would be meaningless: shots that do not exist, a transition that is
//! not on a cut, a duration out of range).
//!
//! Order (plan §5.2), per transition in document order: the issues of effect, family, params and
//! easing that `transition_read` kept ([`TransitionDraft::early`]); the shots exist; they are
//! neighbours; one transition per cut; `1 <= D <= edit`; the window inside the edit; the window
//! inside the two shots; the handles. Then, for the whole list, the windows that overlap. A bundle
//! with any finding is refused whole (`ImportError::Invalid`): there is no partial import.
//!
//! Warnings (`TRANSITION_ON_CONTINUOUS`, `TRANSITION_SPLAT_COST`) never refuse; they describe an
//! admitted scene and are returned by [`transition_warnings`].

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::directed_scene::issue::Issue;
use crate::directed_scene::{Bundle, NodeKind, Shot, Transition};

use super::transition_read::TransitionDraft;
use super::window::check_duration;
use super::{
    FxTransition, Handles, TRANSITION_DUPLICATE, TRANSITION_NO_HANDLES, TRANSITION_NOT_AT_CUT,
    TRANSITION_ON_CONTINUOUS, TRANSITION_OUT_OF_RANGE, TRANSITION_OVERLAP, TRANSITION_SPLAT_COST,
    TRANSITION_TOO_LONG, TransitionWindow, UNKNOWN_SHOT, WindowError, available_frames,
    max_transition_frames, transition_window,
};
use crate::directed_scene::read::{at, child};

const REFUSE_NOTE: &str = " \"refuse\" asks for the same real frames as \"use\" (they differ in no calculation yet, scene-fx-decisions.md §18 #6); only \"hold\" accepts frozen ones.";

fn issue(
    code: &'static str,
    path: String,
    transition: &str,
    message: String,
    hint: String,
) -> Issue {
    Issue::new(code, path, message)
        .with_details(json!({ "transitionId": transition, "hint": hint }))
}

fn frames(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Frames a shot's instance and camera still have behind (`tail`) or before (`head`) the shot.
struct Reach {
    head_instance: i64,
    tail_instance: i64,
    head_camera: i64,
    tail_camera: i64,
}

fn reach(bundle: &Bundle, shot: &Shot) -> Option<Reach> {
    let instance = bundle.instance(&shot.scene_instance_id)?;
    let length = frames(shot.duration_frames());
    let start = frames(shot.instance_start_frame);
    let camera_in = frames(shot.camera_in_frame);
    // A camera that cannot be measured is taken as having every frame (the oracle's infinity).
    let camera = bundle.camera_clip_frames(&shot.camera_clip_id).map(frames);
    Some(Reach {
        head_instance: start,
        tail_instance: available_frames(frames(instance.duration_frames), start, length),
        head_camera: camera.map_or(i64::MAX, |_| camera_in),
        tail_camera: camera.map_or(i64::MAX, |total| available_frames(total, camera_in, length)),
    })
}

struct Lack<'a> {
    shot: &'a Shot,
    side: &'static str,
    instance: i64,
    camera: i64,
}

fn lack<'a>(
    shot: &'a Shot,
    side: &'static str,
    needed: i64,
    instance_handle: i64,
    camera_handle: i64,
) -> Option<Lack<'a>> {
    let instance = (needed - instance_handle).max(0);
    let camera = (needed - camera_handle).max(0);
    (instance > 0 || camera > 0).then_some(Lack {
        shot,
        side,
        instance,
        camera,
    })
}

fn describe_lack(lack: &Lack<'_>) -> String {
    let most = lack.instance.max(lack.camera);
    let what = if lack.instance > 0 && lack.camera > 0 {
        format!(
            "scene instance {}, camera clip {}",
            lack.instance, lack.camera
        )
    } else if lack.instance > 0 {
        "scene instance".to_string()
    } else {
        "camera clip".to_string()
    };
    format!(
        "{} lacks {most} frames {} ({what})",
        lack.shot.id, lack.side
    )
}

fn fixes_of(lack: &Lack<'_>) -> Vec<String> {
    let mut fixes = Vec::new();
    if lack.instance > 0 {
        fixes.push(format!(
            "give {} {} more frames of its scene instance {} {}",
            lack.shot.id, lack.instance, lack.shot.scene_instance_id, lack.side
        ));
    }
    if lack.camera > 0 {
        fixes.push(format!(
            "extend its camera clip {} by {} frames {}",
            lack.shot.camera_clip_id, lack.camera, lack.side
        ));
    }
    fixes
}

/// `TRANSITION_NO_HANDLES` for `use` and `refuse`: the window asks the outgoing shot for frames
/// after its end and the incoming one for frames before its start. `hold` never asks.
fn handle_issue(
    draft: &TransitionDraft,
    handles: Handles,
    from: (&Shot, &Reach),
    to: (&Shot, &Reach),
    window: TransitionWindow,
    path: &str,
) -> Option<Issue> {
    if !handles.needs_frames() {
        return None;
    }
    let lacks: Vec<Lack<'_>> = [
        lack(
            from.0,
            "after its end",
            window.after,
            from.1.tail_instance,
            from.1.tail_camera,
        ),
        lack(
            to.0,
            "before its start",
            window.before,
            to.1.head_instance,
            to.1.head_camera,
        ),
    ]
    .into_iter()
    .flatten()
    .collect();
    if lacks.is_empty() {
        return None;
    }
    let fixes: Vec<String> = lacks.iter().flat_map(fixes_of).collect();
    let mut hint = format!(
        "Set handles to \"hold\" to freeze the first/last frame, shorten the transition, or {}.",
        fixes.join("; ")
    );
    if handles == Handles::Refuse {
        hint.push_str(REFUSE_NOTE);
    }
    let message = format!(
        "Transition {} (handles: {}): {}",
        draft.id,
        handles.name(),
        lacks
            .iter()
            .map(describe_lack)
            .collect::<Vec<_>>()
            .join(" and ")
    );
    let lacks_json: Vec<Value> = lacks
        .iter()
        .map(|lack| {
            json!({
                "shot": lack.shot.id, "side": lack.side,
                "instance": lack.instance, "camera": lack.camera,
            })
        })
        .collect();
    Some(
        Issue::new(TRANSITION_NO_HANDLES, path, message).with_details(json!({
            "transitionId": draft.id, "hint": hint, "lacks": lacks_json,
        })),
    )
}

/// Steps 2 to 11 for every transition. `Ok` carries the typed transitions of a bundle with no
/// finding at all; `Err` carries every finding, in the order the contract fixes.
pub(in crate::directed_scene) fn transition_issues(
    bundle: &Bundle,
    drafts: &[TransitionDraft],
) -> Result<Vec<FxTransition>, Vec<Issue>> {
    let mut shots: Vec<&Shot> = bundle.shots.iter().collect();
    shots.sort_by_key(|shot| shot.start_frame);
    let edit = frames(bundle.edit_duration_frames);
    let base = child(&child("", "fx"), "transitions");
    let mut issues: Vec<Issue> = Vec::new();
    let mut windows: Vec<(usize, &str, TransitionWindow)> = Vec::new();
    let mut cuts: HashMap<(usize, usize), &str> = HashMap::new();

    for draft in drafts {
        let path = at(&base, draft.index);
        let id = draft.id.as_str();
        issues.extend(draft.early.iter().cloned());

        let from_index = shots.iter().position(|shot| shot.id == draft.from_shot_id);
        let to_index = shots.iter().position(|shot| shot.id == draft.to_shot_id);
        for (wanted, found, key) in [
            (&draft.from_shot_id, from_index, "fromShotId"),
            (&draft.to_shot_id, to_index, "toShotId"),
        ] {
            if found.is_none() {
                issues.push(issue(
                    UNKNOWN_SHOT,
                    child(&path, key),
                    id,
                    format!("Transition {id} names shot {wanted}, which does not exist"),
                    format!(
                        "Shots: {}.",
                        shots
                            .iter()
                            .map(|shot| shot.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
        }
        let (Some(from_index), Some(to_index)) = (from_index, to_index) else {
            continue;
        };
        let (from_shot, to_shot) = (shots[from_index], shots[to_index]);

        if to_index != from_index + 1 {
            let tail = match shots.get(from_index + 1) {
                None => " (this is the last shot)".to_string(),
                Some(next) => format!(" (after it comes {})", next.id),
            };
            issues.push(issue(
                TRANSITION_NOT_AT_CUT,
                path,
                id,
                format!(
                    "Transition {id}: {} does not follow {} in the edit",
                    to_shot.id, from_shot.id
                ),
                format!("A transition sits on a cut, between a shot and the one after it{tail}."),
            ));
            continue;
        }

        if let Some(first) = cuts.get(&(from_index, to_index)) {
            issues.push(issue(
                TRANSITION_DUPLICATE,
                path.clone(),
                id,
                format!(
                    "Transitions {first} and {id} are on the same cut ({} to {})",
                    from_shot.id, to_shot.id
                ),
                "One transition per cut: setTransition replaces the one that is there.".to_string(),
            ));
        }
        cuts.insert((from_index, to_index), id);

        let duration = draft.duration_frames;
        if let Err(error) = check_duration(duration, edit) {
            let (message, hint) = match error {
                WindowError::DurationBelowOne => (
                    format!("Transition {id} lasts {duration} frames"),
                    "A transition lasts at least 1 frame.".to_string(),
                ),
                _ => (
                    format!(
                        "Transition {id} lasts {duration} frames, more than the whole edit ({edit})"
                    ),
                    "A transition cannot be longer than the edit: shorten it.".to_string(),
                ),
            };
            issues.push(issue(
                TRANSITION_OUT_OF_RANGE,
                child(&path, "durationFrames"),
                id,
                message,
                hint,
            ));
            continue;
        }
        let Some(alignment) = draft.alignment else {
            continue;
        };
        let cut = frames(to_shot.start_frame);
        let window = match transition_window(cut, duration, alignment, edit) {
            Ok(window) => window,
            Err(WindowError::OutsideEdit(window)) => {
                issues.push(issue(
                    TRANSITION_OUT_OF_RANGE,
                    path.clone(),
                    id,
                    format!(
                        "Transition {id} (frames {} to {}) leaves the edit (0 to {edit})",
                        window.start, window.end
                    ),
                    "Shorten it or change its alignment so the window stays inside the edit."
                        .to_string(),
                ));
                window
            }
            Err(_) => {
                issues.push(issue(
                    TRANSITION_OUT_OF_RANGE,
                    path,
                    id,
                    format!(
                        "Transition {id}: a window of {duration} frames at the cut {cut} does not fit the frame arithmetic"
                    ),
                    "Shorten it or change its alignment so the window stays inside the edit."
                        .to_string(),
                ));
                continue;
            }
        };
        // The oracle lists the window for the overlap rule even when it left the edit.
        windows.push((draft.index, id, window));

        let from_length = frames(from_shot.duration_frames());
        let to_length = frames(to_shot.duration_frames());
        if from_length < window.before || to_length < window.after {
            let most = max_transition_frames(alignment, from_length, to_length);
            issues.push(issue(
                TRANSITION_TOO_LONG,
                path.clone(),
                id,
                format!(
                    "Transition {id} needs {} frames inside {} ({from_length}) and {} inside {} ({to_length})",
                    window.before, from_shot.id, window.after, to_shot.id
                ),
                format!(
                    "With alignment {} the longest these two shots hold is {most} frames.",
                    alignment.name()
                ),
            ));
        }
        if let Some(handles) = draft.handles
            && let (Some(from_reach), Some(to_reach)) =
                (reach(bundle, from_shot), reach(bundle, to_shot))
            && let Some(found) = handle_issue(
                draft,
                handles,
                (from_shot, &from_reach),
                (to_shot, &to_reach),
                window,
                &path,
            )
        {
            issues.push(found);
        }
    }

    // Overlap, by start of window: each window against the one before it.
    windows.sort_by_key(|(_, _, window)| window.start);
    for pair in windows.windows(2) {
        let (_, before_id, before) = pair[0];
        let (index, after_id, after) = pair[1];
        if after.start < before.end {
            issues.push(issue(
                TRANSITION_OVERLAP,
                at(&base, index),
                after_id,
                format!(
                    "Transitions {before_id} (frames {} to {}) and {after_id} (from frame {}) overlap",
                    before.start, before.end, after.start
                ),
                "Shorten one of them or change its alignment: the windows of neighbouring cuts cannot share a frame."
                    .to_string(),
            ));
        }
    }

    if !issues.is_empty() {
        return Err(issues);
    }
    Ok(drafts
        .iter()
        .filter_map(|draft| {
            let resolved = draft.resolved.as_ref()?;
            Some(FxTransition {
                id: draft.id.clone(),
                effect: resolved.effect,
                from_shot_id: draft.from_shot_id.clone(),
                to_shot_id: draft.to_shot_id.clone(),
                duration_frames: draft.duration_frames,
                alignment: draft.alignment?,
                handles: draft.handles?,
                easing: resolved.easing,
                params: resolved.params.clone(),
                seed: resolved.seed,
                hidden: resolved.hidden,
            })
        })
        .collect())
}

/// What an admitted scene costs or risks, never a refusal: a transition on a continuous
/// junction (both sides show the same instance), and one between two shots that both draw a
/// splat scene (two decks of splats, about twice the cost per frame). Hidden transitions draw
/// nothing and warn of nothing. Paths point at the transition (`/fx/transitions/<i>`).
pub fn transition_warnings(bundle: &Bundle) -> Vec<Issue> {
    let base = child(&child("", "fx"), "transitions");
    let has_splat = |shot: &Shot| {
        bundle
            .scene_of_instance(&shot.scene_instance_id)
            .is_some_and(|scene| {
                scene
                    .nodes()
                    .iter()
                    .any(|node| node.kind == NodeKind::Splat)
            })
    };
    let mut warnings = Vec::new();
    for (index, transition) in bundle.fx.transitions.iter().enumerate() {
        if transition.hidden {
            continue;
        }
        let (Some(from), Some(to)) = (
            bundle.shot(&transition.from_shot_id),
            bundle.shot(&transition.to_shot_id),
        ) else {
            continue;
        };
        let id = transition.id.as_str();
        if to.transition_into == Some(Transition::Continuous) {
            warnings.push(issue(
                TRANSITION_ON_CONTINUOUS,
                at(&base, index),
                id,
                format!(
                    "Transition {id} is on a continuous junction: {} follows {} without a cut",
                    to.id, from.id
                ),
                "A continuous junction means the picture should not change; a transition on it is allowed, check that it is wanted.".to_string(),
            ));
        }
        if has_splat(from) && has_splat(to) {
            warnings.push(issue(
                TRANSITION_SPLAT_COST,
                at(&base, index),
                id,
                format!(
                    "Transition {id} draws two splat scenes at once ({} and {}); a frame in its window costs about twice a single shot",
                    from.id, to.id
                ),
                "Use handles \"hold\" for a lighter preview, or keep the window short.".to_string(),
            ));
        }
    }
    warnings
}
