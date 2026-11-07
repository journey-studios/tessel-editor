"""Regression tests of the investigation drivers' failure paths (PR #14 review); no editor, GPU, Chrome or Rust build.

The real drivers (`s_sweep`, `s_scrub`, `s_ui`, `s_video`, `mainthread`, with `drv`/`scen` underneath) run end to end
against a fake editor, a fake clock and fake subprocesses. Each wrong path is forced and must leave an explicit
record instead of a result that looks measured:

  * a rejected `playback.start`, a playback that never plays, or one that draws nothing: the run is recorded as an
    error, the other runs and the partial file survive, the GPU sampler is stopped, the exit code is 1;
  * a scrub whose proof fails: an old JSON of the same label/drag/rep is never read as the new measurement;
  * a rejected `clip.add`, an import that never ends, media that is never ready, a disk-low abort: no run is
    recorded for what did not happen (and nothing divides by zero);
  * a missing repository root or main-thread header: a clear error, never a guess.

A last group (GuardMutationTests) deletes each guard from a copy of the drivers and proves the tests above then fail,
so they discriminate the guards and not only the happy path. Run: `python3 drivers.spec.py` (Python >= 3.9).
"""
import atexit
import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
DRIVERS = Path(os.environ.get("TESSEL_DRIVERS_DIR") or HERE).resolve()
REPO = HERE.parents[3]
WORK = Path(tempfile.mkdtemp(prefix="te-driver-spec-"))
atexit.register(shutil.rmtree, WORK, ignore_errors=True)
os.environ["TESSEL_PERF_INV_WORK"] = str(WORK)
os.environ.pop("TESSEL_REPO_ROOT", None)
sys.path.insert(0, str(DRIVERS))

import drv  # noqa: E402
import scen  # noqa: E402
import mainthread  # noqa: E402
import s_sweep, s_scrub, s_ui, s_video  # noqa: E402,E401

MODULES = (drv, scen, s_sweep, s_scrub, s_ui, s_video)
REJECT = {"ok": False, "version": 1, "error": {"code": "media_preparing", "message": "decoding"}}


class Clock:
    """`time.time`/`time.sleep` that never wait; `advance=False` keeps the real clock (file mtimes matter)."""

    def __init__(self, advance=True):
        self.now = time.time()
        self.advance = advance

    def time(self):
        return self.now if self.advance else REAL_TIME()

    def sleep(self, seconds):
        self.now += max(0, seconds)


REAL_TIME = time.time


class FakeSampler:
    live = 0
    started = 0

    def __init__(self, *a, **k):
        self.on = False

    def __enter__(self):
        FakeSampler.live += 1
        FakeSampler.started += 1
        self.on = True
        return self

    def __exit__(self, *a):
        if self.on:
            FakeSampler.live -= 1
            self.on = False

    def report(self):
        return {"n": 3, "mean": 20.0, "max": 30}


class FakeEditor:
    """A healthy instrumented editor unless `handlers` (op name -> function(editor, request) -> reply) says otherwise."""

    instances = []

    def __init__(self, label, handlers=None, **kwargs):
        self.label = label
        self.handlers = handlers or {}
        self.home = str(WORK / f"home-{label}")
        os.makedirs(self.home, exist_ok=True)
        self.perf_log = str(WORK / f"{label}.perflog.jsonl")
        open(self.perf_log, "w").close()
        self.socket_path = "/fake/ui.sock"
        self.pid = os.getpid()
        self.ops = []
        self.trace = []
        self.stopped = False
        self.playing = False
        self.start_frame = 0
        self.imported = 0
        FakeEditor.instances.append(self)

    # --- editor transport -------------------------------------------------
    def op(self, request, timeout=0):
        self.ops.append(request)
        handler = self.handlers.get(request["op"])
        if handler:
            reply = handler(self, request)
            if reply is not None:
                return reply
        return getattr(self, "do_" + request["op"].replace(".", "_"), lambda r: {"ok": True, "data": {}})(request)

    def get(self, target):
        return {"ok": True, "_ms": 1.0, "data": {}}

    def wait_socket(self, seconds=60):
        return 0.5

    def activate(self):
        pass

    def open_project(self, path, timeout=300):
        return {"ok": True}, 0.0

    def seek(self, frame):
        return {"ok": True}

    def settle(self, timeout=60):
        return True

    def stop(self):
        self.stopped = True

    def memory(self):
        return {"editor_rss_mb": 1.0}

    # --- default op behaviour ---------------------------------------------
    def do_playback_start(self, request):
        self.playing = True
        self.start_frame = request["from"]
        with open(self.perf_log, "a") as handle:
            for i in range(1, 6):
                row = {"k": "page", "t": 1000.0 + i, "msg": {"perf": "frame", "id": i, "outcome": "done", "render_ms": 14.0, "total_ms": 16.0, "queue_ms": 1.0,
                                                             "sum": {"settle_total": 14.0}, "cnt": {"lease_decode": 0}, "layers": [{"kind": "scene"}]}}
                handle.write(json.dumps(row) + "\n")
            handle.write(json.dumps({"k": "tick", "t": 1000.0, "dur": 0.05}) + "\n")
            handle.write(json.dumps({"k": "render", "t": 1000.0, "dur": 0.01}) + "\n")
        self.trace = [{"id": i, "shown_frame": self.start_frame + i, "done_ms": 1000.0 + 41.7 * i, "prepare_ms": 0.1, "serialize_ms": 0.1, "dispatch_ms": 0.1,
                       "transit_ms": 0.9, "runtime_total_ms": 14.0, "accept_ms": 0.1, "submit_to_done_ms": 15.0, "presented_after_done_ms": 0} for i in range(1, 6)]
        return {"ok": True, "data": {}}

    def do_playback_status(self, request):
        return {"ok": True, "data": {"state": "playing" if self.playing else "idle"}}

    def do_playback_stop(self, request):
        was = self.playing
        self.playing = False
        return {"ok": True, "data": {"report": {"frame": self.start_frame + 5, "elapsed_ms": 200, "lag_frames": 0, "skipped_frames": 0}} if was else {}}

    def do_render_status(self, request):
        return {"ok": True, "data": {"report": {"session": {"stats": {"requests": 0, "accepted": len(self.trace)}, "trace": {"frames_total": len(self.trace), "recent": self.trace}}}}}

    # --- video flow ---------------------------------------------------------
    def do_media_import(self, request):
        self.imported = len(request["paths"])
        return {"ok": True, "data": {}}

    def do_media_status(self, request):
        return {"ok": True, "data": {"assets": [{"id": f"seg{i}", "state": "ok"} for i in range(self.imported)]}}


