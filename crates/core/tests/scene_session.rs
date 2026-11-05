//! F039 session safety (R3: BL066–BL069): who a gesture ID belongs to, what the shared summary
//! shows of the scene tools, what the byte budget does to a gesture, and what undo/redo do to the
//! selection and to an open gesture.
//!
//! Three surfaces, kept apart on purpose:
//!
//! - **library**: `EditorState::execute` / `headless::execute` in this process;
//! - **public JSONL executable**: the real `tessel-headless` binary, one request per line with
//!   the answer read before the next line is written (a gesture ID is only known after `begin`);
//! - **not covered**: the desktop socket and any UI. They share these rules (the core), but this
//!   file proves nothing about the window.
//!
//! WHY the budget test allocates ~128 MiB: the budget (`MAX_SCENE_BYTES`) is a constant of the
//! book, and a scene's size is the JSON of the bundle, so the only honest way to meet it through
//! the public API is a bundle that really is that big. The commit-time refusal, which the public
//! API cannot reach (see the test), is covered next to the code (`scene_project/interaction.rs`).

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command as Process, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::scene_project::{MAX_SCENE_BYTES, SceneCommand, SceneSource};
use tessel_editor_core::{Command, CommandError, EditorState, headless};

const MINIMAL: &str = "valid-minimal-continuous";
const FIGURE: &str = "scene/scene-a/node/figure";
const FIGURE_POSITION: &str = "scene/scene-a/node/figure/position";

fn bundle(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/migration/fixtures/bundles")
        .join(format!("{name}.bundle.json"));
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn run(state: &mut EditorState, request: Value) -> Result<Value, CommandError> {
    headless::execute(state, &request)
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    run(state, request.clone()).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn err(state: &mut EditorState, request: Value) -> CommandError {
    match run(state, request.clone()) {
        Ok(answer) => panic!("{request} should have failed, got {answer}"),
        Err(error) => error,
    }
}

fn project_with_street() -> EditorState {
    let mut state = EditorState::demo();
    ok(&mut state, json!({"op":"project.create","name":"Session"}));
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle(MINIMAL)}),
    );
    state
}

/// Everything a refused, read-only or cancelled command must leave exactly as it was.
fn observable(state: &EditorState) -> (Value, Value, u64) {
    (
        state.to_document(None),
        state.history.to_json(),
        state.project.revision,
    )
}

fn undo_depth(state: &EditorState) -> u64 {
    state.history.to_json()["undo_depth"].as_u64().unwrap()
}

fn move_figure(x: f64) -> Value {
    json!({"set": FIGURE_POSITION, "value": {"x": x, "y": 0, "z": 0}})
}

fn begin(state: &mut EditorState) -> String {
    let answer = ok(
        state,
        json!({"op":"scene.gesture.begin","scene":"street","targets":[FIGURE]}),
    );
    answer["gesture"].as_str().unwrap().to_string()
}

fn update(state: &mut EditorState, gesture: &str, ops: Vec<Value>) -> Value {
    ok(
        state,
        json!({"op":"scene.gesture.update","gesture":gesture,"ops":ops}),
    )
}

fn figure_x(state: &mut EditorState) -> f64 {
    ok(
        state,
        json!({"op":"scene.get","scene":"street","property":FIGURE_POSITION}),
    )["value"]["x"]
        .as_f64()
        .unwrap()
}

