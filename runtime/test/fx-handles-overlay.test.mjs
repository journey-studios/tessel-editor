// The vignette handles on the overlay page (F061, G4.1): the payload check, the hover rule, what is
// drawn, and, on a real page with real mouse events, what a press, a drag, Escape, a lost capture
// and a resize post. The geometry (where a handle is, what value a position means) is the core's;
// this proves the page half of the seam: it draws what the core lists, hits what the core offers,
// and posts picture coordinates, nothing else.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { FX_PRIORITY, drawOverlay, hoverHandle, parseOverlay, toPane } from '../src/overlay.js';
import { openRuntime, skipUnavailable } from './support.mjs';

const ASPECT = 160 / 90;
const STAMP = { scene: 'a', session: 'session-1', revision: 1, frame: 20, fx_clip: 'v1' };

/** The core's `fx` block for a vignette at (0.3, 0.5): radius at 0.5, softness at 0.7. */
const fxBlock = (extra = {}) => ({
  clip: 'v1',
  effect: 'vignette',
  frame: 20,
  frame_in_clip: 10,
  output: { width: 320, height: 180 },
  visible: true,
  reason: null,
  handles: [
    { id: 'center', param: 'center', kind: 'center', at: { u: 0.3, v: 0.5 } },
    { id: 'radius', param: 'radius', kind: 'edge', at: { u: 0.5, v: 0.5 } },
    { id: 'softness', param: 'softness', kind: 'edge', at: { u: 0.7, v: 0.5 } },
  ],
  disabled_handles: [],
  ellipses: [
    { id: 'radius', center: { u: 0.3, v: 0.5 }, rx: 0.2, ry: 0.35 },
    { id: 'softness', center: { u: 0.3, v: 0.5 }, rx: 0.4, ry: 0.7 },
  ],
  active: null,
  metrics: { hit_radius: 0.025, unit: 'picture_height' },
  ...extra,
});

const overlayJson = (fx = fxBlock(), extra = {}) => ({
  version: 1,
  visible: false,
  reason: 'fx_target',
  target: 'fx',
  entity: null,
  origin: null,
  handles: [],
  outlines: [],
  active: null,
  aspect: ASPECT,
  stamp: STAMP,
  metrics: { handle_length: 0.14, hit_radius: 0.025, center_radius: 0.03, unit: 'picture_height' },
  fx,
  ...extra,
});

/** A recording 2D context: what a draw asked for, in order. */
const recorder = () => {
  const calls = [];
  const handler = {
    get: (_, name) => {
      if (name === 'calls') return calls;
      return (...args) => {
        calls.push([String(name), ...args]);
      };
    },
    set: (_, name, value) => {
      calls.push(['set', String(name), value]);
      return true;
    },
  };
  return new Proxy({}, handler);
};

const RECT = { x: 0, y: 0, width: 800, height: 450 };

test('the fx block is checked whole and an overlay without it is what it was', () => {
  const parsed = parseOverlay(overlayJson());
  assert.equal(parsed.fx.clip, 'v1');
  assert.deepEqual(parsed.fx.handles.map((h) => h.id), ['center', 'radius', 'softness']);
  assert.equal(parsed.fx.handles[0].kind, 'center');
  assert.equal(parsed.fx.ellipses.length, 2);
  assert.equal(parsed.fx.hitRadius, 0.025);
  assert.equal(parseOverlay(overlayJson(undefined, { fx: undefined })).fx, null, 'no fx block, no fx');
  assert.equal(parseOverlay({ ...overlayJson(), fx: null }).fx, null);
  assert.throws(() => parseOverlay(overlayJson(fxBlock({ handles: [{ id: 'radius', at: { u: 'x', v: 0 } }] }))), /fx.handles\[0\].at/);
  assert.throws(() => parseOverlay(overlayJson(fxBlock({ handles: [{ at: { u: 0, v: 0 } }] }))), /needs an id/);
  assert.throws(() => parseOverlay(overlayJson(fxBlock({ ellipses: [{ id: 'radius', center: { u: 0, v: 0 }, rx: -1, ry: 1 }] }))), /non-negative/);
  assert.throws(() => parseOverlay(overlayJson('nope')), /fx must be an object/);
});

