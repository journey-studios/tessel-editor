// Runtime entry: speaks bridge v1 (crates/core/src/bridge.rs) over a host-provided `post`.
// The host decides the transport (Node worker stdio, WebView IPC) and how files are fetched;
// everything about what to draw comes from the plan.

import * as THREE from 'three';

import { ASSET_IDENTITY_CAPABILITIES, assetEcho } from './asset-identity.js';
import { AssetStore } from './assets.js';
import { Compositor, DEFAULT_PREVIEW_SORT, describePixels } from './compositor.js';
import { DIRECTED_SCENE_CAPABILITIES } from './directed-scene.js';
import { refreshAfterFrame } from './overlay.js';
import { TexturePreupload, stillsToUpload } from './preupload.js';
import { RetentionCounters } from './retention.js';
import { SPLAT_FORMATS } from './splat-format.js';
import { SPARK_VERSION } from './splat-stage.js';
import {
  BRIDGE_VERSION,
  LAYERS,
  RUNTIME_NAME,
  RUNTIME_VERSION,
  RuntimeError,
  toErrorJson,
} from './protocol.js';

const FORMATS = {
  still: ['png', 'jpeg', 'webp', 'gif', 'bmp'],
  // `mp4` is H.264 only as a promise; any other codec/container is accepted only if the load-time
  // proof in native-video.js reads real, changing pictures on this engine (WebM VP8/VP9 does not
  // in WebKit, AV1 sticks on the first frame there). The media adapter's frames cover the rest.
  video: ['frame_sequence', 'mp4'],
  audio: ['wav'],
  splat: SPLAT_FORMATS,
  scene: ['tessel-scene@1'],
  text: ['inline'],
};

/** What the native `<video>` path promises (see native-video.js for the time contract). */
const NATIVE_VIDEO = {
  codecs: ['h264'],
  timing: 'constant_rate_from_pts_zero',
  proof: 'load_time_read',
};

/** Optional runtime behaviours a host asks for in `hello`; absent means the default. */
const FEATURES = [
  'frame.presented',
  'cache.telemetry',
  'asset.proxy',
  'frame.quality',
  'scene.result_cache',
  'directed_scene',
  'asset.identity',
  // BL157: `frame.warm` gives the splat scenes of a plan their Spark stage and first sort ahead of the frame that draws them.
  'scene.warm',
];
// F055: announced only by a page that has an overlay surface (the desktop WebView, see
// `wantsOverlay`), never by the headless worker (same bundle, no surface): a host sends `overlay.*`
// messages only to a runtime that says it can draw them.
const OVERLAY_FEATURE = 'overlay.gizmo';
// BL157.3: announced only to a host whose `hello` asked for it (`preupload_textures: true`), so a default `ready` is the one it
// always was. A `frame.warm` that says `textures: true` then uploads the textures of the loaded still images of its plan
// before a frame draws them (preupload.js).
const PREUPLOAD_FEATURE = 'texture.preupload';
// BL157.3 (second, separate experiment): announced only to a host whose `hello` asked for it (`warm_draw: true`). A warm of
// splat scenes then also draws each scene it sorted once into a scratch target (`Compositor.warmDraw`).
const WARM_DRAW_FEATURE = 'scene.warm_draw';
// A warm draw older than this is dropped unrun: the cut it was for has come or gone.
const WARM_DRAW_MAX_AGE_MS = 3000;

