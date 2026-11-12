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
  // `cfg.workerlog`: every Worker the page makes (Spark decodes and sorts on them) with what was posted to it and what came back, and
  // every fetch with when its headers and its body arrived. A load that never ends is a message that was posted and never answered, a
  // worker that never started, or a body that never came: this says which. (Listeners are added next to Spark's own; nothing is changed.)
  if (cfg.workerlog) {
    const stamp = () => Math.round(performance.now() * 10) / 10;
    const workers = (window.__workers = []);
    const NativeWorker = window.Worker;
    window.Worker = class extends NativeWorker {
      constructor(...args) {
        super(...args);
        const record = { n: workers.length, created: stamp(), posts: [], recv: [], errors: [] };
        workers.push(record);
        const post = this.postMessage.bind(this);
        this.postMessage = (message, ...rest) => {
          const bytes = message?.args?.fileBytes?.byteLength ?? message?.args?.readback?.byteLength;
          record.posts.push({ t: stamp(), name: message?.name ?? typeof message, id: message?.id, ...(bytes ? { bytes } : {}) });
          return post(message, ...rest);
        };
        this.addEventListener('message', (event) => {
          const data = event.data ?? {};
          record.recv.push({ t: stamp(), id: data.id, kind: data.error !== undefined ? 'error' : data.status !== undefined ? 'status' : 'result' });
        });
        this.addEventListener('error', (event) => record.errors.push({ t: stamp(), kind: 'error', message: String(event.message) }));
        this.addEventListener('messageerror', () => record.errors.push({ t: stamp(), kind: 'messageerror' }));
      }
    };
    const fetches = (window.__fetches = []);
    const nativeFetch = window.fetch.bind(window);
    window.fetch = (input, init) => {
      const record = { url: String(input?.url ?? input).replace(/^.*\//, '').slice(0, 60), start: stamp(), headers: null, body: null };
      fetches.push(record);
      return nativeFetch(input, init).then((response) => {
        record.headers = stamp();
        for (const method of ['arrayBuffer', 'blob']) {
          const original = response[method].bind(response);
          response[method] = () => original().then((value) => ((record.body = stamp()), value));
        }
        return response;
      });
    };
  }
  // `cfg.psokeys`: the set of draw configurations the page has used so far, with the frame each one was first used in. A Metal pipeline
  // state is made lazily the first time a draw uses a configuration it has not seen (program, topology, blend, masks, depth, cull, formats
  // of what is drawn into), at a cost of tens of milliseconds for a large shader: a configuration that is new at the frame of an event
  // is a candidate for the cost of that event. (The vertex layout is not part of the key.) Run it ALONE, like `glcalls`.
  if (cfg.psokeys) {
    const glproto = WebGL2RenderingContext.prototype;
    window.__frame = -1;
    window.__pso = [];
    const seen = new Set();
    const programs = new WeakMap();
    const textures = new WeakMap();
    const renderbuffers = new WeakMap();
    const framebuffers = new WeakMap();
    const units = new Map();
    let nextProgram = 1;
    let program = 0;
    let unit = 33984;
    let drawFb = null;
    let renderbuffer = null;
    const state = { blend: 0, depthTest: 0, cull: 0, stencil: 0, alphaToCoverage: 0, blendFunc: [1, 0, 1, 0], blendEq: [32774, 32774], colorMask: [1, 1, 1, 1], depthMask: 1, depthFunc: 513, cullFace: 1029 };
    const CAPS = { 3042: 'blend', 2929: 'depthTest', 2884: 'cull', 2960: 'stencil', 32926: 'alphaToCoverage' };
    const cubeTarget = (target) => (target >= 34069 && target <= 34074 ? 34067 : target);
    const hook = (name, before) => {
      const original = glproto[name];
      if (typeof original !== 'function') return;
      glproto[name] = function hooked(...args) {
        before.call(this, args);
        return original.apply(this, args);
      };
    };
    const attachments = () => {
      if (!drawFb) return 'canvas';
      const map = framebuffers.get(drawFb);
      if (!map) return 'fb?';
      return [...map.entries()]
        .sort(([a], [b]) => a - b)
        .map(([attachment, item]) => {
          const info = (item.kind === 'texture' ? textures : renderbuffers).get(item.object);
          return `${attachment}:${info?.format ?? '?'}${info?.samples ? `x${info.samples}` : ''}`;
        })
        .join(',');
    };
    const draw = (mode) => {
      const key = [
        `p${program}`, `m${mode}`, `B${state.blend}${state.blendFunc.join('.')}${state.blendEq.join('.')}`, `W${state.colorMask.join('')}`,
        `D${state.depthTest}${state.depthMask}${state.depthFunc}`, `C${state.cull}${state.cullFace}`, `S${state.stencil}A${state.alphaToCoverage}`, `fb(${attachments()})`,
      ].join('|');
      if (seen.has(key)) return;
      seen.add(key);
      window.__pso.push({ f: window.__frame, key, at: Number(performance.now().toFixed(1)) });
    };
    hook('useProgram', ([value]) => {
      if (value && !programs.has(value)) programs.set(value, nextProgram++);
      program = value ? programs.get(value) : 0;
    });
    hook('activeTexture', ([value]) => {
      unit = value;
    });
    hook('bindTexture', ([target, texture]) => {
      units.set(`${unit}:${target}`, texture);
    });
    const formatOf = (target, format, extra = {}) => {
      const texture = units.get(`${unit}:${cubeTarget(target)}`);
      if (texture) textures.set(texture, { format, ...extra });
    };
    hook('texImage2D', ([target, , format]) => formatOf(target, format));
    hook('texStorage2D', ([target, , format]) => formatOf(target, format));
    hook('texImage3D', ([target, , format]) => formatOf(target, format));
    hook('texStorage3D', ([target, , format]) => formatOf(target, format));
    hook('bindRenderbuffer', ([, value]) => {
      renderbuffer = value;
    });
    hook('renderbufferStorage', ([, format]) => {
      if (renderbuffer) renderbuffers.set(renderbuffer, { format, samples: 0 });
    });
    hook('renderbufferStorageMultisample', ([, samples, format]) => {
      if (renderbuffer) renderbuffers.set(renderbuffer, { format, samples });
    });
    hook('bindFramebuffer', ([target, value]) => {
      if (target === 36160 || target === 36009) drawFb = value;
    });
    const attach = (target, attachment, item) => {
      if (target !== 36160 && target !== 36009) return;
      if (!drawFb) return;
      if (!framebuffers.has(drawFb)) framebuffers.set(drawFb, new Map());
      if (item.object) framebuffers.get(drawFb).set(attachment, item);
      else framebuffers.get(drawFb).delete(attachment);
    };
    hook('framebufferTexture2D', ([target, attachment, , texture]) => attach(target, attachment, { kind: 'texture', object: texture }));
    hook('framebufferTextureLayer', ([target, attachment, texture]) => attach(target, attachment, { kind: 'texture', object: texture }));
    hook('framebufferRenderbuffer', ([target, attachment, , value]) => attach(target, attachment, { kind: 'renderbuffer', object: value }));
    hook('enable', ([cap]) => {
      if (CAPS[cap]) state[CAPS[cap]] = 1;
    });
    hook('disable', ([cap]) => {
      if (CAPS[cap]) state[CAPS[cap]] = 0;
    });
    hook('blendFunc', ([a, b]) => {
      state.blendFunc = [a, b, a, b];
    });
    hook('blendFuncSeparate', (args) => {
      state.blendFunc = args.slice(0, 4);
    });
    hook('blendEquation', ([a]) => {
      state.blendEq = [a, a];
    });
    hook('blendEquationSeparate', ([a, b]) => {
      state.blendEq = [a, b];
    });
    hook('colorMask', (args) => {
      state.colorMask = args.slice(0, 4).map(Number);
    });
    hook('depthMask', ([value]) => {
      state.depthMask = Number(value);
    });
    hook('depthFunc', ([value]) => {
      state.depthFunc = value;
    });
    hook('cullFace', ([value]) => {
      state.cullFace = value;
    });
    for (const name of ['drawArrays', 'drawElements', 'drawArraysInstanced', 'drawElementsInstanced', 'drawRangeElements']) hook(name, ([mode]) => draw(mode));
  }
  // `cfg.glcalls` (ms): every WebGL call that took at least that long, with the frame in flight (`window.__frame`, set by the
  // driver; -1 between frames). All methods are wrapped, so run it ALONE (never with `count`, never to compare totals): it says
  // WHICH calls are long and when, the figures of the other runs say how long a frame takes.
  if (cfg.glcalls) {
    const threshold = Number(cfg.glcalls) || 1;
    const glproto = WebGL2RenderingContext.prototype;
    window.__frame = -1;
    window.__slow = [];
    const describe = (args) =>
      args.slice(0, 9).map((arg) => (typeof arg === 'number' ? arg : arg && typeof arg === 'object' ? (arg.constructor?.name ?? 'object') : typeof arg));
    // Calls that make something new on the GPU are recorded whatever they take (they are rare, and a program that is linked or a
    // texture that is allocated at the frame of an event is the finding even when the call itself returns at once).
    const makes = new Set([
      'createProgram', 'linkProgram', 'createShader', 'compileShader', 'texImage3D', 'texStorage3D', 'texStorage2D', 'texImage2D',
      'createTexture', 'createFramebuffer', 'createRenderbuffer', 'renderbufferStorageMultisample', 'renderbufferStorage',
    ]);
    for (const name of Object.getOwnPropertyNames(glproto)) {
      const original = Object.getOwnPropertyDescriptor(glproto, name)?.value;
      if (typeof original !== 'function' || name === 'constructor') continue;
      glproto[name] = function timed(...args) {
        const started = performance.now();
        try {
          return original.apply(this, args);
        } finally {
          const ms = performance.now() - started;
          const rare = makes.has(name);
          if ((rare || ms >= threshold) && window.__slow.length < 4000) {
            window.__slow.push({ f: window.__frame, name, ms: Number(ms.toFixed(1)), rare, args: describe(args) });
          }
        }
      };
    }
  }
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
