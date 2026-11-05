//! Why a plan could not be built or read. Nothing here panics; every variant is a refusal the
//! caller can show, and `From<PlanError> for CommandError` lets adapters report it unchanged.

use std::fmt;

use serde_json::{Value, json};

use crate::directed_scene::TemporalError;
use crate::state::CommandError;

use super::model::{Capability, UnsupportedDomain};

#[derive(Debug, Clone, PartialEq)]
pub enum PlanError {
    /// The frame itself could not be evaluated: out of range, no shot, a node that overflows, two
    /// channels writing one property... The evaluator's own error, unchanged.
    Frame(TemporalError),
    /// The frame declares domains no plan of this version carries (lighting, weather, rigs,
    /// effects, paths, reveal, data maps, splats) and a complete plan was asked for. `domains` is
    /// never empty and is sorted.
    Unsupported { domains: Vec<UnsupportedDomain> },
    /// An id the plan needs does not resolve (`kind`: `"scene definition"`, `"asset"`, `"node"`).
    /// An imported scene never reaches this; the API stays total instead of panicking.
    Unresolved { kind: &'static str, id: String },
    /// The bundle document lacks a field the importer guarantees (`pointer` names it).
    MalformedDocument { pointer: String },
    /// The plan needs capabilities the runtime does not have.
    CapabilityUnavailable { missing: Vec<Capability> },
    /// A plan of a version this build does not read. Checked before any other field.
    UnsupportedVersion { found: u64, supported: u32 },
    /// A plan (JSON or hand-built) that breaks its own contract. `pointer` is a JSON Pointer into
    /// the plan.
    Invalid { pointer: String, message: String },
}

impl PlanError {
    /// Stable machine-readable code (what `CommandError::code` carries).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Frame(error) => error.code(),
            Self::Unsupported { .. } | Self::CapabilityUnavailable { .. } => "unsupported",
            Self::UnsupportedVersion { .. } => "unsupported_plan_version",
            Self::Unresolved { .. } | Self::MalformedDocument { .. } | Self::Invalid { .. } => {
                "invalid_plan"
            }
        }
    }

    pub(super) fn invalid(pointer: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Invalid {
            pointer: pointer.into(),
            message: message.into(),
        }
    }

    /// Structured detail for the errors that carry a list, for adapters and tests.
    pub fn details(&self) -> Option<Value> {
        match self {
            Self::Unsupported { domains } => Some(json!({
                "unsupported": "scene_plan_domains",
                "domains": domains.iter().map(|entry| json!({
                    "domain": entry.domain.as_str(),
                    "subject": entry.subject,
                })).collect::<Vec<_>>(),
            })),
            Self::CapabilityUnavailable { missing } => Some(json!({
                "unsupported": "scene_plan_capabilities",
                "missing": missing.iter().map(|capability| capability.as_str()).collect::<Vec<_>>(),
            })),
            _ => None,
        }
    }
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frame(error) => write!(f, "{error}"),
            Self::Unsupported { domains } => {
                let names: Vec<String> = domains
                    .iter()
                    .map(|entry| match &entry.subject {
                        Some(node) => format!("{} on {node}", entry.domain.as_str()),
                        None => entry.domain.as_str().to_string(),
                    })
                    .collect();
                write!(
                    f,
                    "the frame declares {} that the scene plan does not evaluate",
                    names.join(", ")
                )
            }
            Self::Unresolved { kind, id } => write!(f, "Unknown {kind}: {id}"),
            Self::MalformedDocument { pointer } => {
                write!(f, "the bundle document has no valid field at `{pointer}`")
            }
            Self::CapabilityUnavailable { missing } => {
                let names: Vec<&str> = missing.iter().map(|c| c.as_str()).collect();
                write!(f, "the runtime lacks: {}", names.join(", "))
            }
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "scene plan version {found} is not supported; this build reads version {supported}"
            ),
            Self::Invalid { pointer, message } => {
                write!(f, "invalid scene plan at `{pointer}`: {message}")
            }
        }
    }
}

impl std::error::Error for PlanError {}

impl From<TemporalError> for PlanError {
    fn from(error: TemporalError) -> Self {
        Self::Frame(error)
    }
}

/// Adapters (CLI, UI) already speak `CommandError`.
impl From<PlanError> for CommandError {
    fn from(error: PlanError) -> Self {
        let command = CommandError::new(error.code(), error.to_string());
        match error.details() {
            Some(details) => command.details(details),
            None => command,
        }
    }
}
