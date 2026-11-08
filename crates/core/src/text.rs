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
/// (or base + mark sequence) the font cannot draw. Control characters (including newlines) are refused:
/// a title is one line.
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
    fn control_characters_and_newlines_are_refused() {
        for bad in ["a\nb", "a\tb", "\u{7f}", "\r"] {
            assert!(normalize(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn ascii_passes_through_unchanged() {
        let ascii: String = (32u8..127).map(char::from).collect();
        assert_eq!(normalize(&ascii).unwrap(), ascii);
    }
}
