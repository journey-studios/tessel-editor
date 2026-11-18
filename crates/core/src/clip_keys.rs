//! Keyframes of what a clip shows (F073): position, zoom, rotation and opacity over the clip's own
//! time, for any clip drawn from one picture (an image, a GIF, a video, a title, a shape, a scene).
//!
//! It is the group key model (`group::KeyTrack`: integer milliseconds, a value, the easing of the
//! segment that leaves the key, optional repeats) pointed at the inspector properties instead of
//! at a group child, so an image or a video is animated by the same timeline, inspector and CLI as
//! everything else, with no editor of its own.
//!
//! Rules:
//! - Key times are milliseconds on the clip's media clock, the *source time* (`source_start` plus
//!   the elapsed time at the clip's speed, exactly what a scene's camera keys read). For a clip
//!   that was never trimmed that is the time from its first frame. WHY: a key then stays on the
//!   picture it was set on when the clip is moved, trimmed at its head or split (the right half's
//!   `source_start` is the source time at the cut), with no rewriting of keys. A looping clip
//!   repeats its keys with the media.
//! - A property with a track takes the track's value instead of the inspector value; a property
//!   without one keeps the inspector value. Opacity is then still multiplied by the fades, and a
//!   zoom track replaces Ken Burns (the drift is the zoom's own slow key pair).
//! - Values are the ones the inspector shows (`position_x` in percent, `zoom` in percent,
//!   `rotation` in degrees, `opacity` in percent). The track limits are the group's (wider than
//!   the sliders, so a spin of several turns or an off-screen start is allowed).
//! - Evaluated from the sequence clock like everything else, so the monitor, `render.frame` and
//!   the export agree and a seek in any order gives the same picture.

use serde_json::{Value, json};

use crate::easing::KeyEasing as Easing;
use crate::group::{Key, KeyTrack, MAX_KEYS, Prop};
use crate::state::{CommandError, Raw};

/// Inspector properties that can be keyed, in the order the inspector lists them, with the group
/// property whose track model and limits they share.
pub const PROPERTIES: [(&str, Prop); 5] = [
    ("position_x", Prop::X),
    ("position_y", Prop::Y),
    ("zoom", Prop::Scale),
    ("rotation", Prop::Rotation),
    ("opacity", Prop::Opacity),
];

/// Names a request or a node target may use for the keyable properties.
pub fn property_ids() -> Vec<&'static str> {
    PROPERTIES.iter().map(|(id, _)| *id).collect()
}

/// The inspector property id of a keyed property.
pub fn property_id(prop: Prop) -> &'static str {
    PROPERTIES
        .iter()
        .find(|(_, candidate)| *candidate == prop)
        .map_or("", |(id, _)| id)
}

/// The keyed property an inspector property id names.
pub fn parse_property(raw: &str) -> Result<Prop, CommandError> {
    PROPERTIES
        .iter()
        .find(|(id, _)| *id == raw.trim())
        .map(|(_, prop)| *prop)
        .ok_or_else(|| {
            CommandError::invalid_value(
                format!("`{}` cannot be keyed", raw.trim()),
                json!({ "field": "prop", "options": property_ids() }),
            )
        })
}

/// The keyed value of `prop` at `local_ms` milliseconds after the clip starts, or `None` when the
/// clip has no track for it.
pub fn evaluate(tracks: &[KeyTrack], prop: Prop, local_ms: f64) -> Option<f64> {
    tracks
        .iter()
        .find(|track| track.prop == prop)
        .map(|track| track.evaluate(local_ms))
}

fn time_error() -> CommandError {
    CommandError::invalid_value(
        "A key `time_ms` is a whole number of milliseconds from 0",
        json!({ "field": "time_ms", "min": 0, "max": crate::group::MAX_TIME_MS, "unit": "ms" }),
    )
}

fn easing_error(label: &str) -> CommandError {
    CommandError::invalid_value(
        format!("`{label}` is not an easing"),
        json!({ "field": "easing", "options": Easing::labels() }),
    )
}

/// Tracks in the inspector's order, each with its keys strictly increasing in time and valid; an
/// empty track is dropped. The one place a candidate list is checked before it replaces a clip's.
pub fn normalise(mut tracks: Vec<KeyTrack>) -> Result<Vec<KeyTrack>, CommandError> {
    tracks.retain(|track| !track.keys.is_empty());
    for track in &tracks {
        track.validate()?;
    }
    tracks.sort_by_key(|track| {
        PROPERTIES
            .iter()
            .position(|(_, prop)| *prop == track.prop)
            .unwrap_or(usize::MAX)
    });
    Ok(tracks)
}

