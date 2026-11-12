//! The effects layer of a directed scene (F016): `bundle.fx`, clips of an effect laid on tracks
//! over the edit's timeline, validated with the bundle and resolved to the passes of one frame.
//!
//! **Where the layer lives (decision, `docs/features/effects.md`).** `fx` is a top-level key of
//! the bundle document, the pilot's own shape (`Fx { catalogVersion, tracks, transitions }`).
//! The lossless JSON tree stays the canonical data: the importer reads it into the typed view
//! below, `scene_edit::apply_fx` edits the tree atomically and re-imports the candidate, and the
//! project saves it like the rest of the scene. Nothing here belongs to the project's sequence
//! timeline (F023): there is exactly one model of effects, so nothing can drift between two.
//!
//! What is implemented, and nothing else is accepted (an unsupported value is a typed refusal
//! at import and at edit, never a quietly ignored one):
//!
//! * the `vignette`, `fade` and `filmGrain` looks, scope `output`, on tracks of non-overlapping
//!   clips;
//! * IDs of tracks and clips by the pilot's rule (see [`id_problem`]);
//! * fixed parameters (`params`; a missing one is the catalog default), `fadeInFrames` /
//!   `fadeOutFrames` envelope, `hidden` on clips and tracks;
//! * **parameter keys** (`keys`, P3: whole frames local to the clip, strictly increasing, the
//!   easing of the arrival key; see [`keys`]) and the `seed` of the noise effects, explicit or
//!   derived from the clip ID (see [`hash`]);
//! * **refused for now**: scope `shot`/`node` (`FX_SCOPE_NOT_YET`) and any `transitions` entry
//!   (`FX_TRANSITION_NOT_YET`).
//!
//! Evaluation is a pure function of the document and an integer edit frame
//! ([`Fx::resolve`]); the plan (`scene_plan`) carries the result to a runtime, which only draws.

mod catalog;
mod control;
mod handles;
mod hash;
mod keys;
mod read;
mod reference;

use serde_json::{Value, json};

pub use catalog::{
    EffectSpec, FADE, FADE_AMOUNT, FADE_COLOR, FADE_INTENSITY, FILM_GRAIN, FILM_GRAIN_AMOUNT,
    FILM_GRAIN_COLORED, FILM_GRAIN_INTENSITY, FILM_GRAIN_LUMA, FILM_GRAIN_SIZE, Labels, Locale,
    ParamKind, ParamSpec, VIGNETTE, VIGNETTE_CENTER, VIGNETTE_COLOR, VIGNETTE_INTENSITY,
    VIGNETTE_RADIUS, VIGNETTE_ROUNDNESS, VIGNETTE_SOFTNESS, catalog, catalog_json, effect,
};
pub use control::quantize_control;
pub use handles::{
    CENTER_STEP, HandleValue, VignetteHandle, VignetteParams, VignetteShape, hit as hit_handle,
    on_picture, params_at as vignette_params_at,
};
pub use hash::{
    Reseed, base_seed, colored_triangular, fnv1a32, fnv1a64, hash_pixel, lowbias32, seed_for,
    triangular, unit_float,
};
pub use keys::{
    FX_EASINGS, FxEasing, FxKey, MAX_FX_KEYS_PER_CLIP, MAX_FX_KEYS_PER_CURVE,
    MAX_FX_KEYS_PER_LAYER, ParamKeys, first_out_of_order, resolve_keys,
};
pub(super) use read::read_fx;
pub use read::{ParamProblem, parse_param};
pub use reference::{
    GRAIN_REFERENCE_HEIGHT, apply_pass_to_rgba8, apply_pass_to_srgb, grain_cell_px,
    grain_encoded_delta, linear_to_srgb, pass_strength, smoothstep, srgb_to_linear, vignette_shade,
};

/// The catalog this build draws. It goes up whenever the *result* of an existing effect changes
/// (not when one is added); a bundle written against a later catalog is refused.
pub const FX_CATALOG_VERSION: u32 = 1;

