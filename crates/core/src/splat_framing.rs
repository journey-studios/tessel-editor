//! Framing of a bare splat for its library thumbnail (BL144.6): the rule, none of the pixels.
//!
//! A `.splat`, PLY or SPZ file has no camera. Drawn from the default scene camera
//! (`[0,1,5]` looking at the origin) a cloud that is displaced, or at the real scale of a
//! SHARP capture (metres, far from the origin), is outside the frustum and the picture is only
//! the background. So the adapter measures the file ([`crate::splat::measure`]) and this module
//! decides, from that box alone:
//!
//! - **where the cloud goes**: the scene object that holds it is moved and scaled so the bulk of
//!   the splats is a sphere of radius 1 at the origin ([`Framing::position`], [`Framing::scale`]).
//!   Normalising also keeps any cloud between the camera's near and far planes, whatever its
//!   real scale;
//! - **where the camera stands**: on the direction of [`VIEW`], far enough that the unit sphere
//!   fits the picture with [`MARGIN`], by the narrower of the vertical and horizontal fields of view.
//!
//! The runtime is not asked to guess anything: it draws the scene file and the camera it is
//! handed. The view direction is a parameter ([`fit`]'s `view`): the orientation of a SHARP
//! capture (PR #23, BL144.2) will arrive there, and the identity of a thumbnail already names it
//! (`view=default`), so a capture changes the picture's identity the day it is wired.
//!
//! A cloud that cannot be framed is [`Undrawable`], never drawn at a guess; and a picture that
//! came out as background only is detected from the frame the renderer reports ([`is_blank`]).

use crate::content::Camera;
use crate::splat::Extent;

/// Vertical field of view of the thumbnail camera, in degrees (the default scene camera's).
pub const FOV: f64 = 50.0;
/// The unit sphere fills this fraction of the narrower field of view, inverted: `1.15` leaves 15 %.
pub const MARGIN: f64 = 1.15;
/// From the target to the camera: the tilt of the default scene camera, `[0,1,5]`.
pub const VIEW: [f64; 3] = [0.0, 1.0, 5.0];
/// Smallest and largest bulk radius (file units) that can be framed.
pub const MIN_RADIUS: f64 = 1e-6;
pub const MAX_RADIUS: f64 = 1e9;
/// The background of the stand-in scene, as the scene file says it (0..1) and as it reads back
/// from a frame (8 bits).
pub const BACKGROUND_RGBA: [f64; 4] = [0.04, 0.05, 0.1, 1.0];
pub const BACKGROUND_8BIT: [u8; 3] = [10, 13, 26];

/// What the stand-in scene and the plan's camera are made with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Framing {
    /// `position` of the splat object in the stand-in scene.
    pub position: [f64; 3],
    /// Uniform `scale` of the splat object.
    pub scale: f64,
    /// The camera of the plan's scene layer: looks at the origin.
    pub camera: Camera,
    /// What was measured, kept for reports and tests: centre and radius in file units.
    pub center: [f64; 3],
    pub radius: f64,
}

/// Why a cloud is not drawn at all.
#[derive(Debug, Clone, PartialEq)]
pub struct Undrawable {
    pub code: &'static str,
    pub message: String,
}

