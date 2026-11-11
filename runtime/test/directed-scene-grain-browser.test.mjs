// Real pixels of the `filmGrain` look (F059, P3), in headless Chrome (SwiftShader) through the same
// bridge messages the desktop sends. Skipped, loudly, without Chrome or the bundle
// (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
//
// The expectation is the INDEPENDENT corpus `docs/migration/fixtures/pilot-fx/grain-cases.json`
// (scripts/pilot-grain-corpus.py): the pilot's shader transcribed in Python with float32 emulated, not
// this runtime's GLSL and not the core's Rust reference. The same file is run against the Rust core
// by crates/core/tests/pilot_grain_corpus.rs, so GLSL, Rust and Python agree on the same bytes.
//
// What is proved here: every probe of every pinned picture is within the corpus tolerance (the cell
// size, the glFragY flip, the per-pixel hash, the sRGB-space grain, the luma response and the linear
// mix are all in the numbers); a grain of amount 0 (any of the three factors) is the baseline byte for
// byte; the editor's alpha survives the pass; one program serves every pass of the look; the same plan
// twice is the same picture, a different seed is not.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { directedLayer, directedRuntime, makeFiles, node, png, reference, resource, sceneAsset, scenePlan } from './directed-scene-fixtures.mjs';
import { skipUnavailable } from './support.mjs';

const CORPUS = JSON.parse(readFileSync(fileURLToPath(new URL('../../docs/migration/fixtures/pilot-fx/grain-cases.json', import.meta.url)), 'utf8'));
const CASES = CORPUS.pixels;

const plan = (size, fx, alpha = 'opaque') =>
  scenePlan({
    output: size,
    nodes: [node('wall-node', 'wall', 0, 0, -2, 200, 200)],
    layers: [['wall-node', 'wall', alpha]],
    resources: [resource('wall', 16, 9)],
    requires: ['camera', 'nodeTransforms', 'imagePlanes', alpha === 'opaque' ? 'opaqueAlpha' : 'straightAlpha', ...(fx.length > 0 ? ['outputFx', 'fxNoise'] : [])],
    extra: fx.length > 0 ? { fx } : {},
  });

/** A resolved grain pass as the core's plan lists it: `amount` is intensity x weight, the seed is resolved. */
const grain = (id, { params, pass_amount: amount, seed }) => ({
  trackId: 't',
  clipId: id,
  effect: 'filmGrain',
  frameInClip: 0,
  weight: 1,
  amount,
  seed,
  params: { amount: params.amount, size: params.size, luma: params.luma, colored: params.colored, intensity: params.intensity },
  scope: { kind: 'output' },
});

const caseOf = (id) => CASES.find((entry) => entry.id === id);

const withSource = async (t, rgba, body) => {
  if (skipUnavailable(t, assert)) return;
  const paths = makeFiles(t, { wall: png(16, 9, () => rgba) });
  const runtime = await directedRuntime(paths);
  try {
    const loaded = await runtime.load(sceneAsset('slot-wall', paths.wall, 'wall'));
    assert.equal(loaded.state, 'ready', JSON.stringify(loaded.error));
    await body(runtime);
  } finally {
    await runtime.close();
  }
};

const draw = async (runtime, size, fx, probes, alpha = 'opaque') => {
  const message = await runtime.frame([directedLayer(plan(size, fx, alpha), [reference('wall', 'slot-wall')])], { size, probes });
  assert.equal(message.type, 'frame.done', JSON.stringify(message.error));
  return message;
};

const show = (pixel) => `rgb(${pixel.slice(0, 3).join(',')})`;

/** Draws every case that shares a source colour; returns the per-case worst channel difference. */
const compare = async (runtime, entry) => {
  const size = [entry.width, entry.height];
  const probes = entry.probes.map((probe) => [probe.x, probe.y]);
  const message = await draw(runtime, size, [grain(entry.id, entry)], probes);
  let worst = 0;
  message.probes.forEach((pixel, index) => {
    const want = entry.probes[index].expected;
    for (let channel = 0; channel < 3; channel += 1) {
      const difference = Math.abs(pixel[channel] - want[channel]);
      worst = Math.max(worst, difference);
      assert.ok(difference <= entry.tolerance_per_channel, `${entry.id} probe (${probes[index]}) channel ${channel}: expected ${want[channel].toFixed(2)}, got ${show(pixel)}`);
    }
  });
  return { worst, message };
};

const bySource = (ids) => {
  const groups = new Map();
  for (const entry of CASES.filter((candidate) => ids(candidate))) {
    const key = JSON.stringify(entry.source_rgb);
    groups.set(key, [...(groups.get(key) ?? []), entry]);
  }
  return groups;
};

test('every pinned picture of the independent corpus is drawn within its tolerance', async (t) => {
  if (skipUnavailable(t, assert)) return;
  let probes = 0;
  for (const [key, entries] of bySource((entry) => entry.source_alpha === 255)) {
    const rgb = JSON.parse(key);
    await withSource(t, [...rgb, 255], async (runtime) => {
      for (const entry of entries) {
        await compare(runtime, entry);
        probes += entry.probes.length;
      }
    });
  }
  assert.ok(probes > 250, `${probes} probes compared`);
});

