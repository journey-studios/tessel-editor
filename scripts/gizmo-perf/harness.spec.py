#!/usr/bin/env python3
"""Spec of what the gizmo performance runners write down (R1): no path of the machine, no list of processes.

BL015.24: `gizmo-perf.spec.py` (too long) is to be split one spec per module; this file is the one of `harness.py` and of
the way each runner reaches it. No benchmark, no window, no real lock, no real `ps`: the outputs are doubles."""
import atexit
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import signal
import sys
import tempfile
import unittest
import importlib.util
from unittest.mock import MagicMock, patch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import harness  # noqa: E402  (the one module object every runner shares through `sys.modules`)


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


runner, ab, mutate, native, audit = (load(name) for name in ('run', 'ab', 'mutate', 'native', 'audit'))

# The per-checkout build lock lives under /tmp: a spec run must neither wait for a real build of this checkout nor hold it.
CHECKOUT_LOCKS = tempfile.mkdtemp(prefix='gizmo-perf-harness-spec-locks-')
atexit.register(shutil.rmtree, CHECKOUT_LOCKS, ignore_errors=True)
harness.CHECKOUT_LOCK_DIR = CHECKOUT_LOCKS

# Literals that a published file must not hold. Built by concatenation: this file is itself swept for them.
USERS, VOLUMES, PRIVATE = '/' + 'Users/', '/' + 'Volumes/', '/' + 'private/'
PROCESS_LIST = 'top_cpu' + '_processes'
FOREIGN = USERS + 'someone-else-xyz/.cargo/registry/src/index-1/foo-1.0.0'   # a path of this machine, not the checkout's
CLEAN = {'head': 'h' * 40, 'tree': 't' * 40, 'branch': 'b', 'dirty': ''}


ABSOLUTE = re.compile(r"(?<![\w./~<>-])/\w")   # the start of an absolute path: this spec's own detector, not the code's


def leaks(text):
    """Everything in `text` that names this machine: its home, checkout or temporary directories, a path, a process list."""
    machine = [harness.ROOT.as_posix(), os.path.expanduser('~'), tempfile.gettempdir(), USERS, VOLUMES, PRIVATE, PROCESS_LIST]
    return [word for word in machine if len(word) > 1 and word in text] + ABSOLUTE.findall(text)


def done(stdout='', code=0, stderr=''):
    return harness.Done(['x'], code, stdout, stderr, False, 1.0, None)


def run_main(module, argv):
    """A runner's `main` as the command line calls it: stdout kept off the test output, signal handlers put back."""
    saved = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGINT)}
    try:
        with contextlib.redirect_stdout(io.StringIO()):
            return module.main(argv)
    finally:
        for sig, handler in saved.items():
            signal.signal(sig, handler)


def argv_of(script, out):
    """What `sys.argv` holds when a runner is started by its absolute path."""
    return [str(HERE / script), '--phase', 'build', '--out', str(out)]


def noise(root):
    """The tail of a compiler's output: paths of the checkout, of a temporary directory and of somebody's cargo home."""
    return (f'error: linking with `cc` failed\n  = note: {root}/target/x.o\n  --> {harness.ROOT}/crates/core/src/x.rs:1:2\n'
            f'  --> {FOREIGN}/lib.rs:3:1\n  = note: copied to {os.path.expanduser("~")}/cache/y')


