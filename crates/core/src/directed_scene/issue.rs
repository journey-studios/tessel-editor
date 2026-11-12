//! Findings of an import: blocking issues and non-blocking notices.
//!
//! Codes are stable strings. Where the origin (`render-bundle.schema.ts`,
//! `bundle-integrity.ts`, `scene-graph.ts`, `edit-joins.ts`) already names a rule, its code is
//! reused verbatim; the codes below are this importer's own, for rules the origin expresses only
//! as a zod type error or does not have (see `docs/features/directed-scene.md`).

use std::fmt;

use serde_json::{Value, json};

use crate::state::CommandError;

/// A value has the wrong JSON type (`expected string, got number`).
pub const INVALID_TYPE: &str = "INVALID_TYPE";
/// A required field is absent.
pub const MISSING_FIELD: &str = "MISSING_FIELD";
/// Right type, value outside what the format allows (range, enum, empty string, fractional int).
pub const INVALID_VALUE: &str = "INVALID_VALUE";
/// `schemaVersion` is not exactly the string `"1.0"`.
pub const UNSUPPORTED_SCHEMA_VERSION: &str = "UNSUPPORTED_SCHEMA_VERSION";
/// An array or the document is larger than the caller's `Limits`.
pub const LIMIT_EXCEEDED: &str = "LIMIT_EXCEEDED";
/// `relativePath` is absolute or climbs out of the bundle folder.
pub const UNSAFE_RELATIVE_PATH: &str = "UNSAFE_RELATIVE_PATH";
/// The frame rate is valid for the origin but outside `time::Rate` (more than 1000 fps, or not 32-bit).
pub const FPS_OUT_OF_RANGE: &str = "FPS_OUT_OF_RANGE";
/// A node's effective world matrix is not finite (overflow through a deep hierarchy).
pub const NON_FINITE_TRANSFORM: &str = "NON_FINITE_TRANSFORM";

/// Something that makes the bundle unusable. `path` is a JSON Pointer (RFC 6901) into the
/// document, `""` for the document itself. The origin reports only a coarse path; the pointer is
/// this importer's addition so an adapter can point at the exact value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub code: &'static str,
    pub path: String,
    pub message: String,
    /// What an adapter may act on besides the words (the legal interval of a shot, the ids there
    /// are). Additive: `None` for almost every code, and then absent from [`Self::to_json`].
    pub details: Option<Value>,
}

impl Issue {
    pub fn new(code: &'static str, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
            details: None,
        }
    }

    /// This issue with structured facts attached (see [`Self::details`]).
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn to_json(&self) -> Value {
        let mut issue = json!({ "code": self.code, "path": self.path, "message": self.message });
        if let Some(details) = &self.details {
            issue["details"] = details.clone();
        }
        issue
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} (at `{}`)", self.code, self.message, self.path)
    }
}

/// Information that does not block the import: the data is kept, the caller is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

/// A key the origin schema does not declare. The origin's zod parse drops it silently; this
/// importer keeps it verbatim and reports it here.
pub const UNKNOWN_FIELD: &str = "UNKNOWN_FIELD";

/// Why an import produced no scene.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportError {
    /// The text is larger than `Limits::max_bytes`.
    TooLarge { bytes: usize, max: usize },
    /// The text is not JSON, or nests deeper than `Limits::max_depth` (`line` 0 for a `Value`).
    Syntax {
        message: String,
        line: usize,
        column: usize,
    },
    /// The JSON is not a valid bundle. Never empty. Shape and vocabulary problems come first; the
    /// integrity rules (references, graph, shots) only run on a bundle whose shape is valid, since
    /// a reference into a field that failed to read would only add noise.
    Invalid { issues: Vec<Issue> },
}

impl ImportError {
    /// The issues of an `Invalid` error; empty for the other variants.
    pub fn issues(&self) -> &[Issue] {
        match self {
            Self::Invalid { issues } => issues,
            _ => &[],
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::TooLarge { .. } => "bundle_too_large",
            Self::Syntax { .. } => "invalid_json",
            Self::Invalid { .. } => "invalid_bundle",
        }
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes, max } => {
                write!(f, "bundle is {bytes} bytes; the limit is {max}")
            }
            Self::Syntax {
                message,
                line,
                column,
            } => write!(f, "not valid JSON ({line}:{column}): {message}"),
            Self::Invalid { issues } => match issues.first() {
                Some(first) if issues.len() > 1 => {
                    write!(f, "{first} (+{} more)", issues.len() - 1)
                }
                Some(first) => write!(f, "{first}"),
                None => write!(f, "invalid bundle"),
            },
        }
    }
}

impl std::error::Error for ImportError {}

/// Adapters (CLI, UI) already speak `CommandError`; the issue list travels in `details`.
impl From<ImportError> for CommandError {
    fn from(error: ImportError) -> Self {
        let message = error.to_string();
        let code = error.code();
        let command = CommandError::new(code, message);
        match &error {
            ImportError::Invalid { issues } => command.details(json!({
                "issues": issues.iter().map(Issue::to_json).collect::<Vec<_>>(),
            })),
            _ => command,
        }
    }
}
