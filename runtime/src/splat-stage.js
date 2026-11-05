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

export class SplatStage {
  constructor(renderer) {
    // minSortIntervalMs 0: a sort is never deferred to a timer, which `settle` relies on.
    // autoUpdate false: only `settle` sorts, for the camera of the frame in flight.
    this.spark = new SparkRenderer({ renderer, minSortIntervalMs: 0, autoUpdate: false });
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
