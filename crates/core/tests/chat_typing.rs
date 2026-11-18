//! F070: progressive typing with stable line breaks, reveal by line, and the chat preset
//! (campaign, question, answer, source) as a group, through the headless contract: the plan on
//! the project clock, edits through `set`, undo, save and reopen, and refusals that leave the
//! project untouched. Every text here is fictitious and generic: the repo is public.
use std::collections::HashMap;

use serde_json::{Value, json};
use tessel_editor_core::state::layout::Layout;
use tessel_editor_core::state::tree::{TreeContext, nodes};
use tessel_editor_core::{EditorState, headless};

const CANVAS: (u32, u32) = (1280, 720);
const RATE: f64 = 30.0;

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn refused(state: &mut EditorState, request: Value) -> String {
    headless::execute(state, &request)
        .expect_err(&format!("{request} must be refused"))
        .code
        .to_string()
}

/// A project with one chat clip at frame 0 (the preset makes it 8 s long).
fn chat() -> EditorState {
    let mut state = EditorState::demo();
    run(
        &mut state,
        json!({"op":"project.create","name":"Chat","rate":{"num":30,"den":1},
            "width":CANVAS.0,"height":CANVAS.1,"length":600}),
    );
    run(
        &mut state,
        json!({"op":"invoke","target":"timeline.add_chat","action":"invoke"}),
    );
    state
}

fn layers(state: &mut EditorState, frame: i64) -> Vec<Value> {
    run(state, json!({"op":"plan","frame":frame}))["layers"]
        .as_array()
        .unwrap()
        .clone()
}

/// First frame whose clock is at or after `ms` on the clip.
fn frame_at(ms: i64) -> i64 {
    (ms as f64 * RATE / 1000.0).ceil() as i64
}

fn layer(layers: &[Value], child: &str) -> Option<Value> {
    layers
        .iter()
        .find(|layer| layer["id"].as_str().is_some_and(|id| id.ends_with(child)))
        .cloned()
}

fn timings(state: &EditorState) -> HashMap<String, (i64, i64)> {
    let instance = state.timeline.clips[0].group.clone().unwrap();
    let def = state.groups.def(instance.def.as_str()).unwrap();
    state
        .groups
        .timings(&instance, CANVAS)
        .unwrap()
        .iter()
        .zip(&def.children)
        .map(|(timing, child)| (child.id.to_string(), (timing.start_ms, timing.end_ms)))
        .collect()
}

#[test]
fn one_chat_action_defines_the_group_and_its_clip_and_undo_removes_both() {
    let mut state = chat();
    assert_eq!(state.groups.defs.len(), 1);
    assert_eq!(state.groups.defs[0].children.len(), 6);
    assert_eq!(state.groups.styles.len(), 4);
    assert_eq!(state.timeline.clips.len(), 1);
    assert_eq!(state.timeline.clips[0].duration, 240, "8 s at 30 fps");
    assert_eq!(state.history.to_json()["undo_depth"], 1);

    // A second chat shares the styles and is a second group.
    run(&mut state, json!({"op":"chat.add"}));
    assert_eq!(state.groups.defs.len(), 2);
    assert_eq!(state.groups.styles.len(), 4);

    run(&mut state, json!({"op":"history.undo"}));
    run(&mut state, json!({"op":"history.undo"}));
    assert!(state.groups.is_empty());
    assert!(state.timeline.clips.is_empty());
}

#[test]
fn the_sequence_runs_question_then_answer_by_line_then_source_then_mark() {
    let mut state = chat();
    let at = timings(&state);
    let (question, answer, source, mark) = (
        at["question"],
        at["answer"],
        at["source"],
        at["source-mark"],
    );
    // Each step starts when the one before ends, plus its delay.
    assert_eq!(answer.0, question.1 + 300);
    assert_eq!(source.0, answer.1 + 200);
    assert_eq!(mark.0, source.1);

    // Before the question starts nothing of the sequence is drawn.
    let waiting = layers(&mut state, frame_at(300));
    for child in ["question", "answer", "source"] {
        assert!(
            layer(&waiting, child).is_none(),
            "{child} waits for its turn"
        );
    }

    // The question types in a character at a time.
    let mut typed = Vec::new();
    for ms in [question.0 + 1, question.0 + 200, question.0 + 600] {
        let planned = layers(&mut state, frame_at(ms));
        let text = layer(&planned, "question").expect("the question is typing")["text"].clone();
        assert!(
            layer(&planned, "answer").is_none(),
            "the answer has not started"
        );
        typed.push(
            text["visible"]
                .as_u64()
                .expect("a partial text states `visible`"),
        );
    }
    assert!(
        typed[0] >= 1 && typed[0] < typed[1] && typed[1] < typed[2],
        "{typed:?}"
    );

    // Once typed the text states no count: the whole text is drawn.
    let done = layers(&mut state, frame_at(question.1 + 40));
    assert!(
        layer(&done, "question").unwrap()["text"]
            .get("visible")
            .is_none()
    );

    // The answer shows one whole line at a time, and its lines never change.
    let first = layers(&mut state, frame_at(answer.0 + 5));
    let first = layer(&first, "answer").expect("the answer starts")["text"].clone();
    let lines = first["lines"]
        .as_array()
        .expect("a boxed text states its lines")
        .clone();
    assert!(lines.len() >= 2, "the sample answer wraps: {lines:?}");
    assert_eq!(
        first["visible"].as_u64().unwrap() as usize,
        lines[0].as_str().unwrap().chars().count(),
        "only the first line shows"
    );
    let finished = layers(&mut state, frame_at(answer.1 + 5));
    let finished = layer(&finished, "answer").unwrap()["text"].clone();
    assert_eq!(finished["lines"], first["lines"], "line breaks are stable");
    assert!(finished.get("visible").is_none());

    // The source types in after the answer, then the mark appears.
    let before_source = layers(&mut state, frame_at(source.0 - 40));
    assert!(layer(&before_source, "source").is_none());
    let mark_before = layers(&mut state, frame_at(mark.0 - 1));
    assert_eq!(layer(&mark_before, "source-mark").unwrap()["opacity"], 0.0);
    let mark_after = layers(&mut state, frame_at(mark.0 + 400));
    assert_eq!(layer(&mark_after, "source-mark").unwrap()["opacity"], 1.0);
}

