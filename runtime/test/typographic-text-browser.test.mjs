import assert from 'node:assert/strict';
import { test } from 'node:test';

import { openRuntime, plan, skipUnavailable, textLayer, transform } from './support.mjs';

const size = [320, 180];
const title = (reveal) => textLayer('Ação\nPróxima sessão', {
  transform: transform({ x: -45, y: -40 }),
  text: { content: 'Ação\nPróxima sessão', font: 'sans', align: 'left', size: 18, color: '#ffffff', reveal },
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

// Analyse the PNG returned by the bridge, the same picture exported by the CLI.
const bounds = (runtime, frame) => runtime.page.evaluate(async (base64) => {
  const image = new Image();
  image.src = `data:image/png;base64,${base64}`;
  await image.decode();
  const canvas = document.createElement('canvas');
  canvas.width = image.width;
  canvas.height = image.height;
  const context = canvas.getContext('2d');
  context.drawImage(image, 0, 0);
  const { data } = context.getImageData(0, 0, image.width, image.height);
  const result = { left: image.width, top: image.height, bottom: 0, ink: 0 };
  for (let y = 0; y < image.height; y += 1) {
    for (let x = 0; x < image.width; x += 1) {
      if (data[(y * image.width + x) * 4] < 32) continue;
      result.ink += 1;
      result.left = Math.min(result.left, x);
      result.top = Math.min(result.top, y);
      result.bottom = Math.max(result.bottom, y);
    }
  }
  return result;
}, frame.png_data);

test('typographic title keeps a left anchor and reveals from below without rerasterizing', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  try {
    await runtime.hello({ retain: true });
    const settled = await exportFrame(runtime, 100, [title(1)]);
    assert.equal(settled.cache.text_created, 1);
    assert.equal(await runtime.page.evaluate(() =>
      [...document.fonts].some((face) => face.family === 'Tessel Inter' && face.status === 'loaded')), true);
    const ink = await bounds(runtime, settled);
    assert.ok(ink.left >= 14 && ink.left <= 22, JSON.stringify(ink));
    assert.ok(ink.top >= 18 && ink.top <= 42, JSON.stringify(ink));
    assert.ok(ink.bottom > ink.top + 42, 'the explicit second line is rendered');

    const entering = await exportFrame(runtime, 101, [title(0.45)]);
    assert.equal(entering.cache.text_created, 0);
    assert.equal(entering.cache.text_reused, 1);
    const partial = await bounds(runtime, entering);
    assert.ok(partial.ink > 0 && partial.ink < ink.ink, JSON.stringify(partial));
    assert.ok(partial.top > ink.top + 20, 'only the lower part of the title is visible');
    assert.equal(partial.bottom, ink.bottom, 'masking does not move the letters');

    const hidden = await exportFrame(runtime, 102, [title(0)]);
    assert.equal((await bounds(runtime, hidden)).ink, 0);
    const seek = await runtime.frame([title(undefined)], { size, quality: 'preview' });
    assert.equal(seek.hash, settled.hash, 'settled preview and export produce the same pixels after seeking');
  } finally {
    await runtime.close();
  }
});

// F070: typing and line reveal draw the first `visible` characters of the core's lines on a canvas
// sized from the whole layout, so the letters already shown never move.
const typed = (font, visible) => textLayer('um dois tres quatro', {
  transform: transform({ x: 0, y: 0 }),
  text: {
    content: 'um dois tres quatro', font, align: 'left', size: 14, color: '#ffffff',
    lines: ['um dois', 'tres', 'quatro'], width: 60, visible,
  },
});

for (const font of ['sans', 'bitmap']) {
  test(`${font} text typed by character keeps the layout stable and adds ink`, async (t) => {
    if (skipUnavailable(t, assert)) return;
    const runtime = await openRuntime();
    try {
      await runtime.hello({ retain: true });
      const whole = await bounds(runtime, await exportFrame(runtime, 200, [typed(font, undefined)]));
      assert.ok(whole.ink > 0, JSON.stringify(whole));

      const none = await bounds(runtime, await exportFrame(runtime, 201, [typed(font, 0)]));
      assert.equal(none.ink, 0, 'nothing typed yet draws nothing');

      const first = await bounds(runtime, await exportFrame(runtime, 202, [typed(font, 3)]));
      const second = await bounds(runtime, await exportFrame(runtime, 203, [typed(font, 10)]));
      const last = await bounds(runtime, await exportFrame(runtime, 204, [typed(font, 19)]));
      assert.ok(first.ink > 0 && first.ink < second.ink && second.ink < last.ink,
        JSON.stringify({ first, second, last }));
      // Letters already shown stay where the whole text puts them: the first line starts at the
      // same column and top as when the text is complete, and the full count is the whole text.
      // Glyph side bearings differ by a pixel or two between letters, so compare the column loosely.
      assert.ok(first.left >= whole.left && first.left - whole.left <= 3, JSON.stringify({ first, whole }));
      // "um" has no ascender, so only "um dois" (with its `d`) reaches the top of the whole text.
      assert.equal(second.top, whole.top);
      assert.deepEqual(last, whole, 'every character typed is the settled picture');
    } finally {
      await runtime.close();
    }
  });
}
