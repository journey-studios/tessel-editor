//! Pure, deterministic evaluation of time in a directed scene (BB03): edit frame → shot →
//! instance frame → camera clip frame → camera pose.
//!
//! Everything here is a function of the imported `Bundle` and an integer frame: no clock, no
//! playback state, no random access cost. Seeking forward, backward, out of order or twice gives
//! the same answer, bit for bit. Frame numbers are integers; the only rational quantity is time,
//! and `Rate`/`Ratio` keep it exact at `24000/1001` and `30000/1001` (`FrameMapping` times).
//!
//! What this module evaluates and what it does not:
//!
//! | Question | Answer |
//! |---|---|
//! | Which shot, instance frame and clip frame does an edit frame play? | `Bundle::resolve_frame_mapping` |
//! | Camera pose and speed at an edit frame (recipes, keys, easings, layers) | `Bundle::evaluate_camera` |
//! | Camera of a clip at a clip frame, or many frames in any order | `Bundle::camera_at_clip_frame`, `Bundle::camera_timeline` |
//! | Everything above plus the nodes (performance channels, matrices) | `Bundle::evaluate_frame`, built on `animation` |
//! | Are continuous cuts between shots smooth? | `DirectedScene::temporal_join_issues` / `temporal_issues` |
//!
//! Contract, tolerances and what was compared with the origin: `docs/features/scene-temporal.md`.

mod camera;
mod frame;
mod gate;

use std::fmt;

use crate::state::CommandError;

use super::ChannelProperty;

pub(in crate::directed_scene) use camera::distance;
pub use camera::{
    CameraPose, CameraState, CameraTimeline, SegmentPoses, movement_end_pose, movement_pose_at,
};
pub use frame::{CameraEvaluation, FrameEvaluation, FrameMapping, NodeAnimation, Unsupported};
pub use gate::LAYER_FOV_SWEEP_MAX_FRAMES;
pub(in crate::directed_scene) use gate::layered_fov_issues;

/// Why a frame could not be evaluated. Nothing here panics: a bundle the importer accepted never
/// reaches the structural variants (`Unresolved`, `NoShotAtFrame`, ...) but the API stays total,
/// since a `Bundle` can also be built by hand.
#[derive(Debug, Clone, PartialEq)]
pub enum TemporalError {
    /// The bundle's frame rate is not one `time::Rate` can represent.
    InvalidFrameRate,
    /// The edit frame is outside `[0, editDurationFrames)`.
    EditFrameOutOfRange { frame: i64, duration_frames: u64 },
    /// No shot covers the edit frame (the importer guarantees shots tile the edit).
    NoShotAtFrame { frame: u64 },
    /// An id the shot points at does not exist (`kind`: `"scene instance"`, `"scene definition"`, `"camera clip"`).
    Unresolved { kind: &'static str, id: String },
    /// The shot maps the edit frame past the end of its instance.
    InstanceFrameOutOfRange {
        instance_id: String,
        frame: u64,
        duration_frames: u64,
    },
    /// A clip frame before 0.
    CameraFrameBeforeStart { clip_id: String, frame: i64 },
    /// A camera on its own target has no view to move along (`Camera direction is degenerate`).
    DegenerateCamera,
    /// An easing name outside `linear`, `easeInOutCubic`, `easeOutCubic`.
    UnknownEasing { easing: String },
    /// A keyframes movement without keys.
    EmptyKeyframes,
    /// A layer that holds keyframes (the importer refuses it: `CAMERA_LAYER_KEYFRAMES`).
    LayerKeyframes,
    /// A pose, speed or field of view that is not a finite number.
    NonFiniteCamera,
    /// Two channels the instance applies write the same node property (`Multiple performance
    /// channels write ...` in the origin, raised when its first frame is evaluated).
    PerformanceWriterConflict {
        instance_id: String,
        node_id: String,
        property: ChannelProperty,
    },
    /// A node whose transform or matrix is not finite at the frame (a channel or a hierarchy that
    /// overflows).
    NonFiniteNode { node_id: String },
    /// Frame or time arithmetic that does not fit the integer types.
    FrameOverflow,
    /// The question is outside what this evaluator computes; see `Unsupported`.
    Unsupported(Unsupported),
}

impl TemporalError {
    /// Stable machine-readable code (what `CommandError::code` carries).
    pub fn code(&self) -> &'static str {
        match self {
            Self::EditFrameOutOfRange { .. }
            | Self::InstanceFrameOutOfRange { .. }
            | Self::CameraFrameBeforeStart { .. } => "frame_out_of_range",
            Self::Unsupported(_) => "unsupported",
            Self::InvalidFrameRate | Self::FrameOverflow => "invalid_time",
            Self::PerformanceWriterConflict { .. } | Self::NonFiniteNode { .. } => {
                "invalid_animation"
            }
            _ => "invalid_camera",
        }
    }
}

impl fmt::Display for TemporalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The two range messages are the origin's own words, so they can be compared verbatim.
        match self {
            Self::InvalidFrameRate => write!(f, "the bundle's frame rate is not representable"),
            Self::EditFrameOutOfRange {
                frame,
                duration_frames,
            } => write!(f, "editFrame {frame} out of range [0, {duration_frames})"),
            Self::NoShotAtFrame { frame } => write!(f, "No shot covers editFrame {frame}"),
            Self::Unresolved { kind, id } => write!(f, "Unknown {kind}: {id}"),
            Self::InstanceFrameOutOfRange {
                instance_id,
                frame,
                duration_frames,
            } => write!(
                f,
                "instanceFrame {frame} exceeds instance duration {duration_frames} ({instance_id})"
            ),
            Self::CameraFrameBeforeStart { clip_id, frame } => {
                write!(f, "clipFrame {frame} is before the start of clip {clip_id}")
            }
            Self::DegenerateCamera => write!(f, "Camera direction is degenerate"),
            Self::UnknownEasing { easing } => write!(f, "Unknown camera easing: {easing}"),
            Self::EmptyKeyframes => write!(f, "A keyframes movement needs at least one key"),
            Self::LayerKeyframes => write!(f, "A layer cannot hold keyframes"),
            Self::NonFiniteCamera => write!(f, "the camera evaluates to a non-finite number"),
            Self::PerformanceWriterConflict {
                instance_id,
                node_id,
                property,
            } => write!(
                f,
                "Multiple performance channels write {node_id}:{} on instance {instance_id}",
                property.as_str()
            ),
            Self::NonFiniteNode { node_id } => write!(
                f,
                "node {node_id} evaluates to a non-finite transform at this frame"
            ),
            Self::FrameOverflow => write!(f, "frame or time arithmetic overflows"),
            Self::Unsupported(what) => write!(f, "not evaluated: {what}"),
        }
    }
}

impl std::error::Error for TemporalError {}

/// Adapters (CLI, UI) already speak `CommandError`.
impl From<TemporalError> for CommandError {
    fn from(error: TemporalError) -> Self {
        let command = CommandError::new(error.code(), error.to_string());
        match &error {
            TemporalError::Unsupported(what) => command.details(what.to_json()),
            _ => command,
        }
    }
}
