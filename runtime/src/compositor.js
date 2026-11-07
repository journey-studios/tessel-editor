// Draws a render plan into one WebGL canvas: layers bottom to top in an orthographic scene
// measured in canvas pixels (y up). Scene layers are rendered to an offscreen target and
// composited like any other layer, so transform, crop, opacity and blend apply uniformly.
//
// Conventions shared with crates/core/src/plan.rs:
//   x, y        percent of canvas width/height; +x right, +y DOWN (screen convention)
//   scale       multiplier of the "contain" fit of the content inside the canvas
//   rotation    degrees, clockwise
//   crop        [left, right, top, bottom] percent of the layer's own size
//
// Between frames the compositor keeps (bounded, see retention.js) the objects whose rebuild was
// the measured cost of a frame: image textures (owned by the AssetStore), text textures, the
// layer materials and with them the shader program, scene rigs without splats and their render
// targets. With `retain` off it takes the original path: everything is built and freed per frame,
// so the two can be compared on the same bundle.

import * as THREE from 'three';

import { imageTexture } from './assets.js';
import {
  buildDirectedScene,
  buildSplatScenes,
  containSize,
  createSceneMaterial,
  drawDirectedScene,
  parseDirectedLayer,
  releaseSplatMesh,
  resolveSceneResources,
} from './directed-scene.js';
import { forEachLitCell, measureText, GLYPH_ROWS } from './font.js';
import { fnv1a64, meanColor } from './hash.js';
import { BLEND_MODES, RuntimeError } from './protocol.js';
import { LIMITS, LruCache, RetentionCounters, rgbaBytes, targetBytes } from './retention.js';
import { SceneRig } from './scene.js';
import { SplatStage } from './splat-stage.js';

THREE.ColorManagement.enabled = false;

const BLENDING = {
  normal: { blending: THREE.NormalBlending },
  multiply: {
    blending: THREE.CustomBlending,
    blendEquation: THREE.AddEquation,
    blendSrc: THREE.DstColorFactor,
    blendDst: THREE.OneMinusSrcAlphaFactor,
  },
  screen: {
    blending: THREE.CustomBlending,
    blendEquation: THREE.AddEquation,
    blendSrc: THREE.OneMinusDstColorFactor,
    blendDst: THREE.OneFactor,
  },
  darken: {
    blending: THREE.CustomBlending,
    blendEquation: THREE.MinEquation,
    blendSrc: THREE.OneFactor,
    blendDst: THREE.OneFactor,
  },
  lighten: {
    blending: THREE.CustomBlending,
    blendEquation: THREE.MaxEquation,
    blendSrc: THREE.OneFactor,
    blendDst: THREE.OneFactor,
  },
};

const canvasTexture = (canvas) => {
  const texture = new THREE.CanvasTexture(canvas);
  texture.flipY = false;
  texture.minFilter = THREE.LinearFilter;
  texture.magFilter = THREE.LinearFilter;
  texture.generateMipmaps = false;
  texture.colorSpace = THREE.NoColorSpace;
  return texture;
};

const drawText = (spec, canvasHeight) => {
  const { columns, rows } = measureText(spec.content);
  // `size` is the text height in percent of the canvas. Glyph cells are drawn at a whole
  // pixel size and the quad is scaled to the exact height, so crisp cells and exact size agree.
  const cell = (spec.size / 100) * canvasHeight / GLYPH_ROWS;
  const cellPixels = Math.max(1, Math.ceil(cell));
  const canvas = document.createElement('canvas');
  canvas.width = columns * cellPixels;
  canvas.height = rows * cellPixels;
  const context = canvas.getContext('2d');
  context.fillStyle = spec.color;
  forEachLitCell(spec.content, (column, row) => {
    context.fillRect(column * cellPixels, row * cellPixels, cellPixels, cellPixels);
  });
  const ratio = cell / cellPixels;
  return { canvas, width: canvas.width * ratio, height: canvas.height * ratio };
};

