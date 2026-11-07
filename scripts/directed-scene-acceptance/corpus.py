#!/usr/bin/env python3
"""Corpus and independent expectations for the S10 acceptance (BL019: splat, camera, audio, package, export).

    python3 scripts/directed-scene-acceptance/corpus.py            # (re)writes ./corpus
    python3 scripts/directed-scene-acceptance/corpus.py --check    # fails on any difference or stray file

Provenance: every byte is drawn here from scratch (solid-colour PNGs, SPZ v2 clouds of a few gaussians, a sine-tone WAV);
no photograph, no private media, nothing copied from the origin or from another proof's corpus. `expected.json` holds
only numbers computed by `oracle.py` (closed-form maths); it never stores output of the destination. Hashes are real
SHA-256, so the resource gate of the editor accepts the bundles. The corpus-wide invariants (markers never overlap on
screen in a judged frame, every splat is isolated from the others, far corners are background) are *asserted* while the
expectations are written: a layout that cannot be judged does not get generated.
"""
import argparse
import copy
import hashlib
import json
import math
import pathlib
import shutil
import struct
import sys
import tempfile
import zlib
from fractions import Fraction

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import oracle  # noqa: E402

CORPUS_DIR = HERE / "corpus"
CORPUS_VERSION = 3  # 1 -> 2: amendment A1 (audio window semantics; version 1 is kept in history/A1-audio-window/before); 2 -> 3: round R2
TOLERANCES_PATH = HERE / "tolerances.json"
TOL = json.loads(TOLERANCES_PATH.read_text(encoding="utf-8"))

# Palette: solid colours, far from each other and from the background (max channel distance >= 120).
BG = (20, 24, 40)
RED, GREEN, BLUE, YELLOW = (230, 50, 50), (50, 200, 70), (60, 90, 240), (240, 210, 40)
ORANGE, CYAN, PURPLE = (240, 150, 30), (30, 210, 220), (170, 60, 200)

SPLAT_W, SPLAT_H = 640, 360
CAMERA_SIZES = [(640, 360), (360, 640)]
assert [list(size) for size in CAMERA_SIZES] == TOL["camera"]["renderSizes"], "camera sizes differ from tolerances.json"
assert [SPLAT_W, SPLAT_H] == TOL["splat"]["renderSize"], "splat render size differs from tolerances.json"
CAMERA_FRAMES = [0, 8, 15, 16, 24, 31, 32, 40, 47]
SPLAT_FRAMES = [0, 12, 23, 24, 36, 47]
AUDIO_RATE_OUT, AUDIO_RATE_MIX = 48000, 44100
assert (AUDIO_RATE_OUT, AUDIO_RATE_MIX) == (TOL["audio"]["outputSampleRate"], TOL["audio"]["mixSampleRate"]), "audio rates differ from tolerances.json"
MIX_CHANNELS = 1  # the mix WAV is mono; the origin's graph writes stereo, so the expected gain is oracle.channel_gain_db(1, 2)

# File time (seconds from the first sample of the WHOLE mix WAV; the origin's `editSecondsToSourceTime` reads the file at
# `editStartSeconds + edit time`): tone, silence, tone, tone, silence, tone. The 880 -> 1320 join has no silence on purpose.
# The WAV must cover [0, editEndSecondsExclusive) of every bundle that declares it (origin `node/render.ts`, AUDIO_SHORT):
# the longest window is audio24's [1, 13).
MIX_SEGMENTS = [(0, 3, 440, 0.5), (Fraction(7, 2), 6, 880, 0.5), (6, Fraction(15, 2), 1320, 0.25), (Fraction(33, 4), 13, 440, 0.5)]
MIX_SECONDS = 13

# The pre-amendment mix (8 s, tones laid out for the retired assumption A1) is kept byte for byte, but only as the
# negative control "source shorter than the declared window": it is the right fixture for the origin's AUDIO_SHORT rule.
SHORT_MIX_SEGMENTS = [(0, 2, 440, 0.5), (Fraction(5, 2), 4, 880, 0.5), (4, Fraction(11, 2), 1320, 0.25), (6, 8, 440, 0.5)]
SHORT_MIX_SECONDS = 8
SHORT_MIX_SHA256 = "cc252a2b0f8ec9695772e0ef51b01b7700d242b72af9a1e77ca10f0eeaffba75"  # the A1-era audio/mix.wav

