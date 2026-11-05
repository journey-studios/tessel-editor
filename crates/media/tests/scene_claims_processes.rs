//! BL111 (F040): ownership of pending scene resources between **real processes**.
//!
//! Every case here runs a second OS process: this very test binary re-executed with
//! `TESSEL_CLAIMS_ROLE` set, so `child_role` below is a small command loop on stdin/stdout that
//! drives the real `adopt` and its owners against the same workspace folder as the test. The
//! "crash" is `SIGKILL` (`Child::kill`): no destructor, no accept, no discard runs in the victim.
//!
//! What is proven, and what is not:
//!
//! * a file a pending result of one process created and another process reused survives the
//!   discard of either, in both directions, and an accepted file is never removed by the other
//!   process (BL096, now across processes);
//! * a holder is alive only while its owner holds the lease (a held `flock`), never because of a
//!   process ID: after `SIGKILL` the claims are reaped, the orphans are kept until a result reuses
//!   them or `recover` verifies and removes them, and what a live process still relies on is
//!   never recovered;
//! * a file rewritten after its holder died is kept, an unreadable ledger refuses before any
//!   copy, and two processes cycling adopt/discard on one workspace end with the workspace as it
//!   was.
//!
//! Not proven: Windows, network filesystems, a hostile writer, an operating system crash (the
//! ledger is replaced atomically but not synced), a window (no GPUI here).

mod directed_resources_common;

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use directed_resources_common::*;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::jobs::CancelToken;
use tessel_editor_media::directed_resources::Options;
use tessel_editor_media::scene_resources::{
    Adoption, Provisional, ResourceBase, adopt, claims_status, recover, tracked_under,
};

const ROLE: &str = "TESSEL_CLAIMS_ROLE";
const PROTOCOL: &str = "@@ ";

fn adopt_into(scene: &DirectedScene, workspace: &Path, source: &Path) -> Result<Adoption, String> {
    adopt(
        scene,
        "street",
        workspace,
        &ResourceBase::explicit(source).unwrap(),
        &Options::default(),
        &CancelToken::default(),
        &mut |_| {},
        &mut |_| {},
    )
    .map_err(|error| {
        if error.code != "root_busy" {
            eprintln!(
                "adopt refused: {} ({}) {:?}",
                error.code, error.message, error.details
            );
        }
        error.code.to_string()
    })
}

// ------------------------------------------------------------------ the second process --

