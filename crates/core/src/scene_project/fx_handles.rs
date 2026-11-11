//! Running the F061 vignette handles (`scene.fx.select`, `scene.fx.handles`, and the effect branch
//! of `scene.pointer.*`) against a project. The geometry is
//! [`directed_scene::fx::handles`](crate::directed_scene::fx); what a view offers is
//! `scene_interaction::fx_handles`; this file is the glue that needs `EditorState`: the target the
//! session holds, the gesture slot and the history.
//!
//! WHY the drag is the F016 effect gesture and not a second mechanism: `down` on a handle runs
//! `scene.fx.gesture.begin`, every `move` runs `scene.fx.gesture.update` with ONE `set_param`
//! (the very op the inspector's field would send), and `up` runs `scene.fx.gesture.commit`, that
//! is `edit_fx`: ONE "Edit effects" entry, none when the candidate equals the base. The candidate
//! stays out of the document, a refused proposal keeps the last legal candidate, a stale gesture
//! is refused by name, and the document that results is the one `scene.fx.edit` gives for the same
//! values, bit for bit. The handle never writes a key and never takes the `FX_STATIC_UNDER_KEYS`
//! road: a parameter with keys has its handle off (`animated_param`).
//!
//! ONE target at a time. The session holds either entities (the node gizmo, the light handles) or
//! an effect clip; the funnel is `scene.fx.select` (sets the clip, empties the selection) and
//! `scene.select` (drops the clip). Swapping the target cancels a live drag, so no drag survives
//! onto a target that is not the one that began it.

use serde_json::{Value, json};

use crate::directed_scene::DirectedScene;
use crate::directed_scene::fx::VignetteHandle;
use crate::scene_edit::{self, FxOp};
use crate::scene_interaction::{
    Applied, Drag, DragAction, FrameAt, FxHandlesView, FxOverlay, FxTarget, GizmoCommand, Hidden,
    Overlay, Point, PointerCommand, SelectMode, fx_clip_has_handles,
};
use crate::state::{Command, CommandError, EditorState};

use super::fx::{changes_json, edit_frame_of};
use super::{HandleProbe, SceneCommand};

/// The handle of the targeted vignette that a point lies on: what a press there holds, or what it
/// refuses by name when the handle is drawn greyed. One reading for `scene.gizmo.hit` and
/// `scene.pointer.down`, so the query that asks "what is under this point" is the click.
struct FxPick {
    view: FxHandlesView,
    handle: VignetteHandle,
    /// The handle's parameter has keys: drawn, greyed, and refused (`animated_param`).
    animated: bool,
}

impl FxPick {
    /// The `hit` of the answer, the same for the query and the press.
    fn hit_json(&self) -> Value {
        json!({
            "kind": "fx_handle", "clip": self.view.clip, "handle": self.handle.as_str(),
            "param": self.handle.param(),
        })
    }

    /// Why a press here writes nothing, if it does not.
    fn refusal(&self) -> Option<&'static str> {
        self.animated
            .then_some(crate::scene_interaction::fx_handle_reason::ANIMATED_PARAM)
    }
}

/// What leaving the effect target said, carried from the `scene.select` a press ran.
pub(super) fn carry_fx_target(answer: &mut Value, outcome: &Value) {
    for key in ["fx_target_dropped", "cancelled_drag"] {
        if let Some(value) = outcome.get(key) {
            answer[key] = value.clone();
        }
    }
}

impl EditorState {
    /// What `scene.gizmo.hit` adds to its answer when a handle of the targeted vignette is under
    /// the point: the `hit` and, for a greyed handle, `refused`. False: no handle there.
    pub(super) fn fx_hit_into(
        &self,
        answer: &mut Value,
        scene: &str,
        frame: i64,
        point: Point,
    ) -> bool {
        let Some(pick) = self.fx_pick(scene, frame, point) else {
            return false;
        };
        answer["hit"] = pick.hit_json();
        if let Some(reason) = pick.refusal() {
            answer["refused"] = json!(reason);
        }
        true
    }

