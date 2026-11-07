//! F052 (BL015/BL042): the geometry of the translation gizmo of a directed scene: where its
//! handles are on the picture, which handle or image plane a pointer is over, and how far a drag
//! along a handle moves the node. Pure functions over an evaluated [`ScenePlan`]: no window, no
//! pointer device, no pixels, no GPUI. A runtime only *draws* what [`Overlay::to_json`] lists and
//! only *reports* where the pointer is; every decision (which entity, which handle, what the new
//! position is) is made here, so the same drag can be replayed by a script.
//!
//! # Coordinates
//!
//! A pointer is a [`Point`] in the **picture**: `u` runs 0..1 left to right and `v` 0..1 top to
//! bottom over the plan's output (the picture the renderer draws, not the pane around it). The
//! adapter that owns the canvas converts its own pixels (letterbox included) to that space, so
//! the core is independent of the window size and a resize never changes the meaning of a point.
//! Lengths on the picture are measured in *picture heights* (`v` units; `u` is scaled by the
//! aspect ratio), so a handle looks the same size at any output resolution.
//!
//! # The camera
//!
//! The projection is the one the runtime draws with (`PerspectiveCamera(fov, aspect)` looking at
//! the plan's target with +Y up, vertical field of view): see `runtime/src/directed-scene.js`.
//! Lens distortion does not exist, so a point projects and unprojects exactly.
//!
//! # What a drag edits
//!
//! The gizmo moves the **world** position of a node along a scene axis (`x`, `y`, `z`) or in the
//! plane facing the camera (`screen`). The edit is written to the one catalog property that
//! exists for it, `position` (the node's `localPosition`), converted through the node's own
//! transform: `Δlocal = L · W⁻¹ · Δworld`, with `L` the linear part of the evaluated local matrix
//! and `W` the one of the world matrix. For a node without a parent that is the identity; with a
//! rotated or scaled parent it is what makes the node follow the pointer anyway.

use serde_json::{Value, json};

use crate::directed_scene::{Mat4, NodeKind, Vec3};
use crate::scene_plan::{PlanNode, ScenePlan};
use crate::state::CommandError;

/// Length of an axis handle, in picture heights.
pub const HANDLE_LENGTH: f64 = 0.14;
/// How close (picture heights) the pointer must be to a handle to hold it.
pub const HIT_RADIUS: f64 = 0.025;
/// Radius of the centre (`screen`) handle, in picture heights.
pub const CENTER_RADIUS: f64 = 0.03;
/// A `down` must land on the picture; a `move` may leave it (the pointer is captured) but not
/// by an absurd amount, which would be a coordinate bug rather than a drag.
pub const OUTSIDE_LIMIT: f64 = 8.0;
/// Closest the camera plane may come to a point for it to be drawn or picked (the runtime's
/// `near` plane).
const NEAR: f64 = 0.1;
const EPS: f64 = 1e-9;

type V3 = [f64; 3];

/// A position on the picture: `u` and `v` in 0..1, origin at the top left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub u: f64,
    pub v: f64,
}

impl Point {
    pub fn to_json(self) -> Value {
        json!({ "u": self.u, "v": self.v })
    }

    fn inside_picture(self) -> bool {
        (0.0..=1.0).contains(&self.u) && (0.0..=1.0).contains(&self.v)
    }

    /// Validates a point from outside. `strict` (a `down`) requires the picture itself.
    pub fn checked(self, strict: bool) -> Result<Self, CommandError> {
        let finite = self.u.is_finite() && self.v.is_finite();
        let bounded = if strict {
            self.inside_picture()
        } else {
            self.u.abs() <= OUTSIDE_LIMIT && self.v.abs() <= OUTSIDE_LIMIT
        };
        if finite && bounded {
            return Ok(self);
        }
        Err(CommandError::invalid_value(
            if strict {
                "A pointer press must land on the picture: `u` and `v` are between 0 and 1"
            } else {
                "The pointer point is not a finite position near the picture"
            },
            json!({ "point": { "u": self.u, "v": self.v }, "range": [0, 1] }),
        )
        .target("point"))
    }
}

