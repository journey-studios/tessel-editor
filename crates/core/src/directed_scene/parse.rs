//! Stage 1: shape and vocabulary. Turns the JSON tree into a `Draft` or reports why not.
//!
//! The field list of every object is that of `render-bundle.schema.ts` (snapshot d6adac15). A
//! field the origin declares but BB01/BB02 do not interpret is marked `deferred` and stays in the
//! raw tree. Values the origin accepts in the schema and then refuses in its own integrity rules
//! (`premultiplied`, `rec709-video`, `supportPlane`, dissolve, ...) are refused here, with the
//! origin's code, as soon as they are read: the typed model only holds supported vocabulary.

use std::collections::{BTreeSet, HashSet};

use serde_json::Value;

use crate::time::Rate;

use super::SCHEMA_VERSION;
use super::graph::SceneDef;
use super::issue::{
    FPS_OUT_OF_RANGE, INVALID_VALUE, UNSAFE_RELATIVE_PATH, UNSUPPORTED_SCHEMA_VERSION,
};
use super::model::{
    Asset, AssetAlpha, AssetColorSpace, AssetFormat, AssetRef, AudioMix, CameraClip, CameraKey,
    CameraMotion, CameraSegment, ChannelKind, ChannelMotion, ChannelProperty, CompositionPass, Fps,
    MoveRecipe, Node, NodeKind, PerformanceChannel, PerformanceTrack, RenderProfile, SceneInstance,
    SegmentStart, Shot, Socket, SocketAttachment, Transition, ZoomRecipe,
};
use super::read::{Arr, Bound, Obj, Sink, at, child, kind_of};
use super::transform::Transform;

/// Everything stage 1 read, before the integrity rules (stage 2) run.
pub(super) struct Draft {
    pub bundle_id: String,
    pub revision_id: String,
    pub fps: Fps,
    pub edit_duration_frames: u64,
    pub render_profile: RenderProfile,
    pub profile_fps: Fps,
    pub assets: Vec<Asset>,
    pub defs: Vec<SceneDef>,
    pub instances: Vec<SceneInstance>,
    pub tracks: Vec<PerformanceTrack>,
    pub clips: Vec<CameraClip>,
    pub shots: Vec<Shot>,
    pub passes: Vec<CompositionPass>,
    pub audio: Option<AudioMix>,
    pub fx: super::fx::Fx,
    /// `metadata.pilotFrameAudit` is present (the origin refuses it: `LEGACY_FRAME_AUDIT`).
    pub legacy_frame_audit: bool,
}

fn version(obj: &Obj<'_>) -> bool {
    let Some(found) = obj.text("schemaVersion") else {
        return false;
    };
    if found == SCHEMA_VERSION {
        return true;
    }
    obj.sink_issue(
        UNSUPPORTED_SCHEMA_VERSION,
        "schemaVersion",
        format!("schemaVersion `{found}` is not supported; this importer reads exactly `{SCHEMA_VERSION}`"),
    );
    false
}

/// A path inside the bundle folder: not absolute, no `..` segment, no drive letter, no NUL.
fn safe_relative(path: &str) -> bool {
    let first = path.chars().next();
    let drive = path.len() >= 2 && path.as_bytes()[1] == b':';
    !(matches!(first, Some('/' | '\\'))
        || drive
        || path.contains('\0')
        || path.split(['/', '\\']).any(|segment| segment == ".."))
}

fn relative_path<'a>(obj: &Obj<'a>, key: &str) -> Option<&'a str> {
    let path = obj.id(key)?;
    if safe_relative(path) {
        return Some(path);
    }
    obj.sink_issue(
        UNSAFE_RELATIVE_PATH,
        key,
        format!("`{path}` must stay inside the bundle folder (no absolute path, no `..`)"),
    );
    None
}

