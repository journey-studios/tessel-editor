//! BL081.2: the fade envelope of a cut clip.
//!
//! A fade is two inspector numbers measured from the edges of its clip. A cut inside a ramp
//! leaves each half with a piece of a ramp that began (or ends) outside it, so the half needs
//! to know where the ramp of the **original** clip sat. Each half carries a [`FadeWindow`]: one
//! optional [`Reach`] per ramp (`lead`: fade-in, `tail`: fade-out), in frames **relative to the
//! clip's own edges** (so a move never ages it):
//!
//! * `lead.frames`: frames from the origin of the fade-in ramp to the clip's first frame;
//! * `tail.frames`: frames from the clip's end to the end of the fade-out ramp.
//!
//! A ramp has two independent facts, and the model keeps them apart (BL081.2.2):
//!
//! * its **anchor** (`Reach::frames`): where the ramp starts (in) or ends (out);
//! * its **normalization** (`Reach::norm`, sound only): the span the mixer shortened the ramp
//!   against ([`crate::audio::resolve_fades`]) and the requested length of the *other* ramp in
//!   that span. A sound has to freeze it per ramp: when the user edits one fade of a half, the
//!   other ramp must keep its shape and place, and that shape came from the whole clip.
//!
//! The picture ([`crate::plan`], opacity) measures each fade from its own anchor, and without a
//! window from the clip itself, which is what every clip did before. The mixer
//! (`plan_audio`) resolves each pinned ramp on its frozen span and each ramp without a pin on
//! the clip itself (two plain ramps are resolved together, exactly as before). The rules of the
//! window (cut, trim, edit, load) live here, once.
//!
//! Each ramp remembers the fade value it was cut for (`Reach::fade`). It is only live while the
//! property still has that value, so an edit hands that fade over to the half (its inherited
//! ramp is dropped, the other ramp is untouched) and editing back to the old number cannot
//! bring a stale ramp back ([`EditorState::sweep_fade_windows`]).

use serde_json::{Value, json};

use super::EditorState;
use super::error::CommandError;
use super::inspector::{PropertyKind, Value as PropertyValue, property_def};
use super::timeline::{Clip, MAX_LENGTH_FRAMES};

/// How the mixer normalizes a pinned sound ramp (see the module docs). Frames, relative to the
/// clip's edges, so a move never ages it; a trim keeps it in place on the sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Norm {
    /// Frames from the clip's **opposite** edge to the far end of the span the ramps were
    /// normalized on (for the fade-in: past the clip's end; for the fade-out: before its start).
    /// Signed: a trim can leave the span's end inside the clip.
    pub far: i64,
    /// The other ramp's requested length (whole frames, `>= 0`) in that span.
    pub peer: i64,
}

/// One ramp a cut left on a half: where it is anchored and the fade value it was cut for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reach {
    /// Frames past the edge. `> 0` for a picture; `>= 0` for a sound (a sound ramp anchored on
    /// the edge can still need its [`Norm`]).
    pub frames: i64,
    /// The value of the fade property this reach was cut for (the guard).
    pub fade: f64,
    /// Present exactly on the ramps of an audio clip.
    pub norm: Option<Norm>,
}

/// Where the fades of a clip are measured (see the module docs). Empty = the clip's own edges.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FadeWindow {
    pub lead: Option<Reach>,
    pub tail: Option<Reach>,
}

/// The live anchors of a window, in frames, for the fade values the clip has right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub lead: i64,
    pub tail: i64,
}

impl FadeWindow {
    pub fn is_empty(&self) -> bool {
        self.lead.is_none() && self.tail.is_none()
    }

    /// The extent that is still valid for the clip's current `fade_in`/`fade_out` values: a side
    /// whose guard no longer matches the property does not count.
    pub fn live(&self, fade_in: f64, fade_out: f64) -> Extent {
        let (lead, tail) = self.live_reach(fade_in, fade_out);
        Extent {
            lead: lead.map_or(0, |reach| reach.frames),
            tail: tail.map_or(0, |reach| reach.frames),
        }
    }

    /// The ramps whose guard still matches the clip's current fade values.
    pub fn live_reach(&self, fade_in: f64, fade_out: f64) -> (Option<Reach>, Option<Reach>) {
        (
            self.lead.filter(|reach| reach.fade == fade_in),
            self.tail.filter(|reach| reach.fade == fade_out),
        )
    }

