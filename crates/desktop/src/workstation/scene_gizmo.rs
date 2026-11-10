//! Desktop adapter of the translation and rotation gizmo (F055, BL015/BL042/BL015.1).
//!
//! The gizmo is drawn and hit on a canvas INSIDE the monitor's WebView (`runtime/src/overlay.js`),
//! never as a GPUI element over it (a native WebView paints above every GPUI element). This file
//! is the other half of that seam, and it is thin on purpose:
//!
//! * the page sends `overlay.pointer` (picture coordinates, one message per press/move/release/
//!   cancel); each becomes ONE `scene.pointer.*` command and goes through the same
//!   `scene_execute` every other gesture uses, so the core decides everything (hit, handle,
//!   position, undo entry) and a script can send the same commands;
//! * after any command, [`Workstation::overlay_refresh`] asks the core for `scene.gizmo.state` and
//!   sends the result to the page when it changed, so a selection made in the tree, a value typed
//!   in the inspector, an undo and a drag all redraw the same way;
//! * Escape, the window losing focus and the monitor being hidden cancel an open drag here too
//!   (the page covers the cases it can see itself: its own Escape, blur, `pointercancel`).
//!
//! Nothing here knows what a handle is or converts a coordinate; there is no state of its own.

use serde_json::{Value, json};
use tessel_editor_core::scene_interaction::{
    CancelReason, FrameAt, GizmoCommand, Point, PointerCommand,
};
use tessel_editor_core::scene_project::SceneCommand;

use super::{Source, Workstation};
use crate::state::{Command, CommandError};

/// Errors that only mean "that pointer event arrived for a drag that is not there any more":
/// the page raced an Escape, a cancel or an undo. Not worth a notice.
fn is_stale(error: &CommandError) -> bool {
    matches!(error.code, "no_drag" | "no_gesture" | "unknown_gesture")
}

fn point_of(message: &Value) -> Option<Point> {
    Some(Point {
        u: message["point"]["u"].as_f64()?,
        v: message["point"]["v"].as_f64()?,
    })
}

impl Workstation {
    /// Handles the `overlay.*` messages of the page. `true` if `message` was one (consumed).
    pub(super) fn overlay_ipc(
        &mut self,
        message: &Value,
        cx: &mut gpui_kit::Context<Self>,
    ) -> bool {
        if !self.viewport.overlay_note(message) {
            return false;
        }
        if message["type"] == "overlay.pointer" {
            self.overlay_pointer(message, cx);
        }
        true
    }

    /// The identity of the overlay the page must hold for `scene` at `frame`: the core's stamp
    /// (scene, session, project revision, frame) plus two things only this side knows, the
    /// incarnation of the page (a reload is a new one: the old page's queued messages never
    /// equal it) and the selection the handles belong to (a message made against the overlay of
    /// the previous selection is not the current one). The page only echoes it; the adapter only
    /// compares it (M4 of the PR #25 review).
    pub(super) fn overlay_stamp(&self, scene: &str, frame: i64) -> Value {
        let mut stamp = self.state.gizmo_stamp(scene, frame);
        stamp["epoch"] = json!(self.viewport.overlay_epoch());
        stamp["selection"] = json!(self.state.gizmo_selection(scene));
        stamp
    }

