"""What every experiment does around its own science, once: arm order, the prerequisites of a run, the manifest, the run loop, loading a saved
batch and publishing the analysis.

An experiment hands in an `Experiment` (how its raw records are named, how they become a report, how the report reads as a text and whether it
ended as it claims) and a function that plays one session; nothing else about a batch is the experiment's business."""
import argparse
import json
import pathlib
import time
import uuid
from collections import namedtuple
from dataclasses import dataclass
from typing import Callable

from ab import ROOT, warm_proof
from ab.project import check_sources, git_state, parse_project, sha256_file
from ab.session import MIN_FREE_GIB_START

RUNTIME_JS = ROOT / "runtime" / "dist" / "runtime.js"
Run = namedtuple("Run", "binary document project locks lock_wait_s")


@dataclass(frozen=True)
class Experiment:
    """`record` names one raw record (formatted with `index`, `arm` and `pair`), `build(records, project, manifest)` is its report, `markdown(report)`
    its text, `ok(report)` whether the batch ran as it claims, `stem` the name of the analysis files and `charts(records, project, report, folder)`
    an optional writer of charts (names returned)."""
    record: str
    build: Callable
    markdown: Callable
    ok: Callable
    stem: str = "analysis"
    charts: Callable = None

    def names(self, order):
        return [self.record.format(index=index, arm=arm, pair=index // 2) for index, arm in enumerate(order)]

    def stored(self, out):
        """Names of the raw records of this experiment that `out` holds, whichever batch they belong to."""
        return sorted(path.name for path in out.glob(self.record.split("{")[0] + "*.json"))


def positive_int(text):
    """argparse `type=` of every count of sessions a runner takes (`--pairs`, `--iterations`): a whole number of at least 1. A batch of none runs no editor and
    has nothing to be wrong about, so it must not come out as a success; a usage error (exit 2) before any prerequisite, lock or folder is touched."""
    try:
        value = int(text)
    except ValueError:
        value = 0
    if value < 1:
        raise argparse.ArgumentTypeError(f"{text!r} is not a whole number of at least 1 (a batch of no session measures nothing)")
    return value


def arm_order(arms, pairs):
    """first/second, second/first, first/second, ...: which arm goes first alternates by pair, so a drift of the machine is spread over both arms."""
    first, second = arms
    order = []
    for pair in range(pairs):
        order += [first, second] if pair % 2 == 0 else [second, first]
    return order


def prerequisites(args, experiment, needed=(RUNTIME_JS,), gate=None):
    """The binary, the document and `needed` exist, `--out` holds no batch of `experiment` yet, `gate()` (a reason or None) lets the run start, the
    project's sources are present, the disk has room and both bench locks were taken. Returns a `Run`, or None after saying why it is BLOCKED.

    WHY `--out` must be new: a run writes a manifest and one record per place of its order, and `load_batch` refuses a folder that holds anything
    else. A folder that already holds a batch would be half overwritten and then refused, after the GPU hours were spent."""
    W = warm_proof()
    binary, document = pathlib.Path(args.binary).resolve(), pathlib.Path(args.project).resolve()
    for path in (binary, document, *needed):
        if not path.exists():
            print(f"BLOCKED: missing {path.name}")
            return None
    out = pathlib.Path(args.out)
    kept = (["manifest.json"] if (out / "manifest.json").exists() else []) + experiment.stored(out)
    if kept:
        print(f"BLOCKED: --out {args.out} already holds a batch ({len(kept)} file(s) such as {kept[0]}): use a new folder, nothing was written")
        return None
    why = gate() if gate else None
    if why:
        print(f"BLOCKED: {why}")
        return None
    project = parse_project(json.loads(document.read_text()))
    if [s for s in check_sources(project["sources"]) if not s["present"]]:
        print("BLOCKED: project source(s) missing")
        return None
    if W.free_gib() < MIN_FREE_GIB_START:
        print(f"BLOCKED: {W.free_gib():.1f} GiB free")
        return None
    waited = time.monotonic()
    locks = W.acquire_locks(args.lock_wait)
    if locks is None:
        print(f"BLOCKED: the GPU bench locks were not free within {args.lock_wait:.0f} s")
        return None
    return Run(binary, document, project, locks, round(time.monotonic() - waited, 1))


def manifest_of(run, order, runtime_js=RUNTIME_JS, **extra):
    """The identity of a batch: what was run (code, binary, page, document), in which order, under which locks, and a `batch_id` its raw records carry."""
    manifest = {"batch_id": uuid.uuid4().hex, "git": git_state(), "binary_sha256": sha256_file(run.binary),
                "document_sha256": sha256_file(run.document), "order": order, "locks": [handle.name for handle in run.locks],
                "lock_wait_s": run.lock_wait_s, "started_unix": time.time(), **extra}
    if runtime_js:
        manifest["runtime_sha256"] = sha256_file(runtime_js)
    return manifest


def run_arms(run, out, manifest, experiment, session, summary=None):
    """One fresh session per place of `manifest["order"]`: the manifest is written first and again at the end, every raw record the moment it exists,
    and the locks are released on every path. `session(index, arm)` returns the raw record, `summary(record)` a line to show."""
    out.mkdir(parents=True, exist_ok=True)
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2))
    print(f"build {manifest['git']['sha'][:9]} dirty={manifest['git']['dirty_files']} binary {manifest['binary_sha256'][:12]} order {manifest['order']}", flush=True)
    records = []
    try:
        for index, arm in enumerate(manifest["order"]):
            record = session(index, arm)
            record.update(arm=arm, pair=index // 2, order_index=index, batch_id=manifest["batch_id"])
            (out / experiment.names(manifest["order"])[index]).write_text(json.dumps(record))
            records.append(record)
            if summary:
                print(summary(record), flush=True)
    finally:
        for handle in run.locks:
            handle.close()
    manifest["finished_unix"] = time.time()
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2))
    return records


