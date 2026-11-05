//! BL064 — the cost of building a `ScenePlan` for many frames, and the size of its JSON.
//!
//! This file is a **measurement**, not a gate: every test here is `#[ignore]` and prints one JSON
//! line per scenario (`SCENE_PLAN_PERF {...}`). Run it through `scripts/scene-plan-performance/
//! run.py`, which builds in release, repeats the process, collects the lines and the peak RSS of
//! each run, and writes the report. By hand:
//!
//! ```text
//! python3 ~/.cache/tessel-editor/cargo-serial.py test --release -p tessel-editor-core --locked \
//!     --test scene_plan_performance_bench -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Environment (all optional): `SCENE_PLAN_PERF_SCENARIO` (a name of `SHAPES`, default: all),
//! `SCENE_PLAN_PERF_REPS` (repetitions per metric, default 7; the median is reported),
//! `SCENE_PLAN_PERF_FRAMES` (frames sampled across the edit, default 480).
//!
//! What is timed, through the public API only:
//!
//! | Metric | Question |
//! |---|---|
//! | `build_ns` | one `ScenePlan::build` (what a consumer pays per frame today) |
//! | `evaluate_frame_ns` | the evaluator's share of it (`Bundle::evaluate_frame`) |
//! | `node_timeline_new_ns`, `camera_timeline_new_ns` | the setup that is rebuilt every frame |
//! | `node_state_at_ns`, `camera_state_at_ns` | the per-frame work on a timeline built once |
//! | `validate_ns` | `plan.validate()` on a built plan |
//! | `to_json_string_ns`, `from_json_str_ns` | wire cost |
//! | `json_bytes_per_frame` | size of the canonical JSON of one frame |
//! | `timeline_warm_ns` | one `PlanTimeline::build` once its shots are prepared (steady state of a long run) |
//! | `timeline_cold_ns` | the same with a new timeline for every pass: the first frame of each shot pays the preparation |
//! | `timeline_shuffled_ns` | warm, frames in a shuffled order (the cost must not depend on the order) |
//! | `*_and_json_ns` | build + `to_json_string` + drop: what an export loop pays per frame |
//!
//! Peak memory cannot be sampled here (`unsafe_code` is forbidden workspace-wide, so no counting
//! allocator); the runner reads the process's peak RSS instead.

use std::hint::black_box;
use std::time::Instant;

use serde_json::json;
// @timeline-begin
use tessel_editor_core::scene_plan::PlanTimeline;
// @timeline-end
use tessel_editor_core::scene_plan::ScenePlan;

#[path = "scene_plan_performance_support/mod.rs"]
mod support;

use support::{SHAPES, Shape, shape_named, synthetic_scene};

