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

test('a release right after a lost capture (no move in between) is a cancel, never an up', async (t) => {
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

  runtime.messages.length = 0;
  await runtime.send({ type: 'overlay.set', overlay: overlayJson({ version: 2 }) });
  const refused = await runtime.waitFor((m) => m.type === 'overlay.error', 'overlay.error');
  assert.match(JSON.stringify(refused), /version 2/);
  assert.equal(refused.request, 'overlay.set');
  assert.equal(refused.error.code, 'invalid_overlay');
  // An overlay that is operational and refuses a payload is the overlay's failure: the generic
  // `error` would be read by the host as a closed backend (S8, thread 4504060122).
  assert.ok(!runtime.messages.some((m) => m.type === 'error'), 'no generic error for an overlay failure');
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
  const failure = runtime.messages.find((m) => m.type === 'overlay.error');
  assert.equal(failure?.error.code, 'overlay_failed');
  assert.match(failure.error.message, /layout exploded/);
  // The next frame is still served.
  await runtime.page.evaluate(() => delete window.TesselRuntime.debug.overlay.surface.clientWidth);
  const next = await runtime.frame([], { id: 51 });
  assert.equal(next.type, 'frame.done');
});

test('an overlay payload the page refuses is its own error and the frame after it is served', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await runtime.hello({ telemetry: { presented: true } });
  await runtime.send({ type: 'overlay.enable', enabled: true });
  runtime.messages.length = 0;
  await runtime.send({ type: 'overlay.set', overlay: { version: 99 } });
  const failure = await runtime.waitFor((m) => m.type === 'overlay.error', 'overlay.error');
  assert.equal(failure.request, 'overlay.set');
  assert.equal(failure.error.code, 'invalid_overlay');
  assert.match(failure.error.message, /version 99/);
  const done = await runtime.frame([], { id: 60 });
  assert.equal(done.type, 'frame.done', 'the page still renders');
  const presented = await runtime.waitFor((m) => m.type === 'frame.presented' && m.id === 60, 'frame.presented');
  assert.equal(presented.id, 60);
  const types = runtime.messages.map((m) => m.type);
  assert.ok(!types.includes('error') && !types.includes('frame.failed'), `only overlay.error was said, got ${types}`);
  // The feature is still the page's: a good overlay is drawn afterwards.
  await runtime.send({ type: 'overlay.set', overlay: overlayJson(), ack: true });
  assert.equal((await runtime.waitFor((m) => m.type === 'overlay.drawn', 'overlay.drawn')).stamp.revision, STAMP.revision);
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

// ---- BL015.1: the rotate tool's rings on the real canvas -----------------------------------------

const ASPECT = 160 / 90;
const ellipse = (rx, ry, gap = null) =>
  Array.from({ length: 65 }, (_, index) => {
    if (gap !== null && index >= gap[0] && index < gap[1]) return null;
    const angle = (2 * Math.PI * (index % 64)) / 64;
    return { u: 0.5 + (rx * Math.cos(angle)) / ASPECT, v: 0.5 + ry * Math.sin(angle) };
  });
const ringHandle = (id, points) => ({ id, kind: 'ring', from: { u: 0.5, v: 0.5 }, to: points.find((p) => p !== null), points });
const ringOverlay = (extra = {}) => ({
  tool: 'rotate',
  space: 'world',
  handles: [
    ringHandle('x', ellipse(0.04, 0.1)),
    ringHandle('y', ellipse(0.1, 0.04)),
    ringHandle('z', ellipse(0.1, 0.1, [8, 16])),
  ],
  metrics: { handle_length: 0.14, hit_radius: 0.025, center_radius: 0.03, ring_radius: 0.1, unit: 'picture_height' },
  ...extra,
});

test('the rings are drawn on the canvas, in the axis colours, and the centre stays empty', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const drawn = await enable(runtime, ringOverlay());
  assert.equal(drawn.tool, 'rotate');
  assert.equal(drawn.space, 'world');
  const [x, y, z] = drawn.probes;
  assert.deepEqual([x.id, y.id, z.id], ['x', 'y', 'z']);
  // The probe of a ring is one of its own vertices: a really painted pixel in the axis colour.
  assert.ok(x.rgba[3] > 200 && x.rgba[0] > x.rgba[1] && x.rgba[0] > x.rgba[2], `x ring pixel ${x.rgba}`);
  assert.ok(y.rgba[3] > 200 && y.rgba[1] > y.rgba[0] && y.rgba[1] > y.rgba[2], `y ring pixel ${y.rgba}`);
  assert.ok(z.rgba[3] > 200 && z.rgba[2] > z.rgba[0] && z.rgba[2] > z.rgba[1], `z ring pixel ${z.rgba}`);
  // No centre disc with this tool, and the gap of the z ring (behind the camera) is not stroked.
  const rect = await picture(runtime);
  const pixels = await runtime.page.evaluate(({ rect: r, aspect }) => {
    const surface = document.getElementById('overlay');
    const context = surface.getContext('2d');
    const ratio = window.devicePixelRatio || 1;
    const at = (u, v) => [...context.getImageData(Math.round((r.x + u * r.width) * ratio), Math.round((r.y + v * r.height) * ratio), 1, 1).data];
    const angle = (2 * Math.PI * 12) / 64; // the middle of the z ring's gap, away from the x and y rings
    return { centre: at(0.5, 0.5), gap: at(0.5 + (0.1 * Math.cos(angle)) / aspect, 0.5 + 0.1 * Math.sin(angle)) };
  }, { rect, aspect: ASPECT });
  assert.equal(pixels.centre[3], 0, `centre ${pixels.centre}`);
  assert.equal(pixels.gap[3], 0, `gap ${pixels.gap}`);
});

