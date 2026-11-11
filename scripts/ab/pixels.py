"""The pixel proof of the synthetic project: explicit frames are byte-identical across the arms, hot and cold, in the real window.

A FUNCTIONAL proof (hashes of pixels, not a time), so it neither takes the GPU bench locks nor runs below the disk floor; its sessions use their own
HOME and socket. The environment of each arm is handed in, and so is how a refused request is retried: no module is patched."""
import json
import os
import pathlib
import shutil
import time

from ab import warm_proof
from ab.session import MIN_FREE_GIB_START, workdir
from ab.validity import published_error

CUTS, CUT_FRAMES, SPLATS = 5, 72, 200000


def retrying_on_cancel(W, refused, label):
    """`retry_while_preparing`, and a frame request replaced by a newer one (the monitor's own refine) is asked again: the explicit render is idempotent.
    Every refusal stays in `refused` as `{arm, error: {code, kind}}` (a race of the harness seen in every arm, not a pixel result; the text of an
    answer can carry a path and `pixels.json` is a report)."""
    def retry(call, *rest, **named):
        answer = W.retry_while_preparing(call, *rest, **named)
        for _ in range(4):
            if not (isinstance(answer, dict) and not answer.get("ok")):
                break
            refused.append({"arm": label, "error": published_error(answer)})
            if ((answer.get("error") or {}).get("code")) != "cancelled":
                break
            time.sleep(1.0)
            answer = W.retry_while_preparing(call, *rest, **named)
        return answer
    return retry


def one_session(W, binary, document, label, env, order, frames, work, refused):
    """One editor, one (arm, order). A session that raised (the editor never opened, the page never came) is a FAILED RESULT of the proof, not the end of
    it: the sessions that ran stay on record. Its error, whether raised or answered, is published as `{code, kind}`, never as its text."""
    try:
        session = W.pixel_session(str(binary), document, "warm", {**W.env_for("warm", True), **env}, order, frames, CUT_FRAMES, work,
                                  retry=retrying_on_cancel(W, refused, label))
    except Exception as error:  # the lab's own failures are of any class
        session = {"arm": "warm", "order": order, "hashes": {}, "monitor": {}, "error": repr(error)}
    if session.get("error"):
        session["error"] = published_error(session["error"])
    session["label"] = label
    return session


def run_pixels(binary, out, arms, more_verdicts=None, extra=None):
    """`arms` is [(label, env)]: each runs the project hot (playing into the cut) and cold (seeking to it). Prints the verdicts, writes `pixels.json`
    in `out` and returns the exit code (2 BLOCKED by the disk, 1 a failed verdict or a session that raised, else 0)."""
    W = warm_proof()
    if W.free_gib() < MIN_FREE_GIB_START:
        print(f"BLOCKED: {W.free_gib():.1f} GiB free")
        return 2
    refused, results = [], []
    work = workdir("abpx-")
    try:
        document = W.build_project(str(binary), os.path.join(work, "build"), SPLATS, CUTS, CUT_FRAMES)
        if not document:
            return 1
        frames = sorted({CUT_FRAMES + 5, 2 * CUT_FRAMES + 5, (CUTS - 1) * CUT_FRAMES + 5, CUT_FRAMES + 40})
        for label, env in arms:
            for order in ("hot", "cold"):
                results.append(one_session(W, binary, document, label, env, order, frames, os.path.join(work, f"px{len(results)}"), refused))
    finally:
        shutil.rmtree(work, ignore_errors=True)
    verdicts = W.pixel_verdicts(results, frames) + (more_verdicts(results) if more_verdicts else [])
    for name, ok, detail in verdicts:
        print(("PASS " if ok else "FAIL ") + name + ("" if ok else f"  [{str(detail)[:200]}]"))
    out = pathlib.Path(out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "pixels.json").write_text(json.dumps({**(extra or {}), "frames": frames, "sessions": results, "refused_requests": refused,
                                                  "verdicts": [{"check": name, "ok": ok} for name, ok, _ in verdicts]}, indent=2))
    for item in refused:
        print("REFUSED", item["arm"], item["error"]["code"])
    for session in results:
        if session.get("error"):
            print("SESSION FAILED", session["label"], session["order"], session["error"]["kind"], session["error"]["code"])
    return 1 if any(not ok for _, ok, _ in verdicts) or any(s.get("error") for s in results) else 0
