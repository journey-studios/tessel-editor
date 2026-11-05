//! Demonstration imagery and illustrative waveform data.

use std::sync::{Arc, OnceLock};

use gpui_kit::{Image, ImageFormat, ImageSource};

use crate::state::library::DemoImage;

// Origin and licensing caveats: docs/proui-reference.md. Only these two files
// are bundled; nothing else is decoded or read from disk at run time.
static COAST: &[u8] = include_bytes!("../../../../assets/demo/coast.png");
static HIGHLANDS: &[u8] = include_bytes!("../../../../assets/demo/highlands.png");

pub fn demo_image(image: DemoImage) -> ImageSource {
    static CACHE: OnceLock<[Arc<Image>; 2]> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        [
            Arc::new(Image::from_bytes(ImageFormat::Png, COAST.to_vec())),
            Arc::new(Image::from_bytes(ImageFormat::Png, HIGHLANDS.to_vec())),
        ]
    });
    let index = match image {
        DemoImage::Coast => 0,
        DemoImage::Highlands => 1,
    };
    ImageSource::Image(cache[index].clone())
}

fn seed_hash(seed: &str) -> u64 {
    seed.bytes().fold(0x9E37_79B9_7F4A_7C15_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

fn mix(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

/// Deterministic pseudo-audio envelope in `0.12..=1.0` at a given frame.
///
/// It is a function of the clip seed and the frame only, so zooming the
/// timeline resamples the same shape instead of re-rolling it. This is
/// decorative data: no audio file is read or analysed, so it must never be
/// presented as a real waveform.
pub fn waveform_amplitude(seed: &str, frame: i64) -> f32 {
    let hash = seed_hash(seed);
    let at = frame as f32;
    let swell = ((at / 190.0).sin() * 0.5 + 0.5) * 0.35;
    let body = (at / 41.0).sin().abs() * 0.25;
    let noise = (mix(hash ^ (frame / 2) as u64) >> 40) as f32 / (1u64 << 24) as f32;
    (0.12 + swell + body + noise * 0.3).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform_is_deterministic_bounded_and_seed_specific() {
        let sample = |seed| -> Vec<f32> { (0..400).map(|f| waveform_amplitude(seed, f)).collect() };
        assert_eq!(sample("score-clip"), sample("score-clip"));
        assert_ne!(sample("score-clip"), sample("ambience-clip"));
        assert!(
            sample("score-clip")
                .iter()
                .all(|value| (0.12..=1.0).contains(value))
        );
    }
}
