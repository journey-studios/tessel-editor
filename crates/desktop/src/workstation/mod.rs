//! The live editor surface: library, single monitor, timeline and inspector,
//! hosted as panels of the Kit's `DockArea`.
//!
//! `Workstation` owns the only copy of `EditorState`; gestures and the
//! automation server both call [`Workstation::execute`]. The dock layout is
//! not copied here: it lives in the `DockArea` and is read on demand
//! ([`Workstation::layout_snapshot`]) and edited through
//! [`Workstation::execute_layout`].

mod controls;
mod dock;
mod drag;
mod env_mode;
mod flows;
mod geometry;
mod inspector_panel;
mod library_panel;
mod media;
mod monitor;
mod ops;
mod perf_history_view;
mod perf_meter;
mod playback;
mod playback_poll;
mod scene_fx;
mod scene_fx_editors;
mod scene_fx_handles;
mod scene_fx_keys;
mod scene_gesture;
mod scene_gizmo;
mod scene_import;
mod scene_model;
mod scene_monitor;
mod scene_panel;
mod scene_ui;
mod slider_gesture;
mod status_bar;
mod timeline_panel;
mod tokens;
mod viewport;
mod viewport_host;
mod viewport_protocol;
mod waveform;
mod workflow;

use std::collections::HashMap;

use gpui_kit::component::IndexPath;
use gpui_kit::component::dock::{DockEvent, DockToggleEvent};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{SelectEvent, SelectState};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::tree::{TreeEvent, TreeState};
use gpui_kit::*;
use serde_json::{Value, json};

use self::dock::DockHost;
use self::drag::{ClipPreview, Preview, TrackPreview, frame_at, insertion_at, row_at};
pub use self::geometry::Geometry;
pub use self::ops::OpReply;
pub use self::perf_meter::{install_perf_meter, shortcut_label};
use self::playback::PlaybackState;
use self::scene_ui::SceneUi;
use self::slider_gesture::SliderGesture;
pub use self::tokens::Tokens;
pub use self::viewport_host::FrameWaiter;
use self::viewport_host::ViewportHost;
use self::workflow::Workflow;
use crate::state::inspector::{PROPERTIES, PropertyKind};
use crate::state::layout::{Layout, LayoutCommand, PanelKind};
use crate::state::timeline::{Placement as TrackPlacement, ZOOM_MAX, ZOOM_MIN};
use crate::state::tree::{self, TreeContext};
use crate::state::{Command, CommandError, EditorState, Raw};
use gpui_webview::WebView;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Gesture,
    Cli,
}

/// The last thing that happened, shown in the status bar so a CLI command is
/// visible in the window it changed.
pub struct Activity {
    pub source: Source,
    pub text: String,
    pub error: bool,
}

type OptionSelect = Entity<SelectState<SearchableVec<&'static str>>>;