const drawWaveform = (layer, width, height) => {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext('2d');
  context.fillStyle = '#0d1117';
  context.fillRect(0, 0, width, height);
  const { peaks, position } = layer;
  const mid = height / 2;
  const barWidth = width / Math.max(1, peaks.length);
  context.fillStyle = '#5fb0ff';
  peaks.forEach((peak, index) => {
    const barHeight = Math.max(1, peak * height * 0.9);
    context.fillRect(index * barWidth, mid - barHeight / 2, Math.max(1, barWidth - 1), barHeight);
  });
  context.fillStyle = '#ffffff';
  context.fillRect(Math.round(position * width), 0, Math.max(1, Math.round(width / 320)), height);
  return canvas;
};

/** Quad centered on the origin with the crop applied to both geometry and UVs. */
const quadGeometry = (width, height, crop, flipV) => {
  const [left, right, top, bottom] = crop.map((percent) => percent / 100);
  const x0 = -width / 2 + left * width;
  const x1 = width / 2 - right * width;
  const y1 = height / 2 - top * height;
  const y0 = -height / 2 + bottom * height;
  const v = (value) => (flipV ? 1 - value : value);
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute(
    'position',
    new THREE.BufferAttribute(new Float32Array([x0, y0, 0, x1, y0, 0, x1, y1, 0, x0, y1, 0]), 3),
  );
  geometry.setAttribute(
    'uv',
    new THREE.BufferAttribute(
      new Float32Array([left, v(1 - bottom), 1 - right, v(1 - bottom), 1 - right, v(top), left, v(top)]),
      2,
    ),
  );
  geometry.setIndex([0, 1, 2, 0, 2, 3]);
  return geometry;
};

/** MSAA samples of a scene layer's render target unless the host asks for another count. */
const DEFAULT_SCENE_SAMPLES = 4;
export const SCENE_SAMPLE_CHOICES = [0, 2, 4];

const createLayerMaterial = (blend, texture, opacity, tint) =>
  new THREE.MeshBasicMaterial({
    map: texture,
    transparent: true,
    depthTest: false,
    depthWrite: false,
    opacity,
    color: new THREE.Color().setScalar(tint),
    ...BLENDING[blend],
  });

const disposeTarget = (target) => target.dispose();
const disposeText = (text) => {
  text.texture.dispose();
  // The canvas is CPU memory held by the texture's image; a texture that is gone needs none.
  text.canvas.width = 0;
  text.canvas.height = 0;
};

export class Compositor {
  constructor(canvas, { retain = true, resultCache = true, counters = new RetentionCounters(), limits = LIMITS } = {}) {
    this.canvas = canvas;
    this.lost = false;
    this.retain = retain;
    // Reuse of a splat scene's picture when nothing it depends on changed (see sceneResultKey).
    this.resultCache = resultCache;
    this.sceneSamples = DEFAULT_SCENE_SAMPLES;
    this.counters = counters;
    this.limits = limits;
    // Bumped when the context is lost: everything retained for the old one is dropped with it.
    this.contextGeneration = 0;
    const evicted = (name) => () => counters.add(name);
    this.text = new LruCache({
      maxEntries: limits.textEntries,
      maxBytes: limits.textBytes,
      dispose: disposeText,
      onEvict: evicted('text_evicted'),
    });
    this.materials = new LruCache({
      maxEntries: limits.materials,
      dispose: (material) => material.dispose(),
      onEvict: evicted('material_evicted'),
    });
    this.rigs = new LruCache({
      maxEntries: limits.rigs,
      dispose: ({ rig }) => rig.dispose(),
      onEvict: evicted('rig_evicted'),
    });
    this.targets = new LruCache({
      maxEntries: limits.targets,
      maxBytes: limits.targetBytes,
      dispose: ({ target }) => disposeTarget(target),
      onEvict: evicted('target_evicted'),
    });
    this.renderer = new THREE.WebGLRenderer({
      canvas,
      antialias: false,
      alpha: false,
      preserveDrawingBuffer: true,
      powerPreference: 'high-performance',
    });
    this.renderer.outputColorSpace = THREE.LinearSRGBColorSpace;
    this.renderer.autoClear = false;
    this.renderer.setPixelRatio(1);
    const gl = this.renderer.getContext();
    if (!(gl instanceof WebGL2RenderingContext)) {
      throw new RuntimeError('backend_unavailable', 'WebGL2 is required');
    }
    this.gl = gl;
    this.maxSize = Math.min(gl.getParameter(gl.MAX_RENDERBUFFER_SIZE), gl.getParameter(gl.MAX_TEXTURE_SIZE));
  }