/// The command loop of the second process. Runs only when re-executed by [`Peer::spawn`]; as a
/// plain test it returns at once.
#[test]
fn child_role() {
    if std::env::var(ROLE).is_err() {
        return;
    }
    let var = |name: &str| PathBuf::from(std::env::var(name).unwrap());
    let (workspace, source) = (var("TESSEL_CLAIMS_WS"), var("TESSEL_CLAIMS_SRC"));
    let bundle: Value =
        serde_json::from_str(&fs::read_to_string(var("TESSEL_CLAIMS_BUNDLE")).unwrap()).unwrap();
    let scene = scene(bundle);
    let say = |value: Value| {
        println!("{PROTOCOL}{value}");
        std::io::stdout().flush().unwrap();
    };
    let mut held: Option<Provisional> = None;
    say(json!({"event": "ready", "pid": std::process::id()}));
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let mut words = line.split_whitespace();
        match words.next().unwrap_or_default() {
            "adopt" => match adopt_into(&scene, &workspace, &source) {
                Ok(mut adoption) => {
                    say(json!({"copied": adoption.report.copied(),
                        "reused": adoption.report.reused()}));
                    held = Some(adoption.provisional());
                }
                Err(code) => say(json!({"error": code})),
            },
            "discard" => {
                let report = held.take().unwrap().discard();
                say(
                    json!({"removedFiles": report.removed_files, "removedFolders":
                    report.removed_dirs, "shared": report.shared.len(),
                    "kept": report.kept.len()}),
                );
            }
            "accept" => {
                drop(held.take().unwrap().release());
                say(json!({"accepted": true}));
            }
            "cycle" => {
                let rounds: usize = words.next().unwrap().parse().unwrap();
                let (mut done, mut busy) = (0, 0);
                while done < rounds {
                    match adopt_into(&scene, &workspace, &source) {
                        Ok(mut adoption) => {
                            adoption.provisional().discard();
                            done += 1;
                        }
                        Err(code) if code == "root_busy" => {
                            busy += 1;
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(code) => panic!("cycle failed: {code}"),
                    }
                }
                say(json!({"cycled": done, "busy": busy}));
            }
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
            .env("TESSEL_CLAIMS_WS", &world.workspace)
            .env("TESSEL_CLAIMS_SRC", &world.sandbox.src)
            .env("TESSEL_CLAIMS_BUNDLE", &world.bundle)
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
        peer.pid = ready["pid"].as_u64().unwrap() as u32;
        assert_eq!(peer.pid, peer.child.id());
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

    fn quit(mut self) {
        assert_eq!(self.ask("quit")["bye"], true);
        assert!(self.child.wait().unwrap().success());
    }
}

// ------------------------------------------------------------------------- the world --

struct World {
    sandbox: Sandbox,
    scene: DirectedScene,
    workspace: PathBuf,
    bundle: PathBuf,
}

fn world(name: &str) -> World {
    let sandbox = Sandbox::new(&format!("te-proc-{name}"));
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let document = standard_bundle(&sandbox);
    let bundle = sandbox.root.join("bundle.json");
    fs::write(&bundle, document.to_string()).unwrap();
    World {
        scene: scene(document),
        sandbox,
        workspace,
        bundle,
    }
}

impl World {
    fn adopt(&self) -> Adoption {
        adopt_into(&self.scene, &self.workspace, &self.sandbox.src).unwrap()
    }

    fn backdrop(&self) -> PathBuf {
        self.workspace.join("scenes/street/assets/backdrop.png")
    }

    fn figure(&self) -> PathBuf {
        self.workspace.join("scenes/street/assets/figure.png")
    }

    fn assert_intact(&self) {
        assert_eq!(
            fs::read(self.backdrop()).unwrap(),
            fs::read(self.sandbox.src.join("assets/backdrop.png")).unwrap()
        );
        assert_eq!(
            fs::read(self.figure()).unwrap(),
            fs::read(self.sandbox.src.join("assets/figure.png")).unwrap()
        );
    }

    /// Nothing is claimed and the workspace carries no ledger: as if no pending result ever was.
    fn assert_no_claims(&self) {
        assert_eq!(tracked_under(&self.workspace), 0);
        assert!(
            !self.workspace.join(".tessel-claims").exists(),
            "{:?}",
            tree(&self.workspace)
        );
    }
}

fn reuse_only(adoption: &Adoption) {
    assert_eq!(adoption.report.copied(), 0, "{:?}", adoption.report);
    assert_eq!(adoption.report.reused(), 2, "{:?}", adoption.report);
}

fn entries(workspace: &Path) -> Vec<Value> {
    claims_status(workspace)["entries"]
        .as_array()
        .unwrap()
        .clone()
}

// -------------------------------------------------------------------- live processes --

#[test]
fn a_file_two_live_processes_share_survives_the_first_discard_in_both_directions() {
    for peer_first in [true, false] {
        let world = world(&format!("share-{peer_first}"));
        let before = tree(&world.workspace);
        let mut peer = Peer::spawn(&world);
        let mut mine;
        if peer_first {
            let answer = peer.ask("adopt");
            assert_eq!(answer["copied"], 2, "{answer}");
            mine = world.adopt();
            reuse_only(&mine);
        } else {
            mine = world.adopt();
            let answer = peer.ask("adopt");
            assert_eq!(answer["copied"], 0, "{answer}");
            assert_eq!(answer["reused"], 2, "{answer}");
        }

        // Two holders on each file: the lease of each process is held, by their own owners.
        let status = claims_status(&world.workspace);
        let files: Vec<_> = status["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["kind"] == "file")
            .collect();
        assert_eq!(files.len(), 2, "{status}");
        assert!(
            files
                .iter()
                .all(|entry| entry["holders"].as_array().unwrap().len() == 2),
            "{status}"
        );
        assert_eq!(status["owners"].as_array().unwrap().len(), 2, "{status}");
        let pids: Vec<u64> = status["owners"]
            .as_array()
            .unwrap()
            .iter()
            .map(|owner| owner["pid"].as_u64().unwrap())
            .collect();
        assert!(pids.contains(&u64::from(peer.pid)), "{status}");

        // The one that created them leaves first (a discard): the other still relies on them.
        if peer_first {
            let gone = peer.ask("discard");
            assert_eq!(gone["removedFiles"], 0, "{gone}");
            // Two files and the three folders they sit in.
            assert_eq!(gone["shared"], 5, "{gone}");
            world.assert_intact();
            drop(mine.provisional().release());
        } else {
            let gone = mine.provisional().discard();
            assert_eq!(gone.removed_files, 0, "{gone:?}");
            assert_eq!(gone.shared.len(), 5, "{gone:?}");
            world.assert_intact();
            assert_eq!(peer.ask("accept")["accepted"], true);
        }
        world.assert_intact();
        world.assert_no_claims();
        peer.quit();
        assert_ne!(tree(&world.workspace), before);
    }
}

#[test]
fn the_last_live_holder_to_discard_takes_everything_back() {
    let world = world("last-discard");
    let before = tree(&world.workspace);
    let mut peer = Peer::spawn(&world);
    assert_eq!(peer.ask("adopt")["copied"], 2);
    let mut mine = world.adopt();
    reuse_only(&mine);

    assert_eq!(peer.ask("discard")["removedFiles"], 0);
    world.assert_intact();
    let report = mine.provisional().discard();
    assert_eq!(report.removed_files, 2, "{report:?}");
    assert_eq!(tree(&world.workspace), before, "the folders go too");
    world.assert_no_claims();
    peer.quit();
}

#[test]
fn an_accepted_file_is_never_removed_by_the_other_process() {
    for peer_accepts in [true, false] {
        let world = world(&format!("accepted-{peer_accepts}"));
        let mut peer = Peer::spawn(&world);
        if peer_accepts {
            assert_eq!(peer.ask("adopt")["copied"], 2);
            assert_eq!(peer.ask("accept")["accepted"], true);
            let mut mine = world.adopt();
            reuse_only(&mine);
            let report = mine.provisional().discard();
            assert_eq!(report.removed_files, 0, "{report:?}");
        } else {
            let mut mine = world.adopt();
            drop(mine.provisional().release());
            let answer = peer.ask("adopt");
            assert_eq!(answer["reused"], 2, "{answer}");
            let gone = peer.ask("discard");
            assert_eq!(gone["removedFiles"], 0, "{gone}");
            assert_eq!(gone["removedFolders"], 0, "{gone}");
        }
        world.assert_intact();
        world.assert_no_claims();
        peer.quit();
    }
}

#[test]
fn a_ledger_the_process_cannot_trust_refuses_before_copying_anything() {
    let world = world("unreadable");
    let ledger = world.workspace.join(".tessel-claims");
    fs::create_dir_all(&ledger).unwrap();
    fs::write(ledger.join("ledger.lock"), b"").unwrap();
    fs::write(ledger.join("claims.json"), b"{ not json").unwrap();
    let before = tree(&world.workspace);

    let error = adopt_into(&world.scene, &world.workspace, &world.sandbox.src).unwrap_err();
    assert_eq!(error, "claims_unavailable");
    assert_eq!(tree(&world.workspace), before, "nothing was changed");
    assert_eq!(
        fs::read(ledger.join("claims.json")).unwrap(),
        b"{ not json",
        "and the ledger was left for a person to look at"
    );
    assert_eq!(
        claims_status(&world.workspace)["available"],
        false,
        "the status says so"
    );
}

#[test]
fn two_processes_cycling_adoptions_on_one_workspace_leave_it_as_it_was() {
    let world = world("cycle");
    let before = tree(&world.workspace);
    let mut peer = Peer::spawn(&world);
    writeln!(peer.stdin, "cycle 25").unwrap();
    peer.stdin.flush().unwrap();
    let mut mine = 0;
    while mine < 25 {
        match adopt_into(&world.scene, &world.workspace, &world.sandbox.src) {
            Ok(mut adoption) => {
                adoption.provisional().discard();
                mine += 1;
            }
            Err(code) if code == "root_busy" => std::thread::sleep(Duration::from_millis(2)),
            Err(code) => panic!("{code}"),
        }
    }
    let answer = peer.next();
    assert_eq!(answer["cycled"], 25, "{answer}");
    assert_eq!(tree(&world.workspace), before);
    world.assert_no_claims();
    peer.quit();
}

// ------------------------------------------------------------------------ a holder dies --

#[test]
fn after_a_crash_the_dead_holders_work_is_an_orphan_and_nothing_is_removed() {
    let world = world("crash-orphan");
    let mut peer = Peer::spawn(&world);
    let victim = peer.pid;
    assert_eq!(peer.ask("adopt")["copied"], 2);
    peer.crash();

    let status = claims_status(&world.workspace);
    assert_eq!(status["reaped"].as_array().unwrap().len(), 1, "{status}");
    assert_eq!(status["reaped"][0]["pid"], victim, "{status}");
    assert_eq!(status["owners"].as_array().unwrap().len(), 0, "{status}");
    let entries = entries(&world.workspace);
    assert!(!entries.is_empty());
    assert_eq!(status["orphans"], entries.len(), "every entry is an orphan");
    world.assert_intact();
}

#[test]
fn reusing_what_a_crashed_process_left_makes_the_new_holder_the_owner_and_accepting_keeps_it() {
    let world = world("crash-reuse");
    let mut peer = Peer::spawn(&world);
    assert_eq!(peer.ask("adopt")["copied"], 2);
    peer.crash();

    let mut mine = world.adopt();
    reuse_only(&mine);
    assert_eq!(
        claims_status(&world.workspace)["orphans"],
        0,
        "the files have a live holder again"
    );
    // Recovery in the meantime must leave what a live holder relies on alone.
    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.orphans, 0, "{recovery:?}");
    world.assert_intact();

    drop(mine.provisional().release());
    world.assert_intact();
    world.assert_no_claims();
}

#[test]
fn reusing_what_a_crashed_process_left_and_discarding_removes_it_as_the_last_holder() {
    let world = world("crash-reuse-discard");
    let before = tree(&world.workspace);
    let mut peer = Peer::spawn(&world);
    assert_eq!(peer.ask("adopt")["copied"], 2);
    peer.crash();

    let mut mine = world.adopt();
    reuse_only(&mine);
    let report = mine.provisional().discard();
    assert_eq!(report.removed_files, 2, "{report:?}");
    assert_eq!(tree(&world.workspace), before);
    world.assert_no_claims();
}

#[test]
fn recovery_removes_what_a_crashed_process_left_and_only_that() {
    let world = world("recover");
    // Something the workspace had before, which is nobody's to remove.
    fs::write(world.workspace.join("project.json"), b"{}").unwrap();
    fs::create_dir_all(world.workspace.join("scenes/other")).unwrap();
    fs::write(world.workspace.join("scenes/other/keep.txt"), b"mine").unwrap();
    let before = tree(&world.workspace);
    let mut peer = Peer::spawn(&world);
    assert_eq!(peer.ask("adopt")["copied"], 2);
    peer.crash();

    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.reaped_holders, 1, "{recovery:?}");
    assert_eq!(recovery.report.removed_files, 2, "{recovery:?}");
    assert_eq!(recovery.report.removed_dirs, 2, "{recovery:?}");
    assert!(recovery.report.kept.is_empty(), "{recovery:?}");
    assert_eq!(tree(&world.workspace), before);
    assert_eq!(
        fs::read(world.workspace.join("scenes/other/keep.txt")).unwrap(),
        b"mine"
    );
    world.assert_no_claims();

    // Idempotent: nothing left to recover.
    let again = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(again.orphans, 0, "{again:?}");
}

#[test]
fn recovery_keeps_a_file_that_was_rewritten_after_its_holder_died() {
    let world = world("recover-rewritten");
    let mut peer = Peer::spawn(&world);
    assert_eq!(peer.ask("adopt")["copied"], 2);
    peer.crash();

    // Same inode, same length, other bytes: only the content tells it from the published file.
    let mut file = OpenOptions::new()
        .write(true)
        .open(world.backdrop())
        .unwrap();
    file.seek(SeekFrom::End(-4)).unwrap();
    file.write_all(b"edit").unwrap();
    drop(file);
    let edited = fs::read(world.backdrop()).unwrap();

    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.report.removed_files, 1, "{recovery:?}");
    assert!(
        recovery
            .report
            .kept
            .iter()
            .any(|why| why.contains("rewritten in place")),
        "{recovery:?}"
    );
    assert_eq!(
        fs::read(world.backdrop()).unwrap(),
        edited,
        "the edit is kept"
    );
    assert!(!world.figure().exists(), "the untouched one went");
    assert_eq!(
        tracked_under(&world.workspace),
        0,
        "and nothing stays claimed"
    );
}

#[test]
fn a_dead_holder_does_not_keep_a_live_one_from_being_the_last_and_recovery_ignores_what_it_holds() {
    let world = world("dead-and-live");
    let before = tree(&world.workspace);
    let mut dying = Peer::spawn(&world);
    assert_eq!(dying.ask("adopt")["copied"], 2);
    let mut living = Peer::spawn(&world);
    let answer = living.ask("adopt");
    assert_eq!(answer["reused"], 2, "{answer}");
    dying.crash();

    // The living one holds both files; the crashed one is not counted, and not recovered over it.
    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.reaped_holders, 1, "{recovery:?}");
    assert_eq!(recovery.orphans, 0, "{recovery:?}");
    assert_eq!(recovery.report.removed_files, 0, "{recovery:?}");
    world.assert_intact();

    let gone = living.ask("discard");
    assert_eq!(
        gone["removedFiles"], 2,
        "the last live holder removes them: {gone}"
    );
    assert_eq!(tree(&world.workspace), before);
    world.assert_no_claims();
    living.quit();
}
