//! The reference of what a pass does to a picture, written independently of the runtime's GLSL.
//!
//! It is the specification the pixels of the looks are compared with (the proof compares frames the
//! runtime drew against this function, not against another render). It follows the pilot's shaders
//! (`runtime/fx/shaders/vignette.ts`, `fade.ts`): the picture is decoded sRGB → linear light, mixed
//! with the colour by `strength × amount` and encoded again, where `strength` is the vignette's
//! `shade` or the fade's `amount` parameter. A pass whose total weight is 0 returns its input
//! exactly (no decode/encode round trip, so zero intensity or zero fade amount cannot move a pixel).
//!
//! `filmGrain` (P3) is different in kind: it adds noise from the integer hash of `(seed, cell)` to
//! the **encoded** picture (see [`apply_grain_to_srgb`]) and mixes the result back in linear light.

use super::{FxPass, FxValue, colored_triangular, hash_pixel, triangular};

/// The height at which a grain `size` is declared in pixels (the pilot's `REFERENCE_HEIGHT`).
pub const GRAIN_REFERENCE_HEIGHT: f64 = 720.0;

/// The effects this reference draws.
fn knows(effect: &str) -> bool {
    matches!(effect, "vignette" | "fade" | "filmGrain")
}

/// The side of one grain cell, in pixels of a picture `height` pixels tall:
/// `max(size × height / 720, 1)`, applied **once** (the pilot's `pxScale`).
pub fn grain_cell_px(size: f64, height: usize) -> f64 {
    (size * (height as f64 / GRAIN_REFERENCE_HEIGHT)).max(1.0)
}

/// How much a pass mixes toward its colour at a point, before `amount`: the vignette's shade, the
/// fade's `amount` parameter (the same everywhere). `None` for an effect this reference does not
/// know or a pass without its parameters.
pub fn pass_strength(pass: &FxPass, u: f64, v: f64, width: f64, height: f64) -> Option<f64> {
    // The film grain has no scalar strength: it adds noise, it does not mix toward a colour.
    match pass.effect.as_str() {
        "vignette" => vignette_shade(pass, u, v, width, height),
        "fade" => pass.number("amount"),
        _ => None,
    }
}

