//! The ops of `scene.fx.edit` and their JSON shapes (F016): what a batch is made of, before any
//! document is touched. Moved out of `fx.rs` unchanged (BL082.4.5, P4b S3a); the batch runner
//! stays there.

use serde_json::Value;

use crate::directed_scene::fx::{self, FxScope};
use crate::time::Ratio;

use super::error::{EditError, INVALID_OP};

/// The most ops one `scene.fx.edit` batch (or one gesture update) may hold. WHY a budget: each op
/// locates its clip and copies at most one curve, so the work of a batch is bounded only when its
/// length is; past it the batch is refused whole, as `FX_LIMIT` at `ops`, before the document is
/// copied or an op is parsed or run. A real edit (a drag, a retime, a form) is a handful of ops;
/// split a bulk import into several batches, each one undo entry.
pub const MAX_FX_BATCH_OPS: usize = 1024;

/// Refuses a batch of `count` ops past [`MAX_FX_BATCH_OPS`] (the check every entry point makes
/// first, before it copies, parses or runs anything).
pub fn check_fx_batch_len(count: usize) -> Result<(), EditError> {
    if count > MAX_FX_BATCH_OPS {
        Err(batch_too_long(count))
    } else {
        Ok(())
    }
}

/// The refusal of a batch that holds more ops than [`MAX_FX_BATCH_OPS`].
fn batch_too_long(count: usize) -> EditError {
    EditError::new(
        fx::FX_LIMIT,
        format!(
            "a batch holds at most {MAX_FX_BATCH_OPS} fx ops, got {count}; split it into several \
             batches (each one undo entry)"
        ),
    )
    .with_path("ops")
}

/// A numeric field of a clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipField {
    StartFrame,
    DurationFrames,
    FadeInFrames,
    FadeOutFrames,
}

impl ClipField {
    pub const ALL: [Self; 4] = [
        Self::StartFrame,
        Self::DurationFrames,
        Self::FadeInFrames,
        Self::FadeOutFrames,
    ];

    /// The key in the document.
    pub fn key(self) -> &'static str {
        match self {
            Self::StartFrame => "startFrame",
            Self::DurationFrames => "durationFrames",
            Self::FadeInFrames => "fadeInFrames",
            Self::FadeOutFrames => "fadeOutFrames",
        }
    }

    /// The name in commands and the UI.
    pub fn name(self) -> &'static str {
        match self {
            Self::StartFrame => "start_frame",
            Self::DurationFrames => "duration_frames",
            Self::FadeInFrames => "fade_in_frames",
            Self::FadeOutFrames => "fade_out_frames",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.name() == name)
    }

    pub(super) fn is_fade(self) -> bool {
        matches!(self, Self::FadeInFrames | Self::FadeOutFrames)
    }
}

