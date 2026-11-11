//! The shared half of the gizmo performance suite (F055 / BL015.12 / BL015.18 / BL015.21): the
//! journey (`docs/migration/fixtures/gizmo-perf/journey.json`) and a driver that runs it on any
//! surface that can answer a JSON request, timing and recording EVERY request. The record keeps
//! the nanoseconds of each request, but the suites that include this module judge the layout
//! counts and the results, never the time (judging time is the timing stack, BL015.23).
//!
//! Included with `#[path]` by `scene_gizmo_perf_structure.rs` and `scene_gizmo_perf_threads.rs`
//! (the core, through `headless::execute`) and by the desktop's `scene_gizmo_perf_tests.rs` (the
//! GPUI host, through `Workstation::run_op`), so the runs are literally the same script:
//! the same requests, in the same order, with the same expectations. A step a surface cannot run
//! (a window step on the core) is recorded as skipped with its reason, never as a pass.
//!
//! The driver does not know what a handle is: where to press and where to move come from the
//! overlay the surface itself returned (`to` of an axis handle, a point of a ring). What it
//! checks is the RESULT, on top of the time: the stored transform of the target node (read with
//! `scene.get`, a request that is a probe and never part of a cost), the history depths, and the
//! equalities a gesture owes (a commit is one undo entry, a cancel changes nothing, undo returns
//! the exact value before the commit, redo the exact value after it).
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::{Value, json};

pub mod core_surface;

pub const TOLERANCE: f64 = 1e-9;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

pub fn json_file(relative: &str) -> Value {
    serde_json::from_str(&read(relative)).unwrap()
}

pub const JOURNEY: &str = "docs/migration/fixtures/gizmo-perf/journey.json";
pub const MANIFEST: &str = "docs/migration/fixtures/gizmo-perf/corpus.manifest.json";
pub const EXPECTED: &str = "docs/migration/fixtures/gizmo-perf/journey.expected.json";
pub const CORPORA: [&str; 3] = ["small", "dense", "deep"];

// --- sha256 (the core has no hashing dependency and this is test support) ---------------------

pub fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut data = bytes.to_vec();
    let bits = (bytes.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bits.to_be_bytes());
    for chunk in data.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

pub fn sha256_of(relative: &str) -> String {
    sha256_hex(read(relative).as_bytes())
}

// --- the surface -------------------------------------------------------------------------------

/// What a surface answered: success with the data, or a typed failure.
#[derive(Debug, Clone)]
pub struct Reply {
    pub ok: bool,
    pub value: Value,
}

impl Reply {
    pub fn code(&self) -> Option<String> {
        (!self.ok).then(|| self.value["code"].as_str().unwrap_or("?").to_string())
    }
}

/// Where the journey runs. `layouts` is how many full plans the surface has laid out so far
/// (monotonic), when it can tell; the core and the host can, a real window cannot.
pub trait Surface {
    fn name(&self) -> &'static str;
    fn call(&mut self, request: &Value) -> Reply;
    /// A window capability (`set window size`, `invoke flow.jobs`): `None` when the surface has none.
    fn ui(&mut self, _words: &[String]) -> Option<Reply> {
        None
    }
    fn layouts(&self) -> Option<u64> {
        None
    }
    /// Steps of class `presentation` are run only when this says the surface can present.
    fn presents(&self) -> bool {
        false
    }
    /// A step whose `wait` is `presented`: block until the monitor presented the frame.
    fn wait_presented(&mut self, _frame: i64) -> Option<Reply> {
        None
    }
}

// --- the record --------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Sample {
    pub step: String,
    pub label: String,
    pub ns: u128,
    pub ok: bool,
    pub code: Option<String>,
    /// Full plans laid out by this request alone (`None` when the surface cannot count them).
    pub layouts: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct StepRecord {
    pub id: String,
    pub phase: String,
    pub class: String,
    /// Why the step did not run on this surface (never a pass).
    pub skipped: Option<String>,
    /// Stored transform of the target after the step: `[position, rotation, scale]`.
    pub transform: Option<Value>,
    pub undo_depth: Option<u64>,
    pub redo_depth: Option<u64>,
}

