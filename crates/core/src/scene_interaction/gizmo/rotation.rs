//! F055 / BL015.1 (first slice, G1): rotation of ONE node by the gizmo. The pure part: tool and
//! space, the conventions, the ring geometry and the arithmetic that turns "rotate by an angle
//! about an axis" into the node's authored `localRotation` quaternion. No window, no pointer
//! device, no GPUI, no document: the project glue is `scene_project/gizmo.rs`.
//!
//! # Conventions (decided before the code, pinned by tests)
//!
//! * **Unit.** Angles in the API are **degrees** (the catalog's unit for angles elsewhere); the
//!   arithmetic is in radians. The document keeps what it always kept: the unit quaternion
//!   `localRotation`, composed as Three.js does (`Matrix4.compose`). No Euler angles, no second
//!   representation is written.
//! * **Sign.** Positive is counter-clockwise looking from the tip of the axis towards the node
//!   (right-hand rule): `+90°` about world `+Z` turns `+X` into `+Y`.
//! * **Pivot.** The node's own origin (its world translation). A rotation changes only
//!   `localRotation`, so the origin stays where it is and the children turn with the node.
//! * **Axes.** `world`: the scene's `x`, `y`, `z`. `local`: the node's own axes in the world,
//!   `O·eᵢ` with `O = Pₙ·R` the orientation of the node *without its own scale* (`Pₙ` the rotation
//!   of the parent chain, `R` the authored rotation). They are unit and orthogonal whatever the
//!   node's own scale is, and the handle of a ring is the same axis as the one a script names.
//! * **Composition with the parent.** The node's world linear part is `W = P·R·S` (`P` the
//!   parent chain and sockets, `S` the node's own scale). A world rotation `G` is applied as
//!   `W' = G·W`, which is `R' = Pₙᵀ·G·Pₙ·R` when `P` is a similarity (rotation times one
//!   uniform scale): the new local rotation turns about `Pₙᵀ·a` in the parent's frame. For a
//!   local axis that is `R' = R·Rot(eᵢ, θ)`. Any other `P` has no such `R'` (the result would
//!   be a shear a TRS node cannot hold), so it is **refused**, typed, not approximated.
//! * **Angle of a drag.** The angle between the press and the pointer, both projected on the plane
//!   of the ring, in `(-180°, 180°]`. It is recomputed from the press every time (like a
//!   translation), so it never depends on the path; a full turn is one numeric command or
//!   several drags.
//! * **A turn is not a change.** The angle is reduced modulo 360°, and an angle that reduces to
//!   (almost) zero returns the authored quaternion unchanged: no edit, no history entry.

use serde_json::{Value, json};

use super::{
    EPS, Handle, HandleGeometry, Layout, Point, V3, View, add, apply3, cross, dot, inverse3,
    linear, normalize, plane_hit, scale, sub,
};
use crate::directed_scene::{Mat4, Quat, Vec3};
use crate::scene_plan::PlanNode;

/// Radius of a rotation ring, in picture heights.
pub const RING_RADIUS: f64 = 0.10;
/// Segments of the polygon a ring is sent as (one extra point closes it).
pub const RING_SEGMENTS: usize = 64;
/// A press or move whose eye ray is this close to the plane of the ring (|cos| of the angle between
/// the ray and the axis) does not determine an angle: the ring is seen edge-on.
const GRAZING: f64 = 0.05;
/// An angle (degrees) under this is no rotation.
const NO_TURN_DEGREES: f64 = 1e-9;
/// How far `P·Pᵀ` may be from `s²·I` for `P` to be a similarity.
const SIMILARITY_TOLERANCE: f64 = 1e-6;

/// The tool of the gizmo: what the handles of the selected node do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GizmoTool {
    #[default]
    Translate,
    Rotate,
    /// BL015.1 (G1.2): scale of one node, see `scaling.rs`.
    Scale,
}

impl GizmoTool {
    pub const ALL: [GizmoTool; 3] = [Self::Translate, Self::Rotate, Self::Scale];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Translate => "translate",
            Self::Rotate => "rotate",
            Self::Scale => "scale",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.as_str() == text)
    }
}

/// The orientation of the axes of a rotation or a scale: the scene's or the node's own.
/// Translation always moves along the scene's axes (this slice does not change it), so the space
/// only matters to the rotate and scale tools and to `scene.rotate` / `scene.scale`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GizmoSpace {
    #[default]
    World,
    Local,
}

impl GizmoSpace {
    pub const ALL: [GizmoSpace; 2] = [Self::World, Self::Local];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::World => "world",
            Self::Local => "local",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|space| space.as_str() == text)
    }
}

