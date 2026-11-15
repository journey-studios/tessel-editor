//! The characters a title can hold and their canonical (NFC) spelling.
//!
//! The runtime draws text with its own 5x7 bitmap font (`runtime/src/font.js`): no font file, no
//! system font. That font covers printable ASCII plus the Latin letters and punctuation listed in
//! `text_coverage.txt` (the Portuguese set and the other Western European accents). This module is
//! the core's side of the contract: it composes `base + combining mark` into the precomposed letter
//! (what NFC does for this repertoire), so `ã` and `a` + U+0303 are one string everywhere (plan,
//! document, undo, export), and it refuses what the font cannot draw instead of letting the runtime
//! draw an outlined box.

use serde_json::json;

use crate::state::error::CommandError;

/// Non-ASCII characters the font draws, one source for the core and the runtime tests.
pub const COVERAGE: &str = include_str!("text_coverage.txt");

/// `(precomposed, base, combining mark)` for every accented letter in [`COVERAGE`].
const COMPOSED: &[(char, char, char)] = &[
    ('À', 'A', '\u{0300}'),
    ('Á', 'A', '\u{0301}'),
    ('Â', 'A', '\u{0302}'),
    ('Ã', 'A', '\u{0303}'),
    ('Ä', 'A', '\u{0308}'),
    ('Ç', 'C', '\u{0327}'),
    ('È', 'E', '\u{0300}'),
    ('É', 'E', '\u{0301}'),
    ('Ê', 'E', '\u{0302}'),
    ('Ë', 'E', '\u{0308}'),
    ('Ì', 'I', '\u{0300}'),
    ('Í', 'I', '\u{0301}'),
    ('Î', 'I', '\u{0302}'),
    ('Ï', 'I', '\u{0308}'),
    ('Ñ', 'N', '\u{0303}'),
    ('Ò', 'O', '\u{0300}'),
    ('Ó', 'O', '\u{0301}'),
    ('Ô', 'O', '\u{0302}'),
    ('Õ', 'O', '\u{0303}'),
    ('Ö', 'O', '\u{0308}'),
    ('Ù', 'U', '\u{0300}'),
    ('Ú', 'U', '\u{0301}'),
    ('Û', 'U', '\u{0302}'),
    ('Ü', 'U', '\u{0308}'),
    ('Ý', 'Y', '\u{0301}'),
    ('à', 'a', '\u{0300}'),
    ('á', 'a', '\u{0301}'),
    ('â', 'a', '\u{0302}'),
    ('ã', 'a', '\u{0303}'),
    ('ä', 'a', '\u{0308}'),
    ('ç', 'c', '\u{0327}'),
    ('è', 'e', '\u{0300}'),
    ('é', 'e', '\u{0301}'),
    ('ê', 'e', '\u{0302}'),
    ('ë', 'e', '\u{0308}'),
    ('ì', 'i', '\u{0300}'),
    ('í', 'i', '\u{0301}'),
    ('î', 'i', '\u{0302}'),
    ('ï', 'i', '\u{0308}'),
    ('ñ', 'n', '\u{0303}'),
    ('ò', 'o', '\u{0300}'),
    ('ó', 'o', '\u{0301}'),
    ('ô', 'o', '\u{0302}'),
    ('õ', 'o', '\u{0303}'),
    ('ö', 'o', '\u{0308}'),
    ('ù', 'u', '\u{0300}'),
    ('ú', 'u', '\u{0301}'),
    ('û', 'u', '\u{0302}'),
    ('ü', 'u', '\u{0308}'),
    ('ý', 'y', '\u{0301}'),
];

fn is_covered(ch: char) -> bool {
    if ch == '\n' {
        // An explicit line break (BL141.1); every other control character is refused.
        return true;
    }
    if ch.is_ascii() {
        return !ch.is_ascii_control();
    }
    COVERAGE.chars().any(|covered| covered == ch)
}

fn is_mark(ch: char) -> bool {
    COMPOSED.iter().any(|(_, _, mark)| *mark == ch)
}

fn unsupported(sequence: &str) -> CommandError {
    let described: Vec<String> = sequence
        .chars()
        .map(|ch| format!("U+{:04X}", ch as u32))
        .collect();
    CommandError::new(
        "unsupported_text",
        format!(
            "`{sequence}` ({}) is outside the built-in font (printable ASCII, Portuguese and Western European accents)",
            described.join(" ")
        ),
    )
    .details(json!({ "text": sequence, "code_points": described }))
}

