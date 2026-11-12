//! The wire form of the plan: stable camelCase JSON, written and read by hand.
//!
//! WHY by hand: `tessel-editor-core` depends on `serde` without the `derive` feature, like the rest
//! of the core's serialization (`FrameEvaluation::to_json`, `Command::to_json`), and this module
//! must not change `Cargo.toml`. The cost is two small mirror images (`to_json`, `from_json`); the
//! round-trip test keeps them honest.
//!
//! Every optional field is written (`null` when absent) and required on read, and an unknown key
//! is refused: a plan that carried data this build ignores would not round-trip. New data that
//! changes meaning means a new `PLAN_VERSION`, which `from_json` refuses by version before it
//! reads anything else. The one additive exception is `splats` (S7): written only when the scene
//! has splat nodes, so plans without splats are byte-identical to before.
//! Numbers are the exact `f64`s (`serde_json/float_roundtrip`); times are exact reduced ratios.
//! Keys come out in `serde_json`'s map order (sorted), so equal plans give equal text.

use serde_json::{Map, Value, json};

use crate::directed_scene::fx::{FxPass, FxValue, ParamKind, effect};
use crate::directed_scene::lighting::{LightingReason, Rgb};
use crate::directed_scene::{
    AssetAlpha, AssetColorSpace, AssetFormat, CameraState, Fps, Mat4, NodeAnimation, NodeKind, Vec3,
};
use crate::time::Ratio;

use super::model::{
    Capability, Domain, PlanFrame, PlanLayer, PlanLight, PlanNode, PlanOutput, PlanParent,
    PlanResource, PlanSource, PlanSplat, ScenePlan, UninterpretedField, UninterpretedReason,
    UnsupportedDomain,
};
use super::{OUTPUT_COLOR_SPACE, PLAN_KIND, PLAN_VERSION, PlanError};

// ---- writing -------------------------------------------------------------------------------

fn ratio_json(value: Ratio) -> Value {
    json!({ "numerator": value.num(), "denominator": value.den() })
}

fn fps_json(fps: Fps) -> Value {
    json!({ "numerator": fps.numerator, "denominator": fps.denominator })
}

fn alpha_str(alpha: AssetAlpha) -> &'static str {
    match alpha {
        AssetAlpha::Straight => "straight",
        AssetAlpha::Opaque => "opaque",
    }
}

fn color_space_str(color_space: AssetColorSpace) -> &'static str {
    match color_space {
        AssetColorSpace::Srgb => "srgb",
        AssetColorSpace::LinearSrgb => "linear-srgb",
    }
}

fn node_json(node: &PlanNode) -> Value {
    json!({
        "id": node.id,
        "kind": node.kind.as_str(),
        "parent": node.parent.as_ref().map(|parent| json!({
            "nodeId": parent.node_id,
            "socket": parent.socket,
        })),
        "localPosition": node.local_position.to_json(),
        "localRotation": node.local_rotation_deg.to_json(),
        "localScale": node.local_scale.to_json(),
        "localMatrix": node.local_matrix.0,
        "worldMatrix": node.world_matrix.0,
        "renderOrder": node.render_order,
        "assetId": node.asset_id,
        "widthSceneUnits": node.width_scene_units,
        "heightSceneUnits": node.height_scene_units,
    })
}

fn layer_json(layer: &PlanLayer) -> Value {
    json!({
        "nodeId": layer.node_id,
        "assetId": layer.asset_id,
        "alpha": alpha_str(layer.alpha),
        "renderOrder": layer.render_order,
        "passOrder": layer.pass_order,
    })
}

fn splat_json(splat: &PlanSplat) -> Value {
    json!({
        "nodeId": splat.node_id,
        "assetId": splat.asset_id,
        "afterLayers": splat.after_layers,
    })
}

fn resource_json(resource: &PlanResource) -> Value {
    json!({
        "assetId": resource.asset_id,
        "revisionId": resource.revision_id,
        "contentHash": resource.content_hash,
        "relativePath": resource.relative_path,
        "format": resource.format.as_str(),
        "widthPx": resource.width_px,
        "heightPx": resource.height_px,
        "alpha": alpha_str(resource.alpha),
        "colorSpace": color_space_str(resource.color_space),
        "usedBy": resource.used_by,
    })
}

