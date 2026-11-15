//! Duration and cuts of the shots of a directed scene (F065, P5 S1, BL082.5.2).
//!
//! Four ops over the `shots[]` of a render-bundle 1.0 document: `trim_shot`, `move_cut`,
//! `set_shot_duration` and `set_edit_duration`, with the options `mode` (`ripple`/`roll`),
//! `content` (`anchor`/`follow`/`auto`), `keep_absolute`, `shrink_transitions` and
//! `with_narration`. The contract is the pilot's decisions #1..#34
//! (`docs/migration/origin-specs/scene-shot-trim-decisions.md`), restated in
//! `docs/features/shot-duration.md` and `docs/migration/p5-shot-duration-plan.md`.
//!
//! * [`plan`] is the pure planner: typed shots in, a [`ShotPlan`] (new ranges, the head frames
//!   that stay or follow, the growing instances, the chain of splices) or a [`Refusal`] out.
//!   No JSON, no I/O.
//! * `apply` writes the plan into a copy of the document, op after op (D02), and hands the
//!   candidate to the same gate every other edit uses: the importer, then the structural and the
//!   temporal join rules. A finding is `CANDIDATE_REJECTED` linked to the op by JSON pointer.
//!
//! **This slice (S1) moves the shots, the instances and the edit length only.** The layers that
//! count edit frames (effect clips, transitions, light keys, `weatherKeys`) are S2's: a document
//! that needs one of them moved fails the gate (`FX_SCOPE_MISMATCH`, `FX_CLIP_OUT_OF_RANGE`,
//! `TRANSITION_TOO_LONG`, ...), it is not repaired. `keep_absolute` and `shrink_transitions` are
//! parsed and carried, and have no effect until then. No property of a shot is in the catalog
//! (D15): a property would write an `editRange` without the ripple.
//!
//! The command (`SceneCommand`), the JSONL op and the localized texts are S3's.

mod apply;
mod error;
mod plan;

pub use apply::{
    AUTO_FELL_TO_FOLLOW, INSTANCE_GREW, ShotNote, ShotsOutcome, apply_shots, apply_shots_with,
};
pub use error::{
    CODES, CONTINUOUS_JOIN, EDIT_TOO_LONG, Facts, LOCKED, NO_HEAD_FRAMES, Refusal, SHOT_NOT_FOUND,
    SHOT_TOO_SHORT, SHOTS_LAYER_NOT_YET, SHOTS_NOT_ADJACENT, SHOTS_OPTION_NOT_YET,
};
pub use plan::{
    FollowFallback, Growth, MovedEdge, Placement, PlanInput, PlanInstance, PlanShot, PlanTarget,
    ShotPlan, Splice, plan_op,
};

use serde_json::{Map, Number, Value};

use super::error::{EditError, INVALID_OP};

/// One hour at 60 fps (decision #18).
pub const MAX_EDIT_FRAMES: i64 = 216_000;

/// A batch holds at most this many ops. The whole batch works on one clone of the document, but
/// each op is still planned and written, so the bound keeps a hostile batch from costing more
/// than a person's edit would.
pub const MAX_SHOT_BATCH_OPS: usize = 256;

/// A frame in an op or in the document is read as an `i64` and must be within `+-FRAME_BOUND`
/// (2^53 - 1, the importer's `MAX_SAFE_INT` in `directed_scene/read.rs`, the largest integer
/// JavaScript carries exactly): every sum the planner makes then fits in an `i64`.
pub(super) const FRAME_BOUND: i64 = (1 << 53) - 1;

/// Whether `frame` is within `+-FRAME_BOUND`. A range test, never `abs()` (`i64::MIN.abs()` panics).
pub(super) fn in_bound(frame: i64) -> bool {
    (-FRAME_BOUND..=FRAME_BOUND).contains(&frame)
}

/// An integer-valued JSON number read as the importer reads a frame (`12` and `12.0` both count,
/// as in JavaScript), within `+-FRAME_BOUND`. Replicates `as_integer` + the range check of
/// `directed_scene/read.rs` (`Obj::int`), which this module does not own.
pub(super) fn lenient_int(value: &Value) -> Option<i64> {
    let Value::Number(number) = value else {
        return None;
    };
    let integer = number.as_i64().or_else(|| float_integer(number))?;
    in_bound(integer).then_some(integer)
}