/// A private directory under the system temp dir, removed by `Drop` (te-r3 prefix: this front's).
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("te-r3-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---- BL066: an ID belongs to the session that issued it ------------------------------------

#[test]
fn an_id_kept_from_the_previous_project_cannot_touch_the_next_gesture() {
    let mut state = project_with_street();
    let old = begin(&mut state);
    update(&mut state, &old, vec![move_figure(2.0)]);

    // The user (or a script) starts another project in the same session and begins again.
    ok(&mut state, json!({"op":"project.create","name":"Next"}));
    ok(
        &mut state,
        json!({"op":"scene.attach","id":"street","bundle":bundle(MINIMAL)}),
    );
    let fresh = begin(&mut state);
    assert_ne!(old, fresh, "the new project's first gesture has its own ID");
    let candidate = update(&mut state, &fresh, vec![move_figure(3.0)]);
    assert_eq!(candidate["updates"], 1);

    // Same scene ID, same attach order, same revision: only the ID tells the two gestures apart.
    let before = observable(&state);
    let status_before = ok(&mut state, json!({"op":"scene.gesture.status"}));
    for request in [
        json!({"op":"scene.gesture.update","gesture":old,"ops":[move_figure(9.0)]}),
        json!({"op":"scene.gesture.commit","gesture":old}),
        json!({"op":"scene.gesture.cancel","gesture":old}),
        json!({"op":"scene.evaluate","scene":"street","frame":10,"gesture":old}),
        json!({"op":"scene.plan","scene":"street","frame":10,"gesture":old}),
    ] {
        let error = err(&mut state, request.clone());
        assert_eq!(error.code, "unknown_gesture", "{request}");
        let details = error.details.as_ref().unwrap();
        assert_eq!(details["reason"], "other_session", "{request}");
        assert_eq!(details["current"], json!(fresh), "{request}");
    }
    assert_eq!(
        observable(&state),
        before,
        "refused requests change nothing"
    );
    assert_eq!(
        ok(&mut state, json!({"op":"scene.gesture.status"})),
        status_before,
        "the new gesture kept its candidate and update count"
    );
    let live = state.scene_interaction.gesture().unwrap();
    assert_eq!(live.id(), fresh);
    assert_eq!(live.updates(), 1);

    // The new gesture is still fully usable and commits its own value, not the old one's.
    ok(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":fresh}),
    );
    assert_eq!(figure_x(&mut state), 3.0);
}

#[test]
fn reopening_the_same_saved_project_starts_a_new_session() {
    let dir = TempDir::new("reopen");
    let document = dir.0.join("session.tessel.json");
    let mut state = project_with_street();
    ok(&mut state, json!({"op":"project.save","path":document}));
    let old = begin(&mut state);

    ok(&mut state, json!({"op":"project.open","path":document}));
    assert!(
        state.scene_interaction.gesture().is_none(),
        "an opened project starts without gesture or selection"
    );
    let fresh = begin(&mut state);
    assert_ne!(
        old, fresh,
        "same project file, same scene, still another ID"
    );
    update(&mut state, &fresh, vec![move_figure(2.0)]);

    let before = observable(&state);
    let error = err(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":old}),
    );
    assert_eq!(error.code, "unknown_gesture");
    assert_eq!(error.details.as_ref().unwrap()["reason"], "other_session");
    assert_eq!(observable(&state), before);
    assert!(state.scene_interaction.gesture().unwrap().has_candidate());
}

#[test]
fn a_gesture_id_is_issued_once_across_many_sessions() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..40 {
        // Every iteration is what `project.create` / `project.open` build: a fresh state.
        let mut state = project_with_street();
        for _ in 0..3 {
            let id = begin(&mut state);
            assert!(seen.insert(id.clone()), "`{id}` was issued twice");
            ok(
                &mut state,
                json!({"op":"scene.gesture.cancel","gesture":id}),
            );
        }
    }
}

#[test]
fn an_id_of_this_session_that_is_not_the_open_one_is_not_blamed_on_the_session() {
    let mut state = project_with_street();
    let first = begin(&mut state);
    ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":first}),
    );
    let second = begin(&mut state);

    let cancelled = err(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":first}),
    );
    assert_eq!(cancelled.code, "unknown_gesture");
    assert_eq!(cancelled.details.as_ref().unwrap()["reason"], "not_current");
    let typo = err(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":"gesture-99"}),
    );
    assert_eq!(typo.details.as_ref().unwrap()["reason"], "not_current");

    ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":second}),
    );
    let none = err(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":second}),
    );
    assert_eq!(
        none.code, "no_gesture",
        "nothing open: the code is unchanged"
    );
}

