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
//! handed. The view direction is a parameter ([`fit`]'s `view`).
//!
//! **A SHARP capture is not framed, it is looked through** ([`fit_capture`], BL144.2). The file
//! carries the camera that took its source image (`sharp::Capture`): the cloud is held at its
//! real position and scale, turned once by [`crate::sharp::ORIENTATION`] (the stand-in scene's
//! `rotation`, the same half turn the plan gives a clip of that splat), and the camera is the
//! capture's own (`Capture::camera`: at the origin, looking down -z, with the vertical field of
//! view of the intrinsics). The thumbnail is then the source picture, upright, the way the clip
//! preview shows it. The stand-in is a *scene*, whose plan layer has no orientation, so the turn
//! exists in exactly one place. [`identity_token`] names the capture, so another capture is
//! another picture.
//!
//! A cloud that cannot be framed is [`Undrawable`], never drawn at a guess; and a picture that
//! came out as background only is detected from the frame the renderer reports ([`is_blank`]).

use crate::content::Camera;
use crate::sharp::Capture;
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
    /// `rotation` of the splat object, degrees around X, Y, Z (`[0; 3]` unless a capture turns it).
    pub rotation: [f64; 3],
    /// The camera of the plan's scene layer: looks at the origin, or is the capture's.
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

/// The centre and radius of `extent` when it can be drawn at all, for a picture of `aspect`.
fn measured(extent: &Extent, aspect: f64) -> Result<([f64; 3], f64), Undrawable> {
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
    Ok((center, radius))
}

/// Frames the bulk of `extent` for a picture of `aspect` (width / height). `view` is the
/// direction from the cloud to the camera; `None` is [`VIEW`]. The cloud is not turned.
pub fn fit(extent: &Extent, aspect: f64, view: Option<[f64; 3]>) -> Result<Framing, Undrawable> {
    let (center, radius) = measured(extent, aspect)?;
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
        rotation: [0.0; 3],
        camera,
        center,
        radius,
    })
}

/// Looks through the camera of a SHARP `capture` (BL144.2): the cloud stays where the file puts it
/// (`position` 0, `scale` 1) and is turned once by [`crate::sharp::ORIENTATION`] (the file's y-down,
/// z-forward axes into the editor's y-up, -z-forward); the camera is [`Capture::camera`] with
/// its vertical field of view. A picture of another aspect than the source image keeps that
/// vertical field (the lens is the same, the crop is not), as the clip preview does.
///
/// Nothing is guessed: a cloud whose bulk lies entirely behind the capture camera (`z <= 0` in
/// the file's axes) is [`Undrawable`], never drawn from some other side.
pub fn fit_capture(extent: &Extent, capture: &Capture, aspect: f64) -> Result<Framing, Undrawable> {
    let (center, radius) = measured(extent, aspect)?;
    if extent.max[2] <= 0.0 {
        return Err(Undrawable::new(
            "splat_not_drawable",
            format!(
                "The whole cloud is behind the capture camera (its farthest point is at z {} in the \
                 file's forward axis); it cannot be drawn through that camera",
                extent.max[2]
            ),
        ));
    }
    let camera = capture.camera();
    camera.validate().map_err(|error| {
        Undrawable::new(
            "splat_not_drawable",
            format!("The capture camera is not usable: {}", error.message),
        )
    })?;
    Ok(Framing {
        position: [0.0; 3],
        scale: 1.0,
        rotation: crate::sharp::ORIENTATION,
        camera,
        center,
        radius,
    })
}

