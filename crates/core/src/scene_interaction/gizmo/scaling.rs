//! F055 / BL015.1 (G1.2): scale of ONE node by the gizmo. The pure part: the conventions, the
//! limits, the handle geometry and the arithmetic that turns "scale by a factor along a handle"
//! into the node's authored `localScale`. No window, no pointer device, no GPUI, no document: the
//! project glue is `scene_project/gizmo.rs`.
//!
//! # Conventions (decided before the code, pinned by tests)
//!
//! * **Representation.** The document keeps what it always kept, `localScale` (the `scale`
//!   property, three numbers). No second representation is written; the node's position,
//!   rotation and parent are never touched.
//! * **Factor.** A scale is a **multiplicative factor** `f`, relative to the value at the press:
//!   the new component is `old · f`. A drag recomputes `f` from the press and the pointer every
//!   time (a ratio of two distances), so it never depends on the path and never accumulates; a
//!   numeric command carries the same `f`. `f = 1` is no change: the authored vector comes back
//!   with the same bits, so nothing is written and no history entry exists.
//! * **Handles.** `x`, `y`, `z` scale one axis; `uniform` applies the same `f` to the three
//!   components (so `uniform` is `f` on `x`, `y` and `z` at once, and an uneven authored scale
//!   keeps its ratios).
//! * **Pivot.** The node's own origin. Only `localScale` changes, so the origin does not move,
//!   the rotation stays the authored quaternion and the children scale with the node.
//! * **Axes.** `local`: the node's own axes in the world, the columns of `M = W·S⁻¹ = P·R` (the
//!   parent chain and the authored rotation, without the node's own scale). Handle `x` scales
//!   the node's own `x`: that is always representable (`W' = W·diag(f, 1, 1)`), whatever the
//!   parent is. `world`: a scale by `f` along the scene's axis `a` is `W' = (I + (f-1)·a·aᵀ)·W`.
//!   That is `M·D·S` with `D = I + (f-1)·T` and `T = M⁻¹·a·aᵀ·M`, whose trace is exactly 1; it is a
//!   TRS node again only when `T` has one `1` on the diagonal and zeros elsewhere, that is, when
//!   `a` runs along one of the node's own axes. Then the factor lands on that component. Any
//!   other direction would be a shear a TRS node cannot hold: that handle is **disabled** with
//!   the typed reason `world_axis_shear`, never approximated. The `uniform` handle is the same
//!   in both spaces (`W' = f·W`).
//! * **Limits (the core's, the document has none beyond "finite").** The factor must be finite
//!   and `> 0`. A changed component keeps its sign (the factor is positive: no mirroring is ever
//!   created and a pointer that crosses the origin is a refused position, not a flip) and its
//!   magnitude must stay in `[MIN_SCALE, MAX_SCALE] = [1e-3, 1e3]`; a component the document
//!   already holds outside that range may only move towards it. The new world matrix must still
//!   be invertible (the same `1e-12` determinant bound `movable` applies). A refused value is a
//!   typed reason; a drag keeps its last legal candidate.
//! * **Spark.** A splat is drawn from one uniform scale (BL123). On a node that is a splat, or
//!   has a splat below it, `x`, `y` and `z` are disabled (`splat_non_uniform`): a handle must
//!   not exist for something the renderer cannot draw. `uniform` works and keeps the world matrix
//!   a similarity.
//! * **Animation.** A performance channel that writes this node's `localScale` at the frame
//!   **replaces** the authored scale, so a base edit would not show: the node is refused
//!   (`animated_scale`), and no key is ever written. Other channels and animated parents do not
//!   matter: the frame's matrices already include them.

use super::rotation::{M3, determinant};
use super::{
    CENTER_RADIUS, EPS, HANDLE_LENGTH, HIT_RADIUS, Handle, HandleGeometry, Layout, Point, V3, View,
    add, axis_parameter, inverse3, linear, normalize, scale as scale_by,
};
use super::{GizmoSpace, GizmoTool};
use crate::directed_scene::Vec3;
use crate::scene_plan::PlanNode;

