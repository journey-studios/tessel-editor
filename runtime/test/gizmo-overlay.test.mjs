// The gizmo overlay on a real page and a real canvas (F055): real mouse events from puppeteer, a
// real 2D canvas whose pixels are read back, and the policy for Escape, blur, hidden, resize and
// a lost pointer. The geometry itself is the core's; this proves the page half of the seam.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { openRuntime, skipUnavailable } from './support.mjs';

const STAMP = { scene: 'a', session: 'session-1', revision: 1, frame: 0 };

const overlayJson = (extra = {}) => ({
  version: 1,
  visible: true,
  reason: null,
  entity: 'scene/a/node/n',
  origin: { u: 0.5, v: 0.5 },
  handles: [
    { id: 'x', kind: 'axis', from: { u: 0.5, v: 0.5 }, to: { u: 0.7, v: 0.5 } },
    { id: 'y', kind: 'axis', from: { u: 0.5, v: 0.5 }, to: { u: 0.5, v: 0.3 } },
    { id: 'screen', kind: 'center', from: { u: 0.5, v: 0.5 }, to: { u: 0.5, v: 0.5 } },
  ],
  outlines: [],
  active: null,
  aspect: 160 / 90,
  stamp: STAMP,
  metrics: { handle_length: 0.14, hit_radius: 0.025, center_radius: 0.03, unit: 'picture_height' },
  ...extra,
});

const open = async (t) => {
  if (skipUnavailable(t, assert)) return null;
  const runtime = await openRuntime({ overlay: true });
  t.after(() => runtime.close());
  const ready = await runtime.hello();
  assert.ok(ready.capabilities.features.includes('overlay.gizmo'), 'the page negotiates the overlay');
  return runtime;
};

const state = (runtime) => runtime.page.evaluate(() => {
  const s = window.TesselRuntime.debug.overlay.state();
  return s ? { ...s, overlay: undefined } : null;
});

/** The picture rectangle in pane pixels, as the page computes it. */
const picture = async (runtime) => (await state(runtime)).rect;
const pane = (rect, point) => ({ x: rect.x + point.u * rect.width, y: rect.y + point.v * rect.height });

const pointerMessages = (runtime) => runtime.messages.filter((m) => m.type === 'overlay.pointer');
const settle = (runtime) => runtime.page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));

const enable = async (runtime, extra = {}) => {
  await runtime.send({ type: 'overlay.enable', enabled: true });
  await runtime.send({ type: 'overlay.set', overlay: overlayJson(extra), ack: true });
  return runtime.waitFor((m) => m.type === 'overlay.drawn', 'overlay.drawn');
};

test('the overlay is drawn on the canvas and the pixels say so', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const layout = runtime.waitFor((m) => m.type === 'overlay.layout', 'overlay.layout');
  const drawn = await enable(runtime);
  const reported = await layout;
  assert.ok(reported.picture.width > 0 && reported.output.width > 0);
  assert.equal(drawn.visible, true);
  assert.equal(drawn.entity, 'scene/a/node/n');
  const [x, y, screen] = drawn.probes;
  assert.deepEqual([x.id, y.id, screen.id], ['x', 'y', 'screen']);
  // The middle of the x axis is red (the axis colour), the y axis green: really painted pixels.
  assert.ok(x.rgba[3] > 200 && x.rgba[0] > x.rgba[1] && x.rgba[0] > x.rgba[2], `x pixel ${x.rgba}`);
  assert.ok(y.rgba[3] > 200 && y.rgba[1] > y.rgba[0], `y pixel ${y.rgba}`);
  assert.ok(screen.rgba[3] > 0, `centre pixel ${screen.rgba}`);
  // A pixel far from any handle is untouched.
  const far = await runtime.page.evaluate(() => {
    const surface = document.getElementById('overlay');
    return [...surface.getContext('2d').getImageData(3, 3, 1, 1).data];
  });
  assert.deepEqual(far, [0, 0, 0, 0]);
  // overlay.clear wipes it.
  await runtime.send({ type: 'overlay.clear' });
  await runtime.send({ type: 'overlay.set', overlay: overlayJson({ visible: false, handles: [], origin: null, reason: 'no_selection' }), ack: true });
  const cleared = await runtime.waitFor((m) => m.type === 'overlay.drawn');
  assert.equal(cleared.visible, false);
  assert.deepEqual(cleared.probes, []);
});

