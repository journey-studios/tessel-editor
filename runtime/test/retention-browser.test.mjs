// Retention in a real page (headless Chrome, SwiftShader): the same pixels with it on and off,
// objects that are actually kept, and every way they must die. Skipped, loudly, when there is no
// Chrome or no bundle (TESSEL_REQUIRE_RUNTIME=1 turns that into a failure).
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  imageLayer,
  makeProject,
  openRuntime,
  picture,
  sceneAsset,
  sceneLayer,
  sequenceAsset,
  shuffled,
  skipUnavailable,
  stillAsset,
  textLayer,
  transform,
  videoLayer,
} from './support.mjs';

/** Every kind of content the monitor draws, as named frames a test can request in any order. */
const frameSpecs = (project) => [
  { name: 'image', layers: [imageLayer('a')] },
  { name: 'image-wide', layers: [imageLayer('b')] },
  {
    name: 'two-layers-blend',
    layers: [imageLayer('a'), imageLayer('b', { blend: 'screen', opacity: 0.5, transform: transform({ scale: 0.6, x: 10 }) })],
  },
  {
    name: 'edited',
    layers: [imageLayer('a', { blend: 'multiply', opacity: 0.7, transform: transform({ rotation: 15, scale: 0.8, crop: [10, 0, 5, 0] }) }), textLayer('A')],
  },
  { name: 'text', layers: [textLayer('TESSEL')] },
  { name: 'text-changed', layers: [textLayer('TESSEL 2')] },
  { name: 'image+text', layers: [imageLayer('a'), textLayer('TESSEL')] },
  ...Array.from({ length: project.frames }, (_, frame) => ({ name: `video-${frame}`, layers: [videoLayer('seq', frame)] })),
  { name: 'scene-0', layers: [sceneLayer('box', 0)] },
  { name: 'scene-1', layers: [sceneLayer('box', 1.5)] },
  { name: 'scene+image', layers: [imageLayer('c'), sceneLayer('box', 0.5)] },
];

const loadProject = async (runtime, project) => {
  for (const asset of [
    stillAsset('a', project.files.a),
    stillAsset('b', project.files.b),
    stillAsset('c', project.files.c),
    sceneAsset('box', project.files.scene),
    sequenceAsset('seq', project.files.pattern, project.frames),
  ]) {
    assert.equal((await runtime.load(asset)).state, 'ready');
  }
};

const hashesOf = async (retain, order, project) => {
  const runtime = await openRuntime({ project });
  try {
    await runtime.hello({ retain });
    await loadProject(runtime, project);
    const specs = new Map(frameSpecs(project).map((spec) => [spec.name, spec]));
    const hashes = {};
    for (const name of order) {
      const done = await runtime.frame(specs.get(name).layers);
      assert.equal(done.type, 'frame.done', `${name}: ${JSON.stringify(done.error)}`);
      assert.equal(done.cache.retain, retain);
      hashes[name] = done.hash;
    }
    return hashes;
  } finally {
    await runtime.close();
  }
};

test('retention: the same pixels with it on and off, in any order, for every kind of layer', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const names = frameSpecs(project).map((spec) => spec.name);

  const baseline = await hashesOf(false, names, project);
  assert.equal(new Set(Object.values(baseline)).size > names.length - 4, true, 'the frames really differ from each other');
  const retainedInOrder = await hashesOf(true, names, project);
  assert.deepEqual(retainedInOrder, baseline, 'retained, in order');
  // Out of order: a retained texture, material, rig or target must not remember the previous frame.
  for (const seed of [7, 99]) {
    const retained = await hashesOf(true, shuffled([...names, ...names], seed), project);
    assert.deepEqual(retained, baseline, `retained, shuffled with seed ${seed}`);
  }
});

