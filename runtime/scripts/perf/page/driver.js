// Drives one measurement or proof inside the page, through bridge v1 exactly as a host does
// (`TesselRuntime.receive`), and hands the result to `window.__benchDone`.
//
// Modes (cfg.mode):
//   bench   N frames of one fixed case, cold and warm reported apart (default)
//   hash    pixels of listed frames in the given (shuffled) order: hash, probes, PNG
//   life    same-id asset swap, restore, lost context: pixels must come back identical
//   probe   facts about the clock and `canvas.width =` that the protocol depends on
//   bridge  only loads assets; the native host drives frames (WKWebView round trip)
//
// WHAT IS MEASURED (and what is not):
//   render/prepare/draw/flush  CPU milliseconds the runtime reports in frame.done.timings
//   wall                       page clock from sending frame.render to receiving frame.done
//   drain (cfg.drain)          a 1x1 readPixels (or fence poll) right after the frame: an UPPER
//                              BOUND of "GPU finished" that includes IPC. Separate runs only.
//   Never call any of these "GPU time". WebKit exposes no timer query; Chrome's
//   EXT_disjoint_timer_query_webgl2 is a different engine's number.
(() => {
  const cfg = window.__cfg;
  const dir = cfg.assetsDir;
  const T0 = { x: 0, y: 0, scale: 1, rotation: 0, crop: [0, 0, 0, 0] };

  const still = (id, file) => ({ id, kind: 'still', source: { type: 'file', path: `${dir}/${file}` }, meta: {} });
  const sequence = (id, first = 0) => ({
    id,
    kind: 'video',
    source: { type: 'frame_sequence', pattern: `${dir}/frames/f%05d.png`, first, count: 120 - first },
    meta: { rate: { num: 24, den: 1 } },
  });
  // 1080p "decoded video" (60 frames) and its 540p proxies, JPEG or PNG (assets.mjs, OPT01/OPT06).
  const sequence1080 = (id) => ({
    id,
    kind: 'video',
    source: { type: 'frame_sequence', pattern: `${dir}/frames1080/f%05d.png`, first: 0, count: 60 },
    meta: { rate: { num: 24, den: 1 } },
  });
  const proxyOf = (format) => ({ pattern: `${dir}/proxy-${format}/p%05d.${format}`, first: 0, count: 60 });
  const sceneFile = (file) => ({ id: 'scene', kind: 'scene', source: { type: 'file', path: `${dir}/${file}` }, meta: {} });
  const splatFile = (file) => ({ id: 'splat', kind: 'splat', source: { type: 'file', path: `${dir}/${file}` }, meta: { format: 'splat' } });

  const image = (asset, extra = {}) => ({ id: `l:${asset}`, kind: 'image', asset, opacity: 1, blend: 'normal', transform: T0, ...extra });
  const text = (content) => ({
    id: 'l:t',
    kind: 'text',
    opacity: 1,
    blend: 'normal',
    transform: { ...T0, y: 30 },
    text: { content, size: 8, color: '#ffffff' },
  });
  const video = (frame) => ({
    id: 'l:v',
    kind: 'video',
    asset: 'vid',
    opacity: 1,
    blend: 'normal',
    transform: T0,
    source_frame: frame,
    source_time: { seconds: frame / 24 },
  });
  const camera = (angle) => ({ position: [Math.sin(angle) * 4, 0.3, Math.cos(angle) * 4], target: [0, 0, 0], fov: 50 });
  const scene = (cam, time) => ({
    id: 'l:s',
    kind: 'scene',
    asset: 'scene',
    opacity: 1,
    blend: 'normal',
    transform: T0,
    camera: cam,
    scene_time: { seconds: time },
  });

  // The fixed matrix. `frames` is the declared N of the warm window (splats are slower, so fewer).
  const CASES = {
    image: { assets: () => [still('a', 'image.png')], layers: () => [image('a')], swap: () => still('a', 'image-alt.png') },
    image4k: { assets: () => [still('a', 'image4k.png')], layers: () => [image('a')], swap: () => still('a', 'image-alt.png') },
    'image-2layers': {
      assets: () => [still('a', 'image.png'), still('b', 'image4k.png')],
      layers: () => [image('a'), image('b', { blend: 'screen', opacity: 0.5 })],
      swap: () => still('b', 'image.png'),
    },
    text: { assets: () => [], layers: () => [text('TESSEL EDITOR')] },
    'text-changing': { assets: () => [], layers: (i) => [text(`TC ${String(i % 1000).padStart(3, '0')}`)] },
    'image+text': { assets: () => [still('a', 'image.png')], layers: () => [image('a'), text('TESSEL EDITOR')], swap: () => still('a', 'image-alt.png') },
    'video-hit': { assets: () => [sequence('vid')], layers: (i) => [video(i % 8)], swap: () => sequence('vid', 10) },
    'video-miss': { assets: () => [sequence('vid')], layers: (i) => [video(i % 120)], swap: () => sequence('vid', 10) },
    scene: { assets: () => [sceneFile('box.json')], layers: (i) => [scene(camera(0), i / 24)], swap: () => sceneFile('box-alt.json') },
    'spark-100k': { assets: () => [splatFile('s100k.splat'), sceneFile('s100k.json')], layers: () => [scene(camera(0), 0)], frames: 120 },
    'spark-100k-orbit': { assets: () => [splatFile('s100k.splat'), sceneFile('s100k.json')], layers: (i) => [scene(camera(i * 0.05), 0)], frames: 120 },
    'spark-1m': { assets: () => [splatFile('s1m.splat'), sceneFile('s1m.json')], layers: () => [scene(camera(0), 0)], frames: 90 },
    'spark-1m-orbit': { assets: () => [splatFile('s1m.splat'), sceneFile('s1m.json')], layers: (i) => [scene(camera(i * 0.05), 0)], frames: 90 },
    // Scrub forward (every frame new) and at random (revisits): original vs preview proxy.
    ...Object.fromEntries(
      [['', null], ['-jpg', 'jpg'], ['-png', 'png']].flatMap(([suffix, format]) =>
        [['fwd', (i) => i % 60], ['rand', (i) => (i * 37 + 11) % 60]].map(([order, frameOf]) => [
          `video1080-${order}${suffix}`,
          {
            assets: () => [sequence1080('vid')],
            layers: (i) => [video(frameOf(i))],
            frames: 120,
            ...(format ? { proxy: { asset: 'vid', ...proxyOf(format) }, quality: 'preview' } : {}),
          },
        ]),
      ),
    ),
    'spark-3m': { assets: () => [splatFile('s3m.splat'), sceneFile('s3m.json')], layers: () => [scene(camera(0), 0)], frames: 40 },
    'spark-3m-orbit': { assets: () => [splatFile('s3m.splat'), sceneFile('s3m.json')], layers: (i) => [scene(camera(i * 0.05), 0)], frames: 40 },
  };

  const width = cfg.width ?? 960;
  const height = cfg.height ?? 540;
  const send = (message) => window.TesselRuntime.receive(JSON.stringify({ v: 1, ...message }));
  const until = (predicate) => {
    const index = window.__msgs.findIndex(predicate);
    if (index >= 0) return Promise.resolve(window.__msgs.splice(index, 1)[0]);
    return new Promise((resolve) => window.__waiters.push({ predicate, resolve }));
  };
  const within = (promise, label, ms = 60000) =>
    Promise.race([promise, new Promise((_, reject) => setTimeout(() => reject(new Error(`timeout: ${label}`)), ms))]);
  // A WebView that is occluded or asleep stops delivering rAF; a run would then sit until its
  // timeout. The race keeps the run alive and counts the stalls, so the record says the pacing
  // was not the display's (`raf_fallbacks` > 0 means: do not read this run's fps as rAF-paced).
  let rafFallbacks = 0;
  const nextRaf = () =>
    new Promise((resolve) => {
      let settled = false;
      const settle = (viaTimer) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        if (viaTimer) rafFallbacks += 1;
        resolve();
      };
      const timer = setTimeout(() => settle(true), 250);
      requestAnimationFrame(() => settle(false));
    });
  const yieldTask = () =>
    new Promise((resolve) => {
      const channel = new MessageChannel();
      channel.port1.onmessage = () => resolve();
      channel.port2.postMessage(0);
    });
  const round = (value) => Number(value.toFixed(3));

  const stats = (values) => {
    const sorted = values.filter((value) => Number.isFinite(value)).sort((a, b) => a - b);
    if (!sorted.length) return null;
    const at = (share) => sorted[Math.min(sorted.length - 1, Math.floor(share * sorted.length))];
    return {
      n: sorted.length,
      mean: round(sorted.reduce((sum, value) => sum + value, 0) / sorted.length),
      p50: round(at(0.5)),
      p95: round(at(0.95)),
      p99: round(at(0.99)),
      min: round(sorted[0]),
      max: round(sorted[sorted.length - 1]),
    };
  };

  /** Smallest step of performance.now(): WebKit coarsens it to 1 ms, so sub-ms p50 means nothing. */
  const clockQuantum = () => {
    let smallest = Infinity;
    let last = performance.now();
    const end = last + 30;
    while (last < end) {
      const now = performance.now();
      if (now > last) {
        smallest = Math.min(smallest, now - last);
        last = now;
      }
    }
    return smallest;
  };

  const planFor = (layers, index) => ({
    version: 1,
    canvas: { width, height },
    intent: { mode: 'sequence' },
    frame: index,
    layers,
    requires: [],
    warnings: [],
  });

  let sequenceNumber = 0;
  const gl = () => document.getElementById('viewport').getContext('webgl2');

  /** One frame through the bridge. `inspect: false` is the monitor's interactive path. */
  const renderFrame = async (layers, index, extra = {}) => {
    sequenceNumber += 1;
    const id = sequenceNumber;
    const started = performance.now();
    send({
      type: 'frame.render',
      id,
      revision: id,
      plan: planFor(layers, index),
      output: { width, height },
      ...extra,
    });
    const done = await within(
      until((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === id),
      `frame ${id}`,
    );
    if (done.type === 'frame.failed') throw new Error(`frame ${index}: ${JSON.stringify(done.error)}`);
    return { done, wall: performance.now() - started };
  };

  const drain = async () => {
    const started = performance.now();
    const context = gl();
    if (cfg.drain === 'read1x1') {
      context.readPixels(0, 0, 1, 1, context.RGBA, context.UNSIGNED_BYTE, new Uint8Array(4));
    } else if (cfg.drain === 'fence') {
      const sync = context.fenceSync(context.SYNC_GPU_COMMANDS_COMPLETE, 0);
      context.flush();
      for (;;) {
        const status = context.clientWaitSync(sync, 0, 0);
        if (status === context.ALREADY_SIGNALED || status === context.CONDITION_SATISFIED || status === context.WAIT_FAILED) break;
        await yieldTask();
      }
      context.deleteSync(sync);
    }
    return performance.now() - started;
  };

  const helloOptions = () => {
    const options = {};
    if (typeof cfg.retain === 'boolean') options.retain = cfg.retain;
    if (typeof cfg.result_cache === 'boolean') options.result_cache = cfg.result_cache;
    if (Number.isInteger(cfg.scene_samples)) options.scene_samples = cfg.scene_samples;
    return options;
  };

  const start = async () => {
    send({ type: 'hello', ...helloOptions() });
    const ready = await within(until((m) => m.type === 'ready'), 'ready');
    const result = {
      cfg: { ...cfg, assetsDir: undefined },
      renderer: ready.runtime.renderer,
      runtime: ready.runtime,
      ua: navigator.userAgent,
      clock_quantum_ms: clockQuantum(),
    };
    return result;
  };

  const loadAll = async (assets) => {
    const started = performance.now();
    for (const asset of assets) send({ type: 'asset.load', asset });
    for (const asset of assets) {
      const message = await within(until((m) => m.type === 'asset.state' && m.id === asset.id && m.state !== 'loading'), `asset ${asset.id}`);
      if (message.state !== 'ready') throw new Error(`asset ${asset.id}: ${JSON.stringify(message.error)}`);
    }
    return performance.now() - started;
  };

  const timingsOf = ({ done, wall }) => ({
    wall,
    render: done.timings.render_ms,
    prepare: done.timings.prepare_ms,
    draw: done.timings.draw_ms,
    flush: done.timings.flush_ms ?? done.timings.settle_ms,
    total: done.timings.total_ms,
  });

  const CACHE_EVENTS = [
    'texture_created', 'texture_reused', 'texture_evicted',
    'text_created', 'text_reused', 'text_evicted',
    'material_created', 'material_reused', 'material_evicted',
    'rig_built', 'rig_reused', 'rig_evicted',
    'target_created', 'target_reused', 'target_evicted',
    'result_hit', 'result_miss', 'result_invalidated', 'proxy_attached', 'proxy_failed',
  ];

  const bench = async (result, definition) => {
    const warmup = cfg.warmup ?? 10;
    const frames = cfg.frames ?? definition.frames ?? 300;
    result.asset_load_ms = await loadAll(definition.assets());
    if (definition.proxy) {
      const { asset, ...proxy } = definition.proxy;
      send({ type: 'asset.proxy', id: asset, proxy });
      const attached = await within(until((m) => m.type === 'asset.proxy' && m.id === asset), 'asset.proxy');
      if (attached.state !== 'attached') throw new Error(`proxy: ${JSON.stringify(attached.error)}`);
    }
    // `quality` is the monitor's: only the proxy cases ask for `preview`.
    const base = definition.quality ? { inspect: false, quality: definition.quality } : { inspect: false };
    result.declared = { warmup, frames, width, height, pacing: cfg.pacing ?? 'raf', drain: cfg.drain ?? null, quality: definition.quality ?? 'original' };
    const pace = async () => {
      if ((cfg.pacing ?? 'raf') === 'raf') {
        const started = performance.now();
        await nextRaf();
        return performance.now() - started;
      }
      await yieldTask();
      return 0;
    };

    // Cold: a page that never drew. Frame 0 compiles and uploads, frame 1 shows what is left.
    const cold = [];
    for (let index = 0; index < warmup; index += 1) {
      const coldFrame = await renderFrame(definition.layers(index), index, base);
      if (coldFrame.done.source?.proxy_error) result.proxy_error ??= coldFrame.done.source.proxy_error;
      cold.push(timingsOf(coldFrame));
      await pace();
    }
    result.cold_first = cold[0];
    result.cold_second = cold[1] ?? null;

    for (const key of Object.keys(window.__counts)) {
      delete window.__counts[key];
      delete window.__times[key];
    }
    const rows = [];
    const cacheSum = Object.fromEntries(CACHE_EVENTS.map((name) => [name, 0]));
    let lastCache = null;
    const windowStart = performance.now();
    for (let index = warmup; index < warmup + frames; index += 1) {
      const frame = await renderFrame(definition.layers(index), index, base);
      const row = timingsOf(frame);
      row.source = frame.done.source?.proxy_layers ? 'proxy' : 'original';
      if (frame.done.source?.proxy_error) result.proxy_error ??= frame.done.source.proxy_error;
      if (cfg.drain) {
        row.drain = await drain();
        // frame.done -> drained: the only figure comparable between a path that waits for the GPU
        // inside the frame (finish) and one that does not (flush). Upper bound, includes IPC.
        row.complete = row.wall + row.drain;
      }
      row.raf_wait = await pace();
      rows.push(row);
      if (frame.done.cache) {
        lastCache = frame.done.cache;
        for (const name of CACHE_EVENTS) cacheSum[name] += frame.done.cache[name] ?? 0;
      }
    }
    result.window_ms = performance.now() - windowStart;
    result.raf_fallbacks = rafFallbacks;
    result.frames_per_second = frames / (result.window_ms / 1000);
    for (const key of ['wall', 'render', 'prepare', 'draw', 'flush', 'total', 'drain', 'complete', 'raf_wait']) {
      result[key] = stats(rows.map((row) => row[key]));
    }
    // The raw series stays in the record: a mean can be recomputed, a hidden outlier cannot.
    result.series = { render: rows.map((row) => round(row.render)), wall: rows.map((row) => round(row.wall)) };
    result.sources = { proxy_frames: rows.filter((row) => row.source === 'proxy').length, original_frames: rows.filter((row) => row.source === 'original').length };
    result.cache_events = lastCache ? cacheSum : null;
    result.cache_last = lastCache;
    if (cfg.count) {
      result.per_frame = {};
      result.ms_per_frame = {};
      for (const key of Object.keys(window.__counts).sort()) {
        result.per_frame[key] = Number((window.__counts[key] / frames).toFixed(2));
        result.ms_per_frame[key] = Number((window.__times[key] / frames).toFixed(3));
      }
    }
    return result;
  };

  const probesOf = () => [
    [1, 1],
    [Math.floor(width / 2), Math.floor(height / 2)],
    [width - 2, height - 2],
  ];

  const stringHash = (value) => {
    let hash = 0x811c9dc5;
    for (let index = 0; index < value.length; index += 1) {
      hash ^= value.charCodeAt(index);
      hash = Math.imul(hash, 0x01000193) >>> 0;
    }
    return hash.toString(16).padStart(8, '0');
  };

  /** Explicit render: pixels hash, probes and the PNG (what export and proofs read). */
  const explicit = async (definition, index) => {
    const { done } = await renderFrame(definition.layers(index), index, { probes: probesOf() });
    return { index, hash: done.hash, probes: done.probes, mean: done.mean };
  };

  const explicitPng = async (definition, index) => {
    sequenceNumber += 1;
    const id = sequenceNumber;
    send({
      type: 'frame.render',
      id,
      revision: id,
      plan: planFor(definition.layers(index), index),
      output: { width, height, png: '/dev/null/ignored.png' },
      probes: [],
    });
    const done = await within(until((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === id), `png ${id}`);
    if (done.type === 'frame.failed') throw new Error(`png ${index}: ${JSON.stringify(done.error)}`);
    return { index, hash: done.hash, png: stringHash(done.png_data ?? '') };
  };

  const hashes = async (result, definition) => {
    result.asset_load_ms = await loadAll(definition.assets());
    result.hashes = [];
    for (const index of cfg.order) {
      result.hashes.push(await explicit(definition, index));
      await nextRaf();
    }
    // A monitor frame (no readback, then an explicit one) must not change what an explicit one shows.
    await renderFrame(definition.layers(cfg.order[0]), cfg.order[0], { inspect: false });
    result.after_interactive = await explicit(definition, cfg.order[0]);
    result.png = await explicitPng(definition, cfg.order[0]);
    return result;
  };

  const lifecycle = async (result, definition) => {
    const at = async (name, work) => {
      try {
        result.steps[name] = await within(work(), name);
      } catch (error) {
        result.steps[name] = `ERROR ${error.message}`;
      }
    };
    result.steps = {};
    result.asset_load_ms = await loadAll(definition.assets());
    const hash = async (index) => (await explicit(definition, index)).hash;
    await at('before', () => hash(0));
    await at('before2', () => hash(1));
    const swap = definition.swap?.();
    if (swap) {
      await at('swap_load', async () => {
        send({ type: 'asset.load', asset: swap });
        return (await until((m) => m.type === 'asset.state' && m.id === swap.id && m.state !== 'loading')).state;
      });
      await at('after_swap', () => hash(0));
      await at('restore_load', async () => {
        const original = definition.assets().find((asset) => asset.id === swap.id);
        send({ type: 'asset.load', asset: original });
        return (await until((m) => m.type === 'asset.state' && m.id === original.id && m.state !== 'loading')).state;
      });
      await at('after_restore', () => hash(0));
    }
    await at('lost', async () => {
      window.TesselRuntime.debug.loseContext();
      return (await until((m) => m.type === 'context.lost')).type;
    });
    await at('restored', async () => {
      await new Promise((resolve) => setTimeout(resolve, 100));
      window.TesselRuntime.debug.restoreContext();
      return (await until((m) => m.type === 'context.restored')).type;
    });
    await at('after_context', () => hash(0));
    await at('after_context2', () => hash(1));
    await at('release', async () => {
      for (const asset of definition.assets()) send({ type: 'asset.release', id: asset.id });
      return JSON.stringify(window.TesselRuntime.debug.retention?.() ?? null);
    });
    return result;
  };

  const probe = async (result) => {
    const canvas = document.getElementById('viewport');
    const context = gl();
    result.isolated = self.crossOriginIsolated;
    result.extensions = (context.getSupportedExtensions() ?? []).filter((name) => /timer|debug|disjoint/i.test(name));
    canvas.width = width;
    canvas.height = height;
    const started = performance.now();
    for (let index = 0; index < 300; index += 1) {
      canvas.width = width;
      canvas.height = height;
    }
    result.canvas_same_size_us = ((performance.now() - started) / 300) * 1000;
    context.bindFramebuffer(context.FRAMEBUFFER, null);
    context.viewport(0, 0, width, height);
    context.clearColor(1, 0, 0, 1);
    context.clear(context.COLOR_BUFFER_BIT);
    canvas.width = width;
    const pixel = new Uint8Array(4);
    context.readPixels(0, 0, 1, 1, context.RGBA, context.UNSIGNED_BYTE, pixel);
    result.pixel_after_same_width = [...pixel];
    return result;
  };

  const main = async () => {
    const result = await start();
    if (cfg.mode === 'probe') return probe(result);
    const definition = CASES[cfg.case];
    if (!definition) throw new Error(`unknown case ${cfg.case}`);
    if (cfg.mode === 'hash') return hashes(result, definition);
    if (cfg.mode === 'life') return lifecycle(result, definition);
    if (cfg.mode === 'bridge') {
      result.asset_load_ms = await loadAll(definition.assets());
      window.__planJson = (index) =>
        JSON.stringify({
          v: 1,
          type: 'frame.render',
          id: index + 1,
          revision: index + 1,
          plan: planFor(definition.layers(index), index),
          output: { width, height },
          inspect: false,
        });
      return { ...result, bridgeReady: true };
    }
    return bench(result, definition);
  };

  main()
    .then((result) => window.__benchDone({ ...result, errors: window.__errs }))
    .catch((error) => window.__benchDone({ cfg: { ...cfg, assetsDir: undefined }, fatal: String(error?.stack ?? error), errors: window.__errs }));
})();
