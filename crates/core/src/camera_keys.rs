//! Camera keyframes of a scene clip (BL140): position, target and field of view over time, with
//! an easing per segment. Pure and deterministic: a function of the keys and a time, with no clock,
//! playback state or window.
//!
//! **Time base.** A key's `time` is *scene time* in exact seconds, the same time the plan reports
//! as `scene_time` (the clip's `source_start` plus the elapsed clip time at its speed). It is not
//! counted from the clip's first frame, so a keyframe stays on the picture it was set on: moving a
//! clip changes nothing, trimming the start edge (which advances `source_start`) hides the keys
//! before the new first frame without moving the rest, and trimming the end hides the later ones.
//! A clip without keys keeps the older behaviour (`Camera::at`, the `orbit` rate counted from the
//! clip start).
//!
//! **Evaluation.** Before the first key the camera holds the first pose, after the last key it
//! holds the last one. Between two keys position, target and FOV interpolate linearly with the
//! *earlier* key's easing applied to the segment progress. The easings are the directed-scene
//! ones (`directed_scene::apply_easing`, the origin's `applyEasing`), so a directed move and a
//! clip move with the same easing and the same progress give the same factor.
//!
//! **Validity.** Every key is a valid `Camera`; times are strictly increasing, between 0 and one
//! day; a clip has at most [`MAX_KEYS`] keys. Because a segment interpolates linearly, the vector
//! from target to position is linear in the progress, so a camera that would cross its own target
//! (or look straight along Y) *between* two valid keys is found analytically and refused when the
//! keys are set, not when a frame is drawn.

use serde_json::{Value, json};

use crate::content::Camera;
use crate::directed_scene::apply_easing;
use crate::state::error::CommandError;
use crate::time::Ratio;

/// Most keys one clip carries.
pub const MAX_KEYS: usize = 256;
/// Latest key time, in seconds of scene time.
pub const MAX_TIME_SECONDS: i64 = 86_400;
/// Smallest squared horizontal distance between position and target along a segment; the same
/// limit `Camera::validate` applies to a single pose.
const MIN_HORIZONTAL_SQ: f64 = 1e-12;

/// How a segment's progress is shaped. The names are the origin's easing names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Easing {
    #[default]
    Linear,
    EaseInOutCubic,
    EaseOutCubic,
}

impl Easing {
    pub const ALL: [Self; 3] = [Self::Linear, Self::EaseInOutCubic, Self::EaseOutCubic];

    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::EaseInOutCubic => "easeInOutCubic",
            Self::EaseOutCubic => "easeOutCubic",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|easing| easing.label() == raw.trim())
    }

    pub fn labels() -> Vec<&'static str> {
        Self::ALL.iter().map(|easing| easing.label()).collect()
    }

    /// The eased progress for `t` (clamped to 0..=1 first), by the directed evaluator.
    pub fn apply(self, t: f64) -> f64 {
        apply_easing(t, self.label()).unwrap_or(t)
    }
}

/// One keyframe: the pose the camera has at `time` and the easing of the segment that starts here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraKey {
    /// Scene time in seconds (exact).
    pub time: Ratio,
    pub position: [f64; 3],
    pub target: [f64; 3],
    /// Vertical field of view in degrees.
    pub fov: f64,
    /// Easing of the segment from this key to the next; unused on the last key.
    pub easing: Easing,
}

fn time_error(message: impl Into<String>) -> CommandError {
    CommandError::invalid_value(
        message,
        json!({ "min": 0, "max": MAX_TIME_SECONDS, "unit": "seconds" }),
    )
}

impl CameraKey {
    pub fn from_camera(time: Ratio, camera: &Camera, easing: Easing) -> Self {
        Self {
            time,
            position: camera.position,
            target: camera.target,
            fov: camera.fov,
            easing,
        }
    }

