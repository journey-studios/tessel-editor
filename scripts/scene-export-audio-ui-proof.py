#!/usr/bin/env python3
"""F042A (BL016): the native window exports a directed scene WITH its WAV mix, through the dialog.

Run it under the shared bench lock (the editor paints a real WebView and renders with Chrome):

    python3 scripts/directed-scene-proof/with-gpu-lock.py python3 scripts/scene-export-audio-ui-proof.py \
        --bin-editor target/debug/tessel-editor [--keep]

What it does, and only this:

  * starts its OWN editor: the direct binary, a short private HOME, its own unix socket. The user's
    window, project and socket are never addressed;
  * drives the export dialog (`flow.export_scene`) only with the semantic protocol
    (`get` / `set` / `invoke` / `op`). No pointer, no coordinates. `dialog.submit` reaches the same
    `scene.export` handler the library and the JSONL CLI use;
  * judges the MP4 it wrote by measurement: ffprobe (AAC, 48 kHz, stereo, audio as long as the video)
    and a spectral read of the decoded audio (440 Hz in the first half second, 880 Hz in the second).
    Nothing is heard and no audio device is involved (BL046).

Negative control: the approved mix is rewritten with other bytes of the very same size after
`scene.resources.resolve`; the dialog's export must fail with the typed `resource_changed`, write no
file and leave no staging directory. `audio: no` on the same project must still export, without audio.

Exit 0 only if every check passed. It uses < 200 MiB, refuses to start under 12 GiB free, and removes
only the directory it created.
"""
import argparse
import contextlib
import importlib.util
import json
import math
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
CORPUS = HERE / "directed-scene-export-proof" / "corpus"
MIN_FREE_GIB = 12


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules.setdefault(name, module)
    spec.loader.exec_module(module)
    return module


# Read-only reuse of the harness' private-instance helper (direct binary, own HOME, own socket).
harness = _load("tsp_harness", HERE / "directed-scene-proof" / "harness.py")


class Report:
    def __init__(self):
        self.checks = []
        self.failed = 0

    def check(self, name, condition, detail=""):
        ok = bool(condition)
        self.checks.append({"check": name, "ok": ok, "detail": "" if ok else str(detail)[:500]})
        print(("  PASS " if ok else "  FAIL ") + name + ("" if ok else f"  -> {str(detail)[:400]}"), flush=True)
        if not ok:
            self.failed += 1
        return ok


class Abort(Exception):
    pass


def free_gib(path="/tmp"):
    stat = os.statvfs(path)
    return stat.f_bavail * stat.f_frsize / 1024 ** 3


def goertzel(samples, rate, hz):
    coeff = 2.0 * math.cos(2.0 * math.pi * hz / rate)
    s1 = s2 = 0.0
    for sample in samples:
        s0 = sample + coeff * s1 - s2
        s2, s1 = s1, s0
    return (s1 * s1 + s2 * s2 - coeff * s1 * s2) / (len(samples) ** 2)


