#!/usr/bin/env python3
"""End-to-end proof of the directed-scene integration (BL019, front P).

    python3 scripts/directed-scene-proof/proof.py --list
    python3 scripts/directed-scene-proof/proof.py                       # every phase
    python3 scripts/directed-scene-proof/proof.py --phases corpus,headless_flow
    python3 scripts/directed-scene-proof/proof.py --report /tmp/tsp-report.json

Exit status (never 0 unless every phase that was selected ran and passed):
    0  all selected phases PASS
    1  at least one FAIL
    3  no FAIL, but at least one phase is BLOCKED (an API or prerequisite does not exist yet)
    2  usage/prerequisite error of the harness itself

A BLOCKED phase names the missing operation and its backlog ID. It is not a skip that hides: it
fails the run. Phases not selected with --phases are listed as NOT RUN in the summary and the
report. Surfaces are separate on purpose: the lib is proven by `crates/*/tests/scene_flow*`;
this script proves the `tessel-headless` process, the `tessel-render` process and the native
window socket, each by its own phase. A command is not a pointer: no coordinate input is used
and no physical-pointer or seek-to-photon claim is made.
"""
import argparse
import contextlib
import importlib.util
import json
import pathlib
import shutil
import struct
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import harness as h  # noqa: E402  (needs the sys.path entry above)
from harness import Abort, Blocked, Phase  # noqa: E402

HARNESS_VERSION = 1
CORPUS = h.CORPUS
EXP = json.loads((CORPUS / "expected.json").read_text()) if (CORPUS / "expected.json").is_file() else None
spec = importlib.util.spec_from_file_location("corpus_gen", HERE / "corpus.py")
corpus_gen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(corpus_gen)

NOT_CLAIMED = [
    "physical pointer input (BL043): no coordinate clicks anywhere; gestures are semantic commands",
    "seek-to-photon latency (BL032): rAF/hash timing is not a photon; nothing here measures one",
    "heard audio (BL046): no audio device is involved",
    "other GPUs, platforms, WKWebView-vs-Chrome pixel identity (BL054): pixels are compared to the model with a fixed tolerance, per surface",
    "domains the plan refuses (light, weather, effects, rigs, paths, data maps, splat): the corpus does not use them",
]


# ---------------------------------------------------------------------------------- shared steps
def must(op, request):
    ok, body = op(request)
    if not ok:
        raise Abort(f"{request['op']} failed: {json.dumps(body)[:300]}")
    return body


def history(op):
    body = must(op, {"op": "history.status"})
    return {"undo_depth": body["history"]["undo_depth"], "redo_depth": body["history"]["redo_depth"],
            "revision": body["project"]["revision"]}


def matrices(evaluate_body):
    return {n["nodeId"]: n["worldMatrix"] for n in evaluate_body["nodes"]}


def canonical(value):
    return json.dumps(value, sort_keys=True)


def flatten(tree):
    found = {}

    def walk(entry):
        found[entry["path"]] = entry
        for child in entry.get("children", []):
            walk(child)
    walk(tree)
    return found


def evaluate(op, scene, frame, gesture=None):
    request = {"op": "scene.evaluate", "scene": scene, "frame": frame}
    if gesture:
        request["gesture"] = gesture
    return must(op, request)


def check_frame(p, label, body, want):
    tol = EXP["tolerances"]
    camera = body["camera"]
    p.check(f"{label}: camera position/target/fov match the model",
            h.vec_close(camera["position"], want["camera"]["position"], tol["camera"])
            and h.vec_close(camera["target"], want["camera"]["target"], tol["camera"])
            and h.close(camera["fovDeg"], want["camera"]["fovDeg"], tol["camera"]["absolute"], tol["camera"]["relative"]),
            f"{camera} vs {want['camera']}")
    got = matrices(body)
    bad = [node for node, matrix in want["worldMatrices"].items()
           if node not in got or not h.matrix_close(got[node], matrix, tol["worldMatrix"])]
    p.check(f"{label}: world matrices of all {len(want['worldMatrices'])} nodes match the model", not bad, bad)
    p.check(f"{label}: shot and instance frame", body["shotId"] == want["shotId"] and body["instanceFrame"] == want["instanceFrame"],
            (body["shotId"], body["instanceFrame"]))


def hero_matrix(body):
    return matrices(body)["hero"]


