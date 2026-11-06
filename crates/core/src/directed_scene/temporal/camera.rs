//! Camera pose of a clip at one of its frames (`resolve-camera-clip.ts`, `easing.ts`,
//! `resolve-keyframes.ts`).
//!
//! The arithmetic is the origin's, operation for operation and in the same order, because the
//! equivalence oracle compares the poses it produces. Two deliberate differences:
//!
//! - Where the origin throws (a camera on its own target, an easing it does not know) this module
//!   returns a `TemporalError`. A clip is walked lazily like the origin does: a frame before the
//!   failing movement still evaluates.
//! - The origin walks the clip's movements from the start at every frame (`O(segments)` each).
//!   `CameraTimeline` walks them once and answers a frame by binary search, with the same carried
//!   poses, so a gate that checks every frame of a long clip stays cheap.

use crate::directed_scene::{
    CameraClip, CameraKey, CameraMotion, CameraSegment, Mat4, MoveRecipe, SegmentStart, Vec3,
    ZoomRecipe,
};

use super::TemporalError;

const DEGENERATE_LENGTH: f64 = 1e-9;
const DEG: f64 = std::f64::consts::PI / 180.0;

/// Where a camera stands, looks and how wide it sees (the origin's `CameraPose`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPose {
    pub position: Vec3,
    pub target: Vec3,
    pub fov_deg: f64,
}

impl CameraPose {
    /// The camera's placement in the scene, as Three.js `Camera.lookAt` + position builds it:
    /// the camera looks down its local -Z with +Y up. Not compared with the oracle (it records
    /// no matrix for the camera); the convention is Three.js `Matrix4.lookAt`, covered by tests.
    pub fn to_world_matrix(&self) -> Mat4 {
        let mut z = sub(self.position, self.target);
        if length_sq(z) == 0.0 {
            z.z = 1.0;
        }
        z = normalized(z);
        let up = Vec3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        };
        let mut x = cross(up, z);
        if length_sq(x) == 0.0 {
            // Looking straight up or down: Three.js nudges z so the cross product exists.
            z.z += 0.0001;
            z = normalized(z);
            x = cross(up, z);
        }
        x = normalized(x);
        let y = cross(z, x);
        let p = self.position;
        Mat4([
            x.x, x.y, x.z, 0.0, y.x, y.y, y.z, 0.0, z.x, z.y, z.z, 0.0, p.x, p.y, p.z, 1.0,
        ])
    }
}

/// A camera at one frame: its pose and how fast it is moving. Velocity is per clip frame (the
/// origin's `velocity` / `fovVelocityDegPerFrame`) and is what the continuous-join gate compares.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraState {
    pub position: Vec3,
    pub target: Vec3,
    pub fov_deg: f64,
    pub velocity: Vec3,
    pub fov_velocity_deg_per_frame: f64,
}

impl CameraState {
    pub fn pose(&self) -> CameraPose {
        CameraPose {
            position: self.position,
            target: self.target,
            fov_deg: self.fov_deg,
        }
    }

    fn is_finite(&self) -> bool {
        let (p, t, v) = (self.position, self.target, self.velocity);
        [
            p.x,
            p.y,
            p.z,
            t.x,
            t.y,
            t.z,
            v.x,
            v.y,
            v.z,
            self.fov_deg,
            self.fov_velocity_deg_per_frame,
        ]
        .iter()
        .all(|value| value.is_finite())
    }
}

/// The pose a main-track movement starts on and the one it ends on (`mainTrackPoses`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentPoses {
    pub start: CameraPose,
    pub end: CameraPose,
}

// ---- small vector helpers (Vec3 only carries what the graph needed) ----------------------------

