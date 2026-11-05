//! Pointer and keyboard adapter that turns ONE scene-slider interaction into ONE scene gesture
//! and so ONE undo entry (BL014; F039 `scene.gesture.*`).
//!
//! It is the same contract as `slider_gesture.rs` (clip properties, BL062), applied to the
//! `scene.gesture.begin|update|commit|cancel` commands. The rules (candidate separate from the
//! document, one `Edit scene` entry on commit, typed invalidation) live in the core; this file
//! only translates the Kit slider's four phases and keeps no history of its own:
//!
//! - **Begin**: the first `Change` after the previous interaction (or a fresh key press).
//! - **Update**: every later `Change` and key repeat; each one is an absolute `set` of the one
//!   property, so the candidate always restarts from the document the gesture began on.
//! - **Commit**: `Release`, the key-up of a step key, or the window losing focus.
//! - **Cancel**: Escape. The rest of that press is ignored (`Ended`), so a pointer still down
//!   cannot start a second gesture.
//!
//! The gesture begins on the entity of the slider, never on the current selection: changing the
//! selection mid-drag cannot redirect the edit (the core never reads the selection).
//!
//! Known limit, shared with `slider_gesture.rs`: the Kit's accessibility Increment/Decrement
//! call `set_value` without emitting an event, so they never reach the model (BL074).

use gpui_kit::component::slider::SliderEvent;
use gpui_kit::*;
use tessel_editor_core::scene_edit::{EditOp, PropertyRef};
use tessel_editor_core::scene_interaction::GestureCommand;
use tessel_editor_core::scene_project::SceneCommand;

use super::scene_model::with_component;
use super::{Source, Workstation};
use crate::state::{Command, CommandError};

/// Which slider of which property of which entity. `index` is the component (X/Y/Z of a vector,
/// the Euler angles of a rotation, 0 of a number).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SliderKey {
    /// ID of the attached scene.
    pub scene: String,
    /// Address of the entity the gesture is begun on (`scene/<definition>/node/<node>`).
    pub entity: String,
    /// Catalog property path (`<entity>/position`).
    pub path: String,
    pub index: usize,
}

impl SliderKey {
    /// The geometry-probe and element ID, also what tests use to find the slider.
    pub fn element_id(&self) -> String {
        format!("scene:{}:{}", self.path, self.index)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// The core has an open gesture with this ID; values go to it.
    Open(String),
    /// The gesture ended (Escape, a rejected begin, a stale target, focus loss) while the pointer
    /// or key is still down: ignore everything until the release or the next press.
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    Commit,
    Cancel,
}

/// The slider interaction in progress, if any. Input state of the adapter only.
#[derive(Debug, Default)]
pub struct SceneSliderGesture {
    tracked: Option<(SliderKey, Phase)>,
}

impl SceneSliderGesture {
    pub fn is_tracked(&self, key: &SliderKey) -> bool {
        self.tracked
            .as_ref()
            .is_some_and(|(tracked, _)| tracked == key)
    }

    #[cfg(test)]
    pub fn is_tracking(&self) -> bool {
        self.tracked.is_some()
    }

    pub fn clear(&mut self) {
        self.tracked = None;
    }
}

/// Errors that mean the core no longer holds the gesture the drag was feeding.
fn gesture_is_gone(error: &CommandError) -> bool {
    matches!(
        error.code,
        "gesture_invalidated" | "no_gesture" | "unknown_gesture"
    )
}

impl Workstation {
    pub(super) fn scene_slider_event(
        &mut self,
        key: SliderKey,
        event: &SliderEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            SliderEvent::Change(value) => {
                self.scene_slider_changed(&key, f64::from(value.end()), cx)
            }
            SliderEvent::Release(_) => self.scene_slider_released(&key, cx),
        }
    }

    /// A new press: whatever is still tracked lost its release and ends as released.
    pub(super) fn scene_slider_pressed(&mut self, cx: &mut Context<Self>) {
        self.settle_scene_gesture(cx);
    }

