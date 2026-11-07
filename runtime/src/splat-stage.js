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
/** Sort attempts before a frame is refused: a frame sorted for another viewpoint is never drawn. */
const MAX_SORT_ATTEMPTS = 400;
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
   */
  async settle(scene, camera, aborted) {
    const target = camera.getWorldPosition(camera.position.clone());
    for (let attempt = 0; attempt < MAX_SORT_ATTEMPTS; attempt += 1) {
      if (aborted()) throw new RuntimeError('cancelled', 'The frame was superseded');
      await this.spark.update({ scene, camera });
      const idle = !(this.spark.sorting || this.spark.sortDirty);
      if (idle && this.spark.sortedCenter.distanceTo(target) < SETTLE_DISTANCE) return;
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    throw new RuntimeError(
      'render_failed',
      `The splat sort did not settle after ${MAX_SORT_ATTEMPTS} passes for the requested camera; the frame was not drawn`,
    );
  }

  dispose() {
    this.spark.removeFromParent();
    this.spark.dispose();
  }
}