/// Tracks a bundle may hold.
pub const MAX_FX_TRACKS: usize = 64;
/// Clips on one track. The whole layer is bounded by both limits and `Limits::max_items`.
pub const MAX_FX_CLIPS_PER_TRACK: usize = 1024;
/// Longest id of a track or clip, in bytes.
pub const MAX_FX_ID_BYTES: usize = 64;
/// Ids a track, clip or transition may not take: the property paths use them as segments.
pub const FX_RESERVED_IDS: [&str; 3] = ["track", "fx", "transition"];

// Stable issue codes. Where the pilot (`fx-schema.ts`, `FX_ERROR_CODES`) names a rule, its code
// is reused verbatim.
pub const UNKNOWN_EFFECT: &str = "UNKNOWN_EFFECT";
pub const UNKNOWN_PARAM: &str = "UNKNOWN_PARAM";
pub const PARAM_TYPE: &str = "PARAM_TYPE";
pub const PARAM_OUT_OF_RANGE: &str = "PARAM_OUT_OF_RANGE";
pub const UNKNOWN_TRACK: &str = "UNKNOWN_FX_TRACK";
pub const UNKNOWN_CLIP: &str = "UNKNOWN_FX_CLIP";
pub const RESERVED_ID: &str = "RESERVED_ID";
pub const DUPLICATE_ID: &str = "DUPLICATE_ID";
pub const FX_CLIP_OVERLAP: &str = "FX_CLIP_OVERLAP";
pub const FX_CLIP_EMPTY: &str = "FX_CLIP_EMPTY";
pub const FX_CLIP_OUT_OF_RANGE: &str = "FX_CLIP_OUT_OF_RANGE";
pub const ENVELOPE_TOO_LONG: &str = "ENVELOPE_TOO_LONG";
pub const CATALOG_TOO_NEW: &str = "CATALOG_TOO_NEW";
/// Scope `node` in the pilot (`NODE_SCOPE_NOT_YET`); here also `shot`, which is a later slice.
pub const FX_SCOPE_NOT_YET: &str = "FX_SCOPE_NOT_YET";
/// A key outside its clip (`0..durationFrames`), the pilot's `KEY_OUT_OF_CLIP`.
pub const KEY_OUT_OF_CLIP: &str = "KEY_OUT_OF_CLIP";
/// Key frames that are not whole, not from 0 or not strictly increasing (`KEYFRAME_ORDER`).
pub const KEYFRAME_ORDER: &str = "KEYFRAME_ORDER";
/// An easing the engine cannot draw (`UNKNOWN_KEYFRAME_EASING`).
pub const UNKNOWN_KEYFRAME_EASING: &str = "UNKNOWN_KEYFRAME_EASING";
/// An edit named a key frame (or a keyed parameter) the clip does not have (`UNKNOWN_KEYFRAME`).
pub const UNKNOWN_KEYFRAME: &str = "UNKNOWN_KEYFRAME";
pub const FX_TRANSITION_NOT_YET: &str = "FX_TRANSITION_NOT_YET";
/// A limit of the layer (tracks in a bundle, clips on a track). Reported as an importer issue
/// with this code; the command code of the rejected edit stays `invalid_bundle`.
pub const FX_LIMIT: &str = "FX_LIMIT";
/// The code of the structured warning for a frame that asks for more passes than the budget.
pub const FX_PASS_BUDGET_WARNING: &str = "FX_PASS_BUDGET";

/// How many output passes a frame should have at most. **Policy: warn, never refuse.** Every pass
/// is one fullscreen draw over the picture (the pilot's editor warns at the same number), so past
/// it a frame is slower, not wrong: the document stays valid, the plan keeps every pass in order and
/// the runtime draws them all (two ping-pong targets whatever the count), and `scene.fx.list`,
/// `scene.fx.edit` and `scene.fx.frame` report the overrun as a structured warning
/// ([`PassBudget`]). A refusal here would make a bundle the pilot accepts impossible to open.
pub const FX_PASS_BUDGET: usize = 8;

