//! F017 / BL081: split one timeline clip into two at an absolute sequence frame.
//!
//! The rule lives here, once, for the library, the headless JSONL and the window:
//! [`EditorState::plan_split`] checks everything (frame, ID, rational source time, fades, Ken
//! Burns, orbit) without touching the state, and [`EditorState::split_clip`] applies the plan
//! with nothing left that can fail. A refused split therefore changes no clip, no property, no
//! selection, no revision and no history entry.
//!
//! Continuity, in one sentence: every frame of the sequence reads, after the cut, the same
//! source time, the same camera, the same zoom and the same opacity it read before (see
//! `docs/features/split.md`). What the model cannot keep that way is refused with
//! `split_unsupported` instead of being approximated.

use serde_json::{Value, json};

use super::EditorState;
use super::Subject;
use super::error::CommandError;
use super::inspector::{Value as PropertyValue, property_def};
use super::timeline::ClipKind;
use crate::assets::MediaKind;
use crate::content::Camera;
use crate::id::Id;
use crate::time::Ratio;

/// Which half of the cut a property adjustment lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Half {
    Left,
    Right,
}

/// One inspector value the split rewrites so a half keeps what the whole clip showed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Adjustment {
    half: Half,
    property: &'static str,
    value: f64,
}

/// A checked split: everything [`EditorState::split_clip`] will write.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitPlan {
    /// The clip that is cut; its left half keeps this ID.
    pub clip: Id,
    /// The ID the right half gets (validated, free, not yet interned).
    pub right_id: String,
    /// The cut: the first frame of the right half.
    pub frame: i64,
    pub left_duration: i64,
    pub right_duration: i64,
    /// Where in the asset the right half's first frame reads, exact.
    pub right_source_start: Ratio,
    right_camera: Option<Camera>,
    adjustments: Vec<Adjustment>,
}

/// Why the "cut at the playhead" control is not offered. [`SplitBlock::reason`] is the text of
/// the node's `disabled_reason` and of the error `invoke` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitBlock {
    NoSelection,
    /// Not strictly inside the clip: a half would be empty or the playhead is elsewhere.
    PlayheadOutside,
    /// A fade ramp straddles the cut and cannot be written as two ramps.
    FadeCrossesCut,
    /// Overlapping audio fades are shortened by the mixer at every playhead position.
    FadesLongerThanClip,
    /// Any other refusal of [`EditorState::plan_split`] (overflow, ID policy): run `clip.split`
    /// to read the typed error.
    Refused,
}

impl SplitBlock {
    pub fn reason(self) -> &'static str {
        match self {
            Self::NoSelection => "Select a clip to cut it at the playhead",
            Self::PlayheadOutside => {
                "Move the playhead inside the selected clip, past its first frame, to cut it"
            }
            Self::FadeCrossesCut => {
                "A fade of the clip spans the playhead: move the playhead or shorten the fade"
            }
            Self::FadesLongerThanClip => {
                "The clip's fades are longer than the clip: shorten a fade to cut it"
            }
            Self::Refused => "The core refuses a cut here: run clip.split to read the reason",
        }
    }
}

/// What the "cut at the playhead" control would do right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitAvailability {
    Ready { clip: Id, frame: i64 },
    Blocked(SplitBlock),
}

fn property_number(state: &EditorState, clip: &str, id: &str) -> f64 {
    match property_def(id).map(|property| state.inspector.value(clip, property)) {
        Some(PropertyValue::Number(number)) => number,
        _ => 0.0,
    }
}

impl EditorState {
    fn split_id_in_use(&self, raw: &str) -> bool {
        self.timeline.track(raw).is_some()
            || self.timeline.clip(raw).is_some()
            || self
                .sequences
                .iter()
                .any(|timeline| timeline.clip(raw).is_some())
    }

    /// `<clip>-split-<n>` with the smallest `n` (from 1) that is free in every sequence. An ID
    /// that would not fit [`crate::id::MAX_ID_LEN`] is refused: truncating would be a guess.
    fn split_default_id(&self, clip: &str) -> Result<String, CommandError> {
        for n in 1u32.. {
            let candidate = format!("{clip}-split-{n}");
            if let Err(error) = Id::validate(&candidate) {
                return Err(CommandError::new(
                    "invalid_id",
                    format!(
                        "No default ID fits for the right half of `{clip}` ({error}): pass `new_id`"
                    ),
                )
                .target(format!("clip:{clip}")));
            }
            if !self.split_id_in_use(&candidate) {
                return Ok(candidate);
            }
        }
        // Unreachable: `u32::MAX` clips would not fit in memory first.
        Err(CommandError::new(
            "invalid_id",
            "No free default ID: pass `new_id`",
        ))
    }