test('a real press, drag and release post picture coordinates; moves are coalesced', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await enable(runtime);
  const rect = await picture(runtime);
  const down = pane(rect, { u: 0.5, v: 0.5 });
  const end = pane(rect, { u: 0.62, v: 0.55 });
  await runtime.page.mouse.move(down.x, down.y);
  await runtime.page.mouse.down();
  for (let step = 1; step <= 12; step += 1) {
    const f = step / 12;
    await runtime.page.mouse.move(down.x + (end.x - down.x) * f, down.y + (end.y - down.y) * f);
  }
  await runtime.page.mouse.up();
  await settle(runtime);
  const messages = pointerMessages(runtime);
  assert.equal(messages[0].phase, 'down');
  assert.ok(Math.abs(messages[0].point.u - 0.5) < 1e-9 && Math.abs(messages[0].point.v - 0.5) < 1e-9);
  const up = messages.at(-1);
  assert.equal(up.phase, 'up');
  assert.ok(Math.abs(up.point.u - 0.62) < 1e-6 && Math.abs(up.point.v - 0.55) < 1e-6, JSON.stringify(up.point));
  const moves = messages.filter((m) => m.phase === 'move');
  assert.ok(moves.length <= 12, `moves are coalesced per frame, got ${moves.length}`);
  assert.deepEqual(messages.map((m) => m.seq), messages.map((_, i) => messages[0].seq + i), 'sequence is gapless');
  assert.equal(messages.filter((m) => m.phase === 'cancel').length, 0);
});

test('a press on the letterbox bar or with the overlay off is not a gizmo press', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await enable(runtime);
  const rect = await picture(runtime);
  assert.ok(rect.y > 0 || rect.x > 0, 'the test page letterboxes the picture');
  const bar = rect.y > 0 ? { x: rect.x + 10, y: rect.y / 2 } : { x: rect.x / 2, y: rect.y + 10 };
  await runtime.page.mouse.move(bar.x, bar.y);
  await runtime.page.mouse.down();
  await runtime.page.mouse.up();
  assert.deepEqual(pointerMessages(runtime), [], 'outside the picture');

  await runtime.send({ type: 'overlay.enable', enabled: false });
  const inside = pane(rect, { u: 0.5, v: 0.5 });
  await runtime.page.mouse.move(inside.x, inside.y);
  await runtime.page.mouse.down();
  await runtime.page.mouse.up();
  assert.deepEqual(pointerMessages(runtime), [], 'disabled: the canvas does not take the event');
  const events = await runtime.page.evaluate(() => getComputedStyle(document.getElementById('overlay')).pointerEvents);
  assert.equal(events, 'none');
});

test('Escape cancels the drag and the rest of the press is silent', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await enable(runtime);
  const rect = await picture(runtime);
  const start = pane(rect, { u: 0.5, v: 0.5 });
  await runtime.page.mouse.move(start.x, start.y);
  await runtime.page.mouse.down();
  await runtime.page.mouse.move(start.x + 30, start.y);
  await settle(runtime);
  await runtime.page.keyboard.press('Escape');
  await runtime.page.mouse.move(start.x + 60, start.y);
  await runtime.page.mouse.up();
  await settle(runtime);
  const phases = pointerMessages(runtime).map((m) => m.phase);
  assert.deepEqual(phases.filter((p) => p !== 'move'), ['down', 'cancel']);
  const cancel = pointerMessages(runtime).find((m) => m.phase === 'cancel');
  assert.equal(cancel.reason, 'escape');
  assert.equal(phases.at(-1), 'cancel', 'nothing after the cancel: no move, no release');
  // The next press starts clean.
  await runtime.page.mouse.move(start.x, start.y);
  await runtime.page.mouse.down();
  await runtime.page.mouse.up();
  await settle(runtime);
  const last = pointerMessages(runtime).at(-1);
  assert.equal(last.phase, 'up');
});

