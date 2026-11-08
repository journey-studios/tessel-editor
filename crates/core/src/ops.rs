//! Semantic project operations: the JSON vocabulary for creating projects, assets, tracks and
//! clips, configuring cameras, seeking, planning and rendering.
//!
//! [`parse_op`] only translates a request into an [`Op`]; adapters execute it. The headless
//! CLI runs `Op::Run` through `EditorState::execute`, and the desktop runs the same command
//! through its workstation handler, so a request has one meaning on every surface.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::assets::{Asset, Declared, MediaKind, ProxyFormat};
use crate::content::{Camera, TextSpec};
use crate::directed_scene::Vec3;
use crate::id::Id;
use crate::plan::{PlanRequest, PlanTime, PreviewIntent};
use crate::preview::PreviewMode;
use crate::scene_edit::{PropertyRef, parse_batch};
use crate::scene_interaction::{
    CancelReason, FrameAt, GestureCommand, GizmoCommand, Point, PointerCommand, SelectMode,
};
use crate::scene_project::{CameraAt, SceneCommand, SceneSource};
use crate::state::timeline::{SequenceSettings, TrackKind};
use crate::state::{Command, CommandError, EditorState, ProjectSettings, Raw};
use crate::time::{Rate, Ratio};

#[derive(Debug, Clone, PartialEq)]
pub struct RenderFrame {
    pub plan: PlanRequest,
    pub out: Option<PathBuf>,
    /// Output size; defaults to the canvas of the plan.
    pub size: Option<(u32, u32)>,
    pub probes: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderAudio {
    pub from: i64,
    pub frames: i64,
    pub out: Option<PathBuf>,
    /// `(output sample frame, channel)` values to report.
    pub samples: Vec<(u64, usize)>,
}

/// Project-level operations that touch the filesystem (document, autosave, recovery).
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectOp {
    /// `path: None` saves to the document the project was opened from.
    Save {
        path: Option<PathBuf>,
    },
    Open {
        path: PathBuf,
    },
    Autosave,
    /// Lists (`apply: false`) or loads the newest/chosen autosave of a document.
    Recover {
        path: PathBuf,
        revision: Option<u64>,
        apply: bool,
    },
    DiscardRecovery {
        path: PathBuf,
    },
    Describe {
        path: PathBuf,
    },
    History,
    /// Built-in and project presets, and the selected one.
    Presets,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Run(Command),
    Project(ProjectOp),
    CreateProject(ProjectSettings),
    Plan(PlanRequest),
    /// The preview quality policy and, per video, why it is or is not drawn from a proxy.
    PreviewPolicy,
    RenderFrame(RenderFrame),
    RenderAudio(RenderAudio),
    Capabilities,
    RenderStatus,
    Status,
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new("invalid_request", message)
}

fn string<'a>(request: &'a Value, key: &str) -> Result<&'a str, CommandError> {
    request
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("Expected string field `{key}`")))
}

fn optional_string(request: &Value, key: &str) -> Result<Option<String>, CommandError> {
    match request.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(invalid(format!("Field `{key}` must be a string"))),
    }
}

fn integer(request: &Value, key: &str) -> Result<i64, CommandError> {
    request
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid(format!("Expected integer field `{key}`")))
}

fn optional_u32(request: &Value, key: &str) -> Result<Option<u32>, CommandError> {
    match request.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => raw
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| invalid(format!("Field `{key}` must be a non-negative integer"))),
    }
}

fn time_field(request: &Value, key: &str) -> Result<Option<Ratio>, CommandError> {
    let Some(raw) = request.get(key).filter(|raw| !raw.is_null()) else {
        return Ok(None);
    };
    // A bare number is whole seconds; fractional seconds need the exact {num, den} form so
    // a caller cannot lose precision to a float by accident.
    Ratio::from_json(raw).map(Some).ok_or_else(|| {
        CommandError::invalid_value(
            format!("`{key}` is seconds: an integer or {{\"num\":1,\"den\":24}}"),
            json!({ "format": {"num": 1, "den": 24} }),
        )
    })
}

