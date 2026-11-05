//! JSON requests over the same commands used by the desktop adapter.
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::audio::{AudioBackend, OfflineMixer, encode_wav_pcm16};
use crate::backend::{BackendError, FrameResult, Output};
use crate::commands::{self, Parsed};
use crate::ops::{self, Op, ProjectOp, RenderAudio, RenderFrame};
use crate::plan::RenderPlan;
use crate::session::Renderer;
use crate::state::ProjectSettings;
use crate::state::layout::Layout;
use crate::{CommandError, EditorState, Raw};

/// How long one `render.frame` may take, loading its assets included.
pub const RENDER_TIMEOUT: Duration = Duration::from_secs(60);

fn text<'a>(request: &'a Value, key: &str) -> Result<&'a str, CommandError> {
    request.get(key).and_then(Value::as_str).ok_or_else(|| {
        CommandError::new("invalid_request", format!("Expected string field `{key}`"))
    })
}

/// Maps a backend failure onto the CLI's stable error codes. Unknown codes are kept in
/// `details.backend_code` so nothing a backend says is lost.
pub fn backend_error(error: BackendError) -> CommandError {
    let code = match error.code.as_str() {
        "backend_unavailable" => "backend_unavailable",
        "version_mismatch" => "version_mismatch",
        "asset_unreadable" => "asset_unreadable",
        "decode_failed" => "decode_failed",
        // A clip whose timing the native video path cannot honour is an unsupported input; the
        // specific reason stays readable in `details.backend_code`.
        "unsupported_format" | "unsupported_timing" => "unsupported_format",
        "context_lost" => "context_lost",
        "timeout" => "timeout",
        "cancelled" => "cancelled",
        "protocol_error" => "protocol_error",
        _ => "render_failed",
    };
    let mut mapped = CommandError::new(code, error.message.clone());
    mapped =
        mapped.details(json!({ "backend_code": error.code, "backend_details": error.details }));
    mapped
}

