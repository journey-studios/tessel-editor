// What the transition-plan reader REFUSES, and where (F064, P4b S2-R0). Pure: no GL, no browser, no bridge.
// Every refusal is a typed `RuntimeError` whose pointer names the field at fault, and none of them touches the
// plan (it is deep-frozen before it is read and compared byte for byte after).
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { invalid } from '../src/directed-deck.js';
import { parseTransitionPlan, parseTransitionPlanText } from '../src/directed-transition.js';
import { withinOther } from '../src/directed-transition-decks.js';
import { RuntimeError } from '../src/protocol.js';
import { SCENARIOS, basePlan, brokenBase, buildPlan, clone, deepFreeze, edit, fadePass, ratio, shotScope } from './directed-transition-fixtures.mjs';

const refuse = (plan, references = []) => {
  const before = JSON.stringify(plan);
  deepFreeze(plan);
  try {
    parseTransitionPlan(plan, references);
  } catch (error) {
    assert.ok(error instanceof RuntimeError, `a typed RuntimeError, got ${error}`);
    assert.equal(JSON.stringify(plan), before, 'a refusal leaves the plan as it was');
    return error;
  }
  return assert.fail('expected a refusal');
};

/** The plan with `changes` (pointer -> value, `undefined` deletes) must be refused as `invalid_plan` at `pointer`. */
const refusedAt = (pointer, changes, scenario = SCENARIOS.base) => {
  const plan = buildPlan(scenario);
  for (const [at, value] of Object.entries(changes)) edit(plan, at, value);
  const error = refuse(plan);
  assert.equal(error.code, 'invalid_plan', `${error.code}: ${error.message}`);
  assert.equal(error.details.pointer, pointer, error.message);
  assert.ok(error.message.startsWith(`Scene plan ${pointer}: `), error.message);
  return error;
};

const TRANSITION_KEYS = [
  'id', 'effect', 'fromShotId', 'toShotId', 'primary', 'startFrame', 'durationFrames', 'alignment', 'timing',
  'frameInWindow', 'easing', 'progress', 'amount', 'params', 'heldInstance', 'heldCamera', 'other',
];
const SIDE_KEYS = [
  'shotStartFrame', 'shotEndFrameExclusive', 'instanceStartFrame', 'instanceDurationFrames', 'cameraInFrame',
  'cameraClipDurationFrames', 'sampledInstanceFrame', 'sampledCameraClipFrame',
];

// ---- the wire ------------------------------------------------------------------------------------------------------

test('the old wire (no alignment, no timing) is refused at /transition/alignment, with no default', () => {
  const error = refusedAt('/transition/alignment', { '/transition/alignment': undefined, '/transition/timing': undefined });
  assert.match(error.message, /missing field/);
  refusedAt('/transition/alignment', { '/transition/alignment': undefined });
  refusedAt('/transition/timing', { '/transition/timing': undefined });
});

for (const key of TRANSITION_KEYS) {
  test(`transition.${key} is mandatory`, () => {
    refusedAt(`/transition/${key}`, { [`/transition/${key}`]: undefined });
  });
}

for (const side of ['from', 'to']) {
  for (const key of SIDE_KEYS) {
    test(`transition.timing.${side}.${key} is mandatory`, () => {
      refusedAt(`/transition/timing/${side}/${key}`, { [`/transition/timing/${side}/${key}`]: undefined });
    });
  }
}

test('an unknown key is refused where it is, in the transition and in each timing object', () => {
  refusedAt('/transition/extra', { '/transition/extra': 1 });
  refusedAt('/transition/timing/extra', { '/transition/timing/extra': 1 });
  refusedAt('/transition/timing/from/extra', { '/transition/timing/from/extra': 1 });
  refusedAt('/transition/timing/to/extra', { '/transition/timing/to/extra': 1 });
});

