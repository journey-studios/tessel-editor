//! P4b S2-R0: the runtime's internal reader of the two-deck plan, against plans the CORE built.
//!
//! `runtime/src/directed-transition.js` reads the wire of `ScenePlan.transition` (C0b). This test hands it the text
//! of real plans, frame by frame across the window of the repository's public `valid-cut-instance-offsets` bundle
//! (`shot-a` `[0, 40)`, `shot-b` `[40, 100)`, a 12-frame window centred on the cut), and compares what the reader
//! understood with the typed plan the core holds: the mix, the window, both decks' clocks, the sampled and held
//! values and the counts of each deck. It then breaks the core's own text one field at a time and requires that
//! the core and the reader refuse it at the same pointer.
//!
//! No GPU, no Chrome, no export: only Node, which is REQUIRED. A machine without Node fails these tests; they are
//! never skipped (a skipped interop test would prove nothing). Node is found as the renderer finds it:
//! `TESSEL_NODE`, then the `PATH`, then the usual install places.
//!
//! The transition is built through the hidden admitting importer only in the shared fixture of the S2-H tests
//! (`scene_host_union_common`), where it already lives; nothing here names the admission, and no `src` file does.
mod scene_host_union_common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use scene_host_union_common::{AFTER_CUT, BEFORE_CUT, Content, Fixture, PLAIN, Scratch};
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::DeckSide;
use tessel_editor_core::scene_plan::{
    Capability, MAX_TRANSITION_PLAN_BYTES, MAX_TRANSITION_PLAN_ITEMS, PlanError, ScenePlan,
};
use tessel_editor_media::tools::locate::{System, Tool, locate};

/// The window of the fixture: 12 frames, `[34, 46)`.
const WINDOW: std::ops::Range<i64> = 34..46;

fn node() -> PathBuf {
    locate(&System, Tool::Node)
        .expect("Node.js is required by this test (set TESSEL_NODE or put node on the PATH): it must not be skipped")
        .path
}

fn helper() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/test/directed-transition-interop.mjs");
    assert!(path.is_file(), "{} is missing", path.display());
    path
}

/// Writes each text to a file of its own and runs the reader on all of them with one Node process.
fn read(scratch: &Scratch, texts: &[String]) -> Value {
    let files: Vec<PathBuf> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let path = scratch.0.join(format!("plan-{index}.json"));
            fs::write(&path, text).unwrap();
            path
        })
        .collect();
    let output = Command::new(node())
        .arg(helper())
        .args(&files)
        .output()
        .expect("Node could not be started");
    assert!(
        output.status.success(),
        "the reader crashed: {}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).expect("the helper prints one JSON document")
}

fn close(left: &Value, right: f64, what: &str) {
    let left = left
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: not a number: {left}"));
    assert!((left - right).abs() <= 1e-12, "{what}: {left} vs {right}");
}

fn side(side: DeckSide) -> DeckSide {
    match side {
        DeckSide::From => DeckSide::To,
        DeckSide::To => DeckSide::From,
    }
}

fn ids(deck: &ScenePlan) -> (Vec<String>, usize, usize, usize, Vec<String>) {
    (
        deck.layers
            .iter()
            .map(|layer| layer.node_id.clone())
            .collect(),
        deck.splats.len(),
        deck.lights.len(),
        deck.fx.len(),
        deck.resources.iter().map(|r| r.asset_id.clone()).collect(),
    )
}