fn absolute(path: &Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Execute one request without a window, socket, or runtime.
///
/// A caller owns its session's state. Domain requests do not consult UI visibility.
/// Dock and OS window operations remain desktop capabilities and are rejected here.
/// Operations that need a render backend report `backend_unavailable`; use
/// [`execute_with`] to supply one.
pub fn execute(state: &mut EditorState, request: &Value) -> Result<Value, CommandError> {
    execute_with(state, None, request)
}

/// Operations that live outside the core because they need files, tools or threads (media
/// import, packages, video export). The core only knows the hook; the `tessel-editor-media`
/// crate provides the implementation, so the core stays free of I/O adapters and GPUI.
pub trait Extension {
    /// `None` when `request["op"]` is not this extension's: the core then handles it.
    fn execute(
        &mut self,
        state: &mut EditorState,
        renderer: Option<&mut dyn Renderer>,
        request: &Value,
    ) -> Option<Result<Value, CommandError>>;
}

/// [`execute`] with an optional render backend for `render.frame` and `capabilities`.
pub fn execute_with(
    state: &mut EditorState,
    renderer: Option<&mut dyn Renderer>,
    request: &Value,
) -> Result<Value, CommandError> {
    execute_ext(state, renderer, None, request)
}

/// [`execute_with`] plus an [`Extension`] that gets the first look at every request.
pub fn execute_ext(
    state: &mut EditorState,
    mut renderer: Option<&mut dyn Renderer>,
    extension: Option<&mut dyn Extension>,
    request: &Value,
) -> Result<Value, CommandError> {
    if let Some(extension) = extension
        && let Some(outcome) = extension.execute(state, reborrow(&mut renderer), request)
    {
        return outcome;
    }
    let op = text(request, "op")?;
    if let Some(parsed) = ops::parse_op(state, request)? {
        return run_op(state, renderer, parsed);
    }
    let target = text(request, "target")?;
    // A headless session has no dock. Reject presentation targets before parsing,
    // so callers cannot mistake a synthetic layout for a live desktop snapshot.
    if [
        "panel:",
        "dock",
        "region:",
        "group:",
        "split:",
        "window",
        "toolbar.theme",
    ]
    .iter()
    .any(|prefix| target.starts_with(prefix))
    {
        return Err(CommandError::new(
            "desktop_required",
            "This target belongs to the desktop adapter",
        ));
    }
    let layout = Layout {
        regions: Vec::new(),
        area: (0.0, 0.0),
    };
    let parsed = match op {
        "set" => {
            let value = request
                .get("value")
                .and_then(Raw::from_json)
                .ok_or_else(|| {
                    CommandError::new("invalid_request", "Expected scalar field `value`")
                })?;
            commands::parse_set(target, text(request, "field")?, &value)?
        }
        "invoke" => commands::parse_invoke(state, &layout, target, text(request, "action")?)?,
        "move" => {
            let args = request
                .get("args")
                .filter(|args| args.is_object())
                .ok_or_else(|| {
                    CommandError::new("invalid_request", "Expected object field `args`")
                })?;
            commands::parse_move(&layout, target, args)?
        }
        _ => {
            return Err(CommandError::new(
                "unknown_operation",
                format!("Unknown operation `{op}`"),
            ));
        }
    };
    match parsed {
        Parsed::Run(command) => state.execute(command),
        Parsed::Layout(_) | Parsed::ToggleTheme | Parsed::ReloadViewport => Err(CommandError::new(
            "desktop_required",
            "Dock layout, theme and viewport page commands require the desktop adapter",
        )),
    }
}

fn run_op(
    state: &mut EditorState,
    renderer: Option<&mut dyn Renderer>,
    op: Op,
) -> Result<Value, CommandError> {
    match op {
        Op::Status => Ok(state.summary()),
        Op::Run(command) => state.execute(command),
        Op::Project(op) => run_project_op(state, renderer, op),
        Op::CreateProject(settings) => create_project(state, renderer, &settings),
        Op::Plan(request) => Ok(state.plan(&request)?.to_json()),
        Op::PreviewPolicy => Ok(state.preview_policy.report(state)),
        Op::Capabilities => {
            let renderer = renderer.ok_or_else(no_backend)?;
            Ok(renderer.capabilities().map_err(backend_error)?.to_json())
        }
        Op::RenderStatus => {
            let renderer = renderer.ok_or_else(no_backend)?;
            Ok(renderer.summary())
        }
        Op::RenderFrame(frame) => {
            let plan = state.plan(&frame.plan)?;
            let renderer = renderer.ok_or_else(no_backend)?;
            let (width, height) = frame_geometry(&plan, &frame)?;
            let summary = plan_summary(&plan);
            let output = Output {
                width,
                height,
                png: frame.out.as_deref().map(absolute),
            };
            let result = renderer
                .render_frame(state, plan, output, frame.probes, RENDER_TIMEOUT)
                .map_err(backend_error)?;
            frame_report(&summary, &result)
        }
        Op::RenderAudio(audio) => render_audio(state, &audio),
    }
}

/// What `project.open` reports about a loaded candidate. Shared with adapters that load the
/// candidate off the caller's thread and swap it in later, so the answer is the same either way.
pub fn open_summary(candidate: &EditorState, path: &Path) -> Value {
    let recoveries = crate::persist::recoveries_for_document(path);
    json!({ "action": "project.open", "project": candidate.project_name,
        "id": candidate.project.id,
        "sequences": candidate.all_sequences().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        "assets": candidate.library.items.len(),
        "recoveries": recoveries.iter().map(crate::persist::Recovery::to_json).collect::<Vec<_>>(),
        "changed": true })
}

/// Document-level operations. `open` and `recover` build the candidate project first and swap
/// it in only on success, so a damaged file leaves the open project exactly as it was.
pub fn run_project_op(
    state: &mut EditorState,
    renderer: Option<&mut dyn Renderer>,
    op: ProjectOp,
) -> Result<Value, CommandError> {
    let replace =
        |state: &mut EditorState, renderer: Option<&mut dyn Renderer>, candidate: EditorState| {
            *state = candidate;
            if let Some(renderer) = renderer {
                renderer.reset_project();
            }
        };
    match op {
        ProjectOp::Save { path } => {
            let path = path
                .or_else(|| state.project.document.clone())
                .ok_or_else(|| {
                    CommandError::new("no_document", "Give a `path`: the project was never saved")
                })?;
            crate::persist::save(state, &path)
        }
        ProjectOp::Open { path } => {
            let candidate = crate::persist::load(&path)?;
            let summary = open_summary(&candidate, &path);
            replace(state, renderer, candidate);
            Ok(summary)
        }
        ProjectOp::Autosave => crate::persist::autosave(state),
        ProjectOp::Recover {
            path,
            revision,
            apply,
        } => {
            if !apply {
                return Ok(json!({ "action": "project.recover", "applied": false,
                    "recoveries": crate::persist::recoveries_for_document(&path)
                        .iter().map(crate::persist::Recovery::to_json).collect::<Vec<_>>() }));
            }
            let candidate = crate::persist::recover(&path, revision)?;
            let summary = json!({ "action": "project.recover", "applied": true,
                "project": candidate.project_name, "dirty": true, "changed": true });
            replace(state, renderer, candidate);
            Ok(summary)
        }
        ProjectOp::DiscardRecovery { path } => {
            let described = crate::persist::describe(&path)?;
            if let Some(id) = described["id"].as_str() {
                crate::persist::discard_recovery(&crate::persist::document_folder(&path), id);
            }
            Ok(json!({ "action": "project.discard_recovery", "changed": true }))
        }
        ProjectOp::Describe { path } => crate::persist::describe(&path),
        ProjectOp::Presets => Ok(crate::preset::list(state)),
        ProjectOp::History => Ok(json!({ "history": state.history.to_json(),
            "project": state.project.to_json() })),
    }
}

/// Replaces the project. Build first, swap after: a rejected setting must not discard the
/// session. `renderer` forgets its loaded assets because IDs may now mean something else.
pub fn create_project(
    state: &mut EditorState,
    renderer: Option<&mut dyn Renderer>,
    settings: &ProjectSettings,
) -> Result<Value, CommandError> {
    let created = EditorState::empty(settings)?;
    if let Some(workspace) = &settings.workspace {
        std::fs::create_dir_all(workspace).map_err(|error| {
            CommandError::new(
                "io_error",
                format!("Cannot create `{}`: {error}", workspace.display()),
            )
        })?;
    }
    *state = created;
    if let Some(renderer) = renderer {
        renderer.reset_project();
    }
    Ok(
        json!({ "action": "project.create", "project": state.project_name,
        "sequence": state.timeline.id, "rate": state.timeline.rate.to_json(),
        "width": state.timeline.width, "height": state.timeline.height,
        "length": state.timeline.length, "id": state.project.id,
        "workspace": state.project.workspace, "changed": true }),
    )
}

/// Output size of a frame request and the checks every host applies to it.
pub fn frame_geometry(plan: &RenderPlan, frame: &RenderFrame) -> Result<(u32, u32), CommandError> {
    let (width, height) = frame.size.unwrap_or((plan.width, plan.height));
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err(CommandError::invalid_value(
            "Output size is between 1x1 and 8192x8192",
            json!({ "max": 8192 }),
        ));
    }
    if let Some((x, y)) = frame
        .probes
        .iter()
        .find(|(x, y)| *x >= width || *y >= height)
    {
        return Err(CommandError::invalid_value(
            format!("Probe ({x}, {y}) is outside the {width}x{height} output"),
            json!({ "width": width, "height": height }),
        ));
    }
    Ok((width, height))
}