  /** GPU or software renderer string, for reproducing a measurement. */
  rendererName() {
    const extension = this.gl.getExtension('WEBGL_debug_renderer_info');
    return extension ? this.gl.getParameter(extension.UNMASKED_RENDERER_WEBGL) : 'unknown';
  }

  setSize(width, height) {
    if (width > this.maxSize || height > this.maxSize) {
      throw new RuntimeError(
        'render_failed',
        `Output ${width}x${height} exceeds the GPU limit of ${this.maxSize}px`,
      );
    }
    this.renderer.setSize(width, height, false);
  }

  /**
   * Switches retention on or off. Turning it off frees what was kept so the next frame is the
   * original build-and-free path.
   */
  setRetain(retain) {
    if (this.retain === retain) return;
    this.retain = retain;
    if (!retain) this.dropRetained();
  }

  /** Replaces bounds (tests and measurements); what is over a new bound is freed after the next frame. */
  setLimits(limits) {
    this.limits = { ...this.limits, ...limits };
    this.text.maxEntries = this.limits.textEntries;
    this.text.maxBytes = this.limits.textBytes;
    this.materials.maxEntries = this.limits.materials;
    this.rigs.maxEntries = this.limits.rigs;
    this.targets.maxEntries = this.limits.targets;
    this.targets.maxBytes = this.limits.targetBytes;
  }

  /**
   * EXPERIMENT (OPT07): MSAA samples of scene targets. 4 is the contract's quality; fewer samples
   * are a visible quality trade (edges), so only a host that asks for it gets them.
   */
  setSceneSamples(samples) {
    if (!SCENE_SAMPLE_CHOICES.includes(samples) || samples === this.sceneSamples) return;
    this.sceneSamples = samples;
    // A target made for other samples must not be reused or serve a cached result.
    for (const { value } of this.targets.entries.values()) value.result = null;
  }

  /** Turns the splat result cache on or off (A/B and rollback); what it holds is forgotten either way. */
  setResultCache(enabled) {
    this.resultCache = enabled;
    for (const { value } of this.targets.entries.values()) value.result = null;
  }

  /** Sizes of what is retained now, for the frame report and the tests. */
  retained() {
    return {
      text_entries: this.text.size,
      text_bytes: this.text.bytes,
      material_entries: this.materials.size,
      rig_entries: this.rigs.size,
      target_entries: this.targets.size,
      target_bytes: this.targets.bytes,
      result_entries: [...this.targets.entries.values()].filter(({ value }) => value.result).length,
    };
  }

  /**
   * Frees every retained object. Called when the context is lost (the objects belong to the old
   * context; three.js builds its own tables again on restore, so disposing them later would
   * address the new context) and when retention is switched off.
   */
  dropRetained() {
    this.contextGeneration += 1;
    for (const cache of [this.text, this.materials, this.rigs, this.targets]) {
      try {
        cache.clear();
      } catch {
        // The context is already gone; there is nothing left to free on it.
      }
    }
  }

