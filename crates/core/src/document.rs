//! The persistent project document (`*.tessel.json`), schemas v1 and v2.
//!
//! The document is a projection of the editor model, never a dump of its structs: IDs are
//! strings (the interner is not serialized), time and rates are rationals, paths of managed
//! media are relative to the document, and presentation (playhead, zoom, snap) is kept in a
//! separate optional `presentation` object that a loader may ignore. Serialization is
//! canonical (sorted keys, stable array order), so saving an unchanged project twice yields
//! identical bytes. See `docs/features/project-document.md` for the full schema.

use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::assets::{Asset, AudioStream, DemoImage, Identity, MediaKind, Meta, Source, Storage};
use crate::content::{Camera, TextSpec};
use crate::id::Id;
use crate::scene_project::SceneBook;
use crate::state::history::History;
use crate::state::inspector::Inspector;
use crate::state::library::{Folder, FolderIcon, Library, MediaItem};
use crate::state::timeline::{
    Clip, ClipKind, Timeline, Track, TrackKind, ZOOM_DEFAULT, ZOOM_MAX, ZOOM_MIN,
};
use crate::state::{CommandError, EditorState, ProjectMeta, Subject, WorkspaceTab};
use crate::time::{Rate, Ratio};

pub const FORMAT: &str = "tessel-project";
/// Latest document version this reader supports.
pub const SCHEMA_VERSION: u64 = 2;
/// Projects without directed-scene data keep the legacy encoding.
pub const LEGACY_SCHEMA_VERSION: u64 = 1;
/// Renderer versions a document was written against, checked by the export preflight.
pub const SPARK_VERSION: &str = "2.4.0";
pub const THREE_VERSION: &str = "0.187.0";

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new("invalid_document", message)
}

/// Document path of `path`: relative to `base` (with `/` separators) when it lies inside it,
/// otherwise unchanged. WHY only inside: a relative path that climbs out of the workspace
/// would make the project depend on where the folder sits, which is what portability avoids.
pub fn encode_path(base: Option<&Path>, path: &str) -> String {
    let Some(base) = base else {
        return path.to_string();
    };
    match Path::new(path).strip_prefix(base) {
        Ok(relative) if !relative.as_os_str().is_empty() => relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
        _ => path.to_string(),
    }
}

/// Absolute path for a stored one. Relative paths resolve against `base` (the document's
/// folder, never the process cwd) and may not contain `..` or a root.
pub fn decode_path(base: Option<&Path>, stored: &str) -> Result<String, CommandError> {
    let path = Path::new(stored);
    if path.is_absolute() {
        return Ok(stored.to_string());
    }
    let escapes = path
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir));
    if escapes || stored.is_empty() {
        return Err(invalid(format!(
            "The relative path `{stored}` must stay inside the project folder"
        )));
    }
    let base = base.ok_or_else(|| {
        invalid(format!(
            "The path `{stored}` is relative to the document, but no document folder is known"
        ))
    })?;
    Ok(base.join(path).to_string_lossy().into_owned())
}

fn put_opt(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        map.insert(key.to_string(), value);
    }
}

fn ratio_json(value: Ratio) -> Value {
    json!({ "num": value.num(), "den": value.den() })
}

fn rate_json(value: Rate) -> Value {
    json!({ "num": value.num(), "den": value.den() })
}