pub fn plan_summary(plan: &RenderPlan) -> Value {
    json!({ "frame": plan.frame, "time": plan.time.to_json(),
        "layers": plan.layers.iter().map(|layer| json!({
            "id": layer.id, "kind": layer.content.kind(),
        })).collect::<Vec<_>>(),
        "warnings": plan.warnings.iter().map(|w| json!({"code": w.code, "message": w.message})).collect::<Vec<_>>() })
}

/// The `render.frame` answer, after checking that a reported PNG really exists.
pub fn frame_report(summary: &Value, result: &FrameResult) -> Result<Value, CommandError> {
    if let Some(path) = &result.png
        && !Path::new(path).is_file()
    {
        return Err(CommandError::new(
            "render_failed",
            format!("The backend reported `{path}` but the file does not exist"),
        ));
    }
    Ok(json!({ "action": "render.frame", "plan": summary, "frame": result.to_json() }))
}

/// Offline mix of the audio plan: the processed signal, never what a device played.
pub fn render_audio(state: &EditorState, audio: &RenderAudio) -> Result<Value, CommandError> {
    let plan = state.plan_audio(audio.from, audio.frames)?;
    let mixed = OfflineMixer::default().mix(&plan).map_err(backend_error)?;
    let mut out = None;
    if let Some(path) = &audio.out {
        std::fs::write(path, encode_wav_pcm16(&mixed)).map_err(|error| {
            CommandError::new(
                "io_error",
                format!("Cannot write `{}`: {error}", path.display()),
            )
        })?;
        out = Some(absolute(path));
    }
    Ok(json!({
        "action": "render.audio", "kind": "processed",
        "note": "offline mix; not played through an output device",
        "sample_rate": mixed.sample_rate, "frames": mixed.frames(), "channels": 2,
        "segments": plan.segments.len(),
        "peak": mixed.peak(), "rms": mixed.rms(),
        "samples": audio.samples.iter().map(|(frame, channel)| json!({
            "frame": frame, "channel": channel, "value": mixed.sample(*frame as usize, *channel),
        })).collect::<Vec<_>>(),
        "out": out,
        "plan": plan.to_json(),
    }))
}