fn light_json(light: &PlanLight) -> Value {
    let mut wire = json!({
        "id": light.id,
        "position": light.position.to_json(),
        "color": light.color.to_json(),
        "intensity": light.intensity,
        "radius": light.radius,
    });
    // Additive inside the light: only a hidden one says so.
    if light.hidden {
        wire["hidden"] = json!(true);
    }
    wire
}

fn read_light(light: &Obj<'_>) -> Result<PlanLight, PlanError> {
    light.only(&["id", "position", "color", "intensity", "radius", "hidden"])?;
    let color = light.object("color")?;
    color.only(&["r", "g", "b"])?;
    Ok(PlanLight {
        id: light.text("id")?,
        position: light.vec3("position")?,
        color: Rgb {
            r: color.number("r")?,
            g: color.number("g")?,
            b: color.number("b")?,
        },
        intensity: light.number("intensity")?,
        radius: light.number("radius")?,
        hidden: if light.map.contains_key("hidden") {
            light.boolean("hidden")?
        } else {
            false
        },
    })
}

impl ScenePlan {
    /// The wire form, for a runtime or an adapter. A hand-built plan with a non-finite number
    /// writes `null` there and `from_json` refuses it; `validate` says so first.
    pub fn to_json(&self) -> Value {
        let animation = match &self.animation {
            NodeAnimation::Static => json!({ "status": "static" }),
            NodeAnimation::Animated {
                track_ids,
                channels,
            } => json!({ "status": "animated", "trackIds": track_ids, "channels": channels }),
        };
        let camera = &self.camera;
        let mut wire = json!({
            "kind": PLAN_KIND,
            "planVersion": self.version,
            "complete": self.is_complete(),
            "requires": self.requires.iter().map(|capability| capability.as_str()).collect::<Vec<_>>(),
            "source": {
                "bundleId": self.source.bundle_id,
                "revisionId": self.source.revision_id,
                "sceneDefinitionId": self.source.scene_definition_id,
                "sceneInstanceId": self.source.scene_instance_id,
            },
            "output": {
                "width": self.output.width,
                "height": self.output.height,
                "fps": fps_json(self.output.fps),
                "colorSpace": OUTPUT_COLOR_SPACE,
                "alphaMode": alpha_str(self.output.alpha_mode),
            },
            "frame": {
                "editFrame": self.frame.edit_frame,
                "editTime": ratio_json(self.frame.edit_time),
                "shotId": self.frame.shot_id,
                "instanceFrame": self.frame.instance_frame,
                "instanceTime": ratio_json(self.frame.instance_time),
                "cameraClipId": self.frame.camera_clip_id,
                "cameraClipFrame": self.frame.camera_clip_frame,
                "cameraClipTime": ratio_json(self.frame.camera_clip_time),
            },
            "camera": {
                "position": camera.position.to_json(),
                "target": camera.target.to_json(),
                "fovDeg": camera.fov_deg,
                "velocity": camera.velocity.to_json(),
                "fovVelocityDegPerFrame": camera.fov_velocity_deg_per_frame,
            },
            "animation": animation,
            "nodes": self.nodes.iter().map(node_json).collect::<Vec<_>>(),
            "layers": self.layers.iter().map(layer_json).collect::<Vec<_>>(),
            "resources": self.resources.iter().map(resource_json).collect::<Vec<_>>(),
            "unsupported": self.unsupported.iter().map(|entry| {
                let mut item = json!({
                    "domain": entry.domain.as_str(),
                    "subject": entry.subject,
                });
                // Additive: written only when there is a reason (F060), so plans without one
                // keep their bytes.
                if let Some(reason) = entry.reason {
                    item["reason"] = json!(reason.as_str());
                }
                item
            }).collect::<Vec<_>>(),
            "uninterpreted": self.uninterpreted.iter().map(|field| json!({
                "pointer": field.pointer,
                "reason": field.reason.as_str(),
            })).collect::<Vec<_>>(),
        });
        // WHY only when there are splats: the key is additive. A plan without splats keeps the
        // exact bytes it had before S7 (the golden digests of the planner), and a runtime that
        // does not know splats ignores an absent key but refuses `requires: ["splats"]`.
        if !self.splats.is_empty() {
            wire["splats"] = Value::Array(self.splats.iter().map(splat_json).collect());
        }
        // The same for `fx` (F016): written only when the frame has effect passes.
        if !self.fx.is_empty() {
            wire["fx"] = Value::Array(self.fx.iter().map(FxPass::to_json).collect());
        }
        // The same for `lights` (F060): written only when the scene has point lights.
        if !self.lights.is_empty() {
            wire["lights"] = Value::Array(self.lights.iter().map(light_json).collect());
        }
        wire
    }

