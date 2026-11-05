#!/usr/bin/env python3
"""Self-tests of the export proof: the harness must be able to fail before its pass means anything.

    python3 scripts/directed-scene-export-proof/proof.spec.py

CPU and ffmpeg only (no Chrome, no GPU, no editor binary): the corpus is deterministic and pinned, the
adapter keeps the contract fields, a report that lies about the file is caught, and the data and
verifier-control phases of `proof.py` pass. The absence of `scene.export` is covered by running the
export phases against a binary that does not answer it (see docs/migration/scene-export-proof.md);
that needs a build and is not repeated here.
"""
import importlib.util
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from fractions import Fraction

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
PROOF = HERE / "proof.py"
sys.path.insert(0, str(HERE))

spec = importlib.util.spec_from_file_location("export_proof", PROOF)
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)


class Recorder:
    """Stands in for a Phase: records every check."""

    def __init__(self):
        self.results = []
        self.metrics = {}

    def check(self, name, ok, detail=""):
        self.results.append((name, bool(ok), detail))
        return bool(ok)

    def failed(self):
        return [name for name, ok, _ in self.results if not ok]


class Context:
    adapters = json.loads((HERE / "adapters.json").read_text())
    _bundle = {"assets": [{"assetId": "sky", "revisionId": "rev-1", "contentHash": "aa"}]}

    @classmethod
    def bundle(cls, key):
        return cls._bundle


CASE = {"bundle": "main", "frames": 24, "fps": "24/1", "durationSeconds": "1/1", "output": [1920, 1080]}
TRUTH = {"destination": "/x/out.mp4", "frames": 24, "fps": {"num": 24, "den": 1}, "duration_seconds": 1.0,
         "output": {"width": 1920, "height": 1080}, "audio": {"included": False}, "warnings": [],
         "verification": {"ok": True},
         "resources": [{"asset": "sky", "sha256": "aa", "revision": "rev-1", "bytes": 1}]}


def compare(result, destination="/x/out.mp4", case=CASE):
    recorder = Recorder()
    unreported = proof.compare_report(Context, recorder, "t", result, destination, case)
    return recorder, unreported


class Adapter(unittest.TestCase):
    def test_contract_fields_are_sent(self):
        export = Context.adapters["ops"]["export"]
        self.assertEqual(export["op"], "scene.export")
        for key in ("scene", "preset", "destination", "from", "frames"):
            self.assertIn(key, export["args"])

    def test_request_drops_null_values(self):
        request = proof.export_request(Context, "proof", "fullhd", "/x.mp4", 0, 8, True)
        self.assertEqual(request["op"], "scene.export")
        self.assertNotIn("audio", request)
        self.assertNotIn("replace", request)
        self.assertIs(proof.export_request(Context, "proof", "fullhd", "/x.mp4", 0, 8, True, audio=False)["audio"], False)

    def test_result_paths_walk_nested_keys(self):
        self.assertEqual(proof.dig({"output": {"width": 3}}, "output.width"), 3)
        self.assertIsNone(proof.dig({"output": {}}, "output.width"))
        self.assertEqual(proof.dig({"a": 1}, ""), {"a": 1}, "an empty path is the whole body")


class Blocking(unittest.TestCase):
    """A missing operation or integration is BLOCKED (exit 3), never a pass and never silently skipped."""

    def answer(self, code):
        request = proof.export_request(Context, "proof", "fullhd", "/x.mp4", 0, 8, True)
        return proof.send_export(Context, lambda *_: (False, {"code": code, "message": "m"}), request, 1)

    def test_unknown_operation_blocks(self):
        with self.assertRaises(proof.Blocked):
            self.answer("unknown_operation")

    def test_a_surface_without_a_renderer_blocks(self):
        with self.assertRaises(proof.Blocked):
            self.answer("backend_unavailable")

    def test_a_typed_refusal_is_data(self):
        self.assertEqual(self.answer("invalid_range")[0], False)


class Rates(unittest.TestCase):
    def test_every_report_shape_means_the_same_rational(self):
        ntsc = Fraction(30000, 1001)
        for shape in ("30000/1001", {"num": 30000, "den": 1001}, {"numerator": 30000, "denominator": 1001}):
            self.assertEqual(proof.rate_of(shape), ntsc)
        self.assertNotEqual(proof.rate_of(30), ntsc, "integer rounding of NTSC must not be accepted")
        self.assertIsNone(proof.rate_of({"num": 1}))

    def test_audio_claims(self):
        self.assertFalse(proof.audio_claimed({"included": False, "declared": ["audio"]}))
        self.assertTrue(proof.audio_claimed({"included": True}))
        self.assertTrue(proof.audio_claimed({"enabled": True}))