AUDIO_CASES = {
    "audio24_window_a": {"bundle": "audio24", "from": 18, "frames": 72,
                         "covers": "edit window offset 1 s: the export reads the file from 1.75 s; the last 1.25 s of the 440 Hz tone, a gap, the first 1.25 s of 880 Hz"},
    "audio24_window_b": {"bundle": "audio24", "from": 96, "frames": 96,
                         "covers": "file 5.0-9.0 s: 880 -> 1320 Hz join without silence, a gap, 440 Hz; mix offset 1 s respected"},
    "audio_ntsc_window": {"bundle": "audio_ntsc", "from": 180, "frames": 96,
                          "covers": "rate 30000/1001, offset 0: the window starts at 6.006 s, not 6.000 s; 1320 Hz ends and 440 Hz begins inside the window"},
    "full_av": {"bundle": "full", "from": 0, "frames": 48,
                "covers": "splat scene and audio mix together (integrated acceptance of the splat and mix fronts): first 2 s of the 440 Hz tone"},
}

# The source-too-short control (negative, live): the A1-era WAV is 8 s but the declared window is [1, 13). The origin
# (`node/render.ts`, AUDIO_SHORT) refuses a source that does not cover `editEndSecondsExclusive` BEFORE looking at the range, so
# every range of this bundle must be refused with `audio_source_short`: one the file covers entirely (file seconds 1-3: a
# partial render of a covered stretch) as well as one that reads past the end (file seconds 5-9), and no file may be created.
# A range that the 8 s file covers must NOT export: the old phase expected that and contradicted the origin (review R2-1).
SHORT_SOURCE_CASE = {"bundle": "audio_short", "expectedCode": "audio_source_short",
                     "ranges": [{"name": "covered by the file (file seconds 1 to 3)", "from": 0, "frames": 48},
                                {"name": "reads past the end of the file (file seconds 5 to 9)", "from": 96, "frames": 96}]}


# ---------------------------------------------------------------------------------- PNG
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


def solid_png(color, size=8):
    """Opaque RGBA PNG of one colour; stored deflate blocks so the bytes do not depend on a zlib build."""
    row = b"\x00" + b"".join(bytes(color) + b"\xff" for _ in range(size))
    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + _chunk(b"IHDR", header) + _chunk(b"IDAT", _stored_zlib(row * size)) + _chunk(b"IEND", b"")


def sha256(data):
    return hashlib.sha256(data).hexdigest()


# ---------------------------------------------------------------------------------- bundle pieces
Q_IDENTITY = {"x": 0, "y": 0, "z": 0, "w": 1}


def v3(x, y, z):
    return {"x": x, "y": y, "z": z}


def quaternion_y(degrees):
    half = math.radians(degrees) / 2
    return {"x": 0, "y": math.sin(half), "z": 0, "w": math.cos(half)}


def node(node_id, parent, kind, position, rotation=None, scale=1.0, **extra):
    record = {"id": node_id, "parentId": parent, "type": kind, "localPosition": position,
              "localRotation": rotation or dict(Q_IDENTITY), "localScale": v3(scale, scale, scale)}
    record.update(extra)
    return record


def asset_record(asset_id, path, data, kind, **extra):
    record = {"assetId": asset_id, "revisionId": "rev-1", "contentHash": sha256(data), "relativePath": path,
              "widthPx": 8 if kind == "png" else 1, "heightPx": 8 if kind == "png" else 1,
              "alphaMode": "opaque", "colorSpace": "srgb"}
    if kind == "spz":
        record["format"] = "spz"
    record.update(extra)
    return record


def reference(record):
    return {key: record[key] for key in ("assetId", "revisionId", "contentHash", "relativePath")}


def plane(node_id, record, position, size, order, rotation=None):
    return node(node_id, "root", "imagePlane", position, rotation, assetRef=reference(record),
                widthSceneUnits=size[0], heightSceneUnits=size[1], renderOrder=order)


def render_profile(width, height, fps):
    return {"width": width, "height": height, "fps": dict(fps), "colorSpace": "srgb-display", "alphaMode": "straight",
            "premultiplyOnUpload": False, "videoCodec": "h264-yuv420p", "threeRevision": "0.187.0",
            "browserChannel": "synthetic-acceptance-s10"}


def shot(shot_id, start, end, clip, in_frame=0, cut=None):
    out = {"id": shot_id, "sceneInstanceId": "inst-a", "editRange": {"startFrame": start, "endFrameExclusive": end},
           "instanceStartFrame": start, "cameraClipId": clip, "cameraInFrame": in_frame, "intent": f"s10 {shot_id}"}
    if cut:
        out["transitionInto"] = {"type": "cut", "reason": cut}
    return out


