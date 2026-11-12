#!/usr/bin/env python3
"""Every row of the shared backlog is in a table and keeps the columns of its header (BL165, BL166).

    python3 scripts/backlog-tables.spec.py

A row with an extra cell, a missing closing pipe or an unescaped `|` inside a code span moves part of
its text outside the columns of the table, and renderers drop the extra cells (markdownlint MD055 and
MD056, as the review of PR #53 reported for BL015.15). `\\|` is an escaped pipe and does not split a
cell. A line without the leading pipe is a defect too: GFM keeps it inside the table (its text lands in
the first columns and whatever goes beyond the width of the header is dropped), and the tools that look
for `| BL` rows do not see it. A run of rows with no header and delimiter row above it is no table at
all: GitHub shows it as one paragraph of text with bars (BL166). The backlog is where the evidence of
every other task lives, so a row that hides it is a defect.

The scan reads a table the way GitHub does. Each rule below was checked against GitHub's own renderer
(`gh api -X POST markdown -f mode=gfm`): a table is a header line followed right away by its
delimiter row, which has a pipe, as many cells as the header and only hyphens (one is enough) with an
optional colon on each side in each cell (a bare `---`, a blank line, a row or a delimiter of another
width in its place leave plain text, and a header may follow a line of text with no blank line; the scan
opens a table at a header with the leading pipe, as the backlog writes them); a table ends at a blank
line or where another block starts (a
heading, a quote, a fence, a list item, a thematic break of one repeated marker, or an HTML block: a
comment, a block-level tag, or a tag alone on its line); an inline tag with text after it, `a < b`,
`#tag`, `***bold***` and `--- x` stay rows; a row, a header, a delimiter or a block start may be indented
by up to three spaces, and four spaces or a tab make it code, which ends the table; nothing inside a fence
is a table.

It only needs the standard library. The controls try the scan on synthetic tables with each defect,
so a scan that stopped finding anything would fail here and not pass in silence.
"""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BACKLOG = ROOT / 'docs' / 'backlog.md'

# What ends a table without a blank line, once the indentation is out: an ATX heading, a quote, a list item (an empty one
# too), or a thematic break made of one repeated marker.
ANOTHER_BLOCK = re.compile(r'(#{1,6}(\s|$)|>|[-*+](\s|$)|\d{1,9}[.)](\s|$)|(\*\s*){3,}$|(-\s*){3,}$|(_\s*){3,}$)')

# The HTML blocks of CommonMark that can end a table: a comment, a processing instruction, CDATA or a declaration,
# script/pre/style/textarea, a block-level tag, or a complete tag alone on its line. `<span>x</span>` is none of them.
BLOCK_TAGS = (
    'address|article|aside|base|basefont|blockquote|body|caption|center|col|colgroup|dd|details|dialog|dir|div|dl|dt|'
    'fieldset|figcaption|figure|footer|form|frame|frameset|h[1-6]|head|header|hr|html|iframe|legend|li|link|main|menu|'
    'menuitem|nav|noframes|ol|optgroup|option|p|param|section|summary|table|tbody|td|tfoot|th|thead|title|tr|track|ul'
)
HTML_BLOCK = re.compile(
    r'<(!--|\?|!\[CDATA\[|![A-Za-z]|(script|pre|style|textarea)(\s|>|$)|/?(' + BLOCK_TAGS + r')(\s|/?>|$)'
    r'|/?[A-Za-z][A-Za-z0-9-]*(\s[^>]*)?/?>\s*$)',
    re.IGNORECASE,
)

# A backtick fence has no backtick in its info string (else the line is a code span); a tilde fence may have anything.
FENCE = re.compile(r'(?:(`{3,})[^`]*|(~{3,}).*)$')

# One cell of a delimiter row: hyphens (at least one) with an optional colon on each side, and spaces around them.
DELIMITER_CELL = re.compile(r'\s*:?-+:?\s*')

# What `broken_rows` says of a row that has no table: no header and delimiter row open one above it.
NOT_A_TABLE = 'no delimiter row, not a table'


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


def another_block_starts(body):
    return bool(ANOTHER_BLOCK.match(body) or HTML_BLOCK.match(body) or FENCE.match(body))


def cells_between_pipes(row):
    """The cells of a header or delimiter row, as GFM counts them: the outer pipes are optional."""
    stripped = row.strip()
    cells = cells_of(stripped)
    if stripped.startswith('|'):
        cells = cells[1:]
    if stripped.endswith('|') and not stripped.endswith('\\|'):
        cells = cells[:-1]
    return cells


def is_delimiter_row(line, width):
    """Whether `line` is the delimiter row of a header with `width` cells."""
    row = line.rstrip()
    lead = len(row) - len(row.lstrip(' \t'))
    if len(row[:lead].expandtabs(4)) > 3 or '|' not in row:  # indented code, or a bare `---` (a setext heading)
        return False
    cells = cells_between_pipes(row)
    return len(cells) == width and all(DELIMITER_CELL.fullmatch(cell) for cell in cells)