// `createCompositor` is the seam of the Node unit tests (`test/orchestrator-fixtures.mjs`): a page never passes it, and the
// default is the one compositor there has always been. WebGL cannot run in plain Node, so a test that wants to prove what THIS
// file decides (the serial queue, a frame going first, what a lost context or a dispose drops, which span the host hears) hands
// in a compositor that records instead of drawing; what the real compositor does stays in the Chrome tests.
export const createRuntime = ({
  post,
  assetUrl,
  canvas,
  overlay = null,
  createCompositor = (target, compositorOptions) => new Compositor(target, compositorOptions),
}) => {
  const send = (message) => post({ v: BRIDGE_VERSION, ...message });
  const counters = new RetentionCounters();
  const assets = new AssetStore({ assetUrl, counters });
  // Additive fields of `hello` (bridge v1): `retain: false` turns the GPU retention off (A/B and
  // quick rollback), `telemetry.presented: true` asks for a `frame.presented` message per frame.
  // A host that never sends them gets retention on and no extra messages.
  // `result_cache: false` makes every splat scene frame sort and draw again (A/B and rollback).
  const options = { retain: true, presented: false, timing: false, gpuFinish: false, resultCache: true, preupload: false, warmDraw: false };
  let compositor = null;
  let latestRender = 0;
  // BL157.3: textures to upload before their clip starts (opt-in), and the frames waiting behind the one being drawn.
  const preupload = new TexturePreupload();
  let framesQueued = 0;
  let preuploadPumping = false;
  const warmDrawStats = { requested: 0, drawn: 0, skipped: 0, expired: 0, draw_ms: 0, max_draw_ms: 0, outcomes: {} };
  const cancelled = new Set();
  let renderQueue = Promise.resolve();
  // Durations of the page's own background work (asset loads, warms), on the page's clock, handed to the host with the next
  // `frame.done` so the host can place them next to its own stamps without ever subtracting the two clocks (BL157.1).
  // Bounded: a page nobody drains keeps the newest, and says how many it let go.
  const SPAN_LIMIT = 256;
  const runtimeSpans = [];
  let spansLost = 0;
  const noteSpan = (span) => {
    if (runtimeSpans.length >= SPAN_LIMIT) {
      runtimeSpans.shift();
      spansLost += 1;
    }
    // With `telemetry.timing` a span also says when it ended, as a duration back from the moment it is handed over
    // (`ago_ms`), so the host can place it without a shared clock; the message that carries it may leave much later.
    runtimeSpans.push(options.timing ? { ...span, at: performance.now() } : span);
  };
  const drainSpans = () => {
    const now = performance.now();
    const spans = runtimeSpans.splice(0).map(({ at, ...span }) =>
      at === undefined ? span : { ...span, ago_ms: Math.max(0, now - at) });
    if (spansLost > 0) spans.push({ k: 'lost', n: spansLost });
    spansLost = 0;
    return spans;
  };

  // BL157.2: what the page spends between receiving a frame request and answering it, and whether its own event loop was
  // stalled. Only with `telemetry.timing`; every figure is a duration on this page's clock, never a stamp the host would
  // subtract from its own. `ahead` is what shared the page's one serial queue when the request arrived.
  const backlog = { frames: 0, warms: 0, loads: 0 };
  const BEAT_MS = 4;
  // A beat later than this is also kept as a span of its own, to be placed next to the host's events.
  const STALL_MS = 16;
  const beat = { timer: null, due: 0, n: 0, maxMs: 0, over: 0 };
  const stopBeat = () => {
    if (beat.timer !== null) clearTimeout(beat.timer);
    beat.timer = null;
  };
  const startBeat = () => {
    if (beat.timer !== null) return;
    beat.due = performance.now() + BEAT_MS;
    const tick = () => {
      const now = performance.now();
      const lag = Math.max(0, now - beat.due);
      beat.n += 1;
      if (lag > beat.maxMs) beat.maxMs = lag;
      if (lag >= STALL_MS) {
        beat.over += 1;
        noteSpan({ k: 'loop_stall', ms: lag });
      }
      beat.due = now + BEAT_MS;
      beat.timer = setTimeout(tick, BEAT_MS);
    };
    beat.timer = setTimeout(tick, BEAT_MS);
  };
  const drainBeat = () => {
    const report = { n: beat.n, max_ms: beat.maxMs, over: beat.over, beat_ms: BEAT_MS };
    beat.n = 0;
    beat.maxMs = 0;
    beat.over = 0;
    return report;
  };
  // The WebGL calls that can make the page's thread wait for the GPU process (a readback, a fence, an upload, a link, a draw
  // that has to flush). With `telemetry.timing` each is wrapped, and one that takes GL_SPAN_MS or more becomes a span
  // `{ k: 'gl_call', name, ms }`: it says whether a stall of the event loop was inside a WebGL call (and which) or somewhere
  // else. Wrapped on the prototype, restored when the timing goes off; bounded to one span per slow call.
  const GL_SPAN_MS = 4;
  const GL_CALLS = [
    'readPixels', 'getBufferSubData', 'clientWaitSync', 'getSyncParameter', 'finish', 'flush', 'getError', 'getParameter',
    'getProgramParameter', 'getShaderParameter', 'linkProgram', 'compileShader', 'useProgram', 'texImage2D', 'texSubImage2D',
    'texImage3D', 'texSubImage3D', 'bufferData', 'bufferSubData', 'copyBufferSubData', 'blitFramebuffer', 'generateMipmap',
    'drawArrays', 'drawElements', 'drawArraysInstanced', 'drawElementsInstanced', 'deleteTexture', 'deleteBuffer',
    'deleteFramebuffer', 'deleteProgram', 'deleteRenderbuffer', 'createTexture', 'createBuffer', 'invalidateFramebuffer',
  ];
  const glWrapped = [];
  const stopGlProbe = () => {
    glWrapped.splice(0).forEach(({ prototype, name, original }) => {
      prototype[name] = original;
    });
  };
  const startGlProbe = () => {
    if (glWrapped.length > 0) return;
    for (const prototype of [globalThis.WebGL2RenderingContext?.prototype, globalThis.WebGLRenderingContext?.prototype]) {
      if (!prototype) continue;
      for (const name of GL_CALLS) {
        const original = Object.getOwnPropertyDescriptor(prototype, name)?.value;
        if (typeof original !== 'function') continue;
        prototype[name] = function (...args) {
          const started = performance.now();
          try {
            return original.apply(this, args);
          } finally {
            const ms = performance.now() - started;
            if (ms >= GL_SPAN_MS) noteSpan({ k: 'gl_call', name, ms });
          }
        };
        glWrapped.push({ prototype, name, original });
      }
    }
  };
  const finishPixel = new Uint8Array(4);
  let lastPostMs = null;
  let contextLost = false;
  // Counts context losses; a frame that sees it change between its start and its end was drawn
  // on (or partly on) a context that died, whatever the outcome of the calls looked like.
  let contextLosses = 0;
  let disposed = false;
  let loseExtension = null;

  const ensureCompositor = () => {
    if (compositor) return compositor;
    try {
      compositor = createCompositor(canvas, { retain: options.retain, resultCache: options.resultCache, counters });
      if (options.sceneSamples !== undefined) compositor.setSceneSamples(options.sceneSamples);
      if (options.previewSceneSamples !== undefined) compositor.setPreviewSceneSamples(options.previewSceneSamples);
      if (options.previewSort !== undefined) compositor.setPreviewSort(options.previewSort);
      compositor.setWarmDraw(options.warmDraw);
    } catch (error) {
      throw error instanceof RuntimeError
        ? error
        : new RuntimeError('backend_unavailable', `Cannot start WebGL: ${error.message}`);
    }
    canvas.addEventListener('webglcontextlost', (event) => {
      // preventDefault asks the browser to restore the context later.
      event.preventDefault();
      // dispose() loses the context on purpose; that is not news for the host.
      if (disposed) return;
      contextLost = true;
      contextLosses += 1;
      // Spark's textures and the splat meshes' GPU data die with the context; both rebuild lazily.
      compositor?.dropSplatStage();
      preupload.clear();
      // Retained textures, materials, rigs and targets belong to the lost context too; dropping
      // them now (before the restore) keeps three.js from addressing the new context with them.
      compositor?.dropRetained();
      assets.dropGpu();
      send({ type: 'context.lost' });
    });
    canvas.addEventListener('webglcontextrestored', () => {
      contextLost = false;
      send({ type: 'context.restored' });
    });
    // An asset released or replaced takes what was built from its old entry with it.
    assets.releaseListeners.add((id) => compositor?.forgetAsset(id));
    return compositor;
  };

  const applyOptions = (message) => {
    if (typeof message.retain === 'boolean') options.retain = message.retain;
    options.presented = message.telemetry?.presented === true;
    options.timing = message.telemetry?.timing === true;
    // BL157.3 (additive): every hello says it whole; absent is off, and turning it off forgets what was queued.
    options.preupload = message.preupload_textures === true;
    if (!options.preupload) preupload.clear();
    options.warmDraw = message.warm_draw === true;
    compositor?.setWarmDraw(options.warmDraw);
    // DIAGNOSTIC arm (BL157.2): wait for the GPU after each draw and report it; never without `timing`.
    options.gpuFinish = options.timing && message.telemetry?.gpu_finish === true;
    if (options.timing) {
      startBeat();
      startGlProbe();
    } else {
      stopBeat();
      stopGlProbe();
    }
    if (typeof message.result_cache === 'boolean') options.resultCache = message.result_cache;
    compositor?.setResultCache(options.resultCache);
    options.sceneSamples = Number.isInteger(message.scene_samples) ? message.scene_samples : undefined;
    if (options.sceneSamples !== undefined) compositor?.setSceneSamples(options.sceneSamples);
    // Additive (BL129): MSAA of scene layers in `preview` frames; absent keeps the runtime's default.
    options.previewSceneSamples = Number.isInteger(message.preview_scene_samples) ? message.preview_scene_samples : undefined;
    if (options.previewSceneSamples !== undefined) compositor?.setPreviewSceneSamples(options.previewSceneSamples);
    // Additive (BL130): how a splat scene of a `preview` frame is ordered. Every hello says it whole: absent is the
    // default (`latest`), also on a later hello that follows one that asked for `exact`; a name that is not one of
    // the two is `exact` (see `Compositor.setPreviewSort`).
    options.previewSort = message.preview_sort ?? DEFAULT_PREVIEW_SORT;
    compositor?.setPreviewSort(options.previewSort);
    compositor?.setRetain(options.retain);
    assets.setRetainSplatMeshes(options.retain);
    if (!options.retain) assets.dropTextures();
  };

  const hello = (message) => {
    applyOptions(message);
    const active = ensureCompositor();
    send({
      type: 'ready',
      runtime: {
        name: RUNTIME_NAME,
        version: RUNTIME_VERSION,
        renderer: active.rendererName(),
        three: THREE.REVISION,
        spark: SPARK_VERSION,
      },
      capabilities: {
        layers: LAYERS,
        formats: FORMATS,
        max_size: active.maxSize,
        features: [...FEATURES, ...(overlay ? [OVERLAY_FEATURE] : []), ...(options.preupload ? [PREUPLOAD_FEATURE] : []), ...(options.warmDraw ? [WARM_DRAW_FEATURE] : [])],
        native_video: NATIVE_VIDEO,
        // Additive: what a `directed_scene` layer needs and how its resources are identified.
        directed_scene: DIRECTED_SCENE_CAPABILITIES,
        // Additive: `asset.state` repeats the `request`/`scene` identity of the `asset.load` it answers.
        asset_identity: ASSET_IDENTITY_CAPABILITIES,
      },
    });
  };

  const loadAsset = async (descriptor) => {
    // Every state of this load carries the identity of the request that caused it, including the
    // `cancelled` of a load a release overtook: the host tells it from the answer of the load that
    // took its slot (BL102). A descriptor without identity gets the messages it always got.
    let echo;
    try {
      echo = assetEcho(descriptor);
    } catch (error) {
      send({ type: 'asset.state', id: descriptor.id, state: 'failed', error: toErrorJson(error, 'invalid_request') });
      return;
    }
    send({ type: 'asset.state', id: descriptor.id, state: 'loading', ...echo });
    backlog.loads += 1;
    try {
      const loadStarted = performance.now();
      const meta = await assets.load(descriptor);
      // Additive: how long the load took on this page's clock and, where measured, its fetch and decode parts.
      const timings = { load_ms: performance.now() - loadStarted, ...assets.takeLoadTimings(meta) };
      send({ type: 'asset.state', id: descriptor.id, state: 'ready', meta, timings, ...echo });
    } catch (error) {
      send({ type: 'asset.state', id: descriptor.id, state: 'failed', error: toErrorJson(error, 'asset_unreadable'), ...echo });
    } finally {
      backlog.loads -= 1;
    }
  };

  const cacheReport = (active, before) => {
    const { memory, programs } = active.renderer.info;
    return {
      retain: active.retain,
      ...counters.delta(before),
      ...active.retained(),
      memory: assets.memoryReport(),
      texture_entries: assets.textureCount(),
      texture_bytes: assets.textureBytes(),
      asset_generation: assets.generation,
      scene_resources: assets.sceneResourceCount(),
      scene_generation: assets.sceneGeneration,
      context_generation: active.contextGeneration,
      gl_textures: memory.textures,
      gl_geometries: memory.geometries,
      gl_programs: programs?.length ?? 0,
    };
  };

  // `frame.presented` is a requestAnimationFrame callback after `frame.done`: the browser got to
  // a rendering opportunity once the commands were handed over. It is a proxy for "on screen",
  // not a measurement of it (no rAF fires in a hidden view, and the compositor may still be
  // behind). `after_done_ms` is measured on the page clock, so the host needs no clock sync.
  const announcePresented = (id, revision) => {
    const doneAt = performance.now();
    requestAnimationFrame(() => {
      send({ type: 'frame.presented', id, revision, after_done_ms: performance.now() - doneAt });
    });
  };

  /** A failure that happened while the context was lost is reported as the loss, with the original as `cause`. */
  const failureOf = (error, lost) => {
    const json = toErrorJson(error);
    if (!lost || json.code === 'context_lost' || json.code === 'cancelled') return json;
    return {
      code: 'context_lost',
      message: 'The graphics context was lost while the frame was drawn',
      details: { cause: json },
    };
  };

  const renderFrame = async (request, arrival = null) => {
    const started = performance.now();
    framesQueued -= 1;
    if (arrival) backlog.frames -= 1;
    const { id, revision, plan, output, probes = [] } = request;
    // Additive field of bridge v1: `preview` lets a frame sequence draw its attached proxy; absent
    // (every host that predates it, and every explicit/export request) means the original.
    const quality = request.quality === 'preview' ? 'preview' : 'original';
    const sources = {};
    const aborted = () => cancelled.has(id) || latestRender !== id;
    const lossesAtStart = contextLosses;
    // WHY the check runs on every outcome: three.js keeps "succeeding" on a lost context (calls are
    // no-ops, readPixels returns zeros) and other failures are only a symptom of the loss. Either
    // way the host must hear `context_lost` (it re-sends the frame once the context is restored),
    // with the first error kept as `cause` so the real reason is not hidden by the classification.
    // `isContextLost()` covers the window between the loss and its event (a separate task).
    const lostDuringFrame = () => contextLosses !== lossesAtStart || contextLost || Boolean(compositor?.gl.isContextLost());
    try {
      if (aborted()) throw new RuntimeError('cancelled', 'The frame was cancelled before it started');
      if (contextLost) throw new RuntimeError('context_lost', 'The graphics context is lost');
      const active = ensureCompositor();
      const { width, height } = output;
      // Additive field of bridge v1: a host that predates it never sends it, and the answer is
      // then the full description it always got. Pixels asked for by probes or a PNG are never
      // skipped, whatever the flag says.
      const readback = request.inspect !== false || Boolean(output.png) || probes.length > 0;
      const timings = {};
      const cacheBefore = counters.snapshot();
      const drawStarted = performance.now();
      const pixels = await active.draw({ plan, width, height, assets, aborted, readback, timings, quality, sources });
      if (lostDuringFrame()) throw new RuntimeError('context_lost', 'The graphics context was lost while the frame was drawn');
      if (options.gpuFinish) {
        // `finish()` alone returned at once in WebKit (the context is remote: it only flushes), so a one-pixel readback,
        // which cannot return before the GPU has run what was drawn, is the wait.
        const finishStarted = performance.now();
        const { gl } = active;
        gl.finish();
        gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, finishPixel);
        timings.gpu_finish_ms = performance.now() - finishStarted;
      }
      const describeStarted = performance.now();
      // Not inspected: no hash/mean/probes (the host reads an empty hash as "not measured").
      const description = pixels
        ? describePixels(pixels, width, height, probes)
        : { hash: null, mean: null, probes: [], width, height };
      timings.describe_ms = performance.now() - describeStarted;
      const message = {
        type: 'frame.done',
        id,
        revision,
        ...description,
        inspected: readback,
        png: null,
        timings: {
          ...timings,
          render_ms: performance.now() - drawStarted,
          total_ms: performance.now() - started,
        },
        // Additive: what the retention layer did for this frame and what it holds now.
        cache: cacheReport(active, cacheBefore),
        // Additive: the quality asked for and what each layer really read, so a host can tell a
        // proxy picture from an original one (and see a proxy that had to be dropped).
        source: { quality, ...sources },
      };
      // Additive: the page's own background work since the last picture (see `noteSpan`); absent when there was none.
      const spans = drainSpans();
      if (spans.length > 0) message.runtime_spans = spans;
      // The host that owns the filesystem writes the file; the page only encodes it.
      if (output.png) message.png_data = canvas.toDataURL('image/png').split(',')[1];
      if (arrival && options.timing) {
        // Additive (`telemetry.timing`): the page's own account of this frame, durations on its clock. `queue_ms` is the
        // wait in the page's serial queue before the frame started (which `total_ms` leaves out), `held_ms` all the time
        // since the request arrived, up to the answer being handed to the transport.
        message.page = {
          queue_ms: started - arrival.at,
          held_ms: performance.now() - arrival.at,
          ahead: arrival.ahead,
          post_prev_ms: lastPostMs,
          loop: drainBeat(),
        };
      }
      const postStarted = options.timing ? performance.now() : 0;
      send(message);
      if (options.timing) lastPostMs = performance.now() - postStarted;
      if (options.presented) announcePresented(id, revision);
      // The canvas may have changed shape with this picture: an active overlay re-checks that it
      // still describes it (F055). After `frame.done` and `frame.presented`, so it never delays
      // them, and isolated: a failure here is the overlay's, never `frame.failed` after a success.
      refreshAfterFrame(overlay, (error) =>
        send({ type: 'overlay.error', error: toErrorJson(error, 'overlay_failed') }));
    } catch (error) {
      send({ type: 'frame.failed', id, revision, error: failureOf(error, lostDuringFrame()) });
    } finally {
      cancelled.delete(id);
    }
  };

  /**
   * One upload of the queue (BL157.3) as a task of the page's serial queue. A frame that is waiting goes first: the upload
   * puts itself back behind it, and after an upload the next one is queued behind whatever arrived meanwhile. A lost
   * context, a retention switch-off or a dispose drops the queue instead of touching a context that is not the one asked.
   */
  const pumpPreupload = () => {
    if (preuploadPumping || preupload.pending.length === 0) return;
    preuploadPumping = true;
    renderQueue = renderQueue.then(() => {
      preuploadPumping = false;
      if (framesQueued > 0) {
        pumpPreupload();
        return;
      }
      if (disposed || contextLost || !options.preupload || !options.retain || !compositor || compositor.gl.isContextLost()) {
        preupload.clear();
        return;
      }
      const result = preupload.step(assets, (texture) => compositor.uploadTexture(texture));
      if (result) noteSpan({ k: 'preupload', outcome: result.outcome, ms: result.ms });
      pumpPreupload();
    });
  };

  /**
   * The warm draws of the scenes a warm just started (BL157.3, opt-in): each after ITS sort, as a task of the serial queue, behind
   * any frame that is waiting (a frame that arrived has the page's GPU first; it also takes the lineage over, which turns the
   * draw into a no-op). A dispose, a lost context or the switch off drops them.
   */
  const scheduleWarmDraws = () => {
    for (const item of compositor?.takeWarmDraws() ?? []) {
      warmDrawStats.requested += 1;
      const run = () => {
        renderQueue = renderQueue.then(async () => {
          if (framesQueued > 0) {
            run();
            return;
          }
          const outcome = await runWarmDraw(item);
          if (outcome) noteSpan({ k: 'warm_draw', outcome: outcome.outcome, ms: outcome.ms });
        });
      };
      void item.sorted.then(run);
    }
  };

  const runWarmDraw = async (item) => {
    const note = (outcome, ms = 0) => {
      warmDrawStats.outcomes[outcome] = (warmDrawStats.outcomes[outcome] ?? 0) + 1;
      return { outcome, ms };
    };
    if (disposed || contextLost || !options.warmDraw || !compositor) {
      warmDrawStats.skipped += 1;
      return note('dropped');
    }
    if (performance.now() - item.at > WARM_DRAW_MAX_AGE_MS) {
      warmDrawStats.expired += 1;
      return note('expired');
    }
    const began = performance.now();
    const outcome = await compositor.warmDraw(item, assets);
    const ms = performance.now() - began;
    if (outcome === 'drawn') {
      warmDrawStats.drawn += 1;
      warmDrawStats.draw_ms += ms;
      warmDrawStats.max_draw_ms = Math.max(warmDrawStats.max_draw_ms, ms);
    } else {
      warmDrawStats.skipped += 1;
    }
    return note(outcome, ms);
  };

  const handlers = {
    hello,
    'asset.load': (message) => {
      void loadAsset(message.asset);
    },
    'asset.release': (message) => assets.release(message.id),
    // Additive: attaches (or, with `proxy: null`, removes) the preview proxy of a loaded frame sequence.
    'asset.proxy': (message) => {
      try {
        assets.setProxy(message.id, message.proxy ?? null);
        send({ type: 'asset.proxy', id: message.id, state: message.proxy ? 'attached' : 'detached' });
      } catch (error) {
        send({ type: 'asset.proxy', id: message.id, state: 'failed', error: toErrorJson(error, 'invalid_proxy') });
      }
    },
    'frame.render': (message) => {
      latestRender = message.id;
      framesQueued += 1;
      // One GL context: frames run strictly one after another, newest wins.
      let arrival = null;
      if (options.timing) {
        arrival = { at: performance.now(), ahead: { ...backlog } };
        backlog.frames += 1;
      }
      renderQueue = renderQueue.then(() => renderFrame(message, arrival));
    },
    // Additive (negotiated by `scene.warm`): no picture, no answer. Behind the frames in the queue, like them.
    'frame.warm': (message) => {
      const queuedAt = performance.now();
      // `scenes: false` (BL157.3): a warm of textures only; absent is the warm of splat scenes it always was.
      if (message.scenes !== false) {
        backlog.warms += 1;
        renderQueue = renderQueue.then(async () => {
          backlog.warms -= 1;
          if (disposed || contextLost) return;
          try {
            const warmStarted = performance.now();
            const started = await ensureCompositor().warmScenes({ plan: message.plan, assets });
            noteSpan({ k: 'warm', queued_ms: warmStarted - queuedAt, ms: performance.now() - warmStarted, started });
            scheduleWarmDraws();
          } catch {
            // Best effort: a frame that needs the scene reports what is wrong with it.
          }
        });
      }
      // Asked for here (the serial of each entry is the one loaded now: a replacement before the upload makes it stale).
      if (message.textures === true && options.preupload && options.retain && !disposed && !contextLost) {
        preupload.enqueue(stillsToUpload(message.plan, assets));
        pumpPreupload();
      }
    },
    'frame.cancel': (message) => {
      cancelled.add(message.id);
    },
    'viewport.resize': () => {},
    // F055 (additive, negotiated by the `overlay.gizmo` feature): the gizmo drawn over the picture.
    ...Object.fromEntries(
      ['overlay.enable', 'overlay.set', 'overlay.clear'].map((type) => [
        type,
        (message) => {
          // No overlay surface (the headless worker): the feature was never negotiated, so this is
          // a protocol refusal and stays the generic `unsupported` error.
          if (!overlay) throw new RuntimeError('unsupported', 'This page has no overlay surface');
          // An operational overlay that fails (a malformed payload): the overlay's own failure, not a
          // protocol error. The generic `error` closes the host's render session; `overlay.error` is
          // consumed by the overlay adapter, and the page keeps its previous overlay and its frames.
          try {
            overlay.receive(message);
          } catch (error) {
            send({ type: 'overlay.error', request: type, error: toErrorJson(error, 'invalid_overlay') });
          }
        },
      ]),
    ),
    dispose: () => {
      disposed = true;
      preupload.clear();
      stopBeat();
      stopGlProbe();
      assets.releaseAll();
      compositor?.dispose();
      compositor = null;
      send({ type: 'closed', reason: 'disposed' });
    },
  };

  const receive = (raw) => {
    let message;
    try {
      message = typeof raw === 'string' ? JSON.parse(raw) : raw;
    } catch (error) {
      send({ type: 'error', error: { code: 'protocol_error', message: `Invalid JSON: ${error.message}` } });
      return;
    }
    if (message.v !== BRIDGE_VERSION) {
      send({
        type: 'error',
        error: {
          code: 'version_mismatch',
          message: `Host speaks bridge v${message.v}, runtime speaks v${BRIDGE_VERSION}`,
        },
      });
      return;
    }
    const handler = handlers[message.type];
    if (!handler) {
      send({ type: 'error', error: { code: 'protocol_error', message: `Unknown message type \`${message.type}\`` } });
      return;
    }
    try {
      handler(message);
    } catch (error) {
      send({ type: 'error', error: toErrorJson(error, 'protocol_error') });
    }
  };

  // Test and measurement hooks; never part of the bridge.
  const debug = {
    // The extension object is kept: a lost context may refuse to hand it out again.
    loseContext: () => {
      loseExtension = compositor?.gl.getExtension('WEBGL_lose_context') ?? null;
      loseExtension?.loseContext();
    },
    restoreContext: () => loseExtension?.restoreContext(),
    loadedAssets: () => [...assets.entries.keys()],
    // What is retained right now and the generations that guard it (tests and measurements).
    retention: () => ({
      retain: options.retain,
      presented: options.presented,
      preview_sort: compositor?.previewSort ?? null,
      ...(compositor?.retained() ?? {}),
      texture_entries: assets.textureCount(),
      texture_bytes: assets.textureBytes(),
      asset_generation: assets.generation,
      scene_resources: assets.sceneResourceCount(),
      scene_generation: assets.sceneGeneration,
      context_generation: compositor?.contextGeneration ?? 0,
      counters: counters.snapshot(),
    }),
    // GL objects three.js believes alive on the current context: what a lifecycle test compares to
    // prove a lost context, a replaced project or a cancelled frame left nothing behind.
    glMemory: () => {
      const info = compositor?.renderer.info;
      if (!info) return null;
      return { textures: info.memory.textures, geometries: info.memory.geometries, programs: info.programs?.length ?? 0 };
    },
    idle: () => renderQueue,
    // Holds the page's serial queue until the returned function is called (tests): whatever is queued meanwhile waits.
    holdQueue: () => {
      let release;
      const gate = new Promise((resolve) => {
        release = resolve;
      });
      renderQueue = renderQueue.then(() => gate);
      return release;
    },
    // The texture pre-upload queue and what it did (tests and measurements).
    preupload: () => ({ enabled: options.preupload, ...preupload.report() }),
    // What the scene warm draw did (tests and measurements).
    warmDraw: () => ({ enabled: options.warmDraw, ...warmDrawStats, outcomes: { ...warmDrawStats.outcomes } }),
    // Lineages whose warm sort is still running (tests).
    warming: () => compositor?.warming.size ?? 0,
    // Every SparkRenderer alive, to inject a failure into one or count them (tests and measurements).
    splatStages: () => compositor?.liveStages() ?? [],
    // The compositor, to make one pass of a frame fail on purpose (tests): the frame must report it and the next must draw.
    compositor: () => compositor,
    // The store itself and the bounds, so a test can hold a frame at an await or shrink a budget.
    assets,
    setLimits: (limits) => {
      assets.limits = { ...assets.limits, ...limits };
      ensureCompositor().setLimits(limits);
    },
  };

  return { receive, debug };
};