test('losing the window focus or the page visibility cancels, never commits', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await enable(runtime);
  const rect = await picture(runtime);
  const start = pane(rect, { u: 0.5, v: 0.5 });
  for (const [reason, trigger] of [
    ['blur', () => window.dispatchEvent(new Event('blur'))],
    ['hidden', () => {
      Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' });
      document.dispatchEvent(new Event('visibilitychange'));
      delete document.visibilityState;
    }],
    ['pointer_cancel', () => document.getElementById('overlay').dispatchEvent(new Event('pointercancel'))],
  ]) {
    runtime.messages.length = 0;
    await runtime.page.mouse.move(start.x, start.y);
    await runtime.page.mouse.down();
    await runtime.page.evaluate(trigger);
    await runtime.page.mouse.up();
    await settle(runtime);
    const phases = pointerMessages(runtime).map((m) => [m.phase, m.reason]);
    assert.deepEqual(phases, [['down', undefined], ['cancel', reason]], reason);
  }
});

test('only the primary button of the primary pointer presses; a secondary pointer neither starts nor ends a drag', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await enable(runtime);
  const rect = await picture(runtime);
  const start = pane(rect, { u: 0.5, v: 0.5 });
  await runtime.page.mouse.move(start.x, start.y);
  for (const button of ['right', 'middle']) {
    await runtime.page.mouse.down({ button });
    await runtime.page.mouse.up({ button });
  }
  assert.deepEqual(pointerMessages(runtime), [], 'a right or middle button is not a gizmo press');

  const secondary = (type) => runtime.page.evaluate((kind, x, y) => {
    document.getElementById('overlay').dispatchEvent(new PointerEvent(kind, {
      pointerId: 7, isPrimary: false, button: 0, clientX: x, clientY: y, bubbles: true,
    }));
  }, type, start.x, start.y);
  await secondary('pointerdown');
  await secondary('pointerup');
  assert.deepEqual(pointerMessages(runtime), [], 'a second finger is not a gizmo press');

  await runtime.page.mouse.down();
  await secondary('pointerup');
  await settle(runtime);
  assert.deepEqual(pointerMessages(runtime).map((m) => m.phase), ['down'], 'it did not release the real press');
  await runtime.page.mouse.up();
  await settle(runtime);
  assert.deepEqual(pointerMessages(runtime).map((m) => m.phase), ['down', 'up']);
});

test('a lost pointer capture (a real release of it, not a synthetic event) cancels the drag', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await enable(runtime);
  const rect = await picture(runtime);
  const start = pane(rect, { u: 0.5, v: 0.5 });
  await runtime.page.mouse.move(start.x, start.y);
  await runtime.page.mouse.down();
  const held = await runtime.page.evaluate(() => {
    const surface = document.getElementById('overlay');
    const had = surface.hasPointerCapture(1);
    if (had) surface.releasePointerCapture(1);
    return had;
  });
  assert.equal(held, true, 'the press captured the pointer (otherwise this test proves nothing)');
  await runtime.page.mouse.move(start.x + 20, start.y);
  await runtime.page.mouse.up();
  await settle(runtime);
  const phases = pointerMessages(runtime).map((m) => [m.phase, m.reason]);
  assert.deepEqual(phases, [['down', undefined], ['cancel', 'pointer_cancel']]);
});

test('resizing during a drag does not end it: the same pointer keeps the same picture point', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await enable(runtime);
  const before = await picture(runtime);
  const start = pane(before, { u: 0.5, v: 0.5 });
  await runtime.page.mouse.move(start.x, start.y);
  await runtime.page.mouse.down();
  await runtime.page.setViewport({ width: 500, height: 700 });
  await settle(runtime);
  const after = await picture(runtime);
  assert.notDeepEqual(after, before, 'the picture was re-fitted');
  const target = pane(after, { u: 0.7, v: 0.4 });
  await runtime.page.mouse.move(target.x, target.y);
  await runtime.page.mouse.up();
  await settle(runtime);
  const messages = pointerMessages(runtime);
  assert.deepEqual(messages.map((m) => m.phase).filter((p) => p !== 'move'), ['down', 'up'], 'no cancel from a resize');
  const up = messages.at(-1);
  assert.ok(Math.abs(up.point.u - 0.7) < 1e-6 && Math.abs(up.point.v - 0.4) < 1e-6, JSON.stringify(up.point));
  const layouts = runtime.messages.filter((m) => m.type === 'overlay.layout');
  assert.ok(layouts.length >= 1, 'the host is told where the picture is now');
});

