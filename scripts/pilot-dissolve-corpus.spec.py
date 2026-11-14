"""Tests of the pilot crossDissolve corpus generator (P4b-S0, BL082.4.1, F064). No editor, GPU, runtime page,
engine or origin execution is needed; the origin blobs are only read when TESSEL_ORIGIN_PATH names a clone.

Four layers, none of which compares the generator only with itself:

1. HAND VECTORS written here as literals (the plan's tables, worked by hand), checked against the module.
2. An INDEPENDENT recomputation: `fractions.Fraction` for the easing and the weights (exact rationals) and
   `decimal.Decimal` for the sRGB curves (a different `pow` than the float one the oracle uses).
3. MUTATIONS: each is a mistake a port could make; applied to the oracle's own functions it must change (or
   refuse to build) the corpus, and the formula mutants must also fail the hand vectors of layer 1.
4. TAMPER and CLI checks on a temporary copy, and the origin-verifier logic on a throwaway git repository.

The spec REFUSES a Microsoft Store stub as the interpreter (a refusal is never a skip), prints `sys.version`,
and is written with `unittest` assertions, never a bare `assert`, so it still tests under `python -O`.
"""
import ast
import importlib.util
import io
import json
import math
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from decimal import ROUND_FLOOR, Decimal, localcontext
from fractions import Fraction
from functools import lru_cache
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).parent
SPEC_NAME = "pilot_dissolve_corpus"
loader = importlib.util.spec_from_file_location(SPEC_NAME, HERE / "pilot-dissolve-corpus.py")
d = importlib.util.module_from_spec(loader)
loader.loader.exec_module(d)

_problem = d.interpreter_problem()
if _problem:
    sys.stderr.write("INTERPRETER_NOT_REAL: %s\n" % _problem)
    raise SystemExit(d.EXIT_BAD_INTERPRETER)
sys.stderr.write("python: %s\n" % sys.version.replace("\n", " "))

PINNED = (d.CORPUS / d.CASES_NAME).read_bytes()
PINNED_MANIFEST = (d.CORPUS / d.MANIFEST_NAME).read_bytes()
CORPUS = json.loads(PINNED.decode("utf-8"))
MANIFEST = json.loads(PINNED_MANIFEST.decode("utf-8"))
FACTS = d.repo_facts()
ORIG = {name: getattr(d, name) for name in dir(d) if callable(getattr(d, name)) and not name.startswith("_")}
ORIGIN_PATH = os.environ.get("TESSEL_ORIGIN_PATH")


def build_bytes():
    return d.canonical(d.build_corpus(FACTS))


def run_main(*argv):
    out, err = io.StringIO(), io.StringIO()
    with redirect_stdout(out), redirect_stderr(err):
        code = d.main(list(argv))
    return code, out.getvalue(), err.getvalue()


# ---- layer 2: the independent recomputation --------------------------------------------------------------


def ease_exact(t, name):
    if name == "linear":
        return t
    if name == "easeInOutCubic":
        return 4 * t ** 3 if t < Fraction(1, 2) else 1 - (-2 * t + 2) ** 3 / 2
    if name == "easeOutCubic":
        return 1 - (1 - t) ** 3
    raise ValueError(name)


def exact_progress(k, duration, easing):
    return ease_exact(Fraction(2 * k + 1, 2 * duration), easing)


def exact_weight(progress, curve, amount):
    shaped = progress * progress * (3 - 2 * progress) if curve == "smooth" else progress
    return progress * (1 - amount) + shaped * amount


def as_decimal(fraction):
    return Decimal(fraction.numerator) / Decimal(fraction.denominator)


@lru_cache(maxsize=None)
def dec_decode(code):
    with localcontext() as ctx:
        ctx.prec = 60
        v = Decimal(code) / Decimal(255)
        if v <= Decimal("0.04045"):
            return v / Decimal("12.92")
        return ((v + Decimal("0.055")) / Decimal("1.055")) ** Decimal("2.4")


def dec_encode255(linear):
    with localcontext() as ctx:
        ctx.prec = 60
        x = min(max(linear, Decimal(0)), Decimal(1))
        if x <= Decimal("0.0031308"):
            return x * Decimal("12.92") * 255
        return (Decimal("1.055") * (x ** (Decimal(5) / Decimal(12))) - Decimal("0.055")) * 255


def exact_pixel(deck_a, deck_b, weight):
    """(codes, closest distance of any channel to a rounding tie) of out = a * (1 - w) + b * w in linear light."""
    with localcontext() as ctx:
        ctx.prec = 60
        w = as_decimal(weight)
        codes, margin = [], Decimal(1)
        for a, b in zip(deck_a, deck_b):
            value = dec_encode255(dec_decode(a) * (1 - w) + dec_decode(b) * w)
            floor = value.to_integral_value(rounding=ROUND_FLOOR)
            codes.append(int(min(255, max(0, (value + Decimal("0.5")).to_integral_value(rounding=ROUND_FLOOR)))))
            margin = min(margin, abs(value - floor - Decimal("0.5")))
        return codes, margin


# ---- layer 1: the hand vectors, literals of this file ----------------------------------------------------

HAND_WINDOWS = [("center", 12, 42, 54, 6, 6), ("center", 5, 46, 51, 2, 3), ("center", 1, 48, 49, 0, 1), ("start", 12, 48, 60, 0, 12),
                ("end", 12, 36, 48, 12, 0), ("center", 11, 43, 54, 5, 6), ("center", 2, 47, 49, 1, 1)]
# plan 6.2, D = 12: k, progress, weight (6 decimals)
HAND_PROGRESS = [(0, 0.000289, 0.000000), (1, 0.007813, 0.000182), (3, 0.099248, 0.027595), (5, 0.385127, 0.330723), (6, 0.614873, 0.669277), (11, 0.999711, 1.000000)]
# plan 6.2, channel 200 -> 40, k = 5, easeInOutCubic: (curve, intensity, unrounded, code)
HAND_INTENSITY = [("smooth", 0.0, 162.4061, 162), ("smooth", 0.5, 165.4320, 165), ("smooth", 1.0, 168.3881, 168),
                  ("linear", 0.0, 162.4061, 162), ("linear", 0.5, 162.4061, 162), ("linear", 1.0, 162.4061, 162)]


# plan 6.2 at k = 5: weight = mix(progress, shaped, intensity); progress 0.385127, shaped 0.330723 -> 0.357925 at 0.5 by hand
HAND_WEIGHTS = [("smooth", 0.0, 0.385127), ("smooth", 0.5, 0.357925), ("smooth", 1.0, 0.330723), ("linear", 0.0, 0.385127), ("linear", 0.5, 0.385127),
                ("linear", 1.0, 0.385127)]
EXACT_PROBE_DECKS = [([200, 128, 16], [40, 128, 240]), ([255, 0, 0], [0, 0, 255])]


def literal_failures():
    """Layer 1 against whatever module functions are currently bound (so a mutant can be run through it)."""
    problems = []
    try:
        for alignment, duration, start, end, before, after in HAND_WINDOWS:
            window = d.raw_window(48, duration, alignment)
            if (window["start"], window["end"], window["before"], window["after"]) != (start, end, before, after):
                problems.append("window %s/%d" % (alignment, duration))
        for k, progress, weight in HAND_PROGRESS:
            p = d.progress_of(k, 12, "easeInOutCubic")
            w = d.weight_of(p, "smooth", 1.0)
            if abs(p - progress) > 5.0001e-7 or abs(w - weight) > 5.0001e-7:
                problems.append("progress k=%d" % k)
        if abs(d.progress_of(3, 12, "easeInOutCubic") - 0.099247685185) > 1e-12:
            problems.append("k=3 exact")
        for curve, amount, unrounded, code in HAND_INTENSITY:
            pixel = d.mix_pixel([200], [40], d.progress_of(5, 12, "easeInOutCubic"), curve, amount)
            if abs(pixel["unrounded_255"][0] - unrounded) > 5e-4 or pixel["rgb8"][0] != code:
                problems.append("intensity %s/%s" % (curve, amount))
        for curve, amount, weight in HAND_WEIGHTS:  # k = 5: progress 0.385127, shaped 0.330723, mix in between by hand
            if abs(d.weight_of(d.progress_of(5, 12, "easeInOutCubic"), curve, amount) - weight) > 5.0001e-7:
                problems.append("weight %s/%s" % (curve, amount))
        for deck_a, deck_b in EXACT_PROBE_DECKS:  # the plan's vectors cannot tell rounding from truncation: the exact reference can
            for k in range(12):
                for amount in (0.5, 1.0):
                    try:
                        got = d.mix_pixel(deck_a, deck_b, d.progress_of(k, 12, "easeInOutCubic"), "smooth", amount)["rgb8"]
                    except d.Refusal:
                        continue
                    wanted = exact_pixel(deck_a, deck_b, exact_weight(exact_progress(k, 12, "easeInOutCubic"), "smooth", Fraction(amount)))[0]
                    if got != wanted:
                        problems.append("exact pixel k=%d amount=%s" % (k, amount))
    except d.Refusal as refusal:
        problems.append("refusal %s" % refusal)
    return problems


class HandVectorTests(unittest.TestCase):
    def test_the_literals_of_this_file_agree_with_the_module(self):
        self.assertEqual(literal_failures(), [])

    def test_the_module_self_check_is_clean(self):
        self.assertEqual(d.self_check(), [])

    def test_the_windows_of_the_plan_at_c_48(self):
        for alignment, duration, start, end, before, after in HAND_WINDOWS:
            window = d.raw_window(48, duration, alignment)
            self.assertEqual((window["start"], window["end"], window["before"], window["after"]), (start, end, before, after), (alignment, duration))
            self.assertEqual(window["before"] + window["after"], duration)

    def test_k3_progress_is_the_eased_value_and_the_weight_is_after_smoothstep(self):
        progress = d.progress_of(3, 12, "easeInOutCubic")
        self.assertEqual(Fraction(progress).limit_denominator(10 ** 12), Fraction(1372, 13824))  # 4 * (7/24) ** 3
        weight = d.weight_of(progress, "smooth", 1.0)
        self.assertAlmostEqual(weight, 0.0275951, places=6)
        self.assertNotAlmostEqual(weight, progress, places=3)  # the review's B1: progress is NOT the weight

    def test_intensity_zero_is_the_eased_progress_not_zero_and_not_the_raw_t(self):
        progress = d.progress_of(5, 12, "easeInOutCubic")
        self.assertEqual(d.weight_of(progress, "smooth", 0.0), progress)
        self.assertNotEqual(progress, 5.5 / 12)
        self.assertGreater(progress, 0.3)

    def test_curve_linear_ignores_intensity_so_plain_equals_curved(self):
        progress = d.progress_of(5, 12, "easeInOutCubic")
        for amount in (0.0, 0.25, 0.5, 1.0):
            self.assertEqual(d.weight_of(progress, "linear", amount), progress)

    def test_the_plan_colour_steps_for_channel_200_to_40(self):
        self.assertAlmostEqual(d.decode(200), 0.577580, places=6)
        self.assertAlmostEqual(d.decode(40), 0.021219, places=6)
        weight = d.weight_of(d.progress_of(5, 12, "easeInOutCubic"), "smooth", 1.0)
        self.assertAlmostEqual(weight, 0.330723, places=6)
        self.assertAlmostEqual(d.mix(d.decode(200), d.decode(40), weight), 0.393579, places=6)
        self.assertAlmostEqual(d.encode(0.393579) * 255.0, 168.39, places=2)

    def test_each_default_curve_pair_makes_the_first_and_last_three_frames_nearly_one_deck(self):
        for k in (0, 1, 2):
            self.assertLess(d.weight_of(d.progress_of(k, 12, "easeInOutCubic"), "smooth", 1.0), 0.01)
            self.assertGreater(d.weight_of(d.progress_of(11 - k, 12, "easeInOutCubic"), "smooth", 1.0), 0.99)


