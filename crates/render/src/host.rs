//! What serves the runtime page and the project's files to a browser engine, independent of
//! which engine it is: the desktop WebView calls this from its custom protocol handler, the
//! Node worker has an equivalent in `runtime/worker/assets.mjs` (kept in step by hand, with
//! the same tests on both sides).
//!
//! A file is served only if an `asset.load` message named it: the page cannot read anything
//! else, even though it runs with the user's privileges.

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

#[derive(Debug, Clone)]
struct Pattern {
    prefix: String,
    suffix: String,
    min_digits: usize,
}

impl Pattern {
    /// `/a/f%05d.png` -> prefix `/a/f`, suffix `.png`, at least 5 digits. Only the index varies.
    fn parse(pattern: &str) -> Option<Self> {
        let at = pattern.find('%')?;
        let rest = &pattern[at + 1..];
        let zero = rest.starts_with('0');
        let digits: String = rest
            .chars()
            .skip(usize::from(zero))
            .take_while(char::is_ascii_digit)
            .collect();
        let after = &rest[usize::from(zero) + digits.len()..];
        let suffix = after.strip_prefix('d')?;
        let width = digits.parse::<usize>().unwrap_or(0);
        Some(Self {
            prefix: pattern[..at].to_string(),
            suffix: suffix.to_string(),
            min_digits: if zero { width.max(1) } else { 1 },
        })
    }

    fn matches(&self, path: &str) -> bool {
        let Some(middle) = path
            .strip_prefix(&self.prefix)
            .and_then(|rest| rest.strip_suffix(&self.suffix))
        else {
            return false;
        };
        middle.len() >= self.min_digits && middle.bytes().all(|byte| byte.is_ascii_digit())
    }
}

/// Files the page may read.
#[derive(Debug, Clone, Default)]
pub struct AssetAccess {
    files: HashSet<PathBuf>,
    patterns: Vec<Pattern>,
}

/// Absolute and without `.`/`..`, decided on the text alone. Both what an `asset.load` names
/// and what the page asks for go through this, so `a/../b` can only ever mean `b`, which is
/// then checked against the allow-list like any other path.
fn absolute(path: &str) -> PathBuf {
    let path = std::path::absolute(path).unwrap_or_else(|_| PathBuf::from(path));
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                clean.pop();
            }
            std::path::Component::CurDir => {}
            other => clean.push(other),
        }
    }
    clean
}

impl AssetAccess {
    /// Registers the files an `asset.load` descriptor (bridge JSON) refers to.
    pub fn register(&mut self, descriptor: &Value) {
        let source = &descriptor["source"];
        match source["type"].as_str() {
            Some("file") => {
                if let Some(path) = source["path"].as_str() {
                    self.files.insert(absolute(path));
                }
            }
            Some("frame_sequence") => {
                let pattern = source["pattern"].as_str().map(absolute);
                if let Some(pattern) = pattern
                    .as_deref()
                    .and_then(Path::to_str)
                    .and_then(Pattern::parse)
                {
                    self.patterns.push(pattern);
                }
            }
            _ => {}
        }
    }

    pub fn allows(&self, path: &str) -> bool {
        let candidate = absolute(path);
        // `absolute` does not resolve `..`; refuse it outright instead of normalizing.
        if candidate
            .components()
            .any(|part| part == std::path::Component::ParentDir)
        {
            return false;
        }
        self.files.contains(&candidate)
            || candidate
                .to_str()
                .is_some_and(|text| self.patterns.iter().any(|pattern| pattern.matches(text)))
    }

