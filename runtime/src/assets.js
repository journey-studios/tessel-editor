// Asset cache: fetches files through the host's asset URL and keeps decoded CPU-side data
// (bitmaps, bytes, parsed scenes), which a lost WebGL context never invalidates.
//
// Image textures are GPU-side and retained on the entry that owns the bitmap (see
// retention.js): created on first use, freed with the entry, on a lost context (they are made
// again from the bitmap) and by the byte budget. Re-uploading a static image every frame was
// most of a preview frame's cost.

import * as THREE from 'three';

import { parseSceneIdentity } from './directed-scene.js';
import { closeVideo, frameCount, openVideo, proveVideo, readFrame } from './native-video.js';
import { RuntimeError, expandPattern } from './protocol.js';
import { LIMITS, RetentionCounters, rgbaBytes } from './retention.js';
import { splatFileName } from './splat-format.js';
import { cloneSplatMesh, disposeSplatMeshes, loadSplatMesh } from './splat-stage.js';

const FRAME_CACHE_LIMIT = 16;
// Preview-proxy frames are small (a 540p picture is ~2 MiB decoded), and a scrub that turns back
// revisits them, so they get a larger window than full-size frames. Still bounded: the texture
// byte budget is the hard limit, this one keeps the CPU-side bitmaps in check.
const PROXY_FRAME_CACHE_LIMIT = 48;

/** Proxy frames share the entry's frame map with the original's; their keys are `p<index>`. */
const proxyKey = (index) => `p${index}`;
const isProxyKey = (key) => typeof key === 'string';

const fetchChecked = async (url, label) => {
  let response;
  try {
    response = await fetch(url);
  } catch (error) {
    throw new RuntimeError('asset_unreadable', `Cannot fetch ${label}: ${error.message}`);
  }
  if (!response.ok) {
    throw new RuntimeError('asset_unreadable', `Cannot read ${label}: HTTP ${response.status}`);
  }
  return response;
};

const decodeImage = async (response, label) => {
  const blob = await response.blob();
  try {
    // No premultiply and no color conversion: pixels must reach the framebuffer untouched.
    return await createImageBitmap(blob, { premultiplyAlpha: 'none', colorSpaceConversion: 'none' });
  } catch (error) {
    throw new RuntimeError('decode_failed', `Cannot decode ${label}: ${error.message}`);
  }
};

const imageTexture = (bitmap) => {
  const texture = new THREE.Texture(bitmap);
  texture.flipY = false;
  texture.premultiplyAlpha = false;
  texture.minFilter = THREE.LinearFilter;
  texture.magFilter = THREE.LinearFilter;
  texture.generateMipmaps = false;
  texture.colorSpace = THREE.NoColorSpace;
  texture.needsUpdate = true;
  return texture;
};

export class AssetStore {
  constructor({ assetUrl, counters = new RetentionCounters(), limits = LIMITS }) {
    this.assetUrl = assetUrl;
    this.counters = counters;
    this.limits = limits;
    this.entries = new Map();
    // id -> token of the load in flight; dropping it is how release() cancels that load.
    this.loading = new Map();
    // Bumps whenever the set of loaded assets or their GPU data changes (load, release, lost
    // context). A retained object that records it can tell it was made for a world that is gone.
    this.generation = 0;
    // Recency clock for the texture budget: higher means used more recently.
    this.clock = 0;
    // Textures the frame being drawn uses; the budget never frees these.
    this.pinned = new Set();
    // Told when an id is released so objects built from it elsewhere (scene rigs) die with it.
    this.releaseListeners = new Set();
    // Every loaded entry gets a number nobody else had. A cache of a *result* built from an asset
    // keys on it: the id may come back with other bytes (relink, reload), the serial never does.
    this.serials = 0;
    // Highest project generation a scene resource (`descriptor.scene`) has announced. A resource of
    // a lower one belongs to a world that was replaced: it is refused and what it left is released.
    this.sceneGeneration = 0;
  }

  /** Called by the compositor when a frame starts: nothing is pinned yet. */
  beginFrame() {
    this.pinned.clear();
  }

  /** Called when the frame ends (also when it failed or was cancelled): enforce the byte budget. */
  endFrame() {
    this.trimTextures();
    this.pinned.clear();
  }

  /** The current entry of every asset id a frame used, to detect a replacement mid-frame. */
  isCurrent(id, entry) {
    return this.entries.get(id) === entry;
  }