/// What a thumbnail's identity says about how a splat is framed: `fit:default` without a capture
/// (the common framing, unchanged), and with one the capture's image size, focal lengths and
/// principal point plus the version of this rule (`cv1`), so another capture, or the same file
/// read without one (`capture: ignore`), is another picture and the cache never serves the old one.
pub fn identity_token(capture: Option<&Capture>) -> String {
    match capture {
        None => "fit:default".to_string(),
        Some(capture) => format!(
            "capture:cv1:{}x{}:f={},{}:c={},{}",
            capture.image[0], capture.image[1], capture.fx, capture.fy, capture.cx, capture.cy
        ),
    }
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

    /// A capture of a 160 x 120 picture with a 50 degree vertical field of view.
    fn capture() -> Capture {
        let fy = 60.0 / (25f64.to_radians()).tan();
        Capture {
            image: [160, 120],
            fx: fy,
            fy,
            cx: 80.0,
            cy: 60.0,
            vfov: 50.0,
            color_space: crate::sharp::ColorSpace::Srgb,
            color_space_declared: true,
            version: Some([1, 5, 0]),
        }
    }

    /// Turns `point` by `degrees` around X (the only axis a capture turns). The stand-in scene
    /// applies `position + R * (scale * p)`; for a pure X turn that is this.
    fn turn_x(point: [f64; 3], degrees: f64) -> [f64; 3] {
        let (sin, cos) = degrees.to_radians().sin_cos();
        [
            point[0],
            point[1] * cos - point[2] * sin,
            point[1] * sin + point[2] * cos,
        ]
    }

    /// Where the stand-in scene puts a file point: `position + R * (scale * p)`.
    fn placed_turned(framing: &Framing, point: [f64; 3]) -> [f64; 3] {
        assert_eq!(framing.rotation[1], 0.0);
        assert_eq!(framing.rotation[2], 0.0);
        let turned = turn_x(
            point.map(|value| value * framing.scale),
            framing.rotation[0],
        );
        std::array::from_fn(|axis| turned[axis] + framing.position[axis])
    }

    /// Points of a capture seen from a file's axes (x right, y down, z forward), on pixels of the
    /// 160 x 120 source image that are not symmetric about either axis, at different depths.
    fn source_points(capture: &Capture) -> Vec<([f64; 3], [f64; 2])> {
        [
            ([30.0, 20.0], 3.2),
            ([130.0, 45.0], 6.0),
            ([55.0, 100.0], 2.5),
            ([140.0, 110.0], 9.0),
            ([80.0, 60.0], 4.0),
        ]
        .into_iter()
        .map(|(pixel, depth): ([f64; 2], f64)| {
            let point = [
                (pixel[0] - capture.cx) * depth / capture.fx,
                (pixel[1] - capture.cy) * depth / capture.fy,
                depth,
            ];
            (point, pixel)
        })
        .collect()
    }

    #[test]
    fn a_capture_is_seen_through_its_own_camera_with_one_turn() {
        let capture = capture();
        let measured = extent([-3.0, -2.0, 2.0], [4.0, 3.0, 9.0]);
        for (width, height) in [(240.0, 180.0), (240.0, 136.0), (136.0, 240.0)] {
            let framing = fit_capture(&measured, &capture, width / height).unwrap();
            assert_eq!(framing.rotation, crate::sharp::ORIENTATION);
            assert_eq!((framing.scale, framing.position), (1.0, [0.0; 3]));
            assert_eq!(framing.camera, capture.camera());
            assert_eq!(framing.camera.fov, capture.vfov);
            // The lens is the same whatever the crop: a point over source pixel (u, v) lands
            // where that pixel does in the source picture scaled to the same height, centred.
            let scale = height / 120.0;
            for (point, pixel) in source_points(&capture) {
                let (x, y) = project(
                    &framing.camera,
                    placed_turned(&framing, point),
                    width,
                    height,
                )
                .unwrap_or_else(|| panic!("{point:?} is behind the camera"));
                let expected = (
                    width / 2.0 + (pixel[0] - 80.0) * scale,
                    height / 2.0 + (pixel[1] - 60.0) * scale,
                );
                assert!(
                    (x - expected.0).abs() < 1e-6 && (y - expected.1).abs() < 1e-6,
                    "{point:?} over {pixel:?} lands at ({x}, {y}), expected {expected:?} ({width}x{height})"
                );
            }
        }
    }

    #[test]
    fn without_the_turn_or_with_it_twice_the_cloud_is_not_where_the_source_had_it() {
        let capture = capture();
        let framing = fit_capture(
            &extent([-3.0, -2.0, 2.0], [4.0, 3.0, 9.0]),
            &capture,
            4.0 / 3.0,
        )
        .unwrap();
        for (point, _) in source_points(&capture) {
            // No turn: the file's forward axis is +z, behind a camera that looks down -z.
            assert!(project(&framing.camera, point, 240.0, 180.0).is_none());
            // Two half turns are no turn at all.
            let twice = turn_x(placed_turned(&framing, point), framing.rotation[0]);
            assert!(project(&framing.camera, twice, 240.0, 180.0).is_none());
        }
        // The turn is what puts the picture upright: a point above the middle of the source
        // image (pixel y < 60, so negative y in the file) is above the middle of the thumbnail.
        let above = [0.0, -1.0, 4.0];
        let (_, y) = project(
            &framing.camera,
            placed_turned(&framing, above),
            240.0,
            180.0,
        )
        .unwrap();
        assert!(y < 90.0, "{y}");
        let left = [-1.0, 0.0, 4.0];
        let (x, _) = project(&framing.camera, placed_turned(&framing, left), 240.0, 180.0).unwrap();
        assert!(x < 120.0, "{x}");
    }

    #[test]
    fn a_cloud_behind_the_capture_camera_is_named_not_drawn() {
        let capture = capture();
        let behind =
            fit_capture(&extent([-1.0, -1.0, -9.0], [1.0, 1.0, -2.0]), &capture, 1.5).unwrap_err();
        assert_eq!(behind.code, "splat_not_drawable");
        assert!(behind.message.contains("behind"), "{}", behind.message);
        // Partly in front is drawn.
        assert!(fit_capture(&extent([-1.0, -1.0, -2.0], [1.0, 1.0, 5.0]), &capture, 1.5).is_ok());
        assert!(fit_capture(&extent([f64::NAN; 3], [1.0; 3]), &capture, 1.5).is_err());
        assert!(fit_capture(&extent([0.0; 3], [1.0; 3]), &capture, 0.0).is_err());
        let mut absurd = capture;
        absurd.vfov = 175.0;
        let error = fit_capture(&extent([0.0, 0.0, 1.0], [1.0; 3]), &absurd, 1.5).unwrap_err();
        assert!(
            error.message.contains("capture camera"),
            "{}",
            error.message
        );
    }

    #[test]
    fn without_a_capture_the_common_framing_is_unchanged() {
        let framing = fit(&extent([10.0; 3], [14.0; 3]), 1.7, None).unwrap();
        assert_eq!(framing.rotation, [0.0; 3]);
        assert!((framing.scale * framing.radius - 1.0).abs() < 1e-12);
        assert_eq!(framing.camera.fov, FOV);
        // The identity of a plain splat is what it was before captures existed.
        assert_eq!(identity_token(None), "fit:default");
    }

    #[test]
    fn the_identity_names_the_capture_and_changes_with_any_part_of_it() {
        let base = capture();
        let token = identity_token(Some(&base));
        assert_ne!(token, identity_token(None));
        assert_eq!(token, identity_token(Some(&base)), "stable");
        for changed in [
            Capture {
                image: [320, 240],
                ..base
            },
            Capture {
                fx: base.fx + 0.5,
                ..base
            },
            Capture {
                fy: base.fy + 0.5,
                ..base
            },
            Capture {
                cx: base.cx + 0.5,
                ..base
            },
            Capture {
                cy: base.cy - 0.5,
                ..base
            },
        ] {
            assert_ne!(identity_token(Some(&changed)), token, "{changed:?}");
        }
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
