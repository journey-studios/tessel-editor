//! BL113a on the real executable: `tessel-media` is started as a process and driven line by line
//! with JSON Lines on stdin, exactly like `./scripts/media`. Nothing here links the host, the
//! core or a window; what is proven is the contract of the binary.
//!
//! The race is built with the protocol's own barriers: `scene.attach.start` with `wait:false`
//! returns a job, `job.wait` returns only when that job has finished (its result is then held,
//! not applied), and `job.commit` applies it. No sleeps; the order of the lines is the order of
//! events. Between the barrier and the commit the project is closed and its own document is
//! opened again: same ID, revision back to 0.

use core::prelude::v1::test;

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command as Process, Stdio};

use serde_json::{Value, json};

struct Media {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Media {
    fn start() -> Self {
        let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-media"))
            .arg("--empty")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("the tessel-media binary starts");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    /// One request line in, the response line out.
    fn send(&mut self, request: Value) -> Value {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{request} answered {line:?}: {e}"))
    }

    fn ok(&mut self, request: Value) -> Value {
        let answer = self.send(request.clone());
        assert_eq!(answer["ok"], true, "{request} -> {answer}");
        answer["result"].clone()
    }

    fn error_code(&mut self, request: Value) -> String {
        let answer = self.send(request.clone());
        assert_eq!(answer["ok"], false, "{request} -> {answer}");
        answer["error"]["code"].as_str().unwrap().to_string()
    }

    /// `(id, revision, session)` as the process reports them.
    fn project(&mut self) -> (String, u64, u64) {
        let history = self.ok(json!({"op":"history.status"}));
        let project = &history["project"];
        (
            project["id"].as_str().unwrap().to_string(),
            project["revision"].as_u64().unwrap(),
            project["session"].as_u64().unwrap(),
        )
    }

    fn undo_depth(&mut self) -> u64 {
        self.ok(json!({"op":"history.status"}))["history"]["undo_depth"]
            .as_u64()
            .unwrap()
    }

    fn scenes(&mut self) -> Vec<String> {
        self.ok(json!({"op":"scene.list"}))["scenes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|scene| scene["id"].as_str().unwrap().to_string())
            .collect()
    }

    fn finish(mut self) -> Option<i32> {
        drop(self.stdin.take());
        self.child.wait().unwrap().code()
    }
}

/// A saved project (so there is a document to reopen) and a small valid bundle on disk.
fn setup(name: &str) -> (Media, PathBuf, PathBuf) {
    let dir =
        std::env::temp_dir().join(format!("tessel-attach-proc-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let bundle = dir.join("street.bundle.json");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/migration/fixtures/bundles/valid-minimal-continuous.bundle.json"),
        &bundle,
    )
    .unwrap();
    let document = dir.join("p.tessel.json");
    let mut media = Media::start();
    media.ok(json!({"op":"project.save","path":document}));
    (media, bundle, document)
}

fn start_held(media: &mut Media, bundle: &Path, id: &str) -> String {
    let started = media.ok(json!({"op":"scene.attach.start","path":bundle,"id":id,"wait":false}));
    let job = started["job"].as_str().unwrap().to_string();
    // The barrier: the job has finished and its result waits to be applied.
    let waited = media.ok(json!({"op":"job.wait","job":job}));
    assert_eq!(waited["state"], "succeeded", "{waited}");
    job
}

#[test]
fn a_job_finished_before_the_document_was_reopened_never_attaches_to_the_reopened_project() {
    let (mut media, bundle, document) = setup("reopen");
    let job = start_held(&mut media, &bundle, "late");
    let (id, revision, session) = media.project();

    // Close and reopen the same document, in the same process.
    let opened = media.ok(json!({"op":"project.open","path":document}));
    assert_eq!(opened["id"], id);
    let (reopened_id, reopened_revision, reopened_session) = media.project();
    assert_eq!(
        (&reopened_id, reopened_revision),
        (&id, revision),
        "the reopened project is indistinguishable by ID and revision"
    );
    assert_ne!(reopened_session, session, "but it is another session");

    assert_eq!(
        media.error_code(json!({"op":"job.commit","job":job})),
        "stale_result"
    );
    assert!(media.scenes().is_empty(), "nothing was attached");
    assert_eq!(media.undo_depth(), 0, "nothing can be undone");
    assert_eq!(
        media.error_code(json!({"op":"history.undo"})),
        "nothing_to_undo"
    );
    assert_eq!(media.project(), (id.clone(), revision, reopened_session));

    // Control: the same bundle attaches in the reopened session, as one undo step.
    let attached = media.ok(json!({"op":"scene.attach.start","path":bundle,"id":"late"}));
    assert_eq!(attached["applied"]["scene"], "late", "{attached}");
    assert_eq!(media.scenes(), ["late"]);
    assert_eq!(media.undo_depth(), 1);

    // A second reopen, same story: the process keeps refusing, and exits like any failed session.
    let second = start_held(&mut media, &bundle, "later");
    media.ok(json!({"op":"project.open","path":document}));
    assert_eq!(
        media.error_code(json!({"op":"job.commit","job":second})),
        "stale_result"
    );
    assert!(
        media.scenes().is_empty(),
        "the reopened project holds nothing"
    );
    assert_eq!(media.undo_depth(), 0);
    assert_eq!(media.finish(), Some(1), "failed requests exit nonzero");
}

#[test]
fn saving_querying_and_undoing_without_reopening_keeps_the_job_valid() {
    let (mut media, bundle, document) = setup("same-session");
    // History moves before the job starts; the session is the same one.
    media.ok(json!({"op":"scene.attach","path":bundle,"id":"first"}));
    media.ok(json!({"op":"history.undo"}));
    media.ok(json!({"op":"history.redo"}));
    let (_, _, session) = media.project();

    let job = start_held(&mut media, &bundle, "second");
    media.ok(json!({"op":"project.save","path":document}));
    assert_eq!(media.scenes(), ["first"]);
    media.ok(json!({"op":"scene.resources.status"}));
    assert_eq!(media.project().2, session, "the session did not change");

    let committed = media.ok(json!({"op":"job.commit","job":job}));
    assert_eq!(committed["applied"]["scene"], "second", "{committed}");
    assert_eq!(media.scenes(), ["first", "second"]);
    assert_eq!(media.undo_depth(), 2, "one undo entry for the job");
    media.ok(json!({"op":"history.undo"}));
    assert_eq!(media.scenes(), ["first"]);
    assert_eq!(media.finish(), Some(0), "every request succeeded");
}

#[test]
fn an_edit_while_the_job_waits_is_still_stale_in_the_same_session() {
    // BL113 is unchanged: any registered edit makes the result stale, session or not.
    let (mut media, bundle, _) = setup("edit");
    let job = start_held(&mut media, &bundle, "late");
    media.ok(json!({"op":"scene.attach","path":bundle,"id":"first"}));
    assert_eq!(
        media.error_code(json!({"op":"job.commit","job":job})),
        "stale_result"
    );
    assert_eq!(media.scenes(), ["first"]);
    assert_eq!(media.undo_depth(), 1);
    assert_eq!(media.finish(), Some(1), "the refused commit exits nonzero");
}
