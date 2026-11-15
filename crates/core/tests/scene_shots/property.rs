//! T21: a seeded property over random op sequences, and T22: no shot property in the catalog.

use core::prelude::v1::test;

use serde_json::{Map, Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_edit::shots::{CODES, MAX_EDIT_FRAMES, apply_shots};
use tessel_editor_core::scene_edit::{INVALID_OP, catalog};

use super::support::*;
// ---- T21: a property over random sequences ---------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: i64) -> i64 {
        (self.next() % n as u64) as i64
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as i64) as usize]
    }
}

/// Five shots: a continuous take (`a`,`b` on `ia`), two shots in step on the locked `ib`, one on `ic`.
fn property_scene() -> DirectedScene {
    build(
        110,
        &[
            sh("a", "ia", 0, 20, 0, 0),
            sh("b", "ia", 20, 45, 20, 20).carries_on(),
            sh("c", "ib", 45, 70, 15, 15),
            sh("d", "ib", 70, 90, 40, 40),
            sh("e", "ic", 90, 110, 10, 10),
        ],
        &[("ia", 80, false), ("ib", 100, true), ("ic", 60, false)],
    )
}

fn random_op(rng: &mut Rng, scene: &DirectedScene) -> Value {
    let ids: Vec<String> = scene.document()["shots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|shot| shot["id"].as_str().unwrap().to_string())
        .collect();
    let edit = edit_frames(scene);
    let frame = rng.below(edit + 60) - 15;
    let mut body = Map::new();
    match rng.below(4) {
        0 => {
            body.insert("shot".into(), json!(rng.pick(&ids)));
            body.insert("edge".into(), json!(rng.pick(&["start", "end"])));
            body.insert("frame".into(), json!(frame));
        }
        1 => {
            let index = rng.below(ids.len() as i64 - 1) as usize;
            body.insert("from".into(), json!(ids[index]));
            body.insert("to".into(), json!(ids[index + 1]));
            body.insert("frame".into(), json!(frame));
        }
        2 => {
            body.insert("shot".into(), json!(rng.pick(&ids)));
            body.insert("frames".into(), json!(rng.below(80) - 5));
        }
        _ => {
            body.insert("frames".into(), json!(rng.below(edit + 40) - 5));
        }
    }
    let name = if body.contains_key("edge") {
        "trim_shot"
    } else if body.contains_key("from") {
        "move_cut"
    } else if body.contains_key("shot") {
        "set_shot_duration"
    } else {
        "set_edit_duration"
    };
    if matches!(name, "trim_shot" | "set_shot_duration") && rng.below(10) < 3 {
        body.insert("mode".into(), json!("roll"));
    }
    match rng.below(4) {
        0 => {}
        1 => {
            body.insert("content".into(), json!("anchor"));
        }
        2 => {
            body.insert("content".into(), json!("follow"));
        }
        _ => {
            body.insert("content".into(), json!("auto"));
        }
    }
    json!({ name: body })
}

/// The same op asked at `limit` (the frame the refusal named) instead of what was asked.
fn at_limit(value: &Value, scene: &DirectedScene, limit: i64) -> Value {
    let (name, body) = value.as_object().unwrap().iter().next().unwrap();
    let mut body = body.as_object().unwrap().clone();
    match name.as_str() {
        "trim_shot" | "move_cut" => {
            body.insert("frame".into(), json!(limit));
        }
        "set_edit_duration" => {
            body.insert("frames".into(), json!(limit));
        }
        _ => {
            let shot = body["shot"].as_str().unwrap();
            let start = scene.document()["shots"]
                .as_array()
                .unwrap()
                .iter()
                .find(|candidate| candidate["id"] == shot)
                .unwrap()["editRange"]["startFrame"]
                .as_i64()
                .unwrap();
            body.insert("frames".into(), json!(limit - start));
        }
    }
    json!({ name: body })
}

/// What every accepted edit keeps true, whatever the op was.
#[track_caller]
fn assert_sound(before: &DirectedScene, after: &DirectedScene) {
    let shots = after.document()["shots"].as_array().unwrap();
    let mut ranges: Vec<(i64, i64, String)> = shots
        .iter()
        .map(|shot| {
            (
                shot["editRange"]["startFrame"].as_i64().unwrap(),
                shot["editRange"]["endFrameExclusive"].as_i64().unwrap(),
                shot["id"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    ranges.sort();
    let mut cursor = 0;
    for (start, end, id) in &ranges {
        assert_eq!(
            *start, cursor,
            "{id} leaves a gap or an overlap: {ranges:?}"
        );
        assert!(end > start, "{id} is empty");
        cursor = *end;
    }
    assert_eq!(
        cursor,
        edit_frames(after),
        "the edit is not the end of the last shot"
    );
    assert!(cursor <= MAX_EDIT_FRAMES);
    assert_eq!(
        before.document()["shots"].as_array().unwrap().len(),
        shots.len(),
        "a shot appeared or vanished"
    );
    for instance in after.document()["sceneInstances"].as_array().unwrap() {
        let id = instance["id"].as_str().unwrap();
        let frames = instance["durationFrames"].as_i64().unwrap();
        assert!(frames >= instance_frames(before, id), "{id} shrank");
        for shot in shots.iter().filter(|shot| shot["sceneInstanceId"] == id) {
            let used = shot["instanceStartFrame"].as_i64().unwrap()
                + shot["editRange"]["endFrameExclusive"].as_i64().unwrap()
                - shot["editRange"]["startFrame"].as_i64().unwrap();
            assert!(used <= frames, "{id} is shorter than what its shot shows");
        }
    }
}

#[test]
fn t21_random_sequences_pass_the_gate_refusals_change_nothing_and_the_limit_does_not_refuse() {
    let (mut accepted, mut refusals, mut twins) = (0, 0, 0);
    for seed in 1..=300_u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut scene = property_scene();
        for _ in 0..6 {
            let value = random_op(&mut rng, &scene);
            let before = scene.document().clone();
            match apply_shots(&scene, &op(value.clone())) {
                Ok(out) => {
                    assert_sound(&scene, &out.candidate);
                    if !out.changed {
                        assert_eq!(out.candidate.document(), scene.document(), "{value}");
                    }
                    accepted += 1;
                    scene = out.candidate;
                }
                Err(error) => {
                    refusals += 1;
                    assert_eq!(*scene.document(), before, "{value} changed the input");
                    assert!(CODES.contains(&error.code), "{value}: {}", error.code);
                    assert_ne!(error.code, INVALID_OP, "{value}");
                    if let Some(limit) = error.details().and_then(|d| d["limit_frame"].as_i64()) {
                        let twin = at_limit(&value, &scene, limit);
                        if let Err(again) = apply_shots(&scene, &op(twin.clone())) {
                            assert_ne!(
                                again.code, error.code,
                                "{value} refused with {}; its limit {limit} ({twin}) refuses again",
                                error.code
                            );
                        }
                        twins += 1;
                    }
                }
            }
        }
    }
    // The property is not vacuous: the generator reaches accepted ops, refusals and limits.
    assert!(accepted > 200, "accepted {accepted}");
    assert!(refusals > 200, "refusals {refusals}");
    assert!(twins > 50, "limit twins {twins}");
}

// ---- T22: no property of a shot in the catalog (D15) -----------------------------------------

#[test]
fn t22_no_shot_or_edit_path_is_in_the_property_catalog() {
    let banned = [
        "shot",
        "edit",
        "editRange",
        "instanceStartFrame",
        "cameraInFrame",
    ];
    for spec in catalog() {
        for name in [spec.name, spec.document_field] {
            let first = name.split('/').next().unwrap();
            assert!(!banned.contains(&first), "`{name}` is a shot property");
            assert!(
                !name.starts_with("shot/") && !name.starts_with("edit/"),
                "{name}"
            );
        }
        assert!(
            !spec.document_field.contains("editRange")
                && !spec.document_field.contains("editDurationFrames"),
            "{}: a property would write the edit without the ripple",
            spec.name
        );
    }
    let scene = three(20);
    for info in tessel_editor_core::scene_edit::properties_with_components(&scene) {
        let name = info.spec().name;
        assert!(
            !name.starts_with("shot/") && !name.starts_with("edit/"),
            "{name}"
        );
    }
}
