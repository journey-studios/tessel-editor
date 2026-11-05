#!/usr/bin/env python3
"""Independent acceptance of the directed-scene export (BL008, BL019; front R10).

    python3 scripts/directed-scene-export-proof/proof.py --list
    python3 scripts/directed-scene-export-proof/proof.py --phases corpus,verifier_controls_scene
    python3 scripts/directed-scene-export-proof/proof.py --report /tmp/te-r10-report.json
    # GPU phases (Chrome/SwiftShader) under the shared bench lock, taken by the wrapper:
    python3 scripts/directed-scene-proof/with-gpu-lock.py python3 scripts/directed-scene-export-proof/proof.py --gpu-lock-held

Exit status (never 0 unless every selected phase ran and passed):
    0  all selected phases PASS
    1  at least one FAIL
    3  no FAIL, but at least one phase is BLOCKED (an operation or prerequisite is missing: NOT a pass)
    2  usage / prerequisite error of the harness itself

What each group proves, and only that (details: docs/migration/scene-export-proof.md):
  * data / harness phases     -> the corpus and the verifier are right, with positive AND negative controls
                                 built from the independent model and plain ffmpeg. Not evidence about the export.
  * baseline_* phases         -> the corpus holds against the renderer that exists today (scene.render through the
                                 `tessel-render` CLI) and the verifier works on real renderer pixels encoded by
                                 THIS harness. Not evidence about `scene.export`.
  * export_* phases           -> `scene.export` itself, through the `tessel-render` JSONL process (CLI surface).
  * socket_export             -> the same op through the native window socket of an isolated editor instance.
  * ui_export_flow            -> the flow.export_scene dialog driven by the semantic protocol only (no pointer), then the file
                                 judged like any export; BLOCKED while the build has no such node, never PASS by finding a node.
The Rust library surface is `crates/render/tests/scene_export_proof.rs`. A missing operation is BLOCKED (exit 3).
No coordinate input is used, nothing here hears audio or measures a photon, the user's app is never touched.
"""
import argparse
import hashlib
import importlib.util
import json
import os
import pathlib
import platform
import shutil
import subprocess
import sys
import tempfile
import time
from fractions import Fraction

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
P_DIR = ROOT / "scripts" / "directed-scene-proof"
sys.path.insert(0, str(HERE))


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules.setdefault(name, module)
    spec.loader.exec_module(module)
    return module


import export_controls as controls  # noqa: E402
import export_corpus as corpus  # noqa: E402
import export_verify as v  # noqa: E402

harness = _load("tsp_harness", P_DIR / "harness.py")  # read-only reuse: sessions, Phase, binary identity
Phase, Blocked, Abort = harness.Phase, harness.Blocked, harness.Abort
HARNESS_VERSION = 1
MIN_FREE_GIB = 10
WORK_BUDGET_BYTES = 2 * 1024 ** 3
ADAPTERS_PATH = HERE / "adapters.json"

NOT_CLAIMED = [
    "heard audio (BL046): audio is measured spectrally on decoded samples; nothing was listened to and no device is involved",
    "photons / seek-to-photon (BL032) and physical pointer input (BL043): no coordinate clicks, no display measurement",
    "other GPUs, platforms and codecs (BL054): one machine, SwiftShader GL unless --gl gpu, libx264 as found by ffmpeg",
    "BL052 colour contract beyond what the file declares (BT.709 limited) and what flat interiors decode to",
    "lossless or bit-exact frame equality: the codec is lossy; pixels are judged by probes, dense picture agreement and tolerances fixed in the corpus",
    "domains the plan refuses (light, weather, effects, rigs, paths, data maps, splat): the corpus does not use them",
    "that a lib/CLI result proves the window, or that the window's socket proves a visible dialog (BL086 UI is separate)",
]


# ---------------------------------------------------------------------------------- environment
def free_gib(path="/tmp"):
    stat = os.statvfs(path)
    return stat.f_bavail * stat.f_frsize / 1024 ** 3


class Workspace:
    """Own temporary directory (`te-r10-*`): budget 2 GiB, refuses under 10 GiB free, removes only itself."""

    def __init__(self, keep=False, base=None):
        if free_gib() < MIN_FREE_GIB:
            raise SystemExit(f"refusing to start: {free_gib():.1f} GiB free under /tmp, need {MIN_FREE_GIB}")
        self.keep = keep
        self.path = pathlib.Path(base) if base else pathlib.Path(tempfile.mkdtemp(prefix="te-r10-", dir="/tmp"))
        self.path.mkdir(parents=True, exist_ok=True)

    def sub(self, name):
        path = self.path / name
        path.mkdir(parents=True, exist_ok=True)
        return path

    def bytes_used(self):
        return harness.tree_bytes(self.path)

    def within_budget(self):
        return self.bytes_used() <= WORK_BUDGET_BYTES

    def close(self):
        if not self.keep:
            shutil.rmtree(self.path, ignore_errors=True)


class Channel:
    """One way to send a request: a JSONL process or the window socket. Same call shape either way."""

    def __init__(self, session=None, lab=None, surface=""):
        self.session, self.lab, self.surface = session, lab, surface

    def __call__(self, request, timeout=60):
        if self.session is not None:
            previous, self.session.timeout = self.session.timeout, timeout
            try:
                return self.session(request)
            finally:
                self.session.timeout = previous
        return self.lab.op(request, timeout)

    def close(self):
        if self.session is not None:
            self.session.close()


class Context:
    def __init__(self, args, work, expected, adapters):
        self.args, self.work, self.exp, self.adapters = args, work, expected, adapters
        self.gl = args.gl
        self.bins = {"headless": args.bin_headless, "render": args.bin_render, "editor": args.bin_editor}
        self.binaries = {}
        self.corpus_dir = pathlib.Path(args.corpus).resolve() if args.corpus else corpus.CORPUS_DIR
        self._materialized = None
        self._lab = None
        self._export_probe = None
        self.images = {}  # model pictures shared by every identity check of the run
        self.bundles = {}
        self.gpu_taken = args.gpu_lock_held

    def gpu(self):
        if not self.gpu_taken:
            harness.acquire_gpu_lock()
            self.gpu_taken = True

    def bundle(self, key):
        if key not in self.bundles:
            self.bundles[key] = json.loads((self.corpus_dir / self.exp["bundles"][key]["file"]).read_text())
        return self.bundles[key]

    def materialized(self):
        """The committed corpus plus the 33 MB 4K PNG, generated in the workspace and hash-checked."""
        if self._materialized is None:
            self._materialized = corpus.materialize(self.work.sub("corpus"))
        return self._materialized

    def editor_lab(self, binary):
        if self._lab is None:
            self.gpu()
            self._lab = harness.EditorLab(binary, self.work.sub("home"))
            self._lab.wait_ready(90)
        return self._lab

    def close(self):
        if self._lab is not None:
            self._lab.stop()


# ---------------------------------------------------------------------------------- requests
def must(op, request, timeout=60):
    ok, body = op(request, timeout)
    if not ok:
        raise Abort(f"{request['op']} failed: {json.dumps(body)[:300]}")
    return body


def require_binary(ctx, key, hint):
    path = ctx.bins[key]
    if not pathlib.Path(path).is_file():
        raise Blocked(f"binary not built: {path}", f"build it with the coordinator's serialized helper, e.g. {hint}", "BL019")
    ctx.binaries[key] = harness.binary_info(path)
    return path


RENDER_HINT = "python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py build -p tessel-editor-render --locked"
EDITOR_HINT = "python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py build -p tessel-editor --release --locked"


def history(op):
    body = must(op, {"op": "history.status"})
    return {"undo_depth": body["history"]["undo_depth"], "redo_depth": body["history"]["redo_depth"],
            "revision": body["project"]["revision"]}


def wait_job(ctx, op, job, timeout=60):
    """Polls job.get until the job is final; returns the last info."""
    deadline = time.monotonic() + timeout
    interval = ctx.adapters["limits"]["poll_seconds"]
    while time.monotonic() < deadline:
        info = must(op, harness.request_for(ctx.adapters["ops"]["job_get"], job=job))
        if info["state"] in ("succeeded", "failed", "cancelled"):
            return info
        time.sleep(interval)
    raise Abort(f"job {job} still not final after {timeout}s")


