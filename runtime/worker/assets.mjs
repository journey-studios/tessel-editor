// Local asset server for the headless page: serves the built runtime and, only for files an
// `asset.load` message named, the project's media (with Range, which <video> needs to seek).
import { createReadStream, existsSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { extname, join, resolve } from 'node:path';

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.webp': 'image/webp',
  '.gif': 'image/gif',
  '.webm': 'video/webm',
  '.mp4': 'video/mp4',
  '.wav': 'audio/wav',
  '.json': 'application/json',
  '.splat': 'application/octet-stream',
};

/** Regex for a frame pattern like `/a/f%05d.png`: only the index may vary. */
const patternRegex = (pattern) => {
  const escaped = resolve(pattern).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(`^${escaped.replace(/%(0?)(\d*)d/, (_, zero, width) => `\\d{${zero ? width || 1 : 1},}`)}$`);
};

export class AllowList {
  constructor() {
    this.files = new Set();
    this.patterns = [];
  }

  /** Registers what a descriptor's `source` may read. */
  register(descriptor) {
    const { source } = descriptor ?? {};
    if (source?.type === 'file') this.files.add(resolve(source.path));
    if (source?.type === 'frame_sequence') this.patterns.push(patternRegex(source.pattern));
  }

  allows(path) {
    const absolute = resolve(path);
    return this.files.has(absolute) || this.patterns.some((pattern) => pattern.test(absolute));
  }
}

const parseRange = (header, size) => {
  const match = /^bytes=(\d*)-(\d*)$/.exec(header ?? '');
  if (!match || (match[1] === '' && match[2] === '')) return null;
  let start = match[1] === '' ? size - Number(match[2]) : Number(match[1]);
  let end = match[1] === '' || match[2] === '' ? size - 1 : Number(match[2]);
  start = Math.max(0, start);
  end = Math.min(size - 1, end);
  return start <= end ? { start, end } : 'invalid';
};

export const startAssetServer = ({ distDir, allowList }) =>
  new Promise((resolveServer, reject) => {
    const server = createServer((request, response) => {
      const url = new URL(request.url, 'http://127.0.0.1');
      const send = (status, body, headers = {}) => {
        response.writeHead(status, { 'Cache-Control': 'no-store', ...headers });
        response.end(body);
      };
      if (request.method !== 'GET' && request.method !== 'HEAD') return send(405, 'method not allowed');

      if (url.pathname === '/favicon.ico') return send(204, '');
      let path;
      if (url.pathname === '/' || url.pathname === '/index.html') path = join(distDir, 'index.html');
      else if (url.pathname === '/runtime.js') path = join(distDir, 'runtime.js');
      else if (url.pathname === '/asset') {
        const requested = url.searchParams.get('p');
        if (!requested || !allowList.allows(requested)) return send(403, 'not an asset of this project');
        path = resolve(requested);
      } else return send(404, 'not found');

      if (!existsSync(path) || !statSync(path).isFile()) return send(404, `missing ${path}`);
      const { size } = statSync(path);
      const headers = {
        'Content-Type': TYPES[extname(path).toLowerCase()] ?? 'application/octet-stream',
        'Accept-Ranges': 'bytes',
      };
      const range = parseRange(request.headers.range, size);
      if (range === 'invalid') return send(416, 'bad range', { 'Content-Range': `bytes */${size}` });
      if (range) {
        response.writeHead(206, {
          ...headers,
          'Cache-Control': 'no-store',
          'Content-Range': `bytes ${range.start}-${range.end}/${size}`,
          'Content-Length': range.end - range.start + 1,
        });
        if (request.method === 'HEAD') return response.end();
        return createReadStream(path, { start: range.start, end: range.end }).pipe(response);
      }
      response.writeHead(200, { ...headers, 'Cache-Control': 'no-store', 'Content-Length': size });
      if (request.method === 'HEAD') return response.end();
      return createReadStream(path).pipe(response);
    });
    server.on('error', reject);
    // Port 0: the OS picks a free one; bound to loopback only.
    server.listen(0, '127.0.0.1', () => resolveServer({ server, port: server.address().port }));
  });
