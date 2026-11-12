//! The scope choice of an effect clip: the one setting of the inspector that is one of a few
//! named options rather than a number (F062, BL082.3). It lives beside [`super::fx_inspector`]
//! like the other ops of the layer live beside `fx` (`fx_keys`, `fx_intent`, `fx_animation`): the
//! model is the same kind of value (the id, three languages, the value, the options and the op),
//! but it needs the shots of the edit, which a number field does not.
//!
//! The rules are not here. Whether a shot holds a clip is [`shot_holds`] (the containment half of
//! the one rule [`scope_fault`] applies at import and at edit), so an option is listed with the
//! code the edit would refuse with, never left out and never enabled.

use serde_json::{Value, json};

use crate::directed_scene::Shot;
use crate::directed_scene::fx::{
    FX_SCOPE_MISMATCH, FX_SCOPE_NOT_YET, FxClip, FxScope, Labels, Locale, scope_end, scope_fault,
    scope_start, shot_holds,
};
use crate::state::CommandError;

use super::FxOp;

/// The scopes an effect can be laid on and whether this build draws them: the output and (F062) a
/// shot; `node` is refused, with its code, until a node target exists.
pub fn scopes() -> Vec<(&'static str, bool, Option<&'static str>)> {
    vec![
        ("output", true, None),
        ("shot", true, None),
        ("node", false, Some(FX_SCOPE_NOT_YET)),
    ]
}

/// The frames the timing fields of `clip` can take: `((first start, last start), longest duration)`.
/// A clip on the output may start anywhere in the edit; a clip on a shot lies inside it (the rule
/// [`scope_fault`] judges), so its start stops where the clip as it is would pass the shot's end and
/// its duration where it would pass it from where it starts: the shot's limits, never the edit's.
pub(super) fn timing_limits(clip: &FxClip, edit_frames: i64, shots: &[Shot]) -> ((i64, i64), i64) {
    if clip.scope == FxScope::Output {
        return ((0, edit_frames - 1), edit_frames);
    }
    let end = scope_end(&clip.scope, shots, edit_frames);
    (
        (scope_start(&clip.scope, shots), end - clip.duration_frames),
        end - clip.start_frame,
    )
}

/// The words of the scope choice (the model's first setting that is one of a few named options
/// rather than a number) and of its options.
pub const SCOPE_LABEL: Labels = Labels::new("Escopo", "Scope", "Alcance");
pub const OUTPUT_LABEL: Labels = Labels::new("Saída", "Output", "Salida");
/// The word before a shot's id in its option (`Plano s1`, `Shot s1`).
pub const SHOT_LABEL: Labels = Labels::new("Plano", "Shot", "Plano");

/// The text of a scope in `locale`: `Saída`, `Plano s1`. The one place these words are put
/// together: the options of the choice and a window's title read it.
pub fn scope_label(scope: &FxScope, locale: Locale) -> String {
    match scope {
        FxScope::Output => OUTPUT_LABEL.get(locale).to_string(),
        FxScope::Shot(id) => format!("{} {id}", SHOT_LABEL.get(locale)),
    }
}

/// One option of an [`InspectorChoice`]: where a clip can be laid.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceOption {
    /// What an op carries for it ([`FxScope::to_text`]): `output`, `shot:<id>`.
    pub value: String,
    pub scope: FxScope,
    /// A shot's own note about what it is for (`intent`): the edit's text, not translated.
    pub intent: Option<String>,
    /// The frames `[start, end)` of a shot, in the edit.
    pub frames: Option<(u64, u64)>,
    /// Whether the clip may go there as it stands. A shot that does not hold the whole clip is
    /// listed with the code the edit would refuse with, never left out and never enabled.
    pub applicable: bool,
    pub reason: Option<&'static str>,
}

impl ChoiceOption {
    /// The text of the option in `locale`: `Saída`, `Plano s1`.
    pub fn label(&self, locale: Locale) -> String {
        scope_label(&self.scope, locale)
    }

