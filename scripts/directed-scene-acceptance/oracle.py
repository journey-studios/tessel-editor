"""Independent expected values for the S10 acceptance (BL019, splat / camera / audio / package / export).

WHY this file exists: the destination (core, runtime, Spark, FFmpeg adapters) must be judged by numbers it did not
produce. Everything here is closed-form maths written from the public definitions (pinhole camera, isotropic
gaussian, SPZ v2 layout, rational frame times, sine tones) without reading the Rust or the runtime:

  * camera: poses come from `scripts/directed-scene-proof/model.py` (validated against the origin oracle by its
    `model.spec.py`, read-only); this file adds the picture-space consequences (exact projected masks of flat
    quads, projected centre and size of a gaussian);
  * splat: a gaussian with sigma `s` at world point `c` seen at depth `d` by a camera of vertical fov `f` and image
    height `H` has screen sigma `s * (H/2) / tan(f/2) / d` and centre at the pinhole projection of `c`;
  * audio: a tone table in *file time* (seconds from the first sample of the whole WAV), an exact `Fraction` mapping
    from edit frames to seconds, and the origin's window rule: edit time t plays the file at `editStartSeconds + t`
    (`packages/scene-engine/src/transport/audio-window.ts`, `editSecondsToSourceTime`, origin commit d6adac15). The WAV
    is the *whole* source and must cover `[0, editEndSecondsExclusive)` (`node/render.ts`, `AUDIO_SHORT`). This replaced
    the earlier assumption A1 (`mix[0] <-> editStartSeconds`) by amendment A1 (docs/features/scene-acceptance.md).

Nothing here reads a destination output. Unsupported inputs raise instead of guessing.
"""
import importlib.util
import math
import pathlib
import sys
from fractions import Fraction

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
P_DIR = ROOT / "scripts" / "directed-scene-proof"


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules.setdefault(name, module)
    spec.loader.exec_module(module)
    return module


model = _load("tsp_model", P_DIR / "model.py")  # read-only: camera clips, world matrices, plane sampling


# ---------------------------------------------------------------------------------- camera and pixels
def frame_state(bundle, edit_frame):
    return model.frame_state(bundle, edit_frame)


def focal_px(camera, height):
    """Pixels per unit of tangent: (H/2) / tan(fov/2), the fov being the vertical one (Three.js PerspectiveCamera)."""
    return (height / 2) / math.tan(math.radians(camera["fovDeg"]) / 2)


def project(camera, width, height, point):
    """Pinhole projection. Returns `(x, y, depth)` in continuous pixel units where the centre of pixel (i, j) is
    (i + 0.5, j + 0.5), or None when the point is not in front of the camera."""
    forward, right, up = model.camera_basis(camera)
    rel = model.sub(point, camera["position"])
    depth = model.dot(rel, forward)
    if depth <= 1e-9:
        return None
    f = focal_px(camera, height)
    return (width / 2 + f * model.dot(rel, right) / depth, height / 2 - f * model.dot(rel, up) / depth, depth)


def apply(matrix, point):
    """Column-major 4x4 times a point."""
    return [sum(matrix[col * 4 + row] * (point[col] if col < 3 else 1.0) for col in range(4)) for row in range(3)]


def plane_matrix(frame, node):
    """World matrix of an image plane with its widthSceneUnits/heightSceneUnits folded into the axes."""
    matrix = list(frame["worldMatrices"][node["id"]])
    matrix[0:3] = model.scale(matrix[0:3], node["widthSceneUnits"])
    matrix[4:7] = model.scale(matrix[4:7], node["heightSceneUnits"])
    return matrix


def plane_corners(frame, node):
    matrix = plane_matrix(frame, node)
    return [apply(matrix, [sx, sy, 0.0]) for sx, sy in ((-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5))]


