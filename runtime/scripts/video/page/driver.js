// Drives the real runtime through its bridge with barcoded video clips.
//
// A run (`cfg`) is { name, variant, mode, clip, ... } and the page reports one JSON record through
// `__vvFinish`. Plans mirror what crates/core produces: `source_frame` and the exact rational
// `source_time` of a layer, asset meta as `Asset::to_json` writes it.
//
//   mode "verify"  render the frames of `order`, read the 10-bit code of each picture back and
//                  compare it with the frame that was asked for (0 wrong pictures is the claim)
//   mode "paced"   input at a fixed rate, one frame in flight, newest wins (what a scrub does);
//                  reports latency percentiles and the slowest frames with their runtime timings
//   mode "raw"     the browser's own <video> without the runtime: which way of reading it shows the
//                  right picture (drawImage, createImageBitmap, WebGL upload) and how long each takes
//   mode "caps"    what the engine says it can decode (informational, never a proof)
const cfg = window.__cfg;
const { clip } = cfg;
const T0 = { x: 0, y: 0, scale: 1, rotation: 0, crop: [0, 0, 0, 0] };
const CANVAS = cfg.canvas ?? [clip?.width ?? 640, clip?.height ?? 360];

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const nextRaf = () => new Promise((resolve) => requestAnimationFrame(resolve));
const until = (predicate, ms) => {
  const found = __msgs.findIndex(predicate);
  if (found >= 0) return Promise.resolve(__msgs.splice(found, 1)[0]);
  return new Promise((resolve, reject) => {
    const waiter = { predicate, resolve };
    __waiters.push(waiter);
    if (!ms) return;
    setTimeout(() => {
      const at = __waiters.indexOf(waiter);
      if (at < 0) return;
      __waiters.splice(at, 1);
      reject(new Error('timeout'));
    }, ms);
  });
};
const send = (message) => TesselRuntime.receive(JSON.stringify({ v: 1, ...message }));

const stats = (values) => {
  const sorted = values.filter((value) => Number.isFinite(value)).sort((a, b) => a - b);
  if (!sorted.length) return null;
  const at = (share) => sorted[Math.min(sorted.length - 1, Math.floor(share * sorted.length))];
  const round = (value) => Number(value.toFixed(2));
  return {
    n: sorted.length,
    mean: round(sorted.reduce((sum, value) => sum + value, 0) / sorted.length),
    p50: round(at(0.5)),
    p95: round(at(0.95)),
    p99: round(at(0.99)),
    max: round(sorted[sorted.length - 1]),
  };
};

/** Deterministic pseudo-random sequence, so a failing order can be replayed. */
const lcg = (seed) => {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 4294967296;
  };
};

const ORDERS = {
  forward: (count, n) => Array.from({ length: n }, (_, i) => i % count),
  reverse: (count, n) => Array.from({ length: n }, (_, i) => count - 1 - (i % count)),
  random: (count, n, seed = 7) => {
    const random = lcg(seed);
    return Array.from({ length: n }, () => Math.floor(random() * count));
  },
  // first, last, first, last, the middle, then the neighbours of both ends: the boundaries are
  // where an off-by-one frame or a clamp shows first
  edges: (count) => [0, count - 1, 0, count - 1, count >> 1, 1, count - 2, 0, count - 1, count >> 1],
  scrub: (count, n, seed = 7) => {
    const random = lcg(seed);
    let position = count >> 1;
    let step = 1;
    return Array.from({ length: n }, () => {
      if (random() < 0.08) step = -step;
      position += step * (1 + Math.floor(random() * 4));
      if (position < 0) [position, step] = [0, 1];
      if (position >= count) [position, step] = [count - 1, -1];
      return position;
    });
  },
};

const rateOf = () => {
  if (typeof clip.rate === 'object') return clip.rate;
  const [num, den = 1] = String(clip.rate).split('/').map(Number);
  return { num, den };
};

