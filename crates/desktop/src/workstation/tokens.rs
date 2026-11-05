//! Workstation colours, resolved from the Kit's active theme. No hex
//! literals: switching the Kit theme mode re-tints everything.

use gpui_kit::component::ActiveTheme;
use gpui_kit::{App, Hsla};

#[derive(Clone, Copy)]
pub struct Tokens {
    /// Window background, seen between panels.
    pub app: Hsla,
    pub panel: Hsla,
    /// Headers, toolbars and raised rows.
    pub raised: Hsla,
    pub field: Hsla,
    pub hover: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub dim: Hsla,
    pub faint: Hsla,
    pub accent: Hsla,
    pub lane: Hsla,
    pub lane_alt: Hsla,
    pub video_fill: Hsla,
    pub video_border: Hsla,
    pub video_label: Hsla,
    pub audio_fill: Hsla,
    pub audio_border: Hsla,
    pub audio_wave: Hsla,
    pub title_fill: Hsla,
    pub title_border: Hsla,
    pub selected: Hsla,
    pub playhead: Hsla,
    pub ok: Hsla,
    pub bad: Hsla,
}

impl Tokens {
    pub fn new(cx: &App) -> Self {
        let colors = cx.theme().colors;
        Self {
            app: colors.background,
            panel: colors.sidebar,
            raised: colors.secondary,
            field: colors.input.opacity(0.35),
            hover: colors.secondary_hover,
            border: colors.border,
            border_strong: colors.input,
            text: colors.foreground,
            dim: colors.muted_foreground,
            faint: colors.muted_foreground.opacity(0.65),
            accent: colors.primary,
            lane: colors.background,
            lane_alt: colors.muted.opacity(0.45),
            video_fill: colors.blue.opacity(0.4),
            video_border: colors.blue,
            video_label: colors.foreground,
            audio_fill: colors.green.opacity(0.35),
            audio_border: colors.green,
            audio_wave: colors.green_light,
            title_fill: colors.yellow.opacity(0.35),
            title_border: colors.yellow,
            selected: colors.yellow_light,
            playhead: colors.red,
            ok: colors.success,
            bad: colors.danger,
        }
    }
}
