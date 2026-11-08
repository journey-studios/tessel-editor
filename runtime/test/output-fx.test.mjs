// Pure checks of the output-scope effects of a directed-scene plan (F016, no GL, no browser):
// what a `fx` pass must be to be drawn and which refusals are typed. The pixels are proved with a
// real browser in directed-scene-fx-browser.test.mjs and, against the core's reference, in
// scripts/fx-proof/.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import { SCENE_CAPABILITIES, parseDirectedLayer } from '../src/directed-scene.js';
import { OUTPUT_FX_EFFECTS, OutputFxPipeline, parseOutputFx } from '../src/output-fx.js';
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

test('a name of the prototype chain is never an effect or a parameter (typed refusals, not a TypeError)', () => {
  for (const name of ['constructor', 'toString', '__proto__', 'hasOwnProperty', 'valueOf']) {
    const error = refusal(() => parseOutputFx([pass({ effect: name })], ['outputFx']));
    assert.equal(error.code, 'unsupported', name);
    assert.equal(error.details.unsupported, 'fx_effect', name);
    assert.equal(error.details.effect, name, name);
    // As a parameter the name is not part of the catalog either (`'toString' in params` was true).
    const extra = refusal(() => parseOutputFx([pass({}, { [name]: 1 })], ['outputFx']));
    assert.equal(extra.code, 'invalid_plan', name);
    assert.equal(extra.details.pointer, `/fx/0/params/${name}`, name);
  }
  // `effect` that is not even text is unsupported too, and is not echoed back as an object.
  for (const effect of [7, null, undefined, { name: 'vignette' }, ['vignette']]) {
    const error = refusal(() => parseOutputFx([pass({ effect })], ['outputFx']));
    assert.equal(error.code, 'unsupported');
    assert.equal(error.details.effect, null);
  }
  // A params object that carries `__proto__` as an own key (JSON.parse makes one) is an extra parameter, not an inheritance.
  const entry = pass();
  entry.params = JSON.parse(`{"__proto__": 1, ${JSON.stringify(entry.params).slice(1)}`);
  assert.ok(Object.hasOwn(entry.params, '__proto__'));
  assert.equal(refusal(() => parseOutputFx([entry], ['outputFx'])).details.pointer, '/fx/0/params/__proto__');
});

// What the core's catalog says (`directed_scene::fx::catalog`), as a fixture the Rust side compares
// to its own catalog (crates/core/tests/scene_fx.rs): both ends of the contract are pinned by the
// same file, so neither can change a range, a kind or the parameter order alone.
const RUST_KINDS = { number: 'number', color: 'color', vec2: 'point' };

test('the ranges, kinds and order of the runtime match the core catalog (pinned by a shared fixture)', () => {
  const contract = JSON.parse(readFileSync(new URL('./fixtures/fx-catalog.json', import.meta.url), 'utf8'));
  assert.deepEqual(Object.keys(OUTPUT_FX_EFFECTS), contract.effects.map((effect) => effect.id), 'effects, in catalog order');
  for (const effect of contract.effects) {
    const known = OUTPUT_FX_EFFECTS[effect.id];
    assert.deepEqual(Object.keys(known.params), effect.params.map((param) => param.id), `${effect.id} parameter order`);
    for (const param of effect.params) {
      const mine = known.params[param.id];
      assert.equal(mine.kind, RUST_KINDS[param.kind], `${effect.id}.${param.id} kind`);
      if (mine.kind === 'color') {
        assert.deepEqual([param.min, param.max], [0, 1], `${effect.id}.${param.id} channels`);
        assert.equal(mine.min, undefined);
      } else {
        assert.deepEqual([mine.min, mine.max], [param.min, param.max], `${effect.id}.${param.id} range`);
      }
    }
  }
});

// ---- The pipeline, with a renderer that records what it is asked to do (no GL) ---------------------

const fakeTarget = (name) => ({ name, texture: { name: `${name}.texture` } });

const fakeRenderer = ({ failOnRender } = {}) => {
  const log = [];
  let current = null;
  return {
    log,
    setRenderTarget: (target) => {
      current = target;
      log.push(['target', target?.name ?? null]);
    },
    render: (scene) => {
      const { material } = scene.children[0];
      const { uniforms } = material;
      log.push({
        into: current?.name,
        from: uniforms.tMap.value?.name,
        amount: uniforms.uAmount.value,
        radius: uniforms.uRadius.value,
        material: material.uuid,
      });
      if (log.filter((entry) => !Array.isArray(entry)).length === failOnRender) throw new Error('boom');
    },
  };
};

