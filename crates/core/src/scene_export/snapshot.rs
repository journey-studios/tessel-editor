//! The immutable view of a scene an export reads for its whole duration.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::directed_scene::{DirectedScene, ImportError};
use crate::preset::{Fit, Preset};
use crate::scene_plan::{PlanTimeline, ScenePlan};
use crate::state::CommandError;
use crate::{EditorState, scene_project::AttachedScene};

use super::schedule::SceneExportSchedule;
use super::spec::{AudioChoice, SceneExportSpec};

/// What the file does about the scene's sound. There is no directed-scene audio mix yet, so the
/// only two outcomes are "the scene has none" and "the caller chose to leave it out".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioPlan {
    /// The scene declares no audio: the file has no audio stream.
    NoSources,
    /// The scene declares `declared` and the caller explicitly exported video only.
    Omitted { declared: Vec<&'static str> },
}

impl AudioPlan {
    pub fn to_json(&self) -> Value {
        match self {
            Self::NoSources => json!({ "included": false, "declared": [] }),
            Self::Omitted { declared } => json!({
                "included": false, "declared": declared,
                "omitted": "left out because the export asked for video only",
            }),
        }
    }
}

/// The audio families a bundle declares. Mix files and narration are the two the schema has.
fn declared_audio(scene: &DirectedScene) -> Vec<&'static str> {
    let mut declared = Vec::new();
    if scene.bundle().audio.is_some() {
        declared.push("audio");
    }
    if scene
        .document()
        .get("narration")
        .is_some_and(|value| !value.is_null())
    {
        declared.push("narration");
    }
    declared
}

fn audio_plan(
    scene: &DirectedScene,
    preset: &Preset,
    choice: AudioChoice,
) -> Result<AudioPlan, CommandError> {
    let declared = declared_audio(scene);
    if declared.is_empty() {
        return Ok(AudioPlan::NoSources);
    }
    if choice == AudioChoice::Omitted || !preset.audio {
        return Ok(AudioPlan::Omitted { declared });
    }
    Err(CommandError::new(
        "unsupported",
        "This scene declares audio and directed-scene audio is not exported yet; \
         pass `audio: false` to export the picture only",
    )
    .details(json!({ "domains": declared, "hint": "audio: false" })))
}

/// One export, frozen: the scene (shared, never copied), the preset, the schedule and the audio
/// decision. Every plan comes from [`ScenePlan::build`] on the frozen scene, so an edit, a gesture
/// or a detach that happens while the export runs cannot change a frame already promised.
#[derive(Debug, Clone)]
pub struct SceneExportSnapshot {
    scene_id: String,
    scene: Arc<DirectedScene>,
    preset: Preset,
    schedule: SceneExportSchedule,
    audio: AudioPlan,
    /// Every `assetId` any frame of the range draws, ascending. See [`required_assets`](Self::required_assets).
    required: Vec<String>,
}

impl SceneExportSnapshot {
    /// Freezes `spec` against the committed document: the scene as the project holds it, never a
    /// live gesture's candidate. Nothing in `state` changes, and a rejected request leaves no
    /// trace. Admission (structural joins and temporal gates) is checked again here for the same
    /// reason `scene.plan` does: a caller may have built the state without going through attach.
    pub fn capture(state: &EditorState, spec: &SceneExportSpec) -> Result<Self, CommandError> {
        let AttachedScene { scene, .. } = state
            .scenes
            .get(&spec.scene)
            .ok_or_else(|| CommandError::unknown_element("scene", &spec.scene))?;
        spec.preset.validate()?;
        let mut issues = scene.edit_join_issues();
        issues.extend(scene.temporal_issues());
        if !issues.is_empty() {
            return Err(ImportError::Invalid { issues }.into());
        }
        let schedule =
            SceneExportSchedule::new(scene.bundle(), &spec.preset, spec.from, spec.frames)?;
        let audio = audio_plan(scene, &spec.preset, spec.audio)?;
        let mut snapshot = Self {
            scene_id: spec.scene.clone(),
            scene: Arc::clone(scene),
            preset: spec.preset.clone(),
            schedule,
            audio,
            required: Vec::new(),
        };
        // One complete plan per shot the range touches, plus the last frame, stand for "this scene
        // can be drawn complete": a declared domain nobody evaluates (lighting, weather...) refuses
        // the export now, before a job, a staging folder or a renderer exist. Frames in between
        // can still fail alone; the job then fails without publishing anything.
        snapshot.required = snapshot.collect_required_assets()?;
        snapshot.plan(snapshot.schedule.frames - 1)?;
        Ok(snapshot)
    }

    /// WHY one frame per shot is enough: the image planes a plan draws belong to the scene
    /// definition of the shot's instance, not to the frame (animation moves them, never adds one),
    /// so the first frame of each shot lists every resource the shot will ever ask for.
    fn collect_required_assets(&self) -> Result<Vec<String>, CommandError> {
        let bundle = self.scene.bundle();
        let (from, to) = (self.schedule.from, self.schedule.to());
        let mut assets = std::collections::BTreeSet::new();
        let timeline = PlanTimeline::new(&self.scene);
        for shot in &bundle.shots {
            let start = i64::try_from(shot.start_frame)
                .unwrap_or(i64::MAX)
                .max(from);
            let end = i64::try_from(shot.end_frame_exclusive)
                .unwrap_or(i64::MAX)
                .min(to);
            if start >= end {
                continue;
            }
            let plan = timeline.build(start)?;
            assets.extend(
                plan.resources
                    .iter()
                    .map(|resource| resource.asset_id.clone()),
            );
        }
        Ok(assets.into_iter().collect())
    }

