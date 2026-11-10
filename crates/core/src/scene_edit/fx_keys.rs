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
    self, FX_LIMIT, FxEasing, FxKey, KEY_OUT_OF_CLIP, KEYFRAME_ORDER, MAX_FX_KEYS_PER_CLIP,
    MAX_FX_KEYS_PER_CURVE, MAX_FX_KEYS_PER_LAYER, ParamSpec, UNKNOWN_KEYFRAME,
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

/// What a batch of fx ops cost, counted where the work happens so a test can assert a bound
/// instead of a stopwatch. `document_scans` are passes over every key of the layer (the quota is
/// read once per batch, never once per op); `curve_items_copied` are raw key items cloned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FxWork {
    /// Ops that started running (a refused op counts; the ones after it never start).
    pub ops_run: usize,
    pub document_scans: usize,
    pub curve_items_copied: usize,
}

/// The state a batch carries between its ops: the work done and the layer's key count, read
/// from the document once (the first op that grows a curve) and then kept up to date by the ops.
#[derive(Debug, Default)]
pub(super) struct Batch {
    pub(super) work: FxWork,
    layer_keys: Option<usize>,
}

impl Batch {
    /// The keys of every curve of the layer. One pass per batch, however many ops it holds.
    fn layer_total(&mut self, work: &Value) -> usize {
        if let Some(total) = self.layer_keys {
            return total;
        }
        self.work.document_scans += 1;
        let total = work
            .pointer("/fx/tracks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|track| track.get("clips").and_then(Value::as_array))
            .flatten()
            .map(|clip| clip_key_total(clip.as_object()))
            .sum();
        self.layer_keys = Some(total);
        total
    }

    /// The count `layer_total` read (0 before the first read).
    fn known_layer_total(&self) -> usize {
        self.layer_keys.unwrap_or(0)
    }

    fn grew(&mut self, by: usize) {
        if let Some(total) = &mut self.layer_keys {
            *total = total.saturating_add(by);
        }
    }

    pub(super) fn shrank(&mut self, by: usize) {
        if let Some(total) = &mut self.layer_keys {
            *total = total.saturating_sub(by);
        }
    }
}

/// The keys over all the curves of a clip object.
pub(super) fn clip_key_total(clip: Option<&Map<String, Value>>) -> usize {
    clip.and_then(|clip| clip.get("keys"))
        .and_then(Value::as_object)
        .map_or(0, |keys| {
            keys.values()
                .filter_map(Value::as_array)
                .map(Vec::len)
                .sum()
        })
}

/// The raw keys of a curve, borrowed.
fn curve_ref<'a>(clip: &'a Map<String, Value>, param: &str) -> &'a [Value] {
    clip.get("keys")
        .and_then(|keys| keys.get(param))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// The raw keys of a curve, copied (the ops that rewrite a whole curve work on the copy and write
/// it back only when the rewrite is legal). Every copy is counted.
fn curve_items(batch: &mut Batch, clip: &Map<String, Value>, param: &str) -> Vec<Value> {
    let items = curve_ref(clip, param).to_vec();
    batch.work.curve_items_copied += items.len();
    items
}

/// The array of a curve, to edit where it lies.
fn curve_mut<'a>(clip: &'a mut Map<String, Value>, param: &str) -> Option<&'a mut Vec<Value>> {
    clip.get_mut("keys")
        .and_then(Value::as_object_mut)
        .and_then(|keys| keys.get_mut(param))
        .and_then(Value::as_array_mut)
}