/// What a drag along the gizmo moves the node along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    X,
    Y,
    Z,
    /// The plane through the node that faces the camera: free movement on the picture.
    Screen,
}

impl Handle {
    pub const ALL: [Handle; 4] = [Self::X, Self::Y, Self::Z, Self::Screen];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
            Self::Z => "z",
            Self::Screen => "screen",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|handle| handle.as_str() == text)
    }

    fn axis(self) -> Option<V3> {
        match self {
            Self::X => Some([1.0, 0.0, 0.0]),
            Self::Y => Some([0.0, 1.0, 0.0]),
            Self::Z => Some([0.0, 0.0, 1.0]),
            Self::Screen => None,
        }
    }
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(a: V3) -> Option<V3> {
    let length = dot(a, a).sqrt();
    (length.is_finite() && length > EPS).then(|| scale(a, 1.0 / length))
}
fn of(vec: Vec3) -> V3 {
    [vec.x, vec.y, vec.z]
}
fn vec3(a: V3) -> Vec3 {
    Vec3 {
        x: a[0],
        y: a[1],
        z: a[2],
    }
}

/// The 3x3 linear part of a column-major matrix, row-major here (`m[row][column]`).
fn linear(matrix: &Mat4) -> [[f64; 3]; 3] {
    let m = &matrix.0;
    [[m[0], m[4], m[8]], [m[1], m[5], m[9]], [m[2], m[6], m[10]]]
}

fn apply3(m: &[[f64; 3]; 3], v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

fn inverse3(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let c0 = [
        m[1][1] * m[2][2] - m[1][2] * m[2][1],
        m[1][2] * m[2][0] - m[1][0] * m[2][2],
        m[1][0] * m[2][1] - m[1][1] * m[2][0],
    ];
    let det = m[0][0] * c0[0] + m[0][1] * c0[1] + m[0][2] * c0[2];
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    Some([
        [
            c0[0] * inv,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv,
        ],
        [
            c0[1] * inv,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv,
        ],
        [
            c0[2] * inv,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv,
        ],
    ])
}

/// The plan's camera as a projection. Built once per plan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    position: V3,
    right: V3,
    up: V3,
    forward: V3,
    tan_half_fov: f64,
    aspect: f64,
}

impl View {
    pub fn from_plan(plan: &ScenePlan) -> Result<Self, CommandError> {
        let degenerate = |why: &str| {
            CommandError::new(
                "gizmo_unavailable",
                format!("The camera of this frame cannot project the gizmo: {why}"),
            )
            .details(json!({ "reason": "degenerate_camera" }))
        };
        let position = of(plan.camera.position);
        let forward = normalize(sub(of(plan.camera.target), position))
            .ok_or_else(|| degenerate("the camera looks at its own position"))?;
        let right = normalize(cross(forward, [0.0, 1.0, 0.0]))
            .ok_or_else(|| degenerate("the camera looks straight up or down"))?;
        let up = cross(right, forward);
        let tan_half_fov = (plan.camera.fov_deg.to_radians() / 2.0).tan();
        let aspect = plan.output.width as f64 / plan.output.height as f64;
        if !(tan_half_fov.is_finite() && tan_half_fov > 0.0 && aspect.is_finite() && aspect > 0.0) {
            return Err(degenerate(
                "the field of view or the output size is not usable",
            ));
        }
        Ok(Self {
            position,
            right,
            up,
            forward,
            tan_half_fov,
            aspect,
        })
    }

    pub fn aspect(&self) -> f64 {
        self.aspect
    }

    /// Distance of `point` in front of the camera plane (negative: behind).
    fn depth(&self, point: V3) -> f64 {
        dot(sub(point, self.position), self.forward)
    }

