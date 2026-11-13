//! BL088: `project.package --replace` must keep the previous package when anything fails between
//! taking it out of the way and publishing the new one.
//!
//! Faults are real wherever the filesystem lets us trigger them deterministically (an unwritable
//! parent, a read-only folder inside the old package, cancellation). The one syscall that cannot be
//! made to fail on demand, the publishing `rename`, goes through the `PublishFs` seam of
//! `create_with_fs`; everything else (staging, hashing, verification) runs for real, and the
//! surviving package is always checked with `verify_package`, never by looking at file names.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{StandardSet, proof_requests, write_standard_set};
use tessel_editor_core::jobs::{JobError, JobKind, JobRegistry};
use tessel_editor_core::{EditorState, headless, persist};
use tessel_editor_media::package::{
    self, PackageFormat, PackageInputs, PackageOptions, PublishFs, StdFs,
};

use common::temp;

fn ok(state: &mut EditorState, request: Value) -> Value {
    headless::execute(state, &request).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn proof_state(set: &StandardSet, workspace: &Path) -> EditorState {
    let mut state = EditorState::demo();
    for request in proof_requests(set) {
        ok(&mut state, request);
    }
    persist::save(&mut state, &workspace.join("proof.tessel.json")).unwrap();
    state
}

/// The same project plus one more sequence: its package differs from the first one in the
/// document, so "old" and "new" are told apart by content, not by a name.
fn with_alt_sequence(state: &EditorState) -> EditorState {
    let mut next = state.clone();
    ok(
        &mut next,
        json!({"op":"sequence.add","id":"alt","name":"Alt","activate":true}),
    );
    next
}

struct Fixture {
    dir: PathBuf,
    old: EditorState,
    new: EditorState,
}

fn fixture(name: &str) -> Fixture {
    let dir = temp(name);
    let set = write_standard_set(&dir.join("src")).unwrap();
    let old = proof_state(&set, &dir.join("ws"));
    let new = with_alt_sequence(&old);
    Fixture { dir, old, new }
}

fn options(format: PackageFormat, replace: bool) -> PackageOptions {
    PackageOptions {
        format,
        sequences: None,
        allow_partial: false,
        replace,
    }
}

fn run(
    body: impl FnOnce(&tessel_editor_core::jobs::JobContext) -> Result<Value, JobError>,
) -> Result<Value, JobError> {
    JobRegistry::new(1)
        .run_inline(JobKind::Package, "package", body)
        .1
}

fn package_with(
    state: &EditorState,
    destination: &Path,
    options: &PackageOptions,
    fs_ops: &dyn PublishFs,
) -> Result<Value, JobError> {
    run(|ctx| {
        package::create_with_fs(
            state,
            destination,
            options,
            &PackageInputs::default(),
            ctx,
            fs_ops,
        )
    })
}

fn publish(
    state: &EditorState,
    destination: &Path,
    format: PackageFormat,
    replace: bool,
) -> Result<Value, JobError> {
    package_with(state, destination, &options(format, replace), &StdFs)
}

fn verify(path: &Path) -> Result<Value, JobError> {
    run(|ctx| package::verify_package(path, ctx))
}

/// Every byte under `path`, or `None` when nothing is there. Two equal snapshots are the same
/// package, down to each file.
fn snapshot(path: &Path) -> Option<BTreeMap<String, Vec<u8>>> {
    fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap().filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, into);
            } else {
                let name = path.strip_prefix(root).unwrap().to_string_lossy();
                into.insert(name.into_owned(), fs::read(&path).unwrap());
            }
        }
    }
    let meta = fs::symlink_metadata(path).ok()?;
    let mut files = BTreeMap::new();
    if meta.is_dir() {
        walk(path, path, &mut files);
    } else {
        files.insert(String::new(), fs::read(path).unwrap());
    }
    Some(files)
}

