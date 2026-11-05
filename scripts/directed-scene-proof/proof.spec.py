#!/usr/bin/env python3
"""Self-tests of the harness itself (not of the editor): its comparators must fail when they should.

    python3 scripts/directed-scene-proof/proof.spec.py

These tests use no editor binary and no renderer. They guard against a harness that passes
everything: tolerance edges, a missing/uninspected pixel, a wrong layer colour, the BLOCKED
mechanism, and the interface template filling. They are NOT evidence for any phase.
"""
import importlib.util
import json
import pathlib
import sys
import unittest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import harness as h  # noqa: E402

spec = importlib.util.spec_from_file_location("corpus_gen", HERE / "corpus.py")
corpus_gen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(corpus_gen)
EXP = json.loads((h.CORPUS / "expected.json").read_text())
PIXEL = EXP["tolerances"]["pixel"]


class Comparators(unittest.TestCase):
    def test_numeric_tolerance_edges(self):
        tol = {"absolute": 1e-9, "relative": 1e-12}
        self.assertTrue(h.close(1.0 + 5e-10, 1.0, tol["absolute"], tol["relative"]))
        self.assertFalse(h.close(1.0 + 5e-9, 1.0, tol["absolute"], tol["relative"]))
        self.assertTrue(h.close(1e6 + 1e-7, 1e6, tol["absolute"], tol["relative"]), "relative term scales with magnitude")
        self.assertFalse(h.matrix_close([0.0] * 15, [0.0] * 16, tol), "a short matrix never matches")

    def test_pixels_match_inside_the_tolerance_and_fail_outside(self):
        probes = EXP["frames"][0]["pixelProbes"]
        exact = [p["color"] + [255] for p in probes]
        self.assertEqual(h.compare_probes(probes, exact, PIXEL), [])
        nudged = [[min(255, c + PIXEL["maxChannelDelta"]) for c in p["color"]] + [255] for p in probes]
        self.assertEqual(h.compare_probes(probes, nudged, PIXEL), [], "exactly at the tolerance passes")
        over = [[min(255, c + PIXEL["maxChannelDelta"] + 1) if c < 200 else c - PIXEL["maxChannelDelta"] - 1 for c in p["color"]] + [255]
                for p in probes]
        self.assertEqual(len(h.compare_probes(probes, over, PIXEL)), len(probes), "one unit over fails every probe")

    def test_a_missing_or_short_read_is_never_a_match(self):
        probes = EXP["frames"][0]["pixelProbes"]
        self.assertTrue(h.compare_probes(probes, [], PIXEL), "no pixels at all")
        self.assertTrue(h.compare_probes(probes, [None] * len(probes), PIXEL), "pixels not read")
        self.assertTrue(h.compare_probes(probes, [[1, 2]] * len(probes), PIXEL), "pixels without channels")

    def test_every_asset_has_a_colour_of_its_own_on_every_frame(self):
        # If two assets shared a colour, a wrong draw order or a wrong texture could pass unnoticed.
        # `echo` deliberately reuses the `hero` asset (one resource, two users), so the two are
        # told apart by position only; the corpus never overlaps them (documented limit).
        asset_of = {"sky": "sky", "mid": "mid", "hero": "hero", "echo": "hero"}
        for frame in EXP["frames"]:
            by_asset = {}
            for probe in frame["pixelProbes"]:
                by_asset.setdefault(asset_of[probe["layer"]], set()).add(tuple(probe["color"]))
            for asset, mine in by_asset.items():
                others = {c for other, theirs in by_asset.items() if other != asset for c in theirs}
                self.assertTrue(mine - others, f"frame {frame['editFrame']}: {asset} has no colour of its own")
                self.assertFalse(mine & others, f"frame {frame['editFrame']}: {asset} shares a colour with another asset")

    def test_the_swap_changes_the_expected_colour_of_the_swapped_layer_only(self):
        colours = lambda frames, layer: {tuple(p["color"]) for f in frames for p in f["pixelProbes"] if p["layer"] == layer}
        self.assertFalse(colours(EXP["frames"], "hero") & colours(EXP["swapFrames"], "hero"))
        self.assertEqual(colours(EXP["frames"], "sky") | colours(EXP["swapFrames"], "sky"), colours(EXP["frames"], "sky"))


