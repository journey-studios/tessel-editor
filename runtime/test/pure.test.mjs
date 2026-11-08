import assert from 'node:assert/strict';
import { test } from 'node:test';

import { readFileSync } from 'node:fs';

import { forEachLitCell, hasGlyph, measureText, normalizeText } from '../src/font.js';
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
  forEachLitCell('€', () => {
    box += 1;
  });
  assert.equal(box, 20, 'a character outside the coverage is still the outlined box');
  assert.deepEqual(measureText('AB'), { columns: 11, rows: 7, pad: 0 });
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

const cells = (text) => {
  const lit = [];
  forEachLitCell(text, (column, row) => lit.push(`${column},${row}`));
  return lit.sort().join(' ');
};

// The list the core accepts (`crates/core/src/text_coverage.txt`): one source for both sides.
const COVERAGE = [...readFileSync(new URL('../../crates/core/src/text_coverage.txt', import.meta.url), 'utf8').trim()];

test('every character the core accepts has a glyph that is not the unknown box', () => {
  const box = cells('€');
  for (const character of COVERAGE) {
    assert.ok(hasGlyph(character), `glyph for ${character}`);
    assert.notEqual(cells(character), box, `${character} is not the box`);
  }
  for (const outside of ['€', 'ß', '日', 'a\u0307']) {
    assert.equal(hasGlyph(outside), false, `${JSON.stringify(outside)} is outside the coverage`);
  }
});

test('an accent is a mark composed on the capital: the body keeps its place and the box grows evenly', () => {
  assert.deepEqual(measureText('AÇÃO'), { columns: 23, rows: 11, pad: 2 });
  assert.deepEqual(measureText('ACAO'), { columns: 23, rows: 7, pad: 0 });
  const row = (cell) => Number(cell.split(',')[1]);
  const body = cells('A').split(' ').map((cell) => `${cell.split(',')[0]},${row(cell) + 2}`);
  const tilde = cells('Ã').split(' ');
  assert.deepEqual(tilde.filter((cell) => row(cell) >= 2), body, 'the body of Ã is A, two rows lower');
  assert.equal(tilde.filter((cell) => row(cell) < 2).length, 4, 'the tilde adds its own cells above');
  const cedilla = cells('Ç').split(' ');
  assert.equal(cedilla.filter((cell) => row(cell) >= 9).length, 3, 'the cedilla hangs below the body');
});

test('composed and decomposed spellings draw the same picture', () => {
  const pairs = [['ã', 'a\u0303'], ['ç', 'c\u0327'], ['É', 'E\u0301'], ['Õ', 'O\u0303'], ['ü', 'u\u0308']];
  for (const [composed, decomposed] of pairs) {
    assert.equal(normalizeText(decomposed), composed);
    assert.equal(cells(decomposed), cells(composed), `${composed}`);
    assert.deepEqual(measureText(decomposed), measureText(composed));
  }
  const phrase = 'Coração e emoção, não é só açaí';
  assert.equal(cells(phrase.normalize('NFD')), cells(phrase.normalize('NFC')));
  assert.equal(cells('ação'), cells('AÇÃO'), 'lowercase is still drawn as capitals');
});

test('splat file names follow the content format the core identified, then the extension', () => {
  assert.deepEqual(SPLAT_FORMATS, ['splat', 'ply', 'spz']);
  assert.equal(splatFileName('ply', '/p/scan.splat'), 'asset.ply');
  assert.equal(splatFileName('spz', '/p/x.bin'), 'asset.spz');
  assert.equal(splatFileName(undefined, '/p/Scan.PLY'), 'asset.ply');
  assert.equal(splatFileName('mystery', '/p/noext'), 'asset.splat');
});

test('runtime glyph coverage rejects non-core letters even when Unicode folds them to ASCII', () => {
  const coverage = readFileSync(new URL('../../crates/core/src/text_coverage.txt', import.meta.url), 'utf8').trim();
  for (let code = 0; code < 0x250; code += 1) {
    const ch = String.fromCodePoint(code);
    assert.equal(hasGlyph(ch), (code >= 32 && code <= 126) || coverage.includes(ch), `U+${code.toString(16)}`);
  }
  for (const ch of ['K', 'ſ', 'ı', 'ÿ']) assert.equal(hasGlyph(ch), false, ch);
});

test('a bare splat layer is a one-splat scene turned by the plan orientation (BL143)', () => {
  const layer = { kind: 'scene', asset: 'shot', orientation: [180, 0, 0] };
  assert.deepEqual(splatLayerScene(layer).objects, [{ type: 'splat', asset: 'shot', rotation: [180, 0, 0] }]);
  assert.deepEqual(layerScene({ asset: 'shot' }, { kind: 'splat' }).objects[0].rotation, [0, 0, 0]);
  const file = { objects: [] };
  assert.equal(layerScene(layer, { kind: 'scene', scene: file }), file);
  assert.equal(layerScene(layer, undefined), undefined);
});