/// What the reader understood, against what the core holds, for one frame.
fn compare(plan: &ScenePlan, got: &Value) {
    let frame = plan.frame.edit_frame;
    assert_eq!(got["ok"], json!(true), "frame {frame}: {got}");
    let transition = plan.transition.as_ref().expect("a frame in the window");
    let wire = plan.to_json();
    let wire = &wire["transition"];

    let mix = &got["mix"];
    assert_eq!(mix["id"], json!(transition.id), "frame {frame}");
    assert_eq!(mix["effect"], json!(transition.effect), "frame {frame}");
    assert_eq!(mix["fromShotId"], json!(transition.from_shot_id));
    assert_eq!(mix["toShotId"], json!(transition.to_shot_id));
    assert_eq!(
        mix["primary"],
        json!(transition.primary.name()),
        "frame {frame}"
    );
    assert_eq!(mix["easing"], wire["easing"], "frame {frame}");
    close(&mix["progress"], transition.progress, "progress");
    close(&mix["amount"], transition.amount, "amount");
    assert_eq!(wire["params"][0][0], json!("curve"));
    assert_eq!(mix["curve"], wire["params"][0][1], "frame {frame}");
    assert_eq!(wire["params"][1][0], json!("intensity"));
    close(
        &mix["intensity"],
        wire["params"][1][1].as_f64().unwrap(),
        "intensity",
    );

    let window = &got["window"];
    assert_eq!(window["startFrame"], json!(transition.start_frame));
    assert_eq!(window["durationFrames"], json!(transition.duration_frames));
    assert_eq!(window["frameInWindow"], json!(transition.frame_in_window));
    assert_eq!(window["alignment"], wire["alignment"]);
    assert_eq!(window["cutFrame"], json!(transition.timing.cut()));
    assert_eq!(window["editFrame"], json!(frame));

    let timing = &got["timing"];
    assert_eq!(
        timing["editDurationFrames"],
        json!(transition.timing.edit_duration_frames)
    );
    for (key, deck) in [
        ("from", transition.timing.from),
        ("to", transition.timing.to),
    ] {
        let numbers = [
            ("shotStartFrame", deck.shot_start_frame),
            ("shotEndFrameExclusive", deck.shot_end_frame_exclusive),
            ("instanceStartFrame", deck.instance_start_frame),
            ("instanceDurationFrames", deck.instance_duration_frames),
            ("cameraInFrame", deck.camera_in_frame),
            ("cameraClipDurationFrames", deck.camera_clip_duration_frames),
            ("sampledInstanceFrame", deck.sampled_instance_frame),
            ("sampledCameraClipFrame", deck.sampled_camera_clip_frame),
        ];
        for (name, value) in numbers {
            assert_eq!(
                timing[key][name],
                json!(value),
                "frame {frame}: {key}.{name}"
            );
        }
    }

    // The clocks of each deck: the core's one rule against the reader's port of it.
    let primary = transition.clocks(transition.primary).unwrap();
    let other = transition.clocks(side(transition.primary)).unwrap();
    for (deck, clocks) in [("primary", primary), ("other", other)] {
        for (clock, sample) in [("instance", clocks.instance), ("camera", clocks.camera)] {
            let got = &got["clocks"][deck][clock];
            assert_eq!(
                got["raw"],
                json!(sample.raw),
                "frame {frame}: {deck} {clock}"
            );
            assert_eq!(
                got["sampled"],
                json!(sample.sampled),
                "frame {frame}: {deck} {clock}"
            );
            assert_eq!(
                got["held"],
                json!(sample.held),
                "frame {frame}: {deck} {clock}"
            );
        }
    }
    // What the frames carry: the primary's counters are the raw clocks, the neighbour's the samples and held flags.
    assert_eq!(
        got["clocks"]["primary"]["instance"]["raw"],
        json!(plan.frame.instance_frame)
    );
    assert_eq!(
        got["clocks"]["primary"]["camera"]["raw"],
        json!(plan.frame.camera_clip_frame)
    );
    assert_eq!(
        got["clocks"]["other"]["instance"]["sampled"],
        json!(transition.other.frame.instance_frame)
    );
    assert_eq!(
        got["clocks"]["other"]["camera"]["sampled"],
        json!(transition.other.frame.camera_clip_frame)
    );
    assert_eq!(
        got["clocks"]["other"]["instance"]["held"],
        json!(transition.held_instance)
    );
    assert_eq!(
        got["clocks"]["other"]["camera"]["held"],
        json!(transition.held_camera)
    );

    for (index, deck) in [plan, &transition.other].into_iter().enumerate() {
        let read = &got["decks"][index];
        let (planes, splats, lights, fx, resources) = ids(deck);
        assert_eq!(
            read["planes"],
            json!(planes),
            "frame {frame}: deck {index} planes"
        );
        assert_eq!(
            read["splats"],
            json!(splats),
            "frame {frame}: deck {index} splats"
        );
        assert_eq!(
            read["lights"],
            json!(lights),
            "frame {frame}: deck {index} lights"
        );
        assert_eq!(read["fx"], json!(fx), "frame {frame}: deck {index} fx");
        assert_eq!(
            read["resources"],
            json!(resources),
            "frame {frame}: deck {index} resources"
        );
    }
}

