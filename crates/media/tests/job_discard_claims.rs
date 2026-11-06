//! The seam between `job.discard` (BL094/BL112, S1) and the persistent claims ledger with its
//! fallible acceptance (BL111/BL111.2, S2), through the real `MediaHost` of a window
//! (`interactive()`), with real second processes where the claim is between processes.
//!
//! What is proven:
//!
//! * a result handed to a `discard` job stays **owner of its files and its claims** while the job is
//!   `queued` or running, across processes: another process sees the holder alive (its lease is
//!   held), `recover` takes nothing from it, and a file a second process reused stays when the job
//!   ends (it removes only what nobody else relies on);
//! * `SIGKILL` of the process while its discard job is queued leaves unaccepted claims as orphans
//!   (`recover` takes back exactly those, after the content proof), and a result the window
//!   already **accepted** is never recovered, whatever happens to the process afterwards;
//! * a refused acceptance (the ledger cannot be written) is not applied: the adoption or the
//!   import goes to a discard job **still claimed** (`claims_unavailable`, `details.phase =
//!   "accept"`, `details.discard_job`), the project, its undo history and its revision are as they
//!   were, and the job removes the files once the ledger can be written again;
//! * the blocking callers (`job.commit` of a CLI) keep the same refusal and its code.
//!
//! The ledger write failure is provoked as in `scene_claims_integrity.rs`: `claims.json.tmp` is a
//! folder, and a bystander adoption keeps the ledger non-empty so the write really happens. The
//! "queued" discard is held deterministically by occupying the registry's running slots, never by
//! a delay. Not proven here: a native window tick (`scripts/job-discard-proof.py` and
//! `scripts/scene-claims-proof.py` drive it), Windows, network filesystems, a hostile writer.

mod directed_resources_common;

use std::collections::BTreeMap;
use std::fs::{self};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use directed_resources_common::*;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::jobs::{CancelToken, JobKind, JobRegistry, JobState, ResultState};
use tessel_editor_core::{CommandError, EditorState, headless, persist};
use tessel_editor_media::directed_resources::Options;
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::scene_resources::{Adoption, ResourceBase, adopt, claims_status, recover};

const ROLE: &str = "TESSEL_DISCARD_ROLE";
const PROTOCOL: &str = "@@ ";

// ---------------------------------------------------------------------------- shared pieces --

/// Fills every running slot of `registry` with jobs that end when released, so any job spawned
/// afterwards stays `queued`.
///
/// Dropping it releases the slots too (N3): a failed assertion while it is held would otherwise
/// unwind into a non-interactive host that waits for a discard queued behind these slots, and the
/// suite would hang instead of reporting the failure.
struct Blockers {
    release: CancelToken,
    jobs: Vec<String>,
    registry: JobRegistry,
}

