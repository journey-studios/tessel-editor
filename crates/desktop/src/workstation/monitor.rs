//! The single monitor. It shows the live viewport (the sequence at the playhead, or one
//! source) and the transport: seeking, and play/pause (the clock and the device audio live in
//! `playback.rs`).

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Selectable, Sizable, h_flex};
use gpui_kit::*;
use tessel_editor_core::plan::PreviewIntent;
use tessel_editor_core::scene_interaction::{GizmoCommand, GizmoSpace, GizmoTool};
use tessel_editor_core::scene_project::SceneCommand;

use super::Workstation;
use super::controls::{click, tool_button};
use super::tokens::Tokens;
use super::viewport::{ViewportContent, render_viewport};
use crate::state::timeline::ClipKind;
use crate::state::{Command, Raw, Subject};
use tessel_editor_core::time::Ratio;

fn seek(frame: i64) -> Command {
    Command::SetPlayhead {
        frame: Raw::Number(frame as f64),
    }
}

impl Workstation {
    pub(super) fn render_monitor(
        &mut self,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let state = &self.state;
        let playhead = state.timeline.playhead;
        let media = state.subject_media();
        let title_card = (state.subject == Subject::Clip)
            .then(|| state.selected_clip())
            .flatten()
            .filter(|clip| clip.kind == ClipKind::Title);

        let heading = match (media, title_card) {
            (Some(item), _) => format!("{} · {}", item.code, item.name),
            (None, Some(clip)) => clip.name.to_string(),
            (None, None) => "Sem seleção".to_string(),
        };

        let content = match (self.viewport.blocking_error(), &self.viewport_view) {
            (Some(error), _) => ViewportContent::Placeholder {
                icon: IconName::Film,
                text: error.message.clone(),
            },
            (None, Some(view)) => ViewportContent::Web(view.clone()),
            (None, None) => ViewportContent::Placeholder {
                icon: IconName::Film,
                text: "O visualizador não pôde ser criado".to_string(),
            },
        };
        let stage = render_viewport(t, &self.geometry, content);
        let headline = self.viewport.headline();
        // A directed scene is attached but nothing can draw it yet: say so where the picture
        // would be, instead of leaving the monitor silent about it (see `scene_monitor.rs`).
        let scene_badge = self.scene_attached().then(|| self.scene_monitor().badge());
        let failed = self.viewport.failure().is_some() || self.viewport.blocking_error().is_some();
        let sequence_preview = matches!(state.preview, PreviewIntent::Sequence);
        let length = state.timeline.length;
        let (start_code, end_code) = (
            state.timeline.timecode(playhead),
            state.timeline.timecode(length),
        );

        let preview_modes = h_flex()
            .items_center()
            .gap(px(2.))
            .child(
                Button::new("monitor-preview-sequence")
                    .ghost()
                    .xsmall()
                    .label("Sequência")
                    .selected(sequence_preview)
                    .on_click(click(
                        cx,
                        Command::SetPreview {
                            intent: PreviewIntent::Sequence,
                        },
                    )),
            )
            .child({
                let source = Button::new("monitor-preview-source")
                    .ghost()
                    .xsmall()
                    .label("Fonte")
                    .selected(!sequence_preview);
                match state.previewable_media() {
                    Some(item) => source
                        .on_click(click(
                            cx,
                            Command::SetPreview {
                                intent: PreviewIntent::Source {
                                    asset: item.id,
                                    time: Ratio::ZERO,
                                },
                            },
                        ))
                        .tooltip("Mostrar a mídia selecionada, sem a sequência".to_string()),
                    None => source
                        .disabled(true)
                        .tooltip("Selecione uma mídia com arquivo para ver a fonte".to_string()),
                }
            });

        // BL015.1: the gizmo's tool and the space of its rotation axes, a toolbar over the
        // picture's header (a GPUI control: the rings themselves are drawn in the page). Only
        // with a scene attached; each press is the same `scene.gizmo.tool` a script sends.
        let gizmo_tools = self.scene_attached().then(|| {
            let (tool, space) = (
                state.scene_interaction.tool(),
                state.scene_interaction.space(),
            );
            let geometry = &self.geometry;
            // Each button has a probe (`control:<id>`), the semantic address of its bounds.
            let mut button = |id: &'static str,
                              label: &'static str,
                              tip: &'static str,
                              selected: bool,
                              tool: Option<GizmoTool>,
                              space: Option<GizmoSpace>| {
                div()
                    .relative()
                    .child(geometry.probe(format!("control:{id}")))
                    .child(
                        Button::new(id)
                            .ghost()
                            .xsmall()
                            .label(label)
                            .selected(selected)
                            .tooltip(tip.to_string())
                            .on_click(click(
                                cx,
                                Command::Scene(SceneCommand::Gizmo(GizmoCommand::Tool {
                                    tool,
                                    space,
                                })),
                            )),
                    )
            };
            let mut group = h_flex()
                .items_center()
                .gap(px(2.))
                .child(button(
                    "monitor-gizmo-translate",
                    "Mover",
                    "Setas de translação do nó selecionado",
                    tool == GizmoTool::Translate,
                    Some(GizmoTool::Translate),
                    None,
                ))
                .child(button(
                    "monitor-gizmo-rotate",
                    "Girar",
                    "Anéis de rotação do nó selecionado",
                    tool == GizmoTool::Rotate,
                    Some(GizmoTool::Rotate),
                    None,
                ));
            if tool == GizmoTool::Rotate {
                group = group
                    .child(button(
                        "monitor-gizmo-world",
                        "Mundo",
                        "Girar em torno dos eixos da cena",
                        space == GizmoSpace::World,
                        None,
                        Some(GizmoSpace::World),
                    ))
                    .child(button(
                        "monitor-gizmo-local",
                        "Local",
                        "Girar em torno dos eixos do próprio nó",
                        space == GizmoSpace::Local,
                        None,
                        Some(GizmoSpace::Local),
                    ));
            }
            group
        });

        let playing = self.playback.is_playing();
        let step = |delta: i64| seek(playhead + delta);
        let transport = h_flex()
            .items_center()
            .gap(px(2.))
            .child(
                tool_button("monitor-go-start", IconName::SkipBack, "Ir para o início")
                    .on_click(click(cx, seek(0))),
            )
            .child(
                tool_button("monitor-step-back", IconName::StepBack, "Voltar um quadro")
                    .on_click(click(cx, step(-1))),
            )
            .child(
                tool_button(
                    "monitor-play",
                    if playing {
                        IconName::Pause
                    } else {
                        IconName::Play
                    },
                    if playing { "Pausar" } else { "Reproduzir" },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    let _ = this.playback_toggle(cx);
                })),
            )
            .child(
                tool_button(
                    "monitor-step-forward",
                    IconName::StepForward,
                    "Avançar um quadro",
                )
                .on_click(click(cx, step(1))),
            )
            .child(
                tool_button("monitor-go-end", IconName::SkipForward, "Ir para o fim")
                    .on_click(click(cx, seek(length))),
            );

        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.))
            .bg(t.panel)
            .child(self.geometry.probe("region:monitor"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .h(px(28.))
                    .px(px(10.))
                    .text_size(px(11.))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(div().text_color(t.text).child(heading)),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(8.))
                            .children(gizmo_tools)
                            .child(preview_modes)
                            .children(scene_badge.map(|badge| {
                                div()
                                    .id("monitor-scene-badge")
                                    .text_size(px(10.))
                                    .text_color(t.faint)
                                    .child(badge)
                            }))
                            .child(
                                div()
                                    .id("monitor-status")
                                    // A long failure message must not push the mode buttons out of the header.
                                    .max_w(px(260.))
                                    .truncate()
                                    .text_size(px(10.))
                                    .text_color(if failed { t.text } else { t.faint })
                                    .child(headline),
                            ),
                    ),
            )
            .child(stage)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .h(px(34.))
                    .px(px(10.))
                    .text_size(px(11.))
                    .child(div().w(px(110.)).text_color(t.text).child(start_code))
                    .child(transport)
                    .child(
                        div()
                            .w(px(110.))
                            .flex()
                            .justify_end()
                            .text_color(t.dim)
                            .child(end_code),
                    ),
            )
    }
}
