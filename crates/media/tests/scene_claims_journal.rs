//! BL112.7 (F025/F040): what the acceptance of the window's tick does to a **large** claims ledger,
//! through the real `MediaHost` and with a real second process.
//!
//! A ledger past `INLINE_LIMIT` (64 KiB) is not rewritten by the tick: the acceptance appends one
//! record to `.tessel-claims/claims.journal`, and every reader applies the journal when it opens
//! the ledger (design and invariants: `ledger.rs`, `docs/features/scene-resource-integration.md`).
//! The unit proofs are in `claims/tests.rs`; the timing is in `scene_claims_budget.rs`. Here:
//!
//! * the tick of an adoption and of an import appends one record, leaves `claims.json` byte for
//!   byte as it was, and the project, the undo history and the files are what a rewrite would
//!   have left; the next reader sees the result's entries gone and the others' untouched;
//! * a **stale** result (the project was replaced), a result **discarded** before the tick and one
//!   whose acceptance **cannot be written** never write an acceptance: the files and claims of
//!   the result are handled exactly as with a small ledger, and the claims of others survive;
//! * the blocking caller (the CLI's `job.commit`) takes the same path, keeps the same result, and
//!   keeps the refusal code and its `details.phase` when the record cannot be written;
//! * **recovery**: a window process killed with `SIGKILL` right after its tick (the journal is
//!   still there, nobody folded it) leaves nothing for `recover` of another process to remove;
//!   what it accepted stays, and the claims of a live peer are untouched. A reader hammering the
//!   ledger from the other process while the tick appends never sees a fault.
//!
//! The large ledger is seeded once, in the format the ledger writes, as the claims of an owner
//! whose lease this test holds (a live peer: its entries are never orphans). No test here
//! depends on the clock. The journal is never read through `claims_status` before it is checked
//! (every reader folds it), and the failure is provoked by putting a folder where the journal file
//! should be, so the record cannot be opened for appending.
#![cfg(unix)]

mod directed_resources_common;

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use directed_resources_common::*;
use serde_json::{Map, Value, json};
use tessel_editor_core::jobs::CancelToken;
use tessel_editor_core::{CommandError, EditorState, headless, persist};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::scene_resources::{claims_status, recover};

const ROLE: &str = "TESSEL_JOURNAL_ROLE";
const PROTOCOL: &str = "@@ ";
/// The token of the seeded owner (never zero, never this process's).
const SEEDED: u128 = 0x5eed_0b5e_55ed_1ed9_0000_0000_0000_0002;
/// Entries of the seeded owner: about 130 bytes each, so the ledger is past 64 KiB.
const SEEDS: usize = 800;
const SEED_ROOT: &str = "/te-journal-seed";

