"""Tests of what every A/B runner does around its own science (`scripts/ab/batch.py`): the prerequisites of a run, the manifest, the run loop,
saving and analysing a batch. No editor, GPU, runtime page or private media is started; the lab (`scene-warm-proof.py`) is a fake."""
import contextlib
import fcntl
import io
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ab import batch as B  # noqa: E402
from ab import warm_proof  # noqa: E402
from ab.project import sha256_file  # noqa: E402

PROJECT = {"length": 600, "fps": 24.0, "cuts": [186], "video_starts": [], "sources": [{"path": "/private/not-published.mov", "kind": "video"}]}
ORDER = ["first", "second", "second", "first"]


def build(records, project, manifest):
    return {"manifest": manifest, "n": len(records), "project": bool(project), "all_ran": not any(record.get("failed") for record in records)}


EXPERIMENT = B.Experiment(record="run-{index:02d}-{arm}.json", build=build, markdown=lambda report: f"# report of {report['n']} sessions", stem="analysis",
                          ok=lambda report: report["all_ran"])


class Locks:
    def __init__(self, name):
        self.name, self.closed = name, 0

    def close(self):
        self.closed += 1


class Warm:
    """The part of the lab a run asks: the free disk and the bench locks."""

    def __init__(self, free=40.0, locks=True):
        self.free, self.locks, self.waited = free, locks, None

    def free_gib(self):
        return self.free

    def acquire_locks(self, wait):
        self.waited = wait
        return [Locks("/tmp/gpu.lock"), Locks("/tmp/review.lock")] if self.locks else None