pub struct Workstation {
    pub state: EditorState,
    pub geometry: Geometry,
    dock: DockHost,
    search: Entity<InputState>,
    /// The text input of the inspector's "Texto" section (title clips). Like the search box it
    /// follows the model: `title_text_shown` is the `(clip, content)` last pushed into it, so the
    /// model is written back only when it changed, never over what is being typed.
    title_text: Entity<InputState>,
    title_text_shown: Option<(String, String)>,
    sliders: HashMap<&'static str, Entity<SliderState>>,
    selects: HashMap<&'static str, OptionSelect>,
    library_tree: Entity<TreeState>,
    zoom_slider: Entity<SliderState>,
    /// The sequence selector of the timeline toolbar. Like every Kit control it follows the model
    /// (`sequence_choices` is what it currently lists) and only dispatches `ActivateSequence`.
    sequence_select: Entity<SelectState<SearchableVec<String>>>,
    sequence_choices: Vec<(String, String)>,
    /// Folder IDs, expansion and selection last pushed into `library_tree`, so
    /// the tree is only rebuilt when the model really changed (rebuilding
    /// resets the tree's own selection).
    library_signature: Vec<(String, bool)>,
    pub preview: Option<Preview>,
    /// Pointer offset inside the dragged clip, known only when the drag starts.
    clip_grab_px: f32,
    pub activity: Option<Activity>,
    /// Request/answer coordinator of the Three.js runtime (no GPUI types inside).
    viewport: ViewportHost,
    /// The one WebView, created in `new` and never rebuilt; `None` when its page is missing.
    viewport_view: Option<Entity<WebView>>,
    /// A timer is already waiting to replace a proxy picture with the original.
    viewport_refine_armed: bool,
    /// File/import/package/export dialogs, jobs and autosave (see `workflow.rs`).
    workflow: Workflow,
    /// The play clock and its device audio (see `playback.rs`).
    playback: PlaybackState,
    /// The property-slider interaction in progress (see `slider_gesture.rs`). Input state of
    /// the adapter only: the gesture and its history entry live in `EditorState`.
    slider_gesture: SliderGesture,
    /// The directed-scene inspector's window state (see `scene_ui.rs`): presentation only, the
    /// scene, its selection and its gestures live in `EditorState`.
    scene_ui: SceneUi,
    /// The debug performance meter (F049): off until asked for, and while off it keeps nothing.
    perf: perf_meter::PerfState,
    focus: FocusHandle,
}

