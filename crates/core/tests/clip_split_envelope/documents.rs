use super::*;

#[test]
fn undo_redo_save_and_reopen_keep_the_envelope() {
    let mut state = audio_project((30_000, 1001), 44_100, true);
    set(&mut state, "main", "fade_in", json!(20));
    set(&mut state, "main", "fade_out", json!(20));
    let whole = mix(&state);
    split(&mut state, "main", 10);
    split(&mut state, "main-split-1", 55);
    let cut_clips = state.timeline.clips.clone();
    same_bits(&whole, &mix(&state), "two cuts");
    assert!(cut_clips.iter().any(|clip| !clip.fade_window.is_empty()));

    let path = shared_dir().join("cut-fades.tessel.json");
    persist::save(&mut state, &path).unwrap();
    let document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        document["schema_version"], 5,
        "schema-4 readers must reject this document"
    );
    let reopened = persist::load(&path).unwrap();
    for clip in &cut_clips {
        assert_eq!(
            reopened.timeline.clip(clip.id).unwrap(),
            clip,
            "{}",
            clip.id
        );
    }
    same_bits(&whole, &mix(&reopened), "reopened");

    run(&mut state, json!({"op":"history.undo"}));
    run(&mut state, json!({"op":"history.undo"}));
    assert!(
        state
            .timeline
            .clips
            .iter()
            .all(|clip| clip.fade_window.is_empty())
    );
    same_bits(&whole, &mix(&state), "undone");
    run(&mut state, json!({"op":"history.redo"}));
    run(&mut state, json!({"op":"history.redo"}));
    same_bits(&whole, &mix(&state), "redone");
    assert_eq!(state.timeline.clips, cut_clips);
}

