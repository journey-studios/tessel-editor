//! Performance channels (`resolve-performance.ts`): which channels an instance applies and the
//! offset each one gives at a frame of the instance.
//!
//! The arithmetic is the origin's, operation for operation and in the same order, including doing
//! the frame sums in `f64` the way JavaScript does, because the equivalence oracle compares the
//! offsets it produces and a bundle may carry frame numbers close to 2^53.

use std::collections::HashSet;
use std::f64::consts::PI;

use crate::directed_scene::{
    Bundle, ChannelMotion, ChannelProperty, PerformanceChannel, PerformanceTrack, SceneInstance,
    TemporalError, Vec3,
};

/// What one channel writes to one node property at a frame (`NodeTransformOverride`).
///
/// For `LocalRotation` (XYZ Euler degrees) and `LocalPosition` the value is an **offset** added to
/// the node's own transform; for `LocalScale` it **replaces** the node's scale. The merge is
/// `NodeFrame`'s, not this struct's.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeOverride {
    pub node_id: String,
    pub property: ChannelProperty,
    pub value: Vec3,
}

/// The channels an instance applies, resolved once: ask for any frame afterwards without walking
/// the tracks again, in any order.
///
/// A track applies when it belongs to the instance **and** the instance lists it in
/// `performanceTrackIds`; any other track is ignored, as in the origin (so a track left out of
/// the list may even conflict with the listed ones).
#[derive(Debug, Clone)]
pub struct PerformancePlan<'a> {
    instance_id: &'a str,
    tracks: Vec<&'a PerformanceTrack>,
    /// Channels in track order, then channel order: the order of the origin's override list.
    channels: Vec<&'a PerformanceChannel>,
}

impl<'a> PerformancePlan<'a> {
    /// Fails with `PerformanceWriterConflict` when two applied channels write the same node
    /// property. The origin throws that while evaluating the first frame of the instance, not while
    /// validating, so a bundle that has it still imports; this makes the same finding typed.
    pub(in crate::directed_scene) fn new(
        bundle: &'a Bundle,
        instance: &'a SceneInstance,
    ) -> Result<Self, TemporalError> {
        let tracks: Vec<&PerformanceTrack> = bundle
            .performance_tracks
            .iter()
            .filter(|track| {
                track.instance_id == instance.id
                    && instance.performance_track_ids.contains(&track.id)
                    && !track.channels.is_empty()
            })
            .collect();
        let channels: Vec<&PerformanceChannel> = tracks
            .iter()
            .flat_map(|track| track.channels.iter())
            .collect();
        let mut writers = HashSet::new();
        for channel in &channels {
            let property = channel.motion.property();
            if !writers.insert((channel.node_id.as_str(), property)) {
                return Err(TemporalError::PerformanceWriterConflict {
                    instance_id: instance.id.clone(),
                    node_id: channel.node_id.clone(),
                    property,
                });
            }
        }
        Ok(Self {
            instance_id: &instance.id,
            tracks,
            channels,
        })
    }

    pub fn instance_id(&self) -> &str {
        self.instance_id
    }

    /// Ids of the tracks that apply (those with at least one channel), in bundle order.
    pub fn track_ids(&self) -> Vec<&str> {
        self.tracks.iter().map(|track| track.id.as_str()).collect()
    }

    /// Number of channels that apply.
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// The offset every channel gives at `instance_frame`, in the origin's order. Pure: the same
    /// frame gives the same bits however the frames before it were asked.
    pub fn overrides_at(&self, instance_frame: u64) -> Vec<NodeOverride> {
        self.channels
            .iter()
            .map(|channel| NodeOverride {
                node_id: channel.node_id.clone(),
                property: channel.motion.property(),
                value: motion_value(&channel.motion, instance_frame),
            })
            .collect()
    }
}

/// Position of an event between its start (0) and its end (1); before it 0, after it 1 (hold).
fn event_progress(frame: f64, start: f64, duration: f64) -> f64 {
    if frame < start {
        return 0.0;
    }
    let end = start + duration;
    if frame >= end {
        return 1.0;
    }
    (frame - start) / duration
}

fn scaled(by: Vec3, factor: f64) -> Vec3 {
    Vec3 {
        x: by.x * factor,
        y: by.y * factor,
        z: by.z * factor,
    }
}

/// `resolvePerformanceOverrides` for one channel.
fn motion_value(motion: &ChannelMotion, instance_frame: u64) -> Vec3 {
    // Frame counts are integers below 2^53 (the importer refuses larger), exact as `f64`; the sums
    // below then round exactly like JavaScript's.
    let frame = instance_frame as f64;
    match motion {
        ChannelMotion::Static { value, .. } => *value,
        ChannelMotion::Cycle {
            period_frames,
            amplitude,
            axis,
            phase_offset_frames,
            ..
        } => {
            let period = *period_frames as f64;
            let phase = (frame + *phase_offset_frames as f64) % period;
            // A sine, not a ramp: a cycle must be smooth across the period boundary.
            let angle = ((phase / period) * PI * 2.0).sin() * amplitude;
            scaled(*axis, angle)
        }
        ChannelMotion::Event {
            start_instance_frame,
            duration_frames,
            delta,
            ..
        } => scaled(
            *delta,
            event_progress(frame, *start_instance_frame as f64, *duration_frames as f64),
        ),
    }
}
