//! F052 (BL015/BL042): the translation gizmo of directed scenes, through the public library API
//! and the JSON executor (in process: no window, no GPU, no Chrome, no GPUI, no pointer device).
//!
//! What this file proves, and what it does not:
//!
//! - **Consumer flow**: attach, select, read the overlay, press on a handle, move, release,
//!   undo; the same flow as a JSON Lines session.
//! - **Geometry against an oracle written here**, not against the code under test: the camera
//!   convention (a point on the optical axis is the middle of the picture, +X is to the right,
//!   +Y is up) is checked on fixed numbers, and the drag is checked by *where the node ends up*
//!   (its projected origin must be under the pointer, and a parent's rotation and scale must not
//!   change that).
//! - **Equivalence**: the document a drag leaves equals the one `scene.translate` leaves for the
//!   `world_delta` the drag reported.
//! - **Errors and invariance**: a refused, cancelled or stale interaction leaves the document,
//!   the history and the revision as they were.
//! - **Not covered**: a physical pointer, the canvas of the runtime and the window. Those are
//!   `runtime/test/gizmo-overlay.test.mjs` and the native proof (`docs/features/scene-gizmo.md`).

use std::fs;
use std::path::{Path, PathBuf};

use core::prelude::v1::test;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::Vec3;
use tessel_editor_core::scene_interaction::{
    CancelReason, FrameAt, GizmoCommand, Point, PointerCommand,
};
use tessel_editor_core::scene_project::SceneCommand;
use tessel_editor_core::{Command, CommandError, EditorState, headless};

const MINIMAL: &str = "valid-minimal-continuous";
const FIGURE: &str = "scene/scene-a/node/figure";
const BACKDROP: &str = "scene/scene-a/node/backdrop";
const ROOT: &str = "scene/scene-a/node/root";
const FIGURE_POSITION: &str = "scene/scene-a/node/figure/position";

fn bundle(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/migration/fixtures/bundles")
        .join(format!("{name}.bundle.json"));
    serde_json::from_str(&fs::read_to_string(PathBuf::from(path)).unwrap()).unwrap()
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
    ok(&mut state, json!({"op":"project.create","name":"Gizmo"}));
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

fn overlay(state: &mut EditorState) -> Value {
    ok(
        state,
        json!({"op":"scene.gizmo.state","scene":"street","frame":0}),
    )["overlay"]
        .clone()
}

fn point(u: f64, v: f64) -> Value {
    json!({"u": u, "v": v})
}

fn at(value: &Value) -> Value {
    point(value["u"].as_f64().unwrap(), value["v"].as_f64().unwrap())
}

fn handle<'a>(overlay: &'a Value, id: &str) -> &'a Value {
    overlay["handles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|handle| handle["id"] == id)
        .unwrap_or_else(|| panic!("no handle `{id}` in {overlay}"))
}

fn press(state: &mut EditorState, at: Value) -> Result<Value, CommandError> {
    run(
        state,
        json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":at}),
    )
}

// --- the oracle -------------------------------------------------------------------------------

/// The camera of the fixture at edit frame 0 (the first frame of `cam-main`).
struct Camera {
    position: [f64; 3],
    target: [f64; 3],
    fov_deg: f64,
    aspect: f64,
}

fn camera(state: &mut EditorState) -> Camera {
    camera_at(state, 0)
}

fn camera_at(state: &mut EditorState, frame: i64) -> Camera {
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

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: [f64; 3]) -> [f64; 3] {
    let length = dot(a, a).sqrt();
    [a[0] / length, a[1] / length, a[2] / length]
}

impl Camera {
    fn forward(&self) -> [f64; 3] {
        unit(sub(self.target, self.position))
    }

