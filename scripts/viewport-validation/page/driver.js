// Bench page: drives the real runtime through its bridge. Plans mirror what crates/core produces.
const A = cfg.assetsDir, W0 = cfg.width ?? 960, H0 = cfg.height ?? 540;
const until = (pred, ms) => { const i = __msgs.findIndex(pred); if (i >= 0) return Promise.resolve(__msgs.splice(i, 1)[0]); return new Promise((resolve, reject) => { const w = { pred, resolve }; __waiters.push(w); if (ms) setTimeout(() => { const k = __waiters.indexOf(w); if (k >= 0) { __waiters.splice(k, 1); reject(new Error('timeout')); } }, ms); }); };
const send = (m) => TesselRuntime.receive(JSON.stringify({ v: 1, ...m }));
const T0 = { x: 0, y: 0, scale: 1, rotation: 0, crop: [0, 0, 0, 0] };
const still = (id, f) => ({ id, kind: 'still', source: { type: 'file', path: `${A}/${f}` }, meta: {} });
const sceneA = (f, id = 'scene') => ({ id, kind: 'scene', source: { type: 'file', path: `${A}/${f}` }, meta: {} });
const splatA = (f, id = 'splat') => ({ id, kind: 'splat', source: { type: 'file', path: `${A}/${f}` }, meta: { format: 'splat' } });
const img = (asset, extra = {}) => ({ id: `l:${asset}`, kind: 'image', asset, opacity: 1, blend: 'normal', transform: T0, ...extra });
const txt = (content, extra = {}) => ({ id: 'l:t', kind: 'text', opacity: 1, blend: 'normal', transform: { ...T0, y: 30 }, text: { content, size: 8, color: '#ffffff' }, ...extra });
const scl = (cam, t = 0, extra = {}, asset = 'scene') => ({ id: `l:${asset}`, kind: 'scene', asset, opacity: 1, blend: 'normal', transform: T0, camera: cam, scene_time: { seconds: t }, ...extra });
const orbit = (a, r = 4) => ({ position: [Math.sin(a) * r, 0.3, Math.cos(a) * r], target: [0, 0, 0], fov: 50 });
const room = (a) => ({ position: [Math.sin(a) * 1.5, 0.2, Math.cos(a) * 1.5], target: [Math.sin(a + 1.2) * 3, 0, Math.cos(a + 1.2) * 3], fov: 70 });
const PATHS = { still: () => orbit(0), orbit: (i) => orbit(i * 0.05), near: (i) => orbit(i * 0.05, 1.2), dolly: (i) => orbit(0.4, 1.2 + 4 * Math.abs(Math.sin(i * 0.02))), jitter: (i) => orbit(0.3 + 0.002 * Math.sin(i * 0.3)), room: (i) => room(i * 0.03), roomstill: () => room(0) };
const planOf = (i, layers, W = W0, H = H0) => ({ version: 1, canvas: { width: W, height: H }, intent: { mode: 'sequence' }, frame: i, layers, requires: [], warnings: [] });
const stats = (v) => { const s = v.filter((x) => x !== undefined && x !== null).sort((a, b) => a - b); if (!s.length) return null; const q = (p) => s[Math.min(s.length - 1, Math.floor(p * s.length))]; const r = (x) => Number(x.toFixed(3)); return { n: s.length, p50: r(q(0.5)), p95: r(q(0.95)), p99: r(q(0.99)), mean: r(s.reduce((a, b) => a + b, 0) / s.length), max: r(s[s.length - 1]) }; };
const nextRaf = () => new Promise((r) => requestAnimationFrame(r));
const yieldTask = () => new Promise((r) => { const c = new MessageChannel(); c.port1.onmessage = () => r(); c.port2.postMessage(0); });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let nextId = 1;
const memory = () => new Promise((resolve) => { if (!window.__vvCtl) return resolve(null); window.__vvMemReply = resolve; window.__vvCtl(JSON.stringify({ op: 'mem' })); setTimeout(() => resolve(null), 1000); });
const info = () => { const c = TesselRuntime.debug.compositor?.(); if (!c) return null; const m = c.renderer.info; return { textures: m.memory.textures, geometries: m.memory.geometries, programs: m.programs?.length, retained_text: c.texts?.size, retained_targets: c.targets?.size, retained_materials: c.materials?.size, rt_hits: c.rtHits, rt_misses: c.rtMisses }; };
const loadAll = async (assets) => { for (const asset of assets) send({ type: 'asset.load', asset }); for (const asset of assets) { const m = await until((x) => x.type === 'asset.state' && x.id === asset.id && x.state !== 'loading'); if (m.state !== 'ready') throw new Error('asset ' + JSON.stringify(m)); } };
const renderOnce = async (layers, { w = W0, h = H0, inspect = false, probes, png, i = 0, timeout } = {}) => {
  const id = nextId++;
  const t0 = performance.now();
  send({ type: 'frame.render', id, revision: id, plan: planOf(i, layers, w, h), output: { width: w, height: h, ...(png ? { png: 'x' } : {}) }, inspect, ...(probes ? { probes } : {}) });
  const done = await until((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === id, timeout);
  done.wall = performance.now() - t0;
  return done;
};

// ---- scenarios for the perf mode
const seqA = (v) => ({ id: 'vid', kind: 'video', source: { type: 'frame_sequence', pattern: `${A}/video/${v.dir}/x%05d.${v.ext}`, first: 0, count: v.count }, meta: { rate: { num: 24, den: 1 } } });
const fileV = (v) => ({ id: 'vid', kind: 'video', source: { type: 'file', path: `${A}/video/${v.file}` }, meta: { rate: { num: 24, den: 1 } } });
const vidL = (f, off = 0) => ({ id: 'l:v', kind: 'video', asset: 'vid', opacity: 1, blend: 'normal', transform: T0, source_frame: f, source_time: { seconds: (f + off) / 24 } });
const SPL = (name) => [splatA(`${name}.splat`), sceneA(`${name}.json`)];
const SC = {
  image: { assets: () => [still('a', 'still.png')], layers: () => [img('a')] },
  image4k: { assets: () => [still('a', 'still4k.png')], layers: () => [img('a')] },
  image2: { assets: () => [still('a', 'still.png'), still('b', 'still4k.png')], layers: () => [img('a'), img('b', { blend: 'screen', opacity: 0.5 })] },
  text: { assets: () => [], layers: () => [txt('TESSEL EDITOR')] },
  textchange: { assets: () => [], layers: (i) => [txt(`TC ${String(i % 1000).padStart(3, '0')}`)] },
  box: { assets: () => [sceneA('box.json')], layers: (i) => [scl(orbit(0), i / 24)] },
};
const scenario = (name) => { if (SC[name]) return SC[name]; const [s, path] = name.split(':'); return { assets: () => SPL(s), layers: (i) => [scl(PATHS[path](i), 0)] }; };

const perf = async (res) => {
  const sc = scenario(cfg.scenario);
  const canvas = document.getElementById('viewport');
  const gl = canvas.getContext('webgl2');
  const t0 = performance.now(); await loadAll(sc.assets()); res.asset_load_ms = performance.now() - t0;
  res.mem_before = await memory();
  const render = async (i) => {
    const d = await renderOnce(sc.layers(i), { i, timeout: 120000 });
    if (d.type === 'frame.failed') throw new Error('frame ' + JSON.stringify(d.error));
    const row = { wall: d.wall, render: d.timings.render_ms, settle: d.timings.settle_ms, final: d.timings.final_draw_ms, sort: d.timings.spark?.settle_ms, update: d.timings.spark?.update_ms, attempts: d.timings.spark?.attempts };
    if (cfg.gpu === 'read1x1') { const a = performance.now(); const px = new Uint8Array(4); gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px); row.gpu = performance.now() - a; }
    if (cfg.gpu === 'fence') { const a = performance.now(); const s = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0); gl.flush(); for (;;) { const st = gl.clientWaitSync(s, 0, 0); if (st === gl.ALREADY_SIGNALED || st === gl.CONDITION_SATISFIED || st === gl.WAIT_FAILED) break; await yieldTask(); } gl.deleteSync(s); row.gpu = performance.now() - a; }
    return row;
  };
  const warm = cfg.warmup ?? 10, n = cfg.frames ?? 300; const cold = [];
  for (let i = 0; i < warm; i++) { cold.push(await render(i)); await nextRaf(); }
  res.cold_first = cold[0]; res.cold_second = cold[1];
  for (const k of Object.keys(__counts)) { delete __counts[k]; delete __times[k]; }
  const rows = []; const start = performance.now();
  for (let i = warm; i < warm + n; i++) { const row = await render(i); const a = performance.now(); await nextRaf(); row.raf_lag = performance.now() - a; rows.push(row); }
  res.total_ms = performance.now() - start; res.fps = n / (res.total_ms / 1000);
  for (const k of ['wall', 'render', 'settle', 'final', 'sort', 'update', 'attempts', 'gpu', 'raf_lag']) res[k] = stats(rows.map((r) => r[k]));
  if (cfg.count) { res.per_frame = {}; res.ms_per_frame = {}; for (const k of Object.keys(__counts).sort()) { res.per_frame[k] = +(__counts[k] / n).toFixed(2); res.ms_per_frame[k] = +(__times[k] / n).toFixed(3); } }
  res.mem_after = await memory(); res.info = info();
};

