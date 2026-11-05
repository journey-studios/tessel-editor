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

use super::controls::{
    disabled_tool_button, glyph, property_row, quantize, tool_button, value_after_key,
};
use super::scene_gesture::{SliderKey, scene_slider_key_up, scene_slider_keys, scene_slider_press};
use super::scene_model::{
    Availability, PropertyView, Snapshot, Target, TargetView, TreeEntry, component_values,
    components, format_number, scrub, unit_label,
};
use super::scene_ui::{InspectorMode, NO_SCENE_REASON, SliderEntry, depth_of};
use super::tokens::Tokens;
use super::{Source, Workstation};

/// Tree row height of the Kit's default list item.
const TREE_ROW: f32 = 28.0;
/// The tree never grows past this: a large scene scrolls inside it instead of pushing the
/// properties off the panel.
const TREE_MAX_HEIGHT: f32 = 240.0;
/// Slider state is `f32`; anything closer than this is the same value.
const SLIDER_EPSILON: f32 = 1e-4;

fn entity_icon(kind: &str) -> IconName {
    match kind {
        "camera" => IconName::Film,
        "shot" => IconName::ChartGantt,
        "track" => IconName::AudioLines,
        _ => IconName::Layers,
    }
}

impl Workstation {
    /// The inspector with its mode bar: the clip inspector of the timeline, or the scene one.
    pub(super) fn render_inspector(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mode = self.scene_ui.mode;
        let workstation = cx.weak_entity();
        let bar = TabBar::new("inspector-mode")
            .pill()
            .selected_index(
                InspectorMode::ALL
                    .iter()
                    .position(|m| *m == mode)
                    .unwrap_or(0),
            )
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
                .render_scene_inspector(t, window, cx)
                .into_any_element(),
        };
        v_flex()
            .relative()
            .size_full()
            .min_h(px(0.))
            .bg(t.panel)
            .child(self.geometry.probe("region:inspector"))
            .child(div().flex_shrink_0().px(px(8.)).py(px(4.)).child(bar))
            .child(div().flex_1().min_h(px(0.)).child(body))
    }

    fn render_scene_inspector(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let snapshot = self.scene_snapshot();
        self.sync_scene_controls(snapshot.as_deref(), window, cx);
        let monitor = self.scene_monitor();

        let mut content = v_flex().w_full().gap(px(6.));
        content = content.child(self.scene_monitor_note(t, monitor.badge(), monitor.reason(), cx));
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
        content = content.child(self.scene_tree_box(t, &snapshot, cx));
        if let Some(notice) = self.scene_notice(t) {
            content = content.child(notice);
        }
        if let Some(status) = self.scene_gesture_status(t, &snapshot, cx) {
            content = content.child(status);
        }
        content = content.child(self.scene_target(t, &snapshot, window, cx));
        self.scene_scroll(t, content)
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

    /// Says the monitor is not integrated and offers the button that will show the scene there.
    /// The button is inert (with its reason) until a real renderer exists; see `scene_monitor.rs`.
    fn scene_monitor_note(
        &self,
        t: &Tokens,
        badge: &str,
        reason: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        h_flex()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(4.))
            .text_size(px(10.))
            .text_color(t.faint)
            .child(glyph(IconName::Info, 12.0, t.faint))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .child(badge.to_string()),
            )
            .child(if self.scene_monitor().is_available() {
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
            })
    }

    fn scene_tree_box(
        &mut self,
        t: &Tokens,
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
        let rows = visible.len().max(1);
        let workstation = cx.weak_entity();
        let tokens = *t;
        let list = tree(&self.scene_ui.tree, move |_, entry, selected, _, _| {
            let path = entry.item().id.to_string();
            let icon = kinds
                .get(&path)
                .map_or(IconName::Layers, |kind| entity_icon(kind));
            let workstation = workstation.clone();
            let target = path.clone();
            ListItem::new(SharedString::from(format!("scene-entity:{path}")))
                .child(
                    h_flex()
                        .w_full()
                        .gap(px(6.))
                        .items_center()
                        .pl(px(entry.depth() as f32 * 12.))
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
                    .flex_shrink_0()
                    .h(px((rows as f32 * TREE_ROW).min(TREE_MAX_HEIGHT)))
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
    /// leaves one open on purpose): the candidate is on screen and can be cancelled.
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
                .items_center()
                .gap(px(6.))
                .mx(px(10.))
                .text_size(px(11.))
                .text_color(t.dim)
                .child(
                    div()
                        .id("scene-gesture-status")
                        .flex_1()
                        .truncate()
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
                .scene_properties(t, snapshot, target, window, cx)
                .into_any_element(),
        };
        div().w_full().child(body)
    }

    fn scene_properties(
        &mut self,
        t: &Tokens,
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

        let mut list = v_flex().w_full().pb(px(8.)).child(
            div()
                .px(px(10.))
                .pt(px(6.))
                .pb(px(2.))
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
            .child(div().child(property.label.to_string()))
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
            let label = if components(spec.kind).len() == 1 {
                property.label.to_string()
            } else {
                format!("{} {}", property.label, component.label)
            };
            let control = self.scene_slider(key, &label, property, value, window, cx);
            block = block.child(property_row(
                t,
                component.label,
                property.is_modified(),
                true,
                control,
                Some(format_number(value)),
                unit,
            ));
        }
        block
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