// ---- BL068: selection and gesture in the shared summary ------------------------------------

#[test]
fn the_summary_has_no_scene_tool_block_until_the_tools_are_used() {
    let mut state = project_with_street();
    assert!(state.summary().get("scene_interaction").is_none());

    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":[FIGURE]}),
    );
    ok(&mut state, json!({"op":"scene.select","mode":"clear"}));
    assert!(
        state.summary().get("scene_interaction").is_none(),
        "an emptied selection leaves the summary as it was"
    );
}

#[test]
fn the_summary_shows_selection_and_gesture_and_asking_for_it_changes_nothing() {
    let mut state = project_with_street();
    let depth = undo_depth(&state);
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":[FIGURE]}),
    );
    let gesture = begin(&mut state);
    update(&mut state, &gesture, vec![move_figure(2.0)]);

    let summary = ok(&mut state, json!({"op":"status"}));
    let block = &summary["scene_interaction"];
    assert_eq!(block["selection"]["count"], 1);
    assert_eq!(block["selection"]["truncated"], false);
    assert_eq!(block["selection"]["entities"][0]["path"], FIGURE);
    assert_eq!(block["selection"]["entities"][0]["scene"], "street");
    assert!(block["selection"]["entities"][0].get("stale").is_none());
    assert_eq!(block["gesture"]["id"], json!(gesture));
    assert_eq!(block["gesture"]["state"], "open");
    assert_eq!(block["gesture"]["has_candidate"], true);
    assert_eq!(block["gesture"]["would_change"], true);

    assert_eq!(
        undo_depth(&state),
        depth,
        "selecting, beginning and updating are not history"
    );
    let before = observable(&state);
    let _ = state.summary();
    let _ = state.summary();
    assert_eq!(observable(&state), before);
    assert_eq!(state.summary(), summary);
}

#[test]
fn the_summary_reflects_an_invalidated_gesture() {
    let mut state = project_with_street();
    let gesture = begin(&mut state);
    update(&mut state, &gesture, vec![move_figure(2.0)]);
    // Another edit of the scene moves it away from what the gesture started from.
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[move_figure(4.0)]}),
    );

    let block = state.summary()["scene_interaction"].clone();
    assert_eq!(block["gesture"]["state"], "invalidated");
    assert_eq!(block["gesture"]["invalidation"], "scene_changed");
    assert_eq!(
        block["gesture"]["id"],
        json!(gesture),
        "still the gesture, so a UI can cancel it"
    );

    ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":gesture}),
    );
    assert!(state.summary().get("scene_interaction").is_none());
}

#[test]
fn the_summary_lists_a_bounded_selection_and_counts_all_of_it() {
    use tessel_editor_core::scene_interaction::SUMMARY_LIMIT;
    let mut state = project_with_street();
    let tree = ok(&mut state, json!({"op":"scene.tree","scene":"street"}));
    let mut entities = Vec::new();
    let mut stack = vec![&tree["tree"]];
    while let Some(entry) = stack.pop() {
        if let Some(path) = entry["path"].as_str().filter(|_| entry["kind"] != "bundle") {
            entities.push(path.to_string());
        }
        stack.extend(entry["children"].as_array().into_iter().flatten());
    }
    assert!(!entities.is_empty());
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":entities}),
    );
    let selection = &state.summary()["scene_interaction"]["selection"];
    assert_eq!(selection["count"], json!(entities.len()));
    assert!(
        selection["entities"].as_array().unwrap().len() <= SUMMARY_LIMIT,
        "the list is bounded"
    );
    assert_eq!(
        selection["truncated"],
        json!(entities.len() > SUMMARY_LIMIT)
    );
}

// ---- BL069: what undo/redo do to the selection and to an open gesture ----------------------

