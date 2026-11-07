//! Scene resources wired into the editor (BL010, BL011; F040): the layer between the pure
//! `ResourceManifest` / `directed_resources` of F038 and everything that needs the files of a
//! directed scene: a renderer, a project folder, a package, another project.
//!
//! ```text
//! DirectedScene ─ResourceManifest─▶ preflight ─▶ ResolvedResourceMap ─▶ SceneResourceSession ─▶ renderer
//!                      │                                  ▲ generation, budget, release
//!                      ├─adopt──▶ <workspace>/scenes/<id>/    (managed base, transportable)
//!                      ├─pack───▶ package  scenes/<id>/…      restore ▶ opens with no origin
//!                      └─import─▶ another project (IDs and paths without collisions)
//! ```
//!
//! No rule of F038 is repeated here: containment, symlinks, hashes, headers, limits, staging,
//! rollback, cancellation and the root lease all live in `directed_resources` and are reused.
//! What is new:
//!
//! | Module | Contract |
//! |---|---|
//! | [`base`] | An explicit, absolute base. Managed = `<workspace>/scenes/<id>`, derived (persistent and transportable); never the working directory |
//! | [`map`] | `assetId` → approved path + content/revision identity + generation; the [`ResourceResolver`] seam a renderer holds; typed [`ResolveError`] |
//! | [`snapshot`] | Private, read-only copies of the approved files, verified by hash while they are copied (BL109): bytes, not only paths, frozen for a long reader |
//! | [`session`] | Ownership, generation, release, byte budget; a late job result cannot install |
//! | [`load`] | The job body that produces a map |
//! | [`adopt`] | Copy from where a bundle was authored into the managed base (also the relink) |
//! | [`cleanup`] | What an adoption/import created and may remove again; the owner that removes it unless the result is accepted (BL089), checked by content (BL097) |
//! | `claims` | Who relies on a file a pending result created, so a discard never removes what another result reused (BL096); persisted per workspace under a lease so it holds across processes (BL111) |
//! | [`recover`] | What a result that died left claimed, taken back after the same content proof; the read-only claims view (BL111) |
//! | [`pack`] | What a package carries and proves for scenes |
//! | [`import`] | Carrying scenes between projects, renaming IDs without losing references |
//!
//! Contract and limits: `docs/features/scene-resource-integration.md`.

pub mod adopt;
pub mod base;
mod claims;
pub mod cleanup;
pub mod import;
pub mod load;
pub mod map;
pub(crate) mod pack;
pub mod recover;
#[cfg(all(test, unix))]
mod recover_budget;
pub mod session;
pub mod snapshot;

pub use adopt::{Adoption, AdoptionTarget, PendingAdoption, StaleAdoption, adopt};
pub use base::{BaseKind, MANAGED_DIR, ResourceBase};
pub use claims::tracked_under;
pub use cleanup::{CleanupReport, Created, Provisional};
pub use load::{LoadRequest, check, load, needs_files};
pub use map::{
    ApprovedResource, Generation, ResolveError, ResolvedResource, ResolvedResourceMap,
    ResourceResolver,
};
pub use recover::{Recovery, recover, recover_with, status as claims_status};
pub use session::{DEFAULT_BUDGET_BYTES, Invalidation, SceneResourceSession};
pub use snapshot::{ResourceSnapshot, snapshot_bytes};