fn float_integer(number: &Number) -> Option<i64> {
    let float = number.as_f64()?;
    // `|float| <= 2^53` is exact in f64; the caller's range test rejects what is past 2^53 - 1.
    (float.is_finite() && float.fract() == 0.0 && float.abs() <= 9.007_199_254_740_992e15)
        .then_some(float as i64)
}

/// Whether a ripple moves what follows (`ripple`) or the neighbour absorbs the change (`roll`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Ripple,
    Roll,
}

/// What happens to the picture when the entering side of a shot goes earlier (#28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Content {
    /// The picture stays on the same edit frames; the shot gains head frames or is refused
    /// (`NO_HEAD_FRAMES`).
    Anchor,
    /// The scene starts at the new cut; the whole picture of the shot slides earlier.
    Follow,
    /// `anchor` with the head frames that exist (less the transition reserve), `follow` for the
    /// rest; never refuses for lack of head.
    #[default]
    Auto,
}

/// A border of a shot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

impl Edge {
    pub fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::End => "end",
        }
    }
}

/// The options of an op (#19). The defaults are the origin's: `ripple`, `auto`, everything else off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShotOptions {
    pub mode: Mode,
    pub content: Content,
    /// Layers in edit frames stay where they are instead of following the splice chain (S2).
    pub keep_absolute: bool,
    /// Shrink the transitions tied to the cut instead of refusing (S2).
    pub shrink_transitions: bool,
    /// Refused in this phase (`SHOTS_OPTION_NOT_YET`, D13).
    pub with_narration: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShotOp {
    /// Moves a border of `shot` to `frame` (an edit frame; the end is exclusive).
    TrimShot {
        shot: String,
        edge: Edge,
        frame: i64,
        options: ShotOptions,
    },
    /// Roll: the cut between two neighbours goes to `frame`; the edit keeps its length.
    MoveCut {
        from: String,
        to: String,
        frame: i64,
        options: ShotOptions,
    },
    /// `trim_shot` of the end, as a length in frames.
    SetShotDuration {
        shot: String,
        frames: i64,
        options: ShotOptions,
    },
    /// Grows or shrinks the last shot, and so the edit (#21).
    SetEditDuration { frames: i64, options: ShotOptions },
}

impl ShotOp {
    pub fn options(&self) -> &ShotOptions {
        match self {
            Self::TrimShot { options, .. }
            | Self::MoveCut { options, .. }
            | Self::SetShotDuration { options, .. }
            | Self::SetEditDuration { options, .. } => options,
        }
    }

    /// The JSONL name of the op.
    pub fn name(&self) -> &'static str {
        match self {
            Self::TrimShot { .. } => "trim_shot",
            Self::MoveCut { .. } => "move_cut",
            Self::SetShotDuration { .. } => "set_shot_duration",
            Self::SetEditDuration { .. } => "set_edit_duration",
        }
    }

    /// `{"trim_shot": {...}}`: an object with exactly one key, the op, whose body names the
    /// fields (plan §6.2). A malformed op is `INVALID_OP`, not a refusal of the domain.
    pub fn from_json(value: &Value) -> Result<Self, EditError> {
        let Some(wrapper) = value.as_object().filter(|map| map.len() == 1) else {
            return Err(invalid("an op is an object with exactly one key"));
        };
        let (name, body) = wrapper.iter().next().expect("one key");
        let (required, mode_allowed): (&[&str], bool) = match name.as_str() {
            "trim_shot" => (&["shot", "edge", "frame"], true),
            "move_cut" => (&["from", "to", "frame"], false),
            "set_shot_duration" => (&["shot", "frames"], true),
            "set_edit_duration" => (&["frames"], false),
            other => {
                return Err(invalid(format!(
                    "unknown op `{other}` (expected trim_shot, move_cut, set_shot_duration or \
                     set_edit_duration)"
                )));
            }
        };
        let Some(body) = body.as_object() else {
            return Err(invalid(format!("`{name}` takes an object")));
        };
        for key in body.keys() {
            let known = required.contains(&key.as_str())
                || OPTION_KEYS.contains(&key.as_str()) && (mode_allowed || key != "mode");
            if !known {
                return Err(invalid(format!("`{name}` has no field `{key}`")));
            }
        }
        if let Some(missing) = required.iter().find(|key| !body.contains_key(**key)) {
            return Err(invalid(format!("`{name}` needs `{missing}`")));
        }
        let options = options_of(body)?;
        Ok(match name.as_str() {
            "trim_shot" => Self::TrimShot {
                shot: text(body, "shot")?,
                edge: match text(body, "edge")?.as_str() {
                    "start" => Edge::Start,
                    "end" => Edge::End,
                    _ => return Err(invalid("`edge` is `start` or `end`")),
                },
                frame: frame(body, "frame")?,
                options,
            },
            "move_cut" => Self::MoveCut {
                from: text(body, "from")?,
                to: text(body, "to")?,
                frame: frame(body, "frame")?,
                options,
            },
            "set_shot_duration" => Self::SetShotDuration {
                shot: text(body, "shot")?,
                frames: frame(body, "frames")?,
                options,
            },
            _ => Self::SetEditDuration {
                frames: frame(body, "frames")?,
                options,
            },
        })
    }
}

