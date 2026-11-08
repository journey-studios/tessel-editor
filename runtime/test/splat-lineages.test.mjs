// BL130: how many Spark stages a moving playhead may hold, and who keeps which mesh. Real headless Chrome
// (SwiftShader) through the bridge messages the desktop uses; skipped, loudly, without Chrome or the bundle
// (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
//
//   * the stages (SparkRenderers, each with its sorted splat data on the GPU) alive are bounded by the budget
//     (`splatStages`) + the one shared stage, however many splat scene layers a frame has; the layers past the
//     budget are drawn exact by the shared stage (never stale), and the layers that already have a stage keep it;
//   * a mesh of a shared splat file is not taken from the scene that will ask for it: not by a directed scene
//     that comes first in the plan, not by a scene entering a crossfade.
import assert from 'node:assert/strict';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';

import { spareFor } from '../src/assets.js';
import { directedLayer, reference, resource, sceneAsset as directedAsset, scenePlan, translation } from './directed-scene-fixtures.mjs';
import { PREVIEW, SAME_MSAA, at, draw, kit, open, reference as exactHash, withRuntime } from './splat-kit.mjs';
import { SPLAT_ENCODERS, skipUnavailable } from './support.mjs';

/** Layers of `names` at one camera, each with an equal share of the picture. */
const stack = (names, x) => names.map((name) => ({ ...at(`scene-${name}`, x), opacity: 1 / names.length }));

/** SparkRenderers alive in the page. */
const alive = (runtime) => runtime.page.evaluate(() => window.TesselRuntime.debug.splatStages().length);

// ---- who gets which spare mesh (pure) -------------------------------------------------------------------------

test('a lease takes its own spare, then one nobody owns, then one whose owner is not asking, and otherwise nothing', () => {
  const spare = (leaseOwner) => ({ leaseOwner });
  const reserved = new Set(['a#1', 'b#1']);
  // Its own comes first, wherever it is in the pool.
  assert.equal(spareFor([spare('a#1'), spare(null), spare('b#1')], 'a#1', reserved), 0);
  // A new owner, or none (a directed scene), takes what nobody owns before what belongs to someone else.
  assert.equal(spareFor([spare(null), spare('c#1'), spare('a#1')], 'z#1', reserved), 0);
  assert.equal(spareFor([spare('c#1'), spare(undefined), spare('a#1')], null, reserved), 1, 'never owned = nobody');
  // A spare of an owner the frame will ask for is kept for it, whoever asks first...
  assert.equal(spareFor([spare('a#1'), spare('b#1')], 'z#1', reserved), -1);
  assert.equal(spareFor([spare('a#1')], null, reserved), -1);
  // ...and one whose owner is not in this frame is free to take.
  assert.equal(spareFor([spare('a#1'), spare('gone#1')], 'z#1', reserved), 1);
  assert.equal(spareFor([], 'a#1', reserved), -1);
});

// ---- the budget of stages ---------------------------------------------------------------------------------------

test('three scenes: two keep a stage and reuse their order, the third is drawn exact by the shared one', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b', 'c']);
  const exact = await exactHash(files, stack(['a', 'b', 'c'], 2));
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    const first = await draw(runtime, stack(['a', 'b', 'c'], -2), PREVIEW);
    assert.equal(first.source.stale_sort_layers, 0, 'no scene had an order yet');
    assert.equal(first.cache.stage_created, 2, 'the budget, not one per layer');
    assert.equal(first.cache.stage_shared_layers, 1, 'the third scene has no stage of its own');
    assert.deepEqual([first.cache.stage_entries, first.cache.shared_stage_entries], [2, 1]);
    for (const x of [-1.2, -0.4, 0.4, 1.2]) {
      const moving = await draw(runtime, stack(['a', 'b', 'c'], x), PREVIEW);
      assert.equal(moving.source.stale_sort_layers, 2, `camera ${x}: two scenes reuse their order and the third is exact, not three stale`);
      assert.equal(moving.cache.stage_created, 0, 'nothing is built again');
      assert.equal(moving.cache.stage_evicted, 0);
      assert.equal(moving.cache.stage_shared_layers, 1);
      assert.deepEqual([moving.cache.stage_entries, moving.cache.shared_stage_entries], [2, 1]);
    }
    assert.equal(await alive(runtime), 3, 'budget (2) + the shared stage: three SparkRenderers for three scenes');
    const final = await draw(runtime, stack(['a', 'b', 'c'], 2));
    assert.equal(final.source.stale_sort_layers, 0);
    assert.equal(final.hash, exact, 'the exact frame (explicit, export, refine) is the one a runtime that never previewed draws');
    // And a preview frame after that is again two stale layers, still three renderers.
    assert.equal((await draw(runtime, stack(['a', 'b', 'c'], 1.5), PREVIEW)).source.stale_sort_layers, 2);
    assert.equal(await alive(runtime), 3);
  });
});

