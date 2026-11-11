#!/usr/bin/env python3
"""P4a / F062 (BL082.3): one mutant at a time over the production code of the shot scope.

Each mutant changes ONE place of a source file, runs the suite that must notice it, and puts the
file back (in `finally`, and on SIGINT/SIGTERM/SIGHUP, which are turned into exceptions so the
`finally` runs). The sha256 of every mutated file is compared before and after the whole run, and
`git status` of those files before and after, so a mutant can never reach a commit.

Verdicts (a verdict is never invented from a runner's own failure):
  KILLED           the suite failed AND at least one test failed by name (the names are kept)
  SURVIVED         the suite passed with the mutant in (each one needs an explanation in the report)
  COMPILE-ERROR    the mutant does not compile: says nothing about the tests, fix the mutant
  EXECUTION-ERROR  non-zero exit with no named failing test (no mbx, no node, a crash)
  NOT-APPLICABLE   the text to replace is not there exactly once: the code moved, fix the mutant
  REFUSED_DISK     `scripts/disk-guard.sh` refused the batch: the run stops there, nothing is cleaned

Runners:
  core     ./scripts/cargo test -p tessel-editor-core ... --lib --test scene_fx_shot_scope
  js       node --test on runtime/src (pure: never touches runtime/dist)
  render   `./scripts/runtime build` with the mutant in, the image proofs
           (crates/render/tests/scene_fx_shot_scope.rs), and `./scripts/runtime build` AGAIN after
           the file is back: runtime/dist is git-ignored and is what those tests draw with, so a
           stale mutant there would pass for the runtime
  desktop  ./scripts/cargo test -p tessel-editor ... scene_fx_scope (the GPUI tests of the window)

Usage (from anywhere; the root is this script's checkout, never another one):
  python3 docs/migration/p4a-shot-scope/mutate.py --check          applicability only, runs nothing
  python3 docs/migration/p4a-shot-scope/mutate.py                  every mutant
  python3 docs/migration/p4a-shot-scope/mutate.py C01 J02 W03      only these
  python3 docs/migration/p4a-shot-scope/mutate.py --kind js        only one runner
Environment: TESSEL_MUTATION_OUT (the folder of the report, default: this folder).
The report is rewritten after EVERY mutant, so an aborted run leaves what it had judged.
"""
import datetime
import hashlib
import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]  # the checkout this script lives in, never another one
OUT = Path(os.environ.get('TESSEL_MUTATION_OUT', Path(__file__).resolve().parent))
FLOOR_GIB = 12
STEP_GIB = {'core': 2, 'js': 0, 'render': 2, 'desktop': 2}

SCOPE = 'crates/core/src/directed_scene/fx/scope.rs'
FX = 'crates/core/src/directed_scene/fx/mod.rs'
INTEGRITY = 'crates/core/src/directed_scene/integrity.rs'
VALIDATE = 'crates/core/src/scene_plan/validate.rs'
EDIT = 'crates/core/src/scene_edit/fx.rs'
INSPECTOR = 'crates/core/src/scene_edit/fx_inspector.rs'
OUTPUT_FX = 'runtime/src/output-fx.js'
DIRECTED = 'runtime/src/directed-scene.js'
WINDOW = 'crates/desktop/src/workstation/scene_fx_scope.rs'

