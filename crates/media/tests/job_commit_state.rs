//! F4 (BL094.7): `result_state` says what the project did with a result, not whether the answer was
//! an error. An import that changed the project and then could not decode what it brought answers
//! with an error, but the result *was* taken (`applied`), and the error says so
//! (`details.applied`). It used to read `discarded`, which let a later `job.discard` believe there
//! was something to take back.
//!
//! Proved through the **real `tessel-media` process** (JSON Lines): the decode that follows a
//! `job.commit` discovers FFmpeg on its own, and the only way to make that fail on purpose, without
//! `unsafe` and without touching the environment of the test process, is the environment of a
//! child. `TESSEL_FFMPEG` points to a wrapper that works until a flag file exists, so the import
//! job (and its probes) run for real and the commit finds FFmpeg gone.
//!
//! The opposite case (a decoded or proxy result that is stale answers `Ok` and is `discarded`) is
//! in `async_host.rs`.

mod common;

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use common::{temp, tools, video_with_audio};
use serde_json::{Value, json};

struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Value>,
}

impl Session {
    fn spawn(ffmpeg: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_tessel-media"))
            .arg("--empty")
            .env("TESSEL_FFMPEG", ffmpeg)
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

    fn end(mut self) {
        drop(self.stdin.take());
        self.child.wait().unwrap();
    }
}

/// An `ffmpeg` that is the real one until `flag` exists, then a tool that is gone.
fn wrapper(dir: &Path, real: &Path, flag: &Path) -> std::path::PathBuf {
    let path = dir.join("flaky-ffmpeg");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nif [ -e '{}' ]; then echo 'forced failure' >&2; exit 1; fi\nexec '{}' \"$@\"\n",
            flag.display(),
            real.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn an_import_that_fails_to_decode_after_it_was_committed_is_applied_not_discarded() {
    let dir = temp("commit-state");
    let real = tools();
    let video = video_with_audio(&real, &dir, "clip.mp4");
    let flag = dir.join("fail-now");
    let mut session = Session::spawn(&wrapper(&dir, &real.ffmpeg, &flag));
    session.ok(
        json!({"op":"project.create","name":"commit-state","sequence":"main","rate":24,
        "width":320,"height":180,"length":48,"dir":dir.join("project")}),
    );
    let started = session.ok(json!({"op":"media.import","paths":[video],"wait":false}));
    let job = started["job"].as_str().unwrap().to_string();
    let waited = session.ok(json!({"op":"job.wait","job":job}));
    assert_eq!(waited["state"], "succeeded", "{waited}");
    let before = session.ok(json!({"op":"job.get","job":job}));
    assert_eq!(before["result_state"], "pending", "{before}");

    // FFmpeg is gone from here on: the import commits, the decoding that follows cannot start.
    fs::write(&flag, b"x").unwrap();
    let commit = session.reply(json!({"op":"job.commit","job":job}));
    assert_eq!(commit["ok"], false, "{commit}");
    assert_eq!(
        commit["error"]["details"]["applied"], true,
        "the error says the project changed: {commit}"
    );

    let after = session.ok(json!({"op":"job.get","job":job}));
    assert_eq!(
        after["result_state"], "applied",
        "the project took the result: {after}"
    );
    let discard = session.reply(json!({"op":"job.discard","job":job}));
    assert_eq!(discard["ok"], false, "{discard}");
    assert_eq!(discard["error"]["code"], "job_applied", "{discard}");
    session.end();
}
