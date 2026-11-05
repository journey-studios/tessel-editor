//! Contract between the evaluated plan (H) and the resource manifest (I).
use std::path::PathBuf;
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_plan::{PlanOptions, ScenePlan};
use tessel_editor_core::scene_resources::ResourceManifest;

#[test]
fn every_required_asset_is_known_by_the_manifest_for_each_corpus_scene() {
    let fixtures =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/migration/fixtures/bundles");
    let mut checked_bundles = 0;
    let mut checked_frames = 0;
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let path = entry.unwrap().path();
        if !path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("valid-")
        {
            continue;
        }
        let scene = DirectedScene::import_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let manifest = ResourceManifest::from_scene(&scene);
        let last = i64::try_from(scene.bundle().edit_duration_frames).unwrap() - 1;
        for frame in [0, 1, last / 2, last] {
            // Some corpus scenes declare deferred domains. They stay explicitly partial;
            // this proves identity agreement, not full rendering or portable transport.
            let plan = ScenePlan::build_with(
                &scene,
                frame,
                PlanOptions {
                    allow_partial: true,
                },
            )
            .unwrap();
            let required = plan.required_asset_ids();
            let lookup = manifest.lookup(&required);
            assert!(
                lookup.unknown.is_empty(),
                "{} frame {frame}: {:?}",
                path.display(),
                lookup.unknown
            );
            assert_eq!(lookup.found.len(), required.len());
            checked_frames += 1;
        }
        assert_eq!(
            manifest.lookup(&["not-a-declared-resource"]).unknown,
            ["not-a-declared-resource"]
        );
        checked_bundles += 1;
    }
    assert_eq!(checked_bundles, 4);
    assert_eq!(checked_frames, 16);
}
