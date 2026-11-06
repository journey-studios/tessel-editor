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
            phase = self.run_case(case_id, lambda ctx, cid: controls.correct_export(ctx.exp, controls.window(ctx.exp, ctx.bundles, cid)))
            failed = [c for c in phase.checks if not c["ok"]]
            self.assertEqual(failed, [], f"{case_id}: {failed[:2]}")

    def test_an_exporter_that_ignores_the_range_fails(self):
        def whole_mix(ctx, case_id):
            case = ctx.exp["audio"]["cases"][case_id]
            n = case["outputSamples"]
            table = controls.table_of(ctx.exp)
            return controls.correct_export(ctx.exp, [float(table.value(Fraction(i, 48000))) for i in range(n)])

        phase = self.run_case("audio24_window_a", whole_mix)
        self.assertTrue(any(not c["ok"] for c in phase.checks))

    def test_a_mono_source_exported_at_0_db_fails_and_at_the_origin_law_passes(self):
        """F3. The old fake wrote 0 dB and so froze the wrong law: the origin's graph (stereo layout) gives mono -3.01 dB per channel."""
        unlawful = self.run_case("audio24_window_a", lambda ctx, cid: [controls.window(ctx.exp, ctx.bundles, cid)] * 2)
        self.assertTrue(any("gain" in c["check"] and not c["ok"] for c in unlawful.checks), "0 dB must be rejected")
        lawful = self.run_case("audio24_window_a", lambda ctx, cid: controls.correct_export(ctx.exp, controls.window(ctx.exp, ctx.bundles, cid)))
        self.assertEqual([c for c in lawful.checks if not c["ok"]], [])
        self.assertTrue(any("-3.01 dB" in c["check"] for c in lawful.checks), "the judged gain is the law's")

    def test_an_exporter_that_shifts_audio_5_ms_fails(self):
        def shifted(ctx, case_id):
            exact = controls.window(ctx.exp, ctx.bundles, case_id)
            return controls.correct_export(ctx.exp, ([0.0] * 240 + exact)[:len(exact)])

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


class AudioWindowAmendment(unittest.TestCase):
    """Amendment A1: the origin reads the WHOLE WAV at `editStartSeconds + edit time`; the retired assumption A1 read it from
    `editStartSeconds` on (`mix[0] <-> editStart`). The corpus and the oracle now follow the origin; these tests keep it so."""

    def test_an_exporter_implementing_the_retired_assumption_fails(self):
        def retired_a1(ctx, case_id):
            case = ctx.exp["audio"]["cases"][case_id]
            bundle = ctx.bundles[case["bundle"]]
            fps = Fraction(bundle["fps"]["numerator"], bundle["fps"]["denominator"])
            start = Fraction(bundle["audio"]["editStartSeconds"]).limit_denominator(1000)
            table = controls.table_of(ctx.exp)
            origin = float(oracle.frames_to_seconds(case["from"], fps) - start)  # A1: mix[0] plays at editStart
            return controls.correct_export(ctx.exp, [table.value(origin + n / 48000) for n in range(case["outputSamples"])])

        phase = AudioWindowAmendment.run_case_with(self, "audio24_window_a", retired_a1)
        self.assertTrue(any(not c["ok"] for c in phase.checks), "the retired reading must not pass the amended oracle")

    def run_case_with(self, case_id, samples_of):
        return FakeAudioSeam().run_case(case_id, samples_of)

    def test_the_whole_wav_covers_every_declared_window_and_the_window_is_the_edit(self):
        expected, bundles, corpus_dir = controls.load()
        import wave
        with wave.open(str(corpus_dir / "audio/mix.wav")) as handle:
            seconds = Fraction(handle.getnframes(), handle.getframerate())
        for key in ("audio24", "audio_ntsc", "full"):
            audio = bundles[key]["audio"]
            end = Fraction(audio["editEndSecondsExclusive"]).limit_denominator(100000)
            fps = Fraction(bundles[key]["fps"]["numerator"], bundles[key]["fps"]["denominator"])
            self.assertLessEqual(end, seconds, key)
            self.assertLessEqual(abs(end - Fraction(audio["editStartSeconds"]).limit_denominator(1000) - bundles[key]["editDurationFrames"] / fps), Fraction(1, 1000), key)

    def test_the_short_wav_is_its_own_negative_fixture_and_the_old_mix_byte_for_byte(self):
        expected, bundles, corpus_dir = controls.load()
        self.assertEqual(proof.sha256_file(corpus_dir / "audio/mix-short.wav"), corpus.SHORT_MIX_SHA256)
        self.assertEqual(bundles["audio_short"]["audio"]["sourcePcmHashSha256"], corpus.SHORT_MIX_SHA256)
        self.assertNotEqual(bundles["audio24"]["audio"]["sourcePcmHashSha256"], corpus.SHORT_MIX_SHA256)
        history = json.loads((HERE / "history/A1-audio-window/before/provenance.json").read_text())
        self.assertEqual(history["files"]["audio/mix.wav"]["sha256"], corpus.SHORT_MIX_SHA256)

    def test_the_tone_table_is_in_file_time(self):
        table = controls.table_of(controls.load()[0])
        self.assertEqual(table.hz_at(Fraction(7, 4)), 440)  # file 1.75 s: audio24_window_a's first sample
        self.assertEqual(oracle.export_time_of(Fraction(7, 4), Fraction(24), 1, 18), 0)
        self.assertEqual(oracle.export_time_of(Fraction(3), Fraction(24), 1, 18), Fraction(5, 4))