  has(id) {
    return this.entries.has(id);
  }

  get(id) {
    const entry = this.entries.get(id);
    if (!entry) {
      throw new RuntimeError('asset_unreadable', `Asset \`${id}\` is not loaded`, { asset: id });
    }
    return entry;
  }

  /** Loads (or reloads) an asset and returns the metadata the runtime measured. */
  async load(descriptor) {
    const { id, kind, source } = descriptor;
    // Refused before `release`: a refused load leaves the entry that was there untouched.
    if (descriptor.scene !== undefined) this.admitScene(descriptor);
    this.release(id);
    // release() while the bytes are still arriving cancels this load (see below).
    const token = Symbol(id);
    this.loading.set(id, token);
    const loaders = {
      still: () => this.loadStill(descriptor),
      video: () => this.loadVideo(descriptor),
      audio: () => this.loadAudio(descriptor),
      splat: () => this.loadSplat(descriptor),
      scene: () => this.loadScene(descriptor),
      text: async () => ({ entry: { text: descriptor.meta.text }, meta: {} }),
    };
    const loader = loaders[kind];
    if (!loader) {
      throw new RuntimeError('unsupported_format', `The runtime cannot load \`${kind}\` assets`);
    }
    if (kind !== 'text' && source.type === 'inline') {
      throw new RuntimeError('asset_unreadable', `\`${id}\` has no file behind it`);
    }
    let loaded;
    try {
      loaded = await loader();
    } catch (error) {
      if (this.loading.get(id) === token) this.loading.delete(id);
      throw error;
    }
    const { entry, meta, after } = loaded;
    if (this.loading.get(id) !== token) {
      // Released (or reloaded) while loading: nothing of this load may outlive it.
      disposeSplatMeshes(entry);
      if (entry.video) closeVideo(entry.video);
      after?.close?.();
      throw new RuntimeError('cancelled', `Asset \`${id}\` was released while loading`, { asset: id });
    }
    this.loading.delete(id);
    if (descriptor.scene !== undefined) {
      const refusal = this.sceneRefusal(descriptor, entry);
      if (refusal) {
        entry.bitmap?.close?.();
        throw refusal;
      }
    }
    this.entries.set(id, { id, kind, descriptor, serial: (this.serials += 1), ...entry });
    this.generation += 1;
    if (after) {
      // The probe frame is kept as frame 0; its texture is made when a frame first draws it.
      entry.frames.set(0, { bitmap: after, texture: null, lastUsed: 0 });
    }
    return meta;
  }

  /** Entries loaded for a directed scene (`descriptor.scene`), other than the one under `exceptId`. */
  sceneEntries(exceptId = null) {
    return [...this.entries.values()].filter((entry) => entry.id !== exceptId && entry.descriptor?.scene !== undefined);
  }

  sceneResourceCount() {
    return this.sceneEntries().length;
  }

  /** Decoded RGBA bytes held for directed-scene pictures (the CPU side; GPU textures have their own budget). */
  sceneResourceBytes(exceptId = null) {
    return this.sceneEntries(exceptId).reduce((bytes, entry) => bytes + rgbaBytes(entry.width, entry.height), 0);
  }

  /**
   * Checks a directed-scene resource's identity before it loads (see `directed-scene.js`). A higher
   * generation than any seen releases every resource of the older ones: replacing the project is
   * an invalidation even when the host forgets to release one by one.
   */
  admitScene({ id, kind, scene }) {
    const identity = parseSceneIdentity(scene);
    if (kind !== 'still') {
      throw new RuntimeError('unsupported_format', `Directed-scene resources are still images, not \`${kind}\``, { asset: id });
    }
    if (identity.generation < this.sceneGeneration) {
      throw new RuntimeError(
        'resource_stale',
        `\`${identity.assetId}\` is of generation ${identity.generation}; the runtime is already at ${this.sceneGeneration}`,
        { asset_id: identity.assetId, generation: identity.generation, current: this.sceneGeneration },
      );
    }
    if (identity.generation > this.sceneGeneration) this.advanceSceneGeneration(identity.generation);
    const refusal = this.sceneEntryRefusal(id, identity);
    if (refusal) throw refusal;
  }

