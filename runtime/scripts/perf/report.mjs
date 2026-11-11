// Reads the raw JSONL of run.mjs and writes the concise, durable summary plus markdown tables.
//
//   node scripts/perf/report.mjs --in DIR[,DIR...] [--out DIR]
//
// With several input directories, a run of an earlier one that failed (timeout, fetch error) is
// replaced by the same run (`name` + `pass`) of a later one; a run that succeeded is never replaced,
// and duplicates of one run collapse to the latest. Nothing is dropped silently: failures that
// remain are counted in the tables.
//
// What the summary keeps: per engine/case/variant, the per-pass mean of every phase, their median
// (so one noisy pass cannot carry a claim), cold first/second frame, cache events, load average,
// and the pass-to-pass spread. Raw series stay in the JSONL (outside the repository).
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const root = join(dirname(fileURLToPath(import.meta.url)), '../../..');
const { values: opts } = parseArgs({
  options: {
    in: { type: 'string' },
    out: { type: 'string', default: join(root, 'docs/viewport-performance-results') },
  },
});
if (!opts.in) throw new Error('--in DIR is required');

const median = (values) => {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  if (!sorted.length) return null;
  const mid = sorted.length >> 1;
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
};
const r = (value, digits = 2) => (value === null || value === undefined ? null : Number(value.toFixed(digits)));
const PHASES = ['wall', 'render', 'prepare', 'draw', 'flush', 'drain', 'complete'];

const readRuns = (file) =>
  readFileSync(file, 'utf8')
    .split('\n')
    .filter(Boolean)
    .map((line) => JSON.parse(line));

const group = (runs, key) => {
  const groups = new Map();
  for (const run of runs) {
    const name = key(run);
    groups.set(name, [...(groups.get(name) ?? []), run]);
  }
  return groups;
};

const summarizeBench = (runs) => {
  const rows = [];
  for (const [caseName, byCase] of group(runs, (run) => run.run.case)) {
    for (const [variant, list] of group(byCase, (run) => run.run.variant)) {
      const ok = list.filter((run) => !run.fatal && run.wall);
      const row = {
        case: caseName,
        variant,
        passes: ok.length,
        failed: list.length - ok.length,
        frames: ok[0]?.declared?.frames ?? null,
        quantum_ms: r(median(ok.map((run) => run.clock_quantum_ms)), 3),
        load_1m: ok.map((run) => r(run.loadavg?.[0], 1)),
        cold_first_render_ms: r(median(ok.map((run) => run.cold_first?.render))),
        cold_second_render_ms: r(median(ok.map((run) => run.cold_second?.render))),
        fps: r(median(ok.map((run) => run.frames_per_second)), 1),
        errors: ok.reduce((sum, run) => sum + (run.errors?.length ?? 0), 0),
        raf_fallbacks: ok.reduce((sum, run) => sum + (run.raf_fallbacks ?? 0), 0),
      };
      for (const phase of PHASES) {
        const stats = ok.map((run) => run[phase]).filter(Boolean);
        if (!stats.length) continue;
        const means = stats.map((stat) => r(stat.mean));
        row[phase] = {
          mean_per_pass: means,
          mean: r(median(means)),
          p50: r(median(stats.map((stat) => stat.p50))),
          p95: r(median(stats.map((stat) => stat.p95))),
          p99: r(median(stats.map((stat) => stat.p99))),
          spread: r(Math.max(...means) - Math.min(...means)),
        };
      }
      row.cache_events = ok.find((run) => run.cache_events)?.cache_events ?? null;
      rows.push(row);
    }
  }
  return rows;
};

/**
 * Cases about one event (`series` in the record): what the frame of the event cost against the frames before it, per pass.
 * `complete` is the frame plus the wait for the GPU (the drain). `before_excess_ms` is the cost the variant moved AHEAD of the
 * event (what its frames before it spent above the steady state): a gain at the event that only shows up there is not a gain.
 */
