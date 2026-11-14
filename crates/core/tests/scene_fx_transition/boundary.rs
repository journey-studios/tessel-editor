//! L (boundary): no production code opens the transition gate. The hidden `Accept` policy is for
//! the core's tests and corpus; it is not a security barrier, so what keeps it out of the product
//! is this scan over the production text of every crate, plus the inventory of the places that
//! create or rehydrate a `DirectedScene`.
//!
//! The scan reads text, not behavior, and says so (see `scan.rs` for what it does and does not
//! do): it proves nothing about code outside `crates/*/src`, macros, or `include!`. The behavior
//! of the doors that matter is proved dynamically in `reload.rs` and `public.rs`; this file is the
//! architectural net around them. It is not self-referential: this file is under `tests/` and is
//! never scanned, and the scanner is checked against hostile text in `scan_checks.rs`.

use crate::scan::{Source, scan_repository};

pub const DIRECTED_SCENE: &str = "core/src/directed_scene/";

/// The places that build a `DirectedScene` from text or a tree in production, and how many
/// times. A new one fails until it is reviewed and added to the report
/// `docs/migration/p4b-s1a-transition-admission-report.md` (all of them are `Refuse`).
const INVENTORY: [(&str, usize); 3] = [
    ("core/src/scene_project/run.rs", 3),
    ("core/src/scene_project/store.rs", 1),
    ("render/src/scene.rs", 1),
];

fn production_sources() -> Vec<Source> {
    scan_repository().sources
}

/// Nothing outside the importer names the policy or the admitting entries.
pub fn outside_importer(sources: &[Source]) -> Vec<String> {
    let mut found = Vec::new();
    for source in sources
        .iter()
        .filter(|source| !source.path.starts_with(DIRECTED_SCENE))
    {
        for needle in [
            "TransitionAdmission",
            "_admitting",
            "import_value_admitting",
            "::Accept",
        ] {
            if source.count(needle) > 0 {
                found.push(format!("{} mentions {needle}", source.path));
            }
        }
    }
    found
}

/// Inside the importer no line names `Accept`, no environment reaches the policy, and only
/// `mod.rs` names the admitting entries.
pub fn inside_importer(sources: &[Source]) -> Vec<String> {
    let mut found = Vec::new();
    for source in sources
        .iter()
        .filter(|source| source.path.starts_with(DIRECTED_SCENE))
    {
        if source.count("TransitionAdmission::Accept") > 0 {
            found.push(format!("{} names the Accept variant", source.path));
        }
        let admitting = source.count("_admitting");
        if source.path == "core/src/directed_scene/mod.rs" {
            // Two public importers and the two hidden definitions, plus the `reimport` copy.
            if admitting == 0 || admitting > 8 {
                found.push(format!("mod.rs names an admitting entry {admitting} times"));
            }
        } else if admitting != 0 {
            found.push(format!("{} calls an admitting entry", source.path));
        }
        for needle in ["std::env", "env::var", "TESSEL_ADMIT", "TESSEL_TRANSITION"] {
            if source.count(needle) > 0 {
                found.push(format!("{} reads {needle}", source.path));
            }
        }
    }
    found
}

/// The set of production files that import a scene is the reviewed inventory.
pub fn inventory(sources: &[Source]) -> Vec<String> {
    let found: Vec<(String, usize)> = sources
        .iter()
        .filter(|source| !source.path.starts_with(DIRECTED_SCENE))
        .map(|source| (source.path.clone(), source.count("DirectedScene::import_")))
        .filter(|(_, uses)| *uses > 0)
        .collect();
    let expected: Vec<(String, usize)> = INVENTORY
        .iter()
        .map(|(path, uses)| (path.to_string(), *uses))
        .collect();
    if found == expected {
        Vec::new()
    } else {
        vec![format!("inventory {found:?}, reviewed {expected:?}")]
    }
}