def assemble(bundle_id, fps, frames, assets, nodes, clips, shots, size=(640, 360), audio=None):
    bundle = {
        "schemaVersion": "1.0", "bundleId": bundle_id, "revisionId": "rev-1", "fps": dict(fps),
        "editDurationFrames": frames, "renderProfile": render_profile(size[0], size[1], fps),
        "assets": assets, "performanceTracks": [], "compositionPasses": [],
        "sceneDefinitions": [{"id": "scene-a", "schemaVersion": "1.0", "rootNodeId": "root", "nodes": nodes}],
        "sceneInstances": [{"id": "inst-a", "sceneDefinitionId": "scene-a", "durationFrames": frames,
                            "performanceTrackIds": []}],
        "cameraClips": clips, "shots": shots,
    }
    if audio:
        bundle["audio"] = audio
    return bundle


FPS24 = {"numerator": 24, "denominator": 1}
FPS_NTSC = {"numerator": 30000, "denominator": 1001}

# ---------------------------------------------------------------------------------- files
FILES = {}  # relative path -> bytes, filled by `build_files`


def build_files():
    files = {
        "assets/bg.png": solid_png(BG),
        "assets/mk-red.png": solid_png(RED),
        "assets/mk-green.png": solid_png(GREEN),
        "assets/mk-blue.png": solid_png(BLUE),
        "assets/mk-yellow.png": solid_png(YELLOW),
    }

    def cloud(*splats):
        return oracle.spz_v2([{"position": p, "color": c, "alpha": 1.0, "sigma": s} for p, c, s in splats])

    files["splat/sp-red.spz"] = cloud(((0, 0, 0), RED, 0.30))
    files["splat/sp-green.spz"] = cloud(((0, 0, 0), GREEN, 0.25))
    # Order pair: front gaussian (orange) in front of a smaller back one (cyan) whose centre sits inside the front core.
    files["splat/sp-order.spz"] = cloud(((0, 0, 0.5), ORANGE, 0.30), ((0.05, 0.02, -0.8), CYAN, 0.15))
    files["splat/sp-pair.spz"] = cloud(((-1.2, 0, 0), RED, 0.20), ((1.2, 0, 0), BLUE, 0.20))
    files["splat/sp-purple.spz"] = cloud(((0, 0, 0), PURPLE, 0.20))
    mix = oracle.ToneTable(MIX_SEGMENTS, MIX_SECONDS)
    files["audio/mix.wav"] = oracle.wav_bytes(mix, AUDIO_RATE_MIX)
    files["audio/mix-short.wav"] = oracle.wav_bytes(oracle.ToneTable(SHORT_MIX_SEGMENTS, SHORT_MIX_SECONDS), AUDIO_RATE_MIX)
    assert sha256(files["audio/mix-short.wav"]) == SHORT_MIX_SHA256, "the short-source control must stay byte-identical to the pre-amendment mix"
    return files


SPLAT_SPECS = {
    # asset -> list of (colour, local position, sigma) as written above (the model's own copy of the intent)
    "sp-red": [(RED, (0, 0, 0), 0.30)],
    "sp-green": [(GREEN, (0, 0, 0), 0.25)],
    "sp-order": [(ORANGE, (0, 0, 0.5), 0.30), (CYAN, (0.05, 0.02, -0.8), 0.15)],
    "sp-pair": [(RED, (-1.2, 0, 0), 0.20), (BLUE, (1.2, 0, 0), 0.20)],
    "sp-purple": [(PURPLE, (0, 0, 0), 0.20)],
}

SPLAT_NODES = [
    # node id, asset, position, rotation (deg about Y), uniform scale, role
    ("s-red", "sp-red", (-3.5, 1.2, 0.0), 0, 1.0, "isolated"),
    ("s-green", "sp-green", (3.2, -1.0, -2.0), 0, 1.0, "isolated, farther from the camera"),
    ("s-order", "sp-order", (0.5, -0.2, 0.0), 0, 1.0, "front/back order at the overlap"),
    ("s-pair", "sp-pair", (-2.4, -1.0, 0.0), 60, 1.0, "node rotation: the two gaussians swap depth"),
    ("s-purple", "sp-purple", (1.6, 1.9, 0.0), 0, 1.5, "node uniform scale 1.5"),
]


