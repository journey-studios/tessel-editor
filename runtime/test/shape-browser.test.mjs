import assert from 'node:assert/strict';
import { test } from 'node:test';

import { openRuntime, plan, skipUnavailable, transform } from './support.mjs';

// F067: the rectangle clip is drawn from its spec alone (no asset). Sizes are even on an even
// canvas so the edges land on whole pixels and the pixel checks below are exact.
const size = [320, 180];

const shapeLayer = (shape, extra = {}) => ({
  id: 'layer:shape',
  kind: 'shape',
  opacity: 1,
  blend: 'normal',
  transform: transform(),
  shape: { kind: 'rect', width: 50, height: 40, fill: '#ff0000', radius: 0, ...shape },
  ...extra,
});

const exportFrame = async (runtime, id, layers) => {
  await runtime.send({
    type: 'frame.render', id, revision: id,
    plan: plan(layers, size),
    output: { width: size[0], height: size[1], png: true },
  });
  const frame = await runtime.waitFor((message) => message.id === id && ['frame.done', 'frame.failed'].includes(message.type));
  assert.equal(frame.type, 'frame.done', JSON.stringify(frame.error));
  return frame;
};

// Reads RGBA values of the PNG the bridge returned (the picture the CLI exports).
const pixels = (runtime, frame, points) => runtime.page.evaluate(async ({ base64, points }) => {
  const image = new Image();
  image.src = `data:image/png;base64,${base64}`;
  await image.decode();
  const canvas = document.createElement('canvas');
  canvas.width = image.width;
  canvas.height = image.height;
  const context = canvas.getContext('2d');
  context.drawImage(image, 0, 0);
  return points.map(([x, y]) => [...context.getImageData(x, y, 1, 1).data]);
}, { base64: frame.png_data, points });

const isRed = ([r, g, b]) => r > 240 && g < 16 && b < 16;
const isBlack = ([r, g, b]) => r < 16 && g < 16 && b < 16;

// ---- F068 (step 1B): ellipse, path, contour, gradients and shadow ---------------------------
// Probes read the real framebuffer (top-left origin). The box of `shapeLayer` is 160x72 centred:
// x 80..240, y 54..126 on the 320x180 canvas.
const probe = async (runtime, layers, points) => {
  const frame = await runtime.frame(layers, { size, probes: points });
  assert.equal(frame.type, 'frame.done', JSON.stringify(frame.error));
  return { frame, probes: frame.probes };
};

const close = (actual, wanted, tolerance, label) => {
  actual.slice(0, 3).forEach((channel, index) => {
    assert.ok(Math.abs(channel - wanted[index]) <= tolerance, `${label}: got ${actual.slice(0, 3)}, want ${wanted} (±${tolerance})`);
  });
};

test('an ellipse fills the ellipse of its box and nothing of the corners', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    await runtime.hello({ retain: true });
    const { probes } = await probe(runtime, [shapeLayer({ kind: 'ellipse' })], [
      [160, 90], [85, 90], [235, 90], [160, 57], [160, 123], [84, 58], [236, 122], [76, 90], [160, 50],
    ]);
    const [centre, leftEdge, rightEdge, top, bottom, cornerA, cornerB, outsideLeft, outsideTop] = probes;
    for (const [label, pixel] of [['centre', centre], ['left', leftEdge], ['right', rightEdge], ['top', top], ['bottom', bottom]]) {
      assert.ok(isRed(pixel), `${label} is inside the ellipse: ${pixel}`);
    }
    assert.ok(isBlack(cornerA) && isBlack(cornerB), `the corners of the box are outside the ellipse: ${cornerA} ${cornerB}`);
    assert.ok(isBlack(outsideLeft) && isBlack(outsideTop), 'outside the box nothing is drawn');
  } finally {
    await runtime.close();
  }
});

