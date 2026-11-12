//! F060 (BL023/BL002/BL003/BL065, G2.1): the point lights of a scene definition. The pure part:
//! what `sceneDefinitions[].lighting` may hold for this slice, how it is read from the document,
//! and how a light is evaluated at an edit frame. No GPUI, no runtime, no IO.
//!
//! # What is supported, and what is refused
//!
//! The origin's `lighting` is `{ ambient, lights[≤4], shadows? }`. This slice evaluates **point
//! lights only**, over image planes. Everything else the section can say is refused with a typed
//! [`LightingReason`], never dropped:
//!
//! | In the document | Answer |
//! |---|---|
//! | `ambient == 1` (the art's own light untouched) | supported (the neutral value) |
//! | `ambient != 1` | refused: [`LightingReason::AmbientNotNeutral`] |
//! | `shadows` | refused: [`LightingReason::Shadows`] |
//! | more than [`MAX_SCENE_LIGHTS`] lights | refused: [`LightingReason::TooManyLights`] (the origin's schema refuses it too; its shader would only drop the rest) |
//! | a light with a malformed or out-of-range field, unknown easing, keys out of order | refused: [`LightingReason::Invalid`] |
//! | a light the document marks `hidden` | supported: kept, flagged, not drawn |
//!
//! There are no spot, directional or ambient light types in the origin's schema: every entry of
//! `lights[]` is a point light, so nothing else needs a refusal of its own beyond the table.
//!
//! # Units, space and conventions (decided before the code, pinned by tests)
//!
//! * **Position** is in scene units. Without a `parent` it is the light's world position. With a
//!   `parent` (a node of the scene) it is the **offset from that node's world position**: the
//!   light follows the node's *translation only*, as the origin's `resolveLightFrame` does, so a
//!   rotated or scaled parent does not swing or stretch the offset. The parent's position is the
//!   translation column of the evaluated world matrix of the frame (animation and sockets
//!   included). Consequence for editing: a world displacement `Δ` of the light is exactly the
//!   same displacement of the stored offset, whatever the parent's rotation or scale.
//! * **Colour** is linear RGB, each channel in `[0, 1]`; **intensity** in `[0, 20]`.
//! * **Radius** (`radius`, the "alcance") is the distance, in scene units, at which the light has
//!   fallen to a quarter: `falloff(d) = 1 / (1 + (d / radius)²)`, applied twice
//!   ([`falloff`]), so `falloff(radius) = 1/4`. It is not a hard cut-off: the light reaches
//!   further, fainter. The editor draws it as a sphere of that radius.
//! * **Flicker** `[0, 1]` takes up to 45% off the intensity with the origin's deterministic noise
//!   of `(light id, edit time in seconds)`; at 0 it is exactly 1.
//! * **Keys** (`keys.intensity|color|position`) are on the **edit's frames**, eased like the
//!   origin's `resolveKeyframes`: before the first key the first value, after the last the last,
//!   on a key's own frame exactly its value. A keyed channel takes its value from the keys, the
//!   static field is ignored for it, and an edit of that static field is refused (the gizmo does
//!   not write a key: see `scene_edit`).
//! * **Parent unknown** is an import error (`LIGHT_PARENT_UNKNOWN`), so evaluation never meets one.
//!
//! What a light does to the picture (`lambert · falloff²` on a flat plane, splats unlit) is the
//! runtime's, documented in `docs/features/scene-light.md`.

use serde_json::{Value, json};

use super::integrity::KNOWN_EASINGS;
use super::temporal::apply_easing;
use super::transform::Vec3;

/// At most this many lights in a scene definition (the origin's `MAX_SCENE_LIGHTS`).
pub const MAX_SCENE_LIGHTS: usize = 4;
/// Largest intensity the origin's schema allows.
pub const MAX_INTENSITY: f64 = 20.0;
/// How much of the intensity the flicker takes away at flicker 1 and the noise at its maximum.
const FLICKER_DEPTH: f64 = 0.45;

