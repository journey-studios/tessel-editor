//! Structured command failures shared by the gesture handlers and the CLI.

use serde_json::{Value, json};

/// A rejected command. `code` is a stable machine-readable identifier; the CLI
/// forwards it unchanged and exits nonzero, while gestures show `message`.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
    pub target: Option<String>,
    /// Boxed so `Result<_, CommandError>` stays small on every call path.
    pub details: Option<Box<Value>>,
}

impl CommandError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            target: None,
            details: None,
        }
    }

    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn details(mut self, details: Value) -> Self {
        self.details = Some(Box::new(details));
        self
    }

    pub fn unknown_element(kind: &str, id: &str) -> Self {
        Self::new("unknown_element", format!("No {kind} with ID `{id}`")).target(id)
    }

    pub fn invalid_value(message: impl Into<String>, expected: Value) -> Self {
        Self::new("invalid_value", message).details(json!({ "expected": expected }))
    }

    pub fn disabled(message: impl Into<String>) -> Self {
        Self::new("disabled_element", message)
    }

    pub fn to_json(&self) -> Value {
        let mut error = json!({ "code": self.code, "message": self.message });
        if let Some(target) = &self.target {
            error["target"] = json!(target);
        }
        if let Some(details) = &self.details {
            error["details"] = (**details).clone();
        }
        json!({ "ok": false, "version": 1, "error": error })
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// A value as it arrives from a gesture or the CLI, before the target component
/// decides its type. Strings are what the CLI always sends; typed JSON clients
/// may send numbers and booleans.
#[derive(Debug, Clone, PartialEq)]
pub enum Raw {
    Text(String),
    Number(f64),
    Bool(bool),
}

impl Raw {
    pub fn from_json(value: &Value) -> Option<Self> {
        match value {
            Value::String(text) => Some(Self::Text(text.clone())),
            Value::Number(number) => number.as_f64().map(Self::Number),
            Value::Bool(flag) => Some(Self::Bool(*flag)),
            _ => None,
        }
    }

    /// A finite number. `NaN`/`inf` are rejected here so no component stores them.
    pub fn number(&self) -> Result<f64, CommandError> {
        let parsed = match self {
            Self::Number(number) => Some(*number),
            Self::Text(text) => text.trim().parse::<f64>().ok(),
            Self::Bool(_) => None,
        };
        parsed
            .filter(|number| number.is_finite())
            .ok_or_else(|| CommandError::invalid_value("Expected a finite number", json!("number")))
    }

    pub fn integer(&self) -> Result<i64, CommandError> {
        let number = self.number()?;
        if number.fract() != 0.0 || number.abs() > 1.0e12 {
            return Err(CommandError::invalid_value(
                "Expected a whole number",
                json!("integer"),
            ));
        }
        Ok(number as i64)
    }

    pub fn boolean(&self) -> Result<bool, CommandError> {
        match self {
            Self::Bool(flag) => Ok(*flag),
            Self::Text(text) => match text.trim().to_ascii_lowercase().as_str() {
                "true" | "on" | "yes" | "1" => Ok(true),
                "false" | "off" | "no" | "0" => Ok(false),
                _ => Err(CommandError::invalid_value(
                    "Expected true or false",
                    json!("boolean"),
                )),
            },
            Self::Number(_) => Err(CommandError::invalid_value(
                "Expected true or false",
                json!("boolean"),
            )),
        }
    }

    pub fn text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Number(number) => number.to_string(),
            Self::Bool(flag) => flag.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_reject_non_finite_and_text() {
        assert_eq!(Raw::Text("110".into()).number().unwrap(), 110.0);
        for bad in ["abc", "NaN", "inf", ""] {
            assert!(Raw::Text(bad.into()).number().is_err(), "{bad}");
        }
        assert!(Raw::Bool(true).number().is_err());
    }

    #[test]
    fn integers_must_be_whole() {
        assert_eq!(Raw::Text("72".into()).integer().unwrap(), 72);
        assert!(Raw::Number(1.5).integer().is_err());
    }

    #[test]
    fn booleans_accept_common_spellings_only() {
        assert!(Raw::Text("true".into()).boolean().unwrap());
        assert!(!Raw::Text("off".into()).boolean().unwrap());
        assert!(Raw::Text("maybe".into()).boolean().is_err());
    }
}
