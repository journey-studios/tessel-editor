//! Editing the effects layer (`bundle.fx`, F016) atomically, like `batch.rs` edits properties.
//!
//! The batch works on a copy of the document's JSON, checks each op against the catalog and the
//! IDs as it runs (so an error names the op), and hands the final candidate to the importer: the
//! rules of the layer (overlap, range, envelope, limits) live in `directed_scene::fx::read`, not
//! here, and judge an edited bundle exactly as they judge one from disk. On any error nothing
//! exists besides the error.
//!
//! Canonical spelling: parameters are written as the catalog types them (`FxValue::to_json`),
//! fades of 0 and `hidden: false` are left out, and a layer without clips is left out of the
//! document, so adding an effect and removing it again gives back the document byte for byte.

use serde_json::{Map, Value, json};

use crate::directed_scene::fx::{
    self, FX_CATALOG_VERSION, FX_CLIP_EMPTY, FX_CLIP_OUT_OF_RANGE, FxValue, UNKNOWN_CLIP,
    UNKNOWN_EFFECT, UNKNOWN_PARAM, UNKNOWN_TRACK, id_problem, parse_param,
};
use crate::directed_scene::{DirectedScene, INVALID_VALUE, ImportError, Limits};
use crate::time::Ratio;

use super::batch::rejected;
use super::error::{DOCUMENT_MISMATCH, EditError, INVALID_OP};
use super::fx_keys::{self, Batch, FxWork, clip_key_total};

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

    fn is_fade(self) -> bool {
        matches!(self, Self::FadeInFrames | Self::FadeOutFrames)
    }
}

/// A new clip. Everything but the effect and the start is optional: no track means the first
/// track where it fits (or a new one), no ID means `fx-<n>`, no duration means the effect's
/// typical length cut at the end of the edit, no `params` means the catalog defaults.
///
/// The start is a frame of the edit (`start_frame`) **or** an exact time in seconds
/// (`start_time`), which the core turns into the edit frame that contains it
/// (`frame_at_time`, floor, like every other time of `scene.*`). WHY a time: an adapter that
/// knows only the project's playhead must not hand a sequence frame to a bundle that may run at
/// another rate; it hands the time, and the bundle's own clock names the frame. With a
/// `start_time` the `start_frame` field is ignored (it must be 0 in JSON, where it is absent).
#[derive(Debug, Clone, PartialEq)]
pub struct AddClip {
    pub track: Option<String>,
    pub clip: Option<String>,
    pub effect: String,
    pub start_frame: i64,
    pub start_time: Option<Ratio>,
    pub duration_frames: Option<i64>,
    pub params: Vec<(String, Value)>,
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

const SHAPES: &str = r#"{ "op": "add_clip", "effect": "vignette", "start_frame": 0 | "start_time": seconds, "duration_frames"?, "track"?, "id"?, "params"? }, { "op": "remove_clip", "clip" }, { "op": "set_param", "clip", "param", "value" }, { "op": "set_field", "clip", "field": "start_frame|duration_frames|fade_in_frames|fade_out_frames", "value" } { "op": "set_hidden", "clip", "hidden" }, { "op": "key_param", "clip", "param", "frame", "value"?, "easing"? }, { "op": "set_key_easing", "clip", "param", "frame", "easing" }, { "op": "remove_key", "clip", "param", "frame" }, { "op": "move_key", "clip", "param", "from", "to" }, { "op": "shift_keys", "clip", "param"?, "delta" }, { "op": "scale_keys", "clip", "param"?, "anchor", "num", "den" }, { "op": "clear_keys", "clip", "param"? } or { "op": "set_seed", "clip", "seed": uint32|null }"#;

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
                ])?;
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

