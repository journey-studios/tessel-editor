//! F055 / BL015.1 (G1.2): the scale tool of the gizmo and `scene.scale`, through the public library
//! API and the JSON executor (in process: no window, no GPU, no Chrome, no GPUI, no pointer device).
//!
//! Every expectation is computed **here**, with formulas written in this file (a quaternion to
//! matrix, `W·diag(d)` for a local scale, `(I + (f-1)·a·aᵀ)·W` for a world one, a pinhole camera),
//! never with the code under test:
//!
//! - **Factors.** Multiplicative, relative to the press: `f = 1` is no change (same bits), the
//!   uniform handle applies the same `f` to the three components, factors compose by product.
//! - **Space.** `local`: the node's own axes, always representable, whatever the parents are.
//!   `world`: only when the scene's axis runs along one of the node's own axes (then the factor
//!   lands on that component); otherwise the handle is disabled with `world_axis_shear`.
//! - **Parents.** For a rotated, scaled, non-uniformly scaled or mirroring parent the world matrix
//!   after the edit is exactly the oracle's; the parent, the position and the rotation are intact.
//! - **Limits.** Positive finite factor; `[1e-3, 1e3]` for a changed component, a component outside
//!   may only move towards it; no mirror is ever created, the sign it had is kept.
//! - **Equivalence.** The document a handle drag leaves equals the one `scene.scale` leaves for the
//!   `factor`/`axis`/`space` the drag reported, bit for bit.
//! - **Lifecycle.** One undo entry per gesture, none for a no-op, cancel and a refused position
//!   leave the document as it was, a refused position keeps the last legal candidate.
//! - **Not covered**: the canvas of the runtime, the window and a physical pointer
//!   (`runtime/test/*overlay*.mjs`, the GPUI tests and the native proof).

use std::fs;
use std::path::Path;

use core::prelude::v1::test;
use serde_json::{Value, json};
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
    ok(&mut state, json!({"op":"project.create","name":"Scale"}));
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle}),
    );
    state
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
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