    pub(super) fn scene_slider_changed(
        &mut self,
        key: &SliderKey,
        value: f64,
        cx: &mut Context<Self>,
    ) {
        if self
            .scene_ui
            .gesture
            .tracked
            .as_ref()
            .is_some_and(|(tracked, _)| tracked != key)
        {
            // Another slider took over (focus moved while a key was held).
            self.settle_scene_gesture(cx);
        }
        let phase = self
            .scene_ui
            .gesture
            .tracked
            .as_ref()
            .map(|(_, phase)| phase.clone());
        let id = match phase {
            Some(Phase::Ended) => return,
            Some(Phase::Open(id)) => id,
            None => {
                let Some(id) = self.begin_scene_gesture(key, cx) else {
                    return;
                };
                id
            }
        };
        let update = match self.scene_edit_op(key, value) {
            Ok(op) => SceneCommand::Gesture(GestureCommand::Update {
                gesture: id,
                ops: vec![op],
            }),
            Err(error) => {
                self.scene_report(error, cx);
                return;
            }
        };
        // A rejected value (`invalid_value`) leaves the gesture open: the panel shows the typed
        // error and the next value may be fine. Only a gesture the core dropped stops the drag.
        if let Err(error) = self.scene_execute(update, Source::Gesture, cx)
            && gesture_is_gone(&error)
        {
            self.scene_ui.gesture.tracked = Some((key.clone(), Phase::Ended));
        }
    }

    /// The pointer was released or the step key came up: the gesture is committed.
    pub(super) fn scene_slider_released(&mut self, key: &SliderKey, cx: &mut Context<Self>) {
        let Some((tracked, phase)) = self.scene_ui.gesture.tracked.clone() else {
            return;
        };
        if &tracked != key {
            return;
        }
        self.scene_ui.gesture.tracked = None;
        self.finish_scene_gesture(phase, End::Commit, cx);
    }

    /// Escape: the gesture is cancelled and the rest of the press is ignored.
    pub(super) fn cancel_scene_gesture(&mut self, cx: &mut Context<Self>) {
        let Some((key, phase)) = self.scene_ui.gesture.tracked.clone() else {
            return;
        };
        self.scene_ui.gesture.tracked = Some((key, Phase::Ended));
        self.finish_scene_gesture(phase, End::Cancel, cx);
    }

    /// The window lost focus: a release may never arrive (BL043), so the gesture is committed.
    pub(super) fn scene_slider_focus_lost(&mut self, cx: &mut Context<Self>) {
        let Some((key, phase)) = self.scene_ui.gesture.tracked.clone() else {
            return;
        };
        self.scene_ui.gesture.tracked = Some((key, Phase::Ended));
        self.finish_scene_gesture(phase, End::Commit, cx);
    }

    fn settle_scene_gesture(&mut self, cx: &mut Context<Self>) {
        let Some((_, phase)) = self.scene_ui.gesture.tracked.take() else {
            return;
        };
        self.finish_scene_gesture(phase, End::Commit, cx);
    }

    fn begin_scene_gesture(&mut self, key: &SliderKey, cx: &mut Context<Self>) -> Option<String> {
        let id = self.scene_gesture_begin(key, cx).ok();
        self.scene_ui.gesture.tracked =
            Some((key.clone(), id.clone().map_or(Phase::Ended, Phase::Open)));
        id
    }

    /// `scene.gesture.begin` on the slider's entity. Shared by the pointer, the keyboard and the
    /// semantic `set` of a property node.
    pub(super) fn scene_gesture_begin(
        &mut self,
        key: &SliderKey,
        cx: &mut Context<Self>,
    ) -> Result<String, CommandError> {
        let begin = SceneCommand::Gesture(GestureCommand::Begin {
            scene: key.scene.clone(),
            targets: vec![key.entity.clone()],
            label: None,
        });
        let outcome = self.scene_execute(begin, Source::Gesture, cx)?;
        outcome["gesture"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| CommandError::new("internal", "scene.gesture.begin returned no ID"))
    }

