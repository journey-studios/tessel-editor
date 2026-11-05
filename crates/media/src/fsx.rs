//! Filesystem helpers shared by import, derived media, packages and export.
//!
//! Everything that writes uses a *staging* directory next to the destination and publishes by
//! rename only after the work finished and was verified. A failure or a cancellation removes the
//! staging directory and leaves both the source and any previous destination untouched.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use tessel_editor_core::jobs::{CancelToken, JobError};

/// Chunk of the streaming copy/hash: constant memory whatever the file size.
pub const CHUNK: usize = 1 << 20;

pub fn io_error(path: &Path, error: &io::Error) -> JobError {
    let code = match error.kind() {
        io::ErrorKind::StorageFull => "disk_full",
        io::ErrorKind::NotFound => "asset_unreadable",
        io::ErrorKind::PermissionDenied => "permission_denied",
        _ => "io_error",
    };
    JobError::new(code, format!("`{}`: {error}", path.display()))
}

/// Free bytes of the volume that holds `path` (or its nearest existing ancestor), via `df`.
/// `None` when it cannot be determined; callers then skip the estimate instead of guessing.
pub fn free_bytes(path: &Path) -> Option<u64> {
    let mut probe = std::path::absolute(path).ok()?;
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let output = Command::new("df").arg("-Pk").arg(&probe).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().nth(1)?;
    let available: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    available.checked_mul(1024)
}

/// Fails with `disk_full` before any byte is written when the estimate cannot fit.
/// `margin` keeps a little room for the filesystem and the other writers.
pub fn ensure_space(path: &Path, needed: u64, margin: u64) -> Result<(), JobError> {
    let Some(free) = free_bytes(path) else {
        return Ok(());
    };
    let required = needed.saturating_add(margin);
    if free >= required {
        return Ok(());
    }
    Err(JobError::new(
        "disk_full",
        format!(
            "`{}` needs about {} MiB but only {} MiB are free",
            path.display(),
            required.div_ceil(1 << 20),
            free / (1 << 20)
        ),
    )
    .with_details(serde_json::json!({ "needed": required, "free": free })))
}

static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A directory that is deleted on drop unless it was published.
#[derive(Debug)]
pub struct Staging {
    path: PathBuf,
    published: bool,
}