/// `content` in NFC for the supported repertoire, or `unsupported_text` naming the first character
/// (or base + mark sequence) the font cannot draw. `\n` is an explicit line break; every other
/// control character (`\r` and `\t` included) is refused.
pub fn normalize(content: &str) -> Result<String, CommandError> {
    let mut out = String::with_capacity(content.len());
    for ch in content.chars() {
        if is_mark(ch) {
            let Some(base) = out.pop() else {
                return Err(unsupported(&ch.to_string()));
            };
            match COMPOSED
                .iter()
                .find(|(_, b, mark)| *b == base && *mark == ch)
            {
                Some((composed, _, _)) => out.push(*composed),
                None => return Err(unsupported(&format!("{base}{ch}"))),
            }
        } else if is_covered(ch) {
            out.push(ch);
        } else {
            return Err(unsupported(&ch.to_string()));
        }
    }
    Ok(out)
}

/// Columns of one glyph plus the blank column after it (`ADVANCE` in `runtime/src/font.js`): a
/// line of `n` characters is `6n - 1` glyph columns wide. The font is monospaced.
pub const GLYPH_ADVANCE: usize = 6;
/// Rows of a glyph body (`GLYPH_ROWS`); a title's `size` is the height of this body in percent of
/// the canvas height, so one glyph cell is `size / 100 * height / 7`.
pub const GLYPH_ROWS: usize = 7;
/// Rows an accent takes above, or a cedilla below, the body (`MARK_ROWS`).
pub const MARK_ROWS: usize = 2;
/// Distance from the top of one line's body to the next one's, in glyph rows
/// (`LINE_PITCH_ROWS` in the runtime): room for a mark above and below and one blank row, whether
/// or not the title has accents, so the spacing never changes when one is typed.
pub const LINE_PITCH_ROWS: usize = GLYPH_ROWS + 2 * MARK_ROWS + 1;
/// Share of the canvas width and height a title may fill (the title-safe area).
pub const SAFE_AREA: f64 = 0.9;

/// How a title sits in the frame (BL141.1): its `lines` (centred, top to bottom) and the
/// `scale` its `size` is drawn at (`1`, or less when it had to shrink to fit).
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayout {
    pub lines: Vec<String>,
    pub scale: f64,
}

impl TextLayout {
    /// Whether this is the plain one-line layout every title had before BL141.1: the plan then
    /// leaves `lines` and `scale` out, so the picture and the plan stay byte for byte as they were.
    pub fn is_single_line_of(&self, content: &str) -> bool {
        self.scale == 1.0 && self.lines.len() == 1 && self.lines[0] == content
    }
}

/// Whether a character carries a mark the font draws above or below the body.
fn is_marked(ch: char) -> bool {
    COMPOSED.iter().any(|(composed, _, _)| *composed == ch)
}

/// Lays a title out in a `width x height` canvas at `size` (% of the height, see [`GLYPH_ROWS`]):
///
/// * `\n` always starts a new line;
/// * a line wider than [`SAFE_AREA`] of the width breaks at the last space that fits (the space is
///   dropped), greedily; a word is never cut;
/// * when one word alone is wider than that, or the lines are taller than [`SAFE_AREA`] of the
///   height, the whole title shrinks (`scale < 1`) until it fits, and is wrapped again at that size.
///
/// The text is never clipped by the frame and never grows. Lines are centred by the renderer.
pub fn layout(content: &str, size: f64, width: u32, height: u32) -> TextLayout {
    let (width, height) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let cell = |scale: f64| size / 100.0 * height / GLYPH_ROWS as f64 * scale;
    let columns = |chars: usize| (chars * GLYPH_ADVANCE).saturating_sub(1) as f64;
    let paragraphs: Vec<Vec<&str>> = content
        .split('\n')
        .map(|line| line.split(' ').collect())
        .collect();
    let longest = paragraphs
        .iter()
        .flatten()
        .map(|word| word.chars().count())
        .max()
        .unwrap_or(0);
    let max_width = SAFE_AREA * width;
    let max_height = SAFE_AREA * height;
    let mut scale = if longest == 0 || columns(longest) * cell(1.0) <= max_width {
        1.0
    } else {
        max_width / (columns(longest) * cell(1.0))
    };
    let marked = content.chars().any(is_marked);
    let body_rows = (GLYPH_ROWS + if marked { 2 * MARK_ROWS } else { 0 }) as f64;
    let mut lines = Vec::new();
    // Shrinking only ever joins lines, so the height falls with the scale: a few rounds settle it.
    for _ in 0..16 {
        // How many characters a line holds at this scale; the tiny slack keeps the longest word,
        // which defined the scale, from losing its last character to rounding.
        let fit = ((max_width / cell(scale) + 1.0) / GLYPH_ADVANCE as f64 + 1e-9).floor() as usize;
        lines = wrap(&paragraphs, fit.max(1));
        let rows = (lines.len() - 1) as f64 * LINE_PITCH_ROWS as f64 + body_rows;
        let block = rows * cell(scale);
        if block <= max_height * (1.0 + 1e-12) {
            break;
        }
        scale *= max_height / block;
    }
    TextLayout { lines, scale }
}

