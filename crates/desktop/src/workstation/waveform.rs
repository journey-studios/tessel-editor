//! Illustrative waveform painter for audio clips.

use gpui_kit::*;

use super::media::waveform_amplitude;

const BAR_PITCH: f32 = 3.0;
const BAR_WIDTH: f32 = 2.0;

/// Vertical bars sampled from the clip's deterministic envelope. `origin_frame`
/// is the clip-local frame at the left edge, so a bar always shows the same
/// frame whatever the zoom.
pub fn waveform(
    seed: &'static str,
    color: Hsla,
    pixels_per_frame: f32,
) -> impl IntoElement + use<> {
    canvas(
        |bounds, _, _| bounds,
        move |bounds: Bounds<Pixels>, _, window, _| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let center = f32::from(bounds.origin.y) + height / 2.0;
            let bars = (width / BAR_PITCH).floor() as usize;
            for index in 0..bars {
                let x = index as f32 * BAR_PITCH;
                let frame = (x / pixels_per_frame.max(0.01)).round() as i64;
                let bar_height = (height * waveform_amplitude(seed, frame)).max(2.0);
                window.paint_quad(fill(
                    Bounds::new(
                        point(
                            px(f32::from(bounds.origin.x) + x),
                            px(center - bar_height / 2.0),
                        ),
                        size(px(BAR_WIDTH), px(bar_height)),
                    ),
                    color,
                ));
            }
        },
    )
    .size_full()
}
