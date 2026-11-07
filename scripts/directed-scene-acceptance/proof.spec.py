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
import math
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
sys.dont_write_bytecode = True
import ledger  # noqa: E402
import link  # noqa: E402
import sensitivity  # noqa: E402

_CONTROLS = {}


def controls_of(domain):
    """The controls of a domain, run once per spec run (the splat controls take two minutes: ~230 judgments and ~115 ffmpeg encodes)."""
    if domain not in _CONTROLS:
        expected, bundles, corpus_dir = controls.load()
        runner = {"camera": proof._camera, "splat": proof._splat, "audio": proof._audio, "package": proof._package}[domain]
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            _CONTROLS[domain] = runner(expected, bundles, corpus_dir, pathlib.Path(tmp))
    return _CONTROLS[domain]


def tree_is_dirty():
    return bool(subprocess.run(["git", "status", "--porcelain"], cwd=HERE, capture_output=True, text=True).stdout.strip())


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
    args = argparse.Namespace(corpus=None, gpu_lock_held=True, bin_render="/nonexistent", gl="swiftshader", allow_stale_binary=False)
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

    def test_data_phases_pass_and_a_dirty_tree_is_never_exit_0(self):
        """A subset that passes exits 0 on a clean tree and 4 (informational, not an acceptance) on a dirty one; never anything else."""
        result = run("--phases", "tolerances_frozen,corpus,models")
        self.assertEqual(result.returncode, 4 if tree_is_dirty() else 0, result.stdout[-900:])
        if tree_is_dirty():
            self.assertIn("NOT AN ACCEPTANCE: the working tree is dirty", result.stdout)

    def test_an_empty_phase_selection_is_a_usage_error_never_a_green_run(self):
        for selection in ("", ",", " , "):
            result = run("--phases", selection)
            self.assertEqual(result.returncode, 2, (selection, result.stdout, result.stderr))
            self.assertIn("no phase selected", result.stderr)

    def test_work_that_is_a_file_is_a_typed_error_not_a_traceback(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            target = pathlib.Path(tmp) / "a-file"
            target.write_text("precious")
            result = run("--phases", "corpus", "--work", str(target))
            self.assertEqual(result.returncode, 2, result.stdout[-300:])
            self.assertIn("is not a directory", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertEqual(target.read_text(), "precious")


class Tolerances(unittest.TestCase):
    def test_every_negative_control_is_exercised_in_every_regime_and_on_every_frame(self):
        expected, bundles, corpus_dir = controls.load()
        for domain in ("camera", "splat", "audio", "package"):
            names = [r["control"] for r in controls_of(domain) if not r["should_pass"]]
            missing = proof.frozen_coverage(domain, verify.TOLERANCES[domain]["negativeControlsMustFail"], names, expected["splat"]["frames"])
            self.assertEqual(missing, [], f"{domain}: declared and not exercised")

    def test_a_declared_control_that_one_regime_or_one_frame_lacks_is_reported(self):
        names = ["negative: fov +5 deg", "negative: fov +5 deg (portrait 360x640@24)"]
        self.assertEqual(proof.frozen_coverage("camera", ["fov +5 deg"], names, []), ["fov +5 deg [video]"])
        splat = [f"negative: size x2 of each splat alone: s-red[0] @f{f}{tag}" for f in (0, 12) for tag in ("", " (video regime)")]
        self.assertEqual(proof.frozen_coverage("splat", ["size x2 of each splat alone"], splat, [0, 12]), [])
        self.assertEqual(proof.frozen_coverage("splat", ["size x2 of each splat alone"], splat, [0, 12, 23]),
                         ["size x2 of each splat alone [png]", "size x2 of each splat alone [video]"])
        self.assertEqual(proof.frozen_coverage("splat", ["size x1.5"], splat, [0, 12]), ["size x1.5 [png]", "size x1.5 [video]"],
                         "`size x1.5` is not `size x1.5 of each splat alone`")

    def test_the_tolerances_hash_in_the_corpus_is_the_file(self):
        expected, _, _ = controls.load()
        self.assertEqual(expected["tolerancesSha256"], proof.sha256_file(HERE / "tolerances.json"))

    def test_every_number_has_a_justification_and_a_reader_and_none_is_dead(self):
        tolerances = json.loads((HERE / "tolerances.json").read_text(encoding="utf-8"))
        unjustified, stale, malformed = ledger.justification_gaps(tolerances)
        self.assertEqual((unjustified, stale, malformed), ([], [], []))
        readers = "\n".join((HERE / name).read_text(encoding="utf-8") for name in ("verify.py", "proof.py", "controls.py", "corpus.py", "oracle.py", "sensitivity.py"))
        unread = [path for path in ledger.numeric_leaves(tolerances) if f'"{path.rsplit(".", 1)[-1]}"' not in readers]
        self.assertEqual(unread, [], "a number nothing reads is a dead parameter")

    def test_a_dead_parameter_or_an_unargued_number_is_caught(self):
        import copy
        tolerances = json.loads((HERE / "tolerances.json").read_text(encoding="utf-8"))
        added = copy.deepcopy(tolerances)
        added["camera"]["invented"] = 7
        self.assertEqual(ledger.justification_gaps(added)[0], ["camera.invented"])
        stale = copy.deepcopy(tolerances)
        stale["justifications"]["camera.gone"] = {"basis": "choice", "text": "x"}
        self.assertEqual(ledger.justification_gaps(stale)[1], ["camera.gone"])
        vague = copy.deepcopy(tolerances)
        vague["justifications"]["camera.maskCentroidPx"] = {"basis": "obvious", "text": ""}
        self.assertEqual(ledger.justification_gaps(vague)[2], ["camera.maskCentroidPx"])

    def test_no_pinned_file_or_doc_claims_a_pre_registration_or_reading_the_origin_blind(self):
        banned = ("frozen before", "fixed beforehand", "declared before results", "without reading the Rust", "pre-registered", "preregistered")
        files = [HERE / name for name in ("tolerances.json", "oracle.py", "corpus.py", "verify.py", "controls.py", "proof.py", "ledger.py", "link.py", "sensitivity.py")]
        files.append(HERE.parent.parent / "docs" / "features" / "scene-acceptance.md")
        for path in files:
            text = path.read_text(encoding="utf-8")
            for phrase in banned:
                for line in text.splitlines():
                    if phrase in line.lower() or phrase.lower() in line.lower():
                        # a sentence that denies the claim is allowed to name it
                        self.assertTrue(any(word in line.lower() for word in ("no ", "not ", "never", "without a", "does not", "nor ", "is not", "no-")),
                                        f"{path.name}: {line.strip()[:120]}")


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

    def test_every_audio_window_has_an_edge_so_an_offset_is_observable(self):
        """A13: the first full_av window was one 440 Hz tone; its envelope lag was arbitrary and 0.1 s is 44 whole cycles."""
        expected = controls.load()[0]
        for case_id, case in expected["audio"]["cases"].items():
            self.assertGreaterEqual(len(case["transitions"]), 1, case_id)
        full = expected["audio"]["cases"]["full_av"]
        self.assertEqual(sorted(t["kind"] for t in full["transitions"]), ["off", "on"])

    def test_a_constant_tone_cannot_show_a_shift_but_the_edged_window_does(self):
        """The arithmetic behind A13: a pure 440 Hz tone shifted 0.1 s is the same signal; the full_av window shifted 0.1 s is not."""
        rate = 48000
        tone = [math.sin(2 * math.pi * 440 * n / rate) for n in range(2 * rate)]
        shifted = tone[4800:] + tone[:4800]
        self.assertGreater(verify.correlation(tone, shifted), 0.999)
        expected, bundles, _ = controls.load()
        exact = controls.window(expected, bundles, "full_av")
        late = [0.0] * 4800 + exact[:-4800]  # the corpus window has edges: 0.1 s late moves them by 100 ms
        case = expected["audio"]["cases"]["full_av"]
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            path = pathlib.Path(tmp) / "late.mp4"
            verify.write_av_file(path, controls.correct_export(expected, late), case["frames"], case["fps"])
            collector = verify.Collector()
            verify.judge_audio(collector, path, case, bundles["full"], label="", video_seconds=case["durationSecondsFloat"])
        failed = collector.failed()
        self.assertTrue(any("edge at" in name for name in failed), failed)
        self.assertTrue(any("envelope lag" in name for name in failed), failed)

    def test_judge_audio_refuses_a_case_without_an_edge(self):
        expected, bundles, _ = controls.load()
        case = dict(expected["audio"]["cases"]["full_av"], transitions=[])
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            path = pathlib.Path(tmp) / "flat.mp4"
            verify.write_av_file(path, controls.correct_export(expected, controls.window(expected, bundles, "full_av")), case["frames"], case["fps"])
            collector = verify.Collector()
            verify.judge_audio(collector, path, case, bundles["full"], label="", video_seconds=case["durationSecondsFloat"])
            self.assertTrue(any("lag observable" in name for name in collector.failed()), collector.failed())


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
    """`phase_audio_short_source` against fakes: EVERY range of a source shorter than its window is refused (the origin decides
    `AUDIO_SHORT` from the window before it reads the range); the refusal is judged by code, rule, numbers, absence of output and leftovers."""

    MESSAGE = "The mix file holds 352800 frames (8.000 s at 44100 Hz) but the declared window ends at 13.000 s: the source must cover the whole window, not only the exported range"

    def run_phase(self, export, probe_ok=True):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            ctx = make_context(tmp + "/w")
            work = pathlib.Path(tmp) / "scratch"
            work.mkdir()
            probe = FakeChannel({"scene.export": lambda r: (True, {}) if probe_ok else (False, {"code": "unsupported", "message": "no", "details": {"domains": ["audio"]}})})
            channel = FakeChannel({"scene.export": export})
            opened = []

            def opener(name, bundle_key, **kwargs):
                opened.append(bundle_key)
                return (probe, work) if len(opened) == 1 else (channel, work)

            ctx.open = opener
            phase = proof.Phase("fake", "fake")
            try:
                proof.phase_audio_short_source(ctx, phase)
            finally:
                phase.opened, phase.channel = opened, channel
            return phase

    @classmethod
    def refuse(cls, request, code="audio_source_short", rule="window", message=None, write=False):
        if write:
            pathlib.Path(request["destination"]).write_bytes(b"x")
        return False, {"code": code, "message": message or cls.MESSAGE, "details": {"rule": rule}}

    def failed(self, phase):
        return [c["check"] for c in phase.checks if not c["ok"]]

    def test_a_correct_refusal_of_every_range_passes_and_the_seam_is_probed_on_a_valid_bundle(self):
        phase = self.run_phase(self.refuse)
        self.assertEqual(self.failed(phase), [])
        self.assertEqual(phase.opened, ["audio24", "audio_short"])
        self.assertEqual([c["from"] for c in phase.channel.calls], [0, 96], "both ranges are exported: the covered one and the one past the end")

    def test_an_exporter_that_accepts_the_range_the_file_covers_fails(self):
        """The origin refuses it too: a destination that exports file seconds 1-3 of an 8 s file under a 13 s window is wrong."""
        def lenient(request):
            return (True, {}) if request["from"] == 0 else self.refuse(request)

        phase = self.run_phase(lenient)
        self.assertTrue(any("covered by the file" in name and "refused" in name for name in self.failed(phase)), self.failed(phase))

    def test_an_exporter_that_accepts_a_short_source_fails(self):
        self.assertTrue(self.failed(self.run_phase(lambda r: (True, {}))))

    def test_a_refusal_with_the_wrong_code_or_rule_fails(self):
        self.assertTrue(any("typed" in n for n in self.failed(self.run_phase(lambda r: self.refuse(r, code="job_failed")))))
        self.assertTrue(any("details.rule" in n for n in self.failed(self.run_phase(lambda r: self.refuse(r, rule="range")))))

    def test_a_refusal_that_does_not_name_the_numbers_fails(self):
        phase = self.run_phase(lambda r: self.refuse(r, message="the source is too short"))
        self.assertTrue(any("names the 352800 frames" in n for n in self.failed(phase)), self.failed(phase))

    def test_a_refusal_that_leaves_a_file_fails(self):
        phase = self.run_phase(lambda r: self.refuse(r, write=True))
        self.assertTrue(any("no file" in n for n in self.failed(phase)), self.failed(phase))

    def test_the_seam_absent_blocks_instead_of_failing(self):
        with self.assertRaises(proof.Blocked):
            self.run_phase(self.refuse, probe_ok=False)

    def test_the_corpus_fixture_matches_what_the_phase_expects(self):
        case = controls.load()[0]["audio"]["shortSource"]
        self.assertEqual((case["fileFrames"], case["windowEndSeconds"], case["expectedCode"]), (352800, 13, "audio_source_short"))
        self.assertEqual([(r["from"], r["frames"]) for r in case["ranges"]], [(0, 48), (96, 96)])
        self.assertLessEqual(case["ranges"][0]["readsThroughSeconds"], case["fileSeconds"])
        self.assertGreater(case["ranges"][1]["readsThroughSeconds"], case["fileSeconds"])


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
            verify.write_av_file(path, [half, half], case["frames"], case["fps"])  # stereo: the origin always writes two channels
            at_zero, at_pan = verify.Collector(), verify.Collector()
            verify.judge_audio(at_zero, path, case, bundles["audio24"], video_seconds=case["durationSecondsFloat"], gain_db=0.0)
            verify.judge_audio(at_pan, path, case, bundles["audio24"], video_seconds=case["durationSecondsFloat"])  # default: the origin's channel law
            verify.judge_audio(verify.Collector(), path, case, bundles["audio24"], video_seconds=case["durationSecondsFloat"], gain_db=-3.0103)
        self.assertTrue(any("gain" in n for n in at_zero.failed()), "a 0 dB expectation rejects the -3 dB file")
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

    def test_a_correct_stereo_file_passes_the_verifier_and_a_mono_file_never_does(self):
        expected, bundles, corpus_dir = controls.load()
        case = expected["audio"]["cases"]["audio24_window_a"]
        exact = controls.window(expected, bundles, "audio24_window_a")
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            for name, channels, should_pass in (("law", controls.correct_export(expected, exact), True), ("mono", [exact], False), ("unlawful", [exact, exact], False)):
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

    def test_every_splat_control_is_exercised_on_every_frame_and_regime(self):
        expected, bundles, corpus_dir = controls.load()
        results = controls_of("splat")
        for frame in expected["splat"]["frames"]:
            isolated = len(controls.isolated_splats(expected["splat"]["cases"][str(frame)]))
            for tag in ("", controls.VIDEO_TAG):
                names = [r["control"] for r in results if r["control"].endswith(f"@f{frame}{tag}")]
                self.assertEqual(len(names), 1 + 8 + 2 * isolated, (frame, tag, len(names)))
        self.assertTrue(all(r["ok"] for r in results), [r["control"] for r in results if not r["ok"]])
        self.assertGreater(len(results), 200)

    def test_the_back_splat_controls_are_detected_on_every_frame_in_both_regimes(self):
        for result in controls_of("splat"):
            if "back splat missing" in result["control"] or "opaque disc" in result["control"]:
                self.assertTrue(result["detected"] and result["stance"] == "detect", result["control"])
                self.assertTrue(any("back splat is present" in name for name in result["named_failed"]), result["control"])

    def test_size_blind_spots_are_declared_two_sided_and_x2_is_never_blind(self):
        """Video x1.5 is below the radius rule's allowance on the small splats: stated as `blind`, never counted as detected; x2 is seen everywhere."""
        expected = controls.load()[0]
        results = [r for r in controls_of("splat") if "of each splat alone" in r["control"]]
        blind = [r for r in results if r["stance"] == "blind"]
        self.assertTrue(blind, "the video regime has blind spots at x1.5 and the controls must say so")
        self.assertTrue(all("x1.5" in r["control"] and controls.VIDEO_TAG in r["control"] and not r["detected"] and r["ok"] for r in blind))
        for r in results:
            if "x2" in r["control"]:
                self.assertEqual((r["stance"], r["detected"]), ("detect", True), r["control"])
        for r in results:
            self.assertEqual(r["size_state"] == "visible", r["stance"] == "detect", r["control"])
        # a blind declaration that stops being true fails the control instead of lingering
        entry = {"r50Px": 30.0}
        self.assertEqual(controls.size_error_state(entry, True, 1.5), "visible")
        self.assertEqual(controls.size_error_state({"r50Px": 8.0}, True, 1.5), "blind")
        self.assertEqual(controls.size_error_state({"r50Px": 8.0}, True, 2.0), "visible")
        self.assertEqual(controls.size_error_state({"r50Px": 13.3}, True, 1.5), "borderline")
        self.assertGreater(len(expected["splat"]["frames"]), 0)


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
        self.assertEqual(proof.exit_code_of([P, P], informational=["dirty"]), 4)
        self.assertEqual(proof.exit_code_of([P, B], informational=["dirty"]), 3)
        self.assertEqual(proof.exit_code_of([F], informational=["dirty"]), 1)
        self.assertEqual(proof.exit_code_of([]), 2, "nothing ran: never a green exit")

    def test_a_passing_run_is_acceptable_only_on_a_clean_tree_without_stale_binary_and_with_every_phase(self):
        ids = ["a", "b"]
        clean = {"dirty_files": 0}
        self.assertEqual(proof.acceptance_of(clean, False, [], [], ids, ids), {"acceptable": True, "reasons": [], "informational": []})
        dirty = proof.acceptance_of({"dirty_files": 3}, False, [], [], ids, ids)
        self.assertFalse(dirty["acceptable"])
        self.assertEqual(proof.exit_code_of([proof.harness.PASS], informational=dirty["informational"]), 4)
        stale = proof.acceptance_of(clean, True, [], [], ids, ids)
        self.assertEqual(proof.exit_code_of([proof.harness.PASS], informational=stale["informational"]), 4)
        used = proof.acceptance_of(clean, False, ["binary x used under --allow-stale-binary"], [], ids, ids)
        self.assertEqual(proof.exit_code_of([proof.harness.PASS], informational=used["informational"]), 4)
        subset = proof.acceptance_of(clean, False, [], [], ids, ["a"])
        self.assertFalse(subset["acceptable"])
        self.assertEqual(subset["informational"], [], "a subset may exit 0 for its own phases but is never an acceptance")

    def test_phase_selection(self):
        ids = ["a", "b"]
        self.assertEqual(proof.selected_phases("all", ids), (["a", "b"], None))
        self.assertEqual(proof.selected_phases("b", ids), (["b"], None))
        self.assertIn("no phase selected", proof.selected_phases("", ids)[1])
        self.assertIn("unknown", proof.selected_phases("a,z", ids)[1])

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
            self.assertEqual(result.returncode, 4 if tree_is_dirty() else 0, result.stdout[-400:])
            self.assertEqual((base / "sentinel.txt").read_text(), "precious")
            self.assertEqual([x.name for x in base.iterdir()], ["sentinel.txt"])

    def test_a_low_disk_refusal_is_a_prerequisite_error_not_a_failure(self):
        real = proof.free_gib
        proof.free_gib = lambda path="/tmp": 1.0
        try:
            with self.assertRaises(proof.WorkspaceError):
                proof.Workspace()
        finally:
            proof.free_gib = real

    def test_the_reserve_is_read_on_the_volume_of_work_not_on_tmp(self):
        seen = []
        real = proof.free_gib
        proof.free_gib = lambda path="/tmp": seen.append(str(path)) or 100.0
        try:
            with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
                ws = proof.Workspace(base=pathlib.Path(tmp) / "elsewhere")
                self.assertEqual(seen[0], str(pathlib.Path(tmp) / "elsewhere"))
                self.assertEqual(ws.free_gib(), 100.0)
                ws.close()
        finally:
            proof.free_gib = real

    def test_free_gib_of_a_directory_that_does_not_exist_yet_is_its_nearest_parent(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            self.assertGreater(proof.free_gib(pathlib.Path(tmp) / "a" / "b" / "c"), 0)

    def test_work_that_is_a_file_is_refused_and_the_file_is_untouched(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            target = pathlib.Path(tmp) / "f"
            target.write_text("precious")
            with self.assertRaises(proof.WorkspaceError):
                proof.Workspace(base=target)
            self.assertEqual(target.read_text(), "precious")
            self.assertEqual(sorted(x.name for x in pathlib.Path(tmp).iterdir()), ["f"])


class StaleBinary(unittest.TestCase):
    """A binary older than the sources, or whose link to this head does not hold, is not accepted silently."""

    def context(self, tmp, stale, allow, linked=True):
        binary = pathlib.Path(tmp) / "tessel-render"
        binary.write_bytes(b"x")
        ctx = make_context(tmp + "/w")
        ctx.args.bin_render = str(binary)
        ctx.args.allow_stale_binary = allow
        real_info, real_check = proof.harness.binary_info, proof.link.check
        proof.harness.binary_info = lambda path: {"path": str(path), "present": True, "older_than_sources": stale}
        proof.link.check = lambda path, key="render", dist=None: (linked, [] if linked else ["the binary on disk is not the one attested (sha256 differs)"], {"head": "h"})
        self.addCleanup(setattr, proof.harness, "binary_info", real_info)
        self.addCleanup(setattr, proof.link, "check", real_check)
        return ctx

    def test_a_stale_binary_blocks(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            with self.assertRaises(proof.Blocked):
                self.context(tmp, True, False).binary("render")

    def test_a_binary_whose_link_does_not_hold_blocks_even_when_it_is_newer_than_the_sources(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            with self.assertRaises(proof.Blocked) as raised:
                self.context(tmp, False, False, linked=False).binary("render")
            self.assertIn("not attested", raised.exception.reason)

    def test_a_stale_binary_allowed_makes_the_run_informational_never_exit_0(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            ctx = self.context(tmp, True, True)
            self.assertTrue(ctx.binary("render").is_file())
            ctx.binary("render")
            self.assertEqual(len(ctx.informational), 1, "noted once, not once per phase")
            acceptance = proof.acceptance_of({"dirty_files": 0}, True, ctx.informational, [], ["a"], ["a"])
            self.assertEqual(proof.exit_code_of([proof.harness.PASS], informational=acceptance["informational"]), 4)

    def test_an_unattested_binary_allowed_is_informational_too(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            ctx = self.context(tmp, False, True, linked=False)
            self.assertTrue(ctx.binary("render").is_file())
            self.assertTrue(any("attested" in note for note in ctx.informational))

    def test_a_fresh_attested_binary_is_accepted_and_informational_is_empty(self):
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            ctx = self.context(tmp, False, False)
            self.assertTrue(ctx.binary("render").is_file())
            self.assertEqual(ctx.informational, [])


class BinaryLink(unittest.TestCase):
    """`link.py check` against synthetic links built from the repository's own HEAD and a fake runtime bundle."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="s10-spec-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.binary = pathlib.Path(self.tmp) / "tessel-render"
        self.binary.write_bytes(b"binary")
        self.dist = pathlib.Path(self.tmp) / "dist"
        self.dist.mkdir()
        for name in link.DIST_FILES:
            (self.dist / name).write_text(name)
        self.document = {"schema": link.SCHEMA, "head": link.git("rev-parse", "HEAD"), "inputs": link.tree_ids("HEAD"),
                         "binaries": {"render": {"path": str(self.binary), "sha256": link.sha256_file(self.binary)}}, "runtime_dist": link.dist_hashes(self.dist)}

    def write(self, document=None):
        link.link_path(self.binary).write_text(json.dumps(document or self.document), encoding="utf-8")

    def check(self):
        return link.check(self.binary, "render", self.dist)

    def test_a_link_that_matches_head_the_binary_and_the_bundle_holds(self):
        self.write()
        ok, reasons, info = self.check()
        self.assertEqual((ok, reasons), (True, []))
        self.assertEqual(info["attested_head"], info["head"])

    def test_no_link_file_fails(self):
        ok, reasons, _ = self.check()
        self.assertFalse(ok)
        self.assertIn("does not exist", reasons[0])

    def test_a_swapped_binary_fails(self):
        self.write()
        self.binary.write_bytes(b"another build")
        self.assertTrue(any("sha256 differs" in r for r in self.check()[1]))

    def test_inputs_of_another_head_fail(self):
        other = dict(self.document, inputs=dict(self.document["inputs"], crates="0" * 40))
        self.write(other)
        self.assertTrue(any("crates" in r for r in self.check()[1]))

    def test_a_changed_runtime_bundle_fails(self):
        self.write()
        (self.dist / "runtime.js").write_text("changed")
        self.assertTrue(any("runtime/dist" in r for r in self.check()[1]))

    def test_an_empty_bundle_or_an_unknown_schema_fails(self):
        self.write(dict(self.document, schema="other", runtime_dist={}))
        reasons = self.check()[1]
        self.assertTrue(any("schema" in r for r in reasons) and any("runtime/dist" in r for r in reasons), reasons)

    def test_creating_a_link_needs_a_binary_and_a_bundle(self):
        ok, reasons = link.create({"render": pathlib.Path(self.tmp) / "missing"})
        self.assertFalse(ok)
        self.assertTrue(any("is not a file" in r for r in reasons))


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

    def failed(self, book, **kwargs):
        import ledger
        return [n for n, ok, _ in ledger.verify_ledger(book, **kwargs) if not ok]

    def test_the_ledger_is_anchored_and_pins_every_file_that_decides_a_verdict(self):
        import ledger
        book = ledger.load()
        self.assertEqual(book["baseline"]["commit"], ledger.ANCHOR)
        for name in ("proof.py", "controls.py", "verify.py", "oracle.py", "corpus.py", "corpus/expected.json", "tolerances.json", "link.py", "sensitivity.py",
                     "../directed-scene-proof/model.py", "../directed-scene-proof/harness.py",
                     "../directed-scene-export-proof/export_verify.py", "../directed-scene-export-proof/export_corpus.py"):
            self.assertIn(name, ledger.effective_pinned(book), f"{name} decides a verdict and must be pinned")

    def test_another_anchor_or_one_that_is_not_an_ancestor_fails(self):
        import copy
        import ledger
        moved = copy.deepcopy(ledger.load())
        moved["baseline"]["commit"] = "0" * 40
        self.assertTrue(any("anchor fixed" in n for n in self.failed(moved)))
        self.assertTrue(any("ancestor" in n for n in self.failed(ledger.load(), ancestor=lambda commit, descendant="HEAD": False)))

    def test_resetting_or_rewriting_the_book_is_caught_by_git_history(self):
        import copy
        import ledger
        book = ledger.load()
        history = lambda: [("c-new", book)]  # noqa: E731 - a committed version that had every round
        reset = copy.deepcopy(book)
        reset["rounds"] = []
        self.assertTrue(any("only grows" in n for n in self.failed(reset, versions=history)), "rounds = [] after rounds were committed")
        rewritten = copy.deepcopy(book)
        rewritten["rounds"][0]["reason"] = "nothing happened here"
        self.assertTrue(any("only grows" in n for n in self.failed(rewritten, versions=history)), "a published round rewritten")
        rebased = copy.deepcopy(book)
        rebased["baseline"]["hashes"]["oracle.py"] = "3" * 64
        self.assertTrue(any("only grows" in n for n in self.failed(rebased, versions=history)), "the baseline moved")
        self.assertFalse(any("only grows" in n for n in self.failed(book, versions=history)), "the untouched book is a prefix of itself")
        grown = copy.deepcopy(book)
        grown["rounds"].append({"id": "RX"})
        self.assertFalse(any("only grows" in n for n in self.failed(grown, versions=history)), "appending a round keeps the history a prefix")
        self.assertTrue(any("readable" in n for n in self.failed(book, versions=lambda: None)))
        self.assertTrue(any("only grows" in n for n in self.failed(book, versions=lambda: [("c", {"rounds": "unreadable"})])))

    def test_the_set_of_pinned_files_cannot_shrink_silently(self):
        import copy
        import ledger
        book = copy.deepcopy(ledger.load())
        for round_ in book["rounds"]:
            round_["pins"] = [p for p in round_.get("pins", []) if p != "proof.py"]
        self.assertTrue(any("exactly the pinned set" in n for n in self.failed(book)))

    def test_every_round_records_a_reason_a_finding_and_the_files_it_changed(self):
        import ledger
        book = ledger.load()
        self.assertGreaterEqual(len(book["rounds"]), 2)
        for round_ in book["rounds"]:
            self.assertTrue(round_["reason"].strip() and round_["amendments"], round_["id"])
            for amendment in round_["amendments"]:
                self.assertTrue(amendment["finding"].strip() and amendment["change"].strip() and amendment["files"], (round_["id"], amendment.get("id")))

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


class TypedControls(unittest.TestCase):
    """A control that cannot fail for the right reason proves nothing: exceptions and unrelated failures never count as detection."""

    def test_a_verifier_that_raises_fails_a_negative_and_a_positive_control(self):
        def boom(p):
            raise RuntimeError("kaput")

        negative = controls.run_control("n", False, boom, ("anything",))
        positive = controls.run_control("p", True, boom)
        for result in (negative, positive):
            self.assertFalse(result["ok"])
            self.assertIn("kaput", result["raised"])
        self.assertFalse(negative["detected"] and negative["ok"])

    def test_a_negative_control_needs_the_named_check_to_fail_not_any_check(self):
        def unrelated(p):
            p.check("something unrelated", False, "x")

        def named(p):
            p.check("centroid within 1.5 px", False, "x")

        self.assertFalse(controls.run_control("n", False, unrelated, ("centroid within",))["ok"])
        self.assertTrue(controls.run_control("n", False, named, ("centroid within",))["ok"])
        self.assertFalse(controls.run_control("n", False, lambda p: p.check("centroid within", True), ("centroid within",))["ok"], "the check passed: nothing was detected")
        with self.assertRaises(ValueError):
            controls.run_control("n", False, named)  # a negative control that names nothing

    def test_a_positive_control_fails_on_any_failed_check(self):
        self.assertFalse(controls.run_control("p", True, lambda p: p.check("x", False, "no"))["ok"])
        self.assertTrue(controls.run_control("p", True, lambda p: p.check("x", True))["ok"])

    def test_stances(self):
        named = lambda p: p.check("half-intensity radius", False, "x")  # noqa: E731
        silent = lambda p: p.check("half-intensity radius", True)  # noqa: E731
        self.assertTrue(controls.run_control("b", False, silent, ("half-intensity radius",), stance="blind")["ok"])
        self.assertFalse(controls.run_control("b", False, named, ("half-intensity radius",), stance="blind")["ok"], "a stale blind declaration fails")
        self.assertTrue(controls.run_control("e", False, named, ("half-intensity radius",), stance="either")["ok"])
        self.assertTrue(controls.run_control("e", False, silent, ("half-intensity radius",), stance="either")["ok"])
        self.assertFalse(controls.run_control("d", False, silent, ("half-intensity radius",))["ok"])
        with self.assertRaises(ValueError):
            controls.Control("p", True, stance="blind")

    def test_the_camera_controls_cover_both_regimes_portrait_and_the_absent_marker(self):
        results = controls_of("camera")
        names = [r["control"] for r in results]
        self.assertTrue(all(r["ok"] for r in results), [r["control"] for r in results if not r["ok"]])
        self.assertTrue(all(r["named_failed"] for r in results if not r["should_pass"]), "every negative fails a NAMED check")
        for tag in ("", controls.VIDEO_TAG):
            self.assertTrue(any("(portrait 360x640@24)" in n and (controls.VIDEO_TAG in n) == bool(tag) for n in names), tag)
            self.assertIn(f"positive: a frame with a marker outside the picture, nothing stray (portrait 360x640@0){tag}", names)
            self.assertIn(f"negative: marker colour where the model has none (portrait 360x640@0){tag}", names)
        for domain_name in verify.TOLERANCES["camera"]["negativeControlsMustFail"]:
            self.assertTrue(any(domain_name in n and controls.VIDEO_TAG in n for n in names), domain_name)

    def test_the_video_regime_is_a_real_codec_not_the_identity(self):
        expected, bundles, _ = controls.load()
        case = expected["camera"]["cases"]["640x360@8"]
        picture = controls.camera_picture(bundles["camera"], 8, 640, 360, expected["palette"])
        decoded = controls.video_roundtrip(picture, 640, 360)
        self.assertEqual(len(decoded), len(picture))
        self.assertNotEqual(decoded, picture, "4:2:0 libx264 must change at least an edge pixel")
        pixels = len(picture) // 3
        moved = sum(1 for i in range(pixels) if max(abs(picture[3 * i + c] - decoded[3 * i + c]) for c in range(3)) > 8)
        self.assertGreater(moved, 100, "4:2:0 and block coding move the pixels at the marker edges")
        self.assertLess(moved, pixels // 10, "a flat picture stays flat away from the edges")
        exact = verify.exact_masks_for(bundles["camera"], case)
        collector = verify.Collector()
        verify.judge_camera_frame(collector, decoded, 640, 360, exact, case, expected["palette"], video=True)
        self.assertEqual(collector.failed(), [])

    def test_the_audio_controls_include_the_mono_output_as_a_negative(self):
        results = controls_of("audio")
        self.assertTrue(all(r["ok"] for r in results), [r["control"] for r in results if not r["ok"]])
        mono = [r for r in results if "mono output" in r["control"]]
        self.assertEqual(len(mono), 1)
        self.assertFalse(mono[0]["should_pass"])
        self.assertTrue(mono[0]["detected"])

    def test_a_mono_output_is_refused_by_the_verifier_at_any_gain(self):
        expected, bundles, _ = controls.load()
        case = expected["audio"]["cases"]["audio24_window_a"]
        exact = controls.window(expected, bundles, "audio24_window_a")
        with tempfile.TemporaryDirectory(prefix="s10-spec-") as tmp:
            path = pathlib.Path(tmp) / "mono.mp4"
            verify.write_av_file(path, [[x * 0.7079 for x in exact]], case["frames"], case["fps"])
            collector = verify.Collector()
            verify.judge_audio(collector, path, case, bundles["audio24"], video_seconds=case["durationSecondsFloat"], gain_db=-3.0103)
            self.assertTrue(any("always writes 2" in n for n in collector.failed()), collector.failed())

    def test_the_package_controls_cover_name_hash_and_unknown_declarations(self):
        results = controls_of("package")
        self.assertTrue(all(r["ok"] for r in results), [r["control"] for r in results if not r["ok"]])
        names = " | ".join(r["control"] for r in results)
        for wanted in verify.TOLERANCES["package"]["negativeControlsMustFail"]:
            self.assertIn(wanted, names)


class DeclaredHashes(unittest.TestCase):
    """`judge_declared_hashes` against the corpus names in expected.json (not the names of the bundle under test)."""

    def setUp(self):
        self.expected, self.bundles, _ = controls.load()
        entry = self.expected["bundles"]["audio24"]
        self.wanted = {path: self.expected["assets"][path] for path in entry["declaredAssets"]}

    def judge(self, bundle):
        collector = verify.Collector()
        verify.judge_declared_hashes(collector, bundle, self.wanted)
        return collector.failed()

    def test_every_committed_bundle_declares_exactly_its_corpus_assets(self):
        for key, entry in self.expected["bundles"].items():
            collector = verify.Collector()
            verify.judge_declared_hashes(collector, self.bundles[key], {p: self.expected["assets"][p] for p in entry["declaredAssets"]})
            self.assertEqual(collector.failed(), [], key)
            self.assertTrue(entry["declaredAssets"], key)

    def test_a_wrong_missing_unknown_or_repeated_declaration_fails(self):
        import copy
        wrong = copy.deepcopy(self.bundles["audio24"])
        wrong["assets"][0]["contentHash"] = "0" * 64
        missing = copy.deepcopy(self.bundles["audio24"])
        missing["assets"] = missing["assets"][1:]
        unknown = copy.deepcopy(self.bundles["audio24"])
        unknown["assets"].append(dict(unknown["assets"][0], relativePath="assets/other.png"))
        repeated = copy.deepcopy(self.bundles["audio24"])
        repeated["assets"].append(dict(repeated["assets"][0]))
        for name, bundle in (("wrong", wrong), ("missing", missing), ("unknown", unknown), ("repeated", repeated)):
            self.assertTrue(self.judge(bundle), name)
        self.assertEqual(self.judge(self.bundles["audio24"]), [])

    def test_the_live_check_names_come_from_the_corpus_not_from_the_bundle_under_test(self):
        """A bundle that simply loses an asset must fail even though every name it still declares is internally consistent."""
        import copy
        short = copy.deepcopy(self.bundles["audio24"])
        short["assets"] = short["assets"][:1]
        self.assertTrue(self.judge(short))


class SensitivityAnalysis(unittest.TestCase):
    """The justification of the numbers is computed from the model, so it can be re-derived and does not drift from the controls."""

    def test_the_size_sensitivity_matches_the_controls_blind_spots(self):
        expected = controls.load()[0]
        summary = sensitivity.splat_size(expected)["summary"]
        self.assertEqual(summary["png"]["blindToX1_5"], 0)
        self.assertEqual((summary["png"]["blindToX2"], summary["video"]["blindToX2"]), (0, 0))
        self.assertGreater(summary["video"]["blindToX1_5"], 0)
        blind_in_controls = [r for r in controls_of("splat") if r["stance"] == "blind"]
        self.assertLessEqual(len(blind_in_controls), summary["video"]["blindToX1_5"], "controls call blind only what the arithmetic proves blind (with a guard band)")
        for regime, rows in sensitivity.splat_size(expected)["rows"].items():
            for row in rows:
                state = controls.size_error_state({"r50Px": row["r50Px"]}, regime == "video", 1.5)
                self.assertEqual(state == "blind", row["detectsFactorAbove"] > 1.5 + controls.GUARD_PX / row["r50Px"], row)

    def test_the_back_splat_separation_is_above_the_frozen_minimum_on_every_frame(self):
        expected = controls.load()[0]
        report = sensitivity.back_splat(expected)
        self.assertGreaterEqual(report["minSeparation"], verify.TOLERANCES["splat"]["backSplatSeparationMinDistance"])
        self.assertEqual(len(report["rows"]), len(expected["splat"]["frames"]))


if __name__ == "__main__":
    unittest.main(verbosity=1)