/// A resolved parameter value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FxValue {
    Number(f64),
    Bool(bool),
    Color { r: f64, g: f64, b: f64 },
    Point { x: f64, y: f64 },
}

impl FxValue {
    pub fn to_json(self) -> Value {
        match self {
            Self::Number(value) => json!(value),
            Self::Bool(value) => json!(value),
            Self::Color { r, g, b } => json!({ "r": r, "g": g, "b": b }),
            Self::Point { x, y } => json!({ "x": x, "y": y }),
        }
    }

    pub fn as_number(self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_bool(self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(value),
            _ => None,
        }
    }
}

/// One effect clip: `effect` is the catalog entry, `params` the value of each of its parameters
/// in the catalog's order (defaults filled in), `start_frame..start_frame + duration_frames` the
/// edit frames it runs (the end is exclusive).
#[derive(Debug, Clone, PartialEq)]
pub struct FxClip {
    pub id: String,
    pub effect: &'static EffectSpec,
    pub start_frame: i64,
    pub duration_frames: i64,
    /// The static (base) value of each parameter, in catalog order. With keys on a parameter
    /// the curve prevails (see [`FxClip::value_at`]); the base shows through only where the
    /// parameter has no keys.
    pub params: Vec<FxValue>,
    /// The curves of the keyed parameters, ordered by catalog index; a parameter without keys
    /// has no entry.
    pub keys: Vec<ParamKeys>,
    pub fade_in_frames: i64,
    pub fade_out_frames: i64,
    pub seed: Option<u32>,
    pub hidden: bool,
}

impl FxClip {
    pub fn end_frame_exclusive(&self) -> i64 {
        self.start_frame.saturating_add(self.duration_frames)
    }

    /// The value of a parameter by id.
    pub fn param(&self, id: &str) -> Option<FxValue> {
        let (index, _) = self.effect.param(id)?;
        self.params.get(index).copied()
    }

    /// The curve of a parameter by catalog index, if it has keys.
    pub fn curve(&self, param: usize) -> Option<&[FxKey]> {
        self.keys
            .iter()
            .find(|curve| curve.param == param)
            .map(|curve| curve.keys.as_slice())
    }

    /// The curve of a parameter by id.
    pub fn curve_of(&self, id: &str) -> Option<&[FxKey]> {
        self.curve(self.effect.param(id)?.0)
    }

    /// The value of the parameter at `index` at a frame **of the clip**: its keys when it has
    /// any (the first value holds before the first key, the last after the last, a key's own
    /// frame answers its stored value), else the static value (the catalog default when the
    /// document leaves it out).
    pub fn value_at(&self, index: usize, frame_in_clip: i64) -> Option<FxValue> {
        match self.curve(index) {
            Some(keys) => resolve_keys(keys, frame_in_clip),
            None => self.params.get(index).copied(),
        }
    }

    /// Every parameter at a frame of the clip, in catalog order.
    pub fn sampled(&self, frame_in_clip: i64) -> Vec<FxValue> {
        (0..self.params.len())
            .filter_map(|index| self.value_at(index, frame_in_clip))
            .collect()
    }

    /// The seed of this clip's noise at a frame of the clip, or `None` for an effect that draws
    /// none. Explicit when the document gives one, else derived from the ID; the document is
    /// never written to by asking.
    pub fn seed_at(&self, frame_in_clip: i64) -> Option<u32> {
        seed_for(
            self.effect.reseed,
            base_seed(self.seed, &self.id),
            frame_in_clip,
        )
    }

    /// The fade-in/fade-out envelope at a frame of the clip, sampled at the middle of the frame,
    /// 0..1 (the pilot's `envelopeWeight`).
    pub fn weight_at(&self, frame_in_clip: i64) -> f64 {
        let mut weight: f64 = 1.0;
        if self.fade_in_frames > 0 {
            weight = weight.min((frame_in_clip as f64 + 0.5) / self.fade_in_frames as f64);
        }
        if self.fade_out_frames > 0 {
            weight = weight.min(
                (self.duration_frames as f64 - frame_in_clip as f64 - 0.5)
                    / self.fade_out_frames as f64,
            );
        }
        weight.clamp(0.0, 1.0)
    }
}