/// A copy of `tracks` with a key added at `time_ms` on `prop`, or the key already there replaced.
/// The easing of a replaced key stays unless one is given.
pub fn with_key(
    tracks: &[KeyTrack],
    prop: Prop,
    time_ms: i64,
    value: f64,
    easing: Option<Easing>,
) -> Result<Vec<KeyTrack>, CommandError> {
    let mut next = tracks.to_vec();
    let index = match next.iter().position(|track| track.prop == prop) {
        Some(index) => index,
        None => {
            next.push(KeyTrack::new(prop));
            next.len() - 1
        }
    };
    let track = &mut next[index];
    match track.keys.iter().position(|key| key.time_ms == time_ms) {
        Some(at) => {
            track.keys[at].value = value;
            if let Some(easing) = easing {
                track.keys[at].easing = easing;
            }
        }
        None => {
            if track.keys.len() >= MAX_KEYS {
                return Err(CommandError::invalid_value(
                    format!("A track has at most {MAX_KEYS} keys"),
                    json!({ "field": "keys", "max": MAX_KEYS }),
                ));
            }
            track.keys.push(Key {
                time_ms,
                value,
                easing: easing.unwrap_or_default(),
            });
            track.keys.sort_by_key(|key| key.time_ms);
        }
    }
    normalise(next)
}

/// A copy of `tracks` without the key at exactly `time_ms` on `prop`; the track goes with its last
/// key.
pub fn without_key(
    tracks: &[KeyTrack],
    prop: Prop,
    time_ms: i64,
) -> Result<Vec<KeyTrack>, CommandError> {
    let mut next = tracks.to_vec();
    let missing = || {
        CommandError::new(
            "unknown_element",
            format!(
                "The clip has no `{}` key at {time_ms} ms",
                property_id(prop)
            ),
        )
    };
    let track = next
        .iter_mut()
        .find(|track| track.prop == prop)
        .ok_or_else(missing)?;
    let before = track.keys.len();
    track.keys.retain(|key| key.time_ms != time_ms);
    if track.keys.len() == before {
        return Err(missing());
    }
    normalise(next)
}

/// A copy of `tracks` without the track of `prop` (every track when `None`).
pub fn cleared(tracks: &[KeyTrack], prop: Option<Prop>) -> Vec<KeyTrack> {
    tracks
        .iter()
        .filter(|track| prop.is_some_and(|prop| track.prop != prop))
        .cloned()
        .collect()
}

/// A copy of `tracks` with one field (`time_ms`, `value` or `easing`) of the `index`-th key of
/// `prop` changed. A retimed key that lands past its neighbour is refused, never re-sorted under
/// the caller's index.
pub fn with_key_field(
    tracks: &[KeyTrack],
    prop: Prop,
    index: usize,
    field: &str,
    value: &Raw,
) -> Result<Vec<KeyTrack>, CommandError> {
    let mut next = tracks.to_vec();
    let key = next
        .iter_mut()
        .find(|track| track.prop == prop)
        .and_then(|track| track.keys.get_mut(index))
        .ok_or_else(|| {
            CommandError::new(
                "unknown_element",
                format!("The clip has no `{}` key number {index}", property_id(prop)),
            )
        })?;
    match field {
        "time_ms" => key.time_ms = value.integer().map_err(|_| time_error())?,
        "value" => key.value = value.number()?,
        "easing" => {
            let label = value.text();
            key.easing = Easing::parse(&label).ok_or_else(|| easing_error(&label))?;
        }
        other => {
            return Err(CommandError::new(
                "unknown_field",
                format!("A key has no field `{other}`"),
            )
            .details(json!({ "available": Key::FIELDS })));
        }
    }
    normalise(next)
}

/// The tracks of a clip as the document stores them: the group track shape, with the inspector
/// property id as `prop`.
pub fn tracks_to_json(tracks: &[KeyTrack]) -> Value {
    Value::Array(
        tracks
            .iter()
            .map(|track| {
                let mut value = track.to_json();
                value["prop"] = json!(property_id(track.prop));
                value
            })
            .collect(),
    )
}