test('a linear gradient runs from the fill to fill_to along its angle', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    await runtime.hello({ retain: true });
    // The whole canvas, red to blue: left to right (0 degrees) and top to bottom (90).
    const wide = { width: 100, height: 100, fill: '#ff0000', fill_to: '#0000ff', fill_mode: 'linear' };
    const horizontal = await probe(runtime, [shapeLayer({ ...wide, gradient_angle: 0 })], [[1, 90], [160, 90], [318, 90], [160, 2], [160, 177]]);
    close(horizontal.probes[0], [253, 0, 2], 4, 'left end is the fill');
    close(horizontal.probes[1], [127, 0, 128], 4, 'the middle is halfway');
    close(horizontal.probes[2], [3, 0, 252], 4, 'right end is fill_to');
    close(horizontal.probes[3], [127, 0, 128], 4, 'a horizontal gradient does not change with y (top)');
    close(horizontal.probes[4], [127, 0, 128], 4, 'a horizontal gradient does not change with y (bottom)');

    const vertical = await probe(runtime, [shapeLayer({ ...wide, gradient_angle: 90 })], [[160, 1], [160, 90], [160, 178], [2, 90], [317, 90]]);
    close(vertical.probes[0], [253, 0, 2], 5, 'top is the fill');
    close(vertical.probes[1], [127, 0, 128], 5, 'middle');
    close(vertical.probes[2], [3, 0, 252], 5, 'bottom is fill_to');
    close(vertical.probes[3], [127, 0, 128], 5, 'a vertical gradient does not change with x (left)');
    assert.notEqual(horizontal.frame.hash, vertical.frame.hash);

    const reversed = await probe(runtime, [shapeLayer({ ...wide, gradient_angle: 180 })], [[1, 90], [318, 90]]);
    close(reversed.probes[0], [3, 0, 252], 4, '180 degrees starts on the right');
    close(reversed.probes[1], [253, 0, 2], 4, '180 degrees ends on the left');
  } finally {
    await runtime.close();
  }
});

test('a radial gradient is an ellipse of the box: opaque in the middle, gone at the edge', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    await runtime.hello({ retain: true });
    const halo = shapeLayer({ kind: 'ellipse', fill: '#ffffff', fill_to: '#ffffff00', fill_mode: 'radial' });
    const { probes } = await probe(runtime, [halo], [[160, 90], [120, 90], [82, 90], [160, 56], [160, 72], [30, 90]]);
    // The probe is half a pixel off the exact centre, which is already 1.5% of the way to the edge.
    close(probes[0], [255, 255, 255], 6, 'the centre keeps the start colour');
    // Half way to the edge on the long axis (x 120) and on the short one (y 72): about half alpha over black.
    close(probes[1], [128, 128, 128], 8, 'half way along the width');
    close(probes[4], [128, 128, 128], 8, 'half way along the height: the ellipse reaches the edge on both axes');
    assert.ok(probes[2][0] < 24, `the edge is almost transparent: ${probes[2]}`);
    assert.ok(probes[3][0] < 24, `the top edge is almost transparent: ${probes[3]}`);
    close(probes[5], [0, 0, 0], 1, 'outside is untouched');

    // The same gradient on a rectangle leaves the corners at the end colour, fully transparent.
    const card = shapeLayer({ kind: 'rect', fill: '#ffffff', fill_to: '#ffffff00', fill_mode: 'radial' });
    const rect = await probe(runtime, [card], [[160, 90], [82, 56], [238, 124]]);
    close(rect.probes[0], [255, 255, 255], 6, 'rectangle centre');
    close(rect.probes[1], [0, 0, 0], 2, 'rectangle corner is past the edge of the ellipse: transparent');
    close(rect.probes[2], [0, 0, 0], 2, 'opposite corner');
  } finally {
    await runtime.close();
  }
});

test('a contour is centred on the edge, can stand alone and has its own colour', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    await runtime.hello({ retain: true });
    // 5% of 180 is a 9px contour: it covers x 75.5..84.5 around the left edge at x 80.
    const outline = await probe(runtime, [shapeLayer({ fill_mode: 'none', stroke: '#ffffff', stroke_width: 5 })], [
      [78, 90], [83, 90], [90, 90], [160, 90], [70, 90], [160, 57], [160, 44], [160, 123], [243, 90],
    ]);
    const [leftOutside, leftInside, inside, centre, farOutside, topInside, farTop, bottomInside, rightOutside] = outline.probes;
    assert.ok(leftOutside[0] > 240, `half of the contour lies outside the declared size: ${leftOutside}`);
    assert.ok(leftInside[0] > 240, `and half inside: ${leftInside}`);
    assert.ok(isBlack(inside) && isBlack(centre), 'a shape without fill shows only its contour');
    assert.ok(isBlack(farOutside) && isBlack(farTop), 'beyond the contour nothing is drawn');
    assert.ok(topInside[0] > 240 && bottomInside[0] > 240, 'the contour runs along every side');
    assert.ok(rightOutside[0] > 240, `right side: ${rightOutside}`);

    const filled = await probe(runtime, [shapeLayer({ stroke: '#0000ff', stroke_width: 5 })], [[160, 90], [80, 90], [78, 90], [92, 90], [74, 90]]);
    assert.ok(isRed(filled.probes[0]) && isRed(filled.probes[3]), 'the fill is inside the contour');
    close(filled.probes[1], [0, 0, 255], 2, 'the contour paints over the fill on the edge');
    close(filled.probes[2], [0, 0, 255], 2, 'and outside it');
    assert.ok(isBlack(filled.probes[4]), 'past the contour: nothing');
    // A rectangle keeps a sharp outer corner (a border, not a rounded pen).
    const corner = await probe(runtime, [shapeLayer({ stroke: '#0000ff', stroke_width: 5 })], [[76, 50], [79, 54]]);
    close(corner.probes[0], [0, 0, 255], 2, 'the outer corner of the contour is square');
  } finally {
    await runtime.close();
  }
});

