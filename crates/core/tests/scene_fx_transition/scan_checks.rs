//! The boundary scanner against hostile text. Each case here is one the old line-and-brace
//! scanner got wrong or could not tell apart (a test file named by suffix, a lone `{` in a string
//! swallowing the rest of a file, an ambiguous `cfg`), and each is paired with a control that
//! must stay quiet, so the scanner cannot pass by flagging everything.
//!
//! These are scanner tests, not gate tests: the gate itself is proved dynamically in `reload.rs`
//! and `public.rs`. The scanner stays textual and architectural, and these cases do not make it a
//! security barrier.

use std::collections::BTreeMap;

use crate::boundary::{inside_importer, outside_importer};
use crate::scan::{Scan, crate_roots, parse, production_text, scan};

fn files(list: &[(&str, &str)]) -> BTreeMap<String, String> {
    list.iter()
        .map(|(path, text)| (path.to_string(), text.to_string()))
        .collect()
}

fn run(list: &[(&str, &str)]) -> Scan {
    let files = files(list);
    let roots = crate_roots(&files);
    scan(&files, &roots).unwrap_or_else(|error| panic!("{error}"))
}

fn scanned(scan: &Scan, path: &str) -> bool {
    scan.sources.iter().any(|source| source.path == path)
}

/// What the gate rules say about a synthetic repository.
fn violations(list: &[(&str, &str)]) -> Vec<String> {
    let scan = run(list);
    let mut found = outside_importer(&scan.sources);
    found.extend(inside_importer(&scan.sources));
    found
}

const ACCEPT: &str = "fn open() { import_value_admitting(v, l, TransitionAdmission::Accept) }";

#[test]
fn a_file_name_never_excludes_active_source() {
    let tree = run(&[
        (
            "core/src/lib.rs",
            "mod tests; mod run_tests; mod nested; \
             #[cfg(test)] mod helpers; \
             #[cfg(test)] #[path = \"elsewhere/shared.rs\"] mod shared;",
        ),
        ("core/src/tests.rs", ACCEPT),
        ("core/src/run_tests.rs", ACCEPT),
        ("core/src/nested/mod.rs", "mod tests;"),
        ("core/src/nested/tests.rs", ACCEPT),
        ("core/src/helpers.rs", ACCEPT),
        ("core/src/elsewhere/shared.rs", ACCEPT),
    ]);
    // Declared without a `cfg`: scanned, whatever the name says.
    for path in [
        "core/src/tests.rs",
        "core/src/run_tests.rs",
        "core/src/nested/tests.rs",
    ] {
        assert!(scanned(&tree, path), "{path} is active source");
        let source = tree.sources.iter().find(|s| s.path == path).unwrap();
        assert_eq!(source.count("TransitionAdmission::Accept"), 1, "{path}");
    }
    // Declared by a test-only `mod`, whatever the name says (a `#[path]` file included).
    assert_eq!(
        tree.test_only,
        ["core/src/elsewhere/shared.rs", "core/src/helpers.rs"]
    );
    assert!(!scanned(&tree, "core/src/helpers.rs"));
    assert!(!scanned(&tree, "core/src/elsewhere/shared.rs"));
    assert!(tree.orphans.is_empty() && tree.unresolved.is_empty());
}