/// Writes the ledger of `workspace` with `SEEDS` entries of the seeded owner and returns the
/// lease: while the returned file lives the owner is alive for every other reader.
fn seed(workspace: &Path) -> File {
    let ledger = workspace.join(".tessel-claims");
    fs::create_dir_all(ledger.join("live")).unwrap();
    File::create(ledger.join("ledger.lock")).unwrap();
    let owner = format!("{SEEDED:032x}");
    let lease = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(ledger.join("live").join(&owner))
        .unwrap();
    lease.try_lock().unwrap();
    let holder = format!("{owner}:1");
    let mut entries = Map::new();
    for index in 0..SEEDS {
        entries.insert(
            format!("{SEED_ROOT}/assets/f{index}.bin"),
            json!({"kind": "file", "root": SEED_ROOT, "stamp": null, "holders": [holder.clone()]}),
        );
    }
    let document = json!({
        "version": 1,
        "owners": { owner: { "pid": std::process::id(), "since": 1 } },
        "entries": entries,
    });
    fs::write(
        ledger.join("claims.json"),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    assert!(
        fs::metadata(ledger.join("claims.json")).unwrap().len() > 64 * 1024,
        "the seeded ledger is past the inline limit"
    );
    lease
}

/// A project saved in `workspace` with the scene `street` attached, as a window has it.
fn project_with_scene(workspace: &Path, bundle: &Path) -> EditorState {
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":"journal"})).unwrap();
    persist::save(&mut state, &workspace.join("p.tessel.json")).unwrap();
    headless::execute(
        &mut state,
        &json!({"op":"scene.attach","id":"street","path":bundle}),
    )
    .unwrap();
    persist::save(&mut state, &workspace.join("p.tessel.json")).unwrap();
    state
}

struct World {
    sandbox: Sandbox,
    state: EditorState,
    host: MediaHost,
    workspace: PathBuf,
    bundle: PathBuf,
    _lease: File,
}

fn world(name: &str, host: MediaHost) -> World {
    let sandbox = Sandbox::new(&format!("te-journal-{name}"));
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let document = standard_bundle(&sandbox);
    let bundle = sandbox.root.join("street.bundle.json");
    fs::write(&bundle, document.to_string()).unwrap();
    let state = project_with_scene(&workspace, &bundle);
    let lease = seed(&workspace);
    World {
        sandbox,
        state,
        host,
        workspace,
        bundle,
        _lease: lease,
    }
}

impl World {
    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        headless::execute_ext(&mut self.state, None, Some(&mut self.host), &request)
    }

    fn state_file(&self) -> PathBuf {
        self.workspace.join(".tessel-claims/claims.json")
    }

    fn journal(&self) -> PathBuf {
        self.workspace.join(".tessel-claims/claims.journal")
    }

    /// A finished adoption nobody has applied yet (the result waits in its slot).
    fn adopt(&mut self) -> String {
        let source = self.sandbox.src.clone();
        let started = self
            .run(
                json!({"op":"scene.resources.adopt","scene":"street","source":source,"wait":false}),
            )
            .unwrap();
        let job = started["job"].as_str().unwrap().to_string();
        let registry = self.host.registry().clone();
        registry
            .wait(&job, Duration::from_secs(30))
            .unwrap_or_else(|| panic!("`{job}` never ended"));
        job
    }

    /// The window's tick until `job` is reported.
    fn tick(&mut self, job: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(event) = self
                .host
                .poll(&mut self.state)
                .into_iter()
                .find(|event| event["job"] == job)
            {
                return event;
            }
            assert!(Instant::now() < deadline, "`{job}` was never reported");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// How many entries of the ledger lie under `fragment` (opening the ledger folds the journal).
    fn claims(&self, fragment: &str) -> usize {
        claims_status(&self.workspace)["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| {
                entry["path"]
                    .as_str()
                    .is_some_and(|path| path.contains(fragment))
            })
            .count()
    }

    fn files(&self) -> [PathBuf; 2] {
        [
            self.workspace.join("scenes/street/assets/backdrop.png"),
            self.workspace.join("scenes/street/assets/figure.png"),
        ]
    }

    fn assert_files_kept(&self) {
        for (path, original) in self.files().iter().zip(["backdrop.png", "figure.png"]) {
            assert_eq!(
                fs::read(path).unwrap(),
                fs::read(self.sandbox.src.join("assets").join(original)).unwrap(),
                "`{}` must be there with its content",
                path.display()
            );
        }
    }

    fn assert_files_gone(&self) {
        for path in self.files() {
            assert!(!path.exists(), "`{}` must be removed", path.display());
        }
    }
}

/// The record the tick appended: one complete line, naming the result's files.
fn assert_one_record_of_the_street(journal: &Path) {
    let record = fs::read_to_string(journal)
        .unwrap_or_else(|error| panic!("no journal record after the tick: {error}"));
    assert_eq!(record.lines().count(), 1, "one record: {record}");
    assert!(record.ends_with('\n'), "a complete record: {record}");
    assert!(
        record.contains("/scenes/street/assets/backdrop.png")
            && record.contains("/scenes/street/assets/figure.png"),
        "{record}"
    );
}

// ----------------------------------------------------------------------- the tick accepts --

#[test]
fn the_tick_accepts_an_adoption_in_a_large_ledger_with_one_record_and_leaves_the_state_alone() {
    let mut world = world("adopt", MediaHost::new().interactive());
    let job = world.adopt();
    let before = fs::read(world.state_file()).unwrap();
    assert!(world.claims("/scenes/street") >= 2, "its files are claimed");
    // `claims` folded nothing (no journal yet), so the state is what the job left.
    assert!(
        fs::read(world.state_file()).unwrap() == before,
        "the tick rewrote the state of the ledger"
    );

    let event = world.tick(&job);
    assert_eq!(event["state"], "succeeded", "{event}");

    assert_eq!(
        fs::read(world.state_file()).unwrap(),
        before,
        "the tick did not rewrite the ledger"
    );
    assert_one_record_of_the_street(&world.journal());
    world.assert_files_kept();
    // The next reader sees the result's entries as gone, and the peer's as they were.
    assert_eq!(world.claims("/scenes/street"), 0);
    assert_eq!(world.claims(SEED_ROOT), SEEDS);
    assert!(
        !world.journal().exists(),
        "and folded the journal into the state before it answered"
    );
}

