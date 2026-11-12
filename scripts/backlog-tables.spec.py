#!/usr/bin/env python3
"""Every row of a table in the shared backlog keeps the columns of its header (BL165).

    python3 scripts/backlog-tables.spec.py

A row with an extra cell, a missing closing pipe or an unescaped `|` inside a code span moves part of
its text outside the columns of the table, and renderers drop the extra cells (markdownlint MD055 and
MD056, as the review of PR #53 reported for BL015.15). `\\|` is an escaped pipe and does not split a
cell. A line without the leading pipe is a defect too: GFM keeps it inside the table (its text lands in
the first columns and whatever goes beyond the width of the header is dropped), and the tools that look
for `| BL` rows do not see it. As GFM says, a table ends at a blank line or where another block starts
(a heading, a quote, a fence, a list item, a thematic break or an HTML block), and a row or a block start
may be indented by up to three spaces. The backlog is where the evidence of every other task lives,
so a row that hides it is a defect.

It only needs the standard library. The controls try the scan on synthetic tables with each defect,
so a scan that stopped finding anything would fail here and not pass in silence.
"""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BACKLOG = ROOT / 'docs' / 'backlog.md'

# What ends a table without a blank line, once the indentation is out: an ATX heading, a quote, a fence, a list
# item, a thematic break or an HTML block (any line that opens with `<`, which can only err towards not reporting).
ANOTHER_BLOCK = re.compile(r'(#{1,6}(\s|$)|>|`{3}|~{3}|[-*+]\s|\d+[.)]\s|([-*_]\s*){3,}$|<)')


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
    """`(line, start of the row, cells, expected, what)` of each row that leaves its table.

    A table starts at a line with the leading pipe (its header) and lasts until a blank line or the
    start of another block. A line inside it counts as a row even without the leading pipe: that is
    how GFM reads it, and it is a defect of its own.
    """
    broken, expected = [], None
    for number, line in enumerate(text.split('\n'), 1):
        row = line.rstrip()
        indent = len(row) - len(row.lstrip(' '))
        body = row[indent:] if indent <= 3 else row
        if not body or (expected is None and not body.startswith('|')) or (expected is not None and ANOTHER_BLOCK.match(body)):
            expected = None
            continue
        leading = body.startswith('|')
        cells = cells_of(body if leading else '|' + body)
        if expected is None:
            expected = len(cells)
            continue
        closed = body.endswith('|') and not body.endswith('\\|')
        defects = [
            what
            for what, found in (('leading pipe', not leading), ('closing pipe', not closed), ('columns', len(cells) != expected))
            if found
        ]
        if defects:
            broken.append((number, row[:32], len(cells), expected, ', '.join(defects)))
    return broken


class TheBacklogTables(unittest.TestCase):
    def test_every_row_keeps_the_columns_of_its_header(self):
        broken = broken_rows(BACKLOG.read_text(encoding='utf-8'))
        self.assertEqual([], broken, 'rows outside the columns of their table: (line, row, cells, expected, what)')

    def test_the_backlog_is_read_and_has_tables_to_scan(self):
        text = BACKLOG.read_text(encoding='utf-8')
        rows = [line for line in text.split('\n') if line.startswith('| BL')]
        self.assertGreater(len(rows), 400)


class TheControls(unittest.TestCase):
    HEAD = '| ID | Name |\n|---|---|\n'

    def test_a_table_that_keeps_its_columns_has_nothing_broken(self):
        self.assertEqual([], broken_rows(self.HEAD + '| 1 | one |\n| 2 | two |\n'))

    def test_a_row_without_its_closing_pipe_is_found(self):
        self.assertEqual(
            [(4, '| 2 | two', 3, 4, 'closing pipe, columns')], broken_rows(self.HEAD + '| 1 | one |\n| 2 | two\n')
        )

    def test_a_row_with_an_extra_cell_is_found(self):
        self.assertEqual([(3, '| 1 | one | extra |', 5, 4, 'columns')], broken_rows(self.HEAD + '| 1 | one | extra |\n'))

    def test_an_unescaped_pipe_in_code_is_found_and_an_escaped_one_is_not(self):
        self.assertEqual([(3, '| 1 | `a|b` |', 5, 4, 'columns')], broken_rows(self.HEAD + '| 1 | `a|b` |\n'))
        self.assertEqual([], broken_rows(self.HEAD + '| 1 | `a\\|b` |\n'))

    def test_a_row_without_its_leading_pipe_is_found_even_when_it_has_the_right_width(self):
        self.assertEqual(
            [(3, '1 | one | extra |', 5, 4, 'leading pipe, columns')], broken_rows(self.HEAD + '1 | one | extra |\n')
        )
        self.assertEqual([(3, '1 | one |', 4, 4, 'leading pipe')], broken_rows(self.HEAD + '1 | one |\n'))

    def test_text_glued_to_the_last_row_belongs_to_the_table_and_is_found(self):
        glued = self.HEAD + '| 1 | one |\nText right after.\n'
        self.assertEqual([(4, 'Text right after.', 2, 4, 'leading pipe, closing pipe, columns')], broken_rows(glued))
        self.assertEqual([], broken_rows(self.HEAD + '| 1 | one |\n\nText after a blank line.\n'))

    def test_another_block_ends_the_table_without_a_blank_line(self):
        blocks = ('## A heading', '> A quote', '- A list item', '* A list item', '1. A numbered item', '```', '---', '***', '___',
                  '- - -', '<details>', '<!-- a note -->', '  ## An indented heading', '   > An indented quote')
        for block in blocks:
            self.assertEqual([], broken_rows(self.HEAD + '| 1 | one |\n' + block + '\n'), block)

    def test_a_row_indented_by_up_to_three_spaces_is_still_a_row_and_four_is_not(self):
        self.assertEqual([], broken_rows(self.HEAD + '  | 1 | one |\n   | 2 | two |\n'))
        self.assertEqual([], broken_rows('   | ID | Name |\n   |---|---|\n   | 1 | one |\n'))
        self.assertEqual(
            [(3, '   | 1 | one | extra |', 5, 4, 'columns')], broken_rows(self.HEAD + '   | 1 | one | extra |\n')
        )
        self.assertEqual(
            [(3, '    | 1 | one |', 5, 4, 'leading pipe, columns')], broken_rows(self.HEAD + '    | 1 | one |\n')
        )

    def test_each_table_has_the_width_of_its_own_header(self):
        two_tables = self.HEAD + '| 1 | one |\n\nText between them.\n\n| A | B | C |\n|---|---|---|\n| 1 | 2 | 3 |\n'
        self.assertEqual([], broken_rows(two_tables))
        self.assertEqual(
            [(9, '| 1 | 2 |', 4, 5, 'columns')], broken_rows(two_tables.replace('| 1 | 2 | 3 |', '| 1 | 2 |'))
        )


if __name__ == '__main__':
    unittest.main()