    /// Compact JSON text of `to_json`.
    pub fn to_json_string(&self) -> String {
        self.to_json().to_string()
    }

    /// Indented JSON text of `to_json`, ending in a newline.
    pub fn to_json_pretty(&self) -> String {
        // A `Value` always serializes; the fallback only keeps the signature total.
        let mut text = serde_json::to_string_pretty(&self.to_json()).unwrap_or_default();
        text.push('\n');
        text
    }
}

// ---- reading -------------------------------------------------------------------------------

/// A JSON object being read, with its JSON Pointer so every refusal names where it happened.
struct Obj<'a> {
    map: &'a Map<String, Value>,
    at: String,
}

fn as_obj(value: &Value, at: String) -> Result<Obj<'_>, PlanError> {
    match value.as_object() {
        Some(map) => Ok(Obj { map, at }),
        None => Err(PlanError::invalid(at, "expected an object")),
    }
}

impl<'a> Obj<'a> {
    fn path(&self, key: &str) -> String {
        format!("{}/{key}", self.at)
    }

    fn fail(&self, key: &str, message: &str) -> PlanError {
        PlanError::invalid(self.path(key), message)
    }

    fn field(&self, key: &str) -> Result<&'a Value, PlanError> {
        self.map
            .get(key)
            .ok_or_else(|| self.fail(key, "missing field"))
    }

    fn text_ref(&self, key: &str) -> Result<&'a str, PlanError> {
        self.field(key)?
            .as_str()
            .ok_or_else(|| self.fail(key, "expected a string"))
    }

    fn text(&self, key: &str) -> Result<String, PlanError> {
        self.text_ref(key).map(str::to_string)
    }

    fn nullable_text(&self, key: &str) -> Result<Option<String>, PlanError> {
        let value = self.field(key)?;
        if value.is_null() {
            return Ok(None);
        }
        value
            .as_str()
            .map(|text| Some(text.to_string()))
            .ok_or_else(|| self.fail(key, "expected a string or null"))
    }

    fn number(&self, key: &str) -> Result<f64, PlanError> {
        self.field(key)?
            .as_f64()
            .ok_or_else(|| self.fail(key, "expected a number"))
    }

    fn nullable_number(&self, key: &str) -> Result<Option<f64>, PlanError> {
        let value = self.field(key)?;
        if value.is_null() {
            return Ok(None);
        }
        value
            .as_f64()
            .map(Some)
            .ok_or_else(|| self.fail(key, "expected a number or null"))
    }

    fn integer(&self, key: &str) -> Result<i64, PlanError> {
        self.field(key)?
            .as_i64()
            .ok_or_else(|| self.fail(key, "expected an integer"))
    }

    fn nullable_integer(&self, key: &str) -> Result<Option<i64>, PlanError> {
        let value = self.field(key)?;
        if value.is_null() {
            return Ok(None);
        }
        value
            .as_i64()
            .map(Some)
            .ok_or_else(|| self.fail(key, "expected an integer or null"))
    }

    fn unsigned(&self, key: &str) -> Result<u64, PlanError> {
        self.field(key)?
            .as_u64()
            .ok_or_else(|| self.fail(key, "expected a non-negative integer"))
    }

    fn boolean(&self, key: &str) -> Result<bool, PlanError> {
        self.field(key)?
            .as_bool()
            .ok_or_else(|| self.fail(key, "expected true or false"))
    }

    fn object(&self, key: &str) -> Result<Obj<'a>, PlanError> {
        as_obj(self.field(key)?, self.path(key))
    }

    fn nullable_object(&self, key: &str) -> Result<Option<Obj<'a>>, PlanError> {
        let value = self.field(key)?;
        if value.is_null() {
            return Ok(None);
        }
        as_obj(value, self.path(key)).map(Some)
    }

    fn array(&self, key: &str) -> Result<&'a Vec<Value>, PlanError> {
        self.field(key)?
            .as_array()
            .ok_or_else(|| self.fail(key, "expected an array"))
    }

    fn items(&self, key: &str) -> Result<Vec<Obj<'a>>, PlanError> {
        self.array(key)?
            .iter()
            .enumerate()
            .map(|(position, item)| as_obj(item, format!("{}/{position}", self.path(key))))
            .collect()
    }

    fn texts(&self, key: &str) -> Result<Vec<String>, PlanError> {
        self.array(key)?
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| self.fail(key, "expected strings"))
            })
            .collect()
    }

    /// Refuses a key that is not in `allowed`.
    fn only(&self, allowed: &[&str]) -> Result<(), PlanError> {
        match self.map.keys().find(|key| !allowed.contains(&key.as_str())) {
            Some(key) => Err(self.fail(key, "unknown field")),
            None => Ok(()),
        }
    }

    fn vec3(&self, key: &str) -> Result<Vec3, PlanError> {
        let vector = self.object(key)?;
        vector.only(&["x", "y", "z"])?;
        Ok(Vec3 {
            x: vector.number("x")?,
            y: vector.number("y")?,
            z: vector.number("z")?,
        })
    }

    fn matrix(&self, key: &str) -> Result<Mat4, PlanError> {
        let items = self.array(key)?.as_slice();
        if items.len() != 16 {
            return Err(self.fail(key, "expected 16 numbers"));
        }
        let mut elements = [0.0_f64; 16];
        for (slot, item) in elements.iter_mut().zip(items) {
            *slot = item
                .as_f64()
                .ok_or_else(|| self.fail(key, "expected numbers"))?;
        }
        Ok(Mat4(elements))
    }

    fn ratio(&self, key: &str) -> Result<Ratio, PlanError> {
        let object = self.object(key)?;
        object.only(&["numerator", "denominator"])?;
        let (numerator, denominator) =
            (object.integer("numerator")?, object.integer("denominator")?);
        // Reduced only: a plan is canonical, so equal plans have equal text.
        Ratio::new(numerator, denominator)
            .filter(|ratio| ratio.num() == numerator && ratio.den() == denominator)
            .ok_or_else(|| self.fail(key, "expected a reduced ratio with a positive denominator"))
    }

    fn fps(&self, key: &str) -> Result<Fps, PlanError> {
        let object = self.object(key)?;
        object.only(&["numerator", "denominator"])?;
        let term = |name: &str| -> Result<u32, PlanError> {
            u32::try_from(object.unsigned(name)?).map_err(|_| object.fail(name, "too large"))
        };
        Ok(Fps {
            numerator: term("numerator")?,
            denominator: term("denominator")?,
        })
    }

    fn alpha(&self, key: &str) -> Result<AssetAlpha, PlanError> {
        match self.text_ref(key)? {
            "straight" => Ok(AssetAlpha::Straight),
            "opaque" => Ok(AssetAlpha::Opaque),
            _ => Err(self.fail(key, "expected `straight` or `opaque`")),
        }
    }

    fn color_space(&self, key: &str) -> Result<AssetColorSpace, PlanError> {
        match self.text_ref(key)? {
            "srgb" => Ok(AssetColorSpace::Srgb),
            "linear-srgb" => Ok(AssetColorSpace::LinearSrgb),
            _ => Err(self.fail(key, "expected `srgb` or `linear-srgb`")),
        }
    }

    fn format(&self, key: &str) -> Result<AssetFormat, PlanError> {
        match self.text_ref(key)? {
            "png" => Ok(AssetFormat::Png),
            "spz" => Ok(AssetFormat::Spz),
            "f32" => Ok(AssetFormat::F32),
            _ => Err(self.fail(key, "expected `png`, `spz` or `f32`")),
        }
    }

    fn node_kind(&self, key: &str) -> Result<NodeKind, PlanError> {
        match self.text_ref(key)? {
            "group" => Ok(NodeKind::Group),
            "imagePlane" => Ok(NodeKind::ImagePlane),
            "splat" => Ok(NodeKind::Splat),
            _ => Err(self.fail(key, "expected `group`, `imagePlane` or `splat`")),
        }
    }
}