test('prototype names are not fields: they are unknown keys, and an inherited field is missing', () => {
  refusedAt('/transition/constructor', { '/transition/constructor': 1 });
  refusedAt('/transition/toString', { '/transition/toString': 1 });
  // `__proto__` as a real key can only come from JSON text.
  const text = JSON.stringify(basePlan()).replace('"transition":{', '"transition":{"__proto__":1,');
  const error = (() => {
    try {
      parseTransitionPlanText(text, []);
    } catch (caught) {
      return caught;
    }
    return assert.fail('expected a refusal');
  })();
  assert.equal(error.details.pointer, '/transition/__proto__');

  const plan = basePlan();
  const { alignment, ...rest } = plan.transition;
  const inherited = Object.assign(Object.create({ alignment }), rest);
  edit(plan, '/transition', inherited);
  assert.equal(refuse(plan).details.pointer, '/transition/alignment');

  const root = basePlan();
  const heir = Object.create(root);
  assert.equal(refuse(heir).details.pointer, '/transition', 'a `transition` the plan only inherits is not its own');
});

test('the plan, kind and transition must be what they are', () => {
  for (const bad of [null, undefined, 1, 'x', true, []]) {
    assert.equal(refuse(bad).details.pointer, '', 'the root of the plan (message: `Scene plan /`)');
  }
  assert.equal(refuse({ ...basePlan(), kind: 'other' }).details.pointer, '/kind');
  const noTransition = basePlan();
  delete noTransition.transition;
  assert.equal(refuse(noTransition).details.pointer, '/transition');
  for (const bad of [null, 1, 'x', true, []]) {
    refusedAt('/transition', { '/transition': bad });
  }
  const version = refuse({ ...basePlan(), planVersion: 2 });
  assert.equal(version.code, 'unsupported_plan_version');
});

const BAD_INTEGERS = [-1, 1.5, '1', null, true, NaN, Infinity, -Infinity, 2 ** 53, [1], {}];
for (const [pointer, fields] of [
  ['/transition/startFrame', {}],
  ['/transition/durationFrames', {}],
  ['/transition/frameInWindow', {}],
  ['/transition/timing/editDurationFrames', {}],
  ...SIDE_KEYS.flatMap((key) => ['from', 'to'].map((side) => [`/transition/timing/${side}/${key}`, {}])),
]) {
  test(`${pointer} must be a safe whole number, never negative`, () => {
    for (const bad of BAD_INTEGERS) {
      const plan = basePlan();
      edit(plan, pointer, bad);
      const error = refuse(plan);
      assert.equal(error.code, 'invalid_plan', `${String(bad)}: ${error.message}`);
      assert.equal(error.details.pointer, pointer, `${String(bad)}: ${error.message}`);
    }
    void fields;
  });
}

test('strings, enumerations, the booleans and the mix numbers must be what the wire says', () => {
  for (const bad of ['', 1, null, true, [], {}]) {
    refusedAt('/transition/id', { '/transition/id': bad });
    refusedAt('/transition/fromShotId', { '/transition/fromShotId': bad });
    refusedAt('/transition/toShotId', { '/transition/toShotId': bad });
  }
  refusedAt('/transition/toShotId', { '/transition/toShotId': 'shot-a' });
  for (const bad of ['wipe', 'CrossDissolve', '', 1, null]) refusedAt('/transition/effect', { '/transition/effect': bad });
  for (const bad of ['both', 'From', '', 1, null]) refusedAt('/transition/primary', { '/transition/primary': bad });
  for (const bad of ['mid', 'Center', '', 1, null]) refusedAt('/transition/alignment', { '/transition/alignment': bad });
  for (const bad of ['bounce', 'Linear', '', 1, null]) refusedAt('/transition/easing', { '/transition/easing': bad });
  for (const bad of ['yes', 1, 0, null, undefined === 0 ? 0 : 'true', []]) {
    refusedAt('/transition/heldInstance', { '/transition/heldInstance': bad });
    refusedAt('/transition/heldCamera', { '/transition/heldCamera': bad });
  }
  for (const bad of ['0.5', null, NaN, Infinity, -0.1, 1.5, true]) {
    refusedAt('/transition/progress', { '/transition/progress': bad });
    refusedAt('/transition/amount', { '/transition/amount': bad });
  }
});

