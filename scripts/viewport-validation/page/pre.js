// Runs before runtime.js: config, bridge tap, optional GL counters, picks the runtime variant.
const cfg = JSON.parse(atob(new URLSearchParams(location.search).get('cfg')));
window.__cfg = cfg; if (cfg.exp) window.__exp = cfg.exp; if (cfg.noRaf) window.__vvNoRaf = true;
window.__msgs = []; window.__waiters = []; window.__errs = [];
window.__tesselPost = (raw) => { const m = JSON.parse(raw); const i = __waiters.findIndex((w) => w.pred(m)); if (i >= 0) { const [w] = __waiters.splice(i, 1); w.resolve(m); } else __msgs.push(m); };
addEventListener('error', (e) => __errs.push(String(e.message)));
addEventListener('unhandledrejection', (e) => __errs.push(String(e.reason?.message ?? e.reason)));
window.__counts = {}; window.__times = {};
if (cfg.count) { const P = WebGL2RenderingContext.prototype; for (const name of Object.getOwnPropertyNames(P)) { if (!/^(createProgram|linkProgram|compileShader|deleteProgram|getProgramInfoLog|texImage2D|texSubImage2D|texStorage2D|texImage3D|texSubImage3D|bufferData|blitFramebuffer|readPixels|drawArraysInstanced|drawElements|drawArrays|fenceSync|renderbufferStorageMultisample)$/.test(name)) continue; const f = P[name]; P[name] = function (...a) { const t = performance.now(); try { return f.apply(this, a); } finally { __counts[name] = (__counts[name] ?? 0) + 1; __times[name] = (__times[name] ?? 0) + performance.now() - t; } }; } }
document.write(`<script src="/rt/${cfg.variant}/runtime.js"><\/script>`);
