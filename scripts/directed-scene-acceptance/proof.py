#!/usr/bin/env python3
"""Independent acceptance of splat, camera, audio, package and export (BL019 ampliação; front S10).

    python3 scripts/directed-scene-acceptance/proof.py --list
    python3 scripts/directed-scene-acceptance/proof.py --phases tolerances_frozen,corpus,controls_camera
    # GPU phases (Chrome/SwiftShader) under the shared bench lock, taken by the wrapper:
    python3 scripts/directed-scene-proof/with-gpu-lock.py python3 scripts/directed-scene-acceptance/proof.py --gpu-lock-held --report /tmp/s10.json

Exit status (0 only when the selected phases passed on a clean, attested run, not merely green):
    0  every selected phase ran and PASSED, the working tree is clean, no `--allow-stale-binary`, and every binary is linked to this
       head (`link.py`). A subset of phases exits 0 too; the report's `acceptance.acceptable` is true only when ALL phases were selected,
       and only that run can be quoted as an acceptance result
    1  at least one FAIL (or the work budget was exceeded). FAIL wins over every other status
    2  usage / prerequisite error of the harness itself (empty or unknown phase list, `--work` is not a directory, disk reserve under
       12 GiB on the volume of `--work`, missing corpus)
    3  no FAIL, but at least one phase is BLOCKED: a dependency (front S7: splat in the directed scene; front S9: the audio mix in
       `scene.export`) is absent, or a binary is not linked to this head. BLOCKED is NOT a pass and never replaces a FAIL
    4  informational, NOT acceptance: every selected phase passed, but the tree is dirty, `--allow-stale-binary` was used, or a binary
       is not linked to this head under that flag. The report says which, under `acceptance`

Groups, and what each proves (docs/features/scene-acceptance.md):
  * data / controls  -> tolerance ancestry and the amendment ledger, corpus determinism, models checked against analytic identities and the
                        origin oracle, and the verifiers *failing* on known-wrong inputs (positive and negative controls, in the PNG
                        and in the codec regime)
  * camera_*         -> the camera as pictures (exact ray-cast masks) through plan, scene.render and scene.export
  * baseline_*       -> the splat model and the audio oracle against the runtime paths that exist today (project scene layer,
                        offline mixer), so the verifiers are trusted before the dependent fronts land
  * audio_off_*, package_* -> `audio:false` preserved; package/restore with the origin deleted; tampered packages refused
  * splat_*, audio_*, full_* -> the fronts S7 and S9: BLOCKED until they exist, then judged by the same verifiers
The window (socket / UI) is not driven here: see docs/features/scene-acceptance.md "Limits". No pointer input, no heard audio.
"""
import argparse
import hashlib
import importlib.util
import json
import math
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


import controls  # noqa: E402
import corpus as corpus_module  # noqa: E402
import ledger  # noqa: E402
import link  # noqa: E402
import oracle  # noqa: E402
import verify  # noqa: E402

harness = _load("tsp_harness", P_DIR / "harness.py")  # read-only reuse: sessions, Phase, binary identity, GPU lock
Phase, Blocked, Abort = harness.Phase, harness.Blocked, harness.Abort
HARNESS_VERSION = 1
MIN_FREE_GIB = 12
WORK_BUDGET_BYTES = 2 * 1024 ** 3
TOL = verify.TOLERANCES
ANALYTIC = TOL["camera"]["matrixAbsolute"]  # double precision: closed-form identities of the models
PRESET_H = {"id": "s10-360p", "name": "S10 640x360", "width": 640, "height": 360}
PRESET_V = {"id": "s10-360v", "name": "S10 360x640", "width": 360, "height": 640}
RENDER_HINT = "./scripts/cargo build -p tessel-editor-render --locked"

NOT_CLAIMED = [
    "heard audio: audio is measured on decoded samples; nothing was listened to and no device is involved",
    "photons, seek-to-photon and physical pointer input: no coordinate clicks, no display measurement",
    "the native window: socket and UI flows of scene.export are R10's (scripts/directed-scene-export-proof); this harness drives the tessel-render process only",
    "other GPUs, platforms and codecs: one machine, SwiftShader GL unless --gl gpu, libx264/aac as found by ffmpeg",
    "partial opacity blending, anisotropic splat scales, spherical harmonics, splat lighting/weather/effects (BL025 beyond the subset), splats occluded by planes",
    "bit-exact frames or samples: pictures are judged by masks, centroids and the tolerances of tolerances.json (each with its justification and sensitivity, none approved by a reviewer); audio by correlation, edges and spectra",
    "a reproducible build: link.py attests that the binary and runtime bundle on disk are the ones recorded for this head's build inputs, by whoever ran it right after the build",
    "the editor's own clock/seek behaviour and playback in a device (BL046)",
]

SEAMS = {
    "S7": {"backlog": "BL025", "signal": "scene.plan on a bundle with `splat` nodes answers `unsupported` with details.domains containing `splat`",
           "needs": "ScenePlan carries the approved splat reference, generation, evaluated matrix and camera; the runtime draws it; preview, headless and export coincide (docs/migration/wave-s1-s10-plan.md, S7)"},
    "S9": {"backlog": "BL016", "signal": "scene.export with the default `audio` answers `unsupported` with details.domains containing `audio`",
           "needs": "mix of the bundle's WAV into the exported file with rational frame/sample mapping, offset and rate conversion; `audio:false` omits with a warning (S9)"},
}


# ---------------------------------------------------------------------------------- environment
def free_gib(path="/tmp"):
    """Free GiB on the volume that holds `path` (the nearest existing ancestor when `path` does not exist yet)."""
    path = pathlib.Path(path)
    while not path.exists() and path != path.parent:
        path = path.parent
    stat = os.statvfs(path)
    return stat.f_bavail * stat.f_frsize / 1024 ** 3


class WorkspaceError(Exception):
    """A prerequisite of the harness itself (not a result about the editor): `main` turns it into exit status 2."""


class Workspace:
    """Own temporary directory: budget 2 GiB, refuses under 12 GiB free on ITS volume, removes only itself.

    `base` (`--work`) is a directory the caller owns and may already fill with their own files: the harness never writes into
    it directly and never removes it. Everything it creates lives in an `s10-acc-*` directory it makes inside `base` (default
    `/tmp`), and `close()` removes exactly that directory. The reserve is read on the volume of `base`, which is where the files
    go (the default `/tmp` is only the volume of the default). `base` that exists and is not a directory is a `WorkspaceError`."""

    def __init__(self, keep=False, base=None):
        self.keep = keep
        self.base = pathlib.Path(base) if base else pathlib.Path("/tmp")
        if self.base.exists() and not self.base.is_dir():
            raise WorkspaceError(f"--work {self.base} exists and is not a directory")
        free = free_gib(self.base)
        if free < MIN_FREE_GIB:  # a prerequisite refusal, not a FAIL of the editor (exit 1)
            raise WorkspaceError(f"{free:.1f} GiB free on the volume of {self.base}, need {MIN_FREE_GIB}")
        try:
            self.base.mkdir(parents=True, exist_ok=True)
            self.path = pathlib.Path(tempfile.mkdtemp(prefix="s10-acc-", dir=self.base))
        except OSError as error:
            raise WorkspaceError(f"cannot create the work directory inside {self.base}: {error}") from error

    scope = None  # the running phase: every directory of a phase lives under its own subtree

    def _base(self):
        base = self.path / self.scope if self.scope else self.path
        base.mkdir(parents=True, exist_ok=True)
        return base

    def sub(self, name):
        """A directory named `name` private to the running phase (the same name in another phase is another directory)."""
        path = self._base() / name
        path.mkdir(parents=True, exist_ok=True)
        return path

    def fresh(self, name):
        """A new, empty directory that nothing else has used: outputs of one request never collide with an earlier request's
        (`destination_exists`), inside the phase or across phases, whether the earlier request passed or failed."""
        return pathlib.Path(tempfile.mkdtemp(prefix=f"{name}-", dir=self._base()))

    def bytes_used(self):
        return harness.tree_bytes(self.path)

    def free_gib(self):
        return free_gib(self.base)

    def within_budget(self):
        return self.bytes_used() <= WORK_BUDGET_BYTES

    def close(self):
        if not self.keep:
            shutil.rmtree(self.path, ignore_errors=True)  # only the `s10-acc-*` directory this object created, never `base`


class Channel:
    """One `tessel-render` JSONL process; `(ok, body)` per request."""

    def __init__(self, session):
        self.session = session

    def __call__(self, request, timeout=60):
        previous, self.session.timeout = self.session.timeout, timeout
        try:
            return self.session(request)
        finally:
            self.session.timeout = previous

    def close(self):
        self.session.close()