/// Smallest magnitude a scaled component may take (see the module docs).
pub const MIN_SCALE: f64 = 1e-3;
/// Largest magnitude a scaled component may take.
pub const MAX_SCALE: f64 = 1e3;
/// Where the `uniform` handle sits from the node's origin on the picture, in picture heights, up
/// and to the right at 45°. It is placed on the picture (not in the world) so it never flies
/// behind the camera and its drag factor, a ratio of distances from the origin, is stable.
pub const UNIFORM_OFFSET: f64 = 0.09;
/// A factor within this of 1 is no change.
const NO_CHANGE: f64 = 1e-12;
/// Fraction of the handle's length the press must be away from the origin along an axis: closer
/// to the origin the ratio of two positions is not a measure (a division by almost zero).
const MIN_PRESS_FRACTION: f64 = 0.1;
/// Same for the `uniform` handle, in picture heights.
const MIN_UNIFORM_PRESS: f64 = 0.02;
/// How far `T` may be from "one 1 on the diagonal, zeros elsewhere" for a world axis to count as
/// running along a node axis (numeric noise of products of rotations, ~1e-16).
const ALIGNMENT_TOLERANCE: f64 = 1e-9;
/// The shear a world-axis scale may introduce: `|f - 1| · (misalignment)`.
const SHEAR_TOLERANCE: f64 = 1e-6;
/// The determinant bound of `movable` (`inverse3`): a result below it would not be a node.
const MIN_DETERMINANT: f64 = 1e-12;

/// Why a node, a handle or a factor cannot be scaled: a reason, never a silent no-op. The strings
/// are the wire spelling (`details.reason` of `not_scalable`, `reason` of a hidden overlay, of
/// `disabled_handles` and of a refused drag position). Matrix and node reasons are shared with
/// the rotate tool (`refusal::NOT_MOVABLE`, `refusal::DEGENERATE_TRANSFORM`).
pub mod scale_refusal {
    /// A performance channel writes this node's `localScale` at the frame (it replaces it).
    pub const ANIMATED_SCALE: &str = "animated_scale";
    /// The authored scale is not finite or has a (nearly) zero component.
    pub const DEGENERATE_SCALE: &str = "degenerate_scale";
    /// The scale the plan shows is not the authored one (defensive: only an animated frame
    /// differs, and that is refused first).
    pub const SCALE_NOT_AUTHORED: &str = "scale_not_authored";
    /// The node is, or has below it, a splat: one uniform scale only (`x`, `y`, `z` disabled).
    pub const SPLAT_NON_UNIFORM: &str = "splat_non_uniform";
    /// The world axis does not run along one of the node's own axes: the result would be a
    /// shear (`x`, `y`, `z` disabled in `world`).
    pub const WORLD_AXIS_SHEAR: &str = "world_axis_shear";
    /// `screen` is the translation plane, not a scale handle.
    pub const NOT_A_SCALE_HANDLE: &str = "not_a_scale_handle";
    pub const FACTOR_NOT_FINITE: &str = "factor_not_finite";
    /// Zero or negative: a pointer past the origin, or a mirror the tool never creates.
    pub const FACTOR_NOT_POSITIVE: &str = "factor_not_positive";
    pub const BELOW_MINIMUM: &str = "below_minimum_scale";
    pub const ABOVE_MAXIMUM: &str = "above_maximum_scale";
    /// The scaled world matrix would not be invertible.
    pub const DEGENERATE_RESULT: &str = "degenerate_result";
}

use scale_refusal as why;

/// What one of the `x`, `y`, `z` handles does on a node.
#[derive(Debug, Clone, Copy, PartialEq)]
struct AxisScale {
    /// World direction of the handle (unit): the node's own axis in `local`, the scene's in `world`.
    direction: V3,
    /// The component of `localScale` the factor lands on (`0` x, `1` y, `2` z).
    component: usize,
    /// How far the world axis is from running exactly along that component (0 in `local`).
    skew: f64,
}

/// A node ready to be scaled: what each handle does and the authored scale. Built from the plan's
/// matrices and the document's `localScale` at ONE frame; everything a drag needs afterwards is
/// in here, so a drag is a function of the press alone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScaleFrame {
    space: GizmoSpace,
    /// The authored `localScale` (returned unchanged for "no change").
    base: [f64; 3],
    /// Determinant of the node's world linear part.
    world_determinant: f64,
    axes: [Result<AxisScale, &'static str>; 3],
}

fn handle_index(handle: Handle) -> Option<usize> {
    match handle {
        Handle::X => Some(0),
        Handle::Y => Some(1),
        Handle::Z => Some(2),
        Handle::Screen | Handle::Uniform => None,
    }
}