/// A linear colour, each channel in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl Rgb {
    pub fn to_json(self) -> Value {
        json!({ "r": self.r, "g": self.g, "b": self.b })
    }

    fn mix(self, other: Self, t: f64) -> Self {
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
        }
    }

    fn in_range(self) -> bool {
        [self.r, self.g, self.b]
            .iter()
            .all(|channel| (0.0..=1.0).contains(channel))
    }
}

/// Why a `lighting` section is not evaluated: the stable wire spelling is [`as_str`](Self::as_str).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LightingReason {
    /// `ambient` is not 1: the art's own light would have to be scaled, which this slice does not do.
    AmbientNotNeutral,
    /// `shadows` is declared: casters and contact shadows are not drawn.
    Shadows,
    /// More lights than [`MAX_SCENE_LIGHTS`].
    TooManyLights,
    /// The section or one of its lights is malformed (see the pointer of the refusal).
    Invalid,
}

impl LightingReason {
    pub const ALL: [LightingReason; 4] = [
        Self::AmbientNotNeutral,
        Self::Shadows,
        Self::TooManyLights,
        Self::Invalid,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AmbientNotNeutral => "ambient_not_neutral",
            Self::Shadows => "shadows",
            Self::TooManyLights => "too_many_lights",
            Self::Invalid => "invalid_lighting",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.as_str() == text)
    }
}

/// A refusal with where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightingRefusal {
    pub reason: LightingReason,
    /// JSON Pointer into the bundle document.
    pub pointer: String,
    pub message: String,
}

/// One key of a track: at `frame` the track has `value`; `easing` shapes the span that arrives.
#[derive(Debug, Clone, PartialEq)]
pub struct Key<T> {
    pub frame: i64,
    pub value: T,
    pub easing: String,
}

/// The keyed channels of a light. An empty track is a channel without keys.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LightKeys {
    pub intensity: Vec<Key<f64>>,
    pub color: Vec<Key<Rgb>>,
    pub position: Vec<Key<Vec3>>,
}

/// The channel of a light a key track animates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightChannel {
    Intensity,
    Color,
    Position,
}

impl LightChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intensity => "intensity",
            Self::Color => "color",
            Self::Position => "position",
        }
    }
}

/// A point light as the document states it.
#[derive(Debug, Clone, PartialEq)]
pub struct PointLight {
    pub id: String,
    /// World position, or the offset from `parent` when there is one (scene units).
    pub position: Vec3,
    pub color: Rgb,
    pub intensity: f64,
    /// Distance at which the light has fallen to a quarter (scene units, `> 0`).
    pub radius: f64,
    pub flicker: f64,
    pub hidden: bool,
    pub parent: Option<String>,
    pub keys: LightKeys,
}

impl PointLight {
    /// Whether the channel takes its value from keys (and its static field is not what is shown).
    pub fn is_keyed(&self, channel: LightChannel) -> bool {
        match channel {
            LightChannel::Intensity => !self.keys.intensity.is_empty(),
            LightChannel::Color => !self.keys.color.is_empty(),
            LightChannel::Position => !self.keys.position.is_empty(),
        }
    }
}

/// The supported `lighting` section of one scene definition: the lights, in document order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Lighting {
    pub lights: Vec<PointLight>,
}

/// A light at an edit frame: keys resolved, flicker applied, parent added.
#[derive(Debug, Clone, PartialEq)]
pub struct LightState {
    pub id: String,
    /// World position at the frame (scene units).
    pub position: Vec3,
    pub color: Rgb,
    /// Keyed intensity times the flicker level: what multiplies the colour.
    pub intensity: f64,
    pub radius: f64,
    pub hidden: bool,
}

// ---- reading ------------------------------------------------------------------------------

fn refusal(reason: LightingReason, pointer: &str, message: impl Into<String>) -> LightingRefusal {
    LightingRefusal {
        reason,
        pointer: pointer.to_string(),
        message: message.into(),
    }
}

fn invalid(pointer: &str, message: impl Into<String>) -> LightingRefusal {
    refusal(LightingReason::Invalid, pointer, message)
}

