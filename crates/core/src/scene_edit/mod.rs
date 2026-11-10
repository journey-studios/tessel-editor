//! BB04 (minimal): a catalog of editable properties and atomic, ID-addressed edit batches over a
//! [`DirectedScene`](crate::directed_scene::DirectedScene).
//!
//! * [`catalog`] lists what can be edited: node `position`/`rotation`/`scale` (and `position/x`
//!   ... `scale/z` one component at a time), image-plane `width`/`height`/`renderOrder`, and the
//!   start pose of a camera clip (position and target by component too), each with a stable path,
//!   kind, unit, default, range and applicability.
//! * [`EditOp`] sets or resets one property of one node or clip by ID. No selection is involved
//!   and no op takes free-form JSON that skips the catalog.
//! * [`apply`] / [`dry_run`] take an immutable scene and a batch. Either every op is valid and
//!   the result is a candidate scene plus a summary, or the error names the op (index, path, ID)
//!   or the importer finding (JSON Pointer) that stopped it. The input scene never changes.
//!
//! Integrity rules are not duplicated: the candidate goes back through the importer
//! (`DirectedScene::import_value_with`) and its structural/temporal camera gates. A candidate
//! passes this shared subset; deferred domains and render certification remain outside it.
//!
//! ```
//! use tessel_editor_core::directed_scene::{DirectedScene, Vec3};
//! use tessel_editor_core::scene_edit::{
//!     EditOp, NodeProperty, PropertyRef, PropertyValue, apply,
//! };
//!
//! # fn demo(text: &str) -> Result<(), Box<dyn std::error::Error>> {
//! let scene = DirectedScene::import_str(text)?;
//! let ops = [EditOp::set(
//!     PropertyRef::node("street", "hero-plane", NodeProperty::Position),
//!     PropertyValue::Vec3(Vec3 { x: 4.0, y: 1.0, z: 0.0 }),
//! )];
//! let outcome = apply(&scene, &ops)?; // Err leaves `scene` exactly as it was
//! assert!(outcome.summary.modified);
//! # Ok(()) }
//! ```

mod batch;
mod camera_keys;
mod catalog;
mod error;
mod fx;
mod fx_animation;
mod fx_inspector;
mod fx_intent;
mod fx_keys;
mod op;
mod reference;
mod value;

pub use batch::{
    EditOutcome, EditSummary, PropertyChange, PropertyInfo, apply, apply_with, dry_run,
    dry_run_with, properties_of, properties_with_components, read_property,
};
pub use camera_keys::MovementConversion;
pub use catalog::{
    Applicability, CameraProperty, ComponentOf, Limit, NodeProperty, NumberRange, OriginRelation,
    PropertyId, PropertySpec, Unit, catalog,
};
pub use error::{
    CAMERA_CONVERSION_FAILED, CANDIDATE_REJECTED, DOCUMENT_MISMATCH, EditError, INVALID_OP,
    LocalizedIssue, NO_DEFAULT, NOT_APPLICABLE, UNKNOWN_CLIP, UNKNOWN_NODE, UNKNOWN_PATH,
    UNKNOWN_SCENE,
};
pub use fx::{
    AddClip, ClipField, FxChange, FxOp, FxOutcome, FxSummary, MAX_FX_BATCH_OPS, apply_fx,
    apply_fx_measured, apply_fx_with, check_fx_batch_len, dry_run_fx, parse_fx_batch,
};
pub use fx_animation::{
    ClipAnimation, ParamAnimation, REASON_FRAME_OUTSIDE_CLIP, REASON_KEY_ON_FRAME,
    REASON_NEEDS_TWO_KEYS, REASON_NO_FRAME, REASON_NO_KEYS, SeedAnimation,
};
pub use fx_inspector::{
    ClipInspector, Component, DURATION_LABEL, FADE_IN_LABEL, FADE_OUT_LABEL, FieldGroup, FieldKind,
    InspectorField, Offer, REASON_ANIMATED, REASON_FAMILY_NOT_YET, REASON_SINGLE_VALUE,
    START_LABEL, Target, has_reset, inspector_fields, layer_inspector, offers, reset_ops, scopes,
};
pub use fx_intent::{FxIntent, GESTURE_INTENT_MISMATCH, GESTURE_TARGET_MISMATCH, IntentRefusal};
pub use fx_keys::FxWork;
pub use fx_keys::STATIC_UNDER_KEYS;
pub use op::{EditOp, parse_batch};
pub use reference::PropertyRef;
pub use value::{PropertyValue, ValueKind};