/// Where a title of several lines sits vertically when its clip is moved by `position_y`
/// (decision of 2026-11-14 for BL141.1): how much to add to the layer's `y` (percent of the
/// canvas height, positive down).
///
/// One line is drawn where the position puts it, as always. The extra height of the other lines
/// grows into the room that single line leaves above and below it inside the safe area, in
/// proportion to that room: a title in the lower third grows upwards, one near the top grows
/// downwards, a centred one both ways equally, and the change is continuous as the position moves
/// (an animated title never jumps). The block never goes past the edge the single line already
/// had on either side: when that line is inside the safe area, so is the block (as long as the
/// block fits it, which [`layout`] ensures at zoom 100 %); when the line was placed partly past it,
/// the block keeps that edge and grows the other way. A one-line title gets `0`: it is exactly
/// where it was.
/// `zoom` is the layer's scale (`1` is 100 %); rotation is not taken into account.
pub fn anchor_offset(
    layout: &TextLayout,
    content: &str,
    size: f64,
    height: u32,
    y: f64,
    zoom: f64,
) -> f64 {
    if layout.lines.len() <= 1 || !(y.is_finite() && zoom.is_finite() && zoom > 0.0) {
        return 0.0;
    }
    let height = f64::from(height.max(1));
    let cell = size / 100.0 * height / GLYPH_ROWS as f64 * layout.scale * zoom;
    let pad = if content.chars().any(is_marked) {
        2 * MARK_ROWS
    } else {
        0
    };
    let line = (GLYPH_ROWS + pad) as f64 * cell;
    let block = ((layout.lines.len() - 1) * LINE_PITCH_ROWS + GLYPH_ROWS + pad) as f64 * cell;
    let extra = block - line;
    let centre = y / 100.0 * height;
    let half_safe = SAFE_AREA * height / 2.0;
    let above = (centre - line / 2.0 + half_safe).max(0.0);
    let below = (half_safe - centre - line / 2.0).max(0.0);
    let grow_up = if above + below > 0.0 {
        extra * above / (above + below)
    } else {
        extra / 2.0
    };
    let grow_down = extra - grow_up;
    (grow_down - grow_up) / 2.0 / height * 100.0
}