/// How the world axis `h` sits on the node's own axes: `Ok` when it runs along exactly one.
fn world_axis(m: &M3, inverse: &M3, h: usize) -> Result<AxisScale, &'static str> {
    // `T = M⁻¹·a·aᵀ·M` for `a = eₕ`: column `h` of `M⁻¹` times row `h` of `M`; trace 1.
    let t = |i: usize, j: usize| inverse[i][h] * m[h][j];
    let along = (0..3)
        .max_by(|a, b| t(*a, *a).abs().total_cmp(&t(*b, *b).abs()))
        .unwrap_or(0);
    let mut skew = 0.0_f64;
    for i in 0..3 {
        for j in 0..3 {
            let off = if i == along && j == along {
                t(i, j) - 1.0
            } else {
                t(i, j)
            };
            skew = skew.max(off.abs());
        }
    }
    // A NaN skew is refused too.
    if skew.is_nan() || skew > ALIGNMENT_TOLERANCE {
        return Err(why::WORLD_AXIS_SHEAR);
    }
    let mut direction = [0.0; 3];
    direction[h] = 1.0;
    Ok(AxisScale {
        direction,
        component: along,
        skew,
    })
}

impl ScaleFrame {
    /// The frame of `node` whose document holds `base` as `localScale`, in `space`. `animated` is
    /// whether a performance channel writes the node's scale at this frame and `splat` whether the
    /// node is a splat or has one below it (the caller knows: the plan does not keep the
    /// channels). Refuses with one of the [`scale_refusal`] reasons or `refusal::*`.
    pub fn of(
        node: &PlanNode,
        base: Vec3,
        space: GizmoSpace,
        animated: bool,
        splat: bool,
    ) -> Result<Self, &'static str> {
        super::movable(node)?;
        if animated {
            return Err(why::ANIMATED_SCALE);
        }
        let base = vec_array(base);
        if base.iter().any(|c| !c.is_finite() || c.abs() <= EPS) {
            return Err(why::DEGENERATE_SCALE);
        }
        let shown = vec_array(node.local_scale);
        if shown != base {
            return Err(why::SCALE_NOT_AUTHORED);
        }
        let world = linear(&node.world_matrix);
        // The node's world matrix without its own scale: `M = W·S⁻¹ = P·R`.
        let mut m = world;
        for row in &mut m {
            for (value, scale) in row.iter_mut().zip(base) {
                *value /= scale;
            }
        }
        let inverse = inverse3(&m).ok_or(super::refusal::DEGENERATE_TRANSFORM)?;
        let axes = [0, 1, 2].map(|h| {
            if splat {
                return Err(why::SPLAT_NON_UNIFORM);
            }
            match space {
                GizmoSpace::Local => normalize([m[0][h], m[1][h], m[2][h]])
                    .map(|direction| AxisScale {
                        direction,
                        component: h,
                        skew: 0.0,
                    })
                    .ok_or(super::refusal::DEGENERATE_TRANSFORM),
                GizmoSpace::World => world_axis(&m, &inverse, h),
            }
        });
        Ok(Self {
            space,
            base,
            world_determinant: determinant(&world),
            axes,
        })
    }

    pub fn space(&self) -> GizmoSpace {
        self.space
    }

    /// The authored `localScale`.
    pub fn base(&self) -> Vec3 {
        array_vec(self.base)
    }

    /// The world direction an axis handle scales along; `None` for `uniform`, `screen` and a
    /// disabled handle.
    pub fn direction(&self, handle: Handle) -> Option<V3> {
        let axis = self.axes[handle_index(handle)?].as_ref().ok()?;
        Some(axis.direction)
    }

    /// The component (`"x"`, `"y"`, `"z"`) of `localScale` an axis handle writes. In `world` it can
    /// differ from the handle's own letter (a node turned a quarter turn).
    pub fn component(&self, handle: Handle) -> Option<&'static str> {
        let axis = self.axes[handle_index(handle)?].as_ref().ok()?;
        Some(["x", "y", "z"][axis.component])
    }

    /// Why an axis handle is off on this node, or `None` when it is usable (or not an axis).
    pub fn disabled(&self, handle: Handle) -> Option<&'static str> {
        self.axes[handle_index(handle)?].as_ref().err().copied()
    }

    /// The authored scale after scaling the node by `factor` along `handle`. Exactly the authored
    /// vector when `factor` is 1 (or within `1e-12` of it): then nothing is to be written.
    pub fn scale(&self, handle: Handle, factor: f64) -> Result<Vec3, &'static str> {
        let mut changed = [true; 3];
        let mut skew = 0.0;
        match handle {
            Handle::Uniform => {}
            Handle::Screen => return Err(why::NOT_A_SCALE_HANDLE),
            axis => {
                let usable = self.axes[handle_index(axis).unwrap_or(0)]
                    .as_ref()
                    .map_err(|reason| *reason)?;
                changed = [false; 3];
                changed[usable.component] = true;
                skew = usable.skew;
            }
        }
        if !factor.is_finite() {
            return Err(why::FACTOR_NOT_FINITE);
        }
        if factor <= 0.0 {
            return Err(why::FACTOR_NOT_POSITIVE);
        }
        // What the factor adds to a world axis that is not exactly along a node axis.
        if (factor - 1.0).abs() * skew > SHEAR_TOLERANCE {
            return Err(why::WORLD_AXIS_SHEAR);
        }
        if (factor - 1.0).abs() < NO_CHANGE {
            return Ok(self.base());
        }
        let mut next = self.base;
        for (value, scaled) in next.iter_mut().zip(changed) {
            if scaled {
                let result = *value * factor;
                within_limits(*value, result)?;
                *value = result;
            }
        }
        let count = changed.iter().filter(|scaled| **scaled).count() as i32;
        let determinant = self.world_determinant * factor.powi(count);
        if !determinant.is_finite() || determinant.abs() < MIN_DETERMINANT {
            return Err(why::DEGENERATE_RESULT);
        }
        Ok(array_vec(next))
    }
}

