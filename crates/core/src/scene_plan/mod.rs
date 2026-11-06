//! The evaluated plan of one frame of a directed scene (BB05, first slice): the typed,
//! serializable state a renderer needs to draw an edit frame, resolved by the core.
//!
//! The plan is a pure function of an imported `DirectedScene` and an integer edit frame. It is
//! built on `Bundle::evaluate_frame` (shot mapping, camera, node transforms and world matrices,
//! performance channels) and on the existing graph (`SceneGraph`, `Bundle::paint_order`); nothing
//! about the camera, the animation or the matrices is computed again here. What the plan adds:
//!
//! - the **draw order** of the image planes, with the alpha mode of each layer, and the **splat
//!   nodes** (Spark, S7), each with the slot among the planes where a runtime paints it
//!   (`afterLayers`) and drawn with the node's world matrix, which must be a similarity (rotation,
//!   one positive uniform scale, translation: anything else is refused, never approximated);
//! - the stable list of **resources** the frame requires, keyed by the bundle's `assetId`
//!   (the identity; paths, revisions and hashes travel as metadata only);
//! - a **version** and the **capabilities** a runtime must have to execute it (`requires`);
//! - the **domains the frame declares and nobody evaluates** (lighting, weather, rigs, effects,
//!   paths, reveal, data maps), refused with a typed error, or listed when the caller
//!   explicitly accepts a partial plan; and the fields the importer kept without interpreting.
//!
//! `ScenePlan::build` lays the scene out again for every frame. For many frames of one scene (an
//! export, a scrub) use `PlanTimeline`: it returns the same plan or error for every frame, in any
//! order, preparing what does not depend on the frame once per shot (BL064).
//!
//! There is no IO and no renderer here: this is the contract between the core and a runtime, not
//! a render. The wire form is JSON (`to_json`/`from_json_str`), written by hand like the rest of
//! the core's serialization (the crate does not enable serde derive). Contract, policies and
//! limits: `docs/features/scene-plan.md`.
//!
//! ```
//! use tessel_editor_core::directed_scene::DirectedScene;
//! use tessel_editor_core::scene_plan::ScenePlan;
//!
//! # fn demo(text: &str) -> Result<(), Box<dyn std::error::Error>> {
//! let scene = DirectedScene::import_str(text)?;
//! let plan = ScenePlan::build(&scene, 12)?;          // Err(PlanError) leaves `scene` untouched
//! for asset_id in plan.required_asset_ids() {          // stable: ascending by assetId
//!     println!("needs {asset_id}");
//! }
//! let wire = plan.to_json_string();                    // what a runtime receives
//! assert_eq!(ScenePlan::from_json_str(&wire)?, plan);  // lossless
//! # Ok(()) }
//! ```

mod build;
mod error;
mod json;
mod model;
mod timeline;
mod validate;

pub use build::PlanOptions;
pub use error::PlanError;
pub use model::{
    Capabilities, Capability, Domain, PlanFrame, PlanLayer, PlanNode, PlanOutput, PlanParent,
    PlanResource, PlanSource, PlanSplat, ScenePlan, UninterpretedField, UninterpretedReason,
    UnsupportedDomain, capabilities,
};
pub use timeline::PlanTimeline;

/// `kind` of the serialized plan, so a runtime can tell it from any other JSON it is handed.
pub const PLAN_KIND: &str = "tessel-scene-plan";

/// Version of the plan's wire form and semantics. A plan of another version is refused with
/// `PlanError::UnsupportedVersion` before any of its fields is read: the contract only grows by
/// adding a version, never by changing the meaning of a field of this one.
pub const PLAN_VERSION: u32 = 1;

/// The one output color space `render-bundle` 1.0 supports (the importer refuses `rec709-video`),
/// written in every plan so a runtime does not have to assume it.
pub const OUTPUT_COLOR_SPACE: &str = "srgb-display";
