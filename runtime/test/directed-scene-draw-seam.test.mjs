// Controls of what the draw of a directed scene asks of the compositor that owns it (P4b S2-R1a). They
// are pinned on the behaviour that existed before `prepareDirectedScene` and `sceneMaterial` moved to
// directed-deck-draw.js (each passes on the base `7d88545` as well), through the real runtime in headless
// Chrome, and they look at the seam the draw is made of: which target keys and sizes it asks for, in
// which order it builds and refuses, which fields its telemetry writes, and who owns what it builds until
// the frame ends. They do not read pixels (the other directed-scene tests do) and they do not know the
// helper: the compositor's own methods are wrapped, so they hold for whoever draws behind them.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { directedLayer, directedRuntime, flat, makeFiles, node, reference, resource, sceneAsset, scenePlan, SIZE } from './directed-scene-fixtures.mjs';
import { plan as framePlan, skipUnavailable } from './support.mjs';

const files = (t) => makeFiles(t, { wall: flat(16, 9, [200, 160, 80]), other: flat(16, 9, [10, 90, 200]) });

const vignette = (id, amount, scope) => ({
  trackId: 't',
  clipId: id,
  effect: 'vignette',
  frameInClip: 0,
  weight: 1,
  amount,
  params: { radius: 0.75, softness: 0.45, intensity: amount, color: { r: 0, g: 0, b: 0 }, roundness: 0.6, center: { x: 0.5, y: 0.5 } },
  scope,
});

/** A wall that covers the frame; `fx` are the passes the core resolved for it (output and/or shot scope). */
const planOf = (fx = []) =>
  scenePlan({
    nodes: [node('wall-node', 'wall', 0, 0, -2, 60, 34)],
    layers: [['wall-node', 'wall', 'opaque']],
    resources: [resource('wall', 16, 9)],
    requires: ['camera', 'nodeTransforms', 'imagePlanes', 'opaqueAlpha', ...(fx.length > 0 ? ['outputFx', 'fxShotScope'] : [])],
    extra: fx.length > 0 ? { fx, frame: { shotId: 's1' } } : {},
  });

const layerOf = (plan = planOf()) => directedLayer(plan, [reference('wall', 'slot-wall')]);

const ready = async (t, hello) => {
  const paths = files(t);
  const runtime = await directedRuntime(paths, hello);
  const loaded = await runtime.load(sceneAsset('slot-wall', paths.wall, 'wall'));
  assert.equal(loaded.state, 'ready', JSON.stringify(loaded.error));
  // The compositor exists once a frame was drawn; the wrappers go on it and stay.
  const warm = await runtime.frame([layerOf()], { size: SIZE });
  assert.equal(warm.type, 'frame.done', JSON.stringify(warm.error));
  await runtime.page.evaluate(() => {
    const host = window.TesselRuntime.debug.compositor();
    const log = [];
    const wrapped = new WeakSet();
    window.__seam = { log, gate: null, reached: false };
    const watch = (name, describe) => {
      const original = host[name].bind(host);
      host[name] = (...args) => {
        log.push(describe(...args));
        const result = original(...args);
        // A material is the frame's to dispose unless the compositor retains it: record the disposals.
        if (name === 'sceneMaterial' && !wrapped.has(result)) {
          wrapped.add(result);
          const dispose = result.dispose.bind(result);
          result.dispose = () => {
            log.push(['material.dispose']);
            return dispose();
          };
        }
        return result;
      };
    };
    watch('sceneTarget', (slot, width, height, keep, ...rest) => ['sceneTarget', slot, width, height, typeof keep, ...rest]);
    watch('sceneMaterial', (order, alpha, index, texture, keep) => ['sceneMaterial', order, alpha, index, typeof texture, typeof keep]);
    watch('fxPipeline', (keep) => ['fxPipeline', typeof keep]);
    watch('splatStage', () => ['splatStage']);
    watch('discardFailedStage', () => ['discardFailedStage']);
    watch('dropFxPipeline', () => ['dropFxPipeline']);
  });
  return { paths, runtime };
};