/// A new clip. Everything but the effect and the start is optional: no track means the first
/// track where it fits (or a new one), no ID means `fx-<n>`, no duration means the effect's
/// typical length cut where its scope ends ([`fx::default_duration`]: the shot's end or the
/// edit's), no `params` means the catalog defaults. A duration that is given is never cut: it is
/// judged as it is and refused when it does not fit.
///
/// The start is a frame of the edit (`start_frame`) **or** an exact time in seconds
/// (`start_time`), which the core turns into the edit frame that contains it
/// (`frame_at_time`, floor, like every other time of `scene.*`). WHY a time: an adapter that
/// knows only the project's playhead must not hand a sequence frame to a bundle that may run at
/// another rate; it hands the time, and the bundle's own clock names the frame. With a
/// `start_time` the `start_frame` field is ignored (it must be 0 in JSON, where it is absent).
///
/// The `scope` is where the clip lies (F062): the output, the default (the key stays out of the
/// document), or a shot, which the clip must lie entirely inside; an add that does not fit is
/// refused whole (`FX_SCOPE_MISMATCH`, `UNKNOWN_SHOT`) and nothing is written.
#[derive(Debug, Clone, PartialEq)]
pub struct AddClip {
    pub track: Option<String>,
    pub clip: Option<String>,
    pub effect: String,
    pub start_frame: i64,
    pub start_time: Option<Ratio>,
    pub duration_frames: Option<i64>,
    pub params: Vec<(String, Value)>,
    pub scope: FxScope,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FxOp {
    AddClip(AddClip),
    RemoveClip {
        clip: String,
    },
    /// Sets a catalog parameter. The value is checked against the effect's catalog entry.
    SetParam {
        clip: String,
        param: String,
        value: Value,
    },
    SetField {
        clip: String,
        field: ClipField,
        value: i64,
    },
    SetHidden {
        clip: String,
        hidden: bool,
    },
    /// Moves the clip to the output or to a shot (F062). A shot must exist and hold the whole
    /// clip; the same scope as the clip has is a no-op, and going back to `output` leaves the
    /// `scope` key out of the document again.
    SetScope {
        clip: String,
        scope: FxScope,
    },
    /// Puts a key of `param` on `frame` of the clip (or changes the one there). No `value`
    /// captures what the parameter is at that frame; no `easing` keeps the key's own.
    KeyParam {
        clip: String,
        param: String,
        frame: i64,
        value: Option<Value>,
        easing: Option<String>,
    },
    /// The easing of the span that arrives at the key on `frame`.
    SetKeyEasing {
        clip: String,
        param: String,
        frame: i64,
        easing: String,
    },
    RemoveKey {
        clip: String,
        param: String,
        frame: i64,
    },
    MoveKey {
        clip: String,
        param: String,
        from: i64,
        to: i64,
    },
    /// Every key of `param` (or of the whole clip) `delta` frames later.
    ShiftKeys {
        clip: String,
        param: Option<String>,
        delta: i64,
    },
    /// The distance of every key from `anchor` times `num / den`.
    ScaleKeys {
        clip: String,
        param: Option<String>,
        anchor: i64,
        num: i64,
        den: i64,
    },
    /// The explicit removal of an animation; the base values stay.
    ClearKeys {
        clip: String,
        param: Option<String>,
    },
    /// The base seed of a noise effect; `None` goes back to the one the clip's ID gives.
    SetSeed {
        clip: String,
        seed: Option<u32>,
    },
}

const SHAPES: &str = r#"{ "op": "add_clip", "effect": "vignette", "start_frame": 0 | "start_time": seconds, "duration_frames"?, "track"?, "id"?, "params"?, "scope"?: "output|shot:<id>" }, { "op": "remove_clip", "clip" }, { "op": "set_param", "clip", "param", "value" }, { "op": "set_field", "clip", "field": "start_frame|duration_frames|fade_in_frames|fade_out_frames", "value" }, { "op": "set_hidden", "clip", "hidden" }, { "op": "set_scope", "clip", "scope": "output|shot:<id>" }, { "op": "key_param", "clip", "param", "frame", "value"?, "easing"? }, { "op": "set_key_easing", "clip", "param", "frame", "easing" }, { "op": "remove_key", "clip", "param", "frame" }, { "op": "move_key", "clip", "param", "from", "to" }, { "op": "shift_keys", "clip", "param"?, "delta" }, { "op": "scale_keys", "clip", "param"?, "anchor", "num", "den" }, { "op": "clear_keys", "clip", "param"? } or { "op": "set_seed", "clip", "seed": uint32|null }"#;

impl FxOp {
    /// The JSON shape of an op (the `ops` of `scene.fx.edit`).
    pub fn from_json(value: &Value) -> Result<Self, EditError> {
        let invalid =
            |message: String| EditError::new(INVALID_OP, format!("{message}; expected {SHAPES}"));
        let Value::Object(map) = value else {
            return Err(invalid("an fx op must be an object".to_string()));
        };
        let text = |key: &str| -> Result<String, EditError> {
            map.get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| invalid(format!("`{key}` must be a string")))
        };
        let optional_text = |key: &str| -> Result<Option<String>, EditError> {
            match map.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(text)) => Ok(Some(text.clone())),
                Some(_) => Err(invalid(format!("`{key}` must be a string"))),
            }
        };
        let integer = |key: &str| -> Result<i64, EditError> {
            map.get(key)
                .and_then(Value::as_i64)
                .ok_or_else(|| invalid(format!("`{key}` must be a whole number")))
        };
        let only = |keys: &[&str]| -> Result<(), EditError> {
            match map.keys().find(|key| !keys.contains(&key.as_str())) {
                Some(extra) => Err(invalid(format!("`{extra}` is not a field of this op"))),
                None => Ok(()),
            }
        };
        let op = text("op")?;
        match op.as_str() {
            "add_clip" => {
                only(&[
                    "op",
                    "effect",
                    "start_frame",
                    "start_time",
                    "duration_frames",
                    "track",
                    "id",
                    "params",
                    "scope",
                ])?;
                // Left out or `null`: the output, the clip every add made before F062.
                let scope = match map.get("scope") {
                    None | Some(Value::Null) => FxScope::Output,
                    Some(raw) => scope_of(raw)?,
                };
                let params = match map.get("params") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(Value::Object(params)) => params
                        .iter()
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect(),
                    Some(_) => return Err(invalid("`params` must be an object".to_string())),
                };
                let duration_frames = match map.get("duration_frames") {
                    None | Some(Value::Null) => None,
                    Some(_) => Some(integer("duration_frames")?),
                };
                let has = |key: &str| map.get(key).is_some_and(|value| !value.is_null());
                let (start_frame, start_time) = match (has("start_frame"), has("start_time")) {
                    (true, false) => (integer("start_frame")?, None),
                    (false, true) => {
                        let time = map
                            .get("start_time")
                            .and_then(Ratio::from_json)
                            .ok_or_else(|| {
                                invalid(
                                    "`start_time` is seconds: an integer or {\"num\":1,\"den\":24}"
                                        .to_string(),
                                )
                            })?;
                        (0, Some(time))
                    }
                    _ => {
                        return Err(invalid(
                            "give exactly one of `start_frame` or `start_time`".to_string(),
                        ));
                    }
                };
                Ok(Self::AddClip(AddClip {
                    track: optional_text("track")?,
                    clip: optional_text("id")?,
                    effect: text("effect")?,
                    start_frame,
                    start_time,
                    duration_frames,
                    params,
                    scope,
                }))
            }
            "remove_clip" => {
                only(&["op", "clip"])?;
                Ok(Self::RemoveClip {
                    clip: text("clip")?,
                })
            }
            "set_param" => {
                only(&["op", "clip", "param", "value"])?;
                Ok(Self::SetParam {
                    clip: text("clip")?,
                    param: text("param")?,
                    value: map
                        .get("value")
                        .cloned()
                        .ok_or_else(|| invalid("`value` is required".to_string()))?,
                })
            }
            "set_field" => {
                only(&["op", "clip", "field", "value"])?;
                let field = text("field")?;
                let field = ClipField::parse(&field)
                    .ok_or_else(|| invalid(format!("`{field}` is not a clip field")))?;
                Ok(Self::SetField {
                    clip: text("clip")?,
                    field,
                    value: integer("value")?,
                })
            }
            "set_hidden" => {
                only(&["op", "clip", "hidden"])?;
                let hidden = map
                    .get("hidden")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| invalid("`hidden` must be true or false".to_string()))?;
                Ok(Self::SetHidden {
                    clip: text("clip")?,
                    hidden,
                })
            }
            "set_scope" => {
                only(&["op", "clip", "scope"])?;
                // Required, and `null` is not a scope: going back to the output is `output`.
                let raw = map.get("scope").ok_or_else(|| {
                    invalid("`scope` is required: `output` or `shot:<id>`".to_string())
                })?;
                Ok(Self::SetScope {
                    clip: text("clip")?,
                    scope: scope_of(raw)?,
                })
            }
            "key_param" => {
                only(&["op", "clip", "param", "frame", "value", "easing"])?;
                Ok(Self::KeyParam {
                    clip: text("clip")?,
                    param: text("param")?,
                    frame: integer("frame")?,
                    value: map.get("value").filter(|value| !value.is_null()).cloned(),
                    easing: optional_text("easing")?,
                })
            }
            "set_key_easing" => {
                only(&["op", "clip", "param", "frame", "easing"])?;
                Ok(Self::SetKeyEasing {
                    clip: text("clip")?,
                    param: text("param")?,
                    frame: integer("frame")?,
                    easing: text("easing")?,
                })
            }
            "remove_key" => {
                only(&["op", "clip", "param", "frame"])?;
                Ok(Self::RemoveKey {
                    clip: text("clip")?,
                    param: text("param")?,
                    frame: integer("frame")?,
                })
            }
            "move_key" => {
                only(&["op", "clip", "param", "from", "to"])?;
                Ok(Self::MoveKey {
                    clip: text("clip")?,
                    param: text("param")?,
                    from: integer("from")?,
                    to: integer("to")?,
                })
            }
            "shift_keys" => {
                only(&["op", "clip", "param", "delta"])?;
                Ok(Self::ShiftKeys {
                    clip: text("clip")?,
                    param: optional_text("param")?,
                    delta: integer("delta")?,
                })
            }
            "scale_keys" => {
                only(&["op", "clip", "param", "anchor", "num", "den"])?;
                Ok(Self::ScaleKeys {
                    clip: text("clip")?,
                    param: optional_text("param")?,
                    anchor: integer("anchor")?,
                    num: integer("num")?,
                    den: integer("den")?,
                })
            }
            "clear_keys" => {
                only(&["op", "clip", "param"])?;
                Ok(Self::ClearKeys {
                    clip: text("clip")?,
                    param: optional_text("param")?,
                })
            }
            "set_seed" => {
                only(&["op", "clip", "seed"])?;
                let seed = match map.get("seed") {
                    Some(Value::Null) => None,
                    Some(raw) => Some(
                        raw.as_u64()
                            .and_then(|seed| u32::try_from(seed).ok())
                            .ok_or_else(|| {
                                invalid(
                                    "`seed` is a whole number from 0 to 4294967295, or null"
                                        .to_string(),
                                )
                            })?,
                    ),
                    None => {
                        return Err(invalid(
                            "`seed` is required (a number, or null for the automatic one)"
                                .to_string(),
                        ));
                    }
                };
                Ok(Self::SetSeed {
                    clip: text("clip")?,
                    seed,
                })
            }
            other => Err(invalid(format!("`{other}` is not an fx op"))),
        }
    }

    /// The clip an op targets (for an add, the explicit ID if there is one).
    pub fn clip_id(&self) -> Option<&str> {
        match self {
            Self::AddClip(add) => add.clip.as_deref(),
            Self::RemoveClip { clip }
            | Self::SetParam { clip, .. }
            | Self::SetField { clip, .. }
            | Self::SetHidden { clip, .. }
            | Self::SetScope { clip, .. }
            | Self::KeyParam { clip, .. }
            | Self::SetKeyEasing { clip, .. }
            | Self::RemoveKey { clip, .. }
            | Self::MoveKey { clip, .. }
            | Self::ShiftKeys { clip, .. }
            | Self::ScaleKeys { clip, .. }
            | Self::ClearKeys { clip, .. }
            | Self::SetSeed { clip, .. } => Some(clip),
        }
    }
}

/// A scope an op carries, from its text (`output`, `shot:<id>`) or the document's own object. What
/// is not a scope is `INVALID_VALUE` and scope `node` is `FX_SCOPE_NOT_YET`, both at the path
/// `scope`: an op that cannot say where its clip goes is refused before the document is read.
fn scope_of(raw: &Value) -> Result<FxScope, EditError> {
    FxScope::from_op_json(raw)
        .map_err(|(code, message)| EditError::new(code, message).with_path("scope"))
}

/// `ops` as the JSON array of `scene.fx.edit`.
pub fn parse_fx_batch(value: &Value) -> Result<Vec<FxOp>, EditError> {
    let Some(items) = value.as_array() else {
        return Err(EditError::new(
            INVALID_OP,
            "a batch must be an array of fx ops",
        ));
    };
    check_fx_batch_len(items.len())?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| FxOp::from_json(item).map_err(|error| error.at_op(index)))
        .collect()
}
