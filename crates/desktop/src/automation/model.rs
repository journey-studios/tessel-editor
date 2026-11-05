//! Shared protocol pieces: error envelope, native window actions and window-size parsing.

use serde_json::{Value, json};

pub fn error(code: &str, message: &str) -> Value {
    json!({"ok":false,"version":1,"error":{"code":code,"message":message}})
}

pub fn native_actions() -> Value {
    json!([
        {"id":"window.close","action":"invoke","enabled":true},
        {"id":"window.minimize","action":"invoke","enabled":true},
        {"id":"window.maximize","action":"invoke","enabled":true},
        {"id":"window.fullscreen","action":"invoke","enabled":true},
        {"id":"window.activate","action":"invoke","enabled":true},
        {"id":"theme.dark","action":"invoke","enabled":true},
        {"id":"theme.light","action":"invoke","enabled":true}
    ])
}

/// Keep native resize requests finite, bounded, and above the supported layout minimum.
pub fn parse_window_size(raw: &str) -> Result<(f32, f32), String> {
    let invalid = || "Expected WIDTHxHEIGHT between 960x640 and 8192x8192".to_string();
    let (width, height) = raw.split_once('x').ok_or_else(invalid)?;
    let width: u16 = width.parse().map_err(|_| invalid())?;
    let height: u16 = height.parse().map_err(|_| invalid())?;
    if !(960..=8192).contains(&width) || !(640..=8192).contains(&height) {
        return Err(invalid());
    }
    Ok((f32::from(width), f32::from(height)))
}

#[cfg(test)]
mod resize_tests {
    use super::parse_window_size;
    #[test]
    fn accepts_minimum_and_wide_layouts() {
        assert_eq!(parse_window_size("960x640"), Ok((960.0, 640.0)));
        assert_eq!(parse_window_size("1440x900"), Ok((1440.0, 900.0)));
    }
    #[test]
    fn rejects_invalid_and_unsupported_sizes() {
        for raw in [
            "NaNx640",
            "-1x640",
            "960x639",
            "959x640",
            "10000x900",
            "960.5x640",
            "960x640x2",
        ] {
            assert!(parse_window_size(raw).is_err(), "{raw}");
        }
    }
}
