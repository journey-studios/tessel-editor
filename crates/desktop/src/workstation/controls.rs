//! Small helpers around Kit controls.
//!
//! The controls themselves (`Button`, `TabBar`, `Slider`, `Select`, `Tree`,
//! `Accordion`, `Input`) come from the Kit; nothing here re-implements one.
//! What lives here is the glue that is shared by the panels: turning a click
//! or a key into a [`Command`] that runs through the same handler the CLI
//! uses, and the property-row layout around a control.
//!
//! Every control is stateless with respect to the editor: each frame it is
//! built from `EditorState`, and every interaction becomes a `Command`. Kit
//! controls that keep a private copy of a value (`SliderState`, `SelectState`,
//! `InputState`, `TreeState`) are re-synchronised from the model, never the
//! other way round, so the CLI and the pointer cannot disagree.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::Workstation;
use super::tokens::Tokens;
use crate::state::Command;

/// A click that runs `command` through the shared handler.
pub fn click(
    cx: &mut Context<Workstation>,
    command: Command,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |this, _: &ClickEvent, _, cx| {
        this.dispatch(command.clone(), cx);
    })
}

/// A key handler: `map` turns a key (and shift state) into a command.
pub fn keys(
    cx: &mut Context<Workstation>,
    map: impl Fn(&str, bool) -> Option<Command> + 'static,
) -> impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |this, event: &KeyDownEvent, _, cx| {
        let Some(command) = map(
            event.keystroke.key.as_str(),
            event.keystroke.modifiers.shift,
        ) else {
            return;
        };
        cx.stop_propagation();
        this.dispatch(command, cx);
    })
}

/// An icon at a fixed pixel size and colour.
pub fn glyph(name: IconName, size: f32, color: Hsla) -> Icon {
    Icon::new(name).size(px(size)).text_color(color)
}

/// An icon-only toolbar button. The tooltip doubles as the accessible name, as
/// the Kit requires for icon buttons.
pub fn tool_button(id: impl Into<ElementId>, icon: IconName, label: &str) -> Button {
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(icon)
        .tooltip(label.to_string())
        .accessibility_label(label.to_string())
}

/// Like [`tool_button`], but inert; the tooltip says why.
pub fn disabled_tool_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: &str,
    reason: &str,
) -> Button {
    use gpui_kit::component::Disableable;
    tool_button(id, icon, &format!("{label} — {reason}")).disabled(true)
}

pub fn separator(t: &Tokens) -> Div {
    div()
        .w(px(1.))
        .h(px(16.))
        .mx(px(4.))
        .flex_shrink_0()
        .bg(t.border)
}