class Folder(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.runtime = self.root / "runtime.js"
        self.runtime.write_text("runtime")
        self.binary = self.root / "tessel-editor"
        self.binary.write_text("binary")
        self.document = self.root / "p.tessel.json"
        self.document.write_text("{}")
        self.args = SimpleNamespace(binary=str(self.binary), project=str(self.document), out=str(self.root / "out"), lock_wait=7.0, report_dir=None, charts=False)
        for patch in (mock.patch.object(B, "parse_project", return_value=PROJECT), mock.patch.object(B, "check_sources", return_value=[{"present": True}]),
                      mock.patch.object(B, "git_state", return_value={"sha": "c" * 40, "dirty_files": 1, "diff_sha256": None})):
            patch.start()
            self.addCleanup(patch.stop)

    def prerequisites(self, warm, gate=None):
        shown = io.StringIO()
        with mock.patch.object(B, "warm_proof", return_value=warm), contextlib.redirect_stdout(shown):
            return B.prerequisites(self.args, needed=(self.runtime,), gate=gate), shown.getvalue()

    def run_batch(self, sessions=None):
        """`run_arms` over ORDER with a fake session; returns (records or the error, the locks, what the sessions were asked)."""
        run, _ = self.prerequisites(Warm())
        manifest = B.manifest_of(run, ORDER, runtime_js=self.runtime, pairs=2)
        asked = []

        def session(index, arm):
            asked.append((index, arm))
            if sessions is not None and index >= sessions:
                raise RuntimeError("the editor never opened")
            return {"number": index, "mode": "x"}

        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            try:
                result = B.run_arms(run, Path(self.args.out), manifest, EXPERIMENT, session, lambda record: f"  ran {record['order_index']} {record['arm']}")
            except RuntimeError as error:
                result = error
        return result, run.locks, asked, manifest, shown.getvalue()


class PrerequisiteTests(Folder):
    def test_everything_in_place_gives_the_binary_the_document_the_project_and_the_locks(self):
        warm = Warm()
        run, shown = self.prerequisites(warm)
        self.assertEqual((run.binary, run.document, run.project), (self.binary.resolve(), self.document.resolve(), PROJECT))
        self.assertEqual([handle.name for handle in run.locks], ["/tmp/gpu.lock", "/tmp/review.lock"])
        self.assertEqual(warm.waited, 7.0)
        self.assertIsInstance(run.lock_wait_s, float)
        self.assertEqual(shown, "")

    def test_a_missing_binary_or_needed_file_is_blocked_by_name(self):
        self.binary.unlink()
        run, shown = self.prerequisites(Warm())
        self.assertIsNone(run)
        self.assertEqual(shown, "BLOCKED: missing tessel-editor\n")
        self.binary.write_text("binary")
        self.runtime.unlink()
        run, shown = self.prerequisites(Warm())
        self.assertIsNone(run)
        self.assertEqual(shown, "BLOCKED: missing runtime.js\n")

    def test_a_missing_document_is_blocked(self):
        self.document.unlink()
        run, shown = self.prerequisites(Warm())
        self.assertIsNone(run)
        self.assertIn("BLOCKED: missing p.tessel.json", shown)

    def test_a_missing_project_source_is_blocked(self):
        with mock.patch.object(B, "check_sources", return_value=[{"present": True}, {"present": False}]):
            run, shown = self.prerequisites(Warm())
        self.assertIsNone(run)
        self.assertEqual(shown, "BLOCKED: project source(s) missing\n")

    def test_too_little_disk_is_blocked_before_the_locks_are_taken(self):
        warm = Warm(free=B.MIN_FREE_GIB_START - 0.1)
        run, shown = self.prerequisites(warm)
        self.assertIsNone(run)
        self.assertIn("GiB free", shown)
        self.assertIsNone(warm.waited)
        self.assertIsNotNone(self.prerequisites(Warm(free=B.MIN_FREE_GIB_START))[0])

    def test_locks_that_are_not_free_in_time_are_blocked(self):
        run, shown = self.prerequisites(Warm(locks=False))
        self.assertIsNone(run)
        self.assertIn("BLOCKED: the GPU bench locks were not free within 7 s", shown)

    def test_a_gate_that_gives_a_reason_blocks_before_the_disk_and_the_locks(self):
        warm = Warm()
        run, shown = self.prerequisites(warm, gate=lambda: "the rule is not fixed")
        self.assertIsNone(run)
        self.assertEqual(shown, "BLOCKED: the rule is not fixed\n")
        self.assertIsNone(warm.waited)


class RunTests(Folder):
    def test_one_session_per_place_of_the_order_and_the_locks_are_released(self):
        records, locks, asked, _, shown = self.run_batch()
        self.assertEqual(asked, list(enumerate(ORDER)))
        self.assertEqual([record["arm"] for record in records], ORDER)
        self.assertEqual([lock.closed for lock in locks], [1, 1])
        self.assertIn("ran 0 first", shown)
        self.assertIn("ran 3 first", shown)

    def test_every_record_is_written_as_it_exists_with_its_pair_its_place_and_the_batch(self):
        _, _, _, manifest, _ = self.run_batch()
        out = Path(self.args.out)
        self.assertEqual(sorted(path.name for path in out.glob("run-*.json")), ["run-00-first.json", "run-01-second.json", "run-02-second.json", "run-03-first.json"])
        record = json.loads((out / "run-02-second.json").read_text())
        self.assertEqual((record["pair"], record["order_index"], record["arm"], record["batch_id"]), (1, 2, "second", manifest["batch_id"]))

    def test_the_manifest_names_what_was_run_under_which_locks_and_for_how_long_it_waited(self):
        _, _, _, _, _ = self.run_batch()
        saved = json.loads((Path(self.args.out) / "manifest.json").read_text())
        self.assertEqual(saved["order"], ORDER)
        self.assertEqual(saved["binary_sha256"], sha256_file(self.binary))
        self.assertEqual(saved["document_sha256"], sha256_file(self.document))
        self.assertEqual(saved["runtime_sha256"], sha256_file(self.runtime))
        self.assertEqual(saved["git"]["sha"], "c" * 40)
        self.assertEqual(saved["locks"], ["/tmp/gpu.lock", "/tmp/review.lock"])
        self.assertIsInstance(saved["lock_wait_s"], float)
        self.assertEqual(saved["pairs"], 2)
        self.assertIn("finished_unix", saved)
        self.assertRegex(saved["batch_id"], r"^[0-9a-f]{32}$")

    def test_every_batch_has_its_own_identity(self):
        run, _ = self.prerequisites(Warm())
        self.assertNotEqual(B.manifest_of(run, ORDER, runtime_js=None)["batch_id"], B.manifest_of(run, ORDER, runtime_js=None)["batch_id"])
        self.assertNotIn("runtime_sha256", B.manifest_of(run, ORDER, runtime_js=None), "a runner whose arms have their own bundles names them itself")

    def test_a_session_that_raises_still_releases_the_locks_and_keeps_what_ran(self):
        result, locks, _, _, _ = self.run_batch(sessions=2)
        self.assertIsInstance(result, RuntimeError)
        self.assertEqual([lock.closed for lock in locks], [1, 1])
        out = Path(self.args.out)
        self.assertEqual(sorted(path.name for path in out.glob("run-*.json")), ["run-00-first.json", "run-01-second.json"])
        self.assertTrue((out / "manifest.json").exists())


class AnalysisTests(Folder):
    def finish(self, records, report_dir=None, manifest=None):
        out = Path(self.args.out)
        out.mkdir(exist_ok=True)
        manifest = manifest or {"order": ["first", "second"], "batch_id": "b" * 32}
        for name in EXPERIMENT.names(manifest["order"]):
            (out / name).write_text(json.dumps({"name": name}))
        (out / "notes.txt").write_text("not a run")
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            code = B.finish(EXPERIMENT, out, records, PROJECT, manifest, report_dir)
        return code, {path.name: path.read_text() for path in out.iterdir()}, shown.getvalue()

    def test_the_files_a_batch_leaves_are_the_analysis_and_the_hashes_of_the_raw_records(self):
        code, files, printed = self.finish([{}, {}])
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(files["analysis.json"])["n"], 2)
        self.assertEqual(files["analysis.md"], "# report of 2 sessions")
        self.assertEqual(printed, files["analysis.md"] + "\n")
        lines = files["runs.sha256"].splitlines()
        self.assertEqual([line.split("  ")[1] for line in lines], ["run-00-first.json", "run-01-second.json"])
        self.assertTrue(all(re.fullmatch(r"[0-9a-f]{64}", line.split("  ")[0]) for line in lines))

    def test_the_report_folder_gets_the_analysis_and_the_hashes_never_the_raw_records(self):
        target = Path(self.tmp.name) / "report"
        _, files, _ = self.finish([{}, {}], report_dir=str(target))
        copied = {path.name: path.read_text() for path in target.iterdir()}
        self.assertEqual(sorted(copied), ["analysis.json", "analysis.md", "runs.sha256"])
        for name in copied:
            self.assertEqual(copied[name], files[name])

    def test_the_exit_code_is_one_when_the_batch_did_not_run_as_it_claims_and_the_report_is_still_written(self):
        code, files, _ = self.finish([{"failed": True}, {}])
        self.assertEqual(code, 1)
        self.assertIn("analysis.md", files)

    def test_charts_are_written_only_when_asked_and_a_report_folder_never_needs_them(self):
        drawn = []

        def charts(records, project, report, folder):
            folder.mkdir(parents=True, exist_ok=True)
            (folder / "gaps.svg").write_bytes(b"<svg/>")
            drawn.append(folder)
            return ["gaps.svg"]

        with_charts = B.Experiment(record=EXPERIMENT.record, build=build, markdown=EXPERIMENT.markdown, ok=EXPERIMENT.ok, charts=charts)
        out, target = Path(self.args.out), Path(self.tmp.name) / "report"
        out.mkdir()
        manifest = {"order": ["first"], "batch_id": "b" * 32}
        (out / "run-00-first.json").write_text("{}")
        with contextlib.redirect_stdout(io.StringIO()):
            B.finish(with_charts, out, [{}], PROJECT, manifest, str(target), charts=False)
            self.assertEqual((drawn, (out / "charts").exists(), sorted(path.name for path in target.iterdir())), ([], False, ["analysis.json", "analysis.md", "runs.sha256"]))
            B.finish(with_charts, out, [{}], PROJECT, manifest, str(target), charts=True)
        self.assertEqual(len(drawn), 1)
        self.assertEqual((target / "gaps.svg").read_bytes(), b"<svg/>")

    def test_analyze_reads_the_saved_records_again_under_the_manifest_the_run_wrote(self):
        result = self.run_batch()
        out = Path(self.args.out)
        saved = json.loads((out / "manifest.json").read_text())
        self.assertFalse((out / "analysis.md").exists(), "a run that has not been analysed yet")
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            code = B.analyze(EXPERIMENT, self.args)
        self.assertEqual((result[0][0]["arm"], code), ("first", 0))
        self.assertEqual(json.loads((out / "analysis.json").read_text())["manifest"], saved)
        self.assertIn("report of 4 sessions", shown.getvalue())
        self.assertEqual(len((out / "runs.sha256").read_text().splitlines()), 4)