    /// The picture position of a world point, with the standard pinhole of a vertical field of
    /// view and +Y up.
    fn project(&self, point: [f64; 3]) -> (f64, f64) {
        let forward = self.forward();
        let right = unit(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        let relative = sub(point, self.position);
        let depth = dot(relative, forward);
        let half = (self.fov_deg.to_radians() / 2.0).tan();
        let x = dot(relative, right) / (depth * half * self.aspect);
        let y = dot(relative, up) / (depth * half);
        ((x + 1.0) / 2.0, (1.0 - y) / 2.0)
    }

    /// The direction (not normalised) of the ray through a picture point: the inverse of
    /// [`Camera::project`], derived here on its own.
    fn ray(&self, u: f64, v: f64) -> [f64; 3] {
        let forward = self.forward();
        let right = unit(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        let half = (self.fov_deg.to_radians() / 2.0).tan();
        let x = (u * 2.0 - 1.0) * half * self.aspect;
        let y = (1.0 - v * 2.0) * half;
        [
            forward[0] + right[0] * x + up[0] * y,
            forward[1] + right[1] * x + up[1] * y,
            forward[2] + right[2] * x + up[2] * y,
        ]
    }

    /// Where along the line `origin + t * direction` the point of the picture `(u, v)` lies, as
    /// the closest approach of the eye ray to that line.
    fn along(&self, (u, v): (f64, f64), origin: [f64; 3], direction: [f64; 3]) -> f64 {
        let ray = self.ray(u, v);
        let offset = sub(origin, self.position);
        let (a, b, c) = (
            dot(direction, direction),
            dot(direction, ray),
            dot(ray, ray),
        );
        let (d, e) = (dot(direction, offset), dot(ray, offset));
        // Minimise |origin + t d - (eye + s r)|: the standard two-line closest point.
        (b * e - c * d) / (b * b - a * c) * -1.0
    }

    /// The pointer position that, after grabbing the handle of the line `origin + t * direction`
    /// at `grabbed`, asks for the node to move by `delta` along it: the grabbed point follows the
    /// pointer, so the target is the grabbed point displaced by `delta`.
    fn pointer_for(
        &self,
        grabbed: &Value,
        origin: [f64; 3],
        direction: [f64; 3],
        delta: f64,
    ) -> (f64, f64) {
        let held = self.along(
            (
                grabbed["u"].as_f64().unwrap(),
                grabbed["v"].as_f64().unwrap(),
            ),
            origin,
            direction,
        );
        let t = held + delta;
        self.project([
            origin[0] + direction[0] * t,
            origin[1] + direction[1] * t,
            origin[2] + direction[2] * t,
        ])
    }
}

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

fn vec_of(value: &Value) -> [f64; 3] {
    [
        value["x"].as_f64().unwrap(),
        value["y"].as_f64().unwrap(),
        value["z"].as_f64().unwrap(),
    ]
}

/// The translation of a node's world matrix at frame 0, read from the plan.
fn world_origin(state: &mut EditorState, node: &str) -> [f64; 3] {
    world_origin_at(state, node, 0)
}

fn world_origin_at(state: &mut EditorState, node: &str, frame: i64) -> [f64; 3] {
    let plan = ok(
        state,
        json!({"op":"scene.plan","scene":"street","frame":frame}),
    )["plan"]
        .clone();
    let node = plan["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == node)
        .unwrap();
    let matrix = node["worldMatrix"].as_array().unwrap();
    [
        matrix[12].as_f64().unwrap(),
        matrix[13].as_f64().unwrap(),
        matrix[14].as_f64().unwrap(),
    ]
}

fn figure_position(state: &EditorState) -> Value {
    let document = document_of(state);
    let nodes = document["sceneDefinitions"][0]["nodes"].as_array().unwrap();
    nodes.iter().find(|node| node["id"] == "figure").unwrap()["localPosition"].clone()
}

// --- the camera convention ----------------------------------------------------------------------

#[test]
fn the_oracle_agrees_with_the_camera_convention_on_fixed_numbers() {
    let mut state = street();
    let camera = camera(&mut state);
    // The fixture: eye (0, 1.5, 8), target (0, 1, 0), fov 45, 16:9.
    assert_eq!(camera.position, [0.0, 1.5, 8.0]);
    assert_eq!(camera.target, [0.0, 1.0, 0.0]);
    let (u, v) = camera.project(camera.target);
    assert!(
        near(u, 0.5, 1e-12) && near(v, 0.5, 1e-12),
        "target is the picture centre"
    );
    let (right_u, _) = camera.project([1.0, 1.0, 0.0]);
    assert!(right_u > 0.5, "+X is to the right");
    let (_, up_v) = camera.project([0.0, 2.0, 0.0]);
    assert!(up_v < 0.5, "+Y is up (v grows downward)");
}

// --- overlay ------------------------------------------------------------------------------------

#[test]
fn the_overlay_of_a_selected_node_has_four_handles_on_the_projected_origin() {
    let mut state = street();
    let hidden = overlay(&mut state);
    assert_eq!(hidden["visible"], false);
    assert_eq!(hidden["reason"], "no_selection");
    assert_eq!(hidden["handles"], json!([]));

    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    assert_eq!(shown["visible"], true);
    assert_eq!(shown["entity"], FIGURE);
    let camera = camera(&mut state);
    let (u, v) = camera.project([0.5, 0.0, 0.0]);
    assert!(near(shown["origin"]["u"].as_f64().unwrap(), u, 1e-9));
    assert!(near(shown["origin"]["v"].as_f64().unwrap(), v, 1e-9));
    let ids: Vec<&str> = shown["handles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["x", "y", "z", "screen"]);
    // X goes right, Y goes up on the picture.
    assert!(handle(&shown, "x")["to"]["u"].as_f64().unwrap() > u);
    assert!(handle(&shown, "y")["to"]["v"].as_f64().unwrap() < v);
    // The selected image plane is outlined.
    assert_eq!(shown["outlines"][0]["entity"], FIGURE);
    assert_eq!(shown["outlines"][0]["quad"].as_array().unwrap().len(), 4);
}

#[test]
fn the_overlay_says_why_there_is_no_gizmo() {
    let mut state = street();
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":[FIGURE, BACKDROP]}),
    );
    assert_eq!(overlay(&mut state)["reason"], "multiple_selection");
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":["camera/cam-main"]}),
    );
    assert_eq!(overlay(&mut state)["reason"], "not_a_node");
    select(&mut state, "asset/figure");
    assert_eq!(overlay(&mut state)["reason"], "not_a_node");
}

// --- hit test -----------------------------------------------------------------------------------

#[test]
fn a_hit_test_prefers_a_handle_then_the_front_most_plane_then_nothing() {
    let mut state = street();
    let camera = camera(&mut state);
    let hit = |state: &mut EditorState, at: Value| {
        ok(
            state,
            json!({"op":"scene.gizmo.hit","scene":"street","frame":0,"point":at}),
        )["hit"]
            .clone()
    };
    // The figure (2x2 at x=0.5, in front of the 8x4.5 backdrop) beats the backdrop at its centre.
    let (fu, fv) = camera.project([0.5, 0.0, 0.0]);
    assert_eq!(
        hit(&mut state, point(fu, fv)),
        json!({"kind":"entity","entity":FIGURE})
    );
    // Beside the figure, still on the backdrop.
    let (bu, bv) = camera.project([-3.0, 0.0, -2.0]);
    assert_eq!(
        hit(&mut state, point(bu, bv)),
        json!({"kind":"entity","entity":BACKDROP})
    );
    // Outside the backdrop: nothing.
    assert_eq!(hit(&mut state, point(0.01, 0.01)), json!({"kind":"none"}));

    // With the figure selected, the tip of its X handle is a handle, not the figure.
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    let x = handle(&shown, "x");
    let middle = point(
        (x["from"]["u"].as_f64().unwrap() + x["to"]["u"].as_f64().unwrap()) / 2.0,
        (x["from"]["v"].as_f64().unwrap() + x["to"]["v"].as_f64().unwrap()) / 2.0,
    );
    assert_eq!(
        hit(&mut state, middle),
        json!({"kind":"handle","handle":"x","entity":FIGURE})
    );
    assert_eq!(
        hit(&mut state, at(&shown["origin"]))["handle"],
        "screen",
        "the centre wins over the axes that start there"
    );
    // A hit test never changes anything.
    assert_eq!(ok(&mut state, json!({"op":"scene.selection"}))["count"], 1);
}

// --- the drag ------------------------------------------------------------------------------------

#[test]
fn dragging_the_x_handle_moves_the_node_so_its_origin_follows_the_pointer() {
    let mut state = street();
    let camera = camera(&mut state);
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    let tip = handle(&shown, "x")["to"].clone();
    let depth_before = undo_depth(&state);
    let before = document_of(&state);

    let down = press(&mut state, at(&tip)).unwrap();
    assert_eq!(down["hit"]["handle"], "x");
    assert_eq!(down["dragging"], true);
    let gesture = down["gesture"].as_str().unwrap().to_string();
    // A press never writes the document or the history.
    assert_eq!(undo_depth(&state), depth_before);

    // The grabbed point follows the pointer: it goes where it would be with the node 1 unit
    // further along +X (the node starts at x = 0.5).
    let (u, v) = camera.pointer_for(&tip, [0.5, 0.0, 0.0], [1.0, 0.0, 0.0], 1.0);
    let (origin_u, origin_v) = camera.project([1.5, 0.0, 0.0]);
    let moved = ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(u, v)}),
    );
    assert_eq!(moved["applied"], true);
    let delta = vec_of(&moved["world_delta"]);
    assert!(
        near(delta[0], 1.0, 1e-9) && near(delta[1], 0.0, 1e-12) && near(delta[2], 0.0, 1e-12),
        "{delta:?}"
    );
    let position = vec_of(&moved["position"]);
    assert!(near(position[0], 1.5, 1e-9));
    // The candidate is not the document: the project did not change.
    assert_eq!(document_of(&state), before);
    assert_eq!(undo_depth(&state), depth_before);
    // The overlay of a move follows the candidate.
    let origin = moved["overlay"]["origin"].clone();
    assert!(
        near(origin["u"].as_f64().unwrap(), origin_u, 1e-9)
            && near(origin["v"].as_f64().unwrap(), origin_v, 1e-9)
    );
    assert_eq!(moved["overlay"]["active"]["handle"], "x");
    // ... and the gesture preview shows the node moved.
    let preview = ok(
        &mut state,
        json!({"op":"scene.evaluate","scene":"street","frame":0,"gesture":gesture}),
    );
    assert_eq!(preview["source"], "gesture");

    let up = ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(up["changed"], true);
    assert_eq!(up["document_changed"], true);
    assert_eq!(
        undo_depth(&state),
        depth_before + 1,
        "ONE undo entry for the whole drag"
    );
    assert!(near(
        figure_position(&state)["x"].as_f64().unwrap(),
        1.5,
        1e-9
    ));
    // y and z are exactly what they were: an axis drag does not leak into the other axes.
    assert_eq!(figure_position(&state)["y"].as_f64().unwrap(), 0.0);
    assert_eq!(figure_position(&state)["z"].as_f64().unwrap(), 0.0);
    // The drag is over: no gesture, no drag, and the gizmo stands where the node is now.
    assert_eq!(
        ok(&mut state, json!({"op":"scene.gesture.status"}))["active"],
        false
    );
    assert_eq!(up["overlay"]["active"], Value::Null);
    assert!(near(
        up["overlay"]["origin"]["u"].as_f64().unwrap(),
        origin_u,
        1e-9
    ));

    // Undo restores the whole drag, redo brings it back.
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(document_of(&state), before);
    ok(&mut state, json!({"op":"history.redo"}));
    assert!(near(
        figure_position(&state)["x"].as_f64().unwrap(),
        1.5,
        1e-9
    ));
}