    /// The picture position of a world point, or `None` when it is not in front of the camera.
    pub fn project(&self, point: V3) -> Option<Point> {
        let relative = sub(point, self.position);
        let depth = dot(relative, self.forward);
        if depth <= NEAR {
            return None;
        }
        let ndc_x = dot(relative, self.right) / (depth * self.tan_half_fov * self.aspect);
        let ndc_y = dot(relative, self.up) / (depth * self.tan_half_fov);
        Some(Point {
            u: (ndc_x + 1.0) / 2.0,
            v: (1.0 - ndc_y) / 2.0,
        })
    }

    /// The ray (origin, unit direction) through a picture position.
    fn ray(&self, point: Point) -> (V3, V3) {
        let ndc_x = 2.0 * point.u - 1.0;
        let ndc_y = 1.0 - 2.0 * point.v;
        let direction = add(
            add(
                self.forward,
                scale(self.right, ndc_x * self.tan_half_fov * self.aspect),
            ),
            scale(self.up, ndc_y * self.tan_half_fov),
        );
        // The sum has a forward component of 1: never zero.
        (self.position, normalize(direction).unwrap_or(self.forward))
    }

    /// Distance on the picture, in picture heights.
    fn distance(&self, a: Point, b: Point) -> f64 {
        ((a.u - b.u) * self.aspect).hypot(a.v - b.v)
    }

    /// Distance from `p` to the segment `a`-`b` on the picture, in picture heights.
    fn segment_distance(&self, p: Point, a: Point, b: Point) -> f64 {
        let (ax, ay) = (a.u * self.aspect, a.v);
        let (bx, by) = (b.u * self.aspect, b.v);
        let (px, py) = (p.u * self.aspect, p.v);
        let (dx, dy) = (bx - ax, by - ay);
        let length_squared = dx * dx + dy * dy;
        let t = if length_squared < EPS {
            0.0
        } else {
            (((px - ax) * dx + (py - ay) * dy) / length_squared).clamp(0.0, 1.0)
        };
        (px - (ax + t * dx)).hypot(py - (ay + t * dy))
    }
}

/// One handle as drawn: where it starts and ends on the picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandleGeometry {
    pub handle: Handle,
    pub from: Point,
    /// The tip of an axis; the centre again for `Screen`.
    pub to: Point,
}

/// The gizmo of one node: its origin and its handles, on the picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub world_origin: V3,
    pub origin: Point,
    pub handles: Vec<HandleGeometry>,
}

impl Layout {
    /// The handles of a node whose origin is at `world_origin`, or `None` when the origin is not
    /// in front of the camera (nothing to draw or to hit).
    pub fn at(view: &View, world_origin: V3) -> Option<Self> {
        let origin = view.project(world_origin)?;
        let depth = view.depth(world_origin);
        // A handle is HANDLE_LENGTH picture heights long where it stands: the world length that
        // spans that much of the frustum at the node's depth.
        let world_length = HANDLE_LENGTH * 2.0 * view.tan_half_fov * depth;
        let mut handles = Vec::with_capacity(Handle::ALL.len());
        for handle in Handle::ALL {
            let to = match handle.axis() {
                Some(axis) => view.project(add(world_origin, scale(axis, world_length)))?,
                None => origin,
            };
            handles.push(HandleGeometry {
                handle,
                from: origin,
                to,
            });
        }
        Some(Self {
            world_origin,
            origin,
            handles,
        })
    }

    /// The handle under `point`: the centre wins inside its radius, then the nearest axis within
    /// `HIT_RADIUS` (ties go to X, Y, Z in that order).
    pub fn hit(&self, view: &View, point: Point) -> Option<Handle> {
        if view.distance(point, self.origin) <= CENTER_RADIUS {
            return Some(Handle::Screen);
        }
        let mut best: Option<(f64, Handle)> = None;
        for geometry in self.handles.iter().filter(|g| g.handle != Handle::Screen) {
            let distance = view.segment_distance(point, geometry.from, geometry.to);
            if distance <= HIT_RADIUS && best.is_none_or(|(nearest, _)| distance < nearest) {
                best = Some((distance, geometry.handle));
            }
        }
        best.map(|(_, handle)| handle)
    }
}

