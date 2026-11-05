//! The transport path of a resource: a bundle `relativePath` turned into a canonical, portable
//! form that is safe to join under any root.
//!
//! WHY stricter than the importer: `directed_scene` only refuses what escapes the bundle folder
//! (absolute, `..`, drive letter, NUL). A transport root is read back on other systems (a package
//! travels to Windows, macOS volumes are case-insensitive), so this also refuses names those
//! systems would rewrite or alias. The declared `relativePath` stays untouched in the manifest;
//! `ResourcePath` is only what a materializer is allowed to create.
//!
//! The type is pure text: no filesystem access, no absolute path. Symbolic links and the real
//! containment of a joined path are the media adapter's job (a textual check cannot see them).

use std::fmt;

/// Longest accepted path, in bytes, after normalization.
pub const MAX_PATH_BYTES: usize = 1024;
/// Longest accepted path segment (a file or folder name), in bytes.
pub const MAX_SEGMENT_BYTES: usize = 255;
/// Deepest accepted path, counting the file name.
pub const MAX_SEGMENTS: usize = 32;
/// Folders that start like this are the materializer's scratch space and can never be a resource.
pub const RESERVED_PREFIX: &str = ".tessel-staging-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// Nothing is left after normalization (`""`, `"."`, `"./"`).
    Empty,
    /// Starts with a separator or a drive letter.
    Absolute,
    /// A `..` segment, however it is spelled with `/` or `\`.
    ParentSegment,
    /// A control character or one of `<>:"|?*`, which Windows and macOS Finder rewrite or refuse.
    InvalidCharacter(char),
    /// A segment ending in `.` or a space; Windows drops it, so `a.` and `a` would alias.
    TrailingDotOrSpace,
    /// A Windows device name (`CON`, `NUL`, `COM1`...) or the materializer's scratch prefix.
    Reserved(String),
    TooLong,
    TooDeep,
}

impl PathError {
    /// Stable code for reports and JSON.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Empty => "path_empty",
            Self::Absolute => "path_absolute",
            Self::ParentSegment => "path_parent_segment",
            Self::InvalidCharacter(_) => "path_invalid_character",
            Self::TrailingDotOrSpace => "path_trailing_dot_or_space",
            Self::Reserved(_) => "path_reserved_name",
            Self::TooLong => "path_too_long",
            Self::TooDeep => "path_too_deep",
        }
    }
}

impl fmt::Display for PathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "the path is empty"),
            Self::Absolute => write!(formatter, "the path is absolute"),
            Self::ParentSegment => write!(formatter, "the path climbs out of its folder (`..`)"),
            Self::InvalidCharacter(character) => write!(
                formatter,
                "the path contains {character:?}, which is not portable"
            ),
            Self::TrailingDotOrSpace => {
                write!(formatter, "a path segment ends with a dot or a space")
            }
            Self::Reserved(name) => write!(formatter, "`{name}` is a reserved name"),
            Self::TooLong => write!(
                formatter,
                "the path or one of its segments is too long (at most {MAX_PATH_BYTES} / {MAX_SEGMENT_BYTES} bytes)"
            ),
            Self::TooDeep => write!(formatter, "the path is deeper than {MAX_SEGMENTS} segments"),
        }
    }
}

impl std::error::Error for PathError {}

/// A normalized relative path: `/` separators, no empty or `.` segments, never absolute.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourcePath {
    text: String,
}

impl ResourcePath {
    /// Normalizes a declared `relativePath`.
    ///
    /// `\` is a separator on every platform (the bundle is cross-platform), so a file literally
    /// named `a\b` cannot be addressed. Empty and `.` segments are dropped (`a//./b` is `a/b`);
    /// `..` is refused, never resolved.
    pub fn parse(declared: &str) -> Result<Self, PathError> {
        if declared.len() > MAX_PATH_BYTES {
            return Err(PathError::TooLong);
        }
        if declared.starts_with(['/', '\\']) || has_drive_prefix(declared) {
            return Err(PathError::Absolute);
        }
        let mut segments: Vec<&str> = Vec::new();
        for segment in declared.split(['/', '\\']) {
            match segment {
                "" | "." => {}
                ".." => return Err(PathError::ParentSegment),
                _ => {
                    check_segment(segment)?;
                    segments.push(segment);
                }
            }
        }
        if segments.is_empty() {
            return Err(PathError::Empty);
        }
        if segments.len() > MAX_SEGMENTS {
            return Err(PathError::TooDeep);
        }
        Ok(Self {
            text: segments.join("/"),
        })
    }

    /// The normalized text, `/`-separated.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Folders and then the file name, in order. Never empty.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.text.split('/')
    }

    /// The text two paths must not share on a case-insensitive volume. `A/b.png` and `a/B.png`
    /// have the same key. Unicode normalization forms are not folded (limit of this check).
    pub fn collision_key(&self) -> String {
        self.text.to_lowercase()
    }
}

impl fmt::Display for ResourcePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

fn has_drive_prefix(path: &str) -> bool {
    let mut characters = path.chars();
    matches!(
        (characters.next(), characters.next()),
        (Some(first), Some(':')) if first.is_ascii_alphabetic()
    )
}