fn fps(parent: &Obj<'_>, key: &str) -> Option<Fps> {
    let obj = parent.object(key)?;
    let (numerator, denominator) = (
        obj.count("numerator", Bound::Positive),
        obj.count("denominator", Bound::Positive),
    );
    let (numerator, denominator) = (numerator?, denominator?);
    let declared = u32::try_from(numerator)
        .ok()
        .zip(u32::try_from(denominator).ok())
        .filter(|&(n, d)| Rate::new(n, d).is_some());
    let Some((numerator, denominator)) = declared else {
        obj.sink_issue_at(
            FPS_OUT_OF_RANGE,
            obj.path(),
            format!(
                "{numerator}/{denominator} is outside the supported frame rates (at most 1000 fps, 32-bit terms)"
            ),
        );
        return None;
    };
    Some(Fps {
        numerator,
        denominator,
    })
}

fn render_profile(parent: &Obj<'_>) -> Option<(RenderProfile, Fps)> {
    let obj = parent.object("renderProfile")?;
    let (width, height) = (
        obj.count("width", Bound::Positive),
        obj.count("height", Bound::Positive),
    );
    let fps_value = fps(&obj, "fps");
    let color = obj.one_of("colorSpace", &["srgb-display", "rec709-video"]);
    let alpha = obj.one_of("alphaMode", &["straight", "premultiplied", "opaque"]);
    let premultiply = obj.flag("premultiplyOnUpload");
    let codec = obj.literal("videoCodec", "h264-yuv420p");
    let (three, browser) = (obj.id("threeRevision"), obj.id("browserChannel"));
    let mut supported = true;
    if color == Some("rec709-video") {
        supported = false;
        obj.sink_issue(
            "REC709_VIDEO_UNSUPPORTED",
            "colorSpace",
            "rec709-video is not implemented; use srgb-display (canvas is display-referred sRGB)",
        );
    }
    if alpha == Some("premultiplied") {
        supported = false;
        obj.sink_issue(
            "PREMULTIPLIED_ALPHA_UNSUPPORTED",
            "alphaMode",
            "premultiplied renderProfile.alphaMode is not implemented",
        );
    }
    if premultiply == Some(true) {
        supported = false;
        obj.sink_issue(
            "PREMULTIPLY_UNSUPPORTED",
            "premultiplyOnUpload",
            "premultiplyOnUpload is declared but not implemented; set false or use straight alpha",
        );
    }
    let fps_value = fps_value?;
    let profile = RenderProfile {
        width: width?,
        height: height?,
        fps: fps_value,
        three_revision: three?.to_string(),
        browser_channel: browser?.to_string(),
    };
    (supported && color.is_some() && alpha.is_some() && premultiply.is_some() && codec.is_some())
        .then_some((profile, fps_value))
}

fn asset(obj: &Obj<'_>) -> Option<Asset> {
    let asset_id = obj.id("assetId");
    let (revision, hash) = (obj.id("revisionId"), obj.id("contentHash"));
    let path = relative_path(obj, "relativePath");
    let format = obj.opt_one_of("format", &["png", "spz", "f32"]);
    let (width, height) = (
        obj.count("widthPx", Bound::Positive),
        obj.num("heightPx", Bound::Positive),
    );
    let alpha = obj.one_of("alphaMode", &["straight", "premultiplied", "opaque"]);
    let color = obj.one_of("colorSpace", &["srgb", "linear-srgb"]);
    if alpha == Some("premultiplied") {
        obj.sink_issue(
            "ASSET_PREMULTIPLIED_UNSUPPORTED",
            "alphaMode",
            format!(
                "Asset {} premultiplied alpha is not implemented",
                asset_id.unwrap_or("?")
            ),
        );
    }
    let alpha = match alpha? {
        "straight" => AssetAlpha::Straight,
        "opaque" => AssetAlpha::Opaque,
        _ => return None,
    };
    let format = match format? {
        Some("spz") => AssetFormat::Spz,
        Some("f32") => AssetFormat::F32,
        _ => AssetFormat::Png,
    };
    Some(Asset {
        asset_id: asset_id?.to_string(),
        revision_id: revision?.to_string(),
        content_hash: hash?.to_string(),
        relative_path: path?.to_string(),
        format,
        width_px: width?,
        height_px: height?,
        alpha,
        color_space: match color? {
            "linear-srgb" => AssetColorSpace::LinearSrgb,
            _ => AssetColorSpace::Srgb,
        },
    })
}