fn contents() -> Vec<(&'static str, Content)> {
    vec![
        ("plain", Content::default()),
        ("sky", Content::sky()),
        (
            "sky-and-splats",
            Content::sky().with_cloud(2, &["scene-a", "scene-b"]),
        ),
    ]
}

#[test]
fn the_reader_understands_every_frame_of_the_window_the_core_builds() {
    for (name, content) in contents() {
        let fixture = Fixture::new(&format!("reader-{name}"), &content);
        let scratch = Scratch::new(&format!("reader-{name}"));
        let plans: Vec<ScenePlan> = WINDOW.map(|frame| fixture.plan(frame)).collect();
        let texts: Vec<String> = plans.iter().map(ScenePlan::to_json_string).collect();
        let answer = read(&scratch, &texts);
        let results = answer["results"].as_array().unwrap();
        assert_eq!(results.len(), plans.len());
        for (plan, got) in plans.iter().zip(results) {
            compare(plan, got);
        }
        // The two roles of the cut are both exercised.
        let primaries: Vec<&str> = plans
            .iter()
            .map(|p| p.transition.as_ref().unwrap().primary.name())
            .collect();
        assert!(
            primaries.contains(&"from") && primaries.contains(&"to"),
            "{name}"
        );
        assert!(WINDOW.contains(&BEFORE_CUT) && WINDOW.contains(&AFTER_CUT));
    }
}

#[test]
fn a_frame_outside_the_window_is_not_a_transition_plan_and_the_ceilings_are_the_cores() {
    let fixture = Fixture::new("outside", &Content::default());
    let scratch = Scratch::new("outside");
    let outside = [PLAIN, 33, 46, 90]
        .iter()
        .map(|frame| {
            let plan = fixture.plan(*frame);
            assert!(plan.transition.is_none(), "frame {frame}");
            plan.to_json_string()
        })
        .collect::<Vec<_>>();
    let answer = read(&scratch, &outside);
    assert_eq!(answer["maxBytes"], json!(MAX_TRANSITION_PLAN_BYTES));
    assert_eq!(answer["maxItems"], json!(MAX_TRANSITION_PLAN_ITEMS));
    for got in answer["results"].as_array().unwrap() {
        assert_eq!(got["ok"], json!(false), "{got}");
        assert_eq!(got["code"], json!("invalid_plan"));
        assert_eq!(got["pointer"], json!("/transition"));
    }
}

/// One field of the core's text changed (`None` deletes it).
struct Tamper {
    pointer: &'static str,
    value: Option<Value>,
}

fn tamper(pointer: &'static str, value: Value) -> Tamper {
    Tamper {
        pointer,
        value: Some(value),
    }
}

fn remove(pointer: &'static str) -> Tamper {
    Tamper {
        pointer,
        value: None,
    }
}

fn apply(plan: &Value, change: &Tamper) -> String {
    let mut value = plan.clone();
    let (parent, key) = change.pointer.rsplit_once('/').unwrap();
    let target = value.pointer_mut(parent).unwrap_or_else(|| {
        panic!("{} is not in the plan", change.pointer);
    });
    match (&change.value, target) {
        (Some(new), Value::Object(map)) => {
            map.insert(key.to_string(), new.clone());
        }
        (Some(new), Value::Array(list)) => list[key.parse::<usize>().unwrap()] = new.clone(),
        (None, Value::Object(map)) => {
            map.remove(key)
                .unwrap_or_else(|| panic!("{} is not there", change.pointer));
        }
        _ => panic!("cannot change {}", change.pointer),
    }
    value.to_string()
}

/// What the core says about a text: the code and, for an invalid plan, the pointer.
fn core_says(text: &str) -> (String, Option<String>) {
    match ScenePlan::from_json_str(text) {
        Ok(_) => ("accepted".to_string(), None),
        Err(PlanError::Invalid { pointer, .. }) => ("invalid_plan".to_string(), Some(pointer)),
        Err(other) => (other.code().to_string(), None),
    }
}