class DriverCase(unittest.TestCase):
    label_n = 0

    def label(self):
        DriverCase.label_n += 1
        return f"{self.id().split('.')[-1][:40]}-{DriverCase.label_n}"

    def setUp(self):
        FakeSampler.live = FakeSampler.started = 0
        FakeEditor.instances.clear()

    def drive(self, main, argv, handlers=None, advance=True, **kwargs):
        """Run `main(argv)` of a driver with a fake editor/clock/sampler; returns (exit code, saved JSON, editor)."""
        label = argv[0]
        factory = lambda lbl, **kw: FakeEditor(lbl, handlers=handlers, **kw)
        clock = Clock(advance)
        with contextlib.ExitStack() as stack:
            for module in MODULES:
                for name, value in (("Editor", factory), ("GpuSampler", FakeSampler), ("wait_quiet_gpu", lambda *a, **k: True)):
                    if hasattr(module, name):
                        stack.enter_context(patch.object(module, name, value))
            stack.enter_context(patch.object(time, "sleep", clock.sleep))
            stack.enter_context(patch.object(time, "time", clock.time))
            if kwargs.get("subprocess_run"):
                stack.enter_context(patch.object(subprocess, "run", kwargs["subprocess_run"]))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            code = main(argv, **kwargs.get("main_kwargs", {}))
        saved = json.loads((WORK / "results" / f"{label}.json").read_text())
        return code, saved, (FakeEditor.instances[0] if FakeEditor.instances else None)


def ps_sample_run(cmd, *a, **k):
    out = "0:01.00\n" if cmd and cmd[0] == "ps" else ""
    return subprocess.CompletedProcess(cmd, 0, stdout=out, stderr="")


# ---- playback_run / s_sweep / s_ui -----------------------------------------------------------------------------------

