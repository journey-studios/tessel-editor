//! Library panel: project bins (Kit `Tree`), media search (Kit `Input`) and a
//! thumbnail grid.
//!
//! The Kit tree and input keep private copies of their data (expansion,
//! selection, text). They are re-synchronised from `EditorState` before every
//! paint, so a CLI `set folder:x expanded false` and a chevron click end in
//! the same state.

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tree::{TreeItem, tree};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::Workstation;
use super::controls::{glyph, keys};
use super::media::demo_image;
use super::tokens::Tokens;
use crate::state::Command;
use crate::state::library::{Folder, FolderIcon, Library, MediaItem, MediaKind};

/// Tree row height of the Kit's default list item.
const TREE_ROW: f32 = 28.0;
/// Tree ID of the "all media" pseudo-bin; real folder IDs are never `folder-all`.
const ALL_ID: &str = "folder-all";

fn folder_icon(folder: &Folder) -> IconName {
    match folder.icon {
        FolderIcon::Project if folder.expanded => IconName::FolderOpen,
        FolderIcon::Project => IconName::Folder,
        FolderIcon::Footage => IconName::Film,
        FolderIcon::Timelines => IconName::ChartGantt,
        FolderIcon::Audio => IconName::AudioLines,
    }
}

fn folder_item(library: &Library, folder: &Folder) -> TreeItem {
    TreeItem::new(folder.id.as_str(), folder.label.clone())
        .expanded(folder.expanded)
        .children(
            library
                .children(folder.id.as_str())
                .map(|child| folder_item(library, child)),
        )
}

/// What the tree must show, as IDs and labels plus expansion plus the selected row, so a
/// change in any of them is noticed with one comparison. The label is part of it because
/// every project's root bin has the same ID: opening another project renames it.
fn signature(library: &Library) -> Vec<(String, bool)> {
    let mut signature: Vec<(String, bool)> = library
        .folders
        .iter()
        .map(|folder| {
            (
                format!("{}\u{1}{}", folder.id, folder.label),
                folder.expanded,
            )
        })
        .collect();
    signature.push((
        format!(
            "selected:{}",
            library
                .selected_folder
                .map_or(ALL_ID, |folder| folder.as_str())
        ),
        true,
    ));
    signature
}

impl Workstation {
    /// Pushes the model into the Kit tree and search input when it changed.
    fn sync_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.state.library.search.clone();
        self.search.update(cx, |input, cx| {
            if input.value() != query.as_str() {
                // `set_value` does not emit `Change`, so this cannot loop.
                input.set_value(query, window, cx);
            }
        });

