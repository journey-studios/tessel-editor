import assert from 'node:assert/strict';
import { test } from 'node:test';

import { lineStarts, shownCounts } from '../src/text-reveal.js';

// The same fixtures as `text::line_starts` in the core (crates/core/src/text.rs): a count of
// typed characters must name the same letters on both sides.
const FIXTURES = [
  ['um dois três quatro', ['um dois', 'três', 'quatro'], [0, 8, 13]],
  ['ab\ncd', ['ab', 'cd'], [0, 3]],
  ['aa aa aa', ['aa aa', 'aa'], [0, 6]],
  ['a\n\nb', ['a', '', 'b'], [0, 1, 3]],
  ['ação é já', ['ação é', 'já'], [0, 7]],
];

test('line starts match the core fixtures', () => {
  for (const [content, lines, expected] of FIXTURES) {
    assert.deepEqual(lineStarts(content, lines), expected, content);
  }
});

test('typed characters map to the leading letters of each line', () => {
  const content = 'um dois três quatro';
  const lines = ['um dois', 'três', 'quatro'];
  assert.deepEqual(shownCounts(content, lines, 0), [0, 0, 0]);
  assert.deepEqual(shownCounts(content, lines, 3), [3, 0, 0]);
  // The space the layout broke at is not on any line: 7 is "um dois" whole, 8 is still it.
  assert.deepEqual(shownCounts(content, lines, 8), [7, 0, 0]);
  assert.deepEqual(shownCounts(content, lines, 10), [7, 2, 0]);
  assert.deepEqual(shownCounts(content, lines, 14), [7, 4, 1]);
  assert.deepEqual(shownCounts(content, lines, 999), [7, 4, 6]);
});

test('a text without a visible count draws every line whole', () => {
  assert.deepEqual(shownCounts('ab\ncd', ['ab', 'cd'], undefined), [2, 2]);
  assert.deepEqual(shownCounts('ab', ['ab'], null), [2]);
});