test('a real press on a ring and a drag around it post picture coordinates; the centre is not a press target', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await enable(runtime, ringOverlay());
  const rect = await picture(runtime);
  const ring = ellipse(0.1, 0.1, [8, 16]);
  // Hover: a ring shows the grab cursor, the centre does not.
  const hoverCursor = async (point) => {
    const at = pane(rect, point);
    await runtime.page.mouse.move(at.x, at.y);
    await settle(runtime);
    return runtime.page.evaluate(() => document.getElementById('overlay').style.cursor);
  };
  assert.equal(await hoverCursor(ring[24]), 'grab');
  assert.equal(await hoverCursor({ u: 0.5, v: 0.5 }), 'default');
  // Press on the ring, sweep a quarter of it, release.
  const start = pane(rect, ring[24]);
  await runtime.page.mouse.move(start.x, start.y);
  await runtime.page.mouse.down();
  for (const index of [26, 28, 30, 32, 34, 36, 38, 40]) {
    const at = pane(rect, ring[index]);
    await runtime.page.mouse.move(at.x, at.y);
  }
  const end = pane(rect, ring[40]);
  await runtime.page.mouse.up();
  await settle(runtime);
  const messages = pointerMessages(runtime);
  assert.equal(messages[0].phase, 'down');
  assert.ok(Math.abs(messages[0].point.u - ring[24].u) < 1e-6 && Math.abs(messages[0].point.v - ring[24].v) < 1e-6, JSON.stringify(messages[0].point));
  const up = messages.at(-1);
  assert.equal(up.phase, 'up');
  assert.ok(Math.abs(up.point.u - ring[40].u) < 1e-6 && Math.abs(up.point.v - ring[40].v) < 1e-6, JSON.stringify([up.point, end]));
  assert.ok(messages.every((m) => m.stamp.session === STAMP.session && m.stamp.revision === STAMP.revision), 'every message echoes the stamp');
  assert.equal(messages.filter((m) => m.phase === 'cancel').length, 0);
});

test('Escape and a lost focus cancel a ring drag like an arrow drag', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await enable(runtime, ringOverlay());
  const rect = await picture(runtime);
  const ring = ellipse(0.1, 0.1, [8, 16]);
  const start = pane(rect, ring[24]);
  const next = pane(rect, ring[30]);
  for (const [reason, end] of [
    ['escape', () => runtime.page.keyboard.press('Escape')],
    ['blur', () => runtime.page.evaluate(() => window.dispatchEvent(new Event('blur')))],
  ]) {
    runtime.messages.length = 0;
    await runtime.page.mouse.move(start.x, start.y);
    await runtime.page.mouse.down();
    await runtime.page.mouse.move(next.x, next.y);
    await settle(runtime);
    await end();
    await runtime.page.mouse.move(start.x, start.y);
    await runtime.page.mouse.up();
    await settle(runtime);
    const phases = pointerMessages(runtime).map((m) => m.phase);
    assert.deepEqual(phases.filter((p) => p !== 'move'), ['down', 'cancel'], reason);
    assert.equal(pointerMessages(runtime).find((m) => m.phase === 'cancel').reason, reason);
    assert.equal(phases.at(-1), 'cancel', `${reason}: nothing after the cancel`);
  }
});

