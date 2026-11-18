// F072: the sweeping edge that uncovers a clip's layers. The edge is cut in screen space, so it
// does not turn with the layer, every layer of one clip shares it, it composes with the mask and
// with colour correction, and a layer without a reveal stays byte for byte the picture it was.
// Real page (headless Chrome, SwiftShader); skipped, loudly, without Chrome or the bundle
// (TESSEL_REQUIRE_RUNTIME=1 fails).
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { gradeColor } from '../src/grading.js';
import { revealCoverage, revealValues } from '../src/reveal.js';
import { flat, imageLayer, openRuntime, skipUnavailable, stillAsset, transform } from './support.mjs';

const size = [320, 180];
const [W, H] = size;
const COLOR = [200, 100, 50];

const near = (actual, wanted, tolerance, label) => {
  for (let index = 0; index < 3; index += 1) {
    assert.ok(Math.abs(actual[index] - wanted[index]) <= tolerance, `${label}: got ${actual.slice(0, 3)}, want ${wanted} (±${tolerance})`);
  }
};
const isBlack = (pixel) => pixel[0] < 6 && pixel[1] < 6 && pixel[2] < 6;

const shapeLayer = (shape, extra = {}) => ({
  id: 'layer:shape',
  kind: 'shape',
  opacity: 1,
  blend: 'normal',
  transform: transform(),
  shape: { kind: 'rect', width: 100, height: 100, fill: '#ff0000', radius: 0, ...shape },
  ...extra,
});

test('reveal: the CPU reference is one on the near side of the edge, zero past it, a ramp across it', () => {
  const straight = { edge: 50, angle: 0, direction: 1 };
  assert.equal(revealCoverage(158, 10, W, H, straight), 1);
  assert.equal(revealCoverage(162, 10, W, H, straight), 0);
  assert.equal(revealCoverage(160, 10, W, H, straight), 0.5);
  assert.equal(revealCoverage(162, 10, W, H, { ...straight, direction: -1 }), 1);
  // A positive angle leads with the top: the same row sits further along the travel at the top.
  const leaning = { edge: 50, angle: 20, direction: 1 };
  assert.ok(revealCoverage(190, 0, W, H, leaning) > revealCoverage(190, H, W, H, leaning));
  // The uniform of the shader: world x of the edge, signed lean, direction and cosine.
  const [edgeX, lean, direction, cosine] = revealValues(leaning, W);
  assert.equal(edgeX, 0);
  assert.ok(Math.abs(lean - Math.tan((20 * Math.PI) / 180)) < 1e-12);
  assert.equal(direction, 1);
  assert.ok(Math.abs(cosine - Math.cos((20 * Math.PI) / 180)) < 1e-12);
});

test('reveal: a flat picture is uncovered behind the edge exactly as the reference says', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const directory = mkdtempSync(join(tmpdir(), 'tessel-reveal-'));
  const file = join(directory, 'flat.png');
  writeFileSync(file, flat(64, 36, COLOR));
  const runtime = await openRuntime({ extraFiles: [file] });
  try {
    await runtime.hello({});
    assert.equal((await runtime.load(stillAsset('flat', file))).state, 'ready');
    const points = [[2, 2], [2, 90], [2, 177], [100, 2], [100, 177], [158, 90], [162, 90], [250, 90], [317, 2], [317, 177]];

    const plain = await runtime.frame([imageLayer('flat')], { size, probes: points });
    assert.equal(plain.type, 'frame.done', JSON.stringify(plain.error));

    for (const reveal of [
      { edge: 50, angle: 0, direction: 1 },
      { edge: 50, angle: 12.33, direction: 1 },
      { edge: 40, angle: -25, direction: 1 },
      { edge: 60, angle: 12.33, direction: -1 },
      { edge: 30, angle: 45, direction: -1 },
    ]) {
      const frame = await runtime.frame([imageLayer('flat', { reveal })], { size, probes: points });
      assert.equal(frame.type, 'frame.done', JSON.stringify(frame.error));
      points.forEach(([x, y], index) => {
        // The probe is the centre of pixel (x, y).
        const coverage = revealCoverage(x + 0.5, y + 0.5, W, H, reveal);
        near(frame.probes[index], COLOR.map((channel) => channel * coverage), 4, `${JSON.stringify(reveal)} at ${x},${y}`);
      });
      assert.notEqual(frame.hash, plain.hash, 'a partial reveal changes the picture');
    }

    // Before the sweep nothing shows; once past the far side everything does.
    const hidden = await runtime.frame([imageLayer('flat', { reveal: { edge: -30, angle: 12.33, direction: 1 } })], { size, probes: points });
    assert.ok(hidden.probes.every(isBlack), 'an edge off the left uncovers nothing');
    const open = await runtime.frame([imageLayer('flat', { reveal: { edge: 140, angle: 12.33, direction: 1 } })], { size, probes: points });
    open.probes.forEach((pixel, index) => near(pixel, COLOR, 1, `an edge off the right uncovers all: ${points[index]}`));
    assert.equal(open.hash, plain.hash, 'a fully uncovered layer is the plain picture');

    // The plain material was not touched by the revealing one that came between.
    const again = await runtime.frame([imageLayer('flat')], { size });
    assert.equal(again.hash, plain.hash, 'a layer without a reveal is the picture it was, byte for byte');
  } finally {
    await runtime.close();
  }
});

