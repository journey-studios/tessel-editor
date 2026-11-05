//! Inspector tabs, accordion sections and per-clip property values.
//!
//! Values describe UI state only. Changing one does not promise rendering,
//! playback or export behavior.

use std::collections::HashMap;

use serde_json::{Value as Json, json};

use super::error::{CommandError, Raw};
use super::timeline::ClipKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InspectorTab {
    Video,
    Audio,
    Color,
}

impl InspectorTab {
    pub const ALL: [Self; 3] = [Self::Video, Self::Audio, Self::Color];

    pub fn id(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Color => "color",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tab| tab.id() == raw.trim())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Number(f64),
    Choice(&'static str),
    Bool(bool),
}

impl Value {
    pub fn to_json(self) -> Json {
        match self {
            Self::Number(number) => json!(number),
            Self::Choice(choice) => json!(choice),
            Self::Bool(flag) => json!(flag),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropertyKind {
    Slider {
        min: f64,
        max: f64,
        step: f64,
        decimals: u32,
        unit: &'static str,
    },
    Select {
        options: &'static [&'static str],
    },
    Toggle,
}

/// Which clips carry the property. Title clips are visual layers too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applies {
    Visual,
    Audio,
}

impl Applies {
    pub fn includes(self, kind: ClipKind) -> bool {
        match self {
            Self::Visual => matches!(kind, ClipKind::Video | ClipKind::Title | ClipKind::Scene),
            // A video clip carries its own sound: its levels are the same controls as an audio clip's.
            Self::Audio => matches!(kind, ClipKind::Audio | ClipKind::Video),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Visual => "video and title clips",
            Self::Audio => "audio and video clips",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SectionDef {
    pub id: &'static str,
    pub label: &'static str,
    pub tab: InspectorTab,
    pub applies: Applies,
    pub expanded: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct PropertyDef {
    pub id: &'static str,
    pub label: &'static str,
    pub section: &'static str,
    pub kind: PropertyKind,
    pub default: Value,
    /// The property is editable only while this toggle property is on.
    pub requires: Option<&'static str>,
}

pub const BLEND_MODES: &[&str] = &[
    "normal", "multiply", "screen", "overlay", "darken", "lighten",
];

const fn slider(min: f64, max: f64, step: f64, decimals: u32, unit: &'static str) -> PropertyKind {
    PropertyKind::Slider {
        min,
        max,
        step,
        decimals,
        unit,
    }
}

pub const SECTIONS: &[SectionDef] = &[
    SectionDef {
        id: "transform",
        label: "Transformar",
        tab: InspectorTab::Video,
        applies: Applies::Visual,
        expanded: true,
    },
    SectionDef {
        id: "composite",
        label: "Composição",
        tab: InspectorTab::Video,
        applies: Applies::Visual,
        expanded: true,
    },
    SectionDef {
        id: "cropping",
        label: "Recorte",
        tab: InspectorTab::Video,
        applies: Applies::Visual,
        expanded: false,
    },
    SectionDef {
        id: "timing",
        label: "Tempo",
        tab: InspectorTab::Video,
        applies: Applies::Visual,
        expanded: true,
    },
    SectionDef {
        id: "levels",
        label: "Níveis",
        tab: InspectorTab::Audio,
        applies: Applies::Audio,
        expanded: true,
    },
    SectionDef {
        id: "grading",
        label: "Correção de cor",
        tab: InspectorTab::Color,
        applies: Applies::Visual,
        expanded: true,
    },
];

pub const PROPERTIES: &[PropertyDef] = &[
    PropertyDef {
        id: "zoom",
        label: "Zoom",
        section: "transform",
        kind: slider(10.0, 400.0, 0.5, 1, "%"),
        default: Value::Number(100.0),
        requires: None,
    },
    PropertyDef {
        id: "position_x",
        label: "Posição X",
        section: "transform",
        kind: slider(-100.0, 100.0, 1.0, 0, "%"),
        default: Value::Number(0.0),
        requires: None,
    },
    PropertyDef {
        id: "position_y",
        label: "Posição Y",
        section: "transform",
        kind: slider(-100.0, 100.0, 1.0, 0, "%"),
        default: Value::Number(0.0),
        requires: None,
    },
    PropertyDef {
        id: "rotation",
        label: "Rotação",
        section: "transform",
        kind: slider(-180.0, 180.0, 0.5, 1, "°"),
        default: Value::Number(0.0),
        requires: None,
    },
    PropertyDef {
        id: "ken_burns",
        label: "Ken Burns",
        section: "transform",
        kind: PropertyKind::Toggle,
        default: Value::Bool(true),
        requires: None,
    },
    PropertyDef {
        id: "end_zoom",
        label: "Zoom final",
        section: "transform",
        kind: slider(10.0, 400.0, 0.5, 1, "%"),
        default: Value::Number(106.0),
        requires: Some("ken_burns"),
    },
    PropertyDef {
        id: "blend",
        label: "Mesclagem",
        section: "composite",
        kind: PropertyKind::Select {
            options: BLEND_MODES,
        },
        default: Value::Choice("normal"),
        requires: None,
    },
    PropertyDef {
        id: "opacity",
        label: "Opacidade",
        section: "composite",
        kind: slider(0.0, 100.0, 1.0, 0, "%"),
        default: Value::Number(100.0),
        requires: None,
    },
    PropertyDef {
        id: "crop_left",
        label: "Esquerda",
        section: "cropping",
        kind: slider(0.0, 45.0, 1.0, 0, "%"),
        default: Value::Number(0.0),
        requires: None,
    },
    PropertyDef {
        id: "crop_right",
        label: "Direita",
        section: "cropping",
        kind: slider(0.0, 45.0, 1.0, 0, "%"),
        default: Value::Number(0.0),
        requires: None,
    },
    PropertyDef {
        id: "crop_top",
        label: "Topo",
        section: "cropping",
        kind: slider(0.0, 45.0, 1.0, 0, "%"),
        default: Value::Number(0.0),
        requires: None,
    },
    PropertyDef {
        id: "crop_bottom",
        label: "Base",
        section: "cropping",
        kind: slider(0.0, 45.0, 1.0, 0, "%"),
        default: Value::Number(0.0),
        requires: None,
    },
    PropertyDef {
        id: "speed",
        label: "Velocidade",
        section: "timing",
        kind: slider(25.0, 400.0, 5.0, 0, "%"),
        default: Value::Number(100.0),
        requires: None,
    },
    PropertyDef {
        id: "fade_in",
        label: "Entrada",
        section: "timing",
        kind: slider(0.0, 48.0, 1.0, 0, "fr"),
        default: Value::Number(0.0),
        requires: None,
    },
    PropertyDef {
        id: "fade_out",
        label: "Saída",
        section: "timing",
        kind: slider(0.0, 48.0, 1.0, 0, "fr"),
        default: Value::Number(0.0),
        requires: None,
    },
    PropertyDef {
        id: "volume",
        label: "Volume",
        section: "levels",
        kind: slider(-60.0, 12.0, 0.5, 1, "dB"),
        default: Value::Number(0.0),
        requires: None,
    },
    PropertyDef {
        id: "pan",
        label: "Pan",
        section: "levels",
        kind: slider(-100.0, 100.0, 1.0, 0, "%"),
        default: Value::Number(0.0),
        requires: None,
    },
    PropertyDef {
        id: "audio_muted",
        label: "Mudo",
        section: "levels",
        kind: PropertyKind::Toggle,
        default: Value::Bool(false),
        requires: None,
    },
    // Which audio stream of a container video the clip plays (`0:a:<n>`). A stream the file
    // does not have is a typed error at plan time, never silence.
    PropertyDef {
        id: "audio_stream",
        label: "Faixa de áudio",
        section: "levels",
        kind: slider(0.0, 7.0, 1.0, 0, ""),
        default: Value::Number(0.0),
        requires: None,
    },
    PropertyDef {
        id: "brightness",
        label: "Brilho",
        section: "grading",
        kind: slider(-100.0, 100.0, 1.0, 0, ""),
        default: Value::Number(0.0),
        requires: None,
    },
    PropertyDef {
        id: "contrast",
        label: "Contraste",
        section: "grading",
        kind: slider(-100.0, 100.0, 1.0, 0, ""),
        default: Value::Number(0.0),
        requires: None,
    },
    PropertyDef {
        id: "saturation",
        label: "Saturação",
        section: "grading",
        kind: slider(-100.0, 100.0, 1.0, 0, ""),
        default: Value::Number(0.0),
        requires: None,
    },
];

pub fn property_def(id: &str) -> Option<&'static PropertyDef> {
    PROPERTIES.iter().find(|property| property.id == id)
}

pub fn section_def(id: &str) -> Option<&'static SectionDef> {
    SECTIONS.iter().find(|section| section.id == id)
}

pub fn section_properties(section: &str) -> impl Iterator<Item = &'static PropertyDef> {
    PROPERTIES
        .iter()
        .filter(move |property| property.section == section)
}

fn round_to(value: f64, decimals: u32) -> f64 {
    let scale = 10f64.powi(decimals as i32);
    (value * scale).round() / scale
}

#[derive(Debug, Clone)]
pub struct Inspector {
    pub tab: InspectorTab,
    expanded: HashMap<&'static str, bool>,
    /// Only values that differ from the default are stored, per clip.
    overrides: HashMap<String, HashMap<&'static str, Value>>,
}

impl Default for Inspector {
    fn default() -> Self {
        Self {
            tab: InspectorTab::Video,
            expanded: SECTIONS
                .iter()
                .map(|section| (section.id, section.expanded))
                .collect(),
            overrides: HashMap::new(),
        }
    }
}

/// Per-clip property values that differ from the defaults (the persistent part of the inspector).
pub type Overrides = HashMap<String, HashMap<&'static str, Value>>;

impl Inspector {
    /// Overrides in a stable order, for the project document.
    pub fn overrides_sorted(&self) -> Vec<(String, &'static str, Value)> {
        let mut all: Vec<_> = self
            .overrides
            .iter()
            .flat_map(|(clip, values)| {
                values
                    .iter()
                    .map(|(property, value)| (clip.clone(), *property, *value))
            })
            .collect();
        all.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        all
    }