const sizeOf = { width: 160, height: 90 };
const parsed = (...amounts) => parseOutputFx(amounts.map((amount, index) => pass({ clipId: `c${index}`, amount }, { radius: 0.1 * (index + 1) })), ['outputFx']);
const targetsOf = () => {
  const targets = [fakeTarget('a'), fakeTarget('b')];
  const asked = [];
  return { asked, targetOf: (slot) => (asked.push(slot), targets[slot]) };
};

test('passes ping-pong between two targets: each reads the one before and none reads what it writes', () => {
  const pipeline = new OutputFxPipeline();
  const renderer = fakeRenderer();
  const { targetOf, asked } = targetsOf();
  const result = pipeline.draw(renderer, fakeTarget('src'), parsed(0.1, 0.2, 0.3, 0.4, 0.5), { targetOf, size: sizeOf });
  const draws = renderer.log.filter((entry) => !Array.isArray(entry));
  assert.deepEqual(draws.map((draw) => [draw.from, draw.into]), [
    ['src.texture', 'a'],
    ['a.texture', 'b'],
    ['b.texture', 'a'],
    ['a.texture', 'b'],
    ['b.texture', 'a'],
  ]);
  assert.ok(draws.every((draw) => draw.from !== `${draw.into}.texture`), 'never a feedback loop');
  assert.deepEqual(draws.map((draw) => draw.amount), [0.1, 0.2, 0.3, 0.4, 0.5], 'each pass writes its own uniforms before its draw');
  assert.deepEqual(draws.map((draw) => draw.radius), [0.1, 0.2, 0.3, 0.4, 0.5].map((_, index) => 0.1 * (index + 1)));
  assert.equal(result.name, 'a.texture', 'the last pass wrote slot a');
  assert.equal(new Set(asked).size, 2, 'only two slots exist, however many passes');
  assert.deepEqual(renderer.log.at(-1), ['target', null], 'the canvas is the target again');
  pipeline.dispose();
});

test('one pass needs one target, and no pass returns the source texture untouched', () => {
  const pipeline = new OutputFxPipeline();
  const renderer = fakeRenderer();
  const { targetOf, asked } = targetsOf();
  assert.equal(pipeline.draw(renderer, fakeTarget('src'), parsed(0.5), { targetOf, size: sizeOf }).name, 'a.texture');
  assert.deepEqual(asked, [0]);
  const none = targetsOf();
  const source = fakeTarget('src');
  assert.equal(pipeline.draw(renderer, source, [], { targetOf: none.targetOf, size: sizeOf }), source.texture);
  assert.deepEqual(none.asked, []);
  pipeline.dispose();
});

test('the material of an effect is built once and serves every pass of every frame', () => {
  const pipeline = new OutputFxPipeline();
  const renderer = fakeRenderer();
  const seen = [];
  const draw = () => pipeline.draw(renderer, fakeTarget('src'), parsed(0.1, 0.2, 0.3), { targetOf: targetsOf().targetOf, size: sizeOf, onMaterial: (created) => seen.push(created) });
  draw();
  draw();
  assert.deepEqual(seen, [true, false, false, false, false, false], 'one build, then reuse, across passes and frames');
  const materials = new Set(renderer.log.filter((entry) => !Array.isArray(entry)).map((entry) => entry.material));
  assert.equal(materials.size, 1);
  assert.equal(pipeline.materials.size, 1);
  assert.equal(pipeline.materials.get('vignette').uniforms.tMap.value, null, 'no texture of a retained target is held between frames');
  pipeline.dispose();
});

test('a pass that fails restores the canvas target and drops the texture it held; a disposed pipeline refuses to draw', () => {
  const pipeline = new OutputFxPipeline();
  const renderer = fakeRenderer({ failOnRender: 2 });
  assert.throws(() => pipeline.draw(renderer, fakeTarget('src'), parsed(0.1, 0.2, 0.3), { targetOf: targetsOf().targetOf, size: sizeOf }), /boom/);
  assert.deepEqual(renderer.log.at(-1), ['target', null]);
  assert.equal(pipeline.materials.get('vignette').uniforms.tMap.value, null);
  // The pipeline itself is sound (nothing is written in it that a pass could leave half done).
  assert.equal(pipeline.draw(renderer, fakeTarget('src'), parsed(0.1), { targetOf: targetsOf().targetOf, size: sizeOf }).name, 'a.texture');
  pipeline.dispose();
  assert.equal(pipeline.materials.size, 0);
  assert.throws(() => pipeline.draw(renderer, fakeTarget('src'), parsed(0.1), { targetOf: targetsOf().targetOf, size: sizeOf }), /disposed/);
});