/// Sequence IDs of a verified package's document, to tell the old package from the new one.
fn sequences_of(path: &Path) -> Vec<String> {
    let report = verify(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let document_hash = report["manifest"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["role"] == "document")
        .unwrap()["sha256"]
        .clone();
    assert!(document_hash.is_string());
    let restored = path.with_file_name(format!(
        "restored-{}",
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .replace('.', "-")
    ));
    let _ = fs::remove_dir_all(&restored);
    run(|ctx| package::restore(path, &restored, ctx)).unwrap();
    let state = persist::load(&restored.join(package::DOCUMENT_NAME)).unwrap();
    let mut ids: Vec<String> = state
        .all_sequences()
        .map(|timeline| timeline.id.as_str().to_string())
        .collect();
    ids.sort();
    fs::remove_dir_all(&restored).unwrap();
    ids
}

/// Names under `parent` that the package operation may create and must not leave behind.
fn hidden_leftovers(parent: &Path, planted: &[&str]) -> Vec<String> {
    fs::read_dir(parent)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".tessel-") && !planted.contains(&name.as_str()))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Old package moved out of the way.
    SetAside,
    /// New package renamed onto the destination.
    Publish,
    /// Old package moved back after a failed publish.
    Restore,
}

/// Renames for real except for the steps it was told to fail, which return the given I/O error
/// without touching the disk. Records every step it saw.
struct Faulty {
    destination: PathBuf,
    faults: Vec<(Step, io::ErrorKind)>,
    seen: Mutex<Vec<Step>>,
}

impl Faulty {
    fn new(destination: &Path, faults: &[(Step, io::ErrorKind)]) -> Self {
        Self {
            destination: destination.to_path_buf(),
            faults: faults.to_vec(),
            seen: Mutex::default(),
        }
    }

    fn classify(&self, from: &Path) -> Step {
        let name = from.file_name().unwrap().to_string_lossy();
        if from == self.destination {
            Step::SetAside
        } else if name.starts_with(".tessel-previous-") {
            Step::Restore
        } else {
            Step::Publish
        }
    }

    fn steps(&self) -> Vec<Step> {
        self.seen.lock().unwrap().clone()
    }
}

impl PublishFs for Faulty {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let step = self.classify(from);
        self.seen.lock().unwrap().push(step);
        if let Some((_, kind)) = self.faults.iter().find(|(wanted, _)| *wanted == step) {
            return Err(io::Error::from(*kind));
        }
        fs::rename(from, to)
    }
}

fn destination_of(dir: &Path, format: PackageFormat) -> PathBuf {
    match format {
        PackageFormat::Archive => dir.join("out/proof.tesselpack"),
        PackageFormat::Directory => dir.join("out/proof-folder"),
    }
}

const FORMATS: [PackageFormat; 2] = [PackageFormat::Directory, PackageFormat::Archive];

#[test]
fn a_failed_publish_leaves_the_previous_package_readable_and_identical() {
    for format in FORMATS {
        for (kind, code) in [
            (io::ErrorKind::PermissionDenied, "permission_denied"),
            (io::ErrorKind::StorageFull, "disk_full"),
            (io::ErrorKind::Other, "io_error"),
        ] {
            let fixture = fixture(&format!("atomic-publish-{format:?}-{code}"));
            let destination = destination_of(&fixture.dir, format);
            publish(&fixture.old, &destination, format, false).unwrap();
            let before = snapshot(&destination);

            let faulty = Faulty::new(&destination, &[(Step::Publish, kind)]);
            let error = package_with(&fixture.new, &destination, &options(format, true), &faulty)
                .unwrap_err();

            assert_eq!(error.code, code, "{format:?}: {error}");
            // A folder is moved aside, published and put back; an archive replaces the old file
            // in one rename, so a failed rename has nothing to undo.
            let expected_steps = match format {
                PackageFormat::Directory => vec![Step::SetAside, Step::Publish, Step::Restore],
                PackageFormat::Archive => vec![Step::Publish],
            };
            assert_eq!(faulty.steps(), expected_steps, "{format:?}/{code}");
            assert_eq!(
                snapshot(&destination),
                before,
                "{format:?}/{code}: the previous package changed"
            );
            assert_eq!(sequences_of(&destination), ["main"], "{format:?}/{code}");
            assert_eq!(
                hidden_leftovers(destination.parent().unwrap(), &[]),
                Vec::<String>::new(),
                "{format:?}/{code}: staging or backup left behind"
            );
            fs::remove_dir_all(&fixture.dir).ok();
        }
    }
}

