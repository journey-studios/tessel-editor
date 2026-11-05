use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

use crate::header::Header;
use crate::workstation::Workstation;

/// Window content: the unified header above the dockable workstation.
pub struct EditorShell {
    header: Entity<Header>,
    pub workstation: Entity<Workstation>,
}

impl EditorShell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workstation = cx.new(|cx| Workstation::new(window, cx));
        let header = cx.new(|cx| Header::new(workstation.clone(), cx));
        // Escape cancels a drag only while the workstation holds focus.
        workstation.update(cx, |workstation, cx| workstation.focus_root(window, cx));
        Self {
            header,
            workstation,
        }
    }
}

impl Render for EditorShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.background)
            .text_color(colors.foreground)
            .child(self.header.clone())
            .child(div().flex_1().min_h(px(0.)).child(self.workstation.clone()))
    }
}
