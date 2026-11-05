#!/usr/bin/env python3
"""Synthetic corpus for the directed-scene end-to-end proof (BL019, front P).

    python3 scripts/directed-scene-proof/corpus.py            # (re)writes ./corpus
    python3 scripts/directed-scene-proof/corpus.py --check    # fails if ./corpus is not exactly what this script writes
    python3 scripts/directed-scene-proof/corpus.py --out DIR  # writes elsewhere (used by the harness for the determinism check)

Provenance (also written to corpus/provenance.json):

  * every picture is drawn by this file from scratch with flat colours and a binary barcode;
    no photograph, no third-party asset, nothing copied from the origin or from a private project;
  * PNG files use *stored* deflate blocks, so the bytes do not depend on the zlib build;
  * the bundle follows render-bundle 1.0 with REAL SHA-256 hashes and real dimensions, so the
    resource transport (`directed_resources`) accepts it. The oracle fixtures use fake hashes
    and cannot be materialised;
  * `expected.json` comes from `model.py`, an independent model checked against the origin
    oracle (`model.spec.py`). It never contains output of the destination (core, runtime, render).
"""
import argparse
import copy
import hashlib
import importlib.util
import json
import pathlib
import platform
import shutil
import struct
import sys
import tempfile
import zlib

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("model", HERE / "model.py")
model = importlib.util.module_from_spec(spec)
spec.loader.exec_module(model)

CORPUS_VERSION = 1
WIDTH, HEIGHT = 320, 180
EDIT_FRAMES = 96
# Boundaries on purpose: before/at the start of the event (12), its end (36), the shot cut (47/48),
# the camera segment joins (31/32 and 47/48 of the clip) and the last frame.
FRAMES = [0, 11, 12, 20, 31, 36, 47, 48, 60, 95]
# Non-monotonic: backward jumps, a repeat, first and last frame, across both shots.
SEEK_ORDER = [95, 0, 48, 12, 47, 36, 11, 60, 20, 12, 31, 0]
# Tolerances are fixed here, before any destination output exists to compare with.
TOLERANCES = {
    "camera": {"absolute": 1e-9, "relative": 1e-12, "rule": "abs(a-e) <= absolute + relative*abs(e)"},
    "worldMatrix": {"absolute": 1e-10, "relative": 1e-12, "rule": "abs(a-e) <= absolute + relative*abs(e)"},
    "pixel": {"maxChannelDelta": 6, "rule": "max(|r-r'|,|g-g'|,|b-b'|) <= maxChannelDelta, sRGB 8-bit",
              "why": "flat blocks only (screen neighbours ±2 px agree), so texture filtering and "
                     "sRGB/linear round trips can move a channel by a few units, never change the layer"},
    "gestureEdit": {"absolute": 1e-12, "relative": 0.0,
                    "rule": "an edited value is stored as sent: exact to binary64 round trip"},
}

# ---------------------------------------------------------------------------------- PNG output


def _chunk(kind, data):
    body = kind + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


def _stored_zlib(raw):
    out = bytearray(b"\x78\x01")
    position = 0
    while True:
        block = raw[position:position + 65535]
        position += len(block)
        last = position >= len(raw)
        out += struct.pack("<BHH", 1 if last else 0, len(block), len(block) ^ 0xFFFF) + block
        if last:
            break
    return bytes(out) + struct.pack(">I", zlib.adler32(raw) & 0xFFFFFFFF)


def png_bytes(rows):
    height, width = len(rows), len(rows[0])
    raw = b"".join(b"\x00" + bytes(channel for pixel in row for channel in pixel) for row in rows)
    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + _chunk(b"IHDR", header) + _chunk(b"IDAT", _stored_zlib(raw)) + _chunk(b"IEND", b"")


# ---------------------------------------------------------------------------------- textures
BLACK, WHITE = (0, 0, 0, 255), (255, 255, 255, 255)


def blank(width, height, pixel):
    return [[pixel for _ in range(width)] for _ in range(height)]


def fill(rows, x0, y0, x1, y1, pixel):
    for y in range(y0, y1):
        for x in range(x0, x1):
            rows[y][x] = pixel