const summarizeEvent = (runs) => {
  const rows = [];
  // A run that died has no `entry_at`: it still belongs to its case, and is counted as failed (not silently left out).
  const eventCases = new Set(runs.filter((run) => run.entry_at !== undefined).map((run) => run.run.case));
  for (const [caseName, byCase] of group(runs.filter((run) => eventCases.has(run.run.case)), (run) => run.run.case)) {
    for (const [variant, list] of group(byCase, (run) => run.run.variant)) {
      const ok = list.filter((run) => !run.fatal && run.series?.complete);
      const perPass = ok.map((run) => {
        const at = run.entry_at - run.first_index;
        const complete = run.series.complete;
        const steady = median(complete.slice(Math.max(1, at - 20), at));
        // The state the frames settle into after the event (a second layer costs more than one: the event is measured against it too).
        const settled = median(complete.slice(at + 14));
        return {
          steady,
          settled,
          event: complete[at],
          after: Math.max(...complete.slice(at, at + 12)),
          before_excess: complete.slice(1, at).reduce((sum, value) => sum + Math.max(0, value - steady - 2), 0),
          max_raf_wait_after: Math.max(...run.series.raf_wait.slice(at - 1, at + 12)),
          load_ms: run.late ? Math.max(...Object.values(run.late).map((entry) => entry.ms ?? 0)) : null,
          load: run.loadavg?.[0],
        };
      });
      rows.push({
        case: caseName,
        variant,
        passes: ok.length,
        failed: list.length - ok.length,
        steady_ms: r(median(perPass.map((pass) => pass.steady)), 1),
        settled_ms: r(median(perPass.map((pass) => pass.settled)), 1),
        event_ms: perPass.map((pass) => r(pass.event, 0)),
        event_median_ms: r(median(perPass.map((pass) => pass.event)), 1),
        event_max_ms: r(Math.max(...perPass.map((pass) => pass.event)), 0),
        extra_median_ms: r(median(perPass.map((pass) => pass.event - pass.settled)), 1),
        worst_of_12_after_median_ms: r(median(perPass.map((pass) => pass.after)), 1),
        before_excess_median_ms: r(median(perPass.map((pass) => pass.before_excess)), 1),
        max_raf_wait_after_ms: r(Math.max(...perPass.map((pass) => pass.max_raf_wait_after)), 0),
        load_ms: r(median(perPass.map((pass) => pass.load_ms).filter(Number.isFinite)), 0),
        load_1m: perPass.map((pass) => r(pass.load, 1)),
      });
    }
  }
  return rows;
};

/** Every explicit-frame hash of a case must agree across variants, passes and shuffle orders. */
const summarizeHash = (runs) => {
  const out = [];
  for (const [caseName, byCase] of group(runs, (run) => run.run.case)) {
    const byIndex = new Map();
    const variants = new Set();
    let failed = 0;
    for (const run of byCase) {
      if (run.fatal || !run.hashes) {
        failed += 1;
        continue;
      }
      variants.add(run.run.variant);
      for (const hash of run.hashes) byIndex.set(hash.index, new Set([...(byIndex.get(hash.index) ?? []), hash.hash]));
      const first = run.hashes.find((hash) => hash.index === run.run.order[0]);
      const label = `after_interactive:${run.run.order[0]}`;
      byIndex.set(label, new Set([...(byIndex.get(label) ?? []), run.after_interactive.hash, first.hash]));
    }
    const mismatches = [...byIndex].filter(([, set]) => set.size > 1).map(([index, set]) => ({ index, hashes: [...set] }));
    out.push({
      case: caseName,
      runs: byCase.length - failed,
      failed,
      variants: [...variants],
      frames_compared: byIndex.size,
      identical: failed === 0 && mismatches.length === 0,
      mismatches,
      png: [...new Set(byCase.map((run) => run.png?.png))].length,
    });
  }
  return out;
};

/** Lifecycle: before == after_restore == after_context, after_swap differs, no step errored. */
const summarizeLife = (runs) =>
  runs.map((run) => {
    const steps = run.steps ?? {};
    const errors = Object.entries(steps).filter(([, value]) => String(value).startsWith('ERROR'));
    const swapped = steps.after_swap !== undefined;
    return {
      case: run.run.case,
      variant: run.run.variant,
      pass: run.run.pass,
      fatal: run.fatal ?? null,
      errors: errors.map(([name, value]) => `${name}: ${value}`),
      swap_changed_pixels: swapped ? steps.after_swap !== steps.before : null,
      restore_exact: swapped ? steps.after_restore === steps.before : null,
      context_exact: steps.after_context === steps.before && steps.after_context2 === steps.before2,
      retention_after_release: steps.release ?? null,
    };
  });

const summarizeBridge = (runs) =>
  runs.map((run) => ({ case: run.run.case, pass: run.run.pass, fatal: run.fatal ?? null, frames: run.frames, round_trip_ms: run.round_trip_ms, cold_first_ms: run.cold_first_ms }));

const table = (rows, columns) => {
  const lines = [`| ${columns.map(([name]) => name).join(' | ')} |`, `|${columns.map(() => '---').join('|')}|`];
  for (const row of rows) lines.push(`| ${columns.map(([, get]) => get(row) ?? '-').join(' | ')} |`);
  return lines.join('\n');
};

