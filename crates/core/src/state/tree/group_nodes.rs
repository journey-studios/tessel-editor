//! The inspector nodes of a group clip (F069): the group it shows, its children, their key
//! tracks and keys, the texts the clip changes and the project's text styles.
//!
//! Every node is a target of `set`/`invoke` (`group_child:<group>/<child>`, `group_key:<group>/
//! <child>/<prop>/<index>`, ...), so a script edits exactly what the inspector shows.

use serde_json::json;

use super::{Node, with_shape_fields};
use crate::content::{TitleAlignment, TitleFont};
use crate::easing::KeyEasing as Easing;
use crate::group::{
    Child, ChildContent, GroupDef, MAX_NAME_CHARS, MAX_REPEAT, MAX_STAGGER_MS, MAX_STEP_MS,
    MAX_TIME_MS, MIN_STEP_MS, Prop, RepeatMode, RevealMode, TextStyle,
};
use crate::state::EditorState;
use crate::state::timeline::Clip;

/// Appends the nodes of the selected group clip under the `group` section node `parent`.
pub(super) fn build_group(
    state: &EditorState,
    clip: &Clip,
    parent: &str,
    shown: bool,
    nodes: &mut Vec<Node>,
) {
    let Some(instance) = &clip.group else {
        return;
    };
    let book = &state.groups;
    let defs: Vec<&str> = book.defs.iter().map(|def| def.id.as_str()).collect();
    nodes.push(
        Node::new(
            format!("clip_group:{}", clip.id),
            "group-input",
            "Grupo do clipe",
        )
        .parent(parent)
        .visible(shown)
        .state("clip", format!("clip:{}", clip.id))
        .state("group", instance.def.as_str())
        .state("options", defs.clone())
        .field("group", "choice", json!({ "options": defs })),
    );
    if let Some(def) = book.def(instance.def.as_str()) {
        build_def(state, clip, def, parent, shown, nodes);
    }
    for style in &book.styles {
        nodes.push(style_node(
            style,
            book.style_in_use(style.id.as_str()),
            parent,
            shown,
        ));
    }
}

fn build_def(
    state: &EditorState,
    clip: &Clip,
    def: &GroupDef,
    parent: &str,
    shown: bool,
    nodes: &mut Vec<Node>,
) {
    let children: Vec<&str> = def.children.iter().map(|child| child.id.as_str()).collect();
    nodes.push(
        Node::new(format!("group_def:{}", def.id), "group", def.name.clone())
            .parent(parent)
            .visible(shown)
            .state("name", def.name.clone())
            .state("stagger_ms", def.stagger_ms)
            .state("children", children)
            .state(
                "used_by",
                state
                    .clips_using_group(def.id.as_str())
                    .iter()
                    .map(|clip| clip.to_string())
                    .collect::<Vec<_>>(),
            )
            .field("name", "text", json!({ "max_chars": MAX_NAME_CHARS }))
            .field(
                "stagger_ms",
                "integer",
                json!({ "min": 0, "max": MAX_STAGGER_MS, "unit": "ms" }),
            )
            .action("remove"),
    );
    let styles: Vec<&str> = state
        .groups
        .styles
        .iter()
        .map(|style| style.id.as_str())
        .collect();
    for child in &def.children {
        nodes.push(child_node(def, child, &styles, parent, shown));
        for track in &child.tracks {
            let track_id = format!("{}/{}/{}", def.id, child.id, track.prop.label());
            nodes.push(
                Node::new(
                    format!("group_track:{track_id}"),
                    "group-track",
                    format!("{} · {}", child.id, track.prop.label()),
                )
                .parent(parent)
                .visible(shown)
                .state("repeat", track.repeat)
                .state("mode", track.mode.label())
                .state("keys", track.keys.len())
                .field("repeat", "integer", json!({ "min": 1, "max": MAX_REPEAT }))
                .field("mode", "choice", json!({ "options": RepeatMode::OPTIONS }))
                .action("remove"),
            );
            for (index, key) in track.keys.iter().enumerate() {
                nodes.push(
                    Node::new(
                        format!("group_key:{track_id}/{index}"),
                        "group-key",
                        format!("{} {} ms", track.prop.label(), key.time_ms),
                    )
                    .parent(parent)
                    .visible(shown)
                    .state("index", index)
                    .state("time_ms", key.time_ms)
                    .state("value", key.value)
                    .state("easing", key.easing.label())
                    .field(
                        "time_ms",
                        "integer",
                        json!({ "min": 0, "max": crate::group::MAX_TIME_MS, "unit": "ms" }),
                    )
                    .field("value", "number", prop_range(track.prop))
                    .field("easing", "choice", json!({ "options": Easing::labels() }))
                    .action("remove"),
                );
            }
        }
        if let ChildContent::Text { content, .. } = &child.content {
            let overridden = clip
                .group
                .as_ref()
                .and_then(|instance| instance.texts.get(&child.id));
            nodes.push(
                Node::new(
                    format!("group_text:{}/{}", clip.id, child.id),
                    "text-input",
                    format!("Texto · {}", child.id),
                )
                .parent(parent)
                .visible(shown)
                .state("clip", format!("clip:{}", clip.id))
                .state("child", child.id.as_str())
                .state("text", overridden.unwrap_or(content).clone())
                .state("overridden", overridden.is_some())
                .field(
                    "text",
                    "text",
                    json!({ "max_chars": crate::content::TEXT_MAX_CHARS, "single_line": true }),
                )
                .action("reset"),
            );
        }
    }
}