test('retention: images, text, materials, scene rigs and targets are built once and then reused', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const runtime = await openRuntime({ project });
  try {
    await runtime.hello({ retain: true });
    await loadProject(runtime, project);
    const layers = [imageLayer('a'), textLayer('KEEP'), sceneLayer('box', 0)];

    const first = await runtime.frame(layers);
    assert.equal(first.cache.texture_created, 1);
    assert.equal(first.cache.text_created, 1);
    assert.equal(first.cache.material_created, 3, 'one material per layer slot');
    assert.equal(first.cache.rig_built, 1);
    assert.equal(first.cache.target_created, 1);
    const programs = first.cache.gl_programs;
    assert.ok(programs >= 2, 'the layer program and the scene program exist');

    for (let index = 0; index < 4; index += 1) {
      const again = await runtime.frame([imageLayer('a'), textLayer('KEEP'), sceneLayer('box', index)]);
      assert.equal(again.cache.texture_created, 0, 'a loaded still is not uploaded again');
      assert.equal(again.cache.texture_reused, 1);
      assert.equal(again.cache.text_created, 0);
      assert.equal(again.cache.text_reused, 1);
      assert.equal(again.cache.material_created, 0);
      assert.equal(again.cache.material_reused, 3);
      assert.equal(again.cache.rig_built, 0);
      assert.equal(again.cache.rig_reused, 1);
      assert.equal(again.cache.target_created, 0);
      assert.equal(again.cache.target_reused, 1);
      assert.equal(again.cache.gl_programs, programs, 'no program is destroyed and linked again');
      assert.equal(again.cache.gl_textures, first.cache.gl_textures, 'no GPU texture is created per frame');
    }
  } finally {
    await runtime.close();
  }
});

// WHY a test of its own: a single render target shared by every scene layer (the first prototype of
// this optimization) draws the second scene over the first and the compositor then samples the same
// texture twice. Only a frame with two scene layers can tell, and only against the build-and-free path.
test('retention: two scene layers keep a render target each and draw what the original path draws', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const otherScene = join(project.directory, 'other.json');
  writeFileSync(
    otherScene,
    JSON.stringify({
      format: 'tessel-scene',
      version: 1,
      background: [0.2, 0.05, 0.05, 1],
      objects: [{ type: 'box', size: [1.4, 0.6, 1], color: '#40c070', position: [0, 0, 0], spin: [10, 30, 0] }],
    }),
  );
  const farCamera = { position: [2.5, 1, 5], target: [0, 0, 0], fov: 40 };
  const frames = {
    two: [sceneLayer('box', 0.5), { ...sceneLayer('other', 1), camera: farCamera, blend: 'screen', opacity: 0.6, transform: transform({ scale: 0.7, x: 12 }) }],
    // The same scene file in two layers with two cameras: two targets, one rig.
    twin: [sceneLayer('box', 0.5), { ...sceneLayer('box', 0.5), camera: farCamera, opacity: 0.5 }],
    first: [sceneLayer('box', 0.5)],
    second: [{ ...sceneLayer('other', 1), camera: farCamera }],
  };
  const names = Object.keys(frames);

  const run = async (retain, order) => {
    const runtime = await openRuntime({ project, extraFiles: [otherScene] });
    try {
      await runtime.hello({ retain });
      await loadProject(runtime, project);
      assert.equal((await runtime.load(sceneAsset('other', otherScene))).state, 'ready');
      const hashes = {};
      const reports = [];
      for (const name of order) {
        const done = await runtime.frame(frames[name]);
        assert.equal(done.type, 'frame.done', `${name}: ${JSON.stringify(done.error)}`);
        hashes[name] = done.hash;
        reports.push({ name, cache: done.cache });
      }
      return { hashes, reports };
    } finally {
      await runtime.close();
    }
  };

  const baseline = await run(false, names);
  assert.equal(new Set(Object.values(baseline.hashes)).size, names.length, 'the four frames really differ');
  for (const [seed, order] of [[0, [...names, ...names]], [11, shuffled([...names, ...names, ...names], 11)]]) {
    const retained = await run(true, order);
    assert.deepEqual(retained.hashes, baseline.hashes, `two scene layers, retained, order ${seed}`);
    const twoLayerFrames = retained.reports.filter((report) => report.name === 'two' || report.name === 'twin');
    for (const { name, cache } of twoLayerFrames) {
      assert.equal(cache.target_entries, 2, `${name}: one render target per scene layer slot, not one shared`);
    }
  }
});

test('retention off keeps nothing between frames (the original path)', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const runtime = await openRuntime({ project });
  try {
    await runtime.hello({ retain: false });
    await loadProject(runtime, project);
    await runtime.frame([imageLayer('a'), textLayer('KEEP'), sceneLayer('box', 0)]);
    const done = await runtime.frame([imageLayer('a'), textLayer('KEEP'), sceneLayer('box', 0)]);
    assert.equal(done.cache.retain, false);
    assert.equal(done.cache.texture_entries, 0);
    assert.equal(done.cache.text_entries, 0);
    assert.equal(done.cache.material_entries, 0);
    assert.equal(done.cache.rig_entries, 0);
    assert.equal(done.cache.target_entries, 0);
    assert.equal(done.cache.material_created, 0);
  } finally {
    await runtime.close();
  }
});