test('switching from the rings to the arrows replaces what is drawn and what is hit', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await enable(runtime, ringOverlay());
  const rect = await picture(runtime);
  const ring = ellipse(0.1, 0.1, [8, 16]);
  const at = pane(rect, ring[24]);
  await runtime.page.mouse.move(at.x, at.y);
  await settle(runtime);
  assert.equal((await runtime.page.evaluate(() => window.TesselRuntime.debug.overlay.state().hover)), 'z');
  // The arrows come back (the translate overlay of the first slice): that point hits nothing now.
  await runtime.send({ type: 'overlay.set', overlay: overlayJson({ stamp: { ...STAMP, revision: 2 } }), ack: true });
  const drawn = await runtime.waitFor((m) => m.type === 'overlay.drawn' && m.stamp?.revision === 2, 'overlay.drawn');
  assert.equal(drawn.tool, 'translate');
  await runtime.page.mouse.move(at.x + 1, at.y + 1);
  await settle(runtime);
  assert.equal((await runtime.page.evaluate(() => window.TesselRuntime.debug.overlay.state().hover)), null);
  assert.deepEqual(drawn.probes.map((p) => p.id), ['x', 'y', 'screen']);
});

// ---- BL015.1 (G1.2): the scale tool's handles on the real canvas ----------------------------------

const scaleOverlay = (extra = {}) => ({
  tool: 'scale',
  space: 'local',
  handles: [
    { id: 'x', kind: 'scale_axis', from: { u: 0.5, v: 0.5 }, to: { u: 0.62, v: 0.5 } },
    { id: 'y', kind: 'scale_axis', from: { u: 0.5, v: 0.5 }, to: { u: 0.5, v: 0.36 } },
    { id: 'z', kind: 'scale_axis', from: { u: 0.5, v: 0.5 }, to: { u: 0.44, v: 0.58 } },
    { id: 'uniform', kind: 'scale_uniform', from: { u: 0.5, v: 0.5 }, to: { u: 0.5 + 0.0636 / ASPECT, v: 0.5 - 0.0636 } },
  ],
  disabled_handles: [],
  metrics: { handle_length: 0.14, hit_radius: 0.025, center_radius: 0.03, uniform_offset: 0.09, unit: 'picture_height' },
  ...extra,
});

test('the scale handles are boxes on the canvas in the axis colours, the uniform one is light, and nothing is painted at the centre', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const drawn = await enable(runtime, scaleOverlay());
  assert.equal(drawn.tool, 'scale');
  assert.deepEqual(drawn.disabled, []);
  const [x, y, z, uniform] = drawn.probes;
  assert.deepEqual([x.id, y.id, z.id, uniform.id], ['x', 'y', 'z', 'uniform']);
  // The probe of a scale handle is the middle of its box: a really painted pixel in its colour.
  assert.ok(x.rgba[3] > 200 && x.rgba[0] > x.rgba[1] && x.rgba[0] > x.rgba[2], `x pixel ${x.rgba}`);
  assert.ok(y.rgba[3] > 200 && y.rgba[1] > y.rgba[0] && y.rgba[1] > y.rgba[2], `y pixel ${y.rgba}`);
  assert.ok(z.rgba[3] > 200 && z.rgba[2] > z.rgba[0] && z.rgba[2] > z.rgba[1], `z pixel ${z.rgba}`);
  assert.ok(uniform.rgba[3] > 200 && Math.min(...uniform.rgba.slice(0, 3)) > 200, `uniform pixel ${uniform.rgba}`);
  // The shaft of an axis is painted halfway; the node's own origin is not (no disc for this tool).
  const rect = await picture(runtime);
  const pixels = await runtime.page.evaluate(({ rect: r }) => {
    const context = document.getElementById('overlay').getContext('2d');
    const ratio = window.devicePixelRatio || 1;
    const at = (u, v) => [...context.getImageData(Math.round((r.x + u * r.width) * ratio), Math.round((r.y + v * r.height) * ratio), 1, 1).data];
    return { shaft: at(0.56, 0.5), far: at(0.9, 0.9) };
  }, { rect });
  assert.ok(pixels.shaft[3] > 200, `shaft ${pixels.shaft}`);
  assert.equal(pixels.far[3], 0, `nothing elsewhere ${pixels.far}`);
});

test('the handles the core disabled are reported with their reason and not drawn or pressable', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const overlay = scaleOverlay({ disabled_handles: [{ id: 'x', reason: 'world_axis_shear' }] });
  overlay.handles = overlay.handles.filter((handle) => handle.id !== 'x');
  const drawn = await enable(runtime, overlay);
  assert.deepEqual(drawn.disabled, [{ id: 'x', reason: 'world_axis_shear' }]);
  assert.deepEqual(drawn.probes.map((p) => p.id), ['y', 'z', 'uniform']);
  const rect = await picture(runtime);
  const at = pane(rect, { u: 0.56, v: 0.5 });
  await runtime.page.mouse.move(at.x, at.y);
  await settle(runtime);
  assert.equal(await runtime.page.evaluate(() => window.TesselRuntime.debug.overlay.state().hover), null);
  assert.notEqual(await runtime.page.evaluate(() => document.getElementById('overlay').style.cursor), 'grab');
});