fn read_node(node: &Obj<'_>) -> Result<PlanNode, PlanError> {
    node.only(&[
        "id",
        "kind",
        "parent",
        "localPosition",
        "localRotation",
        "localScale",
        "localMatrix",
        "worldMatrix",
        "renderOrder",
        "assetId",
        "widthSceneUnits",
        "heightSceneUnits",
    ])?;
    let parent = match node.nullable_object("parent")? {
        None => None,
        Some(parent) => {
            parent.only(&["nodeId", "socket"])?;
            Some(PlanParent {
                node_id: parent.text("nodeId")?,
                socket: parent.nullable_text("socket")?,
            })
        }
    };
    Ok(PlanNode {
        id: node.text("id")?,
        kind: node.node_kind("kind")?,
        parent,
        local_position: node.vec3("localPosition")?,
        local_rotation_deg: node.vec3("localRotation")?,
        local_scale: node.vec3("localScale")?,
        local_matrix: node.matrix("localMatrix")?,
        world_matrix: node.matrix("worldMatrix")?,
        render_order: node.integer("renderOrder")?,
        asset_id: node.nullable_text("assetId")?,
        width_scene_units: node.nullable_number("widthSceneUnits")?,
        height_scene_units: node.nullable_number("heightSceneUnits")?,
    })
}