/// Only the importer and the edit layer name `reimport`, and the edit layer never imports afresh.
pub fn reimport(sources: &[Source]) -> Vec<String> {
    let mut found = Vec::new();
    for source in sources {
        let in_edit = source.path.starts_with("core/src/scene_edit/");
        if source.path.starts_with(DIRECTED_SCENE) {
            continue;
        }
        if in_edit && source.count("DirectedScene::import_") > 0 {
            found.push(format!("{} would reset the policy", source.path));
        }
        if !in_edit && source.count(".reimport(") > 0 {
            found.push(format!("{} calls reimport", source.path));
        }
    }
    found
}

fn assert_clean(found: Vec<String>) {
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn the_scan_sees_the_whole_repository_and_every_file_is_accounted_for() {
    let scan = scan_repository();
    assert!(
        scan.sources.len() > 100,
        "scanned {} files",
        scan.sources.len()
    );
    assert!(
        scan.unresolved.is_empty(),
        "production `mod x;` that name no file: {:#?}",
        scan.unresolved
    );
    // A file nothing declares is scanned as production; today there is none, so a new one is
    // reviewed here instead of drifting in unseen.
    assert!(
        scan.orphans.is_empty(),
        "undeclared files: {:#?}",
        scan.orphans
    );
    for expected in [
        "core/src/directed_scene/mod.rs",
        "core/src/scene_project/run.rs",
        "core/src/scene_project/store.rs",
        "core/src/scene_edit/fx.rs",
        "render/src/scene.rs",
        "desktop/src/workstation/mod.rs",
    ] {
        assert!(
            scan.sources.iter().any(|source| source.path == expected),
            "{expected} not scanned"
        );
    }
    // Test-only files are excluded because a `#[cfg(test)] mod` declares them, never by name.
    for test_only in [
        "desktop/src/workstation/scene_fx_transition_tests.rs",
        "core/src/scene_project/admission_tests.rs",
        "media/src/scene_resources/claims/tests.rs",
    ] {
        assert!(
            scan.test_only.iter().any(|path| path == test_only),
            "{test_only} should be a declared test-only module"
        );
        assert!(
            scan.sources.iter().all(|source| source.path != test_only),
            "{test_only} must not be scanned as production"
        );
    }
    // The scanner does find the gate where it is defined, so a rename cannot turn it blind.
    let admission = scan
        .sources
        .iter()
        .find(|source| source.path == "core/src/directed_scene/fx/admission.rs")
        .unwrap();
    assert_eq!(
        admission.count("Accept"),
        2,
        "the definition and its predicate"
    );
}

#[test]
fn nothing_outside_the_importer_names_the_policy_or_the_admitting_entries() {
    assert_clean(outside_importer(&production_sources()));
}

#[test]
fn inside_the_importer_only_one_file_defines_the_entries_and_none_passes_accept() {
    let sources = production_sources();
    assert_clean(inside_importer(&sources));
    let mod_rs = sources
        .iter()
        .find(|source| source.path == "core/src/directed_scene/mod.rs")
        .unwrap();
    // Every public importer hands `Refuse` to the shared implementation.
    assert!(
        mod_rs.count("fx::TransitionAdmission::Refuse") >= 2,
        "public importers are Refuse"
    );
    assert!(
        mod_rs.count("self.admission") >= 1,
        "reimport copies the scene's own policy"
    );
}

#[test]
fn every_creation_and_rehydration_of_a_scene_is_in_the_reviewed_inventory() {
    assert_clean(inventory(&production_sources()));
}

#[test]
fn the_edit_candidates_inherit_the_scene_policy_instead_of_importing_afresh() {
    let sources = production_sources();
    for path in ["core/src/scene_edit/fx.rs", "core/src/scene_edit/batch.rs"] {
        let source = sources.iter().find(|source| source.path == path).unwrap();
        assert!(
            source.count(".reimport(") >= 1,
            "{path} must re-read through `reimport`"
        );
    }
    assert_clean(reimport(&sources));
}
