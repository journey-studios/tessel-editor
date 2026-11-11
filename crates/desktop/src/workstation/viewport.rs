//! The viewport surface inside the monitor panel.
//!
//! The picture comes from one persistent WebView running the Three.js runtime
//! ([`super::viewport_host`]); this file places it in the monitor, keeps it hidden whenever the
//! monitor is not on screen, and moves messages between the WebView and the host. The monitor
//! still owns the header and transport. When the runtime bundle is missing or the runtime
//! closes, the pane says so with the command that fixes it instead of showing a stale or fake
//! picture.

use gpui_kit::assets::IconName;
use gpui_kit::*;

use std::time::{Duration, Instant};

use gpui_webview::WebView;

use super::controls::glyph;
use super::tokens::Tokens;
use super::viewport_host::{FrameWaiter, ORIGIN, PAGE_URL, SCHEME, ViewportHost};
use super::{Geometry, Workstation};
use crate::state::layout::PanelKind;
use tessel_editor_core::CommandError;
use tessel_editor_core::ops::RenderFrame;
use tessel_editor_core::trace::{ms, round3};

/// What the viewport pane shows.
pub enum ViewportContent {
    /// The live runtime.
    Web(Entity<WebView>),
    /// No runtime to show: why, and what to do about it.
    Placeholder { icon: IconName, text: String },
}

/// The bridge message type a queued script carries, for the trace only (no name of anything private is read).
fn script_kind(script: &str) -> &'static str {
    [
        "frame.render",
        "frame.warm",
        "frame.cancel",
        "asset.load",
        "asset.release",
        "asset.proxy",
        "hello",
    ]
    .into_iter()
    .find(|kind| script.contains(&format!("\"type\":\"{kind}\"")))
    .unwrap_or("other")
}

const TICK: Duration = Duration::from_millis(500);
/// Slightly past the session's settle time, so the first check usually finds it settled.
const REFINE_WAIT: Duration = Duration::from_millis(220);

