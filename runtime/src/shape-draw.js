// The picture of a shape clip (F067 step 1A, F068 step 1B): a rectangle, an ellipse or a path with
// a solid or gradient fill, a contour and a drop shadow, drawn from the spec the core sent in
// `layer.shape` and from the canvas size alone, so the monitor, the PNG of `render.frame` and the
// export draw the same pixels.
//
// Units (crates/core/src/shape.rs): `width`/`height` are percent of the canvas width/height,
// `radius`, `stroke_width` and `shadow_blur` percent of the canvas height, `shadow_x` percent of
// the canvas width, `shadow_y` of the height. The box is `round(width/100 × canvasWidth)` by
// `round(height/100 × canvasHeight)`; the contour is centred on its edge and the shadow spreads
// past it, so the picture is the box plus a symmetric margin and the box stays centred on the clip.

/** What an older plan may leave out: step 1A sent only the first five fields. */
const DEFAULTS = {
  kind: 'rect',
  radius: 0,
  fill_mode: 'solid',
  fill_to: '#ffffff',
  stops: '',
  gradient_angle: 0,
  stroke: '#ffffff',
  stroke_width: 0,
  shadow: '#00000080',
  shadow_blur: 0,
  shadow_x: 0,
  shadow_y: 0,
  path: 'M 50 0 L 100 100 L 0 100 Z',
};

/** The picture is a texture: its margin never takes the whole picture past what a GPU accepts. */
const MAX_PICTURE_SIDE = 8192;
/** Canvas `shadowBlur` is a Gaussian of sigma `blur / 2`: three sigmas is where it has faded out. */
const BLUR_REACH = 1.5;

const margin = (reach, side) => Math.max(0, Math.min(Math.ceil(reach), Math.floor((MAX_PICTURE_SIDE - side) / 2)));

/** Pixel measures of a shape on a canvas: the box, the contour, the shadow and the margin around the box. */
export const shapeMetrics = (spec, canvasWidth, canvasHeight) => {
  const style = { ...DEFAULTS, ...spec };
  const width = Math.max(1, Math.round((style.width / 100) * canvasWidth));
  const height = Math.max(1, Math.round((style.height / 100) * canvasHeight));
  const stroke = (style.stroke_width / 100) * canvasHeight;
  const blur = (style.shadow_blur / 100) * canvasHeight;
  const offsetX = (style.shadow_x / 100) * canvasWidth;
  const offsetY = (style.shadow_y / 100) * canvasHeight;
  const shadowOn = blur > 0 || offsetX !== 0 || offsetY !== 0;
  const reachX = stroke / 2 + (shadowOn ? blur * BLUR_REACH + Math.abs(offsetX) : 0);
  const reachY = stroke / 2 + (shadowOn ? blur * BLUR_REACH + Math.abs(offsetY) : 0);
  const marginX = margin(reachX, width);
  const marginY = margin(reachY, height);
  return { style, width, height, stroke, blur, offsetX, offsetY, shadowOn, marginX, marginY };
};

/** The outline of the shape in canvas pixels, the box's top-left corner at `(left, top)`. */
const outline = ({ style, width, height }, canvasHeight, left, top) => {
  const path = new Path2D();
  if (style.kind === 'ellipse') {
    path.ellipse(left + width / 2, top + height / 2, width / 2, height / 2, 0, 0, Math.PI * 2);
    return path;
  }
  if (style.kind === 'path') {
    // The path lives in a 100 x 100 box stretched over the shape; the matrix moves the geometry
    // and leaves the contour's thickness alone.
    path.addPath(new Path2D(style.path), new DOMMatrix([width / 100, 0, 0, height / 100, left, top]));
    return path;
  }
  const radius = Math.min((style.radius / 100) * canvasHeight, Math.min(width, height) / 2);
  path.roundRect(left, top, width, height, radius);
  return path;
};

/**
 * The colours of a gradient: `fill` at 0, the intermediate `stops` (canonical text of
 * crates/core/src/shape.rs, `<at> <colour>` pairs, positions strictly inside 0..1) and `fill_to`
 * at 1. No stops is the two-colour gradient of step 1B, drawn exactly as before.
 */