    /// Copies the overrides of `old` in `from` onto `new` (used when a clip is merged in from
    /// another project under a different ID).
    pub fn copy_overrides_from(&mut self, from: &Inspector, old: &str, new: &str) {
        if let Some(values) = from.overrides.get(old) {
            self.overrides.insert(new.to_string(), values.clone());
        }
    }

    /// Drops the values of a clip that no longer exists.
    pub fn forget_clip(&mut self, clip: &str) {
        self.overrides.remove(clip);
    }

    pub fn overrides_snapshot(&self) -> Overrides {
        self.overrides.clone()
    }

    pub fn restore_overrides(&mut self, overrides: Overrides) {
        self.overrides = overrides;
    }

    /// Stores one persisted value after checking it against the property definition, so a
    /// hand-edited document cannot put an out-of-range number or a foreign choice in the state.
    pub fn restore_override(
        &mut self,
        clip: &str,
        id: &str,
        raw: &Json,
    ) -> Result<(), CommandError> {
        let property =
            property_def(id).ok_or_else(|| CommandError::unknown_element("property", id))?;
        let value = match (property.kind, raw) {
            (PropertyKind::Slider { min, max, .. }, Json::Number(number)) => number
                .as_f64()
                .filter(|number| number.is_finite() && (min..=max).contains(number))
                .map(Value::Number),
            (PropertyKind::Select { options }, Json::String(text)) => options
                .iter()
                .find(|option| **option == text)
                .map(|option| Value::Choice(option)),
            (PropertyKind::Toggle, Json::Bool(flag)) => Some(Value::Bool(*flag)),
            _ => None,
        }
        .ok_or_else(|| {
            CommandError::new(
                "invalid_document",
                format!("Property `{id}` of clip `{clip}` has an invalid value"),
            )
        })?;
        self.store(clip, property, value);
        Ok(())
    }

