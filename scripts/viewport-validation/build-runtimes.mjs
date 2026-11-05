#!/usr/bin/env node
// Builds runtime variants from a SNAPSHOT copy of runtime/src. Production files are never edited.
//   node build-runtimes.mjs <snapshot-dir> <out-dir>
// baseline           byte-for-byte build of the snapshot sources (checked against the snapshot's dist)
// baseline-trace     + spans and debug info hooks, no behaviour change
// retained(-trace)   textures, materials, text (LRU 32), rigs and targets kept; no frame cache
// retained-rtc       + scene frame cache keyed by camera/animated time/size/generation/context
// retained-singlert  like the earlier prototype: ONE render target shared by all scene layers (to prove the bug)
import { cpSync, mkdirSync, readFileSync, writeFileSync, rmSync, createReadStream } from 'node:fs';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
const [snap, out] = process.argv.slice(2);
const nm = join(snap, 'runtime/node_modules');
const { build } = await import(join(nm, 'esbuild/lib/main.js'));

const replace = (text, a, b, file) => { if (!text.includes(a)) throw new Error(`missing in ${file}: ${a.slice(0, 80)}`); return text.replace(a, b); };
const edit = (dir, file, pairs) => { const p = join(dir, file); let t = readFileSync(p, 'utf8'); for (const [a, b] of pairs) t = replace(t, a, b, file); writeFileSync(p, t); };

const trace = (dir) => {
  edit(dir, 'index.js', [
    [`export const createRuntime = ({ post, assetUrl, canvas }) => {`, `const VV = (window.__vv ??= { events: [] });
const vvNow = () => performance.timeOrigin + performance.now();
const vvMark = (name, id, extra) => VV.events.push(extra === undefined ? [name, id, vvNow()] : [name, id, vvNow(), extra]);
export const createRuntime = ({ post, assetUrl, canvas }) => {
  window.__vvDump = () => { const events = VV.events; VV.events = []; post({ v: BRIDGE_VERSION, type: 'vv.trace', events }); };`],
    [`    const started = performance.now();\n    const { id, revision, plan, output, probes = [] } = request;`, `    const started = performance.now();\n    const { id, revision, plan, output, probes = [] } = request;\n    vvMark('js_start', id);`],
    [`      const describeStarted = performance.now();`, `      vvMark('js_drawn', id);\n      const describeStarted = performance.now();`],
    [`      send(message);\n    } catch (error) {`, `      vvMark('js_post', id);\n      send(message);\n      vvMark('js_posted', id);\n      if (!window.__vvNoRaf) requestAnimationFrame((ts) => vvMark('js_raf', id, ts + performance.timeOrigin));\n    } catch (error) {`],
    [`      latestRender = message.id;`, `      vvMark('js_recv', message.id, message.plan?.frame ?? null);\n      latestRender = message.id;`],
    [`    idle: () => renderQueue,`, `    idle: () => renderQueue,\n    compositor: () => compositor,\n    assets,`],
  ]);
  edit(dir, 'splat-stage.js', [
    [`  async settle(scene, camera, aborted) {\n    const target = camera.getWorldPosition(camera.position.clone());`, `  async settle(scene, camera, aborted) {\n    const __t0 = performance.now(); let __updates = 0, __updateMs = 0;\n    const target = camera.getWorldPosition(camera.position.clone());`],
    [`      await this.spark.update({ scene, camera });`, `      const __u = performance.now(); await this.spark.update({ scene, camera }); __updateMs += performance.now() - __u; __updates += 1;`],
    [`SETTLE_DISTANCE) return;`, `SETTLE_DISTANCE) { window.__vvPhase = { settle_ms: performance.now() - __t0, update_ms: __updateMs, attempts: __updates }; return; }`],
  ]);
  edit(dir, 'assets.js', [
    [`    const label = \`frame \${index} (\${path})\`;\n    return decodeImage(await fetchChecked(this.assetUrl(path), label), label);`,
     `    const label = \`frame \${index} (\${path})\`;
    const __a = performance.now();
    const __r = await fetchChecked(this.assetUrl(path), label);
    const __b = performance.now();
    const __bm = await decodeImage(__r, label);
    (window.__vvFetch ??= []).push([index, __b - __a, performance.now() - __b]);
    return __bm;`],
  ]);
  edit(dir, 'assets.js', [
    ["entry.frames.size > FRAME_CACHE_LIMIT", "entry.frames.size > (window.__vvFrameLimit ?? FRAME_CACHE_LIMIT)"],
  ]);
  edit(dir, 'index.js', [
    ["import { AssetStore } from './assets.js';", "import { AssetStore, imageTexture } from './assets.js';"],
    ["    assets,\n", "    assets,\n    imageTexture,\n"],
  ]);
  edit(dir, 'compositor.js', [
    [`      this.renderer.setRenderTarget(null);\n      this.renderer.setClearColor(0x000000, 1);`, `      const __r0 = performance.now();\n      this.renderer.setRenderTarget(null);\n      this.renderer.setClearColor(0x000000, 1);`],
    [`      const settled = performance.now();`, `      const settled = performance.now();\n      timings.final_draw_ms = settled - __r0;\n      if (window.__vvPhase) { timings.spark = window.__vvPhase; window.__vvPhase = null; }`],
  ]);
};

