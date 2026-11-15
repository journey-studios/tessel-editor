// The clocks, the window and the exact times of a transition plan (F064, P4b S2-R0): a port of
// `crates/core/src/scene_plan/temporal/validate.rs` over the JS wire, comparing and never deciding.
//
// The plan DECLARES where the two shots sit in the edit and where each deck's two clocks land; this
// file recomputes them with the core's one rule and refuses a plan that says otherwise. It judges
// coherence, not where the plan came from: a wholly consistent forged plan still describes another
// scene. Nothing here moves a pose or a pixel.
//
// Arithmetic is BigInt, so a sum that leaves the safe range is exact, not rounded; a clock whose raw
// value cannot be a JS safe integer is refused (the subset of the Rust i64 this runtime reads), never
// rounded. Every refusal is `invalid_plan` with the pointer of the field that is wrong.

import { invalid } from './directed-deck.js';
import { OTHER } from './directed-transition-decks.js';

const SIDES = ['from', 'to'];
const MAX_SAFE = BigInt(Number.MAX_SAFE_INTEGER);

const at = (side, key) => `/transition/timing/${side}/${key}`;

const exact = (value, pointer) => {
  if (value > MAX_SAFE || value < -MAX_SAFE) {
    throw invalid(pointer, 'is beyond the 2^53 - 1 this runtime reads exactly');
  }
  return Number(value);
};

/**
 * One clock held to its own length: `raw` as it runs, `sampled = clamp(raw, 0, length - 1)` (a clip with
 * no frames holds frame 0) and `held` when the clip has no real frame there. The last real frame is not
 * held. Mirrors `ClockSample::of`, with the same independence of the instance and the camera.
 */
export const clockSample = (raw, length, pointer) => {
  const last = length > 0n ? length - 1n : 0n;
  const sampled = raw < 0n ? 0n : raw > last ? last : raw;
  return { raw: exact(raw, pointer), sampled: exact(sampled, pointer), held: length === 0n || sampled !== raw };
};

/** The two clocks of the shot of `side` at `editFrame` (`DeckClocks::at`), each on its own length. */
const deckClocks = (editFrame, deck, side) => {
  const delta = editFrame - BigInt(deck.shotStartFrame);
  return {
    instance: clockSample(BigInt(deck.instanceStartFrame) + delta, BigInt(deck.instanceDurationFrames), at(side, 'instanceStartFrame')),
    camera: clockSample(BigInt(deck.cameraInFrame) + delta, BigInt(deck.cameraClipDurationFrames), at(side, 'cameraInFrame')),
  };
};

const gcd = (a, b) => {
  let [x, y] = [a < 0n ? -a : a, b < 0n ? -b : b];
  while (y !== 0n) [x, y] = [y, x % y];
  return x;
};

/**
 * The exact time of a frame counter at `fps`: the reduced ratio `counter x den / num` (`frame_start_time`),
 * as BigInts. `null` when the rate is not one the core accepts (`Rate::new`: positive, at most 1000 fps).
 */
const frameTime = (counter, fps) => {
  const [num, den] = [BigInt(fps.numerator), BigInt(fps.denominator)];
  const divisor = gcd(num, den);
  const [rateNum, rateDen] = [num / divisor, den / divisor];
  const scaled = BigInt(counter) * rateDen;
  const common = gcd(scaled, rateNum);
  return { numerator: scaled / common, denominator: rateNum / common };
};

const isRate = (fps) => BigInt(fps.numerator) <= 1000n * BigInt(fps.denominator);

const sameTime = (ratio, time) => BigInt(ratio.numerator) === time.numerator && BigInt(ratio.denominator) === time.denominator;

/** Non-empty shots inside the edit, one cut, the right primary, and a frame inside it. */
const checkShape = (fields, primary) => {
  const { timing } = fields;
  for (const side of SIDES) {
    const deck = timing[side];
    const end = at(side, 'shotEndFrameExclusive');
    if (deck.shotStartFrame >= deck.shotEndFrameExclusive) throw invalid(end, 'a shot has at least one frame: its end is after its start');
    if (deck.shotEndFrameExclusive > timing.editDurationFrames) throw invalid(end, 'a shot ends inside the edit (editDurationFrames)');
  }
  const cut = timing.to.shotStartFrame;
  if (timing.from.shotEndFrameExclusive !== cut) {
    throw invalid(at('from', 'shotEndFrameExclusive'), 'the outgoing shot ends on the cut, the first frame of the incoming one');
  }
  const frame = primary.frame.editFrame;
  if (frame >= timing.editDurationFrames) throw invalid('/frame/editFrame', 'the frame is inside the edit');
  if (fields.primary !== (frame < cut ? 'from' : 'to')) {
    throw invalid('/transition/primary', 'the primary deck is `from` before the cut and `to` from it');
  }
  const deck = timing[fields.primary];
  if (frame < deck.shotStartFrame) throw invalid(at(fields.primary, 'shotStartFrame'), 'the frame is inside the primary shot: it starts at or before it');
  if (frame >= deck.shotEndFrameExclusive) throw invalid(at(fields.primary, 'shotEndFrameExclusive'), 'the frame is inside the primary shot: it ends after it');
};

