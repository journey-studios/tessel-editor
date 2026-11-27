use serde_json::{Value, json};
use tessel_editor_core::localization::{Edit, SequencePatch};
use tessel_editor_core::{Command, EditorState, headless};

fn run(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn invoke(state: &mut EditorState, edit: Edit) -> Value {
    state.execute(Command::Localization(edit)).unwrap()
}

fn project() -> EditorState {
    let mut state = EditorState::demo();
    for request in [
        json!({"op":"project.create","name":"Adventure","length":60}),
        json!({"op":"track.add","id":"titles","kind":"video","name":"Titles"}),
        json!({"op":"track.add","id":"voice","kind":"audio","name":"Voice"}),
        json!({"op":"clip.add","id":"name","track":"titles","start":0,"duration":25,
            "text":{"content":"Tocha","size":5,"color":"#ffffff"}}),
        json!({"op":"clip.add","id":"end","track":"titles","start":25,"duration":25,
            "text":{"content":"Fim","size":5,"color":"#ffffff"}}),
        json!({"op":"localization.add","locale":"pt-BR"}),
        json!({"op":"localization.add","locale":"en"}),
        json!({"op":"localization.add","locale":"es"}),
    ] {
        run(&mut state, request);
    }
    for id in ["pt", "en"] {
        add_audio_asset(&mut state, id);
    }
    run(
        &mut state,
        json!({"op":"clip.add","id":"voice-clip","track":"voice",
        "asset":"pt","start":0,"duration":50}),
    );
    state
}

fn add_audio_asset(state: &mut EditorState, id: &str) {
    use tessel_editor_core::assets::{Asset, MediaKind, Source};
    state
        .execute(Command::AddAsset {
            id: Some(id.into()),
            name: id.into(),
            kind: MediaKind::Audio,
            asset: Box::new(Asset {
                source: Source::Inline,
                meta: Default::default(),
                identity: Default::default(),
                decoded: None,
            }),
            folder: None,
        })
        .unwrap();
}

fn set(state: &mut EditorState, locale: &str, overrides: Value) {
    let sequence = state.timeline.id.to_string();
    invoke(
        state,
        Edit::Set {
            locale: locale.into(),
            sequence,
            overrides: SequencePatch::from_json(&overrides).unwrap(),
        },
    );
}

fn review(state: &EditorState, code: &str) -> Value {
    let report = state.localization_status();
    report["languages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|locale| locale["locale"] == code)
        .unwrap()
        .clone()
}

fn title(state: &EditorState, clip: &str) -> String {
    state
        .timeline
        .clip(clip)
        .unwrap()
        .text
        .as_ref()
        .unwrap()
        .content
        .clone()
}

#[test]
fn languages_share_structure_but_have_separate_text_sound_timing_and_duration() {
    let mut state = project();
    set(
        &mut state,
        "en",
        json!({"length":65,"clips":{
            "name":{"text":"Torch","duration":30},
            "end":{"text":"End","start":30},
            "voice-clip":{"audio":"en","duration":60,"start":5}
        }}),
    );
    set(
        &mut state,
        "es",
        json!({"clips":{"name":{"text":"Antorcha"}}}),
    );
    assert_eq!(title(&state, "name"), "Tocha");

    run(
        &mut state,
        json!({"op":"set","target":"timeline.language","field":"value","value":"en"}),
    );
    assert_eq!(title(&state, "name"), "Torch");
    assert_eq!(state.timeline.clip("end").unwrap().start, 30);
    assert_eq!(
        state
            .timeline
            .clip("voice-clip")
            .unwrap()
            .media
            .unwrap()
            .as_str(),
        "en"
    );
    assert_eq!(state.timeline.length, 65);
    let document = state.to_document(None);
    assert_eq!(document["schema_version"], 4);
    assert_eq!(document["sequences"][0]["length"], 60);
    assert_eq!(
        document["sequences"][0]["clips"][0]["text"]["content"],
        "Tocha"
    );
    assert_eq!(document["sequences"].as_array().unwrap().len(), 1);

    let mut reopened = EditorState::from_document(&document, None).unwrap();
    assert_eq!(reopened.active_language(), Some("pt-br"));
    invoke(&mut reopened, Edit::Select("es".into()));
    assert_eq!(title(&reopened, "name"), "Antorcha");
    assert_eq!(title(&reopened, "end"), "Fim");
    assert_eq!(reopened.timeline.length, 60);
    assert_eq!(
        reopened
            .timeline
            .clip("voice-clip")
            .unwrap()
            .media
            .unwrap()
            .as_str(),
        "pt"
    );
    assert_eq!(reopened.to_document(None), document);
}

#[test]
fn reviews_track_content_dependencies_and_undo_restores_approval() {
    let mut state = project();
    assert_eq!(review(&state, "en")["missing"].as_array().unwrap().len(), 3);
    set(
        &mut state,
        "en",
        json!({"clips":{
            "name":{"text":"Torch"}, "end":{"text":null}, "voice-clip":{"audio":"en"}
        }}),
    );
    assert_eq!(review(&state, "en")["status"], "draft");
    invoke(&mut state, Edit::Approve("en".into()));
    assert_eq!(review(&state, "en")["status"], "approved");
    set(
        &mut state,
        "es",
        json!({"clips":{"name":{"text":"Antorcha"}}}),
    );
    add_audio_asset(&mut state, "es");
    assert_eq!(review(&state, "en")["status"], "approved");
    invoke(&mut state, Edit::Select("en".into()));
    run(
        &mut state,
        json!({"op":"set","target":"clip_text:name","field":"content","value":"Third torch"}),
    );
    assert_eq!(title(&state, "name"), "Third torch");
    assert_eq!(review(&state, "en")["status"], "stale");
    state.undo().unwrap();
    assert_eq!(state.active_language(), Some("en"));
    assert_eq!(title(&state, "name"), "Torch");
    assert_eq!(review(&state, "en")["status"], "approved");
    invoke(&mut state, Edit::Select("pt-BR".into()));
    run(
        &mut state,
        json!({"op":"set","target":"clip_text:name","field":"content","value":"Terceira tocha"}),
    );
    assert_eq!(review(&state, "en")["status"], "stale");
    assert_eq!(review(&state, "es")["status"], "incomplete");
}

#[test]
fn localized_edits_undo_without_replacing_base_and_invalid_updates_are_atomic() {
    let mut state = project();
    invoke(&mut state, Edit::Select("en".into()));
    let before = state.to_document(None);
    for request in [
        json!({"op":"localization.set","locale":"en","overrides":{"clips":{"name":{"track":"voice"}}}}),
        json!({"op":"localization.set","locale":"en","overrides":{"clips":{"name":{"duration":30}}}}),
        json!({"op":"localization.set","locale":"en","overrides":{"clips":{"voice-clip":{"audio":"no-such-audio"}}}}),
        json!({"op":"localization.set","locale":"en","overrides":{"clips":{"name":{"start":i64::MAX}}}}),
        json!({"op":"localization.approve","locale":"en"}),
        json!({"op":"track.add","id":"other","kind":"video","name":"Blocked"}),
    ] {
        assert!(
            headless::execute(&mut state, &request).is_err(),
            "{request}"
        );
        assert_eq!(state.to_document(None), before);
        assert_eq!(state.active_language(), Some("en"));
    }
    run(
        &mut state,
        json!({"op":"move","target":"clip:end","args":{"track":"titles","frame":27,"snap":false}}),
    );
    assert_eq!(state.timeline.clip("end").unwrap().start, 27);
    state.undo().unwrap();
    assert_eq!(state.timeline.clip("end").unwrap().start, 25);
    state.redo().unwrap();
    assert_eq!(state.timeline.clip("end").unwrap().start, 27);
    invoke(&mut state, Edit::Select("pt-br".into()));
    assert_eq!(state.timeline.clip("end").unwrap().start, 25);
    assert_eq!(state.timeline.tracks.len(), 2);
}

#[test]
fn a_base_edit_can_create_a_detectable_conflict_that_is_repairable_after_reopen() {
    let mut state = project();
    set(
        &mut state,
        "en",
        json!({"clips":{"end":{"start":35,"duration":25}}}),
    );
    run(&mut state, json!({"op":"sequence.length","frames":55}));
    assert_eq!(review(&state, "en")["status"], "conflict");
    let doc = state.to_document(None);
    let mut reopened = EditorState::from_document(&doc, None).unwrap();
    assert!(
        reopened
            .execute(Command::Localization(Edit::Select("en".into())))
            .is_err()
    );
    let saved = run(
        &mut reopened,
        json!({"op":"localization.get","locale":"en"}),
    );
    assert_eq!(saved["overrides"]["clips"]["end"]["start"], 35);
    set(
        &mut reopened,
        "en",
        json!({"clips":{"end":{"start":30,"duration":25}}}),
    );
    invoke(&mut reopened, Edit::Select("en".into()));
    assert_eq!(reopened.timeline.clip("end").unwrap().start, 30);
}

#[test]
fn an_inspector_group_edit_and_caption_retiming_are_specific_to_the_language() {
    let mut state = project();
    for request in [
        json!({"op":"track.add","id":"ui","kind":"video","name":"Cards"}),
        json!({"op":"track.add","id":"subs","kind":"video","name":"Captions"}),
        json!({"op":"text_style.set","style":{"id":"card","size":8,"color":"#ffffff","font":"sans"}}),
        json!({"op":"group.define","definition":{"id":"card","children":[
            {"id":"title","kind":"text","style":"card","content":"Fragmentos"}]}}),
        json!({"op":"clip.add","id":"card","track":"ui","start":0,"duration":30,"group":{"def":"card"}}),
        json!({"op":"clip.add","id":"captions","track":"subs","start":0,"duration":40,
            "subtitles":{"cues":[{"start_ms":0,"end_ms":500,"text":"Tocha"}]}}),
        json!({"op":"localization.select","locale":"en"}),
        json!({"op":"set","target":"group_text:card/title","field":"text","value":"Fragments"}),
        json!({"op":"subtitles.cue.set","clip":"captions","index":0,"field":"text","value":"Torch"}),
        json!({"op":"subtitles.cue.set","clip":"captions","index":0,"field":"end_ms","value":700}),
    ] {
        run(&mut state, request);
    }
    let before = state.to_document(None);
    let oversized = json!({"op":"localization.set","locale":"en","overrides":{
        "clips":{"card":{"group_texts":{"title":"x".repeat(257)}}}
    }});
    assert!(headless::execute(&mut state, &oversized).is_err());
    assert_eq!(state.to_document(None), before);
    let patch = run(&mut state, json!({"op":"localization.get","locale":"en"}));
    assert_eq!(
        patch["overrides"]["clips"]["card"]["group_texts"]["title"],
        "Fragments"
    );
    assert_eq!(
        patch["overrides"]["clips"]["captions"]["cues"][0]["end_ms"],
        700
    );
    let plan = run(&mut state, json!({"op":"plan","frame":2}));
    assert!(
        plan["layers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|layer| layer["text"]["content"] == "Fragments")
    );
    assert!(
        plan["layers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|layer| layer["text"]["content"] == "Torch")
    );
    invoke(&mut state, Edit::Select("pt-br".into()));
    let plan = run(&mut state, json!({"op":"plan","frame":2}));
    assert!(
        plan["layers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|layer| layer["text"]["content"] == "Fragmentos")
    );
}

#[test]
fn visual_and_mix_adjustments_project_without_mutating_the_shared_document() {
    let mut state = project();
    for request in [
        json!({"op":"clip.property","clip":"voice-clip","property":"volume","value":-5}),
        json!({"op":"sequence.add","id":"plates","name":"Plates"}),
        json!({"op":"localization.set","locale":"en","sequence":"main","overrides":{"clips":{
            "name":{"text":"Torch","title_style":{"size":7},"properties":{"position_x":10}},
            "end":{"text":null},
            "voice-clip":{"audio":"en","properties":{"volume":-9}}
        }}}),
        json!({"op":"localization.select","locale":"en"}),
    ] {
        run(&mut state, request);
    }
    let full = state.to_document(None);
    assert_eq!(full["clip_properties"]["voice-clip"]["volume"], -5.0);
    assert!(full["clip_properties"].get("name").is_none());
    let resolved = state.resolve_locale("en", Some("main")).unwrap();
    let view = &resolved["document"];
    assert_eq!(view["clip_properties"]["voice-clip"]["volume"], -9.0);
    assert_eq!(view["clip_properties"]["name"]["position_x"], 10.0);
    assert_eq!(view["sequences"][0]["clips"][0]["text"]["size"], 7.0);
    assert_eq!(
        state
            .timeline
            .clip("name")
            .unwrap()
            .text
            .as_ref()
            .unwrap()
            .size,
        7.0
    );
    let plate = state.resolve_locale("en", Some("plates")).unwrap();
    assert_eq!(plate["document"]["clip_properties"], json!({}));
    EditorState::from_document(&plate["document"], None).unwrap();
    assert_eq!(state.to_document(None), full);
    assert_eq!(state.active_language(), Some("en"));

    let refused = json!({"op":"localization.set","locale":"en","sequence":"main",
        "overrides":{"clips":{"voice-clip":{"properties":{"position_x":35}}}}
    });
    assert!(headless::execute(&mut state, &refused).is_err());
    assert_eq!(state.to_document(None), full);
    assert_eq!(state.resolve_locale("en", None).unwrap(), resolved);

    run(
        &mut state,
        json!({"op":"sequence.activate","sequence":"plates"}),
    );
    run(
        &mut state,
        json!({"op":"sequence.activate","sequence":"main"}),
    );
    assert_eq!(state.resolve_locale("en", None).unwrap(), resolved);
    invoke(&mut state, Edit::Select("pt-br".into()));
    assert_eq!(title(&state, "name"), "Tocha");
}