impl Undrawable {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Frames the bulk of `extent` for a picture of `aspect` (width / height). `view` is the
/// direction from the cloud to the camera; `None` is [`VIEW`].
pub fn fit(extent: &Extent, aspect: f64, view: Option<[f64; 3]>) -> Result<Framing, Undrawable> {
    let center = extent.center();
    let radius = extent.radius();
    if !radius.is_finite() || center.iter().any(|value| !value.is_finite()) {
        return Err(Undrawable::new(
            "splat_not_drawable",
            "The cloud has no finite extent",
        ));
    }
    if radius < MIN_RADIUS {
        return Err(Undrawable::new(
            "splat_not_drawable",
            format!(
                "The splats are all at one point (radius {radius:e}); there is nothing to frame"
            ),
        ));
    }
    if radius > MAX_RADIUS {
        return Err(Undrawable::new(
            "splat_not_drawable",
            format!("The cloud is {radius:e} units across, beyond what can be framed"),
        ));
    }
    if !aspect.is_finite() || aspect <= 0.0 {
        return Err(Undrawable::new(
            "splat_not_drawable",
            "The picture has no aspect ratio",
        ));
    }
    let direction = view.unwrap_or(VIEW);
    let length = direction
        .iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    if !length.is_finite() || length < 1e-9 {
        return Err(Undrawable::new(
            "splat_not_drawable",
            "The view direction has no length",
        ));
    }
    let vertical = (FOV / 2.0).to_radians();
    let horizontal = (vertical.tan() * aspect).atan();
    let distance = MARGIN / vertical.min(horizontal).sin();
    let scale = 1.0 / radius;
    let camera = Camera {
        position: direction.map(|value| value / length * distance),
        target: [0.0; 3],
        fov: FOV,
        orbit: 0.0,
    };
    camera.validate().map_err(|error| {
        Undrawable::new(
            "splat_not_drawable",
            format!("The view cannot frame the cloud: {}", error.message),
        )
    })?;
    Ok(Framing {
        position: center.map(|value| -value * scale),
        scale,
        camera,
        center,
        radius,
    })
}

/// Pixels a frame is asked to report so [`is_blank`] can look at it: a 5 × 5 grid inside the
/// picture.
pub fn probes(width: u32, height: u32) -> Vec<(u32, u32)> {
    let at = |size: u32, step: u32| ((u64::from(size) * u64::from(2 * step + 1)) / 10) as u32;
    (0..5)
        .flat_map(|row| (0..5).map(move |column| (column, row)))
        .map(|(column, row)| {
            (
                at(width, column).min(width - 1),
                at(height, row).min(height - 1),
            )
        })
        .collect()
}

/// Whether a frame is the background and nothing else: the mean colour of the whole picture and
/// every probe are the background's. A picture of a cloud that fell outside the frustum looks
/// exactly like that, and must not be offered as the thumbnail.
pub fn is_blank(mean: [f64; 4], probes: &[[u8; 4]]) -> bool {
    let flat =
        (0..3).all(|channel| (mean[channel] - f64::from(BACKGROUND_8BIT[channel])).abs() <= 0.3);
    let still = probes.iter().all(|probe| {
        (0..3).all(|channel| {
            (i32::from(probe[channel]) - i32::from(BACKGROUND_8BIT[channel])).abs() <= 12
        })
    });
    flat && still
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::splat::SplatFormat;
    use core::prelude::v1::test;

    fn extent(min: [f64; 3], max: [f64; 3]) -> Extent {
        Extent {
            format: SplatFormat::Splat,
            count: 100,
            measured: 100,
            min,
            max,
        }
    }

    /// Pixel of `point` seen from `camera` on a `width` × `height` picture, with a plain pinhole
    /// (the plan's camera looks at its target with +Y up), or `None` behind the camera.
    fn project(camera: &Camera, point: [f64; 3], width: f64, height: f64) -> Option<(f64, f64)> {
        let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let cross = |a: [f64; 3], b: [f64; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let unit = |a: [f64; 3]| {
            let length = dot(a, a).sqrt();
            [a[0] / length, a[1] / length, a[2] / length]
        };
        let forward = unit(sub(camera.target, camera.position));
        let right = unit(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        let offset = sub(point, camera.position);
        let depth = dot(offset, forward);
        if depth <= 0.0 {
            return None;
        }
        let focal = height / 2.0 / (camera.fov / 2.0).to_radians().tan();
        Some((
            width / 2.0 + focal * dot(offset, right) / depth,
            height / 2.0 - focal * dot(offset, up) / depth,
        ))
    }

    /// Where the box of `extent` lands once the stand-in scene places it: a splat object at
    /// `position` with `scale` puts a file point `p` at `p * scale + position`.
    fn placed(framing: &Framing, point: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|axis| point[axis] * framing.scale + framing.position[axis])
    }

    #[test]
    fn a_cloud_far_from_the_origin_at_any_scale_lands_inside_the_picture() {
        for (min, max) in [
            ([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]),
            ([996.0, -54.0, 290.0], [1004.0, -46.0, 310.0]),
            ([-0.002, 0.098, 4.998], [0.002, 0.102, 5.002]),
            ([-4e6, 1e6, -2e6], [4e6, 3e6, 2e6]),
        ] {
            for (width, height) in [(240.0, 136.0), (136.0, 240.0), (240.0, 240.0)] {
                let measured = extent(min, max);
                let framing = fit(&measured, width / height, None).unwrap();
                assert!((framing.scale * framing.radius - 1.0).abs() < 1e-12);
                // The centre of the box is the centre of the picture.
                let middle = project(
                    &framing.camera,
                    placed(&framing, measured.center()),
                    width,
                    height,
                )
                .unwrap();
                assert!(
                    (middle.0 - width / 2.0).abs() < 1e-6 && (middle.1 - height / 2.0).abs() < 1e-6,
                    "{min:?}: {middle:?}"
                );
                // All eight corners of the box are inside the picture.
                for corner in 0..8 {
                    let point: [f64; 3] = std::array::from_fn(|axis| {
                        if (corner >> axis) & 1 == 1 {
                            max[axis]
                        } else {
                            min[axis]
                        }
                    });
                    let (x, y) = project(&framing.camera, placed(&framing, point), width, height)
                        .unwrap_or_else(|| panic!("{point:?} is behind the camera"));
                    assert!(
                        (0.0..width).contains(&x) && (0.0..height).contains(&y),
                        "corner {point:?} of {min:?}..{max:?} lands at ({x}, {y}) in {width}x{height}"
                    );
                }
                // Between the camera's near and far planes (0.05 and 1000), whatever the scale.
                let distance = framing
                    .camera
                    .position
                    .iter()
                    .map(|v| v * v)
                    .sum::<f64>()
                    .sqrt();
                assert!((0.05..900.0).contains(&distance), "{distance}");
            }
        }
    }

    #[test]
    fn a_cloud_that_cannot_be_framed_is_named_not_drawn() {
        let flat = fit(&extent([3.0; 3], [3.0; 3]), 1.5, None).unwrap_err();
        assert_eq!(flat.code, "splat_not_drawable");
        assert!(flat.message.contains("one point"), "{}", flat.message);
        assert!(
            fit(&extent([0.0; 3], [1e12; 3]), 1.5, None)
                .unwrap_err()
                .message
                .contains("beyond")
        );
        assert!(fit(&extent([f64::NAN; 3], [1.0; 3]), 1.5, None).is_err());
        assert!(fit(&extent([-1.0; 3], [1.0; 3]), 0.0, None).is_err());
        // A view straight along Y makes the camera's fixed up vector degenerate.
        let along_y = fit(&extent([-1.0; 3], [1.0; 3]), 1.5, Some([0.0, 1.0, 0.0])).unwrap_err();
        assert!(
            along_y.message.contains("cannot frame"),
            "{}",
            along_y.message
        );
        assert!(fit(&extent([-1.0; 3], [1.0; 3]), 1.5, Some([0.0; 3])).is_err());
    }

    #[test]
    fn a_given_view_turns_the_camera_and_keeps_the_cloud_in_frame() {
        let measured = extent([10.0; 3], [14.0; 3]);
        let default = fit(&measured, 1.7, None).unwrap();
        let side = fit(&measured, 1.7, Some([5.0, 0.5, 0.0])).unwrap();
        assert_ne!(default.camera.position, side.camera.position);
        assert!(side.camera.position[0] > 1.0 && side.camera.position[2].abs() < 1e-9);
        let length = |camera: &Camera| camera.position.iter().map(|v| v * v).sum::<f64>().sqrt();
        assert!(
            (length(&default.camera) - length(&side.camera)).abs() < 1e-9,
            "the same distance from any side"
        );
    }

    #[test]
    fn blank_means_the_background_and_nothing_else() {
        let background = [10.0, 13.0, 26.0, 255.0];
        let probes = vec![[10, 13, 26, 255]; 25];
        assert!(is_blank(background, &probes));
        // One probe on a splat is not blank.
        let mut one = probes.clone();
        one[12] = [200, 40, 40, 255];
        assert!(!is_blank(background, &one));
        // A mean that moved with every probe on the background (a thin cloud) is not blank.
        assert!(!is_blank([12.0, 13.0, 26.0, 255.0], &probes));
        // A black frame is not the stand-in's background: a failed draw is not "empty cloud".
        assert!(!is_blank([0.0; 4], &[]));
    }

    #[test]
    fn the_probe_grid_stays_inside_any_picture() {
        for (width, height) in [(240, 136), (2, 2), (1, 1), (240, 426)] {
            let grid = probes(width, height);
            assert_eq!(grid.len(), 25);
            assert!(
                grid.iter().all(|&(x, y)| x < width && y < height),
                "{width}x{height}"
            );
        }
        assert_eq!(
            probes(100, 100)[12],
            (50, 50),
            "the middle probe is the middle"
        );
    }
}