test('a shadow is cast by the whole silhouette, hard or soft, and moves with its offset', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    await runtime.hello({ retain: true });
    // A hard shadow, 5% of the width (16px) to the right: green where the box is not.
    const hard = await probe(runtime, [shapeLayer({ shadow: '#00ff00', shadow_x: 5, shadow_blur: 0 })], [[200, 90], [248, 90], [262, 90], [248, 120], [248, 40]]);
    assert.ok(isRed(hard.probes[0]), 'the shape itself is not tinted by its shadow');
    close(hard.probes[1], [0, 255, 0], 2, 'the shadow shows right of the box');
    assert.ok(isBlack(hard.probes[2]), 'past the shadow nothing is drawn');
    assert.ok(isBlack(hard.probes[4]), 'the shadow follows the silhouette: nothing above it');

    // A soft shadow: partial colour beside the box, fading with the distance, none far away.
    const soft = await probe(runtime, [shapeLayer({ shadow: '#00ff00', shadow_blur: 6 })], [[77, 90], [70, 90], [40, 90], [160, 90]]);
    const near = soft.probes[0];
    const mid = soft.probes[1];
    assert.ok(near[1] > 40 && near[1] < 200 && near[0] < 24, `soft edge beside the box: ${near}`);
    assert.ok(mid[1] > 0 && mid[1] < near[1], `fainter farther out: ${mid} vs ${near}`);
    assert.ok(isBlack(soft.probes[2]), 'gone far away');
    assert.ok(isRed(soft.probes[3]), 'the body is untouched');

    // A shadow with neither blur nor offset is not drawn: the plain picture (1A) is unchanged.
    const none = await probe(runtime, [shapeLayer({ shadow: '#00ff00' })], [[77, 90]]);
    assert.ok(isBlack(none.probes[0]), 'no blur and no offset draws no shadow');

    // The shadow of a contour-only shape follows the contour.
    const ring = await probe(runtime, [shapeLayer({ fill_mode: 'none', stroke: '#ffffff', stroke_width: 2, shadow: '#00ff00', shadow_x: 3 })], [[160, 90], [162, 54], [165, 90]]);
    assert.ok(isBlack(ring.probes[0]), 'no fill, so no shadow in the middle');
  } finally {
    await runtime.close();
  }
});

test('a path is drawn in its 100x100 box, filled or as a pen stroke, and a style change is another picture', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    await runtime.hello({ retain: true });
    // The default triangle: apex at the top centre of the box, base along the bottom.
    const triangle = await probe(runtime, [shapeLayer({ kind: 'path', path: 'M 50 0 L 100 100 L 0 100 Z' })], [
      [160, 100], [160, 120], [92, 124], [90, 60], [230, 60], [160, 50],
    ]);
    assert.ok(isRed(triangle.probes[0]) && isRed(triangle.probes[1]) && isRed(triangle.probes[2]), 'inside the triangle');
    assert.ok(isBlack(triangle.probes[3]) && isBlack(triangle.probes[4]), 'the empty corners above the base');
    assert.ok(isBlack(triangle.probes[5]), 'above the apex');

    // An open polyline with a pen: only the line is drawn; a curve bends away from its chord.
    const line = { kind: 'path', path: 'M 0 50 L 100 50', fill_mode: 'none', stroke: '#ffffff', stroke_width: 2 };
    const pen = await probe(runtime, [shapeLayer(line)], [[160, 90], [100, 90], [160, 80], [160, 100]]);
    assert.ok(pen.probes[0][0] > 240 && pen.probes[1][0] > 240, 'the line is on the middle of the box');
    assert.ok(isBlack(pen.probes[2]) && isBlack(pen.probes[3]), 'and only there');

    // The quadratic peaks at the top edge of the box (y 54) in its middle and leaves the base empty.
    const curve = await probe(runtime, [shapeLayer({ ...line, path: 'M 0 100 Q 50 -100 100 100' })], [[160, 54], [160, 100]]);
    assert.ok(curve.probes[0][0] > 240, `the curve peaks on the top edge of the box: ${curve.probes[0]}`);
    assert.ok(isBlack(curve.probes[1]), 'an open path draws no chord');

    // Each style is its own cached picture and the same one is served again.
    const first = await runtime.frame([shapeLayer(line)], { size });
    const again = await runtime.frame([shapeLayer(line)], { size });
    assert.equal(again.cache.text_created, 0);
    assert.equal(again.cache.text_reused, 1);
    assert.equal(again.hash, first.hash);
    const thicker = await runtime.frame([shapeLayer({ ...line, stroke_width: 4 })], { size });
    assert.equal(thicker.cache.text_created, 1, 'another contour is another picture');
    assert.notEqual(thicker.hash, first.hash);
  } finally {
    await runtime.close();
  }
});