class LoadBatchTests(Folder):
    """Reading a saved batch back: exactly the records of the manifest's order, every one of them from that batch."""

    def saved(self, names=None, manifest=None, stamp=None):
        out = Path(self.args.out)
        out.mkdir(exist_ok=True)
        manifest = manifest or {"order": ORDER, "batch_id": "b" * 32}
        for name in names if names is not None else EXPERIMENT.names(manifest["order"]):
            record = {"name": name} if stamp is False else {"name": name, "batch_id": stamp or manifest.get("batch_id")}
            (out / name).write_text(json.dumps(record))
        return out, manifest

    def refused(self, out, manifest):
        with self.assertRaises(SystemExit) as raised:
            B.load_batch(out, EXPERIMENT, manifest)
        return str(raised.exception)

    def test_the_records_of_the_batch_come_back_in_the_order_of_its_manifest(self):
        out, manifest = self.saved()
        self.assertEqual([record["name"] for record in B.load_batch(out, EXPERIMENT, manifest)], EXPERIMENT.names(ORDER))
        self.assertEqual(len(B.load_batch(out, EXPERIMENT, manifest)), len(ORDER))

    def test_a_missing_record_is_refused_and_named(self):
        names = EXPERIMENT.names(ORDER)
        out, manifest = self.saved(names=names[:-1])
        text = self.refused(out, manifest)
        self.assertIn("refused", text)
        self.assertIn(names[-1], text)

    def test_an_extra_record_from_a_bigger_older_batch_is_refused_and_named(self):
        out, manifest = self.saved(names=EXPERIMENT.names(ORDER) + ["run-09-first.json"])
        self.assertIn("run-09-first.json", self.refused(out, manifest))

    def test_a_record_of_another_batch_is_refused_even_under_the_right_name(self):
        out, manifest = self.saved()
        name = EXPERIMENT.names(ORDER)[1]
        (out / name).write_text(json.dumps({"name": name, "batch_id": "a" * 32}))
        text = self.refused(out, manifest)
        self.assertIn(name, text)
        self.assertIn("batch_id", text)

    def test_a_lot_from_before_batches_were_stamped_is_refused_whole(self):
        out, manifest = self.saved(manifest={"order": ORDER}, stamp=False)
        self.assertIn("batch_id", self.refused(out, manifest))
        out, manifest = self.saved(manifest={"order": ORDER, "batch_id": "b" * 32}, stamp=False)
        self.assertIn("batch_id", self.refused(out, manifest))

    def test_a_manifest_without_a_batch_refuses_records_that_carry_none_too(self):
        out, manifest = self.saved(manifest={"order": ORDER}, stamp=None)
        self.assertIn("batch_id", self.refused(out, manifest), "two missing identities are not the same identity")

    def test_other_files_of_the_folder_are_not_records(self):
        out, manifest = self.saved()
        (out / "manifest.json").write_text("{}")
        (out / "analysis.json").write_text("{}")
        (out / "runs.sha256").write_text("")
        self.assertEqual(len(B.load_batch(out, EXPERIMENT, manifest)), len(ORDER))


