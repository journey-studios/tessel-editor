//! Library thumbnails of splats and scenes (BL144): the rules, none of the pixels.
//!
//! A splat or a `tessel-scene` has no frame of its own, so the library tile was an icon. The
//! media adapter renders one real frame of it (a `Source` plan of the asset, through the same
//! renderer the export uses) and hands the PNG back through a job. This module decides *what*
//! is wanted and *whether a result still belongs*, so the window, the headless CLI and the
//! tests apply one rule:
//!
//! - A thumbnail has an **identity**: kind, content hash, the picture size, the rule version
//!   **and what the picture is drawn from besides its own file**: a scene is drawn with the
//!   splats it names, so every asset it depends on (transitively, by asset ID) is part of the
//!   identity with its kind and content hash, or with `missing` when the library lacks it. A
//!   relinked splat, a splat imported after the scene failed to draw, a removed one: each is a
//!   different identity, so a stale picture is never shown and a remembered failure is never
//!   kept once its cause is gone. The identity is computed from the model, with no file read.
//! - A result carries the **stamp** of the project it was rendered for (ID and editing
//!   session). Reopening a document, even the same one, is a new session: a result of the
//!   previous project is refused with `stale_result` and nothing is stored.
//! - A result for an asset whose identity changed while the job ran is refused too.
//!
//! Thumbnails are session state, like the playhead: not in the document, not in an undo
//! snapshot, not an edit (no revision bump). They live in [`EditorState::thumbnails`], which a
//! new project starts empty.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::assets::{MediaKind, Source};
use crate::state::library::MediaItem;
use crate::state::timeline::SequenceSettings;
use crate::state::{CommandError, EditorState, ProjectSettings};

/// Pixels across a rendered thumbnail; the height follows the sequence's aspect ratio.
pub const WIDTH: u32 = 240;
/// Bump when the way a thumbnail is drawn changes: every identity changes with it.
pub const VERSION: u32 = 2;

/// What draws a picture of itself only through the renderer.
pub fn is_rendered(kind: MediaKind) -> bool {
    matches!(kind, MediaKind::Splat | MediaKind::Scene)
}

/// The picture size for a sequence of `width` × `height`: [`WIDTH`] across, an even height.
pub fn size_for(width: u32, height: u32) -> (u32, u32) {
    let height = (u64::from(WIDTH) * u64::from(height.max(1)) / u64::from(width.max(1))).max(2);
    let maximum = crate::preset::MAX_SIDE;
    if height > u64::from(maximum) {
        let width = (u64::from(WIDTH) * u64::from(maximum) / height).max(2) as u32;
        (width.next_multiple_of(2), maximum)
    } else {
        (WIDTH, (height as u32).next_multiple_of(2))
    }
}

/// The project a result was rendered for. Taken when the job starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    project: String,
    session: u64,
}

impl Stamp {
    pub fn is_current(&self, state: &EditorState) -> bool {
        self.project == state.project.id && self.session == state.project.session.number()
    }

    pub fn project(&self) -> &str {
        &self.project
    }
}

/// One thumbnail the project wants.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub asset: String,
    pub kind: MediaKind,
    pub identity: String,
    pub width: u32,
    pub height: u32,
}

/// A thumbnail the adapter made.
#[derive(Debug, Clone, PartialEq)]
pub struct Thumbnail {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub identity: String,
}

/// Why a thumbnail could not be made, kept so it is neither retried every tick nor hidden.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub identity: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct ThumbnailBook {
    ready: BTreeMap<String, Thumbnail>,
    failed: BTreeMap<String, Failure>,
}

impl ThumbnailBook {
    pub fn is_empty(&self) -> bool {
        self.ready.is_empty() && self.failed.is_empty()
    }
}

