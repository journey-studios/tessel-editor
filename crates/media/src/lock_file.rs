//! What an exclusive lock file needs from Windows beyond `File::try_lock` (BL168.6.1). Compiled on
//! Windows only: the Unix answers stay where they were, unchanged.
//!
//! Two lock files follow the same protocol: the root lease (`directed_resources::lease`) and the
//! claims ledger (`scene_resources::claims::ledger`). The holder **unlinks the file while it still
//! holds the lock** and unlocks afterwards, so a waiter that opened the old file and locks it
//! after the release must notice that it holds a file nobody can find any more, and start again.
//! On Unix that is a device/inode comparison.
//!
//! Windows needs its own answer, for facts measured on Windows 11 / NTFS:
//!
//! * `fs::remove_file` of a file another handle has open succeeds at once (POSIX delete): the
//!   name is gone and a new file can be created under it while the old handle still locks the old
//!   file. "The path exists" is therefore no proof: it is true for the *new* file, and two
//!   holders believed they had the same ledger (BL168.6.1).
//! * The standard library exposes no file index on Windows (`windows_by_handle` is unstable) and
//!   the crate takes no `windows-sys` dependency. The creation time cannot stand in for it as it
//!   is either: NTFS *tunnels* it, giving a file created under a name that was removed a moment
//!   before the creation time of the removed one.
//!
//! So the holder **stamps** the file it has just locked: it sets the creation time of its own
//! handle to a fresh random value and reads the creation time at the path. Only the holder of a
//! lock writes that stamp, a stamp is set after the old name was removed (the releaser unlinks
//! before it unlocks), and tunnelling can only hand out a value a file had *when it was removed*,
//! so the path shows this holder's stamp if and only if it is this holder's file.
//!
//! Byte-range locks on Windows are mandatory: while a lease is held no other handle can read the
//! lock file, so the holder's process ID (informational, for the `root_busy` message) is also
//! written to an NTFS stream of the lock file, `<lock>:holder`, which the lock does not cover. It
//! goes away with the file, and a directory listing does not show it. A volume without streams
//! (FAT, exFAT) only loses the name of the holder (`holder: null`), never the lease.

use std::ffi::OsString;
use std::fs::{self, File, FileTimes, OpenOptions};
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io::{Read, Write};
use std::os::windows::fs::{FileTimesExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Whether `file`, which this process has **just locked**, is still the file at `path`.
///
/// A volume that refuses to change a creation time cannot be stamped: the answer is then only
/// "the path exists", the weaker check this replaced (a limit of such volumes, stated in
/// `docs/migration/windows-claims-report.md`).
pub(crate) fn is_still_at(path: &Path, file: &File) -> bool {
    if file
        .set_times(FileTimes::new().set_created(fresh_stamp()))
        .is_err()
    {
        return path.exists();
    }
    match (fs::metadata(path), file.metadata()) {
        (Ok(on_disk), Ok(open)) => on_disk.creation_time() == open.creation_time(),
        _ => false,
    }
}

/// Records `pid` as the holder of the lock file `lock` (best effort: it only names the holder).
pub(crate) fn write_holder(lock: &Path, pid: u32) {
    if let Ok(mut stream) = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(holder_stream(lock))
    {
        let _ = write!(stream, "{pid}");
    }
}

/// The process ID the holder of `lock` recorded, if it could and the lock file is still there.
pub(crate) fn read_holder(lock: &Path) -> Option<u32> {
    let mut text = String::new();
    File::open(holder_stream(lock))
        .and_then(|mut stream| stream.read_to_string(&mut text))
        .ok()?;
    text.trim().parse().ok()
}

/// `<lock>:holder`, the stream of the lock file that names its holder.
fn holder_stream(lock: &Path) -> PathBuf {
    let mut name = OsString::from(lock.as_os_str());
    name.push(":holder");
    PathBuf::from(name)
}

/// A creation time no other lock file has: random, between 2001 and 2051, in the 100 ns steps
/// NTFS keeps (FAT keeps 10 ms steps, which still leaves about 10^11 values).
fn fresh_stamp() -> SystemTime {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    const SPAN: u64 = 50 * 365 * 24 * 3600 * 10_000_000;
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.write_u32(std::process::id());
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos()),
    );
    let ticks = hasher.finish() % SPAN;
    UNIX_EPOCH + Duration::from_secs(1_000_000_000) + Duration::from_nanos(ticks * 100)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsx::Staging;

    fn locked(path: &Path) -> File {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .unwrap();
        file.try_lock().unwrap();
        file
    }

    #[test]
    fn the_file_at_the_path_is_recognised() {
        let dir = Staging::create(&std::env::temp_dir(), "lock-file-same").unwrap();
        let path = dir.path().join("lock");
        let file = locked(&path);
        assert!(is_still_at(&path, &file));
        assert!(
            is_still_at(&path, &file),
            "and again: a new stamp each time"
        );
    }

    /// The case the check exists for: the lock taken on a file whose name was removed and given
    /// to a new file, which NTFS gives the creation time of the removed one.
    #[test]
    fn a_lock_on_a_removed_file_is_not_the_file_at_the_path_even_after_tunnelling() {
        let dir = Staging::create(&std::env::temp_dir(), "lock-file-unlinked").unwrap();
        let path = dir.path().join("lock");
        let first = locked(&path);
        assert!(is_still_at(&path, &first), "stamped by the first holder");
        let waiter = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        // The first holder releases: unlink, then unlock.
        fs::remove_file(&path).unwrap();
        drop(first);
        waiter.try_lock().unwrap();
        assert!(!is_still_at(&path, &waiter), "no file at the path at all");
        let newcomer = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        assert!(
            !is_still_at(&path, &waiter),
            "a file has the name again (and the tunnelled creation time): not the waiter's"
        );
        newcomer.try_lock().unwrap();
        assert!(is_still_at(&path, &newcomer));
    }

    #[test]
    fn the_holder_is_readable_while_the_lock_is_held_and_gone_with_the_file() {
        let dir = Staging::create(&std::env::temp_dir(), "lock-file-holder").unwrap();
        let path = dir.path().join("lock");
        let file = locked(&path);
        write_holder(&path, 4242);
        assert_eq!(read_holder(&path), Some(4242));
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["lock"], "the stream is not a file of its own");
        fs::remove_file(&path).unwrap();
        drop(file);
        assert_eq!(read_holder(&path), None);
    }
}
