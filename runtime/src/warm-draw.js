// BL157.3 (second, separate experiment, opt-in, `hello.warm_draw`): the warm DRAW. After the warm sort of a splat scene the page draws
// that scene ONCE into a scratch render target, so what a first draw of a scene makes the GPU do (its splat data, its program
// variant) is paid while the frames before the cut play and not by the frame at the cut. It is the draw of a frame without the wait
// for a sort: the stage already holds the order the warm made.
//
// What keeps it from being a change of the picture: the target is its own (16 px wide, no samples, never one of the retained
// targets, freed in every case), nothing it draws is kept, the canvas and the retained targets and results are not touched, and
// the stage, the meshes and the sort lineage are the warm's own, which the frame at the cut takes over as before. What keeps it
// from costing a frame: the page runs it as a task of its serial queue behind any waiting frame (index.js), and it does nothing
// for a lineage whose stage was replaced, taken over by a frame, or lost with the context.
//
// Not part of the compositor: `Compositor.warmScenes` only hands each warm it starts to the `onWarmed` it was given, and this file
// reads what a draw must not disturb (the lineage's stage, its mesh owners, the context) from the compositor it is asked about.

import * as THREE from 'three';

import { SceneRig } from './scene.js';

/** Width of the scratch target of a warm draw: the draw is for what the GPU sets up, not for the pixels. */
export const WARM_DRAW_WIDTH = 16;

/**
 * The switch and the queue of the warm draws: the warms started since the last `take` that a draw may follow. Off, nothing is
 * recorded, and turning it off forgets what was recorded.
 */
export class WarmDraws {
  constructor() {
    this.enabled = false;
    this.pending = [];
  }

  setEnabled(enabled) {
    this.enabled = enabled === true;
    if (!this.enabled) this.clear();
  }

  /** What `Compositor.warmScenes` hands over for every warm it starts: `{ layer, scene, lineage, stage, sorted, width, height }`. */
  record(item) {
    if (this.enabled) this.pending.push({ ...item, at: performance.now() });
  }

  /** The warms recorded since the last call; each is handed out once. */
  take() {
    const taken = this.pending;
    this.pending = [];
    return taken;
  }

  /** Forgets everything recorded: the world it was recorded for is gone (context loss, a switch off, dispose). */
  clear() {
    this.pending = [];
  }
}

/**
 * Draws the scene `item` (one warm of `WarmDraws`) once into a scratch target of `compositor`. Resolves the outcome: `drawn`, or
 * why it was not (`gone`, `taken`, `no_order`, `lost`, `failed`).
 */
export const drawWarmed = async (compositor, item, assets) => {
  const { lineage, layer, scene, stage, width, height } = item;
  if (!compositor.retain || compositor.gl.isContextLost()) return 'lost';
  if (stage.failed || compositor.stages.entries.get(lineage)?.value !== stage) return 'gone';
  // A frame that drew the lineage took it over (`planLineages`); a warm that never owned it has nothing to draw.
  if (!compositor.warmOwned.has(lineage) || compositor.warming.has(lineage)) return 'taken';
  if (!stage.holdsOrderFor()) return 'no_order';
  let rig = null;
  let target = null;
  let attached = false;
  try {
    assets.reserveSplatOwners([...compositor.drawnOwners, ...compositor.warmedOwners()]);
    rig = await SceneRig.build(scene, assets, lineage);
    if (compositor.gl.isContextLost() || compositor.stages.entries.get(lineage)?.value !== stage || !compositor.warmOwned.has(lineage)) return 'taken';
    const high = Math.max(1, Math.round((WARM_DRAW_WIDTH * height) / width));
    const camera = SceneRig.camera(layer.camera, WARM_DRAW_WIDTH, high);
    rig.pose(layer.scene_time?.seconds ?? 0);
    stage.attach(rig.scene);
    attached = true;
    target = new THREE.WebGLRenderTarget(WARM_DRAW_WIDTH, high, { samples: 0, depthBuffer: true, colorSpace: THREE.NoColorSpace });
    rig.render(compositor.renderer, target, camera);
    return 'drawn';
  } catch {
    return compositor.gl.isContextLost() ? 'lost' : 'failed';
  } finally {
    try {
      if (attached) stage.detach(rig.scene);
      rig?.dispose();
      target?.dispose();
    } catch {
      // The context is gone with what it held.
    }
    assets.reserveSplatOwners([]);
  }
};
