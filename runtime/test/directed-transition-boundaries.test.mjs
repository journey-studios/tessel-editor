// R1 review boundaries of the internal R0 reader. No browser, rendering or origin execution.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { parseDirectedLayer } from '../src/directed-scene.js';
import { parseTransitionPlan } from '../src/directed-transition.js';
import { checkResourceUnion } from '../src/directed-transition-decks.js';
import { RuntimeError } from '../src/protocol.js';
import { directedLayer } from './directed-scene-fixtures.mjs';
import { SCENARIOS, basePlan, buildPlan, clone } from './directed-transition-fixtures.mjs';

const refused = (read, pointer, code = 'invalid_plan') => {
  assert.throws(read, (error) => {
    assert.ok(error instanceof RuntimeError, `expected a typed refusal, got ${error}`);
    assert.equal(error.code, code);
    if (pointer !== null) assert.equal(error.details?.pointer, pointer);
    assert.doesNotThrow(() => JSON.stringify(error.toJSON()), 'a refusal can cross the bridge');
    return true;
  });
};

test('cyclic and BigInt versions give serializable typed refusals on both entries', () => {
  const cycle = {};
  cycle.self = cycle;
  for (const version of [cycle, 1n, { nested: 1n }]) {
    const plan = basePlan();
    plan.planVersion = version;
    const plain = { ...plan };
    delete plain.transition;
    const label = `[unserializable ${typeof version}]`;
    for (const read of [() => parseTransitionPlan(plan, []), () => parseDirectedLayer(directedLayer(plain, []))]) {
      assert.throws(read, (error) => {
        assert.ok(error instanceof RuntimeError);
        assert.equal(error.code, 'unsupported_plan_version');
        assert.deepEqual(error.details, { found: label, supported: 1 });
        assert.equal(error.message, `Scene plan version ${JSON.stringify(label)} is not supported (this runtime draws 1)`);
        assert.doesNotThrow(() => JSON.stringify(error.toJSON()));
        return true;
      });
    }
  }
  for (const version of [undefined, null, 2, '2', false, {}, [2]]) {
    const plan = basePlan();
    plan.planVersion = version;
    assert.throws(() => parseTransitionPlan(plan, []), (error) => {
      assert.deepEqual(error.details, { found: version, supported: 1 });
      assert.equal(error.message, `Scene plan version ${JSON.stringify(version)} is not supported (this runtime draws 1)`);
      return true;
    });
  }
});

test('the exclusive window end and a frame before the primary shot are refused', () => {
  // f46 is otherwise a coherent `to` plan; only the [34,46) window excludes it.
  const end = buildPlan({
    ...SCENARIOS.afterCut, f: 46, inWindow: 12, progress: 1,
    A: { ...SCENARIOS.afterCut.A, sI: 56 }, B: { ...SCENARIOS.afterCut.B, sI: 6, sC: 6 },
    top: { i: 6, c: 6 }, oth: { i: 56, c: 39 },
    times: { edit: [23, 12], top: { i: [1, 4], c: [1, 4] }, oth: { i: [7, 3], c: [13, 8] } },
  });
  refused(() => parseTransitionPlan(end, []), '/transition/frameInWindow');
  const early = basePlan();
  early.transition.timing.from.shotStartFrame = 37;
  refused(() => parseTransitionPlan(early, []), '/transition/timing/from/shotStartFrame');
});

test('a zero-length neighbour camera with positive raw time still samples zero', () => {
  const plan = buildPlan(SCENARIOS.emptyCamera); // f39, to starts40; cameraIn2 gives raw1.
  plan.transition.timing.to.cameraInFrame = 2;
  assert.deepEqual(parseTransitionPlan(plan, []).clocks.other.camera, { raw: 1, sampled: 0, held: true });
});

test('a transition window may cover the whole edit including both boundaries', () => {
  const plan = basePlan();
  Object.assign(plan.transition, { startFrame: 0, durationFrames: 100, frameInWindow: 36, easing: 'linear', progress: 0.365 });
  plan.transition.timing.from.shotEndFrameExclusive = 50;
  plan.transition.timing.to.shotStartFrame = 50;
  // from covers [0,50): instance10+50=60. Other raw36-50=-14 samples0.
  const got = parseTransitionPlan(plan, []);
  assert.equal(got.window.startFrame, 0);
  assert.equal(got.window.startFrame + got.window.durationFrames, 100);
  assert.deepEqual(got.clocks.other.instance, { raw: -14, sampled: 0, held: true });
});

