//! F055 / BL015.1 (G1, first slice): the rotate tool of the gizmo and `scene.rotate`, through the
//! public library API and the JSON executor (in process: no window, no GPU, no Chrome, no GPUI,
//! no pointer device).
//!
//! Every expectation is computed **here**, with formulas written in this file (Rodrigues' matrix,
//! a quaternion to matrix, a pinhole camera), never with the code under test:
//!
//! - **Known rotations.** A quarter turn about each world axis lands on the numbers a reader can
//!   check by hand, and the sign is the right-hand rule (`+90°` about `+Z` takes `+X` to `+Y`).
//! - **Space.** `world` turns about the scene's axes, `local` about the node's own: after a
//!   first rotation they are different rotations, each equal to its own oracle.
//! - **Parents.** For a rotated and uniformly scaled parent the world matrix after the edit is
//!   exactly `G · W` (the pivot, the node's origin, does not move); an uneven or mirrored parent
//!   is refused, typed, with the document untouched.
//! - **Equivalence.** The document a ring drag leaves equals the one `scene.rotate` leaves for the
//!   `degrees`/`axis`/`space` the drag reported, bit for bit.
//! - **Lifecycle.** One undo entry per gesture, none for a no-op or a full turn, cancel and a
//!   refused move leave the document as it was, a refused move keeps the last legal candidate.
//! - **Not covered**: the canvas of the runtime, the window and a physical pointer
//!   (`runtime/test/gizmo-overlay.test.mjs`, the GPUI tests and the native proof).

use std::f64::consts::{FRAC_1_SQRT_2, PI};
use std::fs;
use std::path::Path;

use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::{Quat, Vec3};
use tessel_editor_core::scene_interaction::{FrameAt, GizmoCommand, GizmoSpace, GizmoTool, Handle};
use tessel_editor_core::scene_project::SceneCommand;
use tessel_editor_core::{Command, CommandError, EditorState, headless};

const MINIMAL: &str = "valid-minimal-continuous";
const FIGURE: &str = "scene/scene-a/node/figure";
const ROOT: &str = "scene/scene-a/node/root";
const CAMERA_START: &str = "camera/cam-main/start";

type M3 = [[f64; 3]; 3];
type V3 = [f64; 3];

fn bundle(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/migration/fixtures/bundles")
        .join(format!("{name}.bundle.json"));
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn run(state: &mut EditorState, request: Value) -> Result<Value, CommandError> {
    headless::execute(state, &request)
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    run(state, request.clone()).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn err(state: &mut EditorState, request: Value) -> CommandError {
    match run(state, request.clone()) {
        Ok(answer) => panic!("{request} should have failed, got {answer}"),
        Err(error) => error,
    }
}

fn street() -> EditorState {
    street_of(bundle(MINIMAL))
}

fn street_of(bundle: Value) -> EditorState {
    let mut state = EditorState::demo();
    ok(&mut state, json!({"op":"project.create","name":"Rotation"}));
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle}),
    );
    state
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

fn redo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["redo_depth"].as_u64().unwrap()
}

/// What a refused, cancelled or read-only interaction must leave as it was.
fn observable(state: &EditorState) -> (Value, Value, u64) {
    (
        state.to_document(None),
        state.history.to_json(),
        state.project.revision,
    )
}

fn document_of(state: &EditorState) -> Value {
    state.scenes.get("street").unwrap().scene.document().clone()
}

fn select(state: &mut EditorState, entity: &str) {
    ok(
        state,
        json!({"op":"scene.select","scene":"street","entities":[entity]}),
    );
}

fn tool(state: &mut EditorState, tool: &str, space: &str) {
    ok(
        state,
        json!({"op":"scene.gizmo.tool","tool":tool,"space":space}),
    );
}

fn overlay_at(state: &mut EditorState, frame: i64) -> Value {
    ok(
        state,
        json!({"op":"scene.gizmo.state","scene":"street","frame":frame}),
    )["overlay"]
        .clone()
}

fn overlay(state: &mut EditorState) -> Value {
    overlay_at(state, 0)
}

fn point(u: f64, v: f64) -> Value {
    json!({"u": u, "v": v})
}

fn handle<'a>(overlay: &'a Value, id: &str) -> &'a Value {
    overlay["handles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|handle| handle["id"] == id)
        .unwrap_or_else(|| panic!("no handle `{id}` in {overlay}"))
}

fn rotate_request(entity: &str, axis: &str, degrees: f64, space: &str) -> Value {
    json!({"op":"scene.rotate","scene":"street","entity":entity,"frame":0,
           "axis":axis,"degrees":degrees,"space":space})
}

fn reason(error: &CommandError) -> String {
    error.details.as_ref().unwrap()["reason"]
        .as_str()
        .unwrap()
        .to_string()
}

// --- the oracle -------------------------------------------------------------------------------

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(a: V3) -> V3 {
    let length = dot(a, a).sqrt();
    [a[0] / length, a[1] / length, a[2] / length]
}