fn rate_field(request: &Value, key: &str) -> Result<Option<Rate>, CommandError> {
    let Some(raw) = request.get(key).filter(|raw| !raw.is_null()) else {
        return Ok(None);
    };
    Rate::from_json(raw).map(Some).ok_or_else(|| {
        CommandError::invalid_value(
            format!("`{key}` is frames per second: an integer or {{\"num\":24000,\"den\":1001}}"),
            json!({ "max_fps": 1000 }),
        )
    })
}

fn id_field(request: &Value, key: &str) -> Result<Id, CommandError> {
    let raw = string(request, key)?;
    Id::new(raw).map_err(|error| CommandError::new("invalid_id", error.to_string()).target(raw))
}

fn kind_field(request: &Value) -> Result<MediaKind, CommandError> {
    let raw = string(request, "kind")?;
    MediaKind::parse(raw)
        .filter(|kind| MediaKind::ASSETS.contains(kind) || *kind == MediaKind::Timeline)
        .ok_or_else(|| {
            CommandError::invalid_value(
                format!("`{raw}` is not an asset kind"),
                json!({ "options": ["image", "video", "audio", "text", "scene", "splat"] }),
            )
        })
}

fn track_kind(request: &Value) -> Result<TrackKind, CommandError> {
    match string(request, "kind")? {
        "video" => Ok(TrackKind::Video),
        "audio" => Ok(TrackKind::Audio),
        other => Err(CommandError::invalid_value(
            format!("`{other}` is not a track kind"),
            json!({ "options": ["video", "audio"] }),
        )),
    }
}

fn plan_time(request: &Value) -> Result<PlanTime, CommandError> {
    match (request.get("frame"), time_field(request, "time")?) {
        (Some(_), Some(_)) => Err(invalid("Use either `frame` or `time`, not both")),
        (Some(frame), None) => frame
            .as_i64()
            .map(PlanTime::Frame)
            .ok_or_else(|| invalid("`frame` must be an integer")),
        (None, Some(time)) => Ok(PlanTime::Time(time)),
        (None, None) => Ok(PlanTime::Playhead),
    }
}

fn plan_request(state: &EditorState, request: &Value) -> Result<PlanRequest, CommandError> {
    let intent = match request.get("intent").filter(|intent| !intent.is_null()) {
        Some(intent) => PreviewIntent::from_json(intent)?,
        None => state.preview.clone(),
    };
    Ok(PlanRequest {
        intent,
        at: plan_time(request)?,
    })
}

fn probes(request: &Value) -> Result<Vec<(u32, u32)>, CommandError> {
    let Some(items) = request.get("probes") else {
        return Ok(Vec::new());
    };
    let bad = || invalid("`probes` is a list of [x, y] pixel coordinates");
    items
        .as_array()
        .ok_or_else(bad)?
        .iter()
        .map(|item| {
            let pair = item
                .as_array()
                .filter(|pair| pair.len() == 2)
                .ok_or_else(bad)?;
            let coordinate = |value: &Value| {
                value
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(bad)
            };
            Ok((coordinate(&pair[0])?, coordinate(&pair[1])?))
        })
        .collect()
}

fn asset_add(request: &Value) -> Result<Command, CommandError> {
    let kind = kind_field(request)?;
    let name = string(request, "name")?.to_string();
    let declared = Declared {
        width: optional_u32(request, "width")?,
        height: optional_u32(request, "height")?,
        rate: rate_field(request, "rate")?,
        duration: time_field(request, "duration")?,
        text: None,
        frames: request.get("frames").and_then(Value::as_i64),
        first: request.get("first").and_then(Value::as_i64),
    };
    let asset = if kind == MediaKind::Text {
        let spec = request
            .get("text")
            .ok_or_else(|| invalid("A text asset needs `text`"))?;
        Asset::text(TextSpec::from_json(spec)?)?
    } else if kind == MediaKind::Video && request.get("pattern").is_some() {
        Asset::probe_frame_sequence(string(request, "pattern")?, &declared)?
    } else {
        Asset::probe(kind, string(request, "path")?, &declared)?
    };
    Ok(Command::AddAsset {
        id: optional_string(request, "id")?,
        name,
        kind,
        asset: Box::new(asset),
        folder: optional_string(request, "folder")?,
    })
}

