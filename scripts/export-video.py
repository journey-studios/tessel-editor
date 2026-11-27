#!/usr/bin/env python3
"""Compatibility entry point for video export with an already built renderer."""

import sys

from tessel_workflows import ROOT, main


if __name__ == "__main__":
    binary = "tessel-render.exe" if sys.platform == "win32" else "tessel-render"
    raise SystemExit(main(["export", *sys.argv[1:]], default_renderer=ROOT / "target" / "debug" / binary))
