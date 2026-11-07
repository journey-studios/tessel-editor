// The gizmo overlay of the directed-scene monitor (F055, BL015/BL042): drawn and hit on a 2D canvas
// that sits over the viewport canvas INSIDE the page, so it composes with the picture and can never
// be an element of the host UI laid over a native WebView.
//
// WHY this module decides nothing about the scene: the core owns the gizmo (which handle, which
// entity, what the new position is). The page does three things only:
//   1. draws what `overlay.set` lists, in picture coordinates (u, v in 0..1 over the *picture*,
//      not over the pane around it);
//   2. turns pointer and keyboard events into `overlay.pointer` messages in those same
//      coordinates, with the explicit focus/Escape/resize policy below;
//   3. highlights the handle under the pointer (presentation only: the press is decided by the
//      core, which re-tests it).
//
// Policy, one place (docs/features/scene-gizmo.md):
//   * a press must land on the picture; the pointer is then captured, so the release arrives even
//     outside the pane;
//   * moves are coalesced to one per animation frame (the last wins);
//   * Escape cancels the drag and the rest of the press is ignored (no second drag from a button
//     still down);
//   * the window or page losing focus, the page becoming hidden, `pointercancel` and a lost
//     capture CANCEL (a release may never arrive; committing a position nobody confirmed is worse
//     than discarding it);
//   * a resize redraws from the same picture coordinates and never ends a drag.
//
// The pure functions are exported for `node --test`; `attachOverlay` is the DOM part.

import { BRIDGE_VERSION } from './protocol.js';

export const OVERLAY_VERSION = 1;

/** Handle colors by id; the active one is drawn in `ACTIVE`. Presentation, not data. */
export const PALETTE = {
  x: '#e5484d',
  y: '#30a46c',
  z: '#3e63dd',
  screen: '#f5f5f5',
  active: '#ffd60a',
  outline: '#4cc2ff',
  shadow: 'rgba(0, 0, 0, 0.55)',
};

const isObject = (value) => typeof value === 'object' && value !== null && !Array.isArray(value);
const finite = (value) => typeof value === 'number' && Number.isFinite(value);

/** The rectangle the picture takes inside a `box` ("contain", as the canvas's CSS does). */
export const containRect = (box, source) => {
  if (!(box.width > 0 && box.height > 0 && source.width > 0 && source.height > 0)) return null;
  const fit = Math.min(box.width / source.width, box.height / source.height);
  const width = source.width * fit;
  const height = source.height * fit;
  return { x: (box.width - width) / 2, y: (box.height - height) / 2, width, height };
};

/**
 * Whether the canvas holds the picture the overlay was computed for. The overlay says the aspect
 * of the output it describes; the canvas carries the picture last drawn. Right after the output
 * changes shape (landscape to portrait) the pane still shows the old picture until the next
 * frame, and a handle mapped over it would sit on nothing. The preview size is the output size
 * scaled and rounded to whole pixels, so the aspects agree to within one pixel of the short side.
 */
export const pictureInStep = (overlay, source) => {
  if (!overlay) return true;
  if (!(source.width > 0 && source.height > 0)) return false;
  const tolerance = overlay.aspect / Math.min(source.width, source.height);
  return Math.abs(source.width / source.height - overlay.aspect) <= tolerance;
};

/** Pane position (CSS px, relative to the box) to picture position. */
export const toPicture = (rect, x, y) => ({ u: (x - rect.x) / rect.width, v: (y - rect.y) / rect.height });

export const insidePicture = ({ u, v }) => u >= 0 && u <= 1 && v >= 0 && v <= 1;

/** Picture position to pane position. */
export const toPane = (rect, point) => ({ x: rect.x + point.u * rect.width, y: rect.y + point.v * rect.height });

const distanceToSegment = (p, a, b) => {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const lengthSquared = dx * dx + dy * dy;
  const t = lengthSquared < 1e-12 ? 0 : Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / lengthSquared));
  return Math.hypot(p.x - (a.x + t * dx), p.y - (a.y + t * dy));
};

/**
 * Checks an `overlay.set` payload (the core's `Overlay::to_json`, version 1) and returns it, or
 * throws a plain Error naming the field. A malformed overlay is never half drawn.
 */
