// Pure rules of the gizmo overlay page (F055): the picture/pane mapping, the overlay payload check,
// the hover rule and the pointer policy (coalescing, Escape, focus). No DOM and no browser; the
// real pointer events on a real canvas are gizmo-overlay.test.mjs.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  PointerSession,
  containRect,
  drawOverlay,
  hoverHandle,
  insidePicture,
  parseOverlay,
  pictureInStep,
  refreshAfterFrame,
  toPane,
  toPicture,
  wantsOverlay,
} from '../src/overlay.js';

const overlayJson = (extra = {}) => ({
  version: 1,
  visible: true,
  reason: null,
  entity: 'scene/a/node/n',
  origin: { u: 0.5, v: 0.5 },
  handles: [
    { id: 'x', kind: 'axis', from: { u: 0.5, v: 0.5 }, to: { u: 0.6, v: 0.5 } },
    { id: 'y', kind: 'axis', from: { u: 0.5, v: 0.5 }, to: { u: 0.5, v: 0.35 } },
    { id: 'screen', kind: 'center', from: { u: 0.5, v: 0.5 }, to: { u: 0.5, v: 0.5 } },
  ],
  outlines: [],
  active: null,
  aspect: 16 / 9,
  metrics: { handle_length: 0.14, hit_radius: 0.025, center_radius: 0.03, unit: 'picture_height' },
  ...extra,
});

test('the picture is the contained rectangle and points map both ways', () => {
  // 16:9 picture in a 800x600 pane: full width, letterboxed vertically.
  const rect = containRect({ width: 800, height: 600 }, { width: 160, height: 90 });
  assert.deepEqual(rect, { x: 0, y: 75, width: 800, height: 450 });
  assert.deepEqual(toPicture(rect, 400, 300), { u: 0.5, v: 0.5 });
  assert.deepEqual(toPane(rect, { u: 0.25, v: 1 }), { x: 200, y: 525 });
  // A tall pane pillarboxes the other way.
  const tall = containRect({ width: 300, height: 900 }, { width: 160, height: 90 });
  assert.deepEqual(tall, { x: 0, y: 365.625, width: 300, height: 168.75 });
  assert.equal(insidePicture(toPicture(rect, 400, 20)), false, 'the bar is not the picture');
  assert.equal(insidePicture({ u: 0, v: 1 }), true);
  assert.equal(containRect({ width: 0, height: 10 }, { width: 1, height: 1 }), null);
});

test('an overlay payload is checked whole and never half accepted', () => {
  const ok = parseOverlay(overlayJson());
  assert.equal(ok.visible, true);
  assert.equal(ok.handles.length, 3);
  assert.equal(ok.handleRadius, 0.025);
  assert.throws(() => parseOverlay(overlayJson({ version: 2 })), /version 2 is not supported/);
  assert.throws(() => parseOverlay(overlayJson({ aspect: 0 })), /aspect/);
  assert.throws(() => parseOverlay(overlayJson({ origin: { u: 'x', v: 0 } })), /origin/);
  assert.throws(() => parseOverlay(overlayJson({ outlines: [{ entity: 'e', quad: [] }] })), /four points/);
  assert.throws(() => parseOverlay(null), /object/);
  // A hidden overlay is valid and has nothing to hit.
  const hidden = parseOverlay(overlayJson({ visible: false, origin: null, handles: [] }));
  assert.equal(hoverHandle(hidden, { u: 0.5, v: 0.5 }), null);
});

test('hover follows the core rule: the centre first, then the nearest axis within the radius', () => {
  const overlay = parseOverlay(overlayJson());
  assert.equal(hoverHandle(overlay, { u: 0.5, v: 0.5 }), 'screen');
  assert.equal(hoverHandle(overlay, { u: 0.56, v: 0.5 }), 'x');
  assert.equal(hoverHandle(overlay, { u: 0.5, v: 0.42 }), 'y');
  assert.equal(hoverHandle(overlay, { u: 0.56, v: 0.5 + 0.02 }), 'x', 'inside the radius, off the line');
  assert.equal(hoverHandle(overlay, { u: 0.56, v: 0.5 + 0.06 }), null, 'outside the radius');
  assert.equal(hoverHandle(overlay, { u: 0.9, v: 0.9 }), null);
});

const record = () => {
  const sent = [];
  return { sent, session: new PointerSession((message) => sent.push(message)) };
};