/// Why a node cannot be rotated: a reason, never a silent no-op. The strings are the wire spelling
/// (`details.reason` of `not_rotatable`, `reason` of a hidden overlay).
pub mod refusal {
    pub const NOT_MOVABLE: &str = "not_movable";
    /// The world or local matrix cannot be inverted (a zero scale somewhere).
    pub const DEGENERATE_TRANSFORM: &str = "degenerate_transform";
    /// The authored rotation is not a finite, non-zero quaternion.
    pub const DEGENERATE_ROTATION: &str = "degenerate_rotation";
    /// The parent chain scales or shears unevenly: no local rotation can turn the node about a
    /// world axis.
    pub const NON_UNIFORM_PARENT_SCALE: &str = "non_uniform_parent_scale";
    /// The parent chain mirrors (negative determinant): the sense of a rotation would flip.
    pub const MIRRORED_PARENT: &str = "mirrored_parent";
    /// A performance channel writes this node's rotation at the frame: the base rotation is not
    /// what is shown, and a base edit would be silently offset by the animation.
    pub const ANIMATED_ROTATION: &str = "animated_rotation";
}

pub(super) type M3 = [[f64; 3]; 3];

fn transpose(m: &M3) -> M3 {
    [
        [m[0][0], m[1][0], m[2][0]],
        [m[0][1], m[1][1], m[2][1]],
        [m[0][2], m[1][2], m[2][2]],
    ]
}

fn multiply(a: &M3, b: &M3) -> M3 {
    let mut out = [[0.0; 3]; 3];
    for (row, out_row) in out.iter_mut().enumerate() {
        for (column, slot) in out_row.iter_mut().enumerate() {
            *slot = (0..3).map(|k| a[row][k] * b[k][column]).sum();
        }
    }
    out
}

pub(super) fn determinant(m: &M3) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// Hamilton product `a ⊗ b`: applies `b` first, then `a` (Three.js `Quaternion.multiply`).
fn quat_mul(a: Quat, b: Quat) -> Quat {
    Quat {
        x: a.x * b.w + a.w * b.x + a.y * b.z - a.z * b.y,
        y: a.y * b.w + a.w * b.y + a.z * b.x - a.x * b.z,
        z: a.z * b.w + a.w * b.z + a.x * b.y - a.y * b.x,
        w: a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
    }
}

fn quat_unit(q: Quat) -> Option<Quat> {
    let length = q.length();
    (length.is_finite() && length > EPS).then(|| Quat {
        x: q.x / length,
        y: q.y / length,
        z: q.z / length,
        w: q.w / length,
    })
}

/// The rotation by `radians` about the unit `axis`.
fn axis_angle(axis: V3, radians: f64) -> Quat {
    let (sine, cosine) = (radians / 2.0).sin_cos();
    Quat {
        x: axis[0] * sine,
        y: axis[1] * sine,
        z: axis[2] * sine,
        w: cosine,
    }
}

/// The angle in `(-180, 180]` equal to `degrees` modulo 360. Untouched inside that range, so a
/// value a drag reports and the same value sent to `scene.rotate` are the same bits.
fn reduce_degrees(degrees: f64) -> f64 {
    if degrees.abs() <= 180.0 {
        return degrees;
    }
    let wrapped = degrees.rem_euclid(360.0);
    if wrapped > 180.0 {
        wrapped - 360.0
    } else {
        wrapped
    }
}

/// A node ready to be rotated: its axes in the chosen space, the rotation of its parent chain
/// and its authored rotation. Built from the plan's matrices and the document's quaternion at
/// ONE frame; everything a drag needs afterwards is in here, so a drag is a function of the
/// press alone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RotationFrame {
    space: GizmoSpace,
    /// World direction of the rotation about the node's `x`, `y`, `z` handle: unit, orthogonal.
    axes: [V3; 3],
    /// Rotation of the parent chain (parent-local to world), orthogonal with determinant 1.
    parent: M3,
    /// The authored quaternion as the document holds it (returned unchanged for "no turn").
    authored: Quat,
    /// The same, normalised: what the arithmetic composes with.
    unit: Quat,
}

