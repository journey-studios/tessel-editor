// Gaussian splats through Spark (@sparkjsdev/spark 2.4.0), bundled into runtime.js: no CDN, no
// network. Spark sorts splats per camera on a worker and draws them from its own SparkRenderer
// mesh inside the scene being rendered; this module owns the parts that make a frame a pure
// function of (plan, time): one SparkRenderer per WebGL context, and the settle step that waits
// until the sort belongs to exactly the camera of the frame being drawn.
//
// Formats: `.splat`, `.ply` and `.spz` (decoded by Spark from the bytes the core identified).

import { SparkRenderer, SplatMesh } from '@sparkjsdev/spark';

import { RuntimeError } from './protocol.js';

export const SPARK_VERSION = '2.4.0';
/** Sorts a frame may start before it is refused: a frame sorted for another viewpoint is never drawn. */
const MAX_SORT_ATTEMPTS = 400;
/**
 * Longest a frame waits for sorts it did not start (one a moving playhead left running) before it is refused.
 * A sort is not a pass: it takes as long as the GPU readback and the worker take, which on a loaded machine
 * or a software renderer is seconds, so the wait is bounded by time and not by polls.
 */
const MAX_SORT_WAIT_MS = 60_000;
const SETTLE_DISTANCE = 1e-4;
/** How often a wait for a sort looks at a cancel and at the budget; the sort itself wakes the wait the moment it ends. */
const WAIT_POLL_MS = 25;

/** Decodes splat bytes into a mesh whose data is ready; Spark's own errors become decode_failed. */
export const loadSplatMesh = async (bytes, fileName, label) => {
  // No copy of `bytes` here: Spark copies what it sends to its worker, so a caller's buffer is only read.
  const mesh = new SplatMesh({ fileBytes: bytes, fileName });
  try {
    await mesh.initialized;
  } catch (error) {
    mesh.dispose();
    throw new RuntimeError('decode_failed', `Cannot decode splats of ${label}: ${error?.message ?? error}`);
  }
  const count = mesh.packedSplats?.numSplats ?? 0;
  if (count === 0) {
    mesh.dispose();
    throw new RuntimeError('decode_failed', `${label} holds no splats`);
  }
  return mesh;
};

/**
 * Another mesh over the packed splats `source` already decoded (BL125): several nodes that read one
 * file share one decode and one set of GPU textures; each mesh has its own transform and its own
 * Spark generator. The encoding is passed through so the shared data is not re-labelled.
 */
export const shareSplatMesh = (source) =>
  new SplatMesh({ packedSplats: source.packedSplats, splatEncoding: source.packedSplats.splatEncoding });

/**
 * Drops a mesh made by `shareSplatMesh` without touching the data it borrowed: `SplatMesh.dispose`
 * disposes its packed splats, which belong to the mesh that decoded them.
 */
export const disposeSharedSplatMesh = (mesh) => {
  mesh.removeFromParent();
  mesh.splats = undefined;
  mesh.packedSplats = undefined;
  mesh.dispose();
};

/**
 * Gives a lent mesh back in the state a new one has (BL131): not in any scene, no transform of the
 * use it just served. Every borrower sets the pose it needs, but one that only sets some of it must
 * never inherit the rest from the last use (a different position, scale, spin or matrix).
 */
export const resetLentMesh = (mesh) => {
  mesh.removeFromParent();
  mesh.matrixAutoUpdate = true;
  mesh.matrix.identity();
  mesh.position.set(0, 0, 0);
  mesh.quaternion.identity();
  mesh.scale.set(1, 1, 1);
};

export class SplatStage {
  constructor(renderer) {
    // minSortIntervalMs 0: a sort is never deferred to a timer, which `settle` relies on.
    // autoUpdate false: only `settle` sorts, for the camera of the frame in flight.
    this.spark = new SparkRenderer({ renderer, minSortIntervalMs: 0, autoUpdate: false });
    this.keepStoredColour();
    // Why this stage can no longer be used, once it cannot (see `watchSort`); `null` while it is sound.
    this.failure = null;
    // Longest one frame waits for sorts (a field only so that a test can shorten it).
    this.maxWaitMs = MAX_SORT_WAIT_MS;
    this.watchSort();
  }