def broken_rows(text):
    """`(line, start of the row, cells, expected, what)` of each row that leaves its table.

    A table starts at a line with the leading pipe (its header) when the next line is its delimiter row,
    and lasts until a blank line or the start of another block. A line with the leading pipe that opens
    no table is a row of no table (`expected` is 0): GitHub shows it as text. A line inside a table
    counts as a row even without the leading pipe: that is how GFM reads it, and it is a defect of its own.
    """
    broken, expected, fence, opened_at, delimiter_next = [], None, None, 0, False
    lines = text.split('\n')
    for number, line in enumerate(lines, 1):
        if delimiter_next:  # the delimiter row of the header above: `is_delimiter_row` checked it already
            delimiter_next = False
            continue
        row = line.rstrip()
        lead = len(row) - len(row.lstrip(' \t'))
        indent = len(row[:lead].expandtabs(4))
        body = row[lead:]
        if fence:  # nothing inside a fenced block is a table; a closing fence is as long and has no text
            if indent <= 3 and re.fullmatch(re.escape(fence[0]) + '{%d,}' % len(fence) + r'\s*', body):
                fence = None
            continue
        if not body:
            expected = None
            continue
        if indent > 3:  # indented code: the table ends here, and a row that lands in it is lost to the table
            if expected is not None and body.startswith('|'):
                broken.append((number, row[:32], len(cells_of(body)), expected, 'indented code, not a row'))
            expected = None
            continue
        opening = FENCE.match(body)
        if opening:
            fence, opened_at, expected = opening.group(1) or opening.group(2), number, None
            continue
        if expected is None and not body.startswith('|'):
            continue
        if expected is not None and another_block_starts(body):
            expected = None
            continue
        leading = body.startswith('|')
        cells = cells_of(body if leading else '|' + body)
        if expected is None:
            next_line = lines[number] if number < len(lines) else ''
            if not is_delimiter_row(next_line, len(cells_between_pipes(body))):
                broken.append((number, row[:32], len(cells), 0, NOT_A_TABLE))
                continue
            expected, delimiter_next = len(cells), True
            continue
        closed = body.endswith('|') and not body.endswith('\\|')
        defects = [
            what
            for what, found in (('leading pipe', not leading), ('closing pipe', not closed), ('columns', len(cells) != expected))
            if found
        ]
        if defects:
            broken.append((number, row[:32], len(cells), expected, ', '.join(defects)))
    if fence:  # an open fence runs to the end of the document and hides everything below it from the scan and the reader
        broken.append((opened_at, fence, 0, 0, 'unclosed fence'))
    return broken


class TheBacklogTables(unittest.TestCase):
    def test_every_row_is_in_a_table_and_keeps_the_columns_of_its_header(self):
        broken = broken_rows(BACKLOG.read_text(encoding='utf-8'))
        self.assertEqual([], broken, 'rows with no table or outside its columns: (line, row, cells, expected, what)')

    def test_the_backlog_is_read_and_has_tables_to_scan(self):
        text = BACKLOG.read_text(encoding='utf-8')
        rows = [line for line in text.split('\n') if line.startswith('| BL')]
        self.assertGreater(len(rows), 400)


