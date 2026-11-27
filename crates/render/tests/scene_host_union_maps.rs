//! S2-H, Marco A (BL082.4.4, matrix T10/T18): the resource MAPS of the host over a plan with two
//! decks. `SceneResourceMap::from_root`, `scene_operation::approved_map` and
//! `scene_export::resource_map` (alone and behind the export renderer's `FrameJob`) walk the
//! union of both decks, so an asset only the neighbour deck draws is a file of the frame.
//!
//! What is real here: tiny PNGs with their real SHA-256, a bundle restated with them, and a media
//! `SceneResourceSession` that approved them (`media::scene_resources::load`: hash, header,
//! containment). What is NOT: the transition is not drawn (no runtime, no pixels) and no public
//! entry accepts a transition: these are host helpers handed a typed plan, not a monitor or a
//! positive CLI. `resource_map` over a `Spy` only proves what it asked, not that a file exists.

mod scene_host_union_common;

use std::fs;
use std::path::PathBuf;

use scene_host_union_common::*;
use tessel_editor_core::jobs::JobError;
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_core::scene_resources::ResourceKey;
use tessel_editor_media::scene_encoder::{FrameJob, SceneFrameRenderer};
use tessel_editor_media::scene_resources::ResourceResolver;
use tessel_editor_render::scene_export::{ProcessSceneRenderer, resource_map};
use tessel_editor_render::scene_operation::approved_map;
use tessel_editor_render::{RuntimeConfig, SceneResourceMap};

const FILES: [(&str, &str); 3] = [
    ("backdrop", "assets/backdrop.png"),
    ("figure", "assets/figure.png"),
    ("sky", "neighbour/sky.png"),
];

fn expect_map(map: &SceneResourceMap, fixture: &Fixture, generation: u64) {
    assert_eq!(map.generation(), generation);
    assert_eq!(map.len(), 3, "the union has three assets");
    for (asset, relative) in FILES {
        let entry = map
            .get(asset)
            .unwrap_or_else(|| panic!("{asset} is missing"));
        assert_eq!(entry.revision_id, "rev-1", "{asset}");
        // The independent oracle: the hash of the bytes on disk, not a number the host chose.
        assert_eq!(
            entry.content_hash,
            hash_of(&fixture.path(relative)),
            "{asset}"
        );
        assert_eq!(
            entry.path.canonicalize().unwrap(),
            fixture.path(relative).canonicalize().unwrap(),
            "{asset}"
        );
    }
}

/// A plan hand-edited into a metadata conflict: `backdrop` of the neighbour has another hash.
fn conflicting(fixture: &Fixture) -> ScenePlan {
    let mut plan = fixture.plan(BEFORE_CUT);
    let other = &mut plan.transition.as_mut().unwrap().other;
    assert_eq!(other.resources[0].asset_id, "backdrop");
    other.resources[0].content_hash = "another-hash".into();
    plan
}

fn job<'a>(
    plan: &'a ScenePlan,
    resources: &'a dyn ResourceResolver,
    png: &'a std::path::Path,
) -> FrameJob<'a> {
    FrameJob {
        scene_id: PROJECT_SCENE,
        plan,
        resources,
        generation: 3,
        size: (64, 36),
        png,
    }
}

/// Never started: every refusal below happens in `resource_map`, before the renderer touches a
/// process, so the paths need not exist.
fn export_renderer() -> ProcessSceneRenderer {
    ProcessSceneRenderer::new(RuntimeConfig {
        node: PathBuf::from("node-that-is-never-started"),
        runtime_dir: PathBuf::from("runtime-that-is-never-read"),
        gl: None,
    })
}

fn failed(result: Result<impl std::fmt::Debug, JobError>) -> JobError {
    result.expect_err("the frame should be refused")
}

