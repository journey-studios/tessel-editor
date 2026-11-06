// Bounded retention of GPU-side objects between frames.
//
// WHY: measured on a real WKWebView (docs/viewport-performance-opus-review.md §1) the 9 ms of
// a still-image preview came from re-uploading the texture and from destroying/relinking the
// shader program every frame, not from building JS objects. Keeping the texture and the
// materials alive removes both. Everything kept here has an owner, a bound and a way to die:
//
//   resource            owner                    key                       dies on
//   image texture       AssetStore entry         the entry itself          release/load of the id, context loss, budget (LRU)
//   text texture        Compositor.text          content+size+color+height context loss, count/byte LRU
//   layer material      Compositor.materials     blend + layer slot        context loss, count LRU
//   scene rig (no splat) Compositor.rigs         scene asset id + entry    release/load of the id, context loss, count LRU
//   scene render target Compositor.targets       slot + size               size change, context loss, count/byte LRU
//
// The one exception to "nothing here caches a picture" is the RESULT of a scene layer with splats
// (compositor.js, `sceneResultKey`): its render target remembers the key of the picture it holds
// and a frame whose key is the same skips the Spark sort and the draw. The key is the full list of
// what the picture depends on, so the pixels stay a pure function of (plan, assets); the
// `result_cache` hello option turns it off to prove that.

export const MIB = 1024 * 1024;

/** Upper bounds. Past them the least recently used entry that this frame does not need is freed. */
export const LIMITS = {
  /** GPU bytes of retained image textures (stills and sequence frames); evicted ones re-upload from the bitmap. */
  imageTextureBytes: 256 * MIB,
  textEntries: 32,
  textBytes: 32 * MIB,
  materials: 24,
  rigs: 4,
  targets: 2,
  targetBytes: 192 * MIB,
  /** Directed-scene pictures kept decoded (CPU side), by count and RGBA bytes; a load past either is refused. */
  sceneResourceEntries: 64,
  sceneResourceBytes: 768 * MIB,
  /**
   * Splats a directed-scene frame may draw in all: the sum over its splat nodes of the splats of the
   * file each reads (a file used by many nodes counts once per node, since each is sorted and drawn).
   * 16 bytes per splat of packed data per node is what the frame's own budget would hold; a frame past
   * this is refused (`resource_budget`) before anything is drawn. The host applies the same number.
   */
  frameSplats: 8 * 1024 * 1024,
};

/** Event counters of the retention layer; `delta` gives what one frame did. */
export class RetentionCounters {
  constructor() {
    this.values = {
      texture_created: 0,
      texture_reused: 0,
      texture_evicted: 0,
      text_created: 0,
      text_reused: 0,
      text_evicted: 0,
      material_created: 0,
      material_reused: 0,
      material_evicted: 0,
      rig_built: 0,
      rig_reused: 0,
      rig_evicted: 0,
      target_created: 0,
      target_reused: 0,
      target_evicted: 0,
      // Preview proxies (assets.js) and the result cache of static splat scenes (compositor.js).
      proxy_attached: 0,
      proxy_failed: 0,
      result_hit: 0,
      result_miss: 0,
      result_invalidated: 0,
    };
  }

  add(name, amount = 1) {
    this.values[name] += amount;
  }

  snapshot() {
    return { ...this.values };
  }

  delta(since) {
    return Object.fromEntries(Object.entries(this.values).map(([name, value]) => [name, value - since[name]]));
  }
}

/**
 * Least-recently-used map with an entry and a byte bound. `touch` marks an entry as needed by
 * the frame in flight; `trim` runs after the frame and never frees what that frame used, so a
 * resource cannot disappear under a mesh that is about to be drawn.
 */
export class LruCache {
  constructor({ maxEntries, maxBytes = Infinity, dispose, onEvict = () => {} }) {
    this.maxEntries = maxEntries;
    this.maxBytes = maxBytes;
    this.disposeValue = dispose;
    this.onEvict = onEvict;
    // Map order is recency order: the first key is the least recently used.
    this.entries = new Map();
    this.used = new Set();
    this.bytes = 0;
  }

  get size() {
    return this.entries.size;
  }

  beginFrame() {
    this.used.clear();
  }

  get(key) {
    const entry = this.entries.get(key);
    if (!entry) return undefined;
    this.entries.delete(key);
    this.entries.set(key, entry);
    this.used.add(key);
    return entry.value;
  }

  set(key, value, bytes = 0) {
    this.delete(key);
    this.entries.set(key, { value, bytes });
    this.bytes += bytes;
    this.used.add(key);
  }

  delete(key) {
    const entry = this.entries.get(key);
    if (!entry) return false;
    this.entries.delete(key);
    this.used.delete(key);
    this.bytes -= entry.bytes;
    this.disposeValue(entry.value);
    return true;
  }

  /** Frees least-recently-used entries the last frame did not touch until both bounds hold. */
  trim() {
    for (const key of [...this.entries.keys()]) {
      if (this.entries.size <= this.maxEntries && this.bytes <= this.maxBytes) break;
      if (this.used.has(key)) continue;
      this.delete(key);
      this.onEvict();
    }
  }

  /** Frees every entry whose key matches; used when an asset is released. */
  deleteWhere(predicate) {
    for (const [key, entry] of [...this.entries]) {
      if (predicate(key, entry.value)) this.delete(key);
    }
  }

  clear() {
    for (const key of [...this.entries.keys()]) this.delete(key);
  }
}

/** Byte size of a texture that holds `width`x`height` RGBA8 pixels (no mipmaps are generated). */
export const rgbaBytes = (width, height) => width * height * 4;

/** Estimated GPU bytes of one MSAA render target: samples x (color + depth) plus the resolve texture. */
export const targetBytes = (width, height, samples) => width * height * 4 * (samples * 2 + 1);