/// A track: one effect at a time, in the order of composition (the first track is drawn first).
#[derive(Debug, Clone, PartialEq)]
pub struct FxTrack {
    pub id: String,
    pub label: String,
    pub hidden: bool,
    pub clips: Vec<FxClip>,
}

/// The effects layer of a bundle. `Fx::default()` is a bundle without effects: no pass at any
/// frame, which is exactly the baseline picture.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fx {
    pub tracks: Vec<FxTrack>,
}

/// One pass of one frame, resolved: what a runtime draws. `amount` is `intensity × weight`
/// clamped to 0..1, computed here (the runtime does not know the envelope); an effect is exactly
/// the identity at `amount <= 0`.
#[derive(Debug, Clone, PartialEq)]
pub struct FxPass {
    pub track_id: String,
    pub clip_id: String,
    pub effect: String,
    pub frame_in_clip: i64,
    /// The envelope at this frame, 0..1.
    pub weight: f64,
    pub amount: f64,
    /// Every parameter of the effect at this frame (keys applied), in catalog order.
    pub params: Vec<(String, FxValue)>,
    /// The seed of the noise at this frame; only effects that draw noise carry one.
    pub seed: Option<u32>,
}

impl FxPass {
    pub fn param(&self, id: &str) -> Option<FxValue> {
        self.params
            .iter()
            .find(|(name, _)| name == id)
            .map(|(_, value)| *value)
    }

    pub fn number(&self, id: &str) -> Option<f64> {
        self.param(id).and_then(FxValue::as_number)
    }

    pub fn boolean(&self, id: &str) -> Option<bool> {
        self.param(id).and_then(FxValue::as_bool)
    }

    pub fn to_json(&self) -> Value {
        let mut params = serde_json::Map::new();
        for (name, value) in &self.params {
            params.insert(name.clone(), value.to_json());
        }
        let mut wire = json!({
            "trackId": self.track_id,
            "clipId": self.clip_id,
            "effect": self.effect,
            "frameInClip": self.frame_in_clip,
            "weight": self.weight,
            "amount": self.amount,
            "params": Value::Object(params),
            "scope": { "kind": "output" },
        });
        // Written only for an effect that draws noise, so the bytes of the passes of the looks
        // without it (and the digests of their plans) do not change.
        if let Some(seed) = self.seed {
            wire["seed"] = json!(seed);
        }
        wire
    }

    /// What `amount` must be for these parameters and weight; the plan's validation compares it.
    pub fn expected_amount(&self) -> f64 {
        (self.number("intensity").unwrap_or(1.0) * self.weight).clamp(0.0, 1.0)
    }
}

/// How the layer stands against [`FX_PASS_BUDGET`] over the whole edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PassBudget {
    pub budget: usize,
    /// The most passes any frame of the edit has (hidden clips and tracks draw nothing).
    pub peak: usize,
    /// The first edit frame with more than `budget` passes.
    pub first_frame_over: Option<i64>,
    /// How many frames of the edit have more than `budget` passes.
    pub frames_over: i64,
}

impl PassBudget {
    pub fn over(&self) -> bool {
        self.peak > self.budget
    }

    /// The budget as data, always present (so a caller can tell "inside" from "not reported").
    pub fn to_json(&self) -> Value {
        json!({
            "max_passes": self.budget, "policy": "warn", "peak_passes": self.peak,
            "over_budget": self.over(), "first_frame_over": self.first_frame_over,
            "frames_over": self.frames_over,
        })
    }