fn vec_array(vec: Vec3) -> [f64; 3] {
    [vec.x, vec.y, vec.z]
}

fn array_vec(a: [f64; 3]) -> Vec3 {
    Vec3 {
        x: a[0],
        y: a[1],
        z: a[2],
    }
}

/// The magnitude of a changed component stays in `[MIN_SCALE, MAX_SCALE]`; one the document
/// already holds outside may only move towards the range.
fn within_limits(old: f64, new: f64) -> Result<(), &'static str> {
    let (old, new) = (old.abs(), new.abs());
    if !new.is_finite() {
        return Err(why::ABOVE_MAXIMUM);
    }
    if new < MIN_SCALE && new < old {
        return Err(why::BELOW_MINIMUM);
    }
    if new > MAX_SCALE && new > old {
        return Err(why::ABOVE_MAXIMUM);
    }
    Ok(())
}

impl Layout {
    /// The handles of a node whose origin is at `world_origin`, scaling as `frame` says, or `None`
    /// when the origin (or a tip) is not in front of the camera. The handles `frame` disables are
    /// not drawn: they are in `disabled` with their reason.
    pub fn scale_handles(view: &View, world_origin: V3, frame: &ScaleFrame) -> Option<Self> {
        let origin = view.project(world_origin)?;
        let depth = view.depth(world_origin);
        let world_length = HANDLE_LENGTH * 2.0 * view.tan_half_fov * depth;
        let mut handles = Vec::with_capacity(4);
        let mut disabled = Vec::new();
        for handle in [Handle::X, Handle::Y, Handle::Z] {
            match frame.direction(handle) {
                Some(direction) => {
                    let to = view.project(add(world_origin, scale_by(direction, world_length)))?;
                    handles.push(HandleGeometry {
                        handle,
                        from: origin,
                        to,
                        ring: Vec::new(),
                    });
                }
                None => disabled.push((handle, frame.disabled(handle)?)),
            }
        }
        let diagonal = std::f64::consts::FRAC_1_SQRT_2 * UNIFORM_OFFSET;
        handles.push(HandleGeometry {
            handle: Handle::Uniform,
            from: origin,
            to: Point {
                u: origin.u + diagonal / view.aspect,
                v: origin.v - diagonal,
            },
            ring: Vec::new(),
        });
        Some(Self {
            tool: GizmoTool::Scale,
            world_origin,
            origin,
            handles,
            disabled,
        })
    }

    /// The `uniform` box wins inside its radius, then the nearest axis within `HIT_RADIUS` (ties
    /// go to X, Y, Z in that order).
    pub(super) fn hit_scale(&self, view: &View, point: Point) -> Option<Handle> {
        if let Some(uniform) = self.handles.iter().find(|g| g.handle == Handle::Uniform)
            && view.distance(point, uniform.to) <= CENTER_RADIUS
        {
            return Some(Handle::Uniform);
        }
        let mut best: Option<(f64, Handle)> = None;
        for geometry in self.handles.iter().filter(|g| g.handle != Handle::Uniform) {
            let distance = view.segment_distance(point, geometry.from, geometry.to);
            if distance <= HIT_RADIUS && best.is_none_or(|(nearest, _)| distance < nearest) {
                best = Some((distance, geometry.handle));
            }
        }
        best.map(|(_, handle)| handle)
    }
}

