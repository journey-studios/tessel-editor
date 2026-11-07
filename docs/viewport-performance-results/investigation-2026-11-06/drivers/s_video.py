import sys, argparse, shutil, os; sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from scen import *

NOT_READY_CODES = ("media_preparing", "cancelled")


def wait_for(pred, seconds, step=0.3):
    t = time.time()
    while time.time() - t < seconds:
        v = pred()
        if v: return v
        time.sleep(step)


def work_free_gib():
    """Free space of the volume that holds the editor home, project, media and proxies: WORK, which
    TESSEL_PERF_INV_WORK can move away from /tmp."""
    return shutil.disk_usage(WORK).free / 2**30


def main(argv=None, free_gib=None):
    free_gib = free_gib or work_free_gib
    ap = argparse.ArgumentParser()
    ap.add_argument("label"); ap.add_argument("--runtime", default="instr"); ap.add_argument("--binary", default="release")
    ap.add_argument("--env", default=""); ap.add_argument("--counts", default="1,2,4,8"); ap.add_argument("--assets", type=int, default=4)
    ap.add_argument("--policies", default="original,auto"); ap.add_argument("--frames", type=int, default=96)
    ap.add_argument("--mix", action="store_true")
    a = ap.parse_args(argv)
    env = dict(kv.split("=", 1) for kv in a.env.split(",") if kv)
    out = {"label": a.label, "args": vars(a), "runs": []}

    def make_editor():
        return Editor(a.label, binary=DEBUG if a.binary == "debug" else RELEASE, runtime=a.runtime, env=env)

    def body(ed):
        def op(o, timeout=300):
            """Every op must succeed: a rejected clip.add used to be printed and ignored, and the run then measured an
            empty timeline under the label of N clips."""
            r = ed.op(o, timeout)
            if not r.get("ok"):
                print("OP FAIL", o, json.dumps(r)[:300], flush=True)
                raise DriverError(f"op {o.get('op')} rejected", r)
            return r

        def media_assets():
            reply = ed.op({"op": "media.status"})
            if not reply.get("ok"):
                raise DriverError("op media.status rejected", reply)
            return (reply.get("data") or {}).get("assets") or []

        ed.wait_socket(); require_ok(ed.activate(), "window.activate")
        ws = f"{ed.home}/project"; doc = f"{ws}/v.tessel.json"
        if a.mix:
            os.makedirs(ws, exist_ok=True)
            shutil.copytree(f"{WORK}/proj/h", ws + "/h", dirs_exist_ok=True)
            doc = ws + "/h/tessel-splat-h.tessel.json"
            op({"op": "project.open", "path": doc})
            length = 600
        else:
            length = 144
            op({"op": "project.create", "name": "Video perf", "sequence": "main", "rate": 24, "width": 1280, "height": 720, "length": length, "dir": ws})
            op({"op": "project.save", "path": doc})
        n0 = len(media_assets())
        paths = [f"{WORK}/videos/seg{i}.mp4" for i in range(a.assets)]
        op({"op": "media.import", "paths": paths})
        t_imp = time.time()
        imported = wait_for(lambda: len(media_assets()) >= n0 + a.assets, 300)
        print("imported", imported is not None, round(time.time() - t_imp, 1), "s", flush=True)
        if not imported:
            raise DriverError(f"media import timed out: {len(media_assets()) - n0} of {a.assets} assets after 300 s")
        new_assets = media_assets()[n0:]
        assets = [x["id"] for x in new_assets]
        if not assets:
            raise DriverError("media import reported no new asset")
        not_ok = [x["id"] for x in new_assets if x.get("state") != "ok"]
        if not_ok:
            raise DriverError(f"imported assets not in state ok: {not_ok}")
        print("assets", assets, json.dumps(new_assets[:1])[:400], flush=True)
        tracks = 0
        counts = [int(x) for x in a.counts.split(",")]
        base_start = 40 if a.mix else 0
        for target in counts:
            while tracks < target:
                i = tracks
                op({"op": "track.add", "id": f"vt{i}", "name": f"V{i}", "kind": "video"})
                op({"op": "clip.add", "id": f"vc{i}", "track": f"vt{i}", "asset": assets[i % len(assets)], "start": base_start, "duration": a.frames + 8, "source_start": {"num": ((i * 10) % 30), "den": 24}})
                tracks += 1
            op({"op": "project.save", "path": doc})
            free = free_gib()
            if free < 0.6:
                print('DISK LOW', free, flush=True)
                out.setdefault("errors", []).append({"kind": "disk_low", "videos": target, "free_gib": round(free, 3), "error": "aborted: free disk below 0.6 GiB; this count and the following were not measured"})
                break
            for policy in a.policies.split(","):
                ctx = {"videos": target, "policy": policy}
                op({"op": "preview.policy", "mode": policy})

                def ready():
                    r = ed.op({"op": "render.frame", "frame": base_start + 10}, 300)
                    if r.get("ok"):
                        return True
                    code = (r.get("error") or {}).get("code")
                    if code not in NOT_READY_CODES:
                        raise DriverError(f"render.frame failed with {code}", r)
                    return None
                t = time.time()
                errors_before = len(out.get("errors", []))
                rd = guarded(out, {**ctx, "stage": "readiness"}, lambda: wait_for(ready, 900, 0.5))
                prep = time.time() - t
                if rd is None:
                    if len(out.get("errors", [])) == errors_before:  # timed out without an exception
                        out.setdefault("errors", []).append({**ctx, "stage": "readiness", "kind": "not_ready", "ready_wait_s": round(prep, 1), "error": "media never became ready (render.frame not ok within 900 s); playback not measured"})
                    continue
                seek_reply = ed.seek(base_start)
                if not seek_reply.get("ok"):
                    out.setdefault("errors", []).append({**ctx, "stage": "seek", "kind": "seek_rejected", "reply": seek_reply, "error": f"the editor rejected the seek to frame {base_start}; playback not measured"})
                    continue
                if not ed.settle(120):
                    out.setdefault("errors", []).append({**ctx, "stage": "settle", "kind": "not_settled", "error": "the monitor did not settle at the playhead within 120 s; playback not measured"})
                    continue
                time.sleep(0.5)
                for rep in range(2):
                    def one():
                        res = playback_run(ed, base_start, a.frames, f"{target}-{policy}-{rep}")
                        an = analyze_play(res)
                        pb = res.get("playback_report") or {}
                        shown = an.get("distinct_frames_shown")
                        entry = {"videos": target, "policy": policy, "rep": rep, "ready": True, "ready_wait_s": round(prep, 1), "playback": {k: pb.get(k) for k in ("elapsed_ms", "frame", "lag_frames", "skipped_frames")},
                                 "distinct_shown": shown, "asked": (pb.get("frame") or 0) - base_start, "dropped_pct": round(100 * (1 - (shown or 0) / max(1, (pb.get("frame") or base_start + a.frames) - base_start)), 1),
                                 "header_render_ms": an.get("render_ms(header)"), "interval": an.get("frame_interval_ms(host done)"), "page_total": an.get("page_total_ms"),
                                 "gpu": res.get("gpu_util"), "gpui_rps": an.get("gpui_renders_per_s"), "layers": an.get("scene_layers_per_frame"), "stats_delta": res.get("stats_delta"),
                                 "host": {k: an.get(k) for k in an if k.startswith("host.")}}
                        out["runs"].append(entry)
                        print(a.label, target, policy, rep, "shown", shown, "/", entry["asked"], "drop", entry["dropped_pct"], "hdr", an.get("render_ms(header)", {}).get("p50"), an.get("render_ms(header)", {}).get("p95"), "iv", (an.get("frame_interval_ms(host done)") or {}).get("p50"), (an.get("frame_interval_ms(host done)") or {}).get("p95"), "gpu", res["gpu_util"]["mean"], "skipped", pb.get("skipped_frames"), flush=True)
                    guarded(out, {**ctx, "rep": rep, "stage": "playback"}, one)
        out["memory"] = ed.memory()
        out["disk_home_mb"] = subprocess.run(["du", "-sm", ed.home], capture_output=True, text=True).stdout.split()[0]

    return run_session(a.label, out, make_editor, body)


if __name__ == "__main__":
    sys.exit(main())