  /** An asset id was released or replaced: what was built from its old entry dies with it. */
  forgetAsset(id) {
    this.rigs.delete(id);
    // A result built from this asset names its serial, so it could never be served again; clearing
    // it now only stops a dead picture from looking alive in the counters.
    for (const { value } of this.targets.entries.values()) {
      if (value.result?.includes(`"${id}"`)) {
        value.result = null;
        this.counters.add('result_invalidated');
      }
    }
  }

  /**
   * Draws `plan`. `aborted()` is checked between layers so a seek that overtook this frame
   * stops it early. With `readback` (the default) it returns top-left-origin RGBA pixels; without
   * it the frame is only drawn and `null` comes back once the commands were handed to the GPU
   * (flush): that is not "the GPU finished". The picture is already on its canvas and hashing
   * 2 MB per frame is measurement, not display.
   *
   * `timings` receives the CPU phases: `prepare_ms` (assets, textures, scene rigs), `draw_ms`
   * (clear + render submission) and `flush_ms`; `settle_ms` repeats `flush_ms` for hosts that
   * read the older name.
   */
  async draw({ plan, width, height, assets, aborted, readback = true, timings = {}, quality = 'original', sources = {} }) {
    this.setSize(width, height);
    const camera = new THREE.OrthographicCamera(-width / 2, width / 2, height / 2, -height / 2, -10, 10);
    const scene = new THREE.Scene();
    const disposables = [];
    const keep = (item) => {
      disposables.push(item);
      return item;
    };
    // id -> entry of every asset this frame reads: if one is replaced before the draw, the
    // frame would show (or upload from) a source that is gone.
    const used = new Map();
    // `quality: 'preview'` lets a frame sequence with an attached proxy draw the proxy's pixels;
    // anything else draws the original. `sources` reports what each layer really read.
    sources.proxy_layers = 0;
    sources.original_layers = 0;
    sources.result_hits = 0;
    const frame = { plan, width, height, assets, keep, aborted, used, timings, quality, sources };

    assets.beginFrame();
    [this.text, this.materials, this.rigs, this.targets].forEach((cache) => cache.beginFrame());
    timings.prepare_ms = 0;
    try {
      for (const [order, layer] of plan.layers.entries()) {
        if (aborted()) throw new RuntimeError('cancelled', 'The frame was superseded');
        if (!BLEND_MODES.includes(layer.blend)) {
          throw new RuntimeError('unsupported_format', `Blend mode \`${layer.blend}\` is not supported by the runtime`, {
            layer: layer.id,
            supported: BLEND_MODES,
          });
        }
        const prepareStarted = performance.now();
        const prepared = await this.prepareLayer(layer, { ...frame, order });
        timings.prepare_ms += performance.now() - prepareStarted;
        scene.add(this.layerMesh(layer, prepared, { width, height, order, keep }));
      }
      if (aborted()) throw new RuntimeError('cancelled', 'The frame was superseded');
      for (const [id, entry] of used) {
        if (!assets.isCurrent(id, entry)) {
          throw new RuntimeError('cancelled', `Asset \`${id}\` was replaced while the frame was drawn`, { asset: id });
        }
      }

      const drawStarted = performance.now();
      this.renderer.setRenderTarget(null);
      this.renderer.setClearColor(0x000000, 1);
      this.renderer.clear(true, true, true);
      this.renderer.render(scene, camera);
      timings.draw_ms = performance.now() - drawStarted;
      if (!readback) {
        const flushStarted = performance.now();
        // WHY flush, not finish: finish() does not wait for the GPU in WebKit or Chromium (it is
        // asynchronous in the WebKit GPU process and only flushes in Chromium), so it never proved a
        // frame was complete; it only looked like it by taking ~0 ms. flush() hands the commands
        // over, which is all the browser lets a page guarantee. Whether the pixels are on the glass
        // is measured by the host (frame.presented, a rAF proxy) and by pixel proofs, not here.
        this.gl.flush();
        timings.flush_ms = performance.now() - flushStarted;
        timings.settle_ms = timings.flush_ms;
        return null;
      }
      const readStarted = performance.now();
      const pixels = this.readPixels(width, height);
      timings.readback_ms = performance.now() - readStarted;
      return pixels;
    } finally {
      disposables.forEach((item) => item.dispose?.());
      assets.endFrame();
      [this.text, this.materials, this.rigs, this.targets].forEach((cache) => cache.trim());
    }
  }

