//! Drag payloads, ghosts and the pointer-to-model math for timeline gestures.
//!
//! Dragging never edits state. A gesture only produces a *preview*; the model
//! changes when the drop runs the same `Command` the CLI sends.

use gpui_kit::*;

use crate::state::CommandError;
use crate::state::timeline::{MovePlan, Placement, ReorderPlan, TrimEdge, TrimPlan};

/// Payload of a clip being dragged. The grab offset lives in the workstation
/// because GPUI only reports it when the drag starts.
#[derive(Clone)]
pub struct ClipDrag {
    pub clip: &'static str,
}

#[derive(Clone)]
pub struct TrackDrag {
    pub track: &'static str,
}

/// Scrubbing the playhead along the ruler.
#[derive(Clone)]
pub struct ScrubDrag;

pub struct ClipPreview {
    pub clip: &'static str,
    pub track: &'static str,
    pub requested_frame: i64,
    pub verdict: Result<MovePlan, CommandError>,
}

pub struct TrackPreview {
    pub track: &'static str,
    pub anchor: &'static str,
    pub placement: Placement,
    pub verdict: Result<ReorderPlan, CommandError>,
}

#[derive(Clone)]
pub struct TrimDrag {
    pub clip: &'static str,
    pub edge: TrimEdge,
}

pub struct TrimPreview {
    pub clip: &'static str,
    pub edge: TrimEdge,
    pub frame: i64,
    pub verdict: Result<TrimPlan, CommandError>,
}

pub enum Preview {
    Trim(TrimPreview),
    Clip(ClipPreview),
    Track(TrackPreview),
}

/// The frame a clip would start at when the pointer is at `x` and the clip was
/// grabbed `grab_px` from its left edge.
pub fn frame_at(x: f32, lane_left: f32, pixels_per_frame: f32, grab_px: f32) -> i64 {
    if pixels_per_frame <= 0.0 {
        return 0;
    }
    ((x - lane_left - grab_px) / pixels_per_frame).round() as i64
}

/// Frame under the pointer, for scrubbing.
pub fn scrub_frame(x: f32, lane_left: f32, pixels_per_frame: f32) -> i64 {
    frame_at(x, lane_left, pixels_per_frame, 0.0)
}

/// The row containing `y`, given `(id, top, bottom)` rows.
pub fn row_at(y: f32, rows: &[(&'static str, f32, f32)]) -> Option<&'static str> {
    rows.iter()
        .find(|(_, top, bottom)| y >= *top && y < *bottom)
        .map(|(id, _, _)| *id)
}

/// Insertion point for a dragged header: the upper half of a row inserts
/// before it, the lower half after it.
pub fn insertion_at(
    y: f32,
    rows: &[(&'static str, f32, f32)],
) -> Option<(&'static str, Placement)> {
    let id = row_at(y, rows)?;
    let (_, top, bottom) = rows.iter().find(|(row, _, _)| *row == id)?;
    let placement = if y < (top + bottom) / 2.0 {
        Placement::Before
    } else {
        Placement::After
    };
    Some((id, placement))
}

pub struct DragGhost {
    pub label: SharedString,
    pub width: f32,
    pub height: f32,
    pub fill: Hsla,
    pub border: Hsla,
    pub text: Hsla,
}

impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(self.width))
            .h(px(self.height))
            .px(px(6.))
            .flex()
            .items_center()
            .rounded(px(3.))
            .border_1()
            .border_color(self.border)
            .bg(self.fill)
            .opacity(0.85)
            .text_size(px(10.))
            .text_color(self.text)
            .overflow_hidden()
            .child(self.label.clone())
    }
}

/// Draws nothing: slider and scrub drags only need pointer capture.
pub struct NoGhost;

impl Render for NoGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size(px(0.))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` also exports GPUI's `test` attribute, which would shadow this one.
    use core::prelude::v1::test;

    #[test]
    fn frames_follow_the_pointer_minus_the_grab_offset() {
        // 2 px per frame, lane at x=100, grabbed 20 px into the clip.
        assert_eq!(frame_at(100.0 + 20.0 + 48.0, 100.0, 2.0, 20.0), 24);
        assert_eq!(frame_at(100.0, 100.0, 2.0, 20.0), -10);
        assert_eq!(frame_at(0.0, 0.0, 0.0, 0.0), 0);
        assert_eq!(scrub_frame(150.0, 100.0, 2.0), 25);
    }

    #[test]
    fn rows_resolve_by_vertical_position() {
        let rows = [("a", 0.0, 30.0), ("b", 30.0, 100.0)];
        assert_eq!(row_at(10.0, &rows), Some("a"));
        assert_eq!(row_at(30.0, &rows), Some("b"));
        assert_eq!(row_at(100.0, &rows), None);
        assert_eq!(row_at(-1.0, &rows), None);
    }

    #[test]
    fn header_insertion_uses_the_row_half() {
        let rows = [("a", 0.0, 40.0), ("b", 40.0, 80.0)];
        assert_eq!(insertion_at(10.0, &rows), Some(("a", Placement::Before)));
        assert_eq!(insertion_at(30.0, &rows), Some(("a", Placement::After)));
        assert_eq!(insertion_at(41.0, &rows), Some(("b", Placement::Before)));
        assert_eq!(insertion_at(200.0, &rows), None);
    }
}
