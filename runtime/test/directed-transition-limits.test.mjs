// The bounds of the transition-plan reader (F064, P4b S2-R0): the item ceiling over both decks, the depth, and the
// byte ceiling that only the TEXT door has. Pure: no GL, no browser, no bridge.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  MAX_TRANSITION_PLAN_BYTES,
  MAX_TRANSITION_PLAN_ITEMS,
  parseTransitionPlan,
  parseTransitionPlanText,
} from '../src/directed-transition.js';
import { RuntimeError } from '../src/protocol.js';
import { basePlan, deepFreeze, edit } from './directed-transition-fixtures.mjs';

const refusal = (callback) => {
  try {
    callback();
  } catch (error) {
    assert.ok(error instanceof RuntimeError, `a typed RuntimeError, got ${error}`);
    return error;
  }
  return assert.fail('expected a refusal');
};

/** The base plan has 12 items over the two decks: per deck 2 nodes + 2 layers + 2 resources. */
const BASE_ITEMS = 12;

/** `count` more resources nobody uses, each a valid entry of its own. */
const extraResources = (prefix, count) =>
  Array.from({ length: count }, (_, index) => ({
    assetId: `${prefix}-${index}`,
    revisionId: `${prefix}-${index}-r1`,
    contentHash: `hash-${prefix}-${index}`,
    widthPx: 4,
    heightPx: 4,
  }));

const withExtra = (topCount, otherCount) => {
  const plan = basePlan();
  plan.resources.push(...extraResources('t', topCount));
  plan.transition.other.resources.push(...extraResources('o', otherCount));
  return plan;
};

// ---- items ---------------------------------------------------------------------------------------------------------

test('exactly 20000 items over both decks are read, and one more is refused at /transition', () => {
  assert.equal(MAX_TRANSITION_PLAN_ITEMS, 20000);
  const exact = withExtra(10000, 20000 - BASE_ITEMS - 10000);
  const got = parseTransitionPlan(exact, []);
  assert.equal(got.primary.resources.size, 10002);
  assert.equal(got.other.resources.size, 9990);

  const over = refusal(() => parseTransitionPlan(withExtra(10000, 20000 - BASE_ITEMS - 10000 + 1), []));
  assert.equal(over.code, 'invalid_plan');
  assert.equal(over.details.pointer, '/transition');
  assert.match(over.message, /20000/);
});

test('the neighbour and the lights are counted, whatever they hold, before any element is read', () => {
  // Junk the deck reader would refuse as a light or a node, if it ever got to read it: only the length counts first.
  const lights = basePlan();
  edit(lights, '/lights', new Array(MAX_TRANSITION_PLAN_ITEMS).fill(0));
  assert.equal(refusal(() => parseTransitionPlan(lights, [])).details.pointer, '/transition');

  const otherNodes = basePlan();
  edit(otherNodes, '/transition/other/nodes', new Array(MAX_TRANSITION_PLAN_ITEMS + 1).fill(null));
  assert.equal(refusal(() => parseTransitionPlan(otherNodes, [])).details.pointer, '/transition');

  const otherLights = basePlan();
  edit(otherLights, '/transition/other/lights', new Array(MAX_TRANSITION_PLAN_ITEMS).fill(null));
  assert.equal(refusal(() => parseTransitionPlan(otherLights, [])).details.pointer, '/transition');

  // Each deck alone is under the ceiling; together they are over it.
  const split = basePlan();
  edit(split, '/splats', new Array(10000).fill(0));
  edit(split, '/transition/other/fx', new Array(10000).fill(0));
  assert.equal(refusal(() => parseTransitionPlan(split, [])).details.pointer, '/transition');

  // All six arrays count: nodes, layers, splats, fx, resources, lights.
  for (const field of ['nodes', 'layers', 'splats', 'fx', 'resources', 'lights']) {
    const top = basePlan();
    edit(top, `/${field}`, new Array(MAX_TRANSITION_PLAN_ITEMS + 1).fill(0));
    assert.equal(refusal(() => parseTransitionPlan(top, [])).details.pointer, '/transition', `top ${field}`);
    const other = basePlan();
    edit(other, `/transition/other/${field}`, new Array(MAX_TRANSITION_PLAN_ITEMS + 1).fill(0));
    assert.equal(refusal(() => parseTransitionPlan(other, [])).details.pointer, '/transition', `other ${field}`);
  }
});

