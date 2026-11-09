// 3D scene layers: a `tessel-scene` file (boxes, spheres, Gaussian splats) seen through the
// plan's camera. Time and camera come from the plan; this module only draws them. Splats are
// Spark meshes (see splat-stage.js): opaque objects draw first and the splat pass is depth-tested
// against them.

import * as THREE from 'three';

import { RuntimeError } from './protocol.js';

const vector = (value, fallback) =>
  Array.isArray(value) && value.length === 3 && value.every(Number.isFinite) ? value : fallback;

const requireObject = (condition, index, message) => {
  if (!condition) throw new RuntimeError('decode_failed', `Scene object ${index}: ${message}`);
};

/**
 * The scene of a layer whose asset is a bare splat (a clip of a splat file, BL143): one splat object,
 * turned by the plan's `orientation` (degrees around X, Y, Z). The SHARP axes (y down, z forward) are a
 * half turn around X from the editor's (y up, -z forward), so the plan says `[180, 0, 0]` and the
 * capture camera then looks at the picture the way the source image did.
 */
export const splatLayerScene = (layer) => ({
  format: 'tessel-scene',
  version: 1,
  objects: [{ type: 'splat', asset: layer.asset, rotation: vector(layer.orientation, [0, 0, 0]) }],
});

/** The scene a `scene` layer draws: the file of a scene asset, or the synthesized one of a splat asset. */
export const layerScene = (layer, entry) => (entry?.kind === 'splat' ? splatLayerScene(layer) : entry?.scene);

/** A built scene ready to draw at a given time and camera. */
export class SceneRig {
  /**
   * Whether the scene file has splat objects. A rig without them is a pure function of the scene
   * file, so the compositor may keep it between frames; one with splats holds leases on meshes
   * that belong to the asset store and is built (and released) per frame: the store hands the same
   * meshes out again, so building the rig decodes nothing and creates no Spark generator (BL131).
   */
  static hasSplatObjects(sceneFile) {
    return sceneFile.objects.some((object) => object.type === 'splat');
  }

  /**
   * Async because splat meshes may need to be (re)built after a lost graphics context. `owner` names
   * the Spark sort lineage the rig draws in (BL130): the store lends the same mesh to the same owner
   * and object every frame, so what the lineage sorted is still the set of splats it is handed.
   */
  static async build(sceneFile, assets, owner = null) {
    const leases = new Map();
    try {
      for (const [index, object] of sceneFile.objects.entries()) {
        if (object.type !== 'splat') continue;
        requireObject(assets.get(object.asset).kind === 'splat', index, `\`${object.asset}\` is not a splat asset`);
        leases.set(index, await assets.splatLease(object.asset, owner === null ? null : `${owner}#${index}`));
      }
      return new SceneRig(sceneFile, leases);
    } catch (error) {
      leases.forEach((lease) => lease.release());
      throw error;
    }
  }

  constructor(sceneFile, splatMeshes) {
    this.scene = new THREE.Scene();
    const [r, g, b] = sceneFile.background ?? [0, 0, 0, 1];
    // Alpha of the background is ignored: scene layers are opaque in v1.
    this.background = new THREE.Color(r, g, b);
    this.spinners = [];
    this.splats = [];
    this.disposables = [];

    this.scene.add(new THREE.AmbientLight(0xffffff, 1.6));
    const sun = new THREE.DirectionalLight(0xffffff, 2.2);
    sun.position.set(3, 5, 4);
    this.scene.add(sun);

    sceneFile.objects.forEach((object, index) => {
      const position = vector(object.position, [0, 0, 0]);
      const spin = vector(object.spin, [0, 0, 0]);
      // Degrees around X, Y, Z before the spin is added: how an object is turned in the scene.
      const turn = vector(object.rotation, [0, 0, 0]);
      let node;
      if (object.type === 'box' || object.type === 'sphere') {
        const color = new THREE.Color(object.color ?? '#cccccc');
        requireObject(typeof (object.color ?? '#cccccc') === 'string', index, '`color` must be a string');
        const geometry =
          object.type === 'box'
            ? new THREE.BoxGeometry(...vector(object.size, [1, 1, 1]))
            : new THREE.SphereGeometry(Number(object.radius ?? 0.5), 32, 16);
        const material = new THREE.MeshLambertMaterial({ color });
        this.disposables.push(geometry, material);
        node = new THREE.Mesh(geometry, material);
      } else if (object.type === 'splat') {
        const lease = splatMeshes.get(index);
        this.splats.push(lease);
        node = lease.mesh;
        node.scale.setScalar(Number(object.scale ?? 1));
      } else {
        throw new RuntimeError('decode_failed', `Scene object ${index}: unknown type \`${object.type}\``);
      }
      node.position.set(...position);
      this.spinners.push({ node, spin, turn });
      this.scene.add(node);
    });
  }

  get hasSplats() {
    return this.splats.length > 0;
  }

  /** Turns every object to where `time` (seconds) puts it: its own rotation plus its spin. */
  pose(time) {
    this.spinners.forEach(({ node, spin, turn }) => {
      node.rotation.set(
        THREE.MathUtils.degToRad(turn[0] + spin[0] * time),
        THREE.MathUtils.degToRad(turn[1] + spin[1] * time),
        THREE.MathUtils.degToRad(turn[2] + spin[2] * time),
      );
      node.updateMatrixWorld(true);
    });
  }

  /** The camera a plan layer names, at the aspect of the picture. */
  static camera(cameraSpec, width, height) {
    const camera = new THREE.PerspectiveCamera(cameraSpec.fov, width / height, 0.05, 1000);
    camera.position.set(...cameraSpec.position);
    camera.up.set(0, 1, 0);
    camera.lookAt(...cameraSpec.target);
    camera.updateMatrixWorld(true);
    camera.updateProjectionMatrix();
    return camera;
  }

  /**
   * `time` in seconds drives `spin` (degrees per second per axis). `stage` is the SplatStage of
   * the compositor when the scene has splats; `aborted` stops the wait for a sort. `motion` lets a
   * splat scene be drawn with the order of the last finished sort (BL130, see `SplatStage.settle`).
   * Resolves `{ stale }`: the splats were drawn with an order that is not the one for this camera.
   */
  async draw(renderer, target, { camera: cameraSpec, time, width, height, stage, aborted, motion = false }) {
    const camera = SceneRig.camera(cameraSpec, width, height);

    this.pose(time);

    if (this.hasSplats) {
      stage.attach(this.scene);
      try {
        const { stale } = await stage.settle(this.scene, camera, aborted, { motion });
        this.render(renderer, target, camera);
        return { stale };
      } finally {
        stage.detach(this.scene);
      }
    }
    this.render(renderer, target, camera);
    return { stale: false };
  }

  render(renderer, target, camera) {
    renderer.setRenderTarget(target);
    renderer.setClearColor(this.background, 1);
    renderer.clear(true, true, true);
    renderer.render(this.scene, camera);
    renderer.setRenderTarget(null);
  }

  /** Gives the splat meshes back (they belong to the asset store) and frees the plain geometry. */
  dispose() {
    this.disposables.forEach((item) => item.dispose());
    this.splats.forEach((lease) => lease.release());
  }
}
