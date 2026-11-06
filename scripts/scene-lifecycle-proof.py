#!/usr/bin/env python3
"""Native lifecycle proof of the directed-scene monitor in the *real window* (WKWebView) — BL092 / F041.

    python3 scripts/directed-scene-proof/with-gpu-lock.py \\
        python3 scripts/scene-lifecycle-proof.py [--binary target/debug/tessel-editor] [--out FILE] [--keep] [--only a,b]

Starts the DIRECT binary with a short isolated HOME (own socket and settings: the user's editor and
project are never addressed) and a scratch copy of the runtime page. Everything is driven with semantic
requests (`op`, `set timeline.playhead`, `invoke panel:monitor`, `invoke monitor.reload`); no pointer and
no coordinates. It breaks the monitor of the directed scene the ways the host tests only simulate and checks
that the editor CONVERGES to the picture of the current frame:

  S0  baseline            frame/hash of the monitor == an explicit render of the same frame (and the independent
                          corpus model for the probes); resources held == resources the book says
  S1  loss mid frame      real WEBGL_lose_context while the monitor's own frame is drawn
  S2  hidden + loss       the monitor hidden, an explicit frame still draws and meets a lost context
  S3  hide / show         frames of a hidden monitor park, an explicit one draws, showing converges (and an
                          explicit frame pending across a hide is answered)
  S4  reload              `monitor.reload` with an explicit frame waiting for slow assets
  S5  resource swap       the scene's resources replaced (rev-1 -> rev-2 -> rev-1) while the frame that needs
                          them is in flight; the final picture is the CURRENT revision, never the old one
  S6  project switch      a new project while the old one's frame is in flight: its replaced loads end as
                          `cancelled` after the new project asked for the same slots and must not fail it
  S7  loss + switch       the project replaced before the context is restored
  S8  never restored      typed `context_lost`, no hang, `monitor.reload` recovers
  S9  loss + reload       the page reloaded before the context is restored
  S10 six losses          nothing accumulates: slots, runtime scene resources, GL textures stay bounded

HOW THE CONTEXT IS LOST (same technique as scripts/viewport-lifecycle-proof.py). The only way to lose a context
in the real WKWebView through semantic requests is a scratch copy of the runtime page (`TESSEL_RUNTIME_DIR`, the
UNMODIFIED `runtime.js`) whose index.html installs, before the runtime, a wrapper around `TesselRuntime.receive`:
when a directed `frame.render` for a chosen edit frame arrives it calls the runtime's own `debug.loseContext()`
(WEBGL_lose_context on the real WebKit context) and later `debug.restoreContext()`. The browser fires genuine
`webglcontextlost`/`webglcontextrestored`; the unmodified editor and runtime recover. This is a FORCED loss: not
a GPU reset or a driver crash. The same page delays `fetch('/asset?p=...')` of files under `slow-a/`/`slow-b/` so a
frame is deterministically "in flight" (asset still loading) when the scenario invalidates it. Triggers re-arm
whenever the page document reloads, so a scenario never renders another scenario's trigger frame.

Node and host unit tests (`viewport_host_tests.rs`, `scene_lifecycle_tests.rs`) do NOT substitute for this proof.

Prints a JSON report on the last line of stdout (and to --out). Exit 1 on a failed check, 2 on a missing
prerequisite. Own artifacts live under /tmp/te-r8-* (budget 2 GiB, refuses to start under 10 GiB free) and are
removed unless --keep. It never touches another editor instance.
"""
import argparse
import json
import os
import pathlib
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts" / "directed-scene-proof"))
import harness as h  # noqa: E402  (pure helpers only: probe comparison, sizes)

CORPUS = ROOT / "scripts" / "directed-scene-proof" / "corpus"
EXP = json.loads((CORPUS / "expected.json").read_text())
FRAMES = {f["editFrame"]: f for f in EXP["frames"]}
SWAP_FRAMES = {f["editFrame"]: f for f in EXP["swapFrames"]}
PIXEL_TOL = EXP["tolerances"]["pixel"]
WIDTH, HEIGHT = EXP["output"]["width"], EXP["output"]["height"]
MIN_FREE_GIB, BUDGET_GIB = 10, 2
SLOW_MS = 1200
# Host budget of the monitor's book (docs/features/scene-renderer.md): 32 slots.
HOST_SLOT_BUDGET = 32
SCENE_ASSETS = 3

failures = []
report = {"checks": [], "scenarios": {}}
current = {"name": None}


def check(name, condition, detail=""):
    label = f"{current['name']}: {name}" if current["name"] else name
    report["checks"].append({"check": label, "ok": bool(condition), "detail": str(detail)[:500]})
    print(("PASS " if condition else "FAIL ") + label + (f"  [{str(detail)[:400]}]" if detail and not condition else ""), flush=True)
    if not condition:
        failures.append(label)
    return bool(condition)


def note(key, value):
    report["scenarios"].setdefault(current["name"] or "setup", {})[key] = value