#[test]
fn old_documents_have_no_window_and_a_bad_window_is_refused_whole() {
    let mut state = visual_project((24, 1));
    set(&mut state, "t", "fade_in", json!(12));
    let plain = state.to_document(None);
    assert!(
        plain["sequences"][0]["clips"][0]
            .get("fade_window")
            .is_none()
    );
    assert_eq!(plain["schema_version"], 1);
    split(&mut state, "t", 10);
    let doc = state.to_document(None);
    let clip = doc["sequences"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .position(|clip| clip["id"] == "t-split-1")
        .unwrap();
    assert_eq!(
        doc["sequences"][0]["clips"][clip]["fade_window"],
        json!({"lead": {"frames": 4, "fade": 12.0}})
    );
    assert!(EditorState::from_document(&doc, None).is_ok());

    let mut open = visual_project((24, 1));
    let untouched = fingerprint(&open);
    let mutate = |change: &dyn Fn(&mut Value)| {
        let mut bad = doc.clone();
        change(&mut bad["sequences"][0]["clips"][clip]);
        bad
    };
    let bad_windows = [
        mutate(&|c| c["fade_window"] = json!({"lead": {"frames": 0, "fade": 12}})),
        mutate(&|c| c["fade_window"] = json!({"lead": {"frames": -3, "fade": 12}})),
        mutate(&|c| c["fade_window"] = json!({"lead": {"frames": 2.5, "fade": 12}})),
        mutate(&|c| c["fade_window"] = json!({"lead": {"frames": 3, "fade": -1}})),
        mutate(&|c| c["fade_window"] = json!({"lead": {"frames": 3}})),
        mutate(&|c| c["fade_window"] = json!({"lead": {"frames": 3, "fade": 1, "x": 0}})),
        mutate(&|c| c["fade_window"] = json!({})),
        mutate(&|c| c["fade_window"] = json!("lead")),
        mutate(&|c| c["fade_window"] = json!({"lead": {"frames": 1_000_000, "fade": 12}})),
        mutate(&|c| c["fade_window"] = json!({"middle": {"frames": 3, "fade": 12}})),
        {
            let mut v1 = doc.clone();
            v1["schema_version"] = json!(1);
            v1
        },
    ];
    for bad in bad_windows {
        let Err(error) = EditorState::from_document(&bad, None) else {
            panic!("{bad} must be refused");
        };
        assert_eq!(error.code, "invalid_document", "{bad}");
        // Opening is a candidate swap: nothing of the open project moved.
        assert_eq!(fingerprint(&open), untouched);
    }
    open.timeline.playhead = 0;

    // A window whose guard no longer matches the stored fade is not live: it is dropped on open.
    let mut stale = doc.clone();
    stale["clip_properties"]["t-split-1"]["fade_in"] = json!(7);
    let reopened = EditorState::from_document(&stale, None).unwrap();
    assert!(window_of(&reopened, "t-split-1").is_empty());
}

// ---- edits after a cut ----------------------------------------------------------------------

#[test]
fn windows_require_schema_five_but_other_projects_keep_their_old_version() {
    let mut state = visual_project((24, 1));
    assert_eq!(state.document_schema_version(), 1);
    set(&mut state, "t", "fade_in", json!(30));
    split(&mut state, "t", 20);
    let doc = state.to_document(None);
    assert_eq!(doc["schema_version"], 5);
    for version in 1..5 {
        let mut stale = doc.clone();
        stale["schema_version"] = json!(version);
        assert_eq!(
            EditorState::from_document(&stale, None).unwrap_err().code,
            "invalid_document"
        );
    }
    assert!(EditorState::from_document(&doc, None).is_ok());
}

#[test]
fn a_localized_document_keeps_the_shared_fade_windows_and_uses_schema_five() {
    let mut state = visual_project((24, 1));
    set(&mut state, "t", "fade_in", json!(12));
    run(
        &mut state,
        json!({"op":"localization.add","locale":"pt-br"}),
    );
    run(&mut state, json!({"op":"localization.add","locale":"en"}));
    assert_eq!(state.document_schema_version(), 4);

    split(&mut state, "t", 10);
    let base = state.to_document(None);
    run(
        &mut state,
        json!({"op":"localization.select","locale":"en"}),
    );
    let selected = state.to_document(None);
    assert_eq!(selected["schema_version"], 5);
    assert_eq!(selected["sequences"], base["sequences"]);
    assert_eq!(selected["localization"], base["localization"]);

    let reopened = EditorState::from_document(&selected, None).unwrap();
    assert!(!window_of(&reopened, "t-split-1").is_empty());
    assert_eq!(reopened.language_codes(), ["pt-br", "en"]);
}

#[test]
fn trim_at_the_timeline_limit_is_atomic_and_leaves_a_reopenable_document() {
    let mut state = visual_project((24, 1));
    state.timeline.length = 1_000_000;
    let clip = state
        .timeline
        .clips
        .iter_mut()
        .find(|clip| clip.id == "t")
        .unwrap();
    clip.start = 0;
    clip.duration = 999_000;
    set(&mut state, "t", "fade_in", json!(48));
    set(&mut state, "t", "fade_out", json!(48));
    split(&mut state, "t", 10);
    split(&mut state, "t-split-1", 998_990);
    state
        .execute(Command::MoveClip {
            clip: "t-split-1".into(),
            track: Some("ut".into()),
            frame: Some(1020),
            snap: Some(false),
        })
        .unwrap();
    let before = fingerprint(&state);
    assert_eq!(
        state
            .timeline
            .plan_trim("t-split-1", TrimEdge::Start, 0)
            .unwrap_err()
            .code,
        "invalid_time"
    );
    let error = state
        .execute(Command::TrimClip {
            clip: "t-split-1".into(),
            edge: TrimEdge::Start,
            frame: 0,
        })
        .unwrap_err();
    assert_eq!(error.code, "invalid_time");
    assert_eq!(fingerprint(&state), before);
    assert!(EditorState::from_document(&state.to_document(None), None).is_ok());
}