#[test]
fn undo_prunes_a_selection_it_orphans_at_once_and_redo_never_brings_it_back() {
    let mut state = project_with_street();
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":[FIGURE]}),
    );
    // The attach is the only entry: undoing it takes the scene, and with it the entity, away.
    let undone = ok(&mut state, json!({"op":"history.undo"}));
    assert_eq!(undone["label"], "Attach scene");
    assert_eq!(undone["selection_dropped"][0]["path"], FIGURE);
    assert_eq!(undone["selection_dropped"][0]["reason"], "scene_detached");
    assert!(
        state.scene_interaction.selection().is_empty(),
        "pruned by the undo itself, without anyone asking for the selection"
    );
    assert!(
        state.summary()["scene_interaction"]["selection"]["dropped"][0]["reason"]
            .as_str()
            .is_some(),
        "the summary says why it left"
    );

    // Redo puts the scene back. The selection is presentation: it is not restored.
    let redone = ok(&mut state, json!({"op":"history.redo"}));
    assert!(redone.get("selection_dropped").is_none());
    assert!(state.scene_interaction.selection().is_empty());
    let selection = ok(&mut state, json!({"op":"scene.selection"}));
    assert_eq!(selection["count"], 0);
    assert_eq!(
        selection["dropped"][0]["reason"], "scene_detached",
        "a client that missed the undo still learns why the entity left"
    );
}

#[test]
fn the_outcome_does_not_depend_on_whether_someone_queried_the_selection_between() {
    // Before BL069 the prune was lazy, so this diverged: asked in between, the entity was gone for
    // good; not asked, a redo made it selected again.
    let asked = {
        let mut state = project_with_street();
        ok(
            &mut state,
            json!({"op":"scene.select","scene":"street","entities":[FIGURE]}),
        );
        ok(&mut state, json!({"op":"history.undo"}));
        ok(&mut state, json!({"op":"scene.selection"}));
        ok(&mut state, json!({"op":"history.redo"}));
        ok(&mut state, json!({"op":"scene.selection"}))["entities"].clone()
    };
    let not_asked = {
        let mut state = project_with_street();
        ok(
            &mut state,
            json!({"op":"scene.select","scene":"street","entities":[FIGURE]}),
        );
        ok(&mut state, json!({"op":"history.undo"}));
        ok(&mut state, json!({"op":"history.redo"}));
        ok(&mut state, json!({"op":"scene.selection"}))["entities"].clone()
    };
    assert_eq!(asked, not_asked);
    assert_eq!(asked, json!([]));
}

#[test]
fn an_undo_that_keeps_the_entity_keeps_the_selection_and_says_nothing() {
    let mut state = project_with_street();
    ok(
        &mut state,
        json!({"op":"scene.select","scene":"street","entities":[FIGURE]}),
    );
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[move_figure(2.0)]}),
    );
    let undone = ok(&mut state, json!({"op":"history.undo"}));
    assert!(undone.get("selection_dropped").is_none());
    let redone = ok(&mut state, json!({"op":"history.redo"}));
    assert!(redone.get("selection_dropped").is_none());
    let selection = ok(&mut state, json!({"op":"scene.selection"}));
    assert_eq!(selection["entities"][0]["path"], FIGURE);
    assert_eq!(selection["dropped"], json!([]));
}

#[test]
fn a_gesture_stays_dead_after_undo_and_redo_and_can_still_be_cancelled() {
    let mut state = project_with_street();
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[move_figure(1.0)]}),
    );
    let gesture = begin(&mut state);
    update(&mut state, &gesture, vec![move_figure(2.0)]);
    ok(&mut state, json!({"op":"history.undo"}));
    ok(&mut state, json!({"op":"history.redo"}));

    // The same scene content is back, but not the same scene: the gesture does not revive.
    let before = observable(&state);
    let error = err(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":gesture}),
    );
    assert_eq!(error.code, "gesture_invalidated");
    assert_eq!(observable(&state), before);
    assert_eq!(figure_x(&mut state), 1.0, "nothing of the candidate leaked");
    assert_eq!(
        state.summary()["scene_interaction"]["gesture"]["state"],
        "invalidated"
    );
    let cancelled = ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":gesture}),
    );
    assert_eq!(cancelled["was_invalidated"], true);
}