fn number(value: Option<&Value>, pointer: &str) -> Result<f64, LightingRefusal> {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .ok_or_else(|| invalid(pointer, "expected a finite number"))
}

fn vec3(value: Option<&Value>, pointer: &str) -> Result<Vec3, LightingRefusal> {
    let Some(Value::Object(map)) = value else {
        return Err(invalid(pointer, "expected an object { x, y, z }"));
    };
    let axis = |key: &str| number(map.get(key), &format!("{pointer}/{key}"));
    Ok(Vec3 {
        x: axis("x")?,
        y: axis("y")?,
        z: axis("z")?,
    })
}

fn rgb(value: Option<&Value>, pointer: &str) -> Result<Rgb, LightingRefusal> {
    let Some(Value::Object(map)) = value else {
        return Err(invalid(pointer, "expected an object { r, g, b }"));
    };
    let channel = |key: &str| number(map.get(key), &format!("{pointer}/{key}"));
    let color = Rgb {
        r: channel("r")?,
        g: channel("g")?,
        b: channel("b")?,
    };
    if !color.in_range() {
        return Err(invalid(pointer, "each channel is between 0 and 1"));
    }
    Ok(color)
}

fn intensity(value: Option<&Value>, pointer: &str) -> Result<f64, LightingRefusal> {
    let intensity = number(value, pointer)?;
    if !(0.0..=MAX_INTENSITY).contains(&intensity) {
        return Err(invalid(
            pointer,
            format!("intensity is between 0 and {MAX_INTENSITY}"),
        ));
    }
    Ok(intensity)
}

/// A track of keys: never empty, frames whole, non-negative and strictly increasing, easings known.
fn keys<T>(
    value: Option<&Value>,
    pointer: &str,
    read: impl Fn(Option<&Value>, &str) -> Result<T, LightingRefusal>,
) -> Result<Vec<Key<T>>, LightingRefusal> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array().filter(|items| !items.is_empty()) else {
        return Err(invalid(pointer, "expected a non-empty array of keys"));
    };
    let mut out: Vec<Key<T>> = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let at = format!("{pointer}/{index}");
        let frame = item
            .get("frame")
            .and_then(Value::as_f64)
            .filter(|frame| {
                frame.fract() == 0.0 && (0.0..=9.007_199_254_740_991e15).contains(frame)
            })
            .ok_or_else(|| {
                invalid(
                    &format!("{at}/frame"),
                    "expected a whole, non-negative frame",
                )
            })? as i64;
        if out.last().is_some_and(|previous| frame <= previous.frame) {
            return Err(invalid(
                &format!("{at}/frame"),
                "key frames strictly increase",
            ));
        }
        let easing = item
            .get("easing")
            .and_then(Value::as_str)
            .filter(|easing| KNOWN_EASINGS.contains(easing))
            .ok_or_else(|| invalid(&format!("{at}/easing"), "expected a known easing"))?;
        out.push(Key {
            frame,
            value: read(item.get("value"), &format!("{at}/value"))?,
            easing: easing.to_string(),
        });
    }
    Ok(out)
}

