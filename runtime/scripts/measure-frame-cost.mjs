#!/usr/bin/env node
// Measures what one frame costs in the runtime and how much of it is the readback and the hash:
// the same image plan drawn N times with and without `inspect`, through the real worker and
// Chrome (TESSEL_GL=gpu for the machine's GPU, swiftshader by default).
//
//   TESSEL_GL=gpu node runtime/scripts/measure-frame-cost.mjs [--size 960x540] [--frames 40]
//
// Prints one JSON object: the median of each phase the runtime reports in `frame.done.timings`
// (`settle_ms` = wait for the GPU when nothing is read back, `readback_ms` = readPixels + flip,
// `describe_ms` = FNV hash + mean) and the host-side round trip. It measures; it asserts nothing.
import { spawn } from 'node:child_process';

import { dirname, join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const argument = (name, fallback) => {
  const index = process.argv.indexOf(`--${name}`);
  return index === -1 ? fallback : process.argv[index + 1];
};
const [width, height] = argument('size', '960x540').split('x').map(Number);
const frames = Number(argument('frames', '40'));
const image = join(root, '..', 'assets', 'demo', 'coast.png');

const child = spawn('node', [join(root, 'worker/worker.mjs')], { stdio: ['pipe', 'pipe', 'inherit'] });
const queue = [];
const waiters = [];
createInterface({ input: child.stdout }).on('line', (line) => {
  const message = JSON.parse(line);
  const waiter = waiters.shift();
  if (waiter) waiter(message);
  else queue.push(message);
});
const send = (message) => child.stdin.write(`${JSON.stringify({ v: 1, ...message })}\n`);
const next = () => (queue.length ? Promise.resolve(queue.shift()) : new Promise((resolve) => waiters.push(resolve)));
const until = async (predicate) => {
  for (;;) {
    const message = await next();
    if (predicate(message)) return message;
  }
};
const median = (values) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted.length ? Number(sorted[Math.floor(sorted.length / 2)].toFixed(2)) : null;
};

const plan = {
  version: 1,
  canvas: { width, height },
  intent: { mode: 'sequence' },
  frame: 0,
  layers: [
    {
      id: 'layer:a',
      kind: 'image',
      asset: 'img',
      opacity: 1,
      blend: 'normal',
      transform: { x: 0, y: 0, scale: 1, rotation: 0, crop: [0, 0, 0, 0] },
    },
  ],
  requires: ['img'],
  warnings: [],
};

send({ type: 'hello' });
const ready = await until((m) => m.type === 'ready');
send({ type: 'asset.load', asset: { id: 'img', kind: 'still', source: { type: 'file', path: image }, meta: {} } });
await until((m) => m.type === 'asset.state' && m.state === 'ready');

const run = async (inspect) => {
  const timings = [];
  const roundTrips = [];
  for (let index = 0; index < frames + 3; index += 1) {
    const id = (inspect ? 1000 : 2000) + index;
    const started = performance.now();
    send({ type: 'frame.render', id, revision: id, plan, output: { width, height }, ...(inspect ? {} : { inspect: false }) });
    const done = await until((m) => m.type === 'frame.done' && m.id === id);
    // The first frames compile shaders and upload the texture: not the steady state.
    if (index >= 3) {
      timings.push(done.timings);
      roundTrips.push(performance.now() - started);
    }
  }
  const phase = (name) => median(timings.map((t) => t[name]).filter((value) => value !== undefined));
  return {
    inspect,
    render_ms: phase('render_ms'),
    settle_ms: phase('settle_ms'),
    readback_ms: phase('readback_ms'),
    describe_ms: phase('describe_ms'),
    round_trip_ms: median(roundTrips),
  };
};

const withInspect = await run(true);
const withoutInspect = await run(false);
console.log(
  JSON.stringify(
    { renderer: ready.runtime.renderer, size: `${width}x${height}`, frames, withInspect, withoutInspect },
    null,
    2,
  ),
);
send({ type: 'dispose' });
child.kill();