def run_scene_flow(p, op, reopen, workspace, label):
    """The same flow for every surface that exposes the shared vocabulary: attach, tree, selection,
    out-of-order frame queries, a cancelled gesture, a committed gesture with preview, a property edit,
    rejected input, undo/redo, save, and a reload from disk. `reopen(document)` returns an op callable
    that has the saved document loaded from disk (fresh process, or fresh project in the same window)."""
    tol = EXP["tolerances"]
    edit, scene = EXP["edit"], EXP["edit"]["scene"]
    gesture = edit["gesture"]
    frames = {f["editFrame"]: f for f in EXP["frames"]}

    must(op, {"op": "project.create", "name": f"proof-p-{label}", "dir": str(workspace)})
    attached = must(op, {"op": "scene.attach", "id": scene, "path": str(CORPUS / "bundle.json")})
    p.check("attach: counts match the corpus",
            attached["counts"] == {"definitions": 1, "nodes": 6, "instances": 1, "shots": 2, "camera_clips": 1,
                                   "performance_tracks": 1, "assets": 3}, attached["counts"])
    p.check("attach: no deferred family in use and no notice", attached["deferred_in_use"] == [] and attached["notices"] == [])
    attach_state = history(op)
    p.check("attach is one undo entry", attach_state["undo_depth"] == 1, attach_state)

    tree = flatten(must(op, {"op": "scene.tree", "scene": scene, "depth": 8})["tree"])
    p.check("tree: every node is addressable", all(f"scene/scene-a/node/{n}" in tree for n in ("root", "sky", "mid", "stage", "hero", "echo")))
    p.check("tree: assets, shots and the camera clip are addressable",
            all(k in tree for k in ("asset/sky", "asset/mid", "asset/hero", "shot/shot-1", "shot/shot-2", "camera/cam-main")))
    p.check("tree: hero offers its position as editable", gesture["property"] in tree[gesture["target"]]["editable"])

    seen = {}
    for frame in EXP["seekOrder"]:
        body = evaluate(op, scene, frame)
        check_frame(p, f"frame {frame} (out of order)", body, frames[frame])
        if frame in seen:
            p.check(f"frame {frame} asked again is answered identically", canonical(body) == seen[frame])
        seen[frame] = canonical(body)
    camera_only = must(op, {"op": "scene.camera", "scene": scene, "frame": 48})
    p.check("scene.camera agrees with scene.evaluate on the shot cut frame",
            canonical(camera_only["camera"]) == canonical(json.loads(seen[48])["camera"]))

    must(op, {"op": "scene.select", "scene": scene, "entities": edit["select"]})
    selected = must(op, {"op": "scene.selection"})
    p.check("selection holds the chosen entity", edit["select"][0] in canonical(selected), selected)
    p.check("selection adds no undo and no revision", history(op) == attach_state)

    # A cancelled gesture leaves nothing.
    base_value = must(op, {"op": "scene.get", "scene": scene, "property": gesture["property"]})["value"]
    begin = must(op, {"op": "scene.gesture.begin", "scene": scene, "targets": [gesture["target"]]})
    must(op, {"op": "scene.gesture.update", "gesture": begin["gesture"], "ops": [{"set": gesture["property"], "value": gesture["updates"][0]}]})
    must(op, {"op": "scene.gesture.cancel", "gesture": begin["gesture"]})
    p.check("cancel: history and value unchanged",
            history(op) == attach_state and must(op, {"op": "scene.get", "scene": scene, "property": gesture["property"]})["value"] == base_value)

    # A committed gesture: preview is separate from the document, commit is one undo entry.
    begin = must(op, {"op": "scene.gesture.begin", "scene": scene, "targets": [gesture["target"]]})
    gid = begin["gesture"]
    must(op, {"op": "scene.gesture.update", "gesture": gid, "ops": [{"set": gesture["property"], "value": gesture["updates"][0]}]})
    p.check("update: no undo entry and no revision yet", history(op) == attach_state)
    p.check("update: the document still holds the base value",
            must(op, {"op": "scene.get", "scene": scene, "property": gesture["property"]})["value"] == base_value)
    for frame in gesture["probeFrames"]:
        preview = evaluate(op, scene, frame, gid)
        p.check(f"frame {frame}: preview evaluates the candidate",
                preview.get("source") == "gesture" and h.matrix_close(hero_matrix(preview), gesture["candidateAfterFirstUpdate"][str(frame)], tol["worldMatrix"]))
        p.check(f"frame {frame}: the document is not the candidate",
                h.matrix_close(hero_matrix(evaluate(op, scene, frame)), frames[frame]["worldMatrices"]["hero"], tol["worldMatrix"]))
    must(op, {"op": "scene.gesture.update", "gesture": gid, "ops": [{"set": gesture["property"], "value": gesture["updates"][1]}]})
    must(op, {"op": "scene.gesture.commit", "gesture": gid})
    after_gesture = history(op)
    p.check("commit: exactly one undo entry", after_gesture["undo_depth"] == attach_state["undo_depth"] + 1, after_gesture)
    stored = must(op, {"op": "scene.get", "scene": scene, "property": gesture["property"]})["value"]
    p.check("commit: the value is stored as sent", all(h.close(stored[k], gesture["finalValue"][k], tol["gestureEdit"]["absolute"], 0) for k in "xyz"), stored)
    for frame in gesture["probeFrames"]:
        body = evaluate(op, scene, frame)
        p.check(f"frame {frame}: committed hero matrix matches the model", h.matrix_close(hero_matrix(body), gesture["afterCommit"][str(frame)], tol["worldMatrix"]))
        p.check(f"frame {frame}: an untouched node did not move",
                h.matrix_close(matrices(body)[gesture["untouchedNode"]], frames[frame]["worldMatrices"][gesture["untouchedNode"]], tol["worldMatrix"]))

    # A property edit through the batch API: dry-run first.
    fov = edit["fov"]
    dry = must(op, {"op": "scene.edit", "scene": scene, "dry_run": True, "ops": [{"set": fov["property"], "value": fov["value"]}]})
    p.check("dry-run changes nothing", dry.get("changed") is False and history(op) == after_gesture, dry)
    must(op, {"op": "scene.edit", "scene": scene, "ops": [{"set": fov["property"], "value": fov["value"]}]})
    after_fov = history(op)
    p.check("edit: one more undo entry", after_fov["undo_depth"] == after_gesture["undo_depth"] + 1, after_fov)
    for frame in (0, 60):
        camera = evaluate(op, scene, frame)["camera"]
        p.check(f"frame {frame}: fov edited", h.close(camera["fovDeg"], fov["afterEdit"][str(frame)], 1e-9, 1e-12), camera["fovDeg"])
    moved = evaluate(op, scene, 60)["camera"]["position"]
    p.check("frame 60: the camera path is the model's path with the new fov", h.vec_close(moved, fov["camera"]["60"]["position"], tol["camera"]))

    # Rejected input leaves everything exactly as it was.
    before = (history(op), canonical(evaluate(op, scene, 0)))
    refused, refused_body = op({"op": "scene.edit", "scene": scene, "ops": [{"set": fov["property"], "value": 200}]})
    p.check("an out-of-range fov is refused with a typed error", (not refused) and bool(refused_body.get("code")), refused_body)
    unknown, unknown_body = op({"op": "scene.get", "scene": "nope", "property": gesture["property"]})
    p.check("an unknown scene is refused with a typed error", (not unknown) and bool(unknown_body.get("code")), unknown_body)
    p.check("rejections leave history and frame untouched", (history(op), canonical(evaluate(op, scene, 0))) == before)
    p.metrics.setdefault(label, {})["refusal_codes"] = {"fov_out_of_range": refused_body.get("code"), "unknown_scene": unknown_body.get("code")}

    # Undo/redo walk the three steps one by one.
    must(op, {"op": "history.undo"})
    p.check("undo 1 restores the original fov", h.close(evaluate(op, scene, 60)["camera"]["fovDeg"], frames[60]["camera"]["fovDeg"], 1e-9, 1e-12))
    p.check("undo 1 keeps the committed gesture", h.matrix_close(hero_matrix(evaluate(op, scene, 20)), gesture["afterCommit"]["20"], tol["worldMatrix"]))
    must(op, {"op": "history.undo"})
    body = evaluate(op, scene, 20)
    p.check("undo 2 undoes the whole gesture at once", h.matrix_close(hero_matrix(body), frames[20]["worldMatrices"]["hero"], tol["worldMatrix"]))
    must(op, {"op": "history.redo"})
    must(op, {"op": "history.redo"})
    p.check("redo twice returns to the edited state",
            h.matrix_close(hero_matrix(evaluate(op, scene, 20)), gesture["afterCommit"]["20"], tol["worldMatrix"])
            and h.close(evaluate(op, scene, 60)["camera"]["fovDeg"], fov["value"], 1e-9, 1e-12))

    # Save and reload from disk.
    document = pathlib.Path(workspace) / "proof.tessel.json"
    saved = must(op, {"op": "project.save", "path": str(document)})
    stored_doc = json.loads(document.read_text())
    item = stored_doc["scenes"]["items"][0]
    hero_node = next(n for n in item["bundle"]["sceneDefinitions"][0]["nodes"] if n["id"] == "hero")
    p.check("document: schema 2 with the scene block", stored_doc.get("schema_version") == 2 and item["id"] == scene)
    p.check("document: carries the committed edits",
            hero_node["localPosition"] == gesture["finalValue"] and item["bundle"]["cameraClips"][0]["initialFovDeg"] == fov["value"])
    p.check("document: assets are named by assetId and relative path, never absolute",
            all(not a["relativePath"].startswith("/") for a in item["bundle"]["assets"]))
    p.metrics.setdefault(label, {})["document_bytes"] = saved.get("bytes")

    reopened = reopen(document)
    listing = must(reopened, {"op": "scene.list"})
    p.check("reopen: the scene is back by its ID", scene in canonical(listing), listing)
    for frame in gesture["probeFrames"]:
        p.check(f"reopen frame {frame}: hero matrix is the committed one",
                h.matrix_close(hero_matrix(evaluate(reopened, scene, frame)), gesture["afterCommit"][str(frame)], tol["worldMatrix"]))
    p.check("reopen: the edited fov persisted", h.close(evaluate(reopened, scene, 0)["camera"]["fovDeg"], fov["value"], 1e-9, 1e-12))
    second = pathlib.Path(workspace) / "proof-again.tessel.json"
    must(reopened, {"op": "project.save", "path": str(second)})
    p.check("reopen then save: the scene block is identical", json.loads(second.read_text())["scenes"] == stored_doc["scenes"])
    p.metrics.setdefault(label, {})["history_after_reopen"] = history(reopened)


