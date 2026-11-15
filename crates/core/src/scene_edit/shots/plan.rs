//! The pure planner of the shot ops (F065, decisions #3, #6, #8..#11, #21, #28..#33).
//!
//! Typed shots and instances in, a [`ShotPlan`] or a [`Refusal`] out: no JSON, no I/O, no gate.
//! A plan names everything the apply step writes (new ranges, `instanceStartFrame` and
//! `cameraInFrame`, instances that grow, the edit length) and everything the layers of S2 need
//! (the edges that moved, the `content_shift` of each shot, the chain of splices).
//!
//! The order in which two rules that refuse the same op are reported is fixed by the reference
//! (`scripts/shot-trim-reference.py`, `REFUSAL_ORDER`): the option, the ids, the length of the
//! shot, a continuous join, the head frames, the length of the edit, the lock, then (S2) the
//! transitions, then the gate.

use std::collections::HashMap;

use super::error::{
    CONTINUOUS_JOIN, EDIT_TOO_LONG, LOCKED, NO_HEAD_FRAMES, Refusal, SHOT_NOT_FOUND,
    SHOT_TOO_SHORT, SHOTS_LAYER_NOT_YET, SHOTS_NOT_ADJACENT, SHOTS_OPTION_NOT_YET,
};
use super::{Content, Edge, FRAME_BOUND, MAX_EDIT_FRAMES, Mode, ShotOp, in_bound};
use crate::scene_edit::error::INVALID_OP;

/// Where a shot sits on the edit and on its scene and camera clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub start: i64,
    /// Exclusive.
    pub end: i64,
    pub instance_start: i64,
    pub camera_in: i64,
}

impl Placement {
    pub fn frames(&self) -> i64 {
        self.end - self.start
    }

    /// `instanceStartFrame - startFrame`: how far the scene clock runs from the edit clock. Shots
    /// of one `editLocked` instance that share it are "in step" (#10).
    fn step(&self) -> i64 {
        self.instance_start - self.start
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanShot {
    pub id: String,
    pub instance: String,
    pub at: Placement,
    /// The join into this shot is `continuous`: it carries on the previous shot.
    pub continuous_in: bool,
    /// Head frames the transition into this shot draws before the cut (the reserve of `auto`,
    /// #29): 0 when it has none or its `handles` is `hold`.
    pub reserve: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanInstance {
    pub id: String,
    pub frames: i64,
    pub locked: bool,
}

/// What the planner reads: the shots **ordered by start** (a document may list them in any
/// order), the instances in document order and the edit length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanInput {
    pub edit_frames: i64,
    pub shots: Vec<PlanShot>,
    pub instances: Vec<PlanInstance>,
    /// A field of the S0 audit that this phase refuses (`SHOTS_LAYER_NOT_YET`, D12) and that the
    /// document carries: the planner refuses right after it resolved the ids, as the reference
    /// does. `None` with the audit's list of today (it is empty).
    pub refused_layer: Option<String>,
}

impl PlanInput {
    pub fn new(edit_frames: i64, mut shots: Vec<PlanShot>, instances: Vec<PlanInstance>) -> Self {
        shots.sort_by_key(|shot| shot.at.start);
        Self {
            edit_frames,
            shots,
            instances,
            refused_layer: None,
        }
    }

    fn index_of(&self, id: &str) -> Result<usize, Refusal> {
        self.shots
            .iter()
            .position(|shot| shot.id == id)
            .ok_or_else(|| {
                Refusal::new(SHOT_NOT_FOUND, format!("the edit has no shot `{id}`")).shot(id)
            })
    }

    /// The shots that carry on the one at `index` through continuous joins, as indexes.
    fn continuation(&self, index: usize) -> std::ops::Range<usize> {
        let mut last = index + 1;
        while self.shots.get(last).is_some_and(|shot| shot.continuous_in) {
            last += 1;
        }
        index + 1..last
    }

    /// The old end of the last shot of the take that starts at `index` (#8): the splice of a
    /// ripple sits here, not at the end of the shot that was edited.
    fn take_end(&self, index: usize) -> i64 {
        let carried = self.continuation(index);
        self.shots[carried.end - 1].at.end
    }
}

/// A change of the edit's time line that every layer in edit frames follows (#31).
///
/// `Insert`: `frames` frames enter at `at`; points at or after `at` move by `frames`. `Remove`:
/// the stretch `[from, to)` leaves; points inside it are dropped and points at or after `to` move
/// back by `to - from`. A chain applies them in order, each in the coordinates the previous left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Splice {
    Insert { at: i64, frames: i64 },
    Remove { from: i64, to: i64 },
}

/// A border an op moved (it limits `TRANSITION_TOO_LONG`, #33, in S2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedEdge {
    pub shot: String,
    pub edge: Edge,
}