test('retention: the same id with another source never draws the old picture', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const runtime = await openRuntime({ project });
  try {
    await runtime.hello({ retain: true });
    assert.equal((await runtime.load(stillAsset('x', project.files.a))).state, 'ready');
    const fromA = await runtime.frame([imageLayer('x')]);
    const again = await runtime.frame([imageLayer('x')]);
    assert.equal(again.cache.texture_reused, 1, 'the texture was retained before the swap');
    const generation = (await runtime.retention()).asset_generation;

    // Same id, a different file (and a different size: the layout must follow it too).
    assert.equal((await runtime.load(stillAsset('x', project.files.b))).state, 'ready');
    assert.ok((await runtime.retention()).asset_generation > generation);
    const fromB = await runtime.frame([imageLayer('x')]);
    assert.notEqual(fromB.hash, fromA.hash);
    assert.equal(fromB.cache.texture_created, 1, 'the old texture was not reused for the new source');

    // Going back restores the original picture exactly.
    assert.equal((await runtime.load(stillAsset('x', project.files.a))).state, 'ready');
    assert.equal((await runtime.frame([imageLayer('x')])).hash, fromA.hash);

    // The same path with other bytes behind it (a file edited in place) is a new load as well:
    // the host reloads the asset, and the runtime must not keep the old decode or texture.
    writeFileSync(project.files.a, picture(5));
    assert.equal((await runtime.load(stillAsset('x', project.files.a))).state, 'ready');
    const edited = await runtime.frame([imageLayer('x')]);
    assert.notEqual(edited.hash, fromA.hash);
    assert.equal(edited.cache.texture_created, 1);
  } finally {
    await runtime.close();
  }
});

test('retention: releasing an asset frees its texture and the rig built from it', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const runtime = await openRuntime({ project });
  try {
    await runtime.hello({ retain: true });
    await loadProject(runtime, project);
    await runtime.frame([imageLayer('a'), sceneLayer('box', 0)]);
    let state = await runtime.retention();
    assert.equal(state.texture_entries, 1);
    assert.equal(state.rig_entries, 1);

    await runtime.release('a');
    await runtime.release('box');
    state = await runtime.retention();
    assert.equal(state.texture_entries, 0, 'the texture dies with its asset');
    assert.equal(state.rig_entries, 0, 'the rig dies with its scene file');

    const refused = await runtime.frame([imageLayer('a')]);
    assert.equal(refused.type, 'frame.failed');
    assert.equal(refused.error.code, 'asset_unreadable');
  } finally {
    await runtime.close();
  }
});

test('retention: a lost context drops everything retained and the restored one draws the same pixels', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const runtime = await openRuntime({ project });
  try {
    await runtime.hello({ retain: true });
    await loadProject(runtime, project);
    const layers = [imageLayer('a'), textLayer('LOST'), sceneLayer('box', 1)];
    const before = await runtime.frame(layers);
    assert.ok((await runtime.retention()).text_entries > 0);
    const generations = await runtime.retention();

    await runtime.page.evaluate(() => window.TesselRuntime.debug.loseContext());
    await runtime.waitFor((m) => m.type === 'context.lost');
    const lost = await runtime.retention();
    assert.equal(lost.text_entries, 0);
    assert.equal(lost.material_entries, 0);
    assert.equal(lost.rig_entries, 0);
    assert.equal(lost.target_entries, 0);
    assert.equal(lost.texture_entries, 0);
    assert.ok(lost.context_generation > generations.context_generation);
    assert.ok(lost.asset_generation > generations.asset_generation);

    assert.equal((await runtime.frame(layers)).error.code, 'context_lost');
    await runtime.page.evaluate(() => window.TesselRuntime.debug.restoreContext());
    await runtime.waitFor((m) => m.type === 'context.restored');
    const after = await runtime.frame(layers);
    assert.equal(after.hash, before.hash, 'the picture after the restore is the picture before the loss');
    assert.equal(after.cache.texture_created, 1, 'textures were made again from the kept bitmaps');
    assert.equal(after.cache.rig_built, 1);
    const second = await runtime.frame(layers);
    assert.equal(second.cache.texture_created, 0, 'and retained again');
  } finally {
    await runtime.close();
  }
});