test('a handle off the picture is listed, not drawn, and one with keys is drawn dimmed', () => {
  const fx = fxBlock({
    handles: [{ id: 'center', param: 'center', kind: 'center', at: { u: 0.3, v: 0.5 } }],
    disabled_handles: [
      { id: 'radius', reason: 'animated_param', at: { u: 0.5, v: 0.5 } },
      { id: 'softness', reason: 'handle_off_picture', at: { u: 1.4, v: 0.5 } },
    ],
  });
  const parsed = parseOverlay(overlayJson(fx));
  const context = recorder();
  drawOverlay(context, RECT, parsed, null);
  const discs = context.calls.filter(([name]) => name === 'arc');
  // The centre (ring + dot, with a halo) and the dimmed radius handle (disc, with a halo).
  const at = (u) => toPane(RECT, { u, v: 0.5 }).x;
  const xs = new Set(discs.map(([, x]) => Math.round(x)));
  assert.ok(xs.has(Math.round(at(0.3))), 'the centre is drawn');
  assert.ok(xs.has(Math.round(at(0.5))), 'the handle with keys is drawn (dimmed)');
  assert.ok(![...xs].some((x) => x > RECT.width), 'the off-picture handle is not drawn');
  const fills = context.calls.filter(([name, key]) => name === 'set' && key === 'fillStyle').map(([, , value]) => value);
  assert.ok(fills.includes('rgba(245, 245, 245, 0.45)'), 'the dimmed colour was used');
  assert.deepEqual(parsed.fx.disabled.map((d) => [d.id, d.reason]), [['radius', 'animated_param'], ['softness', 'handle_off_picture']]);
});

test('the two ellipses are drawn inside the picture, the second dashed, and no node gizmo', () => {
  const context = recorder();
  drawOverlay(context, RECT, parseOverlay(overlayJson()), null);
  const ellipses = context.calls.filter(([name]) => name === 'ellipse');
  assert.equal(ellipses.length, 4, 'each ellipse is stroked twice: halo and line');
  const [, cx, cy, rx, ry] = ellipses[0];
  assert.deepEqual([cx, cy, rx, ry].map(Math.round), [240, 225, 160, 158], 'in pane pixels of the picture');
  const dashes = context.calls.filter(([name]) => name === 'setLineDash').map(([, dash]) => dash.length);
  assert.ok(dashes.includes(2), 'the softness ellipse is dashed');
  assert.ok(context.calls.some(([name]) => name === 'clip'), 'clipped to the picture');
  assert.ok(!context.calls.some(([name]) => name === 'lineTo'), 'no arrow of the node gizmo');
  // A clip that does not cover the frame draws nothing.
  const quiet = recorder();
  drawOverlay(quiet, RECT, parseOverlay(overlayJson(fxBlock({ visible: false, reason: 'clip_not_in_frame', handles: [], ellipses: [] }))), null);
  assert.deepEqual(quiet.calls, []);
});

test('hover follows the core: the nearest offered handle, the outer one on a tie, nothing for a dimmed one', () => {
  const parsed = parseOverlay(overlayJson());
  assert.equal(hoverHandle(parsed, { u: 0.5, v: 0.5 }), 'radius');
  assert.equal(hoverHandle(parsed, { u: 0.7 + 0.01 / ASPECT, v: 0.5 }), 'softness');
  assert.equal(hoverHandle(parsed, { u: 0.3, v: 0.5 }), 'center');
  assert.equal(hoverHandle(parsed, { u: 0.6, v: 0.5 }), null, 'between handles');
  assert.equal(hoverHandle(parsed, { u: 0.5, v: 0.5 + 0.03 }), null, 'past the hit radius (picture heights)');
  const stacked = parseOverlay(overlayJson(fxBlock({
    handles: [
      { id: 'center', param: 'center', kind: 'center', at: { u: 0.3, v: 0.5 } },
      { id: 'radius', param: 'radius', kind: 'edge', at: { u: 0.3, v: 0.5 } },
      { id: 'softness', param: 'softness', kind: 'edge', at: { u: 0.3, v: 0.5 } },
    ],
  })));
  assert.equal(hoverHandle(stacked, { u: 0.3, v: 0.5 }), 'softness', 'a tie goes to the outer handle');
  assert.deepEqual(FX_PRIORITY, ['softness', 'radius', 'center']);
  const dimmed = parseOverlay(overlayJson(fxBlock({
    handles: [],
    disabled_handles: [{ id: 'radius', reason: 'animated_param', at: { u: 0.5, v: 0.5 } }],
  })));
  assert.equal(hoverHandle(dimmed, { u: 0.5, v: 0.5 }), null, 'a handle with keys does not light up');
  // Whatever the core listed, a block that is not visible offers nothing to the pointer.
  const hidden = parseOverlay(overlayJson(fxBlock({ visible: false, reason: 'clip_not_in_frame' })));
  assert.equal(hoverHandle(hidden, { u: 0.5, v: 0.5 }), null);
});

