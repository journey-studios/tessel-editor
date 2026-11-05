//! Reading a JSON tree into typed values while reporting every problem, not just the first.
//!
//! WHY a reader and not serde derive: the origin's zod parse reports all issues, the equivalence
//! oracle compares verdicts and first messages, and a caller needs the JSON Pointer of the bad
//! value. serde errors stop at the first problem and carry no pointer. The reader also records
//! which keys of each object were looked at, so every key left over is an unknown field, reported
//! as a notice while the raw tree keeps it.

use std::cell::RefCell;
use std::collections::HashSet;

use serde_json::{Map, Number, Value};

use super::issue::{
    INVALID_TYPE, INVALID_VALUE, Issue, LIMIT_EXCEEDED, MISSING_FIELD, Notice, UNKNOWN_FIELD,
};
use super::transform::{Quat, Vec3};

/// Largest integer a JavaScript number holds exactly. The origin parses with `JSON.parse`, so a
/// larger "integer" was already rounded there; refusing it here keeps verdicts comparable.
pub(super) const MAX_SAFE_INT: i64 = (1 << 53) - 1;

pub(super) struct Sink {
    issues: RefCell<Vec<Issue>>,
    notices: RefCell<Vec<Notice>>,
    pub(super) max_items: usize,
}

impl Sink {
    pub(super) fn new(max_items: usize) -> Self {
        Self {
            issues: RefCell::new(Vec::new()),
            notices: RefCell::new(Vec::new()),
            max_items,
        }
    }

    pub(super) fn issue(&self, code: &'static str, path: &str, message: impl Into<String>) {
        self.issues
            .borrow_mut()
            .push(Issue::new(code, path, message));
    }

    pub(super) fn into_parts(self) -> (Vec<Issue>, Vec<Notice>) {
        let mut notices = self.notices.into_inner();
        // Children drop before their parents; sorting makes the order independent of that.
        notices.sort_by(|a, b| a.path.cmp(&b.path));
        (self.issues.into_inner(), notices)
    }
}

/// RFC 6901 pointer segment: `~` and `/` are escaped.
pub(super) fn child(path: &str, key: &str) -> String {
    format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"))
}

pub(super) fn at(path: &str, index: usize) -> String {
    format!("{path}/{index}")
}

pub(super) fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// An integer-valued JSON number (`120` and `120.0` both count, as in JavaScript).
fn as_integer(number: &Number) -> Option<i64> {
    if let Some(value) = number.as_i64() {
        return Some(value);
    }
    let float = number.as_f64()?;
    // `|float| <= 2^53` is exact in f64; anything larger is rejected by the caller's range check.
    (float.is_finite() && float.fract() == 0.0 && float.abs() <= 9.007_199_254_740_992e15)
        .then_some(float as i64)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Bound {
    Any,
    NonNegative,
    Positive,
}

impl Bound {
    fn accepts(self, value: f64) -> bool {
        match self {
            Self::Any => true,
            Self::NonNegative => value >= 0.0,
            Self::Positive => value > 0.0,
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Any => "",
            Self::NonNegative => " that is not negative",
            Self::Positive => " greater than zero",
        }
    }
}

pub(super) struct Obj<'a> {
    map: &'a Map<String, Value>,
    path: String,
    sink: &'a Sink,
    seen: RefCell<HashSet<&'a str>>,
}

impl<'a> Obj<'a> {
    /// `value` must be an object; otherwise an `INVALID_TYPE` issue is recorded.
    pub(super) fn open(value: &'a Value, path: String, sink: &'a Sink) -> Option<Self> {
        let Value::Object(map) = value else {
            sink.issue(
                INVALID_TYPE,
                &path,
                format!("expected object, got {}", kind_of(value)),
            );
            return None;
        };
        Some(Self {
            map,
            path,
            sink,
            seen: RefCell::new(HashSet::new()),
        })
    }

    pub(super) fn path(&self) -> &str {
        &self.path
    }