test('an array that is not an array counts nothing, and is refused where it is when it is read', () => {
  const plan = basePlan();
  edit(plan, '/lights', 'x'.repeat(30000));
  const error = refusal(() => parseTransitionPlan(plan, []));
  assert.equal(error.details.pointer, '/lights');
});

// ---- depth and cycles ----------------------------------------------------------------------------------------------

test('depth is one: the neighbour never has a transition of its own, however deep the chain behind it', () => {
  const plan = basePlan();
  let chain = { kind: 'tessel-scene-plan' };
  for (let level = 0; level < 100000; level += 1) chain = { transition: { other: chain } };
  edit(plan, '/transition/other/transition', chain);
  const error = refusal(() => parseTransitionPlan(plan, []));
  assert.equal(error.code, 'invalid_plan');
  assert.equal(error.details.pointer, '/transition/other/transition');
});

test('a cyclic object is refused without being followed', () => {
  const selfOther = basePlan();
  selfOther.transition.other = selfOther;
  assert.equal(refusal(() => parseTransitionPlan(selfOther, [])).details.pointer, '/transition/other/transition');

  const loop = basePlan();
  loop.transition.other.transition = loop.transition.other;
  assert.equal(refusal(() => parseTransitionPlan(loop, [])).details.pointer, '/transition/other/transition');

  const inNodes = basePlan();
  inNodes.nodes.push(inNodes);
  const error = refusal(() => parseTransitionPlan(inNodes, []));
  assert.equal(error.code, 'invalid_plan');
  assert.match(error.details.pointer, /^\/nodes\/2/);

  const inTransition = basePlan();
  inTransition.transition.params = inTransition;
  assert.equal(refusal(() => parseTransitionPlan(inTransition, [])).details.pointer, '/transition/params');
});

test('a nested array bomb in text is refused as a plan problem, never as a crash', () => {
  const depth = 200000;
  const text = `{"kind":"tessel-scene-plan","planVersion":1,"transition":${'['.repeat(depth)}${']'.repeat(depth)}}`;
  const error = refusal(() => parseTransitionPlanText(text, []));
  assert.equal(error.code, 'invalid_plan');
  assert.ok(error.details.pointer === '/transition' || error.details.pointer === '', error.message);
});

// ---- bytes: only the text door ---------------------------------------------------------------------------------------

const utf8 = (text) => Buffer.byteLength(text, 'utf8');

/** `text` padded with JSON whitespace to exactly `bytes` bytes. */
const padded = (text, bytes) => {
  const missing = bytes - utf8(text);
  assert.ok(missing >= 0, `the text is already ${utf8(text)} bytes`);
  return `${text}${' '.repeat(missing)}`;
};

test('a text of exactly 4194304 bytes is read, one more byte is refused at /transition before it is parsed', () => {
  assert.equal(MAX_TRANSITION_PLAN_BYTES, 4194304);
  const text = JSON.stringify(basePlan());
  const exact = padded(text, 4194304);
  assert.equal(utf8(exact), 4194304);
  assert.equal(parseTransitionPlanText(exact, []).mix.id, 'tr');

  const over = `${exact} `;
  assert.equal(utf8(over), 4194305);
  const error = refusal(() => parseTransitionPlanText(over, []));
  assert.equal(error.code, 'invalid_plan');
  assert.equal(error.details.pointer, '/transition');
  // Over the ceiling is refused before parsing: text that is not even JSON says the same.
  const notJson = refusal(() => parseTransitionPlanText(`{${' '.repeat(4194304)}`, []));
  assert.equal(notJson.details.pointer, '/transition');
});