// ---- the real page ------------------------------------------------------------------------

const open = async (t) => {
  if (skipUnavailable(t, assert)) return null;
  const runtime = await openRuntime({ overlay: true });
  t.after(() => runtime.close());
  await runtime.hello();
  return runtime;
};

const state = (runtime) => runtime.page.evaluate(() => {
  const s = window.TesselRuntime.debug.overlay.state();
  return s ? { ...s, overlay: undefined } : null;
});
const picture = async (runtime) => (await state(runtime)).rect;
const pane = (rect, point) => ({ x: rect.x + point.u * rect.width, y: rect.y + point.v * rect.height });
const pointerMessages = (runtime) => runtime.messages.filter((m) => m.type === 'overlay.pointer');
const settle = (runtime) => runtime.page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));

const enable = async (runtime, overlay = overlayJson()) => {
  await runtime.send({ type: 'overlay.enable', enabled: true });
  await runtime.send({ type: 'overlay.set', overlay, ack: true });
  return runtime.waitFor((m) => m.type === 'overlay.drawn', 'overlay.drawn');
};

test('the page paints the handles and the ellipses and the pixels say so', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const drawn = await enable(runtime);
  assert.equal(drawn.target, 'fx');
  assert.equal(drawn.fx.clip, 'v1');
  assert.deepEqual(drawn.fx.handles, ['center', 'radius', 'softness']);
  assert.equal(drawn.fx.ellipses, 2);
  assert.deepEqual(drawn.probes.map((probe) => probe.id), ['center', 'radius', 'softness']);
  for (const probe of drawn.probes) {
    assert.ok(probe.rgba[3] > 200, `${probe.id} is really painted: ${probe.rgba}`);
  }
  // A pixel on the radius ellipse (its top, at (0.3, 0.5 - 0.35)) is painted; one far from every shape is not.
  const rect = await picture(runtime);
  const top = pane(rect, { u: 0.3, v: 0.5 - 0.35 });
  const ratio = await runtime.page.evaluate(() => window.devicePixelRatio || 1);
  const onEllipse = await runtime.page.evaluate((x, y, k) => {
    // A 7x7 window around the top of the ellipse: the stroke is a line about a pixel and a half wide.
    const data = document.getElementById('overlay').getContext('2d').getImageData(Math.round(x * k) - 3, Math.round(y * k) - 3, 7, 7).data;
    return Math.max(...Array.from({ length: 49 }, (_, i) => data[i * 4 + 3]));
  }, top.x, top.y, ratio);
  assert.ok(onEllipse > 0, `the ellipse is on the canvas: max alpha ${onEllipse}`);
  const far = await runtime.page.evaluate(() => [...document.getElementById('overlay').getContext('2d').getImageData(3, 3, 1, 1).data]);
  assert.deepEqual(far, [0, 0, 0, 0]);
  // Nothing of the node gizmo: no outline, no arrow.
  assert.equal(drawn.outlines, 0);
  assert.equal(drawn.fx.active, null, 'no handle is active at rest');
  // The overlay of a drag in progress names the active handle, and the page says it drew it.
  await runtime.send({ type: 'overlay.set', overlay: overlayJson(fxBlock({ active: 'radius' })), ack: true });
  const dragging = await runtime.waitFor((m) => m.type === 'overlay.drawn' && m.fx?.active === 'radius', 'the active handle drawn');
  assert.equal(dragging.target, 'fx');
});