class AnalyzeFolderTests(Folder):
    """`analyze` over a folder a run wrote: the document must be the one the sessions played, and the experiment the manifest names."""

    def ran(self, **manifest_extra):
        run, _ = self.prerequisites(Warm())
        manifest = B.manifest_of(run, ORDER, runtime_js=None, **manifest_extra)
        with contextlib.redirect_stdout(io.StringIO()):
            B.run_arms(run, Path(self.args.out), manifest, EXPERIMENT, lambda index, arm: {"number": index})
        return manifest

    def analyze(self, experiments=EXPERIMENT):
        with contextlib.redirect_stdout(io.StringIO()):
            return B.analyze(experiments, self.args)

    def test_the_document_the_sessions_played_is_the_one_analysed_with(self):
        self.ran()
        parsed = []
        with mock.patch.object(B, "parse_project", side_effect=lambda document: parsed.append(document) or PROJECT):
            self.assertEqual(self.analyze(), 0)
        self.assertEqual(parsed, [{}])
        self.assertTrue(json.loads((Path(self.args.out) / "analysis.json").read_text())["project"])

    def test_another_document_is_refused_before_anything_is_read_or_written(self):
        self.ran()
        out = Path(self.args.out)
        (out / "analysis.json").write_text("old")
        self.document.write_text('{"another": "document"}')
        with self.assertRaises(SystemExit) as raised, mock.patch.object(B, "parse_project") as parse:
            self.analyze()
        self.assertIn("not the document", str(raised.exception))
        parse.assert_not_called()
        self.assertEqual((out / "analysis.json").read_text(), "old")

    def test_without_a_document_the_analysis_has_no_project_and_asks_nothing_of_it(self):
        self.ran()
        self.args.project = None
        self.assertEqual(self.analyze(), 0)
        self.assertFalse(json.loads((Path(self.args.out) / "analysis.json").read_text())["project"])

    def test_a_runner_with_several_experiments_analyses_the_one_its_manifest_names(self):
        self.ran(kind="second")
        built = []

        def experiment(name):
            return B.Experiment(record=EXPERIMENT.record, build=lambda records, project, manifest: built.append(name) or build(records, project, manifest),
                                markdown=EXPERIMENT.markdown, ok=EXPERIMENT.ok, stem=name)

        self.assertEqual(self.analyze({"first": experiment("first"), "second": experiment("second")}), 0)
        self.assertEqual(built, ["second"])
        self.assertTrue((Path(self.args.out) / "second.json").exists())
        self.assertFalse((Path(self.args.out) / "first.json").exists())

    def test_a_manifest_that_names_no_known_kind_is_refused(self):
        self.ran()
        with self.assertRaises(SystemExit) as raised:
            self.analyze({"first": EXPERIMENT})
        self.assertIn("kind", str(raised.exception))


