//! Pointer and keyboard adapter that turns ONE slider interaction into ONE undoable edit
//! (BL062; F005 and F024).
//!
//! The history rules live in the core (`state::property_gesture`): this file only translates
//! the Kit slider's input into its four phases and keeps no history of its own.
//!
//! The Kit 0.9.0 `SliderState` emits `Change(value)` continuously while the track is pressed
//! or a thumb is dragged, and `Release(value)` once, on the mouse-up inside or outside the
//! slider, but only after a real change (a thumb pressed and released without moving emits
//! nothing). There is no "start" event and no cancel, and the slider has no keyboard handling
//! of its own (the inspector wrapper steps it with the arrow keys). So:
//!
//! - **Begin**: the first `Change` after the previous interaction ended (or a fresh key
//!   press) begins a gesture on the selected clip. A press on the slider is also seen in the
//!   capture phase (the thumb swallows it later) to end any interaction whose release never
//!   arrived (BL043: a lost MouseUp).
//! - **Update**: every later `Change` and every key repeat updates the gesture.
//! - **Commit**: `Release`, or the key-up of a step key. The release is explicit: no timer.
//! - **Cancel**: Escape. The adapter then ignores the rest of that press (`Ended`) so the
//!   pointer that is still down cannot start a second gesture, and the next press clears it.
//! - **Focus loss** (window deactivation) commits, for the same reason.
//!
//! The adapter never applies a value to a clip it did not begin on: the core locks the clip
//! at `Begin`, and values after the core ended the gesture (another edit, a project swap) are
//! dropped until the next press.
//!
//! Known limit: the Kit's accessibility Increment/Decrement actions call `set_value` without
//! emitting an event, so they never reach the model (and the next render re-syncs the slider
//! from it). Tracked as BL070 in `docs/slider-gesture-history-report.md`.

use gpui_kit::component::slider::SliderEvent;
use gpui_kit::*;

use super::{Source, Workstation};
use crate::state::property_gesture::GestureCommand;
use crate::state::{Command, Raw};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// The core has an open gesture with this ID; values go to it.
    Open(u64),
    /// The gesture was ended (Escape, a rejected begin, a stale target, focus loss) while the
    /// pointer or key is still down: ignore everything until the release or the next press.
    Ended,
}

/// How an open gesture ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    Commit,
    Cancel,
}

/// Which slider interaction is in progress, if any.
#[derive(Debug, Default)]
pub struct SliderGesture {
    tracked: Option<(&'static str, Phase)>,
}

impl SliderGesture {
    #[cfg(test)]
    pub fn is_tracking(&self) -> bool {
        self.tracked.is_some()
    }
}

impl Workstation {
    /// Routes one event of the slider of `property` to the gesture it belongs to.
    pub(super) fn slider_event(
        &mut self,
        property: &'static str,
        event: &SliderEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            SliderEvent::Change(value) => self.slider_changed(property, f64::from(value.end()), cx),
            SliderEvent::Release(_) => self.slider_released(property, cx),
        }
    }

    /// A new press (pointer down on a slider, or a key press that is not auto-repeat). Whatever
    /// is still tracked lost its release: it ends as released, so the press starts clean.
    pub(super) fn slider_pressed(&mut self, cx: &mut Context<Self>) {
        self.settle_slider_gesture(cx);
    }

    /// A value from the pointer or the keyboard for the slider of `property`.
    pub(super) fn slider_changed(
        &mut self,
        property: &'static str,
        value: f64,
        cx: &mut Context<Self>,
    ) {
        if self
            .slider_gesture
            .tracked
            .is_some_and(|(tracked, _)| tracked != property)
        {
            // Another slider took over (focus moved while a key was held).
            self.settle_slider_gesture(cx);
        }
        let id = match self.slider_gesture.tracked {
            Some((_, Phase::Ended)) => return,
            Some((_, Phase::Open(id))) => id,
            None => {
                let Some(id) = self.begin_slider_gesture(property, cx) else {
                    return;
                };
                id
            }
        };
        let update = Command::PropertyGesture(GestureCommand::Update {
            gesture: Some(id),
            value: Raw::Number(value),
        });
        // A rejected value (`invalid_value`) leaves the gesture open: the status bar shows the
        // error and the next value may be fine. Only a gesture the core ended stops the drag.
        if let Err(error) = self.execute(update, Source::Gesture, cx)
            && matches!(error.code, "stale_gesture" | "no_gesture")
        {
            self.slider_gesture.tracked = Some((property, Phase::Ended));
        }
    }