fn clip_add(request: &Value) -> Result<Command, CommandError> {
    let properties = match request.get("properties") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(name, value)| {
                Raw::from_json(value)
                    .map(|raw| (name.clone(), raw))
                    .ok_or_else(|| {
                        invalid(format!(
                            "Property `{name}` must be a string, number or boolean"
                        ))
                    })
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(invalid("`properties` is an object of inspector values")),
    };
    Ok(Command::AddClip {
        id: optional_string(request, "id")?,
        name: optional_string(request, "name")?,
        track: string(request, "track")?.to_string(),
        asset: optional_string(request, "asset")?,
        start: integer(request, "start")?,
        duration: integer(request, "duration")?,
        source_start: time_field(request, "source_start")?,
        text: request
            .get("text")
            .filter(|t| !t.is_null())
            .map(TextSpec::from_json)
            .transpose()?,
        camera: request
            .get("camera")
            .filter(|c| !c.is_null())
            .map(Camera::from_json)
            .transpose()?,
        properties,
    })
}

fn project_create(request: &Value) -> Result<ProjectSettings, CommandError> {
    let base = SequenceSettings::default();
    let rate = rate_field(request, "rate")?.unwrap_or(base.rate);
    let length = match (request.get("length"), time_field(request, "seconds")?) {
        (Some(_), Some(_)) => return Err(invalid("Use either `length` (frames) or `seconds`")),
        (Some(frames), None) => frames
            .as_i64()
            .ok_or_else(|| invalid("`length` must be an integer number of frames"))?,
        (None, Some(seconds)) => seconds
            .mul(Ratio::new(i64::from(rate.num()), i64::from(rate.den())).unwrap_or(Ratio::ZERO))
            .map_or(0, Ratio::round),
        (None, None) => base.length,
    };
    Ok(ProjectSettings {
        workspace: optional_string(request, "dir")?.map(PathBuf::from),
        name: optional_string(request, "name")?.unwrap_or_else(|| "Untitled".to_string()),
        sequence: SequenceSettings {
            id: match request.get("sequence") {
                Some(Value::String(_)) => id_field(request, "sequence")?,
                _ => base.id,
            },
            rate,
            length,
            width: optional_u32(request, "width")?.unwrap_or(base.width),
            height: optional_u32(request, "height")?.unwrap_or(base.height),
        },
    })
}