fn asset_ref(parent: &Obj<'_>) -> Option<Option<AssetRef>> {
    let Some(obj) = parent.opt_object("assetRef")? else {
        return Some(None);
    };
    let (asset_id, revision) = (obj.id("assetId"), obj.id("revisionId"));
    let (hash, path) = (obj.id("contentHash"), obj.id("relativePath"));
    Some(Some(AssetRef {
        asset_id: asset_id?.to_string(),
        revision_id: revision?.to_string(),
        content_hash: hash?.to_string(),
        relative_path: path?.to_string(),
    }))
}

/// `assetId` of `{ assetRef: { assetId } }` inside a section kept raw.
fn nested_asset_id(section: Option<&Value>) -> Option<String> {
    section?
        .get("assetRef")?
        .get("assetId")?
        .as_str()
        .map(str::to_string)
}

/// Sections of a scene definition that move or light the frame and that no evaluator computes yet.
/// A frame evaluation lists the ones a scene carries (`SceneGraph::unevaluated_sections`) so it
/// cannot be taken for the whole picture. Static data maps (surface, relief, depth) and audit
/// fields are not here: they hold no state that changes over time.
const UNEVALUATED_DEFINITION_SECTIONS: [&str; 7] = [
    "weather",
    "weatherKeys",
    "lighting",
    "effects",
    "weatherGround",
    "rigs",
    "paths",
];

/// The same for the sections that hang off a node: `(key, label reported)`.
const UNEVALUATED_NODE_SECTIONS: [(&str, &str); 2] =
    [("rigBind", "nodes[].rigBind"), ("reveal", "nodes[].reveal")];

/// Reads a section that is kept raw and reports whether the scene actually declares it (`null`
/// counts as absent: the origin's schema would refuse it, and it carries no state).
fn declared(section: Option<&Value>) -> bool {
    section.is_some_and(|value| !value.is_null())
}

fn node(
    obj: &Obj<'_>,
    data_assets: &mut HashSet<String>,
    unevaluated: &mut BTreeSet<&'static str>,
) -> Option<Node> {
    let id = obj.id("id");
    let parent_id = obj.nullable_text("parentId");
    let type_name = obj.one_of(
        "type",
        &["group", "imagePlane", "supportPlane", "overlay", "splat"],
    );
    let asset = asset_ref(obj);
    let position = obj.vec3("localPosition");
    let rotation = obj.quat("localRotation");
    let scale = obj.vec3("localScale");
    let width = obj.opt_num("widthSceneUnits", Bound::Positive);
    let height = obj.opt_num("heightSceneUnits", Bound::Positive);
    let render_order = obj.opt_int("renderOrder", Bound::Any);
    let socket_id = obj.opt_text("socketId");
    let attachment = match obj.opt_object("attachToSocket") {
        None => None,
        Some(None) => Some(None),
        Some(Some(inner)) => {
            let (owner, socket) = (inner.id("parentNodeId"), inner.id("socketName"));
            owner.zip(socket).map(|(owner, socket)| {
                Some(SocketAttachment {
                    owner: owner.to_string(),
                    socket: socket.to_string(),
                })
            })
        }
    };
    // Sections of lighting, weather and rig that hang off a node: kept raw (BB02 is the graph).
    let surface = obj.deferred("surface");
    let normal_map = obj.deferred("normalMap");
    for section in ["sceneDepth", "relief"] {
        obj.deferred(section);
    }
    for (key, label) in UNEVALUATED_NODE_SECTIONS {
        if declared(obj.deferred(key)) {
            unevaluated.insert(label);
        }
    }
    data_assets.extend(nested_asset_id(surface));
    data_assets.extend(nested_asset_id(normal_map));

    let kind = match type_name? {
        "group" => NodeKind::Group,
        "imagePlane" => NodeKind::ImagePlane,
        "splat" => NodeKind::Splat,
        other => {
            obj.sink_issue(
                "UNSUPPORTED_NODE_TYPE",
                "type",
                format!(
                    "Node type {other} is not supported in pilot ({})",
                    id.unwrap_or("?")
                ),
            );
            return None;
        }
    };
    Some(Node {
        id: id?.to_string(),
        parent_id: parent_id?.map(str::to_string),
        kind,
        asset: asset?,
        transform: Transform {
            position: position?,
            rotation: rotation?,
            scale: scale?,
        },
        width_scene_units: width?,
        height_scene_units: height?,
        render_order: render_order?,
        socket_id: socket_id?.map(str::to_string),
        attach_to_socket: attachment?,
    })
}

