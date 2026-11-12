#!/usr/bin/env python3
"""Self-test of the mutation runner (`mutate.py`): what it does when `runtime/dist` cannot be rebuilt from the
restored source (the build fails, or a signal ends it half way), a mutant id it does not know, and the shape of
the commands it sends to cargo.

Hermetic: the runner is loaded with its root and its report folder in a temporary directory that holds one synthetic
file and its own mutants, the runtime build, the suites, the disk guard and git are replaced, and the signal handlers
the runner installs are put back after each test. Nothing of this checkout is read for a verdict or written
(`runtime/src/output-fx.js` included: a test asserts its hash did not move).

Usage: `python3 docs/migration/p4a-shot-scope/test_mutate.py`."""
import contextlib
import hashlib
import importlib.util
import io
import json
import signal
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
REAL_TARGET = HERE.parents[2] / 'runtime' / 'src' / 'output-fx.js'
SOUND = 'const a = 1;\nconst b = 2;\n'
# Two mutants of a runner that draws with runtime/dist (`render`): one text each, exactly once in the synthetic file.
SYNTHETIC = [
    ('T1', 'the first line changes', 'src/a.js', 'const a = 1;', 'const a = 9;', 'render'),
    ('T2', 'the second line changes', 'src/a.js', 'const b = 2;', 'const b = 9;', 'render'),
]