const take = (runtime) => runtime.page.evaluate(() => window.__seam.log.splice(0));
const counters = (runtime) => runtime.retention().then((state) => state.counters);
const diff = (after, before) => Object.fromEntries(Object.entries(after).map(([key, value]) => [key, value - (before[key] ?? 0)]).filter(([, value]) => value !== 0));

test('a plan draws into the slots `ds:<order>` (final MSAA, with depth) and its passes into `dsfx:<order>:<slot>`', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const { runtime } = await ready(t, { retain: true });
  try {
    const shot = vignette('s', 0.5, { kind: 'shot', shotId: 's1' });
    const output = vignette('o', 0.5, { kind: 'output' });
    const one = await runtime.frame([layerOf(planOf([shot, output]))], { size: SIZE });
    assert.equal(one.type, 'frame.done', JSON.stringify(one.error));
    assert.deepEqual(await take(runtime), [
      ['sceneTarget', 'ds:0', 160, 90, 'function'],
      ['sceneMaterial', 0, 'opaque', 0, 'object', 'function'],
      ['fxPipeline', 'function'],
      ['sceneTarget', 'dsfx:0:0', 160, 90, 'function', 0, false],
      ['sceneTarget', 'dsfx:0:1', 160, 90, 'function', 0, false],
    ]);

    // Two directed layers of one frame are two slots, each with its own materials, and both are counted.
    const two = await runtime.frame([layerOf(), layerOf()], { size: SIZE });
    assert.equal(two.type, 'frame.done', JSON.stringify(two.error));
    assert.deepEqual(await take(runtime), [
      ['sceneTarget', 'ds:0', 160, 90, 'function'],
      ['sceneMaterial', 0, 'opaque', 0, 'object', 'function'],
      ['sceneTarget', 'ds:1', 160, 90, 'function'],
      ['sceneMaterial', 1, 'opaque', 0, 'object', 'function'],
    ]);
    assert.equal(two.source.directed_scene_layers, 2);

    // The size is the contain fit of the plan's output in the canvas, not the canvas.
    const wide = planOf();
    wide.output = { ...wide.output, width: 32, height: 9 };
    const fitted = await runtime.frame([layerOf(wide)], { size: SIZE });
    assert.equal(fitted.type, 'frame.done', JSON.stringify(fitted.error));
    assert.deepEqual((await take(runtime))[0], ['sceneTarget', 'ds:0', 160, 45, 'function']);
  } finally {
    await runtime.close();
  }
});

test('the telemetry of a plan names only what the frame drew', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const { runtime } = await ready(t, { retain: true });
  try {
    const plain = await runtime.frame([layerOf()], { size: SIZE });
    assert.equal(plain.source.directed_scene_layers, 1);
    for (const field of ['directed_scene_splats', 'directed_scene_fx', 'directed_scene_fx_shot']) {
      assert.equal(field in plain.source, false, `${field} is absent from a plan without it`);
    }
    const output = await runtime.frame([layerOf(planOf([vignette('o', 0.5, { kind: 'output' })]))], { size: SIZE });
    assert.equal(output.source.directed_scene_fx, 1);
    for (const field of ['directed_scene_splats', 'directed_scene_fx_shot']) {
      assert.equal(field in output.source, false, `${field} is absent from a plan without it`);
    }
  } finally {
    await runtime.close();
  }
});

