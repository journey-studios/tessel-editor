// What the transition-plan reader ACCEPTS and returns (F064, P4b S2-R0). Pure: no GL, no browser, no bridge.
// The expected numbers are the hand-written ones of directed-transition-fixtures.mjs, each with its arithmetic.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { SCENE_CAPABILITIES, parseDirectedLayer } from '../src/directed-scene.js';
import {
  MAX_TRANSITION_PLAN_BYTES,
  MAX_TRANSITION_PLAN_ITEMS,
  TRANSITION_CAPABILITIES,
  parseTransitionPlan,
  parseTransitionPlanText,
} from '../src/directed-transition.js';
import { RuntimeError } from '../src/protocol.js';
import { directedLayer, reference } from './directed-scene-fixtures.mjs';
import { SCENARIOS, basePlan, buildPlan, clone, deepFreeze, edit, fadePass, shotScope } from './directed-transition-fixtures.mjs';

const close = (actual, expected, message) =>
  assert.ok(Math.abs(actual - expected) <= 1e-12, `${message ?? 'value'}: ${actual} vs ${expected}`);

/** The reader never writes into what it was given: it reads a frozen plan, and the plan is the same after. */
const read = (plan, references) => {
  const before = JSON.stringify(plan);
  deepFreeze(plan);
  const result = parseTransitionPlan(plan, references);
  assert.equal(JSON.stringify(plan), before, 'the plan is untouched');
  return result;
};

const sampledOf = (scenario) => ({ primary: scenario.primary === 'from' ? scenario.A : scenario.B, other: scenario.primary === 'from' ? scenario.B : scenario.A });

// The raw instance clock minus the edit frame: shot-a enters its instance at 10 on its first frame (0); shot-b at 0 on frame 40.
const A_OFFSET = { instance: 10 };
const B_OFFSET = { instance: -40 };

for (const [name, scenario] of Object.entries(SCENARIOS)) {
  test(`scenario \`${name}\`: the mix, the window and each deck's two clocks are what the contract gives`, () => {
    const got = read(buildPlan(scenario));
    const { primary, other } = sampledOf(scenario);

    assert.equal(got.mix.effect, 'crossDissolve');
    assert.equal(got.mix.primary, scenario.primary);
    assert.equal(got.mix.easing, scenario.easing);
    assert.equal(got.mix.curve, scenario.curve);
    assert.equal(got.mix.intensity, scenario.intensity);
    close(got.mix.progress, scenario.progress, 'progress');
    close(got.mix.amount, scenario.amount, 'amount');

    assert.deepEqual(got.window, {
      startFrame: scenario.startFrame,
      durationFrames: scenario.D,
      frameInWindow: scenario.inWindow,
      alignment: scenario.alignment,
      cutFrame: 40,
      editFrame: scenario.f,
    });
    assert.equal(got.timing.editDurationFrames, scenario.edit);
    assert.equal(got.timing.from.shotEndFrameExclusive, 40);
    assert.equal(got.timing.to.shotStartFrame, 40);

    // The primary deck's legacy counters are the RAW clock; the neighbour's are the held sample.
    assert.equal(got.clocks.primary.instance.raw, scenario.top.i);
    assert.equal(got.clocks.primary.camera.raw, scenario.top.c);
    assert.equal(got.clocks.primary.instance.sampled, primary.sI);
    assert.equal(got.clocks.primary.camera.sampled, primary.sC);
    assert.equal(got.clocks.other.instance.sampled, scenario.oth.i);
    assert.equal(got.clocks.other.camera.sampled, scenario.oth.c);
    assert.equal(got.clocks.other.instance.sampled, other.sI);
    assert.equal(got.clocks.other.camera.sampled, other.sC);
    assert.equal(got.clocks.other.instance.held, scenario.held[0]);
    assert.equal(got.clocks.other.camera.held, scenario.held[1]);
    // The two clocks of one deck do not depend on each other, nor the decks on each other.
    assert.equal(got.clocks.primary.instance.raw - scenario.f, (scenario.primary === 'from' ? A_OFFSET : B_OFFSET).instance);
  });
}

test('the primary is the shot of the frame: from before the cut, to from the cut on (C-1 / C)', () => {
  const before = read(buildPlan(SCENARIOS.beforeCut));
  const at = read(buildPlan(SCENARIOS.atCut));
  assert.equal(before.mix.primary, 'from');
  assert.equal(before.window.editFrame, 39);
  assert.equal(at.mix.primary, 'to');
  assert.equal(at.window.editFrame, 40);
  assert.equal(at.window.cutFrame, 40);
});

