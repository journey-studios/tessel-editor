//! F069: reusable groups with editable children, text styles, keyframes with easing, staggered
//! entrances and repetition, through the headless contract: define, edit, plan, undo, save,
//! reopen and the refusals that leave the project untouched.
use serde_json::{Value, json};
use tessel_editor_core::{EditorState, headless};

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn refused(state: &mut EditorState, request: Value) -> String {
    headless::execute(state, &request)
        .expect_err(&format!("{request} must be refused"))
        .code
        .to_string()
}

fn project() -> EditorState {
    let mut state = EditorState::demo();
    run(
        &mut state,
        json!({"op":"project.create","name":"Groups","rate":{"num":30,"den":1},
            "width":1920,"height":1080,"length":240}),
    );
    run(
        &mut state,
        json!({"op":"track.add","id":"overlay","name":"Overlay","kind":"video"}),
    );
    state
}

fn fade_in() -> Value {
    json!([
        {"prop":"opacity","keys":[
            {"time_ms":0,"value":0,"easing":"easeOutCubic"},
            {"time_ms":400,"value":100}]},
        {"prop":"y","keys":[
            {"time_ms":0,"value":4,"easing":"easeOutCubic"},
            {"time_ms":400,"value":0}]}
    ])
}

/// A card: a panel and a title that fade and slide in one after the other.
fn define_card(state: &mut EditorState) {
    run(
        state,
        json!({"op":"text_style.set","style":{"id":"title","size":8,"color":"#ffffff","font":"sans"}}),
    );
    run(
        state,
        json!({"op":"group.define","definition":{"id":"card","name":"Card","stagger_ms":200,
        "children":[
            {"id":"panel","kind":"shape",
             "shape":{"kind":"rect","width":60,"height":30,"fill":"#102030","radius":4},
             "tracks": fade_in()},
            {"id":"title","kind":"text","content":"Hello","style":"title",
             "tracks": fade_in()}
        ]}}),
    );
    run(
        state,
        json!({"op":"clip.add","id":"card1","track":"overlay","start":0,"duration":90,
            "group":{"def":"card"}}),
    );
}

fn layers(state: &mut EditorState, frame: i64) -> Vec<Value> {
    run(state, json!({"op":"plan","frame":frame}))["layers"]
        .as_array()
        .unwrap()
        .clone()
}

fn near(value: &Value, expected: f64) {
    let value = value
        .as_f64()
        .unwrap_or_else(|| panic!("{value} is not a number"));
    assert!((value - expected).abs() < 1e-6, "{value} is not {expected}");
}

#[test]
fn a_group_clip_plans_one_layer_per_child_bottom_to_top() {
    let mut state = project();
    define_card(&mut state);
    let at_rest = layers(&mut state, 30);
    assert_eq!(at_rest.len(), 2);
    assert_eq!(at_rest[0]["id"], "layer:card1:panel");
    assert_eq!(at_rest[0]["kind"], "shape");
    assert_eq!(at_rest[0]["shape"]["fill"], "#102030");
    assert_eq!(at_rest[1]["id"], "layer:card1:title");
    assert_eq!(at_rest[1]["kind"], "text");
    assert_eq!(at_rest[1]["text"]["content"], "Hello");
    assert_eq!(at_rest[1]["text"]["size"], 8.0);
    assert_eq!(at_rest[1]["text"]["font"], "sans");
    // The group, not its children, is what a clip is: they all point back at it.
    assert_eq!(at_rest[0]["clip"], "card1");
    assert_eq!(at_rest[1]["clip"], "card1");
    // Outside the clip nothing is drawn.
    assert!(layers(&mut state, 100).is_empty());
}

#[test]
fn keys_ease_and_the_stagger_delays_each_child_by_its_position() {
    let mut state = project();
    define_card(&mut state);

    // 30 fps: frame 0 is 0 ms. Both are still hidden (the title waits for its turn).
    let start = layers(&mut state, 0);
    near(&start[0]["opacity"], 0.0);
    near(&start[1]["opacity"], 0.0);
    near(&start[0]["transform"]["y"], 4.0);

    // Frame 6 = 200 ms: the panel is halfway through an ease-out, the title only starts.
    let early = layers(&mut state, 6);
    near(&early[0]["opacity"], 0.875);
    near(&early[1]["opacity"], 0.0);

    // Frame 12 = 400 ms: the panel is done, the title is at the same point the panel was.
    let middle = layers(&mut state, 12);
    near(&middle[0]["opacity"], 1.0);
    near(&middle[0]["transform"]["y"], 0.0);
    near(&middle[1]["opacity"], 0.875);

    // Frame 18 = 600 ms: both rest on their last key, however long the clip lasts.
    let done = layers(&mut state, 60);
    near(&done[1]["opacity"], 1.0);
    near(&done[1]["transform"]["y"], 0.0);
}