# ------------------------------------------------------------------------------------------ chaos page

# Installed BEFORE runtime.js so the wrapper is in place for the first message the host sends.
CHAOS_SCRIPT = r"""
(() => {
  const config = __CONFIG__;
  const realFetch = window.fetch.bind(window);
  const pathOf = (input) => {
    try {
      const url = new URL(typeof input === 'string' ? input : input.url, location.href);
      return decodeURIComponent(url.searchParams.get('p') || '');
    } catch (error) {
      return '';
    }
  };
  // A slow disk: the asset of a scene file under a `slow-*` directory arrives late, so a frame that needs
  // it is genuinely in flight (loading) while the scenario acts.
  window.fetch = (input, init) => {
    const path = pathOf(input);
    const rule = path ? config.slow.find((item) => path.includes(item.match)) : null;
    if (!rule) return realFetch(input, init);
    return new Promise((resolve) => setTimeout(resolve, rule.delay_ms)).then(() => realFetch(input, init));
  };
  const directedFrame = (message) => {
    const layers = message.plan?.layers ?? [];
    return layers.some((layer) => layer.kind === 'directed_scene') ? message.plan.frame : null;
  };
  let current;
  const wrap = (runtime) => ({
    ...runtime,
    receive: (raw) => {
      const message = typeof raw === 'string' ? JSON.parse(raw) : raw;
      runtime.receive(raw);
      if (message.type !== 'frame.render') return;
      const frame = directedFrame(message);
      const trigger = config.triggers.find((item) => !item.used && item.frame === frame);
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

# One trigger per scenario, on edit frames no other scenario renders before it (see the module docstring).
TRIGGERS = [
    {"name": "S1 loss mid monitor frame", "frame": 31, "after_ms": 0, "restore_after_ms": 300},
    {"name": "S2 loss mid explicit frame (hidden)", "frame": 36, "after_ms": 0, "restore_after_ms": 400},
    {"name": "S7 loss then project switch", "frame": 95, "after_ms": 0, "restore_after_ms": 1500},
    {"name": "S8 loss never restored", "frame": 70, "after_ms": 0, "restore_after_ms": -1},
    {"name": "S9 loss then reload", "frame": 80, "after_ms": 0, "restore_after_ms": 1500},
] + [{"name": f"S10 cycle {frame}", "frame": frame, "after_ms": 0, "restore_after_ms": 250} for frame in range(81, 87)]
SLOW = [{"match": "/slow-a/", "delay_ms": SLOW_MS}, {"match": "/slow-b/", "delay_ms": SLOW_MS}]


def chaos_runtime(work, triggers=TRIGGERS, slow=SLOW):
    """A scratch `runtime/` whose page loses and restores the real context on cue (see the module docstring)."""
    source = ROOT / "runtime" / "dist"
    target = pathlib.Path(work) / "runtime" / "dist"
    target.mkdir(parents=True)
    shutil.copy(source / "runtime.js", target / "runtime.js")
    html = (source / "index.html").read_text()
    marker = '<script src="/runtime.js"></script>'
    if marker not in html:
        raise RuntimeError("runtime/dist/index.html changed: the chaos page cannot be built")
    script = CHAOS_SCRIPT.replace("__CONFIG__", json.dumps({"triggers": triggers, "slow": slow}))
    (target / "index.html").write_text(html.replace(marker, f"<script>{script}</script>\n    {marker}"))
    return str(pathlib.Path(work) / "runtime")


WORK = None  # set by main(): the proof's own scratch dir, where the slow corpus copies live


def slow(name):
    """`<work>/<name>`: the corpus copy the chaos page serves late (see slow_corpus)."""
    return pathlib.Path(WORK) / name


def slow_corpus(work, name):
    """A copy of the corpus under `<work>/<name>/`: same bytes, so same pixels, but served late by the chaos page."""
    target = pathlib.Path(work) / name
    shutil.copytree(CORPUS, target, ignore=shutil.ignore_patterns("expected.json", "provenance.json"))
    return target


# ------------------------------------------------------------------------------------------ the editor under test


class Lab:
    """One isolated editor (own HOME, socket and runtime page)."""

    def __init__(self, binary, work, env):
        self.home = os.path.join(work, "h")
        os.makedirs(self.home)
        self.socket_path = os.path.join(self.home, ".cache/tessel-editor/ui.sock")
        if len(self.socket_path) > 100:
            raise SystemExit("socket path too long for a unix socket; use a shorter TMPDIR")
        self.log_path = os.path.join(self.home, "editor.log")
        self.log = open(self.log_path, "w")
        self.process = subprocess.Popen([binary], env={**os.environ, "HOME": self.home, **env}, stdout=self.log, stderr=self.log, start_new_session=True)

    def send(self, request, timeout=60):
        with socket.socket(socket.AF_UNIX) as conn:
            conn.settimeout(timeout)
            conn.connect(self.socket_path)
            conn.sendall((json.dumps({"version": 1, **request}) + "\n").encode())
            data = b""
            while not data.endswith(b"\n"):
                chunk = conn.recv(1 << 20)
                if not chunk:
                    break
                data += chunk
        return json.loads(data)

    def op(self, request, timeout=60):
        """`(ok, data-or-error)` of one semantic operation."""
        answer = self.send({"command": "op", "request": request}, timeout)
        return answer["ok"], (answer.get("data") if answer["ok"] else answer.get("error"))

    def invoke(self, target, action=None):
        request = {"command": "invoke", "target": target}
        if action:
            request["action"] = action
        return self.send(request)

    def seek(self, frame):
        return self.send({"command": "set", "target": "timeline.playhead", "field": "frame", "value": str(frame)})

    def viewport(self):
        return self.send({"command": "get", "target": "monitor.viewport"})["data"]["state"]

    def wait_ready(self, seconds=90):
        deadline = time.time() + seconds
        while time.time() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError(f"the editor exited ({self.process.returncode}): {pathlib.Path(self.log_path).read_text()[-600:]}")
            if os.path.exists(self.socket_path):
                try:
                    if self.viewport()["phase"] == "ready":
                        return
                except (OSError, ValueError, KeyError, TypeError):
                    pass
            time.sleep(0.25)
        raise RuntimeError("the viewport never became ready")

    def stop(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(10)
            except subprocess.TimeoutExpired:
                self.process.kill()
        self.log.close()


def wait_for(predicate, seconds, step=0.03):
    deadline = time.time() + seconds
    while time.time() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(step)
    return None


class Explicit:
    """An explicit `scene.render` sent from its own thread, so the scenario can act while it waits."""

    def __init__(self, lab, frame, probes=(), timeout=60):
        self.frame, self.started, self.seconds, self.result = frame, time.time(), None, None
        request = {"op": "scene.render", "scene": "proof", "frame": frame, "width": WIDTH, "height": HEIGHT, "probes": [list(p) for p in probes]}

        def run():
            try:
                self.result = lab.op(request, timeout)
            except Exception as error:  # a hang surfaces as a socket timeout
                self.result = (False, {"code": "proof_transport", "message": repr(error)})
            self.seconds = time.time() - self.started

        self.thread = threading.Thread(target=run, daemon=True)
        self.thread.start()

    def answer(self, seconds=40):
        """The typed answer, or None if the request is still hanging after `seconds`."""
        self.thread.join(seconds)
        return self.result if self.result is not None else None


def error_code(answer):
    return (answer[1] or {}).get("code") if answer and not answer[0] else None


# ------------------------------------------------------------------------------------------ facts about the monitor


def directed(state):
    return state.get("directed") or {}


def cache_of(state):
    return (state.get("last_frame") or {}).get("cache") or {}


def converged(state, frame):
    """The picture of `frame` was drawn by the monitor and nothing is in flight, parked, lost or failed."""
    d = directed(state)
    shown = d.get("shown") or {}
    return (
        state["phase"] == "ready"
        and d.get("scene") == "proof"
        and d.get("active") is None
        and d.get("queued") is None
        and not d.get("suspended")
        and shown.get("edit_frame") == frame
        and bool(state.get("last_frame"))
        and not str(state.get("headline", "")).startswith("Falha")
    )


def settle(lab, frame, seconds=40):
    return wait_for(lambda: (lambda s: s if converged(s, frame) else None)(lab.viewport()), seconds, 0.03)


def reference(lab, frame, expectations=None, label="reference"):
    """An explicit render of `frame` at the monitor's size, checked against the independent corpus model
    (probes) when the corpus has expectations for the frame. Returns the frame report (hash, probes)."""
    table = expectations or FRAMES
    want = table.get(frame)
    probes = [(p["x"], p["y"]) for p in want["pixelProbes"]] if want else []
    ok, body = lab.op({"op": "scene.render", "scene": "proof", "frame": frame, "width": WIDTH, "height": HEIGHT, "probes": [list(p) for p in probes]})
    if not check(f"{label}: explicit render of frame {frame} answers", ok, body):
        return None
    result = body["frame"]
    check(f"{label}: frame {frame} was read back (inspected)", result.get("inspected") is True, result.get("inspected"))
    if want:
        mismatches = h.compare_probes(want["pixelProbes"], result.get("probes", []), PIXEL_TOL)
        check(f"{label}: frame {frame} probes match the independent model", not mismatches, mismatches[:3])
    return result


def prove_current(lab, frame, expectations=None, label="the monitor"):
    """After convergence: the picture the monitor drew has the hash of an explicit (model-checked) render of
    the same frame, i.e. it is the current frame of the original, not a stale or partial one."""
    state = lab.viewport()
    screen = (state.get("last_frame") or {}).get("hash")
    ref = reference(lab, frame, expectations, label)
    if ref is None:
        return None
    check(f"{label} shows frame {frame} with the hash of its original render", bool(screen) and screen == ref["hash"], f"monitor={screen} explicit={ref['hash']}")
    return ref["hash"]


def budget(lab, label="resources", spare=0):
    """Resources freed/bounded: the book, the runtime's own count and the GL objects agree and stay in budget."""
    state = lab.viewport()
    d = directed(state)
    cache = cache_of(state)
    occupied = (d.get("slots") or {}).get("occupied")
    held = cache.get("scene_resources")
    # `spare`: a swapped resource has another identity (revision/hash), so its old picture may still sit in its own slot
    # until the budget needs it; the runtime must hold exactly what the book holds either way.
    in_range = occupied is not None and SCENE_ASSETS <= occupied <= min(SCENE_ASSETS + spare, HOST_SLOT_BUDGET)
    check(f"{label}: book holds {SCENE_ASSETS}..{SCENE_ASSETS + spare} slots (<= {HOST_SLOT_BUDGET}) and the runtime holds the same ({occupied} vs {held})", in_range and held == occupied, (occupied, held))
    check(f"{label}: no load is left tracked beyond the slots held", (d.get("slots") or {}).get("tracked_loads", 0) <= HOST_SLOT_BUDGET, d.get("slots"))
    return {key: cache.get(key) for key in ("scene_resources", "scene_generation", "context_generation", "gl_textures", "texture_entries")} | {"occupied": occupied}