test('hovering a handle shows the grab cursor; an unsupported overlay is refused whole', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await enable(runtime);
  const rect = await picture(runtime);
  const onAxis = pane(rect, { u: 0.6, v: 0.5 });
  await runtime.page.mouse.move(onAxis.x, onAxis.y);
  await settle(runtime);
  assert.equal(await runtime.page.evaluate(() => document.getElementById('overlay').style.cursor), 'grab');
  const away = pane(rect, { u: 0.2, v: 0.8 });
  await runtime.page.mouse.move(away.x, away.y);
  await settle(runtime);
  assert.equal(await runtime.page.evaluate(() => document.getElementById('overlay').style.cursor), 'default');

  await runtime.send({ type: 'overlay.set', overlay: overlayJson({ version: 2 }) });
  const refused = await runtime.waitFor((m) => m.type === 'error', 'error');
  assert.match(JSON.stringify(refused), /version 2/);
  assert.equal((await state(runtime)).enabled, true, 'the page stays up with the previous overlay');
});

test('every pointer message and the ack echo the stamp of the overlay they answer', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const drawn = await enable(runtime);
  assert.deepEqual(drawn.stamp, STAMP);
  const rect = await picture(runtime);
  const start = pane(rect, { u: 0.5, v: 0.5 });
  await runtime.page.mouse.move(start.x, start.y);
  await runtime.page.mouse.down();
  // A new overlay (another revision) arrives mid-press: the press keeps answering the one it began on.
  await runtime.send({ type: 'overlay.set', overlay: overlayJson({ stamp: { ...STAMP, revision: 2 } }) });
  await runtime.page.mouse.move(start.x + 20, start.y);
  await settle(runtime);
  await runtime.page.mouse.up();
  await settle(runtime);
  const messages = pointerMessages(runtime);
  assert.ok(messages.length >= 2);
  for (const message of messages) assert.deepEqual(message.stamp, STAMP, `${message.phase} carries the press stamp`);
  // The next press answers the overlay that is current now.
  runtime.messages.length = 0;
  await runtime.page.mouse.move(start.x, start.y);
  await runtime.page.mouse.down();
  await runtime.page.mouse.up();
  await settle(runtime);
  assert.deepEqual(pointerMessages(runtime)[0].stamp, { ...STAMP, revision: 2 });
});

test('a canvas of another shape than the overlay describes draws nothing and takes no press', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await enable(runtime);
  assert.equal((await state(runtime)).inStep, true);

  // The output turned portrait: the canvas holds a portrait picture, the overlay still describes
  // the landscape one until the host sends the one for the new output.
  const done = await runtime.frame([], { size: [90, 160] });
  assert.equal(done.type, 'frame.done');
  assert.equal((await state(runtime)).inStep, false, 'the page noticed after the frame');
  const layout = await runtime.waitFor((m) => m.type === 'overlay.layout' && m.in_step === false, 'layout out of step');
  assert.deepEqual(layout.output, { width: 90, height: 160 });
  const blank = await runtime.page.evaluate(() => {
    const surface = document.getElementById('overlay');
    const { data } = surface.getContext('2d').getImageData(0, 0, surface.width, surface.height);
    return data.every((byte) => byte === 0);
  });
  assert.equal(blank, true, 'nothing is drawn over a picture it does not describe');
  const rect = await picture(runtime);
  const middle = pane(rect, { u: 0.5, v: 0.5 });
  await runtime.page.mouse.move(middle.x, middle.y);
  await runtime.page.mouse.down();
  await runtime.page.mouse.up();
  assert.deepEqual(pointerMessages(runtime), [], 'no press is posted against a picture that is not the overlay\'s');

  // The overlay for the portrait output arrives: drawn and live again, over the portrait picture.
  await runtime.send({
    type: 'overlay.set',
    overlay: overlayJson({ aspect: 90 / 160, stamp: { ...STAMP, revision: 3 } }),
    ack: true,
  });
  const drawn = await runtime.waitFor((m) => m.type === 'overlay.drawn', 'overlay.drawn');
  assert.ok(drawn.probes[0].rgba[3] > 200, 'drawn over the portrait picture');
  const portrait = await picture(runtime);
  assert.ok(portrait.height > portrait.width, 'the picture is contained as a portrait');
  const press = pane(portrait, { u: 0.5, v: 0.5 });
  await runtime.page.mouse.move(press.x, press.y);
  await runtime.page.mouse.down();
  await runtime.page.mouse.up();
  await settle(runtime);
  const messages = pointerMessages(runtime);
  assert.deepEqual(messages.map((m) => m.phase), ['down', 'up']);
  assert.ok(Math.abs(messages[0].point.u - 0.5) < 1e-9 && Math.abs(messages[0].point.v - 0.5) < 1e-9);
});

