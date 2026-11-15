//! Shared plumbing of the BL081.2.2 regressions: sound ramps of cut clips.
//!
//! Positions are **sample frames of the mix** (`S(frame)` of the absolute clock), so a range is
//! the same samples in every grid. A plan segment says where its ramps sit through
//! `output_start`, `clip_offset`, `clip_frames`, `fade_in` and `fade_out`; [`Segment`] turns
//! them into the two anchors and the two ends of the ramps.
use super::*;
use tessel_editor_core::audio_clock::sample_at;
use tessel_editor_core::time::Rate;

pub(super) const CONFIGS: [(u32, bool); 2] = [(44_100, true), (48_000, false)];

/// Sample frame of the mix at which sequence frame `frame` starts.
pub(super) fn at(rate: (u32, u32), frame: i64) -> usize {
    sample_at(Rate::new(rate.0, rate.1).unwrap(), frame).unwrap() as usize
}

pub(super) fn differs(a: &Pcm, b: &Pcm, from: usize, to: usize) -> bool {
    assert_eq!(a.samples.len(), b.samples.len(), "length");
    a.samples[from * 2..to * 2]
        .iter()
        .zip(&b.samples[from * 2..to * 2])
        .any(|(x, y)| x.to_bits() != y.to_bits())
}

/// The mixes are the same bits on the sample frames `from..to`.
pub(super) fn assert_same(a: &Pcm, b: &Pcm, from: usize, to: usize, label: &str) {
    assert!(from <= to, "{label}: range {from}..{to}");
    assert_eq!(a.samples.len(), b.samples.len(), "{label}: length");
    let first = a.samples[from * 2..to * 2]
        .iter()
        .zip(&b.samples[from * 2..to * 2])
        .position(|(x, y)| x.to_bits() != y.to_bits());
    assert!(
        first.is_none(),
        "{label}: first differing sample frame {:?} of {from}..{to}",
        first.map(|n| from + n / 2)
    );
}

/// What the plan says of one clip's segment (the clip is whole inside the range).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Segment {
    pub output_start: u64,
    pub fade_in: u64,
    pub fade_out: u64,
    pub clip_offset: u64,
    pub clip_frames: u64,
}

impl Segment {
    /// Where the fade-in ramp begins (gain 0).
    pub fn in_anchor(&self) -> u64 {
        self.output_start - self.clip_offset
    }
    /// Where the fade-out ramp ends (gain 0).
    pub fn out_anchor(&self) -> u64 {
        self.in_anchor() + self.clip_frames
    }
    /// First sample frame at which the fade-in is over (gain 1).
    pub fn in_end(&self) -> u64 {
        self.in_anchor() + self.fade_in
    }
    /// First sample frame of the fade-out ramp.
    pub fn out_start(&self) -> u64 {
        self.out_anchor() - self.fade_out
    }
}

pub(super) fn segment(state: &EditorState, clip: &str) -> Segment {
    let plan = state.plan_audio(0, LENGTH).unwrap().to_json();
    let json = plan["segments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|segment| segment["clip"] == clip)
        .unwrap_or_else(|| panic!("no segment of {clip}"))
        .clone();
    let field = |name: &str| json[name].as_u64().unwrap();
    Segment {
        output_start: field("output_start"),
        fade_in: field("fade_in"),
        fade_out: field("fade_out"),
        clip_offset: field("clip_offset"),
        clip_frames: field("clip_frames"),
    }
}

/// The clip `main` (3..63) with the given fades: the uncut reference.
pub(super) fn whole(
    rate: (u32, u32),
    sample_rate: u32,
    stereo: bool,
    fade_in: i64,
    fade_out: i64,
) -> EditorState {
    let mut state = audio_project(rate, sample_rate, stereo);
    set(&mut state, "main", "fade_in", json!(fade_in));
    set(&mut state, "main", "fade_out", json!(fade_out));
    state
}

/// A copy of `state` with `clip` cut at `frame` (the right half is `<clip>-split-1`).
pub(super) fn cut(state: &EditorState, clip: &str, frame: i64) -> EditorState {
    let mut cut = state.clone();
    split(&mut cut, clip, frame);
    cut
}

/// What the previous model did after an edit: the window is gone, so both ramps are measured on
/// the half's own edges. Used as a **control**: the checks of the new model must see it.
pub(super) fn without_windows(state: &EditorState) -> EditorState {
    let mut control = state.clone();
    drop_windows(&mut control);
    control
}

pub(super) fn reopened(state: &EditorState) -> EditorState {
    let mut reopened = EditorState::from_document(&state.to_document(None), None)
        .expect("a document the editor saves reopens");
    // The document does not carry the session; the project is the same.
    reopened.timeline.playhead = state.timeline.playhead;
    reopened
}

/// The gain the half applies on top of the whole at `sample` of a ramp that rises over
/// `from..to` (`rises`) or falls over it, as the mixer computes it, read off the two mixes:
/// the ratio of the same sample (`None` where the source is silent).
pub(super) fn gain_ratio(edited: &Pcm, reference: &Pcm, sample: usize) -> Option<f32> {
    let (a, b) = (edited.sample(sample, 0), reference.sample(sample, 0));
    (b.abs() > 1e-6).then(|| a / b)
}
