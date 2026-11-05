#!/usr/bin/env python3
"""Instruments a COPY of the desktop host. Usage: patch-host.py <copy-root>. Refuses to touch the real repo."""
import os, sys
root = sys.argv[1]
if os.path.exists(os.path.join(root, ".git")): raise SystemExit("refusing: this is the real repository")
W = os.path.join(root, "crates/desktop/src/workstation")
def patch(path, pairs):
    text = open(path).read()
    if "vv_trace" in text: return
    for old, new in pairs:
        if old not in text: raise SystemExit(f"missing in {path}: {old[:70]}")
        text = text.replace(old, new, 1)
    open(path, "w").write(text)

open(os.path.join(W, "vv_trace.rs"), "w").write('''//! Validation-only spans (never in production): ring of (name, arg, epoch ms).
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
static RING: Mutex<Vec<(&'static str, u64, f64)>> = Mutex::new(Vec::new());
static JS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
pub fn now_ms() -> f64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0) }
pub fn mark(name: &'static str, arg: u64) { let t = now_ms(); if let Ok(mut r) = RING.lock() { r.push((name, arg, t)); } }
pub fn store_js(events: serde_json::Value) { if let Ok(mut r) = JS.lock() { r.push(events); } }
pub fn drain() -> serde_json::Value {
    let native: Vec<serde_json::Value> = RING.lock().map(|mut r| r.drain(..).map(|(n, a, t)| serde_json::json!([n, a, t])).collect()).unwrap_or_default();
    let js: Vec<serde_json::Value> = JS.lock().map(|mut r| r.drain(..).collect()).unwrap_or_default();
    serde_json::json!({"native": native, "js": js})
}
''')

patch(os.path.join(W, "mod.rs"), [
 ("    viewport: ViewportHost,", "    pub(crate) viewport: ViewportHost,"),
 ("            .child(status)\n    }\n}", "            .child(status)\n            .child(canvas(|_, _, _| vv_trace::mark(\"prepaint_end\", 0), |_, _, _, _| vv_trace::mark(\"paint_end\", 0)).absolute().size_0())\n    }\n}"),
 ("impl Render for Workstation {\n    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {\n",
  "impl Render for Workstation {\n    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {\n        vv_trace::mark(\"render_start\", 0);\n"),
 ("        let result = self.state.execute(command);\n        if result.is_ok() {",
  "        vv_trace::mark(\"exec_start\", 0);\n        let result = self.state.execute(command);\n        vv_trace::mark(\"state_done\", self.state.timeline.playhead.max(0) as u64);\n        if result.is_ok() {"),
 ("        cx.notify();\n        result\n    }\n\n    pub fn dispatch(",
  "        cx.notify();\n        vv_trace::mark(\"exec_end\", 0);\n        result\n    }\n\n    pub fn vv_ruler_point(&self) -> Option<(f32, f32, f32)> {\n        let content = self.geometry.get(\"timeline-content\")?;\n        Some((f32::from(content.origin.x), f32::from(content.origin.y), self.state.timeline.pixels_per_frame()))\n    }\n\n    pub fn vv_ask_page_dump(&mut self, cx: &mut Context<Self>) {\n        self.viewport.vv_queue_script(\"window.__vvDump&&window.__vvDump()\");\n        self.viewport_flush(cx);\n    }\n\n    pub fn dispatch("),
])
text = open(os.path.join(W, "mod.rs")).read()
first = text.index("\nmod ") if "\nmod " in text else None
if "mod vv_trace;" not in text:
    text = text[:first] + "\npub(crate) mod vv_trace;" + text[first:]
    open(os.path.join(W, "mod.rs"), "w").write(text)

