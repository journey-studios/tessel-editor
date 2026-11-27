"""File-based editing workflows over the editor's native JSONL backends."""

import argparse
import json
from pathlib import Path
import signal
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[1]


class NativeError(RuntimeError):
    def __init__(self, operation, error):
        super().__init__(f"{operation}: {json.dumps(error, ensure_ascii=False)}")
        self.code = error["code"]


def request(process, op, **fields):
    process.stdin.write(json.dumps({"op": op, **fields}) + "\n")
    process.stdin.flush()
    line = process.stdout.readline()
    if not line:
        raise RuntimeError(f"Backend closed stdout during {op}")
    reply = json.loads(line)
    if not reply["ok"]:
        raise NativeError(op, reply["error"])
    return reply["result"]


def run_job(process, op, **fields):
    cancelled = False

    def interrupt(_signal, _frame):
        # Finish the current JSONL exchange before asking the native job to cancel.
        nonlocal cancelled
        cancelled = True

    previous_handler = signal.signal(signal.SIGINT, interrupt)
    try:
        job = request(process, op, **fields, wait=False)["job"]
        last_progress = None
        cancel_sent = False
        late_cancel = False
        while True:
            if cancelled and not cancel_sent:
                try:
                    request(process, "job.cancel", job=job)
                except NativeError as error:
                    # The job may have published just before the cancel request arrived.
                    if error.code != "job_finished":
                        raise
                    late_cancel = True
                cancel_sent = True
            info = request(process, "job.get", job=job)
            progress = (info["state"], info["stage"], info["done"], info["total"])
            if progress != last_progress:
                state, stage, done, total = progress
                print(f"{state} · {stage} · {done}/{total}", file=sys.stderr, flush=True)
                last_progress = progress
            if info["state"] == "succeeded":
                return info["result"], late_cancel
            if info["state"] == "failed":
                raise NativeError(op, info["error"])
            if info["state"] == "cancelled":
                raise KeyboardInterrupt
            time.sleep(0.25)
    finally:
        signal.signal(signal.SIGINT, previous_handler)


def backend_command(*args):
    return ["sh", str(ROOT / "scripts" / "tessel"), *args]


def with_backend(command, action):
    # SIGINT goes to the CLI so it can cancel the job before its backend closes.
    process_group = (
        {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP}
        if sys.platform == "win32"
        else {"start_new_session": True}
    )
    with subprocess.Popen(
        command,
        cwd=ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        **process_group,
    ) as process:
        result, late_cancel = action(process)
    # JSONL exits 1 after any refused request, including a cancel sent after publication.
    if process.returncode != 0 and not (process.returncode == 1 and late_cancel):
        raise RuntimeError(f"Backend exited with status {process.returncode}")
    return result


def export(args):
    fields = {
        "destination": str(args.destination.resolve()),
        "replace": args.replace,
    }
    if args.sequence is not None:
        fields["sequence"] = args.sequence
    if args.preset is not None:
        fields["preset"] = args.preset
    elif args.clip is None:
        fields["preset"] = "fullhd"
    if args.range is not None:
        fields["range"] = args.range
    if args.clip is not None:
        fields["camera_preview"] = args.clip
    project = str(args.project.resolve())

    def run(process):
        request(process, "project.open", path=project)
        return run_job(process, "export.start", **fields)

    if args.renderer is None:
        command = backend_command("render", "--gl", args.gl)
    else:
        command = [str(args.renderer.resolve()), "--gl", args.gl]
    return with_backend(command, run)


def setup(args):
    if not args.check:
        subprocess.run(
            ["sh", str(ROOT / "scripts" / "runtime"), "setup"],
            cwd=ROOT,
            stdout=sys.stderr,
            check=True,
        )
    report = subprocess.check_output(
        backend_command("render", "--check", "--gl", args.gl),
        cwd=ROOT,
        encoding="utf-8",
    )
    return json.loads(report)


def package(args):
    project = str(args.project.resolve())
    fields = {
        "path": str(args.destination.resolve()),
        "format": args.format,
        "replace": args.replace,
    }
    if args.sequence:
        fields["sequences"] = args.sequence

    def run(process):
        request(process, "project.open", path=project)
        return run_job(process, "project.package", **fields)

    return with_backend(backend_command("media", "--empty"), run)


def restore(args):
    return with_backend(
        backend_command("media", "--empty"),
        lambda process: run_job(
            process,
            "project.restore",
            path=str(args.package.resolve()),
            folder=str(args.folder.resolve()),
            open=False,
        ),
    )


def parser(default_renderer):
    root = argparse.ArgumentParser(prog="./scripts/tessel")
    commands = root.add_subparsers(dest="command", required=True)
    video = commands.add_parser("export", help="Export video with progress")
    video.add_argument("project", type=Path)
    video.add_argument("destination", type=Path)
    video.add_argument("--renderer", type=Path, default=default_renderer)
    video.add_argument("--preset", help="Export preset ID (fullhd by default for whole videos and ranges)")
    video.add_argument("--sequence", help="Sequence ID")
    video.add_argument("--gl", choices=("gpu", "swiftshader"), default="gpu")
    interval = video.add_mutually_exclusive_group()
    interval.add_argument("--range", type=int, nargs=2, metavar=("START", "END"), help="Half-open sequence frame range")
    interval.add_argument("--clip", help="Preview the saved camera push-in of this clip ID")
    video.add_argument("--replace", action="store_true", help="Replace the destination on success")
    video.set_defaults(run=export)

    deps = commands.add_parser("setup", help="Prepare and check the local rendering stack")
    deps.add_argument("--check", action="store_true", help="Check the current installation without running pnpm")
    deps.add_argument("--gl", choices=("gpu", "swiftshader"), default="gpu")
    deps.set_defaults(run=setup)

    archive = commands.add_parser("package", help="Pack the editable project and its media")
    archive.add_argument("project", type=Path)
    archive.add_argument("destination", type=Path)
    archive.add_argument("--format", choices=("archive", "directory"), default="archive")
    archive.add_argument("--sequence", action="append", help="Sequence ID to include (repeat for multiple IDs)")
    archive.add_argument("--replace", action="store_true", help="Replace the destination on success")
    archive.set_defaults(run=package)

    extract = commands.add_parser("restore", help="Verify and restore a project package")
    extract.add_argument("package", type=Path)
    extract.add_argument("folder", type=Path)
    extract.set_defaults(run=restore)
    return root


def main(argv=None, *, default_renderer=None):
    args = parser(default_renderer).parse_args(argv)
    try:
        result = args.run(args)
    except (OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as error:
        print(error, file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
