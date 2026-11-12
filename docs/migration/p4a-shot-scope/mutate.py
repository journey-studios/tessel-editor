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
  browser  the same build / run / build again, with the compositor's real-browser tests of the
           effects (runtime/test/directed-scene-fx-browser.test.mjs): what a frame reports
  desktop  ./scripts/cargo test -p tessel-editor ... scene_fx_scope (the GPUI tests of the window) and the one
           of the handles of a clip on a shot (`scene_fx_handles_tests::a_vignette_on_a_shot`)

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
STEP_GIB = {'core': 2, 'js': 0, 'render': 2, 'browser': 0, 'desktop': 2}
REBUILDS = ('render', 'browser')  # the runners that draw with runtime/dist

SCOPE = 'crates/core/src/directed_scene/fx/scope.rs'
FX = 'crates/core/src/directed_scene/fx/mod.rs'
INTEGRITY = 'crates/core/src/directed_scene/integrity.rs'
VALIDATE = 'crates/core/src/scene_plan/validate.rs'
EDIT = 'crates/core/src/scene_edit/fx.rs'
EDIT_SCOPE = 'crates/core/src/scene_edit/fx_scope.rs'
INSPECTOR = 'crates/core/src/scene_edit/fx_inspector.rs'
CHOICE = 'crates/core/src/scene_edit/fx_choice.rs'
MODEL = 'crates/core/src/directed_scene/model.rs'
OUTPUT_FX = 'runtime/src/output-fx.js'
DIRECTED = 'runtime/src/directed-scene.js'
COMPOSITOR = 'runtime/src/compositor.js'
WINDOW = 'crates/desktop/src/workstation/scene_fx_scope.rs'
WINDOW_PANEL = 'crates/desktop/src/workstation/scene_fx.rs'
WINDOW_HANDLES = 'crates/desktop/src/workstation/scene_fx_handles.rs'