fn matmul(a: &M3, b: &M3) -> M3 {
    let mut out = [[0.0; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            out[r][c] = (0..3).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    out
}

fn apply(m: &M3, v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

/// Rodrigues' rotation matrix: `radians` counter-clockwise about the unit `axis`.
fn rodrigues(axis: V3, radians: f64) -> M3 {
    let (s, c) = radians.sin_cos();
    let k = 1.0 - c;
    let [x, y, z] = axis;
    [
        [c + x * x * k, x * y * k - z * s, x * z * k + y * s],
        [y * x * k + z * s, c + y * y * k, y * z * k - x * s],
        [z * x * k - y * s, z * y * k + x * s, c + z * z * k],
    ]
}

/// The rotation matrix of a unit quaternion (the textbook formula, written here).
fn matrix_of(q: [f64; 4]) -> M3 {
    let [x, y, z, w] = q;
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

fn quat_about(axis: V3, degrees: f64) -> [f64; 4] {
    let axis = unit(axis);
    let (s, c) = (degrees.to_radians() / 2.0).sin_cos();
    [axis[0] * s, axis[1] * s, axis[2] * s, c]
}

fn quat_value(q: [f64; 4]) -> Value {
    json!({"x": q[0], "y": q[1], "z": q[2], "w": q[3]})
}

fn quat_of(value: &Value) -> [f64; 4] {
    [
        value["x"].as_f64().unwrap(),
        value["y"].as_f64().unwrap(),
        value["z"].as_f64().unwrap(),
        value["w"].as_f64().unwrap(),
    ]
}

/// The world matrix of a node at a frame, from the plan: (linear part row-major, translation).
fn world(state: &mut EditorState, node: &str, frame: i64) -> (M3, V3) {
    let plan = ok(
        state,
        json!({"op":"scene.plan","scene":"street","frame":frame}),
    )["plan"]
        .clone();
    let found = plan["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == node)
        .unwrap();
    let m: Vec<f64> = found["worldMatrix"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_f64().unwrap())
        .collect();
    let linear = [[m[0], m[4], m[8]], [m[1], m[5], m[9]], [m[2], m[6], m[10]]];
    (linear, [m[12], m[13], m[14]])
}

fn assert_matrix(actual: &M3, expected: &M3, what: &str) {
    for r in 0..3 {
        for c in 0..3 {
            assert!(
                near(actual[r][c], expected[r][c], 1e-9),
                "{what}: [{r}][{c}] {} vs {}\nactual {actual:?}\nexpected {expected:?}",
                actual[r][c],
                expected[r][c]
            );
        }
    }
}

fn assert_vec(actual: V3, expected: V3, what: &str) {
    for i in 0..3 {
        assert!(
            near(actual[i], expected[i], 1e-9),
            "{what}: {actual:?} vs {expected:?}"
        );
    }
}

fn node_field(state: &EditorState, node: &str, field: &str) -> Value {
    let document = document_of(state);
    document["sceneDefinitions"][0]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == node)
        .unwrap()[field]
        .clone()
}

fn figure_rotation(state: &EditorState) -> [f64; 4] {
    quat_of(&node_field(state, "figure", "localRotation"))
}

fn figure_position(state: &EditorState) -> Value {
    node_field(state, "figure", "localPosition")
}

fn set_root(state: &mut EditorState, rotation: Option<[f64; 4]>, scale: Option<[f64; 3]>) {
    let mut ops = Vec::new();
    if let Some(q) = rotation {
        ops.push(json!({"set": format!("{ROOT}/rotation"), "value": quat_value(q)}));
    }
    if let Some(s) = scale {
        ops.push(
            json!({"set": format!("{ROOT}/scale"), "value": {"x": s[0], "y": s[1], "z": s[2]}}),
        );
    }
    ok(state, json!({"op":"scene.edit","scene":"street","ops":ops}));
}

fn set_figure(state: &mut EditorState, rotation: Option<[f64; 4]>, scale: Option<[f64; 3]>) {
    let mut ops = Vec::new();
    if let Some(q) = rotation {
        ops.push(json!({"set": format!("{FIGURE}/rotation"), "value": quat_value(q)}));
    }
    if let Some(s) = scale {
        ops.push(
            json!({"set": format!("{FIGURE}/scale"), "value": {"x": s[0], "y": s[1], "z": s[2]}}),
        );
    }
    ok(state, json!({"op":"scene.edit","scene":"street","ops":ops}));
}

/// The axis a rotation turns about, in the world, as the contract defines it and written
/// independently: world = the scene's axis; local = the node's own axis without its own scale,
/// which is the parent's rotation times the authored one.
fn expected_axis(
    state: &mut EditorState,
    parent: Option<[f64; 4]>,
    axis: usize,
    space: &str,
) -> V3 {
    if space == "world" {
        let mut e = [0.0; 3];
        e[axis] = 1.0;
        return e;
    }
    let parent = parent.map_or(
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        matrix_of,
    );
    let own = matrix_of(figure_rotation(state));
    let oriented = matmul(&parent, &own);
    [oriented[0][axis], oriented[1][axis], oriented[2][axis]]
}

/// A pinhole camera written here (vertical field of view, +Y up): the oracle of where a point of
/// the world lands on the picture and which world ray a picture point is.
struct Camera {
    position: V3,
    target: V3,
    fov_deg: f64,
    aspect: f64,
}

fn camera(state: &mut EditorState, frame: i64) -> Camera {
    let plan = ok(
        state,
        json!({"op":"scene.plan","scene":"street","frame":frame}),
    )["plan"]
        .clone();
    let vec = |value: &Value| {
        [
            value["x"].as_f64().unwrap(),
            value["y"].as_f64().unwrap(),
            value["z"].as_f64().unwrap(),
        ]
    };
    Camera {
        position: vec(&plan["camera"]["position"]),
        target: vec(&plan["camera"]["target"]),
        fov_deg: plan["camera"]["fovDeg"].as_f64().unwrap(),
        aspect: plan["output"]["width"].as_f64().unwrap()
            / plan["output"]["height"].as_f64().unwrap(),
    }
}

impl Camera {
    fn basis(&self) -> (V3, V3, V3) {
        let forward = unit(sub(self.target, self.position));
        let right = unit(cross(forward, [0.0, 1.0, 0.0]));
        (forward, right, cross(right, forward))
    }

    fn project(&self, p: V3) -> (f64, f64) {
        let (forward, right, up) = self.basis();
        let relative = sub(p, self.position);
        let depth = dot(relative, forward);
        let half = (self.fov_deg.to_radians() / 2.0).tan();
        let x = dot(relative, right) / (depth * half * self.aspect);
        let y = dot(relative, up) / (depth * half);
        ((x + 1.0) / 2.0, (1.0 - y) / 2.0)
    }

    fn ray(&self, u: f64, v: f64) -> V3 {
        let (forward, right, up) = self.basis();
        let half = (self.fov_deg.to_radians() / 2.0).tan();
        let x = (u * 2.0 - 1.0) * half * self.aspect;
        let y = (1.0 - v * 2.0) * half;
        [
            forward[0] + right[0] * x + up[0] * y,
            forward[1] + right[1] * x + up[1] * y,
            forward[2] + right[2] * x + up[2] * y,
        ]
    }

    /// Where the eye ray through the picture point meets the plane through `origin` with `normal`.
    fn on_plane(&self, (u, v): (f64, f64), origin: V3, normal: V3) -> V3 {
        let ray = self.ray(u, v);
        let along = dot(sub(origin, self.position), normal) / dot(ray, normal);
        [
            self.position[0] + ray[0] * along,
            self.position[1] + ray[1] * along,
            self.position[2] + ray[2] * along,
        ]
    }

    /// The pointer position that, after grabbing the ring at `grabbed`, asks for a turn of
    /// `degrees` about `axis` through `origin`: the grabbed point of the ring plane follows.
    fn pointer_for(&self, grabbed: &Value, origin: V3, axis: V3, degrees: f64) -> (f64, f64) {
        let held = self.on_plane(
            (
                grabbed["u"].as_f64().unwrap(),
                grabbed["v"].as_f64().unwrap(),
            ),
            origin,
            axis,
        );
        let turned = apply(&rodrigues(axis, degrees.to_radians()), sub(held, origin));
        self.project([
            origin[0] + turned[0],
            origin[1] + turned[1],
            origin[2] + turned[2],
        ])
    }
}

/// A point on the ring (a visible vertex of its polygon) to press on, as a picture position.
fn on_ring(shown: &Value, id: &str) -> Value {
    let points = handle(shown, id)["points"].as_array().unwrap();
    // A quarter of the way round, where the ring is far from the other two and from the origin.
    points[points.len() / 8]
        .as_object()
        .map(|_| points[points.len() / 8].clone())
        .or_else(|| points.iter().find(|p| !p.is_null()).cloned())
        .unwrap()
}

fn press(state: &mut EditorState, at: Value) -> Result<Value, CommandError> {
    run(
        state,
        json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":at}),
    )
}

// --- the tool and the space ----------------------------------------------------------------------

#[test]
fn the_tool_defaults_to_translate_in_world_space_and_the_overlay_says_so() {
    let mut state = street();
    let tool = ok(&mut state, json!({"op":"scene.gizmo.tool"}));
    assert_eq!(tool["tool"], "translate");
    assert_eq!(tool["space"], "world");
    assert_eq!(tool["changed"], false);
    assert_eq!(tool["tools"], json!(["translate", "rotate", "scale"]));
    assert_eq!(tool["spaces"], json!(["world", "local"]));
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    assert_eq!(shown["tool"], "translate");
    assert_eq!(shown["space"], "world");
    // The translate tool is exactly what it was: three arrows and a centre, no ring.
    let kinds: Vec<&str> = shown["handles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["axis", "axis", "axis", "center"]);
    assert!(shown["handles"][0].get("points").is_none());
}

#[test]
fn choosing_the_tool_and_the_space_is_session_state_not_an_edit() {
    let mut state = street();
    select(&mut state, FIGURE);
    let before = observable(&state);
    let set = ok(
        &mut state,
        json!({"op":"scene.gizmo.tool","tool":"rotate","space":"local"}),
    );
    assert_eq!(
        (set["tool"].as_str(), set["space"].as_str()),
        (Some("rotate"), Some("local"))
    );
    assert_eq!(set["changed"], true);
    assert_eq!(set["previous"], json!({"tool":"translate","space":"world"}));
    assert_eq!(
        (
            state.scene_interaction.tool(),
            state.scene_interaction.space()
        ),
        (GizmoTool::Rotate, GizmoSpace::Local)
    );
    // Nothing of the project moved: no document change, no history entry, no revision.
    assert_eq!(observable(&state), before);
    // Setting the same value again is not a change; a partial request keeps the other field.
    assert_eq!(
        ok(&mut state, json!({"op":"scene.gizmo.tool","tool":"rotate"}))["changed"],
        false
    );
    let space_only = ok(&mut state, json!({"op":"scene.gizmo.tool","space":"world"}));
    assert_eq!(
        (space_only["tool"].as_str(), space_only["space"].as_str()),
        (Some("rotate"), Some("world"))
    );
    // The overlay follows the state.
    let shown = overlay(&mut state);
    assert_eq!(
        (shown["tool"].as_str(), shown["space"].as_str()),
        (Some("rotate"), Some("world"))
    );
    // Unknown names are typed errors that leave the state alone.
    for request in [
        json!({"op":"scene.gizmo.tool","tool":"resize"}),
        json!({"op":"scene.gizmo.tool","space":"screen"}),
        json!({"op":"scene.gizmo.tool","tool":7}),
    ] {
        let error = err(&mut state, request);
        assert!(
            ["invalid_value", "invalid_request"].contains(&error.code),
            "{error:?}"
        );
    }
    assert_eq!(state.scene_interaction.tool(), GizmoTool::Rotate);
    assert_eq!(observable(&state), before);
}

#[test]
fn the_tool_and_the_space_are_part_of_the_overlay_identity() {
    let mut state = street();
    select(&mut state, FIGURE);
    let translate = overlay(&mut state)["stamp"].clone();
    let key = state.gizmo_overlay_key("street", 0);
    tool(&mut state, "rotate", "world");
    let rotate = overlay(&mut state)["stamp"].clone();
    // A press made against the arrows must not be accepted for the rings: the stamp differs
    // although the project revision does not.
    assert_ne!(translate, rotate);
    assert_eq!(translate["revision"], rotate["revision"]);
    assert_ne!(key, state.gizmo_overlay_key("street", 0));
    let world_key = state.gizmo_overlay_key("street", 0);
    tool(&mut state, "rotate", "local");
    assert_ne!(overlay(&mut state)["stamp"], rotate);
    assert_ne!(world_key, state.gizmo_overlay_key("street", 0));
}

#[test]
fn the_tool_is_settable_like_any_control_through_set() {
    let mut state = street();
    // `set monitor.gizmo tool rotate` is the sentence a script and the window's controls send.
    let set = ok(
        &mut state,
        json!({"op":"set","target":"monitor.gizmo","field":"tool","value":"rotate"}),
    );
    assert_eq!(set["tool"], "rotate");
    assert_eq!(state.scene_interaction.tool(), GizmoTool::Rotate);
    ok(
        &mut state,
        json!({"op":"set","target":"monitor.gizmo","field":"space","value":"local"}),
    );
    assert_eq!(state.scene_interaction.space(), GizmoSpace::Local);
    let bad = err(
        &mut state,
        json!({"op":"set","target":"monitor.gizmo","field":"tool","value":"resize"}),
    );
    assert_eq!(bad.code, "invalid_value");
    assert_eq!(state.scene_interaction.tool(), GizmoTool::Rotate);
}

// --- known rotations -----------------------------------------------------------------------------

#[test]
fn a_quarter_turn_about_each_world_axis_lands_on_numbers_checked_by_hand() {
    // +90° about +Z takes +X to +Y; about +X takes +Y to +Z; about +Y takes +Z to +X (right-hand
    // rule). The figure has no parent rotation and the identity as its own.
    let half = FRAC_1_SQRT_2;
    for (axis, expected, image_of) in [
        (
            "x",
            [half, 0.0, 0.0, half],
            ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ),
        (
            "y",
            [0.0, half, 0.0, half],
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ),
        (
            "z",
            [0.0, 0.0, half, half],
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ),
    ] {
        let mut state = street();
        let answer = ok(&mut state, rotate_request(FIGURE, axis, 90.0, "world"));
        assert_eq!(answer["changed"], true, "{axis}");
        let written = figure_rotation(&state);
        for i in 0..4 {
            assert!(
                near(written[i], expected[i], 1e-12),
                "{axis}: {written:?} vs {expected:?}"
            );
        }
        // The same thing through the world matrix: where the node's own axis ends up.
        let (linear, _) = world(&mut state, "figure", 0);
        assert_vec(apply(&linear, image_of.0), image_of.1, axis);
    }
}

#[test]
fn the_angle_is_in_degrees_and_its_sign_is_the_right_hand_rule() {
    let mut forward = street();
    ok(&mut forward, rotate_request(FIGURE, "z", 30.0, "world"));
    let mut backward = street();
    ok(&mut backward, rotate_request(FIGURE, "z", -30.0, "world"));
    let (a, b) = (figure_rotation(&forward), figure_rotation(&backward));
    assert!(near(a[2], (15f64).to_radians().sin(), 1e-12));
    assert!(near(b[2], -(15f64).to_radians().sin(), 1e-12));
    assert!(near(a[3], b[3], 1e-12));
    // `0.5` degrees is a half degree, not half a radian.
    let mut small = street();
    ok(&mut small, rotate_request(FIGURE, "z", 0.5, "world"));
    assert!(near(
        figure_rotation(&small)[2],
        (0.25f64).to_radians().sin(),
        1e-12
    ));
}

#[test]
fn the_written_rotation_is_a_unit_quaternion_in_the_existing_format() {
    let mut state = street();
    ok(&mut state, rotate_request(FIGURE, "x", 37.0, "world"));
    ok(&mut state, rotate_request(FIGURE, "y", -71.0, "local"));
    ok(&mut state, rotate_request(FIGURE, "z", 123.0, "world"));
    let written = node_field(&state, "figure", "localRotation");
    // The document keeps `localRotation` as `{x, y, z, w}` and nothing else is added next to it.
    let object = written.as_object().unwrap();
    assert_eq!(object.len(), 4);
    let q = quat_of(&written);
    assert!(near(
        dot([q[0], q[1], q[2]], [q[0], q[1], q[2]]) + q[3] * q[3],
        1.0,
        1e-12
    ));
    let node = document_of(&state)["sceneDefinitions"][0]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "figure")
        .unwrap()
        .clone();
    let mut keys: Vec<&str> = node
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert!(
        !keys.contains(&"localEuler")
            && !keys.contains(&"rotation")
            && !keys.contains(&"localRotationDeg"),
        "{keys:?}"
    );
}

// --- local versus world ------------------------------------------------------------------------

#[test]
fn local_and_world_turn_about_different_axes_and_each_equals_its_own_oracle() {
    // The figure is first turned a quarter turn about +Y: its own X now points along world -Z.
    let start = quat_about([0.0, 1.0, 0.0], 90.0);
    let mut world_state = street();
    set_figure(&mut world_state, Some(start), None);
    let mut local_state = street();
    set_figure(&mut local_state, Some(start), None);
    let (before, origin) = world(&mut world_state, "figure", 0);

    ok(&mut world_state, rotate_request(FIGURE, "x", 30.0, "world"));
    ok(&mut local_state, rotate_request(FIGURE, "x", 30.0, "local"));
    let (after_world, origin_world) = world(&mut world_state, "figure", 0);
    let (after_local, origin_local) = world(&mut local_state, "figure", 0);

    // World: about the scene's +X. Local: about the node's own +X, which is world (0, 0, -1).
    let about_world_x = rodrigues([1.0, 0.0, 0.0], 30f64.to_radians());
    let about_own_x = rodrigues([0.0, 0.0, -1.0], 30f64.to_radians());
    assert_matrix(&after_world, &matmul(&about_world_x, &before), "world");
    assert_matrix(&after_local, &matmul(&about_own_x, &before), "local");
    // They are different rotations, not two spellings of one.
    let differ = (0..3).any(|r| (0..3).any(|c| !near(after_world[r][c], after_local[r][c], 1e-3)));
    assert!(differ, "local and world must not give the same node");
    // The pivot is the node's own origin in both.
    assert_vec(origin_world, origin, "world pivot");
    assert_vec(origin_local, origin, "local pivot");
    // A local turn about X is the authored quaternion times the turn: independent product.
    let local_written = quat_of(&node_field(&local_state, "figure", "localRotation"));
    let expected_matrix = matmul(
        &matrix_of(start),
        &rodrigues([1.0, 0.0, 0.0], 30f64.to_radians()),
    );
    assert_matrix(
        &matrix_of(local_written),
        &expected_matrix,
        "local = q · turn",
    );
}

#[test]
fn the_space_of_a_numeric_rotation_defaults_to_the_session_space_and_is_echoed() {
    let start = quat_about([0.0, 1.0, 0.0], 90.0);
    let mut by_default = street();
    set_figure(&mut by_default, Some(start), None);
    tool(&mut by_default, "translate", "local");
    let answer = ok(
        &mut by_default,
        json!({"op":"scene.rotate","scene":"street","entity":FIGURE,"frame":0,"axis":"x","degrees":30}),
    );
    assert_eq!(answer["space"], "local");
    let mut explicit = street();
    set_figure(&mut explicit, Some(start), None);
    ok(&mut explicit, rotate_request(FIGURE, "x", 30.0, "local"));
    assert_eq!(document_of(&by_default), document_of(&explicit));
    // And the other default.
    let mut world_default = street();
    set_figure(&mut world_default, Some(start), None);
    let answer = ok(
        &mut world_default,
        json!({"op":"scene.rotate","scene":"street","entity":FIGURE,"frame":0,"axis":"x","degrees":30}),
    );
    assert_eq!(answer["space"], "world");
}

#[test]
fn a_node_with_its_own_negative_and_uneven_scale_still_turns_about_its_unscaled_axes() {
    // The node's own scale is applied before its rotation, so it neither changes the axes of a
    // local rotation nor flips their sense: the oracle uses the authored rotation only.
    let start = quat_about([1.0, 1.0, 0.0], 50.0);
    let mut state = street();
    set_figure(&mut state, Some(start), Some([-1.0, 2.0, 3.0]));
    let (before, origin) = world(&mut state, "figure", 0);
    ok(&mut state, rotate_request(FIGURE, "z", 40.0, "local"));
    let (after, after_origin) = world(&mut state, "figure", 0);
    let axis = {
        let o = matrix_of(start);
        [o[0][2], o[1][2], o[2][2]]
    };
    assert_matrix(
        &after,
        &matmul(&rodrigues(axis, 40f64.to_radians()), &before),
        "negative scale",
    );
    assert_vec(after_origin, origin, "pivot");
    // The scale survived untouched.
    assert_eq!(
        node_field(&state, "figure", "localScale"),
        json!({"x":-1,"y":2,"z":3})
    );
}

// --- parents -------------------------------------------------------------------------------------

#[test]
fn a_rotated_and_uniformly_scaled_parent_gives_g_times_w_in_both_spaces() {
    let parent = quat_about([1.0, 2.0, 2.0], 70.0);
    for space in ["world", "local"] {
        for (index, axis) in ["x", "y", "z"].into_iter().enumerate() {
            let mut state = street();
            set_root(&mut state, Some(parent), Some([2.0, 2.0, 2.0]));
            set_figure(&mut state, Some(quat_about([0.0, 1.0, 1.0], 25.0)), None);
            let (before, origin) = world(&mut state, "figure", 0);
            let about = expected_axis(&mut state, Some(parent), index, space);
            ok(&mut state, rotate_request(FIGURE, axis, 33.0, space));
            let (after, after_origin) = world(&mut state, "figure", 0);
            assert_matrix(
                &after,
                &matmul(&rodrigues(about, 33f64.to_radians()), &before),
                &format!("{space} {axis}"),
            );
            // The node's origin did not move and the parent was not touched.
            assert_vec(after_origin, origin, &format!("{space} {axis} pivot"));
            assert_eq!(
                node_field(&state, "root", "localScale"),
                json!({"x":2,"y":2,"z":2})
            );
        }
    }
}

#[test]
fn the_children_of_a_rotated_node_turn_with_it_about_its_origin() {
    // The backdrop and the figure are siblings; turn a middle node instead: make the figure the
    // parent of nobody here, so use the root: rotating it carries both children.
    let mut state = street();
    let (backdrop_before, backdrop_origin) = world(&mut state, "backdrop", 0);
    let (root_before, root_origin) = world(&mut state, "root", 0);
    ok(&mut state, rotate_request(ROOT, "y", 90.0, "world"));
    let (backdrop_after, after_origin) = world(&mut state, "backdrop", 0);
    let g = rodrigues([0.0, 1.0, 0.0], PI / 2.0);
    assert_matrix(
        &backdrop_after,
        &matmul(&g, &backdrop_before),
        "child orientation",
    );
    // The child's origin turns about the parent's origin (the pivot).
    let moved = apply(&g, sub(backdrop_origin, root_origin));
    assert_vec(
        after_origin,
        [
            root_origin[0] + moved[0],
            root_origin[1] + moved[1],
            root_origin[2] + moved[2],
        ],
        "child origin",
    );
    assert_vec(world(&mut state, "root", 0).1, root_origin, "root pivot");
    let (root_after, _) = world(&mut state, "root", 0);
    assert_matrix(&root_after, &matmul(&g, &root_before), "root");
}

#[test]
fn a_parent_that_scales_unevenly_is_refused_typed_and_the_document_is_untouched() {
    let mut state = street();
    set_root(&mut state, None, Some([2.0, 1.0, 1.0]));
    select(&mut state, FIGURE);
    let before = observable(&state);
    for space in ["world", "local"] {
        let error = err(&mut state, rotate_request(FIGURE, "z", 30.0, space));
        assert_eq!(error.code, "not_rotatable", "{space}");
        assert_eq!(reason(&error), "non_uniform_parent_scale");
        assert_eq!(error.target.as_deref(), Some(FIGURE));
    }
    assert_eq!(observable(&state), before);
    // The overlay says why there are no rings instead of drawing a gizmo that refuses.
    tool(&mut state, "rotate", "world");
    let shown = overlay(&mut state);
    assert_eq!(shown["visible"], false);
    assert_eq!(shown["reason"], "non_uniform_parent_scale");
    assert_eq!(shown["entity"], FIGURE);
    // The translate tool on the same node still has its arrows: nothing regressed there.
    tool(&mut state, "translate", "world");
    assert_eq!(overlay(&mut state)["visible"], true);
}

#[test]
fn a_parent_that_shears_is_refused_like_one_that_scales_unevenly() {
    // A non-uniform scale under a rotation is a shear in the child's frame.
    let mut state = street();
    set_root(
        &mut state,
        Some(quat_about([0.0, 0.0, 1.0], 40.0)),
        Some([3.0, 1.0, 1.0]),
    );
    let error = err(&mut state, rotate_request(FIGURE, "y", 20.0, "world"));
    assert_eq!(reason(&error), "non_uniform_parent_scale");
}

#[test]
fn a_mirroring_parent_is_refused_typed() {
    for scale in [[-1.0, 1.0, 1.0], [-1.0, -1.0, -1.0], [1.0, -2.0, 1.0]] {
        let mut state = street();
        set_root(&mut state, None, Some(scale));
        let before = observable(&state);
        let error = err(&mut state, rotate_request(FIGURE, "z", 30.0, "world"));
        let expected = if scale[0] == scale[1] && scale[1] == scale[2] || scale == [-1.0, 1.0, 1.0]
        {
            "mirrored_parent"
        } else {
            "non_uniform_parent_scale"
        };
        assert_eq!(error.code, "not_rotatable", "{scale:?}");
        assert_eq!(reason(&error), expected, "{scale:?}");
        assert_eq!(observable(&state), before);
    }
}

#[test]
fn a_zero_scale_is_degenerate_whether_it_is_the_parent_s_or_the_node_s() {
    for (parent, own) in [
        ([0.0, 1.0, 1.0], None),
        ([1.0, 1.0, 1.0], Some([1.0, 0.0, 1.0])),
    ] {
        let mut state = street();
        set_root(&mut state, None, Some(parent));
        if let Some(scale) = own {
            set_figure(&mut state, None, Some(scale));
        }
        let before = observable(&state);
        let error = err(&mut state, rotate_request(FIGURE, "z", 30.0, "world"));
        assert_eq!(error.code, "not_rotatable");
        assert_eq!(reason(&error), "degenerate_transform");
        assert_eq!(observable(&state), before);
    }
}

#[test]
fn a_node_with_its_own_uneven_scale_is_rotated_and_keeps_its_scale() {
    let mut state = street();
    set_figure(&mut state, None, Some([2.0, 0.5, 3.0]));
    let (before, origin) = world(&mut state, "figure", 0);
    ok(&mut state, rotate_request(FIGURE, "y", 25.0, "world"));
    let (after, after_origin) = world(&mut state, "figure", 0);
    assert_matrix(
        &after,
        &matmul(&rodrigues([0.0, 1.0, 0.0], 25f64.to_radians()), &before),
        "own scale",
    );
    assert_vec(after_origin, origin, "pivot");
    assert_eq!(
        node_field(&state, "figure", "localScale"),
        json!({"x":2,"y":0.5,"z":3})
    );
}

// --- no-op, full turn, undo, redo ----------------------------------------------------------------

#[test]
fn zero_and_whole_turns_are_no_ops_with_no_history_and_no_revision() {
    for degrees in [0.0, 360.0, -360.0, 720.0, 1e-12] {
        let mut state = street();
        set_figure(&mut state, Some(quat_about([1.0, 2.0, 3.0], 77.0)), None);
        let before = observable(&state);
        let answer = ok(&mut state, rotate_request(FIGURE, "y", degrees, "local"));
        assert_eq!(answer["changed"], false, "{degrees}");
        assert_eq!(answer["document_changed"], false);
        assert_eq!(observable(&state), before, "{degrees}");
    }
}

#[test]
fn four_quarter_turns_come_back_and_each_is_its_own_undo_entry() {
    let mut state = street();
    set_figure(&mut state, Some(quat_about([1.0, 2.0, 3.0], 77.0)), None);
    let (start, origin) = world(&mut state, "figure", 0);
    let depth = undo_depth(&state);
    for step in 1..=4 {
        ok(&mut state, rotate_request(FIGURE, "z", 90.0, "world"));
        assert_eq!(undo_depth(&state), depth + step);
    }
    let (end, end_origin) = world(&mut state, "figure", 0);
    assert_matrix(&end, &start, "four quarter turns");
    assert_vec(end_origin, origin, "pivot");
    // 540° is a half turn: reduced, not wrapped to something else.
    let mut half = street();
    ok(&mut half, rotate_request(FIGURE, "x", 540.0, "world"));
    let mut direct = street();
    ok(&mut direct, rotate_request(FIGURE, "x", 180.0, "world"));
    assert_matrix(
        &world(&mut half, "figure", 0).0,
        &world(&mut direct, "figure", 0).0,
        "540 = 180",
    );
}

#[test]
fn undo_and_redo_restore_the_exact_document() {
    let mut state = street();
    let original = document_of(&state);
    ok(&mut state, rotate_request(FIGURE, "y", 41.0, "world"));
    let rotated = document_of(&state);
    assert_ne!(original, rotated);
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(document_of(&state), original);
    assert_eq!(redo_depth(&state), 1);
    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(document_of(&state), rotated);
}

// --- refusals leave no trace ---------------------------------------------------------------------

#[test]
fn what_cannot_be_rotated_is_refused_atomically_with_a_typed_code() {
    let mut state = street();
    let before = observable(&state);
    let cases: Vec<(Value, &str)> = vec![
        (
            rotate_request("camera/cam-main", "z", 10.0, "world"),
            "not_rotatable",
        ),
        (
            rotate_request("asset/figure", "z", 10.0, "world"),
            "not_rotatable",
        ),
        (
            rotate_request("scene/scene-a/node/ghost", "z", 10.0, "world"),
            "unknown_entity",
        ),
        (
            rotate_request("not an address", "z", 10.0, "world"),
            "invalid_entity_path",
        ),
        (
            rotate_request(FIGURE, "screen", 10.0, "world"),
            "invalid_value",
        ),
        (rotate_request(FIGURE, "w", 10.0, "world"), "invalid_value"),
        (
            rotate_request(FIGURE, "z", 10.0, "sideways"),
            "invalid_value",
        ),
        (
            json!({"op":"scene.rotate","scene":"nowhere","entity":FIGURE,"frame":0,"axis":"z","degrees":1}),
            "unknown_element",
        ),
        (
            json!({"op":"scene.rotate","scene":"street","entity":FIGURE,"axis":"z","degrees":1}),
            "invalid_request",
        ),
        (
            json!({"op":"scene.rotate","scene":"street","entity":FIGURE,"frame":0,"axis":"z"}),
            "invalid_request",
        ),
        (
            json!({"op":"scene.rotate","scene":"street","entity":FIGURE,"frame":0,"axis":"z","degrees":"ten"}),
            "invalid_request",
        ),
    ];
    for (request, code) in cases {
        assert_eq!(err(&mut state, request.clone()).code, code, "{request}");
        assert_eq!(observable(&state), before, "{request}");
    }
}

#[test]
fn a_non_finite_angle_through_the_typed_api_is_refused_without_a_trace() {
    let mut state = street();
    let before = observable(&state);
    for degrees in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = state
            .execute(Command::Scene(SceneCommand::Gizmo(GizmoCommand::Rotate {
                scene: "street".into(),
                entity: FIGURE.into(),
                at: FrameAt::Frame(0),
                axis: Handle::Z,
                degrees,
                space: None,
            })))
            .unwrap_err();
        assert_eq!(error.code, "invalid_value", "{degrees}");
    }
    assert_eq!(observable(&state), before);
}

#[test]
fn a_near_unit_authored_quaternion_is_normalised_when_rotated() {
    // The importer keeps a quaternion near unit length; the gizmo composes with the normalised
    // one but a zero quaternion cannot be edited at all. A scene with a slightly long quaternion
    // still rotates and writes a unit one.
    let mut bundle = bundle(MINIMAL);
    let nodes = bundle["sceneDefinitions"][0]["nodes"]
        .as_array_mut()
        .unwrap();
    let figure = nodes.iter_mut().find(|n| n["id"] == "figure").unwrap();
    figure["localRotation"] = json!({"x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0000001});
    let mut state = street_of(bundle);
    ok(&mut state, rotate_request(FIGURE, "z", 90.0, "world"));
    let q = figure_rotation(&state);
    assert!(
        near(q.iter().map(|c| c * c).sum::<f64>(), 1.0, 1e-12),
        "{q:?}"
    );
}

// --- the ring drag -----------------------------------------------------------------------------

fn drag_ring(
    state: &mut EditorState,
    axis: &str,
    parent: Option<[f64; 4]>,
    degrees: f64,
    space: &str,
) -> (Value, Value) {
    select(state, FIGURE);
    tool(state, "rotate", space);
    let camera = camera(state, 0);
    let (_, origin) = world(state, "figure", 0);
    let index = ["x", "y", "z"]
        .iter()
        .position(|name| *name == axis)
        .unwrap();
    let about = expected_axis(state, parent, index, space);
    let shown = overlay(state);
    let grabbed = on_ring(&shown, axis);
    let pressed = press(state, grabbed.clone()).unwrap();
    assert_eq!(pressed["dragging"], true, "{pressed}");
    assert_eq!(pressed["hit"]["handle"], axis);
    assert_eq!(pressed["tool"], "rotate");
    let (u, v) = camera.pointer_for(&grabbed, origin, about, degrees);
    let moved = ok(
        state,
        json!({"op":"scene.pointer.move","point":point(u, v)}),
    );
    assert_eq!(moved["applied"], true);
    assert_eq!(moved["document_changed"], false);
    let up = ok(state, json!({"op":"scene.pointer.up","point":point(u, v)}));
    (moved, up)
}

#[test]
fn dragging_a_ring_turns_the_node_by_the_angle_the_pointer_swept_in_both_spaces() {
    let parent = quat_about([1.0, 2.0, 2.0], 70.0);
    for space in ["world", "local"] {
        for axis in ["x", "y", "z"] {
            let mut state = street();
            set_root(&mut state, Some(parent), Some([2.0, 2.0, 2.0]));
            set_figure(&mut state, Some(quat_about([0.0, 1.0, 1.0], 25.0)), None);
            let (before, origin) = world(&mut state, "figure", 0);
            let depth = undo_depth(&state);
            let index = ["x", "y", "z"]
                .iter()
                .position(|name| *name == axis)
                .unwrap();
            let about = expected_axis(&mut state, Some(parent), index, space);
            let (_, up) = drag_ring(&mut state, axis, Some(parent), 35.0, space);
            // The angle the core reports is the one the pointer swept, within the numerical noise
            // of the picture (the oracle projects with a pinhole of its own).
            let swept = up["degrees"].as_f64().unwrap();
            assert!(near(swept, 35.0, 1e-6), "{space} {axis}: {swept}");
            assert_eq!(up["axis"], axis);
            assert_eq!(up["space"], space);
            assert_eq!(up["tool"], "rotate");
            assert_eq!(up["changed"], true);
            assert_eq!(
                undo_depth(&state),
                depth + 1,
                "one undo entry for the whole drag"
            );
            let (after, after_origin) = world(&mut state, "figure", 0);
            assert_matrix(
                &after,
                &matmul(&rodrigues(about, swept.to_radians()), &before),
                &format!("{space} {axis}"),
            );
            assert_vec(after_origin, origin, "pivot");
        }
    }
}

#[test]
fn a_negative_sweep_turns_the_other_way_and_the_angle_wraps_at_a_half_turn() {
    let mut state = street();
    let (_, up) = drag_ring(&mut state, "z", None, -50.0, "world");
    assert!(near(up["degrees"].as_f64().unwrap(), -50.0, 1e-6));
    let q = figure_rotation(&state);
    assert!(
        q[2] < 0.0,
        "a negative sweep about +Z has a negative z component: {q:?}"
    );
    // 170° swept the long way is reported as 170°, and 190° as -170° (the other side of the wrap),
    // never as 190°: a drag spans (-180, 180].
    for (sweep, expected) in [(170.0, 170.0), (190.0, -170.0)] {
        let mut state = street();
        let (_, up) = drag_ring(&mut state, "z", None, sweep, "world");
        assert!(
            near(up["degrees"].as_f64().unwrap(), expected, 1e-6),
            "{sweep}"
        );
    }
}

#[test]
fn a_ring_drag_and_scene_rotate_leave_the_same_document_in_both_spaces() {
    let parent = quat_about([2.0, 1.0, 0.5], 55.0);
    for space in ["world", "local"] {
        for axis in ["x", "y", "z"] {
            let prepare = |state: &mut EditorState| {
                set_root(state, Some(parent), Some([1.5, 1.5, 1.5]));
                set_figure(
                    state,
                    Some(quat_about([0.3, 1.0, -0.4], 62.0)),
                    Some([1.0, 2.0, 0.7]),
                );
            };
            let mut dragged = street();
            prepare(&mut dragged);
            let (_, up) = drag_ring(&mut dragged, axis, Some(parent), 41.5, space);
            let mut scripted = street();
            prepare(&mut scripted);
            let rotated = ok(
                &mut scripted,
                json!({"op":"scene.rotate","scene":"street","entity":FIGURE,"frame":0,
                       "axis":up["axis"],"degrees":up["degrees"],"space":up["space"]}),
            );
            assert_eq!(rotated["changed"], true);
            assert_eq!(rotated["rotation"], up["rotation"], "{space} {axis}");
            assert_eq!(
                document_of(&dragged),
                document_of(&scripted),
                "{space} {axis}"
            );
            assert_eq!(undo_depth(&dragged), undo_depth(&scripted));
            ok(&mut scripted, json!({"op":"history.undo"}));
            ok(&mut dragged, json!({"op":"history.undo"}));
            assert_eq!(document_of(&dragged), document_of(&scripted));
        }
    }
}

#[test]
fn many_moves_are_one_entry_and_each_angle_is_computed_from_the_press() {
    let mut state = street();
    select(&mut state, FIGURE);
    tool(&mut state, "rotate", "world");
    let camera = camera(&mut state, 0);
    let (_, origin) = world(&mut state, "figure", 0);
    let shown = overlay(&mut state);
    let grabbed = on_ring(&shown, "y");
    let depth = undo_depth(&state);
    press(&mut state, grabbed.clone()).unwrap();
    let mut last = None;
    for degrees in [10.0, 80.0, -40.0, 25.0, 60.0] {
        let (u, v) = camera.pointer_for(&grabbed, origin, [0.0, 1.0, 0.0], degrees);
        last = Some(ok(
            &mut state,
            json!({"op":"scene.pointer.move","point":point(u, v)}),
        ));
        // The document did not change while the candidate moves.
        assert_eq!(undo_depth(&state), depth);
        assert_eq!(figure_rotation(&state), [0.0, 0.0, 0.0, 1.0]);
    }
    let up = ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(undo_depth(&state), depth + 1);
    assert!(near(up["degrees"].as_f64().unwrap(), 60.0, 1e-6));
    assert_eq!(
        up["degrees"],
        last.unwrap()["degrees"],
        "the release commits the last candidate"
    );
    // The result is the one a single move to the last pointer gives: nothing accumulated.
    let mut single = street();
    let (_, single_up) = drag_ring(&mut single, "y", None, 60.0, "world");
    assert!(near(
        single_up["degrees"].as_f64().unwrap(),
        up["degrees"].as_f64().unwrap(),
        1e-9
    ));
}

#[test]
fn a_move_the_core_refuses_keeps_the_last_legal_candidate() {
    let mut state = street();
    select(&mut state, FIGURE);
    tool(&mut state, "rotate", "world");
    let camera = camera(&mut state, 0);
    let (_, origin) = world(&mut state, "figure", 0);
    let shown = overlay(&mut state);
    let grabbed = on_ring(&shown, "z");
    let depth = undo_depth(&state);
    press(&mut state, grabbed.clone()).unwrap();
    let (u, v) = camera.pointer_for(&grabbed, origin, [0.0, 0.0, 1.0], 30.0);
    let good = ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(u, v)}),
    );
    // A pointer position far outside the picture is a coordinate bug: refused, typed.
    let error = err(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(1e6, 0.5)}),
    );
    assert_eq!(error.code, "invalid_value");
    let up = ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(
        up["degrees"], good["degrees"],
        "the refused move did not replace the candidate"
    );
    assert_eq!(up["rotation"], good["rotation"]);
    assert_eq!(undo_depth(&state), depth + 1);
}

