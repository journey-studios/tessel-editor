// Runs before the runtime: reads the run configuration (`?cfg=<base64 JSON>`), taps the bridge the
// runtime posts to and loads the runtime variant the run asks for (`/rt/<variant>/runtime.js`).
const config = JSON.parse(atob(new URLSearchParams(location.search).get('cfg')));
window.__cfg = config;
window.__msgs = [];
window.__waiters = [];
window.__errs = [];
window.__tesselPost = (raw) => {
  const message = JSON.parse(raw);
  const index = window.__waiters.findIndex((waiter) => waiter.predicate(message));
  if (index >= 0) {
    const [waiter] = window.__waiters.splice(index, 1);
    waiter.resolve(message);
  } else {
    window.__msgs.push(message);
  }
};
addEventListener('error', (event) => window.__errs.push(String(event.message)));
addEventListener('unhandledrejection', (event) => window.__errs.push(String(event.reason?.message ?? event.reason)));
document.write(`<script src="/rt/${config.variant}/runtime.js"><\/script>`);