def resolve_resources(ctx, op, scene, base=None):
    """Loads the scene's resources by assetId. `base` is the folder the caller authorizes; after a restore it is
    omitted on purpose (the origin is gone: nothing may need it)."""
    request = {"op": "scene.resources.resolve", "scene": scene}
    if base is not None:
        request["base"] = str(base)
    loaded = must(op, request)
    if loaded.get("state") == "queued":
        info = wait_job(ctx, op, loaded["job"], 60)
        if info["state"] != "succeeded":
            raise Abort(f"resource job failed: {info.get('error')}")
        committed, commit = op({"op": "job.commit", "job": loaded["job"]})
        if not committed and commit.get("code") != "job_finished":
            raise Abort(f"resource job could not be applied: {commit}")
    return loaded


def new_project(op, name, workspace):
    must(op, {"op": "project.create", "name": name, "dir": str(workspace)})


def attach(op, scene, path):
    return must(op, {"op": "scene.attach", "id": scene, "path": str(path)})


def open_render(ctx, name, bundle_key="main", scene="proof", base="corpus", extra_scenes=()):
    """A `tessel-render` process with a fresh project and the scene attached and resolved from the corpus."""
    ctx.gpu()
    binary = require_binary(ctx, "render", RENDER_HINT)
    work = ctx.work.sub(name)
    channel = Channel(session=harness.JsonlSession(binary, ["--gl", ctx.gl]), surface="tessel-render JSONL (CLI)")
    source = ctx.materialized() if bundle_key == "detail" or base == "materialized" else ctx.corpus_dir
    try:
        new_project(channel, name, work / "ws")
        attach(channel, scene, source / ctx.exp["bundles"][bundle_key]["file"])
        resolve_resources(ctx, channel, scene, source)
        for extra_scene, extra_key in extra_scenes:
            attach(channel, extra_scene, ctx.corpus_dir / ctx.exp["bundles"][extra_key]["file"])
            resolve_resources(ctx, channel, extra_scene, ctx.corpus_dir)
    except BaseException:
        channel.close()
        raise
    return channel, work, source


# ---------------------------------------------------------------------------------- export op
def rate_of(value):
    """Rational from the shapes a report may use: 'n/d', {num,den}, {numerator,denominator}, number."""
    if isinstance(value, str):
        return v.rational(value)
    if isinstance(value, dict):
        for num_key, den_key in (("num", "den"), ("numerator", "denominator")):
            if num_key in value and den_key in value and value[den_key]:
                return Fraction(int(value[num_key]), int(value[den_key]))
        return None
    if isinstance(value, (int, float)):
        return Fraction(value).limit_denominator(100000)
    return None


def dig(value, path):
    for key in [k for k in str(path).split(".") if k] if path else []:
        if not isinstance(value, dict) or key not in value:
            return None
        value = value[key]
    return value


class ExportRun:
    def __init__(self, ok, result, error, seconds, job=None, info=None):
        self.ok, self.result, self.error, self.seconds, self.job, self.info = ok, result, error, seconds, job, info


def export_request(ctx, scene, preset, destination, frm, frames, wait, replace=None, audio=None):
    values = {"scene": scene, "preset": preset, "destination": str(destination), "from": frm, "frames": frames,
              "wait": wait, "replace": replace, "audio": audio}
    return harness.request_for(ctx.adapters["ops"]["export"], **values)


def send_export(ctx, ch, request, timeout):
    """Sends a scene.export request; `unknown_operation` is BLOCKED, any other answer is data."""
    spec = ctx.adapters["ops"]["export"]
    ok, body = ch(request, timeout)
    if not ok and body.get("code") == "unknown_operation":
        raise Blocked(f"`{spec['op']}` is not implemented ({body.get('message')})", spec["needs"], spec["backlog"])
    if not ok and body.get("code") == "backend_unavailable":
        # The op exists but this surface has no renderer attached (e.g. the window host before the coordinator
        # wires `with_scene_renderer`): a missing integration, not a failed export, so never a PASS and never a FAIL.
        raise Blocked(f"`{spec['op']}` answers backend_unavailable on this surface ({body.get('message')})",
                      "attach the scene renderer to this surface's MediaHost (MediaHost::with_scene_renderer)", spec["backlog"])
    return ok, body


def settle(ctx, ch, ok, body, started, timeout):
    """From a first answer to a final one: a waited export carries its result, an async one a job to poll."""
    spec = ctx.adapters["ops"]["export"]
    if not ok:
        return ExportRun(False, None, body, time.monotonic() - started)
    result_path = spec["result"]["path"]
    result = dig(body, result_path) if result_path else body
    job = body.get(spec["async"]["job_key"]) if isinstance(body, dict) else None
    if isinstance(result, dict) and spec["result"]["fields"]["destination"] in result:
        return ExportRun(True, result, None, time.monotonic() - started, job)
    if not job:
        return ExportRun(False, None, {"code": "no_result", "message": f"answer has neither a result nor a job: {json.dumps(body)[:200]}"},
                         time.monotonic() - started)
    info = wait_job(ctx, ch, job, timeout)
    elapsed = time.monotonic() - started
    if info["state"] == "succeeded":
        return ExportRun(True, info.get("result"), None, elapsed, job, info)
    return ExportRun(False, None, info.get("error") or {"code": info["state"]}, elapsed, job, info)


def run_export(ctx, ch, scene, preset, destination, frm, frames, replace=None, timeout=None, budget=300, audio=None):
    started = time.monotonic()
    limit = timeout or budget + 120
    ok, body = send_export(ctx, ch, export_request(ctx, scene, preset, destination, frm, frames, True, replace, audio), limit)
    return settle(ctx, ch, ok, body, started, limit)


def ensure_export_op(ctx):
    """BLOCKED unless `scene.export` exists: asked once with a request that is invalid on purpose (0 frames), so an
    implemented op answers with a typed error and an absent one answers `unknown_operation`. Nothing is written."""
    if ctx._export_probe is None:
        binary = require_binary(ctx, "render", RENDER_HINT)
        channel = Channel(session=harness.JsonlSession(binary, ["--gl", ctx.gl]))
        try:
            request = export_request(ctx, "proof", "fullhd", ctx.work.sub("probe") / "never.mp4", 0, 0, True)
            try:
                send_export(ctx, channel, request, 60)
                ctx._export_probe = True
            except Blocked as blocked:
                ctx._export_probe = blocked
        finally:
            channel.close()
    if isinstance(ctx._export_probe, Blocked):
        raise ctx._export_probe


def compare_report(ctx, p, label, result, destination, case):
    """The result the export answers vs what was measured in the file. Absent fields are listed, never assumed."""
    paths = ctx.adapters["ops"]["export"]["result"]["fields"]
    unreported = []

    def got(name):
        value = dig(result, paths[name])
        if value is None:
            unreported.append(name)
        return value
    destination_value = got("destination")
    if destination_value is not None:
        p.check(f"{label}: result.destination is the file that exists",
                os.path.realpath(destination_value) == os.path.realpath(destination), destination_value)
    frames = got("frames")
    if frames is not None:
        p.check(f"{label}: result.frames equals the measured count ({case['frames']})", frames == case["frames"], frames)
    fps = got("fps")
    if fps is not None:
        p.check(f"{label}: result.fps equals the file's exact rate ({case['fps']})", rate_of(fps) == v.rational(case["fps"]), fps)
    duration = got("duration_seconds")
    if duration is not None:
        want = float(v.rational(case["durationSeconds"]))
        p.check(f"{label}: result.duration_seconds equals frames*den/num", abs(float(duration) - want) <= 1e-3, f"{duration} vs {want}")
    width, height = got("output_width"), got("output_height")
    if width is not None and height is not None:
        p.check(f"{label}: result.output is {case['output'][0]}x{case['output'][1]}", [width, height] == case["output"], [width, height])
    verification = got("verification")
    if isinstance(verification, dict) and "ok" in verification:
        p.check(f"{label}: the export's own verification reports ok", verification["ok"] is True, verification)
    for name in ("audio", "warnings"):
        got(name)
    resources = got("resources")
    if isinstance(resources, list):
        # What the export says it drew from must be the bundle's own assets, by id, revision and content hash:
        # these come from the corpus (the bundle's declared contentHash), not from the export.
        declared = {asset["assetId"]: asset for asset in ctx.bundle(case["bundle"])["assets"]}
        reported = {entry.get("asset"): entry for entry in resources if isinstance(entry, dict)}
        p.check(f"{label}: result.resources names exactly the bundle's assets", set(reported) == set(declared),
                sorted(reported))
        wrong = [name for name, entry in reported.items()
                 if name in declared and (entry.get("sha256") != declared[name]["contentHash"]
                                          or entry.get("revision") != declared[name]["revisionId"])]
        p.check(f"{label}: each resource carries the bundle's content hash and revision", not wrong, wrong)
    return unreported


