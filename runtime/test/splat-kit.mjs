// Shared fixtures of the BL130 browser tests (preview-sort, splat-lineages, splat-failures): translucent splat
// clouds whose painting order shows in the picture, scenes that share an effect cloud as the scenes of a crossfade
// do, and a runtime opened over them. Not a test file (no `.test.` in the name).
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { extname, join } from 'node:path';

import { openRuntime, sceneAsset, sceneLayer } from './support.mjs';

/** Large, translucent, overlapping splats: the order they are painted in is what the picture shows. */
export const writeCloud = (directory, name, seed) => {
  const count = 60;
  const bytes = Buffer.alloc(count * 32);
  for (let index = 0; index < count; index += 1) {
    const offset = index * 32;
    [Math.sin(index + seed) * 0.5, Math.cos(index * 1.7 + seed) * 0.4, Math.sin(index * 0.9 + seed) * 1.2, 0.3, 0.25, 0.28].forEach((value, slot) =>
      bytes.writeFloatLE(value, offset + slot * 4),
    );
    bytes.set([60 + ((index * 37) % 190), 40 + ((index * 91) % 200), 80 + ((index * 53) % 170), 150, 255, 128, 128, 128], offset + 24);
  }
  const path = join(directory, `${name}.splat`);
  writeFileSync(path, bytes);
  return path;
};

/** Each scene has its own cloud and the effect cloud every scene shares, as the scenes of a crossfade do. */
export const writeScene = (directory, name, asset) => {
  const path = join(directory, `${name}.json`);
  writeFileSync(
    path,
    JSON.stringify({
      format: 'tessel-scene',
      version: 1,
      background: [0, 0, 0.05, 1],
      objects: [
        { type: 'splat', asset, position: [0, 0, 0], scale: 1 },
        { type: 'splat', asset: 'splat-fx', position: [0.3, 0.1, 0.5], scale: 0.4 },
      ],
    }),
  );
  return path;
};

const SEEDS = { a: 1, b: 4, c: 2.5, d: 6, fx: 7 };

/** Files of the scenes named in `names` (each with its own cloud) and of the shared effect cloud. */
export const kit = (names = ['a', 'b']) => {
  const directory = mkdtempSync(join(tmpdir(), 'tessel-preview-sort-'));
  const files = { directory, clouds: {}, scenes: {} };
  for (const id of [...names, 'fx']) files.clouds[id] = writeCloud(directory, `cloud-${id}`, SEEDS[id]);
  for (const id of names) files.scenes[id] = writeScene(directory, `scene-${id}`, `splat-${id}`);
  return files;
};

export const at = (asset, x) => ({ ...sceneLayer(asset, 0), camera: { position: [x, 0.3, 4], target: [0, 0, 0], fov: 50 } });

export const open = async (files, hello = {}) => {
  const runtime = await openRuntime({ extraFiles: [...Object.values(files.clouds), ...Object.values(files.scenes)] });
  await runtime.hello(hello);
  for (const id of Object.keys(files.clouds)) {
    const loaded = await runtime.load({ id: `splat-${id}`, kind: 'splat', source: { type: 'file', path: files.clouds[id] }, meta: { format: extname(files.clouds[id]).slice(1) } });
    assert.equal(loaded.state, 'ready');
  }
  for (const id of Object.keys(files.scenes)) {
    assert.equal((await runtime.load(sceneAsset(`scene-${id}`, files.scenes[id]))).state, 'ready');
  }
  return runtime;
};

export const draw = async (runtime, layers, options = {}) => {
  const done = await runtime.frame(layers, { size: [320, 180], ...options });
  assert.equal(done.type, 'frame.done', JSON.stringify(done.error));
  return done;
};

export const PREVIEW = { quality: 'preview' };
/** No MSAA reduction: the order of the splats is the only thing a preview frame can differ in. */
export const SAME_MSAA = { preview_scene_samples: 4 };

/** The picture the final quality draws for `layers`, from a runtime that never drew anything else. */
export const reference = async (files, layers, hello = SAME_MSAA) => {
  const fresh = await open(files, hello);
  try {
    return (await draw(fresh, layers)).hash;
  } finally {
    await fresh.close();
  }
};

/** Opens a runtime, hands it to `body` and closes it, whatever `body` does. */
export const withRuntime = async (files, hello, body) => {
  const runtime = await open(files, hello);
  try {
    return await body(runtime);
  } finally {
    await runtime.close();
  }
};