    pub fn clear(&mut self) {
        self.files.clear();
        self.patterns.clear();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Served {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    /// `Content-Range` value for 206 answers.
    pub content_range: Option<String>,
}

impl Served {
    fn text(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: message.into().into_bytes(),
            content_range: None,
        }
    }
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("webm") => "video/webm",
        Some("mp4") => "video/mp4",
        Some("wav") => "audio/wav",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let hex = raw.get(index + 1..index + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == key)
        .and_then(|(_, value)| percent_decode(value))
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n` against a file of `size` bytes.
fn parse_range(header: &str, size: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = header.strip_prefix("bytes=")?;
    let (from, to) = spec.split_once('-')?;
    let (start, end) = match (from.is_empty(), to.is_empty()) {
        (true, true) => return None,
        (true, false) => (size.saturating_sub(to.parse().ok()?), size.checked_sub(1)?),
        (false, true) => (from.parse().ok()?, size.checked_sub(1)?),
        (false, false) => (
            from.parse().ok()?,
            to.parse::<u64>().ok()?.min(size.saturating_sub(1)),
        ),
    };
    Some(if start <= end && start < size {
        Ok((start, end))
    } else {
        Err(())
    })
}

/// The most one answer carries. The body is one `Vec` handed to the WebView, and a freed block this
/// large stays in the allocator's cache: answering a whole 66 MB splat file (or a `<video>`'s open
/// `bytes=0-`) left ~60 MB resident per file, for good. A range answer may be shorter than asked
/// (`Content-Range` says what it is), so players and `fetch` loops simply ask for the rest.
pub const MAX_RANGE_BODY: u64 = 4 << 20;

/// Cuts a range to [`MAX_RANGE_BODY`]: from its start, or, for a suffix range (`bytes=-n`), keeping the tail.
fn clamp_range(start: u64, end: u64, suffix: bool) -> (u64, u64) {
    if end - start < MAX_RANGE_BODY {
        (start, end)
    } else if suffix {
        (end + 1 - MAX_RANGE_BODY, end)
    } else {
        (start, start + MAX_RANGE_BODY - 1)
    }
}

/// What a request resolves to before any file is touched: an answer that needs no read (the page's own refusals, an empty
/// `204`), or a file to read with the `Range` the page sent. Authorization is decided here, on the caller's thread, from
/// the allow-list as it is at this moment; the read that follows can then run elsewhere without sharing that list.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    Answer(Served),
    File {
        file: PathBuf,
        range: Option<String>,
    },
}

/// Decides what `GET <path>?<query>` refers to: the page (`/`, `/runtime.js`) from `dist`, or a registered asset
/// (`/asset?p=<absolute path>`). No file is opened.
pub fn resolve(
    dist: &Path,
    access: &AssetAccess,
    path: &str,
    query: &str,
    range: Option<&str>,
) -> Resolved {
    let file = match path {
        "/" | "/index.html" => dist.join("index.html"),
        "/runtime.js" => dist.join("runtime.js"),
        "/favicon.ico" => return Resolved::Answer(Served::text(204, "")),
        "/asset" => {
            let Some(requested) = query_param(query, "p") else {
                return Resolved::Answer(Served::text(403, "not an asset of this project"));
            };
            if !access.allows(&requested) {
                return Resolved::Answer(Served::text(403, "not an asset of this project"));
            }
            absolute(&requested)
        }
        _ => return Resolved::Answer(Served::text(404, "not found")),
    };
    Resolved::File {
        file,
        range: range.map(str::to_owned),
    }
}

/// Reads what a [`Resolved::File`] names: open, size, byte range, body. All the blocking I/O of serving is here.
pub fn read_file(file: &Path, range: Option<&str>) -> Served {
    let Ok(mut handle) = File::open(file) else {
        return Served::text(404, format!("missing {}", file.display()));
    };
    let Ok(size) = handle.metadata().map(|meta| meta.len()) else {
        return Served::text(404, format!("unreadable {}", file.display()));
    };
    let kind = content_type(file);
    let ranged = range.and_then(|header| parse_range(header, size));
    let (start, end, status) = match ranged {
        Some(Ok((start, end))) => {
            let (start, end) =
                clamp_range(start, end, range.is_some_and(|h| h.starts_with("bytes=-")));
            (start, end, 206)
        }
        Some(Err(())) => {
            return Served {
                status: 416,
                content_type: "text/plain; charset=utf-8",
                body: b"bad range".to_vec(),
                content_range: Some(format!("bytes */{size}")),
            };
        }
        None => (0, size.saturating_sub(1), 200),
    };
    // Exact capacity: `read_to_end` on an empty Vec doubles by `realloc` (1, 2, 4 ... 64 MiB for a splat
    // file), and every step leaves a freed block the allocator keeps resident.
    let mut body = Vec::with_capacity(if size > 0 {
        (end - start + 1) as usize
    } else {
        0
    });
    if size > 0
        && (handle.seek(SeekFrom::Start(start)).is_err()
            || handle.take(end - start + 1).read_to_end(&mut body).is_err())
    {
        return Served::text(500, format!("cannot read {}", file.display()));
    }
    Served {
        status,
        content_type: kind,
        body,
        content_range: (status == 206).then(|| format!("bytes {start}-{end}/{size}")),
    }
}

/// Answers `GET <path>?<query>`: [`resolve`] and, for a file, [`read_file`], on the calling thread.
pub fn serve(
    dist: &Path,
    access: &AssetAccess,
    path: &str,
    query: &str,
    range: Option<&str>,
) -> Served {
    match resolve(dist, access, path, query, range) {
        Resolved::Answer(served) => served,
        Resolved::File { file, range } => read_file(&file, range.as_deref()),
    }
}

/// Standard base64 (padding optional), as `canvas.toDataURL` produces it.
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in text.bytes().filter(|byte| *byte != b'=') {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Writes the PNG a page sent as base64 to the path the request named.
pub fn write_png(path: &Path, base64: &str) -> Result<(), String> {
    let bytes = decode_base64(base64).ok_or("the page sent a PNG that is not valid base64")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
    }
    std::fs::write(path, bytes)
        .map_err(|error| format!("cannot write `{}`: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tessel-host-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn encode(path: &Path) -> String {
        path.to_string_lossy()
            .replace('/', "%2F")
            .replace(' ', "%20")
    }

    #[test]
    fn a_range_answer_never_exceeds_the_body_cap_and_says_what_it_carries() {
        let dir = scratch("cap");
        let size = 2 * MAX_RANGE_BODY + 10;
        let mut content = vec![b'a'; size as usize];
        content[size as usize - 1] = b'z';
        std::fs::write(dir.join("big.ply"), &content).unwrap();
        let mut access = AssetAccess::default();
        access.register(&json!({"source": {"type": "file", "path": dir.join("big.ply")}}));
        let query = format!("p={}", encode(&dir.join("big.ply")));

        let open = serve(&dir, &access, "/asset", &query, Some("bytes=5-"));
        assert_eq!((open.status, open.body.len() as u64), (206, MAX_RANGE_BODY));
        assert_eq!(
            open.content_range,
            Some(format!("bytes 5-{}/{size}", 5 + MAX_RANGE_BODY - 1))
        );
        let tail = serve(&dir, &access, "/asset", &query, Some("bytes=-100000000"));
        assert_eq!(tail.body.len() as u64, MAX_RANGE_BODY);
        assert_eq!(
            tail.body.last(),
            Some(&b'z'),
            "a suffix range keeps the end of the file"
        );
        assert_eq!(
            tail.content_range,
            Some(format!(
                "bytes {}-{}/{size}",
                size - MAX_RANGE_BODY,
                size - 1
            ))
        );
        let small = serve(&dir, &access, "/asset", &query, Some("bytes=0-9"));
        assert_eq!(small.body.len(), 10, "a range under the cap is untouched");
    }

    #[test]
    fn registered_files_are_served_with_ranges_and_others_are_refused() {
        let dir = scratch("serve");
        std::fs::write(dir.join("clip.webm"), b"0123456789").unwrap();
        std::fs::write(dir.join("secret.txt"), b"nope").unwrap();
        let mut access = AssetAccess::default();
        access.register(&json!({"source": {"type": "file", "path": dir.join("clip.webm")}}));
        let query = format!("p={}", encode(&dir.join("clip.webm")));

        let whole = serve(&dir, &access, "/asset", &query, None);
        assert_eq!(
            (whole.status, whole.body.as_slice(), whole.content_type),
            (200, &b"0123456789"[..], "video/webm")
        );
        let part = serve(&dir, &access, "/asset", &query, Some("bytes=2-5"));
        assert_eq!((part.status, part.body.as_slice()), (206, &b"2345"[..]));
        assert_eq!(part.content_range.as_deref(), Some("bytes 2-5/10"));
        assert_eq!(
            serve(&dir, &access, "/asset", &query, Some("bytes=-3")).body,
            b"789"
        );
        assert_eq!(
            serve(&dir, &access, "/asset", &query, Some("bytes=50-60")).status,
            416
        );

        let secret = format!("p={}", encode(&dir.join("secret.txt")));
        assert_eq!(serve(&dir, &access, "/asset", &secret, None).status, 403);
        let traversal = format!("p={}", encode(&dir.join("x/../secret.txt")));
        assert_eq!(serve(&dir, &access, "/asset", &traversal, None).status, 403);
        // A registered file reached through `..` is the same file, and is allowed.
        let roundabout = format!("p={}", encode(&dir.join("x/../clip.webm")));
        assert_eq!(
            serve(&dir, &access, "/asset", &roundabout, None).body,
            b"0123456789"
        );
        assert_eq!(serve(&dir, &access, "/asset", "", None).status, 403);
        assert_eq!(serve(&dir, &access, "/other", "", None).status, 404);
    }

    #[test]
    fn frame_patterns_allow_numbered_frames_only() {
        let dir = scratch("pattern");
        std::fs::write(dir.join("f00001.png"), b"png").unwrap();
        let mut access = AssetAccess::default();
        access.register(
            &json!({"source": {"type": "frame_sequence", "pattern": dir.join("f%05d.png")}}),
        );
        assert!(access.allows(dir.join("f00001.png").to_str().unwrap()));
        assert!(access.allows(dir.join("f123456.png").to_str().unwrap()));
        assert!(
            !access.allows(dir.join("f0001.png").to_str().unwrap()),
            "fewer digits than the pattern pads"
        );
        assert!(!access.allows(dir.join("f00001.png.bak").to_str().unwrap()));
        assert!(!access.allows(dir.join("fabcde.png").to_str().unwrap()));
        access.clear();
        assert!(!access.allows(dir.join("f00001.png").to_str().unwrap()));
    }

    #[test]
    fn base64_round_trips_with_and_without_padding() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("aGVsbG8").unwrap(), b"hello");
        assert_eq!(decode_base64("").unwrap(), b"");
        assert_eq!(
            decode_base64("iVBORw0KGgo=").unwrap(),
            [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
        );
        assert!(decode_base64("a*b").is_none());
        let dir = scratch("png");
        write_png(&dir.join("a/b.png"), "aGVsbG8=").unwrap();
        assert_eq!(std::fs::read(dir.join("a/b.png")).unwrap(), b"hello");
        assert!(write_png(&dir.join("c.png"), "!!").is_err());
    }

    #[test]
    fn the_page_is_served_from_dist() {
        let dir = scratch("dist");
        std::fs::write(dir.join("index.html"), b"<html>").unwrap();
        let page = serve(&dir, &AssetAccess::default(), "/", "", None);
        assert_eq!(
            (page.status, page.content_type),
            (200, "text/html; charset=utf-8")
        );
        assert_eq!(
            serve(&dir, &AssetAccess::default(), "/runtime.js", "", None).status,
            404
        );
    }
}
