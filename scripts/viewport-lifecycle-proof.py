#!/usr/bin/env python3
"""Lifecycle proof of the monitor with the automatic preview proxy, in the *real window* (WKWebView).

    python3 scripts/viewport-lifecycle-proof.py [--binary target/release/tessel-editor]
                                                [--out FILE] [--shots DIR] [--keep] [--only a,b,...]

Starts the DIRECT binary with an isolated HOME (own socket, own settings: the user's editor and
project are never touched) and a scratch copy of the runtime page. With semantic requests only
(`set timeline.playhead`, `invoke panel:monitor`, `invoke monitor.reload`, `op ...`; no OS pointer,
no coordinate clicks) it breaks the viewport in the ways the OPT01-OPT10 report left unproved and
checks that the LAST position asked for converges to the right picture:

  * context lost mid scrub frame / mid refine frame / while the refine timer is waiting /
    while the playhead keeps moving / while the monitor is hidden / six times in a row;
  * the monitor hidden and shown, and the window minimized and activated, with a proxy picture
    on screen and the playhead still moving;
  * `monitor.reload` and a project replaced (`project.open`) with a frame in flight and an
    explicit `render.frame` waiting (the reload is retried until the in-flight frame is really
    cancelled; the attempts are recorded);
  * the context lost and then the project replaced / the page reloaded before the restore, and a
    context that is NEVER restored (typed `context_lost` failure, no hang, `monitor.reload` recovers);
  * the monitor flapping hide/show 8 times and a dialog (`flow.import` / `dialog.cancel`) opened over it;
  * the editor's own export (original quality) of 12 frames before and after all of it: framemd5 must
    be identical (the incidents never reach the original render).

"Converged" is checked three ways: the host's own state (`status ready`, nothing pending, no refine
left, `presented.frame` == the playhead, `last_frame.revision` == `session.revision`); the picture
(the editor runs with TESSEL_VIEWPORT_INSPECT=1, so the hash of the picture on screen must equal the
hash of an explicit original `render.frame` of that frame at the same size, and its barcode must decode
to the frame, +100 for the second project); and the resources (`last_frame.cache`: GL textures, retained
entries, `context_generation` counting exactly the losses this script caused).

HOW THE CONTEXT IS LOST. The only way to lose a context in the real WKWebView through semantic requests
is a scratch copy of the runtime page (`TESSEL_RUNTIME_DIR`, the same `runtime.js`) whose index.html
installs a small trigger *before* the runtime: when a `frame.render` for a chosen frame/quality arrives, it
calls the runtime's own `debug.loseContext()` (WEBGL_lose_context on the real WebKit context) after a delay and
`debug.restoreContext()` later. The browser fires genuine `webglcontextlost`/`webglcontextrestored` and the
unmodified editor and runtime recover. This is a FORCED loss: not a GPU reset, not a driver crash, and the
timing of the photon is not measured. The runtime bundle itself is not modified.

Prints a JSON report on stdout's last line (and to --out). Exit 1 on a failed check, 2 on a missing prerequisite.
Own artifacts live under /tmp/tvl-* (budget 2 GiB, refuses to start under 6 GiB free) and are removed unless --keep.
"""
import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WIDTH, HEIGHT, FRAMES = 640, 360, 48
MIN_FREE_GIB, BUDGET_GIB = 6, 2
# The second project's barcode is shifted by this much, so a picture of the other project can never be mistaken for the right one.
OTHER_OFFSET = 100
PROXY_HEIGHT = 180

failures = []
report = {"checks": [], "scenarios": {}}
current = {"name": None}


def check(name, condition, detail=""):
    scope = current["name"]
    label = f"{scope}: {name}" if scope else name
    report["checks"].append({"check": label, "ok": bool(condition), "detail": str(detail)[:400]})
    print(("PASS " if condition else "FAIL ") + label + (f"  [{str(detail)[:300]}]" if detail and not condition else ""), flush=True)
    if not condition:
        failures.append(label)


def note(key, value):
    report["scenarios"].setdefault(current["name"] or "setup", {})[key] = value


# --------------------------------------------------------------------------- fixtures


def barcode_filter(offset):
    """Frame number (+ offset) as a 10-bit barcode in the top row, over noise, like video-live-proof.py."""
    cell = WIDTH // 20
    boxes = [f"drawbox=x=0:y=0:w={cell * 10}:h={cell}:color=black:t=fill"]
    for bit in range(10):
        boxes.append(f"drawbox=x={bit * cell}:y=0:w={cell}:h={cell}:color=white:t=fill:enable='mod(floor((n+{offset})/{2 ** bit}),2)'")
    return "noise=c0s=5:allf=t," + ",".join(boxes)


def make_clip(path, offset):
    command = [
        "ffmpeg", "-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i", f"testsrc2=size={WIDTH}x{HEIGHT}:rate=24:duration=100",
        "-frames:v", str(FRAMES), "-vf", barcode_filter(offset), "-c:v", "libx264", "-crf", "18", "-preset", "veryfast",
        "-pix_fmt", "yuv420p", "-g", "24", "-movflags", "+faststart", path,
    ]
    return subprocess.run(command, capture_output=True, text=True).returncode == 0


def decode(probes):
    return sum((1 << bit) for bit, pixel in enumerate(probes) if sum(pixel[:3]) / 3 > 128)