/// What one op did.
#[derive(Debug, Clone, PartialEq)]
pub struct FxChange {
    pub index: usize,
    /// `add_clip`, `remove_clip`, `set_param`, `set_field` or `set_hidden`.
    pub action: &'static str,
    pub track: String,
    pub clip: String,
    /// What was set (a parameter or field name), when the op sets something.
    pub target: Option<String>,
    pub before: Option<Value>,
    pub after: Option<Value>,
    /// The op wrote what was already there (or removed nothing): the document was left alone.
    pub noop: bool,
    /// What the op did that its author may not have meant (`FX_STATIC_UNDER_KEYS`): the edit
    /// still happened.
    pub warnings: Vec<Value>,
}

impl FxChange {
    pub fn to_json(&self) -> Value {
        json!({
            "index": self.index, "action": self.action, "track": self.track, "clip": self.clip,
            "target": self.target, "before": self.before, "after": self.after, "noop": self.noop,
            "warnings": self.warnings,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FxSummary {
    pub changes: Vec<FxChange>,
    /// What the batch cost, counted (ops run, passes over the layer's keys, curve items copied).
    pub work: FxWork,
    /// Whether the document ends different from where it began (a batch that undoes itself, or
    /// writes what is already there, is not).
    pub modified: bool,
}

impl FxSummary {
    /// The warnings of every op, in order.
    pub fn warnings(&self) -> Vec<Value> {
        self.changes
            .iter()
            .flat_map(|change| change.warnings.iter().cloned())
            .collect()
    }

    pub fn changed_count(&self) -> usize {
        self.changes.iter().filter(|change| !change.noop).count()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FxOutcome {
    pub candidate: DirectedScene,
    pub summary: FxSummary,
}

/// The stable command code (what `CommandError` carries) for the codes of the layer; `None` for
/// the codes `error.rs` already maps.
pub(super) fn command_code(code: &str) -> Option<&'static str> {
    Some(match code {
        fx::UNKNOWN_EFFECT => "unknown_effect",
        fx::UNKNOWN_PARAM => "unknown_param",
        fx::PARAM_TYPE => "param_type",
        fx::PARAM_OUT_OF_RANGE => "param_out_of_range",
        fx::UNKNOWN_TRACK => "unknown_fx_track",
        fx::UNKNOWN_CLIP => "unknown_fx_clip",
        fx::RESERVED_ID => "reserved_id",
        fx::DUPLICATE_ID => "duplicate_id",
        fx::FX_CLIP_OVERLAP => "fx_clip_overlap",
        fx::FX_CLIP_EMPTY => "fx_clip_empty",
        fx::FX_CLIP_OUT_OF_RANGE => "fx_clip_out_of_range",
        fx::ENVELOPE_TOO_LONG => "envelope_too_long",
        fx::CATALOG_TOO_NEW => "catalog_too_new",
        fx::FX_SCOPE_NOT_YET => "fx_scope_not_yet",
        fx::KEY_OUT_OF_CLIP => "key_out_of_clip",
        fx::KEYFRAME_ORDER => "keyframe_order",
        fx::UNKNOWN_KEYFRAME_EASING => "unknown_keyframe_easing",
        fx::UNKNOWN_KEYFRAME => "unknown_keyframe",
        fx::FX_TRANSITION_NOT_YET => "fx_transition_not_yet",
        fx::FX_LIMIT => "fx_limit",
        _ => return None,
    })
}

pub(super) fn mismatch(what: impl Into<String>) -> EditError {
    EditError::new(DOCUMENT_MISMATCH, what)
}

pub(super) fn tracks_mut(work: &mut Value) -> Result<&mut Vec<Value>, EditError> {
    let root = work
        .as_object_mut()
        .ok_or_else(|| mismatch("the document is not an object"))?;
    let layer = root.entry("fx").or_insert_with(
        || json!({ "catalogVersion": FX_CATALOG_VERSION, "tracks": [], "transitions": [] }),
    );
    layer
        .get_mut("tracks")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| mismatch("`/fx/tracks` is not an array"))
}

fn tracks(work: &Value) -> &[Value] {
    work.pointer("/fx/tracks")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

pub(super) fn text_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn clips_of(track: &Value) -> &[Value] {
    track
        .get("clips")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn frame_of(clip: &Value, key: &str) -> i64 {
    clip.get(key)
        .and_then(|value| value.as_i64().or_else(|| value.as_f64().map(|f| f as i64)))
        .unwrap_or(0)
}

/// `(track slot, clip slot)` of a clip ID.
fn find_clip(work: &Value, id: &str) -> Option<(usize, usize)> {
    tracks(work).iter().enumerate().find_map(|(track, entry)| {
        clips_of(entry)
            .iter()
            .position(|clip| text_of(clip, "id") == id)
            .map(|clip| (track, clip))
    })
}

fn every_id(work: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    for track in tracks(work) {
        ids.push(text_of(track, "id").to_string());
        ids.extend(
            clips_of(track)
                .iter()
                .map(|clip| text_of(clip, "id").to_string()),
        );
    }
    ids
}

fn fresh_id(taken: &[String], prefix: &str) -> String {
    (1..)
        .map(|n| format!("{prefix}-{n}"))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| format!("{prefix}-0"))
}

/// `FX n` for the smallest `n` no track carries: removing a track and adding another does not
/// give two tracks the same label (the count of tracks would).
fn fresh_label(tracks: &[Value]) -> String {
    let taken: Vec<&str> = tracks.iter().map(|track| text_of(track, "label")).collect();
    (1..)
        .map(|n| format!("FX {n}"))
        .find(|label| !taken.contains(&label.as_str()))
        .unwrap_or_else(|| "FX".to_string())
}

fn overlaps(track: &Value, start: i64, duration: i64) -> bool {
    clips_of(track).iter().any(|clip| {
        let other_start = frame_of(clip, "startFrame");
        let other_end = other_start.saturating_add(frame_of(clip, "durationFrames"));
        start < other_end && other_start < start.saturating_add(duration)
    })
}

pub(super) fn clip_ref(work: &Value, id: &str) -> Result<(usize, usize), EditError> {
    find_clip(work, id)
        .ok_or_else(|| EditError::new(UNKNOWN_CLIP, format!("no fx clip `{id}`")).with_id(id))
}

fn bad_id(id: &str) -> Option<EditError> {
    id_problem(id).map(|(code, message)| EditError::new(code, message).with_id(id))
}

/// Applies the batch with the importer's default `Limits`. See `apply_fx_with`.
pub fn apply_fx(scene: &DirectedScene, ops: &[FxOp]) -> Result<FxOutcome, EditError> {
    apply_fx_with(scene, ops, &Limits::default())
}

/// Applies the ops in order to a copy of the document and revalidates the result with the
/// importer plus the structural and temporal gates `scene_edit::apply` runs. The first op that
/// fails ends the batch with its position; later ops see what earlier ones wrote.
pub fn apply_fx_with(
    scene: &DirectedScene,
    ops: &[FxOp],
    limits: &Limits,
) -> Result<FxOutcome, EditError> {
    apply_fx_measured(scene, ops, limits).0
}

/// [`apply_fx_with`] and the work the batch did, also for a refused batch (so a test can prove a
/// batch that cannot fit stopped at the op that overflowed instead of running to the end).
pub fn apply_fx_measured(
    scene: &DirectedScene,
    ops: &[FxOp],
    limits: &Limits,
) -> (Result<FxOutcome, EditError>, FxWork) {
    if let Err(error) = check_fx_batch_len(ops.len()) {
        return (Err(error), FxWork::default());
    }
    let mut batch = Batch::default();
    let outcome = run_batch(scene, ops, limits, &mut batch);
    (outcome, batch.work)
}

fn run_batch(
    scene: &DirectedScene,
    ops: &[FxOp],
    limits: &Limits,
    batch: &mut Batch,
) -> Result<FxOutcome, EditError> {
    let mut work = scene.document().clone();
    let edit_length = i64::try_from(scene.bundle().edit_duration_frames).unwrap_or(i64::MAX);
    let mut changes = Vec::with_capacity(ops.len());
    // The clip each op touched: resolved to a pointer at the end, when slots are final.
    let mut touched: Vec<(usize, String)> = Vec::new();
    for (index, op) in ops.iter().enumerate() {
        let fail = |error: EditError| error.at_op(index);
        let resolved = resolve_start(scene, op).map_err(fail)?;
        batch.work.ops_run += 1;
        let change = run_op(
            &mut work,
            batch,
            resolved.as_ref().unwrap_or(op),
            index,
            edit_length,
        )
        .map_err(fail)?;
        if !change.noop {
            touched.push((index, change.clip.clone()));
        }
        changes.push(change);
    }
    // A layer that holds nothing but its empty frame is no layer: the baseline document has no
    // `fx` at all (a layer with other keys, kept as written, stays).
    // Only an edit that changed something may tidy it: a batch that changed nothing (an empty one, or
    // only no-ops) leaves a layer the author wrote bare exactly as written, so no revision, no undo entry.
    let bare = work != *scene.document()
        && !touched.is_empty()
        && work
            .get("fx")
            .and_then(Value::as_object)
            .is_some_and(|layer| {
                layer.len() == 3
                    && layer.get("catalogVersion") == Some(&json!(FX_CATALOG_VERSION))
                    && layer
                        .get("tracks")
                        .and_then(Value::as_array)
                        .is_some_and(Vec::is_empty)
                    && layer
                        .get("transitions")
                        .and_then(Value::as_array)
                        .is_some_and(Vec::is_empty)
            });
    if bare && let Some(root) = work.as_object_mut() {
        root.remove("fx");
    }
    let modified = work != *scene.document();
    let pointers: Vec<(usize, String)> = touched
        .iter()
        .filter_map(|(index, clip)| {
            find_clip(&work, clip)
                .map(|(track, slot)| (*index, format!("/fx/tracks/{track}/clips/{slot}")))
        })
        .collect();
    let candidate = DirectedScene::import_value_with(work, limits)
        .map_err(|error| rejected(error, &pointers))?;
    let mut issues = candidate.edit_join_issues();
    issues.extend(candidate.temporal_issues());
    if !issues.is_empty() {
        return Err(rejected(ImportError::Invalid { issues }, &pointers));
    }
    let candidate = if modified { candidate } else { scene.clone() };
    Ok(FxOutcome {
        candidate,
        summary: FxSummary {
            changes,
            work: batch.work,
            modified,
        },
    })
}

/// The op with its `start_time` turned into the edit frame that contains it, or `None` when
/// the op has no time to convert. The conversion is the bundle's own clock, so a time that no
/// frame of the edit holds is refused as `fx_clip_out_of_range` by the range check that follows,
/// not clamped.
fn resolve_start(scene: &DirectedScene, op: &FxOp) -> Result<Option<FxOp>, EditError> {
    let FxOp::AddClip(add) = op else {
        return Ok(None);
    };
    let Some(time) = add.start_time else {
        return Ok(None);
    };
    let frame = scene.bundle().frame_at_time(time).map_err(|error| {
        EditError::new(
            FX_CLIP_OUT_OF_RANGE,
            format!("time {time} s is not a frame of the edit: {error}"),
        )
    })?;
    Ok(Some(FxOp::AddClip(AddClip {
        start_frame: frame,
        start_time: None,
        ..add.clone()
    })))
}

/// Validates the batch exactly as `apply_fx` would and reports what it would change.
pub fn dry_run_fx(scene: &DirectedScene, ops: &[FxOp]) -> Result<FxSummary, EditError> {
    apply_fx(scene, ops).map(|outcome| outcome.summary)
}

fn run_op(
    work: &mut Value,
    batch: &mut Batch,
    op: &FxOp,
    index: usize,
    edit_length: i64,
) -> Result<FxChange, EditError> {
    match op {
        FxOp::AddClip(add) => add_clip(work, add, index, edit_length),
        FxOp::RemoveClip { clip } => {
            let (track_slot, clip_slot) = clip_ref(work, clip)?;
            let tracks = tracks_mut(work)?;
            let track_id = text_of(&tracks[track_slot], "id").to_string();
            let removed = remove_at(&mut tracks[track_slot], clip_slot);
            // The keys of the clip leave the layer with it: their room is free again.
            batch.shrank(clip_key_total(removed.as_ref().and_then(Value::as_object)));
            let track_empty = clips_of(&tracks[track_slot]).is_empty();
            if track_empty {
                tracks.remove(track_slot);
            }
            Ok(FxChange {
                index,
                warnings: Vec::new(),
                action: "remove_clip",
                track: track_id,
                clip: clip.clone(),
                target: None,
                before: removed,
                after: None,
                noop: false,
            })
        }
        FxOp::SetParam { clip, param, value } => set_param(work, clip, param, value, index),
        FxOp::SetField { clip, field, value } => set_field(work, clip, *field, *value, index),
        FxOp::KeyParam {
            clip,
            param,
            frame,
            value,
            easing,
        } => fx_keys::key_param(
            work,
            batch,
            clip,
            param,
            *frame,
            value.as_ref(),
            easing.as_deref(),
            index,
        ),
        FxOp::SetKeyEasing {
            clip,
            param,
            frame,
            easing,
        } => fx_keys::set_key_easing(work, clip, param, *frame, easing, index),
        FxOp::RemoveKey { clip, param, frame } => {
            fx_keys::remove_key(work, batch, clip, param, *frame, index)
        }
        FxOp::MoveKey {
            clip,
            param,
            from,
            to,
        } => fx_keys::move_key(work, clip, param, *from, *to, index),
        FxOp::ShiftKeys { clip, param, delta } => {
            fx_keys::shift_keys(work, batch, clip, param.as_deref(), *delta, index)
        }
        FxOp::ScaleKeys {
            clip,
            param,
            anchor,
            num,
            den,
        } => fx_keys::scale_keys(
            work,
            batch,
            clip,
            param.as_deref(),
            *anchor,
            (*num, *den),
            index,
        ),
        FxOp::ClearKeys { clip, param } => {
            fx_keys::clear_keys(work, batch, clip, param.as_deref(), index)
        }
        FxOp::SetSeed { clip, seed } => fx_keys::set_seed(work, clip, *seed, index),
        FxOp::SetHidden { clip, hidden } => {
            let (track_slot, clip_slot) = clip_ref(work, clip)?;
            let tracks = tracks_mut(work)?;
            let track_id = text_of(&tracks[track_slot], "id").to_string();
            let target = clip_object(&mut tracks[track_slot], clip_slot)?;
            let before = target.get("hidden") == Some(&Value::Bool(true));
            if *hidden != before {
                if *hidden {
                    target.insert("hidden".to_string(), Value::Bool(true));
                } else {
                    target.remove("hidden");
                }
            }
            Ok(FxChange {
                index,
                warnings: Vec::new(),
                action: "set_hidden",
                track: track_id,
                clip: clip.clone(),
                target: Some("hidden".to_string()),
                before: Some(json!(before)),
                after: Some(json!(hidden)),
                noop: *hidden == before,
            })
        }
    }
}

fn remove_at(track: &mut Value, clip_slot: usize) -> Option<Value> {
    track
        .get_mut("clips")
        .and_then(Value::as_array_mut)
        .map(|clips| clips.remove(clip_slot))
}

pub(super) fn clip_object(
    track: &mut Value,
    clip_slot: usize,
) -> Result<&mut Map<String, Value>, EditError> {
    track
        .get_mut("clips")
        .and_then(Value::as_array_mut)
        .and_then(|clips| clips.get_mut(clip_slot))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| mismatch("an fx clip is not an object"))
}

fn add_clip(
    work: &mut Value,
    add: &AddClip,
    index: usize,
    edit_length: i64,
) -> Result<FxChange, EditError> {
    let spec = fx::effect(&add.effect).ok_or_else(|| {
        EditError::new(
            UNKNOWN_EFFECT,
            format!(
                "`{}` is not an effect; known: {}",
                add.effect,
                fx::catalog()
                    .iter()
                    .map(|spec| spec.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
        .with_id(add.effect.as_str())
    })?;
    if add.start_frame < 0 || add.start_frame >= edit_length {
        return Err(EditError::new(
            FX_CLIP_OUT_OF_RANGE,
            format!(
                "an effect starts inside the edit (frames 0 to {}), not at {}",
                edit_length - 1,
                add.start_frame
            ),
        ));
    }
    let duration = match add.duration_frames {
        Some(duration) if duration < 1 => {
            return Err(EditError::new(
                FX_CLIP_EMPTY,
                format!("a clip lasts at least 1 frame, not {duration}"),
            ));
        }
        Some(duration) => duration,
        None => spec
            .typical_duration_frames
            .min(edit_length - add.start_frame),
    };
    let mut ids = every_id(work);
    let clip_id = match &add.clip {
        Some(id) => {
            if let Some(problem) = bad_id(id) {
                return Err(problem);
            }
            if ids.contains(id) {
                return Err(EditError::new(
                    fx::DUPLICATE_ID,
                    format!("an fx track or clip `{id}` already exists"),
                )
                .with_id(id.as_str()));
            }
            id.clone()
        }
        None => fresh_id(&ids, "fx"),
    };
    // The clip's ID is taken from here on: a track made for it must not be given the same one
    // (`id: "fx-track-1"` on a clip that needs a new track).
    ids.push(clip_id.clone());
    let mut params = Map::new();
    for (name, value) in &add.params {
        let (_, param) = spec.param(name).ok_or_else(|| {
            EditError::new(
                UNKNOWN_PARAM,
                format!("`{name}` is not a parameter of {}", spec.id),
            )
            .with_path(name.as_str())
        })?;
        let parsed = parse_param(param, value)
            .map_err(|(code, message)| EditError::new(code, message).with_path(name.as_str()))?;
        params.insert(name.clone(), parsed.to_json());
    }
    let mut clip = Map::new();
    clip.insert("id".to_string(), json!(clip_id));
    clip.insert("effect".to_string(), json!(spec.id));
    clip.insert("startFrame".to_string(), json!(add.start_frame));
    clip.insert("durationFrames".to_string(), json!(duration));
    if !params.is_empty() {
        clip.insert("params".to_string(), Value::Object(params));
    }
    let all = tracks_mut(work)?;
    let track_slot = match &add.track {
        Some(id) => all
            .iter()
            .position(|track| text_of(track, "id") == id)
            .ok_or_else(|| {
                EditError::new(UNKNOWN_TRACK, format!("no fx track `{id}`")).with_id(id.as_str())
            })?,
        // No track named: the first VISIBLE track where it fits (a clip on a hidden track would draw
        // nothing, an effect nobody sees), or a new one. A hidden track is still a track one can name.
        None => match all.iter().position(|track| {
            track.get("hidden") != Some(&Value::Bool(true))
                && !overlaps(track, add.start_frame, duration)
        }) {
            Some(slot) => slot,
            None => {
                let track_id = fresh_id(&ids, "fx-track");
                let label = fresh_label(all);
                all.push(json!({
                    "id": track_id,
                    "label": label,
                    "clips": [],
                }));
                all.len() - 1
            }
        },
    };
    let track_id = text_of(&all[track_slot], "id").to_string();
    let clips = all[track_slot]
        .get_mut("clips")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| mismatch("an fx track has no `clips` array"))?;
    let written = Value::Object(clip);
    clips.push(written.clone());
    Ok(FxChange {
        index,
        warnings: Vec::new(),
        action: "add_clip",
        track: track_id,
        clip: clip_id,
        target: None,
        before: None,
        after: Some(written),
        noop: false,
    })
}

fn clip_effect(clip: &Map<String, Value>) -> Result<&'static fx::EffectSpec, EditError> {
    let name = clip
        .get("effect")
        .and_then(Value::as_str)
        .unwrap_or_default();
    fx::effect(name)
        .ok_or_else(|| EditError::new(UNKNOWN_EFFECT, format!("clip names the effect `{name}`")))
}

fn set_param(
    work: &mut Value,
    clip: &str,
    param: &str,
    value: &Value,
    index: usize,
) -> Result<FxChange, EditError> {
    let (track_slot, clip_slot) = clip_ref(work, clip)?;
    let tracks = tracks_mut(work)?;
    let track_id = text_of(&tracks[track_slot], "id").to_string();
    let target = clip_object(&mut tracks[track_slot], clip_slot)?;
    let spec = clip_effect(target)?;
    let (_, catalog_param) = spec.param(param).ok_or_else(|| {
        EditError::new(
            UNKNOWN_PARAM,
            format!(
                "`{param}` is not a parameter of {}; known: {}",
                spec.id,
                spec.params
                    .iter()
                    .map(|param| param.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
        .with_path(param)
    })?;
    let after = parse_param(catalog_param, value)
        .map_err(|(code, message)| EditError::new(code, message).with_path(param))?;
    let before: FxValue = target
        .get("params")
        .and_then(|params| params.get(param))
        .and_then(|raw| parse_param(catalog_param, raw).ok())
        .unwrap_or_else(|| catalog_param.kind.default_value());
    // A no-op only when the very value is already written: a parameter the clip leaves out
    // reads as the default, and writing the default down is a change of the document.
    let written = target
        .get("params")
        .and_then(|params| params.get(param))
        .and_then(|raw| parse_param(catalog_param, raw).ok());
    let noop = written == Some(after);
    if !noop {
        let params = target
            .entry("params")
            .or_insert_with(|| Value::Object(Map::new()));
        let Some(params) = params.as_object_mut() else {
            return Err(mismatch("`params` of a clip is not an object"));
        };
        params.insert(param.to_string(), after.to_json());
    }
    // A static write under an animation: the base changes, the curve stays, and the author is told.
    let warnings = if noop {
        Vec::new()
    } else {
        fx_keys::static_under_keys(target, clip, param)
            .into_iter()
            .collect()
    };
    Ok(FxChange {
        index,
        warnings,
        action: "set_param",
        track: track_id,
        clip: clip.to_string(),
        target: Some(param.to_string()),
        before: Some(before.to_json()),
        after: Some(after.to_json()),
        noop,
    })
}

fn set_field(
    work: &mut Value,
    clip: &str,
    field: ClipField,
    value: i64,
    index: usize,
) -> Result<FxChange, EditError> {
    if field.is_fade() && value < 0 {
        return Err(EditError::new(
            INVALID_VALUE,
            format!("`{}` is not negative, got {value}", field.name()),
        )
        .with_path(field.name()));
    }
    if field == ClipField::DurationFrames && value < 1 {
        return Err(EditError::new(
            FX_CLIP_EMPTY,
            format!("a clip lasts at least 1 frame, not {value}"),
        )
        .with_path(field.name()));
    }
    let (track_slot, clip_slot) = clip_ref(work, clip)?;
    let tracks = tracks_mut(work)?;
    let track_id = text_of(&tracks[track_slot], "id").to_string();
    let target = clip_object(&mut tracks[track_slot], clip_slot)?;
    let before = target
        .get(field.key())
        .and_then(|raw| raw.as_i64().or_else(|| raw.as_f64().map(|f| f as i64)))
        .unwrap_or(0);
    let noop = before == value;
    if !noop {
        if field.is_fade() && value == 0 {
            target.remove(field.key());
        } else {
            target.insert(field.key().to_string(), json!(value));
        }
    }
    Ok(FxChange {
        index,
        warnings: Vec::new(),
        action: "set_field",
        track: track_id,
        clip: clip.to_string(),
        target: Some(field.name().to_string()),
        before: Some(json!(before)),
        after: Some(json!(value)),
        noop,
    })
}