/// An instance that had to grow so the shot fits (#6): never the reverse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Growth {
    pub instance: String,
    pub from: i64,
    pub to: i64,
}

/// `auto` anchored what it could and followed the rest (#28): the op says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowFallback {
    pub shot: String,
    pub anchored: i64,
    pub followed: i64,
}

/// What the op named, in the words of the `details` of a refusal (`shot`, `edge`, `asked_frame`):
/// the apply step needs it to refuse in the op's name after the planner is done.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlanTarget {
    /// The shot the op named; `None` for `move_cut`, which names a cut.
    pub shot: Option<String>,
    pub edge: Option<&'static str>,
    /// The frame the op asked for, in its own clock (the end of the shot for a duration).
    pub asked_frame: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotPlan {
    /// The target of the op that made the plan.
    pub target: PlanTarget,
    /// Every shot with its new placement, ordered by old start.
    pub shots: Vec<(String, Placement)>,
    pub edit_frames: i64,
    pub splices: Vec<Splice>,
    pub edges: Vec<MovedEdge>,
    pub growth: Vec<Growth>,
    /// `Δ(startFrame - instanceStartFrame)` of each shot whose picture moved on the edit: the
    /// frames the clips of that shot slide by (D09, S2). Shots that did not move are absent.
    pub content_shift: Vec<(String, i64)>,
    pub follow: Option<FollowFallback>,
    /// The shots the op named or pulled along; the transitions at their cuts are judged (S2).
    pub touched: Vec<String>,
}

impl ShotPlan {
    /// One lookup, linear. A caller that looks up every shot builds [`Self::placements_by_id`].
    pub fn placement(&self, id: &str) -> Option<&Placement> {
        self.shots
            .iter()
            .find(|(shot, _)| shot == id)
            .map(|(_, at)| at)
    }

    /// Every shot's placement by id, for a caller that looks up all of them.
    pub fn placements_by_id(&self) -> HashMap<&str, &Placement> {
        self.shots
            .iter()
            .map(|(id, at)| (id.as_str(), at))
            .collect()
    }
}

/// The frames the planner will add up, checked against [`FRAME_BOUND`] before any arithmetic: the
/// op's own frame (a typed op does not pass through the JSON parser, so the bound is asked
/// again) and the document the typed input describes. A frame outside is `INVALID_OP`, never a
/// wrap-around (`i64::MIN` has no absolute value).
fn check_bounds(input: &PlanInput, op: &ShotOp) -> Result<(), Refusal> {
    let (asked, name) = match op {
        ShotOp::TrimShot { frame, .. } | ShotOp::MoveCut { frame, .. } => (*frame, "frame"),
        ShotOp::SetShotDuration { frames, .. } | ShotOp::SetEditDuration { frames, .. } => {
            (*frames, "frames")
        }
    };
    if !in_bound(asked) {
        let mut refusal = Refusal::new(
            INVALID_OP,
            format!("`{name}` is beyond +-{FRAME_BOUND}: {asked}"),
        );
        refusal.asked_frame = Some(asked);
        return Err(refusal);
    }
    let document_is_in_bound = in_bound(input.edit_frames)
        && input
            .instances
            .iter()
            .all(|instance| in_bound(instance.frames))
        && input.shots.iter().all(|shot| {
            [
                shot.at.start,
                shot.at.end,
                shot.at.instance_start,
                shot.at.camera_in,
                shot.reserve,
            ]
            .into_iter()
            .all(in_bound)
        });
    if !document_is_in_bound {
        return Err(Refusal::new(
            INVALID_OP,
            format!("a frame of the edit is beyond +-{FRAME_BOUND}"),
        ));
    }
    Ok(())
}