/// Translates one operation request. Operations on `target`s (`set`, `invoke`, `move`) are
/// handled by `headless::execute`; this vocabulary is the project-level one.
pub fn parse_op(state: &EditorState, request: &Value) -> Result<Option<Op>, CommandError> {
    let op = string(request, "op")?;
    let command = match op {
        "status" => return Ok(Some(Op::Status)),
        "capabilities" => return Ok(Some(Op::Capabilities)),
        "render.status" => return Ok(Some(Op::RenderStatus)),
        "project.create" => return Ok(Some(Op::CreateProject(project_create(request)?))),
        "asset.add" => asset_add(request)?,
        "track.add" => Command::CreateTrack {
            id: optional_string(request, "id")?,
            name: string(request, "name")?.to_string(),
            kind: track_kind(request)?,
        },
        "clip.add" => clip_add(request)?,
        "clip.text" => Command::SetClipText {
            clip: string(request, "clip")?.to_string(),
            text: TextSpec::from_json(
                request
                    .get("text")
                    .ok_or_else(|| invalid("Expected object field `text`"))?,
            )?,
        },
        "clip.camera" => Command::SetClipCamera {
            clip: string(request, "clip")?.to_string(),
            camera: Camera::from_json(request)?,
        },
        "clip.property" => Command::SetClipProperty {
            clip: string(request, "clip")?.to_string(),
            property: string(request, "property")?.to_string(),
            value: request
                .get("value")
                .and_then(Raw::from_json)
                .ok_or_else(|| invalid("Expected scalar field `value`"))?,
        },
        "sequence.length" => Command::SetSequenceLength {
            frames: integer(request, "frames")?,
        },
        "history.undo" => Command::Undo,
        "history.redo" => Command::Redo,
        "history.status" => return Ok(Some(Op::Project(ProjectOp::History))),
        "sequence.add" => {
            let base = SequenceSettings::default();
            let any_format = ["rate", "width", "height", "length"]
                .iter()
                .any(|key| request.get(key).is_some());
            let settings = if any_format {
                let rate = rate_field(request, "rate")?.unwrap_or(state.timeline.rate);
                Some(SequenceSettings {
                    id: base.id,
                    rate,
                    length: request
                        .get("length")
                        .and_then(Value::as_i64)
                        .unwrap_or(state.timeline.length),
                    width: optional_u32(request, "width")?.unwrap_or(state.timeline.width),
                    height: optional_u32(request, "height")?.unwrap_or(state.timeline.height),
                })
            } else {
                None
            };
            Command::AddSequence {
                id: optional_string(request, "id")?,
                name: optional_string(request, "name")?,
                settings,
                activate: request
                    .get("activate")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }
        }
        "sequence.activate" => Command::ActivateSequence {
            sequence: string(request, "sequence")?.to_string(),
        },
        "sequence.remove" => Command::RemoveSequence {
            sequence: string(request, "sequence")?.to_string(),
        },
        "sequence.rename" => Command::RenameSequence {
            sequence: string(request, "sequence")?.to_string(),
            name: string(request, "name")?.to_string(),
        },
        "preset.save" => Command::SavePreset {
            preset: request.get("preset").cloned().ok_or_else(|| {
                CommandError::new("invalid_request", "Expected object field `preset`")
            })?,
        },
        "preset.delete" => Command::DeletePreset {
            id: string(request, "id")?.to_string(),
        },
        "preset.select" => Command::SelectPreset {
            id: string(request, "id")?.to_string(),
        },
        "preset.list" => return Ok(Some(Op::Project(ProjectOp::Presets))),
        "project.save" => {
            return Ok(Some(Op::Project(ProjectOp::Save {
                path: optional_string(request, "path")?.map(PathBuf::from),
            })));
        }
        "project.open" => {
            return Ok(Some(Op::Project(ProjectOp::Open {
                path: PathBuf::from(string(request, "path")?),
            })));
        }
        "project.autosave" => return Ok(Some(Op::Project(ProjectOp::Autosave))),
        "project.recover" => {
            return Ok(Some(Op::Project(ProjectOp::Recover {
                path: PathBuf::from(string(request, "path")?),
                revision: request.get("revision").and_then(Value::as_u64),
                apply: request
                    .get("apply")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            })));
        }
        "project.discard_recovery" => {
            return Ok(Some(Op::Project(ProjectOp::DiscardRecovery {
                path: PathBuf::from(string(request, "path")?),
            })));
        }
        "project.describe" => {
            return Ok(Some(Op::Project(ProjectOp::Describe {
                path: PathBuf::from(string(request, "path")?),
            })));
        }
        "preview.policy" => {
            let mode = optional_string(request, "mode")?
                .map(|label| {
                    PreviewMode::from_label(&label)
                        .ok_or_else(|| invalid("`mode` is `original` or `auto`"))
                })
                .transpose()?;
            let format = optional_string(request, "format")?
                .map(|label| {
                    ProxyFormat::from_label(&label)
                        .ok_or_else(|| invalid("`format` is `jpeg` or `png`"))
                })
                .transpose()?;
            let max_height = optional_u32(request, "max_height")?;
            if mode.is_none() && format.is_none() && max_height.is_none() {
                return Ok(Some(Op::PreviewPolicy));
            }
            Command::SetPreviewPolicy {
                mode,
                max_height,
                format,
            }
        }
        "preview.set" => Command::SetPreview {
            intent: PreviewIntent::from_json(request)?,
        },
        "seek" => {
            let frame = match plan_time(request)? {
                PlanTime::Frame(frame) => frame,
                PlanTime::Time(time) => state.timeline.rate.time_to_frame_floor(time),
                PlanTime::Playhead => return Err(invalid("`seek` needs `frame` or `time`")),
            };
            Command::SetPlayhead {
                frame: Raw::Number(frame as f64),
            }
        }
        "plan" => return Ok(Some(Op::Plan(plan_request(state, request)?))),
        "render.frame" => {
            let size = match (
                optional_u32(request, "width")?,
                optional_u32(request, "height")?,
            ) {
                (Some(width), Some(height)) => Some((width, height)),
                (None, None) => None,
                _ => return Err(invalid("Give both `width` and `height`, or neither")),
            };
            return Ok(Some(Op::RenderFrame(RenderFrame {
                plan: plan_request(state, request)?,
                out: optional_string(request, "out")?.map(PathBuf::from),
                size,
                probes: probes(request)?,
            })));
        }
        "render.audio" => {
            let from = request.get("from").and_then(Value::as_i64).unwrap_or(0);
            let frames = match request.get("frames") {
                Some(frames) => frames
                    .as_i64()
                    .ok_or_else(|| invalid("`frames` must be an integer"))?,
                None => state.timeline.length - from,
            };
            let samples = match request.get("samples").and_then(Value::as_array) {
                None => Vec::new(),
                Some(items) => items
                    .iter()
                    .map(|item| {
                        let pair = item.as_array().filter(|pair| pair.len() == 2);
                        let frame = pair.and_then(|pair| pair[0].as_u64());
                        let channel = pair.and_then(|pair| pair[1].as_u64());
                        frame
                            .zip(channel)
                            .filter(|(_, channel)| *channel < 2)
                            .map(|(frame, channel)| (frame, channel as usize))
                            .ok_or_else(|| {
                                invalid("`samples` is a list of [output_frame, channel 0|1]")
                            })
                    })
                    .collect::<Result<_, _>>()?,
            };
            return Ok(Some(Op::RenderAudio(RenderAudio {
                from,
                frames,
                out: optional_string(request, "out")?.map(PathBuf::from),
                samples,
            })));
        }
        scene if scene.starts_with("scene.") => Command::Scene(scene_command(scene, request)?),
        // BL062: the slider gesture cycle (`property.gesture.begin|update|commit|cancel`). Its
        // shape rules live next to the state machine, like the scene gesture's do.
        gesture if gesture.starts_with("property.gesture.") => {
            crate::state::property_gesture::parse_op(gesture, request)?
        }
        _ => return Ok(None),
    };
    Ok(Some(Op::Run(command)))
}