    /// Checks a split without changing anything. `frame` is an absolute frame of the active
    /// sequence, strictly inside the clip; `new_id` names the right half (`None`: the default
    /// policy, see [`Self::split_default_id`]).
    pub fn plan_split(
        &self,
        clip: &str,
        frame: i64,
        new_id: Option<&str>,
    ) -> Result<SplitPlan, CommandError> {
        let timeline = &self.timeline;
        let entry = timeline
            .clip(clip)
            .ok_or_else(|| CommandError::unknown_element("clip", clip))?;
        let target = format!("clip:{}", entry.id);
        let overflow = |what: &str| {
            CommandError::new("invalid_time", format!("{what}; nothing changed"))
                .target(target.clone())
        };
        let end = entry
            .start
            .checked_add(entry.duration)
            .ok_or_else(|| overflow("The clip's end overflows a frame count"))?;
        let span = json!({ "start": entry.start, "end": end, "frame": frame });
        if frame < entry.start || frame > end {
            return Err(CommandError::new(
                "out_of_bounds",
                format!(
                    "Frame {frame} is outside clip `{}` ({}..{end})",
                    entry.id, entry.start
                ),
            )
            .target(target.clone())
            .details(span));
        }
        if frame == entry.start || frame == end {
            return Err(CommandError::new(
                "invalid_duration",
                format!(
                    "Cutting `{}` at its edge (frame {frame}) would leave a half with no frame",
                    entry.id
                ),
            )
            .target(target.clone())
            .details(span));
        }
        let right_id = match new_id {
            Some(raw) => {
                Id::validate(raw).map_err(|error| {
                    CommandError::new("invalid_id", error.to_string()).target(raw)
                })?;
                if self.split_id_in_use(raw) {
                    return Err(CommandError::new(
                        "duplicate_id",
                        format!("`{raw}` is already a track or a clip"),
                    )
                    .target(raw));
                }
                raw.to_string()
            }
            None => self.split_default_id(entry.id.as_str())?,
        };

        // Frames before the cut (`k`) and after it, both positive here.
        let before = frame - entry.start;
        let after = end - frame;
        // Source time the right half starts at: the head of the clip plus the elapsed clip time
        // at the clip's speed, exactly like `trim_clip` (but checked at every step).
        let rate = timeline.rate;
        let elapsed = before
            .checked_mul(i64::from(rate.den()))
            .and_then(|numerator| Ratio::new(numerator, i64::from(rate.num())))
            .ok_or_else(|| overflow("The elapsed time of the left half overflows"))?;
        let shift = elapsed
            .mul(self.clip_speed(entry))
            .ok_or_else(|| overflow("The source time of the cut overflows"))?;
        let right_source_start = entry
            .source_start
            .add(shift)
            .ok_or_else(|| overflow("The source time of the cut overflows"))?;
        // Sound needs no rewriting (BL081.5): the mixer reads output sample `n` at the exact
        // source time `source_start + (n / 48000 - start / rate) * speed` (`audio_clock`), and the
        // right half's `source_start` is exactly what that formula gives for the whole clip at
        // the cut. Both halves therefore read the same sample positions as the whole clip, on any
        // frame rate, source rate and speed. What must hold is that the formula is computable
        // there (no overflow), so a split never leaves a clip the mixer cannot place.
        let audio_stream = if matches!(entry.kind, ClipKind::Audio | ClipKind::Video) {
            entry
                .media
                .and_then(|id| self.library.item(id))
                .and_then(|item| item.asset.as_ref())
                .filter(|asset| matches!(asset.source, crate::assets::Source::File { .. }))
                .and_then(|asset| {
                    asset
                        .meta
                        .audio_stream(property_number(self, clip, "audio_stream").round() as u32)
                })
        } else {
            None
        };
        if let Some(stream) = audio_stream {
            let placed = crate::audio_clock::sample_at(rate, frame).and_then(|sample| {
                crate::audio_clock::source_position(
                    rate,
                    frame,
                    right_source_start,
                    self.clip_speed(entry),
                    stream.offset,
                    stream.sample_rate,
                    sample,
                )
            });
            if placed.is_none() {
                return Err(overflow("The sound position of the cut overflows"));
            }
        }

        // An orbiting camera counts seconds from the clip's start, so the right half starts
        // where the orbit already is; keyed cameras read absolute scene time and need nothing.
        let right_camera = entry.camera.map(|camera| {
            if entry.camera_keys.is_empty() {
                camera.at(shift.to_f64())
            } else {
                camera
            }
        });

        let mut adjustments = Vec::new();
        // Loaded visual fades may be fractional: match the renderer's unrounded opacity
        // ramps. Only audio uses the mixer's whole-frame fade durations.
        let fade = |property| {
            let value = property_number(self, clip, property);
            if entry.kind == ClipKind::Audio {
                value.round()
            } else {
                value
            }
        };
        let fade_in = fade("fade_in");
        let fade_out = fade("fade_out");
        let unsupported = |reason: &'static str, message: String| {
            CommandError::new("split_unsupported", message)
                .target(format!("clip:{}", entry.id))
                .details(json!({ "reason": reason, "frame": frame,
                    "fade_in": fade_in, "fade_out": fade_out,
                    "frames_before": before, "frames_after": after }))
        };
        // The mixer shortens two fades that together outlast a sound, in proportion; a half is
        // shorter, so it would shorten them differently and the gain would move.
        if entry.kind == ClipKind::Audio
            && fade_in > 0.0
            && fade_out > 0.0
            && fade_in + fade_out > entry.duration as f64
        {
            return Err(unsupported(
                "fades_longer_than_clip",
                format!(
                    "The fades of `{}` ({fade_in} + {fade_out} frames) are longer than the clip, \
                     so the mixer shortens them; the halves would not shorten them the same way",
                    entry.id
                ),
            ));
        }
        if fade_in > 0.0 && (before as f64) < fade_in {
            return Err(unsupported(
                "fade_in_crosses_cut",
                format!(
                    "The {fade_in}-frame fade-in of `{}` is still ramping at frame {frame}; \
                     two clips cannot share one ramp. Cut at {} or later, or shorten the fade",
                    entry.id,
                    entry.start + fade_in.ceil() as i64
                ),
            ));
        }
        // The last frame of the left half must already be past the fade-out ramp. A picture
        // fades by frame (`remaining / fade`, one at the ramp's start); a sound fades by sample,
        // where the last sample of a frame is one sample short of the next frame boundary.
        let slack = if entry.kind == ClipKind::Audio { 0 } else { 1 };
        if fade_out > 0.0 && ((after + slack) as f64) < fade_out {
            return Err(unsupported(
                "fade_out_crosses_cut",
                format!(
                    "The {fade_out}-frame fade-out of `{}` has already started at frame {frame}; \
                     two clips cannot share one ramp. Cut at {} or earlier, or shorten the fade",
                    entry.id,
                    end - fade_out.ceil() as i64 + slack
                ),
            ));
        }
        // Each half keeps the ramp at its own outer edge; the inner edges are plain cuts.
        if fade_out > 0.0 {
            adjustments.push(Adjustment {
                half: Half::Left,
                property: "fade_out",
                value: 0.0,
            });
        }
        if fade_in > 0.0 {
            adjustments.push(Adjustment {
                half: Half::Right,
                property: "fade_in",
                value: 0.0,
            });
        }
        // A still drifts from `zoom` to `end_zoom` over `duration - 1` frames: the left half
        // ends where the whole clip was one frame before the cut, the right half starts where
        // it was at the cut and ends at the original `end_zoom`.
        let still = entry.kind == ClipKind::Video
            && entry
                .media
                .and_then(|media| self.library.item(media))
                .is_some_and(|item| item.kind == MediaKind::Still);
        let ken_burns = matches!(
            property_def("ken_burns").map(|property| self.inspector.value(clip, property)),
            Some(PropertyValue::Bool(true))
        );
        if still && ken_burns {
            let zoom = property_number(self, clip, "zoom");
            let end_zoom = property_number(self, clip, "end_zoom");
            let spans = entry.duration - 1;
            let fraction = |frames: i64| Ratio::new(frames, spans).map_or(0.0, Ratio::to_f64);
            if end_zoom != zoom {
                adjustments.push(Adjustment {
                    half: Half::Left,
                    property: "end_zoom",
                    value: zoom + (end_zoom - zoom) * fraction(before - 1),
                });
                adjustments.push(Adjustment {
                    half: Half::Right,
                    property: "zoom",
                    value: zoom + (end_zoom - zoom) * fraction(before),
                });
            }
        }

