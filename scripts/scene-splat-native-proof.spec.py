"""Regression tests of the native splat proof's own guards (F048 / BL127); no editor, GPU, Chrome or runtime bundle.

`main()` of `scene-splat-native-proof.py` is driven end to end against a fake monitor (the script's real seams: its
`load_lifecycle` driver, its harness helpers and its corpus reader). Two lies must be refused instead of passing:

  * a corpus whose only splat is the `sp-order` one, so there is no isolated colour probe to judge;
  * a `scene.render` answer whose probe list is not the exact length that was asked for (a short answer used to be
    zipped into silence).

A genuine, complete answer still passes. A third group deletes each guard from a copy of the script and proves these
same checks then stop rejecting, so the tests discriminate the guards and not just the happy path.
"""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import re
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("scene-splat-native-proof.py")
STORED = [200, 60, 30]  # colour the file stores for the isolated splat
BACKGROUND = [10, 10, 10]
FAR = [[2, 2], [300, 170]]


def load(source=None):
    """A fresh copy of the proof (its `checks`/`failures` are module globals), optionally from mutated source."""
    if source is None:
        spec = importlib.util.spec_from_file_location("splat_native_proof", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module
    module = types.ModuleType("splat_native_proof_mutant")
    module.__file__ = str(SCRIPT)
    exec(compile(source, str(SCRIPT), "exec"), module.__dict__)
    return module


def corpus_case(frame, *, isolated):
    splats = [{"node": "order", "index": 0, "asset": "sp-order", "x": 100.0, "y": 50.0, "color": [255, 255, 255]}]
    if isolated:
        splats.append({"node": "cloud", "index": 1, "asset": "sp-cloud", "x": 160.0 + frame, "y": 90.0, "color": STORED})
    return {"splats": splats, "backgroundPoints": FAR}


def write_corpus(directory, *, isolated):
    expected = {"palette": {"background": BACKGROUND},
                "splat": {"renderSize": [320, 180], "cases": {str(f): corpus_case(f, isolated=isolated) for f in (0, 12, 24, 36, 47)}}}
    (directory / "expected.json").write_text(json.dumps(expected))
    (directory / "bundle.splat.json").write_text("{}")


class FakeLab:
    """The monitor driver: answers every semantic op; `scene.render` echoes probes by how `reply` shapes them."""

    def __init__(self, reply, isolated):
        self.reply, self.isolated = reply, isolated
        self.render_requests = []

    def wait_ready(self):
        pass

    def invoke(self, name):
        return True

    def stop(self):
        pass

    def seek(self, frame):
        pass

    def viewport(self):
        return {"phase": "ready", "session": {}, "capabilities": {"renderer": "fake"},
                "last_frame": {"hash": "H", "directed_scene_splats": 1}, "directed": {"slots": {}}}

    def op(self, request):
        if request["op"] != "scene.render":
            return True, {}
        self.render_requests.append(request)
        asked = len(request["probes"])
        n = 1 if self.isolated else 0
        # What an honest renderer reads: the stored colour at the centre, something else 40 px away, the flat background.
        honest = [STORED] * n + [[0, 0, 0]] * n + [BACKGROUND] * len(FAR)
        assert len(honest) == asked, (len(honest), asked)
        return True, {"frame": {"inspected": True, "hash": "H", "probes": self.reply(honest)}}


def fake_lifecycle(lab):
    module = types.SimpleNamespace(
        Lab=lambda binary, work, env: lab,
        settle=lambda lab_, frame, seconds: True,
        commit_job=lambda lab_, loaded, label: True,
        directed=lambda state: {"asset_identity": True, "slots": {"tracked_loads": 1, "identified_loads": 1, "occupied": 1}},
        cache_of=lambda state: {"scene_resources": 1},
    )
    return module


def run(module, *, isolated=True, reply=lambda honest: honest):
    """Drive `module.main()` through its own seams; returns (exit code, failed check names, render requests)."""
    module.checks.clear()
    module.failures.clear()
    lab = FakeLab(reply, isolated)
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        corpus = tmp / "corpus"
        corpus.mkdir()
        write_corpus(corpus, isolated=isolated)
        (tmp / "runtime" / "dist").mkdir(parents=True)
        (tmp / "runtime" / "dist" / "runtime.js").write_text("")
        binary = tmp / "tessel-editor"
        binary.write_text("")
        with contextlib.ExitStack() as stack:
            stack.enter_context(patch.object(sys, "argv", ["proof", "--corpus", str(corpus), "--binary", str(binary)]))
            stack.enter_context(patch.object(module, "ROOT", tmp))
            stack.enter_context(patch.object(module, "load_lifecycle", return_value=fake_lifecycle(lab)))
            stack.enter_context(patch.object(module.h, "free_gib", return_value=100.0))
            stack.enter_context(patch.object(module.h, "binary_info", return_value={}))
            stack.enter_context(patch.object(module.h, "git_state", return_value={}))
            stack.enter_context(patch.object(module.h, "tree_bytes", return_value=0))
            stack.enter_context(patch("time.sleep"))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            code = module.main()
    return code, list(module.failures), lab.render_requests


def only_cardinality(honest):
    """The renderer answers just the centres: every later slice (shifted, background) is empty and zips to nothing."""
    return honest[:1]


def one_short(honest):
    return honest[:-1]


def one_extra(honest):
    return honest + [BACKGROUND]


ISOLATED_GUARD = "the corpus has isolated colour probes"
CARDINALITY_GUARD = "every requested probe was returned"
FRAMES = 5


def rejects(module, needle, **scenario):
    """True when the proof exits 1 and every failure is the named guard (one per frame), not an incidental one."""
    code, failures, _ = run(module, **scenario)
    return code == 1 and len(failures) == FRAMES and all(needle in name for name in failures)


def remove_guard(source, guard):
    """The proof's source without the `if not check(<guard>...): continue` statement; refuses to mutate nothing."""
    pattern = re.compile(r'\n[ ]+if not check\(f"\{label\}: ' + re.escape(guard) + r'".*?\n[ ]+continue', re.DOTALL)
    mutated, count = pattern.subn("", source)
    if count != 1:
        raise AssertionError(f"expected to remove exactly one guard `{guard}`, removed {count}")
    return mutated


class IsolatedProbeGuardTests(unittest.TestCase):
    def test_a_corpus_with_only_the_order_splat_is_rejected_on_every_frame(self):
        self.assertTrue(rejects(load(), ISOLATED_GUARD, isolated=False))

    def test_the_proof_does_not_render_when_there_is_nothing_to_judge(self):
        _, _, requests = run(load(), isolated=False)
        self.assertEqual(requests, [], "a frame without isolated probes must stop before scene.render")

    def test_a_normal_corpus_passes_with_the_colour_rules_actually_evaluated(self):
        module = load()
        code, failures, requests = run(module)
        self.assertEqual((code, failures), (0, []))
        self.assertEqual(len(requests), FRAMES)
        for request in requests:
            self.assertEqual(len(request["probes"]), 2 + len(FAR), "centre + shifted + background points")
        colour = [c for c in module.checks if "shows the stored colour" in c["check"]]
        self.assertEqual(len(colour), FRAMES)
        self.assertTrue(all(c["ok"] for c in colour))


class ProbeCardinalityGuardTests(unittest.TestCase):
    def test_a_short_answer_is_rejected_instead_of_zipped_into_silence(self):
        self.assertTrue(rejects(load(), CARDINALITY_GUARD, reply=only_cardinality))
        self.assertTrue(rejects(load(), CARDINALITY_GUARD, reply=one_short))

    def test_an_answer_with_extra_probes_is_rejected_too(self):
        self.assertTrue(rejects(load(), CARDINALITY_GUARD, reply=one_extra))

    def test_a_missing_or_non_list_answer_is_rejected_not_a_crash(self):
        self.assertTrue(rejects(load(), CARDINALITY_GUARD, reply=lambda honest: None))
        self.assertTrue(rejects(load(), CARDINALITY_GUARD, reply=lambda honest: "none"))

    def test_a_wrong_colour_with_the_right_count_still_fails_the_colour_rule(self):
        code, failures, _ = run(load(), reply=lambda honest: [[0, 0, 0]] + honest[1:])
        self.assertEqual(code, 1)
        self.assertTrue(all("shows the stored colour" in name or "darkened" in name for name in failures), failures)


class GuardMutationTests(unittest.TestCase):
    """The checks above are discriminating: without the guard they stop rejecting (the proof would pass vacuously)."""

    def test_without_the_isolated_probe_guard_the_order_only_corpus_passes(self):
        mutant = load(remove_guard(SCRIPT.read_text(encoding="utf-8"), ISOLATED_GUARD))
        self.assertFalse(rejects(mutant, ISOLATED_GUARD, isolated=False))
        self.assertEqual(run(mutant, isolated=False)[0], 0, "vacuous pass: this is what the guard prevents")
        self.assertEqual(run(mutant)[0], 0, "the mutant still passes the honest corpus")

    def test_without_the_cardinality_guard_a_short_answer_passes(self):
        mutant = load(remove_guard(SCRIPT.read_text(encoding="utf-8"), CARDINALITY_GUARD))
        for reply in (only_cardinality, one_short):
            self.assertFalse(rejects(mutant, CARDINALITY_GUARD, reply=reply))
            self.assertEqual(run(mutant, reply=reply)[0], 0, f"vacuous pass with {reply.__name__}")
        self.assertEqual(run(mutant)[0], 0)

    def test_the_mutation_helper_refuses_to_mutate_nothing(self):
        with self.assertRaises(AssertionError):
            remove_guard("print('no guard here')", ISOLATED_GUARD)


if __name__ == "__main__":
    unittest.main()