#[test]
fn many_moves_are_one_undo_entry_and_each_is_computed_from_the_press() {
    let mut state = street();
    let camera = camera(&mut state);
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    let grabbed = handle(&shown, "y")["to"].clone();
    press(&mut state, at(&grabbed)).unwrap();
    let depth = undo_depth(&state);
    let mut last = Value::Null;
    for step in 0..40 {
        let (u, v) = camera.pointer_for(
            &grabbed,
            [0.5, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            f64::from(step) * 0.05,
        );
        last = ok(
            &mut state,
            json!({"op":"scene.pointer.move","point":point(u, v)}),
        );
    }
    // The last move asked for y = 39 * 0.05 = 1.95; earlier moves did not accumulate.
    let position = vec_of(&last["position"]);
    assert!(near(position[1], 1.95, 1e-9), "{position:?}");
    assert!(near(position[0], 0.5, 1e-12), "x is untouched by a Y drag");
    ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(undo_depth(&state), depth + 1);
}

#[test]
fn the_screen_handle_moves_in_the_plane_facing_the_camera_under_the_pointer() {
    let mut state = street();
    let camera = camera(&mut state);
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    let depth = undo_depth(&state);
    let down = press(&mut state, at(&shown["origin"])).unwrap();
    assert_eq!(down["handle"], "screen");
    let target = point(0.62, 0.41);
    let moved = ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":target}),
    );
    let delta = vec_of(&moved["world_delta"]);
    // The node stayed in the plane through its origin that faces the camera ...
    assert!(near(dot(delta, camera.forward()), 0.0, 1e-9), "{delta:?}");
    // ... and its origin is now under the pointer.
    let now = [0.5 + delta[0], delta[1], delta[2]];
    let (u, v) = camera.project(now);
    assert!(near(u, 0.62, 1e-9) && near(v, 0.41, 1e-9), "({u}, {v})");
    ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(undo_depth(&state), depth + 1);
}

#[test]
fn a_rotated_and_scaled_parent_does_not_change_where_the_node_goes() {
    let mut state = street();
    // The root group: scale 2 and a quarter turn about Z. The figure's local X is now world Y.
    let half = std::f64::consts::FRAC_1_SQRT_2;
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set": format!("{ROOT}/scale"), "value": {"x": 2, "y": 2, "z": 2}},
            {"set": format!("{ROOT}/rotation"), "value": {"x": 0, "y": 0, "z": half, "w": half}},
        ]}),
    );
    let camera = camera(&mut state);
    let before = world_origin(&mut state, "figure");
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    let grabbed = handle(&shown, "x")["to"].clone();
    press(&mut state, at(&grabbed)).unwrap();
    // 0.5 world units along +X of the scene.
    let wanted = [before[0] + 0.5, before[1], before[2]];
    let (u, v) = camera.pointer_for(&grabbed, before, [1.0, 0.0, 0.0], 0.5);
    ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(u, v)}),
    );
    ok(&mut state, json!({"op":"scene.pointer.up"}));
    let after = world_origin(&mut state, "figure");
    for axis in 0..3 {
        assert!(
            near(after[axis], wanted[axis], 1e-9),
            "axis {axis}: {after:?} vs {wanted:?}"
        );
    }
    // The document moved the LOCAL position by something else than 0.5 on x.
    let local = figure_position(&state);
    assert!(
        !near(local["x"].as_f64().unwrap(), 1.0, 1e-6),
        "local {local}"
    );
}

// --- the same result without a pointer ------------------------------------------------------------