test('four scenes and repeats: the budget holds, stages stay with the layers that have them, eviction makes room first', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b', 'c', 'd']);
  const exact = await exactHash(files, stack(['a', 'b', 'c', 'd'], 2));
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    const entries = (done) => [done.cache.stage_entries, done.cache.shared_stage_entries];
    const first = await draw(runtime, stack(['a', 'b', 'c', 'd'], -2), PREVIEW);
    assert.equal(first.source.stale_sort_layers, 0);
    assert.deepEqual([first.cache.stage_created, first.cache.stage_shared_layers], [2, 2]);
    const second = await draw(runtime, stack(['a', 'b', 'c', 'd'], -1), PREVIEW);
    assert.equal(second.source.stale_sort_layers, 2, 'a and b reuse their orders; c and d are exact');
    assert.deepEqual(entries(second), [2, 1]);

    // c and d come first in the plan but a and b hold the stages: they keep them (no stage is built or freed).
    const reordered = await draw(runtime, stack(['c', 'd', 'a', 'b'], 0), PREVIEW);
    assert.equal(reordered.source.stale_sort_layers, 2, 'the layers that had a stage keep it, wherever the plan lists them');
    assert.deepEqual([reordered.cache.stage_created, reordered.cache.stage_evicted], [0, 0]);

    // Only c and d: both are new to the pool, which makes room by freeing the idle ones first (never three alive).
    const swapped = await draw(runtime, stack(['c', 'd'], 0.5), PREVIEW);
    assert.equal(swapped.source.stale_sort_layers, 0, 'new stages sort for their own camera first');
    assert.deepEqual([swapped.cache.stage_created, swapped.cache.stage_evicted], [2, 2]);
    assert.deepEqual(entries(swapped), [2, 1]);
    assert.equal(await alive(runtime), 3);
    assert.equal((await draw(runtime, stack(['c', 'd'], 1), PREVIEW)).source.stale_sort_layers, 2);

    // All four again: c and d keep their stages, a and b (evicted) are the ones drawn exact now.
    const back = await draw(runtime, stack(['a', 'b', 'c', 'd'], 1.5), PREVIEW);
    assert.equal(back.source.stale_sort_layers, 2);
    assert.deepEqual([back.cache.stage_created, back.cache.stage_evicted, back.cache.stage_shared_layers], [0, 0, 2]);
    assert.equal(await alive(runtime), 3);

    const final = await draw(runtime, stack(['a', 'b', 'c', 'd'], 2));
    assert.equal(final.source.stale_sort_layers, 0);
    assert.equal(final.hash, exact, 'the exact frame of four scenes is the one a runtime that never previewed draws');
  });
});

test('three layers of one scene are three lineages against the same budget', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a']);
  const exact = await exactHash(files, stack(['a', 'a', 'a'], 2));
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    const first = await draw(runtime, stack(['a', 'a', 'a'], -2), PREVIEW);
    assert.deepEqual([first.cache.stage_created, first.cache.stage_shared_layers], [2, 1]);
    const moving = await draw(runtime, stack(['a', 'a', 'a'], 0), PREVIEW);
    assert.equal(moving.source.stale_sort_layers, 2);
    assert.equal(await alive(runtime), 3);
    assert.equal((await draw(runtime, stack(['a', 'a', 'a'], 2))).hash, exact);
  });
});

test('without retention there is no lineage to keep: every splat scene is drawn exact by the one stage', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b', 'c']);
  const exact = await exactHash(files, stack(['a', 'b', 'c'], 2), { ...SAME_MSAA, retain: false });
  await withRuntime(files, { ...SAME_MSAA, retain: false }, async (runtime) => {
    await draw(runtime, stack(['a', 'b', 'c'], -2), PREVIEW);
    const moving = await draw(runtime, stack(['a', 'b', 'c'], 2), PREVIEW);
    assert.equal(moving.source.stale_sort_layers, 0);
    assert.equal(moving.cache.stage_entries, 0);
    assert.equal(await alive(runtime), 1);
    assert.equal(moving.hash, exact);
  });
});