class Context:
    def __init__(self, args, work, expected, bundles):
        self.args, self.work, self.exp, self.bundles = args, work, expected, bundles
        self.corpus_dir = pathlib.Path(args.corpus).resolve() if args.corpus else corpus_module.CORPUS_DIR
        self.binaries = {}
        self.informational = []  # why a run that passed every phase is still not an acceptance (exit 4)
        self.gpu_taken = args.gpu_lock_held
        self.cache = {}
        self.palette = expected["palette"]

    def gpu(self):
        if not self.gpu_taken:
            harness.acquire_gpu_lock()
            self.gpu_taken = True

    def binary(self, key="render"):
        path = pathlib.Path(getattr(self.args, f"bin_{key}"))
        if not path.is_file():
            raise Blocked(f"binary not built: {path}", f"build it with the serialized helper: {RENDER_HINT}", "BL019")
        info = harness.binary_info(path)
        linked, reasons, details = link.check(path, key)
        info["link"] = {"holds": linked, "reasons": reasons, **details}
        self.binaries[key] = info
        allowed = getattr(self.args, "allow_stale_binary", False)
        problems = list(reasons)
        if info.get("older_than_sources"):
            problems.append("older than the sources of this checkout")
        if problems and not allowed:
            raise Blocked(f"{path} is not attested for this head ({'; '.join(problems)}): a result would describe a different build",
                          f"rebuild from a clean head ({RENDER_HINT}), run `python3 scripts/directed-scene-acceptance/link.py create --render {path}`, "
                          "or pass --allow-stale-binary (the run is then informational, exit 4, never an acceptance)", "BL019")
        if problems:
            note = f"binary {path} used under --allow-stale-binary although: {'; '.join(problems)}"
            if note not in self.informational:
                self.informational.append(note)
        return path

    def open(self, name, bundle_key, scene="proof", resolve=True, gpu=True, base=None):
        """A fresh `tessel-render` with an empty project and the corpus bundle attached (and resolved from the corpus)."""
        if gpu:
            self.gpu()
        binary = self.binary("render")
        work = self.work.fresh(name)
        channel = Channel(harness.JsonlSession(binary, ["--gl", self.args.gl]))
        try:
            must(channel, {"op": "project.create", "name": name, "dir": str(work / "ws")})
            must(channel, {"op": "scene.attach", "id": scene, "path": str(self.corpus_dir / self.exp["bundles"][bundle_key]["file"])})
            if resolve:
                must(channel, {"op": "scene.resources.resolve", "scene": scene, "base": str(base or self.corpus_dir)}, 120)
        except BaseException:
            channel.close()
            raise
        return channel, work

    def expected_case(self, bundle_key, frame, size):
        """Marker stats and exact masks of any bundle's frame, computed by the oracle (cached)."""
        key = ("case", bundle_key, frame, tuple(size))
        if key not in self.cache:
            bundle = self.bundles[bundle_key]
            state = oracle.frame_state(bundle, frame)
            nodes = oracle.nodes_of(bundle)
            masks = {name: oracle.marker_mask(state, nodes[name], size[0], size[1]) for name in corpus_module.MARKERS}
            markers = {}
            for name, mask in masks.items():
                stats = oracle.mask_stats(mask)
                markers[name] = {"area": 0, "x": None, "y": None} if stats is None else {"area": stats["area"], "x": stats["x"], "y": stats["y"]}
            self.cache[key] = ({"frame": frame, "size": list(size), "markers": markers}, masks)
        return self.cache[key]


def must(op, request, timeout=60):
    ok, body = op(request, timeout)
    if not ok:
        raise Abort(f"{request['op']} failed: {json.dumps(body)[:300]}")
    return body


def sha256_file(path):
    return harness.sha256_file(path)


# ---------------------------------------------------------------------------------- shared checks
def ensure_tools():
    for tool in (verify.FFMPEG, verify.FFPROBE):
        if not pathlib.Path(tool).exists():
            raise Blocked(f"{tool} not found", "install ffmpeg/ffprobe (docs/media-setup.md)", "BL019")


def render_png(ch, scene, frame, size, out, base=None):
    request = {"op": "scene.render", "scene": scene, "frame": frame, "width": size[0], "height": size[1], "out": str(out), "probes": []}
    if base is not None:  # a restored project renders from its own approved resources, never from a folder
        request["resources_root"] = str(base)
    body = must(ch, request, 300)
    return body["frame"]


def rate_of(value):
    if isinstance(value, dict) and value.get("den"):
        return Fraction(int(value["num"]), int(value["den"]))
    if isinstance(value, str):
        return verify.ev.rational(value)
    return None


def result_vs_file(p, result, probe, frames, fps, size, bundle, audio_expected):
    """The export's own report is compared with what was measured in the file: a report that lies fails."""
    videos = [s for s in probe["streams"] if s["codec_type"] == "video"]
    audios = verify.audio_streams(probe)
    p.check("the result reports the frame count the file holds", result.get("frames") == frames, result.get("frames"))
    p.check("the result reports the exact rational rate of the file", rate_of(result.get("fps")) == fps, result.get("fps"))
    p.check("the result reports the output size of the file", (result.get("output", {}).get("width"), result.get("output", {}).get("height")) == tuple(size),
            result.get("output"))
    claimed = result.get("audio", {})
    included = claimed.get("included") if isinstance(claimed, dict) else None
    p.check(f"the result says audio included = {audio_expected}, and the file has {'an' if audios else 'no'} audio stream",
            included is audio_expected and bool(audios) == audio_expected, (claimed, len(audios)))
    reported = {r["asset"]: r["sha256"] for r in result.get("resources", [])}
    declared = {a["assetId"]: a["contentHash"] for a in bundle["assets"] if a["relativePath"].endswith((".png", ".spz"))}
    p.check("the result lists exactly the bundle's image/splat resources with the bundle's own hashes", reported == declared,
            {k: (reported.get(k), v) for k, v in declared.items() if reported.get(k) != v} or sorted(set(reported) ^ set(declared)))
    p.check("the file has exactly one video stream", len(videos) == 1, len(videos))


def judge_video_frames(p, ctx, path, bundle_key, size, first, frames, which, label, kind="camera"):
    """Decodes the exported video (BT.709 limited) and judges the picture of the frames in `which` (edit frames)."""
    wanted = {frame: index for index, frame in enumerate(range(first, first + frames)) if frame in which}
    decoded = {}
    for index, rgb in enumerate(verify.ev.decode(path, size[0], size[1])):
        if index in wanted.values():
            decoded[index] = rgb
    metrics = {}
    for frame, index in sorted(wanted.items()):
        rgb = decoded.get(index)
        if rgb is None:
            p.check(f"{label} frame {frame}: decoded", False, "not decoded")
            continue
        if kind == "camera":
            case, masks = ctx.expected_case(bundle_key, frame, size)
            metrics[frame] = verify.judge_camera_frame(p, rgb, size[0], size[1], masks, case, ctx.palette, video=True, label=f"{label} f{frame} ")
        else:
            case = ctx.exp["splat"]["cases"][str(frame)]
            metrics[frame] = verify.judge_splat_frame(p, rgb, size[0], size[1], case, ctx.palette, video=True, label=f"{label} f{frame} ")
    return metrics


def export_and_verify(p, ctx, ch, scene, bundle_key, destination, frm, frames, preset, audio=None, kind="camera", judge_frames=(), audio_case=None,
                      audio_expected=False, label="export"):
    """Runs `scene.export`, then judges container, report, pictures and (when asked) audio against the oracle."""
    request = {"op": "scene.export", "scene": scene, "preset": preset, "destination": str(destination), "from": frm, "frames": frames}
    if audio is not None:
        request["audio"] = audio
    started = time.monotonic()
    ok, body = ch(request, 900)
    seconds = time.monotonic() - started
    if not ok:
        return ok, body, None
    bundle = ctx.bundles[bundle_key]
    fps = Fraction(bundle["fps"]["numerator"], bundle["fps"]["denominator"])
    size = (preset["width"], preset["height"])
    expect = {"fps": fps, "frames": frames, "width": size[0], "height": size[1]}
    probe, metrics = verify.ev.verify_container(p, destination, expect)
    metrics["seconds"] = round(seconds, 2)
    if probe is None:
        return ok, body, metrics
    result_vs_file(p, body, probe, frames, fps, size, bundle, audio_expected)
    if judge_frames:
        metrics["pictures"] = judge_video_frames(p, ctx, destination, bundle_key, size, frm, frames, set(judge_frames), label, kind)
    if audio_case is not None:
        metrics["audio"] = verify.judge_audio(p, destination, audio_case, bundle, label=f"{label} ", video_seconds=metrics.get("duration_seconds"))
    elif audio_expected is False:
        p.check(f"{label}: no audio stream, as asked (`audio:false`)", not verify.audio_streams(probe), len(verify.audio_streams(probe)))
    return ok, body, metrics


# ---------------------------------------------------------------------------------- git / tolerances
def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True).stdout.strip()


def phase_tolerances_frozen(ctx, p):
    """Bookkeeping, and it says so. It proves: the tolerances file makes no pre-registration claim, the corpus was generated against
    these exact tolerances, every number has a justification and a basis, the amendment ledger is anchored to Git (anchor ancestry,
    baseline hashes, append-only history, every deciding file pinned), and no pinned file differs from the last recorded state.
    It does NOT prove that any number was fixed before a result, nor that a reviewer approved it."""
    path = HERE / "tolerances.json"
    data = json.loads(path.read_text(encoding="utf-8"))
    p.check("tolerances.json is schema s10-tolerances/2", data.get("schema") == "s10-tolerances/2", data.get("schema"))
    p.check("tolerances.json makes no pre-registration or approval claim (no `frozenBeforeResults`, no `frozenFrom`; the provenance block disclaims both)",
            "frozenBeforeResults" not in data and "frozenFrom" not in data and "None of these numbers is claimed" in data.get("provenance", {}).get("claim", ""),
            sorted(data))
    p.check("the corpus was generated against exactly these tolerances (hash recorded in expected.json)",
            ctx.exp["tolerancesSha256"] == sha256_file(path), (ctx.exp["tolerancesSha256"], sha256_file(path)))
    unjustified, stale, malformed = ledger.justification_gaps(data)
    p.check("every number in tolerances.json has a justification with a basis (analytic, inherited or choice) and a text", not unjustified and not malformed,
            {"unjustified": unjustified[:6], "malformed": malformed[:6]})
    p.check("no justification describes a number that is not in tolerances.json (a dead parameter's argument)", not stale, stale[:6])
    rel = str(path.relative_to(ROOT))
    introduced = git("log", "--diff-filter=A", "--format=%H", "--", rel).splitlines()
    if p.check("git knows the commit that introduced tolerances.json", bool(introduced), "not committed yet"):
        first = introduced[-1]
        p.metrics["introduced_in"] = first
        for name in [str(f.relative_to(ROOT)) for f in HERE.glob("results*.json")]:
            earliest = git("log", "--diff-filter=A", "--format=%H", "--", name).splitlines()
            ok = bool(earliest) and subprocess.run(["git", "merge-base", "--is-ancestor", first, earliest[-1]], cwd=ROOT).returncode == 0
            p.check(f"{name}: the commit that introduced the tolerances is an ancestor of the first commit that holds this result (ancestry only: it says nothing about when the numbers were chosen)",
                    ok, earliest[-1:] or "result not committed")
    for name, ok, detail in ledger.verify_ledger():
        p.check(f"amendment ledger: {name}", ok, detail)
    book = ledger.load()
    p.metrics["amendments"] = [a | {"round": r["id"]} for r in book["rounds"] for a in r["amendments"]]
    p.metrics["ledger_baseline"] = book["baseline"]["commit"]
    p.metrics["pinned_sha256"] = ledger.current()
    p.metrics["tolerances_sha256"] = sha256_file(path)


