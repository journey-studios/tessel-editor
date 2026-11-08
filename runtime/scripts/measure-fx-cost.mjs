#!/usr/bin/env node
// Measures what the output-scope effects (F016) cost per frame in the runtime, with the same
// scenario before and after a change: one flat wall of a directed scene at --size, drawn with 0, 1,
// 2 and 8 vignette passes, every frame through the real bridge in headless Chrome.
//
//   python3 scripts/directed-scene-proof/with-gpu-lock.py node runtime/scripts/measure-fx-cost.mjs [--size 1280x720] [--frames 30]
//
// It runs under SwiftShader (software GL; `openRuntime` has no other mode): the numbers compare two
// builds of this runtime on the same machine, they are NOT the speed of a native GPU and must not
// be quoted as such. Prints one JSON object: the median of `render_ms`/`total_ms`/`draw_ms` per
// configuration, the programs and targets alive after the run and the retention counters it saw.
// It measures; it asserts nothing.
import { directedLayer, directedRuntime, flat, makeFiles, node, reference, resource, sceneAsset, scenePlan } from '../test/directed-scene-fixtures.mjs';

const argument = (name, fallback) => {
  const index = process.argv.indexOf(`--${name}`);
  return index === -1 ? fallback : process.argv[index + 1];
};
const [width, height] = argument('size', '1280x720').split('x').map(Number);
const frames = Number(argument('frames', '30'));
const PASSES = [0, 1, 2, 8];

const median = (values) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted.length ? Number(sorted[Math.floor(sorted.length / 2)].toFixed(2)) : null;
};

const pass = (index) => ({
  trackId: `t${index}`,
  clipId: `v${index}`,
  effect: 'vignette',
  frameInClip: 0,
  weight: 1,
  amount: 0.5,
  params: {
    radius: 0.6 + index * 0.01,
    softness: 0.45,
    intensity: 0.5,
    color: { r: 0, g: 0, b: 0 },
    roundness: 0.6,
    center: { x: 0.5, y: 0.5 },
  },
  scope: { kind: 'output' },
});

const plan = (count) =>
  scenePlan({
    nodes: [node('wall-node', 'wall', 0, 0, -2, 60, 34)],
    layers: [['wall-node', 'wall', 'opaque']],
    resources: [resource('wall', 16, 9)],
    output: [width, height],
    requires: ['camera', 'nodeTransforms', 'imagePlanes', 'opaqueAlpha', ...(count > 0 ? ['outputFx'] : [])],
    extra: count > 0 ? { fx: Array.from({ length: count }, (_, index) => pass(index)) } : {},
  });

const cleanups = [];
const t = { after: (callback) => cleanups.push(callback) };
const paths = makeFiles(t, { wall: flat(16, 9, [200, 160, 80]) });
const runtime = await directedRuntime(paths);
try {
  const loaded = await runtime.load(sceneAsset('slot-wall', paths.wall, 'wall'));
  if (loaded.state !== 'ready') throw new Error(`load failed: ${JSON.stringify(loaded.error)}`);
  const result = { size: [width, height], frames, gl: 'swiftshader', configurations: {} };
  for (const count of PASSES) {
    const layer = directedLayer(plan(count), [reference('wall', 'slot-wall')]);
    const runs = [];
    for (let index = 0; index < frames + 3; index += 1) {
      const message = await runtime.frame([layer], { size: [width, height] });
      if (message.type !== 'frame.done') throw new Error(`frame failed: ${JSON.stringify(message.error)}`);
      // The first frames compile the programs and allocate the targets: not the steady state.
      if (index >= 3) runs.push(message);
    }
    const last = runs.at(-1);
    result.configurations[`${count}_passes`] = {
      render_ms: median(runs.map((message) => message.timings.render_ms)),
      draw_ms: median(runs.map((message) => message.timings.draw_ms)),
      total_ms: median(runs.map((message) => message.timings.total_ms)),
      gl_programs: last.cache.gl_programs,
      gl_geometries: last.cache.gl_geometries,
      target_entries: last.cache.target_entries,
      // What the last (steady) frame created, i.e. what a frame rebuilds that it could have kept.
      steady_frame: Object.fromEntries(Object.entries(last.cache).filter(([name, value]) => /_(created|built|reused|evicted)$/.test(name) && value !== 0)),
    };
  }
  console.log(JSON.stringify(result, null, 2));
} finally {
  await runtime.close();
  for (const callback of cleanups) callback();
}