mkdirSync(opts.out, { recursive: true });
const markdown = [];
const inputs = opts.in.split(',');

const failed = (run) => Boolean(run.fatal) || (run.steps && Object.values(run.steps).some((value) => String(value).startsWith('ERROR')));

/** Latest record per run key; a failure never replaces a success. */
const mergeRuns = (lists) => {
  const byKey = new Map();
  for (const run of lists.flat()) {
    const key = `${run.run.name}#${run.run.pass}`;
    const known = byKey.get(key);
    if (known && !failed(known) && failed(run)) continue;
    byKey.set(key, run);
  }
  return [...byKey.values()];
};

const files = [...new Set(inputs.flatMap((dir) => readdirSync(dir).filter((name) => name.endsWith('.jsonl'))))].sort();
for (const file of files) {
  const [engine, suite] = file.replace('.jsonl', '').split(/-(?=[^-]+$)/);
  const runs = mergeRuns(inputs.map((dir) => (existsSync(join(dir, file)) ? readRuns(join(dir, file)) : [])));
  const summaries = { bench: summarizeBench, xfade: summarizeBench, entry: summarizeBench, real: summarizeBench, hash: summarizeHash, life: summarizeLife, bridge: summarizeBridge };
  if (!summaries[suite]) continue;
  const rows = summaries[suite](runs);
  const events = ['entry', 'real'].includes(suite) ? summarizeEvent(runs) : [];
  const environment = { engine, suite, runs: runs.length, renderer: runs.find((run) => run.renderer)?.renderer ?? null, browser: runs.find((run) => run.browser)?.browser ?? null };
  writeFileSync(join(opts.out, `${engine}-${suite}.json`), `${JSON.stringify({ environment, rows, events }, null, 1)}\n`);
  markdown.push(`\n### ${engine} / ${suite}\n`);
  if (['bench', 'xfade', 'entry', 'real'].includes(suite)) {
    const cell = (phase) => (row) => (row[phase] ? `${row[phase].mean} (p50 ${row[phase].p50}, p95 ${row[phase].p95}, p99 ${row[phase].p99})` : null);
    markdown.push(
      table(rows, [
        ['case', (row) => row.case],
        ['variant', (row) => row.variant],
        ['render ms mean (p50/p95/p99)', cell('render')],
        ['wall ms', cell('wall')],
        ['wall+drain ms', cell('complete')],
        ['cold 1st', (row) => row.cold_first_render_ms],
        ['spread', (row) => row.render?.spread],
        ['err', (row) => row.errors + row.failed],
        ['raf stalls', (row) => row.raf_fallbacks],
      ]),
    );
    if (events.length) {
      markdown.push(
        '\nOne event per run (ms; the frame of the event includes the wait for the GPU):\n',
        table(events, [
          ['case', (row) => row.case],
          ['variant', (row) => row.variant],
          ['passes (failed)', (row) => `${row.passes} (${row.failed})`],
          ['before', (row) => row.steady_ms],
          ['after (settled)', (row) => row.settled_ms],
          ['event frame, per pass', (row) => row.event_ms.join(' ')],
          ['median', (row) => row.event_median_ms],
          ['max', (row) => row.event_max_ms],
          ['extra over settled (median)', (row) => row.extra_median_ms],
          ['moved ahead of the event', (row) => row.before_excess_median_ms],
          ['load of the asset', (row) => row.load_ms],
          ['max rAF wait after', (row) => row.max_raf_wait_after_ms],
        ]),
      );
    }
  } else if (suite === 'hash') {
    markdown.push(table(rows, [['case', (row) => row.case], ['runs', (row) => row.runs], ['variants', (row) => row.variants.join(',')], ['frames cmp', (row) => row.frames_compared], ['identical', (row) => row.identical], ['failed', (row) => row.failed]]));
  } else if (suite === 'life') {
    markdown.push(table(rows, [['case', (row) => row.case], ['variant', (row) => row.variant], ['pass', (row) => row.pass], ['swap changed', (row) => row.swap_changed_pixels], ['restore exact', (row) => row.restore_exact], ['context exact', (row) => row.context_exact], ['errors', (row) => row.errors.length + (row.fatal ? 1 : 0)]]));
  } else {
    markdown.push(table(rows, [['case', (row) => row.case], ['pass', (row) => row.pass], ['round trip mean', (row) => r(row.round_trip_ms?.mean)], ['p95', (row) => r(row.round_trip_ms?.p95)], ['p99', (row) => r(row.round_trip_ms?.p99)]]));
  }
}
console.log(markdown.join('\n'));