fn handle_ids(overlay: &Value) -> Vec<String> {
    overlay["handles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|handle| handle["id"].as_str().unwrap().to_string())
        .collect()
}

fn disabled(overlay: &Value) -> Vec<(String, String)> {
    overlay["disabled_handles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["id"].as_str().unwrap().to_string(),
                entry["reason"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn scale_request(entity: &str, axis: &str, factor: f64, space: &str) -> Value {
    json!({"op":"scene.scale","scene":"street","entity":entity,"frame":0,
           "axis":axis,"factor":factor,"space":space})
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

const IDENTITY: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn diag(d: V3) -> M3 {
    [[d[0], 0.0, 0.0], [0.0, d[1], 0.0], [0.0, 0.0, d[2]]]
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

fn vec_value(v: V3) -> Value {
    json!({"x": v[0], "y": v[1], "z": v[2]})
}

fn vec_of(value: &Value) -> V3 {
    [
        value["x"].as_f64().unwrap(),
        value["y"].as_f64().unwrap(),
        value["z"].as_f64().unwrap(),
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

fn figure_scale(state: &EditorState) -> V3 {
    vec_of(&node_field(state, "figure", "localScale"))
}

fn set_node(
    state: &mut EditorState,
    node: &str,
    rotation: Option<[f64; 4]>,
    scale: Option<V3>,
    position: Option<V3>,
) {
    let entity = format!("scene/scene-a/node/{node}");
    let mut ops = Vec::new();
    if let Some(q) = rotation {
        ops.push(json!({"set": format!("{entity}/rotation"), "value": quat_value(q)}));
    }
    if let Some(s) = scale {
        ops.push(json!({"set": format!("{entity}/scale"), "value": vec_value(s)}));
    }
    if let Some(p) = position {
        ops.push(json!({"set": format!("{entity}/position"), "value": vec_value(p)}));
    }
    ok(state, json!({"op":"scene.edit","scene":"street","ops":ops}));
}

fn set_root(state: &mut EditorState, rotation: Option<[f64; 4]>, scale: Option<V3>) {
    set_node(state, "root", rotation, scale, None);
}

fn set_figure(state: &mut EditorState, rotation: Option<[f64; 4]>, scale: Option<V3>) {
    set_node(state, "figure", rotation, scale, None);
}

/// A camera that sees all three axes of a node near the origin from a corner (the default one
/// looks down -Z, so the z handle is almost a point).
fn angled_camera(state: &mut EditorState) {
    ok(
        state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set": format!("{CAMERA_START}/position"), "value": {"x": 2.5, "y": 1.8, "z": 6}},
            {"set": format!("{CAMERA_START}/target"), "value": {"x": 0.3, "y": 0, "z": 0}}]}),
    );
}

/// `M = P·R`: the world matrix of the figure without its own scale, built from the authored parts:
/// the root's rotation times its scale, then the figure's rotation.
fn m_of(root_rotation: [f64; 4], root_scale: V3, figure_rotation: [f64; 4]) -> M3 {
    matmul(
        &matmul(&matrix_of(root_rotation), &diag(root_scale)),
        &matrix_of(figure_rotation),
    )
}

fn column(m: &M3, c: usize) -> V3 {
    [m[0][c], m[1][c], m[2][c]]
}

/// A pinhole camera written here (vertical field of view, +Y up).
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
    Camera {
        position: vec_of(&plan["camera"]["position"]),
        target: vec_of(&plan["camera"]["target"]),
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

    fn half(&self) -> f64 {
        (self.fov_deg.to_radians() / 2.0).tan()
    }

    fn project(&self, p: V3) -> (f64, f64) {
        let (forward, right, up) = self.basis();
        let relative = sub(p, self.position);
        let depth = dot(relative, forward);
        let x = dot(relative, right) / (depth * self.half() * self.aspect);
        let y = dot(relative, up) / (depth * self.half());
        ((x + 1.0) / 2.0, (1.0 - y) / 2.0)
    }

    /// The world length of a handle `0.14` picture heights long at the depth of `origin`.
    fn handle_length(&self, origin: V3) -> f64 {
        let (forward, _, _) = self.basis();
        0.14 * 2.0 * self.half() * dot(sub(origin, self.position), forward)
    }
}

fn along(origin: V3, direction: V3, length: f64) -> V3 {
    [
        origin[0] + direction[0] * length,
        origin[1] + direction[1] * length,
        origin[2] + direction[2] * length,
    ]
}

fn uv(value: &Value) -> (f64, f64) {
    (value["u"].as_f64().unwrap(), value["v"].as_f64().unwrap())
}

fn press(state: &mut EditorState, at: Value) -> Result<Value, CommandError> {
    run(
        state,
        json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":at}),
    )
}

/// Scales the figure by `factor` with a drag of `axis` (`x`, `y`, `z`, `uniform`), the pointer
/// following the point of the handle it grabbed. `direction` is the world direction the axis
/// handle runs along (from the oracle), unused for the uniform handle. Returns (last move, release).
fn drag_handle(
    state: &mut EditorState,
    axis: &str,
    direction: V3,
    factor: f64,
    space: &str,
) -> (Value, Value) {
    select(state, FIGURE);
    tool(state, "scale", space);
    let camera = camera(state, 0);
    let (_, origin) = world(state, "figure", 0);
    let shown = overlay(state);
    let grabbed = handle(&shown, axis)["to"].clone();
    let pressed = press(state, grabbed.clone()).unwrap();
    assert_eq!(pressed["dragging"], true, "{pressed}");
    assert_eq!(pressed["hit"]["handle"], axis);
    assert_eq!(pressed["tool"], "scale");
    let (u, v) = pointer_for(&camera, origin, &shown, axis, direction, factor);
    let moved = ok(
        state,
        json!({"op":"scene.pointer.move","point":point(u, v)}),
    );
    assert_eq!(moved["applied"], true, "{moved}");
    assert_eq!(moved["document_changed"], false);
    let up = ok(state, json!({"op":"scene.pointer.up","point":point(u, v)}));
    (moved, up)
}

/// Where the pointer must be for a drag that grabbed the tip of `axis` to ask for `factor`.
fn pointer_for(
    camera: &Camera,
    origin: V3,
    shown: &Value,
    axis: &str,
    direction: V3,
    factor: f64,
) -> (f64, f64) {
    if axis == "uniform" {
        // A ratio of picture distances from the projected origin: scale the offset of the box.
        let (ou, ov) = uv(&shown["origin"]);
        let (bu, bv) = uv(&handle(shown, "uniform")["to"]);
        return (ou + (bu - ou) * factor, ov + (bv - ov) * factor);
    }
    let length = camera.handle_length(origin);
    camera.project(along(origin, direction, length * factor))
}

// --- the tool and the space ----------------------------------------------------------------------

#[test]
fn scale_is_the_third_tool_a_session_choice_and_not_an_edit() {
    let mut state = street();
    let before = observable(&state);
    let read = ok(&mut state, json!({"op":"scene.gizmo.tool"}));
    assert_eq!(read["tool"], "translate");
    assert_eq!(read["tools"], json!(["translate", "rotate", "scale"]));
    let chosen = ok(
        &mut state,
        json!({"op":"scene.gizmo.tool","tool":"scale","space":"local"}),
    );
    assert_eq!(chosen["tool"], "scale");
    assert_eq!(chosen["space"], "local");
    assert_eq!(chosen["previous"]["tool"], "translate");
    assert_eq!(chosen["changed"], true);
    assert_eq!(chosen["document_changed"], false);
    assert_eq!(state.scene_interaction.tool(), GizmoTool::Scale);
    assert_eq!(observable(&state), before, "a tool is not an edit");
    // Asking again for what is set is a read.
    assert_eq!(
        ok(&mut state, json!({"op":"scene.gizmo.tool","tool":"scale"}))["changed"],
        false
    );
    // An unknown tool names the options, the three of them.
    let error = err(&mut state, json!({"op":"scene.gizmo.tool","tool":"resize"}));
    assert_eq!(error.code, "invalid_value");
    assert_eq!(
        error.details.as_ref().unwrap()["expected"]["options"],
        json!(["translate", "rotate", "scale"])
    );
    assert_eq!(state.scene_interaction.tool(), GizmoTool::Scale);
}

#[test]
fn the_tool_is_settable_like_any_control_and_part_of_the_overlay_identity() {
    let mut state = street();
    select(&mut state, FIGURE);
    let translate_stamp = overlay(&mut state)["stamp"].clone();
    ok(
        &mut state,
        json!({"op":"set","target":"monitor.gizmo","field":"tool","value":"scale"}),
    );
    assert_eq!(state.scene_interaction.tool(), GizmoTool::Scale);
    assert_eq!(
        ok(&mut state, json!({"op":"scene.gizmo.tool"}))["tool"],
        "scale"
    );
    let shown = overlay(&mut state);
    assert_eq!(shown["tool"], "scale");
    assert_eq!(shown["stamp"]["tool"], "scale");
    assert_ne!(
        shown["stamp"], translate_stamp,
        "a press made against the arrows is not one against the scale handles"
    );
    // The memo key moves with the tool too, so the adapter redraws.
    let key = state.gizmo_overlay_key("street", 0);
    tool(&mut state, "rotate", "world");
    assert_ne!(state.gizmo_overlay_key("street", 0), key);
}

// --- the overlay ---------------------------------------------------------------------------------

#[test]
fn the_scale_overlay_has_three_axis_handles_and_the_uniform_one_where_the_oracle_puts_them() {
    for space in ["world", "local"] {
        // The world axes only exist where they run along the node's: quarter turns for that case.
        let (root_q, figure_q) = if space == "world" {
            (
                quat_about([0.0, 1.0, 0.0], 90.0),
                quat_about([0.0, 0.0, 1.0], 90.0),
            )
        } else {
            (
                quat_about([1.0, 2.0, 0.5], 40.0),
                quat_about([0.0, 1.0, 1.0], 25.0),
            )
        };
        let mut state = street();
        set_root(&mut state, Some(root_q), Some([2.0, 2.0, 2.0]));
        set_figure(&mut state, Some(figure_q), Some([1.0, 2.0, 0.7]));
        angled_camera(&mut state);
        select(&mut state, FIGURE);
        tool(&mut state, "scale", space);
        let (_, origin) = world(&mut state, "figure", 0);
        let camera = camera(&mut state, 0);
        let shown = overlay(&mut state);
        assert_eq!(shown["visible"], true, "{shown}");
        assert_eq!(shown["tool"], "scale");
        assert_eq!(shown["space"], space);
        assert_eq!(handle_ids(&shown), ["x", "y", "z", "uniform"], "{space}");
        assert_eq!(disabled(&shown), []);
        let m = m_of(root_q, [2.0, 2.0, 2.0], figure_q);
        let length = camera.handle_length(origin);
        for (index, id) in ["x", "y", "z"].into_iter().enumerate() {
            let entry = handle(&shown, id);
            assert_eq!(entry["kind"], "scale_axis");
            assert_eq!(entry["from"], shown["origin"]);
            let direction = if space == "local" {
                unit(column(&m, index))
            } else {
                let mut e = [0.0; 3];
                e[index] = 1.0;
                e
            };
            let (u, v) = camera.project(along(origin, direction, length));
            let (tu, tv) = uv(&entry["to"]);
            assert!(
                near(u, tu, 1e-9) && near(v, tv, 1e-9),
                "{space} {id}: ({u}, {v}) vs ({tu}, {tv})"
            );
        }
        // The uniform handle: a fixed offset from the origin on the picture, up and to the right.
        let uniform = handle(&shown, "uniform");
        assert_eq!(uniform["kind"], "scale_uniform");
        let (ou, ov) = uv(&shown["origin"]);
        let (bu, bv) = uv(&uniform["to"]);
        let diagonal = 0.09 * std::f64::consts::FRAC_1_SQRT_2;
        assert!(near(bu, ou + diagonal / camera.aspect, 1e-12) && near(bv, ov - diagonal, 1e-12));
        assert_eq!(shown["metrics"]["uniform_offset"], 0.09);
        assert_eq!(
            shown["metrics"]["scale_limits"],
            json!({"min": 0.001, "max": 1000.0})
        );
        // The outline of the selected plane is still there.
        assert_eq!(shown["outlines"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn the_world_handles_the_node_cannot_hold_are_disabled_with_the_reason_and_not_drawn() {
    // A node turned 30° about Z: the scene's Z axis is still a node axis, X and Y are not.
    let mut state = street();
    set_figure(&mut state, Some(quat_about([0.0, 0.0, 1.0], 30.0)), None);
    angled_camera(&mut state);
    select(&mut state, FIGURE);
    tool(&mut state, "scale", "world");
    let shown = overlay(&mut state);
    assert_eq!(shown["visible"], true);
    assert_eq!(handle_ids(&shown), ["z", "uniform"]);
    assert_eq!(
        disabled(&shown),
        [
            ("x".to_string(), "world_axis_shear".to_string()),
            ("y".to_string(), "world_axis_shear".to_string())
        ]
    );
    // Nothing can be pressed where the missing handles would have been.
    let camera = camera(&mut state, 0);
    let (_, origin) = world(&mut state, "figure", 0);
    let tip = camera.project(along(origin, [1.0, 0.0, 0.0], camera.handle_length(origin)));
    let hit = ok(
        &mut state,
        json!({"op":"scene.gizmo.hit","scene":"street","frame":0,"point":point(tip.0, tip.1)}),
    );
    assert_ne!(hit["hit"]["handle"], "x", "{hit}");
    // The same node in its own axes has all of them.
    tool(&mut state, "scale", "local");
    let local = overlay(&mut state);
    assert_eq!(handle_ids(&local), ["x", "y", "z", "uniform"]);
    assert_eq!(disabled(&local), []);
}

// --- known factors -------------------------------------------------------------------------------

#[test]
fn a_factor_multiplies_the_authored_component_and_touches_nothing_else() {
    for space in ["world", "local"] {
        let mut state = street();
        select(&mut state, FIGURE);
        let before = (
            node_field(&state, "figure", "localPosition"),
            node_field(&state, "figure", "localRotation"),
            node_field(&state, "root", "localScale"),
        );
        let answer = ok(&mut state, scale_request(FIGURE, "x", 2.0, space));
        assert_eq!(answer["action"], "scene.scale");
        assert_eq!(answer["changed"], true);
        assert_eq!(answer["axis"], "x");
        assert_eq!(answer["space"], space);
        assert_eq!(answer["factor"], 2.0);
        assert_eq!(answer["component"], "x");
        assert_eq!(answer["scale"], json!({"x": 2.0, "y": 1.0, "z": 1.0}));
        assert_eq!(figure_scale(&state), [2.0, 1.0, 1.0]);
        // The pivot, the rotation and the parent are what they were.
        assert_eq!(node_field(&state, "figure", "localPosition"), before.0);
        assert_eq!(node_field(&state, "figure", "localRotation"), before.1);
        assert_eq!(node_field(&state, "root", "localScale"), before.2);
        assert_eq!(undo_depth(&state), 2, "attach + one scale ({space})");
    }
}

#[test]
fn the_uniform_handle_applies_the_same_factor_to_the_three_components_and_keeps_the_ratios() {
    for space in ["world", "local"] {
        let mut state = street();
        set_figure(&mut state, None, Some([1.0, 2.0, 0.7]));
        let answer = ok(&mut state, scale_request(FIGURE, "uniform", 1.5, space));
        assert_eq!(answer["component"], "all");
        let scale = figure_scale(&state);
        assert!(near(scale[0], 1.5, 1e-15), "{scale:?}");
        assert!(near(scale[1], 3.0, 1e-15), "{scale:?}");
        assert!(near(scale[2], 1.05, 1e-15), "{scale:?}");
        // The ratios between the components are the ones the node had.
        assert!(near(scale[1] / scale[0], 2.0, 1e-12));
        assert!(near(scale[2] / scale[0], 0.7, 1e-12));
    }
}

#[test]
fn factors_are_multiplicative_and_each_is_its_own_undo_entry() {
    let mut state = street();
    ok(&mut state, scale_request(FIGURE, "y", 2.0, "local"));
    ok(&mut state, scale_request(FIGURE, "y", 1.5, "local"));
    assert_eq!(figure_scale(&state), [1.0, 3.0, 1.0]);
    assert_eq!(undo_depth(&state), 3);
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(figure_scale(&state), [1.0, 2.0, 1.0]);
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(figure_scale(&state), [1.0, 1.0, 1.0]);
    ok(&mut state, json!({"op":"history.redo"}));
    ok(&mut state, json!({"op":"history.redo"}));
    assert_eq!(
        figure_scale(&state),
        [1.0, 3.0, 1.0],
        "redo gives the same bits"
    );
}

// --- parents, local axes -------------------------------------------------------------------------

#[test]
fn a_local_axis_scales_the_node_for_any_parent_and_the_world_matrix_is_w_times_diag() {
    let rotated = quat_about([1.0, 2.0, 2.0], 70.0);
    let own = quat_about([0.0, 1.0, 1.0], 25.0);
    let parents: [(&str, V3); 4] = [
        ("rotated and uniformly scaled", [2.0, 2.0, 2.0]),
        ("rotated and unevenly scaled", [2.0, 1.0, 3.0]),
        ("mirroring", [-1.0, 1.0, 1.0]),
        ("shrunk", [0.5, 0.5, 0.5]),
    ];
    for (name, parent_scale) in parents {
        for (index, axis) in ["x", "y", "z"].into_iter().enumerate() {
            let mut state = street();
            set_root(&mut state, Some(rotated), Some(parent_scale));
            set_figure(&mut state, Some(own), Some([1.0, 2.0, 0.7]));
            let (before, origin) = world(&mut state, "figure", 0);
            let (root_before, root_origin) = world(&mut state, "root", 0);
            let authored = figure_scale(&state);
            ok(&mut state, scale_request(FIGURE, axis, 1.8, "local"));
            let (after, after_origin) = world(&mut state, "figure", 0);
            let mut d = [1.0; 3];
            d[index] = 1.8;
            assert_matrix(
                &after,
                &matmul(&before, &diag(d)),
                &format!("{name} {axis}: W·diag(d)"),
            );
            assert_vec(after_origin, origin, "the pivot does not move");
            let (root_after, root_after_origin) = world(&mut state, "root", 0);
            assert_matrix(&root_after, &root_before, "the parent is untouched");
            assert_vec(root_after_origin, root_origin, "the parent does not move");
            let mut expected = authored;
            expected[index] *= 1.8;
            assert_eq!(figure_scale(&state), expected, "{name} {axis}");
        }
    }
}

#[test]
fn a_world_axis_along_a_node_axis_scales_that_component_and_the_matrix_is_g_times_w() {
    // The parent turned a quarter about Y (scale 2) and the node a quarter about Z: the scene's
    // axes are the node's axes up to a permutation and signs, so every world handle is legal.
    let root_q = quat_about([0.0, 1.0, 0.0], 90.0);
    let own = quat_about([0.0, 0.0, 1.0], 90.0);
    let o = matmul(&matrix_of(root_q), &matrix_of(own));
    for (index, axis) in ["x", "y", "z"].into_iter().enumerate() {
        let mut state = street();
        set_root(&mut state, Some(root_q), Some([2.0, 2.0, 2.0]));
        set_figure(&mut state, Some(own), Some([1.0, 2.0, 0.7]));
        let (before, origin) = world(&mut state, "figure", 0);
        let answer = ok(&mut state, scale_request(FIGURE, axis, 1.6, "world"));
        // The component the factor landed on: the node axis the world axis runs along.
        let landed = (0..3)
            .find(|c| near(o[index][*c].abs(), 1.0, 1e-9))
            .unwrap();
        assert_eq!(answer["component"], ["x", "y", "z"][landed], "{axis}");
        // G = I + (f-1)·a·aᵀ for the scene's axis `a`, written here.
        let mut g = IDENTITY;
        g[index][index] += 0.6;
        let (after, after_origin) = world(&mut state, "figure", 0);
        assert_matrix(&after, &matmul(&g, &before), &format!("world {axis}: G·W"));
        assert_vec(after_origin, origin, "the pivot does not move");
        // Only that component changed, by the factor.
        let mut expected = [1.0, 2.0, 0.7];
        expected[landed] *= 1.6;
        let scale = figure_scale(&state);
        for i in 0..3 {
            assert!(near(scale[i], expected[i], 1e-12), "{axis}: {scale:?}");
        }
    }
}

#[test]
fn a_world_axis_through_an_unevenly_scaled_parent_is_legal_when_it_runs_along_a_node_axis() {
    let mut state = street();
    set_root(&mut state, None, Some([2.0, 1.0, 3.0]));
    let (before, _) = world(&mut state, "figure", 0);
    ok(&mut state, scale_request(FIGURE, "x", 2.5, "world"));
    let mut g = IDENTITY;
    g[0][0] += 1.5;
    let (after, _) = world(&mut state, "figure", 0);
    assert_matrix(&after, &matmul(&g, &before), "uneven parent, world x");
    assert_eq!(figure_scale(&state), [2.5, 1.0, 1.0]);
}

#[test]
fn a_world_axis_that_would_shear_is_refused_typed_and_the_document_is_untouched() {
    let cases: Vec<(&str, [f64; 4], V3, &str)> = vec![
        (
            "node turned 30° about z",
            quat_about([0.0, 0.0, 1.0], 30.0),
            [1.0; 3],
            "x",
        ),
        (
            "node turned 30° about z",
            quat_about([0.0, 0.0, 1.0], 30.0),
            [1.0; 3],
            "y",
        ),
        (
            "node turned 45° about x",
            quat_about([1.0, 0.0, 0.0], 45.0),
            [1.0; 3],
            "y",
        ),
        (
            "node and axis 1e-3 off",
            quat_about([0.0, 0.0, 1.0], 0.001),
            [1.0; 3],
            "x",
        ),
    ];
    for (name, rotation, scale, axis) in cases {
        let mut state = street();
        set_figure(&mut state, Some(rotation), Some(scale));
        let before = observable(&state);
        let error = err(&mut state, scale_request(FIGURE, axis, 2.0, "world"));
        assert_eq!(error.code, "not_scalable", "{name} {axis}");
        assert_eq!(reason(&error), "world_axis_shear", "{name} {axis}");
        assert_eq!(observable(&state), before, "{name} {axis}");
        // The same factor in the node's own axes is legal, and the uniform handle in the world too.
        ok(&mut state, scale_request(FIGURE, axis, 2.0, "local"));
        ok(&mut state, scale_request(FIGURE, "uniform", 2.0, "world"));
    }
    // A parent turned 45° and scaled unevenly: the world X is no node axis either.
    let mut state = street();
    set_root(
        &mut state,
        Some(quat_about([0.0, 0.0, 1.0], 45.0)),
        Some([2.0, 1.0, 1.0]),
    );
    let (before, _) = world(&mut state, "figure", 0);
    assert_eq!(
        reason(&err(&mut state, scale_request(FIGURE, "x", 2.0, "world"))),
        "world_axis_shear"
    );
    // ... while its local X still scales exactly.
    ok(&mut state, scale_request(FIGURE, "x", 2.0, "local"));
    let (after, _) = world(&mut state, "figure", 0);
    assert_matrix(&after, &matmul(&before, &diag([2.0, 1.0, 1.0])), "local x");
}

#[test]
fn the_uniform_handle_is_f_times_w_in_both_spaces_even_where_axes_would_shear() {
    for space in ["world", "local"] {
        let mut state = street();
        set_root(
            &mut state,
            Some(quat_about([0.0, 0.0, 1.0], 45.0)),
            Some([2.0, 1.0, 1.0]),
        );
        set_figure(&mut state, Some(quat_about([1.0, 1.0, 0.0], 33.0)), None);
        let (before, origin) = world(&mut state, "figure", 0);
        ok(&mut state, scale_request(FIGURE, "uniform", 1.25, space));
        let (after, after_origin) = world(&mut state, "figure", 0);
        let mut expected = before;
        for row in &mut expected {
            for value in row {
                *value *= 1.25;
            }
        }
        assert_matrix(&after, &expected, &format!("f·W ({space})"));
        assert_vec(after_origin, origin, "the pivot does not move");
    }
}

#[test]
fn the_children_of_a_scaled_node_scale_with_it_about_its_origin() {
    let mut bundle = bundle(MINIMAL);
    let nodes = bundle["sceneDefinitions"][0]["nodes"]
        .as_array_mut()
        .unwrap();
    nodes.push(json!({
        "id": "child", "parentId": "figure", "type": "group",
        "localPosition": {"x": 1, "y": 0, "z": 0},
        "localRotation": {"x": 0, "y": 0, "z": 0, "w": 1},
        "localScale": {"x": 1, "y": 1, "z": 1},
    }));
    let mut state = street_of(bundle);
    set_figure(&mut state, Some(quat_about([0.0, 0.0, 1.0], 90.0)), None);
    let (_, figure_origin) = world(&mut state, "figure", 0);
    let (_, child_before) = world(&mut state, "child", 0);
    ok(&mut state, scale_request(FIGURE, "x", 3.0, "local"));
    let (_, child_after) = world(&mut state, "child", 0);
    // The child sat one unit along the figure's own x; the figure's x is now three times longer.
    let offset = sub(child_before, figure_origin);
    let scaled = sub(child_after, figure_origin);
    for i in 0..3 {
        assert!(
            near(scaled[i], 3.0 * offset[i], 1e-9),
            "{scaled:?} {offset:?}"
        );
    }
}

// --- signs and the factor ------------------------------------------------------------------------

#[test]
fn a_negative_scale_keeps_its_sign_and_no_mirror_is_created_by_a_factor() {
    let mut state = street();
    set_figure(&mut state, None, Some([-1.0, 1.0, 1.0]));
    let (before, _) = world(&mut state, "figure", 0);
    ok(&mut state, scale_request(FIGURE, "x", 2.0, "local"));
    assert_eq!(figure_scale(&state), [-2.0, 1.0, 1.0]);
    ok(&mut state, scale_request(FIGURE, "uniform", 0.5, "local"));
    assert_eq!(figure_scale(&state), [-1.0, 0.5, 0.5]);
    let (after, _) = world(&mut state, "figure", 0);
    assert_matrix(
        &after,
        &matmul(&before, &diag([1.0, 0.5, 0.5])),
        "the mirror it had, no other",
    );
    // A positive scale is never turned negative, however small the factor.
    let mut state = street();
    for factor in [0.0, -1.0, -1e-12, -1e9] {
        let before = observable(&state);
        let error = err(&mut state, scale_request(FIGURE, "x", factor, "local"));
        assert_eq!(error.code, "not_scalable", "{factor}");
        assert_eq!(reason(&error), "factor_not_positive", "{factor}");
        assert_eq!(observable(&state), before, "{factor}");
    }
}

#[test]
fn a_non_finite_factor_through_the_typed_api_is_refused_without_a_trace() {
    let mut state = street();
    let before = observable(&state);
    for factor in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = state
            .execute(Command::Scene(SceneCommand::Gizmo(GizmoCommand::Scale {
                scene: "street".into(),
                entity: FIGURE.into(),
                at: FrameAt::Frame(0),
                axis: Handle::X,
                factor,
                space: None,
            })))
            .unwrap_err();
        assert_eq!(error.code, "invalid_value", "{factor}");
    }
    assert_eq!(observable(&state), before);
}

// --- limits --------------------------------------------------------------------------------------

#[test]
fn the_limits_are_inclusive_one_thousand_and_one_thousandth_in_either_direction() {
    for (factor, allowed) in [
        (1000.0, true),
        (1000.0 * (1.0 + 1e-9), false),
        (0.001, true),
        (0.001 * (1.0 - 1e-9), false),
    ] {
        let mut state = street();
        let before = observable(&state);
        let result = run(&mut state, scale_request(FIGURE, "x", factor, "local"));
        if allowed {
            let answer = result.unwrap();
            assert_eq!(answer["changed"], true, "{factor}");
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.code, "not_scalable", "{factor}");
            assert_eq!(
                reason(&error),
                if factor > 1.0 {
                    "above_maximum_scale"
                } else {
                    "below_minimum_scale"
                },
                "{factor}"
            );
            assert_eq!(observable(&state), before, "{factor}");
        }
    }
    // The limit is on the result, not on the factor: from 10, a factor of 100 is the top.
    let mut state = street();
    set_figure(&mut state, None, Some([10.0, 1.0, 1.0]));
    ok(&mut state, scale_request(FIGURE, "x", 100.0, "local"));
    assert_eq!(figure_scale(&state)[0], 1000.0);
    assert_eq!(
        reason(&err(&mut state, scale_request(FIGURE, "x", 1.001, "local"))),
        "above_maximum_scale"
    );
    // The magnitude of a negative component is what is limited.
    let mut state = street();
    set_figure(&mut state, None, Some([-10.0, 1.0, 1.0]));
    assert_eq!(
        reason(&err(&mut state, scale_request(FIGURE, "x", 101.0, "local"))),
        "above_maximum_scale"
    );
    ok(&mut state, scale_request(FIGURE, "x", 100.0, "local"));
    assert_eq!(figure_scale(&state)[0], -1000.0);
}

#[test]
fn a_component_the_document_holds_outside_the_range_may_only_move_towards_it() {
    // 5000 is above the maximum (a hand-edited bundle can hold it): shrinking is legal, growing not.
    let mut state = street();
    set_figure(&mut state, None, Some([5000.0, 1.0, 1.0]));
    assert_eq!(
        reason(&err(&mut state, scale_request(FIGURE, "x", 1.5, "local"))),
        "above_maximum_scale"
    );
    ok(&mut state, scale_request(FIGURE, "x", 0.5, "local"));
    assert_eq!(figure_scale(&state)[0], 2500.0);
    // 1e-5 is below the minimum: growing is legal, shrinking not.
    let mut state = street();
    set_figure(&mut state, None, Some([1e-5, 1.0, 1.0]));
    assert_eq!(
        reason(&err(&mut state, scale_request(FIGURE, "x", 0.5, "local"))),
        "below_minimum_scale"
    );
    ok(&mut state, scale_request(FIGURE, "x", 2.0, "local"));
    assert!(near(figure_scale(&state)[0], 2e-5, 1e-20));
    // The other components are not judged by this one's range: a uniform factor that moves the
    // out-of-range one towards it and keeps the rest in range is fine, one that pushes any out is not.
    let mut state = street();
    set_figure(&mut state, None, Some([5000.0, 1.0, 1.0]));
    ok(&mut state, scale_request(FIGURE, "uniform", 0.5, "local"));
    assert_eq!(figure_scale(&state), [2500.0, 0.5, 0.5]);
    assert_eq!(
        reason(&err(
            &mut state,
            scale_request(FIGURE, "uniform", 0.0015, "local")
        )),
        "below_minimum_scale"
    );
}

#[test]
fn a_result_whose_world_matrix_would_not_be_invertible_is_refused_even_inside_the_range() {
    // Parent 0.005 on every axis (det 1.25e-7); the node (0.01, 0.01, 1) has det 1.25e-11 in the
    // world. Uniform 0.1 keeps x and y at their 1e-3 limit and z at 0.1, all inside the range,
    // but the world determinant falls to 1.25e-14, under the 1e-12 a node must keep.
    let mut state = street();
    set_root(&mut state, None, Some([0.005, 0.005, 0.005]));
    set_figure(&mut state, None, Some([0.01, 0.01, 1.0]));
    let before = observable(&state);
    let error = err(&mut state, scale_request(FIGURE, "uniform", 0.1, "local"));
    assert_eq!(error.code, "not_scalable");
    assert_eq!(reason(&error), "degenerate_result");
    assert_eq!(observable(&state), before);
    // One axis at a time stays above the bound.
    ok(&mut state, scale_request(FIGURE, "x", 0.1, "local"));
}

// --- no-op ---------------------------------------------------------------------------------------

#[test]
fn a_factor_of_one_is_a_no_op_with_the_same_bits_no_history_and_no_revision() {
    let awkward = 0.1 + 0.2;
    let mut state = street();
    set_figure(&mut state, None, Some([awkward, 2.0, 0.7]));
    let before = observable(&state);
    for factor in [1.0, 1.0 + 1e-13, 1.0 - 1e-13] {
        for axis in ["x", "y", "z", "uniform"] {
            let answer = ok(&mut state, scale_request(FIGURE, axis, factor, "local"));
            assert_eq!(answer["changed"], false, "{axis} {factor}");
            assert_eq!(answer["noop"], true);
            assert_eq!(answer["document_changed"], false);
            assert_eq!(
                answer["scale"],
                json!({"x": awkward, "y": 2.0, "z": 0.7}),
                "the authored vector comes back with its own bits"
            );
        }
    }
    assert_eq!(observable(&state), before);
    assert_eq!(figure_scale(&state)[0].to_bits(), awkward.to_bits());
    // 1 + 1e-9 is a change (the no-change band is 1e-12).
    assert_eq!(
        ok(&mut state, scale_request(FIGURE, "x", 1.0 + 1e-9, "local"))["changed"],
        true
    );
}

// --- refusals leave no trace ---------------------------------------------------------------------

#[test]
fn what_cannot_be_scaled_is_refused_atomically_with_a_typed_code() {
    let mut state = street();
    let before = observable(&state);
    let cases: Vec<(Value, &str)> = vec![
        (
            scale_request("camera/cam-main", "x", 2.0, "local"),
            "not_scalable",
        ),
        (
            scale_request("asset/figure", "x", 2.0, "local"),
            "not_scalable",
        ),
        (
            scale_request("scene/scene-a/node/ghost", "x", 2.0, "local"),
            "unknown_entity",
        ),
        (
            scale_request("not an address", "x", 2.0, "local"),
            "invalid_entity_path",
        ),
        (
            scale_request(FIGURE, "screen", 2.0, "local"),
            "invalid_value",
        ),
        (scale_request(FIGURE, "w", 2.0, "local"), "invalid_value"),
        (scale_request(FIGURE, "x", 2.0, "sideways"), "invalid_value"),
        (
            json!({"op":"scene.scale","scene":"nowhere","entity":FIGURE,"frame":0,"axis":"x","factor":2}),
            "unknown_element",
        ),
        (
            json!({"op":"scene.scale","scene":"street","entity":FIGURE,"axis":"x","factor":2}),
            "invalid_request",
        ),
        (
            json!({"op":"scene.scale","scene":"street","entity":FIGURE,"frame":0,"axis":"x"}),
            "invalid_request",
        ),
        (
            json!({"op":"scene.scale","scene":"street","entity":FIGURE,"frame":0,"axis":"x","factor":"two"}),
            "invalid_request",
        ),
        (
            json!({"op":"scene.scale","scene":"street","entity":FIGURE,"frame":0,"factor":2}),
            "invalid_request",
        ),
    ];
    for (request, code) in cases {
        assert_eq!(err(&mut state, request.clone()).code, code, "{request}");
        assert_eq!(observable(&state), before, "{request}");
    }
    // The ring axis parser does not take the scale handle, and the scale one takes no screen.
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.rotate","scene":"street","entity":FIGURE,"frame":0,"axis":"uniform","degrees":10})
        )
        .code,
        "invalid_value"
    );
    assert_eq!(observable(&state), before);
}

#[test]
fn a_zero_scale_node_is_degenerate_and_shows_no_handles() {
    let mut state = street();
    set_figure(&mut state, None, Some([0.0, 1.0, 1.0]));
    select(&mut state, FIGURE);
    tool(&mut state, "scale", "local");
    let before = observable(&state);
    let error = err(&mut state, scale_request(FIGURE, "uniform", 2.0, "local"));
    assert_eq!(reason(&error), "degenerate_transform");
    assert_eq!(observable(&state), before);
    let shown = overlay(&mut state);
    assert_eq!(shown["visible"], false);
    assert_eq!(shown["reason"], "degenerate_transform");
}

// --- the handle drag -----------------------------------------------------------------------------

#[test]
fn dragging_a_handle_scales_by_the_ratio_the_pointer_swept_in_both_spaces() {
    // Parent turned a quarter about Y (scale 2), node a quarter about Z: every world axis legal.
    let root_q = quat_about([0.0, 1.0, 0.0], 90.0);
    let own = quat_about([0.0, 0.0, 1.0], 90.0);
    let m = m_of(root_q, [2.0, 2.0, 2.0], own);
    for space in ["world", "local"] {
        for (index, axis) in ["x", "y", "z", "uniform"].into_iter().enumerate() {
            let mut state = street();
            set_root(&mut state, Some(root_q), Some([2.0, 2.0, 2.0]));
            set_figure(&mut state, Some(own), Some([1.0, 2.0, 0.7]));
            angled_camera(&mut state);
            let (before, origin) = world(&mut state, "figure", 0);
            let depth = undo_depth(&state);
            let direction = if axis == "uniform" {
                [0.0; 3]
            } else if space == "local" {
                unit(column(&m, index))
            } else {
                let mut e = [0.0; 3];
                e[index] = 1.0;
                e
            };
            let (_, up) = drag_handle(&mut state, axis, direction, 1.7, space);
            let swept = up["factor"].as_f64().unwrap();
            assert!(near(swept, 1.7, 1e-9), "{space} {axis}: {swept}");
            assert_eq!(up["axis"], axis);
            assert_eq!(up["space"], space);
            assert_eq!(up["tool"], "scale");
            assert_eq!(up["changed"], true);
            assert_eq!(
                undo_depth(&state),
                depth + 1,
                "one entry for the whole drag"
            );
            let (after, after_origin) = world(&mut state, "figure", 0);
            let expected = if axis == "uniform" {
                let mut e = before;
                for row in &mut e {
                    for value in row {
                        *value *= swept;
                    }
                }
                e
            } else if space == "local" {
                let mut d = [1.0; 3];
                d[index] = swept;
                matmul(&before, &diag(d))
            } else {
                let mut g = IDENTITY;
                g[index][index] += swept - 1.0;
                matmul(&g, &before)
            };
            assert_matrix(&after, &expected, &format!("{space} {axis}"));
            assert_vec(after_origin, origin, "pivot");
        }
    }
}

#[test]
fn a_handle_drag_and_scene_scale_leave_the_same_document_in_both_spaces() {
    let root_q = quat_about([0.0, 1.0, 0.0], 90.0);
    let own = quat_about([0.0, 0.0, 1.0], 90.0);
    let m = m_of(root_q, [1.5, 1.5, 1.5], own);
    for space in ["world", "local"] {
        for (index, axis) in ["x", "y", "z", "uniform"].into_iter().enumerate() {
            let prepare = |state: &mut EditorState| {
                set_root(state, Some(root_q), Some([1.5, 1.5, 1.5]));
                set_figure(state, Some(own), Some([1.0, 2.0, 0.7]));
                angled_camera(state);
            };
            let direction = if axis == "uniform" {
                [0.0; 3]
            } else if space == "local" {
                unit(column(&m, index))
            } else {
                let mut e = [0.0; 3];
                e[index] = 1.0;
                e
            };
            let mut dragged = street();
            prepare(&mut dragged);
            let (_, up) = drag_handle(&mut dragged, axis, direction, 0.63, space);
            let mut scripted = street();
            prepare(&mut scripted);
            let scaled = ok(
                &mut scripted,
                json!({"op":"scene.scale","scene":"street","entity":FIGURE,"frame":0,
                       "axis":up["axis"],"factor":up["factor"],"space":up["space"]}),
            );
            assert_eq!(scaled["changed"], true);
            assert_eq!(scaled["scale"], up["scale"], "{space} {axis}");
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
fn many_moves_are_one_entry_and_each_factor_is_computed_from_the_press() {
    let mut state = street();
    angled_camera(&mut state);
    select(&mut state, FIGURE);
    tool(&mut state, "scale", "local");
    let camera = camera(&mut state, 0);
    let (_, origin) = world(&mut state, "figure", 0);
    let shown = overlay(&mut state);
    let grabbed = handle(&shown, "y")["to"].clone();
    let depth = undo_depth(&state);
    press(&mut state, grabbed).unwrap();
    let mut last = None;
    for factor in [1.4, 3.0, 0.5, 2.0, 1.25] {
        let (u, v) = pointer_for(&camera, origin, &shown, "y", [0.0, 1.0, 0.0], factor);
        last = Some(ok(
            &mut state,
            json!({"op":"scene.pointer.move","point":point(u, v)}),
        ));
        assert_eq!(
            undo_depth(&state),
            depth,
            "the candidate is not in the history"
        );
        assert_eq!(figure_scale(&state), [1.0, 1.0, 1.0], "nor in the document");
    }
    let up = ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(undo_depth(&state), depth + 1);
    assert!(near(up["factor"].as_f64().unwrap(), 1.25, 1e-9));
    assert_eq!(up["factor"], last.unwrap()["factor"]);
    assert!(
        near(figure_scale(&state)[1], 1.25, 1e-9),
        "nothing accumulated"
    );
}

#[test]
fn a_position_past_the_origin_or_past_a_limit_is_refused_and_the_last_legal_candidate_stays() {
    // The pointer is bounded (a few pictures away), so the upper limit is reached from a node that
    // is already near it: 600 grown by 1.5 is legal (900), by 2 is not (1200).
    for (axis, direction, base, good, bad) in [
        ("x", [1.0, 0.0, 0.0], 1.0, 2.0, -0.5),
        ("x", [1.0, 0.0, 0.0], 1.0, 2.0, 0.0),
        ("x", [1.0, 0.0, 0.0], 600.0, 1.5, 2.0),
        ("uniform", [0.0; 3], 1.0, 2.0, 0.0),
        ("uniform", [0.0; 3], 600.0, 1.5, 2.0),
    ] {
        let mut state = street();
        set_figure(&mut state, None, Some([base; 3]));
        angled_camera(&mut state);
        select(&mut state, FIGURE);
        tool(&mut state, "scale", "local");
        let camera = camera(&mut state, 0);
        let (_, origin) = world(&mut state, "figure", 0);
        let shown = overlay(&mut state);
        let grabbed = handle(&shown, axis)["to"].clone();
        let depth = undo_depth(&state);
        press(&mut state, grabbed).unwrap();
        let (u, v) = pointer_for(&camera, origin, &shown, axis, direction, good);
        let first = ok(
            &mut state,
            json!({"op":"scene.pointer.move","point":point(u, v)}),
        );
        assert_eq!(first["applied"], true, "{first}");
        let (u, v) = pointer_for(&camera, origin, &shown, axis, direction, bad);
        let refused = ok(
            &mut state,
            json!({"op":"scene.pointer.move","point":point(u, v)}),
        );
        assert_eq!(refused["applied"], false, "{axis} {bad}: {refused}");
        assert_eq!(
            refused["reason"],
            if bad > 1.0 {
                "above_maximum_scale"
            } else {
                "factor_not_positive"
            },
            "{axis} {bad}"
        );
        assert_eq!(refused["document_changed"], false);
        // The release commits what was last legal, as one entry.
        let up = ok(&mut state, json!({"op":"scene.pointer.up"}));
        assert!(
            near(up["factor"].as_f64().unwrap(), good, 1e-9),
            "{axis} {bad}"
        );
        assert_eq!(up["scale"], first["scale"]);
        assert_eq!(undo_depth(&state), depth + 1);
        assert!(state.scene_interaction.drag().is_none());
    }
}

#[test]
fn a_move_the_core_rejects_as_a_bad_coordinate_keeps_the_last_legal_candidate() {
    let mut state = street();
    angled_camera(&mut state);
    select(&mut state, FIGURE);
    tool(&mut state, "scale", "local");
    let camera = camera(&mut state, 0);
    let (_, origin) = world(&mut state, "figure", 0);
    let shown = overlay(&mut state);
    let depth = undo_depth(&state);
    press(&mut state, handle(&shown, "x")["to"].clone()).unwrap();
    let (u, v) = pointer_for(&camera, origin, &shown, "x", [1.0, 0.0, 0.0], 1.5);
    let good = ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(u, v)}),
    );
    let error = err(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(1e6, 0.5)}),
    );
    assert_eq!(error.code, "invalid_value");
    let up = ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(up["scale"], good["scale"]);
    assert_eq!(undo_depth(&state), depth + 1);
}

#[test]
fn cancel_discards_a_scale_drag_without_a_trace_for_every_reason() {
    for reason in [
        "escape",
        "blur",
        "pointer_cancel",
        "hidden",
        "unavailable",
        "api",
    ] {
        for axis in ["x", "uniform"] {
            let mut state = street();
            angled_camera(&mut state);
            select(&mut state, FIGURE);
            tool(&mut state, "scale", "local");
            let camera = camera(&mut state, 0);
            let (_, origin) = world(&mut state, "figure", 0);
            let shown = overlay(&mut state);
            let before = observable(&state);
            press(&mut state, handle(&shown, axis)["to"].clone()).unwrap();
            let (u, v) = pointer_for(&camera, origin, &shown, axis, [1.0, 0.0, 0.0], 2.2);
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
            assert_eq!(observable(&state), before, "{reason} {axis}");
            assert!(state.scene_interaction.drag().is_none());
            // The next press starts clean.
            let again = handle(&overlay(&mut state), axis)["to"].clone();
            assert_eq!(press(&mut state, again).unwrap()["dragging"], true);
        }
    }
}

#[test]
fn a_press_released_without_moving_and_a_return_to_the_press_are_no_edit() {
    for axis in ["z", "uniform"] {
        let mut state = street();
        angled_camera(&mut state);
        select(&mut state, FIGURE);
        tool(&mut state, "scale", "local");
        let shown = overlay(&mut state);
        let grabbed = handle(&shown, axis)["to"].clone();
        let before = observable(&state);
        press(&mut state, grabbed.clone()).unwrap();
        let up = ok(&mut state, json!({"op":"scene.pointer.up"}));
        assert_eq!(up["changed"], false, "{axis}");
        assert_eq!(observable(&state), before);
        // A move to the press point is factor 1: the candidate equals the base.
        press(&mut state, grabbed.clone()).unwrap();
        let moved = ok(
            &mut state,
            json!({"op":"scene.pointer.move","point":grabbed}),
        );
        assert_eq!(moved["applied"], true);
        assert!(near(moved["factor"].as_f64().unwrap(), 1.0, 1e-9));
        let up = ok(&mut state, json!({"op":"scene.pointer.up","point":grabbed}));
        assert_eq!(up["changed"], false, "{axis}");
        assert_eq!(observable(&state), before);
    }
}

#[test]
fn a_press_too_close_to_the_origin_does_not_open_a_drag() {
    let mut state = street();
    angled_camera(&mut state);
    select(&mut state, FIGURE);
    tool(&mut state, "scale", "local");
    let camera = camera(&mut state, 0);
    let (_, origin) = world(&mut state, "figure", 0);
    // 4 % of the way along the x handle: on the shaft (the hit test finds x) but the ratio of two
    // positions that close to the origin is not a measure.
    let near_origin = camera.project(along(
        origin,
        [1.0, 0.0, 0.0],
        0.04 * camera.handle_length(origin),
    ));
    let before = observable(&state);
    let pressed = press(&mut state, point(near_origin.0, near_origin.1)).unwrap();
    assert_eq!(pressed["hit"]["handle"], "x", "{pressed}");
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
fn the_scale_hit_test_finds_axis_handles_and_the_uniform_box_and_nothing_of_the_other_tools() {
    let mut state = street();
    angled_camera(&mut state);
    select(&mut state, FIGURE);
    tool(&mut state, "scale", "local");
    let shown = overlay(&mut state);
    let hit_at = |state: &mut EditorState, at: Value| {
        ok(
            state,
            json!({"op":"scene.gizmo.hit","scene":"street","frame":0,"point":at}),
        )
    };
    for id in ["x", "y", "z", "uniform"] {
        let hit = hit_at(&mut state, handle(&shown, id)["to"].clone());
        assert_eq!(hit["hit"]["kind"], "handle", "{id}: {hit}");
        assert_eq!(hit["hit"]["handle"], id);
    }
    // The origin is the node's centre: no free-move `screen` handle exists for scale.
    let centre = hit_at(&mut state, shown["origin"].clone());
    assert_ne!(centre["hit"]["handle"], "screen");
    assert_ne!(centre["hit"]["handle"], "uniform");
    // The other tools never answer with the scale handle.
    for other in ["translate", "rotate"] {
        tool(&mut state, other, "local");
        let other_shown = overlay(&mut state);
        let at = other_shown["handles"][0]["to"].clone();
        let hit = hit_at(&mut state, at);
        assert_ne!(hit["hit"]["handle"], "uniform", "{other}");
        let box_of_scale = hit_at(&mut state, handle(&shown, "uniform")["to"].clone());
        assert_ne!(box_of_scale["hit"]["handle"], "uniform", "{other}");
    }
}

#[test]
fn the_uniform_box_wins_where_an_axis_handle_runs_under_it() {
    // An axis whose tip is at the box: put the camera so the +x handle projects onto the up-right
    // diagonal of the origin, where the uniform box sits. The box is on top, as the centre is in
    // translate.
    let mut state = street();
    select(&mut state, FIGURE);
    tool(&mut state, "scale", "world");
    let shown = overlay(&mut state);
    let (ou, ov) = uv(&shown["origin"]);
    let (bu, bv) = uv(&handle(&shown, "uniform")["to"]);
    // Whatever the geometry, a point exactly on the box is the uniform handle.
    let hit = ok(
        &mut state,
        json!({"op":"scene.gizmo.hit","scene":"street","frame":0,"point":point(bu, bv)}),
    );
    assert_eq!(hit["hit"]["handle"], "uniform");
    // And the point halfway to the box, on its own shaft, is no handle of the tool (the shaft of
    // the uniform handle is not a place to grab) unless an axis handle is there.
    let halfway = point((ou + bu) / 2.0, (ov + bv) / 2.0);
    let hit = ok(
        &mut state,
        json!({"op":"scene.gizmo.hit","scene":"street","frame":0,"point":halfway}),
    );
    assert_ne!(hit["hit"]["handle"], "uniform");
}

// --- the tool during a drag, other gestures, the old tools ---------------------------------------

#[test]
fn the_tool_cannot_change_under_an_open_scale_drag_and_the_drag_is_left_alone() {
    let mut state = street();
    angled_camera(&mut state);
    select(&mut state, FIGURE);
    tool(&mut state, "scale", "world");
    let shown = overlay(&mut state);
    press(&mut state, handle(&shown, "uniform")["to"].clone()).unwrap();
    let before = observable(&state);
    for request in [
        json!({"op":"scene.gizmo.tool","tool":"translate"}),
        json!({"op":"scene.gizmo.tool","tool":"rotate"}),
        json!({"op":"scene.gizmo.tool","space":"local"}),
    ] {
        assert_eq!(err(&mut state, request).code, "gesture_active");
    }
    assert!(state.scene_interaction.drag().is_some());
    assert_eq!(state.scene_interaction.tool(), GizmoTool::Scale);
    assert_eq!(state.scene_interaction.space(), GizmoSpace::World);
    assert_eq!(observable(&state), before);
    ok(&mut state, json!({"op":"scene.pointer.cancel"}));
    tool(&mut state, "translate", "world");
}

#[test]
fn a_scale_press_while_another_gesture_is_open_is_refused_and_a_late_release_is_inert() {
    let mut state = street();
    angled_camera(&mut state);
    select(&mut state, FIGURE);
    tool(&mut state, "scale", "world");
    let shown = overlay(&mut state);
    ok(
        &mut state,
        json!({"op":"scene.gesture.begin","scene":"street","targets":[FIGURE]}),
    );
    let before = observable(&state);
    assert_eq!(
        press(&mut state, handle(&shown, "uniform")["to"].clone())
            .unwrap_err()
            .code,
        "gesture_active"
    );
    assert_eq!(observable(&state), before);
    let open = state.scene_interaction.gesture().unwrap().id().to_string();
    ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":open}),
    );
    assert_eq!(
        ok(&mut state, json!({"op":"scene.pointer.up"}))["noop"],
        true
    );
}

#[test]
fn a_scale_drag_does_not_survive_an_edit_under_it() {
    let mut state = street();
    angled_camera(&mut state);
    select(&mut state, FIGURE);
    tool(&mut state, "scale", "local");
    let camera = camera(&mut state, 0);
    let (_, origin) = world(&mut state, "figure", 0);
    let shown = overlay(&mut state);
    let depth = undo_depth(&state);
    press(&mut state, handle(&shown, "x")["to"].clone()).unwrap();
    ok(&mut state, scale_request(FIGURE, "y", 2.0, "local"));
    let after_edit = observable(&state);
    let (u, v) = pointer_for(&camera, origin, &shown, "x", [1.0, 0.0, 0.0], 2.0);
    let error = err(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(u, v)}),
    );
    assert_eq!(error.code, "gesture_invalidated");
    assert!(state.gizmo_drag_is_dead());
    let _ = run(
        &mut state,
        json!({"op":"scene.pointer.up","point":point(u, v)}),
    );
    assert_eq!(
        figure_scale(&state),
        [1.0, 2.0, 1.0],
        "the stale candidate never landed"
    );
    assert_eq!(state.to_document(None), after_edit.0);
    assert_eq!(
        undo_depth(&state),
        depth + 1,
        "only the scripted edit made an entry"
    );
}

#[test]
fn translation_and_rotation_are_what_they_were_around_a_visit_to_the_scale_tool() {
    let mut state = street();
    angled_camera(&mut state);
    select(&mut state, FIGURE);
    // Scale, then rotate: the rings follow the scaled node and the rotation is exact.
    ok(&mut state, scale_request(FIGURE, "x", 2.0, "local"));
    tool(&mut state, "rotate", "world");
    let turned = ok(
        &mut state,
        json!({"op":"scene.rotate","scene":"street","entity":FIGURE,"frame":0,
               "axis":"z","degrees":90,"space":"world"}),
    );
    assert_eq!(turned["changed"], true);
    assert_eq!(
        figure_scale(&state),
        [2.0, 1.0, 1.0],
        "the rotation leaves the scale"
    );
    // Translate after the visit: a drag of the x arrow equals `scene.translate`.
    tool(&mut state, "scale", "local");
    tool(&mut state, "translate", "world");
    let shown = overlay(&mut state);
    let camera = camera(&mut state, 0);
    let (_, origin) = world(&mut state, "figure", 0);
    let pressed = press(&mut state, handle(&shown, "x")["to"].clone()).unwrap();
    assert_eq!(pressed["tool"], "translate");
    let (u, v) = camera.project([origin[0] + 0.5, origin[1], origin[2]]);
    ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(u, v)}),
    );
    let up = ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert!(up.get("factor").is_none() && up.get("degrees").is_none());
    assert_eq!(
        figure_scale(&state),
        [2.0, 1.0, 1.0],
        "a translation leaves the scale"
    );
    assert_eq!(
        node_field(&state, "figure", "localRotation")["z"]
            .as_f64()
            .unwrap()
            > 0.0,
        true,
        "and the rotation"
    );
}