    /// The structured warnings of the budget: none inside it, one past it.
    pub fn warnings(&self) -> Vec<Value> {
        let Some(first) = self.first_frame_over.filter(|_| self.over()) else {
            return Vec::new();
        };
        vec![json!({
            "code": FX_PASS_BUDGET_WARNING,
            "message": format!(
                "{} frames draw more than {} effect passes (up to {}, from frame {first}); each pass is one fullscreen draw, so those frames are slower",
                self.frames_over, self.budget, self.peak
            ),
            "max_passes": self.budget, "peak_passes": self.peak,
            "first_frame_over": first, "frames_over": self.frames_over,
        })]
    }
}

impl Fx {
    /// The layer against [`FX_PASS_BUDGET`] over the `edit_duration_frames` of the edit: a sweep
    /// over the visible clips, so it costs the number of clips, not of frames.
    pub fn pass_budget(&self, edit_duration_frames: u64) -> PassBudget {
        let length = i64::try_from(edit_duration_frames).unwrap_or(i64::MAX);
        let mut edges: Vec<(i64, i32)> = Vec::new();
        for track in self.tracks.iter().filter(|track| !track.hidden) {
            for clip in track.clips.iter().filter(|clip| !clip.hidden) {
                let start = clip.start_frame.max(0);
                let end = clip.end_frame_exclusive().min(length);
                if start < end {
                    edges.push((start, 1));
                    edges.push((end, -1));
                }
            }
        }
        // At a frame both ends and starts happen: a clip ends *before* the one starting there begins.
        edges.sort_unstable();
        let mut running: i64 = 0;
        let mut peak: i64 = 0;
        let mut first_frame_over = None;
        let mut frames_over = 0;
        let budget = i64::try_from(FX_PASS_BUDGET).unwrap_or(i64::MAX);
        for (index, &(frame, delta)) in edges.iter().enumerate() {
            running += i64::from(delta);
            peak = peak.max(running);
            let next = edges.get(index + 1).map_or(frame, |&(next, _)| next);
            if running > budget && next > frame {
                first_frame_over.get_or_insert(frame);
                frames_over += next - frame;
            }
        }
        PassBudget {
            budget: FX_PASS_BUDGET,
            peak: usize::try_from(peak).unwrap_or(0),
            first_frame_over,
            frames_over,
        }
    }

    /// Whether no clip exists at all (the document may still hold empty tracks).
    pub fn is_empty(&self) -> bool {
        self.tracks.iter().all(|track| track.clips.is_empty())
    }

    pub fn clip(&self, id: &str) -> Option<(&FxTrack, &FxClip)> {
        self.tracks.iter().find_map(|track| {
            track
                .clips
                .iter()
                .find(|clip| clip.id == id)
                .map(|clip| (track, clip))
        })
    }

    pub fn track(&self, id: &str) -> Option<&FxTrack> {
        self.tracks.iter().find(|track| track.id == id)
    }

    pub fn clip_count(&self) -> usize {
        self.tracks.iter().map(|track| track.clips.len()).sum()
    }

    /// The passes at an edit frame, in the order of the tracks (the order of composition). A
    /// hidden track or clip draws nothing; a clip draws on `start_frame <= frame < end`. Pure and
    /// deterministic: no clock, no state carried from another frame, so a seek to any frame
    /// answers what linear playback answered there.
    pub fn resolve(&self, edit_frame: i64) -> Vec<FxPass> {
        let mut passes = Vec::new();
        for track in self.tracks.iter().filter(|track| !track.hidden) {
            for clip in track.clips.iter().filter(|clip| !clip.hidden) {
                if edit_frame < clip.start_frame || edit_frame >= clip.end_frame_exclusive() {
                    continue;
                }
                let frame_in_clip = edit_frame - clip.start_frame;
                let weight = clip.weight_at(frame_in_clip);
                let mut pass = FxPass {
                    track_id: track.id.clone(),
                    clip_id: clip.id.clone(),
                    effect: clip.effect.id.to_string(),
                    frame_in_clip,
                    weight,
                    amount: 0.0,
                    params: clip
                        .effect
                        .params
                        .iter()
                        .zip(clip.sampled(frame_in_clip))
                        .map(|(spec, value)| (spec.id.to_string(), value))
                        .collect(),
                    seed: clip.seed_at(frame_in_clip),
                };
                pass.amount = pass.expected_amount();
                passes.push(pass);
            }
        }
        passes
    }