# ---------------------------------------------------------------------------------- phases
def require_binary(ctx, key, hint):
    path = ctx.bins[key]
    if not pathlib.Path(path).is_file():
        raise Blocked(f"binary not built: {path}", f"build it with the coordinator's serialized helper, e.g. {hint}", "BL019")
    ctx.binaries[key] = h.binary_info(path)
    return path


def phase_corpus(ctx, p):
    problems = corpus_gen.check(CORPUS)
    p.check("committed corpus is byte-identical to what corpus.py writes", not problems, problems)
    p.check("generation is deterministic", corpus_gen.generate() == corpus_gen.generate())
    provenance = json.loads((CORPUS / "provenance.json").read_text())
    p.check("provenance declares no private media", provenance["private_media"] is False)
    bundle = json.loads((CORPUS / "bundle.json").read_text())
    swap = json.loads((CORPUS / "bundle.swap.json").read_text())
    for label, doc in (("bundle", bundle), ("swap bundle", swap)):
        for asset in doc["assets"]:
            path = CORPUS / asset["relativePath"]
            data = path.read_bytes()
            width, height = struct.unpack(">II", data[16:24])
            p.check(f"{label}: {asset['assetId']} hash and size are real",
                    h.sha256_file(path) == asset["contentHash"] and (width, height) == (asset["widthPx"], asset["heightPx"]),
                    (h.sha256_file(path), width, height))
    p.check("the swap changes content, revision and path but not the assetId",
            swap["assets"][2]["assetId"] == "hero" and swap["assets"][2]["contentHash"] != bundle["assets"][2]["contentHash"]
            and swap["assets"][2]["revisionId"] != bundle["assets"][2]["revisionId"]
            and swap["assets"][2]["relativePath"] != bundle["assets"][2]["relativePath"])
    p.check("expected.json pins the bundle bytes", h.sha256_file(CORPUS / "bundle.json") == EXP["bundle"]["sha256"])
    p.check("a frame set exists for every probe class (boundaries, shot cut, out-of-order)",
            {0, 12, 36, 47, 48, 95} <= {f["editFrame"] for f in EXP["frames"]} and set(EXP["seekOrder"]) <= {f["editFrame"] for f in EXP["frames"]})
    p.check("every probe frame has probes on all four layers",
            all({q["layer"] for q in f["pixelProbes"]} == {"sky", "mid", "echo", "hero"} for f in EXP["frames"]))
    p.metrics["corpus"] = {"version": EXP["corpusVersion"], "bundle_sha256": EXP["bundle"]["sha256"],
                           "probe_pixels": sum(len(f["pixelProbes"]) for f in EXP["frames"]), "frames": len(EXP["frames"])}