#[test]
fn a_scale_is_one_edit_scene_entry_and_settles_an_open_slider_like_every_labelled_command() {
    // The rule is the existing one (docs/features/scene-gizmo.md, "Liquidação de slider aberto"):
    // a command with a history label settles an open slider gesture first, before it is
    // validated. This pins what `scene.scale` does with it so it is not an unknown: the slider
    // becomes its own entry, and a refused scale does not undo that.
    let mut state = EditorState::demo();
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle(MINIMAL)}),
    );
    select(&mut state, FIGURE);
    ok(
        &mut state,
        json!({"op":"property.gesture.begin","property":"zoom"}),
    );
    ok(
        &mut state,
        json!({"op":"property.gesture.update","value":120}),
    );
    let depth = undo_depth(&state);
    // A refused scale: the slider was settled anyway (one entry), nothing else changed.
    let error = err(&mut state, scale_request(FIGURE, "x", -1.0, "local"));
    assert_eq!(reason(&error), "factor_not_positive");
    assert!(
        state.open_property_gesture().is_none(),
        "the slider was settled"
    );
    assert_eq!(undo_depth(&state), depth + 1, "as its own entry");
    assert_eq!(figure_scale(&state), [1.0, 1.0, 1.0]);
    // A valid scale is one more entry, separate from the slider's.
    ok(&mut state, scale_request(FIGURE, "x", 2.0, "local"));
    assert_eq!(undo_depth(&state), depth + 2);
}

