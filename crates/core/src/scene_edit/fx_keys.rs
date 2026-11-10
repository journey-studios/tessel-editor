//! The key operations of an effect clip (`bundle.fx`, F058, P3, BL082.2): the same atomic batch
//! as `fx.rs`, on the `keys` of a clip.
//!
//! Every op works on the copy of the document's JSON and leaves the judgement of the whole layer
//! (ranges, order, limits, overlap, the envelope) to the importer, exactly as a bundle from disk
//! is judged. The ops add only what makes their own refusal precise: the frame of a key that
//! leaves the clip, a move that crosses a neighbour, a key that is not there. Nothing here samples
//! a clock; a capture asks the same pure resolver (`resolve_keys`) the plan asks.
//!
//! Rules that are visible in the document:
//!
//! * a key is `{ "frame", "value", "easing"? }`; an easing nobody wrote stays unwritten, and an
//!   upsert that does not name one keeps the one the key already has;
//! * a curve with no key is no curve: removing the last key removes the curve, and the parameter
//!   goes back to its base value (`params`), which no key op ever touches;
//! * `keys` of a clip with no curve is left out of the document.

use serde_json::{Map, Value, json};

use crate::directed_scene::fx::{
    self, FxEasing, FxKey, KEY_OUT_OF_CLIP, KEYFRAME_ORDER, ParamSpec, UNKNOWN_KEYFRAME,
    UNKNOWN_KEYFRAME_EASING, UNKNOWN_PARAM, first_out_of_order, parse_param, resolve_keys,
};

use super::error::{EditError, NOT_APPLICABLE};
use super::fx::{FxChange, clip_object, clip_ref, mismatch, text_of, tracks_mut};

/// The warning a static `set_param` on an animated parameter carries: the curve was kept, the
/// base value changed, and the picture does not follow the base while the curve exists.
pub const STATIC_UNDER_KEYS: &str = "FX_STATIC_UNDER_KEYS";

