// Colour correction of a layer (inspector tab "Correção de cor"): brightness, contrast and saturation,
// each -100..100 with 0 untouched. The core sends `layer.grading` only when one of them is not 0
// (crates/core/src/plan.rs, `Grading`); an absent or all-zero grading draws with the plain material,
// so an ungraded picture is byte-for-byte the one drawn before grading existed.
//
// The formula lives here twice, once for the GPU (GLSL) and once for the CPU (`gradeColor`, what the
// tests compare real pixels against); keep them identical. It works on the encoded (display) values
// the compositor draws with (textures are `NoColorSpace`, output `LinearSRGBColorSpace`: no
// conversion either way), on the layer's own texel before its opacity, tint and blend:
//   1. brightness adds brightness / 200 (±100 shifts half the range);
//   2. contrast scales the distance from mid grey by 1 + contrast / 100 (-100 is flat grey);
//   3. saturation scales the distance from the Rec. 709 luma by 1 + saturation / 100
//      (-100 is greyscale, +100 doubles it);
// each step clamped to 0..1. Alpha is never touched.
import * as THREE from 'three';

const LUMA = [0.2126, 0.7152, 0.0722];

const clamp01 = (value) => Math.min(1, Math.max(0, value));

/** Whether `grading` leaves every pixel as it is (absent, or all three at 0). */
export const isIdentityGrading = (grading) =>
  !grading || ((grading.brightness ?? 0) === 0 && (grading.contrast ?? 0) === 0 && (grading.saturation ?? 0) === 0);

/** The three values as numbers, a missing one being 0. */
const values = (grading) => [grading?.brightness ?? 0, grading?.contrast ?? 0, grading?.saturation ?? 0];

/** CPU reference of the shader: `rgb` and the result are 0..1 channels. */
export const gradeColor = (rgb, grading) => {
  const [brightness, contrast, saturation] = values(grading);
  let color = rgb.map((channel) => clamp01(channel + brightness / 200));
  color = color.map((channel) => clamp01((channel - 0.5) * (1 + contrast / 100) + 0.5));
  const luma = color.reduce((sum, channel, index) => sum + channel * LUMA[index], 0);
  return color.map((channel) => clamp01(luma + (channel - luma) * (1 + saturation / 100)));
};

const GLSL = /* glsl */ `
uniform vec3 tesselGrading;
vec3 tesselGrade( vec3 color ) {
	color = clamp( color + tesselGrading.x / 200.0, 0.0, 1.0 );
	color = clamp( ( color - 0.5 ) * ( 1.0 + tesselGrading.y / 100.0 ) + 0.5, 0.0, 1.0 );
	float luma = dot( color, vec3( ${LUMA.map((weight) => weight.toFixed(4)).join(', ')} ) );
	return clamp( vec3( luma ) + ( color - vec3( luma ) ) * ( 1.0 + tesselGrading.z / 100.0 ), 0.0, 1.0 );
}
`;

const SAMPLE = 'diffuseColor *= sampledDiffuseColor;';

/**
 * Makes `material` (a `MeshBasicMaterial` with a `map`) grade its texel. The values live in
 * `material.userData.grading` (a uniform shared with the compiled program), so a retained material
 * takes new values with `setGrading` and keeps its program.
 */
export const makeGraded = (material, grading) => {
  const uniform = { value: new THREE.Vector3() };
  material.userData.grading = uniform;
  setGrading(material, grading);
  material.onBeforeCompile = (shader) => {
    const chunk = THREE.ShaderChunk.map_fragment;
    if (!chunk.includes(SAMPLE)) throw new Error('three.js map_fragment changed: colour correction cannot be applied');
    shader.uniforms.tesselGrading = uniform;
    shader.fragmentShader = shader.fragmentShader
      .replace('void main() {', `${GLSL}\nvoid main() {`)
      .replace('#include <map_fragment>', chunk.replace(SAMPLE, `sampledDiffuseColor.rgb = tesselGrade( sampledDiffuseColor.rgb );\n\t${SAMPLE}`));
  };
  material.customProgramCacheKey = () => 'tessel-grading';
  return material;
};

/** New values for a material made by `makeGraded`. */
export const setGrading = (material, grading) => {
  material.userData.grading.value.set(...values(grading));
};