#[test]
fn a_drag_and_scene_translate_leave_the_same_document() {
    let mut dragged = street();
    let camera = camera(&mut dragged);
    select(&mut dragged, FIGURE);
    let shown = overlay(&mut dragged);
    press(&mut dragged, at(&handle(&shown, "x")["to"])).unwrap();
    let (u, v) = camera.project([1.25, 0.0, 0.0]);
    ok(
        &mut dragged,
        json!({"op":"scene.pointer.move","point":point(u, v)}),
    );
    let up = ok(
        &mut dragged,
        json!({"op":"scene.pointer.up","point":point(u, v)}),
    );

    let mut scripted = street();
    let translated = ok(
        &mut scripted,
        json!({"op":"scene.translate","scene":"street","entity":FIGURE,"frame":0,
               "delta": up["world_delta"]}),
    );
    assert_eq!(translated["changed"], true);
    assert_eq!(translated["position"], up["position"]);
    assert_eq!(document_of(&dragged), document_of(&scripted));
    assert_eq!(undo_depth(&dragged), undo_depth(&scripted));
    // ... and undo gives back the original from either road.
    ok(&mut scripted, json!({"op":"history.undo"}));
    ok(&mut dragged, json!({"op":"history.undo"}));
    assert_eq!(document_of(&dragged), document_of(&scripted));
}

#[test]
fn scene_translate_is_one_entry_and_a_zero_delta_is_a_no_op() {
    let mut state = street();
    let depth = undo_depth(&state);
    let moved = ok(
        &mut state,
        json!({"op":"scene.translate","scene":"street","entity":FIGURE,"frame":0,
               "delta":{"x":2,"y":0,"z":0}}),
    );
    assert_eq!(moved["position"], json!({"x":2.5,"y":0.0,"z":0.0}));
    assert_eq!(undo_depth(&state), depth + 1);
    let still = observable(&state);
    let none = ok(
        &mut state,
        json!({"op":"scene.translate","scene":"street","entity":FIGURE,"frame":0,
               "delta":{"x":0,"y":0,"z":0}}),
    );
    assert_eq!(none["changed"], false);
    assert_eq!(observable(&state), still);
}

// --- selection by pointer --------------------------------------------------------------------------

#[test]
fn a_press_on_a_plane_selects_it_and_an_empty_press_clears() {
    let mut state = street();
    let camera = camera(&mut state);
    let (fu, fv) = camera.project([0.5, 0.0, 0.0]);
    let (bu, bv) = camera.project([-3.0, 0.0, -2.0]);
    let before = observable(&state);

    let pressed = press(&mut state, point(fu, fv)).unwrap();
    assert_eq!(pressed["hit"]["entity"], FIGURE);
    assert_eq!(pressed["selection_changed"], true);
    assert_eq!(pressed["dragging"], false);
    assert_eq!(
        pressed["overlay"]["visible"], true,
        "the overlay of a press shows the new gizmo"
    );
    // Selecting is session state: the document and the history did not move.
    assert_eq!(observable(&state), before);
    // Releasing without a drag is a no-op that makes no history entry.
    let up = ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(up["changed"], false);
    assert_eq!(observable(&state), before);

    // Additive toggles; a plain press on another plane replaces.
    run(
        &mut state,
        json!({"op":"scene.pointer.down","scene":"street","frame":0,
        "point":point(bu, bv),"additive":true}),
    )
    .unwrap();
    assert_eq!(ok(&mut state, json!({"op":"scene.selection"}))["count"], 2);
    press(&mut state, point(fu, fv)).unwrap();
    assert_eq!(ok(&mut state, json!({"op":"scene.selection"}))["count"], 1);

    let cleared = press(&mut state, point(0.01, 0.01)).unwrap();
    assert_eq!(cleared["hit"]["kind"], "none");
    assert_eq!(ok(&mut state, json!({"op":"scene.selection"}))["count"], 0);
    assert_eq!(observable(&state), before);
}

#[test]
fn pressing_a_handle_keeps_the_selection() {
    let mut state = street();
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    let down = press(&mut state, at(&shown["origin"])).unwrap();
    assert_eq!(down["selection_changed"], false);
    assert_eq!(
        ok(&mut state, json!({"op":"scene.selection"}))["entities"][0]["scene"],
        "street"
    );
    ok(
        &mut state,
        json!({"op":"scene.pointer.cancel","reason":"escape"}),
    );
}

// --- cancel, focus, resize -------------------------------------------------------------------------

#[test]
fn escape_blur_and_the_other_reasons_discard_the_drag_without_a_trace() {
    for reason in ["escape", "blur", "pointer_cancel", "hidden", "api"] {
        let mut state = street();
        let camera = camera(&mut state);
        select(&mut state, FIGURE);
        let shown = overlay(&mut state);
        let before = observable(&state);
        press(&mut state, at(&handle(&shown, "x")["to"])).unwrap();
        let (u, v) = camera.project([2.0, 0.0, 0.0]);
        ok(
            &mut state,
            json!({"op":"scene.pointer.move","point":point(u, v)}),
        );

        let cancelled = ok(
            &mut state,
            json!({"op":"scene.pointer.cancel","reason":reason}),
        );
        assert_eq!(cancelled["reason"], reason);
        assert_eq!(cancelled["dragging"], false);
        assert_eq!(cancelled["document_changed"], false);
        assert_eq!(observable(&state), before, "{reason}");
        assert_eq!(
            ok(&mut state, json!({"op":"scene.gesture.status"}))["active"],
            false
        );
        // A later release or move has nothing to act on.
        assert_eq!(
            ok(&mut state, json!({"op":"scene.pointer.up"}))["noop"],
            true
        );
        assert_eq!(
            err(
                &mut state,
                json!({"op":"scene.pointer.move","point":point(u, v)})
            )
            .code,
            "no_drag"
        );
        // The gizmo is back where the document says.
        assert_eq!(cancelled["overlay"]["active"], Value::Null);
    }
}

#[test]
fn escape_with_nothing_to_cancel_is_not_an_error() {
    let mut state = street();
    let before = observable(&state);
    let answer = ok(
        &mut state,
        json!({"op":"scene.pointer.cancel","reason":"escape"}),
    );
    assert_eq!(answer["noop"], true);
    assert_eq!(answer["changed"], false);
    assert_eq!(observable(&state), before);
    let bad = err(
        &mut state,
        json!({"op":"scene.pointer.cancel","reason":"sneeze"}),
    );
    assert_eq!(bad.code, "invalid_value");
}

