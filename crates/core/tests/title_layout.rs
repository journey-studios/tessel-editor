//! BL141.1: a title that does not fit one line of the frame is wrapped at its spaces, an explicit
//! `\n` starts a line, and a word or a block too large for the frame shrinks the title instead of
//! being cut by the edges. The core lays the title out (`text::layout`) and the plan carries the
//! result (`text.lines`, `text.scale`) only when it is not the one full-size line every title had
//! before, so a title that fits keeps the plan it had. Pixels: `crates/render/tests/title_layout.rs`.
use std::io::Write;
use std::process::{Command as Process, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::text::{LINE_PITCH_ROWS, SAFE_AREA};
use tessel_editor_core::{EditorState, headless};

const LONG: &str = "Ação, Câmera e Edição: São Paulo";

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

/// A project of `width × height` with one title clip.
fn project(width: u32, height: u32, content: &str, size: f64) -> EditorState {
    let mut state = EditorState::demo();
    for request in [
        json!({"op":"project.create","name":"Titles","sequence":"main","rate":24,
            "width":width,"height":height,"length":48}),
        json!({"op":"asset.add","kind":"text","id":"label","name":"Label",
            "text":{"content":content,"size":size,"color":"#ffffff"}}),
        json!({"op":"track.add","id":"titles","name":"Titles","kind":"video"}),
        json!({"op":"clip.add","id":"c-title","track":"titles","asset":"label",
            "start":0,"duration":48}),
    ] {
        run(&mut state, request);
    }
    state
}

fn planned_text(state: &mut EditorState) -> Value {
    let plan = run(state, json!({"op":"plan","frame":0}));
    plan["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|layer| layer["kind"] == "text")
        .unwrap_or_else(|| panic!("no text layer in {plan}"))["text"]
        .clone()
}

fn lines(text: &Value) -> Vec<String> {
    text["lines"]
        .as_array()
        .unwrap_or_else(|| panic!("no lines in {text}"))
        .iter()
        .map(|line| line.as_str().unwrap().to_string())
        .collect()
}

/// Width and height the drawn block takes, in fractions of the frame (the runtime's geometry).
fn block(text: &Value, width: f64, height: f64) -> (f64, f64) {
    let size = text["size"].as_f64().unwrap();
    let scale = text["scale"].as_f64().unwrap_or(1.0);
    let cell = size / 100.0 * height / 7.0 * scale;
    let lines = text.get("lines").map_or_else(
        || vec![text["content"].as_str().unwrap().to_string()],
        |_| lines(text),
    );
    let widest = lines.iter().map(|line| line.chars().count()).max().unwrap();
    // A title with any accented letter gets 2 rows of marks above and below (MARK_ROWS).
    let marked = text["content"]
        .as_str()
        .unwrap()
        .chars()
        .any(|ch| "ÀÁÂÃÄÇÈÉÊËÌÍÎÏÑÒÓÔÕÖÙÚÛÜÝàáâãäçèéêëìíîïñòóôõöùúûüý".contains(ch));
    let rows = (lines.len() - 1) * LINE_PITCH_ROWS + 7 + if marked { 4 } else { 0 };
    (
        (widest * 6 - 1) as f64 * cell / width,
        rows as f64 * cell / height,
    )
}

#[test]
fn a_title_that_fits_has_the_plan_it_always_had() {
    let mut state = project(1920, 1080, "AÇÃO NA PRAIA", 10.0);
    let text = planned_text(&mut state);
    assert_eq!(
        text,
        json!({"content":"AÇÃO NA PRAIA","size":10.0,"color":"#ffffff"}),
        "no lines, no scale"
    );
}

#[test]
fn a_long_title_is_wrapped_inside_the_frame_in_every_shape_of_sequence() {
    for (width, height) in [(1920, 1080), (1080, 1920), (1080, 1080)] {
        let mut state = project(width, height, LONG, 10.0);
        let text = planned_text(&mut state);
        let lines = lines(&text);
        assert!(lines.len() >= 2, "{width}x{height}: {text}");
        assert_eq!(
            lines.join(" "),
            LONG,
            "{width}x{height}: no word lost or cut"
        );
        assert_eq!(text["content"], LONG, "the stored text is untouched");
        let (w, h) = block(&text, f64::from(width), f64::from(height));
        assert!(
            w <= SAFE_AREA + 1e-9 && h <= SAFE_AREA + 1e-9,
            "{width}x{height}: {w} x {h}"
        );
        let scale = text["scale"].as_f64().unwrap();
        if height <= width {
            assert_eq!(scale, 1.0, "{width}x{height}: wrapping was enough");
        } else {
            // 1080 wide at size 10 holds 6 characters a line and "Edição:" has 7: the title
            // shrinks just enough for its longest word, never more.
            assert!(scale < 1.0 && scale > 0.8, "{width}x{height}: {scale}");
        }
    }
}

#[test]
fn an_explicit_break_and_a_giant_size_are_honoured_without_leaving_the_frame() {
    let mut state = project(1920, 1080, "PRIMEIRA\nSEGUNDA", 10.0);
    let text = planned_text(&mut state);
    assert_eq!(lines(&text), ["PRIMEIRA", "SEGUNDA"]);

    // A size of 100 % with a long word: it shrinks to the safe width, it is never cut.
    let mut state = project(1920, 1080, "INCONSTITUCIONALISSIMAMENTE", 100.0);
    let text = planned_text(&mut state);
    let scale = text["scale"].as_f64().unwrap();
    assert!(scale < 0.2, "{text}");
    let (w, h) = block(&text, 1920.0, 1080.0);
    assert!(
        (w - SAFE_AREA).abs() < 1e-9,
        "the word fills the safe width: {w}"
    );
    assert!(h <= SAFE_AREA, "{h}");
}

#[test]
fn the_cli_plans_the_layout_and_an_edit_back_to_a_short_title_drops_it() {
    let lines_in = [
        json!({"op":"project.create","name":"Titles","sequence":"main","rate":24,
            "width":1080,"height":1920,"length":48}),
        json!({"op":"asset.add","kind":"text","id":"label","name":"Label",
            "text":{"content":LONG,"size":10,"color":"#ffffff"}}),
        json!({"op":"track.add","id":"titles","name":"Titles","kind":"video"}),
        json!({"op":"clip.add","id":"c-title","track":"titles","asset":"label","start":0,"duration":48}),
        json!({"op":"plan","frame":0}),
        json!({"op":"set","target":"clip_text:c-title","field":"content","value":"CURTO"}),
        json!({"op":"plan","frame":0}),
        json!({"op":"history.undo"}),
        json!({"op":"plan","frame":0}),
    ];
    let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for line in &lines_in {
        writeln!(stdin, "{line}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), lines_in.len());
    assert!(
        replies.iter().all(|reply| reply["ok"] == true),
        "{replies:?}"
    );
    let text_of = |reply: &Value| {
        reply["result"]["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|layer| layer["kind"] == "text")
            .unwrap()["text"]
            .clone()
    };
    let long = text_of(&replies[4]);
    assert!(lines(&long).len() >= 3, "a vertical frame: {long}");
    let short = text_of(&replies[6]);
    assert!(
        short.get("lines").is_none() && short.get("scale").is_none(),
        "{short}"
    );
    assert_eq!(
        text_of(&replies[8]),
        long,
        "undo brings the layout back with the text"
    );
}

/// The layer the plan draws the title with, at frame 0.
fn title_layer(state: &mut EditorState) -> Value {
    let plan = run(state, json!({"op":"plan","frame":0}));
    plan["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|layer| layer["kind"] == "text")
        .unwrap()
        .clone()
}

#[test]
fn a_wrapped_title_in_the_lower_third_grows_upwards_and_a_single_line_stays_put() {
    let mut state = project(320, 180, LONG, 14.0);
    run(
        &mut state,
        json!({"op":"clip.property","clip":"c-title","property":"position_y","value":30}),
    );
    let layer = title_layer(&mut state);
    let y = layer["transform"]["y"].as_f64().unwrap();
    assert!(
        y < 30.0,
        "the block moved up to grow away from the bottom edge: {y}"
    );
    let (_, h) = block(&layer["text"], 320.0, 180.0);
    let bottom = y / 100.0 + h / 2.0;
    assert!(
        bottom <= SAFE_AREA / 2.0 + 1e-9,
        "its bottom stays in the safe area: {bottom}"
    );

    run(
        &mut state,
        json!({"op":"set","target":"clip_text:c-title","field":"content","value":"CURTO"}),
    );
    assert_eq!(
        title_layer(&mut state)["transform"]["y"],
        30.0,
        "one line is where the position puts it, as before"
    );

    run(
        &mut state,
        json!({"op":"set","target":"clip_text:c-title","field":"content","value":LONG}),
    );
    run(
        &mut state,
        json!({"op":"clip.property","clip":"c-title","property":"position_y","value":0}),
    );
    assert_eq!(
        title_layer(&mut state)["transform"]["y"],
        0.0,
        "centred, it grows both ways and nothing moves"
    );
}
