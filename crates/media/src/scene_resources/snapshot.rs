//! Immutable bytes for a long reader (BL109, F042): the approved files of a scene, copied once
//! into a directory the reader owns and checked against the approved identity while they are
//! copied.
//!
//! WHY: `scene.resources.resolve` hashes every file when the map is approved, but a map only
//! freezes *paths*. A file swapped for one of the same size afterwards (or while a long export is
//! running) would make the video a mixture of two assets and nothing would notice. The copy is
//! read **once**, hashed in the same pass that writes it, so the bytes that were verified are
//! exactly the bytes the reader gets: there is no window between "checked" and "used". Writes to
//! the original after that cannot reach the reader.
//!
//! The snapshot owns nothing outside `dir`: removing it is the caller's `Staging` (dropped on
//! success, failure and cancellation alike). Files are stored under `<dir>/<sha256>/<name>`, so
//! assets that share content share one copy, and the staged name keeps the original extension
//! (a renderer may pick the decoder from it).
//!
//! Nothing here depends on the render crate or on what the resources are: any caller that holds a
//! [`ResourceResolver`] and a list of [`ResourceKey`]s (images today; the splat and the audio mix
//! of S7/S9 are further keys) gets the same guarantee.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::json;
use tessel_editor_core::jobs::{CancelToken, JobError};
use tessel_editor_core::scene_resources::ResourceKey;

use crate::fsx::{copy_file_hashed, io_error, safe_file_name};

use super::map::{ApprovedResource, ResolveError, ResourceResolver};

/// One copy the snapshot made, remembered so it can be checked again.
#[derive(Debug, Clone)]
struct Staged {
    path: PathBuf,
    bytes: u64,
    modified: Option<SystemTime>,
}

/// The approved resources of one scene, as private read-only copies. It answers the same
/// [`ResourceResolver`] questions as the map it was taken from, with the same identity
/// (hash, revision, generation, cache key) and a path inside the snapshot directory.
#[derive(Debug)]
pub struct ResourceSnapshot {
    scene_id: String,
    entries: BTreeMap<ResourceKey, ApprovedResource>,
    staged: Vec<Staged>,
    bytes: u64,
}

fn changed(
    key: &ResourceKey,
    message: String,
    expected: &ApprovedResource,
    got: serde_json::Value,
) -> JobError {
    JobError::new("resource_changed", message).with_details(json!({
        "resource": key.label(),
        "expected": { "sha256": expected.content_hash, "bytes": expected.bytes },
        "got": got,
    }))
}

/// Bytes [`ResourceSnapshot::take`] would copy for `keys`: every distinct content once. The
/// caller uses it to check the disk and to size the progress bar before any byte moves.
pub fn snapshot_bytes(
    resolver: &dyn ResourceResolver,
    scene_id: &str,
    keys: &[ResourceKey],
) -> Result<u64, ResolveError> {
    let mut seen = BTreeMap::new();
    for key in keys {
        let approved = resolver.resolve(scene_id, key)?;
        seen.insert(approved.content_hash, approved.bytes);
    }
    Ok(seen.values().sum())
}

impl ResourceSnapshot {
    /// Copies and verifies every resource of `keys` into `dir` (created if missing).
    ///
    /// A file that is missing, of another size or of another SHA-256 than the one approved fails
    /// with `resource_changed` / `resource_missing`, naming the resource and both identities.
    /// `progress` receives the bytes copied since the previous call. Cancellation is observed
    /// between chunks. On any error the files already copied stay in `dir`: the directory is the
    /// caller's staging and goes away with it.
    pub fn take(
        resolver: &dyn ResourceResolver,
        scene_id: &str,
        keys: &[ResourceKey],
        dir: &Path,
        token: &CancelToken,
        mut progress: impl FnMut(u64),
    ) -> Result<Self, JobError> {
        let mut approved = Vec::with_capacity(keys.len());
        for key in keys {
            approved.push((key.clone(), resolver.resolve(scene_id, key)?));
        }
        fs::create_dir_all(dir).map_err(|error| io_error(dir, &error))?;

        let mut by_hash: BTreeMap<String, PathBuf> = BTreeMap::new();
        let mut staged = Vec::new();
        let mut entries = BTreeMap::new();
        let mut total = 0u64;
        for (key, resource) in approved {
            let path = match by_hash.get(&resource.content_hash) {
                Some(path) => path.clone(),
                None => {
                    let path = copy_one(&key, &resource, dir, token, &mut progress)?;
                    let modified = fs::metadata(&path).and_then(|meta| meta.modified()).ok();
                    staged.push(Staged {
                        path: path.clone(),
                        bytes: resource.bytes,
                        modified,
                    });
                    total += resource.bytes;
                    by_hash.insert(resource.content_hash.clone(), path.clone());
                    path
                }
            };
            entries.insert(key, ApprovedResource { path, ..resource });
        }
        Ok(Self {
            scene_id: scene_id.to_string(),
            entries,
            staged,
            bytes: total,
        })
    }