    /// The layer as the importer reads it back from a document: used by queries.
    pub fn to_json(&self) -> Value {
        json!({
            "catalog_version": FX_CATALOG_VERSION,
            "tracks": self.tracks.iter().map(|track| json!({
                "id": track.id, "label": track.label, "hidden": track.hidden,
                "clips": track.clips.iter().map(clip_json).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    }
}

/// A clip as `scene.fx.list` reports it (every parameter resolved, defaults included).
pub fn clip_json(clip: &FxClip) -> Value {
    let mut params = serde_json::Map::new();
    for (spec, value) in clip.effect.params.iter().zip(&clip.params) {
        params.insert(spec.id.to_string(), value.to_json());
    }
    let mut keys = serde_json::Map::new();
    for curve in &clip.keys {
        let Some(spec) = clip.effect.params.get(curve.param) else {
            continue;
        };
        keys.insert(
            spec.id.to_string(),
            Value::Array(curve.keys.iter().map(key_json).collect()),
        );
    }
    json!({
        "id": clip.id, "effect": clip.effect.id,
        "start_frame": clip.start_frame, "duration_frames": clip.duration_frames,
        "end_frame_exclusive": clip.end_frame_exclusive(),
        "fade_in_frames": clip.fade_in_frames, "fade_out_frames": clip.fade_out_frames,
        "hidden": clip.hidden, "seed": clip.seed, "scope": "output",
        "params": Value::Object(params), "keys": Value::Object(keys),
    })
}

/// One key as the queries report it: its frame (local to the clip), typed value and easing.
pub fn key_json(key: &FxKey) -> Value {
    json!({ "frame": key.frame, "value": key.value.to_json(), "easing": key.easing.name() })
}

/// A syntactically fine ID for a track or clip, the pilot's rule (`fx-schema.ts`):
/// `^[a-z0-9][a-z0-9_-]{0,63}$` read case-insensitively (ASCII letters of either case, digits,
/// `-` and `_`; it starts with a letter or digit, so no `.`, no leading `-` or `_`), at most
/// [`MAX_FX_ID_BYTES`] bytes, and not reserved. The reserved words are compared in lowercase, as the
/// pilot does: `FX`, `Track` and `TRANSITION` are as reserved as `fx`, `track` and `transition`.
/// `None` means fine; `Some` is the reason (and code).
pub fn id_problem(id: &str) -> Option<(&'static str, String)> {
    if id.is_empty() {
        return Some((super::INVALID_VALUE, "an ID cannot be empty".to_string()));
    }
    if id.len() > MAX_FX_ID_BYTES {
        return Some((
            super::INVALID_VALUE,
            format!(
                "an ID has at most {MAX_FX_ID_BYTES} bytes, got {}",
                id.len()
            ),
        ));
    }
    if let Some(bad) = id
        .chars()
        .find(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')))
    {
        return Some((
            super::INVALID_VALUE,
            format!("`{bad}` is not allowed in an ID (use letters, digits, `-` or `_`)"),
        ));
    }
    if !id.starts_with(|ch: char| ch.is_ascii_alphanumeric()) {
        return Some((
            super::INVALID_VALUE,
            format!("an ID starts with a letter or a digit, not `{}`", &id[..1]),
        ));
    }
    let lowercase = id.to_ascii_lowercase();
    if FX_RESERVED_IDS.contains(&lowercase.as_str()) {
        return Some((
            RESERVED_ID,
            format!(
                "`{id}` is reserved in any case (property paths use it as a segment); reserved: {}",
                FX_RESERVED_IDS.join(", ")
            ),
        ));
    }
    None
}
