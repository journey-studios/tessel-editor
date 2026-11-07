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

/** Decodes splat bytes into a mesh whose data is ready; Spark's own errors become decode_failed. */
export const loadSplatMesh = async (bytes, fileName, label) => {
  const mesh = new SplatMesh({ fileBytes: bytes.slice(0), fileName });
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
    const target = camera.getWorldPosition(camera.position.clone());
    // The first pass of the wait below, when the motion path already ran it.
    let started = null;
    if (motion) {
      if (aborted()) throw new RuntimeError('cancelled', 'The frame was superseded');
      // The part of `update` that does not wait (this frame's splats are generated, the sort for this
      // camera starts) runs now; what is left of it is the sort, which this frame does not wait for.
      started = this.spark.update({ scene, camera });
      // A failure of that sort is not this frame's: an exact frame repeats the call and reports it.
      started.catch(() => {});
      if (this.holdsOrderFor()) return { stale: !this.isSortedFor(target) };
    }
    let sorts = 0;
    const waitUntil = performance.now() + MAX_SORT_WAIT_MS;
    for (;;) {
      if (aborted()) throw new RuntimeError('cancelled', 'The frame was superseded');
      // A sort already running (one a moving playhead left behind) is waited for, not fed: every
      // `update` marks the sort dirty again, so asking while one runs keeps a new sort queued behind
      // it and the order would never be idle for this camera.
      if (started) {
        // Nothing to reuse (first frame of a scene, other data on display): wait for its own sort,
        // as an exact frame does, without a timer in between.
        await started;
        started = null;
        sorts += 1;
      } else if (!this.spark.sorting) {
        if (sorts >= MAX_SORT_ATTEMPTS) break;
        sorts += 1;
        await this.spark.update({ scene, camera });
      }
      if (this.isSortedFor(target)) return { stale: false };
      if (performance.now() > waitUntil) break;
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    throw new RuntimeError(
      'render_failed',
      `The splat sort did not settle after ${sorts} sorts for the requested camera; the frame was not drawn`,
    );
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