#[test]
fn editing_the_question_retimes_the_answer_and_the_source() {
    let mut state = chat();
    let before = timings(&state);
    let clip = state.timeline.clips[0].id.to_string();
    run(
        &mut state,
        json!({"op":"clip.group_text","clip":clip,"child":"question",
            "text":"Quem guarda a chave da torre, e onde ela fica escondida?"}),
    );
    let after = timings(&state);
    assert!(after["question"].1 > before["question"].1);
    assert_eq!(after["answer"].0, after["question"].1 + 300);
    assert_eq!(after["source"].0, after["answer"].1 + 200);

    // At the old moment the answer would have started, the longer question is still typing.
    let planned = layers(&mut state, frame_at(before["answer"].0 + 100));
    assert!(layer(&planned, "answer").is_none());
    assert!(layer(&planned, "question").is_some());
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(timings(&state), before);
}

#[test]
fn reveal_schedule_and_style_width_are_editable_by_set_and_undo_restores_the_document() {
    let mut state = chat();
    let group = state.groups.defs[0].id.to_string();
    let original = state.to_document(None);
    let before = timings(&state);

    for (target, field, value) in [
        (
            format!("group_child:{group}/question"),
            "step_ms",
            json!(20),
        ),
        (
            format!("group_child:{group}/answer"),
            "reveal",
            json!("type"),
        ),
        (format!("group_child:{group}/answer"), "delay_ms", json!(0)),
        (format!("group_child:{group}/source"), "after", json!("")),
        (
            format!("group_child:{group}/source"),
            "delay_ms",
            json!(1000),
        ),
        ("text_style:chat-answer".to_string(), "width", json!(40)),
    ] {
        run(
            &mut state,
            json!({"op":"set","target":target,"field":field,"value":value}),
        );
    }
    let edited = timings(&state);
    assert_eq!(
        edited["question"].1 - edited["question"].0,
        29 * 20,
        "29 characters at 20 ms"
    );
    assert_eq!(
        edited["source"].0, 1000,
        "no `after` any more: stagger plus its delay"
    );
    assert_ne!(edited, before);
    assert_eq!(state.groups.style("chat-answer").unwrap().width, 40.0);
    assert_eq!(state.history.to_json()["undo_depth"], 7);

    for _ in 0..6 {
        run(&mut state, json!({"op":"history.undo"}));
    }
    assert_eq!(state.to_document(None), original);
    assert_eq!(timings(&state), before);
}

