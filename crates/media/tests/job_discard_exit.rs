//! F2 (BL094.5, BL112): the CLI does not leave while a `job.discard {wait:false}` is still taking
//! files back, through the **real `tessel-media` process** (JSON Lines on stdin/stdout).
//!
//! The session adopts a scene with `wait:false` (the result waits in its memory), discards it with
//! `wait:false` (the answer is `discarding`, the work runs on a job thread) and ends its input. The
//! earlier host dropped without waiting, and `main` returned (or called `process::exit(1)` after a
//! refused line) while the job was still hashing: the files stayed and so did `.tessel-claims`.
//! The backdrop is a valid PNG followed by 64 MiB of padding (the transport only reads the header),
//! so the removal takes seconds in debug: the process cannot win the race by luck.
//!
//! Not proven here: the window (it never waits, its limit is documented and the claim is kept
//! until the file is gone, see `job_discard_claims.rs`).

mod directed_resources_common;

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use directed_resources_common::*;
use serde_json::{Value, json};

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

    fn reply(&mut self, request: Value) -> Value {
        let stdin = self.stdin.as_mut().expect("the session is still open");
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        match self.lines.recv_timeout(Duration::from_secs(120)) {
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

    /// EOF on stdin, then the exit code of the process once it has really ended.
    fn end(mut self) -> Option<i32> {
        drop(self.stdin.take());
        self.child.wait().unwrap().code()
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
        let sandbox = Sandbox::new(&format!("te-discard-exit-{name}"));
        let workspace = sandbox.root.join("ws");
        fs::create_dir_all(&workspace).unwrap();
        let mut document = fixture();
        let mut big = png(64, 36, 10);
        big.resize(big.len() + (64 << 20), 0);
        let backdrop = sandbox.source_file("assets/backdrop.png", &big);
        let figure = sandbox.source_file("assets/figure.png", &png(32, 32, 20));
        restate(&mut document, "backdrop", "assets/backdrop.png", &backdrop);
        restate(&mut document, "figure", "assets/figure.png", &figure);
        let bundle = sandbox.root.join("bundle.json");
        fs::write(&bundle, document.to_string()).unwrap();
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

    fn ledger(&self) -> PathBuf {
        self.workspace.join(".tessel-claims")
    }

    /// A session with the project saved, the scene attached and an adoption nobody applied: the
    /// result waits in the process's memory. Returns the session and the adoption job.
    fn adopted(&self) -> (Session, String) {
        let mut session = Session::spawn();
        session.ok(json!({"op":"project.save","path":self.project}));
        session.ok(json!({"op":"scene.attach","id":"street","path":self.bundle}));
        session.ok(json!({"op":"project.save"}));
        let started = session.ok(json!({"op":"scene.resources.adopt","scene":"street",
            "source":self.sandbox.src,"wait":false}));
        let job = started["job"].as_str().unwrap().to_string();
        let done = session.ok(json!({"op":"job.wait","job":job}));
        assert_eq!(done["state"], "succeeded", "{done}");
        assert!(self.backdrop().is_file(), "{:?}", tree(&self.workspace));
        (session, job)
    }
}

fn nothing_left(world: &World) {
    let left: Vec<String> = tree(&world.workspace)
        .into_iter()
        .filter(|path| path.starts_with("scenes") || path.starts_with(".tessel-claims"))
        .collect();
    assert!(
        left.is_empty(),
        "the process ended with the discard unfinished: {left:?}"
    );
    assert!(!Path::new(&world.ledger()).exists());
}

#[test]
fn a_discard_without_waiting_is_finished_before_the_process_leaves_at_end_of_input() {
    let world = World::new("eof");
    let (mut session, job) = world.adopted();
    let answer = session.ok(json!({"op":"job.discard","job":job,"wait":false}));
    assert_eq!(answer["state"], "discarding", "{answer}");
    // The work is running on a job thread: the caller got its answer, the files are not back yet.
    assert!(
        world.backdrop().is_file(),
        "the answer came before the removal ended"
    );
    assert_eq!(session.end(), Some(0));
    nothing_left(&world);
}

#[test]
fn a_session_that_had_a_refused_line_still_waits_for_the_discard_before_exiting_with_one() {
    let world = World::new("refused-line");
    let (mut session, job) = world.adopted();
    session.ok(json!({"op":"job.discard","job":job,"wait":false}));
    // A refused line makes the process exit with 1 through `process::exit`, which skips
    // destructors: the host must already have been dropped (and waited) by then.
    let refused = session.reply(json!({"op":"job.get","job":"job-404"}));
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(session.end(), Some(1));
    nothing_left(&world);
}