/// The image plane (front-most first) whose rectangle the pointer is over. By rectangle, not by
/// pixel: the core does not read the picture, so transparent texels still count.
pub fn pick_plane<'a>(plan: &'a ScenePlan, view: &View, point: Point) -> Option<&'a PlanNode> {
    let (origin, direction) = view.ray(point);
    plan.layers.iter().rev().find_map(|layer| {
        let node = plan.node(&layer.node_id)?;
        let (width, height) = (node.width_scene_units?, node.height_scene_units?);
        let world = linear(&node.world_matrix);
        let inverse = inverse3(&world)?;
        let translation = of(node.world_matrix.translation());
        let local_origin = apply3(&inverse, sub(origin, translation));
        let local_direction = apply3(&inverse, direction);
        if local_direction[2].abs() < EPS {
            return None;
        }
        let along = -local_origin[2] / local_direction[2];
        if along <= 0.0 {
            return None;
        }
        let x = local_origin[0] + along * local_direction[0];
        let y = local_origin[1] + along * local_direction[1];
        (x.abs() <= width / 2.0 && y.abs() <= height / 2.0).then_some(node)
    })
}

/// Where a drag started and what it is bound to: everything `delta` needs, fixed at the press so
/// the result depends on the pointer positions alone, not on the candidate the drag produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Anchor {
    pub handle: Handle,
    view: View,
    origin: V3,
    /// Parameter along the axis, or the hit on the screen plane, under the press.
    start: Start,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Start {
    Axis(f64),
    Plane(V3),
}

fn axis_parameter(origin: V3, axis: V3, ray: (V3, V3)) -> Option<f64> {
    let (from, direction) = ray;
    let b = dot(axis, direction);
    let denominator = 1.0 - b * b;
    if denominator < 1e-6 {
        // The pointer looks along the axis: its position does not say how far along it is.
        return None;
    }
    let w = sub(origin, from);
    Some((b * dot(direction, w) - dot(axis, w)) / denominator)
}

fn plane_hit(origin: V3, normal: V3, ray: (V3, V3)) -> Option<V3> {
    let (from, direction) = ray;
    let facing = dot(direction, normal);
    if facing.abs() < EPS {
        return None;
    }
    let along = dot(sub(origin, from), normal) / facing;
    (along > 0.0).then(|| add(from, scale(direction, along)))
}

impl Anchor {
    /// Anchors a drag of `handle`, pressed at `point`, on a node whose world origin is
    /// `origin`. `None` when the press does not determine a position (the pointer is parallel
    /// to the axis, or the plane is edge-on).
    pub fn begin(view: &View, origin: V3, handle: Handle, point: Point) -> Option<Self> {
        let ray = view.ray(point);
        let start = match handle.axis() {
            Some(axis) => Start::Axis(axis_parameter(origin, axis, ray)?),
            None => Start::Plane(plane_hit(origin, view.forward, ray)?),
        };
        Some(Self {
            handle,
            view: *view,
            origin,
            start,
        })
    }

    /// How far the node has moved in the world when the pointer is at `point`. `None` when this
    /// pointer position cannot be resolved (the caller keeps the previous result).
    pub fn delta(&self, point: Point) -> Option<Vec3> {
        let ray = self.view.ray(point);
        let moved = match self.start {
            Start::Axis(start) => {
                let axis = self.handle.axis()?;
                scale(axis, axis_parameter(self.origin, axis, ray)? - start)
            }
            Start::Plane(start) => sub(plane_hit(self.origin, self.view.forward, ray)?, start),
        };
        moved.iter().all(|c| c.is_finite()).then(|| vec3(moved))
    }
}

/// The node the gizmo moves: why a node cannot be moved is a typed reason, not a silent no-op.
pub fn movable(node: &PlanNode) -> Result<(), &'static str> {
    if !matches!(
        node.kind,
        NodeKind::Group | NodeKind::ImagePlane | NodeKind::Splat
    ) {
        return Err("not_movable");
    }
    if inverse3(&linear(&node.world_matrix)).is_none() {
        return Err("degenerate_transform");
    }
    Ok(())
}

