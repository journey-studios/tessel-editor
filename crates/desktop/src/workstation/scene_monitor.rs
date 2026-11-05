use serde_json::{Value, json};

use super::Workstation;
use crate::state::CommandError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneMonitor {
    Available,
    /// No renderer is integrated. `reason` is shown to the user and exposed as `disabled_reason`.
    Unavailable {
        reason: &'static str,
    },
}

/// Why the directed scene cannot be shown yet. One sentence, pt-BR like the rest of the window.
#[allow(dead_code)]
pub const UNAVAILABLE_REASON: &str =
    "O monitor da cena dirigida ainda não está integrado; a edição das propriedades já funciona";

impl SceneMonitor {
    pub fn is_available(&self) -> bool {
        match self {
            Self::Unavailable { .. } => false,
            Self::Available => true,
        }
    }

    /// The short label of the header badge and of the monitor note in the inspector.
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "Cena dirigida: indisponível",
            Self::Available => "Cena dirigida",
        }
    }

    pub fn reason(&self) -> &'static str {
        match self {
            Self::Unavailable { reason } => reason,
            Self::Available => "Recursos verificados; monitor Three.js disponível",
        }
    }

    /// Facts for the `state` of `scene.panel`.
    pub fn report(&self) -> Value {
        json!({
            "available": self.is_available(),
            "reason": (!self.is_available()).then(|| self.reason()),
        })
    }

    /// The `monitor.scene` semantic node, a child of `region:monitor`.
    pub fn node(&self) -> Value {
        let mut node = json!({
            "id": "monitor.scene", "kind": "viewport", "label": "Monitor da cena dirigida",
            "parent": "region:monitor", "children": [], "enabled": self.is_available(),
            "visible": true, "fields": [], "actions": if self.is_available() { vec!["show"] } else { vec![] },
            "state": self.report(),
        });
        if !self.is_available() {
            node["disabled_reason"] = json!(self.reason());
        }
        node
    }

    /// What `invoke monitor.scene` answers while nothing can be shown: a typed refusal.
    pub fn show_error(&self) -> CommandError {
        CommandError::disabled(self.reason()).target("monitor.scene")
    }
}

impl Workstation {
    pub(super) fn scene_monitor(&self) -> SceneMonitor {
        if !self.viewport.is_ready() {
            return SceneMonitor::Unavailable {
                reason: "O monitor está iniciando ou indisponível",
            };
        }
        let Some(scene) = self.scene_active_id() else {
            return SceneMonitor::Unavailable {
                reason: "Anexe e selecione uma cena",
            };
        };
        let session = self.workflow.scene_resources();
        let Some(map) = session.get(&scene) else {
            return SceneMonitor::Unavailable {
                reason: "Carregue os recursos com scene.resources.resolve antes de exibir a cena",
            };
        };
        if map.project_id != self.state.project.id {
            return SceneMonitor::Unavailable {
                reason: "Os recursos pertencem a outro projeto",
            };
        }
        SceneMonitor::Available
    }

    pub(super) fn show_directed_scene(
        &mut self,
        scene: &str,
        cx: &mut gpui_kit::Context<Self>,
    ) -> Result<Value, CommandError> {
        let request = serde_json::json!({ "scene": scene, "frame": self.state.timeline.playhead,
            "width": 960, "height": 540 });
        self.viewport.render_scene(
            &mut self.state,
            &self.workflow.scene_resources(),
            &request,
            false,
        )?;
        self.viewport_sync(cx);
        Ok(serde_json::json!({ "action": "monitor.show_scene", "scene": scene }))
    }

    /// Whether any directed scene is attached, so the monitor header says what is missing.
    pub(super) fn scene_attached(&self) -> bool {
        !self.state.scenes.is_empty()
    }
}
