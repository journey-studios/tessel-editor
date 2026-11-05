// node vv-chrome.mjs <runs.json> <out.jsonl> <page-dir> <rt-dir> <assets-dir> <png-dir> <node_modules>  (SW=1 for SwiftShader)
import { createServer } from 'node:http'; import { readFileSync, appendFileSync, existsSync, statSync, createReadStream, writeFileSync } from 'node:fs'; import { extname, join } from 'node:path';
const [runsF, out, pageDir, rtDir, assetsDir, pngDir, nm] = process.argv.slice(2);
const puppeteer = (await import(join(nm, 'puppeteer-core/lib/puppeteer/puppeteer-core.js'))).default;
const T = { '.html': 'text/html', '.js': 'text/javascript', '.png': 'image/png', '.jpg': 'image/jpeg', '.webp': 'image/webp', '.json': 'application/json', '.mp4': 'video/mp4', '.webm': 'video/webm' };
const server = createServer((q, s) => { const u = new URL(q.url, 'http://x'); let f;
  if (u.pathname === '/asset') f = u.searchParams.get('p'); else if (u.pathname.startsWith('/rt/')) f = join(rtDir, u.pathname.split('/')[2], 'dist/runtime.js'); else if (u.pathname.startsWith('/page/')) f = join(pageDir, u.pathname.slice(6)); else f = join(pageDir, 'index.html');
  if (!existsSync(f)) { s.writeHead(404); return s.end(); }
  const size = statSync(f).size; const h = { 'Content-Type': T[extname(f)] ?? 'application/octet-stream', 'Accept-Ranges': 'bytes', 'Cache-Control': 'no-store' }; const r = q.headers.range;
  if (r?.startsWith('bytes=')) { const [a, b] = r.slice(6).split('-'); const st = Number(a || 0), en = b ? Math.min(Number(b), size - 1) : size - 1; s.writeHead(206, { ...h, 'Content-Range': `bytes ${st}-${en}/${size}`, 'Content-Length': en - st + 1 }); return createReadStream(f, { start: st, end: en }).pipe(s); }
  s.writeHead(200, { ...h, 'Content-Length': size }); createReadStream(f).pipe(s); });
await new Promise((r) => server.listen(0, '127.0.0.1', r)); const port = server.address().port;
const gl = process.env.SW ? ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'] : ['--use-angle=metal', '--enable-gpu'];
const browser = await puppeteer.launch({ executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true, protocolTimeout: 900000, args: [...gl, '--autoplay-policy=no-user-gesture-required', '--disable-background-timer-throttling', '--disable-renderer-backgrounding', '--no-first-run', '--window-size=480,270'] });
const engine = process.env.SW ? 'chrome-swiftshader' : 'chrome-metal';
for (const run of JSON.parse(readFileSync(runsF))) {
  const page = await browser.newPage();
  await new Promise((resolve) => { const t = setTimeout(() => { appendFileSync(out, JSON.stringify({ engine, cfg: run, fatal: 'timeout' }) + '\n'); resolve(); }, (run.timeoutSec ?? 240) * 1000);
    page.on('error', (e) => { appendFileSync(out, JSON.stringify({ engine, cfg: run, fatal: 'crash:' + e.message }) + '\n'); clearTimeout(t); resolve(); });
    Promise.all([page.exposeFunction('__vvFinish', (r) => { clearTimeout(t); appendFileSync(out, JSON.stringify({ ...r, engine }) + '\n'); resolve(); }),
      page.exposeFunction('__vvCtl', async (raw) => { const o = JSON.parse(raw); if (o.op === 'save') writeFileSync(join(pngDir, `${engine}-${o.name}`), Buffer.from(o.b64, 'base64')); else if (o.op === 'mem') { const m = await page.metrics(); await page.evaluate((v) => window.__vvMemReply?.(v), { js_heap_mb: m.JSHeapUsedSize / 1048576 }); } })])
      .then(() => page.goto(`http://127.0.0.1:${port}/index.html?cfg=${encodeURIComponent(Buffer.from(JSON.stringify({ assetsDir, ...run })).toString('base64'))}`)); });
  await page.close().catch(() => {});
}
await browser.close(); server.close();