// ---- BL067: the scene byte budget and a gesture --------------------------------------------

/// Bytes the compact JSON of the scene grows by when `ops` are applied: measured, not assumed,
/// because an edit rewrites a whole vector (`0` may become `0.0`).
fn growth_of(ops: &Value) -> usize {
    let mut state = project_with_street();
    let size = |state: &EditorState| {
        serde_json::to_vec(state.scenes.get("street").unwrap().scene.document())
            .unwrap()
            .len()
    };
    let before = size(&state);
    ok(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":ops}),
    );
    size(&state).saturating_sub(before)
}

#[test]
fn an_update_over_the_scene_budget_is_refused_and_the_gesture_keeps_its_last_good_candidate() {
    let good_op = move_figure(1.0);
    let too_big = json!({"set": FIGURE_POSITION,
        "value": {"x": 1234567.123456789_f64, "y": 0, "z": 0}});
    let good_growth = growth_of(&json!([good_op.clone()]));
    let big_growth = growth_of(&json!([too_big.clone()]));
    assert!(
        big_growth > good_growth,
        "the fixture needs an edit that grows more than the good one ({big_growth} vs {good_growth})"
    );

    // The budget is shared by every scene of the project, and a scene's size is cached once it
    // is attached: a second scene ("filler") takes everything but what the good edit needs, so
    // the 128 MiB is serialized once (at its attach) and not on every edit of `street`. Unknown
    // fields survive import byte for byte, so a padding string sets the size exactly.
    let street_len = serde_json::to_vec(&bundle(MINIMAL)).unwrap().len();
    let mut filler = bundle(MINIMAL);
    filler["x-pad"] = json!("");
    let filler_base = serde_json::to_vec(&filler).unwrap().len();
    filler["x-pad"] = json!("a".repeat(MAX_SCENE_BYTES - good_growth - street_len - filler_base));

    let mut state = EditorState::demo();
    ok(&mut state, json!({"op":"project.create","name":"Budget"}));
    for (id, source) in [("filler", filler), ("street", bundle(MINIMAL))] {
        state
            .execute(Command::Scene(SceneCommand::Attach {
                id: Some(id.to_string()),
                name: None,
                source: SceneSource::Bundle(source),
            }))
            .unwrap();
    }

    let gesture = begin(&mut state);
    // Fits the budget to the byte.
    let good = update(&mut state, &gesture, vec![move_figure(1.0)]);
    assert_eq!(good["would_change"], true);

    // Not `observable`: its document clone would copy the 128 MiB bundle on every call.
    let street = state.scenes.get("street").unwrap().scene.clone();
    let before = (state.history.to_json(), state.project.revision);
    // Grows the bundle past what is left of the budget.
    let error = err(
        &mut state,
        json!({"op":"scene.gesture.update","gesture":gesture,"ops":[too_big.clone()]}),
    );
    assert_eq!(error.code, "scenes_too_large");
    assert_eq!(error.target.as_deref(), Some("street"));
    assert_eq!(
        (state.history.to_json(), state.project.revision),
        before,
        "history and revision stay"
    );
    assert!(
        std::sync::Arc::ptr_eq(&street, &state.scenes.get("street").unwrap().scene),
        "the scene is the very same one"
    );
    let open = state.scene_interaction.gesture().unwrap();
    assert_eq!(open.updates(), 1, "the refused update was not counted");
    assert!(
        open.has_candidate(),
        "the last good candidate is still there"
    );

    // The same oversized edit is refused as `scene.edit` too: one rule, one error.
    let direct = err(
        &mut state,
        json!({"op":"scene.edit","scene":"street","ops":[too_big]}),
    );
    assert_eq!(direct.code, "scenes_too_large");

    // And the good candidate still commits as one undo entry.
    let depth = undo_depth(&state);
    ok(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":gesture}),
    );
    assert_eq!(undo_depth(&state), depth + 1);
    assert_eq!(figure_x(&mut state), 1.0);
}