#[test]
fn repetition_restarts_or_bounces() {
    let mut state = project();
    define_card(&mut state);
    run(
        &mut state,
        json!({"op":"group.key","group":"card","child":"panel","prop":"rotation",
            "time_ms":0,"value":0}),
    );
    run(
        &mut state,
        json!({"op":"group.key","group":"card","child":"panel","prop":"rotation",
            "time_ms":1000,"value":90}),
    );
    // A single key time cannot be repeated until the track says so: it plays once and rests.
    near(&layers(&mut state, 45)[0]["transform"]["rotation"], 90.0);

    run(
        &mut state,
        json!({"op":"group.repeat","group":"card","child":"panel","prop":"rotation",
            "repeat":0,"mode":"pingpong"}),
    );
    // 1250 ms (frame 37.5 is not a frame; 1233 ms at frame 37) runs the second cycle backwards.
    let backwards = layers(&mut state, 37)[0]["transform"]["rotation"]
        .as_f64()
        .unwrap();
    assert!(
        (backwards - 90.0 * (1.0 - 0.233_333)).abs() < 0.01,
        "{backwards}"
    );

    run(
        &mut state,
        json!({"op":"set","target":"group_track:card/panel/rotation","field":"mode","value":"restart"}),
    );
    let restarted = layers(&mut state, 37)[0]["transform"]["rotation"]
        .as_f64()
        .unwrap();
    assert!((restarted - 90.0 * 0.233_333).abs() < 0.01, "{restarted}");
}

#[test]
fn the_clip_moves_scales_and_turns_the_whole_group() {
    let mut state = project();
    define_card(&mut state);
    run(
        &mut state,
        json!({"op":"group.child","group":"card","child":{
            "id":"badge","kind":"shape","shape":{"width":10,"height":10},"y":10}}),
    );
    for (property, value) in [("position_x", 10), ("zoom", 50), ("rotation", 90)] {
        run(
            &mut state,
            json!({"op":"clip.property","clip":"card1","property":property,"value":value}),
        );
    }
    let badge = layers(&mut state, 60)[2].clone();
    assert_eq!(badge["id"], "layer:card1:badge");
    near(&badge["transform"]["scale"], 0.5);
    near(&badge["transform"]["rotation"], 90.0);
    // The child sits 10 % of the height (108 px) below the origin; the group is half size, so
    // 54 px, and a quarter turn clockwise puts it 54 px to the left of the origin.
    near(&badge["transform"]["x"], 10.0 - 54.0 / 1920.0 * 100.0);
    near(&badge["transform"]["y"], 0.0);
}

#[test]
fn an_instance_changes_a_text_and_clearing_it_goes_back_to_the_group() {
    let mut state = project();
    define_card(&mut state);
    run(
        &mut state,
        json!({"op":"clip.add","id":"card2","track":"overlay","start":100,"duration":60,
            "group":{"def":"card","texts":{"title":"Second"}}}),
    );
    assert_eq!(layers(&mut state, 130)[1]["text"]["content"], "Second");
    assert_eq!(layers(&mut state, 30)[1]["text"]["content"], "Hello");

    run(
        &mut state,
        json!({"op":"set","target":"group_text:card1/title","field":"text","value":"Changed"}),
    );
    assert_eq!(layers(&mut state, 30)[1]["text"]["content"], "Changed");
    // The other instance and the group itself keep their own.
    assert_eq!(layers(&mut state, 130)[1]["text"]["content"], "Second");

    run(
        &mut state,
        json!({"op":"invoke","target":"group_text:card1/title","action":"reset"}),
    );
    assert_eq!(layers(&mut state, 30)[1]["text"]["content"], "Hello");

    // Editing the group's own text shows on every instance that does not override it.
    run(
        &mut state,
        json!({"op":"set","target":"group_child:card/title","field":"content","value":"Welcome"}),
    );
    assert_eq!(layers(&mut state, 30)[1]["text"]["content"], "Welcome");
    assert_eq!(layers(&mut state, 130)[1]["text"]["content"], "Second");
}

