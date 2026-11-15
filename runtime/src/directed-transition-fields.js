// The fields of `transition` and of its `timing` (F064, P4b S2-R0): which keys and types the C0b wire
// has, and what the transition says about itself before it is related to the two decks.
//
// WHY every key is looked up by `Object.hasOwn`: a plan's names are data, never keys into the
// prototype chain (`constructor`, `toString` and `__proto__` are not fields, and a plan that has none
// of the required keys cannot borrow them). Wire first, then meaning, in the order of the core's
// reader (`transition_json.rs`) and validator (`transition_validate.rs`), each refusal naming the
// pointer of the field that is wrong. There is no default and no fallback: the wire before the C0b
// (no `alignment`, no `timing`) is refused at `/transition/alignment`.
//
// Numbers: every integer is a JS safe integer (`Number.isSafeInteger`), the subset of the Rust i64/u64
// this reader can represent. A fractional number, a bool, a string, `null`, `NaN`, an infinity and a number beyond
// 2^53 - 1 are refused in place, never rounded. JSON.parse erases integer token spellings (12/12.0/12e0). The checks are pure (no GL, no DOM).

import { finiteAt, invalid, objectAt, stringAt } from './directed-deck.js';

/** The one effect of the transition catalog. */
export const TRANSITION_EFFECT = 'crossDissolve';
const ALIGNMENTS = ['center', 'start', 'end'];
const SIDES = ['from', 'to'];
const EASINGS = ['linear', 'easeInOutCubic', 'easeOutCubic'];
/** `curve`, then `intensity`: the catalog order of `crossDissolve`. */
const CURVES = ['linear', 'smooth'];

const TRANSITION_KEYS = [
  'id', 'effect', 'fromShotId', 'toShotId', 'primary', 'startFrame', 'durationFrames', 'alignment', 'timing',
  'frameInWindow', 'easing', 'progress', 'amount', 'params', 'heldInstance', 'heldCamera', 'other',
];
const TIMING_KEYS = ['editDurationFrames', 'from', 'to'];
/** The numbers of one side of `timing`, in wire order. */
export const DECK_TIMING_KEYS = [
  'shotStartFrame', 'shotEndFrameExclusive', 'instanceStartFrame', 'instanceDurationFrames',
  'cameraInFrame', 'cameraClipDurationFrames', 'sampledInstanceFrame', 'sampledCameraClipFrame',
];

/** Tolerance of the checked `progress` and `amount` (plan §7.3), fixed. */
export const MIX_TOLERANCE = 1e-9;

const escape = (key) => key.replaceAll('~', '~0').replaceAll('/', '~1');

/** The value of a required key of `object`, an own property, or the refusal at its pointer. */
export const need = (object, pointer, key) => {
  if (!Object.hasOwn(object, key)) throw invalid(`${pointer}/${key}`, 'missing field');
  return object[key];
};

/** Refuses the first key of `object` that is not in `allowed`. */
export const onlyKeys = (object, pointer, allowed) => {
  const extra = Object.keys(object).find((key) => !allowed.includes(key));
  if (extra !== undefined) throw invalid(`${pointer}/${escape(extra)}`, 'unknown field');
};

/** A non-negative safe integer. */
export const uintAt = (value, pointer) => {
  if (!Number.isSafeInteger(value) || value < 0) throw invalid(pointer, 'expected a non-negative integer (a safe integer)');
  return value;
};

/** A safe integer, of either sign. */
export const intAt = (value, pointer) => {
  if (!Number.isSafeInteger(value)) throw invalid(pointer, 'expected an integer (a safe integer)');
  return value;
};

/** Exact greatest common divisor, shared by ratio reading and frame-time checks. */
export const gcd = (a, b) => {
  let [x, y] = [a < 0n ? -a : a, b < 0n ? -b : b];
  while (y !== 0n) [x, y] = [y, x % y];
  return x;
};

const boolAt = (value, pointer) => {
  if (typeof value !== 'boolean') throw invalid(pointer, 'expected true or false');
  return value;
};

const choiceAt = (value, pointer, options, message) => {
  if (typeof value !== 'string' || !options.includes(value)) throw invalid(pointer, message);
  return value;
};

/** `timing`: its keys and the eight numbers of each side. Presence and type only, no relation. */
const readTiming = (raw) => {
  const pointer = '/transition/timing';
  const timing = objectAt(raw, pointer);
  onlyKeys(timing, pointer, TIMING_KEYS);
  const edit = uintAt(need(timing, pointer, 'editDurationFrames'), `${pointer}/editDurationFrames`);
  const sides = {};
  for (const side of SIDES) {
    const at = `${pointer}/${side}`;
    const deck = objectAt(need(timing, pointer, side), at);
    onlyKeys(deck, at, DECK_TIMING_KEYS);
    const numbers = {};
    for (const key of DECK_TIMING_KEYS) numbers[key] = uintAt(need(deck, at, key), `${at}/${key}`);
    sides[side] = numbers;
  }
  return { editDurationFrames: edit, from: sides.from, to: sides.to };
};

/**
 * `params`: `[["curve", "linear" | "smooth"], ["intensity", n]]`, exactly the catalog's two, in order, and
 * `n` a number from 0 to 1. Anything else (a third pair, another name, an object) is refused in place.
 */