def phase_corpus(ctx, p):
    problems = corpus_module.check(ctx.corpus_dir)
    p.check("committed corpus is byte-identical to what corpus.py writes", not problems, problems)
    provenance = json.loads((ctx.corpus_dir / "provenance.json").read_text(encoding="utf-8"))
    p.check("provenance declares no private media", provenance["private_media"] is False)
    for name, entry in provenance["files"].items():
        p.check(f"{name}: hash matches provenance", sha256_file(ctx.corpus_dir / name) == entry["sha256"])
    for key, entry in ctx.exp["bundles"].items():
        p.check(f"bundle {key}: expected.json pins its bytes", sha256_file(ctx.corpus_dir / entry["file"]) == entry["sha256"])
        bundle = ctx.bundles[key]
        # the same verifier the package controls exercise, called on the bundles the editor will read: exact names, exact hashes. The names
        # come from expected.json (written by corpus.py), not from the bundle under test, so a name missing from the declaration, an unknown
        # or repeated one, or a wrong hash all fail; the hashes are the corpus' own, taken from the pinned bytes
        wanted = {path: ctx.exp["assets"][path] for path in entry["declaredAssets"]}
        verify.judge_declared_hashes(p, bundle, wanted, label=f"bundle {key}: ")
        for asset in bundle["assets"]:
            file_hash = sha256_file(ctx.corpus_dir / asset["relativePath"])
            if file_hash != ctx.exp["assets"].get(asset["relativePath"], {}).get("sha256"):
                p.check(f"bundle {key}: {asset['assetId']} is the file expected.json pins", False, file_hash)
        if "audio" in bundle:
            p.check(f"bundle {key}: sourcePcmHashSha256 is the hash of the whole WAV file (the rule the resource gate enforces)",
                    bundle["audio"]["sourcePcmHashSha256"] == sha256_file(ctx.corpus_dir / bundle["audio"]["mixRelativePath"]))
    p.check("the NTSC audio bundle carries the rational rate 30000/1001", ctx.exp["bundles"]["audio_ntsc"]["fps"] == "30000/1001")
    p.check("the corpus has camera frames in three shots (fov 30/50/75), both orientations, and splats in two shots",
            {c["shot"] for c in ctx.exp["camera"]["cases"].values()} == {"shot-a", "shot-b", "shot-c"}
            and {tuple(c["size"]) for c in ctx.exp["camera"]["cases"].values()} == {(640, 360), (360, 640)}
            and {c["shot"] for c in ctx.exp["splat"]["cases"].values()} == {"shot-a", "shot-b"})
    p.check(f"every splat has at least {TOL['splat']['minScreenSigmaPx']} px of sigma (asserted when the corpus is generated)",
            all(s["sigmaPx"] >= TOL["splat"]["minScreenSigmaPx"] for c in ctx.exp["splat"]["cases"].values() for s in c["splats"]))
    p.check("every splat frame stores a back-splat probe whose three hypotheses are separated by at least the frozen minimum",
            all(c["backProbe"]["separation"] >= TOL["splat"]["backSplatSeparationMinDistance"] for c in ctx.exp["splat"]["cases"].values()),
            {k: c["backProbe"]["separation"] for k, c in ctx.exp["splat"]["cases"].items()})
    p.metrics["corpus"] = {"version": ctx.exp["corpusVersion"], "bundles": {k: e["sha256"] for k, e in ctx.exp["bundles"].items()}}


