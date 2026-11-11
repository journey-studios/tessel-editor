//! F060 (G2.1): the gizmo of a point light, on the rails of the translation gizmo (F055).
//!
//! Nothing here is a second implementation of a drag. The light gizmo is the translate tool's
//! three arrows and centre (the same [`Anchor`](super::Anchor)) plus ONE more handle, `range`: a
//! knob on the light's range sphere. This file adds only what is specific to a light:
//!
//! * where the knob and the sphere are on the picture ([`Layout::light_at`]);
//! * the arithmetic of a range drag ([`RangeAnchor`]) and the two pure functions both a drag and
//!   the numeric commands call, [`light_range`] and [`light_position`], so that the pointer and
//!   `scene.range` / `scene.translate` / `scene.edit` give the same document bit for bit;
//! * the marker of each light, which a press can select ([`pick_marker`]).
//!
//! # Conventions
//!
//! * The **range** edited is the light's `radius`: the distance at which it has fallen to a
//!   quarter (see `directed_scene::lighting`). The sphere drawn has that radius; it is the place
//!   the light is a quarter as bright, not where it stops.
//! * The knob stands on the sphere, to the **right of the light as the camera sees it**
//!   (`View::right`), so a drag along the picture's horizontal moves it. A drag is the signed
//!   distance of the pointer along that world axis since the press: `radius' = radius + Δ`.
//!   Dragging past the light makes the radius non-positive, which is refused with
//!   [`refusal::RADIUS_NOT_POSITIVE`]; the last legal candidate stays (as the scale tool's limits do).
//! * A **parent** only adds its translation to the light's offset, so a world displacement of the
//!   light is the same displacement of the stored `position`: no matrix is involved
//!   ([`light_position`]).
//! * The handles of a light whose `position` is animated (keys) are **disabled** with
//!   [`refusal::ANIMATED_POSITION`]: its static value is not what is shown, and no key is written.

use crate::directed_scene::Vec3;

use super::rotation::RING_SEGMENTS;
use super::{
    GizmoTool, HANDLE_LENGTH, Handle, HandleGeometry, Layout, Point, V3, View, add, axis_parameter,
    scale,
};

/// How close (picture heights) a press must be to a light's marker to select it.
pub const MARKER_RADIUS: f64 = 0.025;
/// The largest radius an edit accepts: far beyond any scene, it only keeps the number sane.
pub const MAX_RADIUS: f64 = 1.0e6;

/// Why a light gizmo, or one of its handles, is not available. Stable wire strings.
pub mod refusal {
    /// `keys.position` replaces the static position: the arrows are off.
    pub const ANIMATED_POSITION: &str = "animated_position";
    /// A light has no orientation: the rotate tool has nothing to turn.
    pub const NO_ROTATION: &str = "light_has_no_rotation";
    /// A light has no scale: its size is the radius, edited by the range handle.
    pub const NO_SCALE: &str = "light_has_no_scale";
    /// The scene's `lighting` section is refused by the plan (ambient, shadows, too many lights,
    /// a malformed light): no light is placed on the picture.
    pub const NOT_EVALUATED: &str = "light_not_evaluated";
    /// The radius a drag or a delta asks for is zero or negative.
    pub const RADIUS_NOT_POSITIVE: &str = "radius_not_positive";
    /// The radius a drag or a delta asks for is beyond [`super::MAX_RADIUS`] or not finite.
    pub const RADIUS_TOO_LARGE: &str = "radius_too_large";
    /// A position that would not be a finite number.
    pub const POSITION_NOT_FINITE: &str = "position_not_finite";
}

/// The radius after moving the range by `delta` scene units: the ONE function the range drag and
/// `scene.range` share. `Err` carries the typed reason; the caller keeps the last legal value.
pub fn light_range(base: f64, delta: f64) -> Result<f64, &'static str> {
    let radius = base + delta;
    if !radius.is_finite() || radius > MAX_RADIUS {
        return Err(refusal::RADIUS_TOO_LARGE);
    }
    if radius <= 0.0 {
        return Err(refusal::RADIUS_NOT_POSITIVE);
    }
    Ok(radius)
}

/// The stored `position` after moving the light by `delta` in the world: the ONE function the
/// arrows' drag and `scene.translate` share. A parent contributes its translation only, so the
/// offset moves by exactly the world displacement.
pub fn light_position(base: Vec3, delta: Vec3) -> Result<Vec3, &'static str> {
    let moved = Vec3 {
        x: base.x + delta.x,
        y: base.y + delta.y,
        z: base.z + delta.z,
    };
    if [moved.x, moved.y, moved.z].iter().all(|c| c.is_finite()) {
        Ok(moved)
    } else {
        Err(refusal::POSITION_NOT_FINITE)
    }
}

