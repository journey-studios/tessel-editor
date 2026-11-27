// The draw of a directed scene plan (directed-scene.js), moved out of the compositor so that module
// stays small (P4b S2-R1a). A mechanical extraction: the bodies are those `Compositor.prepareDirectedScene`
// and `Compositor.sceneMaterial` had, with `this` read as `host`, the compositor that owns the targets, the
// stage, the effect pipeline, the material cache and the counters. The compositor keeps both methods as thin
// wrappers over these functions. Nothing here keeps state of its own, and nothing changed: the order of the
// checks and of the actions, the keys (`ds:`, `dsfx:`), the lifetimes (everything a frame builds goes to its
// `keep`) and the errors are the ones the methods had.

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
import { RuntimeError } from './protocol.js';

/**
 * A layer that is the evaluated plan of a directed scene (directed-scene.js). The plan is
 * checked whole before anything is drawn; its pictures are the store's own retained textures
 * (so release, reload, context loss and the byte budget invalidate them like any other still);
 * the picture goes to an offscreen target and is composited like a `scene` layer.
 */
export async function prepareDirectedScene(host, layer, frame) {
  const { assets, keep, width, height, aborted, used } = frame;
  const spec = parseDirectedLayer(layer);
  const bound = resolveSceneResources(spec, assets);
  // A slot replaced before the draw makes the frame cancelled, never a mix of two revisions.
  bound.forEach(({ slot, entry }) => used.set(slot, entry));
  if (aborted()) throw new RuntimeError('cancelled', 'The frame was superseded');

  const size = containSize(spec.output, width, height);
  // Its own slot space (`ds:`): a directed scene never shares a target, or a cached result, with a `scene` layer.
  // It always draws at the final MSAA: the reduced preview is the project `scene` layer's (BL129).
  const { target, cached } = host.sceneTarget(`ds:${frame.order}`, size.width, size.height, keep);
  if (cached) cached.result = null;
  const textureOf = (assetId) => {
    const { slot, entry } = bound.get(assetId);
    return host.retain ? assets.stillTexture(slot) : keep(imageTexture(entry.bitmap));
  };
  const directed = buildDirectedScene(spec, {
    textureOf,
    materialOf: (alpha, index, texture) => host.sceneMaterial(frame.order, alpha, index, texture, keep),
    keep,
  });
  // What the frame will sort and draw: every node counts the splats of its file, however many nodes
  // share one decode (the data is shared, the draw is not). Refused before any lease or draw.
  const frameSplats = spec.splats.reduce((total, splat) => total + (bound.get(splat.assetId).entry.count ?? 0), 0);
  if (frameSplats > host.limits.frameSplats) {
    throw new RuntimeError(
      'resource_budget',
      `The frame would draw ${frameSplats} splats across ${spec.splats.length} splat nodes (limit ${host.limits.frameSplats})`,
      { frame_splats: frameSplats, nodes: spec.splats.length, limit: host.limits.frameSplats },
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
  const stage = splatScenes.length > 0 ? host.splatStage() : null;
  try {
    await drawDirectedScene(host.renderer, target, { ...directed, splatScenes }, { stage, aborted });
  } catch (error) {
    host.discardFailedStage(stage, null);
    throw error;
  }
  frame.sources.directed_scene_layers = (frame.sources.directed_scene_layers ?? 0) + 1;
  if (splatScenes.length > 0) frame.sources.directed_scene_splats = (frame.sources.directed_scene_splats ?? 0) + spec.splats.length;
  // The effects of the plan (F016, and of a shot, F062): the core resolved them for this frame, they
  // are drawn here over the finished picture, the shot's first and the output's after, each stage in
  // the plan's order. The pipeline and its materials are kept
  // (no relink per frame, A1); the passes ping-pong between two plain targets (A5), so any number of
  // passes holds two. A target slot is looked up once per draw, so a target too big to retain is still
  // one object per slot, never one per pass.
  let texture = target.texture;
  if (spec.fx.length > 0) {
    const pipeline = host.fxPipeline(keep);
    const slots = [null, null];
    try {
      texture = pipeline.draw(host.renderer, target, spec.fx, {
        targetOf: (slot) =>
          (slots[slot] ??= host.sceneTarget(`dsfx:${frame.order}:${slot}`, size.width, size.height, keep, 0, false).target),
        size,
        onMaterial: (created) => host.counters.add(created ? 'fx_program_created' : 'fx_program_reused'),
      });
    } catch (error) {
      // A pass that failed leaves nothing half-built behind: the next frame makes a new pipeline.
      if (host.fx === pipeline) {
        host.counters.add('fx_pipeline_discarded');
        host.dropFxPipeline();
      }
      throw error;
    }
  }
  if (spec.fx.length > 0) frame.sources.directed_scene_fx = (frame.sources.directed_scene_fx ?? 0) + spec.fx.length;
  // F062: how many of them were the passes of a shot (drawn first); absent without any, so the
  // telemetry of a scene with output effects only is what it was.
  const deckPasses = spec.fx.filter((pass) => pass.stage === 'deck').length;
  if (deckPasses > 0) frame.sources.directed_scene_fx_shot = (frame.sources.directed_scene_fx_shot ?? 0) + deckPasses;
  return { texture, width: size.width, height: size.height, fit: false, flipV: true };
}

/** Same retention rule as `layerMaterial`: one material per layer slot keeps its shader program linked. */
export function sceneMaterial(host, order, alpha, index, texture, keep) {
  if (!host.retain) return keep(createSceneMaterial(alpha, texture));
  const key = `ds:${order}:${alpha}:${index}`;
  let material = host.materials.get(key);
  if (material) {
    host.counters.add('material_reused');
  } else {
    material = createSceneMaterial(alpha, texture);
    host.materials.set(key, material);
    host.counters.add('material_created');
  }
  material.map = texture;
  return material;
}