  /**
   * Makes a failed sort observable. Spark 2.4.0's `driveSort` sets `sorting = true`, then awaits the depth
   * readback and the sort worker with no cleanup: when either rejects, `sorting` stays true for good, so
   * nothing is ever sorted again and a wait for an idle sort would spend its whole budget on an error that
   * already happened. The rejection reaches the caller of `update` when that call ran the sort, and
   * otherwise (the sort Spark chains after a finished one, or starts from its own timer) goes to
   * `dropErrorIfDisposed`, which rethrows into a promise nobody holds: an unhandled rejection. Spark gives
   * no way to reset that flag and nothing here writes to it: the stage is recorded as failed and the owner
   * drops it (`failed`), so the next frame sorts from a new SparkRenderer and never from this state.
   */
  watchSort() {
    const spark = this.spark;
    spark.dropErrorIfDisposed = (error) => {
      if (!spark.isDisposed) this.fail(error);
    };
  }

  /** Records why the stage is unusable; the first cause is the one kept. */
  fail(error) {
    this.failure ??= error instanceof Error ? error : new Error(String(error));
  }

  get failed() {
    return this.failure !== null;
  }

  /** A frame that asks a failed stage for a sort gets the failure, at once: it never waits for the sort that cannot finish. */
  throwIfFailed() {
    if (this.failure === null) return;
    const message = this.failure?.message ?? String(this.failure);
    throw new RuntimeError(
      'render_failed',
      `The splat sort failed (${message}); the sort state is dropped and the next frame sorts again from a new one`,
      { cause: message },
    );
  }

  /**
   * `spark.update` for `camera`, whose failure is recorded on the stage instead of being thrown: callers
   * check `throwIfFailed` after waiting, and a call nobody waits for (a moving frame's sort, a frame that
   * was cancelled meanwhile) cannot leave an unhandled rejection behind.
   */
  begin(scene, camera) {
    return this.spark.update({ scene, camera }).catch((error) => this.fail(error));
  }

  /**
   * BL019-F1: every splat frame is drawn into a render target, and for a render target Spark 2.4.0
   * does not look at `renderer.outputColorSpace` (SparkRenderer.ts, `outputColorSpace` of the draw):
   * it takes `THREE.ColorManagement.workingColorSpace`, which is linear, so `encodeLinear` is turned
   * on at every draw and each splat colour is decoded with sRGB->linear. Nothing re-encodes it here
   * (the compositor writes the working space as it is so that pictures reach the output untouched;
   * `outputColorSpace = LinearSRGBColorSpace` is that choice, not what decides Spark's flag): a
   * stored (230,49,49) was drawn as (203,7,7). A splat colour is an sRGB value like a picture's, so
   * the uniform is held at false after Spark's own per-draw update and the stored colour is the
   * displayed one. This touches only Spark's splat material, in both the project (`scene`) path and
   * the directed-scene path.
   */
  keepStoredColour() {
    const spark = this.spark;
    const original = spark.onBeforeRender.bind(spark);
    spark.onBeforeRender = (...args) => {
      original(...args);
      spark.uniforms.encodeLinear.value = false;
    };
  }

  attach(scene) {
    scene.add(this.spark);
  }

  detach(scene) {
    scene.remove(this.spark);
  }