  /** One SparkRenderer per WebGL context, created on the first frame that has splats. */
  splatStage() {
    this.splats ??= new SplatStage(this.renderer);
    return this.splats;
  }

  /** Spark's GPU state belongs to the lost context; the next splat frame builds a new stage. */
  dropSplatStage() {
    try {
      this.splats?.dispose();
    } catch {
      // The context is already gone; there is nothing left to free on it.
    }
    this.splats = null;
  }

  /** Marks an asset entry as read by the frame in flight and returns it. */
  entryFor(layer, { assets, used }) {
    const entry = assets.get(layer.asset);
    used.set(layer.asset, entry);
    return entry;
  }

  async prepareLayer(layer, frame) {
    const { plan, width, height, assets, keep, aborted } = frame;
    switch (layer.kind) {
      case 'image': {
        const entry = this.entryFor(layer, frame);
        const texture = this.retain ? assets.stillTexture(layer.asset) : keep(imageTexture(entry.bitmap));
        return { texture, width: entry.width, height: entry.height, fit: true, flipV: false };
      }
      case 'video': {
        const entry = this.entryFor(layer, frame);
        let texture;
        if (entry.sequence) {
          const picked = await assets.qualityFrameTexture(layer.asset, layer.source_frame, frame.quality);
          texture = picked.texture;
          frame.sources[picked.source === 'proxy' ? 'proxy_layers' : 'original_layers'] += 1;
          if (picked.proxyError) frame.sources.proxy_error = picked.proxyError;
        } else {
          texture = await assets.videoFrameTexture(layer.asset, layer.source_frame, { aborted, timings: frame.timings });
          frame.sources.original_layers += 1;
        }
        // Textures of video assets are cached by the store, which owns their lifetime. The quad is
        // sized from the ORIGINAL's width and height, so a proxy texture fills exactly the same
        // rectangle: same framing and aspect, fewer pixels.
        return { texture, width: entry.width, height: entry.height, fit: true, flipV: false };
      }
      case 'text': {
        const text = this.retain ? this.retainedText(layer.text, height) : this.freshText(layer.text, height, keep);
        return { texture: text.texture, width: text.width, height: text.height, fit: false, flipV: false };
      }
      case 'waveform': {
        const texture = keep(canvasTexture(drawWaveform(layer, width, height)));
        return { texture, width, height, fit: false, flipV: false };
      }
      case 'scene': {
        const entry = this.entryFor(layer, frame);
        const key = this.sceneResultKey(layer, entry, frame);
        const { target, cached } = this.sceneTarget(frame.order, width, height, keep);
        if (key && cached?.result === key) {
          this.counters.add('result_hit');
          frame.sources.result_hits += 1;
          return { texture: target.texture, width, height, fit: false, flipV: true };
        }
        if (key) this.counters.add('result_miss');
        // The target is about to hold another picture (or half of one if this frame is cancelled):
        // until the draw completes it matches no key.
        if (cached) cached.result = null;
        const rig = await this.sceneRig(layer.asset, entry, assets, keep);
        await rig.draw(this.renderer, target, {
          camera: layer.camera,
          time: layer.scene_time.seconds,
          width,
          height,
          stage: rig.hasSplats ? this.splatStage() : null,
          aborted,
        });
        if (key && cached && !aborted()) cached.result = key;
        return { texture: target.texture, width, height, fit: false, flipV: true };
      }
      case 'directed_scene':
        return this.prepareDirectedScene(layer, frame);
      default:
        throw new RuntimeError('unsupported_format', `Cannot draw \`${layer.kind}\` layers`);
    }
  }