#[test]
fn a_real_cfg_decides_and_an_ambiguous_one_keeps_the_text_in() {
    let test_only = [
        "#[cfg(test)]",
        "#[cfg(all(test, unix))]",
        "#[cfg(all(unix, test))]",
        "#[cfg(not(not(test)))]",
        "#[cfg(all(test, feature = \"prod\"))]",
    ];
    let production = [
        "#[cfg(any(test, feature = \"prod\"))]",
        "#[cfg(any(test, debug_assertions))]",
        "#[cfg(not(test))]",
        "#[cfg(feature = \"x\")]",
        "#[cfg(all(not(test), unix))]",
        "#[cfg(any(unix, windows))]",
        "#[cfg(any())]",
        "#[cfg()]",
        "#[cfg(test, unix)]",
        "#[cfg_attr(test, allow(dead_code))]",
        "#[test]",
        "",
    ];
    let mut lib = String::new();
    let mut list: Vec<(String, String)> = Vec::new();
    for (index, attribute) in test_only.iter().chain(production.iter()).enumerate() {
        lib.push_str(&format!("{attribute} mod m{index};\n"));
        list.push((format!("core/src/m{index}.rs"), ACCEPT.to_string()));
    }
    list.push(("core/src/lib.rs".to_string(), lib));
    let borrowed: Vec<(&str, &str)> = list.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let tree = run(&borrowed);
    for (index, attribute) in test_only.iter().enumerate() {
        let path = format!("core/src/m{index}.rs");
        assert!(tree.test_only.contains(&path), "{attribute} is test-only");
        assert!(!scanned(&tree, &path), "{attribute}");
    }
    for (offset, attribute) in production.iter().enumerate() {
        let path = format!("core/src/m{}.rs", test_only.len() + offset);
        assert!(
            scanned(&tree, &path),
            "`{attribute}` is not provably test-only"
        );
    }
}

#[test]
fn modules_resolve_through_inline_nesting_and_path_attributes() {
    let tree = run(&[
        (
            "core/src/lib.rs",
            "mod a; mod inl { mod leaf; } \
             #[cfg(test)] mod t { mod deep; } \
             #[path = \"weird.rs\"] mod active;",
        ),
        ("core/src/a.rs", "mod b;"),
        ("core/src/a/b.rs", "fn b() {}"),
        ("core/src/inl/leaf.rs", ACCEPT),
        ("core/src/t/deep.rs", ACCEPT),
        ("core/src/weird.rs", ACCEPT),
    ]);
    for path in [
        "core/src/a.rs",
        "core/src/a/b.rs",
        "core/src/inl/leaf.rs",
        "core/src/weird.rs",
    ] {
        assert!(scanned(&tree, path), "{path}");
    }
    assert_eq!(tree.test_only, ["core/src/t/deep.rs"]);
    assert!(tree.orphans.is_empty() && tree.unresolved.is_empty());
    // A file nothing declares is never skipped, and a declaration with no file is reported.
    let loose = run(&[
        ("core/src/lib.rs", "mod missing;"),
        ("core/src/stray_tests.rs", ACCEPT),
    ]);
    assert_eq!(loose.orphans, ["core/src/stray_tests.rs"]);
    assert!(scanned(&loose, "core/src/stray_tests.rs"));
    assert_eq!(loose.unresolved, ["core/src/lib.rs: mod missing"]);
    // An inner `#![cfg(test)]` empties a file that a plain `mod` declares.
    let inner = run(&[
        ("core/src/lib.rs", "mod only_tests;"),
        (
            "core/src/only_tests.rs",
            "//! docs\n#![cfg(test)]\nfn t() { TransitionAdmission::Accept }",
        ),
    ]);
    assert!(scanned(&inner, "core/src/only_tests.rs"));
    let source = &inner
        .sources
        .iter()
        .find(|s| s.path == "core/src/only_tests.rs")
        .unwrap();
    assert_eq!(source.count("Accept"), 0);
}

#[test]
fn delimiters_in_strings_chars_and_comments_never_move_an_item_boundary() {
    let sample = r####"
#[cfg(test)]
fn brace_in_string() { let _ = "{"; let _ = '{'; let _ = r#"}"#; let _ = b"{"; HIDDEN_A; }
fn after_string() { KEEP_A }

#[cfg(test)]
mod tests {
    fn lone_brace_char() { let _ = '}'; /* } */ // }
        /* /* nested } */ still comment } */ }
        fn lifetimes<'a>(x: &'a str) -> &'a str { x }
    fn escaped() { let _ = "\"}"; let _ = "\\"; HIDDEN_B; }
    const RAW: &str = r##"} "# }"##;
}
fn after_mod<'a>(x: &'a str) -> &'a str { KEEP_B; x }