impl Workstation {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Buscar mídia"));
        cx.subscribe(&search, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.dispatch(Command::SetMediaSearch { query }, cx);
            }
        })
        .detach();

        let title_text = cx.new(|cx| InputState::new(window, cx).placeholder("Texto do título"));
        // Enter or leaving the field commits ONE `SetClipTextField` (one undo entry), never one
        // per keystroke; a refused text (a character the font cannot draw) puts the model's back.
        cx.subscribe_in(
            &title_text,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.commit_title_text(input, cx);
                }
            },
        )
        .detach();

        let mut sliders = HashMap::new();
        let mut selects = HashMap::new();
        for property in PROPERTIES {
            let id = property.id;
            match property.kind {
                PropertyKind::Slider { min, max, step, .. } => {
                    let slider = cx.new(|_| {
                        SliderState::new()
                            .min(min as f32)
                            .max(max as f32)
                            .step(step as f32)
                    });
                    // WHY not `SetProperty` per `Change`: a drag emits dozens of values and
                    // each one used to become an undo entry. The adapter turns the drag into
                    // one gesture (live preview, one entry on release); see `slider_gesture`.
                    cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
                        this.slider_event(id, event, cx);
                    })
                    .detach();
                    sliders.insert(id, slider);
                }
                PropertyKind::Select { options } => {
                    let select = cx.new(|cx| {
                        SelectState::new(
                            SearchableVec::new(options.to_vec()),
                            Some(IndexPath::new(0)),
                            window,
                            cx,
                        )
                    });
                    cx.subscribe(
                        &select,
                        move |this, _, event: &SelectEvent<SearchableVec<&'static str>>, cx| {
                            let SelectEvent::Confirm(Some(choice)) = event else {
                                return;
                            };
                            this.dispatch(
                                Command::SetProperty {
                                    property: id.to_string(),
                                    value: Raw::Text((*choice).to_string()),
                                },
                                cx,
                            );
                        },
                    )
                    .detach();
                    selects.insert(id, select);
                }
                PropertyKind::Toggle => {}
            }
        }

        let zoom_slider = cx.new(|_| {
            SliderState::new()
                .min(ZOOM_MIN as f32)
                .max(ZOOM_MAX as f32)
                .step(1.0)
        });
        cx.subscribe(&zoom_slider, |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                this.dispatch(
                    Command::SetTimelineZoom {
                        zoom: Raw::Number(value.end().into()),
                    },
                    cx,
                );
            }
        })
        .detach();

        let sequence_select = cx
            .new(|cx| SelectState::new(SearchableVec::new(Vec::<String>::new()), None, window, cx));
        cx.subscribe(
            &sequence_select,
            |this, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                let SelectEvent::Confirm(Some(label)) = event else {
                    return;
                };
                let Some((id, _)) = this
                    .sequence_choices
                    .iter()
                    .find(|(_, choice)| choice == label)
                else {
                    return;
                };
                let sequence = id.clone();
                this.dispatch(Command::ActivateSequence { sequence }, cx);
            },
        )
        .detach();

        let library_tree = cx.new(|cx| TreeState::new(cx));
        cx.subscribe(&library_tree, |this, _, event: &TreeEvent, cx| {
            let (folder, expanded) = match event {
                TreeEvent::Expanded(id) => (id.to_string(), true),
                TreeEvent::Collapsed(id) => (id.to_string(), false),
            };
            this.dispatch(Command::SetFolderExpanded { folder, expanded }, cx);
        })
        .detach();

        let dock = DockHost::new(cx.weak_entity(), window, cx);
        // Gestures (dragging a tab, resizing a divider, the collapse buttons)
        // change the area without going through `execute_layout`; repaint so
        // the header toggles and the CLI-facing summary follow.
        // A drag that empties a side dock leaves it open and blank; collapse it
        // like the CLI's move does. Runs after the edit settles, so a reset or
        // load that refills the dock never sees it closed half-way.
        cx.subscribe_in(&dock.area, window, |this, _, _: &DockEvent, window, cx| {
            this.dock.close_empty_docks(window, cx);
            cx.notify();
        })
        .detach();
        cx.subscribe(&dock.area, |_, _, _: &DockToggleEvent, cx| cx.notify())
            .detach();

        // The panels are separate views; any workstation change repaints them.
        let this = cx.entity();
        cx.observe(&this, |this, _, cx| {
            for panel in this.dock.views() {
                panel.update(cx, |_, cx| cx.notify());
            }
        })
        .detach();

        // A window that loses the focus mid-drag may never see the MouseUp (BL043): the slider
        // gesture is committed then, never left open.
        cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.slider_focus_lost(cx);
                this.scene_slider_focus_lost(cx);
                this.fx_focus_lost(cx);
                // F055: a release may never arrive; a gizmo drag is cancelled, never committed.
                this.cancel_gizmo_drag(
                    tessel_editor_core::scene_interaction::CancelReason::Blur,
                    cx,
                );
            }
        })
        .detach();

        // GPUI's test windows have no native view to attach to, so tests exercise the host
        // through its own unit tests and see the viewport as unavailable here.
        let page = if cfg!(test) {
            Err(tessel_editor_render::SetupError::BundleMissing(
                "runtime/dist (WebView disabled in tests)".into(),
            ))
        } else {
            tessel_editor_render::discover_page()
        };
        let scene_ui = SceneUi::new(window, cx);
        let mut viewport = ViewportHost::new(page);
        let perf = perf_meter::PerfState::new(cx);
        viewport.attach_meter(perf.meter(), perf.clock());
        cx.set_global(perf_meter::PerfTarget(cx.weak_entity()));
        let viewport_view = Self::build_viewport(&mut viewport, window, cx);

        if !cfg!(test) {
            // Tests drive `workflow_tick` themselves: a timer loop would never let them park.
            Self::spawn_workflow_tick(cx);
        }

        Self {
            // The demonstration project is an explicit bootstrap, not the default state.
            state: EditorState::demo(),
            geometry: Geometry::default(),
            dock,
            search,
            title_text,
            title_text_shown: None,
            sliders,
            selects,
            library_tree,
            zoom_slider,
            sequence_select,
            sequence_choices: Vec::new(),
            library_signature: Vec::new(),
            preview: None,
            clip_grab_px: 0.0,
            activity: None,
            viewport,
            viewport_view,
            viewport_refine_armed: false,
            workflow: Workflow::new(),
            playback: PlaybackState::from_env(),
            slider_gesture: SliderGesture::default(),
            scene_ui,
            perf,
            focus: cx.focus_handle(),
        }
    }

    /// Gives keyboard focus to the workstation root so Escape reaches it.
    pub fn focus_root(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    /// Runs a command against the shared state. Every path (pointer, keyboard,
    /// CLI) ends here.
    pub fn execute(
        &mut self,
        command: Command,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let ordinary_preview = matches!(command, Command::SetPreview { .. });
        let moved_playhead = matches!(
            command,
            Command::SetPlayhead { .. }
                | Command::Scene(tessel_editor_core::scene_project::SceneCommand::FxSeek { .. })
        );
        let result = self.state.execute(command);
        if result.is_ok() && moved_playhead {
            // The history's event line: a gesture or `ui set` moved the playhead on purpose.
            self.perf_playhead_moved();
        }
        if result.is_ok() && ordinary_preview {
            self.viewport.leave_directed();
        }
        if let Ok(outcome) = &result
            && outcome.get("overlay").is_some()
        {
            // F055: a gizmo command answered with the overlay of the state it left; the sync below
            // reuses it instead of planning the scene again for the same state.
            self.overlay_adopt(outcome);
        }
        if result.is_ok() {
            // After a failed command the state is untouched, so the picture stays as it was.
            self.viewport_sync(cx);
        }
        self.activity = Some(match &result {
            Ok(outcome) => Activity {
                source,
                text: describe(outcome),
                error: false,
            },
            Err(error) => Activity {
                source,
                text: error.to_string(),
                error: true,
            },
        });
        cx.notify();
        result
    }

    pub fn dispatch(&mut self, command: Command, cx: &mut Context<Self>) {
        let _ = self.execute(command, Source::Gesture, cx);
    }

    // ---- dock ----

    pub fn layout_snapshot(&self, cx: &App) -> Layout {
        self.dock.snapshot(cx)
    }

    /// Validates and applies a dock command to the live `DockArea`. A rejected
    /// command leaves the layout exactly as it was.
    pub fn execute_layout(
        &mut self,
        command: LayoutCommand,
        source: Source,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let layout = self.layout_snapshot(cx);
        let result = layout
            .validate(&command)
            .and_then(|()| self.dock.apply(&command, window, cx));
        self.activity = Some(match &result {
            Ok(outcome) => Activity {
                source,
                text: describe(outcome),
                error: false,
            },
            Err(error) => Activity {
                source,
                text: error.to_string(),
                error: true,
            },
        });
        cx.notify();
        result
    }

    /// The live layout plus the Kit's own dump, for `ui dock`.
    pub fn dock_report(&self, cx: &App) -> Value {
        let mut report = self.layout_snapshot(cx).to_json();
        report["dump"] = self.dock.dump(cx);
        report["id"] = json!("dock");
        report
    }

    // ---- automation tree ----

    fn with_tree<R>(&self, cx: &App, read: impl FnOnce(&EditorState, &TreeContext) -> R) -> R {
        let metrics = self.geometry.regions();
        let layout = self.layout_snapshot(cx);
        read(
            &self.state,
            &TreeContext {
                theme: crate::theme::scheme_name(cx),
                metrics: &metrics,
                layout: &layout,
                perf_meter: self.perf.visible(),
            },
        )
    }

    pub fn tree_nodes(&self, cx: &App) -> Vec<Value> {
        let mut nodes = self.with_tree(cx, tree::nodes);
        for node in &mut nodes {
            self.adopt_viewport(node);
            self.adopt_playback(node);
            self.adopt_scene(node);
            self.adopt_perf(node);
            self.adopt_perf_history(node);
        }
        // The scene inspector and the scene monitor seam are window state too.
        nodes.extend(self.scene_nodes());
        // The viewport state lives in the window, not in `EditorState`.
        nodes.push(self.viewport_node());
        // Flows, dialog fields and jobs: window state the core cannot describe.
        nodes.extend(self.workflow_nodes(cx));
        nodes
    }

    pub fn tree_node(&self, id: &str, cx: &App) -> Option<Value> {
        if id == "monitor.viewport" {
            return Some(self.viewport_node());
        }
        if let Some(node) = self.scene_node(id) {
            return Some(node);
        }
        if let Some(node) = self
            .workflow_nodes(cx)
            .into_iter()
            .find(|node| node["id"] == id)
        {
            return Some(node);
        }
        let mut node = self.with_tree(cx, |state, context| tree::find(state, context, id))?;
        self.adopt_viewport(&mut node);
        self.adopt_playback(&mut node);
        self.adopt_scene(&mut node);
        self.adopt_perf(&mut node);
        self.adopt_perf_history(&mut node);
        Some(node)
    }

    /// The core cannot play (it has no clock); the window can, so its nodes say so.
    fn adopt_playback(&self, node: &mut Value) {
        match node["id"].as_str() {
            Some("monitor.play") => {
                node["enabled"] = json!(true);
                node.as_object_mut()
                    .map(|object| object.remove("disabled_reason"));
                node["label"] = json!(if self.playback.is_playing() {
                    "Pause"
                } else {
                    "Play"
                });
                node["state"] = self.playback.report();
            }
            Some("region:monitor") | Some("monitor") => {
                node["playback"] = self.playback.report();
            }
            _ => {}
        }
    }

    /// The core builds `region:monitor` without the viewport (it has no WebView to describe).
    fn adopt_viewport(&self, node: &mut Value) {
        if node["id"] == "region:monitor"
            && let Some(children) = node["children"].as_array_mut()
        {
            children.push(json!("monitor.viewport"));
        }
    }

    pub fn tree_actions(&self, cx: &App) -> Vec<Value> {
        self.with_tree(cx, tree::actions)
    }

    /// Draws one dockable region. The dock calls this through its panel, so a
    /// region keeps working wherever the user drops it.
    pub fn render_panel(
        &mut self,
        kind: PanelKind,
        tokens: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match kind {
            PanelKind::Library => self.render_library(tokens, window, cx).into_any_element(),
            PanelKind::Monitor => self.render_monitor(tokens, cx).into_any_element(),
            PanelKind::Timeline => self.render_timeline(tokens, window, cx).into_any_element(),
            PanelKind::Inspector => self.render_inspector(tokens, window, cx).into_any_element(),
        }
    }

    // ---- timeline drag gestures: previews only, state changes on drop ----

    #[cfg(test)]
    pub fn trim_gesture_points(
        &self,
        id: &str,
        frame: i64,
    ) -> Result<(Point<Pixels>, Point<Pixels>), String> {
        let handle = self.geometry.get(id).ok_or("Handle is not rendered")?;
        let content = self
            .geometry
            .get("timeline-content")
            .ok_or("Timeline is not rendered")?;
        let scroll = self
            .geometry
            .get("timeline-scroll")
            .ok_or("Timeline is not rendered")?;
        let start = handle.center();
        let end = point(
            content.origin.x + px(frame as f32 * self.state.timeline.pixels_per_frame()),
            start.y,
        );
        if !scroll.contains(&start) || !scroll.contains(&end) {
            return Err("Handle or destination is outside the visible timeline".into());
        }
        Ok((start, end))
    }

    pub fn update_trim_drag(
        &mut self,
        drag: &drag::TrimDrag,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(content) = self.geometry.get("timeline-content") else {
            return;
        };
        let Some(scroll) = self.geometry.get("timeline-scroll") else {
            return;
        };
        if !scroll.contains(&position) {
            self.cancel_drag(cx);
            return;
        }
        let frame = frame_at(
            f32::from(position.x),
            f32::from(content.origin.x),
            self.state.timeline.pixels_per_frame(),
            0.0,
        );
        self.preview = Some(Preview::Trim(drag::TrimPreview {
            clip: drag.clip,
            edge: drag.edge,
            frame,
            verdict: self.state.timeline.plan_trim(drag.clip, drag.edge, frame),
        }));
        cx.notify();
    }

    pub fn finish_trim_drag(&mut self, cx: &mut Context<Self>) {
        let Some(Preview::Trim(preview)) = self.preview.take() else {
            cx.notify();
            return;
        };
        self.dispatch(
            Command::TrimClip {
                clip: preview.clip.into(),
                edge: preview.edge,
                frame: preview.frame,
            },
            cx,
        );
    }

    pub fn begin_clip_drag(&mut self, grab_px: f32) {
        self.clip_grab_px = grab_px;
    }

    pub fn update_clip_drag(
        &mut self,
        clip: &'static str,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let preview = self.preview_clip_drop(clip, position);
        self.preview = preview.map(Preview::Clip);
        cx.notify();
    }

    fn lane_rows(&self) -> Vec<(&'static str, f32, f32)> {
        self.state
            .timeline
            .tracks
            .iter()
            .filter_map(|track| {
                let bounds = self.geometry.get(&format!("lane:{}", track.id))?;
                let top = f32::from(bounds.origin.y);
                Some((track.id.as_str(), top, top + f32::from(bounds.size.height)))
            })
            .collect()
    }

    fn preview_clip_drop(
        &self,
        clip: &'static str,
        position: Point<Pixels>,
    ) -> Option<ClipPreview> {
        let scroll = self.geometry.get("timeline-scroll")?;
        if !scroll.contains(&position) {
            return None;
        }
        let track = row_at(f32::from(position.y), &self.lane_rows())?;
        let content = self.geometry.get("timeline-content")?;
        let frame = frame_at(
            f32::from(position.x),
            f32::from(content.origin.x),
            self.state.timeline.pixels_per_frame(),
            self.clip_grab_px,
        );
        Some(ClipPreview {
            clip,
            track,
            requested_frame: frame,
            verdict: self
                .state
                .timeline
                .plan_move(clip, Some(track), Some(frame), None),
        })
    }

    pub fn finish_clip_drag(&mut self, cx: &mut Context<Self>) {
        let Some(Preview::Clip(preview)) = self.preview.take() else {
            cx.notify();
            return;
        };
        self.dispatch(
            Command::MoveClip {
                clip: preview.clip.to_string(),
                track: Some(preview.track.to_string()),
                frame: Some(preview.requested_frame),
                snap: None,
            },
            cx,
        );
    }

    pub fn update_track_drag(
        &mut self,
        track: &'static str,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let rows: Vec<(&'static str, f32, f32)> = self
            .state
            .timeline
            .tracks
            .iter()
            .filter_map(|entry| {
                let bounds = self.geometry.get(&format!("header:{}", entry.id))?;
                let top = f32::from(bounds.origin.y);
                Some((entry.id.as_str(), top, top + f32::from(bounds.size.height)))
            })
            .collect();
        let inside = self
            .geometry
            .get("timeline-headers")
            .is_some_and(|bounds| bounds.contains(&position));
        self.preview = inside
            .then(|| insertion_at(f32::from(position.y), &rows))
            .flatten()
            .filter(|(anchor, _): &(&'static str, TrackPlacement)| *anchor != track)
            .map(|(anchor, placement)| {
                Preview::Track(TrackPreview {
                    track,
                    anchor,
                    placement,
                    verdict: self.state.timeline.plan_reorder(track, anchor, placement),
                })
            });
        cx.notify();
    }

    pub fn finish_track_drag(&mut self, cx: &mut Context<Self>) {
        let Some(Preview::Track(preview)) = self.preview.take() else {
            cx.notify();
            return;
        };
        self.dispatch(
            Command::ReorderTrack {
                track: preview.track.to_string(),
                anchor: preview.anchor.to_string(),
                placement: preview.placement,
            },
            cx,
        );
    }

    /// Drops the drag preview without touching state: Escape, or a release
    /// that landed outside any valid target.
    pub fn cancel_drag(&mut self, cx: &mut Context<Self>) {
        if self.preview.take().is_some() {
            cx.notify();
        }
    }
}

fn describe(outcome: &Value) -> String {
    let target = outcome["target"].as_str().unwrap_or("state");
    let detail = [
        "value", "frame", "action", "visible", "open", "size", "sizes", "gesture",
    ]
    .iter()
    .find_map(|key| outcome.get(*key).filter(|value| !value.is_null()))
    .map(|value| match value.as_str() {
        Some(text) => text.to_string(),
        None => value.to_string(),
    });
    match detail {
        Some(detail) => format!("{target} → {detail}"),
        None => target.to_string(),
    }
}

impl Render for Workstation {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_viewport_visibility(cx);
        let t = Tokens::new(cx);
        let perf = self.render_perf(&t, cx);
        let strip = perf
            .strip
            .map(|text| self.perf_strip(&t, text, cx).into_any_element());
        let status = self.render_status(&t, strip);
        div()
            .id("workstation")
            .key_context("Workstation")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.))
            .bg(t.app)
            .text_color(t.text)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                // GPUI keeps a drag alive until it is stopped, so Escape both
                // ends the framework drag and drops our preview.
                if event.keystroke.key == "escape" {
                    cx.stop_active_drag(window);
                    this.cancel_drag(cx);
                    // A slider drag is a gesture too: Escape puts the original value back.
                    this.cancel_slider_gesture(cx);
                    this.cancel_scene_gesture(cx);
                    this.fx_cancel_drag(cx);
                    this.cancel_gizmo_drag(
                        tessel_editor_core::scene_interaction::CancelReason::Escape,
                        cx,
                    );
                }
            }))
            // A release over no drop target ends the drag with no change. A
            // successful drop stops propagation before this runs.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.cancel_drag(cx)),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .children(perf.area_probe)
                    .child(self.dock.area.clone())
                    .children(perf.overlay)
                    .children(perf.recorder),
            )
            .child(status)
    }
}