// ---- exact pixels: every step's hash; PNGs stay on disk for the cross-stack comparison
const hashes = async (res) => {
  const sc = scenario(cfg.scenario); await loadAll(sc.assets()); res.hashes = [];
  for (const [k, i] of cfg.frames_list.entries()) {
    const probes = [[10, 10], [W0 >> 1, H0 >> 1], [W0 >> 2, H0 >> 2], [W0 - 10, H0 - 10]];
    const png = cfg.png && (cfg.png === 'all' || cfg.png.includes(k));
    const d = await renderOnce(sc.layers(i), { i, inspect: true, probes, png, timeout: 300000 });
    if (png && d.png_data) { await window.__vvCtl(JSON.stringify({ op: 'save', name: `${cfg.name}-${k}-${i}.png`, b64: d.png_data })); delete d.png_data; }
    res.hashes.push({ k, i, hash: d.hash ?? JSON.stringify(d.error), mean: d.mean, probes: d.probes, ms: d.timings?.render_ms });
    await nextRaf();
  }
  res.mem_after = await memory(); res.info = info();
};

// ---- lifecycle scripts: a list of steps, compared step by step between runtime variants.
const lose = async (res) => { TesselRuntime.debug.loseContext(); await until((m) => m.type === 'context.lost', 8000); };
const restore = async () => { await sleep(100); TesselRuntime.debug.restoreContext(); await until((m) => m.type === 'context.restored', 8000); };
const hashOf = async (layers, o = {}) => { const d = await renderOnce(layers, { inspect: true, timeout: 180000, ...o }); return d.type === 'frame.done' ? d.hash : `FAILED:${d.error.code}`; };
const reload = async (asset) => { await loadAll([asset]); return 'loaded'; };
const SCRIPTS = {
  image: (s) => [
    ['load', () => loadAll([still('a', 'still.png')])], ['f0', () => s.h([img('a')])], ['f1', () => s.h([img('a', { opacity: .5 })])],
    ['lose', () => lose()], ['restore', () => restore()], ['f0_after_loss', () => s.h([img('a')])], ['f1_after_loss', () => s.h([img('a', { opacity: .5 })])],
    ['swap_same_id', () => reload(still('a', 'still-alt.png'))], ['f_alt', () => s.h([img('a')])], ['swap_back', () => reload(still('a', 'still.png'))], ['f0_back', () => s.h([img('a')])],
    ['release', async () => send({ type: 'asset.release', id: 'a' })], ['f_released', () => s.h([img('a')])], ['reload', () => reload(still('a', 'still.png'))], ['f0_reload', () => s.h([img('a')])],
  ],
  box: (s) => [
    ['load', () => loadAll([sceneA('box.json')])], ['t0', () => s.h([scl(orbit(0), 0)])], ['t1', () => s.h([scl(orbit(0), 1)])], ['c1', () => s.h([scl(orbit(1), 1)])],
    ['lose', () => lose()], ['restore', () => restore()], ['t0_after_loss', () => s.h([scl(orbit(0), 0)])], ['c1_after_loss', () => s.h([scl(orbit(1), 1)])],
    ['swap_scene', () => reload(sceneA('static.json'))], ['static', () => s.h([scl(orbit(0), 0)])], ['swap_back', () => reload(sceneA('box.json'))], ['t0_back', () => s.h([scl(orbit(0), 0)])],
    ['lose2', () => lose()], ['restore2', () => restore()], ['t0_after_loss2', () => s.h([scl(orbit(0), 0)])],
  ],
  splat: (s) => { const n = cfg.splat ?? 'gauss100k'; return [
    ['load', () => loadAll(SPL(n))], ['c0', () => s.h([scl(orbit(0))])], ['c1', () => s.h([scl(orbit(1))])],
    ['lose', () => lose()], ['restore', () => restore()], ['c0_after_loss', () => s.h([scl(orbit(0))])], ['c1_after_loss', () => s.h([scl(orbit(1))])],
    ['swap_splat', () => reload(splatA('gauss100k-alt.splat'))], ['alt_c0', () => s.h([scl(orbit(0))])], ['alt_c1', () => s.h([scl(orbit(1))])],
    ['swap_back', () => reload(splatA(`${n}.splat`))], ['c0_back', () => s.h([scl(orbit(0))])],
    ['release_scene', async () => send({ type: 'asset.release', id: 'scene' })], ['released', () => s.h([scl(orbit(0))])], ['reload_scene', () => reload(sceneA(`${n}.json`))], ['c0_reload', () => s.h([scl(orbit(0))])],
    ['release_splat', async () => send({ type: 'asset.release', id: 'splat' })], ['released_splat', () => s.h([scl(orbit(0))])], ['reload_splat', () => reload(splatA(`${n}.splat`))], ['c0_reload2', () => s.h([scl(orbit(0))])],
    ['lose2', () => lose()], ['restore2', () => restore()], ['c1_after_loss2', () => s.h([scl(orbit(1))])], ['c0_after_loss2', () => s.h([scl(orbit(0))])],
  ]; },
  rtc: (s) => [
    ['load', () => loadAll([...SPL('gauss100k'), sceneA('animsplat.json', 'anim'), still('a', 'still.png')])],
    ['c0', () => s.h([scl(orbit(0))])], ['c0_again', () => s.h([scl(orbit(0))])], ['c1_camera', () => s.h([scl(orbit(1))])], ['c0_back', () => s.h([scl(orbit(0))])],
    ['fov', () => s.h([scl({ ...orbit(0), fov: 35 })])], ['opacity', () => s.h([scl(orbit(0), 0, { opacity: .5 })])], ['screen', () => s.h([scl(orbit(0), 0, { blend: 'screen' })])],
    ['transform', () => s.h([scl(orbit(0), 0, { transform: { ...T0, scale: .5, rotation: 30, x: 10 } })])], ['crop', () => s.h([scl(orbit(0), 0, { transform: { ...T0, crop: [10, 20, 5, 0] } })])],
    ['static_time_a', () => s.h([scl(orbit(0), 1)])], ['static_time_b', () => s.h([scl(orbit(0), 2)])],
    ['anim_t0', () => s.h([scl(orbit(0), 0, {}, 'anim')])], ['anim_t1', () => s.h([scl(orbit(0), 1, {}, 'anim')])], ['anim_t0_again', () => s.h([scl(orbit(0), 0, {}, 'anim')])],
    ['size_w', () => s.h([scl(orbit(0))], { w: 640, h: 360 })], ['size_back', () => s.h([scl(orbit(0))])],
    ['unrelated_load', () => reload(still('b', 'still-alt.png'))], ['after_unrelated', () => s.h([scl(orbit(0))])],
    ['swap_splat', () => reload(splatA('gauss100k-alt.splat'))], ['alt', () => s.h([scl(orbit(0))])], ['alt_again', () => s.h([scl(orbit(0))])], ['swap_back', () => reload(splatA('gauss100k.splat'))], ['orig', () => s.h([scl(orbit(0))])],
    ['lose', () => lose()], ['restore', () => restore()], ['after_loss', () => s.h([scl(orbit(0))])], ['after_loss_again', () => s.h([scl(orbit(0))])],
    ['png', () => s.h([scl(orbit(0))], { png: true })], ['with_image', () => s.h([img('a'), scl(orbit(0), 0, { opacity: .5 })])],
  ],
  two: (s) => [
    ['load', () => loadAll([...SPL('gauss100k'), sceneA('box.json', 'bx'), sceneA('static.json', 'st')])],
    ['two_boxes', () => s.h([scl(orbit(0), 0, { transform: { ...T0, x: -25, scale: .5 } }, 'bx'), scl(orbit(1), 0, { transform: { ...T0, x: 25, scale: .5 } }, 'st')])],
    ['one_box', () => s.h([scl(orbit(0), 0, { transform: { ...T0, x: -25, scale: .5 } }, 'bx')])],
    ['same_scene_twice', () => s.h([scl(orbit(0), 0, { transform: { ...T0, x: -25, scale: .5 } }, 'bx'), scl(orbit(2), 0, { transform: { ...T0, x: 25, scale: .5 } }, 'bx')])],
    ['two_splat_layers', () => s.h([scl(orbit(0), 0, { transform: { ...T0, x: -25, scale: .5 } }), scl(orbit(1), 0, { transform: { ...T0, x: 25, scale: .5 } })])],
    ['two_splat_again', () => s.h([scl(orbit(0), 0, { transform: { ...T0, x: -25, scale: .5 } }), scl(orbit(1), 0, { transform: { ...T0, x: 25, scale: .5 } })])],
    ['one_splat', () => s.h([scl(orbit(0))])],
  ],
  cancel: (s) => { const big = cfg.splat ?? 'gauss1m'; const issue = (layers, extra = {}) => { const id = nextId++; send({ type: 'frame.render', id, revision: id, plan: planOf(id, layers), output: { width: W0, height: H0 }, inspect: true, ...extra }); return id; };
    const outcome = async (id) => { const d = await until((m) => (m.type === 'frame.done' || m.type === 'frame.failed') && m.id === id, 180000); return d.type === 'frame.done' ? d.hash : `FAILED:${d.error.code}`; };
    return [
    ['load', () => loadAll([...SPL(big), still('a', 'still.png')])], ['base_c0', () => s.h([scl(orbit(0))])], ['base_c1', () => s.h([scl(orbit(1))])],
    ['cancel_then_new', async () => { const a = issue([scl(orbit(2))]); send({ type: 'frame.cancel', id: a }); const b = issue([scl(orbit(0))]); return [await outcome(a), await outcome(b)]; }],
    ['supersede', async () => { const a = issue([scl(orbit(2))]); const b = issue([scl(orbit(1))]); return [await outcome(a), await outcome(b)]; }],
    ['supersede_during_sort', async () => { const a = issue([scl(orbit(2))]); await sleep(8); const b = issue([scl(orbit(0))]); return [await outcome(a), await outcome(b)]; }],
    ['abort_multilayer', async () => { const a = issue([img('a'), scl(orbit(3)), scl(orbit(1), 0, { opacity: .4 })]); await sleep(4); const b = issue([img('a')]); return [await outcome(a), await outcome(b)]; }],
    ['c0_after_cancels', () => s.h([scl(orbit(0))])], ['c1_after_cancels', () => s.h([scl(orbit(1))])],
    ['release_during_frame', async () => { const a = issue([scl(orbit(4))]); await sleep(1); send({ type: 'asset.release', id: 'splat' }); return await outcome(a); }],
    ['reload_splat', () => reload(splatA(`${big}.splat`))], ['c0_after_release', () => s.h([scl(orbit(0))])],
    ['release_during_load', async () => { send({ type: 'asset.load', asset: still('late', 'still4k.png') }); send({ type: 'asset.release', id: 'late' }); const st = await until((m) => m.type === 'asset.state' && m.id === 'late' && m.state !== 'loading', 20000); return `${st.state}:${st.error?.code ?? ''}`; }],
    ['double_load_old_result', async () => { send({ type: 'asset.load', asset: still('d', 'still.png') }); send({ type: 'asset.load', asset: still('d', 'still-alt.png') }); const x = await until((m) => m.type === 'asset.state' && m.id === 'd' && m.state !== 'loading', 20000); const y = await until((m) => m.type === 'asset.state' && m.id === 'd' && m.state !== 'loading', 20000); return `${x.state}:${x.error?.code ?? ''},${y.state}:${y.error?.code ?? ''}`; }],
    ['d_is_new', () => s.h([img('d')])], ['alt_direct', async () => { await loadAll([still('e', 'still-alt.png')]); return s.h([img('e')]); }],
    ['lose_during_frame', async () => { const a = issue([scl(orbit(5))]); await sleep(2); TesselRuntime.debug.loseContext(); const o = await outcome(a); await until((m) => m.type === 'context.lost', 8000); return o; }],
    ['frame_while_lost', () => s.h([img('a')])], ['restore', () => restore()], ['c0_after_loss', () => s.h([scl(orbit(0))])],
  ]; },
  limits: (s) => { const growth = []; const snap = async (label) => { growth.push({ label, info: info(), mem: await memory() }); return 'ok'; };
    s.res.growth = growth; const seq = (i) => ({ id: 'seq', kind: 'video', source: { type: 'frame_sequence', pattern: `${A}/video/seq-720-jpg/x%05d.jpg`, first: 0, count: 120 }, meta: { rate: { num: 24, den: 1 } } });
    return [
    ['load', () => loadAll([still('a', 'still.png'), seq(), sceneA('box.json', 'bx')])], ['start', () => snap('start')],
    ['text_600_unique', async () => { for (let i = 0; i < 600; i++) await renderOnce([txt(`N${i}`)]); return snap('after 600 distinct texts'); }],
    ['text_600_more', async () => { for (let i = 600; i < 1200; i++) await renderOnce([txt(`N${i}`)]); return snap('after 1200 distinct texts'); }],
    ['video_miss_240', async () => { for (let i = 0; i < 240; i++) await renderOnce([{ id: 'l:v', kind: 'video', asset: 'seq', opacity: 1, blend: 'normal', transform: T0, source_frame: i % 120, source_time: { seconds: i / 24 } }]); return snap('after 240 video frames'); }],
    ['scene_300', async () => { for (let i = 0; i < 300; i++) await renderOnce([scl(orbit(i * .1), i / 24, {}, 'bx')]); return snap('after 300 scene frames'); }],
    ['blend_cycle', async () => { for (let i = 0; i < 200; i++) await renderOnce([img('a', { blend: ['normal', 'multiply', 'screen', 'darken', 'lighten'][i % 5], opacity: .5 })]); return snap('after blend cycle'); }],
    ['sizes', async () => { for (let i = 0; i < 60; i++) await renderOnce([scl(orbit(0), 0, {}, 'bx')], { w: 320 + (i % 20) * 16, h: 180 + (i % 20) * 9 }); return snap('after 60 size changes'); }],
    ['stills_churn', async () => { const hold = []; for (let i = 0; i < 40; i++) { const id = `c${i}`; await loadAll([still(id, 'still.png')]); await renderOnce([img(id)]); hold.push(id); while (hold.length > 16) send({ type: 'asset.release', id: hold.shift() }); } return snap('after 40 stills, 16 kept (RETAINED_ASSETS)'); }],
    ['stills_4k_16', async () => { const hold = []; for (let i = 0; i < 16; i++) { const id = `k${i}`; await loadAll([still(id, 'still4k.png')]); await renderOnce([img(id)]); hold.push(id); } await snap('16 4K stills drawn and retained'); hold.forEach((id) => send({ type: 'asset.release', id })); await nextRaf(); return snap('16 4K stills released'); }],
  ]; },
};
const script = async (res) => { res.steps = {}; const h = (layers, o) => hashOf(layers, o); const ctx = { h, res };
  for (const [name, fn] of SCRIPTS[cfg.script](ctx)) { const t = performance.now(); try { res.steps[name] = await fn(); } catch (e) { res.steps[name] = 'THROW:' + e.message; } res.steps[name + '__ms'] = +(performance.now() - t).toFixed(1); }
  res.mem_after = await memory(); res.info = info(); };