test('retention: every cache stays inside its bound and a frame never loses what it draws', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const runtime = await openRuntime({ project });
  try {
    await runtime.hello({ retain: true });
    await loadProject(runtime, project);
    await runtime.page.evaluate(() =>
      window.TesselRuntime.debug.setLimits({ textEntries: 4, materials: 2, rigs: 1, targets: 1, imageTextureBytes: 48 * 27 * 4 * 2 }),
    );

    // A counter that changes every frame cycles through the cache instead of growing it.
    for (let index = 0; index < 20; index += 1) {
      const done = await runtime.frame([textLayer(`T${index}`)]);
      assert.equal(done.type, 'frame.done');
      assert.ok(done.cache.text_entries <= 4, `text entries ${done.cache.text_entries}`);
    }
    const afterText = await runtime.retention();
    assert.equal(afterText.text_entries, 4);
    assert.ok(afterText.text_bytes > 0 && afterText.text_bytes < 32 * 1024 * 1024);
    assert.equal(afterText.counters.text_evicted, 16);

    // More layers than the material bound inside one frame: they all draw, and the surplus goes after.
    const many = Array.from({ length: 5 }, (_, slot) => imageLayer('a', { id: `l${slot}`, opacity: 0.3 }));
    const drawn = await runtime.frame(many);
    assert.equal(drawn.type, 'frame.done');
    assert.equal(drawn.cache.material_entries, 5, 'the frame that needs five keeps five');
    const next = await runtime.frame([imageLayer('a')]);
    assert.equal(next.cache.material_entries, 2);

    // Image textures: the budget holds two 48x27 pictures; the third use frees the least recent.
    await runtime.frame([imageLayer('a')]);
    await runtime.frame([imageLayer('c')]);
    assert.equal((await runtime.retention()).texture_entries, 2);
    await runtime.frame([imageLayer('b')]);
    assert.ok((await runtime.retention()).texture_bytes <= 64 * 36 * 4 + 48 * 27 * 4);
    const back = await runtime.frame([imageLayer('a')]);
    assert.equal(back.type, 'frame.done');
    assert.equal(back.cache.texture_created, 1, 'an evicted texture is uploaded again, never missing');

    // Scene rigs: one is retained; asking for another scene file replaces it.
    const second = `${project.files.scene}`;
    assert.equal((await runtime.load(sceneAsset('box2', second))).state, 'ready');
    await runtime.frame([sceneLayer('box', 0)]);
    await runtime.frame([sceneLayer('box2', 0)]);
    assert.equal((await runtime.retention()).rig_entries, 1);
  } finally {
    await runtime.close();
  }
});

test('retention: an asset replaced while a frame is still being built cancels that frame', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const runtime = await openRuntime({ project });
  try {
    await runtime.hello({ retain: true });
    await loadProject(runtime, project);
    await runtime.frame([imageLayer('a')]);

    // Hold the frame at the video layer (after the image layer was prepared) and replace the image.
    await runtime.page.evaluate(() => {
      const { assets } = window.TesselRuntime.debug;
      const original = assets.frameTexture.bind(assets);
      window.__gate = new Promise((resolve) => {
        window.__openGate = resolve;
      });
      assets.frameTexture = async (...args) => {
        await window.__gate;
        assets.frameTexture = original;
        return original(...args);
      };
    });
    await runtime.send({
      type: 'frame.render',
      id: 900,
      revision: 900,
      plan: { version: 1, canvas: { width: 160, height: 90 }, intent: { mode: 'sequence' }, frame: 0, layers: [imageLayer('a'), videoLayer('seq', 1)], requires: [], warnings: [] },
      output: { width: 160, height: 90 },
    });
    await new Promise((resolve) => setTimeout(resolve, 100));
    assert.equal((await runtime.load(stillAsset('a', project.files.c))).state, 'ready');
    await runtime.page.evaluate(() => window.__openGate());
    const failed = await runtime.waitFor((m) => m.id === 900 && (m.type === 'frame.failed' || m.type === 'frame.done'));
    assert.equal(failed.type, 'frame.failed');
    assert.equal(failed.error.code, 'cancelled');
    assert.match(failed.error.message, /replaced/);

    // The next frame draws the new source from a fresh texture.
    const fresh = await runtime.frame([imageLayer('a')]);
    assert.equal(fresh.type, 'frame.done');
    assert.equal(fresh.cache.texture_created, 1);
  } finally {
    await runtime.close();
  }
});