  sceneEntryRefusal(id, identity) {
    if (this.sceneEntries(id).length >= this.limits.sceneResourceEntries) {
      return new RuntimeError(
        'resource_budget',
        `${this.limits.sceneResourceEntries} directed-scene resources are loaded; release some before loading \`${identity.assetId}\``,
        { asset_id: identity.assetId, limit: this.limits.sceneResourceEntries },
      );
    }
    return null;
  }

  advanceSceneGeneration(generation) {
    this.sceneGeneration = generation;
    for (const entry of this.sceneEntries()) {
      if (entry.descriptor.scene.generation < generation) this.release(entry.id);
    }
  }

  /** After the bytes arrived: what `admitScene` could not know (the world moved on while loading, the size). */
  sceneRefusal({ id, scene }, entry) {
    const identity = parseSceneIdentity(scene);
    if (identity.generation < this.sceneGeneration) {
      return new RuntimeError('resource_stale', `\`${identity.assetId}\` finished loading after its generation was replaced`, {
        asset_id: identity.assetId,
        generation: identity.generation,
        current: this.sceneGeneration,
      });
    }
    const countRefusal = this.sceneEntryRefusal(id, identity);
    if (countRefusal) return countRefusal;
    const bytes = this.sceneResourceBytes(id) + rgbaBytes(entry.width, entry.height);
    if (bytes > this.limits.sceneResourceBytes) {
      return new RuntimeError(
        'resource_budget',
        `\`${identity.assetId}\` would take directed-scene pictures to ${bytes} bytes (limit ${this.limits.sceneResourceBytes})`,
        { asset_id: identity.assetId, bytes, limit: this.limits.sceneResourceBytes },
      );
    }
    return null;
  }

  release(id) {
    this.loading.delete(id);
    const entry = this.entries.get(id);
    if (!entry) return;
    disposeSplatMeshes(entry);
    this.disposeTextures(entry);
    entry.bitmap?.close?.();
    entry.frames?.forEach((frame) => frame.bitmap.close());
    if (entry.video) closeVideo(entry.video);
    this.entries.delete(id);
    this.generation += 1;
    this.releaseListeners.forEach((listener) => listener(id, entry));
  }

  /** Frees the GPU textures of an entry; the bitmaps stay, so a texture can be made again. */
  disposeTextures(entry) {
    entry.texture?.dispose();
    entry.texture = null;
    entry.frames?.forEach((frame) => {
      frame.texture?.dispose();
      frame.texture = null;
    });
  }

  /** GPU bytes of the image textures retained right now (stills and sequence frames). */
  textureBytes() {
    let bytes = 0;
    this.entries.forEach((entry) => {
      if (entry.kind === 'still' && entry.texture) bytes += rgbaBytes(entry.width, entry.height);
      entry.frames?.forEach((frame) => {
        if (frame.texture) bytes += rgbaBytes(frame.bitmap.width, frame.bitmap.height);
      });
    });
    return bytes;
  }

  /**
   * What the store holds, by category, in bytes (CPU unless noted). Splat figures: `decoded` is Spark's packed data (16 bytes a splat, CPU copy and GPU
   * texture alike) counted once per asset because clones share it. A report, never a limit.
   */
  memoryReport() {
    const report = {
      splat_assets: 0,
      splat_decoded_bytes: 0,
      splat_clones: 0,
      still_bitmap_bytes: 0,
      sequence_frames: 0,
      sequence_frame_bytes: 0,
      sequence_proxy_frames: 0,
      sequence_proxy_bytes: 0,
      scene_picture_bytes: this.sceneResourceBytes(),
      gpu_texture_bytes: this.textureBytes(),
    };
    this.entries.forEach((entry) => {
      if (entry.kind === 'splat') {
        report.splat_assets += 1;
        report.splat_decoded_bytes += (entry.mesh?.packedSplats?.numSplats ?? 0) * 16;
        report.splat_clones += entry.clones?.length ?? 0;
      }
      if (entry.kind === 'still' && entry.bitmap) report.still_bitmap_bytes += rgbaBytes(entry.width, entry.height);
      entry.frames?.forEach((frame, key) => {
        const bytes = rgbaBytes(frame.bitmap.width, frame.bitmap.height);
        if (isProxyKey(key)) {
          report.sequence_proxy_frames += 1;
          report.sequence_proxy_bytes += bytes;
        } else {
          report.sequence_frames += 1;
          report.sequence_frame_bytes += bytes;
        }
      });
    });
    return report;
  }