/// The index of the shot the op works on, in the order of the reference's refusals: the ids, then
/// the adjacency of a cut. For `move_cut` it is the shot **before** the cut.
fn resolve(input: &PlanInput, op: &ShotOp) -> Result<usize, Refusal> {
    match op {
        ShotOp::SetEditDuration { .. } => input
            .shots
            .len()
            .checked_sub(1)
            .ok_or_else(|| Refusal::new(SHOT_NOT_FOUND, "the edit has no shots")),
        ShotOp::MoveCut { from, to, .. } => {
            let previous = input.index_of(from)?;
            let following = input.index_of(to)?;
            if previous + 1 != following {
                return Err(Refusal::new(
                    SHOTS_NOT_ADJACENT,
                    format!("`{from}` and `{to}` are not neighbours in this order"),
                )
                .with("from", from.as_str().into())
                .with("to", to.as_str().into()));
            }
            Ok(previous)
        }
        ShotOp::SetShotDuration { shot, .. } | ShotOp::TrimShot { shot, .. } => {
            input.index_of(shot)
        }
    }
}

/// Plans one op over the shots as they are now. `Ok(None)` is a no-op (the op writes what is
/// there). A refusal carries the shot, the edge and the frame the op asked for.
///
/// Order of the refusals (the reference's `apply_op`): a frame out of bounds, the option, the ids,
/// the refused fields of the audit, then the rules of each planner.
pub fn plan_op(input: &PlanInput, op: &ShotOp) -> Result<Option<ShotPlan>, Refusal> {
    check_bounds(input, op)?;
    let options = op.options();
    if options.with_narration {
        return Err(Refusal::new(
            SHOTS_OPTION_NOT_YET,
            "`with_narration` is not supported yet: narration never moves in this phase",
        )
        .with("option", "with_narration".into()));
    }
    let index = resolve(input, op)?;
    if let Some(field) = &input.refused_layer {
        return Err(Refusal::new(
            SHOTS_LAYER_NOT_YET,
            format!("the document has `{field}`, a layer in edit time this phase cannot move"),
        )
        .with("field", field.as_str().into()));
    }
    let content = options.content;
    let roll = options.mode == Mode::Roll;
    let (plan, target) = match op {
        ShotOp::SetEditDuration { frames, .. } => {
            let id = input.shots[index].id.clone();
            let plan = end_ripple(input, index, *frames)
                .map_err(|refusal| refusal.in_op(Some(&id), Some("end"), *frames))?;
            (plan, (Some(id), Some("end"), *frames))
        }
        ShotOp::MoveCut {
            from, to, frame, ..
        } => {
            // A cut is the border of two shots: no single `edge` names it.
            let plan = roll_cut(input, index, *frame, content)
                .map_err(|refusal| refusal.in_op(None, None, *frame).cut(from, to))?;
            (plan, (None, None, *frame))
        }
        ShotOp::SetShotDuration { shot, frames, .. } => {
            let frame = input.shots[index].at.start + frames;
            let plan = edge_op(input, index, Edge::End, frame, roll, content)?;
            (plan, (Some(shot.clone()), Some("end"), frame))
        }
        ShotOp::TrimShot {
            shot, edge, frame, ..
        } => {
            let plan = edge_op(input, index, *edge, *frame, roll, content)?;
            (plan, (Some(shot.clone()), Some(edge.name()), *frame))
        }
    };
    Ok(plan.map(|mut plan| {
        plan.target = PlanTarget {
            shot: target.0,
            edge: target.1,
            asked_frame: target.2,
        };
        plan
    }))
}