#[test]
fn a_new_press_discards_a_drag_whose_release_was_lost() {
    let mut state = street();
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    let before = observable(&state);
    let first = press(&mut state, at(&handle(&shown, "x")["to"])).unwrap();
    let second = press(&mut state, at(&handle(&shown, "y")["to"])).unwrap();
    assert_eq!(second["abandoned"]["gesture"], first["gesture"]);
    assert_eq!(second["handle"], "y");
    assert_ne!(second["gesture"], first["gesture"]);
    ok(&mut state, json!({"op":"scene.pointer.cancel"}));
    assert_eq!(observable(&state), before);
}

#[test]
fn the_pointer_space_is_the_picture_so_a_resize_changes_nothing() {
    // The core never sees pixels or a window size: the same picture positions give the same
    // result however large the pane is. The adapter's resize policy is "the drag continues".
    let run_drag = || {
        let mut state = street();
        select(&mut state, FIGURE);
        let shown = overlay(&mut state);
        press(&mut state, at(&handle(&shown, "x")["to"])).unwrap();
        ok(
            &mut state,
            json!({"op":"scene.pointer.move","point":point(0.7, 0.55)}),
        );
        ok(&mut state, json!({"op":"scene.pointer.up"}));
        document_of(&state)
    };
    assert_eq!(run_drag(), run_drag());
}

// --- errors and invariance --------------------------------------------------------------------------

#[test]
fn refused_pointer_commands_leave_everything_as_it_was() {
    let mut state = street();
    select(&mut state, FIGURE);
    let before = observable(&state);

    // Move/up/cancel need a drag; a press must land on the picture; a bad point is a typed error.
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.pointer.move","point":point(0.5, 0.5)})
        )
        .code,
        "no_drag"
    );
    assert_eq!(
        press(&mut state, point(1.5, 0.5)).unwrap_err().code,
        "invalid_value"
    );
    assert_eq!(
        press(&mut state, point(-0.01, 0.5)).unwrap_err().code,
        "invalid_value"
    );
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":{"u":"x","v":0}})
        )
        .code,
        "invalid_request"
    );
    assert_eq!(
        err(&mut state, json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":{"u":0.5,"v":0.5,"w":1}})).code,
        "invalid_request"
    );
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.pointer.down","scene":"nowhere","frame":0,"point":point(0.5, 0.5)})
        )
        .code,
        "unknown_element"
    );
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.pointer.down","scene":"street","point":point(0.5, 0.5)})
        )
        .code,
        "invalid_request"
    );
    assert_eq!(observable(&state), before);

    // A move far outside the picture is a coordinate bug, not a drag.
    let shown = overlay(&mut state);
    press(&mut state, at(&handle(&shown, "x")["to"])).unwrap();
    let candidate = observable(&state);
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.pointer.move","point":point(1e9, 0.0)})
        )
        .code,
        "invalid_value"
    );
    // The drag survived the refusal and still commits.
    assert_eq!(observable(&state), candidate);
    assert_eq!(
        ok(&mut state, json!({"op":"scene.pointer.up"}))["changed"],
        false
    );
}

#[test]
fn a_press_while_another_gesture_is_open_is_refused_on_a_handle() {
    let mut state = street();
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    ok(
        &mut state,
        json!({"op":"scene.gesture.begin","scene":"street","targets":[FIGURE]}),
    );
    let before = observable(&state);
    let error = press(&mut state, at(&handle(&shown, "x")["to"])).unwrap_err();
    assert_eq!(error.code, "gesture_active");
    assert_eq!(observable(&state), before);
    // The slider's gesture is untouched.
    assert_eq!(
        ok(&mut state, json!({"op":"scene.gesture.status"}))["active"],
        true
    );
}

#[test]
fn an_edit_under_a_drag_invalidates_it_and_never_applies_the_old_candidate() {
    let mut state = street();
    let camera = camera(&mut state);
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    press(&mut state, at(&handle(&shown, "x")["to"])).unwrap();
    let (u, v) = camera.project([1.5, 0.0, 0.0]);
    ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(u, v)}),
    );

    // Something else edits the scene meanwhile.
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set": format!("{BACKDROP}/position"), "value": {"x": 0, "y": 0, "z": -3}}]}),
    );
    let after_edit = observable(&state);
    let stale = err(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(u, v)}),
    );
    assert_eq!(stale.code, "gesture_invalidated");
    assert_eq!(observable(&state), after_edit);
    // The dead drag is still the open one; a release is refused the same way, ends it, and the
    // old candidate never lands.
    let up = err(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(up.code, "gesture_invalidated");
    assert_eq!(observable(&state), after_edit);
    assert!(state.scene_interaction.drag().is_none());
    assert!(
        state.scene_interaction.gesture().is_none(),
        "the release settled the gesture under the dead drag: nothing blocks the next press"
    );
    assert_eq!(figure_position(&state)["x"].as_f64().unwrap(), 0.5);

    // A cancel settles it just as well, with no release at all.
    let shown = overlay(&mut state);
    press(&mut state, at(&handle(&shown, "x")["to"])).unwrap();
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set": format!("{BACKDROP}/position"), "value": {"x": 0, "y": 0, "z": -4}}]}),
    );
    let after_second_edit = observable(&state);
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.pointer.move","point":point(u, v)})
        )
        .code,
        "gesture_invalidated"
    );
    let cancelled = ok(&mut state, json!({"op":"scene.pointer.cancel"}));
    assert_eq!(cancelled["dragging"], false);
    assert_eq!(observable(&state), after_second_edit);
    assert!(state.scene_interaction.drag().is_none());
    assert!(state.scene_interaction.gesture().is_none());
}

#[test]
fn a_gesture_settled_elsewhere_kills_the_drag_without_reviving_it() {
    let mut state = street();
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    let down = press(&mut state, at(&handle(&shown, "x")["to"])).unwrap();
    let gesture = down["gesture"].as_str().unwrap();
    ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":gesture}),
    );
    assert_eq!(
        err(
            &mut state,
            json!({"op":"scene.pointer.move","point":point(0.6, 0.5)})
        )
        .code,
        "no_drag"
    );
    assert!(state.scene_interaction.drag().is_none());
}