/// The `scene.*` vocabulary of F036. Shape only: every rule (IDs, bundle validity, edit gates)
/// belongs to `scene_project`, `directed_scene` and `scene_edit`, so a request means the same
/// through the CLI, a script, the socket and the library.
fn scene_command(op: &str, request: &Value) -> Result<SceneCommand, CommandError> {
    let scene = || string(request, "scene").map(str::to_string);
    Ok(match op {
        "scene.attach" => SceneCommand::Attach {
            id: optional_string(request, "id")?,
            name: optional_string(request, "name")?,
            source: scene_source(request)?,
        },
        "scene.detach" => SceneCommand::Detach { scene: scene()? },
        "scene.edit" => SceneCommand::Edit {
            scene: scene()?,
            ops: parse_batch(
                request
                    .get("ops")
                    .ok_or_else(|| invalid("Expected array field `ops`"))?,
            )?,
            dry_run: optional_bool(request, "dry_run")?.unwrap_or(false),
        },
        // F016: the effects layer. Shape only; the catalog, IDs and layer rules are the core's.
        "scene.fx.edit" => SceneCommand::FxEdit {
            scene: scene()?,
            ops: crate::scene_edit::parse_fx_batch(
                request
                    .get("ops")
                    .ok_or_else(|| invalid("Expected array field `ops`"))?,
            )?,
            dry_run: optional_bool(request, "dry_run")?.unwrap_or(false),
        },
        "scene.fx.list" => SceneCommand::FxList { scene: scene()? },
        "scene.fx.catalog" => SceneCommand::FxCatalog,
        "scene.fx.frame" => SceneCommand::FxFrame {
            scene: scene()?,
            at: frame_at(request)?,
        },
        "scene.list" => SceneCommand::List,
        "scene.capabilities" => SceneCommand::Capabilities,
        "scene.catalog" => SceneCommand::Catalog,
        "scene.describe" => SceneCommand::Describe { scene: scene()? },
        "scene.properties" => SceneCommand::Properties {
            scene: scene()?,
            path_prefix: optional_string(request, "prefix")?,
            components: optional_bool(request, "components")?.unwrap_or(false),
        },
        "scene.get" => SceneCommand::Get {
            scene: scene()?,
            property: PropertyRef::parse(string(request, "property")?)?,
        },
        "scene.camera" => SceneCommand::Camera {
            scene: scene()?,
            at: camera_at(request)?,
        },
        // F039: tree, frame evaluation, selection and the gesture cycle.
        "scene.tree" => SceneCommand::Tree {
            scene: scene()?,
            root: optional_string(request, "root")?,
            depth: optional_usize(request, "depth")?,
        },
        "scene.evaluate" => SceneCommand::Evaluate {
            scene: scene()?,
            at: frame_at(request)?,
            gesture: optional_string(request, "gesture")?,
        },
        "scene.plan" => SceneCommand::Plan {
            scene: scene()?,
            at: frame_at(request)?,
            partial: optional_bool(request, "partial")?.unwrap_or(false),
            gesture: optional_string(request, "gesture")?,
        },
        "scene.select" => SceneCommand::Select {
            scene: optional_string(request, "scene")?,
            entities: string_list(request, "entities")?,
            mode: select_mode(request)?,
        },
        "scene.selection" => SceneCommand::Selection,
        "scene.gesture.begin" => SceneCommand::Gesture(GestureCommand::Begin {
            scene: scene()?,
            targets: string_list(request, "targets")?,
            label: optional_string(request, "label")?,
        }),
        "scene.gesture.update" => SceneCommand::Gesture(GestureCommand::Update {
            gesture: string(request, "gesture")?.to_string(),
            ops: parse_batch(
                request
                    .get("ops")
                    .ok_or_else(|| invalid("Expected array field `ops`"))?,
            )?,
        }),
        "scene.gesture.commit" => SceneCommand::Gesture(GestureCommand::Commit {
            gesture: string(request, "gesture")?.to_string(),
        }),
        "scene.gesture.cancel" => SceneCommand::Gesture(GestureCommand::Cancel {
            gesture: string(request, "gesture")?.to_string(),
        }),
        "scene.gesture.status" => SceneCommand::Gesture(GestureCommand::Status),
        // F055: the translation gizmo. Shape only; the geometry and every rule are the core's.
        "scene.gizmo.state" => SceneCommand::Gizmo(GizmoCommand::State {
            scene: scene()?,
            at: frame_at(request)?,
        }),
        "scene.gizmo.hit" => SceneCommand::Gizmo(GizmoCommand::Hit {
            scene: scene()?,
            at: frame_at(request)?,
            point: picture_point(request, "point")?,
        }),
        "scene.pointer.down" => SceneCommand::Gizmo(GizmoCommand::Pointer(PointerCommand::Down {
            scene: scene()?,
            at: frame_at(request)?,
            point: picture_point(request, "point")?,
            additive: optional_bool(request, "additive")?.unwrap_or(false),
        })),
        "scene.pointer.move" => SceneCommand::Gizmo(GizmoCommand::Pointer(PointerCommand::Move {
            point: picture_point(request, "point")?,
        })),
        "scene.pointer.up" => SceneCommand::Gizmo(GizmoCommand::Pointer(PointerCommand::Up {
            point: match request.get("point").filter(|raw| !raw.is_null()) {
                Some(_) => Some(picture_point(request, "point")?),
                None => None,
            },
        })),
        "scene.pointer.cancel" => {
            SceneCommand::Gizmo(GizmoCommand::Pointer(PointerCommand::Cancel {
                reason: cancel_reason(request)?,
            }))
        }
        "scene.translate" => SceneCommand::Gizmo(GizmoCommand::Translate {
            scene: scene()?,
            entity: string(request, "entity")?.to_string(),
            at: frame_at(request)?,
            delta: vector(request, "delta")?,
        }),
        other => {
            return Err(CommandError::new(
                "unknown_operation",
                format!("Unknown operation `{other}`"),
            ));
        }
    })
}