test('reveal: the edge is fixed on the screen, not on the layer, and every layer of a clip shares it', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    await runtime.hello({ retain: true });
    const reveal = { edge: 50, angle: 0, direction: 1 };
    // A red box turned a quarter and scaled down: it spans x 83..173, y 10..170 around x = 128.
    // The edge is the screen's vertical at x = 160, so it cuts the box there; if it were fixed on
    // the layer it would be the horizontal of the turned box and the probes would not agree.
    const turned = shapeLayer({ width: 100, height: 100 }, { transform: transform({ rotation: 90, scale: 0.5, x: -10 }), reveal });
    const frame = await runtime.frame([turned], { size, probes: [[100, 90], [150, 90], [166, 90], [100, 20], [100, 160], [200, 90]] });
    assert.equal(frame.type, 'frame.done', JSON.stringify(frame.error));
    near(frame.probes[0], [255, 0, 0], 3, 'inside the box, behind the edge');
    near(frame.probes[1], [255, 0, 0], 3, 'right up to the edge');
    assert.ok(isBlack(frame.probes[2]), 'the box goes on past the edge but is cut there: ' + frame.probes[2]);
    near(frame.probes[3], [255, 0, 0], 3, 'top of the box, behind the edge');
    near(frame.probes[4], [255, 0, 0], 3, 'bottom of the box, behind the edge');
    assert.ok(isBlack(frame.probes[5]), 'outside the box');
    // Two layers (a group's children) with the same edge are cut by the same line.
    const left = shapeLayer({ width: 50, height: 100, fill: '#ff0000' }, { transform: transform({ x: -25 }), reveal });
    const right = shapeLayer({ width: 50, height: 100, fill: '#00ff00' }, { id: 'layer:right', transform: transform({ x: 25 }), reveal });
    const both = await runtime.frame([left, right], { size, probes: [[100, 90], [158, 90], [162, 90], [250, 90]] });
    assert.equal(both.type, 'frame.done', JSON.stringify(both.error));
    near(both.probes[0], [255, 0, 0], 3, 'the left child is behind the edge');
    near(both.probes[1], [255, 0, 0], 3, 'right up to the edge');
    assert.ok(isBlack(both.probes[2]), 'the right child starts past the edge: cut');
    assert.ok(isBlack(both.probes[3]), 'and stays cut');
  } finally {
    await runtime.close();
  }
});

