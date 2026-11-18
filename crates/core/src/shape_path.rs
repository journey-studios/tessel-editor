//! The `path` of a path shape (F068, BL169.6 step 1B): a strict subset of SVG path data.
//!
//! The core owns the grammar so that the runtime never receives a string it could draw two ways,
//! and so a typo is refused when it is typed, not when a frame is drawn. Only absolute commands
//! are accepted (`M L H V C Q Z`); coordinates live in a 100 × 100 box that is stretched over the
//! shape's width and height. The stored text is the canonical spelling (single spaces, one command
//! per segment, shortest numbers), so the same path always compares and saves identically.

use serde_json::json;

use crate::state::error::CommandError;

/// Most segments a path may have. A path is an icon or a halo outline, not traced artwork; the cap
/// keeps a plan and an inspector row small.
pub const MAX_SEGMENTS: usize = 128;
/// Coordinates may leave the 0..100 box (a curve that bulges, a line that bleeds) but not by much.
pub const COORD_MIN: f64 = -200.0;
pub const COORD_MAX: f64 = 300.0;
/// A closed triangle: what a new path shape draws until it is edited.
pub const DEFAULT_PATH: &str = "M 50 0 L 100 100 L 0 100 Z";

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token {
    Command(char),
    Number(f64),
}

fn refusal(message: impl Into<String>) -> CommandError {
    CommandError::invalid_value(
        message,
        json!({ "field": "path", "commands": "M L H V C Q Z (absolute)", "box": "0..100",
            "min": COORD_MIN, "max": COORD_MAX, "max_segments": MAX_SEGMENTS }),
    )
}

/// How many numbers one segment of `command` takes; `None` for a command outside the subset.
const fn arity(command: char) -> Option<usize> {
    match command {
        'M' | 'L' => Some(2),
        'H' | 'V' => Some(1),
        'C' => Some(6),
        'Q' => Some(4),
        'Z' => Some(0),
        _ => None,
    }
}

fn tokens(raw: &str) -> Result<Vec<Token>, CommandError> {
    let chars: Vec<char> = raw.chars().collect();
    let mut found = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let ch = chars[at];
        if ch.is_whitespace() || ch == ',' {
            at += 1;
            continue;
        }
        if ch.is_ascii_alphabetic() {
            found.push(Token::Command(ch));
            at += 1;
            continue;
        }
        let start = at;
        if ch == '+' || ch == '-' {
            at += 1;
        }
        let mut digits = 0;
        while at < chars.len() && chars[at].is_ascii_digit() {
            at += 1;
            digits += 1;
        }
        if at < chars.len() && chars[at] == '.' {
            at += 1;
            while at < chars.len() && chars[at].is_ascii_digit() {
                at += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return Err(refusal(format!("Unexpected `{ch}` in the path")));
        }
        // An exponent only counts when digits follow it; otherwise the `e` is a (refused) command.
        if at < chars.len() && (chars[at] == 'e' || chars[at] == 'E') {
            let mut ahead = at + 1;
            if ahead < chars.len() && (chars[ahead] == '+' || chars[ahead] == '-') {
                ahead += 1;
            }
            if ahead < chars.len() && chars[ahead].is_ascii_digit() {
                while ahead < chars.len() && chars[ahead].is_ascii_digit() {
                    ahead += 1;
                }
                at = ahead;
            }
        }
        let text: String = chars[start..at].iter().collect();
        let number = text
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .ok_or_else(|| refusal(format!("`{text}` is not a finite number")))?;
        found.push(Token::Number(number));
    }
    Ok(found)
}

fn spell(number: f64) -> String {
    // `-0` and `0` are one coordinate.
    (number + 0.0).to_string()
}