class WindowAndStrictTests(unittest.TestCase):
    def refusal(self, function, *args):
        with self.assertRaises(d.Refusal) as caught:
            function(*args)
        return caught.exception.code

    def test_odd_even_and_one_frame_windows_are_exact_and_partition_the_duration(self):
        for alignment in d.ALIGNMENTS:
            for duration in range(1, 30):
                window = d.raw_window(48, duration, alignment)
                self.assertEqual(window["end"] - window["start"], duration)
                self.assertEqual(window["before"] + window["after"], duration)
                self.assertGreaterEqual(window["before"], 0)
                self.assertGreaterEqual(window["after"], 0)

    def test_the_center_window_puts_the_extra_frame_after_the_cut(self):
        self.assertEqual((d.raw_window(48, 5, "center")["before"], d.raw_window(48, 5, "center")["after"]), (2, 3))
        self.assertEqual((d.raw_window(48, 1, "center")["before"], d.raw_window(48, 1, "center")["after"]), (0, 1))

    def test_edit_edges_are_valid_and_one_frame_beyond_is_refused_not_clamped(self):
        self.assertEqual(d.checked_window(50, 100, "center", 100)["start"], 0)
        self.assertEqual(d.checked_window(12, 12, "end", 100), {"start": 0, "end": 12, "before": 12, "after": 0})
        self.assertEqual(d.checked_window(88, 12, "start", 100)["end"], 100)
        self.assertEqual(self.refusal(d.checked_window, 11, 12, "end", 100), "TRANSITION_OUT_OF_RANGE")
        self.assertEqual(self.refusal(d.checked_window, 89, 12, "start", 100), "TRANSITION_OUT_OF_RANGE")
        self.assertEqual(self.refusal(d.checked_window, 50, 101, "center", 100), "TRANSITION_OUT_OF_RANGE")
        self.assertEqual(self.refusal(d.checked_window, 50, 0, "center", 100), "TRANSITION_OUT_OF_RANGE")

    def test_the_duration_is_checked_against_the_edit_before_any_window_arithmetic(self):
        self.assertEqual(self.refusal(d.checked_window, 5, 2 ** 53 - 1, "center", 100), "TRANSITION_OUT_OF_RANGE")

    def test_no_coercion_of_floats_bools_strings_or_none(self):
        for bad in (12.5, 12.0, True, False, "12", None, [12]):
            self.assertEqual(self.refusal(d.strict_int, bad, "x"), "ORACLE_NOT_AN_INTEGER", repr(bad))
            self.assertEqual(self.refusal(d.checked_window, 48, bad, "center", 100), "ORACLE_NOT_AN_INTEGER", repr(bad))

    def test_integers_beyond_2_pow_53_minus_1_are_refused(self):
        self.assertEqual(d.strict_int(2 ** 53 - 1, "x"), 2 ** 53 - 1)
        self.assertEqual(self.refusal(d.strict_int, 2 ** 53, "x"), "ORACLE_INTEGER_NOT_SAFE")
        self.assertEqual(self.refusal(d.strict_int, 2 ** 63, "x"), "ORACLE_INTEGER_OUT_OF_I64")
        self.assertEqual(self.refusal(d.strict_int, -2 ** 63 - 1, "x"), "ORACLE_INTEGER_OUT_OF_I64")

    def test_checked_arithmetic_refuses_instead_of_wrapping_or_saturating(self):
        self.assertEqual(d.checked(2 ** 63 - 1, "x"), 2 ** 63 - 1)
        self.assertEqual(d.checked(-2 ** 63, "x"), -2 ** 63)
        self.assertEqual(self.refusal(d.checked, 2 ** 63, "x"), "ORACLE_ARITHMETIC_OVERFLOW")
        self.assertEqual(self.refusal(d.checked, -2 ** 63 - 1, "x"), "ORACLE_ARITHMETIC_OVERFLOW")

    def test_the_seed_is_a_strict_uint32(self):
        self.assertEqual(d.strict_seed(0), 0)
        self.assertEqual(d.strict_seed(4294967295), 4294967295)
        for bad in (-1, 4294967296, 12.5, 1.0, True, "1", None):
            self.assertEqual(self.refusal(d.strict_seed, bad), "ORACLE_BAD_SEED", repr(bad))

    def test_availability_is_a_semantic_zero_floor_never_negative(self):
        self.assertEqual(d.available(60, 10, 40), 10)
        self.assertEqual(d.available(40, 5, 40), 0)  # the camera clip already runs out inside the shot
        self.assertEqual(d.available(0, 0, 48), 0)

    def test_unknown_alignment_is_refused(self):
        self.assertEqual(self.refusal(d.checked_window, 48, 12, "middle", 100), "ORACLE_UNKNOWN_ALIGNMENT")

    def test_the_longest_transition_two_shots_hold_matches_the_before_after_rule(self):
        for alignment in d.ALIGNMENTS:
            for from_frames in range(0, 8):
                for to_frames in range(0, 8):
                    for duration in range(1, 20):
                        fits = d.explicit_duration_fits(alignment, duration, from_frames, to_frames)
                        self.assertEqual(fits, duration <= d.max_transition_frames(alignment, from_frames, to_frames), (alignment, from_frames, to_frames, duration))

    def test_an_omitted_duration_is_cut_and_a_given_one_is_refused(self):
        self.assertEqual(d.default_duration("center", 40, 60), 24)
        self.assertEqual(d.default_duration("center", 5, 5), 10)
        self.assertEqual(self.refusal(d.default_duration, "center", 10, 0), "TRANSITION_TOO_LONG")
        self.assertFalse(d.explicit_duration_fits("center", 24, 5, 5))


class ProgressTests(unittest.TestCase):
    def test_progress_matches_the_exact_rational_for_every_frame_of_small_windows(self):
        for duration in (1, 2, 3, 4, 5, 7, 12, 13, 24, 60):
            for k in range(duration):
                for easing in d.EASINGS:
                    exact = exact_progress(k, duration, easing)
                    self.assertAlmostEqual(d.progress_of(k, duration, easing), float(exact), delta=1e-12, msg=(duration, k, easing))

    def test_progress_stays_inside_the_unit_interval_and_finite_at_extremes(self):
        for duration in (1, 2, 2 ** 31, 2 ** 52):
            for k in (0, duration - 1):
                for easing in d.EASINGS:
                    value = d.progress_of(k, duration, easing)
                    self.assertTrue(math.isfinite(value))
                    self.assertTrue(0.0 <= value <= 1.0, (duration, k, easing, value))

    def test_the_symmetry_p_k_plus_p_d_minus_1_minus_k_is_one_for_linear_and_cubic(self):
        for duration in (1, 2, 5, 12, 13, 24):
            for k in range(duration):
                for easing in ("linear", "easeInOutCubic"):
                    total = d.progress_of(k, duration, easing) + d.progress_of(duration - 1 - k, duration, easing)
                    self.assertAlmostEqual(total, 1.0, delta=1e-12)
        self.assertNotAlmostEqual(d.progress_of(0, 12, "easeOutCubic") + d.progress_of(11, 12, "easeOutCubic"), 1.0, places=3)

    def test_the_shape_weight_symmetry_holds_after_smoothstep(self):
        for k in range(12):
            first = d.weight_of(d.progress_of(k, 12, "easeInOutCubic"), "smooth", 1.0)
            second = d.weight_of(d.progress_of(11 - k, 12, "easeInOutCubic"), "smooth", 1.0)
            self.assertAlmostEqual(first + second, 1.0, delta=1e-12)

    def test_the_weight_matches_the_exact_rational_for_every_curve_and_amount(self):
        for k in range(12):
            for curve in d.CURVES:
                for amount in (0.0, 0.25, 0.5, 0.75, 1.0):
                    progress = exact_progress(k, 12, "easeInOutCubic")
                    wanted = exact_weight(progress, curve, Fraction(amount))
                    got = d.weight_of(d.progress_of(k, 12, "easeInOutCubic"), curve, amount)
                    self.assertAlmostEqual(got, float(wanted), delta=1e-12)

    def test_progress_refuses_a_frame_outside_the_window_and_a_non_strict_duration(self):
        for k, duration in ((12, 12), (-1, 12), (0, 0), (0, 12.5), (True, 12)):
            with self.assertRaises(d.Refusal):
                d.progress_of(k, duration, "linear")