/// Where a scale drag started: everything `factor` needs, fixed at the press so the factor depends
/// on the pointer positions alone, never on the candidate the drag produced.
#[derive(Debug, Clone, PartialEq)]
pub struct ScaleAnchor {
    pub handle: Handle,
    view: View,
    origin: V3,
    start: Start,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Start {
    /// Along a world axis through the origin: the parameter of the press on it.
    Axis { direction: V3, parameter: f64 },
    /// The `uniform` handle: the picture distance of the press from the projected origin.
    Uniform {
        centre: Point,
        press: Point,
        distance: f64,
    },
}

impl ScaleAnchor {
    /// Anchors a drag of `handle`, pressed at `point`, on a node whose world origin is `origin`.
    /// `direction` is the world direction of an axis handle (`ScaleFrame::direction`). `None` when
    /// the press does not determine a ratio: the pointer looks along the axis, or the press is too
    /// close to the origin (or behind it) for a ratio to mean anything.
    pub fn begin(
        view: &View,
        origin: V3,
        handle: Handle,
        direction: Option<V3>,
        point: Point,
    ) -> Option<Self> {
        let start = match handle {
            Handle::Uniform => {
                let centre = view.project(origin)?;
                let distance = view.distance(point, centre);
                if distance.is_nan() || distance < MIN_UNIFORM_PRESS {
                    return None;
                }
                Start::Uniform {
                    centre,
                    press: point,
                    distance,
                }
            }
            Handle::X | Handle::Y | Handle::Z => {
                let direction = direction?;
                let parameter = axis_parameter(origin, direction, view.ray(point))?;
                let reference = HANDLE_LENGTH * 2.0 * view.tan_half_fov * view.depth(origin);
                if parameter.is_nan() || parameter < MIN_PRESS_FRACTION * reference {
                    return None;
                }
                Start::Axis {
                    direction,
                    parameter,
                }
            }
            Handle::Screen => return None,
        };
        Some(Self {
            handle,
            view: *view,
            origin,
            start,
        })
    }

    /// The factor the pointer at `point` stands for: its distance from the origin (along the axis,
    /// or on the picture for `uniform`) over the press's. Not clamped: zero or negative means the
    /// pointer crossed the origin, which `ScaleFrame::scale` refuses with its typed reason.
    /// `None` when this position does not resolve (the caller keeps the previous result).
    pub fn factor(&self, point: Point) -> Option<f64> {
        let factor = match self.start {
            Start::Axis {
                direction,
                parameter,
            } => axis_parameter(self.origin, direction, self.view.ray(point))? / parameter,
            Start::Uniform {
                centre,
                press,
                distance,
            } => {
                let dot = (point.u - centre.u) * (press.u - centre.u) * self.view.aspect.powi(2)
                    + (point.v - centre.v) * (press.v - centre.v);
                if dot <= 0.0 {
                    0.0
                } else {
                    self.view.distance(point, centre) / distance
                }
            }
        };
        factor.is_finite().then_some(factor)
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;
    use crate::directed_scene::{Mat4, Quat, Vec3};

    fn plan_node(world: Mat4, scale: Vec3) -> PlanNode {
        PlanNode {
            id: "n".to_string(),
            kind: crate::directed_scene::NodeKind::Group,
            parent: None,
            local_position: Vec3::ZERO,
            local_rotation_deg: Vec3::ZERO,
            local_scale: scale,
            local_matrix: world,
            world_matrix: world,
            render_order: 0,
            asset_id: None,
            width_scene_units: None,
            height_scene_units: None,
        }
    }

    fn about_z(radians: f64, scale: Vec3) -> Mat4 {
        let (s, c) = (radians / 2.0).sin_cos();
        Mat4::compose(
            Vec3::ZERO,
            Quat {
                x: 0.0,
                y: 0.0,
                z: s,
                w: c,
            },
            scale,
        )
    }

    fn frame(space: GizmoSpace, radians: f64, splat: bool) -> Result<ScaleFrame, &'static str> {
        ScaleFrame::of(
            &plan_node(about_z(radians, Vec3::ONE), Vec3::ONE),
            Vec3::ONE,
            space,
            false,
            splat,
        )
    }

