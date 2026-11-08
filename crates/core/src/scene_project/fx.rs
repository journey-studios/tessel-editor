//! Running the F016 effect commands (`scene.fx.*`) against a project. Every rule is the core's
//! (`directed_scene::fx` judges a layer, `scene_edit::apply_fx` edits it atomically); this file
//! only decides what the project does with the answer, exactly as `edit_scene` does for
//! `scene.edit`: a rejection, a dry run and a no-op leave the project (and its undo history)
//! untouched, and a real change is one replacement, so `EditorState::execute` records one entry.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::directed_scene::DirectedScene;
use crate::directed_scene::fx::{self, FX_CATALOG_VERSION};
use crate::scene_edit::{self, FxOp, FxSummary};
use crate::scene_interaction::FrameAt;
use crate::state::{CommandError, EditorState};

/// What the layer covers and what it refuses, as data: reported by `scene.fx.catalog`.
pub(super) fn fx_scope_json() -> Value {
    json!({
        "catalog_version": FX_CATALOG_VERSION,
        "domain": "bundle.fx of the directed scene (render-bundle document key); one model, no copy in the sequence timeline",
        "scope": ["output"],
        "evaluation": "deterministic per edit frame; envelope sampled at the middle of the frame",
        "pass_budget": {
            "max_passes": fx::FX_PASS_BUDGET,
            "policy": "warn",
            "warning": fx::FX_PASS_BUDGET_WARNING,
            "meaning": "a frame with more passes is valid and drawn in full, only slower; list, edit and frame report it as a warning",
        },
        "refused": {
            "keys": fx::FX_KEYS_NOT_YET,
            "scope shot|node": fx::FX_SCOPE_NOT_YET,
            "transitions": fx::FX_TRANSITION_NOT_YET,
        },
    })
}

pub(super) fn catalog_answer() -> Value {
    let mut answer = fx::catalog_json();
    answer["action"] = json!("scene.fx.catalog");
    answer["scope"] = fx_scope_json();
    answer
}

pub(super) fn list_answer(id: &str, scene: &DirectedScene) -> Value {
    let layer = &scene.bundle().fx;
    let mut tracks = Vec::new();
    for track in &layer.tracks {
        tracks.push(json!({
            "id": track.id, "label": track.label, "hidden": track.hidden,
            "clips": track.clips.iter().map(fx::clip_json).collect::<Vec<_>>(),
        }));
    }
    let budget = layer.pass_budget(scene.bundle().edit_duration_frames);
    json!({
        "action": "scene.fx.list", "scene": id, "catalog_version": FX_CATALOG_VERSION,
        "edit_duration_frames": scene.bundle().edit_duration_frames,
        "track_count": layer.tracks.len(), "clip_count": layer.clip_count(),
        "tracks": tracks,
        "budget": budget.to_json(), "warnings": budget.warnings(),
    })
}

pub(super) fn frame_answer(
    id: &str,
    scene: &DirectedScene,
    at: &FrameAt,
) -> Result<Value, CommandError> {
    let frame = match at {
        FrameAt::Frame(frame) => *frame,
        FrameAt::Time(time) => scene.bundle().frame_at_time(*time)?,
    };
    let length = scene.bundle().edit_duration_frames;
    if frame < 0 || u64::try_from(frame).is_ok_and(|frame| frame >= length) {
        return Err(CommandError::new(
            "frame_out_of_range",
            format!("Frame {frame} is outside the edit (0 to {length}, exclusive)"),
        )
        .target(id));
    }
    let passes: Vec<Value> = scene
        .bundle()
        .fx
        .resolve(frame)
        .iter()
        .map(fx::FxPass::to_json)
        .collect();
    let over = passes.len() > fx::FX_PASS_BUDGET;
    let warnings: Vec<Value> = if over {
        vec![json!({
            "code": fx::FX_PASS_BUDGET_WARNING,
            "message": format!(
                "this frame draws {} effect passes, more than the {} the budget allows without a slowdown",
                passes.len(), fx::FX_PASS_BUDGET
            ),
            "max_passes": fx::FX_PASS_BUDGET, "passes": passes.len(),
        })]
    } else {
        Vec::new()
    };
    Ok(json!({
        "action": "scene.fx.frame", "scene": id, "edit_frame": frame,
        "count": passes.len(), "passes": passes,
        "max_passes": fx::FX_PASS_BUDGET, "over_budget": over, "warnings": warnings,
    }))
}

pub(super) fn changes_json(summary: &FxSummary) -> Vec<Value> {
    summary
        .changes
        .iter()
        .map(scene_edit::FxChange::to_json)
        .collect()
}

impl EditorState {
    pub(super) fn edit_fx(
        &mut self,
        id: &str,
        ops: &[FxOp],
        dry_run: bool,
    ) -> Result<Value, CommandError> {
        let attached = self
            .scenes
            .get(id)
            .ok_or_else(|| CommandError::unknown_element("scene", id))?;
        let current = Arc::clone(&attached.scene);
        let outcome = scene_edit::apply_fx(&current, ops)?;
        let summary = outcome.summary;
        let clips = outcome.candidate.bundle().fx.clip_count();
        let budget = outcome
            .candidate
            .bundle()
            .fx
            .pass_budget(outcome.candidate.bundle().edit_duration_frames);
        let warnings = budget.warnings();
        if dry_run {
            if summary.modified {
                self.scenes.check_replacement_size(id, &outcome.candidate)?;
            }
            return Ok(json!({
                "action": "scene.fx.edit", "scene": id, "dry_run": true, "changed": false,
                "would_change": summary.modified, "changed_ops": summary.changed_count(),
                "clip_count": clips, "changes": changes_json(&summary),
                "budget": budget.to_json(), "warnings": warnings,
            }));
        }
        if !summary.modified {
            return Ok(json!({
                "action": "scene.fx.edit", "scene": id, "dry_run": false, "changed": false,
                "noop": true, "changed_ops": summary.changed_count(),
                "clip_count": clips, "changes": changes_json(&summary),
                "budget": budget.to_json(), "warnings": warnings,
            }));
        }
        self.scenes.replace_scene(id, outcome.candidate)?;
        Ok(json!({
            "action": "scene.fx.edit", "scene": id, "dry_run": false, "changed": true,
            "noop": false, "changed_ops": summary.changed_count(),
            "clip_count": clips, "changes": changes_json(&summary),
            "budget": budget.to_json(), "warnings": warnings,
        }))
    }
}