// --- animation -----------------------------------------------------------------------------------

fn animated(channels: Value) -> Value {
    let mut bundle = bundle(MINIMAL);
    bundle["performanceTracks"] =
        json!([{"id": "perf-1", "instanceId": "inst-a", "channels": channels}]);
    bundle["sceneInstances"][0]["performanceTrackIds"] = json!(["perf-1"]);
    bundle
}

fn grow(node: &str, scale: f64) -> Value {
    json!({"kind": "static", "channelId": format!("grow-{node}"), "nodeId": node,
           "property": "localScale", "value": {"x": scale, "y": scale, "z": scale}})
}

fn swing(node: &str) -> Value {
    json!({"kind": "cycle", "channelId": format!("swing-{node}"), "nodeId": node,
           "property": "localRotation", "periodFrames": 48, "amplitude": 25,
           "axis": {"x": 0, "y": 0, "z": 1}, "phaseOffsetFrames": 0})
}

#[test]
fn a_node_whose_scale_is_animated_is_refused_and_nothing_is_keyed() {
    let mut state = street_of(animated(json!([grow("figure", 1.5)])));
    select(&mut state, FIGURE);
    let before = observable(&state);
    let tracks = document_of(&state)["performanceTracks"].clone();
    for axis in ["x", "uniform"] {
        let error = err(&mut state, scale_request(FIGURE, axis, 2.0, "local"));
        assert_eq!(error.code, "not_scalable");
        assert_eq!(reason(&error), "animated_scale");
        assert_eq!(observable(&state), before);
    }
    // The tool says so instead of drawing handles, and a press cannot open a drag.
    tool(&mut state, "scale", "local");
    for frame in [0, 5] {
        let shown = overlay_at(&mut state, frame);
        assert_eq!(shown["visible"], false);
        assert_eq!(shown["reason"], "animated_scale");
        assert_eq!(handle_ids(&shown), Vec::<String>::new());
    }
    let hit = ok(
        &mut state,
        json!({"op":"scene.gizmo.hit","scene":"street","frame":5,"point":point(0.5, 0.5)}),
    );
    assert_ne!(hit["hit"]["kind"], "handle");
    assert_eq!(
        document_of(&state)["performanceTracks"],
        tracks,
        "no key, no channel"
    );
    // The other tools are not affected by an animated scale: rotation is fine.
    ok(
        &mut state,
        json!({"op":"scene.rotate","scene":"street","entity":FIGURE,"frame":0,
               "axis":"z","degrees":15,"space":"world"}),
    );
}