    /// `{lead, tail}` as the document stores it; `None` when empty.
    pub fn to_json(&self) -> Option<Value> {
        if self.is_empty() {
            return None;
        }
        let side = |reach: Option<Reach>| {
            reach.map(|reach| {
                let mut side = json!({ "frames": reach.frames, "fade": reach.fade });
                if let Some(norm) = reach.norm {
                    side["norm"] = json!({ "far": norm.far, "peer": norm.peer });
                }
                side
            })
        };
        let mut map = serde_json::Map::new();
        if let Some(lead) = side(self.lead) {
            map.insert("lead".into(), lead);
        }
        if let Some(tail) = side(self.tail) {
            map.insert("tail".into(), tail);
        }
        Some(Value::Object(map))
    }

    /// Parses and validates a stored window of a clip of `duration` frames (`audio`: a sound,
    /// whose ramps carry a `norm`; a picture's never do). A window the editor itself could not
    /// have written is refused (`Err` carries the reason), so a reopened project never holds one.
    pub fn from_json(value: &Value, duration: i64, audio: bool) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "`fade_window` must be an object".to_string())?;
        if let Some(key) = object
            .keys()
            .find(|key| !matches!(key.as_str(), "lead" | "tail"))
        {
            return Err(format!("`fade_window` has an unknown field `{key}`"));
        }
        let side = |name: &str| -> Result<Option<Reach>, String> {
            let Some(raw) = object.get(name) else {
                return Ok(None);
            };
            let raw = raw
                .as_object()
                .ok_or_else(|| format!("`fade_window.{name}` must be an object"))?;
            if let Some(key) = raw
                .keys()
                .find(|key| !matches!(key.as_str(), "frames" | "fade" | "norm"))
            {
                return Err(format!("`fade_window.{name}` has an unknown field `{key}`"));
            }
            let floor = if audio { 0 } else { 1 };
            let frames = raw
                .get("frames")
                .and_then(Value::as_i64)
                .filter(|frames| *frames >= floor && *frames <= MAX_LENGTH_FRAMES)
                .ok_or_else(|| {
                    format!("`fade_window.{name}.frames` must be an integer >= {floor}")
                })?;
            let fade = raw
                .get("fade")
                .and_then(Value::as_f64)
                .filter(|fade| {
                    fade.is_finite() && *fade >= 0.0 && *fade <= MAX_LENGTH_FRAMES as f64
                })
                .ok_or_else(|| format!("`fade_window.{name}.fade` must be a finite number >= 0"))?;
            let norm = match (raw.get("norm"), audio) {
                (Some(_), false) => {
                    return Err(format!(
                        "`fade_window.{name}.norm` belongs to a sound; this clip is a picture"
                    ));
                }
                (None, true) => {
                    return Err(format!(
                        "`fade_window.{name}` of a sound needs its `norm` (far, peer)"
                    ));
                }
                (None, false) => None,
                (Some(norm), true) => Some(parse_norm(norm, name)?),
            };
            Ok(Some(Reach { frames, fade, norm }))
        };
        let window = Self {
            lead: side("lead")?,
            tail: side("tail")?,
        };
        if window.is_empty() {
            return Err("`fade_window` has neither `lead` nor `tail`".to_string());
        }
        if window.checked_frames(duration).is_none() {
            return Err(format!(
                "`fade_window` is longer than {MAX_LENGTH_FRAMES} frames with the clip"
            ));
        }
        Ok(window)
    }

    /// The one limit of a loaded window and of a newly trimmed clip: the anchors with the clip
    /// fit [`MAX_LENGTH_FRAMES`] and so does the span each frozen normalization measures.
    /// Returns the frames the anchors and the clip add up to.
    pub fn checked_frames(&self, duration: i64) -> Option<i64> {
        let total = self
            .lead
            .map_or(0, |reach| reach.frames)
            .checked_add(duration)?
            .checked_add(self.tail.map_or(0, |reach| reach.frames))
            .filter(|total| duration > 0 && *total <= MAX_LENGTH_FRAMES)?;
        for reach in [self.lead, self.tail].into_iter().flatten() {
            let Some(norm) = reach.norm else {
                continue;
            };
            // The span is what a trim never changes (the anchor and `far` move together).
            reach
                .frames
                .checked_add(duration)?
                .checked_add(norm.far)
                .filter(|span| (1..=MAX_LENGTH_FRAMES).contains(span))?;
            if !(0..=MAX_LENGTH_FRAMES).contains(&norm.peer) {
                return None;
            }
        }
        Some(total)
    }

    /// Moves the window with a trim of the clip: the ramps stay where they are in the sequence,
    /// so trimming an edge inward lengthens that side and outward shortens it. A ramp whose
    /// anchor passes its origin is gone (the edge went past it: the fade is then measured on the
    /// edge, as on any clip). The span a sound ramp was normalized on also stays in place.
    /// `None` on overflow.
    pub fn trimmed(&self, head_shift: i64, tail_shift: i64) -> Option<Self> {
        let side = |reach: Option<Reach>, near: i64, far: i64| -> Option<Option<Reach>> {
            let Some(reach) = reach else {
                return Some(None);
            };
            // A pin at the current edge only preserves normalization. Trimming that edge
            // starts a fresh edge ramp, exactly as on a clip without inherited metadata.
            if reach.frames == 0 && near != 0 {
                return Some(None);
            }
            let frames = reach.frames.checked_add(near)?;
            let floor = if reach.norm.is_some() { 0 } else { 1 };
            if frames < floor {
                return Some(None);
            }
            let norm = match reach.norm {
                Some(norm) => Some(Norm {
                    far: norm.far.checked_add(far)?,
                    ..norm
                }),
                None => None,
            };
            Some(Some(Reach {
                frames,
                norm,
                ..reach
            }))
        };
        Some(Self {
            lead: side(self.lead, head_shift, tail_shift)?,
            tail: side(self.tail, tail_shift, head_shift)?,
        })
    }
}