/// `smoothstep(edge0, edge1, x)` as GLSL defines it.
pub fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// sRGB-encoded 0..1 → linear light (the IEC 61966-2-1 curve).
pub fn srgb_to_linear(encoded: f64) -> f64 {
    if encoded <= 0.040_45 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light → sRGB-encoded 0..1; the input is clamped to 0..1.
pub fn linear_to_srgb(linear: f64) -> f64 {
    let linear = linear.clamp(0.0, 1.0);
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

/// How dark the vignette is at a point: 0 (untouched) .. 1 (fully the colour, before `amount`).
/// `u`, `v` are picture fractions, `v` measured DOWN from the top row (the plan's `center` has
/// the same convention), and `width`/`height` the picture size in pixels.
///
/// The distance is `|(p - center) × 2 × shape| / max(|shape|, 1)` with
/// `shape = (mix(1, width/height, roundness), 1)`, then `smoothstep(radius, radius + softness, d)`.
pub fn vignette_shade(pass: &FxPass, u: f64, v: f64, width: f64, height: f64) -> Option<f64> {
    let radius = pass.number("radius")?;
    let softness = pass.number("softness")?;
    let roundness = pass.number("roundness")?;
    let FxValue::Point { x: cx, y: cy } = pass.param("center")? else {
        return None;
    };
    let stretch = 1.0 + (width / height - 1.0) * roundness;
    let shape = (stretch, 1.0);
    // The shader works with `y` up; the plan states `center` with `y` down.
    let dx = (u - cx) * 2.0 * shape.0;
    let dy = ((1.0 - v) - (1.0 - cy)) * 2.0 * shape.1;
    let norm = (shape.0 * shape.0 + shape.1 * shape.1).sqrt().max(1.0);
    let distance = (dx * dx + dy * dy).sqrt() / norm;
    Some(smoothstep(radius, radius + softness, distance))
}

/// The noise one pixel receives from a grain pass, before it is weighed: the triangular draw(s)
/// of the cell the pixel is in. `(x, y)` count from the **top-left**; the shader's
/// `gl_FragCoord` counts from the **bottom-left**, so the row is converted explicitly
/// (`glY = height − 1 − y`, the pixel's centre at `glY + 0.5`) and the cell is
/// `floor(coord / max(cellPx, 1))` in single precision as the GPU computes it.
pub fn grain_encoded_delta(pass: &FxPass, x: usize, y: usize, height: usize) -> Option<[f32; 3]> {
    let seed = pass.seed?;
    let cell = grain_cell_px(pass.number("size")?, height) as f32;
    let gl_y = height.checked_sub(1)?.checked_sub(y)?;
    let cell_x = ((x as f32 + 0.5) / cell).floor() as u32;
    let cell_y = ((gl_y as f32 + 0.5) / cell).floor() as u32;
    let hash = hash_pixel(seed, cell_x, cell_y);
    Some(if pass.boolean("colored")? {
        colored_triangular(hash)
    } else {
        [triangular(hash); 3]
    })
}

/// One pixel of a grain pass (the pilot's `filmGrain` shader, in the editor's encoded textures):
/// the grain is added to the sRGB-**encoded** pixel, weighed by `amount × 0.25 × response` where
/// `response = mix(1, 1 − smoothstep(0.05, 0.95, luma), luma_param)` (`luma` is Rec.709 on the
/// encoded colour), clamped, decoded to linear light and mixed with the source by the pass's
/// `amount` (= `intensity × weight`, **once**); the result is encoded again. Exactly the identity
/// when `amount × pass.amount <= 0`.
pub fn apply_grain_to_srgb(
    pass: &FxPass,
    rgb: [f64; 3],
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Option<[f64; 3]> {
    let _ = width;
    let grain = pass.number("amount")?;
    let luma_weight = pass.number("luma")?;
    if grain * pass.amount <= 0.0 {
        return Some(rgb);
    }
    let noise = grain_encoded_delta(pass, x, y, height)?;
    let encoded = [
        rgb[0].clamp(0.0, 1.0),
        rgb[1].clamp(0.0, 1.0),
        rgb[2].clamp(0.0, 1.0),
    ];
    let luma = 0.2126 * encoded[0] + 0.7152 * encoded[1] + 0.0722 * encoded[2];
    let response = 1.0 + ((1.0 - smoothstep(0.05, 0.95, luma)) - 1.0) * luma_weight;
    let mut out = [0.0; 3];
    for channel in 0..3 {
        let grained = (encoded[channel] + f64::from(noise[channel]) * (grain * 0.25 * response))
            .clamp(0.0, 1.0);
        let linear = srgb_to_linear(encoded[channel]);
        let mixed = linear + (srgb_to_linear(grained) - linear) * pass.amount;
        out[channel] = linear_to_srgb(mixed);
    }
    Some(out)
}

/// One pass over one pixel: `rgb` is sRGB-encoded 0..1, `(x, y)` the pixel's column and row from
/// the top-left of a `width` × `height` picture (the pixel's centre is sampled). Returns the
/// encoded result, or `None` for an effect this reference does not know.
pub fn apply_pass_to_srgb(
    pass: &FxPass,
    rgb: [f64; 3],
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Option<[f64; 3]> {
    if !knows(&pass.effect) {
        return None;
    }
    if pass.amount <= 0.0 {
        return Some(rgb);
    }
    if pass.effect == "filmGrain" {
        return apply_grain_to_srgb(pass, rgb, x, y, width, height);
    }
    let u = (x as f64 + 0.5) / width as f64;
    let v = (y as f64 + 0.5) / height as f64;
    let strength = pass_strength(pass, u, v, width as f64, height as f64)?;
    let FxValue::Color { r, g, b } = pass.param("color")? else {
        return None;
    };
    let weight = strength * pass.amount;
    if weight <= 0.0 {
        // The pilot's fade is "exactly the identity at 0": no round trip through linear light.
        return Some(rgb);
    }
    let mix = |source: f64, color: f64| {
        let linear = srgb_to_linear(source);
        linear_to_srgb(linear + (color - linear) * weight)
    };
    Some([mix(rgb[0], r), mix(rgb[1], g), mix(rgb[2], b)])
}

/// The pass over a whole RGBA8 picture (row 0 on top, alpha untouched). `false` when the effect
/// is unknown or the buffer is not `width × height × 4` bytes; the buffer is then left alone.
pub fn apply_pass_to_rgba8(pass: &FxPass, rgba: &mut [u8], width: usize, height: usize) -> bool {
    if rgba.len() != width * height * 4 || !knows(&pass.effect) {
        return false;
    }
    if pass.amount <= 0.0 {
        return true;
    }
    for y in 0..height {
        for x in 0..width {
            let at = (y * width + x) * 4;
            let source = [
                f64::from(rgba[at]) / 255.0,
                f64::from(rgba[at + 1]) / 255.0,
                f64::from(rgba[at + 2]) / 255.0,
            ];
            let Some(out) = apply_pass_to_srgb(pass, source, x, y, width, height) else {
                return false;
            };
            for (channel, value) in out.iter().enumerate() {
                rgba[at + channel] = (value * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;
    use crate::directed_scene::fx::FxScope;
    use crate::directed_scene::fx::catalog::VIGNETTE;

    fn pass_with(overrides: &[(&str, FxValue)]) -> FxPass {
        pass(overrides, 1.0)
    }

    fn pass(overrides: &[(&str, FxValue)], amount: f64) -> FxPass {
        let params = VIGNETTE
            .params
            .iter()
            .map(|spec| {
                let value = overrides
                    .iter()
                    .find(|(name, _)| *name == spec.id)
                    .map_or_else(|| spec.kind.default_value(), |(_, value)| *value);
                (spec.id.to_string(), value)
            })
            .collect();
        FxPass {
            track_id: "t".into(),
            clip_id: "c".into(),
            effect: "vignette".into(),
            frame_in_clip: 0,
            weight: 1.0,
            amount,
            params,
            seed: None,
            scope: FxScope::Output,
        }
    }

    #[test]
    fn the_srgb_curves_are_inverses_and_match_known_values() {
        for level in 0..=255 {
            let encoded = f64::from(level) / 255.0;
            assert!((linear_to_srgb(srgb_to_linear(encoded)) - encoded).abs() < 1e-12);
        }
        assert!((srgb_to_linear(0.5) - 0.214_041).abs() < 1e-5);
        assert!((linear_to_srgb(0.18) - 0.4614).abs() < 1e-3);
        assert_eq!(linear_to_srgb(-1.0), 0.0);
        assert!((linear_to_srgb(3.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_centre_is_untouched_and_the_corner_is_dark() {
        let pass = pass(&[], 1.0);
        let centre = apply_pass_to_srgb(&pass, [0.8, 0.8, 0.8], 50, 50, 101, 101).unwrap();
        for channel in centre {
            assert!((channel - 0.8).abs() < 1e-9, "{centre:?}");
        }
        let corner = apply_pass_to_srgb(&pass, [0.8, 0.8, 0.8], 0, 0, 101, 101).unwrap();
        // The default vignette (radius 0.75, softness 0.45) reaches 55% at the corner.
        for channel in corner {
            assert!(channel < 0.6 && channel > 0.4, "{corner:?}");
        }
        let hard = pass_with(&[
            ("radius", FxValue::Number(0.0)),
            ("softness", FxValue::Number(0.05)),
        ]);
        let black = apply_pass_to_srgb(&hard, [0.8, 0.8, 0.8], 0, 0, 101, 101).unwrap();
        assert!(black.iter().all(|channel| *channel < 1e-6), "{black:?}");
    }

    #[test]
    fn zero_amount_returns_the_input_bit_for_bit() {
        let pass = pass(&[], 0.0);
        let out = apply_pass_to_srgb(&pass, [0.123, 0.456, 0.789], 0, 0, 16, 9).unwrap();
        assert_eq!(out, [0.123, 0.456, 0.789]);
        let mut rgba = vec![200_u8; 16 * 9 * 4];
        let before = rgba.clone();
        assert!(apply_pass_to_rgba8(&pass, &mut rgba, 16, 9));
        assert_eq!(rgba, before);
    }

    #[test]
    fn the_centre_moves_the_dark_region() {
        let left = pass(&[("center", FxValue::Point { x: 0.0, y: 0.5 })], 1.0);
        // At the left edge middle the vignette is centred: untouched; at the right edge: dark.
        let near = apply_pass_to_srgb(&left, [0.9, 0.9, 0.9], 0, 50, 100, 100).unwrap();
        let far = apply_pass_to_srgb(&left, [0.9, 0.9, 0.9], 99, 50, 100, 100).unwrap();
        assert!(near[0] > far[0] + 0.3, "{near:?} {far:?}");
    }

    #[test]
    fn a_wrong_buffer_is_refused_untouched() {
        let pass = pass(&[], 1.0);
        let mut rgba = vec![7_u8; 10];
        assert!(!apply_pass_to_rgba8(&pass, &mut rgba, 4, 4));
        assert_eq!(rgba, vec![7_u8; 10]);
    }

    fn fade(amount_param: f64, color: [f64; 3], weight: f64) -> FxPass {
        use crate::directed_scene::fx::catalog::FADE;
        let params = FADE
            .params
            .iter()
            .map(|spec| {
                let value = match spec.id {
                    "amount" => FxValue::Number(amount_param),
                    "color" => FxValue::Color {
                        r: color[0],
                        g: color[1],
                        b: color[2],
                    },
                    _ => spec.kind.default_value(),
                };
                (spec.id.to_string(), value)
            })
            .collect();
        FxPass {
            track_id: "t".into(),
            clip_id: "c".into(),
            effect: "fade".into(),
            frame_in_clip: 0,
            weight: 1.0,
            amount: weight,
            params,
            seed: None,
            scope: FxScope::Output,
        }
    }

    #[test]
    fn a_fade_mixes_in_linear_light_by_amount_times_weight_the_same_everywhere() {
        // Half way to white from 0.5 (encoded) is the mean of the LINEAR values, then encoded.
        let pass = fade(1.0, [1.0, 1.0, 1.0], 0.5);
        let expected = linear_to_srgb((srgb_to_linear(0.5) + 1.0) / 2.0);
        for (x, y) in [(0, 0), (7, 3), (15, 8)] {
            let out = apply_pass_to_srgb(&pass, [0.5, 0.5, 0.5], x, y, 16, 9).unwrap();
            for channel in out {
                assert!((channel - expected).abs() < 1e-12, "{out:?} vs {expected}");
            }
        }
        // The amount parameter and the weight multiply: 0.5 × 0.5 equals 1 × 0.25.
        let product = fade(0.5, [1.0, 0.0, 0.0], 0.5);
        let direct = fade(1.0, [1.0, 0.0, 0.0], 0.25);
        assert_eq!(
            apply_pass_to_srgb(&product, [0.3, 0.6, 0.9], 4, 4, 9, 9),
            apply_pass_to_srgb(&direct, [0.3, 0.6, 0.9], 4, 4, 9, 9)
        );
        // Full strength is the colour, and a colour is not a gradient: red stays 0..1 per channel.
        let solid = fade(1.0, [1.0, 0.0, 0.0], 1.0);
        let out = apply_pass_to_srgb(&solid, [0.3, 0.6, 0.9], 0, 0, 4, 4).unwrap();
        for (channel, want) in out.iter().zip([1.0, 0.0, 0.0]) {
            assert!((channel - want).abs() < 1e-12, "{out:?}");
        }
    }

    #[test]
    fn a_fade_of_zero_amount_or_zero_weight_is_the_exact_identity() {
        let source = [0.123, 0.456, 0.789];
        for pass in [
            fade(0.0, [1.0, 1.0, 1.0], 1.0),
            fade(1.0, [1.0, 1.0, 1.0], 0.0),
        ] {
            assert_eq!(
                apply_pass_to_srgb(&pass, source, 2, 2, 8, 8).unwrap(),
                source
            );
            let mut rgba = vec![201_u8; 8 * 8 * 4];
            let before = rgba.clone();
            assert!(apply_pass_to_rgba8(&pass, &mut rgba, 8, 8));
            assert_eq!(rgba, before);
        }
    }

    #[test]
    fn the_strength_is_the_vignette_shade_or_the_fade_amount_and_nothing_else_is_known() {
        let amount = fade(0.3, [0.0; 3], 1.0);
        assert_eq!(pass_strength(&amount, 0.0, 0.0, 16.0, 9.0), Some(0.3));
        assert_eq!(pass_strength(&amount, 0.9, 0.9, 16.0, 9.0), Some(0.3));
        let mut unknown = fade(0.3, [0.0; 3], 1.0);
        unknown.effect = "glitch".into();
        assert_eq!(pass_strength(&unknown, 0.0, 0.0, 16.0, 9.0), None);
        let mut rgba = vec![9_u8; 4 * 4 * 4];
        assert!(!apply_pass_to_rgba8(&unknown, &mut rgba, 4, 4));
    }
}