    fn view() -> View {
        View {
            position: [0.0, 0.0, 5.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            tan_half_fov: 0.5,
            aspect: 1.5,
        }
    }

    #[test]
    fn the_limits_are_decided_on_the_magnitude_and_only_against_moving_away_from_the_range() {
        // Inside, and exactly on the edges.
        for (old, new) in [
            (1.0, 2.0),
            (1.0, 1e3),
            (1.0, 1e-3),
            (-1.0, -1e3),
            (-1.0, -1e-3),
        ] {
            assert_eq!(within_limits(old, new), Ok(()), "{old} -> {new}");
        }
        // Past an edge from inside.
        assert_eq!(within_limits(1.0, 1000.0000001), Err(why::ABOVE_MAXIMUM));
        assert_eq!(within_limits(1.0, 0.00099999), Err(why::BELOW_MINIMUM));
        assert_eq!(within_limits(-1.0, -2000.0), Err(why::ABOVE_MAXIMUM));
        // From outside: towards the range is fine, away (or staying put) is not.
        assert_eq!(within_limits(5000.0, 2500.0), Ok(()));
        assert_eq!(within_limits(5000.0, 6000.0), Err(why::ABOVE_MAXIMUM));
        assert_eq!(within_limits(1e-5, 2e-5), Ok(()));
        assert_eq!(within_limits(1e-5, 5e-6), Err(why::BELOW_MINIMUM));
        // The product of two finite numbers can overflow: that is a maximum, not a NaN.
        assert_eq!(within_limits(1e200, f64::INFINITY), Err(why::ABOVE_MAXIMUM));
        assert_eq!(within_limits(1e200, f64::NAN), Err(why::ABOVE_MAXIMUM));
    }

    #[test]
    fn a_frame_refuses_what_cannot_be_scaled_with_the_typed_reason() {
        let node = |scale| plan_node(Mat4::IDENTITY, scale);
        assert!(
            ScaleFrame::of(&node(Vec3::ONE), Vec3::ONE, GizmoSpace::Local, false, false).is_ok()
        );
        assert_eq!(
            ScaleFrame::of(&node(Vec3::ONE), Vec3::ONE, GizmoSpace::Local, true, false)
                .unwrap_err(),
            why::ANIMATED_SCALE
        );
        for degenerate in [
            Vec3 {
                x: 0.0,
                ..Vec3::ONE
            },
            Vec3 {
                y: f64::NAN,
                ..Vec3::ONE
            },
            Vec3 {
                z: f64::INFINITY,
                ..Vec3::ONE
            },
        ] {
            assert_eq!(
                ScaleFrame::of(
                    &node(degenerate),
                    degenerate,
                    GizmoSpace::World,
                    false,
                    false
                )
                .unwrap_err(),
                why::DEGENERATE_SCALE,
                "{degenerate:?}"
            );
        }
        // Defensive: the scale the plan shows is not the authored one (only an animated frame can
        // differ and that is refused first, so no document reaches this; guarded, not reachable).
        let two = Vec3 {
            x: 2.0,
            ..Vec3::ONE
        };
        assert_eq!(
            ScaleFrame::of(&node(two), Vec3::ONE, GizmoSpace::Local, false, false).unwrap_err(),
            why::SCALE_NOT_AUTHORED
        );
        // A splat keeps only the uniform handle, in both spaces.
        for space in GizmoSpace::ALL {
            let splat = ScaleFrame::of(&node(Vec3::ONE), Vec3::ONE, space, false, true).unwrap();
            for handle in [Handle::X, Handle::Y, Handle::Z] {
                assert_eq!(splat.disabled(handle), Some(why::SPLAT_NON_UNIFORM));
                assert_eq!(splat.direction(handle), None);
                assert_eq!(splat.scale(handle, 2.0), Err(why::SPLAT_NON_UNIFORM));
            }
            assert_eq!(
                splat.scale(Handle::Uniform, 2.0).unwrap(),
                Vec3 {
                    x: 2.0,
                    y: 2.0,
                    z: 2.0
                }
            );
        }
    }

    #[test]
    fn the_screen_handle_is_not_a_scale_handle_and_has_no_direction() {
        let f = frame(GizmoSpace::Local, 0.0, false).unwrap();
        assert_eq!(f.scale(Handle::Screen, 2.0), Err(why::NOT_A_SCALE_HANDLE));
        assert_eq!(f.direction(Handle::Screen), None);
        assert_eq!(f.direction(Handle::Uniform), None);
        assert_eq!(f.component(Handle::Uniform), None);
        assert_eq!(f.disabled(Handle::Uniform), None);
    }

    #[test]
    fn a_world_axis_counts_as_aligned_only_within_numeric_noise() {
        // Rotated 5e-10 rad about Z: the scene's X is a node axis up to rounding noise.
        let aligned = frame(GizmoSpace::World, 5e-10, false).unwrap();
        assert_eq!(aligned.component(Handle::X), Some("x"));
        assert!(aligned.scale(Handle::X, 2.0).is_ok());
        // 2e-9 rad is over the bound: a visible-to-the-oracle shear, refused.
        let skewed = frame(GizmoSpace::World, 2e-9, false).unwrap();
        assert_eq!(skewed.disabled(Handle::X), Some(why::WORLD_AXIS_SHEAR));
        assert_eq!(skewed.disabled(Handle::Y), Some(why::WORLD_AXIS_SHEAR));
        assert_eq!(skewed.disabled(Handle::Z), None, "Z is still a node axis");
        // A quarter turn about Z swaps the components: world X lands on the node's y.
        let quarter = frame(GizmoSpace::World, std::f64::consts::FRAC_PI_2, false).unwrap();
        assert_eq!(quarter.component(Handle::X), Some("y"));
        assert_eq!(quarter.component(Handle::Y), Some("x"));
        assert_eq!(quarter.component(Handle::Z), Some("z"));
        // The same node in its own axes lands each handle on its own component.
        let local = frame(GizmoSpace::Local, std::f64::consts::FRAC_PI_2, false).unwrap();
        for (handle, component) in [(Handle::X, "x"), (Handle::Y, "y"), (Handle::Z, "z")] {
            assert_eq!(local.component(handle), Some(component));
        }
    }

    #[test]
    fn the_residual_shear_guard_bounds_what_a_huge_factor_could_add_to_an_aligned_axis() {
        // Defensive: an axis aligned within the 1e-9 noise band with a factor so large that the
        // residual would be visible. Reachable only with a factor far outside the limits (those
        // are checked after), so it is pinned here, not by an end-to-end scenario.
        let f = frame(GizmoSpace::World, 9e-10, false).unwrap();
        assert!(f.axes[0].as_ref().unwrap().skew > 0.0);
        assert_eq!(f.scale(Handle::X, 2000.0), Err(why::WORLD_AXIS_SHEAR));
        assert_eq!(f.scale(Handle::X, 500.0).unwrap().x, 500.0);
    }

    #[test]
    fn the_factor_is_checked_before_anything_is_computed() {
        let f = frame(GizmoSpace::Local, 0.3, false).unwrap();
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(f.scale(Handle::X, bad), Err(why::FACTOR_NOT_FINITE));
            assert_eq!(f.scale(Handle::Uniform, bad), Err(why::FACTOR_NOT_FINITE));
        }
        for bad in [0.0, -0.0, -1e-300, -2.0] {
            assert_eq!(
                f.scale(Handle::Y, bad),
                Err(why::FACTOR_NOT_POSITIVE),
                "{bad}"
            );
        }
        // Inside the no-change band: the authored value, bit for bit.
        assert_eq!(f.scale(Handle::Z, 1.0 + 5e-13).unwrap(), Vec3::ONE);
    }