#[test]
fn the_core_and_the_reader_refuse_the_same_broken_plan_at_the_same_pointer() {
    let fixture = Fixture::new("parity", &Content::sky());
    let scratch = Scratch::new("parity");
    let before = fixture.plan(BEFORE_CUT).to_json();
    let after = fixture.plan(AFTER_CUT).to_json();

    let cases: Vec<(&str, &Value, Tamper)> = vec![
        (
            "old wire: no alignment",
            &before,
            remove("/transition/alignment"),
        ),
        ("no timing", &before, remove("/transition/timing")),
        (
            "unknown key",
            &before,
            tamper("/transition/extra", json!(1)),
        ),
        (
            "false camera sample",
            &before,
            tamper("/transition/timing/from/sampledCameraClipFrame", json!(40)),
        ),
        (
            "false instance sample",
            &before,
            tamper("/transition/timing/to/sampledInstanceFrame", json!(2)),
        ),
        (
            "held instance flipped",
            &before,
            tamper("/transition/heldInstance", json!(false)),
        ),
        (
            "held camera flipped",
            &after,
            tamper("/transition/heldCamera", json!(false)),
        ),
        (
            "progress",
            &before,
            tamper("/transition/progress", json!(0.5)),
        ),
        ("amount", &after, tamper("/transition/amount", json!(0.25))),
        (
            "window start",
            &before,
            tamper("/transition/startFrame", json!(33)),
        ),
        (
            "swapped primary",
            &before,
            tamper("/transition/primary", json!("to")),
        ),
        (
            "zero duration",
            &before,
            tamper("/transition/durationFrames", json!(0)),
        ),
        (
            "frame outside the window",
            &before,
            tamper("/transition/frameInWindow", json!(12)),
        ),
        (
            "neighbour on the primary's clock",
            &before,
            tamper("/transition/other/frame/instanceFrame", json!(46)),
        ),
        (
            "neighbour shot",
            &before,
            tamper("/transition/other/frame/shotId", json!("shot-a")),
        ),
        (
            "neighbour size",
            &before,
            tamper("/transition/other/output/width", json!(161)),
        ),
        (
            "neighbour bundle",
            &before,
            tamper("/transition/other/source/bundleId", json!("elsewhere")),
        ),
        (
            "short instance",
            &before,
            tamper("/transition/timing/from/instanceDurationFrames", json!(49)),
        ),
        (
            "neighbour edit time",
            &before,
            tamper(
                "/transition/other/frame/editTime",
                json!({"numerator": 1, "denominator": 2}),
            ),
        ),
        (
            "primary instance time",
            &before,
            tamper(
                "/frame/instanceTime",
                json!({"numerator": 1, "denominator": 2}),
            ),
        ),
        (
            "same shot twice",
            &before,
            tamper("/transition/toShotId", json!("shot-a")),
        ),
        (
            "effect",
            &before,
            tamper("/transition/effect", json!("wipe")),
        ),
        (
            "easing",
            &before,
            tamper("/transition/easing", json!("bounce")),
        ),
        ("params", &before, tamper("/transition/params", json!([]))),
        (
            "primary non-reduced edit time",
            &before,
            tamper("/frame/editTime", json!({"numerator": 6, "denominator": 4})),
        ),
        (
            "cut moved",
            &before,
            tamper("/transition/timing/to/shotStartFrame", json!(41)),
        ),
        (
            "edit shorter than the shots",
            &before,
            tamper("/transition/timing/editDurationFrames", json!(99)),
        ),
        (
            "a transition inside the neighbour",
            &before,
            tamper("/transition/other/transition", json!({})),
        ),
        (
            "neighbour resource conflict",
            &before,
            tamper(
                "/transition/other/resources/0/contentHash",
                json!("another"),
            ),
        ),
    ];

    let texts: Vec<String> = cases
        .iter()
        .map(|(_, plan, change)| apply(plan, change))
        .collect();
    let answer = read(&scratch, &texts);
    let results = answer["results"].as_array().unwrap();

    let mut disagreements = Vec::new();
    for ((name, _, _), (text, got)) in cases.iter().zip(texts.iter().zip(results)) {
        let (code, pointer) = core_says(text);
        assert_ne!(code, "accepted", "{name}: the core accepted a broken plan");
        let js_code = got["code"].as_str().unwrap_or("accepted");
        let js_pointer = got["pointer"].as_str().map(str::to_string);
        if code != js_code || pointer != js_pointer {
            disagreements.push(format!(
                "{name}: core {code} {pointer:?}, reader {js_code} {js_pointer:?} ({})",
                got["message"]
            ));
        }
    }
    assert!(
        disagreements.is_empty(),
        "the core and the reader disagree:\n{}",
        disagreements.join("\n")
    );
}