    fn overlay_pointer(&mut self, message: &Value, cx: &mut gpui_kit::Context<Self>) {
        let Some(scene) = self.viewport.directed_scene().map(str::to_owned) else {
            return;
        };
        let phase = message["phase"].as_str().unwrap_or_default();
        // A move or release for a drag the core no longer holds (Escape raced it, or the press
        // only selected a plane): nothing to do, no notice and nothing counted.
        if matches!(phase, "move" | "up") && self.state.scene_interaction.drag().is_none() {
            return;
        }
        // A cancel is always honoured (it only discards). Anything else must come from a page
        // that negotiated the overlay and holds it on, and must answer the overlay the core would
        // draw now: the page that sent it may be the one before a reload, the project before a
        // replacement, the frame before a seek or the selection before a click in the tree. The
        // core produced the stamp; this only compares.
        if phase != "cancel" {
            let negotiated = self.viewport.overlay_supported() && self.viewport.overlay_enabled();
            let stamp = self.overlay_stamp(&scene, self.state.timeline.playhead);
            if !negotiated || message["stamp"] != stamp {
                self.viewport.overlay_note_stale();
                // The page believes in a drag the overlay no longer describes: end it, so a
                // release that will be refused too cannot leave it open.
                if phase != "down" {
                    self.cancel_gizmo_drag(CancelReason::Api, cx);
                }
                return;
            }
        }
        let pointer = match (phase, point_of(message)) {
            ("down", Some(point)) => PointerCommand::Down {
                scene,
                at: FrameAt::Frame(self.state.timeline.playhead),
                point,
                additive: message["additive"].as_bool().unwrap_or(false),
            },
            // A move or release for a drag the core no longer holds (Escape raced it): nothing
            // to do, and no notice for a thing the user did not do wrong.
            ("move", Some(point)) if self.state.scene_interaction.drag().is_some() => {
                PointerCommand::Move { point }
            }
            ("up", point) if self.state.scene_interaction.drag().is_some() => {
                PointerCommand::Up { point }
            }
            ("cancel", _) => PointerCommand::Cancel {
                reason: message["reason"]
                    .as_str()
                    .and_then(CancelReason::parse)
                    .unwrap_or(CancelReason::Api),
            },
            _ => return,
        };
        self.overlay_run(pointer, cx);
    }

    /// Runs one pointer command through the shared handler and settles what a failure leaves.
    pub(super) fn overlay_run(
        &mut self,
        pointer: PointerCommand,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        let command = SceneCommand::Gizmo(GizmoCommand::Pointer(pointer));
        let Err(error) = self.scene_execute(command, Source::Gesture, cx) else {
            return;
        };
        if is_stale(&error) {
            return;
        }
        if error.code == "gesture_invalidated" {
            // The scene moved under the drag: it can never apply. End it, so the gizmo stands
            // where the document says and the next press starts clean.
            let _ = self.scene_execute(
                SceneCommand::Gizmo(GizmoCommand::Pointer(PointerCommand::Cancel {
                    reason: CancelReason::Api,
                })),
                Source::Gesture,
                cx,
            );
        }
        self.scene_report(error, cx);
    }

    /// Cancels the open gizmo drag, if any (Escape, focus lost, monitor hidden).
    pub(super) fn cancel_gizmo_drag(
        &mut self,
        reason: CancelReason,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        if self.state.scene_interaction.drag().is_none() {
            return;
        }
        self.overlay_run(PointerCommand::Cancel { reason }, cx);
    }

    /// Ends a drag nobody can finish any more: the page is gone or no longer shows the scene, the
    /// monitor is hidden, the playhead moved away from the frame it was pressed at, or the scene
    /// was edited, replaced or detached under it (an undo, an import that replaced the scene).
    /// Idempotent and cheap while no drag is open; called after every sync, tick and page message.
    pub(super) fn settle_gizmo_drag(&mut self, cx: &mut gpui_kit::Context<Self>) {
        let Some(drag) = self.state.scene_interaction.drag() else {
            return;
        };
        let shown = self.viewport.directed_scene() == Some(drag.scene());
        let reason = if !self.viewport.on_screen {
            Some(CancelReason::Hidden)
        } else if !shown || !self.viewport.overlay_supported() {
            Some(CancelReason::Unavailable)
        } else if drag.frame() != self.state.timeline.playhead || self.state.gizmo_drag_is_dead() {
            Some(CancelReason::Api)
        } else {
            None
        };
        if let Some(reason) = reason {
            self.cancel_gizmo_drag(reason, cx);
        }
    }

