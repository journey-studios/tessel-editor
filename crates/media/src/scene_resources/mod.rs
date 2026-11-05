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
//! | [`session`] | Ownership, generation, release, byte budget; a late job result cannot install |
//! | [`load`] | The job body that produces a map |
//! | [`adopt`] | Copy from where a bundle was authored into the managed base (also the relink) |
//! | [`pack`] | What a package carries and proves for scenes |
//! | [`import`] | Carrying scenes between projects, renaming IDs without losing references |
//!
//! Contract and limits: `docs/features/scene-resource-integration.md`.

pub mod adopt;
pub mod base;
pub mod import;
pub mod load;
pub mod map;
pub(crate) mod pack;
pub mod session;

pub use adopt::{Adoption, adopt};
pub use base::{BaseKind, MANAGED_DIR, ResourceBase};
pub use load::{LoadRequest, check, load};
pub use map::{
    ApprovedResource, Generation, ResolveError, ResolvedResource, ResolvedResourceMap,
    ResourceResolver,
};
pub use session::{DEFAULT_BUDGET_BYTES, Invalidation, SceneResourceSession};