    /// `scene.fx.select`: which effect clip the monitor draws handles for (`clip` omitted or null
    /// clears it). Session state, never an edit. Selecting a clip empties the entity selection and
    /// cancels a live drag; only a vignette has handles (`fx_has_no_handles` for anything else).
    pub(super) fn fx_select(
        &mut self,
        scene: &str,
        clip: Option<&str>,
    ) -> Result<Value, CommandError> {
        let next = match clip {
            None => None,
            Some(clip) => {
                let entry = self.attached_scene(scene)?;
                // The view says `unknown_element` / `fx_has_no_handles` by name.
                FxHandlesView::of(&entry.scene, clip, 0).map_err(|error| {
                    if error.code == "unknown_element" {
                        error.details(json!({ "scene": scene }))
                    } else {
                        error
                    }
                })?;
                Some(FxTarget {
                    scene: scene.to_string(),
                    clip: clip.to_string(),
                })
            }
        };
        let previous = self.scene_interaction.fx_target.clone();
        let moved = next != previous;
        let cancelled = if moved { self.cancel_live_drag() } else { None };
        let mut cleared = false;
        if next.is_some() && !self.scene_interaction.selection.is_empty() {
            self.scene_interaction
                .selection
                .apply(None, Vec::new(), SelectMode::Clear)?;
            cleared = true;
        }
        self.scene_interaction.fx_target = next.clone();
        Ok(json!({
            "action": "scene.fx.select", "scene": scene, "clip": next.as_ref().map(|t| &t.clip),
            "target": next.as_ref().map(FxTarget::to_json),
            "previous": previous.as_ref().map(FxTarget::to_json),
            "selection_cleared": cleared, "cancelled_drag": cancelled,
            "changed": moved || cleared, "document_changed": false,
        }))
    }

    /// `scene.fx.handles`: what the monitor offers for one vignette clip at one frame of the
    /// document, the two ellipses, the handles off with their reason, and with a `probe` the value
    /// a pointer at a position would give a handle and whether the layer would take it (a dry run
    /// of the very op the drag sends). A read: writes nothing.
    pub(super) fn fx_handles(
        &self,
        scene: &str,
        clip: Option<&str>,
        at: &FrameAt,
        probe: Option<&HandleProbe>,
    ) -> Result<Value, CommandError> {
        let entry = self.attached_scene(scene)?;
        let target = self
            .scene_interaction
            .fx_target
            .as_ref()
            .filter(|target| target.scene == scene);
        let clip = match (clip, target) {
            (Some(clip), _) => clip.to_string(),
            (None, Some(target)) => target.clip.clone(),
            (None, None) => {
                return Err(CommandError::new(
                    "invalid_request",
                    "`clip` is required when no effect clip is selected for handles \
                     (`scene.fx.select`)",
                ));
            }
        };
        let frame = edit_frame_of(scene, &entry.scene, at)?;
        let view = FxHandlesView::of(&entry.scene, &clip, frame)?;
        let active = self
            .scene_interaction
            .live_drag()
            .and_then(|drag| match &drag.action {
                DragAction::Fx {
                    clip: dragged,
                    handle,
                    ..
                } if drag.scene == scene && *dragged == clip => Some(*handle),
                _ => None,
            });
        let mut answer = view.to_json(active);
        answer["action"] = json!("scene.fx.handles");
        answer["scene"] = json!(scene);
        answer["selected"] = json!(target.is_some_and(|target| target.clip == clip));
        answer["changed"] = json!(false);
        answer["document_changed"] = json!(false);
        if let Some(probe) = probe {
            answer["probe"] = handle_probe(&entry.scene, &clip, &view, probe);
        }
        Ok(answer)
    }

    /// Drops the target when the project no longer holds it (the clip was removed, the scene
    /// detached, an undo took it away) or it is no longer a vignette. Reports what it dropped.
    /// Run where the project changes (`EditorState::apply`, `restore`), never by a read.
    pub(crate) fn prune_fx_target(&mut self) -> Option<FxTarget> {
        let target = self.scene_interaction.fx_target.as_ref()?;
        let held = self
            .scenes
            .get(&target.scene)
            .is_some_and(|entry| fx_clip_has_handles(&entry.scene, &target.clip));
        if held {
            return None;
        }
        self.scene_interaction.fx_target.take()
    }

    /// The release that commits the drag of a vignette handle: its undo entry is the one
    /// `scene.fx.edit` makes. Decided before the command runs, while the drag is still open.
    pub(crate) fn is_fx_handle_release(&self, command: &Command) -> bool {
        matches!(
            command,
            Command::Scene(SceneCommand::Gizmo(GizmoCommand::Pointer(
                PointerCommand::Up { .. }
            )))
        ) && self.scene_interaction.live_drag().is_some_and(Drag::is_fx)
    }