    /// The `assetId`s the export needs from the resource resolver, ascending. A resolver that
    /// cannot answer all of them refuses the export before it starts.
    pub fn required_assets(&self) -> &[String] {
        &self.required
    }

    pub fn scene_id(&self) -> &str {
        &self.scene_id
    }

    pub fn scene(&self) -> &Arc<DirectedScene> {
        &self.scene
    }

    pub fn preset(&self) -> &Preset {
        &self.preset
    }

    pub fn schedule(&self) -> &SceneExportSchedule {
        &self.schedule
    }

    pub fn audio(&self) -> &AudioPlan {
        &self.audio
    }

    /// The complete plan of output frame `index`: the same value `scene.plan` answers for the
    /// frame, never a partial one (`PlanError::Unsupported` surfaces as `unsupported`).
    ///
    /// One call asks one question; an export loop asks hundreds on the same scene, so it takes a
    /// [`Self::timeline`] once and calls [`Self::plan_with`] (BL064).
    pub fn plan(&self, index: u64) -> Result<ScenePlan, CommandError> {
        let frame = self.schedule.edit_frame(self.scene.bundle(), index)?;
        Ok(ScenePlan::build(&self.scene, frame)?)
    }

    /// What does not depend on the frame (instances, camera clips, node graphs), prepared once for
    /// the whole export. It borrows the frozen scene, so it lives in the job, next to the snapshot,
    /// not inside it.
    pub fn timeline(&self) -> PlanTimeline<'_> {
        PlanTimeline::new(&self.scene)
    }

    /// [`Self::plan`] through a prepared timeline: the same range check, the same frame mapping,
    /// the same plan and the same errors, without redoing the per-shot setup for every frame.
    pub fn plan_with(
        &self,
        timeline: &PlanTimeline<'_>,
        index: u64,
    ) -> Result<ScenePlan, CommandError> {
        let frame = self.schedule.edit_frame(self.scene.bundle(), index)?;
        if !std::ptr::eq(timeline.scene(), self.scene.as_ref()) {
            return Err(CommandError::new(
                "stale_scene",
                "The prepared timeline belongs to another scene snapshot",
            ));
        }
        Ok(timeline.build(frame)?)
    }

    /// Choices the caller should know about; none blocks the export.
    pub fn warnings(&self) -> Vec<Value> {
        let mut warnings = Vec::new();
        let warn = |code: &str, message: String| json!({ "code": code, "message": message });
        if self.schedule.converts_rate() {
            warnings.push(warn(
                "frame_rate_conversion",
                format!(
                    "The scene runs at {:.3} fps and the preset at {:.3} fps: frames are repeated or dropped, never blended",
                    self.schedule.scene_rate.as_f64(),
                    self.schedule.output_rate.as_f64()
                ),
            ));
        }
        if self.schedule.geometry.render != self.schedule.geometry.output {
            warnings.push(warn(
                "aspect_mismatch",
                match self.schedule.geometry.fit {
                    Fit::Contain => {
                        "The scene's aspect differs from the output: black bars are added"
                    }
                    Fit::Cover => {
                        "The scene's aspect differs from the output: the picture is cropped from the center"
                    }
                }
                .to_string(),
            ));
        }
        match &self.audio {
            AudioPlan::NoSources if self.preset.audio => warnings.push(warn(
                "no_audio_sources",
                "The scene declares no audio: the file has no audio stream".to_string(),
            )),
            AudioPlan::Omitted { declared } => warnings.push(warn(
                "audio_omitted",
                format!(
                    "The scene declares {} and it is not in the file: video only, as requested",
                    declared.join(" and ")
                ),
            )),
            AudioPlan::NoSources => {}
        }
        warnings
    }

    /// What the export will do, as the answer of `scene.export` and its preflight.
    pub fn summary(&self) -> Value {
        let bundle = self.scene.bundle();
        let schedule = &self.schedule;
        json!({
            "scene": self.scene_id,
            "bundle_id": bundle.bundle_id,
            "revision_id": bundle.revision_id,
            "preset": self.preset.id,
            "range": { "from": schedule.from, "frames": schedule.edit_frames, "to": schedule.to() },
            "frames": schedule.frames,
            "scene_fps": schedule.scene_rate.to_json(),
            "fps": schedule.output_rate.to_json(),
            "duration_seconds": schedule.duration_seconds(),
            "render": { "width": schedule.geometry.render.0, "height": schedule.geometry.render.1 },
            "output": { "width": self.preset.width, "height": self.preset.height },
            "fit": self.preset.fit.label(),
            "audio": self.audio.to_json(),
            "assets": self.required,
            "warnings": self.warnings(),
        })
    }
}