// ---- lib: no-op, cancel and rejection leave no trace ---------------------------------------

#[test]
fn a_noop_commit_a_cancel_and_a_rejected_update_leave_history_and_scene_untouched() {
    let mut state = project_with_street();
    let before = observable(&state);

    // No-op: the same value the scene already has.
    let gesture = begin(&mut state);
    let same = update(&mut state, &gesture, vec![move_figure(0.5)]);
    assert_eq!(same["would_change"], false);
    let noop = ok(
        &mut state,
        json!({"op":"scene.gesture.commit","gesture":gesture}),
    );
    assert_eq!(noop["noop"], true);
    assert_eq!(noop["document_changed"], false);
    assert_eq!(observable(&state), before);

    // Rejected update (unknown node): nothing changes and the previous candidate survives.
    let gesture = begin(&mut state);
    update(&mut state, &gesture, vec![move_figure(2.0)]);
    let ghost = json!({"set":"scene/scene-a/node/ghost/position","value":{"x":1,"y":1,"z":1}});
    let error = err(
        &mut state,
        json!({"op":"scene.gesture.update","gesture":gesture,"ops":[ghost]}),
    );
    assert_eq!(error.code, "gesture_target_mismatch");
    assert_eq!(observable(&state), before);
    assert_eq!(state.scene_interaction.gesture().unwrap().updates(), 1);

    // Cancel: the candidate goes, the document never saw it.
    ok(
        &mut state,
        json!({"op":"scene.gesture.cancel","gesture":gesture}),
    );
    assert!(state.scene_interaction.gesture().is_none());
    assert_eq!(observable(&state), before);
    assert_eq!(figure_x(&mut state), 0.5);
}

// ---- the public JSONL executable ------------------------------------------------------------

/// One `tessel-headless` process, driven line by line: the answer is read before the next request
/// is written, because a gesture ID only exists after `begin`.
struct Headless {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Headless {
    fn start() -> Self {
        let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
            .arg("--empty")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn ask(&mut self, request: Value) -> Value {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|error| panic!("{request} -> `{line}`: {error}"))
    }

    fn ok(&mut self, request: Value) -> Value {
        let answer = self.ask(request.clone());
        assert_eq!(answer["ok"], true, "{request} -> {answer}");
        answer["result"].clone()
    }

    fn err(&mut self, request: Value) -> Value {
        let answer = self.ask(request.clone());
        assert_eq!(answer["ok"], false, "{request} -> {answer}");
        answer["error"].clone()
    }