class Machine(unittest.TestCase):
    OUTPUTS = {'pmset -g therm': 'Note: No thermal warning level has been recorded', 'sysctl -n hw.memsize': '17179869184',
               'sysctl -n machdep.cpu.brand_string': 'Test CPU', 'pmset -g batt': "Now drawing from 'AC Power'\n -Battery (id=1)"}

    def machine(self):
        asked = []

        def out(command, cwd=None):
            asked.append(' '.join(command))
            if command[0] == 'ps':  # what `ps` would print: processes, with the paths they run from
                return f'  1 5.0 {VOLUMES}Some Disk/App.app/Contents/MacOS/App\n  2 4.0 {USERS}someone/bin/tool'
            return self.OUTPUTS.get(' '.join(command), '')

        with patch.object(harness, 'out', out), patch.object(harness, 'free_gib', return_value=40.0):
            return harness.machine(), asked

    def test_the_machine_record_holds_no_process_list_and_no_path(self):
        state, asked = self.machine()
        self.assertNotIn(PROCESS_LIST, state)
        self.assertEqual(leaks(json.dumps(state)), [])
        self.assertEqual([command for command in asked if command.split()[0] == 'ps'], [])  # nobody even asks for it

    def test_the_machine_record_keeps_what_a_reader_needs_to_judge_a_measurement(self):
        state, _ = self.machine()
        self.assertEqual(sorted(state), sorted(['utc', 'platform', 'machine', 'cpu', 'cpus', 'memory_bytes', 'loadavg',
                                                'thermal', 'thermal_state', 'power', 'free_gib']))
        self.assertEqual((state['cpu'], state['memory_bytes'], state['free_gib']), ('Test CPU', '17179869184', 40.0))
        self.assertEqual(state['thermal_state']['status'], 'no_limit_reported')