    pub fn is_expanded(&self, section: &str) -> bool {
        self.expanded.get(section).copied().unwrap_or(false)
    }

    pub fn set_tab(&mut self, tab: InspectorTab) -> Json {
        let previous = std::mem::replace(&mut self.tab, tab);
        json!({ "target": "inspector.tabs", "field": "value", "value": tab.id(),
            "previous": previous.id(), "changed": previous != tab })
    }

    /// Accordions keep their property values while collapsed.
    pub fn set_expanded(&mut self, section: &str, expanded: bool) -> Result<Json, CommandError> {
        let def = section_def(section)
            .ok_or_else(|| CommandError::unknown_element("section", section))?;
        let previous = self.expanded.insert(def.id, expanded).unwrap_or(false);
        Ok(
            json!({ "target": format!("inspector.{section}"), "field": "expanded",
            "value": expanded, "previous": previous, "changed": previous != expanded }),
        )
    }

    pub fn value(&self, clip: impl AsRef<str>, property: &PropertyDef) -> Value {
        self.overrides
            .get(clip.as_ref())
            .and_then(|values| values.get(property.id))
            .copied()
            .unwrap_or(property.default)
    }

    pub fn is_modified(&self, clip: impl AsRef<str> + Copy, property: &PropertyDef) -> bool {
        self.value(clip, property) != property.default
    }