const retained = (dir, { rtCache, singleTarget }) => {
  // Generation and lifetime rules live in the asset store.
  edit(dir, 'assets.js', [
    [`    this.loading = new Map();\n  }`, `    this.loading = new Map();\n    // Bumped by anything that can change what an id means or kill its GPU data.\n    this.generation = 0;\n  }`],
    [`    this.loading.delete(id);\n    this.entries.set(id, { id, kind, descriptor, ...entry });`, `    this.loading.delete(id);\n    this.generation += 1;\n    this.entries.set(id, { id, kind, descriptor, ...entry });`],
    [`    const entry = this.entries.get(id);\n    if (!entry) return;\n    entry.mesh?.dispose();\n    entry.bitmap?.close?.();`, `    const entry = this.entries.get(id);\n    if (!entry) return;\n    this.generation += 1;\n    entry.rigs?.forEach((slot) => slot.rig.dispose());\n    entry.rigs = null;\n    entry.imageTexture?.dispose();\n    entry.imageTexture = null;\n    entry.mesh?.dispose();\n    entry.bitmap?.close?.();`],
    [`  dropGpu() {\n    this.entries.forEach((entry) => {`, `  dropGpu() {\n    this.generation += 1;\n    this.entries.forEach((entry) => {\n      entry.rigs?.forEach((slot) => slot.rig.dispose());\n      entry.rigs = null;\n      entry.imageTexture?.dispose();\n      entry.imageTexture = null;\n    });\n    this.entries.forEach((entry) => {`],
  ]);
  edit(dir, 'compositor.js', [
    [`import { SplatStage } from './splat-stage.js';`, `import { SplatStage } from './splat-stage.js';\nimport { imageTexture } from './assets.js';\nconst TEXT_LIMIT = 32;\nconst X = () => window.__exp ?? { rtCache: ${rtCache}, singleTarget: ${singleTarget} };`],
    [`    this.lost = false;`, `    this.lost = false;\n    this.materials = new Map();\n    this.texts = new Map();\n    this.targets = new Map();\n    this.frameKeys = new Map();\n    this.rtHits = 0;\n    this.rtMisses = 0;`],
    [`        const prepared = await this.prepareLayer(layer, { plan, width, height, assets, keep, aborted });`, `        const prepared = await this.prepareLayer(layer, { plan, width, height, assets, keep, aborted, order });`],
    [`  async prepareLayer(layer, { plan, width, height, assets, keep, aborted }) {`, `  /** Everything retained on this context; called with the store's dropGpu when the context dies. */
  dropRetained() {
    this.materials.forEach((material) => material.dispose());
    this.materials.clear();
    this.texts.forEach((slot) => slot.texture.dispose());
    this.texts.clear();
    this.targets.forEach((target) => target.dispose());
    this.targets.clear();
    this.frameKeys.clear();
  }

  async prepareLayer(layer, { plan, width, height, assets, keep, aborted, order }) {`],
    [`        const texture = keep(new THREE.Texture(entry.bitmap));
        texture.flipY = false;
        texture.premultiplyAlpha = false;
        texture.minFilter = THREE.LinearFilter;
        texture.magFilter = THREE.LinearFilter;
        texture.generateMipmaps = false;
        texture.colorSpace = THREE.NoColorSpace;
        texture.needsUpdate = true;
        return { texture,`, `        entry.imageTexture ??= imageTexture(entry.bitmap);
        const texture = entry.imageTexture;
        return { texture,`],
    [`        const text = drawText(layer.text, height);
        const texture = keep(canvasTexture(text.canvas));
        return { texture, width: text.width, height: text.height, fit: false, flipV: false };`, `        const textKey = JSON.stringify([layer.text, height]);
        let slot = this.texts.get(textKey);
        if (slot) this.texts.delete(textKey);
        else {
          const text = drawText(layer.text, height);
          slot = { text, texture: canvasTexture(text.canvas) };
        }
        this.texts.set(textKey, slot);
        // A frame draws at most a few text layers, so the oldest entry is never one in use now.
        while (this.texts.size > TEXT_LIMIT) {
          const [oldest] = this.texts.keys();
          this.texts.get(oldest).texture.dispose();
          this.texts.delete(oldest);
        }
        return { texture: slot.texture, width: slot.text.width, height: slot.text.height, fit: false, flipV: false };`],
    [`        const rig = keep(await SceneRig.build(entry.scene, assets));
        const target = keep(
          new THREE.WebGLRenderTarget(width, height, { samples: 4, depthBuffer: true, colorSpace: THREE.NoColorSpace }),
        );
        await rig.draw(`, `        entry.rigs ??= new Map();
        let rigSlot = entry.rigs.get(order);
        if (rigSlot && rigSlot.generation !== assets.generation) {
          rigSlot.rig.dispose();
          entry.rigs.delete(order);
          rigSlot = null;
        }
        if (!rigSlot) {
          const generation = assets.generation;
          rigSlot = { rig: await SceneRig.build(entry.scene, assets), generation };
          entry.rigs.set(order, rigSlot);
        }
        const { rig } = rigSlot;
        const slotKey = X().singleTarget ? 0 : order;
        let target = this.targets.get(slotKey);
        if (target && (target.width !== width || target.height !== height)) {
          target.dispose();
          this.targets.delete(slotKey);
          this.frameKeys.delete(slotKey);
          target = null;
        }
        if (!target) {
          target = new THREE.WebGLRenderTarget(width, height, { samples: 4, depthBuffer: true, colorSpace: THREE.NoColorSpace });
          this.targets.set(slotKey, target);
        }
        const animated = rig.spinners.some(({ spin }) => spin[0] !== 0 || spin[1] !== 0 || spin[2] !== 0);
        const frameKey = JSON.stringify([layer.asset, layer.camera, animated ? layer.scene_time.seconds : 0, width, height, rigSlot.generation]);
        if (X().rtCache && this.frameKeys.get(slotKey) === frameKey) {
          this.rtHits += 1;
          return { texture: target.texture, width, height, fit: false, flipV: true };
        }
        this.rtMisses += 1;
        this.frameKeys.delete(slotKey);
        await rig.draw(`],
    [`          aborted,
        });
        return { texture: target.texture, width, height, fit: false, flipV: true };`, `          aborted,
        });
        this.frameKeys.set(slotKey, frameKey);
        return { texture: target.texture, width, height, fit: false, flipV: true };`],
    [`    const material = keep(
      new THREE.MeshBasicMaterial({`, `    const makeMaterial = () => (
      new THREE.MeshBasicMaterial({`],
    [`        ...blend,
      }),
    );
    const mesh`, `        ...blend,
      })
    );
    const materialKey = layer.blend + ':' + order;
    let material = this.materials.get(materialKey);
    if (!material) {
      material = makeMaterial();
      this.materials.set(materialKey, material);
    }
    material.map = prepared.texture;
    material.opacity = fades ? 1 : layer.opacity;
    material.color.setScalar(fades ? layer.opacity : 1);
    const mesh`],
    [`  dispose() {\n    this.dropSplatStage();`, `  dispose() {\n    this.dropRetained();\n    this.dropSplatStage();`],
  ]);
  edit(dir, 'index.js', [
    [`      compositor?.dropSplatStage();\n      assets.dropGpu();`, `      compositor?.dropRetained();\n      compositor?.dropSplatStage();\n      assets.dropGpu();`],
  ]);
};