#[test]
fn the_inspector_publishes_the_new_fields_of_a_chat_child() {
    let state = chat();
    let metrics = HashMap::new();
    let layout = Layout {
        regions: Vec::new(),
        area: (0.0, 0.0),
    };
    let cx = TreeContext {
        theme: "dark",
        metrics: &metrics,
        layout: &layout,
        perf_meter: false,
    };
    let published = nodes(&state, &cx);
    let node = |id: &str| {
        published
            .iter()
            .find(|node| node["id"] == id)
            .unwrap_or_else(|| panic!("node {id}"))
            .clone()
    };
    let group = state.groups.defs[0].id.to_string();
    let answer = node(&format!("group_child:{group}/answer"));
    assert_eq!(answer["state"]["reveal"], "lines");
    assert_eq!(answer["state"]["after"], "question");
    assert_eq!(answer["state"]["delay_ms"], 300);
    let fields: Vec<&str> = answer["fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|field| field["field"].as_str())
        .collect();
    for field in ["reveal", "step_ms", "after", "delay_ms"] {
        assert!(fields.contains(&field), "{field} in {fields:?}");
    }
    assert_eq!(node("text_style:chat-answer")["state"]["width"], 62.0);
    assert_eq!(node("timeline.add_chat")["label"], "Adicionar chat");
    // A shape has a schedule but no reveal.
    let mark = node(&format!("group_child:{group}/source-mark"));
    assert_eq!(mark["state"]["after"], "source");
    assert!(mark["state"].get("reveal").is_none());
}

#[test]
fn refusals_leave_the_project_and_its_history_untouched() {
    let mut state = chat();
    let group = state.groups.defs[0].id.to_string();
    let before = state.to_document(None);
    let history = state.history.to_json();
    let requests = [
        // `after` must name an earlier child (and not the child itself).
        (
            json!({"op":"set","target":format!("group_child:{group}/question"),"field":"after","value":"answer"}),
            "invalid_value",
        ),
        (
            json!({"op":"set","target":format!("group_child:{group}/question"),"field":"after","value":"question"}),
            "invalid_value",
        ),
        // A step needs a reveal; a shape has none.
        (
            json!({"op":"set","target":format!("group_child:{group}/campaign"),"field":"step_ms","value":50}),
            "not_applicable",
        ),
        (
            json!({"op":"set","target":format!("group_child:{group}/panel"),"field":"reveal","value":"type"}),
            "unknown_field",
        ),
        (
            json!({"op":"set","target":format!("group_child:{group}/question"),"field":"step_ms","value":0}),
            "invalid_value",
        ),
        (
            json!({"op":"set","target":format!("group_child:{group}/question"),"field":"delay_ms","value":-5}),
            "invalid_value",
        ),
        (
            json!({"op":"set","target":format!("group_child:{group}/question"),"field":"reveal","value":"slowly"}),
            "invalid_value",
        ),
        (
            json!({"op":"set","target":"text_style:chat-answer","field":"width","value":140}),
            "invalid_value",
        ),
        // Deleting what another child waits for is refused, not silently rewired.
        (
            json!({"op":"invoke","target":format!("group_child:{group}/question"),"action":"remove"}),
            "in_use",
        ),
    ];
    for (request, code) in requests {
        assert_eq!(refused(&mut state, request.clone()), code, "{request}");
        assert_eq!(
            state.to_document(None),
            before,
            "{request} changed the project"
        );
        assert_eq!(
            state.history.to_json(),
            history,
            "{request} recorded history"
        );
    }
}

#[test]
fn the_chat_survives_save_and_reopen_with_the_same_plan() {
    let mut state = chat();
    let clip = state.timeline.clips[0].id.to_string();
    run(
        &mut state,
        json!({"op":"clip.group_text","clip":clip,"child":"answer",
            "text":"A resposta cabe em outra linha quando o texto fica mais longo do que o cartão."}),
    );
    let frames = [0, 14, 30, 45, 60, 75, 90, 120, 180];
    let planned: Vec<Vec<Value>> = frames.iter().map(|f| layers(&mut state, *f)).collect();

    let document = state.to_document(None);
    assert_eq!(document["schema_version"], 2);
    let text = document.to_string();
    for key in ["\"reveal\"", "\"after\"", "\"delay_ms\""] {
        assert!(text.contains(key), "{key} is saved");
    }
    assert!(
        document["groups"]["styles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|style| style["id"] == "chat-answer" && style["width"] == 62.0),
        "the box width of a style is saved"
    );
    let mut reopened = EditorState::from_document(&document, None).unwrap();
    assert_eq!(reopened.to_document(None), document);
    let replanned: Vec<Vec<Value>> = frames.iter().map(|f| layers(&mut reopened, *f)).collect();
    assert_eq!(planned, replanned);

    // A reveal on a shape, or `after` pointing forward, cannot be forged into a document.
    let mut forged = document.clone();
    forged["groups"]["definitions"][0]["children"][0]["reveal"] = json!({"mode":"type"});
    assert!(EditorState::from_document(&forged, None).is_err());
    let mut forward = document.clone();
    forward["groups"]["definitions"][0]["children"][2]["after"] = json!("answer");
    assert!(EditorState::from_document(&forward, None).is_err());
}

#[test]
fn a_group_without_the_new_fields_is_saved_exactly_as_before() {
    let mut state = EditorState::demo();
    run(
        &mut state,
        json!({"op":"project.create","name":"Plain","rate":{"num":30,"den":1},
            "width":1280,"height":720,"length":300}),
    );
    run(&mut state, json!({"op":"group.add"}));
    let text = state.to_document(None).to_string();
    for key in ["\"reveal\"", "\"after\"", "\"delay_ms\""] {
        assert!(!text.contains(key), "{key} must stay out of an older group");
    }
    // A style with the automatic width has no `width` either.
    let document = state.to_document(None);
    assert!(document["groups"]["styles"][0].get("width").is_none());
}