# ------------------------------------------------------------------------------------------ project setup


def commit_job(lab, loaded, what):
    if loaded.get("state") != "queued":
        return True
    deadline = time.time() + 30
    status = {}
    while time.time() < deadline:
        ok, status = lab.op({"op": "job.get", "job": loaded["job"]})
        if ok and status["state"] in ("succeeded", "failed", "cancelled"):
            break
        time.sleep(0.03)
    else:
        return check(f"{what}: the resource job finishes", False, status)
    if status["state"] != "succeeded":
        return check(f"{what}: the resource job succeeds", False, status)
    ok, commit = lab.op({"op": "job.commit", "job": loaded["job"]})
    return check(f"{what}: the resource job is applied", ok or (commit or {}).get("code") == "job_finished", commit)


def attach_and_resolve(lab, bundle, base, what):
    ok, body = lab.op({"op": "scene.attach", "id": "proof", "path": str(bundle)})
    if not check(f"{what}: scene.attach", ok, body):
        return False
    ok, loaded = lab.op({"op": "scene.resources.resolve", "scene": "proof", "base": str(base)})
    return check(f"{what}: scene.resources.resolve", ok, loaded) and commit_job(lab, loaded, what)


def new_project(lab, work, name):
    folder = os.path.join(work, "ws", name)
    ok, body = lab.op({"op": "project.create", "name": name, "sequence": "main", "rate": 24, "width": WIDTH, "height": HEIGHT, "length": EXP["output"]["editFrames"], "dir": folder})
    return check(f"project {name} is created", ok, body)


