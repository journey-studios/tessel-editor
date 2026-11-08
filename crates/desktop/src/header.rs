//! The unified window header: title, sequence name, the file actions (open, save, import,
//! attach scene, undo/redo, package, export, jobs) and the live controls for panels and theme.
//!
//! It replaces the header the Tailor design used to generate. The static part
//! is declared once in [`NODES`], which drives both the rendering and the
//! automation tree, so the two cannot disagree. IDs keep the `node:<n>` values
//! the Tailor export used so existing scripts keep resolving them.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::toolbar::Toolbar;
use gpui_kit::component::{ActiveTheme, Selectable, Sizable, h_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::state::layout::{LayoutCommand, PanelKind};
use crate::theme;
use crate::workstation::{Source, Workstation, shortcut_label};
use tessel_editor_core::perf::PerfMeterCommand;

/// What node 13 declares before it is given the live project title.
const TITLE_NODE_LABEL: &str = "";
/// The node that carries the open project's title.
const TITLE_NODE: u32 = 13;

/// Strip height; the native traffic lights are centred in it.
pub const HEIGHT: f32 = 40.0;
/// Room the traffic lights take on the left, see `traffic_light_position`.
const TRAFFIC_LIGHTS: f32 = 80.0;

/// A static header element: `(node number, kind, label, parent node, hidden)`.
type StaticNode = (u32, &'static str, Option<&'static str>, Option<u32>, bool);

/// Declaration of the header's static elements. `None` parent means the
/// element hangs directly from `region:header`.
const NODES: [StaticNode; 27] = [
    (1, "frame", None, None, false),
    (2, "frame", None, Some(1), false),
    (3, "frame", None, Some(2), false),
    (4, "frame", None, Some(3), false),
    (5, "text", Some("Tessel Editor"), Some(4), false),
    (6, "badge", Some("Estúdio de cena"), Some(4), true),
    (7, "badge", Some("Estado demonstrativo"), Some(4), true),
    (8, "frame", None, Some(3), false),
    (9, "button", Some("Desfazer"), Some(8), false),
    (10, "icon", None, Some(9), false),
    (11, "button", Some("Refazer"), Some(8), false),
    (12, "icon", None, Some(11), false),
    // The text of node 13 is the open project's `Project / Sequence` (BL144): `nodes(title)`
    // replaces this placeholder with the live label, the same one the header paints.
    (13, "text", Some(TITLE_NODE_LABEL), Some(3), false),
    (14, "button", Some("Importar"), Some(8), false),
    (15, "icon", None, Some(14), false),
    (16, "button", Some("Exportar"), Some(8), false),
    (17, "icon", None, Some(16), false),
    (18, "button", Some("Abrir"), Some(8), false),
    (19, "button", Some("Salvar"), Some(8), false),
    (20, "button", Some("Empacotar"), Some(8), false),
    (21, "button", Some("Tarefas"), Some(8), false),
    (22, "icon", None, Some(18), false),
    (23, "icon", None, Some(19), false),
    (24, "icon", None, Some(20), false),
    (25, "icon", None, Some(21), false),
    (26, "button", Some("Anexar cena"), Some(8), false),
    (27, "icon", None, Some(26), false),
];

/// The file actions, in on-screen order, with their node numbers. Each is a gesture on the
/// workstation (`Workstation::workflow_invoke`) so a click and `ui invoke node:<N>` are one path.
const ACTIONS: [(u32, &str, IconName); 9] = [
    (18, "Abrir", IconName::FolderOpen),
    (19, "Salvar", IconName::Save),
    (14, "Importar", IconName::FolderInput),
    (26, "Anexar cena", IconName::Clapperboard),
    (9, "Desfazer", IconName::Undo2),
    (11, "Refazer", IconName::Redo2),
    (20, "Empacotar", IconName::Package),
    (16, "Exportar", IconName::Download),
    (21, "Tarefas", IconName::ListChecks),
];

/// The header's static nodes for the automation tree, parented to
/// `region:header` (the root) or to each other.
pub fn nodes(title: &str) -> Vec<Value> {
    NODES
        .iter()
        .map(|(number, kind, label, parent, hidden)| {
            let label = if *number == TITLE_NODE {
                Some(title)
            } else {
                *label
            };
            let children: Vec<String> = NODES
                .iter()
                .filter(|(_, _, _, candidate, _)| *candidate == Some(*number))
                .map(|(child, ..)| format!("node:{child}"))
                .collect();
            let node = json!({
                "id": format!("node:{number}"), "kind": kind, "label": label,
                "parent": parent.map_or("region:header".to_string(), |parent| format!("node:{parent}")),
                "children": children, "enabled": true, "visible": !hidden,
                "actions": if *kind == "button" { json!([{ "action": "invoke", "label": label }]) } else { json!([]) },
                "fields": [], "source": "header.rs",
            });
            node
        })
        .collect()
}

pub fn node(id: &str, title: &str) -> Option<Value> {
    nodes(title).into_iter().find(|node| node["id"] == id)
}

/// Header content for one panel toggle: icon and the Portuguese accessible name.
fn panel_toggle(panel: PanelKind) -> (IconName, &'static str) {
    match panel {
        PanelKind::Library => (IconName::PanelLeft, "Mostrar ou ocultar a biblioteca"),
        PanelKind::Monitor => (IconName::Monitor, "Mostrar ou ocultar o monitor"),
        PanelKind::Timeline => (IconName::PanelBottom, "Mostrar ou ocultar a timeline"),
        PanelKind::Inspector => (IconName::PanelRight, "Mostrar ou ocultar o inspetor"),
    }
}

pub struct Header {
    workstation: Entity<Workstation>,
    probe: crate::workstation::Geometry,
}

impl Header {
    pub fn new(workstation: Entity<Workstation>, cx: &mut Context<Self>) -> Self {
        // The toggles show the live dock, so they repaint with the workstation.
        cx.observe(&workstation, |_, _, cx| cx.notify()).detach();
        let probe = workstation.read(cx).geometry.clone();
        Self { workstation, probe }
    }

    /// A header button is the semantic action `node:<N>` of the workstation.
    fn invoke_node(&self, number: u32, window: &mut Window, cx: &mut App) {
        self.workstation.update(cx, |workstation, cx| {
            let _ = workstation.workflow_invoke(&format!("node:{number}"), None, window, cx);
        });
    }

    fn run(&self, command: LayoutCommand, window: &mut Window, cx: &mut App) {
        self.workstation.update(cx, |workstation, cx| {
            let _ = workstation.execute_layout(command, Source::Gesture, window, cx);
        });
    }
}

impl Render for Header {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = self.workstation.read(cx).layout_snapshot(cx);
        let dark = theme::is_dark(cx);
        let colors = cx.theme().colors;

        let title = self.workstation.read(cx).state.header().label();
        let perf_visible = self.workstation.read(cx).perf_visible();
        let perf_label = format!("Medidor de FPS e debug ({})", shortcut_label());
        let compact = window.viewport_size().width < px(1040.);
        let actions = ACTIONS.iter().fold(
            Toolbar::new("header-actions").xsmall(),
            |toolbar, (number, label, icon)| {
                let number = *number;
                toolbar.child(
                    Button::new(format!("node-{number}"))
                        .ghost()
                        .icon(*icon)
                        .when(!compact, |button| button.label(*label))
                        .tooltip(*label)
                        .accessibility_label(*label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.invoke_node(number, window, cx);
                        })),
                )
            },
        );

        let toggles = PanelKind::ALL.into_iter().fold(
            Toolbar::new("header-panels").xsmall(),
            |toolbar, panel| {
                let visible = layout.is_visible(panel);
                let (icon, label) = panel_toggle(panel);
                toolbar.child(
                    Button::new(format!("toolbar-panel-{}", panel.id()))
                        .ghost()
                        .icon(icon)
                        .tooltip(label)
                        .accessibility_label(label)
                        .selected(visible)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run(
                                LayoutCommand::SetVisible {
                                    panel,
                                    visible: !visible,
                                },
                                window,
                                cx,
                            );
                        })),
                )
            },
        );
        let controls = Toolbar::new("header-controls")
            .xsmall()
            .child(
                Button::new("toolbar-layout-reset")
                    .ghost()
                    .icon(IconName::LayoutPanelTop)
                    .tooltip("Restaurar o layout padrão")
                    .accessibility_label("Restaurar o layout padrão")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run(LayoutCommand::Reset, window, cx);
                    })),
            )
            .child(
                Button::new("toolbar-perf")
                    .ghost()
                    .icon(IconName::Gauge)
                    .tooltip(perf_label.clone())
                    .accessibility_label(perf_label)
                    .selected(perf_visible)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.workstation.update(cx, |workstation, cx| {
                            let _ = workstation.execute_perf(
                                PerfMeterCommand::Toggle,
                                Source::Gesture,
                                cx,
                            );
                        });
                    })),
            )
            .child(
                Button::new("toolbar-theme")
                    .ghost()
                    .icon(if dark { IconName::Sun } else { IconName::Moon })
                    .tooltip(if dark {
                        "Usar tema claro"
                    } else {
                        "Usar tema escuro"
                    })
                    .accessibility_label(if dark {
                        "Usar tema claro"
                    } else {
                        "Usar tema escuro"
                    })
                    .on_click(|_, _, cx| theme::apply(cx, !theme::is_dark(cx))),
            );

        h_flex()
            .relative()
            .h(px(HEIGHT))
            .w_full()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .gap_2()
            .pl(px(TRAFFIC_LIGHTS))
            .pr_2()
            .bg(colors.title_bar)
            .border_b_1()
            .border_color(colors.title_bar_border)
            .child(self.probe.probe("region:header"))
            .child(
                h_flex()
                    .gap_2()
                    .min_w(px(0.))
                    .items_center()
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(px(14.))
                            .child("Tessel Editor"),
                    )
                    .child(
                        div()
                            .truncate()
                            .debug_selector(|| "header-title".to_string())
                            .text_size(px(12.))
                            .text_color(colors.muted_foreground)
                            .child(title),
                    ),
            )
            .child(
                h_flex()
                    .relative()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .child(self.probe.probe("region:toolbar"))
                    .child(actions)
                    .child(div().w(px(1.)).h(px(16.)).bg(colors.border))
                    .child(toggles)
                    .child(div().w(px(1.)).h(px(16.)).bg(colors.border))
                    .child(controls),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` also exports GPUI's `test` attribute, which would shadow this one.
    use core::prelude::v1::test;
    use std::collections::HashSet;

    #[test]
    fn header_nodes_are_addressable_and_unique() {
        // Ported from `automation::model`: the header used to come from design.json.
        let all = nodes("Film / Cut");
        let ids: HashSet<_> = all
            .iter()
            .map(|node| node["id"].as_str().unwrap())
            .collect();
        assert_eq!(all.len(), ids.len());
        assert!(all.len() >= 15);
        for id in &ids {
            assert!(node(id, "").is_some());
        }
        assert!(node("node:missing", "").is_none());
    }

    #[test]
    fn header_has_one_root_under_the_header_region() {
        let roots: Vec<_> = nodes("")
            .into_iter()
            .filter(|node| node["parent"] == "region:header")
            .collect();
        assert_eq!(roots.len(), 1);
    }

    #[test]
    fn header_buttons_are_enabled_and_invokable() {
        for node in nodes("") {
            if node["kind"] == "button" {
                assert_eq!(node["enabled"], true);
                assert_eq!(node["actions"][0]["action"], "invoke");
            }
        }
    }

    #[test]
    fn every_rendered_action_has_a_declared_button() {
        for (number, label, _) in ACTIONS {
            let declared = node(&format!("node:{number}"), "").unwrap();
            assert_eq!(declared["kind"], "button");
            assert_eq!(declared["label"], label);
        }
    }
}