class Window:
    """Semantic access to the editor: `op` requests, `get`/`set`/`invoke` on live nodes."""

    def __init__(self, lab):
        self.lab = lab

    def op(self, request, timeout=60):
        ok, body = self.lab.op(request, timeout)
        if not ok:
            raise Abort(f"{request['op']} failed: {json.dumps(body)[:300]}")
        return body

    def get(self, target):
        return self.lab.send({"command": "get", "target": target})

    def state(self, target):
        answer = self.get(target)
        return answer["data"]["state"] if answer.get("ok") else None

    def fill(self, **fields):
        for name, value in fields.items():
            answer = self.lab.send({"command": "set", "target": f"dialog.field.{name}", "field": "value", "value": value})
            if not answer.get("ok"):
                raise Abort(f"cannot set dialog.field.{name}: {json.dumps(answer)[:300]}")

    def invoke(self, target, action=None):
        request = {"command": "invoke", "target": target}
        if action:
            request["action"] = action
        return self.lab.send(request)

    def job_ids(self):
        found = set()

        def walk(value):
            if isinstance(value, dict):
                if isinstance(value.get("id"), str):
                    found.add(value["id"])
                for child in value.values():
                    walk(child)
            elif isinstance(value, list):
                for child in value:
                    walk(child)

        walk(self.op({"op": "job.list"}))
        return found

    def wait_final(self, job, timeout):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            info = self.op({"op": "job.get", "job": job})
            if info["state"] in ("succeeded", "failed", "cancelled"):
                return info
            time.sleep(0.25)
        raise Abort(f"job {job} not final after {timeout}s")

    def resolve(self, scene, base):
        loaded = self.op({"op": "scene.resources.resolve", "scene": scene, "base": str(base)})
        if loaded.get("state") == "queued":
            info = self.wait_final(loaded["job"], 60)
            if info["state"] != "succeeded":
                raise Abort(f"resource job failed: {info.get('error')}")
            ok, body = self.lab.op({"op": "job.commit", "job": loaded["job"]})
            if not ok and body.get("code") != "job_finished":
                raise Abort(f"resource job could not be applied: {body}")


def submit_export(win, rep, label, destination, audio, timeout):
    """Fills and submits the dialog; returns the job info once final (None if no job started)."""
    before = win.job_ids()
    opened = win.invoke("flow.export_scene", "open")
    rep.check(f"{label}: flow.export_scene opens the dialog", opened.get("ok") and (opened.get("data") or {}).get("flow") == "export_scene", opened)
    win.fill(scene="withaudio", preset="fullhd", destination=str(destination), audio=audio, **{"from": "36", "frames": "24"})
    submitted = win.invoke("dialog.submit")
    if not rep.check(f"{label}: dialog.submit starts the export", submitted.get("ok"), submitted):
        return None
    rep.check(f"{label}: the dialog closed", not win.get("dialog").get("ok"), win.get("dialog"))
    new, deadline = set(), time.monotonic() + 30
    while not new and time.monotonic() < deadline:
        new = win.job_ids() - before
        time.sleep(0.1)
    if not rep.check(f"{label}: exactly one job was created", len(new) == 1, sorted(new)):
        return None
    return win.wait_final(next(iter(new)), timeout)


def probe(ffprobe, path):
    out = subprocess.run([ffprobe, "-v", "error", "-show_streams", "-of", "json", str(path)], capture_output=True, check=True)
    return json.loads(out.stdout)["streams"]


def decoded_mono(ffmpeg, path):
    out = subprocess.run([ffmpeg, "-v", "error", "-nostdin", "-i", str(path), "-map", "0:a:0", "-f", "s16le", "-ac", "1", "-ar", "48000", "-"],
                         capture_output=True, check=True)
    data = out.stdout
    return [int.from_bytes(data[i:i + 2], "little", signed=True) / 32768.0 for i in range(0, len(data) - 1, 2)]


def tool(name):
    found = shutil.which(name)
    if not found:
        raise SystemExit(f"{name} not found on PATH")
    return found


