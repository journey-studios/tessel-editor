//! S2-H, Marco B (BL082.4.4, matrix T10/T18): the BOOK of runtime slots over a plan with two
//! decks. `SceneSlots::prepare` takes the union of both decks as what the frame needs: one
//! identity, one slot, one load and one byte cost per asset id, however many nodes of either deck
//! use it, and one splat use per qualified node of either deck.
//!
//! The maps come from `scene_operation::approved_map` over a media session that REALLY approved
//! tiny PNG/SPZ files (real SHA-256), except where a test says it hand-builds one to inject a
//! wrong revision, hash or an absent entry. No runtime, no pixels: nothing here draws a transition
//! or releases a lease in a runtime; it proves the host's book (loads, releases, refusals).

mod scene_host_union_common;

use std::fs;

use scene_host_union_common::*;
use serde_json::{Value, json};
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_core::splat;
use tessel_editor_render::scene::{DEFAULT_FRAME_SPLATS, import_gated};
use tessel_editor_render::scene_operation::approved_map;
use tessel_editor_render::{ResolvedResource, SceneResourceMap, SceneSlots};

const FILES: [(&str, &str); 3] = [
    ("backdrop", "assets/backdrop.png"),
    ("figure", "assets/figure.png"),
    ("sky", "neighbour/sky.png"),
];
/// Decoded RGBA of the three pictures, by hand: 64x36x4, 32x32x4, 8x8x4.
const BYTES: [u64; 3] = [9216, 4096, 256];
const UNION_BYTES: u64 = 13568;

/// The three pictures' identity, where the book keeps them, and whether each is confirmed.
type Book = (usize, u64, Vec<(Option<String>, bool)>);

struct Truth {
    hashes: Vec<String>,
}

impl Truth {
    fn of(fixture: &Fixture) -> Self {
        Self {
            hashes: FILES
                .iter()
                .map(|(_, relative)| hash_of(&fixture.path(relative)))
                .collect(),
        }
    }

    fn book(&self, slots: &SceneSlots, generation: u64) -> Book {
        let entries = FILES
            .iter()
            .zip(&self.hashes)
            .map(|((asset, _), hash)| {
                let slot = slots
                    .slot_of(generation, asset, "rev-1", hash)
                    .map(str::to_string);
                let ready = slot.as_deref().is_some_and(|slot| slots.is_ready(slot));
                (slot, ready)
            })
            .collect();
        (slots.occupied(), slots.generation(), entries)
    }
}