def verify_case(ctx, p, label, run, destination, case_id, bundle_key, audio="none"):
    """Everything R10 checks about one successful export of a corpus case."""
    case = ctx.exp["cases"][case_id]
    p.check(f"{label}: the export answers ok", run.ok, run.error)
    if not run.ok:
        return {}
    p.check(f"{label}: the file exists at the destination", pathlib.Path(destination).is_file(), str(destination))
    budget = case["budgetSeconds"]
    p.check(f"{label}: wall clock {run.seconds:.1f}s within the {budget}s budget", run.seconds <= budget, run.seconds)
    expect = v.expectation(case_id, ctx.exp, ctx.bundle(bundle_key))
    metrics = v.verify_export(p, destination, expect, audio=audio, result=run.result, image_cache=ctx.images)
    metrics["seconds"] = round(run.seconds, 2)
    metrics["unreported_result_fields"] = compare_report(ctx, p, label, run.result, destination, case)
    return metrics


def scene_export_phase(case_id):
    def phase(ctx, p):
        ensure_export_op(ctx)
        case = ctx.exp["cases"][case_id]
        channel, work, _ = open_render(ctx, case_id, case["bundle"])
        try:
            destination = work / "out" / f"{case_id}.mp4"
            destination.parent.mkdir(parents=True, exist_ok=True)
            run = run_export(ctx, channel, "proof", case["preset"], destination, case["from"], case["frames"], budget=case["budgetSeconds"])
            p.metrics[case_id] = verify_case(ctx, p, case_id, run, destination, case_id, case["bundle"])
            p.metrics["surface"] = channel.surface
        finally:
            channel.close()
    return phase


# ---------------------------------------------------------------------------------- data / harness phases
def phase_corpus(ctx, p):
    problems = corpus.check(ctx.corpus_dir)
    p.check("committed corpus is byte-identical to what export_corpus.py writes", not problems, problems)
    provenance = json.loads((ctx.corpus_dir / "provenance.json").read_text())
    p.check("provenance declares no private media", provenance["private_media"] is False)
    for name, entry in provenance["files"].items():
        if name != "provenance.json":
            p.check(f"{name}: hash and size match provenance", harness.sha256_file(ctx.corpus_dir / name) == entry["sha256"])
    for key, entry in ctx.exp["bundles"].items():
        p.check(f"bundle {key}: expected.json pins its bytes", harness.sha256_file(ctx.corpus_dir / entry["file"]) == entry["sha256"])
    p.check("the 4K detail PNG is regenerated byte-for-byte and matches its recorded SHA-256 (not committed: 33 MB)",
            harness.sha256_file(ctx.materialized() / "assets/detail.png") == ctx.exp["assets"]["detail"]["sha256"]
            == provenance["notCommitted"]["assets/detail.png"]["sha256"])
    p.check("main, ntsc and audio bundles share one geometry (one set of probes serves them)",
            ctx.exp["geometry"]["bundles"] == ["main", "ntsc", "audio"])
    p.check("NTSC bundle carries the rational rate 30000/1001", ctx.exp["bundles"]["ntsc"]["fps"] == "30000/1001")
    ranges = [(c["from"], c["from"] + c["frames"]) for c in ctx.exp["cases"].values() if c["kind"] == "scene"]
    p.check("every scene frame of every case has probes on all four layers",
            all({q["layer"] for q in ctx.exp["geometry"]["frameProbes"][str(f)]} == {"sky", "mid", "echo", "hero"}
                for lo, hi in ranges for f in range(lo, hi)))
    p.check("the cases cross the shot cut (48) and the event boundary (12) and the 4K case straddles the cut",
            any(c["from"] < 48 < c["from"] + c["frames"] for c in ctx.exp["cases"].values())
            and any(c["from"] <= 12 < c["from"] + c["frames"] for c in ctx.exp["cases"].values())
            and ctx.exp["cases"]["main_4k_cut"]["from"] < 48 < ctx.exp["cases"]["main_4k_cut"]["from"] + ctx.exp["cases"]["main_4k_cut"]["frames"])
    p.metrics["corpus"] = {"version": ctx.exp["corpusVersion"], "probe_frames": len(ctx.exp["geometry"]["frameProbes"]),
                           "cases": list(ctx.exp["cases"]), "bundles": {k: e["sha256"] for k, e in ctx.exp["bundles"].items()}}


def phase_model(ctx, p):
    run = subprocess.run([sys.executable, str(P_DIR / "model.spec.py")], capture_output=True, text=True)
    p.check("the reference model agrees with the origin oracle (scripts/directed-scene-proof/model.spec.py, read-only)",
            run.returncode == 0, run.stderr[-400:])
    p.metrics["model_spec"] = run.stderr.strip().splitlines()[-1] if run.stderr.strip() else ""


def controls_phase(runner):
    def phase(ctx, p):
        work = ctx.work.sub("controls-" + runner.__name__)
        results, extra = runner(ctx, work)
        for result in results:
            p.check(("positive control: " if result["should_pass"] else "negative control: ") + result["control"], result["ok"],
                    result["failed_checks"] or "the control did not fail where it must")
        p.metrics["controls"] = results
        if extra:
            p.metrics["extra"] = extra
    return phase


def _scene_controls(ctx, work):
    return controls.run_scene_controls(work, ctx.exp, ctx.corpus_dir, log=lambda *a: None)


def _audio_controls(ctx, work):
    return controls.run_audio_controls(work, ctx.exp, ctx.corpus_dir, log=lambda *a: None), None


def _detail_controls(ctx, work):
    return controls.run_detail_controls(work, ctx.materialized() / "assets/detail.png", ctx.exp, log=lambda *a: None)


def phase_identity_coverage(ctx, p):
    """The model itself must tell neighbouring frames apart in every case, or the identity check is vacuous."""
    bundle = ctx.bundle("main")
    for case_id, case in ctx.exp["cases"].items():
        if case["kind"] != "scene":
            continue
        identity = v.Identity(bundle, case["from"], case["frames"], ctx.images)
        identity.prepare(range(case["from"], case["from"] + case["frames"]))
        judged = failures = 0
        for index in range(case["frames"]):
            problems, pairs = identity.judge(index, identity.image(case["from"] + index)[0])
            judged += 1 if pairs else 0
            failures += len(problems)
        p.check(f"{case_id}: the model tells neighbours apart on {judged} of {case['frames']} frames (>= 50%)",
                judged >= case["frames"] * v.ID_MIN_JUDGED_FRAMES, judged)
        p.check(f"{case_id}: the model agrees with itself (no self-contradiction)", failures == 0, failures)


# ---------------------------------------------------------------------------------- baseline (existing renderer)
def render_png(ctx, ch, scene, frame, width, height, out, probes=None, base=None):
    request = {"op": "scene.render", "scene": scene, "frame": frame, "width": width, "height": height, "out": str(out),
               "probes": probes or [], "resources_root": str(base or ctx.corpus_dir)}
    body = must(ch, request, 300)
    return body["frame"]


def phase_baseline_render(ctx, p):
    """Corpus vs the renderer that exists today (scene.render, 320x180, probes read back)."""
    ensure_binaries(ctx)
    tol = ctx.exp["tolerances"]["framePixel"]["maxChannelDelta"]
    channel, work, _ = open_render(ctx, "baseline", "main", extra_scenes=[("ntsc", "ntsc"), ("audio", "audio")])
    try:
        for scene, frames in (("proof", [0, 12, 20, 36, 47, 48, 55]), ("ntsc", [20, 31]), ("audio", [40])):
            for frame in frames:
                probes = ctx.exp["geometry"]["frameProbes"][str(frame)]
                result = render_png(ctx, channel, scene, frame, corpus.REF_W, corpus.REF_H, work / f"{scene}-{frame}.png",
                                    [[q["refX"], q["refY"]] for q in probes])
                p.check(f"{scene} frame {frame}: pixels were read back (inspected)", result.get("inspected") is True)
                bad = [f"{q['layer']}@({q['refX']},{q['refY']})" for q, got in zip(probes, result.get("probes", []))
                       if got is None or max(abs(a - b) for a, b in zip(got[:3], q["color"])) > tol]
                p.check(f"{scene} frame {frame}: {len(probes)} probes match the independent model within {tol}",
                        not bad and len(result.get("probes", [])) == len(probes), bad[:3])
    finally:
        channel.close()