class PlaybackFailureTests(DriverCase):
    def sweep(self, handlers, reps=2, **kw):
        return self.drive(s_sweep.main, [self.label(), "--runs", "40:60", "--reps", str(reps), "--warm", "5"], handlers, **kw)

    def test_a_healthy_sweep_is_complete_and_exits_zero(self):  # negative control for the guards below
        code, saved, ed = self.sweep(None)
        self.assertEqual(code, 0)
        self.assertEqual(saved["status"], "complete")
        self.assertEqual(len(saved["runs"]), 2)
        self.assertNotIn("errors", saved)
        self.assertTrue(ed.stopped)
        self.assertEqual(FakeSampler.live, 0)

    def test_a_rejected_playback_start_keeps_the_runs_already_measured_and_stops_the_sampler(self):
        flags = iter([True, False])  # rep 0 starts, rep 1 is rejected

        def start(editor, request):
            return None if next(flags) else dict(REJECT)

        code, saved, ed = self.sweep({"playback.start": start})
        self.assertEqual(code, 1)
        self.assertEqual(saved["status"], "partial")
        self.assertEqual([r["rep"] for r in saved["runs"]], [0], "the completed run survives")
        self.assertEqual(len(saved["errors"]), 1)
        self.assertEqual(saved["errors"][0]["rep"], 1)
        self.assertEqual(saved["errors"][0]["type"], "PlaybackError")
        self.assertEqual(saved["errors"][0]["reply"]["error"]["code"], "media_preparing")
        self.assertEqual(FakeSampler.live, 0, "the GPU sampler is stopped on the failure path")
        self.assertEqual(FakeSampler.started, 2)
        self.assertTrue(ed.stopped)

    def test_when_every_playback_start_is_rejected_nothing_is_reported_as_measured(self):
        code, saved, ed = self.sweep({"playback.start": lambda e, r: dict(REJECT)})
        self.assertEqual(code, 1)
        self.assertEqual(saved["status"], "failed")
        self.assertEqual(saved["runs"], [])
        self.assertEqual(len(saved["errors"]), 2)
        self.assertEqual(FakeSampler.live, 0)

    def test_a_playback_that_never_plays_is_an_error_and_is_stopped(self):
        handlers = {"playback.status": lambda e, r: {"ok": True, "data": {"state": "idle"}}}
        code, saved, ed = self.sweep(handlers, reps=1)
        self.assertEqual(code, 1)
        self.assertEqual(saved["status"], "failed")
        self.assertIn("never reached state=playing", saved["errors"][0]["error"])
        self.assertEqual(FakeSampler.live, 0)
        self.assertEqual(sum(1 for o in ed.ops if o["op"] == "playback.stop"), 1, "the started playback is stopped again")

    def test_a_playback_that_drew_nothing_is_not_a_measurement(self):
        def silent_start(editor, request):
            editor.playing = True
            editor.start_frame = request["from"]
            return {"ok": True, "data": {}}

        code, saved, _ = self.sweep({"playback.start": silent_start}, reps=1)
        self.assertEqual(code, 1)
        self.assertEqual(saved["runs"], [])
        self.assertIn("drew no frame rows", saved["errors"][0]["error"])

    def test_a_stop_without_a_report_is_an_error(self):
        code, saved, _ = self.sweep({"playback.stop": lambda e, r: {"ok": True, "data": {}}}, reps=1)
        self.assertEqual(code, 1)
        self.assertEqual(saved["runs"], [])
        self.assertIn("no report", saved["errors"][0]["error"])

    def test_a_connection_error_mid_run_is_recorded_and_the_partial_file_is_saved(self):
        calls = iter([True, False])

        def die(editor, request):
            if next(calls):
                return None
            raise ConnectionRefusedError("editor exited")

        code, saved, ed = self.sweep({"playback.start": die})
        self.assertEqual(code, 1)
        self.assertEqual(len(saved["runs"]), 1)
        self.assertEqual(saved["status"], "partial")
        self.assertEqual(saved["errors"][0]["type"], "ConnectionRefusedError")
        self.assertEqual(FakeSampler.live, 0)
        self.assertTrue(ed.stopped)

    def test_an_interrupt_still_saves_what_was_measured_and_is_re_raised(self):
        calls = iter([True, False])

        def interrupt(editor, request):
            if next(calls):
                return None
            raise KeyboardInterrupt

        label = self.label()
        with self.assertRaises(KeyboardInterrupt):
            self.drive(s_sweep.main, [label, "--runs", "40:60", "--reps", "2", "--warm", "5"], {"playback.start": interrupt})
        saved = json.loads((WORK / "results" / f"{label}.json").read_text())
        self.assertEqual(saved["status"], "failed")
        self.assertEqual(len(saved["runs"]), 1)
        self.assertEqual(saved["fatal"]["type"], "KeyboardInterrupt")
        self.assertEqual(FakeSampler.live, 0)

    def test_a_run_whose_frames_were_never_shown_is_not_recorded_as_measured(self):
        def blind(editor, request):
            editor.do_playback_start(request)
            for frame in editor.trace:
                frame["shown_frame"] = None
            return {"ok": True, "data": {}}

        code, saved, _ = self.sweep({"playback.start": blind}, reps=1)
        self.assertEqual(code, 1)
        self.assertEqual(saved["runs"], [])
        self.assertIn("no shown frame", saved["errors"][0]["error"])

    def test_an_empty_run_is_not_a_pass(self):
        code, saved, _ = self.sweep(None, reps=0)
        self.assertEqual(code, 1)
        self.assertEqual(saved["status"], "empty")

    def test_ui_phases_skip_a_rejected_phase_and_keep_the_others(self):
        flags = iter([True, False, True])  # play-sample ok, play-probe rejected, xfade-probe ok

        def start(editor, request):
            return None if next(flags) else dict(REJECT)

        code, saved, ed = self.drive(s_ui.main, [self.label(), "--warm", "5"], {"playback.start": start}, subprocess_run=ps_sample_run)
        self.assertEqual(code, 1)
        self.assertEqual(saved["status"], "partial")
        self.assertEqual(sorted(saved["phases"]), ["idle-probe", "play-sample", "xfade-probe"])
        self.assertEqual(saved["errors"][0]["phase"], "play-probe")
        self.assertEqual(FakeSampler.live, 0)
        self.assertEqual([t.name for t in __import__("threading").enumerate() if isinstance(t, s_ui.Probe)], [])


# ---- s_scrub ---------------------------------------------------------------------------------------------------------

GOOD = {"presentations_in_window": 99, "present_gap_ms": {"median": 20}, "lag_frames": {"median": 1}, "release_to_final_ms": 20, "first_present_ms": 5, "stats_delta": {"accepted": 3}}
STALE = {**GOOD, "presentations_in_window": 1, "release_to_final_ms": 9999}


def jf_of(argv_list):
    return argv_list[argv_list.index("--json") + 1]


