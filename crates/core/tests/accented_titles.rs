//! BL141: Portuguese titles through the public library and the JSONL entry, without a window.
//! The picture of an accented title is proven by the runtime and render tests; this file proves
//! what the core decides: which text is accepted, its single (NFC) spelling, and that a rejected
//! edit changes nothing.
use std::io::Write;
use std::process::{Command as Process, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::{EditorState, headless};

const TITLE: &str = "title-north";
const COMPOSED: &str = "Coração: não é só açaí, É ÀS ÔNIBUS Ü";
fn decomposed() -> String {
    // Every accented letter spelled as a base letter plus a combining mark.
    let marks = [
        ('ç', "c\u{0327}"),
        ('ã', "a\u{0303}"),
        ('é', "e\u{0301}"),
        ('ó', "o\u{0301}"),
        ('í', "i\u{0301}"),
        ('É', "E\u{0301}"),
        ('À', "A\u{0300}"),
        ('Ô', "O\u{0302}"),
        ('Ü', "U\u{0308}"),
    ];
    COMPOSED
        .chars()
        .map(|ch| {
            marks
                .iter()
                .find(|(composed, _)| *composed == ch)
                .map_or(ch.to_string(), |(_, spelled)| spelled.to_string())
        })
        .collect()
}

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn fail(state: &mut EditorState, request: Value) -> String {
    headless::execute(state, &request)
        .expect_err(&format!("{request} must fail"))
        .code
        .to_string()
}

fn set_text(content: &str) -> Value {
    json!({"op":"clip.text","clip":TITLE,"text":{"content":content,"size":14,"color":"#ffffff"}})
}

/// The text layer the plan hands the runtime at the title's first frame.
fn planned_text(state: &mut EditorState) -> Value {
    let start = state.timeline.clip(TITLE).unwrap().start;
    let plan = run(state, json!({"op":"plan","frame":start}));
    plan["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|layer| layer["kind"] == "text")
        .unwrap_or_else(|| panic!("no text layer in {plan}"))["text"]
        .clone()
}

#[test]
fn an_accented_title_is_planned_with_one_canonical_spelling() {
    let mut state = EditorState::demo();
    run(&mut state, set_text(COMPOSED));
    let composed = planned_text(&mut state);
    assert_eq!(composed["content"], COMPOSED);

    let mut other = EditorState::demo();
    run(&mut other, set_text(&decomposed()));
    assert_ne!(decomposed(), COMPOSED, "the two spellings really differ");
    assert_eq!(planned_text(&mut other), composed, "NFD in, NFC out");
    assert_eq!(
        other.to_document(None)["sequences"],
        state.to_document(None)["sequences"],
        "the saved sequences are the same for both spellings"
    );
}

#[test]
fn a_rejected_text_leaves_the_project_and_its_history_untouched() {
    let mut state = EditorState::demo();
    run(&mut state, set_text("AÇÃO"));
    let document = state.to_document(None);
    let history = run(&mut state, json!({"op":"history.status"}));
    for (bad, code) in [
        ("preço €5", "unsupported_text"),
        ("日本", "unsupported_text"),
        ("duas\nlinhas", "unsupported_text"),
        ("a\u{0307}", "unsupported_text"),
        ("\u{0301}", "unsupported_text"),
        ("", "invalid_value"),
    ] {
        assert_eq!(fail(&mut state, set_text(bad)), code, "{bad:?}");
        assert_eq!(
            state.to_document(None),
            document,
            "{bad:?} changed the document"
        );
    }
    assert_eq!(run(&mut state, json!({"op":"history.status"})), history);
    assert_eq!(planned_text(&mut state)["content"], "AÇÃO");
}

#[test]
fn the_text_edit_is_one_undoable_step_and_a_repeat_is_not_one() {
    let mut state = EditorState::demo();
    let before = planned_text(&mut state);
    run(&mut state, set_text("Olá, mundo — ação!"));
    assert_eq!(planned_text(&mut state)["content"], "Olá, mundo — ação!");
    let revision = state.project.revision;
    // The same text again changes nothing and records nothing.
    run(
        &mut state,
        set_text(&"Olá, mundo — ação!".replace('á', "a\u{0301}")),
    );
    assert_eq!(state.project.revision, revision);
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(planned_text(&mut state), before);
    run(&mut state, json!({"op":"history.redo"}));
    assert_eq!(planned_text(&mut state)["content"], "Olá, mundo — ação!");
}

#[test]
fn set_clip_text_fields_run_the_same_rules_as_the_op() {
    let mut state = EditorState::demo();
    let target = format!("clip_text:{TITLE}");
    let set =
        |field: &str, value: Value| json!({"op":"set","target":target,"field":field,"value":value});
    run(
        &mut state,
        set("content", json!("Ac\u{0327}a\u{0303}o ra\u{0301}pida")),
    );
    run(&mut state, set("size", json!(20)));
    run(&mut state, set("color", json!("#ff8800")));
    let text = planned_text(&mut state);
    assert_eq!(text["content"], "Ação rápida");
    assert_eq!(text["size"], 20.0);
    assert_eq!(text["color"], "#ff8800");

    let document = state.to_document(None);
    assert_eq!(
        fail(&mut state, set("content", json!("preço €"))),
        "unsupported_text"
    );
    assert_eq!(fail(&mut state, set("size", json!(500))), "invalid_value");
    assert_eq!(
        fail(&mut state, set("color", json!("laranja"))),
        "invalid_value"
    );
    assert_eq!(
        fail(&mut state, set("font", json!("serif"))),
        "unknown_field"
    );
    assert_eq!(state.to_document(None), document);
}

#[test]
fn only_a_title_has_text_and_a_title_without_a_selection_has_no_text_node() {
    let mut state = EditorState::demo();
    let image = state
        .timeline
        .clips
        .iter()
        .find(|clip| clip.text.is_none())
        .unwrap()
        .id
        .to_string();
    let request = json!({"op":"clip.text","clip":image,"text":{"content":"AÇÃO"}});
    assert_eq!(fail(&mut state, request), "not_applicable");
    let target = format!("clip_text:{image}");
    assert_eq!(
        fail(
            &mut state,
            json!({"op":"set","target":target,"field":"content","value":"x"})
        ),
        "not_applicable"
    );
}

#[test]
fn a_saved_project_with_a_decomposed_title_reopens_composed_and_a_bad_one_is_refused() {
    let mut state = EditorState::demo();
    run(&mut state, set_text("Coração"));
    let mut document = state.to_document(None);
    // Hand-edit the file the way another tool could write it: NFD.
    let text = serde_json::to_string(&document).unwrap();
    assert!(text.contains("Coração"));
    let rewritten = text.replace("Coração", "Corac\u{0327}a\u{0303}o");
    document = serde_json::from_str(&rewritten).unwrap();
    let reopened = EditorState::from_document(&document, None).unwrap();
    assert_eq!(
        reopened
            .timeline
            .clip(TITLE)
            .unwrap()
            .text
            .as_ref()
            .unwrap()
            .content,
        "Coração"
    );
    assert_eq!(
        reopened.to_document(None)["sequences"],
        state.to_document(None)["sequences"]
    );

    let broken = text.replace("Coração", "Cora€ão");
    let broken: Value = serde_json::from_str(&broken).unwrap();
    assert!(EditorState::from_document(&broken, None).is_err());
}

#[test]
fn the_headless_process_accepts_accents_and_still_rejects_the_unsupported() {
    let lines = [
        set_text(&decomposed()),
        json!({"op":"plan","frame": 0}),
        set_text("preço €"),
        json!({"op":"plan","frame": 0}),
    ];
    let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for line in &lines {
        writeln!(stdin, "{line}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "the unsupported text fails the session"
    );
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), lines.len());
    assert_eq!(replies[0]["ok"], true);
    assert_eq!(replies[2]["error"]["code"], "unsupported_text");
    // The accepted title is still there after the rejected one.
    let layer = replies[3]["result"]["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|layer| layer["kind"] == "text")
        .unwrap()
        .clone();
    assert_eq!(layer["text"]["content"], COMPOSED);
}