test('reveal: it composes with the mask, with colour correction and with opacity', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const directory = mkdtempSync(join(tmpdir(), 'tessel-reveal-'));
  const file = join(directory, 'flat.png');
  writeFileSync(file, flat(64, 36, COLOR));
  const runtime = await openRuntime({ extraFiles: [file] });
  try {
    await runtime.hello({ retain: true });
    assert.equal((await runtime.load(stillAsset('flat', file))).state, 'ready');
    const reveal = { edge: 50, angle: 0, direction: 1 };
    const grading = { brightness: 30, contrast: 0, saturation: -50 };
    const graded = gradeColor(COLOR.map((channel) => channel / 255), grading).map((channel) => Math.round(channel * 255));
    const frame = await runtime.frame(
      [imageLayer('flat', { reveal, grading, mask: { shape: 'ellipse', radius: 0 } })],
      { size, probes: [[100, 90], [250, 90], [2, 2]] },
    );
    assert.equal(frame.type, 'frame.done', JSON.stringify(frame.error));
    near(frame.probes[0], graded, 2, 'graded, inside the mask and behind the edge');
    assert.ok(isBlack(frame.probes[1]), 'inside the mask but past the edge');
    assert.ok(isBlack(frame.probes[2]), 'behind the edge but outside the mask');

    const half = await runtime.frame([imageLayer('flat', { reveal, opacity: 0.5 })], { size, probes: [[100, 90], [250, 90]] });
    near(half.probes[0], COLOR.map((channel) => channel / 2), 2, 'half opacity behind the edge');
    assert.ok(isBlack(half.probes[1]));

    // The program of each combination is its own: the same frame twice is the same picture.
    const again = await runtime.frame([imageLayer('flat', { reveal, grading, mask: { shape: 'ellipse', radius: 0 } })], { size });
    assert.equal(again.hash, frame.hash);
  } finally {
    await runtime.close();
  }
});

test('reveal: a retained material takes the next frame\'s edge and keeps drawing the right one', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    await runtime.hello({ retain: true });
    const probes = [[40, 90], [160, 90], [280, 90]];
    const redAt = async (edge) => {
      const frame = await runtime.frame([shapeLayer({}, { reveal: { edge, angle: 0, direction: 1 } })], { size, probes });
      assert.equal(frame.type, 'frame.done', JSON.stringify(frame.error));
      return frame.probes.map((pixel) => pixel[0] > 200);
    };
    assert.deepEqual(await redAt(25), [true, false, false]);
    assert.deepEqual(await redAt(75), [true, true, false]);
    assert.deepEqual(await redAt(10), [false, false, false]);
    assert.deepEqual(await redAt(25), [true, false, false], 'going back is the same picture as the first time');
  } finally {
    await runtime.close();
  }
});

test('shape: intermediate stops put colours between the two ends, and no stops is the old gradient', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    await runtime.hello({ retain: true });
    const wide = { fill: '#000000', fill_to: '#000000', fill_mode: 'linear', gradient_angle: 0 };
    const probes = [[4, 90], [128, 90], [186, 90], [250, 90], [316, 90]];
    // Black, then opaque red at 0.6 (x ~ 192), then black again.
    const banded = await runtime.frame([shapeLayer({ ...wide, stops: '0.6 #ff0000' })], { size, probes });
    assert.equal(banded.type, 'frame.done', JSON.stringify(banded.error));
    assert.ok(banded.probes[0][0] < 12, `the start is the fill: ${banded.probes[0]}`);
    assert.ok(banded.probes[1][0] > 160 && banded.probes[1][0] < 180, `two thirds up the first ramp: ${banded.probes[1]}`);
    assert.ok(banded.probes[2][0] > 240, `at the stop the colour is the stop's: ${banded.probes[2]}`);
    assert.ok(banded.probes[3][0] > 100 && banded.probes[3][0] < 160, `falling back to fill_to: ${banded.probes[3]}`);
    assert.ok(banded.probes[4][0] < 12, `the end is fill_to: ${banded.probes[4]}`);

    const plain = await runtime.frame([shapeLayer({ ...wide, fill: '#ff0000', fill_to: '#0000ff' })], { size });
    const explicit = await runtime.frame([shapeLayer({ ...wide, fill: '#ff0000', fill_to: '#0000ff', stops: '' })], { size });
    assert.equal(explicit.hash, plain.hash, 'an empty stops text draws the two-colour gradient it always did');
  } finally {
    await runtime.close();
  }
});