    /// The `set` op that moves component `key.index` of the property to `value`. The other
    /// components keep what the gesture shows now, so one slider never rewrites its siblings.
    pub(super) fn scene_edit_op(
        &self,
        key: &SliderKey,
        value: f64,
    ) -> Result<EditOp, CommandError> {
        let reference = PropertyRef::parse(&key.path)?;
        let current = self.scene_property_now(key).ok_or_else(|| {
            CommandError::new(
                "unknown_path",
                format!("`{}` has no value in scene `{}`", key.path, key.scene),
            )
            .target(key.path.clone())
        })?;
        let next = with_component(&current, key.index, value).ok_or_else(|| {
            CommandError::new(
                "invalid_value",
                format!("`{}` has no component {}", key.path, key.index),
            )
            .target(key.path.clone())
        })?;
        Ok(EditOp::set(reference, next))
    }

    /// Sends `end` for the gesture of `phase`, unless the core no longer holds it (another edit
    /// invalidated it, the project was swapped): there is then nothing to report.
    fn finish_scene_gesture(&mut self, phase: Phase, end: End, cx: &mut Context<Self>) {
        let Phase::Open(id) = phase else {
            return;
        };
        if self
            .state
            .scene_interaction
            .gesture()
            .is_none_or(|open| open.id() != id)
        {
            return;
        }
        let gesture = id.clone();
        match end {
            End::Cancel => {
                let _ = self.scene_execute(
                    SceneCommand::Gesture(GestureCommand::Cancel { gesture }),
                    Source::Gesture,
                    cx,
                );
            }
            End::Commit => {
                let commit = SceneCommand::Gesture(GestureCommand::Commit { gesture });
                if self.scene_execute(commit, Source::Gesture, cx).is_err() {
                    // A rejected commit gives the gesture back to the core (so a caller could
                    // retry). A drag has nothing to retry with: drop the candidate so the panel
                    // shows the document again, and keep the typed error on screen.
                    let _ = self.state.execute(Command::Scene(SceneCommand::Gesture(
                        GestureCommand::Cancel { gesture: id },
                    )));
                    cx.notify();
                }
            }
        }
    }
}

/// A capture-phase press listener for a slider's wrapper. The Kit thumb stops propagation of its
/// own mouse-down, so the bubble phase would never see a press that lands on the thumb.
pub fn scene_slider_press(
    cx: &mut Context<Workstation>,
) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
    cx.listener(|this, event: &MouseDownEvent, window, cx| {
        if event.button == MouseButton::Left {
            // A new press cannot continue a framework drag whose MouseUp was lost.
            cx.stop_active_drag(window);
            this.scene_slider_pressed(cx);
        }
    })
}

/// A key handler for one scene slider: `next_value` turns a key (and the shift state) into the
/// value to preview, or `None` for keys the slider ignores. Holding a key keeps one gesture
/// open (auto-repeat continues it); a new press starts its own.
pub fn scene_slider_keys(
    cx: &mut Context<Workstation>,
    key: SliderKey,
    next_value: impl Fn(&str, bool) -> Option<f64> + 'static,
) -> impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |this, event: &KeyDownEvent, _, cx| {
        let Some(value) = next_value(
            event.keystroke.key.as_str(),
            event.keystroke.modifiers.shift,
        ) else {
            return;
        };
        cx.stop_propagation();
        if !event.is_held {
            this.scene_slider_pressed(cx);
        }
        this.scene_slider_changed(&key, value, cx);
    })
}

/// The key-up that commits a keyboard gesture. Only step keys count: releasing Shift while an
/// arrow is still held must not end the gesture.
pub fn scene_slider_key_up(
    cx: &mut Context<Workstation>,
    key: SliderKey,
    is_step_key: impl Fn(&str) -> bool + 'static,
) -> impl Fn(&KeyUpEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |this, event: &KeyUpEvent, _, cx| {
        if is_step_key(event.keystroke.key.as_str()) {
            this.scene_slider_released(&key, cx);
        }
    })
}