def open_scene(lab, work, name, bundle, base, show=True):
    if not (new_project(lab, work, name) and attach_and_resolve(lab, bundle, base, name)):
        return False
    if not show:
        return True
    ok, body = lab.op({"op": "monitor.show_scene", "scene": "proof"})
    return check(f"{name}: monitor.show_scene", ok, body)


def scenario(name):
    current["name"] = name
    print(f"\n== {name}", flush=True)


# ------------------------------------------------------------------------------------------ scenarios


def s0_baseline(lab, work, env):
    scenario("S0 baseline")
    if not open_scene(lab, work, "s0", CORPUS / "bundle.json", CORPUS):
        return None
    check("the monitor draws frame 0 of the scene", bool(settle(lab, 0)), lab.viewport().get("directed"))
    lab.seek(20)
    state = settle(lab, 20)
    if not check("the playhead position converges on the monitor", bool(state), lab.viewport().get("directed")):
        return None
    env["hash_rev1_20"] = prove_current(lab, 20)
    # BL102: the real WebView runtime lists `asset.identity`, so the host stamps every load and
    # trusts only the answer that repeats its request.
    d = directed(lab.viewport())
    slots = d.get("slots") or {}
    check("the host negotiated `asset.identity` with the real runtime", d.get("asset_identity") is True, d)
    check("every load the book tracks was stamped with a request", slots.get("tracked_loads", 0) >= 1 and slots.get("identified_loads") == slots.get("tracked_loads"), slots)
    foot = budget(lab, "baseline")
    note("footprint", foot)
    note("renderer", (state.get("capabilities") or {}).get("renderer"))
    env["baseline"] = foot
    return foot


