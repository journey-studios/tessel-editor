// A layer the core marked `occluded` (a layer above covers the canvas with opaque pixels) is not
// prepared nor drawn, and the picture is the one of drawing it. Real page (headless Chrome,
// SwiftShader); skipped, loudly, without Chrome or the bundle (TESSEL_REQUIRE_RUNTIME=1 fails).
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { imageLayer, makeProject, openRuntime, sceneAsset, sceneLayer, skipUnavailable, stillAsset } from './support.mjs';

const loadStills = async (runtime, project) => {
  for (const asset of [stillAsset('a', project.files.a), stillAsset('b', project.files.b), sceneAsset('box', project.files.scene)]) {
    assert.equal((await runtime.load(asset)).state, 'ready');
  }
};

test('occlusion: skipping a covered layer gives the same pixels and counts the skip', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const runtime = await openRuntime({ project });
  try {
    await runtime.hello({});
    await loadStills(runtime, project);
    // `a` (48x27) and `b` (64x36) are opaque 16:9 pictures: `b` on top covers the 160x90 canvas.
    const drawn = await runtime.frame([imageLayer('a'), imageLayer('b')]);
    assert.equal(drawn.type, 'frame.done', JSON.stringify(drawn.error));
    const skipped = await runtime.frame([imageLayer('a', { occluded: true }), imageLayer('b')]);
    assert.equal(skipped.type, 'frame.done', JSON.stringify(skipped.error));
    assert.equal(skipped.hash, drawn.hash, 'the covered layer contributes nothing');
    assert.equal(skipped.source.occluded_layers, 1);
    assert.equal(drawn.source.occluded_layers, 0);

    // The layer above keeps its slot: it is drawn exactly as when nothing is skipped, and
    // what is below the cover (a scene, the expensive case) is not even decoded or sorted.
    const scene = await runtime.frame([sceneLayer('box', 0), imageLayer('b')]);
    const sceneSkipped = await runtime.frame([{ ...sceneLayer('box', 0), occluded: true }, imageLayer('b')]);
    assert.equal(sceneSkipped.hash, scene.hash);
    assert.equal(sceneSkipped.source.occluded_layers, 1);

    // A flag is a hint about layers *below* a cover: dropping the cover would show the layer.
    const alone = await runtime.frame([imageLayer('a')]);
    assert.notEqual(alone.hash, drawn.hash, 'the frames really differ, so equality above means something');
  } finally {
    await runtime.close();
  }
});

test('occlusion: a layer that is not flagged is always drawn', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const runtime = await openRuntime({ project });
  try {
    await runtime.hello({});
    await loadStills(runtime, project);
    const partial = await runtime.frame([imageLayer('a'), imageLayer('b', { opacity: 0.5 })]);
    const only = await runtime.frame([imageLayer('b', { opacity: 0.5 })]);
    assert.notEqual(partial.hash, only.hash, 'the layer under a translucent one shows through');
    assert.equal(partial.source.occluded_layers, 0);
  } finally {
    await runtime.close();
  }
});
