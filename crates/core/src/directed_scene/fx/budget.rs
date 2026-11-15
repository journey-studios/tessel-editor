//! The pass budget of the layer, with the passes of a transition window (F064, P4b S1b).
//!
//! A frame of a window draws the passes of BOTH decks, the blend (one pass) and the output's once
//! (plan §8.2). The sweep stays a sweep over edges, not over frames: a deck outside its own range
//! holds one border frame (D15), so the neighbour's clips that touch that border add a CONSTANT
//! number of passes over a whole half window, and the blend adds one over the whole window. The
//! cost is the number of clips and transitions, whatever the length of the edit.
//!
//! Policy unchanged from F016: a warning, never a refusal. Nothing here measures time.

use crate::directed_scene::Shot;

use super::frame::{border_frame, shots_of};
use super::{FX_PASS_BUDGET, Fx, PassBudget, transition_window};

impl Fx {
    /// [`Self::pass_budget`] over the edit, counting the transition windows of the layer: the
    /// blend over each window and the neighbour deck's border clips over its half. Without a
    /// visible transition it is exactly `pass_budget`.
    pub fn pass_budget_in(&self, shots: &[Shot], edit_duration_frames: u64) -> PassBudget {
        self.budget_with(
            edit_duration_frames,
            self.transition_edges(shots, edit_duration_frames),
        )
    }

    /// The `(frame, delta)` edges the visible transitions add to the sweep.
    fn transition_edges(&self, shots: &[Shot], edit_duration_frames: u64) -> Vec<(i64, i32)> {
        let edit = i64::try_from(edit_duration_frames).unwrap_or(i64::MAX);
        let mut edges = Vec::new();
        for transition in self
            .transitions
            .iter()
            .filter(|transition| !transition.hidden)
        {
            let Some((from, to)) = shots_of(transition, shots) else {
                continue;
            };
            let Ok(cut) = i64::try_from(to.start_frame) else {
                continue;
            };
            let Ok(window) =
                transition_window(cut, transition.duration_frames, transition.alignment, edit)
            else {
                continue;
            };
            let mut span = |start: i64, end: i64, passes: usize| {
                let passes = i32::try_from(passes).unwrap_or(i32::MAX);
                if start < end && passes > 0 {
                    edges.push((start, passes));
                    edges.push((end, -passes));
                }
            };
            span(window.start, window.end, 1);
            // Before the cut the neighbour is the incoming shot's head (its first frame held);
            // from the cut on it is the outgoing shot's tail (its last frame held).
            span(
                window.start,
                window.end.min(cut),
                self.border_clips(to, border_frame(to, window.start)),
            );
            span(
                window.start.max(cut),
                window.end,
                self.border_clips(from, border_frame(from, window.end)),
            );
        }
        edges
    }

    /// Visible `shot` clips of `shot` that hold `frame` (the border the deck draws).
    fn border_clips(&self, shot: &Shot, frame: i64) -> usize {
        self.tracks
            .iter()
            .filter(|track| !track.hidden)
            .flat_map(|track| &track.clips)
            .filter(|clip| {
                !clip.hidden
                    && clip.scope.shot_id() == Some(shot.id.as_str())
                    && clip.start_frame <= frame
                    && frame < clip.end_frame_exclusive()
            })
            .count()
    }

    /// The sweep: the layer's clips plus the `extra` edges, ends before starts at one frame.
    pub(super) fn budget_with(
        &self,
        edit_duration_frames: u64,
        mut edges: Vec<(i64, i32)>,
    ) -> PassBudget {
        let length = i64::try_from(edit_duration_frames).unwrap_or(i64::MAX);
        for track in self.tracks.iter().filter(|track| !track.hidden) {
            for clip in track.clips.iter().filter(|clip| !clip.hidden) {
                let start = clip.start_frame.max(0);
                let end = clip.end_frame_exclusive().min(length);
                if start < end {
                    edges.push((start, 1));
                    edges.push((end, -1));
                }
            }
        }
        // At a frame both ends and starts happen: a clip ends *before* the one starting there begins.
        edges.sort_unstable();
        let mut running: i64 = 0;
        let mut peak: i64 = 0;
        let mut first_frame_over = None;
        let mut frames_over = 0;
        let budget = i64::try_from(FX_PASS_BUDGET).unwrap_or(i64::MAX);
        for (index, &(frame, delta)) in edges.iter().enumerate() {
            running += i64::from(delta);
            peak = peak.max(running);
            let next = edges.get(index + 1).map_or(frame, |&(next, _)| next);
            if running > budget && next > frame {
                first_frame_over.get_or_insert(frame);
                frames_over += next - frame;
            }
        }
        PassBudget {
            budget: FX_PASS_BUDGET,
            peak: usize::try_from(peak).unwrap_or(0),
            first_frame_over,
            frames_over,
        }
    }
}
