// Lifecycle regressions of the preview proxy, proven in a real page (headless Chrome, SwiftShader)
// through the same bridge messages the desktop sends. The cases the OPT01-OPT10 report listed as
// "not covered by an automated test":
//
//   * a lost WebGL context while a proxy is attached and a scrub frame or its refine is drawn;
//   * a frame cancelled, a project replaced or the runtime disposed with a frame in flight.
//
// What "correct" means here, in all of them: the last frame asked for converges to the same
// pixels it had before the incident, the original (explicit/export/refine) never reads the proxy,
// and what belonged to the dead context or the dead project is freed (no growth across cycles).
//
// The loss is forced with WEBGL_lose_context on a real context: genuine `webglcontextlost` and
// `webglcontextrestored` events, not a GPU reset (see lifecycle-fixtures.mjs).
// Skipped, loudly, without Chrome or the bundle (TESSEL_REQUIRE_RUNTIME=1 makes that a failure).
import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  COUNT,
  PROBES,
  dominantColor,
  entryOf,
  frameMessage,
  glMemory,
  lifecycleRuntime,
  loseDuring,
  loseNow,
  makeSequences,
  preview,
  proxyOf,
  restore,
} from './lifecycle-fixtures.mjs';
import { sequenceAsset, skipUnavailable } from './support.mjs';

const answeredBy = (id) => (message) => (message.type === 'frame.done' || message.type === 'frame.failed') && message.id === id;

test('a context lost mid scrub frame with a proxy attached: context_lost, then the same proxy picture and the same original', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const sequences = makeSequences(t);
  const runtime = await lifecycleRuntime(sequences);
  try {
    assert.equal((await runtime.attachProxy('seq', proxyOf(sequences))).state, 'attached');
    const proxyBefore = await runtime.frame(preview(2), { quality: 'preview', probes: PROBES });
    assert.deepEqual(proxyBefore.probes.map(dominantColor), ['black', 'green', 'green', 'black']);
    const originalBefore = await runtime.frame(preview(2), { probes: PROBES });
    assert.deepEqual(originalBefore.probes.map(dominantColor), ['black', 'red', 'red', 'black']);
    // The same frame again, so the counters below describe a steady state to compare with.
    const steady = await runtime.frame(preview(2), { quality: 'preview' });

    // A scrub frame that was not drawn yet: the loss lands while it is built.
    const failed = await loseDuring(runtime, preview(4), { quality: 'preview' });
    assert.equal(failed.type, 'frame.failed', 'a frame on a dead context never reports success');
    assert.equal(failed.error.code, 'context_lost', JSON.stringify(failed.error));

    // What belonged to the dead context is gone; what is CPU-side (bitmaps, the proxy itself) is not.
    const held = await entryOf(runtime);
    // At most the JS object of the frame the loss interrupted (made after the loss, never given to GL).
    assert.ok(held.textured <= 1, `textures kept after the loss: ${held.textured}`);
    assert.equal((await glMemory(runtime)).textures, 0, 'no GL texture of the lost context stays alive');
    assert.ok(held.proxy, 'the proxy stays attached: it is a source, not a GPU object');
    assert.ok(held.proxies >= 1 && held.originals >= 1, 'decoded frames survive, so the redraw does not fetch again');

    await restore(runtime);
    const proxyAfter = await runtime.frame(preview(2), { quality: 'preview', probes: PROBES });
    assert.equal(proxyAfter.type, 'frame.done', JSON.stringify(proxyAfter.error));
    assert.equal(proxyAfter.hash, proxyBefore.hash, 'the restored context draws the same proxy picture');
    assert.equal(proxyAfter.source.proxy_layers, 1);
    assert.equal(proxyAfter.cache.proxy_failed, 0, 'the loss is not blamed on the proxy');
    const originalAfter = await runtime.frame(preview(2), { probes: PROBES });
    assert.equal(originalAfter.hash, originalBefore.hash, 'the refine frame is the original, pixel for pixel');
    assert.equal(originalAfter.source.proxy_layers, 0);
    // Before the loss both pictures of frame 2 were alive (2 GL textures). The restored context starts
    // empty, uploads the proxy one, then the original: back to the same two and not one more.
    assert.equal(proxyAfter.cache.gl_textures, 1, 'only what the redraw needs is on the new context');
    assert.equal(originalAfter.cache.gl_textures, steady.cache.gl_textures, 'the same GL textures as before the loss: nothing leaked');
    // The frame the loss interrupted is drawn on demand, from the frames that were already decoded.
    const interrupted = await runtime.frame(preview(4), { quality: 'preview' });
    assert.equal(interrupted.type, 'frame.done');
    assert.equal(interrupted.source.proxy_layers, 1);
  } finally {
    await runtime.close();
  }
});