impl Blockers {
    fn hold(registry: &JobRegistry) -> Self {
        let release = CancelToken::new();
        let jobs: Vec<String> = (0..2)
            .map(|_| {
                let hold = release.clone();
                registry.spawn(JobKind::Proxy, "blocker", move |_| {
                    while !hold.is_cancelled() {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Ok(json!(null))
                })
            })
            .collect();
        for job in &jobs {
            let deadline = Instant::now() + Duration::from_secs(10);
            while registry.get(job).unwrap().state != JobState::Running {
                assert!(Instant::now() < deadline, "a blocker never started");
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Self {
            release,
            jobs,
            registry: registry.clone(),
        }
    }

    fn release(self) {
        self.release.cancel();
        for job in &self.jobs {
            self.registry.wait(job, Duration::from_secs(10)).unwrap();
        }
    }
}

impl Drop for Blockers {
    fn drop(&mut self) {
        self.release.cancel();
    }
}

#[test]
fn blockers_that_are_dropped_instead_of_released_give_their_slots_back() {
    let registry = JobRegistry::new(2);
    let blockers = Blockers::hold(&registry);
    let jobs = blockers.jobs.clone();
    drop(blockers);
    for job in &jobs {
        let info = registry
            .wait(job, Duration::from_secs(10))
            .expect("a dropped blocker ends its jobs");
        assert_eq!(info.state, JobState::Succeeded);
    }
}

fn wait_final(registry: &JobRegistry, job: &str) {
    registry
        .wait(job, Duration::from_secs(30))
        .unwrap_or_else(|| panic!("`{job}` never ended"));
}

/// A project saved in `workspace` with the scene `street` attached, as a window has it.
fn project_with_scene(sandbox: &Sandbox, workspace: &Path, bundle: &Path) -> EditorState {
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":"discard"})).unwrap();
    persist::save(&mut state, &workspace.join("p.tessel.json")).unwrap();
    headless::execute(
        &mut state,
        &json!({"op":"scene.attach","id":"street","path":bundle}),
    )
    .unwrap();
    persist::save(&mut state, &workspace.join("p.tessel.json")).unwrap();
    let _ = sandbox;
    state
}

fn run(
    state: &mut EditorState,
    host: &mut MediaHost,
    request: Value,
) -> Result<Value, CommandError> {
    headless::execute_ext(state, None, Some(host), &request)
}

fn poll_until_reported(host: &mut MediaHost, state: &mut EditorState, job: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(event) = host
            .poll(state)
            .into_iter()
            .find(|event| event["job"] == job)
        {
            return event;
        }
        assert!(Instant::now() < deadline, "`{job}` was never reported");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn adopt_into(
    scene: &DirectedScene,
    id: &str,
    workspace: &Path,
    source: &Path,
) -> Result<Adoption, String> {
    adopt(
        scene,
        id,
        workspace,
        &ResourceBase::explicit(source).unwrap(),
        &Options::default(),
        &CancelToken::default(),
        &mut |_| {},
        &mut |_| {},
    )
    .map_err(|error| error.code.to_string())
}

fn block_writes(workspace: &Path) {
    let ledger = workspace.join(".tessel-claims");
    assert!(
        ledger.join("claims.json").is_file(),
        "{:?}",
        tree(workspace)
    );
    fs::create_dir(ledger.join("claims.json.tmp")).unwrap();
}

fn unblock_writes(workspace: &Path) {
    fs::remove_dir(workspace.join(".tessel-claims/claims.json.tmp")).unwrap();
}

fn file_entries(workspace: &Path) -> Vec<Value> {
    claims_status(workspace)["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == "file")
        .cloned()
        .collect()
}

/// The claims of the scene `scene` (by its path under `scenes/`).
fn claims_of(workspace: &Path, scene: &str) -> Vec<Value> {
    file_entries(workspace)
        .into_iter()
        .filter(|entry| {
            entry["path"]
                .as_str()
                .is_some_and(|path| path.contains(&format!("/scenes/{scene}/")))
        })
        .collect()
}

// ------------------------------------------------------------------------- the second process --

/// The command loop of the second process: a window's host (`interactive`) with its project, in
/// the same workspace as the test. Runs only when re-executed by [`Peer::spawn`].
#[test]
fn child_role() {
    if std::env::var(ROLE).is_err() {
        return;
    }
    let var = |name: &str| PathBuf::from(std::env::var(name).unwrap());
    let (workspace, source, bundle) = (
        var("TESSEL_DISCARD_WS"),
        var("TESSEL_DISCARD_SRC"),
        var("TESSEL_DISCARD_BUNDLE"),
    );
    let sandbox = Sandbox::new("te-discard-claims-child");
    let mut state = project_with_scene(&sandbox, &workspace, &bundle);
    let mut host = MediaHost::new().interactive();
    let registry = host.registry().clone();
    let say = |value: Value| {
        println!("{PROTOCOL}{value}");
        std::io::stdout().flush().unwrap();
    };
    let (mut blockers, mut result_job, mut discard_job) = (None, String::new(), String::new());
    let mut _held: Option<fs::File> = None;
    say(json!({"event": "ready", "pid": std::process::id()}));
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        match line.trim() {
            // A finished adoption nobody has applied yet: the result waits in its slot.
            "adopt" => {
                let started = run(
                    &mut state,
                    &mut host,
                    json!({"op":"scene.resources.adopt","scene":"street","source":source}),
                )
                .unwrap();
                result_job = started["job"].as_str().unwrap().to_string();
                wait_final(&registry, &result_job);
                say(json!({"job": result_job}));
            }
            "block" => {
                blockers = Some(Blockers::hold(&registry));
                say(json!({"blocked": true}));
            }
            "discard" => {
                let answer = run(
                    &mut state,
                    &mut host,
                    json!({"op":"job.discard","job":result_job}),
                )
                .unwrap();
                discard_job = answer["job"].as_str().unwrap().to_string();
                say(json!({"state": answer["state"], "discard": discard_job,
                    "discardState": format!("{:?}", registry.get(&discard_job).unwrap().state).to_lowercase()}));
            }
            // The discard job has started removing: it reached its `cleanup` stage, which it sets
            // as the first thing it does, before it takes the root lease and hashes the files.
            "hashing" => {
                let deadline = Instant::now() + Duration::from_secs(30);
                loop {
                    let info = registry.get(&discard_job).unwrap();
                    if info.state == JobState::Running && info.stage == "cleanup" {
                        break;
                    }
                    assert!(Instant::now() < deadline, "the discard job never started");
                    std::thread::sleep(Duration::from_millis(1));
                }
                say(json!({"hashing": true}));
            }
            // Takes the ledger's lock and keeps it, as a process does that is stopped in the
            // middle of a read-modify-write.
            "hold_ledger" => {
                let file = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(workspace.join(".tessel-claims/ledger.lock"))
                    .unwrap();
                file.lock().unwrap();
                _held = Some(file);
                say(json!({"held": true}));
            }
            // The window's tick.
            "poll" => say(json!({"events": host.poll(&mut state)})),
            "release" => {
                blockers.take().unwrap().release();
                wait_final(&registry, &discard_job);
                let ended = poll_until_reported(&mut host, &mut state, &discard_job);
                say(json!({"ended": ended}));
            }
            "release_only" => {
                blockers.take().unwrap().release();
                say(json!({"released": true}));
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
            .env("TESSEL_DISCARD_WS", &world.workspace)
            .env("TESSEL_DISCARD_SRC", &world.sandbox.src)
            .env("TESSEL_DISCARD_BUNDLE", &world.bundle)
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

struct World {
    sandbox: Sandbox,
    scene: DirectedScene,
    workspace: PathBuf,
    bundle: PathBuf,
}

fn world(name: &str) -> World {
    let sandbox = Sandbox::new(&format!("te-discard-claims-{name}"));
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
    fn files(&self, scene: &str) -> [PathBuf; 2] {
        [
            self.workspace
                .join(format!("scenes/{scene}/assets/backdrop.png")),
            self.workspace
                .join(format!("scenes/{scene}/assets/figure.png")),
        ]
    }

    fn assert_intact(&self, scene: &str) {
        for (path, original) in self.files(scene).iter().zip(["backdrop.png", "figure.png"]) {
            assert_eq!(
                fs::read(path).unwrap(),
                fs::read(self.sandbox.src.join("assets").join(original)).unwrap(),
                "`{}` must be there with its content",
                path.display()
            );
        }
    }

    fn assert_gone(&self, scene: &str) {
        for path in self.files(scene) {
            assert!(!path.exists(), "`{}` must be removed", path.display());
        }
    }

    /// Another pending result that keeps the ledger non-empty (see the module docs).
    fn bystander(&self) -> Adoption {
        adopt_into(&self.scene, "bystander", &self.workspace, &self.sandbox.src).unwrap()
    }
}

// ------------------------------------------------------------- live processes, queued discard --

#[test]
fn a_queued_discard_keeps_the_live_holders_files_and_claims_from_the_other_process() {
    let world = world("queued");
    let mut peer = Peer::spawn(&world);
    let adopted = peer.ask("adopt");
    assert!(adopted["job"].is_string(), "{adopted}");
    peer.ask("block");
    let discarding = peer.ask("discard");
    assert_eq!(discarding["state"], "discarding", "{discarding}");
    assert_eq!(discarding["discardState"], "queued", "{discarding}");

    // The job has not run: the files are there and claimed by a holder that is alive, which is
    // what stops this process (or a recovery) from taking them.
    world.assert_intact("street");
    let held = claims_of(&world.workspace, "street");
    assert_eq!(held.len(), 2, "{held:?}");
    assert!(
        held.iter().all(|entry| entry["orphan"] == false),
        "the live process still owns the result: {held:?}"
    );
    let owners = claims_status(&world.workspace)["owners"].clone();
    assert!(
        owners
            .as_array()
            .unwrap()
            .iter()
            .any(|owner| owner["pid"] == peer.pid),
        "{owners}"
    );
    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.report.removed_files, 0, "{recovery:?}");
    world.assert_intact("street");

    // The job ends: it takes back what only this result created and releases every claim.
    let ended = peer.ask("release");
    assert_eq!(ended["ended"]["state"], "succeeded", "{ended}");
    assert_eq!(
        ended["ended"]["result"]["cleanup"]["removedFiles"], 2,
        "{ended}"
    );
    world.assert_gone("street");
    assert!(claims_of(&world.workspace, "street").is_empty());
    peer.quit();
    assert!(
        !world.workspace.join(".tessel-claims").exists(),
        "{:?}",
        tree(&world.workspace)
    );
}

#[test]
fn a_discard_never_removes_what_the_other_process_reused_in_the_meantime() {
    let world = world("shared");
    let mut peer = Peer::spawn(&world);
    peer.ask("adopt");
    peer.ask("block");
    peer.ask("discard");

    // Another process adopts the same scene while the discard is still queued: both files exist,
    // so it reuses them and claims them before anything is removed.
    let mine = adopt_into(&world.scene, "street", &world.workspace, &world.sandbox.src).unwrap();
    assert_eq!(mine.report.copied(), 0, "{:?}", mine.report);
    assert_eq!(mine.report.reused(), 2, "{:?}", mine.report);
    let mut mine = mine;
    let mut mine = mine.provisional();

    let ended = peer.ask("release");
    assert_eq!(ended["ended"]["state"], "succeeded", "{ended}");
    let cleanup = &ended["ended"]["result"]["cleanup"];
    assert_eq!(cleanup["removedFiles"], 0, "{ended}");
    // Both files and the three folders above them are listed as relied on by the other process.
    let shared = cleanup["shared"].as_array().unwrap();
    assert_eq!(shared.len(), 5, "{ended}");
    assert!(
        ["backdrop.png", "figure.png"].iter().all(|name| shared
            .iter()
            .any(|line| line.as_str().unwrap().contains(name))),
        "{ended}"
    );
    world.assert_intact("street");

    // Accepted by this process: the files are the project's, whatever happens next.
    mine.accept().unwrap();
    peer.quit();
    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.report.removed_files, 0, "{recovery:?}");
    world.assert_intact("street");
    assert!(claims_of(&world.workspace, "street").is_empty());
}

// ------------------------------------------------------------------------------------ SIGKILL --

#[test]
fn killing_the_process_with_a_queued_discard_leaves_orphans_that_recover_takes_back() {
    let world = world("kill-queued");
    let bystander = world.bystander();
    let mut peer = Peer::spawn(&world);
    peer.ask("adopt");
    peer.ask("block");
    peer.ask("discard");
    peer.crash();

    // Nothing ran in the victim: the files are there and listed as orphans, not accepted.
    world.assert_intact("street");
    let orphans = claims_of(&world.workspace, "street");
    assert_eq!(orphans.len(), 2, "{orphans:?}");
    assert!(orphans.iter().all(|entry| entry["orphan"] == true));
    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.report.removed_files, 2, "{recovery:?}");
    world.assert_gone("street");
    // What a live holder (this process's bystander) owns is not touched.
    world.assert_intact("bystander");
    drop(bystander);
}

/// A world whose backdrop is a valid PNG followed by `extra` bytes of padding (the transport only
/// reads the header): hashing it takes long enough to be interrupted on purpose.
fn big_world(name: &str, extra: usize) -> World {
    let sandbox = Sandbox::new(&format!("te-discard-claims-{name}"));
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let mut document = fixture();
    let mut big = png(64, 36, 10);
    big.resize(big.len() + extra, 0);
    let backdrop = sandbox.source_file("assets/backdrop.png", &big);
    let figure = sandbox.source_file("assets/figure.png", &png(32, 32, 20));
    restate(&mut document, "backdrop", "assets/backdrop.png", &backdrop);
    restate(&mut document, "figure", "assets/figure.png", &figure);
    let bundle = sandbox.root.join("bundle.json");
    fs::write(&bundle, document.to_string()).unwrap();
    World {
        scene: scene(document),
        sandbox,
        workspace,
        bundle,
    }
}

/// F1 (BL094.4). The discard of a result takes the claim out of the ledger only **after** the file
/// is proven and removed. Killed while it hashes a large file, the process leaves the claim
/// behind (as an orphan, because the lease dies with it) and `recover` takes the file back; the
/// earlier order dropped the claim first and left a file nobody listed, forever.
#[test]
fn killing_the_process_while_its_discard_hashes_a_large_file_leaves_a_claim_recover_takes_back() {
    // 128 MiB: well over a second to hash in debug, still several hundred ms in release, against
    // the 150 ms the test waits after the job reports that it started.
    let world = big_world("kill-hashing", 128 << 20);
    let mut peer = Peer::spawn(&world);
    peer.ask("adopt");
    let discarding = peer.ask("discard");
    assert_eq!(discarding["state"], "discarding", "{discarding}");
    assert_eq!(peer.ask("hashing")["hashing"], true);
    std::thread::sleep(Duration::from_millis(150));
    peer.crash();

    let [backdrop, _figure] = world.files("street");
    assert!(
        backdrop.is_file(),
        "the large file was still being hashed: it is still there"
    );
    // Whatever is still on disk is listed, as an orphan: nothing is a file the ledger does not know.
    let orphans = claims_of(&world.workspace, "street");
    for file in world.files("street") {
        if file.exists() {
            let canonical = fs::canonicalize(&file).unwrap();
            assert!(
                orphans
                    .iter()
                    .any(|entry| entry["path"].as_str().map(Path::new) == Some(&canonical)),
                "`{}` exists but is not claimed: {orphans:?} {:?}",
                file.display(),
                tree(&world.workspace)
            );
        }
    }
    assert!(orphans.iter().all(|entry| entry["orphan"] == true));
    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert!(recovery.report.removed_files >= 1, "{recovery:?}");
    world.assert_gone("street");
    assert!(
        claims_of(&world.workspace, "street").is_empty(),
        "{:?}",
        tree(&world.workspace)
    );
}

#[test]
fn a_result_the_window_accepted_is_never_recovered_after_the_process_is_killed() {
    let world = world("kill-accepted");
    let bystander = world.bystander();
    let mut peer = Peer::spawn(&world);
    peer.ask("adopt");
    // The window's tick applies the result: the project still fits, so the claims leave the
    // ledger before the tick returns.
    let polled = peer.ask("poll");
    let events = polled["events"].as_array().unwrap();
    assert!(
        events.iter().any(|event| event["state"] == "succeeded"),
        "{polled}"
    );
    assert!(claims_of(&world.workspace, "street").is_empty());
    peer.crash();

    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.report.removed_files, 0, "{recovery:?}");
    world.assert_intact("street");
    drop(bystander);
}

// -------------------------------------------------------------------- refused acceptance --

#[test]
fn a_refused_acceptance_goes_to_a_discard_job_that_still_owns_the_claims() {
    let world = world("refused");
    let bystander = world.bystander();
    let mut peer = Peer::spawn(&world);
    peer.ask("adopt");
    peer.ask("block");
    block_writes(&world.workspace);

    let polled = peer.ask("poll");
    let failed = polled["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["state"] == "failed")
        .unwrap_or_else(|| panic!("the refused result is reported as failed: {polled}"))
        .clone();
    assert_eq!(failed["error"]["code"], "claims_unavailable", "{failed}");
    let details = &failed["error"]["details"];
    assert_eq!(details["phase"], "accept", "{failed}");
    assert!(details["discard_job"].is_string(), "{failed}");

    // Not accepted, not removed: the discard job is queued and the result still owns both.
    world.assert_intact("street");
    unblock_writes(&world.workspace);
    let held = claims_of(&world.workspace, "street");
    assert_eq!(held.len(), 2, "still claimed: {held:?}");
    assert!(
        held.iter().all(|entry| entry["orphan"] == false),
        "{held:?}"
    );
    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.report.removed_files, 0, "{recovery:?}");

    // The ledger works again: the job removes what only this result created.
    let discard = details["discard_job"].as_str().unwrap();
    peer.ask("release_only");
    let ended = poll_peer_until(&mut peer, discard);
    assert_eq!(ended["state"], "succeeded", "{ended}");
    assert_eq!(ended["result"]["cleanup"]["removedFiles"], 2, "{ended}");
    world.assert_gone("street");
    world.assert_intact("bystander");
    peer.quit();
    drop(bystander);
}

fn poll_peer_until(peer: &mut Peer, job: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let polled = peer.ask("poll");
        if let Some(event) = polled["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|event| event["job"] == job)
        {
            return event.clone();
        }
        assert!(Instant::now() < deadline, "`{job}` was never reported");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_refused_result_whose_process_is_killed_is_an_orphan_the_ledger_can_recover_once_writable() {
    let world = world("refused-kill");
    let bystander = world.bystander();
    let mut peer = Peer::spawn(&world);
    peer.ask("adopt");
    peer.ask("block");
    block_writes(&world.workspace);
    let polled = peer.ask("poll");
    assert!(
        polled["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["error"]["code"] == "claims_unavailable"),
        "{polled}"
    );
    peer.crash();
    unblock_writes(&world.workspace);

    // The refusal was not an acceptance: the claims are still the dead process's, so they are
    // orphans and `recover` (and only it) takes the unaccepted files back.
    world.assert_intact("street");
    let orphans = claims_of(&world.workspace, "street");
    assert!(
        orphans.iter().all(|entry| entry["orphan"] == true),
        "{orphans:?}"
    );
    let recovery = recover(&world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.report.removed_files, 2, "{recovery:?}");
    world.assert_gone("street");
    world.assert_intact("bystander");
    drop(bystander);
}

fn signal(name: &str, pid: u32) {
    let status = Command::new("kill")
        .arg(format!("-{name}"))
        .arg(pid.to_string())
        .status()
        .unwrap();
    assert!(status.success(), "kill -{name} {pid}");
}

/// BL112.3, hypothesis confirmed and fixed. The window's tick accepts a result, which writes the
/// ledger under its `flock`. Another process stopped (`SIGSTOP`) while it holds that lock kept the
/// tick waiting for as long as it stayed stopped. A first fix bounded the wait at 1.5 s, which is
/// still above the tick's 100 ms ceiling, so the tick now waits a **fixed 20 ms** (`Ledger::try_lock`,
/// enough to ride out a holder of this very process, see `claims/tests.rs`), finds the lock
/// still held, and the ledger is reported unavailable. The
/// acceptance is refused like any other ledger failure (nothing applied, the result still claimed
/// and handed to a discard job) and the tick comes back within that budget. The bounded wait remains for
/// callers that may block (jobs, the CLI).
#[test]
fn a_peer_stopped_while_holding_the_ledger_lock_cannot_freeze_the_tick() {
    let world = world("sigstop");
    let mut state = project_with_scene(&world.sandbox, &world.workspace, &world.bundle);
    let mut host = MediaHost::new().interactive();
    let registry = host.registry().clone();
    let started = run(
        &mut state,
        &mut host,
        json!({"op":"scene.resources.adopt","scene":"street","source":world.sandbox.src}),
    )
    .unwrap();
    let job = started["job"].as_str().unwrap().to_string();
    wait_final(&registry, &job);
    let (summary, revision) = (state.summary(), state.project.revision);

    let mut peer = Peer::spawn(&world);
    assert_eq!(peer.ask("hold_ledger")["held"], true);
    let pid = peer.pid;
    signal("STOP", pid);
    // Without the bound the tick below never returns; the watchdog resumes the peer after 20 s so
    // that a regression fails the assertion on the elapsed time instead of hanging the suite.
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let finished = done.clone();
    let watchdog = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if finished.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = Command::new("kill")
            .args(["-CONT", &pid.to_string()])
            .status();
    });
    let blockers = Blockers::hold(&registry);

    let tick = Instant::now();
    let events = host.poll(&mut state);
    let elapsed = tick.elapsed();
    eprintln!(
        "BUDGET {}",
        json!({"measure": "tick-sigstop-peer", "pollMs": elapsed.as_secs_f64() * 1000.0,
            "peerPid": pid, "ceilingMs": 100})
    );
    // Not the 1.5 s of the bounded wait either: the tick waits its fixed 20 ms budget and no more,
    // and the whole tick stays under the UI's 100 ms ceiling (not a loose second).
    assert!(
        elapsed < Duration::from_millis(100),
        "the tick was held for {elapsed:?} by a stopped peer"
    );
    let failed = events
        .iter()
        .find(|event| event["job"] == job.as_str())
        .unwrap_or_else(|| panic!("the refused result is reported: {events:?}"));
    assert_eq!(failed["state"], "failed", "{failed}");
    assert_eq!(failed["error"]["code"], "claims_unavailable", "{failed}");
    let details = &failed["error"]["details"];
    assert_eq!(details["phase"], "accept", "{failed}");
    assert!(
        details["why"]
            .as_str()
            .is_some_and(|why| why.contains("gave up after") && !why.contains("another process")),
        "the reason is the tick's budget on a lock that stayed held, without blaming anyone: {failed}"
    );
    // Nothing was applied, and the result is still owned (the discard job is queued).
    assert_eq!(state.summary(), summary);
    assert_eq!(state.project.revision, revision);
    let discard = details["discard_job"].as_str().unwrap().to_string();
    assert_eq!(registry.get(&discard).unwrap().state, JobState::Queued);
    world.assert_intact("street");

    // The peer comes back and dies: the lock is free, the discard job removes what is ours.
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    watchdog.join().unwrap();
    signal("CONT", pid);
    peer.crash();
    blockers.release();
    let ended = poll_until_reported(&mut host, &mut state, &discard);
    assert_eq!(ended["state"], "succeeded", "{ended}");
    assert_eq!(ended["result"]["cleanup"]["removedFiles"], 2, "{ended}");
    world.assert_gone("street");
}

// ----------------------------------------------------- in-process: the import and the CLI --

struct ImportWorld {
    world: World,
    state: EditorState,
    host: MediaHost,
    source_document: PathBuf,
}

fn import_world(name: &str, host: MediaHost) -> ImportWorld {
    let world = world(name);
    let mut state = EditorState::demo();
    headless::execute(&mut state, &json!({"op":"project.create","name":"host"})).unwrap();
    persist::save(&mut state, &world.workspace.join("host.tessel.json")).unwrap();
    // The source is a saved project too: `project.import_scenes` reads a document.
    let mut source = EditorState::demo();
    headless::execute(&mut source, &json!({"op":"project.create","name":"source"})).unwrap();
    headless::execute(
        &mut source,
        &json!({"op":"scene.attach","id":"street","path":world.bundle}),
    )
    .unwrap();
    let source_document = world.sandbox.root.join("source/source.tessel.json");
    fs::create_dir_all(source_document.parent().unwrap()).unwrap();
    persist::save(&mut source, &source_document).unwrap();
    ImportWorld {
        world,
        state,
        host,
        source_document,
    }
}

impl ImportWorld {
    fn start_import(&mut self) -> String {
        let bases: BTreeMap<&str, &Path> =
            BTreeMap::from([("street", self.world.sandbox.src.as_path())]);
        let started = run(
            &mut self.state,
            &mut self.host,
            json!({"op":"project.import_scenes","source":self.source_document,
                "scene_bases":bases,"wait":false}),
        )
        .unwrap();
        started["job"].as_str().unwrap().to_string()
    }
}

#[test]
fn a_window_whose_import_acceptance_is_refused_attaches_nothing_and_discards_in_a_job() {
    let mut import = import_world("import-refused", MediaHost::new().interactive());
    let job = import.start_import();
    let registry = import.host.registry().clone();
    wait_final(&registry, &job);
    let bystander = import.world.bystander();
    let (summary, revision, can_undo) = (
        import.state.summary(),
        import.state.project.revision,
        import.state.history.can_undo(),
    );
    let blockers = Blockers::hold(&registry);
    block_writes(&import.world.workspace);

    let event = poll_until_reported(&mut import.host, &mut import.state, &job);
    assert_eq!(event["state"], "failed", "{event}");
    assert_eq!(event["error"]["code"], "claims_unavailable", "{event}");
    let details = &event["error"]["details"];
    assert_eq!(details["phase"], "accept", "{event}");
    let discard = details["discard_job"].as_str().unwrap().to_string();

    // The project is exactly as it was: no scene, no undo step, no new revision.
    assert!(import.state.scenes.is_empty());
    assert_eq!(import.state.summary(), summary);
    assert_eq!(import.state.project.revision, revision);
    assert_eq!(import.state.history.can_undo(), can_undo);
    // The files stay, still claimed by the result the discard job now owns; it has not started.
    import.world.assert_intact("street");
    assert_eq!(
        registry.get(&job).unwrap().result_state,
        ResultState::Discarding
    );
    assert_eq!(registry.get(&discard).unwrap().state, JobState::Queued);
    unblock_writes(&import.world.workspace);
    let held = claims_of(&import.world.workspace, "street");
    assert_eq!(held.len(), 2, "{held:?}");
    let recovery = recover(&import.world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.report.removed_files, 0, "{recovery:?}");

    blockers.release();
    let ended = poll_until_reported(&mut import.host, &mut import.state, &discard);
    assert_eq!(ended["state"], "succeeded", "{ended}");
    assert_eq!(ended["result"]["cleanup"]["removedFiles"], 2, "{ended}");
    import.world.assert_gone("street");
    import.world.assert_intact("bystander");
    assert!(import.state.scenes.is_empty());
    drop(bystander);
}

#[test]
fn a_window_whose_import_acceptance_is_written_attaches_and_keeps_the_files_for_good() {
    let mut import = import_world("import-ok", MediaHost::new().interactive());
    let job = import.start_import();
    let registry = import.host.registry().clone();
    wait_final(&registry, &job);
    let bystander = import.world.bystander();

    let event = poll_until_reported(&mut import.host, &mut import.state, &job);
    assert_eq!(event["state"], "succeeded", "{event}");
    assert_eq!(import.state.scenes.len(), 1);
    assert!(import.state.history.can_undo());
    assert!(claims_of(&import.world.workspace, "street").is_empty());
    let recovery = recover(&import.world.workspace, &CancelToken::default()).unwrap();
    assert_eq!(recovery.report.removed_files, 0, "{recovery:?}");
    import.world.assert_intact("street");
    drop(bystander);
}

#[test]
fn the_blocking_commit_of_a_cli_keeps_the_refusal_code_and_changes_nothing() {
    let mut import = import_world("cli-refused", MediaHost::new());
    let job = import.start_import();
    let waited = run(
        &mut import.state,
        &mut import.host,
        json!({"op":"job.wait","job":job}),
    )
    .unwrap();
    assert_eq!(waited["state"], "succeeded", "{waited}");
    let bystander = import.world.bystander();
    let (summary, revision) = (import.state.summary(), import.state.project.revision);
    block_writes(&import.world.workspace);

    let error = run(
        &mut import.state,
        &mut import.host,
        json!({"op":"job.commit","job":job}),
    )
    .expect_err("an acceptance nobody could write down must not apply the import");
    assert_eq!(error.code, "claims_unavailable", "{error:?}");
    assert_eq!(
        error.details.as_ref().unwrap()["phase"],
        "accept",
        "{error:?}"
    );
    assert!(import.state.scenes.is_empty());
    assert_eq!(import.state.summary(), summary);
    assert_eq!(import.state.project.revision, revision);
    // Nothing is lost by the refusal: the files were kept, and are still claimed by this process.
    import.world.assert_intact("street");
    unblock_writes(&import.world.workspace);
    let held = claims_of(&import.world.workspace, "street");
    assert_eq!(held.len(), 2, "{held:?}");
    assert!(held.iter().all(|entry| entry["orphan"] == false));
    drop(bystander);
}