// ---- video: stage timings, exact image at a time, seek traces, prefetch prototype
const decodeCode = (probes) => probes.reduce((v, p, k) => v + ((p[0] + p[1] + p[2]) / 3 > 128 ? 1 << k : 0), 0);
const trace = (kind, n, count, seed = 7) => { let s = seed; const rnd = () => (s = (s * 1664525 + 1013904223) >>> 0) / 4294967296; const out = [];
  if (kind === 'forward') for (let i = 0; i < n; i++) out.push(i % count);
  if (kind === 'backward') for (let i = 0; i < n; i++) out.push(count - 1 - (i % count));
  if (kind === 'random') for (let i = 0; i < n; i++) out.push(Math.floor(rnd() * count));
  if (kind === 'scrub') { let p = Math.floor(count / 2), d = 1; for (let i = 0; i < n; i++) { if (rnd() < 0.08) d = -d; p += d * (1 + Math.floor(rnd() * 4)); if (p < 0) { p = 0; d = 1; } if (p >= count) { p = count - 1; d = -1; } out.push(p); } }
  return out; };
class Prefetcher { // prototype only: directional, bounded in bytes, newest position wins
  constructor(id, { ahead, behind, concurrency, bytes }, w, h) { this.id = id; this.ahead = ahead; this.behind = behind; this.concurrency = concurrency; this.limit = Math.max(16, Math.floor(bytes / (w * h * 4))); window.__vvFrameLimit = this.limit; this.busy = new Set(); this.last = null; this.dir = 1; this.wasted = 0; this.fetched = 0; this.pos = 0; }
  note(i) { if (this.last !== null && i !== this.last) this.dir = Math.sign(i - this.last); this.last = i; this.pos = i; this.pump(); }
  pump() { const store = TesselRuntime.debug.assets; const entry = store.entries.get(this.id); if (!entry) return; const count = entry.sequence.count;
    const want = []; for (let k = 1; k <= this.ahead; k++) want.push(this.pos + this.dir * k); for (let k = 1; k <= this.behind; k++) want.push(this.pos - this.dir * k);
    for (const i of want) { if (this.busy.size >= this.concurrency) break; if (i < 0 || i >= count || entry.frames.has(i) || this.busy.has(i)) continue; this.busy.add(i); this.fetched += 1;
      store.fetchFrame(entry.sequence, entry.sequence.first + i).then((bitmap) => { this.busy.delete(i); if (store.entries.get(this.id) !== entry || entry.frames.has(i)) { bitmap.close(); return; } entry.frames.set(i, { bitmap, texture: TesselRuntime.debug.imageTexture(bitmap) }); while (entry.frames.size > this.limit) { let far = null, dist = -1; for (const f of entry.frames.keys()) { const d = Math.abs(f - this.pos); if (d > dist) { far = f; dist = d; } } const ev = entry.frames.get(far); ev.texture.dispose(); ev.bitmap.close(); entry.frames.delete(far); } this.pump(); }).catch(() => this.busy.delete(i)); } } }