    /// `scene.select` (any mode) and a press that selects leave the effect target: one target at a
    /// time. Said in the answer, and a live drag of the clip is cancelled with it.
    pub(super) fn leave_fx_target(&mut self, answer: &mut Value) {
        let Some(target) = self.scene_interaction.fx_target.take() else {
            return;
        };
        let cancelled = self.cancel_live_drag();
        answer["fx_target_dropped"] = target.to_json();
        if let Some(gesture) = cancelled {
            answer["cancelled_drag"] = json!(gesture);
        }
        answer["changed"] = json!(true);
    }

    /// Drops the drag in progress, whatever its target, with the gesture under it. Returns the
    /// gesture it dropped. Never touches the document.
    pub(super) fn cancel_live_drag(&mut self) -> Option<String> {
        let drag = self.live_drag()?;
        self.discard_drag();
        Some(drag.gesture)
    }

    /// The handle of the vignette the session targets that `point` lies on, or `None` when nothing
    /// there is for it (no target in this scene, the clip does not cover the frame, nothing under
    /// the point). A read: the query and the press share it.
    fn fx_pick(&self, scene: &str, frame: i64, point: Point) -> Option<FxPick> {
        let target = self
            .scene_interaction
            .fx_target
            .as_ref()
            .filter(|target| target.scene == scene)?;
        let view = FxHandlesView::of(&self.scenes.get(scene)?.scene, &target.clip, frame).ok()?;
        if !view.visible {
            return None;
        }
        let (handle, animated) = match view.hit(point) {
            Some(handle) => (handle, false),
            None => (view.hit_animated(point)?, true),
        };
        Some(FxPick {
            view,
            handle,
            animated,
        })
    }

    /// The press on a handle of the vignette the session targets, or `None` when the press is not
    /// for it ([`fx_pick`](Self::fx_pick) found nothing): the caller then does what a press always
    /// did. A press on a handle whose parameter has keys is answered here, as refused, and writes
    /// nothing; like every press it abandons the drag that is still open.
    pub(super) fn fx_pointer_down(
        &mut self,
        scene: &str,
        frame: i64,
        point: Point,
    ) -> Result<Option<Value>, CommandError> {
        let Some(pick) = self.fx_pick(scene, frame, point) else {
            return Ok(None);
        };
        let clip = pick.view.clip.clone();
        let mut answer = json!({
            "action": "scene.pointer", "phase": "down", "scene": scene, "frame": frame,
            "point": point.to_json(), "target": "fx", "clip": clip, "hit": pick.hit_json(),
            "abandoned": null, "gesture": null, "dragging": false, "selection_changed": false,
            "changed": false, "document_changed": false,
        });
        // A press while a drag is open means its release was lost: the open one goes first, so a
        // press never leaves two of them (also the press that is refused below).
        if let Some(open) = self.live_drag() {
            answer["abandoned"] = json!({ "gesture": open.gesture, "reason": "replaced_by_press" });
            self.discard_drag();
        }
        if let Some(reason) = pick.refusal() {
            // Drawn and greyed: the press says why it does nothing instead of picking what is
            // behind the handle (which would also drop the target).
            answer["refused"] = json!(reason);
            answer["overlay"] = self.gizmo_overlay(scene, frame)?.to_json();
            return Ok(Some(answer));
        }
        let FxPick { view, handle, .. } = pick;
        let begun = self.fx_gesture_begin(scene, &clip, None)?;
        let gesture = begun["gesture"]
            .as_str()
            .ok_or_else(|| {
                CommandError::new(
                    "internal",
                    "`scene.fx.gesture.begin` answered no gesture id",
                )
            })?
            .to_string();
        self.scene_interaction.drag = Some(Drag {
            gesture: gesture.clone(),
            scene: scene.to_string(),
            entity: clip.clone(),
            frame,
            node: None,
            action: DragAction::Fx {
                clip,
                handle,
                press: Box::new(view),
            },
            last: None,
        });
        answer["gesture"] = json!(gesture);
        answer["dragging"] = json!(true);
        answer["handle"] = json!(handle.as_str());
        answer["tool"] = json!("fx");
        answer["changed"] = json!(true);
        answer["overlay"] = self.gizmo_overlay(scene, frame)?.to_json();
        Ok(Some(answer))
    }