  /**
   * A layer that is the evaluated plan of a directed scene (directed-scene.js). The plan is
   * checked whole before anything is drawn; its pictures are the store's own retained textures
   * (so release, reload, context loss and the byte budget invalidate them like any other still);
   * the picture goes to an offscreen target and is composited like a `scene` layer.
   */
  async prepareDirectedScene(layer, frame) {
    const { assets, keep, width, height, aborted, used } = frame;
    const spec = parseDirectedLayer(layer);
    const bound = resolveSceneResources(spec, assets);
    // A slot replaced before the draw makes the frame cancelled, never a mix of two revisions.
    bound.forEach(({ slot, entry }) => used.set(slot, entry));
    if (aborted()) throw new RuntimeError('cancelled', 'The frame was superseded');

    const size = containSize(spec.output, width, height);
    // Its own slot space (`ds:`): a directed scene never shares a target, or a cached result, with a `scene` layer.
    const { target, cached } = this.sceneTarget(`ds:${frame.order}`, size.width, size.height, keep);
    if (cached) cached.result = null;
    const textureOf = (assetId) => {
      const { slot, entry } = bound.get(assetId);
      return this.retain ? assets.stillTexture(slot) : keep(imageTexture(entry.bitmap));
    };
    const directed = buildDirectedScene(spec, {
      textureOf,
      materialOf: (alpha, index, texture) => this.sceneMaterial(frame.order, alpha, index, texture, keep),
      keep,
    });
    // What the frame will sort and draw: every node counts the splats of its file, however many nodes
    // share one decode (the data is shared, the draw is not). Refused before any lease or draw.
    const frameSplats = spec.splats.reduce((total, splat) => total + (bound.get(splat.assetId).entry.count ?? 0), 0);
    if (frameSplats > this.limits.frameSplats) {
      throw new RuntimeError(
        'resource_budget',
        `The frame would draw ${frameSplats} splats across ${spec.splats.length} splat nodes (limit ${this.limits.frameSplats})`,
        { frame_splats: frameSplats, nodes: spec.splats.length, limit: this.limits.frameSplats },
      );
    }
    // Splats are leased from the asset store for this frame only (a mesh has one parent, and the
    // GPU side is rebuilt from the kept bytes after a lost context); `keep` hands them back when
    // the frame ends, drawn, failed or cancelled.
    const meshes = [];
    for (const splat of spec.splats) {
      const lease = await assets.splatLease(bound.get(splat.assetId).slot);
      keep({
        dispose: () => {
          releaseSplatMesh(lease.mesh);
          lease.release();
        },
      });
      meshes.push(lease.mesh);
      if (aborted()) throw new RuntimeError('cancelled', 'The frame was superseded');
    }
    const splatScenes = buildSplatScenes(spec, meshes);
    await drawDirectedScene(this.renderer, target, { ...directed, splatScenes }, { stage: splatScenes.length > 0 ? this.splatStage() : null, aborted });
    frame.sources.directed_scene_layers = (frame.sources.directed_scene_layers ?? 0) + 1;
    if (splatScenes.length > 0) frame.sources.directed_scene_splats = (frame.sources.directed_scene_splats ?? 0) + spec.splats.length;
    return { texture: target.texture, width: size.width, height: size.height, fit: false, flipV: true };
  }

  /** Same retention rule as `layerMaterial`: one material per layer slot keeps its shader program linked. */
  sceneMaterial(order, alpha, index, texture, keep) {
    if (!this.retain) return keep(createSceneMaterial(alpha, texture));
    const key = `ds:${order}:${alpha}:${index}`;
    let material = this.materials.get(key);
    if (material) {
      this.counters.add('material_reused');
    } else {
      material = createSceneMaterial(alpha, texture);
      this.materials.set(key, material);
      this.counters.add('material_created');
    }
    material.map = texture;
    return material;
  }