// ---- who keeps which mesh ---------------------------------------------------------------------------------------

const blob = (position, color, scale = 0.3) => ({ position, scale: [scale, scale, scale], color, alpha: 200 });

/** A directed scene with one splat node reading the effect file; it asks the store for a mesh without an owner. */
const directedFx = () => {
  const node_ = {
    id: 'fx-node',
    kind: 'splat',
    parent: null,
    assetId: 'fx',
    widthSceneUnits: null,
    heightSceneUnits: null,
    worldMatrix: translation(0, 0, 0),
    localMatrix: translation(0, 0, 0),
    renderOrder: 0,
  };
  const plan = scenePlan({
    nodes: [node_],
    layers: [],
    resources: [resource('fx', 1, 1, { format: 'spz' })],
    requires: ['camera', 'nodeTransforms', 'splats'],
    extra: { splats: [{ nodeId: 'fx-node', assetId: 'fx', afterLayers: 0 }] },
  });
  return directedLayer(plan, [reference('fx', 'splat-fx')]);
};

/** The files of the kit with the effect cloud as an `spz`, which a directed scene can read, loaded with its identity. */
const openWithDirectedEffect = async (names) => {
  const files = kit(names);
  const path = join(files.directory, 'cloud-fx.spz');
  writeFileSync(path, SPLAT_ENCODERS.spz([blob([0, 0, 0], [255, 220, 0]), blob([0.5, 0.2, 0.1], [255, 90, 0])]));
  files.clouds.fx = path;
  const runtime = await open(files, SAME_MSAA);
  const loaded = await runtime.load({ ...directedAsset('splat-fx', path, 'fx'), kind: 'splat', meta: { format: 'spz' } });
  assert.equal(loaded.state, 'ready', JSON.stringify(loaded.error));
  return { runtime, files };
};

test('a directed scene that comes first in the plan does not take the mesh a scene reads the same splat through', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const { runtime } = await openWithDirectedEffect(['a']);
  try {
    const layers = (x) => [directedFx(), at('scene-a', x)];
    const first = await draw(runtime, layers(-2), PREVIEW);
    assert.equal(first.source.directed_scene_layers, 1);
    assert.equal(first.source.stale_sort_layers, 0, 'nothing to reuse yet');
    for (const x of [-1.2, -0.4, 0.4, 1.2]) {
      const moving = await draw(runtime, layers(x), PREVIEW);
      assert.equal(moving.source.directed_scene_layers, 1);
      // The directed scene asks first and has no owner: it must not take the effect mesh that scene `a` drew
      // with last frame (then `a` would get the other one, its set of splats would change every frame and no
      // order could be reused).
      assert.equal(moving.source.stale_sort_layers, 1, `camera ${x}: the scene keeps the order of its own meshes`);
      assert.equal(moving.cache.splat_mesh_created, 0, 'no generator is built again either');
    }
  } finally {
    await runtime.close();
  }
});

test('a scene entering a crossfade ahead of the one on screen does not take that scene\'s meshes', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b']);
  const exact = await exactHash(files, [at('scene-b', 2), at('scene-a', 2)]);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await draw(runtime, [at('scene-a', -2)], PREVIEW);
    assert.equal((await draw(runtime, [at('scene-a', -1)], PREVIEW)).source.stale_sort_layers, 1);
    // `b` is new and comes first in the plan: it asks for the effect mesh before `a` does, and `a` owns the only spare.
    const entering = await draw(runtime, [{ ...at('scene-b', -0.5), opacity: 0.4 }, { ...at('scene-a', -0.5), opacity: 0.6 }], PREVIEW);
    assert.equal(entering.source.stale_sort_layers, 1, '`a` keeps its meshes, so its order; `b` has nothing yet');
    for (const x of [0, 0.5, 1]) {
      const moving = await draw(runtime, [{ ...at('scene-b', x), opacity: 0.4 }, { ...at('scene-a', x), opacity: 0.6 }], PREVIEW);
      assert.equal(moving.source.stale_sort_layers, 2, `camera ${x}: both scenes reuse their orders`);
      assert.equal(moving.cache.splat_mesh_created, 0);
    }
    const final = await draw(runtime, [at('scene-b', 2), at('scene-a', 2)]);
    assert.equal(final.source.stale_sort_layers, 0);
    assert.equal(final.hash, exact, 'meshes lent by owner never change a pixel of the exact frame');
  });
});