class TheControls(unittest.TestCase):
    HEAD = '| ID | Name |\n|---|---|\n'
    ROW = '| 1 | one |\n'

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
        glued = self.HEAD + self.ROW + 'Text right after.\n'
        self.assertEqual([(4, 'Text right after.', 2, 4, 'leading pipe, closing pipe, columns')], broken_rows(glued))
        self.assertEqual([], broken_rows(self.HEAD + self.ROW + '\nText after a blank line.\n'))

    def test_what_starts_another_block_ends_the_table_without_a_blank_line(self):
        blocks = (
            '## A heading', '> A quote', '- A list item', '* A list item', '+ A list item', '-', '1. A numbered item',
            '10) A numbered item', '2.', '---', '***', '___', '- - -', '<details>', '<!-- a note -->', '<br>', '<custom-tag>',
            '</div>', '<div>x</div>', '<p>para', '<?php echo 1; ?>', '<!DOCTYPE html>', '<script>x</script>',
            '<table><tr><td>x</td></tr></table>', '  ## An indented heading', '   > An indented quote',
        )
        for block in blocks:
            self.assertEqual([], broken_rows(self.HEAD + self.ROW + block + '\n'), block)

    def test_what_is_only_text_stays_a_row_of_the_table_and_is_found(self):
        texts = (
            '***bold***', '-- two dashes', '--- with text', '===', '#nospace', '<span>x</span>', '<em>x</em> text',
            '<a href="x">link</a> text', 'a < b', '<3 hearts', '```a``` text', '``` a`b',
        )
        for text in texts:
            found = broken_rows(self.HEAD + self.ROW + text + '\n')
            self.assertEqual(1, len(found), text)
            self.assertIn('leading pipe', found[0][4], text)

    def test_a_fence_ends_the_table_and_hides_what_is_inside_it(self):
        fences = (('```', '```'), ('~~~ python', '~~~'), ('````', '````'), ('   ```', '   ```'), ('``` ~x', '```'),
                  ('~~~ `x`', '~~~'))
        for opening, closing in fences:
            fenced = self.HEAD + self.ROW + opening + '\n| A | B | C |\n| 1 | 2 |\n' + closing + '\n' + self.HEAD + self.ROW
            self.assertEqual([], broken_rows(fenced), opening)
        self.assertEqual([(4, '```', 0, 0, 'unclosed fence')], broken_rows(self.HEAD + self.ROW + '```\n| 1 | 2 | 3 |\n'))

    def test_a_row_indented_by_up_to_three_spaces_is_still_a_row(self):
        self.assertEqual([], broken_rows(self.HEAD + '  | 1 | one |\n   | 2 | two |\n'))
        self.assertEqual([], broken_rows('   | ID | Name |\n   |---|---|\n   | 1 | one |\n'))
        self.assertEqual(
            [(3, '   | 1 | one | extra |', 5, 4, 'columns')], broken_rows(self.HEAD + '   | 1 | one | extra |\n')
        )

    def test_four_spaces_or_a_tab_make_code_that_ends_the_table_and_a_row_there_is_found(self):
        for indent in ('    ', '\t', '  \t'):
            self.assertEqual(
                [(4, (indent + '| 2 | two |')[:32], 4, 4, 'indented code, not a row')],
                broken_rows(self.HEAD + self.ROW + indent + '| 2 | two |\n'),
                repr(indent),
            )
        self.assertEqual([], broken_rows(self.HEAD + self.ROW + '    just code\n'))
        self.assertEqual([], broken_rows('    | 1 | one | extra |\n'))

    def test_each_table_has_the_width_of_its_own_header(self):
        two_tables = self.HEAD + '| 1 | one |\n\nText between them.\n\n| A | B | C |\n|---|---|---|\n| 1 | 2 | 3 |\n'
        self.assertEqual([], broken_rows(two_tables))
        self.assertEqual(
            [(9, '| 1 | 2 |', 4, 5, 'columns')], broken_rows(two_tables.replace('| 1 | 2 | 3 |', '| 1 | 2 |'))
        )

    def test_rows_with_no_header_and_delimiter_are_no_table_and_each_one_is_found(self):
        orphans = '| 1 | one |\n| 2 | two |\n'
        expected = [(1, '| 1 | one |', 4, 0, NOT_A_TABLE), (2, '| 2 | two |', 4, 0, NOT_A_TABLE)]
        self.assertEqual(expected, broken_rows(orphans))

    def test_rows_after_text_or_after_a_table_with_a_blank_line_are_no_table_either(self):
        after_text = [(3, '| 1 | one |', 4, 0, NOT_A_TABLE), (4, '| 2 | two |', 4, 0, NOT_A_TABLE)]
        self.assertEqual(after_text, broken_rows('Text.\n\n| 1 | one |\n| 2 | two |\n'))
        glued = [(2, '| 1 | one |', 4, 0, NOT_A_TABLE), (3, '| 2 | two |', 4, 0, NOT_A_TABLE)]
        self.assertEqual(glued, broken_rows('Text.\n| 1 | one |\n| 2 | two |\n'))
        after_a_table = [(5, '| 3 | three |', 4, 0, NOT_A_TABLE), (6, '| 4 | four |', 4, 0, NOT_A_TABLE)]
        self.assertEqual(after_a_table, broken_rows(self.HEAD + self.ROW + '\n| 3 | three |\n| 4 | four |\n'))

    def test_a_header_may_follow_a_line_of_text_and_may_have_no_rows(self):
        self.assertEqual([], broken_rows('Text.\n' + self.HEAD + self.ROW))
        self.assertEqual([], broken_rows(self.HEAD))
        self.assertEqual([], broken_rows('| ID |\n|---|\n| 1 |\n'))

    def test_the_delimiter_row_is_the_one_github_accepts(self):
        accepted = (
            '|---|---|', '| --- | :---: |', '|:-|-:|', '--- | ---', '|  ---  |  :---:  |', '|---|---|\t', '   |---|---|',
        )
        for delimiter in accepted:
            self.assertEqual([], broken_rows('| a | b |\n' + delimiter + '\n| 1 | 2 |\n'), delimiter)
        refused = ('|---|', '|---|---|---|', '|- -|---|', '|:|:|', '---', '', '    |---|---|', '| 1 | 2 |', '\t|---|---|')
        for delimiter in refused:
            found = broken_rows('| a | b |\n' + delimiter + '\n| 1 | 2 |\n')
            self.assertEqual((1, '| a | b |', 4, 0, NOT_A_TABLE), found[0], repr(delimiter))
        # Under a header of one cell, a bare `---` has the right width and is still no delimiter: it makes a heading.
        self.assertEqual((1, '| ID |', 3, 0, NOT_A_TABLE), broken_rows('| ID |\n---\n| 1 |\n')[0])

    def test_rows_in_a_fence_or_in_indented_code_are_no_table_and_not_a_defect(self):
        self.assertEqual([], broken_rows('```\n| 1 | one |\n| 2 | two |\n```\n'))
        self.assertEqual([], broken_rows('    | 1 | one |\n    | 2 | two |\n'))


if __name__ == '__main__':
    unittest.main()