def tone(samples, start, end, hz, other):
    part = samples[int(start * 48000):int(end * 48000)]
    wanted, unwanted = goertzel(part, 48000, hz), goertzel(part, 48000, other)
    return wanted > 50 * unwanted and wanted > 1e-3, wanted, unwanted


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin-editor", default=str(ROOT / "target/debug/tessel-editor"))
    parser.add_argument("--keep", action="store_true", help="keep the work directory")
    parser.add_argument("--min-free-gib", type=float, default=MIN_FREE_GIB,
                        help="refuse to start under this much free disk (default 12; the run itself writes < 200 MiB)")
    parser.add_argument("--json", help="write the checks to this file")
    args = parser.parse_args()
    if free_gib() < args.min_free_gib:
        raise SystemExit(f"refusing to start: {free_gib():.1f} GiB free, need {args.min_free_gib}")
    binary = pathlib.Path(args.bin_editor).resolve()
    if not binary.is_file():
        raise SystemExit(f"{binary} not found: build it with `python3 .../cargo-serial.py build -p tessel-editor --locked`")
    ffmpeg, ffprobe = tool("ffmpeg"), tool("ffprobe")

    work = pathlib.Path(tempfile.mkdtemp(prefix="te-s9u-", dir="/tmp"))
    rep = Report()
    lab = None
    try:
        # A private copy of the corpus: the negative control rewrites the approved original.
        corpus = work / "corpus"
        shutil.copytree(CORPUS, corpus, ignore=shutil.ignore_patterns("expected.json", "provenance.json"))
        mix = corpus / "audio" / "mix.wav"
        pristine = mix.read_bytes()
        out = work / "out"
        out.mkdir()
        (work / "h").mkdir()
        lab = harness.EditorLab(binary, work / "h")
        lab.wait_ready(90)
        win = Window(lab)

        win.op({"op": "project.create", "name": "s9ui", "dir": str(work / "ws")})
        win.op({"op": "scene.attach", "id": "withaudio", "path": str(corpus / "bundle.audio.json")})
        win.resolve("withaudio", corpus)

        tree = lab.send({"command": "tree"})
        nodes = json.dumps(tree.get("data"))
        rep.check("the live tree exposes the export control", "flow.export_scene" in nodes, nodes[:200])

        print("== export with the mix (audio yes)")
        opened = win.invoke("flow.export_scene", "open")
        audio_field = (win.get("dialog.field.audio").get("data") or {})
        rep.check("the dialog offers audio yes/no and defaults to yes", audio_field.get("value") == "yes", audio_field)
        win.invoke("dialog.cancel") if opened.get("ok") else None
        destination = out / "with-audio.mp4"
        started = time.monotonic()
        info = submit_export(win, rep, "audio yes", destination, "yes", 600)
        if info is not None:
            rep.check("the job succeeded", info["state"] == "succeeded", info.get("error"))
            result = info.get("result") or {}
            audio = result.get("audio") or {}
            rep.check("the result says the mix is included", audio.get("included") is True and audio.get("declared") == ["audio"], audio)
            rep.check("the stream has the 48000 samples of the requested second",
                      (audio.get("mix") or {}).get("samples") == 48000 and (audio.get("measured") or {}).get("samples") == 48000, audio)
            rep.check("the file read is the approved one (SHA-256 of the pristine bytes)",
                      (audio.get("source") or {}).get("sha256") == __import__("hashlib").sha256(pristine).hexdigest(), audio.get("source"))
            snapshot = result.get("snapshot") or {}
            rep.check("the private byte copy includes the mix", snapshot.get("verified") is True and snapshot.get("bytes", 0) >= len(pristine), snapshot)
            rep.check("the export verified itself (ffprobe in the job)", (result.get("verification") or {}).get("ok") is True, result.get("verification"))
            rep.check("the file exists", destination.is_file(), destination)
            if destination.is_file():
                streams = probe(ffprobe, destination)
                aac = [s for s in streams if s["codec_type"] == "audio"]
                video = [s for s in streams if s["codec_type"] == "video"]
                rep.check("an AAC, 48 kHz, stereo audio stream is in the MP4",
                          len(aac) == 1 and aac[0]["codec_name"] == "aac" and aac[0]["sample_rate"] == "48000" and aac[0]["channels"] == 2, aac)
                if aac and video:
                    gap = abs(float(aac[0]["duration"]) - float(video[0]["duration"]))
                    rep.check("audio is as long as the video (± 60 ms)", gap < 0.06, gap)
                samples = decoded_mono(ffmpeg, destination)
                rep.check("about one second of audio was decoded", abs(len(samples) - 48000) <= 2048, len(samples))
                first = tone(samples, 0.05, 0.40, 440.0, 880.0)
                second = tone(samples, 0.60, 0.95, 880.0, 440.0)
                rep.check("the first half second is 440 Hz (the window 1.5 s)", first[0], first)
                rep.check("the second half second is 880 Hz (the window 2.0 s)", second[0], second)
            print(f"   ({time.monotonic() - started:.1f} s)")

        print("== audio no leaves the mix out, explicitly")
        destination_no = out / "no-audio.mp4"
        info = submit_export(win, rep, "audio no", destination_no, "no", 600)
        if info is not None:
            result = info.get("result") or {}
            rep.check("the job succeeded", info["state"] == "succeeded", info.get("error"))
            rep.check("the result records that audio was left out", (result.get("audio") or {}).get("included") is False, result.get("audio"))
            rep.check("a typed omission warning is reported", any(w.get("code") == "audio_omitted" for w in result.get("warnings") or []), result.get("warnings"))
            rep.check("the MP4 has no audio stream", destination_no.is_file() and not [s for s in probe(ffprobe, destination_no) if s["codec_type"] == "audio"], "")

        print("== negative: the approved mix is rewritten (same size) after resolve")
        # Same size and still a valid WAV (only sample bytes change): the stat check and the header
        # check pass, so only the hash of the job's private copy can notice.
        swapped = bytearray(pristine)
        for index in range(len(swapped) - 4096, len(swapped)):
            swapped[index] ^= 0x7F
        mix.write_bytes(bytes(swapped))
        rep.check("the swap kept the size", mix.stat().st_size == len(pristine), mix.stat().st_size)
        destination_swap = out / "swapped.mp4"
        info = submit_export(win, rep, "swapped mix", destination_swap, "yes", 600)
        if info is not None:
            error = info.get("error") or {}
            rep.check("the job failed", info["state"] == "failed", info["state"])
            rep.check("with the typed resource_changed (never job_failed)", error.get("code") == "resource_changed", error)
            details = error.get("details") or {}
            rep.check("naming the resource and both identities",
                      details.get("resource") == "audio-mix" and details.get("expected", {}).get("sha256") != details.get("got", {}).get("sha256"), details)
            jobs_view = json.dumps(lab.send({"command": "get", "target": "jobs"}).get("data"))
            rep.check("the Jobs view (get jobs) carries the failure", "resource_changed" in jobs_view or "approved" in jobs_view, jobs_view[:300])
            rep.check("no MP4 was published", not destination_swap.exists(), destination_swap)
            leftovers = sorted(p.name for p in out.iterdir() if p.name.startswith(".tessel-"))
            rep.check("no staging directory was left behind", leftovers == [], leftovers)
            rep.check("the original was not repaired or removed", mix.read_bytes() == bytes(swapped), "")
        print("== negative: a mix that is no longer a WAV is refused by the dialog before any job")
        mix.write_bytes(bytes([0x55]) * len(pristine))
        jobs_before = win.job_ids()
        win.invoke("flow.export_scene", "open")
        win.fill(scene="withaudio", preset="fullhd", destination=str(out / "notwav.mp4"), audio="yes", **{"from": "36", "frames": "24"})
        refused = win.invoke("dialog.submit")
        rep.check("dialog.submit is refused with the typed unsupported",
                  not refused.get("ok") and (refused.get("error") or {}).get("code") == "unsupported", refused)
        rep.check("the dialog stays open, no job started and no file written",
                  (win.state("dialog") or {}).get("flow") == "export_scene" and win.job_ids() == jobs_before
                  and not (out / "notwav.mp4").exists(), win.state("dialog"))
        win.invoke("dialog.cancel")
        destination_after = out / "after-swap-no-audio.mp4"
        info = submit_export(win, rep, "unusable mix, audio no", destination_after, "no", 600)
        if info is not None:
            rep.check("audio no does not touch the swapped mix and still exports", info["state"] == "succeeded" and destination_after.is_file(), info.get("error"))
    except Abort as error:
        rep.check("the flow could proceed", False, error)
    finally:
        if lab is not None:
            with contextlib.suppress(Exception):
                lab.stop()
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)

    total = len(rep.checks)
    print(f"\n{total - rep.failed}/{total} checks passed")
    if args.json:
        pathlib.Path(args.json).write_text(json.dumps({"checks": rep.checks, "failed": rep.failed}, indent=2))
    return 1 if rep.failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
