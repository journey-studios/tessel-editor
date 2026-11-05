//! Stage 2: the cross-reference and structure rules of a bundle whose shape is valid.
//!
//! Ported from `validateBundleIntegrity` (`bundle-integrity.ts`) with the origin's codes and
//! rule order, limited to what BB01/BB02 interpret: assets, scene graphs, instances, performance
//! track references, camera clip structure, shots, composition passes and audio. Rules about
//! lighting, weather, shadows, frame audit, surface/normal/relief maps and rig are listed in
//! `DEFERRED_RULES`. Rules that need the camera or animation evaluated (the composed field of
//! view of stacked layers, pose jumps at joins) wait for BB03.

use std::collections::{HashMap, HashSet};

use super::graph;
use super::issue::Issue;
use super::model::{
    AssetColorSpace, AssetFormat, CameraClip, CameraMotion, CameraSegment, SegmentStart,
};
use super::parse::Draft;
use super::read::{at, child};

/// Camera easing names the origin evaluates (`KNOWN_EASINGS`).
pub const KNOWN_EASINGS: [&str; 3] = ["linear", "easeInOutCubic", "easeOutCubic"];

fn duplicates<'a>(
    label: &str,
    code: &'static str,
    list_path: &str,
    ids: impl Iterator<Item = &'a str>,
    issues: &mut Vec<Issue>,
) {
    let mut seen = HashSet::new();
    for (index, id) in ids.enumerate() {
        if !seen.insert(id) {
            issues.push(Issue::new(
                code,
                at(list_path, index),
                format!("Duplicate {label} id {id}"),
            ));
        }
    }
}

pub(super) fn check(draft: &Draft, document: &serde_json::Value) -> Vec<Issue> {
    let mut issues = Vec::new();

    if draft.fps != draft.profile_fps {
        issues.push(Issue::new(
            "FPS_MISMATCH",
            "/fps",
            "bundle.fps must match renderProfile.fps",
        ));
    }

    assets(draft, &mut issues);

    let mut definition_ids = HashSet::new();
    for def in &draft.defs {
        if !definition_ids.insert(def.id.as_str()) {
            issues.push(Issue::new(
                "DUPLICATE_SCENE_DEFINITION_ID",
                &def.pointer,
                format!("Duplicate sceneDefinitions id {}", def.id),
            ));
        }
        graph::validate(def, &draft.assets, &mut issues);
    }

    light_parent_references(draft, document, &mut issues);
    instances_and_tracks(draft, &definition_ids, &mut issues);

    duplicates(
        "cameraClips",
        "DUPLICATE_ID",
        "/cameraClips",
        draft.clips.iter().map(|c| c.id.as_str()),
        &mut issues,
    );
    duplicates(
        "performanceTracks",
        "DUPLICATE_ID",
        "/performanceTracks",
        draft.tracks.iter().map(|t| t.id.as_str()),
        &mut issues,
    );
    duplicates(
        "shots",
        "DUPLICATE_ID",
        "/shots",
        draft.shots.iter().map(|s| s.id.as_str()),
        &mut issues,
    );
    for (index, clip) in draft.clips.iter().enumerate() {
        camera_clip(clip, &at("/cameraClips", index), &mut issues);
    }

    shots(draft, &mut issues);
    composition_passes(draft, &mut issues);

    if let Some(audio) = &draft.audio {
        if audio.edit_end_seconds_exclusive <= audio.edit_start_seconds {
            issues.push(Issue::new(
                "AUDIO_RANGE_INVALID",
                "/audio",
                "Audio end must exceed start",
            ));
        }
        // An empty hash counts as absent, as in the origin (`!hash`).
        if audio
            .source_pcm_hash_sha256
            .as_deref()
            .is_none_or(str::is_empty)
        {
            issues.push(Issue::new(
                "AUDIO_HASH_REQUIRED",
                "/audio",
                "audio.sourcePcmHashSha256 is required when audio.mixRelativePath is set",
            ));
        }
    }
    if draft.legacy_frame_audit {
        issues.push(Issue::new(
            "LEGACY_FRAME_AUDIT",
            "/metadata/pilotFrameAudit",
            "Use sceneDefinitions[].frameAudit; untyped pilotFrameAudit is unsupported",
        ));
    }
    issues
}