impl RotationFrame {
    /// The frame of `node` whose document holds `authored`, in `space`. `animated` is whether a
    /// performance channel writes the node's rotation at this frame (the caller knows: the plan
    /// does not keep the channels). Refuses with one of the [`refusal`] reasons.
    pub fn of(
        node: &PlanNode,
        authored: Quat,
        space: GizmoSpace,
        animated: bool,
    ) -> Result<Self, &'static str> {
        super::movable(node)?;
        if animated {
            return Err(refusal::ANIMATED_ROTATION);
        }
        let unit = quat_unit(authored).ok_or(refusal::DEGENERATE_ROTATION)?;
        let local = linear(&node.local_matrix);
        let world = linear(&node.world_matrix);
        // `movable` already inverted the world matrix; the local one may still be singular.
        let local_inverse = inverse3(&local).ok_or(refusal::DEGENERATE_TRANSFORM)?;
        // What stands above the node (ancestors and sockets): W = P·L, so P = W·L⁻¹.
        let parent = multiply(&world, &local_inverse);
        let determinant = determinant(&parent);
        if !determinant.is_finite() || determinant.abs() < 1e-12 {
            return Err(refusal::DEGENERATE_TRANSFORM);
        }
        let scale_squared = determinant.abs().powf(2.0 / 3.0);
        let gram = multiply(&parent, &transpose(&parent));
        for (row, gram_row) in gram.iter().enumerate() {
            for (column, value) in gram_row.iter().enumerate() {
                let wanted = if row == column { scale_squared } else { 0.0 };
                if (value - wanted).abs() > SIMILARITY_TOLERANCE * scale_squared {
                    return Err(refusal::NON_UNIFORM_PARENT_SCALE);
                }
            }
        }
        if determinant < 0.0 {
            return Err(refusal::MIRRORED_PARENT);
        }
        let scale = determinant.cbrt();
        let parent: M3 = parent.map(|row| row.map(|value| value / scale));

        let rotation = linear(&Mat4::compose(Vec3::ZERO, unit, Vec3::ONE));
        let own = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let axes = match space {
            GizmoSpace::World => own,
            GizmoSpace::Local => {
                // The columns of `Pₙ·R`: the node's x, y, z in the world, without its own scale.
                let oriented = multiply(&parent, &rotation);
                let transposed = transpose(&oriented);
                let mut axes = [[0.0; 3]; 3];
                for (axis, column) in axes.iter_mut().zip(transposed) {
                    *axis = normalize(column).ok_or(refusal::DEGENERATE_TRANSFORM)?;
                }
                axes
            }
        };
        Ok(Self {
            space,
            axes,
            parent,
            authored,
            unit,
        })
    }

    pub fn space(&self) -> GizmoSpace {
        self.space
    }

    /// The world direction a ring turns about. `None` for the screen handle (not a ring).
    pub fn axis(&self, handle: Handle) -> Option<V3> {
        match handle {
            Handle::X => Some(self.axes[0]),
            Handle::Y => Some(self.axes[1]),
            Handle::Z => Some(self.axes[2]),
            Handle::Screen | Handle::Uniform => None,
        }
    }

    /// The authored rotation after turning the node by `degrees` about the ring `handle`, or `None`
    /// for the screen handle and a non-finite angle. Exactly the authored quaternion when the
    /// angle is a whole number of turns (or too small to be a turn): then nothing is to be written.
    pub fn rotate(&self, handle: Handle, degrees: f64) -> Option<Quat> {
        let axis = self.axis(handle)?;
        if !degrees.is_finite() {
            return None;
        }
        let degrees = reduce_degrees(degrees);
        if degrees.abs() < NO_TURN_DEGREES {
            return Some(self.authored);
        }
        // Into the parent's frame: `Pₙᵀ·a`, still a unit vector (`Pₙ` is orthogonal).
        let in_parent = normalize(apply3(&transpose(&self.parent), axis))?;
        let turned = quat_mul(axis_angle(in_parent, degrees.to_radians()), self.unit);
        let turned = quat_unit(turned)?;
        [turned.x, turned.y, turned.z, turned.w]
            .iter()
            .all(|component| component.is_finite())
            .then_some(turned)
    }
}