#[test]
fn a_text_style_restyles_every_child_that_uses_it() {
    let mut state = project();
    define_card(&mut state);
    run(
        &mut state,
        json!({"op":"set","target":"text_style:title","field":"size","value":12}),
    );
    run(
        &mut state,
        json!({"op":"set","target":"text_style:title","field":"color","value":"#ffcc00"}),
    );
    let title = layers(&mut state, 30)[1]["text"].clone();
    assert_eq!(title["size"], 12.0);
    assert_eq!(title["color"], "#ffcc00");
}

#[test]
fn shape_properties_are_keyed_on_the_shape() {
    let mut state = project();
    define_card(&mut state);
    run(
        &mut state,
        json!({"op":"group.key","group":"card","child":"panel","prop":"width","time_ms":0,"value":10}),
    );
    run(
        &mut state,
        json!({"op":"group.key","group":"card","child":"panel","prop":"width","time_ms":1000,
            "value":50}),
    );
    run(
        &mut state,
        json!({"op":"group.key","group":"card","child":"panel","prop":"radius","time_ms":0,"value":0}),
    );
    run(
        &mut state,
        json!({"op":"group.key","group":"card","child":"panel","prop":"radius","time_ms":1000,
            "value":20}),
    );
    // Frame 15 = 500 ms: linear halfway.
    let panel = layers(&mut state, 15)[0]["shape"].clone();
    near(&panel["width"], 30.0);
    near(&panel["radius"], 10.0);
    near(&panel["height"], 30.0);
}

#[test]
fn every_edit_is_one_undo_entry_that_restores_the_project_exactly() {
    let mut state = project();
    define_card(&mut state);
    let edits = [
        json!({"op":"group.set","group":"card","field":"stagger_ms","value":350}),
        json!({"op":"group.child.set","group":"card","child":"panel","field":"x","value":5}),
        json!({"op":"group.child.set","group":"card","child":"panel","field":"fill","value":"#ffffff"}),
        json!({"op":"group.key","group":"card","child":"panel","prop":"scale","time_ms":0,"value":50}),
        json!({"op":"group.key","group":"card","child":"panel","prop":"scale","time_ms":300,"value":100,"easing":"easeInOutCubic"}),
        json!({"op":"set","target":"group_key:card/panel/scale/1","field":"value","value":120}),
        json!({"op":"group.repeat","group":"card","child":"panel","prop":"scale","repeat":3,"mode":"pingpong"}),
        json!({"op":"clip.group_text","clip":"card1","child":"title","text":"Edited"}),
        json!({"op":"invoke","target":"group_key:card/panel/scale/1","action":"remove"}),
        json!({"op":"invoke","target":"group_track:card/panel/scale","action":"remove"}),
        json!({"op":"text_style.field","style":"title","field":"align","value":"left"}),
        json!({"op":"group.child","group":"card","child":{"id":"dot","kind":"shape","shape":{}}}),
        json!({"op":"invoke","target":"group_child:card/dot","action":"remove"}),
    ];
    let mut documents = vec![state.to_document(None)];
    let mut depths = vec![state.history.to_json()["undo_depth"].as_i64().unwrap()];
    for edit in &edits {
        run(&mut state, edit.clone());
        documents.push(state.to_document(None));
        let depth = state.history.to_json()["undo_depth"].as_i64().unwrap();
        assert_eq!(depth, depths.last().unwrap() + 1, "{edit} is one entry");
        depths.push(depth);
    }
    for index in (0..edits.len()).rev() {
        run(&mut state, json!({"op":"history.undo"}));
        assert_eq!(
            state.to_document(None),
            documents[index],
            "undo of {}",
            edits[index]
        );
    }
    for index in 0..edits.len() {
        run(&mut state, json!({"op":"history.redo"}));
        assert_eq!(
            state.to_document(None),
            documents[index + 1],
            "redo of {}",
            edits[index]
        );
    }
}