#[test]
fn cancel_discards_a_ring_drag_without_a_trace() {
    for reason in [
        "escape",
        "blur",
        "pointer_cancel",
        "hidden",
        "unavailable",
        "api",
    ] {
        let mut state = street();
        select(&mut state, FIGURE);
        tool(&mut state, "rotate", "local");
        let camera = camera(&mut state, 0);
        let (_, origin) = world(&mut state, "figure", 0);
        let shown = overlay(&mut state);
        let grabbed = on_ring(&shown, "x");
        let before = observable(&state);
        press(&mut state, grabbed.clone()).unwrap();
        let (u, v) = camera.pointer_for(&grabbed, origin, [1.0, 0.0, 0.0], 45.0);
        ok(
            &mut state,
            json!({"op":"scene.pointer.move","point":point(u, v)}),
        );
        let cancelled = ok(
            &mut state,
            json!({"op":"scene.pointer.cancel","reason":reason}),
        );
        assert_eq!(cancelled["changed"], true, "{reason}");
        assert_eq!(cancelled["document_changed"], false);
        assert_eq!(observable(&state), before, "{reason}");
        assert!(state.scene_interaction.drag().is_none());
        // The next press starts clean.
        let again = on_ring(&overlay(&mut state), "x");
        assert_eq!(press(&mut state, again).unwrap()["dragging"], true);
    }
}