fn parse_norm(value: &Value, name: &str) -> Result<Norm, String> {
    let raw = value
        .as_object()
        .ok_or_else(|| format!("`fade_window.{name}.norm` must be an object"))?;
    if let Some(key) = raw
        .keys()
        .find(|key| !matches!(key.as_str(), "far" | "peer"))
    {
        return Err(format!(
            "`fade_window.{name}.norm` has an unknown field `{key}`"
        ));
    }
    let far = raw
        .get("far")
        .and_then(Value::as_i64)
        .filter(|far| far.unsigned_abs() <= MAX_LENGTH_FRAMES as u64)
        .ok_or_else(|| format!("`fade_window.{name}.norm.far` must be an integer"))?;
    let peer = raw
        .get("peer")
        .and_then(Value::as_i64)
        .filter(|peer| (0..=MAX_LENGTH_FRAMES).contains(peer))
        .ok_or_else(|| format!("`fade_window.{name}.norm.peer` must be an integer >= 0"))?;
    Ok(Norm { far, peer })
}

impl Clip {
    /// First frame and end of the span the fades of a **picture** are measured on, for the clip's
    /// current fade values (`start - lead`, `end + tail`). The clip itself when it has no live
    /// window.
    pub fn fade_bounds(&self, fade_in: f64, fade_out: f64) -> (i64, i64) {
        let extent = self.fade_window.live(fade_in, fade_out);
        (
            self.start.saturating_sub(extent.lead),
            self.end().saturating_add(extent.tail),
        )
    }
}

// ---- sound: the group a ramp is resolved in --------------------------------------------------

/// The group one sound ramp is resolved in: everything `plan_audio` needs to give the ramp its
/// length, whatever the other fade is now. All in frames, relative to the clip's edges.
///
/// The ramp starts (in) or ends (out) `anchor` frames beyond the clip's near edge. The mixer
/// shortens it ([`crate::audio::resolve_fades`]) against the span that goes from that anchor to
/// `far` frames beyond the clip's opposite edge, together with the `peer` (the other ramp's
/// request, measured from that far end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RampGroup {
    /// The ramp's own request (whole frames, `>= 0`; `0` is no ramp).
    pub own: i64,
    pub anchor: i64,
    pub far: i64,
    pub peer: i64,
}

impl RampGroup {
    /// Frames of the span the group is resolved on.
    pub fn span(&self, duration: i64) -> Option<i64> {
        self.anchor.checked_add(duration)?.checked_add(self.far)
    }
}

/// The two ramps of a sound as the mixer reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioRamps {
    pub fade_in: RampGroup,
    pub fade_out: RampGroup,
}

/// A sound's fade as the mixer rounds it: whole frames.
pub fn ramp_frames(raw: f64) -> i64 {
    (raw.round() as i64).max(0)
}

