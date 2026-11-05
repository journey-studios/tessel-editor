#!/usr/bin/env python3
"""Run a command under the shared local GPU bench lock, without a shell."""
import fcntl
import subprocess
import sys


def main():
    if len(sys.argv) < 2:
        print("usage: with-gpu-lock.py COMMAND [ARG ...]", file=sys.stderr)
        return 2
    with open("/tmp/tessel-scene-gpu.lock", "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        return subprocess.call(sys.argv[1:])


if __name__ == "__main__":
    raise SystemExit(main())
