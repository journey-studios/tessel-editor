// BL157.3: the activation contract, runtime side (`fixtures/activation-spans.json`).
//
// The A/B only means something if an `on` arm really did the work, and the only witness is a span the page hands the host with a frame:
// `{ k: 'preupload' | 'warm_draw', outcome, ms }`. That is a chain of three links, each tested where it lives from ONE file: this suite makes
// the real runtime report every outcome the contract lists (and no other); `asset_io_tests.rs` sends each through the host and finds it in
// the trace; `first-use-ab.spec.py` has the harness judge an arm by them. A name changed on one side only breaks the others.
//
// The runs below use the fake compositor, so they cannot say what the REAL compositor returns for a warm draw: the source scans at the end
// pin the outcome names of the real code (`Compositor.warmDraw`, `AssetStore.preuploadStill`, the queue and the page) to the contract.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import { rgbaBytes } from '../src/retention.js';
import { SIDE, fakeClock, frameMessage, openFake, sceneWarm, scenes, stills, textureWarm } from './orchestrator-fixtures.mjs';

const CONTRACT = JSON.parse(readFileSync(new URL('./fixtures/activation-spans.json', import.meta.url), 'utf8')).spans;
const source = (name) => readFileSync(new URL(`../src/${name}`, import.meta.url), 'utf8');
const named = (spec) => [spec.worked, ...spec.not_worked].sort();

const PRE = { retain: true, preupload_textures: true };
const WARM_DRAW = { retain: true, warm_draw: true };
const LATE = 3500; // past the 3 s a queued upload or a waiting draw is kept

/** A queued warm of `ids` whose step has not run: the page's queue is held so the test chooses what happens first. */
const queued = async (page, ...ids) => {
  page.hello(PRE);
  await page.loadAll(...ids);
  const release = page.debug.holdQueue();
  page.send(textureWarm(stills(...ids)));
  return release;
};

const warmedScene = async (page) => {
  page.hello(WARM_DRAW);
  await page.warm(sceneWarm(scenes('s')));
};
const sortEnds = async (page) => {
  page.compositor.sorts[0].resolve();
  await page.settle();
};
const drawReports = (outcome) => async (page) => {
  await warmedScene(page);
  page.compositor.warmDrawOutcome = outcome;
  await sortEnds(page);
};

/** One run per outcome: what makes the REAL page report it. A scenario ends when the work is done; the test then asks for a frame. */
const SCENARIOS = {
  preupload: {
    uploaded: async (page) => {
      page.hello(PRE);
      await page.loadAll('a');
      await page.warm(textureWarm(stills('a')));
    },
    resident: async (page) => {
      const release = await queued(page, 'a');
      page.send(frameMessage(stills('a'), 1)); // the frame makes the texture before the step reaches it
      release();
      await page.settle();
    },
    budget: async (page) => {
      page.hello(PRE);
      page.debug.setLimits({ imageTextureBytes: rgbaBytes(SIDE, SIDE) }); // room for one
      await page.loadAll('a', 'b');
      await page.warm(textureWarm(stills('a', 'b')));
    },
    stale: async (page) => {
      const release = await queued(page, 'a');
      await page.load('a'); // replaced before the step
      release();
      await page.settle();
    },
    missing: async (page) => {
      const release = await queued(page, 'a');
      page.release('a');
      release();
      await page.settle();
    },
    expired: async (page, clock) => {
      const release = await queued(page, 'a');
      clock.advance(LATE);
      release();
      await page.settle();
    },
    failed: async (page) => {
      page.hello(PRE);
      await page.loadAll('a');
      page.compositor.failUploads = 1;
      await page.warm(textureWarm(stills('a')));
    },
  },
  warm_draw: {
    drawn: drawReports('drawn'),
    gone: drawReports('gone'),
    taken: drawReports('taken'),
    no_order: drawReports('no_order'),
    lost: drawReports('lost'),
    failed: drawReports('failed'),
    dropped: async (page) => {
      await warmedScene(page);
      page.loseContext();
      await sortEnds(page);
      page.restoreContext(); // spans ride on `frame.done`; the host asks again once the context is back
    },
    expired: async (page, clock) => {
      await warmedScene(page);
      clock.advance(LATE);
      await sortEnds(page);
    },
  },
};