fn meta_json(meta: &Meta) -> Value {
    let mut map = Map::new();
    put_opt(&mut map, "width", meta.width.map(Value::from));
    put_opt(&mut map, "height", meta.height.map(Value::from));
    put_opt(&mut map, "rate", meta.rate.map(rate_json));
    put_opt(&mut map, "duration", meta.duration.map(ratio_json));
    put_opt(&mut map, "sample_rate", meta.sample_rate.map(Value::from));
    put_opt(&mut map, "channels", meta.channels.map(Value::from));
    put_opt(&mut map, "splat_count", meta.splat_count.map(Value::from));
    put_opt(&mut map, "format", meta.format.clone().map(Value::from));
    put_opt(&mut map, "text", meta.text.as_ref().map(TextSpec::to_json));
    if !meta.audio_streams.is_empty() {
        map.insert(
            "audio_streams".into(),
            json!(
                meta.audio_streams
                    .iter()
                    .map(|stream| json!({
                        "index": stream.index, "sample_rate": stream.sample_rate,
                        "channels": stream.channels, "offset": ratio_json(stream.offset),
                    }))
                    .collect::<Vec<_>>()
            ),
        );
    }
    if meta.vfr_conformed {
        map.insert("vfr_conformed".into(), json!(true));
    }
    if !meta.requires.is_empty() {
        map.insert(
            "requires".into(),
            json!(
                meta.requires
                    .iter()
                    .map(|id| id.as_str())
                    .collect::<Vec<_>>()
            ),
        );
    }
    Value::Object(map)
}

fn asset_json(asset: &Asset, base: Option<&Path>) -> Value {
    let source = match &asset.source {
        Source::Demo(image) => json!({ "type": "demo", "name": image.file_name() }),
        Source::File { path } => json!({ "type": "file", "path": encode_path(base, path) }),
        Source::FrameSequence {
            pattern,
            first,
            count,
        } => json!({ "type": "frame_sequence", "pattern": encode_path(base, pattern),
            "first": first, "count": count }),
        Source::Inline => json!({ "type": "inline" }),
    };
    let identity = &asset.identity;
    let mut id = Map::new();
    put_opt(&mut id, "hash", identity.hash.clone().map(Value::from));
    put_opt(&mut id, "size", identity.size.map(Value::from));
    id.insert("storage".into(), json!(identity.storage.label()));
    put_opt(&mut id, "origin", identity.origin.clone().map(Value::from));
    put_opt(
        &mut id,
        "file_name",
        identity.file_name.clone().map(Value::from),
    );
    put_opt(&mut id, "probe", identity.probe.clone());
    json!({ "source": source, "meta": meta_json(&asset.meta), "identity": Value::Object(id) })
}

fn clip_json(clip: &crate::state::timeline::Clip) -> Value {
    let mut map = Map::new();
    map.insert("id".into(), json!(clip.id));
    map.insert("name".into(), json!(clip.name));
    map.insert("kind".into(), json!(clip.kind.label()));
    map.insert("track".into(), json!(clip.track));
    map.insert("start".into(), json!(clip.start));
    map.insert("duration".into(), json!(clip.duration));
    put_opt(&mut map, "media", clip.media.map(|id| json!(id)));
    map.insert("source_start".into(), ratio_json(clip.source_start));
    put_opt(&mut map, "text", clip.text.as_ref().map(TextSpec::to_json));
    put_opt(
        &mut map,
        "camera",
        clip.camera.as_ref().map(Camera::to_json),
    );
    Value::Object(map)
}

fn sequence_json(timeline: &Timeline) -> Value {
    json!({
        "id": timeline.id, "name": timeline.name,
        "rate": rate_json(timeline.rate), "length": timeline.length,
        "width": timeline.width, "height": timeline.height,
        "tracks": timeline.tracks.iter().map(|track| json!({
            "id": track.id, "name": track.name, "kind": track.kind.label(),
            "hidden": track.hidden, "muted": track.muted, "height": track.height,
        })).collect::<Vec<_>>(),
        "clips": timeline.clips.iter().map(clip_json).collect::<Vec<_>>(),
    })
}

impl EditorState {
    /// Smallest schema that can preserve this project's data when it is saved.
    pub fn document_schema_version(&self) -> u64 {
        if self.scenes.has_document_data() {
            SCHEMA_VERSION
        } else {
            LEGACY_SCHEMA_VERSION
        }
    }

