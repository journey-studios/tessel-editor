//! Minimal ZIP64 container (stored entries only) for `.tesselpack`.
//!
//! WHY hand-written: packages hold already-compressed media, so compression buys nothing, and a
//! writer that streams every entry once (CRC-32 and SHA-256 computed while writing, sizes in a
//! trailing data descriptor) keeps memory constant and works past 4 GiB. Entries always use the
//! ZIP64 extra field, so a 5 GiB file and a 5 byte file are written the same way.
//! Reading validates names (no absolute paths, `..`, backslashes or duplicates) before anything
//! reaches the file system.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use sha2::{Digest, Sha256};
use tessel_editor_core::jobs::{CancelToken, JobError};

use crate::fsx::{CHUNK, hex, io_error};

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const EOCD64_SIG: u32 = 0x0606_4b50;
const LOCATOR_SIG: u32 = 0x0706_4b50;
const DESCRIPTOR_SIG: u32 = 0x0807_4b50;
/// UTF-8 names (bit 11) and trailing data descriptor (bit 3).
const FLAGS: u16 = 0x0808;
/// 1980-01-01: entries carry no wall-clock time so the same project packs to the same bytes.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = 0x0021;
const MAX_ENTRIES: u64 = 1_000_000;

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub crc: u32,
    pub offset: u64,
}

struct Counting<W: Write> {
    inner: W,
    written: u64,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(buf)?;
        self.written += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub struct ZipWriter<W: Write> {
    out: Counting<W>,
    entries: Vec<Entry>,
    names: HashSet<String>,
}

fn invalid(message: impl Into<String>) -> JobError {
    JobError::new("invalid_package", message)
}

/// `Ok` for a name that is a safe relative path made of normal segments.
pub fn check_entry_name(name: &str) -> Result<(), JobError> {
    let bad = name.is_empty()
        || name.starts_with('/')
        || name.contains('\\')
        || name.contains('\0')
        || name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.contains(':'));
    if bad {
        return Err(invalid(format!("Unsafe entry name `{name}`")));
    }
    Ok(())
}

impl<W: Write> ZipWriter<W> {
    pub fn new(out: W) -> Self {
        Self {
            out: Counting {
                inner: out,
                written: 0,
            },
            entries: Vec::new(),
            names: HashSet::new(),
        }
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), JobError> {
        self.out
            .write_all(bytes)
            .map_err(|error| io_error(Path::new("package"), &error))
    }

    fn local_header(&mut self, name: &str) -> Result<u64, JobError> {
        check_entry_name(name)?;
        // Case-insensitive: two names that differ only by case collide on common file systems.
        if !self.names.insert(name.to_lowercase()) {
            return Err(invalid(format!("Duplicate entry `{name}`")));
        }
        let offset = self.out.written;
        let mut header = Vec::with_capacity(30 + name.len() + 20);
        header.extend_from_slice(&LOCAL_SIG.to_le_bytes());
        header.extend_from_slice(&45u16.to_le_bytes());
        header.extend_from_slice(&FLAGS.to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes());
        header.extend_from_slice(&DOS_TIME.to_le_bytes());
        header.extend_from_slice(&DOS_DATE.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&u32::MAX.to_le_bytes());
        header.extend_from_slice(&u32::MAX.to_le_bytes());
        header.extend_from_slice(&(name.len() as u16).to_le_bytes());
        header.extend_from_slice(&20u16.to_le_bytes());
        header.extend_from_slice(name.as_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&16u16.to_le_bytes());
        header.extend_from_slice(&0u64.to_le_bytes());
        header.extend_from_slice(&0u64.to_le_bytes());
        self.put(&header)?;
        Ok(offset)
    }

