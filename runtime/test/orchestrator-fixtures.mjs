// Fixtures of the Node tests of the page orchestrator (`src/index.js`), with no browser and no GPU.
//
// `createRuntime` is the real one: its serial queue, its `AssetStore`, `TexturePreupload`, `WarmDraws`, spans and messages. Only the
// compositor is replaced (the `createCompositor` seam of index.js) by `FakeCompositor`, which records what it was asked instead of
// drawing, the canvas by one that can lose its context, and the draw of a warm (the `warmDraw` seam) by one that records too. So these
// tests prove what index.js DECIDES (what runs before what, what a lost context or a dispose drops, which span the host hears); what a
// real compositor does with WebGL (a pixel, a texture on the GPU, a sort, how long any of it takes) stays in the Chrome tests and in
// the native A/B. Not a test file (no `.test.` in the name).
import { createRuntime } from '../src/index.js';
import { RuntimeError } from '../src/protocol.js';
import { imageLayer, plan, sceneLayer } from './support.mjs';

/** Every still decodes to SIDE x SIDE (40 000 RGBA bytes), far below the default texture budget. */
export const SIDE = 100;

export const deferred = () => {
  let resolve;
  const promise = new Promise((done) => {
    resolve = done;
  });
  return { promise, resolve };
};

const tick = () => new Promise((resolve) => setImmediate(resolve));

/** Fails instead of hanging: a test that waits for a queue it is still holding must say so. */
const within = (promise, label) =>
  Promise.race([
    promise,
    new Promise((_, reject) => {
      setTimeout(() => reject(new Error(label)), 5000).unref();
    }),
  ]);

/** Polls (by yielding to the event loop, never by sleeping) until `predicate` holds. */
export const until = async (predicate, label) => {
  for (let round = 0; round < 500; round += 1) {
    if (predicate()) return;
    await tick();
  }
  throw new Error(`never happened: ${label}`);
};

/**
 * A clock the test can move: `performance.now()` plus a skew. The page ages a queued upload and a pending warm draw against it (3 s), so a
 * test advances it instead of sleeping. Restore it with `t.after(clock.restore)`.
 */
export const fakeClock = () => {
  const real = performance.now.bind(performance);
  let skew = 0;
  performance.now = () => real() + skew;
  return {
    advance: (ms) => {
      skew += ms;
    },
    restore: () => {
      delete performance.now;
    },
  };
};

/**
 * What index.js needs of a Compositor and nothing more. It does not draw: `log` keeps the order in which the page asked it for things
 * (`draw`, `upload`, `warm_scenes`, `warm_draw`, `dispose`), and a few knobs make the world misbehave on purpose (`failUploads`,
 * `warmDrawOutcome`, `warmScenesError`, `lost`, `onUpload`). A still layer goes through the REAL store (`stillTexture`: reused or created,
 * pinned, counted), so what a frame finds is what the real compositor would find. The GPU is a set of textures (`onGpu`); one that a frame
 * had to put there is a `frameUpload`: exactly the cost the pre-upload moves out of the frame.
 */
export class FakeCompositor {
  constructor(canvas, { retain, resultCache }) {
    this.canvas = canvas;
    this.retain = retain;
    this.resultCache = resultCache;
    this.contextGeneration = 0;
    this.maxSize = 4096;
    this.lost = false;
    this.gl = { isContextLost: () => this.lost, getExtension: () => null };
    this.onGpu = new Set();
    this.log = [];
    this.uploads = [];
    this.frameUploads = 0;
    this.failUploads = 0;
    this.onUpload = null;
    this.warming = new Map();
    this.sorts = [];
    this.warmDrawOutcome = 'drawn';
    this.warmScenesError = null;
    this.disposed = false;
  }

  get renderer() {
    return { info: { memory: { textures: this.onGpu.size, geometries: 0 }, programs: [] }, initTexture: (texture) => this.initTexture(texture) };
  }

  rendererName() {
    return 'fake-compositor';
  }

  retained() {
    return {};
  }

  setLimits() {}