test('raw 41 against 40 camera frames is sample 39 and held, while the primary keeps the raw 41 (raw23 / sample11 shape)', () => {
  const got = read(buildPlan(SCENARIOS.base));
  assert.deepEqual(got.clocks.primary.camera, { raw: 41, sampled: 39, held: true });
  assert.deepEqual(got.clocks.primary.instance, { raw: 46, sampled: 46, held: false });
  assert.deepEqual(got.clocks.other.instance, { raw: -4, sampled: 0, held: true });
});

test('the two decks keep their own node ids and resources: the same ids are valid in both', () => {
  const got = read(basePlan());
  assert.deepEqual(got.primary.planes.map((plane) => plane.nodeId), ['backdrop', 'figure']);
  assert.deepEqual(got.other.planes.map((plane) => plane.nodeId), ['backdrop', 'figure']);
  assert.deepEqual(got.primary.planes.map((plane) => plane.assetId), ['wall-a', 'hero']);
  assert.deepEqual(got.other.planes.map((plane) => plane.assetId), ['wall-b', 'hero']);
  assert.deepEqual([...got.primary.resources.keys()].sort(), ['hero', 'wall-a']);
  assert.deepEqual([...got.other.resources.keys()].sort(), ['hero', 'wall-b']);
  assert.notEqual(got.primary.resources, got.other.resources, 'one Map per deck');
});

test('the passes are listed by owner: the shot of the primary, the shot of the neighbour and the output (top only)', () => {
  const plan = basePlan();
  for (const deck of [plan, plan.transition.other]) deck.requires.push('outputFx', 'fxShotScope');
  edit(plan, '/fx', [
    fadePass('p-shot', shotScope('shot-a')),
    fadePass('p-out', { kind: 'output' }),
  ]);
  edit(plan, '/transition/other/fx', [fadePass('o-shot', shotScope('shot-b'))]);
  edit(plan, '/transition/other/frame/shotId', 'shot-b');
  const got = read(plan);
  assert.deepEqual(got.passes.primary.map((pass) => pass.clipId), ['p-shot']);
  assert.deepEqual(got.passes.other.map((pass) => pass.clipId), ['o-shot']);
  assert.deepEqual(got.passes.output.map((pass) => pass.clipId), ['p-out']);
});

test('references are read as the production door reads them', () => {
  const got = read(basePlan(), [reference('hero', 'slot-hero')]);
  assert.equal(got.references.get('hero').slot, 'slot-hero');
  assert.equal(read(basePlan(), undefined).references.size, 0);
});

test('the text door reads the same plan as the object door', () => {
  const plan = basePlan();
  const text = JSON.stringify(plan);
  assert.deepEqual(parseTransitionPlanText(text, []), parseTransitionPlan(JSON.parse(text), []));
  assert.equal(MAX_TRANSITION_PLAN_BYTES, 4194304);
  assert.equal(MAX_TRANSITION_PLAN_ITEMS, 20000);
});

test('the reader negotiates the production capabilities plus fxTransition, and the production list does not change', () => {
  assert.equal(SCENE_CAPABILITIES.length, 13);
  assert.equal(SCENE_CAPABILITIES.includes('fxTransition'), false);
  assert.deepEqual(TRANSITION_CAPABILITIES, [...SCENE_CAPABILITIES, 'fxTransition']);
});

// ---- the production door keeps refusing a transition ---------------------------------------------------------------

const refusalOf = (callback) => {
  try {
    callback();
  } catch (error) {
    assert.ok(error instanceof RuntimeError, `a typed error, got ${error}`);
    return error;
  }
  return assert.fail('expected a refusal');
};

test('the production entry still refuses the plan this reader accepts: unsupported (it names fxTransition) or missing requires', () => {
  const plan = basePlan();
  const references = [reference('hero', 's-hero'), reference('wall-a', 's-wall')];
  const unsupported = refusalOf(() => parseDirectedLayer(directedLayer(clone(plan), references)));
  assert.equal(unsupported.code, 'unsupported');
  assert.deepEqual(unsupported.details.missing, ['fxTransition']);

  const withoutRequirement = clone(plan);
  withoutRequirement.requires = withoutRequirement.requires.filter((name) => name !== 'fxTransition');
  const invalidPlan = refusalOf(() => parseDirectedLayer(directedLayer(withoutRequirement, references)));
  assert.equal(invalidPlan.code, 'invalid_plan');
  assert.equal(invalidPlan.details.pointer, '/requires');
});