#[test]
fn the_three_loaders_carry_the_asset_only_the_neighbour_draws_before_and_after_the_cut() {
    let fixture = Fixture::new("maps-union", &Content::sky());
    let session = fixture.approve();
    let mut seen: Vec<[SceneResourceMap; 3]> = Vec::new();
    // (frame, own resources of the primary deck, of the neighbour)
    for (frame, primary, neighbour) in [(BEFORE_CUT, 2, 3), (AFTER_CUT, 3, 2)] {
        let plan = fixture.plan(frame);
        // Each deck keeps its own serialized table; the union is never a field of the plan.
        assert_eq!(plan.resources.len(), primary, "frame {frame}");
        assert_eq!(
            plan.transition.as_ref().unwrap().other.resources.len(),
            neighbour,
            "frame {frame}"
        );
        assert!(plan.to_json().get("resourceUnion").is_none());
        assert_eq!(plan.resource("sky").is_some(), frame == AFTER_CUT);

        let from_root = SceneResourceMap::from_root(7, &plan, &fixture.root).unwrap();
        expect_map(&from_root, &fixture, 7);

        let approved = approved_map(&plan, PROJECT_SCENE, &session).unwrap();
        expect_map(&approved, &fixture, session.generation().0);

        let spy = Spy::new(&session);
        let exported = resource_map(&plan, PROJECT_SCENE, &spy, 3).unwrap();
        expect_map(&exported, &fixture, 3);
        // One question per asset of the union, ascending, always under the project's scene id.
        let asked: Vec<(String, String)> = ["backdrop", "figure", "sky"]
            .iter()
            .map(|asset| (PROJECT_SCENE.to_string(), format!("asset:{asset}")))
            .collect();
        assert_eq!(spy.calls(), asked, "frame {frame}");
        seen.push([from_root, approved, exported]);
    }
    // The cut swaps the roles and changes nothing about which files the frame needs.
    assert_eq!(seen[0][0], seen[1][0]);
    assert_eq!(seen[0][1], seen[1][1]);
    assert_eq!(seen[0][2].len(), seen[1][2].len());
}

#[test]
fn a_file_missing_for_the_neighbour_only_is_a_real_canonicalize_refusal_that_names_the_other_deck()
{
    let fixture = Fixture::new("maps-missing-file", &Content::sky());
    fs::remove_file(fixture.path("neighbour/sky.png")).unwrap();

    let error =
        SceneResourceMap::from_root(1, &fixture.plan(BEFORE_CUT), &fixture.root).unwrap_err();
    assert_eq!(error.code, "resource_unresolved");
    let details = error.details.unwrap();
    assert_eq!(details["asset_id"], "sky", "the existing detail stays");
    assert_eq!(
        details["resource"],
        context("sky", &[("other", "sky-plane")])
    );
    assert!(
        error
            .message
            .ends_with("[asset `sky` used by: other deck node `sky-plane`]"),
        "{}",
        error.message
    );

    // After the cut the very same asset belongs to the primary deck, and the context says so.
    let error =
        SceneResourceMap::from_root(1, &fixture.plan(AFTER_CUT), &fixture.root).unwrap_err();
    assert_eq!(error.code, "resource_unresolved");
    assert_eq!(
        error.details.unwrap()["resource"],
        context("sky", &[("primary", "sky-plane")])
    );
    assert!(error.message.contains("primary deck node `sky-plane`"));

    // Control: a frame that does not use `sky` never looked for it.
    assert!(SceneResourceMap::from_root(1, &fixture.plan(PLAIN), &fixture.root).is_ok());
}

#[test]
fn the_root_that_does_not_exist_is_a_global_refusal_without_an_invented_asset() {
    let fixture = Fixture::new("maps-root", &Content::sky());
    let missing = fixture.path("no-such-root");
    let error = SceneResourceMap::from_root(1, &fixture.plan(BEFORE_CUT), &missing).unwrap_err();
    assert_eq!(error.code, "resource_unresolved");
    assert!(error.details.is_none(), "{:?}", error.details);
    assert!(!error.message.contains("used by"), "{}", error.message);
}

