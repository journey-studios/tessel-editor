//! Parameter keys of an effect clip and how they are sampled (F016, P3, BL082.2).
//!
//! The pilot's rules (`fx/fx-schema.ts`, `fx-keys.ts`, `keyframes/keyframe-rules.ts`,
//! `keyframes/resolve-keyframes.ts`, `fx-clip-sample.ts`, snapshot `d40ac3f7c0`), kept as pure
//! functions of the typed keys and an integer frame of the clip:
//!
//! * a key sits on a whole frame **local to the clip** (`0..durationFrames`), strictly after the
//!   key before it, and carries a typed value and the easing of the span that *arrives* at it;
//! * before the first key the first value holds, after the last the last one;
//! * on a key's own frame the answer is the stored value itself (never `from + (to - from) * 1`,
//!   which would answer 0.20000000000000018 for a key set to 0.2);
//! * between keys numbers, the parts of a point and the channels of a colour (already in linear
//!   light) interpolate after the easing; a boolean keeps the previous value until the frame of
//!   the next key, when it takes exactly the new one.
//!
//! Nothing here reads a clock, a frame asked before, or a cache: a seek to any frame answers
//! what playback answered there, and the lookup is a binary search over the prepared curve.

use super::FxValue;

/// The easings a key may name: the camera's three, in the pilot's order (`CAMERA_EASINGS`).
pub const FX_EASINGS: [&str; 3] = ["linear", "easeInOutCubic", "easeOutCubic"];

/// The shape of the span that arrives at a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FxEasing {
    #[default]
    Linear,
    EaseInOutCubic,
    EaseOutCubic,
}

impl FxEasing {
    pub const ALL: [Self; 3] = [Self::Linear, Self::EaseInOutCubic, Self::EaseOutCubic];

    pub fn name(self) -> &'static str {
        match self {
            Self::Linear => FX_EASINGS[0],
            Self::EaseInOutCubic => FX_EASINGS[1],
            Self::EaseOutCubic => FX_EASINGS[2],
        }
    }

    /// Exactly the three names; nothing is trimmed or case-folded.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|easing| easing.name() == name)
    }

    /// The eased progress for `t` (clamped to `0..=1` first), by the evaluator the camera uses:
    /// the contracts are the same, so there is one implementation.
    pub fn apply(self, t: f64) -> f64 {
        crate::directed_scene::apply_easing(t, self.name()).unwrap_or(t)
    }
}

/// One keyed value: at `frame` of the clip the parameter has `value`; `easing` shapes the span
/// that ARRIVES at this key (the first key's is unused).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FxKey {
    pub frame: i64,
    pub value: FxValue,
    pub easing: FxEasing,
}

/// The keys of one parameter of a clip (`param` is the index in the effect's catalog entry).
/// Never empty; frames strictly increasing; all inside the clip (the importer guarantees it).
#[derive(Debug, Clone, PartialEq)]
pub struct ParamKeys {
    pub param: usize,
    pub keys: Vec<FxKey>,
}

/// Most keys one curve (one parameter of one clip) holds.
pub const MAX_FX_KEYS_PER_CURVE: usize = 256;
/// Most keys all the curves of one clip hold together.
pub const MAX_FX_KEYS_PER_CLIP: usize = 1024;
/// Most keys the whole effects layer of a bundle holds.
pub const MAX_FX_KEYS_PER_LAYER: usize = 16_384;

fn mix(from: f64, to: f64, t: f64) -> f64 {
    from + (to - from) * t
}

/// Blends two values by their shape; a boolean (or any mismatch) keeps `from` until the next key.
fn mix_value(from: FxValue, to: FxValue, t: f64) -> FxValue {
    match (from, to) {
        (FxValue::Number(a), FxValue::Number(b)) => FxValue::Number(mix(a, b, t)),
        (
            FxValue::Color {
                r: r0,
                g: g0,
                b: b0,
            },
            FxValue::Color {
                r: r1,
                g: g1,
                b: b1,
            },
        ) => FxValue::Color {
            r: mix(r0, r1, t),
            g: mix(g0, g1, t),
            b: mix(b0, b1, t),
        },
        (FxValue::Point { x: x0, y: y0 }, FxValue::Point { x: x1, y: y1 }) => FxValue::Point {
            x: mix(x0, x1, t),
            y: mix(y0, y1, t),
        },
        _ => from,
    }
}

