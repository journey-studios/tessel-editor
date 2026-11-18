#!/usr/bin/env python3
"""Export a Tessel project or frame range with progress from the native JSONL job."""

import argparse
import json
from pathlib import Path
import signal
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "target" / "debug" / (
    "tessel-render.exe" if sys.platform == "win32" else "tessel-render"
)


def request(renderer, op, **fields):
    renderer.stdin.write(json.dumps({"op": op, **fields}) + "\n")
    renderer.stdin.flush()
    line = renderer.stdout.readline()
    if not line:
        raise RuntimeError(f"Renderer closed stdout during {op}")
    reply = json.loads(line)
    if not reply["ok"]:
        raise RuntimeError(f'{op}: {json.dumps(reply["error"], ensure_ascii=False)}')
    return reply["result"]


def export(renderer, args):
    request(renderer, "project.open", path=str(args.project.resolve()))
    fields = {
        "preset": args.preset,
        "destination": str(args.destination.resolve()),
        "replace": args.replace,
        "wait": False,
    }
    if args.range is not None:
        fields["range"] = args.range
    job = request(renderer, "export.start", **fields)["job"]
    last_progress = None
    cancel_requested = False

    def on_interrupt(_signal, _frame):
        # Keep JSONL request/response pairs intact before sending the native cancellation.
        nonlocal cancel_requested
        cancel_requested = True

    previous_handler = signal.signal(signal.SIGINT, on_interrupt)
    try:
        while True:
            if cancel_requested:
                request(renderer, "job.cancel", job=job)
                final = request(renderer, "job.wait", job=job)
                if final["state"] == "succeeded":
                    return final["result"]
                if final["state"] == "failed":
                    raise RuntimeError(json.dumps(final["error"], ensure_ascii=False))
                raise KeyboardInterrupt
            info = request(renderer, "job.get", job=job)
            progress = (info["state"], info["stage"], info["done"], info["total"])
            if progress != last_progress:
                state, stage, done, total = progress
                print(f"{state} · {stage} · {done}/{total}", file=sys.stderr, flush=True)
                last_progress = progress
            if info["state"] == "succeeded":
                return info["result"]
            if info["state"] in {"failed", "cancelled"}:
                raise RuntimeError(json.dumps(info["error"], ensure_ascii=False))
            time.sleep(5)
    finally:
        signal.signal(signal.SIGINT, previous_handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("project", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    parser.add_argument("--preset", default="fullhd")
    parser.add_argument("--gl", choices=("gpu", "swiftshader"), default="gpu")
    parser.add_argument("--range", type=int, nargs=2, metavar=("START", "END"))
    parser.add_argument("--replace", action="store_true")
    args = parser.parse_args()
    if not args.renderer.is_file():
        parser.error("Build tessel-render with ./scripts/cargo build --locked -p tessel-editor-render --bin tessel-render")
    process_group = (
        {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP} if sys.platform == "win32"
        else {"start_new_session": True}
    )
    try:
        with subprocess.Popen(
            [str(args.renderer.resolve()), "--gl", args.gl],
            cwd=ROOT,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            **process_group,
        ) as renderer:
            result = export(renderer, args)
        if renderer.returncode:
            raise RuntimeError(f"Renderer exited with status {renderer.returncode}")
    except (OSError, RuntimeError) as error:
        print(error, file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