  setSceneSamples() {}

  setPreviewSceneSamples() {}

  setResultCache(enabled) {
    this.resultCache = enabled;
  }

  setPreviewSort(sort) {
    this.previewSort = sort;
  }

  setRetain(retain) {
    this.retain = retain;
    if (!retain) this.dropRetained();
  }

  forgetAsset() {}

  dropRetained() {
    this.contextGeneration += 1;
    this.onGpu.clear();
  }

  dropSplatStage() {
    this.warming.clear();
  }

  dispose() {
    this.disposed = true;
    this.log.push('dispose');
  }

  /** The GPU upload of a texture made ahead of its frame (`renderer.initTexture`, which index.js calls). */
  initTexture(texture) {
    if (this.lost) throw new RuntimeError('context_lost', 'The graphics context is lost');
    if (this.failUploads > 0) {
      this.failUploads -= 1;
      throw new Error('the GPU refused the texture');
    }
    this.onGpu.add(texture);
    this.uploads.push(texture);
    this.log.push('upload');
    this.onUpload?.(texture);
  }

  async draw({ plan: framePlan, assets, aborted }) {
    assets.beginFrame();
    try {
      if (aborted()) throw new RuntimeError('cancelled', 'The frame was superseded');
      this.log.push('draw');
      for (const layer of framePlan.layers) {
        if (layer.kind !== 'image') continue;
        if (!this.retain) {
          this.frameUploads += 1;
          continue;
        }
        const texture = assets.stillTexture(layer.asset);
        if (!this.onGpu.has(texture)) {
          this.onGpu.add(texture);
          this.frameUploads += 1;
        }
      }
      return null;
    } finally {
      assets.endFrame();
    }
  }

  /** Starts a sort per scene layer; the test decides when it ends (`sorts[i].resolve()`). */
  async warmScenes({ plan: warmPlan, onWarmed }) {
    this.log.push('warm_scenes');
    if (this.warmScenesError) throw this.warmScenesError;
    if (!this.retain || this.lost) return 0;
    let started = 0;
    for (const layer of warmPlan.layers) {
      if (layer.kind !== 'scene') continue;
      const lineage = `lineage:${layer.asset}`;
      if (this.warming.has(lineage)) continue;
      const sort = deferred();
      this.sorts.push(sort);
      this.warming.set(lineage, sort.promise);
      sort.promise.then(() => this.warming.delete(lineage));
      onWarmed?.({ lineage, layer, sorted: sort.promise });
      started += 1;
    }
    return started;
  }
}

/** A canvas that records the listeners the runtime adds and can fire them (a lost and a restored context). */
export class FakeCanvas {
  constructor() {
    this.listeners = new Map();
  }

  addEventListener(type, listener) {
    this.listeners.set(type, listener);
  }

  dispatch(type) {
    this.listeners.get(type)?.({ preventDefault() {} });
  }

  toDataURL() {
    return 'data:image/png;base64,';
  }
}

export const stills = (...ids) => ids.map((id) => imageLayer(id));
export const scenes = (...ids) => ids.map((id) => sceneLayer(id));

/** What the host sends for the cuts a moment ahead: a warm of textures only (`scenes: false`). */
export const textureWarm = (layers) => ({ type: 'frame.warm', plan: plan(layers), scenes: false, textures: true });
/** The warm of splat scenes it always was (no flags). */
export const sceneWarm = (layers) => ({ type: 'frame.warm', plan: plan(layers) });
export const frameMessage = (layers, id) => ({
  type: 'frame.render',
  id,
  revision: id,
  plan: plan(layers),
  output: { width: 160, height: 90 },
  inspect: false,
});

const bitmap = () => ({ width: SIDE, height: SIDE, close() {} });
export const stillDescriptor = (id, path = `${id}.png`, request = undefined) => ({
  id,
  kind: 'still',
  source: { type: 'file', path },
  meta: {},
  ...(request === undefined ? {} : { request }),
});