class ScrubTests(DriverCase):
    def scrub(self, proof, stale=None, drags="take:20:100:60", reps=1, scrub_root=None):
        label = self.label()
        calls = []

        def fake_run(cmd, *a, **k):
            if "--json" in cmd:
                calls.append(cmd)
                return proof(cmd)
            return ps_sample_run(cmd)

        if stale is not None:
            jf = WORK / "results" / f"{label}.take.0.scrub.json"
            jf.parent.mkdir(parents=True, exist_ok=True)
            jf.write_text(json.dumps(stale))
            os.utime(jf, (REAL_TIME() - 3600, REAL_TIME() - 3600))
        with contextlib.ExitStack() as stack:
            if scrub_root is not None:
                stack.enter_context(patch.object(drv, "ROOT", str(scrub_root)))
            else:
                stack.enter_context(patch.object(s_scrub, "scrub_script", lambda: "/fake/seek-scrub-proof.py"))
            result = self.drive(s_scrub.main, [label, "--drags", drags, "--reps", str(reps), "--warm", "5"], advance=False, subprocess_run=fake_run)
        return result + (calls,)

    @staticmethod
    def write(cmd, data, code=0, text=None):
        Path(jf_of(cmd)).write_text(text if text is not None else json.dumps(data))
        return subprocess.CompletedProcess(cmd, code, stdout="", stderr="")

    def test_a_good_scrub_is_recorded_from_the_fresh_file(self):  # control: the fresh numbers replace the stale ones
        code, saved, ed, calls = self.scrub(lambda cmd: self.write(cmd, GOOD), stale=STALE)
        self.assertEqual(code, 0)
        self.assertEqual(saved["status"], "complete")
        self.assertEqual(saved["runs"][0]["presentations_in_window"], 99)
        self.assertEqual(saved["runs"][0]["release_to_final_ms"], 20)
        self.assertEqual(len(calls), 1)

    def test_a_failed_proof_does_not_leave_the_old_json_to_be_read_as_this_run(self):
        proof = lambda cmd: subprocess.CompletedProcess(cmd, 1, stdout="", stderr="socket error")
        code, saved, ed, calls = self.scrub(proof, stale=STALE)
        self.assertEqual(code, 1)
        self.assertEqual(saved["runs"], [], "the stale presentations/lag must not become a run")
        self.assertEqual(saved["status"], "failed")
        self.assertIn("exited 1", saved["errors"][0]["error"])
        self.assertIn("socket error", saved["errors"][0]["reply"]["stderr"])
        self.assertFalse(os.path.exists(jf_of(calls[0])), "the stale output was removed before the proof ran")

    def test_exit_zero_without_a_new_file_is_a_failure_even_when_an_old_one_existed(self):
        code, saved, _, _ = self.scrub(lambda cmd: subprocess.CompletedProcess(cmd, 0, stdout="", stderr=""), stale=STALE)
        self.assertEqual(code, 1)
        self.assertEqual(saved["runs"], [])
        self.assertIn("wrote no JSON", saved["errors"][0]["error"])

    def test_a_non_zero_exit_with_a_json_is_a_failure(self):  # the proof's own checks failed (e.g. not converged)
        code, saved, _, _ = self.scrub(lambda cmd: self.write(cmd, GOOD, code=1))
        self.assertEqual(code, 1)
        self.assertEqual(saved["runs"], [])
        self.assertIn("exited 1", saved["errors"][0]["error"])

    def test_a_json_older_than_the_run_is_refused(self):
        def proof(cmd):
            self.write(cmd, GOOD)
            os.utime(jf_of(cmd), (REAL_TIME() - 3600, REAL_TIME() - 3600))
            return subprocess.CompletedProcess(cmd, 0, stdout="", stderr="")

        code, saved, _, _ = self.scrub(proof)
        self.assertEqual(code, 1)
        self.assertEqual(saved["runs"], [])
        self.assertIn("older than this run", saved["errors"][0]["error"])

    def test_unparsable_or_incomplete_json_is_refused(self):
        code, saved, _, _ = self.scrub(lambda cmd: self.write(cmd, None, text="{not json"))
        self.assertEqual((code, saved["runs"]), (1, []))
        self.assertIn("not valid", saved["errors"][0]["error"])
        code, saved, _, _ = self.scrub(lambda cmd: self.write(cmd, {"presentations_in_window": 5}))
        self.assertEqual((code, saved["runs"]), (1, []))
        self.assertIn("lacks", saved["errors"][0]["error"])

    def test_one_failed_rep_does_not_discard_the_others_or_leak_into_them(self):
        results = iter([GOOD, None, {**GOOD, "presentations_in_window": 77}])

        def proof(cmd):
            data = next(results)
            if data is None:
                return subprocess.CompletedProcess(cmd, 1, stdout="", stderr="boom")
            return self.write(cmd, data)

        code, saved, _, _ = self.scrub(proof, reps=3)
        self.assertEqual(code, 1)
        self.assertEqual(saved["status"], "partial")
        self.assertEqual([r["presentations_in_window"] for r in saved["runs"]], [99, 77])
        self.assertEqual([e["rep"] for e in saved["errors"]], [1])

    def test_a_gpu_sampler_is_stopped_when_the_proof_cannot_run(self):
        def boom(cmd):
            raise FileNotFoundError("python vanished")

        code, saved, _, _ = self.scrub(boom)
        self.assertEqual(code, 1)
        self.assertEqual(FakeSampler.live, 0)
        self.assertEqual(saved["errors"][0]["type"], "FileNotFoundError")

    def test_a_wrong_repository_root_fails_before_any_editor_starts(self):
        with tempfile.TemporaryDirectory() as empty:
            code, saved, ed, calls = self.scrub(lambda cmd: self.write(cmd, GOOD), scrub_root=empty)
        self.assertEqual(code, 1)
        self.assertIsNone(ed, "no editor was created")
        self.assertEqual(calls, [])
        self.assertEqual(saved["status"], "failed")
        self.assertIn("not a tessel-editor checkout", saved["fatal"]["error"])
        self.assertIn("TESSEL_REPO_ROOT", saved["fatal"]["error"])

    def test_the_real_subprocess_path_never_reads_a_stale_file(self):
        """run_scrub with a real child process: one that fails and one that writes."""
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            jf = tmp / "x.scrub.json"
            failing = tmp / "fail.py"
            failing.write_text("import sys\nsys.stderr.write('proof crashed')\nsys.exit(3)\n")
            writing = tmp / "write.py"
            writing.write_text("import json, sys\nopen(sys.argv[sys.argv.index('--json') + 1], 'w').write(json.dumps(%r))\n" % GOOD)
            jf.write_text(json.dumps(STALE))
            with self.assertRaises(scen.DriverError) as raised:
                s_scrub.run_scrub(str(failing), "/s", "60", "100", str(jf))
            self.assertIn("exited 3", str(raised.exception))
            self.assertIn("proof crashed", raised.exception.reply["stderr"])
            self.assertFalse(jf.exists())
            jf.write_text(json.dumps(STALE))
            self.assertEqual(s_scrub.run_scrub(str(writing), "/s", "60", "100", str(jf))["presentations_in_window"], 99)


# ---- s_video ---------------------------------------------------------------------------------------------------------