def s1_loss_mid_monitor_frame(lab, work, env):
    scenario("S1 context lost while the monitor draws its frame")
    before = lab.viewport()
    lost_before = directed(before).get("context_losses", 0)
    generation_before = cache_of(before).get("context_generation", 0)
    lab.seek(31)
    state = settle(lab, 31, 45)
    if not check("the monitor converges on frame 31 after the loss", bool(state), lab.viewport().get("directed")):
        return
    now = directed(state).get("context_losses", 0)
    check("the host saw exactly one context.lost", now - lost_before == 1, (lost_before, now))
    check("the runtime counted exactly one real loss", cache_of(state).get("context_generation", 0) - generation_before == 1, (generation_before, cache_of(state).get("context_generation")))
    prove_current(lab, 31)
    note("footprint", budget(lab, "after the loss"))


def s2_hidden_explicit_loss(lab, work, env):
    scenario("S2 hidden monitor, explicit frame, context lost mid frame")
    before = lab.viewport()
    lost_before = directed(before).get("context_losses", 0)
    check("the monitor hides", lab.invoke("panel:monitor", "hide").get("ok"))
    check("the viewport reports off-screen", bool(wait_for(lambda: lab.viewport()["on_screen"] is False, 5)))
    explicit = Explicit(lab, 36, [(p["x"], p["y"]) for p in FRAMES[36]["pixelProbes"]])
    answer = explicit.answer(40)
    check("the explicit frame of a hidden monitor is answered (not parked, not hanging)", answer is not None, "no answer in 40s")
    if answer:
        code = error_code(answer)
        check("it is the frame or a typed context_lost", answer[0] or code == "context_lost", answer)
        note("explicit_outcome", {"ok": answer[0], "error": code, "seconds": round(explicit.seconds, 2)})
        if answer[0]:
            mismatches = h.compare_probes(FRAMES[36]["pixelProbes"], answer[1]["frame"].get("probes", []), PIXEL_TOL)
            check("an answered frame of the hidden monitor matches the model", not mismatches, mismatches[:3])
    check("the loss happened in the hidden monitor (host saw context.lost)", bool(wait_for(lambda: directed(lab.viewport()).get("context_losses", 0) - lost_before == 1, 8)), directed(lab.viewport()))
    check("the context comes back (not suspended)", bool(wait_for(lambda: not directed(lab.viewport()).get("suspended"), 10)))
    check("the monitor shows again", lab.invoke("panel:monitor", "show").get("ok"))
    state = settle(lab, 31, 45)
    check("showing the monitor converges on the current playhead frame", bool(state), lab.viewport().get("directed"))
    if state:
        prove_current(lab, 31)


def s3_hide_show(lab, work, env):
    scenario("S3 hide / show parks the monitor and converges on show")
    check("the monitor hides", lab.invoke("panel:monitor", "hide").get("ok"))
    check("the viewport reports off-screen", bool(wait_for(lambda: lab.viewport()["on_screen"] is False, 5)))
    lab.seek(47)
    time.sleep(1.0)
    parked = lab.viewport()
    d = directed(parked)
    check("the frame of a hidden monitor is parked, not drawn", (d.get("shown") or {}).get("edit_frame") != 47 and d.get("active") is None and d.get("queued") is not None, d)
    explicit = reference(lab, 12, label="hidden monitor")
    check("an explicit frame of the hidden monitor is drawn", explicit is not None)
    check("the monitor shows again", lab.invoke("panel:monitor", "show").get("ok"))
    state = settle(lab, 47)
    check("showing the monitor starts the parked frame and converges on frame 47", bool(state), lab.viewport().get("directed"))
    if state:
        prove_current(lab, 47)
    note("footprint", budget(lab, "after hide/show"))

    scenario("S3b explicit frame pending while the monitor hides")
    if not open_scene(lab, work, "s3b", slow("slow-a") / "bundle.json", slow("slow-a"), show=True):
        return
    explicit = Explicit(lab, 48, [(p["x"], p["y"]) for p in FRAMES[48]["pixelProbes"]])
    time.sleep(0.3)
    check("the monitor hides with the explicit frame waiting for its assets", lab.invoke("panel:monitor", "hide").get("ok"))
    answer = explicit.answer(40)
    check("the pending explicit frame is answered across the hide", answer is not None)
    if answer:
        code = error_code(answer)
        check("it is the frame or a typed error", answer[0] or code in ("cancelled", "context_lost"), answer)
        note("pending_across_hide", {"ok": answer[0], "error": code, "seconds": round(explicit.seconds, 2)})
        if answer[0]:
            mismatches = h.compare_probes(FRAMES[48]["pixelProbes"], answer[1]["frame"].get("probes", []), PIXEL_TOL)
            check("an answered frame matches the model", not mismatches, mismatches[:3])
    check("the monitor shows again", lab.invoke("panel:monitor", "show").get("ok"))
    state = settle(lab, 0, 45)
    check("the monitor converges on the playhead frame after show", bool(state), lab.viewport().get("directed"))
    if state:
        prove_current(lab, 0)


