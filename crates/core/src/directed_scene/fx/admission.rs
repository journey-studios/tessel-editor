//! The one admission point of `fx.transitions` (F064, P4b S1a).
//!
//! The document model of a transition is built and validated, but the editor does not enter it
//! yet. The policy is a value stored on the [`DirectedScene`](crate::directed_scene::DirectedScene)
//! at import, and every edit candidate inherits it (`DirectedScene::reimport`), so there is no
//! place where a caller of the editor picks it:
//!
//! * [`Refuse`](TransitionAdmission::Refuse) is the default and the only policy of every public
//!   entry (`import_str`, `import_value`, `import_str_with`, `import_value_with`, `scene.attach`,
//!   open, package restore, `scene.edit`, `scene.fx.edit`). A non-empty `transitions` is refused
//!   as `FX_TRANSITION_NOT_YET`, byte for byte as before this layer existed.
//! * [`Accept`](TransitionAdmission::Accept) reads and validates the transitions. It is
//!   `#[doc(hidden)]` and reachable only through the `*_admitting` importers, for the core's own
//!   tests and the equivalence corpus across crates. **It is not a supported production API and
//!   not a security barrier**: nothing stops Rust code that links the crate from calling it. What
//!   the project guarantees is that no production handler (desktop, headless, ops, media, render)
//!   does: `tests/scene_fx_transition/boundary.rs` scans their sources for it.

/// Whether `fx.transitions` may be non-empty in a scene.
// WHY the allow: the hidden variant is not a "future variant" placeholder. It is a real policy that
// the tests of other crates must be able to name and match, which `#[non_exhaustive]` forbids.
#[allow(clippy::manual_non_exhaustive)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TransitionAdmission {
    /// Refuses a non-empty `transitions` with `FX_TRANSITION_NOT_YET` (every public entry).
    #[default]
    Refuse,
    /// Reads and validates the transitions. Hidden: see the module documentation.
    #[doc(hidden)]
    Accept,
}

impl TransitionAdmission {
    /// Whether transitions are read at all.
    pub fn admits(self) -> bool {
        matches!(self, Self::Accept)
    }
}