pub struct TabSpec {
    pub label: SharedString,
    pub active: bool,
    pub command: Command,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TabStyle {
    /// Equal-width tabs with an underline, as in the inspector.
    Underline,
    /// Compact pills, as in the workspace mode bar.
    Pill,
}

/// A Kit `TabBar` whose tabs each run a command. The bar is controlled: the
/// selected tab always comes from `spec.active`.
pub fn command_tabs(
    cx: &mut Context<Workstation>,
    id: &'static str,
    style: TabStyle,
    tabs: Vec<TabSpec>,
) -> TabBar {
    let selected = tabs.iter().position(|tab| tab.active).unwrap_or(0);
    let commands: Vec<Command> = tabs.iter().map(|tab| tab.command.clone()).collect();
    let bar = TabBar::new(id).selected_index(selected);
    let bar = match style {
        TabStyle::Underline => bar.underline(),
        TabStyle::Pill => bar.pill(),
    };
    let workstation = cx.weak_entity();
    let bar = bar.on_click(move |index: &usize, _, cx: &mut App| {
        let Some(command) = commands.get(*index).cloned() else {
            return;
        };
        let _ = workstation.update(cx, |this, cx| this.dispatch(command, cx));
    });
    bar.children(tabs.into_iter().map(|spec| {
        // WHY: the Kit's `Tab` draws either an icon or a label, never both, and
        // an icon-only mode bar hides what each mode is. Modes are labelled.
        Tab::new().label(spec.label)
    }))
}

pub fn property_row(
    t: &Tokens,
    label: impl Into<SharedString>,
    modified: bool,
    enabled: bool,
    control: impl IntoElement,
    value: Option<String>,
    unit: &str,
) -> Div {
    let text = if enabled { t.dim } else { t.faint };
    let value_color = if enabled { t.text } else { t.faint };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .h(px(28.))
        .px(px(10.))
        .child(
            // A dot marks a value that differs from its default.
            div()
                .flex_shrink_0()
                .w(px(5.))
                .h(px(5.))
                .rounded_full()
                .when(modified, |dot| dot.bg(t.selected)),
        )
        .child(
            div()
                .flex_shrink_0()
                .w(px(76.))
                .truncate()
                .text_size(px(11.))
                .text_color(text)
                .child(label.into()),
        )
        .child(div().flex_1().min_w(px(0.)).child(control))
        .child(
            div()
                .flex_shrink_0()
                .w(px(38.))
                .flex()
                .justify_end()
                .text_size(px(11.))
                .text_color(value_color)
                .children(value),
        )
        .child(
            div()
                .flex_shrink_0()
                .w(px(16.))
                .text_size(px(10.))
                .text_color(t.faint)
                .child(unit.to_string()),
        )
}

/// Rounds a raw value to the slider step, anchored at the minimum.
///
/// The Kit slider works in `f32`, so a step such as 0.01 leaves artefacts like
/// 0.30000001; this brings the value back onto the step grid before it becomes
/// a `Command`.
pub fn quantize(value: f64, min: f64, step: f64) -> f64 {
    if step <= 0.0 {
        return value;
    }
    min + ((value - min) / step).round() * step
}

/// Value for a pointer at `fraction` of the track.
#[cfg(test)]
pub fn value_at(fraction: f64, min: f64, max: f64, step: f64) -> f64 {
    quantize(min + fraction * (max - min), min, step).clamp(min, max)
}

/// Value after a key press, or `None` for keys the slider ignores. The Kit
/// slider only reacts to the pointer, so keyboard stepping stays here.
pub fn value_after_key(
    key: &str,
    shift: bool,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
) -> Option<f64> {
    let big = step * 10.0;
    let next = match key {
        "left" | "down" => value - if shift { big } else { step },
        "right" | "up" => value + if shift { big } else { step },
        "pageup" => value + big,
        "pagedown" => value - big,
        "home" => min,
        "end" => max,
        _ => return None,
    };
    Some(next.clamp(min, max))
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` also exports GPUI's `test` attribute, which would shadow this one.
    use core::prelude::v1::test;

    #[test]
    fn pointer_values_snap_to_the_step_and_stay_in_range() {
        assert_eq!(value_at(0.0, 10.0, 400.0, 0.5), 10.0);
        assert_eq!(value_at(1.0, 10.0, 400.0, 0.5), 400.0);
        assert_eq!(value_at(0.5, 0.0, 100.0, 1.0), 50.0);
        assert_eq!(value_at(0.333, 0.0, 100.0, 5.0), 35.0);
        assert_eq!(value_at(2.0, 0.0, 100.0, 1.0), 100.0);
    }

    #[test]
    fn f32_slider_artefacts_return_to_the_step_grid() {
        let raw = f64::from(0.3_f32);
        assert_ne!(raw, 0.3);
        assert!((quantize(raw, 0.0, 0.01) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn keys_step_page_and_jump() {
        let key = |name, shift, value| value_after_key(name, shift, value, 0.0, 100.0, 1.0);
        assert_eq!(key("right", false, 50.0), Some(51.0));
        assert_eq!(key("left", true, 50.0), Some(40.0));
        assert_eq!(key("pageup", false, 50.0), Some(60.0));
        assert_eq!(key("home", false, 50.0), Some(0.0));
        assert_eq!(key("end", false, 50.0), Some(100.0));
        assert_eq!(key("right", false, 100.0), Some(100.0));
        assert_eq!(key("x", false, 50.0), None);
    }
}