fn assets(draft: &Draft, issues: &mut Vec<Issue>) {
    // Surface maps, normal maps and f32 grids hold numbers read without colour conversion: the
    // only assets allowed to be `linear-srgb`.
    let mut data_assets: HashSet<&str> = draft
        .defs
        .iter()
        .flat_map(|def| def.data_assets.iter().map(String::as_str))
        .collect();
    data_assets.extend(
        draft
            .assets
            .iter()
            .filter(|a| a.format == AssetFormat::F32)
            .map(|a| a.asset_id.as_str()),
    );
    let mut seen = HashSet::new();
    for (index, asset) in draft.assets.iter().enumerate() {
        let path = at("/assets", index);
        if !seen.insert(asset.asset_id.as_str()) {
            issues.push(Issue::new(
                "DUPLICATE_ASSET_ID",
                child(&path, "assetId"),
                format!("Duplicate assets[].assetId {}", asset.asset_id),
            ));
        }
        if asset.color_space == AssetColorSpace::LinearSrgb
            && !data_assets.contains(asset.asset_id.as_str())
        {
            issues.push(Issue::new(
                "ASSET_LINEAR_SRGB_UNSUPPORTED",
                child(&path, "colorSpace"),
                format!("Asset {} linear-srgb is not implemented", asset.asset_id),
            ));
        }
    }
}

fn instances_and_tracks(draft: &Draft, definition_ids: &HashSet<&str>, issues: &mut Vec<Issue>) {
    let mut seen = HashSet::new();
    for (index, instance) in draft.instances.iter().enumerate() {
        let path = at("/sceneInstances", index);
        if !seen.insert(instance.id.as_str()) {
            issues.push(Issue::new(
                "DUPLICATE_SCENE_INSTANCE_ID",
                child(&path, "id"),
                format!("Duplicate sceneInstances id {}", instance.id),
            ));
        }
        if !definition_ids.contains(instance.scene_definition_id.as_str()) {
            issues.push(Issue::new(
                "UNKNOWN_SCENE_DEFINITION",
                child(&path, "sceneDefinitionId"),
                format!(
                    "Instance {} references unknown sceneDefinitionId {}",
                    instance.id, instance.scene_definition_id
                ),
            ));
        }
    }

    let track_by_id: HashMap<&str, (usize, _)> = draft
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), (i, t)))
        .collect();
    let def_by_id: HashMap<&str, _> = draft.defs.iter().map(|d| (d.id.as_str(), d)).collect();
    for (index, instance) in draft.instances.iter().enumerate() {
        let list_path = child(&at("/sceneInstances", index), "performanceTrackIds");
        let node_ids: HashSet<&str> = def_by_id
            .get(instance.scene_definition_id.as_str())
            .map(|def| def.nodes.iter().map(|n| n.id.as_str()).collect())
            .unwrap_or_default();
        for (t, track_id) in instance.performance_track_ids.iter().enumerate() {
            let path = at(&list_path, t);
            let Some(&(track_index, track)) = track_by_id.get(track_id.as_str()) else {
                issues.push(Issue::new(
                    "PERFORMANCE_TRACK_REF_MISSING",
                    path,
                    format!(
                        "Instance {} references unknown performanceTrack {track_id}",
                        instance.id
                    ),
                ));
                continue;
            };
            if track.instance_id != instance.id {
                issues.push(Issue::new(
                    "PERFORMANCE_TRACK_INSTANCE_MISMATCH",
                    path,
                    format!(
                        "Track {track_id} instanceId {} !== {}",
                        track.instance_id, instance.id
                    ),
                ));
            }
            for (c, channel) in track.channels.iter().enumerate() {
                if !node_ids.contains(channel.node_id.as_str()) {
                    issues.push(Issue::new(
                        "PERFORMANCE_NODE_MISSING",
                        child(
                            &at(
                                &child(&at("/performanceTracks", track_index), "channels"),
                                c,
                            ),
                            "nodeId",
                        ),
                        format!(
                            "Channel {} targets missing node {} on instance {}",
                            channel.channel_id, channel.node_id, instance.id
                        ),
                    ));
                }
            }
        }
    }

    for (index, track) in draft.tracks.iter().enumerate() {
        let mut channel_ids = HashSet::new();
        for (c, channel) in track.channels.iter().enumerate() {
            if !channel_ids.insert(channel.channel_id.as_str()) {
                issues.push(Issue::new(
                    "DUPLICATE_CHANNEL_ID",
                    at(&child(&at("/performanceTracks", index), "channels"), c),
                    format!("Duplicate performance channelId: {}", channel.channel_id),
                ));
            }
        }
    }
}