pub struct Run {
    pub corpus: String,
    pub steps: Vec<StepRecord>,
    pub samples: Vec<Sample>,
}

impl Run {
    pub fn samples_of<'a>(&'a self, step: &'a str) -> impl Iterator<Item = &'a Sample> {
        self.samples
            .iter()
            .filter(move |sample| sample.step == step)
    }

    /// Every sample of a request kind (`label`, e.g. `scene.gizmo.state`) in a phase.
    pub fn by_label<'a>(&'a self, label: &'a str) -> impl Iterator<Item = &'a Sample> {
        self.samples
            .iter()
            .filter(move |sample| sample.label == label)
    }

    pub fn step(&self, id: &str) -> &StepRecord {
        self.steps
            .iter()
            .find(|step| step.id == id)
            .unwrap_or_else(|| panic!("no step `{id}`"))
    }

    pub fn trace(&self) -> Value {
        let mut steps = serde_json::Map::new();
        for step in &self.steps {
            if step.skipped.is_some() {
                continue;
            }
            steps.insert(
                step.id.clone(),
                json!({"transform": step.transform, "undo": step.undo_depth, "redo": step.redo_depth}),
            );
        }
        Value::Object(steps)
    }
}

// --- the driver --------------------------------------------------------------------------------

pub struct Journey {
    pub value: Value,
}

pub struct Corpus {
    pub name: String,
    pub bundle: String,
    pub bundle_b: String,
    pub node: String,
    pub node_b: String,
    pub other: String,
    pub other_rotate: String,
}

impl Journey {
    pub fn load() -> Self {
        Self {
            value: json_file(JOURNEY),
        }
    }

    pub fn version(&self) -> u64 {
        self.value["version"].as_u64().unwrap()
    }

    pub fn frame(&self) -> i64 {
        self.value["frame"].as_i64().unwrap()
    }

    pub fn corpus(&self, name: &str) -> Corpus {
        let entry = &self.value["corpora"][name];
        let text = |key: &str| entry[key].as_str().unwrap().to_string();
        Corpus {
            name: name.to_string(),
            bundle: text("bundle"),
            bundle_b: text("bundle_b"),
            node: text("node"),
            node_b: text("node_b"),
            other: text("other"),
            other_rotate: text("other_rotate"),
        }
    }
}

fn fill(value: &Value, vars: &BTreeMap<&str, Value>) -> Value {
    match value {
        Value::String(text) if text.starts_with('$') => vars
            .get(&text[1..])
            .cloned()
            .unwrap_or_else(|| panic!("journey placeholder {text} has no value")),
        Value::Array(items) => Value::Array(items.iter().map(|item| fill(item, vars)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), fill(item, vars)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn near(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            (x.as_f64().unwrap() - y.as_f64().unwrap()).abs() <= TOLERANCE
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| near(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(key, p)| y.get(key).is_some_and(|q| near(p, q)))
        }
        _ => a == b,
    }
}

pub fn transforms_equal(a: &Value, b: &Value) -> bool {
    near(a, b)
}

struct Driver<'a, S: Surface> {
    surface: &'a mut S,
    vars: BTreeMap<&'static str, Value>,
    node: String,
    scene: String,
    samples: Vec<Sample>,
    /// (before, after) transforms of each committed gesture, newest last, for undo and redo.
    committed: Vec<(Value, Value)>,
    undone: Vec<(Value, Value)>,
    /// A scene is attached: before that there is no target to read.
    attached: bool,
}