fn env_number(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

/// Frames spread across the whole edit, so shot changes and the far end are inside the sample.
fn sample_frames(edit_frames: u64, wanted: usize) -> Vec<i64> {
    let count = (wanted as u64).min(edit_frames).max(1);
    (0..count)
        .map(|index| (index * edit_frames / count) as i64)
        .collect()
}

/// Median over `reps` of the wall time of `run`, in nanoseconds per frame.
fn per_frame_ns(reps: usize, frames: usize, mut run: impl FnMut()) -> f64 {
    // One untimed pass: page in code and data, so the first repetition is not an outlier.
    run();
    let mut samples: Vec<f64> = (0..reps)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed().as_nanos() as f64 / frames as f64
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn measure(shape: &Shape, reps: usize, wanted_frames: usize) -> serde_json::Value {
    let scene = synthetic_scene(shape);
    let bundle = scene.bundle();
    let frames = sample_frames(shape.edit_frames, wanted_frames);
    let count = frames.len();

    // The instance and clip frames every sample maps to, so the "timeline built once" metrics ask
    // for the same frames the build does.
    let mapped: Vec<(u64, u64)> = frames
        .iter()
        .map(|frame| {
            let mapping = bundle.resolve_frame_mapping(*frame).unwrap();
            (mapping.instance_frame, mapping.camera_clip_frame)
        })
        .collect();

    let plans: Vec<ScenePlan> = frames
        .iter()
        .map(|frame| ScenePlan::build(&scene, *frame).unwrap())
        .collect();
    let texts: Vec<String> = plans.iter().map(ScenePlan::to_json_string).collect();
    let total_bytes: usize = texts.iter().map(String::len).sum();

    let build_ns = per_frame_ns(reps, count, || {
        for frame in &frames {
            black_box(ScenePlan::build(&scene, *frame).unwrap());
        }
    });
    let evaluate_frame_ns = per_frame_ns(reps, count, || {
        for frame in &frames {
            black_box(bundle.evaluate_frame(*frame).unwrap());
        }
    });
    let node_timeline_new_ns = per_frame_ns(reps, count, || {
        for _ in &frames {
            black_box(bundle.node_timeline("instance").unwrap());
        }
    });
    let camera_timeline_new_ns = per_frame_ns(reps, count, || {
        for _ in &frames {
            black_box(bundle.camera_timeline("clip").unwrap());
        }
    });
    let nodes = bundle.node_timeline("instance").unwrap();
    let node_state_at_ns = per_frame_ns(reps, count, || {
        for (instance_frame, _) in &mapped {
            black_box(nodes.state_at(*instance_frame).unwrap());
        }
    });
    let camera = bundle.camera_timeline("clip").unwrap();
    let camera_state_at_ns = per_frame_ns(reps, count, || {
        for (_, clip_frame) in &mapped {
            black_box(camera.state_at(*clip_frame as i64).unwrap());
        }
    });
    let validate_ns = per_frame_ns(reps, count, || {
        for plan in &plans {
            plan.validate().unwrap();
        }
    });
    let to_json_string_ns = per_frame_ns(reps, count, || {
        for plan in &plans {
            black_box(plan.to_json_string());
        }
    });
    let from_json_str_ns = per_frame_ns(reps, count, || {
        for text in &texts {
            black_box(ScenePlan::from_json_str(text).unwrap());
        }
    });
    // @timeline-begin
    let timeline = PlanTimeline::new(&scene);
    let timeline_warm_ns = per_frame_ns(reps, count, || {
        for frame in &frames {
            black_box(timeline.build(*frame).unwrap());
        }
    });
    let timeline_cold_ns = per_frame_ns(reps, count, || {
        let cold = PlanTimeline::new(&scene);
        for frame in &frames {
            black_box(cold.build(*frame).unwrap());
        }
    });
    let mixed = support::shuffled(&frames, 0x5eed);
    let timeline_shuffled_ns = per_frame_ns(reps, count, || {
        for frame in &mixed {
            black_box(timeline.build(*frame).unwrap());
        }
    });
    let timeline_and_json_ns = per_frame_ns(reps, count, || {
        for frame in &frames {
            black_box(timeline.build(*frame).unwrap().to_json_string());
        }
    });
    // @timeline-end
    // What an export loop pays per frame today: build, serialize, drop.
    let build_and_json_ns = per_frame_ns(reps, count, || {
        for frame in &frames {
            black_box(ScenePlan::build(&scene, *frame).unwrap().to_json_string());
        }
    });

    let setup_ns = node_timeline_new_ns + camera_timeline_new_ns;
    let bytes_per_frame = total_bytes as f64 / count as f64;
    json!({
        "scenario": shape.name,
        "shape": {
            "planes": shape.planes, "groups": shape.groups, "channels": shape.channels,
            "editFrames": shape.edit_frames, "cameraSegments": shape.camera_segments,
            "cameraLayers": shape.camera_layers, "shots": shape.shots,
            "nodes": 1 + shape.groups + shape.planes,
        },
        "sampledFrames": count,
        "reps": reps,
        "buildNs": build_ns,
        "evaluateFrameNs": evaluate_frame_ns,
        "nodeTimelineNewNs": node_timeline_new_ns,
        "cameraTimelineNewNs": camera_timeline_new_ns,
        "nodeStateAtNs": node_state_at_ns,
        "cameraStateAtNs": camera_state_at_ns,
        "validateNs": validate_ns,
        "toJsonStringNs": to_json_string_ns,
        "fromJsonStrNs": from_json_str_ns,
        "buildAndJsonNs": build_and_json_ns,
        // @timeline-begin
        "timelineWarmNs": timeline_warm_ns,
        "timelineColdNs": timeline_cold_ns,
        "timelineShuffledNs": timeline_shuffled_ns,
        "timelineAndJsonNs": timeline_and_json_ns,
        "timelineSpeedupWarm": build_ns / timeline_warm_ns,
        "timelineSpeedupCold": build_ns / timeline_cold_ns,
        "exportLoopSpeedup": build_and_json_ns / timeline_and_json_ns,
        // @timeline-end
        // Upper bound of what reusing the two timelines can save: their whole setup cost.
        "timelineSetupShareOfBuild": setup_ns / build_ns,
        "jsonBytesPerFrame": bytes_per_frame,
        "jsonBytesAllEditFrames": bytes_per_frame * shape.edit_frames as f64,
        "jsonBytesBySection": json_sections(&plans[0]),
    })
}

/// Where the bytes of one frame's JSON go: each top-level section, and the matrices alone (two of
/// 16 numbers per node), which are the part a runtime could derive from the transforms.
fn json_sections(plan: &ScenePlan) -> serde_json::Value {
    let wire = plan.to_json();
    let mut sections = serde_json::Map::new();
    for (key, value) in wire.as_object().unwrap() {
        sections.insert(key.clone(), json!(value.to_string().len()));
    }
    let matrices: usize = wire["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["localMatrix"].to_string().len() + node["worldMatrix"].to_string().len())
        .sum();
    sections.insert("nodes.matrices".into(), json!(matrices));
    serde_json::Value::Object(sections)
}

fn selected() -> Vec<Shape> {
    match std::env::var("SCENE_PLAN_PERF_SCENARIO") {
        Ok(name) => vec![shape_named(&name).unwrap_or_else(|| panic!("unknown scenario {name}"))],
        Err(_) => SHAPES.to_vec(),
    }
}

#[test]
#[ignore = "measurement: run through scripts/scene-plan-performance/run.py (release build)"]
fn report_plan_cost_and_size() {
    let reps = env_number("SCENE_PLAN_PERF_REPS", 7);
    let frames = env_number("SCENE_PLAN_PERF_FRAMES", 480);
    for shape in selected() {
        println!("SCENE_PLAN_PERF {}", measure(&shape, reps, frames));
    }
}

/// Resident set size of this process, in KiB, through `ps` (no `unsafe`, no extra dependency).
fn rss_kib() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps must be available to read the resident size");
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

/// Memory retention over a long run: plan `SCENE_PLAN_PERF_LONG` frames (default 20 000, cycling
/// over the edit), dropping each plan at once, with the direct builder (`MODE=direct`) or one
/// timeline (`MODE=timeline`; the runner's pristine-main build has no timeline). Prints RSS
/// before, after and the highest sampled: a planner that retained something per frame would climb
/// with the count. One mode per process, so the two numbers are not contaminated by each other.
#[test]
#[ignore = "measurement: run through scripts/scene-plan-performance/run.py (release build)"]
fn report_memory_over_a_long_run() {
    let long = env_number("SCENE_PLAN_PERF_LONG", 20_000);
    let mode = std::env::var("SCENE_PLAN_PERF_MODE").unwrap_or_else(|_| "direct".into());
    for shape in selected() {
        let scene = synthetic_scene(&shape);
        // @timeline-begin
        let timeline = PlanTimeline::new(&scene);
        // @timeline-end
        let ask = |index: usize| {
            let frame = (index as u64 % shape.edit_frames) as i64;
            // @timeline-begin
            if mode == "timeline" {
                return black_box(timeline.build(frame).unwrap().nodes.len());
            }
            // @timeline-end
            black_box(ScenePlan::build(&scene, frame).unwrap().nodes.len())
        };
        // Warm the allocator and the shots first, so "before" is the steady floor.
        for index in 0..shape.edit_frames.min(200) as usize {
            ask(index);
        }
        let before = rss_kib();
        let mut highest = before;
        for index in 0..long {
            ask(index);
            if index % 500 == 0 {
                highest = highest.max(rss_kib());
            }
        }
        let after = rss_kib();
        println!(
            "SCENE_PLAN_PERF_MEMORY {}",
            json!({
                "scenario": shape.name, "mode": mode, "frames": long,
                "rssKiBBefore": before, "rssKiBAfter": after, "rssKiBHighest": highest.max(after),
                // @timeline-begin
                "layouts": timeline.prepared_layouts(),
                // @timeline-end
            })
        );
    }
}
