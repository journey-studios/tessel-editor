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
use super::scene_fx::{FxView, LOCALE, no_scene};
use super::{Source, Workstation};
use crate::state::CommandError;

/// `handles` on a vignette clip node shows or hides its 2D handles on the monitor.
pub(super) const HANDLES_ACTION: &str = "handles";

/// Why `clip` has no handles on the monitor, in the words of the core (`None` when it has them):
/// ONE reading of `FxClip::handles_refusal` for the action and the button.
fn no_handles_text(clip: &FxClip) -> Option<&'static str> {
    clip.handles_refusal().map(|refusal| refusal.text(LOCALE))
}

/// The `handles` entry of a clip node's actions. Only a vignette on the output has handles;
/// another effect, or a clip laid on a shot, lists the action disabled with the core's reason,
/// never as a callback that does nothing. The label is static: whether they are on is
/// `state.handles`, and the button already says it.
pub(super) fn handles_action(clip: &FxClip) -> Value {
    let reason = no_handles_text(clip);
    json!({
        "action": HANDLES_ACTION, "label": "Alças no monitor",
        "enabled": reason.is_none(), "disabled_reason": reason,
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
    /// (`fx_has_no_handles`, or `scope_not_supported` on a shot) and empties the entity
    /// selection (one target at a time).
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

    /// The header button of a clip: lit while its handles are on the monitor, disabled with the
    /// core's reason for a clip that has none (another effect, or a clip laid on a shot).
    pub(super) fn fx_handles_button(
        &self,
        view: &FxView,
        clip: &FxClip,
        workstation: WeakEntity<Self>,
    ) -> Div {
        let id = clip.id.clone();
        let on = self.fx_handles_on(&view.scene, &id);
        let button_id = SharedString::from(format!("fx-handles:{id}"));
        let button = if let Some(reason) = no_handles_text(clip) {
            disabled_tool_button(
                button_id,
                IconName::Frame,
                &format!("Alças de {id} no monitor"),
                reason,
            )
        } else {
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
        };
        div()
            .relative()
            .child(self.geometry.probe(format!("control:fx-handles:{id}")))
            .child(button)
    }
}
