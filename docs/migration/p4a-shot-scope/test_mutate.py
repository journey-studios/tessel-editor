#!/usr/bin/env python3
"""Self-test of the mutation runner (`mutate.py`): what it does when `runtime/dist` cannot be rebuilt from the
restored source. Runs nothing real: the runtime build and the suites are replaced, only the runner's own bookkeeping
is judged. Usage: `python3 docs/migration/p4a-shot-scope/test_mutate.py`."""
import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load(out):
    os.environ['TESSEL_MUTATION_OUT'] = out
    spec = importlib.util.spec_from_file_location('mutate_under_test', HERE / 'mutate.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.disk_ok = lambda kind: (True, '')
    return module


class DistRestore(unittest.TestCase):
    def run_batch(self, builds):
        """Runs two runtime mutants; `builds` says whether each call of the runtime build succeeds, in order
        (the baseline, then for each mutant the build with it and the build that restores the sound source)."""
        with tempfile.TemporaryDirectory() as out:
            module = load(out)
            calls, suites = [], []

            def rebuild():
                calls.append(1)
                return builds[len(calls) - 1], 'build output'

            def suite(kind):
                suites.append(kind)
                return (0, '') if len(suites) == 1 else (1, 'test some::test ... FAILED\ntest result: FAILED')

            module.rebuild_runtime, module.run_suite = rebuild, suite
            ids = [mutant[0] for mutant in module.MUTANTS if mutant[5] in module.REBUILDS][:2]
            code = module.main(ids)
            report = json.loads((Path(out) / 'mutation-report.json').read_text())
            return code, report['runs'][-1], report['mutants']

    def test_a_restore_that_fails_stops_the_batch_and_is_never_restored(self):
        code, run, judged = self.run_batch([True, True, False])
        self.assertEqual(code, 2)
        self.assertEqual(len(judged), 1, 'the second mutant must not run over a dist that holds the first')
        self.assertIn('could NOT be rebuilt', run['stopped'])
        self.assertIs(run['dist_restored'], False)
        self.assertIs(run['restored'], False)

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


if __name__ == '__main__':
    unittest.main()