test('the bytes are those of the text received, in UTF-8, not the characters and not a re-serialisation', () => {
  // Two decks with a long multibyte bundle id: 2 bytes a character, so the length is half the bytes.
  const plan = basePlan();
  const bundle = 'é'.repeat(1100000);
  plan.source.bundleId = bundle;
  plan.transition.other.source.bundleId = bundle;
  const text = JSON.stringify(plan);
  assert.ok(text.length < 4194304, `${text.length} characters`);
  assert.ok(utf8(text) > 4194304, `${utf8(text)} bytes`);
  const error = refusal(() => parseTransitionPlanText(text, []));
  assert.equal(error.details.pointer, '/transition');
  // The same plan as an object has no text, so the object door does not refuse it.
  assert.equal(parseTransitionPlan(plan, []).mix.id, 'tr');

  // Every size of character, at the exact boundary: 2 bytes (é), 3 (€), 4 (an emoji, a surrogate pair) and 3 for a
  // lone surrogate (what `TextEncoder` writes for it). Written raw into the text, in both decks' bundle id.
  const heavy = (character, count) => {
    const marked = basePlan();
    marked.source.bundleId = 'MARK';
    marked.transition.other.source.bundleId = 'MARK';
    return JSON.stringify(marked).replaceAll('"MARK"', `"${character.repeat(count)}"`);
  };
  for (const [name, character, bytes] of [['é', 'é', 2], ['€', '€', 3], ['emoji', '😀', 4], ['lone surrogate', '\u{D83D}', 3]]) {
    const count = Math.floor(2090000 / bytes);
    const text = heavy(character, count);
    assert.equal(utf8(character), bytes, name);
    assert.ok(utf8(text) < 4194304, `${name}: ${utf8(text)} bytes`);
    const exact = padded(text, 4194304);
    assert.equal(parseTransitionPlanText(exact, []).mix.id, 'tr', name);
    assert.equal(refusal(() => parseTransitionPlanText(`${exact} `, [])).details.pointer, '/transition', name);
    // Counted by characters this text would be well under the ceiling; by bytes it is over it.
    const over = heavy(character, count + 10000);
    assert.ok(utf8(over) > 4194304 && over.length < 4194304, `${name}: ${over.length} characters, ${utf8(over)} bytes`);
    assert.equal(refusal(() => parseTransitionPlanText(over, [])).details.pointer, '/transition', name);
  }
  // The text received: `"\u0074ransition"` is the key `transition`, written with 6 bytes more than `JSON.stringify`
  // of the object would. It is exactly at the ceiling as received, and one byte over it with a space more.
  const escaped = JSON.stringify(basePlan()).replace('"transition":', '"\\u0074ransition":');
  assert.ok(escaped.includes('\\u0074ransition'));
  const exact = padded(escaped, 4194304);
  assert.equal(parseTransitionPlanText(exact, []).mix.id, 'tr');
  assert.equal(refusal(() => parseTransitionPlanText(`${exact} `, [])).details.pointer, '/transition');
  assert.ok(utf8(JSON.stringify(JSON.parse(exact))) < 4194304, 'the re-serialised object is shorter than the text');
});

test('text that is not JSON, truncated text included, is invalid_plan at the root', () => {
  const full = JSON.stringify(basePlan());
  for (const text of ['', 'not json', '{', full.slice(0, full.length - 1), full.slice(0, 100), `${full}x`, 'undefined']) {
    const error = refusal(() => parseTransitionPlanText(text, []));
    assert.equal(error.code, 'invalid_plan', text.slice(0, 20));
    assert.equal(error.details.pointer, '', text.slice(0, 20));
    assert.match(error.message, /not JSON/);
  }
  for (const bad of [null, undefined, 1, {}, [], Buffer.from('{}')]) {
    assert.equal(refusal(() => parseTransitionPlanText(bad, [])).code, 'invalid_plan');
  }
  // JSON that is not a plan is the plan's problem, not the text's.
  for (const text of ['null', '[]', '1', '"x"']) {
    assert.equal(refusal(() => parseTransitionPlanText(text, [])).details.pointer, '');
  }
});

test('the object door has no byte ceiling to claim: it never serialises what it is given', () => {
  const plan = basePlan();
  plan.toJSON = () => assert.fail('the object door serialised the plan');
  deepFreeze(plan);
  assert.equal(Object.isFrozen(plan), true);
  assert.equal(parseTransitionPlan(plan, []).mix.id, 'tr');
});
