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
use gpui_kit::component::{ActiveTheme, Icon, Selectable, Sizable, h_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::state::layout::{LayoutCommand, PanelKind};
use crate::theme;
use crate::workstation::{Source, Workstation, shortcut_label};
use tessel_editor_core::perf::PerfMeterCommand;
use tessel_editor_core::perf_history::HistoryCommand;

/// What node 13 declares before it is given the live project title.
const TITLE_NODE_LABEL: &str = "";
/// The node that carries the open project's title.
const TITLE_NODE: u32 = 13;

/// Strip height; the native traffic lights (macOS) and the caption buttons are centred in it.
pub const HEIGHT: f32 = 40.0;
/// macOS paints the native traffic lights over the transparent title bar, on the left.
/// Everywhere else the window has no native caption, so the header draws its own buttons
/// on the right (`caption_buttons`) and needs no room on the left.
const NATIVE_TRAFFIC_LIGHTS: bool = cfg!(target_os = "macos");
/// Room the traffic lights take on the left, see `traffic_light_position`.
const TRAFFIC_LIGHTS: f32 = 80.0;
/// Left inset when there are no traffic lights.
const PLAIN_INSET: f32 = 12.0;
/// Width of each drawn caption button (Windows' own are 46 px).
const CAPTION_BUTTON_WIDTH: f32 = 46.0;

/// The buttons the header draws on platforms without native traffic lights.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Caption {
    Minimize,
    Maximize,
    Restore,
    Close,
}

impl Caption {
    fn id(self) -> &'static str {
        match self {
            Self::Minimize => "caption-minimize",
            Self::Maximize => "caption-maximize",
            Self::Restore => "caption-restore",
            Self::Close => "caption-close",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Minimize => IconName::WindowMinimize,
            Self::Maximize => IconName::WindowMaximize,
            Self::Restore => IconName::WindowRestore,
            Self::Close => IconName::WindowClose,
        }
    }

    /// Windows resolves the click itself from this hit-test area (`WM_NCHITTEST`), which also
    /// gives the snap-layouts flyout; the button must not handle the click as well.
    fn area(self) -> WindowControlArea {
        match self {
            Self::Minimize => WindowControlArea::Min,
            Self::Maximize | Self::Restore => WindowControlArea::Max,
            Self::Close => WindowControlArea::Close,
        }
    }

    /// The same operations the automation `window.*` actions run.
    fn run(self, window: &mut Window) {
        match self {
            Self::Minimize => window.minimize_window(),
            Self::Maximize | Self::Restore => window.zoom_window(),
            Self::Close => window.remove_window(),
        }
    }
}

/// Minimize, maximize/restore and close, drawn flush with the right edge of the strip.
fn caption_buttons(window: &Window, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let (hover, hover_text, active) = (
        theme.secondary_hover,
        theme.secondary_foreground,
        theme.secondary_active,
    );
    let (close_hover, close_text, close_active) =
        (theme.danger, theme.danger_foreground, theme.danger_active);
    let text = theme.foreground;
    let maximized = window.is_maximized();
    let button = |kind: Caption| {
        let close = kind == Caption::Close;
        let (hover, hover_text, active) = if close {
            (close_hover, close_text, close_active)
        } else {
            (hover, hover_text, active)
        };
        div()
            .id(kind.id())
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .w(px(CAPTION_BUTTON_WIDTH))
            .h_full()
            .text_color(text)
            .hover(|style| style.bg(hover).text_color(hover_text))
            .active(|style| style.bg(active).text_color(hover_text))
            .window_control_area(kind.area())
            // Only Linux has no hit-test area for the click, so it is handled here.
            .when(cfg!(target_os = "linux"), |this| {
                this.on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    kind.run(window);
                })
            })
            .child(Icon::new(kind.icon()).small())
    };
    h_flex()
        .id("header-caption")
        .flex_shrink_0()
        .h_full()
        .child(button(Caption::Minimize))
        .child(button(if maximized {
            Caption::Restore
        } else {
            Caption::Maximize
        }))
        .child(button(Caption::Close))
}

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
        let history_visible = self.workstation.read(cx).history_visible();
        let history_label = "Histórico de desempenho (60 s)";
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
                Button::new("toolbar-perf-history")
                    .ghost()
                    .icon(IconName::ChartLine)
                    .tooltip(history_label)
                    .accessibility_label(history_label)
                    .selected(history_visible)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.workstation.update(cx, |workstation, cx| {
                            let command = if workstation.history_visible() {
                                HistoryCommand::Hide
                            } else {
                                HistoryCommand::Show
                            };
                            let _ = workstation.execute_perf_history(
                                command,
                                Source::Gesture,
                                window,
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
            .pl(px(if NATIVE_TRAFFIC_LIGHTS {
                TRAFFIC_LIGHTS
            } else {
                PLAIN_INSET
            }))
            // Drawn caption buttons sit flush with the edge, the actions keep their own gap.
            .when(NATIVE_TRAFFIC_LIGHTS, |this| this.pr_2())
            .bg(colors.title_bar)
            .border_b_1()
            .border_color(colors.title_bar_border)
            .child(self.probe.probe("region:header"))
            .child(
                // The title zone is the drag handle and takes the free width. It never overlaps a
                // button: on Windows the first hit-test area under the cursor wins.
                h_flex()
                    .gap_2()
                    .min_w(px(0.))
                    .flex_1()
                    .h_full()
                    .items_center()
                    .when(!NATIVE_TRAFFIC_LIGHTS, |this| {
                        this.window_control_area(WindowControlArea::Drag).when(
                            cfg!(target_os = "linux"),
                            |this| {
                                this.on_mouse_down(MouseButton::Left, |_, window, _| {
                                    window.start_window_move();
                                })
                            },
                        )
                    })
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
                    .child(controls)
                    .when(!NATIVE_TRAFFIC_LIGHTS, |this| this.pr_2()),
            )
            .when(!NATIVE_TRAFFIC_LIGHTS, |this| {
                this.child(caption_buttons(window, cx))
            })
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
