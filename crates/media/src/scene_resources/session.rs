//! The session that owns every approved resource map of the open project: install, resolve,
//! release, invalidate, and a byte budget (BL010).
//!
//! Ownership is explicit. A map exists only between `install` and `release`/`invalidate`; the
//! session is the single owner, hands out owned [`ApprovedResource`] values and never a
//! reference, and forgets everything when the generation moves. Loading itself (hashing,
//! header checks) is `load::load`, which runs as a job; only the cheap `install` happens on the
//! caller's thread, and it re-checks the generation, so a job that finished after a project
//! switch can never install anything.
//!
//! Thread-safe (`Clone` shares the state): the host, a job's completion and a renderer thread
//! may all hold a handle.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{Value, json};
use tessel_editor_core::scene_resources::ResourceKey;

use super::map::{
    ApprovedResource, Generation, ResolveError, ResolvedResourceMap, ResourceResolver,
};

/// Approved file bytes the session accepts at once. WHY bounded: every installed map pins files
/// that a renderer is allowed to open and keep decoded; without a bound a project with many
/// scenes could ask a renderer for unbounded memory. The renderer applies its own, smaller
/// texture budget on top.
pub const DEFAULT_BUDGET_BYTES: u64 = 2 << 30;

/// Why the generation moved. Informational (it appears in `status`); every reason has the same
/// effect: everything approved before is forgotten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invalidation {
    ProjectSwitch,
    /// The renderer reloaded or the document was reopened.
    Reload,
    /// The graphics context was lost: textures built from approved files are gone.
    ContextLoss,
    /// An asset of a scene was replaced (relink, new content).
    AssetSwap,
    /// The caller asked for it (`scene.resources.invalidate`).
    Requested,
}

impl Invalidation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProjectSwitch => "project_switch",
            Self::Reload => "reload",
            Self::ContextLoss => "context_loss",
            Self::AssetSwap => "asset_swap",
            Self::Requested => "requested",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        [
            Self::ProjectSwitch,
            Self::Reload,
            Self::ContextLoss,
            Self::AssetSwap,
            Self::Requested,
        ]
        .into_iter()
        .find(|reason| reason.as_str() == label)
    }
}

struct Inner {
    project: Option<String>,
    generation: u64,
    last_invalidation: Option<Invalidation>,
    maps: BTreeMap<String, Arc<ResolvedResourceMap>>,
    budget: u64,
}

#[derive(Clone)]
pub struct SceneResourceSession {
    inner: Arc<Mutex<Inner>>,
}

impl Default for SceneResourceSession {
    fn default() -> Self {
        Self::with_budget(DEFAULT_BUDGET_BYTES)
    }
}

