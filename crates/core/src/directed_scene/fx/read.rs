//! Reading `bundle.fx` at import: shape, vocabulary and the rules of the layer (the pilot's
//! `explainFx`, limited to what is implemented), reporting every problem with its JSON Pointer.
//!
//! The import is the single door: an edit (`scene_edit::apply_fx`) builds a candidate document
//! and re-imports it, so the same rules judge a bundle from disk and an edited one.

use std::collections::HashSet;

use serde_json::Value;

use crate::directed_scene::issue::{INVALID_TYPE, INVALID_VALUE, LIMIT_EXCEEDED};
use crate::directed_scene::read::{Bound, Obj, Sink, at, child, kind_of};

use super::catalog::{EffectSpec, ParamKind, ParamSpec, effect};
use super::{
    CATALOG_TOO_NEW, DUPLICATE_ID, ENVELOPE_TOO_LONG, FX_CATALOG_VERSION, FX_CLIP_EMPTY,
    FX_CLIP_OUT_OF_RANGE, FX_CLIP_OVERLAP, FX_KEYS_NOT_YET, FX_LIMIT, FX_SCOPE_NOT_YET,
    FX_TRANSITION_NOT_YET, Fx, FxClip, FxTrack, FxValue, MAX_FX_CLIPS_PER_TRACK, MAX_FX_TRACKS,
    PARAM_OUT_OF_RANGE, PARAM_TYPE, UNKNOWN_EFFECT, UNKNOWN_PARAM, id_problem,
};

/// Why a parameter value is refused: a code and a message naming what was expected.
pub type ParamProblem = (&'static str, String);

/// A number in JSON as `f64`, finite.
fn finite(value: &Value) -> Option<f64> {
    value.as_f64().filter(|number| number.is_finite())
}

fn in_range(label: &str, value: f64, min: f64, max: f64) -> Result<f64, ParamProblem> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err((
            PARAM_OUT_OF_RANGE,
            format!("{label} is {value}, outside {min}..={max}"),
        ))
    }
}

fn strict_object<'a>(
    value: &'a Value,
    label: &str,
    keys: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, ParamProblem> {
    let wrong = || {
        (
            PARAM_TYPE,
            format!(
                "{label} must be an object {{{}}} with numbers, got {}",
                keys.join(", "),
                kind_of(value)
            ),
        )
    };
    let map = value.as_object().ok_or_else(wrong)?;
    if map.len() != keys.len() || keys.iter().any(|key| !map.contains_key(*key)) {
        return Err(wrong());
    }
    Ok(map)
}

/// Checks a JSON value against a catalog parameter: its type first (`PARAM_TYPE`), then its range
/// (`PARAM_OUT_OF_RANGE`). The one rule behind import and `scene.fx.set_param`.
pub fn parse_param(spec: &ParamSpec, value: &Value) -> Result<FxValue, ParamProblem> {
    match spec.kind {
        ParamKind::Number { min, max, .. } => {
            let number = finite(value).ok_or_else(|| {
                (
                    PARAM_TYPE,
                    format!("`{}` must be a number, got {}", spec.id, kind_of(value)),
                )
            })?;
            in_range(&format!("`{}`", spec.id), number, min, max).map(FxValue::Number)
        }
        ParamKind::Color { .. } => {
            let map = strict_object(value, &format!("`{}`", spec.id), &["r", "g", "b"])?;
            let channel = |key: &str| {
                let number = finite(&map[key])
                    .ok_or_else(|| (PARAM_TYPE, format!("`{}.{key}` must be a number", spec.id)))?;
                in_range(&format!("`{}.{key}`", spec.id), number, 0.0, 1.0)
            };
            Ok(FxValue::Color {
                r: channel("r")?,
                g: channel("g")?,
                b: channel("b")?,
            })
        }
        ParamKind::Point { min, max, .. } => {
            let map = strict_object(value, &format!("`{}`", spec.id), &["x", "y"])?;
            let axis = |key: &str| {
                let number = finite(&map[key])
                    .ok_or_else(|| (PARAM_TYPE, format!("`{}.{key}` must be a number", spec.id)))?;
                in_range(&format!("`{}.{key}`", spec.id), number, min, max)
            };
            Ok(FxValue::Point {
                x: axis("x")?,
                y: axis("y")?,
            })
        }
    }
}

