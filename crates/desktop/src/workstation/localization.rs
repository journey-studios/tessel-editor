//! Native language picker and review controls over the shared core.
use gpui_kit::component::button::Button;
use gpui_kit::component::h_flex;
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::*;
use serde_json::Value;
use tessel_editor_core::localization::Edit;

use super::flows::FlowKind;
use super::{Tokens, Workstation};
use crate::state::{Command, ProjectStamp};

pub(super) struct LanguageControls {
    select: Entity<SelectState<SearchableVec<String>>>,
    choices: Vec<(String, String)>,
    stamp: Option<ProjectStamp>,
    review: Value,
}

impl LanguageControls {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Workstation>) -> Self {
        let select = cx
            .new(|cx| SelectState::new(SearchableVec::new(Vec::<String>::new()), None, window, cx));
        cx.subscribe(
            &select,
            |this, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                let SelectEvent::Confirm(Some(label)) = event else {
                    return;
                };
                if let Some((code, _)) = this
                    .language_controls
                    .choices
                    .iter()
                    .find(|(_, choice)| choice == label)
                {
                    this.dispatch(Command::Localization(Edit::Select(code.clone())), cx);
                }
            },
        )
        .detach();
        Self {
            select,
            choices: Vec::new(),
            stamp: None,
            review: Value::Null,
        }
    }
}

fn caption(review: &Value) -> String {
    match review["status"].as_str() {
        Some("source") => "Base".into(),
        Some("approved") => "Aprovado".into(),
        Some("stale") => "Revisão vencida".into(),
        Some("incomplete") => {
            let count = review["missing"].as_array().map_or(0, Vec::len);
            format!("Pendências ({count})")
        }
        Some("conflict") => "Conflito".into(),
        _ => "A revisar".into(),
    }
}

impl Workstation {
    pub(super) fn render_language_controls(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let controls = &mut self.language_controls;
        if !controls
            .stamp
            .as_ref()
            .is_some_and(|stamp| stamp.is_current(&self.state.project))
        {
            controls.stamp = Some(self.state.project.stamp());
            controls.review = self.state.localization_status();
            let choices: Vec<_> = controls.review["languages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|review| {
                    let code = review["locale"].as_str()?;
                    Some((code.to_string(), format!("{code} · {}", caption(review))))
                })
                .collect();
            if choices != controls.choices {
                let labels: Vec<_> = choices.iter().map(|(_, label)| label.clone()).collect();
                controls.select.update(cx, |select, cx| {
                    select.set_items(SearchableVec::new(labels), window, cx)
                });
                controls.choices = choices;
            }
        }
        let active = self.state.active_language().unwrap_or_default();
        let selected = controls.choices.iter().find(|(code, _)| code == active);
        if let Some((_, label)) = selected
            && controls.select.read(cx).selected_value() != Some(label)
        {
            controls.select.update(cx, |select, cx| {
                select.set_selected_value(label, window, cx)
            });
        }
        let edit = self.state.is_localized_view();
        let review = controls.review["languages"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|review| review["locale"] == active);
        let can_approve = review.is_some_and(|review| {
            ["draft", "stale"].contains(&review["status"].as_str().unwrap_or_default())
        });
        let enabled = !active.is_empty();
        let add_label = if enabled {
            "+ Idioma"
        } else {
            "Ativar localização"
        };
        h_flex()
            .flex_shrink_0()
            .h(px(34.))
            .px_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(t.border)
            .bg(t.raised)
            .children(enabled.then(|| {
                div().w(px(160.)).child(
                    Select::new(&self.language_controls.select)
                        .id("timeline-language")
                        .small()
                        .accessibility_label("Idioma do projeto"),
                )
            }))
            .child(
                Button::new("language-add")
                    .small()
                    .label(add_label)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let _ = this.open_flow(FlowKind::AddLanguage, window, cx);
                    })),
            )
            .children(edit.then(|| {
                Button::new("language-overrides")
                    .small()
                    .label("Ajustes")
                    .on_click(cx.listener(|this, _, window, cx| {
                        let _ = this.open_flow(FlowKind::LanguageOverrides, window, cx);
                    }))
            }))
            .children(edit.then(|| {
                Button::new("language-approve")
                    .small()
                    .label("Aprovar idioma")
                    .disabled(!can_approve)
                    .on_click(super::controls::click(
                        cx,
                        Command::Localization(Edit::Approve(active.to_string())),
                    ))
            }))
    }
}
