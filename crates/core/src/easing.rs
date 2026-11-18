//! The easings a keyed value or a timed reveal can name (F069 keys, F072 reveal).
//!
//! The first three are the directed-scene easings (`camera_keys::Easing`, the origin's names):
//! they are evaluated by that same code, so a group key and a camera key with the same name
//! shape their progress identically. `smoothStep` (the cubic Hermite `3t² − 2t³`) is the one
//! this module adds. It is **not** a directed-scene easing: camera, effect and transition keys
//! keep the origin's three, and only groups and the reveal accept the fourth.

use crate::camera_keys::Easing as DirectedEasing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyEasing {
    #[default]
    Linear,
    EaseInOutCubic,
    EaseOutCubic,
    SmoothStep,
}

impl KeyEasing {
    pub const ALL: [Self; 4] = [
        Self::Linear,
        Self::EaseInOutCubic,
        Self::EaseOutCubic,
        Self::SmoothStep,
    ];

    /// [`Self::ALL`] as the names a select property lists (the inspector wants a `'static` slice).
    pub const OPTIONS: &'static [&'static str] =
        &["linear", "easeInOutCubic", "easeOutCubic", "smoothStep"];

    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::EaseInOutCubic => "easeInOutCubic",
            Self::EaseOutCubic => "easeOutCubic",
            Self::SmoothStep => "smoothStep",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|easing| easing.label() == raw.trim())
    }

    pub fn labels() -> Vec<&'static str> {
        Self::ALL.iter().map(|easing| easing.label()).collect()
    }

    /// The eased progress for `t`, clamped to `0..=1` first.
    pub fn apply(self, t: f64) -> f64 {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        match self {
            Self::Linear => DirectedEasing::Linear.apply(t),
            Self::EaseInOutCubic => DirectedEasing::EaseInOutCubic.apply(t),
            Self::EaseOutCubic => DirectedEasing::EaseOutCubic.apply(t),
            Self::SmoothStep => t * t * (3.0 - 2.0 * t),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directed_three_keep_their_values_and_smooth_step_is_the_hermite_curve() {
        for (easing, directed) in [
            (KeyEasing::Linear, DirectedEasing::Linear),
            (KeyEasing::EaseInOutCubic, DirectedEasing::EaseInOutCubic),
            (KeyEasing::EaseOutCubic, DirectedEasing::EaseOutCubic),
        ] {
            assert_eq!(easing.label(), directed.label());
            for step in 0..=10 {
                let t = f64::from(step) / 10.0;
                assert_eq!(easing.apply(t), directed.apply(t), "{easing:?} at {t}");
            }
        }
        assert_eq!(KeyEasing::SmoothStep.apply(0.0), 0.0);
        assert_eq!(KeyEasing::SmoothStep.apply(0.5), 0.5);
        assert_eq!(KeyEasing::SmoothStep.apply(1.0), 1.0);
        // 3(1/9)² − 2(1/9)³ = 0.0342…, the curve of the reference's first frame step.
        assert!((KeyEasing::SmoothStep.apply(1.0 / 9.0) - 25.0 / 729.0).abs() < 1e-12);
        assert_eq!(KeyEasing::SmoothStep.apply(-3.0), 0.0, "clamped first");
        assert_eq!(KeyEasing::SmoothStep.apply(7.0), 1.0, "clamped first");
        assert_eq!(KeyEasing::SmoothStep.apply(f64::NAN), 0.0);
    }

    #[test]
    fn labels_parse_back_and_unknown_names_are_refused() {
        for easing in KeyEasing::ALL {
            assert_eq!(KeyEasing::parse(easing.label()), Some(easing));
        }
        assert_eq!(KeyEasing::parse("smoothstep"), None, "names are exact");
        assert_eq!(KeyEasing::parse("bounce"), None);
        assert_eq!(KeyEasing::labels().len(), KeyEasing::ALL.len());
        assert_eq!(KeyEasing::OPTIONS, KeyEasing::labels().as_slice());
    }
}