/// `trim_shot` and `set_shot_duration`: a border of one shot, in ripple or in roll.
fn edge_op(
    input: &PlanInput,
    index: usize,
    edge: Edge,
    frame: i64,
    roll: bool,
    content: Content,
) -> Result<Option<ShotPlan>, Refusal> {
    let id = input.shots[index].id.clone();
    let name = Some(edge.name());
    let result = match (edge, roll) {
        (Edge::End, true) => {
            if index + 1 >= input.shots.len() {
                Err(no_neighbour(&id, "no shot after it to roll with"))
            } else {
                roll_cut(input, index, frame, content)
            }
        }
        (Edge::Start, true) => {
            if index == 0 {
                Err(no_neighbour(&id, "no shot before it to roll with"))
            } else {
                roll_cut(input, index - 1, frame, content)
            }
        }
        (Edge::End, false) => end_ripple(input, index, frame),
        (Edge::Start, false) => start_ripple(input, index, frame, content),
    };
    result.map_err(|refusal| refusal.in_op(Some(&id), name, frame))
}

fn no_neighbour(id: &str, reason: &str) -> Refusal {
    Refusal::new(SHOTS_NOT_ADJACENT, format!("shot `{id}` has {reason}"))
        .shot(id)
        .with("reason", reason.into())
}

/// The state of every shot while a plan is being made.
fn placements(input: &PlanInput) -> Vec<Placement> {
    input.shots.iter().map(|shot| shot.at).collect()
}

fn shift_later(new: &mut [Placement], index: usize, delta: i64) {
    for later in &mut new[index + 1..] {
        later.start += delta;
        later.end += delta;
    }
}

/// The shots that carry on `index` start `frames` later in the scene and in the camera clip
/// (#8, #30): the edit ranges do not change, the picture they show slides.
fn slide_continuation(input: &PlanInput, new: &mut [Placement], index: usize, frames: i64) {
    for carried in input.continuation(index) {
        new[carried].instance_start += frames;
        new[carried].camera_in += frames;
    }
}

/// How many of the `d` frames the entering side gains `auto` anchors (#29, `AUTO_RESERVE`): the
/// head frames that exist, less the reserve of the transition into the shot, at most `d`.
fn anchored_frames(d: i64, head: i64, reserve: i64) -> i64 {
    d.min(head - reserve).max(0)
}

/// The values of `a`, the frames anchored of the `d` the entering side gains, in the order they
/// are tried (#28..#30). `anchor` anchors all `d` or refuses with `NO_HEAD_FRAMES`; `follow`
/// anchors none, and is `CONTINUOUS_JOIN` into a continuous join; `auto` anchors what it can,
/// never refuses for lack of head and never follows into a continuous join.
fn anchor_candidates(
    content: Content,
    d: i64,
    head: i64,
    reserve: i64,
    continuous_in: bool,
    no_head_limit: i64,
) -> Result<Vec<i64>, Refusal> {
    let no_head = || {
        Refusal::new(
            NO_HEAD_FRAMES,
            format!(
                "the entering shot has {head} head frames of scene or camera and needs {d}; its \
                 start can go no earlier than edit frame {no_head_limit}"
            ),
        )
        .limit(no_head_limit)
    };
    match content {
        Content::Anchor => {
            if head < d {
                return Err(no_head());
            }
            Ok(vec![d])
        }
        Content::Follow => {
            if continuous_in {
                return Err(follow_into_continuous());
            }
            Ok(vec![0])
        }
        Content::Auto => {
            if continuous_in {
                if head < d {
                    return Err(no_head());
                }
                return Ok(vec![d]);
            }
            let first = anchored_frames(d, head, reserve);
            let mut tries = vec![first];
            if first < d && head >= d {
                tries.push(d);
            }
            if !tries.contains(&0) {
                tries.push(0);
            }
            Ok(tries)
        }
    }
}

fn follow_into_continuous() -> Refusal {
    Refusal::new(
        CONTINUOUS_JOIN,
        "`follow` would make the picture jump at a continuous join; use `anchor` or `auto`",
    )
    .with("reason", "follow into a continuous join".into())
}