test('moves are coalesced: the last position wins and the release carries its own', () => {
  const { sent, session } = record();
  session.down({ u: 0.5, v: 0.5 }, true);
  session.move({ u: 0.51, v: 0.5 });
  session.move({ u: 0.52, v: 0.5 });
  session.move({ u: 0.53, v: 0.5 });
  assert.equal(sent.length, 1, 'nothing is posted until the frame flushes');
  assert.equal(session.flush(), true);
  assert.equal(session.flush(), false, 'nothing pending after a flush');
  session.move({ u: 0.54, v: 0.5 });
  session.up({ u: 0.55, v: 0.5 });
  assert.deepEqual(
    sent.map((m) => [m.phase, m.point?.u]),
    [['down', 0.5], ['move', 0.53], ['up', 0.55]],
    'the pending move is superseded by the release position',
  );
  assert.equal(sent[0].additive, true);
  assert.deepEqual(sent.map((m) => m.seq), [1, 2, 3], 'every message has the next sequence number');
  assert.equal(sent.every((m) => m.type === 'overlay.pointer'), true);
});

test('Escape and every other cancel end the press: nothing more is posted until the next press', () => {
  for (const reason of ['escape', 'blur', 'hidden', 'pointer_cancel']) {
    const { sent, session } = record();
    session.down({ u: 0.5, v: 0.5 });
    session.move({ u: 0.6, v: 0.5 });
    assert.equal(session.cancel(reason), true);
    assert.equal(session.cancel(reason), false, 'a second cancel is nothing');
    session.move({ u: 0.7, v: 0.5 });
    assert.equal(session.flush(), false);
    assert.equal(session.up({ u: 0.7, v: 0.5 }), false, 'the release of a cancelled press posts nothing');
    assert.deepEqual(sent.map((m) => m.phase), ['down', 'cancel']);
    assert.equal(sent[1].reason, reason);
    assert.equal(sent[1].point, undefined);
    // The button coming up readies the next press; so does a new press on its own.
    session.release();
    session.down({ u: 0.4, v: 0.4 });
    assert.equal(sent.at(-1).phase, 'down');
  }
});

test('a cancel with no press, a move with no press and a stray release post nothing', () => {
  const { sent, session } = record();
  assert.equal(session.cancel('escape'), false);
  session.move({ u: 0.5, v: 0.5 });
  assert.equal(session.flush(), false);
  assert.equal(session.up({ u: 0.5, v: 0.5 }), false);
  assert.deepEqual(sent, []);
});

test('drawing records the outline, the handles and the active one without touching the host', () => {
  const calls = [];
  const context = new Proxy({}, { get: (_, name) => (...args) => calls.push([String(name), ...args]), set: (_, name, value) => { calls.push(['set', String(name), value]); return true; } });
  const rect = { x: 0, y: 75, width: 800, height: 450 };
  const overlay = parseOverlay(
    overlayJson({
      active: { handle: 'x', gesture: 'g' },
      outlines: [{ entity: 'e', quad: [{ u: 0.4, v: 0.4 }, { u: 0.6, v: 0.4 }, { u: 0.6, v: 0.6 }, { u: 0.4, v: 0.6 }] }],
    }),
  );
  drawOverlay(context, rect, overlay, 'y');
  const strokes = calls.filter(([name]) => name === 'stroke').length;
  assert.ok(strokes >= 5, `outline + 2 axes (shadow and colour) + centre: got ${strokes}`);
  // The active handle is drawn in the active colour.
  assert.ok(calls.some(([name, key, value]) => name === 'set' && key === 'strokeStyle' && value === '#ffd60a'));
  // A hidden overlay draws only outlines; nothing at all for null.
  calls.length = 0;
  drawOverlay(context, rect, parseOverlay(overlayJson({ visible: false })), null);
  assert.equal(calls.filter(([name]) => name === 'arc').length, 0);
  drawOverlay(context, rect, null);
});

test('the picture is in step with an overlay when the shapes agree to within one pixel of the short side', () => {
  const landscape = parseOverlay(overlayJson({ aspect: 16 / 9 }));
  assert.equal(pictureInStep(null, { width: 0, height: 0 }), true, 'no overlay, nothing to disagree with');
  assert.equal(pictureInStep(landscape, { width: 160, height: 90 }), true);
  assert.equal(pictureInStep(landscape, { width: 960, height: 540 }), true, 'a scaled preview of the same output');
  // 1081x1921 previewed at 960 max: rounded to whole pixels, still the same shape.
  const odd = parseOverlay(overlayJson({ aspect: 1081 / 1921 }));
  assert.equal(pictureInStep(odd, { width: 540, height: 960 }), true);
  assert.equal(pictureInStep(landscape, { width: 90, height: 160 }), false, 'portrait canvas, landscape overlay');
  assert.equal(pictureInStep(landscape, { width: 160, height: 100 }), false);
  assert.equal(pictureInStep(landscape, { width: 0, height: 0 }), false, 'an empty canvas holds no picture');
});