#[test]
fn translate_refuses_what_has_no_position_or_cannot_be_inverted() {
    let mut state = street();
    let before = observable(&state);
    let request = |entity: &str| {
        json!({"op":"scene.translate","scene":"street","entity":entity,"frame":0,
               "delta":{"x":1,"y":0,"z":0}})
    };
    assert_eq!(
        err(&mut state, request("camera/cam-main")).code,
        "not_translatable"
    );
    assert_eq!(
        err(&mut state, request("asset/figure")).code,
        "not_translatable"
    );
    assert_eq!(
        err(&mut state, request("scene/scene-a/node/ghost")).code,
        "unknown_entity"
    );
    assert_eq!(
        err(&mut state, request("not an address")).code,
        "invalid_entity_path"
    );
    assert_eq!(
        err(&mut state, json!({"op":"scene.translate","scene":"nowhere","entity":FIGURE,"frame":0,"delta":{"x":1,"y":0,"z":0}})).code,
        "unknown_element"
    );
    assert_eq!(
        err(&mut state, json!({"op":"scene.translate","scene":"street","entity":FIGURE,"frame":0,"delta":{"x":1,"y":0}})).code,
        "invalid_request"
    );
    assert_eq!(
        err(&mut state, json!({"op":"scene.translate","scene":"street","entity":FIGURE,"delta":{"x":1,"y":0,"z":0}})).code,
        "invalid_request"
    );
    assert_eq!(observable(&state), before);

    // A zero scale makes the node's world transform singular: it cannot be moved by a world delta.
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set": format!("{ROOT}/scale"), "value": {"x": 0, "y": 1, "z": 1}}]}),
    );
    let flat = observable(&state);
    assert_eq!(err(&mut state, request(FIGURE)).code, "not_translatable");
    assert_eq!(observable(&state), flat);
    select(&mut state, FIGURE);
    assert_eq!(overlay(&mut state)["reason"], "degenerate_transform");
}

#[test]
fn translate_with_a_non_finite_delta_through_the_typed_api_is_refused() {
    let mut state = street();
    let before = observable(&state);
    let error = state
        .execute(Command::Scene(SceneCommand::Gizmo(
            GizmoCommand::Translate {
                scene: "street".into(),
                entity: FIGURE.into(),
                at: FrameAt::Frame(0),
                delta: Vec3 {
                    x: f64::NAN,
                    y: 0.0,
                    z: 0.0,
                },
            },
        )))
        .unwrap_err();
    assert_eq!(error.code, "invalid_value");
    assert_eq!(observable(&state), before);
}

#[test]
fn a_node_behind_the_camera_has_no_gizmo_and_a_pointer_hit_cannot_find_it() {
    let mut state = street();
    // Move the figure behind the eye (z = 20, the camera is at z = 8).
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set": FIGURE_POSITION, "value": {"x": 0, "y": 0, "z": 20}}]}),
    );
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    assert_eq!(shown["visible"], false);
    assert_eq!(shown["reason"], "behind_camera");
}

// --- the typed API and the JSON Lines session ----------------------------------------------------------

#[test]
fn the_typed_commands_match_the_json_ones() {
    let mut state = street();
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    let tip = &handle(&shown, "x")["to"];
    let pressed = state
        .execute(Command::Scene(SceneCommand::Gizmo(GizmoCommand::Pointer(
            PointerCommand::Down {
                scene: "street".into(),
                at: FrameAt::Frame(0),
                point: Point {
                    u: tip["u"].as_f64().unwrap(),
                    v: tip["v"].as_f64().unwrap(),
                },
                additive: false,
            },
        ))))
        .unwrap();
    assert_eq!(pressed["hit"]["handle"], "x");
    let cancelled = state
        .execute(Command::Scene(SceneCommand::Gizmo(GizmoCommand::Pointer(
            PointerCommand::Cancel {
                reason: CancelReason::Escape,
            },
        ))))
        .unwrap();
    assert_eq!(cancelled["reason"], "escape");
}

#[test]
fn a_json_lines_session_runs_the_whole_gizmo_flow_in_process() {
    let mut state = street();
    let camera = camera(&mut state);
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    let tip = at(&handle(&shown, "x")["to"]);
    let (u, v) = camera.project([1.0, 0.0, 0.0]);
    let lines = [
        json!({"op":"scene.gizmo.state","scene":"street","frame":0}),
        json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":tip}),
        json!({"op":"scene.pointer.move","point":point(u, v)}),
        json!({"op":"scene.pointer.up","point":point(u, v)}),
        json!({"op":"history.undo"}),
        json!({"op":"scene.capabilities"}),
    ]
    .iter()
    .map(Value::to_string)
    .collect::<Vec<_>>()
    .join("\n");
    let mut output = Vec::new();
    headless::run_session(&mut state, None, lines.as_bytes(), &mut output).unwrap();
    let answers: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(answers.len(), 6);
    assert!(
        answers.iter().all(|answer| answer["ok"] == true),
        "{answers:?}"
    );
    assert_eq!(answers[3]["result"]["document_changed"], true);
    let listed = answers[5]["result"]["commands"].to_string();
    for op in [
        "scene.gizmo.state",
        "scene.gizmo.hit",
        "scene.pointer.down",
        "scene.pointer.move",
        "scene.pointer.up",
        "scene.pointer.cancel",
        "scene.translate",
    ] {
        assert!(
            listed.contains(op),
            "`{op}` is not listed by scene.capabilities"
        );
    }
    assert!(
        near(figure_position(&state)["x"].as_f64().unwrap(), 0.5, 1e-12),
        "undone"
    );
}

// --- animated nodes, other outputs and other lenses ----------------------------------------------

/// The fixture with a performance track on the figure and its parent: the figure is lifted (an
/// offset added to its position), scaled (the one channel that REPLACES a transform field) and
/// the root group swings about Z, all depending on the frame.
fn animated_bundle() -> Value {
    let mut bundle = bundle(MINIMAL);
    bundle["performanceTracks"] = json!([{
        "id": "perf-1",
        "instanceId": "inst-a",
        "channels": [
            {"kind": "event", "channelId": "lift", "nodeId": "figure", "property": "localPosition",
             "startInstanceFrame": 10, "durationFrames": 20,
             "delta": {"x": 0.25, "y": 0.5, "z": 0}, "holdFinal": true},
            {"kind": "static", "channelId": "grow", "nodeId": "figure", "property": "localScale",
             "value": {"x": 1.5, "y": 1.5, "z": 1.5}},
            {"kind": "cycle", "channelId": "swing", "nodeId": "root", "property": "localRotation",
             "periodFrames": 48, "amplitude": 25, "axis": {"x": 0, "y": 0, "z": 1},
             "phaseOffsetFrames": 0},
        ],
    }]);
    bundle["sceneInstances"][0]["performanceTrackIds"] = json!(["perf-1"]);
    bundle
}