    pub(super) fn field_path(&self, key: &str) -> String {
        child(&self.path, key)
    }

    /// Records a rule violation on this object's field `key`.
    pub(super) fn sink_issue(&self, code: &'static str, key: &str, message: impl Into<String>) {
        self.sink.issue(code, &self.field_path(key), message);
    }

    /// Records a rule violation at an explicit pointer.
    pub(super) fn sink_issue_at(&self, code: &'static str, path: &str, message: impl Into<String>) {
        self.sink.issue(code, path, message);
    }

    fn take(&self, key: &str) -> Option<&'a Value> {
        let (stored, value) = self.map.get_key_value(key)?;
        self.seen.borrow_mut().insert(stored.as_str());
        Some(value)
    }

    fn required(&self, key: &str) -> Option<&'a Value> {
        let found = self.take(key);
        if found.is_none() {
            self.sink.issue(
                MISSING_FIELD,
                &self.path,
                format!("missing required field `{key}`"),
            );
        }
        found
    }

    fn mismatch(&self, key: &str, expected: &str, value: &Value) {
        self.sink.issue(
            INVALID_TYPE,
            &self.field_path(key),
            format!("expected {expected}, got {}", kind_of(value)),
        );
    }

    fn invalid(&self, key: &str, message: impl Into<String>) {
        self.sink
            .issue(INVALID_VALUE, &self.field_path(key), message);
    }

    // ---- strings -------------------------------------------------------------------------

    fn read_string(&self, key: &str, value: &'a Value, non_empty: bool) -> Option<&'a str> {
        let Value::String(text) = value else {
            self.mismatch(key, "string", value);
            return None;
        };
        if non_empty && text.is_empty() {
            self.invalid(key, "must not be an empty string");
            return None;
        }
        Some(text)
    }

    /// A required non-empty string (every id and path of the format).
    pub(super) fn id(&self, key: &str) -> Option<&'a str> {
        self.read_string(key, self.required(key)?, true)
    }

    /// A required string that may be empty (the origin does not constrain it).
    pub(super) fn text(&self, key: &str) -> Option<&'a str> {
        self.read_string(key, self.required(key)?, false)
    }

    pub(super) fn opt_text(&self, key: &str) -> Option<Option<&'a str>> {
        let Some(value) = self.take(key) else {
            return Some(None);
        };
        self.read_string(key, value, false).map(Some)
    }

    /// `string | null`, required (`z.string().nullable()`): `Some(None)` is an explicit `null`.
    pub(super) fn nullable_text(&self, key: &str) -> Option<Option<&'a str>> {
        let value = self.required(key)?;
        if value.is_null() {
            return Some(None);
        }
        self.read_string(key, value, false).map(Some)
    }

    /// A required string that must be one of `allowed`; returns the matching entry.
    pub(super) fn one_of(&self, key: &str, allowed: &[&'static str]) -> Option<&'static str> {
        let text = self.read_string(key, self.required(key)?, false)?;
        let found = allowed.iter().copied().find(|candidate| *candidate == text);
        if found.is_none() {
            self.invalid(
                key,
                format!("`{text}` is not one of: {}", allowed.join(", ")),
            );
        }
        found
    }

    pub(super) fn opt_one_of(
        &self,
        key: &str,
        allowed: &[&'static str],
    ) -> Option<Option<&'static str>> {
        if !self.map.contains_key(key) {
            return Some(None);
        }
        self.one_of(key, allowed).map(Some)
    }

    /// A required string that must equal `literal` (`z.literal`).
    pub(super) fn literal(&self, key: &str, literal: &str) -> Option<()> {
        let text = self.read_string(key, self.required(key)?, false)?;
        if text == literal {
            return Some(());
        }
        self.invalid(key, format!("expected `{literal}`, got `{text}`"));
        None
    }

    // ---- numbers -------------------------------------------------------------------------

    fn read_integer(&self, key: &str, value: &Value, bound: Bound) -> Option<i64> {
        let Value::Number(number) = value else {
            self.mismatch(key, "integer", value);
            return None;
        };
        let Some(integer) = as_integer(number) else {
            self.invalid(key, format!("expected a whole number, got {number}"));
            return None;
        };
        if !(-MAX_SAFE_INT..=MAX_SAFE_INT).contains(&integer) {
            self.invalid(
                key,
                format!("{integer} is outside the exactly representable range (+-2^53 - 1)"),
            );
            return None;
        }
        if !bound.accepts(integer as f64) {
            self.invalid(
                key,
                format!("expected a whole number{}, got {integer}", bound.describe()),
            );
            return None;
        }
        Some(integer)
    }

    pub(super) fn int(&self, key: &str, bound: Bound) -> Option<i64> {
        self.read_integer(key, self.required(key)?, bound)
    }

    pub(super) fn opt_int(&self, key: &str, bound: Bound) -> Option<Option<i64>> {
        let Some(value) = self.take(key) else {
            return Some(None);
        };
        self.read_integer(key, value, bound).map(Some)
    }

    /// A required non-negative or positive integer as `u64`.
    pub(super) fn count(&self, key: &str, bound: Bound) -> Option<u64> {
        self.int(key, bound).and_then(|v| u64::try_from(v).ok())
    }

    fn read_number(&self, key: &str, value: &Value, bound: Bound) -> Option<f64> {
        let Value::Number(number) = value else {
            self.mismatch(key, "number", value);
            return None;
        };
        // serde_json cannot hold NaN or infinity, so a Number is always finite here; the check
        // stays because `import_value` accepts a tree built by the caller.
        let Some(float) = number.as_f64().filter(|float| float.is_finite()) else {
            self.invalid(key, "expected a finite number");
            return None;
        };
        if !bound.accepts(float) {
            self.invalid(
                key,
                format!("expected a number{}, got {number}", bound.describe()),
            );
            return None;
        }
        Some(float)
    }

    pub(super) fn num(&self, key: &str, bound: Bound) -> Option<f64> {
        self.read_number(key, self.required(key)?, bound)
    }

    pub(super) fn opt_num(&self, key: &str, bound: Bound) -> Option<Option<f64>> {
        let Some(value) = self.take(key) else {
            return Some(None);
        };
        self.read_number(key, value, bound).map(Some)
    }

    // ---- booleans, vectors ---------------------------------------------------------------

    /// `z.literal(true).optional()`: absent, or exactly `true`.
    pub(super) fn opt_true(&self, key: &str) -> Option<bool> {
        let Some(value) = self.take(key) else {
            return Some(false);
        };
        match value {
            Value::Bool(true) => Some(true),
            Value::Bool(false) => {
                self.invalid(key, "expected `true` (omit the field instead of `false`)");
                None
            }
            other => {
                self.mismatch(key, "boolean", other);
                None
            }
        }
    }

    /// A required boolean.
    pub(super) fn flag(&self, key: &str) -> Option<bool> {
        match self.required(key)? {
            Value::Bool(flag) => Some(*flag),
            other => {
                self.mismatch(key, "boolean", other);
                None
            }
        }
    }

    pub(super) fn vec3(&self, key: &str) -> Option<Vec3> {
        let obj = self.object(key)?;
        let (x, y, z) = (
            obj.num("x", Bound::Any),
            obj.num("y", Bound::Any),
            obj.num("z", Bound::Any),
        );
        Some(Vec3 {
            x: x?,
            y: y?,
            z: z?,
        })
    }

    pub(super) fn quat(&self, key: &str) -> Option<Quat> {
        let obj = self.object(key)?;
        let (x, y, z, w) = (
            obj.num("x", Bound::Any),
            obj.num("y", Bound::Any),
            obj.num("z", Bound::Any),
            obj.num("w", Bound::Any),
        );
        Some(Quat {
            x: x?,
            y: y?,
            z: z?,
            w: w?,
        })
    }

    // ---- containers ----------------------------------------------------------------------

    pub(super) fn object(&self, key: &str) -> Option<Obj<'a>> {
        let value = self.required(key)?;
        Obj::open(value, self.field_path(key), self.sink)
    }

    pub(super) fn opt_object(&self, key: &str) -> Option<Option<Obj<'a>>> {
        let Some(value) = self.take(key) else {
            return Some(None);
        };
        Obj::open(value, self.field_path(key), self.sink).map(Some)
    }

    fn open_array(&self, key: &str, value: &'a Value, min: usize) -> Option<Arr<'a>> {
        let Value::Array(items) = value else {
            self.mismatch(key, "array", value);
            return None;
        };
        if items.len() < min {
            self.invalid(key, format!("needs at least {min} item(s)"));
            return None;
        }
        if items.len() > self.sink.max_items {
            self.sink.issue(
                LIMIT_EXCEEDED,
                &self.field_path(key),
                format!(
                    "{} items; the limit is {}",
                    items.len(),
                    self.sink.max_items
                ),
            );
            return None;
        }
        Some(Arr {
            items,
            path: self.field_path(key),
            sink: self.sink,
        })
    }

    pub(super) fn array(&self, key: &str, min: usize) -> Option<Arr<'a>> {
        self.open_array(key, self.required(key)?, min)
    }

    pub(super) fn opt_array(&self, key: &str, min: usize) -> Option<Option<Arr<'a>>> {
        let Some(value) = self.take(key) else {
            return Some(None);
        };
        self.open_array(key, value, min).map(Some)
    }

    // ---- sections the importer keeps raw -------------------------------------------------

    /// A field the origin schema declares but this importer does not interpret yet: marked as
    /// known (so it is not reported as unknown) and returned untouched. It stays in the raw tree.
    pub(super) fn deferred(&self, key: &str) -> Option<&'a Value> {
        self.take(key)
    }

    /// Marks every remaining key as deferred (payload of a section interpreted only by its
    /// identity: performance channels).
    pub(super) fn defer_rest(&self) {
        let mut seen = self.seen.borrow_mut();
        seen.extend(self.map.keys().map(String::as_str));
    }
}