fn add(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn scale(v: Vec3, s: f64) -> Vec3 {
    Vec3 {
        x: v.x * s,
        y: v.y * s,
        z: v.z * s,
    }
}

fn lerp_vec(a: Vec3, b: Vec3, t: f64) -> Vec3 {
    Vec3 {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
        z: a.z + (b.z - a.z) * t,
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

fn length_sq(v: Vec3) -> f64 {
    v.x * v.x + v.y * v.y + v.z * v.z
}

fn normalized(v: Vec3) -> Vec3 {
    scale(v, 1.0 / length_sq(v).sqrt())
}

/// `Math.hypot(x, y, z)` as V8 computes it (scale by the largest, Kahan sum), so a vector length
/// rounds like the origin's. `f64::hypot` is two-argument and rounds differently.
fn hypot3(v: Vec3) -> f64 {
    let abs = [v.x.abs(), v.y.abs(), v.z.abs()];
    if abs.iter().any(|a| a.is_infinite()) {
        return f64::INFINITY;
    }
    if abs.iter().any(|a| a.is_nan()) {
        return f64::NAN;
    }
    let max = abs.iter().copied().fold(0.0, f64::max);
    if max == 0.0 {
        return 0.0;
    }
    let (mut sum, mut compensation) = (0.0_f64, 0.0_f64);
    for value in abs {
        let n = value / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

pub(in crate::directed_scene) fn distance(a: Vec3, b: Vec3) -> f64 {
    hypot3(sub(a, b))
}

// ---- easing ---------------------------------------------------------------------------------

/// `applyEasing`: clamps to 0..1, then shapes. Only the three names the schema allows.
// WHY not `f64::clamp`: the origin answers a positive zero for `-0` and passes NaN through
// unchanged; `clamp` keeps the sign of a negative zero. The branches keep the origin's bits.
#[allow(clippy::manual_clamp)]
fn apply_easing(t: f64, easing: &str) -> Result<f64, TemporalError> {
    let t = if t <= 0.0 {
        0.0
    } else if t >= 1.0 {
        1.0
    } else {
        t
    };
    match easing {
        "linear" => Ok(t),
        "easeInOutCubic" if t < 0.5 => Ok(4.0 * t * t * t),
        "easeInOutCubic" => {
            let p = -2.0 * t + 2.0;
            Ok(1.0 - (p * p * p) / 2.0)
        }
        "easeOutCubic" => {
            let p = 1.0 - t;
            Ok(1.0 - p * p * p)
        }
        other => Err(TemporalError::UnknownEasing {
            easing: other.to_string(),
        }),
    }
}

// ---- recipes --------------------------------------------------------------------------------

fn forward(pose: &CameraPose) -> Result<Vec3, TemporalError> {
    let delta = sub(pose.target, pose.position);
    let len = hypot3(delta);
    if len < DEGENERATE_LENGTH {
        return Err(TemporalError::DegenerateCamera);
    }
    Ok(Vec3 {
        x: delta.x / len,
        y: delta.y / len,
        z: delta.z / len,
    })
}

fn shifted(pose: &CameraPose, delta: Vec3) -> CameraPose {
    CameraPose {
        position: add(pose.position, delta),
        target: add(pose.target, delta),
        fov_deg: pose.fov_deg,
    }
}

fn move_along_forward(pose: &CameraPose, distance: f64) -> Result<CameraPose, TemporalError> {
    Ok(shifted(pose, scale(forward(pose)?, distance)))
}

fn truck_right(pose: &CameraPose, distance: f64) -> Result<CameraPose, TemporalError> {
    let fwd = forward(pose)?;
    let up = Vec3 {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    };
    let right = cross(fwd, up);
    let rlen = hypot3(right);
    let mut unit = right;
    if rlen > DEGENERATE_LENGTH {
        unit = Vec3 {
            x: right.x / rlen,
            y: right.y / rlen,
            z: right.z / rlen,
        };
    }
    Ok(shifted(pose, scale(unit, distance)))
}

/// The camera turned around its target keeping its distance: yaw about the vertical (positive
/// moves it to its right), pitch up. Angles add to where the camera already is.
fn orbit_pose(
    pose: &CameraPose,
    yaw_deg: f64,
    pitch_deg: f64,
) -> Result<CameraPose, TemporalError> {
    let offset = sub(pose.position, pose.target);
    let radius = hypot3(offset);
    if radius < DEGENERATE_LENGTH {
        return Err(TemporalError::DegenerateCamera);
    }
    let azimuth = offset.x.atan2(offset.z) + yaw_deg * DEG;
    let elevation = (offset.y / radius).asin() + pitch_deg * DEG;
    Ok(CameraPose {
        position: Vec3 {
            x: pose.target.x + radius * elevation.cos() * azimuth.sin(),
            y: pose.target.y + radius * elevation.sin(),
            z: pose.target.z + radius * elevation.cos() * azimuth.cos(),
        },
        target: pose.target,
        fov_deg: pose.fov_deg,
    })
}

fn key_pose(key: &CameraKey) -> CameraPose {
    CameraPose {
        position: key.position,
        target: key.target,
        fov_deg: key.fov_deg,
    }
}

fn mix_pose(from: &CameraPose, to: &CameraPose, t: f64) -> CameraPose {
    CameraPose {
        position: lerp_vec(from.position, to.position, t),
        target: lerp_vec(from.target, to.target, t),
        fov_deg: lerp(from.fov_deg, to.fov_deg, t),
    }
}

/// The pose a movement ends on when it starts on `start` (`segmentEndPose`).
fn segment_end_pose(
    start: &CameraPose,
    segment: &CameraSegment,
) -> Result<CameraPose, TemporalError> {
    match &segment.motion {
        CameraMotion::Keyframes { keys } => keys
            .last()
            .map(key_pose)
            .ok_or(TemporalError::EmptyKeyframes),
        CameraMotion::Hold => Ok(*start),
        CameraMotion::Move {
            recipe,
            distance_scene_units,
            ..
        } => match recipe {
            MoveRecipe::DollyIn => move_along_forward(start, *distance_scene_units),
            MoveRecipe::DollyOut => move_along_forward(start, -distance_scene_units),
            MoveRecipe::TruckLeft => truck_right(start, -distance_scene_units),
            MoveRecipe::TruckRight => truck_right(start, *distance_scene_units),
        },
        CameraMotion::Zoom {
            recipe,
            fov_delta_deg,
            ..
        } => {
            let fov_deg = match recipe {
                ZoomRecipe::In => start.fov_deg - fov_delta_deg,
                ZoomRecipe::Out => start.fov_deg + fov_delta_deg,
            };
            Ok(CameraPose { fov_deg, ..*start })
        }
        CameraMotion::Orbit {
            yaw_deg, pitch_deg, ..
        } => orbit_pose(start, *yaw_deg, *pitch_deg),
    }
}

fn easing_of(motion: &CameraMotion) -> Option<&str> {
    match motion {
        CameraMotion::Move { easing, .. }
        | CameraMotion::Zoom { easing, .. }
        | CameraMotion::Orbit { easing, .. } => Some(easing),
        CameraMotion::Hold | CameraMotion::Keyframes { .. } => None,
    }
}

/// How far into a movement a frame is, eased: 0 on its first frame, 1 on its last. A hold has no
/// easing and keeps the linear ramp, as in the origin.
fn progress_at(segment: &CameraSegment, local: u64) -> Result<f64, TemporalError> {
    let span = segment.duration_frames.saturating_sub(1).max(1);
    let t = local as f64 / span as f64;
    match easing_of(&segment.motion) {
        Some(easing) => apply_easing(t, easing),
        None => Ok(t),
    }
}

/// The pose a movement has at `local` (0..durationFrames-1) of itself, given where it starts.
fn interpolate_segment(
    start: &CameraPose,
    segment: &CameraSegment,
    local: u64,
) -> Result<CameraPose, TemporalError> {
    if let CameraMotion::Keyframes { keys } = &segment.motion {
        return resolve_keys(start, keys, local);
    }
    let t = progress_at(segment, local)?;
    // WHY: an orbit follows the arc; a straight line between its ends would cut inside it.
    if let CameraMotion::Orbit {
        yaw_deg, pitch_deg, ..
    } = &segment.motion
    {
        return orbit_pose(start, yaw_deg * t, pitch_deg * t);
    }
    let end = segment_end_pose(start, segment)?;
    Ok(mix_pose(start, &end, t))
}

/// `resolveKeyframes` over the keys of a movement, with the pose it starts on as an implicit key
/// at its frame 0 (linear). Before the first key the track holds `start`, after the last it holds
/// the last key, and on a key's own frame it answers the key exactly (no blend at `t = 1`).
fn resolve_keys(
    start: &CameraPose,
    keys: &[CameraKey],
    local: u64,
) -> Result<CameraPose, TemporalError> {
    let frame = i64::try_from(local).unwrap_or(i64::MAX);
    if frame <= 0 {
        return Ok(*start);
    }
    let Some(arriving) = keys.iter().position(|key| frame <= key.frame) else {
        return keys
            .last()
            .map(key_pose)
            .ok_or(TemporalError::EmptyKeyframes);
    };
    let to = &keys[arriving];
    if frame == to.frame {
        return Ok(key_pose(to));
    }
    let (from_frame, from_pose) = match arriving.checked_sub(1) {
        Some(previous) => (keys[previous].frame, key_pose(&keys[previous])),
        None => (0, *start),
    };
    let t = apply_easing(
        (frame - from_frame) as f64 / (to.frame - from_frame) as f64,
        &to.easing,
    )?;
    Ok(mix_pose(&from_pose, &key_pose(to), t))
}

// ---- layers ---------------------------------------------------------------------------------

/// What a layer has moved the camera by, in the camera's own terms.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct LayerOffset {
    yaw_deg: f64,
    pitch_deg: f64,
    /// Along the view, positive in.
    forward: f64,
    /// Sideways, positive to the camera's right.
    right: f64,
    fov_deg: f64,
}

impl LayerOffset {
    fn plus(self, other: Self) -> Self {
        Self {
            yaw_deg: self.yaw_deg + other.yaw_deg,
            pitch_deg: self.pitch_deg + other.pitch_deg,
            forward: self.forward + other.forward,
            right: self.right + other.right,
            fov_deg: self.fov_deg + other.fov_deg,
        }
    }
}

/// The share `t` (0..1) of what a movement does, as an offset. `None` for keyframes: a layer
/// has no poses of its own.
fn segment_offset(segment: &CameraSegment, t: f64) -> Option<LayerOffset> {
    let none = LayerOffset::default();
    Some(match &segment.motion {
        CameraMotion::Hold => none,
        CameraMotion::Move {
            recipe,
            distance_scene_units: d,
            ..
        } => match recipe {
            MoveRecipe::DollyIn => LayerOffset {
                forward: d * t,
                ..none
            },
            MoveRecipe::DollyOut => LayerOffset {
                forward: -d * t,
                ..none
            },
            MoveRecipe::TruckRight => LayerOffset {
                right: d * t,
                ..none
            },
            MoveRecipe::TruckLeft => LayerOffset {
                right: -d * t,
                ..none
            },
        },
        CameraMotion::Zoom {
            recipe,
            fov_delta_deg: d,
            ..
        } => match recipe {
            ZoomRecipe::In => LayerOffset {
                fov_deg: -d * t,
                ..none
            },
            ZoomRecipe::Out => LayerOffset {
                fov_deg: d * t,
                ..none
            },
        },
        CameraMotion::Orbit {
            yaw_deg, pitch_deg, ..
        } => LayerOffset {
            yaw_deg: yaw_deg * t,
            pitch_deg: pitch_deg * t,
            ..none
        },
        CameraMotion::Keyframes { .. } => return None,
    })
}

/// A layer's movements laid out once: where each ends and what the earlier ones added up to.
#[derive(Clone)]
struct LayerTrack<'a> {
    segments: &'a [CameraSegment],
    ends: Vec<u64>,
    /// `done_before[i]`: everything movements `0..i` did, at their end.
    done_before: Vec<LayerOffset>,
    /// First keyframes movement. The importer refuses it (`CAMERA_LAYER_KEYFRAMES`); a frame that
    /// reaches it fails like the origin, frames before it still evaluate.
    keyframes_at: Option<usize>,
}

impl<'a> LayerTrack<'a> {
    fn new(segments: &'a [CameraSegment]) -> Result<Self, TemporalError> {
        let mut ends = Vec::with_capacity(segments.len());
        let mut done_before = Vec::with_capacity(segments.len() + 1);
        let mut done = LayerOffset::default();
        let mut cursor = 0_u64;
        let mut keyframes_at = None;
        for (index, segment) in segments.iter().enumerate() {
            done_before.push(done);
            cursor = cursor
                .checked_add(segment.duration_frames)
                .ok_or(TemporalError::FrameOverflow)?;
            ends.push(cursor);
            match segment_offset(segment, 1.0) {
                Some(full) => done = done.plus(full),
                None => {
                    keyframes_at.get_or_insert(index);
                }
            }
        }
        done_before.push(done);
        Ok(Self {
            segments,
            ends,
            done_before,
            keyframes_at,
        })
    }

    fn frames(&self) -> u64 {
        self.ends.last().copied().unwrap_or(0)
    }

    /// Everything the layer has moved by at `frame`; past its end, all of it.
    fn offset_at(&self, frame: u64) -> Result<LayerOffset, TemporalError> {
        let index = self.ends.partition_point(|&end| end <= frame);
        if self.keyframes_at.is_some_and(|first| index >= first) {
            return Err(TemporalError::LayerKeyframes);
        }
        let done = self.done_before[index];
        let Some(segment) = self.segments.get(index) else {
            return Ok(done);
        };
        let start = index
            .checked_sub(1)
            .map_or(0, |previous| self.ends[previous]);
        let t = progress_at(segment, frame - start)?;
        // `segment_offset` is `Some`: a keyframes movement would have been caught above.
        Ok(segment_offset(segment, t).map_or(done, |share| done.plus(share)))
    }
}

/// The main pose moved by the layers: turned around its target first, then slid sideways and
/// pushed along the turned view, then zoomed.
fn apply_layers(pose: &CameraPose, offset: LayerOffset) -> Result<CameraPose, TemporalError> {
    let turned = orbit_pose(pose, offset.yaw_deg, offset.pitch_deg)?;
    let moved = move_along_forward(&truck_right(&turned, offset.right)?, offset.forward)?;
    Ok(CameraPose {
        fov_deg: moved.fov_deg + offset.fov_deg,
        ..moved
    })
}

// ---- timeline -------------------------------------------------------------------------------

/// The main track laid out once.
#[derive(Clone)]
struct MainTrack<'a> {
    segments: &'a [CameraSegment],
    ends: Vec<u64>,
    /// The pose each movement starts on (after a `from: initial` reset), one per movement that
    /// could be reached. Shorter than `segments` when carrying the pose failed (`failure`).
    starts: Vec<CameraPose>,
    /// Where the camera rests after the last movement; `None` when carrying failed.
    rest: Option<CameraPose>,
    failure: Option<TemporalError>,
}

impl<'a> MainTrack<'a> {
    fn new(clip: &'a CameraClip) -> Result<Self, TemporalError> {
        let initial = CameraPose {
            position: clip.initial_position,
            target: clip.initial_target,
            fov_deg: clip.initial_fov_deg,
        };
        let mut ends = Vec::with_capacity(clip.segments.len());
        let mut starts = Vec::with_capacity(clip.segments.len());
        let mut carry = initial;
        let mut cursor = 0_u64;
        let mut failure = None;
        for segment in &clip.segments {
            cursor = cursor
                .checked_add(segment.duration_frames)
                .ok_or(TemporalError::FrameOverflow)?;
            ends.push(cursor);
            if failure.is_some() {
                continue;
            }
            let start = match segment.from {
                SegmentStart::Initial => initial,
                SegmentStart::PreviousEnd => carry,
            };
            starts.push(start);
            match segment_end_pose(&start, segment) {
                Ok(end) => carry = end,
                Err(error) => failure = Some(error),
            }
        }
        Ok(Self {
            segments: &clip.segments,
            ends,
            starts,
            rest: failure.is_none().then_some(carry),
            failure,
        })
    }

    fn frames(&self) -> u64 {
        self.ends.last().copied().unwrap_or(0)
    }

    fn failure(&self) -> TemporalError {
        self.failure
            .clone()
            .unwrap_or(TemporalError::DegenerateCamera)
    }

    /// The movement a frame falls in, with the pose it starts on and the frame within it; `None`
    /// past the clip's last movement.
    fn locate(
        &self,
        frame: u64,
    ) -> Result<Option<(&CameraSegment, CameraPose, u64)>, TemporalError> {
        let index = self.ends.partition_point(|&end| end <= frame);
        let Some(segment) = self.segments.get(index) else {
            return Ok(None);
        };
        let start = *self.starts.get(index).ok_or_else(|| self.failure())?;
        let begin = index
            .checked_sub(1)
            .map_or(0, |previous| self.ends[previous]);
        Ok(Some((segment, start, frame - begin)))
    }

    fn pose_at(&self, frame: u64) -> Result<CameraPose, TemporalError> {
        match self.locate(frame)? {
            Some((segment, start, local)) => interpolate_segment(&start, segment, local),
            None => self.rest.ok_or_else(|| self.failure()),
        }
    }

    /// Pose and per-frame speed. The speed is measured inside the movement: on its last frame it
    /// is 0, and past the clip's end the camera stands still (both as in the origin).
    fn state_at(&self, frame: u64) -> Result<CameraState, TemporalError> {
        let Some((segment, start, local)) = self.locate(frame)? else {
            let rest = self.rest.ok_or_else(|| self.failure())?;
            return Ok(state_of(rest, Vec3::ZERO, 0.0));
        };
        let current = interpolate_segment(&start, segment, local)?;
        let next_local = (local + 1).min(segment.duration_frames.saturating_sub(1));
        let next = interpolate_segment(&start, segment, next_local)?;
        Ok(state_of(
            current,
            sub(next.position, current.position),
            next.fov_deg - current.fov_deg,
        ))
    }
}

fn state_of(pose: CameraPose, velocity: Vec3, fov_velocity: f64) -> CameraState {
    CameraState {
        position: pose.position,
        target: pose.target,
        fov_deg: pose.fov_deg,
        velocity,
        fov_velocity_deg_per_frame: fov_velocity,
    }
}

/// The pose a single movement ends on when it starts on `start` (`segmentEndPose`). Pure and
/// stateless, for callers that rewrite a movement (scene editing turns a recipe into keys).
pub fn movement_end_pose(
    start: &CameraPose,
    segment: &CameraSegment,
) -> Result<CameraPose, TemporalError> {
    segment_end_pose(start, segment)
}

/// The pose a single movement has at `local` (0..durationFrames-1) of itself when it starts on
/// `start`: the arithmetic the resolver uses, so a conversion can reproduce it frame by frame
/// (`poseOfMovementAt`).
pub fn movement_pose_at(
    start: &CameraPose,
    segment: &CameraSegment,
    local: u64,
) -> Result<CameraPose, TemporalError> {
    interpolate_segment(start, segment, local)
}

/// A camera clip laid out for evaluation: build it once, ask for as many frames as you like in
/// any order. Every answer is a pure function of the clip and the frame, so asking twice, or
/// out of order, gives the same bits.
#[derive(Clone)]
pub struct CameraTimeline<'a> {
    clip: &'a CameraClip,
    main: MainTrack<'a>,
    layers: Vec<LayerTrack<'a>>,
}

impl<'a> CameraTimeline<'a> {
    pub(super) fn new(clip: &'a CameraClip) -> Result<Self, TemporalError> {
        Ok(Self {
            clip,
            main: MainTrack::new(clip)?,
            layers: clip
                .layers
                .iter()
                .map(|segments| LayerTrack::new(segments))
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn clip_id(&self) -> &str {
        &self.clip.id
    }

    /// Until the last of its tracks ends (`cameraClipFrames`): past that every track holds where
    /// it stopped.
    pub fn frames(&self) -> u64 {
        self.layers
            .iter()
            .map(LayerTrack::frames)
            .fold(self.main.frames(), u64::max)
    }

    /// Whether movements are stacked on the main track.
    pub fn has_layers(&self) -> bool {
        !self.layers.is_empty()
    }

    /// The main track alone with the layers on top, at a clip frame. A frame before the clip is an
    /// error; a frame after it holds the final pose with zero speed.
    pub fn state_at(&self, frame: i64) -> Result<CameraState, TemporalError> {
        let frame = self.clip_frame(frame)?;
        let state = self.state_at_frame(frame)?;
        if !state.is_finite() {
            return Err(TemporalError::NonFiniteCamera);
        }
        Ok(state)
    }

    /// Pose only: the main track plus layers, without the speed. What a field-of-view sweep needs.
    pub fn pose_at(&self, frame: i64) -> Result<CameraPose, TemporalError> {
        self.composed_pose(self.clip_frame(frame)?)
    }

    /// Pose each main-track movement starts and ends on (layers are not included: they add to
    /// these poses).
    pub fn main_track_poses(&self) -> Result<Vec<SegmentPoses>, TemporalError> {
        if let Some(error) = &self.main.failure {
            return Err(error.clone());
        }
        self.main
            .segments
            .iter()
            .zip(&self.main.starts)
            .map(|(segment, start)| {
                Ok(SegmentPoses {
                    start: *start,
                    end: segment_end_pose(start, segment)?,
                })
            })
            .collect()
    }

    fn clip_frame(&self, frame: i64) -> Result<u64, TemporalError> {
        u64::try_from(frame).map_err(|_| TemporalError::CameraFrameBeforeStart {
            clip_id: self.clip.id.clone(),
            frame,
        })
    }

    fn composed_pose(&self, frame: u64) -> Result<CameraPose, TemporalError> {
        let main = self.main.pose_at(frame)?;
        if self.layers.is_empty() {
            return Ok(main);
        }
        let mut offset = LayerOffset::default();
        for layer in &self.layers {
            offset = offset.plus(layer.offset_at(frame)?);
        }
        apply_layers(&main, offset)
    }

    fn state_at_frame(&self, frame: u64) -> Result<CameraState, TemporalError> {
        if self.layers.is_empty() {
            return self.main.state_at(frame);
        }
        let now = self.composed_pose(frame)?;
        // WHY: the layers change the pose from frame to frame on their own, so the speed is
        // measured on the composed pose; past the clip's end everything holds and the camera
        // stands still.
        let (mut velocity, mut fov_velocity) = (Vec3::ZERO, 0.0);
        if let Some(next_frame) = frame.checked_add(1).filter(|next| *next < self.frames()) {
            let next = self.composed_pose(next_frame)?;
            velocity = sub(next.position, now.position);
            fov_velocity = next.fov_deg - now.fov_deg;
        }
        Ok(state_of(now, velocity, fov_velocity))
    }
}

#[cfg(test)]
mod tests {
    use core::prelude::v1::test;

    use super::*;

    fn v(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 { x, y, z }
    }

    fn hold(frames: u64, from: SegmentStart) -> CameraSegment {
        CameraSegment {
            duration_frames: frames,
            from,
            motion: CameraMotion::Hold,
        }
    }

    fn dolly(frames: u64, distance: f64) -> CameraSegment {
        CameraSegment {
            duration_frames: frames,
            from: SegmentStart::PreviousEnd,
            motion: CameraMotion::Move {
                recipe: MoveRecipe::DollyIn,
                distance_scene_units: distance,
                easing: "linear".to_string(),
            },
        }
    }

    fn clip(segments: Vec<CameraSegment>, layers: Vec<Vec<CameraSegment>>) -> CameraClip {
        CameraClip {
            id: "c".to_string(),
            initial_position: v(0.0, 0.0, 10.0),
            initial_target: v(0.0, 0.0, 0.0),
            initial_fov_deg: 50.0,
            segments,
            layers,
        }
    }

    #[test]
    fn hypot3_matches_the_euclidean_length_and_survives_extremes() {
        assert!((hypot3(v(3.0, 4.0, 12.0)) - 13.0).abs() < 1e-12);
        assert_eq!(hypot3(v(0.0, 0.0, 0.0)), 0.0);
        assert_eq!(hypot3(v(f64::INFINITY, f64::NAN, 1.0)), f64::INFINITY);
        assert!(hypot3(v(f64::NAN, 1.0, 1.0)).is_nan());
        assert!(hypot3(v(1e-200, 1e-200, 1e-200)) > 0.0, "no underflow");
        assert!(hypot3(v(1e200, 1e200, 1e200)).is_finite(), "no overflow");
    }

    #[test]
    fn easings_are_clamped_and_unknown_names_are_typed_errors() {
        assert_eq!(apply_easing(-1.0, "linear"), Ok(0.0));
        assert_eq!(apply_easing(2.0, "easeInOutCubic"), Ok(1.0));
        assert_eq!(apply_easing(0.5, "easeInOutCubic"), Ok(0.5));
        assert_eq!(apply_easing(0.5, "easeOutCubic"), Ok(0.875));
        assert_eq!(
            apply_easing(0.5, "bounce"),
            Err(TemporalError::UnknownEasing {
                easing: "bounce".to_string()
            })
        );
    }

    #[test]
    fn a_clip_is_evaluated_lazily_up_to_the_movement_that_cannot_be_drawn() {
        // The importer refuses a camera on its own target; evaluating a hand-built one shows that
        // a hold (which never asks for a direction) still draws, and the first dolly does not.
        let mut broken = clip(
            vec![hold(3, SegmentStart::PreviousEnd), dolly(5, 1.0)],
            vec![],
        );
        broken.initial_target = broken.initial_position;
        let timeline = CameraTimeline::new(&broken).unwrap();
        assert_eq!(
            timeline.state_at(1).unwrap().position,
            broken.initial_position
        );
        assert_eq!(timeline.state_at(3), Err(TemporalError::DegenerateCamera));
        // Past the end the camera would rest on a pose that could not be carried there.
        assert_eq!(timeline.state_at(99), Err(TemporalError::DegenerateCamera));
        assert_eq!(
            timeline.main_track_poses(),
            Err(TemporalError::DegenerateCamera)
        );
    }

    #[test]
    fn a_movement_with_an_unknown_easing_fails_only_inside_itself() {
        let mut odd = dolly(5, 1.0);
        if let CameraMotion::Move { easing, .. } = &mut odd.motion {
            *easing = "bounce".to_string();
        }
        let c = clip(vec![hold(3, SegmentStart::PreviousEnd), odd], vec![]);
        let timeline = CameraTimeline::new(&c).unwrap();
        assert!(timeline.state_at(2).is_ok());
        assert!(matches!(
            timeline.state_at(4),
            Err(TemporalError::UnknownEasing { .. })
        ));
        // The easing is not needed to know where the movement ends.
        assert!(timeline.state_at(50).is_ok());
    }

    #[test]
    fn a_layer_with_keyframes_fails_only_from_the_movement_that_holds_them() {
        let keyed = CameraSegment {
            duration_frames: 5,
            from: SegmentStart::PreviousEnd,
            motion: CameraMotion::Keyframes { keys: vec![] },
        };
        let layered = clip(
            vec![hold(20, SegmentStart::PreviousEnd)],
            vec![vec![hold(4, SegmentStart::PreviousEnd), keyed]],
        );
        let timeline = CameraTimeline::new(&layered).unwrap();
        assert!(timeline.state_at(2).is_ok());
        // Frame 3 is still the hold, but its speed looks one frame ahead, into the keyframes.
        assert_eq!(timeline.state_at(3), Err(TemporalError::LayerKeyframes));
        assert_eq!(timeline.state_at(19), Err(TemporalError::LayerKeyframes));
    }

    #[test]
    fn a_frame_before_the_clip_is_an_error_and_after_it_the_camera_rests() {
        let c = clip(vec![dolly(10, 5.0)], vec![]);
        let timeline = CameraTimeline::new(&c).unwrap();
        assert_eq!(
            timeline.state_at(-1),
            Err(TemporalError::CameraFrameBeforeStart {
                clip_id: "c".to_string(),
                frame: -1
            })
        );
        let rest = timeline.state_at(10).unwrap();
        assert_eq!(rest.position, v(0.0, 0.0, 5.0));
        assert_eq!(
            (rest.velocity, rest.fov_velocity_deg_per_frame),
            (Vec3::ZERO, 0.0)
        );
        // The last frame of a movement has no next frame inside it: speed 0 (origin behaviour).
        assert_eq!(timeline.state_at(9).unwrap().velocity, Vec3::ZERO);
        assert!((timeline.state_at(8).unwrap().velocity.z + 5.0 / 9.0).abs() < 1e-12);
        assert_eq!(timeline.frames(), 10);
    }

    #[test]
    fn world_matrix_follows_the_threejs_look_at_convention() {
        let pose = CameraPose {
            position: v(0.0, 0.0, 5.0),
            target: v(0.0, 0.0, 0.0),
            fov_deg: 50.0,
        };
        assert_eq!(
            pose.to_world_matrix().0,
            [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 5., 1.
            ]
        );
        // Looking down the -Z axis of the camera: the target sits at local (0, 0, -distance).
        let slanted = CameraPose {
            position: v(3.0, 4.0, 5.0),
            target: v(-1.0, 2.0, 0.5),
            fov_deg: 40.0,
        };
        let matrix = slanted.to_world_matrix();
        let local_target = {
            let m = &matrix.0;
            let d = sub(slanted.target, slanted.position);
            // Inverse of a rotation: transpose.
            v(
                m[0] * d.x + m[1] * d.y + m[2] * d.z,
                m[4] * d.x + m[5] * d.y + m[6] * d.z,
                m[8] * d.x + m[9] * d.y + m[10] * d.z,
            )
        };
        let distance = distance(slanted.target, slanted.position);
        assert!(local_target.x.abs() < 1e-12 && local_target.y.abs() < 1e-12);
        assert!((local_target.z + distance).abs() < 1e-12);
        // Straight down does not collapse: the cross product is nudged like Three.js does.
        let down = CameraPose {
            position: v(0.0, 5.0, 0.0),
            target: v(0.0, 0.0, 0.0),
            fov_deg: 50.0,
        };
        assert!(down.to_world_matrix().is_finite());
    }
}