test('params are exactly [["curve", linear|smooth], ["intensity", 0..1]], in that order', () => {
  refusedAt('/transition/params', { '/transition/params': 'x' });
  refusedAt('/transition/params', { '/transition/params': null });
  refusedAt('/transition/params', { '/transition/params': [] });
  refusedAt('/transition/params', { '/transition/params': [['curve', 'smooth']] });
  refusedAt('/transition/params', { '/transition/params': [['curve', 'smooth'], ['intensity', 1], ['extra', 1]] });
  refusedAt('/transition/params/0', { '/transition/params/0': ['intensity', 1] });
  refusedAt('/transition/params/0', { '/transition/params/0': 'curve' });
  refusedAt('/transition/params/0', { '/transition/params/0': ['curve'] });
  refusedAt('/transition/params/0', { '/transition/params/0': ['curve', 'cubic'] });
  refusedAt('/transition/params/0', { '/transition/params/0': ['curve', 1] });
  refusedAt('/transition/params/1', { '/transition/params/1': ['curve', 'smooth'] });
  for (const bad of [-0.1, 1.1, '1', null, NaN, Infinity]) {
    refusedAt('/transition/params/1', { '/transition/params/1': ['intensity', bad] });
  }
});

test('frameInWindow lies in [0, duration) and startFrame + frameInWindow is the edit frame of the deck', () => {
  refusedAt('/transition/frameInWindow', { '/transition/frameInWindow': 12 });
  refusedAt('/transition/frameInWindow', { '/transition/frameInWindow': 3 });
  refusedAt('/transition/frameInWindow', { '/transition/frameInWindow': 1 });
  refusedAt('/transition/durationFrames', { '/transition/durationFrames': 0 });
});

test('progress is the eased (frameInWindow + 0.5) / duration, and amount is the intensity, not intensity x progress', () => {
  // 2 / 12 instead of 2.5 / 12: the half frame is missing. 4 t^3 = 4 * (1/6)^3 = 1/54.
  refusedAt('/transition/progress', { '/transition/progress': 1 / 54 });
  // No easing at all (the base is easeInOutCubic): linear 5/24.
  refusedAt('/transition/progress', { '/transition/progress': 5 / 24 });
  refusedAt('/transition/progress', { '/transition/progress': 0 }, SCENARIOS.afterCut);
  // afterCut has intensity 1/2: the amount is 1/2, not 1/2 x 3113/3456, not the progress.
  refusedAt('/transition/amount', { '/transition/amount': 0.5 * (3113 / 3456) }, SCENARIOS.afterCut);
  refusedAt('/transition/amount', { '/transition/amount': 3113 / 3456 }, SCENARIOS.afterCut);
  refusedAt('/transition/amount', { '/transition/amount': 1 }, SCENARIOS.afterCut);
  // The tolerance is 1e-9: 1e-10 away is the same number, 1e-8 away is not.
  const base = SCENARIOS.base;
  assert.doesNotThrow(() => parseTransitionPlan(brokenBase({ '/transition/progress': base.progress + 1e-10 }), []));
  refusedAt('/transition/progress', { '/transition/progress': base.progress + 1e-8 });
});

// ---- the window and the shots --------------------------------------------------------------------------------------