// ---- camera --------------------------------------------------------------------------------

fn fov_in_range(fov: f64) -> bool {
    fov > 0.0 && fov < 180.0
}

fn known_easing(easing: &str) -> bool {
    KNOWN_EASINGS.contains(&easing)
}

fn segment_easing(motion: &CameraMotion) -> Option<&str> {
    match motion {
        CameraMotion::Move { easing, .. }
        | CameraMotion::Zoom { easing, .. }
        | CameraMotion::Orbit { easing, .. } => Some(easing),
        CameraMotion::Hold | CameraMotion::Keyframes { .. } => None,
    }
}

/// The rules of a keyframes movement: keys strictly increasing inside it, the last one on its
/// last frame, each a camera that can look somewhere with a field of view in range.
fn camera_keys(
    clip_id: &str,
    where_: &str,
    path: &str,
    duration: u64,
    keys: &[super::model::CameraKey],
    issues: &mut Vec<Issue>,
) {
    let last_frame = i64::try_from(duration).unwrap_or(i64::MAX) - 1;
    let mut previous = 0_i64;
    for (k, key) in keys.iter().enumerate() {
        let label = format!("Clip {clip_id} {where_} key {k}");
        let key_path = at(&child(path, "keys"), k);
        if key.frame <= previous || key.frame > last_frame {
            issues.push(Issue::new(
                "CAMERA_KEY_FRAME_ORDER",
                child(&key_path, "frame"),
                format!(
                    "{label} is at frame {}; keys must be strictly increasing, from frame 1 to frame {last_frame} of the movement",
                    key.frame
                ),
            ));
        }
        previous = key.frame;
        if !fov_in_range(key.fov_deg) {
            issues.push(Issue::new(
                "FOV_OUT_OF_RANGE",
                child(&key_path, "fovDeg"),
                format!("{label} fovDeg must be in (0, 180)"),
            ));
        }
        if key.target.distance(key.position) < 1e-6 {
            issues.push(Issue::new(
                "CAMERA_DIRECTION_DEGENERATE",
                &key_path,
                format!("{label} target equals position"),
            ));
        }
        if !known_easing(&key.easing) {
            issues.push(Issue::new(
                "UNKNOWN_CAMERA_EASING",
                child(&key_path, "easing"),
                format!("{label} easing {} is not supported", key.easing),
            ));
        }
    }
    if let Some(last) = keys.last().filter(|last| last.frame != last_frame) {
        issues.push(Issue::new(
            "CAMERA_KEYS_END_SHORT",
            child(path, "keys"),
            format!(
                "Clip {clip_id} {where_} ends at frame {last_frame} but its last key is at frame {}; the last key must be on the movement's last frame",
                last.frame
            ),
        ));
    }
}

fn camera_clip(clip: &CameraClip, path: &str, issues: &mut Vec<Issue>) {
    let before = issues.len();
    if !fov_in_range(clip.initial_fov_deg) {
        issues.push(Issue::new(
            "FOV_OUT_OF_RANGE",
            child(path, "initialFovDeg"),
            format!("Clip {} initialFovDeg must be in (0, 180)", clip.id),
        ));
    }
    if clip.initial_target.distance(clip.initial_position) < 1e-6 {
        issues.push(Issue::new(
            "CAMERA_DIRECTION_DEGENERATE",
            path,
            format!("Clip {} target equals position", clip.id),
        ));
    }
    let segments_path = child(path, "segments");
    let mut carried = clip.initial_fov_deg;
    for (i, segment) in clip.segments.iter().enumerate() {
        let segment_path = at(&segments_path, i);
        let is_hold = matches!(segment.motion, CameraMotion::Hold);
        if !is_hold && segment.duration_frames < 2 {
            issues.push(Issue::new(
                "CAMERA_SEGMENT_TOO_SHORT",
                child(&segment_path, "durationFrames"),
                format!("Clip {} motion needs at least two frames", clip.id),
            ));
        }
        if let CameraMotion::Keyframes { keys } = &segment.motion {
            // A keyframes movement carries its own poses: its field of view is checked key by
            // key and the next movement carries on from its last key.
            camera_keys(
                &clip.id,
                &format!("segment {i}"),
                &segment_path,
                segment.duration_frames,
                keys,
                issues,
            );
            carried = keys.last().map_or(carried, |key| key.fov_deg);
            continue;
        }
        let mut next = if segment.from == SegmentStart::Initial {
            clip.initial_fov_deg
        } else {
            carried
        };
        if let CameraMotion::Zoom {
            recipe,
            fov_delta_deg,
            ..
        } = &segment.motion
        {
            match recipe {
                super::model::ZoomRecipe::In => next -= fov_delta_deg,
                super::model::ZoomRecipe::Out => next += fov_delta_deg,
            }
        }
        if !fov_in_range(next) {
            issues.push(Issue::new(
                "FOV_OUT_OF_RANGE",
                &segment_path,
                format!("Clip {} segment {i} produces invalid FOV", clip.id),
            ));
        }
        carried = next;
        if let Some(easing) = segment_easing(&segment.motion).filter(|e| !known_easing(e)) {
            issues.push(Issue::new(
                "UNKNOWN_CAMERA_EASING",
                child(&segment_path, "easing"),
                format!(
                    "Clip {} segment {i} easing {easing} is not supported",
                    clip.id
                ),
            ));
        }
    }
    // Layers are checked only on a main track that is sound, as in the origin: a camera on its
    // own target has no view to turn.
    if issues.len() == before {
        camera_layers(clip, path, issues);
    }
}