def build_bundles(files):
    rec = {}
    for name, kind in (("bg", "png"), ("mk-red", "png"), ("mk-green", "png"), ("mk-blue", "png"), ("mk-yellow", "png")):
        rec[name] = asset_record(name, f"assets/{name}.png", files[f"assets/{name}.png"], kind)
    for name in SPLAT_SPECS:
        rec[name] = asset_record(name, f"splat/{name}.spz", files[f"splat/{name}.spz"], "spz")
    mix_hash = sha256(files["audio/mix.wav"])  # `sourcePcmHashSha256` is the hash of the whole WAV file
    short_hash = sha256(files["audio/mix-short.wav"])

    def marker_nodes():
        return [
            node("root", None, "group", v3(0, 0, 0)),
            plane("bg", rec["bg"], v3(0, 0, -12), (40, 40), 0),
            plane("mk_red", rec["mk-red"], v3(-2.0, 1.0, -1.0), (1.2, 0.8), 1),
            plane("mk_green", rec["mk-green"], v3(1.6, 0.2, 0.0), (0.8, 1.2), 2),
            plane("mk_blue", rec["mk-blue"], v3(0.2, -0.9, 1.0), (1.0, 0.6), 3, rotation=quaternion_y(25)),
            plane("mk_yellow", rec["mk-yellow"], v3(3.0, 1.6, -3.0), (1.6, 1.0), 4),
        ]

    marker_assets = [rec[n] for n in ("bg", "mk-red", "mk-green", "mk-blue", "mk-yellow")]
    camera_clips = [
        {"id": "cam-a", "initialPosition": v3(0, 1, 9), "initialTarget": v3(0, 0.5, 0), "initialFovDeg": 30,
         "segments": [{"recipe": "dollyIn", "durationFrames": 16, "from": "initial", "distanceSceneUnits": 2.0,
                       "easing": "easeInOutCubic"}]},
        {"id": "cam-b", "initialPosition": v3(0, 1, 8), "initialTarget": v3(0, 0.5, 0), "initialFovDeg": 50,
         "segments": [{"recipe": "truckRight", "durationFrames": 16, "from": "initial", "distanceSceneUnits": 1.5,
                       "easing": "easeOutCubic"}]},
        {"id": "cam-c", "initialPosition": v3(-1, 2, 7), "initialTarget": v3(0.5, 0, 0), "initialFovDeg": 75,
         "segments": [{"recipe": "dollyIn", "durationFrames": 8, "from": "initial", "distanceSceneUnits": 1.0, "easing": "linear"},
                      {"recipe": "hold", "durationFrames": 8, "from": "previousEnd"}]},
    ]
    camera = assemble("s10-camera", FPS24, 48, marker_assets, marker_nodes(), camera_clips,
                      [shot("shot-a", 0, 16, "cam-a"), shot("shot-b", 16, 32, "cam-b", cut="fov and move change"),
                       shot("shot-c", 32, 48, "cam-c", cut="third camera")])

    hold = lambda frames: [{"id": "cam-hold", "initialPosition": v3(0, 1, 8), "initialTarget": v3(0, 0.5, 0),
                            "initialFovDeg": 50, "segments": [{"recipe": "hold", "durationFrames": frames, "from": "initial"}]}]

    def audio_block(start, frames, fps, path="audio/mix.wav", digest=mix_hash):
        """The window is `[start, start + edit duration)`: origin `node/render.ts` refuses a window whose length differs from the
        edit's by more than 1 ms (`AUDIO_VIDEO_DURATION`), and a source shorter than `editEndSecondsExclusive` (`AUDIO_SHORT`)."""
        edit_seconds = Fraction(frames * fps["denominator"], fps["numerator"])
        return {"mixRelativePath": path, "editStartSeconds": start,
                "editEndSecondsExclusive": float(Fraction(start).limit_denominator(1000) + edit_seconds), "sourcePcmHashSha256": digest}

    audio24 = assemble("s10-audio24", FPS24, 288, marker_assets, marker_nodes(), hold(288),
                       [shot("shot-1", 0, 288, "cam-hold")], audio=audio_block(1.0, 288, FPS24))
    audio_ntsc = assemble("s10-audio-ntsc", FPS_NTSC, 300, marker_assets, marker_nodes(), hold(300),
                          [shot("shot-1", 0, 300, "cam-hold")], audio=audio_block(0.0, 300, FPS_NTSC))
    # Negative control, own fixture: the same edit as audio24, but its WAV (8 s) does not cover the window [1, 13).
    audio_short = assemble("s10-audio-short", FPS24, 288, marker_assets, marker_nodes(), hold(288),
                           [shot("shot-1", 0, 288, "cam-hold")], audio=audio_block(1.0, 288, FPS24, "audio/mix-short.wav", short_hash))

    splat_nodes = [node("root", None, "group", v3(0, 0, 0)), plane("bg", rec["bg"], v3(0, 0, -12), (40, 40), 0)]
    for node_id, asset, position, rotation, scale, _ in SPLAT_NODES:
        record = node(node_id, "root", "splat", v3(*position), quaternion_y(rotation) if rotation else None, scale,
                      assetRef=reference(rec[asset]))
        splat_nodes.append(record)
    splat_clips = [
        {"id": "cam-a", "initialPosition": v3(0, 0.5, 8), "initialTarget": v3(0, 0.5, 0), "initialFovDeg": 50,
         "segments": [{"recipe": "dollyIn", "durationFrames": 24, "from": "initial", "distanceSceneUnits": 2.0,
                       "easing": "easeInOutCubic"}]},
        {"id": "cam-b", "initialPosition": v3(0, 0.5, 8), "initialTarget": v3(0, 0.5, 0), "initialFovDeg": 50,
         "segments": [{"recipe": "truckRight", "durationFrames": 24, "from": "initial", "distanceSceneUnits": 1.5,
                       "easing": "easeOutCubic"}]},
    ]
    splat_assets = [rec["bg"]] + [rec[n] for n in SPLAT_SPECS]
    splat = assemble("s10-splat", FPS24, 48, splat_assets, splat_nodes, splat_clips,
                     [shot("shot-a", 0, 24, "cam-a"), shot("shot-b", 24, 48, "cam-b", cut="second camera move")])
    # Splat and audio together: what the integrated acceptance packages and exports.
    full = copy.deepcopy(splat)
    full["bundleId"] = "s10-full"
    full["audio"] = audio_block(0.0, 48, FPS24)
    # The same scene and camera moves authored for a 9:16 deliverable: the render profile is the authored frame, and the
    # destination fits the requested output into it (a 16:9 profile asked for 360x640 gets bars, not a taller picture).
    portrait = copy.deepcopy(camera)
    portrait["bundleId"] = "s10-camera-portrait"
    portrait["renderProfile"] = render_profile(360, 640, FPS24)
    return {"camera": camera, "camera_portrait": portrait, "audio24": audio24, "audio_ntsc": audio_ntsc, "audio_short": audio_short, "splat": splat, "full": full}


