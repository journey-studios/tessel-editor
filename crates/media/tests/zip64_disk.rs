//! P07/P12: a ZIP64 archive larger than 4 GiB written to a real file and read back by an
//! external tool (the unit test in `zip.rs` streams the same headers into `io::sink`, which
//! proves the header arithmetic but not that another implementation accepts the file).
//!
//! The archive is ~4.5 GiB, so the test is `#[ignore]` and refuses to start unless the temporary
//! volume has the archive's size plus a margin free. It never frees space by deleting anything
//! of the user's: a machine without the room keeps this case *pending* and says why.
//!
//! `TESSEL_ZIP64_DISK=1 ./scripts/cargo test -p tessel-editor-media --test zip64_disk -- --ignored`

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;
use std::process::Command;

use tessel_editor_core::jobs::CancelToken;
use tessel_editor_media::zip::{ZipReader, ZipWriter};

const BIG: u64 = 4_500_000_000;
/// Room kept free besides the archive: the OS and the build keep writing while the test runs.
const MARGIN: u64 = 2 * 1024 * 1024 * 1024;

struct Zeros(u64);

impl Read for Zeros {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min(self.0 as usize);
        buf[..n].fill(0);
        self.0 -= n as u64;
        Ok(n)
    }
}

/// Bytes available to this user on the volume holding `path`, from `df -k`.
fn available(path: &Path) -> u64 {
    let output = Command::new("df")
        .arg("-k")
        .arg(path)
        .output()
        .expect("df runs");
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().nth(1).expect("df prints a volume line");
    // Filesystem, 1K-blocks, Used, Available, ...
    let kib: u64 = line
        .split_whitespace()
        .nth(3)
        .and_then(|field| field.parse().ok())
        .expect("df prints the available KiB");
    kib * 1024
}

#[test]
#[ignore = "writes a 4.5 GiB file; needs TESSEL_ZIP64_DISK=1 and free disk space"]
fn a_zip64_archive_past_four_gib_is_written_to_disk_and_read_by_unzip() {
    let dir = std::env::temp_dir().join(format!("tessel-zip64-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let free = available(&dir);
    let needed = BIG + MARGIN;
    if free < needed {
        std::fs::remove_dir_all(&dir).ok();
        panic!(
            "PENDING (not run): the temporary volume has {:.1} GiB free and this case needs {:.1} GiB \
             (4.5 GiB archive + 2 GiB margin). Nothing was written or deleted. Free space elsewhere or \
             point TMPDIR at a bigger volume, then run again.",
            free as f64 / 1_073_741_824.0,
            needed as f64 / 1_073_741_824.0
        );
    }
    let archive = dir.join("big.zip");
    let outcome = std::panic::catch_unwind(|| {
        let mut zip = ZipWriter::new(BufWriter::new(File::create(&archive).unwrap()));
        let token = CancelToken::new();
        zip.add_stream(
            "big.bin",
            &mut Zeros(BIG),
            &token,
            Path::new("zeros"),
            |_| {},
        )
        .unwrap();
        // This entry's local header sits past the 4 GiB mark: its offset needs the ZIP64 record.
        zip.add_bytes("tail.txt", b"after four gibibytes").unwrap();
        zip.finish().unwrap().flush().unwrap();
        assert!(std::fs::metadata(&archive).unwrap().len() > BIG);

        // Our own reader.
        let reader = ZipReader::open(&archive).unwrap();
        let names: Vec<&str> = reader.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["big.bin", "tail.txt"]);
        assert!(reader.entries[1].offset > u64::from(u32::MAX));

        // An independent implementation: Info-ZIP verifies every CRC (streaming, nothing is
        // extracted to disk) and then extracts just the entry that lives past 4 GiB.
        let tested = Command::new("unzip")
            .arg("-tq")
            .arg(&archive)
            .output()
            .unwrap();
        assert!(
            tested.status.success(),
            "unzip -t: {}{}",
            String::from_utf8_lossy(&tested.stdout),
            String::from_utf8_lossy(&tested.stderr)
        );
        let tail = Command::new("unzip")
            .args(["-p"])
            .arg(&archive)
            .arg("tail.txt")
            .output()
            .unwrap();
        assert!(tail.status.success());
        assert_eq!(tail.stdout, b"after four gibibytes");
    });
    std::fs::remove_dir_all(&dir).ok();
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