/// `editLocked` (#10): shots of one locked instance that were in step (the same
/// `instanceStartFrame - startFrame`) must still be after the op.
///
/// Linear in the shots: the members of an instance are grouped by their old step, and a group is
/// broken when its members no longer share one new step. The pair reported is the first one in
/// order of the reference's double loop (the first member, and the first later member that
/// differs from it); across groups the earliest pair wins.
fn lock_problem(input: &PlanInput, new: &[Placement]) -> Option<Refusal> {
    let mut members: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, shot) in input.shots.iter().enumerate() {
        members
            .entry(shot.instance.as_str())
            .or_default()
            .push(index);
    }
    for instance in input.instances.iter().filter(|instance| instance.locked) {
        let Some(members) = members.get(instance.id.as_str()) else {
            continue;
        };
        let mut groups: HashMap<i64, Vec<usize>> = HashMap::new();
        for (position, &shot) in members.iter().enumerate() {
            groups
                .entry(input.shots[shot].at.step())
                .or_default()
                .push(position);
        }
        let broken = groups
            .values()
            .filter_map(|group| {
                let first = group[0];
                let step = new[members[first]].step();
                let other = group[1..]
                    .iter()
                    .find(|&&position| new[members[position]].step() != step)?;
                Some((first, *other))
            })
            .min();
        if let Some((x, y)) = broken {
            let (a, b) = (&input.shots[members[x]], &input.shots[members[y]]);
            return Some(
                Refusal::new(
                    LOCKED,
                    format!(
                        "shots `{}` and `{}` of the locked instance `{}` would leave step",
                        a.id, b.id, instance.id
                    ),
                )
                .with("instance", instance.id.as_str().into())
                .with("shots", vec![a.id.as_str(), b.id.as_str()].into()),
            );
        }
    }
    None
}

/// Lock check, instance growth and `content_shift`: what every plan needs before it is a plan.
///
/// S2 judges the transitions here (`TRANSITION_TOO_LONG`, `shrink_transitions`); in S1 a
/// transition that no longer fits is the gate's finding.
fn finish(
    input: &PlanInput,
    new: Vec<Placement>,
    edit_frames: i64,
    splices: Vec<Splice>,
    edges: Vec<MovedEdge>,
    touched: Vec<String>,
    follow: Option<FollowFallback>,
) -> Result<ShotPlan, Refusal> {
    if let Some(refusal) = lock_problem(input, &new) {
        return Err(refusal);
    }
    let mut needs: HashMap<&str, i64> = HashMap::new();
    for (shot, at) in input.shots.iter().zip(&new) {
        let need = needs.entry(shot.instance.as_str()).or_insert(i64::MIN);
        *need = (*need).max(at.instance_start + at.frames());
    }
    let growth = input
        .instances
        .iter()
        .filter_map(|instance| {
            let needed = *needs.get(instance.id.as_str())?;
            (needed > instance.frames).then(|| Growth {
                instance: instance.id.clone(),
                from: instance.frames,
                to: needed,
            })
        })
        .collect();
    let content_shift = input
        .shots
        .iter()
        .zip(&new)
        .filter_map(|(shot, at)| {
            let shift = (at.start - at.instance_start) - (shot.at.start - shot.at.instance_start);
            (shift != 0).then(|| (shot.id.clone(), shift))
        })
        .collect();
    Ok(ShotPlan {
        // `plan_op` fills the target: the rules below do not know which op they serve.
        target: PlanTarget::default(),
        shots: input
            .shots
            .iter()
            .zip(new)
            .map(|(shot, at)| (shot.id.clone(), at))
            .collect(),
        edit_frames,
        splices,
        edges,
        growth,
        content_shift,
        follow,
        touched,
    })
}

fn follow_note(
    shot: &PlanShot,
    anchored: i64,
    followed: i64,
    content: Content,
) -> Option<FollowFallback> {
    // An explicit `follow` asked for it: no note (#28).
    (followed > 0 && content == Content::Auto).then(|| FollowFallback {
        shot: shot.id.clone(),
        anchored,
        followed,
    })
}

