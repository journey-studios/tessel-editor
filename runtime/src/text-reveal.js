/**
 * Progressive text (F070): which letters of each line are drawn when the plan says
 * `text.visible` characters of the content are shown.
 *
 * The core breaks a revealed text into lines (`text::lines_of`) and counts the visible
 * characters from the start of the content. A layout drops the space it breaks at, so a line is
 * found in the content by walking forward from the end of the previous one. This is the same
 * walk as `text::line_starts` in the core (same fixtures in both tests), which is what makes a
 * count mean the same letters on both sides. Characters are code points, like Rust's `char`.
 */
export const lineStarts = (content, lines) => {
  const characters = [...content];
  let cursor = 0;
  return lines.map((line) => {
    const wanted = [...line];
    const fits = (at) => at + wanted.length <= characters.length;
    const matches = (at) => wanted.every((character, offset) => characters[at + offset] === character);
    let at = cursor;
    while (fits(at) && !matches(at)) at += 1;
    const start = fits(at) ? at : cursor;
    cursor = start + wanted.length;
    return start;
  });
};

/**
 * How many leading characters of each line to draw. `visible` absent (or not a number) draws
 * every line whole, which is how every text drew before F070.
 */
export const shownCounts = (content, lines, visible) => {
  if (!Number.isFinite(visible)) return lines.map((line) => [...line].length);
  const starts = lineStarts(content, lines);
  return lines.map((line, index) => Math.max(0, Math.min([...line].length, visible - starts[index])));
};
