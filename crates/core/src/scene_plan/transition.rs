//! The plan of a frame inside a transition window (F064, P4b S1b): two decks and the mix.
//!
//! The top-level `ScenePlan` stays the deck under the needle (consumers, gizmos, the resource list
//! and `scene.fx.frame` do not change); the neighbour is additive, under `transition.other`, and is
//! a `ScenePlan` read, written and validated by the same code (no second model of a plan).
//! `progress` is the value already eased and `amount` the clamped `intensity`: a runtime computes
//! no easing and keeps no rule. The shader's own smoothing of `progress` is the runtime's.
//!
//! Where the rest lives: JSON in [`transition_json`](super::transition_json), the invariants in
//! [`transition_validate`](super::transition_validate).

use std::collections::BTreeMap;

use crate::directed_scene::fx::{Alignment, DeckSide, FxEasing, FxTransitionFrame, FxValue};
use crate::directed_scene::{AssetAlpha, AssetColorSpace, AssetFormat};

use super::PlanError;
use super::model::{PlanResource, ScenePlan};
use super::temporal::TransitionTiming;

/// The most bytes of the JSON text of a plan that carries a `transition` (the two decks together).
/// Fixed by the S0 contract before the code; a plan without a `transition` has no such ceiling.
pub const MAX_TRANSITION_PLAN_BYTES: usize = 4_194_304;

/// The most items (nodes, layers, splats, passes, resources and lights) of the two decks of a plan
/// that carries a `transition`. Fixed by the S0 contract before the code.
pub const MAX_TRANSITION_PLAN_ITEMS: usize = 20_000;

/// The mix and the neighbour deck of one frame of a transition window.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanTransition {
    pub id: String,
    pub effect: String,
    pub from_shot_id: String,
    pub to_shot_id: String,
    /// The deck under the needle (the plan's top level): `from` before the cut, `to` from it.
    pub primary: DeckSide,
    pub start_frame: i64,
    pub duration_frames: i64,
    /// Where the window sits on the cut: with the cut and the duration it fixes `start_frame`.
    pub alignment: Alignment,
    /// `edit_frame − start_frame`, inside `[0, duration_frames)`.
    pub frame_in_window: i64,
    pub easing: FxEasing,
    /// `ease(easing, (frame_in_window + 0.5) / duration_frames)`.
    pub progress: f64,
    /// `clamp01(intensity)`.
    pub amount: f64,
    /// Every parameter of the effect in catalog order (`curve`, `intensity`), an ordered list: a
    /// map would reorder with the build's JSON features.
    pub params: Vec<(String, FxValue)>,
    /// Where the two shots sit in the edit, their instances and camera clips, and the clocks the
    /// rule gave each deck at this frame (`PlanTransition::clocks` reads them back).
    pub timing: TransitionTiming,
    /// The neighbour's instance frame was held (it has no real frame there).
    pub held_instance: bool,
    /// The neighbour's camera clip frame was held.
    pub held_camera: bool,
    /// The neighbour deck: a plan of the other shot at the same real edit frame.
    pub other: ScenePlan,
}

/// Which deck of a two-deck plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeckRole {
    /// The plan's top level: the shot under the needle.
    Primary,
    /// `transition.other`.
    Other,
}

impl DeckRole {
    pub fn name(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Other => "other",
        }
    }
}

/// A node of one deck that uses a resource. Node ids collide between decks (two scenes may share
/// `root` and `figure`), so a node is only ever named together with its deck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeckUse {
    pub deck: DeckRole,
    pub node_id: String,
}

/// A resource of the union of both decks: the metadata of the resource (identical in both decks
/// when it is in both) and every node that uses it, primary deck first.
#[derive(Debug, Clone, PartialEq)]
pub struct UnionResource {
    pub asset_id: String,
    pub revision_id: String,
    pub content_hash: String,
    pub relative_path: String,
    pub format: AssetFormat,
    pub width_px: u64,
    pub height_px: f64,
    pub alpha: AssetAlpha,
    pub color_space: AssetColorSpace,
    pub used_by: Vec<DeckUse>,
}

/// The nodes of `resource` in `deck`, qualified.
fn uses(resource: &PlanResource, deck: DeckRole) -> Vec<DeckUse> {
    resource
        .used_by
        .iter()
        .map(|node_id| DeckUse {
            deck,
            node_id: node_id.clone(),
        })
        .collect()
}