/// What a drawn asset needs besides its own file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub id: String,
    pub state: DependencyState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyState {
    /// In the library with a file whose content hash is known.
    Hashed { kind: &'static str, hash: String },
    /// In the library but not hashed yet (the import is still running): no identity until it is.
    Unhashed,
    /// Not in the library (or an entry with no media behind it).
    Missing,
}

impl Dependency {
    /// The part of an identity that names this dependency.
    fn token(&self) -> String {
        match &self.state {
            DependencyState::Hashed { kind, hash } => format!("{}={kind}:{hash}", self.id),
            DependencyState::Unhashed => format!("{}=unhashed", self.id),
            DependencyState::Missing => format!("{}=missing", self.id),
        }
    }

    pub fn is_missing(&self) -> bool {
        self.state == DependencyState::Missing
    }
}

/// The identity of the picture `item` would have; `None` for what is not drawn through the
/// renderer, has no file, was never hashed, or depends on something that was not hashed yet.
fn identity_of(state: &EditorState, item: &MediaItem, size: (u32, u32)) -> Option<String> {
    let asset = item.asset.as_ref()?;
    if !is_rendered(item.kind) || !matches!(asset.source, Source::File { .. }) {
        return None;
    }
    let hash = asset.identity.hash.as_deref()?;
    let dependencies = state.thumbnail_dependencies(item.id.as_str());
    if dependencies
        .iter()
        .any(|dependency| dependency.state == DependencyState::Unhashed)
    {
        return None;
    }
    let dependencies = if dependencies.is_empty() {
        "-".to_string()
    } else {
        dependencies
            .iter()
            .map(Dependency::token)
            .collect::<Vec<_>>()
            .join(",")
    };
    // How the picture is framed: a scene draws itself; a splat is framed by its measured box
    // from the default view. A capture's own view (BL144.2) will take the place of `default`.
    let framing = match item.kind {
        MediaKind::Splat => "fit:default",
        _ => "own",
    };
    Some(format!(
        "v{VERSION}|{}|{hash}|{}x{}|deps={dependencies}|framing={framing}",
        item.kind.label(),
        size.0,
        size.1
    ))
}

/// 64-bit FNV-1a, to derive short stable names. Not a security boundary.
fn fnv64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Why a result was refused.
fn stale(message: impl Into<String>, id: &str) -> CommandError {
    CommandError::new("stale_result", message).target(id)
}

impl EditorState {
    pub fn thumbnail_stamp(&self) -> Stamp {
        Stamp {
            project: self.project.id.clone(),
            session: self.project.session.number(),
        }
    }

    fn thumbnail_size(&self) -> (u32, u32) {
        size_for(self.timeline.width, self.timeline.height)
    }

    /// The identity `id` would have right now.
    pub fn thumbnail_identity(&self, id: &str) -> Option<String> {
        let item = self.library.item(id)?;
        identity_of(self, item, self.thumbnail_size())
    }

    /// Everything the picture of `id` is drawn from besides its own file: the assets it names
    /// (`meta.requires`), and theirs in turn, by asset ID in this library, sorted by ID. Cheap:
    /// model lookups only, no file is read.
    pub fn thumbnail_dependencies(&self, id: &str) -> Vec<Dependency> {
        let mut found: BTreeMap<String, Dependency> = BTreeMap::new();
        let mut queue: Vec<String> = self
            .library
            .item(id)
            .and_then(|item| item.asset.as_ref())
            .map(|asset| {
                asset
                    .meta
                    .requires
                    .iter()
                    .map(|required| required.as_str().to_string())
                    .collect()
            })
            .unwrap_or_default();
        while let Some(next) = queue.pop() {
            if next == id || found.contains_key(&next) {
                continue;
            }
            let state = match self
                .library
                .item(&next)
                .and_then(|item| item.asset.as_ref().map(|asset| (item.kind, asset)))
            {
                None => DependencyState::Missing,
                Some((kind, asset)) => {
                    queue.extend(
                        asset
                            .meta
                            .requires
                            .iter()
                            .map(|required| required.as_str().to_string()),
                    );
                    match asset.identity.hash.as_deref() {
                        Some(hash) => DependencyState::Hashed {
                            kind: kind.label(),
                            hash: hash.to_string(),
                        },
                        None => DependencyState::Unhashed,
                    }
                }
            };
            found.insert(next.clone(), Dependency { id: next, state });
        }
        found.into_values().collect()
    }

    /// A private project that holds only what drawing `targets` needs: their assets and the
    /// assets those depend on, in a sequence of the same canvas. The job draws from this, never
    /// from a copy of the whole project (whose undo history would be copied with it), and never
    /// touches the open project. Every target keeps the identity it was asked with.
    pub fn thumbnail_snapshot(&self, targets: &[Target]) -> Result<EditorState, CommandError> {
        let mut wanted: Vec<String> = Vec::new();
        for target in targets {
            wanted.push(target.asset.clone());
            wanted.extend(
                self.thumbnail_dependencies(&target.asset)
                    .into_iter()
                    .map(|dependency| dependency.id),
            );
        }
        wanted.sort();
        wanted.dedup();
        let mut copy = EditorState::empty(&ProjectSettings {
            name: self.project_name.clone(),
            sequence: SequenceSettings {
                id: self.timeline.id,
                rate: self.timeline.rate,
                length: self.timeline.length,
                width: self.timeline.width,
                height: self.timeline.height,
            },
            workspace: self.project.workspace.clone(),
        })?;
        copy.project.id = self.project.id.clone();
        for id in &wanted {
            if let Some(item) = self.library.item(id) {
                copy.library.items.push(item.clone());
            }
        }
        Ok(copy)
    }

    /// The ID of the one-object scene that stands in for the splat `splat` while its thumbnail
    /// is drawn: short and stable (`thumb-` and 16 hex digits of the splat's ID, whatever the
    /// length of that ID), and never the ID of an asset this project has.
    pub fn thumbnail_stand_in_id(&self, splat: &str) -> String {
        let base = format!("thumb-{:016x}", fnv64(splat.as_bytes()));
        let mut id = base.clone();
        let mut attempt = 2;
        while self.library.item(&id).is_some() {
            id = format!("{base}-{attempt}");
            attempt += 1;
        }
        id
    }

    /// The thumbnail of `id`, only while it is still the picture of the asset as it is now.
    pub fn thumbnail(&self, id: &str) -> Option<&Thumbnail> {
        let current = self.thumbnail_identity(id)?;
        self.thumbnails
            .ready
            .get(id)
            .filter(|thumbnail| thumbnail.identity == current)
    }

    /// What `id` failed with, only while it is still failing for the asset as it is now.
    pub fn thumbnail_failure(&self, id: &str) -> Option<&Failure> {
        let current = self.thumbnail_identity(id)?;
        self.thumbnails
            .failed
            .get(id)
            .filter(|failure| failure.identity == current)
    }

    /// The thumbnails to make: every splat and scene with a hashed file and neither a current
    /// thumbnail nor a recorded failure for its current identity. `retry` also asks again for
    /// the ones that failed (the runtime may have been installed since).
    pub fn thumbnail_targets(&self, retry: bool) -> Vec<Target> {
        let (width, height) = self.thumbnail_size();
        self.library
            .items
            .iter()
            .filter_map(|item| {
                let identity = identity_of(self, item, (width, height))?;
                let id = item.id.as_str();
                let done = self
                    .thumbnails
                    .ready
                    .get(id)
                    .is_some_and(|thumbnail| thumbnail.identity == identity)
                    || (!retry
                        && self
                            .thumbnails
                            .failed
                            .get(id)
                            .is_some_and(|failure| failure.identity == identity));
                (!done).then(|| Target {
                    asset: id.to_string(),
                    kind: item.kind,
                    identity,
                    width,
                    height,
                })
            })
            .collect()
    }

    /// Stores a thumbnail the adapter rendered for the project `stamp` names. Refused with
    /// `stale_result` when that project is no longer the open one, or when the asset is not the
    /// one the picture was rendered from; nothing changes then.
    pub fn set_thumbnail(
        &mut self,
        stamp: &Stamp,
        id: &str,
        thumbnail: Thumbnail,
    ) -> Result<(), CommandError> {
        self.check_thumbnail(stamp, id, &thumbnail.identity)?;
        self.thumbnails.failed.remove(id);
        self.thumbnails.ready.insert(id.to_string(), thumbnail);
        Ok(())
    }

    /// Records that the thumbnail with `identity` could not be made, under the same guards.
    pub fn set_thumbnail_failure(
        &mut self,
        stamp: &Stamp,
        id: &str,
        failure: Failure,
    ) -> Result<(), CommandError> {
        self.check_thumbnail(stamp, id, &failure.identity)?;
        self.thumbnails.failed.insert(id.to_string(), failure);
        Ok(())
    }

    fn check_thumbnail(&self, stamp: &Stamp, id: &str, identity: &str) -> Result<(), CommandError> {
        if !stamp.is_current(self) {
            return Err(stale(
                "Another project (or another opening of it) is open now; the thumbnail was not applied",
                id,
            ));
        }
        if self.thumbnail_identity(id).as_deref() != Some(identity) {
            return Err(stale(
                format!(
                    "`{id}` changed while its thumbnail was made; the thumbnail was not applied"
                ),
                id,
            ));
        }
        Ok(())
    }

    /// Forgets the thumbnails of assets that are gone or whose identity changed, so the book
    /// never grows with what the project no longer has.
    pub fn prune_thumbnails(&mut self) {
        let keep = |state: &Self, id: &str, identity: &str| {
            state.thumbnail_identity(id).as_deref() == Some(identity)
        };
        let ready: Vec<String> = self
            .thumbnails
            .ready
            .iter()
            .filter(|(id, thumbnail)| !keep(self, id, &thumbnail.identity))
            .map(|(id, _)| id.clone())
            .collect();
        let failed: Vec<String> = self
            .thumbnails
            .failed
            .iter()
            .filter(|(id, failure)| !keep(self, id, &failure.identity))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ready {
            self.thumbnails.ready.remove(&id);
        }
        for id in failed {
            self.thumbnails.failed.remove(&id);
        }
    }

    /// The thumbnail facts of one library item for the automation tree and `media.thumbnails`.
    pub fn thumbnail_report(&self, id: &str) -> Value {
        let Some(identity) = self.thumbnail_identity(id) else {
            return json!({ "state": "none" });
        };
        if let Some(thumbnail) = self.thumbnail(id) {
            return json!({ "state": "ready", "path": thumbnail.path, "width": thumbnail.width,
                "height": thumbnail.height, "identity": identity });
        }
        if let Some(failure) = self.thumbnail_failure(id) {
            return json!({ "state": "failed", "code": failure.code, "message": failure.message,
                "identity": identity });
        }
        json!({ "state": "pending", "identity": identity })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn the_picture_follows_the_sequence_aspect_with_an_even_height() {
        assert_eq!(size_for(1920, 1080), (WIDTH, 135 + 1));
        assert_eq!(size_for(1080, 1920), (WIDTH, 426));
        assert_eq!(size_for(1000, 1000), (WIDTH, WIDTH));
        assert_eq!(size_for(0, 0).0, WIDTH);
        for (width, height) in [(1, 8192), (8192, 1), (1, u32::MAX), (u32::MAX, 1)] {
            let (w, h) = size_for(width, height);
            assert!((2..=crate::preset::MAX_SIDE).contains(&w));
            assert!((2..=crate::preset::MAX_SIDE).contains(&h));
            assert_eq!(w % 2, 0);
            assert_eq!(h % 2, 0);
        }
    }

    #[test]
    fn only_splats_and_scenes_are_drawn_through_the_renderer() {
        assert!(is_rendered(MediaKind::Splat) && is_rendered(MediaKind::Scene));
        for kind in [
            MediaKind::Still,
            MediaKind::Video,
            MediaKind::Audio,
            MediaKind::Text,
            MediaKind::Timeline,
        ] {
            assert!(!is_rendered(kind));
        }
    }

    #[test]
    fn a_new_project_wants_nothing_and_has_nothing() {
        let state = EditorState::demo();
        assert!(state.thumbnail_targets(false).is_empty());
        assert!(state.thumbnails.is_empty());
        assert_eq!(state.thumbnail_report("ao01")["state"], "none");
    }
}