class VideoTests(DriverCase):
    def video(self, handlers=None, counts="1,2", policies="original", free=lambda: 50.0, **kw):
        argv = [self.label(), "--counts", counts, "--policies", policies, "--assets", "2", "--frames", "4"]
        return self.drive(s_video.main, argv, handlers, main_kwargs={"free_gib": free}, subprocess_run=lambda cmd, *a, **k: subprocess.CompletedProcess(cmd, 0, stdout="1\t/x\n", stderr=""), **kw)

    def test_a_healthy_video_run_is_complete(self):  # control
        code, saved, ed = self.video()
        self.assertEqual(code, 0)
        self.assertEqual(saved["status"], "complete")
        self.assertEqual([(r["videos"], r["policy"], r["rep"], r["ready"]) for r in saved["runs"]],
                         [(1, "original", 0, True), (1, "original", 1, True), (2, "original", 0, True), (2, "original", 1, True)])

    def test_a_rejected_clip_add_stops_the_run_and_records_no_video_measurement(self):
        """The vid-b failure: clip.add was rejected, printed, ignored, and an empty timeline was measured as N clips."""
        reply = {"ok": False, "version": 1, "error": {"code": "invalid_value", "message": "`source_start` is seconds"}}
        code, saved, ed = self.video({"clip.add": lambda e, r: dict(reply)})
        self.assertEqual(code, 1)
        self.assertEqual(saved["status"], "failed")
        self.assertEqual(saved["runs"], [])
        self.assertEqual(saved["fatal"]["reply"]["error"]["code"], "invalid_value")
        self.assertFalse([o for o in ed.ops if o["op"] == "playback.start"], "nothing was played on an empty timeline")
        self.assertTrue(ed.stopped)

    def test_the_clip_add_request_carries_a_rational_source_start(self):
        _, _, ed = self.video()
        starts = [o["source_start"] for o in ed.ops if o["op"] == "clip.add"]
        self.assertTrue(starts and all(isinstance(s, dict) and set(s) == {"num", "den"} for s in starts), starts)

    def test_an_import_that_never_finishes_is_an_error_not_a_division_by_zero(self):
        code, saved, ed = self.video({"media.status": lambda e, r: {"ok": True, "data": {"assets": []}}})
        self.assertEqual(code, 1)
        self.assertEqual(saved["status"], "failed")
        self.assertIn("import timed out", saved["fatal"]["error"])
        self.assertEqual(saved["fatal"]["type"], "DriverError")
        self.assertFalse([o for o in ed.ops if o["op"] in ("track.add", "clip.add", "playback.start")])

    def test_assets_that_are_not_ok_are_refused(self):
        bad = lambda e, r: {"ok": True, "data": {"assets": [{"id": f"seg{i}", "state": "error" if i == 0 else "ok"} for i in range(e.imported)]}}
        code, saved, ed = self.video({"media.status": bad})
        self.assertEqual(code, 1)
        self.assertIn("not in state ok", saved["fatal"]["error"])
        self.assertEqual(saved["runs"], [])

    def test_media_that_never_becomes_ready_skips_the_measurement(self):
        code, saved, ed = self.video({"render.frame": lambda e, r: dict(REJECT)}, counts="1", policies="original,auto")
        self.assertEqual(code, 1)
        self.assertEqual(saved["status"], "failed")
        self.assertEqual(saved["runs"], [], "no entry labelled with a policy for media that was never ready")
        self.assertEqual([(e["policy"], e["kind"]) for e in saved["errors"]], [("original", "not_ready"), ("auto", "not_ready")])
        self.assertFalse([o for o in ed.ops if o["op"] == "playback.start"])
        self.assertTrue(all(e["ready_wait_s"] >= 900 for e in saved["errors"]))

    def test_a_policy_that_is_ready_is_still_measured_beside_one_that_is_not(self):
        def frame(editor, request):
            policy = [o["mode"] for o in editor.ops if o["op"] == "preview.policy"][-1]
            return dict(REJECT) if policy == "original" else None

        code, saved, _ = self.video({"render.frame": frame}, counts="1", policies="original,auto")
        self.assertEqual(code, 1)
        self.assertEqual(saved["status"], "partial")
        self.assertEqual({r["policy"] for r in saved["runs"]}, {"auto"})
        self.assertEqual([e["policy"] for e in saved["errors"]], ["original"])

    def test_a_render_frame_failure_that_is_not_preparing_stops_at_once(self):
        hard = {"ok": False, "error": {"code": "asset_missing", "message": "gone"}}
        code, saved, ed = self.video({"render.frame": lambda e, r: dict(hard)}, counts="1")
        self.assertEqual(code, 1)
        self.assertEqual(saved["runs"], [])
        self.assertEqual(saved["errors"][0]["reply"]["error"]["code"], "asset_missing")
        self.assertLess(len([o for o in ed.ops if o["op"] == "render.frame"]), 5, "no 900 s of polling a permanent error")

    def test_a_disk_low_abort_is_marked_and_keeps_the_counts_already_measured(self):
        levels = iter([50.0, 0.3])
        code, saved, _ = self.video(free=lambda: next(levels))
        self.assertEqual(code, 1)
        self.assertEqual(saved["status"], "partial")
        self.assertEqual({r["videos"] for r in saved["runs"]}, {1})
        self.assertEqual(saved["errors"][0]["kind"], "disk_low")
        self.assertEqual(saved["errors"][0]["videos"], 2)

    def test_one_rejected_playback_keeps_the_other_reps(self):
        flags = iter([True, False, True, True])
        code, saved, _ = self.video({"playback.start": lambda e, r: None if next(flags) else dict(REJECT)}, counts="1")
        self.assertEqual(code, 1)
        self.assertEqual(saved["status"], "partial")
        self.assertEqual([r["rep"] for r in saved["runs"]], [0])
        self.assertEqual(saved["errors"][0]["rep"], 1)
        self.assertEqual(FakeSampler.live, 0)