/// The groups of a sound clip from its live pins and its fade values. A ramp with a pin is
/// resolved on its frozen group; a ramp without one on the clip's own edges, together with the
/// other ramp when that one has no pin either (the pair a clip always had), alone otherwise
/// (the other one is frozen: it does not move for this one).
pub fn audio_ramps(
    pin_in: Option<Reach>,
    pin_out: Option<Reach>,
    raw_in: f64,
    raw_out: f64,
) -> AudioRamps {
    let (own_in, own_out) = (ramp_frames(raw_in), ramp_frames(raw_out));
    let group = |own: i64, pin: Option<Reach>, other_own: i64, other_pinned: bool| match pin {
        Some(Reach {
            frames,
            norm: Some(norm),
            ..
        }) => RampGroup {
            own,
            anchor: frames,
            far: norm.far,
            peer: norm.peer,
        },
        _ => RampGroup {
            own,
            anchor: 0,
            far: 0,
            peer: if other_pinned { 0 } else { other_own },
        },
    };
    let pinned = |pin: Option<Reach>| pin.is_some_and(|reach| reach.norm.is_some());
    AudioRamps {
        fade_in: group(own_in, pin_in, own_out, pinned(pin_out)),
        fade_out: group(own_out, pin_out, own_in, pinned(pin_in)),
    }
}

// ---- the cut ---------------------------------------------------------------------------------

/// What one half of a cut keeps of the clip's fades.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HalfFades {
    /// The half's `fade_in` / `fade_out` property after the cut (`true`: set to 0).
    pub zero_in: bool,
    pub zero_out: bool,
    pub window: FadeWindow,
}

/// The fades of both halves of a cut, from the fades of the whole clip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutFades {
    pub left: HalfFades,
    pub right: HalfFades,
}

/// What a cut needs to know about the clip being cut (see the module docs and the report).
#[derive(Debug, Clone, Copy)]
pub struct CutInput {
    /// A sound fades by whole frames, resolves its ramps in groups and freezes each of them.
    pub audio: bool,
    pub before: i64,
    pub after: i64,
    /// The clip's live ramps (a guard that no longer matches is not passed in).
    pub lead: Option<Reach>,
    pub tail: Option<Reach>,
    /// The fade properties as stored (the guards).
    pub raw_in: f64,
    pub raw_out: f64,
}

/// Splits the fades of a clip at the cut. `None` when a frame count overflows (the cut is
/// refused, atomically).
///
/// A ramp that does not reach a half is zeroed on it (as the editor always did for cuts outside
/// the ramps); a ramp that does reach it stays on both halves. A half only gets a window for a
/// ramp measuring it on the half's own edges would change.
pub fn split_fades(input: CutInput) -> Option<CutFades> {
    if input.audio {
        split_audio(input)
    } else {
        split_visual(input)
    }
}

fn split_visual(input: CutInput) -> Option<CutFades> {
    let CutInput {
        before,
        after,
        lead,
        tail,
        raw_in: fi,
        raw_out: fo,
        ..
    } = input;
    let extent = Extent {
        lead: lead.map_or(0, |reach| reach.frames),
        tail: tail.map_or(0, |reach| reach.frames),
    };
    let whole = extent
        .lead
        .checked_add(before)?
        .checked_add(after)?
        .checked_add(extent.tail)?;
    if whole > MAX_LENGTH_FRAMES {
        return None;
    }
    let left_tail = extent.tail.checked_add(after)?;
    let right_lead = extent.lead.checked_add(before)?;

    // Gain is already 1 on every frame of the half: the ramp does not reach it.
    let in_reaches_right = fi > 0.0 && ((extent.lead + before) as f64) < fi;
    let out_reaches_left = fo > 0.0 && ((extent.tail + after + 1) as f64) < fo;
    let zero_in_right = fi > 0.0 && !in_reaches_right;
    let zero_out_left = fo > 0.0 && !out_reaches_left;

    let reach = |frames: i64, fade: f64| {
        (frames > 0).then_some(Reach {
            frames,
            fade,
            norm: None,
        })
    };
    // A half needs the window unless measuring on its own edges gives the same envelope.
    let left_in = fi; // the left half always keeps its fade-in
    let left_out = if zero_out_left { 0.0 } else { fo };
    let left_needs = left_out > 0.0 || (left_in > 0.0 && extent.lead > 0);
    let right_in = if zero_in_right { 0.0 } else { fi };
    let right_out = fo; // the right half always keeps its fade-out
    let right_needs = right_in > 0.0 || (right_out > 0.0 && extent.tail > 0);

    let left_window = if left_needs {
        FadeWindow {
            lead: (left_in > 0.0).then(|| reach(extent.lead, fi)).flatten(),
            tail: (left_out > 0.0).then(|| reach(left_tail, fo)).flatten(),
        }
    } else {
        FadeWindow::default()
    };
    let right_window = if right_needs {
        FadeWindow {
            lead: (right_in > 0.0).then(|| reach(right_lead, fi)).flatten(),
            tail: (right_out > 0.0).then(|| reach(extent.tail, fo)).flatten(),
        }
    } else {
        FadeWindow::default()
    };
    Some(CutFades {
        left: HalfFades {
            zero_in: false,
            zero_out: zero_out_left,
            window: left_window,
        },
        right: HalfFades {
            zero_in: zero_in_right,
            zero_out: false,
            window: right_window,
        },
    })
}

