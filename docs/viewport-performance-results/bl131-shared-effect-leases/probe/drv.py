#!/usr/bin/env python3
"""BL131 measurement driver: isolated editor (own HOME and socket), semantic requests only."""
import json, os, re, shutil, socket, statistics, subprocess, sys, threading, time

BIN = sys.argv[1]
RT = sys.argv[2]
LABEL = sys.argv[3]
SAMPLES = os.environ.get("SAMPLES", "0")
PASSES = int(os.environ.get("PASSES", "5"))
FRAMES = list(range(int(os.environ.get("F0", "183")), int(os.environ.get("F1", "204"))))
if os.environ.get("RANGES"):
    FRAMES = [f for part in os.environ["RANGES"].split(",") for f in range(int(part.split("-")[0]), int(part.split("-")[1]) + 1)]
PROJECT = "/tmp/bl131/project/tessel-splat-h.tessel.json"
HOME = f"/tmp/bl131/h-{LABEL}"
OUT = os.environ.get("OUT", f"/tmp/bl131/results/{LABEL}.json")

shutil.rmtree(HOME, ignore_errors=True)
os.makedirs(HOME)
os.makedirs(os.path.dirname(OUT), exist_ok=True)
sock = os.path.join(HOME, ".cache/tessel-editor/ui.sock")
env = {**os.environ, "HOME": HOME, "TESSEL_RUNTIME_DIR": RT, "TESSEL_VIEWPORT_SCENE_SAMPLES": SAMPLES}
log = open(os.path.join(HOME, "editor.log"), "w")
proc = subprocess.Popen([BIN], env=env, stdout=log, stderr=log, start_new_session=True)


def send(req, timeout=300):
    with socket.socket(socket.AF_UNIX) as c:
        c.settimeout(timeout)
        c.connect(sock)
        c.sendall((json.dumps(req) + "\n").encode())
        data = b""
        while not data.endswith(b"\n"):
            chunk = c.recv(1 << 20)
            if not chunk:
                break
            data += chunk
    return json.loads(data)


def op(req, timeout=300):
    return send({"version": 1, "command": "op", "request": req}, timeout)


def gpu_util():
    out = subprocess.run(["ioreg", "-r", "-d", "1", "-w0", "-c", "IOAccelerator"], capture_output=True, text=True).stdout
    m = re.search(r'"Device Utilization %"=(\d+)', out)
    return int(m.group(1)) if m else None


gpu = []
stop = threading.Event()


def sample():
    while not stop.is_set():
        v = gpu_util()
        if v is not None:
            gpu.append(v)
        stop.wait(0.5)


def gpu_util():
    out = subprocess.run(["ioreg", "-r", "-d", "1", "-w0", "-c", "IOAccelerator"], capture_output=True, text=True).stdout
    m = re.search(r'"Device Utilization %"=(\d+)', out)
    return int(m.group(1)) if m else None


gpu = []
stop = threading.Event()


def sample():
    while not stop.is_set():
        v = gpu_util()
        if v is not None:
            gpu.append(v)
        stop.wait(0.5)


def pct(v, q):
    v = sorted(v)
    return v[min(len(v) - 1, round(q * (len(v) - 1)))]


try:
    t = time.time()
    while time.time() - t < 60:
        if proc.poll() is not None:
            raise SystemExit("editor exited: " + open(os.path.join(HOME, "editor.log")).read()[-800:])
        if os.path.exists(sock):
            try:
                if send({"version": 1, "command": "get", "target": "monitor.viewport"})["data"]["state"]["phase"] == "ready":
                    break
            except Exception:
                pass
        time.sleep(0.2)
    r = op({"op": "project.open", "path": PROJECT})
    assert r.get("ok"), r
    # first frame: wait for decode jobs
    t = time.time()
    while True:
        r = op({"op": "render.frame", "frame": FRAMES[0]})
        if r.get("ok"):
            break
        code = (r.get("error") or {}).get("code")
        if code not in ("media_preparing", "cancelled"):
            raise SystemExit(json.dumps(r)[:600])
        time.sleep(0.3)
    if os.environ.get("SHOW"):
        print(json.dumps(r)[:3000])
    rows = []
    MODE = os.environ.get("MODE", "frame")
    threading.Thread(target=sample, daemon=True).start()
    load0 = os.getloadavg()[0]
    def viewport():
        return send({"version": 1, "command": "get", "target": "monitor.viewport"})["data"]["state"]

    def seek(frame):
        return send({"version": 1, "command": "set", "target": "timeline.playhead", "field": "frame", "value": str(frame)})

    def settled(timeout=30):
        end = time.time() + timeout
        while time.time() < end:
            st = viewport(); s = st["session"]; last = st.get("last_frame")
            if st["phase"] == "ready" and s["status"] == "ready" and not s.get("pending") and last and last["revision"] == s["revision"] and not s.get("refine_pending"):
                return True
            time.sleep(0.02)
        return False

    def interactive(frame):
        # The first picture of the seek (the interactive one); the refine that may follow is not measured.
        before = viewport()["session"]["revision"]
        seek(frame)
        end = time.time() + 30
        while time.time() < end:
            st = viewport(); last = st.get("last_frame")
            if last and last["revision"] > before and st["session"]["revision"] == last["revision"]:
                return last
            time.sleep(0.004)
        raise SystemExit("no frame for seek " + str(frame))

    if MODE == "seek":
        op({"op": "render.frame", "frame": FRAMES[0]})
        settled()
    for p in range(PASSES + 1):  # pass 0 is warm-up (cold loads), not reported
        for f in FRAMES:
            if MODE == "seek":
                fr = interactive(f)
                perf = (fr.get("source") or {}).get("perf") or {}
                rows.append({"pass": p, "frame": f, "size": [fr.get("width"), fr.get("height")], "hash": fr.get("hash"), "render_ms": fr["render_ms"], "total_ms": fr.get("total_ms"),
                             "prepare_ms": fr.get("prepare_ms"), "sum": perf.get("sum", {}), "cnt": perf.get("cnt", {}),
                             "cache": {k: v for k, v in (fr.get("cache") or {}).items() if k.startswith(("splat_", "result_", "rig_"))}})
                settled()
                continue
            r = op({"op": "render.frame", "frame": f})
            assert r.get("ok"), r
            fr = r["data"].get("frame") or r["data"]
            perf = (fr.get("source") or {}).get("perf") or {}
            rows.append({"pass": p, "frame": f, "hash": fr["hash"], "render_ms": fr["render_ms"], "total_ms": fr.get("total_ms"),
                         "prepare_ms": fr.get("prepare_ms"), "sum": perf.get("sum", {}), "cnt": perf.get("cnt", {}),
                         "layers": (fr.get("source") or {}).get("original_layers")})
    stop.set()
    json.dump({"rows": rows, "gpu_util": gpu, "load1_start": load0, "load1_end": os.getloadavg()[0]}, open(OUT, "w"))
    print("rows", len(rows), "->", OUT)
finally:
    proc.terminate()
    try:
        proc.wait(10)
    except Exception:
        proc.kill()
    log.close()