const assetOf = () => {
  const rate = rateOf();
  const meta = {
    width: clip.width,
    height: clip.height,
    rate: { ...rate, fps: rate.num / rate.den },
    duration: { num: clip.frames * rate.den, den: rate.num, seconds: (clip.frames * rate.den) / rate.num },
    ...(cfg.meta ?? {}),
  };
  if (clip.pattern) {
    return { id: 'vid', kind: 'video', source: { type: 'frame_sequence', pattern: clip.pattern, first: 0, count: clip.frames }, meta };
  }
  return { id: 'vid', kind: 'video', source: { type: 'file', path: clip.path }, meta };
};

/** Layer for frame `f`, the way `asset_layer` in crates/core/src/plan.rs writes it. */
const layerOf = (frame, extraFrames = 0) => {
  const { num, den } = rateOf();
  // `extraFrames` shifts source_time only (never source_frame) to reproduce a hypothesis
  const ticks = frame * 2 + extraFrames * 2;
  const seconds = (ticks * den) / (2 * num);
  return {
    id: 'l:v',
    kind: 'video',
    asset: 'vid',
    opacity: 1,
    blend: 'normal',
    transform: T0,
    source_frame: frame,
    source_time: { num: ticks * den, den: 2 * num, seconds },
  };
};

let nextId = 1;
const render = async (layers, { inspect = false, probes, png = false, timeout = 30000, frame = 0 } = {}) => {
  const id = nextId++;
  const started = performance.now();
  const [width, height] = CANVAS;
  send({
    type: 'frame.render',
    id,
    revision: id,
    plan: { version: 1, canvas: { width, height }, intent: { mode: 'sequence' }, frame, layers, requires: [], warnings: [] },
    output: { width, height, ...(png ? { png: 'x' } : {}) },
    inspect,
    ...(probes ? { probes } : {}),
  });
  const done = await until((message) => (message.type === 'frame.done' || message.type === 'frame.failed') && message.id === id, timeout);
  done.wall = performance.now() - started;
  return done;
};

const loadClip = async (res) => {
  const started = performance.now();
  send({ type: 'asset.load', asset: assetOf() });
  const state = await until((message) => message.type === 'asset.state' && message.id === 'vid' && message.state !== 'loading', 60000);
  res.load = { state: state.state, error: state.error, ms: Number((performance.now() - started).toFixed(1)), meta: state.meta };
  return state.state === 'ready';
};

const probesOf = (width, height) => {
  const cell = clip.width / 20;
  const fit = Math.min(width / clip.width, height / clip.height);
  return Array.from({ length: 10 }, (_, bit) => [Math.round((bit + 0.5) * cell * fit), Math.round(0.5 * cell * fit)]);
};
const decodeCode = (probes) => probes.reduce((code, pixel, bit) => code + ((pixel[0] + pixel[1] + pixel[2]) / 3 > 128 ? 1 << bit : 0), 0);
const isBlack = (mean) => mean && mean[0] + mean[1] + mean[2] < 0.5;

const verify = async (res) => {
  if (!(await loadClip(res))) return;
  const count = cfg.count ?? clip.frames;
  const order = cfg.explicit ?? ORDERS[cfg.order ?? 'forward'](count, cfg.n ?? count, cfg.seed);
  const probes = probesOf(...CANVAS);
  const wrong = [];
  const errors = {};
  const walls = [];
  let black = 0;
  for (const frame of order) {
    const done = await render([layerOf(frame, cfg.shift ?? 0)], { inspect: true, probes, frame });
    if (done.type !== 'frame.done') {
      const code = done.error?.code ?? 'unknown';
      errors[code] = (errors[code] ?? 0) + 1;
      (res.error_samples ??= []).length < 3 && res.error_samples.push([frame, done.error]);
      continue;
    }
    walls.push(done.wall);
    const got = decodeCode(done.probes);
    if (isBlack(done.mean)) black += 1;
    if (got !== frame && wrong.length < 40) wrong.push([frame, got, isBlack(done.mean) ? 'black' : 'picture']);
    else if (got !== frame) wrong.push(null);
  }
  const mismatches = wrong.length;
  res.checked = order.length;
  res.mismatches = mismatches;
  res.black = black;
  res.errors_by_code = errors;
  res.wrong_samples = wrong.filter(Boolean).slice(0, 12);
  res.wall = stats(walls);
  res.order_head = order.slice(0, 12);
};