fn socket(obj: &Obj<'_>) -> Option<Socket> {
    let (owner, name) = (obj.id("nodeId"), obj.id("name"));
    let (position, rotation) = (obj.vec3("localPosition"), obj.quat("localRotation"));
    Some(Socket {
        owner: owner?.to_string(),
        name: name?.to_string(),
        position: position?,
        rotation: rotation?,
    })
}

fn scene_definition(obj: &Obj<'_>, pointer: String) -> Option<SceneDef> {
    let id = obj.id("id");
    let versioned = version(obj);
    let root = obj.id("rootNodeId");
    let mut data_assets = HashSet::new();
    let mut unevaluated = BTreeSet::new();
    let nodes = obj.array("nodes", 1).and_then(|array| {
        array.objects(|_, node_obj| node(node_obj, &mut data_assets, &mut unevaluated))
    });
    let sockets = match obj.opt_array("sockets", 0) {
        None => None,
        Some(None) => Some(Vec::new()),
        Some(Some(array)) => array.objects(|_, socket_obj| socket(socket_obj)),
    };
    for section in ["contactPlaneId", "frameAudit"] {
        obj.deferred(section);
    }
    for section in UNEVALUATED_DEFINITION_SECTIONS {
        if declared(obj.deferred(section)) {
            unevaluated.insert(section);
        }
    }
    versioned.then_some(())?;
    Some(SceneDef {
        id: id?.to_string(),
        root: root?.to_string(),
        nodes: nodes?,
        sockets: sockets?,
        data_assets,
        unevaluated: unevaluated.into_iter().collect(),
        pointer,
    })
}

fn instance(obj: &Obj<'_>) -> Option<SceneInstance> {
    let (id, definition) = (obj.id("id"), obj.id("sceneDefinitionId"));
    let duration = obj.count("durationFrames", Bound::Positive);
    let tracks = obj
        .array("performanceTrackIds", 0)
        .and_then(|array| array.strings());
    let locked = obj.opt_true("editLocked");
    Some(SceneInstance {
        id: id?.to_string(),
        scene_definition_id: definition?.to_string(),
        duration_frames: duration?,
        performance_track_ids: tracks?.into_iter().map(str::to_string).collect(),
        edit_locked: locked?,
    })
}

/// Cycles and events move a node by an offset; only a static channel may also replace its scale
/// (`performanceChannelSchema`).
const OFFSET_PROPERTIES: [&str; 2] = ["localRotation", "localPosition"];
const ALL_PROPERTIES: [&str; 3] = ["localRotation", "localPosition", "localScale"];

fn channel_property(channel: &Obj<'_>, allowed: &[&'static str]) -> Option<ChannelProperty> {
    Some(match channel.one_of("property", allowed)? {
        "localRotation" => ChannelProperty::LocalRotation,
        "localPosition" => ChannelProperty::LocalPosition,
        _ => ChannelProperty::LocalScale,
    })
}