#[test]
fn a_failure_to_take_the_previous_package_aside_changes_nothing() {
    for format in FORMATS {
        let fixture = fixture(&format!("atomic-aside-{format:?}"));
        let destination = destination_of(&fixture.dir, format);
        publish(&fixture.old, &destination, format, false).unwrap();
        let before = snapshot(&destination);

        // For an archive the direct file-over-file rename is the publish step itself; for a
        // folder it is the move aside. Failing both covers whichever strategy runs.
        let faulty = Faulty::new(
            &destination,
            &[
                (Step::SetAside, io::ErrorKind::PermissionDenied),
                (Step::Publish, io::ErrorKind::PermissionDenied),
            ],
        );
        let error =
            package_with(&fixture.new, &destination, &options(format, true), &faulty).unwrap_err();

        assert_eq!(error.code, "permission_denied", "{format:?}: {error}");
        assert_eq!(snapshot(&destination), before, "{format:?}");
        assert_eq!(sequences_of(&destination), ["main"]);
        assert_eq!(
            hidden_leftovers(destination.parent().unwrap(), &[]),
            Vec::<String>::new()
        );
        fs::remove_dir_all(&fixture.dir).ok();
    }
}

#[test]
fn when_the_previous_package_cannot_even_be_restored_it_is_parked_and_reported() {
    let fixture = fixture("atomic-parked");
    let format = PackageFormat::Directory;
    let destination = destination_of(&fixture.dir, format);
    publish(&fixture.old, &destination, format, false).unwrap();
    let before = snapshot(&destination).unwrap();

    let faulty = Faulty::new(
        &destination,
        &[
            (Step::Publish, io::ErrorKind::PermissionDenied),
            (Step::Restore, io::ErrorKind::PermissionDenied),
        ],
    );
    let error =
        package_with(&fixture.new, &destination, &options(format, true), &faulty).unwrap_err();

    assert_eq!(error.code, "permission_denied", "{error}");
    let details = error.details.clone().expect("details name the parked copy");
    let parked = PathBuf::from(details["previous_package"].as_str().unwrap());
    assert!(
        error.message.contains(parked.to_str().unwrap()),
        "{}",
        error.message
    );
    assert_eq!(parked.parent(), destination.parent());
    assert_eq!(snapshot(&parked).unwrap(), before, "parked copy is intact");
    assert_eq!(sequences_of(&parked), ["main"], "and still verifies");
    assert!(
        !destination.exists(),
        "nothing half-published at the destination"
    );
    assert_eq!(
        hidden_leftovers(
            destination.parent().unwrap(),
            &[parked.file_name().unwrap().to_str().unwrap()]
        ),
        Vec::<String>::new(),
        "the new staging was cleaned"
    );
    // The parked copy is the user's data: a later successful replace must not delete it.
    fs::rename(&parked, &destination).unwrap();
    publish(&fixture.new, &destination, format, true).unwrap();
    assert_eq!(sequences_of(&destination), ["alt", "main"]);
    fs::remove_dir_all(&fixture.dir).ok();
}

#[test]
fn a_replace_publishes_the_complete_new_package_in_every_format_combination() {
    for from in FORMATS {
        for to in FORMATS {
            let fixture = fixture(&format!("atomic-ok-{from:?}-{to:?}"));
            // Same path whatever the format, so the replace also switches the kind of entry.
            let destination = fixture.dir.join("out/package");
            publish(&fixture.old, &destination, from, false).unwrap();
            assert_eq!(sequences_of(&destination), ["main"]);

            let report = publish(&fixture.new, &destination, to, true).unwrap();

            assert_eq!(report["complete"], true, "{from:?}→{to:?}: {report}");
            assert_eq!(destination.is_dir(), to == PackageFormat::Directory);
            assert_eq!(
                sequences_of(&destination),
                ["alt", "main"],
                "{from:?}→{to:?}"
            );
            assert!(report["warnings"].as_array().is_none_or(|w| {
                w.iter()
                    .all(|w| !w.as_str().unwrap_or_default().contains("previous"))
            }));
            assert_eq!(
                hidden_leftovers(destination.parent().unwrap(), &[]),
                Vec::<String>::new(),
                "{from:?}→{to:?}: the old package must be gone after success"
            );
            fs::remove_dir_all(&fixture.dir).ok();
        }
    }
}