fn ids(value: &Value) -> Vec<String> {
    value["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["asset_id"].as_str().unwrap().to_string())
        .collect()
}

/// `map` with every entry passed through `tweak` (which may drop it by returning `false`).
fn rebuilt(
    map: &SceneResourceMap,
    generation: u64,
    tweak: impl Fn(&str, &mut ResolvedResource) -> bool,
) -> SceneResourceMap {
    let mut out = SceneResourceMap::new(generation);
    for (asset, _) in FILES {
        if let Some(entry) = map.get(asset) {
            let mut entry = entry.clone();
            if tweak(asset, &mut entry) {
                out.insert(asset, entry).unwrap();
            }
        }
    }
    out
}

fn confirm(slots: &mut SceneSlots, pending: &[&str]) {
    for slot in pending {
        slots.mark_ready(slot);
    }
}

#[test]
fn a_shared_and_an_exclusive_resource_are_one_entry_each_and_load_once_across_the_cut() {
    let fixture = Fixture::new("slots-union", &Content::sky());
    let truth = Truth::of(&fixture);
    let session = fixture.approve();
    let before = fixture.plan(BEFORE_CUT);
    let map = approved_map(&before, PROJECT_SCENE, &session).unwrap();
    assert_eq!(map.generation(), 1);
    let mut slots = SceneSlots::default();

    let first = slots.prepare(&before, &map).unwrap();
    // Union size 3 (the primary deck alone has 2): one load per asset id, ascending, in order.
    let loaded: Vec<(&str, &str)> = first
        .loads
        .iter()
        .map(|load| {
            (
                load.slot.as_str(),
                load.message["asset"]["scene"]["asset_id"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        loaded,
        [
            ("scene-res-0", "backdrop"),
            ("scene-res-1", "figure"),
            ("scene-res-2", "sky")
        ]
    );
    assert_eq!(slots.occupied(), 3);
    assert_eq!(slots.generation(), 1);
    assert!(first.releases.is_empty() && first.waiting.is_empty());
    assert_eq!(ids(&first.layer), ["backdrop", "figure", "sky"]);
    // The load message of the neighbour's own picture: identity, kind and declared size.
    let sky = &first.loads[2].message["asset"];
    assert_eq!(sky["kind"], "still");
    assert_eq!(sky["meta"], json!({"width": 8, "height": 8.0}));
    assert_eq!(
        sky["scene"],
        json!({"asset_id": "sky", "revision_id": "rev-1", "content_hash": truth.hashes[2], "generation": 1})
    );
    // The references of the layer say where each picture is, by slot.
    assert_eq!(first.layer["resources"][2]["slot"], "scene-res-2");

    // The same frame again: nothing new to load, the three are still waiting for the runtime.
    let again = slots.prepare(&before, &map).unwrap();
    assert!(again.loads.is_empty() && again.releases.is_empty());
    assert_eq!(
        again.pending_slots(),
        ["scene-res-0", "scene-res-1", "scene-res-2"]
    );
    confirm(&mut slots, &again.pending_slots());

    // After the cut the roles swap and the union is the same: still no load, no release.
    let after = fixture.plan(AFTER_CUT);
    assert!(after.resource("sky").is_some());
    let swapped = slots.prepare(&after, &map).unwrap();
    assert!(swapped.loads.is_empty() && swapped.waiting.is_empty() && swapped.releases.is_empty());
    assert!(swapped.pending_slots().is_empty());
    assert_eq!(swapped.layer["resources"], first.layer["resources"]);
    assert_eq!(slots.occupied(), 3);
    assert_eq!(
        truth.book(&slots, 1),
        (
            3,
            1,
            vec![
                (Some("scene-res-0".into()), true),
                (Some("scene-res-1".into()), true),
                (Some("scene-res-2".into()), true)
            ]
        )
    );
}

#[test]
fn a_newer_generation_releases_what_the_older_one_held_and_an_older_map_stays_refused() {
    let fixture = Fixture::new("slots-generation", &Content::sky());
    let truth = Truth::of(&fixture);
    let plan = fixture.plan(BEFORE_CUT);
    let mut slots = SceneSlots::default();
    let old = SceneResourceMap::from_root(1, &plan, &fixture.root).unwrap();
    let first = slots.prepare(&plan, &old).unwrap();
    confirm(&mut slots, &first.pending_slots());

    let new = SceneResourceMap::from_root(5, &plan, &fixture.root).unwrap();
    let second = slots.prepare(&plan, &new).unwrap();
    let released: Vec<&str> = second
        .releases
        .iter()
        .map(|message| message["id"].as_str().unwrap())
        .collect();
    assert_eq!(released, ["scene-res-0", "scene-res-1", "scene-res-2"]);
    assert_eq!(second.loads.len(), 3);
    assert_eq!(
        (
            slots.generation(),
            slots.occupied(),
            second.loads[2].message["asset"]["scene"]["generation"].clone()
        ),
        (5, 3, json!(5))
    );
    let held = truth.book(&slots, 5);

    let error = slots.prepare(&plan, &old).unwrap_err();
    assert_eq!(error.code, "resource_stale");
    assert_eq!(
        error.details.unwrap(),
        json!({"generation": 1, "current": 5})
    );
    assert_eq!(truth.book(&slots, 5), held, "a stale map changes nothing");
}

#[test]
fn every_refusal_about_one_resource_keeps_its_code_names_the_neighbour_and_changes_nothing() {
    let fixture = Fixture::new("slots-atomic", &Content::sky());
    let truth = Truth::of(&fixture);
    let session = fixture.approve();
    let plan = fixture.plan(BEFORE_CUT);
    let good = approved_map(&plan, PROJECT_SCENE, &session).unwrap();
    let sky = context("sky", &[("other", "sky-plane")]);
    let tail = "[asset `sky` used by: other deck node `sky-plane`]";

    let mut slots = SceneSlots::default();
    let first = slots.prepare(&plan, &good).unwrap();
    confirm(&mut slots, &first.pending_slots());
    let held = truth.book(&slots, 1);
    assert_eq!(held.0, 3);

    // Another revision, another hash, and an absent entry: all of the neighbour's picture.
    let revision = rebuilt(&good, 1, |asset, entry| {
        if asset == "sky" {
            entry.revision_id = "rev-9".into();
        }
        true
    });
    let hash = rebuilt(&good, 1, |asset, entry| {
        if asset == "sky" {
            entry.content_hash = "not-the-hash".into();
        }
        true
    });
    let absent = rebuilt(&good, 1, |asset, _| asset != "sky");
    for (name, map, code) in [
        ("revision", &revision, "resource_mismatch"),
        ("hash", &hash, "resource_mismatch"),
        ("absent", &absent, "resource_unresolved"),
    ] {
        let error = slots.prepare(&plan, map).unwrap_err();
        assert_eq!(error.code, code, "{name}");
        assert!(error.message.ends_with(tail), "{name}: {}", error.message);
        let details = error.details.unwrap();
        assert_eq!(
            details["asset_id"], "sky",
            "{name}: the existing detail stays"
        );
        assert_eq!(details["resource"], sky, "{name}");
        if name != "absent" {
            // `expected`/`got` coexist with the new context.
            assert_eq!(details["expected"]["revision_id"], "rev-1", "{name}");
            assert_eq!(
                details["expected"]["content_hash"], truth.hashes[2],
                "{name}"
            );
        }
        assert_eq!(
            details["got"],
            match name {
                "revision" => json!({"revision_id": "rev-9", "content_hash": truth.hashes[2]}),
                "hash" => json!({"revision_id": "rev-1", "content_hash": "not-the-hash"}),
                _ => Value::Null,
            },
            "{name}"
        );
        assert_eq!(truth.book(&slots, 1), held, "{name}: a retained slot stays");
    }
    // Control: the good map is still just the same frame (nothing to load, nothing waiting).
    let control = slots.prepare(&plan, &good).unwrap();
    assert!(control.loads.is_empty() && control.pending_slots().is_empty());

    // A fresh book and a NEWER generation: the failure of the neighbour's picture must not leave
    // `backdrop` and `figure` half loaded, nor move the generation.
    let mut fresh = SceneSlots::default();
    let newer = rebuilt(&good, 9, |asset, entry| {
        if asset == "sky" {
            entry.content_hash = "not-the-hash".into();
        }
        true
    });
    assert_eq!(
        fresh.prepare(&plan, &newer).unwrap_err().code,
        "resource_mismatch"
    );
    assert_eq!((fresh.occupied(), fresh.generation()), (0, 0));
}

#[test]
fn a_file_removed_after_the_approval_is_refused_by_the_book_and_names_the_deck() {
    let fixture = Fixture::new("slots-removed", &Content::sky());
    let session = fixture.approve();
    let plan = fixture.plan(BEFORE_CUT);
    let map = approved_map(&plan, PROJECT_SCENE, &session).unwrap();
    fs::remove_file(fixture.path("neighbour/sky.png")).unwrap();
    assert_eq!(map.len(), 3, "the approved map still has it");

    let mut slots = SceneSlots::default();
    let error = slots.prepare(&plan, &map).unwrap_err();
    assert_eq!(error.code, "resource_unresolved");
    assert!(
        error.message.contains("is not a readable file"),
        "{}",
        error.message
    );
    assert!(
        error
            .message
            .ends_with("[asset `sky` used by: other deck node `sky-plane`]")
    );
    let details = error.details.unwrap();
    assert_eq!(details["asset_id"], "sky");
    assert_eq!(
        details["resource"],
        context("sky", &[("other", "sky-plane")])
    );
    assert_eq!((slots.occupied(), slots.generation()), (0, 0));

    // After the cut the picture is the primary deck's: same refusal, the role is the current one.
    let after = fixture.plan(AFTER_CUT);
    let map = approved_map(&after, PROJECT_SCENE, &session).unwrap();
    let error = slots.prepare(&after, &map).unwrap_err();
    assert_eq!(
        error.details.unwrap()["resource"],
        context("sky", &[("primary", "sky-plane")])
    );
}

#[test]
fn a_conflict_between_the_decks_is_refused_by_the_book_before_it_looks_at_the_map_or_moves() {
    let fixture = Fixture::new("slots-conflict", &Content::sky());
    let mut plan = fixture.plan(BEFORE_CUT);
    plan.transition.as_mut().unwrap().other.resources[1].content_hash = "another".into();
    assert_eq!(
        plan.transition.as_ref().unwrap().other.resources[1].asset_id,
        "figure"
    );
    let mut slots = SceneSlots::default();
    // An EMPTY map of a newer generation: had the book resolved anything it would say
    // `resource_unresolved`, and had it moved it would hold generation 4.
    let error = slots.prepare(&plan, &SceneResourceMap::new(4)).unwrap_err();
    assert_eq!(error.code, "invalid_plan");
    assert!(
        error.message.contains("/transition/other/resources/1"),
        "{}",
        error.message
    );
    assert_eq!(
        error.details.unwrap()["resource"],
        context("figure", &[("primary", "figure"), ("other", "figure")])
    );
    assert_eq!((slots.occupied(), slots.generation()), (0, 0));
}

#[test]
fn the_union_is_what_the_slot_and_byte_ceilings_judge_and_a_shared_picture_costs_once() {
    let fixture = Fixture::new("slots-budget", &Content::sky());
    let session = fixture.approve();
    let before = fixture.plan(BEFORE_CUT);
    let map = approved_map(&before, PROJECT_SCENE, &session).unwrap();
    assert_eq!(BYTES.iter().sum::<u64>(), UNION_BYTES);

    // Exactly the union (shared pictures once): accepted; one byte less: refused, same taxonomy.
    let mut exact = SceneSlots::new(3, UNION_BYTES);
    assert!(exact.prepare(&before, &map).is_ok());
    let mut below = SceneSlots::new(3, UNION_BYTES - 1);
    let error = below.prepare(&before, &map).unwrap_err();
    assert_eq!(error.code, "resource_budget");
    assert_eq!(
        error.details.unwrap(),
        json!({"resources": 3, "bytes": UNION_BYTES, "max_resources": 3, "max_bytes": UNION_BYTES - 1})
    );
    assert!(
        !error.message.contains("used by"),
        "a frame-wide ceiling names no asset"
    );
    assert_eq!((below.occupied(), below.generation()), (0, 0));

    // One slot fewer than the union: refused although the primary deck alone (2) would fit.
    let mut few = SceneSlots::new(2, 1 << 30);
    let error = few.prepare(&before, &map).unwrap_err();
    assert_eq!(error.code, "resource_budget");
    assert_eq!(error.details.unwrap()["resources"], 3);
    assert_eq!((few.occupied(), few.generation()), (0, 0));
    // Control: the plan of one deck (backdrop + figure = 13312 bytes, 2 slots) fits exactly.
    let plain = fixture.plan(PLAIN);
    let plain_map = approved_map(&plain, PROJECT_SCENE, &session).unwrap();
    assert!(
        SceneSlots::new(2, 13312)
            .prepare(&plain, &plain_map)
            .is_ok()
    );
    assert!(
        SceneSlots::new(2, 13311)
            .prepare(&plain, &plain_map)
            .is_err()
    );
}

#[test]
fn two_decks_whose_sizes_overflow_are_refused_as_a_budget_instead_of_wrapping() {
    let fixture = Fixture::new("slots-overflow", &Content::sky());
    let session = fixture.approve();
    let mut plan = fixture.plan(BEFORE_CUT);
    let map = approved_map(&plan, PROJECT_SCENE, &session).unwrap();
    // The neighbour declares a picture whose decoded size saturates u64; the primary's own
    // pictures add to it. Summing them with `+` panicked in debug and wrapped to a small number in
    // release (and was accepted).
    {
        let sky = &mut plan.transition.as_mut().unwrap().other.resources[2];
        assert_eq!(sky.asset_id, "sky");
        sky.width_px = u64::MAX;
        sky.height_px = 1.0e300;
    }
    let mut slots = SceneSlots::default();
    let error = slots.prepare(&plan, &map).unwrap_err();
    assert_eq!(error.code, "resource_budget");
    assert_eq!(error.details.unwrap()["bytes"], json!(u64::MAX));
    assert_eq!((slots.occupied(), slots.generation()), (0, 0));
}

#[test]
fn a_huge_incoming_picture_cannot_wrap_the_eviction_arithmetic() {
    let fixture = Fixture::new("slots-incoming", &Content::sky());
    let session = fixture.approve();
    let plain = fixture.plan(PLAIN);
    let plain_map = approved_map(&plain, PROJECT_SCENE, &session).unwrap();
    // A book whose byte ceiling is the largest number: it already holds two pictures (13312
    // bytes) when the neighbour's picture, which saturates to u64::MAX, arrives. `held + incoming`
    // overflowed there; now it saturates and the frame is judged by the ceiling like any other.
    let mut slots = SceneSlots::new(32, u64::MAX);
    slots.prepare(&plain, &plain_map).unwrap();
    let mut plan = fixture.plan(BEFORE_CUT);
    plan.transition.as_mut().unwrap().other.resources[2].width_px = u64::MAX;
    plan.transition.as_mut().unwrap().other.resources[2].height_px = 1.0e300;
    let map = approved_map(&plan, PROJECT_SCENE, &session).unwrap();
    let prepared = slots.prepare(&plan, &map).unwrap();
    assert_eq!(
        prepared.loads.len(),
        1,
        "only the neighbour's picture is new"
    );
    assert_eq!(slots.occupied(), 3);
}

fn cloud_fixture(name: &str, scenes: &[&'static str]) -> (Fixture, SceneResourceMap, ScenePlan) {
    let fixture = Fixture::new(name, &Content::sky().with_cloud(100, scenes));
    // The oracle of the splat count is the core's own header reader, not the code under test.
    assert_eq!(
        splat::inspect(&fixture.path("splats/cloud.spz"))
            .unwrap()
            .count,
        100
    );
    let session = fixture.approve();
    let plan = fixture.plan(BEFORE_CUT);
    let map = approved_map(&plan, PROJECT_SCENE, &session).unwrap();
    (fixture, map, plan)
}

fn splat_ceiling(map: &SceneResourceMap, plan: &ScenePlan, limit: u64) -> Result<(), Value> {
    let mut slots = SceneSlots::default().with_frame_splats(limit);
    match slots.prepare(plan, map) {
        Ok(_) => Ok(()),
        Err(error) => {
            assert_eq!(error.code, "resource_budget");
            assert_eq!(
                (slots.occupied(), slots.generation()),
                (0, 0),
                "the refusal left the book moved"
            );
            Err(error.details.unwrap())
        }
    }
}

#[test]
fn a_splat_both_decks_read_under_the_same_node_id_counts_two_uses_and_its_bytes_once() {
    let (fixture, map, before) = cloud_fixture("splats-shared", &["scene-a", "scene-b"]);
    let after = fixture.plan(AFTER_CUT);
    // `cloud-node` is the node id in BOTH scenes: two uses, 2 x 100 splats, not 100.
    for (name, plan) in [("before the cut", &before), ("after the cut", &after)] {
        assert_eq!(splat_ceiling(&map, plan, 200), Ok(()), "{name}");
        assert_eq!(
            splat_ceiling(&map, plan, 199),
            Err(json!({"frame_splats": 200, "max_frame_splats": 199})),
            "{name}"
        );
    }
    // The file is read and decoded once: its length plus 16 packed bytes per splat, once.
    let length = fs::metadata(fixture.path("splats/cloud.spz"))
        .unwrap()
        .len();
    let total = UNION_BYTES + length + 16 * 100;
    assert!(SceneSlots::new(4, total).prepare(&before, &map).is_ok());
    let mut below = SceneSlots::new(4, total - 1);
    let error = below.prepare(&before, &map).unwrap_err();
    assert_eq!(error.code, "resource_budget");
    assert_eq!(error.details.unwrap()["bytes"], json!(total));
    assert_eq!((below.occupied(), below.generation()), (0, 0));
    // Four slots, not five: the shared splat is one entry.
    let mut book = SceneSlots::new(4, total);
    assert_eq!(book.prepare(&before, &map).unwrap().loads.len(), 4);
    assert_eq!(
        DEFAULT_FRAME_SPLATS,
        8 * 1024 * 1024,
        "the ceiling was not raised"
    );
}

#[test]
fn a_splat_only_the_neighbour_reads_counts_before_and_after_the_cut() {
    let (fixture, map, before) = cloud_fixture("splats-other", &["scene-b"]);
    assert!(
        before.resource("cloud").is_none(),
        "the primary deck does not read it"
    );
    let after = fixture.plan(AFTER_CUT);
    assert!(after.resource("cloud").is_some());
    for (name, plan) in [("neighbour", &before), ("primary", &after)] {
        assert_eq!(splat_ceiling(&map, plan, 100), Ok(()), "{name}");
        assert_eq!(
            splat_ceiling(&map, plan, 99),
            Err(json!({"frame_splats": 100, "max_frame_splats": 99})),
            "{name}"
        );
    }
    let (_fixture, map, before) = cloud_fixture("splats-primary", &["scene-a"]);
    assert!(before.resource("cloud").is_some());
    assert_eq!(splat_ceiling(&map, &before, 100), Ok(()));
    assert!(splat_ceiling(&map, &before, 99).is_err());
}

#[test]
fn a_file_that_is_not_an_spz_is_refused_for_the_neighbour_too_with_its_format_detail() {
    let fixture = Fixture::new(
        "splats-format",
        &Content::sky().with_cloud(100, &["scene-b"]),
    );
    let plan = fixture.plan(BEFORE_CUT);
    // `from_root` trusts the root, so a picture standing where the splat should be reaches the book.
    fs::copy(
        fixture.path("assets/figure.png"),
        fixture.path("splats/cloud.spz"),
    )
    .unwrap();
    let map = SceneResourceMap::from_root(1, &plan, &fixture.root).unwrap();
    let mut slots = SceneSlots::default();
    let error = slots.prepare(&plan, &map).unwrap_err();
    assert_eq!(error.code, "resource_mismatch");
    let details = error.details.unwrap();
    assert_eq!(details["expected"], json!({"format": "spz"}));
    assert_eq!(
        details["resource"],
        context("cloud", &[("other", "cloud-node")])
    );
    assert_eq!((slots.occupied(), slots.generation()), (0, 0));
}

#[test]
fn a_plan_without_a_transition_loads_and_refuses_exactly_as_before() {
    let fixture = Fixture::new("slots-plain", &Content::sky());
    let session = fixture.approve();
    let plain = fixture.plan(PLAIN);
    assert!(plain.transition.is_none());
    let map = approved_map(&plain, PROJECT_SCENE, &session).unwrap();
    let mut slots = SceneSlots::default();
    let prepared = slots.prepare(&plain, &map).unwrap();
    assert_eq!(ids(&prepared.layer), ["backdrop", "figure"]);
    assert_eq!(prepared.loads.len(), 2);
    assert_eq!(slots.occupied(), 2);
    // The layer's resources are the old shape, one reference per resource of the plan.
    assert_eq!(
        prepared.layer["resources"][1],
        json!({"asset_id": "figure", "slot": "scene-res-1", "revision_id": "rev-1",
               "content_hash": hash_of(&fixture.path("assets/figure.png")), "generation": 1})
    );
    // An absent entry is `resource_unresolved` with its detail, now with the one deck there is.
    let absent = rebuilt(&map, 1, |asset, _| asset != "figure");
    let error = SceneSlots::default().prepare(&plain, &absent).unwrap_err();
    assert_eq!(error.code, "resource_unresolved");
    let details = error.details.unwrap();
    assert_eq!(details["asset_id"], "figure");
    assert_eq!(
        details["resource"],
        context("figure", &[("primary", "figure")])
    );
    // Frame 60 is plain `shot-b`: its three pictures, `sky` among them, in the same order.
    let shot_b = fixture.plan(60);
    let map = approved_map(&shot_b, PROJECT_SCENE, &session).unwrap();
    let prepared = SceneSlots::default().prepare(&shot_b, &map).unwrap();
    assert_eq!(ids(&prepared.layer), ["backdrop", "figure", "sky"]);
}

#[test]
fn the_public_door_still_refuses_a_transition_and_changes_nothing() {
    use tessel_editor_core::directed_scene::fx::FX_TRANSITION_NOT_YET;
    use tessel_editor_core::{EditorState, headless};
    let scratch = Scratch::new("public-closed");
    let bundle = bundle_and_files(&scratch.0, &Content::sky());
    let mut state = EditorState::demo();
    headless::execute(
        &mut state,
        &json!({"op": "project.create", "name": "Closed"}),
    )
    .unwrap();
    let error = headless::execute(
        &mut state,
        &json!({"op": "scene.attach", "id": "street", "bundle": bundle}),
    )
    .unwrap_err();
    assert_eq!(error.code, "invalid_bundle");
    assert_eq!(
        error.details.as_ref().unwrap()["issues"][0]["code"],
        FX_TRANSITION_NOT_YET
    );
    assert!(state.scenes.get("street").is_none());
    // The renderer's gated import is the same public door.
    let refused = import_gated(&bundle.to_string()).unwrap_err();
    assert_eq!(refused.code, "bundle_invalid");
}