    /// The still camera this key describes (`orbit` is 0: keys replace the orbit).
    pub fn camera(&self) -> Camera {
        Camera {
            position: self.position,
            target: self.target,
            fov: self.fov,
            orbit: 0.0,
        }
    }

    pub fn validate(&self) -> Result<(), CommandError> {
        if self.time.is_negative() || self.time > Ratio::from_int(MAX_TIME_SECONDS) {
            return Err(time_error(format!(
                "Camera key time {} s is outside 0..={MAX_TIME_SECONDS} s of scene time",
                self.time
            )));
        }
        self.camera().validate()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "time": self.time.to_json(),
            "position": self.position,
            "target": self.target,
            "fov": self.fov,
            "easing": self.easing.label(),
        })
    }

    pub fn from_json(value: &Value) -> Result<Self, CommandError> {
        let time = value
            .get("time")
            .and_then(Ratio::from_json)
            .ok_or_else(|| {
                time_error("Camera key `time` is seconds: an integer or {\"num\":1,\"den\":24}")
            })?;
        // A key has no orbit (keys replace it): refuse the field instead of dropping it silently.
        if let Some(orbit) = value.get("orbit").filter(|raw| !raw.is_null())
            && orbit.as_f64() != Some(0.0)
        {
            return Err(CommandError::new(
                "camera_orbit_conflict",
                "A camera key has no `orbit`: keys replace the orbit (omit it or use 0)",
            )
            .details(json!({ "orbit": orbit })));
        }
        let camera = Camera::from_json(value)?;
        let easing = match value.get("easing") {
            None | Some(Value::Null) => Easing::Linear,
            Some(raw) => raw.as_str().and_then(Easing::parse).ok_or_else(|| {
                CommandError::invalid_value(
                    "Camera key `easing` is not an option",
                    json!({ "options": Easing::labels() }),
                )
            })?,
        };
        let key = Self::from_camera(time, &camera, easing);
        key.validate()?;
        Ok(key)
    }
}

/// `[{time, position, target, fov, easing}, ...]`.
pub fn keys_from_json(value: &Value) -> Result<Vec<CameraKey>, CommandError> {
    let items = value.as_array().ok_or_else(|| {
        CommandError::invalid_value("`keys` is an array of camera keys", json!({}))
    })?;
    let mut keys = items
        .iter()
        .map(CameraKey::from_json)
        .collect::<Result<Vec<_>, _>>()?;
    sort_keys(&mut keys);
    validate_keys(&keys)?;
    Ok(keys)
}

pub fn keys_to_json(keys: &[CameraKey]) -> Value {
    Value::Array(keys.iter().map(CameraKey::to_json).collect())
}

pub fn sort_keys(keys: &mut [CameraKey]) {
    keys.sort_by_key(|key| key.time);
}

/// Squared horizontal (x, z) distance between position and target at the closest point of a
/// segment, without sampling: that vector is linear in the segment progress, so the minimum of
/// its squared length over `[0, 1]` is closed-form.
fn closest_horizontal_sq(from: &CameraKey, to: &CameraKey) -> f64 {
    let offset = |key: &CameraKey| {
        [
            key.position[0] - key.target[0],
            key.position[2] - key.target[2],
        ]
    };
    let (a, b) = (offset(from), offset(to));
    let delta = [b[0] - a[0], b[1] - a[1]];
    let length_sq = delta[0] * delta[0] + delta[1] * delta[1];
    let at = |s: f64| {
        let (x, z) = (a[0] + delta[0] * s, a[1] + delta[1] * s);
        x * x + z * z
    };
    if length_sq == 0.0 {
        return at(0.0);
    }
    let s = (-(a[0] * delta[0] + a[1] * delta[1]) / length_sq).clamp(0.0, 1.0);
    at(s)
}