BUNDLE_FILES = {"camera": "bundle.camera.json", "camera_portrait": "bundle.camera-portrait.json", "audio24": "bundle.audio24.json", "audio_ntsc": "bundle.audio-ntsc.json",
                "audio_short": "bundle.audio-short.json", "splat": "bundle.splat.json", "full": "bundle.full.json"}


# ---------------------------------------------------------------------------------- camera expectations
MARKERS = ["mk_red", "mk_green", "mk_blue", "mk_yellow"]
MARKER_COLOR = {"mk_red": RED, "mk_green": GREEN, "mk_blue": BLUE, "mk_yellow": YELLOW}


def camera_expectations(bundle):
    cases = {}
    nodes = oracle.nodes_of(bundle)
    for width, height in CAMERA_SIZES:
        for frame in CAMERA_FRAMES:
            state = oracle.frame_state(bundle, frame)
            masks = {name: oracle.marker_mask(state, nodes[name], width, height) for name in MARKERS}
            for index, a in enumerate(MARKERS):
                for b in MARKERS[index + 1:]:
                    assert not (masks[a] & masks[b]), f"markers {a}/{b} overlap on screen at frame {frame} {width}x{height}"
            entry = {"frame": frame, "size": [width, height], "shot": state["shotId"],
                     "bundle": "camera" if width > height else "camera_portrait",
                     "worldMatrices": {name: [round(v, 12) for v in state["worldMatrices"][name]] for name in MARKERS},
                     "camera": {"position": state["camera"]["position"], "target": state["camera"]["target"],
                                "fovDeg": state["camera"]["fovDeg"]},
                     "markers": {}}
            for name in MARKERS:
                stats = oracle.mask_stats(masks[name])
                entry["markers"][name] = {"area": 0, "x": None, "y": None} if stats is None else \
                    {"area": stats["area"], "x": round(stats["x"], 4), "y": round(stats["y"], 4)}
            cases[f"{width}x{height}@{frame}"] = entry
    assert sorted({round(c["camera"]["fovDeg"], 6) for c in cases.values()}) == sorted(TOL["camera"]["fovsDeg"]), "camera fovs differ from tolerances.json"
    judged = [c for c in cases.values() if c["size"] == [640, 360]]
    assert all(sum(1 for m in c["markers"].values() if m["area"] >= 400) >= 2 for c in judged), \
        "every 640x360 frame needs at least two markers with an area of 400 px or more"
    portrait = [c for c in cases.values() if c["size"] == [360, 640]]
    # Portrait is judged by fewer markers (a tall frame sees fewer of them): every frame still needs one judged marker, and the
    # frames that judge only one (`@8`, `@15`) are covered by the other six portrait frames in all three shots.
    assert all(sum(1 for m in c["markers"].values() if m["area"] >= 400) >= 1 for c in portrait), \
        "every 360x640 frame needs at least one marker with an area of 400 px or more"
    assert sum(1 for c in portrait if sum(1 for m in c["markers"].values() if m["area"] >= 400) >= 2) >= 6
    return cases