def barcode(rows, x0, y0, cell, height, bits, value):
    """`bits` cells, least-significant bit first, white = 1 on a black strip: the picture says which
    asset it is even when only a decoded PNG is available."""
    fill(rows, x0, y0, x0 + cell * bits, y0 + height, BLACK)
    for bit in range(bits):
        if (value >> bit) & 1:
            fill(rows, x0 + bit * cell, y0, x0 + (bit + 1) * cell, y0 + height, WHITE)


def texture_sky():
    rows = blank(64, 36, WHITE)
    bands = [(200, 60, 60), (60, 160, 70), (60, 90, 200), (220, 200, 70)]
    for index, (r, g, b) in enumerate(bands):
        fill(rows, index * 16, 0, (index + 1) * 16, 36, (r, g, b, 255))
    barcode(rows, 0, 0, 8, 3, 8, 1)
    return rows


def texture_mid():
    # Straight alpha: the transparent frame keeps a magenta RGB on purpose. A premultiplied or
    # wrongly blended upload would show a magenta fringe; straight alpha shows none.
    rows = blank(48, 48, (255, 0, 255, 0))
    fill(rows, 8, 8, 24, 24, (240, 140, 20, 255))
    fill(rows, 24, 8, 40, 24, (20, 200, 200, 255))
    fill(rows, 8, 24, 24, 40, (150, 30, 170, 255))
    fill(rows, 24, 24, 40, 40, (30, 30, 30, 255))
    barcode(rows, 8, 8, 4, 3, 8, 2)
    return rows


def texture_hero(left, right, ident):
    rows = blank(32, 32, (0, 0, 0, 255))
    fill(rows, 0, 0, 16, 32, left + (255,))
    fill(rows, 16, 0, 32, 32, right + (255,))
    fill(rows, 24, 0, 32, 8, (0, 255, 255, 0))  # transparent corner with a cyan RGB (straight alpha)
    barcode(rows, 0, 29, 4, 3, 8, ident)
    return rows


ASSETS = [
    # assetId, file, revision, alpha mode, rows
    ("sky", "assets/sky.png", "rev-1", "opaque", texture_sky()),
    ("mid", "assets/mid.png", "rev-1", "straight", texture_mid()),
    ("hero", "assets/hero.png", "rev-1", "straight", texture_hero((250, 120, 160), (40, 120, 250), 3)),
]
# The same assetId with other content under a new revision and path: a resource swap.
HERO_REV2 = ("hero", "assets-rev2/hero.png", "rev-2", "straight", texture_hero((250, 240, 60), (30, 200, 90), 5))


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def asset_record(entry):
    asset_id, path, revision, alpha, rows = entry
    data = png_bytes(rows)
    return {
        "assetId": asset_id,
        "revisionId": revision,
        "contentHash": sha256(data),
        "relativePath": path,
        "widthPx": len(rows[0]),
        "heightPx": len(rows),
        "alphaMode": alpha,
        "colorSpace": "srgb",
    }, data


# ---------------------------------------------------------------------------------- bundle
Q_IDENTITY = {"x": 0, "y": 0, "z": 0, "w": 1}
ONE = {"x": 1, "y": 1, "z": 1}


def v3(x, y, z):
    return {"x": x, "y": y, "z": z}


def node(node_id, parent, kind, position, **extra):
    record = {"id": node_id, "parentId": parent, "type": kind, "localPosition": position,
              "localRotation": dict(Q_IDENTITY), "localScale": dict(ONE)}
    record.update(extra)
    return record


def plane(node_id, parent, position, size, order, asset, *, parent_order=None):
    ref = {key: asset[key] for key in ("assetId", "revisionId", "contentHash", "relativePath")}
    return node(node_id, parent, "imagePlane", position, assetRef=ref, widthSceneUnits=size[0],
                heightSceneUnits=size[1], renderOrder=order)