const readParams = (raw) => {
  const pointer = '/transition/params';
  if (!Array.isArray(raw) || raw.length !== 2) {
    throw invalid(pointer, 'must hold exactly the parameters of the effect, in catalog order');
  }
  const pair = (index, name) => {
    const at = `${pointer}/${index}`;
    const entry = raw[index];
    if (!Array.isArray(entry) || entry.length !== 2) throw invalid(at, 'expected a [name, value] pair');
    if (entry[0] !== name) throw invalid(at, 'must hold exactly the parameters of the effect, in catalog order');
    return [at, entry[1]];
  };
  const [curveAt, curve] = pair(0, 'curve');
  if (typeof curve !== 'string' || !CURVES.includes(curve)) throw invalid(curveAt, 'not one of the options of the parameter');
  const [intensityAt, intensity] = pair(1, 'intensity');
  if (typeof intensity !== 'number' || !Number.isFinite(intensity)) throw invalid(intensityAt, 'expected a number');
  if (intensity < 0 || intensity > 1) throw invalid(intensityAt, 'expected a number from 0 to 1');
  return { curve, intensity };
};

/**
 * The transition object as the core's `transition_json::read` takes it: keys, types, the neighbour only
 * as an object (it is read by the deck reader). Returns plain data; nothing of `raw` is kept.
 */
export const readTransitionWire = (raw) => {
  const pointer = '/transition';
  onlyKeys(raw, pointer, TRANSITION_KEYS);
  const alignment = choiceAt(need(raw, pointer, 'alignment'), `${pointer}/alignment`, ALIGNMENTS, 'expected `center`, `start` or `end`');
  const timing = readTiming(need(raw, pointer, 'timing'));
  const effect = need(raw, pointer, 'effect');
  if (typeof effect !== 'string') throw invalid(`${pointer}/effect`, 'expected a string');
  if (effect !== TRANSITION_EFFECT) throw invalid(`${pointer}/effect`, 'not a transition effect');
  const primary = choiceAt(need(raw, pointer, 'primary'), `${pointer}/primary`, SIDES, 'expected `from` or `to`');
  const easing = choiceAt(need(raw, pointer, 'easing'), `${pointer}/easing`, EASINGS, 'not an easing of the effects');
  const fields = {
    id: stringAt(need(raw, pointer, 'id'), `${pointer}/id`),
    effect,
    fromShotId: stringAt(need(raw, pointer, 'fromShotId'), `${pointer}/fromShotId`),
    toShotId: stringAt(need(raw, pointer, 'toShotId'), `${pointer}/toShotId`),
    primary,
    startFrame: uintAt(need(raw, pointer, 'startFrame'), `${pointer}/startFrame`),
    durationFrames: uintAt(need(raw, pointer, 'durationFrames'), `${pointer}/durationFrames`),
    alignment,
    timing,
    frameInWindow: uintAt(need(raw, pointer, 'frameInWindow'), `${pointer}/frameInWindow`),
    easing,
    progress: finiteAt(need(raw, pointer, 'progress'), `${pointer}/progress`),
    amount: finiteAt(need(raw, pointer, 'amount'), `${pointer}/amount`),
    params: readParams(need(raw, pointer, 'params')),
    heldInstance: boolAt(need(raw, pointer, 'heldInstance'), `${pointer}/heldInstance`),
    heldCamera: boolAt(need(raw, pointer, 'heldCamera'), `${pointer}/heldCamera`),
  };
  objectAt(need(raw, pointer, 'other'), `${pointer}/other`);
  return fields;
};

/**
 * The easing of the core (`apply_easing`): `t` is held to `[0, 1]` first. Written term by term as the
 * core does, so the doubles are the same ones.
 */
export const ease = (name, t) => {
  const x = t <= 0 ? 0 : t >= 1 ? 1 : t;
  if (name === 'linear') return x;
  if (name === 'easeOutCubic') {
    const p = 1 - x;
    return 1 - p * p * p;
  }
  if (x < 0.5) return 4 * x * x * x;
  const p = -2 * x + 2;
  return 1 - (p * p * p) / 2;
};

/**
 * What the transition says about itself (`check_transition_fields`): two different shots, a frame inside
 * its window that is the plan's real edit frame, and the `progress` and `amount` the core resolved.
 * `progress` is already eased and `amount` is the intensity: they are only compared, never recomputed
 * into a new value (`amount` is NOT `intensity x progress`).
 */
export const checkTransitionFields = (fields, editFrame) => {
  if (fields.fromShotId === fields.toShotId) throw invalid('/transition/toShotId', 'a transition joins two different shots');
  if (fields.durationFrames < 1) throw invalid('/transition/durationFrames', 'must be at least 1');
  if (fields.frameInWindow >= fields.durationFrames) {
    throw invalid('/transition/frameInWindow', 'must be inside the window, from 0 to durationFrames - 1');
  }
  if (BigInt(fields.startFrame) + BigInt(fields.frameInWindow) !== BigInt(editFrame)) {
    throw invalid('/transition/frameInWindow', 'must be the edit frame minus startFrame');
  }
  const expected = ease(fields.easing, (fields.frameInWindow + 0.5) / fields.durationFrames);
  if (fields.progress < 0 || fields.progress > 1 || Math.abs(fields.progress - expected) > MIX_TOLERANCE) {
    throw invalid('/transition/progress', 'must be the eased position in the window, ease((frameInWindow + 0.5) / durationFrames)');
  }
  const { intensity } = fields.params;
  if (fields.amount < 0 || fields.amount > 1 || Math.abs(fields.amount - Math.min(Math.max(intensity, 0), 1)) > MIX_TOLERANCE) {
    throw invalid('/transition/amount', 'must be the intensity of the transition, between 0 and 1');
  }
};
