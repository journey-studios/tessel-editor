//! Group clips in the plan (F069): one clip becomes the shape and text layers of its children.
//!
//! The compositor knows no group. A group clip is resolved here, at the frame being drawn, into
//! ordinary layers (bottom to top, in the order of the definition's children), so the monitor, the
//! PNG of `render.frame` and the export draw it with the same runtime as every other clip.
//!
//! The clip's own properties move the whole composition: its position is the group's origin, its
//! zoom scales the offsets and the children, its rotation turns the offsets around the origin and
//! each child with it, and its opacity, fades, blend and colour correction apply to every child.
//! A child's own values (base or keyed) are relative to that origin.

use super::*;
use crate::group::{Placed, PlacedContent};

impl EditorState {
    /// The layers of the group clip `clip` at sequence `frame`; empty (with a warning) when the
    /// group cannot be resolved, so a damaged reference never stops a frame from being drawn.
    pub(super) fn group_layers(
        &self,
        resolver: &mut Resolver<'_>,
        clip: &Clip,
        frame: i64,
    ) -> Result<Vec<Layer>, CommandError> {
        let skip = |resolver: &mut Resolver<'_>, code: &'static str, message: String| {
            resolver.warnings.push(Warning {
                code,
                message,
                clip: Some(clip.id),
            });
            Ok(Vec::new())
        };
        let Some(instance) = &clip.group else {
            return skip(
                resolver,
                "group_without_instance",
                format!("Group clip `{}` shows no group and is skipped", clip.id),
            );
        };
        // WHY milliseconds as a float: keys are whole milliseconds and the frame clock is exact
        // (a rational of seconds), so one conversion keeps every surface on the same instant.
        let local_ms = self
            .timeline
            .rate
            .frame_to_time(frame - clip.start)
            .to_f64()
            * 1000.0;
        let canvas = (self.timeline.width, self.timeline.height);
        let placed = match self.groups.place(instance, local_ms, canvas) {
            Ok(placed) => placed,
            Err(error) => {
                return skip(
                    resolver,
                    "group_unresolved",
                    format!("Group clip `{}` is skipped: {}", clip.id, error.message),
                );
            }
        };
        Ok(self.placed_layers(resolver, clip, frame, placed))
    }

    /// The layers of `placed` parts (shapes and texts, bottom to top) moved, scaled, turned and
    /// faded by the properties of `clip` at `frame`: what a group clip, a player clip and a
    /// subtitle clip share (F069, F071). One function, so the three move, fade, blend and grade
    /// their parts the same way.
    pub(super) fn placed_layers(
        &self,
        resolver: &mut Resolver<'_>,
        clip: &Clip,
        frame: i64,
        placed: Vec<Placed>,
    ) -> Vec<Layer> {
        let origin = resolver.transform(clip, frame, false);
        let opacity = resolver.opacity(clip, frame);
        let blend = resolver.choice(clip, "blend");
        let grading = resolver.grading(clip);
        let (width, height) = (
            f64::from(self.timeline.width),
            f64::from(self.timeline.height),
        );
        let (sin, cos) = origin.rotation.to_radians().sin_cos();

        let mut layers = Vec::with_capacity(placed.len());
        for Placed {
            child,
            content,
            x,
            y,
            scale,
            rotation,
            opacity: child_opacity,
            visible,
        } in placed
        {
            // A text that has not typed its first character yet draws nothing: no layer, so
            // the runtime does not rasterise an empty picture on every frame of the wait.
            if visible == Some(0) {
                continue;
            }
            let content = match content {
                PlacedContent::Shape(shape) if !shape.draws_anything() => {
                    resolver.warnings.push(Warning {
                        code: "shape_draws_nothing",
                        message: format!(
                            "Child `{child}` of clip `{}` has no fill and no contour thickness and is skipped",
                            clip.id
                        ),
                        clip: Some(clip.id),
                    });
                    continue;
                }
                PlacedContent::Shape(shape) => Content::Shape(shape),
                PlacedContent::Text(text) => Content::Text(text),
            };
            // The offset is turned in pixels (the percent axes have different lengths), then
            // brought back to the percent of the canvas the layer transform speaks in.
            let (dx, dy) = (
                x / 100.0 * width * origin.scale,
                y / 100.0 * height * origin.scale,
            );
            let (turned_x, turned_y) = (dx * cos - dy * sin, dx * sin + dy * cos);
            let mut layer = Layer {
                id: format!("layer:{}:{child}", clip.id),
                clip: Some(clip.id),
                asset: None,
                content,
                transform: Transform {
                    x: origin.x + turned_x / width * 100.0,
                    y: origin.y + turned_y / height * 100.0,
                    scale: origin.scale * scale,
                    rotation: origin.rotation + rotation,
                    crop: [0.0; 4],
                },
                opacity: (opacity * child_opacity).clamp(0.0, 1.0),
                blend: blend.clone(),
                grading,
                mask: None,
                reveal: None,
                occluded: false,
                text_reveal: None,
                text_visible: visible,
            };
            self.anchor_text(&mut layer);
            layers.push(layer);
        }
        layers
    }
}