impl Staging {
    /// Creates `<parent>/.tessel-staging-<tag>-<pid>-<n>`. It lives on the destination volume so
    /// publishing is an atomic rename, never a cross-device copy.
    pub fn create(parent: &Path, tag: &str) -> Result<Self, JobError> {
        fs::create_dir_all(parent).map_err(|error| io_error(parent, &error))?;
        let unique = STAGING_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = parent.join(format!(
            ".tessel-staging-{tag}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).map_err(|error| io_error(&path, &error))?;
        Ok(Self {
            path,
            published: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Moves the staging directory to `destination`. Refuses to replace anything.
    pub fn publish(mut self, destination: &Path) -> Result<(), JobError> {
        if destination.exists() {
            return Err(JobError::new(
                "destination_exists",
                format!(
                    "`{}` already exists; nothing was overwritten",
                    destination.display()
                ),
            ));
        }
        fs::rename(&self.path, destination).map_err(|error| io_error(destination, &error))?;
        self.published = true;
        Ok(())
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Moves a finished staged *file* over `destination`. With `replace` false an existing file is
/// an error; with it true the swap is a single rename, so readers see the old or the new file.
pub fn publish_file(staged: &Path, destination: &Path, replace: bool) -> Result<(), JobError> {
    if !replace && destination.exists() {
        let _ = fs::remove_file(staged);
        return Err(JobError::new(
            "destination_exists",
            format!(
                "`{}` already exists; nothing was overwritten",
                destination.display()
            ),
        ));
    }
    fs::rename(staged, destination).map_err(|error| {
        let _ = fs::remove_file(staged);
        io_error(destination, &error)
    })
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Copies `reader` into `writer`, hashing while it goes. `on_bytes` receives the running total.
/// Cancellation is observed between chunks; a failed write keeps the writer's error code
/// (`disk_full` for ENOSPC).
pub fn copy_hashed<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    token: &CancelToken,
    describe: &Path,
    mut on_bytes: impl FnMut(u64),
) -> Result<(String, u64), JobError> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        token.check()?;
        let read = reader
            .read(&mut buffer)
            .map_err(|error| io_error(describe, &error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        writer
            .write_all(&buffer[..read])
            .map_err(|error| io_error(describe, &error))?;
        total += read as u64;
        on_bytes(total);
    }
    writer.flush().map_err(|error| io_error(describe, &error))?;
    Ok((hex(&hasher.finalize()), total))
}

/// SHA-256 of a file, streaming.
pub fn hash_file(
    path: &Path,
    token: &CancelToken,
    on_bytes: impl FnMut(u64),
) -> Result<(String, u64), JobError> {
    let mut file = File::open(path).map_err(|error| io_error(path, &error))?;
    copy_hashed(&mut file, &mut io::sink(), token, path, on_bytes)
}

/// Copies a file into `staged` (hashing it), fsyncs and returns `(sha256, size)`. The original
/// is only ever opened for reading.
pub fn copy_file_hashed(
    source: &Path,
    staged: &Path,
    token: &CancelToken,
    on_bytes: impl FnMut(u64),
) -> Result<(String, u64), JobError> {
    let mut input = File::open(source).map_err(|error| io_error(source, &error))?;
    let mut output = File::create(staged).map_err(|error| io_error(staged, &error))?;
    let result = copy_hashed(&mut input, &mut output, token, staged, on_bytes)?;
    output
        .sync_all()
        .map_err(|error| io_error(staged, &error))?;
    Ok(result)
}

/// A file name that is safe inside a package or `media/<hash>/` folder: no separators, no
/// control characters, no leading dot, bounded length. The original name is kept in the asset.
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches([' ', '.']).to_string();
    let mut name = if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed
    };
    if name.len() > 120 {
        let extension = Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| e.len() <= 12)
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let keep: String = name.chars().take(100).collect();
        name = format!("{keep}{extension}");
    }
    name
}

/// Total size of the regular files under `dir`; unreadable entries count as zero.
pub fn directory_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => directory_size(&path),
                Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |meta| meta.len()),
                _ => 0,
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Limited(u64);

    impl Write for Limited {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.0 < buf.len() as u64 {
                return Err(io::Error::from(io::ErrorKind::StorageFull));
            }
            self.0 -= buf.len() as u64;
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn copy_hashes_while_streaming_and_reports_disk_full() {
        let data = vec![7u8; CHUNK * 2 + 5];
        let mut sink = Vec::new();
        let (hash, size) = copy_hashed(
            &mut &data[..],
            &mut sink,
            &CancelToken::new(),
            Path::new("x"),
            |_| {},
        )
        .unwrap();
        assert_eq!(size, data.len() as u64);
        assert_eq!(hash, hex(&Sha256::digest(&data)));
        assert_eq!(sink, data);

        let error = copy_hashed(
            &mut &data[..],
            &mut Limited(CHUNK as u64),
            &CancelToken::new(),
            Path::new("x"),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error.code, "disk_full");
    }

    #[test]
    fn copy_observes_cancellation() {
        let token = CancelToken::new();
        token.cancel();
        let error = copy_hashed(
            &mut &[1u8; 10][..],
            &mut io::sink(),
            &token,
            Path::new("x"),
            |_| {},
        )
        .unwrap_err();
        assert!(error.is_cancelled());
    }

    #[test]
    fn file_names_cannot_escape_or_hide() {
        assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_name(".hidden"), "hidden");
        assert_eq!(safe_file_name("a:b*c.png"), "a_b_c.png");
        assert_eq!(safe_file_name("..."), "file");
        assert!(safe_file_name(&"x".repeat(300)).len() <= 120);
    }

    #[test]
    fn staging_is_removed_unless_published() {
        let base = std::env::temp_dir().join(format!("tessel-fsx-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let path = {
            let staging = Staging::create(&base, "t").unwrap();
            fs::write(staging.path().join("f"), "x").unwrap();
            staging.path().to_path_buf()
        };
        assert!(!path.exists(), "dropped staging is cleaned");
        let staging = Staging::create(&base, "t").unwrap();
        fs::write(staging.path().join("f"), "x").unwrap();
        let destination = base.join("final");
        staging.publish(&destination).unwrap();
        assert!(destination.join("f").is_file());
        let again = Staging::create(&base, "t").unwrap();
        assert_eq!(
            again.publish(&destination).unwrap_err().code,
            "destination_exists"
        );
        fs::remove_dir_all(base).ok();
    }
}