impl Drop for Obj<'_> {
    fn drop(&mut self) {
        let seen = self.seen.borrow();
        for key in self.map.keys().filter(|key| !seen.contains(key.as_str())) {
            self.sink.notices.borrow_mut().push(Notice {
                code: UNKNOWN_FIELD,
                path: child(&self.path, key),
                message: format!(
                    "`{key}` is not a field of render-bundle 1.0; kept as is, not interpreted"
                ),
            });
        }
    }
}

pub(super) struct Arr<'a> {
    items: &'a [Value],
    path: String,
    sink: &'a Sink,
}

impl<'a> Arr<'a> {
    pub(super) fn len(&self) -> usize {
        self.items.len()
    }

    /// Reads every element as an object. `Some` only if every element read cleanly; all
    /// elements are visited either way so all their problems are reported.
    pub(super) fn objects<T>(
        &self,
        mut read: impl FnMut(usize, &Obj<'a>) -> Option<T>,
    ) -> Option<Vec<T>> {
        let mut out = Vec::with_capacity(self.items.len());
        let mut clean = true;
        for (index, value) in self.items.iter().enumerate() {
            let element = Obj::open(value, at(&self.path, index), self.sink)
                .and_then(|obj| read(index, &obj));
            match element {
                Some(item) => out.push(item),
                None => clean = false,
            }
        }
        clean.then_some(out)
    }

    /// Every element is a string (possibly empty).
    pub(super) fn strings(&self) -> Option<Vec<&'a str>> {
        let mut out = Vec::with_capacity(self.items.len());
        let mut clean = true;
        for (index, value) in self.items.iter().enumerate() {
            if let Value::String(text) = value {
                out.push(text.as_str());
                continue;
            }
            clean = false;
            self.sink.issue(
                INVALID_TYPE,
                &at(&self.path, index),
                format!("expected string, got {}", kind_of(value)),
            );
        }
        clean.then_some(out)
    }
}