class Interfaces(unittest.TestCase):
    def test_placeholders_are_filled_and_null_values_are_dropped(self):
        iface = {"op": "x", "args": {"scene": "$scene", "frame": "$frame", "out": "$out", "fixed": 3}}
        self.assertEqual(h.request_for(iface, scene="s", frame=0, out=None), {"op": "x", "scene": "s", "frame": 0, "fixed": 3})

    def test_an_unknown_operation_blocks_and_any_other_answer_does_not(self):
        iface = {"op": "scene.nope", "args": {}, "needs": "n", "backlog": "BL000"}
        with self.assertRaises(h.Blocked) as caught:
            h.probe_interface(lambda r: (False, {"code": "unknown_operation", "message": "m"}), iface)
        self.assertEqual(caught.exception.backlog, "BL000")
        self.assertEqual(h.probe_interface(lambda r: (False, {"code": "backend_unavailable"}), iface), (False, {"code": "backend_unavailable"}))
        self.assertEqual(h.probe_interface(lambda r: (True, {"frame": 1}), iface), (True, {"frame": 1}))

    def test_the_shipped_interfaces_name_a_backlog_item_and_a_need(self):
        for key, iface in h.load_interfaces().items():
            self.assertTrue(iface["backlog"].startswith("BL"), key)
            self.assertTrue(iface["needs"], key)


class Corpus(unittest.TestCase):
    def test_the_committed_corpus_is_what_the_generator_writes(self):
        self.assertEqual(corpus_gen.check(h.CORPUS), [])

    def test_a_tampered_corpus_is_detected(self):
        import shutil
        import tempfile
        with tempfile.TemporaryDirectory(prefix="tsp-spec-", dir="/tmp") as scratch:
            copy = pathlib.Path(scratch) / "corpus"
            shutil.copytree(h.CORPUS, copy)
            (copy / "assets/hero.png").write_bytes(b"x")
            (copy / "stray.txt").write_text("x")
            problems = corpus_gen.check(copy)
            self.assertIn("differs assets/hero.png", problems)
            self.assertIn("unexpected stray.txt", problems)


class PortableProvenance(unittest.TestCase):
    def test_python_minor_version_does_not_change_corpus_bytes(self):
        from unittest.mock import patch
        with patch("platform.python_version_tuple", return_value=("3", "12", "0")):
            first = corpus_gen.generate()
        with patch("platform.python_version_tuple", return_value=("3", "99", "8")):
            second = corpus_gen.generate()
        self.assertEqual(first, second)


class JsonlProcesses(unittest.TestCase):
    def session(self, code, timeout=2):
        session = h.JsonlSession(sys.executable, ["-u", "-c", code], timeout=timeout)
        self.addCleanup(session.close)
        return session

    def test_stderr_larger_than_a_pipe_does_not_block_the_answer(self):
        session = self.session("import sys,json;sys.stdin.readline();sys.stderr.write('x'*262144);sys.stderr.flush();print(json.dumps({'ok':True,'result':42}))")
        self.assertEqual(session({"op":"test"}), (True,42))

    def test_closed_process_reports_the_tail_of_stderr(self):
        session = self.session("import sys;sys.stdin.readline();sys.stderr.write('x'*262144+'TAIL-MARKER');sys.stderr.flush()")
        with self.assertRaisesRegex(h.Abort,"TAIL-MARKER"):
            session({"op":"closed"})

    def test_silent_process_times_out_and_is_reaped(self):
        session = self.session("import sys,time;sys.stdin.readline();time.sleep(30)", timeout=0.15)
        with self.assertRaisesRegex(h.Abort,"timeout waiting for silent"):
            session({"op":"silent"})
        self.assertIsNotNone(session.process.poll())

    def test_partial_line_also_times_out(self):
        session = self.session("import sys,time;sys.stdin.readline();sys.stdout.write('{');sys.stdout.flush();time.sleep(30)",timeout=0.15)
        with self.assertRaisesRegex(h.Abort,"timeout"):
            session({"op":"partial"})


if __name__ == "__main__":
    unittest.main(verbosity=2)