test('the window is the one its alignment gives, inside the edit', () => {
  // The window one frame early: 33 + 3 = 36, so the frame is still in it (progress at 3.5 / 12 = 7/24: 4 t^3 = 343/3456),
  // but center D12 starts at 34.
  refusedAt('/transition/startFrame', { '/transition/startFrame': 33, '/transition/frameInWindow': 3, '/transition/progress': 343 / 3456 });
  // `start` would put the window at the cut.
  refusedAt('/transition/startFrame', { '/transition/alignment': 'start' });
  refusedAt('/transition/startFrame', { '/transition/alignment': 'end' });
  // A window longer than the edit (101 of 100 frames; its eased progress at 2.5 / 101 is 4 (2.5/101)^3).
  refusedAt('/transition/durationFrames', { '/transition/durationFrames': 101, '/transition/progress': 4 * (2.5 / 101) ** 3 });
  // `start` D70 from the cut 40 would end at 110, after the 100 frames of the edit (linear 0.5 / 70).
  refusedAt(
    '/transition/startFrame',
    { '/transition/alignment': 'start', '/transition/durationFrames': 70, '/transition/startFrame': 40, '/transition/frameInWindow': 0, '/transition/progress': 0.5 / 70 },
    SCENARIOS.atCut,
  );
});

test('the primary is the shot of the frame: from before the cut, to from it on', () => {
  refusedAt('/transition/primary', { '/transition/primary': 'to' });
  refusedAt('/transition/primary', { '/transition/primary': 'from' }, SCENARIOS.afterCut);
  refusedAt('/transition/primary', { '/transition/primary': 'from' }, SCENARIOS.atCut);
  refusedAt('/transition/primary', { '/transition/primary': 'to' }, SCENARIOS.beforeCut);
  // The same with both decks' shots told to agree with the declared primary: only the cut can refuse it now.
  // Frame 42 with `from` and shot-a: past the cut, so it is `to`'s turn.
  refusedAt('/transition/primary', { '/transition/primary': 'from', '/frame/shotId': 'shot-a', '/transition/other/frame/shotId': 'shot-b' }, SCENARIOS.afterCut);
  refusedAt('/transition/primary', { '/transition/primary': 'to', '/frame/shotId': 'shot-b', '/transition/other/frame/shotId': 'shot-a' }, SCENARIOS.beforeCut);
});

test('an error of the neighbour that already points under /transition/other is not moved a second time', () => {
  const already = invalid('/transition/other/nodes/0', 'x');
  assert.equal(withinOther(already), already);
  const own = invalid('/nodes/0', 'x');
  const moved = withinOther(own);
  assert.equal(moved.details.pointer, '/transition/other/nodes/0');
  assert.equal(moved.message, 'Scene plan /transition/other/nodes/0: x');
  const root = withinOther(invalid('', 'x'));
  assert.equal(root.details.pointer, '/transition/other');
  // Not the plan's: anything that is not a RuntimeError is left alone.
  const stray = new Error('boom');
  assert.equal(withinOther(stray), stray);
});

test('the cut is where from ends and to starts, and the shots lie inside the edit', () => {
  refusedAt('/transition/timing/from/shotEndFrameExclusive', { '/transition/timing/to/shotStartFrame': 41 });
  refusedAt('/transition/timing/from/shotEndFrameExclusive', { '/transition/timing/from/shotEndFrameExclusive': 39 });
  refusedAt('/transition/timing/to/shotEndFrameExclusive', { '/transition/timing/to/shotEndFrameExclusive': 101 });
  refusedAt('/transition/timing/to/shotEndFrameExclusive', { '/transition/timing/to/shotEndFrameExclusive': 40 });
  refusedAt('/transition/timing/from/shotEndFrameExclusive', { '/transition/timing/from/shotStartFrame': 40 });
});