test('a real press on the uniform box and a drag away from the node post picture coordinates', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const overlay = scaleOverlay();
  await enable(runtime, overlay);
  const rect = await picture(runtime);
  const box = overlay.handles[3].to;
  const start = pane(rect, box);
  await runtime.page.mouse.move(start.x, start.y);
  await settle(runtime);
  assert.equal(await runtime.page.evaluate(() => window.TesselRuntime.debug.overlay.state().hover), 'uniform');
  assert.equal(await runtime.page.evaluate(() => document.getElementById('overlay').style.cursor), 'grab');
  await runtime.page.mouse.down();
  const end = { u: 0.5 + 2 * (box.u - 0.5), v: 0.5 + 2 * (box.v - 0.5) };
  for (const step of [0.25, 0.5, 0.75, 1]) {
    const at = pane(rect, { u: box.u + (end.u - box.u) * step, v: box.v + (end.v - box.v) * step });
    await runtime.page.mouse.move(at.x, at.y);
  }
  await runtime.page.mouse.up();
  await settle(runtime);
  const messages = pointerMessages(runtime);
  assert.equal(messages[0].phase, 'down');
  assert.ok(Math.abs(messages[0].point.u - box.u) < 1e-6 && Math.abs(messages[0].point.v - box.v) < 1e-6, JSON.stringify(messages[0].point));
  const up = messages.at(-1);
  assert.equal(up.phase, 'up');
  assert.ok(Math.abs(up.point.u - end.u) < 1e-6 && Math.abs(up.point.v - end.v) < 1e-6, JSON.stringify(up.point));
  assert.ok(messages.every((m) => m.stamp.session === STAMP.session), 'every message echoes the stamp');
  assert.equal(messages.filter((m) => m.phase === 'cancel').length, 0);
});

test('Escape cancels a scale drag, and a resize keeps the same picture point', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const overlay = scaleOverlay();
  await enable(runtime, overlay);
  const rect = await picture(runtime);
  const start = pane(rect, overlay.handles[0].to);
  const next = pane(rect, { u: 0.7, v: 0.5 });
  runtime.messages.length = 0;
  await runtime.page.mouse.move(start.x, start.y);
  await runtime.page.mouse.down();
  await runtime.page.mouse.move(next.x, next.y);
  await settle(runtime);
  await runtime.page.keyboard.press('Escape');
  await runtime.page.mouse.move(start.x, start.y);
  await runtime.page.mouse.up();
  await settle(runtime);
  const phases = pointerMessages(runtime).map((m) => m.phase);
  assert.deepEqual(phases.filter((p) => p !== 'move'), ['down', 'cancel']);
  assert.equal(phases.at(-1), 'cancel', 'nothing after the cancel');
  // A resize in the middle of a drag does not end it: the same pointer still has its picture point.
  runtime.messages.length = 0;
  await runtime.page.mouse.move(start.x, start.y);
  await runtime.page.mouse.down();
  await runtime.page.setViewport({ width: 640, height: 480 });
  await settle(runtime);
  const resized = await picture(runtime);
  const after = pane(resized, { u: 0.7, v: 0.5 });
  await runtime.page.mouse.move(after.x, after.y);
  await runtime.page.mouse.up();
  await settle(runtime);
  const up = pointerMessages(runtime).at(-1);
  assert.equal(up.phase, 'up');
  assert.ok(Math.abs(up.point.u - 0.7) < 1e-6 && Math.abs(up.point.v - 0.5) < 1e-6, JSON.stringify(up.point));
});

test('switching from the scale boxes to the arrows replaces what is drawn and hit', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const overlay = scaleOverlay();
  await enable(runtime, overlay);
  const rect = await picture(runtime);
  const at = pane(rect, overlay.handles[3].to);
  await runtime.page.mouse.move(at.x, at.y);
  await settle(runtime);
  assert.equal(await runtime.page.evaluate(() => window.TesselRuntime.debug.overlay.state().hover), 'uniform');
  await runtime.send({ type: 'overlay.set', overlay: overlayJson({ stamp: { ...STAMP, revision: 2 } }), ack: true });
  const drawn = await runtime.waitFor((m) => m.type === 'overlay.drawn' && m.stamp?.revision === 2, 'overlay.drawn');
  assert.equal(drawn.tool, 'translate');
  assert.deepEqual(drawn.disabled, []);
  await runtime.page.mouse.move(at.x + 1, at.y + 1);
  await settle(runtime);
  assert.notEqual(await runtime.page.evaluate(() => window.TesselRuntime.debug.overlay.state().hover), 'uniform');
});