def build_bundle(assets):
    by_id = {a["assetId"]: a for a in assets}
    nodes = [
        node("root", None, "group", v3(0, 0, 0)),
        plane("sky", "root", v3(0, 0, -4), (20, 11.25), 0, by_id["sky"]),
        plane("mid", "root", v3(-1.8, 0.3, -2), (4, 4), 1, by_id["mid"]),
        node("stage", "root", "group", v3(0, 0, 0)),
        plane("hero", "stage", v3(0.5, -0.2, 0), (2.4, 2.4), 3, by_id["hero"]),
        # Same asset as `hero`: one resource used by two nodes.
        plane("echo", "root", v3(-2.4, -1.0, 0.5), (2.4, 2.4), 2, by_id["hero"]),
    ]
    fps = {"numerator": 24, "denominator": 1}
    return {
        "schemaVersion": "1.0",
        "bundleId": "proof-p-corpus",
        "revisionId": "rev-1" if by_id["hero"]["revisionId"] == "rev-1" else "rev-2",
        "fps": fps,
        "editDurationFrames": EDIT_FRAMES,
        "renderProfile": {
            "width": WIDTH, "height": HEIGHT, "fps": dict(fps), "colorSpace": "srgb-display",
            "alphaMode": "straight", "premultiplyOnUpload": False, "videoCodec": "h264-yuv420p",
            "threeRevision": "0.187.0", "browserChannel": "synthetic-proof-p",
        },
        "assets": [by_id["sky"], by_id["mid"], by_id["hero"]],
        "performanceTracks": [{
            "id": "perf-main", "instanceId": "inst-a",
            "channels": [
                {"kind": "event", "channelId": "stage-slide", "nodeId": "stage", "property": "localPosition",
                 "startInstanceFrame": 12, "durationFrames": 24, "delta": v3(2, 0, 0), "holdFinal": True},
                {"kind": "cycle", "channelId": "hero-sway", "nodeId": "hero", "property": "localRotation",
                 "periodFrames": 48, "amplitude": 12, "axis": v3(0, 0, 1), "phaseOffsetFrames": 5},
                {"kind": "static", "channelId": "echo-small", "nodeId": "echo", "property": "localScale",
                 "value": v3(0.6, 0.6, 0.6)},
            ],
        }],
        "compositionPasses": [],
        "sceneDefinitions": [{"id": "scene-a", "schemaVersion": "1.0", "rootNodeId": "root", "nodes": nodes}],
        "sceneInstances": [{"id": "inst-a", "sceneDefinitionId": "scene-a", "durationFrames": EDIT_FRAMES,
                            "performanceTrackIds": ["perf-main"]}],
        "cameraClips": [{
            "id": "cam-main",
            "initialPosition": v3(0, 1, 8), "initialTarget": v3(0, 0.5, 0), "initialFovDeg": 45,
            "segments": [
                {"recipe": "dollyIn", "durationFrames": 32, "from": "initial", "distanceSceneUnits": 1.5,
                 "easing": "easeInOutCubic"},
                {"recipe": "hold", "durationFrames": 16, "from": "previousEnd"},
                {"recipe": "truckRight", "durationFrames": 48, "from": "previousEnd", "distanceSceneUnits": 1,
                 "easing": "easeOutCubic"},
            ],
        }],
        "shots": [
            {"id": "shot-1", "sceneInstanceId": "inst-a", "editRange": {"startFrame": 0, "endFrameExclusive": 48},
             "instanceStartFrame": 0, "cameraClipId": "cam-main", "cameraInFrame": 0, "intent": "synthetic shot 1"},
            {"id": "shot-2", "sceneInstanceId": "inst-a", "editRange": {"startFrame": 48, "endFrameExclusive": 96},
             "instanceStartFrame": 48, "cameraClipId": "cam-main", "cameraInFrame": 48, "intent": "synthetic shot 2",
             "transitionInto": {"type": "continuous"}},
        ],
    }


# ---------------------------------------------------------------------------------- expected
def decoded(rows):
    return (len(rows[0]), len(rows), rows)