    pub fn section_modified(&self, clip: impl AsRef<str> + Copy, section: &str) -> bool {
        section_properties(section).any(|property| self.is_modified(clip, property))
    }

    pub fn is_enabled(&self, clip: impl AsRef<str> + Copy, property: &PropertyDef) -> bool {
        property.requires.is_none_or(|required| {
            property_def(required)
                .is_some_and(|dependency| self.value(clip, dependency) == Value::Bool(true))
        })
    }

    fn lookup(
        kind: ClipKind,
        id: &str,
    ) -> Result<(&'static PropertyDef, &'static SectionDef), CommandError> {
        let property =
            property_def(id).ok_or_else(|| CommandError::unknown_element("property", id))?;
        let section = section_def(property.section)
            .ok_or_else(|| CommandError::unknown_element("section", property.section))?;
        if !section.applies.includes(kind) {
            return Err(CommandError::new(
                "not_applicable",
                format!("`{id}` applies to {} only", section.applies.label()),
            )
            .target(format!("property:{id}")));
        }
        Ok((property, section))
    }

    /// Validates `raw` against the property definition.
    ///
    /// Out-of-range numbers are clamped to the slider limits (the same rule a
    /// drag or arrow key follows) and reported as `clamped`; text that is not a
    /// finite number, or a choice that is not an option, is rejected.
    pub fn set_property(
        &mut self,
        clip: impl AsRef<str> + Copy,
        kind: ClipKind,
        id: &str,
        raw: &Raw,
    ) -> Result<Json, CommandError> {
        let (property, _) = Self::lookup(kind, id)?;
        let target = format!("property:{id}");
        if !self.is_enabled(clip, property) {
            return Err(CommandError::disabled(format!(
                "`{id}` is available only while `{}` is on",
                property.requires.unwrap_or_default()
            ))
            .target(target));
        }
        let (value, clamped) = match property.kind {
            PropertyKind::Slider {
                min, max, decimals, ..
            } => {
                let requested = raw.number().map_err(|error| error.target(&target))?;
                let bounded = requested.clamp(min, max);
                (
                    Value::Number(round_to(bounded, decimals)),
                    bounded != requested,
                )
            }
            PropertyKind::Select { options } => {
                let text = raw.text();
                let choice = options
                    .iter()
                    .find(|option| **option == text.trim())
                    .ok_or_else(|| {
                        CommandError::invalid_value(
                            format!("`{text}` is not an option"),
                            json!({ "options": options }),
                        )
                        .target(&target)
                    })?;
                (Value::Choice(choice), false)
            }
            PropertyKind::Toggle => (
                Value::Bool(raw.boolean().map_err(|error| error.target(&target))?),
                false,
            ),
        };
        let previous = self.value(clip, property);
        self.store(clip, property, value);
        Ok(
            json!({ "target": target, "field": "value", "clip": format!("clip:{}", clip.as_ref()),
            "value": value.to_json(), "previous": previous.to_json(),
            "clamped": clamped, "changed": previous != value }),
        )
    }