test('a pointer session echoes the stamp of the overlay it pressed on, and drops it when the press ends', () => {
  const posted = [];
  let current = { revision: 1 };
  const session = new PointerSession((m) => posted.push(m), () => current);
  session.down({ u: 0.5, v: 0.5 });
  current = { revision: 2 };
  session.move({ u: 0.6, v: 0.5 });
  session.flush();
  session.up({ u: 0.6, v: 0.5 });
  assert.deepEqual(posted.map((m) => m.stamp), [{ revision: 1 }, { revision: 1 }, { revision: 1 }]);
  // A cancel outside a press (a drag begun by a script) answers the overlay that is current now.
  session.emit('cancel', { reason: 'escape' });
  assert.deepEqual(posted.at(-1).stamp, { revision: 2 });
  assert.equal(parseOverlay(overlayJson({ stamp: { revision: 9 } })).stamp.revision, 9);
  assert.equal(parseOverlay(overlayJson()).stamp, null, 'an overlay without a stamp parses as null');
});

test('after a frame: no overlay or an inactive one is never touched; a throwing refresh is reported, not thrown', () => {
  const calls = [];
  const overlay = (active, refresh = () => true) => ({
    active: () => active,
    refresh: () => {
      calls.push('refresh');
      return refresh();
    },
  });
  const errors = [];
  const onError = (error) => errors.push(error.message);

  assert.equal(refreshAfterFrame(null, onError), false);
  assert.equal(refreshAfterFrame(overlay(false), onError), false);
  assert.deepEqual(calls, [], 'inactive: refresh is not even called');

  assert.equal(refreshAfterFrame(overlay(true), onError), true);
  assert.deepEqual(calls, ['refresh']);

  assert.equal(refreshAfterFrame(overlay(true, () => { throw new Error('layout exploded'); }), onError), false);
  assert.deepEqual(errors, ['layout exploded'], 'the failure goes to the error channel');
});

test('only the desktop WebView page has an overlay surface; the worker page does not unless it asks', () => {
  const win = (extra) => ({ location: { search: '' }, ...extra });
  assert.equal(wantsOverlay(win({ ipc: { postMessage: () => {} } })), true, 'wry WebView');
  assert.equal(wantsOverlay(win({ __tesselPost: () => {} })), false, 'headless worker (puppeteer)');
  assert.equal(wantsOverlay(win({})), false, 'a page opened by hand');
  assert.equal(wantsOverlay(win({ __tesselPost: () => {}, location: { search: '?overlay=gizmo' } })), true, 'a test page that asks');
  assert.equal(wantsOverlay(win({ __tesselPost: () => {}, location: { search: '?overlay=other' } })), false);
  assert.equal(wantsOverlay(win({ ipc: { postMessage: () => {} }, __tesselPost: () => {} })), false, 'the worker transport wins over a stray ipc');
});

// ---- BL015.1: the rotate tool's rings ----------------------------------------------------------

const ASPECT = 16 / 9;

/** A closed ellipse of 64 segments (65 points, first = last) around the centre, radii in picture heights. */
const ellipse = (rx, ry, gap = null) =>
  Array.from({ length: 65 }, (_, index) => {
    if (gap !== null && index >= gap[0] && index < gap[1]) return null;
    const angle = (2 * Math.PI * (index % 64)) / 64;
    return { u: 0.5 + (rx * Math.cos(angle)) / ASPECT, v: 0.5 + ry * Math.sin(angle) };
  });

const ring = (id, points) => ({ id, kind: 'ring', from: { u: 0.5, v: 0.5 }, to: points.find((p) => p !== null) });

const ringsJson = (extra = {}) =>
  overlayJson({
    tool: 'rotate',
    space: 'local',
    aspect: ASPECT,
    handles: [
      { ...ring('x', ellipse(0.04, 0.1)), points: ellipse(0.04, 0.1) },
      { ...ring('y', ellipse(0.1, 0.04)), points: ellipse(0.1, 0.04) },
      { ...ring('z', ellipse(0.1, 0.1, [8, 16])), points: ellipse(0.1, 0.1, [8, 16]) },
    ],
    metrics: { hit_radius: 0.025, center_radius: 0.03, ring_radius: 0.1, unit: 'picture_height' },
    ...extra,
  });

