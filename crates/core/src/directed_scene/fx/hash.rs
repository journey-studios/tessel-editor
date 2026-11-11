//! The integer hashes the noise of the effects layer is built on (F016 / F059, P3).
//!
//! The pilot writes them with `Math.imul`, xor and shifts only, so that GLSL ES 3.00 (`uint`)
//! reproduces them bit for bit (`fx/fx-hash.ts`, `runtime/fx/shaders/chunks/hash.ts` of the
//! snapshot `d40ac3f7c0`). Here every step is `u32` with `wrapping_*`: no float, no `sin`, no
//! clock, no state, so the noise of a frame is a pure function of `(seed, frame, pixel)` and a
//! seek answers what linear playback drew.
//!
//! The vectors that pin these functions are held outside this file: the exact integers of the
//! pilot's own spec (`resolve-fx-frame-hashes.spec.ts`) and an independent Python implementation
//! in `scripts/pilot-grain-corpus.py`, compared with the GLSL of the runtime in a browser test.

/// FNV-1a over the UTF-8 **bytes** of `text` (not UTF-16 code units): the seed an effect gets
/// from its ID when none is written.
pub fn fnv1a32(text: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Chris Wellons' `lowbias32`: a bijective integer hash with good avalanche.
pub fn lowbias32(value: u32) -> u32 {
    let mut hash = value;
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7feb_352d);
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x846c_a68b);
    hash ^= hash >> 16;
    hash
}

/// The base seed of a clip: the one it was given, or the hash of its ID.
pub fn base_seed(explicit: Option<u32>, clip_id: &str) -> u32 {
    explicit.unwrap_or_else(|| fnv1a32(clip_id))
}

/// How the noise of an effect is reseeded over the clip (the pilot's `reseed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reseed {
    /// The effect draws no noise: its passes carry no seed.
    Never,
    /// One seed for the whole clip: `lowbias32(base)`.
    Static,
    /// A new seed on every frame of the clip: `lowbias32(base ^ frameInClip)`.
    PerFrame,
}

/// The seed of a pass at `frame_in_clip` (the pilot's `seedFor`). The frame is reduced modulo
/// 2^32 exactly as JavaScript's `^` does, so frames past 32 bits wrap like the origin and never
/// panic or saturate.
pub fn seed_for(reseed: Reseed, base: u32, frame_in_clip: i64) -> Option<u32> {
    match reseed {
        Reseed::Never => None,
        Reseed::Static => Some(lowbias32(base)),
        Reseed::PerFrame => Some(lowbias32(base ^ (frame_in_clip as u32))),
    }
}

/// The noise of one grain cell for a seed: the shader's `hashPixel`.
pub fn hash_pixel(seed: u32, x: u32, y: u32) -> u32 {
    lowbias32(
        lowbias32(seed ^ 0x9e37_79b9)
            .wrapping_add(x)
            .wrapping_add(y << 16),
    )
}

/// 24 bits of a hash as a float in `[0, 1)`; exact in `f32` (a shift and a conversion).
pub fn unit_float(hash: u32) -> f32 {
    (hash >> 8) as f32 / 16_777_216.0
}

/// Triangular noise in `(-1, 1)`: two uniform draws summed, in `f32` like the shader.
pub fn triangular(hash: u32) -> f32 {
    unit_float(hash) + unit_float(lowbias32(hash ^ 0x68bc_21eb)) - 1.0
}

