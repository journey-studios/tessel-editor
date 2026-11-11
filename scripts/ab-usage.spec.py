"""Usage of the A/B runners from the command line (one table over all of them): every count of sessions is a whole number of at least 1, and a wrong one is a
usage error (exit 2) before any prerequisite, lock, folder or editor is touched. No editor, GPU, runtime page or private media is started: each runner's
command is replaced by a recorder, so what is read here is only what `main()` lets through."""
import contextlib
import importlib.util
import io
import sys
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

# runner, its command line up to the count, the function `main()` hands the arguments to, and the options that count sessions
RUNNERS = [
    ("real-latency-ab.py", ["run", "--project", "p", "--out", "o"], "cmd_run", ["--pairs"]),
    ("first-use-ab.py", ["full", "--factor", "preupload", "--project", "p", "--out", "o"], "cmd_run", ["--pairs"]),
    ("runtime-bundle-ab.py", ["run", "--binary", "b", "--project", "p", "--unpatched-bundle", "u", "--out", "o"], "cmd_run", ["--pairs"]),
    ("playback-poll-ab.py", ["run", "--binary", "b", "--project", "p", "--out", "o"], "cmd_run", ["--pairs"]),
    ("viewport-transit-probe.py", ["ab", "--project", "p", "--out", "o"], "cmd_ab", ["--pairs"]),
    ("viewport-transit-probe.py", ["loop", "--project", "p", "--out", "o"], "cmd_loop", ["--iterations"]),
]


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_").removesuffix(".py") + "_usage", Path(__file__).with_name(name))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def invoke(runner, argv, command, extra):
    """(what `main()` ended with, the arguments the command got or None, what it said on stderr)."""
    module, seen, said = load(runner), [], io.StringIO()
    with mock.patch.object(module, command, side_effect=lambda args: seen.append(vars(args)) or 0), \
            mock.patch.object(sys, "argv", [runner, *argv, *extra]), contextlib.redirect_stderr(said):
        try:
            ended = module.main()
        except SystemExit as exit_:
            ended = exit_.code
    return ended, (seen[0] if seen else None), said.getvalue()


class CountTests(unittest.TestCase):
    def test_a_count_under_one_or_that_is_not_a_whole_number_is_refused_before_the_command_runs(self):
        for runner, argv, command, options in RUNNERS:
            for option in options:
                for value in ("0", "-3", "x", "1.5"):
                    with self.subTest(runner=runner, option=option, value=value):
                        ended, seen, said = invoke(runner, argv, command, [option, value])
                        self.assertEqual((ended, seen), (2, None), "a usage error, and the command (prerequisites, locks, folders) never ran")
                        self.assertIn("at least 1", said)

    def test_one_is_the_smallest_batch_and_the_defaults_are_batches(self):
        for runner, argv, command, options in RUNNERS:
            for option in options:
                key = option.lstrip("-").replace("-", "_")
                with self.subTest(runner=runner, option=option):
                    ended, seen, _ = invoke(runner, argv, command, [option, "1"])
                    self.assertEqual((ended, seen[key]), (0, 1))
                    ended, seen, _ = invoke(runner, argv, command, [])
                    self.assertEqual(ended, 0)
                    self.assertGreaterEqual(seen[key], 1)


if __name__ == "__main__":
    unittest.main()