class LockFileTests(unittest.TestCase):
    def test_a_run_releases_its_locks_and_never_deletes_the_lock_files(self):
        proof = warm_proof()
        with tempfile.TemporaryDirectory() as folder:
            paths = [str(Path(folder) / "a.lock"), str(Path(folder) / "b.lock")]
            held = proof.acquire_locks(0, paths)
            self.assertEqual([handle.name for handle in held], paths)
            run = B.Run(Path(folder) / "bin", Path(folder) / "doc", PROJECT, held, 0.0)
            with contextlib.redirect_stdout(io.StringIO()), mock.patch.object(B, "git_state", return_value={"sha": "c" * 40, "dirty_files": 0}), \
                    mock.patch.object(B, "sha256_file", return_value="0" * 64):
                manifest = B.manifest_of(run, ["first"], runtime_js=None)
                B.run_arms(run, Path(folder) / "out", manifest, EXPERIMENT, lambda index, arm: {})
            self.assertTrue(all(Path(path).exists() for path in paths), "a lock is a file other benchmarks wait on: it is released, never removed")
            again = proof.acquire_locks(0, paths)
            self.assertEqual(len(again), 2, "closed handles leave the locks free")
            for handle in again:
                fcntl.flock(handle, fcntl.LOCK_UN)
                handle.close()
            self.assertEqual(manifest["locks"], paths)


if __name__ == "__main__":
    unittest.main()
