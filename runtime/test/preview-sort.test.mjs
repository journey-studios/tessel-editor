// BL130: the Spark sort of a moving playhead, proven in a real page (headless Chrome, SwiftShader)
// through the same bridge messages the desktop uses.
//
//   * a `preview` frame (the monitor while the playhead moves) draws a splat scene with the order of the
//     last finished sort instead of waiting for the sort of its own camera, and says so
//     (`source.stale_sort_layers`); the picture it makes is not the exact one;
//   * a frame without `quality` (explicit `render.frame`, export, probes, the refine after a stop) waits for
//     the sort of its camera and its pixels are the ones it always had, whatever came before;
//   * a splat scene with nothing to reuse (the first frame, another scene's data on display) is never drawn
//     from old data: it waits like an exact frame;
//   * the result cache never serves a stale-order picture as the exact one.
//
// MSAA is kept equal for preview and final (`preview_scene_samples: 4`) wherever a test compares pixels, so
// what differs between the two is the order and nothing else; BL129's MSAA has its own tests.
//
// Skipped, loudly, without Chrome or the bundle (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
import assert from 'node:assert/strict';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';

import { PREVIEW, SAME_MSAA, at, draw, kit, open, reference } from './splat-kit.mjs';
import { boxScene, openRuntime, sceneAsset, sceneLayer, skipUnavailable } from './support.mjs';

test('a moving playhead draws with the last sort and says so; the first frame and the final frame are exact', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const exact = await reference(files, [at('scene-a', 2)]);
  const runtime = await open(files, SAME_MSAA);
  try {
    const first = await draw(runtime, [at('scene-a', -2)], PREVIEW);
    assert.equal(first.source.stale_sort_layers, 0, 'there is no earlier order to reuse: the first frame waits for its own sort');
    assert.equal(first.source.reduced_scene_layers, 0, 'MSAA is not reduced here, so the sort is all there is to refine');
    const moving = await draw(runtime, [at('scene-a', 2)], PREVIEW);
    assert.equal(moving.source.quality, 'preview');
    assert.equal(moving.source.stale_sort_layers, 1, 'the camera moved: the picture uses the order of the earlier camera');
    assert.notEqual(moving.hash, exact, 'a stale order is a different picture, not a label');
    // The refine after a stop (and an explicit render, and the export): the exact order, the very pixels
    // a runtime that never saw a preview frame draws.
    const final = await draw(runtime, [at('scene-a', 2)]);
    assert.equal(final.source.stale_sort_layers, 0);
    assert.equal(final.hash, exact);
    // And it stays that way: the next preview frame is stale again, the final one exact again.
    await draw(runtime, [at('scene-a', 1)], PREVIEW);
    assert.equal((await draw(runtime, [at('scene-a', 2)])).hash, exact);
  } finally {
    await runtime.close();
  }
});

test('the final frame is the same picture whatever preview frames came before it', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const exact = await reference(files, [at('scene-a', 1.5)]);
  const runtime = await open(files);
  try {
    // A scrub: previews at many cameras without a pause between them, then the one the playhead stops on.
    for (const x of [-2, -1.2, -0.4, 0.4, 1.2, 2, 1.7]) await draw(runtime, [at('scene-a', x)], { ...PREVIEW, inspect: false });
    const final = await draw(runtime, [at('scene-a', 1.5)]);
    assert.equal(final.hash, exact, 'refine/explicit pixels do not depend on the previews before them');
    assert.equal(final.source.stale_sort_layers, 0);
  } finally {
    await runtime.close();
  }
});

test('a stale picture is never served from the result cache as the exact one', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const exact = await reference(files, [at('scene-a', 2)]);
  const runtime = await open(files, SAME_MSAA);
  try {
    await draw(runtime, [at('scene-a', -2)], PREVIEW);
    const stale = await draw(runtime, [at('scene-a', 2)], PREVIEW);
    assert.equal(stale.source.stale_sort_layers, 1);
    // Same samples, same camera, same size: the key is the same as the final picture's. Only the rule
    // that a stale picture does not become the cached result keeps the final from being served it.
    const final = await draw(runtime, [at('scene-a', 2)]);
    assert.equal(final.cache.result_hit, 0, 'a real, exact draw');
    assert.equal(final.hash, exact);
    const again = await draw(runtime, [at('scene-a', 2)]);
    assert.equal(again.cache.result_hit, 1, 'the exact picture is cached in turn');
    assert.equal(again.hash, exact);
    // A preview frame that finds the exact picture cached uses it: it is not stale.
    const preview = await draw(runtime, [at('scene-a', 2)], PREVIEW);
    assert.equal(preview.cache.result_hit, 1);
    assert.equal(preview.source.stale_sort_layers, 0);
    assert.equal(preview.hash, exact);
  } finally {
    await runtime.close();
  }
});

test('a scene whose splats changed under it is never drawn from the old data', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  // The splat file of `scene-a` is replaced (same id, other content) while the scene keeps its sort
  // lineage: Spark still shows the old splats until a sort of the new set lands. A stale draw here would
  // paint the old file.
  const replaced = { ...files, clouds: { ...files.clouds, a: files.clouds.b } };
  const exact = await reference(replaced, [at('scene-a', 0.5)]);
  const runtime = await open(files, SAME_MSAA);
  try {
    await draw(runtime, [at('scene-a', -1)], PREVIEW);
    assert.equal((await draw(runtime, [at('scene-a', 0)], PREVIEW)).source.stale_sort_layers, 1);
    const reloaded = await runtime.load({ id: 'splat-a', kind: 'splat', source: { type: 'file', path: files.clouds.b }, meta: { format: 'splat' } });
    assert.equal(reloaded.state, 'ready');
    const cut = await draw(runtime, [at('scene-a', 0.5)], PREVIEW);
    assert.equal(cut.source.stale_sort_layers, 0, 'it waited for the sort of its own splats');
    assert.equal(cut.hash, exact, 'and drew them');
    // The next frame has an order to reuse again.
    const next = await draw(runtime, [at('scene-a', 1.5)], PREVIEW);
    assert.equal(next.source.stale_sort_layers, 1);
  } finally {
    await runtime.close();
  }
});

