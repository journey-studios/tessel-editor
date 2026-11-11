#!/usr/bin/env python3
"""Mutation proof of the gizmo performance suite (F055 / BL015.21).

    python3 scripts/gizmo-perf/mutate.py [--out DIR] [--only NAME ...]

A suite that cannot fail proves nothing. Each mutant below puts one real performance defect into the PRODUCT code
(not into the measuring code: that would be the artificial injection of the negative controls inside
`scene_gizmo_perf_budget`, which stays a control and is never counted here), runs the detectors that must catch it,
restores the file and checks its hash:

  extra_plan_per_handler  every gizmo plan is built twice          -> structural (core) AND time must fail
  delay_1ms_per_handler   `apply_gizmo` sleeps 1 ms               -> structural must PASS (no extra layout: the
                                                                      counter cannot see a delay), time must FAIL
  memo_off                `overlay_memo_hit` never hits            -> host idle-window test must fail
  replan_per_tick         `workflow_tick` asks for the overlay     -> host idle-window test must fail

The time detector is the controlled release run (`scene_gizmo_perf_budget`, judged against `budgets.json`) under the
performance lock, taken once for all the time runs and never waited for (BLOCKED, exit 3). INCONCLUSIVE is not
"killed". The mutated source is restored in a `finally` and the file hash compared with `git`'s; a mismatch aborts
loudly. Exit 0 only when every mutant was detected by every detector that must detect it.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import harness  # noqa: E402
import run as runner  # noqa: E402

ROOT = harness.ROOT
GIZMO = 'crates/core/src/scene_project/gizmo.rs'
OVERLAY = 'crates/desktop/src/workstation/viewport_overlay.rs'
WORKFLOW = 'crates/desktop/src/workstation/workflow.rs'
STRUCTURE = ['./scripts/cargo', 'test', '--locked', '-p', 'tessel-editor-core', '--test', 'scene_gizmo_perf_structure']
HOST = ['./scripts/cargo', 'test', '--locked', '-p', 'tessel-editor', 'scene_gizmo_perf_tests::an_idle']

REPLAN_TICK = '''pub(super) fn workflow_tick(&mut self, cx: &mut Context<Self>) {
        if let Some(scene) = self.viewport.directed_scene().map(str::to_owned) {
            let frame = self.state.timeline.playhead;
            let _ = self.state.execute(crate::state::Command::Scene(
                tessel_editor_core::scene_project::SceneCommand::Gizmo(
                    tessel_editor_core::scene_interaction::GizmoCommand::State {
                        scene,
                        at: tessel_editor_core::scene_interaction::FrameAt::Frame(frame),
                    },
                ),
            ));
        }'''

MUTANTS = {
    'extra_plan_per_handler': {
        'file': GIZMO,
        'old': 'fn plan_of(scene: &DirectedScene, frame: i64) -> Result<ScenePlan, CommandError> {\n',
        'new': 'fn plan_of(scene: &DirectedScene, frame: i64) -> Result<ScenePlan, CommandError> {\n'
               '    let _ = ScenePlan::build_with(scene, frame, PlanOptions { allow_partial: true });\n',
        'detectors': {'structural': True, 'time': True},
    },
    'delay_1ms_per_handler': {
        'file': GIZMO,
        'old': 'pub(super) fn apply_gizmo(&mut self, command: GizmoCommand) -> Result<Value, CommandError> {\n',
        'new': 'pub(super) fn apply_gizmo(&mut self, command: GizmoCommand) -> Result<Value, CommandError> {\n'
               '        std::thread::sleep(std::time::Duration::from_millis(1));\n',
        'detectors': {'structural': False, 'time': True},
    },
    'memo_off': {
        'file': OVERLAY,
        'old': 'let hit = self.overlay.enabled\n',
        'new': 'let hit = false\n            && self.overlay.enabled\n',
        'detectors': {'host': True},
    },
    'replan_per_tick': {
        'file': WORKFLOW,
        'old': 'pub(super) fn workflow_tick(&mut self, cx: &mut Context<Self>) {',
        'new': REPLAN_TICK,
        'detectors': {'host': True},
    },
}


def committed_hash(path):
    return harness.out(['git', 'rev-parse', f'HEAD:{path}'])


def file_hash(path):
    return harness.out(['git', 'hash-object', path])


class Mutated:
    """Applies a mutant and ALWAYS puts the file back (and proves it did)."""

    def __init__(self, mutant):
        self.mutant = mutant
        self.path = ROOT / mutant['file']

    def __enter__(self):
        self.original = self.path.read_bytes()
        text = self.original.decode()
        if text.count(self.mutant['old']) != 1:
            raise SystemExit(f"the anchor of the mutant is not unique in {self.mutant['file']}")
        self.path.write_text(text.replace(self.mutant['old'], self.mutant['new']))
        return self

    def __exit__(self, *exc):
        self.path.write_bytes(self.original)
        if file_hash(self.mutant['file']) != committed_hash(self.mutant['file']):
            raise SystemExit(f"{self.mutant['file']} was NOT restored to HEAD: stop and inspect")


def detect(command):
    done = harness.run(command)
    text = done.stdout + done.stderr
    return {'command': command, 'exit': done.returncode, 'detected': done.returncode != 0 and 'test result: FAILED' in text,
            'failed_lines': [l for l in text.splitlines() if l.startswith('test ') and l.endswith('FAILED')][:8],
            'panic': [l for l in text.splitlines() if 'panicked' in l or l.startswith('  ')][:6],
            'compile_error': 'error[E' in text or 'could not compile' in text}


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('--out', default='/tmp/gizmo-perf-mutants')
    parser.add_argument('--only', nargs='*')
    parser.add_argument('--phase', choices=['all', 'prepare', 'time'], default='all',
                        help='`prepare`: structural/host detectors and the release builds of the time mutants '
                             '(the builds leave the machine loaded, which the guard refuses); `time`: only the lock '
                             'and the controlled runs, from the executables `prepare` left in --out/exes')
    args = parser.parse_args(argv)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    names = args.only or list(MUTANTS)
    manifest = {'suite': 'gizmo-perf-mutants', 'utc_start': harness.utc(), 'identity': harness.git_identity(),
                'toolchain': harness.toolchain(), 'mutants': {}, 'status': 'ABORTED'}
    manifest_path = out / 'manifest.json'
    for name in names:
        path = MUTANTS[name]['file']
        if file_hash(path) != committed_hash(path):
            manifest.update(status='REFUSED', reason=f'{path} is not at HEAD: mutants start from a committed tree')
            return harness.die(1, manifest, manifest_path)
    exes_dir = out / 'exes'
    exes_dir.mkdir(exist_ok=True)
    manifest_path = out / ('manifest.json' if args.phase != 'time' else 'manifest.time.json')
    previous = out / 'manifest.json'
    if args.phase == 'time':
        manifest['mutants'] = {name: record for name, record in json.loads(previous.read_text())['mutants'].items()
                               if name in names}
    time_exes = {}
    try:
        # Everything that is not a measurement first: structural and host detectors, and the release builds
        # of the time mutants (executables copied out; the source is restored right after each build).
        for name in names if args.phase != 'time' else []:
            mutant = MUTANTS[name]
            record = manifest['mutants'][name] = {'file': mutant['file'], 'old': mutant['old'], 'new': mutant['new'],
                                                 'must': mutant['detectors']}
            with Mutated(mutant):
                if 'structural' in mutant['detectors']:
                    record['structural'] = detect(STRUCTURE)
                if 'host' in mutant['detectors']:
                    record['host'] = detect(HOST)
                if 'time' in mutant['detectors']:
                    command, done, exe = runner.build()
                    record['time_build'] = {'exit': done.returncode, 'executable_sha256': harness.sha256(exe) if exe else None}
                    if exe and done.returncode == 0:
                        shutil.copy(exe, exes_dir / f'{name}.exe')
        # The time runs, under the lock, once.
        if args.phase != 'prepare':
            time_exes = {name: exes_dir / f'{name}.exe' for name in names
                         if 'time' in MUTANTS[name]['detectors'] and (exes_dir / f'{name}.exe').exists()}
        if time_exes:
            state, reason = harness.guards()
            manifest['machine_before'] = state
            if reason:
                for name in time_exes:
                    manifest['mutants'][name]['time'] = {'status': 'BLOCKED', 'reason': reason}
            else:
                with harness.Lock() as lock:
                    for name, exe in time_exes.items():
                        if not lock.held:
                            manifest['mutants'][name]['time'] = {'status': 'BLOCKED', 'reason': 'performance lock busy'}
                            continue
                        report = out / f'{name}.report.json'
                        env = {**os.environ, 'TESSEL_GIZMO_PERF_OUT': str(report)}
                        done = subprocess.run([str(exe), '--ignored', '--test-threads=1', '--nocapture',
                                               'the_gizmo_handlers_stay_within_their_budget_cold_and_hot_on_every_corpus'],
                                              cwd=ROOT, env=env, capture_output=True, text=True)
                        (out / f'{name}.time.stdout.txt').write_text(done.stdout)
                        text = done.stdout + done.stderr
                        inconclusive = 'INCONCLUSIVE' in text
                        failed = [l.strip() for l in text.splitlines() if 'x the plan (budget' in l or 'over a 60 Hz frame' in l]
                        manifest['mutants'][name]['time'] = {
                            'status': 'INCONCLUSIVE' if inconclusive else ('DETECTED' if done.returncode != 0 else 'MISSED'),
                            'exit': done.returncode, 'failures_seen': failed[:12],
                            'stdout_sha256': harness.sha256(out / f'{name}.time.stdout.txt')}
                manifest['machine_after'] = harness.machine()
    finally:
        if args.phase != 'prepare':
            shutil.rmtree(exes_dir, ignore_errors=True)  # discardable, 20 MB each
    if args.phase == 'prepare':
        manifest['status'] = 'PREPARED'
        harness.write(manifest_path, manifest)
        print(json.dumps({'status': 'PREPARED', 'structural_and_host': {n: {d: r[d]['detected'] for d in ('structural', 'host') if d in r} for n, r in manifest['mutants'].items()}}))
        return 0
    ok = True
    for name, record in manifest['mutants'].items():
        verdict = {}
        for detector, must in record['must'].items():
            if detector == 'time':
                got = record.get('time', {}).get('status') == 'DETECTED'
            else:
                got = bool(record.get(detector, {}).get('detected'))
            if record.get(detector, {}).get('compile_error'):
                got = False  # a mutant that does not compile was not detected by the suite
            verdict[detector] = {'must_detect': must, 'detected': got, 'as_expected': got == must}
            ok &= got == must
        record['verdict'] = verdict
    manifest['status'] = 'ALL_AS_EXPECTED' if ok else 'UNEXPECTED'
    manifest['utc_end'] = harness.utc()
    manifest['exit'] = 0 if ok else 1
    harness.write(manifest_path, manifest)
    print(json.dumps({'status': manifest['status'], 'mutants': {n: r['verdict'] for n, r in manifest['mutants'].items()}}, indent=1))
    return manifest['exit']


if __name__ == '__main__':
    raise SystemExit(main())