fn optional_bool(request: &Value, key: &str) -> Result<Option<bool>, CommandError> {
    match request.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(invalid(format!("Field `{key}` must be true or false"))),
    }
}

fn optional_usize(request: &Value, key: &str) -> Result<Option<usize>, CommandError> {
    match request.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => raw
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| invalid(format!("Field `{key}` must be a non-negative integer"))),
    }
}

/// A list of strings; absent or `null` is the empty list.
fn string_list(request: &Value, key: &str) -> Result<Vec<String>, CommandError> {
    let bad = || invalid(format!("Field `{key}` must be a list of strings"));
    match request.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_string).ok_or_else(bad))
            .collect(),
        Some(_) => Err(bad()),
    }
}

/// `replace` unless the request says otherwise.
fn select_mode(request: &Value) -> Result<SelectMode, CommandError> {
    let Some(raw) = optional_string(request, "mode")? else {
        return Ok(SelectMode::Replace);
    };
    SelectMode::parse(&raw).ok_or_else(|| {
        CommandError::invalid_value(
            format!("`{raw}` is not a selection mode"),
            json!({ "options": ["replace", "add", "remove", "toggle", "clear"] }),
        )
    })
}

/// `{ "u": 0..1, "v": 0..1 }`: a position on the picture. Range is the core's to judge (a press
/// must be inside, a move may leave), so only the shape is checked here.
fn picture_point(request: &Value, key: &str) -> Result<Point, CommandError> {
    let bad = || {
        invalid(format!(
            "Field `{key}` must be {{\"u\": number, \"v\": number}}"
        ))
    };
    let raw = request.get(key).ok_or_else(bad)?;
    let object = raw.as_object().ok_or_else(bad)?;
    if object.keys().any(|name| name != "u" && name != "v") {
        return Err(bad());
    }
    Ok(Point {
        u: raw.get("u").and_then(Value::as_f64).ok_or_else(bad)?,
        v: raw.get("v").and_then(Value::as_f64).ok_or_else(bad)?,
    })
}

