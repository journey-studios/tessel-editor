//! The handles of the vignette in the effect inspector (F061, `docs/features/fx-handles.md`): the
//! node action, the toggle and the header button. Nothing here owns the target: it is the core's
//! `scene.fx.select`, and the button, the node action and a script's command are one command.

use gpui_kit::assets::IconName;
use gpui_kit::component::Selectable;
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx::FxClip;
use tessel_editor_core::scene_project::SceneCommand;

use super::controls::{disabled_tool_button, tool_button};
use super::scene_fx::{FxView, no_scene};
use super::{Source, Workstation};
use crate::state::CommandError;

/// `handles` on a vignette clip node shows or hides its 2D handles on the monitor.
pub(super) const HANDLES_ACTION: &str = "handles";
const NO_HANDLES: &str = "Só a vinheta tem alças no monitor";

/// The `handles` entry of a clip node's actions. Only a vignette has handles; another effect lists
/// the action disabled with its reason, never as a callback that does nothing. The label is
/// static: whether they are on is `state.handles`, and the button already says it.
pub(super) fn handles_action(clip: &FxClip) -> Value {
    let has_handles = clip.has_handles();
    json!({
        "action": HANDLES_ACTION, "label": "Alças no monitor",
        "enabled": has_handles, "disabled_reason": (!has_handles).then_some(NO_HANDLES),
    })
}

impl Workstation {
    /// Whether the monitor draws the handles of `clip` now: the core's session target, not a flag
    /// of the panel.
    pub(super) fn fx_handles_on(&self, scene: &str, clip: &str) -> bool {
        self.state
            .scene_interaction
            .fx_target()
            .is_some_and(|target| target.scene == scene && target.clip == clip)
    }

    /// Shows the handles of `clip` on the monitor, or hides them when they are shown: ONE
    /// `scene.fx.select`, the command a script sends. The core refuses a clip that has none
    /// (`fx_has_no_handles`) and empties the entity selection (one target at a time).
    pub(super) fn fx_handles_toggle(
        &mut self,
        clip: &str,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let view = self.fx_view().ok_or_else(no_scene)?;
        self.fx_known(&view, clip)?;
        let next = (!self.fx_handles_on(&view.scene, clip)).then(|| clip.to_string());
        self.scene_execute(
            SceneCommand::FxSelect {
                scene: view.scene,
                clip: next,
            },
            source,
            cx,
        )
    }

    /// The header button of a clip: lit while its handles are on the monitor, disabled with its
    /// reason for an effect that has none.
    pub(super) fn fx_handles_button(
        &self,
        view: &FxView,
        clip: &FxClip,
        workstation: WeakEntity<Self>,
    ) -> Div {
        let id = clip.id.clone();
        let on = self.fx_handles_on(&view.scene, &id);
        let button_id = SharedString::from(format!("fx-handles:{id}"));
        let button = if clip.has_handles() {
            let label = if on {
                format!("Ocultar alças de {id} no monitor")
            } else {
                format!("Mostrar alças de {id} no monitor")
            };
            let clip_id = id.clone();
            tool_button(button_id, IconName::Frame, &label)
                .selected(on)
                .on_click(move |_, _, cx| {
                    let clip_id = clip_id.clone();
                    let _ = workstation.update(cx, |this, cx| {
                        let _ = this.fx_handles_toggle(&clip_id, Source::Gesture, cx);
                    });
                })
        } else {
            disabled_tool_button(
                button_id,
                IconName::Frame,
                &format!("Alças de {id} no monitor"),
                NO_HANDLES,
            )
        };
        div()
            .relative()
            .child(self.geometry.probe(format!("control:fx-handles:{id}")))
            .child(button)
    }
}