test('the frame exactly at the exclusive end of the primary shot is outside it', () => {
  // Window `start`, D 60: [40, 100), and shot-b ends at 90 (exclusive). Frame 89 is its last frame; frame 90 is outside it.
  // f = 89: linear 49.5 / 60 = 33/40. shot-b: delta 49 -> instance 49, camera 49, inside. shot-a (neighbour): delta 89:
  // instance 10 + 89 = 99 -> 59 held; camera 5 + 89 = 94 -> 39 held. Times: 89/24, 49/24, 59/24, 39/24 = 13/8.
  const last = {
    ...SCENARIOS.startAligned,
    f: 89, D: 60, startFrame: 40, inWindow: 49, progress: 33 / 40,
    A: { ...SCENARIOS.startAligned.A, sI: 59, sC: 39 },
    B: { ...SCENARIOS.startAligned.B, end: 90, sI: 49, sC: 49 },
    top: { i: 49, c: 49 }, oth: { i: 59, c: 39 }, held: [true, true],
    times: { edit: [89, 24], top: { i: [49, 24], c: [49, 24] }, oth: { i: [59, 24], c: [13, 8] } },
  };
  const got = parseTransitionPlan(buildPlan(last), []);
  assert.equal(got.window.editFrame, 89);
  assert.deepEqual(got.clocks.other.camera, { raw: 94, sampled: 39, held: true });
  // f = 90: linear 50.5 / 60 = 101/120, in the window and in the edit, but not in the shot of the primary.
  const at90 = {
    ...last,
    f: 90, inWindow: 50, progress: 101 / 120,
    times: { edit: [15, 4], top: { i: [25, 12], c: [25, 12] }, oth: { i: [25, 12], c: [13, 8] } },
  };
  const error = refuse(buildPlan(at90));
  assert.equal(error.details.pointer, '/transition/timing/to/shotEndFrameExclusive');
});

// ---- identities, clocks, sampled values and times ------------------------------------------------------------------

test('a declared sample is the one the clock gives, never the raw value or the other deck\'s', () => {
  refusedAt('/transition/timing/from/sampledCameraClipFrame', { '/transition/timing/from/sampledCameraClipFrame': 40 });
  refusedAt('/transition/timing/from/sampledCameraClipFrame', { '/transition/timing/from/sampledCameraClipFrame': 41 });
  refusedAt('/transition/timing/from/sampledInstanceFrame', { '/transition/timing/from/sampledInstanceFrame': 45 });
  refusedAt('/transition/timing/to/sampledInstanceFrame', { '/transition/timing/to/sampledInstanceFrame': -1 + 2 });
  refusedAt('/transition/timing/to/sampledCameraClipFrame', { '/transition/timing/to/sampledCameraClipFrame': 1 });
  // Raw and sample swapped: the neighbour's raw is -4, its sample 0.
  refusedAt('/transition/timing/from/sampledInstanceFrame', { '/transition/timing/from/sampledInstanceFrame': 36 });
});

test('the held flags are exactly the clock\'s, `false` included', () => {
  refusedAt('/transition/heldInstance', { '/transition/heldInstance': false });
  refusedAt('/transition/heldCamera', { '/transition/heldCamera': false });
  refusedAt('/transition/heldInstance', { '/transition/heldInstance': true }, SCENARIOS.afterCut);
  refusedAt('/transition/heldCamera', { '/transition/heldCamera': false }, SCENARIOS.afterCut);
  // A camera with no frame is always held, and one with the last real frame is not.
  refusedAt('/transition/heldCamera', { '/transition/heldCamera': false }, SCENARIOS.emptyCamera);
  refusedAt('/transition/heldCamera', { '/transition/heldCamera': true }, SCENARIOS.lastRealFrame);
});

test('the two clocks of a deck, and the two decks, are independent of one another', () => {
  // The neighbour on the primary\'s clock (46 instead of the sample 0) or on its delta.
  refusedAt('/transition/other/frame/instanceFrame', { '/transition/other/frame/instanceFrame': 46 });
  refusedAt('/transition/other/frame/cameraClipFrame', { '/transition/other/frame/cameraClipFrame': 39 });
  // The primary camera on the instance clock (46) instead of its own raw 41.
  refusedAt('/frame/cameraClipFrame', { '/frame/cameraClipFrame': 46 });
  refusedAt('/frame/instanceFrame', { '/frame/instanceFrame': 39 });
  refusedAt('/frame/instanceFrame', { '/frame/instanceFrame': 45 });
  // The primary keeps the RAW camera clock: the sample is a different number.
  refusedAt('/frame/cameraClipFrame', { '/frame/cameraClipFrame': 39 });
});