#[test]
fn a_press_released_without_moving_is_not_an_edit_and_a_zero_sweep_is_not_either() {
    let mut state = street();
    select(&mut state, FIGURE);
    tool(&mut state, "rotate", "world");
    let shown = overlay(&mut state);
    let before = observable(&state);
    press(&mut state, on_ring(&shown, "z")).unwrap();
    let up = ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(up["changed"], false);
    assert_eq!(observable(&state), before);
    // A move back onto the press point is a zero sweep: the candidate equals the base.
    let grabbed = on_ring(&shown, "z");
    press(&mut state, grabbed.clone()).unwrap();
    ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":grabbed}),
    );
    let up = ok(&mut state, json!({"op":"scene.pointer.up","point":grabbed}));
    assert_eq!(up["changed"], false);
    assert_eq!(observable(&state), before);
}

#[test]
fn the_ring_hit_test_follows_the_space_and_ignores_the_arrows_and_the_centre() {
    // After a first turn about +Y the node's own X ring is a different circle than the world's.
    let mut state = street();
    set_figure(&mut state, Some(quat_about([0.0, 1.0, 0.0], 90.0)), None);
    select(&mut state, FIGURE);
    tool(&mut state, "rotate", "world");
    let world_shown = overlay(&mut state);
    tool(&mut state, "rotate", "local");
    let local_shown = overlay(&mut state);
    assert_ne!(
        handle(&world_shown, "x")["points"],
        handle(&local_shown, "x")["points"]
    );
    // The world X ring is the local Z ring (the node was turned a quarter turn about Y).
    let (world_x, local_z) = (
        handle(&world_shown, "x")["points"].as_array().unwrap(),
        handle(&local_shown, "z")["points"].as_array().unwrap(),
    );
    let same_circle = world_x.iter().all(|a| {
        a.is_null()
            || local_z.iter().any(|b| {
                !b.is_null()
                    && near(a["u"].as_f64().unwrap(), b["u"].as_f64().unwrap(), 1e-9)
                    && near(a["v"].as_f64().unwrap(), b["v"].as_f64().unwrap(), 1e-9)
            })
    });
    assert!(
        same_circle,
        "world X and local Z are the same circle after a quarter turn about Y"
    );
    // A press on a ring point finds that ring in each space.
    for (space, shown) in [("world", &world_shown), ("local", &local_shown)] {
        tool(&mut state, "rotate", space);
        let hit = ok(
            &mut state,
            json!({"op":"scene.gizmo.hit","scene":"street","frame":0,"point":on_ring(shown, "y")}),
        );
        assert_eq!(hit["hit"]["kind"], "handle", "{space}");
        assert_eq!(hit["hit"]["handle"], "y", "{space}");
    }
    // The centre of the node is the free-move handle of the translate tool and no handle of the
    // rotate tool (a ring seen edge-on may still pass through it, but never `screen`).
    let centre = local_shown["origin"].clone();
    tool(&mut state, "translate", "world");
    let hit = ok(
        &mut state,
        json!({"op":"scene.gizmo.hit","scene":"street","frame":0,"point":centre}),
    );
    assert_eq!(hit["hit"]["handle"], "screen");
    tool(&mut state, "rotate", "world");
    let hit = ok(
        &mut state,
        json!({"op":"scene.gizmo.hit","scene":"street","frame":0,"point":centre}),
    );
    assert_ne!(hit["hit"]["handle"], "screen");
}