class PixelTests(unittest.TestCase):
    PAIRS = [([200, 128, 16], [40, 128, 240]), ([255, 0, 0], [0, 0, 255]), ([5, 250, 128], [128, 5, 250])]

    def test_the_oracle_agrees_with_the_decimal_reference_over_a_grid(self):
        compared = refused = 0
        for duration in (1, 2, 3, 7, 12, 24):
            for k in range(duration):
                for easing in d.EASINGS:
                    progress = exact_progress(k, duration, easing)
                    for curve in d.CURVES:
                        for amount in (0.0, 0.25, 0.5, 1.0):
                            weight = exact_weight(progress, curve, Fraction(amount))
                            for deck_a, deck_b in self.PAIRS[:2]:
                                codes, margin = exact_pixel(deck_a, deck_b, weight)
                                try:
                                    got = d.mix_pixel(deck_a, deck_b, d.progress_of(k, duration, easing), curve, amount)
                                except d.Refusal as refusal:
                                    self.assertEqual(refusal.code, "ORACLE_TIE_MARGIN")
                                    self.assertLess(margin, Decimal("1e-5"), "a refusal needs a real near-tie")
                                    refused += 1
                                    continue
                                self.assertEqual(got["rgb8"], codes, (duration, k, easing, curve, amount, deck_a))
                                self.assertEqual(got["alpha8"], 255)
                                compared += 1
        self.assertEqual(compared + refused, 2352)  # 49 frames x 3 easings x 2 curves x 4 amounts x 2 deck pairs
        self.assertGreater(compared, 2300)
        self.assertLess(refused, compared // 100, "near-tie refusals must stay rare")

    def test_the_plan_vector_codes_are_162_165_168(self):
        progress = d.progress_of(5, 12, "easeInOutCubic")
        self.assertEqual([d.mix_pixel([200], [40], progress, "smooth", amount)["rgb8"][0] for amount in (0.0, 0.5, 1.0)], [162, 165, 168])

    def test_equal_decks_stay_equal_at_every_code_and_the_extremes_are_exact(self):
        for code in range(256):
            self.assertEqual(d.mix_pixel([code], [code], 0.37, "smooth", 0.5)["rgb8"], [code])
        self.assertEqual(d.mix_pixel([0, 255], [255, 0], 0.0, "linear", 1.0)["rgb8"], [0, 255])
        self.assertEqual(d.mix_pixel([0, 255], [255, 0], 1.0, "linear", 1.0)["rgb8"], [255, 0])

    def test_mixing_in_linear_light_is_not_the_gamma_mix(self):
        linear = d.mix_pixel([0], [255], 0.5, "linear", 0.0)["rgb8"][0]
        self.assertEqual(linear, 188)  # encode(0.5) * 255
        self.assertNotEqual(linear, 128)

    def test_input_colours_are_quantized_half_up_before_the_mix(self):
        self.assertEqual(d.quantize_input(0.5), 128)
        self.assertEqual(d.quantize_input(0.0019), 0)  # 0.4845 codes
        self.assertEqual(d.quantize_input(0.002), 1)  # 0.51 codes
        self.assertEqual(d.quantize_input(0.5 / 255.0), 1)  # an exact tie goes up (half-up), not to even
        self.assertEqual(d.quantize_input(1.0), 255)
        for bad in (True, None, "1", float("nan"), float("inf")):
            with self.assertRaises(d.Refusal):
                d.quantize_input(bad)

    def test_the_decode_encode_roundtrip_is_exact_for_all_eight_bit_codes(self):
        for code in range(256):
            self.assertEqual(d.quant8(d.encode(d.decode(code))), code)

    def test_output_rounding_is_half_up(self):
        self.assertEqual(d.quant8(0.5 / 255.0 + 1e-9), 1)
        self.assertEqual(d.quant8(0.5 / 255.0 - 1e-9), 0)
        self.assertEqual(d.quant8(1.5 / 255.0 + 1e-9), 2)

    def test_the_tracer_pixel_equals_the_decimal_reference(self):
        tracer = CORPUS["tracer_bullet"]
        weight = exact_weight(exact_progress(3, 12, "easeInOutCubic"), "smooth", Fraction(1))
        codes, margin = exact_pixel(tracer["pixel_constant_decks"]["deck_a"], tracer["pixel_constant_decks"]["deck_b"], weight)
        self.assertGreater(margin, Decimal("1e-6"))
        self.assertEqual(tracer["pixel_constant_decks"]["expected"]["rgb8"], codes)


class DeckFrameTests(unittest.TestCase):
    def test_instance_and_camera_are_clamped_apart(self):
        shot = d.shot("s", 0, 40, "i", 10, "c", 5)
        row = d.resolve_shot_at_offset(shot, 60, 40, 35)
        self.assertEqual((row["instance_frame"], row["held_instance"]), (45, False))
        self.assertEqual((row["camera_clip_frame"], row["held_camera"]), (39, True))
        before = d.resolve_shot_at_offset(shot, 60, 40, -8)
        self.assertEqual((before["instance_frame"], before["held_instance"]), (2, False))
        self.assertEqual((before["camera_clip_frame"], before["held_camera"]), (0, True))

    def test_the_edit_clock_is_the_real_frame_even_when_both_clips_are_held(self):
        shot = d.shot("s", 40, 100, "i", 0, "c", 0)
        row = d.resolve_shot_at_offset(shot, 60, 60, -3)
        self.assertEqual((row["instance_frame"], row["camera_clip_frame"], row["held_instance"], row["held_camera"]), (0, 0, True, True))
        self.assertEqual(row["edit_frame"], 37)
        self.assertEqual(d.resolve_shot_at_offset(shot, 60, 60, 120)["edit_frame"], 160)

    def test_empty_clips_draw_frame_zero_and_report_held(self):
        shot = d.shot("e", 0, 10, "i", 0, "c", 0)
        row = d.resolve_shot_at_offset(shot, 0, 0, 5)
        self.assertEqual((row["instance_frame"], row["camera_clip_frame"], row["held_instance"], row["held_camera"]), (0, 0, True, True))

    def test_the_tracer_bullet_of_the_plan(self):
        tracer = CORPUS["tracer_bullet"]
        self.assertEqual((tracer["cut"], tracer["window"]["start"], tracer["window"]["end"]), (40, 34, 46))
        self.assertEqual((tracer["edit_frame"], tracer["frame_in_window"], tracer["primary"], tracer["other_shot"]), (37, 3, "from", "shot-b"))
        self.assertTrue(tracer["other_deck_frame"]["held_instance"] and tracer["other_deck_frame"]["held_camera"])
        self.assertEqual(tracer["other_deck_frame"]["edit_frame"], 37)
        self.assertAlmostEqual(tracer["progress"], 0.099248, places=6)
        self.assertAlmostEqual(tracer["weight_not_stored_in_the_plan"], 0.027595, places=6)
        self.assertIn("straight", tracer["alpha_note"])
        self.assertIn("NOT accepted by the origin", tracer["alpha_note"])

    def test_the_corpus_bundle_tail_of_the_instance_is_10_and_of_the_camera_is_0(self):
        rows = CORPUS["deck_frames"]["corpus_valid_cut_instance_offsets"]["from"]
        self.assertEqual((rows["tail_instance"], rows["tail_camera"]), (10, 0))
        self.assertEqual((rows["camera_frames"], rows["camera_in"]), (40, 5))

    def test_shot_clip_frame_edges(self):
        frame = d.shot_clip_frame
        self.assertEqual(frame(60, 12, 48, 96, 70), 70)  # inside the clip
        self.assertEqual(frame(60, 12, 48, 96, 72), None)  # the clip end is exclusive
        self.assertEqual(frame(48, 12, 48, 96, 44), 48)  # touches the start: holds its first frame in the head
        self.assertEqual(frame(60, 10, 48, 96, 44), None)  # a clip in the middle stays out
        self.assertEqual(frame(80, 16, 48, 96, 100), 95)  # touches the end: holds its last frame in the tail
        self.assertEqual(frame(80, 16, 48, 96, 96), 95)
        self.assertEqual(frame(80, 10, 48, 96, 100), None)
        self.assertEqual(frame(48, 12, 48, 96, 60), None)  # starts at the shot but the frame is after the clip

    def test_only_a_shot_end_touching_clip_holds_in_the_tail_by_clip_end(self):
        self.assertEqual(d.shot_clip_frame(80, 16, 48, 96, 140), 95)
        self.assertEqual(d.shot_clip_frame(80, 15, 48, 96, 140), None)


class ExplainTests(unittest.TestCase):
    def codes(self, edit, *transitions):
        return [issue["code"] for issue in d.explain_transitions(edit, list(transitions))]

    def setUp(self):
        self.edit = d.spec_edit()

    def test_a_valid_transition_has_no_issue(self):
        self.assertEqual(self.codes(self.edit, d.tr()), [])

    def test_the_family_is_not_an_unknown_effect(self):
        self.assertEqual(self.codes(self.edit, d.tr(effect="vignette")), ["WRONG_FAMILY"])
        self.assertEqual(self.codes(self.edit, d.tr(effect="nope")), ["UNKNOWN_EFFECT"])

    def test_a_look_with_parameters_and_keys_are_not_modelled_not_guessed(self):
        with self.assertRaises(d.Refusal) as caught:
            d.explain_transitions(self.edit, [d.tr(effect="vignette", params={"amount": 1})])
        self.assertEqual(caught.exception.code, "ORACLE_LOOK_PARAMS_NOT_MODELLED")
        with self.assertRaises(d.Refusal) as caught:
            d.explain_transitions(self.edit, [d.tr(keys={"intensity": []})])
        self.assertEqual(caught.exception.code, "ORACLE_KEYS_NOT_MODELLED")

    def test_an_integral_float_range_error_uses_the_origin_numeric_spelling(self):
        issues = d.explain_transitions(self.edit, [d.tr(params={"intensity": 2.0})])
        self.assertEqual(issues[0]["code"], "PARAM_OUT_OF_RANGE")
        self.assertIn("Parameter intensity is 2, outside", issues[0]["message"])
        self.assertNotIn("2.0", issues[0]["message"])

    def test_the_refusals_come_in_the_origin_order(self):
        self.assertEqual(self.codes(self.edit, d.tr(effect="nope", easing="bogus", from_id="zz")), ["UNKNOWN_EFFECT", "UNKNOWN_KEYFRAME_EASING", "UNKNOWN_SHOT"])
        self.assertEqual(self.codes(self.edit, d.tr(effect="vignette", easing="bogus")), ["WRONG_FAMILY", "UNKNOWN_KEYFRAME_EASING"])

    def test_parameter_codes(self):
        cases = [({"curve": "wobble"}, "PARAM_OUT_OF_RANGE"), ({"curve": 3}, "PARAM_TYPE"), ({"intensity": True}, "PARAM_TYPE"), ({"intensity": 1.5}, "PARAM_OUT_OF_RANGE"),
                 ({"intensity": -0.1}, "PARAM_OUT_OF_RANGE"), ({"intensity": float("nan")}, "PARAM_OUT_OF_RANGE"), ({"foo": 1}, "UNKNOWN_PARAM"),
                 ({"intensity": "1"}, "PARAM_TYPE")]
        for params, code in cases:
            self.assertEqual(self.codes(self.edit, d.tr(params=params)), [code], params)
        for params in ({"intensity": 0}, {"intensity": 1}, {"curve": "linear", "intensity": 0.5}):
            self.assertEqual(self.codes(self.edit, d.tr(params=params)), [], params)

    def test_adjacency_and_the_hint_names_what_comes_next(self):
        issues = d.explain_transitions(self.edit, [d.tr(from_id="s1", to_id="s3")])
        self.assertEqual([item["code"] for item in issues], ["TRANSITION_NOT_AT_CUT"])
        self.assertIn("after it comes s2", issues[0]["hint"])
        self.assertEqual(self.codes(self.edit, d.tr(from_id="s2", to_id="s1")), ["TRANSITION_NOT_AT_CUT"])
        self.assertIn("this is the last shot", d.explain_transitions(self.edit, [d.tr(from_id="s3", to_id="s1")])[0]["hint"])

    def test_a_duplicate_on_one_cut_and_next_to_a_legacy_dissolve(self):
        # the second one has the same window as the first, so it overlaps it too: both codes, in the origin's order
        self.assertEqual(self.codes(self.edit, d.tr(transition_id="a"), d.tr(transition_id="b")), ["TRANSITION_DUPLICATE", "TRANSITION_OVERLAP"])
        legacy = d.with_changes(self.edit, shot_changes={"s2": {"into": {"type": "dissolve", "duration": 12}}})
        issues = d.explain_transitions(legacy, [d.tr()])
        self.assertEqual([item["code"] for item in issues], ["TRANSITION_DUPLICATE"])
        self.assertIn("the dissolve of shot s2", issues[0]["message"])

    def test_duration_window_and_length_codes(self):
        self.assertEqual(self.codes(self.edit, d.tr(duration=0)), ["TRANSITION_OUT_OF_RANGE"])
        self.assertEqual(self.codes(self.edit, d.tr(duration=-5)), ["TRANSITION_OUT_OF_RANGE"])
        self.assertEqual(self.codes(self.edit, d.tr(from_id="s2", to_id="s3", duration=60, alignment="start")), ["TRANSITION_OUT_OF_RANGE", "TRANSITION_TOO_LONG"])
        self.assertEqual(self.codes(self.edit, d.tr(duration=48, alignment="end")), [])  # window [0, 48) exactly
        self.assertEqual(self.codes(self.edit, d.tr(duration=49, alignment="end")), ["TRANSITION_OUT_OF_RANGE", "TRANSITION_TOO_LONG"])

    def test_overlap_and_touching_windows(self):
        self.assertEqual(self.codes(self.edit, d.tr(transition_id="a", duration=60), d.tr(transition_id="b", from_id="s2", to_id="s3", duration=60)), ["TRANSITION_OVERLAP"])
        self.assertEqual(self.codes(self.edit, d.tr(transition_id="a", duration=24, alignment="start"),
                                    d.tr(transition_id="b", from_id="s2", to_id="s3", duration=24, alignment="end")), [])

    def test_handles_use_refuse_and_hold(self):
        use = d.explain_transitions(self.edit, [d.tr(handles="use")])
        self.assertEqual([item["code"] for item in use], ["TRANSITION_NO_HANDLES"])
        self.assertEqual(use[0]["lacks"], [{"shot": "s1", "side": "after its end", "instance": 0, "camera": 6},
                                           {"shot": "s2", "side": "before its start", "instance": 6, "camera": 6}])
        self.assertIn("s1 lacks 6 frames after its end (camera clip)", use[0]["message"])
        self.assertIn("s2 lacks 6 frames before its start (scene instance 6, camera clip 6)", use[0]["message"])
        for fix in ("extend its camera clip cam1 by 6 frames after its end", "give s2 6 more frames of its scene instance inst2 before its start",
                    "extend its camera clip cam2 by 6 frames before its start"):
            self.assertIn(fix, use[0]["hint"])
        refuse = d.explain_transitions(self.edit, [d.tr(handles="refuse")])
        self.assertEqual(refuse[0]["lacks"], use[0]["lacks"])  # refuse reads exactly as use in the calculation
        self.assertTrue(refuse[0]["hint"].endswith(d.REFUSE_NOTE))
        self.assertEqual(self.codes(self.edit, d.tr(handles="hold")), [])

    def test_handles_count_instance_and_camera_apart(self):
        only_neighbour = d.with_changes(self.edit, cameras={"cam1": 54})
        lacks = d.explain_transitions(only_neighbour, [d.tr(handles="use")])[0]["lacks"]
        self.assertEqual([item["shot"] for item in lacks], ["s2"])
        enough = d.with_changes(self.edit, cameras={"cam1": 54}, shot_changes={"s2": {"instance_start": 6, "camera_in": 6}})
        self.assertEqual(self.codes(enough, d.tr(handles="use")), [])
        only_camera = d.with_changes(self.edit, cameras={"cam1": 54}, shot_changes={"s2": {"instance_start": 6}})
        lacks = d.explain_transitions(only_camera, [d.tr(handles="use")])[0]["lacks"]
        self.assertEqual(lacks, [{"shot": "s2", "side": "before its start", "instance": 0, "camera": 6}])

    def test_a_camera_clip_shorter_than_its_shot_has_no_negative_tail(self):
        short = d.with_changes(self.edit, cameras={"cam1": 40})
        lacks = d.explain_transitions(short, [d.tr(handles="use")])[0]["lacks"]
        self.assertEqual(lacks[0], {"shot": "s1", "side": "after its end", "instance": 0, "camera": 6})

    def test_the_alpha_verdict_of_the_origin(self):
        verdict = d.origin_alpha_verdict
        self.assertEqual(verdict("opaque", 1, False), {"verdict": "accepted"})
        self.assertEqual(verdict("straight", 1, False), {"verdict": "refused", "code": "UNSUPPORTED_ALPHA"})
        self.assertEqual(verdict("premultiplied", 1, False), {"verdict": "refused", "code": "UNSUPPORTED_ALPHA"})
        self.assertEqual(verdict("straight", 0, True), {"verdict": "refused", "code": "UNSUPPORTED_ALPHA"})  # a look clip counts too
        self.assertEqual(verdict("straight", 0, False), {"verdict": "accepted"})  # no fx at all

    def test_ids(self):
        problems = d.id_problems
        self.assertEqual(problems("tr", []), [])
        self.assertEqual(problems("a" * 64, []), [])
        self.assertEqual(problems("a" * 65, []), ["ID_PATTERN"])
        for bad in ("", "-x", "_x", "tr 1", "tr/1", "é", "a\n", "tr.1"):
            self.assertEqual(problems(bad, []), ["ID_PATTERN"], repr(bad))
        for reserved in ("track", "FX", "Transition"):
            self.assertEqual(problems(reserved, []), ["RESERVED_ID"], reserved)
        self.assertEqual(problems("tr", ["tr"]), ["DUPLICATE_ID"])
        self.assertEqual(problems("Tr-1", []), [])  # the pattern is case-insensitive

    def test_the_plan_ceilings(self):
        self.assertEqual(d.plan_limit_verdict(4194304, 20000, 1), {"verdict": "accepted"})
        self.assertEqual(d.plan_limit_verdict(4194305, 1, 1)["detail"], "MAX_TRANSITION_PLAN_BYTES")
        self.assertEqual(d.plan_limit_verdict(1, 20001, 1)["detail"], "MAX_TRANSITION_PLAN_ITEMS")
        self.assertEqual(d.plan_limit_verdict(1, 1, 2)["path"], "/transition/other/transition")
        self.assertEqual(d.plan_limit_verdict(1, 1, 10 ** 9)["verdict"], "refused")  # a bomb is refused by a number, never by recursing

    def test_fx_limit_is_shots_minus_one(self):
        self.assertEqual(d.fx_limit_verdict(2, 1)["verdict"], "accepted")
        self.assertEqual(d.fx_limit_verdict(2, 2), {"max": 1, "verdict": "refused", "code": "FX_LIMIT"})
        self.assertEqual(d.fx_limit_verdict(0, 0)["max"], 0)
        self.assertEqual(d.fx_limit_verdict(1, 1)["verdict"], "refused")


class ResourceUnionTests(unittest.TestCase):
    def test_nodes_and_assets_collide_between_the_decks_and_nothing_crosses(self):
        union = CORPUS["resource_union"]
        self.assertEqual(union["colliding_node_ids_between_decks"], ["backdrop", "figure"])
        value = union["union"]["value"]
        self.assertEqual(list(value), ["backdrop", "figure"])
        for asset_id in ("backdrop", "figure"):
            self.assertEqual(value[asset_id]["used_by"], [{"deck": "primary", "node_id": asset_id}, {"deck": "other", "node_id": asset_id}])
        self.assertIn("no identifier crosses decks", union["no_cross_deck_identifier"])
        self.assertEqual(union["requires_added_by_a_transition"], ["fxTransition"])

    def test_the_same_asset_id_with_other_content_is_an_invalid_plan(self):
        outcome = CORPUS["resource_union"]["conflict"]["outcome"]
        self.assertEqual((outcome["ok"], outcome["code"]), (False, "RESOURCE_CONFLICT"))
        self.assertIn("/transition/other/resources", outcome["detail"])
        for field in ("revisionId", "contentHash", "relativePath"):
            primary = {"resources": {"a": {"revisionId": "r", "contentHash": "h", "relativePath": "p", "used_by": ["n"]}}}
            other = json.loads(json.dumps(primary))
            other["resources"]["a"][field] = "different"
            with self.assertRaises(d.Refusal):
                d.resource_union(primary, other)

    def test_an_asset_only_the_neighbour_uses_is_in_the_union_for_the_preflight(self):
        value = CORPUS["resource_union"]["only_the_neighbour_uses_an_asset"]["outcome"]["value"]
        self.assertEqual(value["sky"]["used_by"], [{"deck": "other", "node_id": "sky-plane"}])

    def test_the_union_order_is_by_asset_id_whatever_the_insertion_order(self):
        entry = {"revisionId": "r", "contentHash": "h", "relativePath": "p"}
        primary = {"resources": {"zeta": dict(entry, used_by=["n1"]), "alpha": dict(entry, used_by=["n2"])}}
        other = {"resources": {"mid": dict(entry, used_by=["n1"]), "alpha": dict(entry, used_by=["n1"])}}
        union = d.resource_union(primary, other)
        self.assertEqual(list(union), ["alpha", "mid", "zeta"])
        self.assertEqual(union["alpha"]["used_by"], [{"deck": "primary", "node_id": "n2"}, {"deck": "other", "node_id": "n1"}])


class AlphaAndTargetFactTests(unittest.TestCase):
    ROWS = {row["name"]: row for row in CORPUS["alpha"]["rows"]}

    def test_a_straight_case_is_refused_by_the_origin_and_accepted_by_the_destination_contract(self):
        row = self.ROWS["straight-with-a-declared-transition"]
        self.assertEqual(row["origin"], {"verdict": "refused", "code": "UNSUPPORTED_ALPHA"})
        self.assertTrue(row["destination_contract"]["verdict"].startswith("accepted"), row["destination_contract"])
        self.assertIn("DIVERGENCE", row["relation"])

    def test_an_opaque_case_is_accepted_by_both(self):
        row = self.ROWS["opaque-with-a-declared-transition"]
        self.assertEqual(row["origin"]["verdict"], "accepted")
        self.assertTrue(row["destination_contract"]["verdict"].startswith("accepted"))

    def test_no_row_calls_a_non_opaque_case_with_a_transition_accepted_by_the_origin(self):
        for row in CORPUS["alpha"]["rows"]:
            if row["alpha_mode"] != "opaque" and (row["transitions_drawn"] or row["has_look_clips"]):
                self.assertEqual(row["origin"]["verdict"], "refused", row["name"])
        self.assertIn("does NOT", CORPUS["alpha"]["never_say"])

    def test_the_legacy_dissolve_stays_refused_by_the_destination_until_s6(self):
        row = self.ROWS["opaque-with-a-legacy-dissolve"]
        self.assertEqual(row["destination_contract"], {"verdict": "refused", "code": "DISSOLVE_UNSUPPORTED"})
        self.assertEqual(row["origin"]["verdict"], "accepted")
        self.assertIn("S6", CORPUS["alpha"]["legacy_fixture"]["destination_says"])

    def test_the_legacy_fixture_is_untouched_and_still_self_consistent(self):
        expected = json.loads((d.ROOT / d.TARGET_FILES["legacy_expected"]).read_text(encoding="utf-8"))
        bundle = (d.ROOT / d.TARGET_FILES["legacy_bundle"]).read_bytes()
        self.assertEqual(expected["bundleSha256"], d.sha256_of(bundle))
        self.assertEqual(expected["validation"]["schema"]["issues"][0]["message"], "Dissolve transitions are not supported in pilot MVP")
        self.assertTrue(FACTS["dissolve_unsupported_in_parser"] and FACTS["premultiplied_refused"])

    def test_keys_and_explicit_hidden_false_are_refused_by_the_destination_contract(self):
        keys = CORPUS["keys_hidden_seed"]["keys"]
        self.assertIn("FX_TRANSITION_KEYS_NOT_YET", keys["destination_plan"])
        hidden = CORPUS["keys_hidden_seed"]["hidden"]
        self.assertIn("DECIDED", hidden["destination"])
        self.assertIn("structural refusal", hidden["destination"])
        self.assertIn("refused", hidden["cases"][2]["origin"])


class ToleranceModelTests(unittest.TestCase):
    """The gate is per-pixel max 1 AND a bias limit; a truncating port passes the first and fails the second."""

    def probes(self):
        rows = []
        for deck_a, deck_b in PixelTests.PAIRS:
            for k in range(12):
                for amount in (0.5, 1.0):
                    progress = d.progress_of(k, 12, "easeInOutCubic")
                    try:
                        pixel = d.mix_pixel(deck_a, deck_b, progress, "smooth", amount)
                    except d.Refusal:
                        continue
                    weight = exact_weight(exact_progress(k, 12, "easeInOutCubic"), "smooth", Fraction(amount))
                    for channel, (a, b) in enumerate(zip(deck_a, deck_b)):
                        rows.append((pixel["rgb8"][channel], pixel["unrounded_255"][channel], exact_pixel([a], [b], weight)[0][0]))
        return rows

    def test_a_correct_port_has_zero_bias_and_a_truncating_port_is_caught_by_the_bias_gate(self):
        rows = self.probes()
        limit = d.TOLERANCES["monitor_and_render"]["bias_limit"]
        self.assertGreaterEqual(len(rows), limit["min_probes_per_channel"] * 3)
        correct = [exact - oracle for oracle, _, exact in rows]
        self.assertEqual(max(abs(delta) for delta in correct), 0)
        truncated = [math.floor(unrounded) - oracle for oracle, unrounded, _ in rows]
        self.assertLessEqual(max(abs(delta) for delta in truncated), d.TOLERANCES["monitor_and_render"]["max_abs_delta_8bit"], "the per-pixel gate alone lets it through")
        mean = sum(truncated) / len(truncated)
        self.assertGreater(abs(mean), limit["max_abs_mean_signed_delta_8bit"], "the bias gate must fail it, got %.3f" % mean)
        self.assertLess(abs(sum(correct) / len(correct)), limit["max_abs_mean_signed_delta_8bit"])

    def test_the_registered_numbers(self):
        tolerances = CORPUS["tolerances"]
        self.assertEqual(tolerances["monitor_and_render"]["max_abs_delta_8bit"], 1)
        self.assertEqual(tolerances["monitor_and_render"]["bias_limit"]["max_abs_mean_signed_delta_8bit"], 0.25)
        self.assertEqual(tolerances["export_h264"]["max_abs_delta_8bit"], 16)
        self.assertIn("chroma subsampling", tolerances["export_h264"]["rationale"])
        self.assertIn("never retuned", tolerances["export_h264"]["discrimination_requirement"])
        self.assertIn("is an observation, not a criterion", tolerances["export_h264"]["not_used_as_tolerance"])
        self.assertNotEqual(tolerances["export_h264"]["max_abs_delta_8bit"], 7)
        self.assertIn("a tolerance is never retuned", tolerances["failure_policy"])
        self.assertIn("A validar", tolerances["status"])
        self.assertTrue(FACTS["f016_codec_text_present"] and FACTS["codec_tolerance_is_16"])

    def test_the_plan_ceilings_are_registered_as_contract_not_measurement(self):
        ceilings = CORPUS["tolerances"]["plan_ceilings"]
        self.assertEqual((ceilings["MAX_TRANSITION_PLAN_BYTES"], ceilings["MAX_TRANSITION_PLAN_ITEMS"], ceilings["TRANSITION_PLAN_MAX_DEPTH"]), (4194304, 20000, 1))
        self.assertIn("NOT measured", ceilings["status"])
        self.assertIn("A validar", ceilings["status"])

    def test_the_grid_has_enough_probes_for_the_bias_gate(self):
        grid = CORPUS["pixels"]["grid"]
        self.assertGreaterEqual(grid["probes_per_channel"], CORPUS["tolerances"]["monitor_and_render"]["bias_limit"]["min_probes_per_channel"])


class BaselineTests(unittest.TestCase):
    def test_the_generator_reproduces_the_pinned_corpus_and_manifest_byte_for_byte(self):
        corpus_bytes, manifest_bytes = d.expected_bytes()
        self.assertEqual(corpus_bytes, PINNED)
        self.assertEqual(manifest_bytes, PINNED_MANIFEST)

    def test_the_manifest_pins_the_corpus_it_ships_with(self):
        self.assertEqual(MANIFEST["corpus"]["sha256"], d.sha256_of(PINNED))

    def test_the_files_are_canonical_utf8_lf_json(self):
        for raw in (PINNED, PINNED_MANIFEST):
            text = raw.decode("utf-8")
            self.assertNotIn("\r", text)
            self.assertTrue(text.endswith("}\n"))
            self.assertEqual(d.canonical(json.loads(text)), raw)

    def test_the_blockers_stay_declared_and_the_origin_is_not_called_executed(self):
        ids = {blocker["id"] for blocker in MANIFEST["blockers"]}
        self.assertTrue({"ORIGIN_NOT_EXECUTED", "ORIGIN_GOLDEN_NOT_COMPARABLE", "PIN_COMMIT_ABSENT_FOR_THE_WINDOWS_AUTHOR", "NOT_THE_FEATURE"} <= ids)
        self.assertFalse(MANIFEST["origin"]["origin_executed"])
        self.assertNotIn("origin-output", PINNED.decode("utf-8"))

    def test_the_manifest_keeps_the_three_things_apart_the_pin_the_tip_and_the_attestation(self):
        origin = MANIFEST["origin"]
        self.assertEqual(origin["pin_named_by_the_plan"], d.PIN)
        self.assertEqual(origin["tip_commit_read"], d.TIP)
        self.assertIn("coordinator attested", json.dumps(origin).replace("what_the_coordinator_attested", "coordinator attested"))
        self.assertFalse(origin["observed_when_authoring"]["pin_commit_present"])

    def test_the_manifest_records_the_21_blobs_with_pin_and_tip_and_the_five_tip_only_files(self):
        attested = [item for item in MANIFEST["sources"] if "pin_blob_attested" in item]
        tip_only = [item for item in MANIFEST["sources"] if "pin_blob_attested" not in item]
        self.assertEqual((len(attested), len(tip_only)), (21, 5))
        different = sorted(item["path"].split("/src/")[-1] for item in attested if not item["pin_equals_tip"])
        self.assertEqual(different, ["fx/fx-schema.ts", "schema/render-bundle.schema.ts"])
        for item in MANIFEST["sources"]:
            self.assertEqual(len(item["tip_blob"]), 40, item["path"])
            self.assertEqual(len(item["tip_sha256"]), 64, item["path"])
            self.assertTrue(re.fullmatch(r"[0-9a-f]{40}", item.get("pin_blob_attested", "0" * 40)), item["path"])
        names = {item["path"] for item in attested}
        for needed in ("fx/resolve-fx-frame-transitions.spec.ts", "fx/fx-explain-legacy-dissolve.spec.ts", "fx/fx-explain-handles.spec.ts", "validate/edit-joins.ts",
                       "schema/render-bundle.schema.ts", "fx/fx-explain.ts", "fx/fx-windows.ts", "runtime/fx/shaders/crossDissolve.ts"):
            self.assertIn(d.SE + needed, names)

    def test_the_two_differences_are_notes_never_silent_adjustments(self):
        ids = {item["id"] for item in MANIFEST["documented_differences"]}
        self.assertTrue({"NARRATION_OUT_OF_SCOPE", "ALPHA_VERDICT", "DECKS_8_BIT", "KEYS_REFUSED", "HIDDEN_FALSE", "LEGACY_FIXTURES_UNTOUCHED"} <= ids)
        self.assertIn("comment", MANIFEST["origin"]["what_the_coordinator_attested"])
        self.assertIn("narration", MANIFEST["origin"]["what_the_coordinator_attested"])

    def test_the_manifest_records_the_plan_head_it_follows(self):
        self.assertEqual(MANIFEST["authoring"]["plan_head_read"], "2395719046875a06c4986e4e8a949a448bfcb9d3")

    def test_nothing_private_or_host_dependent_is_in_the_json(self):
        for raw in (PINNED, PINNED_MANIFEST):
            text = raw.decode("utf-8")
            self.assertIsNone(re.search(r"[A-Za-z]:[\\/]", text), "a drive path")
            self.assertIsNone(re.search(r"/(Users|home)/", text), "a home path")
            self.assertNotIn("wally", text.lower())
            self.assertEqual(sorted(set(re.findall(r"\d{4}-\d{2}-\d{2}", text))) or [], ["2026-11-14"] if "2026-11-14" in text else [])
        self.assertNotIn("python:", PINNED_MANIFEST.decode("utf-8"))

    def test_neither_script_uses_a_bare_assert(self):
        for name in ("pilot-dissolve-corpus.py", "pilot-dissolve-corpus.spec.py"):
            tree = ast.parse((HERE / name).read_text(encoding="utf-8"))
            self.assertEqual([node.lineno for node in ast.walk(tree) if isinstance(node, ast.Assert)], [], name)

    def test_the_windows_corpus_windows_equal_the_plain_formula(self):
        for row in CORPUS["windows"]["raw"]:
            cut, duration, alignment = row["cut"], row["duration"], row["alignment"]
            start = {"center": cut - duration // 2, "start": cut, "end": cut - duration}[alignment]
            self.assertEqual((row["window"]["start"], row["window"]["end"]), (start, start + duration))

    def test_the_corpus_extremes_equal_the_exact_rational(self):
        for row in CORPUS["progress"]["extremes"]:
            for easing in d.EASINGS:
                exact = float(exact_progress(row["k"], row["duration"], easing))
                self.assertAlmostEqual(row[easing]["progress"], exact, delta=max(1e-12, abs(exact) * 1e-9), msg=(row["duration"], row["k"], easing))

    def test_every_pixel_case_equals_the_decimal_reference(self):
        for case in CORPUS["pixels"]["cases"]:
            weight = exact_weight(exact_progress(case["frame_in_window"], case["duration"], case["easing"]), case["curve"], Fraction(case["intensity"]))
            codes, margin = exact_pixel(case["deck_a"], case["deck_b"], weight)
            self.assertGreater(margin, Decimal("1e-6"), case["id"])
            self.assertEqual(case["expected"]["rgb8"], codes, case["id"])
            self.assertAlmostEqual(case["weight"], float(weight), delta=1e-12, msg=case["id"])

    def test_the_new_plan_sections_exist_and_name_their_kind(self):
        for section in ("tracer_bullet", "limits", "resource_union"):
            self.assertIn(section, CORPUS)
        self.assertEqual(CORPUS["limits"]["kind"], "destination-contract-candidate")
        self.assertEqual(CORPUS["resource_union"]["kind"], "destination-contract-candidate")


# ---- layer 3: mutations ----------------------------------------------------------------------------------


def reference_pixel(deck_a, deck_b, progress, curve, intensity, *, gamma=False, ignore_amount=False, ignore_smooth=False):
    shaped = progress if ignore_smooth else d.shaped_of(progress, curve)
    amount = 1.0 if ignore_amount else d.clamp01(intensity)
    linear, unrounded, rgb8 = [], [], []
    for a8, b8 in zip(deck_a, deck_b):
        if gamma:
            weight = d.mix(progress, shaped, amount)
            encoded = d.mix(a8 / 255.0, b8 / 255.0, weight)
            out = d.decode(encoded * 255.0)
        else:
            a, b = d.decode(a8), d.decode(b8)
            out = d.mix(d.mix(a, b, progress), d.mix(a, b, shaped), amount)
            encoded = d.encode(out)
        linear.append(d.r9(out))
        unrounded.append(d.r9(encoded * 255.0))
        rgb8.append(d.quant8(encoded))
    return {"linear": linear, "unrounded_255": unrounded, "rgb8": rgb8, "alpha8": 255}


def corpus_under(**patches):
    try:
        if not patches:
            return d.canonical(d.build_corpus(FACTS))
        with patch.multiple(d, **patches):
            return d.canonical(d.build_corpus(FACTS))
    except d.Refusal as refusal:
        return ("refusal", refusal.code)


def literal_under(**patches):
    with patch.multiple(d, **patches):
        return literal_failures()


def coerce(value):
    """What a lenient port does: JS `Number(x) | 0` style, never a refusal."""
    try:
        return int(float(value))
    except (TypeError, ValueError, OverflowError):
        return 0


def swapped_window(cut, duration, alignment):
    return ORIG["raw_window"](cut, duration, {"start": "end", "end": "start", "center": "center"}[alignment])


def ceil_center_window(cut, duration, alignment):
    window = ORIG["raw_window"](cut, duration, alignment)
    if alignment == "center":
        start = cut - (duration + 1) // 2
        end = start + duration
        before = max(0, min(end, cut) - start)
        return {"start": start, "end": end, "before": before, "after": duration - before}
    return window


def joint_clamp(shot, instance_frames, camera_frames, delta):
    low = max(-shot["instance_start"], -shot["camera_in"])
    high = min(instance_frames - 1 - shot["instance_start"], camera_frames - 1 - shot["camera_in"])
    row = ORIG["resolve_shot_at_offset"](shot, instance_frames, camera_frames, max(low, min(high, delta)))
    row["delta"], row["edit_frame"] = delta, shot["start"] + delta
    return row


def clamped_clock(shot, instance_frames, camera_frames, delta):
    row = ORIG["resolve_shot_at_offset"](shot, instance_frames, camera_frames, delta)
    row["edit_frame"] = row["instance_frame"] - shot["instance_start"] + shot["start"]
    return row


def clip_end_inclusive(clip_start, clip_duration, shot_start, shot_end, edit_frame):
    clip_end = clip_start + clip_duration
    if clip_start <= edit_frame <= clip_end:
        return edit_frame
    return ORIG["shot_clip_frame"](clip_start, clip_duration, shot_start, shot_end, edit_frame)


def tail_off_by_one(clip_start, clip_duration, shot_start, shot_end, edit_frame):
    clip_end = clip_start + clip_duration
    if clip_start <= edit_frame < clip_end:
        return edit_frame
    if edit_frame < shot_start and clip_start == shot_start:
        return clip_start
    if edit_frame >= shot_end and clip_end == shot_end:
        return clip_end
    return None


def swapped_sides(transition, from_shot, to_shot, window, edit):
    return ORIG["handle_issue"](transition, from_shot, to_shot, dict(window, before=window["after"], after=window["before"]), edit)


def refuse_as_hold(transition, from_shot, to_shot, window, edit):
    return [] if transition["handles"] == "refuse" else ORIG["handle_issue"](transition, from_shot, to_shot, window, edit)


def use_as_hold(transition, from_shot, to_shot, window, edit):
    return [] if transition["handles"] == "use" else ORIG["handle_issue"](transition, from_shot, to_shot, window, edit)


def clamped_window(cut, duration, alignment, edit_duration):
    cut, duration, edit_duration = coerce(cut), coerce(duration), coerce(edit_duration)
    window = ORIG["raw_window"](cut, duration, alignment if alignment in d.ALIGNMENTS else "center")
    start, end = max(0, window["start"]), min(edit_duration, window["end"])
    return {"start": start, "end": end, "before": window["before"], "after": window["after"]}


def case_sensitive_ids(identifier, taken):
    problems = []
    if not isinstance(identifier, str) or re.fullmatch(r"[a-z0-9][a-z0-9_-]{0,63}", identifier) is None:
        problems.append("ID_PATTERN")
    if identifier in d.RESERVED_IDS:
        problems.append("RESERVED_ID")
    if identifier in taken:
        problems.append("DUPLICATE_ID")
    return problems


class MutationTests(unittest.TestCase):
    """Each mistake changes (or stops) the corpus; the formula mistakes also fail the hand vectors."""

    def assertCaught(self, name, literal=False, **patches):
        self.assertNotEqual(corpus_under(**patches), PINNED, "the corpus does not notice: " + name)
        if literal:
            self.assertNotEqual(literal_under(**patches), [], "the hand vectors do not notice: " + name)

    def test_the_unmutated_oracle_is_the_pinned_corpus(self):
        self.assertEqual(corpus_under(), PINNED)

    def test_the_progress_without_the_half_frame(self):
        self.assertCaught("no +0.5", literal=True, raw_t=lambda frame, duration: frame / duration)

    def test_the_easing_ignored(self):
        self.assertCaught("easing ignored", literal=True, apply_easing=lambda t, name: d.clamp01(t))

    def test_ceil_instead_of_round_in_the_output_code(self):
        self.assertCaught("floor to ceil", literal=True, quant8=lambda encoded: int(min(255, max(0, math.ceil(encoded * 255.0)))))

    def test_truncation_instead_of_rounding_in_the_output_code(self):
        self.assertCaught("truncation", literal=True, quant8=lambda encoded: int(min(255, max(0, math.floor(encoded * 255.0)))))

    def test_the_start_and_end_alignments_swapped(self):
        self.assertCaught("start/end swapped", literal=True, raw_window=swapped_window)

    def test_the_center_floor_changed_to_a_ceiling(self):
        self.assertCaught("floor(D/2) -> ceil(D/2)", literal=True, raw_window=ceil_center_window)

    def test_intensity_zero_treated_as_no_effect(self):
        self.assertCaught("intensity 0 zeroes the weight", literal=True, weight_of=lambda p, c, i: 0.0 if i == 0 else ORIG["weight_of"](p, c, i))

    def test_intensity_zero_treated_as_the_raw_t_without_easing_is_not_the_plan_vector(self):
        self.assertNotEqual(literal_under(weight_of=lambda p, c, i: 5.5 / 12 if i == 0 else ORIG["weight_of"](p, c, i)), [])

    def test_the_amount_ignored(self):
        self.assertCaught("amount ignored", literal=True, mix_pixel=lambda a, b, p, c, i: reference_pixel(a, b, p, c, i, ignore_amount=True))

    def test_the_smooth_curve_ignored(self):
        self.assertCaught("smooth ignored", literal=True, mix_pixel=lambda a, b, p, c, i: reference_pixel(a, b, p, c, i, ignore_smooth=True),
                          shaped_of=lambda p, c: p)

    def test_the_mix_done_in_gamma_space(self):
        self.assertCaught("gamma-space mix", literal=True, mix_pixel=lambda a, b, p, c, i: reference_pixel(a, b, p, c, i, gamma=True))

    def test_the_deck_inputs_not_quantized_to_eight_bits(self):
        self.assertCaught("no deck quantization", quantize_input=lambda unit: unit * 255.0)

    def test_the_deck_inputs_truncated_instead_of_rounded(self):
        self.assertCaught("deck truncation", quantize_input=lambda unit: int(min(255, max(0, math.floor(unit * 255.0)))))

    def test_the_srgb_decode_with_a_plain_gamma(self):
        self.assertCaught("gamma 2.2 decode", literal=True, decode=lambda code: (code / 255.0) ** 2.2)

    def test_the_instance_and_camera_clamped_together(self):
        self.assertCaught("joint clamp", resolve_shot_at_offset=joint_clamp)

    def test_the_edit_clock_clamped_with_the_instance(self):
        self.assertCaught("clamped edit clock", resolve_shot_at_offset=clamped_clock)

    def test_a_clip_end_made_inclusive(self):
        self.assertCaught("clip end inclusive", shot_clip_frame=clip_end_inclusive)

    def test_the_tail_frame_off_by_one(self):
        self.assertCaught("tail off by one", shot_clip_frame=tail_off_by_one)

    def test_the_two_handle_sides_swapped(self):
        self.assertCaught("handle sides swapped", handle_issue=swapped_sides)

    def test_refuse_treated_as_hold(self):
        self.assertCaught("refuse as hold", handle_issue=refuse_as_hold)

    def test_use_treated_as_hold(self):
        self.assertCaught("use as hold", handle_issue=use_as_hold)

    def test_a_negative_availability(self):
        self.assertCaught("negative availability", available=lambda total, start, length: total - (start + length))

    def test_an_invalid_window_clamped_instead_of_refused(self):
        self.assertCaught("window clamped into the edit", checked_window=clamped_window)

    def test_the_strict_integer_coerced(self):
        self.assertCaught("int coercion", strict_int=lambda value, field: coerce(value))

    def test_a_seed_coerced(self):
        self.assertCaught("seed coercion", strict_seed=lambda value: coerce(value) & 0xFFFFFFFF)

    def test_the_id_pattern_made_case_sensitive(self):
        self.assertCaught("case-sensitive ids", id_problems=case_sensitive_ids)

    def test_the_alpha_verdict_accepting_a_straight_profile(self):
        self.assertCaught("origin accepts straight", origin_alpha_verdict=lambda mode, drawn, clips: {"verdict": "accepted"})

    def test_a_wrong_family_reported_as_an_unknown_effect(self):
        self.assertCaught("WRONG_FAMILY -> UNKNOWN_EFFECT", CATALOG={"crossDissolve": d.CATALOG["crossDissolve"]})

    def test_the_default_duration_not_cut(self):
        self.assertCaught("default duration not cut", default_duration=lambda alignment, from_frames, to_frames: 24)

    def test_the_fx_limit_off_by_one(self):
        self.assertCaught("FX_LIMIT shots, not shots - 1", fx_limit_verdict=lambda shots, count: {"max": shots, "verdict": "accepted"})

    def test_the_resource_union_unqualified_by_deck(self):
        def unqualified(primary, other):
            union = ORIG["resource_union"](primary, other)
            for entry in union.values():
                entry["used_by"] = [item["node_id"] for item in entry["used_by"]]
            return union

        self.assertCaught("used_by without the deck", resource_union=unqualified)

    def test_the_resource_conflict_not_detected(self):
        def lenient(primary, other):
            merged = {}
            for deck_name, deck in (("primary", primary), ("other", other)):
                for asset_id, entry in deck["resources"].items():
                    current = merged.setdefault(asset_id, dict(entry, used_by=[]))
                    current["used_by"].extend({"deck": deck_name, "node_id": node} for node in entry["used_by"])
            return {key: merged[key] for key in sorted(merged)}

        self.assertCaught("conflict accepted", resource_union=lenient)

    def test_a_tolerance_constant_changed(self):
        changed = json.loads(json.dumps(d.TOLERANCES))
        changed["monitor_and_render"]["max_abs_delta_8bit"] = 2
        self.assertCaught("tolerance 2", TOLERANCES=changed)

    def test_every_mutant_above_is_killed_by_a_distinct_symptom_not_one_shared_refusal(self):
        symptoms = {repr(corpus_under(raw_t=lambda frame, duration: frame / duration)), repr(corpus_under(quant8=lambda e: int(min(255, max(0, math.floor(e * 255.0))))))}
        self.assertEqual(len(symptoms), 2)


# ---- layer 4: tamper and CLI -----------------------------------------------------------------------------


class TamperAndCliTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        (self.dir / d.CASES_NAME).write_bytes(PINNED)
        (self.dir / d.MANIFEST_NAME).write_bytes(PINNED_MANIFEST)

    def tearDown(self):
        self.tmp.cleanup()

    def verify(self, *extra):
        return run_main("verify", "--out", str(self.dir), *extra)

    def put_corpus(self, mutate):
        value = json.loads(PINNED.decode("utf-8"))
        mutate(value)
        (self.dir / d.CASES_NAME).write_bytes(d.canonical(value))

    def put_manifest(self, mutate):
        value = json.loads(PINNED_MANIFEST.decode("utf-8"))
        mutate(value)
        (self.dir / d.MANIFEST_NAME).write_bytes(d.canonical(value))

    def snapshot(self):
        return {path.name: path.read_bytes() for path in sorted(self.dir.iterdir())}

    def test_a_clean_copy_verifies_offline_and_says_the_origin_was_not_re_read(self):
        code, out, err = self.verify()
        self.assertEqual(code, d.EXIT_OK, err)
        self.assertIn("origin blobs not re-read", out)
        self.assertIn("python: ", err)

    def test_offline_verify_is_idempotent_and_writes_nothing(self):
        before = self.snapshot()
        first = self.verify()
        second = self.verify()
        self.assertEqual((first[0], second[0]), (0, 0))
        self.assertEqual(self.snapshot(), before)

    def test_a_changed_number_in_the_corpus_fails(self):
        self.put_corpus(lambda value: value["tracer_bullet"].__setitem__("progress", 0.099249))
        code, _, err = self.verify()
        self.assertEqual(code, d.EXIT_FINDING)
        self.assertIn("edited", err)

    def test_a_changed_number_with_the_manifest_hash_updated_still_fails(self):
        self.put_corpus(lambda value: value["tracer_bullet"].__setitem__("progress", 0.099249))
        digest = d.sha256_of((self.dir / d.CASES_NAME).read_bytes())
        self.put_manifest(lambda value: value["corpus"].__setitem__("sha256", digest))
        code, _, err = self.verify()
        self.assertEqual(code, d.EXIT_FINDING)
        self.assertIn("not what the generator produces", err)

    def test_a_changed_hand_vector_in_the_corpus_fails(self):
        self.put_corpus(lambda value: value["pixels"]["cases"][0]["expected"]["rgb8"].__setitem__(0, 163))
        self.assertEqual(self.verify()[0], d.EXIT_FINDING)

    def test_an_expectation_kind_changed_to_an_origin_output_fails(self):
        self.put_corpus(lambda value: value["tracer_bullet"].__setitem__("kind", "origin-output"))
        code, _, err = self.verify()
        self.assertEqual(code, d.EXIT_FINDING)
        self.assertIn("origin output", err)

    def test_a_source_blob_id_or_digest_changed_in_the_manifest_fails(self):
        self.put_manifest(lambda value: value["sources"][0].__setitem__("tip_blob", "0" * 40))
        self.assertEqual(self.verify()[0], d.EXIT_FINDING)
        (self.dir / d.MANIFEST_NAME).write_bytes(PINNED_MANIFEST)
        self.put_manifest(lambda value: value["sources"][3].__setitem__("tip_sha256", "f" * 64))
        self.assertEqual(self.verify()[0], d.EXIT_FINDING)

    def test_a_pin_blob_changed_in_the_manifest_fails(self):
        self.put_manifest(lambda value: value["sources"][0].__setitem__("pin_blob_attested", "1" * 40))
        self.assertEqual(self.verify()[0], d.EXIT_FINDING)

    def test_a_dropped_blocker_fails_with_its_name(self):
        self.put_manifest(lambda value: value.__setitem__("blockers", [item for item in value["blockers"] if item["id"] != "ORIGIN_NOT_EXECUTED"]))
        code, _, err = self.verify()
        self.assertEqual(code, d.EXIT_FINDING)
        self.assertIn("blocker dropped from the manifest: ORIGIN_NOT_EXECUTED", err)

    def test_a_missing_or_unreadable_manifest_fails(self):
        (self.dir / d.MANIFEST_NAME).write_bytes(b"{not json")
        self.assertEqual(self.verify()[0], d.EXIT_FINDING)
        (self.dir / d.MANIFEST_NAME).unlink()
        self.assertEqual(self.verify()[0], d.EXIT_FINDING)

    def test_a_crlf_or_trailing_byte_is_a_finding(self):
        (self.dir / d.CASES_NAME).write_bytes(PINNED.replace(b"\n", b"\r\n"))
        self.assertEqual(self.verify()[0], d.EXIT_FINDING)
        (self.dir / d.CASES_NAME).write_bytes(PINNED + b" ")
        self.assertEqual(self.verify()[0], d.EXIT_FINDING)

    def test_check_writes_nothing_and_reports_a_difference(self):
        self.put_corpus(lambda value: value["tracer_bullet"].__setitem__("progress", 0.5))
        before = self.snapshot()
        code, _, err = run_main("generate", "--check", "--out", str(self.dir))
        self.assertEqual(code, d.EXIT_FINDING)
        self.assertIn("not what the generator produces today", err)
        self.assertEqual(self.snapshot(), before)

    def test_check_on_a_clean_copy_passes_and_check_on_an_empty_directory_creates_nothing(self):
        self.assertEqual(run_main("generate", "--check", "--out", str(self.dir))[0], d.EXIT_OK)
        empty = self.dir / "empty"
        empty.mkdir()
        code, _, err = run_main("generate", "--check", "--out", str(empty))
        self.assertEqual(code, d.EXIT_FINDING)
        self.assertIn("missing", err)
        self.assertEqual(list(empty.iterdir()), [])

    def test_generate_without_an_origin_is_origin_unavailable_and_writes_nothing(self):
        target = self.dir / "fresh"
        code, _, err = run_main("generate", "--out", str(target))
        self.assertEqual(code, d.EXIT_ORIGIN_UNAVAILABLE)
        self.assertIn("ORIGIN_UNAVAILABLE", err)
        self.assertFalse(target.exists())

    def test_generate_writes_the_pinned_bytes_and_is_idempotent(self):
        target = self.dir / "fresh"
        with patch.object(d, "check_origin", return_value=([], [])):
            self.assertEqual(run_main("generate", "--out", str(target), "--origin", "unused")[0], d.EXIT_OK)
            first = {path.name: path.read_bytes() for path in target.iterdir()}
            self.assertEqual(run_main("generate", "--out", str(target), "--origin", "unused")[0], d.EXIT_OK)
        self.assertEqual(first, {d.CASES_NAME: PINNED, d.MANIFEST_NAME: PINNED_MANIFEST})
        self.assertEqual({path.name: path.read_bytes() for path in target.iterdir()}, first)

    def test_a_failed_transcription_stops_generate_before_writing(self):
        target = self.dir / "fresh"
        with patch.object(d, "check_origin", return_value=(["a transcribed line changed"], [])):
            code, _, err = run_main("generate", "--out", str(target), "--origin", "unused")
        self.assertEqual(code, d.EXIT_FINDING)
        self.assertIn("a transcribed line changed", err)
        self.assertFalse(target.exists())

    def test_require_pin_offline_is_exit_4_for_verify_and_generate_check(self):
        # Exercise main/CLI without mocking the origin verifier, including missing output paths.
        for argv in (["verify", "--require-pin"], ["generate", "--check", "--require-pin"]):
            code, out, err = run_main(*argv)
            self.assertEqual(code, d.EXIT_PIN_UNVERIFIED)
            self.assertIn("needs --origin", err)
            self.assertNotIn("verified", out)

    def test_digests_requires_an_origin_as_a_usage_error(self):
        with self.assertRaises(SystemExit) as caught, redirect_stderr(io.StringIO()):
            d.main(["digests"])
        self.assertEqual(caught.exception.code, 2)

    def test_require_pin_exits_4_only_for_the_pin_and_1_when_anything_else_is_wrong(self):
        with patch.object(d, "check_origin", return_value=(["PIN_UNVERIFIED"], ["PIN_COMMIT_ABSENT: x"])):
            code, _, err = self.verify("--origin", "unused", "--require-pin")
        self.assertEqual(code, d.EXIT_PIN_UNVERIFIED)
        self.assertIn("PIN_COMMIT_ABSENT", err)
        with patch.object(d, "check_origin", return_value=(["PIN_UNVERIFIED", "a blob changed"], [])):
            self.assertEqual(self.verify("--origin", "unused", "--require-pin")[0], d.EXIT_FINDING)

    def test_an_unreachable_origin_is_exit_3_and_not_a_silent_offline_pass(self):
        bad = self.dir / "not-a-repo"
        bad.mkdir()
        code, out, err = self.verify("--origin", str(bad))
        self.assertEqual(code, d.EXIT_ORIGIN_UNAVAILABLE)
        self.assertIn("ORIGIN_UNAVAILABLE", err)
        self.assertNotIn("verified", out)

    def test_the_digests_command_on_an_unreachable_origin_is_exit_3(self):
        self.assertEqual(run_main("digests", "--origin", str(self.dir))[0], d.EXIT_ORIGIN_UNAVAILABLE)

    def test_a_usage_error_is_exit_2(self):
        with self.assertRaises(SystemExit) as caught, redirect_stderr(io.StringIO()):
            d.main(["frobnicate"])
        self.assertEqual(caught.exception.code, 2)


class InterpreterTests(unittest.TestCase):
    STORE_STUBS = [r"C:\Users\x\AppData\Local\Microsoft\WindowsApps\python.exe", r"C:\Users\x\AppData\Local\Microsoft\WindowsApps\python3.exe",
                   "C:/Users/x/AppData/Local/Microsoft/WindowsApps/python.exe", r"c:\users\x\appdata\local\microsoft\windowsapps\PYTHON.EXE"]

    def test_the_microsoft_store_stub_is_not_an_interpreter(self):
        for path in self.STORE_STUBS:
            self.assertIn("Microsoft Store stub", d.interpreter_problem(path, (3, 13), 100) or "", path)

    def test_a_real_store_install_and_a_normal_install_are_accepted(self):
        real_store = r"C:\Program Files\WindowsApps\PythonSoftwareFoundation.Python.3.13_3.13.1520.0_x64__qbz5n2kfra8p0\python.exe"
        self.assertIsNone(d.interpreter_problem(real_store, (3, 13), 100))
        self.assertIsNone(d.interpreter_problem(r"C:\Users\x\AppData\Local\Programs\Python\Python313\python.exe", (3, 13), 100))
        self.assertIsNone(d.interpreter_problem("/usr/bin/python3", (3, 9), 100))

    def test_a_running_real_store_package_alias_may_have_zero_bytes(self):
        alias = r"C:\Users\x\AppData\Local\Microsoft\WindowsApps\PythonSoftwareFoundation.Python.3.13_pkg\python.exe"
        self.assertIsNone(d.interpreter_problem(alias, (3, 13), 0))
        # sys.version is supplied by the running interpreter, so an old package is still refused.
        self.assertIsNotNone(d.interpreter_problem(alias, (3, 8), 0))
        self.assertIsNotNone(d.interpreter_problem(self.STORE_STUBS[0], (3, 13), 0))

    def test_an_empty_executable_a_zero_byte_file_and_an_old_python_are_refused(self):
        self.assertIsNotNone(d.interpreter_problem("", (3, 13), 100))
        self.assertIsNotNone(d.interpreter_problem(r"C:\x\python.exe", (3, 13), 0))
        self.assertIn("older than", d.interpreter_problem("/usr/bin/python3", (3, 8), 100))

    def test_the_interpreter_running_this_spec_is_accepted_and_the_banner_names_it(self):
        self.assertIsNone(d.interpreter_problem())
        self.assertEqual(sys.version_info[:2] >= d.MIN_PYTHON, True)

    def test_the_cli_refuses_under_a_stub_with_exit_5_and_writes_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "out"
            with patch.object(d, "interpreter_problem", return_value="the Microsoft Store stub"):
                for argv in (["generate", "--check", "--out", str(target)], ["verify", "--out", str(target)], ["digests", "--origin", tmp]):
                    code, out, err = run_main(*argv)
                    self.assertEqual(code, d.EXIT_BAD_INTERPRETER, argv)
                    self.assertIn("INTERPRETER_NOT_REAL", err)
                    self.assertEqual(out, "")
            self.assertFalse(target.exists())

    def test_every_cli_run_prints_the_interpreter_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / d.CASES_NAME).write_bytes(PINNED)
            (Path(tmp) / d.MANIFEST_NAME).write_bytes(PINNED_MANIFEST)
            _, _, err = run_main("verify", "--out", tmp)
        self.assertIn("python: " + sys.version.split()[0], err)


# ---- the origin verifier, on a throwaway git repository --------------------------------------------------

GIT_ENV = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1", GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@example.invalid",
               GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@example.invalid")


def git(repo, *args):
    done = subprocess.run(["git", "-C", str(repo), "-c", "commit.gpgsign=false", *args], capture_output=True, env=GIT_ENV, timeout=60)
    if done.returncode != 0:
        raise RuntimeError("git %s: %s" % (" ".join(args), done.stderr.decode("utf-8", "replace")))
    return done.stdout.decode("utf-8").strip()


class OriginVerifierTests(unittest.TestCase):
    """`check_origin` against a tiny repository shaped like the origin (the constants are patched; the git logic is real)."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name)
        git(self.repo, "init", "-q")
        base = self.repo / "packages/scene-engine/src"
        base.mkdir(parents=True)
        (base / "a.ts").write_text("export const a = 1;\n", encoding="utf-8")
        (base / "b.ts").write_text("export const b = 1;\n", encoding="utf-8")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "pin")
        self.pin = git(self.repo, "rev-parse", "HEAD")
        (base / "b.ts").write_text("export const b = 2; // tip\n", encoding="utf-8")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "tip")
        self.tip = git(self.repo, "rev-parse", "HEAD")
        blob = lambda ref, name: git(self.repo, "rev-parse", "%s:packages/scene-engine/src/%s" % (ref, name))
        self.attested = {"a.ts": (blob(self.pin, "a.ts"), blob(self.tip, "a.ts")), "b.ts": (blob(self.pin, "b.ts"), blob(self.tip, "b.ts"))}
        self.digest = {}
        for name in ("a.ts", "b.ts"):
            data = (base / name).read_bytes()
            self.digest[name] = (self.attested[name][1], d.sha256_of(data), len(data))

    def tearDown(self):
        self.tmp.cleanup()

    def run_check(self, require_pin=False, **overrides):
        constants = dict(TIP=self.tip, PIN=self.pin[:10], SOURCES=[("a.ts", "r"), ("b.ts", "r")], TIP_ONLY=[], ATTESTED=self.attested,
                         TIP_DIGEST=self.digest, check_against=lambda texts: [])
        constants.update(overrides)
        with patch.multiple(d, **constants):
            return d.check_origin(self.repo, require_pin)

    def repo_state(self):
        return git(self.repo, "rev-parse", "HEAD"), git(self.repo, "status", "--porcelain"), git(self.repo, "count-objects")

    def test_a_clean_origin_with_the_pin_present_has_no_finding_and_no_warning(self):
        self.assertEqual(self.run_check(), ([], []))

    def test_the_check_is_read_only(self):
        before = self.repo_state()
        self.run_check()
        self.run_check(PIN="0" * 10)
        self.assertEqual(self.repo_state(), before)

    def test_an_absent_pin_commit_is_a_typed_warning_never_silent(self):
        findings, warnings = self.run_check(PIN="0" * 10)
        self.assertEqual(findings, [])
        self.assertTrue(any(item.startswith("PIN_COMMIT_ABSENT") for item in warnings), warnings)

    def test_a_pin_blob_that_exists_by_id_is_reported_as_present_by_id_not_as_the_pin(self):
        _, warnings = self.run_check(PIN="0" * 10)
        self.assertTrue(any(item.startswith("PIN_BLOB_PRESENT_BY_ID: b.ts") for item in warnings), warnings)

    def test_a_pin_blob_that_is_not_an_object_is_unverified(self):
        attested = dict(self.attested, **{"b.ts": ("e" * 40, self.attested["b.ts"][1])})
        findings, warnings = self.run_check(PIN="0" * 10, ATTESTED=attested)
        self.assertEqual(findings, [])
        self.assertTrue(any(item.startswith("PIN_BLOB_UNVERIFIED: b.ts") for item in warnings), warnings)

    def test_require_pin_adds_the_typed_finding_when_the_pin_cannot_be_checked(self):
        attested = dict(self.attested, **{"b.ts": ("e" * 40, self.attested["b.ts"][1])})
        findings, _ = self.run_check(require_pin=True, PIN="0" * 10, ATTESTED=attested)
        self.assertEqual(findings, ["PIN_UNVERIFIED"])
        findings, _ = self.run_check(require_pin=True)  # the pin is present: nothing to refuse
        self.assertEqual(findings, [])

    def test_strict_pin_refuses_even_when_all_attested_blobs_are_present_by_id(self):
        # This is a real git repository. a.ts is identical at both refs and old b.ts is still
        # present by object id; none of this proves membership in the requested absent commit.
        for pin_blob, _ in self.attested.values():
            self.assertTrue(git(self.repo, "cat-file", "-t", pin_blob) == "blob")
        findings, warnings = self.run_check(require_pin=True, PIN="0" * 10)
        self.assertEqual(findings, ["PIN_UNVERIFIED"])
        self.assertTrue(any(w.startswith("PIN_COMMIT_ABSENT:") for w in warnings))
        self.assertTrue(any(w.startswith("PIN_BLOB_PRESENT_BY_ID: b.ts") for w in warnings))
        self.assertEqual(self.run_check(require_pin=True), ([], []))

    def test_a_pin_blob_different_from_the_attested_one_is_a_finding_when_the_pin_is_present(self):
        attested = dict(self.attested, **{"a.ts": ("d" * 40, self.attested["a.ts"][1])})
        findings, _ = self.run_check(ATTESTED=attested)
        self.assertEqual(findings, ["the pin blob is not the attested one: a.ts"])

    def test_a_tip_blob_that_is_not_the_attested_one_is_a_finding(self):
        attested = dict(self.attested, **{"a.ts": (self.attested["a.ts"][0], "c" * 40)})
        findings, _ = self.run_check(ATTESTED=attested)
        self.assertIn("the tip blob is not the one the coordinator attested: a.ts", findings)

    def test_a_tip_source_whose_digest_changed_is_a_finding(self):
        digest = dict(self.digest, **{"a.ts": (self.digest["a.ts"][0], "0" * 64, self.digest["a.ts"][2])})
        findings, _ = self.run_check(TIP_DIGEST=digest)
        self.assertEqual(findings, ["the tip blob changed or was tampered with: a.ts"])

    def test_a_source_without_a_recorded_digest_is_a_finding(self):
        findings, _ = self.run_check(TIP_DIGEST={"a.ts": self.digest["a.ts"]})
        self.assertEqual(findings, ["no recorded digest for the tip source (run `digests`): b.ts"])

    def test_a_failed_transcription_check_is_a_finding(self):
        findings, _ = self.run_check(check_against=lambda texts: ["the shader formula changed"])
        self.assertEqual(findings, ["the shader formula changed"])

    def test_a_missing_tip_commit_or_a_missing_file_or_a_non_repository_is_origin_unavailable(self):
        with self.assertRaises(d.OriginUnavailable):
            self.run_check(TIP="f" * 40)
        with self.assertRaises(d.OriginUnavailable):
            self.run_check(SOURCES=[("a.ts", "r"), ("missing.ts", "r")])
        with tempfile.TemporaryDirectory() as plain:
            with patch.multiple(d, TIP=self.tip), self.assertRaises(d.OriginUnavailable):
                d.check_origin(Path(plain), False)

    def test_the_real_ordering_of_the_checks_names_each_source_once(self):
        digest = {name: (blob, "0" * 64, size) for name, (blob, _, size) in self.digest.items()}
        findings, _ = self.run_check(TIP_DIGEST=digest)
        self.assertEqual(sorted(findings), ["the tip blob changed or was tampered with: a.ts", "the tip blob changed or was tampered with: b.ts"])


@unittest.skipUnless(ORIGIN_PATH, "set TESSEL_ORIGIN_PATH to a read-only clone of the origin to re-check the real blobs")
class RealOriginTests(unittest.TestCase):
    """Only with a clone: the transcribed lines are really in the tip blobs, and tampering with the text is noticed."""

    @classmethod
    def setUpClass(cls):
        cls.texts = {path: data.decode("utf-8") for path, (_, data) in d.read_tip(Path(ORIGIN_PATH)).items()}

    def test_the_tip_blobs_are_the_recorded_ones_and_the_transcription_holds(self):
        findings, warnings = d.check_origin(Path(ORIGIN_PATH), False)
        self.assertEqual(findings, [])
        self.assertTrue(all(item.split(":")[0] in {"PIN_COMMIT_ABSENT", "PIN_BLOB_UNVERIFIED", "PIN_BLOB_PRESENT_BY_ID"} for item in warnings), warnings)

    def test_at_least_one_transcribed_rule_file_notices_a_text_change(self):
        broken = 0
        for path, text in self.texts.items():
            if path not in dict(d.SOURCES) or path.endswith(".spec.ts"):
                continue
            for needle in ("+ 0.5", "smoothstep", "Math.floor", "UNSUPPORTED_ALPHA", "WRONG_FAMILY"):
                if needle in text:
                    changed = dict(self.texts, **{path: text.replace(needle, needle + " /*x*/", 1).replace("0.5", "0.6", 1)})
                    if d.check_against(changed):
                        broken += 1
                        break
        self.assertGreater(broken, 0, "no transcribed rule file noticed a text change")


if __name__ == "__main__":
    unittest.main()