test('an instance or a camera clip shared by both decks has one length, and each instance covers its shot', () => {
  refusedAt('/transition/timing/to/instanceDurationFrames', { '/transition/other/source/sceneInstanceId': 'inst-a' });
  refusedAt('/transition/timing/to/cameraClipDurationFrames', { '/transition/other/frame/cameraClipId': 'cam-a' });
  // 10 + (40 - 0) = 50 frames needed from shot-a's instance; 60 + 0 for shot-b's.
  refusedAt('/transition/timing/from/instanceDurationFrames', { '/transition/timing/from/instanceDurationFrames': 49 });
  refusedAt('/transition/timing/to/instanceDurationFrames', { '/transition/timing/to/instanceDurationFrames': 59 });
  refusedAt('/transition/timing/to/instanceDurationFrames', { '/transition/timing/to/instanceStartFrame': 141 });
});

test('a non-reduced ratio is refused at its own field before comparing the decks', () => {
  for (const prefix of ['', '/transition/other']) {
    for (const field of ['editTime', 'instanceTime', 'cameraClipTime']) {
      const plan = basePlan();
      const frame = prefix ? plan.transition.other.frame : plan.frame;
      const { numerator, denominator } = frame[field];
      const at = `${prefix}/frame/${field}`;
      refusedAt(at, { [at]: ratio(numerator * 2, denominator * 2) });
    }
  }
});

test('zero over two is refused during reading before a later frame or held fault', () => {
  refusedAt('/transition/other/frame/instanceTime', {
    '/transition/other/frame/instanceTime': ratio(0, 2), '/transition/heldInstance': false,
  });
  refusedAt('/frame/editTime', { '/frame/editTime': ratio(0, 2) });
});

test('every time is the exact reduced ratio counter x den / num, in both decks', () => {
  for (const field of ['instanceTime', 'cameraClipTime']) {
    refusedAt(`/frame/${field}`, { [`/frame/${field}`]: ratio(1, 2) });
    refusedAt(`/transition/other/frame/${field}`, { [`/transition/other/frame/${field}`]: ratio(1, 2) });
  }
  // The edit time is one number in both decks; wrong in both it is still wrong (the top is read first).
  refusedAt('/frame/editTime', { '/frame/editTime': ratio(1, 2), '/transition/other/frame/editTime': ratio(1, 2) });
  refusedAt('/transition/other/frame/editTime', { '/frame/editTime': ratio(1, 2) });
  refusedAt('/transition/other/frame/editTime', { '/transition/other/frame/editTime': ratio(1, 2) });
  refusedAt('/frame/instanceTime', { '/frame/instanceTime': ratio(46, 24) }); // not reduced
  refusedAt('/frame/instanceTime', { '/frame/instanceTime': ratio(23, 0) });
  refusedAt('/frame/instanceTime', { '/frame/instanceTime': ratio(-23, 12) });
  refusedAt('/frame/instanceTime/denominator', { '/frame/instanceTime': { numerator: 23 } });
  refusedAt('/frame/instanceTime/numerator', { '/frame/instanceTime': ratio(23.5, 12) });
  refusedAt('/frame/instanceTime', { '/frame/instanceTime': 23 / 12 });
  // 30000/1001: the same frame number with the 24 fps ratios is wrong.
  refusedAt('/frame/editTime', { '/frame/editTime': ratio(3, 2), '/transition/other/frame/editTime': ratio(3, 2) }, SCENARIOS.ntsc);
  refusedAt('/output/fps/denominator', { '/output/fps/denominator': 0 });
  refusedAt('/output/fps/numerator', { '/output/fps/numerator': 0 });
  // 25001/24 is more than 1000 frames a second (the core's `Rate`), in both decks so that they agree.
  refusedAt('/output/fps', {
    '/output/fps/numerator': 25001, '/output/fps/denominator': 24,
    '/transition/other/output/fps/numerator': 25001, '/transition/other/output/fps/denominator': 24,
  });
});

