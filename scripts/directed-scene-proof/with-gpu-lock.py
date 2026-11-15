#!/usr/bin/env python3
"""Run a command under the shared local GPU bench lock, without a shell."""
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import portable  # noqa: E402  (BL168.8: `flock` on Unix, `msvcrt.locking` and the temporary folder on Windows)


def main():
    if len(sys.argv) < 2:
        print("usage: with-gpu-lock.py COMMAND [ARG ...]", file=sys.stderr)
        return 2
    with open(portable.temp_path("tessel-scene-gpu.lock"), "a") as lock:
        portable.lock_file(lock)
        # Windows starts no `#!` script: `portable.program` runs one under its interpreter there (unchanged on Unix).
        return subprocess.call(portable.program(*sys.argv[1:]))


if __name__ == "__main__":
    raise SystemExit(main())