/// The value of a curve at `frame`. `keys` is sorted by strictly increasing frame and not empty
/// (what the importer and the edit operations produce); an empty slice yields `None`.
pub fn resolve_keys(keys: &[FxKey], frame: i64) -> Option<FxValue> {
    let first = keys.first()?;
    if frame <= first.frame {
        return Some(first.value);
    }
    // The first key at or after the frame: a binary search, so a long curve costs a few steps.
    let arriving = keys.partition_point(|key| key.frame < frame);
    let Some(to) = keys.get(arriving) else {
        return keys.last().map(|last| last.value);
    };
    // On a key's own frame the track holds the stored value exactly.
    if frame == to.frame {
        return Some(to.value);
    }
    let from = &keys[arriving - 1];
    let span = to.frame as f64 - from.frame as f64;
    let t = to.easing.apply((frame as f64 - from.frame as f64) / span);
    Some(mix_value(from.value, to.value, t))
}

/// Why a list of key frames breaks the pilot's order rule, if it does: the index of the first
/// key that is not on a whole frame from 0 or not strictly after the one before.
pub fn first_out_of_order(frames: &[i64]) -> Option<usize> {
    let mut previous: i64 = -1;
    for (index, &frame) in frames.iter().enumerate() {
        if frame < 0 || frame <= previous {
            return Some(index);
        }
        previous = frame;
    }
    None
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;

    fn num(frame: i64, value: f64, easing: FxEasing) -> FxKey {
        FxKey {
            frame,
            value: FxValue::Number(value),
            easing,
        }
    }

    fn at(keys: &[FxKey], frame: i64) -> f64 {
        resolve_keys(keys, frame).unwrap().as_number().unwrap()
    }

    #[test]
    fn before_the_first_key_the_first_value_holds_and_after_the_last_the_last() {
        let keys = [num(4, 0.2, FxEasing::Linear), num(8, 0.8, FxEasing::Linear)];
        assert_eq!(at(&keys, -50), 0.2);
        assert_eq!(at(&keys, 0), 0.2);
        assert_eq!(at(&keys, 3), 0.2);
        assert_eq!(at(&keys, 9), 0.8);
        assert_eq!(at(&keys, 10_000), 0.8);
        assert_eq!(at(&keys, i64::MAX), 0.8);
        assert_eq!(at(&keys, i64::MIN), 0.2);
    }

    #[test]
    fn a_key_frame_returns_the_stored_value_bit_for_bit() {
        // 0.3 + (0.9 - 0.3) * 1 is 0.9000000000000001: reconstructing the end of a span is not the key.
        let keys = [
            num(0, 0.3, FxEasing::Linear),
            num(10, 0.9, FxEasing::EaseOutCubic),
            num(20, 0.2, FxEasing::EaseInOutCubic),
        ];
        assert_eq!(at(&keys, 0).to_bits(), 0.3_f64.to_bits());
        assert_eq!(at(&keys, 10).to_bits(), 0.9_f64.to_bits());
        assert_eq!(at(&keys, 20).to_bits(), 0.2_f64.to_bits());
        assert_ne!(0.3 + (0.9 - 0.3) * 1.0, 0.9, "the case is a real one");
    }

    #[test]
    fn the_easing_of_the_arrival_key_shapes_the_span_and_the_first_key_easing_is_unused() {
        let linear_arrival = [
            num(0, 0.0, FxEasing::EaseOutCubic),
            num(10, 1.0, FxEasing::Linear),
        ];
        assert_eq!(at(&linear_arrival, 5), 0.5, "linear arrives at half");
        let eased_arrival = [
            num(0, 0.0, FxEasing::Linear),
            num(10, 1.0, FxEasing::EaseOutCubic),
        ];
        assert_eq!(at(&eased_arrival, 5), 1.0 - 0.5_f64.powi(3));
        let in_out = [
            num(0, 0.0, FxEasing::Linear),
            num(10, 1.0, FxEasing::EaseInOutCubic),
        ];
        assert_eq!(at(&in_out, 2), 4.0 * 0.2_f64.powi(3));
        assert_eq!(at(&in_out, 5), 0.5);
        let p = -2.0 * 0.8 + 2.0;
        assert_eq!(at(&in_out, 8), 1.0 - p * p * p / 2.0);
    }

    #[test]
    fn a_one_frame_interval_has_no_interior_frame() {
        let keys = [num(3, 0.0, FxEasing::Linear), num(4, 1.0, FxEasing::Linear)];
        assert_eq!(at(&keys, 3), 0.0);
        assert_eq!(at(&keys, 4), 1.0);
    }

    #[test]
    fn colors_and_points_interpolate_by_component() {
        let colors = [
            FxKey {
                frame: 0,
                value: FxValue::Color {
                    r: 0.0,
                    g: 0.5,
                    b: 1.0,
                },
                easing: FxEasing::Linear,
            },
            FxKey {
                frame: 4,
                value: FxValue::Color {
                    r: 1.0,
                    g: 0.5,
                    b: 0.0,
                },
                easing: FxEasing::Linear,
            },
        ];
        assert_eq!(
            resolve_keys(&colors, 1),
            Some(FxValue::Color {
                r: 0.25,
                g: 0.5,
                b: 0.75
            })
        );
        let points = [
            FxKey {
                frame: 0,
                value: FxValue::Point { x: 0.0, y: 1.0 },
                easing: FxEasing::Linear,
            },
            FxKey {
                frame: 2,
                value: FxValue::Point { x: 1.0, y: 0.0 },
                easing: FxEasing::Linear,
            },
        ];
        assert_eq!(
            resolve_keys(&points, 1),
            Some(FxValue::Point { x: 0.5, y: 0.5 })
        );
    }

    #[test]
    fn a_boolean_keeps_the_previous_value_until_the_frame_of_the_next_key() {
        let keys = [
            FxKey {
                frame: 2,
                value: FxValue::Bool(false),
                easing: FxEasing::Linear,
            },
            FxKey {
                frame: 6,
                value: FxValue::Bool(true),
                easing: FxEasing::EaseInOutCubic,
            },
        ];
        assert_eq!(resolve_keys(&keys, 0), Some(FxValue::Bool(false)));
        assert_eq!(resolve_keys(&keys, 5), Some(FxValue::Bool(false)));
        assert_eq!(resolve_keys(&keys, 6), Some(FxValue::Bool(true)));
        assert_eq!(resolve_keys(&keys, 100), Some(FxValue::Bool(true)));
    }

    #[test]
    fn an_empty_curve_has_no_value() {
        assert_eq!(resolve_keys(&[], 3), None);
    }

    #[test]
    fn a_long_curve_is_searched_not_walked() {
        let keys: Vec<FxKey> = (0..MAX_FX_KEYS_PER_CURVE as i64)
            .map(|index| num(index * 4, index as f64, FxEasing::Linear))
            .collect();
        assert_eq!(at(&keys, 4 * 100 + 2), 100.5);
        assert_eq!(at(&keys, 4 * 255), 255.0);
    }

    #[test]
    fn the_order_rule_is_whole_frames_from_zero_each_after_the_one_before() {
        assert_eq!(first_out_of_order(&[0, 1, 5]), None);
        assert_eq!(first_out_of_order(&[]), None);
        assert_eq!(first_out_of_order(&[-1]), Some(0));
        assert_eq!(first_out_of_order(&[3, 3]), Some(1));
        assert_eq!(first_out_of_order(&[3, 2]), Some(1));
        assert_eq!(first_out_of_order(&[0, 4, 4, 9]), Some(2));
    }

    #[test]
    fn easing_names_are_exact() {
        for easing in FxEasing::ALL {
            assert_eq!(FxEasing::parse(easing.name()), Some(easing));
        }
        assert_eq!(FxEasing::parse("Linear"), None);
        assert_eq!(FxEasing::parse(" linear"), None);
        assert_eq!(FxEasing::parse("hold"), None);
        assert_eq!(FxEasing::parse("step"), None);
        assert_eq!(FX_EASINGS.len(), FxEasing::ALL.len());
    }
}