  freshText(spec, canvasHeight, keep) {
    const text = drawText(spec, canvasHeight);
    return { texture: keep(canvasTexture(text.canvas)), width: text.width, height: text.height };
  }

  /**
   * A text layer's texture. The picture of a text depends only on its content, size, color and
   * the canvas height (the glyph cell is rounded to whole pixels from it), so those are the key.
   * Bounded by count and bytes: a text that changes every frame (a counter) cycles through the
   * cache instead of growing it.
   */
  retainedText(spec, canvasHeight) {
    const key = JSON.stringify([spec.content, spec.size, spec.color, canvasHeight]);
    const hit = this.text.get(key);
    if (hit) {
      this.counters.add('text_reused');
      return hit;
    }
    const drawn = drawText(spec, canvasHeight);
    const text = { texture: canvasTexture(drawn.canvas), canvas: drawn.canvas, width: drawn.width, height: drawn.height };
    this.text.set(key, text, rgbaBytes(drawn.canvas.width, drawn.canvas.height));
    this.counters.add('text_created');
    return text;
  }

  /**
   * The rig of a scene layer. Scenes without splats are a pure function of the scene file, so
   * the rig (geometry, materials and therefore the shader program) is kept for the entry it was
   * built from and for this context. Scenes with splats lease meshes from the asset store and
   * are rebuilt per frame (the leases are what is not kept: the store lends the same Spark meshes
   * again, BL131): their cost is the Spark sort, not the rig.
   */
  async sceneRig(id, entry, assets, keep) {
    if (!this.retain || SceneRig.hasSplatObjects(entry.scene)) {
      return keep(await SceneRig.build(entry.scene, assets));
    }
    const cached = this.rigs.get(id);
    if (cached && cached.entry === entry && cached.contextGeneration === this.contextGeneration) {
      this.counters.add('rig_reused');
      return cached.rig;
    }
    const rig = await SceneRig.build(entry.scene, assets);
    this.rigs.set(id, { entry, contextGeneration: this.contextGeneration, rig });
    this.counters.add('rig_built');
    return rig;
  }

  /**
   * The MSAA target a scene layer renders into, with the retained entry that owns it (`cached`,
   * `null` when the target is built and freed per frame). One per layer slot and size, retained
   * when it fits the byte budget; a target that does not (export-size scenes) is never retained,
   * so it can never serve a cached result either.
   */
  sceneTarget(slot, width, height, keep) {
    const create = () =>
      new THREE.WebGLRenderTarget(width, height, {
        samples: this.sceneSamples,
        depthBuffer: true,
        colorSpace: THREE.NoColorSpace,
      });
    const bytes = targetBytes(width, height, this.sceneSamples);
    if (!this.retain || bytes > this.limits.targetBytes) return { target: keep(create()), cached: null };
    const key = `${slot}`;
    const cached = this.targets.get(key);
    if (cached && cached.width === width && cached.height === height && cached.samples === this.sceneSamples) {
      this.counters.add('target_reused');
      return { target: cached.target, cached };
    }
    const entry = { target: create(), width, height, samples: this.sceneSamples, result: null };
    this.targets.set(key, entry, bytes);
    this.counters.add('target_created');
    return { target: entry.target, cached: entry };
  }

