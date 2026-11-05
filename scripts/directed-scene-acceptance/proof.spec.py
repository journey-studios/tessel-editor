#!/usr/bin/env python3
"""Self-tests of the S10 harness: it must be able to fail, to block and to pass for the right reasons.

    python3 scripts/directed-scene-acceptance/proof.spec.py

No GPU and no Chrome. The destination is replaced by fakes that answer the way the awaited seams are specified
(`SEAMS` in proof.py), so the glue that no live phase can reach yet (S7 splat plan, S9 mixed export) is executed and
judged here. This does NOT prove that the real fronts behave: that is `proof.py` against `tessel-render`.
"""
import argparse
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
sys.path.insert(0, str(HERE))


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules.setdefault(name, module)
    spec.loader.exec_module(module)
    return module


proof = _load("s10_proof", HERE / "proof.py")
import controls  # noqa: E402
import corpus  # noqa: E402
import oracle  # noqa: E402
import verify  # noqa: E402

PROOF = [sys.executable, str(HERE / "proof.py")]


def run(*args):
    return subprocess.run([*PROOF, *args], capture_output=True, text=True)


class FakeChannel:
    """Answers `scene.plan`/`scene.export` from handlers; every call is recorded."""

    def __init__(self, handlers):
        self.handlers, self.calls = handlers, []

    def __call__(self, request, timeout=60):
        self.calls.append(request)
        return self.handlers[request["op"]](request)

    def close(self):
        pass


def make_context(work):
    expected, bundles, corpus_dir = controls.load()
    args = argparse.Namespace(corpus=None, gpu_lock_held=True, bin_render="/nonexistent", gl="swiftshader")
    ws = proof.Workspace(base=work)
    return proof.Context(args, ws, expected, bundles)