#[test]
fn the_tick_applies_an_import_in_a_large_ledger_with_one_record_and_the_project_is_as_a_rewrite_leaves_it()
 {
    let mut world = world("import", MediaHost::new().interactive());
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":"host"})).unwrap();
    persist::save(&mut state, &world.workspace.join("host.tessel.json")).unwrap();
    world.state = state;
    let mut source = EditorState::demo();
    headless::execute(&mut source, &json!({"op":"project.create","name":"source"})).unwrap();
    headless::execute(
        &mut source,
        &json!({"op":"scene.attach","id":"street","path":world.bundle}),
    )
    .unwrap();
    let document = world.sandbox.root.join("source/source.tessel.json");
    fs::create_dir_all(document.parent().unwrap()).unwrap();
    persist::save(&mut source, &document).unwrap();

    let bases = json!({ "street": world.sandbox.src });
    let started = world
        .run(json!({"op":"project.import_scenes","source":document,"scene_bases":bases,"wait":false}))
        .unwrap();
    let job = started["job"].as_str().unwrap().to_string();
    let registry = world.host.registry().clone();
    registry.wait(&job, Duration::from_secs(30)).unwrap();
    let before = fs::read(world.state_file()).unwrap();

    let event = world.tick(&job);
    assert_eq!(event["state"], "succeeded", "{event}");
    assert_eq!(world.state.scenes.len(), 1, "the scene is attached");
    assert!(world.state.history.can_undo(), "as one undo step");
    assert!(
        fs::read(world.state_file()).unwrap() == before,
        "the tick rewrote the state of the ledger"
    );
    assert_one_record_of_the_street(&world.journal());
    world.assert_files_kept();
    assert_eq!(world.claims("/scenes/street"), 0);
    assert_eq!(world.claims(SEED_ROOT), SEEDS);
}

// ------------------------------------------------------- what is not accepted writes nothing --