#[cfg(test)]
mod asset_io_tests;
#[cfg(test)]
mod audio_fades_tests;
#[cfg(test)]
mod camera_keys_tests;
#[cfg(test)]
mod dock_tests;
#[cfg(test)]
mod env_mode_tests;
#[cfg(test)]
mod identity_tests;
#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod perf_history_tests;
#[cfg(test)]
mod perf_tests;
#[cfg(test)]
mod playback_poll_tests;
#[cfg(test)]
mod playback_tests;
#[cfg(test)]
mod scene_fx_editors_tests;
#[cfg(test)]
mod scene_fx_handles_tests;
#[cfg(test)]
mod scene_fx_keys_tests;
#[cfg(test)]
mod scene_fx_tests;
#[cfg(test)]
mod scene_gizmo_perf_tests;
#[cfg(test)]
mod scene_gizmo_tests;
#[cfg(test)]
mod scene_light_tests;
#[cfg(test)]
mod scene_tests;
#[cfg(test)]
mod sharp_capture_tests;
#[cfg(test)]
mod slider_gesture_tests;
#[cfg(test)]
mod title_text_tests;
#[cfg(test)]
mod viewport_host_tests;
#[cfg(test)]
mod viewport_tests;
#[cfg(test)]
mod workflow_tests;

impl Drop for Workstation {
    fn drop(&mut self) {
        // The page cannot be told any more (the view dies with the window); this drops the
        // pending requests so nothing waits for an answer that will not come.
        self.viewport.shutdown();
    }
}