def raw_rgb(png, width, height):
    run = subprocess.run([v.FFMPEG, "-v", "error", "-i", str(png), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"], capture_output=True)
    if run.returncode != 0 or len(run.stdout) != width * height * 3:
        raise Abort(f"cannot decode {png}: {run.stderr.decode()[-200:]}")
    return run.stdout


def phase_baseline_detail(ctx, p):
    """The renderer draws the original 3840x2160 PNG 1:1 (the premise of the detail case), PNG out, no codec."""
    ensure_binaries(ctx)
    channel, work, source = open_render(ctx, "baseline-detail", "detail")
    try:
        result = render_png(ctx, channel, "proof", 0, 3840, 2160, work / "detail.png", base=source)
        p.check("the renderer answers a 3840x2160 frame for the detail scene", result.get("inspected") is True)
        metrics = v.detail_frame_metrics(raw_rgb(work / "detail.png", 3840, 2160), 3840, 2160)
        p.check("1/2/4 px stripes are drawn 1:1 by the renderer (contrast >= 0.98): no proxy, no resampling",
                all(value >= 0.98 for value in metrics["contrast"].values()), metrics["contrast"])
        p.check("colour blocks and markers of the original PNG are where and what they should be",
                metrics["block_delta"] <= 6 and metrics["marker_delta"] <= 6, metrics)
        p.metrics["detail_render"] = {k: (round(x, 4) if isinstance(x, float) else x) for k, x in metrics["contrast"].items()}
    finally:
        channel.close()


def ensure_binaries(ctx):
    require_binary(ctx, "render", RENDER_HINT)
    for tool in (v.FFMPEG, v.FFPROBE):
        if not pathlib.Path(tool).exists():
            raise Blocked(f"{tool} not found", "install ffmpeg/ffprobe (docs/media-setup.md)", "BL019")


def phase_baseline_encode(ctx, p):
    """Real renderer pixels encoded by THIS harness (ffmpeg, not the editor) and judged by the same verifier:
    evidence for the verifier on real pixels at 1080p, and for rational timing, never for scene.export."""
    ensure_binaries(ctx)
    channel, work, _ = open_render(ctx, "baseline-encode", "main", extra_scenes=[("ntsc", "ntsc")])
    try:
        for scene, first, count, fps_text in (("proof", 44, 8, "24/1"), ("ntsc", 20, 12, "30000/1001")):
            frames_dir = work / f"{scene}-frames"
            frames_dir.mkdir(exist_ok=True)
            for index in range(count):
                render_png(ctx, channel, scene, first + index, 1920, 1080, frames_dir / f"f{index:03d}.png")
            video = work / f"{scene}-baseline.mp4"
            subprocess.run([v.FFMPEG, "-v", "error", "-y", "-framerate", fps_text, "-i", str(frames_dir / "f%03d.png"),
                            "-vf", controls.RGB_TO_YUV, "-c:v", "libx264", "-crf", "18", "-preset", "fast", *controls.COLOR_TAGS,
                            "-r", fps_text, "-movflags", "+faststart", str(video)], check=True)
            expect = {"fps": v.rational(fps_text), "frames": count, "from": first, "width": 1920, "height": 1080, "kind": "scene",
                      "bundle": ctx.bundle("ntsc" if scene == "ntsc" else "main"),
                      "probes": {str(f): ctx.exp["geometry"]["frameProbes"][str(f)] for f in range(first, first + count)}}
            metrics = v.verify_export(p, video, expect, image_cache=ctx.images)
            p.metrics[f"baseline_{scene}"] = metrics
            shutil.rmtree(frames_dir, ignore_errors=True)
    finally:
        channel.close()


# ---------------------------------------------------------------------------------- export phases
def listing(directory):
    return sorted(str(path.relative_to(directory)) for path in pathlib.Path(directory).rglob("*"))


def phase_export_cancel(ctx, p):
    ensure_export_op(ctx)
    case = ctx.exp["cancelCase"]
    channel, work, _ = open_render(ctx, "cancel", case["bundle"])
    out = work / "out"
    out.mkdir()
    sentinel = out / "keep-me.txt"
    sentinel.write_bytes(b"belongs to the harness, must survive")
    destination = out / "cancelled.mp4"
    before = listing(out)
    state_before = history(channel)
    try:
        started = time.monotonic()
        ok, body = send_export(ctx, channel, export_request(ctx, "proof", case["preset"], destination, case["frm"], case["frames"], False), 120)
        if not p.check("an async export is accepted", ok, body):
            return
        job = body.get(ctx.adapters["ops"]["export"]["async"]["job_key"])
        if not job:
            raise Blocked("scene.export has no job to cancel: wait=false answered without a job id",
                          "an export job with progress and job.cancel, as export.start", ctx.adapters["ops"]["export"]["backlog"])
        deadline = time.monotonic() + ctx.adapters["limits"]["job_start_timeout_seconds"]
        info = None
        while time.monotonic() < deadline:
            info = must(channel, harness.request_for(ctx.adapters["ops"]["job_get"], job=job))
            if info["state"] in ("succeeded", "failed", "cancelled") or (info["state"] == "running" and (info.get("done") or 0) >= 1):
                break
            time.sleep(ctx.adapters["limits"]["poll_seconds"])
        p.check("the job reports progress while running (done >= 1) before it is cancelled",
                info is not None and info["state"] == "running" and (info.get("done") or 0) >= 1, info)
        if info["state"] != "running":
            return
        cancelled = must(channel, harness.request_for(ctx.adapters["ops"]["job_cancel"], job=job))
        final = wait_job(ctx, channel, job, 120)
        p.check("the job ends as cancelled (not succeeded, not failed)", final["state"] == "cancelled", final["state"])
        p.check("the job did not reach the end of the interval before the cancel", (final.get("done") or 0) < (final.get("total") or 10 ** 9),
                (final.get("done"), final.get("total")))
        p.check("no file at the destination", not destination.exists(), str(destination))
        p.check("the destination folder holds exactly what it held before: no staging, no partial file", listing(out) == before, listing(out))
        p.check("the file the harness put there is untouched", sentinel.read_bytes() == b"belongs to the harness, must survive")
        p.check("the session still answers and nothing was added to the history", history(channel) == state_before)
        p.metrics["cancel_seconds_to_final"] = round(time.monotonic() - started, 2)
        again = run_export(ctx, channel, "proof", case["preset"], destination, 0, 4, budget=120)
        p.check("the same destination is reusable right after the cancel (nothing left locked)", again.ok and destination.is_file(), again.error)
    finally:
        channel.close()


def phase_export_invalid(ctx, p):
    ensure_export_op(ctx)
    channel, work, _ = open_render(ctx, "invalid", "main")
    out = work / "out"
    out.mkdir()
    before_state, before_files = history(channel), listing(out)
    codes = ctx.adapters["error_codes"]
    try:
        scene_edit_frames = ctx.exp["bundles"]["main"]["editFrames"]
        p.check("the corpus scene has 96 frames (the range cases are written for it)", scene_edit_frames == 96)
        for item in ctx.exp["rangeErrors"]:
            destination = out / f"{item['id']}.mp4"
            run = run_export(ctx, channel, "proof", "fullhd", destination, item["frm"], item["frames"], budget=60)
            p.check(f"{item['id']}: refused with a typed error", (not run.ok) and bool((run.error or {}).get("code")), run.error)
            p.check(f"{item['id']}: no file was created", not destination.exists())
        unknown = run_export(ctx, channel, "no-such-scene", "fullhd", out / "unknown.mp4", 0, 4, budget=60)
        p.check("an unknown scene is refused with a typed error", (not unknown.ok) and bool((unknown.error or {}).get("code")), unknown.error)
        preset = run_export(ctx, channel, "proof", "no-such-preset", out / "preset.mp4", 0, 4, budget=60)
        p.check("an unknown preset is refused with a typed error", (not preset.ok) and bool((preset.error or {}).get("code")), preset.error)
        p.check("every rejection left the destination folder as it was", listing(out) == before_files, listing(out))
        p.check("every rejection left history and revision untouched", history(channel) == before_state)
        p.metrics["accepted_code_names"] = {"range": codes["range"], "unknown_preset": codes["unknown_preset"]}
    finally:
        channel.close()


def phase_export_destination_exists(ctx, p):
    ensure_export_op(ctx)
    channel, work, _ = open_render(ctx, "exists", "main")
    out = work / "out"
    out.mkdir()
    destination = out / "taken.mp4"
    marker = b"a file that was here first"
    destination.write_bytes(marker)
    before = listing(out)
    try:
        run = run_export(ctx, channel, "proof", "fullhd", destination, 0, 4, budget=120)
        p.check("an existing destination is refused", not run.ok, run.error)
        p.check("the refusal is typed", bool((run.error or {}).get("code")), run.error)
        p.check("the code is the one the media exporter uses (destination_exists)", (run.error or {}).get("code") in ctx.adapters["error_codes"]["destination_exists"], run.error)
        p.check("the file that was there is byte-identical", destination.read_bytes() == marker)
        p.check("the folder holds nothing else (no staging left)", listing(out) == before, listing(out))
        replaced = run_export(ctx, channel, "proof", "fullhd", destination, 0, 4, replace=True, budget=120)
        if replaced.ok:
            p.check("replace=true publishes the new file over the old one", destination.read_bytes() != marker)
            probe, metrics = v.verify_container(p, destination, {"fps": Fraction(24), "frames": 4, "width": 1920, "height": 1080})
        else:
            p.metrics["replace"] = {"offered": False, "error": replaced.error}
            p.check("if replace is not offered it is refused as a typed error and the old file survives",
                    bool((replaced.error or {}).get("code")) and destination.read_bytes() == marker, replaced.error)
    finally:
        channel.close()


def phase_export_snapshot(ctx, p):
    ensure_export_op(ctx)
    case_id = "snapshot_1080"
    case = ctx.exp["cases"][case_id]
    snapshot = ctx.exp["snapshot"]
    edit = snapshot["edit"]
    channel, work, _ = open_render(ctx, "snapshot", case["bundle"])
    out = work / "out"
    out.mkdir()
    destination = out / "snapshot.mp4"
    try:
        ok, body = send_export(ctx, channel, export_request(ctx, "proof", case["preset"], destination, case["from"], case["frames"], False), 120)
        if not p.check("an async export is accepted", ok, body):
            return
        job = body.get(ctx.adapters["ops"]["export"]["async"]["job_key"])
        if not job:
            raise Blocked("scene.export has no job: wait=false answered without a job id",
                          "an export job with progress, as export.start", ctx.adapters["ops"]["export"]["backlog"])
        deadline = time.monotonic() + ctx.adapters["limits"]["job_start_timeout_seconds"]
        info = None
        while time.monotonic() < deadline:
            info = must(channel, harness.request_for(ctx.adapters["ops"]["job_get"], job=job))
            if info["state"] != "queued" and (info["state"] != "running" or (info.get("done") or 0) >= 1):
                break
            time.sleep(ctx.adapters["limits"]["poll_seconds"])
        p.check("the export job is running (frames already produced) when the scene is edited", info["state"] == "running" and (info.get("done") or 0) >= 1, info)
        if info["state"] != "running":
            return
        must(channel, {"op": "scene.edit", "scene": "proof", "ops": [{"set": edit["hero_property"], "value": edit["hero_value"]},
                                                                      {"set": edit["fov_property"], "value": edit["fov_value"]}]})
        edited = must(channel, {"op": "scene.get", "scene": "proof", "property": edit["hero_property"]})["value"]
        p.check("the edit is in the live scene", all(abs(edited[k] - edit["hero_value"][k]) < 1e-12 for k in "xyz"), edited)
        still = must(channel, harness.request_for(ctx.adapters["ops"]["job_get"], job=job))
        p.check("the job had not finished when the edit landed (the test is not vacuous)", still["state"] == "running", still["state"])
        run = settle(ctx, channel, True, {ctx.adapters["ops"]["export"]["async"]["job_key"]: job}, time.monotonic(), ctx.adapters["limits"]["job_final_timeout_seconds"])
        p.check("the export finishes ok after the edit", run.ok, run.error)
        if not run.ok:
            return
        expect = v.expectation(case_id, ctx.exp, ctx.bundle("main"))
        p.metrics["unedited_scene_checks"] = v.verify_export(p, destination, expect, result=run.result, image_cache=ctx.images)
        # The contrast: where the edited scene would differ from the original, the file must show the original.
        width, height = expect["width"], expect["height"]
        tolerance = ctx.exp["tolerances"]["videoPixel"]["maxChannelDelta"]
        shows_edit = would_differ = 0
        for index, frame in enumerate(v.decode(destination, width, height)):
            scene_frame = case["from"] + index
            probes = ctx.exp["geometry"]["frameProbes"][str(scene_frame)]
            colors = v.probe_colors(frame, width, height, probes)
            for probe, got, edited_color in zip(probes, colors, snapshot["editedAtSamePositions"][str(scene_frame)]):
                if edited_color is None or max(abs(a - b) for a, b in zip(edited_color, probe["color"])) <= 2 * tolerance:
                    continue
                would_differ += 1
                if max(abs(a - b) for a, b in zip(got, edited_color)) <= tolerance:
                    shows_edit += 1
        p.check(f"the edit would change {would_differ} probes: enough to tell a snapshot from a live read", would_differ >= 20, would_differ)
        p.check("none of those probes shows the edited scene: the file holds the snapshot taken at the request", shows_edit == 0, shows_edit)
        # The edit was real: a render of the live scene now shows it (so the export did not simply miss an edit).
        frame_index = 20
        positions = [(q["refX"], q["refY"]) for q in ctx.exp["geometry"]["frameProbes"][str(frame_index)]]
        rendered = render_png(ctx, channel, "proof", frame_index, corpus.REF_W, corpus.REF_H, work / "after-edit.png", [list(x) for x in positions])
        wanted = snapshot["editedAtSamePositions"][str(frame_index)]
        agree = sum(1 for got, want in zip(rendered["probes"], wanted) if want is not None and max(abs(a - b) for a, b in zip(got[:3], want)) <= 6)
        p.check("a render of the live scene after the job does show the edit (control)", agree >= 0.9 * len([w for w in wanted if w is not None]), agree)
    finally:
        channel.close()


def audio_claimed(audio):
    """Whether a result says the file carries sound: `enabled` (proposed) or `included` (what R1 answers)."""
    if not isinstance(audio, dict):
        return bool(audio)
    return bool(audio.get("enabled") or audio.get("included"))


def phase_export_audio(ctx, p):
    """The scene declares an audio mix that nothing mixes into a directed export yet. Three outcomes are honest:
    the mix is in the file (measured), the export is refused with a typed code and no file, or the file is
    video-only and the result says so explicitly. A silent drop is the one outcome that fails."""
    ensure_export_op(ctx)
    case_id = "audio_1080"
    case = ctx.exp["cases"][case_id]
    channel, work, _ = open_render(ctx, "audio", "audio")
    out = work / "out"
    out.mkdir()
    fields = ctx.adapters["ops"]["export"]["result"]["fields"]
    refusal_codes = ctx.adapters["error_codes"]["audio_refusal"]
    try:
        destination = out / "audio.mp4"
        run = run_export(ctx, channel, "proof", case["preset"], destination, case["from"], case["frames"], budget=case["budgetSeconds"])
        if not run.ok:
            code = (run.error or {}).get("code")
            p.check("the default request on a scene that declares audio is refused with a typed code", code in refusal_codes, run.error)
            p.check("refused: no file and no staging left", listing(out) == [], listing(out))
            p.metrics["policy_observed"] = {"outcome": "refused", "code": code}
            # The explicit video-only request is the supported way out of the refusal; it must work and say what it left out.
            destination = out / "audio-video-only.mp4"
            run = run_export(ctx, channel, "proof", case["preset"], destination, case["from"], case["frames"],
                             budget=case["budgetSeconds"], audio=False)
            if not p.check("`audio: false` exports the picture of the same scene", run.ok, run.error):
                return
            p.metrics["policy_observed"]["video_only"] = True
            claimed = dig(run.result, fields["audio"])
            p.check("the video-only result states that the audio is not in the file", isinstance(claimed, dict) and not audio_claimed(claimed), claimed)
            p.check("the video-only result names what the scene declared and left out",
                    isinstance(claimed, dict) and bool(claimed.get("declared")) and bool(claimed.get("omitted")), claimed)
            warned = [w for w in (dig(run.result, fields["warnings"]) or [])
                      if isinstance(w, dict) and w.get("code") in ctx.adapters["error_codes"]["audio_omitted_warning_codes"]]
            p.check("the video-only result carries the typed warning that the declared audio was left out", bool(warned),
                    dig(run.result, fields["warnings"]))
            verify_case(ctx, p, "audio video-only", run, destination, case_id, "audio")
            return
        expect = v.expectation(case_id, ctx.exp, ctx.bundle("audio"))
        probe, metrics = v.verify_container(p, destination, expect)
        audios = [s for s in (probe or {"streams": []})["streams"] if s["codec_type"] == "audio"]
        if audios:
            p.metrics["policy_observed"] = {"outcome": "mixed"}
            p.metrics["audio"] = v.verify_mixed_audio(p, destination, probe, metrics.get("duration_seconds", 0.0))
            p.check("the result reports audio as present", audio_claimed(dig(run.result, fields["audio"])), dig(run.result, fields["audio"]))
            return
        warnings = dig(run.result, fields["warnings"]) or []
        known = ctx.adapters["error_codes"]["audio_not_mixed_warning_codes"]
        said = [w for w in warnings if isinstance(w, dict) and w.get("code") in known]
        p.metrics["policy_observed"] = {"outcome": "explicit_limit" if said else "dropped_silently", "warnings": warnings}
        p.check("no audio stream, so the result must say so with an explicit typed limit: a silent drop fails", bool(said), warnings)
        v.verify_no_audio(p, probe, run.result)
    finally:
        channel.close()


def restored_checks(ctx, p, label, channel, work, scene="proof"):
    """After the origin is gone: the scene is listed, a frame renders to the model's pixels, the export matches."""
    listing_ = must(channel, {"op": "scene.list"})
    p.check(f"{label}: the scene is back by its id", scene in json.dumps(listing_), listing_)
    resolve_resources(ctx, channel, scene, None)
    probes = ctx.exp["geometry"]["frameProbes"]["20"]
    tol = ctx.exp["tolerances"]["framePixel"]["maxChannelDelta"]
    body = must(channel, {"op": "scene.render", "scene": scene, "frame": 20, "width": corpus.REF_W, "height": corpus.REF_H,
                          "out": str(work / f"{label}-render.png"), "probes": [[q["refX"], q["refY"]] for q in probes]}, 300)["frame"]
    bad = [q["layer"] for q, got in zip(probes, body["probes"]) if max(abs(a - b) for a, b in zip(got[:3], q["color"])) > tol]
    p.check(f"{label}: a render without the origin matches the independent pixels", not bad and body.get("inspected") is True, bad[:3])
    case_id = "restored_1080"
    case = ctx.exp["cases"][case_id]
    destination = work / "out" / f"{label}.mp4"
    destination.parent.mkdir(exist_ok=True)
    run = run_export(ctx, channel, scene, case["preset"], destination, case["from"], case["frames"], budget=case["budgetSeconds"])
    p.metrics[label] = verify_case(ctx, p, label, run, destination, case_id, "main")


def phase_export_restored_package(ctx, p):
    ensure_export_op(ctx)
    ctx.gpu()
    binary = require_binary(ctx, "render", RENDER_HINT)
    work = ctx.work.sub("restored-package")
    origin = work / "origin"
    shutil.copytree(ctx.corpus_dir, origin)  # the harness' own copy: the only thing deleted below
    archive, workspace, restored = work / "scene.zip", work / "ws", work / "restored"
    first = Channel(session=harness.JsonlSession(binary, ["--gl", ctx.gl]))
    try:
        new_project(first, "pkg", workspace)
        attach(first, "proof", origin / "bundle.main.json")
        must(first, {"op": "project.save", "path": str(workspace / "pkg.tessel.json")})
        must(first, {"op": "scene.resources.adopt", "scene": "proof", "source": str(origin)})
        report = must(first, {"op": "project.package_preflight", "path": str(archive)})
        if not report.get("complete") or any(b.get("code") == "unsupported" for b in report.get("blockers", [])):
            raise Blocked(f"package preflight refuses a project with a directed scene ({json.dumps(report.get('missing'))[:200]})",
                          "BL011: the package carries the scene's resources", "BL011")
        must(first, {"op": "project.package", "path": str(archive)}, 300)
        must(first, {"op": "project.verify", "path": str(archive)}, 300)
    finally:
        first.close()
    shutil.rmtree(origin)  # own temporary copy; from here nothing may need the origin
    p.check("the origin copy is gone", not origin.exists())
    second = Channel(session=harness.JsonlSession(binary, ["--gl", ctx.gl]))
    try:
        must(second, {"op": "project.restore", "path": str(archive), "folder": str(restored)}, 300)
        found = {harness.sha256_file(f) for f in restored.rglob("*") if f.is_file()}
        for asset_id in ("sky", "mid", "hero"):
            p.check(f"restored root carries {asset_id} byte-identical to the corpus", ctx.exp["assets"][asset_id]["sha256"] in found)
        restored_checks(ctx, p, "restored-from-package", second, work)
    finally:
        second.close()
    documents = sorted(restored.rglob("*.tessel.json"))
    if not p.check("the restored folder holds a project document to reopen in a new process", bool(documents), listing(restored)[:10]):
        return
    third = Channel(session=harness.JsonlSession(binary, ["--gl", ctx.gl]))
    try:
        must(third, {"op": "project.open", "path": str(documents[0])})
        restored_checks(ctx, p, "reopened-after-restore", third, work)
    finally:
        third.close()


def phase_export_restored_managed(ctx, p):
    ensure_export_op(ctx)
    ctx.gpu()
    binary = require_binary(ctx, "render", RENDER_HINT)
    work = ctx.work.sub("restored-managed")
    origin, workspace = work / "origin", work / "ws"
    shutil.copytree(ctx.corpus_dir, origin)
    first = Channel(session=harness.JsonlSession(binary, ["--gl", ctx.gl]))
    document = workspace / "managed.tessel.json"
    try:
        new_project(first, "managed", workspace)
        attach(first, "proof", origin / "bundle.main.json")
        must(first, {"op": "scene.resources.adopt", "scene": "proof", "source": str(origin)})
        must(first, {"op": "project.save", "path": str(document)})
    finally:
        first.close()
    shutil.rmtree(origin)
    p.check("the origin copy is gone", not origin.exists())
    second = Channel(session=harness.JsonlSession(binary, ["--gl", ctx.gl]))
    try:
        must(second, {"op": "project.open", "path": str(document)})
        restored_checks(ctx, p, "opened-with-managed-base", second, work)
    finally:
        second.close()


def phase_socket_export(ctx, p):
    ensure_export_op(ctx)
    binary = require_binary(ctx, "editor", EDITOR_HINT)
    lab = ctx.editor_lab(binary)
    channel = Channel(lab=lab, surface="native window socket (isolated editor instance)")
    work = ctx.work.sub("socket-export")
    new_project(channel, "sockexport", work / "ws")
    attach(channel, "proof", ctx.corpus_dir / "bundle.main.json")
    resolve_resources(ctx, channel, "proof", ctx.corpus_dir)
    case_id = "restored_1080"
    case = ctx.exp["cases"][case_id]
    destination = work / "out" / "socket.mp4"
    destination.parent.mkdir()
    run = run_export(ctx, channel, "proof", case["preset"], destination, case["from"], case["frames"], budget=case["budgetSeconds"])
    p.metrics["socket"] = verify_case(ctx, p, "socket", run, destination, case_id, "main")
    p.metrics["note"] = "the window exports through the same shared operation; this is not evidence of a visible export dialog"


def collect_ids(value, found=None):
    """Every string `id` of every dict inside a listing (a jobs list may be a list, or an object holding one)."""
    found = set() if found is None else found
    if isinstance(value, dict):
        if isinstance(value.get("id"), str):
            found.add(value["id"])
        for child in value.values():
            collect_ids(child, found)
    elif isinstance(value, list):
        for child in value:
            collect_ids(child, found)
    return found


def live_node_ids(lab):
    tree = lab.send({"command": "tree"})
    if not tree.get("ok"):
        raise Abort(f"the live `tree` command failed: {json.dumps(tree)[:300]}")
    return collect_ids(tree.get("data"))


class Dialog:
    """Semantic access to the open dialog of an isolated editor instance: get/set/invoke only, no pointer."""

    def __init__(self, lab):
        self.lab = lab

    def send(self, request, timeout=60):
        return self.lab.send(request, timeout)

    def state(self, target):
        answer = self.send({"command": "get", "target": target})
        return answer["data"]["state"] if answer.get("ok") else None

    def get(self, target):
        return self.send({"command": "get", "target": target})

    def fill(self, **fields):
        for name, value in fields.items():
            answer = self.send({"command": "set", "target": f"dialog.field.{name}", "field": "value", "value": value})
            if not answer.get("ok"):
                raise Abort(f"cannot set dialog.field.{name}: {json.dumps(answer)[:300]}")

    def invoke(self, target, action=None):
        request = {"command": "invoke", "target": target}
        if action:
            request["action"] = action
        return self.send(request)


def phase_ui_export_flow(ctx, p):
    """The export dialog of the window, driven only through the semantic protocol (get/set/invoke), then the file it
    wrote judged exactly like a headless export. Proves the dialog reaches the shared operation and says what it left
    out; it does not prove that a person can see or reach the control (no pixels, no pointer: BL043)."""
    binary = require_binary(ctx, "editor", EDITOR_HINT)
    lab = ctx.editor_lab(binary)
    channel = Channel(lab=lab, surface="native window socket (isolated editor instance)")
    ui = Dialog(lab)
    work = ctx.work.sub("ui-export")
    new_project(channel, "uiexport", work / "ws")
    attach(channel, "proof", ctx.corpus_dir / "bundle.main.json")
    resolve_resources(ctx, channel, "proof", ctx.corpus_dir)
    prefixes = ctx.adapters["ui"]["export_node_prefixes"]
    nodes = sorted(i for i in live_node_ids(lab) if any(i.startswith(prefix) for prefix in prefixes))
    if not nodes:
        raise Blocked("the live UI tree has no export control for a directed scene (none of " + ", ".join(prefixes) + ")",
                      "the coordinator wires flow.export_scene (scene.export through the shared operation) into the "
                      "window build under test; rebuild it with the serialized helper", ctx.adapters["ui"]["backlog"])
    p.check("the live tree exposes the export control by its semantic id", "flow.export_scene" in nodes, nodes[:6])
    out = work / "out"
    out.mkdir()
    jobs_before = collect_ids(must(channel, {"op": "job.list"}))

    opened = ui.invoke("flow.export_scene", "open")
    p.check("invoking flow.export_scene opens the dialog", opened.get("ok") and (opened.get("data") or {}).get("flow") == "export_scene", opened)
    dialog = ui.state("dialog") or {}
    p.check("the dialog is the scene export dialog", dialog.get("flow") == "export_scene", dialog)
    fields = {name: ui.get(f"dialog.field.{name}") for name in ("scene", "preset", "from", "frames", "audio", "destination")}
    p.check("every field is a semantic node", all(entry.get("ok") for entry in fields.values()),
            {k: v.get("error") for k, v in fields.items() if not v.get("ok")})
    audio = (fields["audio"].get("data") or {})
    p.check("audio offers yes/no and defaults to yes (the omission is the user's explicit choice)",
            audio.get("value") == "yes" and (audio.get("fields") or [{}])[0].get("options") == ["yes", "no"], audio)

    case = ctx.exp["cases"]["restored_1080"]
    destination = out / "ui.mp4"
    ui.fill(scene="proof", preset=case["preset"], destination=str(destination))
    ui.fill(**{"from": str(case["from"]), "frames": "abc"})
    refused = ui.invoke("dialog.submit")
    p.check("a non-numeric frame count is refused with a typed code", not refused.get("ok") and (refused.get("error") or {}).get("code") == "invalid_value", refused)
    p.check("the dialog stays open and says why", (ui.state("dialog") or {}).get("flow") == "export_scene" and bool((ui.state("dialog") or {}).get("error")), ui.state("dialog"))
    p.check("a refused submit started no job and left no file",
            collect_ids(must(channel, {"op": "job.list"})) == jobs_before and listing(out) == [], listing(out))

    ui.fill(frames=str(case["frames"]))
    started = time.monotonic()
    submitted = ui.invoke("dialog.submit")
    if not p.check("a valid submit starts the export", submitted.get("ok"), submitted):
        return
    p.check("the dialog closed", not ui.get("dialog").get("ok"), ui.get("dialog"))
    new_jobs = set()
    deadline = time.monotonic() + ctx.adapters["limits"]["job_start_timeout_seconds"]
    while not new_jobs and time.monotonic() < deadline:
        new_jobs = collect_ids(must(channel, {"op": "job.list"})) - jobs_before
        time.sleep(ctx.adapters["limits"]["poll_seconds"])
    if not p.check("the submit created exactly one job", len(new_jobs) == 1, sorted(new_jobs)):
        return
    info = wait_job(ctx, channel, next(iter(new_jobs)), case["budgetSeconds"])
    run = ExportRun(info["state"] == "succeeded", info.get("result"), info.get("error"), time.monotonic() - started, next(iter(new_jobs)), info)
    p.metrics["ui_export"] = verify_case(ctx, p, "ui export", run, destination, "restored_1080", "main")

    # Audio: a scene that declares a mix is refused by default, and only the user's explicit `no` leaves it out.
    attach(channel, "withaudio", ctx.corpus_dir / "bundle.audio.json")
    resolve_resources(ctx, channel, "withaudio", ctx.corpus_dir)
    audio_case = ctx.exp["cases"]["audio_1080"]
    audio_destination = out / "ui-audio.mp4"
    ui.invoke("flow.export_scene", "open")
    ui.fill(scene="withaudio", preset=audio_case["preset"], destination=str(audio_destination),
            audio="yes", **{"from": str(audio_case["from"]), "frames": str(audio_case["frames"])})
    jobs_before = collect_ids(must(channel, {"op": "job.list"}))
    refused = ui.invoke("dialog.submit")
    code = (refused.get("error") or {}).get("code")
    p.check("audio yes on a scene that declares a mix is refused with a typed code (never exported silently)",
            not refused.get("ok") and code in ctx.adapters["error_codes"]["audio_refusal"], refused)
    p.check("the refusal keeps the dialog open, starts no job and writes no file",
            (ui.state("dialog") or {}).get("flow") == "export_scene" and collect_ids(must(channel, {"op": "job.list"})) == jobs_before
            and not audio_destination.exists(), ui.state("dialog"))
    ui.fill(audio="no")
    started = time.monotonic()
    submitted = ui.invoke("dialog.submit")
    if p.check("audio no is accepted: the user chose to leave the mix out", submitted.get("ok"), submitted):
        new_jobs = set()
        deadline = time.monotonic() + ctx.adapters["limits"]["job_start_timeout_seconds"]
        while not new_jobs and time.monotonic() < deadline:
            new_jobs = collect_ids(must(channel, {"op": "job.list"})) - jobs_before
            time.sleep(ctx.adapters["limits"]["poll_seconds"])
        if p.check("exactly one job was started", len(new_jobs) == 1, sorted(new_jobs)):
            info = wait_job(ctx, channel, next(iter(new_jobs)), audio_case["budgetSeconds"])
            run = ExportRun(info["state"] == "succeeded", info.get("result"), info.get("error"), time.monotonic() - started,
                            next(iter(new_jobs)), info)
            p.metrics["ui_export_audio_no"] = verify_case(ctx, p, "ui export (audio no)", run, audio_destination, "audio_1080", "audio")
            warnings = dig(run.result, ctx.adapters["ops"]["export"]["result"]["fields"]["warnings"]) or []
            p.check("the file's result carries the typed omission warning",
                    any(isinstance(w, dict) and w.get("code") in ctx.adapters["error_codes"]["audio_omitted_warning_codes"] for w in warnings), warnings)
    p.metrics["note"] = ("semantic protocol only (get/set/invoke): proves the dialog reaches scene.export and its refusals, "
                         "not that a person can see or reach the control; header node:16 and the preflight report are not driven here")


PHASES = [
    ("corpus", "data (corpus, expectations, provenance)", phase_corpus),
    ("model_vs_origin", "data (reference model vs origin oracle)", phase_model),
    ("verifier_controls_scene", "harness (reference + defective scene videos; ffmpeg only)", controls_phase(_scene_controls)),
    ("verifier_controls_audio", "harness (audio policy controls; ffmpeg only)", controls_phase(_audio_controls)),
    ("verifier_controls_detail", "harness (4K original vs proxy controls; ffmpeg only)", controls_phase(_detail_controls)),
    ("identity_coverage", "data (the model tells neighbouring frames apart)", phase_identity_coverage),
    ("baseline_render", "tessel-render CLI + Chrome/GPU: scene.render vs corpus (existing renderer)", phase_baseline_render),
    ("baseline_detail", "tessel-render CLI + Chrome/GPU: original 4K PNG drawn 1:1 (existing renderer)", phase_baseline_detail),
    ("baseline_encode", "tessel-render CLI + Chrome/GPU + ffmpeg by the harness (verifier on real pixels)", phase_baseline_encode),
    ("export_main_1080_event", "tessel-render CLI: scene.export 1080p, event + sway", scene_export_phase("main_1080_event")),
    ("export_main_1080_cut", "tessel-render CLI: scene.export 1080p across the shot cut", scene_export_phase("main_1080_cut")),
    ("export_main_4k_cut", "tessel-render CLI: scene.export UHD across the shot cut", scene_export_phase("main_4k_cut")),
    ("export_ntsc_1080", "tessel-render CLI: scene.export 30000/1001", scene_export_phase("ntsc_1080")),
    ("export_detail_4k", "tessel-render CLI: scene.export original 4K PNG, no proxy", scene_export_phase("detail_4k")),
    ("export_invalid_requests", "tessel-render CLI: typed refusals, no file, no mutation", phase_export_invalid),
    ("export_destination_exists", "tessel-render CLI: existing destination survives", phase_export_destination_exists),
    ("export_cancel", "tessel-render CLI: cancel leaves no partial file", phase_export_cancel),
    ("export_snapshot", "tessel-render CLI: immutable snapshot during edit", phase_export_snapshot),
    ("export_audio_policy", "tessel-render CLI: directed audio mixed, refused or explicit limit", phase_export_audio),
    ("export_restored_package", "tessel-render CLI: package restore, origin deleted", phase_export_restored_package),
    ("export_restored_managed", "tessel-render CLI: reopen with managed base, origin deleted", phase_export_restored_managed),
    ("socket_export", "native window socket (isolated instance): scene.export", phase_socket_export),
    ("ui_export_flow", "native UI semantic flow (flow.export_scene dialog; get/set/invoke only)", phase_ui_export_flow),
]


# ---------------------------------------------------------------------------------- report
def runtime_identity():
    dist = ROOT / "runtime" / "dist"
    files = {}
    for name in ("runtime.js", "index.html"):
        path = dist / name
        if path.is_file():
            files[name] = harness.sha256_file(path)
    node = subprocess.run(["node", "--version"], capture_output=True, text=True).stdout.strip()
    package = ROOT / "runtime" / "package.json"
    return {"dist": files, "node": node, "package_json_sha256": harness.sha256_file(package) if package.is_file() else None}


def environment():
    return {"python": platform.python_version(), "platform": platform.platform(), "machine": platform.machine(),
            **v.tool_versions(), "free_gib_tmp": round(free_gib(), 1)}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--phases", default="all")
    parser.add_argument("--bin-headless", default=str(ROOT / "target/debug/tessel-headless"))
    parser.add_argument("--bin-render", default=str(ROOT / "target/debug/tessel-render"))
    parser.add_argument("--bin-editor", default=str(ROOT / "target/release/tessel-editor"))
    parser.add_argument("--gl", default="swiftshader", choices=["swiftshader", "gpu"])
    parser.add_argument("--adapters", default=None)
    parser.add_argument("--work", default=None)
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--corpus", default=None, help="corpus directory; a tampered copy is the negative control (exits 1)")
    parser.add_argument("--gpu-lock-held", action="store_true", help="run under scripts/directed-scene-proof/with-gpu-lock.py")
    parser.add_argument("--report", default=None)
    args = parser.parse_args()
    ids = [phase_id for phase_id, _, _ in PHASES]
    if args.list:
        for phase_id, surface, _ in PHASES:
            print(f"{phase_id:28s} {surface}")
        return 0
    wanted = ids if args.phases == "all" else [x.strip() for x in args.phases.split(",") if x.strip()]
    unknown = [x for x in wanted if x not in ids]
    if unknown:
        print(f"unknown phase(s): {unknown}; see --list", file=sys.stderr)
        return 2
    corpus_dir = pathlib.Path(args.corpus).resolve() if args.corpus else corpus.CORPUS_DIR
    if not (corpus_dir / "expected.json").is_file():
        print("corpus/expected.json missing: run export_corpus.py first", file=sys.stderr)
        return 2
    expected = json.loads((corpus_dir / "expected.json").read_text())
    adapters = json.loads(pathlib.Path(args.adapters or ADAPTERS_PATH).read_text())
    work = Workspace(keep=args.keep, base=args.work)
    ctx = Context(args, work, expected, adapters)
    results, started = [], time.time()
    try:
        for phase_id, surface, run in PHASES:
            if phase_id not in wanted:
                continue
            print(f"== {phase_id} [{surface}]", flush=True)
            phase = Phase(phase_id, surface)
            t0 = time.time()
            try:
                run(ctx, phase)
            except Blocked as blocked:
                phase.status = harness.BLOCKED
                phase.blocked = {"reason": blocked.reason, "needs": blocked.needs, "backlog": blocked.backlog}
                print(f"  BLOCKED {blocked.backlog}: {blocked.reason}", flush=True)
            except Exception as error:  # noqa: BLE001 - any harness/request failure is a FAIL with its text
                phase.status = harness.FAIL
                phase.error = f"{type(error).__name__}: {error}"
                print(f"  FAIL {phase.error}", flush=True)
            phase.seconds = time.time() - t0
            results.append(phase)
            if not work.within_budget():
                print(f"  workspace over its 2 GiB budget ({work.bytes_used()} bytes)", flush=True)
        within = work.within_budget()
        peak_bytes = work.bytes_used()
    finally:
        ctx.close()
        work.close()
    statuses = [r.status for r in results]
    exit_code = 1 if harness.FAIL in statuses else 3 if harness.BLOCKED in statuses else 0
    if not within:
        exit_code = 1
    provenance = json.loads((corpus_dir / "provenance.json").read_text())
    report = {
        "harness": {"name": "directed-scene-export-proof", "version": HARNESS_VERSION,
                    "files": {name: harness.sha256_file(HERE / name) for name in
                              ("proof.py", "export_corpus.py", "export_verify.py", "export_controls.py", "adapters.json")}},
        "git": harness.git_state(),
        "corpus": {"version": expected["corpusVersion"], "dir": str(corpus_dir), "files": provenance["files"],
                   "not_committed": provenance["notCommitted"], "expected_sha256": harness.sha256_file(corpus_dir / "expected.json")},
        "tolerances": expected["tolerances"],
        "binaries": ctx.binaries,
        "runtime_bundle": runtime_identity(),
        "environment": environment(),
        "gl": ctx.gl,
        "adapters": {"file": str(pathlib.Path(args.adapters or ADAPTERS_PATH)), "sha256": harness.sha256_file(pathlib.Path(args.adapters or ADAPTERS_PATH))},
        "seconds": round(time.time() - started, 1),
        "work_budget_respected": within, "work_bytes_at_end": peak_bytes,
        "phases": [r.to_json() for r in results],
        "not_run": [x for x in ids if x not in wanted],
        "not_claimed": NOT_CLAIMED,
        "exit_code": exit_code,
    }
    print("\nSUMMARY")
    for r in results:
        failed = sum(1 for c in r.checks if not c["ok"])
        detail = f"{len(r.checks) - failed}/{len(r.checks)} checks" if r.checks else ""
        extra = f"  [{r.blocked['backlog']}]" if r.blocked else ""
        print(f"  {r.status:8s} {r.id:28s} {detail}{extra}")
    for x in report["not_run"]:
        print(f"  NOT RUN  {x}")
    print(f"exit {exit_code} ({'all selected phases passed' if exit_code == 0 else 'failures' if exit_code == 1 else 'blocked phases: not a pass'})")
    if args.report:
        harness.write_report(args.report, report)
        print(f"report: {args.report}")
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