def phase_models(ctx, p):
    run = subprocess.run([sys.executable, str(P_DIR / "model.spec.py")], capture_output=True, text=True)
    p.check("the camera model agrees with the origin oracle (scripts/directed-scene-proof/model.spec.py, read-only)", run.returncode == 0, run.stderr[-300:])
    camera = {"position": [0, 0.5, 8], "target": [0, 0.5, 0], "fovDeg": 50}
    centre = oracle.project(camera, 640, 360, [0, 0.5, 0])
    p.check("the target projects to the exact centre of the picture", abs(centre[0] - 320) < ANALYTIC and abs(centre[1] - 180) < ANALYTIC and abs(centre[2] - 8) < ANALYTIC, centre)
    top = oracle.project(camera, 640, 360, [0, 0.5 + 8 * math.tan(math.radians(25)), 0])
    p.check("a point at the vertical half-fov projects to the top edge (the fov is vertical)", abs(top[1]) < ANALYTIC, top)
    p.check("a point behind the camera does not project", oracle.project(camera, 640, 360, [0, 0.5, 20]) is None)
    right = oracle.project(camera, 640, 360, [1, 0.5, 0])
    p.check("+x in the world is to the right of the picture", right[0] > 320, right)
    frame = {"camera": camera, "worldMatrices": {"n": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]}}
    shot = oracle.splat_screen(frame, 640, 360, {"id": "n"}, {"position": [0, 0.5, 0], "sigma": 0.3}, (1, 1, 1))
    expect = oracle.quantised_sigma(0.3) * (180 / math.tan(math.radians(25))) / 8
    p.check("a gaussian of sigma 0.3 at depth 8 has the closed-form screen sigma", abs(shot["sigma_px"] - expect) < ANALYTIC, (shot["sigma_px"], expect))
    p.check("the splat centre uses pixel-index coordinates (the picture centre is (319.5, 179.5))", abs(shot["x"] - 319.5) < ANALYTIC and abs(shot["y"] - 179.5) < ANALYTIC, shot)
    import gzip
    import struct
    raw = gzip.decompress(oracle.spz_v2([{"position": (1.5, -2.25, 0.5), "color": (240, 150, 30), "alpha": 1.0, "sigma": 0.3}]))
    magic, version, count, sh, bits = struct.unpack("<IIIBB", raw[:14])
    p.check("the SPZ writer's header is NGSP, version 2, one point, SH degree 0, 12 fractional bits", (magic, version, count, sh, bits) == (0x5053474E, 2, 1, 0, 12),
            (hex(magic), version, count, sh, bits))
    # an independent reader of the payload: fixed-point positions, alpha, quantised colour and scale back from the bytes
    position = [int.from_bytes(raw[16 + 3 * i:19 + 3 * i], "little", signed=True) / 4096 for i in range(3)]
    p.check("positions decode from the 24-bit fixed point within one step", all(abs(a - b) <= 1 / 4096 for a, b in zip(position, (1.5, -2.25, 0.5))), position)
    p.check("alpha 1.0 is stored as 255", raw[25] == 255, raw[25])
    sigma = math.exp(raw[29] / 16 - 10)
    p.check("scale is stored as (ln sigma + 10) * 16 and decodes within one quantisation step", abs(math.log(sigma) - math.log(0.3)) <= 1 / 16, sigma)
    import io
    import wave
    wav = wave.open(io.BytesIO((ctx.corpus_dir / "audio/mix.wav").read_bytes()))
    shape = (wav.getnchannels(), wav.getsampwidth(), wav.getframerate(), wav.getnframes())
    p.check("the mix WAV is 13 s of mono 16-bit at 44.1 kHz", shape == (1, 2, 44100, corpus_module.MIX_SECONDS * 44100), shape)
    for key, bundle in ctx.bundles.items():
        audio = bundle.get("audio")
        if not audio or key == "audio_short":
            continue
        # origin node/render.ts: the source must cover the whole window (AUDIO_SHORT), and the window is the edit's length
        fps_of = Fraction(bundle["fps"]["numerator"], bundle["fps"]["denominator"])
        end = Fraction(audio["editEndSecondsExclusive"]).limit_denominator(100000)
        p.check(f"{key}: the whole WAV covers [0, editEndSecondsExclusive = {float(end)}) (origin AUDIO_SHORT rule)", end <= Fraction(shape[3], shape[2]), (float(end), shape[3] / shape[2]))
        length = end - Fraction(audio["editStartSeconds"]).limit_denominator(1000)
        p.check(f"{key}: the window is exactly the edit's length (origin AUDIO_VIDEO_DURATION rule)",
                abs(length - oracle.frames_to_seconds(bundle["editDurationFrames"], fps_of)) <= Fraction(1, 1000), float(length))
    short = wave.open(io.BytesIO((ctx.corpus_dir / "audio/mix-short.wav").read_bytes()))
    short_end = Fraction(ctx.bundles["audio_short"]["audio"]["editEndSecondsExclusive"]).limit_denominator(1000)
    p.check("the negative-control WAV is shorter than the window its bundle declares (a correct AUDIO_SHORT fixture)",
            Fraction(short.getnframes(), short.getframerate()) < short_end, (short.getnframes() / short.getframerate(), float(short_end)))
    p.check("the negative-control WAV is byte-identical to the pre-amendment mix (history/A1-audio-window)",
            sha256_file(ctx.corpus_dir / "audio/mix-short.wav") == corpus_module.SHORT_MIX_SHA256)
    pcm = struct.unpack("<%dh" % (44100 // 10), wav.readframes(44100 // 10))
    table = controls.table_of(ctx.exp)
    worst = max(abs(pcm[i] / 32768 - table.value(Fraction(i, 44100))) for i in range(len(pcm)))
    p.check("the WAV samples equal the analytic tone table to one 16-bit step (first 0.1 s)", worst <= 1.5 / 32768, worst)
    p.check("tone table (file time): 440 Hz at 1 s, silence at 3.2 s, 880 Hz at 5 s, 1320 Hz at 7 s, silence at 8 s, 440 Hz at 9 s",
            (table.hz_at(Fraction(1)), table.hz_at(Fraction(16, 5)), table.hz_at(Fraction(5)), table.hz_at(Fraction(7)), table.hz_at(Fraction(8)), table.hz_at(Fraction(9)))
            == (440, None, 880, 1320, None, 440))
    ntsc = Fraction(30000, 1001)
    p.check("frame 180 at 30000/1001 is exactly 6.006 s", oracle.frames_to_seconds(180, ntsc) == Fraction(6006, 1000), oracle.frames_to_seconds(180, ntsc))
    p.check("rational window (origin rule): frame 18 at 24 fps with editStartSeconds 1 plays file time 7/4 s",
            oracle.source_time_of(oracle.frames_to_seconds(18, Fraction(24)), 1) == Fraction(7, 4))
    p.check("the origin rule is not the retired assumption A1: file time 1.75 s is heard 0 s into an export of frame 18, not 2 s",
            oracle.export_time_of(Fraction(7, 4), Fraction(24), 1, 18) == 0 and oracle.export_time_of(Fraction(7, 4), Fraction(24), 1, 18) != Fraction(2))


def control_base(name):
    """`negative: fov +5 deg @f12 (video regime)` -> (`fov +5 deg`, regime, frame): the frozen name, the regime and the frame of a control."""
    base = name[len("negative: "):] if name.startswith("negative: ") else name
    regime = "video" if base.endswith(controls.VIDEO_TAG) else "png"
    base = base[:-len(controls.VIDEO_TAG)] if regime == "video" else base
    frame = None
    if " @f" in base:
        base, _, tail = base.rpartition(" @f")
        frame = tail
    return base, regime, frame


def frozen_coverage(domain, frozen, names, frames):
    """Frozen negative controls that no control of `names` exercises, in every regime (and every judged frame for the splat)."""
    seen = {}
    for name in names:
        base, regime, frame = control_base(name)
        for wanted in frozen:
            if base == wanted or base.startswith(wanted + " (") or base.startswith(wanted + ": "):
                seen.setdefault((wanted, regime), set()).add(frame)
    regimes = ("png", "video") if domain in ("camera", "splat") else ("png",)
    missing = []
    for wanted in frozen:
        for regime in regimes:
            got = seen.get((wanted, regime), set())
            if not got or (domain == "splat" and {str(f) for f in frames} - got):
                missing.append(f"{wanted} [{regime}]")
    return missing


def describe_control(result):
    if result["raised"]:
        return f"the verifier raised: {result['raised']}"
    if result["should_pass"]:
        return result["failed_checks"][:3]
    if result["stance"] == "blind":
        return f"declared blind spot no longer holds: {result['named_failed']}"
    return f"no check named {list(result['must_fail'])} failed; failed instead: {result['failed_checks'][:2]}" if result["failed_checks"] else "the verifier accepted a known-wrong input"


def controls_phase(domain, runner):
    def phase(ctx, p):
        ensure_tools()
        work = ctx.work.sub(f"controls-{domain}")
        expected, bundles, corpus_dir = controls.load(ctx.corpus_dir)
        results = runner(expected, bundles, corpus_dir, work)
        for result in results:
            kind = "positive: must pass" if result["should_pass"] else f"negative: must fail `{'`/`'.join(result['must_fail'])}`" + (
                "" if result["stance"] == "detect" else f", stance {result['stance']}")
            p.check(f"{result['control']} ({kind})" if not result["ok"] else result["control"], result["ok"], describe_control(result))
        missing = frozen_coverage(domain, TOL[domain].get("negativeControlsMustFail", []), [r["control"] for r in results if not r["should_pass"]],
                                  expected["splat"]["frames"])
        p.check(f"every frozen negative control of `{domain}` is exercised in every regime" + (" and on every judged frame" if domain == "splat" else ""), not missing, missing[:6])
        negatives = [r for r in results if not r["should_pass"]]
        p.check(f"at least one positive and one negative control ran ({len(results) - len(negatives)} positive, {len(negatives)} negative)",
                0 < len(negatives) < len(results))
        p.metrics["controls"] = [{k: r[k] for k in ("control", "should_pass", "stance", "detected", "ok")} | ({"size_state": r["size_state"]} if "size_state" in r else {})
                                 for r in results]
        # what the controls say a rule can NOT see is published, never folded into a pass count
        p.metrics["blind_spots"] = [r["control"] for r in negatives if r["stance"] == "blind"]
        p.metrics["not_claimed_borderline"] = [r["control"] for r in negatives if r["stance"] == "either"]
    return phase


def _camera(expected, bundles, corpus_dir, work):
    return controls.camera_controls(expected, bundles["camera"], bundles["camera_portrait"])


def _splat(expected, bundles, corpus_dir, work):
    return controls.splat_controls(expected, bundles["splat"])


def _audio(expected, bundles, corpus_dir, work):
    return controls.audio_controls(expected, bundles, work)


def _package(expected, bundles, corpus_dir, work):
    return controls.package_controls(expected, corpus_dir, work)


# ---------------------------------------------------------------------------------- camera
def phase_camera_plan(ctx, p):
    """`scene.plan` poses and node matrices against the model, for every camera frame (3 shots, 3 fovs)."""
    channel, _ = ctx.open("camera-plan", "camera", scene="cam", gpu=False)
    try:
        worst_pose = worst_matrix = 0.0
        for key, case in sorted(ctx.exp["camera"]["cases"].items()):
            if case["size"] != [640, 360]:
                continue
            plan = must(channel, {"op": "scene.plan", "scene": "cam", "frame": case["frame"]})["plan"]
            camera, want = plan["camera"], case["camera"]
            got = [camera["position"][a] for a in "xyz"] + [camera["target"][a] for a in "xyz"] + [camera["fovDeg"]]
            exp = want["position"] + want["target"] + [want["fovDeg"]]
            bad = [(i, g, e) for i, (g, e) in enumerate(zip(got, exp)) if abs(g - e) > TOL["camera"]["poseAbsolute"] + TOL["camera"]["poseRelative"] * abs(e)]
            worst_pose = max([worst_pose] + [abs(g - e) for g, e in zip(got, exp)])
            p.check(f"frame {case['frame']} ({case['shot']}): camera position, target and fov equal the model", not bad, bad[:2])
            nodes = {n["id"]: n for n in plan["nodes"]}
            for name, matrix in case["worldMatrices"].items():
                have = nodes[name]["worldMatrix"]
                diff = max(abs(a - b) for a, b in zip(have, matrix))
                worst_matrix = max(worst_matrix, diff)
                p.check(f"frame {case['frame']}: world matrix of {name} equals the model (<= {TOL['camera']['matrixAbsolute']})", diff <= TOL["camera"]["matrixAbsolute"], diff)
        p.metrics["worst_pose_error"] = worst_pose
        p.metrics["worst_matrix_error"] = worst_matrix
    finally:
        channel.close()


def phase_camera_render(ctx, p):
    ensure_tools()
    metrics = {}
    for bundle_key in ("camera", "camera_portrait"):
        channel, work = ctx.open(f"{bundle_key}-render", bundle_key, scene="cam")
        try:
            for key, case in sorted(ctx.exp["camera"]["cases"].items()):
                if case["bundle"] != bundle_key:
                    continue
                size = tuple(case["size"])
                png = work / f"{key.replace('@', '-')}.png"
                render_png(channel, "cam", case["frame"], size, png, ctx.corpus_dir)
                rgb = verify.png_to_rgb(png, *size)
                _, masks = ctx.expected_case(bundle_key, case["frame"], size)
                metrics[key] = verify.judge_camera_frame(p, rgb, size[0], size[1], masks, case, ctx.palette, label=f"{key} ")
        finally:
            channel.close()
    p.metrics["frames"] = metrics


def phase_camera_export(ctx, p):
    ensure_tools()
    for bundle_key, preset, label in (("camera", PRESET_H, "640x360"), ("camera_portrait", PRESET_V, "360x640")):
        channel, work = ctx.open(f"{bundle_key}-export", bundle_key, scene="cam")
        try:
            frames = [c["frame"] for c in ctx.exp["camera"]["cases"].values() if c["bundle"] == bundle_key]
            ok, body, metrics = export_and_verify(p, ctx, channel, "cam", bundle_key, work / f"{bundle_key}.mp4", 0, 48, preset, audio=None,
                                                  judge_frames=frames, label=label)
            if p.check(f"{label}: scene.export of the camera scene succeeds", ok, body):
                p.metrics[f"export_{label}"] = metrics
        finally:
            channel.close()


# ---------------------------------------------------------------------------------- baselines (runtime paths that exist today)
def _baseline_splat_render(ctx):
    """The project scene layer draws `.splat` files through the same runtime (Spark). Same splats, node positions and camera as
    frame 0 of the directed corpus (s-red, s-green, s-order, s-purple); the s-pair needs a node rotation this path cannot express."""
    if "baseline_splat" in ctx.cache:
        return ctx.cache["baseline_splat"]
    ensure_tools()
    ctx.gpu()
    binary = ctx.binary("render")
    work = ctx.work.sub("baseline-splat")
    camera = ctx.exp["splat"]["cases"]["0"]["camera"]
    wanted = []
    for node, asset, position, _, scale, _ in corpus_module.SPLAT_NODES:
        if node == "s-pair":
            continue
        splats = [{"position": pos, "color": color, "alpha": 1.0, "sigma": sigma} for color, pos, sigma in corpus_module.SPLAT_SPECS[asset]]
        (work / f"{node}.splat").write_bytes(oracle.dot_splat_file(splats))
        wanted.append((node, position, scale))
    background = [c / 255 for c in ctx.palette["background"]] + [1.0]
    (work / "scene.tessel-scene.json").write_text(json.dumps({"format": "tessel-scene", "version": 1, "background": background, "objects": [
        {"type": "splat", "asset": n, "position": list(pos), "scale": sc} for n, pos, sc in wanted]}), encoding="utf-8")
    channel = Channel(harness.JsonlSession(binary, ["--gl", ctx.args.gl]))
    try:
        must(channel, {"op": "project.create", "name": "baseline-splat", "sequence": "main", "rate": 24, "width": 640, "height": 360, "length": 48, "dir": str(work / "ws")})
        for node, _, _ in wanted:
            must(channel, {"op": "asset.add", "kind": "splat", "id": node, "name": node, "path": str(work / f"{node}.splat")})
        must(channel, {"op": "asset.add", "kind": "scene", "id": "scn", "name": "scn", "path": str(work / "scene.tessel-scene.json")})
        must(channel, {"op": "track.add", "id": "t", "name": "T", "kind": "video"})
        must(channel, {"op": "clip.add", "id": "c", "track": "t", "asset": "scn", "start": 0, "duration": 48,
                       "camera": {"position": camera["position"], "target": camera["target"], "fov": camera["fovDeg"], "orbit": 0}})
        out = work / "f0.png"
        must(channel, {"op": "render.frame", "frame": 0, "out": str(out), "probes": []}, 120)
    finally:
        channel.close()
    ctx.cache["baseline_splat"] = (verify.png_to_rgb(out, 640, 360), {n for n, _, _ in wanted})
    return ctx.cache["baseline_splat"]


def phase_baseline_splat_geometry(ctx, p):
    rgb, nodes = _baseline_splat_render(ctx)
    case = ctx.exp["splat"]["cases"]["0"]
    collector = verify.Collector()
    metrics = verify.judge_splat_frame(collector, rgb, 640, 360, case, ctx.palette, label="", only=nodes)
    geometric = ("background", "a splat is drawn", "centroid", "half-intensity radius")
    for name, ok, detail in collector.checks:
        if any(word in name for word in geometric):
            p.check(name, ok, detail)
    p.metrics["geometry"] = metrics


def phase_baseline_splat_colour(ctx, p):
    """Colour is judged separately: the runtime's output colour handling is not the geometry. FINDING: see the doc."""
    rgb, nodes = _baseline_splat_render(ctx)
    case = ctx.exp["splat"]["cases"]["0"]
    collector = verify.Collector()
    verify.judge_splat_frame(collector, rgb, 640, 360, case, ctx.palette, label="", only=nodes)
    diagnosis = {}
    for name, ok, detail in collector.checks:
        if "colour" in name:
            p.check(name, ok, detail)
    for entry in case["splats"]:
        if entry["node"] in nodes and entry["asset"] != "sp-order":
            got = verify.pixel(rgb, 640, int(round(entry["x"])), int(round(entry["y"])))
            stored = entry["color"]
            gamma = [round(255 * (c / 255) ** 2.2, 1) for c in stored]
            diagnosis[entry["node"]] = {"measured": list(got), "stored": stored, "stored_decoded_as_srgb_to_linear": gamma,
                                        "matches_decode_without_reencode": verify.distance(got, gamma) <= 4}
    p.metrics["diagnosis"] = diagnosis


def phase_baseline_audio(ctx, p):
    """The offline mixer (`render.audio`) against the analytic oracle. The reference mixer pans a centred mono source
    with the constant-power law (crates/core/src/audio.rs), -3.01 dB per channel: that is this surface's expected gain."""
    ensure_tools()
    binary = ctx.binary("render")
    work = ctx.work.sub("baseline-audio")
    channel = Channel(harness.JsonlSession(binary, ["--gl", ctx.args.gl]))
    pan_db = 20 * math.log10(math.cos(math.pi / 4))
    try:
        must(channel, {"op": "project.create", "name": "baseline-audio", "sequence": "main", "rate": 24, "width": 640, "height": 360, "length": 288, "dir": str(work / "ws")})
        must(channel, {"op": "asset.add", "kind": "audio", "id": "mix", "name": "mix", "path": str(ctx.corpus_dir / "audio/mix.wav")})
        must(channel, {"op": "track.add", "id": "a", "name": "A", "kind": "audio"})
        # The project layer places a clip on the timeline. To play the file the way the bundle's window does (edit time t -> file
        # `editStartSeconds + t`, origin audio-window.ts) the clip covers the whole edit from frame 0 and starts reading the
        # file `editStartSeconds` in (`source_start`, seconds). The earlier layout (clip at 1 s, file from its first sample)
        # was the retired assumption A1 (amendment A1).
        start = ctx.bundles["audio24"]["audio"]["editStartSeconds"]
        must(channel, {"op": "clip.add", "id": "c", "track": "a", "asset": "mix", "start": 0, "duration": 288, "source_start": int(start)})
        for case_id in ("audio24_window_a", "audio24_window_b"):
            case = ctx.exp["audio"]["cases"][case_id]
            out = work / f"{case_id}.wav"
            body = must(channel, {"op": "render.audio", "from": case["from"], "frames": case["frames"], "out": str(out)}, 120)
            p.check(f"{case_id}: the mixer reports {case['outputSamples']} frames at 48000 Hz", (body.get("frames"), body.get("sample_rate")) == (case["outputSamples"], 48000), body.get("frames"))
            judged = dict(case)
            collector = verify.Collector()
            metrics = verify.judge_audio(collector, out, judged, ctx.bundles[case["bundle"]], label=f"{case_id} ", video_seconds=case["durationSecondsFloat"], gain_db=pan_db)
            for name, ok, detail in collector.checks:
                p.check(name, ok, detail)
            p.metrics[case_id] = metrics
    finally:
        channel.close()


# ---------------------------------------------------------------------------------- audio:false, package
def phase_audio_off(ctx, p):
    """`audio:false` stays what it is: a video-only file, the declared audio listed as omitted, a typed warning."""
    ensure_tools()
    channel, work = ctx.open("audio-off", "audio24", scene="au")
    try:
        destination = work / "off.mp4"
        ok, body, metrics = export_and_verify(p, ctx, channel, "au", "audio24", destination, 18, 72, PRESET_H, audio=False,
                                              judge_frames=[18, 40, 89], audio_expected=False, label="audio:false")
        if not p.check("scene.export with audio:false succeeds", ok, body):
            return
        audio = body.get("audio", {})
        p.check("the result lists the declared audio as omitted and says why", audio.get("declared") == ["audio"] and bool(audio.get("omitted")), audio)
        codes = [w.get("code") for w in body.get("warnings", [])]
        p.check("a typed `audio_omitted` warning is present", "audio_omitted" in codes, codes)
        p.metrics["export"] = metrics
    finally:
        channel.close()


def restore_flow(ctx, p, bundle_key, label, scene="proof"):
    """package the project (origin = the harness's own copy), delete the origin, restore, return (restored folder, archive, work)."""
    ctx.gpu()
    binary = ctx.binary("render")
    work = ctx.work.fresh(label)
    origin = work / "origin"
    shutil.copytree(ctx.corpus_dir, origin)
    archive, workspace, restored = work / "project.zip", work / "ws", work / "restored"
    first = Channel(harness.JsonlSession(binary, ["--gl", ctx.args.gl]))
    try:
        must(first, {"op": "project.create", "name": label, "dir": str(workspace)})
        must(first, {"op": "scene.attach", "id": scene, "path": str(origin / ctx.exp["bundles"][bundle_key]["file"])})
        must(first, {"op": "project.save", "path": str(workspace / f"{label}.tessel.json")})
        must(first, {"op": "scene.resources.adopt", "scene": scene, "source": str(origin)}, 300)
        report = must(first, {"op": "project.package_preflight", "path": str(archive)}, 300)
        if not report.get("complete") or report.get("blockers"):
            raise Blocked(f"package preflight refuses this project: {json.dumps(report)[:200]}", "BL011: the package carries the scene's resources", "BL011")
        must(first, {"op": "project.package", "path": str(archive)}, 300)
        must(first, {"op": "project.verify", "path": str(archive)}, 300)
    finally:
        first.close()
    shutil.rmtree(origin)
    p.check(f"{label}: the origin copy is gone", not origin.exists())
    return archive, restored, work


def package_assets(ctx, bundle_key):
    wanted = {a["relativePath"] for a in ctx.bundles[bundle_key]["assets"]}
    if "audio" in ctx.bundles[bundle_key]:
        wanted.add(ctx.bundles[bundle_key]["audio"]["mixRelativePath"])
    return {path: info for path, info in ctx.exp["assets"].items() if path in wanted}


def phase_package_portability(ctx, p):
    ensure_tools()
    archive, restored, work = restore_flow(ctx, p, "audio24", "pkg-audio24", scene="au")
    metrics = verify.judge_package(p, archive, package_assets(ctx, "audio24"), label="package: ")
    entries = verify.read_package(archive) or {}
    manifest = json.loads(__import__("zipfile").ZipFile(archive).read("manifest.json"))
    declared = {f["path"]: f["sha256"] for f in manifest.get("files", [])}
    wanted = package_assets(ctx, "audio24")
    p.check("the package manifest declares a file entry for every corpus asset (a manifest without `files` proves nothing)",
            bool(declared) and all(any(path.endswith(rel) for path in declared) for rel in wanted), sorted(set(wanted) - {r for r in wanted if any(d.endswith(r) for d in declared)}))
    p.check("the package manifest's declared hashes equal the hashes this harness computed from the entries (an independent reader)",
            bool(declared) and all(entries.get(path) == digest for path, digest in declared.items()), {k: v for k, v in declared.items() if entries.get(k) != v})
    binary = ctx.binary("render")
    second = Channel(harness.JsonlSession(binary, ["--gl", ctx.args.gl]))
    try:
        must(second, {"op": "project.restore", "path": str(archive), "folder": str(restored)}, 300)
        found = {sha256_file(f) for f in restored.rglob("*") if f.is_file()}
        for path, info in package_assets(ctx, "audio24").items():
            p.check(f"restored folder carries {path} byte-identical to the corpus", info["sha256"] in found)
        documents = sorted(restored.rglob("*.tessel.json"))
        if not p.check("the restored folder holds a project document", bool(documents), [str(f) for f in restored.rglob("*")][:8]):
            return
        must(second, {"op": "project.open", "path": str(documents[0])}, 120)
        must(second, {"op": "scene.resources.resolve", "scene": "au"}, 120)  # no base: nothing may need the deleted origin
        for frame in (8, 100):
            case, masks = ctx.expected_case("audio24", frame, (640, 360))
            png = work / f"restored-{frame}.png"
            render_png(second, "au", frame, (640, 360), png)
            rgb = verify.png_to_rgb(png, 640, 360)
            verify.judge_camera_frame(p, rgb, 640, 360, masks, case, ctx.palette, label=f"restored f{frame} ")
        destination = work / "restored-off.mp4"
        ok, body, _ = export_and_verify(p, ctx, second, "au", "audio24", destination, 18, 24, PRESET_H, audio=False, judge_frames=[18, 30], label="restored export")
        p.check("scene.export from the restored project (audio:false) succeeds", ok, body)
        p.metrics["package"] = metrics
    finally:
        second.close()


def phase_package_tamper(ctx, p):
    ensure_tools()
    archive, _, work = restore_flow(ctx, p, "audio24", "pkg-tamper", scene="au")
    binary = ctx.binary("render")
    session = Channel(harness.JsonlSession(binary, ["--gl", ctx.args.gl]))
    victim = "mk-green.png"
    try:
        pristine = work / "pristine-restored"
        ok, body = session({"op": "project.restore", "path": str(archive), "folder": str(pristine)}, 300)
        p.check("control: the untouched package restores (so a refusal below is caused by the tampering)", ok, body)
        cases = [
            ("a byte flipped inside an asset", lambda info, data: bytes([data[0] ^ 1]) + data[1:] if info.filename.endswith(victim) else data),
            ("an asset removed", lambda info, data: None if info.filename.endswith(victim) else data),
            ("the mix replaced by another file of the same name", lambda info, data: b"RIFF" + data[4:-8] + b"\x00" * 8 if info.filename.endswith("mix.wav") else data),
        ]
        for number, (name, change) in enumerate(cases):
            bad = work / f"bad-{number}.zip"
            folder = work / f"bad-{number}-restored"
            verify.tamper_zip(archive, bad, change)
            for operation, request in (("project.verify", {"op": "project.verify", "path": str(bad)}),
                                       ("project.restore", {"op": "project.restore", "path": str(bad), "folder": str(folder)})):
                ok, body = session(request, 300)
                p.check(f"{name}: {operation} is refused with a typed error", (not ok) and bool(body.get("code")), (ok, body if not ok else "accepted"))
            p.check(f"{name}: the refused restore leaves no project folder behind", not folder.exists() or not any(folder.rglob("*")), [str(x) for x in folder.rglob("*")][:4] if folder.exists() else "")
    finally:
        session.close()
    # the third frozen control: a bundle that declares the wrong hash for an asset is refused when its resources are resolved
    bad_dir = work / "bad-hash"
    shutil.copytree(ctx.corpus_dir, bad_dir)
    bundle_path = bad_dir / ctx.exp["bundles"]["audio24"]["file"]
    bundle = json.loads(bundle_path.read_text(encoding="utf-8"))
    for asset in bundle["assets"]:
        if asset["assetId"] == "mk-green":
            asset["contentHash"] = "0" * 64
    bundle_path.write_text(json.dumps(bundle), encoding="utf-8")
    binary = ctx.binary("render")
    channel = Channel(harness.JsonlSession(binary, ["--gl", ctx.args.gl]))
    try:
        must(channel, {"op": "project.create", "name": "bad-hash", "dir": str(work / "bad-hash-ws")})
        ok, body = channel({"op": "scene.attach", "id": "bad", "path": str(bundle_path)}, 60)
        if ok:
            ok, body = channel({"op": "scene.resources.resolve", "scene": "bad", "base": str(bad_dir)}, 120)
        p.check("a bundle declaring a wrong asset hash is refused with a typed error before any resource is approved", (not ok) and bool(body.get("code")),
                body if ok else (ok, "accepted"))
        ok, body = channel({"op": "scene.render", "scene": "bad", "frame": 0, "width": 640, "height": 360, "out": str(work / "bad.png"),
                            "probes": [], "resources_root": str(bad_dir)}, 120)
        p.check("and the scene cannot be rendered from the unapproved bundle", not ok, body if ok else "")
    finally:
        channel.close()


# ---------------------------------------------------------------------------------- dependent fronts (S7 splat, S9 audio)
def require_splat(ctx, ch, scene):
    """BLOCKED while the directed plan refuses the splat domain; any other failure is data for the phase."""
    ok, body = ch({"op": "scene.plan", "scene": scene, "frame": 0}, 120)
    if not ok:
        domains = [d.get("domain") if isinstance(d, dict) else d for d in (body.get("details", {}) or {}).get("domains", [])]
        if body.get("code") == "unsupported" and "splat" in domains:
            raise Blocked(f"scene.plan refuses the splat domain ({body.get('message')})", SEAMS["S7"]["needs"], SEAMS["S7"]["backlog"] + "/S7")
        raise Abort(f"scene.plan failed for a reason other than the missing splat seam: {json.dumps(body)[:300]}")
    return body["plan"]


def phase_splat_plan(ctx, p):
    channel, _ = ctx.open("splat-plan", "splat", scene="sp", gpu=False)
    try:
        plan = require_splat(ctx, channel, "sp")
        for frame in ctx.exp["splat"]["frames"]:
            plan = must(channel, {"op": "scene.plan", "scene": "sp", "frame": frame})["plan"]
            case = ctx.exp["splat"]["cases"][str(frame)]
            camera = plan["camera"]
            got = [camera["position"][a] for a in "xyz"] + [camera["target"][a] for a in "xyz"] + [camera["fovDeg"]]
            want = case["camera"]["position"] + case["camera"]["target"] + [case["camera"]["fovDeg"]]
            p.check(f"frame {frame}: camera equals the model", all(abs(g - w) <= TOL["camera"]["poseAbsolute"] + TOL["camera"]["poseRelative"] * abs(w) for g, w in zip(got, want)), (got, want))
            nodes = {n["id"]: n for n in plan["nodes"]}
            state = oracle.frame_state(ctx.bundles["splat"], frame)
            for node_id, *_ in corpus_module.SPLAT_NODES:
                have = nodes.get(node_id)
                if not p.check(f"frame {frame}: the plan carries splat node {node_id}", have is not None and have.get("kind") == "splat", have):
                    continue
                diff = max(abs(a - b) for a, b in zip(have["worldMatrix"], state["worldMatrices"][node_id]))
                p.check(f"frame {frame}: world matrix of {node_id} equals the model (rotation, translation, scale; <= {TOL['camera']['matrixAbsolute']})", diff <= TOL["camera"]["matrixAbsolute"], diff)
            assets = {n["id"]: n.get("assetId") for n in plan["nodes"]}
            wanted = {node_id: asset for node_id, asset, *_ in corpus_module.SPLAT_NODES}
            p.check(f"frame {frame}: every splat node points at its own splat asset", all(assets.get(k) == v for k, v in wanted.items()),
                    {k: (assets.get(k), v) for k, v in wanted.items() if assets.get(k) != v})
            p.check(f"frame {frame}: the plan is complete (nothing declared and left unevaluated)", plan.get("complete") is True, plan.get("complete"))
    finally:
        channel.close()


def phase_splat_render(ctx, p):
    ensure_tools()
    channel, work = ctx.open("splat-render", "splat", scene="sp")
    try:
        require_splat(ctx, channel, "sp")
        metrics = {}
        for frame in ctx.exp["splat"]["frames"]:
            png = work / f"f{frame}.png"
            render_png(channel, "sp", frame, tuple(ctx.exp["splat"]["renderSize"]), png, ctx.corpus_dir)
            rgb = verify.png_to_rgb(png, *ctx.exp["splat"]["renderSize"])
            metrics[frame] = verify.judge_splat_frame(p, rgb, 640, 360, ctx.exp["splat"]["cases"][str(frame)], ctx.palette, label=f"f{frame} ")
        p.metrics["frames"] = metrics
    finally:
        channel.close()


def phase_splat_export(ctx, p):
    ensure_tools()
    channel, work = ctx.open("splat-export", "splat", scene="sp")
    try:
        require_splat(ctx, channel, "sp")
        ok, body, metrics = export_and_verify(p, ctx, channel, "sp", "splat", work / "splat.mp4", 0, 48, PRESET_H, audio=None, kind="splat",
                                              judge_frames=ctx.exp["splat"]["frames"], label="splat export")
        if not p.check("scene.export of the splat scene succeeds", ok, body):
            return
        p.metrics["export"] = metrics
    finally:
        channel.close()


def phase_splat_package(ctx, p):
    ensure_tools()
    probe, _ = ctx.open("splat-probe", "splat", scene="sp", gpu=False)
    try:
        require_splat(ctx, probe, "sp")
    finally:
        probe.close()
    archive, restored, work = restore_flow(ctx, p, "splat", "pkg-splat", scene="sp")
    verify.judge_package(p, archive, package_assets(ctx, "splat"), label="package: ")
    binary = ctx.binary("render")
    second = Channel(harness.JsonlSession(binary, ["--gl", ctx.args.gl]))
    try:
        must(second, {"op": "project.restore", "path": str(archive), "folder": str(restored)}, 300)
        documents = sorted(restored.rglob("*.tessel.json"))
        if not p.check("the restored folder holds a project document", bool(documents), None):
            return
        must(second, {"op": "project.open", "path": str(documents[0])}, 120)
        must(second, {"op": "scene.resources.resolve", "scene": "sp"}, 120)
        for frame in ctx.exp["splat"]["frames"]:
            png = work / f"restored-{frame}.png"
            render_png(second, "sp", frame, (640, 360), png)
            rgb = verify.png_to_rgb(png, 640, 360)
            verify.judge_splat_frame(p, rgb, 640, 360, ctx.exp["splat"]["cases"][str(frame)], ctx.palette, label=f"restored f{frame} ")
    finally:
        second.close()


def require_audio(ctx, ch, scene, preset=PRESET_H):
    """BLOCKED while the default (`audio` omitted) request is refused for lack of the mix.

    The probe writes into a directory of its own (a successful probe leaves a file; a probe or an earlier phase that failed
    must never make this one answer `destination_exists`, which would be read as a failure of the seam) and removes it."""
    directory = ctx.work.fresh("seam-probe")
    try:
        ok, body = ch({"op": "scene.export", "scene": scene, "preset": preset, "destination": str(directory / "probe.mp4"),
                       "from": 0, "frames": 2}, 300)
    finally:
        shutil.rmtree(directory, ignore_errors=True)
    if not ok:
        domains = (body.get("details", {}) or {}).get("domains", [])
        if body.get("code") == "unsupported" and "audio" in domains:
            raise Blocked(f"scene.export refuses a scene with an audio mix ({body.get('message')})", SEAMS["S9"]["needs"], SEAMS["S9"]["backlog"] + "/S9")
        raise Abort(f"scene.export failed for a reason other than the missing audio seam: {json.dumps(body)[:300]}")
    return body


def audio_export_case(ctx, p, case_id, bundle_key, scene, work, channel):
    case = ctx.exp["audio"]["cases"][case_id]
    destination = work / f"{case_id}.mp4"
    frames_to_judge = sorted({case["from"], case["from"] + case["frames"] // 2, case["from"] + case["frames"] - 1})
    ok, body, metrics = export_and_verify(p, ctx, channel, scene, bundle_key, destination, case["from"], case["frames"], PRESET_H, audio=None,
                                          judge_frames=frames_to_judge, audio_case=case, audio_expected=True, label=case_id)
    p.check(f"{case_id}: scene.export succeeds with the mix", ok, body)
    if ok:
        codes = [w.get("code") for w in body.get("warnings", [])]
        p.check(f"{case_id}: no `audio_omitted` warning when the mix is in the file", "audio_omitted" not in codes, codes)
        p.metrics[case_id] = metrics


def phase_audio_export(ctx, p):
    ensure_tools()
    for bundle_key, case_ids in (("audio24", ("audio24_window_a", "audio24_window_b")), ("audio_ntsc", ("audio_ntsc_window",))):
        channel, work = ctx.open(f"audio-export-{bundle_key}", bundle_key, scene="au")
        try:
            require_audio(ctx, channel, "au")
            for case_id in case_ids:
                audio_export_case(ctx, p, case_id, bundle_key, "au", work, channel)
        finally:
            channel.close()


def phase_audio_package_export(ctx, p):
    ensure_tools()
    probe, _ = ctx.open("audio-probe", "audio24", scene="au")
    try:
        require_audio(ctx, probe, "au")
    finally:
        probe.close()
    archive, restored, work = restore_flow(ctx, p, "audio24", "pkg-audio-export", scene="au")
    verify.judge_package(p, archive, package_assets(ctx, "audio24"), label="package: ")
    binary = ctx.binary("render")
    second = Channel(harness.JsonlSession(binary, ["--gl", ctx.args.gl]))
    try:
        must(second, {"op": "project.restore", "path": str(archive), "folder": str(restored)}, 300)
        documents = sorted(restored.rglob("*.tessel.json"))
        if not p.check("the restored folder holds a project document", bool(documents), None):
            return
        must(second, {"op": "project.open", "path": str(documents[0])}, 120)
        must(second, {"op": "scene.resources.resolve", "scene": "au"}, 120)
        audio_export_case(ctx, p, "audio24_window_a", "audio24", "au", work, second)
    finally:
        second.close()


def phase_audio_short_source(ctx, p):
    """Negative control against the destination: a source WAV shorter than the declared window is refused for EVERY range, not exported.

    Fixture `audio/mix-short.wav` (8 s) under a bundle declaring the window [1, 13). The origin (`node/render.ts`, `AUDIO_SHORT`) refuses
    `duration + 1 ms < editEndSecondsExclusive` before it looks at the range, so a range the file does cover (file seconds 1-3) is refused
    too, and so is one that reads past its end (file seconds 5-9). Each must answer `audio_source_short`, write no file and leave nothing
    behind. The seam is detected on a VALID bundle first (the short one is refused whether or not the mix is supported, so it cannot tell
    a missing S9 from a working one)."""
    ensure_tools()
    case = ctx.exp["audio"]["shortSource"]
    probe, _ = ctx.open("audio-short-seam", "audio24", scene="au")
    try:
        require_audio(ctx, probe, "au")  # BLOCKED while the seam is absent
    finally:
        probe.close()
    channel, work = ctx.open("audio-short-source", case["bundle"], scene="sh")
    try:
        metrics = []
        for number, entry in enumerate(case["ranges"]):
            label = entry["name"]
            destination = work / f"short-{number}.mp4"
            ok, body = channel({"op": "scene.export", "scene": "sh", "preset": PRESET_H, "destination": str(destination),
                                "from": entry["from"], "frames": entry["frames"]}, 300)
            p.check(f"{label}: scene.export is refused", not ok, body if ok else "")
            code = body.get("code") if isinstance(body, dict) else None
            p.check(f"{label}: the refusal is the typed `{case['expectedCode']}` (not a generic or unrelated failure)", (not ok) and code == case["expectedCode"], code)
            details = (body.get("details") if isinstance(body, dict) else None) or {}
            p.check(f"{label}: the refusal comes from the declared-window rule, which does not look at the range (details.rule = window)",
                    isinstance(details, dict) and details.get("rule") == "window", details)
            message = (body.get("message") if isinstance(body, dict) else "") or ""
            p.check(f"{label}: the refusal names the {case['fileFrames']} frames the file holds and the {case['windowEndSeconds']:.3f} s the window ends at",
                    str(case["fileFrames"]) in message and f"{case['windowEndSeconds']:.3f}" in message, message[:200])
            p.check(f"{label}: no file is created at the destination", not destination.exists(), str(destination))
            leftovers = sorted(str(x.relative_to(work)) for x in work.rglob("*") if x.is_file() and x.suffix in (".mp4", ".wav", ".m4a", ".part")
                               and "ws" not in x.relative_to(work).parts)
            p.check(f"{label}: nothing else is left (no partial output, no staged mix)", not leftovers, leftovers)
            metrics.append({"range": label, "code": code, "message": message[:200]})
        p.metrics["short_source"] = {"file_seconds": case["fileSeconds"], "file_frames": case["fileFrames"], "window_end_seconds": case["windowEndSeconds"], "ranges": metrics}
    finally:
        channel.close()


def phase_full_av(ctx, p):
    """Splat scene and audio mix together: plan, export (picture and sound) and package. Needs both fronts."""
    ensure_tools()
    channel, work = ctx.open("full-av", "full", scene="full")
    try:
        require_splat(ctx, channel, "full")
        require_audio(ctx, channel, "full")
        case = ctx.exp["audio"]["cases"]["full_av"]
        ok, body, metrics = export_and_verify(p, ctx, channel, "full", "full", work / "full.mp4", 0, 48, PRESET_H, audio=None, kind="splat",
                                              judge_frames=ctx.exp["splat"]["frames"], audio_case=case, audio_expected=True, label="full")
        p.check("scene.export of the splat + audio scene succeeds", ok, body)
        p.metrics["export"] = metrics
    finally:
        channel.close()
    archive, restored, work = restore_flow(ctx, p, "full", "pkg-full", scene="full")
    verify.judge_package(p, archive, package_assets(ctx, "full"), label="package: ")
    # the same scene, from the restored project with the origin deleted: splat pictures AND the mix in one export, judged again
    second = Channel(harness.JsonlSession(ctx.binary("render"), ["--gl", ctx.args.gl]))
    try:
        must(second, {"op": "project.restore", "path": str(archive), "folder": str(restored)}, 300)
        documents = sorted(restored.rglob("*.tessel.json"))
        if not p.check("the restored folder holds a project document", bool(documents), None):
            return
        must(second, {"op": "project.open", "path": str(documents[0])}, 120)
        must(second, {"op": "scene.resources.resolve", "scene": "full"}, 120)  # no base: nothing may need the deleted origin
        ok, body, metrics = export_and_verify(p, ctx, second, "full", "full", work / "restored-full.mp4", 0, 48, PRESET_H, audio=None, kind="splat",
                                              judge_frames=ctx.exp["splat"]["frames"], audio_case=case, audio_expected=True, label="restored full")
        p.check("scene.export of the splat + audio scene from the restored project succeeds", ok, body)
        p.metrics["restored_export"] = metrics
    finally:
        second.close()


PHASES = [
    ("tolerances_frozen", "data (tolerances ancestry and the amendment ledger of the oracle, verifiers, controls and corpus)", phase_tolerances_frozen),
    ("corpus", "data (corpus determinism, provenance, pinned hashes)", phase_corpus),
    ("models", "data (models vs origin oracle and analytic identities)", phase_models),
    ("controls_camera", "harness (camera verifier on right and wrong pictures)", controls_phase("camera", _camera)),
    ("controls_splat", "harness (splat verifier on right and wrong pictures)", controls_phase("splat", _splat)),
    ("controls_audio", "harness (audio verifier on right and wrong files; ffmpeg only)", controls_phase("audio", _audio)),
    ("controls_package", "harness (package reader on right and tampered archives)", controls_phase("package", _package)),
    ("camera_plan", "tessel-render CLI: scene.plan poses and matrices vs the model", phase_camera_plan),
    ("camera_render", "tessel-render CLI + Chrome: scene.render masks vs the model, both orientations", phase_camera_render),
    ("camera_export", "tessel-render CLI: scene.export video frames vs the model", phase_camera_export),
    ("baseline_splat_geometry", "tessel-render CLI + Chrome: existing project-scene splat path, geometry vs the splat model", phase_baseline_splat_geometry),
    ("baseline_splat_colour", "tessel-render CLI + Chrome: existing project-scene splat path, colour vs the stored colour", phase_baseline_splat_colour),
    ("baseline_audio", "tessel-render CLI: existing offline mixer vs the analytic oracle", phase_baseline_audio),
    ("audio_off", "tessel-render CLI: scene.export audio:false preserved", phase_audio_off),
    ("package_portability", "tessel-render CLI: package, independent read, delete origin, restore, render and export", phase_package_portability),
    ("package_tamper", "tessel-render CLI: tampered packages are refused, nothing restored", phase_package_tamper),
    ("splat_plan", "S7 seam: scene.plan with splat nodes vs the model", phase_splat_plan),
    ("splat_render", "S7 seam: scene.render splat pictures vs the model", phase_splat_render),
    ("splat_export", "S7 seam: scene.export splat video vs the model", phase_splat_export),
    ("splat_package", "S7 seam: package/restore with splat assets", phase_splat_package),
    ("audio_export", "S9 seam: scene.export mixed audio vs the analytic oracle (24 fps and 30000/1001)", phase_audio_export),
    ("audio_package_export", "S9 seam: export with the mix from a restored project", phase_audio_package_export),
    ("audio_short_source", "S9 seam: a source shorter than the window is refused (negative control, own fixture)", phase_audio_short_source),
    ("full_av", "S7+S9 seams: splat and audio together, export and package", phase_full_av),
]


# ---------------------------------------------------------------------------------- report
def runtime_identity():
    dist = ROOT / "runtime" / "dist"
    files = {name: sha256_file(dist / name) for name in ("runtime.js", "index.html") if (dist / name).is_file()}
    node = subprocess.run(["node", "--version"], capture_output=True, text=True).stdout.strip()
    package = ROOT / "runtime" / "package.json"
    return {"dist": files, "node": node, "package_json_sha256": sha256_file(package) if package.is_file() else None}


def environment():
    return {"python": platform.python_version(), "platform": platform.platform(), "machine": platform.machine(), **verify.ev.tool_versions(),
            "free_gib_tmp": round(free_gib("/tmp"), 1)}


def run_phase(ctx, phase, run):
    """Runs one phase and settles its status. A FAIL is never overwritten: a phase whose checks failed and whose later step was
    blocked stays FAIL (the blocked step is recorded next to it); BLOCKED only describes a phase that did not fail."""
    try:
        run(ctx, phase)
    except Blocked as blocked:
        phase.blocked = {"reason": blocked.reason, "needs": blocked.needs, "backlog": blocked.backlog}
        if phase.status == harness.FAIL:
            phase.blocked["note"] = "a later step was blocked after checks of this phase had already failed: the phase stays FAIL"
        else:
            phase.status = harness.BLOCKED
        print(f"  BLOCKED {blocked.backlog}: {blocked.reason}", flush=True)
    except Exception as error:  # noqa: BLE001 - any harness/request failure is a FAIL with its text
        phase.status = harness.FAIL
        phase.error = f"{type(error).__name__}: {error}"
        print(f"  FAIL {phase.error}", flush=True)


def exit_code_of(statuses, within_budget=True, stopped_early=False, informational=()):
    """1 if any FAIL (or the budget was exceeded), else 2 if the run stopped on a prerequisite or ran nothing, else 3 if any BLOCKED, else 4 if every phase
    passed but the run is not an acceptance (`informational` reasons), else 0. 0 is the only status that can be quoted as acceptance."""
    if harness.FAIL in statuses or not within_budget:
        return 1
    if stopped_early or not statuses:  # a run that ran nothing proves nothing
        return 2
    if harness.BLOCKED in statuses:
        return 3
    if informational:
        return 4
    return 0


EXIT_TEXT = {0: "the selected phases passed on a clean tree with attested binaries", 1: "failures", 2: "stopped on a prerequisite",
             3: "blocked phases: not a pass", 4: "INFORMATIONAL, not an acceptance (every phase passed, see `acceptance`)"}


def selected_phases(text, ids):
    """`(phase ids, error)`: `all` or a comma list. An empty selection is an error, never an empty green run."""
    if text == "all":
        return list(ids), None
    wanted = [x.strip() for x in text.split(",") if x.strip()]
    if not wanted:
        return [], "no phase selected (--phases is empty): an empty run proves nothing; see --list"
    unknown = [x for x in wanted if x not in ids]
    if unknown:
        return [], f"unknown phase(s): {unknown}; see --list"
    return wanted, None


def acceptance_of(git_state, allow_stale_binary, informational, results, ids, wanted):
    """Why a run that passed may still not be an acceptance. `informational` (exit 4) lists what makes a passing run non-accepting: a dirty
    tree, `--allow-stale-binary`, a binary used without attestation. A subset of phases may exit 0 (the selected ones passed) but is never
    `acceptable`: the report says how many of the phases ran."""
    informational = list(informational)
    if git_state.get("dirty_files"):
        informational.append(f"the working tree is dirty ({git_state['dirty_files']} entries): the report would describe no commit")
    if allow_stale_binary:
        informational.append("--allow-stale-binary was passed")
    reasons = list(informational)
    if set(wanted) != set(ids):
        reasons.append(f"only {len(wanted)} of {len(ids)} phases were selected: nothing outside them is claimed")
    return {"acceptable": not reasons, "reasons": reasons, "informational": informational}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--phases", default="all")
    parser.add_argument("--bin-render", default=str(ROOT / "target/debug/tessel-render"))
    parser.add_argument("--gl", default="swiftshader", choices=["swiftshader", "gpu"])
    parser.add_argument("--work", default=None, help="a directory you own: the harness makes and removes its own s10-acc-* inside it, never touching the rest")
    parser.add_argument("--allow-stale-binary", action="store_true", help="use a tessel-render that is stale or not attested for this head (link.py); the run is then informational (exit 4), never an acceptance")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--corpus", default=None, help="corpus directory; a tampered copy is the negative control (exits 1)")
    parser.add_argument("--gpu-lock-held", action="store_true", help="run under scripts/directed-scene-proof/with-gpu-lock.py")
    parser.add_argument("--report", default=None)
    args = parser.parse_args()
    ids = [phase_id for phase_id, _, _ in PHASES]
    if args.list:
        for phase_id, surface, _ in PHASES:
            print(f"{phase_id:26s} {surface}")
        return 0
    wanted, problem = selected_phases(args.phases, ids)
    if problem:
        print(problem, file=sys.stderr)
        return 2
    corpus_dir = pathlib.Path(args.corpus).resolve() if args.corpus else corpus_module.CORPUS_DIR
    if not (corpus_dir / "expected.json").is_file():
        print("corpus/expected.json missing: run corpus.py first", file=sys.stderr)
        return 2
    expected, bundles, _ = controls.load(corpus_dir)
    try:
        work = Workspace(keep=args.keep, base=args.work)
    except WorkspaceError as error:
        print(f"refusing to start: {error}", file=sys.stderr)
        return 2
    ctx = Context(args, work, expected, bundles)
    results, started, stopped = [], time.time(), None
    disk = {"start_gib": round(work.free_gib(), 2), "min_gib": round(work.free_gib(), 2), "volume_of": str(work.base)}
    try:
        for phase_id, surface, run in PHASES:
            if phase_id not in wanted:
                continue
            free = work.free_gib()
            disk["min_gib"] = min(disk["min_gib"], round(free, 2))
            if free < MIN_FREE_GIB or not work.within_budget():
                stopped = f"stopped before {phase_id}: {free:.1f} GiB free on the volume of {work.base} (reserve {MIN_FREE_GIB}) / work dir {work.bytes_used() / 1024 ** 3:.2f} GiB (budget {WORK_BUDGET_BYTES / 1024 ** 3:.0f})"
                print(f"  {stopped}", flush=True)
                break
            print(f"== {phase_id} [{surface}]", flush=True)
            phase = Phase(phase_id, surface)
            work.scope = phase_id  # outputs of this phase never share a directory with another phase's, passed or failed
            t0 = time.time()
            run_phase(ctx, phase, run)
            phase.seconds = time.time() - t0
            disk["min_gib"] = min(disk["min_gib"], round(work.free_gib(), 2))
            results.append(phase)
        within, peak_bytes = work.within_budget(), work.bytes_used()
        disk["end_gib"] = round(work.free_gib(), 2)
    finally:
        work.close()
    statuses = [r.status for r in results]
    state = harness.git_state()
    acceptance = acceptance_of(state, args.allow_stale_binary, ctx.informational, results, ids, wanted)
    exit_code = exit_code_of(statuses, within, stopped is not None, acceptance["informational"])
    acceptance["acceptable"] = acceptance["acceptable"] and exit_code == 0
    provenance = json.loads((corpus_dir / "provenance.json").read_text(encoding="utf-8"))
    tolerances_path = HERE / "tolerances.json"
    report = {
        "harness": {"name": "directed-scene-acceptance", "version": HARNESS_VERSION,
                    "files": {name: sha256_file(HERE / name) for name in ledger.PINNED} | {"proof.spec.py": sha256_file(HERE / "proof.spec.py"), "amendments.json": sha256_file(ledger.LEDGER)}},
        "git": state,
        "acceptance": acceptance,
        "tolerances": {"sha256": sha256_file(tolerances_path), "introduced_in": (git("log", "--diff-filter=A", "--format=%H", "--", str(tolerances_path.relative_to(ROOT))).splitlines() or [None])[-1],
                       "provenance": json.loads(tolerances_path.read_text(encoding="utf-8"))["provenance"]["claim"]},
        "amendment_ledger": {"sha256": sha256_file(ledger.LEDGER), "pinned_now": ledger.current(), "book": ledger.load()},
        "stopped": stopped, "allow_stale_binary": args.allow_stale_binary,
        "corpus": {"version": expected["corpusVersion"], "dir": str(corpus_dir), "files": provenance["files"],
                   "expected_sha256": sha256_file(corpus_dir / "expected.json")},
        "binaries": ctx.binaries, "runtime_bundle": runtime_identity(), "environment": environment(), "gl": args.gl,
        "disk": disk, "seams": SEAMS, "seconds": round(time.time() - started, 1),
        "work_budget_respected": within, "work_bytes_at_end": peak_bytes,
        "phases": [r.to_json() for r in results], "not_run": [x for x in ids if x not in wanted],
        "not_claimed": NOT_CLAIMED, "exit_code": exit_code,
    }
    print("\nSUMMARY")
    for r in results:
        failed = sum(1 for c in r.checks if not c["ok"])
        detail = f"{len(r.checks) - failed}/{len(r.checks)} checks" if r.checks else ""
        extra = f"  [{r.blocked['backlog']}]" if r.blocked else ""
        print(f"  {r.status:8s} {r.id:26s} {detail}{extra}")
    for x in report["not_run"]:
        print(f"  NOT RUN  {x}")
    for reason in acceptance["reasons"]:
        print(f"  NOT AN ACCEPTANCE: {reason}")
    verdict = "ACCEPTABLE: every phase passed on a clean tree with attested binaries" if exit_code == 0 and acceptance["acceptable"] else EXIT_TEXT[exit_code]
    print(f"exit {exit_code} ({verdict})")
    if args.report:
        harness.write_report(args.report, report)
        print(f"report: {args.report}")
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