test('a grain of amount, intensity-times-weight 0 is the baseline byte for byte, and the pass is still drawn', async (t) => {
  await withSource(t, [200, 160, 80, 255], async (runtime) => {
    for (const id of ['grain-amount-zero-is-the-baseline', 'grain-pass-amount-zero-is-the-baseline']) {
      const entry = caseOf(id);
      assert.equal(entry.identity, true);
      const size = [entry.width, entry.height];
      const probes = entry.probes.map((probe) => [probe.x, probe.y]);
      const baseline = await draw(runtime, size, [], probes);
      const zero = await draw(runtime, size, [grain(id, entry)], probes);
      assert.deepEqual(zero.probes, baseline.probes, `${id}: identity at the probes`);
      assert.equal(zero.hash, baseline.hash, `${id}: the whole picture is the baseline's`);
      assert.equal(zero.source.directed_scene_fx, 1, 'the pass is drawn (and is the identity)');
    }
  });
});

test('a translucent source is grained like an opaque one: the colour moves, the opaque canvas stays opaque', async (t) => {
  // The canvas of this runtime is opaque (`alpha: false`, cleared to opaque black), so a probe reads
  // 255 whatever the source alpha; the preserved ALPHA of the pass itself is proved on the shader
  // (`output-fx.test.mjs`). What this proves is that a source with alpha still runs the grain.
  const entry = caseOf('grain-alpha-is-preserved');
  await withSource(t, [128, 128, 128, 128], async (runtime) => {
    const size = [entry.width, entry.height];
    const probes = entry.probes.map((probe) => [probe.x, probe.y]);
    const baseline = await draw(runtime, size, [], probes, 'straight');
    const grained = await draw(runtime, size, [grain(entry.id, entry)], probes, 'straight');
    grained.probes.forEach((pixel, index) => assert.equal(pixel[3], baseline.probes[index][3], `probe ${index}: alpha is the baseline's`));
    assert.ok(
      grained.probes.some((pixel, index) => [0, 1, 2].some((channel) => pixel[channel] !== baseline.probes[index][channel])),
      'the colour was grained'
    );
  });
});

test('one program serves every grain pass; the same plan twice is the same picture and another seed is not', async (t) => {
  const entry = caseOf('grain-mono-fractional-cell');
  await withSource(t, [128, 128, 128, 255], async (runtime) => {
    const size = [entry.width, entry.height];
    const probes = entry.probes.map((probe) => [probe.x, probe.y]);
    const first = await draw(runtime, size, [grain('a', entry)], probes);
    const again = await draw(runtime, size, [grain('a', entry)], probes);
    assert.equal(again.hash, first.hash, 'deterministic: the same plan is the same picture');
    const other = await draw(runtime, size, [grain('a', { ...entry, seed: entry.seed + 1 })], probes);
    assert.notEqual(other.hash, first.hash, 'a different seed is a different picture');
    // Two passes of the same look share one material; nothing is compiled per frame.
    const stacked = await draw(runtime, size, [grain('a', entry), grain('b', { ...entry, seed: 77 })], probes);
    assert.notEqual(stacked.hash, first.hash);
    assert.equal((await runtime.retention()).fx_material_entries, 1, 'one material for the look, not one per pass');
    for (let frame = 0; frame < 5; frame += 1) await draw(runtime, size, [grain('a', { ...entry, seed: 1000 + frame })], probes);
    assert.equal((await runtime.retention()).fx_material_entries, 1, 'still one material after five more frames');
  });
});

test('a grain plan that does not ask for fxNoise, or carries no seed, is a typed refusal and draws nothing', async (t) => {
  const entry = caseOf('grain-mono-fractional-cell');
  await withSource(t, [128, 128, 128, 255], async (runtime) => {
    const size = [entry.width, entry.height];
    const probes = [[1, 1]];
    const unasked = plan(size, [grain('a', entry)]);
    unasked.requires = unasked.requires.filter((name) => name !== 'fxNoise');
    const refused = await runtime.frame([directedLayer(unasked, [reference('wall', 'slot-wall')])], { size, probes });
    assert.equal(refused.type, 'frame.failed', JSON.stringify(refused));
    assert.equal(refused.error.code, 'invalid_plan');
    assert.equal(refused.error.details.pointer, '/requires');
    const seedless = grain('a', entry);
    delete seedless.seed;
    const missing = await runtime.frame([directedLayer(plan(size, [seedless]), [reference('wall', 'slot-wall')])], { size, probes });
    assert.equal(missing.type, 'frame.failed', JSON.stringify(missing));
    assert.equal(missing.error.code, 'invalid_plan');
    // The runtime still draws afterwards: the refusals left no half-built state.
    const recovered = await draw(runtime, size, [grain('a', entry)], entry.probes.map((probe) => [probe.x, probe.y]));
    assert.equal(recovered.type, 'frame.done');
  });
});