/// Greedy wrap of each paragraph's words into lines of at most `fit` characters (a longer word,
/// which the scale already prevents, keeps a line of its own).
fn wrap(paragraphs: &[Vec<&str>], fit: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for words in paragraphs {
        let mut line = String::new();
        let mut count = 0;
        for (index, word) in words.iter().enumerate() {
            let length = word.chars().count();
            if index > 0 && count + 1 + length > fit {
                lines.push(std::mem::take(&mut line));
                count = 0;
            } else if index > 0 {
                line.push(' ');
                count += 1;
            }
            line.push_str(word);
            count += length;
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn composed_and_decomposed_spellings_are_one_string() {
        for (composed, base, mark) in COMPOSED {
            assert_eq!(
                normalize(&format!("{base}{mark}")).unwrap(),
                composed.to_string()
            );
            assert_eq!(
                normalize(&composed.to_string()).unwrap(),
                composed.to_string()
            );
        }
        let decomposed =
            "Corac\u{0327}a\u{0303}o e emoc\u{0327}a\u{0303}o, na\u{0303}o e\u{0301} so\u{0301}";
        assert_eq!(normalize(decomposed).unwrap(), "Coração e emoção, não é só");
    }

    #[test]
    fn the_whole_coverage_list_is_accepted_and_nothing_else_outside_ascii() {
        for ch in COVERAGE.trim().chars() {
            assert_eq!(normalize(&ch.to_string()).unwrap(), ch.to_string(), "{ch}");
        }
        for outside in ["€", "ß", "日", "😀", "ÿ", "Ā", "\u{200B}"] {
            assert_eq!(
                normalize(outside).unwrap_err().code,
                "unsupported_text",
                "{outside:?}"
            );
        }
    }

    #[test]
    fn marks_that_do_not_compose_or_dangle_are_refused_whole() {
        for bad in [
            "a\u{0307}",
            "\u{0301}",
            "x\u{0301}",
            "ã\u{0301}",
            "a\u{0303}\u{0301}",
            "\u{0301}a",
        ] {
            assert_eq!(
                normalize(bad).unwrap_err().code,
                "unsupported_text",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn control_characters_are_refused_but_a_line_break_is_kept() {
        for bad in ["a\tb", "\u{7f}", "\r", "a\r\nb"] {
            assert!(normalize(bad).is_err(), "{bad:?}");
        }
        assert_eq!(normalize("a\nb").unwrap(), "a\nb");
    }

    #[test]
    fn ascii_passes_through_unchanged() {
        let ascii: String = (32u8..127).map(char::from).collect();
        assert_eq!(normalize(&ascii).unwrap(), ascii);
    }

    /// 1920x1080 at size 10: a cell is 1080 * 0.1 / 7 = 15.43 px, so the safe width (1728 px)
    /// holds (1728 / 15.43 + 1) / 6 = 18 characters per line.
    #[test]
    fn a_title_that_fits_keeps_its_single_line_and_full_size() {
        let fits = layout("AÇÃO NA PRAIA", 10.0, 1920, 1080);
        assert_eq!(fits.lines, ["AÇÃO NA PRAIA"]);
        assert_eq!(fits.scale, 1.0);
        assert!(fits.is_single_line_of("AÇÃO NA PRAIA"));
    }

    #[test]
    fn a_long_title_breaks_at_spaces_inside_the_safe_width() {
        let long = layout("Ação, Câmera e Edição: São Paulo", 10.0, 1920, 1080);
        assert_eq!(long.lines, ["Ação, Câmera e", "Edição: São Paulo"]);
        assert_eq!(long.scale, 1.0);
        for line in &long.lines {
            assert!(line.chars().count() <= 18, "{line}");
            assert!(!line.starts_with(' ') && !line.ends_with(' '), "{line:?}");
        }
        // A vertical frame is narrower: the same title takes more lines at the same size.
        let vertical = layout("Ação, Câmera e Edição: São Paulo", 10.0, 1080, 1920);
        assert!(vertical.lines.len() > long.lines.len(), "{vertical:?}");
        assert_eq!(vertical.lines.join(" "), "Ação, Câmera e Edição: São Paulo");
    }

    #[test]
    fn an_explicit_break_always_starts_a_line_and_a_blank_line_is_kept() {
        let broken = layout("UM\nDOIS\n\nTRES", 10.0, 1920, 1080);
        assert_eq!(broken.lines, ["UM", "DOIS", "", "TRES"]);
        assert_eq!(broken.scale, 1.0);
        assert!(!broken.is_single_line_of("UM\nDOIS\n\nTRES"));
    }

    #[test]
    fn a_word_wider_than_the_frame_shrinks_the_title_instead_of_being_cut() {
        let word = "PARALELEPIPEDOPARALELEPIPEDO"; // 28 characters, more than 18
        let shrunk = layout(word, 10.0, 1920, 1080);
        assert_eq!(shrunk.lines, [word]);
        assert!(shrunk.scale < 1.0);
        let cell = 10.0 / 100.0 * 1080.0 / 7.0 * shrunk.scale;
        let drawn = (28.0 * 6.0 - 1.0) * cell;
        assert!(
            (drawn - 0.9 * 1920.0).abs() < 1e-6,
            "exactly the safe width: {drawn}"
        );
    }

    #[test]
    fn too_many_lines_shrink_the_title_until_it_fits_the_height() {
        let text = ["LINHA"; 12].join("\n");
        let fitted = layout(&text, 20.0, 1920, 1080);
        assert_eq!(fitted.lines.len(), 12, "explicit breaks stay");
        let cell = 20.0 / 100.0 * 1080.0 / 7.0 * fitted.scale;
        let block = (11.0 * LINE_PITCH_ROWS as f64 + GLYPH_ROWS as f64) * cell;
        assert!(
            fitted.scale < 1.0 && block <= 0.9 * 1080.0 + 1e-6,
            "{fitted:?} {block}"
        );
        // Shrinking lets wrapped words join again: still inside the safe area both ways.
        let words = layout(&["PALAVRA"; 30].join(" "), 30.0, 1920, 1080);
        let cell = 30.0 / 100.0 * 1080.0 / 7.0 * words.scale;
        let widest = words
            .lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap();
        assert!(((widest * 6 - 1) as f64) * cell <= 0.9 * 1920.0 + 1e-6);
        let block = ((words.lines.len() - 1) * LINE_PITCH_ROWS + GLYPH_ROWS) as f64 * cell;
        assert!(block <= 0.9 * 1080.0 + 1e-6, "{words:?}");
    }

    /// The block's top and bottom, in percent of the height from the centre (down positive), for a
    /// title at `y` after [`anchor_offset`].
    fn block_span(content: &str, size: f64, y: f64) -> (f64, f64) {
        let layout = layout(content, size, 320, 180);
        let shifted = y + anchor_offset(&layout, content, size, 180, y, 1.0);
        let cell = size / 100.0 * 180.0 / 7.0 * layout.scale;
        let pad = if content.chars().any(is_marked) { 4 } else { 0 };
        let rows = (layout.lines.len() - 1) * LINE_PITCH_ROWS + GLYPH_ROWS + pad;
        let half = rows as f64 * cell / 2.0 / 180.0 * 100.0;
        (shifted - half, shifted + half)
    }

    #[test]
    fn a_wrapped_title_grows_away_from_the_nearest_edge_and_stays_in_the_safe_area() {
        let long = "Coração: não é só açaí, ÀS ÔNIBUS Ü";
        assert!(layout(long, 14.0, 320, 180).lines.len() >= 3);
        // A single line of this title (accents: 11 rows of 3.6 px) is 22 % of the height tall.
        let line = 11.0 * 14.0 / 100.0 * 180.0 / 7.0 / 180.0 * 100.0;
        // Lower third, the single line inside the safe area (bottom at 30 + 11 = 41 %): the block
        // grows upwards and stays inside the safe area (45 % each way).
        let (top, bottom) = block_span(long, 14.0, 30.0);
        assert!(
            bottom <= 45.0 + 1e-9 && top >= -45.0 - 1e-9,
            "{top} {bottom}"
        );
        assert!(
            anchor_offset(&layout(long, 14.0, 320, 180), long, 14.0, 180, 30.0, 1.0) < 0.0,
            "moved up"
        );
        // Lower still (y = 38): the single line already ends 49 % below the centre, past the safe
        // area but in the frame. The block keeps that bottom edge and grows upwards only.
        let (top, bottom) = block_span(long, 14.0, 38.0);
        assert!((bottom - (38.0 + line / 2.0)).abs() < 1e-9, "{bottom}");
        assert!(bottom <= 50.0 && top >= -45.0 - 1e-9, "{top} {bottom}");
        // Near the top: the mirror image.
        let (top, bottom) = block_span(long, 14.0, -30.0);
        assert!(
            bottom <= 45.0 + 1e-9 && top >= -45.0 - 1e-9,
            "{top} {bottom}"
        );
        assert!(anchor_offset(&layout(long, 14.0, 320, 180), long, 14.0, 180, -30.0, 1.0) > 0.0);
        // Centred: both ways equally, nothing moves.
        assert_eq!(
            anchor_offset(&layout(long, 14.0, 320, 180), long, 14.0, 180, 0.0, 1.0),
            0.0
        );
    }

    #[test]
    fn the_anchor_moves_continuously_and_a_single_line_never_moves() {
        let long = "Coração: não é só açaí, ÀS ÔNIBUS Ü";
        let wrapped = layout(long, 14.0, 320, 180);
        let mut previous = anchor_offset(&wrapped, long, 14.0, 180, -50.0, 1.0);
        for step in 1..=200 {
            let y = -50.0 + f64::from(step) * 0.5;
            let offset = anchor_offset(&wrapped, long, 14.0, 180, y, 1.0);
            assert!(
                (offset - previous).abs() < 1.0,
                "a jump at y = {y}: {previous} -> {offset}"
            );
            previous = offset;
        }
        let short = layout("AÇÃO", 14.0, 320, 180);
        for y in [-50.0, -38.0, 0.0, 12.5, 38.0, 50.0] {
            assert_eq!(anchor_offset(&short, "AÇÃO", 14.0, 180, y, 1.0), 0.0, "{y}");
        }
    }
}
