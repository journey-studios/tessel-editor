//! Player and subtitle clips in the plan (F071): each becomes the shape and text layers of its
//! parts, like a group does (`group_layers`), so the compositor knows neither.
//!
//! Both read the sequence frame being drawn and nothing else (no clock of their own), which is
//! what makes a seek, the monitor, `render.frame` and an export agree: the same frame of the same
//! project always resolves to the same layers.

use std::sync::Arc;

use super::*;
use crate::audio::waveform_window;
use crate::player::PlayerClock;

impl EditorState {
    /// The layers of the player clip `clip` at sequence `frame`; empty (with a warning) when the
    /// clip has no player, so a damaged clip never stops a frame from being drawn.
    pub(super) fn player_layers(
        &self,
        resolver: &mut Resolver<'_>,
        clip: &Clip,
        frame: i64,
    ) -> Vec<Layer> {
        let Some(spec) = &clip.player else {
            resolver.warnings.push(Warning {
                code: "player_without_spec",
                message: format!("Player `{}` has no specification and is skipped", clip.id),
                clip: Some(clip.id),
            });
            return Vec::new();
        };
        let timeline = &self.timeline;
        let followed = spec.audio.and_then(|id| {
            timeline
                .clip(id.as_str())
                .filter(|other| other.kind == ClipKind::Audio)
        });
        if let (Some(link), None) = (spec.audio, followed) {
            resolver.warnings.push(Warning {
                code: "player_audio_missing",
                message: format!(
                    "Player `{}` follows `{link}`, which is not an audio clip of the sequence: \
                     it follows its own clip instead",
                    clip.id
                ),
                clip: Some(clip.id),
            });
        }
        // The span the player follows: the audio clip it is linked to, else its own clip.
        let (span_start, span_frames) = followed.map_or((clip.start, clip.duration), |audio| {
            (audio.start, audio.duration)
        });
        let rate = timeline.rate;
        let clock = PlayerClock::at(
            rate.frame_to_time(frame - span_start).to_f64(),
            rate.frame_to_time(span_frames).to_f64(),
        );
        let peaks = match followed.filter(|_| spec.wave) {
            Some(audio) => self.player_peaks(resolver, clip, audio, spec.bars),
            None => None,
        };
        let placed = spec.place(
            peaks.as_deref().map_or(&[], Vec::as_slice),
            clock,
            (timeline.width, timeline.height),
        );
        self.placed_layers(resolver, clip, frame, placed)
    }

    /// The bar heights of `audio`'s waveform over the part of its source the clip plays, or
    /// `None` (with a warning on `player`) when its sound cannot be read: the bars are then flat,
    /// and the progress and the timer still run.
    fn player_peaks(
        &self,
        resolver: &mut Resolver<'_>,
        player: &Clip,
        audio: &Clip,
        bars: u32,
    ) -> Option<Arc<Vec<f32>>> {
        let unavailable = |resolver: &mut Resolver<'_>, why: String| {
            resolver.warnings.push(Warning {
                code: "player_audio_unavailable",
                message: format!(
                    "The waveform of player `{}` is flat: audio clip `{}` {why}",
                    player.id, audio.id
                ),
                clip: Some(player.id),
            });
            None
        };
        let Some(item) = audio.media.and_then(|media| self.library.item(media)) else {
            return unavailable(resolver, "has no media in the library".to_string());
        };
        let Some(asset) = &item.asset else {
            return unavailable(resolver, format!("has no media behind `{}`", item.id));
        };
        let Source::File { path } = &asset.source else {
            return unavailable(resolver, format!("`{}` has no audio file", item.id));
        };
        let index = resolver.number(audio, "audio_stream").round() as u32;
        let Some(stream) = asset.meta.audio_stream(index) else {
            return unavailable(resolver, format!("has no audio stream {index}"));
        };
        // The file the mixer reads (`plan_audio`): the decoded copy of a compressed source, or a
        // WAV original as it is.
        let decoded = asset
            .decoded
            .as_ref()
            .and_then(|decoded| decoded.audio_path(stream.index));
        let direct = stream.index == 0 && asset.meta.format.as_deref() == Some("wav");
        let Some(file) = decoded.or(direct.then_some(path.as_str())) else {
            return unavailable(resolver, format!("`{}` is not decoded yet", item.id));
        };
        // The window of the file the clip plays, on the file's own clock.
        let speed = resolver.speed_ratio(audio);
        let window = audio.source_start.sub(stream.offset).and_then(|from| {
            let span = self
                .timeline
                .rate
                .frame_to_time(audio.duration)
                .mul(speed)?;
            Some((from.to_f64(), from.add(span)?.to_f64()))
        });
        let Some((from, to)) = window else {
            return unavailable(resolver, "has a time that overflows".to_string());
        };
        match waveform_window(file, from, to, bars as usize) {
            Ok(peaks) => Some(peaks),
            Err(message) => unavailable(resolver, format!("cannot be read: {message}")),
        }
    }

    /// The layers of the subtitle clip `clip` at sequence `frame`: the box and text of the cue
    /// the clip's content clock is on, if any.
    pub(super) fn subtitle_layers(
        &self,
        resolver: &mut Resolver<'_>,
        clip: &Clip,
        frame: i64,
    ) -> Vec<Layer> {
        let Some(spec) = &clip.subtitles else {
            resolver.warnings.push(Warning {
                code: "subtitles_without_spec",
                message: format!("Subtitles `{}` have no cues and are skipped", clip.id),
                clip: Some(clip.id),
            });
            return Vec::new();
        };
        // The clip reads its cues like a video reads its file: from `source_start`, so a trimmed
        // or cut clip keeps every cue on the frame it was on.
        let time = resolver.source_time(clip, frame);
        let placed = spec.place(time, (self.timeline.width, self.timeline.height));
        self.placed_layers(resolver, clip, frame, placed)
    }
}