fn press_at(state: &mut EditorState, frame: i64, at: Value) -> Result<Value, CommandError> {
    run(
        state,
        json!({"op":"scene.pointer.down","scene":"street","frame":frame,"point":at}),
    )
}

fn overlay_at(state: &mut EditorState, frame: i64) -> Value {
    ok(
        state,
        json!({"op":"scene.gizmo.state","scene":"street","frame":frame}),
    )["overlay"]
        .clone()
}

#[test]
fn an_animating_node_follows_the_pointer_and_the_document_gets_the_offset_free_position() {
    const FRAME: i64 = 30;
    let mut state = street_of(animated_bundle());
    let camera = camera_at(&mut state, FRAME);
    let before = world_origin_at(&mut state, "figure", FRAME);
    // The animation really moves the node: at frame 30 it is not where frame 0 puts it.
    let at_start = world_origin_at(&mut state, "figure", 0);
    assert!(
        (before[1] - at_start[1]).abs() > 0.1,
        "the fixture animates {at_start:?} -> {before:?}"
    );
    let document_before = figure_position(&state);

    select(&mut state, FIGURE);
    let shown = overlay_at(&mut state, FRAME);
    // The gizmo stands on the evaluated origin, not on the document's `localPosition`.
    let (u, v) = camera.project(before);
    assert!(near(shown["origin"]["u"].as_f64().unwrap(), u, 1e-9));
    assert!(near(shown["origin"]["v"].as_f64().unwrap(), v, 1e-9));

    let grabbed = handle(&shown, "x")["to"].clone();
    let depth = undo_depth(&state);
    press_at(&mut state, FRAME, at(&grabbed)).unwrap();
    let (pu, pv) = camera.pointer_for(&grabbed, before, [1.0, 0.0, 0.0], 0.75);
    ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(pu, pv)}),
    );
    let up = ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(undo_depth(&state), depth + 1, "one undo entry");

    // 0.75 world units to the right at the frame the user was looking at, whatever the parent's
    // swing and the node's own scale do to its local axes.
    let after = world_origin_at(&mut state, "figure", FRAME);
    let wanted = [before[0] + 0.75, before[1], before[2]];
    for axis in 0..3 {
        assert!(
            near(after[axis], wanted[axis], 1e-9),
            "axis {axis}: {after:?} vs {wanted:?}"
        );
    }
    // The document holds the static position; the animation is still on top of it.
    assert_ne!(figure_position(&state), document_before);

    // The same result without a pointer, at the same frame.
    let mut scripted = street_of(animated_bundle());
    ok(
        &mut scripted,
        json!({"op":"scene.translate","scene":"street","entity":FIGURE,"frame":FRAME,
               "delta": up["world_delta"]}),
    );
    assert_eq!(document_of(&state), document_of(&scripted));
    ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(figure_position(&state), document_before);
}

fn with_output(width: u64, height: u64) -> Value {
    let mut bundle = bundle(MINIMAL);
    bundle["renderProfile"]["width"] = json!(width);
    bundle["renderProfile"]["height"] = json!(height);
    bundle
}

#[test]
fn a_portrait_output_gets_a_portrait_picture_space_and_the_same_drag_semantics() {
    let mut state = street_of(with_output(180, 320));
    let camera = camera(&mut state);
    assert!(near(camera.aspect, 180.0 / 320.0, 1e-12));
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    assert!(near(shown["aspect"].as_f64().unwrap(), 0.5625, 1e-12));
    let (u, v) = camera.project([0.5, 0.0, 0.0]);
    assert!(near(shown["origin"]["u"].as_f64().unwrap(), u, 1e-9));
    assert!(near(shown["origin"]["v"].as_f64().unwrap(), v, 1e-9));
    // A handle is as long on the picture (in picture heights) as on a landscape one: the length
    // is a fraction of the picture height, so the gizmo does not shrink with the output.
    let tip = &handle(&shown, "y")["to"];
    let length = (tip["v"].as_f64().unwrap() - v).abs();
    assert!(
        (0.10..0.20).contains(&length),
        "y handle is {length} picture heights long"
    );

    let hit = |state: &mut EditorState, at: Value| {
        ok(
            state,
            json!({"op":"scene.gizmo.hit","scene":"street","frame":0,"point":at}),
        )["hit"]
            .clone()
    };
    let grabbed = handle(&shown, "x")["to"].clone();
    assert_eq!(hit(&mut state, at(&grabbed))["handle"], "x");
    let away = point(
        grabbed["u"].as_f64().unwrap(),
        grabbed["v"].as_f64().unwrap() + 0.08,
    );
    assert_ne!(hit(&mut state, away)["kind"], "handle");

    let before = world_origin(&mut state, "figure");
    press(&mut state, at(&grabbed)).unwrap();
    let (pu, pv) = camera.pointer_for(&grabbed, before, [1.0, 0.0, 0.0], 0.4);
    ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(pu, pv)}),
    );
    ok(&mut state, json!({"op":"scene.pointer.up"}));
    let after = world_origin(&mut state, "figure");
    assert!(
        near(after[0], before[0] + 0.4, 1e-9)
            && near(after[1], before[1], 1e-9)
            && near(after[2], before[2], 1e-9),
        "{after:?} from {before:?}"
    );
}

