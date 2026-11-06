// LOCAL MEASUREMENT PROBE (not part of the product). Wraps prototypes from outside, so the same
// file instruments the sources before and after the change. Spans of one frame go to
// `source.perf` of frame.done.
import { PackedSplats, SplatAccumulator } from '@sparkjsdev/spark';
import { AssetStore } from '__SRC__/assets.js';
import { Compositor } from '__SRC__/compositor.js';
import { SceneRig } from '__SRC__/scene.js';
import { SplatStage } from '__SRC__/splat-stage.js';

const P = { sum: {}, cnt: {} };
const add = (name, ms) => {
  P.sum[name] = (P.sum[name] ?? 0) + ms;
  P.cnt[name] = (P.cnt[name] ?? 0) + 1;
};
const asyncSpan = (owner, key, label) => {
  const original = owner[key];
  owner[key] = async function (...args) {
    const t = performance.now();
    try {
      return await original.apply(this, args);
    } finally {
      add(label, performance.now() - t);
    }
  };
};
const syncSpan = (owner, key, label) => {
  const original = owner[key];
  owner[key] = function (...args) {
    const t = performance.now();
    try {
      return original.apply(this, args);
    } finally {
      add(label, performance.now() - t);
    }
  };
};

asyncSpan(AssetStore.prototype, 'splatLease', 'lease_total');
asyncSpan(Compositor.prototype, 'sceneRig', 'rig_build');
syncSpan(SceneRig.prototype, 'dispose', 'rig_dispose');
asyncSpan(SceneRig.prototype, 'draw', 'scene_draw');
syncSpan(SceneRig.prototype, 'render', 'scene_render_submit');
asyncSpan(SplatStage.prototype, 'settle', 'settle_total');

const reinit = PackedSplats.prototype.reinitialize;
PackedSplats.prototype.reinitialize = function (options) {
  if (options?.fileBytes || options?.url) add('decode', 0);
  return reinit.call(this, options);
};
const prepareProgram = SplatAccumulator.prototype.prepareProgramMaterial;
SplatAccumulator.prototype.prepareProgramMaterial = function (generator, covGenerator) {
  const fresh = !SplatAccumulator.generatorProgram.get(generator ?? covGenerator);
  const t = performance.now();
  try {
    return prepareProgram.call(this, generator, covGenerator);
  } finally {
    if (fresh) add('spark_program_new', performance.now() - t);
  }
};
const prepare = SplatAccumulator.prototype.prepareGenerate;
SplatAccumulator.prototype.prepareGenerate = function (args) {
  const t = performance.now();
  const result = prepare.call(this, args);
  add('spark_prepare', performance.now() - t);
  if (!result.sameMapping) add('spark_mapping_changed', 0);
  const generate = result.generate;
  result.generate = () => {
    const g = performance.now();
    generate();
    add('spark_generate_cpu', performance.now() - g);
  };
  return result;
};

const draw = Compositor.prototype.draw;
Compositor.prototype.draw = async function (args) {
  P.sum = {};
  P.cnt = {};
  try {
    const pixels = await draw.call(this, args);
    if (!args.readback && globalThis.__TESSEL_DRAIN !== false) {
      const px = new Uint8Array(4);
      const d = performance.now();
      this.gl.readPixels(0, 0, 1, 1, this.gl.RGBA, this.gl.UNSIGNED_BYTE, px);
      add('drain', performance.now() - d);
    }
    return pixels;
  } finally {
    args.sources.perf = { sum: P.sum, cnt: P.cnt };
  }
};
