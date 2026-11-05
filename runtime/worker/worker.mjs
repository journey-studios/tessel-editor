#!/usr/bin/env node
// Headless viewport host: bridge v1 as JSON Lines on stdin/stdout, a real browser behind it.
//
//   stdin  one bridge message per line (host -> runtime)
//   stdout one bridge message per line (runtime -> host)
//   stderr human-readable diagnostics only
//
// The worker owns everything the page cannot do: it launches Chrome, serves the runtime and
// the project's files, and writes PNG files. What to draw and how stays in the page.
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

import { AllowList, startAssetServer } from './assets.mjs';
import { ChromeNotFound, findChrome, glFlags } from './chrome.mjs';

const BRIDGE_VERSION = 1;
const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const distDir = process.env.TESSEL_RUNTIME_DIST ?? join(root, 'dist');

const emit = (message) => process.stdout.write(`${JSON.stringify({ v: BRIDGE_VERSION, ...message })}\n`);
const log = (text) => process.stderr.write(`[tessel-worker] ${text}\n`);
const failStart = (code, message) => {
  emit({ type: 'error', error: { code, message } });
  process.exit(1);
};

const state = { browser: null, server: null, page: null, closing: false };
const pngTargets = new Map();

const shutdown = async (code = 0) => {
  if (state.closing) return;
  state.closing = true;
  await Promise.allSettled([state.browser?.close(), new Promise((done) => state.server?.close(done))]);
  process.exit(code);
};

const start = async () => {
  const allowList = new AllowList();
  let puppeteer;
  try {
    puppeteer = (await import('puppeteer-core')).default;
  } catch (error) {
    failStart('backend_unavailable', `puppeteer-core is not installed (${error.message}). Run: pnpm --dir runtime install`);
  }
  let executablePath;
  try {
    executablePath = findChrome();
  } catch (error) {
    failStart(error instanceof ChromeNotFound ? error.code : 'backend_unavailable', error.message);
  }
  try {
    glFlags(process.env.TESSEL_GL);
  } catch (error) {
    failStart('backend_unavailable', error.message);
  }
  const { server, port } = await startAssetServer({ distDir, allowList });
  state.server = server;

  state.browser = await puppeteer.launch({
    executablePath,
    headless: true,
    args: [
      ...glFlags(process.env.TESSEL_GL ?? 'swiftshader'),
      '--autoplay-policy=no-user-gesture-required',
      '--disable-background-timer-throttling',
      '--disable-renderer-backgrounding',
      '--hide-scrollbars',
      '--mute-audio',
      '--no-first-run',
      '--no-default-browser-check',
    ],
  }).catch((error) => failStart('backend_unavailable', `Cannot launch Chrome at ${executablePath}: ${error.message}`));
  state.browser.on('disconnected', () => {
    if (state.closing) return;
    emit({ type: 'closed', reason: 'The browser process exited' });
    process.exit(1);
  });

  const page = await state.browser.newPage();
  state.page = page;
  page.on('pageerror', (error) => log(`page error: ${error.message}`));
  page.on('error', (error) => {
    emit({ type: 'closed', reason: `The page crashed: ${error.message}` });
    process.exit(1);
  });
  if (process.env.TESSEL_RUNTIME_LOG === '1') page.on('console', (entry) => log(`console: ${entry.text()}`));

  await page.exposeFunction('__tesselPost', (raw) => {
    const message = JSON.parse(raw);
    if (message.type === 'page.ready') return; // only the WebView host waits for it
    if (message.type === 'frame.done' && message.png_data) {
      const path = pngTargets.get(message.id);
      const data = message.png_data;
      delete message.png_data;
      try {
        mkdirSync(dirname(path), { recursive: true });
        writeFileSync(path, Buffer.from(data, 'base64'));
        message.png = path;
      } catch (error) {
        emit({
          type: 'frame.failed',
          id: message.id,
          revision: message.revision,
          error: { code: 'io_error', message: `Cannot write ${path}: ${error.message}` },
        });
        return;
      }
    }
    if (message.type === 'frame.done' || message.type === 'frame.failed') pngTargets.delete(message.id);
    emit(message);
  });

  await page.goto(`http://127.0.0.1:${port}/`, { waitUntil: 'load' });
  const booted = await page.evaluate(() => typeof window.TesselRuntime?.receive === 'function');
  if (!booted) {
    failStart('backend_unavailable', `The runtime did not start. Is ${distDir}/runtime.js built? Run: pnpm --dir runtime run build`);
  }
  return { page, allowList };
};

const main = async () => {
  const { page, allowList } = await start();
  const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
  // Lines are handled strictly in order, so an asset.load registered before a frame.render
  // has its file allowed by the time the page asks for it.
  let chain = Promise.resolve();
  input.on('line', (line) => {
    if (!line.trim()) return;
    chain = chain.then(async () => {
      let message;
      try {
        message = JSON.parse(line);
      } catch (error) {
        emit({ type: 'error', error: { code: 'protocol_error', message: `Invalid JSON: ${error.message}` } });
        return;
      }
      if (message.type === 'asset.load') allowList.register(message.asset);
      // A preview proxy is read through the same asset URL, so it must be registered like a load.
      if (message.type === 'asset.proxy' && message.proxy?.pattern) {
        allowList.register({ source: { type: 'frame_sequence', pattern: message.proxy.pattern } });
      }
      if (message.type === 'frame.render' && message.output?.png) pngTargets.set(message.id, message.output.png);
      try {
        await page.evaluate((payload) => window.TesselRuntime.receive(payload), message);
      } catch (error) {
        emit({ type: 'error', error: { code: 'protocol_error', message: error.message } });
      }
      if (message.type === 'dispose') await shutdown(0);
    });
  });
  input.on('close', () => chain.then(() => shutdown(0)));
};

process.on('SIGTERM', () => void shutdown(0));
process.on('SIGINT', () => void shutdown(0));
main().catch((error) => failStart('backend_unavailable', error.message));