#[test]
fn the_text_door_measures_bytes_and_the_core_has_the_same_ceiling() {
    let fixture = Fixture::new("bytes", &Content::default());
    let scratch = Scratch::new("bytes");
    let text = fixture.plan(BEFORE_CUT).to_json_string();
    let padded = |bytes: usize| format!("{text}{}", " ".repeat(bytes - text.len()));
    let at = padded(MAX_TRANSITION_PLAN_BYTES);
    let above = padded(MAX_TRANSITION_PLAN_BYTES + 1);
    // The core reads the first and refuses the second; the reader says the same of the same two texts.
    assert!(ScenePlan::from_json_str(&at).is_ok());
    assert!(ScenePlan::from_json_str(&above).is_err());
    let answer = read(&scratch, &[at, above]);
    let results = answer["results"].as_array().unwrap();
    assert_eq!(results[0]["ok"], json!(true), "{}", results[0]);
    assert_eq!(results[0]["bytes"], json!(MAX_TRANSITION_PLAN_BYTES));
    assert_eq!(results[1]["ok"], json!(false), "{}", results[1]);
    assert_eq!(results[1]["pointer"], json!("/transition"));
}

#[test]
fn the_item_ceiling_accepts_a_core_valid_plan_and_refuses_one_more_pass() {
    let fixture = Fixture::new("items", &Content::default());
    let scratch = Scratch::new("items");
    let original = fixture.plan(BEFORE_CUT);
    assert!(original.fx.is_empty());
    assert!(original.transition.as_ref().unwrap().other.fx.is_empty());
    let mut value = original.to_json();
    let arrays = ["nodes", "layers", "splats", "fx", "resources", "lights"];
    let count = |deck: &Value| -> usize {
        arrays
            .iter()
            .map(|key| deck[key].as_array().map_or(0, Vec::len))
            .sum()
    };
    let base_items = count(&value) + count(&value["transition"]["other"]);
    let pass = |index: usize| {
        json!({
            "trackId": "t", "clipId": format!("c-{index}"), "effect": "fade",
            "frameInClip": 0, "weight": 1, "amount": 1,
            "params": {"color": {"r": 0, "g": 0, "b": 0}, "amount": 1, "intensity": 1},
            "scope": {"kind": "output"}
        })
    };
    let passes: Vec<Value> = (0..MAX_TRANSITION_PLAN_ITEMS - base_items)
        .map(pass)
        .collect();
    value["fx"] = json!(passes);
    let requires: Vec<&str> = Capability::ALL
        .iter()
        .map(|cap| cap.as_str())
        .filter(|name| {
            *name == "outputFx"
                || value["requires"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == name)
        })
        .collect();
    value["requires"] = json!(requires);
    assert_eq!(count(&value) + count(&value["transition"]["other"]), 20_000);
    let plan = ScenePlan::from_json(&value).expect("the whole plan is valid in the core");
    plan.validate().unwrap();
    let exact = plan.to_json_string();
    let normalized = plan.to_json();
    let mut over = normalized.clone();
    let mut extra = over["fx"][0].clone();
    extra["clipId"] = json!(format!("c-{}", MAX_TRANSITION_PLAN_ITEMS));
    over["fx"].as_array_mut().unwrap().push(extra);
    assert_eq!(count(&over) + count(&over["transition"]["other"]), 20_001);
    let mut without_extra = over.clone();
    without_extra["fx"].as_array_mut().unwrap().pop();
    assert_eq!(without_extra, normalized, "only the last pass differs");
    let over = over.to_string();
    assert!(exact.len() < MAX_TRANSITION_PLAN_BYTES);
    assert!(over.len() < MAX_TRANSITION_PLAN_BYTES);
    assert_eq!(
        core_says(&over),
        ("invalid_plan".into(), Some("/transition".into()))
    );
    let answer = read(&scratch, &[exact, over]);
    assert_eq!(answer["results"][0]["ok"], json!(true));
    assert_eq!(answer["results"][1]["code"], json!("invalid_plan"));
    assert_eq!(answer["results"][1]["pointer"], json!("/transition"));
}