/// The structural rules of the tracks stacked on a clip's main one. The origin finishes with a
/// frame-by-frame check that the stacked zooms keep the composed field of view in range; that
/// needs the camera evaluated and is deferred to BB03 (`DEFERRED_RULES`).
fn camera_layers(clip: &CameraClip, path: &str, issues: &mut Vec<Issue>) {
    for (l, layer) in clip.layers.iter().enumerate() {
        let track = l + 1;
        let layer_path = at(&child(path, "layers"), l);
        for (i, segment) in layer.iter().enumerate() {
            layer_segment(
                clip,
                track,
                i,
                segment,
                &at(&child(&layer_path, "segments"), i),
                issues,
            );
        }
    }
}

fn layer_segment(
    clip: &CameraClip,
    track: usize,
    i: usize,
    segment: &CameraSegment,
    path: &str,
    issues: &mut Vec<Issue>,
) {
    if matches!(segment.motion, CameraMotion::Keyframes { .. }) {
        issues.push(Issue::new(
            "CAMERA_LAYER_KEYFRAMES",
            path,
            format!(
                "Clip {} track {track} segment {i} holds keyframes; a layer adds to the main track and has no poses of its own",
                clip.id
            ),
        ));
        return;
    }
    if segment.from == SegmentStart::Initial {
        issues.push(Issue::new(
            "CAMERA_LAYER_FROM_INITIAL",
            child(path, "from"),
            format!(
                "Clip {} track {track} segment {i} starts from \"initial\"; a layer has no start pose of its own",
                clip.id
            ),
        ));
    }
    if matches!(segment.motion, CameraMotion::Hold) {
        return;
    }
    if segment.duration_frames < 2 {
        issues.push(Issue::new(
            "CAMERA_SEGMENT_TOO_SHORT",
            child(path, "durationFrames"),
            format!(
                "Clip {} track {track} motion needs at least two frames",
                clip.id
            ),
        ));
    }
    if let Some(easing) = segment_easing(&segment.motion).filter(|e| !known_easing(e)) {
        issues.push(Issue::new(
            "UNKNOWN_CAMERA_EASING",
            child(path, "easing"),
            format!(
                "Clip {} track {track} segment {i} easing {easing} is not supported",
                clip.id
            ),
        ));
    }
}

// ---- shots and passes ----------------------------------------------------------------------