class Redact(unittest.TestCase):
    def test_a_manifest_is_written_without_a_path_and_keeps_what_its_reader_needs(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            manifest = {
                'cwd': str(harness.ROOT), 'command': [str(HERE / 'run.py'), '--phase', 'build', f'--out={tmp}/out'],
                'build': {'command': ['./scripts/cargo', 'test'], 'executable': VOLUMES + 'Some Disk/mbx/deps/budget-1a2b',
                          'stderr_tail': noise(tmp)},
                'nested': [[{'where': [os.path.expanduser('~') + '/proj/file.png'], FOREIGN: 'a key that is a path'}]],
                # what a later phase reads, or a person: none of it is a path, none of it may change
                'identity': {'head': 'a' * 40, 'tree': 'b' * 40, 'dirty': ' M crates/core/src/x.rs'},
                'exe': 'exes/candidate.exe', 'binary_sha256': 'f' * 64, 'pairs': '3/4', 'ratio': 1.25, 'loadavg': (1.0, 2.0),
                'reason': 'load average (1 min) 5.88 over 5.5 (11 cpus); a / b and c/d, https://example.com/a/b'}
            path = harness.write(tmp / 'manifest.json', manifest)
            text = path.read_text()
            written, name = json.loads(text), tmp.name
        self.assertEqual(leaks(text), [])
        self.assertEqual(written['cwd'], '<checkout>')
        self.assertEqual(written['command'][0], '<checkout>/scripts/gizmo-perf/run.py')
        self.assertEqual(written['command'][3], f'--out=<tmp>/{name}/out')
        self.assertEqual(written['build']['executable'], '<abs>/budget-1a2b')  # a path with a space in it, cut at the name
        self.assertIn('--> <checkout>/crates/core/src/x.rs:1:2', written['build']['stderr_tail'])
        self.assertIn('--> <abs>/lib.rs:3:1', written['build']['stderr_tail'])
        self.assertIn('= note: copied to ~/cache/y', written['build']['stderr_tail'])
        self.assertEqual(written['nested'][0][0]['where'], ['~/proj/file.png'])
        self.assertIn('<abs>/foo-1.0.0', json.dumps(written['nested']))
        for kept in ('identity', 'exe', 'binary_sha256', 'pairs', 'ratio', 'reason'):
            self.assertEqual(written[kept], manifest[kept], kept)
        self.assertEqual(written['loadavg'], [1.0, 2.0])

    def test_what_is_not_an_absolute_path_is_left_alone_and_redacting_twice_changes_nothing(self):
        plain = ['crates/core/src/x.rs', './scripts/cargo', '../up/x', '3/4', 'and/or', 'a / b', 'text/plain',
                 'https://example.com/a/b', '<checkout>/a/b', '<tmp>/x', '<abs>/y', '~/z', '', 'no path at all']
        self.assertEqual(harness.redact(plain), plain)
        dirty = [FOREIGN + '/x.rs', f'see {FOREIGN}/x.rs: not found', {f'k {VOLUMES}a/b c/d': [(1, 'e')]}, None, 3, True]
        once = harness.redact(dirty)
        self.assertEqual(harness.redact(once), once)
        self.assertEqual(leaks(json.dumps(once)), [])

    def test_the_checkout_is_named_by_its_longest_prefix_and_a_sibling_is_not_cut(self):
        root = '/' + 'srv/work/repo'
        redacted = harness.redact([f'{root}/a.rs', f'{root}2/b.rs', f'{root}-old/c.rs', f'{root}'], root=root, environ={})
        self.assertEqual(redacted, ['<checkout>/a.rs', '<abs>/b.rs', '<abs>/c.rs', '<checkout>'])
        home = '/' + 'srv/work'
        with patch.dict(os.environ, {'HOME': home}):  # a checkout under the home directory stays the checkout
            self.assertEqual(harness.redact([f'{root}/a.rs', f'{home}/other/b.rs'], root=root, environ={}),
                             ['<checkout>/a.rs', '~/other/b.rs'])

    def test_every_temporary_directory_becomes_tmp_and_a_cut_path_is_still_a_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(harness.redact(f'{tmp}/out/report.json'), f'<tmp>/{Path(tmp).name}/out/report.json')
        self.assertEqual(harness.redact('/tmp/gizmo-perf-run'), '<tmp>/gizmo-perf-run')
        self.assertEqual(harness.redact(PRIVATE + 'tmp/x'), '<tmp>/x')
        cut = (VOLUMES + 'Some Disk/www/t3/new-wo')[:20]
        self.assertEqual(leaks(harness.redact(f'detail: {cut}')), [])

    def test_the_text_of_a_child_is_written_redacted(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / 'stdout.txt'
            harness.write_text(target, f'panicked at {harness.ROOT}/crates/x.rs:1:1\nsaw {FOREIGN}/a\nok\n')
            text = target.read_text()
        self.assertEqual(leaks(text), [])
        self.assertEqual(text, 'panicked at <checkout>/crates/x.rs:1:1\nsaw <abs>/a\nok\n')

    def test_what_die_prints_is_redacted_too(self):
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            self.assertEqual(harness.die(1, {'status': 'REFUSED', 'reason': f'{FOREIGN}/x does not exist', 'cwd': FOREIGN}, None), 1)
        self.assertEqual(json.loads(shown.getvalue()), {'status': 'REFUSED', 'reason': '<abs>/x does not exist'})


class Writers(unittest.TestCase):
    """The runners and `audit.py`, through their own writers and doubles: what each leaves in `--out` names no path."""

    def test_no_runner_writes_a_file_around_the_one_redacting_writer(self):
        for name in ('run', 'ab', 'mutate', 'native', 'audit'):
            around = re.compile(r"(?<!harness)\.write_text\(|json\.dump\(|'w'\)")  # `mutate.Mutated` edits the PRODUCT: not output
            lines = [line.strip() for line in (HERE / f'{name}.py').read_text().splitlines()
                     if around.search(line) and 'text.replace(' not in line]
            self.assertEqual(lines, [], f'{name}.py writes a file without `harness.write` / `harness.write_text`')

    def test_run_py_records_its_build_redacted_and_names_the_executable_relative_to_out(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            exe = root / 'target' / 'budget-1a2b'
            exe.parent.mkdir()
            exe.write_bytes(b'binary')
            build = MagicMock(return_value=(['cargo'], done(stderr=noise(root)), str(exe)))
            out = root / 'out'
            with patch.object(runner, 'build', build), patch.object(runner.harness, 'git_identity', return_value=CLEAN), \
                    patch.object(runner.harness, 'toolchain', return_value={}), \
                    patch.object(sys, 'argv', argv_of('run.py', out)):
                code = run_main(runner, ['--phase', 'build', '--out', str(out)])
            manifest, record = (json.loads((out / name).read_text()) for name in ('manifest.build.json', 'build.json'))
            texts = [(out / name).read_text() for name in ('manifest.build.json', 'build.json')]
        self.assertEqual((code, manifest['status']), (0, 'BUILT'))
        self.assertEqual(leaks(''.join(texts)), [])
        self.assertEqual(manifest['command'][0], '<checkout>/scripts/gizmo-perf/run.py')
        self.assertIn('--> <abs>/lib.rs:3:1', manifest['build']['stderr_tail'])
        # what `--phase measure` reads back is intact: the executable is relative to --out, the hash is the copy's
        self.assertEqual((record['exe'], record['head'], record['tree']), ('exes/candidate.exe', 'h' * 40, 't' * 40))
        self.assertEqual(record['binary_sha256'], hashlib.sha256(b'binary').hexdigest())

    def test_ab_py_records_a_failed_baseline_build_redacted(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            out = root / 'out'
            candidate = root / 'target-candidate'
            candidate.write_bytes(b'candidate')

            def make_baseline(ref, directory):
                src = Path(directory) / 'baseline-src'
                src.mkdir(parents=True, exist_ok=True)
                return {'src': src, 'product_files_that_differ': {'crates/core/src/x.rs': {}},
                        'product_files_outside_the_measurement': []}

            def build(cwd):
                if Path(cwd) == ab.ROOT:
                    return ['cargo'], done(), str(candidate)
                return ['cargo'], done(stderr=noise(root), code=101), str(root / 'target-baseline')

            with patch.object(ab.harness, 'git_identity', return_value=CLEAN), patch.object(ab.harness, 'toolchain', return_value={}), \
                    patch.object(ab.harness, 'git_strict', return_value=(True, ab.BASELINE)), \
                    patch.object(ab, 'make_baseline', side_effect=make_baseline), patch.object(ab, 'build', side_effect=build), \
                    patch.dict(os.environ, {}), patch.object(sys, 'argv', argv_of('ab.py', out)):
                for name in harness.BUILD_ENV:
                    os.environ.pop(name, None)
                code = run_main(ab, ['--phase', 'build', '--out', str(out)])
            text = (out / 'manifest.json').read_text()
            manifest, name = json.loads(text), root.name
        self.assertEqual((code, manifest['status']), (1, 'BUILD_FAILED'))
        self.assertEqual(leaks(text), [])
        self.assertEqual(manifest['command'][0], '<checkout>/scripts/gizmo-perf/ab.py')
        self.assertEqual(manifest['baseline_build']['cwd'], f'<tmp>/{name}/out/baseline-src')
        self.assertIn('--> <abs>/lib.rs:3:1', manifest['baseline_build']['stderr_tail'])
        self.assertEqual(manifest['baseline_commit'], ab.BASELINE)  # a commit id is not a path

    def test_mutate_py_records_the_output_of_a_timed_mutant_redacted(self):
        name = next(mutant for mutant, detector in mutate.EXPECT if detector == 'time')
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            ran = done(f'running\n{harness.ROOT}/crates/x.rs\n', 101, noise(root))

            def judged(scratch):
                scratch.update(guard_after='clean', machine_after={'loadavg': (0.0, 0.0, 0.0)})

            with patch.object(mutate.harness, 'run_bounded', return_value=ran), \
                    patch.object(mutate.harness, 'judge_after', side_effect=lambda scratch: judged(scratch)):
                record = mutate.run_time(root / 'm.exe', root / 'm.report.json', root, name)
            texts = [(root / f'{name}.time.{stream}.txt').read_text() for stream in ('stdout', 'stderr')]
            written = harness.sha256(root / f'{name}.time.stdout.txt')
        self.assertEqual(leaks(''.join(texts)), [])
        self.assertIn('<checkout>/crates/x.rs', texts[0])
        self.assertEqual(record['stdout_sha256'], written)  # the hash is of the file that was written

    def test_native_py_names_its_binary_relative_to_build_json_and_finds_it_again(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            (root / 'runtime/dist').mkdir(parents=True)
            (root / 'runtime/dist/runtime.js').write_text('runtime')
            exe = root / 'built' / 'tessel-editor'
            exe.parent.mkdir()
            exe.write_bytes(b'binary')
            line = json.dumps({'executable': str(exe), 'target': {'name': 'tessel-editor', 'kind': ['bin']}})
            out = root / 'out'
            state = {'free_gib': 100, 'loadavg': (0.0, 0.0, 0.0)}
            with patch.object(native, 'ROOT', root), patch.object(native.harness, 'git_identity', return_value=CLEAN), \
                    patch.object(native.harness, 'toolchain', return_value={}), \
                    patch.object(native.harness, 'run_bounded', return_value=harness.Done(['cargo'], 0, line + '\n', noise(root))), \
                    patch.object(native, 'runtime_stale_reason', return_value=None), \
                    patch.object(native, 'verify_corpus', return_value=None), \
                    patch.object(native, 'runtime_source_tree', return_value='rt'), \
                    patch.object(sys, 'argv', argv_of('native.py', out)):
                built = run_main(native, ['--phase', 'build', '--out', str(out)])
            record = json.loads((out / 'build.json').read_text())
            texts = [(out / name).read_text() for name in ('manifest.build.json', 'build.json')]
            with patch.object(native, 'ROOT', root), patch.object(native, 'verify_provenance', return_value=None), \
                    patch.object(native, 'run_corpus', side_effect=lambda *_: {'corpus': 'small', 'checks': [], 'pictures': [],
                                                                               'samples': []}), \
                    patch.object(native.harness, 'git_identity', return_value=CLEAN), \
                    patch.object(native.harness, 'toolchain', return_value={}), \
                    patch.object(native.harness, 'guards', return_value=(state, None)), \
                    patch.object(native.harness, 'claims', return_value=[]), patch.object(native.harness, 'machine', return_value={}):
                # no --binary, no --build-record: the binary is the one `build.json` names, relative to it
                ran = run_main(native, ['--phase', 'run', '--out', str(out), '--functional-only'])
            manifest = json.loads((out / 'manifest.json').read_text())
        self.assertEqual(built, 0)
        self.assertEqual(leaks(''.join(texts)), [])
        self.assertEqual(record['binary'], 'exes/tessel-editor')
        self.assertNotEqual(manifest['status'], 'PREREQUISITE_MISSING', manifest.get('reason'))  # the binary was found
        self.assertEqual((ran, manifest['status']), (0, 'FUNCTIONAL_ONLY_NOT_A_MEASUREMENT'))
        self.assertTrue(manifest['binary'].startswith('<tmp>/') and manifest['binary'].endswith('/out/exes/tessel-editor'),
                        manifest['binary'])

    def test_audit_py_writes_the_paths_it_was_given_redacted(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            budgets, report, out = root / 'budgets.json', root / 'report.json', root / 'audit.json'
            budgets.write_text(json.dumps({'ratios': {'small': {'read.rotate': {'p50_ratio': 2.0, 'p95_ratio': 4.0}}}}))
            report.write_text(json.dumps({'corpora': [{'corpus': 'small', 'reference': {'p50_ns': 1000.0}, 'cold': {},
                                                       'hot': {'read.rotate': {'ratio_p50': 1.0, 'ratio_p95': 2.0}}}]}))
            with contextlib.redirect_stdout(io.StringIO()):
                code = audit.main(['--budgets', str(budgets), '--out', str(out), str(report)])
            text = out.read_text()
        self.assertEqual(code, 0)
        self.assertEqual(leaks(text), [])
        written = json.loads(text)
        self.assertEqual(sorted(written['inputs']), ['budgets', 'reports'])
        self.assertTrue(written['inputs']['budgets'].endswith('/budgets.json'), written['inputs'])
        self.assertIn('small/read.rotate', written['kinds'])  # what the audit says is untouched


if __name__ == '__main__':
    unittest.main()