    /// The pointer was released or the step key came up: the gesture is committed.
    pub(super) fn slider_released(&mut self, property: &'static str, cx: &mut Context<Self>) {
        let Some((tracked, phase)) = self.slider_gesture.tracked else {
            return;
        };
        if tracked != property {
            return;
        }
        self.slider_gesture.tracked = None;
        self.finish_slider_gesture(phase, End::Commit, cx);
    }

    /// Escape: the gesture is cancelled and the rest of the press is ignored.
    pub(super) fn cancel_slider_gesture(&mut self, cx: &mut Context<Self>) {
        let Some((property, phase)) = self.slider_gesture.tracked else {
            return;
        };
        self.slider_gesture.tracked = Some((property, Phase::Ended));
        self.finish_slider_gesture(phase, End::Cancel, cx);
    }

    /// The window lost the focus: a release may never arrive, so the gesture is committed and
    /// the rest of the press is ignored.
    pub(super) fn slider_focus_lost(&mut self, cx: &mut Context<Self>) {
        let Some((property, phase)) = self.slider_gesture.tracked else {
            return;
        };
        self.slider_gesture.tracked = Some((property, Phase::Ended));
        self.finish_slider_gesture(phase, End::Commit, cx);
    }

    fn settle_slider_gesture(&mut self, cx: &mut Context<Self>) {
        let Some((_, phase)) = self.slider_gesture.tracked.take() else {
            return;
        };
        self.finish_slider_gesture(phase, End::Commit, cx);
    }

    fn begin_slider_gesture(
        &mut self,
        property: &'static str,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let begin = Command::PropertyGesture(GestureCommand::Begin {
            // The selected clip: the slider shows the selection's value.
            clip: None,
            property: property.to_string(),
        });
        let id = self
            .execute(begin, Source::Gesture, cx)
            .ok()
            .and_then(|outcome| outcome["gesture"].as_u64());
        self.slider_gesture.tracked = Some((property, id.map_or(Phase::Ended, Phase::Open)));
        id
    }

    /// Sends `end` (commit or cancel) for the gesture of `phase`, unless the core already ended
    /// it: an edit that settled it, or a project swap, leaves nothing to report.
    fn finish_slider_gesture(&mut self, phase: Phase, end: End, cx: &mut Context<Self>) {
        let Phase::Open(id) = phase else {
            return;
        };
        if self
            .state
            .open_property_gesture()
            .is_none_or(|open| open.id != id)
        {
            return;
        }
        let gesture = Some(id);
        let end = match end {
            End::Commit => GestureCommand::Commit { gesture },
            End::Cancel => GestureCommand::Cancel { gesture },
        };
        self.dispatch(Command::PropertyGesture(end), cx);
    }
}

/// A capture-phase press listener for a slider's wrapper. The Kit thumb stops propagation of
/// its own mouse-down, so the bubble phase would never see a press that lands on the thumb.
pub fn slider_press(
    cx: &mut Context<Workstation>,
) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
    cx.listener(|this, event: &MouseDownEvent, window, cx| {
        if event.button == MouseButton::Left {
            // A new press cannot continue a framework drag whose MouseUp was lost.
            cx.stop_active_drag(window);
            this.slider_pressed(cx);
        }
    })
}

/// A key handler for the slider of `property`: `next_value` turns a key (and the shift state)
/// into the value to preview, or `None` for keys the slider ignores. Holding a key keeps one
/// gesture open (auto-repeat continues it); a new press starts its own.
pub fn slider_keys(
    cx: &mut Context<Workstation>,
    property: &'static str,
    enabled: bool,
    next_value: impl Fn(&str, bool) -> Option<f64> + 'static,
) -> impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |this, event: &KeyDownEvent, _, cx| {
        if !enabled {
            return;
        }
        let Some(value) = next_value(
            event.keystroke.key.as_str(),
            event.keystroke.modifiers.shift,
        ) else {
            return;
        };
        cx.stop_propagation();
        if !event.is_held {
            this.slider_pressed(cx);
        }
        this.slider_changed(property, value, cx);
    })
}

/// The key-up that commits a keyboard gesture. Only step keys count: releasing Shift while an
/// arrow is still held must not end the gesture.
pub fn slider_key_up(
    cx: &mut Context<Workstation>,
    property: &'static str,
    is_step_key: impl Fn(&str) -> bool + 'static,
) -> impl Fn(&KeyUpEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |this, event: &KeyUpEvent, _, cx| {
        if is_step_key(event.keystroke.key.as_str()) {
            this.slider_released(property, cx);
        }
    })
}