#[test]
fn replace_over_nothing_and_without_replace_behave_as_before() {
    for format in FORMATS {
        let fixture = fixture(&format!("atomic-plain-{format:?}"));
        let destination = destination_of(&fixture.dir, format);
        // `replace` over a path that does not exist just publishes.
        publish(&fixture.old, &destination, format, true).unwrap();
        assert_eq!(sequences_of(&destination), ["main"]);
        // Without `replace` an existing package is never touched.
        let before = snapshot(&destination);
        let error = publish(&fixture.new, &destination, format, false).unwrap_err();
        assert_eq!(error.code, "destination_exists");
        assert_eq!(snapshot(&destination), before);
        fs::remove_dir_all(&fixture.dir).ok();
    }
}

#[test]
fn only_its_own_files_are_ever_removed() {
    for format in FORMATS {
        let fixture = fixture(&format!("atomic-own-{format:?}"));
        let destination = destination_of(&fixture.dir, format);
        let parent = destination.parent().unwrap().to_path_buf();
        publish(&fixture.old, &destination, format, false).unwrap();
        // Things that live next to the package and look like our own scratch space.
        fs::write(parent.join("neighbor.txt"), b"keep").unwrap();
        fs::write(parent.join("other.tesselpack"), b"someone else's").unwrap();
        for name in [".tessel-staging-user-1", ".tessel-previous-user-1"] {
            fs::create_dir_all(parent.join(name)).unwrap();
            fs::write(parent.join(name).join("note"), b"user data").unwrap();
        }
        let planted = [".tessel-staging-user-1", ".tessel-previous-user-1"];
        let before = snapshot(&parent).unwrap();

        // A failing and then a succeeding replace: neither may touch the neighbours.
        let faulty = Faulty::new(&destination, &[(Step::Publish, io::ErrorKind::Other)]);
        package_with(&fixture.new, &destination, &options(format, true), &faulty).unwrap_err();
        assert_eq!(
            snapshot(&parent).unwrap(),
            before,
            "{format:?}: after failure"
        );
        publish(&fixture.new, &destination, format, true).unwrap();

        let mut after = snapshot(&parent).unwrap();
        after.retain(|name, _| {
            !name.starts_with(destination.file_name().unwrap().to_str().unwrap())
        });
        let mut expected = before;
        expected.retain(|name, _| {
            !name.starts_with(destination.file_name().unwrap().to_str().unwrap())
        });
        assert_eq!(after, expected, "{format:?}: after success");
        assert_eq!(hidden_leftovers(&parent, &planted), Vec::<String>::new());
        fs::remove_dir_all(&fixture.dir).ok();
    }
}