/// Reads the tracks of a clip from the document (or from a request), checked as a unit.
pub fn tracks_from_json(value: &Value) -> Result<Vec<KeyTrack>, CommandError> {
    let items = value.as_array().ok_or_else(|| {
        CommandError::invalid_value(
            "Clip keys are an array of tracks",
            json!({ "format": [{"prop": "zoom", "keys": [{"time_ms": 0, "value": 100}]}] }),
        )
    })?;
    let mut tracks = Vec::new();
    for item in items {
        let id = item
            .get("prop")
            .and_then(Value::as_str)
            .ok_or_else(|| CommandError::invalid_value("A track needs `prop`", json!({})))?;
        let prop = parse_property(id)?;
        let mut group_form = item.clone();
        group_form["prop"] = json!(prop.label());
        let track = KeyTrack::from_json(&group_form)?;
        if tracks
            .iter()
            .any(|existing: &KeyTrack| existing.prop == prop)
        {
            return Err(CommandError::invalid_value(
                format!("`{id}` has more than one track"),
                json!({ "field": "prop" }),
            ));
        }
        tracks.push(track);
    }
    normalise(tracks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear(time_ms: i64, value: f64) -> Key {
        Key {
            time_ms,
            value,
            easing: Easing::Linear,
        }
    }

    #[test]
    fn a_key_is_added_replaced_and_removed_on_its_own_property() {
        let tracks = with_key(&[], Prop::Scale, 0, 100.0, None).unwrap();
        let tracks = with_key(&tracks, Prop::Scale, 1000, 150.0, Some(Easing::SmoothStep)).unwrap();
        let tracks = with_key(&tracks, Prop::X, 500, -20.0, None).unwrap();
        assert_eq!(
            tracks
                .iter()
                .map(|t| property_id(t.prop))
                .collect::<Vec<_>>(),
            ["position_x", "zoom"],
            "tracks follow the inspector's order"
        );
        // Replacing keeps the easing unless one is said.
        let tracks = with_key(&tracks, Prop::Scale, 1000, 160.0, None).unwrap();
        let zoom = tracks.iter().find(|t| t.prop == Prop::Scale).unwrap();
        assert_eq!(zoom.keys.len(), 2);
        assert_eq!(zoom.keys[1].value, 160.0);
        assert_eq!(zoom.keys[1].easing, Easing::SmoothStep);

        let tracks = without_key(&tracks, Prop::X, 500).unwrap();
        assert!(
            tracks.iter().all(|t| t.prop != Prop::X),
            "the last key takes its track"
        );
        assert_eq!(
            without_key(&tracks, Prop::Scale, 7).unwrap_err().code,
            "unknown_element"
        );
    }

    #[test]
    fn evaluation_interpolates_and_holds_the_ends() {
        let tracks = vec![KeyTrack {
            prop: Prop::Scale,
            repeat: 1,
            mode: crate::group::RepeatMode::Restart,
            keys: vec![linear(0, 100.0), linear(1000, 200.0)],
        }];
        assert_eq!(evaluate(&tracks, Prop::Scale, -50.0), Some(100.0));
        assert_eq!(evaluate(&tracks, Prop::Scale, 500.0), Some(150.0));
        assert_eq!(evaluate(&tracks, Prop::Scale, 5000.0), Some(200.0));
        assert_eq!(evaluate(&tracks, Prop::X, 500.0), None);
    }

    #[test]
    fn json_round_trips_with_inspector_names_and_refuses_bad_tracks() {
        let tracks = with_key(&[], Prop::Opacity, 0, 0.0, None).unwrap();
        let tracks = with_key(
            &tracks,
            Prop::Opacity,
            400,
            100.0,
            Some(Easing::EaseOutCubic),
        )
        .unwrap();
        let json = tracks_to_json(&tracks);
        assert_eq!(json[0]["prop"], "opacity");
        assert_eq!(tracks_from_json(&json).unwrap(), tracks);

        let unknown = json!([{"prop": "crop_left", "keys": [{"time_ms": 0, "value": 1}]}]);
        assert_eq!(
            tracks_from_json(&unknown).unwrap_err().code,
            "invalid_value"
        );
        let twice = json!([
            {"prop": "zoom", "keys": [{"time_ms": 0, "value": 100}]},
            {"prop": "zoom", "keys": [{"time_ms": 0, "value": 120}]},
        ]);
        assert!(tracks_from_json(&twice).is_err());
        let out_of_order = json!([{"prop": "zoom", "keys": [
            {"time_ms": 500, "value": 100}, {"time_ms": 100, "value": 120}]}]);
        assert!(tracks_from_json(&out_of_order).is_err());
    }
}