        let wanted = signature(&self.state.library);
        if wanted == self.library_signature {
            return;
        }
        let library = &self.state.library;
        let mut items = vec![TreeItem::new(ALL_ID, "Todas as mídias")];
        items.extend(
            library
                .folders
                .iter()
                .filter(|folder| folder.parent.is_none())
                .map(|folder| folder_item(library, folder)),
        );
        let selected = SharedString::from(
            library
                .selected_folder
                .map_or(ALL_ID, |folder| folder.as_str()),
        );
        self.library_tree.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            let row = tree.index_of(&selected);
            tree.set_selected_index(row, cx);
        });
        self.library_signature = wanted;
    }

    fn media_tile(
        &mut self,
        t: &Tokens,
        cx: &mut Context<Self>,
        item: &MediaItem,
    ) -> impl IntoElement + use<> {
        let selected = self.state.selected_media == Some(item.id);
        let hover = t.hover;
        let command = Command::SelectMedia {
            media: item.id.to_string(),
        };
        let thumb = div()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .h(px(60.))
            .rounded(px(3.))
            .overflow_hidden()
            .bg(t.field);
        // A splat or scene shows the frame the adapter drew of it, but only while that picture
        // is still the one of the asset as it is now (`EditorState::thumbnail` checks).
        let drawn = self.state.thumbnail(item.id.as_str()).cloned();
        let thumb = match (item.image, drawn) {
            (Some(image), _) => thumb.child(
                img(demo_image(image))
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            ),
            (None, Some(drawn)) => {
                let id = item.id.to_string();
                thumb.child(
                    img(std::path::PathBuf::from(drawn.path))
                        .debug_selector(move || format!("media-thumb-{id}"))
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                )
            }
            (None, None) => thumb.child(glyph(
                match item.kind {
                    MediaKind::Audio => IconName::AudioLines,
                    _ => IconName::ChartGantt,
                },
                18.0,
                t.dim,
            )),
        };
        div()
            .id(ElementId::Name(format!("media-{}", item.id).into()))
            .focusable()
            .flex()
            .flex_col()
            .gap(px(3.))
            .p(px(3.))
            .rounded(px(4.))
            .border_1()
            .border_color(if selected {
                t.accent
            } else {
                transparent_black()
            })
            .cursor_pointer()
            .when(selected, |tile| tile.bg(t.raised))
            .hover(move |style| style.bg(hover))
            .on_click(super::controls::click(cx, command.clone()))
            .on_key_down(keys(cx, move |key, _| {
                matches!(key, "enter" | "space").then(|| command.clone())
            }))
            .child(thumb)
            .child(
                div()
                    .truncate()
                    .text_size(px(10.))
                    .text_color(if selected { t.text } else { t.dim })
                    .child(format!("{} · {}", item.code, item.name)),
            )
            .child(
                div()
                    .truncate()
                    .text_size(px(9.))
                    .text_color(t.faint)
                    .child(item.meta.clone()),
            )
    }

    pub(super) fn render_library(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        self.sync_library(window, cx);

        // Per-row facts the tree closure needs; it cannot borrow the editor state.
        let meta: Rc<HashMap<String, (IconName, usize)>> = Rc::new(
            self.state
                .library
                .folders
                .iter()
                .map(|folder| {
                    (
                        folder.id.to_string(),
                        (folder_icon(folder), self.state.library.count(folder.id)),
                    )
                })
                .collect(),
        );
        let total = self.state.library.items.len();
        // One row per visible folder plus the "all media" row.
        let rows = self.state.library.visible_folders().len() + 1;
        let workstation = cx.weak_entity();
        let tokens = *t;
        let folders = tree(&self.library_tree, move |_, entry, selected, _, _| {
            let id = entry.item().id.to_string();
            let (icon, count) = meta.get(&id).copied().unwrap_or((IconName::Layers, total));
            let command = Command::SelectFolder {
                folder: (id != ALL_ID).then(|| id.clone()),
            };
            let workstation = workstation.clone();
            ListItem::new(SharedString::from(format!("folder-{id}")))
                .child(
                    h_flex()
                        .w_full()
                        .gap(px(6.))
                        .items_center()
                        .pl(px(entry.depth() as f32 * 12.))
                        .text_size(px(12.))
                        .text_color(if selected { tokens.text } else { tokens.dim })
                        .child(Icon::new(icon).size(px(14.)).text_color(tokens.dim))
                        .child(div().flex_1().truncate().child(entry.item().label.clone()))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(tokens.faint)
                                .child(count.to_string()),
                        ),
                )
                // Selecting runs first; the tree then expands or collapses a
                // folder on the same click and reports it as a `TreeEvent`.
                .on_click(move |_, _, cx| {
                    let _ = workstation.update(cx, |this, cx| {
                        this.dispatch(command.clone(), cx);
                    });
                })
        });

        let items: Vec<MediaItem> = self.state.library.results().into_iter().cloned().collect();
        let mut grid = div().grid().grid_cols(2).gap(px(4.)).w_full().p(px(6.));
        for item in &items {
            grid = grid.child(self.media_tile(t, cx, item));
        }
        let empty = items.is_empty().then(|| {
            div()
                .p(px(12.))
                .text_size(px(11.))
                .text_color(t.dim)
                .child("Nenhuma mídia encontrada nesta pasta")
        });
        let caption = self
            .state
            .selected_media
            .and_then(|id| self.state.library.item(id))
            .map_or("Nenhuma mídia selecionada".to_string(), |item| {
                format!("{} · {}", item.code, item.name)
            });

        v_flex()
            .relative()
            .size_full()
            .min_h(px(0.))
            .bg(t.panel)
            .child(self.geometry.probe("region:library"))
            .child(
                div().px(px(8.)).py(px(6.)).child(
                    Input::new(&self.search)
                        .small()
                        .cleanable(true)
                        .aria_label("Buscar mídia")
                        .prefix(Icon::new(IconName::Search).size(px(14.)).text_color(t.dim)),
                ),
            )
            .child(
                div()
                    .px(px(10.))
                    .pt(px(4.))
                    .pb(px(2.))
                    .text_size(px(9.))
                    .text_color(t.faint)
                    .child("PASTAS DO PROJETO"),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .h(px(rows as f32 * TREE_ROW))
                    .max_h(px(220.))
                    .px(px(4.))
                    .child(folders),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(10.))
                    .pt(px(10.))
                    .pb(px(2.))
                    .text_size(px(10.))
                    .text_color(t.dim)
                    .child(self.state.library.bin_label().to_string())
                    .child(
                        div()
                            .text_color(t.faint)
                            .child(format!("{} itens", items.len())),
                    ),
            )
            .child(
                div()
                    .id("media-scroll")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scrollbar()
                    .child(grid)
                    .children(empty),
            )
            .child(
                v_flex()
                    .gap(px(6.))
                    .p(px(8.))
                    .border_t_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(t.dim)
                            .child(caption),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_center()
                            .gap(px(6.))
                            .h(px(26.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(t.border)
                            .text_size(px(11.))
                            .text_color(t.faint)
                            .child(glyph(IconName::Plus, 12.0, t.faint))
                            .child("Inserir na timeline")
                            .opacity(0.7),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;

    #[test]
    fn the_tree_signature_changes_when_another_project_renames_the_same_root_bin() {
        let first = Library::empty("Aurora");
        let second = Library::empty("Basalt");
        assert_eq!(first.root().map(|f| f.id), second.root().map(|f| f.id));
        assert_ne!(signature(&first), signature(&second));
        assert_eq!(signature(&first), signature(&Library::empty("Aurora")));
    }
}
