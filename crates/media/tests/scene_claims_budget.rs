//! BL112.3 (F025/F040): what the window's tick costs when it accepts a result against a **real
//! ledger on disk** that is large, measured through the real `MediaHost::poll`.
//!
//! The tick accepts a finished adoption on the window's thread: it opens `<workspace>/.tessel-claims`
//! (reads and parses `claims.json`, reaps dead leases), removes the result's entries and writes the
//! file again. That is **not** zero I/O; the question is whether it stays under the 100 ms ceiling
//! of the UI when the ledger has 1 000 and 10 000 entries. Nothing here is a model of the tick:
//! the number is the elapsed time of `MediaHost::poll`, which applies the result through
//! `PendingAdoption::settle` and `Ledger::try_lock` exactly as the window does.
//!
//! Method (what is real and what is seeded):
//!
//! * The ledger is seeded **once**, by writing `claims.json` in the format the ledger itself writes
//!   (`version 1`: owners and entries with kind, root, stamp and holders), because creating `N`
//!   entries through the tracker rewrites the whole file for each one (`O(N²)`: about 20 GB of
//!   writes for 10 000). Every entry names a **real small file** with its real length, device,
//!   inode and SHA-256, held by an owner whose lease (`live/<token>`, an exclusive `flock`) is
//!   held by a **real second process** (this test binary re-executed), so the ledger reads as a
//!   live peer's claims, never as orphans.
//! * The seed is verified by a **third process**: the real `tessel-media` CLI opens the project
//!   and answers `scene.resources.claims` with the same number of entries, and again after the
//!   measurement. The host under test uses the very same ledger directory (the test asserts the
//!   entry count through `claims_status` before and after each tick), not a parallel state.
//! * Three replicas per size, each a different scene adopted by a real job, then one `poll`.
//!   The control is the same flow with an empty ledger (only the result's own two entries).
//!
//! Gaps, stated: the tick of a window also paints and the proofs of the native window measure
//! only the latency of requests (`scripts/job-discard-proof.py`); the seeded entries are one
//! holder's, so reaping looks at one extra owner; three replicas are a bounded sample, not a
//! distribution; a loaded machine moves the numbers (compare with the control before blaming the
//! code). The ceiling is **not** relaxed to pass.
//!
//! The tests are `#[ignore]`d with a reason: they are measurements, run on purpose under the GPU
//! lock (`TESSEL_BUDGET_LOG=<file>` also appends one JSON object per sample):
//! `cargo test -p tessel-editor-media --test scene_claims_budget -- --ignored --nocapture --test-threads=1`.
#![cfg(unix)]

mod directed_resources_common;

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use directed_resources_common::*;
use serde_json::{Map, Value, json};
use tessel_editor_core::{EditorState, headless, persist};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::scene_resources::claims_status;

const ROLE: &str = "TESSEL_BUDGET_ROLE";
const PROTOCOL: &str = "@@ ";
/// The UI's ceiling for one tick. Fixed before any measurement; never raised to make a run pass.
const CEILING: Duration = Duration::from_millis(100);
const REPLICAS: usize = 3;
/// The token of the seeded owner (never zero, never this process's).
const OWNER: u128 = 0x5eed_0b5e_55ed_1ed9_0000_0000_0000_0001;

/// One JSON object per sample, on stderr and (when asked) appended to `TESSEL_BUDGET_LOG`.
fn emit(record: Value) {
    eprintln!("BUDGET {record}");
    if let Some(path) = std::env::var_os("TESSEL_BUDGET_LOG")
        && let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = writeln!(file, "{record}");
    }
}

/// Nearest-rank quantile of an ascending list of milliseconds.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (sorted.len() as f64 * q).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn millis(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1000.0
}

// ------------------------------------------------------------------------- the second process --