def marker_mask(frame, node, width, height):
    """Set of pixel indices (i, j) whose centre ray hits the plane `node`, ignoring every other node.

    Only the bounding box of the projected corners (plus a 2 px margin) is scanned; a corner behind the camera
    makes the box the whole image."""
    corners = [project(frame["camera"], width, height, c) for c in plane_corners(frame, node)]
    if any(c is None for c in corners):
        box = (0, 0, width, height)
    else:
        xs, ys = [c[0] for c in corners], [c[1] for c in corners]
        box = (max(0, int(math.floor(min(xs))) - 2), max(0, int(math.floor(min(ys))) - 2),
               min(width, int(math.ceil(max(xs))) + 2), min(height, int(math.ceil(max(ys))) + 2))
    matrix = plane_matrix(frame, node)
    mask = set()
    for j in range(box[1], box[3]):
        for i in range(box[0], box[2]):
            origin, direction = model.ray_for_pixel(frame["camera"], width, height, i, j)
            hit = model.sample_plane(node, matrix, origin, direction, None)
            if hit is not None and 0 <= hit[0] < 1 and 0 <= hit[1] < 1:
                mask.add((i, j))
    return mask


def mask_stats(mask):
    """Centroid (pixel-index coordinates: the centre of pixel (i, j) is (i, j)) and area."""
    if not mask:
        return None
    return {"x": sum(i for i, _ in mask) / len(mask), "y": sum(j for _, j in mask) / len(mask), "area": len(mask)}


def iou(a, b):
    union = len(a | b)
    return 1.0 if union == 0 else len(a & b) / union


def nodes_of(bundle):
    return {n["id"]: n for definition in bundle["sceneDefinitions"] for n in definition["nodes"]}


# ---------------------------------------------------------------------------------- splats
SH_C0 = 0.28209479177387814


def spz_v2(splats, fractional_bits=12):
    """Niantic SPZ v2 container written from its public layout (gzip of: magic 'NGSP', version, count, shDegree,
    fractionalBits, flags, reserved; then positions (24-bit fixed point x3), alphas, colours, scales, rotations).

    `splats` items: position (x, y, z), color (r, g, b in 0..255), alpha (0..1), sigma (std-dev, world units, same
    on every axis), rotation (x, y, z, w; unit, w >= 0). Deterministic: the gzip header carries mtime 0."""
    import gzip
    import struct
    raw = bytearray(struct.pack("<IIIBBBB", 0x5053474E, 2, len(splats), 0, fractional_bits, 0, 0))
    for s in splats:
        for value in s["position"]:
            fixed = int(round(value * (1 << fractional_bits)))
            if not -(1 << 23) <= fixed < (1 << 23):
                raise ValueError("position outside the 24-bit fixed-point range")
            raw += struct.pack("<i", fixed)[:3]
    for s in splats:
        raw.append(int(round(max(0.0, min(1.0, s["alpha"])) * 255)))
    for s in splats:
        for channel in s["color"]:
            sh = (channel / 255 - 0.5) / SH_C0  # the zeroth SH coefficient that decodes back to the colour
            raw.append(max(0, min(255, int(round((sh * 0.15 + 0.5) * 255)))))
    for s in splats:
        for _ in range(3):
            raw.append(max(0, min(255, int(round((math.log(s["sigma"]) + 10) * 16)))))
    for s in splats:
        x, y, z, w = s.get("rotation", (0.0, 0.0, 0.0, 1.0))
        sign = -1.0 if w < 0 else 1.0
        for axis in (x, y, z):
            raw.append(max(0, min(255, int(round(sign * axis * 127.5 + 127.5)))))
    return gzip.compress(bytes(raw), compresslevel=6, mtime=0)


def quantised_sigma(sigma):
    """The sigma a decoder recovers from the one-byte log encoding of `spz_v2`."""
    byte = max(0, min(255, int(round((math.log(sigma) + 10) * 16))))
    return math.exp(byte / 16 - 10)


def quantised_color(color):
    out = []
    for channel in color:
        sh = (channel / 255 - 0.5) / SH_C0
        byte = max(0, min(255, int(round((sh * 0.15 + 0.5) * 255))))
        out.append(max(0.0, min(255.0, ((byte / 255 - 0.5) / 0.15 * SH_C0 + 0.5) * 255)))
    return out