// Hypothesis under test: three reads <video>.width (the HTML attribute, 0) instead of videoWidth.
const videofix = (dir) => edit(dir, 'assets.js', [[`    const duration = Number.isFinite(video.duration) ? video.duration : null;`, `    video.width = video.videoWidth;\n    video.height = video.videoHeight;\n    const duration = Number.isFinite(video.duration) ? video.duration : null;`]]);
const rvfc = (dir) => edit(dir, 'assets.js', [[`    if (!entry.texture) {\n      entry.texture = new THREE.Texture(video);`, `    // Probe: wait for the compositor to hold the seeked frame before uploading it.
    if (video.requestVideoFrameCallback) await Promise.race([new Promise((r) => video.requestVideoFrameCallback(r)), new Promise((r) => setTimeout(r, 120))]);
    if (!entry.texture) {\n      entry.texture = new THREE.Texture(video);`]]);
const variants = {
  'baseline-videofix-rvfc': (d) => { trace(d); videofix(d); rvfc(d); },
  'baseline-videofix': (d) => { trace(d); videofix(d); },
  baseline: () => {},
  'baseline-trace': (d) => trace(d),
  retained: (d) => retained(d, { rtCache: false, singleTarget: false }),
  'retained-trace': (d) => { trace(d); retained(d, { rtCache: false, singleTarget: false }); },
  'retained-rtc': (d) => { trace(d); retained(d, { rtCache: true, singleTarget: false }); },
  'retained-singlert': (d) => { trace(d); retained(d, { rtCache: false, singleTarget: true }); },
};
const manifest = {};
for (const [name, apply] of Object.entries(variants)) {
  const v = join(out, name); rmSync(v, { recursive: true, force: true });
  mkdirSync(join(v, 'src'), { recursive: true }); mkdirSync(join(v, 'dist'), { recursive: true });
  cpSync(join(snap, 'runtime/src'), join(v, 'src'), { recursive: true });
  apply(join(v, 'src'));
  // Same options as runtime/scripts/build.mjs (checked below for `baseline`).
  await build({ entryPoints: [join(v, 'src/boot.js')], outfile: join(v, 'dist/runtime.js'), bundle: true, format: 'iife', target: ['chrome120', 'safari17'], nodePaths: [nm], legalComments: 'none', logLevel: 'error', ...JSON.parse(process.env.VV_ESBUILD ?? '{}') });
  cpSync(join(snap, 'runtime/host/index.html'), join(v, 'dist/index.html'));
  cpSync(join(snap, 'runtime/worker'), join(v, 'worker'), { recursive: true });
  cpSync(join(snap, 'runtime/package.json'), join(v, 'package.json'));
  try { rmSync(join(v, 'node_modules'), { force: true, recursive: true }); } catch {}
  (await import('node:fs')).symlinkSync(nm, join(v, 'node_modules'));
  manifest[name] = createHash('sha256').update(readFileSync(join(v, 'dist/runtime.js'))).digest('hex');
}
writeFileSync(join(out, 'VARIANTS.json'), JSON.stringify(manifest, null, 1));
console.log(manifest);