impl Workstation {
    /// Creates the persistent WebView (once) and the tasks that serve it. Called from
    /// `Workstation::new`, the only place that has the window the native view attaches to.
    pub(super) fn build_viewport(
        viewport: &mut ViewportHost,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<WebView>> {
        if !viewport.has_page() {
            return None;
        }
        // The stamp is the moment the WebView handed the message over (`None` with the trace off): how long it then waits
        // for the window's task is what the trace calls `ipc_wait` (BL157.2).
        let (sender, receiver) = async_channel::unbounded::<(String, Option<Instant>)>();
        let handle = viewport.protocol_handle();
        let ipc_trace = viewport.trace();
        let builder = wry::WebViewBuilder::new()
            // Asynchronous at the WebView, whichever way the handle reads: `sync` answers before it returns (what the
            // synchronous protocol did), `async` answers from a worker (BL157.1, `TESSEL_VIEWPORT_ASSET_IO`).
            .with_asynchronous_custom_protocol(SCHEME.to_string(), move |_, request, responder| {
                handle.handle(
                    &request,
                    Box::new(move |response| responder.respond(response)),
                );
            })
            .with_ipc_handler(move |request| {
                // Unbounded: the page posts at most a few messages per frame, and dropping one
                // would desynchronize the request/answer pairing.
                let _ = sender.try_send((request.body().clone(), ipc_trace.start()));
            })
            // The page is ours and nothing else: no navigation away, no pop-ups, no downloads.
            .with_navigation_handler(|url| url.starts_with(ORIGIN))
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_background_color((0, 0, 0, 255))
            .with_url(PAGE_URL);
        let view = match WebView::build(builder, window, cx) {
            Ok(view) => view,
            Err(error) => {
                viewport.on_script_error(&format!("cannot create the WebView: {error}"));
                return None;
            }
        };
        let entity = cx.new(|_| view);
        cx.spawn(async move |this, cx| {
            while let Ok((raw, arrived)) = receiver.recv().await {
                if this
                    .update(cx, |this, cx| this.viewport_ipc_arrived(&raw, arrived, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |this, cx| this.viewport_tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Some(entity)
    }

    /// Runs the scripts the host queued. A page that refuses one is reported, not retried.
    fn viewport_send(&mut self, cx: &mut Context<Self>) {
        let scripts = self.viewport.take_outgoing();
        if let Some(view) = self.viewport_view.clone() {
            let trace = self.viewport.trace();
            for script in scripts {
                // Handing the script to the WebView (it returns before the page runs it).
                let started = trace.start();
                let result = match started {
                    // With the trace on, the WebView also says when the page finished running the script (its completion
                    // handler, back on this thread): send to done, on this clock, is what delivery to the page and the
                    // page's own handling of the message cost together (BL157.2).
                    Some(sent) => {
                        let done = trace.clone();
                        let kind = script_kind(&script);
                        view.read(cx).evaluate_script_with_callback(&script, move |_| {
                            done.event("eval_done", || {
                                serde_json::json!({ "kind": kind, "ms": round3(ms(sent.elapsed())) })
                            });
                        })
                    }
                    None => view.read(cx).evaluate_script(&script),
                };
                trace.span("evaluate", started);
                if let Err(error) = result {
                    self.viewport.on_script_error(&error.to_string());
                    break;
                }
            }
        }
    }

    /// [`viewport_send`](Self::viewport_send) and a repaint, for what the user did.
    pub(super) fn viewport_flush(&mut self, cx: &mut Context<Self>) {
        self.viewport_send(cx);
        cx.notify();
    }

    /// Follows the editor state after a command: a change the user made, so the retries of an earlier timeout
    /// start over (BL158).
    pub(super) fn viewport_sync(&mut self, cx: &mut Context<Self>) {
        self.viewport.note_change();
        self.viewport_follow(cx);
    }

    /// Asks the monitor for its picture again (a directed scene or the sequence preview, whichever it shows) without
    /// starting the retries over: what a retry does, and what a change does after counting itself as one.
    fn viewport_follow(&mut self, cx: &mut Context<Self>) {
        // A gizmo drag that nobody can finish is over before the monitor follows the state.
        self.settle_gizmo_drag(cx);
        if let Some(scene) = self.viewport.directed_scene().map(str::to_owned) {
            let gesture = self
                .state
                .scene_interaction
                .gesture()
                // A gesture the core can no longer apply (an external edit, an undo, a swapped
                // scene) stays in its slot until cancelled or replaced, and asking the plan for
                // it is `gesture_invalidated`: the monitor must show the document then, not a
                // failure that nothing in the panel explains (the panel ignores it too).
                .filter(|gesture| {
                    gesture.scene() == scene
                        && gesture
                            .invalidation(&self.state.scenes, self.state.project.revision)
                            .is_none()
                })
                .map(|gesture| gesture.id().to_owned());
            let frame = self.state.scene_frame_at_playhead(&scene);
            let shown = frame.and_then(|frame| {
                let request = serde_json::json!({ "scene": scene, "frame": frame,
                    "gesture": gesture });
                self.viewport.render_scene(
                    &mut self.state,
                    &self.workflow.scene_resources(),
                    &request,
                    false,
                )
            });
            if let Err(error) = shown {
                self.viewport.scene_error(&error);
            }
        } else {
            self.viewport.sync(&self.state);
        }
        // F055: the gizmo the page draws over the picture follows the same state.
        self.overlay_refresh();
        self.viewport_flush(cx);
    }

    #[cfg(test)]
    pub(super) fn viewport_ipc(&mut self, raw: &str, cx: &mut Context<Self>) {
        self.viewport_ipc_arrived(raw, None, cx);
    }

    /// [`viewport_ipc`](Self::viewport_ipc) for a message the WebView handed over at `arrived` (trace on).
    fn viewport_ipc_arrived(
        &mut self,
        raw: &str,
        arrived: Option<Instant>,
        cx: &mut Context<Self>,
    ) {
        let waited = arrived.map(|at| at.elapsed());
        // F055: the overlay's own messages never reach the render session.
        if let Ok(message) = serde_json::from_str::<serde_json::Value>(raw)
            && self.overlay_ipc(&message, cx)
        {
            self.viewport_send(cx);
            return;
        }
        let before = self.viewport.ui_signature();
        self.viewport.on_ipc_waited(raw, &self.state, waited);
        // The page may have said it is gone (lost context, runtime error, reload).
        self.settle_gizmo_drag(cx);
        let restarted = serde_json::from_str::<serde_json::Value>(raw)
            .ok()
            .is_some_and(|message| {
                matches!(message["type"].as_str(), Some("ready" | "context.restored"))
            });
        if restarted && self.viewport.directed_scene().is_some() {
            self.viewport_sync(cx);
        }
        self.viewport_send(cx);
        // OPT04: a message that leaves the header and the error area as they were (a presentation
        // report, an asset or proxy acknowledgement) has nothing to repaint.
        let repainted = self.viewport.ui_signature() != before;
        self.viewport.note_ui(repainted);
        if repainted {
            cx.notify();
        }
        self.arm_refine(cx);
    }

    /// A proxy picture is on screen: after the scrub settles, draw the original in its place.
    /// A one-shot timer instead of the 500 ms tick, so an idle monitor wakes nothing.
    fn arm_refine(&mut self, cx: &mut Context<Self>) {
        if self.viewport_refine_armed || !self.viewport.refine_pending() {
            return;
        }
        self.viewport_refine_armed = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFINE_WAIT).await;
            let _ = this.update(cx, |this, cx| this.viewport_refine(cx));
        })
        .detach();
    }

    fn viewport_refine(&mut self, cx: &mut Context<Self>) {
        self.viewport_refine_armed = false;
        if self.viewport.refine(&self.state, Instant::now()) {
            self.viewport_flush(cx);
        }
        // Not settled yet (or a frame was still in flight): ask again, it stops when nothing is left.
        self.arm_refine(cx);
    }

    fn viewport_tick(&mut self, cx: &mut Context<Self>) {
        self.settle_gizmo_drag(cx);
        let now = Instant::now();
        if self.viewport.tick(now) {
            cx.notify();
        }
        // A frame that timed out is asked for again after a pause (BL158), by the path a change takes but
        // counted as a retry (a directed scene is asked for by the same path as the sequence preview).
        if self.viewport.retry_due(now) {
            self.viewport_follow(cx);
        }
    }

    /// An explicit `render.frame`; the answer comes after the monitor drew the frame.
    pub fn viewport_render_frame(
        &mut self,
        frame: &RenderFrame,
        cx: &mut Context<Self>,
    ) -> Result<FrameWaiter, CommandError> {
        let waiter = self.viewport.render_frame(&self.state, frame);
        self.viewport_flush(cx);
        waiter
    }

    /// Reloads the page, for a runtime that closed or a page that never started.
    pub fn viewport_reload(&mut self, cx: &mut Context<Self>) -> bool {
        let reloaded = self.viewport.begin_reload();
        self.settle_gizmo_drag(cx);
        if let Some(view) = self.viewport_view.clone() {
            view.update(cx, |view, _| view.load_url(PAGE_URL));
        }
        cx.notify();
        reloaded
    }

    /// The project was replaced by an operation.
    pub fn viewport_reset_project(&mut self, cx: &mut Context<Self>) {
        // A clock running over the old project would seek the new one.
        self.playback_stop(cx);
        // Selections, gestures and sliders of the old project's scenes do not carry over.
        self.scene_project_changed();
        self.viewport.reset_project();
        self.viewport_sync(cx);
    }

    /// A native view paints above every GPUI element, so it must be hidden explicitly
    /// whenever the monitor is closed, collapsed or behind another tab.
    pub(super) fn update_viewport_visibility(&mut self, cx: &mut Context<Self>) {
        let layout = self.layout_snapshot(cx);
        // A dialog is a GPUI element, and the native view paints above every one of them, so
        // while one is open the WebView must be hidden or it would cover the dialog.
        let dialog_open = self.workflow.has_dialog();
        let on_screen = !dialog_open
            && layout.is_visible(PanelKind::Monitor)
            && layout
                .locate(PanelKind::Monitor)
                .is_some_and(|place| place.active);
        if on_screen == self.viewport.on_screen {
            return;
        }
        self.viewport.on_screen = on_screen;
        if on_screen {
            self.viewport_sync(cx);
        } else {
            // A native view that is hidden gets no pointer-up: an open gizmo drag ends here.
            self.cancel_gizmo_drag(
                tessel_editor_core::scene_interaction::CancelReason::Hidden,
                cx,
            );
            self.overlay_refresh();
            self.viewport_send(cx);
        }
        if let Some(view) = self.viewport_view.clone() {
            view.update(cx, |view, _| {
                let _ = view.set_visible(on_screen);
            });
        }
    }
}

/// The framed picture area. `flex_1` so the monitor decides its size, which is
/// also the box the native WebView follows when it resizes.
pub fn render_viewport(
    t: &Tokens,
    geometry: &Geometry,
    content: ViewportContent,
) -> impl IntoElement + use<> {
    // The rectangle the native view covers: the debug meter keeps its card out of it (F049).
    let frame = div()
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .flex_1()
        .min_h(px(0.))
        .m(px(8.))
        .rounded(px(3.))
        .overflow_hidden()
        .bg(t.app)
        .border_1()
        .border_color(t.border)
        .child(geometry.probe("viewport.frame"));
    match content {
        ViewportContent::Web(view) => frame.child(view),
        ViewportContent::Placeholder { icon, text } => frame.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.))
                .text_size(px(11.))
                .text_color(t.dim)
                .child(glyph(icon, 24.0, t.faint))
                .child(text),
        ),
    }
}
