// The two decks of a transition plan and what they must agree on (F064, P4b S2-R0).
//
// The neighbour under `transition.other` is a plan like any other and is read by the SAME deck reader as
// the primary (`directed-deck.js`): there is no second reader. Its pointers come out relative to its own
// root and are moved under `/transition/other` once, by `guardOther`; the error of the primary deck keeps
// the contract it always had. This file only adds what a single deck cannot know: the identity of each
// deck (source, output, frame) and the relations between the two (shots, version, output, clips, resources).
//
// It judges coherence, not where the plan came from. The checks are pure (no GL, no DOM).

import {
  arrayAt,
  assertComplete,
  assertPlanKind,
  invalid,
  isObject,
  objectAt,
  readDeckBody,
  readRequires,
  stringAt,
} from './directed-deck.js';
import { gcd, intAt, need, uintAt } from './directed-transition-fields.js';
import { RuntimeError } from './protocol.js';

/** Where the neighbour sits in the plan. */
export const OTHER = '/transition/other';

const MAX_U32 = 4_294_967_295;

/**
 * An error of the neighbour deck, seen from the top: the pointer moves under `/transition/other` (once:
 * a pointer already there is left alone) and a refusal that has no pointer says whose it is. The kind of
 * the error and its details are the deck reader's own.
 */
export const withinOther = (error) => {
  if (!(error instanceof RuntimeError)) return error;
  if (error.code === 'unsupported_plan_version') {
    return invalid(`${OTHER}/planVersion`, 'must be the version of the primary deck');
  }
  const details = isObject(error.details) ? error.details : {};
  if (details.deck === 'other' || (typeof details.pointer === 'string' && details.pointer.startsWith(`${OTHER}/`))) return error;
  if (error.code === 'invalid_plan' && typeof details.pointer === 'string') {
    const prefix = `Scene plan ${details.pointer || '/'}: `;
    const body = error.message.startsWith(prefix) ? error.message.slice(prefix.length) : error.message;
    return invalid(`${OTHER}${details.pointer}`, body);
  }
  const moved = { ...details, deck: 'other' };
  if (typeof details.pointer === 'string') moved.pointer = `${OTHER}${details.pointer}`;
  return new RuntimeError(error.code, error.message, moved);
};

/** Runs `read` and moves its refusal under `/transition/other`. */
export const guardOther = (read) => {
  try {
    return read();
  } catch (error) {
    throw withinOther(error);
  }
};

const ratioAt = (value, pointer) => {
  const ratio = objectAt(value, pointer);
  const numerator = intAt(need(ratio, pointer, 'numerator'), `${pointer}/numerator`);
  const denominator = intAt(need(ratio, pointer, 'denominator'), `${pointer}/denominator`);
  if (denominator <= 0 || gcd(BigInt(numerator), BigInt(denominator)) !== 1n) {
    throw invalid(pointer, 'expected a reduced ratio with a positive denominator');
  }
  return { numerator, denominator };
};

/**
 * What identifies a deck beyond its drawing: where it comes from, its output and its frame. Pointers are
 * the deck's own (`/frame/editFrame`), whichever deck it is.
 */
export const readDeckIdentity = (plan) => {
  const source = objectAt(plan.source, '/source');
  const output = objectAt(plan.output, '/output');
  const fps = objectAt(need(output, '/output', 'fps'), '/output/fps');
  const numerator = uintAt(need(fps, '/output/fps', 'numerator'), '/output/fps/numerator');
  const denominator = uintAt(need(fps, '/output/fps', 'denominator'), '/output/fps/denominator');
  if (numerator < 1 || numerator > MAX_U32) throw invalid('/output/fps/numerator', 'expected a whole number from 1 to 4294967295');
  if (denominator < 1 || denominator > MAX_U32) throw invalid('/output/fps/denominator', 'expected a whole number from 1 to 4294967295');
  const alphaMode = need(output, '/output', 'alphaMode');
  if (alphaMode !== 'straight' && alphaMode !== 'opaque') throw invalid('/output/alphaMode', 'expected `straight` or `opaque`');
  if (need(output, '/output', 'colorSpace') !== 'srgb-display') throw invalid('/output/colorSpace', 'only `srgb-display` is supported');
  const frame = objectAt(plan.frame, '/frame');
  return {
    source: {
      bundleId: stringAt(need(source, '/source', 'bundleId'), '/source/bundleId'),
      revisionId: stringAt(need(source, '/source', 'revisionId'), '/source/revisionId'),
      sceneInstanceId: stringAt(need(source, '/source', 'sceneInstanceId'), '/source/sceneInstanceId'),
    },
    output: { width: plan.output.width, height: plan.output.height, fps: { numerator, denominator }, alphaMode },
    frame: {
      editFrame: uintAt(need(frame, '/frame', 'editFrame'), '/frame/editFrame'),
      editTime: ratioAt(need(frame, '/frame', 'editTime'), '/frame/editTime'),
      shotId: stringAt(need(frame, '/frame', 'shotId'), '/frame/shotId'),
      instanceFrame: uintAt(need(frame, '/frame', 'instanceFrame'), '/frame/instanceFrame'),
      instanceTime: ratioAt(need(frame, '/frame', 'instanceTime'), '/frame/instanceTime'),
      cameraClipId: stringAt(need(frame, '/frame', 'cameraClipId'), '/frame/cameraClipId'),
      cameraClipFrame: uintAt(need(frame, '/frame', 'cameraClipFrame'), '/frame/cameraClipFrame'),
      cameraClipTime: ratioAt(need(frame, '/frame', 'cameraClipTime'), '/frame/cameraClipTime'),
    },
  };
};

