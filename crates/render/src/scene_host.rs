//! Headless extension: ordinary media jobs, directed frames (`scene.render`) and directed-scene
//! video export (`scene.export`) in one programmable session.
use crate::scene_export::scene_renderer_factory;
use crate::{ProcessTransport, RuntimeConfig, SceneRenderer};
use serde_json::{Value, json};
use tessel_editor_core::headless::{Extension, backend_error};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::{CommandError, EditorState};
use tessel_editor_media::host::MediaHost;

pub struct SceneHost {
    pub media: MediaHost,
    config: RuntimeConfig,
    renderer: Option<SceneRenderer<ProcessTransport>>,
    project: Option<String>,
}
impl SceneHost {
    /// Also gives `media` the renderer `scene.export` drives: each export builds its own from
    /// `config`, apart from this session's `scene.render` renderer.
    pub fn new(media: MediaHost, config: RuntimeConfig) -> Self {
        let media = media.with_scene_renderer(scene_renderer_factory(config.clone()));
        Self {
            media,
            config,
            renderer: None,
            project: None,
        }
    }
    /// With `resources_root` the media host verifies and approves the files first; without it the
    /// session must already hold an approved map (`scene.resources.resolve`).
    fn approve_resources(
        &mut self,
        state: &mut EditorState,
        request: &Value,
    ) -> Result<(), CommandError> {
        // The media host verifies bytes on its job path before any resource enters the renderer.
        let Some(root) = request.get("resources_root") else {
            return Ok(());
        };
        let load =
            json!({ "op": "scene.resources.resolve", "scene": request["scene"], "base": root });
        let response = self.media.execute(state, None, &load).ok_or_else(|| {
            CommandError::new("backend_unavailable", "Scene resource loader unavailable")
        })??;
        if response["state"] == "queued" || response["state"] == "running" {
            return Err(CommandError::new(
                "resources_pending",
                "Wait for the scene resource job before rendering",
            ));
        }
        Ok(())
    }

    fn frame(&mut self, state: &mut EditorState, request: &Value) -> Result<Value, CommandError> {
        self.approve_resources(state, request)?;
        let (plan, map, options) =
            crate::scene_operation::prepare(state, request, &self.media.scene_resources())?;
        if self.project.as_deref() != Some(&state.project.id) {
            self.renderer = None;
            self.project = Some(state.project.id.clone());
        }
        if self.renderer.is_none() {
            let transport = ProcessTransport::new(self.config.clone());
            self.renderer = Some(SceneRenderer::new(
                tessel_editor_core::bridge::BridgeBackend::new(
                    "three-directed-headless",
                    transport,
                    crate::HANDSHAKE_TIMEOUT,
                ),
            ));
        }
        let renderer = self.renderer.as_mut().ok_or_else(|| {
            CommandError::new("backend_unavailable", "Scene renderer unavailable")
        })?;
        let result = renderer
            .render(&plan, &map, &options)
            .map_err(backend_error)?;
        Ok(json!({ "action": "scene.render", "scene": request["scene"],
            "plan": plan.to_json(), "frame": result.to_json(), "generation": map.generation() }))
    }
}
impl Extension for SceneHost {
    fn execute(
        &mut self,
        state: &mut EditorState,
        renderer: Option<&mut dyn Renderer>,
        request: &Value,
    ) -> Option<Result<Value, CommandError>> {
        if request["op"] == "scene.render" {
            return Some(self.frame(state, request));
        }
        if request["op"] == "scene.export" || request["op"] == "scene.export.preflight" {
            // Same optional convenience as `scene.render`; the export itself is the media host's.
            if let Err(error) = self.approve_resources(state, request) {
                return Some(Err(error));
            }
        }
        self.media.execute(state, renderer, request)
    }
}