#[test]
fn another_node_a_swinging_node_and_an_animated_parent_can_still_be_scaled() {
    // The root grows (a channel on its scale) and the figure swings (a channel on its rotation):
    // the figure's own scale has no channel, and its matrices at the frame already include both.
    const FRAME: i64 = 30;
    let mut state = street_of(animated(json!([grow("root", 2.0), swing("figure")])));
    let (before, origin) = world(&mut state, "figure", FRAME);
    let answer = ok(
        &mut state,
        json!({"op":"scene.scale","scene":"street","entity":FIGURE,"frame":FRAME,
               "axis":"y","factor":1.5,"space":"local"}),
    );
    assert_eq!(answer["changed"], true);
    let (after, after_origin) = world(&mut state, "figure", FRAME);
    assert_matrix(
        &after,
        &matmul(&before, &diag([1.0, 1.5, 1.0])),
        "animated parent and rotation",
    );
    assert_vec(after_origin, origin, "pivot");
    // The animation is still on top of the document: the channels are untouched.
    assert_eq!(
        document_of(&state)["performanceTracks"][0]["channels"],
        animated(json!([grow("root", 2.0), swing("figure")]))["performanceTracks"][0]["channels"]
    );
    // And the node that grows is the refused one.
    let error = err(
        &mut state,
        json!({"op":"scene.scale","scene":"street","entity":ROOT,"frame":FRAME,
               "axis":"x","factor":1.5,"space":"local"}),
    );
    assert_eq!(reason(&error), "animated_scale");
}