def load_batch(out, experiment, manifest):
    """The raw records of the batch `manifest` describes: exactly one file per place of its order, each carrying its `batch_id`. A folder that holds
    fewer, more or other records (a reused folder, a deleted file) is refused, never analysed as if it were whole."""
    expected = experiment.names(manifest["order"])
    present = experiment.stored(out)
    if present != sorted(expected):
        raise SystemExit(f"refused: the folder does not hold exactly the batch of its manifest (missing {sorted(set(expected) - set(present))}, "
                         f"extra {sorted(set(present) - set(expected))})")
    records = [json.loads((out / name).read_text()) for name in expected]
    foreign = [name for name, record in zip(expected, records) if not manifest.get("batch_id") or record.get("batch_id") != manifest["batch_id"]]
    if foreign:
        raise SystemExit(f"refused: {foreign} do not carry the batch_id of the manifest (another batch, or raw data from before batches were stamped)")
    return records


def finish(experiment, out, records, project, manifest, report_dir=None, charts=False):
    """Analysis of `records`: `<stem>.json`, `<stem>.md`, the sha256 of every raw record, copies in `report_dir` when asked (never the raw records),
    the text on the screen. Returns the exit code: 0 when the batch ran as it claims (and holds at least one session)."""
    report = experiment.build(records, project, manifest)
    text = experiment.markdown(report)
    files = {f"{experiment.stem}.json": json.dumps(report, indent=2), f"{experiment.stem}.md": text,
             "runs.sha256": "".join(f"{sha256_file(out / name)}  {name}\n" for name in experiment.names(manifest["order"]))}
    for name, content in files.items():
        (out / name).write_text(content)
    drawn = experiment.charts(records, project, report, out / "charts") if charts and experiment.charts else []
    if report_dir:
        target = pathlib.Path(report_dir)
        target.mkdir(parents=True, exist_ok=True)
        for name, content in files.items():
            (target / name).write_text(content)
        for name in drawn:
            (target / name).write_bytes((out / "charts" / name).read_bytes())
    print(text)
    return 0 if records and experiment.ok(report) else 1  # `all([])` is true: a batch that holds no session ran as nothing claims


def analyze(experiments, args):
    """`analyze --out DIR [--project DOC] [--report-dir DIR2]`: the same analysis as the run, from the saved batch. `--project` must be the document the
    sessions played (its sha256 is the manifest's). `experiments` is the runner's one `Experiment`, or, for a runner with several, a dict by the `kind`
    its manifests carry: there is one way to analyse a batch, never one per command."""
    out = pathlib.Path(args.out)
    manifest = json.loads((out / "manifest.json").read_text())
    experiment = experiments
    if not isinstance(experiments, Experiment):
        experiment = experiments.get(manifest.get("kind"))
        if experiment is None:
            raise SystemExit(f"refused: the manifest's kind {manifest.get('kind')!r} is not one of {sorted(experiments)}")
    project = None
    if getattr(args, "project", None):
        document = pathlib.Path(args.project)
        if sha256_file(document) != manifest["document_sha256"]:
            raise SystemExit("refused: --project is not the document these sessions played (its sha256 is not the manifest's)")
        project = parse_project(json.loads(document.read_text()))
    return finish(experiment, out, load_batch(out, experiment, manifest), project, manifest, args.report_dir, getattr(args, "charts", False))