test('a press on a handle, a drag and the release post picture coordinates under the clip stamp', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await enable(runtime);
  const rect = await picture(runtime);
  const start = pane(rect, { u: 0.5, v: 0.5 });
  const end = pane(rect, { u: 0.62, v: 0.4 });
  await runtime.page.mouse.move(start.x, start.y);
  await settle(runtime);
  assert.equal(await runtime.page.evaluate(() => document.getElementById('overlay').style.cursor), 'grab', 'the handle lights up');
  await runtime.page.mouse.down();
  for (let step = 1; step <= 8; step += 1) {
    const f = step / 8;
    await runtime.page.mouse.move(start.x + (end.x - start.x) * f, start.y + (end.y - start.y) * f);
  }
  await runtime.page.mouse.up();
  await settle(runtime);
  const messages = pointerMessages(runtime);
  assert.equal(messages[0].phase, 'down');
  assert.ok(Math.abs(messages[0].point.u - 0.5) < 1e-9 && Math.abs(messages[0].point.v - 0.5) < 1e-9);
  const up = messages.at(-1);
  assert.equal(up.phase, 'up');
  assert.ok(Math.abs(up.point.u - 0.62) < 1e-6 && Math.abs(up.point.v - 0.4) < 1e-6, JSON.stringify(up.point));
  for (const message of messages) assert.deepEqual(message.stamp, STAMP, `${message.phase} answers the overlay of the clip`);
  assert.equal(messages.filter((m) => m.phase === 'cancel').length, 0);
});

test('Escape, a lost capture and a resize behave as for the node gizmo', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  await enable(runtime);
  const rect = await picture(runtime);
  const start = pane(rect, { u: 0.5, v: 0.5 });

  await runtime.page.mouse.move(start.x, start.y);
  await runtime.page.mouse.down();
  await runtime.page.mouse.move(start.x + 20, start.y);
  await settle(runtime);
  await runtime.page.keyboard.press('Escape');
  await runtime.page.mouse.move(start.x + 40, start.y);
  await runtime.page.mouse.up();
  await settle(runtime);
  assert.deepEqual(pointerMessages(runtime).map((m) => m.phase).filter((p) => p !== 'move'), ['down', 'cancel']);
  assert.equal(pointerMessages(runtime).find((m) => m.phase === 'cancel').reason, 'escape');

  runtime.messages.length = 0;
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
  assert.deepEqual(pointerMessages(runtime).map((m) => [m.phase, m.reason]), [['down', undefined], ['cancel', 'pointer_cancel']]);

  runtime.messages.length = 0;
  await runtime.page.mouse.move(start.x, start.y);
  await runtime.page.mouse.down();
  await runtime.page.setViewport({ width: 500, height: 700 });
  await settle(runtime);
  const after = await picture(runtime);
  assert.notDeepEqual(after, rect, 'the picture was re-fitted');
  const target = pane(after, { u: 0.6, v: 0.5 });
  await runtime.page.mouse.move(target.x, target.y);
  await runtime.page.mouse.up();
  await settle(runtime);
  const messages = pointerMessages(runtime);
  assert.deepEqual(messages.map((m) => m.phase).filter((p) => p !== 'move'), ['down', 'up'], 'no cancel from a resize');
  const up = messages.at(-1);
  assert.ok(Math.abs(up.point.u - 0.6) < 1e-6 && Math.abs(up.point.v - 0.5) < 1e-6, JSON.stringify(up.point));
});

test('a handle with keys is dimmed, does not light up, and the page still posts the press for the core to refuse', async (t) => {
  const runtime = await open(t);
  if (!runtime) return;
  const fx = fxBlock({
    handles: [
      { id: 'center', param: 'center', kind: 'center', at: { u: 0.3, v: 0.5 } },
      { id: 'softness', param: 'softness', kind: 'edge', at: { u: 0.7, v: 0.5 } },
    ],
    disabled_handles: [{ id: 'radius', reason: 'animated_param', at: { u: 0.5, v: 0.5 } }],
  });
  const drawn = await enable(runtime, overlayJson(fx));
  assert.deepEqual(drawn.disabled, [{ id: 'radius', reason: 'animated_param' }]);
  assert.deepEqual(drawn.probes.map((probe) => probe.id), ['center', 'softness'], 'only the offered handles are probed');
  const rect = await picture(runtime);
  const spot = pane(rect, { u: 0.5, v: 0.5 });
  await runtime.page.mouse.move(spot.x, spot.y);
  await settle(runtime);
  assert.notEqual(await runtime.page.evaluate(() => document.getElementById('overlay').style.cursor), 'grab');
  // The decision is the core's: the page sends the press, the core answers `animated_param`.
  await runtime.page.mouse.down();
  await runtime.page.mouse.up();
  await settle(runtime);
  assert.deepEqual(pointerMessages(runtime).map((m) => m.phase), ['down', 'up']);
});
