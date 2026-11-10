// BL157.3 (opt-in, `hello.preupload_textures`): the GPU upload of a still image's texture before the clip that shows it starts.
//
// WHY: in the real project the first draw of a PNG clip costs 13-27 ms of `texSubImage2D` plus the draw (`gl_call` spans
// of BL157.2, frames 195->196: gaps of 63-67 ms at 24 fps), because a retained texture (assets.js `stillTexture`) is made
// on the frame that first draws it. The host sends `frame.warm` with `textures: true` for the cuts a moment ahead, and the
// runtime uploads the textures of the still layers of that plan that are already loaded, one texture per step, behind any
// frame that is waiting.
//
// What this never does (each is a test in `preupload-browser.test.mjs`):
//  - draw: `initTexture` only uploads, so the visible canvas is untouched and no frame of the future is drawn;
//  - upload twice: a resident texture is `resident`, not uploaded again, and the frame that draws it finds it;
//  - take what is not its own: it only makes the texture of the entry that was loaded when the warm was asked (its serial);
//    an entry released or replaced meanwhile is `stale`/`missing`, and the new one waits for its own warm or its own frame;
//  - exceed the texture byte budget (`limits.imageTextureBytes`): it asks for no eviction, an upload that does not fit is
//    `budget` and the frame uploads it as it always did;
//  - outlive the world it was asked for: a lost context, a retention switch-off, a dispose or a time limit drops the queue.
//
// The queue is bounded (`PREUPLOAD_PENDING_LIMIT`), each item expires (`PREUPLOAD_MAX_AGE_MS`) and each step is its own task of
// the page's serial queue, re-queued behind whatever arrived meanwhile: a frame the host asked for goes first.

/** Pending uploads kept; a warm that would add more is counted as `dropped`, not queued. */
export const PREUPLOAD_PENDING_LIMIT = 8;
/** A queued upload older than this belongs to a plan the playhead has left (the host looks 2 s ahead). */
export const PREUPLOAD_MAX_AGE_MS = 3000;

/**
 * The still layers of `plan` a frame will draw whose texture can be made now: the asset is a loaded still, not covered by a
 * layer above (`occluded`) and without a texture yet. One item per asset, with the serial of the entry that is loaded now.
 */
export const stillsToUpload = (plan, assets) => {
  const seen = new Set();
  const items = [];
  for (const layer of plan?.layers ?? []) {
    if (layer.kind !== 'image' || layer.occluded === true || typeof layer.asset !== 'string' || seen.has(layer.asset)) continue;
    seen.add(layer.asset);
    const entry = assets.entries.get(layer.asset);
    if (entry?.kind !== 'still' || !entry.bitmap || entry.texture) continue;
    items.push({ id: layer.asset, serial: entry.serial });
  }
  return items;
};

export class TexturePreupload {
  constructor({ now = () => performance.now() } = {}) {
    this.now = now;
    this.pending = [];
    this.stats = {
      requested: 0,
      uploaded: 0,
      resident: 0,
      budget: 0,
      stale: 0,
      missing: 0,
      expired: 0,
      dropped: 0,
      cleared: 0,
      failed: 0,
      upload_ms: 0,
      max_upload_ms: 0,
    };
  }

  /** Queues `items` (see `stillsToUpload`); returns how many are pending. */
  enqueue(items) {
    const at = this.now();
    for (const item of items) {
      this.stats.requested += 1;
      if (this.pending.some((queued) => queued.id === item.id && queued.serial === item.serial)) continue;
      if (this.pending.length >= PREUPLOAD_PENDING_LIMIT) {
        this.stats.dropped += 1;
        continue;
      }
      this.pending.push({ ...item, at });
    }
    return this.pending.length;
  }

  /**
   * Does the oldest pending upload, one texture and nothing else, and says what happened: `{ outcome, id, ms }`, or `null`
   * with nothing pending. `upload(texture)` is the GPU upload (the compositor's); a throw is `failed` and leaves no texture.
   */
  step(assets, upload) {
    const item = this.pending.shift();
    if (!item) return null;
    if (this.now() - item.at > PREUPLOAD_MAX_AGE_MS) {
      this.stats.expired += 1;
      return { outcome: 'expired', id: item.id, ms: 0 };
    }
    const started = this.now();
    let outcome;
    try {
      outcome = assets.preuploadStill(item.id, item.serial, upload);
    } catch {
      outcome = 'failed';
    }
    const ms = this.now() - started;
    this.stats[outcome] += 1;
    if (outcome === 'uploaded') {
      this.stats.upload_ms += ms;
      this.stats.max_upload_ms = Math.max(this.stats.max_upload_ms, ms);
    }
    return { outcome, id: item.id, ms };
  }

  /** Forgets everything pending: the world it was asked for is gone (context loss, retention off, dispose, a new hello). */
  clear() {
    this.stats.cleared += this.pending.length;
    this.pending = [];
  }

  report() {
    return { pending: this.pending.length, ...this.stats };
  }
}