#[test]
fn a_stale_result_is_discarded_and_no_acceptance_is_written_in_a_large_ledger() {
    let mut world = world("stale", MediaHost::new().interactive());
    let job = world.adopt();
    // The project is replaced before the tick sees the finished job.
    world
        .run(json!({"op":"project.create","name":"next"}))
        .unwrap();

    let event = world.tick(&job);
    assert_eq!(event["state"], "failed", "{event}");
    assert_eq!(event["error"]["code"], "stale_result", "{event}");
    let discard = event["error"]["details"]["discard_job"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal names its discard job: {event}"))
        .to_string();
    let ended = world.tick(&discard);
    assert_eq!(ended["state"], "succeeded", "{ended}");
    assert!(!world.journal().exists(), "a result never accepted");
    world.assert_files_gone();
    assert_eq!(world.claims("/scenes/street"), 0);
    assert_eq!(world.claims(SEED_ROOT), SEEDS, "the peer's claims survive");
}

#[test]
fn a_result_discarded_before_the_tick_writes_no_acceptance_in_a_large_ledger() {
    let mut world = world("discard", MediaHost::new().interactive());
    let job = world.adopt();
    let answer = world
        .run(json!({"op":"job.discard","job":job,"wait":false}))
        .unwrap();
    let discard = answer["job"].as_str().unwrap().to_string();
    let registry = world.host.registry().clone();
    registry.wait(&discard, Duration::from_secs(30)).unwrap();

    assert!(!world.journal().exists());
    world.assert_files_gone();
    assert_eq!(world.claims("/scenes/street"), 0);
    assert_eq!(world.claims(SEED_ROOT), SEEDS);
}

#[test]
fn a_journal_that_cannot_be_written_refuses_the_acceptance_and_changes_nothing() {
    let mut world = world("fails", MediaHost::new().interactive());
    let job = world.adopt();
    let (summary, revision, can_undo) = (
        world.state.summary(),
        world.state.project.revision,
        world.state.history.can_undo(),
    );
    // A folder where the journal file should be: the record cannot be opened for appending.
    fs::create_dir(world.journal()).unwrap();

    let event = world.tick(&job);
    assert_eq!(event["state"], "failed", "{event}");
    assert_eq!(event["error"]["code"], "claims_unavailable", "{event}");
    let details = &event["error"]["details"];
    assert_eq!(details["phase"], "accept", "{event}");
    assert!(
        details["why"]
            .as_str()
            .is_some_and(|why| why.contains("claims.journal")),
        "{event}"
    );
    let discard = details["discard_job"].as_str().unwrap().to_string();
    // The project is exactly as it was: no undo step, no new revision.
    assert_eq!(world.state.summary(), summary);
    assert_eq!(world.state.project.revision, revision);
    assert_eq!(world.state.history.can_undo(), can_undo);

    // The ledger cannot be read while the folder is there, so the discard job keeps what it
    // cannot prove is its own: the files stay, and so do their claims.
    let ended = world.tick(&discard);
    assert_eq!(ended["state"], "succeeded", "{ended}");
    assert_eq!(ended["result"]["cleanup"]["removedFiles"], 0, "{ended}");
    world.assert_files_kept();
    let state = fs::read_to_string(world.state_file()).unwrap();
    assert!(
        state.contains("backdrop.png") && state.contains("figure.png"),
        "still claimed"
    );
    fs::remove_dir(world.journal()).unwrap();
}

// ----------------------------------------------------------------------- the blocking caller --

#[test]
fn the_blocking_commit_of_a_cli_takes_the_same_path_and_keeps_its_result() {
    let mut world = world("cli", MediaHost::new());
    let job = world.adopt();
    let before = fs::read(world.state_file()).unwrap();

    let committed = world.run(json!({"op":"job.commit","job":job})).unwrap();
    assert_eq!(committed["action"], "scene.resources.adopt", "{committed}");
    assert!(
        fs::read(world.state_file()).unwrap() == before,
        "the tick rewrote the state of the ledger"
    );
    assert_one_record_of_the_street(&world.journal());
    world.assert_files_kept();
    assert_eq!(world.claims("/scenes/street"), 0);
    assert_eq!(world.claims(SEED_ROOT), SEEDS);
}

#[test]
fn the_blocking_commit_of_a_cli_keeps_the_refusal_code_when_the_record_cannot_be_written() {
    let mut world = world("cli-fails", MediaHost::new());
    let job = world.adopt();
    let (summary, revision) = (world.state.summary(), world.state.project.revision);
    fs::create_dir(world.journal()).unwrap();

    let error = world
        .run(json!({"op":"job.commit","job":job}))
        .expect_err("an acceptance nobody could write down must not apply the result");
    assert_eq!(error.code, "claims_unavailable", "{error:?}");
    assert_eq!(
        error.details.as_ref().unwrap()["phase"],
        "accept",
        "{error:?}"
    );
    assert_eq!(world.state.summary(), summary);
    assert_eq!(world.state.project.revision, revision);
    world.assert_files_kept();
    fs::remove_dir(world.journal()).unwrap();
}

// ----------------------------------------------------------------------------- recovery --

/// The command loop of the window process: a window's host (`interactive`) with its project, in
/// the workspace of the test. Runs only when re-executed by [`Peer::spawn`].
#[test]
fn child_role() {
    if std::env::var(ROLE).is_err() {
        return;
    }
    let var = |name: &str| PathBuf::from(std::env::var(name).unwrap());
    let (workspace, source, bundle) = (
        var("TESSEL_JOURNAL_WS"),
        var("TESSEL_JOURNAL_SRC"),
        var("TESSEL_JOURNAL_BUNDLE"),
    );
    let mut state = project_with_scene(&workspace, &bundle);
    let mut host = MediaHost::new().interactive();
    let registry = host.registry().clone();
    let say = |value: Value| {
        println!("{PROTOCOL}{value}");
        std::io::stdout().flush().unwrap();
    };
    say(json!({"event": "ready", "pid": std::process::id()}));
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        match line.trim() {
            "adopt" => {
                let started = headless::execute_ext(
                    &mut state,
                    None,
                    Some(&mut host),
                    &json!({"op":"scene.resources.adopt","scene":"street","source":source}),
                )
                .unwrap();
                let job = started["job"].as_str().unwrap().to_string();
                registry.wait(&job, Duration::from_secs(30)).unwrap();
                say(json!({"job": job}));
            }
            // The window's tick.
            "poll" => say(json!({"events": host.poll(&mut state)})),
            "quit" => {
                say(json!({"bye": true}));
                return;
            }
            other => panic!("unknown command `{other}`"),
        }
    }
}

/// A real second process and the pipe to talk to it.
struct Peer {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<Value>,
    pid: u32,
}