/// The end of the shot goes to `frame` (exclusive) and everything after moves with it (#3). Also
/// `set_edit_duration`, which is this op on the last shot (#21).
fn end_ripple(input: &PlanInput, index: usize, frame: i64) -> Result<Option<ShotPlan>, Refusal> {
    let shot = &input.shots[index];
    let (start, end) = (shot.at.start, shot.at.end);
    if frame == end {
        return Ok(None);
    }
    if frame - start < 1 {
        return Err(Refusal::new(
            SHOT_TOO_SHORT,
            format!(
                "shot `{}` would have {} frames; its end can be no earlier than frame {}",
                shot.id,
                frame - start,
                start + 1
            ),
        )
        .limit(start + 1));
    }
    let delta = frame - end;
    let edit = input.edit_frames + delta;
    if edit > MAX_EDIT_FRAMES {
        return Err(too_long(end + (MAX_EDIT_FRAMES - input.edit_frames)));
    }
    let mut new = placements(input);
    new[index].end = frame;
    shift_later(&mut new, index, delta);
    slide_continuation(input, &mut new, index, delta);
    let chain_end = input.take_end(index);
    let splice = if delta > 0 {
        Splice::Insert {
            at: chain_end,
            frames: delta,
        }
    } else {
        Splice::Remove {
            from: chain_end + delta,
            to: chain_end,
        }
    };
    finish(
        input,
        new,
        edit,
        vec![splice],
        vec![MovedEdge {
            shot: shot.id.clone(),
            edge: Edge::End,
        }],
        vec![shot.id.clone()],
        None,
    )
    .map(Some)
}

fn too_long(limit: i64) -> Refusal {
    Refusal::new(
        EDIT_TOO_LONG,
        format!("the edit can have at most {MAX_EDIT_FRAMES} frames"),
    )
    .with("max_frames", MAX_EDIT_FRAMES.into())
    .limit(limit)
}

/// The start of the shot goes to `frame`: the shot stays where it is on the edit, the picture of
/// its head leaves (or enters) and everything after moves (#11, #32).
fn start_ripple(
    input: &PlanInput,
    index: usize,
    frame: i64,
    content: Content,
) -> Result<Option<ShotPlan>, Refusal> {
    let shot = &input.shots[index];
    let (start, end) = (shot.at.start, shot.at.end);
    let d = start - frame;
    if d == 0 {
        return Ok(None);
    }
    if d < 0 && end - frame < 1 {
        return Err(Refusal::new(
            SHOT_TOO_SHORT,
            format!(
                "shot `{}` would have no frames; its start can be no later than frame {}",
                shot.id,
                end - 1
            ),
        )
        .limit(end - 1));
    }
    if shot.continuous_in {
        return Err(Refusal::new(
            CONTINUOUS_JOIN,
            format!(
                "shot `{}` carries on the previous one: its head cannot change without a jump",
                shot.id
            ),
        )
        .with(
            "reason",
            "head of a shot that carries on the previous one".into(),
        ));
    }
    let edges = vec![MovedEdge {
        shot: shot.id.clone(),
        edge: Edge::Start,
    }];
    let touched = vec![shot.id.clone()];
    if d < 0 {
        // The head is cut: the shot stays, its scene and camera advance, the rest comes back.
        let cut = -d;
        let mut new = placements(input);
        new[index].end = end - cut;
        new[index].instance_start += cut;
        new[index].camera_in += cut;
        shift_later(&mut new, index, -cut);
        return finish(
            input,
            new,
            input.edit_frames - cut,
            vec![Splice::Remove {
                from: start,
                to: start + cut,
            }],
            edges,
            touched,
            None,
        )
        .map(Some);
    }
    let head = shot.at.instance_start.min(shot.at.camera_in);
    let tries = anchor_candidates(content, d, head, shot.reserve, false, start - head)?;
    let edit = input.edit_frames + d;
    if edit > MAX_EDIT_FRAMES {
        return Err(too_long(start - (MAX_EDIT_FRAMES - input.edit_frames)));
    }
    let chain_end = input.take_end(index);
    let mut last = None;
    for anchored in tries {
        let followed = d - anchored;
        let mut new = placements(input);
        new[index].end = end + d;
        new[index].instance_start -= anchored;
        new[index].camera_in -= anchored;
        shift_later(&mut new, index, d);
        if followed > 0 {
            slide_continuation(input, &mut new, index, followed);
        }
        let mut splices = Vec::new();
        if anchored > 0 {
            splices.push(Splice::Insert {
                at: start,
                frames: anchored,
            });
        }
        if followed > 0 {
            splices.push(Splice::Insert {
                at: chain_end + anchored,
                frames: followed,
            });
        }
        let outcome = finish(
            input,
            new,
            edit,
            splices,
            edges.clone(),
            touched.clone(),
            follow_note(shot, anchored, followed, content),
        );
        match outcome {
            Err(refusal) if refusal.code == LOCKED && content == Content::Auto => {
                last = Some(refusal);
            }
            other => return other.map(Some),
        }
    }
    Err(last.expect("`anchor_candidates` yields at least one value"))
}