export const parseOverlay = (raw) => {
  if (!isObject(raw)) throw new Error('overlay must be an object');
  if (raw.version !== OVERLAY_VERSION) throw new Error(`overlay version ${JSON.stringify(raw.version)} is not supported (this page draws ${OVERLAY_VERSION})`);
  const point = (value, where) => {
    if (!isObject(value) || !finite(value.u) || !finite(value.v)) throw new Error(`${where} must be {u, v}`);
    return { u: value.u, v: value.v };
  };
  if (!finite(raw.aspect) || raw.aspect <= 0) throw new Error('aspect must be a positive number');
  const metrics = isObject(raw.metrics) ? raw.metrics : {};
  const handles = (Array.isArray(raw.handles) ? raw.handles : []).map((handle, index) => {
    if (!isObject(handle) || typeof handle.id !== 'string') throw new Error(`handles[${index}] needs an id`);
    return { id: handle.id, kind: handle.kind === 'center' ? 'center' : 'axis', from: point(handle.from, `handles[${index}].from`), to: point(handle.to, `handles[${index}].to`) };
  });
  const outlines = (Array.isArray(raw.outlines) ? raw.outlines : []).map((outline, index) => {
    const quad = Array.isArray(outline?.quad) ? outline.quad.map((p, i) => point(p, `outlines[${index}].quad[${i}]`)) : [];
    if (quad.length !== 4) throw new Error(`outlines[${index}].quad needs four points`);
    return { entity: String(outline.entity ?? ''), quad };
  });
  return {
    visible: raw.visible === true,
    entity: typeof raw.entity === 'string' ? raw.entity : null,
    origin: raw.origin ? point(raw.origin, 'origin') : null,
    handles,
    outlines,
    active: isObject(raw.active) && typeof raw.active.handle === 'string' ? { handle: raw.active.handle, gesture: raw.active.gesture ?? null } : null,
    aspect: raw.aspect,
    // Opaque to the page: echoed in every pointer message and ack so the host can tell an answer
    // to this overlay from one to an older overlay (reloaded page, replaced project, other frame).
    stamp: raw.stamp ?? null,
    handleRadius: finite(metrics.hit_radius) ? metrics.hit_radius : 0.025,
    centerRadius: finite(metrics.center_radius) ? metrics.center_radius : 0.03,
  };
};

/**
 * The handle under `point` for the cursor and the highlight: the same rule the core applies
 * (centre first, then the nearest axis), in picture heights. The press itself is the core's.
 */
export const hoverHandle = (overlay, point) => {
  if (!overlay?.visible || !overlay.origin) return null;
  const toHeights = (p) => ({ x: p.u * overlay.aspect, y: p.v });
  const here = toHeights(point);
  if (Math.hypot(here.x - overlay.origin.u * overlay.aspect, here.y - overlay.origin.v) <= overlay.centerRadius) return 'screen';
  let best = null;
  for (const handle of overlay.handles.filter((candidate) => candidate.kind === 'axis')) {
    const distance = distanceToSegment(here, toHeights(handle.from), toHeights(handle.to));
    if (distance <= overlay.handleRadius && (best === null || distance < best.distance)) best = { distance, id: handle.id };
  }
  return best?.id ?? null;
};

/**
 * The messages a pointer interaction posts, and the rules about when. Pure: no DOM, no timers
 * (`flush` is called by the caller once per animation frame), so the policy is testable.
 */
export class PointerSession {
  constructor(post, stampOf = () => null) {
    this.post = post;
    this.stampOf = stampOf;
    // The stamp of the overlay that was under the pointer at the press: the whole press answers it.
    this.stamp = null;
    this.state = 'idle'; // idle | pressed | ended (cancelled while the button is still down)
    this.pending = null;
    this.seq = 0;
  }

  get pressed() {
    return this.state === 'pressed';
  }

  emit(phase, extra = {}) {
    this.seq += 1;
    this.post({ type: 'overlay.pointer', phase, seq: this.seq, stamp: this.stamp ?? this.stampOf(), ...extra });
  }

  /** `point` must already be on the picture; the caller checks (`insidePicture`). */
  down(point, additive = false) {
    this.pending = null;
    this.state = 'pressed';
    this.stamp = this.stampOf();
    this.emit('down', { point, additive });
  }

  move(point) {
    if (this.state === 'pressed') this.pending = point;
  }

  /** Posts the coalesced move, if any. */
  flush() {
    if (this.state !== 'pressed' || this.pending === null) return false;
    const point = this.pending;
    this.pending = null;
    this.emit('move', { point });
    return true;
  }

  up(point) {
    if (this.state !== 'pressed') {
      this.state = 'idle';
      return false;
    }
    this.pending = null;
    this.state = 'idle';
    this.emit('up', { point });
    this.stamp = null;
    return true;
  }