/// Where a range drag started: the line through the light along the camera's right, and where on
/// it the press was. Everything a later pointer position needs, fixed at the press.
#[derive(Debug, Clone, PartialEq)]
pub struct RangeAnchor {
    view: View,
    origin: V3,
    axis: V3,
    start: f64,
}

impl RangeAnchor {
    /// Anchors a range drag pressed at `point` on a light whose world position is `origin`.
    /// `None` when the press does not determine a position along the axis.
    pub fn begin(view: &View, origin: V3, point: Point) -> Option<Self> {
        let axis = view.right;
        let start = axis_parameter(origin, axis, view.ray(point))?;
        Some(Self {
            view: *view,
            origin,
            axis,
            start,
        })
    }

    /// The signed distance, in scene units, the pointer has moved the knob along the axis since
    /// the press. `None` when this pointer position cannot be resolved (the caller keeps the
    /// previous result).
    pub fn delta(&self, point: Point) -> Option<f64> {
        let at = axis_parameter(self.origin, self.axis, self.view.ray(point))?;
        let moved = at - self.start;
        moved.is_finite().then_some(moved)
    }
}

impl Layout {
    /// The gizmo of a light at `world_origin` with a range sphere of `radius`: the translate
    /// tool's centre and arrows (those of them not in `animated`, which are listed as disabled
    /// with that reason instead), the `range` knob and the three great circles of the sphere.
    /// `None` when the light is not in front of the camera.
    pub fn light_at(
        view: &View,
        world_origin: V3,
        radius: f64,
        animated: Option<&'static str>,
    ) -> Option<Self> {
        let origin = view.project(world_origin)?;
        let depth = view.depth(world_origin);
        let world_length = HANDLE_LENGTH * 2.0 * view.tan_half_fov * depth;
        let mut handles = Vec::new();
        let mut disabled = Vec::new();
        for handle in Handle::ALL {
            if let Some(reason) = animated {
                disabled.push((handle, reason));
                continue;
            }
            let to = match handle.axis() {
                Some(axis) => view.project(add(world_origin, scale(axis, world_length)))?,
                None => origin,
            };
            handles.push(HandleGeometry {
                handle,
                from: origin,
                to,
                ring: Vec::new(),
            });
        }
        // The knob: on the sphere, to the right of the light on the picture. Behind the camera
        // (a radius that wraps around it) it is simply not offered.
        let knob = view.project(add(world_origin, scale(view.right, radius)));
        if let Some(to) = knob {
            handles.push(HandleGeometry {
                handle: Handle::Range,
                from: origin,
                to,
                ring: Vec::new(),
            });
        }
        let sphere = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            .into_iter()
            .map(|axis| {
                let (u, v) = super::rotation::ring_basis(axis);
                (0..=RING_SEGMENTS)
                    .map(|step| {
                        let angle = std::f64::consts::TAU * step as f64 / RING_SEGMENTS as f64;
                        let offset = add(scale(u, angle.cos()), scale(v, angle.sin()));
                        view.project(add(world_origin, scale(offset, radius)))
                    })
                    .collect()
            })
            .collect();
        Some(Self {
            tool: GizmoTool::Translate,
            world_origin,
            origin,
            handles,
            disabled,
            sphere,
        })
    }
}

/// The marker of a light on the picture: its world position projected, `None` behind the camera.
pub fn marker_at(view: &View, world: V3) -> Option<Point> {
    view.project(world)
}

/// The light whose marker is under `point`: the nearest one to the camera among those within
/// [`MARKER_RADIUS`] (picture heights) of it. `lights` are `(entity, world position)`.
pub fn pick_marker<'a>(
    view: &View,
    lights: impl IntoIterator<Item = (&'a str, V3)>,
    point: Point,
) -> Option<&'a str> {
    let mut best: Option<(f64, &'a str)> = None;
    for (entity, world) in lights {
        let Some(at) = view.project(world) else {
            continue;
        };
        if view.distance(point, at) > MARKER_RADIUS {
            continue;
        }
        let depth = view.depth(world);
        if best.is_none_or(|(nearest, _)| depth < nearest) {
            best = Some((depth, entity));
        }
    }
    best.map(|(_, entity)| entity)
}