  /**
   * Sorts the splats of `scene` for `camera` and waits until the uploaded sort is that one, so a
   * frame reached by an out-of-order seek is the same image as one reached in order.
   *
   * `motion` (BL130) is for a frame of a moving playhead only (`quality: 'preview'`): the wait for the
   * sort is what made a splat frame cost two sorts' worth of CPU and GPU readback on the critical path,
   * so the frame is drawn at once with the order of the last finished sort and the sort for this
   * camera runs behind it. That is only valid when Spark already holds an ordering for exactly this set
   * of splats (same mapping): the order is for another viewpoint but indexes the same splats, whose
   * positions are this frame's. Anything else (the first frame of a scene, a changed mapping, another
   * scene's data still on display) is not drawn from the old data: it waits like an exact frame.
   * Resolves `{ stale }`: whether the picture was drawn with an order that is not the one for this
   * camera. The host replaces such a picture by an exact one when the playhead stops.
   */
  async settle(scene, camera, aborted, { motion = false } = {}) {
    // A sort that already failed is this frame's failure too, whatever the frame is (and a moving one does
    // not draw with the order of a sort that can never finish).
    this.throwIfFailed();
    const target = camera.getWorldPosition(camera.position.clone());
    const waitUntil = performance.now() + this.maxWaitMs;
    // The first pass of the wait below, when the motion path already ran it.
    let started = null;
    if (motion) {
      if (aborted()) throw new RuntimeError('cancelled', 'The frame was superseded');
      // The part of `update` that does not wait (this frame's splats are generated, the sort for this
      // camera starts) runs now; what is left of it is the sort, which this frame does not wait for.
      // A failure of that sort is recorded on the stage: the next request for it gets the error.
      started = this.begin(scene, camera);
      if (this.holdsOrderFor()) return { stale: !this.isSortedFor(target) };
    }
    let sorts = 0;
    for (;;) {
      if (aborted()) throw new RuntimeError('cancelled', 'The frame was superseded');
      this.throwIfFailed();
      // A sort already running (one a moving playhead left behind) is waited for, not fed: every
      // `update` marks the sort dirty again, so asking while one runs keeps a new sort queued behind
      // it and the order would never be idle for this camera.
      if (started) {
        // Nothing to reuse (first frame of a scene, other data on display): wait for its own sort,
        // as an exact frame does, without a timer in between.
        await this.waitSort(started, aborted, waitUntil);
        started = null;
        sorts += 1;
      } else if (!this.spark.sorting) {
        if (sorts >= MAX_SORT_ATTEMPTS) break;
        sorts += 1;
        await this.waitSort(this.begin(scene, camera), aborted, waitUntil);
      }
      if (this.isSortedFor(target)) return { stale: false };
      if (performance.now() > waitUntil) break;
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    throw this.retire(`The splat sort did not settle after ${sorts} sorts for the requested camera; the frame was not drawn`);
  }

  /**
   * Waits for `sort` (a `begin` promise, which never rejects) without letting it hold the frame: a frame that is
   * cancelled stops waiting at once, a sort that does not finish within the budget fails the frame, and a sort that
   * failed is reported as soon as it is recorded. The promise itself is never abandoned unobserved.
   */
  async waitSort(sort, aborted, waitUntil) {
    let finished = false;
    sort.then(() => {
      finished = true;
    });
    while (!finished) {
      if (aborted()) throw new RuntimeError('cancelled', 'The frame was superseded');
      this.throwIfFailed();
      if (performance.now() > waitUntil) throw this.retire(`The splat sort did not finish within ${this.maxWaitMs} ms; the frame was not drawn`);
      await Promise.race([sort, new Promise((resolve) => setTimeout(resolve, WAIT_POLL_MS))]);
    }
    this.throwIfFailed();
  }

  /**
   * Out of budget: whether the sort is slow or stuck is not known, and the next frame must not spend the same
   * budget on it again, nor draw from what it left. The stage is recorded as failed; its owner replaces it.
   */
  retire(message) {
    this.fail(new Error(message));
    return new RuntimeError('render_failed', message);
  }

  /** The uploaded sort is the one for `target`, and none is running or wanted. */
  isSortedFor(target) {
    const idle = !(this.spark.sorting || this.spark.sortDirty);
    return idle && this.spark.sortedCenter.distanceTo(target) < SETTLE_DISTANCE;
  }

  /**
   * What is on display is this frame's own set of splats and an order already exists for it: the draw
   * would use the splats just generated with the last finished sort. False while Spark still shows the
   * splats of another mapping (it keeps them until a sort of the new one lands) or has never sorted.
   */
  holdsOrderFor() {
    const spark = this.spark;
    return (
      spark.orderingTexture != null &&
      Number.isFinite(spark.sortedCenter.x) &&
      spark.display.mappingVersion === spark.latestMappingVersion
    );
  }

  dispose() {
    this.spark.removeFromParent();
    this.spark.dispose();
  }
}