/// `{ "x", "y", "z" }` numbers.
fn vector(request: &Value, key: &str) -> Result<Vec3, CommandError> {
    let bad = || {
        invalid(format!(
            "Field `{key}` must be {{\"x\": number, \"y\": number, \"z\": number}}"
        ))
    };
    let raw = request.get(key).ok_or_else(bad)?;
    let object = raw.as_object().ok_or_else(bad)?;
    if object
        .keys()
        .any(|name| !["x", "y", "z"].contains(&name.as_str()))
    {
        return Err(bad());
    }
    let axis = |name: &str| raw.get(name).and_then(Value::as_f64).ok_or_else(bad);
    Ok(Vec3 {
        x: axis("x")?,
        y: axis("y")?,
        z: axis("z")?,
    })
}

/// `api` unless the request says otherwise.
fn cancel_reason(request: &Value) -> Result<CancelReason, CommandError> {
    let Some(raw) = optional_string(request, "reason")? else {
        return Ok(CancelReason::Api);
    };
    CancelReason::parse(&raw).ok_or_else(|| {
        CommandError::invalid_value(
            format!("`{raw}` is not a cancel reason"),
            json!({ "options": ["escape", "blur", "pointer_cancel", "hidden", "unavailable", "api"] }),
        )
    })
}