#[test]
fn a_cancelled_replace_keeps_the_previous_package() {
    for format in FORMATS {
        let fixture = fixture(&format!("atomic-cancel-{format:?}"));
        let destination = destination_of(&fixture.dir, format);
        publish(&fixture.old, &destination, format, false).unwrap();
        let before = snapshot(&destination);

        // Deterministic: cancelled before the job starts working.
        let error = run(|ctx| {
            ctx.token().cancel();
            package::create_with(
                &fixture.new,
                &destination,
                &options(format, true),
                &PackageInputs::default(),
                ctx,
            )
        })
        .unwrap_err();
        assert!(error.is_cancelled(), "{format:?}: {error}");
        assert_eq!(snapshot(&destination), before);
        assert_eq!(
            hidden_leftovers(destination.parent().unwrap(), &[]),
            Vec::<String>::new()
        );

        // Cancelled by another thread as soon as the job reaches the verify stage, i.e. right
        // before publishing. Either outcome is valid; a half state is not.
        for _ in 0..8 {
            let registry = JobRegistry::new(1);
            let (state, target) = (fixture.new.clone(), destination.clone());
            let id = registry.spawn(JobKind::Package, "package", move |ctx| {
                package::create(&state, &target, &options(format, true), ctx)
            });
            loop {
                let info = registry.get(&id).unwrap();
                if info.state.is_final() {
                    break;
                }
                if info.stage == "verify" {
                    let _ = registry.cancel(&id);
                }
                std::thread::sleep(Duration::from_micros(200));
            }
            let info = registry.wait(&id, Duration::from_secs(30)).unwrap();
            let sequences = sequences_of(&destination);
            match info.state.label() {
                "cancelled" => assert_eq!(sequences, ["main"], "{format:?}: cancelled but changed"),
                "succeeded" => assert_eq!(sequences, ["alt", "main"], "{format:?}"),
                other => panic!("{format:?}: {other}: {:?}", info.error),
            }
            assert_eq!(
                hidden_leftovers(destination.parent().unwrap(), &[]),
                Vec::<String>::new(),
                "{format:?}"
            );
            // Back to the old package for the next round.
            publish(&fixture.old, &destination, format, true).unwrap();
        }
        fs::remove_dir_all(&fixture.dir).ok();
    }
}

#[cfg(unix)]
mod permissions {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    // Imported here, not at the top: only this Unix-only module uses it, and `-D warnings`
    // rejects an unused import on the targets that do not compile the module (BL167.1).
    use tessel_editor_media::host::MediaHost;

    use super::*;

    fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    fn is_root() -> bool {
        // Root can write into a 0555 folder, which these tests need to be impossible. The probe
        // folder is unique per call: the tests of this module run in parallel.
        static PROBES: AtomicU64 = AtomicU64::new(0);
        let probe = std::env::temp_dir().join(format!(
            "tessel-root-probe-{}-{}",
            std::process::id(),
            PROBES.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&probe).unwrap();
        set_mode(&probe, 0o555);
        let writable = fs::write(probe.join("x"), b"x").is_ok();
        set_mode(&probe, 0o755);
        fs::remove_dir_all(&probe).ok();
        writable
    }

    /// Makes the first media folder of a directory package read-only: the folder itself can be
    /// moved and replaced, but its contents cannot be deleted. Returns it for the cleanup.
    fn lock_media_folder(package: &Path) -> PathBuf {
        let media = package.join("media");
        let folder = fs::read_dir(&media)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.is_dir())
            .unwrap();
        set_mode(&folder, 0o555);
        folder
    }

    #[test]
    fn an_unwritable_parent_fails_with_permission_denied_and_keeps_the_previous_package() {
        if is_root() {
            return;
        }
        for format in FORMATS {
            let fixture = fixture(&format!("atomic-ro-parent-{format:?}"));
            let destination = destination_of(&fixture.dir, format);
            let parent = destination.parent().unwrap().to_path_buf();
            publish(&fixture.old, &destination, format, false).unwrap();
            let before = snapshot(&destination);

            set_mode(&parent, 0o555);
            let error = publish(&fixture.new, &destination, format, true);
            set_mode(&parent, 0o755);

            let error = error.unwrap_err();
            assert_eq!(error.code, "permission_denied", "{format:?}: {error}");
            assert_eq!(snapshot(&destination), before, "{format:?}");
            assert_eq!(sequences_of(&destination), ["main"]);
            fs::remove_dir_all(&fixture.dir).ok();
        }
    }

