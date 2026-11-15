//! The invariants of a plan with a `transition` (F064, P4b S1b): what a runtime relies on without
//! re-deriving. They run when a plan is built and when one is read from JSON, through
//! `validate_structure`, so a plan that breaks them is never handed out and never accepted.
//!
//! Every refusal is `PlanError::Invalid` with a JSON Pointer. A deck-level fault of the neighbour
//! carries its own pointer under `/transition/other`.

use std::collections::BTreeSet;

use crate::directed_scene::fx::{DeckSide, FxScope, id_problem, parse_param, transition_effect};

use super::model::{Capability, ScenePlan};
use super::transition::{MAX_TRANSITION_PLAN_ITEMS, PlanTransition};
use super::validate::derive_requires;
use super::{PLAN_VERSION, PlanError};

/// Tolerance of the recomputed `progress` and `amount` (plan §7.3).
const TOLERANCE: f64 = 1e-9;

fn invalid(pointer: &str, message: impl Into<String>) -> PlanError {
    PlanError::invalid(pointer, message)
}

/// An error of the neighbour deck, seen from the top: the pointer moves under `/transition/other`.
pub(super) fn within_other(error: PlanError) -> PlanError {
    match error {
        PlanError::Invalid { pointer, message } => PlanError::Invalid {
            pointer: format!("/transition/other{pointer}"),
            message,
        },
        error @ PlanError::UnsupportedVersion { .. } => {
            invalid("/transition/other/planVersion", error.to_string())
        }
        error @ PlanError::SplatTransform { .. } => {
            invalid("/transition/other/nodes", error.to_string())
        }
        error => invalid("/transition/other", error.to_string()),
    }
}

impl ScenePlan {
    /// The items one deck counts against `MAX_TRANSITION_PLAN_ITEMS`.
    fn own_items(&self) -> usize {
        self.nodes.len()
            + self.layers.len()
            + self.splats.len()
            + self.fx.len()
            + self.resources.len()
            + self.lights.len()
    }

    /// Items of both decks (a plain plan has one).
    pub(super) fn transition_items(&self) -> usize {
        self.own_items()
            + self
                .transition
                .as_ref()
                .map_or(0, |transition| transition.other.own_items())
    }

    /// What the plan must ask of a runtime: the ordered union of what each deck needs, plus
    /// `fxTransition` when there is a transition. A plain plan asks what it always did.
    pub(super) fn expected_requires(&self) -> Vec<Capability> {
        let own = |plan: &ScenePlan| {
            derive_requires(
                &plan.nodes,
                &plan.layers,
                &plan.splats,
                &plan.animation,
                &plan.fx,
                &plan.lights,
            )
        };
        let mut found: BTreeSet<Capability> = own(self).into_iter().collect();
        if let Some(transition) = &self.transition {
            found.extend(own(&transition.other));
            found.insert(Capability::FxTransition);
        }
        found.into_iter().collect()
    }

    /// The relations between the two decks and the transition's own fields. The structure of each
    /// deck is checked by `check_transition_decks`.
    pub(super) fn check_transition(&self) -> Result<(), PlanError> {
        let Some(transition) = self.transition.as_deref() else {
            return Ok(());
        };
        transition.other.check_depth()?;
        let items = self.transition_items();
        if items > MAX_TRANSITION_PLAN_ITEMS {
            return Err(invalid(
                "/transition",
                format!(
                    "the two decks hold {items} items, more than the {MAX_TRANSITION_PLAN_ITEMS} a plan with a transition may have"
                ),
            ));
        }
        self.check_transition_fields(transition)?;
        self.check_decks_relation(transition)?;
        self.resource_union().map(|_| ())
    }

    /// The structure of the neighbour as a plan of its own, and the similarity of its splats.
    pub(super) fn check_transition_decks(&self) -> Result<(), PlanError> {
        if let Some(transition) = &self.transition {
            // Before descending: a hand-built chain of neighbours must not recurse.
            transition.other.check_depth()?;
            transition
                .other
                .validate_structure()
                .map_err(within_other)?;
        }
        Ok(())
    }

    /// The neighbour of a transition has no transition of its own (depth 1).
    fn check_depth(&self) -> Result<(), PlanError> {
        if self.transition.is_some() {
            return Err(invalid(
                "/transition/other/transition",
                "a transition does not nest: the neighbour deck has none (depth 1)",
            ));
        }
        Ok(())
    }

