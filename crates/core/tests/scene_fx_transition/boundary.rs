//! L (boundary): no production code opens the transition gate. The hidden `Accept` policy is for
//! the core's tests and corpus; it is not a security barrier, so what keeps it out of the product
//! is this scan over the production sources of every crate, plus the inventory of the places that
//! create or rehydrate a `DirectedScene`.
//!
//! The scan reads text, not behavior, and says so: it proves nothing about code that does not live
//! under `crates/*/src`. It is not self-referential: this file is under `tests/` and is never
//! scanned, and the scanner is checked against synthetic text before it is trusted.

use std::fs;
use std::path::{Path, PathBuf};

/// A source file with comments, and `#[cfg(test)]` items, removed.
struct Source {
    /// Path from `crates/`, forward slashes.
    path: String,
    code: String,
}

/// Drops comment lines and every item introduced by `#[cfg(test)]` (a `mod x;` line or a braced
/// block). Braces in string literals are balanced in this code base; the self-check below pins
/// the behavior on a sample.
fn production_text(text: &str) -> String {
    let mut out = String::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        if trimmed == "#[cfg(test)]" {
            let mut depth: i32 = 0;
            let mut opened = false;
            for item in lines.by_ref() {
                let item = item.trim();
                if item.starts_with("//") || item.starts_with("#[") {
                    continue;
                }
                depth += item.matches('{').count() as i32;
                opened |= item.contains('{');
                depth -= item.matches('}').count() as i32;
                if (!opened && item.ends_with(';')) || (opened && depth <= 0) {
                    break;
                }
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn is_test_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    name == "tests.rs"
        || name.ends_with("_tests.rs")
        || path.components().any(|part| part.as_os_str() == "tests")
}

fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") && !is_test_file(&path) {
            found.push(path);
        }
    }
}

fn production_sources() -> Vec<Source> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let mut files = Vec::new();
    for member in fs::read_dir(&crates).unwrap() {
        let src = member.unwrap().path().join("src");
        if src.is_dir() {
            walk(&src, &mut files);
        }
    }
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(&crates)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let code = production_text(&fs::read_to_string(&path).unwrap());
            Source {
                path: relative,
                code,
            }
        })
        .collect()
}

fn count(source: &Source, needle: &str) -> usize {
    source.code.matches(needle).count()
}

const DIRECTED_SCENE: &str = "core/src/directed_scene/";

