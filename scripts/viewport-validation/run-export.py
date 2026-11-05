#!/usr/bin/env python3
"""Item 6: complete short exports (frames + encode + mux + audio) through tessel-render, per runtime variant.
   run-export.py --out raw.jsonl --work DIR [--gl swiftshader|gpu] [--presets fullhd,uhd4k]
Compares decoded frames (framemd5) between runs; checks audio spectrally; cancel/recover and snapshot immutability."""
import argparse, glob, hashlib, json, os, shutil, subprocess, sys, time
HERE = os.path.dirname(os.path.abspath(__file__)); TEV = os.environ.get("TEV", "/tmp/tev")
SNAP = sorted(glob.glob(f"{TEV}/snap/*"))[0]; RENDER = f"{SNAP}/bin/tessel-render"
sys.path.insert(0, f"{SNAP}/scripts"); import tonecheck
ap = argparse.ArgumentParser(); ap.add_argument("--out", required=True); ap.add_argument("--work", required=True); ap.add_argument("--gl", default="swiftshader"); ap.add_argument("--presets", default="fullhd,uhd4k"); ap.add_argument("--repeat", type=int, default=2); ap.add_argument("--variants", default="baseline,retained"); ap.add_argument("--heavy", default=""); a = ap.parse_args()
os.makedirs(a.work, exist_ok=True); out = open(a.out, "a")
def ff(*x): return subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-y", *x], capture_output=True, text=True)
class S:
    def __init__(self, variant):
        env = dict(os.environ, TESSEL_RUNTIME_DIR=f"{TEV}/rt/{variant}", TESSEL_GL=a.gl)
        self.p = subprocess.Popen([RENDER], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1, env=env)
    def op(self, **r):
        self.p.stdin.write(json.dumps(r) + "\n"); self.p.stdin.flush(); line = self.p.stdout.readline()
        return json.loads(line) if line else {"ok": False, "error": "EOF"}
    def must(self, **r):
        x = self.op(**r)
        if not x.get("ok"): raise SystemExit(f"{r.get('op')}: {json.dumps(x)[:400]}")
        return x
    def close(self):
        try: self.p.stdin.close(); self.p.wait(30)
        except Exception: self.p.kill()