#[test]
fn another_field_of_view_changes_where_the_handles_are_and_what_a_drag_means() {
    let mut narrow = street();
    let mut wide = street();
    ok(
        &mut wide,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set": "camera/cam-main/start/fov", "value": 90}]}),
    );
    let (narrow_camera, wide_camera) = (camera(&mut narrow), camera(&mut wide));
    assert!(near(wide_camera.fov_deg, 90.0, 1e-9) && near(narrow_camera.fov_deg, 45.0, 1e-9));
    select(&mut narrow, FIGURE);
    select(&mut wide, FIGURE);
    let (narrow_overlay, wide_overlay) = (overlay(&mut narrow), overlay(&mut wide));
    let (u, v) = wide_camera.project([0.5, 0.0, 0.0]);
    assert!(near(wide_overlay["origin"]["u"].as_f64().unwrap(), u, 1e-9));
    assert!(near(wide_overlay["origin"]["v"].as_f64().unwrap(), v, 1e-9));
    assert!(
        (narrow_overlay["origin"]["u"].as_f64().unwrap()
            - wide_overlay["origin"]["u"].as_f64().unwrap())
        .abs()
            > 0.01,
        "a wider lens puts the same node somewhere else on the picture"
    );
    // The same picture drag is a longer way in the world with the wider lens.
    let world_x = |state: &mut EditorState, overlay: &Value| {
        let grabbed = handle(overlay, "x")["to"].clone();
        press(state, at(&grabbed)).unwrap();
        let target = point(
            grabbed["u"].as_f64().unwrap() + 0.05,
            grabbed["v"].as_f64().unwrap(),
        );
        let moved = ok(state, json!({"op":"scene.pointer.move","point":target}));
        ok(state, json!({"op":"scene.pointer.cancel"}));
        vec_of(&moved["world_delta"])[0]
    };
    let narrow_delta = world_x(&mut narrow, &narrow_overlay);
    let wide_delta = world_x(&mut wide, &wide_overlay);
    assert!(
        wide_delta > narrow_delta * 1.5,
        "0.05 of the picture is {narrow_delta} with a 45 degree lens and {wide_delta} with 90"
    );
}

#[test]
fn a_camera_that_looks_straight_up_has_no_gizmo_and_refuses_pointer_work_without_a_trace() {
    let mut state = street();
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set": "camera/cam-main/start/target", "value": {"x": 0, "y": 100, "z": 8}}]}),
    );
    select(&mut state, FIGURE);
    let before = observable(&state);
    let shown = overlay(&mut state);
    assert_eq!(shown["visible"], false);
    assert_eq!(shown["reason"], "degenerate_camera");
    for request in [
        json!({"op":"scene.gizmo.hit","scene":"street","frame":0,"point":point(0.5, 0.5)}),
        json!({"op":"scene.pointer.down","scene":"street","frame":0,"point":point(0.5, 0.5)}),
    ] {
        assert_eq!(err(&mut state, request).code, "gizmo_unavailable");
    }
    assert_eq!(observable(&state), before);
    // The semantic road does not need a picture: the node still moves by a world delta.
    let moved = ok(
        &mut state,
        json!({"op":"scene.translate","scene":"street","entity":FIGURE,"frame":0,
               "delta":{"x":1,"y":0,"z":0}}),
    );
    assert_eq!(moved["changed"], true);
}

// --- identity and dead drags ---------------------------------------------------------------------

#[test]
fn the_overlay_stamp_names_scene_session_revision_and_frame() {
    let mut state = street();
    select(&mut state, FIGURE);
    let first = overlay(&mut state)["stamp"].clone();
    assert_eq!(first["scene"], "street");
    assert_eq!(first["frame"], 0);
    assert_eq!(first["revision"], state.project.revision);
    assert_eq!(first, state.gizmo_stamp("street", 0));
    // Reading, selecting and a drag candidate do not change it ...
    assert_eq!(overlay(&mut state)["stamp"], first);
    let shown = overlay(&mut state);
    press(&mut state, at(&handle(&shown, "x")["to"])).unwrap();
    ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(0.7, 0.6)}),
    );
    assert_eq!(
        overlay(&mut state)["stamp"],
        first,
        "a candidate is not a revision"
    );
    ok(&mut state, json!({"op":"scene.pointer.up"}));
    // ... a committed edit, another frame and another project session do.
    let edited = overlay(&mut state)["stamp"].clone();
    assert_ne!(edited["revision"], first["revision"]);
    assert_ne!(overlay_at(&mut state, 5)["stamp"]["frame"], first["frame"]);
    let mut other = street();
    select(&mut other, FIGURE);
    assert_ne!(
        overlay(&mut other)["stamp"]["session"],
        first["session"],
        "a project of its own session: a stamp of the first never matches it"
    );
}

#[test]
fn a_drag_is_dead_when_its_gesture_can_no_longer_apply_and_cancel_still_settles_it() {
    let mut state = street();
    assert!(!state.gizmo_drag_is_dead(), "no drag, nothing dead");
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    press(&mut state, at(&handle(&shown, "x")["to"])).unwrap();
    assert!(!state.gizmo_drag_is_dead());

    // An edit elsewhere (a typed value, an import, an undo) kills it ...
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[
            {"set": "scene/scene-a/node/backdrop/width", "value": 9.0}]}),
    );
    assert!(state.gizmo_drag_is_dead());
    let revision = state.project.revision;
    let depth = undo_depth(&state);
    // ... and a cancel (what the adapter sends the moment it sees that) ends the drag AND the
    // dead candidate under it without a trace, so nothing lingers waiting for a pointer event.
    let cancelled = ok(
        &mut state,
        json!({"op":"scene.pointer.cancel","reason":"unavailable"}),
    );
    assert_eq!(cancelled["reason"], "unavailable");
    assert!(!state.gizmo_drag_is_dead());
    assert!(state.scene_interaction.drag().is_none());
    assert!(state.scene_interaction.gesture().is_none());
    assert_eq!(
        (state.project.revision, undo_depth(&state)),
        (revision, depth)
    );
}

#[test]
fn a_press_released_without_moving_is_not_an_edit() {
    let mut state = street();
    select(&mut state, FIGURE);
    let shown = overlay(&mut state);
    let before = observable(&state);
    press(&mut state, at(&handle(&shown, "screen")["from"])).unwrap();
    let up = ok(&mut state, json!({"op":"scene.pointer.up"}));
    assert_eq!(up["document_changed"], false);
    assert_eq!(observable(&state), before, "no undo entry, no revision");
    // Moving there and back to the pressed point is the same: nothing to commit.
    press(&mut state, at(&handle(&shown, "screen")["from"])).unwrap();
    let start = at(&handle(&shown, "screen")["from"]);
    ok(
        &mut state,
        json!({"op":"scene.pointer.move","point":point(0.8, 0.7)}),
    );
    let back = ok(&mut state, json!({"op":"scene.pointer.up","point":start}));
    assert_eq!(back["document_changed"], false);
    assert_eq!(observable(&state), before);
}