test('a crossfade reuses the order of each of its two scenes, though they share an effect file', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const fade = (x) => [
    { ...at('scene-a', x), opacity: 0.6 },
    { ...at('scene-b', x), opacity: 0.4 },
  ];
  const exact = await reference(files, fade(2));
  const runtime = await open(files, SAME_MSAA);
  try {
    const first = await draw(runtime, fade(-2), PREVIEW);
    assert.equal(first.source.stale_sort_layers, 0, 'neither scene had an order yet');
    // Every frame after that: both layers draw at once with their own last order. This is what the
    // frame of a crossfade costs the playhead: no wait for either sort.
    for (const x of [-1.2, -0.4, 0.4, 1.2]) {
      const moving = await draw(runtime, fade(x), PREVIEW);
      assert.equal(moving.source.stale_sort_layers, 2, `both scenes at ${x}`);
    }
    const final = await draw(runtime, fade(2));
    assert.equal(final.source.stale_sort_layers, 0);
    assert.equal(final.hash, exact, 'the exact crossfade frame is the one a runtime that never previewed draws');
    // After the fade the incoming scene is the only one; it keeps its own order.
    const single = await draw(runtime, [at('scene-b', 2.2)], PREVIEW);
    assert.equal(single.source.stale_sort_layers, 1);
  } finally {
    await runtime.close();
  }
});

test('preview_sort exact keeps preview frames exact: the host can turn the reuse off', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const exact = await reference(files, [at('scene-a', 2)]);
  const runtime = await open(files, { ...SAME_MSAA, preview_sort: 'exact' });
  try {
    await draw(runtime, [at('scene-a', -2)], PREVIEW);
    const moving = await draw(runtime, [at('scene-a', 2)], PREVIEW);
    assert.equal(moving.source.stale_sort_layers, 0);
    assert.equal(moving.hash, exact);
  } finally {
    await runtime.close();
  }
});

test('a scene without splats has no sort to reuse: never stale, edges untouched', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const boxes = join(files.directory, 'boxes.json');
  writeFileSync(boxes, JSON.stringify(boxScene()));
  const runtime = await openRuntime({ extraFiles: [boxes] });
  try {
    await runtime.hello(SAME_MSAA);
    assert.equal((await runtime.load(sceneAsset('boxes', boxes))).state, 'ready');
    const layers = [sceneLayer('boxes', 0.5)];
    const preview = await runtime.frame(layers, { ...PREVIEW });
    assert.equal(preview.type, 'frame.done', JSON.stringify(preview.error));
    assert.equal(preview.source.stale_sort_layers ?? 0, 0);
    assert.equal(preview.hash, (await runtime.frame(layers)).hash);
  } finally {
    await runtime.close();
  }
});

test('a lost context does not leave a stale order behind: the frame after the restore is exact', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const exact = await reference(files, [at('scene-a', 0.75)]);
  const runtime = await open(files, SAME_MSAA);
  try {
    await draw(runtime, [at('scene-a', -1)], PREVIEW);
    await draw(runtime, [at('scene-a', 1)], PREVIEW);
    await runtime.page.evaluate(() => window.TesselRuntime.debug.loseContext());
    await runtime.waitFor((m) => m.type === 'context.lost', 'context.lost');
    await runtime.page.evaluate(() => window.TesselRuntime.debug.restoreContext());
    await runtime.waitFor((m) => m.type === 'context.restored', 'context.restored');
    const preview = await draw(runtime, [at('scene-a', 0.75)], PREVIEW);
    assert.equal(preview.source.stale_sort_layers, 0, 'a new context has no order from the old one');
    assert.equal(preview.hash, exact);
  } finally {
    await runtime.close();
  }
});

test('every hello says the sort order whole: absent is `latest` again, a name that is not one is `exact`', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const runtime = await open(files, { ...SAME_MSAA, preview_sort: 'exact' });
  try {
    const sort = async () => (await runtime.retention()).preview_sort;
    assert.equal(await sort(), 'exact');
    // A later hello that does not mention it is not "keep the last one": it is the default.
    await runtime.hello({ ...SAME_MSAA });
    assert.equal(await sort(), 'latest');
    await draw(runtime, [at('scene-a', -2)], PREVIEW);
    assert.equal((await draw(runtime, [at('scene-a', 2)], PREVIEW)).source.stale_sort_layers, 1, 'latest reuses the order');
    // Neither a typo nor another type is guessed at as `latest`: it is the order that cannot show another camera's sort.
    for (const unknown of ['fast', 'LATEST ', 7, '']) {
      await runtime.hello({ ...SAME_MSAA, preview_sort: unknown });
      assert.equal(await sort(), 'exact', `${JSON.stringify(unknown)}`);
    }
    await draw(runtime, [at('scene-a', -1)], PREVIEW);
    assert.equal((await draw(runtime, [at('scene-a', 1)], PREVIEW)).source.stale_sort_layers, 0, 'exact waits for its own sort');
    await runtime.hello({ ...SAME_MSAA, preview_sort: 'latest' });
    assert.equal(await sort(), 'latest');
  } finally {
    await runtime.close();
  }
});