test('zero intensity and progress inside tolerance are accepted, amount above one is not', () => {
  const zero = basePlan();
  zero.transition.params[1][1] = 0;
  zero.transition.amount = 0;
  assert.equal(parseTransitionPlan(zero, []).mix.amount, 0);
  const near = basePlan();
  near.transition.progress += 5e-10;
  assert.equal(parseTransitionPlan(near, []).mix.progress, near.transition.progress);
  const over = basePlan();
  over.transition.amount = 1 + 5e-10;
  refused(() => parseTransitionPlan(over, []), '/transition/amount');
});

test('a parameter pair has exactly two entries and its catalog name', () => {
  for (const pair of [['curve'], ['curve', 'smooth', 'extra'], ['constructor', 'smooth']]) {
    const plan = basePlan();
    plan.transition.params[0] = pair;
    refused(() => parseTransitionPlan(plan, []), '/transition/params/0');
  }
});

const atRate = (numerator, denominator, times) => {
  const plan = basePlan();
  for (const [deck, own] of [[plan, times], [plan.transition.other, [[0, 1], [0, 1]]]]) {
    deck.output.fps = { numerator, denominator };
    deck.frame.editTime = { numerator: times[0][0], denominator: times[0][1] };
    const local = deck === plan ? [times[1], times[2]] : own;
    [deck.frame.instanceTime, deck.frame.cameraClipTime] = local.map(([n, d]) => ({ numerator: n, denominator: d }));
  }
  return plan;
};

test('1000 fps and u32 fps terms at their limit are accepted, the next value is not', () => {
  const top = atRate(1000, 1, [[9, 250], [23, 500], [41, 1000]]);
  assert.equal(parseTransitionPlan(top, []).window.editFrame, 36);
  const unit = atRate(4294967295, 4294967295, [[36, 1], [46, 1], [41, 1]]);
  assert.equal(parseTransitionPlan(unit, []).window.editFrame, 36);
  const fast = clone(top);
  for (const deck of [fast, fast.transition.other]) deck.output.fps.numerator = 1001;
  refused(() => parseTransitionPlan(fast, []), '/output/fps');
  const tooLarge = clone(unit);
  tooLarge.output.fps.numerator = 4294967296;
  refused(() => parseTransitionPlan(tooLarge, []), '/output/fps/numerator');
});

test('the largest safe integer is read exactly while two to the 53 is refused', () => {
  const plan = basePlan();
  plan.transition.timing.from.instanceDurationFrames = Number.MAX_SAFE_INTEGER;
  assert.equal(parseTransitionPlan(plan, []).timing.from.instanceDurationFrames, 9007199254740991);
  plan.transition.timing.from.instanceDurationFrames = 9007199254740992;
  refused(() => parseTransitionPlan(plan, []), '/transition/timing/from/instanceDurationFrames');
});

test('the union helper discriminates format, alpha and color space on a shared asset', () => {
  const resource = basePlan().resources[0];
  for (const [key, value] of [['format', 'spz'], ['alpha', 'opaque'], ['colorSpace', 'linear']]) {
    refused(() => checkResourceUnion([resource], [{ ...resource, [key]: value }]), '/transition/other/resources/0');
  }
});

test('capability and transition guard errors precede a broken deck body', () => {
  const missing = basePlan();
  missing.requires = missing.requires.filter((name) => name !== 'fxTransition');
  missing.camera = null;
  refused(() => parseDirectedLayer(directedLayer(missing, [])), '/requires');
  const unknown = basePlan();
  unknown.requires.push('unknown-capability');
  unknown.camera = null;
  assert.throws(() => parseTransitionPlan(unknown, []), (error) => {
    assert.equal(error.code, 'unsupported');
    assert.deepEqual(error.details.missing, ['unknown-capability']);
    return true;
  });
});
