// BL129: MSAA of scene layers by the quality of the frame, proven in a real page (headless Chrome,
// SwiftShader) through the same bridge messages the desktop uses.
//
//   * a `preview` frame (the monitor while the playhead moves) draws a scene layer with fewer
//     samples and says so (`source.scene_samples`, `source.reduced_scene_layers`);
//   * a frame without `quality` (explicit `render.frame`, export, probes, the refine after a stop)
//     draws at the final count and its pixels are the ones it always had, whatever came before;
//   * the result cache never serves a reduced picture as the final one (nor the reverse), because
//     the samples are part of its key and of the retained target it lives in.
//
// Skipped, loudly, without Chrome or the bundle (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { boxScene, openRuntime, sceneAsset, sceneLayer, skipUnavailable } from './support.mjs';

/** Few, large, translucent splats: their edges are what MSAA changes. */
const writeCloud = (directory) => {
  const count = 40;
  const bytes = Buffer.alloc(count * 32);
  for (let index = 0; index < count; index += 1) {
    const offset = index * 32;
    [Math.sin(index) * 0.6, Math.cos(index * 1.7) * 0.5, Math.sin(index * 0.3) * 0.4, 0.22, 0.09, 0.14].forEach((value, slot) =>
      bytes.writeFloatLE(value, offset + slot * 4),
    );
    bytes.set([60 + ((index * 37) % 190), 40 + ((index * 91) % 200), 80 + ((index * 53) % 170), 200, 255, 128, 128, 128], offset + 24);
  }
  const path = join(directory, 'cloud.splat');
  writeFileSync(path, bytes);
  return path;
};

const writeScene = (directory) => {
  const path = join(directory, 'scene.json');
  writeFileSync(
    path,
    JSON.stringify({
      format: 'tessel-scene',
      version: 1,
      background: [0, 0, 0.05, 1],
      objects: [{ type: 'splat', asset: 'splat', position: [0, 0, 0], scale: 1 }],
    }),
  );
  return path;
};

const kit = () => {
  const directory = mkdtempSync(join(tmpdir(), 'tessel-preview-samples-'));
  return { directory, cloud: writeCloud(directory), scene: writeScene(directory) };
};

const at = (x) => ({ ...sceneLayer('scene', 0), camera: { position: [x, 0.3, 4], target: [0, 0, 0], fov: 50 } });

const open = async (files, hello = {}) => {
  const runtime = await openRuntime({ extraFiles: [files.cloud, files.scene] });
  await runtime.hello(hello);
  assert.equal((await runtime.load({ id: 'splat', kind: 'splat', source: { type: 'file', path: files.cloud }, meta: { format: 'splat' } })).state, 'ready');
  assert.equal((await runtime.load(sceneAsset('scene', files.scene))).state, 'ready');
  return runtime;
};

const draw = async (runtime, x, options = {}) => {
  const done = await runtime.frame([at(x)], { size: [320, 180], ...options });
  assert.equal(done.type, 'frame.done', JSON.stringify(done.error));
  return done;
};

const PREVIEW = { quality: 'preview' };
// SwiftShader draws 2 samples as 4, so a test that needs the reduced picture to *differ* asks for 0
// (the runtime's default for a moving playhead, spelled out so the test does not depend on it).
const NO_MSAA = { preview_scene_samples: 0 };

test('a preview frame draws the scene with fewer samples and says so; the final one does not', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const runtime = await open(files);
  try {
    const preview = await draw(runtime, 0, PREVIEW);
    assert.equal(preview.source.quality, 'preview');
    assert.equal(preview.source.scene_samples, 0, 'the default of a moving playhead');
    assert.equal(preview.source.reduced_scene_layers, 1);
    const final = await draw(runtime, 0);
    assert.equal(final.source.scene_samples, 4);
    assert.equal(final.source.reduced_scene_layers, 0);
  } finally {
    await runtime.close();
  }
  // Reduced MSAA is a real change of the picture, not a label.
  const bare = await open(files, NO_MSAA);
  try {
    const preview = await draw(bare, 0, PREVIEW);
    assert.equal(preview.source.scene_samples, 0);
    assert.notEqual(preview.hash, (await draw(bare, 0)).hash, 'the edges of a preview picture differ from the final ones');
  } finally {
    await bare.close();
  }
});

test('the final frame is the same picture whatever preview frames came before it', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const fresh = await open(files);
  let reference;
  try {
    reference = await draw(fresh, 1.5);
  } finally {
    await fresh.close();
  }
  const runtime = await open(files, NO_MSAA);
  try {
    // A moving playhead: previews at other cameras, and one at the very camera the final will use.
    for (const x of [-2, -1, 0, 1, 1.5]) await draw(runtime, x, { ...PREVIEW, inspect: false });
    const final = await draw(runtime, 1.5);
    assert.equal(final.hash, reference.hash, 'refine/explicit pixels are unchanged by the previews before them');
    assert.equal(final.source.reduced_scene_layers, 0);
    // And the other way round: a preview after a final one is reduced again, not the cached final.
    const preview = await draw(runtime, 1.5, { ...PREVIEW, inspect: true });
    assert.equal(preview.source.reduced_scene_layers, 1);
    assert.notEqual(preview.hash, reference.hash);
  } finally {
    await runtime.close();
  }
});