const addColorStops = (gradient, style) => {
  gradient.addColorStop(0, style.fill);
  const tokens = String(style.stops ?? '').split(' ').filter(Boolean);
  for (let index = 0; index + 1 < tokens.length; index += 2) {
    gradient.addColorStop(Number(tokens[index]), tokens[index + 1]);
  }
  gradient.addColorStop(1, style.fill_to);
};

/**
 * Linear gradient along `gradient_angle` (0 runs left to right, 90 top to bottom) through the
 * centre of the box, as long as the box's projection on that direction, so the two corners the
 * line points at get exactly the two colours.
 */
const linearGradient = (context, style, width, height, left, top) => {
  const angle = (style.gradient_angle * Math.PI) / 180;
  const [dx, dy] = [Math.cos(angle), Math.sin(angle)];
  const half = (Math.abs(width * dx) + Math.abs(height * dy)) / 2;
  const [cx, cy] = [left + width / 2, top + height / 2];
  const gradient = context.createLinearGradient(cx - dx * half, cy - dy * half, cx + dx * half, cy + dy * half);
  addColorStops(gradient, style);
  return gradient;
};

/**
 * The fill. A radial gradient is an ellipse of the box: it is defined as a unit circle and the
 * path is mapped into that space, so the end colour lands on the edge of the box on both axes.
 */
const paintFill = (context, shape, path, left, top) => {
  const { style, width, height } = shape;
  if (style.fill_mode === 'none') return;
  if (style.fill_mode === 'radial') {
    const gradient = context.createRadialGradient(0, 0, 0, 0, 0, 1);
    addColorStops(gradient, style);
    const [cx, cy] = [left + width / 2, top + height / 2];
    const unit = new Path2D();
    unit.addPath(path, new DOMMatrix([2 / width, 0, 0, 2 / height, (-2 * cx) / width, (-2 * cy) / height]));
    context.save();
    context.translate(cx, cy);
    context.scale(width / 2, height / 2);
    context.fillStyle = gradient;
    context.fill(unit);
    context.restore();
    return;
  }
  context.fillStyle = style.fill_mode === 'linear' ? linearGradient(context, style, width, height, left, top) : style.fill;
  context.fill(path);
};

const paintStroke = (context, shape, path) => {
  const { style, stroke } = shape;
  if (stroke <= 0) return;
  context.lineWidth = stroke;
  context.strokeStyle = style.stroke;
  // A rectangle keeps sharp outer corners like a CSS border; a path is drawn as a pen would.
  context.lineJoin = style.kind === 'rect' ? 'miter' : 'round';
  context.lineCap = style.kind === 'path' ? 'round' : 'butt';
  context.stroke(path);
};

const newCanvas = (width, height) => {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  return canvas;
};

/**
 * Draws the shape. Returns `{ canvas, width, height }`: the picture (box plus margin) and its size,
 * which the compositor lays out centred on the clip's position, so the box stays where the clip says.
 * Even sizes on an even canvas land the edges on whole pixels; otherwise they are antialiased
 * across half a pixel.
 */
export const drawShape = (spec, canvasWidth, canvasHeight) => {
  const shape = shapeMetrics(spec, canvasWidth, canvasHeight);
  const { width, height, marginX, marginY, style } = shape;
  const pictureWidth = width + 2 * marginX;
  const pictureHeight = height + 2 * marginY;
  const canvas = newCanvas(pictureWidth, pictureHeight);
  // The shadow is cast by the finished silhouette (fill and contour together): drawing it per
  // paint call would lay the contour's shadow over the fill.
  const body = shape.shadowOn ? newCanvas(pictureWidth, pictureHeight) : canvas;
  const context = body.getContext('2d');
  const path = outline(shape, canvasHeight, marginX, marginY);
  paintFill(context, shape, path, marginX, marginY);
  paintStroke(context, shape, path);
  if (shape.shadowOn) {
    const target = canvas.getContext('2d');
    target.shadowColor = style.shadow;
    target.shadowBlur = shape.blur;
    target.shadowOffsetX = shape.offsetX;
    target.shadowOffsetY = shape.offsetY;
    target.drawImage(body, 0, 0);
    body.width = 0;
    body.height = 0;
  }
  return { canvas, width: pictureWidth, height: pictureHeight };
};