const paced = async (res) => {
  if (!(await loadClip(res))) return;
  const count = cfg.count ?? clip.frames;
  const trace = ORDERS[cfg.order ?? 'forward'](count, cfg.n ?? 120, cfg.seed);
  const period = 1000 / cfg.hz;
  const stamps = [];
  const rows = [];
  let latest = -1;
  let sent = -1;
  let busy = false;
  let cursor = 0;
  const start = performance.now();
  const pump = async () => {
    if (busy || sent === latest || latest < 0) return;
    busy = true;
    const index = latest;
    sent = index;
    const done = await render([layerOf(trace[index])], { frame: trace[index] });
    rows.push({
      index,
      frame: trace[index],
      latency: performance.now() - stamps[index],
      lag: latest - index,
      wall: done.wall,
      ok: done.type === 'frame.done',
      code: done.error?.code,
      timings: done.timings,
      cache: done.cache && { created: done.cache.texture_created, reused: done.cache.texture_reused },
    });
    busy = false;
    pump();
  };
  await new Promise((resolve) => {
    const tick = () => {
      const now = performance.now();
      while (cursor < trace.length && start + cursor * period <= now) {
        stamps[cursor] = start + cursor * period;
        latest = cursor;
        cursor += 1;
      }
      pump();
      if (cursor < trace.length) setTimeout(tick, Math.max(0, start + cursor * period - performance.now()));
      else resolve();
    };
    tick();
  });
  while (busy || sent !== latest) {
    await sleep(2);
    pump();
  }
  const seconds = (performance.now() - start) / 1000;
  const ok = rows.filter((row) => row.ok);
  res.paced = {
    hz: cfg.hz,
    inputs: trace.length,
    presented: rows.length,
    presented_per_s: Number((rows.length / seconds).toFixed(1)),
    failed: rows.length - ok.length,
    latency: stats(ok.map((row) => row.latency)),
    wall: stats(ok.map((row) => row.wall)),
    over_100ms: ok.filter((row) => row.latency > 100).length,
    slowest: [...ok]
      .sort((a, b) => b.latency - a.latency)
      .slice(0, 5)
      .map((row) => ({ frame: row.frame, latency: Number(row.latency.toFixed(1)), wall: Number(row.wall.toFixed(1)), timings: row.timings })),
  };
};


