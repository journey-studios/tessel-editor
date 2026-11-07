//! Library thumbnails of splats and scenes (BL144): the rules, none of the pixels.
//!
//! A splat or a `tessel-scene` has no frame of its own, so the library tile was an icon. The
//! media adapter renders one real frame of it (a `Source` plan of the asset, through the same
//! renderer the export uses) and hands the PNG back through a job. This module decides *what*
//! is wanted and *whether a result still belongs*, so the window, the headless CLI and the
//! tests apply one rule:
//!
//! - A thumbnail has an **identity**: kind, content hash, the picture size and the rule
//!   version. Whatever differs is a different picture, so a relinked file never shows the old
//!   thumbnail.
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

use crate::assets::{Asset, MediaKind, Source};
use crate::state::{CommandError, EditorState};

/// Pixels across a rendered thumbnail; the height follows the sequence's aspect ratio.
pub const WIDTH: u32 = 240;
/// Bump when the way a thumbnail is drawn changes: every identity changes with it.
pub const VERSION: u32 = 1;

/// What draws a picture of itself only through the renderer.
pub fn is_rendered(kind: MediaKind) -> bool {
    matches!(kind, MediaKind::Splat | MediaKind::Scene)
}

/// The picture size for a sequence of `width` × `height`: [`WIDTH`] across, an even height.
pub fn size_for(width: u32, height: u32) -> (u32, u32) {
    let height = (u64::from(WIDTH) * u64::from(height.max(1)) / u64::from(width.max(1))).max(2);
    (WIDTH, (height as u32).next_multiple_of(2))
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

/// The identity of the picture `asset` would have as `kind`; `None` for what is not drawn
/// through the renderer, has no file or was never hashed.
fn identity_of(kind: MediaKind, asset: &Asset, size: (u32, u32)) -> Option<String> {
    if !is_rendered(kind) || !matches!(asset.source, Source::File { .. }) {
        return None;
    }
    let hash = asset.identity.hash.as_deref()?;
    Some(format!(
        "v{VERSION}|{}|{hash}|{}x{}",
        kind.label(),
        size.0,
        size.1
    ))
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
        identity_of(item.kind, item.asset.as_ref()?, self.thumbnail_size())
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
                let identity = identity_of(item.kind, item.asset.as_ref()?, (width, height))?;
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