def phase_model(ctx, p):
    run = subprocess.run([sys.executable, str(HERE / "model.spec.py")], capture_output=True, text=True)
    p.check("the reference model agrees with the origin oracle (model.spec.py)", run.returncode == 0, run.stderr[-400:])
    p.metrics["model_spec"] = run.stderr.strip().splitlines()[-1] if run.stderr.strip() else ""


def phase_headless_flow(ctx, p):
    binary = require_binary(ctx, "headless", "python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py build -p tessel-editor-core --bin tessel-headless --locked")
    work = ctx.work.sub("headless")
    session = h.JsonlSession(binary, ["--empty"])
    opened = []

    def reopen(document):
        fresh = h.JsonlSession(binary, ["--empty"])
        opened.append(fresh)
        must(fresh, {"op": "project.open", "path": str(document)})
        return fresh
    try:
        run_scene_flow(p, session, reopen, work / "ws", "headless")
    finally:
        for s in [session, *opened]:
            s.close()
    p.metrics["binary"] = ctx.binaries["headless"]


def phase_scene_plan(ctx, p):
    binary = require_binary(ctx, "headless", "see headless_flow")
    iface = ctx.interfaces["plan"]
    session = h.JsonlSession(binary, ["--empty"])
    try:
        must(session, {"op": "project.create", "name": "plan", "dir": str(ctx.work.sub("plan") / "ws")})
        must(session, {"op": "scene.attach", "id": "proof", "path": str(CORPUS / "bundle.json")})
        tol = EXP["tolerances"]
        for want in EXP["frames"]:
            ok, body = h.probe_interface(session, iface, scene="proof", frame=want["editFrame"])
            if not p.check(f"frame {want['editFrame']}: scene.plan answers", ok, body):
                continue
            plan = h.dig(body, iface["result_path"])
            nodes = {n["id"]: n["worldMatrix"] for n in plan["nodes"]}
            p.check(f"frame {want['editFrame']}: plan matrices match the model",
                    all(h.matrix_close(nodes[k], m, tol["worldMatrix"]) for k, m in want["worldMatrices"].items()))
            p.check(f"frame {want['editFrame']}: layers are in the model's paint order",
                    [layer["nodeId"] for layer in plan["layers"]] == want["paintOrder"])
            p.check(f"frame {want['editFrame']}: camera matches the model", h.vec_close(plan["camera"]["position"], want["camera"]["position"], tol["camera"]))
            resources = {r["assetId"]: r for r in plan["resources"]}
            p.check(f"frame {want['editFrame']}: required resources are the corpus' assets by assetId, with their users",
                    sorted(resources) == EXP["resources"]["requiredAssetIds"]
                    and all(sorted(resources[a]["usedBy"]) == users for a, users in EXP["resources"]["usedBy"].items()))
            p.check(f"frame {want['editFrame']}: resource identity carries the corpus hash",
                    all(resources[a]["contentHash"] == EXP["assets"][a]["sha256"] for a in resources))
    finally:
        session.close()


