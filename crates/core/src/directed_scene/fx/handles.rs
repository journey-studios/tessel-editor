//! F061 (G4.1, BL082.35): the geometry of the vignette's 2D handles, as pure functions.
//!
//! The vignette is an ellipse centred on `center` that darkens the picture from where
//! `distance = radius` (untouched) to where `distance = radius + softness` (fully the colour). A
//! monitor draws two handles on that ellipse, `radius` and `softness`, plus the centre itself; this
//! module says **where they are** for a picture of `W×H` pixels, and **what value a pointer at a
//! picture position means** for each of them. A runtime only draws what the core lists, so the
//! numbers here are the whole contract: the same function is used by the overlay, by the drag of
//! the pointer and by the semantic probe, and `vignette_shade` of the reference is the oracle that
//! fixes it (`shade = 0` at the radius handle, `1` at the softness handle).
//!
//! # The formula
//!
//! Taken from [`vignette_shade`](super::vignette_shade), with `u, v` in picture fractions (`v`
//! measured DOWN, like the plan's `center`), `ρ = roundness` and `W×H` the output in pixels:
//!
//! * `stretch = 1 + (W/H − 1)·ρ`, `shape = (stretch, 1)`, `norm = max(√(stretch² + 1), 1)`;
//! * `distance(u, v) = √((2·(u − cx)·stretch)² + (2·(v − cy))²) / norm`;
//! * the ellipse of radius `r` has the semi-axes (in picture fractions)
//!   `a_u = r·norm / (2·stretch)` and `a_v = r·norm / 2`;
//! * the `radius` handle is at `(cx + a_u(radius), cy)` and the `softness` handle at
//!   `(cx + a_u(radius + softness), cy)`: both on the horizontal axis through the centre.
//!
//! The inverse (a pointer at `p`): `radius = distance(p)`, `softness = distance(p) − radius`,
//! `center = p`; then each value goes through the grid of the control it belongs to
//! ([`quantize_control`]: the step of the catalog anchored at its minimum, inside its range), the
//! same grid as the slider of the inspector and the keyboard.

use super::{FxValue, quantize_control};
use super::{ParamKind, VIGNETTE, VIGNETTE_CENTER, VIGNETTE_RADIUS, VIGNETTE_SOFTNESS};

/// The grid of `center`'s two components: the step of the inspector's `center_x`/`center_y` fields
/// (the catalog gives a point a range and no step; `scene_edit::fx_inspector` fixes it at 0.01, and
/// a test of the core pins the two together).
pub const CENTER_STEP: f64 = 0.01;

/// How close (picture heights) a pointer must be to a handle to hold it: the one of the node gizmo.
pub const HIT_RADIUS: f64 = 0.025;

/// The three handles of a vignette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VignetteHandle {
    /// The centre of the falloff; a free 2D drag.
    Center,
    /// Where the falloff starts (`distance = radius`).
    Radius,
    /// Where the falloff ends (`distance = radius + softness`).
    Softness,
}

impl VignetteHandle {
    /// In the order a panel lists them.
    pub const ALL: [Self; 3] = [Self::Center, Self::Radius, Self::Softness];

    /// Who wins when a pointer is as near to two handles (they coincide when `radius` is 0): the
    /// outer ones first, so the centre is never the one that hides the other two.
    const PRIORITY: [Self; 3] = [Self::Softness, Self::Radius, Self::Center];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Radius => "radius",
            Self::Softness => "softness",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|handle| handle.as_str() == text)
    }

    /// The catalog parameter the handle edits: the same text as the handle.
    pub fn param(self) -> &'static str {
        self.as_str()
    }
}

/// What a handle drag writes: a number (`radius`, `softness`) or the whole `center` point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HandleValue {
    Number(f64),
    Point { x: f64, y: f64 },
}

impl HandleValue {
    /// The JSON a `set_param` of the parameter carries (and the answers print).
    pub fn to_json(self) -> serde_json::Value {
        match self {
            Self::Number(value) => serde_json::json!(value),
            Self::Point { x, y } => serde_json::json!({ "x": x, "y": y }),
        }
    }
}

/// The parameters the geometry depends on, at one frame (keys applied).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VignetteParams {
    pub center: (f64, f64),
    pub radius: f64,
    pub softness: f64,
    pub roundness: f64,
}