// ---- M1 of the PR #25 review: the overlay costs nothing when off, never after the frame's answer ----

const surfaceProbe = (runtime) => runtime.page.evaluate(() => {
  // Counts every layout read of the overlay surface (what `redraw` does first) from now on.
  const surface = window.TesselRuntime.debug.overlay?.surface;
  if (!surface) return false;
  window.__layoutReads = 0;
  for (const name of ['clientWidth', 'clientHeight']) {
    Object.defineProperty(surface, name, { configurable: true, get: () => { window.__layoutReads += 1; return 800; } });
  }
  return true;
});
const layoutReads = (runtime) => runtime.page.evaluate(() => window.__layoutReads);

test('an overlay that is off does no layout and no canvas work for a frame (and the frame is unchanged)', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  assert.equal(await surfaceProbe(runtime), true);
  const before = await runtime.page.evaluate(() => {
    const surface = window.TesselRuntime.debug.overlay.surface;
    return { width: surface.width, height: surface.height };
  });
  const done = await runtime.frame([]);
  assert.equal(done.type, 'frame.done');
  await settle(runtime);
  assert.equal(await layoutReads(runtime), 0, 'a frame never reads the layout of an overlay that is off');
  assert.equal(runtime.messages.filter((m) => m.type === 'overlay.layout').length, 0);
  const after = await runtime.page.evaluate(() => {
    const surface = window.TesselRuntime.debug.overlay.surface;
    return { width: surface.width, height: surface.height };
  });
  assert.deepEqual(after, before, 'the overlay canvas was not resized');

  // The probe is real: with the overlay on, the same frame does read the layout.
  await runtime.send({ type: 'overlay.enable', enabled: true });
  await runtime.page.evaluate(() => { window.__layoutReads = 0; });
  await runtime.frame([]);
  assert.ok((await layoutReads(runtime)) > 0, 'an active overlay is refreshed after the frame');
});

test('an overlay that fails after a frame is its own error: done and presented stand, no frame.failed', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await runtime.hello({ telemetry: { presented: true } });
  await runtime.send({ type: 'overlay.enable', enabled: true });
  await runtime.page.evaluate(() => {
    const surface = window.TesselRuntime.debug.overlay.surface;
    Object.defineProperty(surface, 'clientWidth', { configurable: true, get: () => { throw new Error('layout exploded'); } });
  });
  runtime.messages.length = 0;
  const done = await runtime.frame([], { id: 50 });
  assert.equal(done.type, 'frame.done', 'the frame succeeded');
  const presented = await runtime.waitFor((m) => m.type === 'frame.presented' && m.id === 50, 'frame.presented');
  assert.equal(presented.id, 50);
  const types = runtime.messages.map((m) => m.type);
  assert.ok(!types.includes('frame.failed'), `no frame.failed after the success, got ${types}`);
  const failure = runtime.messages.find((m) => m.type === 'error');
  assert.equal(failure?.error.code, 'overlay_failed');
  assert.match(failure.error.message, /layout exploded/);
  // The next frame is still served.
  await runtime.page.evaluate(() => delete window.TesselRuntime.debug.overlay.surface.clientWidth);
  const next = await runtime.frame([], { id: 51 });
  assert.equal(next.type, 'frame.done');
});

test('the headless page (no overlay surface) has no overlay.gizmo, no canvas and refuses overlay messages', async (t) => {
  if (skipUnavailable(t, assert)) return;
  const runtime = await openRuntime();
  t.after(() => runtime.close());
  const ready = await runtime.hello();
  assert.ok(!ready.capabilities.features.includes('overlay.gizmo'), 'the worker never announces the overlay');
  assert.equal(await runtime.page.evaluate(() => document.getElementById('overlay')), null);
  assert.equal(await runtime.page.evaluate(() => window.TesselRuntime.debug.overlay), null);
  const done = await runtime.frame([]);
  assert.equal(done.type, 'frame.done');
  await runtime.send({ type: 'overlay.enable', enabled: true });
  const refused = await runtime.waitFor((m) => m.type === 'error', 'error');
  assert.equal(refused.error.code, 'unsupported');
});
