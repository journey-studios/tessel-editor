//! The animation half of the effects inspector (F057/F058, P3, BL004/BL029/BL040/BL082.2): what a
//! clip's parameters are at a frame, which of them follow keys, the keys themselves and the
//! operations that edit them, as one neutral value that the library returns, `scene.fx.inspect`
//! serialises and the GPUI panel paints.
//!
//! The base value of a parameter (`clip.params`, what [`super::InspectorField::value`] has always
//! been) never changes meaning here: the animation says, next to it, whether keys prevail
//! (`animated`), what the parameter is at the asked frame (`sampled`, the same pure resolver the
//! plan uses) and which keys exist. Nothing here keys by itself: a capture is an op the person (or
//! a script) sends, never a side effect of moving a control.
//!
//! Every operation is listed with the op it would send (`template`) and whether it applies right
//! now (`enabled`, with a typed `reason`), so an adapter never builds an op by hand and never has
//! a button without a reason.

use serde_json::{Value, json};

use crate::directed_scene::fx::{
    FX_EASINGS, FxClip, FxKey, FxValue, Locale, ParamSpec, base_seed, key_json,
};

/// Why an operation does not apply now.
pub const REASON_NO_FRAME: &str = "no_frame";
pub const REASON_FRAME_OUTSIDE_CLIP: &str = "frame_outside_clip";
pub const REASON_NO_KEYS: &str = "no_keys";
pub const REASON_NEEDS_TWO_KEYS: &str = "needs_two_keys";
pub const REASON_KEY_ON_FRAME: &str = "key_on_this_frame";

/// One parameter of a clip, as animation sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamAnimation {
    pub spec: &'static ParamSpec,
    /// The static value; with keys it only shows through where the parameter has none.
    pub base: FxValue,
    /// The curve prevails over the base.
    pub animated: bool,
    pub keys: Vec<FxKey>,
    /// The value at the asked frame of the clip (`None` without a frame, or outside the clip).
    pub sampled: Option<FxValue>,
}

/// The seed of a noise clip.
#[derive(Debug, Clone, PartialEq)]
pub struct SeedAnimation {
    /// What the document says (`None` when the seed comes from the clip's ID).
    pub explicit: Option<u32>,
    /// The base the per-frame seeds derive from: the explicit seed or the hash of the ID.
    pub base: u32,
    /// The seed of the asked frame, as the plan carries it.
    pub at_frame: Option<u32>,
    /// `static` or `per-frame`.
    pub reseed: &'static str,
}

/// The animation state of one clip at (optionally) one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipAnimation {
    pub clip: String,
    /// The frame of the edit that was asked, if any.
    pub edit_frame: Option<i64>,
    /// That frame as a frame of the clip, when the clip covers it.
    pub frame_in_clip: Option<i64>,
    pub duration_frames: i64,
    pub params: Vec<ParamAnimation>,
    pub seed: Option<SeedAnimation>,
}

impl ClipAnimation {
    /// The animation of `clip`, sampled at `edit_frame` of the scene's edit when given.
    pub fn of(clip: &FxClip, edit_frame: Option<i64>) -> Self {
        let frame_in_clip = edit_frame
            .map(|frame| frame - clip.start_frame)
            .filter(|local| (0..clip.duration_frames).contains(local));
        let params = clip
            .effect
            .params
            .iter()
            .enumerate()
            .filter_map(|(index, spec)| {
                let base = clip.params.get(index).copied()?;
                let keys = clip.curve(index).map(<[FxKey]>::to_vec).unwrap_or_default();
                Some(ParamAnimation {
                    spec,
                    base,
                    animated: !keys.is_empty(),
                    keys,
                    sampled: frame_in_clip.and_then(|local| clip.value_at(index, local)),
                })
            })
            .collect();
        let seed = clip.seed_at(0).map(|_| SeedAnimation {
            explicit: clip.seed,
            base: base_seed(clip.seed, &clip.id),
            at_frame: frame_in_clip.and_then(|local| clip.seed_at(local)),
            reseed: match clip.effect.reseed {
                crate::directed_scene::fx::Reseed::PerFrame => "per-frame",
                _ => "static",
            },
        });
        Self {
            clip: clip.id.clone(),
            edit_frame,
            frame_in_clip,
            duration_frames: clip.duration_frames,
            params,
            seed,
        }
    }

    pub fn param(&self, id: &str) -> Option<&ParamAnimation> {
        self.params.iter().find(|param| param.spec.id == id)
    }

    /// Whether any parameter follows keys.
    pub fn is_animated(&self) -> bool {
        self.params.iter().any(|param| param.animated)
    }

