// Built-in 5x7 bitmap font. Text must render identically on every machine, and system
// fonts differ per OS, so the runtime draws glyph cells itself: no font file is loaded and
// no installed font is ever consulted. The glyph table is this project's own work (MIT, see
// `LICENSE`); there is no third-party font to carry a notice for (`docs/features/accented-titles.md`).
//
// Lowercase letters are drawn as capitals. Latin letters with an accent (Portuguese and the
// other Western European ones in `crates/core/src/text_coverage.txt`) are the capital plus a
// diacritic mark composed above (acute, grave, circumflex, tilde, diaeresis) or below (cedilla)
// the 5x7 body. Text is normalized to NFC first, so a composed `ã` and `a` + U+0303 draw the
// same picture. Characters outside the coverage are drawn as an outlined box (the core refuses
// them before they get here).

const ROWS = {
  A: '01110 10001 10001 11111 10001 10001 10001',
  B: '11110 10001 10001 11110 10001 10001 11110',
  C: '01110 10001 10000 10000 10000 10001 01110',
  D: '11110 10001 10001 10001 10001 10001 11110',
  E: '11111 10000 10000 11110 10000 10000 11111',
  F: '11111 10000 10000 11110 10000 10000 10000',
  G: '01110 10001 10000 10111 10001 10001 01111',
  H: '10001 10001 10001 11111 10001 10001 10001',
  I: '01110 00100 00100 00100 00100 00100 01110',
  J: '00111 00010 00010 00010 00010 10010 01100',
  K: '10001 10010 10100 11000 10100 10010 10001',
  L: '10000 10000 10000 10000 10000 10000 11111',
  M: '10001 11011 10101 10101 10001 10001 10001',
  N: '10001 10001 11001 10101 10011 10001 10001',
  O: '01110 10001 10001 10001 10001 10001 01110',
  P: '11110 10001 10001 11110 10000 10000 10000',
  Q: '01110 10001 10001 10001 10101 10010 01101',
  R: '11110 10001 10001 11110 10100 10010 10001',
  S: '01111 10000 10000 01110 00001 00001 11110',
  T: '11111 00100 00100 00100 00100 00100 00100',
  U: '10001 10001 10001 10001 10001 10001 01110',
  V: '10001 10001 10001 10001 10001 01010 00100',
  W: '10001 10001 10001 10101 10101 10101 01010',
  X: '10001 10001 01010 00100 01010 10001 10001',
  Y: '10001 10001 01010 00100 00100 00100 00100',
  Z: '11111 00001 00010 00100 01000 10000 11111',
  0: '01110 10001 10011 10101 11001 10001 01110',
  1: '00100 01100 00100 00100 00100 00100 01110',
  2: '01110 10001 00001 00010 00100 01000 11111',
  3: '11110 00001 00001 01110 00001 00001 11110',
  4: '00010 00110 01010 10010 11111 00010 00010',
  5: '11111 10000 11110 00001 00001 10001 01110',
  6: '00110 01000 10000 11110 10001 10001 01110',
  7: '11111 00001 00010 00100 01000 01000 01000',
  8: '01110 10001 10001 01110 10001 10001 01110',
  9: '01110 10001 10001 01111 00001 00010 01100',
  ' ': '00000 00000 00000 00000 00000 00000 00000',
  '.': '00000 00000 00000 00000 00000 01100 01100',
  ',': '00000 00000 00000 00000 01100 00100 01000',
  '!': '00100 00100 00100 00100 00100 00000 00100',
  '?': '01110 10001 00001 00010 00100 00000 00100',
  '-': '00000 00000 00000 11111 00000 00000 00000',
  _: '00000 00000 00000 00000 00000 00000 11111',
  '+': '00000 00100 00100 11111 00100 00100 00000',
  '=': '00000 00000 11111 00000 11111 00000 00000',
  ':': '00000 01100 01100 00000 01100 01100 00000',
  ';': '00000 01100 01100 00000 01100 00100 01000',
  '/': '00001 00001 00010 00100 01000 10000 10000',
  '\\': '10000 10000 01000 00100 00010 00001 00001',
  "'": '00100 00100 01000 00000 00000 00000 00000',
  '"': '01010 01010 01010 00000 00000 00000 00000',
  '(': '00010 00100 01000 01000 01000 00100 00010',
  ')': '01000 00100 00010 00010 00010 00100 01000',
  '[': '01110 01000 01000 01000 01000 01000 01110',
  ']': '01110 00010 00010 00010 00010 00010 01110',
  '<': '00010 00100 01000 10000 01000 00100 00010',
  '>': '01000 00100 00010 00001 00010 00100 01000',
  '*': '00000 10101 01110 11111 01110 10101 00000',
  '#': '01010 01010 11111 01010 11111 01010 01010',
  '%': '11000 11001 00010 00100 01000 10011 00011',
  '&': '01100 10010 10100 01000 10101 10010 01101',
  '@': '01110 10001 10111 10101 10111 10000 01110',
  '$': '00100 01111 10100 01110 00101 11110 00100',
  '^': '00100 01010 10001 00000 00000 00000 00000',
  '~': '00000 00000 01000 10101 00010 00000 00000',
  '|': '00100 00100 00100 00100 00100 00100 00100',
  '{': '00110 00100 00100 01000 00100 00100 00110',
  '}': '01100 00100 00100 00010 00100 00100 01100',
  '`': '01000 00100 00010 00000 00000 00000 00000',
};
const UNKNOWN = '11111 10001 10001 10001 10001 10001 11111';

