//! BL111.1 (F040): `scene.resources.claims` and `scene.resources.recover` through the **real
//! `tessel-media` process** (JSON Lines on stdin/stdout), with a second real process as the
//! crashed or the living holder.
//!
//! The victim is a `tessel-media` session that ran `scene.resources.adopt` with `wait: false`: the
//! adoption waits for `job.commit` in its memory, so its claims are held by that process alone.
//! `SIGKILL` (`Child::kill`) then leaves them as an orphan; nothing in the victim runs again. The
//! operator is another `tessel-media` session that opens the same project folder.
//!
//! What is proven: claims says who holds what and says it is not read-only (it reaps); recover
//! removes only the orphans, as a job with progress; a live process's holder is not recovered; a
//! file rewritten after the crash is kept; an unreadable ledger answers `available: false` and
//! makes recover fail with `claims_unavailable`, removing nothing; a session with no workspace
//! answers `no_workspace` and changes nothing.
//!
//! Not proven here: the window (`scripts/scene-claims-proof.py` drives it), Windows, network
//! filesystems, a cancellation arriving in the middle of a job (deterministic in the lib tests).

mod directed_resources_common;

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use directed_resources_common::*;
use serde_json::{Value, json};

/// One `tessel-media` process driven line by line.
struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Value>,
}

impl Session {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_tessel-media"))
            .arg("--empty")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                let _ = sender.send(serde_json::from_str(&line).unwrap());
            }
        });
        Self {
            child,
            stdin,
            lines,
        }
    }

    /// The raw reply of one request: `{ok, result}` or `{ok: false, error}`.
    fn reply(&mut self, request: Value) -> Value {
        let stdin = self.stdin.as_mut().expect("the session is still open");
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        match self.lines.recv_timeout(Duration::from_secs(60)) {
            Ok(value) => value,
            Err(RecvTimeoutError::Timeout) => panic!("tessel-media did not answer {request}"),
            Err(RecvTimeoutError::Disconnected) => panic!("tessel-media ended ({request})"),
        }
    }

    fn ok(&mut self, request: Value) -> Value {
        let reply = self.reply(request.clone());
        assert_eq!(reply["ok"], true, "{request} -> {reply}");
        reply["result"].clone()
    }

    fn error(&mut self, request: Value) -> Value {
        let reply = self.reply(request.clone());
        assert_eq!(reply["ok"], false, "{request} -> {reply}");
        reply["error"].clone()
    }

    /// `SIGKILL`: no destructor, no accept, no discard.
    fn crash(mut self) {
        self.child.kill().unwrap();
        assert!(!self.child.wait().unwrap().success());
    }

    /// EOF on stdin: the session ends the way a script does and drops what it still holds. The
    /// exit code is 1 when some line was refused (`error`), 0 otherwise.
    fn finish(mut self) {
        drop(self.stdin.take());
        assert!(self.child.wait().unwrap().success());
    }

    fn finish_after_refusals(mut self) {
        drop(self.stdin.take());
        assert_eq!(self.child.wait().unwrap().code(), Some(1));
    }
}

struct World {
    sandbox: Sandbox,
    workspace: PathBuf,
    project: PathBuf,
    bundle: PathBuf,
}

impl World {
    fn new(name: &str) -> Self {
        let sandbox = Sandbox::new(&format!("te-jsonl-{name}"));
        let workspace = sandbox.root.join("ws");
        fs::create_dir_all(&workspace).unwrap();
        let bundle = sandbox.root.join("bundle.json");
        fs::write(&bundle, standard_bundle(&sandbox).to_string()).unwrap();
        Self {
            project: workspace.join("p.tessel.json"),
            workspace,
            bundle,
            sandbox,
        }
    }

    fn backdrop(&self) -> PathBuf {
        self.workspace.join("scenes/street/assets/backdrop.png")
    }

    fn figure(&self) -> PathBuf {
        self.workspace.join("scenes/street/assets/figure.png")
    }

    /// A session that saved the project in the workspace and attached the scene, so it has a
    /// workspace and a bundle to adopt. Saved again with the scene: a second session opens it.
    fn author(&self) -> Session {
        let mut session = Session::spawn();
        session.ok(json!({"op":"project.save","path":self.project}));
        session.ok(json!({"op":"scene.attach","id":"street","path":self.bundle}));
        session.ok(json!({"op":"project.save"}));
        session
    }

    /// A session on the same project folder, as a second editor or a script would open it.
    fn operator(&self) -> Session {
        let mut session = Session::spawn();
        session.ok(json!({"op":"project.open","path":self.project}));
        session
    }