/// The three channels of a colored grain cell, from the mono hash `h` (the shader's `colored`
/// branch: the other channels re-hash `h` shifted by two constants).
pub fn colored_triangular(h: u32) -> [f32; 3] {
    [
        triangular(h),
        triangular(lowbias32(h.wrapping_add(0x632b_e5ab))),
        triangular(lowbias32(h.wrapping_add(0x8515_7af5))),
    ]
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;

    // The vectors of the pilot's own spec (`resolve-fx-frame-hashes.spec.ts`), which were
    // computed with an independent implementation there.
    #[test]
    fn fnv1a32_matches_the_published_and_the_pilot_vectors() {
        assert_eq!(fnv1a32(""), 0x811c_9dc5);
        assert_eq!(fnv1a32("a"), 0xe40c_292c);
        assert_eq!(fnv1a32("foobar"), 0xbf9c_f968);
        assert_eq!(fnv1a32("viñeta"), 419_953_240);
        assert_eq!(fnv1a32("fx-01"), 3_286_700_573);
        assert_eq!(fnv1a32("transition-01"), 1_697_369_554);
    }

    #[test]
    fn lowbias32_matches_the_pilot_vectors_across_the_32_bit_range() {
        assert_eq!(lowbias32(0), 0);
        assert_eq!(lowbias32(1), 1_753_845_952);
        assert_eq!(lowbias32(2), 3_507_691_905);
        assert_eq!(lowbias32(42), 388_445_122);
        assert_eq!(lowbias32(12_345), 2_435_775_735);
        assert_eq!(lowbias32(0xdead_beef), 3_861_431_939);
        assert_eq!(lowbias32(0xffff_ffff), 1_734_902_346);
    }

    #[test]
    fn lowbias32_is_a_bijection_over_a_run_of_inputs() {
        let seen: std::collections::HashSet<u32> = (0..5000).map(lowbias32).collect();
        assert_eq!(seen.len(), 5000);
    }

    #[test]
    fn the_per_frame_seed_mixes_the_frame_and_wraps_like_the_origin() {
        let base = base_seed(None, "grain-1");
        assert_eq!(base, fnv1a32("grain-1"));
        assert_eq!(
            seed_for(Reseed::PerFrame, base, 0),
            Some(lowbias32(base)),
            "frame 0 is the static seed"
        );
        assert_eq!(
            seed_for(Reseed::PerFrame, base, 7),
            Some(lowbias32(base ^ 7))
        );
        // JS: `(base ^ frame) >>> 0` reduces the frame to 32 bits.
        assert_eq!(
            seed_for(Reseed::PerFrame, base, 1 << 32),
            seed_for(Reseed::PerFrame, base, 0)
        );
        assert_eq!(
            seed_for(Reseed::PerFrame, 0xffff_ffff, 0xffff_ffff),
            Some(lowbias32(0))
        );
        assert_eq!(
            seed_for(Reseed::Static, base, 5),
            Some(lowbias32(base)),
            "a static seed ignores the frame"
        );
        assert_eq!(seed_for(Reseed::Never, base, 5), None);
        assert_eq!(base_seed(Some(0), "x"), 0, "an explicit zero is a seed");
        assert_eq!(base_seed(Some(u32::MAX), "x"), u32::MAX);
    }

    #[test]
    fn the_pixel_hash_wraps_and_never_depends_on_anything_but_its_inputs() {
        let again = hash_pixel(123, 4, 5);
        assert_eq!(hash_pixel(123, 4, 5), again);
        assert_ne!(hash_pixel(123, 5, 4), again);
        // The row goes in the upper half word: a column past 16 bits does not wrap into it
        // differently from the origin's `uint` arithmetic.
        assert_eq!(
            hash_pixel(u32::MAX, u32::MAX, u32::MAX),
            lowbias32(
                lowbias32(u32::MAX ^ 0x9e37_79b9)
                    .wrapping_add(u32::MAX)
                    .wrapping_add(u32::MAX << 16)
            )
        );
    }

    #[test]
    fn the_triangular_noise_is_inside_the_open_interval_and_the_unit_float_exact() {
        assert_eq!(unit_float(0), 0.0);
        assert_eq!(unit_float(0xffff_ffff), 16_777_215.0 / 16_777_216.0);
        for hash in [0_u32, 1, 0x1234_5678, 0xdead_beef, 0xffff_ffff] {
            let noise = triangular(hash);
            assert!(noise > -1.0 && noise < 1.0, "{hash:#x}: {noise}");
        }
    }
}