    fn descriptor(&mut self, crc: u32, size: u64) -> Result<(), JobError> {
        let mut bytes = Vec::with_capacity(24);
        bytes.extend_from_slice(&DESCRIPTOR_SIG.to_le_bytes());
        bytes.extend_from_slice(&crc.to_le_bytes());
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&size.to_le_bytes());
        self.put(&bytes)
    }

    /// Streams `reader` into a new entry. Returns `(sha256 hex, size)`; `describe` names the
    /// source in errors, `on_bytes` receives the running size of this entry.
    pub fn add_stream<R: Read>(
        &mut self,
        name: &str,
        reader: &mut R,
        token: &CancelToken,
        describe: &Path,
        mut on_bytes: impl FnMut(u64),
    ) -> Result<(String, u64), JobError> {
        let offset = self.local_header(name)?;
        let (mut crc, mut sha) = (crc32fast::Hasher::new(), Sha256::new());
        let mut buffer = vec![0u8; CHUNK];
        let mut size = 0u64;
        loop {
            token.check()?;
            let read = reader
                .read(&mut buffer)
                .map_err(|error| io_error(describe, &error))?;
            if read == 0 {
                break;
            }
            crc.update(&buffer[..read]);
            sha.update(&buffer[..read]);
            self.put(&buffer[..read])?;
            size += read as u64;
            on_bytes(size);
        }
        let crc = crc.finalize();
        self.descriptor(crc, size)?;
        self.entries.push(Entry {
            name: name.to_string(),
            size,
            crc,
            offset,
        });
        Ok((hex(&sha.finalize()), size))
    }

    pub fn add_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<String, JobError> {
        let mut reader = bytes;
        self.add_stream(
            name,
            &mut reader,
            &CancelToken::new(),
            Path::new(name),
            |_| {},
        )
        .map(|(sha, _)| sha)
    }

    /// Writes the central directory and the ZIP64 end records; returns the inner writer.
    pub fn finish(mut self) -> Result<W, JobError> {
        let directory_start = self.out.written;
        let entries = std::mem::take(&mut self.entries);
        for entry in &entries {
            let mut record = Vec::with_capacity(46 + entry.name.len() + 28);
            record.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
            record.extend_from_slice(&45u16.to_le_bytes());
            record.extend_from_slice(&45u16.to_le_bytes());
            record.extend_from_slice(&FLAGS.to_le_bytes());
            record.extend_from_slice(&0u16.to_le_bytes());
            record.extend_from_slice(&DOS_TIME.to_le_bytes());
            record.extend_from_slice(&DOS_DATE.to_le_bytes());
            record.extend_from_slice(&entry.crc.to_le_bytes());
            record.extend_from_slice(&u32::MAX.to_le_bytes());
            record.extend_from_slice(&u32::MAX.to_le_bytes());
            record.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
            record.extend_from_slice(&28u16.to_le_bytes());
            record.extend_from_slice(&0u16.to_le_bytes());
            record.extend_from_slice(&0u16.to_le_bytes());
            record.extend_from_slice(&0u16.to_le_bytes());
            record.extend_from_slice(&0u32.to_le_bytes());
            record.extend_from_slice(&u32::MAX.to_le_bytes());
            record.extend_from_slice(entry.name.as_bytes());
            record.extend_from_slice(&1u16.to_le_bytes());
            record.extend_from_slice(&24u16.to_le_bytes());
            record.extend_from_slice(&entry.size.to_le_bytes());
            record.extend_from_slice(&entry.size.to_le_bytes());
            record.extend_from_slice(&entry.offset.to_le_bytes());
            self.put(&record)?;
        }
        let directory_size = self.out.written - directory_start;
        let eocd64_offset = self.out.written;
        let count = entries.len() as u64;
        let mut tail = Vec::new();
        tail.extend_from_slice(&EOCD64_SIG.to_le_bytes());
        tail.extend_from_slice(&44u64.to_le_bytes());
        tail.extend_from_slice(&45u16.to_le_bytes());
        tail.extend_from_slice(&45u16.to_le_bytes());
        tail.extend_from_slice(&0u32.to_le_bytes());
        tail.extend_from_slice(&0u32.to_le_bytes());
        tail.extend_from_slice(&count.to_le_bytes());
        tail.extend_from_slice(&count.to_le_bytes());
        tail.extend_from_slice(&directory_size.to_le_bytes());
        tail.extend_from_slice(&directory_start.to_le_bytes());
        tail.extend_from_slice(&LOCATOR_SIG.to_le_bytes());
        tail.extend_from_slice(&0u32.to_le_bytes());
        tail.extend_from_slice(&eocd64_offset.to_le_bytes());
        tail.extend_from_slice(&1u32.to_le_bytes());
        tail.extend_from_slice(&EOCD_SIG.to_le_bytes());
        tail.extend_from_slice(&0u16.to_le_bytes());
        tail.extend_from_slice(&0u16.to_le_bytes());
        tail.extend_from_slice(&0xFFFFu16.to_le_bytes());
        tail.extend_from_slice(&0xFFFFu16.to_le_bytes());
        tail.extend_from_slice(&u32::MAX.to_le_bytes());
        tail.extend_from_slice(&u32::MAX.to_le_bytes());
        tail.extend_from_slice(&0u16.to_le_bytes());
        self.put(&tail)?;
        self.out
            .flush()
            .map_err(|error| io_error(Path::new("package"), &error))?;
        Ok(self.out.inner)
    }
}

#[derive(Debug)]
pub struct ZipReader {
    file: File,
    pub entries: Vec<Entry>,
}

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn le64(bytes: &[u8], at: usize) -> u64 {
    let mut array = [0u8; 8];
    array.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(array)
}