test('a shape with a contour or shadow stays centred on the clip, so its box does not move', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    await runtime.hello({ retain: true });
    const plain = await probe(runtime, [shapeLayer({})], [[80, 54], [79, 54], [239, 125], [240, 125]]);
    // A black shadow on the black background: invisible, so what is checked is only the box.
    const styled = await probe(runtime, [shapeLayer({ shadow_blur: 8, shadow_x: 4, stroke_width: 0 })], [[80, 54], [79, 54], [239, 125], [240, 125]]);
    assert.ok(isRed(plain.probes[0]) && isBlack(plain.probes[1]), 'the box edge, plain');
    assert.ok(isRed(styled.probes[0]) && isRed(styled.probes[2]), 'the same corners of the box with a shadow around it');
    assert.ok(isBlack(styled.probes[1]) && isBlack(styled.probes[3]), 'and nothing past them');
  } finally {
    await runtime.close();
  }
});

test('a shape layer fills its rectangle, rounds its corners and keeps what it drew', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    await runtime.hello({ retain: true });

    // 50% x 40% of 320x180 is 160x72, centred: x 80..240, y 54..126.
    const square = await exportFrame(runtime, 100, [shapeLayer({})]);
    assert.equal(square.cache.text_created, 1);
    const [centre, cornerIn, left, above, right] = await pixels(runtime, square, [
      [160, 90], [81, 55], [70, 90], [160, 40], [250, 90],
    ]);
    assert.ok(isRed(centre), `centre ${centre}`);
    assert.ok(isRed(cornerIn), `a square corner is filled ${cornerIn}`);
    assert.ok(isBlack(left) && isBlack(above) && isBlack(right), 'outside the rectangle is untouched');

    // The same spec on the same canvas is served from the cache.
    const again = await exportFrame(runtime, 101, [shapeLayer({})]);
    assert.equal(again.cache.text_created, 0);
    assert.equal(again.cache.text_reused, 1);

    // 10% of the canvas height is an 18px radius: the very corner is cut, the middle of the
    // edge is not.
    const rounded = await exportFrame(runtime, 102, [shapeLayer({ radius: 10 })]);
    assert.equal(rounded.cache.text_created, 1, 'another radius is another picture');
    const [roundedCorner, edge, inside] = await pixels(runtime, rounded, [[81, 55], [160, 55], [160, 90]]);
    assert.ok(isBlack(roundedCorner), `the corner is rounded off ${roundedCorner}`);
    assert.ok(isRed(edge) && isRed(inside), 'the straight edge stays filled');

    // A radius beyond half the short side is a pill, never a distorted shape.
    const pill = await exportFrame(runtime, 103, [shapeLayer({ radius: 100 })]);
    const [pillCorner, pillCentre, pillEnd] = await pixels(runtime, pill, [[82, 56], [160, 90], [100, 90]]);
    assert.ok(isBlack(pillCorner) && isRed(pillCentre) && isRed(pillEnd), 'clamped to a pill');

    // Position and opacity are the common ones.
    const moved = await exportFrame(runtime, 104, [shapeLayer({}, {
      opacity: 0.5, transform: transform({ x: 25 }),
    })]);
    const [movedIn, movedOut] = await pixels(runtime, moved, [[300, 90], [100, 90]]);
    assert.ok(movedIn[0] > 100 && movedIn[0] < 156 && movedIn[1] < 16, `half opacity red ${movedIn}`);
    assert.ok(isBlack(movedOut), 'the shape moved right by a quarter of the canvas');
  } finally {
    await runtime.close();
  }
});
