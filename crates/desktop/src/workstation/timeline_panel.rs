//! Central timeline: toolbar, ruler with playhead, track headers, lanes, clips
//! with thumbnails and demonstrative waveforms, and the workspace mode bar.

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::h_flex;
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::Select;
use gpui_kit::component::slider::Slider;
use gpui_kit::component::{Disableable, Selectable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::Workstation;
use super::controls::{
    TabSpec, TabStyle, command_tabs, disabled_tool_button, glyph, keys, separator, tool_button,
};
use super::drag::{
    ClipDrag, DragGhost, NoGhost, Preview, ScrubDrag, TrackDrag, TrimDrag, scrub_frame,
};
use super::media::demo_image;
use super::tokens::Tokens;
use super::waveform::waveform;
use crate::state::split::{SplitAvailability, SplitBlock};
use crate::state::timeline::{Clip, ClipKind, Placement, Track, TrackFlag, TrackKind, TrimEdge};
use crate::state::{Command, Raw, WorkspaceTab};

const HEADER_WIDTH: f32 = 128.0;
const RULER_HEIGHT: f32 = 24.0;
const CLIP_INSET: f32 = 3.0;
const LABEL_HEIGHT: f32 = 14.0;
const FOOTER_HEIGHT: f32 = 12.0;
const TAIL_PADDING: f32 = 24.0;

const NO_TOOL: &str = "Ferramenta ainda não implementada";

/// The core's refusal (`SplitBlock::reason`, English, as `disabled_reason` in the tree) in the
/// words of the window's tooltips.
fn split_block_reason(block: SplitBlock) -> &'static str {
    match block {
        SplitBlock::NoSelection => "selecione um clipe",
        SplitBlock::PlayheadOutside => {
            "posicione o playhead dentro do clipe selecionado, depois do primeiro quadro"
        }
        SplitBlock::FadeCrossesCut => "uma entrada ou saída (fade) do clipe atravessa o playhead",
        SplitBlock::AudioSampleGrid => {
            "este áudio não pode ser cortado nesta taxa de quadros sem alterar o som"
        }
        SplitBlock::Refused => "o core recusa o corte aqui; use clip.split para ver o motivo",
    }
}

/// Frames between labelled ticks: the smallest candidate that leaves at least
/// 72 px between labels at the current zoom.
pub fn major_interval_frames(pixels_per_frame: f32, fps: i64) -> i64 {
    [1, 2, 5, 10, 15, 30]
        .iter()
        .map(|seconds| seconds * fps)
        .find(|frames| *frames as f32 * pixels_per_frame >= 72.0)
        .unwrap_or(60 * fps)
}

/// `(frame, labelled)` ticks across the sequence. Minor ticks mark each second
/// once they are at least 7 px apart.
pub fn ruler_ticks(pixels_per_frame: f32, fps: i64, length: i64) -> Vec<(i64, bool)> {
    let major = major_interval_frames(pixels_per_frame, fps);
    let show_minor = fps as f32 * pixels_per_frame >= 7.0;
    (0..=length / fps)
        .map(|second| second * fps)
        .filter_map(|frame| {
            if frame % major == 0 {
                Some((frame, true))
            } else {
                show_minor.then_some((frame, false))
            }
        })
        .collect()
}

impl Workstation {
    fn seek_command(frame: i64) -> Command {
        Command::SetPlayhead {
            frame: Raw::Number(frame as f64),
        }
    }