// ---- mode "raw": the browser's own <video>, no runtime. Which read path shows the right picture?
const rawVideo = async (res) => {
  const { num, den } = rateOf();
  const video = document.createElement('video');
  video.muted = true;
  video.playsInline = true;
  video.preload = 'auto';
  video.crossOrigin = 'anonymous';
  if (cfg.attach) {
    Object.assign(video.style, { position: 'fixed', left: '0', top: '0', width: '2px', height: '2px', opacity: '0.01', pointerEvents: 'none' });
    document.body.append(video);
  }
  const url = `/asset?p=${encodeURIComponent(clip.path)}`;
  const source = cfg.blob ? URL.createObjectURL(await (await fetch(url)).blob()) : url;
  await new Promise((resolve, reject) => {
    video.onloadeddata = resolve;
    video.onerror = () => reject(new Error(video.error?.message ?? 'video error'));
    video.src = source;
  });
  if (cfg.warmup) {
    const warmStarted = performance.now();
    await new Promise((resolve) => {
      video.onseeked = resolve;
      video.currentTime = (0.5 * rateOf().den) / rateOf().num;
    });
    (await createImageBitmap(video)).close();
    res.warmup_ms = performance.now() - warmStarted;
  }
  res.element = { videoWidth: video.videoWidth, videoHeight: video.videoHeight, duration: video.duration, width_attr: video.width };
  const probes = probesOf(clip.width, clip.height);
  const canvas = Object.assign(document.createElement('canvas'), { width: clip.width, height: clip.height });
  const context = canvas.getContext('2d', { willReadFrequently: true });
  const code2d = (source) => {
    context.drawImage(source, 0, 0);
    return decodeCode(probes.map(([x, y]) => [...context.getImageData(x, y, 1, 1).data]));
  };
  const gl = Object.assign(document.createElement('canvas'), { width: clip.width, height: clip.height }).getContext('webgl2', { preserveDrawingBuffer: true });
  const glCode = (source) => {
    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source);
    const framebuffer = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    const pixels = probes.map(([x, y]) => {
      const out = new Uint8Array(4);
      gl.readPixels(x, y, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, out);
      return [...out];
    });
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.deleteFramebuffer(framebuffer);
    gl.deleteTexture(texture);
    return decodeCode(pixels);
  };
  const count = cfg.count ?? clip.frames;
  const order = ORDERS[cfg.order ?? 'random'](count, cfg.n ?? count, cfg.seed);
  const offset = cfg.offset ?? 0.5;
  const tally = { drawImage: 0, bitmap: 0, texImage2D: 0, rvfc: 0, rvfc_timeout: 0 };
  const times = { seeked: [], rvfc: [] };
  const samples = [];
  for (const frame of order) {
    const target = ((frame + offset) * den) / num;
    const started = performance.now();
    await new Promise((resolve) => {
      video.onseeked = resolve;
      video.currentTime = target;
    });
    times.seeked.push(performance.now() - started);
    const row = { frame };
    if (!cfg.only) {
      try {
        row.drawImage = code2d(video);
      } catch (error) {
        row.drawImage = `E:${error.name}:${error.message}`;
      }
    }
    try {
      const bitmapStarted = performance.now();
      const bitmap = await createImageBitmap(video);
      (times.bitmap ??= []).push(performance.now() - bitmapStarted);
      row.bitmap = code2d(bitmap);
      bitmap.close();
    } catch (error) {
      row.bitmap = `E:${error.message}`;
    }
    if (!cfg.only) try {
      const uploadStarted = performance.now();
      row.texImage2D = glCode(video);
      (times.upload ??= []).push(performance.now() - uploadStarted);
    } catch (error) {
      row.texImage2D = `E:${error.name}:${error.message}`;
    }
    for (const key of cfg.only ? ['bitmap'] : ['drawImage', 'bitmap', 'texImage2D']) if (row[key] !== frame) tally[key] += 1;
    if (cfg.rvfc && video.requestVideoFrameCallback) {
      const waited = performance.now();
      row.rvfc = await new Promise((resolve) => {
        const timer = setTimeout(() => resolve('timeout'), 400);
        video.requestVideoFrameCallback((_now, metadata) => {
          clearTimeout(timer);
          resolve(Number((metadata.mediaTime * num / den).toFixed(3)));
        });
      });
      times.rvfc.push(performance.now() - waited);
      if (row.rvfc === 'timeout') tally.rvfc_timeout += 1;
    }
    if (samples.length < 6) samples.push(row);
  }
  res.raw = { checked: order.length, wrong_by_path: tally, seeked_ms: stats(times.seeked), upload_ms: stats(times.upload ?? []), bitmap_ms: stats(times.bitmap ?? []), bitmap_slow_at: (times.bitmap ?? []).map((ms, i) => [i, Math.round(ms)]).filter(([, ms]) => ms > 100), seeked_slow_at: times.seeked.map((ms, i) => [i, Math.round(ms)]).filter(([, ms]) => ms > 100), upload_first_ms: times.upload?.[0], rvfc_ms: stats(times.rvfc), visibility: document.visibilityState, samples };
};

const caps = async (res) => {
  const video = document.createElement('video');
  const types = ['video/mp4; codecs="avc1.640028"', 'video/mp4; codecs="hvc1.1.6.L120.90"', 'video/mp4; codecs="av01.0.08M.08"', 'video/webm; codecs="vp8"', 'video/webm; codecs="vp9"'];
  res.canPlayType = Object.fromEntries(types.map((type) => [type, video.canPlayType(type)]));
  res.requestVideoFrameCallback = 'requestVideoFrameCallback' in HTMLVideoElement.prototype;
  res.fastSeek = 'fastSeek' in HTMLVideoElement.prototype;
  res.VideoDecoder = 'VideoDecoder' in window;
};

