//! The inspector nodes of a clip's keys (F073): per keyable property a button that keys the value
//! the clip shows at the playhead, one that removes the property's keys, and one node per key.
//!
//! Every node is a target of `set`/`invoke` (`clip_keys.capture:<property>`,
//! `clip_keys.clear:<property>`, `clip_key:<clip>:<property>:<index>`), so a script edits exactly
//! what the inspector shows.

use serde_json::json;

use super::Node;
use crate::clip_keys::{self, PROPERTIES};
use crate::easing::KeyEasing as Easing;
use crate::group::MAX_TIME_MS;
use crate::state::EditorState;
use crate::state::inspector::property_def;
use crate::state::timeline::Clip;

const PLAYHEAD_OUTSIDE: &str = "Move the playhead over the clip to key it";
const NO_KEYS: &str = "The clip has no keys";
const NO_KEYS_ON_PROPERTY: &str = "The clip has no keys on this property";

/// Appends the key nodes of the selected clip under the `keys` section node `parent`.
pub(super) fn build_clip_keys(
    state: &EditorState,
    clip: &Clip,
    parent: &str,
    shown: bool,
    nodes: &mut Vec<Node>,
) {
    let playhead = state.timeline.playhead;
    let inside = playhead >= clip.start && playhead < clip.end();
    for (id, prop) in PROPERTIES {
        let label = property_def(id).map_or(id, |def| def.label);
        let track = clip.keys.iter().find(|track| track.prop == prop);
        let (min, max, _, unit) = prop.limits();
        let mut capture = Node::new(
            format!("clip_keys.capture:{id}"),
            "button",
            format!("Add key at playhead · {label}"),
        )
        .parent(parent)
        .visible(shown)
        .state("clip", format!("clip:{}", clip.id))
        .state("property", id)
        .state("playhead", playhead)
        .state("clip_time_ms", state.clip_local_ms(clip, playhead).round())
        .state("value", state.clip_keyed_value(clip, prop, playhead))
        .state("keyed", track.is_some())
        .action("invoke");
        if !inside {
            capture = capture.disabled(PLAYHEAD_OUTSIDE);
        }
        nodes.push(capture);
        let mut clear = Node::new(
            format!("clip_keys.clear:{id}"),
            "button",
            format!("Remove keys · {label}"),
        )
        .parent(parent)
        .visible(shown)
        .state("clip", format!("clip:{}", clip.id))
        .state("property", id)
        .state("count", track.map_or(0, |track| track.keys.len()))
        .action("invoke");
        if track.is_none() {
            clear = clear.disabled(NO_KEYS_ON_PROPERTY);
        }
        nodes.push(clear);
        let Some(track) = track else {
            continue;
        };
        for (index, key) in track.keys.iter().enumerate() {
            nodes.push(
                Node::new(
                    format!("clip_key:{}:{id}:{index}", clip.id),
                    "clip-key",
                    format!("{label} key {} ms", key.time_ms),
                )
                .parent(parent)
                .visible(shown)
                .state("clip", format!("clip:{}", clip.id))
                .state("property", id)
                .state("index", index)
                .state("time_ms", key.time_ms)
                .state("value", key.value)
                .state("easing", key.easing.label())
                .field(
                    "time_ms",
                    "integer",
                    json!({ "min": 0, "max": MAX_TIME_MS, "unit": "ms of source time (from the clip start while it is untrimmed)" }),
                )
                .field("value", "number", json!({ "min": min, "max": max, "unit": unit }))
                .field("easing", "choice", json!({ "options": Easing::labels() }))
                .action("remove"),
            );
        }
    }
    let mut clear_all = Node::new("clip_keys.clear", "button", "Remove all keys")
        .parent(parent)
        .visible(shown)
        .state("clip", format!("clip:{}", clip.id))
        .state(
            "count",
            clip.keys
                .iter()
                .map(|track| track.keys.len())
                .sum::<usize>(),
        )
        .state("properties", clip_keys::property_ids())
        .action("invoke");
    if clip.keys.is_empty() {
        clear_all = clear_all.disabled(NO_KEYS);
    }
    nodes.push(clear_all);
}