def phase_package(ctx, p):
    h.acquire_gpu_lock()
    binary = require_binary(ctx, "render", "python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py build -p tessel-editor-render --locked")
    work = ctx.work.sub("package")
    origin = work / "origin"
    shutil.copytree(CORPUS, origin)
    session = h.JsonlSession(binary)
    try:
        must(session, {"op": "project.create", "name": "pkg", "dir": str(work / "ws")})
        must(session, {"op": "scene.attach", "id": "proof", "path": str(origin / "bundle.json")})
        must(session, {"op": "project.save", "path": str(work / "ws/pkg.tessel.json")})
        must(session, {"op": "scene.resources.adopt", "scene": "proof", "source": str(origin)})
        report = must(session, {"op": "project.package_preflight", "path": str(work / "out.zip")})
        p.metrics["preflight"] = {k: report.get(k) for k in ("ok", "complete", "missing", "blockers")}
        if not report.get("complete") or any(b.get("code") == "unsupported" for b in report.get("blockers", [])):
            raise Blocked("package preflight refuses a project with a directed scene: its resource files are `not_packaged` "
                          f"({json.dumps(report.get('missing'))[:200]})",
                          "BL011: the package carries the scene's resources by assetId from an explicitly registered origin base, "
                          "and restore reopens the project with them under its own root", "BL011")
        package = must(session, {"op": "project.package", "path": str(work / "out.zip")})
        p.check("package is complete", package.get("complete") is True, package)
        must(session, {"op": "project.verify", "path": str(work / "out.zip")})
        shutil.rmtree(origin)  # own temporary copy: from here nothing may need the origin
        restored = work / "restored"
        must(session, {"op": "project.restore", "path": str(work / "out.zip"), "folder": str(restored)})
        found = {h.sha256_file(f): f for f in restored.rglob("*") if f.is_file()}
        for asset_id, asset in EXP["assets"].items():
            p.check(f"restored root carries {asset_id} with the corpus hash", asset["sha256"] in found, asset["sha256"][:12])
        p.check("restored project still lists the scene", "proof" in canonical(must(session, {"op": "scene.list"})))
        body = evaluate(session, "proof", 20)
        p.check("restored scene evaluates like the original", h.matrix_close(hero_matrix(body), next(f for f in EXP["frames"] if f["editFrame"] == 20)["worldMatrices"]["hero"], EXP["tolerances"]["worldMatrix"]))
        must(session, {"op": "scene.resources.resolve", "scene": "proof"})
        want = next(f for f in EXP["frames"] if f["editFrame"] == 20)
        rendered = must(session, {"op": "scene.render", "scene": "proof", "frame": 20,
            "width": EXP["output"]["width"], "height": EXP["output"]["height"],
            "probes": [[q["x"], q["y"]] for q in want["pixelProbes"]]})
        p.check("restored scene renders without origin and matches independent pixels",
            not h.compare_probes(want["pixelProbes"], rendered["frame"]["probes"], EXP["tolerances"]["pixel"]))
    finally:
        session.close()


def availability(ctx, key, **values):
    """Is the logical op implemented? Asked of `tessel-headless`, which has no renderer and no GPU: an
    existing op answers something else than `unknown_operation` (e.g. `backend_unavailable`)."""
    iface = ctx.interfaces[key]
    if iface["op"] is None:
        raise Blocked(f"no operation configured for `{key}` in interfaces.json", iface["needs"], iface["backlog"])
    surface = "render" if key == "render_frame" else "headless"
    binary = require_binary(ctx, surface, "see headless_flow")
    session = h.JsonlSession(binary, [] if surface == "render" else ["--empty"])
    try:
        must(session, {"op": "project.create", "name": "probe", "dir": str(ctx.work.sub("probe") / "ws")})
        must(session, {"op": "scene.attach", "id": "proof", "path": str(CORPUS / "bundle.json")})
        h.probe_interface(session, iface, scene="proof", frame=0, width=EXP["output"]["width"], height=EXP["output"]["height"],
                          probes=[], out=None, resources_root=str(CORPUS), **values)
    finally:
        session.close()
    return iface