/**
 * mode "loss": the WebGL context dies while a frame is being drawn (image, scene or splat).
 * Claim: the host hears `frame.failed` with `context_lost` (never `frame.done` on a dead context,
 * never `render_failed`), and the frame after the restore draws again.
 */
const loss = async (res) => {
  const still = clip.pattern.replace(/%0?\d*d/, '00000');
  const assets = {
    image: [{ id: 'a', kind: 'still', source: { type: 'file', path: still }, meta: {} }],
    scene: [{ id: 'scene', kind: 'scene', source: { type: 'file', path: cfg.files.scene }, meta: {} }],
    splat: [
      { id: 'cloud', kind: 'splat', source: { type: 'file', path: cfg.files.splat }, meta: { format: 'splat' } },
      { id: 'scene', kind: 'scene', source: { type: 'file', path: cfg.files.splatScene }, meta: {} },
    ],
  }[cfg.kind];
  const layer = {
    image: { id: 'l', kind: 'image', asset: 'a', opacity: 1, blend: 'normal', transform: T0 },
    scene: { id: 'l', kind: 'scene', asset: 'scene', opacity: 1, blend: 'normal', transform: T0, camera: { position: [0, 0.3, 4], target: [0, 0, 0], fov: 50 }, scene_time: { seconds: 0 } },
    splat: { id: 'l', kind: 'scene', asset: 'scene', opacity: 1, blend: 'normal', transform: T0, camera: { position: [0, 0, 5], target: [0, 0, 0], fov: 50 }, scene_time: { seconds: 0 } },
  }[cfg.kind];
  for (const asset of assets) {
    send({ type: 'asset.load', asset });
    const state = await until((message) => message.type === 'asset.state' && message.id === asset.id && message.state !== 'loading', 60000);
    if (state.state !== 'ready') throw new Error(`asset ${asset.id}: ${JSON.stringify(state.error)}`);
  }
  const warm = await render([layer], { inspect: true });
  res.warm = warm.type;
  const id = nextId++;
  send({ type: 'frame.render', id, revision: id, plan: { version: 1, canvas: { width: 160, height: 90 }, intent: { mode: 'sequence' }, frame: 0, layers: [layer], requires: [], warnings: [] }, output: { width: 160, height: 90 } });
  // Same task as the request: the frame has not started when the context goes away.
  TesselRuntime.debug.loseContext();
  const outcome = await until((message) => (message.type === 'frame.done' || message.type === 'frame.failed') && message.id === id, 30000);
  await until((message) => message.type === 'context.lost', 30000);
  // A restore asked for inside the task of the loss event is ignored by the browser.
  await sleep(100);
  res.lost_frame = { type: outcome.type, code: outcome.error?.code, cause: outcome.error?.details?.cause?.code };
  TesselRuntime.debug.restoreContext();
  await until((message) => message.type === 'context.restored', 30000);
  const again = await render([layer], { inspect: true });
  res.after_restore = { type: again.type, code: again.error?.code, hash: again.hash, warm_hash: warm.hash };
};

const main = async () => {
  send({ type: 'hello' });
  const ready = await until((message) => message.type === 'ready', 30000);
  const res = { cfg, ua: navigator.userAgent, renderer: ready.runtime.renderer, three: ready.runtime.three, capabilities: ready.capabilities };
  window.__res = res;
  await ({ verify, paced, caps, loss, raw: rawVideo })[cfg.mode](res);
  res.errors = window.__errs;
  window.__vvFinish(res);
};
main().catch((error) => window.__vvFinish({ cfg, partial: window.__res, fatal: String(error?.stack ?? error), pending: __msgs.map((message) => `${message.type}:${message.id ?? ''}:${message.state ?? ''}:${message.error?.code ?? ''}:${message.error?.message ?? ''}`), errors: window.__errs }));