test('a rotate overlay carries the tool, the space and the polygon of every ring, gaps included', () => {
  const overlay = parseOverlay(ringsJson());
  assert.equal(overlay.tool, 'rotate');
  assert.equal(overlay.space, 'local');
  assert.deepEqual(overlay.handles.map((h) => [h.id, h.kind, h.points.length]), [['x', 'ring', 65], ['y', 'ring', 65], ['z', 'ring', 65]]);
  assert.equal(overlay.handles[2].points[8], null, 'behind the camera stays a gap, not a point at the origin');
  // An overlay of the translate tool (and one of the first slice, which has no `tool`) is translate/world.
  assert.deepEqual([parseOverlay(overlayJson()).tool, parseOverlay(overlayJson()).space], ['translate', 'world']);
  // A ring without its polygon is never half drawn.
  const broken = ringsJson();
  delete broken.handles[0].points;
  assert.throws(() => parseOverlay(broken), /points/);
  const short = ringsJson();
  short.handles[1].points = [{ u: 0.5, v: 0.5 }];
  assert.throws(() => parseOverlay(short), /points/);
  const bad = ringsJson();
  bad.handles[2].points[3] = { u: 'a', v: 0 };
  assert.throws(() => parseOverlay(bad), /points\[3\]/);
});

test('hover on the rings is the nearest polygon within the radius; the centre is no handle', () => {
  const overlay = parseOverlay(ringsJson());
  const on = (points, index) => points[index];
  // A vertex of each ring is that ring (away from where two of them cross).
  assert.equal(hoverHandle(overlay, on(overlay.handles[2].points, 24)), 'z');
  assert.equal(hoverHandle(overlay, on(overlay.handles[2].points, 0)), 'y', 'where two rings cross the tie goes to the first axis, as in the core');
  assert.equal(hoverHandle(overlay, on(overlay.handles[0].points, 16)), 'x');
  assert.equal(hoverHandle(overlay, on(overlay.handles[1].points, 0)), 'y', 'the y ring is the wider ellipse; its far vertex is on it');
  // Inside the radius off the line, and outside it.
  const near = overlay.handles[2].points[24];
  assert.equal(hoverHandle(overlay, { u: near.u, v: near.v + 0.01 }), 'z');
  assert.equal(hoverHandle(overlay, { u: near.u, v: near.v + 0.06 }), null);
  // The centre is not a handle of the rotate tool (translate's `screen`): nothing is under it.
  assert.equal(hoverHandle(overlay, { u: 0.5, v: 0.5 }), null);
  assert.equal(hoverHandle(parseOverlay(overlayJson()), { u: 0.5, v: 0.5 }), 'screen', 'translate is unchanged');
  // The gap of a ring (behind the camera) is not hit, though the chord across it would be near.
  const gapMiddle = (() => {
    const a = overlay.handles[2].points[7];
    const b = overlay.handles[2].points[16];
    return { u: (a.u + b.u) / 2, v: (a.v + b.v) / 2 };
  })();
  assert.notEqual(hoverHandle(overlay, gapMiddle), 'z');
  // Outside everything.
  assert.equal(hoverHandle(overlay, { u: 0.95, v: 0.95 }), null);
});

test('drawing a ring strokes its polygon in the axis colour and opens one subpath per gap', () => {
  const calls = [];
  const context = new Proxy({}, { get: (_, name) => (...args) => calls.push([String(name), ...args]), set: (_, name, value) => { calls.push(['set', String(name), value]); return true; } });
  const rect = { x: 0, y: 75, width: 800, height: 450 };
  drawOverlay(context, rect, parseOverlay(ringsJson({ handles: [{ ...ring('z', ellipse(0.1, 0.1, [8, 16])), points: ellipse(0.1, 0.1, [8, 16]) }] })), null);
  // Shadow pass and colour pass, two subpaths each (before and after the gap).
  assert.equal(calls.filter(([name]) => name === 'moveTo').length, 4);
  assert.equal(calls.filter(([name]) => name === 'stroke').length, 2);
  assert.ok(calls.some(([name, key, value]) => name === 'set' && key === 'strokeStyle' && value === '#3e63dd'), 'z is blue');
  assert.equal(calls.filter(([name]) => name === 'arc').length, 0, 'no centre disc with the rotate tool');
  // An active ring is drawn in the active colour.
  calls.length = 0;
  drawOverlay(context, rect, parseOverlay(ringsJson({ active: { handle: 'z', gesture: 'g' } })), null);
  assert.ok(calls.some(([name, key, value]) => name === 'set' && key === 'strokeStyle' && value === '#ffd60a'));
});