    /// Starts an adoption that nobody commits and waits for the job (not for an apply).
    fn adopt_pending(&self, session: &mut Session) {
        let started = session.ok(json!({"op":"scene.resources.adopt","scene":"street",
            "source":self.sandbox.src,"wait":false}));
        let job = started["job"].as_str().unwrap().to_string();
        let done = session.ok(json!({"op":"job.wait","job":job}));
        assert_eq!(done["state"], "succeeded", "{done}");
        assert!(self.backdrop().is_file() && self.figure().is_file());
    }

    fn ledger(&self) -> PathBuf {
        self.workspace.join(".tessel-claims")
    }
}

/// The result of a job the operator started without waiting, once it is final.
fn job_result(session: &mut Session, started: Value) -> Value {
    let job = started["job"].as_str().expect("a job").to_string();
    session.ok(json!({"op":"job.wait","job":job}))
}

#[test]
fn a_crashed_editors_files_are_reported_as_orphans_and_recovery_removes_only_them() {
    let world = World::new("crash");
    // What the workspace had before: nobody's to remove.
    fs::write(world.workspace.join("notes.txt"), b"mine").unwrap();
    let mut victim = world.author();
    let before_adopt = tree(&world.workspace);
    world.adopt_pending(&mut victim);
    victim.crash();
    let after_crash = tree(&world.workspace);
    assert_ne!(before_adopt, after_crash, "the victim left files behind");
    let resources_after_crash: Vec<String> = after_crash
        .iter()
        .filter(|path| !path.starts_with(".tessel-claims"))
        .cloned()
        .collect();

    let mut operator = world.operator();
    let claims = operator.ok(json!({"op":"scene.resources.claims"}));
    assert_eq!(claims["action"], "scene.resources.claims");
    assert_eq!(
        claims["readOnly"], false,
        "it reaps: it must not say read-only"
    );
    assert_eq!(claims["available"], true, "{claims}");
    assert_eq!(claims["reaped"].as_array().unwrap().len(), 1, "{claims}");
    assert!(claims["orphans"].as_u64().unwrap() >= 2, "{claims}");
    assert!(
        claims["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["orphan"] == true && entry["holders"] == json!([])),
        "{claims}"
    );
    // The ledger's own bookkeeping changes (the dead owner's lease file is reaped); no resource.
    let resources = |dir: &Path| -> Vec<String> {
        tree(dir)
            .into_iter()
            .filter(|path| !path.starts_with(".tessel-claims"))
            .collect()
    };
    assert_eq!(
        resources(&world.workspace),
        resources_after_crash,
        "claims removes no file"
    );

    // As a job: queued answer with a job id, then the final state with progress in files.
    let started = operator.ok(json!({"op":"scene.resources.recover","wait":false}));
    let job = job_result(&mut operator, started);
    assert_eq!(job["state"], "succeeded", "{job}");
    assert_eq!(
        (job["done"].as_u64(), job["total"].as_u64()),
        (Some(2), Some(2)),
        "{job}"
    );
    let recovery = &job["result"]["recovery"];
    assert_eq!(recovery["cleanup"]["removedFiles"], 2, "{job}");
    assert_eq!(recovery["cleanup"]["kept"], json!([]), "{job}");
    assert!(!world.backdrop().exists() && !world.figure().exists());
    assert_eq!(
        fs::read(world.workspace.join("notes.txt")).unwrap(),
        b"mine"
    );
    assert!(!world.ledger().exists(), "{:?}", tree(&world.workspace));

    // Idempotent: the second run has nothing to do.
    let again = operator.ok(json!({"op":"scene.resources.recover"}));
    assert_eq!(again["recovery"]["orphans"], 0, "{again}");
    operator.finish();
}

#[test]
fn recovery_leaves_alone_what_a_living_editor_still_holds() {
    let world = World::new("alive");
    let mut living = world.author();
    world.adopt_pending(&mut living);
    let mut operator = world.operator();

    let claims = operator.ok(json!({"op":"scene.resources.claims"}));
    assert_eq!(claims["orphans"], 0, "{claims}");
    assert_eq!(claims["owners"].as_array().unwrap().len(), 1, "{claims}");
    assert_eq!(claims["owners"][0]["thisProcess"], false, "{claims}");
    let recovery = operator.ok(json!({"op":"scene.resources.recover"}));
    assert_eq!(
        recovery["recovery"]["cleanup"]["removedFiles"], 0,
        "{recovery}"
    );
    assert!(world.backdrop().is_file() && world.figure().is_file());

    // The holder is still the living editor's: committing keeps the files for the project.
    let jobs = living.ok(json!({"op":"job.list"}));
    let job = jobs["jobs"][0]["id"].as_str().unwrap().to_string();
    living.ok(json!({"op":"job.commit","job":job}));
    assert!(world.backdrop().is_file() && world.figure().is_file());
    living.finish();
    assert!(
        world.backdrop().is_file(),
        "accepted files survive the session"
    );
    operator.finish();
}

#[test]
fn a_file_rewritten_after_the_crash_is_kept_and_reported_by_the_job() {
    let world = World::new("rewritten");
    let mut victim = world.author();
    world.adopt_pending(&mut victim);
    victim.crash();

    // Same inode, same length, other bytes: only the content tells it from the published file.
    let mut file = OpenOptions::new()
        .write(true)
        .open(world.backdrop())
        .unwrap();
    file.seek(SeekFrom::End(-4)).unwrap();
    file.write_all(b"edit").unwrap();
    drop(file);
    let edited = fs::read(world.backdrop()).unwrap();

    let mut operator = world.operator();
    let started = operator.ok(json!({"op":"scene.resources.recover","wait":false}));
    let job = job_result(&mut operator, started);
    assert_eq!(job["state"], "succeeded", "{job}");
    assert_eq!(
        job["result"]["recovery"]["cleanup"]["removedFiles"], 1,
        "{job}"
    );
    let warnings = job["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|why| why.as_str().unwrap().contains("rewritten in place")),
        "the job warns about what it kept: {job}"
    );
    assert_eq!(
        fs::read(world.backdrop()).unwrap(),
        edited,
        "the edit is kept"
    );
    assert!(!world.figure().exists());
    operator.finish();
}

#[test]
fn an_unreadable_ledger_is_reported_and_recovery_fails_closed_without_removing_anything() {
    let world = World::new("unreadable");
    let mut victim = world.author();
    world.adopt_pending(&mut victim);
    victim.crash();
    let ledger = world.ledger().join("claims.json");
    fs::write(&ledger, b"{ this is not a ledger").unwrap();
    let before = tree(&world.workspace);

    let mut operator = world.operator();
    let claims = operator.ok(json!({"op":"scene.resources.claims"}));
    assert_eq!(claims["available"], false, "{claims}");
    assert!(
        claims["fault"].as_str().unwrap().contains("claims ledger"),
        "{claims}"
    );
    // Waiting for the job answers `job_failed` with the original code in `details.job_code`.
    let error = operator.error(json!({"op":"scene.resources.recover"}));
    assert_eq!(error["code"], "job_failed", "{error}");
    assert_eq!(
        error["details"]["job_code"], "claims_unavailable",
        "{error}"
    );
    let started = operator.ok(json!({"op":"scene.resources.recover","wait":false}));
    let job = job_result(&mut operator, started);
    assert_eq!(job["state"], "failed", "{job}");
    assert_eq!(job["error"]["code"], "claims_unavailable", "{job}");
    assert_eq!(tree(&world.workspace), before, "nothing was removed");
    assert_eq!(
        fs::read(&ledger).unwrap(),
        b"{ this is not a ledger",
        "nor repaired"
    );
    operator.finish_after_refusals();
}

#[test]
fn a_session_without_a_workspace_refuses_both_operations_and_touches_nothing() {
    let world = World::new("no-workspace");
    let mut session = Session::spawn();
    for op in ["scene.resources.claims", "scene.resources.recover"] {
        let error = session.error(json!({"op": op}));
        assert_eq!(error["code"], "no_workspace", "{op}: {error}");
    }
    assert!(tree(&world.workspace).is_empty());
    session.finish_after_refusals();
}

#[test]
fn an_adoption_the_session_drops_unaccepted_leaves_no_claims_for_the_next_one() {
    // The orderly end of the same flow: EOF removes the pending result and its claims, so the
    // operator sees an empty ledger, not an orphan.
    let world = World::new("orderly");
    let mut session = world.author();
    let before = tree(&world.workspace);
    world.adopt_pending(&mut session);
    session.finish();
    assert_eq!(tree(&world.workspace), before);
    let mut operator = world.operator();
    let claims = operator.ok(json!({"op":"scene.resources.claims"}));
    assert_eq!(claims["orphans"], 0, "{claims}");
    assert_eq!(claims["owners"], json!([]), "{claims}");
    assert!(!Path::new(&world.ledger()).exists());
    operator.finish();
}