// The presentation video's own transition (projects/tessel-apresentacao/tools/motion-revisao.js,
// `transition`), redrawn here with the canvas 2D calls it uses, is the oracle: the generic pieces
// (a reveal edge, a multi-stop rotated strip, a rotated line) must paint what it paints.
test('reveal: an edge plus a rotated multi-stop strip and line reproduce the reference transition', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    await runtime.hello({ retain: true });
    const full = [1920, 1080];
    const [CW, CH] = full;
    const LEAN = 118 / 540;
    const degrees = (Math.atan(LEAN) * 180) / Math.PI;
    const cosine = Math.cos(Math.atan(LEAN));
    const accent = '#c6a7ff';
    // The reference at one instant: previous orange, next green through the polygon, then the beam.
    const reference = (x, points) => runtime.page.evaluate(({ x, points, CW, CH, accent }) => {
      const canvas = document.createElement('canvas');
      canvas.width = CW;
      canvas.height = CH;
      const c = canvas.getContext('2d');
      c.fillStyle = '#c86432';
      c.fillRect(0, 0, CW, CH);
      c.save();
      c.beginPath();
      c.moveTo(0, 0); c.lineTo(x + 118, 0); c.lineTo(x - 118, CH); c.lineTo(0, CH);
      c.closePath();
      c.clip();
      c.fillStyle = '#32c864';
      c.fillRect(0, 0, CW, CH);
      c.restore();
      c.save();
      c.transform(1, 0, -236 / CH, 1, 0, 0);
      const gradient = c.createLinearGradient(x - 92, 0, x + 119, 0);
      [[0, accent + '00'], [0.58, accent + '60'], [0.92, accent + 'fa'], [1, accent + '00']]
        .forEach(([at, color]) => gradient.addColorStop(at, color));
      c.fillStyle = gradient;
      c.fillRect(x - 92, 0, 211, CH);
      c.fillStyle = '#ffffff';
      c.fillRect(x + 120, 0, 3, CH);
      c.restore();
      return points.map(([px, py]) => [...c.getImageData(px, py, 1, 1).data]);
    }, { x, points, CW, CH, accent });

    const layersAt = (x) => {
      const bar = (id, width, shape, centre) => ({
        id,
        kind: 'shape',
        opacity: 1,
        blend: 'normal',
        transform: transform({ x: ((centre - CW / 2) / CW) * 100, rotation: degrees }),
        shape: { kind: 'rect', width, height: 130, radius: 0, ...shape },
      });
      return [
        shapeLayer({ fill: '#c86432' }, { id: 'layer:previous' }),
        shapeLayer({ fill: '#32c864' }, {
          id: 'layer:next',
          reveal: { edge: (x / CW) * 100, angle: degrees, direction: 1 },
        }),
        bar('layer:glow', ((211 * cosine) / CW) * 100, {
          fill: `${accent}00`, fill_to: `${accent}00`, fill_mode: 'linear', gradient_angle: 0,
          stops: `0.58 ${accent}60 0.92 ${accent}fa`,
        }, x - 104.5),
        bar('layer:line', ((3 * cosine) / CW) * 100, { fill: '#ffffff' }, x + 3.5),
      ];
    };

    let worst = 0;
    let total = 0;
    let beyond = 0;
    for (const x of [400, 1010, 1700]) {
      const points = [];
      for (const y of [60, 300, 540, 780, 1020]) {
        const edge = x + LEAN * (CH / 2 - y);
        for (const offset of [-300, -230, -190, -150, -110, -70, -40, -15, -4, 3, 7, 12, 40, 120]) {
          const px = Math.round(edge + offset);
          if (px >= 0 && px < CW) points.push([px, y]);
        }
      }
      const frame = await runtime.frame(layersAt(x), { size: full, probes: points });
      assert.equal(frame.type, 'frame.done', JSON.stringify(frame.error));
      const wanted = await reference(x, points);
      points.forEach(([px, py], index) => {
        const diff = Math.max(...[0, 1, 2].map((channel) => Math.abs(frame.probes[index][channel] - wanted[index][channel])));
        worst = Math.max(worst, diff);
        total += 1;
        if (diff > 8) beyond += 1;
        // The dark/light structure must agree everywhere, whatever the antialiasing does on the edge.
        assert.ok(diff <= 48, `x=${x} at ${px},${py}: runtime ${frame.probes[index].slice(0, 3)} vs reference ${wanted[index].slice(0, 3)}`);
      });
    }
    t.diagnostic(`reference transition: ${total} points, worst channel difference ${worst}/255, ${beyond} above 8`);
    assert.ok(total > 100, `${total} points compared`);
    assert.ok(beyond <= total * 0.05, `${beyond} of ${total} points differ by more than 8/255 (worst ${worst})`);
  } finally {
    await runtime.close();
  }
});