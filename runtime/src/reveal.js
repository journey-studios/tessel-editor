// Reveal of a clip's layers (inspector section "Revelação", F072): a straight edge sweeps across
// the canvas and uncovers what the layer draws, so a shot, a title or a group appears through a
// diagonal wipe over what lies beneath it.
//
// The core sends `layer.reveal = { edge, angle, direction }` only while a sweep is on
// (crates/core/src/plan.rs, `Reveal`), already eased for the frame: this module holds no clock.
// A layer without it draws with the plain material, so a clip that never revealed is byte-for-byte
// the picture drawn before reveals existed.
//
// The cut is in SCREEN space: the vertex shader hands the fragment the layer's world position (the
// scene is in canvas pixels, origin at the centre, y up), so it does not turn or scale with the
// layer; every layer of a group shares one edge. `edge` is the percent of the canvas width where
// the edge crosses mid height, `angle` its lean from the vertical in degrees (positive leads with
// the top of the edge in the direction of travel), `direction` is +1 when the layer shows to the
// left of the edge and -1 to the right. The edge is one pixel of antialiasing.
//
// The rule lives here twice, once for the GPU (GLSL) and once for the CPU (`revealCoverage`, what
// the tests compare real pixels against, itself the twin of `Reveal::coverage` in the core);
// keep all three identical.
import * as THREE from 'three';

const clamp01 = (value) => Math.min(1, Math.max(0, value));
const radians = (degrees) => (degrees * Math.PI) / 180;

/**
 * CPU reference of the shader: the share of the pixel whose centre is `(x, y)` (canvas pixels,
 * `y` from the top) the layer shows through.
 */
export const revealCoverage = (x, y, width, height, reveal) => {
  const direction = reveal.direction >= 0 ? 1 : -1;
  const edgeX = (reveal.edge / 100) * width + direction * Math.tan(radians(reveal.angle)) * (height / 2 - y);
  return clamp01(0.5 + (edgeX - x) * direction * Math.cos(radians(reveal.angle)));
};

/**
 * The four numbers the shader needs: the edge's world x at mid height, how far it moves per world
 * y (already signed by the direction), the direction, and the cosine that turns a horizontal
 * distance into one across the edge.
 */
export const revealValues = (reveal, width) => {
  const direction = reveal.direction >= 0 ? 1 : -1;
  return [
    (reveal.edge / 100) * width - width / 2,
    direction * Math.tan(radians(reveal.angle)),
    direction,
    Math.cos(radians(reveal.angle)),
  ];
};

const VERTEX_DECL = 'varying vec2 vTesselRevealPosition;';
const FRAGMENT_DECL = /* glsl */ `
varying vec2 vTesselRevealPosition;
uniform vec4 tesselReveal;
float tesselRevealCoverage() {
	float edge_x = tesselReveal.x + tesselReveal.y * vTesselRevealPosition.y;
	return clamp( 0.5 + ( edge_x - vTesselRevealPosition.x ) * tesselReveal.z * tesselReveal.w, 0.0, 1.0 );
}
`;

/**
 * Makes `material` uncover its picture behind a sweeping edge. The values live in
 * `material.userData.reveal` (a uniform shared with the compiled program), so a retained
 * material takes the next frame's edge with `setReveal` and keeps its program. Composes with
 * colour correction and the mask: it wraps whatever `onBeforeCompile` the material already has.
 */
export const makeRevealed = (material, reveal, width) => {
  const uniform = { value: new THREE.Vector4() };
  material.userData.reveal = uniform;
  setReveal(material, reveal, width);
  const previous = material.onBeforeCompile;
  const previousKey = material.customProgramCacheKey;
  material.onBeforeCompile = (shader, renderer) => {
    previous.call(material, shader, renderer);
    shader.uniforms.tesselReveal = uniform;
    if (!shader.fragmentShader.includes('#include <alphamap_fragment>')) {
      throw new Error('three.js meshbasic changed: the reveal cannot be applied');
    }
    shader.vertexShader = shader.vertexShader
      .replace('void main() {', `${VERTEX_DECL}\nvoid main() {`)
      .replace(
        '#include <begin_vertex>',
        '#include <begin_vertex>\n\tvTesselRevealPosition = ( modelMatrix * vec4( transformed, 1.0 ) ).xy;',
      );
    shader.fragmentShader = shader.fragmentShader
      .replace('void main() {', `${FRAGMENT_DECL}\nvoid main() {`)
      .replace('#include <alphamap_fragment>', '#include <alphamap_fragment>\n\tdiffuseColor.a *= tesselRevealCoverage();');
  };
  // Part of the program's identity, after whatever grading and mask already put there.
  material.customProgramCacheKey = () => `${previousKey.call(material)}|tessel-reveal`;
  return material;
};

/** New values for a material made by `makeRevealed`. */
export const setReveal = (material, reveal, width) => {
  material.userData.reveal.value.set(...revealValues(reveal, width));
};
