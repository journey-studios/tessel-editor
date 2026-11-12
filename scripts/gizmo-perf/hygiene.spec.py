#!/usr/bin/env python3
"""Hygiene of the files of the gizmo performance suite (F055 / BL015.21).

    python3 scripts/gizmo-perf/hygiene.spec.py

The repository is public. The tracked files of this feature must not carry data of the machine
that produced them: an absolute path of a home or a volume, the user's name, the list of the
busiest processes, the name of a worktree, the folder of a private project. The raw output of a
measurement run is where those leak (the runner records its host), so this spec reads every
tracked file of the feature, compressed ones decompressed, and fails with `file:line: pattern`.

It only needs `git` and the standard library. A `git` that fails is an error of the spec, never a
file list that happens to be empty, and the controls below prove that the scan CAN fail: the
detector is tried on synthetic text and on an in-memory archive, and every pathspec must match at
least one tracked file (a mistyped one would scan nothing and pass).
"""
import gzip
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent

EVIDENCE = 'docs/migration/gizmo-perf-evidence'

# What the feature owns. The backlog, the feature matrix and the migration plan are NOT here: they
# are shared documents with older entries that this feature did not write.
PATHSPECS = (
    'scripts/gizmo-perf/',
    'docs/migration/fixtures/gizmo-perf/',
    'docs/features/scene-gizmo-performance*.md',
    'crates/core/tests/gizmo_perf_support/',
    'crates/core/tests/scene_gizmo_perf_*.rs',
    'crates/desktop/src/workstation/scene_gizmo_perf_tests.rs',
    'crates/core/src/scene_plan/probe.rs',
)

# Built by concatenation: this file is one of the files it scans and must not condemn itself.
# The allowlist is empty on purpose.
FORBIDDEN = (
    '/' + 'Users/',
    '/' + 'Volumes/',
    'wallys' + 'son',
    'top_cpu' + '_processes',
    '.t3/' + 'worktrees',
    'projects' + '/',
)


def git(*args):
    """The stdout of a git command. A failure raises: the scan must never see "nothing" by accident."""
    return subprocess.run(['git', *args], cwd=ROOT, check=True, capture_output=True).stdout


def tracked(*pathspecs):
    listing = git('ls-files', '-z', '--', *pathspecs).decode('utf-8')
    return [name for name in listing.split('\0') if name]


def decoded(name, data):
    """The text of a file; `.gz` and `.tgz` are decompressed first."""
    if name.endswith(('.gz', '.tgz')):
        data = gzip.decompress(data)
    return data.decode('utf-8', errors='replace')


def forbidden_in(text):
    """`(line, pattern)` for every forbidden pattern on every line of `text` (lines count from 1)."""
    return [(number, pattern)
            for number, line in enumerate(text.split('\n'), 1)
            for pattern in FORBIDDEN if pattern in line]


def scan(name, data):
    return [f'{name}:{number}: {pattern}' for number, pattern in forbidden_in(decoded(name, data))]


def feature_files():
    return tracked(*PATHSPECS)


class TheFeatureFiles(unittest.TestCase):
    def test_the_evidence_directory_is_not_tracked(self):
        self.assertEqual([], tracked(EVIDENCE))

    def test_no_feature_file_carries_development_machine_data(self):
        names = feature_files()
        self.assertTrue(names, 'the feature has no tracked file: the pathspecs match nothing')
        found = [hit for name in names for hit in scan(name, (ROOT / name).read_bytes())]
        self.assertEqual([], found, 'development-machine data in tracked files:\n' + '\n'.join(found))


class TheControls(unittest.TestCase):
    def test_every_pathspec_matches_a_tracked_file(self):
        for pathspec in PATHSPECS:
            with self.subTest(pathspec=pathspec):
                self.assertTrue(tracked(pathspec), f'`{pathspec}` matches no tracked file')

    def test_a_failing_git_is_an_error_not_an_empty_listing(self):
        with self.assertRaises(subprocess.CalledProcessError):
            git('ls-files', '--no-such-option')

    def test_the_six_agreed_patterns_are_scanned_and_nothing_is_allowed(self):
        # The other controls walk FORBIDDEN, so they would follow a pattern that was dropped from it.
        agreed = {'/' + 'Users' + '/', '/' + 'Volumes' + '/', 'wally' + 'sson', 'top_cpu' + '_processes',
                  '.t3' + '/worktrees', 'projects' + '/'}
        self.assertEqual(agreed, set(FORBIDDEN))
        self.assertEqual(len(agreed), len(FORBIDDEN))

    def test_the_detector_finds_each_pattern_and_says_which_line(self):
        for pattern in FORBIDDEN:
            with self.subTest(pattern=pattern):
                text = 'nothing wrong here\nthe line with ' + pattern + 'x and more\nclean again\n'
                self.assertEqual([(2, pattern)], forbidden_in(text))

    def test_the_detector_reports_every_line_and_pattern(self):
        text = FORBIDDEN[0] + 'a\nclean\n' + FORBIDDEN[1] + 'b ' + FORBIDDEN[2] + '\n'
        self.assertEqual([(1, FORBIDDEN[0]), (3, FORBIDDEN[1]), (3, FORBIDDEN[2])], forbidden_in(text))

    def test_the_detector_is_silent_on_clean_text(self):
        self.assertEqual([], forbidden_in('relative/path.rs\nnothing else\n'))

    def test_a_failure_names_the_file_the_line_and_the_pattern(self):
        data = ('clean\nstarts at ' + FORBIDDEN[0] + 'x\n').encode()
        self.assertEqual([f'some/file.txt:2: {FORBIDDEN[0]}'], scan('some/file.txt', data))

    def test_a_compressed_file_is_read_decompressed(self):
        data = gzip.compress(('clean\nran in ' + FORBIDDEN[4] + '/x\n').encode())
        self.assertEqual([f'report.json.gz:2: {FORBIDDEN[4]}'], scan('report.json.gz', data))
        self.assertEqual([], scan('report.json.gz', gzip.compress(b'clean\nstill clean\n')))


if __name__ == '__main__':
    unittest.main(verbosity=2)