test('the real edit time is the same in both decks', () => {
  refusedAt('/transition/other/frame/editFrame', { '/transition/other/frame/editFrame': 37 });
});

// ---- the second deck -----------------------------------------------------------------------------------------------

test('the neighbour must be a plan: a refusal in it is pointed under /transition/other, once', () => {
  for (const bad of [null, 1, 'x', true, []]) refusedAt('/transition/other', { '/transition/other': bad });
  refusedAt('/transition/other/kind', { '/transition/other/kind': 'x' });
  const node = refusedAt('/transition/other/nodes/0/widthSceneUnits', { '/transition/other/nodes/0/widthSceneUnits': -1 });
  assert.doesNotMatch(node.message, /\/transition\/other\/transition\/other/);
  // The deck at the top keeps the pointer it always had.
  refusedAt('/nodes/0/widthSceneUnits', { '/nodes/0/widthSceneUnits': -1 });
  refusedAt('/transition/other/layers/1/assetId', { '/transition/other/layers/1/assetId': 'ghost' });
});

test('the neighbour is of the same version, output, bundle and shots as the primary', () => {
  const version = refuse(brokenBase({ '/transition/other/planVersion': 2 }));
  assert.equal(version.code, 'invalid_plan');
  assert.equal(version.details.pointer, '/transition/other/planVersion');
  refusedAt('/transition/other/output', { '/transition/other/output/width': 161 });
  refusedAt('/transition/other/output', { '/transition/other/output/height': 91 });
  refusedAt('/transition/other/output', { '/transition/other/output/fps/numerator': 25 });
  refusedAt('/transition/other/output', { '/transition/other/output/fps/denominator': 2 });
  refusedAt('/transition/other/output', { '/transition/other/output/alphaMode': 'opaque' });
  refusedAt('/transition/other/source', { '/transition/other/source/bundleId': 'other' });
  refusedAt('/transition/other/source', { '/transition/other/source/revisionId': 'rev-2' });
  refusedAt('/transition/other/frame/shotId', { '/transition/other/frame/shotId': 'shot-a' });
  refusedAt('/transition/other/frame/shotId', { '/transition/other/frame/shotId': 'elsewhere' });
  refusedAt('/transition/primary', { '/frame/shotId': 'shot-b' });
});

test('the neighbour has no transition of its own, and does not ask for one', () => {
  refusedAt('/transition/other/transition', { '/transition/other/transition': {} });
  refusedAt('/transition/other/transition', { '/transition/other/transition': null });
  refusedAt('/transition/other/requires/5', { '/transition/other/requires': ['camera', 'nodeTransforms', 'imagePlanes', 'straightAlpha', 'opaqueAlpha', 'fxTransition'] });
});

test('the capabilities: the top must require fxTransition and cover the neighbour; unknown ones are unsupported', () => {
  const lacking = refusedAt('/requires', { '/requires': ['camera', 'nodeTransforms', 'imagePlanes', 'straightAlpha', 'opaqueAlpha'] });
  assert.match(lacking.message, /fxTransition/);
  refusedAt('/requires', { '/requires/4': undefined, '/transition/other/requires/4': 'opaqueAlpha' });
  const unknown = refuse(brokenBase({ '/requires/0': 'teleport' }));
  assert.equal(unknown.code, 'unsupported');
  assert.deepEqual(unknown.details.missing, ['teleport']);
  const partial = refuse(brokenBase({ '/complete': false }));
  assert.equal(partial.code, 'unsupported');
  const partialOther = refuse(brokenBase({ '/transition/other/complete': false }));
  assert.equal(partialOther.code, 'unsupported');
  assert.equal(partialOther.details.deck, 'other');
  const otherUnknown = refuse(brokenBase({ '/transition/other/requires/0': 'teleport' }));
  assert.equal(otherUnknown.code, 'unsupported');
  assert.deepEqual(otherUnknown.details.missing, ['teleport']);
  assert.equal(otherUnknown.details.deck, 'other');
});