/// The payload of a channel, every field read (and every problem recorded) even after one fails,
/// like the other objects of the bundle: the origin's zod parse reports all of them.
fn channel_motion(kind: ChannelKind, channel: &Obj<'_>) -> Option<ChannelMotion> {
    match kind {
        ChannelKind::Cycle => {
            let property = channel_property(channel, &OFFSET_PROPERTIES);
            let period = channel.count("periodFrames", Bound::Positive);
            let amplitude = channel.num("amplitude", Bound::Any);
            let axis = channel.vec3("axis");
            let phase_offset = channel.count("phaseOffsetFrames", Bound::NonNegative);
            Some(ChannelMotion::Cycle {
                property: property?,
                period_frames: period?,
                amplitude: amplitude?,
                axis: axis?,
                phase_offset_frames: phase_offset?,
            })
        }
        ChannelKind::Event => {
            let property = channel_property(channel, &OFFSET_PROPERTIES);
            let start = channel.count("startInstanceFrame", Bound::NonNegative);
            let duration = channel.count("durationFrames", Bound::Positive);
            let delta = channel.vec3("delta");
            // `holdFinal: z.literal(true)`: required, and `false` is not "no hold" but an error.
            let hold_final = match channel.flag("holdFinal") {
                Some(true) => Some(()),
                Some(false) => {
                    channel.sink_issue(
                        INVALID_VALUE,
                        "holdFinal",
                        "expected `true`: an event always holds its final value",
                    );
                    None
                }
                None => None,
            };
            hold_final?;
            Some(ChannelMotion::Event {
                property: property?,
                start_instance_frame: start?,
                duration_frames: duration?,
                delta: delta?,
            })
        }
        ChannelKind::Static => {
            let property = channel_property(channel, &ALL_PROPERTIES);
            let value = channel.vec3("value");
            Some(ChannelMotion::Static {
                property: property?,
                value: value?,
            })
        }
    }
}

fn performance_channel(channel: &Obj<'_>) -> Option<PerformanceChannel> {
    let kind = channel.one_of("kind", &["cycle", "event", "static"]);
    let (channel_id, node_id) = (channel.id("channelId"), channel.id("nodeId"));
    let kind = match kind? {
        "cycle" => ChannelKind::Cycle,
        "event" => ChannelKind::Event,
        _ => ChannelKind::Static,
    };
    let motion = channel_motion(kind, channel);
    Some(PerformanceChannel {
        kind,
        channel_id: channel_id?.to_string(),
        node_id: node_id?.to_string(),
        motion: motion?,
    })
}

fn performance_track(obj: &Obj<'_>) -> Option<PerformanceTrack> {
    let (id, instance_id) = (obj.id("id"), obj.id("instanceId"));
    let channels = obj
        .array("channels", 1)
        .and_then(|array| array.objects(|_, channel| performance_channel(channel)));
    Some(PerformanceTrack {
        id: id?.to_string(),
        instance_id: instance_id?.to_string(),
        channels: channels?,
    })
}

const MOVE_RECIPES: [(&str, MoveRecipe); 4] = [
    ("dollyIn", MoveRecipe::DollyIn),
    ("dollyOut", MoveRecipe::DollyOut),
    ("truckLeft", MoveRecipe::TruckLeft),
    ("truckRight", MoveRecipe::TruckRight),
];

const RECIPES: [&str; 9] = [
    "hold",
    "dollyIn",
    "dollyOut",
    "zoomIn",
    "zoomOut",
    "truckLeft",
    "truckRight",
    "orbit",
    "keyframes",
];

fn camera_key(obj: &Obj<'_>) -> Option<CameraKey> {
    let frame = obj.int("frame", Bound::Any);
    let (position, target) = (obj.vec3("position"), obj.vec3("target"));
    let (fov, easing) = (obj.num("fovDeg", Bound::Positive), obj.id("easing"));
    Some(CameraKey {
        frame: frame?,
        position: position?,
        target: target?,
        fov_deg: fov?,
        easing: easing?.to_string(),
    })
}

