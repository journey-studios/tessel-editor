// The internal reader of the plan of a transition window (F064, P4b S2-R0): two decks and the mix.
//
// WHAT this is: a pure reading of the C0b wire (`ScenePlan.transition`, crates/core/src/scene_plan/
// transition*.rs) into what a later drawing slice will need: the drawing data of BOTH decks, read by the
// one deck reader (`directed-deck.js`), and the mix already resolved (`progress` is eased, `amount` is the
// intensity; this module only checks that they agree with the contract). WHAT it is not: it is wired to
// no compositor, bridge, worker or hello; it loads no asset, recomputes no pose, edits no timeline and
// draws nothing. `parseDirectedLayer` (the production door) still refuses every plan with a transition:
// recognising the shape here does not announce `fxTransition` and does not serve a frame with it.
//
// Two doors, both internal: `parseTransitionPlan` for an object and `parseTransitionPlanText` for the
// JSON text. Only the second measures bytes (the 4 MiB ceiling is a property of the TEXT the host sent);
// the first has none to measure. Both are pure: they never mutate the plan, the references or any array
// of them, and a refusal (`RuntimeError`, `invalid_plan` and the codes the deck reader already has)
// leaves nothing partial. Order of refusals: contract §3 of docs/migration/p4b-s2-r0-parser-plan.md.

import {
  assertComplete,
  assertPlanKind,
  invalid,
  objectAt,
  parseResourceReferences,
  readDeckBody,
  readRequires,
} from './directed-deck.js';
import { SCENE_CAPABILITIES } from './directed-scene.js';
import {
  checkDecksRelation,
  checkPasses,
  checkRequiresCover,
  checkResourceUnion,
  readDeckIdentity,
  readOtherDeck,
} from './directed-transition-decks.js';
import { checkTransitionFields, readTransitionWire } from './directed-transition-fields.js';
import { MAX_TRANSITION_PLAN_BYTES, MAX_TRANSITION_PLAN_ITEMS, checkDepth, checkItemCeiling, checkTextBytes } from './directed-transition-limits.js';
import { checkTransitionTiming } from './directed-transition-timing.js';

export { MAX_TRANSITION_PLAN_BYTES, MAX_TRANSITION_PLAN_ITEMS };

/**
 * What the transition reader negotiates: the capabilities the runtime executes plus `fxTransition`. The
 * production list (`SCENE_CAPABILITIES`, 13, advertised in `hello`) does NOT have it and is not touched.
 */
export const TRANSITION_CAPABILITIES = [...SCENE_CAPABILITIES, 'fxTransition'];

/** The passes of a deck by stage, in plan order: the shot's own, and the output's. */
const stageOf = (fx, stage) => fx.filter((pass) => pass.stage === stage);

/**
 * Reads a plan with a transition (an object, as `JSON.parse` or the host's bridge gives it) into the two
 * decks and the mix, or throws the typed refusal. `references` is `layer.resources`, the host's list of
 * where each resource was loaded (read as `parseDirectedLayer` reads it; not resolved here).
 *
 * @returns {{
 *   primary: object, other: object, mix: object, window: object, timing: object, clocks: object,
 *   passes: { primary: object[], other: object[], output: object[] }, references: Map<string, object>,
 * }}
 */
export const parseTransitionPlan = (plan, references) => {
  const root = objectAt(plan, '');
  assertPlanKind(root);
  if (!Object.hasOwn(root, 'transition')) throw invalid('/transition', 'missing field: a transition plan has a `transition`');
  const raw = objectAt(root.transition, '/transition');

  // Before any element is read, and before anything is entered (depth 1).
  checkItemCeiling(root, raw);
  checkDepth(raw);

  assertComplete(root);
  const requires = readRequires(root, TRANSITION_CAPABILITIES);
  if (!requires.includes('fxTransition')) throw invalid('/requires', 'a plan with a transition must require `fxTransition`');

  const fields = readTransitionWire(raw);
  const body = readDeckBody(root, requires);
  const identity = readDeckIdentity(root);
  checkTransitionFields(fields, identity.frame.editFrame);

  // The neighbour: the same reader, its refusals under `/transition/other`, read with ITS own requires.
  const neighbour = readOtherDeck(raw.other, SCENE_CAPABILITIES);
  checkRequiresCover(requires, neighbour.requires);

  checkDecksRelation(fields, identity, neighbour.identity);
  const { cut, clocks } = checkTransitionTiming(fields, identity, neighbour.identity);
  checkPasses(body.fx, neighbour.body.fx);
  checkResourceUnion(root.resources, neighbour.plan.resources);

  const { timing } = fields;
  return {
    primary: body,
    other: neighbour.body,
    mix: {
      id: fields.id,
      effect: fields.effect,
      fromShotId: fields.fromShotId,
      toShotId: fields.toShotId,
      primary: fields.primary,
      easing: fields.easing,
      curve: fields.params.curve,
      intensity: fields.params.intensity,
      progress: fields.progress,
      amount: fields.amount,
    },
    window: {
      startFrame: fields.startFrame,
      durationFrames: fields.durationFrames,
      frameInWindow: fields.frameInWindow,
      alignment: fields.alignment,
      cutFrame: cut,
      editFrame: identity.frame.editFrame,
    },
    timing: { editDurationFrames: timing.editDurationFrames, from: { ...timing.from }, to: { ...timing.to } },
    clocks,
    passes: {
      primary: stageOf(body.fx, 'deck'),
      other: stageOf(neighbour.body.fx, 'deck'),
      output: stageOf(body.fx, 'output'),
    },
    references: parseResourceReferences(references ?? []),
  };
};

/**
 * `parseTransitionPlan` for the JSON TEXT of a plan. The bytes are measured on the text received, in UTF-8,
 * before it is parsed (a plan over `MAX_TRANSITION_PLAN_BYTES` is refused at `/transition` without parsing);
 * text that is not JSON, truncated text included, is `invalid_plan` at `/`. The object door has no text, so
 * it has no byte ceiling: a transport that hands over an object does not get one from here.
 */
export const parseTransitionPlanText = (text, references) => {
  if (typeof text !== 'string') throw invalid('', 'expected the JSON text of a plan');
  checkTextBytes(text);
  let plan;
  try {
    plan = JSON.parse(text);
  } catch (error) {
    throw invalid('', `not JSON: ${error instanceof Error ? error.message : 'unreadable'}`);
  }
  return parseTransitionPlan(plan, references);
};
