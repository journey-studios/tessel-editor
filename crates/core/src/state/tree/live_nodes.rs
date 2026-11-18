//! The inspector nodes of a player clip and of a subtitle clip (F071).
//!
//! Every node is a target of `set`/`invoke` (`clip_player:<clip>`, `clip_subtitles:<clip>`,
//! `subtitle_cue:<clip>/<index>`), so a script edits exactly what the inspector shows.

use serde_json::json;

use super::Node;
use crate::content::TEXT_MAX_CHARS;
use crate::player::{MAX_BARS, MAX_EXTENT, MIN_BARS, PlayerSpec};
use crate::state::timeline::Clip;
use crate::subtitles::{MAX_CUE_MS, MAX_CUES, SubtitleSpec};

fn extent(unit: &'static str, min: f64, max: f64) -> serde_json::Value {
    json!({ "min": min, "max": max, "unit": unit })
}

/// The player of the selected player clip: one state entry and one typed field per spec field.
pub(super) fn build_player(clip: &Clip, parent: &str, shown: bool, nodes: &mut Vec<Node>) {
    let Some(player) = &clip.player else {
        return;
    };
    let spec = player.to_json();
    let mut node = Node::new(format!("clip_player:{}", clip.id), "player-input", "Player")
        .parent(parent)
        .visible(shown)
        .state("clip", format!("clip:{}", clip.id));
    for field in PlayerSpec::FIELDS {
        node = node.state(field, spec[field].clone());
    }
    let colors = json!({ "format": "#rrggbb or #rrggbbaa" });
    nodes.push(
        node.field(
            "audio",
            "text",
            json!({ "format": "ID of an audio clip of the sequence, or none" }),
        )
        .field(
            "width",
            "number",
            extent("% of the canvas width", 1.0, MAX_EXTENT),
        )
        .field(
            "height",
            "number",
            extent("% of the canvas height", 1.0, MAX_EXTENT),
        )
        .field(
            "bars",
            "integer",
            json!({ "min": MIN_BARS, "max": MAX_BARS }),
        )
        .field("gap", "number", extent("share of a bar's slot", 0.0, 0.9))
        .field("wave", "boolean", json!({}))
        .field("bar", "boolean", json!({}))
        .field("time", "boolean", json!({}))
        .field("wave_color", "text", colors.clone())
        .field("idle_color", "text", colors.clone())
        .field("track_color", "text", colors.clone())
        .field("fill_color", "text", colors)
        .field("text_color", "text", json!({ "format": "#rrggbb" }))
        .field(
            "bar_height",
            "number",
            extent("% of the canvas height", 0.1, 20.0),
        )
        .field(
            "text_size",
            "number",
            extent("% of the canvas height", 1.0, 30.0),
        )
        .field(
            "spacing",
            "number",
            extent("% of the canvas height", 0.0, 20.0),
        ),
    );
}

/// The caption track of the selected subtitle clip: the style node (whose `add_cue` action puts
/// a cue at the playhead) and one node per cue.
pub(super) fn build_subtitles(clip: &Clip, parent: &str, shown: bool, nodes: &mut Vec<Node>) {
    let Some(spec) = &clip.subtitles else {
        return;
    };
    let json = spec.to_json();
    let mut style = Node::new(
        format!("clip_subtitles:{}", clip.id),
        "subtitles-input",
        "Legendas",
    )
    .parent(parent)
    .visible(shown)
    .state("clip", format!("clip:{}", clip.id))
    .state("cue_count", spec.cues.len())
    .state("max_cues", MAX_CUES)
    .state("last_end_ms", spec.last_end_ms());
    for field in SubtitleSpec::STYLE_FIELDS {
        style = style.state(field, json[field].clone());
    }
    style = style
        .field(
            "size",
            "number",
            extent("% of the canvas height", 1.0, 30.0),
        )
        .field("color", "text", json!({ "format": "#rrggbb" }))
        .field(
            "width",
            "number",
            extent("% of the canvas width", 5.0, 100.0),
        )
        .field("y", "number", extent("% of the canvas height", -50.0, 50.0))
        .field(
            "box",
            "text",
            json!({ "format": "#rrggbb, #rrggbbaa or none" }),
        )
        .field("pad", "number", extent("% of the canvas height", 0.0, 20.0))
        .field(
            "radius",
            "number",
            extent("% of the canvas height", 0.0, 20.0),
        )
        .field("burn", "boolean", json!({}))
        .action("add_cue");
    nodes.push(style);
    for (index, cue) in spec.cues.iter().enumerate() {
        nodes.push(
            Node::new(
                format!("subtitle_cue:{}/{index}", clip.id),
                "subtitle-cue",
                format!("Legenda {}", index + 1),
            )
            .parent(parent)
            .visible(shown)
            .state("clip", format!("clip:{}", clip.id))
            .state("index", index)
            .state("start_ms", cue.start_ms)
            .state("end_ms", cue.end_ms)
            .state("text", cue.text.clone())
            .field("start_ms", "integer", json!({ "min": 0, "max": MAX_CUE_MS, "unit": "ms" }))
            .field("end_ms", "integer", json!({ "min": 0, "max": MAX_CUE_MS, "unit": "ms" }))
            .field(
                "text",
                "text",
                json!({ "max_chars": TEXT_MAX_CHARS,
                    "accepts": "printable ASCII, Portuguese and Western European accents; a newline breaks a line" }),
            )
            .action("remove"),
        );
    }
}