  /** Number of image textures retained right now. */
  textureCount() {
    let count = 0;
    this.entries.forEach((entry) => {
      if (entry.kind === 'still' && entry.texture) count += 1;
      entry.frames?.forEach((frame) => {
        if (frame.texture) count += 1;
      });
    });
    return count;
  }

  /**
   * Keeps the retained image textures inside the byte budget by freeing the least recently used
   * ones the frame did not pin. Only the GPU copy goes: the bitmap stays and the next use uploads again.
   */
  trimTextures() {
    let bytes = this.textureBytes();
    if (bytes <= this.limits.imageTextureBytes) return;
    const candidates = [];
    this.entries.forEach((entry) => {
      if (entry.kind === 'still' && entry.texture) {
        candidates.push({ lastUsed: entry.lastUsed ?? 0, bytes: rgbaBytes(entry.width, entry.height), owner: entry });
      }
      entry.frames?.forEach((frame) => {
        if (frame.texture) {
          candidates.push({ lastUsed: frame.lastUsed ?? 0, bytes: rgbaBytes(frame.bitmap.width, frame.bitmap.height), owner: frame });
        }
      });
    });
    candidates.sort((a, b) => a.lastUsed - b.lastUsed);
    for (const candidate of candidates) {
      if (bytes <= this.limits.imageTextureBytes) break;
      if (this.pinned.has(candidate.owner.texture)) continue;
      candidate.owner.texture.dispose();
      candidate.owner.texture = null;
      bytes -= candidate.bytes;
      this.counters.add('texture_evicted');
    }
  }

  /**
   * The retained texture of a still image, uploaded on the frame that first draws it. Not
   * created per frame: a texture that already exists is the same GPU object next time.
   */
  stillTexture(id) {
    const entry = this.get(id);
    entry.lastUsed = ++this.clock;
    if (entry.texture) {
      this.counters.add('texture_reused');
    } else {
      entry.texture = imageTexture(entry.bitmap);
      this.counters.add('texture_created');
    }
    this.pinned.add(entry.texture);
    return entry.texture;
  }

  /** Frees every retained image texture (retention switched off); bitmaps stay. */
  dropTextures() {
    this.entries.forEach((entry) => this.disposeTextures(entry));
  }

  releaseAll() {
    [...this.entries.keys()].forEach((id) => this.release(id));
  }

  async loadStill({ id, source }) {
    const label = `\`${id}\` (${source.path})`;
    const bitmap = await decodeImage(await fetchChecked(this.assetUrl(source.path), label), label);
    return {
      entry: { bitmap, width: bitmap.width, height: bitmap.height },
      meta: { width: bitmap.width, height: bitmap.height },
    };
  }

  async loadAudio({ id, source }) {
    // Audio is mixed offline by the core; the viewport only needs to know the file is there.
    // One byte proves it is readable; the whole file would pass through the editor's memory for nothing.
    const label = `\`${id}\` (${source.path})`;
    const response = await fetch(this.assetUrl(source.path), { headers: { Range: 'bytes=0-0' } }).catch((error) => {
      throw new RuntimeError('asset_unreadable', `Cannot fetch ${label}: ${error.message}`);
    });
    if (!response.ok) throw new RuntimeError('asset_unreadable', `Cannot read ${label}: HTTP ${response.status}`);
    await response.body?.cancel();
    return { entry: {}, meta: { readable: true } };
  }

  async loadSplat({ id, source, meta }) {
    const label = `\`${id}\` (${source.path})`;
    const bytes = await this.fetchSplatBytes(source, label);
    const fileName = splatFileName(meta.format, source.path);
    const mesh = await loadSplatMesh(bytes, fileName, label);
    const count = mesh.packedSplats.numSplats;
    // The file is NOT kept (66 MB for a 1.18 M splat PLY, as much again as the decoded splats): the GPU
    // side is rebuilt after a lost context by fetching it again (see splatLease and dropGpu).
    return {
      entry: { source, fileName, label, mesh, leased: false, clones: [] },
      meta: { splat_count: count, format: fileName.split('.').pop() },
    };
  }

  async fetchSplatBytes(source, label) {
    const response = await fetchChecked(this.assetUrl(source.path), label);
    return response.arrayBuffer();
  }