fn light(value: &Value, pointer: &str) -> Result<PointLight, LightingRefusal> {
    let Value::Object(map) = value else {
        return Err(invalid(pointer, "expected a light object"));
    };
    let at = |key: &str| format!("{pointer}/{key}");
    let id = map
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| invalid(&at("id"), "expected a non-empty string"))?;
    let radius = number(map.get("radius"), &at("radius"))?;
    if radius <= 0.0 {
        return Err(invalid(&at("radius"), "the radius is greater than zero"));
    }
    let flicker = number(map.get("flicker"), &at("flicker"))?;
    if !(0.0..=1.0).contains(&flicker) {
        return Err(invalid(&at("flicker"), "flicker is between 0 and 1"));
    }
    let hidden = match map.get("hidden") {
        None => false,
        Some(Value::Bool(hidden)) => *hidden,
        Some(_) => return Err(invalid(&at("hidden"), "expected a boolean")),
    };
    let parent = match map.get("parent") {
        None => None,
        Some(Value::String(parent)) if !parent.is_empty() => Some(parent.clone()),
        Some(_) => return Err(invalid(&at("parent"), "expected a non-empty string")),
    };
    let tracks = match map.get("keys") {
        None => LightKeys::default(),
        Some(Value::Object(tracks)) => LightKeys {
            intensity: keys(tracks.get("intensity"), &at("keys/intensity"), intensity)?,
            color: keys(tracks.get("color"), &at("keys/color"), rgb)?,
            position: keys(tracks.get("position"), &at("keys/position"), vec3)?,
        },
        Some(_) => return Err(invalid(&at("keys"), "expected an object of key tracks")),
    };
    Ok(PointLight {
        id: id.to_string(),
        position: vec3(map.get("position"), &at("position"))?,
        color: rgb(map.get("color"), &at("color"))?,
        intensity: intensity(map.get("intensity"), &at("intensity"))?,
        radius,
        flicker,
        hidden,
        parent,
        keys: tracks,
    })
}

impl Lighting {
    /// Reads the `lighting` section of a scene definition (`pointer` is the definition's pointer
    /// into the document). `Ok(None)` when the definition declares none (absent or `null`);
    /// `Err` lists **every** refusal found, in a stable order, so a caller can report all of them.
    pub fn read(definition: &Value, pointer: &str) -> Result<Option<Self>, Vec<LightingRefusal>> {
        let section = pointer_of(pointer, "lighting");
        let Some(value) = definition.get("lighting").filter(|value| !value.is_null()) else {
            return Ok(None);
        };
        let Value::Object(map) = value else {
            return Err(vec![invalid(&section, "expected an object")]);
        };
        let mut refusals = Vec::new();
        match map.get("ambient").and_then(Value::as_f64) {
            Some(1.0) => {}
            Some(ambient) if (0.0..=1.5).contains(&ambient) => refusals.push(refusal(
                LightingReason::AmbientNotNeutral,
                &format!("{section}/ambient"),
                format!(
                    "ambient {ambient} scales the art's own light; only 1 (untouched) is drawn"
                ),
            )),
            _ => refusals.push(invalid(
                &format!("{section}/ambient"),
                "expected a number between 0 and 1.5",
            )),
        }
        if map.get("shadows").is_some_and(|shadows| !shadows.is_null()) {
            refusals.push(refusal(
                LightingReason::Shadows,
                &format!("{section}/shadows"),
                "shadows (casters, contact) are not drawn",
            ));
        }
        let mut lights = Vec::new();
        match map.get("lights") {
            Some(Value::Array(items)) => {
                if items.len() > MAX_SCENE_LIGHTS {
                    refusals.push(refusal(
                        LightingReason::TooManyLights,
                        &format!("{section}/lights"),
                        format!(
                            "{} lights; at most {MAX_SCENE_LIGHTS} are allowed",
                            items.len()
                        ),
                    ));
                }
                for (index, item) in items.iter().enumerate().take(MAX_SCENE_LIGHTS) {
                    match light(item, &format!("{section}/lights/{index}")) {
                        // Two lights of one id could not be addressed (selected, edited, undone)
                        // apart: the second is refused, loudly, rather than shadowed.
                        Ok(light) if lights.iter().any(|seen: &PointLight| seen.id == light.id) => {
                            refusals.push(invalid(
                                &format!("{section}/lights/{index}/id"),
                                "duplicate light id",
                            ));
                        }
                        Ok(light) => lights.push(light),
                        Err(refused) => refusals.push(refused),
                    }
                }
            }
            _ => refusals.push(invalid(&format!("{section}/lights"), "expected an array")),
        }
        if refusals.is_empty() {
            Ok(Some(Self { lights }))
        } else {
            Err(refusals)
        }
    }

    pub fn light(&self, id: &str) -> Option<&PointLight> {
        self.lights.iter().find(|light| light.id == id)
    }