test('frames asked for while the context is lost are refused as context_lost and never drop the proxy', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const sequences = makeSequences(t);
  const runtime = await lifecycleRuntime(sequences);
  try {
    await runtime.attachProxy('seq', proxyOf(sequences));
    const reference = await runtime.frame(preview(1), { quality: 'preview' });
    await loseNow(runtime);
    // The monitor keeps scrubbing while the context is gone: every intent is answered, none hangs,
    // and none of them is a reason to discard the proxy (the host only retries after the restore).
    for (const frame of [3, 5, 0]) {
      const refused = await runtime.frame(preview(frame), { quality: 'preview' });
      assert.equal(refused.type, 'frame.failed');
      assert.equal(refused.error.code, 'context_lost', `frame ${frame}: ${JSON.stringify(refused.error)}`);
    }
    const explicit = await runtime.frame(preview(1));
    assert.equal(explicit.error.code, 'context_lost', 'an explicit/export frame is refused the same way');
    assert.ok((await entryOf(runtime)).proxy, 'still attached');
    await restore(runtime);
    const after = await runtime.frame(preview(1), { quality: 'preview' });
    assert.equal(after.hash, reference.hash);
    assert.equal(after.cache.proxy_failed, 0);
  } finally {
    await runtime.close();
  }
});

test('the refine after a loss: the original replaces the proxy picture the restored context could not keep', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const sequences = makeSequences(t);
  const runtime = await lifecycleRuntime(sequences);
  try {
    await runtime.attachProxy('seq', proxyOf(sequences));
    // The original of frame 3 as an explicit frame would draw it, before anything happens.
    const wanted = await runtime.frame(preview(3), { probes: PROBES });
    // Scrub (proxy) picture on screen, refine pending on the host...
    const scrubbed = await runtime.frame(preview(3), { quality: 'preview' });
    assert.notEqual(scrubbed.hash, wanted.hash);
    // ...and the context goes while the host waits for its refine delay.
    await loseNow(runtime);
    const refused = await runtime.frame(preview(3));
    assert.equal(refused.error.code, 'context_lost', 'the refine frame cannot be drawn on a dead context');
    await restore(runtime);
    // The host redraws the monitor (scrub quality) and then refines it: both must converge.
    const redraw = await runtime.frame(preview(3), { quality: 'preview' });
    assert.equal(redraw.hash, scrubbed.hash);
    const refined = await runtime.frame(preview(3), { probes: PROBES });
    assert.equal(refined.hash, wanted.hash, 'the refined picture is the original again, byte for byte');
    assert.deepEqual(refined.probes.map(dominantColor), ['black', 'red', 'red', 'black']);
  } finally {
    await runtime.close();
  }
});

