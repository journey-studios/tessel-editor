//! What a number control of an effect clip may hold: the one grid that the pointer and the
//! keyboard of any adapter put a value on before it becomes an edit (F016, BL082.6).
//!
//! WHY in the core: the Kit slider speaks `f32` (`0.4` reaches an adapter as
//! `0.4000000059604645`), and a keyboard step lands off the grid too. The range, the step and the
//! rounding are rules of the parameter, not of a widget, so a window, a script and a test all ask
//! this one function. A value an author *types or sends* (`scene.fx.edit`, a node `set`) is never
//! put through it: the catalog judges and may refuse it, nothing rounds it.

/// `raw` on the grid of `step` anchored at `min`, inside `[min, max]`.
///
/// * `integer` controls (whole-frame fields) come out as whole numbers.
/// * Other controls are written in the decimals of the step (`0.01` gives two), which removes
///   the float dust of `min + n * step`.
/// * A value that is not finite is returned as it came, so the core refuses it with a typed
///   code instead of an adapter hiding it.
pub fn quantize_control(raw: f64, min: f64, max: f64, step: f64, integer: bool) -> f64 {
    if !raw.is_finite() || !step.is_finite() || step <= 0.0 {
        return raw;
    }
    let steps = ((raw - min) / step).round();
    let snapped = (min + steps * step).clamp(min, max);
    if integer {
        return snapped.round();
    }
    // The decimals the step itself needs (`0.01` two, `0.25` two, `0.125` three), not the order
    // of magnitude of its first digit: `0.25` rounded to one decimal would leave the grid.
    let decimals = (0..=9).find(|places| {
        let scaled = step * 10f64.powi(*places);
        scaled.round() >= 1.0
            && (scaled - scaled.round()).abs() <= 8.0 * f64::EPSILON * scaled.abs()
    });
    // No decimal grid up to nine places: retain the snapped value rather than round a
    // sub-nanounit step to zero (or impose a different grid on a non-decimal step).
    let Some(decimals) = decimals else {
        return snapped;
    };
    let scale = 10f64.powi(decimals);
    ((snapped * scale).round() / scale).clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn the_grid_is_anchored_at_the_minimum_and_stays_in_range() {
        assert_eq!(
            quantize_control(f64::from(0.4_f32), 0.0, 1.0, 0.01, false),
            0.4
        );
        assert_eq!(quantize_control(1.7, 0.0, 1.0, 0.01, false), 1.0);
        assert_eq!(quantize_control(-0.3, 0.0, 1.0, 0.01, false), 0.0);
        // Softness starts at 0.05, so its grid is 0.05, 0.06, ...
        assert_eq!(
            quantize_control(f64::from(0.35_f32), 0.05, 1.0, 0.01, false),
            0.35
        );
        assert_eq!(quantize_control(0.0, 0.05, 1.0, 0.01, false), 0.05);
    }

    #[test]
    fn a_step_with_more_digits_than_its_magnitude_keeps_every_grid_point() {
        // `0.25` has two decimals: 0.75 is on the grid and must not come back as 0.8.
        assert_eq!(quantize_control(0.74, 0.0, 1.0, 0.25, false), 0.75);
        assert_eq!(quantize_control(0.26, 0.0, 1.0, 0.25, false), 0.25);
        assert_eq!(quantize_control(0.13, 0.0, 1.0, 0.125, false), 0.125);
        assert_eq!(quantize_control(0.4, 0.0, 1.0, 0.05, false), 0.4);
    }

    #[test]
    fn subnanounit_steps_keep_their_grid_instead_of_becoming_whole_numbers() {
        let step = 5e-10;
        for index in [1.0, 7.0, 123.0] {
            let expected = index * step;
            let actual = quantize_control(expected, 0.0, 1e-6, step, false);
            assert!((actual - expected).abs() <= expected * 1e-12);
        }
        assert_eq!(quantize_control(2.4, 0.0, 10.0, 2.5, false), 2.5);
    }

    #[test]
    fn whole_frame_controls_are_integers() {
        assert_eq!(quantize_control(7.6, 0.0, 95.0, 1.0, true), 8.0);
        assert_eq!(quantize_control(-3.0, 0.0, 95.0, 1.0, true), 0.0);
        assert_eq!(quantize_control(1e9, 0.0, 95.0, 1.0, true), 95.0);
    }

    #[test]
    fn garbage_is_passed_on_to_be_refused_not_hidden() {
        assert!(quantize_control(f64::NAN, 0.0, 1.0, 0.01, false).is_nan());
        assert!(quantize_control(f64::INFINITY, 0.0, 1.0, 0.01, false).is_infinite());
        assert_eq!(quantize_control(0.3, 0.0, 1.0, 0.0, false), 0.3);
    }
}