/// Exactly one of `frame` or `time` (exact seconds).
fn frame_at(request: &Value) -> Result<FrameAt, CommandError> {
    let frame = request.get("frame").filter(|raw| !raw.is_null());
    match (frame, time_field(request, "time")?) {
        (Some(_), None) => Ok(FrameAt::Frame(integer(request, "frame")?)),
        (None, Some(time)) => Ok(FrameAt::Time(time)),
        _ => Err(invalid("Give exactly one of `frame` or `time`")),
    }
}

/// Exactly one of `bundle` (a JSON object), `text` (the bundle as a string) or `path`. Public so
/// an adapter that attaches off-thread reads the same request shape as `scene.attach`.
pub fn scene_source(request: &Value) -> Result<SceneSource, CommandError> {
    let given: Vec<&str> = ["bundle", "text", "path"]
        .into_iter()
        .filter(|key| request.get(key).is_some_and(|value| !value.is_null()))
        .collect();
    match given.as_slice() {
        ["bundle"] => Ok(SceneSource::Bundle(request["bundle"].clone())),
        ["text"] => Ok(SceneSource::Text(string(request, "text")?.to_string())),
        ["path"] => Ok(SceneSource::Path(PathBuf::from(string(request, "path")?))),
        _ => Err(invalid(
            "Give exactly one of `bundle` (object), `text` (JSON string) or `path` (file)",
        )),
    }
}

/// Exactly one of `frame`, `time` (exact seconds) or `clip` + `clip_frame`.
fn camera_at(request: &Value) -> Result<CameraAt, CommandError> {
    let frame = request.get("frame").filter(|raw| !raw.is_null());
    let time = time_field(request, "time")?;
    let clip = optional_string(request, "clip")?;
    match (frame, time, clip) {
        (Some(_), None, None) => Ok(CameraAt::EditFrame(integer(request, "frame")?)),
        (None, Some(time), None) => Ok(CameraAt::EditTime(time)),
        (None, None, Some(clip)) => Ok(CameraAt::ClipFrame {
            clip,
            frame: integer(request, "clip_frame")?,
        }),
        _ => Err(invalid(
            "Give exactly one of `frame`, `time`, or `clip` with `clip_frame`",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_create_accepts_rate_forms_and_seconds() {
        let ntsc = project_create(
            &json!({"rate":{"num":24000,"den":1001},"seconds":2,"width":640,"height":360}),
        )
        .unwrap();
        assert_eq!(ntsc.sequence.rate, Rate::new(24000, 1001).unwrap());
        assert_eq!(ntsc.sequence.length, 48);
        let plain = project_create(&json!({"rate":30,"length":90,"name":"X"})).unwrap();
        assert_eq!((plain.sequence.rate.num(), plain.sequence.length), (30, 90));
        assert!(project_create(&json!({"length":1,"seconds":1})).is_err());
        assert!(project_create(&json!({"rate":0})).is_err());
    }

    #[test]
    fn unknown_ops_are_not_claimed_and_bad_fields_are_typed_errors() {
        let state = EditorState::demo();
        assert_eq!(parse_op(&state, &json!({"op":"set"})).unwrap(), None);
        assert_eq!(
            parse_op(&state, &json!({"op":"track.add","name":"V","kind":"film"}))
                .unwrap_err()
                .code,
            "invalid_value"
        );
        assert_eq!(
            parse_op(
                &state,
                &json!({"op":"clip.add","track":"t","start":"x","duration":1})
            )
            .unwrap_err()
            .code,
            "invalid_request"
        );
        assert_eq!(
            parse_op(&state, &json!({"op":"seek","frame":1,"time":1}))
                .unwrap_err()
                .code,
            "invalid_request"
        );
    }
}