fn read_layer(layer: &Obj<'_>) -> Result<PlanLayer, PlanError> {
    layer.only(&["nodeId", "assetId", "alpha", "renderOrder", "passOrder"])?;
    Ok(PlanLayer {
        node_id: layer.text("nodeId")?,
        asset_id: layer.text("assetId")?,
        alpha: layer.alpha("alpha")?,
        render_order: layer.integer("renderOrder")?,
        pass_order: layer.nullable_integer("passOrder")?,
    })
}

fn read_splat(splat: &Obj<'_>) -> Result<PlanSplat, PlanError> {
    splat.only(&["nodeId", "assetId", "afterLayers"])?;
    let after_layers = splat.integer("afterLayers")?;
    Ok(PlanSplat {
        node_id: splat.text("nodeId")?,
        asset_id: splat.text("assetId")?,
        after_layers: usize::try_from(after_layers)
            .map_err(|_| splat.fail("afterLayers", "expected a non-negative integer"))?,
    })
}

fn read_fx_value(params: &Obj<'_>, name: &str, kind: &ParamKind) -> Result<FxValue, PlanError> {
    match kind {
        ParamKind::Number { .. } => Ok(FxValue::Number(params.number(name)?)),
        ParamKind::Bool { .. } => Ok(FxValue::Bool(params.boolean(name)?)),
        ParamKind::Color { .. } => {
            let color = params.object(name)?;
            color.only(&["r", "g", "b"])?;
            Ok(FxValue::Color {
                r: color.number("r")?,
                g: color.number("g")?,
                b: color.number("b")?,
            })
        }
        ParamKind::Point { .. } => {
            let point = params.object(name)?;
            point.only(&["x", "y"])?;
            Ok(FxValue::Point {
                x: point.number("x")?,
                y: point.number("y")?,
            })
        }
    }
}

fn read_fx_pass(pass: &Obj<'_>) -> Result<FxPass, PlanError> {
    pass.only(&[
        "trackId",
        "clipId",
        "effect",
        "frameInClip",
        "weight",
        "amount",
        "params",
        "scope",
        "seed",
    ])?;
    let name = pass.text("effect")?;
    let spec = effect(&name).ok_or_else(|| pass.fail("effect", "not an effect of the catalog"))?;
    let scope = pass.object("scope")?;
    scope.only(&["kind"])?;
    if scope.text_ref("kind")? != "output" {
        return Err(scope.fail("kind", "only the `output` scope is supported"));
    }
    let raw = pass.object("params")?;
    raw.only(&spec.params.iter().map(|param| param.id).collect::<Vec<_>>())?;
    let mut params = Vec::with_capacity(spec.params.len());
    for param in spec.params {
        params.push((
            param.id.to_string(),
            read_fx_value(&raw, param.id, &param.kind)?,
        ));
    }
    Ok(FxPass {
        track_id: pass.text("trackId")?,
        clip_id: pass.text("clipId")?,
        effect: name,
        frame_in_clip: pass.integer("frameInClip")?,
        weight: pass.number("weight")?,
        amount: pass.number("amount")?,
        params,
        seed: read_fx_seed(pass)?,
    })
}