const video = async (res) => {
  const v = cfg.video; const asset = v.file ? fileV(v) : seqA(v);
  const t0 = performance.now();
  send({ type: 'asset.load', asset }); const st = await until((x) => x.type === 'asset.state' && x.id === 'vid' && x.state !== 'loading', 30000);
  res.load = { state: st.state, error: st.error, ms: performance.now() - t0, meta: st.meta };
  if (st.state !== 'ready') return;
  const w = st.meta.width, h = st.meta.height; const cell = w / 20; const fit = Math.min(W0 / w, H0 / h);
  const probes = [...Array(10).keys()].map((k) => [Math.round((k + .5) * cell * fit), Math.round(.5 * cell * fit)]);
  const count = Math.min(v.count, 10000); const off = cfg.timeOffset ?? 0;
  const pf = cfg.prefetch && v.dir ? new Prefetcher('vid', cfg.prefetch, w, h) : null; if (cfg.frameLimit) window.__vvFrameLimit = cfg.frameLimit;
  const seq = trace(cfg.trace, cfg.n ?? 120, count);
  res.seq_len = seq.length;
  if (cfg.paced) { // input at a fixed rate, one frame in flight, newest wins: what a scrub does
    let latest = 0, sent = -1, inputAt = 0; const rows = []; const stamps = []; let idx = 0; const period = 1000 / cfg.paced; const start = performance.now(); let busy = false; let done = false;
    const draw = async () => { if (busy || sent === latest) return; busy = true; const f = seq[latest]; const at = stamps[latest]; const li = latest; sent = latest; const d = await renderOnce([vidL(f, off)], { i: f, timeout: 30000 }); const now = performance.now(); rows.push({ lat: now - at, lag: latest - li, wall: d.wall, ok: d.type === 'frame.done' }); if (pf) pf.note(f); busy = false; draw(); };
    await new Promise((resolve) => { const tick = () => { const t = performance.now(); while (idx < seq.length && start + idx * period <= t) { stamps[idx] = start + idx * period; latest = idx; idx += 1; } draw(); if (idx < seq.length) setTimeout(tick, Math.max(0, start + idx * period - performance.now())); else resolve(); }; tick(); });
    const released = performance.now(); while (busy || sent !== latest) { await sleep(1); draw(); }
    res.paced = { hz: cfg.paced, inputs: seq.length, presented: rows.length, presented_per_s: rows.length / ((performance.now() - start) / 1000), latency: stats(rows.map((r) => r.lat)), wall: stats(rows.map((r) => r.wall)), lag_inputs: stats(rows.map((r) => r.lag)), release_to_final_ms: performance.now() - released, failed: rows.filter((r) => !r.ok).length };
  } else {
    const rows = []; let mismatches = 0; const wrong = [];
    for (const [k, f] of seq.entries()) {
      if (pf) pf.note(f);
      const before = (window.__vvFetch ?? []).length;
      const wantPng = cfg.savePngAt && cfg.savePngAt.includes(k);
      const d = await renderOnce([vidL(f, off)], { i: f, inspect: Boolean(cfg.verify), probes: cfg.verify ? probes : undefined, png: wantPng, timeout: 30000 });
      if (wantPng && d.png_data) await window.__vvCtl(JSON.stringify({ op: 'save', name: `${cfg.name}-${k}-${f}.png`, b64: d.png_data }));
      if (cfg.verify && k < 6) (res.probe_dump ??= []).push({ f, probes: d.probes, mean: d.mean });
      if (d.type !== 'frame.done') { rows.push({ fail: d.error.code }); continue; }
      const row = { k, f, wall: d.wall, render: d.timings.render_ms, final: d.timings.final_draw_ms, miss: (window.__vvFetch ?? []).length > before };
      const fx = (window.__vvFetch ?? []).slice(before); if (fx.length) { row.fetch = fx[fx.length - 1][1]; row.decode = fx[fx.length - 1][2]; }
      if (cfg.verify) { const got = decodeCode(d.probes); if (got !== f) { mismatches += 1; if (wrong.length < 12) wrong.push([f, got]); } }
      rows.push(row); if (!cfg.verify) await nextRaf();
    }
    const ok = rows.filter((r) => !r.fail); res.failed = rows.length - ok.length; const cold = ok.slice(0, 5);
    for (const k of ['wall', 'render', 'final', 'fetch', 'decode']) res[k] = stats(ok.slice(5).map((r) => r[k]));
    res.wall_hit = stats(ok.slice(5).filter((r) => !r.miss).map((r) => r.wall)); res.wall_miss = stats(ok.slice(5).filter((r) => r.miss).map((r) => r.wall));
    res.miss_rate = ok.length ? ok.filter((r) => r.miss).length / ok.length : null;
    if (cfg.verify) { res.verified = ok.length; res.mismatches = mismatches; res.wrong_samples = wrong; }
  }
  if (pf) res.prefetch = { fetched: pf.fetched, limit: pf.limit };
  res.mem_after = await memory(); res.info = info();
};
const caps = async (res) => { const v = document.createElement('video');
  const types = ['video/mp4; codecs="avc1.640028"', 'video/mp4; codecs="hvc1.1.6.L120.90"', 'video/mp4; codecs="av01.0.08M.08"', 'video/webm; codecs="vp8"', 'video/webm; codecs="vp9"', 'video/webm; codecs="vp09.00.10.08"', 'video/webm; codecs="av01.0.08M.08"', 'video/quicktime; codecs="apcn"'];
  res.canPlayType = Object.fromEntries(types.map((t) => [t, v.canPlayType(t)]));
  res.requestVideoFrameCallback = 'requestVideoFrameCallback' in HTMLVideoElement.prototype;
  res.fastSeek = 'fastSeek' in HTMLVideoElement.prototype; res.MediaSource = 'MediaSource' in window; res.ManagedMediaSource = 'ManagedMediaSource' in window;
  res.VideoDecoder = 'VideoDecoder' in window; res.VideoFrame = 'VideoFrame' in window; res.ImageDecoder = 'ImageDecoder' in window; res.OffscreenCanvas = 'OffscreenCanvas' in window; res.isSecureContext = isSecureContext; res.crossOriginIsolated = crossOriginIsolated;
  if (res.VideoDecoder) { res.webcodecs = {}; for (const codec of ['avc1.640028', 'hvc1.1.6.L120.90', 'vp8', 'vp09.00.10.08', 'av01.0.08M.08']) { try { const c = await VideoDecoder.isConfigSupported({ codec, codedWidth: 1920, codedHeight: 1080, hardwareAcceleration: 'prefer-hardware' }); res.webcodecs[codec] = c.supported; } catch (e) { res.webcodecs[codec] = String(e.message); } } }
  const gl = document.getElementById('viewport').getContext('webgl2'); res.ext = (gl.getSupportedExtensions() ?? []).filter((e) => /timer|disjoint|debug_renderer|lose_context/i.test(e)); };

const main = async () => {
  send({ type: 'hello' }); const ready = await until((m) => m.type === 'ready', 30000);
  const res = { cfg: { ...cfg, assetsDir: undefined }, renderer: ready.runtime.renderer, ua: navigator.userAgent, three: ready.runtime.three, spark: ready.runtime.spark };
  let quantum = Infinity, last = performance.now(); const qEnd = last + 30; while (last < qEnd) { const t = performance.now(); if (t > last) { quantum = Math.min(quantum, t - last); last = t; } } res.clock_quantum_ms = quantum;
  const canvas = document.getElementById('viewport'); res.hz_probe = await (async () => { const ts = []; for (let i = 0; i < 12; i++) ts.push(await nextRaf() ?? performance.now()); return ts.length > 2 ? stats(ts.slice(1).map((t, i) => t - ts[i])) : null; })().catch(() => null);
  ({ perf, hash: hashes, script, video, caps })[cfg.mode ?? 'perf'](res).then(() => { res.errors = __errs; window.__vvFinish(res); }).catch((e) => window.__vvFinish({ ...res, fatal: String(e?.stack ?? e), errors: __errs }));
};
main().catch((e) => window.__vvFinish({ cfg, fatal: String(e?.stack ?? e), errors: __errs }));