fn no_backend() -> CommandError {
    CommandError::new(
        "backend_unavailable",
        "No render backend is attached to this session. Use `./scripts/render` (Node + Chrome \
         runtime, see docs/features/viewport.md) or inject a backend with `execute_with`.",
    )
}

/// Versioned response envelope, also used by the headless executable.
pub fn response(state: &mut EditorState, request: &Value) -> Value {
    response_with(state, None, request)
}

pub fn response_with(
    state: &mut EditorState,
    renderer: Option<&mut dyn Renderer>,
    request: &Value,
) -> Value {
    response_ext(state, renderer, None, request)
}

pub fn response_ext(
    state: &mut EditorState,
    renderer: Option<&mut dyn Renderer>,
    extension: Option<&mut dyn Extension>,
    request: &Value,
) -> Value {
    match execute_ext(state, renderer, extension, request) {
        Ok(result) => json!({"ok": true, "version": 1, "result": result}),
        Err(error) => error.to_json(),
    }
}

/// Shortens the trait-object lifetime so one renderer can serve a loop of requests.
fn reborrow<'b>(renderer: &'b mut Option<&mut dyn Renderer>) -> Option<&'b mut dyn Renderer> {
    match renderer {
        Some(inner) => Some(&mut **inner),
        None => None,
    }
}

/// Runs a JSON Lines session: one request per input line, one response per output line.
/// Returns whether any request failed. Shared by `tessel-headless` and `tessel-render`.
pub fn run_session<R: std::io::BufRead, W: std::io::Write>(
    state: &mut EditorState,
    renderer: Option<&mut dyn Renderer>,
    input: R,
    output: &mut W,
) -> std::io::Result<bool> {
    run_session_ext(state, renderer, None, input, output)
}

/// [`run_session`] with an [`Extension`].
pub fn run_session_ext<R: std::io::BufRead, W: std::io::Write>(
    state: &mut EditorState,
    mut renderer: Option<&mut dyn Renderer>,
    mut extension: Option<&mut dyn Extension>,
    input: R,
    output: &mut W,
) -> std::io::Result<bool> {
    let mut failed = false;
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str(&line) {
            Ok(request) => response_ext(
                state,
                reborrow(&mut renderer),
                extension
                    .as_deref_mut()
                    .map(|ext| ext as &mut dyn Extension),
                &request,
            ),
            Err(error) => CommandError::new("invalid_json", error.to_string()).to_json(),
        };
        failed |= response["ok"] != true;
        writeln!(output, "{response}")?;
        output.flush()?;
    }
    Ok(failed)
}
