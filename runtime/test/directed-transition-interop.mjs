// Helper of crates/render/tests/scene_transition_parser.rs (F064, P4b S2-R0). Not a test file (no `.test.` in the name).
//
//   node directed-transition-interop.mjs <plan.json>...
//
// Reads the JSON TEXT of each plan the core wrote with `parseTransitionPlanText` and prints ONE JSON document on
// stdout: the two ceilings and, per file, either what the reader understood (the numbers the Rust side compares
// against the plan it built) or the typed refusal. It loads no asset, opens no GL context and sends nothing: it is
// the reader, and only the reader.
import { readFileSync } from 'node:fs';

import { MAX_TRANSITION_PLAN_BYTES, MAX_TRANSITION_PLAN_ITEMS, parseTransitionPlanText } from '../src/directed-transition.js';
import { RuntimeError } from '../src/protocol.js';

const summary = (read) => ({
  ok: true,
  mix: read.mix,
  window: read.window,
  timing: read.timing,
  clocks: read.clocks,
  decks: ['primary', 'other'].map((name) => ({
    deck: name,
    planes: read[name].planes.map((plane) => plane.nodeId),
    splats: read[name].splats.length,
    lights: read[name].lights.length,
    fx: read[name].fx.length,
    resources: [...read[name].resources.keys()].sort(),
  })),
  passes: Object.fromEntries(Object.entries(read.passes).map(([name, list]) => [name, list.map((pass) => pass.clipId)])),
});

const results = process.argv.slice(2).map((path) => {
  const text = readFileSync(path, 'utf8');
  try {
    return { path, bytes: Buffer.byteLength(text, 'utf8'), ...summary(parseTransitionPlanText(text, [])) };
  } catch (error) {
    if (!(error instanceof RuntimeError)) throw error;
    return { path, ok: false, code: error.code, pointer: error.details?.pointer ?? null, message: error.message };
  }
});

process.stdout.write(`${JSON.stringify({ maxBytes: MAX_TRANSITION_PLAN_BYTES, maxItems: MAX_TRANSITION_PLAN_ITEMS, results })}\n`);