impl ZipReader {
    /// Reads the central directory. Anything unexpected (compression, encryption, multi-disk,
    /// unsafe names, duplicates, absurd counts) is `invalid_package`: nothing is extracted yet.
    pub fn open(path: &Path) -> Result<Self, JobError> {
        let mut file = File::open(path).map_err(|error| io_error(path, &error))?;
        let length = file
            .seek(SeekFrom::End(0))
            .map_err(|error| io_error(path, &error))?;
        let tail_len = length.min(65_536 + 22 + 20 + 56);
        file.seek(SeekFrom::Start(length - tail_len))
            .map_err(|error| io_error(path, &error))?;
        let mut tail = vec![0u8; tail_len as usize];
        file.read_exact(&mut tail)
            .map_err(|error| io_error(path, &error))?;
        let eocd = tail
            .windows(4)
            .rposition(|window| window == EOCD_SIG.to_le_bytes())
            .ok_or_else(|| invalid("Not a package: no ZIP end record"))?;
        if tail.len() < eocd + 22 {
            return Err(invalid("The ZIP end record is truncated"));
        }
        let (mut total, mut directory_size, mut directory_start) = (
            u64::from(le16(&tail, eocd + 10)),
            u64::from(le32(&tail, eocd + 12)),
            u64::from(le32(&tail, eocd + 16)),
        );
        let needs64 = total == 0xFFFF
            || directory_size == u64::from(u32::MAX)
            || directory_start == u64::from(u32::MAX);
        if needs64 {
            let locator = eocd
                .checked_sub(20)
                .filter(|at| le32(&tail, *at) == LOCATOR_SIG)
                .ok_or_else(|| invalid("The ZIP64 locator is missing"))?;
            let eocd64_offset = le64(&tail, locator + 8);
            file.seek(SeekFrom::Start(eocd64_offset))
                .map_err(|error| io_error(path, &error))?;
            let mut record = [0u8; 56];
            file.read_exact(&mut record)
                .map_err(|error| io_error(path, &error))?;
            if le32(&record, 0) != EOCD64_SIG {
                return Err(invalid("The ZIP64 end record is damaged"));
            }
            total = le64(&record, 32);
            directory_size = le64(&record, 40);
            directory_start = le64(&record, 48);
        }
        if total > MAX_ENTRIES || directory_size > length || directory_start > length {
            return Err(invalid("The ZIP directory is out of range"));
        }
        file.seek(SeekFrom::Start(directory_start))
            .map_err(|error| io_error(path, &error))?;
        let mut directory = vec![0u8; directory_size as usize];
        file.read_exact(&mut directory)
            .map_err(|error| io_error(path, &error))?;
        let (mut at, mut entries, mut seen) = (0usize, Vec::new(), HashSet::new());
        while entries.len() < total as usize {
            if directory.len() < at + 46 || le32(&directory, at) != CENTRAL_SIG {
                return Err(invalid("The ZIP directory is damaged"));
            }
            let flags = le16(&directory, at + 8);
            let method = le16(&directory, at + 10);
            if flags & 1 != 0 || method != 0 {
                return Err(invalid("Only stored, unencrypted entries are supported"));
            }
            let crc = le32(&directory, at + 16);
            let mut size = u64::from(le32(&directory, at + 24));
            let (name_len, extra_len, comment_len) = (
                le16(&directory, at + 28) as usize,
                le16(&directory, at + 30) as usize,
                le16(&directory, at + 32) as usize,
            );
            let mut offset = u64::from(le32(&directory, at + 42));
            let end = at + 46 + name_len + extra_len + comment_len;
            if directory.len() < end {
                return Err(invalid("The ZIP directory is truncated"));
            }
            let name = std::str::from_utf8(&directory[at + 46..at + 46 + name_len])
                .map_err(|_| invalid("An entry name is not UTF-8"))?
                .to_string();
            let extra = &directory[at + 46 + name_len..at + 46 + name_len + extra_len];
            let mut cursor = 0;
            while cursor + 4 <= extra.len() {
                let (id, len) = (le16(extra, cursor), le16(extra, cursor + 2) as usize);
                let body = cursor + 4;
                if body + len > extra.len() {
                    return Err(invalid("A ZIP extra field is truncated"));
                }
                if id == 1 {
                    let mut field = body;
                    // Fields appear only for the 32-bit values that were saturated.
                    if size == u64::from(u32::MAX) && field + 8 <= body + len {
                        size = le64(extra, field);
                        field += 8;
                    }
                    if le32(&directory, at + 20) == u32::MAX && field + 8 <= body + len {
                        field += 8;
                    }
                    if offset == u64::from(u32::MAX) && field + 8 <= body + len {
                        offset = le64(extra, field);
                    }
                }
                cursor = body + len;
            }
            check_entry_name(name.trim_end_matches('/'))?;
            if !seen.insert(name.to_lowercase()) {
                return Err(invalid(format!("Duplicate entry `{name}`")));
            }
            if !name.ends_with('/') {
                entries.push(Entry {
                    name,
                    size,
                    crc,
                    offset,
                });
            } else {
                total -= 1;
            }
            at = end;
        }
        Ok(Self { file, entries })
    }