# (id, what the mutant breaks, file, text to replace (exactly once), replacement, runner)
MUTANTS = [
    # -- the one rule of the scope: where a shot clip may lie --
    ('C01', 'a clip that fills its shot to the last frame is refused (end bound off by one)', SCOPE,
     'if start_frame >= first && end <= last {', 'if start_frame >= first && end < last {', 'core'),
    ('C02', 'a clip that starts on the first frame of its shot is refused (start bound off by one)', SCOPE,
     'if start_frame >= first && end <= last {', 'if start_frame > first && end <= last {', 'core'),
    ('C03', 'a shot that does not exist is read as the first shot (UNKNOWN_SHOT never raised)', SCOPE,
     'let Some(shot) = shots.iter().find(|shot| &shot.id == id) else {', 'let Some(shot) = shots.first() else {', 'core'),
    ('C04', 'the first frame of the next shot still belongs to the shot that ends there', SCOPE,
     '.find(|shot| shot.start_frame <= frame && frame < shot.end_frame_exclusive)',
     '.find(|shot| shot.start_frame <= frame && frame <= shot.end_frame_exclusive)', 'core'),
    ('C05', 'the first frame of a shot belongs to no shot', SCOPE,
     '.find(|shot| shot.start_frame <= frame && frame < shot.end_frame_exclusive)',
     '.find(|shot| shot.start_frame < frame && frame < shot.end_frame_exclusive)', 'core'),
    # -- the spelling --
    ('C06', 'the output scope is written to the document (the canonical spelling leaves the key out)', SCOPE,
     '            Self::Output => None,\n            Self::Shot(id) => Some(json!({ "kind": "shot", "shotId": id })),',
     '            Self::Output => Some(json!({ "kind": "output" })),\n            Self::Shot(id) => Some(json!({ "kind": "shot", "shotId": id })),', 'core'),
    ('C07', 'a shot id with a colon is cut at the colon', SCOPE,
     '            return Ok(Self::Shot(id.to_string()));',
     "            return Ok(Self::Shot(id.split(':').next().unwrap_or(id).to_string()));", 'core'),
    ('C08', 'the passes of a shot are drawn in the output stage', SCOPE,
     '            Self::Shot(_) => FxStage::Deck,', '            Self::Shot(_) => FxStage::Output,', 'core'),
    ('C09', 'scope `node:<id>` is a plain invalid value instead of FX_SCOPE_NOT_YET', SCOPE,
     '        if text == "node" || text.starts_with("node:") {', '        if text == "node" {', 'core'),
    # -- the composition --
    ('C10', 'the output passes come before the passes of the shot (stages flipped)', FX,
     '        deck.extend(output);\n        deck\n', '        output.extend(deck);\n        output\n', 'core'),
    ('C11', 'a shot pass is drawn in every frame of its interval, whatever shot the frame is in', FX,
     '                        if current != Some(id.as_str()) {', '                        if false && current != Some(id.as_str()) {', 'core'),
    ('C12', 'the pass budget counts only the output clips', FX,
     '            for clip in track.clips.iter().filter(|clip| !clip.hidden) {\n                let start = clip.start_frame.max(0);',
     '            for clip in track.clips.iter().filter(|clip| !clip.hidden && clip.scope.is_output()) {\n                let start = clip.start_frame.max(0);', 'core'),
    # -- the plan --
    ('C13', 'a plan with a pass of a shot does not require fxShotScope', VALIDATE,
     '    if fx.iter().any(|pass| !pass.scope.is_output()) {', '    if fx.iter().any(|pass| !pass.scope.is_output() && false) {', 'core'),
    ('C14', 'the plan accepts a pass of another shot than the frame\'s', VALIDATE,
     '                if *shot != self.frame.shot_id {', '                if false && *shot != self.frame.shot_id {', 'core'),
    ('C15', 'the plan accepts the passes of the shot after the passes of the output', VALIDATE,
     '            if pass.scope.stage() < stage {', '            if false && pass.scope.stage() < stage {', 'core'),
    # -- the importer --
    ('C16', 'the importer does not judge the scope of the clips', INTEGRITY,
     '    fx_scopes(draft, &mut issues);', '    let _ = fx_scopes as fn(&Draft, &mut Vec<Issue>);', 'core'),
    # -- the edits --
    ('C17', 'add_clip does not judge the scope itself (the importer refuses it later, with another code)', EDIT,
     '    if let Some(fault) = fx::scope_fault(&add.scope, &clip_id, add.start_frame, duration, shots) {',
     '    if let Some(fault) = fx::scope_fault(&add.scope, &clip_id, add.start_frame, duration, shots).filter(|_| false) {', 'core'),
    ('C18', 'set_scope to the scope the clip already holds is an edit', EDIT,
     '    let noop = before == *scope;', '    let noop = false;', 'core'),
    ('C19', 'going back to the output writes {kind: output} instead of leaving the key out', EDIT,
     '            None => {\n                target.remove("scope");\n            }',
     '            None => {\n                target.insert("scope".to_string(), json!({ "kind": "output" }));\n            }', 'core'),
    ('C20', 'set_scope does not judge whether the clip fits the shot', EDIT,
     '        if let Some(fault) = fx::scope_fault(scope, clip, start, duration, shots) {',
     '        if let Some(fault) = fx::scope_fault(scope, clip, start, duration, shots).filter(|_| false) {', 'core'),
    ('C21', 'the mismatch is reported to commands with another code', EDIT,
     '        fx::FX_SCOPE_MISMATCH => "fx_scope_mismatch",', '        fx::FX_SCOPE_MISMATCH => "invalid_value",', 'core'),
    # -- the inspector --
    ('C22', 'the inspector offers a shot the clip does not fit', INSPECTOR,
     '            applicable: fault.is_none(),', '            applicable: true,', 'core'),
    # -- the runtime, pure --
    ('J01', 'a pass of a shot is accepted by a plan that does not require fxShotScope', OUTPUT_FX,
     "if (!requires.includes('fxShotScope'))", "if (false && !requires.includes('fxShotScope'))", 'js'),
    ('J02', 'a pass of another shot than the frame\'s is accepted', OUTPUT_FX,
     'if (raw.shotId !== frameShot) {', 'if (false && raw.shotId !== frameShot) {', 'js'),
    ('J03', 'the passes of the shot are accepted after the passes of the output', OUTPUT_FX,
     'if (position < stage) throw', 'if (false && position < stage) throw', 'js'),
    ('J04', 'the output is drawn before the deck', OUTPUT_FX,
     "      this.drawDeck(frame, passes.filter((pass) => pass.stage === 'deck'));\n"
     "      // A later delivery composes the second deck of a transition here, between the two stages.\n"
     "      this.drawOutput(frame, passes.filter((pass) => pass.stage !== 'deck'));",
     "      this.drawOutput(frame, passes.filter((pass) => pass.stage !== 'deck'));\n"
     "      this.drawDeck(frame, passes.filter((pass) => pass.stage === 'deck'));", 'js'),
    ('J05', 'the runtime does not advertise fxShotScope', DIRECTED,
     "  'fxShotScope',\n", '', 'js'),
    ('J06', 'the deck stage draws nothing', OUTPUT_FX,
     '  drawDeck(frame, passes) {\n    this.drawStage(frame, passes);\n  }', '  drawDeck(frame, passes) {}', 'js'),
    # -- the runtime, drawn: the same places, judged by the images --
    ('R01', 'the output is drawn before the deck (images)', OUTPUT_FX,
     "      this.drawDeck(frame, passes.filter((pass) => pass.stage === 'deck'));\n"
     "      // A later delivery composes the second deck of a transition here, between the two stages.\n"
     "      this.drawOutput(frame, passes.filter((pass) => pass.stage !== 'deck'));",
     "      this.drawOutput(frame, passes.filter((pass) => pass.stage !== 'deck'));\n"
     "      this.drawDeck(frame, passes.filter((pass) => pass.stage === 'deck'));", 'render'),
    ('R02', 'the deck stage draws nothing (images)', OUTPUT_FX,
     '  drawDeck(frame, passes) {\n    this.drawStage(frame, passes);\n  }', '  drawDeck(frame, passes) {}', 'render'),
    ('R03', 'the passes of the deck are drawn twice (images)', OUTPUT_FX,
     "this.drawOutput(frame, passes.filter((pass) => pass.stage !== 'deck'));", 'this.drawOutput(frame, passes);', 'render'),
    # -- the window --
    ('W01', 'the option the clip already holds is offered as a move', WINDOW,
     '            let enabled = option.applicable && !selected;', '            let enabled = option.applicable;', 'desktop'),
    ('W02', 'add on the shot does not cut the clip at the end of the shot', WINDOW,
     'spec.typical_duration_frames.min(end - frame)', 'spec.typical_duration_frames', 'desktop'),
    ('W03', 'the shot of the monitor is the first one', WINDOW,
     '    shot_at(&view.shots, frame).ok_or(NOT_IN_A_SHOT)', '    view.shots.first().ok_or(NOT_IN_A_SHOT)', 'desktop'),
    ('W04', 'add on the shot is enabled outside the edit', WINDOW,
     '        Err(reason) => (false, Some(reason.to_string())),', '        Err(_) => (true, None),', 'desktop'),
]