/// The shape of the falloff for one output size and `roundness`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VignetteShape {
    aspect: f64,
    stretch: f64,
    norm: f64,
}

impl VignetteShape {
    /// `None` when the output is not a usable size or `roundness` is not finite.
    pub fn new(width: f64, height: f64, roundness: f64) -> Option<Self> {
        let usable = |value: f64| value.is_finite() && value > 0.0;
        if !(usable(width) && usable(height) && roundness.is_finite()) {
            return None;
        }
        let aspect = width / height;
        let stretch = 1.0 + (aspect - 1.0) * roundness;
        let norm = (stretch * stretch + 1.0).sqrt().max(1.0);
        (stretch.is_finite() && stretch > 0.0 && norm.is_finite()).then_some(Self {
            aspect,
            stretch,
            norm,
        })
    }

    /// `W / H` of the output.
    pub fn aspect(&self) -> f64 {
        self.aspect
    }

    /// The shader's distance of the picture position `at` (fractions, `v` down) from `center`.
    pub fn distance(&self, center: (f64, f64), at: (f64, f64)) -> f64 {
        let dx = (at.0 - center.0) * 2.0 * self.stretch;
        let dy = (at.1 - center.1) * 2.0;
        dx.hypot(dy) / self.norm
    }

    /// The semi-axes (horizontal, vertical), in picture fractions, of the ellipse `distance = r`.
    pub fn semi_axes(&self, r: f64) -> (f64, f64) {
        (r * self.norm / (2.0 * self.stretch), r * self.norm / 2.0)
    }

    /// Where the `radius` or `softness` handle of an ellipse of radius `r` sits: on the horizontal
    /// axis through `center`.
    pub fn edge_point(&self, center: (f64, f64), r: f64) -> (f64, f64) {
        (center.0 + self.semi_axes(r).0, center.1)
    }

    /// Where `handle` is for `params`.
    pub fn handle_point(&self, params: &VignetteParams, handle: VignetteHandle) -> (f64, f64) {
        match handle {
            VignetteHandle::Center => params.center,
            VignetteHandle::Radius => self.edge_point(params.center, params.radius),
            VignetteHandle::Softness => {
                self.edge_point(params.center, params.radius + params.softness)
            }
        }
    }

    /// What a pointer at `at` makes of `handle`, on the grid of its control. `None` when `at` is
    /// not finite. `params` supplies what the other handles fix: the centre for both ellipses and
    /// the radius that `softness` is measured from.
    pub fn pointer_value(
        &self,
        params: &VignetteParams,
        handle: VignetteHandle,
        at: (f64, f64),
    ) -> Option<HandleValue> {
        if !(at.0.is_finite() && at.1.is_finite()) {
            return None;
        }
        Some(match handle {
            VignetteHandle::Center => {
                let (min, max) = center_range();
                HandleValue::Point {
                    x: quantize_control(at.0, min, max, CENTER_STEP, false),
                    y: quantize_control(at.1, min, max, CENTER_STEP, false),
                }
            }
            VignetteHandle::Radius => {
                let (min, max, step) = number_range(VIGNETTE_RADIUS);
                let raw = self.distance(params.center, at);
                HandleValue::Number(quantize_control(raw, min, max, step, false))
            }
            VignetteHandle::Softness => {
                let (min, max, step) = number_range(VIGNETTE_SOFTNESS);
                let raw = self.distance(params.center, at) - params.radius;
                HandleValue::Number(quantize_control(raw, min, max, step, false))
            }
        })
    }
}

/// The range and step of a number parameter of the vignette, from the catalog.
fn number_range(index: usize) -> (f64, f64, f64) {
    match VIGNETTE.params.get(index).map(|param| param.kind) {
        Some(ParamKind::Number { min, max, step, .. }) => (min, max, step),
        _ => (0.0, 1.0, 0.01),
    }
}

/// The range of both components of `center`, from the catalog.
fn center_range() -> (f64, f64) {
    match VIGNETTE.params.get(VIGNETTE_CENTER).map(|param| param.kind) {
        Some(ParamKind::Point { min, max, .. }) => (min, max),
        _ => (0.0, 1.0),
    }
}