/// The `params` of a clip: the catalog defaults with each present key checked.
fn read_params(
    spec: &'static EffectSpec,
    raw: Option<&Value>,
    path: &str,
    sink: &Sink,
) -> Option<Vec<FxValue>> {
    let mut values: Vec<FxValue> = spec
        .params
        .iter()
        .map(|param| param.kind.default_value())
        .collect();
    let Some(raw) = raw else {
        return Some(values);
    };
    let Some(map) = raw.as_object() else {
        sink.issue(
            INVALID_TYPE,
            path,
            format!("expected object, got {}", kind_of(raw)),
        );
        return None;
    };
    let mut clean = true;
    for (key, value) in map {
        let at = child(path, key);
        let Some((index, param)) = spec.param(key) else {
            sink.issue(
                UNKNOWN_PARAM,
                &at,
                format!(
                    "`{key}` is not a parameter of {}; known: {}",
                    spec.id,
                    spec.params
                        .iter()
                        .map(|param| param.id)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
            clean = false;
            continue;
        };
        match parse_param(param, value) {
            Ok(parsed) => values[index] = parsed,
            Err((code, message)) => {
                sink.issue(code, &at, message);
                clean = false;
            }
        }
    }
    clean.then_some(values)
}

fn read_scope(raw: Option<&Value>, path: &str, sink: &Sink) -> bool {
    let Some(raw) = raw else {
        return true;
    };
    let kind = raw
        .as_object()
        .and_then(|map| map.get("kind"))
        .and_then(Value::as_str);
    match kind {
        Some("output") => true,
        Some(other @ ("shot" | "node")) => {
            sink.issue(
                FX_SCOPE_NOT_YET,
                path,
                format!(
                    "scope `{other}` is not drawn yet; use scope output (or leave `scope` out)"
                ),
            );
            false
        }
        _ => {
            sink.issue(
                INVALID_VALUE,
                path,
                "expected an object whose `kind` is `output`",
            );
            false
        }
    }
}

fn read_keys(raw: Option<&Value>, path: &str, sink: &Sink) -> bool {
    match raw {
        None => true,
        Some(Value::Object(map)) if map.is_empty() => true,
        Some(Value::Object(_)) => {
            sink.issue(
                FX_KEYS_NOT_YET,
                path,
                "parameter keyframes are not interpreted yet; give fixed `params`",
            );
            false
        }
        Some(other) => {
            sink.issue(
                INVALID_TYPE,
                path,
                format!("expected object, got {}", kind_of(other)),
            );
            false
        }
    }
}

fn read_clip(obj: &Obj<'_>, sink: &Sink) -> Option<FxClip> {
    let id = obj.id("id");
    if let Some(id) = id
        && let Some((code, message)) = id_problem(id)
    {
        sink.issue(code, &obj.field_path("id"), message);
    }
    let name = obj.id("effect");
    let spec = name.and_then(|name| {
        let found = effect(name);
        if found.is_none() {
            sink.issue(
                UNKNOWN_EFFECT,
                &obj.field_path("effect"),
                format!(
                    "`{name}` is not an effect of catalog v{FX_CATALOG_VERSION}; known: {}",
                    super::catalog()
                        .iter()
                        .map(|spec| spec.id)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
        found
    });
    let start = obj.int("startFrame", Bound::Any);
    let duration = obj.int("durationFrames", Bound::Any);
    let params = obj.deferred("params");
    let params_path = obj.field_path("params");
    let keys_ok = read_keys(obj.deferred("keys"), &obj.field_path("keys"), sink);
    let scope_ok = read_scope(obj.deferred("scope"), &obj.field_path("scope"), sink);
    let seed = obj.opt_int("seed", Bound::NonNegative);
    if let Some(Some(seed)) = seed
        && seed > i64::from(u32::MAX)
    {
        obj.sink_issue(
            INVALID_VALUE,
            "seed",
            "expected a seed from 0 to 4294967295",
        );
    }
    let fade_in = obj.opt_int("fadeInFrames", Bound::NonNegative);
    let fade_out = obj.opt_int("fadeOutFrames", Bound::NonNegative);
    let hidden = obj.opt_true("hidden");

    let values = spec.and_then(|spec| read_params(spec, params, &params_path, sink));
    let seed = seed?.and_then(|seed| u32::try_from(seed).ok());
    let (id, spec, start, duration, values) = (id?, spec?, start?, duration?, values?);
    if !(keys_ok && scope_ok) || id_problem(id).is_some() {
        return None;
    }
    Some(FxClip {
        id: id.to_string(),
        effect: spec,
        start_frame: start,
        duration_frames: duration,
        params: values,
        fade_in_frames: fade_in?.unwrap_or(0),
        fade_out_frames: fade_out?.unwrap_or(0),
        seed,
        hidden: hidden?,
    })
}

fn read_track(obj: &Obj<'_>, sink: &Sink) -> Option<FxTrack> {
    let id = obj.id("id");
    if let Some(id) = id
        && let Some((code, message)) = id_problem(id)
    {
        sink.issue(code, &obj.field_path("id"), message);
    }
    let label = obj.text("label");
    if let Some(label) = label
        && (label.is_empty() || label.chars().count() > 80)
    {
        obj.sink_issue(INVALID_VALUE, "label", "a label has 1 to 80 characters");
    }
    let clips = obj.array("clips", 0);
    if let Some(clips) = &clips
        && clips.len() > MAX_FX_CLIPS_PER_TRACK
    {
        sink.issue(
            FX_LIMIT,
            &obj.field_path("clips"),
            format!("a track holds at most {MAX_FX_CLIPS_PER_TRACK} clips"),
        );
    }
    let clips = clips.and_then(|array| array.objects(|_, clip| read_clip(clip, sink)));
    let hidden = obj.opt_true("hidden");
    let (id, label, clips, hidden) = (id?, label?, clips?, hidden?);
    if id_problem(id).is_some() || label.is_empty() || label.chars().count() > 80 {
        return None;
    }
    Some(FxTrack {
        id: id.to_string(),
        label: label.to_string(),
        hidden,
        clips,
    })
}

/// The rules that need clips next to each other or the length of the edit.
fn check_layer(fx: &Fx, path: &str, edit_duration: Option<u64>, sink: &Sink) -> bool {
    let before = sink_len(sink);
    let mut seen: HashSet<&str> = HashSet::new();
    let tracks_path = child(path, "tracks");
    for (track_index, track) in fx.tracks.iter().enumerate() {
        let track_path = at(&tracks_path, track_index);
        if !seen.insert(track.id.as_str()) {
            sink.issue(
                DUPLICATE_ID,
                &child(&track_path, "id"),
                format!("Duplicate fx id {}", track.id),
            );
        }
        let clips_path = child(&track_path, "clips");
        for (clip_index, clip) in track.clips.iter().enumerate() {
            let clip_path = at(&clips_path, clip_index);
            if !seen.insert(clip.id.as_str()) {
                sink.issue(
                    DUPLICATE_ID,
                    &child(&clip_path, "id"),
                    format!("Duplicate fx id {}", clip.id),
                );
            }
            let end = clip.end_frame_exclusive();
            if clip.duration_frames < 1 {
                sink.issue(
                    FX_CLIP_EMPTY,
                    &child(&clip_path, "durationFrames"),
                    format!(
                        "Clip {} lasts {} frames; a clip lasts at least 1 frame (remove it if it is not wanted)",
                        clip.id, clip.duration_frames
                    ),
                );
            }
            if let Some(length) = edit_duration
                && (clip.start_frame < 0 || end > i64::try_from(length).unwrap_or(i64::MAX))
            {
                sink.issue(
                    FX_CLIP_OUT_OF_RANGE,
                    &clip_path,
                    format!(
                        "Clip {} (frames {} to {end}) leaves the edit (0 to {length}); keep it inside",
                        clip.id, clip.start_frame
                    ),
                );
            }
            let fades = clip.fade_in_frames.saturating_add(clip.fade_out_frames);
            if fades > clip.duration_frames {
                sink.issue(
                    ENVELOPE_TOO_LONG,
                    &clip_path,
                    format!(
                        "Clip {}: fade in and out take {fades} frames of its {}; fadeInFrames + fadeOutFrames must not pass the duration",
                        clip.id, clip.duration_frames
                    ),
                );
            }
        }
        // Overlap by interval, whatever the order the clips are written in: one track plays
        // one effect at a time, so a clip that starts before the furthest end so far overlaps.
        let mut order: Vec<usize> = (0..track.clips.len()).collect();
        order.sort_by_key(|&index| (track.clips[index].start_frame, index));
        let mut furthest: Option<usize> = None;
        for index in order {
            let clip = &track.clips[index];
            if let Some(other) = furthest {
                let other_clip = &track.clips[other];
                if clip.start_frame < other_clip.end_frame_exclusive() && clip.duration_frames >= 1
                {
                    sink.issue(
                        FX_CLIP_OVERLAP,
                        &at(&clips_path, index),
                        format!(
                            "{} ({}) and {} ({}) are on the same track \"{}\" ({}) at the same time: {} runs to frame {} and {} starts at frame {}; move one to another track, start {} at frame {} or later, or shorten {}",
                            other_clip.effect.id,
                            other_clip.id,
                            clip.effect.id,
                            clip.id,
                            track.label,
                            track.id,
                            other_clip.id,
                            other_clip.end_frame_exclusive(),
                            clip.id,
                            clip.start_frame,
                            clip.id,
                            other_clip.end_frame_exclusive(),
                            other_clip.id
                        ),
                    );
                }
            }
            if furthest.is_none_or(|other| {
                clip.end_frame_exclusive() > track.clips[other].end_frame_exclusive()
            }) && clip.duration_frames >= 1
            {
                furthest = Some(index);
            }
        }
    }
    sink_len(sink) == before
}

fn sink_len(sink: &Sink) -> usize {
    sink.issue_count()
}

/// Reads the `fx` object of a bundle. `None` when something was reported.
pub(in crate::directed_scene) fn read_fx(
    obj: &Obj<'_>,
    edit_duration: Option<u64>,
    sink: &Sink,
) -> Option<Fx> {
    let version = obj.int("catalogVersion", Bound::Positive);
    if let Some(version) = version
        && version > i64::from(FX_CATALOG_VERSION)
    {
        obj.sink_issue(
            CATALOG_TOO_NEW,
            "catalogVersion",
            format!(
                "This editor draws catalog v{FX_CATALOG_VERSION}; the bundle was written against v{version}"
            ),
        );
    }
    let tracks = obj.array("tracks", 0);
    if let Some(tracks) = &tracks
        && tracks.len() > MAX_FX_TRACKS
    {
        sink.issue(
            LIMIT_EXCEEDED,
            &obj.field_path("tracks"),
            format!("a bundle holds at most {MAX_FX_TRACKS} fx tracks"),
        );
    }
    let tracks = tracks.and_then(|array| array.objects(|_, track| read_track(track, sink)));
    let transitions = obj.array("transitions", 0);
    if let Some(transitions) = &transitions
        && transitions.len() > 0
    {
        sink.issue(
            FX_TRANSITION_NOT_YET,
            &obj.field_path("transitions"),
            "transitions between shots are not implemented yet; leave `transitions` empty",
        );
    }
    let (version, tracks, transitions) = (version?, tracks?, transitions?);
    if version > i64::from(FX_CATALOG_VERSION) || transitions.len() > 0 {
        return None;
    }
    if tracks.len() > MAX_FX_TRACKS {
        return None;
    }
    let fx = Fx { tracks };
    check_layer(&fx, obj.path(), edit_duration, sink).then_some(fx)
}