        Ok(SplitPlan {
            clip: entry.id,
            right_id,
            frame,
            left_duration: before,
            right_duration: after,
            right_source_start,
            right_camera,
            adjustments,
        })
    }

    /// Whether the selected clip can be cut at the playhead, and if not, why not. The tree
    /// node, the `timeline.split` invoke and the toolbar button all read this one answer.
    pub fn split_availability(&self) -> SplitAvailability {
        let Some(clip) = self.selected_clip() else {
            return SplitAvailability::Blocked(SplitBlock::NoSelection);
        };
        let frame = self.timeline.playhead;
        match self.plan_split(clip.id.as_str(), frame, None) {
            Ok(_) => SplitAvailability::Ready {
                clip: clip.id,
                frame,
            },
            Err(error) => SplitAvailability::Blocked(match error.code {
                "out_of_bounds" | "invalid_duration" => SplitBlock::PlayheadOutside,
                "split_unsupported"
                    if error
                        .details
                        .as_ref()
                        .and_then(|details| details["reason"].as_str())
                        == Some("fades_longer_than_clip") =>
                {
                    SplitBlock::FadesLongerThanClip
                }
                "split_unsupported" => SplitBlock::FadeCrossesCut,
                _ => SplitBlock::Refused,
            }),
        }
    }

    /// Cuts `clip` at the absolute sequence `frame`. The left half keeps the ID and the
    /// clip's head; the right half gets `new_id` (or the default ID) and the tail, on the same
    /// track, with the same asset, and takes the selection.
    pub(super) fn split_clip(
        &mut self,
        clip: &str,
        frame: i64,
        new_id: Option<&str>,
    ) -> Result<Value, CommandError> {
        let plan = self.plan_split(clip, frame, new_id)?;
        let right_id = Id::new(&plan.right_id).map_err(|error| {
            CommandError::new("invalid_id", error.to_string()).target(plan.right_id.clone())
        })?;
        let Some(index) = self
            .timeline
            .clips
            .iter()
            .position(|entry| entry.id == plan.clip)
        else {
            return Err(CommandError::unknown_element("clip", clip));
        };

        // Nothing below can fail.
        let left = plan.clip;
        let mut right = self.timeline.clips[index].clone();
        let (track, media, start, end) = (
            right.track,
            right.media,
            self.timeline.clips[index].start,
            right.start + right.duration,
        );
        right.id = right_id;
        right.start = plan.frame;
        right.duration = plan.right_duration;
        right.source_start = plan.right_source_start;
        right.camera = plan.right_camera;
        self.timeline.clips[index].duration = plan.left_duration;
        self.timeline.clips.insert(index + 1, right);

        self.inspector
            .duplicate_overrides(left.as_str(), right_id.as_str());
        let mut adjusted = Vec::new();
        for adjustment in &plan.adjustments {
            let owner = match adjustment.half {
                Half::Left => left,
                Half::Right => right_id,
            };
            let previous = property_def(adjustment.property)
                .map(|property| self.inspector.value(owner, property).to_json());
            let stored =
                self.inspector
                    .assign_exact(owner.as_str(), adjustment.property, adjustment.value);
            adjusted.push(json!({
                "clip": format!("clip:{owner}"), "property": adjustment.property,
                "value": stored, "previous": previous,
            }));
        }

        self.timeline.selected_clip = Some(right_id);
        self.timeline.selected_track = Some(track);
        if let Some(media) = media {
            self.selected_media = Some(media);
        }
        self.subject = Subject::Clip;

        Ok(json!({
            "target": format!("clip:{left}"), "action": "split", "frame": plan.frame,
            "track": format!("track:{track}"),
            "left": { "id": format!("clip:{left}"), "start": start,
                "end": plan.frame, "duration": plan.left_duration },
            "right": { "id": format!("clip:{right_id}"), "start": plan.frame,
                "end": end, "duration": plan.right_duration,
                "source_start": plan.right_source_start.to_json() },
            "new_id": right_id.as_str(), "selected": format!("clip:{right_id}"),
            "adjusted": adjusted, "changed": true,
        }))
    }
}
