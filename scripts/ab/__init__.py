"""The shared core of the A/B runners: `real-latency-ab.py`, `first-use-ab.py`, `viewport-transit-probe.py`, `runtime-bundle-ab.py`
and `playback-poll-ab.py`.

A runner is one experiment: its arms and their environment, what makes a session valid, its verdict and its tables. Everything the
experiments have in common is here, once, and the runners are thin mains that hand their experiment to it. Nothing is copied between runners
and no runner imports another.

    project    what a project document says; the identity of a run (hashes, git state)
    stats      nearest-rank percentiles, load class, memory
    play       the pictures of a play, the gaps between them, where a gap sits, what the host was doing around it
    transit    a late picture's time split on one clock
    session    one fresh editor and the raw record of what it did
    validity   whether a recorded session measured what it claims, and what a report may say about a failure
    charts     SVGs of the gaps
    batch      arm order, prerequisites, manifest, the run loop, loading a saved batch, publishing the analysis
    pixels     the pixel proof of the synthetic project, with the environment of each arm handed in

The editor lab itself (`Lab`, the bench locks, the synthetic project, the pixel session) is `scripts/scene-warm-proof.py`; `warm_proof()` is the
only place that loads it.
"""
import importlib.util
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent


def load_module(name, path):
    """Import a script whose file name has a hyphen. Loaded once per name."""
    if name not in sys.modules:
        spec = importlib.util.spec_from_file_location(name, path)
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    return sys.modules[name]


def warm_proof():
    """`scripts/scene-warm-proof.py`: the editor lab every experiment drives."""
    return load_module("scene_warm_proof", ROOT / "scripts" / "scene-warm-proof.py")


def cadence():
    """`scripts/playback-cadence.py`: the reader of the host trace's playback clock."""
    return load_module("playback_cadence", ROOT / "scripts" / "playback-cadence.py")
