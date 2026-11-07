//! Desktop adapter of the translation gizmo (F052, BL015/BL042).
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

    fn overlay_pointer(&mut self, message: &Value, cx: &mut gpui_kit::Context<Self>) {
        let Some(scene) = self.viewport.directed_scene().map(str::to_owned) else {
            return;
        };
        let phase = message["phase"].as_str().unwrap_or_default();
        // A cancel is always honoured (it only discards). Anything else must answer the overlay
        // the core would draw now: the page that sent it may be the one before a reload, the
        // project before a replacement, or the frame before a seek. The core produced the stamp;
        // this only compares.
        if phase != "cancel" {
            let stamp = self.state.gizmo_stamp(&scene, self.state.timeline.playhead);
            if message["stamp"] != stamp {
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
    fn overlay_run(&mut self, pointer: PointerCommand, cx: &mut gpui_kit::Context<Self>) {
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

    /// Keeps the page's overlay in step with the core: on for a directed scene that is on screen,
    /// off otherwise, and redrawn whenever the core's answer changed. Called with every viewport
    /// sync, which every command already triggers.
    pub(super) fn overlay_refresh(&mut self) {
        let scene = self.viewport.directed_scene().map(str::to_owned);
        let want = scene.is_some() && self.viewport.on_screen;
        self.viewport.overlay_set_enabled(want);
        let (true, Some(scene)) = (want, scene) else {
            return;
        };
        let frame = self.state.timeline.playhead;
        let stamp = self.state.gizmo_stamp(&scene, frame);
        let request = Command::Scene(SceneCommand::Gizmo(GizmoCommand::State {
            scene,
            at: FrameAt::Frame(frame),
        }));
        // A read: no history label, no revision. A scene without a usable frame at the playhead
        // (a plan error) simply has no overlay; the monitor reports that failure itself.
        match self.state.execute(request) {
            Ok(answer) => {
                self.viewport.overlay_show(&answer["overlay"]);
            }
            Err(_) => {
                self.viewport.overlay_show(&json!({
                    "version": 1, "visible": false, "reason": "no_frame", "entity": null,
                    "origin": null, "handles": [], "outlines": [], "active": null, "aspect": 1.0,
                    "stamp": stamp,
                }));
            }
        }
    }
}