    /// Every light at `edit_frame` (hidden ones flagged, in document order). `seconds` is the edit
    /// time (`frame / fps`) the flicker noise runs on; `anchor` gives the world position of a
    /// node of the scene at that frame (a light whose parent it does not know keeps its own
    /// position, which the importer already forbids).
    pub fn evaluate(
        &self,
        edit_frame: i64,
        seconds: f64,
        anchor: impl Fn(&str) -> Option<Vec3>,
    ) -> Vec<LightState> {
        self.lights
            .iter()
            .map(|light| {
                light.at(
                    edit_frame,
                    seconds,
                    anchor(light.parent.as_deref().unwrap_or("")),
                )
            })
            .collect()
    }
}

fn pointer_of(base: &str, key: &str) -> String {
    format!("{base}/{key}")
}

// ---- evaluation ---------------------------------------------------------------------------

/// The value of a track at `frame` (the origin's `resolveKeyframes`).
fn resolve<T: Copy>(keys: &[Key<T>], frame: i64, mix: impl Fn(T, T, f64) -> T) -> T {
    let first = &keys[0];
    if frame <= first.frame {
        return first.value;
    }
    let Some(arriving) = keys.iter().position(|key| frame <= key.frame) else {
        return keys[keys.len() - 1].value;
    };
    let (from, to) = (&keys[arriving - 1], &keys[arriving]);
    if frame == to.frame {
        return to.value;
    }
    let span = (to.frame - from.frame) as f64;
    // The easing was checked when the section was read; an unknown one cannot get here.
    let t = apply_easing((frame - from.frame) as f64 / span, &to.easing).unwrap_or(0.0);
    mix(from.value, to.value, t)
}

impl PointLight {
    /// The light at a frame. `anchor` is the parent's world position when the light has one.
    pub fn at(&self, edit_frame: i64, seconds: f64, anchor: Option<Vec3>) -> LightState {
        let intensity = if self.keys.intensity.is_empty() {
            self.intensity
        } else {
            resolve(&self.keys.intensity, edit_frame, |a, b, t| a + (b - a) * t)
        };
        let color = if self.keys.color.is_empty() {
            self.color
        } else {
            resolve(&self.keys.color, edit_frame, Rgb::mix)
        };
        let offset = if self.keys.position.is_empty() {
            self.position
        } else {
            resolve(&self.keys.position, edit_frame, |a, b, t| Vec3 {
                x: a.x + (b.x - a.x) * t,
                y: a.y + (b.y - a.y) * t,
                z: a.z + (b.z - a.z) * t,
            })
        };
        let position = match (&self.parent, anchor) {
            (Some(_), Some(anchor)) => anchor.plus(offset),
            _ => offset,
        };
        LightState {
            id: self.id.clone(),
            position,
            color,
            intensity: intensity * flicker_level(&self.id, self.flicker, seconds),
            radius: self.radius,
            hidden: self.hidden,
        }
    }
}

/// The light's falloff at `distance`: `1 / (1 + (d / radius)²)`, twice (see the module docs).
pub fn falloff(distance: f64, radius: f64) -> f64 {
    let once = 1.0 / (1.0 + (distance / radius) * (distance / radius));
    once * once
}

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// `hash01(n, salt)` of the origin (`weather-model.ts`): 32-bit integer mixing, `[0, 1)`.
fn hash01(n: i32, salt: i32) -> f64 {
    let mut h = n.wrapping_mul(0x9e37_79b1_u32 as i32) ^ salt.wrapping_mul(0x85eb_ca6b_u32 as i32);
    h ^= ((h as u32) >> 15) as i32;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= ((h as u32) >> 12) as i32;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= ((h as u32) >> 15) as i32;
    f64::from(h as u32) / 4_294_967_296.0
}

/// Smooth value noise of time, from the light's own seed; `[0, 1)`.
fn noise(seconds: f64, seed: i32) -> f64 {
    let floor = seconds.floor();
    let fraction = seconds - floor;
    // `n | 0` of the origin: the whole part wraps to 32 bits.
    let i = (floor as i64) as i32;
    let low = hash01(i, seed);
    low + (hash01(i.wrapping_add(1), seed) - low) * smooth(fraction)
}