    fn store(&mut self, clip: impl AsRef<str>, property: &PropertyDef, value: Value) {
        let values = self.overrides.entry(clip.as_ref().to_string()).or_default();
        if value == property.default {
            values.remove(property.id);
        } else {
            values.insert(property.id, value);
        }
    }

    pub fn reset_property(
        &mut self,
        clip: impl AsRef<str> + Copy,
        kind: ClipKind,
        id: &str,
    ) -> Result<Json, CommandError> {
        let (property, _) = Self::lookup(kind, id)?;
        let previous = self.value(clip, property);
        self.store(clip, property, property.default);
        Ok(
            json!({ "target": format!("property:{id}"), "action": "reset",
            "value": property.default.to_json(), "previous": previous.to_json(),
            "changed": previous != property.default }),
        )
    }

    pub fn reset_section(
        &mut self,
        clip: impl AsRef<str> + Copy,
        kind: ClipKind,
        section: &str,
    ) -> Result<Json, CommandError> {
        let def = section_def(section)
            .ok_or_else(|| CommandError::unknown_element("section", section))?;
        if !def.applies.includes(kind) {
            return Err(CommandError::new(
                "not_applicable",
                format!("`{section}` applies to {} only", def.applies.label()),
            )
            .target(format!("inspector.{section}")));
        }
        let mut changed = Vec::new();
        for property in section_properties(section) {
            if self.is_modified(clip, property) {
                changed.push(property.id);
                self.store(clip, property, property.default);
            }
        }
        Ok(
            json!({ "target": format!("inspector.{section}"), "action": "reset",
            "clip": format!("clip:{}", clip.as_ref()), "reset_properties": changed,
            "changed": !changed.is_empty() }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(value: f64) -> Raw {
        Raw::Number(value)
    }

    #[test]
    fn sliders_clamp_round_and_report() {
        let mut inspector = Inspector::default();
        let out = inspector
            .set_property("c", ClipKind::Video, "zoom", &Raw::Text("110".into()))
            .unwrap();
        assert_eq!(out["value"], 110.0);
        assert_eq!(out["clamped"], false);
        let out = inspector
            .set_property("c", ClipKind::Video, "zoom", &number(9999.0))
            .unwrap();
        assert_eq!(out["value"], 400.0);
        assert_eq!(out["clamped"], true);
        let out = inspector
            .set_property("c", ClipKind::Video, "zoom", &number(100.04))
            .unwrap();
        assert_eq!(out["value"], 100.0);
        assert_eq!(out["changed"], true);
    }

    #[test]
    fn invalid_values_are_rejected_without_changing_state() {
        let mut inspector = Inspector::default();
        inspector
            .set_property("c", ClipKind::Video, "opacity", &number(40.0))
            .unwrap();
        for bad in ["abc", "NaN", "inf", ""] {
            let error = inspector
                .set_property("c", ClipKind::Video, "opacity", &Raw::Text(bad.into()))
                .unwrap_err();
            assert_eq!(error.code, "invalid_value", "{bad}");
        }
        let property = property_def("opacity").unwrap();
        assert_eq!(inspector.value("c", property), Value::Number(40.0));
    }

    #[test]
    fn select_accepts_only_listed_options() {
        let mut inspector = Inspector::default();
        let out = inspector
            .set_property("c", ClipKind::Video, "blend", &Raw::Text("multiply".into()))
            .unwrap();
        assert_eq!(out["value"], "multiply");
        let error = inspector
            .set_property("c", ClipKind::Video, "blend", &Raw::Text("glow".into()))
            .unwrap_err();
        assert_eq!(error.code, "invalid_value");
        assert_eq!(error.details.unwrap()["expected"]["options"][0], "normal");
        assert_eq!(
            inspector.value("c", property_def("blend").unwrap()),
            Value::Choice("multiply")
        );
    }

    #[test]
    fn values_are_per_clip_and_reset_per_section() {
        let mut inspector = Inspector::default();
        inspector
            .set_property("a", ClipKind::Video, "zoom", &number(150.0))
            .unwrap();
        inspector
            .set_property("a", ClipKind::Video, "opacity", &number(50.0))
            .unwrap();
        let zoom = property_def("zoom").unwrap();
        assert_eq!(inspector.value("b", zoom), Value::Number(100.0));
        let out = inspector
            .reset_section("a", ClipKind::Video, "transform")
            .unwrap();
        assert_eq!(out["reset_properties"], json!(["zoom"]));
        assert_eq!(inspector.value("a", zoom), Value::Number(100.0));
        assert_eq!(
            inspector.value("a", property_def("opacity").unwrap()),
            Value::Number(50.0)
        );
    }

    #[test]
    fn collapsing_keeps_values_and_dependent_rows_follow_their_toggle() {
        let mut inspector = Inspector::default();
        inspector
            .set_property("c", ClipKind::Video, "end_zoom", &number(120.0))
            .unwrap();
        inspector.set_expanded("transform", false).unwrap();
        assert!(!inspector.is_expanded("transform"));
        inspector.set_expanded("transform", true).unwrap();
        assert_eq!(
            inspector.value("c", property_def("end_zoom").unwrap()),
            Value::Number(120.0)
        );
        inspector
            .set_property("c", ClipKind::Video, "ken_burns", &Raw::Bool(false))
            .unwrap();
        let error = inspector
            .set_property("c", ClipKind::Video, "end_zoom", &number(110.0))
            .unwrap_err();
        assert_eq!(error.code, "disabled_element");
    }

    #[test]
    fn properties_do_not_apply_across_clip_kinds() {
        let mut inspector = Inspector::default();
        assert_eq!(
            inspector
                .set_property("s", ClipKind::Audio, "zoom", &number(120.0))
                .unwrap_err()
                .code,
            "not_applicable"
        );
        assert!(
            inspector
                .set_property("s", ClipKind::Audio, "volume", &number(-6.0))
                .is_ok()
        );
        assert_eq!(
            inspector
                .set_property("c", ClipKind::Video, "nope", &number(1.0))
                .unwrap_err()
                .code,
            "unknown_element"
        );
    }

    #[test]
    fn tabs_parse_and_switching_reports_the_change() {
        // The Kit Select owns its popup, so there is no open-list state to reset.
        assert_eq!(InspectorTab::parse("audio"), Some(InspectorTab::Audio));
        assert_eq!(InspectorTab::parse("grade"), None);
        let mut inspector = Inspector::default();
        let out = inspector.set_tab(InspectorTab::Color);
        assert_eq!(out["changed"], true);
        assert_eq!(inspector.tab, InspectorTab::Color);
        assert_eq!(inspector.set_tab(InspectorTab::Color)["changed"], false);
    }
}