/// The seed of a noise pass: absent, or a whole number in `0..=u32::MAX` (never a float, a
/// negative number, a string or `null`; a seed that does not fit is refused, not truncated).
fn read_fx_seed(pass: &Obj<'_>) -> Result<Option<u32>, PlanError> {
    let Some(raw) = pass.map.get("seed") else {
        return Ok(None);
    };
    raw.as_u64()
        .and_then(|seed| u32::try_from(seed).ok())
        .map(Some)
        .ok_or_else(|| pass.fail("seed", "expected a whole number from 0 to 4294967295"))
}

fn read_resource(resource: &Obj<'_>) -> Result<PlanResource, PlanError> {
    resource.only(&[
        "assetId",
        "revisionId",
        "contentHash",
        "relativePath",
        "format",
        "widthPx",
        "heightPx",
        "alpha",
        "colorSpace",
        "usedBy",
    ])?;
    Ok(PlanResource {
        asset_id: resource.text("assetId")?,
        revision_id: resource.text("revisionId")?,
        content_hash: resource.text("contentHash")?,
        relative_path: resource.text("relativePath")?,
        format: resource.format("format")?,
        width_px: resource.unsigned("widthPx")?,
        height_px: resource.number("heightPx")?,
        alpha: resource.alpha("alpha")?,
        color_space: resource.color_space("colorSpace")?,
        used_by: resource.texts("usedBy")?,
    })
}

fn read_animation(root: &Obj<'_>) -> Result<NodeAnimation, PlanError> {
    let animation = root.object("animation")?;
    match animation.text_ref("status")? {
        "static" => {
            animation.only(&["status"])?;
            Ok(NodeAnimation::Static)
        }
        "animated" => {
            animation.only(&["status", "trackIds", "channels"])?;
            let channels = usize::try_from(animation.unsigned("channels")?)
                .map_err(|_| animation.fail("channels", "too large"))?;
            Ok(NodeAnimation::Animated {
                track_ids: animation.texts("trackIds")?,
                channels,
            })
        }
        _ => Err(animation.fail("status", "expected `static` or `animated`")),
    }
}

impl ScenePlan {
    /// Reads a plan from JSON text and validates it. `Err` for text that is not JSON, a plan of
    /// another `kind` or version, a missing or unknown field, or a plan that breaks `validate`.
    pub fn from_json_str(text: &str) -> Result<Self, PlanError> {
        let value: Value = serde_json::from_str(text)
            .map_err(|error| PlanError::invalid("", format!("not JSON: {error}")))?;
        Self::from_json(&value)
    }