/**
 * The real runtime on the fake compositor, talked to the way the host talks to it (bridge v1 messages in, `messages` out). Every still
 * decodes to SIDE x SIDE through the real `AssetStore.load`, so ids get the serial a real load gives them; `hold(path)` keeps the next
 * decode of that path pending until the returned function is called.
 */
export const openFake = () => {
  const messages = [];
  const canvas = new FakeCanvas();
  const compositors = [];
  const runtime = createRuntime({
    post: (message) => messages.push(message),
    assetUrl: (path) => path,
    canvas,
    createCompositor: (target, options) => {
      const compositor = new FakeCompositor(target, options);
      compositors.push(compositor);
      return compositor;
    },
    // The warm draw of the page, without WebGL: what it was asked, in the order the compositor's other calls happened, and an outcome.
    warmDraw: async (compositor) => {
      compositor.log.push('warm_draw');
      return compositor.warmDrawOutcome;
    },
  });
  const { debug } = runtime;
  const { assets } = debug;
  const holds = new Map();
  let nextFrame = 100;

  const settle = async () => {
    for (let round = 0; round < 100; round += 1) {
      const queue = debug.idle();
      await within(queue, 'the page queue is held: release it before settling');
      await tick();
      if (debug.idle() === queue) return;
    }
    throw new Error('the page queue never settled');
  };

  const page = {
    runtime,
    debug,
    assets,
    messages,
    canvas,
    compositors,
    settle,
    /** The figures the fake decoder reports for a path (`asset.state` carries them); a test may replace it. */
    timings: () => ({ fetch_ms: 2, decode_ms: 3 }),
    /** The compositor the page uses now (still reachable after a dispose, which drops the page's own reference). */
    get compositor() {
      return compositors.at(-1) ?? null;
    },
    send: (message) => runtime.receive({ v: 1, ...message }),
    hello: (extra = {}) => {
      page.send({ type: 'hello', ...extra });
      return messages.findLast((message) => message.type === 'ready');
    },
    hold: (path) => {
      const gate = deferred();
      holds.set(path, gate.promise);
      return () => {
        holds.delete(path);
        gate.resolve();
      };
    },
    load: async (id, { path = `${id}.png`, request } = {}) => {
      const from = messages.length;
      page.send({ type: 'asset.load', asset: stillDescriptor(id, path, request) });
      const answered = (message) => message.type === 'asset.state' && message.id === id && message.state !== 'loading';
      await until(() => messages.slice(from).some(answered), `the load of ${id}`);
      return messages.slice(from).find(answered);
    },
    loadAll: async (...ids) => {
      for (const id of ids) await page.load(id);
    },
    release: (id) => page.send({ type: 'asset.release', id }),
    /** Sends a warm and waits for the page's queue to finish everything it started. */
    warm: async (message) => {
      page.send(message);
      await settle();
    },
    /** Asks for a picture and returns its `frame.done` (or `frame.failed`). */
    frame: async (layers, id = (nextFrame += 1)) => {
      page.send(frameMessage(layers, id));
      await settle();
      return messages.findLast((message) => (message.type === 'frame.done' || message.type === 'frame.failed') && message.id === id);
    },
    loseContext: () => {
      page.compositor.lost = true;
      canvas.dispatch('webglcontextlost');
    },
    restoreContext: () => {
      page.compositor.lost = false;
      canvas.dispatch('webglcontextrestored');
    },
    /** What the host has heard from the page's own background work (`frame.done.runtime_spans`), in order; one kind with `k`. */
    spans: (k = null) =>
      messages
        .filter((message) => message.type === 'frame.done')
        .flatMap((message) => message.runtime_spans ?? [])
        .filter((span) => k === null || span.k === k),
    outcomes: (k) => page.spans(k).map((span) => span.outcome),
  };

  assets.loadStill = async ({ source }) => {
    await holds.get(source.path);
    return {
      timings: page.timings(source.path),
      entry: { bitmap: bitmap(), width: SIDE, height: SIDE },
      meta: { width: SIDE, height: SIDE },
    };
  };
  return page;
};