#[test]
fn a_moving_node_is_scaled_because_only_a_scale_channel_is_in_the_way() {
    let channels = json!([
        {"kind": "event", "channelId": "lift", "nodeId": "figure", "property": "localPosition",
         "startInstanceFrame": 10, "durationFrames": 20,
         "delta": {"x": 0.25, "y": 0.5, "z": 0}, "holdFinal": true},
    ]);
    const FRAME: i64 = 30;
    let mut state = street_of(animated(channels));
    let (before, origin) = world(&mut state, "figure", FRAME);
    ok(
        &mut state,
        json!({"op":"scene.scale","scene":"street","entity":FIGURE,"frame":FRAME,
               "axis":"uniform","factor":1.2,"space":"world"}),
    );
    let (after, after_origin) = world(&mut state, "figure", FRAME);
    let mut expected = before;
    for row in &mut expected {
        for value in row {
            *value *= 1.2;
        }
    }
    assert_matrix(&after, &expected, "animated position");
    assert_vec(
        after_origin,
        origin,
        "pivot (including the animated offset)",
    );
}

// --- Spark ---------------------------------------------------------------------------------------

fn with_splat(scale: V3, parent_scale: V3) -> Value {
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
    root["localScale"] = vec_value(parent_scale);
    bundle
}

const CLOUD: &str = "scene/scene-a/node/cloud-node";

