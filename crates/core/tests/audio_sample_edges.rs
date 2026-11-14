//! Border behavior of the exact mixer, independent of sequence fixtures.
use std::fs;
use tessel_editor_core::audio::{
    AudioBackend, AudioPlan, AudioSegment, OfflineMixer, Pcm, decode_wav, encode_wav_pcm16,
};
use tessel_editor_core::id::Id;
use tessel_editor_core::time::Ratio;

fn plan(tag: &str) -> (AudioPlan, Vec<f32>) {
    let dir = std::env::temp_dir().join(format!("tessel-phase-edges-{tag}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("four.wav");
    let bytes = encode_wav_pcm16(&Pcm {
        sample_rate: 48000,
        channels: 1,
        samples: vec![0.25, 0.5, 0.75, 1.0],
    });
    let source = decode_wav(&bytes).unwrap().samples;
    fs::write(&path, bytes).unwrap();
    (
        AudioPlan {
            sample_rate: 48000,
            frames: 10,
            segments: vec![AudioSegment {
                clip: Id::new("edge").unwrap(),
                asset: Id::new("four").unwrap(),
                path: path.to_string_lossy().into_owned(),
                output_start: 0,
                frames: 10,
                source_start: -1,
                source_phase: Ratio::new(1, 2).unwrap(),
                source_rate: 48000,
                speed: Ratio::new(1, 2).unwrap(),
                gain: 1.0,
                pan: -1.0,
                fade_in: 0,
                fade_out: 0,
                clip_offset: 0,
                clip_frames: 10,
            }],
        },
        source,
    )
}

#[test]
fn negative_fraction_is_silent_and_the_final_source_sample_is_held_before_the_end() {
    let (plan, source) = plan("bounds");
    let [a, b, c, d]: [f32; 4] = source.try_into().unwrap();
    let expected = [
        0.0,
        a,
        (a + b) * 0.5,
        b,
        (b + c) * 0.5,
        c,
        (c + d) * 0.5,
        d,
        d,
        0.0,
    ];
    let pcm = OfflineMixer::default().mix(&plan).unwrap();
    for (index, expected) in expected.into_iter().enumerate() {
        assert_eq!(
            pcm.sample(index, 0).to_bits(),
            expected.to_bits(),
            "position {} / 2",
            index as i64 - 1
        );
        assert_eq!(pcm.sample(index, 1), 0.0);
    }
}

#[test]
fn a_manually_constructed_unrepresentable_phase_returns_a_typed_error() {
    let (mut plan, _) = plan("invalid");
    plan.segments[0].source_start = i64::MAX;
    let error = OfflineMixer::default().mix(&plan).unwrap_err();
    assert_eq!(error.code, "invalid_audio_plan");
}