/// The change of the node's `localPosition` that moves it by `delta` in the world:
/// `L · W⁻¹ · Δworld` (see the module docs).
pub fn local_delta(node: &PlanNode, delta: Vec3) -> Option<Vec3> {
    let world_inverse = inverse3(&linear(&node.world_matrix))?;
    let local = linear(&node.local_matrix);
    let moved = apply3(&local, apply3(&world_inverse, of(delta)));
    moved.iter().all(|c| c.is_finite()).then(|| vec3(moved))
}

pub fn world_origin(node: &PlanNode) -> V3 {
    of(node.world_matrix.translation())
}

/// Corners of an image plane on the picture, clockwise from the top left of the plane, or `None`
/// when any corner is behind the camera.
pub fn outline(view: &View, node: &PlanNode) -> Option<[Point; 4]> {
    let (width, height) = (node.width_scene_units?, node.height_scene_units?);
    let (hw, hh) = (width / 2.0, height / 2.0);
    let corners = [(-hw, hh), (hw, hh), (hw, -hh), (-hw, -hh)];
    let mut out = [Point { u: 0.0, v: 0.0 }; 4];
    for (slot, (x, y)) in out.iter_mut().zip(corners) {
        *slot = view.project(of(node.world_matrix.transform_point(vec3([x, y, 0.0]))))?;
    }
    Some(out)
}

/// Why no gizmo is shown, or `None` when there is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hidden {
    NoSelection,
    MultipleSelection,
    NotANode,
    NotInFrame,
    NotMovable(&'static str),
    BehindCamera,
}

impl Hidden {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoSelection => "no_selection",
            Self::MultipleSelection => "multiple_selection",
            Self::NotANode => "not_a_node",
            Self::NotInFrame => "not_in_frame",
            Self::NotMovable(reason) => reason,
            Self::BehindCamera => "behind_camera",
        }
    }
}

/// What a runtime draws, in picture coordinates. Version 1; fields are only ever added.
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    pub entity: Option<String>,
    pub layout: Option<Layout>,
    pub hidden: Option<Hidden>,
    pub outlines: Vec<(String, [Point; 4])>,
    /// The handle being dragged, and the gesture it edits.
    pub active: Option<(Handle, String)>,
    pub aspect: f64,
    /// Which scene, session, revision and frame this overlay was computed for (see
    /// `EditorState::gizmo_stamp`). The page echoes it in every pointer message, so a message
    /// that answers an older overlay can be told from one that answers this one.
    pub stamp: Value,
}

impl Overlay {
    pub fn to_json(&self) -> Value {
        let handles: Vec<Value> = self
            .layout
            .iter()
            .flat_map(|layout| layout.handles.iter())
            .map(|geometry| {
                json!({
                    "id": geometry.handle.as_str(),
                    "kind": if geometry.handle == Handle::Screen { "center" } else { "axis" },
                    "from": geometry.from.to_json(), "to": geometry.to.to_json(),
                })
            })
            .collect();
        json!({
            "version": 1,
            "visible": self.layout.is_some(),
            "reason": self.hidden.map(Hidden::as_str),
            "entity": self.entity,
            "origin": self.layout.as_ref().map(|layout| layout.origin.to_json()),
            "handles": handles,
            "outlines": self.outlines.iter().map(|(entity, quad)| json!({
                "entity": entity,
                "quad": quad.iter().map(|point| point.to_json()).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "active": self.active.as_ref().map(|(handle, gesture)| json!({
                "handle": handle.as_str(), "gesture": gesture,
            })),
            "aspect": self.aspect,
            "stamp": self.stamp,
            "metrics": {
                "handle_length": HANDLE_LENGTH, "hit_radius": HIT_RADIUS,
                "center_radius": CENTER_RADIUS, "unit": "picture_height",
            },
        })
    }
}