/** The window of the cut, the alignment and the duration is the one the plan says (`transition_window`). */
const checkWindow = (fields) => {
  const { timing, durationFrames: duration, alignment } = fields;
  if (duration < 1 || duration > timing.editDurationFrames) throw invalid('/transition/durationFrames', 'must be from 1 to the length of the edit');
  const [cut, length, edit] = [BigInt(timing.to.shotStartFrame), BigInt(duration), BigInt(timing.editDurationFrames)];
  const start = alignment === 'start' ? cut : alignment === 'center' ? cut - length / 2n : cut - length;
  const end = start + length;
  if (start < 0n || end > edit) throw invalid('/transition/startFrame', 'the window of this cut, alignment and duration is not inside the edit');
  if (start !== BigInt(fields.startFrame)) {
    throw invalid('/transition/startFrame', 'must be where the window of the cut, the alignment and the duration starts');
  }
  return { cut: Number(cut), end: Number(end) };
};

/** One instance or camera clip seen from both decks has one length; each instance covers its whole shot. */
const checkIdentities = (fields, decks) => {
  const { timing } = fields;
  const [from, to] = [decks.from, decks.to];
  if (from.source.sceneInstanceId === to.source.sceneInstanceId && timing.from.instanceDurationFrames !== timing.to.instanceDurationFrames) {
    throw invalid(at('to', 'instanceDurationFrames'), 'an instance seen from both decks has one length');
  }
  if (from.frame.cameraClipId === to.frame.cameraClipId && timing.from.cameraClipDurationFrames !== timing.to.cameraClipDurationFrames) {
    throw invalid(at('to', 'cameraClipDurationFrames'), 'a camera clip seen from both decks has one length');
  }
  for (const side of SIDES) {
    const deck = timing[side];
    const frames = BigInt(deck.shotEndFrameExclusive) - BigInt(deck.shotStartFrame);
    if (BigInt(deck.instanceStartFrame) + frames > BigInt(deck.instanceDurationFrames)) {
      throw invalid(at(side, 'instanceDurationFrames'), "the instance holds all of its shot's frames");
    }
  }
};

/**
 * The declared samples are the rule's; the primary keeps the RAW clocks in its legacy frame fields, the
 * neighbour the samples, and the neighbour's held flags are the rule's own (also when `false`).
 */
const checkClocks = (fields, primary, other) => {
  const { timing } = fields;
  const editFrame = BigInt(primary.frame.editFrame);
  const found = {};
  for (const side of SIDES) {
    found[side] = deckClocks(editFrame, timing[side], side);
    if (timing[side].sampledInstanceFrame !== found[side].instance.sampled) {
      throw invalid(at(side, 'sampledInstanceFrame'), 'must be the instance clock held to the instance\'s frames');
    }
    if (timing[side].sampledCameraClipFrame !== found[side].camera.sampled) {
      throw invalid(at(side, 'sampledCameraClipFrame'), 'must be the camera clock held to the camera clip\'s frames');
    }
  }
  const [main, neighbour] = [found[fields.primary], found[fields.primary === 'from' ? 'to' : 'from']];
  if (main.instance.raw !== primary.frame.instanceFrame) throw invalid('/frame/instanceFrame', 'the primary deck keeps the raw instance frame of its shot');
  if (main.camera.raw !== primary.frame.cameraClipFrame) throw invalid('/frame/cameraClipFrame', 'the primary deck keeps the raw camera clip frame of its shot');
  if (other.frame.instanceFrame !== neighbour.instance.sampled) {
    throw invalid(`${OTHER}/frame/instanceFrame`, 'the neighbour deck sits on its held instance frame');
  }
  if (other.frame.cameraClipFrame !== neighbour.camera.sampled) {
    throw invalid(`${OTHER}/frame/cameraClipFrame`, 'the neighbour deck sits on its held camera clip frame');
  }
  if (fields.heldInstance !== neighbour.instance.held) {
    throw invalid('/transition/heldInstance', "true exactly when the neighbour's instance clock was held");
  }
  if (fields.heldCamera !== neighbour.camera.held) {
    throw invalid('/transition/heldCamera', "true exactly when the neighbour's camera clock was held");
  }
  return { primary: main, other: neighbour };
};

/** Every time of both decks is the exact time of its own frame counter at the plan's rate. */
const checkTimes = (primary, other) => {
  const { fps } = primary.output;
  if (!isRate(fps)) throw invalid('/output/fps', 'frame rate is not representable');
  for (const [deck, prefix] of [[primary, ''], [other, OTHER]]) {
    const { frame } = deck;
    for (const [counter, time, key] of [
      [frame.editFrame, frame.editTime, 'editTime'],
      [frame.instanceFrame, frame.instanceTime, 'instanceTime'],
      [frame.cameraClipFrame, frame.cameraClipTime, 'cameraClipTime'],
    ]) {
      if (!sameTime(time, frameTime(counter, fps))) {
        throw invalid(`${prefix}/frame/${key}`, 'must be the exact time of its frame at the frame rate of the plan');
      }
    }
  }
};

/**
 * `check_transition_timing`: shape, window, identities, clocks and times, in that order. `fields` is the
 * transition read by `readTransitionWire`; `primary` and `other` are the identities of the decks
 * (`readDeckIdentity`). Answers the cut, the end of the window and the clocks the rule gives each deck.
 */
export const checkTransitionTiming = (fields, primary, other) => {
  checkShape(fields, primary);
  const window = checkWindow(fields);
  checkIdentities(fields, fields.primary === 'from' ? { from: primary, to: other } : { from: other, to: primary });
  const clocks = checkClocks(fields, primary, other);
  checkTimes(primary, other);
  return { cut: window.cut, windowEnd: window.end, clocks };
};
