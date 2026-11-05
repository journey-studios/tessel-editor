//! Semantic project operations on the live editor: the same JSON vocabulary as the headless
//! CLI (`tessel_editor_core::ops`), run against the window's state and its viewport.
//!
//! A pure state change runs through [`Workstation::execute`] like any gesture, so the monitor
//! follows it. `render.frame` is the only deferred operation: it answers once the WebView has
//! actually drawn the frame.

use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::headless::{create_project, render_audio, run_project_op};
use tessel_editor_core::ops::{Op, ProjectOp, parse_op};

use super::viewport_host::FrameWaiter;
use super::{Activity, Source, Workstation};

pub enum OpReply {
    Done(Value),
    /// The answer arrives when the viewport finishes the frame.
    Pending(FrameWaiter),
}

impl Workstation {
    fn note(&mut self, text: &str, cx: &mut gpui_kit::Context<Self>) {
        self.activity = Some(Activity {
            source: Source::Cli,
            text: text.to_string(),
            error: false,
        });
        cx.notify();
    }

    pub fn run_op(
        &mut self,
        request: &Value,
        cx: &mut gpui_kit::Context<Self>,
    ) -> Result<OpReply, CommandError> {
        if let Some(result) = self.run_playback_op(request, cx) {
            return result.map(OpReply::Done);
        }
        // media.*, export.*, job.* and package/restore/import belong to the media host.
        if let Some(result) = self.run_media_op(request, cx) {
            return result.map(OpReply::Done);
        }
        let Some(op) = parse_op(&self.state, request)? else {
            return Err(CommandError::new(
                "unknown_operation",
                format!(
                    "`{}` is not a project operation; see docs/features/project-creation.md",
                    request["op"].as_str().unwrap_or_default()
                ),
            ));
        };
        match op {
            Op::Status => Ok(OpReply::Done(self.state.summary())),
            Op::Run(command) => self.execute(command, Source::Cli, cx).map(OpReply::Done),
            Op::CreateProject(settings) => {
                // Build first, swap after (inside `create_project`): a rejected setting keeps
                // the session and the viewport as they were.
                let created = create_project(&mut self.state, None, &settings)?;
                self.viewport_reset_project(cx);
                self.note("project.create", cx);
                Ok(OpReply::Done(created))
            }
            Op::Project(project) => {
                // Opening or recovering swaps the whole project: the viewport must forget the
                // assets it loaded for the old one.
                let replaces = matches!(
                    project,
                    ProjectOp::Open { .. } | ProjectOp::Recover { apply: true, .. }
                );
                let result = run_project_op(&mut self.state, None, project)?;
                if replaces {
                    self.viewport_reset_project(cx);
                }
                self.note("project", cx);
                Ok(OpReply::Done(result))
            }
            Op::Plan(plan) => Ok(OpReply::Done(self.state.plan(&plan)?.to_json())),
            Op::PreviewPolicy => Ok(OpReply::Done(self.state.preview_policy.report(&self.state))),
            Op::Capabilities => {
                let report = self.viewport.report();
                if report["capabilities"].is_null() {
                    return Err(CommandError::new(
                        "backend_unavailable",
                        format!(
                            "The viewport runtime has not reported (phase `{}`): {}",
                            report["phase"].as_str().unwrap_or("unknown"),
                            report["session"]["error"]["message"]
                                .as_str()
                                .unwrap_or("starting")
                        ),
                    ));
                }
                Ok(OpReply::Done(report["capabilities"].clone()))
            }
            Op::RenderStatus => Ok(OpReply::Done(self.viewport.report_with_frames(128))),
            Op::RenderFrame(frame) => {
                let waiter = self.viewport_render_frame(&frame, cx)?;
                self.note("render.frame", cx);
                Ok(OpReply::Pending(waiter))
            }
            Op::RenderAudio(audio) => Ok(OpReply::Done(render_audio(&self.state, &audio)?)),
        }
    }

    /// The automation tree: the core's nodes plus the viewport, which only the window knows.
    pub fn viewport_node(&self) -> Value {
        let report = self.viewport.report();
        json!({
            "id": "monitor.viewport", "kind": "viewport", "label": "Viewport",
            "parent": "region:monitor", "children": [], "enabled": true,
            "visible": self.viewport.on_screen, "state": report, "fields": [], "actions": [],
        })
    }
}