class ExitSemantics(unittest.TestCase):
    def test_unknown_phase_is_a_usage_error(self):
        self.assertEqual(run("--phases", "nope").returncode, 2)

    def test_missing_binary_blocks_with_exit_3_and_never_passes(self):
        result = run("--phases", "camera_plan", "--bin-render", "/nonexistent/tessel-render")
        self.assertEqual(result.returncode, 3, result.stdout[-400:])
        self.assertIn("BLOCKED", result.stdout)

    def test_a_tampered_corpus_fails_with_exit_1(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            copy = pathlib.Path(tmp) / "corpus"
            shutil.copytree(corpus.CORPUS_DIR, copy)
            target = copy / "assets/mk-green.png"
            data = bytearray(target.read_bytes())
            data[-20] ^= 0x01
            target.write_bytes(bytes(data))
            result = run("--phases", "corpus", "--corpus", str(copy))
            self.assertEqual(result.returncode, 1, result.stdout[-400:])

    def test_the_committed_corpus_is_what_the_generator_writes(self):
        self.assertEqual(corpus.check(corpus.CORPUS_DIR), [])

    def test_data_phases_pass(self):
        result = run("--phases", "tolerances_frozen,corpus,models")
        self.assertEqual(result.returncode, 0, result.stdout[-600:])


class Tolerances(unittest.TestCase):
    def test_every_negative_control_is_declared_before_results_and_exercised(self):
        expected, bundles, corpus_dir = controls.load()
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            for domain, runner in (("camera", proof._camera), ("splat", proof._splat)):
                names = " | ".join(r["control"] for r in runner(expected, bundles, corpus_dir, tmp) if not r["should_pass"])
                for wanted in verify.TOLERANCES[domain]["negativeControlsMustFail"]:
                    self.assertIn(wanted, names, f"{domain}: `{wanted}` is declared but not exercised")

    def test_the_tolerances_hash_in_the_corpus_is_the_file(self):
        expected, _, _ = controls.load()
        self.assertEqual(expected["tolerancesSha256"], proof.sha256_file(HERE / "tolerances.json"))


class FakeAudioSeam(unittest.TestCase):
    """The S9 glue (`audio_export_case`) with a fake exporter that writes a real MP4 holding the given samples."""

    def run_case(self, case_id, samples_of):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            ctx = make_context(tmp + "/w")
            case = ctx.exp["audio"]["cases"][case_id]
            bundle = ctx.bundles[case["bundle"]]
            rate = Fraction(bundle["fps"]["numerator"], bundle["fps"]["denominator"])

            def export(request):
                samples = samples_of(ctx, case_id)
                verify.write_av_file(request["destination"], samples, request["frames"], case["fps"], size=(640, 360))
                resources = [{"asset": a["assetId"], "sha256": a["contentHash"]} for a in bundle["assets"] if a["relativePath"].endswith((".png", ".spz"))]
                return True, {"frames": request["frames"], "fps": {"num": rate.numerator, "den": rate.denominator}, "output": {"width": 640, "height": 360},
                              "audio": {"included": True}, "resources": resources, "warnings": []}

            real = proof.judge_video_frames
            proof.judge_video_frames = lambda *a, **k: {}  # pictures are the camera phases' business
            phase = proof.Phase("fake", "fake")
            try:
                proof.audio_export_case(ctx, phase, case_id, case["bundle"], "au", ctx.work.sub("x"), FakeChannel({"scene.export": export}))
            finally:
                proof.judge_video_frames = real
            return phase

    def test_a_correct_mix_passes_at_24_and_at_ntsc(self):
        for case_id in ("audio24_window_a", "audio24_window_b", "audio_ntsc_window"):
            phase = self.run_case(case_id, lambda ctx, cid: [controls.window(ctx.exp, ctx.bundles, cid)])
            failed = [c for c in phase.checks if not c["ok"]]
            self.assertEqual(failed, [], f"{case_id}: {failed[:2]}")

    def test_an_exporter_that_ignores_the_range_fails(self):
        def whole_mix(ctx, case_id):
            case = ctx.exp["audio"]["cases"][case_id]
            n = case["outputSamples"]
            table = controls.table_of(ctx.exp)
            return [[float(table.value(Fraction(i, 48000))) for i in range(n)]]

        phase = self.run_case("audio24_window_a", whole_mix)
        self.assertTrue(any(not c["ok"] for c in phase.checks))

    def test_an_exporter_that_shifts_audio_5_ms_fails(self):
        def shifted(ctx, case_id):
            exact = controls.window(ctx.exp, ctx.bundles, case_id)
            return [([0.0] * 240 + exact)[:len(exact)]]

        phase = self.run_case("audio24_window_a", shifted)
        self.assertTrue(any(not c["ok"] for c in phase.checks))

    def test_a_report_that_lies_about_audio_fails(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            ctx = make_context(tmp + "/w")
            bundle = ctx.bundles["audio24"]
            path = pathlib.Path(tmp) / "silent.mp4"
            verify.write_video_only(path, 24, "24/1", size=(640, 360))
            probe = verify.ev.ffprobe(path)
            phase = proof.Phase("fake", "fake")
            proof.result_vs_file(phase, {"frames": 24, "fps": "24/1", "output": {"width": 640, "height": 360}, "audio": {"included": True}, "resources": []},
                                 probe, 24, Fraction(24), (640, 360), bundle, True)
            self.assertTrue(any(not c["ok"] for c in phase.checks if "audio" in c["check"]))


class FakeSplatSeam(unittest.TestCase):
    """The S7 glue (`phase_splat_plan`) with a fake plan built from the model, and with a wrong one."""

    def plan_of(self, ctx, frame, wrong=False):
        state = oracle.frame_state(ctx.bundles["splat"], frame)
        nodes = []
        for node_id, asset, *_ in corpus.SPLAT_NODES:
            matrix = list(state["worldMatrices"][node_id])
            if wrong and node_id == "s-pair":
                matrix = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]  # node rotation ignored
            nodes.append({"id": node_id, "kind": "splat", "assetId": asset, "worldMatrix": matrix})
        camera = state["camera"]
        xyz = lambda v: {"x": v[0], "y": v[1], "z": v[2]}  # noqa: E731
        return {"complete": True, "nodes": nodes, "camera": {"position": xyz(camera["position"]), "target": xyz(camera["target"]), "fovDeg": camera["fovDeg"]}}

    def run_plan(self, wrong):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            ctx = make_context(tmp + "/w")
            channel = FakeChannel({"scene.plan": lambda r: (True, {"plan": self.plan_of(ctx, r["frame"], wrong)})})
            ctx.open = lambda *a, **k: (channel, None)
            phase = proof.Phase("fake", "fake")
            proof.phase_splat_plan(ctx, phase)
            return phase

    def test_a_plan_equal_to_the_model_passes(self):
        phase = self.run_plan(False)
        self.assertEqual([c for c in phase.checks if not c["ok"]], [])
        self.assertGreater(len(phase.checks), 40)

    def test_a_plan_that_ignores_the_node_rotation_fails(self):
        phase = self.run_plan(True)
        failed = [c["check"] for c in phase.checks if not c["ok"]]
        self.assertTrue(any("s-pair" in name for name in failed), failed)

    def test_the_seam_signature_blocks_and_any_other_error_fails(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            ctx = make_context(tmp + "/w")
            refuse = FakeChannel({"scene.plan": lambda r: (False, {"code": "unsupported", "message": "no", "details": {"domains": [{"domain": "splat"}]}})})
            with self.assertRaises(proof.Blocked):
                proof.require_splat(ctx, refuse, "sp")
            broken = FakeChannel({"scene.plan": lambda r: (False, {"code": "internal", "message": "boom"})})
            with self.assertRaises(proof.Abort):
                proof.require_splat(ctx, broken, "sp")
            audio_refuse = FakeChannel({"scene.export": lambda r: (False, {"code": "unsupported", "message": "no", "details": {"domains": ["audio"]}})})
            with self.assertRaises(proof.Blocked):
                proof.require_audio(ctx, audio_refuse, "au")
            audio_broken = FakeChannel({"scene.export": lambda r: (False, {"code": "internal", "message": "boom"})})
            with self.assertRaises(proof.Abort):
                proof.require_audio(ctx, audio_broken, "au")


class Verifiers(unittest.TestCase):
    def test_an_edge_pixel_of_one_marker_is_not_read_as_another(self):
        palette = {"background": [20, 24, 40], "markers": {"a": [240, 210, 40], "b": [50, 200, 70]}}
        # half of the yellow marker over the background: nearest *colour* is green, nearest blend line is yellow
        edge = tuple(round((x + y) / 2) for x, y in zip(palette["background"], palette["markers"]["a"]))
        rgb = bytes([*edge])
        masks = verify.marker_masks(rgb, 1, 1, palette, 60)
        self.assertEqual((len(masks["a"]), len(masks["b"])), (1, 0))

    def test_gain_expectation_is_applied(self):
        expected, bundles, corpus_dir = controls.load()
        case = expected["audio"]["cases"]["audio24_window_a"]
        exact = controls.window(expected, bundles, "audio24_window_a")
        half = [x * 0.7079 for x in exact]  # -3.0 dB
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            path = pathlib.Path(tmp) / "half.mp4"
            verify.write_av_file(path, [half], case["frames"], case["fps"])
            at_zero, at_pan = verify.Collector(), verify.Collector()
            verify.judge_audio(at_zero, path, case, bundles["audio24"], video_seconds=case["durationSecondsFloat"])
            verify.judge_audio(at_pan, path, case, bundles["audio24"], video_seconds=case["durationSecondsFloat"], gain_db=-3.0103)
        self.assertTrue(any("gain" in n for n in at_zero.failed()))
        self.assertFalse(at_pan.failed(), at_pan.failed())


if __name__ == "__main__":
    unittest.main(verbosity=1)