# ---------------------------------------------------------------------------------- splat expectations
def splat_expectations(bundle):
    nodes = oracle.nodes_of(bundle)
    cases = {}
    background_points = [[12, 12], [SPLAT_W - 13, 12], [12, SPLAT_H - 13], [SPLAT_W - 13, SPLAT_H - 13]]
    for frame in SPLAT_FRAMES:
        state = oracle.frame_state(bundle, frame)
        entries = []
        for node_id, asset, _, _, scale, role in SPLAT_NODES:
            for index, (color, position, sigma) in enumerate(SPLAT_SPECS[asset]):
                where = oracle.splat_screen(state, SPLAT_W, SPLAT_H, nodes[node_id],
                                            {"position": position, "sigma": sigma}, (scale, scale, scale))
                assert where is not None, f"{node_id} is behind the camera at frame {frame}"
                entries.append({
                    "node": node_id, "index": index, "asset": asset, "role": role,
                    "x": round(where["x"], 4), "y": round(where["y"], 4), "depth": round(where["depth"], 4),
                    "sigmaPx": round(where["sigma_px"], 4),
                    "r50Px": round(oracle.half_intensity_radius(where["sigma_px"]), 4),
                    "color": [round(c, 2) for c in oracle.quantised_color(color)], "colorName": color,
                    "world": [round(w, 6) for w in where["world"]]})
        for entry in entries:
            assert entry["sigmaPx"] >= TOL["splat"]["minScreenSigmaPx"], f"frame {frame}: {entry['node']}[{entry['index']}] sigma {entry['sigmaPx']} px < {TOL['splat']['minScreenSigmaPx']}"
            margin = 3 * entry["sigmaPx"] + 3
            assert margin <= entry["x"] <= SPLAT_W - margin and margin <= entry["y"] <= SPLAT_H - margin, \
                f"frame {frame}: {entry['node']}[{entry['index']}] is not 3 sigma inside the picture"
        for a in entries:
            for b in entries:
                if a is b or a["node"] == b["node"] and a["asset"] == "sp-order":
                    continue
                gap = math.hypot(a["x"] - b["x"], a["y"] - b["y"])
                limit = 2 * (a["sigmaPx"] + b["sigmaPx"]) + 6  # regions (radius ~1.95 sigma) never touch
                assert gap >= limit, f"frame {frame}: {a['node']}[{a['index']}] and {b['node']}[{b['index']}] are {gap:.1f} px apart (< {limit:.1f})"
        far = [[px, py] for px, py in background_points
               if all(math.hypot(px - e["x"], py - e["y"]) >= TOL["splat"]["backgroundClearanceSigmas"] * e["sigmaPx"] for e in entries)]
        assert far, f"frame {frame}: no corner is >= 6 sigma from every splat"
        probe = oracle.back_separable_point(entries, BG, TOL["splat"]["backSplatSearchSigmas"])
        assert probe["separation"] >= TOL["splat"]["backSplatSeparationMinDistance"], \
            f"frame {frame}: the back splat of s-order is separable by only {probe['separation']} at its best pixel"
        cases[str(frame)] = {"frame": frame, "shot": state["shotId"], "splats": entries, "backgroundPoints": far, "backProbe": probe,
                             "camera": {"position": state["camera"]["position"], "target": state["camera"]["target"],
                                        "fovDeg": state["camera"]["fovDeg"]}}
    return cases


