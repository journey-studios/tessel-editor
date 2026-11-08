// Pure checks of the output-scope effects of a directed-scene plan (F016, no GL, no browser):
// what a `fx` pass must be to be drawn and which refusals are typed. The pixels are proved with a
// real browser in directed-scene-fx-browser.test.mjs and, against the core's reference, in
// scripts/fx-proof/.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { SCENE_CAPABILITIES, parseDirectedLayer } from '../src/directed-scene.js';
import { OUTPUT_FX_EFFECTS, parseOutputFx } from '../src/output-fx.js';
import { RuntimeError } from '../src/protocol.js';
import { directedLayer, node, reference, resource, scenePlan } from './directed-scene-fixtures.mjs';

const pass = (overrides = {}, params = {}) => ({
  trackId: 'fx-track-1',
  clipId: 'v1',
  effect: 'vignette',
  frameInClip: 3,
  weight: 1,
  amount: 0.5,
  params: {
    radius: 0.75,
    softness: 0.45,
    intensity: 0.5,
    color: { r: 0, g: 0, b: 0 },
    roundness: 0.6,
    center: { x: 0.5, y: 0.5 },
    ...params,
  },
  scope: { kind: 'output' },
  ...overrides,
});

const plan = (fx, requires) =>
  scenePlan({
    nodes: [node('back', 'wall', 0, 0, -2, 6, 3)],
    layers: [['back', 'wall', 'opaque']],
    resources: [resource('wall', 16, 9)],
    requires: requires ?? ['camera', 'nodeTransforms', 'imagePlanes', 'opaqueAlpha', 'outputFx'],
    extra: fx === undefined ? {} : { fx },
  });

const refusal = (callback) => {
  try {
    callback();
  } catch (error) {
    assert.ok(error instanceof RuntimeError, `a typed RuntimeError, got ${error}`);
    return error;
  }
  assert.fail('expected a refusal');
};

const parse = (fx, requires) => parseDirectedLayer(directedLayer(plan(fx, requires), [reference('wall', 'slot-0')]));

test('outputFx is a capability of this runtime and the catalog lists the vignette', () => {
  assert.ok(SCENE_CAPABILITIES.includes('outputFx'));
  assert.deepEqual(Object.keys(OUTPUT_FX_EFFECTS), ['vignette']);
  assert.deepEqual(Object.keys(OUTPUT_FX_EFFECTS.vignette.params), ['radius', 'softness', 'intensity', 'color', 'roundness', 'center']);
});

test('a plan without effects has no pass, and one with effects keeps them in the plan order', () => {
  assert.deepEqual(parse(undefined, ['camera', 'nodeTransforms', 'imagePlanes', 'opaqueAlpha']).fx, []);
  const spec = parse([pass({ clipId: 'a' }), pass({ clipId: 'b', amount: 0.25 }, { radius: 0.2 })]);
  assert.deepEqual(spec.fx.map((entry) => entry.clipId), ['a', 'b']);
  assert.equal(spec.fx[1].amount, 0.25);
  assert.equal(spec.fx[1].params.radius, 0.2);
  assert.deepEqual(spec.fx[0].params.color, [0, 0, 0]);
  assert.deepEqual(spec.fx[0].params.center, [0.5, 0.5]);
});

test('effects without the capability in requires are refused before drawing', () => {
  const error = refusal(() => parse([pass()], ['camera', 'nodeTransforms', 'imagePlanes', 'opaqueAlpha']));
  assert.equal(error.code, 'invalid_plan');
  assert.equal(error.details.pointer, '/requires');
});

test('an effect or scope this runtime does not draw is refused by name, never skipped', () => {
  const effect = refusal(() => parseOutputFx([pass({ effect: 'glitch' })], ['outputFx']));
  assert.equal(effect.code, 'unsupported');
  assert.equal(effect.details.unsupported, 'fx_effect');
  assert.equal(effect.details.effect, 'glitch');
  const scope = refusal(() => parseOutputFx([pass({ scope: { kind: 'shot' } })], ['outputFx']));
  assert.equal(scope.code, 'unsupported');
  assert.equal(scope.details.unsupported, 'fx_scope');
});

test('malformed passes name the pointer instead of drawing something', () => {
  const cases = [
    ['/fx/0/amount', pass({ amount: 1.5 })],
    ['/fx/0/weight', pass({ weight: -0.1 })],
    ['/fx/0/params/radius', pass({}, { radius: 9 })],
    ['/fx/0/params/color/g', pass({}, { color: { r: 0, g: 2, b: 0 } })],
    ['/fx/0/params/center/x', pass({}, { center: { x: 'left', y: 0.5 } })],
    ['/fx/0/params/glow', pass({}, { glow: 1 })],
    ['/fx/0/clipId', pass({ clipId: '' })],
  ];
  for (const [pointer, entry] of cases) {
    const error = refusal(() => parseOutputFx([entry], ['outputFx']));
    assert.equal(error.code, 'invalid_plan', pointer);
    assert.equal(error.details.pointer, pointer);
  }
  const missing = pass();
  delete missing.params.softness;
  assert.equal(refusal(() => parseOutputFx([missing], ['outputFx'])).details.pointer, '/fx/0/params/softness');
  assert.equal(refusal(() => parseOutputFx('vignette', ['outputFx'])).details.pointer, '/fx');
  assert.equal(
    refusal(() => parseOutputFx([pass(), pass()], ['outputFx'])).details.pointer,
    '/fx/1/clipId',
    'a clip appears once per frame',
  );
});