impl SceneResourceSession {
    pub fn with_budget(budget: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                project: None,
                generation: 1,
                last_invalidation: None,
                maps: BTreeMap::new(),
                budget,
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn generation(&self) -> Generation {
        Generation(self.lock().generation)
    }

    pub fn project(&self) -> Option<String> {
        self.lock().project.clone()
    }

    pub fn budget(&self) -> u64 {
        self.lock().budget
    }

    /// Tells the session which project is open. The first call adopts it; a different project
    /// invalidates everything (`ProjectSwitch`). Called by the host on every request, so a
    /// project swapped by `project.open`, `project.restore` or a window is noticed even though
    /// the session never sees the swap itself.
    pub fn observe_project(&self, project_id: &str) -> Generation {
        let mut inner = self.lock();
        match inner.project.as_deref() {
            Some(current) if current == project_id => {}
            Some(_) => {
                inner.project = Some(project_id.to_string());
                Self::bump(&mut inner, Invalidation::ProjectSwitch);
            }
            None => inner.project = Some(project_id.to_string()),
        }
        Generation(inner.generation)
    }

    fn bump(inner: &mut Inner, reason: Invalidation) {
        inner.generation += 1;
        inner.last_invalidation = Some(reason);
        inner.maps.clear();
    }

    /// Forgets every installed map and starts a new generation: what any renderer, job or
    /// frame in flight holds from the old one will be refused.
    pub fn invalidate(&self, reason: Invalidation) -> Generation {
        let mut inner = self.lock();
        Self::bump(&mut inner, reason);
        Generation(inner.generation)
    }

    /// Takes ownership of `map`. Refused without any change when it was approved under an old
    /// generation or for another project, or when it would exceed the budget. It replaces the
    /// scene's previous map, whose bytes stop counting.
    pub fn install(
        &self,
        map: ResolvedResourceMap,
    ) -> Result<Arc<ResolvedResourceMap>, ResolveError> {
        let mut inner = self.lock();
        let current = Generation(inner.generation);
        let wrong_project = inner
            .project
            .as_deref()
            .is_some_and(|project| project != map.project_id);
        if map.generation != current || wrong_project {
            return Err(ResolveError::Stale {
                scene: map.scene_id,
                held: map.generation,
                current,
            });
        }
        let others: u64 = inner
            .maps
            .iter()
            .filter(|(scene, _)| **scene != map.scene_id)
            .map(|(_, held)| held.approved_bytes())
            .sum();
        let needed = others.saturating_add(map.approved_bytes());
        if needed > inner.budget {
            return Err(ResolveError::BudgetExceeded {
                needed,
                budget: inner.budget,
            });
        }
        let installed = Arc::new(map);
        inner
            .maps
            .insert(installed.scene_id.clone(), installed.clone());
        Ok(installed)
    }

    /// Drops one scene's map (a detached scene, a scene that is no longer shown).
    pub fn release(&self, scene_id: &str) -> bool {
        self.lock().maps.remove(scene_id).is_some()
    }

    pub fn get(&self, scene_id: &str) -> Option<Arc<ResolvedResourceMap>> {
        self.lock().maps.get(scene_id).cloned()
    }

    pub fn loaded_scenes(&self) -> Vec<String> {
        self.lock().maps.keys().cloned().collect()
    }

    pub fn held_bytes(&self) -> u64 {
        self.lock()
            .maps
            .values()
            .map(|map| map.approved_bytes())
            .sum()
    }

    pub fn status(&self) -> Value {
        let inner = self.lock();
        json!({
            "project": inner.project,
            "generation": inner.generation,
            "lastInvalidation": inner.last_invalidation.map(Invalidation::as_str),
            "budgetBytes": inner.budget,
            "heldBytes": inner.maps.values().map(|map| map.approved_bytes()).sum::<u64>(),
            "scenes": inner.maps.values().map(|map| json!({
                "scene": map.scene_id,
                "bundleRevision": map.bundle_revision,
                "generation": map.generation.0,
                "resources": map.entries.len(),
                "complete": map.complete,
                "approvedBytes": map.approved_bytes(),
            })).collect::<Vec<_>>(),
        })
    }
}

impl ResourceResolver for SceneResourceSession {
    fn resolve(&self, scene_id: &str, key: &ResourceKey) -> Result<ApprovedResource, ResolveError> {
        let inner = self.lock();
        let map = inner
            .maps
            .get(scene_id)
            .ok_or_else(|| ResolveError::SceneNotLoaded {
                scene: scene_id.to_string(),
            })?;
        // Unreachable while `bump` clears the maps, and kept because the contract of a map is
        // "never answer for a generation that is not current".
        if map.generation.0 != inner.generation {
            return Err(ResolveError::Stale {
                scene: scene_id.to_string(),
                held: map.generation,
                current: Generation(inner.generation),
            });
        }
        map.approved(key)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::scene_resources::map::ResolvedResource;
    use tessel_editor_core::scene_resources::ResourceKind;

    fn map(scene: &str, generation: Generation, bytes: u64) -> ResolvedResourceMap {
        ResolvedResourceMap {
            scene_id: scene.to_string(),
            bundle_id: "bundle".to_string(),
            bundle_revision: "r1".to_string(),
            project_id: "p1".to_string(),
            generation,
            base: PathBuf::from("/base"),
            entries: vec![ResolvedResource {
                key: ResourceKey::asset("hero"),
                kind: ResourceKind::Image,
                path: PathBuf::from(format!("/base/{scene}/hero.png")),
                content_hash: "a".repeat(64),
                revision_id: Some("rev-1".to_string()),
                bytes,
                uses: Vec::new(),
            }],
            skipped: Vec::new(),
            complete: true,
        }
    }

    fn session() -> SceneResourceSession {
        let session = SceneResourceSession::with_budget(100);
        session.observe_project("p1");
        session
    }

    #[test]
    fn an_installed_map_resolves_by_asset_id_with_a_scoped_cache_key() {
        let session = session();
        let generation = session.generation();
        session.install(map("a", generation, 10)).unwrap();
        session.install(map("b", generation, 10)).unwrap();
        let a = session.resolve_asset("a", "hero").unwrap();
        let b = session.resolve_asset("b", "hero").unwrap();
        assert_eq!(a.content_hash, b.content_hash, "same asset id and bytes");
        assert_ne!(a.cache_key, b.cache_key, "the scene is part of the key");
        assert_eq!(a.generation, generation);
        assert_eq!(
            session.resolve_asset("a", "ghost").unwrap_err().code(),
            "resource_unknown"
        );
        assert_eq!(
            session.resolve_asset("c", "hero").unwrap_err().code(),
            "scene_resources_not_loaded"
        );
    }

    #[test]
    fn the_cache_key_changes_with_generation_revision_and_content() {
        let session = session();
        let first = map("a", session.generation(), 10);
        let entry = first.entries[0].clone();
        let key = first.cache_key(&entry);
        let mut moved = first.clone();
        moved.generation = Generation(first.generation.0 + 1);
        assert_ne!(key, moved.cache_key(&entry));
        let mut revised = first.clone();
        revised.bundle_revision = "r2".into();
        assert_ne!(key, revised.cache_key(&entry));
        let mut rehashed = entry.clone();
        rehashed.content_hash = "b".repeat(64);
        assert_ne!(key, first.cache_key(&rehashed));
        let mut other_project = first.clone();
        other_project.project_id = "p2".into();
        assert_ne!(key, other_project.cache_key(&entry));
    }

    #[test]
    fn invalidating_forgets_everything_and_refuses_a_late_install() {
        let session = session();
        let old = session.generation();
        session.install(map("a", old, 10)).unwrap();
        let current = session.invalidate(Invalidation::ContextLoss);
        assert_ne!(old, current);
        assert!(session.loaded_scenes().is_empty());
        assert_eq!(
            session.resolve_asset("a", "hero").unwrap_err().code(),
            "scene_resources_not_loaded"
        );
        // A job started before the invalidation finishes afterwards: refused, nothing changes.
        let error = session.install(map("a", old, 10)).unwrap_err();
        assert!(matches!(error, ResolveError::Stale { .. }));
        assert!(session.loaded_scenes().is_empty());
        assert_eq!(session.status()["lastInvalidation"], "context_loss");
    }

    #[test]
    fn switching_the_project_invalidates_and_a_map_of_another_project_is_refused() {
        let session = session();
        let generation = session.generation();
        session.install(map("a", generation, 10)).unwrap();
        let next = session.observe_project("p2");
        assert_ne!(next, generation);
        assert!(session.loaded_scenes().is_empty());
        let mut foreign = map("a", next, 10);
        foreign.project_id = "p1".into();
        assert!(session.install(foreign).is_err());
        assert_eq!(
            session.observe_project("p2"),
            next,
            "same project: no change"
        );
    }

    #[test]
    fn the_budget_refuses_without_changing_what_is_installed() {
        let session = session();
        let generation = session.generation();
        session.install(map("a", generation, 60)).unwrap();
        let error = session.install(map("b", generation, 60)).unwrap_err();
        assert!(matches!(
            error,
            ResolveError::BudgetExceeded {
                needed: 120,
                budget: 100
            }
        ));
        assert_eq!(session.loaded_scenes(), ["a"]);
        assert_eq!(session.held_bytes(), 60);
        // Replacing the same scene counts its new size only, and releasing frees the budget.
        session.install(map("a", generation, 90)).unwrap();
        assert_eq!(session.held_bytes(), 90);
        assert!(session.release("a"));
        assert!(!session.release("a"));
        assert_eq!(session.held_bytes(), 0);
        session.install(map("b", generation, 60)).unwrap();
    }
}
