//! BL081.1.1 (F017): the keyboard entrance of "cut at the playhead" — Cmd+B on macOS, Ctrl+B
//! elsewhere.
//!
//! The shortcut is **presentation only**. It decides whether a key press is for the cut and then
//! hands the very same [`Command::SplitClip`] the toolbar button builds to
//! [`Workstation::dispatch`] (`Workstation::execute`, like a click, `ui invoke timeline.split` and
//! the `clip.split` op). Every cutting rule (selection, playhead inside the clip, ID policy,
//! envelope, one undo entry) stays in `tessel_editor_core::state::split`; this file keeps none.
//!
//! ## Why a focused key listener and not a keymap binding
//!
//! GPUI dispatches a matching `KeyBinding` as an *action before* any key listener runs, and an
//! action carries no `KeyDownEvent::is_held`, so a held key could not be told from a fresh press
//! (it would cut again at every repeat, and again when playback moves the playhead inside the
//! clip). A listener on the `Workstation` root sees the event, so repeats are swallowed there.
//! The keymap was inspected for collisions: no `cmd-b`/`ctrl-b` exists in the Kit (Input binds
//! a/c/x/v/z/y/f/h, Root binds c/tab) nor in the app (`ctrl-shift-d`, `ctrl-shift-i`); a test
//! pins that. No global binding is registered, so there is nothing to register twice and
//! `main.rs` needs no init.
//!
//! ## Focus scope
//!
//! The key counts only while the keyboard focus is **inside the `Workstation` element** (the
//! `Workstation` key context: library, monitor chrome, timeline, inspector). Not covered, on
//! purpose and not faked:
//!
//! - the header (a sibling view) and anything outside the editor window;
//! - the native WebView, which paints above GPUI and may never forward a key to it (BL044) — the
//!   physical key over the picture is unproven;
//! - Kit fields: while the focus is in an `Input`, number/time field, `Select`, `Combobox`,
//!   popover, dialog or sheet the key is left alone, so typing is never a cut and the field keeps
//!   its own editing shortcuts. A dialog's state (`Workflow::has_dialog`) blocks it as well.
//!
//! Headless has no keyboard: the shortcut is N/A there; the edit is `clip.split`.

use gpui_kit::*;
use tessel_editor_core::state::split::SplitAvailability;

use super::Workstation;
use crate::state::Command;
use tessel_editor_core::Id;

/// The platforms whose shortcut differs. One value per running build; parametrised in tests so
/// the macOS policy is checked without claiming a Mac run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Platform {
    Mac,
    Other,
}

impl Platform {
    pub(super) const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Other
        }
    }
}

/// The one definition of the shortcut: the keystroke the listener matches and the text people
/// read (tooltip, `timeline.split` node) come from the same row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Shortcut {
    /// GPUI keystroke syntax (`Keystroke::parse`).
    pub keystroke: &'static str,
    pub label: &'static str,
}

pub(super) const fn shortcut(platform: Platform) -> Shortcut {
    match platform {
        Platform::Mac => Shortcut {
            keystroke: "cmd-b",
            label: "⌘B",
        },
        Platform::Other => Shortcut {
            keystroke: "ctrl-b",
            label: "Ctrl+B",
        },
    }
}

pub(super) const fn current_shortcut() -> Shortcut {
    shortcut(Platform::current())
}

/// The `Workstation` key context the scope is defined by (see `Render for Workstation`).
pub(super) const SCOPE: &str = "Workstation";

/// Kit key contexts that mean "the user is editing or choosing in a field". The shortcut is not
/// delivered while any of them is on the focus path.
const FIELD_CONTEXTS: [&str; 10] = [
    "Input",
    "NumberInput",
    "TimeField",
    "SearchPanel",
    "Select",
    "Combobox",
    "Popover",
    "PopoverContent",
    "Dialog",
    "Sheet",
];

