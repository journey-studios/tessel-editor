// Colour correction (brightness, contrast, saturation) changes the pixels of the graded layer as
// grading.js says, leaves an ungraded layer byte-for-byte as it was, and never touches what is under
// it. Real page (headless Chrome, SwiftShader); skipped, loudly, without Chrome or the bundle
// (TESSEL_REQUIRE_RUNTIME=1 fails).
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { gradeColor, isIdentityGrading } from '../src/grading.js';
import { flat, imageLayer, openRuntime, skipUnavailable, stillAsset, textLayer } from './support.mjs';

const COLOR = [200, 100, 50];
const CENTER = [[80, 45]];

/** The expected 8-bit pixel of `COLOR` graded by `grading`, from the CPU reference. */
const expected = (grading) => gradeColor(COLOR.map((channel) => channel / 255), grading).map((channel) => Math.round(channel * 255));

const near = (actual, wanted, label) => {
  for (let index = 0; index < 3; index += 1) {
    assert.ok(Math.abs(actual[index] - wanted[index]) <= 1, `${label}: got ${actual.slice(0, 3)}, want ${wanted} (±1)`);
  }
};

test('grading: the CPU reference is the documented formula', () => {
  assert.ok(isIdentityGrading(undefined) && isIdentityGrading({ brightness: 0, contrast: 0, saturation: 0 }));
  assert.ok(!isIdentityGrading({ brightness: 0, contrast: 0, saturation: 1 }));
  assert.deepEqual(gradeColor([0.2, 0.4, 0.6], {}), [0.2, 0.4, 0.6]);
  assert.deepEqual(gradeColor([0.2, 0.4, 0.6], { brightness: 100 }).map((v) => +v.toFixed(6)), [0.7, 0.9, 1]);
  assert.deepEqual(gradeColor([0.2, 0.9, 0.5], { contrast: -100 }), [0.5, 0.5, 0.5]);
  const grey = gradeColor([0.8, 0.2, 0.1], { saturation: -100 });
  assert.ok(grey.every((channel) => Math.abs(channel - grey[0]) < 1e-12), `${grey}`);
  assert.ok(Math.abs(grey[0] - (0.8 * 0.2126 + 0.2 * 0.7152 + 0.1 * 0.0722)) < 1e-12);
});

test('grading: a graded layer draws the reference pixels, an ungraded one is unchanged', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const directory = mkdtempSync(join(tmpdir(), 'tessel-grading-'));
  const file = join(directory, 'flat.png');
  writeFileSync(file, flat(32, 18, COLOR));
  const under = join(directory, 'under.png');
  writeFileSync(under, flat(32, 18, [0, 0, 255]));
  const runtime = await openRuntime({ extraFiles: [file, under] });
  try {
    await runtime.hello({});
    assert.equal((await runtime.load(stillAsset('flat', file))).state, 'ready');
    assert.equal((await runtime.load(stillAsset('under', under))).state, 'ready');

    const plain = await runtime.frame([imageLayer('flat')], { probes: CENTER });
    assert.equal(plain.type, 'frame.done', JSON.stringify(plain.error));
    near(plain.probes[0], COLOR, 'ungraded');
    // All-zero grading is no grading: same material, same bytes.
    const zero = await runtime.frame([imageLayer('flat', { grading: { brightness: 0, contrast: 0, saturation: 0 } })], { probes: CENTER });
    assert.equal(zero.hash, plain.hash, 'an all-zero grading draws the ungraded picture byte for byte');

    for (const grading of [
      { brightness: 40, contrast: 0, saturation: 0 },
      { brightness: -60, contrast: 0, saturation: 0 },
      { brightness: 0, contrast: 50, saturation: 0 },
      { brightness: 0, contrast: -100, saturation: 0 },
      { brightness: 0, contrast: 0, saturation: -100 },
      { brightness: 0, contrast: 0, saturation: 80 },
      { brightness: 20, contrast: 30, saturation: -40 },
    ]) {
      const graded = await runtime.frame([imageLayer('flat', { grading })], { probes: CENTER });
      assert.equal(graded.type, 'frame.done', JSON.stringify(graded.error));
      near(graded.probes[0], expected(grading), JSON.stringify(grading));
      assert.notEqual(graded.hash, plain.hash, `${JSON.stringify(grading)} changes the picture`);
    }

    // Back to ungraded after a graded frame (retained materials): exactly the first picture.
    const again = await runtime.frame([imageLayer('flat')], { probes: CENTER });
    assert.equal(again.hash, plain.hash, 'the plain material was not touched by the graded one');

    // Grading is the layer's own: the layer below keeps its pixels where the graded one is
    // transparent, and alpha is kept (half opacity still mixes with what is below).
    const small = { transform: { x: 0, y: 0, scale: 0.5, rotation: 0, crop: [0, 0, 0, 0] } };
    const stack = await runtime.frame(
      [imageLayer('under'), imageLayer('flat', { ...small, opacity: 0.5, grading: { brightness: 0, contrast: 0, saturation: -100 } })],
      { probes: [[80, 45], [4, 4]] },
    );
    assert.equal(stack.type, 'frame.done', JSON.stringify(stack.error));
    near(stack.probes[1], [0, 0, 255], 'the layer below, outside the graded one, is untouched');
    const grey = expected({ saturation: -100 });
    near(stack.probes[0], grey.map((channel, index) => Math.round(channel * 0.5 + [0, 0, 255][index] * 0.5)), 'half-opaque graded layer over blue');

    // A title is a layer too: its colour is graded like any texel.
    const title = await runtime.frame([textLayer('A', { transform: { x: 0, y: 0, scale: 1, rotation: 0, crop: [0, 0, 0, 0] } })]);
    const darkTitle = await runtime.frame([
      textLayer('A', { transform: { x: 0, y: 0, scale: 1, rotation: 0, crop: [0, 0, 0, 0] }, grading: { brightness: -100, contrast: 0, saturation: 0 } }),
    ]);
    assert.notEqual(darkTitle.hash, title.hash, 'a graded title changes');
    assert.ok(darkTitle.mean[0] < title.mean[0], `the darkened title is darker: ${darkTitle.mean} vs ${title.mean}`);
  } finally {
    await runtime.close();
  }
});