COMMANDS = {
    'core': (['./scripts/cargo', 'test', '-p', 'tessel-editor-core', '--locked', '--no-fail-fast',
              '--lib', '--test', 'scene_fx_shot_scope'], ROOT),
    'js': (['node', '--test', '--test-reporter=tap', 'test/output-fx.test.mjs', 'test/directed-scene.test.mjs'], ROOT / 'runtime'),
    'render': (['./scripts/cargo', 'test', '-p', 'tessel-editor-render', '--locked', '--no-fail-fast',
                '--test', 'scene_fx_shot_scope'], ROOT),
    'desktop': (['./scripts/cargo', 'test', '-p', 'tessel-editor', '--locked', '--no-fail-fast', 'scene_fx_scope'], ROOT),
}


def digest(path):
    return hashlib.sha256((ROOT / path).read_bytes()).hexdigest()


def git_state(paths):
    result = subprocess.run(['git', 'status', '--porcelain', '--', *paths], cwd=ROOT, capture_output=True, text=True)
    return result.stdout.splitlines()


def run(command, cwd, env=None):
    try:
        result = subprocess.run(command, cwd=cwd, capture_output=True, text=True, env=env)
    except OSError as error:  # the runner is missing: an execution error, never a verdict
        return 127, f'{command[0]}: {error}'
    return result.returncode, result.stdout + result.stderr