def render_and_compare(p, op, iface, label, scene, frame_expectations, out_dir, tol):
    loaded = must(op, {"op": "scene.resources.resolve", "scene": scene, "base": str(CORPUS)})
    if loaded.get("state") == "queued":
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            status = must(op, {"op": "job.get", "job": loaded["job"]})
            if status["state"] in ("succeeded", "failed", "cancelled"):
                if status["state"] != "succeeded":
                    raise Abort(f"resource job failed: {status}")
                break
            time.sleep(0.03)
        else:
            raise Abort("scene resource job timed out")
        committed, commit = op({"op": "job.commit", "job": loaded["job"]})
        if not committed and commit.get("code") != "job_finished":
            raise Abort(f"resource job could not be applied: {commit}")
    for want in frame_expectations:
        probes = [[q["x"], q["y"]] for q in want["pixelProbes"]]
        out = out_dir / f"{label}-{want['editFrame']}.png"
        ok, body = h.probe_interface(op, iface, scene=scene, frame=want["editFrame"], width=EXP["output"]["width"],
                                     height=EXP["output"]["height"], out=str(out), probes=probes, resources_root=str(CORPUS))
        if not p.check(f"{label}: frame {want['editFrame']} renders", ok, body):
            continue
        frame = h.dig(body, iface["result_path"])
        p.check(f"{label}: frame {want['editFrame']} was read back (inspected)", frame.get("inspected") is True, frame.get("inspected"))
        mismatches = h.compare_probes(want["pixelProbes"], frame.get("probes", []), tol)
        p.check(f"{label}: frame {want['editFrame']} {len(probes)} probe pixels match the model", not mismatches, mismatches[:3])
        p.check(f"{label}: frame {want['editFrame']} PNG exists", out.is_file(), str(out))


def open_render_session(ctx, p, name):
    binary = require_binary(ctx, "render", "see package_restore_no_origin")
    h.acquire_gpu_lock()
    args = ["--gl", ctx.gl]
    session = h.JsonlSession(binary, args)
    work = ctx.work.sub(name)
    must(session, {"op": "project.create", "name": name, "dir": str(work / "ws")})
    must(session, {"op": "scene.attach", "id": "proof", "path": str(CORPUS / "bundle.json")})
    return session, work


def phase_render_headless(ctx, p):
    iface = availability(ctx, "render_frame")
    session, work = open_render_session(ctx, p, "render")
    try:
        render_and_compare(p, session, iface, "headless", "proof", EXP["frames"], work, EXP["tolerances"]["pixel"])
        p.metrics["gl"] = ctx.gl
    finally:
        session.close()


def phase_render_seek_order(ctx, p):
    iface = availability(ctx, "render_frame")
    session, work = open_render_session(ctx, p, "seek")
    try:
        by_frame = {f["editFrame"]: f for f in EXP["frames"]}
        order = [by_frame[f] for f in EXP["seekOrder"]]
        render_and_compare(p, session, iface, "out-of-order", "proof", order, work, EXP["tolerances"]["pixel"])
    finally:
        session.close()


def phase_resource_swap(ctx, p):
    iface = availability(ctx, "render_frame")
    session, work = open_render_session(ctx, p, "swap")
    tol = EXP["tolerances"]["pixel"]
    try:
        base = [f for f in EXP["frames"] if f["editFrame"] in (0, 20, 60)]
        render_and_compare(p, session, iface, "before swap", "proof", base, work, tol)
        must(session, {"op": "scene.detach", "scene": "proof"})
        must(session, {"op": "scene.attach", "id": "proof", "path": str(CORPUS / "bundle.swap.json")})
        render_and_compare(p, session, iface, "after swap (hero rev-2)", "proof", EXP["swapFrames"], work, tol)
        must(session, {"op": "scene.detach", "scene": "proof"})
        must(session, {"op": "scene.attach", "id": "proof", "path": str(CORPUS / "bundle.json")})
        render_and_compare(p, session, iface, "after swapping back", "proof", base, work, tol)
    finally:
        session.close()


def phase_context_loss(ctx, p):
    availability(ctx, "render_frame")
    availability(ctx, "lose_context")
    raise Blocked("context-loss adapter is configured but its flow is not implemented in this harness version",
                  "wire interfaces.lose_context to a forced loss between two renders of the same frame", "BL007")