#[cfg(test)]
impl Probe { fn t() { let _ = "}}}"; HIDDEN_C; } }
fn after_impl() { KEEP_C }

#[cfg(test)]
fn unbalanced_looking() { let _ = "{{{{"; let _ = '"'; HIDDEN_D; }
fn the_last_item() { KEEP_D }
"####;
    let code = production_text(sample);
    for kept in ["KEEP_A", "KEEP_B", "KEEP_C", "KEEP_D"] {
        assert!(code.contains(kept), "{kept} was swallowed: {code}");
    }
    for hidden in [
        "HIDDEN_A",
        "HIDDEN_B",
        "HIDDEN_C",
        "HIDDEN_D",
        "lone_brace_char",
    ] {
        assert!(!code.contains(hidden), "{hidden} leaked: {code}");
    }
    // The reviewer's demonstration: a lone brace in a test item's string, then the code that
    // must still be seen (and flagged).
    let swallowed = format!("#[cfg(test)]\nfn probe() {{ let _ = \"{{\"; }}\n\n{ACCEPT}\n");
    assert!(production_text(&swallowed).contains("TransitionAdmission::Accept"));
}

#[test]
fn a_cfg_test_item_is_removed_in_every_position_and_its_neighbours_stay() {
    let sample = r#"
use a::b; #[cfg(test)] use hidden::HIDDEN_1;
const KEEP_C: u8 = 1; #[cfg(test)] const HIDDEN_2: u8 = 2;
struct S { KEEP_F1: u8, #[cfg(test)] HIDDEN_3: u8, KEEP_F2: u8 }
enum E { KEEP_V1, #[cfg(test)] HIDDEN_4(u8), KEEP_V2 { x: u8 }, #[cfg(test)] HIDDEN_5 { y: u8 }, KEEP_V3 }
impl S { fn KEEP_M1() {} #[cfg(test)] fn HIDDEN_6() { let _ = "}"; } fn KEEP_M2() {} }
fn f(KEEP_P1: u8, #[cfg(test)] HIDDEN_7: u8, KEEP_P2: u8) {
    let KEEP_S1 = 1;
    #[cfg(test)] let HIDDEN_8 = vec![1; 3];
    #[cfg(test)] { let HIDDEN_9 = 1; }
    #[cfg(test)] HIDDEN_10();
    KEEP_S2();
    match x { A => 1, #[cfg(test)] B => HIDDEN_11(), C => { KEEP_ARM } #[cfg(test)] D => { HIDDEN_12 } E => KEEP_ARM2, }
    #[cfg(test)] if HIDDEN_13 { }
    KEEP_S3();
}
#[cfg(test)] #[derive(Debug)] pub(crate) struct HIDDEN_14<const N: usize>([u8; N]);
#[cfg(test)] pub const unsafe fn HIDDEN_15() -> [u8; 4] { [0; 4] }
#[cfg(test)] unsafe impl Send for HIDDEN_16 {}
#[cfg(test)] extern crate HIDDEN_17;
#[cfg(test)] macro_rules! HIDDEN_18 { () => { } }
fn KEEP_Z() {}
"#;
    let code = production_text(sample);
    for kept in [
        "KEEP_C",
        "KEEP_F1",
        "KEEP_F2",
        "KEEP_V1",
        "KEEP_V2",
        "KEEP_V3",
        "KEEP_M1",
        "KEEP_M2",
        "KEEP_P1",
        "KEEP_P2",
        "KEEP_S1",
        "KEEP_S2",
        "KEEP_S3",
        "KEEP_ARM",
        "KEEP_ARM2",
        "KEEP_Z",
    ] {
        assert!(
            code.contains(kept),
            "{kept} was removed with a neighbour: {code}"
        );
    }
    for number in 1..=18 {
        let hidden = format!("HIDDEN_{number}");
        assert!(!code.contains(&hidden), "{hidden} stayed: {code}");
    }
}

#[test]
fn comments_and_docs_are_not_code_but_a_string_is() {
    let code = production_text(
        "/// TransitionAdmission::Accept in a doc\n\
         //! inner doc Accept\n\
         /** block doc Accept */\n\
         // Accept\n\
         /* Accept */\n\
         fn f() { let _ = \"TransitionAdmission::Accept\"; let _ = r#\"Accept\"#; }\n",
    );
    assert_eq!(code.matches("Accept").count(), 2, "{code}");
}

#[test]
fn text_the_lexer_rejects_fails_the_scan_closed() {
    for broken in [
        "fn f() { \"never closed }",
        "/* never closed\nfn f() {}",
        "fn f() { r#\"raw never closed }",
    ] {
        assert!(parse(broken).is_err(), "{broken}");
        let files = files(&[("core/src/lib.rs", "mod x;"), ("core/src/x.rs", broken)]);
        let error = scan(&files, &["core/src/lib.rs".to_string()])
            .err()
            .expect("the scan fails, it does not skip the file");
        assert!(error.starts_with("core/src/x.rs: "), "{error}");
    }
}

#[test]
fn the_gate_rules_see_what_the_old_scanner_could_not() {
    let tests_gate = |attribute: &str| {
        violations(&[
            ("core/src/lib.rs", "mod scene_project;"),
            (
                "core/src/scene_project/mod.rs",
                &format!("{attribute} mod run_tests;"),
            ),
            ("core/src/scene_project/run_tests.rs", ACCEPT),
        ])
    };
    // Blind spot 1: a file named `*_tests.rs` was skipped without asking for `cfg(test)`.
    let found = tests_gate("");
    assert_eq!(
        found.len(),
        4,
        "the four needles, in run_tests.rs: {found:?}"
    );
    assert!(found.iter().all(|line| line.contains("run_tests.rs")));
    assert!(tests_gate("#[cfg(test)]").is_empty(), "control");
    assert!(!tests_gate("#[cfg(any(test, feature = \"prod\"))]").is_empty());

    // Blind spot 2: a lone `{` in the string of a test item swallowed the code after it.
    let store = |lead: &str, attribute: &str| {
        violations(&[
            ("core/src/lib.rs", "mod scene_project;"),
            ("core/src/scene_project/mod.rs", "mod store;"),
            (
                "core/src/scene_project/store.rs",
                &format!("{lead}\n{attribute}\n{ACCEPT}\n"),
            ),
        ])
    };
    let lead = "#[cfg(test)]\nfn probe() { let _ = \"{\"; }";
    assert!(
        !store(lead, "").is_empty(),
        "the Accept after the string is seen"
    );
    assert!(
        store(lead, "#[cfg(test)]").is_empty(),
        "control: the same item under cfg(test)"
    );

    // Ambiguous predicates are production; a conjunction with `test` is not.
    let gated = |predicate: &str| {
        violations(&[
            ("core/src/lib.rs", "mod run;"),
            (
                "core/src/run.rs",
                &format!("#[cfg({predicate})] mod gate {{ {ACCEPT} }}"),
            ),
        ])
    };
    assert!(!gated("any(test, feature = \"prod\")").is_empty());
    assert!(!gated("not(test)").is_empty());
    assert!(gated("all(test, feature = \"prod\")").is_empty());
    assert!(gated("test").is_empty());

    // Inside the importer, a production `Accept` is flagged too, a test one is not.
    let inside = |attribute: &str| {
        violations(&[
            ("core/src/lib.rs", "mod directed_scene;"),
            // The importer defines the hidden entries, so `mod.rs` names them (the rule wants that).
            (
                "core/src/directed_scene/mod.rs",
                "mod other; fn import_str_admitting() {}",
            ),
            (
                "core/src/directed_scene/other.rs",
                &format!("{attribute} fn gate() {{ TransitionAdmission::Accept; }}"),
            ),
        ])
    };
    assert!(!inside("").is_empty());
    assert!(inside("#[cfg(test)]").is_empty());
}