export const GLYPH_COLUMNS = 5;
export const GLYPH_ROWS = 7;
const ADVANCE = GLYPH_COLUMNS + 1;
/** Rows a diacritic mark takes above, and the cedilla below, the 7-row body. */
export const MARK_ROWS = 2;

// Punctuation outside ASCII. Typographic quotes and dashes share the straight glyphs: the cell is
// 5 wide, too narrow to tell them apart.
const EXTRA = {
  '¿': '00100 00000 00100 01000 10000 10001 01110',
  '¡': '00100 00000 00100 00100 00100 00100 00100',
  '«': '00000 00101 01010 10100 01010 00101 00000',
  '»': '00000 10100 01010 00101 01010 10100 00000',
  '‘': ROWS["'"],
  '’': ROWS["'"],
  '“': ROWS['"'],
  '”': ROWS['"'],
  '–': ROWS['-'],
  '—': ROWS['-'],
  '…': '00000 00000 00000 00000 00000 00000 10101',
  'º': '01100 10010 10010 01100 00000 00000 00000',
  ª: '01110 10010 01110 00000 00000 00000 00000',
  '°': '01100 10010 10010 01100 00000 00000 00000',
  '·': '00000 00000 00000 00100 00000 00000 00000',
};

// Combining marks: two rows above the body (`top`) or two below (`bottom`).
const MARKS = {
  '\u0300': { top: '01000 00100' },
  '\u0301': { top: '00010 00100' },
  '\u0302': { top: '00100 01010' },
  '\u0303': { top: '01010 10100' },
  '\u0308': { top: '00000 01010' },
  '\u0327': { bottom: '00100 01100' },
};

/** The text as the font sees it: NFC, so composed and decomposed spellings are one string. */
const COVERED = new Set('ÀÁÂÃÄÇÈÉÊËÌÍÎÏÑÒÓÔÕÖÙÚÛÜÝàáâãäçèéêëìíîïñòóôõöùúûüý¿¡«»‘’“”–—…ºª°·');
const COMBINING = new Set(['\u0300', '\u0301', '\u0302', '\u0303', '\u0308', '\u0327']);

export const normalizeText = (text) => String(text).normalize('NFC');

/**
 * One character as `{ body, top, bottom }` rows, or `null` for a character the font does not have.
 * A letter with a mark is the NFD decomposition: the base letter plus one supported mark.
 */
const glyphOf = (character) => {
  if (ROWS[character] ?? EXTRA[character]) return { body: ROWS[character] ?? EXTRA[character] };
  const upper = /^[a-z]$/.test(character) ? ROWS[character.toUpperCase()] : null;
  if (upper && character.toUpperCase().length === 1) return { body: upper };
  if (!COVERED.has(character)) return null;
  const [base, mark, ...rest] = [...character.normalize('NFD')];
  if (!mark || rest.length || !MARKS[mark]) return null;
  const letter = ROWS[base.toUpperCase()];
  if (!letter || !/^[A-Za-z]$/.test(base)) return null;
  return { body: letter, ...MARKS[mark] };
};

const characters = (text) => [...normalizeText(text)].map((character) => ({ character, glyph: glyphOf(character) }));

/** Whether the font has a glyph for this character (after NFC), false for the unknown box. */
export const hasGlyph = (character) => {
  if ([...String(character)].some((one) => !(/[ -~]/.test(one) || COVERED.has(one) || COMBINING.has(one)))) return false;
  const list = [...normalizeText(character)];
  return list.length > 0 && list.every((one) => glyphOf(one));
};

/**
 * Size in glyph cells (one blank column between glyphs). `pad` is the number of rows added above
 * AND below the 7-row body when any character carries a mark, so the body stays where plain text
 * puts it (the quad is centered) and `size` keeps meaning "body height".
 */
export const measureText = (text) => {
  const list = characters(text);
  const marked = list.some(({ glyph }) => glyph && (glyph.top || glyph.bottom));
  const pad = marked ? MARK_ROWS : 0;
  return {
    columns: Math.max(1, list.length * ADVANCE - 1),
    rows: GLYPH_ROWS + 2 * pad,
    pad,
  };
};

const lit = (bits, column) => bits[column] === '1';

/**
 * Calls `fillCell(column, row)` for every lit cell of `text`; the caller decides what a cell
 * is (a canvas rectangle in the runtime, a counter in tests). Rows count from the top of the
 * `measureText(text).rows` box, so the body starts at `pad`.
 */
export const forEachLitCell = (text, fillCell) => {
  const { pad } = measureText(text);
  characters(text).forEach(({ glyph }, position) => {
    const { body, top, bottom } = glyph ?? { body: UNKNOWN };
    const draw = (rows, offset) =>
      rows.split(' ').forEach((bits, row) => {
        for (let column = 0; column < GLYPH_COLUMNS; column += 1) {
          if (lit(bits, column)) fillCell(position * ADVANCE + column, offset + row);
        }
      });
    draw(body, pad);
    if (top) draw(top, pad - MARK_ROWS);
    if (bottom) draw(bottom, pad + GLYPH_ROWS);
  });
};