def s4_reload_with_explicit_pending(lab, work, env):
    scenario("S4 monitor.reload with an explicit frame waiting for slow assets")
    if not open_scene(lab, work, "s4", slow("slow-a") / "bundle.json", slow("slow-a")):
        return
    explicit = Explicit(lab, 60, [(p["x"], p["y"]) for p in FRAMES[60]["pixelProbes"]])
    time.sleep(0.35)
    check("the reload is accepted", lab.invoke("monitor.reload", "invoke").get("ok"))
    answer = explicit.answer(30)
    check("the explicit frame pending during the reload is answered (not left hanging)", answer is not None)
    if answer:
        code = error_code(answer)
        check("it is typed `cancelled` (the page that would have drawn it is gone)", answer[0] or code == "cancelled", answer)
        note("explicit_during_reload", {"ok": answer[0], "error": code, "seconds": round(explicit.seconds, 2)})
    state = settle(lab, 0, 60)
    check("after the reload the monitor redraws the playhead frame", bool(state), lab.viewport().get("directed"))
    if state:
        prove_current(lab, 0)
        note("footprint", budget(lab, "after the reload"))
    lab.seek(60)
    state = settle(lab, 60, 60)
    check("and follows the playhead again", bool(state))
    if state:
        prove_current(lab, 60)


def s5_resource_swap(lab, work, env):
    scenario("S5a resource swap, no frame in flight (revision -> pixels)")
    if not open_scene(lab, work, "s5a", CORPUS / "bundle.json", CORPUS):
        return
    lab.seek(20)
    check("rev-1 converges on frame 20", bool(settle(lab, 20)), lab.viewport().get("directed"))
    rev1 = prove_current(lab, 20, label="rev-1")
    lab.op({"op": "scene.detach", "scene": "proof"})
    if not attach_and_resolve(lab, CORPUS / "bundle.swap.json", CORPUS, "rev-2"):
        return
    state = settle(lab, 20, 45)
    check("rev-2 converges on frame 20", bool(state), lab.viewport().get("directed"))
    rev2 = prove_current(lab, 20, SWAP_FRAMES, "rev-2")
    check("the swapped resource changes the picture (rev-2 hash differs from rev-1)", bool(rev1) and bool(rev2) and rev1 != rev2, (rev1, rev2))
    budget(lab, "after the swap", spare=1)

    scenario("S5b resources replaced while the frame that needs them is in flight")
    if not open_scene(lab, work, "s5b", slow("slow-a") / "bundle.json", slow("slow-a")):
        return
    lab.seek(20)
    check("rev-1 (slow files) converges on frame 20", bool(settle(lab, 20, 60)), lab.viewport().get("directed"))
    # Swap to rev-2: its hero loads late (slow-a), so the frame is in flight when the next replacement lands.
    lab.op({"op": "scene.detach", "scene": "proof"})
    if not attach_and_resolve(lab, slow("slow-a") / "bundle.swap.json", slow("slow-a"), "rev-2 (slow)"):
        return
    explicit = Explicit(lab, 20, [(p["x"], p["y"]) for p in SWAP_FRAMES[20]["pixelProbes"]])
    time.sleep(0.35)
    in_flight = lab.viewport()
    note("state_when_invalidated", {"active": directed(in_flight).get("active"), "queued": directed(in_flight).get("queued")})
    lab.op({"op": "scene.detach", "scene": "proof"})
    if not attach_and_resolve(lab, slow("slow-a") / "bundle.json", slow("slow-a"), "rev-1 again (slow)"):
        return
    answer = explicit.answer(40)
    check("the explicit frame in flight when the resources were replaced is answered", answer is not None)
    if answer:
        code = error_code(answer)
        check("it is typed (cancelled/resource_*) or a picture of exactly one revision", answer[0] or code in ("cancelled", "resource_stale", "resource_unresolved", "resource_mismatch"), answer)
        note("explicit_across_swap", {"ok": answer[0], "error": code})
        if answer[0]:
            probes = answer[1]["frame"].get("probes", [])
            one = not h.compare_probes(SWAP_FRAMES[20]["pixelProbes"], probes, PIXEL_TOL) or not h.compare_probes(FRAMES[20]["pixelProbes"], probes, PIXEL_TOL)
            check("an answered frame is the picture of ONE revision (matches rev-2 or rev-1 of frame 20)", one, probes[:3])
    state = settle(lab, 20, 60)
    check("after the replacement the monitor converges on the playhead frame", bool(state), lab.viewport().get("directed"))
    if state:
        final = prove_current(lab, 20, FRAMES, "after the swap back")
        check("the picture is the CURRENT revision (rev-1), not the invalidated rev-2", bool(final) and final == env.get("hash_rev1_20"), (final, env.get("hash_rev1_20")))
        note("footprint", budget(lab, "after the replacement", spare=1))