/// Puts `item` at `at` of the curve, making the `keys` object and the curve when there are none.
fn insert_key(clip: &mut Map<String, Value>, param: &str, at: usize, item: Value) {
    let keys = clip
        .entry("keys")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(keys) = keys.as_object_mut() {
        let curve = keys
            .entry(param.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(curve) = curve.as_array_mut() {
            curve.insert(at.min(curve.len()), item);
        }
    }
}

/// Takes the item at `slot` out of the curve; an emptied curve leaves the document (and so does an
/// emptied `keys`).
fn remove_item(clip: &mut Map<String, Value>, param: &str, slot: usize) -> Value {
    let removed = curve_mut(clip, param).map(|curve| curve.remove(slot));
    let emptied = curve_mut(clip, param).is_some_and(|curve| curve.is_empty());
    if emptied {
        write_curve(clip, param, Vec::new());
    }
    removed.unwrap_or(Value::Null)
}

fn limit_error(message: String, path: &str) -> EditError {
    EditError::new(FX_LIMIT, message).with_path(path)
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
///
/// Bounded work: the curve is edited where it lies (no copy), and a key that would be NEW is
/// refused against the curve, clip and layer limits **before** the curve grows, so a batch that
/// cannot fit stops at the op that overflows. Writing what the key already holds, by value and
/// easing (`1` over `1.0`), is a no-op that leaves the author's spelling alone and takes no room.
#[allow(clippy::too_many_arguments)]
pub(super) fn key_param(
    work: &mut Value,
    batch: &mut Batch,
    clip: &str,
    param: &str,
    frame: i64,
    value: Option<&Value>,
    easing: Option<&str>,
    index: usize,
) -> Result<FxChange, EditError> {
    // Read before the clip is borrowed: the first op of a batch that may grow a curve counts the layer.
    batch.layer_total(work);
    let located = locate(work, clip)?;
    let target = target_of(located.clip, param)?;
    let duration = duration_of(located.clip);
    if frame < 0 || frame >= duration {
        return Err(out_of_clip(frame, duration, param));
    }
    let easing = easing.map(|name| easing_of(name, param)).transpose()?;
    let value = match value {
        Some(raw) => parse_param(target, raw)
            .map_err(|(code, message)| EditError::new(code, message).with_path(param))?,
        None => {
            let keys = typed(target, curve_ref(located.clip, param))?;
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
    let curve = curve_ref(located.clip, param);
    let (len, at) = (
        curve.len(),
        curve.partition_point(|item| frame_of(item) < frame),
    );
    let slot = curve.iter().position(|item| frame_of(item) == frame);
    let before = slot.map(|slot| curve[slot].clone());
    // The typed value the key holds now, to compare by meaning and not by spelling.
    let held = before
        .as_ref()
        .and_then(|item| item.get("value"))
        .and_then(|raw| parse_param(target, raw).ok());
    let written = before
        .as_ref()
        .and_then(|item| item.get("easing"))
        .and_then(Value::as_str);
    let after_easing = easing.map(FxEasing::name).or(written);
    let same_value = held == Some(value);
    let same_easing = after_easing == written;
    let noop = before.is_some() && same_value && same_easing;
    let after = match &before {
        None => key_json(frame, value.to_json(), after_easing.map(|name| json!(name))),
        Some(item) => {
            let mut item = item.clone();
            if !same_value {
                item["value"] = value.to_json();
            }
            if !same_easing && let Some(name) = after_easing {
                item["easing"] = json!(name);
            }
            item
        }
    };
    if !noop {
        match slot {
            Some(slot) => {
                if let Some(item) = curve_mut(located.clip, param).and_then(|c| c.get_mut(slot)) {
                    *item = after.clone();
                }
            }
            None => {
                if len + 1 > MAX_FX_KEYS_PER_CURVE {
                    return Err(limit_error(
                        format!(
                            "`{param}` of `{clip}` holds {len} keys; a curve holds at most \
                             {MAX_FX_KEYS_PER_CURVE}"
                        ),
                        &format!("keys.{param}"),
                    ));
                }
                let in_clip = clip_key_total(Some(located.clip));
                if in_clip + 1 > MAX_FX_KEYS_PER_CLIP {
                    return Err(limit_error(
                        format!(
                            "`{clip}` holds {in_clip} keys; a clip holds at most \
                             {MAX_FX_KEYS_PER_CLIP} over all its curves"
                        ),
                        &format!("keys.{param}"),
                    ));
                }
                let in_layer = batch.known_layer_total();
                if in_layer + 1 > MAX_FX_KEYS_PER_LAYER {
                    return Err(limit_error(
                        format!(
                            "the effects layer holds {in_layer} keys; it holds at most \
                             {MAX_FX_KEYS_PER_LAYER}"
                        ),
                        &format!("keys.{param}"),
                    ));
                }
                insert_key(located.clip, param, at, after.clone());
                batch.grew(1);
            }
        }
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
    let curve = curve_ref(located.clip, param);
    let slot = curve
        .iter()
        .position(|item| frame_of(item) == frame)
        .ok_or_else(|| no_key(frame, param))?;
    let before = curve[slot].clone();
    let mut after = before.clone();
    after["easing"] = json!(easing.name());
    let noop = before == after;
    if !noop && let Some(item) = curve_mut(located.clip, param).and_then(|c| c.get_mut(slot)) {
        *item = after.clone();
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
    batch: &mut Batch,
    clip: &str,
    param: &str,
    frame: i64,
    index: usize,
) -> Result<FxChange, EditError> {
    let located = locate(work, clip)?;
    target_of(located.clip, param)?;
    let slot = curve_ref(located.clip, param)
        .iter()
        .position(|item| frame_of(item) == frame)
        .ok_or_else(|| no_key(frame, param))?;
    let removed = remove_item(located.clip, param, slot);
    batch.shrank(1);
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
    let curve = curve_ref(located.clip, param);
    let slot = curve
        .iter()
        .position(|item| frame_of(item) == from)
        .ok_or_else(|| no_key(from, param))?;
    if to < 0 || to >= duration {
        return Err(out_of_clip(to, duration, param));
    }
    let before = curve[slot].clone();
    let noop = from == to;
    let mut after = before.clone();
    after["frame"] = json!(to);
    if !noop {
        let mut frames: Vec<i64> = curve.iter().map(frame_of).collect();
        frames[slot] = to;
        check_frames(&frames, duration, param)?;
        if let Some(item) = curve_mut(located.clip, param).and_then(|c| c.get_mut(slot)) {
            *item = after.clone();
        }
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
    batch: &mut Batch,
    clip: &Map<String, Value>,
    param: Option<&str>,
) -> Result<Vec<(String, Vec<Value>)>, EditError> {
    match param {
        Some(name) => {
            target_of(clip, name)?;
            let items = curve_items(batch, clip, name);
            if items.is_empty() {
                return Err(no_key_at_all(name));
            }
            Ok(vec![(name.to_string(), items)])
        }
        None => {
            let curves: Vec<(String, Vec<Value>)> = clip
                .get("keys")
                .and_then(Value::as_object)
                .map(|keys| {
                    keys.iter()
                        .filter_map(|(name, items)| {
                            items.as_array().map(|items| (name.clone(), items.clone()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            batch.work.curve_items_copied +=
                curves.iter().map(|(_, items)| items.len()).sum::<usize>();
            Ok(curves)
        }
    }
}

fn no_key_at_all(param: &str) -> EditError {
    EditError::new(UNKNOWN_KEYFRAME, format!("`{param}` has no keys"))
        .with_path(format!("keys.{param}"))
}

/// What a remap is named and whether the gesture is already known to leave every key where it is.
struct Remap {
    action: &'static str,
    index: usize,
    unchanged: bool,
}

/// Applies `map` to the frame of every key of the chosen curves; all or nothing.
fn remap_frames(
    work: &mut Value,
    batch: &mut Batch,
    clip: &str,
    param: Option<&str>,
    remap: Remap,
    map: impl Fn(i64) -> Option<i64>,
) -> Result<FxChange, EditError> {
    let Remap {
        action,
        index,
        unchanged,
    } = remap;
    let located = locate(work, clip)?;
    let duration = duration_of(located.clip);
    let curves = curves_of(batch, located.clip, param)?;
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
    batch: &mut Batch,
    clip: &str,
    param: Option<&str>,
    delta: i64,
    index: usize,
) -> Result<FxChange, EditError> {
    remap_frames(
        work,
        batch,
        clip,
        param,
        Remap {
            action: "shift_keys",
            index,
            unchanged: delta == 0,
        },
        |frame| frame.checked_add(delta),
    )
}

/// Round half up (towards +infinity at the half) of `numerator / denominator` for a positive
/// denominator, from the Euclidean quotient and remainder: the numerator is never doubled, so the
/// largest products of the scale (`2^64 * 2^63`) stay inside `i128`, and a negative numerator
/// rounds the same way a positive one does (`-5 / 2` is `-2`, `5 / 2` is `3`).
fn round_div(numerator: i128, denominator: i128) -> i128 {
    let quotient = numerator.div_euclid(denominator);
    let remainder = numerator.rem_euclid(denominator);
    // The fraction is `remainder / denominator` in [0, 1); it is at least half when the two halves
    // of the denominator are `remainder` and what is left of it.
    if remainder >= denominator - remainder {
        quotient + 1
    } else {
        quotient
    }
}

/// The frame a key at `frame` moves to when its distance from `anchor` is scaled by `num / den`,
/// or `None` when that frame is not a whole frame of `i64`. Checked end to end: the largest
/// inputs (an `i64::MIN` anchor, a numerator of `i64::MAX`) are refused or answered exactly,
/// never wrapped or panicked on.
fn scaled_frame(frame: i64, anchor: i64, num: i64, den: i64) -> Option<i64> {
    let distance = i128::from(frame) - i128::from(anchor);
    let scaled = distance.checked_mul(i128::from(num))?;
    let moved = i128::from(anchor).checked_add(round_div(scaled, i128::from(den)))?;
    i64::try_from(moved).ok()
}

/// `scale_keys`: the distance of every key from `anchor` times `num / den` (round half up, exact
/// in integers), so the same gesture gives the same frames on every machine.
pub(super) fn scale_keys(
    work: &mut Value,
    batch: &mut Batch,
    clip: &str,
    param: Option<&str>,
    anchor: i64,
    (num, den): (i64, i64),
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
        batch,
        clip,
        param,
        Remap {
            action: "scale_keys",
            index,
            unchanged: num == den,
        },
        |frame| scaled_frame(frame, anchor, num, den),
    )
}

/// `clear_keys`: the explicit way to remove an animation (one parameter's, or all of the clip's).
/// The base values are left as they are.
pub(super) fn clear_keys(
    work: &mut Value,
    batch: &mut Batch,
    clip: &str,
    param: Option<&str>,
    index: usize,
) -> Result<FxChange, EditError> {
    let located = locate(work, clip)?;
    let removed = match param {
        Some(name) => {
            target_of(located.clip, name)?;
            let items = curve_items(batch, located.clip, name);
            if items.is_empty() {
                None
            } else {
                batch.shrank(items.len());
                write_curve(located.clip, name, Vec::new());
                Some(json!({ name: items }))
            }
        }
        None => {
            batch.shrank(clip_key_total(Some(located.clip)));
            located.clip.remove("keys")
        }
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

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::{round_div, scaled_frame};

    /// The formula this replaced: `2 * numerator + denominator`, which leaves `i128` as soon as
    /// the numerator is within a factor of two of it.
    fn first_formula(numerator: i128, denominator: i128) -> Option<i128> {
        2i128
            .checked_mul(numerator)?
            .checked_add(denominator)
            .map(|sum| sum.div_euclid(2 * denominator))
    }

    #[test]
    fn the_first_formula_left_i128_where_a_scale_from_the_lowest_anchor_lands() {
        // A key at frame 5, anchored at i64::MIN, scaled by i64::MAX: the numerator is ~2^126
        // plus a little, so doubling it leaves i128 (a key at frame 0 would just fit).
        let distance = i128::from(5i64) - i128::from(i64::MIN);
        let numerator = distance * i128::from(i64::MAX);
        assert_eq!(
            first_formula(numerator, 1),
            None,
            "the probe: the old arithmetic overflows here"
        );
        // The exact one answers (and the result is then judged by the range of an i64).
        assert_eq!(round_div(numerator, 1), numerator);
        assert_eq!(
            scaled_frame(5, i64::MIN, i64::MAX, 1),
            None,
            "typed, not a wrap"
        );
    }

    #[test]
    fn round_div_is_round_half_up_for_every_sign_and_never_overflows() {
        // (numerator, denominator) -> floor(numerator / denominator + 1/2)
        for (numerator, denominator, expected) in [
            (1, 2, 1),
            (3, 2, 2),
            (-1, 2, 0),
            (-3, 2, -1),
            (-5, 2, -2),
            (5, 3, 2),
            (-5, 3, -2),
            (4, 3, 1),
            (-4, 3, -1),
            (0, 7, 0),
            (7, 7, 1),
            (-7, 7, -1),
        ] {
            assert_eq!(
                round_div(numerator, denominator),
                expected,
                "{numerator}/{denominator}"
            );
        }
        assert_eq!(round_div(i128::MAX, 1), i128::MAX);
        assert_eq!(round_div(i128::MIN, 1), i128::MIN);
        assert_eq!(round_div(i128::MAX, 2), i128::MAX / 2 + 1);
        assert_eq!(round_div(i128::MIN, i128::MAX), -1);
    }

    #[test]
    fn a_scaled_frame_that_leaves_i64_is_none_and_one_that_stays_is_exact() {
        assert_eq!(scaled_frame(3, 0, 1, 2), Some(2));
        assert_eq!(scaled_frame(1, 4, 1, 2), Some(3), "-3/2 rounds up to -1");
        assert_eq!(scaled_frame(3, 4, 1, 2), Some(4), "-1/2 rounds up to 0");
        assert_eq!(scaled_frame(i64::MAX, 0, 2, 1), None);
        assert_eq!(scaled_frame(i64::MIN, 0, 2, 1), None);
        assert_eq!(scaled_frame(i64::MAX, i64::MIN, 1, 1), Some(i64::MAX));
        assert_eq!(scaled_frame(5, i64::MIN, 1, 1), Some(5));
    }
}
