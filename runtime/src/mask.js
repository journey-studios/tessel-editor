// Rounded or elliptical mask of an image or video layer (inspector section "Máscara", F068).
// The core sends `layer.mask = { shape: 'rounded' | 'ellipse', radius }` only while a mask is on
// (crates/core/src/plan.rs, `Mask`); a layer without it draws with the plain material, so an
// unmasked picture is byte-for-byte the one drawn before masks existed.
//
// The mask cuts the picture AFTER the crop, in the layer's own space (the quad is built in canvas
// pixels and then rotated by the mesh), so the corners belong to the visible rectangle and turn
// and scale with it. `radius` is a percent of the shorter side of that cropped rectangle: 50 turns
// a square into a circle. The edge is one pixel of antialiasing, from a signed distance.
//
// The shape of the cut lives here twice, once for the GPU (GLSL) and once for the CPU
// (`maskCoverage`, what the tests compare real pixels against); keep them identical.
import * as THREE from 'three';

const clamp01 = (value) => Math.min(1, Math.max(0, value));

/** `0` rounded, `1` ellipse: the uniform the shader branches on. */
const SHAPE_CODES = { rounded: 0, ellipse: 1 };

/**
 * The rectangle a crop leaves in the quad's local pixels, and the corner radius in pixels.
 * `width`/`height` are the uncropped layer size, `crop` is `[left, right, top, bottom]` percent
 * (the same arguments as `quadGeometry`).
 */
export const maskRect = (width, height, crop, mask) => {
  const [left, right, top, bottom] = crop.map((percent) => percent / 100);
  const x0 = -width / 2 + left * width;
  const x1 = width / 2 - right * width;
  const y1 = height / 2 - top * height;
  const y0 = -height / 2 + bottom * height;
  const halfW = (x1 - x0) / 2;
  const halfH = (y1 - y0) / 2;
  const shorter = Math.min(halfW, halfH);
  const radius = Math.min(shorter, Math.max(0, ((mask.radius ?? 0) / 100) * 2 * shorter));
  return { centerX: (x0 + x1) / 2, centerY: (y0 + y1) / 2, halfW, halfH, radius };
};

/** CPU reference of the shader: coverage 0..1 of the point `(x, y)` (local pixels). */
export const maskCoverage = (x, y, rect, shape) => {
  const px = x - rect.centerX;
  const py = y - rect.centerY;
  let distance;
  if (shape === 'ellipse') {
    const shorter = Math.min(rect.halfW, rect.halfH);
    distance = (Math.hypot(px / rect.halfW, py / rect.halfH) - 1) * shorter;
  } else {
    const qx = Math.abs(px) - (rect.halfW - rect.radius);
    const qy = Math.abs(py) - (rect.halfH - rect.radius);
    distance = Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) + Math.min(Math.max(qx, qy), 0) - rect.radius;
  }
  return clamp01(0.5 - distance);
};

const VERTEX_DECL = 'varying vec2 vTesselMaskPosition;';
const FRAGMENT_DECL = /* glsl */ `
varying vec2 vTesselMaskPosition;
uniform vec4 tesselMaskRect;
uniform vec2 tesselMaskShape;
float tesselMaskCoverage() {
	vec2 p = vTesselMaskPosition - tesselMaskRect.xy;
	vec2 half_size = tesselMaskRect.zw;
	float radius = tesselMaskShape.y;
	float signed_distance;
	if ( tesselMaskShape.x > 0.5 ) {
		float shorter = min( half_size.x, half_size.y );
		signed_distance = ( length( p / half_size ) - 1.0 ) * shorter;
	} else {
		vec2 q = abs( p ) - ( half_size - vec2( radius ) );
		signed_distance = length( max( q, vec2( 0.0 ) ) ) + min( max( q.x, q.y ), 0.0 ) - radius;
	}
	return clamp( 0.5 - signed_distance, 0.0, 1.0 );
}
`;

/**
 * Makes `material` cut its picture by a mask. The values live in `material.userData.mask` (uniforms
 * shared with the compiled program), so a retained material takes a new rectangle every frame with
 * `setMask` and keeps its program. Composes with colour correction: it wraps whatever
 * `onBeforeCompile` the material already has (grading.js).
 */
export const makeMasked = (material, rect, shape) => {
  const uniforms = { rect: { value: new THREE.Vector4() }, shape: { value: new THREE.Vector2() } };
  material.userData.mask = uniforms;
  setMask(material, rect, shape);
  const previous = material.onBeforeCompile;
  const previousKey = material.customProgramCacheKey;
  material.onBeforeCompile = (shader, renderer) => {
    previous.call(material, shader, renderer);
    shader.uniforms.tesselMaskRect = uniforms.rect;
    shader.uniforms.tesselMaskShape = uniforms.shape;
    if (!shader.fragmentShader.includes('#include <alphamap_fragment>')) {
      throw new Error('three.js meshbasic changed: the mask cannot be applied');
    }
    shader.vertexShader = shader.vertexShader
      .replace('void main() {', `${VERTEX_DECL}\nvoid main() {`)
      .replace('#include <begin_vertex>', '#include <begin_vertex>\n\tvTesselMaskPosition = position.xy;');
    shader.fragmentShader = shader.fragmentShader
      .replace('void main() {', `${FRAGMENT_DECL}\nvoid main() {`)
      .replace('#include <alphamap_fragment>', '#include <alphamap_fragment>\n\tdiffuseColor.a *= tesselMaskCoverage();');
  };
  // The default key is the source of `onBeforeCompile`, identical for every masked material; a
  // graded one has its own, which must stay part of the program's identity.
  material.customProgramCacheKey = () => `${previousKey.call(material)}|tessel-mask`;
  return material;
};

/** New values for a material made by `makeMasked`. */
export const setMask = (material, rect, shape) => {
  const { rect: rectUniform, shape: shapeUniform } = material.userData.mask;
  rectUniform.value.set(rect.centerX, rect.centerY, rect.halfW, rect.halfH);
  shapeUniform.value.set(SHAPE_CODES[shape] ?? 0, rect.radius);
};