test('retained materials are named `ds:<order>:<alpha>:<index>`, reused and counted; without retention the frame owns them', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const retained = await ready(t, { retain: true });
  try {
    const { runtime } = retained;
    const before = await counters(runtime);
    await runtime.frame([layerOf()], { size: SIZE });
    const created = diff(await counters(runtime), before);
    // The material of the plane (ds: key) and the one that composites the layer: both made by the warm-up frame.
    assert.equal(created.material_reused, 2, 'every material of this frame was kept by the one before');
    assert.equal(created.material_created, undefined);
    assert.ok((await take(runtime)).every(([name]) => name !== 'material.dispose'), 'a retained material is not disposed with the frame');
    const keys = await runtime.page.evaluate(() => [...window.TesselRuntime.debug.compositor().materials.entries.keys()]);
    assert.ok(keys.includes('ds:0:opaque:0'), `kept under its key: ${keys}`);
  } finally {
    await retained.runtime.close();
  }
  const fresh = await ready(t, { retain: false });
  try {
    const { runtime } = fresh;
    const before = await counters(runtime);
    await runtime.frame([layerOf()], { size: SIZE });
    const log = await take(runtime);
    assert.equal(log.filter(([name]) => name === 'sceneMaterial').length, 1);
    assert.equal(log.filter(([name]) => name === 'material.dispose').length, 1, 'the frame disposes what it built');
    assert.deepEqual(diff(await counters(runtime), before).material_reused, undefined);
    assert.equal((await runtime.page.evaluate(() => window.TesselRuntime.debug.compositor().materials.size)), 0);
  } finally {
    await fresh.runtime.close();
  }
});

test('a frame superseded before its plan is built asks the compositor for nothing and answers cancelled', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const { runtime } = await ready(t, { retain: true });
  try {
    await take(runtime);
    await runtime.page.evaluate((id) => {
      const host = window.TesselRuntime.debug.compositor();
      const original = host.prepareDirectedScene.bind(host);
      host.prepareDirectedScene = (layer, frame) => {
        host.prepareDirectedScene = original;
        window.TesselRuntime.receive({ v: 1, type: 'frame.cancel', id });
        return original(layer, frame);
      };
    }, 920);
    const failed = await runtime.frame([layerOf()], { size: SIZE, id: 920 });
    assert.equal(failed.type, 'frame.failed');
    assert.equal(failed.error.code, 'cancelled');
    assert.equal(failed.error.message, 'The frame was superseded');
    assert.deepEqual(await take(runtime), [], 'no target, material or pass was asked for');
    const after = await runtime.frame([layerOf()], { size: SIZE });
    assert.equal(after.type, 'frame.done', JSON.stringify(after.error));
  } finally {
    await runtime.close();
  }
});

test('a slot replaced after the plan was prepared cancels the frame, and the frame still disposes what it built', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const { paths, runtime } = await ready(t, { retain: false });
  try {
    await take(runtime);
    await runtime.page.evaluate(() => {
      const host = window.TesselRuntime.debug.compositor();
      const original = host.prepareDirectedScene.bind(host);
      let release;
      window.__seam.gate = new Promise((resolve) => {
        release = resolve;
      });
      window.__seam.open = release;
      host.prepareDirectedScene = async (layer, frame) => {
        host.prepareDirectedScene = original;
        const prepared = await original(layer, frame);
        window.__seam.reached = true;
        await window.__seam.gate;
        return prepared;
      };
    });
    await runtime.send({
      type: 'frame.render',
      id: 930,
      revision: 930,
      plan: framePlan([layerOf()], SIZE),
      output: { width: 160, height: 90 },
    });
    for (let attempt = 0; attempt < 200 && !(await runtime.page.evaluate(() => window.__seam.reached)); attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
    assert.equal(await runtime.page.evaluate(() => window.__seam.reached), true, 'the frame reached the end of the preparation');
    const replaced = await runtime.load(sceneAsset('slot-wall', paths.other, 'wall', { revision_id: 'wall-r2' }));
    assert.equal(replaced.state, 'ready', JSON.stringify(replaced.error));
    await runtime.page.evaluate(() => window.__seam.open());
    const failed = await runtime.waitFor((m) => m.id === 930 && (m.type === 'frame.failed' || m.type === 'frame.done'), 'frame 930');
    assert.equal(failed.type, 'frame.failed');
    assert.equal(failed.error.code, 'cancelled');
    assert.match(failed.error.message, /replaced while the frame was drawn/);
    assert.equal(failed.error.details.asset, 'slot-wall');
    const log = await take(runtime);
    assert.equal(log.filter(([name]) => name === 'sceneMaterial').length, 1);
    assert.equal(log.filter(([name]) => name === 'material.dispose').length, 1, 'a failed frame disposes what it built too');
  } finally {
    await runtime.close();
  }
});