  /**
   * A splat mesh for one scene to draw. A mesh has a single parent, so the asset's own mesh goes
   * to the first taker; further takers (two objects, or layers of one frame) get a clone that
   * shares the decoded splats (no PLY parse, pack or upload of its own). Clones are kept on the
   * entry and reused by the next frame: building one per frame cost ~3.5 ms per layer.
   */
  async splatLease(id) {
    const entry = this.get(id);
    if (!entry.mesh) {
      const mesh = await loadSplatMesh(await this.fetchSplatBytes(entry.source, entry.label), entry.fileName, entry.label);
      if (this.entries.get(id) !== entry) {
        mesh.dispose();
        throw new RuntimeError('cancelled', `Asset \`${id}\` was released while loading`, { asset: id });
      }
      // Two takers may rebuild at once (a lost context): the first mesh stays, the other is not kept.
      if (entry.mesh) mesh.dispose();
      else entry.mesh = mesh;
    }
    if (!entry.leased) {
      entry.leased = true;
      const { mesh } = entry;
      return {
        mesh,
        release: () => {
          mesh.removeFromParent();
          entry.leased = false;
        },
      };
    }
    const mesh = entry.clones.pop() ?? cloneSplatMesh(entry.mesh);
    return {
      mesh,
      release: () => {
        mesh.removeFromParent();
        // An entry that was released or rebuilt meanwhile has already disposed what it kept.
        if (this.entries.get(id) === entry && entry.mesh) entry.clones.push(mesh);
      },
    };
  }

  /** Spark keeps splat data in textures of the WebGL context; after a loss they are rebuilt from the bytes. */
  dropGpu() {
    this.entries.forEach((entry) => {
      // Textures of the lost context are gone with it; they are made again from the bitmaps.
      this.disposeTextures(entry);
      if (entry.kind !== 'splat') return;
      disposeSplatMeshes(entry);
      entry.mesh = null;
      entry.clones = [];
      entry.leased = false;
    });
    this.pinned.clear();
    this.generation += 1;
  }

  async loadScene({ id, source }) {
    const label = `\`${id}\` (${source.path})`;
    const response = await fetchChecked(this.assetUrl(source.path), label);
    let scene;
    try {
      scene = JSON.parse(await response.text());
    } catch (error) {
      throw new RuntimeError('decode_failed', `${label} is not valid JSON: ${error.message}`);
    }
    if (scene.format !== 'tessel-scene' || scene.version !== 1 || !Array.isArray(scene.objects)) {
      throw new RuntimeError('decode_failed', `${label} is not a tessel-scene v1 file`);
    }
    return { entry: { scene }, meta: { objects: scene.objects.length } };
  }

  async loadVideo(descriptor) {
    const { source } = descriptor;
    if (source.type === 'frame_sequence') return this.loadFrameSequence(descriptor);
    return this.loadVideoFile(descriptor);
  }

  async loadFrameSequence({ id, source, meta }) {
    const first = await this.fetchFrame(source, source.first);
    const rate = meta.rate;
    return {
      entry: {
        sequence: source,
        frames: new Map(),
        width: first.width,
        height: first.height,
        rate,
      },
      meta: { width: first.width, height: first.height, frames: source.count, id },
      // The probe frame is kept as frame 0 so the first render does not decode it again.
      after: first,
    };
  }

  async fetchFrame(source, index) {
    const path = expandPattern(source.pattern, index);
    if (!path) {
      throw new RuntimeError('asset_unreadable', `Frame pattern \`${source.pattern}\` has no %d`);
    }
    const label = `frame ${index} (${path})`;
    return decodeImage(await fetchChecked(this.assetUrl(path), label), label);
  }

