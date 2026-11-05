//! The "Cena" inspector: attached-scene picker, scene tree, and the typed properties of the
//! selected entity (BL014/BL021; `docs/features/scene-inspector.md`).
//!
//! Nothing here owns scene data. The panel is built each frame from the [`Snapshot`] that
//! `scene_model` reads out of `EditorState`; every control sends the same commands the socket
//! does (`scene.select`, `scene.gesture.*`, `scene.edit`). A control that cannot act is disabled
//! with its reason on screen, never an empty callback.

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::select::Select;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tree::tree;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::controls::{disabled_tool_button, glyph, quantize, tool_button, value_after_key};
use super::scene_gesture::{SliderKey, scene_slider_key_up, scene_slider_keys, scene_slider_press};
use super::scene_model::{
    Availability, MAX_VALUE, MIN_TRACK, PanelFit, PropertyView, Snapshot, TREE_ROW_HEIGHT, Target,
    TargetView, TreeEntry, component_values, components, format_number, scrub, unit_label,
};
use super::scene_ui::{InspectorMode, NO_SCENE_REASON, SliderEntry, depth_of};
use super::tokens::Tokens;
use super::{Source, Workstation};

/// Opens the "attach scene" dialog of the window. The seam F043 (`scene_import.rs`, frente R2)
/// plugs into: the button below stays disabled, with its reason on screen, while this is `None`
/// and never calls an empty handler.
pub(super) type AttachSceneOpener = fn(&mut Workstation, &mut Window, &mut Context<Workstation>);

const ATTACH_SCENE_OPENER: Option<AttachSceneOpener> = Some(|this, window, cx| {
    let _ = this.open_flow(super::flows::FlowKind::AttachScene, window, cx);
});

/// Whether the attach button can act: the handler to call, or the reason it cannot. Pure so the
/// decision is tested without a window.
pub(super) fn attach_availability(
    opener: Option<AttachSceneOpener>,
) -> Result<AttachSceneOpener, &'static str> {
    opener.ok_or(ATTACH_SCENE_UNAVAILABLE)
}

const ATTACH_SCENE_UNAVAILABLE: &str =
    "O diálogo de anexar cena ainda não está ligado à janela; use scene.attach (ui op ou headless)";

/// Width of the component label ("X", "Y°") column of a one-line slider row.
const COMPONENT_LABEL_WIDTH: f32 = 22.0;
/// Slider rows stack in the compact layout and keep the same height rhythm as one-line rows.
const ROW_HEIGHT: f32 = 28.0;
/// Slider state is `f32`; anything closer than this is the same value.
const SLIDER_EPSILON: f32 = 1e-4;

/// Icon of a tree entity by the `kind` the core's `scene.tree` reports.
fn entity_icon(kind: &str) -> IconName {
    match kind {
        "camera_clip" => IconName::Film,
        "shot" => IconName::ChartGantt,
        "performance_track" => IconName::AudioLines,
        _ => IconName::Layers,
    }
}

/// What one slider row prints around its control.
struct SliderRow<'a> {
    /// Element ID suffix, the same as the slider's (`SliderKey::element_id`).
    element: &'a str,
    /// Component label (`X`, `Y°`); empty for a single number, whose property header says it.
    label: &'a str,
    modified: bool,
    value: String,
    unit: &'a str,
}

impl Workstation {
    /// The layout the inspector's last measured size calls for. The measure is the previous
    /// frame's `region:inspector` probe; `watch_inspector_size` asks for one more frame when a
    /// resize changes the answer, so the layout never stays at a stale size.
    pub(super) fn scene_fit(&self) -> PanelFit {
        self.geometry
            .get("region:inspector")
            .map_or(PanelFit::UNMEASURED, |bounds| {
                PanelFit::of(f32::from(bounds.size.width), f32::from(bounds.size.height))
            })
    }