/// Parses `raw` and returns its canonical spelling, or why it is not a path this editor draws.
pub fn canonicalize(raw: &str) -> Result<String, CommandError> {
    let tokens = tokens(raw)?;
    let mut out: Vec<String> = Vec::new();
    let mut segments = 0;
    let mut at = 0;
    while at < tokens.len() {
        let Token::Command(written) = tokens[at] else {
            return Err(refusal("A number must follow a command"));
        };
        let command = written.to_ascii_uppercase();
        let Some(count) = arity(command) else {
            return Err(refusal(format!(
                "`{written}` is not a path command; use M L H V C Q Z"
            )));
        };
        if written != command {
            return Err(refusal(format!(
                "`{written}` is relative; write the absolute `{command}`"
            )));
        }
        if out.is_empty() && command != 'M' {
            return Err(refusal("A path starts with M"));
        }
        at += 1;
        if count == 0 {
            segments += 1;
            out.push("Z".to_string());
            if matches!(tokens.get(at), Some(Token::Number(_))) {
                return Err(refusal("Z takes no numbers"));
            }
            continue;
        }
        let mut groups = 0;
        while let Some(Token::Number(_)) = tokens.get(at) {
            let group = tokens
                .get(at..at + count)
                .filter(|group| group.iter().all(|token| matches!(token, Token::Number(_))))
                .ok_or_else(|| refusal(format!("`{command}` takes {count} numbers per segment")))?;
            // A pair after the first `M` is an implicit `L`, as in SVG.
            let name = if groups > 0 && command == 'M' {
                'L'
            } else {
                command
            };
            let mut spelled = vec![name.to_string()];
            for token in group {
                let Token::Number(number) = token else {
                    continue;
                };
                if !(COORD_MIN..=COORD_MAX).contains(number) {
                    return Err(refusal(format!(
                        "Coordinate {number} is outside {COORD_MIN}..{COORD_MAX}"
                    )));
                }
                spelled.push(spell(*number));
            }
            segments += 1;
            if segments > MAX_SEGMENTS {
                return Err(refusal(format!(
                    "A path has at most {MAX_SEGMENTS} segments"
                )));
            }
            out.push(spelled.join(" "));
            at += count;
            groups += 1;
        }
        if groups == 0 {
            return Err(refusal(format!(
                "`{command}` takes {count} numbers per segment"
            )));
        }
    }
    if out.is_empty() {
        return Err(refusal("A path needs at least one segment"));
    }
    Ok(out.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_written_in_one_canonical_spelling() {
        assert_eq!(
            canonicalize("M10,90L50 10 -30e0,-0 Z").unwrap(),
            "M 10 90 L 50 10 L -30 0 Z"
        );
        assert_eq!(canonicalize(DEFAULT_PATH).unwrap(), DEFAULT_PATH);
        assert_eq!(
            canonicalize("M 0 50 C 20 0 80 100 100 50").unwrap(),
            "M 0 50 C 20 0 80 100 100 50"
        );
        assert_eq!(
            canonicalize("M 0 0 H 50 V 20 Q 10 10 0 0").unwrap(),
            "M 0 0 H 50 V 20 Q 10 10 0 0"
        );
        // The same path in its canonical spelling stays as it is.
        let once = canonicalize("M0 0L10 10L20 0").unwrap();
        assert_eq!(canonicalize(&once).unwrap(), once);
    }

    #[test]
    fn what_is_not_in_the_subset_is_refused_with_the_reason() {
        for bad in [
            "",
            "   ",
            "L 10 10",
            "M 10",
            "M 10 10 L 20",
            "M 10 10 Z 5",
            "M 10 10 A 5 5 0 0 1 20 20",
            "M 10 10 l 5 5",
            "M 10 abc",
            "M 10 10 e",
            "M 1000 10",
            "M 10 10 C 1 2 3 4 5",
            "10 10",
        ] {
            let error = canonicalize(bad).unwrap_err();
            assert_eq!(error.code, "invalid_value", "{bad:?}");
        }
        let long = format!("M 0 0 {}", "L 1 1 ".repeat(MAX_SEGMENTS));
        assert!(canonicalize(&long).is_err(), "too many segments");
        let limit = format!("M 0 0 {}", "L 1 1 ".repeat(MAX_SEGMENTS - 1));
        assert!(canonicalize(&limit).is_ok(), "the limit itself is allowed");
    }

    #[test]
    fn repeated_pairs_after_a_move_are_lines() {
        assert_eq!(
            canonicalize("M 0 0 10 10 20 0").unwrap(),
            "M 0 0 L 10 10 L 20 0"
        );
    }
}