/// `true` when `at` is on the picture (borders included): a handle off it is not drawn and cannot
/// be pressed.
pub fn on_picture(at: (f64, f64)) -> bool {
    (0.0..=1.0).contains(&at.0) && (0.0..=1.0).contains(&at.1)
}

/// The handle under `at` among `handles`, by distance in picture heights (`u` scaled by the aspect
/// of the picture) within [`HIT_RADIUS`]; the nearest wins, and a tie goes by
/// [`VignetteHandle::PRIORITY`].
pub fn hit(
    handles: &[(VignetteHandle, (f64, f64))],
    at: (f64, f64),
    aspect: f64,
) -> Option<VignetteHandle> {
    let mut best: Option<(f64, VignetteHandle)> = None;
    for handle in VignetteHandle::PRIORITY {
        let Some((_, point)) = handles.iter().find(|(candidate, _)| *candidate == handle) else {
            continue;
        };
        let distance = ((at.0 - point.0) * aspect).hypot(at.1 - point.1);
        if distance <= HIT_RADIUS && best.is_none_or(|(nearest, _)| distance < nearest) {
            best = Some((distance, handle));
        }
    }
    best.map(|(_, handle)| handle)
}

/// The parameters of a vignette as the clip holds them at one frame of the clip (keys applied):
/// `None` when the clip is not a vignette or a value has an unexpected type.
pub fn params_at(clip: &super::FxClip, frame_in_clip: i64) -> Option<VignetteParams> {
    if clip.effect.id != VIGNETTE.id {
        return None;
    }
    let number = |index: usize| match clip.value_at(index, frame_in_clip)? {
        FxValue::Number(value) => Some(value),
        _ => None,
    };
    let FxValue::Point { x, y } = clip.value_at(VIGNETTE_CENTER, frame_in_clip)? else {
        return None;
    };
    Some(VignetteParams {
        center: (x, y),
        radius: number(VIGNETTE_RADIUS)?,
        softness: number(VIGNETTE_SOFTNESS)?,
        roundness: number(super::VIGNETTE_ROUNDNESS)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    /// The shader's distance written the way the GLSL reads, in PIXELS: not a transcription of
    /// `VignetteShape::distance` (which works in fractions with `norm`/`stretch` precomputed).
    fn oracle_distance(w: f64, h: f64, rho: f64, centre: (f64, f64), p: (f64, f64)) -> f64 {
        let shape_x = (1.0 - rho) + rho * (w / h); // mix(1.0, w/h, rho)
        let shape = (shape_x, 1.0);
        let q = (p.0 * w, p.1 * h);
        let c = (centre.0 * w, centre.1 * h);
        let uv = (
            (q.0 / w - c.0 / w) * 2.0 * shape.0,
            (q.1 / h - c.1 / h) * 2.0 * shape.1,
        );
        let length = (uv.0 * uv.0 + uv.1 * uv.1).sqrt();
        let normaliser = (shape.0 * shape.0 + shape.1 * shape.1).sqrt().max(1.0);
        length / normaliser
    }

    /// Where on the horizontal axis through `centre` the oracle's distance reaches `r`, by
    /// bisection (the distance grows with `u` to the right of the centre): an algorithm of its
    /// own, not the closed form of the code under test.
    fn oracle_edge(w: f64, h: f64, rho: f64, centre: (f64, f64), r: f64) -> f64 {
        let (mut low, mut high) = (centre.0, centre.0 + 64.0);
        for _ in 0..200 {
            let mid = (low + high) / 2.0;
            if oracle_distance(w, h, rho, centre, (mid, centre.1)) < r {
                low = mid;
            } else {
                high = mid;
            }
        }
        (low + high) / 2.0
    }

    const SIZES: [(f64, f64); 3] = [(1920.0, 1080.0), (1080.0, 1080.0), (1080.0, 1920.0)];
    const ROUNDNESS: [f64; 3] = [0.0, 0.6, 1.0];

    #[test]
    fn the_handles_sit_where_an_independent_search_finds_the_ellipse() {
        for (w, h) in SIZES {
            for rho in ROUNDNESS {
                for (radius, softness) in [(0.75, 0.45), (0.3, 0.2), (1.2, 1.0), (0.05, 0.05)] {
                    let centre = (0.4, 0.55);
                    let shape = VignetteShape::new(w, h, rho).unwrap();
                    let params = VignetteParams {
                        center: centre,
                        radius,
                        softness,
                        roundness: rho,
                    };
                    let radius_at = shape.handle_point(&params, VignetteHandle::Radius);
                    let softness_at = shape.handle_point(&params, VignetteHandle::Softness);
                    assert!(
                        (radius_at.0 - oracle_edge(w, h, rho, centre, radius)).abs() < 1e-9,
                        "{w}x{h} rho {rho}"
                    );
                    assert!(
                        (softness_at.0 - oracle_edge(w, h, rho, centre, radius + softness)).abs()
                            < 1e-9,
                        "{w}x{h} rho {rho}"
                    );
                    assert_eq!(radius_at.1, centre.1);
                    assert_eq!(softness_at.1, centre.1);
                    assert_eq!(shape.handle_point(&params, VignetteHandle::Center), centre);
                }
            }
        }
    }

    #[test]
    fn the_semi_axes_describe_the_same_ellipse_the_distance_does() {
        for (w, h) in SIZES {
            for rho in ROUNDNESS {
                let shape = VignetteShape::new(w, h, rho).unwrap();
                let centre = (0.5, 0.5);
                let (a_u, a_v) = shape.semi_axes(0.8);
                // The four ends of the axes are on the ellipse of radius 0.8.
                for point in [
                    (centre.0 + a_u, centre.1),
                    (centre.0 - a_u, centre.1),
                    (centre.0, centre.1 + a_v),
                    (centre.0, centre.1 - a_v),
                ] {
                    assert!((oracle_distance(w, h, rho, centre, point) - 0.8).abs() < 1e-12);
                }
            }
        }
    }

    #[test]
    fn a_pointer_on_a_handle_gives_back_the_value_that_put_it_there() {
        for (w, h) in SIZES {
            for rho in ROUNDNESS {
                let shape = VignetteShape::new(w, h, rho).unwrap();
                for (cx, cy) in [(0.5, 0.5), (0.0, 0.0), (1.0, 1.0), (0.2, 0.7)] {
                    for (radius, softness) in [(0.0, 0.05), (0.5, 0.25), (1.5, 1.0), (0.77, 0.33)] {
                        let params = VignetteParams {
                            center: (cx, cy),
                            radius,
                            softness,
                            roundness: rho,
                        };
                        let at = shape.handle_point(&params, VignetteHandle::Radius);
                        assert_eq!(
                            shape.pointer_value(&params, VignetteHandle::Radius, at),
                            Some(HandleValue::Number(radius)),
                            "{w}x{h} rho {rho} centre {cx},{cy} radius {radius}"
                        );
                        let at = shape.handle_point(&params, VignetteHandle::Softness);
                        assert_eq!(
                            shape.pointer_value(&params, VignetteHandle::Softness, at),
                            Some(HandleValue::Number(softness)),
                            "{w}x{h} rho {rho} centre {cx},{cy} softness {softness}"
                        );
                        let at = shape.handle_point(&params, VignetteHandle::Center);
                        assert_eq!(
                            shape.pointer_value(&params, VignetteHandle::Center, at),
                            Some(HandleValue::Point { x: cx, y: cy })
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_value_is_put_on_the_grid_of_its_control_and_inside_its_range() {
        let shape = VignetteShape::new(1920.0, 1080.0, 0.6).unwrap();
        let params = VignetteParams {
            center: (0.5, 0.5),
            radius: 0.5,
            softness: 0.4,
            roundness: 0.6,
        };
        // Between two grid points of 0.01: the nearest one.
        let want = |text: f64| shape.edge_point(params.center, text);
        let at = want(0.7349);
        assert_eq!(
            shape.pointer_value(&params, VignetteHandle::Radius, at),
            Some(HandleValue::Number(0.73))
        );
        let at = want(0.7351);
        assert_eq!(
            shape.pointer_value(&params, VignetteHandle::Radius, at),
            Some(HandleValue::Number(0.74))
        );
        // Far past the picture: the end of the range, never a refusal.
        assert_eq!(
            shape.pointer_value(&params, VignetteHandle::Radius, (50.0, 0.5)),
            Some(HandleValue::Number(1.5))
        );
        // The centre of the vignette: radius 0.
        assert_eq!(
            shape.pointer_value(&params, VignetteHandle::Radius, (0.5, 0.5)),
            Some(HandleValue::Number(0.0))
        );
        // Softness is measured from the radius and starts at 0.05: inside the radius it is the minimum.
        assert_eq!(
            shape.pointer_value(&params, VignetteHandle::Softness, (0.5, 0.5)),
            Some(HandleValue::Number(0.05))
        );
        assert_eq!(
            shape.pointer_value(&params, VignetteHandle::Softness, (50.0, 0.5)),
            Some(HandleValue::Number(1.0))
        );
        // The centre is clamped to the picture and put on 0.01.
        assert_eq!(
            shape.pointer_value(&params, VignetteHandle::Center, (-3.0, 7.0)),
            Some(HandleValue::Point { x: 0.0, y: 1.0 })
        );
        assert_eq!(
            shape.pointer_value(&params, VignetteHandle::Center, (0.1234, 0.5678)),
            Some(HandleValue::Point { x: 0.12, y: 0.57 })
        );
        // Not a position: nothing to propose.
        assert_eq!(
            shape.pointer_value(&params, VignetteHandle::Radius, (f64::NAN, 0.5)),
            None
        );
        assert_eq!(
            shape.pointer_value(&params, VignetteHandle::Center, (0.5, f64::INFINITY)),
            None
        );
    }

    #[test]
    fn an_output_without_a_size_has_no_shape() {
        assert!(VignetteShape::new(0.0, 1080.0, 0.5).is_none());
        assert!(VignetteShape::new(1920.0, -1.0, 0.5).is_none());
        assert!(VignetteShape::new(f64::NAN, 1080.0, 0.5).is_none());
        assert!(VignetteShape::new(1920.0, 1080.0, f64::NAN).is_none());
    }

    #[test]
    fn the_catalog_ranges_are_the_ones_the_geometry_uses() {
        assert_eq!(number_range(VIGNETTE_RADIUS), (0.0, 1.5, 0.01));
        assert_eq!(number_range(VIGNETTE_SOFTNESS), (0.05, 1.0, 0.01));
        assert_eq!(center_range(), (0.0, 1.0));
    }

    #[test]
    fn the_nearest_handle_in_picture_heights_wins_and_a_tie_goes_to_the_outer_one() {
        let aspect = 16.0 / 9.0;
        let handles = [
            (VignetteHandle::Center, (0.5, 0.5)),
            (VignetteHandle::Radius, (0.7, 0.5)),
            (VignetteHandle::Softness, (0.9, 0.5)),
        ];
        assert_eq!(
            hit(&handles, (0.5, 0.5), aspect),
            Some(VignetteHandle::Center)
        );
        assert_eq!(
            hit(&handles, (0.7, 0.51), aspect),
            Some(VignetteHandle::Radius)
        );
        assert_eq!(
            hit(&handles, (0.9, 0.5), aspect),
            Some(VignetteHandle::Softness)
        );
        // The radius in `u` is scaled by the aspect: 0.02 of the width is 0.036 of the height.
        assert_eq!(hit(&handles, (0.72, 0.5), aspect), None);
        assert_eq!(hit(&handles, (0.1, 0.1), aspect), None);
        // Coincident handles (radius 0): the radius one, not the centre.
        let stacked = [
            (VignetteHandle::Center, (0.5, 0.5)),
            (VignetteHandle::Radius, (0.5, 0.5)),
        ];
        assert_eq!(
            hit(&stacked, (0.5, 0.5), aspect),
            Some(VignetteHandle::Radius)
        );
        // A handle that is not offered (off the picture, animated) is never hit.
        assert_eq!(hit(&handles[..1], (0.7, 0.5), aspect), None);
    }

    #[test]
    fn handles_are_on_the_picture_inside_its_border_only() {
        assert!(on_picture((0.0, 1.0)));
        assert!(on_picture((0.5, 0.5)));
        assert!(!on_picture((1.0001, 0.5)));
        assert!(!on_picture((0.5, -0.0001)));
    }

    #[test]
    fn handle_names_are_the_parameter_names() {
        for handle in VignetteHandle::ALL {
            assert_eq!(VignetteHandle::parse(handle.as_str()), Some(handle));
            assert!(VIGNETTE.param(handle.param()).is_some());
        }
        assert_eq!(VignetteHandle::parse("roundness"), None);
    }
}