const OPTION_KEYS: [&str; 5] = [
    "mode",
    "content",
    "keep_absolute",
    "shrink_transitions",
    "with_narration",
];

fn invalid(message: impl Into<String>) -> EditError {
    EditError::new(INVALID_OP, message)
}

fn text(body: &Map<String, Value>, key: &str) -> Result<String, EditError> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| invalid(format!("`{key}` must be a string")))
}

fn frame(body: &Map<String, Value>, key: &str) -> Result<i64, EditError> {
    let value = body
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid(format!("`{key}` must be an integer")))?;
    if !in_bound(value) {
        return Err(invalid(format!("`{key}` is beyond +-{FRAME_BOUND}")));
    }
    Ok(value)
}

fn flag(body: &Map<String, Value>, key: &str) -> Result<bool, EditError> {
    match body.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(invalid(format!("`{key}` is a boolean"))),
    }
}

fn options_of(body: &Map<String, Value>) -> Result<ShotOptions, EditError> {
    let mode = match body.get("mode").map(Value::as_str) {
        None => Mode::Ripple,
        Some(Some("ripple")) => Mode::Ripple,
        Some(Some("roll")) => Mode::Roll,
        Some(_) => return Err(invalid("`mode` is `ripple` or `roll`")),
    };
    let content = match body.get("content").map(Value::as_str) {
        None => Content::Auto,
        Some(Some("auto")) => Content::Auto,
        Some(Some("anchor")) => Content::Anchor,
        Some(Some("follow")) => Content::Follow,
        Some(_) => return Err(invalid("`content` is `anchor`, `follow` or `auto`")),
    };
    Ok(ShotOptions {
        mode,
        content,
        keep_absolute: flag(body, "keep_absolute")?,
        shrink_transitions: flag(body, "shrink_transitions")?,
        with_narration: flag(body, "with_narration")?,
    })
}

/// A batch as the JSONL carries it: an array of ops (see [`ShotOp::from_json`]). A malformed op
/// carries its position.
pub fn parse_shot_ops(value: &Value) -> Result<Vec<ShotOp>, EditError> {
    let Some(items) = value.as_array() else {
        return Err(invalid("a batch must be an array of shot ops"));
    };
    check_batch_len(items.len())?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| ShotOp::from_json(item).map_err(|error| error.at_op(index)))
        .collect()
}

/// Refuses a batch past [`MAX_SHOT_BATCH_OPS`] before anything is copied or run.
pub fn check_batch_len(count: usize) -> Result<(), EditError> {
    if count > MAX_SHOT_BATCH_OPS {
        return Err(invalid(format!(
            "a batch holds at most {MAX_SHOT_BATCH_OPS} shot ops, got {count}"
        ))
        .with_path("ops"));
    }
    Ok(())
}