/**
 * The neighbour deck: its header (kind, version, completeness, its own `requires`, which never name
 * `fxTransition`), its body by the shared reader, and its identity. Every refusal is moved under
 * `/transition/other`; what the TOP's `requires` must cover is the caller's, with the top's pointer.
 */
export const readOtherDeck = (raw, capabilities) =>
  guardOther(() => {
    const plan = objectAt(raw, '');
    assertPlanKind(plan);
    assertComplete(plan);
    const asked = arrayAt(plan.requires, '/requires');
    const nesting = asked.indexOf('fxTransition');
    if (nesting >= 0) throw invalid(`/requires/${nesting}`, 'the neighbour deck has no transition of its own: it does not require `fxTransition`');
    const requires = readRequires(plan, capabilities);
    return { plan, requires, body: readDeckBody(plan, requires), identity: readDeckIdentity(plan) };
  });

const sameRatio = (a, b) => a.numerator === b.numerator && a.denominator === b.denominator;

/**
 * The two decks agree on the world they are drawn in (`check_decks_relation`, without the clocks): the
 * shot of each deck, the output, the bundle revision and the real edit frame and time of the frame.
 */
export const checkDecksRelation = (fields, primary, other) => {
  const [primaryShot, otherShot] =
    fields.primary === 'from' ? [fields.fromShotId, fields.toShotId] : [fields.toShotId, fields.fromShotId];
  if (primary.frame.shotId !== primaryShot) {
    throw invalid(
      '/transition/primary',
      `the primary deck is the \`${fields.primary}\` shot (\`${primaryShot}\`), not the shot of the frame (\`${primary.frame.shotId}\`)`,
    );
  }
  if (other.frame.shotId !== otherShot) throw invalid(`${OTHER}/frame/shotId`, `the neighbour deck is the shot \`${otherShot}\``);
  const [left, right] = [primary.output, other.output];
  if (
    left.width !== right.width ||
    left.height !== right.height ||
    left.fps.numerator !== right.fps.numerator ||
    left.fps.denominator !== right.fps.denominator ||
    left.alphaMode !== right.alphaMode
  ) {
    throw invalid(`${OTHER}/output`, 'must be the output of the primary deck (size, frame rate, alpha)');
  }
  if (primary.source.bundleId !== other.source.bundleId || primary.source.revisionId !== other.source.revisionId) {
    throw invalid(`${OTHER}/source`, 'both decks come from the same bundle revision');
  }
  if (primary.frame.editFrame !== other.frame.editFrame) {
    throw invalid(`${OTHER}/frame/editFrame`, 'the neighbour deck is drawn at the real edit frame of the primary deck');
  }
  if (!sameRatio(primary.frame.editTime, other.frame.editTime)) {
    throw invalid(`${OTHER}/frame/editTime`, 'the scene clock is the real edit time, held or not');
  }
};

/** The top's `requires` cover what the neighbour asks for: the one list the runtime negotiates is the top's. */
export const checkRequiresCover = (requires, otherRequires) => {
  const lacking = otherRequires.find((capability) => !requires.includes(capability));
  if (lacking !== undefined) throw invalid('/requires', `must include \`${lacking}\`, which the neighbour deck requires`);
};

/**
 * The passes of the two decks: the output passes act once after the mix, in the primary deck only, and a
 * clip appears once per frame across both. The deck reader already refused a repeat WITHIN a deck.
 */
export const checkPasses = (primaryFx, otherFx) => {
  const clips = new Set(primaryFx.map((pass) => pass.clipId));
  otherFx.forEach((pass, index) => {
    if (pass.stage === 'output') {
      throw invalid(`${OTHER}/fx/${index}/scope`, 'the output passes act once after the mix, in the primary deck only');
    }
    if (clips.has(pass.clipId)) {
      throw invalid(`${OTHER}/fx/${index}/clipId`, 'a clip appears once per frame, across both decks');
    }
    clips.add(pass.clipId);
  });
};

/** What makes two entries of `resources` the same file. */
const FILE_KEYS = ['revisionId', 'contentHash', 'relativePath', 'widthPx', 'heightPx', 'alpha', 'colorSpace'];

const sameFile = (a, b) => (a.format ?? 'png') === (b.format ?? 'png') && FILE_KEYS.every((key) => a[key] === b[key]);

/**
 * One `assetId` in both decks is one file: the same revision, hash, path, format, size and colour. Node ids
 * are not part of it (two scenes may both have `root`); the decks keep their own `resources`.
 */
export const checkResourceUnion = (primaryResources, otherResources) => {
  const known = new Map(primaryResources.map((resource) => [resource.assetId, resource]));
  otherResources.forEach((resource, index) => {
    const found = known.get(resource.assetId);
    if (found !== undefined && !sameFile(found, resource)) {
      throw invalid(`${OTHER}/resources/${index}`, `asset \`${resource.assetId}\` is in both decks with different content`);
    }
  });
};