  /**
   * Texture of source frame `index` (0-based within the sequence); holds the last frame past the end.
   * `variant: 'proxy'` reads the same frame index from the attached preview proxy (see `setProxy`):
   * same index, same time, only fewer pixels.
   */
  async frameTexture(id, index, variant = 'original') {
    const entry = this.get(id);
    const proxy = variant === 'proxy' ? entry.proxy : null;
    if (variant === 'proxy' && !proxy) {
      throw new RuntimeError('invalid_proxy', `Asset \`${id}\` has no preview proxy attached`, { asset: id });
    }
    const source = proxy ?? entry.sequence;
    const clamped = Math.min(Math.max(0, index), entry.sequence.count - 1);
    const key = proxy ? proxyKey(clamped) : clamped;
    const cached = entry.frames.get(key);
    if (cached) {
      // Refresh recency: Map iteration order is insertion order.
      entry.frames.delete(key);
      entry.frames.set(key, cached);
      return this.frameTextureOf(cached);
    }
    const bitmap = await this.fetchFrame(source, source.first + clamped);
    // `load` of the same id replaces the entry (and a new proxy replaces the old one): this bitmap
    // belongs to a source that is gone.
    // Only the proxy variant is tied to the attached proxy: an original frame must still land while a
    // proxy is attached (exports and explicit frames run with one attached).
    if (!this.isCurrent(id, entry) || (proxy && entry.proxy !== proxy)) {
      bitmap.close();
      throw new RuntimeError('cancelled', `Asset \`${id}\` was released while loading`);
    }
    const frame = { bitmap, texture: null, lastUsed: 0 };
    entry.frames.set(key, frame);
    const texture = this.frameTextureOf(frame);
    this.trimFrames(entry);
    return texture;
  }

  /**
   * The frame texture a request of `quality` should draw, and which source it came from. A
   * `preview` request uses the proxy when one is attached; a proxy that cannot be read is dropped
   * and the original is drawn instead, reported in `proxyError` (the failure is never silent: the
   * frame report carries it and the proxy stays off until the host attaches a new one).
   */
  async qualityFrameTexture(id, index, quality) {
    const entry = this.get(id);
    if (quality !== 'preview' || !entry.proxy) {
      return { texture: await this.frameTexture(id, index), source: 'original' };
    }
    const proxy = entry.proxy;
    try {
      return { texture: await this.frameTexture(id, index, 'proxy'), source: 'proxy' };
    } catch (error) {
      if (error.code === 'cancelled' || (entry.proxy ?? null) !== proxy) throw error;
      this.dropProxy(entry);
      this.counters.add('proxy_failed');
      return {
        texture: await this.frameTexture(id, index),
        source: 'original',
        proxyError: error.toJSON?.() ?? { code: 'asset_unreadable', message: String(error?.message ?? error) },
      };
    }
  }

  /**
   * Attaches (or, with `null`, removes) a preview proxy of a frame sequence: the same frames,
   * numbered the same way, at a smaller size. The proxy never changes the asset's own size, so
   * layout and framing stay those of the original; only the pixels of a `preview` frame differ.
   */
  setProxy(id, proxy) {
    const entry = this.get(id);
    if (!entry.sequence) {
      throw new RuntimeError('unsupported_format', `Asset \`${id}\` is not a frame sequence; it has no preview proxy`, { asset: id });
    }
    this.dropProxy(entry);
    if (!proxy) return;
    if (typeof proxy.pattern !== 'string' || !Number.isInteger(proxy.first) || proxy.count !== entry.sequence.count) {
      throw new RuntimeError(
        'invalid_proxy',
        `The proxy of \`${id}\` must be a pattern with ${entry.sequence.count} frames numbered from an integer`,
        { asset: id, count: proxy.count, expected: entry.sequence.count },
      );
    }
    entry.proxy = { pattern: proxy.pattern, first: proxy.first, count: proxy.count };
    this.counters.add('proxy_attached');
  }

  /** Frees the proxy frames (bitmaps and textures) of an entry and forgets the proxy. */
  dropProxy(entry) {
    if (!entry.frames) return;
    for (const [key, frame] of [...entry.frames]) {
      if (!isProxyKey(key)) continue;
      frame.texture?.dispose();
      frame.bitmap.close();
      entry.frames.delete(key);
    }
    entry.proxy = null;
  }

  frameTextureOf(frame) {
    frame.lastUsed = ++this.clock;
    if (frame.texture) {
      this.counters.add('texture_reused');
    } else {
      frame.texture = imageTexture(frame.bitmap);
      this.counters.add('texture_created');
    }
    this.pinned.add(frame.texture);
    return frame.texture;
  }