    #[test]
    fn the_world_determinant_bound_is_on_the_world_matrix_not_on_the_components() {
        // World scale 1e-5 (determinant 1e-15 already): a component inside the range, a world
        // matrix a node must not have. Shrinking further is refused; growing is fine.
        let tiny = Vec3 {
            x: 1e-5,
            y: 1e-5,
            z: 1e-5,
        };
        let node = plan_node(about_z(0.0, tiny), tiny);
        let f = ScaleFrame::of(&node, tiny, GizmoSpace::Local, false, false);
        // `movable` refuses a node that is already below the bound: nothing to scale.
        assert!(f.is_err());
        // A parent of 0.005 over a node of (0.01, 0.01, 1): world determinant 1.25e-11, above the
        // bound. A uniform 0.1 keeps every component inside the range (1e-3, 1e-3, 0.1) and takes
        // the world to 1.25e-14, under it; one axis at a time stays above.
        let base = Vec3 {
            x: 0.01,
            y: 0.01,
            z: 1.0,
        };
        let world = Vec3 {
            x: 5e-5,
            y: 5e-5,
            z: 5e-3,
        };
        let node = plan_node(about_z(0.0, world), base);
        let f = ScaleFrame::of(&node, base, GizmoSpace::Local, false, false).unwrap();
        assert_eq!(f.scale(Handle::Uniform, 0.1), Err(why::DEGENERATE_RESULT));
        assert!(f.scale(Handle::X, 0.1).is_ok());
    }