patch(os.path.join(W, "viewport.rs"), [
 ("    pub(super) fn viewport_sync", "    pub(super) fn viewport_sync"),
 ("    fn viewport_flush(&mut self", "    pub(crate) fn viewport_flush(&mut self"),
 ("            .with_custom_protocol(SCHEME.to_string(), move |_, request| {\n                handle.respond(&request)\n            })",
  "            .with_custom_protocol(SCHEME.to_string(), move |_, request| {\n                super::vv_trace::mark(\"serve_start\", 0);\n                let answer = handle.respond(&request);\n                super::vv_trace::mark(\"serve_end\", answer.body().len() as u64);\n                answer\n            })"),
 ("                let _ = sender.try_send(request.body().clone());",
  "                super::vv_trace::mark(\"ipc_handler\", request.body().len() as u64);\n                let _ = sender.try_send(request.body().clone());"),
 ("            for script in scripts {\n                if let Err(error) = view.read(cx).evaluate_script(&script) {\n                    self.viewport.on_script_error(&error.to_string());\n                    break;\n                }\n            }",
  "            for script in scripts {\n                super::vv_trace::mark(\"eval_start\", script.len() as u64);\n                let evaluated = view.read(cx).evaluate_script(&script);\n                super::vv_trace::mark(\"eval_end\", 0);\n                if let Err(error) = evaluated {\n                    self.viewport.on_script_error(&error.to_string());\n                    break;\n                }\n            }"),
 ("        self.viewport.sync(&self.state);\n        self.viewport_flush(cx);",
  "        super::vv_trace::mark(\"sync_start\", 0);\n        self.viewport.sync(&self.state);\n        super::vv_trace::mark(\"sync_end\", 0);\n        self.viewport_flush(cx);\n        super::vv_trace::mark(\"sync_flush_end\", 0);"),
 ("        self.viewport.on_ipc(raw, &self.state);\n        self.viewport_flush(cx);",
  "        super::vv_trace::mark(\"on_ipc_start\", raw.len() as u64);\n        self.viewport.on_ipc(raw, &self.state);\n        super::vv_trace::mark(\"on_ipc_end\", 0);\n        self.viewport_flush(cx);\n        super::vv_trace::mark(\"ipc_flush_end\", 0);"),
])
patch(os.path.join(W, "viewport_host.rs"), [
 ("        let plan = match state.plan(&PlanRequest::of(state)) {", "        super::vv_trace::mark(\"plan_start\", 0);\n        let plan = match state.plan(&PlanRequest::of(state)) {"),
 ("        let size = Self::preview_size(&plan);\n        let key = format!(", "        super::vv_trace::mark(\"plan_end\", 0);\n        let size = Self::preview_size(&plan);\n        let key = format!("),
 ("        self.dirty = false;\n        self.last_key = Some(key);\n        self.output_size = Some(size);", "        super::vv_trace::mark(\"key_end\", 0);\n        self.dirty = false;\n        self.last_key = Some(key);\n        self.output_size = Some(size);"),
 ("        self.request_preview(state, plan, output);\n    }\n", "        self.request_preview(state, plan, output);\n        super::vv_trace::mark(\"preview_end\", 0);\n    }\n"),
 ("        match message[\"type\"].as_str() {\n            Some(\"page.ready\") => {",
  "        match message[\"type\"].as_str() {\n            Some(\"vv.trace\") => {\n                super::vv_trace::store_js(message[\"events\"].take());\n                return;\n            }\n            Some(\"page.ready\") => {"),
 ("    /// Scripts to run in the page, in order.\n", "    pub fn vv_queue_script(&mut self, script: &str) {\n        self.outbox.borrow_mut().push_back(script.to_string());\n    }\n\n    /// Scripts to run in the page, in order.\n"),
])
# The window receives the real pointer of whoever uses this machine; only injected moves carry a pressed button.
patch(os.path.join(W, "timeline_panel.rs"), [
 ("                |this, event: &DragMoveEvent<ScrubDrag>, _, cx| {\n", "                |this, event: &DragMoveEvent<ScrubDrag>, _, cx| {\n                    if event.event.pressed_button.is_none() {\n                        super::vv_trace::mark(\"foreign_move\", 0);\n                        return;\n                    }\n"),
])
# Real GPUI dispatch on the real window: hit test, drag threshold, on_drag_move, Command, WebView.
patch(os.path.join(root, "crates/desktop/src/automation/server.rs"), [
 ("        Some(\"set\") => return set(request, workstation, window, cx),",
  """        Some("vv_dump") => {
            workstation.update(cx, |workstation, cx| workstation.vv_ask_page_dump(cx));
            crate::workstation::vv_trace::drain()
        }
        Some("vv_eval") => {
            let script = request["script"].as_str().unwrap_or_default().to_string();
            workstation.update(cx, |workstation, cx| {
                workstation.viewport.vv_queue_script(&script);
                workstation.viewport_flush(cx);
            });
            json!({"queued": true})
        }
        Some("vv_ruler") => {
            let point = workstation.read(cx).vv_ruler_point();
            json!(point.map(|(x, y, ppf)| json!({"x": x, "y": y, "ppf": ppf})))
        }
        Some("vv_drag") => {
            let x0 = request["x0"].as_f64().unwrap_or(0.0) as f32;
            let x1 = request["x1"].as_f64().unwrap_or(0.0) as f32;
            let y = request["y"].as_f64().unwrap_or(0.0) as f32;
            let hz = request["hz"].as_f64().unwrap_or(60.0).max(1.0);
            let ticks = request["ticks"].as_u64().unwrap_or(60).max(2);
            let handle = window.window_handle();
            cx.spawn(async move |cx| {
                let period = std::time::Duration::from_secs_f64(1.0 / hz);
                let pos = |t: f32| point(px(x0 + (x1 - x0) * t), px(y));
                let send = |cx: &mut AsyncApp, input: PlatformInput, tick: u64| {
                    let _ = cx.update_window(handle, |_, window, cx| {
                        crate::workstation::vv_trace::mark("input_tick", tick);
                        window.dispatch_event(input, cx);
                        crate::workstation::vv_trace::mark("input_done", tick);
                    });
                };
                send(cx, PlatformInput::MouseMove(MouseMoveEvent { position: pos(0.0), pressed_button: None, modifiers: Modifiers::default() }), 0);
                cx.background_executor().timer(period).await;
                send(cx, PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: pos(0.0), modifiers: Modifiers::default(), click_count: 1, first_mouse: false }), 0);
                let started = std::time::Instant::now();
                for tick in 1..=ticks {
                    let at = period * tick as u32;
                    if let Some(rest) = at.checked_sub(started.elapsed()) { cx.background_executor().timer(rest).await; }
                    let t = tick as f32 / ticks as f32;
                    send(cx, PlatformInput::MouseMove(MouseMoveEvent { position: pos(t), pressed_button: Some(MouseButton::Left), modifiers: Modifiers::default() }), tick);
                }
                send(cx, PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: pos(1.0), modifiers: Modifiers::default(), click_count: 1 }), ticks + 1);
                crate::workstation::vv_trace::mark("input_end", ticks);
            }).detach();
            json!({"started": true})
        }
        Some("set") => return set(request, workstation, window, cx),"""),
])
print("patched", root)