    /// One pointer position of a handle drag: the position becomes the value of the handle's
    /// parameter (quantized to its control's grid, inside its range) and goes into the gesture as
    /// ONE `set_param`. The caller (`pointer_move`) has checked that the gesture is live; a
    /// refusal of the layer here is an error answer and leaves the last candidate where it was.
    pub(super) fn fx_handle_update(
        &mut self,
        drag: &Drag,
        point: Point,
    ) -> Result<Value, CommandError> {
        let DragAction::Fx {
            clip,
            handle,
            press,
        } = &drag.action
        else {
            return Err(CommandError::new(
                "internal",
                "an effect update reached a drag that is not an effect handle",
            ));
        };
        let Some(value) = press.pointer_value(*handle, point) else {
            return Ok(json!({ "applied": false, "reason": "degenerate_pointer" }));
        };
        let op = FxOp::SetParam {
            clip: clip.clone(),
            param: handle.param().to_string(),
            value: value.to_json(),
        };
        let update = self.fx_gesture_update(&drag.gesture, vec![op])?;
        if let Some(open) = self.scene_interaction.drag.as_mut() {
            open.last = Some(Applied::Fx {
                handle: *handle,
                value,
            });
        }
        Ok(json!({
            "applied": true, "param": handle.param(), "value": value.to_json(),
            "would_change": update["would_change"], "changes": update["changes"],
        }))
    }

    /// The overlay of the effect clip the session targets, or `None` when this scene is not the
    /// target's (the node overlay is then the one drawn). `shown` is what the handles are laid
    /// out on: the document, or the candidate of the drag in progress.
    pub(super) fn fx_overlay(
        &self,
        scene: &str,
        frame: i64,
        shown: &DirectedScene,
        dragging: Option<&Drag>,
    ) -> Option<Overlay> {
        let target = self
            .scene_interaction
            .fx_target
            .as_ref()
            .filter(|target| target.scene == scene)?;
        let view = FxHandlesView::of(shown, &target.clip, frame).ok()?;
        let active = dragging.and_then(|drag| match &drag.action {
            DragAction::Fx { handle, .. } => Some(*handle),
            _ => None,
        });
        let aspect = view.shape.map_or_else(
            || view.width as f64 / (view.height.max(1)) as f64,
            |shape| shape.aspect(),
        );
        Some(Overlay {
            tool: self.scene_interaction.tool,
            space: self.scene_interaction.space,
            entity: None,
            layout: None,
            hidden: Some(Hidden::FxTarget),
            outlines: Vec::new(),
            markers: Vec::new(),
            active: None,
            fx: Some(FxOverlay { view, active }),
            aspect,
            stamp: self.gizmo_stamp(scene, frame),
        })
    }
}

/// What a pointer at `probe.point` would do to `probe.handle` of `clip`, judged without applying.
fn handle_probe(
    scene: &DirectedScene,
    clip: &str,
    view: &FxHandlesView,
    probe: &HandleProbe,
) -> Value {
    let refuse = |reason: &str| {
        json!({
            "handle": probe.handle.as_str(), "point": probe.point.to_json(),
            "ok": false, "reason": reason,
        })
    };
    if let Some(reason) = view.disabled_reason(probe.handle) {
        return refuse(reason);
    }
    let Some(value) = view.pointer_value(probe.handle, probe.point) else {
        return refuse(view.reason.unwrap_or("degenerate_pointer"));
    };
    let op = FxOp::SetParam {
        clip: clip.to_string(),
        param: probe.handle.param().to_string(),
        value: value.to_json(),
    };
    match scene_edit::dry_run_fx(scene, std::slice::from_ref(&op)) {
        Ok(summary) => json!({
            "handle": probe.handle.as_str(), "point": probe.point.to_json(), "ok": true,
            "param": probe.handle.param(), "value": value.to_json(),
            "would_change": summary.modified, "changes": changes_json(&summary),
        }),
        Err(error) => {
            let error = CommandError::from(error);
            json!({
                "handle": probe.handle.as_str(), "point": probe.point.to_json(), "ok": false,
                "param": probe.handle.param(), "value": value.to_json(),
                "reason": error.code, "message": error.message,
            })
        }
    }
}