impl UnionResource {
    fn of(resource: &PlanResource, deck: DeckRole) -> Self {
        Self {
            asset_id: resource.asset_id.clone(),
            revision_id: resource.revision_id.clone(),
            content_hash: resource.content_hash.clone(),
            relative_path: resource.relative_path.clone(),
            format: resource.format,
            width_px: resource.width_px,
            height_px: resource.height_px,
            alpha: resource.alpha,
            color_space: resource.color_space,
            used_by: uses(resource, deck),
        }
    }
    /// Whether `resource` is this very file (everything but the nodes that use it).
    fn is_same_file(&self, resource: &PlanResource) -> bool {
        self.revision_id == resource.revision_id
            && self.content_hash == resource.content_hash
            && self.relative_path == resource.relative_path
            && self.format == resource.format
            && self.width_px == resource.width_px
            && self.height_px.to_bits() == resource.height_px.to_bits()
            && self.alpha == resource.alpha
            && self.color_space == resource.color_space
    }
}

impl ScenePlan {
    /// The decks of the plan, primary first: one for a plain plan, two inside a transition.
    pub fn decks(&self) -> Vec<(DeckRole, &ScenePlan)> {
        let mut decks = vec![(DeckRole::Primary, self)];
        if let Some(transition) = &self.transition {
            decks.push((DeckRole::Other, &transition.other));
        }
        decks
    }

    /// The resources of both decks, by `assetId` (ascending), each with the nodes that use it
    /// qualified by deck. Derived and never serialized: each deck keeps its own `resources` and
    /// its own node ids; this is the view a host loader and the preflight iterate.
    ///
    /// The same `assetId` with another revision, content hash, path or metadata in the two decks
    /// is not one resource: `invalid` at `/transition/other/resources`. With one scene that cannot
    /// happen by construction; the check defends against a plan from outside.
    pub fn resource_union(&self) -> Result<BTreeMap<String, UnionResource>, PlanError> {
        let mut union: BTreeMap<String, UnionResource> = BTreeMap::new();
        for (deck, plan) in self.decks() {
            for (position, resource) in plan.resources.iter().enumerate() {
                match union.get_mut(&resource.asset_id) {
                    None => {
                        union.insert(resource.asset_id.clone(), UnionResource::of(resource, deck));
                    }
                    Some(found) if found.is_same_file(resource) => {
                        found.used_by.extend(uses(resource, deck));
                    }
                    Some(_) => {
                        return Err(PlanError::invalid(
                            format!("/transition/other/resources/{position}"),
                            format!(
                                "asset `{}` is in both decks with different content",
                                resource.asset_id
                            ),
                        ));
                    }
                }
            }
        }
        Ok(union)
    }

    /// The plan of the deck under the needle with the neighbour attached: the transition's
    /// metadata, the union of what both decks require, and a last look at the relations between
    /// the decks. `other` and `self` were each built and validated as a plain plan.
    pub(super) fn with_transition(
        mut self,
        frame: &FxTransitionFrame<'_>,
        timing: TransitionTiming,
        other: ScenePlan,
    ) -> Result<Self, PlanError> {
        let transition = frame.transition;
        // The neighbour's flags are the rule's own, from the very numbers the plan declares.
        let edit_frame = i64::try_from(self.frame.edit_frame).map_err(|_| {
            PlanError::invalid("/frame/editFrame", "does not fit a signed 64-bit frame")
        })?;
        let held = timing.deck(frame.primary.other()).clocks_at(edit_frame)?;
        let params = transition
            .effect
            .params
            .iter()
            .zip(&transition.params)
            .map(|(spec, value)| (spec.id.to_string(), *value))
            .collect();
        self.transition = Some(Box::new(PlanTransition {
            id: transition.id.clone(),
            effect: transition.effect.id.to_string(),
            from_shot_id: transition.from_shot_id.clone(),
            to_shot_id: transition.to_shot_id.clone(),
            primary: frame.primary,
            start_frame: frame.start_frame,
            duration_frames: transition.duration_frames,
            alignment: transition.alignment,
            frame_in_window: frame.frame_in_window,
            easing: transition.easing,
            progress: frame.progress,
            amount: frame.amount,
            params,
            timing,
            held_instance: held.instance.held,
            held_camera: held.camera.held,
            other,
        }));
        self.requires = self.expected_requires();
        self.check_transition()?;
        self.check_transition_bytes()?;
        Ok(self)
    }

    /// The text of a plan with a transition must fit the ceiling too: one that did not could be
    /// built but never read back.
    fn check_transition_bytes(&self) -> Result<(), PlanError> {
        let length = self.to_json_string().len();
        if length > MAX_TRANSITION_PLAN_BYTES {
            return Err(PlanError::invalid(
                "/transition",
                format!(
                    "the plan is {length} bytes, more than the {MAX_TRANSITION_PLAN_BYTES} a plan with a transition may have"
                ),
            ));
        }
        Ok(())
    }
}