    fn check_transition_fields(&self, t: &PlanTransition) -> Result<(), PlanError> {
        let Some(spec) = transition_effect(&t.effect) else {
            return Err(invalid(
                "/transition/effect",
                format!("`{}` is not a transition effect", t.effect),
            ));
        };
        for (pointer, id) in [
            ("/transition/id", &t.id),
            ("/transition/fromShotId", &t.from_shot_id),
            ("/transition/toShotId", &t.to_shot_id),
        ] {
            if id.is_empty() {
                return Err(invalid(pointer, "must not be empty"));
            }
        }
        if let Some((_, message)) = id_problem(&t.id) {
            return Err(invalid("/transition/id", message));
        }
        if t.from_shot_id == t.to_shot_id {
            return Err(invalid(
                "/transition/toShotId",
                "a transition joins two different shots",
            ));
        }
        if t.duration_frames < 1 {
            return Err(invalid("/transition/durationFrames", "must be at least 1"));
        }
        if t.start_frame < 0 {
            return Err(invalid("/transition/startFrame", "must not be negative"));
        }
        if !(0..t.duration_frames).contains(&t.frame_in_window) {
            return Err(invalid(
                "/transition/frameInWindow",
                "must be inside the window, from 0 to durationFrames − 1",
            ));
        }
        let edit_frame = i64::try_from(self.frame.edit_frame).ok();
        if !matches!(
            (edit_frame, t.start_frame.checked_add(t.frame_in_window)),
            (Some(frame), Some(in_window)) if frame == in_window
        ) {
            return Err(invalid(
                "/transition/frameInWindow",
                "must be the edit frame minus startFrame",
            ));
        }
        let expected = t
            .easing
            .apply((t.frame_in_window as f64 + 0.5) / t.duration_frames as f64);
        if !t.progress.is_finite()
            || !(0.0..=1.0).contains(&t.progress)
            || (t.progress - expected).abs() > TOLERANCE
        {
            return Err(invalid(
                "/transition/progress",
                "must be the eased position in the window, ease((frameInWindow + 0.5) / durationFrames)",
            ));
        }
        if t.params.len() != spec.params.len() {
            return Err(invalid(
                "/transition/params",
                "must hold exactly the parameters of the effect, in catalog order",
            ));
        }
        for (position, ((name, value), param)) in t.params.iter().zip(spec.params).enumerate() {
            let at = format!("/transition/params/{position}");
            if name != param.id {
                return Err(invalid(
                    &at,
                    "must hold exactly the parameters of the effect, in catalog order",
                ));
            }
            if let Err((_, message)) = parse_param(param, &value.to_json()) {
                return Err(invalid(&at, message));
            }
        }
        let intensity = t
            .params
            .iter()
            .find(|(name, _)| name == "intensity")
            .and_then(|(_, value)| value.as_number())
            .unwrap_or(1.0);
        if !t.amount.is_finite()
            || !(0.0..=1.0).contains(&t.amount)
            || (t.amount - intensity.clamp(0.0, 1.0)).abs() > TOLERANCE
        {
            return Err(invalid(
                "/transition/amount",
                "must be the intensity of the transition, between 0 and 1",
            ));
        }
        Ok(())
    }

    fn check_decks_relation(&self, t: &PlanTransition) -> Result<(), PlanError> {
        let other = &t.other;
        // The deck under the needle is `from` before the cut and `to` from it.
        let (primary_shot, other_shot) = match t.primary {
            DeckSide::From => (&t.from_shot_id, &t.to_shot_id),
            DeckSide::To => (&t.to_shot_id, &t.from_shot_id),
        };
        if self.frame.shot_id != *primary_shot {
            return Err(invalid(
                "/transition/primary",
                format!(
                    "the primary deck is the `{}` shot (`{primary_shot}`), not the shot of the frame (`{}`)",
                    t.primary.name(),
                    self.frame.shot_id
                ),
            ));
        }
        if other.frame.shot_id != *other_shot {
            return Err(invalid(
                "/transition/other/frame/shotId",
                format!("the neighbour deck is the shot `{other_shot}`"),
            ));
        }
        if other.version != self.version || other.version != PLAN_VERSION {
            return Err(invalid(
                "/transition/other/planVersion",
                "must be the version of the primary deck",
            ));
        }
        if other.output != self.output {
            return Err(invalid(
                "/transition/other/output",
                "must be the output of the primary deck (size, frame rate, alpha)",
            ));
        }
        if other.source.bundle_id != self.source.bundle_id
            || other.source.revision_id != self.source.revision_id
        {
            return Err(invalid(
                "/transition/other/source",
                "both decks come from the same bundle revision",
            ));
        }
        if other.frame.edit_frame != self.frame.edit_frame {
            return Err(invalid(
                "/transition/other/frame/editFrame",
                "the neighbour deck is drawn at the real edit frame of the primary deck",
            ));
        }
        if other.frame.edit_time != self.frame.edit_time {
            return Err(invalid(
                "/transition/other/frame/editTime",
                "the scene clock is the real edit time, held or not",
            ));
        }
        // The clocks of both decks, the window, the ids and the times: `temporal/validate.rs`.
        // It replaces the old rule that a held head sat on frame 0: a camera that starts past
        // its clip is held on the tail even before the cut.
        self.check_transition_timing(t)?;
        let mut clips: BTreeSet<&str> = self.fx.iter().map(|pass| pass.clip_id.as_str()).collect();
        for (position, pass) in other.fx.iter().enumerate() {
            if matches!(pass.scope, FxScope::Output) {
                return Err(invalid(
                    &format!("/transition/other/fx/{position}/scope"),
                    "the output passes act once after the mix, in the primary deck only",
                ));
            }
            if !clips.insert(pass.clip_id.as_str()) {
                return Err(invalid(
                    &format!("/transition/other/fx/{position}/clipId"),
                    "a clip appears once per frame, across both decks",
                ));
            }
        }
        Ok(())
    }
}