#[test]
fn the_scanner_sees_production_code_and_skips_comments_and_test_items() {
    let sample = "\
// TransitionAdmission::Accept in a comment
/// import_value_admitting in a doc
fn production() { run(TransitionAdmission::Accept); }

#[cfg(test)]
mod inline {
    fn only_in_tests() { import_str_admitting(); }
    fn nested() { if true { let _ = \"x\"; } }
}

#[cfg(test)]
mod elsewhere;

fn after() { call(); }
";
    let code = production_text(sample);
    assert!(code.contains("TransitionAdmission::Accept"), "{code}");
    assert!(!code.contains("import_value_admitting"), "{code}");
    assert!(!code.contains("import_str_admitting"), "{code}");
    assert!(
        !code.contains("only_in_tests") && !code.contains("elsewhere"),
        "{code}"
    );
    assert!(
        code.contains("fn after()"),
        "code after a test item is still scanned: {code}"
    );

    let sources = production_sources();
    assert!(sources.len() > 100, "scanned {} files", sources.len());
    for expected in [
        "core/src/directed_scene/mod.rs",
        "core/src/scene_project/run.rs",
        "core/src/scene_edit/fx.rs",
        "render/src/scene.rs",
    ] {
        assert!(
            sources.iter().any(|source| source.path == expected),
            "{expected} not scanned"
        );
    }
    assert!(
        sources
            .iter()
            .any(|source| source.path.starts_with("desktop/src/")),
        "the desktop crate must be scanned"
    );
    // The scanner does find the gate where it is defined, so a rename cannot turn it blind.
    let admission = sources
        .iter()
        .find(|source| source.path == "core/src/directed_scene/fx/admission.rs")
        .unwrap();
    assert_eq!(
        count(admission, "Accept"),
        2,
        "the definition and its predicate"
    );
}

#[test]
fn nothing_outside_the_importer_names_the_policy_or_the_admitting_entries() {
    for source in production_sources() {
        if source.path.starts_with(DIRECTED_SCENE) {
            continue;
        }
        for needle in [
            "TransitionAdmission",
            "_admitting",
            "import_value_admitting",
            "::Accept",
        ] {
            assert_eq!(
                count(&source, needle),
                0,
                "{} mentions {needle}",
                source.path
            );
        }
    }
}

#[test]
fn inside_the_importer_only_one_file_defines_the_entries_and_none_passes_accept() {
    for source in production_sources() {
        if !source.path.starts_with(DIRECTED_SCENE) {
            continue;
        }
        // No production line names the variant: the policy is built as `Refuse` or copied from a
        // scene that already has it.
        assert_eq!(
            count(&source, "TransitionAdmission::Accept"),
            0,
            "{}",
            source.path
        );
        let admitting = count(&source, "_admitting");
        if source.path == "core/src/directed_scene/mod.rs" {
            // Two public importers and the two hidden definitions, plus the `reimport` copy.
            assert!(admitting > 0 && admitting <= 8, "mod.rs: {admitting}");
        } else {
            assert_eq!(admitting, 0, "{} calls an admitting entry", source.path);
        }
        // No environment or file route reaches the policy.
        for needle in ["std::env", "env::var", "TESSEL_ADMIT", "TESSEL_TRANSITION"] {
            assert_eq!(count(&source, needle), 0, "{} reads {needle}", source.path);
        }
    }
    let sources = production_sources();
    let mod_rs = sources
        .iter()
        .find(|source| source.path == "core/src/directed_scene/mod.rs")
        .unwrap();
    // Every public importer hands `Refuse` to the shared implementation.
    assert!(
        count(mod_rs, "fx::TransitionAdmission::Refuse") >= 2,
        "public importers are Refuse"
    );
    assert!(
        count(mod_rs, "self.admission") >= 1,
        "reimport copies the scene's own policy"
    );
}

#[test]
fn every_creation_and_rehydration_of_a_scene_is_in_the_reviewed_inventory() {
    // The places that build a `DirectedScene` from text or a tree in production, and how many
    // times. A new one fails here until it is reviewed and added to the report
    // `docs/migration/p4b-s1a-transition-admission-report.md` (all of them are `Refuse`).
    let inventory = [
        ("core/src/scene_project/run.rs", 3),
        ("core/src/scene_project/store.rs", 1),
        ("render/src/scene.rs", 1),
    ];
    let mut found: Vec<(String, usize)> = Vec::new();
    for source in production_sources() {
        if source.path.starts_with(DIRECTED_SCENE) {
            continue;
        }
        let uses = count(&source, "DirectedScene::import_");
        if uses > 0 {
            found.push((source.path, uses));
        }
    }
    let expected: Vec<(String, usize)> = inventory
        .iter()
        .map(|(path, uses)| (path.to_string(), *uses))
        .collect();
    assert_eq!(found, expected);
}

#[test]
fn the_edit_candidates_inherit_the_scene_policy_instead_of_importing_afresh() {
    let sources = production_sources();
    for path in ["core/src/scene_edit/fx.rs", "core/src/scene_edit/batch.rs"] {
        let source = sources.iter().find(|source| source.path == path).unwrap();
        assert!(
            count(source, ".reimport(") >= 1,
            "{path} must re-read through `reimport`"
        );
        assert_eq!(
            count(source, "DirectedScene::import_"),
            0,
            "{path} would reset the policy"
        );
    }
    // Any other code that re-reads a scene it was given must do the same; the importer is the
    // only place allowed to name the method.
    for source in &sources {
        if source.path.starts_with(DIRECTED_SCENE)
            || source.path.starts_with("core/src/scene_edit/")
        {
            continue;
        }
        assert_eq!(
            count(source, ".reimport("),
            0,
            "{} calls reimport",
            source.path
        );
    }
}
