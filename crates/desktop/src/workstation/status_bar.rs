//! Bottom status bar. Every value is derived from `EditorState`.

use gpui_kit::assets::IconName;
use gpui_kit::*;

use super::controls::glyph;
use super::tokens::Tokens;
use super::{Source, Workstation};

impl Workstation {
    pub(super) fn render_status(
        &mut self,
        t: &Tokens,
        perf_strip: Option<AnyElement>,
    ) -> impl IntoElement + use<> {
        let selection = match (self.state.selected_clip(), self.state.subject_media()) {
            (Some(clip), _) => format!("Clipe: {}", clip.name),
            (None, Some(item)) => format!("Mídia: {}", item.name),
            (None, None) => "Nada selecionado".to_string(),
        };
        let activity = self.activity.as_ref().map(|activity| {
            let origin = match activity.source {
                Source::Cli => "CLI",
                Source::Gesture => "UI",
            };
            let (icon, color) = if activity.error {
                (IconName::CircleAlert, t.bad)
            } else {
                (IconName::Check, t.dim)
            };
            // One line that may be cut (ellipsis), never wrapped and never rigid: a failed
            // command writes its whole message here and the bar must still end inside the
            // window with the save state whole. It shares the overflow with the debug meter's
            // strip (same shrink factor, so each is cut in proportion to its width and neither
            // is removed); the text, not the icon, is what gets cut (F049).
            div()
                .debug_selector(|| "status-activity".to_string())
                .flex_shrink(1.)
                .min_w(px(0.))
                .flex()
                .items_center()
                .gap(px(4.))
                .text_color(color)
                .child(div().flex_shrink_0().child(glyph(icon, 11.0, color)))
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .child(format!("{origin} · {}", activity.text)),
                )
        });
        let jobs = self.workflow.jobs_line().map(|line| {
            // The same rule as the activity: cut with an ellipsis, not rigid.
            div()
                .debug_selector(|| "status-jobs".to_string())
                .flex_shrink(1.)
                .min_w(px(0.))
                .flex()
                .items_center()
                .gap(px(4.))
                .text_color(t.text)
                .child(
                    div()
                        .flex_shrink_0()
                        .child(glyph(IconName::Loader, 11.0, t.dim)),
                )
                .child(div().min_w(px(0.)).truncate().child(line))
        });
        // The project's save state is read from the model, never assumed.
        let saved = match (&self.state.project.document, self.state.project.is_dirty()) {
            (None, _) => "Projeto não salvo".to_string(),
            (Some(path), true) => format!("{} · alterações não salvas", file_label(path)),
            (Some(path), false) => format!("{} · salvo", file_label(path)),
        };
        div()
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .flex_shrink_0()
            .h(px(24.))
            .px(px(10.))
            .gap(px(12.))
            .bg(t.raised)
            .border_t_1()
            .border_color(t.border)
            .text_size(px(10.))
            .text_color(t.dim)
            .child(self.geometry.probe("region:status"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .min_w(px(0.))
                    .child(div().truncate().child(self.state.status_summary()))
                    .child(div().truncate().text_color(t.text).child(selection)),
            )
            .child(
                div()
                    .debug_selector(|| "status-right".to_string())
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .min_w(px(0.))
                    .children(perf_strip)
                    .children(jobs)
                    .children(activity)
                    .child(
                        div()
                            .debug_selector(|| "status-saved".to_string())
                            .flex_shrink_0()
                            .truncate()
                            .text_color(t.faint)
                            .child(saved),
                    ),
            )
    }
}

fn file_label(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}