fn camera_segment(obj: &Obj<'_>) -> Option<CameraSegment> {
    let recipe = obj.one_of("recipe", &RECIPES);
    let duration = obj.count("durationFrames", Bound::Positive);
    let from = obj.one_of("from", &["initial", "previousEnd"]);
    let recipe = recipe?;
    let motion = match recipe {
        "hold" => Some(CameraMotion::Hold),
        "zoomIn" | "zoomOut" => {
            let (delta, easing) = (obj.num("fovDeltaDeg", Bound::Any), obj.id("easing"));
            Some(CameraMotion::Zoom {
                recipe: if recipe == "zoomIn" {
                    ZoomRecipe::In
                } else {
                    ZoomRecipe::Out
                },
                fov_delta_deg: delta?,
                easing: easing?.to_string(),
            })
        }
        "orbit" => {
            let yaw = obj.num("yawDeg", Bound::Any);
            let pitch = obj.opt_num("pitchDeg", Bound::Any);
            let easing = obj.id("easing");
            Some(CameraMotion::Orbit {
                yaw_deg: yaw?,
                pitch_deg: pitch?.unwrap_or(0.0),
                easing: easing?.to_string(),
            })
        }
        "keyframes" => {
            let keys = obj
                .array("keys", 1)
                .and_then(|array| array.objects(|_, key| camera_key(key)));
            keys.map(|keys| CameraMotion::Keyframes { keys })
        }
        _ => {
            let (distance, easing) = (obj.num("distanceSceneUnits", Bound::Any), obj.id("easing"));
            let recipe = MOVE_RECIPES
                .iter()
                .find(|(name, _)| *name == recipe)
                .map(|(_, recipe)| *recipe)?;
            Some(CameraMotion::Move {
                recipe,
                distance_scene_units: distance?,
                easing: easing?.to_string(),
            })
        }
    };
    Some(CameraSegment {
        duration_frames: duration?,
        from: match from? {
            "initial" => SegmentStart::Initial,
            _ => SegmentStart::PreviousEnd,
        },
        motion: motion?,
    })
}

fn segments(array: Arr<'_>) -> Option<Vec<CameraSegment>> {
    array.objects(|_, segment| camera_segment(segment))
}

fn camera_clip(obj: &Obj<'_>) -> Option<CameraClip> {
    let id = obj.id("id");
    let (position, target) = (obj.vec3("initialPosition"), obj.vec3("initialTarget"));
    let fov = obj.num("initialFovDeg", Bound::Positive);
    let main = obj.array("segments", 1).and_then(segments);
    let layers = match obj.opt_array("layers", 0) {
        None => None,
        Some(None) => Some(Vec::new()),
        Some(Some(array)) => {
            if array.len() > super::MAX_CAMERA_LAYERS {
                obj.sink_issue(
                    INVALID_VALUE,
                    "layers",
                    format!(
                        "a clip stacks at most {} tracks on its main one, got {}",
                        super::MAX_CAMERA_LAYERS,
                        array.len()
                    ),
                );
                None
            } else {
                array.objects(|_, layer| layer.array("segments", 1).and_then(segments))
            }
        }
    };
    Some(CameraClip {
        id: id?.to_string(),
        initial_position: position?,
        initial_target: target?,
        initial_fov_deg: fov?,
        segments: main?,
        layers: layers?,
    })
}

fn transition(parent: &Obj<'_>) -> Option<Option<Transition>> {
    let Some(obj) = parent.opt_object("transitionInto")? else {
        return Some(None);
    };
    let kind = obj.one_of("type", &["continuous", "cut", "dissolve"])?;
    match kind {
        "cut" => Some(Some(Transition::Cut {
            reason: obj.id("reason")?.to_string(),
        })),
        "dissolve" => {
            obj.count("durationFrames", Bound::Positive);
            obj.sink_issue(
                "DISSOLVE_UNSUPPORTED",
                "type",
                "Dissolve transitions are not supported in pilot MVP",
            );
            None
        }
        _ => Some(Some(Transition::Continuous)),
    }
}

fn shot(obj: &Obj<'_>) -> Option<Shot> {
    let (id, instance) = (obj.id("id"), obj.id("sceneInstanceId"));
    let range = obj.object("editRange").and_then(|range| {
        let (start, end) = (
            range.count("startFrame", Bound::NonNegative),
            range.count("endFrameExclusive", Bound::Positive),
        );
        start.zip(end)
    });
    let instance_start = obj.count("instanceStartFrame", Bound::NonNegative);
    let (clip, camera_in) = (
        obj.id("cameraClipId"),
        obj.count("cameraInFrame", Bound::NonNegative),
    );
    let transition_into = transition(obj);
    let intent = obj.id("intent");
    let (start_frame, end_frame_exclusive) = range?;
    Some(Shot {
        id: id?.to_string(),
        scene_instance_id: instance?.to_string(),
        start_frame,
        end_frame_exclusive,
        instance_start_frame: instance_start?,
        camera_clip_id: clip?.to_string(),
        camera_in_frame: camera_in?,
        transition_into: transition_into?,
        intent: intent?.to_string(),
    })
}