def s6_project_switch(lab, work, env):
    scenario("S6 project switch with a frame in flight; replaced loads end after the new project asked")
    if not open_scene(lab, work, "s6a", slow("slow-a") / "bundle.json", slow("slow-a")):
        return
    first = Explicit(lab, 48, [(p["x"], p["y"]) for p in FRAMES[48]["pixelProbes"]])
    time.sleep(0.4)
    gen_first = (directed(lab.viewport()).get("slots") or {}).get("generation")
    refused_before = (directed(lab.viewport()).get("refused") or {}).get("stale_asset_states", 0)
    # The new project asks for the same slot names, with its own slow files, while the old loads are still in flight.
    if not open_scene(lab, work, "s6b", slow("slow-b") / "bundle.json", slow("slow-b"), show=False):
        return
    second = Explicit(lab, 11, [(p["x"], p["y"]) for p in FRAMES[11]["pixelProbes"]])
    old = first.answer(40)
    check("the old project's explicit frame is answered across the switch", old is not None)
    if old:
        check("it is typed `cancelled`, not a picture of the old project", not old[0] and error_code(old) == "cancelled", old)
        note("old_explicit_across_switch", {"ok": old[0], "error": error_code(old)})
    new = second.answer(40)
    check("the new project's explicit frame is answered", new is not None)
    if new:
        check("it is the FRAME, not failed by the old project's `cancelled` loads", new[0], new)
        if new[0]:
            mismatches = h.compare_probes(FRAMES[11]["pixelProbes"], new[1]["frame"].get("probes", []), PIXEL_TOL)
            check("and it matches the independent model", not mismatches, mismatches[:3])
    state = lab.viewport()
    d = directed(state)
    refused = (d.get("refused") or {}).get("stale_asset_states", 0)
    note("refused_stale_asset_states", refused - refused_before)
    gen_second = (d.get("slots") or {}).get("generation")
    check("the new project's resources have a newer generation than the old one", gen_first is not None and gen_second is not None and gen_second > gen_first, (gen_first, gen_second))
    check("the replaced loads of the old project were refused, not applied", refused - refused_before >= 1, (refused_before, refused))
    check("the refusal is by identity: the new project's loads were stamped, so a late answer of the old request counts for nothing", d.get("asset_identity") is True and (d.get("slots") or {}).get("identified_loads", 0) >= 1, d.get("slots"))
    check("the new project's frame drew with the old project's resources released (runtime holds only the new ones)", cache_of(state).get("scene_resources") == SCENE_ASSETS, cache_of(state))
    ok, body = lab.op({"op": "monitor.show_scene", "scene": "proof"})
    check("the monitor is shown for the new project", ok, body)
    check("the monitor converges on the new project's playhead frame", bool(settle(lab, 0, 60)), lab.viewport().get("directed"))
    prove_current(lab, 0)
    note("footprint", budget(lab, "after the switch"))


def s7_loss_then_switch(lab, work, env):
    scenario("S7 context lost, project replaced before the restore")
    if not open_scene(lab, work, "s7", CORPUS / "bundle.json", CORPUS):
        return
    check("frame 0 converges", bool(settle(lab, 0, 30)))
    before = lab.viewport()
    generation_before = cache_of(before).get("context_generation", 0)
    lab.seek(95)
    check("the loss is seen by the host", bool(wait_for(lambda: directed(lab.viewport()).get("suspended"), 10)), directed(lab.viewport()))
    if not open_scene(lab, work, "s7b", CORPUS / "bundle.json", CORPUS):
        return
    state = settle(lab, 0, 60)
    check("the new project draws once the context is back", bool(state), lab.viewport().get("directed"))
    if state:
        prove_current(lab, 0)
        check("exactly one real loss happened", cache_of(state).get("context_generation", 0) - generation_before == 1, (generation_before, cache_of(state).get("context_generation")))
        lab.seek(12)
        check("and follows the playhead", bool(settle(lab, 12, 30)))
        note("footprint", budget(lab, "after loss+switch"))


def s8_never_restored(lab, work, env):
    scenario("S8 context never restored")
    if not open_scene(lab, work, "s8", CORPUS / "bundle.json", CORPUS):
        return
    check("frame 0 converges", bool(settle(lab, 0, 30)))
    lab.seek(70)
    suspended = wait_for(lambda: (lambda s: s if directed(s).get("suspended") else None)(lab.viewport()), 10)
    check("the host knows the context is lost", bool(suspended), directed(lab.viewport()))
    state = lab.viewport()
    check("the monitor says so instead of freezing on the last picture", str(state.get("headline", "")).startswith("Falha"), state.get("headline"))
    started = time.time()
    ok, body = lab.op({"op": "scene.render", "scene": "proof", "frame": 12, "width": WIDTH, "height": HEIGHT})
    check("an explicit frame is refused at once with a typed `context_lost`", not ok and (body or {}).get("code") == "context_lost" and time.time() - started < 5, (ok, body))
    lab.seek(12)
    check("the reload recovers", lab.invoke("monitor.reload", "invoke").get("ok"))
    state = settle(lab, 12, 60)
    check("after the reload the monitor draws the playhead frame", bool(state), lab.viewport().get("directed"))
    if state:
        prove_current(lab, 12)
        note("footprint", budget(lab, "after the reload"))