test('the result cache never serves a reduced picture as the final one, nor the reverse', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const runtime = await open(files, NO_MSAA);
  try {
    const first = await draw(runtime, 0.5, PREVIEW);
    assert.equal(first.cache.result_hit, 0);
    const again = await draw(runtime, 0.5, PREVIEW);
    assert.equal(again.cache.result_hit, 1, 'a still camera in preview is served from the preview picture');
    assert.equal(again.source.reduced_scene_layers, 1, 'and it is still reported as reduced');
    assert.equal(again.hash, first.hash);
    const final = await draw(runtime, 0.5);
    assert.equal(final.cache.result_hit, 0, 'the reduced picture is not the final one: a real draw');
    assert.equal(final.source.reduced_scene_layers, 0);
    assert.notEqual(final.hash, first.hash);
    const finalAgain = await draw(runtime, 0.5);
    assert.equal(finalAgain.cache.result_hit, 1, 'the final picture is cached in turn');
    assert.equal(finalAgain.hash, final.hash);
    const back = await draw(runtime, 0.5, PREVIEW);
    assert.equal(back.cache.result_hit, 0, 'the final picture does not stand in for a preview one either');
    assert.equal(back.hash, first.hash, 'and the preview picture is the same one as before');
    assert.ok((await runtime.retention()).target_bytes > 0);
  } finally {
    await runtime.close();
  }
});

test('the host chooses the preview samples; 4 turns the reduction off and never exceeds the final count', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const hashes = new Map();
  for (const [label, hello] of [
    ['p0', { preview_scene_samples: 0 }],
    ['p2', { preview_scene_samples: 2 }],
    ['p4', { preview_scene_samples: 4 }],
    ['all0', { scene_samples: 0 }],
    ['final2', { scene_samples: 2, preview_scene_samples: 4 }],
  ]) {
    const runtime = await open(files, hello);
    try {
      const preview = await draw(runtime, 0.25, PREVIEW);
      const final = await draw(runtime, 0.25);
      hashes.set(label, { preview, final });
    } finally {
      await runtime.close();
    }
  }
  const samples = (label) => hashes.get(label).preview.source.scene_samples;
  assert.deepEqual(['p0', 'p2', 'p4', 'all0', 'final2'].map(samples), [0, 2, 4, 0, 2]);
  assert.equal(hashes.get('p0').preview.source.reduced_scene_layers, 1);
  assert.equal(hashes.get('p4').preview.source.reduced_scene_layers, 0, '4 is no reduction: nothing to refine');
  assert.equal(hashes.get('all0').preview.source.reduced_scene_layers, 0, 'a host that lowers every frame has nothing to refine');
  assert.equal(hashes.get('p4').preview.hash, hashes.get('p4').final.hash);
  // The explicit/final pixels depend only on the final count, not on the preview setting.
  assert.equal(hashes.get('p0').final.hash, hashes.get('p4').final.hash);
  assert.equal(hashes.get('p2').final.hash, hashes.get('p4').final.hash);
  assert.notEqual(hashes.get('p0').preview.hash, hashes.get('p2').preview.hash);
});

test('a lost context does not leave a reduced target serving the final picture', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const runtime = await open(files);
  try {
    const reference = await draw(runtime, 0.75);
    await draw(runtime, 0.75, PREVIEW);
    await runtime.page.evaluate(() => window.TesselRuntime.debug.loseContext());
    await runtime.waitFor((m) => m.type === 'context.lost', 'context.lost');
    await runtime.page.evaluate(() => window.TesselRuntime.debug.restoreContext());
    await runtime.waitFor((m) => m.type === 'context.restored', 'context.restored');
    const final = await draw(runtime, 0.75);
    assert.equal(final.cache.result_hit, 0);
    assert.equal(final.hash, reference.hash);
    assert.equal(final.source.reduced_scene_layers, 0);
  } finally {
    await runtime.close();
  }
});

test('a scene without splats keeps its edges: it is never reduced, preview or not', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const boxes = join(files.directory, 'boxes.json');
  writeFileSync(boxes, JSON.stringify(boxScene()));
  const runtime = await openRuntime({ extraFiles: [boxes] });
  try {
    await runtime.hello(NO_MSAA);
    assert.equal((await runtime.load(sceneAsset('boxes', boxes))).state, 'ready');
    const layers = [sceneLayer('boxes', 0.5)];
    const preview = await runtime.frame(layers, { ...PREVIEW });
    assert.equal(preview.type, 'frame.done', JSON.stringify(preview.error));
    assert.equal(preview.source.scene_samples, 4, 'a procedural scene is cheap and shows its MSAA');
    assert.equal(preview.source.reduced_scene_layers, 0, 'nothing to refine');
    const final = await runtime.frame(layers);
    assert.equal(final.hash, preview.hash);
  } finally {
    await runtime.close();
  }
});

test('a frame mixing a splat scene and a plain one reports the lowest count, whatever the layer order', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const files = kit();
  const boxes = join(files.directory, 'boxes.json');
  writeFileSync(boxes, JSON.stringify(boxScene()));
  const runtime = await openRuntime({ extraFiles: [files.cloud, files.scene, boxes] });
  try {
    await runtime.hello(NO_MSAA);
    assert.equal((await runtime.load({ id: 'splat', kind: 'splat', source: { type: 'file', path: files.cloud }, meta: { format: 'splat' } })).state, 'ready');
    assert.equal((await runtime.load(sceneAsset('scene', files.scene))).state, 'ready');
    assert.equal((await runtime.load(sceneAsset('boxes', boxes))).state, 'ready');
    const splat = { ...sceneLayer('scene', 0), opacity: 0.5 };
    const plain = { ...sceneLayer('boxes', 0.5), opacity: 0.5 };
    for (const layers of [[splat, plain], [plain, splat]]) {
      const preview = await runtime.frame(layers, { ...PREVIEW });
      assert.equal(preview.type, 'frame.done', JSON.stringify(preview.error));
      assert.equal(preview.source.scene_samples, 0, 'the header must not claim more than the weakest layer got');
      assert.equal(preview.source.reduced_scene_layers, 1, 'the splat layer still needs the refine');
    }
  } finally {
    await runtime.close();
  }
});