# ---- the real scripts as the shell runs them ------------------------------------------------------------------------

class EntryPointTests(unittest.TestCase):
    def test_every_driver_script_exits_non_zero_and_saves_a_failed_run_when_the_editor_cannot_start(self):
        for script in ("s_sweep.py", "s_scrub.py", "s_ui.py", "s_video.py"):
            with self.subTest(script=script), tempfile.TemporaryDirectory() as work:
                env = {**os.environ, "TESSEL_PERF_INV_WORK": work}
                env.pop("TESSEL_REPO_ROOT", None)
                run = subprocess.run([sys.executable, str(DRIVERS / script), "entry"], capture_output=True, text=True, env=env, timeout=60)
                self.assertEqual(run.returncode, 1, run.stderr[-500:])
                saved = json.loads((Path(work) / "results" / "entry.json").read_text())
                self.assertEqual(saved["status"], "failed")
                self.assertEqual(saved["fatal"]["type"], "FileNotFoundError")
                self.assertNotIn("Traceback", run.stderr)


# ---- drv: the repository root ----------------------------------------------------------------------------------------

class RootTests(unittest.TestCase):
    def root_in_fresh_interpreter(self, env_extra=None, cwd=None):
        env = {k: v for k, v in os.environ.items() if k != "TESSEL_REPO_ROOT"}
        env.update(env_extra or {})
        code = f"import sys; sys.path.insert(0, {str(DRIVERS)!r}); import drv; print(drv.ROOT)"
        return subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, env=env, cwd=cwd).stdout.strip()

    def test_root_is_derived_from_the_driver_file_whatever_the_cwd(self):
        with tempfile.TemporaryDirectory() as elsewhere:
            self.assertEqual(self.root_in_fresh_interpreter(cwd=elsewhere), str(DRIVERS.parents[3]))

    @unittest.skipUnless(DRIVERS == HERE, "only the committed drivers live inside the repository")
    def test_root_is_the_repository_with_the_scrub_proof(self):
        self.assertEqual(Path(drv.ROOT), REPO)
        self.assertEqual(Path(drv.scrub_script()), REPO / "scripts" / "seek-scrub-proof.py")
        self.assertTrue(Path(drv.scrub_script()).is_file())

    def test_an_explicit_root_wins(self):
        self.assertEqual(self.root_in_fresh_interpreter({"TESSEL_REPO_ROOT": "/some/other/checkout"}), "/some/other/checkout")

    def test_no_developer_path_is_committed_in_the_drivers(self):
        for path in DRIVERS.glob("*.py"):
            if path.name == "drivers.spec.py":
                continue
            text = path.read_text()
            self.assertNotIn("/Users/", text, path.name)
            self.assertNotIn("t3-129bf52a", text, path.name)

    def test_the_work_directory_is_configurable(self):
        self.assertEqual(drv.WORK, str(WORK))
        self.assertTrue(drv.RELEASE.startswith(str(WORK)))


# ---- mainthread ------------------------------------------------------------------------------------------------------

SAMPLE = """Analysis of sampling tessel-editor (pid 1) every 1 millisecond
Call graph:
    800 Thread_9   DispatchQueue_1: com.apple.root.user-interactive-qos
    + 800 other_thread_fn  (in libsystem_pthread.dylib) + 1  [0x9]

    1000 Thread_111   DispatchQueue_1: com.apple.main-thread  (serial)
    + 1000 start  (in dyld) + 1  [0x1]
    +   1000 main  (in tessel-editor) + 10  [0x2]
    +     600 mach_msg2_trap  (in libsystem_kernel.dylib) + 8  [0x3]
    +     400 render  (in tessel-editor) + 5  [0x4]
    +     ! 250 taffy_layout  (in tessel-editor) + 2  [0x5]
    +     ! 100 __psynch_cvwait  (in libsystem_kernel.dylib) + 3  [0x6]

    700 Thread_222   DispatchQueue_2: com.apple.root.user-initiated-qos
    + 700 worker_fn  (in libsystem_pthread.dylib) + 1  [0x7]
"""
NO_MAIN = "Call graph:\n  500 Thread_1 DispatchQueue_1: com.apple.root\n  + 500 x  (in y)\n"
HEADER_ONLY = "Call graph:\n    1000 Thread_111   DispatchQueue_1: com.apple.main-thread  (serial)\n\n    700 Thread_2 Q\n    + 700 w  (in z)\n"