/// What Spark needs of a world matrix: one uniform scale (the three column lengths agree) and
/// orthogonal axes.
fn assert_similarity(m: &M3, what: &str) {
    let columns: Vec<V3> = (0..3).map(|c| column(m, c)).collect();
    let lengths: Vec<f64> = columns.iter().map(|c| dot(*c, *c).sqrt()).collect();
    assert!(
        near(lengths[0], lengths[1], 1e-9) && near(lengths[1], lengths[2], 1e-9),
        "{what}: {lengths:?}"
    );
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        assert!(near(dot(columns[a], columns[b]), 0.0, 1e-9), "{what}");
    }
}

#[test]
fn a_splat_scales_uniformly_and_its_world_matrix_stays_a_similarity() {
    for space in ["world", "local"] {
        let mut state = street_of(with_splat([2.0, 2.0, 2.0], [1.5, 1.5, 1.5]));
        let (before, origin) = world(&mut state, "cloud-node", 0);
        ok(&mut state, scale_request(CLOUD, "uniform", 1.4, space));
        let (after, after_origin) = world(&mut state, "cloud-node", 0);
        let mut expected = before;
        for row in &mut expected {
            for value in row {
                *value *= 1.4;
            }
        }
        assert_matrix(&after, &expected, &format!("splat {space}"));
        assert_vec(after_origin, origin, "pivot");
        assert_similarity(&after, "after");
        // The plan still builds: Spark can draw it.
        ok(
            &mut state,
            json!({"op":"scene.plan","scene":"street","frame":0}),
        );
    }
}

