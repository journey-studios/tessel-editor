//! F062: the reads (`list/frame/catalog/inspect`), the inspector choice, the persistence and the
//! headless process. The helpers and the corpora are the ones of `scene_fx_shot_scope.rs`.

use super::*;
use std::fs;
use std::io::Write;
use std::process::{Command as Process, Stdio};

use tessel_editor_core::persist;

// ---- reads ---------------------------------------------------------------------------------------------

#[test]
fn the_reads_say_the_scope_in_text_and_list_the_shots() {
    let mut state = session_with_layer(layer_in_stages());
    let catalog = ok(&mut state, json!({"op":"scene.fx.catalog"}));
    assert_eq!(catalog["scope"]["scope"], json!(["output", "shot"]));
    assert!(
        catalog["scope"]["scope_text"]["shot"]
            .as_str()
            .unwrap()
            .contains("shot:<id>")
    );
    assert_eq!(
        catalog["scope"]["shot_scope"]["requires"],
        "capability fxShotScope, only in a plan with a shot pass"
    );
    assert_eq!(
        catalog["scope"]["shot_scope"]["refusals"]["a shot the edit does not have"],
        "UNKNOWN_SHOT"
    );
    assert_eq!(
        catalog["scope"]["shot_scope"]["refusals"]["a clip that starts before the shot or ends after it"],
        "FX_SCOPE_MISMATCH"
    );
    assert!(
        catalog["scope"]["refused"]
            .as_object()
            .unwrap()
            .keys()
            .any(|key| key.contains("node")),
        "the node scope is refused, with its reason"
    );

    let listing = ok(&mut state, json!({"op":"scene.fx.list","scene":"street"}));
    assert_eq!(
        listing["shots"],
        json!([
            {"id":"shot-1","start_frame":0,"end_frame_exclusive":48,"intent":"synthetic shot shot-1"},
            {"id":"shot-2","start_frame":48,"end_frame_exclusive":96,"intent":"synthetic shot shot-2"}
        ])
    );
    let texts: Vec<(&str, &str)> = listing["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap().iter())
        .map(|clip| {
            (
                clip["id"].as_str().unwrap(),
                clip["scope"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        texts,
        [
            ("o1", "output"),
            ("a1", "shot:shot-1"),
            ("a2", "shot:shot-2"),
            ("b1", "shot:shot-1")
        ]
    );

    // The displayed frame's shot: its id, its interval and what it is for.
    let at_60 = frame(&mut state, 60);
    assert_eq!(
        at_60["shot"],
        json!({"id":"shot-2","start_frame":48,"end_frame_exclusive":96,"intent":"synthetic shot shot-2"})
    );
}

#[test]
fn the_inspector_offers_the_scope_as_a_choice_in_three_languages_with_the_op_that_chooses_it() {
    let mut state = session();
    edit(&mut state, json!([add("v1", 10, 20)]));
    let inspect = |state: &mut EditorState, locale: &str| -> Value {
        ok(
            state,
            json!({"op":"scene.fx.inspect","scene":"street","clip":"v1","frame":15,"locale":locale}),
        )
    };
    let answer = inspect(&mut state, "en");
    assert_eq!(answer["shot"]["id"], "shot-1", "the displayed frame's shot");
    assert_eq!(answer["shots"].as_array().unwrap().len(), 2);
    let scopes: Vec<(&str, bool)> = answer["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|scope| {
            (
                scope["kind"].as_str().unwrap(),
                scope["applicable"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        scopes,
        [("output", true), ("shot", true), ("node", false)],
        "the node scope is listed and refused"
    );
    let node = &answer["scopes"][2];
    assert_eq!(node["reason"], "FX_SCOPE_NOT_YET");

    let choice = &answer["clips"][0]["choices"][0];
    assert_eq!(choice["id"], "scope");
    assert_eq!(choice["type"], "choice");
    assert_eq!(choice["label"], "Scope");
    assert_eq!(
        choice["labels"],
        json!({"pt-br":"Escopo","en":"Scope","es":"Alcance"})
    );
    assert_eq!(choice["value"], "output");
    assert_eq!(choice["default"], "output");
    assert_eq!(choice["at_default"], true);
    let options = choice["options"].as_array().unwrap();
    assert_eq!(
        options
            .iter()
            .map(|option| option["value"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["output", "shot:shot-1", "shot:shot-2"]
    );
    assert_eq!(options[0]["selected"], true);
    assert_eq!(options[0]["op"], Value::Null, "the one it has needs no op");
    // The clip is [10, 30): it fits `shot-1`, not `shot-2`, and the choice says so with a code.
    assert_eq!(options[1]["applicable"], true);
    assert_eq!(
        options[1]["labels"],
        json!({"pt-br":"Plano shot-1","en":"Shot shot-1","es":"Plano shot-1"})
    );
    assert_eq!(options[1]["intent"], "synthetic shot shot-1");
    assert_eq!(options[1]["start_frame"], 0);
    assert_eq!(options[1]["end_frame_exclusive"], 48);
    assert_eq!(options[2]["applicable"], false);
    assert_eq!(options[2]["reason"], "FX_SCOPE_MISMATCH");
    assert_eq!(
        options[2]["op"],
        Value::Null,
        "a shot that cannot hold it offers no op"
    );

    // The op the inspector hands out is the op that works.
    let chosen = options[1]["op"].clone();
    assert_eq!(chosen, set_scope("v1", "shot:shot-1"));
    edit(&mut state, json!([chosen]));
    let after = inspect(&mut state, "pt-br");
    let choice = &after["clips"][0]["choices"][0];
    assert_eq!(choice["label"], "Escopo");
    assert_eq!(choice["value"], "shot:shot-1");
    assert_eq!(choice["at_default"], false);
    assert_eq!(choice["options"][1]["selected"], true);
    assert_eq!(choice["options"][1]["label"], "Plano shot-1");
    // And now going back is offered, as the op that works.
    assert_eq!(choice["options"][0]["op"], set_scope("v1", "output"));
    assert_eq!(
        inspect(&mut state, "es")["clips"][0]["choices"][0]["label"],
        "Alcance"
    );
}

#[test]
fn the_duration_reset_the_inspector_hands_out_is_an_op_the_edit_accepts() {
    let mut state = session();
    // Frame 40 of `shot-1` ([0, 48)) and of the output: the room that is left is 8 and 56 frames.
    edit(
        &mut state,
        json!([
            add_scoped("in-shot", 40, 8, "shot:shot-1"),
            add("on-output", 40, 56)
        ]),
    );
    for (clip, room) in [("in-shot", 8.0), ("on-output", 56.0)] {
        let field = |state: &mut EditorState| -> Value {
            let answer = ok(
                state,
                json!({"op":"scene.fx.inspect","scene":"street","clip":clip}),
            );
            let fields = answer["clips"][0]["fields"].as_array().unwrap().clone();
            fields
                .into_iter()
                .find(|field| field["id"] == "duration_frames")
                .unwrap()
        };
        // The default is the typical length cut where the scope ends, so a clip of that length has
        // nothing to reset, and a shorter one is offered exactly that.
        let at_room = field(&mut state);
        assert_eq!(at_room["default"], json!(room), "{clip}");
        assert_eq!(at_room["reset"]["needed"], false, "{clip}");
        let retime = |value: f64| json!({"op":"set_field","clip":clip,"field":"duration_frames","value":value as i64});
        edit(&mut state, json!([retime(5.0)]));
        let reset = field(&mut state)["reset"].clone();
        assert_eq!(
            (reset["needed"].clone(), reset["to"].clone()),
            (json!(true), json!(room))
        );
        // The op the inspector hands out is the op that works: the importer accepts it and the
        // clip fills the room.
        assert_eq!(reset["op"], retime(room), "{clip}");
        edit(&mut state, json!([reset["op"].clone()]));
        assert_eq!(
            clip_of(&document(&state), clip)["durationFrames"],
            room as i64,
            "{clip}"
        );
    }
}

/// The limits of the start and the duration fields of a clip on a shot are its shot's, never the edit's: the
/// clip would be refused (`FX_SCOPE_MISMATCH`) past its shot, so the slider must stop there. The numbers are
/// the corpus's shots, `[0, 48)` and `[48, 96)`, over an edit of 96 frames, and every end the fields announce is
/// held to what `edit` does with it: the end is accepted and one frame past it is refused, by the shot
/// (`FX_SCOPE_MISMATCH`) or, where the shot ends with the edit, by the edit (`FX_CLIP_OUT_OF_RANGE`).
///
/// Clips on the output are not judged here: their limits still offer what the rule refuses (BL082.69).
#[test]
fn the_timing_fields_of_a_shot_clip_stop_where_its_shot_ends() {
    // `(clip, scope, start, duration, first start, last start, longest duration, refusal past them)`: the last
    // start leaves the clip as long as it is inside the shot, and the longest duration counts from where the
    // clip starts. The first shot ends inside the edit; the second one ends with it.
    for (clip, scope, start, duration, first, last, longest, refusal) in [
        ("early", "shot:shot-1", 40, 8, 0, 40, 8, "FX_SCOPE_MISMATCH"),
        (
            "late",
            "shot:shot-2",
            60,
            10,
            48,
            86,
            36,
            "FX_CLIP_OUT_OF_RANGE",
        ),
    ] {
        let mut state = session();
        edit(
            &mut state,
            json!([add_scoped(clip, start, duration, scope)]),
        );
        let answer = ok(
            &mut state,
            json!({"op":"scene.fx.inspect","scene":"street","clip":clip}),
        );
        let range = |name: &str| -> (Value, Value) {
            let field = answer["clips"][0]["fields"]
                .as_array()
                .unwrap()
                .iter()
                .find(|field| field["id"] == name)
                .unwrap_or_else(|| panic!("no `{name}` in {answer}"));
            (field["min"].clone(), field["max"].clone())
        };
        assert_eq!(
            range("start_frame"),
            (json!(first as f64), json!(last as f64)),
            "{clip}"
        );
        assert_eq!(
            range("duration_frames"),
            (json!(1.0), json!(longest as f64)),
            "{clip}"
        );

        // What the limits announce is what the edit accepts, and one frame past it is not.
        let retime = |field: &str, value: i64| json!([{"op":"set_field","clip":clip,"field":field,"value":value}]);
        let span = |state: &EditorState| {
            let found = clip_of(&document(state), clip);
            (
                found["startFrame"].as_i64().unwrap(),
                found["durationFrames"].as_i64().unwrap(),
            )
        };
        edit(&mut state, retime("duration_frames", longest));
        assert_eq!(span(&state), (start, longest), "{clip}");
        let past = edit_err(&mut state, retime("duration_frames", longest + 1));
        assert_eq!(issue_codes(&past), [refusal], "{clip}");
        edit(&mut state, retime("duration_frames", duration));
        for edge in [first, last] {
            edit(&mut state, retime("start_frame", edge));
            assert_eq!(span(&state), (edge, duration), "{clip}");
        }
        let past = edit_err(&mut state, retime("start_frame", last + 1));
        assert_eq!(issue_codes(&past), [refusal], "{clip}");
        assert_eq!(
            span(&state),
            (last, duration),
            "{clip}: a refusal writes nothing"
        );
    }
}

// ---- persistence ---------------------------------------------------------------------------------------

#[test]
fn a_shot_scope_survives_save_and_reopen_and_evaluates_the_same() {
    let mut state = session_with_layer(layer_in_stages());
    let scratch = std::env::temp_dir().join(format!("tessel-fx-shot-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    let path = scratch.join("fx-shot.tessel.json");
    persist::save(&mut state, &path).unwrap();

    let mut reopened = EditorState::empty(&ProjectSettings::default()).unwrap();
    ok(&mut reopened, json!({"op":"project.open","path":path}));
    assert_eq!(document(&reopened), document(&state));
    for at in [0, 9, 10, 19, 20, 47, 48, 60, 95] {
        assert_eq!(
            order(&mut reopened, at),
            order(&mut state, at),
            "frame {at}"
        );
        assert_eq!(
            passes(&mut reopened, at),
            passes(&mut state, at),
            "frame {at}"
        );
    }
    // The reopened project edits and undoes its own scope changes from there.
    let depth = undo_depth(&reopened);
    edit(&mut reopened, json!([set_scope("b1", "output")]));
    assert_eq!(
        order(&mut reopened, 10),
        ["a1", "o1", "b1"].map(String::from)
    );
    assert_eq!(undo_depth(&reopened), depth + 1);
    ok(&mut reopened, json!({"op":"history.undo"}));
    assert_eq!(order(&mut reopened, 10), ["a1", "b1", "o1"]);
    fs::remove_dir_all(&scratch).unwrap();
}

// ---- the headless process ---------------------------------------------------------------------------------

fn edit_request(ops: Value) -> String {
    json!({"op":"scene.fx.edit","scene":"street","ops":ops}).to_string()
}

#[test]
fn the_headless_executable_scopes_a_clip_like_the_library_and_refuses_the_same_way() {
    let requests = [
        json!({"op":"scene.attach","id":"street","bundle":bundle_of(MINIMAL)}).to_string(),
        edit_request(json!([add("v1", 10, 20), add("o1", 0, 96)])),
        edit_request(json!([set_scope("v1", "shot:shot-1")])),
        json!({"op":"scene.fx.frame","scene":"street","frame":15}).to_string(),
        json!({"op":"scene.fx.frame","scene":"street","frame":60}).to_string(),
        json!({"op":"scene.plan","scene":"street","frame":15}).to_string(),
        edit_request(json!([set_scope("v1", "shot:shot-2")])),
        edit_request(json!([set_scope("v1", "shot:ghost")])),
        edit_request(json!([set_scope("v1", "node:n1")])),
        edit_request(json!([set_scope("v1", "output")])),
        json!({"op":"scene.fx.frame","scene":"street","frame":15}).to_string(),
        json!({"op":"history.undo"}).to_string(),
        json!({"op":"scene.fx.list","scene":"street"}).to_string(),
    ];
    let input = requests.join("\n");
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let mut library = Vec::new();
    headless::run_session(&mut state, None, input.as_bytes(), &mut library).unwrap();

    let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
        .arg("--empty")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    // A session with a refused request exits 1 (the executable's contract), after answering all.
    assert_eq!(output.status.code(), Some(1));
    let lines = |bytes: &[u8]| -> Vec<Value> {
        String::from_utf8_lossy(bytes)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };
    let answers = lines(&output.stdout);
    assert_eq!(
        answers,
        lines(&library),
        "the process answers like the library"
    );

    assert_eq!(answers[2]["result"]["changes"][0]["action"], "set_scope");
    assert_eq!(answers[3]["result"]["shot"]["id"], "shot-1");
    assert_eq!(
        answers[3]["result"]["passes"][0]["scope"],
        json!({"kind":"shot","shotId":"shot-1"}),
        "frame 15: the shot's pass first"
    );
    assert_eq!(answers[3]["result"]["passes"][0]["clipId"], "v1");
    assert_eq!(answers[3]["result"]["passes"][1]["clipId"], "o1");
    assert_eq!(answers[4]["result"]["shot"]["id"], "shot-2");
    assert_eq!(
        answers[4]["result"]["passes"].as_array().unwrap().len(),
        1,
        "frame 60 is in the other shot: only the output draws"
    );
    assert!(
        answers[5]["result"]["plan"]["requires"]
            .as_array()
            .unwrap()
            .contains(&json!("fxShotScope"))
    );
    assert_eq!(answers[6]["ok"], false, "{:?}", answers[6]);
    assert_eq!(answers[6]["error"]["code"], "fx_scope_mismatch");
    assert_eq!(answers[6]["error"]["details"]["shot"], "shot-2");
    assert_eq!(answers[7]["error"]["code"], "unknown_shot");
    assert_eq!(answers[8]["error"]["code"], "fx_scope_not_yet");
    assert_eq!(answers[9]["result"]["changed"], true);
    assert_eq!(
        answers[10]["result"]["passes"][0]["scope"],
        json!({"kind":"output"}),
        "back to the output"
    );
    assert_eq!(answers[11]["ok"], true);
    assert_eq!(
        answers[12]["result"]["tracks"][0]["clips"][0]["scope"],
        "shot:shot-1"
    );
}

#[test]
fn scope_is_part_of_the_shape_the_edit_vocabulary_documents() {
    // A request with an unknown op names the shapes: `set_scope` and `scope` are among them.
    let mut state = session();
    let error = edit_err(&mut state, json!([{"op":"explode"}]));
    let text = format!("{} {:?}", error.message, error.details);
    assert!(text.contains("set_scope"), "{text}");
    assert!(text.contains("shot:<id>"), "{text}");
    // The scope of the model is one value: its text round-trips for the output and for a shot.
    assert_eq!(FxScope::parse_text("output"), Ok(FxScope::Output));
    assert_eq!(
        FxScope::parse_text("shot:a:b"),
        Ok(FxScope::Shot("a:b".to_string()))
    );
    assert_eq!(FxScope::Shot("a:b".to_string()).to_text(), "shot:a:b");
}
