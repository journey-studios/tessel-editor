//! Importing sequences from another project (P08): a merge, never an open.
//!
//! The destination keeps its own entities. Everything that comes from the source gets an ID that
//! is free in the destination (the source ID when it is, a numbered variant when it is not), and
//! every reference is rewritten with the same map: clip → asset, scene → splat, clip overrides.
//! The source project is only read. The whole merge is one transaction, so a failure leaves the
//! destination exactly as it was and success is one undo step.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::assets::{Asset, MediaKind};
use crate::id::Id;
use crate::state::library::MediaItem;
use crate::state::timeline::{Clip, Timeline};
use crate::state::{CommandError, EditorState};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportPlan {
    /// `(source ID, destination ID)` pairs.
    pub sequences: Vec<(String, String)>,
    pub assets: Vec<(String, String)>,
    pub clips: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

impl ImportPlan {
    pub fn map_asset(&self, old: &str) -> Option<&str> {
        self.assets
            .iter()
            .find(|(from, _)| from == old)
            .map(|(_, to)| to.as_str())
    }

    pub fn to_json(&self) -> Value {
        let pairs = |list: &[(String, String)]| {
            list.iter()
                .map(|(from, to)| json!({ "from": from, "to": to, "renamed": from != to }))
                .collect::<Vec<_>>()
        };
        json!({ "sequences": pairs(&self.sequences), "assets": pairs(&self.assets),
            "clips": pairs(&self.clips), "warnings": self.warnings })
    }
}

fn free_id(wanted: &str, taken: impl Fn(&str) -> bool, reserved: &HashSet<String>) -> String {
    let free = |candidate: &str| !taken(candidate) && !reserved.contains(candidate);
    if free(wanted) {
        return wanted.to_string();
    }
    (2..)
        .map(|n| format!("{wanted}-{n}"))
        // Ids stay within the 64-byte limit: a long ID loses its tail before the suffix.
        .map(|candidate| {
            if candidate.len() <= crate::id::MAX_ID_LEN {
                candidate
            } else {
                let suffix = candidate.rsplit('-').next().unwrap_or("2").to_string();
                let head: String = wanted
                    .chars()
                    .take(crate::id::MAX_ID_LEN - suffix.len() - 1)
                    .collect();
                format!("{head}-{suffix}")
            }
        })
        .find(|candidate| free(candidate))
        .unwrap_or_else(|| wanted.to_string())
}

/// Asset IDs `sequences` of `source` depend on, transitively through scenes → splats.
fn closure(source: &EditorState, sequences: &[&Timeline]) -> Result<Vec<String>, CommandError> {
    let mut needed: Vec<String> = Vec::new();
    for timeline in sequences {
        for clip in &timeline.clips {
            if let Some(media) = clip.media {
                let id = media.as_str().to_string();
                if !needed.contains(&id) {
                    needed.push(id);
                }
            }
        }
    }
    let mut index = 0;
    while index < needed.len() {
        let item = source.library.item(&needed[index]).ok_or_else(|| {
            CommandError::new(
                "missing_dependency",
                format!(
                    "The source references `{}`, which its library lacks",
                    needed[index]
                ),
            )
        })?;
        if let Some(asset) = &item.asset {
            for required in &asset.meta.requires {
                if !needed.iter().any(|id| id == required.as_str()) {
                    needed.push(required.as_str().to_string());
                }
            }
        }
        index += 1;
    }
    // Dependencies first (splats before the scenes that name them): `AddAsset` checks this.
    needed.sort_by_key(|id| {
        source
            .library
            .item(id)
            .map_or(0, |item| usize::from(item.kind == MediaKind::Scene))
    });
    Ok(needed)
}

pub fn plan_import(
    destination: &EditorState,
    source: &EditorState,
    sequences: &[String],
) -> Result<ImportPlan, CommandError> {
    let chosen: Vec<&Timeline> = if sequences.is_empty() {
        source.all_sequences().collect()
    } else {
        sequences
            .iter()
            .map(|id| {
                source
                    .sequence(id)
                    .ok_or_else(|| CommandError::unknown_element("sequence", id))
            })
            .collect::<Result<_, _>>()?
    };
    let mut plan = ImportPlan::default();
    let (mut seq_taken, mut asset_taken, mut clip_taken) =
        (HashSet::new(), HashSet::new(), HashSet::new());
    for timeline in &chosen {
        let new = free_id(
            timeline.id.as_str(),
            |id| destination.sequence(id).is_some(),
            &seq_taken,
        );
        seq_taken.insert(new.clone());
        plan.sequences.push((timeline.id.as_str().to_string(), new));
    }
    for id in closure(source, &chosen)? {
        let item = source.library.item(&id);
        if item.is_some_and(|item| item.asset.is_none()) {
            return Err(CommandError::new(
                "unportable_media",
                format!("`{id}` has no media behind it in the source project"),
            )
            .target(id));
        }
        let new = free_id(
            &id,
            |candidate| {
                destination.library.item(candidate).is_some()
                    || destination.library.folder(candidate).is_some()
            },
            &asset_taken,
        );
        asset_taken.insert(new.clone());
        plan.assets.push((id, new));
    }
    for timeline in &chosen {
        for clip in &timeline.clips {
            let new = free_id(
                clip.id.as_str(),
                |candidate| {
                    destination
                        .all_sequences()
                        .any(|t| t.clip(candidate).is_some())
                },
                &clip_taken,
            );
            clip_taken.insert(new.clone());
            plan.clips.push((clip.id.as_str().to_string(), new));
        }
    }
    for (from, to) in &plan.sequences {
        if from != to {
            plan.warnings.push(format!(
                "sequence `{from}` is imported as `{to}`: the ID was taken"
            ));
        }
    }
    Ok(plan)
}

fn lookup<'a>(pairs: &'a [(String, String)], old: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(from, _)| from == old)
        .map(|(_, to)| to.as_str())
}