fn composition_pass(obj: &Obj<'_>) -> Option<CompositionPass> {
    let (fragment, node_id) = (obj.id("fragmentId"), obj.id("nodeId"));
    let stage = obj.literal("stage", "prePost");
    let order = obj.int("order", Bound::Any);
    stage?;
    Some(CompositionPass {
        fragment_id: fragment?.to_string(),
        node_id: node_id?.to_string(),
        order: order?,
    })
}

fn audio(parent: &Obj<'_>) -> Option<Option<AudioMix>> {
    let Some(obj) = parent.opt_object("audio")? else {
        return Some(None);
    };
    let path = relative_path(&obj, "mixRelativePath");
    let (start, end) = (
        obj.num("editStartSeconds", Bound::NonNegative),
        obj.num("editEndSecondsExclusive", Bound::Positive),
    );
    let hash = obj.opt_text("sourcePcmHashSha256");
    Some(Some(AudioMix {
        mix_relative_path: path?.to_string(),
        edit_start_seconds: start?,
        edit_end_seconds_exclusive: end?,
        source_pcm_hash_sha256: hash?.map(str::to_string),
    }))
}

/// Reads a whole bundle. `None` means at least one issue was recorded in `sink`.
pub(super) fn read_bundle(root: &Value, sink: &Sink) -> Option<Draft> {
    let obj = Obj::open(root, String::new(), sink)?;
    // Version gate: a different version has a different shape, so reading on would only bury
    // the one issue that matters under a pile of shape errors.
    if !version(&obj) {
        obj.defer_rest();
        return None;
    }
    let (bundle_id, revision_id) = (obj.id("bundleId"), obj.id("revisionId"));
    let bundle_fps = fps(&obj, "fps");
    let duration = obj.count("editDurationFrames", Bound::Positive);
    let profile = render_profile(&obj);
    let assets = obj
        .array("assets", 1)
        .and_then(|a| a.objects(|_, o| asset(o)));
    let defs = obj.array("sceneDefinitions", 1).and_then(|array| {
        array.objects(|index, def| scene_definition(def, at(&child("", "sceneDefinitions"), index)))
    });
    let instances = obj
        .array("sceneInstances", 1)
        .and_then(|a| a.objects(|_, o| instance(o)));
    let tracks = obj
        .array("performanceTracks", 0)
        .and_then(|a| a.objects(|_, o| performance_track(o)));
    let clips = obj
        .array("cameraClips", 1)
        .and_then(|a| a.objects(|_, o| camera_clip(o)));
    let shots = obj
        .array("shots", 1)
        .and_then(|a| a.objects(|_, o| shot(o)));
    let passes = obj
        .array("compositionPasses", 0)
        .and_then(|a| a.objects(|_, o| composition_pass(o)));
    let audio = audio(&obj);
    let fx = match obj.opt_object("fx") {
        Some(Some(fx_obj)) => super::fx::read_fx(&fx_obj, duration, sink),
        Some(None) => Some(super::fx::Fx::default()),
        None => None,
    };
    obj.deferred("narration");
    obj.deferred("outline");
    let metadata = obj.deferred("metadata");
    let mut legacy_frame_audit = false;
    match metadata {
        None => {}
        Some(Value::Object(map)) => legacy_frame_audit = map.contains_key("pilotFrameAudit"),
        Some(other) => sink.issue(
            super::issue::INVALID_TYPE,
            &obj.field_path("metadata"),
            format!("expected object, got {}", kind_of(other)),
        ),
    }
    let (profile, profile_fps) = profile?;
    Some(Draft {
        bundle_id: bundle_id?.to_string(),
        revision_id: revision_id?.to_string(),
        fps: bundle_fps?,
        edit_duration_frames: duration?,
        render_profile: profile,
        profile_fps,
        assets: assets?,
        defs: defs?,
        instances: instances?,
        tracks: tracks?,
        clips: clips?,
        shots: shots?,
        passes: passes?,
        audio: audio?,
        fx: fx?,
        legacy_frame_audit,
    })
}