    /// Reader over one entry's bytes (after the local header).
    pub fn open_entry(&mut self, entry: &Entry) -> Result<impl Read + '_, JobError> {
        let mut header = [0u8; 30];
        self.file
            .seek(SeekFrom::Start(entry.offset))
            .map_err(|e| io_error(Path::new("package"), &e))?;
        self.file
            .read_exact(&mut header)
            .map_err(|e| io_error(Path::new("package"), &e))?;
        if le32(&header, 0) != LOCAL_SIG {
            return Err(invalid(format!(
                "The header of `{}` is damaged",
                entry.name
            )));
        }
        let skip = i64::from(le16(&header, 26)) + i64::from(le16(&header, 28));
        self.file
            .seek(SeekFrom::Current(skip))
            .map_err(|e| io_error(Path::new("package"), &e))?;
        Ok((&mut self.file).take(entry.size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Vec::new());
        for (name, bytes) in files {
            zip.add_bytes(name, bytes).unwrap();
        }
        zip.finish().unwrap()
    }

    #[test]
    fn names_are_validated_before_anything_is_written() {
        for bad in [
            "", "/abs", "../up", "a/../b", "a//b", "a\\b", "c:/x", "./a", "a/.",
        ] {
            assert!(check_entry_name(bad).is_err(), "{bad}");
        }
        check_entry_name("media/ab12/clip.mp4").unwrap();
        let mut zip = ZipWriter::new(Vec::new());
        zip.add_bytes("a/b.txt", b"x").unwrap();
        assert_eq!(
            zip.add_bytes("A/B.txt", b"y").unwrap_err().code,
            "invalid_package"
        );
    }

    #[test]
    fn round_trip_keeps_names_sizes_and_bytes() {
        let dir = std::env::temp_dir().join(format!("tessel-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.tesselpack");
        std::fs::write(
            &path,
            pack(&[
                ("a.txt", b"hello"),
                ("d/b.bin", &[7u8; 3000]),
                ("empty", b""),
            ]),
        )
        .unwrap();
        let mut reader = ZipReader::open(&path).unwrap();
        assert_eq!(
            reader
                .entries
                .iter()
                .map(|e| (e.name.as_str(), e.size))
                .collect::<Vec<_>>(),
            [("a.txt", 5), ("d/b.bin", 3000), ("empty", 0)]
        );
        let entry = reader.entries[1].clone();
        let mut bytes = Vec::new();
        reader
            .open_entry(&entry)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, vec![7u8; 3000]);
        assert_eq!(entry.crc, crc32fast::hash(&bytes));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn damaged_or_foreign_files_are_rejected() {
        let dir = std::env::temp_dir().join(format!("tessel-zip-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let junk = dir.join("junk");
        std::fs::write(&junk, b"not a zip at all").unwrap();
        assert_eq!(ZipReader::open(&junk).unwrap_err().code, "invalid_package");
        let mut truncated = pack(&[("a.txt", b"hello")]);
        truncated.truncate(truncated.len() - 30);
        let cut = dir.join("cut");
        std::fs::write(&cut, truncated).unwrap();
        assert!(ZipReader::open(&cut).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    /// A reader of `n` zero bytes without allocating them: proves the 64-bit offsets and sizes
    /// beyond 4 GiB in the headers while the sink drops the bytes.
    struct Zeros(u64);

    impl Read for Zeros {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = buf.len().min(self.0 as usize);
            buf[..n].fill(0);
            self.0 -= n as u64;
            Ok(n)
        }
    }

    #[test]
    #[ignore = "streams 4.5 GiB through CRC/SHA (about 10 s); run with --ignored"]
    fn entries_past_four_gib_use_64_bit_sizes_and_offsets() {
        let mut zip = ZipWriter::new(io::sink());
        let big = 4_500_000_000u64;
        let (_, first) = zip
            .add_stream(
                "big-1.bin",
                &mut Zeros(big),
                &CancelToken::new(),
                Path::new("zeros"),
                |_| {},
            )
            .unwrap();
        zip.add_stream(
            "big-2.bin",
            &mut Zeros(10),
            &CancelToken::new(),
            Path::new("zeros"),
            |_| {},
        )
        .unwrap();
        assert_eq!(first, big);
        assert!(
            zip.entries[1].offset > u64::from(u32::MAX),
            "the second entry starts past 4 GiB"
        );
        zip.finish().unwrap();
    }
}