# (id, what the mutant breaks, file, text to replace (exactly once), replacement, runner)
MUTANTS = [
    # -- the one rule of the scope: where a shot clip may lie --
    ('C01', 'a clip that fills its shot to the last frame is refused (end bound off by one)', SCOPE,
     '    start_frame >= first && start_frame.saturating_add(duration_frames) <= last',
     '    start_frame >= first && start_frame.saturating_add(duration_frames) < last', 'core'),
    ('C02', 'a clip that starts on the first frame of its shot is refused (start bound off by one)', SCOPE,
     '    start_frame >= first && start_frame.saturating_add(duration_frames) <= last',
     '    start_frame > first && start_frame.saturating_add(duration_frames) <= last', 'core'),
    ('C03', 'a shot that does not exist is read as the first shot (UNKNOWN_SHOT never raised)', SCOPE,
     'let Some(shot) = shots.iter().find(|shot| &shot.id == id) else {', 'let Some(shot) = shots.first() else {', 'core'),
    ('C04', 'the first frame of the next shot still belongs to the shot that ends there', MODEL,
     'self.start_frame <= frame && frame < self.end_frame_exclusive',
     'self.start_frame <= frame && frame <= self.end_frame_exclusive', 'core'),
    ('C05', 'the first frame of a shot belongs to no shot', MODEL,
     'self.start_frame <= frame && frame < self.end_frame_exclusive',
     'self.start_frame < frame && frame < self.end_frame_exclusive', 'core'),
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
     '        passes.sort_by_key(|pass| pass.scope.stage());',
     '        passes.sort_by_key(|pass| std::cmp::Reverse(pass.scope.stage()));', 'core'),
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
     '        fx::scope_fault(&add.scope, &clip_id, add.start_frame, duration, span.shots)\n    {',
     '        fx::scope_fault(&add.scope, &clip_id, add.start_frame, duration, span.shots).filter(|_| false)\n    {', 'core'),
    ('C18', 'set_scope to the scope the clip already holds is an edit', EDIT_SCOPE,
     '    let noop = before == *scope;', '    let noop = false;', 'core'),
    ('C19', 'going back to the output writes {kind: output} instead of leaving the key out', EDIT_SCOPE,
     '            None => {\n                target.remove("scope");\n            }',
     '            None => {\n                target.insert("scope".to_string(), json!({ "kind": "output" }));\n            }', 'core'),
    ('C20', 'set_scope does not judge whether the clip fits the shot', EDIT_SCOPE,
     '        if let Some(fault) = fx::scope_fault(scope, clip, start, duration, span.shots) {',
     '        if let Some(fault) = fx::scope_fault(scope, clip, start, duration, span.shots).filter(|_| false) {', 'core'),
    ('C21', 'the mismatch is reported to commands with another code', EDIT,
     '        fx::FX_SCOPE_MISMATCH => "fx_scope_mismatch",', '        fx::FX_SCOPE_MISMATCH => "invalid_value",', 'core'),
    # -- the duration a clip gets when none is given (core.R1, core.R3) --
    ('C23', 'a clip added without a duration is not cut at the end of its shot', SCOPE,
     '.map_or(edit_frames, |shot| shot_limits(shot).1.min(edit_frames))',
     '.map_or(edit_frames, |_| edit_frames)', 'core'),
    ('C24', 'the default duration ignores where the edit ends', SCOPE,
     '.map_or(edit_frames, |shot| shot_limits(shot).1.min(edit_frames))',
     '.map_or(i64::MAX, |shot| shot_limits(shot).1)', 'core'),
    ('C25', 'a duration that is given is cut to what the scope holds (the core never clamps one)', EDIT,
     '        Some(duration) => duration,\n        None => fx::default_duration(',
     '        Some(duration) => duration.min(fx::default_duration(spec, &add.scope, add.start_frame, span.shots, span.frames)),\n        None => fx::default_duration(', 'core'),
    ('C26', 'the inspector restores the duration to the effect\'s typical one, whatever the scope holds', INSPECTOR,
     '    let typical = default_duration(clip.effect, &clip.scope, start, shots, edit_frames);',
     '    let typical = clip.effect.typical_duration_frames;', 'core'),
    # -- the inspector --
    ('C22', 'the inspector offers a shot the clip does not fit', CHOICE,
     '        let applicable = shot_holds(shot, clip.start_frame, clip.duration_frames);', '        let applicable = true;', 'core'),
    # -- the limits of the timing fields of a shot clip (the shot's, never the edit's) --
    ('C27', 'the timing fields of a shot clip are limited by the edit, not by its shot', CHOICE,
     '    if clip.scope == FxScope::Output {\n        return ((0, edit_frames - 1), edit_frames);',
     '    if true {\n        return ((0, edit_frames - 1), edit_frames);', 'core'),
    ('C28', 'the first start of a shot clip is the start of the edit', CHOICE,
     '        (scope_start(&clip.scope, shots), end - clip.duration_frames),',
     '        (0, end - clip.duration_frames),', 'core'),
    ('C29', 'the last start of a shot clip ignores how long the clip is', CHOICE,
     '        (scope_start(&clip.scope, shots), end - clip.duration_frames),',
     '        (scope_start(&clip.scope, shots), end - 1),', 'core'),
    ('C30', 'the longest duration of a shot clip counts from the start of its shot, not from the clip', CHOICE,
     '        end - clip.start_frame,\n    )',
     '        end - scope_start(&clip.scope, shots),\n    )', 'core'),
    # -- the runtime, pure --
    ('J01', 'a pass of a shot is accepted by a plan that does not require fxShotScope', OUTPUT_FX,
     "if (!requires.includes('fxShotScope'))", "if (false && !requires.includes('fxShotScope'))", 'js'),
    ('J02', 'a pass of another shot than the frame\'s is accepted', OUTPUT_FX,
     'if (raw.shotId !== frameShot) {', 'if (false && raw.shotId !== frameShot) {', 'js'),
    ('J03', 'the passes of the shot are accepted after the passes of the output', OUTPUT_FX,
     'if (position < lastPosition) throw', 'if (false && position < lastPosition) throw', 'js'),
    ('J04', 'the passes are drawn last first (the output before the deck)', OUTPUT_FX,
     '      passes.forEach((pass, index) => {', '      [...passes].reverse().forEach((pass, index) => {', 'js'),
    ('J05', 'the runtime does not advertise fxShotScope', DIRECTED,
     "  'fxShotScope',\n", '', 'js'),
    ('J06', 'the passes of a shot are never drawn', OUTPUT_FX,
     '      passes.forEach((pass, index) => {',
     "      passes.filter((pass) => pass.stage !== 'deck').forEach((pass, index) => {", 'js'),
    # -- the runtime, drawn: the same places, judged by the images and by what a frame reports --
    ('R01', 'the passes are drawn last first (images)', OUTPUT_FX,
     '      passes.forEach((pass, index) => {', '      [...passes].reverse().forEach((pass, index) => {', 'render'),
    ('R02', 'the passes of a shot are never drawn (images)', OUTPUT_FX,
     '      passes.forEach((pass, index) => {',
     "      passes.filter((pass) => pass.stage !== 'deck').forEach((pass, index) => {", 'render'),
    ('R03', 'the passes of the deck are drawn twice (images)', OUTPUT_FX,
     '      passes.forEach((pass, index) => {',
     "      [...passes, ...passes.filter((pass) => pass.stage === 'deck')].forEach((pass, index) => {", 'render'),
    ('J07', 'the frame counts one pass of a shot too many (telemetry)', COMPOSITOR,
     '(frame.sources.directed_scene_fx_shot ?? 0) + deckPasses;',
     '(frame.sources.directed_scene_fx_shot ?? 0) + deckPasses + 1;', 'browser'),
    # -- the window --
    ('W01', 'the option the clip already holds is offered as a move', WINDOW,
     '            let enabled = option.applicable && !selected;', '            let enabled = option.applicable;', 'desktop'),
    ('W03', 'the shot of the monitor is the first one', WINDOW,
     '    let shot = shot_at(&view.shots, frame).ok_or(NOT_IN_A_SHOT)?;',
     '    let shot = view.shots.first().ok_or(NOT_IN_A_SHOT)?;', 'desktop'),
    ('W04', 'add on the shot is enabled outside the edit', WINDOW,
     '        Err(reason) => (false, Some(reason.to_string())),', '        Err(_) => (true, None),', 'desktop'),
    ('W05', 'a pick of another scene edits the scene the panel shows now', WINDOW,
     '&& let Ok(view) = self.fx_view_of(&key.scene)', '&& let Some(view) = self.fx_view()', 'desktop'),
    ('W06', 'the selector of a clip that is gone is kept', WINDOW,
     '.retain(|key, _| key.scene == view.scene && view.clip(&key.clip).is_some());',
     '.retain(|key, _| key.scene == view.scene);', 'desktop'),
    ('W08', 'a pick reaches a document other than the one its selector was painted for', WINDOW,
     '.is_some_and(|pick| pick.revision == self.state.project.revision);', '.is_some();', 'desktop'),
    ('W09', 'the selector of a clip is not kept in step with the revision it is painted at', WINDOW,
     '        pick.revision = revision;\n', '', 'desktop'),
    ('W10', 'the handles action of a vignette on a shot is enabled', WINDOW_HANDLES,
     '    clip.handles_refusal().map(|refusal| refusal.text(LOCALE))',
     '    clip.handles_refusal().filter(|_| clip.scope.is_output()).map(|refusal| refusal.text(LOCALE))', 'desktop'),
    ('W11', 'the handles button of a vignette on a shot is a live control (the refusal comes from the core)', WINDOW_HANDLES,
     '        let button = if let Some(reason) = no_handles_text(clip) {',
     '        let button = if let Some(reason) = no_handles_text(clip).filter(|_| clip.scope.is_output()) {', 'desktop'),
    ('W12', 'the node publishes a longer reach than the core computed for the timing fields', WINDOW_PANEL,
     '"max": control.max, "step": control.step, "integer": control.integer,',
     '"max": control.max + 1.0, "step": control.step, "integer": control.integer,', 'desktop'),
    ('W07', 'the hint of the section lists the adds on the output only', WINDOW_PANEL,
     '|view| action_names(&section_actions(&view)),',
     '|view| action_names(&section_actions(&view)).into_iter().filter(|name| name.starts_with(ADD_PREFIX)).collect::<Vec<_>>(),', 'desktop'),
]