fx = f"{TEV}/fx"; orig = f"{a.work}/originals"; os.makedirs(orig, exist_ok=True); clip = f"{orig}/clip.mp4"
if not os.path.exists(clip):
    ff("-f", "lavfi", "-i", "testsrc2=duration=2:size=640x360:rate=24", "-f", "lavfi", "-i", "sine=frequency=330:duration=2", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", clip)
setup = [json.loads(l) for l in open(f"{fx}/proof.jsonl") if l.strip()]
def build(s, tag, heavy=None):
    w = f"{a.work}/proj-{tag}"; shutil.rmtree(w, ignore_errors=True)
    s.must(op="project.create", name="Export proof", sequence="main", rate=24, width=1920, height=1080, length=48, dir=w)
    for r in setup[1:]:
        if heavy and r.get("id") == "cloud": r = dict(r, path=f"{TEV}/assets/{heavy}.splat")
        s.must(**r)
    im = s.must(op="media.import", paths=[clip]); item = im["result"]["applied"]["items"][0]["id"]
    s.must(op="track.add", id="v9", name="Own sound", kind="video")
    s.must(op="clip.add", id="c-own", track="v9", asset=item, start=24, duration=24, properties={"zoom": 40, "position_x": 30})
def probe(path):
    p = json.loads(subprocess.run(["ffprobe", "-v", "error", "-count_frames", "-show_entries", "stream=codec_type,codec_name,width,height,r_frame_rate,nb_read_frames,duration,pix_fmt,sample_rate,channels:format=duration,size", "-of", "json", path], capture_output=True, text=True).stdout)
    ts = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v", "-show_entries", "frame=pts_time", "-of", "csv=p=0", path], capture_output=True, text=True).stdout.replace(",", " ").split()
    p["pts_first_last"] = [ts[0], ts[-1]] if ts else None; p["pts_monotonic"] = ts == sorted(ts, key=float) and len(set(ts)) == len(ts)
    return p
def framemd5(path):
    r = subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-i", path, "-map", "0:v", "-f", "framemd5", "-"], capture_output=True, text=True).stdout
    return [l.split(",")[-1].strip() for l in r.splitlines() if l and not l.startswith("#")]
def audio_md5(path):
    r = subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-i", path, "-map", "0:a", "-f", "md5", "-"], capture_output=True, text=True).stdout.strip(); return r
def sha(p): return hashlib.sha256(open(p, "rb").read()).hexdigest()
def tones(p): return {k: round(tonecheck.tone_level(p, s, .8, f), 4) for k, s, f in [("330_first", .1, 330), ("330_second", 1.1, 330), ("440_first", .1, 440), ("440_second", 1.1, 440)]}
ref = {}
for preset in a.presets.split(","):
    for variant in a.variants.split(","):
        for rep in range(a.repeat):
            s = S(variant); tag = f"{variant}-{preset}-{a.gl}-{rep}"
            try:
                t0 = time.time(); build(s, tag, a.heavy or None); dest = f"{a.work}/{tag}.mp4"
                if os.path.exists(dest): os.remove(dest)
                t1 = time.time(); r = s.op(op="export.start", preset=preset, destination=dest, wait=True); dt = time.time() - t1
                rec = dict(kind="export", variant=variant, preset=preset, gl=a.gl, rep=rep, heavy=a.heavy, ok=r.get("ok"), seconds=round(dt, 2), error=None if r.get("ok") else json.dumps(r)[:400])
                if r.get("ok"):
                    fm = framemd5(dest); rec.update(file_sha=sha(dest), size=os.path.getsize(dest), probe=probe(dest), frames=len(fm), frames_sha=hashlib.sha256("".join(fm).encode()).hexdigest()[:16], audio_md5=audio_md5(dest), tones=tones(dest))
                    k = preset; ref.setdefault(k, (variant, rep, fm)); rv, rr, rf = ref[k]
                    rec["vs_ref"] = dict(ref=f"{rv}#{rr}", identical_frames=sum(x == y for x, y in zip(fm, rf)), of=len(rf))
                    for i in (0, 12, 30, 47): ff("-i", dest, "-vf", f"select=eq(n\\,{i})", "-frames:v", "1", f"{a.work}/{tag}-f{i}.png")
                out.write(json.dumps(rec) + "\n"); out.flush(); print(tag, r.get("ok"), rec["seconds"], rec.get("frames"), rec.get("vs_ref"), rec.get("tones"), rec.get("error"), flush=True)
            finally: s.close()
    # cancel + recover, immutability: only for the variant under test, at the smaller preset
for variant in a.variants.split(","):
    preset = a.presets.split(",")[0]; s = S(variant)
    try:
        tag = f"{variant}-cancel-{a.gl}"; build(s, tag, a.heavy or None); dest = f"{a.work}/{tag}.mp4"
        if os.path.exists(dest): os.remove(dest)
        st = s.must(op="export.start", preset=preset, destination=dest, wait=False); job = st["result"].get("job") or st["result"].get("id") or st["result"].get("info", {}).get("id")
        done0 = 0; t0 = time.time()
        while time.time() - t0 < 120:
            g = s.op(op="job.get", job=job); info = g.get("result", {}); done0 = info.get("done", 0)
            if done0 >= 3 or info.get("state") in ("succeeded", "failed", "cancelled"): break
            time.sleep(.05)
        c = s.op(op="job.cancel", job=job); w = s.op(op="job.wait", job=job, timeout_ms=120000)
        staging = glob.glob(dest + "*") + glob.glob(os.path.dirname(dest) + "/.*" + os.path.basename(dest) + "*")
        rec = dict(kind="cancel", variant=variant, gl=a.gl, done_at_cancel=done0, final=w.get("result", w).get("state"), dest_exists=os.path.exists(dest), leftovers=staging, cancel_ok=c.get("ok"))
        # recover: same process, same destination
        r = s.op(op="export.start", preset=preset, destination=dest, wait=True); rec["recover_ok"] = r.get("ok")
        if r.get("ok"): fm = framemd5(dest); rec["recover_identical"] = sum(x == y for x, y in zip(fm, ref[preset][2])); rec["of"] = len(ref[preset][2])
        # immutable snapshot: edit right after the export starts
        dest2 = f"{a.work}/{tag}-snap.mp4"
        if os.path.exists(dest2): os.remove(dest2)
        st = s.must(op="export.start", preset=preset, destination=dest2, wait=False); job = st["result"].get("job") or st["result"].get("id") or st["result"].get("info", {}).get("id")
        time.sleep(.2); e1 = s.op(op="clip.property", clip="c-img", property="opacity", value=0.1); e2 = s.op(op="clip.camera", clip="c-scene", position=[0, 1.2, 5], target=[0, 0, 0], fov=20, orbit=10)
        w = s.op(op="job.wait", job=job, timeout_ms=600000); rec.update(edit_ok=[e1.get("ok"), e2.get("ok")], snap_state=w.get("result", w).get("state"))
        if os.path.exists(dest2): fm2 = framemd5(dest2); rec["snap_identical"] = sum(x == y for x, y in zip(fm2, ref[preset][2])); rec["snap_of"] = len(ref[preset][2])
        # the edit must be visible in a new export, otherwise the check above proves nothing
        dest3 = f"{a.work}/{tag}-after.mp4"
        if os.path.exists(dest3): os.remove(dest3)
        r = s.op(op="export.start", preset=preset, destination=dest3, wait=True)
        if r.get("ok"): fm3 = framemd5(dest3); rec["after_edit_differs"] = sum(x != y for x, y in zip(fm3, ref[preset][2]))
        out.write(json.dumps(rec) + "\n"); out.flush(); print(json.dumps(rec)[:700], flush=True)
    finally: s.close()