fn prop_range(prop: Prop) -> serde_json::Value {
    let (min, max, exclusive, unit) = prop.limits();
    json!({ "min": min, "max": max, "min_exclusive": exclusive, "unit": unit })
}

fn child_node(def: &GroupDef, child: &Child, styles: &[&str], parent: &str, shown: bool) -> Node {
    let mut node = Node::new(
        format!("group_child:{}/{}", def.id, child.id),
        "group-child",
        child.id.to_string(),
    )
    .parent(parent)
    .visible(shown)
    // `kind` is also a field of a shape child (rect, ellipse, ...): the child's own is `child_kind`.
    .state("child_kind", child.kind_label());
    for field in Child::BASE_FIELDS {
        node = node.state(field, child.field_json(field));
        let prop = Prop::parse(field).unwrap_or(Prop::X);
        node = node.field(field, "number", prop_range(prop));
    }
    match &child.content {
        ChildContent::Shape(_) => {
            let spec = child.field_json("kind");
            node = node.state("shape_kind", spec);
            for field in crate::shape::ShapeSpec::FIELDS {
                node = node.state(field, child.field_json(field));
            }
            node = with_shape_fields(node);
        }
        ChildContent::Text { .. } => {
            node = node
                .state("content", child.field_json("content"))
                .state("style", child.field_json("style"))
                .field(
                    "content",
                    "text",
                    json!({ "max_chars": crate::content::TEXT_MAX_CHARS, "single_line": true }),
                )
                .field("style", "choice", json!({ "options": styles }));
        }
    }
    // When the child starts (F070): after the stagger, or after an earlier child ends, plus a
    // delay. `after` offers only the children before this one, which is what the model accepts.
    let earlier: Vec<String> = def
        .children
        .iter()
        .take_while(|other| other.id != child.id)
        .map(|other| other.id.to_string())
        .collect();
    node = node
        .state("after", child.field_json("after"))
        .state("delay_ms", child.field_json("delay_ms"))
        .field(
            "after",
            "choice",
            json!({ "options": earlier, "allow_empty": true }),
        )
        .field(
            "delay_ms",
            "integer",
            json!({ "min": 0, "max": MAX_TIME_MS, "unit": "ms" }),
        );
    if child.is_text() {
        node = node
            .state("reveal", child.field_json("reveal"))
            .state("step_ms", child.field_json("step_ms"))
            .field("reveal", "choice", json!({ "options": RevealMode::OPTIONS }))
            .field(
                "step_ms",
                "integer",
                json!({ "min": MIN_STEP_MS, "max": MAX_STEP_MS, "unit": "ms per character or line" }),
            );
    }
    node.action("remove")
}

fn style_node(style: &TextStyle, in_use: bool, parent: &str, shown: bool) -> Node {
    Node::new(
        format!("text_style:{}", style.id),
        "text-style",
        format!("Estilo · {}", style.id),
    )
    .parent(parent)
    .visible(shown)
    .state("size", style.size)
    .state("color", style.color.clone())
    .state("font", style.font.as_str())
    .state("align", style.align.as_str())
    .state("width", style.width)
    .state("in_use", in_use)
    .field(
        "size",
        "number",
        json!({ "min": 1, "max": 100, "unit": "% of the canvas height" }),
    )
    .field("color", "text", json!({ "format": "#rrggbb" }))
    .field(
        "width",
        "number",
        json!({ "min": 0, "max": 100, "unit": "% of the canvas width (0 = automatic)" }),
    )
    .field("font", "choice", json!({ "options": TitleFont::OPTIONS }))
    .field(
        "align",
        "choice",
        json!({ "options": TitleAlignment::OPTIONS }),
    )
    .action("remove")
}
