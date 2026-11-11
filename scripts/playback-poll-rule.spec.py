"""Tests of the decision rule of the BL157.6 playback poll A/B: that it is fixed in git before a run, that its numbers are the code's, and that it is the file's."""
import importlib.util
import re
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
spec = importlib.util.spec_from_file_location("playback_poll_ab", HERE / "playback-poll-ab.py")
ab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ab)
from ab import play as shared_play  # noqa: E402  (the runner put `scripts/` on the path)

RULE_FILE = ROOT / "docs" / "viewport-real-latency-ab-results" / "poll-ab" / "rule.txt"


class RuleStateTests(unittest.TestCase):
    """`rule_state` and `rule_identity` against a real git repository in a temporary folder."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        patch = mock.patch.object(ab, "ROOT", self.root)
        patch.start()
        self.addCleanup(patch.stop)
        self.file = self.root / ab.RULE_PATH
        self.git("init", "-q")
        self.git("config", "user.email", "test@example.invalid")
        self.git("config", "user.name", "test")
        self.git("config", "commit.gpgsign", "false")

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.root, capture_output=True, text=True, check=True).stdout.strip()

    def write(self, text):
        self.file.parent.mkdir(parents=True, exist_ok=True)
        self.file.write_text(text)

    def test_the_path_is_the_one_the_docstring_names(self):
        self.assertIn(f"`{ab.RULE_PATH}`", ab.__doc__)
        self.assertEqual(RULE_FILE, ROOT / ab.RULE_PATH)

    def test_a_missing_file_is_not_a_fixed_rule(self):
        committed, why = ab.rule_state()
        self.assertFalse(committed)
        self.assertIn("does not exist", why)

    def test_a_file_git_does_not_know_is_not_a_fixed_rule(self):
        self.write("the rule")
        committed, why = ab.rule_state()
        self.assertFalse(committed)
        self.assertIn("not tracked", why)

    def test_a_file_that_is_only_staged_is_not_a_fixed_rule(self):
        self.write("the rule")
        self.git("add", str(ab.RULE_PATH))
        committed, why = ab.rule_state()
        self.assertFalse(committed)
        self.assertIn("not committed", why)

    def test_a_committed_file_is_a_fixed_rule_until_it_is_changed(self):
        self.write("the rule")
        self.git("add", str(ab.RULE_PATH))
        self.git("commit", "-q", "-m", "rule")
        self.assertEqual(ab.rule_state(), (True, None))
        self.write("the rule, a little different")
        committed, why = ab.rule_state()
        self.assertFalse(committed)
        self.assertIn("not committed", why)

    def test_the_identity_is_the_hash_of_the_file_and_the_commit_that_last_changed_it(self):
        self.write("the rule")
        self.git("add", str(ab.RULE_PATH))
        self.git("commit", "-q", "-m", "rule")
        first = self.git("rev-parse", "HEAD")
        (self.root / "other.txt").write_text("unrelated")
        self.git("add", "other.txt")
        self.git("commit", "-q", "-m", "other")
        found = ab.rule_identity()
        self.assertEqual(found["rule_commit"], first)
        self.assertEqual(found["rule_sha256"], ab.sha256_file(self.file))
        self.assertRegex(found["rule_sha256"], r"^[0-9a-f]{64}$")


class RuleTests(unittest.TestCase):
    """The rule is written in the docstring of the runner and in `rule.txt` before any measurement; its numbers are the ones the code uses."""

    DOC = ab.__doc__

    @classmethod
    def block(cls):
        match = re.search(r"DECISION RULES.*?\n\n(?=Load is recorded)", cls.DOC, re.S)
        assert match, "the docstring has no DECISION RULES block"
        return match.group(0)

    def number(self, pattern):
        found = re.search(pattern, self.block(), re.S)
        self.assertIsNotNone(found, pattern)
        return float(found.group(1))

    def test_the_constants_are_the_numbers_the_rule_states(self):
        for pattern, scale, constant in (
            (r"fewer than (\d+) counted pairs", 1, ab.MIN_COUNTED_PAIRS),
            (r"\(of the (\d+) asked for\)", 1, 12),
            (r"fewer than (\d+)% of the `fixed` arm's pictures", 100, ab.CONTROL_SHORT_SHARE_MIN),
            (r"more than ([\d.]+) percentage points higher", 100, ab.SLOW_SHARE_MARGIN),
            (r"above ([\d.]+) times the fixed arm's", 1, ab.VERY_SLOW_FACTOR),
            (r"number of plays\s+of the deadline arm\) plus (\d+);", 1, ab.VERY_SLOW_PLUS),
            (r"skipped frames per play are more than ([\d.]+) higher", 1, ab.SKIPPED_PER_PLAY_MARGIN),
            (r"more than (\d+)% of the pictures came less than 30 ms", 100, ab.DEADLINE_SHORT_SHARE_MAX),
            (r"mean change lag \([^)]*\) is above ([\d.]+) ms", 1, ab.LAG_DEADLINE_MAX_MS),
            (r"or less than ([\d.]+) ms below the fixed arm's", 1, ab.LAG_GAIN_MIN_MS),
            (r"outside ([\d.]+)-[\d.]+;", 1, ab.POLLS_PER_CHANGE_RANGE[0]),
            (r"outside [\d.]+-([\d.]+);", 1, ab.POLLS_PER_CHANGE_RANGE[1]),
            (r"in at least (\d+)% of the counted pairs", 100, ab.LAG_PAIR_CONSISTENCY),
        ):
            self.assertAlmostEqual(self.number(pattern) / scale, constant, places=9, msg=pattern)

    def test_the_marks_of_the_pictures_are_the_ones_the_cadence_reader_and_the_hang_rule_use(self):
        text = self.block()
        self.assertIn("less than 30 ms after the previous", text)
        self.assertEqual(ab.cadence().SHORT_DELIVERY_MS, 30.0)
        self.assertIn("more than 62.5 ms after the previous", text)
        self.assertEqual(ab.cadence().SLOW_DELIVERY_MS, 62.5)
        self.assertIn("more than 100 ms after the previous", text)
        self.assertEqual(ab.cadence().VERY_SLOW_DELIVERY_MS, 100.0)
        self.assertIn("under 90% of the frames, or waits over 2000 ms", text)
        self.assertEqual((shared_play.HANG_PICTURES_SHARE, shared_play.HANG_GAP_MS), (0.9, 2000.0))

    def test_the_committed_rule_file_holds_the_decision_rules_word_for_word(self):
        self.assertTrue(RULE_FILE.exists(), f"{RULE_FILE.relative_to(ROOT)} has to be committed before any measurement")
        self.assertIn(self.block().rstrip(), RULE_FILE.read_text())

    def test_the_rule_file_names_what_it_decides_and_what_it_does_not(self):
        text = RULE_FILE.read_text()
        self.assertIn("TESSEL_PLAYBACK_POLL", text)
        self.assertIn("NOT an acceptance by the user", text)
        self.assertIn("NOT about the 60-160 ms pauses", text)
        self.assertNotIn("/Users/", text)


if __name__ == "__main__":
    unittest.main()