/// Whether the focus path (`Window::context_stack`) is inside a Kit field.
pub(super) fn focus_is_in_field(stack: &[KeyContext]) -> bool {
    stack
        .iter()
        .any(|context| FIELD_CONTEXTS.iter().any(|name| context.contains(name)))
}

/// `base` followed by the shortcut, as the tooltip reads: `Cortar clipe no playhead (Ctrl+B)`.
pub(super) fn with_shortcut(base: &str, platform: Platform) -> String {
    format!("{base} ({})", shortcut(platform).label)
}

/// The command the button and the shortcut both run: the selected clip at the playhead frame the
/// core reported as cuttable. No ID is chosen here (the core's deterministic policy picks it).
pub(super) fn split_command(clip: impl ToString, frame: i64) -> Command {
    Command::SplitClip {
        clip: clip.to_string(),
        frame,
        new_id: None,
    }
}

/// Exactly the platform's keystroke: the key and every modifier must match, so Shift/Alt
/// variants, AltGr (Ctrl+Alt) and another platform's modifier never count.
pub(super) fn is_split_keystroke(platform: Platform, keystroke: &Keystroke) -> bool {
    let Ok(wanted) = Keystroke::parse(shortcut(platform).keystroke) else {
        return false;
    };
    keystroke.key == wanted.key && keystroke.modifiers == wanted.modifiers
}

/// What the listener does with one key-down.
#[derive(Debug, PartialEq)]
pub(super) enum Verdict {
    /// Not ours, or the cut is not offered: the event keeps travelling and nothing changes.
    Ignore,
    /// A held key: consumed, never a second cut.
    SwallowRepeat,
    /// Run [`split_command`] for this clip and frame through the shared handler.
    Cut { clip: Id, frame: i64 },
}

/// The whole policy, with no window: the key, whether a Kit field has the focus, whether a
/// dialog is open, and the core's answer for the state **at the time of the key**.
pub(super) fn decide(
    platform: Platform,
    event: &KeyDownEvent,
    in_field: bool,
    dialog: bool,
    availability: SplitAvailability,
) -> Verdict {
    if !is_split_keystroke(platform, &event.keystroke)
        || event.prefer_character_input
        || in_field
        || dialog
    {
        return Verdict::Ignore;
    }
    if event.is_held {
        return Verdict::SwallowRepeat;
    }
    match availability {
        SplitAvailability::Ready { clip, frame } => Verdict::Cut { clip, frame },
        SplitAvailability::Blocked(_) => Verdict::Ignore,
    }
}

impl Workstation {
    /// The `Workstation` root's key-down: selection and playhead are read **now**, from the live
    /// state, never from what the last render captured.
    pub(super) fn on_split_shortcut(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Most keys are unrelated: avoid allocating the focus path or planning a split for them.
        if !is_split_keystroke(Platform::current(), &event.keystroke) {
            return;
        }
        let verdict = decide(
            Platform::current(),
            event,
            focus_is_in_field(&window.context_stack()),
            self.workflow.has_dialog(),
            self.state.split_availability(),
        );
        match verdict {
            Verdict::Ignore => {}
            Verdict::SwallowRepeat => cx.stop_propagation(),
            Verdict::Cut { clip, frame } => {
                cx.stop_propagation();
                self.dispatch(split_command(clip, frame), cx);
            }
        }
    }

    /// Live-state metadata of the `timeline.split` node: the binding the window really listens
    /// for (same row as the tooltip) and where it works. No parallel tree.
    pub(super) fn adopt_split_shortcut(&self, node: &mut serde_json::Value) {
        if node["id"] != "timeline.split" {
            return;
        }
        let shortcut = current_shortcut();
        node["shortcut"] = serde_json::json!({
            "keystroke": shortcut.keystroke,
            "label": shortcut.label,
            "scope": SCOPE,
            "repeat": "ignored",
            "blocked_in": FIELD_CONTEXTS,
        });
    }
}