#[test]
fn the_rotate_overlay_has_three_closed_rings_and_the_metrics_to_draw_them() {
    let mut state = street();
    select(&mut state, FIGURE);
    tool(&mut state, "rotate", "world");
    let shown = overlay(&mut state);
    assert_eq!(shown["visible"], true);
    let ids: Vec<&str> = shown["handles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["x", "y", "z"]);
    for ring in shown["handles"].as_array().unwrap() {
        assert_eq!(ring["kind"], "ring");
        let points = ring["points"].as_array().unwrap();
        assert_eq!(points.len(), 65, "64 segments, closed");
        assert_eq!(points.first(), points.last());
        assert!(
            points
                .iter()
                .all(|p| p.is_null() || (p["u"].is_f64() && p["v"].is_f64()))
        );
        assert_eq!(ring["from"], shown["origin"]);
    }
    assert_eq!(shown["metrics"]["ring_radius"], 0.1);
    // The outline of the selected plane is still there.
    assert_eq!(shown["outlines"].as_array().unwrap().len(), 1);
}

#[test]
fn every_ring_closes_on_its_first_point_bit_for_bit_in_any_view_space_and_pose() {
    // BL167.3. The polygon of a ring ends where it starts. The last angle is `TAU`, the `f64`
    // nearest to 2π, whose sine is -2.4e-16, not 0: whether that residue changed the projected
    // last point depended on the libm and on the view, so the closure held on one platform and
    // not on another. The invariant is checked across views, spaces and poses, not for one shot.
    let cameras = [
        ([0.0, 0.0, 8.0], [0.0, 0.0, 0.0]),
        ([3.0, 2.0, 7.0], [0.0, 0.5, 0.0]),
        ([-4.5, 1.25, 6.0], [0.3, 0.0, -0.2]),
        ([0.7, 5.0, 4.0], [0.0, 0.0, 0.0]),
    ];
    let poses = [
        None,
        Some(quat_about([0.0, 1.0, 0.0], 90.0)),
        Some(quat_about([1.0, 2.0, 0.5], 40.0)),
        Some(quat_about([0.3, 1.0, -0.4], 33.0)),
    ];
    let mut rings = 0;
    for (position, target) in cameras {
        for pose in poses {
            for space in ["world", "local"] {
                let mut state = street();
                ok(
                    &mut state,
                    json!({"op":"scene.edit","scene":"street","ops":[
                    {"set": format!("{CAMERA_START}/position"),
                     "value": {"x": position[0], "y": position[1], "z": position[2]}},
                    {"set": format!("{CAMERA_START}/target"),
                     "value": {"x": target[0], "y": target[1], "z": target[2]}}]}),
                );
                set_figure(&mut state, pose, None);
                select(&mut state, FIGURE);
                tool(&mut state, "rotate", space);
                let shown = overlay(&mut state);
                for ring in shown["handles"].as_array().unwrap() {
                    let points = ring["points"].as_array().unwrap();
                    assert_eq!(points.len(), 65, "{position:?} {space} {pose:?}");
                    assert_eq!(
                        points.first(),
                        points.last(),
                        "ring {} is open: camera {position:?} space {space} pose {pose:?}",
                        ring["id"]
                    );
                    rings += 1;
                }
            }
        }
    }
    assert_eq!(rings, 4 * 4 * 2 * 3, "every ring was checked");
}

#[test]
fn a_ring_seen_edge_on_does_not_open_a_drag() {
    // A camera at the height of the figure looks at it along the plane of the Y ring: the
    // pointer says nothing about the angle there.
    let mut state = street();
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set": format!("{CAMERA_START}/position"), "value": {"x": 0, "y": 0, "z": 8}},
            {"set": format!("{CAMERA_START}/target"), "value": {"x": 0, "y": 0, "z": 0}}]}),
    );
    select(&mut state, FIGURE);
    tool(&mut state, "rotate", "world");
    let shown = overlay(&mut state);
    let before = observable(&state);
    let pressed = press(&mut state, on_ring(&shown, "y")).unwrap();
    assert_eq!(pressed["dragging"], false);
    assert_eq!(pressed["refused"], "degenerate_axis");
    assert!(state.scene_interaction.drag().is_none());
    assert_eq!(observable(&state), before, "no gesture was opened");
    assert_eq!(
        ok(&mut state, json!({"op":"scene.gesture.status"}))["active"],
        false
    );
}