    fn to_json(&self, locale: Locale, selected: bool, clip: &str) -> Value {
        let labels = json!({
            "pt-br": self.label(Locale::PtBr), "en": self.label(Locale::En),
            "es": self.label(Locale::Es),
        });
        json!({
            "value": self.value, "kind": self.scope.kind(), "shot": self.scope.shot_id(),
            "label": self.label(locale), "labels": labels,
            "intent": self.intent,
            "start_frame": self.frames.map(|(start, _)| start),
            "end_frame_exclusive": self.frames.map(|(_, end)| end),
            "selected": selected, "applicable": self.applicable, "reason": self.reason,
            // The very op a `scene.fx.edit` would run to choose it (nothing to do when it is
            // the one the clip holds already, or when it cannot be chosen).
            "op": (self.applicable && !selected)
                .then(|| json!({ "op": "set_scope", "clip": clip, "scope": self.value })),
        })
    }
}

/// A setting of a clip that is one of a few named options, not a number (F062: the scope). The
/// neutral model carries it like a field: the id, three languages, the value, the options and the
/// op, so a window, a script and a test read one list.
#[derive(Debug, Clone, PartialEq)]
pub struct InspectorChoice {
    /// The node field name: `scope`.
    pub id: &'static str,
    pub base: Labels,
    /// The `value` of the option the clip holds.
    pub value: String,
    /// The option a reset restores (`output`).
    pub default: String,
    /// The output first, then the shots in document order.
    pub options: Vec<ChoiceOption>,
}

impl InspectorChoice {
    pub fn label(&self, locale: Locale) -> &'static str {
        self.base.get(locale)
    }

    pub fn at_default(&self) -> bool {
        self.value == self.default
    }

    pub fn option(&self, value: &str) -> Option<&ChoiceOption> {
        self.options.iter().find(|option| option.value == value)
    }

    /// The edit op that moves `clip` to the option `value` (the text of a scope). Refuses, with a
    /// typed code, text that is no scope, and a `node` scope (`fx_scope_not_yet`); whether the
    /// shot exists and holds the clip is the core's, which judges the op when it runs. It needs
    /// no choice to build, so an adapter that only writes the scope does not list the options.
    pub fn set_scope_op(clip: &FxClip, value: &str) -> Result<FxOp, CommandError> {
        let scope = FxScope::parse_text(value).map_err(|(code, message)| {
            if code == FX_SCOPE_NOT_YET {
                CommandError::new("fx_scope_not_yet", message)
            } else {
                CommandError::invalid_value(message, json!("output | shot:<id>"))
            }
        })?;
        Ok(FxOp::SetScope {
            clip: clip.id.clone(),
            scope,
        })
    }

    pub fn to_json(&self, clip: &str, locale: Locale) -> Value {
        json!({
            "id": self.id, "group": "scope", "type": "choice",
            "label": self.label(locale), "labels": self.base.to_json(),
            "value": self.value, "default": self.default, "at_default": self.at_default(),
            "options": self.options.iter()
                .map(|option| option.to_json(locale, option.value == self.value, clip))
                .collect::<Vec<_>>(),
            "applicable": true, "reason": Value::Null,
        })
    }
}

