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

use super::Workstation;
use super::controls::glyph;
use super::tokens::Tokens;
use super::viewport_host::{FrameWaiter, PAGE_URL, SCHEME, ViewportHost};
use crate::state::layout::PanelKind;
use tessel_editor_core::CommandError;
use tessel_editor_core::ops::RenderFrame;

/// What the viewport pane shows.
pub enum ViewportContent {
    /// The live runtime.
    Web(Entity<WebView>),
    /// No runtime to show: why, and what to do about it.
    Placeholder { icon: IconName, text: String },
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
        let (sender, receiver) = async_channel::unbounded::<String>();
        let handle = viewport.protocol_handle();
        let builder = wry::WebViewBuilder::new()
            .with_custom_protocol(SCHEME.to_string(), move |_, request| {
                handle.respond(&request)
            })
            .with_ipc_handler(move |request| {
                // Unbounded: the page posts at most a few messages per frame, and dropping one
                // would desynchronize the request/answer pairing.
                let _ = sender.try_send(request.body().clone());
            })
            // The page is ours and nothing else: no navigation away, no pop-ups, no downloads.
            .with_navigation_handler(|url| url.starts_with(&format!("{SCHEME}://")))
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
            while let Ok(raw) = receiver.recv().await {
                if this
                    .update(cx, |this, cx| this.viewport_ipc(&raw, cx))
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
                let result = view.read(cx).evaluate_script(&script);
                trace.span("evaluate", started);
                if let Err(error) = result {
                    self.viewport.on_script_error(&error.to_string());
                    break;
                }
            }
        }
    }

    /// [`viewport_send`](Self::viewport_send) and a repaint, for what the user did.
    fn viewport_flush(&mut self, cx: &mut Context<Self>) {
        self.viewport_send(cx);
        cx.notify();
    }

    /// Follows the editor state after a command.
    pub(super) fn viewport_sync(&mut self, cx: &mut Context<Self>) {
        self.viewport.sync(&self.state);
        self.viewport_flush(cx);
    }

    fn viewport_ipc(&mut self, raw: &str, cx: &mut Context<Self>) {
        let before = self.viewport.ui_signature();
        self.viewport.on_ipc(raw, &self.state);
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
        if self.viewport.tick(Instant::now()) {
            cx.notify();
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
        if let Some(view) = self.viewport_view.clone() {
            view.update(cx, |view, _| {
                let _ = view.set_visible(on_screen);
            });
        }
    }
}

/// The framed picture area. `flex_1` so the monitor decides its size, which is
/// also the box the native WebView follows when it resizes.
pub fn render_viewport(t: &Tokens, content: ViewportContent) -> impl IntoElement + use<> {
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
        .border_color(t.border);
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
