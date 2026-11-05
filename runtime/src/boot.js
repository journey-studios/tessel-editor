// Page bootstrap: picks the transport the host provided and exposes `TesselRuntime.receive`.
//   Node worker  -> window.__tesselPost (puppeteer exposeFunction)
//   WebView      -> window.ipc.postMessage (wry)
// Assets are fetched from the page's own origin: `/asset?p=<absolute path>`.

import { createRuntime } from './index.js';
import { RUNTIME_VERSION } from './protocol.js';

const transport = () => {
  if (typeof window.__tesselPost === 'function') {
    return (message) => window.__tesselPost(JSON.stringify(message));
  }
  if (window.ipc && typeof window.ipc.postMessage === 'function') {
    return (message) => window.ipc.postMessage(JSON.stringify(message));
  }
  return (message) => {
    // No host attached (page opened by hand): keep the last messages inspectable.
    (window.__tesselOutbox ??= []).push(message);
  };
};

const canvas = document.getElementById('viewport');
const runtime = createRuntime({
  post: transport(),
  assetUrl: (path) => `/asset?p=${encodeURIComponent(path)}`,
  canvas,
});

window.TesselRuntime = { version: RUNTIME_VERSION, receive: runtime.receive, debug: runtime.debug };

// Tells a host that loads the page itself (the desktop WebView) that `receive` is ready, so it
// does not send into a page that is still loading. The Node worker drops this message.
transport()({ v: 1, type: 'page.ready', runtime: { version: RUNTIME_VERSION } });