fn check_segment(segment: &str) -> Result<(), PathError> {
    if segment.len() > MAX_SEGMENT_BYTES {
        return Err(PathError::TooLong);
    }
    if let Some(bad) = segment
        .chars()
        .find(|character| character.is_control() || "<>:\"|?*".contains(*character))
    {
        return Err(PathError::InvalidCharacter(bad));
    }
    if segment.ends_with(['.', ' ']) {
        return Err(PathError::TrailingDotOrSpace);
    }
    if segment.starts_with(RESERVED_PREFIX) || is_device_name(segment) {
        return Err(PathError::Reserved(segment.to_string()));
    }
    Ok(())
}

/// `CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`, `LPT1`-`LPT9`, with or without an extension.
fn is_device_name(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or(segment);
    let upper = stem.trim_end().to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let bytes = upper.as_bytes();
    bytes.len() == 4
        && (upper.starts_with("COM") || upper.starts_with("LPT"))
        && matches!(bytes[3], b'1'..=b'9')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_separators_and_dots() {
        let path = ResourcePath::parse("assets\\.//deep/./hero.png").unwrap();
        assert_eq!(path.as_str(), "assets/deep/hero.png");
        assert_eq!(
            path.segments().collect::<Vec<_>>(),
            ["assets", "deep", "hero.png"]
        );
        assert_eq!(path.to_string(), "assets/deep/hero.png");
    }

    #[test]
    fn refuses_what_could_leave_the_root() {
        assert_eq!(ResourcePath::parse("/etc/passwd"), Err(PathError::Absolute));
        assert_eq!(ResourcePath::parse("\\share\\x"), Err(PathError::Absolute));
        assert_eq!(ResourcePath::parse("C:/x.png"), Err(PathError::Absolute));
        assert_eq!(ResourcePath::parse("c:x.png"), Err(PathError::Absolute));
        assert_eq!(
            ResourcePath::parse("../x.png"),
            Err(PathError::ParentSegment)
        );
        assert_eq!(
            ResourcePath::parse("a/../x.png"),
            Err(PathError::ParentSegment)
        );
        assert_eq!(
            ResourcePath::parse("a\\..\\x.png"),
            Err(PathError::ParentSegment)
        );
        assert_eq!(ResourcePath::parse(""), Err(PathError::Empty));
        assert_eq!(ResourcePath::parse("./"), Err(PathError::Empty));
        assert!(matches!(
            ResourcePath::parse("a\0b.png"),
            Err(PathError::InvalidCharacter('\0'))
        ));
    }

    #[test]
    fn refuses_names_other_systems_rewrite() {
        assert!(matches!(
            ResourcePath::parse("a/b:c.png"),
            Err(PathError::InvalidCharacter(':'))
        ));
        assert!(matches!(
            ResourcePath::parse("a/b?.png"),
            Err(PathError::InvalidCharacter('?'))
        ));
        assert_eq!(
            ResourcePath::parse("a/name."),
            Err(PathError::TrailingDotOrSpace)
        );
        assert_eq!(
            ResourcePath::parse("a/name "),
            Err(PathError::TrailingDotOrSpace)
        );
        assert!(matches!(
            ResourcePath::parse("a/CON.png"),
            Err(PathError::Reserved(_))
        ));
        assert!(matches!(
            ResourcePath::parse("nul"),
            Err(PathError::Reserved(_))
        ));
        assert!(matches!(
            ResourcePath::parse("lpt3.txt"),
            Err(PathError::Reserved(_))
        ));
        assert!(ResourcePath::parse("a/COM0.png").is_ok());
        assert!(ResourcePath::parse("a/console.png").is_ok());
        assert!(matches!(
            ResourcePath::parse(".tessel-staging-x/f.png"),
            Err(PathError::Reserved(_))
        ));
    }

    #[test]
    fn bounds_length_and_depth() {
        assert_eq!(
            ResourcePath::parse(&"x".repeat(MAX_SEGMENT_BYTES + 1)),
            Err(PathError::TooLong)
        );
        assert!(ResourcePath::parse(&"x".repeat(MAX_SEGMENT_BYTES)).is_ok());
        assert_eq!(
            ResourcePath::parse(&"a/".repeat(MAX_PATH_BYTES)),
            Err(PathError::TooLong)
        );
        let deep = vec!["d"; MAX_SEGMENTS + 1].join("/");
        assert_eq!(ResourcePath::parse(&deep), Err(PathError::TooDeep));
        let deepest = vec!["d"; MAX_SEGMENTS].join("/");
        assert!(ResourcePath::parse(&deepest).is_ok());
    }

    #[test]
    fn collision_key_folds_case_only() {
        let upper = ResourcePath::parse("Assets/Hero.PNG").unwrap();
        let lower = ResourcePath::parse("assets/hero.png").unwrap();
        assert_ne!(upper, lower);
        assert_eq!(upper.collision_key(), lower.collision_key());
    }

    #[test]
    fn codes_are_stable_and_messages_readable() {
        assert_eq!(PathError::Absolute.code(), "path_absolute");
        assert_eq!(
            PathError::Reserved("CON".into()).code(),
            "path_reserved_name"
        );
        assert!(PathError::InvalidCharacter(':').to_string().contains(':'));
    }
}