def disk_ok(kind):
    step = STEP_GIB[kind]
    if step == 0:
        return True, ''
    code, out = run(['./scripts/disk-guard.sh', str(step), str(FLOOR_GIB)], ROOT)
    return code == 0, out.strip()


def failed_tests(out):
    names = {line.split()[1] for line in out.splitlines() if line.startswith('test ') and line.endswith('FAILED')}
    names |= {line.split(' - ', 1)[1].strip() for line in out.splitlines() if line.startswith('not ok') and ' - ' in line}
    return sorted(names)


def judge(code, out):
    """KILLED needs a test that ran and failed by name; anything else that is not green is the runner's."""
    names = failed_tests(out)
    if code == 0:
        return 'SURVIVED', names
    if names:
        return 'KILLED', names
    if 'error[' in out or 'could not compile' in out or 'SyntaxError' in out:
        return 'COMPILE-ERROR', names
    return 'EXECUTION-ERROR', names


def run_suite(kind):
    command, cwd = COMMANDS[kind]
    env = dict(os.environ, TESSEL_REQUIRE_RUNTIME='1') if kind == 'render' else None
    return run(command, cwd, env)


def rebuild_runtime():
    code, out = run(['./scripts/runtime', 'build'], ROOT)
    return code == 0, out[-400:]


class Interrupted(BaseException):
    """A signal turned into an exception: `finally` restores the file, then the run ends."""


def on_signal(number, _frame):
    raise Interrupted(signal.Signals(number).name)


def head():
    code, out = run(['git', 'rev-parse', '--short', 'HEAD'], ROOT)
    return out.strip() if code == 0 else 'unknown'


def load_report():
    path = OUT / 'mutation-report.json'
    if path.exists():
        return json.loads(path.read_text())
    return {'runs': [], 'mutants': {}}


def write_report(report, run_entry):
    """The report is merged across runs (a run may judge only some mutants, the disk may stop it);
    every run keeps its own entry, aborted or refused ones included."""
    OUT.mkdir(parents=True, exist_ok=True)
    report['runs'][-1] = run_entry
    verdicts = [row['result'] for row in report['mutants'].values()]
    report['summary'] = {verdict: verdicts.count(verdict) for verdict in sorted(set(verdicts))}
    report['mutants'] = dict(sorted(report['mutants'].items()))
    (OUT / 'mutation-report.json').write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')


def check_only(selected):
    bad = 0
    for ident, what, path, old, new, kind in selected:
        count = (ROOT / path).read_text().count(old)
        ok = count == 1 and old != new
        bad += 0 if ok else 1
        print(f'{ident} {"ok " if ok else "BAD"} {kind:7} {path} (occurrences: {count}) {what}')
    print('applicable:', len(selected) - bad, 'of', len(selected))
    return 0 if bad == 0 else 2