  /**
   * What the picture of a scene layer depends on, as one string, or `null` when it must not be
   * reused. Two frames with the same key draw the same pixels, so the second may keep the first's
   * render target as is (no Spark sort, no draw). The key lists, on purpose, everything:
   *   - the scene asset and every splat asset it names, by load serial (a relinked or reloaded
   *     asset has a new serial even under the same id);
   *   - the camera and the output size (aspect, resolution);
   *   - the scene time, but only when an object spins: a static scene's picture does not move with
   *     the playhead, an animated one does and then every time is its own picture;
   *   - the MSAA sample count and the context generation (a restored context has none of it).
   * Layer opacity, blend and transform are applied when the target is composited, not drawn into
   * it, so they are not part of the key. Only scenes with splats are keyed: without Spark there
   * is no sort to skip and the draw is already well under a millisecond.
   */
  sceneResultKey(layer, entry, { assets, used, width, height }) {
    if (!this.retain || !this.resultCache || !SceneRig.hasSplatObjects(entry.scene)) return null;
    const splats = [];
    for (const object of entry.scene.objects) {
      if (object.type !== 'splat') continue;
      const asset = assets.entries.get(object.asset);
      if (!asset) return null;
      used.set(object.asset, asset);
      splats.push([object.asset, asset.serial, object.scale ?? 1, object.position ?? null, object.spin ?? null]);
    }
    const animated = entry.scene.objects.some((object) => Array.isArray(object.spin) && object.spin.some((degrees) => degrees !== 0));
    return JSON.stringify({
      scene: [layer.asset, entry.serial],
      splats,
      camera: layer.camera,
      time: animated ? layer.scene_time.seconds : null,
      size: [width, height],
      samples: this.sceneSamples,
      context: this.contextGeneration,
    });
  }

  /**
   * The mesh of one layer. The material is the same object for the same blend mode and layer
   * slot from frame to frame, so the shader program stays linked; what changes per layer
   * (texture, opacity, tint) is assigned every frame. Geometry is rebuilt per frame: measuring
   * found no gain in pooling it.
   */
  layerMesh(layer, prepared, { width, height, order, keep }) {
    const { transform } = layer;
    const fit = prepared.fit ? Math.min(width / prepared.width, height / prepared.height) : 1;
    const layerWidth = prepared.width * fit * transform.scale;
    const layerHeight = prepared.height * fit * transform.scale;
    const geometry = keep(quadGeometry(layerWidth, layerHeight, transform.crop, prepared.flipV));
    // Blend modes that cannot use source alpha fade by scaling color instead.
    const fades = layer.blend !== 'normal';
    const opacity = fades ? 1 : layer.opacity;
    const tint = fades ? layer.opacity : 1;
    const material = this.retain
      ? this.layerMaterial(layer.blend, order, prepared.texture, opacity, tint)
      : keep(createLayerMaterial(layer.blend, prepared.texture, opacity, tint));
    const mesh = new THREE.Mesh(geometry, material);
    mesh.position.set((transform.x / 100) * width, (-transform.y / 100) * height, 0);
    mesh.rotation.z = -THREE.MathUtils.degToRad(transform.rotation);
    mesh.renderOrder = order;
    return mesh;
  }

  layerMaterial(blend, slot, texture, opacity, tint) {
    const key = `${blend}:${slot}`;
    let material = this.materials.get(key);
    if (material) {
      this.counters.add('material_reused');
      material.map = texture;
      material.opacity = opacity;
      material.color.setScalar(tint);
      return material;
    }
    material = createLayerMaterial(blend, texture, opacity, tint);
    this.materials.set(key, material);
    this.counters.add('material_created');
    return material;
  }

  readPixels(width, height) {
    const { gl } = this;
    const raw = new Uint8Array(width * height * 4);
    gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, raw);
    const pixels = new Uint8Array(raw.length);
    const stride = width * 4;
    for (let row = 0; row < height; row += 1) {
      pixels.set(raw.subarray((height - 1 - row) * stride, (height - row) * stride), row * stride);
    }
    return pixels;
  }

  dispose() {
    this.dropRetained();
    this.dropSplatStage();
    this.renderer.dispose();
    this.renderer.forceContextLoss();
  }
}

/** Hash, mean and probes of top-left-origin RGBA pixels. */
export const describePixels = (pixels, width, height, probes) => ({
  hash: fnv1a64(pixels),
  mean: meanColor(pixels),
  probes: probes.map(([x, y]) => {
    const offset = (y * width + x) * 4;
    return [pixels[offset], pixels[offset + 1], pixels[offset + 2], pixels[offset + 3]];
  }),
  width,
  height,
});