/// Checks a whole key list (the unit of every edit): count, times, each pose and each segment.
pub fn validate_keys(keys: &[CameraKey]) -> Result<(), CommandError> {
    if keys.len() > MAX_KEYS {
        return Err(CommandError::invalid_value(
            format!("A clip has at most {MAX_KEYS} camera keys"),
            json!({ "max": MAX_KEYS }),
        ));
    }
    for key in keys {
        key.validate()?;
    }
    for pair in keys.windows(2) {
        let (from, to) = (&pair[0], &pair[1]);
        if from.time >= to.time {
            return Err(time_error(format!(
                "Camera key times must be strictly increasing ({} s then {} s)",
                from.time, to.time
            )));
        }
        if closest_horizontal_sq(from, to) < MIN_HORIZONTAL_SQ {
            return Err(CommandError::invalid_value(
                format!(
                    "The camera would pass through its own target (or look straight along Y) between the keys at {} s and {} s",
                    from.time, to.time
                ),
                json!({ "from": from.time.to_json(), "to": to.time.to_json() }),
            ));
        }
    }
    Ok(())
}

/// Inserts `key`, or replaces the key at the same time. The list stays sorted.
pub fn upsert(keys: &mut Vec<CameraKey>, key: CameraKey) {
    match keys.iter().position(|existing| existing.time == key.time) {
        Some(index) => keys[index] = key,
        None => {
            keys.push(key);
            sort_keys(keys);
        }
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn lerp3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
    ]
}