for (const [k, spec] of Object.entries(CONTRACT)) {
  for (const outcome of [spec.worked, ...spec.not_worked]) {
    test(`the page tells the host \`${k}\` / \`${outcome}\` as { k, outcome, ms } and nothing else`, async (t) => {
      const scenario = SCENARIOS[k]?.[outcome];
      assert.ok(scenario, `the contract lists ${k}/${outcome} but no scenario makes the runtime report it`);
      const clock = fakeClock();
      t.after(clock.restore);
      const page = openFake();
      await scenario(page, clock);

      const done = await page.frame([]);
      const spans = (done.runtime_spans ?? []).filter((span) => span.k === k);
      const span = spans.find((candidate) => candidate.outcome === outcome);
      assert.ok(span, `\`${k}\` / \`${outcome}\` was not reported; the frame carried ${JSON.stringify(done.runtime_spans)}`);
      assert.deepEqual(Object.keys(span).sort(), ['k', 'ms', 'outcome'], 'the shape the host forwards and the harness reads');
      assert.ok(Number.isFinite(span.ms) && span.ms >= 0, 'a duration on the page clock');
      for (const reported of spans) {
        assert.ok(named(spec).includes(reported.outcome), `\`${reported.outcome}\` is reported but not in the contract`);
      }
    });
  }
}

test('every scenario is an outcome of the contract and every outcome of the contract has one', () => {
  for (const [k, spec] of Object.entries(CONTRACT)) {
    assert.deepEqual(Object.keys(SCENARIOS[k] ?? {}).sort(), named(spec), k);
  }
  assert.deepEqual(Object.keys(SCENARIOS).sort(), Object.keys(CONTRACT).sort());
});

test('only the outcome the contract calls worked is `uploaded` / `drawn`, and it is the only one that means the manipulation happened', () => {
  assert.equal(CONTRACT.preupload.worked, 'uploaded');
  assert.equal(CONTRACT.warm_draw.worked, 'drawn');
  for (const spec of Object.values(CONTRACT)) assert.ok(!spec.not_worked.includes(spec.worked));
});

/** The string literals returned by (or noted in) a method of the real source, found by its signature up to its closing brace. */
const literals = (code, signature, pattern, closing = '\n  }\n') => {
  const start = code.indexOf(signature);
  assert.notEqual(start, -1, `${signature} is not in the source any more: update this scan with the code`);
  const body = code.slice(start, code.indexOf(closing, start));
  return [...new Set([...body.matchAll(pattern)].map((match) => match[1]))].sort();
};
const returnedWords = (code, signature) => {
  const start = code.indexOf(signature);
  assert.notEqual(start, -1, `${signature} is not in the source any more: update this scan with the code`);
  const body = code.slice(start, code.indexOf('\n  }\n', start));
  const words = [...body.matchAll(/return[^;]*;/g)].flatMap(([statement]) => [...statement.matchAll(/'([a-z_]+)'/g)].map((match) => match[1]));
  return [...new Set(words)].sort();
};

test('the real compositor and the page can report exactly the warm draw outcomes of the contract', () => {
  const compositor = returnedWords(source('compositor.js'), 'async warmDraw(item, assets) {');
  const page = literals(source('index.js'), 'const runWarmDraw = async (item) => {', /note\('([a-z_]+)'\)/g, '\n  };\n');
  assert.deepEqual(compositor, ['drawn', 'failed', 'gone', 'lost', 'no_order', 'taken'], 'what Compositor.warmDraw can return');
  assert.deepEqual(page, ['dropped', 'expired'], 'what the page decides itself');
  assert.deepEqual([...new Set([...compositor, ...page])].sort(), named(CONTRACT.warm_draw));
});

test('the real store and the queue can report exactly the pre-upload outcomes of the contract', () => {
  const store = returnedWords(source('assets.js'), 'preuploadStill(id, serial, upload) {');
  const queue = literals(source('preupload.js'), 'step(assets, upload) {', /outcome(?::| =) '([a-z_]+)'/g);
  assert.deepEqual(store, ['budget', 'missing', 'resident', 'stale', 'uploaded'], 'what AssetStore.preuploadStill can return');
  assert.deepEqual(queue, ['expired', 'failed'], 'what the queue decides itself');
  assert.deepEqual([...new Set([...store, ...queue])].sort(), named(CONTRACT.preupload));
});
