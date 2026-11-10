"""Pure-logic tests of the P3 pilot-grain corpus generator; no editor, GPU, runtime page or origin
checkout is needed.

The corpus is only worth its pins if a wrong port disagrees with it. Every mutation below is a mistake
a port could make; each one is applied to the generator's own formulas and must change the corpus it
produces (that is what makes the Rust, GLSL and JS comparisons fail on the same mistake). Written with
`unittest` assertions, never a bare `assert`, so the file still tests something under `python -O`.
"""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).parent
spec = importlib.util.spec_from_file_location("pilot_grain_corpus", HERE / "pilot-grain-corpus.py")
grain = importlib.util.module_from_spec(spec)
spec.loader.exec_module(grain)

PINNED = (grain.CORPUS / grain.CASES_NAME).read_bytes()
MANIFEST = json.loads((grain.CORPUS / grain.MANIFEST_NAME).read_text())


def corpus_bytes():
    return grain.canonical(grain.build_corpus())


class BaselineTests(unittest.TestCase):
    def test_the_generator_reproduces_the_pinned_corpus_byte_for_byte(self):
        self.assertEqual(corpus_bytes(), PINNED)

    def test_the_manifest_pins_the_corpus_it_ships_with(self):
        self.assertEqual(MANIFEST["corpus"]["sha256"], grain.sha256_of(PINNED))

    def test_the_hash_vectors_of_the_origin_spec_are_what_the_script_computes(self):
        self.assertEqual(grain.spec_vector_findings(MANIFEST), [])

    def test_the_two_blockers_stay_declared_and_the_corpus_is_not_called_an_origin_output(self):
        ids = {blocker["id"] for blocker in MANIFEST["blockers"]}
        self.assertIn("ORIGIN_NOT_EXECUTED", ids)
        self.assertIn("ORIGIN_GOLDEN_NOT_COMPARABLE", ids)
        self.assertNotIn("origin-output", PINNED.decode())

    def test_every_source_records_a_blob_and_a_digest(self):
        for source in MANIFEST["sources"]:
            self.assertEqual(len(source["git_blob"]), 40, source)
            self.assertEqual(len(source["sha256"]), 64, source)