/// The catalog parameter a key op targets, resolved before any key is touched.
fn target_of(clip: &Map<String, Value>, name: &str) -> Result<&'static ParamSpec, EditError> {
    let effect = clip
        .get("effect")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let spec = fx::effect(effect).ok_or_else(|| {
        EditError::new(
            fx::UNKNOWN_EFFECT,
            format!("clip names the effect `{effect}`"),
        )
    })?;
    let (_, param) = spec.param(name).ok_or_else(|| {
        EditError::new(
            UNKNOWN_PARAM,
            format!(
                "`{name}` is not a parameter of {}; known: {}",
                spec.id,
                spec.params
                    .iter()
                    .map(|param| param.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
        .with_path(name)
    })?;
    Ok(param)
}

fn duration_of(clip: &Map<String, Value>) -> i64 {
    clip.get("durationFrames")
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

fn frame_of(item: &Value) -> i64 {
    item.get("frame").and_then(Value::as_i64).unwrap_or(0)
}

/// The raw keys of a curve, or none.
fn curve_items(clip: &Map<String, Value>, param: &str) -> Vec<Value> {
    clip.get("keys")
        .and_then(|keys| keys.get(param))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// The typed keys of raw items the document already holds (every item went through the importer or
/// an earlier op of this batch, so a key that does not read back is a mismatch, not a user error).
fn typed(param: &ParamSpec, items: &[Value]) -> Result<Vec<FxKey>, EditError> {
    items
        .iter()
        .map(|item| {
            let value = item
                .get("value")
                .and_then(|raw| parse_param(param, raw).ok())
                .ok_or_else(|| mismatch("a key of the document has no readable value"))?;
            let easing = match item.get("easing").and_then(Value::as_str) {
                Some(name) => FxEasing::parse(name)
                    .ok_or_else(|| mismatch("a key of the document names an unknown easing"))?,
                None => FxEasing::Linear,
            };
            Ok(FxKey {
                frame: frame_of(item),
                value,
                easing,
            })
        })
        .collect()
}

/// Writes a curve back; an empty one removes the curve, and a clip without curves has no `keys`.
fn write_curve(clip: &mut Map<String, Value>, param: &str, items: Vec<Value>) {
    if items.is_empty() {
        let emptied = clip
            .get_mut("keys")
            .and_then(Value::as_object_mut)
            .map(|keys| {
                keys.remove(param);
                keys.is_empty()
            });
        if emptied == Some(true) {
            clip.remove("keys");
        }
        return;
    }
    let keys = clip
        .entry("keys")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(keys) = keys.as_object_mut() {
        keys.insert(param.to_string(), Value::Array(items));
    }
}

fn out_of_clip(frame: i64, duration: i64, param: &str) -> EditError {
    EditError::new(
        KEY_OUT_OF_CLIP,
        format!(
            "a key sits on a frame of its clip (0 to {}), not at {frame}",
            duration - 1
        ),
    )
    .with_path(format!("keys.{param}"))
}

fn easing_of(name: &str, param: &str) -> Result<FxEasing, EditError> {
    FxEasing::parse(name).ok_or_else(|| {
        EditError::new(
            UNKNOWN_KEYFRAME_EASING,
            format!(
                "`{name}` is not an easing; known: {}",
                fx::FX_EASINGS.join(", ")
            ),
        )
        .with_path(format!("keys.{param}"))
    })
}

fn no_key(frame: i64, param: &str) -> EditError {
    EditError::new(
        UNKNOWN_KEYFRAME,
        format!("`{param}` has no key at frame {frame}"),
    )
    .with_path(format!("keys.{param}"))
}

/// A curve whose frames, in this order, are not inside the clip and strictly increasing.
fn check_frames(frames: &[i64], duration: i64, param: &str) -> Result<(), EditError> {
    if let Some(frame) = frames
        .iter()
        .find(|frame| **frame < 0 || **frame >= duration)
    {
        return Err(out_of_clip(*frame, duration, param));
    }
    if let Some(at) = first_out_of_order(frames) {
        return Err(EditError::new(
            KEYFRAME_ORDER,
            format!(
                "keys of `{param}` stay in strictly increasing frames; frame {} does not come after {}",
                frames[at],
                frames[at.saturating_sub(1)]
            ),
        )
        .with_path(format!("keys.{param}")));
    }
    Ok(())
}

struct Located<'a> {
    track: String,
    clip: &'a mut Map<String, Value>,
}

fn locate<'a>(work: &'a mut Value, clip: &str) -> Result<Located<'a>, EditError> {
    let (track_slot, clip_slot) = clip_ref(work, clip)?;
    let tracks = tracks_mut(work)?;
    let track = text_of(&tracks[track_slot], "id").to_string();
    let clip = clip_object(&mut tracks[track_slot], clip_slot)?;
    Ok(Located { track, clip })
}

fn key_json(frame: i64, value: Value, easing: Option<Value>) -> Value {
    let mut key = Map::new();
    key.insert("frame".to_string(), json!(frame));
    key.insert("value".to_string(), value);
    if let Some(easing) = easing {
        key.insert("easing".to_string(), easing);
    }
    Value::Object(key)
}

// WHY allowed: it is the one place that spells an `FxChange`; a builder would repeat its fields.
#[allow(clippy::too_many_arguments)]
fn change(
    index: usize,
    action: &'static str,
    located: &Located<'_>,
    clip: &str,
    target: Option<String>,
    before: Option<Value>,
    after: Option<Value>,
    noop: bool,
) -> FxChange {
    FxChange {
        index,
        action,
        track: located.track.clone(),
        clip: clip.to_string(),
        target,
        before,
        after,
        noop,
        warnings: Vec::new(),
    }
}

/// `key_param`: puts a key on `frame`, or changes the one that is there. With no `value` the key
/// captures what the parameter is at that frame (the curve's own answer, or the base value when
/// there is no curve). An omitted easing is kept (a new key has none written).
pub(super) fn key_param(
    work: &mut Value,
    clip: &str,
    param: &str,
    frame: i64,
    value: Option<&Value>,
    easing: Option<&str>,
    index: usize,
) -> Result<FxChange, EditError> {
    let located = locate(work, clip)?;
    let target = target_of(located.clip, param)?;
    let duration = duration_of(located.clip);
    if frame < 0 || frame >= duration {
        return Err(out_of_clip(frame, duration, param));
    }
    let easing = easing.map(|name| easing_of(name, param)).transpose()?;
    let mut items = curve_items(located.clip, param);
    let value = match value {
        Some(raw) => parse_param(target, raw)
            .map_err(|(code, message)| EditError::new(code, message).with_path(param))?,
        None => {
            let keys = typed(target, &items)?;
            resolve_keys(&keys, frame).unwrap_or_else(|| {
                located
                    .clip
                    .get("params")
                    .and_then(|params| params.get(param))
                    .and_then(|raw| parse_param(target, raw).ok())
                    .unwrap_or_else(|| target.kind.default_value())
            })
        }
    };
    let slot = items.iter().position(|item| frame_of(item) == frame);
    let before = slot.map(|slot| items[slot].clone());
    let kept_easing = before.as_ref().and_then(|key| key.get("easing")).cloned();
    let written_easing = easing.map(|easing| json!(easing.name())).or(kept_easing);
    let after = key_json(frame, value.to_json(), written_easing);
    let noop = before.as_ref() == Some(&after);
    if !noop {
        match slot {
            Some(slot) => items[slot] = after.clone(),
            None => {
                let at = items.partition_point(|item| frame_of(item) < frame);
                items.insert(at, after.clone());
            }
        }
        write_curve(located.clip, param, items);
    }
    Ok(change(
        index,
        "key_param",
        &located,
        clip,
        Some(format!("{param}@{frame}")),
        before,
        Some(after),
        noop,
    ))
}

/// `set_key_easing`: the easing of the span that arrives at the key on `frame`.
pub(super) fn set_key_easing(
    work: &mut Value,
    clip: &str,
    param: &str,
    frame: i64,
    easing: &str,
    index: usize,
) -> Result<FxChange, EditError> {
    let located = locate(work, clip)?;
    target_of(located.clip, param)?;
    let easing = easing_of(easing, param)?;
    let mut items = curve_items(located.clip, param);
    let slot = items
        .iter()
        .position(|item| frame_of(item) == frame)
        .ok_or_else(|| no_key(frame, param))?;
    let before = items[slot].clone();
    let mut after = before.clone();
    after["easing"] = json!(easing.name());
    let noop = before == after;
    if !noop {
        items[slot] = after.clone();
        write_curve(located.clip, param, items);
    }
    Ok(change(
        index,
        "set_key_easing",
        &located,
        clip,
        Some(format!("{param}@{frame}")),
        Some(before),
        Some(after),
        noop,
    ))
}

/// `remove_key`: takes the key on `frame` away. The last key takes the curve with it.
pub(super) fn remove_key(
    work: &mut Value,
    clip: &str,
    param: &str,
    frame: i64,
    index: usize,
) -> Result<FxChange, EditError> {
    let located = locate(work, clip)?;
    target_of(located.clip, param)?;
    let mut items = curve_items(located.clip, param);
    let slot = items
        .iter()
        .position(|item| frame_of(item) == frame)
        .ok_or_else(|| no_key(frame, param))?;
    let removed = items.remove(slot);
    write_curve(located.clip, param, items);
    Ok(change(
        index,
        "remove_key",
        &located,
        clip,
        Some(format!("{param}@{frame}")),
        Some(removed),
        None,
        false,
    ))
}

/// `move_key`: moves the key on `from` to the frame `to`. It may not land on another key or pass
/// one: the order of the curve is the author's, not something an edit reshuffles.
pub(super) fn move_key(
    work: &mut Value,
    clip: &str,
    param: &str,
    from: i64,
    to: i64,
    index: usize,
) -> Result<FxChange, EditError> {
    let located = locate(work, clip)?;
    target_of(located.clip, param)?;
    let duration = duration_of(located.clip);
    let mut items = curve_items(located.clip, param);
    let slot = items
        .iter()
        .position(|item| frame_of(item) == from)
        .ok_or_else(|| no_key(from, param))?;
    if to < 0 || to >= duration {
        return Err(out_of_clip(to, duration, param));
    }
    let before = items[slot].clone();
    let noop = from == to;
    let mut after = before.clone();
    after["frame"] = json!(to);
    if !noop {
        let mut frames: Vec<i64> = items.iter().map(frame_of).collect();
        frames[slot] = to;
        check_frames(&frames, duration, param)?;
        items[slot] = after.clone();
        write_curve(located.clip, param, items);
    }
    Ok(change(
        index,
        "move_key",
        &located,
        clip,
        Some(format!("{param}@{from}")),
        Some(before),
        Some(after),
        noop,
    ))
}

/// The curves a bulk op works on: the named parameter's, or every curve of the clip.
fn curves_of(
    clip: &Map<String, Value>,
    param: Option<&str>,
) -> Result<Vec<(String, Vec<Value>)>, EditError> {
    match param {
        Some(name) => {
            target_of(clip, name)?;
            let items = curve_items(clip, name);
            if items.is_empty() {
                return Err(no_key_at_all(name));
            }
            Ok(vec![(name.to_string(), items)])
        }
        None => Ok(clip
            .get("keys")
            .and_then(Value::as_object)
            .map(|keys| {
                keys.iter()
                    .filter_map(|(name, items)| {
                        items.as_array().map(|items| (name.clone(), items.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default()),
    }
}

fn no_key_at_all(param: &str) -> EditError {
    EditError::new(UNKNOWN_KEYFRAME, format!("`{param}` has no keys"))
        .with_path(format!("keys.{param}"))
}

/// Applies `map` to the frame of every key of the chosen curves; all or nothing.
fn remap_frames(
    work: &mut Value,
    clip: &str,
    param: Option<&str>,
    action: &'static str,
    index: usize,
    unchanged: bool,
    map: impl Fn(i64) -> Option<i64>,
) -> Result<FxChange, EditError> {
    let located = locate(work, clip)?;
    let duration = duration_of(located.clip);
    let curves = curves_of(located.clip, param)?;
    let target = param.map(str::to_string);
    let mut before = Map::new();
    let mut after = Map::new();
    let mut rewritten = Vec::new();
    for (name, items) in curves {
        let mut frames = Vec::with_capacity(items.len());
        for item in &items {
            let moved = map(frame_of(item)).ok_or_else(|| {
                EditError::new(
                    KEY_OUT_OF_CLIP,
                    format!("a key of `{name}` leaves the range of whole frames"),
                )
                .with_path(format!("keys.{name}"))
            })?;
            frames.push(moved);
        }
        check_frames(&frames, duration, &name)?;
        let moved: Vec<Value> = items
            .iter()
            .zip(&frames)
            .map(|(item, frame)| {
                let mut item = item.clone();
                item["frame"] = json!(frame);
                item
            })
            .collect();
        before.insert(name.clone(), Value::Array(items));
        after.insert(name.clone(), Value::Array(moved.clone()));
        rewritten.push((name, moved));
    }
    let noop = unchanged || before == after;
    if !noop {
        for (name, items) in rewritten {
            write_curve(located.clip, &name, items);
        }
    }
    Ok(change(
        index,
        action,
        &located,
        clip,
        target,
        Some(Value::Object(before)),
        Some(Value::Object(after)),
        noop,
    ))
}

/// `shift_keys`: `delta` frames later (earlier when negative), all the keys of one curve or of the
/// whole clip.
pub(super) fn shift_keys(
    work: &mut Value,
    clip: &str,
    param: Option<&str>,
    delta: i64,
    index: usize,
) -> Result<FxChange, EditError> {
    remap_frames(
        work,
        clip,
        param,
        "shift_keys",
        index,
        delta == 0,
        |frame| frame.checked_add(delta),
    )
}

/// Round half up of `numerator / denominator` for a positive denominator.
fn round_div(numerator: i128, denominator: i128) -> i128 {
    (2 * numerator + denominator).div_euclid(2 * denominator)
}

/// `scale_keys`: the distance of every key from `anchor` times `num / den` (round half up, exact
/// in integers), so the same gesture gives the same frames on every machine.
pub(super) fn scale_keys(
    work: &mut Value,
    clip: &str,
    param: Option<&str>,
    anchor: i64,
    num: i64,
    den: i64,
    index: usize,
) -> Result<FxChange, EditError> {
    if num < 1 || den < 1 {
        return Err(EditError::new(
            super::error::INVALID_OP,
            "a scale is a ratio of two positive whole numbers",
        ));
    }
    remap_frames(
        work,
        clip,
        param,
        "scale_keys",
        index,
        num == den,
        |frame| {
            let distance = i128::from(frame) - i128::from(anchor);
            let moved = i128::from(anchor) + round_div(distance * i128::from(num), i128::from(den));
            i64::try_from(moved).ok()
        },
    )
}

/// `clear_keys`: the explicit way to remove an animation (one parameter's, or all of the clip's).
/// The base values are left as they are.
pub(super) fn clear_keys(
    work: &mut Value,
    clip: &str,
    param: Option<&str>,
    index: usize,
) -> Result<FxChange, EditError> {
    let located = locate(work, clip)?;
    let removed = match param {
        Some(name) => {
            target_of(located.clip, name)?;
            let items = curve_items(located.clip, name);
            if items.is_empty() {
                None
            } else {
                write_curve(located.clip, name, Vec::new());
                Some(json!({ name: items }))
            }
        }
        None => located.clip.remove("keys"),
    };
    let noop = removed.is_none();
    Ok(change(
        index,
        "clear_keys",
        &located,
        clip,
        param.map(str::to_string),
        removed,
        None,
        noop,
    ))
}

/// `set_seed`: the base seed of a clip's noise (`None` goes back to the one the clip's ID gives).
pub(super) fn set_seed(
    work: &mut Value,
    clip: &str,
    seed: Option<u32>,
    index: usize,
) -> Result<FxChange, EditError> {
    let located = locate(work, clip)?;
    let effect = located
        .clip
        .get("effect")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let spec = fx::effect(effect).ok_or_else(|| {
        EditError::new(
            fx::UNKNOWN_EFFECT,
            format!("clip names the effect `{effect}`"),
        )
    })?;
    if spec.reseed == fx::Reseed::Never {
        return Err(EditError::new(
            NOT_APPLICABLE,
            format!("{} draws no noise: it has no seed", spec.id),
        )
        .with_path("seed"));
    }
    let before = located.clip.get("seed").cloned();
    let after = seed.map(|seed| json!(seed));
    let noop = before == after;
    if !noop {
        match &after {
            Some(value) => {
                located.clip.insert("seed".to_string(), value.clone());
            }
            None => {
                located.clip.remove("seed");
            }
        }
    }
    Ok(change(
        index,
        "set_seed",
        &located,
        clip,
        Some("seed".to_string()),
        before,
        after,
        noop,
    ))
}

/// The warning of a static write under an animation, when the parameter has a curve.
pub(super) fn static_under_keys(
    clip_object: &Map<String, Value>,
    clip: &str,
    param: &str,
) -> Option<Value> {
    let keys = clip_object
        .get("keys")
        .and_then(|keys| keys.get(param))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    (keys > 0).then(|| {
        json!({
            "code": STATIC_UNDER_KEYS,
            "clip": clip, "param": param, "keys": keys,
            "message": format!(
                "`{param}` of `{clip}` is animated by {keys} key(s): the base value was set and the \
                 curve was kept, so the picture still follows the curve. Remove the animation with \
                 `clear_keys` to use the base value"
            ),
        })
    })
}