/// A directory link: a symlink on Unix, a junction on Windows (`mklink /J` needs no privilege,
/// unlike a symlink). Both are followed by `canonicalize`, which is what the root check relies on.
fn link_directory(link: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()?;
        output.status.success().then_some(()).ok_or_else(|| {
            std::io::Error::other(format!(
                "mklink /J failed: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ))
        })
    }
}

#[test]
fn a_link_that_takes_the_neighbours_asset_out_of_the_root_is_refused_and_names_it() {
    let fixture = Fixture::new("maps-link", &Content::sky());
    // `neighbour/` holds only `sky`; replace it by a link to a folder outside the root that has a
    // file of the same name, so only the neighbour's asset leaves the root.
    let outside = fixture.root.parent().unwrap().join("outside-neighbour");
    fs::create_dir_all(&outside).unwrap();
    fs::copy(fixture.path("neighbour/sky.png"), outside.join("sky.png")).unwrap();
    fs::remove_dir_all(fixture.path("neighbour")).unwrap();
    link_directory(&fixture.path("neighbour"), &outside)
        .unwrap_or_else(|failure| panic!("cannot link a directory: {failure}"));
    let error =
        SceneResourceMap::from_root(1, &fixture.plan(BEFORE_CUT), &fixture.root).unwrap_err();
    assert_eq!(error.code, "resource_path_invalid");
    let details = error.details.unwrap();
    assert_eq!(details["asset_id"], "sky");
    assert_eq!(
        details["resource"],
        context("sky", &[("other", "sky-plane")])
    );
    // The two decks' shared files stayed inside: a frame without `sky` still resolves.
    assert!(SceneResourceMap::from_root(1, &fixture.plan(PLAIN), &fixture.root).is_ok());
    // Unlink before the scratch is removed (a junction is removed like an empty directory).
    #[cfg(windows)]
    fs::remove_dir(fixture.path("neighbour")).unwrap();
    #[cfg(unix)]
    fs::remove_file(fixture.path("neighbour")).unwrap();
    let _ = fs::remove_dir_all(&outside);
}
#[test]
fn an_asset_absent_from_the_approved_map_is_resource_unknown_with_the_deck_in_the_three_error_types()
 {
    // The approval covers a bundle WITHOUT `sky`; the plan (built from the bundle with it) asks for
    // it only through the neighbour. NOT a claim that a file is missing on disk.
    let with_sky = Fixture::new("maps-unknown", &Content::sky());
    let without = Fixture::new("maps-unknown-approved", &Content::default());
    let session = without.approve();
    let plan = with_sky.plan(BEFORE_CUT);
    let sky = context("sky", &[("other", "sky-plane")]);

    let command = approved_map(&plan, PROJECT_SCENE, &session).unwrap_err();
    assert_eq!(
        command.code, "resource_unknown",
        "not flattened to render_failed"
    );
    assert_eq!(command.details.as_deref().unwrap()["resource"], sky);
    assert!(
        command
            .message
            .ends_with("[asset `sky` used by: other deck node `sky-plane`]"),
        "{}",
        command.message
    );

    let backend = resource_map(&plan, PROJECT_SCENE, &session, 3).unwrap_err();
    assert_eq!(backend.code, "resource_unknown");
    assert_eq!(backend.details.unwrap()["resource"], sky);

    // The export's JobError: code, backend code and the context all arrive.
    let png = with_sky.path("frame.png");
    let mut renderer = export_renderer();
    let error = failed(renderer.render(&job(&plan, &session, &png)));
    assert_eq!(error.code, "render_failed");
    let details = error.details.as_ref().unwrap();
    assert_eq!(details["backend_code"], "resource_unknown");
    assert_eq!(details["resource"], sky);
    assert!(
        error.message.contains("other deck node `sky-plane`"),
        "{}",
        error.message
    );

    // After the cut the neighbour is `scene-a`: `sky` is the primary deck's now.
    let after = with_sky.plan(AFTER_CUT);
    let command = approved_map(&after, PROJECT_SCENE, &session).unwrap_err();
    assert_eq!(command.code, "resource_unknown");
    assert_eq!(
        command.details.as_deref().unwrap()["resource"],
        context("sky", &[("primary", "sky-plane")])
    );
}

#[test]
fn the_approval_key_is_the_project_scene_never_a_source_scene_or_instance_id() {
    let fixture = Fixture::new("maps-key", &Content::sky());
    let session = fixture.approve();
    let plan = fixture.plan(BEFORE_CUT);
    // The plan's own sources carry ids the host must never ask the resolver with.
    assert_eq!(plan.source.scene_definition_id, "scene-a");
    assert_eq!(
        plan.transition
            .as_ref()
            .unwrap()
            .other
            .source
            .scene_definition_id,
        "scene-b"
    );
    assert!(approved_map(&plan, PROJECT_SCENE, &session).is_ok());
    for source in ["scene-a", "scene-b", "inst-a", "inst-b", "shot-a", "shot-b"] {
        let error = approved_map(&plan, source, &session).unwrap_err();
        assert_eq!(error.code, "scene_resources_not_loaded", "{source}");
    }
    let spy = Spy::new(&session);
    resource_map(&plan, PROJECT_SCENE, &spy, 3).unwrap();
    assert!(spy.calls().iter().all(|(scene, _)| scene == PROJECT_SCENE));
}

#[test]
fn a_metadata_conflict_is_refused_by_all_loaders_before_any_file_or_resolver_is_consulted() {
    let fixture = Fixture::new("maps-conflict", &Content::sky());
    let plan = conflicting(&fixture);
    let backdrop = context(
        "backdrop",
        &[("primary", "backdrop"), ("other", "backdrop")],
    );
    let tail =
        "[asset `backdrop` used by: primary deck node `backdrop`, other deck node `backdrop`]";

    // The root does not exist: had `from_root` looked at it, the error would be about the root.
    let error = SceneResourceMap::from_root(1, &plan, &fixture.path("no-such-root")).unwrap_err();
    assert_eq!(error.code, "invalid_plan");
    assert!(
        error.message.contains("/transition/other/resources/0"),
        "{}",
        error.message
    );
    assert!(error.message.ends_with(tail), "{}", error.message);
    assert_eq!(error.details.unwrap()["resource"], backdrop);

    // The session holds nothing: had `approved_map` asked it, the code would be
    // `scene_resources_not_loaded`.
    let empty = empty_session();
    let command = approved_map(&plan, PROJECT_SCENE, &empty).unwrap_err();
    assert_eq!(command.code, "invalid_plan");
    assert!(
        command.message.contains("/transition/other/resources/0"),
        "{}",
        command.message
    );
    assert_eq!(command.details.as_deref().unwrap()["resource"], backdrop);

    let approved = fixture.approve();
    let spy = Spy::new(&approved);
    let backend = resource_map(&plan, PROJECT_SCENE, &spy, 3).unwrap_err();
    assert_eq!(backend.code, "invalid_plan");
    assert!(
        backend.message.contains("/transition/other/resources/0"),
        "{}",
        backend.message
    );
    assert_eq!(backend.details.unwrap()["resource"], backdrop);
    assert!(
        spy.calls().is_empty(),
        "the resolver was asked: {:?}",
        spy.calls()
    );

    // The export's JobError carries code, backend code and context.
    let png = fixture.path("frame.png");
    let mut renderer = export_renderer();
    let error = failed(renderer.render(&job(&plan, &spy, &png)));
    assert_eq!(error.code, "render_failed");
    let details = error.details.as_ref().unwrap();
    assert_eq!(details["backend_code"], "invalid_plan");
    assert_eq!(details["resource"], backdrop);
    assert!(error.message.ends_with(tail), "{}", error.message);
    assert!(spy.calls().is_empty());
}

#[test]
fn a_relative_approved_path_keeps_its_code_and_names_the_neighbour() {
    let fixture = Fixture::new("maps-relative", &Content::sky());
    let plan = fixture.plan(BEFORE_CUT);
    let sky = context("sky", &[("other", "sky-plane")]);
    // The media session would never approve a relative path; this one is hand-built (and says so)
    // to reach the `ResolvedResource` check every loader still has.
    let approved = fixture.approve();
    let mut map = (*approved.get(PROJECT_SCENE).unwrap()).clone();
    let entry = map
        .entries
        .iter_mut()
        .find(|entry| entry.key == ResourceKey::asset("sky"))
        .unwrap();
    entry.path = PathBuf::from("neighbour/sky.png");
    let tampered = empty_session();
    tampered.install(map).unwrap();

    let backend = resource_map(&plan, PROJECT_SCENE, &tampered, 3).unwrap_err();
    assert_eq!(backend.code, "resource_path_invalid");
    assert_eq!(backend.details.unwrap()["resource"], sky);

    // `approved_map` keeps routing that check through `headless::backend_error`, as before H:
    // `render_failed`, the backend code inside; the context is added next to it.
    let command = approved_map(&plan, PROJECT_SCENE, &tampered).unwrap_err();
    assert_eq!(command.code, "render_failed");
    let details = command.details.as_deref().unwrap();
    assert_eq!(details["backend_code"], "resource_path_invalid");
    assert_eq!(details["resource"], sky);
    assert!(
        command.message.contains("other deck node `sky-plane`"),
        "{}",
        command.message
    );

    let error =
        failed(export_renderer().render(&job(&plan, &tampered, &fixture.path("frame.png"))));
    assert_eq!(error.code, "render_failed");
    let details = error.details.as_ref().unwrap();
    assert_eq!(details["backend_code"], "resource_path_invalid");
    assert_eq!(details["resource"], sky);
}
#[test]
fn both_mappers_still_hand_back_a_map_after_the_file_is_removed_and_only_the_book_looks() {
    let fixture = Fixture::new("maps-removed", &Content::sky());
    let session = fixture.approve();
    let plan = fixture.plan(BEFORE_CUT);
    fs::remove_file(fixture.path("neighbour/sky.png")).unwrap();
    // The approval is a promise about the past: neither mapper opens a file (decision 4).
    let approved = approved_map(&plan, PROJECT_SCENE, &session).unwrap();
    assert_eq!(approved.len(), 3);
    let exported = resource_map(&plan, PROJECT_SCENE, &session, 3).unwrap();
    assert_eq!(exported.len(), 3);
    assert!(!approved.get("sky").unwrap().path.exists());
    // `SceneSlots::prepare` is where is_file looks: see `scene_host_union_slots.rs`.
}

#[test]
fn a_plan_without_a_transition_resolves_exactly_what_it_always_did() {
    let fixture = Fixture::new("maps-plain", &Content::sky());
    let session = fixture.approve();
    let plan = fixture.plan(PLAIN);
    assert!(plan.transition.is_none());
    assert_eq!(plan.to_json()["resources"].as_array().unwrap().len(), 2);
    for map in [
        SceneResourceMap::from_root(2, &plan, &fixture.root).unwrap(),
        approved_map(&plan, PROJECT_SCENE, &session).unwrap(),
        resource_map(&plan, PROJECT_SCENE, &session, 2).unwrap(),
    ] {
        assert_eq!(map.len(), 2);
        assert!(map.get("backdrop").is_some() && map.get("figure").is_some());
        assert!(map.get("sky").is_none(), "scene-a does not draw it");
    }
    // The context is additive and says `primary` for a plan with one deck: frame 60 is plain
    // `shot-b`, which draws `sky`, and the approval below is of a bundle that does not carry it.
    let without = Fixture::new("maps-plain-approved", &Content::default());
    let plain_sky = fixture.plan(60);
    assert!(plain_sky.transition.is_none() && plain_sky.resource("sky").is_some());
    let command = approved_map(&plain_sky, PROJECT_SCENE, &without.approve()).unwrap_err();
    assert_eq!(command.code, "resource_unknown");
    assert_eq!(
        command.details.as_deref().unwrap()["resource"],
        context("sky", &[("primary", "sky-plane")])
    );
}