/// Orthonormal `(u, v)` with `u × v = axis`: the angle of a point of the ring is measured from `u`
/// towards `v`, so it grows counter-clockwise about the axis.
fn ring_basis(axis: V3) -> (V3, V3) {
    let helper = if axis[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let u = normalize(sub(helper, scale(axis, dot(helper, axis)))).unwrap_or([1.0, 0.0, 0.0]);
    (u, cross(axis, u))
}

impl Layout {
    /// The rings of a node whose origin is at `world_origin`, turning about `frame`'s axes, or
    /// `None` when the origin is not in front of the camera. A part of a ring that is behind the
    /// camera is `None` in its polygon; the rest is still drawn and hit.
    pub fn rings_at(view: &View, world_origin: V3, frame: &RotationFrame) -> Option<Self> {
        let origin = view.project(world_origin)?;
        let depth = view.depth(world_origin);
        let radius = RING_RADIUS * 2.0 * view.tan_half_fov * depth;
        let mut handles = Vec::with_capacity(3);
        for handle in [Handle::X, Handle::Y, Handle::Z] {
            let axis = frame.axis(handle)?;
            let (u, v) = ring_basis(axis);
            let ring: Vec<Option<Point>> = (0..=RING_SEGMENTS)
                .map(|step| {
                    let angle = std::f64::consts::TAU * step as f64 / RING_SEGMENTS as f64;
                    let offset = add(scale(u, angle.cos()), scale(v, angle.sin()));
                    view.project(add(world_origin, scale(offset, radius)))
                })
                .collect();
            let to = ring.iter().flatten().next().copied().unwrap_or(origin);
            handles.push(HandleGeometry {
                handle,
                from: origin,
                to,
                ring,
            });
        }
        Some(Self {
            tool: GizmoTool::Rotate,
            world_origin,
            origin,
            handles,
            disabled: Vec::new(),
        })
    }

    /// The nearest ring within `HIT_RADIUS` of `point` (ties go to X, Y, Z in that order).
    pub(super) fn hit_ring(&self, view: &View, point: Point) -> Option<Handle> {
        let mut best: Option<(f64, Handle)> = None;
        for geometry in &self.handles {
            let distance = geometry
                .ring
                .windows(2)
                .filter_map(|pair| Some((pair[0]?, pair[1]?)))
                .map(|(a, b)| view.segment_distance(point, a, b))
                .fold(f64::INFINITY, f64::min);
            if distance <= super::HIT_RADIUS && best.is_none_or(|(nearest, _)| distance < nearest) {
                best = Some((distance, geometry.handle));
            }
        }
        best.map(|(_, handle)| handle)
    }
}

impl HandleGeometry {
    /// The polygon of a ring as JSON: a point, or `null` where the ring is behind the camera.
    pub(super) fn ring_json(&self) -> Value {
        Value::Array(
            self.ring
                .iter()
                .map(|point| point.map_or(Value::Null, Point::to_json))
                .collect(),
        )
    }
}

/// Where a ring drag started: everything `angle` needs, fixed at the press so the angle depends
/// on the pointer positions alone, never on the candidate the drag produced.
#[derive(Debug, Clone, PartialEq)]
pub struct RingAnchor {
    pub handle: Handle,
    view: View,
    origin: V3,
    /// The world axis the ring turns about, as it was at the press.
    axis: V3,
    u: V3,
    v: V3,
    /// Angle of the press on the ring plane, from `u` towards `v`.
    start: f64,
}

impl RingAnchor {
    /// Anchors a ring drag, pressed at `point`, about `axis` through the node origin. `None` when
    /// the press does not determine an angle: the ring is edge-on, or the pointer is on the origin.
    pub fn begin(view: &View, origin: V3, axis: V3, handle: Handle, point: Point) -> Option<Self> {
        let (u, v) = ring_basis(axis);
        let start = Self::angle_of(view, origin, axis, (u, v), point)?;
        Some(Self {
            handle,
            view: *view,
            origin,
            axis,
            u,
            v,
            start,
        })
    }

    fn angle_of(view: &View, origin: V3, axis: V3, (u, v): (V3, V3), point: Point) -> Option<f64> {
        let ray = view.ray(point);
        if dot(ray.1, axis).abs() < GRAZING {
            return None;
        }
        let hit = plane_hit(origin, axis, ray)?;
        let offset = sub(hit, origin);
        if dot(offset, offset).sqrt() < EPS {
            return None;
        }
        let angle = dot(offset, v).atan2(dot(offset, u));
        angle.is_finite().then_some(angle)
    }

    /// How far the pointer at `point` has turned about the axis since the press, in degrees, in
    /// `(-180, 180]`. `None` when this position does not resolve (the caller keeps the previous
    /// result).
    pub fn degrees(&self, point: Point) -> Option<f64> {
        let now = Self::angle_of(&self.view, self.origin, self.axis, (self.u, self.v), point)?;
        let mut turn = now - self.start;
        if turn > std::f64::consts::PI {
            turn -= std::f64::consts::TAU;
        } else if turn <= -std::f64::consts::PI {
            turn += std::f64::consts::TAU;
        }
        let degrees = turn.to_degrees();
        degrees.is_finite().then_some(degrees)
    }
}

/// Convenience for answers: the quaternion as the bundle spells it.
pub fn quat_json(q: Quat) -> Value {
    json!({ "x": q.x, "y": q.y, "z": q.z, "w": q.w })
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;

    #[test]
    fn degenerate_authored_quaternions_cannot_be_normalised() {
        for q in [
            Quat {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            },
            Quat {
                x: f64::NAN,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
            Quat {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: f64::INFINITY,
            },
        ] {
            assert!(quat_unit(q).is_none(), "{q:?}");
        }
    }

    fn plan_node(local: Mat4, world: Mat4) -> PlanNode {
        PlanNode {
            id: "n".to_string(),
            kind: crate::directed_scene::NodeKind::Group,
            parent: None,
            local_position: Vec3::ZERO,
            local_rotation_deg: Vec3::ZERO,
            local_scale: Vec3::ONE,
            local_matrix: local,
            world_matrix: world,
            render_order: 0,
            asset_id: None,
            width_scene_units: None,
            height_scene_units: None,
        }
    }

    #[test]
    fn a_frame_refuses_what_cannot_be_turned_with_the_typed_reason() {
        let identity = || plan_node(Mat4::IDENTITY, Mat4::IDENTITY);
        let unit = Quat::IDENTITY;
        for space in GizmoSpace::ALL {
            assert!(RotationFrame::of(&identity(), unit, space, false).is_ok());
            // An authored quaternion that cannot be normalised (reachable only through a hand
            // edited document; the importer is not asserted here).
            for degenerate in [
                Quat {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: 0.0,
                },
                Quat {
                    x: f64::NAN,
                    y: 0.0,
                    z: 0.0,
                    w: 1.0,
                },
                Quat {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: f64::INFINITY,
                },
            ] {
                assert_eq!(
                    RotationFrame::of(&identity(), degenerate, space, false).unwrap_err(),
                    refusal::DEGENERATE_ROTATION,
                    "{degenerate:?} in {space:?}"
                );
            }
            // An animated rotation is refused before anything is read from the matrices.
            assert_eq!(
                RotationFrame::of(&identity(), unit, space, true).unwrap_err(),
                refusal::ANIMATED_ROTATION
            );
            // A singular world matrix (a zero scale on the way) and a singular local one.
            let flat = Mat4::compose(
                Vec3::ZERO,
                Quat::IDENTITY,
                Vec3 {
                    x: 0.0,
                    y: 1.0,
                    z: 1.0,
                },
            );
            assert_eq!(
                RotationFrame::of(&plan_node(Mat4::IDENTITY, flat), unit, space, false)
                    .unwrap_err(),
                refusal::DEGENERATE_TRANSFORM
            );
            assert_eq!(
                RotationFrame::of(&plan_node(flat, Mat4::IDENTITY), unit, space, false)
                    .unwrap_err(),
                refusal::DEGENERATE_TRANSFORM
            );
        }
    }

    #[test]
    fn angles_reduce_to_the_half_open_half_turn_and_leave_small_ones_alone() {
        assert_eq!(reduce_degrees(30.123456789012345), 30.123456789012345);
        assert_eq!(reduce_degrees(-30.123456789012345), -30.123456789012345);
        assert_eq!(reduce_degrees(180.0), 180.0);
        assert_eq!(reduce_degrees(-180.0), -180.0);
        assert_eq!(reduce_degrees(360.0), 0.0);
        assert_eq!(reduce_degrees(-360.0), 0.0);
        assert_eq!(reduce_degrees(540.0), 180.0);
        assert_eq!(reduce_degrees(270.0), -90.0);
        assert_eq!(reduce_degrees(-270.0), 90.0);
        assert_eq!(reduce_degrees(720.0 + 15.0), 15.0);
    }

    #[test]
    fn the_quaternion_product_applies_the_right_operand_first() {
        // A quarter turn about Z after a quarter turn about X, applied to +Y: X first sends +Y to
        // +Z, then Z leaves +Z where it is.
        let x = axis_angle([1.0, 0.0, 0.0], std::f64::consts::FRAC_PI_2);
        let z = axis_angle([0.0, 0.0, 1.0], std::f64::consts::FRAC_PI_2);
        let m = Mat4::compose(Vec3::ZERO, quat_mul(z, x), Vec3::ONE);
        let p = m.transform_point(Vec3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        });
        assert!(p.x.abs() < 1e-12 && p.y.abs() < 1e-12 && (p.z - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_ring_basis_is_right_handed_about_the_axis() {
        for axis in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            normalize([1.0, 2.0, -3.0]).unwrap(),
        ] {
            let (u, v) = ring_basis(axis);
            let c = cross(u, v);
            assert!((0..3).all(|i| (c[i] - axis[i]).abs() < 1e-12), "{axis:?}");
            assert!(dot(u, v).abs() < 1e-12 && dot(u, axis).abs() < 1e-12);
        }
    }
}