class ReportsAreChecked(unittest.TestCase):
    def test_a_truthful_report_passes_and_reports_nothing_missing(self):
        recorder, unreported = compare(TRUTH)
        self.assertEqual(recorder.failed(), [])
        self.assertEqual(unreported, [])

    def test_each_lie_fails(self):
        lies = {
            "frames": {**TRUTH, "frames": 23},
            "fps": {**TRUTH, "fps": {"num": 30, "den": 1}},
            "fps as a bare integer": {**TRUTH, "fps": 30},
            "duration": {**TRUTH, "duration_seconds": 1.5},
            "size": {**TRUTH, "output": {"width": 1280, "height": 720}},
            "destination": {**TRUTH, "destination": "/elsewhere.mp4"},
            "own verification": {**TRUTH, "verification": {"ok": False}},
            "a resource hash": {**TRUTH, "resources": [{"asset": "sky", "sha256": "bb", "revision": "rev-1"}]},
            "a missing resource": {**TRUTH, "resources": []},
        }
        for label, result in lies.items():
            with self.subTest(label):
                recorder, _ = compare(result)
                self.assertNotEqual(recorder.failed(), [], f"a report that lies about {label} passed")

    def test_absent_fields_are_listed_not_assumed(self):
        recorder, unreported = compare({"destination": "/x/out.mp4"})
        self.assertEqual(recorder.failed(), [])
        for name in ("frames", "fps", "duration_seconds", "output_width", "verification"):
            self.assertIn(name, unreported)


class Corpus(unittest.TestCase):
    def test_is_the_deterministic_output_of_its_generator(self):
        done = subprocess.run([sys.executable, str(HERE / "export_corpus.py"), "--check"], capture_output=True, text=True)
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)

    def test_a_tampered_expectation_fails_the_proof(self):
        with tempfile.TemporaryDirectory(prefix="te-r10-spec-") as folder:
            copy = pathlib.Path(folder) / "corpus"
            shutil.copytree(HERE / "corpus", copy)
            expected = json.loads((copy / "expected.json").read_text())
            expected["cases"]["main_1080_event"]["frames"] += 1
            (copy / "expected.json").write_text(json.dumps(expected))
            done = subprocess.run([sys.executable, str(PROOF), "--corpus", str(copy), "--phases", "corpus"],
                                  capture_output=True, text=True)
            self.assertEqual(done.returncode, 1, "a corpus that no longer matches its generator must fail\n" + done.stdout)

    def test_a_tampered_bundle_fails_the_proof(self):
        with tempfile.TemporaryDirectory(prefix="te-r10-spec-") as folder:
            copy = pathlib.Path(folder) / "corpus"
            shutil.copytree(HERE / "corpus", copy)
            path = copy / "bundle.main.json"
            path.write_text(path.read_text().replace("hero-sway", "hero-swat", 1))
            done = subprocess.run([sys.executable, str(PROOF), "--corpus", str(copy), "--phases", "corpus"],
                                  capture_output=True, text=True)
            self.assertEqual(done.returncode, 1, done.stdout)


class Phases(unittest.TestCase):
    """The harness's own controls: every defect it claims to detect is injected into a real file."""

    def run_phases(self, phases):
        done = subprocess.run([sys.executable, str(PROOF), "--phases", phases], capture_output=True, text=True)
        self.assertEqual(done.returncode, 0, done.stdout[-3000:] + done.stderr[-1000:])
        return done.stdout

    def test_data_and_verifier_controls_pass(self):
        out = self.run_phases("corpus,identity_coverage,verifier_controls_scene,verifier_controls_audio,verifier_controls_detail")
        self.assertIn("negative control: a duplicated frame", out)
        self.assertIn("negative control: a proxy", out)

    def test_phases_that_need_a_binary_are_blocked_or_fail_never_pass_without_one(self):
        done = subprocess.run([sys.executable, str(PROOF), "--phases", "export_main_1080_event",
                               "--bin-render", "/nonexistent/tessel-render", "--bin-headless", "/nonexistent/h"],
                              capture_output=True, text=True)
        self.assertEqual(done.returncode, 3, done.stdout + done.stderr)
        self.assertIn("BLOCKED", done.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
