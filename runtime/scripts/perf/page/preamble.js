// Runs before runtime.js. Reads the run configuration (`?cfg=<base64 JSON>`), taps the bridge
// the runtime posts to, and optionally wraps WebGL calls to count them.
//
// Counting and timing are never combined: wrapping every GL call perturbs the time it measures,
// so `cfg.count` runs report call counts and their own (inflated) durations, and timing runs
// install nothing.
(() => {
  const cfg = JSON.parse(atob(new URLSearchParams(location.search).get('cfg')));
  window.__cfg = cfg;
  window.__msgs = [];
  window.__waiters = [];
  window.__errs = [];
  window.__tesselPost = (raw) => {
    const message = JSON.parse(raw);
    // Native bridge mode: the host drives the frames and wants the answers on its own IPC.
    if (cfg.bridge && window.webkit && message.type === 'frame.done') {
      window.webkit.messageHandlers.ipc.postMessage(raw);
    }
    const index = window.__waiters.findIndex((waiter) => waiter.predicate(message));
    if (index >= 0) {
      const [waiter] = window.__waiters.splice(index, 1);
      waiter.resolve(message);
    } else {
      window.__msgs.push(message);
    }
  };
  addEventListener('error', (event) => window.__errs.push(String(event.message)));
  addEventListener('unhandledrejection', (event) => window.__errs.push(String(event.reason)));

  window.__counts = {};
  window.__times = {};
  if (!cfg.count) return;
  const proto = WebGL2RenderingContext.prototype;
  const names = [
    'createProgram', 'linkProgram', 'compileShader', 'createShader', 'deleteShader', 'deleteProgram',
    'getProgramParameter', 'getShaderParameter', 'getShaderSource', 'getProgramInfoLog', 'getActiveUniform',
    'getActiveAttrib', 'getUniformLocation', 'getAttribLocation', 'texImage2D', 'texSubImage2D', 'texStorage2D',
    'createTexture', 'deleteTexture', 'createBuffer', 'deleteBuffer', 'bufferData', 'bufferSubData',
    'createVertexArray', 'deleteVertexArray', 'renderbufferStorageMultisample', 'createFramebuffer',
    'deleteFramebuffer', 'createRenderbuffer', 'deleteRenderbuffer', 'blitFramebuffer', 'finish', 'flush',
    'readPixels', 'drawElements', 'drawArrays', 'drawArraysInstanced', 'drawElementsInstanced', 'clear',
    'fenceSync', 'clientWaitSync', 'getBufferSubData', 'useProgram', 'getParameter', 'getExtension', 'getError',
  ];
  for (const name of names) {
    const original = proto[name];
    if (!original) continue;
    proto[name] = function counted(...args) {
      const started = performance.now();
      try {
        return original.apply(this, args);
      } finally {
        window.__counts[name] = (window.__counts[name] ?? 0) + 1;
        window.__times[name] = (window.__times[name] ?? 0) + performance.now() - started;
      }
    };
  }
})();
