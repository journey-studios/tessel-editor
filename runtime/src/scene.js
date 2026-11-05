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

/** A built scene ready to draw at a given time and camera. */
export class SceneRig {
  /**
   * Whether the scene file has splat objects. A rig without them is a pure function of the scene
   * file, so the compositor may keep it between frames; one with splats holds leases on meshes
   * that belong to the asset store and is built (and released) per frame.
   */
  static hasSplatObjects(sceneFile) {
    return sceneFile.objects.some((object) => object.type === 'splat');
  }

  /** Async because splat meshes may need to be (re)built after a lost graphics context. */
  static async build(sceneFile, assets) {
    const leases = new Map();
    try {
      for (const [index, object] of sceneFile.objects.entries()) {
        if (object.type !== 'splat') continue;
        requireObject(assets.get(object.asset).kind === 'splat', index, `\`${object.asset}\` is not a splat asset`);
        leases.set(index, await assets.splatLease(object.asset));
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
      this.spinners.push({ node, spin });
      this.scene.add(node);
    });
  }

  get hasSplats() {
    return this.splats.length > 0;
  }

  /**
   * `time` in seconds drives `spin` (degrees per second per axis). `stage` is the SplatStage of
   * the compositor when the scene has splats; `aborted` stops the wait for a sort.
   */
  async draw(renderer, target, { camera: cameraSpec, time, width, height, stage, aborted }) {
    const camera = new THREE.PerspectiveCamera(cameraSpec.fov, width / height, 0.05, 1000);
    camera.position.set(...cameraSpec.position);
    camera.up.set(0, 1, 0);
    camera.lookAt(...cameraSpec.target);
    camera.updateMatrixWorld(true);
    camera.updateProjectionMatrix();

    this.spinners.forEach(({ node, spin }) => {
      node.rotation.set(
        THREE.MathUtils.degToRad(spin[0] * time),
        THREE.MathUtils.degToRad(spin[1] * time),
        THREE.MathUtils.degToRad(spin[2] * time),
      );
      node.updateMatrixWorld(true);
    });

    if (this.hasSplats) {
      stage.attach(this.scene);
      try {
        await stage.settle(this.scene, camera, aborted);
        this.render(renderer, target, camera);
      } finally {
        stage.detach(this.scene);
      }
      return;
    }
    this.render(renderer, target, camera);
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