    fn render_timeline_toolbar(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let snap = self.state.timeline.snap;
        let zoom = self.state.timeline.zoom as f32;
        // `set_value` does not emit, so following the model cannot loop.
        let differs = (self.zoom_slider.read(cx).value().end() - zoom).abs() > 1e-4;
        if differs {
            self.zoom_slider
                .update(cx, |slider, cx| slider.set_value(zoom, window, cx));
        }
        self.sync_sequence_select(window, cx);
        let snap_button = tool_button(
            "timeline-snap",
            IconName::Magnet,
            "Alinhar às bordas (snap)",
        )
        .selected(snap)
        .on_click(super::controls::click(cx, Command::SetSnap { snap: !snap }));
        let split_button = self.split_button(cx);
        h_flex()
            .items_center()
            .flex_shrink_0()
            .h(px(32.))
            .px(px(8.))
            .gap(px(2.))
            .bg(t.raised)
            .border_b_1()
            .border_color(t.border)
            .child(
                tool_button(
                    "timeline-tool-select",
                    IconName::MousePointer2,
                    "Ferramenta de seleção — é a única ferramenta e já está ativa",
                )
                .selected(true)
                .disabled(true),
            )
            .child(split_button)
            .child(disabled_tool_button(
                "timeline-tool-delete",
                IconName::Trash,
                "Excluir clipe",
                NO_TOOL,
            ))
            .child(separator(t))
            .child(snap_button)
            .child(
                div().flex_1().flex().justify_center().child(
                    div().w(px(220.)).child(
                        Select::new(&self.sequence_select)
                            .id("sequence-select")
                            .small()
                            .accessibility_label("Sequência ativa"),
                    ),
                ),
            )
            .child(glyph(IconName::ZoomIn, 13.0, t.dim))
            .child(
                div().w(px(120.)).child(
                    Slider::new(&self.zoom_slider)
                        .horizontal()
                        .accessibility_label("Zoom da timeline"),
                ),
            )
    }

    /// F017: "cut at the playhead". Offered exactly when the core would split (the tree node and
    /// `timeline.split invoke` read the same `split_availability`); the click is the `SplitClip`
    /// command the CLI sends, so the window keeps no cutting rule of its own.
    fn split_button(&self, cx: &mut Context<Self>) -> Div {
        let button = match self.state.split_availability() {
            SplitAvailability::Ready { clip, frame } => tool_button(
                "timeline-split",
                IconName::Slice,
                &format!("Cortar {clip} no playhead (quadro {frame})"),
            )
            .on_click(super::controls::click(
                cx,
                Command::SplitClip {
                    clip: clip.to_string(),
                    frame,
                    new_id: None,
                },
            )),
            SplitAvailability::Blocked(block) => disabled_tool_button(
                "timeline-split",
                IconName::Slice,
                "Cortar clipe no playhead",
                split_block_reason(block),
            ),
        };
        div()
            .relative()
            .child(self.geometry.probe("control:timeline-split"))
            .child(button)
    }
    /// What the selector lists: every sequence, named by its name (and its ID when two share one).
    fn sequence_choices(&self) -> Vec<(String, String)> {
        let sequences: Vec<_> = self.state.all_sequences().collect();
        sequences
            .iter()
            .map(|sequence| {
                let shared = sequences
                    .iter()
                    .filter(|other| other.name == sequence.name)
                    .count()
                    > 1;
                let label = if shared {
                    format!("{} ({})", sequence.name, sequence.id)
                } else {
                    sequence.name.clone()
                };
                (sequence.id.to_string(), label)
            })
            .collect()
    }