impl<S: Surface> Driver<'_, S> {
    /// A request that is part of the journey: timed and recorded.
    fn timed(&mut self, step: &str, request: &Value) -> Reply {
        let before = self.surface.layouts();
        let started = Instant::now();
        let reply = self.surface.call(request);
        let ns = started.elapsed().as_nanos();
        let layouts = before.zip(self.surface.layouts()).map(|(a, b)| b - a);
        let label = request["op"].as_str().unwrap_or("?").to_string();
        self.samples.push(Sample {
            step: step.to_string(),
            label,
            ns,
            ok: reply.ok,
            code: reply.code(),
            layouts,
        });
        reply
    }

    /// A request that only looks (a probe): never timed, never counted.
    fn probe(&mut self, request: &Value) -> Reply {
        self.surface.call(request)
    }

    fn transform(&mut self, scene: &str, node: &str) -> Value {
        let mut out = Vec::new();
        for property in ["position", "rotation", "scale"] {
            let reply = self.probe(&json!({
                "op": "scene.get", "scene": scene, "property": format!("{node}/{property}")
            }));
            assert!(reply.ok, "scene.get {node}/{property}: {}", reply.value);
            out.push(reply.value["value"].clone());
        }
        Value::Array(out)
    }

    fn depths(&mut self) -> (u64, u64) {
        let reply = self.probe(&json!({"op": "history.status"}));
        assert!(reply.ok, "history.status: {}", reply.value);
        let status = &reply.value;
        let pick = |key: &str| {
            status
                .get(key)
                .or_else(|| status["history"].get(key))
                .and_then(Value::as_u64)
                .unwrap_or_else(|| panic!("history.status has no {key}: {status}"))
        };
        (pick("undo_depth"), pick("redo_depth"))
    }

    fn expect_overlay(&mut self, step: &Value, overlay: &Value, other_rotate: &str) {
        let id = step["id"].as_str().unwrap();
        let expect = &step["expect"];
        if let Some(reason) = expect["hidden"].as_str() {
            assert_eq!(overlay["visible"], false, "{id}: {overlay}");
            assert_eq!(overlay["reason"], reason, "{id}: {overlay}");
        }
        if let Some(key) = expect["hidden_from"].as_str() {
            assert_eq!(key, "other_rotate");
            let want = other_rotate.strip_prefix("refused:").unwrap();
            assert_eq!(overlay["visible"], false, "{id}: {overlay}");
            assert!(
                overlay["reason"]
                    .as_str()
                    .unwrap_or_default()
                    .contains(want),
                "{id}: wanted a refusal `{want}`, got {overlay}"
            );
        }
        if let Some(handles) = expect["handles"].as_array() {
            assert_eq!(overlay["visible"], true, "{id}: {overlay}");
            let mut found: Vec<String> = overlay["handles"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|handle| handle["enabled"] != false)
                .map(|handle| handle["id"].as_str().unwrap().to_string())
                .collect();
            found.sort();
            let mut wanted: Vec<String> = handles
                .iter()
                .map(|handle| handle.as_str().unwrap().to_string())
                .collect();
            wanted.sort();
            assert_eq!(found, wanted, "{id}: handles of {overlay}");
        }
    }

    fn press_point(&self, overlay: &Value, handle: &str) -> (Value, Vec<Value>) {
        let entry = overlay["handles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["id"] == handle)
            .unwrap_or_else(|| panic!("no handle `{handle}` in {overlay}"));
        if let Some(points) = entry["points"].as_array() {
            // A ring: press an eighth of the way round and sweep along the polygon.
            let at = points.len() / 8;
            let ring: Vec<Value> = points[at..]
                .iter()
                .filter(|p| !p.is_null())
                .cloned()
                .collect();
            return (ring[0].clone(), ring);
        }
        (entry["to"].clone(), Vec::new())
    }

    fn drag(&mut self, step: &Value, moves_cap: usize) {
        let id = step["id"].as_str().unwrap().to_string();
        let handle = step["handle"].as_str().unwrap().to_string();
        let scene = self.scene.clone();
        let node = self.node.clone();
        let frame = self.vars["frame"].clone();
        let tool = json!({"op": "scene.gizmo.tool", "tool": step["tool"], "space": step["space"]});
        let set = self.timed(&id, &tool);
        assert!(set.ok, "{id}: {}", set.value);
        let before = self.transform(&scene, &node);
        let (undo_before, _) = self.depths();
        let shown = self.timed(
            &id,
            &json!({"op": "scene.gizmo.state", "scene": scene, "frame": frame}),
        );
        assert!(shown.ok, "{id}: {}", shown.value);
        let overlay = shown.value["overlay"].clone();
        let (start, ring) = self.press_point(&overlay, &handle);
        let down = self.timed(
            &id,
            &json!({"op": "scene.pointer.down", "scene": scene, "frame": frame, "point": start}),
        );
        assert!(down.ok, "{id}: press refused: {}", down.value);
        assert_eq!(down.value["dragging"], true, "{id}: {}", down.value);
        assert_eq!(down.value["hit"]["handle"], handle.as_str(), "{id}");
        let moves = (step["moves"].as_u64().unwrap() as usize).min(moves_cap);
        let delta = step["delta"].as_array();
        let mut applied = 0;
        for index in 1..=moves {
            let point = if ring.is_empty() {
                let [du, dv] = [0, 1].map(|axis| {
                    delta.map_or(0.0, |d| d[axis].as_f64().unwrap()) * index as f64 / moves as f64
                });
                json!({
                    "u": start["u"].as_f64().unwrap() + du,
                    "v": start["v"].as_f64().unwrap() + dv,
                })
            } else {
                ring[index.min(ring.len() - 1)].clone()
            };
            let moved = self.timed(&id, &json!({"op": "scene.pointer.move", "point": point}));
            assert!(moved.ok, "{id}: move {index} refused: {}", moved.value);
            assert_eq!(
                moved.value["document_changed"], false,
                "{id}: a move is a candidate"
            );
            if moved.value["applied"] == true {
                applied += 1;
            }
        }
        assert!(applied > 0, "{id}: no move was applied");
        let candidate_visible = self.transform(&scene, &node);
        assert!(
            transforms_equal(&candidate_visible, &before),
            "{id}: the candidate must not be in the document before the release"
        );
        if step["end"] == "up" {
            let up = self.timed(&id, &json!({"op": "scene.pointer.up"}));
            assert!(up.ok, "{id}: release refused: {}", up.value);
            assert_eq!(up.value["document_changed"], true, "{id}: {}", up.value);
            let after = self.transform(&scene, &node);
            assert!(
                !transforms_equal(&after, &before),
                "{id}: the release left the document unchanged"
            );
            let (undo_after, _) = self.depths();
            assert_eq!(
                undo_after,
                undo_before + 1,
                "{id}: one undo entry per gesture"
            );
            self.committed.push((before, after));
            self.undone.clear();
        } else {
            let cancel = self.timed(&id, &json!({"op": "scene.pointer.cancel", "reason": "api"}));
            assert!(cancel.ok, "{id}: cancel refused: {}", cancel.value);
            let after = self.transform(&scene, &node);
            assert!(
                transforms_equal(&after, &before),
                "{id}: a cancel changes nothing"
            );
            let (undo_after, _) = self.depths();
            assert_eq!(undo_after, undo_before, "{id}: a cancel adds no undo entry");
        }
    }
}