/// The command loop of the second process: it only holds leases for as long as it lives (an
/// exclusive `flock` on a file nobody else can lock), which is what makes the seeded owner alive.
/// Runs only when re-executed by [`Peer::spawn`]; as a plain test it returns at once.
#[test]
fn child_role() {
    if std::env::var(ROLE).is_err() {
        return;
    }
    let say = |value: Value| {
        println!("{PROTOCOL}{value}");
        std::io::stdout().flush().unwrap();
    };
    let mut held = Vec::new();
    say(json!({"event": "ready", "pid": std::process::id()}));
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let (command, argument) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        match command {
            "lease" => {
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(argument)
                    .unwrap();
                file.try_lock().unwrap();
                held.push(file);
                say(json!({"leased": true}));
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
    fn spawn() -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["child_role", "--exact", "--nocapture", "--test-threads=1"])
            .env(ROLE, "1")
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
}

impl Drop for Peer {
    /// `SIGKILL`: the lease goes with the process, whatever the test did.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ------------------------------------------------------------------------------ the seed --

/// Writes the ledger of `workspace` with `count` entries of `OWNER`, each over a real small file,
/// and has the peer hold the owner's lease. One write, in the ledger's own format.
fn seed_ledger(workspace: &Path, peer: &mut Peer, count: usize) {
    let ledger = workspace.join(".tessel-claims");
    fs::create_dir_all(ledger.join("live")).unwrap();
    fs::File::create(ledger.join("ledger.lock")).unwrap();
    let owner = format!("{OWNER:032x}");
    let lease = ledger.join("live").join(&owner);
    assert_eq!(
        peer.ask(&format!("lease {}", lease.display()))["leased"],
        true
    );
    let root = workspace.join("seeded/assets");
    fs::create_dir_all(&root).unwrap();
    let holder = format!("{owner}:1");
    let mut entries = Map::new();
    for index in 0..count {
        let bytes = format!("budget seed file {index}").into_bytes();
        let path = root.join(format!("f{index}.bin"));
        fs::write(&path, &bytes).unwrap();
        let metadata = fs::metadata(&path).unwrap();
        entries.insert(
            path.to_str().unwrap().to_string(),
            json!({
                "kind": "file",
                "root": root.to_str().unwrap(),
                "stamp": {
                    "len": metadata.len(),
                    "device": metadata.dev(),
                    "inode": metadata.ino(),
                    "sha256": sha256(&bytes),
                },
                "holders": [holder.clone()],
            }),
        );
    }
    let document = json!({
        "version": 1,
        "owners": { owner: { "pid": peer.pid, "since": 1 } },
        "entries": entries,
    });
    fs::write(
        ledger.join("claims.json"),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
}

fn ledger_bytes(workspace: &Path) -> u64 {
    fs::metadata(workspace.join(".tessel-claims/claims.json")).map_or(0, |meta| meta.len())
}

/// `(every entry, the entries that are files)` the ledger of `workspace` lists right now. A
/// result also claims the folders it created, so the first number is larger than the second by
/// those.
fn entry_counts(workspace: &Path) -> (usize, usize) {
    let status = claims_status(workspace);
    let entries = status["entries"].as_array().unwrap();
    let files = entries
        .iter()
        .filter(|entry| entry["kind"] == "file")
        .count();
    (entries.len(), files)
}

/// `scene.resources.claims` as a **third process** (the real `tessel-media` CLI over JSON Lines)
/// answers it for the project of `project`: what a second editor sees in the same ledger.
fn claims_from_another_process(project: &Path) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tessel-media"))
        .arg("--empty")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut ask = |request: Value| -> Value {
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["ok"], true, "{request} -> {reply}");
        reply["result"].clone()
    };
    ask(json!({"op": "project.open", "path": project}));
    let claims = ask(json!({"op": "scene.resources.claims"}));
    drop(stdin);
    assert!(child.wait().unwrap().success());
    claims
}

// ------------------------------------------------------------------------ the measurement --

fn measure(count: usize) {
    let sandbox = Sandbox::new(&format!("te-budget-{count}"));
    let workspace = sandbox.root.join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let workspace = fs::canonicalize(&workspace).unwrap();
    let mut state = EditorState::demo();
    headless::execute(
        &mut state,
        &json!({"op": "project.create", "name": "budget"}),
    )
    .unwrap();
    let project = workspace.join("p.tessel.json");
    persist::save(&mut state, &project).unwrap();
    let mut host = MediaHost::new().interactive();

    // One scene per replica, each with files of its own so every adoption really copies.
    let mut scenes = Vec::new();
    for replica in 0..REPLICAS {
        let id = format!("r{replica}");
        let base = sandbox.root.join("authored").join(&id);
        let shade = 10 + 50 * u8::try_from(replica).unwrap();
        let mut bundle = fixture();
        let backdrop = write(&base, "assets/backdrop.png", &png(64, 36, shade));
        let figure = write(&base, "assets/figure.png", &png(32, 32, shade + 1));
        restate(&mut bundle, "backdrop", "assets/backdrop.png", &backdrop);
        restate(&mut bundle, "figure", "assets/figure.png", &figure);
        let path = sandbox
            .root
            .join("bundles")
            .join(format!("{id}.bundle.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();
        headless::execute_ext(
            &mut state,
            None,
            Some(&mut host),
            &json!({"op": "scene.attach", "id": id, "path": path}),
        )
        .unwrap();
        scenes.push((id, base));
    }
    persist::save(&mut state, &project).unwrap();

    // The seed: written once, held by a real second process, checked by a third.
    let mut peer = None;
    if count > 0 {
        let mut holder = Peer::spawn();
        seed_ledger(&workspace, &mut holder, count);
        let claims = claims_from_another_process(&project);
        assert_eq!(claims["available"], true, "{claims}");
        assert_eq!(claims["entries"].as_array().unwrap().len(), count);
        assert_eq!(
            claims["orphans"], 0,
            "the peer's lease is held: not orphans"
        );
        assert_eq!(claims["reaped"].as_array().unwrap().len(), 0, "{claims}");
        emit(
            json!({"measure": "tick-seed-verified-by-third-process", "entries": count,
            "ledgerBytes": ledger_bytes(&workspace), "peerPid": holder.pid}),
        );
        peer = Some(holder);
    }

    let mut samples = Vec::new();
    for (id, base) in &scenes {
        let started = headless::execute_ext(
            &mut state,
            None,
            Some(&mut host),
            &json!({"op": "scene.resources.adopt", "scene": id, "source": base}),
        )
        .unwrap();
        assert_eq!(started["state"], "queued", "{started}");
        let job = started["job"].as_str().unwrap().to_string();
        host.registry()
            .wait(&job, Duration::from_secs(60))
            .expect("the adoption job ended");

        // The host acts on the very ledger the seed wrote: its own two entries sit beside it.
        let before = (entry_counts(&workspace), ledger_bytes(&workspace));
        assert_eq!(
            before.0.1,
            count + 2,
            "{id}: the result's two files join the seed"
        );
        let tick = Instant::now();
        let events = host.poll(&mut state);
        let elapsed = tick.elapsed();
        let event = events
            .iter()
            .find(|event| event["job"] == job.as_str())
            .unwrap_or_else(|| panic!("{id}: the tick applied the result: {events:?}"));
        assert_eq!(event["state"], "succeeded", "{event}");
        let after = (entry_counts(&workspace), ledger_bytes(&workspace));
        assert_eq!(
            after.0.0, count,
            "{id}: accepted, its files and folders are the project's now"
        );

        samples.push(millis(elapsed));
        emit(
            json!({"measure": "tick-accept", "entries": count, "replica": id,
            "pollMs": millis(elapsed), "entriesBefore": before.0.0,
            "ledgerBytesBefore": before.1, "ledgerBytesAfter": after.1, "ceilingMs": millis(CEILING)}),
        );
    }

    if peer.is_some() {
        let claims = claims_from_another_process(&project);
        assert_eq!(
            claims["entries"].as_array().unwrap().len(),
            count,
            "the third process sees what the host left in the ledger"
        );
    }
    let mut sorted = samples.clone();
    sorted.sort_by(f64::total_cmp);
    emit(
        json!({"measure": "tick-accept-summary", "entries": count, "replicas": samples.len(),
        "minMs": sorted[0], "p50Ms": quantile(&sorted, 0.5), "maxMs": sorted[sorted.len() - 1],
        "ceilingMs": millis(CEILING)}),
    );
    assert!(
        sorted[sorted.len() - 1] < millis(CEILING),
        "the tick accepted a result in {:.1} ms with {count} entries in the ledger: over the 100 ms \
         ceiling (compare the control before blaming load): {samples:?}",
        sorted[sorted.len() - 1]
    );
}

#[test]
#[ignore = "BL112.3 measurement (empty-ledger control); run on purpose under the GPU lock with --ignored"]
fn the_tick_accepting_a_result_in_an_empty_ledger_is_the_control() {
    measure(0);
}

#[test]
#[ignore = "BL112.3 measurement (1 000 real entries); run on purpose under the GPU lock with --ignored"]
fn the_tick_accepting_a_result_stays_under_its_ceiling_with_1000_entries() {
    measure(1_000);
}

#[test]
#[ignore = "BL112.3 measurement (10 000 real entries); run on purpose under the GPU lock with --ignored"]
fn the_tick_accepting_a_result_stays_under_its_ceiling_with_10000_entries() {
    measure(10_000);
}