    /// An invisible element that requests one more frame when the size it is painted at no
    /// longer matches the `fit` this frame was built with. WHY: the bounds a render reads are the
    /// previous frame's, and a resize with nothing else to repaint would otherwise leave the
    /// rows laid out for the old width until the next unrelated repaint.
    fn watch_inspector_size(built_with: PanelFit) -> impl IntoElement {
        canvas(
            move |bounds, window, _| {
                let now = PanelFit::of(f32::from(bounds.size.width), f32::from(bounds.size.height));
                if now != built_with {
                    window.request_animation_frame();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .size_full()
    }

    /// The inspector with its mode bar: the clip inspector of the timeline, or the scene one.
    pub(super) fn render_inspector(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mode = self.scene_ui.mode;
        let fit = self.scene_fit();
        let workstation = cx.weak_entity();
        let bar = TabBar::new("inspector-mode").selected_index(
            InspectorMode::ALL
                .iter()
                .position(|m| *m == mode)
                .unwrap_or(0),
        );
        // The pills need about 130 px; the equal-width bar fits the dock's 100 px floor.
        let bar = if fit.equal_tabs {
            bar.underline()
        } else {
            bar.pill()
        };
        let bar = bar
            .on_click(move |index: &usize, _, cx: &mut App| {
                let Some(next) = InspectorMode::ALL.get(*index).copied() else {
                    return;
                };
                let _ = workstation.update(cx, |this, cx| this.scene_set_mode(next, cx));
            })
            .children(
                InspectorMode::ALL
                    .iter()
                    .map(|mode| Tab::new().label(mode.label())),
            );
        let body = match mode {
            InspectorMode::Clip => self.render_clip_inspector(t, window, cx).into_any_element(),
            InspectorMode::Scene => self
                .render_scene_inspector(t, fit, window, cx)
                .into_any_element(),
        };
        v_flex()
            .relative()
            .size_full()
            .min_h(px(0.))
            .bg(t.panel)
            .child(self.geometry.probe("region:inspector"))
            .child(Self::watch_inspector_size(fit))
            .child(
                div()
                    .flex_shrink_0()
                    .overflow_hidden()
                    .px(px(8.))
                    .py(px(4.))
                    .child(bar),
            )
            .child(div().flex_1().min_h(px(0.)).child(body))
    }

    fn render_scene_inspector(
        &mut self,
        t: &Tokens,
        fit: PanelFit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let snapshot = self.scene_snapshot();
        // Before the controls sync: opening an ancestor changes which rows the tree is given.
        let reveal = snapshot
            .as_deref()
            .is_some_and(|snapshot| self.scene_open_selection_ancestors(snapshot, cx));
        self.sync_scene_controls(snapshot.as_deref(), window, cx);
        if reveal {
            self.scene_scroll_tree_to_selection(cx);
        }
        let monitor = self.scene_monitor();

        let mut content = v_flex().w_full().gap(px(6.));
        content = content.child(self.scene_monitor_note(t, monitor.badge(), monitor.reason(), cx));
        content = content.child(self.scene_attach_row(t, fit, ATTACH_SCENE_OPENER, cx));
        let Some(snapshot) = snapshot else {
            content = content.child(self.scene_empty(t, NO_SCENE_REASON));
            return self.scene_scroll(t, content);
        };

        if self.state.scenes.len() > 1 {
            content = content.child(
                div().px(px(10.)).child(
                    Select::new(&self.scene_ui.picker)
                        .id("scene-picker")
                        .small()
                        .accessibility_label("Cena ativa"),
                ),
            );
        }
        content = content.child(self.scene_tree_box(t, fit, &snapshot, cx));
        if let Some(notice) = self.scene_notice(t) {
            content = content.child(notice);
        }
        if let Some(status) = self.scene_gesture_status(t, &snapshot, cx) {
            content = content.child(status);
        }
        content = content.child(self.scene_target(t, fit, &snapshot, window, cx));
        self.scene_scroll(t, content)
    }

    /// A selection made by anything but a click on the row (`scene.select` over the socket, the
    /// headless API) can name an entity whose ancestors are collapsed, which the tree cannot
    /// show. Opens them through the same `expand` the tree's own arrow does. Returns `true` when
    /// this frame has a new selection to scroll to. Asked once per selection change.
    fn scene_open_selection_ancestors(
        &mut self,
        snapshot: &Snapshot,
        cx: &mut Context<Self>,
    ) -> bool {
        if !snapshot.take_reveal() {
            return false;
        }
        let Some(selected) = snapshot.single_selection() else {
            return false;
        };
        let closed: Vec<String> = TreeEntry::ancestors(&snapshot.entries, selected)
            .into_iter()
            .filter(|ancestor| {
                !self
                    .scene_ui
                    .is_expanded(ancestor, depth_of(&snapshot.entries, ancestor))
            })
            .map(str::to_string)
            .collect();
        for ancestor in closed {
            // The node ID is `scene_ui.rs`'s `scene.entity:<path>`; `expand` is its own handler.
            let _ = self.scene_invoke(&format!("scene.entity:{ancestor}"), Some("expand"), cx);
        }
        true
    }

    /// The Kit tree selects a row without scrolling to it. Nearest: a row already on screen
    /// stays where it is, one outside is brought to the closest edge.
    fn scene_scroll_tree_to_selection(&self, cx: &mut Context<Self>) {
        let tree = self.scene_ui.tree.read(cx);
        let Some(row) = tree.selected_index() else {
            return;
        };
        tree.scroll_handle()
            .scroll_to_item(row, ScrollStrategy::Nearest);
    }

    fn scene_scroll(&self, t: &Tokens, content: Div) -> impl IntoElement + use<> {
        div()
            .id("scene-inspector-scroll")
            .size_full()
            .min_h(px(0.))
            .bg(t.panel)
            .overflow_y_scrollbar()
            .child(content)
    }

    fn scene_empty(&self, t: &Tokens, message: &str) -> impl IntoElement + use<> {
        v_flex()
            .items_center()
            .gap(px(8.))
            .p(px(20.))
            .text_size(px(11.))
            .text_color(t.dim)
            .child(glyph(IconName::SlidersHorizontal, 20.0, t.faint))
            .child(div().text_center().child(message.to_string()))
    }

    /// Says whether the monitor can show the scene, offers the button that does, and, when it
    /// cannot, says why on screen (a tooltip alone is invisible to anyone who does not hover).
    /// The row wraps instead of truncating: at the dock's narrowest the badge and the button
    /// each get a line.
    fn scene_monitor_note(
        &self,
        t: &Tokens,
        badge: &str,
        reason: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let available = self.scene_monitor().is_available();
        let action = if available {
            Button::new("scene-show-in-monitor")
                .label("Ver no monitor")
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(scene) = this.scene_active_id()
                        && let Err(error) = this.show_directed_scene(&scene, cx)
                    {
                        this.scene_report(error, cx);
                    }
                }))
                .into_any_element()
        } else {
            disabled_tool_button(
                "scene-show-in-monitor",
                IconName::Film,
                "Ver no monitor",
                reason,
            )
            .into_any_element()
        };
        v_flex()
            .gap(px(2.))
            .px(px(10.))
            .py(px(4.))
            .text_size(px(10.))
            .text_color(t.faint)
            .child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_x(px(6.))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w(px(88.))
                            .items_center()
                            .gap(px(6.))
                            .child(glyph(IconName::Info, 12.0, t.faint))
                            .child(div().flex_1().min_w(px(0.)).child(badge.to_string())),
                    )
                    .child(div().flex_shrink_0().child(action)),
            )
            .children((!available).then(|| {
                div()
                    .id("scene-monitor-reason")
                    .relative()
                    .child(self.geometry.probe("scene:monitor-reason"))
                    .child(reason.to_string())
            }))
    }

    /// "Anexar cena": enabled only when the window has a dialog to open (`opener`); otherwise
    /// disabled with the reason printed under it, never a button that does nothing.
    fn scene_attach_row(
        &self,
        t: &Tokens,
        fit: PanelFit,
        opener: Option<AttachSceneOpener>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        // The full label is wider than the dock's 100 px floor leaves; the short one keeps the
        // button whole and the full name stays on its tooltip and accessibility label.
        let label = if fit.compact { "Anexar" } else { "Anexar cena" };
        let button = Button::new("scene-attach")
            .ghost()
            .xsmall()
            .icon(IconName::Plus)
            .label(label)
            .tooltip("Anexar cena")
            .accessibility_label("Anexar cena");
        let availability = attach_availability(opener);
        let button = match availability {
            Ok(open) => button
                .on_click(cx.listener(move |this, _, window, cx| open(this, window, cx)))
                .into_any_element(),
            Err(_) => button.disabled(true).into_any_element(),
        };
        v_flex()
            .gap(px(2.))
            .px(px(10.))
            .child(
                h_flex().child(
                    div()
                        .relative()
                        .child(self.geometry.probe("scene:attach"))
                        .child(button),
                ),
            )
            .child({
                let reason = availability
                    .err()
                    .unwrap_or("Anexe um render-bundle e escolha a pasta de recursos");
                div()
                    .id("scene-attach-reason")
                    .relative()
                    .text_size(px(10.))
                    .text_color(t.faint)
                    .child(self.geometry.probe("scene:attach-reason"))
                    .child(reason)
            })
    }

    fn scene_tree_box(
        &mut self,
        t: &Tokens,
        fit: PanelFit,
        snapshot: &Snapshot,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut visible = Vec::new();
        TreeEntry::visible(
            &snapshot.entries,
            &|path| {
                self.scene_ui
                    .is_expanded(path, depth_of(&snapshot.entries, path))
            },
            0,
            &mut visible,
        );
        let kinds: Rc<HashMap<String, String>> = Rc::new(
            visible
                .iter()
                .map(|(_, entry)| (entry.path.clone(), entry.kind.clone()))
                .collect(),
        );
        let rows = visible.len();
        let workstation = cx.weak_entity();
        let tokens = *t;
        // 12 px a level leaves a 100 px dock nothing but icons from the third level down.
        let indent = if fit.compact { 6.0 } else { 12.0 };
        let list = tree(&self.scene_ui.tree, move |_, entry, selected, _, _| {
            let path = entry.item().id.to_string();
            let icon = kinds
                .get(&path)
                .map_or(IconName::Layers, |kind| entity_icon(kind));
            let workstation = workstation.clone();
            let target = path.clone();
            // A fixed height, not the text's: `PanelFit::tree_height` counts whole rows, and the
            // row height the font gives (27.5 px in the test platform) would cut the last one.
            ListItem::new(SharedString::from(format!("scene-entity:{path}")))
                .h(px(TREE_ROW_HEIGHT))
                .child(
                    h_flex()
                        .w_full()
                        .gap(px(6.))
                        .items_center()
                        .pl(px(entry.depth() as f32 * indent))
                        .text_size(px(12.))
                        .text_color(if selected { tokens.text } else { tokens.dim })
                        .child(Icon::new(icon).size(px(14.)).text_color(tokens.dim))
                        .child(div().flex_1().truncate().child(entry.item().label.clone())),
                )
                .on_click(move |_, _, cx| {
                    let target = target.clone();
                    let _ = workstation.update(cx, |this, cx| this.scene_pick_entity(target, cx));
                })
        });
        v_flex()
            .px(px(4.))
            .child(
                div()
                    .px(px(6.))
                    .pb(px(2.))
                    .text_size(px(9.))
                    .text_color(t.faint)
                    .child(format!("CENA · {}", snapshot.scene_name.to_uppercase())),
            )
            .child(
                div()
                    .id("scene-tree")
                    .relative()
                    .flex_shrink_0()
                    .h(px(fit.tree_height(rows)))
                    .child(self.geometry.probe("scene:tree"))
                    .child(list),
            )
    }

    /// Selecting from the pointer: the same `scene.select` the socket sends. The Kit tree also
    /// moves its own selection on that click, so the entity is recorded first and the observer
    /// that follows keyboard moves finds nothing new to send.
    fn scene_pick_entity(&mut self, path: String, cx: &mut Context<Self>) {
        self.scene_ui.pushed_selection = Some(path.clone());
        let _ = self.scene_select_entity(&path, Source::Gesture, cx);
    }

    fn scene_notice(&self, t: &Tokens) -> Option<Div> {
        let notice = self.scene_ui.notice.as_ref()?;
        Some(
            h_flex()
                .items_start()
                .gap(px(6.))
                .mx(px(10.))
                .p(px(6.))
                .rounded(px(3.))
                .border_1()
                .border_color(t.bad)
                .text_size(px(11.))
                .text_color(t.text)
                .child(glyph(IconName::CircleAlert, 14.0, t.bad))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(0.))
                        .child(div().child(notice.message.clone()))
                        .child(
                            div()
                                .text_size(px(9.))
                                .text_color(t.faint)
                                .child(notice.code.clone()),
                        ),
                ),
        )
    }

    /// Shown whenever the core holds a gesture on the scene, whoever began it (a socket client
    /// leaves one open on purpose): the candidate is on screen and can be cancelled. The text
    /// wraps (the button drops to its own line when the panel is narrow) instead of being cut.
    fn scene_gesture_status(
        &mut self,
        t: &Tokens,
        snapshot: &Snapshot,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let gesture = snapshot.gesture.as_ref()?;
        let text = match gesture.invalidated {
            Some(reason) => format!("Gesto invalidado ({reason}); cancele para continuar"),
            None => format!("Gesto em andamento · {} atualizações", gesture.updates),
        };
        let workstation = cx.weak_entity();
        Some(
            h_flex()
                .flex_wrap()
                .items_center()
                .gap_x(px(6.))
                .mx(px(10.))
                .text_size(px(11.))
                .text_color(t.dim)
                .child(
                    div()
                        .id("scene-gesture-status")
                        .flex_1()
                        .min_w(px(96.))
                        .child(text),
                )
                .child(
                    Button::new("scene-gesture-cancel")
                        .ghost()
                        .xsmall()
                        .label("Cancelar gesto")
                        .on_click(move |_, _, cx| {
                            let _ = workstation.update(cx, |this, cx| {
                                let _ = this.scene_cancel_open_gesture(Source::Gesture, cx);
                            });
                        }),
                ),
        )
    }

    fn scene_target(
        &mut self,
        t: &Tokens,
        fit: PanelFit,
        snapshot: &Snapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let body = match &snapshot.target {
            Target::None => self
                .scene_empty(
                    t,
                    "Selecione uma entidade na árvore para editar suas propriedades",
                )
                .into_any_element(),
            Target::Multiple(count) => self
                .scene_empty(
                    t,
                    &format!("{count} entidades selecionadas; edite uma por vez"),
                )
                .into_any_element(),
            Target::Entity(target) => self
                .scene_properties(t, fit, snapshot, target, window, cx)
                .into_any_element(),
        };
        div().w_full().child(body)
    }

    fn scene_properties(
        &mut self,
        t: &Tokens,
        fit: PanelFit,
        snapshot: &Snapshot,
        target: &TargetView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        // A slider of another entity is dropped unless a drag still owns it, so the map cannot
        // grow with every selection and a drag in flight never loses its control.
        let scene = snapshot.scene.clone();
        let entity = target.path.clone();
        let tracked = &self.scene_ui.gesture;
        let kept: Vec<SliderKey> = self
            .scene_ui
            .sliders
            .keys()
            .filter(|key| tracked.is_tracked(key) || (key.scene == scene && key.entity == entity))
            .cloned()
            .collect();
        self.scene_ui.sliders.retain(|key, _| kept.contains(key));

        // A rule between the tree and the properties: without it the entity name reads as one
        // more row of the tree.
        let mut list = v_flex().w_full().pb(px(8.)).child(
            div()
                .id("scene-target")
                .mx(px(10.))
                .mt(px(4.))
                .pt(px(6.))
                .pb(px(2.))
                .border_t_1()
                .border_color(t.border)
                .text_size(px(12.))
                .text_color(t.text)
                .child(target.label.clone()),
        );
        if target.properties.is_empty() {
            return list.child(self.scene_empty(
                t,
                "O catálogo não define propriedades editáveis para este tipo de entidade",
            ));
        }
        let gesture_open = snapshot.gesture_open();
        for property in &target.properties {
            list = list.child(self.scene_property_block(
                t,
                fit,
                &snapshot.scene,
                target,
                property,
                gesture_open,
                window,
                cx,
            ));
        }
        list
    }

    #[allow(clippy::too_many_arguments)]
    fn scene_property_block(
        &mut self,
        t: &Tokens,
        fit: PanelFit,
        scene: &str,
        target: &TargetView,
        property: &PropertyView,
        gesture_open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let spec = property.spec;
        let reset = property.reset_state(gesture_open);
        let reset_id = SharedString::from(format!("scene-reset:{}", property.path));
        let reset_label = format!("Redefinir {}", property.label);
        let reset_button = match reset {
            Err(reason) => {
                disabled_tool_button(reset_id, IconName::RotateCcw, &reset_label, &reason)
            }
            Ok(()) => {
                let workstation = cx.weak_entity();
                let path = property.path.clone();
                tool_button(reset_id, IconName::RotateCcw, &reset_label).on_click(
                    move |_, _, cx| {
                        let path = path.clone();
                        let _ = workstation.update(cx, |this, cx| {
                            let _ = this.scene_reset_property(&path, Source::Gesture, cx);
                        });
                    },
                )
            }
        };
        let header = h_flex()
            .items_center()
            .justify_between()
            .px(px(10.))
            .pt(px(6.))
            .text_size(px(11.))
            .text_color(if property.is_editable() {
                t.text
            } else {
                t.faint
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .child(property.label.to_string()),
            )
            .child(reset_button);

        let mut block = v_flex().w_full().child(header);
        if let Availability::Unavailable(reason) = &property.availability {
            return block.child(
                div()
                    .id(SharedString::from(format!(
                        "scene-unavailable:{}",
                        property.path
                    )))
                    .px(px(10.))
                    .pb(px(4.))
                    .text_size(px(10.))
                    .text_color(t.faint)
                    .child(reason.clone()),
            );
        }
        let values = property
            .shown()
            .as_ref()
            .map(component_values)
            .unwrap_or_default();
        let unit = unit_label(spec.unit.as_str());
        for (index, component) in components(spec.kind).iter().enumerate() {
            let Some(value) = values.get(index).copied() else {
                continue;
            };
            let key = SliderKey {
                scene: scene.to_string(),
                entity: target.path.clone(),
                path: property.path.clone(),
                index,
            };
            // The slider's accessible name keeps the property: "Posição X", not just "X".
            let label = if components(spec.kind).len() == 1 {
                property.label.to_string()
            } else {
                format!("{} {}", property.label, component.label)
            };
            let element = key.element_id();
            let control = self.scene_slider(key, &label, property, value, window, cx);
            block = block.child(self.scene_slider_row(
                t,
                fit,
                SliderRow {
                    element: &element,
                    label: component.label,
                    modified: property.is_modified(),
                    value: format_number(value),
                    unit,
                },
                control,
            ));
        }
        block
    }

    /// One slider with what surrounds it. Not `controls::property_row`, which the clip inspector
    /// also uses: that row reserves 76 px for a label that scene components fill with one
    /// letter, and 38 px for a value that `-12345.678` overflows into the track. Here the track
    /// has a floor (`MIN_TRACK`) and the value column grows to `MAX_VALUE` and then cuts with an
    /// ellipsis; when the panel is too narrow for that (`fit.compact`) the slider takes its
    /// own line under the label and the value.
    fn scene_slider_row(
        &self,
        t: &Tokens,
        fit: PanelFit,
        row: SliderRow<'_>,
        control: AnyElement,
    ) -> Div {
        let SliderRow {
            element,
            label,
            modified,
            value,
            unit,
        } = row;
        let dot = div()
            .flex_shrink_0()
            .w(px(5.))
            .h(px(5.))
            .rounded_full()
            .when(modified, |dot| dot.bg(t.selected));
        let value_cell = div()
            .id(SharedString::from(format!("scene-value-{element}")))
            .relative()
            .flex_shrink_0()
            .min_w(px(36.))
            .max_w(px(MAX_VALUE))
            .truncate()
            .text_right()
            .text_size(px(11.))
            .text_color(t.text)
            .child(self.geometry.probe(format!("value:{element}")))
            .child(value);
        let unit_cell = div()
            .flex_shrink_0()
            .w(px(14.))
            .text_size(px(10.))
            .text_color(t.faint)
            .child(unit.to_string());
        let component = div()
            .flex_shrink_0()
            .w(px(COMPONENT_LABEL_WIDTH))
            .truncate()
            .text_size(px(11.))
            .text_color(t.dim)
            .child(label.to_string());
        let has_label = !label.is_empty();

        if fit.compact {
            return v_flex()
                .gap(px(2.))
                .px(px(6.))
                .pt(px(2.))
                .pb(px(4.))
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(6.))
                        .h(px(18.))
                        .child(dot)
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(px(11.))
                                .text_color(t.dim)
                                .child(label.to_string()),
                        )
                        .child(value_cell)
                        .child(unit_cell),
                )
                .child(div().w_full().min_w(px(0.)).child(control));
        }
        h_flex()
            .items_center()
            .gap(px(6.))
            .h(px(ROW_HEIGHT))
            .px(px(10.))
            .child(dot)
            .children(has_label.then_some(component))
            .child(div().flex_1().min_w(px(MIN_TRACK)).child(control))
            .child(value_cell)
            .child(unit_cell)
    }

    /// One slider of one component. Its range is a window around the value (`scrub`), rebuilt
    /// only when the value leaves it by another route (an edit through the socket) and no drag
    /// owns the control; `SliderState` cannot change its range after creation.
    fn scene_slider(
        &mut self,
        key: SliderKey,
        label: &str,
        property: &PropertyView,
        value: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let index = key.index;
        let spec = property.spec;
        let tracked = self.scene_ui.gesture.is_tracked(&key);
        let stale = self
            .scene_ui
            .sliders
            .get(&key)
            .is_some_and(|entry| value < entry.scrub.min || value > entry.scrub.max);
        if stale && !tracked {
            self.scene_ui.sliders.remove(&key);
        }
        if !self.scene_ui.sliders.contains_key(&key) {
            let window_of = scrub(spec, index, value);
            let state = cx.new(|_| {
                SliderState::new()
                    .min(window_of.min as f32)
                    .max(window_of.max as f32)
                    .step(window_of.step as f32)
            });
            let slider_key = key.clone();
            cx.subscribe(&state, move |this, _, event: &SliderEvent, cx| {
                this.scene_slider_event(slider_key.clone(), event, cx);
            })
            .detach();
            self.scene_ui.sliders.insert(
                key.clone(),
                SliderEntry {
                    state,
                    scrub: window_of,
                },
            );
        }
        let Some(entry) = self.scene_ui.sliders.get(&key) else {
            return div().into_any_element();
        };
        let (state, bounds) = (entry.state.clone(), entry.scrub);
        // `set_value` does not emit a change, so following the model cannot loop.
        let target = value as f32;
        if (state.read(cx).value().end() - target).abs() > SLIDER_EPSILON {
            state.update(cx, |slider, cx| slider.set_value(target, window, cx));
        }
        let element = key.element_id();
        let (min, max, step) = (bounds.min, bounds.max, bounds.step);
        div()
            .id(SharedString::from(format!("scene-slider-{element}")))
            .relative()
            .child(self.geometry.probe(format!("control:{element}")))
            .focusable()
            .flex_1()
            // Capture phase: the Kit thumb stops its own mouse-down.
            .capture_any_mouse_down(scene_slider_press(cx))
            .on_key_down(scene_slider_keys(cx, key.clone(), move |pressed, shift| {
                value_after_key(pressed, shift, value, min, max, step)
                    .map(|next| quantize(next, min, step))
            }))
            .on_key_up(scene_slider_key_up(cx, key, move |released| {
                value_after_key(released, false, value, min, max, step).is_some()
            }))
            .child(
                Slider::new(&state)
                    .horizontal()
                    .accessibility_label(label.to_string()),
            )
            .into_any_element()
    }
}
