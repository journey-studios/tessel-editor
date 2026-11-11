//! F061 (G4.1, BL082.35): what the monitor draws and hits for ONE vignette clip: the target the
//! session holds, the handles with their typed reasons, and the two ellipses.
//!
//! The geometry is [`directed_scene::fx::handles`](crate::directed_scene::fx): this module only
//! reads a clip at a frame, says which handle is offered, which is not and why, and prints the
//! JSON an overlay carries. Pure over a [`DirectedScene`]: no window, no pointer device, no GPUI.
//! A runtime draws what it lists; the decision (which handle a press holds, what value a position
//! means) stays in the core.

use serde_json::{Value, json};

use crate::directed_scene::DirectedScene;
use crate::directed_scene::fx::{
    HANDLE_HIT_RADIUS, HandleValue, VignetteHandle, VignetteParams, VignetteShape, hit_handle,
    on_picture, vignette_params_at,
};
use crate::state::CommandError;

use super::gizmo::Point;

/// The typed reasons a handle, or all of them, is not offered. Strings are part of the contract
/// (`disabled_handles[].reason`, `fx.reason`, the node's `disabled_reason`).
pub mod reason {
    /// The parameter has keys: the handle would edit a base the keys hide, so it is off. It never
    /// writes a key and never takes the `FX_STATIC_UNDER_KEYS` road.
    pub const ANIMATED_PARAM: &str = "animated_param";
    /// The handle falls outside the picture (a wide radius plus softness): not drawn, not pressable.
    pub const HANDLE_OFF_PICTURE: &str = "handle_off_picture";
    /// The clip does not cover the displayed frame: no handles at all.
    pub const CLIP_NOT_IN_FRAME: &str = "clip_not_in_frame";
    /// The output has no usable size, so no shape can be drawn.
    pub const DEGENERATE_OUTPUT: &str = "degenerate_output";
}

/// The effect clip the monitor edits with handles, held by the session like the selection is: not
/// in the document, not in an undo snapshot, reset by a new project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FxTarget {
    /// The attached scene (bundle ID).
    pub scene: String,
    pub clip: String,
}

impl FxTarget {
    pub fn to_json(&self) -> Value {
        json!({ "scene": self.scene, "clip": self.clip })
    }
}

/// One vignette clip as the monitor shows it at one edit frame.
#[derive(Debug, Clone, PartialEq)]
pub struct FxHandlesView {
    pub clip: String,
    pub effect: &'static str,
    pub frame: i64,
    /// `None` when the clip does not cover `frame`.
    pub frame_in_clip: Option<i64>,
    pub width: u64,
    pub height: u64,
    pub visible: bool,
    pub reason: Option<&'static str>,
    /// The parameters at the frame (keys applied): the geometry the picture shows.
    pub params: Option<VignetteParams>,
    pub shape: Option<VignetteShape>,
    /// The handles offered (enabled and on the picture), with their position.
    pub handles: Vec<(VignetteHandle, (f64, f64))>,
    /// The handles that exist but are not offered, each with its typed reason.
    pub disabled: Vec<(VignetteHandle, &'static str)>,
}

impl FxHandlesView {
    /// The handles of `clip` at the edit `frame` of `scene`. Errors are typed: `unknown_element`
    /// for a clip the layer does not have, `fx_has_no_handles` for an effect without a spatial
    /// parameter (`fade`, `filmGrain`).
    pub fn of(scene: &DirectedScene, clip: &str, frame: i64) -> Result<Self, CommandError> {
        let bundle = scene.bundle();
        let Some((_, found)) = bundle.fx.clip(clip) else {
            return Err(CommandError::unknown_element("effect clip", clip));
        };
        if found.effect.id != crate::directed_scene::fx::VIGNETTE.id {
            return Err(CommandError::new(
                "fx_has_no_handles",
                format!(
                    "`{clip}` is a `{}`: only the vignette has handles on the monitor",
                    found.effect.id
                ),
            )
            .target(clip)
            .details(json!({ "clip": clip, "effect": found.effect.id })));
        }
        let (width, height) = (bundle.render_profile.width, bundle.render_profile.height);
        let mut view = Self {
            clip: clip.to_string(),
            effect: found.effect.id,
            frame,
            frame_in_clip: None,
            width,
            height,
            visible: false,
            reason: None,
            params: None,
            shape: None,
            handles: Vec::new(),
            disabled: Vec::new(),
        };
        let covers = frame >= found.start_frame && frame < found.end_frame_exclusive();
        if !covers {
            view.reason = Some(reason::CLIP_NOT_IN_FRAME);
            return Ok(view);
        }
        let in_clip = frame - found.start_frame;
        view.frame_in_clip = Some(in_clip);
        let params = vignette_params_at(found, in_clip);
        let shape = params
            .and_then(|params| VignetteShape::new(width as f64, height as f64, params.roundness));
        let (Some(params), Some(shape)) = (params, shape) else {
            view.reason = Some(reason::DEGENERATE_OUTPUT);
            return Ok(view);
        };
        view.visible = true;
        view.params = Some(params);
        view.shape = Some(shape);
        for handle in VignetteHandle::ALL {
            let animated = found
                .curve_of(handle.param())
                .is_some_and(|keys| !keys.is_empty());
            let at = shape.handle_point(&params, handle);
            if animated {
                view.disabled.push((handle, reason::ANIMATED_PARAM));
            } else if !on_picture(at) {
                view.disabled.push((handle, reason::HANDLE_OFF_PICTURE));
            } else {
                view.handles.push((handle, at));
            }
        }
        Ok(view)
    }