#[test]
fn a_ring_seen_almost_edge_on_is_refused_and_a_clearer_view_of_it_opens_a_drag() {
    // The band between "exactly edge-on" (the plane is parallel to the ray: no hit at all) and a
    // usable view: the eye ray is a few degrees off the plane of the Y ring, so the plane still
    // gets hit, but a pixel of pointer would swing the angle wildly. `GRAZING` (|cos| < 0.05)
    // refuses it. A camera 0.3 above the figure at distance 8 is at |cos| ≈ 0.037 (refused); one
    // 1.0 above is at ≈ 0.124 (a drag). The press is on the vertex of the ring on the +X axis, a
    // point only the Y and Z rings share (a tie goes to Y).
    for (height, opens) in [(0.3, false), (1.0, true)] {
        let mut state = street();
        ok(
            &mut state,
            json!({"op":"scene.edit","scene":"street","ops":[
                {"set": format!("{CAMERA_START}/position"), "value": {"x": 0, "y": height, "z": 8}},
                {"set": format!("{CAMERA_START}/target"), "value": {"x": 0, "y": 0, "z": 0}}]}),
        );
        select(&mut state, FIGURE);
        tool(&mut state, "rotate", "world");
        let shown = overlay(&mut state);
        let on_axis = handle(&shown, "y")["points"][0].clone();
        assert!(on_axis.is_object(), "the ring has a point on the +X axis");
        let before = observable(&state);
        let pressed = press(&mut state, on_axis).unwrap();
        if opens {
            assert_eq!(pressed["dragging"], true, "height {height}: {pressed}");
            assert_eq!(pressed["handle"], "y", "height {height}");
            ok(&mut state, json!({"op":"scene.pointer.cancel"}));
        } else {
            assert_eq!(pressed["dragging"], false, "height {height}: {pressed}");
            assert_eq!(pressed["refused"], "degenerate_axis", "height {height}");
            assert!(state.scene_interaction.drag().is_none());
            assert_eq!(observable(&state), before, "no gesture was opened");
        }
    }
}