    /// `from_json_str` for a tree the caller already has.
    pub fn from_json(value: &Value) -> Result<Self, PlanError> {
        let root = as_obj(value, String::new())?;
        // The kind and the version come first: a plan of a later version may have other fields,
        // and must be refused as such, not as a malformed one.
        if root.text_ref("kind")? != PLAN_KIND {
            return Err(root.fail("kind", "not a tessel scene plan"));
        }
        let found = root.unsigned("planVersion")?;
        if found != u64::from(PLAN_VERSION) {
            return Err(PlanError::UnsupportedVersion {
                found,
                supported: PLAN_VERSION,
            });
        }
        root.only(&[
            "kind",
            "planVersion",
            "complete",
            "requires",
            "source",
            "output",
            "frame",
            "camera",
            "animation",
            "nodes",
            "layers",
            "splats",
            "fx",
            "lights",
            "resources",
            "unsupported",
            "uninterpreted",
        ])?;

        let source = root.object("source")?;
        source.only(&[
            "bundleId",
            "revisionId",
            "sceneDefinitionId",
            "sceneInstanceId",
        ])?;
        let output = root.object("output")?;
        output.only(&["width", "height", "fps", "colorSpace", "alphaMode"])?;
        if output.text_ref("colorSpace")? != OUTPUT_COLOR_SPACE {
            return Err(output.fail("colorSpace", "only `srgb-display` is supported"));
        }
        let frame = root.object("frame")?;
        frame.only(&[
            "editFrame",
            "editTime",
            "shotId",
            "instanceFrame",
            "instanceTime",
            "cameraClipId",
            "cameraClipFrame",
            "cameraClipTime",
        ])?;
        let camera = root.object("camera")?;
        camera.only(&[
            "position",
            "target",
            "fovDeg",
            "velocity",
            "fovVelocityDegPerFrame",
        ])?;

        let requires = root
            .texts("requires")?
            .iter()
            .map(|name| {
                Capability::parse(name).ok_or_else(|| root.fail("requires", "unknown capability"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let unsupported = root
            .items("unsupported")?
            .iter()
            .map(|entry| -> Result<UnsupportedDomain, PlanError> {
                entry.only(&["domain", "subject", "reason"])?;
                let name = entry.text_ref("domain")?;
                Ok(UnsupportedDomain {
                    domain: Domain::parse(name)
                        .ok_or_else(|| entry.fail("domain", "unknown domain"))?,
                    subject: entry.nullable_text("subject")?,
                    reason: if entry.map.contains_key("reason") {
                        Some(
                            LightingReason::parse(entry.text_ref("reason")?)
                                .ok_or_else(|| entry.fail("reason", "unknown reason"))?,
                        )
                    } else {
                        None
                    },
                })
            })
            .collect::<Result<Vec<_>, PlanError>>()?;
        let uninterpreted = root
            .items("uninterpreted")?
            .iter()
            .map(|entry| -> Result<UninterpretedField, PlanError> {
                entry.only(&["pointer", "reason"])?;
                let reason = entry.text_ref("reason")?;
                Ok(UninterpretedField {
                    pointer: entry.text("pointer")?,
                    reason: UninterpretedReason::parse(reason)
                        .ok_or_else(|| entry.fail("reason", "unknown reason"))?,
                })
            })
            .collect::<Result<Vec<_>, PlanError>>()?;

        let plan = Self {
            version: PLAN_VERSION,
            source: PlanSource {
                bundle_id: source.text("bundleId")?,
                revision_id: source.text("revisionId")?,
                scene_definition_id: source.text("sceneDefinitionId")?,
                scene_instance_id: source.text("sceneInstanceId")?,
            },
            output: PlanOutput {
                width: output.unsigned("width")?,
                height: output.unsigned("height")?,
                fps: output.fps("fps")?,
                alpha_mode: output.alpha("alphaMode")?,
            },
            frame: PlanFrame {
                edit_frame: frame.unsigned("editFrame")?,
                edit_time: frame.ratio("editTime")?,
                shot_id: frame.text("shotId")?,
                instance_frame: frame.unsigned("instanceFrame")?,
                instance_time: frame.ratio("instanceTime")?,
                camera_clip_id: frame.text("cameraClipId")?,
                camera_clip_frame: frame.unsigned("cameraClipFrame")?,
                camera_clip_time: frame.ratio("cameraClipTime")?,
            },
            camera: CameraState {
                position: camera.vec3("position")?,
                target: camera.vec3("target")?,
                fov_deg: camera.number("fovDeg")?,
                velocity: camera.vec3("velocity")?,
                fov_velocity_deg_per_frame: camera.number("fovVelocityDegPerFrame")?,
            },
            animation: read_animation(&root)?,
            nodes: root
                .items("nodes")?
                .iter()
                .map(read_node)
                .collect::<Result<Vec<_>, _>>()?,
            layers: root
                .items("layers")?
                .iter()
                .map(read_layer)
                .collect::<Result<Vec<_>, _>>()?,
            // Optional (additive): absent means no splat nodes.
            splats: if root.map.contains_key("splats") {
                root.items("splats")?
                    .iter()
                    .map(read_splat)
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                Vec::new()
            },
            // Optional (additive): absent means no effect passes.
            fx: if root.map.contains_key("fx") {
                root.items("fx")?
                    .iter()
                    .map(read_fx_pass)
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                Vec::new()
            },
            // Optional (additive): absent means no lights.
            lights: if root.map.contains_key("lights") {
                root.items("lights")?
                    .iter()
                    .map(read_light)
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                Vec::new()
            },
            resources: root
                .items("resources")?
                .iter()
                .map(read_resource)
                .collect::<Result<Vec<_>, _>>()?,
            requires,
            unsupported,
            uninterpreted,
        };
        if root.boolean("complete")? != plan.is_complete() {
            return Err(root.fail(
                "complete",
                "must be true exactly when `unsupported` is empty",
            ));
        }
        plan.validate()?;
        Ok(plan)
    }
}