fn invalid_id(error: crate::id::IdError, id: &str) -> CommandError {
    CommandError::new("invalid_id", error.to_string()).target(id)
}

/// Applies `plan` as ONE undoable edit. `replacements` holds, by *source* asset ID, the asset
/// to register instead of the source's own (the media adapter copies bytes into the destination
/// workspace and returns the repointed asset); assets without an entry are cloned as they are.
pub fn apply_import(
    destination: &mut EditorState,
    source: &EditorState,
    plan: &ImportPlan,
    replacements: &HashMap<String, Asset>,
) -> Result<Value, CommandError> {
    let root = destination
        .library
        .root()
        .map(|folder| folder.id)
        .ok_or_else(|| {
            CommandError::new("unknown_element", "The destination library has no bin")
        })?;
    let source_name = source.project_name.clone();
    destination.transaction("Import from project", |destination| {
        for (old, new) in &plan.assets {
            let item = source
                .library
                .item(old)
                .ok_or_else(|| CommandError::unknown_element("media", old))?;
            let mut asset = replacements
                .get(old)
                .cloned()
                .or_else(|| item.asset.clone())
                .ok_or_else(|| {
                    CommandError::new("unportable_media", format!("`{old}` has no media"))
                })?;
            // A scene names splats by asset ID: its `requires` follow the rename.
            asset.meta.requires = asset
                .meta
                .requires
                .iter()
                .map(|id| {
                    let mapped = plan.map_asset(id.as_str()).unwrap_or(id.as_str());
                    Id::new(mapped).map_err(|error| invalid_id(error, mapped))
                })
                .collect::<Result<_, _>>()?;
            asset.decoded = None;
            let new_id = Id::new(new).map_err(|error| invalid_id(error, new))?;
            let ordinal = destination
                .library
                .items
                .iter()
                .filter(|other| other.kind == item.kind)
                .count()
                + 1;
            let prefix: String = item
                .code
                .chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            destination.library.add_item(MediaItem {
                id: new_id,
                code: format!("{prefix}{ordinal:02}"),
                name: item.name.clone(),
                meta: item.meta.clone(),
                kind: item.kind,
                folder: root,
                image: item.image,
                asset: Some(asset),
            })?;
        }
        for (old, new) in &plan.sequences {
            let from = source
                .sequence(old)
                .ok_or_else(|| CommandError::unknown_element("sequence", old))?;
            let mut timeline = from.clone();
            timeline.id = Id::new(new).map_err(|error| invalid_id(error, new))?;
            timeline.playhead = 0;
            timeline.selected_clip = None;
            timeline.selected_track = None;
            timeline.clips = from
                .clips
                .iter()
                .map(|clip| remap_clip(clip, plan))
                .collect::<Result<_, _>>()?;
            for clip in &from.clips {
                let Some(new_clip) = lookup(&plan.clips, clip.id.as_str()) else {
                    continue;
                };
                destination.inspector.copy_overrides_from(
                    &source.inspector,
                    clip.id.as_str(),
                    new_clip,
                );
            }
            destination.sequences.push(timeline);
        }
        if let Some(missing) = destination.dangling_clip_media().into_iter().next() {
            return Err(CommandError::new(
                "missing_dependency",
                format!("Clip `{missing}` lost its media during the import"),
            ));
        }
        Ok(())
    })?;
    Ok(
        json!({ "action": "project.import", "source": source_name, "plan": plan.to_json(),
        "sequences": plan.sequences.iter().map(|(_, to)| to.as_str()).collect::<Vec<_>>(),
        "assets": plan.assets.len(), "clips": plan.clips.len(), "changed": true }),
    )
}

fn remap_clip(clip: &Clip, plan: &ImportPlan) -> Result<Clip, CommandError> {
    let mut remapped = clip.clone();
    let new = lookup(&plan.clips, clip.id.as_str())
        .ok_or_else(|| CommandError::unknown_element("clip", clip.id.as_str()))?;
    remapped.id = Id::new(new).map_err(|error| invalid_id(error, new))?;
    if let Some(media) = clip.media {
        let mapped = plan.map_asset(media.as_str()).ok_or_else(|| {
            CommandError::new(
                "missing_dependency",
                format!("`{media}` is not in the import plan"),
            )
        })?;
        remapped.media = Some(Id::new(mapped).map_err(|error| invalid_id(error, mapped))?);
    }
    Ok(remapped)
}
