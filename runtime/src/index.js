// Runtime entry: speaks bridge v1 (crates/core/src/bridge.rs) over a host-provided `post`.
// The host decides the transport (Node worker stdio, WebView IPC) and how files are fetched;
// everything about what to draw comes from the plan.

import * as THREE from 'three';

import { ASSET_IDENTITY_CAPABILITIES, assetEcho } from './asset-identity.js';
import { AssetStore } from './assets.js';
import { Compositor, DEFAULT_PREVIEW_SORT, describePixels } from './compositor.js';
import { DIRECTED_SCENE_CAPABILITIES } from './directed-scene.js';
import { refreshAfterFrame } from './overlay.js';
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

export const createRuntime = ({ post, assetUrl, canvas, overlay = null }) => {
  const send = (message) => post({ v: BRIDGE_VERSION, ...message });
  const counters = new RetentionCounters();
  const assets = new AssetStore({ assetUrl, counters });
  // Additive fields of `hello` (bridge v1): `retain: false` turns the GPU retention off (A/B and
  // quick rollback), `telemetry.presented: true` asks for a `frame.presented` message per frame.
  // A host that never sends them gets retention on and no extra messages.
  // `result_cache: false` makes every splat scene frame sort and draw again (A/B and rollback).
  const options = { retain: true, presented: false, resultCache: true };
  let compositor = null;
  let latestRender = 0;
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
    runtimeSpans.push(span);
  };
  const drainSpans = () => {
    const spans = runtimeSpans.splice(0);
    if (spansLost > 0) spans.push({ k: 'lost', n: spansLost });
    spansLost = 0;
    return spans;
  };
  let contextLost = false;
  // Counts context losses; a frame that sees it change between its start and its end was drawn
  // on (or partly on) a context that died, whatever the outcome of the calls looked like.
  let contextLosses = 0;
  let disposed = false;
  let loseExtension = null;

  const ensureCompositor = () => {
    if (compositor) return compositor;
    try {
      compositor = new Compositor(canvas, { retain: options.retain, resultCache: options.resultCache, counters });
      if (options.sceneSamples !== undefined) compositor.setSceneSamples(options.sceneSamples);
      if (options.previewSceneSamples !== undefined) compositor.setPreviewSceneSamples(options.previewSceneSamples);
      if (options.previewSort !== undefined) compositor.setPreviewSort(options.previewSort);
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
        features: overlay ? [...FEATURES, OVERLAY_FEATURE] : FEATURES,
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
    try {
      const loadStarted = performance.now();
      const meta = await assets.load(descriptor);
      // Additive: how long the load took on this page's clock and, where measured, its fetch and decode parts.
      const timings = { load_ms: performance.now() - loadStarted, ...assets.takeLoadTimings(descriptor.id) };
      send({ type: 'asset.state', id: descriptor.id, state: 'ready', meta, timings, ...echo });
    } catch (error) {
      // Timings noted by a load that then failed or was cancelled must not reach the next load of this id.
      assets.takeLoadTimings(descriptor.id);
      send({ type: 'asset.state', id: descriptor.id, state: 'failed', error: toErrorJson(error, 'asset_unreadable'), ...echo });
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

  const renderFrame = async (request) => {
    const started = performance.now();
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
      send(message);
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
      // One GL context: frames run strictly one after another, newest wins.
      renderQueue = renderQueue.then(() => renderFrame(message));
    },
    // Additive (negotiated by `scene.warm`): no picture, no answer. Behind the frames in the queue, like them.
    'frame.warm': (message) => {
      const queuedAt = performance.now();
      renderQueue = renderQueue.then(async () => {
        if (disposed || contextLost) return;
        try {
          const warmStarted = performance.now();
          const started = await ensureCompositor().warmScenes({ plan: message.plan, assets });
          noteSpan({ k: 'warm', queued_ms: warmStarted - queuedAt, ms: performance.now() - warmStarted, started });
        } catch {
          // Best effort: a frame that needs the scene reports what is wrong with it.
        }
      });
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