def load():
    spec = importlib.util.spec_from_file_location('mutate_under_test', HERE / 'mutate.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)  # import only: the handlers are installed by `main`, the report folder is set below
    return module


class Runner(unittest.TestCase):
    def setUp(self):
        saved = {number: signal.getsignal(number) for number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)}
        self.addCleanup(lambda: [signal.signal(number, handler) for number, handler in saved.items()])
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root, self.out = Path(scratch.name) / 'checkout', Path(scratch.name) / 'report'
        (self.root / 'src').mkdir(parents=True)
        (self.root / 'src' / 'a.js').write_text(SOUND)
        self.real_before = hashlib.sha256(REAL_TARGET.read_bytes()).hexdigest()
        self.addCleanup(lambda: self.assertEqual(
            hashlib.sha256(REAL_TARGET.read_bytes()).hexdigest(), self.real_before, 'the real runtime source moved'))

    def module(self, calls, suites, builds=()):
        """The runner over the synthetic checkout; every call of the runtime build and of a suite is recorded in
        `calls`/`suites`. `builds` says what each call of the build does, in order (the baseline, then for each mutant
        the build with it and the build that restores the sound source): True or False, or 'signal' for a
        SIGTERM that arrives in the middle of it."""
        module = load()
        module.ROOT, module.OUT, module.MUTANTS = self.root, self.out, list(SYNTHETIC)
        module.disk_ok = lambda kind: (calls.append('disk') or True, '')
        module.git_state = lambda paths: []
        module.head = lambda: 'test'

        def rebuild():
            calls.append('build')
            outcome = builds[len([c for c in calls if c == 'build']) - 1]
            if outcome == 'signal':
                raise module.Interrupted('SIGTERM')
            return outcome, 'build output'

        def suite(kind):
            suites.append(kind)
            return (0, '') if len(suites) == 1 else (1, 'test some::test ... FAILED\ntest result: FAILED')

        module.rebuild_runtime, module.run_suite = rebuild, suite
        return module

    def run_batch(self, builds):
        """Runs the two synthetic runtime mutants (see `module` for `builds`)."""
        module = self.module([], [], builds)
        code = module.main([])
        report = json.loads((self.out / 'mutation-report.json').read_text())
        return code, report['runs'][-1], report['mutants']

    def test_a_restore_that_fails_stops_the_batch_and_is_never_restored(self):
        code, run, judged = self.run_batch([True, True, False])
        self.assertEqual(code, 2)
        self.assertEqual(len(judged), 1, 'the second mutant must not run over a dist that holds the first')
        self.assertIn('could NOT be rebuilt', run['stopped'])
        self.assertIs(run['dist_restored'], False)
        self.assertIs(run['restored'], False)
        self.assertEqual((self.root / 'src' / 'a.js').read_text(), SOUND, 'the source goes back whatever the build did')

    def test_a_baseline_build_that_fails_is_not_restored_either(self):
        code, run, judged = self.run_batch([False])
        self.assertEqual((code, judged), (2, {}))
        self.assertIs(run['dist_restored'], False)
        self.assertIs(run['restored'], False)

    def test_a_signal_in_the_middle_of_the_restoring_build_is_never_restored(self):
        """The `Interrupted` leaves the `finally` that rebuilds the dist: what the report said before (None, or True from
        an earlier mutant) must not stand over a dist that still holds the mutant."""
        code, run, judged = self.run_batch([True, True, 'signal'])
        self.assertEqual(code, 2)
        self.assertEqual(judged, {}, 'the interrupted mutant is not judged')
        self.assertIn('interrupted by SIGTERM', run['stopped'])
        self.assertIs(run['dist_restored'], False)
        self.assertIs(run['restored'], False)
        self.assertEqual((self.root / 'src' / 'a.js').read_text(), SOUND, 'the source goes back whatever interrupted')

    def test_a_signal_in_the_restoring_build_of_the_second_mutant_is_not_the_first_ones_restore(self):
        code, run, judged = self.run_batch([True, True, True, True, 'signal'])
        self.assertEqual(code, 2)
        self.assertEqual(list(judged), ['T1'], 'only the first mutant was judged')
        self.assertIs(run['dist_restored'], False, 'the True of the first restore must not survive the second')
        self.assertIs(run['restored'], False)

    def test_a_mutant_id_the_runner_does_not_know_is_refused_before_anything_runs(self):
        for argv in (['T1', 'T99', 'x7'], ['--check', 'T1', 'T99'], ['--kind', 'render', 'T99']):
            calls, suites = [], []
            module = self.module(calls, suites)
            installed = {number: signal.getsignal(number) for number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)}
            stderr, stdout = io.StringIO(), io.StringIO()
            with contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(stdout):
                code = module.main(argv)
            self.assertEqual(code, 2, argv)
            self.assertIn('unknown mutant id(s): ', stderr.getvalue(), argv)
            for unknown in sorted({'T99', 'x7'} & set(argv)):
                self.assertIn(unknown, stderr.getvalue())
            self.assertNotIn('T1', stderr.getvalue().replace('unknown mutant id(s): ', ''), 'a known id is not blamed')
            self.assertEqual((calls, suites), ([], []), f'{argv}: nothing may be built, run or measured')
            self.assertFalse(self.out.exists(), f'{argv}: no report is written for a refused request')
            self.assertEqual((self.root / 'src' / 'a.js').read_text(), SOUND)
            self.assertEqual(installed, {number: signal.getsignal(number) for number in installed},
                             'the signal handlers belong to a run that starts')

    def test_known_ids_still_select_just_those_mutants(self):
        calls, suites = [], []
        module = self.module(calls, suites, [True] * 3)
        with contextlib.redirect_stdout(io.StringIO()):
            code = module.main(['T2'])
        report = json.loads((self.out / 'mutation-report.json').read_text())
        self.assertEqual(code, 0)
        self.assertEqual(list(report['mutants']), ['T2'])

    def test_a_sound_run_is_restored(self):
        code, run, judged = self.run_batch([True] * 5)
        self.assertEqual(code, 0)
        self.assertEqual(len(judged), 2)
        self.assertIsNone(run['stopped'])
        self.assertIs(run['dist_restored'], True)
        self.assertIs(run['restored'], True)
        self.assertEqual((self.root / 'src' / 'a.js').read_text(), SOUND)


class Commands(unittest.TestCase):
    VALUE_FLAGS = {'-p', '--package', '--test', '--bin', '--example', '--bench'}

    def test_cargo_test_gets_one_test_name_before_the_dashes(self):
        """`cargo test a b` is an error ("unexpected argument"): the baseline would exit 1 and no mutant would be
        judged. A second filter goes after `--`, where it is the libtest's own."""
        module = load()
        for kind, (command, _cwd) in module.COMMANDS.items():
            if command[:2] != ['./scripts/cargo', 'test']:
                continue
            own = command[2:command.index('--')] if '--' in command else command[2:]
            names, skip = [], False
            for token in own:
                if skip:
                    skip = False
                elif token in self.VALUE_FLAGS:
                    skip = True
                elif not token.startswith('-'):
                    names.append(token)
            self.assertLessEqual(len(names), 1, f'{kind}: more than one test name before `--`: {names}')


if __name__ == '__main__':
    unittest.main()
