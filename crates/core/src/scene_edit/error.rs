//! Why an edit batch produced no candidate.
//!
//! Codes are stable strings. Where the origin's property layer (`properties/types.ts`,
//! `PropertyErrorCode`) already names the failure, its code is reused verbatim (`UNKNOWN_PATH`,
//! `UNKNOWN_SCENE`, `UNKNOWN_CLIP`, `INVALID_VALUE`, `NOT_APPLICABLE`); `INVALID_TYPE` is the
//! importer's own code for a value of the wrong JSON shape. The rest are this module's.

use std::fmt;

use serde_json::{Map, Value, json};

use crate::directed_scene::{INVALID_TYPE, INVALID_VALUE, ImportError, Issue};
use crate::state::CommandError;

/// The path is not a property of the catalog (grammar, or the name after the target).
pub const UNKNOWN_PATH: &str = "UNKNOWN_PATH";
/// `scene/<sceneId>/...` names a scene definition the bundle does not have.
pub const UNKNOWN_SCENE: &str = "UNKNOWN_SCENE";
/// The scene has no node with that ID (node IDs are unique inside a scene only).
pub const UNKNOWN_NODE: &str = "UNKNOWN_NODE";
/// `camera/<clipId>/...` names a camera clip the bundle does not have.
pub const UNKNOWN_CLIP: &str = "UNKNOWN_CLIP";
/// The property exists but not for this node (`width` on a group).
pub const NOT_APPLICABLE: &str = "NOT_APPLICABLE";
/// `reset` on a property whose catalog entry has no default.
pub const NO_DEFAULT: &str = "NO_DEFAULT";
/// A JSON op is none of `{ "set", "value" }` or `{ "reset" }`.
pub const INVALID_OP: &str = "INVALID_OP";
/// Every op was valid, but the edited document is not a valid bundle for the importer.
pub const CANDIDATE_REJECTED: &str = "CANDIDATE_REJECTED";
/// The typed view and the document disagree about where an entity is. Cannot happen for a scene
/// the importer produced; reported instead of panicking so a future change in either side is loud.
pub const DOCUMENT_MISMATCH: &str = "DOCUMENT_MISMATCH";

/// An issue of the importer on the candidate, with the ops that touched the entity it points into.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalizedIssue {
    pub issue: Issue,
    /// Indexes of the ops that changed the node or clip the issue's JSON Pointer is inside.
    /// Empty when the issue is a consequence elsewhere (a child whose world matrix overflows
    /// because a parent was scaled) or concerns the document as a whole.
    pub ops: Vec<usize>,
}

impl LocalizedIssue {
    pub fn to_json(&self) -> Value {
        let mut value = self.issue.to_json();
        value["ops"] = json!(self.ops);
        value
    }
}

/// What the importer said about a candidate it refused.
#[derive(Debug, Clone, PartialEq)]
struct Rejection {
    issues: Vec<LocalizedIssue>,
    cause: ImportError,
}

/// A rejected batch. Nothing was applied: the input scene is immutable and no candidate exists.
#[derive(Debug, Clone, PartialEq)]
pub struct EditError {
    pub code: &'static str,
    pub message: String,
    /// Position in the batch of the op that failed. For `CANDIDATE_REJECTED`, the first op the
    /// importer's findings can be attributed to (`None` when none can).
    pub index: Option<usize>,
    /// Property path of the failing op, as the catalog spells it.
    pub path: Option<String>,
    /// The scene, node or clip ID that does not exist.
    pub id: Option<String>,
    // WHY boxed: it keeps `Result<_, EditError>` small on every call path (clippy's
    // `result_large_err`); only the rare candidate rejection carries it.
    rejection: Option<Box<Rejection>>,
}

impl EditError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            index: None,
            path: None,
            id: None,
            rejection: None,
        }
    }

    /// The importer's findings on the candidate (only for `CANDIDATE_REJECTED`), each with the
    /// ops that touched the entity it points into. Empty for every other code.
    pub fn issues(&self) -> &[LocalizedIssue] {
        self.rejection
            .as_ref()
            .map_or(&[], |rejection| rejection.issues.as_slice())
    }

    /// The importer's own error, whole, for callers that want its variant.
    pub fn cause(&self) -> Option<&ImportError> {
        self.rejection.as_ref().map(|rejection| &rejection.cause)
    }

    pub(super) fn rejected_by(mut self, issues: Vec<LocalizedIssue>, cause: ImportError) -> Self {
        self.rejection = Some(Box::new(Rejection { issues, cause }));
        self
    }

    pub fn at_op(mut self, index: usize) -> Self {
        self.index = Some(index);
        self
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// The `CommandError` code adapters already speak. A rejected candidate keeps the importer's
    /// code, so `invalid_bundle` means the same thing for an import and for an edit.
    fn command_code(&self) -> &'static str {
        match self.code {
            UNKNOWN_PATH => "unknown_path",
            UNKNOWN_SCENE => "unknown_scene",
            UNKNOWN_NODE => "unknown_node",
            UNKNOWN_CLIP => "unknown_clip",
            NOT_APPLICABLE => "not_applicable",
            NO_DEFAULT => "no_default",
            INVALID_OP => "invalid_op",
            CANDIDATE_REJECTED => self.cause().map_or("invalid_bundle", ImportError::code),
            DOCUMENT_MISMATCH => "document_mismatch",
            INVALID_TYPE => "invalid_type",
            INVALID_VALUE => "invalid_value",
            _ => "invalid_edit",
        }
    }
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.index {
            Some(index) => write!(f, "{}: operation {index}: {}", self.code, self.message),
            None => write!(f, "{}: {}", self.code, self.message),
        }
    }
}

impl std::error::Error for EditError {}

/// Adapters (CLI, UI) already speak `CommandError`; where the failure was and the importer's
/// findings travel in `details`, the missing ID in `target`.
impl From<EditError> for CommandError {
    fn from(error: EditError) -> Self {
        let mut command = CommandError::new(error.command_code(), error.message.clone());
        if let Some(id) = &error.id {
            command = command.target(id.clone());
        }
        let mut details = Map::new();
        if let Some(index) = error.index {
            details.insert("index".to_string(), json!(index));
        }
        if let Some(path) = &error.path {
            details.insert("path".to_string(), json!(path));
        }
        if !error.issues().is_empty() {
            let issues: Vec<Value> = error.issues().iter().map(LocalizedIssue::to_json).collect();
            details.insert("issues".to_string(), Value::Array(issues));
        }
        if details.is_empty() {
            return command;
        }
        command.details(Value::Object(details))
    }
}