test('repeated losses while scrubbing: every cycle recovers, nothing accumulates, the last frame is exact', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const sequences = makeSequences(t);
  const runtime = await lifecycleRuntime(sequences);
  try {
    await runtime.attachProxy('seq', proxyOf(sequences));
    const reference = [];
    for (let frame = 0; frame < COUNT; frame += 1) reference.push((await runtime.frame(preview(frame), { quality: 'preview' })).hash);
    const originalReference = (await runtime.frame(preview(COUNT - 1))).hash;
    const baseline = (await runtime.frame(preview(0), { quality: 'preview' })).cache;
    assert.equal(baseline.gl_textures, COUNT + 1, 'before any loss: every proxy frame retained, plus the original of the last');
    const generation = (await runtime.retention()).context_generation;

    const cycles = 5;
    for (let cycle = 0; cycle < cycles; cycle += 1) {
      const target = cycle % COUNT;
      const failed = await loseDuring(runtime, preview(target), { quality: 'preview', delay: cycle % 2 ? 0 : 1 });
      // Either the loss was seen mid-frame, or the frame was already done and only the next one is refused.
      assert.ok(failed.type === 'frame.failed' ? failed.error.code === 'context_lost' : true, JSON.stringify(failed.error));
      await restore(runtime);
      const drawn = await runtime.frame(preview(target), { quality: 'preview' });
      assert.equal(drawn.type, 'frame.done', `cycle ${cycle}: ${JSON.stringify(drawn.error)}`);
      assert.equal(drawn.hash, reference[target], `cycle ${cycle}: frame ${target} is the picture it was before the loss`);
      // Everything the dead context had is gone, so the redraw uploads exactly the one frame it shows:
      // a leak would make this grow cycle after cycle.
      assert.equal(drawn.cache.gl_textures, 1, `cycle ${cycle}: GL textures ${drawn.cache.gl_textures}`);
      const held = await entryOf(runtime);
      assert.ok(held.originals <= COUNT && held.proxies <= COUNT, `cycle ${cycle}: decoded frames stay bounded`);
    }
    assert.equal((await runtime.retention()).context_generation, generation + cycles, 'one generation per loss, no extra');

    // After the last cycle both qualities are exactly what they were.
    const last = await runtime.frame(preview(COUNT - 1), { quality: 'preview' });
    assert.equal(last.hash, reference[COUNT - 1]);
    const final = await runtime.frame(preview(COUNT - 1));
    assert.equal(final.hash, originalReference, 'the original never picked up the proxy across the cycles');
    assert.equal(final.cache.proxy_failed, 0);
    assert.ok(final.cache.gl_textures <= 3, `GL textures after the last cycle: ${final.cache.gl_textures}`);
  } finally {
    await runtime.close();
  }
});

test('an asset replaced while the context is lost: no stale proxy, no stale picture, nothing of the old one stays', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const first = makeSequences(t, { tone: 'warm' });
  const second = makeSequences(t, { tone: 'cold' });
  const runtime = await lifecycleRuntime(first, { patterns: [second.original, second.proxy] });
  try {
    await runtime.attachProxy('seq', proxyOf(first));
    await runtime.frame(preview(2), { quality: 'preview' });
    await loseNow(runtime);
    // The host reloads the project's assets (the session forgets them on `context.lost`).
    assert.equal((await runtime.load(sequenceAsset('seq', second.original, COUNT))).state, 'ready');
    await restore(runtime);
    const swapped = await runtime.frame(preview(2), { quality: 'preview', probes: PROBES });
    assert.equal(swapped.type, 'frame.done', JSON.stringify(swapped.error));
    assert.equal(swapped.source.proxy_layers, 0, 'the proxy of the old asset is not attached to the new one');
    assert.deepEqual(swapped.probes.map(dominantColor), ['black', 'blue', 'blue', 'black'], 'the second asset, not the first');
    assert.equal((await entryOf(runtime)).proxies, 0, 'no proxy frame of the old asset is held');

    // The new asset takes its own proxy and releases it with itself.
    assert.equal((await runtime.attachProxy('seq', proxyOf(second))).state, 'attached');
    const proxied = await runtime.frame(preview(2), { quality: 'preview', probes: PROBES });
    assert.deepEqual(proxied.probes.map(dominantColor), ['black', 'yellow', 'yellow', 'black']);
    await runtime.release('seq');
    assert.equal((await runtime.retention()).texture_entries, 0, 'releasing frees every texture, proxy ones included');
    assert.equal((await glMemory(runtime)).textures, 0, 'and none is left on the GL side');
    assert.equal(await entryOf(runtime), null);
  } finally {
    await runtime.close();
  }
});

test('a frame cancelled in flight leaves nothing behind and the next frame is exact', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const sequences = makeSequences(t);
  const runtime = await lifecycleRuntime(sequences);
  try {
    await runtime.attachProxy('seq', proxyOf(sequences));
    const reference = await runtime.frame(preview(0), { quality: 'preview' });
    const before = await entryOf(runtime);
    // Frame 5 was never fetched: it awaits its bitmap. The cancel arrives in the same turn.
    const victim = frameMessage(preview(5), { quality: 'preview' });
    await runtime.page.evaluate((payload) => {
      window.TesselRuntime.receive(payload);
      window.TesselRuntime.receive({ v: 1, type: 'frame.cancel', id: payload.id });
    }, { v: 1, ...victim });
    const cancelled = await runtime.waitFor(answeredBy(victim.id), 'the cancelled frame');
    assert.equal(cancelled.type, 'frame.failed');
    assert.equal(cancelled.error.code, 'cancelled', JSON.stringify(cancelled.error));
    // The host answers a cancel by asking for the next position; it must be exact.
    const next = await runtime.frame(preview(0), { quality: 'preview' });
    assert.equal(next.hash, reference.hash);
    assert.equal(next.cache.proxy_failed, 0, 'a cancel is not a proxy failure');
    const after = await entryOf(runtime);
    assert.ok(after.proxies <= COUNT && after.originals <= before.originals + 1);
    assert.ok(after.proxy, 'the proxy is still attached');
  } finally {
    await runtime.close();
  }
});