    pub fn to_json(&self, locale: Locale) -> Value {
        json!({
            "edit_frame": self.edit_frame, "frame_in_clip": self.frame_in_clip,
            "active": self.frame_in_clip.is_some(),
            "animated": self.is_animated(),
            "params": self.params.iter().map(|param| param.to_json(self, locale)).collect::<Vec<_>>(),
            "seed": self.seed.as_ref().map(|seed| json!({
                "explicit": seed.explicit, "base": seed.base, "at_frame": seed.at_frame,
                "reseed": seed.reseed, "from_id": seed.explicit.is_none(),
                "operations": [
                    op("set_seed", true, None, json!({ "op": "set_seed", "clip": self.clip, "seed": seed.base })),
                    op("auto_seed", seed.explicit.is_some(),
                       seed.explicit.is_none().then_some("seed_is_automatic"),
                       json!({ "op": "set_seed", "clip": self.clip, "seed": null })),
                ],
            })),
        })
    }
}

fn op(name: &str, enabled: bool, reason: Option<&str>, template: Value) -> Value {
    json!({ "name": name, "enabled": enabled, "reason": if enabled { None } else { reason }, "template": template })
}

impl ParamAnimation {
    fn to_json(&self, clip: &ClipAnimation, locale: Locale) -> Value {
        let id = self.spec.id;
        let local = clip.frame_in_clip;
        let on_key = local.is_some_and(|frame| self.keys.iter().any(|key| key.frame == frame));
        let capture_reason = match local {
            None if clip.edit_frame.is_none() => Some(REASON_NO_FRAME),
            None => Some(REASON_FRAME_OUTSIDE_CLIP),
            Some(_) => None,
        };
        let template_at = |frame: Option<i64>| json!({ "op": "key_param", "clip": clip.clip, "param": id, "frame": frame });
        let mut operations = vec![op(
            "capture",
            capture_reason.is_none(),
            capture_reason,
            template_at(local),
        )];
        operations.push(op(
            "shift_keys",
            self.animated,
            Some(REASON_NO_KEYS),
            json!({ "op": "shift_keys", "clip": clip.clip, "param": id, "delta": 0 }),
        ));
        operations.push(op(
            "scale_keys",
            self.keys.len() >= 2,
            Some(if self.animated {
                REASON_NEEDS_TWO_KEYS
            } else {
                REASON_NO_KEYS
            }),
            json!({ "op": "scale_keys", "clip": clip.clip, "param": id,
                    "anchor": self.keys.first().map(|key| key.frame), "num": 1, "den": 1 }),
        ));
        operations.push(op(
            "remove_animation",
            self.animated,
            Some(REASON_NO_KEYS),
            json!({ "op": "clear_keys", "clip": clip.clip, "param": id }),
        ));
        let keys: Vec<Value> = self
            .keys
            .iter()
            .map(|key| {
                let mut described = key_json(key);
                described["operations"] = json!([
                    op("remove", true, None,
                       json!({ "op": "remove_key", "clip": clip.clip, "param": id, "frame": key.frame })),
                    op("move", true, None,
                       json!({ "op": "move_key", "clip": clip.clip, "param": id, "from": key.frame, "to": key.frame })),
                    op("set_easing", true, None,
                       json!({ "op": "set_key_easing", "clip": clip.clip, "param": id, "frame": key.frame, "easing": key.easing.name() })),
                ]);
                described["easings"] = json!(FX_EASINGS);
                described
            })
            .collect();
        json!({
            "param": id,
            "label": self.spec.label.get(locale),
            "kind": match self.base {
                FxValue::Number(_) => "number",
                FxValue::Bool(_) => "boolean",
                FxValue::Color { .. } => "color",
                FxValue::Point { .. } => "point",
            },
            "base": self.base.to_json(),
            "animated": self.animated,
            "sampled": self.sampled.map(|value| value.to_json()),
            "on_key": on_key,
            "key_count": self.keys.len(),
            "keys": keys,
            "operations": operations,
        })
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;
    use crate::directed_scene::fx::{FILM_GRAIN, FxEasing, VIGNETTE};

    fn clip(effect: &'static crate::directed_scene::fx::EffectSpec) -> FxClip {
        FxClip {
            id: "g".into(),
            effect,
            start_frame: 10,
            duration_frames: 20,
            params: effect
                .params
                .iter()
                .map(|param| param.kind.default_value())
                .collect(),
            keys: Vec::new(),
            fade_in_frames: 0,
            fade_out_frames: 0,
            seed: None,
            hidden: false,
        }
    }

    #[test]
    fn a_clip_without_keys_is_static_and_its_sample_is_its_base() {
        let clip = clip(&VIGNETTE);
        let animation = ClipAnimation::of(&clip, Some(15));
        assert!(!animation.is_animated());
        assert_eq!(animation.frame_in_clip, Some(5));
        for param in &animation.params {
            assert_eq!(param.sampled, Some(param.base));
            assert!(param.keys.is_empty());
        }
        assert!(animation.seed.is_none(), "a vignette has no seed");
    }

    #[test]
    fn a_frame_outside_the_clip_samples_nothing_and_capture_says_why() {
        let clip = clip(&VIGNETTE);
        let before = ClipAnimation::of(&clip, Some(3));
        assert_eq!(before.frame_in_clip, None);
        let json = before.to_json(Locale::En);
        assert_eq!(json["active"], false);
        let capture = &json["params"][0]["operations"][0];
        assert_eq!(capture["name"], "capture");
        assert_eq!(capture["enabled"], false);
        assert_eq!(capture["reason"], REASON_FRAME_OUTSIDE_CLIP);
        let none = ClipAnimation::of(&clip, None).to_json(Locale::En);
        assert_eq!(
            none["params"][0]["operations"][0]["reason"],
            REASON_NO_FRAME
        );
        let end = ClipAnimation::of(&clip, Some(30));
        assert_eq!(end.frame_in_clip, None, "the end frame is exclusive");
    }

    #[test]
    fn keys_prevail_over_the_base_without_changing_it() {
        let mut clip = clip(&FILM_GRAIN);
        let index = FILM_GRAIN.param("amount").unwrap().0;
        let base = clip.params[index];
        clip.keys.push(crate::directed_scene::fx::ParamKeys {
            param: index,
            keys: vec![
                FxKey {
                    frame: 0,
                    value: FxValue::Number(0.0),
                    easing: FxEasing::Linear,
                },
                FxKey {
                    frame: 10,
                    value: FxValue::Number(1.0),
                    easing: FxEasing::Linear,
                },
            ],
        });
        let animation = ClipAnimation::of(&clip, Some(15));
        let amount = animation.param("amount").unwrap();
        assert!(amount.animated);
        assert_eq!(amount.base, base, "the base keeps its meaning");
        assert_eq!(amount.sampled, Some(FxValue::Number(0.5)));
        let json = animation.to_json(Locale::En);
        assert_eq!(json["animated"], true);
        let entry = json["params"]
            .as_array()
            .unwrap()
            .iter()
            .find(|param| param["param"] == "amount")
            .unwrap();
        assert_eq!(entry["key_count"], 2);
        assert_eq!(entry["sampled"], 0.5);
        assert_eq!(entry["keys"][1]["easing"], "linear");
        let ops: Vec<_> = entry["operations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|operation| {
                (
                    operation["name"].as_str().unwrap(),
                    operation["enabled"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            ops,
            [
                ("capture", true),
                ("shift_keys", true),
                ("scale_keys", true),
                ("remove_animation", true)
            ]
        );
        // A parameter that is not animated can only be captured.
        let size = json["params"]
            .as_array()
            .unwrap()
            .iter()
            .find(|param| param["param"] == "size")
            .unwrap();
        assert_eq!(size["animated"], false);
        assert_eq!(size["operations"][3]["enabled"], false);
        assert_eq!(size["operations"][3]["reason"], REASON_NO_KEYS);
    }

    #[test]
    fn the_seed_comes_from_the_id_until_one_is_written_and_follows_the_frame() {
        let mut clip = clip(&FILM_GRAIN);
        let from_id = ClipAnimation::of(&clip, Some(12)).seed.unwrap();
        assert_eq!(from_id.explicit, None);
        assert_eq!(from_id.reseed, "per-frame");
        let next = ClipAnimation::of(&clip, Some(13)).seed.unwrap();
        assert_eq!(from_id.base, next.base);
        assert_ne!(from_id.at_frame, next.at_frame, "a new seed on every frame");
        clip.seed = Some(7);
        let explicit = ClipAnimation::of(&clip, Some(12)).seed.unwrap();
        assert_eq!((explicit.explicit, explicit.base), (Some(7), 7));
        let json = ClipAnimation::of(&clip, Some(12)).to_json(Locale::En);
        assert_eq!(json["seed"]["operations"][1]["enabled"], true);
        let auto = ClipAnimation::of(&self::clip(&FILM_GRAIN), Some(12)).to_json(Locale::En);
        assert_eq!(auto["seed"]["operations"][1]["enabled"], false);
        assert_eq!(auto["seed"]["from_id"], true);
    }
}