  /** Escape, blur, hidden, pointercancel, lost capture. After it the press is dead until the next one. */
  cancel(reason) {
    if (this.state !== 'pressed') return false;
    this.pending = null;
    this.state = 'ended';
    this.emit('cancel', { reason });
    this.stamp = null;
    return true;
  }

  /** The button came up (or the pointer left) after a cancel: the next press starts clean. */
  release() {
    if (this.state === 'ended') this.state = 'idle';
  }
}

/**
 * What the runtime does with the overlay after a frame was sent and announced (F055, M1 of the
 * PR #25 review). The overlay is OFF for every frame that is not a directed scene on the desktop
 * monitor, and a frame is never reported as failed because of it:
 *   * no overlay, or an inactive one: nothing is read, cleared or reported (`false`);
 *   * the refresh throws: the host hears an `overlay_failed` error message, the frame stands.
 * Returns whether the overlay was refreshed.
 */
export const refreshAfterFrame = (overlay, onError) => {
  if (!overlay || !overlay.active()) return false;
  try {
    return overlay.refresh() !== false;
  } catch (error) {
    onError(error);
    return false;
  }
};

/**
 * Whether this page is the desktop viewport: the one place that draws the gizmo. The WebView host
 * (wry IPC) is, and a test page may ask for it with `?overlay=gizmo`. The headless Node worker
 * serves the same bundle through puppeteer (`__tesselPost`) and never has an overlay surface: it
 * would pay for a canvas nobody can see and announce a feature nobody can use.
 */
export const wantsOverlay = (win = window) => {
  if (typeof win.__tesselPost === 'function') return new URLSearchParams(win.location?.search ?? '').get('overlay') === 'gizmo';
  return Boolean(win.ipc && typeof win.ipc.postMessage === 'function');
};

const drawArrow = (context, from, to, color, width) => {
  context.lineCap = 'round';
  context.strokeStyle = PALETTE.shadow;
  context.lineWidth = width + 2;
  context.beginPath();
  context.moveTo(from.x, from.y);
  context.lineTo(to.x, to.y);
  context.stroke();
  context.strokeStyle = color;
  context.fillStyle = color;
  context.lineWidth = width;
  context.beginPath();
  context.moveTo(from.x, from.y);
  context.lineTo(to.x, to.y);
  context.stroke();
  const angle = Math.atan2(to.y - from.y, to.x - from.x);
  const head = 9;
  context.beginPath();
  context.moveTo(to.x, to.y);
  context.lineTo(to.x - head * Math.cos(angle - 0.45), to.y - head * Math.sin(angle - 0.45));
  context.lineTo(to.x - head * Math.cos(angle + 0.45), to.y - head * Math.sin(angle + 0.45));
  context.closePath();
  context.fill();
};

/** Draws `overlay` into `context` over `rect` (the picture, in CSS px). `hover` is a handle id. */
export const drawOverlay = (context, rect, overlay, hover = null) => {
  if (!overlay || !rect) return;
  for (const outline of overlay.outlines) {
    const corners = outline.quad.map((point) => toPane(rect, point));
    context.strokeStyle = PALETTE.shadow;
    context.lineWidth = 3;
    context.setLineDash([]);
    context.beginPath();
    corners.forEach((corner, index) => (index === 0 ? context.moveTo(corner.x, corner.y) : context.lineTo(corner.x, corner.y)));
    context.closePath();
    context.stroke();
    context.strokeStyle = PALETTE.outline;
    context.lineWidth = 1.5;
    context.setLineDash([6, 4]);
    context.stroke();
    context.setLineDash([]);
  }
  if (!overlay.visible) return;
  const heightPx = rect.height;
  for (const handle of overlay.handles) {
    const active = overlay.active?.handle === handle.id;
    const lit = active || hover === handle.id;
    const color = active ? PALETTE.active : (lit ? PALETTE.active : PALETTE[handle.id] ?? PALETTE.screen);
    const width = lit ? 3.5 : 2.5;
    const from = toPane(rect, handle.from);
    if (handle.kind === 'center') {
      context.fillStyle = PALETTE.shadow;
      context.beginPath();
      context.arc(from.x, from.y, overlay.centerRadius * heightPx + 1.5, 0, Math.PI * 2);
      context.fill();
      context.strokeStyle = color;
      context.lineWidth = width;
      context.beginPath();
      context.arc(from.x, from.y, overlay.centerRadius * heightPx, 0, Math.PI * 2);
      context.stroke();
      continue;
    }
    drawArrow(context, from, toPane(rect, handle.to), color, width);
  }
};

