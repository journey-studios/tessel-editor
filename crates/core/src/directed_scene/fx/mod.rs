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
//! * the `vignette` look, scope `output`, on tracks of non-overlapping clips;
//! * fixed parameters (`params`; a missing one is the catalog default), `fadeInFrames` /
//!   `fadeOutFrames` envelope, `hidden` on clips and tracks, `seed` kept as written;
//! * **refused for now**: `keys` (`FX_KEYS_NOT_YET`), scope `shot`/`node` (`FX_SCOPE_NOT_YET`)
//!   and any `transitions` entry (`FX_TRANSITION_NOT_YET`).
//!
//! Evaluation is a pure function of the document and an integer edit frame
//! ([`Fx::resolve`]); the plan (`scene_plan`) carries the result to a runtime, which only draws.

mod catalog;
mod read;
mod reference;

use serde_json::{Value, json};

pub use catalog::{
    EffectSpec, ParamKind, ParamSpec, VIGNETTE, VIGNETTE_CENTER, VIGNETTE_COLOR,
    VIGNETTE_INTENSITY, VIGNETTE_RADIUS, VIGNETTE_ROUNDNESS, VIGNETTE_SOFTNESS, catalog,
    catalog_json, effect,
};
pub(super) use read::read_fx;
pub use read::{ParamProblem, parse_param};
pub use reference::{
    apply_pass_to_rgba8, apply_pass_to_srgb, linear_to_srgb, smoothstep, srgb_to_linear,
    vignette_shade,
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
pub const FX_KEYS_NOT_YET: &str = "FX_KEYS_NOT_YET";
pub const FX_TRANSITION_NOT_YET: &str = "FX_TRANSITION_NOT_YET";
pub const FX_LIMIT: &str = "FX_LIMIT";

/// A resolved parameter value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FxValue {
    Number(f64),
    Color { r: f64, g: f64, b: f64 },
    Point { x: f64, y: f64 },
}

impl FxValue {
    pub fn to_json(self) -> Value {
        match self {
            Self::Number(value) => json!(value),
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
    pub params: Vec<FxValue>,
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
    /// Every parameter of the effect, in catalog order.
    pub params: Vec<(String, FxValue)>,
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

    pub fn to_json(&self) -> Value {
        let mut params = serde_json::Map::new();
        for (name, value) in &self.params {
            params.insert(name.clone(), value.to_json());
        }
        json!({
            "trackId": self.track_id,
            "clipId": self.clip_id,
            "effect": self.effect,
            "frameInClip": self.frame_in_clip,
            "weight": self.weight,
            "amount": self.amount,
            "params": Value::Object(params),
            "scope": { "kind": "output" },
        })
    }

    /// What `amount` must be for these parameters and weight; the plan's validation compares it.
    pub fn expected_amount(&self) -> f64 {
        (self.number("intensity").unwrap_or(1.0) * self.weight).clamp(0.0, 1.0)
    }
}

impl Fx {
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
                        .zip(&clip.params)
                        .map(|(spec, value)| (spec.id.to_string(), *value))
                        .collect(),
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
    json!({
        "id": clip.id, "effect": clip.effect.id,
        "start_frame": clip.start_frame, "duration_frames": clip.duration_frames,
        "end_frame_exclusive": clip.end_frame_exclusive(),
        "fade_in_frames": clip.fade_in_frames, "fade_out_frames": clip.fade_out_frames,
        "hidden": clip.hidden, "seed": clip.seed, "scope": "output",
        "params": Value::Object(params),
    })
}

/// A syntactically fine ID for a track or clip: letters, digits, `-`, `_`, `.`, at most
/// [`MAX_FX_ID_BYTES`], not reserved. `None` means fine; `Some` is the reason (and code).
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
        .find(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')))
    {
        return Some((
            super::INVALID_VALUE,
            format!("`{bad}` is not allowed in an ID (use letters, digits, `-`, `_` or `.`)"),
        ));
    }
    if FX_RESERVED_IDS.contains(&id) {
        return Some((
            RESERVED_ID,
            format!(
                "`{id}` is reserved (property paths use it as a segment); reserved: {}",
                FX_RESERVED_IDS.join(", ")
            ),
        ));
    }
    None
}