def splat_screen(frame, width, height, node, splat, node_scale):
    """Where a gaussian lands: continuous pixel centre, pixel-index centre, sigma in pixels and depth.

    The node's world matrix (rotation, translation, scale) maps the splat's local position; a *uniform* node
    scale multiplies sigma. Anisotropic scales are not modelled and raise."""
    if not (math.isclose(node_scale[0], node_scale[1]) and math.isclose(node_scale[1], node_scale[2])):
        raise model.Unsupported("anisotropic node scale")
    world = apply(frame["worldMatrices"][node["id"]], splat["position"])
    shot = project(frame["camera"], width, height, world)
    if shot is None:
        return None
    x, y, depth = shot
    sigma_world = quantised_sigma(splat["sigma"]) * node_scale[0]
    return {"world": world, "x": x - 0.5, "y": y - 0.5, "depth": depth,
            "sigma_px": sigma_world * focal_px(frame["camera"], height) / depth}


def half_intensity_radius(sigma_px):
    """Radius at which a gaussian drops to half of its peak: r50 = sigma * sqrt(2 ln 2)."""
    return sigma_px * math.sqrt(2 * math.log(2))


def dot_splat_file(splats):
    """The 32-byte `.splat` record layout (f32 position x3, f32 scale x3, u8 rgba, u8 rotation w,x,y,z) used by the
    baseline phase that exercises the runtime path which exists today (non-directed scene layer)."""
    import struct
    out = bytearray()
    for s in splats:
        out += struct.pack("<6f", *s["position"], s["sigma"], s["sigma"], s["sigma"])
        out += bytes([*(int(round(c)) for c in s["color"]), int(round(s["alpha"] * 255))])
        out += bytes([255, 128, 128, 128])  # identity quaternion, w first
    return bytes(out)


# ---------------------------------------------------------------------------------- audio
class ToneTable:
    """Piecewise sine tones in *file time*. Each segment is `(start, end, hz, amplitude)` in exact `Fraction` seconds
    from the first sample of the WAV; the phase restarts at the segment start (phase = 2 pi hz (t - start)); outside
    every segment the file is silent. `duration` is the length of the WAV: a time past it is not in the file."""

    def __init__(self, segments, duration):
        self.segments = [(Fraction(a), Fraction(b), hz, amp) for a, b, hz, amp in segments]
        self.duration = Fraction(duration)

    def value(self, t):
        """`t` is a Fraction (or float) of seconds in file time."""
        if t < 0 or t >= self.duration:
            return 0.0
        for start, end, hz, amp in self.segments:
            if start <= t < end:
                return amp * math.sin(2 * math.pi * hz * float(t - start))
        return 0.0

    def hz_at(self, t):
        for start, end, hz, _ in self.segments:
            if start <= t < end:
                return hz
        return None

    def transitions(self):
        """`(time, kind)` of every silence<->tone edge in file time, kind in {'on', 'off'}."""
        out = []
        for index, (start, end, _, _) in enumerate(self.segments):
            previous_end = self.segments[index - 1][1] if index else None
            if previous_end != start:
                out.append((start, "on"))
            following = self.segments[index + 1][0] if index + 1 < len(self.segments) else None
            if following != end:
                out.append((end, "off"))
        return out


def frames_to_seconds(frame, fps):
    """Exact seconds of an edit frame at `fps = Fraction(num, den)` (frame / fps)."""
    return Fraction(frame) / fps


def source_time_of(edit_seconds, edit_start):
    """The origin's window rule (`editSecondsToSourceTime`): the WAV second that plays at edit time `edit_seconds`."""
    return Fraction(edit_start) + Fraction(edit_seconds)


def splat_alpha(entry, x, y):
    """Opacity of an isotropic gaussian splat at pixel `(x, y)`: `opacity * exp(-r^2 / (2 sigma^2))` (opacity 1 = opaque)."""
    r2 = (x - entry["x"]) ** 2 + (y - entry["y"]) ** 2
    return entry.get("opacity", 1.0) * math.exp(-r2 / (2 * entry["sigmaPx"] ** 2))


