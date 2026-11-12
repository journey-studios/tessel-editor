#!/usr/bin/env python3
"""Self-test of the mutation runner (`mutate.py`): what it does when `runtime/dist` cannot be rebuilt from the
restored source, and the shape of the commands it sends to cargo.

Hermetic: the runner is loaded with its root and its report folder in a temporary directory that holds one synthetic
file and its own mutants, the runtime build, the suites, the disk guard and git are replaced, and the signal handlers
the runner installs are put back after each test. Nothing of this checkout is read for a verdict or written
(`runtime/src/output-fx.js` included: a test asserts its hash did not move).

Usage: `python3 docs/migration/p4a-shot-scope/test_mutate.py`."""
import hashlib
import importlib.util
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

    def run_batch(self, builds):
        """Runs the two synthetic runtime mutants; `builds` says whether each call of the runtime build succeeds, in
        order (the baseline, then for each mutant the build with it and the build that restores the sound source)."""
        module = load()
        module.ROOT, module.OUT, module.MUTANTS = self.root, self.out, list(SYNTHETIC)
        module.disk_ok = lambda kind: (True, '')
        module.git_state = lambda paths: []
        module.head = lambda: 'test'
        calls, suites = [], []

        def rebuild():
            calls.append(1)
            return builds[len(calls) - 1], 'build output'

        def suite(kind):
            suites.append(kind)
            return (0, '') if len(suites) == 1 else (1, 'test some::test ... FAILED\ntest result: FAILED')

        module.rebuild_runtime, module.run_suite = rebuild, suite
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