    #[test]
    fn a_press_that_does_not_fix_a_ratio_does_not_open_a_drag() {
        let view = view();
        let origin = [0.0, 0.0, 0.0];
        let centre = view.project(origin).unwrap();
        // Uniform: pressed on the origin itself (distance 0): no ratio.
        assert!(ScaleAnchor::begin(&view, origin, Handle::Uniform, None, centre).is_none());
        let on_box = Point {
            u: centre.u + 0.05,
            v: centre.v - 0.05,
        };
        let anchor = ScaleAnchor::begin(&view, origin, Handle::Uniform, None, on_box).unwrap();
        assert!((anchor.factor(on_box).unwrap() - 1.0).abs() < 1e-12);
        let twice = Point {
            u: centre.u + 0.10,
            v: centre.v - 0.10,
        };
        assert!((anchor.factor(twice).unwrap() - 2.0).abs() < 1e-12);
        assert!(anchor.factor(centre).unwrap().abs() < 1e-12);
        // Axis: no direction given, the screen handle, the pointer looking along the axis, and a
        // press at the origin or behind it.
        let tip = view.project([1.0, 0.0, 0.0]).unwrap();
        assert!(ScaleAnchor::begin(&view, origin, Handle::X, None, tip).is_none());
        assert!(ScaleAnchor::begin(&view, origin, Handle::Screen, None, tip).is_none());
        assert!(
            ScaleAnchor::begin(&view, origin, Handle::Z, Some([0.0, 0.0, 1.0]), centre).is_none(),
            "an axis along the line of sight"
        );
        assert!(
            ScaleAnchor::begin(&view, origin, Handle::X, Some([1.0, 0.0, 0.0]), centre).is_none()
        );
        assert!(
            ScaleAnchor::begin(&view, origin, Handle::X, Some([-1.0, 0.0, 0.0]), tip).is_none(),
            "behind the origin"
        );
        let anchor =
            ScaleAnchor::begin(&view, origin, Handle::X, Some([1.0, 0.0, 0.0]), tip).unwrap();
        let double = view.project([2.0, 0.0, 0.0]).unwrap();
        assert!((anchor.factor(double).unwrap() - 2.0).abs() < 1e-9);
        let back = view.project([-1.0, 0.0, 0.0]).unwrap();
        assert!(
            (anchor.factor(back).unwrap() + 1.0).abs() < 1e-9,
            "past the origin is negative"
        );
    }

    #[test]
    fn uniform_scaling_refuses_the_opposite_side_without_changing_valid_ratios() {
        let view = view();
        let origin = [0.0; 3];
        let centre = view.project(origin).unwrap();
        let press = Point {
            u: centre.u + 0.1 / view.aspect,
            v: centre.v - 0.1,
        };
        let anchor = ScaleAnchor::begin(&view, origin, Handle::Uniform, None, press).unwrap();
        let opposite = Point {
            u: centre.u - 0.2 / view.aspect,
            v: centre.v + 0.2,
        };
        assert_eq!(anchor.factor(opposite), Some(0.0));
        assert_eq!(anchor.factor(centre), Some(0.0));
        let double = Point {
            u: centre.u + 0.2 / view.aspect,
            v: centre.v - 0.2,
        };
        assert!((anchor.factor(double).unwrap() - 2.0).abs() < 1e-12);
    }

    #[test]
    fn the_uniform_box_wins_over_an_axis_handle_that_runs_under_it() {
        let view = view();
        let origin = Point { u: 0.5, v: 0.5 };
        let diagonal = std::f64::consts::FRAC_1_SQRT_2 * UNIFORM_OFFSET;
        let uniform = Point {
            u: origin.u + diagonal / view.aspect,
            v: origin.v - diagonal,
        };
        // An x handle whose shaft runs through the box and past it, on the same diagonal.
        let far = Point {
            u: origin.u + 2.0 * diagonal / view.aspect,
            v: origin.v - 2.0 * diagonal,
        };
        let handle = |handle, to| HandleGeometry {
            handle,
            from: origin,
            to,
            ring: Vec::new(),
        };
        let layout = Layout {
            tool: GizmoTool::Scale,
            world_origin: [0.0; 3],
            origin,
            handles: vec![handle(Handle::X, far), handle(Handle::Uniform, uniform)],
            disabled: Vec::new(),
        };
        // 0.01 picture heights along the shaft from the box: on the shaft (distance 0 from `x`)
        // and inside the box's radius, so the box is the answer; an axis would tie-win otherwise.
        let along = 0.01 * std::f64::consts::FRAC_1_SQRT_2;
        let near_box = Point {
            u: uniform.u + along / view.aspect,
            v: uniform.v - along,
        };
        assert!(view.segment_distance(near_box, origin, far) < 1e-12);
        assert_eq!(layout.hit_scale(&view, near_box), Some(Handle::Uniform));
        // Past the box's radius the same shaft is the x handle.
        let beyond = Point {
            u: uniform.u + 0.08 * std::f64::consts::FRAC_1_SQRT_2 / view.aspect,
            v: uniform.v - 0.08 * std::f64::consts::FRAC_1_SQRT_2,
        };
        assert_eq!(layout.hit_scale(&view, beyond), Some(Handle::X));
        // Off every handle: nothing.
        assert_eq!(layout.hit_scale(&view, Point { u: 0.9, v: 0.9 }), None);
    }
}