    /// Distinct bytes held by the snapshot.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Distinct files held by the snapshot.
    pub fn files(&self) -> usize {
        self.staged.len()
    }

    /// Every private copy still has the size and modification time it had when it was verified.
    /// A cheap check (no hashing) that something other than this process did not rewrite the
    /// staging area while a reader was using it.
    pub fn verify_intact(&self) -> Result<(), JobError> {
        for staged in &self.staged {
            let meta = fs::metadata(&staged.path).map_err(|error| {
                JobError::new(
                    "resource_changed",
                    format!(
                        "The private copy `{}` is gone: {error}",
                        staged.path.display()
                    ),
                )
            })?;
            if meta.len() != staged.bytes || meta.modified().ok() != staged.modified {
                return Err(JobError::new(
                    "resource_changed",
                    format!(
                        "The private copy `{}` was modified while it was being read",
                        staged.path.display()
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn copy_one(
    key: &ResourceKey,
    resource: &ApprovedResource,
    dir: &Path,
    token: &CancelToken,
    progress: &mut impl FnMut(u64),
) -> Result<PathBuf, JobError> {
    let name = resource
        .path
        .file_name()
        .map(|name| safe_file_name(&name.to_string_lossy()))
        .unwrap_or_else(|| "file".to_string());
    let folder = dir.join(&resource.content_hash);
    fs::create_dir_all(&folder).map_err(|error| io_error(&folder, &error))?;
    let target = folder.join(name);

    let mut last = 0u64;
    let outcome = copy_file_hashed(&resource.path, &target, token, |total| {
        progress(total - last);
        last = total;
    });
    let (hash, size) = match outcome {
        Ok(result) => result,
        Err(error) if error.code == "asset_unreadable" => {
            return Err(JobError::new(
                "resource_missing",
                format!(
                    "The file of `{}` cannot be read: {}",
                    key.label(),
                    error.message
                ),
            )
            .with_details(json!({ "resource": key.label() })));
        }
        Err(error) => return Err(error),
    };
    if hash != resource.content_hash || size != resource.bytes {
        let _ = fs::remove_file(&target);
        return Err(changed(
            key,
            format!(
                "The file of `{}` is not the one that was approved (approved {} bytes {}, found {size} bytes {hash})",
                key.label(),
                resource.bytes,
                resource.content_hash,
            ),
            resource,
            json!({ "sha256": hash, "bytes": size }),
        ));
    }
    seal(&target)?;
    Ok(target)
}

/// Read-only: the renderer never writes it and an accidental write fails loudly.
fn seal(path: &Path) -> Result<(), JobError> {
    let mut permissions = fs::metadata(path)
        .map_err(|error| io_error(path, &error))?
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).map_err(|error| io_error(path, &error))
}

impl ResourceResolver for ResourceSnapshot {
    fn resolve(&self, scene_id: &str, key: &ResourceKey) -> Result<ApprovedResource, ResolveError> {
        if scene_id != self.scene_id {
            return Err(ResolveError::SceneNotLoaded {
                scene: scene_id.to_string(),
            });
        }
        // A resource that was not part of the snapshot is not answered from anywhere else: the
        // original path is exactly what must not reach the reader.
        self.entries
            .get(key)
            .cloned()
            .ok_or_else(|| ResolveError::UnknownAsset {
                scene: self.scene_id.clone(),
                resource: key.label(),
            })
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;
    use sha2::{Digest, Sha256};
    use tessel_editor_core::scene_resources::ResourceKind;

    use crate::scene_resources::map::Generation;

    struct Fixed(BTreeMap<ResourceKey, ApprovedResource>);

    impl ResourceResolver for Fixed {
        fn resolve(&self, _: &str, key: &ResourceKey) -> Result<ApprovedResource, ResolveError> {
            self.0
                .get(key)
                .cloned()
                .ok_or_else(|| ResolveError::UnknownAsset {
                    scene: "s".into(),
                    resource: key.label(),
                })
        }
    }

    fn approve(id: &str, path: &Path, bytes: &[u8]) -> (ResourceKey, ApprovedResource) {
        let key = ResourceKey::asset(id);
        let resource = ApprovedResource {
            scene_id: "s".into(),
            key: key.clone(),
            kind: ResourceKind::Image,
            path: path.to_path_buf(),
            content_hash: crate::fsx::hex(&Sha256::digest(bytes)),
            revision_id: Some("rev".into()),
            bytes: bytes.len() as u64,
            generation: Generation(3),
            cache_key: format!("cache-{id}"),
            node_ids: Vec::new(),
        };
        (key, resource)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("te-snap-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_copy_survives_a_same_size_swap_of_the_original_and_shares_equal_content() {
        let dir = scratch("swap");
        let (a, b, c) = (dir.join("a.png"), dir.join("b.png"), dir.join("c.png"));
        fs::write(&a, b"AAAA").unwrap();
        fs::write(&b, b"AAAA").unwrap();
        fs::write(&c, b"CCCC").unwrap();
        let entries: BTreeMap<_, _> = [
            approve("a", &a, b"AAAA"),
            approve("b", &b, b"AAAA"),
            approve("c", &c, b"CCCC"),
        ]
        .into();
        let keys: Vec<_> = entries.keys().cloned().collect();
        let resolver = Fixed(entries);
        assert_eq!(snapshot_bytes(&resolver, "s", &keys).unwrap(), 8);

        let mut seen = 0;
        let snapshot = ResourceSnapshot::take(
            &resolver,
            "s",
            &keys,
            &dir.join("snap"),
            &CancelToken::new(),
            |bytes| seen += bytes,
        )
        .unwrap();
        assert_eq!((snapshot.files(), snapshot.bytes(), seen), (2, 8, 8));

        // The original changes after the copy: same size, different bytes.
        fs::write(&a, b"ZZZZ").unwrap();
        let got = snapshot.resolve_asset("s", "a").unwrap();
        assert_eq!(fs::read(&got.path).unwrap(), b"AAAA");
        assert!(got.path.starts_with(dir.join("snap")));
        assert_eq!(got.path.extension().unwrap(), "png");
        assert_eq!(got.cache_key, "cache-a", "identity is the approved one");
        assert_eq!(
            snapshot.resolve_asset("s", "b").unwrap().path,
            got.path,
            "equal content is stored once"
        );
        assert!(fs::write(&got.path, b"x").is_err(), "the copy is read-only");
        snapshot.verify_intact().unwrap();
        assert_eq!(
            snapshot.resolve_asset("other", "a").unwrap_err().code(),
            "scene_resources_not_loaded"
        );
        assert_eq!(
            snapshot.resolve_asset("s", "zzz").unwrap_err().code(),
            "resource_unknown"
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_swap_of_the_same_size_before_the_copy_is_refused_with_both_identities() {
        let dir = scratch("early");
        let a = dir.join("a.png");
        fs::write(&a, b"AAAA").unwrap();
        let (key, approved) = approve("a", &a, b"AAAA");
        fs::write(&a, b"BBBB").unwrap();
        let resolver = Fixed([(key.clone(), approved)].into());
        let error = ResourceSnapshot::take(
            &resolver,
            "s",
            &[key],
            &dir.join("snap"),
            &CancelToken::new(),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error.code, "resource_changed", "{}", error.message);
        let details = error.details.unwrap();
        assert_eq!(details["resource"], "asset:a");
        assert_eq!(details["got"]["bytes"], 4);
        assert_ne!(details["got"]["sha256"], details["expected"]["sha256"]);
        assert_eq!(
            fs::read(&a).unwrap(),
            b"BBBB",
            "the original is never written"
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_missing_original_and_a_cancelled_copy_have_their_own_codes() {
        let dir = scratch("codes");
        let a = dir.join("a.png");
        fs::write(&a, b"AAAA").unwrap();
        let (key, approved) = approve("a", &a, b"AAAA");
        let resolver = Fixed([(key.clone(), approved)].into());
        let cancel = CancelToken::new();
        cancel.cancel();
        let error = ResourceSnapshot::take(
            &resolver,
            "s",
            std::slice::from_ref(&key),
            &dir.join("s1"),
            &cancel,
            |_| {},
        )
        .unwrap_err();
        assert!(error.is_cancelled());

        fs::remove_file(&a).unwrap();
        let error = ResourceSnapshot::take(
            &resolver,
            "s",
            &[key],
            &dir.join("s2"),
            &CancelToken::new(),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error.code, "resource_missing", "{}", error.message);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn tampering_with_the_private_copy_is_detected() {
        let dir = scratch("tamper");
        let a = dir.join("a.png");
        fs::write(&a, b"AAAA").unwrap();
        let (key, approved) = approve("a", &a, b"AAAA");
        let resolver = Fixed([(key.clone(), approved)].into());
        let snapshot = ResourceSnapshot::take(
            &resolver,
            "s",
            &[key],
            &dir.join("snap"),
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
        let private = snapshot.resolve_asset("s", "a").unwrap().path;
        fs::remove_file(&private).unwrap();
        assert_eq!(
            snapshot.verify_intact().unwrap_err().code,
            "resource_changed"
        );
        fs::remove_dir_all(dir).ok();
    }
}