class MainThreadTests(unittest.TestCase):
    def run_main(self, text):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "sample.txt"
            path.write_text(text)
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                code = mainthread.main([str(path)])
        return code, out.getvalue(), err.getvalue()

    def test_the_main_thread_block_is_summarised_and_not_the_neighbours(self):  # control
        code, out, _ = self.run_main(SAMPLE)
        self.assertEqual(code, 0)
        lines = out.splitlines()
        self.assertEqual(lines[0], "total 1000")
        self.assertEqual(lines[1], "idle 700 70.0 % busy 30.0")
        self.assertEqual(lines[2:], ["600 mach_msg2_trap libsystem_kernel.dylib", "250 taffy_layout tessel-editor", "100 __psynch_cvwait libsystem_kernel.dylib", "50 render tessel-editor"])
        self.assertNotIn("other_thread_fn", out)
        self.assertNotIn("worker_fn", out)

    def test_a_file_without_a_main_thread_header_fails_instead_of_reporting_another_thread(self):
        code, out, err = self.run_main(NO_MAIN)
        self.assertEqual(code, 2)
        self.assertEqual(out, "")
        self.assertIn("no main-thread header", err)

    def test_a_header_without_nodes_fails_clearly(self):
        code, out, err = self.run_main(HEADER_ONLY)
        self.assertEqual(code, 2)
        self.assertEqual(out, "")
        self.assertIn("no call-graph nodes", err)

    def test_an_unreadable_file_and_a_bad_usage_are_clear_errors(self):
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            self.assertEqual(mainthread.main(["/does/not/exist"]), 2)
            self.assertEqual(mainthread.main([]), 2)
        self.assertIn("/does/not/exist", err.getvalue())
        self.assertIn("usage", err.getvalue())

    def test_the_cli_exits_non_zero_without_a_traceback(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "s.txt"
            path.write_text(NO_MAIN)
            run = subprocess.run([sys.executable, str(DRIVERS / "mainthread.py"), str(path)], capture_output=True, text=True)
        self.assertEqual(run.returncode, 2)
        self.assertEqual(run.stdout, "")
        self.assertNotIn("Traceback", run.stderr)


# ---- finalize / run_session ------------------------------------------------------------------------------------------

class FinalizeTests(unittest.TestCase):
    def status(self, out, fatal=None):
        label = f"finalize-{id(out)}"
        with contextlib.redirect_stdout(io.StringIO()):
            code = scen.finalize(label, out, fatal)
        return code, json.loads((WORK / "results" / f"{label}.json").read_text())["status"]

    def test_only_a_complete_non_empty_run_exits_zero(self):
        self.assertEqual(self.status({"runs": [1]}), (0, "complete"))
        self.assertEqual(self.status({"runs": []}), (1, "empty"))
        self.assertEqual(self.status({"phases": {}}), (1, "empty"))
        self.assertEqual(self.status({"runs": [1], "errors": [{}]}), (1, "partial"))
        self.assertEqual(self.status({"runs": [], "errors": [{}]}), (1, "failed"))
        self.assertEqual(self.status({"runs": [1]}, RuntimeError("x")), (1, "failed"))


# ---- the guards are discriminated: delete each one from a copy and the tests above must fail --------------------------

REJ = "PlaybackFailureTests.test_a_rejected_playback_start_keeps_the_runs_already_measured_and_stops_the_sampler"
MUTANTS = [
    ("scen.py", '        if not reply.get("ok"):\n            raise PlaybackError(f"playback.start rejected in run {label}", reply)\n', "        pass\n", [REJ]),
    ("scen.py", "        gs.__exit__()\n    report = ", "        pass\n    report = ", [REJ]),
    ("scen.py", '        if started:\n            try:\n                ed.op({"op": "playback.stop"})', '        if False:\n            try:\n                ed.op({"op": "playback.stop"})', ["PlaybackFailureTests.test_a_playback_that_never_plays_is_an_error_and_is_stopped"]),
    ("scen.py", '        if playing_at is None:\n            raise PlaybackError(', '        if False:\n            raise PlaybackError(', ["PlaybackFailureTests.test_a_playback_that_never_plays_is_an_error_and_is_stopped"]),
    ("scen.py", '    if not any(r.get("k") == "page"', '    if False and not any(r.get("k") == "page"', ["PlaybackFailureTests.test_a_playback_that_drew_nothing_is_not_a_measurement"]),
    ("scen.py", '    if not isinstance(report, dict) or report.get("frame") is None:', '    if False:', ["PlaybackFailureTests.test_a_stop_without_a_report_is_an_error"]),
    ("scen.py", '        return fn()\n    except Exception as exc:', '        return fn()\n    except ZeroDivisionError as exc:', [REJ]),
    ("scen.py", '    code = finalize(label, out, fatal)', '    code = 0 if fatal is None else finalize(label, out, fatal)', ["PlaybackFailureTests.test_a_healthy_sweep_is_complete_and_exits_zero"]),
    ("scen.py", '    elif not measured:\n        out["status"] = "empty"', '    elif not measured:\n        out["status"] = "complete"', ["FinalizeTests.test_only_a_complete_non_empty_run_exits_zero", "PlaybackFailureTests.test_an_empty_run_is_not_a_pass"]),
    ("s_sweep.py", 'guarded(out, {"spec": spec, "rep": rep}, one)', 'one()', [REJ]),
    ("s_sweep.py", "                    if not reg:", "                    if False:", ["PlaybackFailureTests.test_a_run_whose_frames_were_never_shown_is_not_recorded_as_measured"]),
    ("s_scrub.py", "    if os.path.lexists(jf):\n        os.remove(jf)\n", "", ["ScrubTests.test_a_failed_proof_does_not_leave_the_old_json_to_be_read_as_this_run", "ScrubTests.test_exit_zero_without_a_new_file_is_a_failure_even_when_an_old_one_existed", "ScrubTests.test_the_real_subprocess_path_never_reads_a_stale_file"]),
    ("s_scrub.py", "    if p.returncode != 0:", "    if False:", ["ScrubTests.test_a_non_zero_exit_with_a_json_is_a_failure", "ScrubTests.test_the_real_subprocess_path_never_reads_a_stale_file"]),
    ("s_scrub.py", "    if not os.path.isfile(jf):", "    if False:", ["ScrubTests.test_exit_zero_without_a_new_file_is_a_failure_even_when_an_old_one_existed"]),
    ("s_scrub.py", "    if os.path.getmtime(jf) < started - 1:", "    if False:", ["ScrubTests.test_a_json_older_than_the_run_is_refused"]),
    ("s_scrub.py", "    if missing:", "    if False:", ["ScrubTests.test_unparsable_or_incomplete_json_is_refused"]),
    ("s_scrub.py", "                    finally:\n                        gs.__exit__()\n", "                    finally:\n                        pass\n", ["ScrubTests.test_a_gpu_sampler_is_stopped_when_the_proof_cannot_run"]),
    ("s_scrub.py", "        script = scrub_script()  #", "        script = '/nope'  #", ["ScrubTests.test_a_wrong_repository_root_fails_before_any_editor_starts"]),
    ("drv.py", 'ROOT = os.path.abspath(os.environ.get("TESSEL_REPO_ROOT") or os.path.join(HERE, *[os.pardir] * 4))', 'ROOT = "/Users/someone/.t3/worktrees/tessel-editor/t3-129bf52a"', ["RootTests.test_root_is_derived_from_the_driver_file_whatever_the_cwd", "RootTests.test_an_explicit_root_wins", "RootTests.test_no_developer_path_is_committed_in_the_drivers"]),
    ("drv.py", '    if not os.path.isfile(path):\n        raise RuntimeError(', '    if False:\n        raise RuntimeError(', ["ScrubTests.test_a_wrong_repository_root_fails_before_any_editor_starts"]),
    ("mainthread.py", "    if start is None:", "    if False:", ["MainThreadTests.test_a_file_without_a_main_thread_header_fails_instead_of_reporting_another_thread", "MainThreadTests.test_the_cli_exits_non_zero_without_a_traceback"]),
    ("mainthread.py", "    if len(nodes) < 2:", "    if False:", ["MainThreadTests.test_a_header_without_nodes_fails_clearly"]),
    ("s_video.py", '            if not r.get("ok"):\n                print("OP FAIL"', '            if False:\n                print("OP FAIL"', ["VideoTests.test_a_rejected_clip_add_stops_the_run_and_records_no_video_measurement"]),
    ("s_video.py", "        if not imported:", "        if False:", ["VideoTests.test_an_import_that_never_finishes_is_an_error_not_a_division_by_zero"]),
    ("s_video.py", "        if not_ok:", "        if False:", ["VideoTests.test_assets_that_are_not_ok_are_refused"]),
    ("s_video.py", "                if rd is None:", "                if False:", ["VideoTests.test_media_that_never_becomes_ready_skips_the_measurement"]),
    ("s_video.py", "                    if code not in NOT_READY_CODES:", "                    if False:", ["VideoTests.test_a_render_frame_failure_that_is_not_preparing_stops_at_once"]),
    ("s_video.py", '.append({"kind": "disk_low"', '.append({"kind": "nope"', ["VideoTests.test_a_disk_low_abort_is_marked_and_keeps_the_counts_already_measured"]),
]
NESTED = bool(os.environ.get("TESSEL_SPEC_NESTED"))


@unittest.skipIf(NESTED, "mutants are run by the outer test run")
class GuardMutationTests(unittest.TestCase):
    """Each guard removed from a copy of the drivers makes at least one named test above fail."""

    def run_nested(self, drivers_dir, tests):
        env = {**os.environ, "TESSEL_DRIVERS_DIR": str(drivers_dir), "TESSEL_SPEC_NESTED": "1"}
        return subprocess.run([sys.executable, str(HERE / "drivers.spec.py"), *tests], capture_output=True, text=True, env=env, cwd=str(drivers_dir))

    def test_the_unmutated_copy_passes_the_tests_the_mutants_must_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            copy = Path(tmp) / "drivers"
            shutil.copytree(DRIVERS, copy, ignore=shutil.ignore_patterns("__pycache__"))
            tests = sorted({t for m in MUTANTS for t in m[3]})
            run = self.run_nested(copy, tests)
            self.assertEqual(run.returncode, 0, run.stderr[-2000:])

    def test_every_guard_is_discriminated(self):
        survivors = []
        with tempfile.TemporaryDirectory() as tmp:
            for n, (name, old, new, tests) in enumerate(MUTANTS):
                copy = Path(tmp) / f"m{n}"
                shutil.copytree(DRIVERS, copy, ignore=shutil.ignore_patterns("__pycache__"))
                source = (copy / name).read_text()
                self.assertEqual(source.count(old), 1, f"mutation {n} ({name}) must match exactly once: {old[:60]!r}")
                (copy / name).write_text(source.replace(old, new))
                run = self.run_nested(copy, tests)
                if run.returncode == 0:
                    survivors.append((n, name, old[:60]))
        self.assertEqual(survivors, [], "these mutants still pass: the tests do not discriminate the guard")

    def test_a_mutant_only_fails_the_tests_of_its_own_guard(self):
        """Negative control: a broken guard does not make every test fail, only the ones that discriminate it."""
        with tempfile.TemporaryDirectory() as tmp:
            copy = Path(tmp) / "drivers"
            shutil.copytree(DRIVERS, copy, ignore=shutil.ignore_patterns("__pycache__"))
            source = (copy / "mainthread.py").read_text()
            self.assertEqual(source.count("    if start is None:"), 1)
            (copy / "mainthread.py").write_text(source.replace("    if start is None:", "    if False:"))
            own = self.run_nested(copy, ["MainThreadTests.test_a_file_without_a_main_thread_header_fails_instead_of_reporting_another_thread"])
            other = self.run_nested(copy, ["ScrubTests.test_a_good_scrub_is_recorded_from_the_fresh_file", "PlaybackFailureTests.test_a_healthy_sweep_is_complete_and_exits_zero"])
            self.assertNotEqual(own.returncode, 0)
            self.assertEqual(other.returncode, 0, other.stderr[-1500:])


if __name__ == "__main__":
    unittest.main()