#[test]
fn refusals_name_the_problem_and_change_nothing() {
    let mut state = project();
    define_card(&mut state);
    let before = state.to_document(None);
    let history = state.history.to_json();
    let requests = [
        // Out of range, for the base values and for the keys.
        (
            json!({"op":"group.child.set","group":"card","child":"panel","field":"opacity","value":101}),
            "invalid_value",
        ),
        (
            json!({"op":"group.child.set","group":"card","child":"panel","field":"scale","value":0}),
            "invalid_value",
        ),
        (
            json!({"op":"group.key","group":"card","child":"panel","prop":"rotation","time_ms":0,"value":99999}),
            "invalid_value",
        ),
        (
            json!({"op":"group.key","group":"card","child":"panel","prop":"opacity","time_ms":-1,"value":5}),
            "invalid_value",
        ),
        (
            json!({"op":"group.set","group":"card","field":"stagger_ms","value":-1}),
            "invalid_value",
        ),
        // Unknown things.
        (
            json!({"op":"group.key","group":"card","child":"ghost","prop":"opacity","time_ms":0,"value":5}),
            "unknown_element",
        ),
        (
            json!({"op":"group.key","group":"ghost","child":"panel","prop":"opacity","time_ms":0,"value":5}),
            "unknown_element",
        ),
        (
            json!({"op":"group.key","group":"card","child":"panel","prop":"blur","time_ms":0,"value":5}),
            "invalid_value",
        ),
        (
            json!({"op":"group.child.set","group":"card","child":"panel","field":"nope","value":5}),
            "unknown_field",
        ),
        (
            json!({"op":"set","target":"group_def:card","field":"nope","value":5}),
            "unknown_field",
        ),
        (
            json!({"op":"invoke","target":"group_key:card/panel/opacity/9","action":"remove"}),
            "unknown_element",
        ),
        (
            json!({"op":"group.key.remove","group":"card","child":"panel","prop":"opacity","time_ms":123}),
            "unknown_element",
        ),
        // Shape-only tracks on text; a text child cannot take a shape's fields.
        (
            json!({"op":"group.key","group":"card","child":"title","prop":"width","time_ms":0,"value":5}),
            "not_applicable",
        ),
        (
            json!({"op":"group.child.set","group":"card","child":"title","field":"fill","value":"#000000"}),
            "unknown_field",
        ),
        // Keys out of order, through the key field editor.
        (
            json!({"op":"set","target":"group_key:card/panel/opacity/0","field":"time_ms","value":900}),
            "invalid_value",
        ),
        // A text style a child needs is not deleted; neither is a group a clip shows.
        (json!({"op":"text_style.remove","style":"title"}), "in_use"),
        (json!({"op":"group.remove","group":"card"}), "in_use"),
        (
            json!({"op":"group.child.set","group":"card","child":"title","field":"style","value":"missing"}),
            "unknown_element",
        ),
        // Instances.
        (
            json!({"op":"clip.group_text","clip":"card1","child":"panel","text":"x"}),
            "unknown_element",
        ),
        (
            json!({"op":"clip.group","clip":"card1","group":"ghost"}),
            "unknown_element",
        ),
        (
            json!({"op":"clip.add","id":"bad","track":"overlay","start":100,"duration":10,"group":{"def":"ghost"}}),
            "unknown_element",
        ),
        (
            json!({"op":"clip.add","id":"bad","track":"overlay","start":100,"duration":10,"group":{"def":"card"},"text":{"content":"x"}}),
            "invalid_request",
        ),
        (
            json!({"op":"clip.add","id":"bad","track":"overlay","start":100,"duration":10,"group":{"def":"card","texts":{"panel":"x"}}}),
            "unknown_element",
        ),
        // A duplicate group, a malformed definition.
        (
            json!({"op":"group.define","definition":{"id":"card"}}),
            "duplicate_id",
        ),
        (
            json!({"op":"group.define","definition":{"id":"x","children":[{"id":"a","kind":"star"}]}}),
            "invalid_value",
        ),
        (
            json!({"op":"text_style.set","style":{"id":"huge","size":500}}),
            "invalid_value",
        ),
    ];
    for (request, code) in requests {
        let got = refused(&mut state, request.clone());
        assert_eq!(got, code, "{request}");
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
    // A group clip is the only clip that shows a group, and only text children take a text.
    run(
        &mut state,
        json!({"op":"clip.add","id":"words","track":"overlay","start":120,"duration":30,
            "text":{"content":"Plain"}}),
    );
    assert_eq!(
        refused(
            &mut state,
            json!({"op":"clip.group","clip":"words","group":"card"})
        ),
        "not_applicable"
    );
}

#[test]
fn deleting_a_child_drops_the_texts_clips_changed_for_it() {
    let mut state = project();
    define_card(&mut state);
    run(
        &mut state,
        json!({"op":"clip.group_text","clip":"card1","child":"title","text":"Edited"}),
    );
    run(
        &mut state,
        json!({"op":"invoke","target":"group_child:card/title","action":"remove"}),
    );
    assert_eq!(layers(&mut state, 30).len(), 1);
    assert!(
        !state.to_document(None).to_string().contains("Edited"),
        "the override went with the child"
    );
    run(&mut state, json!({"op":"history.undo"}));
    assert_eq!(layers(&mut state, 30)[1]["text"]["content"], "Edited");
}

#[test]
fn groups_survive_save_and_reopen_and_older_documents_do_not_gain_them() {
    let mut state = project();
    define_card(&mut state);
    run(
        &mut state,
        json!({"op":"clip.group_text","clip":"card1","child":"title","text":"Edited"}),
    );
    run(
        &mut state,
        json!({"op":"group.repeat","group":"card","child":"panel","prop":"y","repeat":2,"mode":"pingpong"}),
    );
    let planned: Vec<Vec<Value>> = [0, 6, 12, 30]
        .iter()
        .map(|f| layers(&mut state, *f))
        .collect();
    let document = state.to_document(None);
    assert_eq!(document["schema_version"], 2);
    let mut reopened = EditorState::from_document(&document, None).unwrap();
    assert_eq!(reopened.to_document(None), document);
    let replanned: Vec<Vec<Value>> = [0, 6, 12, 30]
        .iter()
        .map(|f| layers(&mut reopened, *f))
        .collect();
    assert_eq!(planned, replanned);

    // A project that never defined a group does not gain the block or the schema.
    let plain = project().to_document(None);
    assert!(plain.get("groups").is_none());
    assert!(!plain.to_string().contains("\"group\""));
    assert_eq!(plain["schema_version"], 1);

    // A schema 1 document cannot carry groups: a v1 reader would drop them.
    let mut forged = document.clone();
    forged["schema_version"] = json!(1);
    assert!(EditorState::from_document(&forged, None).is_err());
}

#[test]
fn a_document_with_a_dangling_group_or_override_is_refused_with_the_clip_named() {
    let mut state = project();
    define_card(&mut state);
    let document = state.to_document(None);

    let missing = document
        .to_string()
        .replace("\"def\":\"card\"", "\"def\":\"ghost\"");
    let error =
        EditorState::from_document(&serde_json::from_str(&missing).unwrap(), None).unwrap_err();
    assert!(format!("{error:?}").contains("card1"), "{error:?}");

    let mut forged = document.clone();
    forged["sequences"][0]["clips"][0]["group"]["texts"] = json!({"panel": "x"});
    let error = EditorState::from_document(&forged, None).unwrap_err();
    assert!(format!("{error:?}").contains("card1"), "{error:?}");

    let mut broken = document.clone();
    broken["groups"]["definitions"][0]["children"][0]["tracks"][0]["keys"][0]["value"] = json!(500);
    assert!(EditorState::from_document(&broken, None).is_err());
}

#[test]
fn one_add_group_action_defines_the_starter_group_and_undoes_everything() {
    let mut state = EditorState::demo();
    run(
        &mut state,
        json!({"op":"project.create","name":"Groups","rate":{"num":30,"den":1},
            "width":1280,"height":720,"length":300}),
    );
    run(&mut state, json!({"op":"seek","frame":15}));
    let added = run(
        &mut state,
        json!({"op":"invoke","target":"timeline.add_group","action":"invoke"}),
    );
    assert_eq!(added["frame"], 15);
    assert_eq!(state.timeline.tracks.len(), 1);
    assert_eq!(state.groups.defs.len(), 1);
    assert_eq!(state.history.to_json()["undo_depth"], 1);
    let planned = layers(&mut state, 60);
    assert_eq!(planned.len(), 2);
    assert_eq!(planned[0]["kind"], "shape");
    assert_eq!(planned[1]["kind"], "text");

    // A second one reuses the style and defines a second group.
    run(&mut state, json!({"op":"group.add"}));
    assert_eq!(state.groups.defs.len(), 2);
    assert_eq!(state.groups.styles.len(), 1);

    run(&mut state, json!({"op":"history.undo"}));
    run(&mut state, json!({"op":"history.undo"}));
    assert!(state.timeline.tracks.is_empty());
    assert!(state.timeline.clips.is_empty());
    assert!(state.groups.is_empty());
}