/**
 * Wires the overlay canvas to the page. Returns `{ receive, destroy, state }`; `receive` takes the
 * host's `overlay.*` messages. `post` is the page's transport (bridge v1 envelope added here).
 */
export const attachOverlay = ({ canvas, post, win = window, doc = document }) => {
  const send = (message) => post({ v: BRIDGE_VERSION, ...message });
  const surface = doc.createElement('canvas');
  surface.id = 'overlay';
  surface.tabIndex = 0;
  surface.setAttribute('aria-label', 'Gizmo overlay');
  surface.style.cssText =
    'position:fixed;left:0;top:0;width:100vw;height:100vh;display:block;pointer-events:none;touch-action:none;outline:none;';
  canvas.insertAdjacentElement('afterend', surface);
  const context = surface.getContext('2d', { willReadFrequently: true });

  let overlay = null;
  let enabled = false;
  let hover = null;
  let capturedPointer = null;
  let frameRequest = 0;
  let layoutKey = '';
  let draws = 0;
  const session = new PointerSession(send, () => overlay?.stamp ?? null);

  const box = () => ({ width: surface.clientWidth, height: surface.clientHeight });
  const rect = () => containRect(box(), { width: canvas.width, height: canvas.height });
  // False while the canvas still holds a picture of another shape than the overlay describes:
  // nothing is drawn or accepted until the frame for the overlay arrives (`refresh`).
  const inStep = () => pictureInStep(overlay, { width: canvas.width, height: canvas.height });

  const resizeSurface = () => {
    const ratio = win.devicePixelRatio || 1;
    const { width, height } = box();
    const pixelWidth = Math.max(1, Math.round(width * ratio));
    const pixelHeight = Math.max(1, Math.round(height * ratio));
    if (surface.width !== pixelWidth || surface.height !== pixelHeight) {
      surface.width = pixelWidth;
      surface.height = pixelHeight;
    }
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
  };

  const redraw = () => {
    resizeSurface();
    const { width, height } = box();
    context.clearRect(0, 0, width, height);
    if (enabled && overlay && inStep()) drawOverlay(context, rect(), overlay, hover);
  };

  /** Tells the host where the picture is (informative: the core never needs pixels). */
  const reportLayout = () => {
    const picture = rect();
    const key = JSON.stringify([box(), picture, canvas.width, canvas.height, win.devicePixelRatio, inStep()]);
    if (key === layoutKey) return;
    layoutKey = key;
    send({
      type: 'overlay.layout',
      pane: box(),
      picture,
      output: { width: canvas.width, height: canvas.height },
      in_step: inStep(),
      dpr: win.devicePixelRatio || 1,
    });
  };

  /**
   * What the canvas really holds at the middle of each handle (a proof that it was drawn, not
   * only that it was told to): one pixel each, so it stays cheap. Only when the host asks.
   */
  const acknowledge = () => {
    const picture = rect();
    const ratio = win.devicePixelRatio || 1;
    const probes =
      overlay?.visible && picture
        ? overlay.handles.map((handle) => {
            const middle =
              handle.kind === 'center'
                ? handle.from
                : { u: (handle.from.u + handle.to.u) / 2, v: (handle.from.v + handle.to.v) / 2 };
            const at = toPane(picture, middle);
            const pixel = context.getImageData(Math.round(at.x * ratio), Math.round(at.y * ratio), 1, 1).data;
            return { id: handle.id, rgba: [...pixel] };
          })
        : [];
    draws += 1;
    send({
      type: 'overlay.drawn',
      stamp: overlay?.stamp ?? null,
      draws,
      visible: overlay?.visible === true,
      entity: overlay?.entity ?? null,
      active: overlay?.active?.handle ?? null,
      outlines: overlay?.outlines.length ?? 0,
      probes,
    });
  };

  const onResize = () => {
    // A resize never ends a drag: picture coordinates do not depend on the pane.
    redraw();
    if (enabled) reportLayout();
  };

  const pointOf = (event) => {
    const picture = rect();
    if (!picture) return null;
    const bounds = surface.getBoundingClientRect();
    return toPicture(picture, event.clientX - bounds.left, event.clientY - bounds.top);
  };

  const scheduleFlush = () => {
    if (frameRequest) return;
    frameRequest = win.requestAnimationFrame(() => {
      frameRequest = 0;
      session.flush();
    });
  };

  const endCapture = () => {
    if (capturedPointer !== null) {
      try {
        surface.releasePointerCapture(capturedPointer);
      } catch {
        // Already released by the browser.
      }
      capturedPointer = null;
    }
  };

  const cancel = (reason) => {
    if (frameRequest) {
      win.cancelAnimationFrame(frameRequest);
      frameRequest = 0;
    }
    if (session.cancel(reason)) endCapture();
  };

  const listeners = [];
  const on = (target, type, handler, options) => {
    target.addEventListener(type, handler, options);
    listeners.push(() => target.removeEventListener(type, handler, options));
  };

  on(surface, 'pointerdown', (event) => {
    if (!enabled || event.button !== 0 || !event.isPrimary || !inStep()) return;
    // A press while another is believed down means its release was lost.
    if (session.pressed) cancel('pointer_cancel');
    const point = pointOf(event);
    if (!point || !insidePicture(point)) return;
    event.preventDefault();
    surface.focus({ preventScroll: true });
    try {
      surface.setPointerCapture(event.pointerId);
      capturedPointer = event.pointerId;
    } catch {
      capturedPointer = null;
    }
    session.down(point, event.shiftKey);
  });
  on(surface, 'pointermove', (event) => {
    if (!enabled) return;
    const point = pointOf(event);
    if (!point) return;
    if (session.pressed) {
      session.move(point);
      scheduleFlush();
      return;
    }
    const next = inStep() ? hoverHandle(overlay, point) : null;
    if (next !== hover) {
      hover = next;
      surface.style.cursor = next ? 'grab' : 'default';
      redraw();
    }
  });
  on(surface, 'pointerup', (event) => {
    if (!event.isPrimary || event.button !== 0) return;
    const point = pointOf(event);
    if (session.pressed && point) {
      if (frameRequest) {
        win.cancelAnimationFrame(frameRequest);
        frameRequest = 0;
      }
      session.up(point);
    } else {
      session.release();
    }
    endCapture();
  });
  on(surface, 'pointercancel', () => cancel('pointer_cancel'));
  on(surface, 'lostpointercapture', () => {
    capturedPointer = null;
    // Still pressed when the capture went away: the release will not reach us.
    cancel('pointer_cancel');
  });
  on(doc, 'keydown', (event) => {
    if (event.key !== 'Escape') return;
    if (session.pressed) {
      cancel('escape');
    } else if (overlay?.active) {
      // A drag the host holds without a press here (begun by a script): Escape still ends it.
      session.emit('cancel', { reason: 'escape' });
    }
  });
  on(win, 'blur', () => cancel('blur'));
  on(doc, 'visibilitychange', () => {
    if (doc.visibilityState === 'hidden') cancel('hidden');
  });

  const observer = typeof win.ResizeObserver === 'function' ? new win.ResizeObserver(onResize) : null;
  observer?.observe(surface);
  on(win, 'resize', onResize);

  const handlers = {
    // The host turns the overlay on for a directed scene and off for everything else, so a
    // timeline picture is never covered by an element that eats its pointer events.
    'overlay.enable': (message) => {
      enabled = message.enabled === true;
      surface.style.pointerEvents = enabled ? 'auto' : 'none';
      if (!enabled) {
        cancel('hidden');
        overlay = null;
        hover = null;
      }
      redraw();
      if (enabled) reportLayout();
    },
    'overlay.set': (message) => {
      overlay = parseOverlay(message.overlay);
      redraw();
      if (message.ack === true) acknowledge();
    },
    'overlay.clear': () => {
      overlay = null;
      hover = null;
      redraw();
    },
  };

  redraw();

  return {
    handles: (type) => type in handlers,
    receive: (message) => handlers[message.type](message),
    // The runtime calls this after it drew (and announced) a picture: the canvas may have changed
    // shape. Off, there is nothing on the overlay canvas and nothing to report, so it touches no
    // layout, no canvas and no JSON.
    refresh: () => {
      if (!enabled) return false;
      redraw();
      reportLayout();
      return true;
    },
    // Whether a frame has to bother the overlay at all (the runtime gates `refresh` on it).
    active: () => enabled,
    destroy: () => {
      listeners.forEach((off) => off());
      observer?.disconnect();
      surface.remove();
    },
    // Test and measurement hooks; never part of the bridge.
    debug: {
      surface,
      state: () => ({ enabled, overlay, hover, pressed: session.state, rect: rect(), inStep: inStep() }),
      redraw,
    },
  };
};