test('frame.presented is only sent to a host that asked for it, after frame.done', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const project = makeProject();
  const quiet = await openRuntime({ project });
  try {
    const ready = await quiet.hello({});
    assert.ok(ready.capabilities.features.includes('frame.presented'));
    await quiet.load(stillAsset('a', project.files.a));
    await quiet.frame([imageLayer('a')]);
    await quiet.page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    assert.equal(quiet.messages.some((m) => m.type === 'frame.presented'), false, 'an older host would reject the message');
  } finally {
    await quiet.close();
  }

  const asked = await openRuntime({ project });
  try {
    await asked.hello({ telemetry: { presented: true } });
    await asked.load(stillAsset('a', project.files.a));
    const done = await asked.frame([imageLayer('a')]);
    const presented = await asked.waitFor((m) => m.type === 'frame.presented' && m.id === done.id);
    assert.equal(presented.revision, done.revision);
    assert.equal(typeof presented.after_done_ms, 'number');
    // A frame that did not complete is never announced.
    const failed = await asked.frame([imageLayer('missing')]);
    assert.equal(failed.type, 'frame.failed');
    await asked.page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    assert.equal(asked.messages.some((m) => m.type === 'frame.presented' && m.id === failed.id), false);
  } finally {
    await asked.close();
  }
});

test('a scene with splats is never served from a retained rig: every frame still goes through Spark', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const directory = mkdtempSync(join(tmpdir(), 'tessel-splat-retention-'));
  // 300 deterministic splats, 32 bytes each: position, scale, RGBA, rotation.
  const bytes = Buffer.alloc(300 * 32);
  for (let index = 0; index < 300; index += 1) {
    const offset = index * 32;
    [Math.sin(index) * 0.8, Math.cos(index * 1.7) * 0.8, Math.sin(index * 0.3) * 0.8, 0.06, 0.06, 0.06].forEach((value, slot) =>
      bytes.writeFloatLE(value, offset + slot * 4),
    );
    bytes.set([40 + (index % 200), 255 - (index % 180), 90 + (index % 120), 220, 255, 128, 128, 128], offset + 24);
  }
  const splat = join(directory, 'cloud.splat');
  const scene = join(directory, 'cloud.json');
  writeFileSync(splat, bytes);
  writeFileSync(
    scene,
    JSON.stringify({ format: 'tessel-scene', version: 1, background: [0, 0, 0, 1], objects: [{ type: 'splat', asset: 'splat', position: [0, 0, 0], scale: 1 }] }),
  );
  const assets = [
    { id: 'splat', kind: 'splat', source: { type: 'file', path: splat }, meta: { format: 'splat' } },
    sceneAsset('cloud', scene),
  ];
  const camera = (x) => ({ ...sceneLayer('cloud', 0), camera: { position: [x, 0.3, 4], target: [0, 0, 0], fov: 50 } });
  const run = async (retain) => {
    const runtime = await openRuntime({ extraFiles: [splat, scene] });
    try {
      await runtime.hello({ retain });
      for (const asset of assets) assert.equal((await runtime.load(asset)).state, 'ready');
      const frames = [];
      for (const x of [0, 2, 0, 0]) {
        const done = await runtime.frame([camera(x)]);
        assert.equal(done.type, 'frame.done', JSON.stringify(done.error));
        frames.push({ hash: done.hash, cache: done.cache });
      }
      return frames;
    } finally {
      await runtime.close();
    }
  };
  const retained = await run(true);
  const legacy = await run(false);
  assert.deepEqual(
    retained.map((frame) => frame.hash),
    legacy.map((frame) => frame.hash),
    'retention does not change a splat frame',
  );
  assert.equal(retained[0].hash, retained[2].hash, 'the same camera draws the same pixels again');
  assert.equal(retained[0].hash, retained[3].hash, 'and again: no stale or skipped settle');
  assert.notEqual(retained[0].hash, retained[1].hash, 'the sort and the picture follow the camera');
  for (const frame of retained) {
    assert.equal(frame.cache.rig_built, 0, 'a splat scene never builds a retained rig');
    assert.equal(frame.cache.rig_reused, 0, 'nor reuses one');
  }
});