class MutationTests(unittest.TestCase):
    """Each mistake changes the pinned bytes; the unmutated generator does not."""

    def assertCaught(self, name, **patches):
        with patch.multiple(grain, **patches):
            mutated = corpus_bytes()
        self.assertNotEqual(mutated, PINNED, "the corpus does not notice: " + name)

    def test_a_wrong_hash_constant(self):
        def lowbias32(value):
            h = value & grain.M32
            h ^= h >> 16
            h = (h * 0x7FEB352D) & grain.M32
            h ^= h >> 15
            h = (h * 0x846CA68C) & grain.M32  # one bit off
            h ^= h >> 16
            return h

        self.assertCaught("lowbias32 constant", lowbias32=lowbias32)

    def test_a_sum_instead_of_a_xor_when_the_seed_is_mixed(self):
        def hash_pixel(seed, x, y):
            return grain.lowbias32((grain.lowbias32((seed + 0x9E3779B9) & grain.M32) + x + ((y << 16) & grain.M32)) & grain.M32)

        self.assertCaught("seed sum", hash_pixel=hash_pixel)

    def test_the_row_counted_from_the_top(self):
        def cell_of(x, y, height, size):
            cell = grain.f32(grain.cell_px(size, height))
            return (
                int(grain.math.floor(grain.f32(grain.f32(x + 0.5) / cell))),
                int(grain.math.floor(grain.f32(grain.f32(y + 0.5) / cell))),
            )

        self.assertCaught("missing glFragY flip", cell_of=cell_of)

    def test_the_pixel_corner_instead_of_its_centre(self):
        def cell_of(x, y, height, size):
            cell = grain.f32(grain.cell_px(size, height))
            gl_y = height - 1 - y
            return (int(grain.math.floor(x / cell)), int(grain.math.floor(gl_y / cell)))

        self.assertCaught("pixel corner", cell_of=cell_of)

    def test_the_resolution_scale_applied_twice(self):
        def cell_px(size, height):
            return max(size * (height / 720) * (height / 720), 1.0)

        self.assertCaught("scale twice", cell_px=cell_px)

    def test_the_resolution_scale_never_applied(self):
        def cell_px(size, height):
            return max(float(size), 1.0)

        self.assertCaught("scale missing", cell_px=cell_px)

    def test_the_cell_not_clamped_to_one_pixel(self):
        def cell_px(size, height):
            return size * (height / 720)

        self.assertCaught("no 1 px floor", cell_px=cell_px)

    def test_a_seed_that_does_not_follow_the_frame(self):
        def seed_for(reseed, base, frame_in_clip):
            return grain.lowbias32(base)

        self.assertCaught("seed not per frame", seed_for=seed_for)

    def test_a_seed_that_is_not_hashed(self):
        def seed_for(reseed, base, frame_in_clip):
            return (base ^ frame_in_clip) & grain.M32 if reseed == "per-frame" else base

        self.assertCaught("seed unhashed", seed_for=seed_for)

    def test_a_triangular_distribution_that_is_uniform(self):
        def triangular(h):
            return grain.unit_float(h) * 2.0 - 1.0

        self.assertCaught("uniform noise", triangular=triangular)

    def test_colour_channels_that_share_one_noise(self):
        def colored(h):
            value = grain.triangular(h)
            return [value, value, value]

        self.assertCaught("coloured as mono", colored=colored)

    def test_a_boolean_mixed_instead_of_stepped(self):
        def mix_param(a, b, t):
            if isinstance(a, bool) or isinstance(b, bool):
                return b if t >= 0.5 else a
            return original(a, b, t)

        original = grain.mix_param
        self.assertCaught("boolean midpoint", mix_param=mix_param)

    def test_the_easing_taken_from_the_departure_key(self):
        def resolve_keyframes(keys, frame):
            first = keys[0]
            if frame <= first["frame"]:
                return first["value"]
            arriving = next((i for i, key in enumerate(keys) if frame <= key["frame"]), -1)
            to = keys[arriving] if arriving >= 0 else None
            if to is not None and frame == to["frame"]:
                return to["value"]
            if arriving > 0 and to is not None:
                frm = keys[arriving - 1]
                t = grain.apply_easing((frame - frm["frame"]) / (to["frame"] - frm["frame"]), frm.get("easing", "linear"))
                return grain.mix_param(frm["value"], to["value"], t)
            return keys[-1]["value"]

        self.assertCaught("easing on the departure key", resolve_keyframes=resolve_keyframes)

    def test_an_easing_curve_that_is_not_the_pilots(self):
        original = grain.apply_easing

        def apply_easing(t, name):
            return original(t, "linear" if name == "easeOutCubic" else name)

        self.assertCaught("easeOutCubic as linear", apply_easing=apply_easing)

    def test_the_grain_added_in_linear_light(self):
        def srgb_to_linear(c):
            return c  # the response and the mix both lose the transfer curve

        self.assertCaught("no transfer curve", srgb_to_linear=srgb_to_linear)

    def test_the_luma_response_ignored(self):
        def smoothstep(edge0, edge1, x):
            return 0.0

        self.assertCaught("luma response", smoothstep=smoothstep)


class ProvenanceTests(unittest.TestCase):
    def test_a_corpus_edited_by_hand_is_refused_by_verify(self):
        edited = json.loads(PINNED)
        edited["pixels"][0]["probes"][0]["expected"][0] += 1.0
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp)
            (folder / grain.CASES_NAME).write_bytes(grain.canonical(edited))
            (folder / grain.MANIFEST_NAME).write_bytes((grain.CORPUS / grain.MANIFEST_NAME).read_bytes())
            with patch.object(grain, "CORPUS", folder), patch("sys.stderr"):
                self.assertEqual(grain.verify(None), 1)

    def test_a_manifest_that_drops_a_blocker_is_refused_by_verify(self):
        manifest = json.loads(json.dumps(MANIFEST))
        manifest["blockers"] = [b for b in manifest["blockers"] if b["id"] != "ORIGIN_NOT_EXECUTED"]
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp)
            (folder / grain.CASES_NAME).write_bytes(PINNED)
            (folder / grain.MANIFEST_NAME).write_bytes(grain.canonical(manifest))
            with patch.object(grain, "CORPUS", folder), patch("sys.stderr"):
                self.assertEqual(grain.verify(None), 1)

    def test_the_untouched_corpus_verifies_offline(self):
        with patch("sys.stdout"):
            self.assertEqual(grain.verify(None), 0)


if __name__ == "__main__":
    unittest.main()