    fn finish(mut self) {
        drop(self.stdin.take());
        let status = self.child.wait().unwrap();
        // The session ended on EOF; a refused request earlier makes the exit code 1.
        assert!(status.code().is_some());
    }
}

#[test]
fn the_executable_keeps_gesture_ids_apart_across_a_project_swap_undo_redo_noop_and_cancel() {
    let mut cli = Headless::start();
    let attach = json!({"op":"scene.attach","id":"street","bundle":bundle(MINIMAL)});
    cli.ok(attach.clone());
    let begin_street = |cli: &mut Headless| {
        cli.ok(json!({"op":"scene.gesture.begin","scene":"street","targets":[FIGURE]}))["gesture"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let position = |cli: &mut Headless| {
        cli.ok(json!({"op":"scene.get","scene":"street","property":FIGURE_POSITION}))["value"]["x"]
            .as_f64()
            .unwrap()
    };

    // No-op commit, then cancel: the history keeps only the attach.
    let first = begin_street(&mut cli);
    cli.ok(json!({"op":"scene.gesture.update","gesture":first,"ops":[move_figure(0.5)]}));
    let noop = cli.ok(json!({"op":"scene.gesture.commit","gesture":first}));
    assert_eq!(noop["noop"], true);
    let second = begin_street(&mut cli);
    assert_ne!(first, second, "an ID is never reused inside a session");
    cli.ok(json!({"op":"scene.gesture.update","gesture":second,"ops":[move_figure(2.0)]}));
    cli.ok(json!({"op":"scene.gesture.cancel","gesture":second}));
    let status = cli.ok(json!({"op":"status"}));
    assert!(status.get("scene_interaction").is_none());
    assert_eq!(position(&mut cli), 0.5);

    // Commit, undo and redo: one entry, and an open gesture does not survive them.
    let third = begin_street(&mut cli);
    cli.ok(json!({"op":"scene.gesture.update","gesture":third,"ops":[move_figure(2.0)]}));
    cli.ok(json!({"op":"scene.gesture.commit","gesture":third}));
    assert_eq!(position(&mut cli), 2.0);
    let stale = begin_street(&mut cli);
    cli.ok(json!({"op":"history.undo"}));
    assert_eq!(position(&mut cli), 0.5);
    let invalidated = cli.err(json!({"op":"scene.gesture.commit","gesture":stale}));
    assert_eq!(invalidated["code"], "gesture_invalidated");
    cli.ok(json!({"op":"history.redo"}));
    assert_eq!(position(&mut cli), 2.0);
    let status = cli.ok(json!({"op":"status"}));
    assert_eq!(
        status["scene_interaction"]["gesture"]["state"], "invalidated",
        "the summary of the executable shows the dead gesture"
    );
    cli.ok(json!({"op":"scene.gesture.cancel","gesture":stale}));

    // Swap the project inside the same process, rebuild the same scene, begin again.
    let kept = begin_street(&mut cli);
    cli.ok(json!({"op":"scene.gesture.update","gesture":kept,"ops":[move_figure(7.0)]}));
    cli.ok(json!({"op":"project.create","name":"Swapped"}));
    cli.ok(attach);
    let fresh = begin_street(&mut cli);
    assert_ne!(kept, fresh);
    cli.ok(json!({"op":"scene.gesture.update","gesture":fresh,"ops":[move_figure(3.0)]}));
    for request in [
        json!({"op":"scene.gesture.update","gesture":kept,"ops":[move_figure(9.0)]}),
        json!({"op":"scene.gesture.commit","gesture":kept}),
    ] {
        let error = cli.err(request);
        assert_eq!(error["code"], "unknown_gesture");
        assert_eq!(error["details"]["reason"], "other_session");
    }
    let depth = cli.ok(json!({"op":"status"}));
    assert_eq!(depth["scene_interaction"]["gesture"]["id"], json!(fresh));
    assert_eq!(depth["scene_interaction"]["gesture"]["updates"], 1);
    assert_eq!(
        position(&mut cli),
        0.5,
        "nothing of `kept` reached the new project"
    );
    cli.ok(json!({"op":"scene.gesture.commit","gesture":fresh}));
    assert_eq!(position(&mut cli), 3.0);

    // Selection: pruned by undo, not restored by redo, shown in the summary.
    cli.ok(json!({"op":"scene.select","scene":"street","entities":[FIGURE]}));
    assert_eq!(
        cli.ok(json!({"op":"status"}))["scene_interaction"]["selection"]["count"],
        1
    );
    cli.ok(json!({"op":"history.undo"}));
    cli.ok(json!({"op":"history.undo"}));
    cli.ok(json!({"op":"history.redo"}));
    cli.ok(json!({"op":"history.redo"}));
    let selection = cli.ok(json!({"op":"scene.selection"}));
    assert_eq!(selection["count"], 0);
    cli.finish();
}
