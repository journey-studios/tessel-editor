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
  const scene = (cam, time, extra = {}) => ({
    id: 'l:s',
    kind: 'scene',
    asset: 'scene',
    opacity: 1,
    blend: 'normal',
    transform: T0,
    camera: cam,
    scene_time: { seconds: time },
    ...extra,
  });
  // Two different 1M clouds faded into each other, the way two overlapping scene clips are: both are drawn in every
  // frame. The ramp stays inside (0.15, 0.85) so neither layer is ever skipped for being invisible.
  const crossfade = (i) => {
    const phase = (i % 28) / 28;
    const ramp = 0.15 + 0.7 * (phase < 0.5 ? phase * 2 : 2 - phase * 2);
    return [
      scene(camera(i * 0.01), 0, { id: 'l:s1', asset: 'scene', opacity: 1 - ramp }),
      scene(camera(i * 0.01 + 1), 0, { id: 'l:s2', asset: 'scene-b', opacity: ramp }),
    ];
  };
  // The entry of a crossfade: one cloud alone, then (at frame ENTRY_AT) a second one that is loaded but was never drawn.
  // With `cfg.entry_warm !== false` the page is sent the `frame.warm` the monitor sends a moment ahead of a cut
  // (`cfg.warm_lead` frames before it), so the entry is measured the way the editor reaches it.
  const ENTRY_AT = 40;
  const entryLayers = (i) => (i < ENTRY_AT ? [scene(camera(i * 0.004), 0, { id: 'l:s1' })] : crossfade(i - ENTRY_AT));
  const entryBefore = (i) =>
    cfg.entry_warm === false || i !== ENTRY_AT - (cfg.warm_lead ?? 30)
      ? []
      : [{ type: 'frame.warm', plan: planFor(entryLayers(ENTRY_AT), ENTRY_AT) }];
  // BL157.4, real clouds (private: passed on the command line and linked into `real/`, never copied): a scene is a ~1.18M-splat cloud
  // plus the two small spinning effect clouds a project puts in each scene, seen by a camera that sits inside it and orbits slowly.
  const realFile = (file) => `${dir}/real/${file}`;
  const realSplat = (id, file, count) => ({ id, kind: 'splat', source: { type: 'file', path: realFile(file) }, meta: { format: 'ply', splat_count: count } });
  const realScene = (id, file) => ({ id, kind: 'scene', source: { type: 'file', path: realFile(file) }, meta: { format: 'tessel-scene' } });
  const realAssets = () => [
    realSplat('cloud-a', 'cloud-a.ply', 1179648),
    realSplat('cloud-b', 'cloud-b.ply', 1179648),
    realSplat('ring', 'ring.ply', 52000),
    realSplat('sparks', 'sparks.ply', 1400),
    realScene('scene-a', 'scene-a.json'),
    realScene('scene-b', 'scene-b.json'),
  ];
  const INSIDE = { position: [-0.11, 0, -0.05], target: [0, -0.14, -2.24], fov: 37, orbit: 0.72 };
  /** `Camera::at` of the core: the position turned around the target's Y axis by `orbit` degrees per second. */
  const orbitAt = (base, seconds) => {
    const angle = (base.orbit * seconds * Math.PI) / 180;
    const sin = Math.sin(angle);
    const cos = Math.cos(angle);
    const dx = base.position[0] - base.target[0];
    const dz = base.position[2] - base.target[2];
    return { ...base, position: [base.target[0] + dx * cos + dz * sin, base.position[1], base.target[2] - dx * sin + dz * cos] };
  };
  const realLayer = (id, asset, seconds, opacity) => scene(orbitAt(INSIDE, seconds), seconds, { id, asset, opacity });
  const realOne = (i) => [realLayer('l:a', 'scene-a', i / 24, 1)];
  const REAL_ENTRY_AT = 40;
  const FADE_FRAMES = 14;
  /** A scene alone; then a second one fades in over it for 14 frames (the project's `fade_in`), then it stands alone. */
  const realEntryLayers = (i) => {
    const since = i - REAL_ENTRY_AT;
    if (since < 0) return [realLayer('l:a', 'scene-a', i / 24, 1)];
    const second = realLayer('l:b', 'scene-b', since / 24, Math.min(1, since / FADE_FRAMES));
    return since < FADE_FRAMES ? [realLayer('l:a', 'scene-a', i / 24, 1), second] : [second];
  };
  /** The same scenes without a fade: the second one replaces the first at the entry (no layer of the first is drawn with it). */
  const realCutLayers = (i) => (i < REAL_ENTRY_AT ? [realLayer('l:a', 'scene-a', i / 24, 1)] : [realLayer('l:b', 'scene-b', (i - REAL_ENTRY_AT) / 24, 1)]);
  /** A scene alone; then a still fades in over it (the crossfade of a layer that has no splat stage). */
  const realFadeImageLayers = (i) => {
    const since = i - REAL_ENTRY_AT;
    if (since < 0) return [realLayer('l:a', 'scene-a', i / 24, 1)];
    const picture = image('pic', { opacity: Math.min(1, since / FADE_FRAMES) });
    return since < FADE_FRAMES ? [realLayer('l:a', 'scene-a', i / 24, 1), picture] : [picture];
  };
  // The whole sequence of the real project: four scenes, each faded into the one before (14 frames) at a different entry. Every
  // entry is a cloud never drawn before in this process, so what the first one costs and what the next ones cost can be told apart.
  const CHAIN = ['a', 'b', 'c', 'd'];
  const CHAIN_AT = [40, 100, 160];
  // `cuts` names the entries that are plain cuts (no fade): the others are the 14-frame crossfades.
  const chainOf = (cuts = []) => {
    const layersAt = (i) => {
      let k = 0;
      while (k < CHAIN_AT.length && i >= CHAIN_AT[k]) k += 1;
      const clip = (n, opacity) => realLayer(`l:${CHAIN[n]}`, `scene-${CHAIN[n]}`, (i - (n === 0 ? 0 : CHAIN_AT[n - 1])) / 24, opacity);
      if (k === 0) return [clip(0, 1)];
      const since = i - CHAIN_AT[k - 1];
      return since < FADE_FRAMES && !cuts.includes(k - 1) ? [clip(k - 1, 1), clip(k, since / FADE_FRAMES)] : [clip(k, 1)];
    };
    const before = (i) =>
      cfg.entry_warm === false ? [] : CHAIN_AT.filter((at) => i === at - (cfg.warm_lead ?? 30)).map((at) => ({ type: 'frame.warm', plan: planFor(layersAt(at), at) }));
    return { layers: layersAt, before };
  };
  // The chain played twice in one page: at frame LOOP the playhead goes back to the start (a replay, or a seek back), where the stages
  // of the first scenes are no longer alive (the retention keeps two), so every entry of the second play is a stage made anew.
  const LOOP = 220;
  const chainLoop = () => {
    const base = chainOf();
    const entries = [...CHAIN_AT, ...CHAIN_AT.map((at) => at + LOOP)];
    const layers = (i) => base.layers(i % LOOP);
    const before = (i) =>
      cfg.entry_warm === false ? [] : entries.filter((at) => i === at - (cfg.warm_lead ?? 30)).map((at) => ({ type: 'frame.warm', plan: planFor(layers(at), at) }));
    return { layers, before, entries };
  };
  const chainLayers = chainOf().layers;
  const chainBefore = chainOf().before;
  const chainAssets = () => [
    ...CHAIN.map((name) => realSplat(`cloud-${name}`, `cloud-${name}.ply`, 1179648)),
    realSplat('ring', 'ring.ply', 52000),
    realSplat('sparks', 'sparks.ply', 1400),
    ...CHAIN.map((name) => realScene(`scene-${name}`, `scene-${name}.json`)),
  ];
  const realEntryBefore = (i) =>
    cfg.entry_warm === false || i !== REAL_ENTRY_AT - (cfg.warm_lead ?? 30)
      ? []
      : [{ type: 'frame.warm', plan: planFor(realEntryLayers(REAL_ENTRY_AT), REAL_ENTRY_AT) }];
  // The same crossfade entry at another frame of the run: how long the page has been drawing before the first second scene.
  const realEntryOf = (at) => {
    const layers = (i) => {
      const since = i - at;
      if (since < 0) return [realLayer('l:a', 'scene-a', i / 24, 1)];
      const second = realLayer('l:b', 'scene-b', since / 24, Math.min(1, since / FADE_FRAMES));
      return since < FADE_FRAMES ? [realLayer('l:a', 'scene-a', i / 24, 1), second] : [second];
    };
    const before = (i) => (cfg.entry_warm === false || i !== at - (cfg.warm_lead ?? 30) ? [] : [{ type: 'frame.warm', plan: planFor(layers(at), at) }]);
    return { layers, before };
  };
  // The load of the next scene while the playhead moves (the monitor's prefetch, a moment ahead of a cut): frames of one scene keep
  // being asked for, and the second scene's cloud and scene file are loaded meanwhile. What this shows is the jank the load itself
  // causes, nothing of the entry (the scene is never drawn).
  const REAL_PREFETCH_AT = 20;
  const LATE = new Set(['cloud-b', 'scene-b']);
  const realPrefetchBefore = (i) => (i === REAL_PREFETCH_AT ? realAssets().filter((asset) => LATE.has(asset.id)).map((asset) => ({ type: 'asset.load', asset })) : []);
  const crossfadeAssets = () => [
    splatFile('s1m.splat'),
    sceneFile('s1m.json'),
    { id: 'splat-b', kind: 'splat', source: { type: 'file', path: `${dir}/s1m-b.splat` }, meta: { format: 'splat' } },
    { id: 'scene-b', kind: 'scene', source: { type: 'file', path: `${dir}/s1m-b.json` }, meta: {} },
  ];

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
    // One cloud moving slowly (the pace of a camera move in a film) and two clouds crossfading: the two shapes of a frame
    // of the real project that are drawn with splats.
    'spark-1m-slow': { assets: () => [splatFile('s1m.splat'), sceneFile('s1m.json')], layers: (i) => [scene(camera(i * 0.004), 0)], frames: 90 },
    'spark-1m-xfade': { assets: crossfadeAssets, layers: crossfade, frames: 90 },
    // BL157.4: the frames around the entry of a crossfade (index ENTRY_AT of the run), series kept per frame.
    'spark-1m-entry': { assets: crossfadeAssets, layers: entryLayers, before: entryBefore, frames: 80, series: true, entryAt: ENTRY_AT },
    // The same, with the real clouds (needs `--real-*`): one scene, and the entry of the next one over it.
    'real-one': { assets: realAssets, layers: realOne, frames: 60 },
    'real-cut': { assets: realAssets, layers: realCutLayers, before: (i) => (cfg.entry_warm === false || i !== REAL_ENTRY_AT - (cfg.warm_lead ?? 30) ? [] : [{ type: 'frame.warm', plan: planFor(realCutLayers(REAL_ENTRY_AT), REAL_ENTRY_AT) }]), frames: 90, series: true, entryAt: REAL_ENTRY_AT },
    // The `frame.warm` the monitor sends ahead of the fade also asks for the textures of its stills when the variant is `preupload`.
    'real-fade-image': {
      assets: () => [...realAssets(), { id: 'pic', kind: 'still', source: { type: 'file', path: realFile('image.png') }, meta: {} }],
      layers: realFadeImageLayers,
      before: (i) =>
        cfg.entry_warm === false || i !== REAL_ENTRY_AT - (cfg.warm_lead ?? 30)
          ? []
          : [{ type: 'frame.warm', plan: planFor(realFadeImageLayers(REAL_ENTRY_AT), REAL_ENTRY_AT), ...(cfg.warm_textures ? { textures: true } : {}) }],
      frames: 90,
      series: true,
      entryAt: REAL_ENTRY_AT,
    },
    // The load alone: how long it takes and whether it always ends (the real clouds, decoded on Spark's workers).
    'real-load': { assets: realAssets, layers: realOne, frames: 1, loadOnly: true },
    // The same, with eight more of each small effect cloud (the same files under other ids): sixteen small decodes in one load, so a
    // fault that hits a small decode now and then shows in a few dozen runs instead of a few hundred.
    'real-load-many': {
      assets: () => [
        realSplat('cloud-a', 'cloud-a.ply', 1179648),
        realSplat('cloud-b', 'cloud-b.ply', 1179648),
        ...Array.from({ length: 8 }, (_, i) => realSplat(`ring-${i}`, 'ring.ply', 52000)),
        ...Array.from({ length: 8 }, (_, i) => realSplat(`sparks-${i}`, 'sparks.ply', 1400)),
      ],
      layers: realOne,
      frames: 1,
      loadOnly: true,
    },
    // The first entry is a cut: the first frame with two scenes in it is then the second entry (what the first crossfade of a process costs).
    'real-chain-cut': { assets: chainAssets, layers: chainOf([0]).layers, before: chainOf([0]).before, frames: 230, series: true, entryAt: CHAIN_AT[0], entries: CHAIN_AT },
    ...Object.fromEntries(
      [100, 186].map((at) => {
        const entry = realEntryOf(at);
        return [`real-entry-${at}`, { assets: realAssets, layers: entry.layers, before: entry.before, frames: at + 40, series: true, entryAt: at }];
      }),
    ),
    'real-chain-loop': { assets: chainAssets, layers: chainLoop().layers, before: chainLoop().before, frames: LOOP * 2 - 10, series: true, entryAt: CHAIN_AT[0], entries: chainLoop().entries },
    'real-chain': { assets: chainAssets, layers: chainLayers, before: chainBefore, frames: 230, series: true, entryAt: CHAIN_AT[0], entries: CHAIN_AT },
    'real-prefetch': { assets: () => realAssets().filter((asset) => !LATE.has(asset.id)), layers: realOne, before: realPrefetchBefore, frames: 120, series: true, entryAt: REAL_PREFETCH_AT },
    'real-entry': { assets: realAssets, layers: realEntryLayers, before: realEntryBefore, frames: 90, series: true, entryAt: REAL_ENTRY_AT },
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
    window.__frame = index;
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
    window.__frame = -1;
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
    if (Number.isInteger(cfg.preview_scene_samples)) options.preview_scene_samples = cfg.preview_scene_samples;
    if (typeof cfg.preview_sort === 'string') options.preview_sort = cfg.preview_sort;
    if (typeof cfg.preupload_textures === 'boolean') options.preupload_textures = cfg.preupload_textures;
    if (typeof cfg.warm_draw === 'boolean') options.warm_draw = cfg.warm_draw;
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
      const message = await within(until((m) => m.type === 'asset.state' && m.id === asset.id && m.state !== 'loading'), `asset ${asset.id}`, cfg.load_timeout_ms ?? 60000);
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
    // A case about the load alone (does it always end, and how long does it take): nothing is drawn.
    if (definition.loadOnly) return result;
    if (definition.proxy) {
      const { asset, ...proxy } = definition.proxy;
      send({ type: 'asset.proxy', id: asset, proxy });
      const attached = await within(until((m) => m.type === 'asset.proxy' && m.id === asset), 'asset.proxy');
      if (attached.state !== 'attached') throw new Error(`proxy: ${JSON.stringify(attached.error)}`);
    }
    // `cfg.prime_open`: the warm of the case's first transition is sent once, now, and the page idles for `prime_wait_ms` (the monitor
    // warming when the project is opened, before anybody presses play). The warm ahead of the transition still follows, as the monitor
    // sends it, unless `cfg.prime_only` (`sendBefore`).
    if (cfg.prime_open && definition.entryAt !== undefined) {
      for (const message of definition.before?.(definition.entryAt - (cfg.warm_lead ?? 30)) ?? []) send(message);
      await new Promise((resolve) => setTimeout(resolve, cfg.prime_wait_ms ?? 1500));
    }
    // `quality` is the monitor's: only the proxy cases ask for `preview` (and `--quality preview` asks for it for any case: the
    // monitor draws a moving playhead with it).
    const quality = cfg.quality ?? definition.quality;
    const base = quality ? { inspect: false, quality } : { inspect: false };
    result.declared = { warmup, frames, width, height, pacing: cfg.pacing ?? 'raf', drain: cfg.drain ?? null, quality: quality ?? 'original' };
    // `raf` waits for the next animation frame, `film` keeps the clock of a playing project (24 frames a second: a frame that is late
    // does not make the next one early), anything else only yields.
    let filmNext = 0;
    const pace = async () => {
      const mode = cfg.pacing ?? 'raf';
      if (mode === 'raf') {
        const started = performance.now();
        await nextRaf();
        return performance.now() - started;
      }
      if (mode === 'film') {
        const now = performance.now();
        if (filmNext === 0) filmNext = now;
        filmNext += 1000 / 24;
        const wait = filmNext - now;
        if (wait <= 0) {
          filmNext = now;
          return 0;
        }
        await new Promise((resolve) => setTimeout(resolve, wait));
        return wait;
      }
      await yieldTask();
      return 0;
    };

    // Messages a case sends between frames (a warm, a load): a load also records how long it took to answer, and at which frame it began.
    const sendBefore = (index) => {
      for (const message of cfg.prime_only ? [] : (definition.before?.(index) ?? [])) {
        send(message);
        if (message.type !== 'asset.load') continue;
        const started = performance.now();
        const note = ((result.late ??= {})[message.asset.id] = { sent_at_frame: index, ms: null, state: null });
        void until((m) => m.type === 'asset.state' && m.id === message.asset.id && m.state !== 'loading').then((m) => {
          note.ms = round(performance.now() - started);
          note.state = m.state;
          note.timings = m.timings ?? null;
        });
      }
    };

    // Cold: a page that never drew. Frame 0 compiles and uploads, frame 1 shows what is left.
    const cold = [];
    for (let index = 0; index < warmup; index += 1) {
      sendBefore(index);
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
    // A page whose main thread was busy: a timer that asks to run every 4 ms and is late by `BUSY_LATE_MS` or more records WHEN and by how much.
    // (A case about one event only; the timer itself costs nothing a frame notices.) At 12 ms the list held nearly every frame of a
    // 24 fps run (13-27 ms late each, the frame's own work); 30 ms keeps what stands out of that: a stall of most of a frame.
    const BUSY_LATE_MS = 30;
    const gaps = [];
    let monitoring = Boolean(definition.series);
    if (monitoring) {
      let expected = performance.now() + 4;
      const tick = () => {
        if (!monitoring) return;
        const now = performance.now();
        if (now - expected >= BUSY_LATE_MS) gaps.push({ at: round(expected - windowStart), late_ms: round(now - expected) });
        expected = now + 4;
        setTimeout(tick, 4);
      };
      setTimeout(tick, 4);
    }
    for (let index = warmup; index < warmup + frames; index += 1) {
      sendBefore(index);
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
    monitoring = false;
    if (definition.series) result.gaps = gaps;
    if (window.__slow) result.slow_gl = window.__slow;
    if (window.__pso) result.pso = window.__pso;
    result.window_ms = performance.now() - windowStart;
    result.raf_fallbacks = rafFallbacks;
    result.frames_per_second = frames / (result.window_ms / 1000);
    for (const key of ['wall', 'render', 'prepare', 'draw', 'flush', 'total', 'drain', 'complete', 'raf_wait']) {
      result[key] = stats(rows.map((row) => row[key]));
    }
    // The raw series stays in the record: a mean can be recomputed, a hidden outlier cannot.
    result.series = { render: rows.map((row) => round(row.render)), wall: rows.map((row) => round(row.wall)) };
    if (definition.series) {
      // A case about the frames around one event: every phase of every frame, in order (a mean would hide the one frame that matters).
      for (const key of ['prepare', 'draw', 'drain', 'complete', 'raf_wait']) result.series[key] = rows.map((row) => round(row[key] ?? 0));
      result.first_index = warmup;
      result.entry_at = definition.entryAt ?? null;
      result.entries = definition.entries ?? null;
      // One explicit frame after the window: proof that the last picture is not an empty one (a run that draws nothing is fast too).
      const last = warmup + frames - 1;
      const { done } = await renderFrame(definition.layers(last), last, { probes: probesOf() });
      result.last_frame = { index: last, hash: done.hash, probes: done.probes, mean: done.mean };
    }
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

  // `cfg.end_hold_ms`: keeps the page (and its processes) alive after the run, so a sampler attached to them (macOS `sample`) can
  // finish before they exit; a process that dies under a sampler leaves an empty profile.
  const hold = () => (cfg.end_hold_ms ? new Promise((resolve) => setTimeout(resolve, cfg.end_hold_ms)) : Promise.resolve());
  // `cfg.workerlog` (preamble): the Workers and fetches of the page, in the record of a run that ended and of one that was given up.
  const diagnostics = () => ({
    ...(window.__workers ? { workers: window.__workers } : {}),
    ...(window.__fetches ? { fetches: window.__fetches } : {}),
    ...(cfg.workerlog ? { unread: window.__msgs.map((message) => `${message.type}:${message.id ?? ''}:${message.state ?? ''}`) } : {}),
  });
  main()
    .then(async (result) => {
      await hold();
      window.__benchDone({ ...result, errors: window.__errs, ...diagnostics() });
    })
    // WebKit's `stack` has no message: the label of a timeout (`frame 12`, `asset x`) would be lost with it. `state` says how far the run got.
    .catch((error) =>
      window.__benchDone({
        cfg: { ...cfg, assetsDir: undefined },
        fatal: `${error?.message ?? error}\n${error?.stack ?? ''}`,
        state: { sequence: sequenceNumber, unread_messages: window.__msgs.map((message) => message.type) },
        errors: window.__errs,
        ...diagnostics(),
      }));
})();
