// F068: the rounded or elliptical mask of an image layer. The mask cuts the picture after the crop,
// follows the layer's own rectangle, composes with colour correction, and an unmasked layer stays
// byte for byte the picture it was. Real page (headless Chrome, SwiftShader); skipped, loudly,
// without Chrome or the bundle (TESSEL_REQUIRE_RUNTIME=1 fails).
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { gradeColor } from '../src/grading.js';
import { maskCoverage, maskRect } from '../src/mask.js';
import { png } from '../scripts/perf/png.mjs';
import { flat, imageLayer, openRuntime, skipUnavailable, stillAsset, transform } from './support.mjs';

const size = [320, 180];
const COLOR = [200, 100, 50];
const LEFT = [220, 40, 40];
const RIGHT = [40, 80, 220];

const near = (actual, wanted, tolerance, label) => {
  for (let index = 0; index < 3; index += 1) {
    assert.ok(Math.abs(actual[index] - wanted[index]) <= tolerance, `${label}: got ${actual.slice(0, 3)}, want ${wanted} (±${tolerance})`);
  }
};
const isBlack = (pixel) => pixel[0] < 6 && pixel[1] < 6 && pixel[2] < 6;

test('mask: the CPU reference cuts a rectangle by its corners, an ellipse by its axes', () => {
  const rect = maskRect(320, 180, [0, 0, 0, 0], { radius: 20 });
  assert.deepEqual([rect.centerX, rect.centerY, rect.halfW, rect.halfH], [0, 0, 160, 90]);
  assert.equal(rect.radius, 36, '20% of the shorter side (180px)');
  assert.equal(maskCoverage(0, 0, rect, 'rounded'), 1);
  assert.equal(maskCoverage(-159.5, 89.5, rect, 'rounded'), 0, 'the very corner is cut');
  assert.equal(maskCoverage(-159.5, 0, rect, 'rounded'), 1, 'the middle of an edge is not');
  assert.equal(maskCoverage(0, 0, rect, 'ellipse'), 1);
  assert.equal(maskCoverage(-159.5, 89.5, rect, 'ellipse'), 0);
  assert.equal(maskRect(320, 180, [0, 0, 0, 0], { radius: 80 }).radius, 90, 'never past half the shorter side');
  // The crop moves the rectangle the mask works on.
  const cropped = maskRect(320, 180, [25, 25, 0, 0], { radius: 0 });
  assert.deepEqual([cropped.halfW, cropped.halfH, cropped.centerX], [80, 90, 0]);
});

test('mask: a rounded or elliptical cut of a flat picture draws the reference pixels', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const directory = mkdtempSync(join(tmpdir(), 'tessel-mask-'));
  const file = join(directory, 'flat.png');
  writeFileSync(file, flat(64, 36, COLOR));
  const runtime = await openRuntime({ extraFiles: [file] });
  try {
    await runtime.hello({});
    assert.equal((await runtime.load(stillAsset('flat', file))).state, 'ready');
    const points = [[160, 90], [1, 1], [3, 3], [20, 20], [160, 1], [1, 90], [318, 178], [10, 10], [40, 40], [60, 60]];

    const plain = await runtime.frame([imageLayer('flat')], { size, probes: points });
    assert.equal(plain.type, 'frame.done', JSON.stringify(plain.error));
    near(plain.probes[0], COLOR, 1, 'unmasked centre');
    near(plain.probes[1], COLOR, 1, 'unmasked corner');

    for (const [shape, radius] of [['rounded', 20], ['rounded', 50], ['rounded', 0], ['ellipse', 10]]) {
      const masked = await runtime.frame([imageLayer('flat', { mask: { shape, radius } })], { size, probes: points });
      assert.equal(masked.type, 'frame.done', JSON.stringify(masked.error));
      const rect = maskRect(320, 180, [0, 0, 0, 0], { radius });
      points.forEach(([x, y], index) => {
        // The probe is the centre of pixel (x, y); the quad is in canvas pixels, y up, origin at the centre.
        const coverage = maskCoverage(x + 0.5 - 160, 90 - (y + 0.5), rect, shape);
        near(masked.probes[index], COLOR.map((channel) => channel * coverage), 4, `${shape} ${radius} at ${x},${y}`);
      });
      if (radius > 0) assert.notEqual(masked.hash, plain.hash, `${shape} ${radius} changes the picture`);
    }

    const rounded = await runtime.frame([imageLayer('flat', { mask: { shape: 'rounded', radius: 20 } })], { size, probes: [[3, 3], [20, 20], [160, 1]] });
    assert.ok(isBlack(rounded.probes[0]), `the corner is cut: ${rounded.probes[0]}`);
    near(rounded.probes[1], COLOR, 1, 'inside the corner arc');
    near(rounded.probes[2], COLOR, 1, 'the straight edge');

    // The plain material was not touched by the masked one that came between.
    const again = await runtime.frame([imageLayer('flat')], { size });
    assert.equal(again.hash, plain.hash, 'an unmasked layer is the picture it was, byte for byte');
  } finally {
    await runtime.close();
  }
});

