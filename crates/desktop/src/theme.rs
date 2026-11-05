//! Theme switching on top of the Kit's `Theme`. Every component and every
//! workstation token reads the active Kit colours, so changing the mode here
//! re-themes the whole window.

use gpui_kit::App;
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};

pub fn apply(cx: &mut App, dark: bool) {
    let mode = if dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    Theme::change(mode, None, cx);
    cx.refresh_windows();
}

pub fn scheme_name(cx: &App) -> &'static str {
    cx.theme().mode.name()
}

pub fn is_dark(cx: &App) -> bool {
    cx.theme().mode.is_dark()
}

pub fn toggle(cx: &mut App) {
    let dark = is_dark(cx);
    apply(cx, !dark);
}

#[cfg(test)]
mod tests {
    use super::*;

    // `gpui_kit::*` also exports GPUI's `test` attribute, which would shadow this one.
    #[gpui_kit::test]
    fn applying_a_scheme_changes_the_active_kit_theme(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            apply(cx, true);
            assert_eq!(scheme_name(cx), "dark");
            apply(cx, false);
            assert_eq!(scheme_name(cx), "light");
            assert!(!is_dark(cx));
        });
    }

    #[gpui_kit::test]
    fn toggling_flips_the_scheme_both_ways(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            apply(cx, true);
            toggle(cx);
            assert_eq!(scheme_name(cx), "light");
            toggle(cx);
            assert_eq!(scheme_name(cx), "dark");
        });
    }
}