// --- the tool during a drag, other gestures, the old tool ----------------------------------------

#[test]
fn the_tool_cannot_change_under_an_open_drag_and_the_drag_is_left_alone() {
    let mut state = street();
    select(&mut state, FIGURE);
    tool(&mut state, "rotate", "world");
    let shown = overlay(&mut state);
    press(&mut state, on_ring(&shown, "z")).unwrap();
    let before = observable(&state);
    for request in [
        json!({"op":"scene.gizmo.tool","tool":"translate"}),
        json!({"op":"scene.gizmo.tool","space":"local"}),
    ] {
        assert_eq!(err(&mut state, request).code, "gesture_active");
    }
    assert!(state.scene_interaction.drag().is_some());
    assert_eq!(state.scene_interaction.tool(), GizmoTool::Rotate);
    assert_eq!(observable(&state), before);
    // Asking for what is already set is a read and is allowed.
    assert_eq!(
        ok(
            &mut state,
            json!({"op":"scene.gizmo.tool","tool":"rotate","space":"world"})
        )["changed"],
        false
    );
    ok(&mut state, json!({"op":"scene.pointer.cancel"}));
    tool(&mut state, "translate", "world");
}

#[test]
fn the_translation_drag_is_what_it_was_after_visiting_the_rotate_tool() {
    let mut state = street();
    select(&mut state, FIGURE);
    tool(&mut state, "rotate", "local");
    tool(&mut state, "translate", "local");
    let shown = overlay(&mut state);
    let camera = camera(&mut state, 0);
    let (_, origin) = world(&mut state, "figure", 0);
    let tip = handle(&shown, "x")["to"].clone();
    let pressed = press(&mut state, tip).unwrap();
    assert_eq!(pressed["dragging"], true);
    assert_eq!(pressed["tool"], "translate");
    let (u, v) = camera.project([origin[0] + 0.5, origin[1], origin[2]]);
    ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(u, v)}),
    );
    let up = ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(up["changed"], true);
    assert!(up.get("degrees").is_none());
    // A translation never touches the rotation and `scene.translate` still gives the same document.
    assert_eq!(figure_rotation(&state), [0.0, 0.0, 0.0, 1.0]);
    let mut scripted = street();
    ok(
        &mut scripted,
        json!({"op":"scene.translate","scene":"street","entity":FIGURE,"frame":0,"delta":up["world_delta"]}),
    );
    assert_eq!(document_of(&state), document_of(&scripted));
    assert_eq!(figure_position(&state), figure_position(&scripted));
}

#[test]
fn a_ring_press_while_another_gesture_is_open_is_refused_and_a_late_release_is_inert() {
    let mut state = street();
    select(&mut state, FIGURE);
    tool(&mut state, "rotate", "world");
    let shown = overlay(&mut state);
    ok(
        &mut state,
        json!({"op":"scene.gesture.begin","scene":"street","targets":[FIGURE]}),
    );
    let before = observable(&state);
    assert_eq!(
        press(&mut state, on_ring(&shown, "z")).unwrap_err().code,
        "gesture_active"
    );
    assert_eq!(observable(&state), before);
    let open = state.scene_interaction.gesture().unwrap().id().to_string();
    ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":open}),
    );

    // A release with no ring drag open does not commit somebody else's slider gesture. The
    // slider needs the demo project's selected clip, so this part uses the demo project.
    let mut state = EditorState::demo();
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle(MINIMAL)}),
    );
    select(&mut state, FIGURE);
    tool(&mut state, "rotate", "world");
    ok(
        &mut state,
        json!({"op":"property.gesture.begin","property":"zoom"}),
    );
    ok(
        &mut state,
        json!({"op":"property.gesture.update","value":120}),
    );
    let release = ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(release["noop"], true);
    assert!(
        state.open_property_gesture().is_some(),
        "the slider gesture is still open"
    );
}

#[test]
fn a_ring_drag_does_not_survive_an_edit_under_it() {
    let mut state = street();
    select(&mut state, FIGURE);
    tool(&mut state, "rotate", "world");
    let camera = camera(&mut state, 0);
    let (_, origin) = world(&mut state, "figure", 0);
    let shown = overlay(&mut state);
    let grabbed = on_ring(&shown, "z");
    let depth = undo_depth(&state);
    press(&mut state, grabbed.clone()).unwrap();
    ok(&mut state, rotate_request(FIGURE, "x", 10.0, "world"));
    let after_edit = observable(&state);
    let (u, v) = camera.pointer_for(&grabbed, origin, [0.0, 0.0, 1.0], 20.0);
    let error = err(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(u, v)}),
    );
    assert_eq!(error.code, "gesture_invalidated");
    assert!(state.gizmo_drag_is_dead());
    // The release is refused too and the stale candidate never reaches the document.
    let _ = run(
        &mut state,
        json!({"op":"scene.pointer.up","point":point(u, v)}),
    );
    assert_eq!(
        figure_rotation(&state),
        quat_of(&node_field(&state, "figure", "localRotation"))
    );
    assert_eq!(state.to_document(None), after_edit.0);
    assert_eq!(
        undo_depth(&state),
        depth + 1,
        "only the scripted edit made an entry"
    );
}

// --- animation -----------------------------------------------------------------------------------

fn animated(channels: Value) -> Value {
    let mut bundle = bundle(MINIMAL);
    bundle["performanceTracks"] =
        json!([{"id": "perf-1", "instanceId": "inst-a", "channels": channels}]);
    bundle["sceneInstances"][0]["performanceTrackIds"] = json!(["perf-1"]);
    bundle
}

fn swing(node: &str) -> Value {
    json!({"kind": "cycle", "channelId": format!("swing-{node}"), "nodeId": node,
           "property": "localRotation", "periodFrames": 48, "amplitude": 25,
           "axis": {"x": 0, "y": 0, "z": 1}, "phaseOffsetFrames": 0})
}

#[test]
fn a_node_whose_rotation_is_animated_is_refused_and_nothing_is_keyed() {
    let mut state = street_of(animated(json!([swing("figure")])));
    select(&mut state, FIGURE);
    let before = observable(&state);
    let tracks = document_of(&state)["performanceTracks"].clone();
    let error = err(&mut state, rotate_request(FIGURE, "z", 10.0, "world"));
    assert_eq!(error.code, "not_rotatable");
    assert_eq!(reason(&error), "animated_rotation");
    assert_eq!(observable(&state), before);
    // The rotate tool says so instead of drawing rings, and a press cannot open a drag.
    tool(&mut state, "rotate", "world");
    let shown = overlay_at(&mut state, 5);
    assert_eq!(shown["visible"], false);
    assert_eq!(shown["reason"], "animated_rotation");
    let mut shown_at_rest = overlay_at(&mut state, 0);
    assert_eq!(
        shown_at_rest["reason"], "animated_rotation",
        "an animated channel is animated at every frame: {shown_at_rest}"
    );
    shown_at_rest = overlay_at(&mut state, 5);
    assert_eq!(shown_at_rest["visible"], false);
    let hit = ok(
        &mut state,
        json!({"op":"scene.gizmo.hit","scene":"street","frame":5,"point":point(0.5, 0.5)}),
    );
    assert_ne!(hit["hit"]["kind"], "handle");
    // The document's tracks are exactly what they were: no key, no channel, no interpolation.
    assert_eq!(document_of(&state)["performanceTracks"], tracks);
}