#[test]
fn a_splat_has_no_axis_handles_and_a_non_uniform_scale_of_it_is_refused_explicitly() {
    let mut state = street_of(with_splat([2.0, 2.0, 2.0], [1.0, 1.0, 1.0]));
    select(&mut state, CLOUD);
    tool(&mut state, "scale", "local");
    let shown = overlay(&mut state);
    assert_eq!(shown["visible"], true, "{shown}");
    assert_eq!(handle_ids(&shown), ["uniform"], "only what Spark can draw");
    assert_eq!(
        disabled(&shown),
        ["x", "y", "z"].map(|id| (id.to_string(), "splat_non_uniform".to_string()))
    );
    let before = observable(&state);
    for space in ["world", "local"] {
        for axis in ["x", "y", "z"] {
            let error = err(&mut state, scale_request(CLOUD, axis, 1.5, space));
            assert_eq!(error.code, "not_scalable", "{space} {axis}");
            assert_eq!(reason(&error), "splat_non_uniform");
            assert_eq!(observable(&state), before);
        }
    }
    // A drag cannot start on a handle that does not exist, even when a script presses where it
    // would be (the hit test has nothing there).
    let camera = camera(&mut state, 0);
    let (_, origin) = world(&mut state, "cloud-node", 0);
    let tip = camera.project(along(origin, [1.0, 0.0, 0.0], camera.handle_length(origin)));
    let pressed = press(&mut state, point(tip.0, tip.1)).unwrap();
    assert_ne!(pressed["hit"]["handle"], "x");
}

#[test]
fn a_group_with_a_splat_below_it_only_scales_uniformly_and_a_plain_sibling_is_unaffected() {
    let mut state = street_of(with_splat([1.0, 1.0, 1.0], [1.0, 1.0, 1.0]));
    // `root` is the splat's parent: its axis handles are off; the figure (a sibling) has all.
    select(&mut state, ROOT);
    tool(&mut state, "scale", "local");
    let shown = overlay(&mut state);
    assert_eq!(handle_ids(&shown), ["uniform"]);
    let before = observable(&state);
    assert_eq!(
        reason(&err(&mut state, scale_request(ROOT, "x", 2.0, "local"))),
        "splat_non_uniform"
    );
    assert_eq!(observable(&state), before);
    ok(&mut state, scale_request(ROOT, "uniform", 2.0, "local"));
    let (cloud, _) = world(&mut state, "cloud-node", 0);
    assert_similarity(&cloud, "splat under a uniformly scaled parent");
    select(&mut state, FIGURE);
    assert_eq!(handle_ids(&overlay(&mut state)), ["x", "y", "z", "uniform"]);
    ok(&mut state, scale_request(FIGURE, "x", 2.0, "local"));
}

#[test]
fn a_splat_that_spark_cannot_draw_is_refused_by_the_plan_and_never_scaled() {
    for (scale, parent) in [
        ([2.0, 1.0, 1.0], [1.0, 1.0, 1.0]),
        ([1.0, 1.0, 1.0], [2.0, 1.0, 1.0]),
    ] {
        let mut state = street_of(with_splat(scale, parent));
        let before = observable(&state);
        let error = err(&mut state, scale_request(CLOUD, "uniform", 1.5, "local"));
        assert_eq!(error.code, "unsupported", "{scale:?} {parent:?}: {error:?}");
        assert!(
            error.to_string().contains("cloud-node") || format!("{error:?}").contains("cloud-node")
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
        json!({"op":"scene.gizmo.tool","tool":"scale","space":"local"}),
    );
    ok(&mut json_state, scale_request(FIGURE, "y", 1.7, "local"));
    let mut typed_state = street();
    typed_state
        .execute(Command::Scene(SceneCommand::Gizmo(GizmoCommand::Tool {
            tool: Some(GizmoTool::Scale),
            space: Some(GizmoSpace::Local),
        })))
        .unwrap();
    typed_state
        .execute(Command::Scene(SceneCommand::Gizmo(GizmoCommand::Scale {
            scene: "street".into(),
            entity: FIGURE.into(),
            at: FrameAt::Frame(0),
            axis: Handle::Y,
            factor: 1.7,
            space: Some(GizmoSpace::Local),
        })))
        .unwrap();
    assert_eq!(document_of(&json_state), document_of(&typed_state));
    assert_eq!(undo_depth(&json_state), undo_depth(&typed_state));
    // The typed command with the screen handle is refused like the JSON one.
    let error = typed_state
        .execute(Command::Scene(SceneCommand::Gizmo(GizmoCommand::Scale {
            scene: "street".into(),
            entity: FIGURE.into(),
            at: FrameAt::Frame(0),
            axis: Handle::Screen,
            factor: 2.0,
            space: None,
        })))
        .unwrap_err();
    assert_eq!(error.code, "invalid_value");
}

#[test]
fn the_capabilities_list_the_scale_command_additively() {
    let mut state = street();
    let commands = ok(&mut state, json!({"op":"scene.capabilities"}))["commands"].clone();
    let listed: Vec<&str> = commands
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    for command in [
        "scene.scale",
        "scene.rotate",
        "scene.gizmo.tool",
        "scene.translate",
        "scene.pointer.down",
    ] {
        assert!(listed.contains(&command), "{command} in {listed:?}");
    }
}

#[test]
fn a_json_lines_session_runs_the_whole_scale_flow_in_process() {
    let mut state = EditorState::demo();
    let lines = [
        json!({"op":"project.create","name":"Jsonl"}),
        json!({"op":"scene.attach","id":"street","bundle":bundle(MINIMAL)}),
        json!({"op":"scene.select","scene":"street","entities":[FIGURE]}),
        json!({"op":"scene.gizmo.tool","tool":"scale","space":"world"}),
        json!({"op":"scene.scale","scene":"street","entity":FIGURE,"frame":0,"axis":"uniform","factor":2}),
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
    assert_eq!(
        answers[4]["result"]["scale"],
        json!({"x": 2.0, "y": 2.0, "z": 2.0})
    );
    assert_eq!(figure_scale(&state), [1.0, 1.0, 1.0], "undone");
}

#[test]
fn only_nodes_have_a_scale_the_overlay_of_another_selection_has_no_scale_handles() {
    let mut state = street();
    tool(&mut state, "scale", "world");
    // Nothing selected, two selected: no handles and the reason is the existing one.
    assert_eq!(overlay(&mut state)["reason"], "no_selection");
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":[FIGURE, ROOT]}),
    );
    let shown = overlay(&mut state);
    assert_eq!(shown["reason"], "multiple_selection");
    assert_eq!(handle_ids(&shown), Vec::<String>::new());
    // A camera clip is not a node.
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":["camera/cam-main"]}),
    );
    assert_eq!(overlay(&mut state)["reason"], "not_a_node");
}