    /// Follows the model into the selector: its items, then which one is active. `set_items` and
    /// `set_selected_value` do not emit, so this cannot loop back into `ActivateSequence`.
    fn sync_sequence_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let choices = self.sequence_choices();
        if choices != self.sequence_choices {
            let labels: Vec<String> = choices.iter().map(|(_, label)| label.clone()).collect();
            self.sequence_select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(labels), window, cx)
            });
            self.sequence_choices = choices;
        }
        let active = self.state.timeline.id.as_str();
        let Some((_, label)) = self.sequence_choices.iter().find(|(id, _)| id == active) else {
            return;
        };
        if self.sequence_select.read(cx).selected_value() == Some(label) {
            return;
        }
        let label = label.clone();
        self.sequence_select.update(cx, |select, cx| {
            select.set_selected_value(&label, window, cx)
        });
    }

    fn render_ruler(&mut self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let ppf = self.state.timeline.pixels_per_frame();
        let (fps, length) = (self.state.timeline.fps(), self.state.timeline.length);
        let playhead = self.state.timeline.playhead;
        let mut ruler = div()
            .id("timeline-ruler")
            .focusable()
            .relative()
            .flex_shrink_0()
            .h(px(RULER_HEIGHT))
            .w_full()
            .bg(t.raised)
            .border_b_1()
            .border_color(t.border)
            .cursor_pointer();
        for (frame, labelled) in ruler_ticks(ppf, fps, length) {
            let x = frame as f32 * ppf;
            ruler = ruler.child(
                div()
                    .absolute()
                    .left(px(x))
                    .bottom(px(0.))
                    .w(px(1.))
                    .h(px(if labelled { 8.0 } else { 4.0 }))
                    .bg(if labelled { t.faint } else { t.border_strong }),
            );
            if labelled {
                ruler = ruler.child(
                    div()
                        .absolute()
                        .left(px(x + 4.0))
                        .top(px(3.))
                        .text_size(px(9.))
                        .text_color(t.dim)
                        .child(self.state.timeline.timecode(frame)),
                );
            }
        }
        ruler
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    let Some(content) = this.geometry.get("timeline-content") else {
                        return;
                    };
                    let frame = scrub_frame(
                        f32::from(event.position.x),
                        f32::from(content.origin.x),
                        this.state.timeline.pixels_per_frame(),
                    );
                    this.dispatch(Self::seek_command(frame), cx);
                }),
            )
            .on_drag(ScrubDrag, |_, _, _, cx| cx.new(|_| NoGhost))
            .on_drag_move::<ScrubDrag>(cx.listener(
                |this, event: &DragMoveEvent<ScrubDrag>, window, cx| {
                    // WHY: GPUI hands every mouse move to this listener while a drag value
                    // exists, pressed or not, and only a MouseUp clears it. A MouseUp that never
                    // arrives (released over another window, an injected gesture that stops
                    // halfway) would otherwise let plain hovering drag the playhead (B6). A move
                    // with the left button up ends the stale drag instead of scrubbing; every
                    // genuine drag move carries the pressed button (also the synthetic ones GPUI
                    // makes for external file drags, which never carry a `ScrubDrag`).
                    if !event.event.dragging() {
                        cx.stop_active_drag(window);
                        return;
                    }
                    let frame = scrub_frame(
                        f32::from(event.event.position.x),
                        f32::from(event.bounds.origin.x),
                        this.state.timeline.pixels_per_frame(),
                    );
                    this.dispatch(Self::seek_command(frame), cx);
                },
            ))
            .on_key_down(keys(cx, move |key, shift| {
                let step = if shift { fps } else { 1 };
                match key {
                    "left" => Some(Self::seek_command(playhead - step)),
                    "right" => Some(Self::seek_command(playhead + step)),
                    "home" => Some(Self::seek_command(0)),
                    "end" => Some(Self::seek_command(length)),
                    _ => None,
                }
            }))
    }

    fn track_header(
        &mut self,
        t: &Tokens,
        cx: &mut Context<Self>,
        track: &Track,
    ) -> impl IntoElement + use<> {
        let label = self.state.timeline.label(track.id);
        let selected = self.state.timeline.selected_track == Some(track.id);
        let id = track.id;
        let (chip_fill, chip_text) = match track.kind {
            TrackKind::Video => (t.video_fill, t.video_label),
            TrackKind::Audio => (t.audio_fill, t.audio_wave),
        };
        let (toggle_icon, toggle_tip, toggle_command, dimmed) = match track.kind {
            TrackKind::Video => (
                if track.hidden {
                    IconName::EyeOff
                } else {
                    IconName::Eye
                },
                if track.hidden {
                    "Mostrar faixa"
                } else {
                    "Ocultar faixa"
                },
                Command::SetTrackFlag {
                    track: id.to_string(),
                    flag: TrackFlag::Hidden,
                    value: !track.hidden,
                },
                track.hidden,
            ),
            TrackKind::Audio => (
                if track.muted {
                    IconName::VolumeX
                } else {
                    IconName::Volume2
                },
                if track.muted {
                    "Reativar áudio"
                } else {
                    "Silenciar faixa"
                },
                Command::SetTrackFlag {
                    track: id.to_string(),
                    flag: TrackFlag::Muted,
                    value: !track.muted,
                },
                track.muted,
            ),
        };
        let toggle_handler = cx.listener(move |this, _: &ClickEvent, _, cx| {
            // The flag button must not also select the track.
            cx.stop_propagation();
            this.dispatch(toggle_command.clone(), cx);
        });
        let toggle = tool_button(format!("track-toggle-{id}"), toggle_icon, toggle_tip)
            .selected(dimmed)
            .on_click(toggle_handler);
        let ghost = (t.raised, t.border_strong, t.text);
        let name = track.name.clone();
        let select = Command::SelectTrack {
            track: id.to_string(),
        };
        let marker = match &self.preview {
            Some(Preview::Track(preview)) if preview.anchor == id => {
                Some((preview.placement, preview.verdict.is_ok()))
            }
            _ => None,
        };
        let hover = t.hover;
        let header = div()
            .id(ElementId::Name(format!("header-{id}").into()))
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .h(px(track.height))
            .px(px(8.))
            .border_b_1()
            .border_color(t.border)
            .bg(if selected { t.raised } else { t.panel })
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .child(self.geometry.probe(format!("header:{id}")))
            .on_click(super::controls::click(cx, select))
            .on_drag(TrackDrag { track: id.as_str() }, move |_, _, _, cx| {
                cx.new(|_| DragGhost {
                    label: SharedString::from(name.clone()),
                    width: 110.0,
                    height: 24.0,
                    fill: ghost.0,
                    border: ghost.1,
                    text: ghost.2,
                })
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_shrink_0()
                    .w(px(24.))
                    .h(px(18.))
                    .rounded(px(3.))
                    .bg(chip_fill)
                    .text_size(px(10.))
                    .text_color(chip_text)
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(px(11.))
                    .text_color(if selected { t.text } else { t.dim })
                    .child(track.name.clone()),
            )
            .child(toggle);
        let marker_line = marker.map(|(placement, ok)| {
            let line = div()
                .absolute()
                .left(px(0.))
                .right(px(0.))
                .h(px(2.))
                .bg(if ok { t.ok } else { t.bad });
            match placement {
                Placement::Before => line.top(px(0.)),
                Placement::After => line.bottom(px(0.)),
            }
        });
        header.children(marker_line)
    }

    fn clip_block(
        &mut self,
        t: &Tokens,
        cx: &mut Context<Self>,
        clip: &Clip,
        lane_height: f32,
    ) -> impl IntoElement + use<> {
        let ppf = self.state.timeline.pixels_per_frame();
        let selected = self.state.timeline.selected_clip == Some(clip.id);
        let dragging =
            matches!(&self.preview, Some(Preview::Clip(preview)) if preview.clip == clip.id);
        let trim = match &self.preview {
            Some(Preview::Trim(preview)) if preview.clip == clip.id => Some(preview),
            _ => None,
        };
        let plan = trim.and_then(|preview| preview.verdict.as_ref().ok());
        let display_start = plan.map_or(clip.start, |plan| plan.start);
        let display_duration = plan.map_or(clip.duration, |plan| plan.end - plan.start);
        let width = (display_duration as f32 * ppf).max(6.0);
        let height = lane_height - 2.0 * CLIP_INSET;
        let (fill, border, label_color, icon) = match clip.kind {
            ClipKind::Video => (t.video_fill, t.video_border, t.video_label, IconName::Film),
            ClipKind::Title => (t.title_fill, t.title_border, t.text, IconName::Type),
            // Scenes are visual clips: they share the video palette with their own glyph.
            ClipKind::Scene => (
                t.video_fill,
                t.video_border,
                t.video_label,
                IconName::Layers,
            ),
            ClipKind::Audio => (
                t.audio_fill,
                t.audio_border,
                t.audio_wave,
                IconName::AudioLines,
            ),
        };
        let border = if trim.is_some_and(|preview| preview.verdict.is_err()) {
            t.bad
        } else {
            border
        };
        let id = clip.id;
        let name = clip.name.clone();
        let me = cx.entity();
        let ghost = (fill, border, label_color);
        let ghost_width = width;
        let ghost_height = height;
        let mut block = div()
            .id(ElementId::Name(format!("clip-{id}").into()))
            .absolute()
            .left(px(display_start as f32 * ppf))
            .top(px(CLIP_INSET))
            .w(px(width))
            .h(px(height))
            .flex()
            .flex_col()
            .rounded(px(3.))
            .overflow_hidden()
            .border_1()
            .border_color(if trim.is_some_and(|preview| preview.verdict.is_err()) {
                t.bad
            } else if selected {
                t.selected
            } else {
                border
            })
            .when(selected, |block| block.border_2())
            .bg(fill)
            .when(dragging, |block| block.opacity(0.4))
            .cursor_pointer()
            .on_click(super::controls::click(
                cx,
                Command::SelectClip {
                    clip: id.to_string(),
                },
            ))
            .on_drag(ClipDrag { clip: id.as_str() }, move |_, offset, _, cx| {
                me.update(cx, |this, _| this.begin_clip_drag(f32::from(offset.x)));
                cx.new(|_| DragGhost {
                    label: SharedString::from(name.clone()),
                    width: ghost_width,
                    height: ghost_height,
                    fill: ghost.0,
                    border: ghost.1,
                    text: ghost.2,
                })
            });
        block = block.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .flex_shrink_0()
                .gap(px(4.))
                .h(px(LABEL_HEIGHT))
                .px(px(4.))
                .text_size(px(10.))
                .text_color(label_color)
                .child(glyph(icon, 10.0, label_color))
                .child(div().truncate().child(clip.name.clone())),
        );
        match clip.kind {
            ClipKind::Video => {
                let body_height = (height - LABEL_HEIGHT - FOOTER_HEIGHT - 2.0).max(8.0);
                let tile_width = body_height * 16.0 / 9.0;
                let tiles = (width / tile_width).ceil() as usize;
                let image = clip
                    .media
                    .and_then(|media| self.state.library.item(media))
                    .and_then(|item| item.image);
                let mut strip = div()
                    .flex()
                    .flex_row()
                    .w_full()
                    .h(px(body_height))
                    .overflow_hidden();
                if let Some(image) = image {
                    for _ in 0..tiles {
                        strip = strip.child(
                            img(demo_image(image))
                                .flex_shrink_0()
                                .w(px(tile_width))
                                .h(px(body_height))
                                .object_fit(ObjectFit::Cover),
                        );
                    }
                }
                block = block.child(strip).child(
                    div()
                        .px(px(4.))
                        .text_size(px(9.))
                        .text_color(label_color)
                        .child(self.state.timeline.timecode(clip.start)),
                );
            }
            ClipKind::Audio => {
                let muted = self
                    .state
                    .timeline
                    .track(clip.track)
                    .is_some_and(|track| track.muted);
                let color = if muted { t.faint } else { t.audio_wave };
                block = block.child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .w_full()
                        .px(px(2.))
                        .pb(px(2.))
                        .child(waveform(clip.id.as_str(), color, ppf)),
                );
            }
            ClipKind::Title | ClipKind::Scene => {}
        }
        for edge in [TrimEdge::Start, TrimEdge::End] {
            let side = match edge {
                TrimEdge::Start => "start",
                TrimEdge::End => "end",
            };
            let hover_fill = t.selected.opacity(0.45);
            let handle = div()
                .id(ElementId::Name(format!("trim-{id}-{side}").into()))
                .absolute()
                .top(px(0.))
                .bottom(px(0.))
                .w(px((width / 2.).min(8.)))
                .flex()
                .items_center()
                .justify_center()
                .cursor(CursorStyle::ResizeLeftRight)
                .bg(t.app.opacity(0.3))
                .hover(move |style| style.bg(hover_fill))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        window.focus(&this.focus, cx);
                        cx.stop_propagation();
                    }),
                )
                .on_click(|_, _, cx| cx.stop_propagation())
                .on_drag(
                    TrimDrag {
                        clip: id.as_str(),
                        edge,
                    },
                    |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| NoGhost)
                    },
                )
                .child(self.geometry.probe(format!("trim:{id}:{side}")))
                .child(div().w(px(2.)).h(px(14.)).rounded(px(1.)).bg(t.text));
            let handle = match edge {
                TrimEdge::Start => handle.left(px(0.)),
                TrimEdge::End => handle.right(px(0.)),
            };
            block = block.child(handle);
        }
        block
    }

    fn lane(
        &mut self,
        t: &Tokens,
        cx: &mut Context<Self>,
        track: &Track,
        alternate: bool,
    ) -> impl IntoElement + use<> {
        let id = track.id;
        let ppf = self.state.timeline.pixels_per_frame();
        let clips: Vec<Clip> = self.state.timeline.clips_on(&id).cloned().collect();
        let mut lane = div()
            .id(ElementId::Name(format!("lane-{id}").into()))
            .relative()
            .flex_shrink_0()
            .w_full()
            .h(px(track.height))
            .border_b_1()
            .border_color(t.border)
            .bg(if alternate { t.lane_alt } else { t.lane })
            .when(track.hidden, |lane| lane.opacity(0.45))
            .child(self.geometry.probe(format!("lane:{id}")));
        for clip in &clips {
            lane = lane.child(self.clip_block(t, cx, clip, track.height));
        }
        let drop_marker = match &self.preview {
            Some(Preview::Clip(preview)) if preview.track == id => {
                let duration = self
                    .state
                    .timeline
                    .clip(preview.clip)
                    .map_or(0, |clip| clip.duration);
                let (start, ok, note) = match &preview.verdict {
                    Ok(plan) => (plan.start, true, String::new()),
                    Err(error) => (preview.requested_frame, false, error.code.replace('_', " ")),
                };
                Some(
                    div()
                        .absolute()
                        .left(px(start as f32 * ppf))
                        .top(px(CLIP_INSET))
                        .w(px((duration as f32 * ppf).max(6.0)))
                        .h(px(track.height - 2.0 * CLIP_INSET))
                        .rounded(px(3.))
                        .border_2()
                        .border_color(if ok { t.ok } else { t.bad })
                        .bg(if ok {
                            t.ok.opacity(0.18)
                        } else {
                            t.bad.opacity(0.18)
                        })
                        .px(px(4.))
                        .text_size(px(9.))
                        .text_color(if ok { t.ok } else { t.bad })
                        .child(note),
                )
            }
            _ => None,
        };
        lane.children(drop_marker)
    }

    fn render_timeline_body(
        &mut self,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let ppf = self.state.timeline.pixels_per_frame();
        let playhead = self.state.timeline.playhead;
        let content_width = self.state.timeline.length as f32 * ppf + TAIL_PADDING;
        let tracks: Vec<Track> = self.state.timeline.tracks.clone();

        let mut headers = div()
            .relative()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(px(HEADER_WIDTH))
            .h_full()
            .border_r_1()
            .border_color(t.border)
            .bg(t.panel)
            .child(self.geometry.probe("timeline-headers"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .h(px(RULER_HEIGHT))
                    .px(px(8.))
                    .border_b_1()
                    .border_color(t.border)
                    .text_size(px(10.))
                    .text_color(t.text)
                    .child(self.state.timeline.timecode(playhead)),
            );
        for track in &tracks {
            headers = headers.child(self.track_header(t, cx, track));
        }
        let headers = headers
            .on_drag_move::<TrackDrag>(cx.listener(
                |this, event: &DragMoveEvent<TrackDrag>, _, cx| {
                    let track = event.drag(cx).track;
                    this.update_track_drag(track, event.event.position, cx);
                },
            ))
            .on_drop::<TrackDrag>(cx.listener(|this, _: &TrackDrag, _, cx| {
                this.finish_track_drag(cx);
            }));

        let mut lanes = div().flex().flex_col().w_full();
        for (index, track) in tracks.iter().enumerate() {
            lanes = lanes.child(self.lane(t, cx, track, index % 2 == 1));
        }

        let playhead_x = playhead as f32 * ppf;
        let content = div()
            .id("timeline-content")
            .relative()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(px(content_width))
            .h_full()
            .child(self.geometry.probe("timeline-content"))
            .child(self.render_ruler(t, cx))
            .child(lanes)
            .child(
                div()
                    .absolute()
                    .left(px(playhead_x))
                    .top(px(0.))
                    .bottom(px(0.))
                    .w(px(1.))
                    .bg(t.playhead),
            )
            .child(
                div()
                    .absolute()
                    .left(px(playhead_x - 4.0))
                    .top(px(0.))
                    .w(px(9.))
                    .h(px(12.))
                    .rounded_b(px(3.))
                    .bg(t.playhead),
            )
            .on_drag_move::<ClipDrag>(cx.listener(
                |this, event: &DragMoveEvent<ClipDrag>, _, cx| {
                    let clip = event.drag(cx).clip;
                    this.update_clip_drag(clip, event.event.position, cx);
                },
            ))
            .on_drop::<ClipDrag>(cx.listener(|this, _: &ClipDrag, _, cx| {
                this.finish_clip_drag(cx);
            }))
            .on_drag_move::<TrimDrag>(cx.listener(
                |this, event: &DragMoveEvent<TrimDrag>, _, cx| {
                    let drag = event.drag(cx).clone();
                    this.update_trim_drag(&drag, event.event.position, cx);
                },
            ))
            .on_drop::<TrimDrag>(cx.listener(|this, drag: &TrimDrag, window, cx| {
                // WHY: the threshold-crossing move starts GPUI's drag and stops propagation;
                // a quick release can arrive before any preview move reaches the timeline.
                this.update_trim_drag(drag, window.mouse_position(), cx);
                this.finish_trim_drag(cx);
            }));

        div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h(px(0.))
            .child(headers)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .overflow_hidden()
                    .child(self.geometry.probe("timeline-scroll"))
                    .child(
                        div()
                            .id("timeline-scroll")
                            .size_full()
                            .overflow_x_scroll()
                            .child(content),
                    ),
            )
    }

    fn render_mode_notice(&self, t: &Tokens) -> impl IntoElement + use<> {
        let (name, icon) = match self.state.workspace_tab {
            WorkspaceTab::Color => ("Cor", IconName::Palette),
            _ => ("Entregar", IconName::Upload),
        };
        div()
            .flex()
            .flex_1()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .text_size(px(11.))
            .text_color(t.dim)
            .child(glyph(icon, 22.0, t.faint))
            .child(format!("O modo {name} ainda não foi implementado."))
            .child(
                div()
                    .text_color(t.faint)
                    .child("Apenas Editar está disponível nesta etapa."),
            )
    }

    pub(super) fn render_timeline(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let active = self.state.workspace_tab;
        let modes: Vec<TabSpec> = [
            (WorkspaceTab::Edit, "Editar"),
            (WorkspaceTab::Color, "Cor"),
            (WorkspaceTab::Deliver, "Entregar"),
        ]
        .into_iter()
        .map(|(tab, label)| TabSpec {
            label: label.into(),
            active: tab == active,
            command: Command::SetWorkspaceTab { tab },
        })
        .collect();
        let mode_bar = command_tabs(cx, "workspace-tabs", TabStyle::Pill, modes);
        let editing = active == WorkspaceTab::Edit;
        let toolbar = self.render_timeline_toolbar(t, window, cx);
        let body = if editing {
            self.render_timeline_body(t, cx).into_any_element()
        } else {
            self.render_mode_notice(t).into_any_element()
        };
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.))
            .bg(t.panel)
            .child(self.geometry.probe("region:timeline"))
            .child(toolbar)
            .child(body)
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(t.border)
                    .child(mode_bar),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::timeline::{FPS, SEQUENCE_FRAMES};
    // `gpui_kit::*` also exports GPUI's `test` attribute, which would shadow this one.
    use core::prelude::v1::test;

    #[test]
    fn labels_never_crowd_whatever_the_zoom() {
        for zoom in [8.0_f32, 14.0, 28.0, 60.0, 120.0] {
            let ppf = zoom / FPS as f32;
            let labelled: Vec<i64> = ruler_ticks(ppf, FPS, SEQUENCE_FRAMES)
                .into_iter()
                .filter(|(_, labelled)| *labelled)
                .map(|(frame, _)| frame)
                .collect();
            for pair in labelled.windows(2) {
                assert!(
                    (pair[1] - pair[0]) as f32 * ppf >= 72.0 - 0.01,
                    "zoom {zoom}"
                );
            }
            assert_eq!(labelled[0], 0);
        }
    }

    #[test]
    fn minor_ticks_appear_only_when_they_fit() {
        let minor = |zoom: f32| {
            ruler_ticks(zoom / FPS as f32, FPS, SEQUENCE_FRAMES)
                .into_iter()
                .filter(|(_, labelled)| !labelled)
                .count()
        };
        assert_eq!(minor(5.0), 0);
        assert!(minor(28.0) > 0);
    }
}