    /// The handle a press at `at` holds, among the offered ones.
    pub fn hit(&self, at: Point) -> Option<VignetteHandle> {
        let shape = self.shape?;
        hit_handle(&self.handles, (at.u, at.v), shape.aspect())
    }

    /// The handle a press at `at` would hold if its parameter did not have keys: drawn, greyed,
    /// and refused by name (`animated_param`) instead of letting the press fall through to what is
    /// behind it. A handle off the picture is not in this list: it is not drawn.
    pub fn hit_animated(&self, at: Point) -> Option<VignetteHandle> {
        let shape = self.shape?;
        let drawn: Vec<(VignetteHandle, (f64, f64))> = self
            .disabled
            .iter()
            .filter(|(_, why)| *why == reason::ANIMATED_PARAM)
            .filter_map(|(handle, _)| Some((*handle, self.point_of(*handle)?)))
            .filter(|(_, point)| on_picture(*point))
            .collect();
        hit_handle(&drawn, (at.u, at.v), shape.aspect())
    }

    /// Why `handle` is not offered, if it is not.
    pub fn disabled_reason(&self, handle: VignetteHandle) -> Option<&'static str> {
        self.disabled
            .iter()
            .find(|(candidate, _)| *candidate == handle)
            .map(|(_, reason)| *reason)
    }

    /// The value a pointer at `at` gives `handle` (see [`VignetteShape::pointer_value`]).
    pub fn pointer_value(&self, handle: VignetteHandle, at: Point) -> Option<HandleValue> {
        self.shape?
            .pointer_value(&self.params?, handle, (at.u, at.v))
    }

    /// Where `handle` is, offered or not.
    pub fn point_of(&self, handle: VignetteHandle) -> Option<(f64, f64)> {
        Some(self.shape?.handle_point(&self.params?, handle))
    }

    /// The `fx` block of the overlay (and the answer of `scene.fx.handles`). `active` is the handle
    /// being dragged.
    pub fn to_json(&self, active: Option<VignetteHandle>) -> Value {
        let at_json = |at: (f64, f64)| json!({ "u": at.0, "v": at.1 });
        let mut out = json!({
            "clip": self.clip, "effect": self.effect,
            "frame": self.frame, "frame_in_clip": self.frame_in_clip,
            "output": { "width": self.width, "height": self.height },
            "visible": self.visible, "reason": self.reason,
            "handles": self.handles.iter().map(|(handle, at)| json!({
                "id": handle.as_str(), "param": handle.param(),
                "kind": if *handle == VignetteHandle::Center { "center" } else { "edge" },
                "at": at_json(*at),
            })).collect::<Vec<_>>(),
            "disabled_handles": self.disabled.iter()
                .map(|(handle, reason)| json!({ "id": handle.as_str(), "reason": reason }))
                .collect::<Vec<_>>(),
            "ellipses": [],
            "active": active.map(VignetteHandle::as_str),
            "metrics": { "hit_radius": HANDLE_HIT_RADIUS, "unit": "picture_height" },
        });
        if let (Some(params), Some(shape)) = (self.params, self.shape) {
            let ellipse = |id: &str, r: f64| {
                let (rx, ry) = shape.semi_axes(r);
                json!({ "id": id, "center": at_json(params.center), "rx": rx, "ry": ry })
            };
            out["ellipses"] = json!([
                ellipse("radius", params.radius),
                ellipse("softness", params.radius + params.softness),
            ]);
            out["values"] = json!({
                "center": { "x": params.center.0, "y": params.center.1 },
                "radius": params.radius, "softness": params.softness,
                "roundness": params.roundness,
            });
        }
        out
    }
}

/// What an overlay carries when the session's target is a vignette clip: the view and the handle
/// being dragged. The 3D gizmo is not in such an overlay.
#[derive(Debug, Clone, PartialEq)]
pub struct FxOverlay {
    pub view: FxHandlesView,
    pub active: Option<VignetteHandle>,
}

impl FxOverlay {
    pub fn to_json(&self) -> Value {
        self.view.to_json(self.active)
    }
}