class PhaseIsolation(unittest.TestCase):
    """A request that leaves a file (passed or failed) must not make a later request answer `destination_exists`."""

    @staticmethod
    def exporting_channel():
        def export(request):
            destination = pathlib.Path(request["destination"])
            if destination.exists():
                return False, {"code": "destination_exists", "message": f"`{destination}` already exists"}
            destination.write_bytes(b"x")
            return True, {}

        return FakeChannel({"scene.export": export})

    def test_probes_in_one_phase_and_in_the_next_never_collide(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            ctx = make_context(tmp + "/w")
            channel = self.exporting_channel()
            for scope in ("audio_export", "audio_export", "audio_package_export"):
                ctx.work.scope = scope
                proof.require_audio(ctx, channel, "au")
            destinations = {call["destination"] for call in channel.calls}
            self.assertEqual(len(destinations), 3)

    def test_a_probe_leaves_nothing_behind(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            ctx = make_context(tmp + "/w")
            proof.require_audio(ctx, self.exporting_channel(), "au")
            self.assertEqual([f for f in pathlib.Path(tmp).rglob("*.mp4")], [])

    def test_a_real_failure_stays_a_failure_and_is_not_blocked(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            ctx = make_context(tmp + "/w")
            exists = FakeChannel({"scene.export": lambda r: (False, {"code": "destination_exists", "message": "already there"})})
            with self.assertRaises(proof.Abort):
                proof.require_audio(ctx, exists, "au")

    def test_directories_of_different_phases_and_requests_are_distinct(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            ctx = make_context(tmp + "/w")
            ctx.work.scope = "audio_export"
            first = ctx.work.sub("same")
            second = ctx.work.fresh("same")
            ctx.work.scope = "audio_package_export"
            third = ctx.work.sub("same")
            self.assertEqual(len({first, second, third}), 3)


class FakeShortSource(unittest.TestCase):
    """`phase_audio_short_source` against fakes: the refusal is judged by its code, by the absence of output and of leftovers."""

    def run_phase(self, export):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            ctx = make_context(tmp + "/w")
            work = pathlib.Path(tmp) / "scratch"
            work.mkdir()
            channel = FakeChannel({"scene.export": export})
            ctx.open = lambda *a, **k: (channel, work)
            phase = proof.Phase("fake", "fake")
            proof.phase_audio_short_source(ctx, phase)
            return phase

    @staticmethod
    def source_aware(request, code="audio_source_short", write=False):
        reads_through = (request["from"] + request["frames"]) / 24 + 1
        if reads_through > 8:
            if write:
                pathlib.Path(request["destination"]).write_bytes(b"x")
            return False, {"code": code, "message": "The mix file holds 352800 frames but this range reads up to frame 396899"}
        return True, {}

    def test_a_correct_refusal_passes(self):
        phase = self.run_phase(self.source_aware)
        self.assertEqual([c for c in phase.checks if not c["ok"]], [])

    def test_an_exporter_that_accepts_a_short_source_fails(self):
        phase = self.run_phase(lambda r: (True, {}))
        self.assertTrue(any(not c["ok"] for c in phase.checks))

    def test_a_refusal_with_the_wrong_code_fails(self):
        phase = self.run_phase(lambda r: self.source_aware(r, code="job_failed"))
        self.assertTrue(any("typed" in c["check"] and not c["ok"] for c in phase.checks))

    def test_a_refusal_that_leaves_a_file_fails(self):
        phase = self.run_phase(lambda r: self.source_aware(r, write=True))
        self.assertTrue(any("no file" in c["check"] and not c["ok"] for c in phase.checks))


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


class ChannelLaw(unittest.TestCase):
    """F3: the mono -> stereo law is the origin's, re-derived here by running the origin's own `aformat` filter in FFmpeg."""

    # The mix chain of `packages/scene-engine/src/node/audio-graph.ts` (origin d6adac15), one mix input, no silences, no takes.
    ORIGIN_GRAPH = ("[0:a]atrim=start=0.000000:duration=1.000000,asetpts=PTS-STARTPTS,"
                    "aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo[mix];"
                    "[mix]anull[sum];[sum]apad=whole_dur=1.000000,atrim=duration=1.000000[out]")

    def rms_db(self, graph, channels):
        import array
        import math
        run = subprocess.run([verify.FFMPEG, "-v", "error", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100:duration=2", "-filter_complex", graph,
                              "-map", "[out]", "-f", "f32le", "-"], capture_output=True)
        self.assertEqual(run.returncode, 0, run.stderr.decode()[-200:])
        flat = array.array("f")
        flat.frombytes(run.stdout)
        chans = [flat[c::channels].tolist() for c in range(channels)]
        return [20 * math.log10(math.sqrt(sum(x * x for x in ch) / len(ch))) for ch in chans]

    def test_the_origin_graph_applies_the_oracle_law_to_a_mono_source(self):
        stereo = self.rms_db(self.ORIGIN_GRAPH, 2)
        mono = self.rms_db(self.ORIGIN_GRAPH.replace("channel_layouts=stereo", "channel_layouts=mono"), 1)[0]
        for channel in stereo:
            self.assertAlmostEqual(channel - mono, oracle.channel_gain_db(1, 2), delta=0.05)
        self.assertEqual(oracle.channel_gain_db(1, 1), 0.0)
        self.assertEqual(oracle.channel_gain_db(2, 2), 0.0)

    def test_an_unestablished_law_raises_instead_of_being_guessed(self):
        with self.assertRaises(ValueError):
            oracle.channel_gain_db(2, 1)

    def test_a_correct_stereo_file_passes_the_verifier_and_a_mono_file_at_0_db_passes_only_as_mono(self):
        expected, bundles, corpus_dir = controls.load()
        case = expected["audio"]["cases"]["audio24_window_a"]
        exact = controls.window(expected, bundles, "audio24_window_a")
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            for name, channels, should_pass in (("law", controls.correct_export(expected, exact), True), ("mono", [exact], True), ("unlawful", [exact, exact], False)):
                path = pathlib.Path(tmp) / f"{name}.mp4"
                verify.write_av_file(path, channels, case["frames"], case["fps"])
                collector = verify.Collector()
                verify.judge_audio(collector, path, case, bundles["audio24"], video_seconds=case["durationSecondsFloat"])
                self.assertEqual(not collector.failed(), should_pass, (name, collector.failed()[:2]))


class SplatOverlapComposition(unittest.TestCase):
    """F2: the back splat's centre is a blend, on every frame the corpus judges."""

    def test_the_old_front_colour_rule_would_reject_the_correct_picture_on_frames_36_and_47(self):
        expected, bundles, corpus_dir = controls.load()
        background = expected["palette"]["background"]
        old = verify.TOLERANCES["splat"]["overlapFrontColorDelta"]
        rejected = []
        for frame in expected["splat"]["frames"]:
            case = expected["splat"]["cases"][str(frame)]
            entries = {e["index"]: e for e in case["splats"] if e["node"] == "s-order"}
            blended = oracle.composite_at(case["splats"], background, round(entries[1]["x"]), round(entries[1]["y"]))
            if verify.distance(blended, entries[0]["color"]) > old:
                rejected.append(frame)
        self.assertEqual(rejected, [36, 47])

    def test_the_correct_blend_passes_and_swapped_order_fails_on_every_frame(self):
        expected, bundles, corpus_dir = controls.load()
        width, height = expected["splat"]["renderSize"]
        for frame in expected["splat"]["frames"]:
            case = expected["splat"]["cases"][str(frame)]
            right = verify.Collector()
            verify.judge_splat_frame(right, controls.splat_picture(case, expected["palette"], width, height), width, height, case, expected["palette"])
            self.assertEqual(right.failed(), [], frame)
            swapped = verify.Collector()
            order = lambda entries: [dict(e, depth=-e["depth"]) for e in entries]  # noqa: E731
            verify.judge_splat_frame(swapped, controls.splat_picture(case, expected["palette"], width, height, order), width, height, case, expected["palette"])
            self.assertTrue(any("s-order" in n for n in swapped.failed()), frame)

    def test_every_splat_control_is_exercised_on_every_frame(self):
        expected, bundles, corpus_dir = controls.load()
        results = controls.splat_controls(expected, bundles["splat"])
        for frame in expected["splat"]["frames"]:
            names = [r["control"] for r in results if r["control"].endswith(f"@f{frame}")]
            self.assertEqual(len(names), 7, (frame, names))
        self.assertTrue(all(r["ok"] for r in results), [r["control"] for r in results if not r["ok"]])


class Outcomes(unittest.TestCase):
    """F6: BLOCKED never replaces FAIL; the exit code follows."""

    def settle(self, steps):
        phase = proof.Phase("p", "fake")

        def run(ctx, phase):
            for step in steps:
                step(phase)

        proof.run_phase(None, phase, run)
        return phase

    @staticmethod
    def fail(phase):
        phase.check("a check that fails", False, "no")

    @staticmethod
    def block(phase):
        raise proof.Blocked("second bundle needs the seam", "S9", "BL016")

    def test_a_failure_followed_by_a_block_stays_a_failure(self):
        phase = self.settle([self.fail, self.block])
        self.assertEqual(phase.status, proof.harness.FAIL)
        self.assertIn("stays FAIL", phase.blocked["note"])
        self.assertEqual(proof.exit_code_of([phase.status]), 1)

    def test_a_block_alone_is_blocked_and_never_a_pass(self):
        phase = self.settle([self.block])
        self.assertEqual(phase.status, proof.harness.BLOCKED)
        self.assertEqual(proof.exit_code_of([phase.status]), 3)

    def test_exit_precedence(self):
        P, F, B = proof.harness.PASS, proof.harness.FAIL, proof.harness.BLOCKED
        self.assertEqual(proof.exit_code_of([P, P]), 0)
        self.assertEqual(proof.exit_code_of([P, B, F]), 1)
        self.assertEqual(proof.exit_code_of([P], within_budget=False), 1)
        self.assertEqual(proof.exit_code_of([P, B], stopped_early=True), 2)
        self.assertEqual(proof.exit_code_of([F], stopped_early=True), 1)

    def test_an_exception_is_a_failure_with_its_text(self):
        def boom(phase):
            raise RuntimeError("kaput")

        phase = self.settle([boom])
        self.assertEqual(phase.status, proof.harness.FAIL)
        self.assertIn("kaput", phase.error)


class WorkspaceSafety(unittest.TestCase):
    """F8: `--work <dir>` never removes or fills a directory the caller owns."""

    def test_close_removes_only_the_harness_directory(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            base = pathlib.Path(tmp) / "mine"
            (base / "keep-dir").mkdir(parents=True)
            (base / "keep-dir" / "sentinel.txt").write_text("precious")
            (base / "top.txt").write_text("precious")
            ws = proof.Workspace(base=base)
            self.assertEqual(ws.path.parent, base)
            self.assertTrue(ws.path.name.startswith("s10-acc-"))
            ws.sub("x").joinpath("f").write_text("scratch")
            ws.close()
            self.assertFalse(ws.path.exists())
            self.assertEqual((base / "keep-dir" / "sentinel.txt").read_text(), "precious")
            self.assertEqual((base / "top.txt").read_text(), "precious")
            self.assertEqual(sorted(x.name for x in base.iterdir()), ["keep-dir", "top.txt"])

    def test_keep_keeps_its_own_directory_and_the_rest_untouched(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            base = pathlib.Path(tmp)
            (base / "sentinel.txt").write_text("precious")
            ws = proof.Workspace(base=base, keep=True)
            ws.close()
            self.assertTrue(ws.path.exists())
            self.assertTrue((base / "sentinel.txt").exists())

    def test_the_command_line_with_work_preserves_a_populated_directory(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            base = pathlib.Path(tmp) / "owned"
            base.mkdir()
            (base / "sentinel.txt").write_text("precious")
            result = run("--phases", "corpus", "--work", str(base))
            self.assertEqual(result.returncode, 0, result.stdout[-400:])
            self.assertEqual((base / "sentinel.txt").read_text(), "precious")
            self.assertEqual([x.name for x in base.iterdir()], ["sentinel.txt"])

    def test_a_low_disk_refusal_is_a_prerequisite_error_not_a_failure(self):
        real = proof.free_gib
        proof.free_gib = lambda path="/tmp": 1.0
        try:
            with self.assertRaises(SystemExit) as raised:
                proof.Workspace()
        finally:
            proof.free_gib = real
        self.assertEqual(raised.exception.code, 2)


class StaleBinary(unittest.TestCase):
    """Low item: a binary older than the sources of this checkout is not accepted silently."""

    def context(self, tmp, stale, allow):
        binary = pathlib.Path(tmp) / "tessel-render"
        binary.write_bytes(b"x")
        ctx = make_context(tmp + "/w")
        ctx.args.bin_render = str(binary)
        ctx.args.allow_stale_binary = allow
        real = proof.harness.binary_info
        proof.harness.binary_info = lambda path: {"path": str(path), "present": True, "older_than_sources": stale}
        self.addCleanup(setattr, proof.harness, "binary_info", real)
        return ctx

    def test_a_stale_binary_blocks(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            with self.assertRaises(proof.Blocked):
                self.context(tmp, True, False).binary("render")

    def test_a_stale_binary_can_be_allowed_and_a_fresh_one_passes(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            self.assertTrue(self.context(tmp, True, True).binary("render").is_file())
            self.assertTrue(self.context(tmp, False, False).binary("render").is_file())


class Ledger(unittest.TestCase):
    """F5: edits to the oracle, verifier, controls, corpus or tolerances that no round records are caught."""

    def test_the_repository_ledger_holds(self):
        import ledger
        failed = [(name, detail) for name, ok, detail in ledger.verify_ledger() if not ok]
        self.assertEqual(failed, [])

    def test_an_unrecorded_edit_to_any_pinned_file_fails(self):
        import ledger
        for name in ledger.PINNED:
            now = ledger.current()
            now[name] = "0" * 64
            failed = [n for n, ok, _ in ledger.verify_ledger(now=now) if not ok]
            self.assertTrue(any("last recorded state" in n for n in failed), name)

    def test_a_round_that_does_not_chain_or_changes_an_unnamed_file_fails(self):
        import copy
        import ledger
        book = ledger.load()
        broken = copy.deepcopy(book)
        broken["rounds"][0]["before"]["oracle.py"] = "1" * 64
        self.assertTrue(any("starts from the previous" in n for n, ok, _ in ledger.verify_ledger(broken) if not ok))
        unnamed = copy.deepcopy(book)
        for amendment in unnamed["rounds"][0]["amendments"]:
            amendment["files"] = [f for f in amendment["files"] if f != "verify.py"]
        self.assertTrue(any("each named by an amendment" in n for n, ok, _ in ledger.verify_ledger(unnamed) if not ok))

    def test_a_baseline_that_git_does_not_hold_fails(self):
        import copy
        import ledger
        wrong = copy.deepcopy(ledger.load())
        wrong["baseline"]["hashes"]["verify.py"] = "2" * 64
        self.assertTrue(any("what Git holds" in n for n, ok, _ in ledger.verify_ledger(wrong) if not ok))

    def test_the_numbers_the_report_pins_are_in_tolerances_and_wired_to_the_verifier(self):
        tolerances = verify.TOLERANCES
        for domain, key in (("camera", "minJudgedAreaPx"), ("camera", "absentMarkerMaxPixels"), ("splat", "videoBackgroundExtraDelta"),
                            ("splat", "overlapBackCompositionDelta"), ("audio", "onsetSearchWindowSeconds"), ("audio", "onsetThresholdFraction")):
            self.assertIn(key, tolerances[domain])
        expected, bundles, corpus_dir = controls.load()
        case = expected["camera"]["cases"]["360x640@0"]  # mk_yellow is outside the frame
        self.assertEqual(case["markers"]["mk_yellow"]["area"], 0)
        exact = verify.exact_masks_for(bundles["camera_portrait"], case)
        picture = bytearray(controls.camera_picture(bundles["camera_portrait"], 0, 360, 640, expected["palette"]))
        collector = verify.Collector()
        verify.judge_camera_frame(collector, bytes(picture), 360, 640, exact, case, expected["palette"])
        self.assertEqual(collector.failed(), [])
        for x in range(40):  # a yellow blob the model does not have
            for y in range(40):
                base = ((600 + y) * 360 + 300 + x) * 3
                picture[base:base + 3] = bytes(expected["palette"]["markers"]["mk_yellow"])
        stray = verify.Collector()
        verify.judge_camera_frame(stray, bytes(picture), 360, 640, exact, case, expected["palette"])
        self.assertTrue(any("absent from the picture" in n for n in stray.failed()), stray.failed())


if __name__ == "__main__":
    unittest.main(verbosity=1)