    /// A command that already answered with the overlay of the state it left (`overlay` in the
    /// answer of `scene.pointer.*` and `scene.gizmo.state`) hands it to the page, so the sync that
    /// follows finds it current instead of asking the core to plan the scene a second time for
    /// the same state (M2 of the PR #25 review). Only when that overlay is the one the core would
    /// draw now (same stamp) and the page holds the overlay on.
    pub(super) fn overlay_adopt(&mut self, outcome: &Value) {
        let overlay = &outcome["overlay"];
        let Some(scene) = overlay["stamp"]["scene"].as_str() else {
            return;
        };
        if !self.viewport.overlay_enabled()
            || self.viewport.directed_scene() != Some(scene)
            || self.playback.running_frame().is_some()
        {
            return;
        }
        let frame = self.state.timeline.playhead;
        if overlay["stamp"] != self.state.gizmo_stamp(scene, frame) {
            return;
        }
        let key = self.state.gizmo_overlay_key(scene, frame);
        let mut overlay = overlay.clone();
        overlay["stamp"] = self.overlay_stamp(scene, frame);
        self.viewport.overlay_show(&overlay);
        self.viewport.overlay_adopted(key);
    }

    /// Keeps the page's overlay in step with the core: on for a directed scene that is on screen,
    /// off otherwise, and redrawn whenever the core's answer changed. Called with every viewport
    /// sync, which every command already triggers, so everything that costs a plan of the scene
    /// is behind a gate that costs none, in this order:
    ///
    /// 1. the page must have negotiated the overlay and hold it on (a runtime without
    ///    `overlay.gizmo`, a hidden monitor, no directed scene: nothing is asked of the core);
    /// 2. playback runs: the picture moves every frame, so the overlay is cleared once and nothing
    ///    is planned per tick (no gizmo drawn in a place the moving picture left);
    /// 3. the page already has the overlay of this very state (`gizmo_overlay_key`), or a command
    ///    just answered with it ([`overlay_adopt`](Self::overlay_adopt)): nothing is planned.
    ///
    /// `PlanTimeline` is not used here on purpose: it borrows the scene and cannot outlive an
    /// edit (its own contract), and an overlay is wanted once per state, not once per frame.
    pub(super) fn overlay_refresh(&mut self) {
        let scene = self.viewport.directed_scene().map(str::to_owned);
        let want = scene.is_some() && self.viewport.on_screen;
        self.viewport.overlay_set_enabled(want);
        let (true, Some(scene)) = (want && self.viewport.overlay_enabled(), scene) else {
            return;
        };
        if self.playback.running_frame().is_some() {
            self.viewport.overlay_hold();
            return;
        }
        let frame = self.state.timeline.playhead;
        let key = self.state.gizmo_overlay_key(&scene, frame);
        if self.viewport.overlay_memo_hit(&key) {
            return;
        }
        self.viewport.overlay_computed(key);
        let stamp = self.overlay_stamp(&scene, frame);
        let request = Command::Scene(SceneCommand::Gizmo(GizmoCommand::State {
            scene,
            at: FrameAt::Frame(frame),
        }));
        // A read: no history label, no revision. A scene without a usable frame at the playhead
        // (a plan error) simply has no overlay; the monitor reports that failure itself.
        match self.state.execute(request) {
            Ok(answer) => {
                let mut overlay = answer["overlay"].clone();
                overlay["stamp"] = stamp;
                self.viewport.overlay_show(&overlay);
            }
            Err(_) => {
                self.viewport.overlay_show(&json!({
                    "version": 1, "visible": false, "reason": "no_frame", "entity": null,
                    "tool": stamp["tool"], "space": stamp["space"],
                    "origin": null, "handles": [], "outlines": [], "active": null, "aspect": 1.0,
                    "stamp": stamp,
                }));
            }
        }
    }
}