  /**
   * Past the limit of its kind (FRAME_CACHE_LIMIT for the original's frames, PROXY_FRAME_CACHE_LIMIT
   * for the proxy's) the oldest frame goes (bitmap too), except one the frame in flight draws.
   */
  trimFrames(entry) {
    let proxies = 0;
    let originals = 0;
    for (const key of entry.frames.keys()) {
      if (isProxyKey(key)) proxies += 1;
      else originals += 1;
    }
    for (const [key, frame] of [...entry.frames]) {
      const proxied = isProxyKey(key);
      if ((proxied ? proxies : originals) <= (proxied ? PROXY_FRAME_CACHE_LIMIT : FRAME_CACHE_LIMIT)) continue;
      if (frame.texture && this.pinned.has(frame.texture)) continue;
      frame.texture?.dispose();
      frame.bitmap.close();
      entry.frames.delete(key);
      if (proxied) proxies -= 1;
      else originals -= 1;
    }
    this.trimBitmaps();
  }

  /**
   * The count limits above are per asset; a project with many 4K clips would still hold their
   * product in CPU bitmaps. Past `frameBitmapBytes` over all assets, the least recently used frames
   * (bitmap and texture) go, except the ones the frame in flight pins.
   */
  trimBitmaps() {
    let bytes = 0;
    const candidates = [];
    this.entries.forEach((entry) => {
      entry.frames?.forEach((frame, key) => {
        const size = rgbaBytes(frame.bitmap.width, frame.bitmap.height);
        bytes += size;
        candidates.push({ entry, key, frame, size });
      });
    });
    if (bytes <= this.limits.frameBitmapBytes) return;
    candidates.sort((a, b) => (a.frame.lastUsed ?? 0) - (b.frame.lastUsed ?? 0));
    for (const { entry, key, frame, size } of candidates) {
      if (bytes <= this.limits.frameBitmapBytes) break;
      if (frame.texture && this.pinned.has(frame.texture)) continue;
      frame.texture?.dispose();
      frame.bitmap.close();
      entry.frames.delete(key);
      bytes -= size;
      this.counters.add('frame_bitmap_evicted');
    }
  }

  /**
   * A container video read through a `<video>` element under the time contract of native-video.js.
   * Refuses what the contract cannot express (a conformed VFR clip) and what the engine cannot
   * show (proved by reading real pictures, not by `canPlayType` or the file extension).
   */
  async loadVideoFile({ id, source, meta }) {
    const label = `${id} (${source.path})`;
    if (meta.vfr_conformed) {
      throw new RuntimeError(
        'unsupported_timing',
        `${label} has a variable frame rate; the native video path needs a constant one. Import it so the media adapter conforms it to frames`,
        { asset: id },
      );
    }
    const rate = meta.rate;
    const seconds = meta.duration?.seconds ?? meta.duration;
    if (!rate?.num || !rate?.den || !(seconds > 0)) {
      throw new RuntimeError('unsupported_timing', `${label} needs a declared rate and duration`, { asset: id });
    }
    const video = await openVideo(this.assetUrl(source.path), label);
    const frames = frameCount(seconds, rate);
    let first;
    try {
      first = await proveVideo(video, { rate, label, frames });
    } catch (error) {
      closeVideo(video);
      throw error;
    }
    return {
      entry: {
        video,
        label,
        queue: Promise.resolve(),
        // Frames are addressed like a sequence's: 0-based indexes in the same bounded cache.
        native: { frames },
        frames: new Map(),
        width: video.videoWidth,
        height: video.videoHeight,
        rate,
        duration: video.duration,
      },
      meta: { width: video.videoWidth, height: video.videoHeight, duration: video.duration, frames },
      after: first,
    };
  }

  /** Texture of frame `index` of a container video: cached snapshot, or seek-and-read. */
  async videoFrameTexture(id, index, { aborted, timings }) {
    const entry = this.get(id);
    const clamped = Math.min(Math.max(0, index), entry.native.frames - 1);
    const cached = entry.frames.get(clamped);
    if (cached) {
      entry.frames.delete(clamped);
      entry.frames.set(clamped, cached);
      return this.frameTextureOf(cached);
    }
    const bitmap = await readFrame(entry, clamped, { aborted, timings });
    // A reload or release while the seek was in flight: the picture belongs to a source that is gone.
    if (!this.isCurrent(id, entry)) {
      bitmap.close();
      throw new RuntimeError('cancelled', `Asset \`${id}\` was released while loading`);
    }
    const frame = { bitmap, texture: null, lastUsed: 0 };
    entry.frames.set(clamped, frame);
    const texture = this.frameTextureOf(frame);
    this.trimFrames(entry);
    return texture;
  }
}

export { imageTexture };