# ---------------------------------------------------------------------------------- audio expectations
def audio_expectations(bundles):
    table = oracle.ToneTable(MIX_SEGMENTS, MIX_SECONDS)
    out = {}
    for case_id, case in AUDIO_CASES.items():
        bundle = bundles[case["bundle"]]
        fps = Fraction(bundle["fps"]["numerator"], bundle["fps"]["denominator"])
        start = Fraction(bundle["audio"]["editStartSeconds"]).limit_denominator(1000)
        duration = oracle.frames_to_seconds(case["frames"], fps)
        window_start = oracle.frames_to_seconds(case["from"], fps)
        transitions = []
        for mix_time, kind in table.transitions():
            at = oracle.export_time_of(mix_time, fps, start, case["from"])
            if 0 < at < duration:
                transitions.append({"kind": kind, "mixSeconds": str(mix_time), "exportSeconds": str(at), "exportSecondsFloat": float(at)})
        segments = []
        for seg_start, seg_end, hz, amp in table.segments:
            a = max(Fraction(0), oracle.export_time_of(seg_start, fps, start, case["from"]))
            b = min(duration, oracle.export_time_of(seg_end, fps, start, case["from"]))
            if b > a:
                segments.append({"from": float(a), "to": float(b), "hz": hz, "amplitude": amp})
        out[case_id] = {
            **{k: case[k] for k in ("bundle", "from", "frames", "covers")},
            "fps": f"{fps.numerator}/{fps.denominator}", "editStartSeconds": float(start), "sourceChannels": MIX_CHANNELS,
            "windowStartSeconds": str(window_start), "durationSeconds": str(duration), "durationSecondsFloat": float(duration),
            "outputSamples": int(round(duration * AUDIO_RATE_OUT)),
            "transitions": transitions, "segments": segments,
            "naive30fpsStartSeconds": float(Fraction(case["from"], 30)) if case["bundle"] == "audio_ntsc" else None,
        }
    # A case must be able to tell NTSC from 30 fps by more than the onset tolerance.
    tolerance = TOL["audio"]["onsetToleranceSeconds"]
    ntsc = out["audio_ntsc_window"]
    ntsc_from = AUDIO_CASES["audio_ntsc_window"]["from"]
    assert abs(float(Fraction(ntsc_from * 1001, 30000)) - ntsc_from / 30) > 1.5 * tolerance, "NTSC window is not distinguishable from 30 fps"
    assert len(ntsc["transitions"]) >= 2, "the NTSC window needs edges to time"
    # Origin rules the corpus must obey for every bundle that declares a mix (the corpus is generated, never hand-edited).
    mix_seconds = Fraction(MIX_SECONDS)
    for key, bundle in bundles.items():
        audio = bundle.get("audio")
        if not audio or key == "audio_short":
            continue
        fps = Fraction(bundle["fps"]["numerator"], bundle["fps"]["denominator"])
        window = Fraction(audio["editEndSecondsExclusive"]).limit_denominator(100000) - Fraction(audio["editStartSeconds"]).limit_denominator(1000)
        assert abs(window - oracle.frames_to_seconds(bundle["editDurationFrames"], fps)) <= Fraction(1, 1000), f"{key}: window length differs from the edit duration"
        assert Fraction(audio["editEndSecondsExclusive"]).limit_denominator(100000) <= mix_seconds, f"{key}: the mix WAV is shorter than editEndSecondsExclusive"
    short = bundles["audio_short"]["audio"]
    assert Fraction(SHORT_MIX_SECONDS) < Fraction(short["editEndSecondsExclusive"]).limit_denominator(1000), "the short fixture must be shorter than its window"
    for entry in SHORT_SOURCE_CASE["ranges"]:
        reads_from = Fraction(entry["from"], 24) + Fraction(short["editStartSeconds"]).limit_denominator(1000)
        reads_through = reads_from + Fraction(entry["frames"], 24)
        entry["readsFromSeconds"], entry["readsThroughSeconds"] = float(reads_from), float(reads_through)
    covered, past_end = SHORT_SOURCE_CASE["ranges"]
    assert Fraction(covered["readsThroughSeconds"]).limit_denominator(1000) <= SHORT_MIX_SECONDS, "the covered range must lie inside the short file"
    assert Fraction(past_end["readsThroughSeconds"]).limit_denominator(1000) > SHORT_MIX_SECONDS, "the past-the-end range must read past the end of the short file"
    return out