    #[test]
    fn an_old_package_that_cannot_be_deleted_is_set_aside_not_half_destroyed() {
        if is_root() {
            return;
        }
        let fixture = fixture("atomic-ro-inner");
        let format = PackageFormat::Directory;
        let destination = destination_of(&fixture.dir, format);
        publish(&fixture.old, &destination, format, false).unwrap();
        let locked = lock_media_folder(&destination);
        let locked_name = locked.file_name().unwrap().to_owned();

        let result = publish(&fixture.new, &destination, format, true);
        // Let the test clean up whatever the operation left, whatever happened.
        let parent = destination.parent().unwrap().to_path_buf();
        let parked: Vec<PathBuf> = fs::read_dir(&parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".tessel-previous-")
            })
            .collect();
        for folder in parked.iter().map(|p| p.join("media").join(&locked_name)) {
            if folder.exists() {
                set_mode(&folder, 0o755);
            }
        }

        let still_locked = destination.join("media").join(&locked_name);
        if still_locked.exists() {
            set_mode(&still_locked, 0o755);
        }
        let report = result.unwrap_or_else(|error| {
            panic!(
                "the new package must be published even if the old one is not deletable: {error}; \
                 the previous package afterwards: {:?}",
                verify(&destination).map(|_| "intact").map_err(|e| e.code)
            )
        });
        assert_eq!(sequences_of(&destination), ["alt", "main"]);
        // The leftover is reported, with its path, instead of being swallowed.
        let warnings = report["warnings"].to_string();
        assert!(warnings.contains(".tessel-previous-"), "{warnings}");
        assert_eq!(parked.len(), 1, "{parked:?}");
        // Only a leftover of a package that was already replaced: deleting it was interrupted at
        // the locked folder, so it is partial by nature. What matters is that it is outside the
        // destination, named by the warning and still holds the locked folder.
        assert!(warnings.contains(parked[0].to_str().unwrap()), "{warnings}");
        assert!(parked[0].join("media").join(&locked_name).is_dir());
        for path in parked {
            fs::remove_dir_all(path).unwrap();
        }
        fs::remove_dir_all(&fixture.dir).ok();
    }

    #[test]
    fn the_public_operation_replaces_reports_leftovers_and_keeps_the_old_on_refusal() {
        if is_root() {
            return;
        }
        let fixture = fixture("atomic-host");
        let destination = fixture.dir.join("out/host-folder");
        let mut host = MediaHost::new();
        let mut state = fixture.old.clone();
        let call = |host: &mut MediaHost, state: &mut EditorState, request: Value| {
            headless::execute_ext(state, None, Some(host), &request)
        };
        let package_request = |replace: bool| {
            json!({"op":"project.package","path":destination,"format":"directory",
                "replace":replace,"wait":true})
        };

        call(&mut host, &mut state, package_request(false)).unwrap();
        let before = snapshot(&destination);
        // Refused without `replace`; the package on disk is the same bytes.
        let error = call(&mut host, &mut state, package_request(false)).unwrap_err();
        assert_eq!(error.code, "destination_exists");
        assert_eq!(snapshot(&destination), before);

        // An unwritable parent: the operation fails and the package survives.
        let parent = destination.parent().unwrap();
        set_mode(parent, 0o555);
        let refused = call(&mut host, &mut state, package_request(true));
        set_mode(parent, 0o755);
        assert_eq!(refused.unwrap_err().code, "permission_denied");
        assert_eq!(snapshot(&destination), before);
        call(
            &mut host,
            &mut state,
            json!({"op":"project.verify","path":destination,"wait":true}),
        )
        .unwrap();

        // A real replace with an old package that cannot be fully deleted: it succeeds and the
        // result says where the leftover is.
        let locked = lock_media_folder(&destination);
        let locked_name = locked.file_name().unwrap().to_owned();
        let mut changed = fixture.new.clone();
        let report = call(&mut host, &mut changed, package_request(true));
        let parked: Vec<PathBuf> = fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.to_string_lossy().contains(".tessel-previous-"))
            .collect();
        for path in &parked {
            let folder = path.join("media").join(&locked_name);
            if folder.exists() {
                set_mode(&folder, 0o755);
            }
        }
        let report = report.unwrap();
        assert!(report["warnings"].to_string().contains(".tessel-previous-"));
        let verified = call(
            &mut host,
            &mut changed,
            json!({"op":"project.verify","path":destination,"wait":true}),
        )
        .unwrap();
        assert_eq!(verified["complete"], true);
        assert_eq!(sequences_of(&destination), ["alt", "main"]);
        for path in parked {
            fs::remove_dir_all(path).unwrap();
        }
        fs::remove_dir_all(&fixture.dir).ok();
    }
}