/// Runs the journey for one corpus on `surface`. `moves_cap` bounds the moves of a drag (the
/// structural suite uses the journey's own count; a long soak may ask for fewer).
pub fn run<S: Surface>(surface: &mut S, journey: &Journey, corpus: &Corpus) -> Run {
    let root = repo_root();
    let abs = |relative: &str| {
        root.join(relative)
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    };
    let mut vars: BTreeMap<&'static str, Value> = BTreeMap::new();
    vars.insert("scene", journey.value["scene"].clone());
    vars.insert("scene_b", journey.value["scene_b"].clone());
    vars.insert("frame", json!(journey.frame()));
    vars.insert("bundle", json!(abs(&corpus.bundle)));
    vars.insert("base", json!(abs(journey.value["base"].as_str().unwrap())));
    vars.insert("bundle_b", json!(abs(&corpus.bundle_b)));
    vars.insert("node", json!(corpus.node));
    vars.insert("node_b", json!(corpus.node_b));
    vars.insert("other", json!(corpus.other));
    let mut driver = Driver {
        surface,
        vars,
        node: corpus.node.clone(),
        scene: journey.value["scene"].as_str().unwrap().to_string(),
        samples: Vec::new(),
        committed: Vec::new(),
        undone: Vec::new(),
        attached: false,
    };
    let mut steps = Vec::new();
    for raw in journey.value["steps"].as_array().unwrap() {
        let step = fill(raw, &driver.vars);
        let id = step["id"].as_str().unwrap().to_string();
        let mut record = StepRecord {
            id: id.clone(),
            phase: step["phase"].as_str().unwrap().to_string(),
            class: step["class"].as_str().unwrap().to_string(),
            skipped: None,
            transform: None,
            undo_depth: None,
            redo_depth: None,
        };
        let runs_here = step["surfaces"]
            .as_array()
            .is_none_or(|list| list.iter().any(|name| name == driver.surface.name()));
        if !runs_here {
            record.skipped = Some(format!(
                "listed for {}, not for {}",
                step["surfaces"],
                driver.surface.name()
            ));
            steps.push(record);
            continue;
        }
        if step["class"] == "presentation" && !driver.surface.presents() {
            record.skipped = Some("needs a presenting window".into());
            steps.push(record);
            continue;
        }
        if step["id"] == "switch.attach" {
            driver.scene = journey.value["scene_b"].as_str().unwrap().to_string();
            driver.node = corpus.node_b.clone();
        }
        if step["kind"] == "drag" {
            driver.drag(&step, usize::MAX);
        } else if let Some(words) = step["ui"].as_array() {
            let words: Vec<String> = words
                .iter()
                .map(|w| w.as_str().unwrap().to_string())
                .collect();
            match driver.surface.ui(&words) {
                Some(reply) => {
                    assert!(reply.ok, "{id}: ui {words:?}: {}", reply.value);
                }
                None => {
                    record.skipped = Some("the surface has no such window capability (N/A)".into())
                }
            }
        } else if step["wait"] == "presented" {
            match driver.surface.wait_presented(journey.frame()) {
                Some(reply) => assert!(reply.ok, "{id}: {}", reply.value),
                None => record.skipped = Some("this surface does not present".into()),
            }
        } else if step["move"].is_object() {
            // Moving the playhead is a window gesture (`ui move`); the op vocabulary has no equivalent.
            record.skipped = Some("playhead move is a window gesture".into());
        } else {
            if let Some(tool) = step["tool"].as_object() {
                let request =
                    json!({"op": "scene.gizmo.tool", "tool": tool["tool"], "space": tool["space"]});
                let reply = driver.timed(&id, &request);
                assert!(reply.ok, "{id}: {}", reply.value);
            }
            let request = step["op"].clone();
            let op = request["op"].as_str().unwrap().to_string();
            let reply = driver.timed(&id, &request);
            assert!(reply.ok, "{id}: {} -> {}", request, reply.value);
            if op == "scene.gizmo.state" {
                let other_rotate = corpus.other_rotate.clone();
                driver.expect_overlay(&step, &reply.value["overlay"], &other_rotate);
            }
            if op == "scene.attach" {
                driver.attached = true;
            }
            if op == "history.undo" {
                let (before, after) = driver
                    .committed
                    .pop()
                    .expect("an undo with nothing committed");
                let now = driver.transform(&driver.scene.clone(), &driver.node.clone());
                assert!(
                    transforms_equal(&now, &before),
                    "{id}: undo must restore the exact value before the commit"
                );
                driver.undone.push((before, after));
            }
            if op == "history.redo" {
                let (before, after) = driver.undone.pop().expect("a redo with nothing undone");
                let now = driver.transform(&driver.scene.clone(), &driver.node.clone());
                assert!(
                    transforms_equal(&now, &after),
                    "{id}: redo must restore the exact value after the commit"
                );
                driver.committed.push((before, after));
            }
        }
        if record.skipped.is_none() && driver.attached {
            let scene = driver.scene.clone();
            let node = driver.node.clone();
            // The target lives in the scene the step worked on; a selection of another node
            // (`select.other`, `select.none`) leaves the target's own transform to read as is.
            record.transform = Some(driver.transform(&scene, &node));
            let (undo, redo) = driver.depths();
            record.undo_depth = Some(undo);
            record.redo_depth = Some(redo);
        }
        steps.push(record);
    }
    Run {
        corpus: corpus.name.clone(),
        steps,
        samples: driver.samples,
    }
}