/// The longest fade an edit can set: the end of the fade sliders (a command, a gesture and the
/// CLI clamp to it).
fn edit_limit() -> i64 {
    ["fade_in", "fade_out"]
        .into_iter()
        .filter_map(property_def)
        .filter_map(|def| match def.kind {
            PropertyKind::Slider { max, .. } => Some(max.ceil() as i64),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

/// Whether the half of `duration` frames reads a ramp exactly as its `group` does **and keeps
/// reading it so whatever fade the user sets next**: the ramp is anchored on the edge and neither
/// the span nor the other fade, current or edited to anything the sliders allow, can make the
/// mixer shorten it ([`crate::audio::resolve_fades`] only shortens when `own + peer` exceeds the
/// span). Then the ramp is the same ramp measured on the half's own edges, in whole frames and so
/// on every grid (the sample count of an unshortened ramp is the clock difference of its own two
/// frames), and nothing has to be remembered about it. `native_peer`: the other fade of the half
/// if it is not pinned.
fn plain(group: &RampGroup, duration: i64, native_peer: i64) -> bool {
    if group.anchor != 0 {
        return false;
    }
    let span = group.span(duration).unwrap_or(0);
    group.own.saturating_add(group.peer) <= span
        && group.own.saturating_add(native_peer.max(edit_limit())) <= duration
}

/// Which ramps of a half need a frozen group (`(fade_in, fade_out)`). A ramp without one is
/// resolved on the half's own edges together with the other one when that has none either, and
/// alone when it has. The maximum editable peer already bounds the pin decision.
fn needs_pins(
    duration: i64,
    fade_in: Option<RampGroup>,
    fade_out: Option<RampGroup>,
) -> (bool, bool) {
    let own = |group: Option<RampGroup>| group.map_or(0, |group| group.own);
    let needs = |group: Option<RampGroup>, native_peer: i64| {
        group.is_some_and(|group| !plain(&group, duration, native_peer))
    };
    (needs(fade_in, own(fade_out)), needs(fade_out, own(fade_in)))
}

fn split_audio(input: CutInput) -> Option<CutFades> {
    let CutInput {
        before,
        after,
        lead,
        tail,
        raw_in,
        raw_out,
        ..
    } = input;
    let AudioRamps {
        fade_in: gi,
        fade_out: go,
    } = audio_ramps(lead, tail, raw_in, raw_out);
    let duration = before.checked_add(after)?;
    // The anchors with the clip, and each frozen span, fit the limit the loader enforces.
    gi.anchor
        .checked_add(duration)?
        .checked_add(go.anchor)
        .filter(|whole| *whole <= MAX_LENGTH_FRAMES)?;
    for group in [&gi, &go] {
        group
            .span(duration)
            .filter(|span| (1..=MAX_LENGTH_FRAMES).contains(span))?;
    }

    // The ramp of each half in the group of the whole: a cut moves one edge, so the anchor of a
    // ramp that follows that edge and the far end of the other ramp move with it.
    let shifted = |group: RampGroup, near: i64, far: i64| -> Option<RampGroup> {
        Some(RampGroup {
            anchor: group.anchor.checked_add(near)?,
            far: group.far.checked_add(far)?,
            ..group
        })
    };
    let left_in = shifted(gi, 0, after)?;
    let left_out = shifted(go, after, 0)?;
    let right_in = shifted(gi, before, 0)?;
    let right_out = shifted(go, 0, before)?;

    // The mixer shortens overlapping or oversize ramps in their group (`resolve_fades`); a
    // group that did so keeps its fades on both halves (the whole's request stays readable
    // and nothing is zeroed). Otherwise a ramp the cut does not reach is zeroed on that half.
    let span_in = gi.span(duration)?;
    let span_out = go.span(duration)?;
    let overlap_in = gi.own.saturating_add(gi.peer) > span_in;
    let overlap_out = go.own.saturating_add(go.peer) > span_out;
    let zero_in_right = gi.own > 0 && !overlap_in && right_in.anchor >= gi.own;
    let zero_out_left = go.own > 0 && !overlap_out && left_out.anchor >= go.own;

    let present = |group: RampGroup, zeroed: bool| (group.own > 0 && !zeroed).then_some(group);
    let half = |duration: i64, fade_in: Option<RampGroup>, fade_out: Option<RampGroup>| {
        let (pin_in, pin_out) = needs_pins(duration, fade_in, fade_out);
        let pin = |group: Option<RampGroup>, pinned: bool, fade: f64| {
            group.filter(|_| pinned).map(|group| Reach {
                frames: group.anchor,
                fade,
                norm: Some(Norm {
                    far: group.far,
                    peer: group.peer,
                }),
            })
        };
        FadeWindow {
            lead: pin(fade_in, pin_in, raw_in),
            tail: pin(fade_out, pin_out, raw_out),
        }
    };
    let left_window = half(
        before,
        present(left_in, false),
        present(left_out, zero_out_left),
    );
    let right_window = half(
        after,
        present(right_in, zero_in_right),
        present(right_out, false),
    );
    Some(CutFades {
        left: HalfFades {
            zero_in: false,
            zero_out: zero_out_left,
            window: left_window,
        },
        right: HalfFades {
            zero_in: zero_in_right,
            zero_out: false,
            window: right_window,
        },
    })
}

fn fade_value(state: &EditorState, clip: &str, property: &str) -> f64 {
    match property_def(property).map(|def| state.inspector.value(clip, def)) {
        Some(PropertyValue::Number(number)) => number,
        _ => 0.0,
    }
}

impl EditorState {
    /// The `fade_in` / `fade_out` values of a clip exactly as stored (no rounding).
    pub fn fade_values(&self, clip: &str) -> (f64, f64) {
        (
            fade_value(self, clip, "fade_in"),
            fade_value(self, clip, "fade_out"),
        )
    }

    /// Drops every window side whose fade was edited since the cut (the property no longer has
    /// the guard's value) and every window that ends up empty. Cheap: it only looks at clips
    /// that carry a window. Returns whether something was dropped. Only the edited ramp goes:
    /// the frozen group of the other one is its own record.
    ///
    /// Run after each command and when a slider gesture is committed. A gesture that is still
    /// previewing is not swept, so cancelling it puts the inherited ramp back.
    pub fn sweep_fade_windows(&mut self) -> bool {
        let mut stale: Vec<(crate::id::Id, FadeWindow)> = Vec::new();
        for clip in std::iter::once(&self.timeline)
            .chain(self.sequences.iter())
            .flat_map(|timeline| timeline.clips.iter())
            .filter(|clip| !clip.fade_window.is_empty())
        {
            let (fade_in, fade_out) = self.fade_values(clip.id.as_str());
            let window = clip.fade_window;
            let (lead, tail) = window.live_reach(fade_in, fade_out);
            let swept = FadeWindow { lead, tail };
            if swept != window {
                stale.push((clip.id, swept));
            }
        }
        let changed = !stale.is_empty();
        for (id, window) in stale {
            for timeline in std::iter::once(&mut self.timeline).chain(self.sequences.iter_mut()) {
                if let Some(clip) = timeline.clips.iter_mut().find(|clip| clip.id == id) {
                    clip.fade_window = window;
                }
            }
        }
        changed
    }
}

/// The error of an envelope that does not fit the frame arithmetic.
pub fn overflow(target: String) -> CommandError {
    CommandError::new(
        "invalid_time",
        "The fade envelope of the cut overflows; nothing changed",
    )
    .target(target)
}

#[cfg(test)]
mod tests;