fn shots(draft: &Draft, issues: &mut Vec<Issue>) {
    let instance_by_id: HashMap<&str, _> =
        draft.instances.iter().map(|i| (i.id.as_str(), i)).collect();
    let clip_ids: HashSet<&str> = draft.clips.iter().map(|c| c.id.as_str()).collect();

    // Stable sort by start frame, remembering the document index for the pointers.
    let mut order: Vec<usize> = (0..draft.shots.len()).collect();
    order.sort_by_key(|&i| draft.shots[i].start_frame);
    if order
        .first()
        .is_some_and(|&first| draft.shots[first].start_frame != 0)
    {
        issues.push(Issue::new(
            "EDIT_RANGE_START",
            "/shots",
            "First shot must start at edit frame 0",
        ));
    }
    let mut expected_start = 0_u64;
    for &index in &order {
        let shot = &draft.shots[index];
        let path = at("/shots", index);
        let instance = instance_by_id.get(shot.scene_instance_id.as_str());
        if instance.is_none() {
            issues.push(Issue::new(
                "SHOT_UNKNOWN_INSTANCE",
                child(&path, "sceneInstanceId"),
                format!(
                    "Shot {} references unknown sceneInstanceId {}",
                    shot.id, shot.scene_instance_id
                ),
            ));
        }
        if !clip_ids.contains(shot.camera_clip_id.as_str()) {
            issues.push(Issue::new(
                "SHOT_UNKNOWN_CAMERA_CLIP",
                child(&path, "cameraClipId"),
                format!(
                    "Shot {} references unknown cameraClipId {}",
                    shot.id, shot.camera_clip_id
                ),
            ));
        }
        if let Some(instance) = instance
            && shot.instance_start_frame + shot.duration_frames() > instance.duration_frames
        {
            issues.push(Issue::new(
                "SHOT_EXCEEDS_INSTANCE_DURATION",
                child(&path, "instanceStartFrame"),
                format!(
                    "Shot {} instance mapping exceeds instance {} duration",
                    shot.id, instance.id
                ),
            ));
        }
        if shot.start_frame != expected_start {
            issues.push(Issue::new(
                "EDIT_GAP_OR_OVERLAP",
                child(&path, "editRange"),
                format!(
                    "Shot {} starts at {}, expected {expected_start}",
                    shot.id, shot.start_frame
                ),
            ));
        }
        if shot.end_frame_exclusive <= shot.start_frame {
            issues.push(Issue::new(
                "EDIT_RANGE_INVALID",
                child(&path, "editRange"),
                format!("Shot {} has non-positive duration", shot.id),
            ));
        }
        expected_start = shot.end_frame_exclusive;
        // A shot that outlasts its camera clip is fine: the clip holds its last pose.
    }
    if expected_start != draft.edit_duration_frames {
        issues.push(Issue::new(
            "EDIT_DURATION_MISMATCH",
            "/editDurationFrames",
            format!(
                "Shots cover {expected_start} frames but editDurationFrames is {}",
                draft.edit_duration_frames
            ),
        ));
    }
}

fn composition_passes(draft: &Draft, issues: &mut Vec<Issue>) {
    let node_ids: HashSet<&str> = draft
        .defs
        .iter()
        .flat_map(|def| def.nodes.iter().map(|n| n.id.as_str()))
        .collect();
    let mut keys = HashSet::new();
    for (index, pass) in draft.passes.iter().enumerate() {
        let path = at("/compositionPasses", index);
        let key = format!("{}:{}:prePost", pass.fragment_id, pass.node_id);
        if !keys.insert(key.clone()) {
            issues.push(Issue::new(
                "COMPOSITION_PASS_CONFLICT",
                &path,
                format!("Duplicate compositionPasses entry {key}"),
            ));
        }
        if !keys.insert(format!("node:{}", pass.node_id)) {
            issues.push(Issue::new(
                "COMPOSITION_ORDER_CONFLICT",
                &path,
                format!("Multiple composition passes target {}", pass.node_id),
            ));
        }
        if !node_ids.contains(pass.node_id.as_str()) {
            issues.push(Issue::new(
                "COMPOSITION_NODE_MISSING",
                child(&path, "nodeId"),
                format!("compositionPasses references unknown node {}", pass.node_id),
            ));
        }
    }
}

// Light rendering is deferred, but a declared attachment already belongs to graph integrity.
fn light_parent_references(draft: &Draft, document: &serde_json::Value, issues: &mut Vec<Issue>) {
    for (index, definition) in draft.defs.iter().enumerate() {
        let Some(lights) = document["sceneDefinitions"][index]["lighting"]["lights"].as_array()
        else {
            continue;
        };
        let nodes: HashSet<&str> = definition
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect();
        for (light_index, light) in lights.iter().enumerate() {
            let Some(parent) = light["parent"].as_str().filter(|id| !id.is_empty()) else {
                continue;
            };
            if !nodes.contains(parent) {
                issues.push(Issue::new(
                    "LIGHT_PARENT_UNKNOWN",
                    format!(
                        "{}/lighting/lights/{light_index}/parent",
                        definition.pointer
                    ),
                    format!(
                        "Light of {} follows {parent}, which is not a layer of the scene",
                        definition.id
                    ),
                ));
            }
        }
    }
}
