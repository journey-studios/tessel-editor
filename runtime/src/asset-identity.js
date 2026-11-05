// Identity of an `asset.load` request, echoed in every `asset.state` it causes (bridge v1, additive;
// advertised as the `asset.identity` feature and `capabilities.asset_identity` of `ready`).
//
// WHY: a host names pictures by slot (`asset.id`), and a slot is reused: it is released, loaded
// with another picture, or loaded again with the same one. The page answers loads in its own order,
// so an old `ready` can reach the host after the slot was given to a new load. The id alone cannot
// tell the two apart; the identity of the request can. The runtime only repeats what the host sent
// (it decides nothing); the host compares.
//
//   request  an opaque non-negative integer the host picks per `asset.load` (one per message sent,
//            never reused by that host), so even the same picture loaded twice in a slot differs
//   scene    the resource identity of a directed-scene load (`descriptor.scene`)
//
// A host that sends neither gets the answers it always got.

import { RuntimeError } from './protocol.js';

/** Advertised in `ready.capabilities.asset_identity`. */
export const ASSET_IDENTITY_CAPABILITIES = {
  echo: ['request', 'scene'],
  scene_fields: ['asset_id', 'revision_id', 'content_hash', 'generation'],
};

const isObject = (value) => typeof value === 'object' && value !== null && !Array.isArray(value);

const sceneEcho = (scene) => {
  if (!isObject(scene)) return undefined;
  const { asset_id: assetId, revision_id: revisionId, content_hash: contentHash, generation } = scene;
  const text = (value) => typeof value === 'string' && value !== '';
  // Only a complete identity is echoed; an incomplete one is refused by the load itself
  // (`invalid_plan`), and echoing half of it would let a host believe it was understood.
  if (!text(assetId) || !text(revisionId) || !text(contentHash) || !Number.isSafeInteger(generation) || generation < 0) {
    return undefined;
  }
  return { asset_id: assetId, revision_id: revisionId, content_hash: contentHash, generation };
};

/**
 * What `descriptor` asks to be echoed, as fields to spread into `asset.state` (empty for a host
 * that predates the identity). A `request` that is not a non-negative safe integer is a refusal.
 */
export const assetEcho = (descriptor) => {
  const echo = {};
  if (descriptor.request !== undefined) {
    if (!Number.isSafeInteger(descriptor.request) || descriptor.request < 0) {
      throw new RuntimeError('invalid_request', '`request` of an asset load must be a non-negative integer', {
        asset: descriptor.id ?? null,
      });
    }
    echo.request = descriptor.request;
  }
  const scene = sceneEcho(descriptor.scene);
  if (scene) echo.scene = scene;
  return echo;
};