/// The scope choice of a clip against the shots of the edit: the output, then each shot with
/// whether it holds the clip as it stands. The same choice [`ClipInspector::of`] lists, for a
/// window that already holds the clip.
///
/// [`ClipInspector::of`]: super::ClipInspector::of
pub fn scope_choice(clip: &FxClip, shots: &[Shot]) -> InspectorChoice {
    let mut options = vec![ChoiceOption {
        value: FxScope::Output.to_text(),
        scope: FxScope::Output,
        intent: None,
        frames: None,
        applicable: true,
        reason: None,
    }];
    options.extend(shots.iter().map(|shot| {
        let scope = FxScope::Shot(shot.id.clone());
        let applicable = shot_holds(shot, clip.start_frame, clip.duration_frames);
        ChoiceOption {
            value: scope.to_text(),
            scope,
            intent: Some(shot.intent.clone()),
            frames: Some((shot.start_frame, shot.end_frame_exclusive)),
            applicable,
            reason: (!applicable).then_some(FX_SCOPE_MISMATCH),
        }
    }));
    // The scope the clip holds is always among the options, even when the shots given do not
    // list it (a layer read without its edit): there, it is shown, and not offered, with the
    // refusal the rule gives it.
    if !options.iter().any(|option| option.scope == clip.scope) {
        let fault = scope_fault(
            &clip.scope,
            &clip.id,
            clip.start_frame,
            clip.duration_frames,
            shots,
        );
        options.push(ChoiceOption {
            value: clip.scope.to_text(),
            scope: clip.scope.clone(),
            intent: None,
            frames: None,
            applicable: false,
            reason: fault.map(|fault| fault.code),
        });
    }
    InspectorChoice {
        id: "scope",
        base: SCOPE_LABEL,
        value: clip.scope.to_text(),
        default: FxScope::Output.to_text(),
        options,
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;
    use crate::directed_scene::fx::{UNKNOWN_SHOT, VIGNETTE};

    /// A vignette clip on frames 10..30 of the output.
    fn clip() -> FxClip {
        FxClip {
            id: "c1".into(),
            effect: &VIGNETTE,
            start_frame: 10,
            duration_frames: 20,
            params: VIGNETTE
                .params
                .iter()
                .map(|param| param.kind.default_value())
                .collect(),
            keys: Vec::new(),
            fade_in_frames: 0,
            fade_out_frames: 0,
            seed: None,
            hidden: false,
            scope: FxScope::Output,
        }
    }

    /// Two shots, `a` `[0, 15)` and `b` `[15, 96)`, the second with a note.
    fn two_shots() -> Vec<Shot> {
        [("a", 0, 15, ""), ("b", 15, 96, "close on the lamp")]
            .into_iter()
            .map(|(id, start, end, intent)| Shot {
                id: id.to_string(),
                scene_instance_id: "i1".to_string(),
                start_frame: start,
                end_frame_exclusive: end,
                instance_start_frame: 0,
                camera_clip_id: "cam".to_string(),
                camera_in_frame: 0,
                transition_into: None,
                intent: intent.to_string(),
            })
            .collect()
    }

    #[test]
    fn the_scopes_say_what_is_not_drawn() {
        // F062: the output and a shot are drawn; a node target does not exist yet and is listed
        // with the code that refuses it, never left out and never enabled.
        assert_eq!(
            scopes(),
            [
                ("output", true, None),
                ("shot", true, None),
                ("node", false, Some(FX_SCOPE_NOT_YET)),
            ]
        );
    }

    #[test]
    fn the_scope_choice_lists_the_output_then_the_shots_that_hold_the_clip() {
        // The clip is frames 10..30: it crosses the cut at frame 15, so neither shot holds it.
        let choice = scope_choice(&clip(), &two_shots());
        assert_eq!(choice.value, "output");
        assert_eq!(choice.default, "output");
        assert!(choice.at_default());
        let options: Vec<(&str, bool, Option<&str>)> = choice
            .options
            .iter()
            .map(|option| (option.value.as_str(), option.applicable, option.reason))
            .collect();
        assert_eq!(
            options,
            [
                ("output", true, None),
                ("shot:a", false, Some("FX_SCOPE_MISMATCH")),
                ("shot:b", false, Some("FX_SCOPE_MISMATCH")),
            ]
        );
        // Frames 20..40 are inside `b` only, and `b` carries its note and its interval.
        let mut inside = clip();
        inside.start_frame = 20;
        let choice = scope_choice(&inside, &two_shots());
        let b = choice.option("shot:b").unwrap();
        assert!(b.applicable, "{b:?}");
        assert_eq!(b.intent.as_deref(), Some("close on the lamp"));
        assert_eq!(b.frames, Some((15, 96)));
        assert!(!choice.option("shot:a").unwrap().applicable);
    }

    #[test]
    fn the_scope_choice_has_every_text_in_three_languages_and_the_op_it_runs() {
        let mut clip = clip();
        clip.start_frame = 20;
        clip.scope = FxScope::Shot("b".into());
        let choice = scope_choice(&clip, &two_shots());
        assert_eq!(choice.value, "shot:b");
        assert!(!choice.at_default());
        for locale in Locale::ALL {
            assert!(!choice.label(locale).is_empty());
            let json = choice.to_json("c1", locale);
            assert_eq!(json["type"], "choice");
            assert_eq!(json["value"], "shot:b");
            for option in json["options"].as_array().unwrap() {
                assert!(!option["label"].as_str().unwrap().is_empty());
            }
        }
        let json = choice.to_json("c1", Locale::PtBr);
        let options = json["options"].as_array().unwrap();
        assert_eq!(options[0]["labels"]["pt-br"], "Saída");
        assert_eq!(options[0]["labels"]["en"], "Output");
        assert_eq!(options[0]["labels"]["es"], "Salida");
        assert_eq!(options[2]["labels"]["pt-br"], "Plano b");
        assert_eq!(options[2]["labels"]["en"], "Shot b");
        assert_eq!(options[2]["selected"], true);
        // Choosing the shot the clip holds is nothing to do; choosing the output is the op.
        assert_eq!(options[2]["op"], Value::Null);
        assert_eq!(
            options[0]["op"],
            json!({ "op": "set_scope", "clip": "c1", "scope": "output" })
        );
        // The shot `a` does not hold the clip (frames 20..40): listed, never offered.
        assert_eq!(options[1]["applicable"], false);
        assert_eq!(options[1]["reason"], "FX_SCOPE_MISMATCH");
        assert_eq!(options[1]["op"], Value::Null);
    }

    #[test]
    fn a_scope_is_worded_once_for_the_options_and_for_whoever_titles_a_clip() {
        let shot = FxScope::Shot("b".into());
        assert_eq!(scope_label(&FxScope::Output, Locale::PtBr), "Saída");
        assert_eq!(scope_label(&shot, Locale::PtBr), "Plano b");
        assert_eq!(scope_label(&shot, Locale::En), "Shot b");
        let choice = scope_choice(&clip(), &two_shots());
        for option in &choice.options {
            assert_eq!(
                option.label(Locale::Es),
                scope_label(&option.scope, Locale::Es)
            );
        }
    }

    #[test]
    fn the_set_scope_op_needs_no_choice_and_refuses_what_is_not_a_scope() {
        let clip = clip();
        assert_eq!(
            InspectorChoice::set_scope_op(&clip, "shot:b").unwrap(),
            FxOp::SetScope {
                clip: "c1".into(),
                scope: FxScope::Shot("b".into()),
            }
        );
        assert_eq!(
            InspectorChoice::set_scope_op(&clip, "node:n1")
                .unwrap_err()
                .code,
            "fx_scope_not_yet"
        );
        assert_eq!(
            InspectorChoice::set_scope_op(&clip, "frame")
                .unwrap_err()
                .code,
            "invalid_value"
        );
    }

    #[test]
    fn a_scope_the_shots_given_do_not_list_is_shown_and_not_offered() {
        let mut clip = clip();
        clip.scope = FxScope::Shot("ghost".into());
        let choice = scope_choice(&clip, &[]);
        assert_eq!(choice.value, "shot:ghost");
        let ghost = choice.option("shot:ghost").unwrap();
        assert!(!ghost.applicable);
        assert_eq!(ghost.reason, Some(UNKNOWN_SHOT));
    }
}