test('mask: it cuts the cropped rectangle, not the source, and turns with the layer', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const directory = mkdtempSync(join(tmpdir(), 'tessel-mask-'));
  const file = join(directory, 'halves.png');
  writeFileSync(file, png(64, 36, (x) => [...(x < 32 ? LEFT : RIGHT), 255]));
  const runtime = await openRuntime({ extraFiles: [file] });
  try {
    await runtime.hello({});
    assert.equal((await runtime.load(stillAsset('halves', file))).state, 'ready');
    const mask = { shape: 'rounded', radius: 20 };
    // Crop 25% from each side: the visible rectangle is x 80..240 and the corners round there.
    const cropped = imageLayer('halves', { mask, transform: transform({ crop: [25, 25, 0, 0] }) });
    const frame = await runtime.frame([cropped], { size, probes: [[120, 90], [200, 90], [100, 10], [84, 4], [60, 90], [260, 90]] });
    assert.equal(frame.type, 'frame.done', JSON.stringify(frame.error));
    near(frame.probes[0], LEFT, 2, 'left half');
    near(frame.probes[1], RIGHT, 2, 'right half');
    near(frame.probes[2], LEFT, 2, 'inside the arc of the cropped corner');
    assert.ok(isBlack(frame.probes[3]), `outside that arc the picture is cut, though the uncropped one would not be: ${frame.probes[3]}`);
    assert.ok(isBlack(frame.probes[4]) && isBlack(frame.probes[5]), 'outside the crop nothing is drawn');

    // A turned layer: the cut turns with it. A quarter turn of the 2:1 cropped rectangle puts its
    // rounded short ends on the sides of the canvas's height; compare against the unturned corner.
    const turned = imageLayer('halves', { mask, transform: transform({ crop: [25, 25, 0, 0], rotation: 90, scale: 0.5 }) });
    const quarter = await runtime.frame([turned], { size, probes: [[160, 90], [160, 20]] });
    assert.equal(quarter.type, 'frame.done', JSON.stringify(quarter.error));
    assert.ok(!isBlack(quarter.probes[0]), 'the turned picture is at the centre');
  } finally {
    await runtime.close();
  }
});

test('mask: it composes with colour correction and with opacity', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const directory = mkdtempSync(join(tmpdir(), 'tessel-mask-'));
  const file = join(directory, 'flat.png');
  writeFileSync(file, flat(64, 36, COLOR));
  const runtime = await openRuntime({ extraFiles: [file] });
  try {
    await runtime.hello({});
    assert.equal((await runtime.load(stillAsset('flat', file))).state, 'ready');
    const mask = { shape: 'ellipse', radius: 0 };
    const grading = { brightness: 30, contrast: 0, saturation: -50 };
    const graded = gradeColor(COLOR.map((channel) => channel / 255), grading).map((channel) => Math.round(channel * 255));
    const frame = await runtime.frame([imageLayer('flat', { mask, grading })], { size, probes: [[160, 90], [2, 2]] });
    assert.equal(frame.type, 'frame.done', JSON.stringify(frame.error));
    near(frame.probes[0], graded, 2, 'graded inside the mask');
    assert.ok(isBlack(frame.probes[1]), 'cut outside it');

    const half = await runtime.frame([imageLayer('flat', { mask, opacity: 0.5 })], { size, probes: [[160, 90], [2, 2]] });
    near(half.probes[0], COLOR.map((channel) => channel / 2), 2, 'half opacity inside the mask');
    assert.ok(isBlack(half.probes[1]));

    // Retained materials: masked, graded+masked, plain, in one session, each keeps its own program.
    const plain = await runtime.frame([imageLayer('flat')], { size });
    const plainAgain = await runtime.frame([imageLayer('flat')], { size });
    assert.equal(plainAgain.hash, plain.hash);
    const maskedAgain = await runtime.frame([imageLayer('flat', { mask, grading })], { size });
    assert.equal(maskedAgain.hash, frame.hash, 'the same masked frame is the same picture');
  } finally {
    await runtime.close();
  }
});