/// The cut between the shot at `previous` and the one after it goes to `frame`; the edit keeps
/// its length (#3, #28..#31).
fn roll_cut(
    input: &PlanInput,
    previous: usize,
    frame: i64,
    content: Content,
) -> Result<Option<ShotPlan>, Refusal> {
    let following = previous + 1;
    let (before, after) = (&input.shots[previous], &input.shots[following]);
    let cut = before.at.end;
    if frame == cut {
        return Ok(None);
    }
    let (p_start, q_end) = (before.at.start, after.at.end);
    if frame <= p_start {
        return Err(Refusal::new(
            SHOT_TOO_SHORT,
            format!(
                "shot `{}` would have no frames; the cut can be no earlier than frame {}",
                before.id,
                p_start + 1
            ),
        )
        .affected(&before.id)
        .limit(p_start + 1));
    }
    if frame >= q_end {
        return Err(Refusal::new(
            SHOT_TOO_SHORT,
            format!(
                "shot `{}` would have no frames; the cut can be no later than frame {}",
                after.id,
                q_end - 1
            ),
        )
        .affected(&after.id)
        .limit(q_end - 1));
    }
    let edges = vec![
        MovedEdge {
            shot: before.id.clone(),
            edge: Edge::End,
        },
        MovedEdge {
            shot: after.id.clone(),
            edge: Edge::Start,
        },
    ];
    let touched = vec![before.id.clone(), after.id.clone()];
    if frame > cut {
        // The cut goes later: the following shot loses head frames.
        let later = frame - cut;
        let mut new = placements(input);
        new[previous].end = frame;
        new[following].start = frame;
        new[following].instance_start += later;
        new[following].camera_in += later;
        return finish(
            input,
            new,
            input.edit_frames,
            Vec::new(),
            edges,
            touched,
            None,
        )
        .map(Some);
    }
    let d = cut - frame;
    let head = after.at.instance_start.min(after.at.camera_in);
    let tries = anchor_candidates(
        content,
        d,
        head,
        after.reserve,
        after.continuous_in,
        cut - head,
    )
    .map_err(|refusal| refusal.affected(&after.id))?;
    let chain_end = input.take_end(following);
    let mut last = None;
    for anchored in tries {
        let followed = d - anchored;
        let mut new = placements(input);
        new[previous].end = frame;
        new[following].start = frame;
        new[following].instance_start -= anchored;
        new[following].camera_in -= anchored;
        let mut splices = Vec::new();
        if followed > 0 {
            slide_continuation(input, &mut new, following, followed);
            splices.push(Splice::Remove {
                from: frame,
                to: cut,
            });
            if anchored > 0 {
                splices.push(Splice::Insert {
                    at: frame,
                    frames: anchored,
                });
            }
            splices.push(Splice::Insert {
                at: chain_end - followed,
                frames: followed,
            });
        }
        let outcome = finish(
            input,
            new,
            input.edit_frames,
            splices,
            edges.clone(),
            touched.clone(),
            follow_note(after, anchored, followed, content),
        );
        match outcome {
            Err(refusal) if refusal.code == LOCKED && content == Content::Auto => {
                last = Some(refusal);
            }
            other => return other.map(Some),
        }
    }
    Err(last.expect("`anchor_candidates` yields at least one value"))
}