def phase_socket_flow(ctx, p):
    binary = require_binary(ctx, "editor", "python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py build -p tessel-editor --release --locked")
    lab = ctx.editor_lab(binary)
    work = ctx.work.sub("socket")

    def reopen(document):
        # Same window, fresh project: the scene must come back from the file, not from memory.
        must(lab.op, {"op": "project.create", "name": "reopen", "dir": str(work / "ws2")})
        must(lab.op, {"op": "project.open", "path": str(document)})
        return lab.op
    run_scene_flow(p, lab.op, reopen, work / "ws", "socket")
    p.metrics["binary"] = ctx.binaries["editor"]
    p.metrics["note"] = "native window via local socket; reopen is in the same process (a window cannot be restarted without losing the socket)"


def phase_socket_render(ctx, p):
    iface = availability(ctx, "render_frame")
    binary = require_binary(ctx, "editor", "see socket_flow")
    h.acquire_gpu_lock()
    lab = ctx.editor_lab(binary)
    work = ctx.work.sub("socket-render")
    must(lab.op, {"op": "project.create", "name": "sockrender", "dir": str(work / "ws")})
    must(lab.op, {"op": "scene.attach", "id": "proof", "path": str(CORPUS / "bundle.json")})
    render_and_compare(p, lab.op, iface, "socket", "proof", EXP["frames"], work, EXP["tolerances"]["pixel"])
    # Compare the interactive monitor against an explicit frame at the same output size.
    ok, reference = h.probe_interface(lab.op, iface, scene="proof", frame=20,
        width=EXP["output"]["width"], height=EXP["output"]["height"],
        out=None, probes=[], resources_root=None)
    if not ok:
        raise Abort(f"cannot obtain monitor reference: {reference}")
    expected_hash = h.dig(reference, iface["result_path"])["hash"]
    baseline = must(lab.op, {"op": "scene.render", "scene": "proof", "frame": 0,
        "width": EXP["output"]["width"], "height": EXP["output"]["height"]})["frame"]["request"]
    must(lab.op, {"op": "seek", "frame": 20})
    must(lab.op, {"op": "monitor.show_scene", "scene": "proof"})
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        status = must(lab.op, {"op": "render.status"})
        if (status.get("last_frame") or {}).get("hash") == expected_hash and (status.get("last_frame") or {}).get("request", 0) > baseline:
            break
        time.sleep(0.03)
    p.check("interactive scene monitor converges to a new frame with the explicit reference pixels", (status.get("last_frame") or {}).get("hash") == expected_hash and (status.get("last_frame") or {}).get("request", 0) > baseline)


def phase_ui_semantic(ctx, p):
    binary = require_binary(ctx, "editor", "see socket_flow")
    lab = ctx.editor_lab(binary)
    work = ctx.work.sub("ui")
    must(lab.op, {"op": "project.create", "name": "ui", "dir": str(work / "ws")})
    must(lab.op, {"op": "scene.attach", "id": "proof", "path": str(CORPUS / "bundle.json")})
    mode = lab.send({"command": "set", "target": "inspector.mode", "field": "value", "value": "scene"})
    if not mode.get("ok"):
        raise Abort(f"cannot open scene inspector: {mode}")
    def nodes(value):
        if isinstance(value, dict):
            if "id" in value:
                yield value
            for child in value.values():
                yield from nodes(child)
        elif isinstance(value, list):
            for child in value:
                yield from nodes(child)
    # Expand the real visible tree; hidden descendants are intentionally absent from the UI tree.
    expanded = set()
    for _ in range(16):
        tree = lab.send({"command": "tree"})
        choices = [node for node in nodes(tree.get("data")) if node.get("kind") == "treeitem"
                   and node["id"].startswith("scene.entity:") and node["id"] not in expanded
                   and any(action.get("action") == "expand" for action in node.get("actions", []))]
        if not choices:
            break
        for node in choices:
            answer = lab.send({"command": "invoke", "target": node["id"], "action": "expand"})
            if not answer.get("ok"):
                raise Abort(f"cannot expand scene node: {answer}")
            expanded.add(node["id"])
    tree = lab.send({"command": "tree"})
    if not tree.get("ok"):
        raise Abort(f"the live `tree` command failed: {json.dumps(tree)[:300]}")
    def ids(value):
        if isinstance(value, dict):
            if isinstance(value.get("id"), str):
                yield value["id"]
            for child in value.values():
                yield from ids(child)
        elif isinstance(value, list):
            for child in value:
                yield from ids(child)
    hero_ids = sorted({node for node in ids(tree.get("data"))
                       if node.startswith("scene.entity:") and node.endswith("/node/hero")})
    if not hero_ids:
        raise Blocked("the live UI tree has no node for the attached scene (no `scene`/`hero` node): the inspector is not there",
                      "O front (BL014/BL021): nodes in state/tree.rs for the scene tree/selection/inspector with semantic actions", "BL014")
    node = hero_ids[0]
    answer = lab.send({"command": "invoke", "target": node, "action": "select"})
    p.check("selecting the hero node by its UI id succeeds", answer.get("ok"), answer)
    selection = must(lab.op, {"op": "scene.selection"})
    p.check("the shared selection now holds the hero", "node/hero" in canonical(selection), selection)


