import { shownCounts } from './text-reveal.js';

const FAMILY = 'Tessel Inter';
const WEIGHT = 860;
const LINE_PITCH = 1.12;
const SAFE_FRACTION = 0.9;
let fontReady;

/** The editor ships the same typeface with both the WebView and the headless runtime. */
export const loadTypographicFont = () => {
  // Node can import the compositor for tests without loading a browser-only font.
  fontReady ??= import('./fonts/InterVariable.woff2')
    .then(({ default: bytes }) => new FontFace(FAMILY, bytes, { weight: '100 900' }).load())
    .then((face) => { document.fonts.add(face); });
  return fontReady;
};

const setFont = (context, size) => {
  context.font = `${WEIGHT} ${size}px "${FAMILY}"`;
  context.textBaseline = 'top';
};

/** Honour explicit line breaks and wrap at words when a line crosses the safe width. */
const wrapLines = (content, context, safeWidth) => content.split('\n').flatMap((line) => {
  if (context.measureText(line).width <= safeWidth) return [line];

  const lines = [];
  let row = '';
  for (const token of line.match(/\S+\s*|\s+/gu) ?? []) {
    const next = row + token;
    if (row.trim() && context.measureText(next.trimEnd()).width > safeWidth) {
      lines.push(row.trimEnd());
      row = token.trimStart();
    } else {
      row = next;
    }
  }
  lines.push(row.trimEnd());
  return lines;
});

/** Rasterize once at output resolution. Transform and reveal are applied to this texture later. */
export const drawTypographicText = (spec, canvasWidth, canvasHeight) => {
  const canvas = document.createElement('canvas');
  const context = canvas.getContext('2d');
  // A text with a box (F070) is as wide as the box, and its lines are the core's: the same
  // lines the reveal counted, so typing never re-wraps.
  const boxFraction = spec.width > 0 ? Math.min(1, spec.width / 100) : SAFE_FRACTION;
  const safeWidth = Math.max(1, canvasWidth * boxFraction);
  const safeHeight = Math.max(1, canvasHeight * SAFE_FRACTION);
  const requestedSize = spec.size * canvasHeight / 100;
  setFont(context, requestedSize);
  const lines = Array.isArray(spec.lines) && spec.lines.length > 0
    ? spec.lines
    : wrapLines(spec.content, context, safeWidth);
  const widest = Math.max(1, ...lines.map((line) => context.measureText(line).width));
  const size = requestedSize * Math.min(1, safeWidth / widest, safeHeight / (lines.length * requestedSize * LINE_PITCH));
  setFont(context, size);

  const metrics = lines.map((line) => context.measureText(line));
  const textWidth = Math.max(...metrics.map((metric) => metric.width));
  const offsets = metrics.map((metric, row) => ({
    x: spec.align === 'left' ? 0 : (textWidth - metric.width) / 2,
    y: row * LINE_PITCH * size,
  }));
  // Advance widths are not ink bounds. Take overshoots into account so accents and
  // a glyph that overhangs the advance do not get clipped by the backing canvas.
  const left = Math.floor(Math.min(0, ...metrics.map((metric, row) => offsets[row].x - metric.actualBoundingBoxLeft)));
  const right = Math.ceil(Math.max(textWidth, ...metrics.map((metric, row) => offsets[row].x + metric.actualBoundingBoxRight)));
  const top = Math.floor(Math.min(0, ...metrics.map((metric, row) => offsets[row].y - metric.actualBoundingBoxAscent)));
  const bottom = Math.ceil(Math.max(lines.length * LINE_PITCH * size,
    ...metrics.map((metric, row) => offsets[row].y + metric.actualBoundingBoxDescent)));
  canvas.width = Math.max(1, right - left);
  canvas.height = Math.max(1, bottom - top);
  setFont(context, size);
  context.fillStyle = spec.color;
  // Only the first `spec.visible` characters are drawn, at the place the whole line gives them.
  const shown = shownCounts(spec.content, lines, spec.visible);
  lines.forEach((line, row) => {
    const typed = [...line].slice(0, shown[row]).join('');
    if (typed) context.fillText(typed, offsets[row].x - left, offsets[row].y - top);
  });
  return { canvas, width: canvas.width, height: canvas.height, anchorX: -left, anchorY: -top };
};