# ---------------------------------------------------------------------------------- build
def build():
    files = build_files()
    bundles = build_bundles(files)
    json_files = {BUNDLE_FILES[k]: json.dumps(b, indent=2, sort_keys=True).encode() + b"\n" for k, b in bundles.items()}
    expected = {
        "corpusVersion": CORPUS_VERSION,
        "note": "Generated by corpus.py from oracle.py (closed-form maths). No destination output is stored here.",
        "tolerancesSha256": sha256(TOLERANCES_PATH.read_bytes()),
        "bundles": {k: {"file": BUNDLE_FILES[k], "bundleId": b["bundleId"], "sha256": sha256(json_files[BUNDLE_FILES[k]]),
                        "fps": f"{b['fps']['numerator']}/{b['fps']['denominator']}", "editFrames": b["editDurationFrames"],
                        "declaredAssets": sorted(a["relativePath"] for a in b["assets"])}
                    for k, b in bundles.items()},
        "assets": {path: {"sha256": sha256(data), "bytes": len(data)} for path, data in sorted(files.items())},
        "palette": {"background": BG, "markers": {k: list(v) for k, v in MARKER_COLOR.items()}},
        "camera": {"markers": MARKERS, "cases": camera_expectations(bundles["camera"])},
        "splat": {"renderSize": [SPLAT_W, SPLAT_H], "frames": SPLAT_FRAMES, "cases": splat_expectations(bundles["splat"]),
                  "nodes": [{"node": n, "asset": a, "role": r} for n, a, _, _, _, r in SPLAT_NODES]},
        "audio": {"mix": {"relativePath": "audio/mix.wav", "sampleRate": AUDIO_RATE_MIX, "seconds": MIX_SECONDS, "timeBase": "file", "channels": MIX_CHANNELS,
                          "windowRule": "file time = editStartSeconds + edit time (origin audio-window.ts); the WAV covers [0, editEndSecondsExclusive)",
                          "segments": [[str(a), str(b), hz, float(amp)] for a, b, hz, amp in oracle.ToneTable(MIX_SEGMENTS, MIX_SECONDS).segments]},
                  "shortSource": {**SHORT_SOURCE_CASE, "relativePath": "audio/mix-short.wav", "fileSeconds": SHORT_MIX_SECONDS,
                                  "fileFrames": SHORT_MIX_SECONDS * AUDIO_RATE_MIX, "windowSeconds": [1.0, 13.0], "windowEndSeconds": 13},
                  "outputSampleRate": AUDIO_RATE_OUT, "cases": audio_expectations(bundles)},
    }
    json_files["expected.json"] = json.dumps(expected, indent=1, sort_keys=True).encode() + b"\n"
    provenance = {
        "generator": "scripts/directed-scene-acceptance/corpus.py", "version": CORPUS_VERSION, "private_media": False,
        "origin": "drawn from scratch: solid-colour PNGs, SPZ v2 written by oracle.spz_v2, sine-tone WAV from oracle.wav_bytes",
        "files": {name: {"sha256": sha256(data), "bytes": len(data)} for name, data in sorted({**files, **json_files}.items())},
    }
    json_files["provenance.json"] = json.dumps(provenance, indent=1, sort_keys=True).encode() + b"\n"
    return {**files, **json_files}


def write(directory):
    directory = pathlib.Path(directory)
    for relative, data in build().items():
        path = directory / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)


def check(directory):
    """Differences between `directory` and what this generator writes (empty when identical)."""
    directory = pathlib.Path(directory)
    wanted = build()
    problems = []
    present = {str(p.relative_to(directory)) for p in directory.rglob("*") if p.is_file()}
    for relative, data in wanted.items():
        path = directory / relative
        if not path.is_file():
            problems.append(f"missing {relative}")
        elif path.read_bytes() != data:
            problems.append(f"differs {relative}")
    problems += [f"stray {name}" for name in sorted(present - set(wanted))]
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--out", default=None)
    args = parser.parse_args()
    if args.check:
        problems = check(CORPUS_DIR)
        for problem in problems:
            print(problem, file=sys.stderr)
        print("corpus is byte-identical to the generator output" if not problems else f"{len(problems)} difference(s)")
        return 1 if problems else 0
    target = pathlib.Path(args.out) if args.out else CORPUS_DIR
    write(target)
    print(f"corpus written to {target}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