def main(argv):
    check = '--check' in argv
    kinds = {argv[i + 1] for i, arg in enumerate(argv) if arg == '--kind' and i + 1 < len(argv)}
    only = {arg for arg in argv if arg not in ('--check', '--kind') and arg not in kinds and not arg.startswith('--')}
    selected = [m for m in MUTANTS if (not only or m[0] in only) and (not kinds or m[5] in kinds)]
    if not selected:
        print('no mutant selected', file=sys.stderr)
        return 2
    if check:
        return check_only(selected)
    for number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(number, on_signal)
    files = sorted({m[2] for m in selected})
    git_before = git_state(files)
    before = {path: digest(path) for path in files}
    report = load_report()
    entry = {'started': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'), 'head': head(),
             'dirty_before': git_before, 'selected': [m[0] for m in selected], 'baselines': {}, 'stopped': None,
             'note': '', 'restored': None, 'hashes_before': before, 'hashes_after': None}
    report['runs'].append(entry)
    write_report(report, entry)
    baselines = entry['baselines']

    def record(ident, what, path, kind, result, **extra):
        report['mutants'][ident] = {'what': what, 'file': path, 'kind': kind, 'result': result,
                                    'run': len(report['runs']) - 1, 'head': entry['head'], **extra}
        write_report(report, entry)

    try:
        for ident, what, path, old, new, kind in selected:
            file = ROOT / path
            original = file.read_text()
            if original.count(old) != 1:
                record(ident, what, path, kind, 'NOT-APPLICABLE', count=original.count(old))
                print(ident, 'NOT APPLICABLE', original.count(old), flush=True)
                continue
            if kind not in baselines:
                ok, message = disk_ok(kind)
                if not ok:
                    entry['stopped'] = f'REFUSED_DISK before the {kind} baseline: {message}'
                    record(ident, what, path, kind, 'REFUSED_DISK', message=message)
                    print(ident, entry['stopped'], flush=True)
                    break
                if kind == 'render':
                    built, tail = rebuild_runtime()
                    if not built:
                        entry['stopped'] = f'the runtime does not build without a mutant: {tail}'
                        break
                started = time.time()
                code, out = run_suite(kind)
                baselines[kind] = {'exit': code, 'seconds': round(time.time() - started), 'failed': failed_tests(out),
                                   'tail': out[-300:] if code else ''}
                print('baseline', kind, 'exit', code, flush=True)
                write_report(report, entry)
                if code != 0:
                    entry['stopped'] = f'the {kind} suite is not green without a mutant: a verdict would mean nothing'
                    break
            ok, message = disk_ok(kind)
            if not ok:
                entry['stopped'] = f'REFUSED_DISK before {ident}: {message}'
                record(ident, what, path, kind, 'REFUSED_DISK', message=message)
                print(ident, entry['stopped'], flush=True)
                break
            started = time.time()
            try:
                file.write_text(original.replace(old, new))
                if kind == 'render':
                    built_mutant, tail = rebuild_runtime()
                    code, out = run_suite(kind) if built_mutant else (1, tail)
                else:
                    code, out = run_suite(kind)
            finally:
                file.write_text(original)
                if kind == 'render':
                    restored_dist, tail = rebuild_runtime()  # the dist must be the sound runtime again
                    if not restored_dist:
                        entry['note'] += f' runtime/dist could NOT be rebuilt after {ident}: {tail}'
            result, names = judge(code, out)
            extra = {'failed': names[:8], 'seconds': round(time.time() - started),
                     'original_sha256': hashlib.sha256(original.encode()).hexdigest()}
            if result in ('COMPILE-ERROR', 'EXECUTION-ERROR'):
                extra['tail'] = out[-600:]
            record(ident, what, path, kind, result, **extra)
            print(ident, result, names[:3], flush=True)
    except Interrupted as signal_name:
        entry['stopped'] = f'interrupted by {signal_name}'
    finally:
        after = {path: digest(path) for path in files}
        git_after = git_state(files)
        entry['hashes_after'] = after
        entry['restored'] = before == after and git_before == git_after
        entry['finished'] = entry['stopped'] is None
        write_report(report, entry)
    print('restored:', entry['restored'], '| git status of the mutated files unchanged:', git_before == git_after)
    if entry['stopped']:
        print('stopped:', entry['stopped'])
    judged = [report['mutants'][m[0]]['result'] for m in selected if m[0] in report['mutants']]
    clean = len(judged) == len(selected) and all(result in ('KILLED', 'SURVIVED') for result in judged)
    return 0 if entry['restored'] and entry['stopped'] is None and clean else 2


if __name__ == '__main__':
    raise SystemExit(main(sys.argv[1:]))