COMMANDS = {
    'core': (['./scripts/cargo', 'test', '-p', 'tessel-editor-core', '--locked', '--no-fail-fast',
              '--lib', '--test', 'scene_fx_shot_scope'], ROOT),
    'js': (['node', '--test', '--test-reporter=tap', 'test/output-fx.test.mjs', 'test/directed-scene.test.mjs'], ROOT / 'runtime'),
    'render': (['./scripts/cargo', 'test', '-p', 'tessel-editor-render', '--locked', '--no-fail-fast',
                '--test', 'scene_fx_shot_scope'], ROOT),
    'browser': (['node', '--test', '--test-reporter=tap', 'test/directed-scene-fx-browser.test.mjs'], ROOT / 'runtime'),
    # `cargo test` takes ONE test name before the `--`; a second filter is the libtest's own, after it (they add up).
    'desktop': (['./scripts/cargo', 'test', '-p', 'tessel-editor', '--locked', '--no-fail-fast', 'scene_fx_scope',
                 '--', 'scene_fx_handles_tests::a_vignette_on_a_shot'], ROOT),
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
    env = dict(os.environ, TESSEL_REQUIRE_RUNTIME='1') if kind in REBUILDS else None
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
    unknown = sorted(only - {m[0] for m in MUTANTS})
    if unknown:  # a typo must not run a smaller batch and exit 0: refuse before anything is read, built or mutated
        print('unknown mutant id(s): ' + ' '.join(unknown), file=sys.stderr)
        return 2
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
             'note': '', 'restored': None,
             # `dist_restored`: runtime/dist rebuilt from the restored source after the last mutant of a runner that
             # draws with it (null: none ran). The corridor of the handles (`g41-handles`) rebuilds ONCE, at the end of
             # the run, and calls it `dist_rebuilt`: same invariant, one build there and one per mutant here.
             'dist_restored': None, 'hashes_before': before, 'hashes_after': None}
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
                if kind in REBUILDS:
                    built, tail = rebuild_runtime()
                    if not built:
                        entry['dist_restored'] = False  # runtime/dist is whatever the failed build left
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
            dist_broken = None
            try:
                file.write_text(original.replace(old, new))
                if kind in REBUILDS:
                    built_mutant, tail = rebuild_runtime()
                    code, out = run_suite(kind) if built_mutant else (1, tail)
                else:
                    code, out = run_suite(kind)
            finally:
                file.write_text(original)
                if kind in REBUILDS:
                    # False until a build that FINISHED says otherwise: a signal in the middle of it (`Interrupted`
                    # leaves this `finally`) must not leave the old None/True standing over a dist that holds the mutant
                    entry['dist_restored'] = False
                    restored_dist, tail = rebuild_runtime()  # the dist must be the sound runtime again
                    entry['dist_restored'] = restored_dist
                    if not restored_dist:
                        dist_broken = tail
            result, names = judge(code, out)
            extra = {'failed': names[:8], 'seconds': round(time.time() - started),
                     'original_sha256': hashlib.sha256(original.encode()).hexdigest()}
            if result in ('COMPILE-ERROR', 'EXECUTION-ERROR'):
                extra['tail'] = out[-600:]
            record(ident, what, path, kind, result, **extra)
            print(ident, result, names[:3], flush=True)
            if dist_broken is not None:
                # runtime/dist holds this mutant and the run cannot be a sound state any more: the batch ends here
                # and the run is never `restored` (a note nobody reads is not a stop). The next mutant of a runner
                # that draws would rebuild before it tests; the point is what is left on disk, not that verdict.
                entry['stopped'] = (f'runtime/dist could NOT be rebuilt after {ident}, it holds the mutant '
                                    f'(run `./scripts/runtime build`): {dist_broken}')
                break
    except Interrupted as signal_name:
        entry['stopped'] = f'interrupted by {signal_name}'
    finally:
        after = {path: digest(path) for path in files}
        git_after = git_state(files)
        entry['hashes_after'] = after
        entry['restored'] = before == after and git_before == git_after and entry['dist_restored'] is not False
        entry['finished'] = entry['stopped'] is None
        write_report(report, entry)
    print('restored:', entry['restored'], '| git status of the mutated files unchanged:', git_before == git_after,
          '| runtime/dist rebuilt from the restored source:', entry['dist_restored'])
    if entry['stopped']:
        print('stopped:', entry['stopped'])
    judged = [report['mutants'][m[0]]['result'] for m in selected if m[0] in report['mutants']]
    clean = len(judged) == len(selected) and all(result in ('KILLED', 'SURVIVED') for result in judged)
    return 0 if entry['restored'] and entry['stopped'] is None and clean else 2


if __name__ == '__main__':
    raise SystemExit(main(sys.argv[1:]))