def phase_export(ctx, p):
    iface = availability(ctx, "export")
    raise Blocked(f"`{iface['op']}` exists but this harness version has no verified export adapter",
                  "frame-by-frame comparison of the exported file against the model (needs the final op contract)", "BL008")


PHASES = [
    ("corpus", "data (synthetic corpus, no destination)", phase_corpus),
    ("model_vs_origin", "data (reference model vs origin oracle)", phase_model),
    ("headless_flow", "headless process (tessel-headless, JSONL)", phase_headless_flow),
    ("scene_plan", "headless process: scene.plan", phase_scene_plan),
    ("package_restore_no_origin", "headless process (tessel-render, no GPU): package/restore", phase_package),
    ("render_headless_pixels", "headless + Chrome/GPU: scene render pixels", phase_render_headless),
    ("render_seek_out_of_order", "headless + Chrome/GPU: out-of-order seek", phase_render_seek_order),
    ("resource_swap", "headless + Chrome/GPU: resource swap by assetId revision", phase_resource_swap),
    ("context_loss", "headless/socket + GPU: forced context loss", phase_context_loss),
    ("socket_flow", "native window socket (scene.* flow)", phase_socket_flow),
    ("socket_render_pixels", "native window socket + GPU: scene render pixels", phase_socket_render),
    ("ui_semantic", "native UI semantic tree/selection (inspector)", phase_ui_semantic),
    ("export_directed", "headless + GPU: directed export", phase_export),
]


class Context:
    def __init__(self, args, work):
        self.work = work
        self.gl = args.gl
        self.bins = {"headless": args.bin_headless, "render": args.bin_render, "editor": args.bin_editor}
        self.binaries = {}
        self.interfaces = h.load_interfaces(args.interfaces)
        self._lab = None

    def editor_lab(self, binary):
        """One private editor shared by the native phases of this run, stopped at the end."""
        if self._lab is None:
            h.acquire_gpu_lock()
            home = self.work.sub("home")
            self._lab = h.EditorLab(binary, home)
            self._lab.wait_ready(90)
        return self._lab

    def close(self):
        if self._lab is not None:
            self._lab.stop()


def main():
    root = h.ROOT
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--list", action="store_true", help="list phases and exit")
    parser.add_argument("--phases", default="all", help="comma separated phase IDs (default: all)")
    parser.add_argument("--bin-headless", default=str(root / "target/debug/tessel-headless"))
    parser.add_argument("--bin-render", default=str(root / "target/debug/tessel-render"))
    parser.add_argument("--bin-editor", default=str(root / "target/release/tessel-editor"))
    parser.add_argument("--gl", default="swiftshader", choices=["swiftshader", "gpu"])
    parser.add_argument("--interfaces", default=None, help="interfaces.json override")
    parser.add_argument("--work", default=None, help="scratch directory (default: a fresh /tmp/tsp-*)")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--corpus", default=None, help="corpus directory (default: ./corpus); a tampered copy is the negative control")
    parser.add_argument("--report", default=None)
    args = parser.parse_args()

    global CORPUS, EXP
    if args.corpus:
        CORPUS = pathlib.Path(args.corpus).resolve()
        h.CORPUS = CORPUS
        EXP = json.loads((CORPUS / "expected.json").read_text())
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
    if EXP is None:
        print("corpus/expected.json missing: run corpus.py first", file=sys.stderr)
        return 2

    work = h.Workspace(keep=args.keep, base=args.work)
    ctx = Context(args, work)
    results = []
    started = time.time()
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
                phase.status = h.BLOCKED
                phase.blocked = {"reason": blocked.reason, "needs": blocked.needs, "backlog": blocked.backlog}
                print(f"  BLOCKED {blocked.backlog}: {blocked.reason}", flush=True)
            except Exception as error:  # noqa: BLE001 - any harness/request failure is a FAIL with its text
                phase.status = h.FAIL
                phase.error = f"{type(error).__name__}: {error}"
                print(f"  FAIL {phase.error}", flush=True)
            phase.seconds = time.time() - t0
            results.append(phase)
        within = work.within_budget()
    finally:
        ctx.close()
        work.close()

    statuses = [r.status for r in results]
    exit_code = 1 if h.FAIL in statuses else 3 if h.BLOCKED in statuses else 0
    if not within:
        exit_code = 1
    report = {
        "harness": {"name": "directed-scene-proof", "version": HARNESS_VERSION},
        "git": h.git_state(),
        "corpus": {"version": EXP["corpusVersion"], "bundle": EXP["bundle"], "swap_bundle": EXP["swapBundle"]},
        "tolerances": EXP["tolerances"],
        "binaries": ctx.binaries,
        "gl": ctx.gl,
        "seconds": round(time.time() - started, 1),
        "work_budget_respected": within,
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
        h.write_report(args.report, report)
        print(f"report: {args.report}")
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
