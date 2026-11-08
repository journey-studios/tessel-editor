import assert from 'node:assert/strict';
import { test } from 'node:test';

import { forEachLitCell, measureText } from '../src/font.js';
import { fnv1a64, meanColor } from '../src/hash.js';
import { expandPattern } from '../src/protocol.js';
import { layerScene, splatLayerScene } from '../src/scene.js';
import { SPLAT_FORMATS, splatFileName } from '../src/splat-format.js';

const bytes = (text) => new TextEncoder().encode(text);

test('fnv1a64 matches the published FNV-1a 64-bit vectors', () => {
  assert.equal(fnv1a64(new Uint8Array()), 'cbf29ce484222325');
  assert.equal(fnv1a64(bytes('a')), 'af63dc4c8601ec8c');
  assert.equal(fnv1a64(bytes('foobar')), '85944171f73967e8');
});

test('mean color averages each channel over the pixels', () => {
  assert.deepEqual(meanColor(new Uint8Array([0, 100, 200, 255, 100, 100, 0, 255])), [50, 100, 100, 255]);
});

test('frame patterns expand like the Rust side', () => {
  assert.equal(expandPattern('/a/f%05d.png', 7), '/a/f00007.png');
  assert.equal(expandPattern('f%03d', 1234), 'f1234');
  assert.equal(expandPattern('f%d.png', 12), 'f12.png');
  assert.equal(expandPattern('f.png', 1), null);
});

test('the font draws every printable ASCII character and flags unknown ones as a box', () => {
  for (let code = 32; code < 127; code += 1) {
    const character = String.fromCharCode(code);
    let lit = 0;
    forEachLitCell(character, () => {
      lit += 1;
    });
    assert.ok(character === ' ' ? lit === 0 : lit > 0, `glyph for ${JSON.stringify(character)}`);
  }
  let box = 0;
  forEachLitCell('é', () => {
    box += 1;
  });
  assert.equal(box, 20);
  assert.deepEqual(measureText('AB'), { columns: 11, rows: 7 });
  let lowerLit = 0;
  let upperLit = 0;
  forEachLitCell('t', () => {
    lowerLit += 1;
  });
  forEachLitCell('T', () => {
    upperLit += 1;
  });
  assert.equal(lowerLit, upperLit, 'lowercase is drawn as capitals');
});

test('splat file names follow the content format the core identified, then the extension', () => {
  assert.deepEqual(SPLAT_FORMATS, ['splat', 'ply', 'spz']);
  assert.equal(splatFileName('ply', '/p/scan.splat'), 'asset.ply');
  assert.equal(splatFileName('spz', '/p/x.bin'), 'asset.spz');
  assert.equal(splatFileName(undefined, '/p/Scan.PLY'), 'asset.ply');
  assert.equal(splatFileName('mystery', '/p/noext'), 'asset.splat');
});

test('a bare splat layer is a one-splat scene turned by the plan orientation (BL143)', () => {
  const layer = { kind: 'scene', asset: 'shot', orientation: [180, 0, 0] };
  assert.deepEqual(splatLayerScene(layer).objects, [{ type: 'splat', asset: 'shot', rotation: [180, 0, 0] }]);
  assert.deepEqual(layerScene({ asset: 'shot' }, { kind: 'splat' }).objects[0].rotation, [0, 0, 0]);
  const file = { objects: [] };
  assert.equal(layerScene(layer, { kind: 'scene', scene: file }), file);
  assert.equal(layerScene(layer, undefined), undefined);
});