impl Peer {
    fn spawn(world: &World) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["child_role", "--exact", "--nocapture", "--test-threads=1"])
            .env(ROLE, "1")
            .env("TESSEL_JOURNAL_WS", &world.workspace)
            .env("TESSEL_JOURNAL_SRC", &world.sandbox.src)
            .env("TESSEL_JOURNAL_BUNDLE", &world.bundle)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                // libtest leaves `test child_role ... ` unterminated before the test prints.
                if let Some((_, body)) = line.split_once(PROTOCOL) {
                    let _ = sender.send(serde_json::from_str(body).unwrap());
                }
            }
        });
        let mut peer = Self {
            child,
            stdin,
            lines,
            pid: 0,
        };
        let ready = peer.next();
        assert_eq!(ready["event"], "ready", "{ready}");
        peer.pid = u32::try_from(ready["pid"].as_u64().unwrap()).unwrap();
        assert_ne!(peer.pid, std::process::id(), "a different process");
        peer
    }

    fn next(&mut self) -> Value {
        match self.lines.recv_timeout(Duration::from_secs(60)) {
            Ok(value) => value,
            Err(RecvTimeoutError::Timeout) => panic!("the second process did not answer"),
            Err(RecvTimeoutError::Disconnected) => panic!("the second process ended"),
        }
    }

    fn ask(&mut self, command: &str) -> Value {
        writeln!(self.stdin, "{command}").unwrap();
        self.stdin.flush().unwrap();
        self.next()
    }

    /// `SIGKILL`: nothing in the victim runs again.
    fn crash(mut self) {
        self.child.kill().unwrap();
        let status = self.child.wait().unwrap();
        assert!(!status.success(), "it was killed, it did not leave");
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn a_window_killed_right_after_its_tick_leaves_nothing_for_recover_to_take_back() {
    let world = world("kill-accepted", MediaHost::new());
    let mut peer = Peer::spawn(&world);
    peer.ask("adopt");

    // Another process reads the ledger over and over while the window's tick appends to it:
    // opening it folds the journal, so the two really interleave on the same `flock`.
    let done = AtomicBool::new(false);
    let polled = std::thread::scope(|scope| {
        let reader = scope.spawn(|| {
            let mut looks = 0u32;
            while !done.load(Ordering::SeqCst) {
                let status = claims_status(&world.workspace);
                assert_eq!(
                    status["available"], true,
                    "a reader never sees a fault: {status}"
                );
                looks += 1;
                std::thread::sleep(Duration::from_millis(1));
            }
            looks
        });
        let polled = peer.ask("poll");
        done.store(true, Ordering::SeqCst);
        assert!(
            reader.join().unwrap() > 0,
            "the reader looked at least once"
        );
        polled
    });
    let events = polled["events"].as_array().unwrap();
    assert!(
        events.iter().any(|event| event["state"] == "succeeded"),
        "the tick was not refused by a reader that held the lock briefly: {polled}"
    );
    // Whether or not a reader folded the record first, the result's entries are gone for every
    // reader and the peer's are not.
    assert_eq!(world.claims("/scenes/street"), 0);
    assert_eq!(world.claims(SEED_ROOT), SEEDS);
    peer.crash();

    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.report.removed_files, 0, "{recovery:?}");
    assert_eq!(recovery.orphans, 0, "{recovery:?}");
    world.assert_files_kept();
    assert_eq!(world.claims(SEED_ROOT), SEEDS, "a live peer's claims stay");
}

#[test]
fn a_window_killed_before_any_reader_folds_its_record_is_still_not_recovered() {
    let world = world("kill-unfolded", MediaHost::new());
    let mut peer = Peer::spawn(&world);
    peer.ask("adopt");
    let before = fs::read(world.state_file()).unwrap();
    let polled = peer.ask("poll");
    assert!(
        polled["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["state"] == "succeeded"),
        "{polled}"
    );
    // Nobody has opened the ledger since the tick: the record is there, the state is untouched.
    assert!(
        fs::read(world.state_file()).unwrap() == before,
        "the tick rewrote the state of the ledger"
    );
    assert_one_record_of_the_street(&world.journal());
    peer.crash();

    // The dead window's lease is gone, so without the journal its entries would be orphans and
    // `recover` would remove files the project references.
    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.report.removed_files, 0, "{recovery:?}");
    assert_eq!(recovery.orphans, 0, "{recovery:?}");
    assert_eq!(recovery.reaped_holders, 1, "the window's holder was reaped");
    world.assert_files_kept();
    assert!(!world.journal().exists(), "folded by the reader");
    assert_eq!(world.claims(SEED_ROOT), SEEDS);
}