/// The camera at `seconds` of scene time, or `None` when there are no keys. Keys must be a valid
/// list (sorted, strictly increasing).
pub fn evaluate(keys: &[CameraKey], seconds: f64) -> Option<Camera> {
    let first = keys.first()?;
    let last = keys.last()?;
    if !seconds.is_finite() || seconds <= first.time.to_f64() {
        return Some(first.camera());
    }
    if seconds >= last.time.to_f64() {
        return Some(last.camera());
    }
    // First key strictly after `seconds`; the segment starts at the one before it.
    let after = keys.partition_point(|key| key.time.to_f64() <= seconds);
    let (from, to) = (&keys[after - 1], &keys[after]);
    let (t0, t1) = (from.time.to_f64(), to.time.to_f64());
    let progress = from.easing.apply((seconds - t0) / (t1 - t0));
    Some(Camera {
        position: lerp3(from.position, to.position, progress),
        target: lerp3(from.target, to.target, progress),
        fov: lerp(from.fov, to.fov, progress),
        orbit: 0.0,
    })
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;

    fn key(seconds: i64, z: f64, fov: f64, easing: Easing) -> CameraKey {
        CameraKey {
            time: Ratio::from_int(seconds),
            position: [0.0, 1.0, z],
            target: [0.0, 0.0, 0.0],
            fov,
            easing,
        }
    }

    #[test]
    fn a_push_in_interpolates_position_and_fov_and_holds_at_the_ends() {
        let keys = [
            key(1, 10.0, 60.0, Easing::Linear),
            key(3, 4.0, 40.0, Easing::Linear),
        ];
        assert_eq!(evaluate(&keys, 0.0).unwrap().position[2], 10.0);
        assert_eq!(evaluate(&keys, 1.0).unwrap().position[2], 10.0);
        let middle = evaluate(&keys, 2.0).unwrap();
        assert_eq!(middle.position[2], 7.0);
        assert_eq!(middle.fov, 50.0);
        assert_eq!(evaluate(&keys, 3.0).unwrap().position[2], 4.0);
        assert_eq!(evaluate(&keys, 99.0).unwrap().fov, 40.0);
        assert!(evaluate(&[], 1.0).is_none());
    }

    #[test]
    fn the_earlier_key_decides_the_easing_with_the_directed_evaluator() {
        let keys = [
            key(0, 10.0, 50.0, Easing::EaseOutCubic),
            key(4, 2.0, 50.0, Easing::Linear),
        ];
        let at = |seconds| evaluate(&keys, seconds).unwrap().position[2];
        // easeOutCubic(0.25) = 1 - 0.75^3
        let eased = 1.0 - 0.75_f64.powi(3);
        assert!((at(1.0) - (10.0 + (2.0 - 10.0) * eased)).abs() < 1e-12);
        assert_eq!(Easing::EaseInOutCubic.apply(0.5), 0.5);
        assert_eq!(Easing::Linear.apply(2.0), 1.0, "progress is clamped first");
    }

    #[test]
    fn validation_rejects_unordered_far_or_degenerate_keys() {
        let ok = [
            key(0, 10.0, 50.0, Easing::Linear),
            key(1, 5.0, 50.0, Easing::Linear),
        ];
        assert!(validate_keys(&ok).is_ok());
        let same_time = [ok[0], key(0, 5.0, 50.0, Easing::Linear)];
        assert_eq!(validate_keys(&same_time).unwrap_err().code, "invalid_value");
        let late = key(MAX_TIME_SECONDS + 1, 5.0, 50.0, Easing::Linear);
        assert!(validate_keys(&[late]).is_err());
        let mut bad_fov = ok[0];
        bad_fov.fov = 0.0;
        assert!(validate_keys(&[bad_fov]).is_err());
        let many: Vec<CameraKey> = (0..=MAX_KEYS as i64)
            .map(|i| key(i, 5.0 + i as f64 * 0.01, 50.0, Easing::Linear))
            .collect();
        assert!(validate_keys(&many).is_err());
    }

    #[test]
    fn a_segment_that_crosses_the_target_is_refused_between_two_valid_keys() {
        // Both poses are fine, the straight path between them goes through the target.
        let front = key(0, 5.0, 50.0, Easing::Linear);
        let back = key(2, -5.0, 50.0, Easing::Linear);
        let error = validate_keys(&[front, back]).unwrap_err();
        assert!(error.message.contains("pass through its own target"));
        // The same poses are fine when the camera walks around instead of through.
        let mut side = back;
        side.position = [5.0, 1.0, 0.0];
        assert!(validate_keys(&[front, side]).is_ok());
        // Vertical look: the horizontal offset reaches zero in the middle.
        let high = CameraKey {
            position: [0.0, 8.0, 0.1],
            ..front
        };
        let high_back = CameraKey {
            time: Ratio::from_int(2),
            position: [0.0, 8.0, -0.1],
            ..front
        };
        assert!(validate_keys(&[high, high_back]).is_err());
    }

    #[test]
    fn json_round_trips_and_names_the_bad_field() {
        let original = key(2, 6.0, 45.0, Easing::EaseInOutCubic);
        let again = CameraKey::from_json(&original.to_json()).unwrap();
        assert_eq!(again, original);
        let bad = json!({"time": 1, "easing": "bounce"});
        let error = CameraKey::from_json(&bad).unwrap_err();
        assert!(error.message.contains("easing"));
        assert!(CameraKey::from_json(&json!({"position":[1,2,3]})).is_err());
        let unsorted = json!([
            {"time": 2, "position": [0,1,5]},
            {"time": 1, "position": [0,1,8]},
        ]);
        let sorted = keys_from_json(&unsorted).unwrap();
        assert_eq!(sorted[0].time, Ratio::from_int(1));
    }

    #[test]
    fn upsert_replaces_the_key_at_the_same_time_and_keeps_order() {
        let mut keys = vec![key(0, 9.0, 50.0, Easing::Linear)];
        upsert(&mut keys, key(2, 5.0, 50.0, Easing::Linear));
        upsert(&mut keys, key(1, 7.0, 50.0, Easing::Linear));
        upsert(&mut keys, key(1, 6.0, 50.0, Easing::Linear));
        let times: Vec<i64> = keys.iter().map(|k| k.time.floor()).collect();
        assert_eq!(times, [0, 1, 2]);
        assert_eq!(keys[1].position[2], 6.0);
    }
}