    /// The document of the project. `base` is the folder that will hold it: managed paths
    /// inside it are stored relative, everything else stays absolute (reference media).
    pub fn to_document(&self, base: Option<&Path>) -> Value {
        let folders: Vec<Value> = self
            .library
            .folders
            .iter()
            .map(|folder| {
                json!({ "id": folder.id, "label": folder.label, "icon": folder.icon.label(),
                    "parent": folder.parent, "expanded": folder.expanded })
            })
            .collect();
        let assets: Vec<Value> = self
            .library
            .items
            .iter()
            .map(|item| {
                let mut map = Map::new();
                map.insert("id".into(), json!(item.id));
                map.insert("code".into(), json!(item.code));
                map.insert("name".into(), json!(item.name));
                map.insert("kind".into(), json!(item.kind.label()));
                map.insert("folder".into(), json!(item.folder));
                map.insert("summary".into(), json!(item.meta));
                put_opt(
                    &mut map,
                    "demo_image",
                    item.image.map(|image| json!(image.file_name())),
                );
                put_opt(
                    &mut map,
                    "asset",
                    item.asset.as_ref().map(|asset| asset_json(asset, base)),
                );
                Value::Object(map)
            })
            .collect();
        let overrides: Map<String, Value> = {
            let mut grouped: std::collections::BTreeMap<String, Map<String, Value>> =
                std::collections::BTreeMap::new();
            for (clip, property, value) in self.inspector.overrides_sorted() {
                grouped
                    .entry(clip)
                    .or_default()
                    .insert(property.to_string(), value.to_json());
            }
            grouped
                .into_iter()
                .map(|(clip, values)| (clip, Value::Object(values)))
                .collect()
        };
        let presentation: Map<String, Value> = self
            .all_sequences()
            .map(|timeline| {
                (
                    timeline.id.to_string(),
                    json!({ "playhead": timeline.playhead, "zoom": timeline.zoom, "snap": timeline.snap }),
                )
            })
            .collect();
        let mut document = json!({
            "format": FORMAT,
            "schema_version": self.document_schema_version(),
            "project": {
                "id": self.project.id,
                "name": self.project_name,
                "demonstration": self.demonstration,
                "requires": { "spark": SPARK_VERSION, "three": THREE_VERSION },
            },
            "library": { "folders": folders, "assets": assets },
            "sequences": self.all_sequences().map(sequence_json).collect::<Vec<_>>(),
            "active_sequence": self.timeline.id,
            "clip_properties": Value::Object(overrides),
            "export": {
                "preset": self.project.export_preset,
                "user_presets": self.project.user_presets,
            },
            "presentation": Value::Object(presentation),
        });
        // Additive block (see `scene_project::store`): absent unless the project has scenes, so
        // a project without any is byte-identical to what earlier editors wrote.
        if let Some(scenes) = self.scenes.to_json() {
            document["scenes"] = scenes;
        }
        document
    }