#[test]
fn another_node_of_an_animated_scene_and_a_swinging_parent_can_still_be_rotated() {
    // The root swings (a channel on its rotation); the figure has none. At frame 30 the figure's
    // world matrix includes the swing, and the edit is exact there: W' = G·W.
    const FRAME: i64 = 30;
    let mut state = street_of(animated(json!([swing("root")])));
    let (before, origin) = world(&mut state, "figure", FRAME);
    let answer = ok(
        &mut state,
        json!({"op":"scene.rotate","scene":"street","entity":FIGURE,"frame":FRAME,
               "axis":"y","degrees":35,"space":"world"}),
    );
    assert_eq!(answer["changed"], true);
    let (after, after_origin) = world(&mut state, "figure", FRAME);
    assert_matrix(
        &after,
        &matmul(&rodrigues([0.0, 1.0, 0.0], 35f64.to_radians()), &before),
        "swinging parent",
    );
    assert_vec(after_origin, origin, "pivot");
    // The animation is still on top of the document: the tracks are untouched.
    assert_eq!(
        document_of(&state)["performanceTracks"][0]["channels"],
        animated(json!([swing("root")]))["performanceTracks"][0]["channels"]
    );
    // And the swinging node itself is the refused one.
    let error = err(
        &mut state,
        json!({"op":"scene.rotate","scene":"street","entity":ROOT,"frame":FRAME,
               "axis":"y","degrees":35,"space":"world"}),
    );
    assert_eq!(reason(&error), "animated_rotation");
}

#[test]
fn a_moving_or_growing_node_is_rotated_because_only_a_rotation_channel_is_in_the_way() {
    let channels = json!([
        {"kind": "event", "channelId": "lift", "nodeId": "figure", "property": "localPosition",
         "startInstanceFrame": 10, "durationFrames": 20,
         "delta": {"x": 0.25, "y": 0.5, "z": 0}, "holdFinal": true},
        {"kind": "static", "channelId": "grow", "nodeId": "figure", "property": "localScale",
         "value": {"x": 1.5, "y": 1.5, "z": 1.5}},
    ]);
    const FRAME: i64 = 30;
    let mut state = street_of(animated(channels));
    let (before, origin) = world(&mut state, "figure", FRAME);
    ok(
        &mut state,
        json!({"op":"scene.rotate","scene":"street","entity":FIGURE,"frame":FRAME,
               "axis":"z","degrees":20,"space":"local"}),
    );
    let (after, after_origin) = world(&mut state, "figure", FRAME);
    let (axis, _) = (unit([before[0][2], before[1][2], before[2][2]]), ());
    assert_matrix(
        &after,
        &matmul(&rodrigues(axis, 20f64.to_radians()), &before),
        "animated position and scale",
    );
    assert_vec(after_origin, origin, "pivot");
}

// --- Spark ---------------------------------------------------------------------------------------

fn with_splat(scale: [f64; 3], parent_scale: [f64; 3]) -> Value {
    let mut bundle = bundle(MINIMAL);
    bundle["assets"].as_array_mut().unwrap().push(json!({
        "assetId": "cloud", "revisionId": "cloud-r1", "contentHash": "hash-cloud",
        "relativePath": "splats/cloud.spz", "format": "spz", "widthPx": 1, "heightPx": 1,
        "alphaMode": "opaque", "colorSpace": "srgb",
    }));
    let nodes = bundle["sceneDefinitions"][0]["nodes"]
        .as_array_mut()
        .unwrap();
    nodes.push(json!({
        "id": "cloud-node", "parentId": "root", "type": "splat",
        "assetRef": {"assetId": "cloud", "revisionId": "cloud-r1", "contentHash": "hash-cloud",
                     "relativePath": "splats/cloud.spz"},
        "localPosition": {"x": 0, "y": 0, "z": 1},
        "localRotation": {"x": 0, "y": 0, "z": 0, "w": 1},
        "localScale": {"x": scale[0], "y": scale[1], "z": scale[2]},
    }));
    let root = nodes.iter_mut().find(|n| n["id"] == "root").unwrap();
    root["localScale"] = json!({"x": parent_scale[0], "y": parent_scale[1], "z": parent_scale[2]});
    bundle
}

const CLOUD: &str = "scene/scene-a/node/cloud-node";

#[test]
fn a_splat_with_a_uniform_scale_turns_and_its_world_matrix_stays_a_similarity() {
    let mut state = street_of(with_splat([2.0, 2.0, 2.0], [1.5, 1.5, 1.5]));
    let (before, origin) = world(&mut state, "cloud-node", 0);
    ok(&mut state, rotate_request(CLOUD, "y", 60.0, "world"));
    let (after, after_origin) = world(&mut state, "cloud-node", 0);
    assert_matrix(
        &after,
        &matmul(&rodrigues([0.0, 1.0, 0.0], 60f64.to_radians()), &before),
        "splat",
    );
    assert_vec(after_origin, origin, "pivot");
    // What Spark needs: one uniform scale (the three column lengths agree) and orthogonal axes.
    let columns: Vec<V3> = (0..3)
        .map(|c| [after[0][c], after[1][c], after[2][c]])
        .collect();
    let lengths: Vec<f64> = columns.iter().map(|c| dot(*c, *c).sqrt()).collect();
    assert!(
        near(lengths[0], lengths[1], 1e-9) && near(lengths[1], lengths[2], 1e-9),
        "{lengths:?}"
    );
    assert!(near(dot(columns[0], columns[1]), 0.0, 1e-9));
    assert!(near(lengths[0], 3.0, 1e-9), "scale 1.5 · 2");
}

#[test]
fn a_splat_that_spark_cannot_draw_is_refused_explicitly_and_never_rotated() {
    // The plan itself refuses a splat whose world matrix is not a similarity (BL123), with the
    // node and the reason; the rotate tool does not route around it.
    for (scale, parent) in [
        ([2.0, 1.0, 1.0], [1.0, 1.0, 1.0]),
        ([1.0, 1.0, 1.0], [2.0, 1.0, 1.0]),
    ] {
        let mut state = street_of(with_splat(scale, parent));
        let before = observable(&state);
        let error = err(&mut state, rotate_request(CLOUD, "y", 60.0, "world"));
        assert_eq!(error.code, "unsupported", "{scale:?} {parent:?}: {error:?}");
        assert!(
            error.message.contains("cloud-node")
                || error
                    .details
                    .as_ref()
                    .is_some_and(|d| d.to_string().contains("cloud-node")),
            "{error:?}"
        );
        assert_eq!(observable(&state), before);
    }
}

// --- the same road for every surface ------------------------------------------------------------

#[test]
fn the_typed_commands_match_the_json_ones() {
    let mut json_state = street();
    ok(
        &mut json_state,
        json!({"op":"scene.gizmo.tool","tool":"rotate","space":"local"}),
    );
    ok(&mut json_state, rotate_request(FIGURE, "y", 33.0, "local"));
    let mut typed_state = street();
    typed_state
        .execute(Command::Scene(SceneCommand::Gizmo(GizmoCommand::Tool {
            tool: Some(GizmoTool::Rotate),
            space: Some(GizmoSpace::Local),
        })))
        .unwrap();
    typed_state
        .execute(Command::Scene(SceneCommand::Gizmo(GizmoCommand::Rotate {
            scene: "street".into(),
            entity: FIGURE.into(),
            at: FrameAt::Frame(0),
            axis: Handle::Y,
            degrees: 33.0,
            space: Some(GizmoSpace::Local),
        })))
        .unwrap();
    assert_eq!(document_of(&json_state), document_of(&typed_state));
    assert_eq!(undo_depth(&json_state), undo_depth(&typed_state));
    let _ = (Quat::IDENTITY, Vec3::ZERO);
}

#[test]
fn the_capabilities_list_the_rotate_vocabulary_additively() {
    let mut state = street();
    let commands = ok(&mut state, json!({"op":"scene.capabilities"}))["commands"].clone();
    let listed: Vec<&str> = commands
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    for command in [
        "scene.rotate",
        "scene.gizmo.tool",
        "scene.translate",
        "scene.pointer.down",
    ] {
        assert!(listed.contains(&command), "{command} in {listed:?}");
    }
}

#[test]
fn a_json_lines_session_runs_the_whole_rotation_flow_in_process() {
    let mut state = EditorState::demo();
    let lines = [
        json!({"op":"project.create","name":"Jsonl"}),
        json!({"op":"scene.attach","id":"street","bundle":bundle(MINIMAL)}),
        json!({"op":"scene.select","scene":"street","entities":[FIGURE]}),
        json!({"op":"scene.gizmo.tool","tool":"rotate","space":"world"}),
        json!({"op":"scene.rotate","scene":"street","entity":FIGURE,"frame":0,"axis":"z","degrees":90}),
        json!({"op":"history.undo"}),
        json!({"op":"scene.gizmo.tool","tool":"translate"}),
    ];
    let session: String = lines.iter().map(|line| format!("{line}\n")).collect();
    let mut out = Vec::new();
    headless::run_session(&mut state, None, session.as_bytes(), &mut out).unwrap();
    let answers: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(answers.len(), lines.len());
    assert!(answers.iter().all(|a| a["ok"] == true), "{answers:?}");
    assert_eq!(answers[4]["result"]["changed"], true);
    assert_eq!(figure_rotation(&state), [0.0, 0.0, 0.0, 1.0], "undone");
}
