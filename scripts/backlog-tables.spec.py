#!/usr/bin/env python3
"""Every row of a table in the shared backlog keeps the columns of its header (BL165).

    python3 scripts/backlog-tables.spec.py

A row with an extra cell, a missing closing pipe or an unescaped `|` inside a code span moves part of
its text outside the columns of the table, and renderers drop the extra cells (markdownlint MD055 and
MD056, as the review of PR #53 reported for BL015.15). `\\|` is an escaped pipe and does not split a
cell. The backlog is where the evidence of every other task lives, so a row that hides it is a defect.

It only needs the standard library. The controls try the scan on synthetic tables with each defect,
so a scan that stopped finding anything would fail here and not pass in silence.
"""
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BACKLOG = ROOT / 'docs' / 'backlog.md'


def cells_of(row):
    """The cells of a table row, with the empty ones outside the first and the last pipe."""
    cells, current, at = [], '', 0
    while at < len(row):
        if row.startswith('\\|', at):
            current += '\\|'
            at += 2
            continue
        if row[at] == '|':
            cells.append(current)
            current = ''
        else:
            current += row[at]
        at += 1
    cells.append(current)
    return cells


def broken_rows(text):
    """`(line, start of the row, cells, expected)` of each row that leaves the columns of its header."""
    broken, expected = [], None
    for number, line in enumerate(text.split('\n'), 1):
        row = line.rstrip()
        if not row.startswith('|'):
            expected = None
            continue
        cells = cells_of(row)
        if expected is None:
            expected = len(cells)
            continue
        closed = row.endswith('|') and not row.endswith('\\|')
        if len(cells) != expected or not closed:
            broken.append((number, row[:32], len(cells), expected))
    return broken


class TheBacklogTables(unittest.TestCase):
    def test_every_row_keeps_the_columns_of_its_header(self):
        broken = broken_rows(BACKLOG.read_text(encoding='utf-8'))
        self.assertEqual([], broken, 'rows outside the columns of their table: (line, row, cells, expected)')

    def test_the_backlog_is_read_and_has_tables_to_scan(self):
        text = BACKLOG.read_text(encoding='utf-8')
        rows = [line for line in text.split('\n') if line.startswith('| BL')]
        self.assertGreater(len(rows), 400)


class TheControls(unittest.TestCase):
    HEAD = '| ID | Name |\n|---|---|\n'

    def test_a_table_that_keeps_its_columns_has_nothing_broken(self):
        self.assertEqual([], broken_rows(self.HEAD + '| 1 | one |\n| 2 | two |\n'))

    def test_a_row_without_its_closing_pipe_is_found(self):
        self.assertEqual([(4, '| 2 | two', 3, 4)], broken_rows(self.HEAD + '| 1 | one |\n| 2 | two\n'))

    def test_a_row_with_an_extra_cell_is_found(self):
        self.assertEqual([(3, '| 1 | one | extra |', 5, 4)], broken_rows(self.HEAD + '| 1 | one | extra |\n'))

    def test_an_unescaped_pipe_in_code_is_found_and_an_escaped_one_is_not(self):
        self.assertEqual([(3, '| 1 | `a|b` |', 5, 4)], broken_rows(self.HEAD + '| 1 | `a|b` |\n'))
        self.assertEqual([], broken_rows(self.HEAD + '| 1 | `a\\|b` |\n'))

    def test_each_table_has_the_width_of_its_own_header(self):
        two_tables = self.HEAD + '| 1 | one |\n\nText between them.\n\n| A | B | C |\n|---|---|---|\n| 1 | 2 | 3 |\n'
        self.assertEqual([], broken_rows(two_tables))
        self.assertEqual([(9, '| 1 | 2 |', 4, 5)], broken_rows(two_tables.replace('| 1 | 2 | 3 |', '| 1 | 2 |')))


if __name__ == '__main__':
    unittest.main()