/// The origin's `seedOf`: a 32-bit rolling hash of the id's code points.
fn seed_of(id: &str) -> i32 {
    let mut seed: i32 = 7;
    for character in id.chars() {
        seed = seed.wrapping_mul(31).wrapping_add(character as i32);
    }
    seed
}

/// Brightness at `seconds`, 1 at rest: a slow sway and a fast flutter take up to 45% off at
/// flicker 1. Deterministic per light id. Exactly 1 for a flicker of 0.
pub fn flicker_level(id: &str, flicker: f64, seconds: f64) -> f64 {
    if flicker <= 0.0 {
        return 1.0;
    }
    let seed = seed_of(id);
    let sway = noise(seconds * 3.0, seed);
    let flutter = noise(seconds * 13.0, seed.wrapping_add(101));
    1.0 - flicker * FLICKER_DEPTH * (0.6 * sway + 0.4 * flutter)
}

/// What one light adds, times its colour, at a point whose surface faces `normal` (a unit vector):
/// `max(0, n · l) · falloff²`. Pure geometry; the colour and intensity multiply it. Zero at the
/// light itself and for a light with no reach. Written for the plan's consumers and as the
/// documented formula: the tests of the pictures use their own independent copy.
pub fn lambert_falloff(light: Vec3, radius: f64, point: Vec3, normal: Vec3) -> f64 {
    let to_light = Vec3 {
        x: light.x - point.x,
        y: light.y - point.y,
        z: light.z - point.z,
    };
    let distance = to_light.x.hypot(to_light.y).hypot(to_light.z);
    if distance < 1e-9 || radius <= 0.0 {
        return 0.0;
    }
    let facing = ((normal.x * to_light.x + normal.y * to_light.y + normal.z * to_light.z)
        / distance)
        .max(0.0);
    facing * falloff(distance, radius)
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;
    use serde_json::json;

    fn section(lights: Value) -> Value {
        json!({ "lighting": { "ambient": 1, "lights": lights } })
    }

    fn lamp(id: &str) -> Value {
        json!({
            "id": id, "position": { "x": 1, "y": 2, "z": 3 },
            "color": { "r": 1, "g": 0.8, "b": 0.5 }, "intensity": 2, "radius": 3, "flicker": 0,
        })
    }

    #[test]
    fn a_neutral_section_with_point_lights_is_read_with_its_values() {
        let read = Lighting::read(&section(json!([lamp("a")])), "/sceneDefinitions/0")
            .unwrap()
            .unwrap();
        let light = read.light("a").unwrap();
        assert_eq!(
            light.position,
            Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0
            }
        );
        assert_eq!(
            light.color,
            Rgb {
                r: 1.0,
                g: 0.8,
                b: 0.5
            }
        );
        assert_eq!(
            (light.intensity, light.radius, light.flicker),
            (2.0, 3.0, 0.0)
        );
        assert!(!light.hidden && light.parent.is_none());
        assert_eq!(Lighting::read(&json!({}), "/d").unwrap(), None);
        assert_eq!(
            Lighting::read(&json!({ "lighting": null }), "/d").unwrap(),
            None
        );
    }

    #[test]
    fn what_this_slice_does_not_draw_is_refused_with_a_typed_reason_and_a_pointer() {
        let refused = |lighting: Value| {
            Lighting::read(&json!({ "lighting": lighting }), "/sceneDefinitions/0").unwrap_err()
        };
        let ambient = refused(json!({ "ambient": 0.5, "lights": [] }));
        assert_eq!(ambient.len(), 1);
        assert_eq!(ambient[0].reason, LightingReason::AmbientNotNeutral);
        assert_eq!(ambient[0].pointer, "/sceneDefinitions/0/lighting/ambient");

        let shadows = refused(
            json!({ "ambient": 1, "lights": [], "shadows": { "casters": ["a"], "contact": 0.5 } }),
        );
        assert_eq!(shadows[0].reason, LightingReason::Shadows);

        let many = refused(json!({
            "ambient": 1, "lights": [lamp("1"), lamp("2"), lamp("3"), lamp("4"), lamp("5")],
        }));
        assert_eq!(many[0].reason, LightingReason::TooManyLights);

        // Every refusal is reported, not only the first.
        let all = refused(json!({
            "ambient": 1.5, "lights": [], "shadows": { "casters": ["a"], "contact": 0 },
        }));
        let reasons: Vec<_> = all.iter().map(|found| found.reason).collect();
        assert_eq!(
            reasons,
            [LightingReason::AmbientNotNeutral, LightingReason::Shadows]
        );
    }

    #[test]
    fn a_malformed_light_is_invalid_with_the_pointer_of_its_field() {
        let broken = |patch: Value| {
            let mut light = lamp("a");
            for (key, value) in patch.as_object().unwrap() {
                light[key] = value.clone();
            }
            Lighting::read(&section(json!([light])), "/d").unwrap_err()
        };
        for (patch, field) in [
            (json!({ "radius": 0 }), "radius"),
            (json!({ "radius": -2 }), "radius"),
            (json!({ "intensity": 20.5 }), "intensity"),
            (json!({ "intensity": -1 }), "intensity"),
            (json!({ "flicker": 1.5 }), "flicker"),
            (json!({ "color": { "r": 2, "g": 0, "b": 0 } }), "color"),
            (json!({ "position": { "x": 1, "y": 2 } }), "position/z"),
            (json!({ "hidden": "yes" }), "hidden"),
            (json!({ "parent": "" }), "parent"),
            (json!({ "id": "" }), "id"),
        ] {
            let found = broken(patch.clone());
            assert_eq!(found[0].reason, LightingReason::Invalid, "{patch}");
            assert_eq!(
                found[0].pointer,
                format!("/d/lighting/lights/0/{field}"),
                "{patch}"
            );
        }
    }

    #[test]
    fn keys_must_be_ordered_known_and_in_range() {
        let with_keys = |keys: Value| {
            let mut light = lamp("a");
            light["keys"] = keys;
            Lighting::read(&section(json!([light])), "/d")
        };
        let key =
            |frame: i64, value: f64| json!({ "frame": frame, "value": value, "easing": "linear" });
        assert!(with_keys(json!({ "intensity": [key(0, 1.0), key(10, 3.0)] })).is_ok());
        for keys in [
            json!({ "intensity": [] }),
            json!({ "intensity": [key(5, 1.0), key(5, 2.0)] }),
            json!({ "intensity": [key(5, 1.0), key(2, 2.0)] }),
            json!({ "intensity": [key(0, 99.0)] }),
            json!({ "intensity": [{ "frame": 0, "value": 1, "easing": "bounce" }] }),
            json!({ "intensity": [{ "frame": 1.5, "value": 1, "easing": "linear" }] }),
            json!({ "color": [{ "frame": 0, "value": { "r": 0, "g": 0, "b": 2 }, "easing": "linear" }] }),
        ] {
            let found = with_keys(keys.clone()).unwrap_err();
            assert_eq!(found[0].reason, LightingReason::Invalid, "{keys}");
        }
    }

    #[test]
    fn keys_hold_their_ends_hit_their_frames_exactly_and_ease_between() {
        let mut light = lamp("a");
        light["keys"] = json!({
            "intensity": [
                { "frame": 10, "value": 2.0, "easing": "linear" },
                { "frame": 20, "value": 6.0, "easing": "linear" },
                { "frame": 30, "value": 0.2, "easing": "easeOutCubic" },
            ],
            "position": [
                { "frame": 0, "value": { "x": 0, "y": 0, "z": 0 }, "easing": "linear" },
                { "frame": 10, "value": { "x": 10, "y": -10, "z": 4 }, "easing": "linear" },
            ],
        });
        let lighting = Lighting::read(&section(json!([light])), "/d")
            .unwrap()
            .unwrap();
        let at = |frame: i64| lighting.evaluate(frame, 0.0, |_| None).remove(0);
        assert_eq!(at(0).intensity, 2.0); // before the first key
        assert_eq!(at(10).intensity, 2.0);
        assert_eq!(at(15).intensity, 4.0); // halfway, linear
        assert_eq!(at(20).intensity, 6.0);
        assert_eq!(at(99).intensity, 0.2); // after the last key; on key 30 exactly 0.2
        assert_eq!(at(30).intensity, 0.2);
        // easeOutCubic over 10 frames at 5/10: 1 - 0.5^3 = 0.875 of the way from 6 to 0.2.
        let expected = 6.0 + (0.2 - 6.0) * 0.875;
        assert!(
            (at(25).intensity - expected).abs() < 1e-12,
            "{}",
            at(25).intensity
        );
        assert_eq!(
            at(5).position,
            Vec3 {
                x: 5.0,
                y: -5.0,
                z: 2.0
            }
        );
        assert_eq!(
            at(50).position,
            Vec3 {
                x: 10.0,
                y: -10.0,
                z: 4.0
            }
        );
    }

    #[test]
    fn a_parent_adds_its_translation_to_the_offset_and_nothing_else() {
        let mut light = lamp("a");
        light["parent"] = json!("root");
        let lighting = Lighting::read(&section(json!([light])), "/d")
            .unwrap()
            .unwrap();
        let anchor = Vec3 {
            x: 10.0,
            y: 20.0,
            z: -5.0,
        };
        let state = lighting
            .evaluate(0, 0.0, |id| (id == "root").then_some(anchor))
            .remove(0);
        assert_eq!(
            state.position,
            Vec3 {
                x: 11.0,
                y: 22.0,
                z: -2.0
            }
        );
        // A parent the frame cannot place leaves the offset as the world position.
        assert_eq!(
            lighting.evaluate(0, 0.0, |_| None)[0].position,
            Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0
            }
        );
    }

    #[test]
    fn the_falloff_is_a_quarter_at_the_radius_and_the_lambert_term_is_clamped() {
        assert_eq!(falloff(0.0, 3.0), 1.0);
        assert!((falloff(3.0, 3.0) - 0.25).abs() < 1e-15);
        let light = Vec3 {
            x: 0.0,
            y: 0.0,
            z: 2.0,
        };
        let on_axis = lambert_falloff(
            light,
            2.0,
            Vec3::ZERO,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        );
        assert!((on_axis - 0.25).abs() < 1e-15, "{on_axis}");
        // Looking away from the light the surface gets nothing, never a negative amount.
        let away = lambert_falloff(
            light,
            2.0,
            Vec3::ZERO,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            },
        );
        assert_eq!(away, 0.0);
        assert_eq!(
            lambert_falloff(
                light,
                2.0,
                light,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0
                }
            ),
            0.0
        );
    }

    #[test]
    fn the_flicker_matches_the_origin_noise_and_is_exactly_one_at_zero() {
        assert_eq!(flicker_level("lamp-1", 0.0, 3.7), 1.0);
        // Values of the origin's `flickerLevel` (scene-lighting.ts) for these inputs, computed by
        // running its code (`Math.imul` mixing) in Node.
        for (id, flicker, seconds, expected) in [
            ("lamp-1", 1.0, 0.0, 0.688_548_438_411_671_6),
            ("lamp-1", 1.0, 1.25, 0.834_336_759_573_561_8),
            ("torch", 0.5, 2.5, 0.933_557_027_329_225_1),
            // Long enough for the seed to overflow 32 bits: the wrapping has to match.
            (
                "a-very-long-light-identifier-that-overflows-the-seed",
                1.0,
                7.3,
                0.812_269_248_848_529_5,
            ),
        ] {
            let level = flicker_level(id, flicker, seconds);
            assert!(
                (level - expected).abs() < 1e-13,
                "{id} {flicker} {seconds}: {level} vs {expected}"
            );
            assert!((1.0 - FLICKER_DEPTH * flicker..=1.0).contains(&level));
        }
    }
}