test('a project replaced with a frame in flight: the frame is cancelled, the new project draws its own pixels, the old one is freed', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const first = makeSequences(t, { tone: 'warm' });
  const second = makeSequences(t, { tone: 'cold' });
  const runtime = await lifecycleRuntime(first, { patterns: [second.original, second.proxy] });
  try {
    await runtime.attachProxy('seq', proxyOf(first));
    await runtime.frame(preview(1), { quality: 'preview' });
    // A frame whose proxy bitmap is still being fetched when the old project's assets are released.
    const inFlight = frameMessage(preview(4), { quality: 'preview' });
    await runtime.page.evaluate((payload) => {
      window.TesselRuntime.receive(payload);
      window.TesselRuntime.receive({ v: 1, type: 'asset.release', id: 'seq' });
    }, { v: 1, ...inFlight });
    const answer = await runtime.waitFor(answeredBy(inFlight.id), 'the in-flight frame');
    assert.equal(answer.type, 'frame.failed', 'the old project never gets a picture');
    assert.ok(['cancelled', 'asset_unreadable'].includes(answer.error.code), JSON.stringify(answer.error));
    assert.equal((await runtime.retention()).texture_entries, 0);
    assert.equal((await glMemory(runtime)).textures, 0, 'the old project left no GL texture');
    assert.deepEqual(await runtime.page.evaluate(() => window.TesselRuntime.debug.loadedAssets()), []);

    // The new project reuses the id.
    assert.equal((await runtime.load(sequenceAsset('seq', second.original, COUNT))).state, 'ready');
    const drawn = await runtime.frame(preview(4), { quality: 'preview', probes: PROBES });
    assert.equal(drawn.type, 'frame.done');
    assert.equal(drawn.source.proxy_layers, 0);
    assert.deepEqual(drawn.probes.map(dominantColor), ['black', 'blue', 'blue', 'black']);
    assert.ok(drawn.cache.gl_textures <= 2, `only the new frame's texture is alive (${drawn.cache.gl_textures})`);
  } finally {
    await runtime.close();
  }
});

test('dispose with proxy frames and a frame in flight frees everything and closes the runtime', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const sequences = makeSequences(t);
  const runtime = await lifecycleRuntime(sequences);
  try {
    await runtime.attachProxy('seq', proxyOf(sequences));
    for (let frame = 0; frame < COUNT; frame += 1) await runtime.frame(preview(frame), { quality: 'preview', inspect: false });
    assert.ok((await runtime.retention()).texture_entries > 0);
    const inFlight = frameMessage(preview(5));
    await runtime.page.evaluate((payload) => {
      window.TesselRuntime.receive(payload);
      window.TesselRuntime.receive({ v: 1, type: 'dispose' });
    }, { v: 1, ...inFlight });
    await runtime.waitFor((m) => m.type === 'closed', 'closed');
    const state = await runtime.page.evaluate(() => ({
      loaded: window.TesselRuntime.debug.loadedAssets(),
      retention: window.TesselRuntime.debug.retention(),
    }));
    assert.deepEqual(state.loaded, [], 'no asset survives the runtime');
    assert.equal(state.retention.texture_entries, 0);
    assert.equal(state.retention.text_entries ?? 0, 0);
    assert.equal(state.retention.material_entries ?? 0, 0);
    assert.equal(state.retention.target_entries ?? 0, 0);
    // The in-flight frame is answered or dropped, but never reported as a picture of a dead runtime.
    const late = await Promise.race([
      runtime.waitFor(answeredBy(inFlight.id), 'late answer').catch(() => null),
      new Promise((resolve) => setTimeout(() => resolve(null), 500)),
    ]);
    assert.ok(late === null || late.type === 'frame.failed', JSON.stringify(late?.error ?? late?.type));
  } finally {
    await runtime.close();
  }
});