BARCODE_PROBES = [[(bit * 2 + 1) * WIDTH // 40, 8] for bit in range(10)]

# Installed BEFORE runtime.js so the wrapper is in place for the first message the host sends.
CHAOS_SCRIPT = r"""
(() => {
  const config = __CONFIG__;
  let current;
  const matches = (trigger, message) => {
    if (message.plan?.frame !== trigger.frame) return false;
    const preview = message.quality === 'preview';
    return trigger.quality === 'any' || (trigger.quality === 'preview') === preview;
  };
  const wrap = (runtime) => ({
    ...runtime,
    receive: (raw) => {
      const message = typeof raw === 'string' ? JSON.parse(raw) : raw;
      runtime.receive(raw);
      if (message.type !== 'frame.render') return;
      const trigger = config.triggers.find((item) => !item.used && matches(item, message));
      if (!trigger) return;
      trigger.used = true;
      setTimeout(() => {
        runtime.debug.loseContext();
        // A negative delay is a context that is never restored (what a lost GPU process looks like to the page).
        if (trigger.restore_after_ms >= 0) setTimeout(() => runtime.debug.restoreContext(), trigger.restore_after_ms);
      }, trigger.after_ms);
    },
  });
  Object.defineProperty(window, 'TesselRuntime', { configurable: true, get: () => current, set: (value) => { current = wrap(value); } });
})();
"""


def chaos_runtime(work, triggers):
    """A scratch `runtime/` whose page loses and restores the real context on cue (see the module docstring)."""
    source = os.path.join(ROOT, "runtime", "dist")
    target = os.path.join(work, "runtime", "dist")
    os.makedirs(target)
    shutil.copy(os.path.join(source, "runtime.js"), target)
    with open(os.path.join(source, "index.html")) as handle:
        html = handle.read()
    marker = '<script src="/runtime.js"></script>'
    if marker not in html:
        raise RuntimeError("runtime/dist/index.html changed: the chaos page cannot be built")
    script = CHAOS_SCRIPT.replace("__CONFIG__", json.dumps({"triggers": triggers}))
    with open(os.path.join(target, "index.html"), "w") as handle:
        handle.write(html.replace(marker, f"<script>{script}</script>\n    {marker}"))
    # The headless export worker (Node + Chrome) is looked up in the same directory. Symlinks make
    # `check()` pass while the worker, resolved to its real path, serves the REAL dist: the export
    # never runs the chaos page, so it is the reference the incidents must not disturb.
    for name in ("worker", "node_modules", "package.json"):
        os.symlink(os.path.join(ROOT, "runtime", name), os.path.join(work, "runtime", name))
    return os.path.join(work, "runtime")


# --------------------------------------------------------------------------- the editor under test


class Lab:
    """One isolated editor (own HOME, socket and runtime page)."""

    def __init__(self, binary, work, env):
        self.home = os.path.join(work, "home")
        os.makedirs(self.home)
        self.socket_path = os.path.join(self.home, ".cache/tessel-editor/ui.sock")
        self.log_path = os.path.join(self.home, "editor.log")
        self.log = open(self.log_path, "w")
        self.process = subprocess.Popen(
            [binary], env={**os.environ, "HOME": self.home, **env}, stdout=self.log, stderr=self.log, start_new_session=True
        )
        self.shots = None

    def send(self, request, timeout=60):
        with socket.socket(socket.AF_UNIX) as conn:
            conn.settimeout(timeout)
            conn.connect(self.socket_path)
            conn.sendall((json.dumps(request) + "\n").encode())
            data = b""
            while not data.endswith(b"\n"):
                chunk = conn.recv(1 << 20)
                if not chunk:
                    break
                data += chunk
        return json.loads(data)

    def op(self, request, timeout=60):
        return self.send({"version": 1, "command": "op", "request": request}, timeout)

    def invoke(self, target, action=None):
        request = {"version": 1, "command": "invoke", "target": target}
        if action:
            request["action"] = action
        return self.send(request)

    def seek(self, frame):
        return self.send({"version": 1, "command": "set", "target": "timeline.playhead", "field": "frame", "value": str(frame)})

    def viewport(self):
        return self.send({"version": 1, "command": "get", "target": "monitor.viewport"})["data"]["state"]

    def wait_ready(self, seconds=90):
        deadline = time.time() + seconds
        while time.time() < deadline:
            if self.process.poll() is not None:
                with open(self.log_path) as handle:
                    raise RuntimeError(f"the editor exited ({self.process.returncode}): {handle.read()[-600:]}")
            if os.path.exists(self.socket_path):
                try:
                    if self.viewport()["phase"] == "ready":
                        return
                except (OSError, ValueError, KeyError, TypeError):
                    pass
            time.sleep(0.25)
        raise RuntimeError("the viewport never became ready")

    def rss_mb(self):
        out = subprocess.run(["ps", "-o", "rss=", "-p", str(self.process.pid)], capture_output=True, text=True).stdout.strip()
        return round(int(out) / 1024, 1) if out else None

    def stop(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(10)
            except subprocess.TimeoutExpired:
                self.process.kill()
        self.log.close()


def wait_for(predicate, seconds, step=0.02):
    deadline = time.time() + seconds
    while time.time() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(step)
    return None


def proxy_layers(state):
    return ((state.get("last_frame") or {}).get("source") or {}).get("proxy_layers") or 0


def cache_of(state):
    return (state.get("last_frame") or {}).get("cache") or {}


def stats_of(state):
    return dict(state["session"]["stats"])


def delta(after, before):
    return {key: after[key] - before.get(key, 0) for key in after if isinstance(after[key], (int, float))}


def is_converged(state, frame):
    """The picture of `frame` is on screen, in original quality, and nothing is left to render or refine."""
    session = state["session"]
    last = state.get("last_frame")
    if state["phase"] != "ready" or session["status"] != "ready" or session.get("pending") or session.get("refine_pending"):
        return False
    if not last or last["revision"] != session["revision"] or proxy_layers(state):
        return False
    presented = session.get("presented")
    return presented is not None and presented["frame"] == frame


def settle(lab, frame, seconds=30):
    return wait_for(lambda: (lambda s: s if is_converged(s, frame) else None)(lab.viewport()), seconds, 0.01)


def explicit(lab, frame, size):
    """An explicit original frame at the monitor's own size: hash and barcode."""
    reply = lab.op({"op": "render.frame", "frame": frame, "width": size["width"], "height": size["height"], "probes": BARCODE_PROBES})
    data = (reply.get("data") or {}).get("frame") or {}
    return reply, data.get("hash"), (decode(data["probes"]) if data.get("probes") else None)


def prove_picture(lab, state, frame, offset=0, label="the picture"):
    """After convergence: the picture on screen is the original of `frame`, pixel for pixel, and the barcode says so."""
    screen = (state.get("last_frame") or {}).get("hash")
    reply, hash_, code = explicit(lab, frame, state["output"])
    check(f"{label} on screen has the hash of an explicit original render of frame {frame}", bool(screen) and screen == hash_, f"screen={screen} explicit={hash_}")
    check(f"{label}: the barcode decodes to frame {frame + offset}", code == frame + offset, f"code={code} reply={str(reply)[:160]}")
    return code


def no_stuck_failure(state):
    return not state["session"].get("error") and state["phase"] == "ready"


# --------------------------------------------------------------------------- projects


def build_project(lab, workspace, name, clip):
    document = os.path.join(workspace, f"{name}.tessel.json")
    steps = [
        {"op": "project.create", "name": name, "sequence": "main", "rate": 24, "width": WIDTH, "height": HEIGHT, "length": FRAMES, "dir": workspace},
        {"op": "project.save", "path": document},
        {"op": "preview.policy", "mode": "auto", "max_height": PROXY_HEIGHT},
        {"op": "media.import", "paths": [clip]},
    ]
    for step in steps:
        response = lab.op(step)
        if not response.get("ok"):
            return None, response

    def imported():
        assets = (lab.op({"op": "media.status"}).get("data") or {}).get("assets") or []
        return assets[0]["id"] if assets else None

    asset = wait_for(imported, 180, 0.3)
    if not asset:
        return None, {"error": "the import never produced an asset"}
    for step in [
        {"op": "track.add", "id": "t", "name": "T", "kind": "video"},
        {"op": "clip.add", "id": "c", "track": "t", "asset": asset, "start": 0, "duration": FRAMES},
        {"op": "project.save", "path": document},
    ]:
        response = lab.op(step)
        if not response.get("ok"):
            return None, response
    return {"document": document, "asset": asset}, None


def policy_reason(lab):
    data = lab.op({"op": "preview.policy"}).get("data") or {}
    assets = (data.get("report") or data).get("assets") or {}
    items = list(assets.values()) if isinstance(assets, dict) else list(assets)
    return items[0].get("reason") if items else None


def ensure_proxy(lab, seconds=240):
    """Policy back to `auto` (it is not part of the document) and the proxy built: the scrub is then drawn from it."""
    lab.op({"op": "preview.policy", "mode": "auto", "max_height": PROXY_HEIGHT})
    if wait_for(lambda: policy_reason(lab) == "proxy_ready", 20, 0.3):
        return True
    lab.op({"op": "media.preview_proxy"})
    return bool(wait_for(lambda: policy_reason(lab) == "proxy_ready", seconds, 0.3))


# --------------------------------------------------------------------------- scenario helpers


def scrub_to(lab, frame):
    """The first intent of a scrub, then wait for the proxy picture and the converged original."""
    lab.seek(frame)
    seen = {"proxy": False}

    def done():
        state = lab.viewport()
        if proxy_layers(state):
            seen["proxy"] = True
        return state if is_converged(state, frame) else None

    return wait_for(done, 30, 0.005), seen["proxy"]


def losses_since(before_state, after_state):
    return cache_of(after_state).get("context_generation", 0) - cache_of(before_state).get("context_generation", 0)


def footprint(state):
    cache = cache_of(state)
    return {key: cache.get(key) for key in ("gl_textures", "gl_geometries", "gl_programs", "texture_entries", "text_entries", "material_entries", "target_entries", "context_generation")}


def settled_scenario(lab, name, frame, expect_losses, steps=None, offset=0):
    """Runs one scenario body, then the common convergence/picture/resource checks for `frame`."""
    previous = current["name"]
    current["name"] = name
    before = lab.viewport()
    stats_before = stats_of(before)
    if steps:
        steps(lab)
    else:
        lab.seek(frame)
    state = settle(lab, frame, 40)
    check("the last position converges on the original (ready, nothing pending/refine, presented == playhead)", bool(state), "no convergence: " + json.dumps((lab.viewport()["session"]))[:400])
    if not state:
        current["name"] = previous
        return None
    check("no failure is left on the monitor", no_stuck_failure(state), state["session"].get("error"))
    check(f"the runtime saw exactly {expect_losses} context loss(es)", losses_since(before, state) == expect_losses, f"generation {cache_of(before).get('context_generation')} -> {cache_of(state).get('context_generation')}")
    prove_picture(lab, state, frame, offset)
    note("stats_delta", delta(stats_of(lab.viewport()), stats_before))
    note("footprint", footprint(state))
    current["name"] = previous
    return state


# --------------------------------------------------------------------------- scenarios

# Each chaos trigger fires once, for one (frame, quality). Frames are distinct per scenario so they never collide.
# Trigger frames are disjoint from every frame another scenario scrubs through (a loss that is never
# restored on a frame a sweep passes by would fail the sweep, not the scenario that owns it).
TRIGGERS = [
    {"name": "loss_mid_scrub", "frame": 21, "quality": "preview", "after_ms": 0, "restore_after_ms": 300},
    {"name": "loss_mid_refine", "frame": 22, "quality": "original", "after_ms": 0, "restore_after_ms": 300},
    {"name": "loss_while_refine_waits", "frame": 23, "quality": "preview", "after_ms": 70, "restore_after_ms": 800},
    {"name": "loss_while_scrubbing_on", "frame": 24, "quality": "preview", "after_ms": 0, "restore_after_ms": 1400},
    {"name": "loss_while_hidden", "frame": 33, "quality": "preview", "after_ms": 0, "restore_after_ms": 500},
    {"name": "loss_then_project_switch", "frame": 19, "quality": "preview", "after_ms": 0, "restore_after_ms": 1500},
    {"name": "loss_never_restored", "frame": 40, "quality": "preview", "after_ms": 0, "restore_after_ms": -1},
    {"name": "loss_then_reload", "frame": 20, "quality": "preview", "after_ms": 0, "restore_after_ms": 1500},
] + [{"name": f"cycle_{frame}", "frame": frame, "quality": "preview", "after_ms": 0, "restore_after_ms": 250} for frame in range(34, 40)]


def scenario_baseline(lab):
    current["name"] = "baseline"
    state, seen = scrub_to(lab, 10)
    check("a scrub frame is drawn from the proxy and the refine replaces it with the original", bool(state) and seen, f"proxy_seen={seen}")
    if not state:
        return None
    check("the refine ran", stats_of(state).get("refined", 0) >= 1, stats_of(state))
    prove_picture(lab, state, 10)
    note("renderer", (state.get("capabilities") or {}).get("renderer"))
    note("footprint", footprint(state))
    return footprint(state)


def scenario_losses(lab):
    settled_scenario(lab, "loss mid scrub frame", 21, 1)
    settled_scenario(lab, "loss mid refine frame", 22, 1)
    settled_scenario(lab, "loss while the refine timer waits (still lost when it fires)", 23, 1)

    def scrub_on(l):
        # The playhead keeps moving while the context is gone: every intent is answered, none hangs, the last one wins.
        l.seek(24)
        for frame in range(25, 32):
            time.sleep(0.12)
            l.seek(frame)

    settled_scenario(lab, "loss while the playhead keeps moving", 31, 1, steps=scrub_on)


def scenario_hidden_loss(lab, shots):
    def hidden(l):
        reply = l.invoke("panel:monitor", "hide")
        check("the monitor hides", reply.get("ok"), reply)
        time.sleep(0.3)
        check("the viewport reports off-screen", l.viewport()["on_screen"] is False)
        l.seek(33)
        time.sleep(1.4)
        reply = l.invoke("panel:monitor", "show")
        check("the monitor shows again", reply.get("ok"), reply)

    state = settled_scenario(lab, "context lost while the monitor is hidden", 33, 1, steps=hidden)
    check("the viewport reports on-screen again", lab.viewport()["on_screen"] is True)
    shots.take("hidden-loss-reshown")
    return state


def scenario_cycles(lab, baseline):
    current["name"] = "six losses in a row"
    rss_before = lab.rss_mb()
    start = lab.viewport()
    for frame in range(34, 40):
        lab.seek(frame)
        state = settle(lab, frame, 30)
        check(f"frame {frame} converged after its loss", bool(state))
        if not state:
            return
    end = lab.viewport()
    check("exactly six losses happened", losses_since(start, end) == 6, f"{cache_of(start).get('context_generation')} -> {cache_of(end).get('context_generation')}")
    prove_picture(lab, end, 39)
    now = footprint(end)
    note("footprint_before", footprint(start))
    note("footprint_after", now)
    # Nothing accumulates across losses: the GL side and the retained caches hold what one frame needs, as before.
    for key in ("gl_textures", "gl_geometries", "gl_programs", "texture_entries", "text_entries", "material_entries", "target_entries"):
        limit = (baseline.get(key) or 0) + 2
        check(f"{key} did not grow across the losses ({now.get(key)} <= {limit})", (now.get(key) or 0) <= limit, f"baseline={baseline.get(key)} now={now.get(key)}")
    rss_after = lab.rss_mb()
    note("editor_rss_mb", {"before": rss_before, "after": rss_after, "note": "the editor process only; the WebKit helper processes are not its children and are not measured"})
    check("the editor's own resident memory did not balloon (< +100 MB)", rss_before is None or rss_after is None or rss_after - rss_before < 100, f"{rss_before} -> {rss_after}")


def scenario_hide_show(lab, shots):
    current["name"] = "hide/show with a proxy picture and a moving playhead"

    def steps(l):
        for frame in range(41, 45):
            l.seek(frame)
            time.sleep(0.03)
        reply = l.invoke("panel:monitor", "hide")
        check("the monitor hides in the middle of the scrub", reply.get("ok"), reply)
        for frame in range(45, 48):
            time.sleep(0.05)
            l.seek(frame)
        time.sleep(0.6)
        check("the viewport reports off-screen while hidden", l.viewport()["on_screen"] is False)
        hidden = l.viewport()
        note("while_hidden", {"status": hidden["session"]["status"], "refine_pending": hidden["session"].get("refine_pending"), "presented": hidden["session"].get("presented")})
        reply = l.invoke("panel:monitor", "show")
        check("the monitor shows again", reply.get("ok"), reply)

    state = settled_scenario(lab, "hide/show with a proxy picture and a moving playhead", 47, 0, steps=steps)
    check("on-screen again after the show", lab.viewport()["on_screen"] is True)
    shots.take("hide-show-final")
    return state


def scenario_window(lab, shots):
    current["name"] = "window minimized and activated"

    def steps(l):
        reply = l.invoke("window.minimize")
        check("the window minimizes", reply.get("ok"), reply)
        time.sleep(1.0)
        note("on_screen_while_minimized", l.viewport()["on_screen"])
        l.seek(5)
        time.sleep(0.5)
        reply = l.invoke("window.activate")
        check("the window activates", reply.get("ok"), reply)
        time.sleep(1.0)

    settled_scenario(lab, "window minimized and activated", 5, 0, steps=steps)
    note("on_screen_after_activate", lab.viewport()["on_screen"])
    shots.take("window-activated")


def scenario_reload(lab):
    current["name"] = "monitor.reload with a frame in flight"
    before = lab.viewport()
    stats_before = stats_of(before)
    outcomes = []
    for attempt in range(6):
        waiter = {}

        def explicit_in_flight():
            started = time.time()
            waiter["reply"] = lab.op({"op": "render.frame", "frame": 9, "width": 1920, "height": 1080, "probes": BARCODE_PROBES}, timeout=60)
            waiter["seconds"] = time.time() - started

        lab.seek(8)
        thread = threading.Thread(target=explicit_in_flight)
        thread.start()
        time.sleep(0.002 * attempt)
        reply = lab.invoke("monitor.reload", "invoke")
        check(f"attempt {attempt}: the reload is accepted", reply.get("ok"), reply)
        thread.join(60)
        answered = "reply" in waiter
        check(f"attempt {attempt}: the explicit frame waiting during the reload is answered (not left hanging)", answered)
        if answered:
            error = (waiter["reply"].get("error") or {}).get("code")
            check(f"attempt {attempt}: it is a real frame or a typed error", waiter["reply"].get("ok") or error in ("cancelled", "backend_unavailable"), waiter["reply"])
            outcomes.append({"ok": waiter["reply"].get("ok"), "error": error, "seconds": round(waiter["seconds"], 2)})
        ready = wait_for(lambda: (lambda s: s if s["phase"] == "ready" and s["last_frame"] else None)(lab.viewport()), 30, 0.1)
        check(f"attempt {attempt}: the viewport is ready and drew a frame again", bool(ready))
        if any(item["error"] == "cancelled" for item in outcomes):
            break
    note("explicit_during_reload", outcomes)
    note("race_hit_cancelled", any(item["error"] == "cancelled" for item in outcomes))
    ensure_proxy(lab)
    settled_scenario(lab, "after the reload: the playhead position converges", 8, 0, steps=lambda l: l.seek(8))
    state, seen = scrub_to(lab, 12)
    check("after the reload the scrub is drawn from the proxy again and refined", bool(state) and seen, f"proxy_seen={seen}")
    if state:
        prove_picture(lab, state, 12)
    stats_after = stats_of(lab.viewport())
    note("stats_delta", delta(stats_after, stats_before))


def scenario_project_switch(lab, a, b, shots):
    current["name"] = "project replaced with a frame in flight"
    waiter = {}

    def explicit_in_flight():
        waiter["reply"] = lab.op({"op": "render.frame", "frame": 16, "width": WIDTH, "height": HEIGHT, "probes": BARCODE_PROBES}, timeout=60)

    lab.seek(15)
    thread = threading.Thread(target=explicit_in_flight)
    thread.start()
    time.sleep(0.04)
    opened = lab.op({"op": "project.open", "path": b["document"]})
    check("the second project opens while frames of the first are in flight", opened.get("ok"), opened)
    thread.join(60)
    reply = waiter.get("reply") or {}
    if reply.get("ok"):
        code = decode(((reply.get("data") or {}).get("frame") or {}).get("probes") or [])
        check("a frame answered across the switch is a picture of ONE project (frame 16 of the first, or of the second)", code in (16, 16 + OTHER_OFFSET), f"code={code}")
    else:
        check("the explicit frame in flight across the switch is a typed error, not a hang", (reply.get("error") or {}).get("code") in ("cancelled", "backend_unavailable", "media_preparing"), reply)
    note("explicit_across_switch", {"ok": reply.get("ok"), "error": (reply.get("error") or {}).get("code")})

    # `media_preparing` may answer until the decode of the opened project is done; wait for it.
    ready = wait_for(lambda: lab.op({"op": "render.frame", "frame": 0, "width": WIDTH, "height": HEIGHT}).get("ok"), 120, 0.5)
    check("the second project decodes and answers frames", bool(ready))
    ids = [item["id"] for item in ((lab.op({"op": "media.status"}).get("data") or {}).get("assets") or [])]
    check("the second project has its own asset", b["asset"] in ids, ids)
    check("the first project's asset is not in the project any more", a["asset"] not in ids or a["asset"] == b["asset"], ids)
    check("the proxy of the second project is built", ensure_proxy(lab))
    state, seen = scrub_to(lab, 15)
    check("the second project scrubs from its own proxy and converges", bool(state) and seen, f"proxy_seen={seen}")
    if state:
        session = state["session"]
        check("the session holds only the second project's assets", set(session.get("loaded_assets") or []) <= {b["asset"]}, session.get("loaded_assets"))
        check("and attached proxies belong to it", set(session.get("proxies") or []) <= {b["asset"]}, session.get("proxies"))
        prove_picture(lab, state, 15, offset=OTHER_OFFSET, label="the second project's picture")
        shots.take("second-project")
    # And back: the first project again, never a picture of the second.
    reopened = lab.op({"op": "project.open", "path": a["document"]})
    check("the first project opens again", reopened.get("ok"), reopened)
    wait_for(lambda: lab.op({"op": "render.frame", "frame": 0, "width": WIDTH, "height": HEIGHT}).get("ok"), 120, 0.5)
    ensure_proxy(lab)
    state, seen = scrub_to(lab, 18)
    check("the first project scrubs again from its proxy and converges", bool(state) and seen, f"proxy_seen={seen}")
    if state:
        prove_picture(lab, state, 18, offset=0, label="the first project's picture")
        check("no loaded asset or proxy of the second project is left", not (set(state["session"].get("loaded_assets") or []) & ({b["asset"]} - {a["asset"]})), state["session"].get("loaded_assets"))


def framemd5(path):
    out = subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-i", path, "-map", "0:v:0", "-f", "framemd5", "-"], capture_output=True, text=True)
    return [line.split(",")[-1].strip() for line in out.stdout.splitlines() if line and not line.startswith("#")]


def export_frames(lab, destination, frames):
    """The editor's own export (Node + Chrome worker, original quality) of the first `frames` frames."""
    started = lab.op({"op": "export.start", "preset": "fullhd", "destination": destination, "range": [0, frames], "replace": True})
    if not started.get("ok"):
        return None, started
    job = (started.get("data") or {}).get("job")

    def done():
        info = lab.op({"op": "job.get", "job": job}).get("data") or {}
        return info if info.get("state") in ("succeeded", "failed", "cancelled") else None

    info = wait_for(done, 300, 0.5)
    if not info or info.get("state") != "succeeded":
        return None, info
    return framemd5(destination), info


def scenario_export(lab, work, label, keep):
    current["name"] = f"export {label}"
    hashes, info = export_frames(lab, os.path.join(work, f"export-{label}.mp4"), 12)
    check("the editor's export of 12 frames succeeds", hashes is not None and len(hashes) == 12, info)
    keep[label] = hashes
    return hashes


def scenario_flap(lab, shots):
    """Hide/show the monitor over and over with a proxy picture on screen and the playhead moving."""

    def steps(l):
        for round_ in range(8):
            l.invoke("panel:monitor", "hide" if round_ % 2 == 0 else "show")
            l.seek(41 + round_ % 4)
            time.sleep(0.03)
        l.invoke("panel:monitor", "show")
        l.seek(46)

    settled_scenario(lab, "the monitor flaps (8 hide/show toggles) while scrubbing", 46, 0, steps=steps)
    check("on-screen at the end of the flapping", lab.viewport()["on_screen"] is True)


def scenario_dialog(lab, shots):
    """A dialog hides the WebView by design (a native view would paint over it); the scrub behind it still converges."""

    def steps(l):
        reply = l.invoke("flow.import")
        check("a dialog opens over the monitor", reply.get("ok"), reply)
        time.sleep(0.5)
        check("the viewport reports off-screen while the dialog is open", l.viewport()["on_screen"] is False)
        for frame in (30, 31, 32):
            l.seek(frame)
            time.sleep(0.08)
        reply = l.invoke("dialog.cancel", "invoke")
        check("the dialog closes", reply.get("ok"), reply)
        time.sleep(0.5)

    settled_scenario(lab, "dialog over the monitor while scrubbing", 32, 0, steps=steps)
    check("on-screen again after the dialog", lab.viewport()["on_screen"] is True)
    shots.take("after-dialog")


def reopen(lab, project, frame):
    """Opens `project`, waits for its decode and proxy, returns True when ready to scrub."""
    opened = lab.op({"op": "project.open", "path": project["document"]})
    if not opened.get("ok"):
        return False
    wait_for(lambda: lab.op({"op": "render.frame", "frame": 0, "width": WIDTH, "height": HEIGHT}).get("ok"), 120, 0.5)
    return ensure_proxy(lab)


def scenario_loss_then_switch(lab, a, b):
    """The context is lost, and before it comes back the project is replaced: the new project must be what shows."""
    current["name"] = "context lost, then the project is replaced before the restore"
    before = lab.viewport()
    lab.seek(19)
    time.sleep(0.4)
    opened = lab.op({"op": "project.open", "path": b["document"]})
    check("the second project opens while the context is lost", opened.get("ok"), opened)
    wait_for(lambda: lab.op({"op": "render.frame", "frame": 0, "width": WIDTH, "height": HEIGHT}).get("ok"), 120, 0.5)
    check("the proxy of the second project is built", ensure_proxy(lab))
    time.sleep(1.6)  # past the restore at 1.5 s
    state, seen = scrub_to(lab, 30)
    check("the second project converges after the restore", bool(state))
    if state:
        check("the runtime saw the loss", losses_since(before, state) >= 1, f"{cache_of(before).get('context_generation')} -> {cache_of(state).get('context_generation')}")
        check("the scrub came from the second project's proxy", seen)
        prove_picture(lab, state, 30, offset=OTHER_OFFSET, label="the second project's picture")
        check("the session holds nothing of the first project", set(state["session"].get("loaded_assets") or []) <= {b["asset"]}, state["session"].get("loaded_assets"))
    check("the first project opens again", reopen(lab, a, 0))


def scenario_never_restored(lab):
    """A context that is never restored: the failure is visible and typed, intents do not hang, and `monitor.reload` recovers."""
    current["name"] = "context lost and never restored"
    lab.seek(40)
    failed = wait_for(lambda: (lambda s: s if s["session"]["status"] == "failed" else None)(lab.viewport()), 15, 0.05)
    check("the monitor reports the failure", bool(failed))
    if failed:
        error = failed["session"].get("error") or {}
        check("it is a context_lost, said as such", error.get("code") == "context_lost", error)
        check("the header says it (`Falha: context_lost`)", "context_lost" in failed.get("headline", ""), failed.get("headline"))
    started = time.time()
    lab.seek(25)
    time.sleep(1.0)
    state = lab.viewport()
    check("a new intent does not hang the host (it answers, still failed, phase ready)", state["phase"] == "ready" and state["session"]["status"] == "failed", state["session"]["status"])
    check("and the failure is still context_lost", (state["session"].get("error") or {}).get("code") == "context_lost", state["session"].get("error"))
    note("seconds_to_answer_intent_while_lost", round(time.time() - started, 2))
    reply = lab.invoke("monitor.reload", "invoke")
    check("monitor.reload is accepted", reply.get("ok"), reply)
    recovered = settle(lab, 25, 45)
    check("the reload recovers: the playhead position converges on a fresh context", bool(recovered))
    if recovered:
        check("the fresh page counts no loss", cache_of(recovered).get("context_generation") == 0, cache_of(recovered).get("context_generation"))
        prove_picture(lab, recovered, 25)
    ensure_proxy(lab)


def scenario_loss_then_reload(lab):
    current["name"] = "context lost, then the page is reloaded before the restore"
    lab.seek(20)
    time.sleep(0.4)
    reply = lab.invoke("monitor.reload", "invoke")
    check("monitor.reload is accepted while the context is lost", reply.get("ok"), reply)
    ready = wait_for(lambda: (lambda s: s if s["phase"] == "ready" else None)(lab.viewport()), 30, 0.1)
    check("the viewport is ready again", bool(ready))
    time.sleep(1.8)  # the old page's restore timer died with it; nothing may fire into the new page
    ensure_proxy(lab)
    state, seen = scrub_to(lab, 29)
    check("the fresh page scrubs from the proxy and converges", bool(state) and seen, f"proxy_seen={seen}")
    if state:
        check("no failure survived the reload", no_stuck_failure(state), state["session"].get("error"))
        prove_picture(lab, state, 29)



# --------------------------------------------------------------------------- screenshots (evidence for a person, not an automatic proof)


class Shots:
    def __init__(self, directory):
        self.directory = directory
        self.window = None
        if directory:
            os.makedirs(directory, exist_ok=True)

    def window_id(self, pid):
        script = (
            "import CoreGraphics\nlet l = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]\n"
            f"for w in l {{ if (w[kCGWindowOwnerPID as String] as? Int) == {pid} {{ print(w[kCGWindowNumber as String]!) ; break }} }}"
        )
        return subprocess.run(["swift", "-e", script], capture_output=True, text=True).stdout.strip()

    def take(self, name):
        if not self.directory or not self.pid:
            return None
        time.sleep(0.4)
        window = self.window_id(self.pid)
        if not window:
            report.setdefault("shots_skipped", []).append(name)
            return None
        path = os.path.join(self.directory, f"{name}.png")
        subprocess.run(["screencapture", "-x", "-o", "-l", window, path], check=False)
        report.setdefault("shots", []).append(path)
        return path

    pid = None


# --------------------------------------------------------------------------- main


def free_gib(path):
    return shutil.disk_usage(path).free / 2**30


def tree_gib(path):
    out = subprocess.run(["du", "-sk", path], capture_output=True, text=True).stdout.split()
    return int(out[0]) / 2**20 if out else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", default=os.path.join(ROOT, "target/release/tessel-editor"))
    parser.add_argument("--out")
    parser.add_argument("--shots")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--only", help="comma separated: export,losses,hidden_loss,cycles,hide_show,dialog,window,loss_switch,switch,never_restored,loss_reload,reload (baseline always runs)")
    args = parser.parse_args()
    if not os.path.exists(args.binary):
        print(f"missing {args.binary}; run ./scripts/cargo build --release --locked")
        return 2
    for needed in ("runtime.js", "index.html"):
        if not os.path.exists(os.path.join(ROOT, "runtime", "dist", needed)):
            print("runtime/dist is not built; run ./scripts/runtime setup")
            return 2
    if shutil.which("ffmpeg") is None:
        print("ffmpeg is needed for the fixtures")
        return 2
    if free_gib("/tmp") < MIN_FREE_GIB:
        print(f"only {free_gib('/tmp'):.1f} GiB free; this proof needs {MIN_FREE_GIB}")
        return 2
    wanted = set(args.only.split(",")) if args.only else None
    run = lambda name: wanted is None or name in wanted

    work = tempfile.mkdtemp(prefix="tvl-", dir="/tmp")
    report["binary"] = args.binary
    lab = None
    shots = Shots(args.shots)
    try:
        clips = {"a": os.path.join(work, "a.mp4"), "b": os.path.join(work, "b.mp4")}
        check("fixture: clip A (barcode = frame)", make_clip(clips["a"], 0))
        check("fixture: clip B (barcode = frame + 100)", make_clip(clips["b"], OTHER_OFFSET))
        runtime_dir = chaos_runtime(work, TRIGGERS)
        lab = Lab(args.binary, work, {"TESSEL_VIEWPORT_TRACE": "1", "TESSEL_VIEWPORT_INSPECT": "1", "TESSEL_RUNTIME_DIR": runtime_dir})
        lab.wait_ready()
        shots.pid = lab.process.pid
        lab.invoke("window.activate")
        time.sleep(0.8)
        report["page"] = lab.viewport().get("page")
        check("the editor runs the scratch page (chaos installed)", str(report["page"]).startswith(runtime_dir), report["page"])
        workspace = os.path.join(lab.home, "ws")
        os.makedirs(workspace)

        current["name"] = "setup"
        a, error = build_project(lab, os.path.join(workspace, "a"), "A", clips["a"])
        check("project A with its clip", a is not None, error)
        if a is None:
            return 1
        b, error = build_project(lab, os.path.join(workspace, "b"), "B", clips["b"])
        check("project B with its clip", b is not None, error)
        if b is None:
            return 1
        opened = lab.op({"op": "project.open", "path": a["document"]})
        check("project A is open", opened.get("ok"), opened)
        ready = wait_for(lambda: lab.op({"op": "render.frame", "frame": 0, "width": WIDTH, "height": HEIGHT}).get("ok"), 120, 0.5)
        check("A decoded (explicit frame answers)", bool(ready))
        check("the preview proxy is built", ensure_proxy(lab), policy_reason(lab))
        lab.invoke("window.activate")
        time.sleep(1.0)

        baseline = scenario_baseline(lab)
        if baseline is None:
            return 1
        shots.take("baseline-converged")
        exports = {}
        if run("export"):
            scenario_export(lab, work, "before", exports)
        if run("losses"):
            scenario_losses(lab)
        if run("hidden_loss"):
            scenario_hidden_loss(lab, shots)
        if run("cycles"):
            scenario_cycles(lab, baseline)
        if run("hide_show"):
            scenario_hide_show(lab, shots)
            scenario_flap(lab, shots)
        if run("dialog"):
            scenario_dialog(lab, shots)
        if run("window"):
            scenario_window(lab, shots)
        if run("loss_switch"):
            scenario_loss_then_switch(lab, a, b)
        if run("switch"):
            scenario_project_switch(lab, a, b, shots)
        if run("never_restored"):
            scenario_never_restored(lab)
        if run("loss_reload"):
            scenario_loss_then_reload(lab)
        if run("reload"):
            scenario_reload(lab)
        if run("export"):
            after = scenario_export(lab, work, "after", exports)
            current["name"] = "export after the incidents"
            check("the export after every incident is frame-for-frame identical to the one before (the original never changed)", bool(exports.get("before")) and exports.get("before") == after, "framemd5 differs")
            report["export"] = {"frames": len(after or []), "identical": exports.get("before") == after}

        current["name"] = "end"
        final = lab.viewport()
        report["final"] = {"stats": stats_of(final), "ui": final.get("ui"), "capabilities_renderer": (final.get("capabilities") or {}).get("renderer")}
        check("the editor is still ready and unfailed at the end", final["phase"] == "ready" and not final["session"].get("error"), final["session"].get("error"))
        used = tree_gib(work)
        report["artifacts_gib"] = round(used, 2)
        check(f"own artifacts stayed under {BUDGET_GIB} GiB ({used:.2f})", used < BUDGET_GIB)
    except Exception as error:
        check("unhandled proof exception", False, repr(error))
        raise
    finally:
        try:
            if lab:
                lab.stop()
        finally:
            if args.keep:
                print("kept", work)
            else:
                shutil.rmtree(work, ignore_errors=True)
            report["failures"] = list(failures)
            try:
                if args.out:
                    with open(args.out, "w") as handle:
                        json.dump(report, handle, indent=1)
            finally:
                print(json.dumps(report))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