const withFx = (plan) => {
  for (const deck of [plan, plan.transition.other]) deck.requires.push('outputFx', 'fxShotScope');
  return plan;
};

test('the passes: the output ones act once, in the top; a clip appears once across both decks', () => {
  const output = withFx(basePlan());
  edit(output, '/transition/other/fx', [fadePass('o1', { kind: 'output' })]);
  assert.equal(refuse(output).details.pointer, '/transition/other/fx/0/scope');

  const second = withFx(basePlan());
  edit(second, '/transition/other/fx', [fadePass('o1', shotScope('shot-b')), { ...fadePass('o2', { kind: 'output' }) }]);
  assert.equal(refuse(second).details.pointer, '/transition/other/fx/1/scope');

  const duplicate = withFx(basePlan());
  edit(duplicate, '/fx', [fadePass('same', shotScope('shot-a'))]);
  edit(duplicate, '/transition/other/fx', [fadePass('same', shotScope('shot-b'))]);
  assert.equal(refuse(duplicate).details.pointer, '/transition/other/fx/0/clipId');

  const outputClip = withFx(basePlan());
  edit(outputClip, '/fx', [fadePass('same', { kind: 'output' })]);
  edit(outputClip, '/transition/other/fx', [fadePass('same', shotScope('shot-b'))]);
  assert.equal(refuse(outputClip).details.pointer, '/transition/other/fx/0/clipId');

  // A pass of the neighbour must be of ITS shot, as the deck reader says for any deck.
  const wrongShot = withFx(basePlan());
  edit(wrongShot, '/transition/other/fx', [fadePass('w', shotScope('shot-a'))]);
  assert.equal(refuse(wrongShot).details.pointer, '/transition/other/fx/0/scope/shotId');
});

test('one asset id in both decks is one file; different node ids are not a conflict', () => {
  for (const [field, value] of [
    ['revisionId', 'hero-r9'], ['contentHash', 'other'], ['relativePath', 'assets/else.png'], ['widthPx', 9], ['heightPx', 9],
    ['alpha', 'opaque'], ['colorSpace', 'linear'],
  ]) {
    const plan = basePlan();
    const index = plan.transition.other.resources.findIndex((entry) => entry.assetId === 'hero');
    edit(plan, `/transition/other/resources/${index}/${field}`, value);
    const error = refuse(plan);
    assert.ok(error.code === 'invalid_plan' || error.code === 'unsupported', `${field}: ${error.message}`);
    assert.ok(error.details.pointer.startsWith(`/transition/other/resources/${index}`), `${field}: ${error.message}`);
  }
  // `format` left out means png, in either deck: the same file.
  const defaulted = basePlan();
  const heroAt = defaulted.transition.other.resources.findIndex((entry) => entry.assetId === 'hero');
  edit(defaulted, `/transition/other/resources/${heroAt}/format`, undefined);
  assert.doesNotThrow(() => parseTransitionPlan(defaulted, []));
  const other = basePlan();
  edit(other, '/transition/other/nodes/0/id', 'something-else');
  edit(other, '/transition/other/layers/0/nodeId', 'something-else');
  assert.doesNotThrow(() => parseTransitionPlan(other, []));
});

test('a clone of the plan is read the same: the reader keeps no state between calls', () => {
  const plan = basePlan();
  const first = parseTransitionPlan(clone(plan), []);
  assert.throws(() => parseTransitionPlan(brokenBase({ '/transition/progress': 0.9 }), []), RuntimeError);
  assert.deepEqual(parseTransitionPlan(clone(plan), []), first);
});