def s9_loss_then_reload(lab, work, env):
    scenario("S9 context lost, page reloaded before the restore")
    lab.seek(80)
    check("the loss is seen by the host", bool(wait_for(lambda: directed(lab.viewport()).get("suspended"), 10)), directed(lab.viewport()))
    lab.seek(12)
    check("the reload is accepted", lab.invoke("monitor.reload", "invoke").get("ok"))
    state = settle(lab, 12, 60)
    check("the monitor converges on the playhead frame in the new page", bool(state), lab.viewport().get("directed"))
    if state:
        prove_current(lab, 12)
        note("footprint", budget(lab, "after loss+reload"))
    time.sleep(2.0)
    state = lab.viewport()
    check("the old page's late restore changed nothing", converged(state, 12), directed(state))


def s10_cycles(lab, work, env):
    scenario("S10 six losses in a row")
    baseline = env.get("baseline") or {}
    start = lab.viewport()
    start_losses = directed(start).get("context_losses", 0)
    for frame in range(81, 87):
        lab.seek(frame)
        state = settle(lab, frame, 45)
        if not check(f"frame {frame} converged after its loss", bool(state), lab.viewport().get("directed")):
            return
    end = lab.viewport()
    check("exactly six losses happened", directed(end).get("context_losses", 0) - start_losses == 6, (start_losses, directed(end).get("context_losses")))
    prove_current(lab, 86)
    foot = budget(lab, "after six losses")
    note("footprint", foot)
    for key in ("gl_textures", "texture_entries"):
        limit = (baseline.get(key) or 0) + 2
        check(f"{key} did not grow across the losses ({foot.get(key)} <= {limit})", (foot.get(key) or 0) <= limit, (baseline.get(key), foot.get(key)))


SCENARIOS = [
    ("s0", s0_baseline),
    ("s1", s1_loss_mid_monitor_frame),
    ("s2", s2_hidden_explicit_loss),
    ("s3", s3_hide_show),
    ("s4", s4_reload_with_explicit_pending),
    ("s5", s5_resource_swap),
    ("s6", s6_project_switch),
    ("s7", s7_loss_then_switch),
    ("s8", s8_never_restored),
    ("s9", s9_loss_then_reload),
    ("s10", s10_cycles),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", default=str(ROOT / "target" / "debug" / "tessel-editor"))
    parser.add_argument("--out")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--only", help="comma separated: " + ",".join(name for name, _ in SCENARIOS) + " (s0 always runs; s2-s3 need s1's playhead, s9/s10 need a project)")
    args = parser.parse_args()
    if not os.path.exists(args.binary):
        print(f"missing {args.binary}; build it with python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py build -p tessel-editor --locked")
        return 2
    for needed in ("runtime.js", "index.html"):
        if not (ROOT / "runtime" / "dist" / needed).exists():
            print("runtime/dist is not built; run ./scripts/runtime setup")
            return 2
    if h.free_gib("/tmp") < MIN_FREE_GIB:
        print(f"only {h.free_gib('/tmp'):.1f} GiB free under /tmp; this proof needs {MIN_FREE_GIB}")
        return 2
    wanted = set(args.only.split(",")) if args.only else None

    work = tempfile.mkdtemp(prefix="te-r8-", dir="/tmp")
    global WORK
    WORK = work
    report["binary"] = h.binary_info(args.binary)
    report["git"] = h.git_state()
    lab = None
    env = {}
    try:
        slow_corpus(work, "slow-a")
        slow_corpus(work, "slow-b")
        runtime_dir = chaos_runtime(work)
        lab = Lab(args.binary, work, {"TESSEL_VIEWPORT_INSPECT": "1", "TESSEL_RUNTIME_DIR": runtime_dir})
        lab.wait_ready()
        lab.invoke("window.activate")
        time.sleep(0.8)
        report["page"] = lab.viewport().get("page")
        current["name"] = "setup"
        check("the editor runs the scratch page (chaos installed)", str(report["page"]).startswith(runtime_dir), report["page"])
        caps = lab.viewport().get("capabilities") or {}
        report["renderer"] = caps.get("renderer")
        for name, function in SCENARIOS:
            if name != "s0" and wanted is not None and name not in wanted:
                continue
            function(lab, work, env)
        current["name"] = "end"
        final = lab.viewport()
        check("the editor is still ready and unfailed at the end", final["phase"] == "ready" and not (final["session"].get("error")), final["session"].get("error"))
        report["final"] = {"directed": directed(final), "ui": final.get("ui")}
        used = h.tree_bytes(work) / 1024 ** 3
        report["artifacts_gib"] = round(used, 3)
        check(f"own artifacts stayed under {BUDGET_GIB} GiB ({used:.3f})", used < BUDGET_GIB)
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
            report["checks_total"] = len(report["checks"])
            report["checks_failed"] = len(failures)
            if args.out:
                pathlib.Path(args.out).write_text(json.dumps(report, indent=1))
            print(json.dumps(report))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