    /// Builds a candidate project from a document. Nothing is returned half built: the caller
    /// swaps the candidate in only on `Ok`, so a bad document never touches the open project.
    pub fn from_document(doc: &Value, base: Option<&Path>) -> Result<Self, CommandError> {
        let doc = migrate(doc.clone())?;
        let project = &doc["project"];
        let name = project["name"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| invalid("`project.name` is required"))?;
        let mut library = parse_library(&doc["library"], base)?;
        library.selected_folder = None;

        let sequences = doc["sequences"]
            .as_array()
            .filter(|items| !items.is_empty())
            .ok_or_else(|| invalid("A project needs at least one sequence"))?;
        let mut timelines = Vec::new();
        for sequence in sequences {
            let timeline = parse_sequence(sequence, &library)?;
            if timelines
                .iter()
                .any(|other: &Timeline| other.id == timeline.id)
            {
                return Err(invalid(format!("Duplicate sequence `{}`", timeline.id)));
            }
            timelines.push(timeline);
        }
        // WHY global: clip properties are keyed by clip ID.
        let mut seen = std::collections::HashSet::new();
        for clip in timelines.iter().flat_map(|timeline| timeline.clips.iter()) {
            if !seen.insert(clip.id) {
                return Err(invalid(format!(
                    "Clip ID `{}` is used by more than one sequence",
                    clip.id
                )));
            }
        }
        let active = doc["active_sequence"]
            .as_str()
            .unwrap_or_else(|| timelines[0].id.as_str());
        let active_index = timelines
            .iter()
            .position(|timeline| timeline.id == active)
            .ok_or_else(|| invalid(format!("`active_sequence` `{active}` does not exist")))?;
        let mut active_timeline = timelines.remove(active_index);

        let mut inspector = Inspector::default();
        if let Some(map) = doc["clip_properties"].as_object() {
            for (clip, values) in map {
                if !active_timeline.clip(clip).is_some()
                    && !timelines
                        .iter()
                        .any(|timeline| timeline.clip(clip).is_some())
                {
                    return Err(invalid(format!("Properties of unknown clip `{clip}`")));
                }
                let values = values
                    .as_object()
                    .ok_or_else(|| invalid(format!("Properties of `{clip}` must be an object")))?;
                for (property, value) in values {
                    inspector.restore_override(clip, property, value)?;
                }
            }
        }
        if let Some(map) = doc["presentation"].as_object() {
            for timeline in std::iter::once(&mut active_timeline).chain(timelines.iter_mut()) {
                let Some(view) = map.get(timeline.id.as_str()) else {
                    continue;
                };
                timeline.playhead = view["playhead"]
                    .as_i64()
                    .unwrap_or(0)
                    .clamp(0, timeline.length);
                timeline.zoom = view["zoom"]
                    .as_f64()
                    .filter(|zoom| zoom.is_finite())
                    .unwrap_or(ZOOM_DEFAULT)
                    .clamp(ZOOM_MIN, ZOOM_MAX);
                timeline.snap = view["snap"].as_bool().unwrap_or(true);
            }
        }
        let mut meta = ProjectMeta::new();
        if let Some(id) = project["id"].as_str().filter(|id| !id.is_empty()) {
            meta.id = id.to_string();
        }
        meta.export_preset = doc["export"]["preset"].as_str().map(str::to_string);
        // Presets are validated on load, like everything else: a hand-edited document cannot
        // carry an impossible output.
        meta.user_presets = doc["export"]["user_presets"]
            .as_array()
            .map(|list| {
                list.iter()
                    .map(|raw| {
                        crate::preset::Preset::from_json(raw)
                            .map(|preset| preset.to_json())
                            .map_err(|error| invalid(format!("Export preset: {}", error.message)))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();

        // Validated like everything else here: a bad stored scene fails the whole open, so the
        // project in memory never holds a scene the editor would have refused to attach.
        let scenes = doc
            .get("scenes")
            .map(SceneBook::from_json)
            .transpose()?
            .unwrap_or_default();

        let selected_media = library.items.first().map(|item| item.id);
        Ok(Self {
            project_name: name.trim().to_string(),
            demonstration: project["demonstration"].as_bool().unwrap_or(false),
            library,
            inspector,
            timeline: active_timeline,
            sequences: timelines,
            project: meta,
            history: History::default(),
            workspace_tab: WorkspaceTab::Edit,
            selected_media,
            subject: Subject::Media,
            preview: crate::plan::PreviewIntent::Sequence,
            preview_policy: crate::preview::PreviewPolicy::default(),
            scenes,
            scene_interaction: Default::default(),
            thumbnails: Default::default(),
        })
    }
}

fn str_of<'a>(value: &'a Value, key: &str, what: &str) -> Result<&'a str, CommandError> {
    value[key]
        .as_str()
        .ok_or_else(|| invalid(format!("{what} needs a string `{key}`")))
}

fn id_of(value: &Value, key: &str, what: &str) -> Result<Id, CommandError> {
    let raw = str_of(value, key, what)?;
    Id::new(raw).map_err(|error| invalid(format!("{what} `{key}` `{raw}`: {error}")))
}

fn ratio_of(value: &Value, what: &str) -> Result<Ratio, CommandError> {
    Ratio::from_json(value).ok_or_else(|| invalid(format!("{what} must be {{num, den}}")))
}

fn parse_library(value: &Value, base: Option<&Path>) -> Result<Library, CommandError> {
    let folders = value["folders"]
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or_else(|| invalid("The library needs at least one folder"))?;
    let mut library = Library {
        folders: Vec::new(),
        items: Vec::new(),
        selected_folder: None,
        search: String::new(),
    };
    for folder in folders {
        let id = id_of(folder, "id", "A folder")?;
        if library.folder(id).is_some() {
            return Err(invalid(format!("Duplicate folder `{id}`")));
        }
        let icon = [
            FolderIcon::Project,
            FolderIcon::Footage,
            FolderIcon::Timelines,
            FolderIcon::Audio,
        ]
        .into_iter()
        .find(|icon| Some(icon.label()) == folder["icon"].as_str())
        .unwrap_or(FolderIcon::Project);
        let parent = match folder["parent"].as_str() {
            Some(raw) => Some(Id::new(raw).map_err(|error| invalid(error.to_string()))?),
            None => None,
        };
        library.folders.push(Folder {
            id,
            label: str_of(folder, "label", "A folder")?.to_string(),
            icon,
            parent,
            expanded: folder["expanded"].as_bool().unwrap_or(true),
        });
    }
    let roots = library
        .folders
        .iter()
        .filter(|f| f.parent.is_none())
        .count();
    if roots != 1 {
        return Err(invalid(format!(
            "The library needs exactly one root folder, found {roots}"
        )));
    }
    for folder in &library.folders {
        if let Some(parent) = folder.parent
            && library.folder(parent).is_none()
        {
            return Err(invalid(format!(
                "Folder `{}` has an unknown parent `{parent}`",
                folder.id
            )));
        }
    }
    for item in value["assets"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let id = id_of(item, "id", "An asset")?;
        let kind = MediaKind::parse(str_of(item, "kind", "An asset")?)
            .ok_or_else(|| invalid(format!("Asset `{id}` has an unknown kind")))?;
        let folder = id_of(item, "folder", "An asset")?;
        let asset = match item.get("asset").filter(|asset| !asset.is_null()) {
            Some(asset) => Some(parse_asset(asset, id, base)?),
            None => None,
        };
        let image = item["demo_image"].as_str().and_then(demo_image);
        library
            .add_item(MediaItem {
                id,
                code: str_of(item, "code", "An asset")?.to_string(),
                name: str_of(item, "name", "An asset")?.to_string(),
                meta: item["summary"].as_str().unwrap_or_default().to_string(),
                kind,
                folder,
                image,
                asset,
            })
            .map_err(|error| invalid(error.message))?;
    }
    // Scenes must find their splats: the same rule `AddAsset` enforces at registration.
    for item in &library.items {
        let Some(asset) = &item.asset else { continue };
        for required in &asset.meta.requires {
            let ok = library
                .item(required)
                .is_some_and(|dep| dep.kind == MediaKind::Splat);
            if !ok {
                return Err(invalid(format!(
                    "Scene `{}` needs splat `{required}`, which the document does not contain",
                    item.id
                )));
            }
        }
    }
    Ok(library)
}

fn demo_image(name: &str) -> Option<DemoImage> {
    [DemoImage::Highlands, DemoImage::Coast]
        .into_iter()
        .find(|image| image.file_name() == name)
}

fn parse_audio_stream(value: &Value, asset: Id) -> Result<AudioStream, CommandError> {
    let number = |key: &str| value[key].as_u64().and_then(|n| u32::try_from(n).ok());
    let (Some(index), Some(sample_rate), Some(channels)) =
        (number("index"), number("sample_rate"), number("channels"))
    else {
        return Err(invalid(format!(
            "Asset `{asset}` has an audio stream without index, sample_rate or channels"
        )));
    };
    Ok(AudioStream {
        index,
        sample_rate,
        channels: u16::try_from(channels)
            .map_err(|_| invalid(format!("Asset `{asset}` has an invalid channel count")))?,
        offset: ratio_of(&value["offset"], &format!("Asset `{asset}` audio offset"))?,
    })
}

fn parse_asset(value: &Value, id: Id, base: Option<&Path>) -> Result<Asset, CommandError> {
    let source = &value["source"];
    let path = |key: &str| -> Result<String, CommandError> {
        decode_path(base, str_of(source, key, &format!("Asset `{id}` source"))?)
    };
    let source = match source["type"].as_str() {
        Some("demo") => Source::Demo(
            source["name"]
                .as_str()
                .and_then(demo_image)
                .ok_or_else(|| invalid(format!("Asset `{id}` has an unknown demo image")))?,
        ),
        Some("file") => Source::File {
            path: path("path")?,
        },
        Some("frame_sequence") => Source::FrameSequence {
            pattern: path("pattern")?,
            first: source["first"].as_i64().unwrap_or(0),
            count: source["count"]
                .as_i64()
                .filter(|count| *count > 0)
                .ok_or_else(|| invalid(format!("Asset `{id}` frame sequence needs `count`")))?,
        },
        Some("inline") => Source::Inline,
        _ => return Err(invalid(format!("Asset `{id}` has an unknown source type"))),
    };
    let m = &value["meta"];
    let uint = |key: &str| m[key].as_u64().and_then(|n| u32::try_from(n).ok());
    let meta = Meta {
        width: uint("width"),
        height: uint("height"),
        rate: match m.get("rate") {
            Some(rate) => Some(
                Rate::from_json(rate)
                    .ok_or_else(|| invalid(format!("Asset `{id}` has an invalid rate")))?,
            ),
            None => None,
        },
        duration: match m.get("duration") {
            Some(duration) => Some(ratio_of(duration, &format!("Asset `{id}` duration"))?),
            None => None,
        },
        sample_rate: uint("sample_rate"),
        channels: m["channels"].as_u64().and_then(|n| u16::try_from(n).ok()),
        splat_count: m["splat_count"].as_u64(),
        requires: m["requires"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .ok_or_else(|| invalid("`requires` holds asset IDs"))
                            .and_then(|raw| {
                                Id::new(raw).map_err(|error| invalid(error.to_string()))
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default(),
        text: match m.get("text") {
            Some(text) => Some(TextSpec::from_json(text).map_err(|error| invalid(error.message))?),
            None => None,
        },
        format: m["format"].as_str().map(str::to_string),
        audio_streams: match m.get("audio_streams").and_then(Value::as_array) {
            Some(streams) => streams
                .iter()
                .map(|stream| parse_audio_stream(stream, id))
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
        },
        vfr_conformed: m["vfr_conformed"].as_bool().unwrap_or(false),
    };
    let i = &value["identity"];
    let identity = Identity {
        hash: i["hash"].as_str().map(str::to_string),
        size: i["size"].as_u64(),
        storage: i["storage"]
            .as_str()
            .and_then(Storage::parse)
            .unwrap_or_default(),
        origin: i["origin"].as_str().map(str::to_string),
        file_name: i["file_name"].as_str().map(str::to_string),
        probe: i.get("probe").cloned(),
    };
    Ok(Asset {
        source,
        meta,
        identity,
        decoded: None,
    })
}

fn parse_sequence(value: &Value, library: &Library) -> Result<Timeline, CommandError> {
    let id = id_of(value, "id", "A sequence")?;
    let rate = value
        .get("rate")
        .and_then(Rate::from_json)
        .ok_or_else(|| invalid(format!("Sequence `{id}` needs a valid `rate`")))?;
    let settings = crate::state::timeline::SequenceSettings {
        id,
        rate,
        length: value["length"]
            .as_i64()
            .ok_or_else(|| invalid(format!("Sequence `{id}` needs `length`")))?,
        width: value["width"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| invalid(format!("Sequence `{id}` needs `width`")))?,
        height: value["height"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| invalid(format!("Sequence `{id}` needs `height`")))?,
    };
    settings
        .validate()
        .map_err(|error| invalid(format!("Sequence `{id}`: {}", error.message)))?;
    let mut timeline = Timeline::empty(&settings);
    if let Some(name) = value["name"]
        .as_str()
        .filter(|name| !name.trim().is_empty())
    {
        timeline.name = name.to_string();
    }
    for track in value["tracks"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let track_id = id_of(track, "id", "A track")?;
        if timeline.track(track_id).is_some() {
            return Err(invalid(format!(
                "Duplicate track `{track_id}` in sequence `{id}`"
            )));
        }
        let kind = match track["kind"].as_str() {
            Some("video") => TrackKind::Video,
            Some("audio") => TrackKind::Audio,
            _ => return Err(invalid(format!("Track `{track_id}` has an unknown kind"))),
        };
        timeline.tracks.push(Track {
            id: track_id,
            name: str_of(track, "name", "A track")?.to_string(),
            kind,
            hidden: track["hidden"].as_bool().unwrap_or(false),
            muted: track["muted"].as_bool().unwrap_or(false),
            height: track["height"].as_f64().unwrap_or(48.0) as f32,
        });
    }
    for clip in value["clips"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let clip_id = id_of(clip, "id", "A clip")?;
        let kind = [
            ClipKind::Video,
            ClipKind::Title,
            ClipKind::Audio,
            ClipKind::Scene,
        ]
        .into_iter()
        .find(|kind| Some(kind.label()) == clip["kind"].as_str())
        .ok_or_else(|| invalid(format!("Clip `{clip_id}` has an unknown kind")))?;
        let media = match clip["media"].as_str() {
            Some(raw) => {
                let media = Id::new(raw).map_err(|error| invalid(error.to_string()))?;
                if library.item(media).is_none() {
                    return Err(invalid(format!(
                        "Clip `{clip_id}` points at asset `{media}`, which the document does not contain"
                    )));
                }
                Some(media)
            }
            None => None,
        };
        let parsed = Clip {
            id: clip_id,
            name: str_of(clip, "name", "A clip")?.to_string(),
            kind,
            track: id_of(clip, "track", "A clip")?,
            start: clip["start"]
                .as_i64()
                .ok_or_else(|| invalid("A clip needs `start`"))?,
            duration: clip["duration"]
                .as_i64()
                .ok_or_else(|| invalid("A clip needs `duration`"))?,
            media,
            source_start: match clip.get("source_start") {
                Some(start) => ratio_of(start, "`source_start`")?,
                None => Ratio::ZERO,
            },
            text: match clip.get("text") {
                Some(text) => Some(TextSpec::from_json(text).map_err(|e| invalid(e.message))?),
                None => None,
            },
            camera: match clip.get("camera") {
                Some(camera) => Some(Camera::from_json(camera).map_err(|e| invalid(e.message))?),
                None => None,
            },
        };
        // Reuses the editing rules (track kind, bounds, overlap): a document cannot smuggle in
        // a layout the editor itself would refuse.
        timeline
            .add_clip(parsed)
            .map_err(|error| invalid(format!("Clip `{clip_id}`: {}", error.message)))?;
    }
    timeline.selected_clip = None;
    timeline.selected_track = None;
    Ok(timeline)
}

/// Validates the supported document versions. V1 has no scene storage; v2 protects it from
/// legacy readers, which reject versions above 1 instead of silently dropping the block.
/// The input is never rewritten here; ordinary projects may still be saved as v1.
pub fn migrate(doc: Value) -> Result<Value, CommandError> {
    if doc["format"] != FORMAT {
        return Err(CommandError::new(
            "invalid_document",
            format!("Not a Tessel project (expected \"format\": \"{FORMAT}\")"),
        ));
    }
    let version = doc["schema_version"]
        .as_u64()
        .ok_or_else(|| invalid("`schema_version` is missing"))?;
    if version > SCHEMA_VERSION {
        return Err(CommandError::new(
            "unsupported_schema",
            format!(
                "This project uses schema {version}; this editor reads up to {SCHEMA_VERSION}. \
                 Open it with a newer Tessel Editor."
            ),
        )
        .details(json!({ "found": version, "supported": SCHEMA_VERSION })));
    }
    if version == LEGACY_SCHEMA_VERSION && doc.get("scenes").is_some() {
        return Err(invalid(
            "Directed scenes require project schema 2; schema 1 readers would discard them",
        ));
    }
    if version == 0 {
        return Err(CommandError::new(
            "unsupported_schema",
            "Schema 0 never existed; the document is damaged or hand-written",
        ));
    }
    Ok(doc)
}

/// Serializes canonical JSON without allocating or returning bytes beyond the loader's budget.
/// The limit includes the trailing newline; failure happens before any file is written.
pub fn checked_canonical_bytes(doc: &Value, max_bytes: u64) -> Result<Vec<u8>, CommandError> {
    struct Bounded {
        bytes: Vec<u8>,
        limit: usize,
        exceeded: bool,
    }
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
                self.exceeded = true;
                return Err(std::io::Error::other(
                    "document serialization limit exceeded",
                ));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut out = Bounded {
        bytes: Vec::new(),
        limit: usize::try_from(max_bytes).unwrap_or(usize::MAX),
        exceeded: false,
    };
    let encoded = serde_json::to_writer_pretty(&mut out, doc)
        .map_err(|error| error.to_string())
        .and_then(|()| {
            std::io::Write::write_all(&mut out, b"\n").map_err(|error| error.to_string())
        });
    if let Err(message) = encoded {
        if out.exceeded {
            return Err(CommandError::new(
                "document_too_large",
                format!("The serialized document exceeds {max_bytes} bytes"),
            ));
        }
        return Err(invalid(format!(
            "The document could not be serialized: {message}"
        )));
    }
    Ok(out.bytes)
}

/// Canonical bytes of a document: sorted keys, two-space indent, trailing newline.
pub fn canonical_bytes(doc: &Value) -> Vec<u8> {
    // `serde_json::Map` is a BTreeMap here (no `preserve_order` feature), so keys are sorted.
    let mut bytes = serde_json::to_vec_pretty(doc).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

/// Absolute paths of every file an asset reads, resolved (frame sequences list first/last).
pub fn asset_files(asset: &Asset) -> Vec<PathBuf> {
    match &asset.source {
        Source::File { path } => vec![PathBuf::from(path)],
        Source::FrameSequence {
            pattern,
            first,
            count,
        } => (*first..*first + *count)
            .filter_map(|index| crate::assets::expand_pattern(pattern, index))
            .map(PathBuf::from)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod serialization_tests {
    use super::*;

    #[test]
    fn canonical_serialization_checks_the_actual_pretty_printed_size() {
        let doc = json!({"unknown": [[[[[1,2,3,4,5,6,7,8]]]]]});
        let expected = canonical_bytes(&doc);
        let limit = u64::try_from(expected.len()).unwrap();
        assert_eq!(checked_canonical_bytes(&doc, limit).unwrap(), expected);
        assert_eq!(
            checked_canonical_bytes(&doc, limit - 1).unwrap_err().code,
            "document_too_large"
        );
        let compact_size = u64::try_from(doc.to_string().len()).unwrap();
        assert!(compact_size < limit);
        assert!(checked_canonical_bytes(&doc, compact_size).is_err());
    }
}