def composite_at(entries, background, x, y):
    """The colour the model expects at pixel `(x, y)`: every gaussian composited far to near with the `over` operator on the
    flat background (straight alpha, 8-bit colour). Entries carry `depth` (distance along the view), `color`, `x`, `y`, `sigmaPx`.
    This is the one blending law the acceptance models (amendment A2); coverage, tonemapping and linear-light blending are not."""
    colour = [float(c) for c in background]
    for entry in sorted(entries, key=lambda e: -e["depth"]):
        alpha = splat_alpha(entry, x, y)
        colour = [c * (1 - alpha) + k * alpha for c, k in zip(colour, entry["color"])]
    return tuple(int(round(c)) for c in colour)


MONO_TO_STEREO_GAIN = math.sqrt(0.5)


def channel_gain_db(source_channels, output_channels):
    """Gain (dB, per output channel) the origin's graph applies to a source with `source_channels` channels written as
    `output_channels` channels.

    Origin `packages/scene-engine/src/node/audio-graph.ts` (commit d6adac15) forces every mix input through
    `aformat=...:channel_layouts=stereo` and sums with `amix=...:normalize=0`, so the channel law is FFmpeg's (libswresample)
    default rematrix: a mono (front-centre) source reaches front-left and front-right each at `center_mix_level = 1/sqrt(2)`
    (-3.0103 dB; `libswresample/rematrix.c`, `M_SQRT1_2`), and a source that already has the output layout is untouched. The
    mono -> stereo number is therefore not a property of the editor under test: `proof.spec.py` re-derives it by running the
    origin's own `aformat` filter in FFmpeg. Any other pair raises instead of being guessed."""
    if source_channels == output_channels:
        return 0.0
    if (source_channels, output_channels) == (1, 2):
        return 20 * math.log10(MONO_TO_STEREO_GAIN)
    raise ValueError(f"no channel law is established for {source_channels} -> {output_channels} channels")


def expected_window(table, fps, edit_start, first_frame, frames, out_rate):
    """Analytic expected PCM of an export of edit frames `[first_frame, first_frame + frames)`.

    Sample n of the output is at edit time `first_frame/fps + n/out_rate`; it plays the file at
    `edit_start + edit time` (origin rule). Returns `(list of floats, N, duration Fraction)` with
    N = round(duration * out_rate)."""
    start = source_time_of(frames_to_seconds(first_frame, fps), edit_start)
    duration = frames_to_seconds(frames, fps)
    count = int(round(duration * out_rate))
    origin = float(start)  # exact Fraction -> float once; sample times then differ from the exact ones by < 1e-12 s
    return [table.value(origin + n / out_rate) for n in range(count)], count, duration


def export_time_of(file_time, fps, edit_start, first_frame):
    """Seconds into the exported file at which `file_time` of the WAV is heard (negative before the export starts)."""
    return Fraction(file_time) - source_time_of(frames_to_seconds(first_frame, fps), edit_start)


def wav_bytes(table, rate, channels=1):
    """16-bit PCM WAV of the whole file at `rate`, sampled from the analytic table (`[0, table.duration)`)."""
    import io
    import struct
    import wave
    count = int(round(table.duration * rate))
    frames = bytearray()
    for n in range(count):
        value = int(round(table.value(Fraction(n, rate)) * 32767))
        frames += struct.pack("<h", value) * channels
    buffer = io.BytesIO()
    with wave.open(buffer, "wb") as handle:
        handle.setnchannels(channels)
        handle.setsampwidth(2)
        handle.setframerate(rate)
        handle.writeframes(bytes(frames))
    return buffer.getvalue()


def goertzel_power(samples, hz, rate):
    """Power of one frequency in `samples` (Goertzel), normalised by the window length squared."""
    n = len(samples)
    if n == 0:
        return 0.0
    k = 2 * math.cos(2 * math.pi * hz / rate)
    s1 = s2 = 0.0
    for x in samples:
        s0 = x + k * s1 - s2
        s2, s1 = s1, s0
    return (s1 * s1 + s2 * s2 - k * s1 * s2) / (n * n)
