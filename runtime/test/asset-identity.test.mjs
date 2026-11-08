import assert from 'node:assert/strict';
import { test } from 'node:test';

import { ASSET_IDENTITY_CAPABILITIES, assetEcho } from '../src/asset-identity.js';

const scene = { asset_id: 'a', revision_id: 'a-r1', content_hash: 'h', generation: 3 };

test('a descriptor without identity echoes nothing', () => {
  assert.deepEqual(assetEcho({ id: 'x', kind: 'still' }), {});
});

test('request and scene identity are repeated as they were sent', () => {
  assert.deepEqual(assetEcho({ id: 'x', request: 0, scene }), { request: 0, scene });
  assert.deepEqual(assetEcho({ id: 'x', request: 7 }), { request: 7 });
  assert.deepEqual(assetEcho({ id: 'x', scene }), { scene });
});

test('only the identity fields are echoed, never what else the descriptor carries', () => {
  assert.deepEqual(assetEcho({ id: 'x', scene: { ...scene, path: '/private/file.png' } }).scene, scene);
});

test('an incomplete scene identity is not echoed (the load itself refuses it)', () => {
  assert.deepEqual(assetEcho({ id: 'x', scene: { ...scene, generation: -1 } }), {});
  assert.deepEqual(assetEcho({ id: 'x', scene: { ...scene, content_hash: '' } }), {});
  assert.deepEqual(assetEcho({ id: 'x', scene: 'nope' }), {});
});

test('a request that is not a non-negative safe integer is refused with a typed error', () => {
  for (const request of [-1, 1.5, '3', null, Number.MAX_SAFE_INTEGER + 1, Number.NaN]) {
    assert.throws(
      () => assetEcho({ id: 'x', request }),
      (error) => error.code === 'invalid_request',
      `request ${String(request)}`,
    );
  }
});

test('the advertised capability lists exactly what is echoed', () => {
  assert.deepEqual(ASSET_IDENTITY_CAPABILITIES.echo, ['request', 'scene']);
  assert.deepEqual(Object.keys(assetEcho({ id: 'x', request: 1, scene })), ASSET_IDENTITY_CAPABILITIES.echo);
});
