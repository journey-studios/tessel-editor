// BL157: `frame.warm` gives the splat scenes of a plan their Spark stage and first sort before a frame draws them, so the
// frame at a cut finds a stage that holds the scene instead of waiting for its own sort. Real headless Chrome (SwiftShader)
// through the bridge messages the desktop uses; skipped, loudly, without Chrome or the bundle
// (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { PREVIEW, SAME_MSAA, at, draw, kit, open, reference as exactHash, withRuntime } from './splat-kit.mjs';
import { plan, skipUnavailable } from './support.mjs';

const SIZE = [320, 180];

/** Sends `frame.warm` for `layers` and waits until the sorts it started have finished. */
const warm = async (runtime, layers) => {
  await runtime.send({ type: 'frame.warm', plan: plan(layers, SIZE) });
  await runtime.page.evaluate(() => window.TesselRuntime.debug.idle());
  for (let attempt = 0; attempt < 400; attempt += 1) {
    if ((await runtime.page.evaluate(() => window.TesselRuntime.debug.warming())) === 0) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  assert.fail('the warm sort never finished');
};

const stages = (runtime) => runtime.page.evaluate(() => window.TesselRuntime.debug.splatStages().length);

test('the runtime announces the warm, and a warmed scene is drawn by its stage from the first frame', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b']);
  const exact = await exactHash(files, [at('scene-a', 2)]);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    const ready = await runtime.hello(SAME_MSAA);
    assert.ok(ready.capabilities.features.includes('scene.warm'));

    await warm(runtime, [at('scene-a', -2)]);
    assert.equal(await stages(runtime), 1, 'the stage exists before any frame');

    const first = await draw(runtime, [at('scene-a', -1)], PREVIEW);
    assert.equal(first.cache.stage_created, 0, 'the frame builds no stage');
    assert.equal(first.cache.splat_mesh_created, 0, 'and creates no mesh: the warm lent them back for the same owner');
    assert.equal(first.source.stale_sort_layers, 1, 'it is drawn at once with the order the warm sorted, not after its own sort');

    // The exact frame (explicit, export, refine) is the picture of a runtime that never warmed anything.
    const final = await draw(runtime, [at('scene-a', 2)]);
    assert.equal(final.source.stale_sort_layers, 0);
    assert.equal(final.hash, exact);
  });
});

test('a scene that was not warmed still waits for its own sort, and the one that was does not', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b']);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await warm(runtime, [at('scene-a', -2)]);
    const mixed = await draw(
      runtime,
      [at('scene-a', -1), at('scene-b', -1)].map((layer) => ({ ...layer, opacity: 0.5 })),
      PREVIEW,
    );
    assert.equal(mixed.cache.stage_created, 1, 'only the cold scene makes a stage');
    assert.equal(mixed.source.stale_sort_layers, 1, 'the warm one reuses its order, the cold one is exact');
  });
});

test('a warm never frees a stage for itself, never starts a load and repeating it does nothing', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b', 'c']);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    await warm(runtime, [at('scene-a', 0), at('scene-b', 0), at('scene-c', 0)]);
    assert.equal(await stages(runtime), 2, 'the budget (2): the third scene is left for its own frame');
    await warm(runtime, [at('scene-a', 0), at('scene-b', 0), at('scene-c', 0)]);
    assert.equal(await stages(runtime), 2, 'asking again changes nothing');

    // A scene whose asset is not loaded is skipped without a message or a load.
    const empty = await open(kit(['a']));
    try {
      await empty.release('splat-a');
      await warm(empty, [at('scene-a', 0)]);
      assert.equal(await stages(empty), 0);
      assert.equal(empty.messages.filter((message) => message.type === 'error').length, 0);
    } finally {
      await empty.close();
    }
  });
});

test('a warm takes the slot of a stage the last frame did not draw, and never of one it did', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit(['a', 'b', 'c']);
  await withRuntime(files, SAME_MSAA, async (runtime) => {
    // Scene a ended, scene b is on screen: the cut to c finds a's idle stage in the way.
    await draw(runtime, [at('scene-a', 0)], PREVIEW);
    await draw(runtime, [at('scene-b', 0)], PREVIEW);
    assert.equal(await stages(runtime), 2);
    await warm(runtime, [at('scene-c', -1)]);
    assert.equal(await stages(runtime), 2, 'a was freed for c');
    const cut = await draw(runtime, [at('scene-b', 1), at('scene-c', 1)].map((layer) => ({ ...layer, opacity: 0.5 })), PREVIEW);
    assert.equal(cut.cache.stage_created, 0, 'b kept its stage and c arrives warm');
    assert.ok(cut.source.stale_sort_layers >= 1, 'c is drawn at once with the order the warm sorted');

    // Both stages drawn by the last frame: nothing is freed, the scene is left for its own frame.
    await warm(runtime, [at('scene-a', -1)]);
    const again = await draw(runtime, [at('scene-b', 2), at('scene-c', 2)].map((layer) => ({ ...layer, opacity: 0.5 })), PREVIEW);
    assert.equal(again.cache.stage_created, 0, 'the stages on screen were not touched');
  });
});
