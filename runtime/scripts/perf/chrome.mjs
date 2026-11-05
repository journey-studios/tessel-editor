// Runs a list of measurement runs in headless Chrome, one fresh page per run (cold page each time).
// GPU backend: `gpu` (Metal) or `swiftshader` (software), the same flags as the headless worker.
import { appendFileSync, existsSync, readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { loadavg } from 'node:os';
import { dirname, extname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

import puppeteer from 'puppeteer-core';

import { findChrome, glFlags } from '../../worker/chrome.mjs';

const pageDir = join(dirname(fileURLToPath(import.meta.url)), 'page');
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.png': 'image/png', '.json': 'application/json' };

/** `plan` is the same shape the WebKit host reads: { assets, bundles: {name: path}, runs: [...] }. */
export const runChrome = async ({ plan, out, gl }) => {
  const assetsRoot = resolve(plan.assets) + sep;
  let bundle = '';
  const server = createServer((request, response) => {
    const url = new URL(request.url, 'http://127.0.0.1');
    let file = null;
    if (url.pathname === '/' || url.pathname === '/index.html') file = join(pageDir, 'index.html');
    else if (url.pathname === '/runtime.js') file = bundle;
    else if (url.pathname === '/asset') {
      const asked = resolve(url.searchParams.get('p') ?? '');
      if (asked.startsWith(assetsRoot)) file = asked;
    } else if (url.pathname.startsWith('/perf/') && !url.pathname.includes('..')) {
      file = join(pageDir, url.pathname.slice('/perf/'.length));
    }
    if (!file || !existsSync(file)) {
      response.writeHead(404);
      response.end();
      return;
    }
    response.writeHead(200, { 'Content-Type': TYPES[extname(file)] ?? 'application/octet-stream', 'Cache-Control': 'no-store' });
    response.end(readFileSync(file));
  });
  await new Promise((done) => server.listen(0, '127.0.0.1', done));
  const { port } = server.address();
  const browser = await puppeteer.launch({
    executablePath: findChrome(),
    headless: true,
    args: [...glFlags(gl), '--disable-background-timer-throttling', '--disable-renderer-backgrounding', '--no-first-run', '--window-size=480,270'],
  });
  const engine = `chrome-${gl}`;
  const version = await browser.version();
  try {
    for (const run of plan.runs) {
      bundle = plan.bundles[run.bundle ?? 'current'];
      const page = await browser.newPage();
      const started = Date.now();
      const record = await new Promise((done) => {
        const timer = setTimeout(() => done({ fatal: `timeout after ${run.timeout_s ?? 240}s` }), (run.timeout_s ?? 240) * 1000);
        page
          .exposeFunction('__benchDone', (result) => {
            clearTimeout(timer);
            done(result);
          })
          .then(() => {
            const cfg = Buffer.from(JSON.stringify({ ...run, assetsDir: plan.assets })).toString('base64');
            return page.goto(`http://127.0.0.1:${port}/index.html?cfg=${encodeURIComponent(cfg)}`);
          })
          .catch((error) => done({ fatal: String(error) }));
      });
      appendFileSync(out, `${JSON.stringify({ engine, browser: version, run, loadavg: loadavg(), wall_s: (Date.now() - started) / 1000, ...record })}\n`);
      await page.close();
    }
  } finally {
    await browser.close();
    server.close();
  }
};
