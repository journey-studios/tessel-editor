//! `scene.fx.seek` (F058, P3, BL082.30): moves the monitor to a key of an effect parameter.
//!
//! The core decides everything an adapter must not: which key (`previous`/`next` of the frame the
//! monitor shows, `first`, `last`, or the one at a clip frame), the bundle frame it is on (the
//! clip's start plus the key's local frame), the exact time of that frame by the bundle's own
//! clock, and the sequence frame whose time shows it. The sequence and the bundle may run at
//! different rates (24 against 30000/1001), so a bundle frame is not always on the sequence's grid:
//! the answer is then a typed refusal (`frame_not_on_sequence`), never a neighbouring frame, and a
//! target past either end of the sequence is refused, never clamped. The playhead moves; no
//! document, history entry or project revision does (`history_label` is `None`).

use serde_json::{Value, json};

use crate::state::{CommandError, EditorState};
use crate::time::Rate;

use super::command::KeySeek;

/// `ceil(time * rate)`: the first sequence frame that starts at or after `time`.
fn first_frame_at_or_after(time: crate::time::Ratio, rate: Rate) -> Option<i64> {
    let scaled = i128::from(time.num()) * i128::from(rate.num());
    let divisor = i128::from(time.den()) * i128::from(rate.den());
    let floor = scaled.div_euclid(divisor);
    let ceil = if scaled.rem_euclid(divisor) == 0 {
        floor
    } else {
        floor + 1
    };
    i64::try_from(ceil).ok()
}

impl EditorState {
    pub(super) fn fx_seek(
        &mut self,
        scene: &str,
        clip: &str,
        param: &str,
        to: &KeySeek,
    ) -> Result<Value, CommandError> {
        let entry = self.attached_scene(scene)?;
        let bundle = entry.scene.bundle();
        let (_, fx_clip) = bundle
            .fx
            .clip(clip)
            .ok_or_else(|| CommandError::unknown_element("effect clip", clip))?;
        let (index, spec) = fx_clip.effect.param(param).ok_or_else(|| {
            CommandError::new(
                "unknown_param",
                format!("`{param}` is not a parameter of {}", fx_clip.effect.id),
            )
            .target(clip)
        })?;
        let keys = fx_clip.curve(index).unwrap_or(&[]);
        let refuse = |code: &'static str, message: String| {
            CommandError::new(code, message)
                .target(clip)
                .details(json!({ "scene": scene, "clip": clip, "param": spec.id }))
        };
        if keys.is_empty() {
            return Err(refuse(
                "no_keys",
                format!("`{param}` of `{clip}` has no keys to go to"),
            ));
        }
        // Where the monitor is, in the clip's own frames (may be before or after the clip).
        let here = bundle
            .frame_at_time(self.timeline.time_of(self.timeline.playhead))?
            .checked_sub(fx_clip.start_frame);
        let local = match to {
            KeySeek::First => keys.first().map(|key| key.frame),
            KeySeek::Last => keys.last().map(|key| key.frame),
            KeySeek::Previous => here.and_then(|here| {
                keys.iter()
                    .rev()
                    .map(|key| key.frame)
                    .find(|frame| *frame < here)
            }),
            KeySeek::Next => {
                here.and_then(|here| keys.iter().map(|key| key.frame).find(|frame| *frame > here))
            }
            KeySeek::Frame(frame) => keys.iter().map(|key| key.frame).find(|k| k == frame),
        }
        .ok_or_else(|| match to {
            KeySeek::Previous => refuse(
                "no_previous_key",
                format!("`{param}` has no key before the frame the monitor shows"),
            ),
            KeySeek::Next => refuse(
                "no_next_key",
                format!("`{param}` has no key after the frame the monitor shows"),
            ),
            _ => refuse(
                "unknown_keyframe",
                format!("`{param}` has no key at the frame asked"),
            ),
        })?;

        let edit_frame = fx_clip.start_frame.checked_add(local).ok_or_else(|| {
            refuse(
                "frame_out_of_range",
                "the key is past the last frame".into(),
            )
        })?;
        let mapping = bundle.resolve_frame_mapping(edit_frame)?;
        let time = mapping.edit_time;
        let rate = self.timeline.rate;
        let sequence = first_frame_at_or_after(time, rate)
            .filter(|frame| (0..=self.timeline.length).contains(frame))
            .ok_or_else(|| {
                refuse(
                    "outside_sequence",
                    format!(
                        "the key is at {time} s, outside the sequence (0 to {} frames); nothing moved",
                        self.timeline.length
                    ),
                )
            })?;
        // The sequence frame must show exactly this bundle frame (the bundle's own clock decides).
        if bundle.frame_at_time(rate.frame_to_time(sequence))? != edit_frame {
            return Err(refuse(
                "frame_not_on_sequence",
                format!(
                    "frame {edit_frame} of the scene has no frame of the sequence ({}) that shows \
                     it; nothing moved",
                    rate.as_f64()
                ),
            ));
        }
        let moved = self.timeline.set_playhead(sequence);
        Ok(json!({
            "action": "scene.fx.seek", "scene": scene, "clip": clip, "param": spec.id,
            "frame_in_clip": local, "edit_frame": edit_frame,
            "time": time.to_json(), "playhead": sequence,
            "previous_playhead": moved["previous"],
            "changed": moved["changed"], "document_changed": false,
        }))
    }
}