def pixel_probes(bundle, frame, textures):
    """Screen pixels whose answer cannot depend on filtering: the layer on top and its colour agree
    for the pixel and its four neighbours at ±2 px. Up to three per visible layer, spread out."""
    state = model.frame_state(bundle, frame)
    found = {}
    for py in range(6, HEIGHT - 6, 6):
        for px in range(6, WIDTH - 6, 6):
            here = model.visible_layer(state, WIDTH, HEIGHT, px, py, textures)
            if here is None:
                continue
            around = [model.visible_layer(state, WIDTH, HEIGHT, px + dx, py + dy, textures)
                      for dx, dy in ((2, 0), (-2, 0), (0, 2), (0, -2))]
            if any(a != here for a in around):
                continue
            found.setdefault(here["layer"], []).append({"x": px, "y": py, "layer": here["layer"], "color": here["color"]})
    chosen = []
    for layer in sorted(found):
        candidates = found[layer]
        step = max(1, len(candidates) // 3)
        chosen.extend(candidates[::step][:3])
    return chosen


def frame_expectation(bundle, frame, textures):
    state = model.frame_state(bundle, frame)
    return {
        "editFrame": frame,
        "shotId": state["shotId"],
        "instanceFrame": state["instanceFrame"],
        "cameraClipFrame": state["cameraClipFrame"],
        "camera": {
            "position": dict(zip("xyz", state["camera"]["position"])),
            "target": dict(zip("xyz", state["camera"]["target"])),
            "fovDeg": state["camera"]["fovDeg"],
        },
        "worldMatrices": state["worldMatrices"],
        "paintOrder": model.paint_order(state["definition"]),
        "pixelProbes": pixel_probes(bundle, frame, textures),
    }


def edited(bundle, hero_position=None, fov=None):
    out = copy.deepcopy(bundle)
    if hero_position is not None:
        hero = next(n for n in out["sceneDefinitions"][0]["nodes"] if n["id"] == "hero")
        hero["localPosition"] = hero_position
    if fov is not None:
        out["cameraClips"][0]["initialFovDeg"] = fov
    return out


def build_expected(bundle, swap_bundle, files, textures, swap_textures):
    first_move = v3(0.8, 0.0, 0.0)
    final_move = v3(1.0, 0.25, 0.0)
    after_gesture = edited(bundle, hero_position=final_move)
    after_fov = edited(after_gesture, fov=55)
    probe_frames = [20, 60]
    required = sorted({n["assetRef"]["assetId"] for d in bundle["sceneDefinitions"] for n in d["nodes"] if "assetRef" in n})
    used_by = {a: sorted(n["id"] for d in bundle["sceneDefinitions"] for n in d["nodes"]
                         if n.get("assetRef", {}).get("assetId") == a) for a in required}
    return {
        "corpusVersion": CORPUS_VERSION,
        "note": "Generated by corpus.py from model.py (independent maths, validated against the origin oracle). "
                "No destination output is stored here.",
        "bundle": {"file": "bundle.json", "bundleId": bundle["bundleId"], "revisionId": bundle["revisionId"],
                   "sha256": files["bundle.json"]["sha256"]},
        "swapBundle": {"file": "bundle.swap.json", "bundleId": swap_bundle["bundleId"],
                       "revisionId": swap_bundle["revisionId"], "sha256": files["bundle.swap.json"]["sha256"]},
        "output": {"width": WIDTH, "height": HEIGHT, "fps": "24/1", "editFrames": EDIT_FRAMES},
        "tolerances": TOLERANCES,
        "assets": {
            a["assetId"]: {"file": a["relativePath"], "sha256": a["contentHash"], "bytes": files[a["relativePath"]]["bytes"],
                           "widthPx": a["widthPx"], "heightPx": a["heightPx"], "alphaMode": a["alphaMode"]}
            for a in bundle["assets"]
        },
        "swapAsset": {"assetId": "hero", "file": HERO_REV2[1], "sha256": swap_bundle["assets"][2]["contentHash"],
                      "bytes": files[HERO_REV2[1]]["bytes"], "revisionId": "rev-2"},
        "resources": {"requiredAssetIds": required, "usedBy": used_by},
        "frames": [frame_expectation(bundle, f, textures) for f in FRAMES],
        "seekOrder": SEEK_ORDER,
        "swapFrames": [frame_expectation(swap_bundle, f, swap_textures) for f in (0, 20, 60)],
        "edit": {
            "scene": "proof",
            "select": ["scene/scene-a/node/hero"],
            "gesture": {
                "target": "scene/scene-a/node/hero",
                "property": "scene/scene-a/node/hero/position",
                "updates": [first_move, final_move],
                "finalValue": final_move,
                "probeFrames": probe_frames,
                "afterCommit": {str(f): model.frame_state(after_gesture, f)["worldMatrices"]["hero"] for f in probe_frames},
                "candidateAfterFirstUpdate": {str(f): model.frame_state(edited(bundle, hero_position=first_move), f)["worldMatrices"]["hero"]
                                              for f in probe_frames},
                "untouchedNode": "echo",
            },
            "fov": {
                "property": "camera/cam-main/start/fov",
                "value": 55,
                "afterEdit": {str(f): model.frame_state(after_fov, f)["camera"]["fovDeg"] for f in (0, 60)},
                "camera": {str(f): {"position": dict(zip("xyz", model.frame_state(after_fov, f)["camera"]["position"]))}
                           for f in (0, 60)},
            },
        },
    }


# ---------------------------------------------------------------------------------- writing
def dump(value):
    return (json.dumps(value, indent=2) + "\n").encode()


def generate():
    """Returns {relative path: bytes} for the whole corpus; no I/O."""
    records, images = [], {}
    for entry in ASSETS:
        record, data = asset_record(entry)
        records.append(record)
        images[record["relativePath"]] = data
    bundle = build_bundle(records)
    swap_record, swap_data = asset_record(HERO_REV2)
    images[swap_record["relativePath"]] = swap_data
    swap_bundle = build_bundle([r for r in records if r["assetId"] != "hero"] + [swap_record])
    # Keep the declared asset order stable regardless of which entry was swapped.
    swap_bundle["assets"] = [next(r for r in swap_bundle["assets"] if r["assetId"] == k) for k in ("sky", "mid", "hero")]
    out = dict(images)
    out["bundle.json"] = dump(bundle)
    out["bundle.swap.json"] = dump(swap_bundle)
    files = {name: {"sha256": sha256(data), "bytes": len(data)} for name, data in out.items()}
    textures = {entry[0]: decoded(entry[4]) for entry in ASSETS}
    swap_textures = dict(textures)
    swap_textures["hero"] = decoded(HERO_REV2[4])
    out["expected.json"] = dump(build_expected(bundle, swap_bundle, files, textures, swap_textures))
    files["expected.json"] = {"sha256": sha256(out["expected.json"]), "bytes": len(out["expected.json"])}
    provenance = {
        "corpusVersion": CORPUS_VERSION,
        "generator": {"file": "scripts/directed-scene-proof/corpus.py", "sha256": sha256((HERE / "corpus.py").read_bytes()),
                      "model": {"file": "scripts/directed-scene-proof/model.py", "sha256": sha256((HERE / "model.py").read_bytes())},
                      "python": platform.python_version_tuple()[0] + "." + platform.python_version_tuple()[1]},
        "origin": "none: drawn from scratch; expected values from model.py validated against docs/migration/fixtures/expected "
                  "(origin oracle, packages/scene-engine@d6adac15)",
        "private_media": False,
        "files": dict(sorted(files.items())),
    }
    out["provenance.json"] = dump(provenance)
    return out


def write(target):
    target = pathlib.Path(target)
    if target.exists():
        shutil.rmtree(target)
    for name, data in generate().items():
        path = target / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)


def load_textures(corpus_dir):
    """Decodes nothing: textures come from the generator itself, the files are checked by hash."""
    return {entry[0]: decoded(entry[4]) for entry in ASSETS}


def check(target):
    target = pathlib.Path(target)
    wanted = generate()
    problems = []
    for name, data in wanted.items():
        path = target / name
        if not path.is_file():
            problems.append(f"missing {name}")
        elif path.read_bytes() != data:
            problems.append(f"differs {name}")
    if target.exists():
        for path in target.rglob("*"):
            if path.is_file() and str(path.relative_to(target)) not in wanted:
                problems.append(f"unexpected {path.relative_to(target)}")
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", default=str(HERE / "corpus"))
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if args.check:
        problems = check(args.out)
        for problem in problems:
            print("FAIL " + problem)
        print("corpus is up to date" if not problems else f"{len(problems)} problem(s)")
        return 1 if problems else 0
    write(args.out)
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
