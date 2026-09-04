//! Dynamic value type used for node properties and events.
//!
//! Mirrors the shape of the JavaScript values the original Elementary runtime
//! exchanged with its frontend, minus functions.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// A dynamically typed value: property payloads and event payloads.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Object(BTreeMap<String, Value>),
    Array(Vec<Value>),
    Float32Array(Vec<f32>),
}

impl Value {
    pub fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
        Value::Object(
            entries
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Interpret an `Array` of numbers or a `Float32Array` as a list of floats.
    /// Returns `None` if the value is neither, or if any array element is not a number.
    pub fn as_f32_list(&self) -> Option<Vec<f32>> {
        match self {
            Value::Float32Array(a) => Some(a.clone()),
            Value::Array(a) => a
                .iter()
                .map(|v| v.as_number().map(|n| n as f32))
                .collect(),
            _ => None,
        }
    }

    /// Serialize to compact JSON. Non-finite numbers and `Undefined` become `null`.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    pub fn write_json(&self, out: &mut String) {
        match self {
            Value::Undefined | Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => write_number(out, *n),
            Value::String(s) => write_json_string(out, s),
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_json(out);
                }
                out.push(']');
            }
            Value::Float32Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_number(out, *item as f64);
                }
                out.push(']');
            }
            Value::Object(map) => {
                out.push('{');
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_json_string(out, k);
                    out.push(':');
                    v.write_json(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_number(out: &mut String, n: f64) {
    if !n.is_finite() {
        out.push_str("null");
    } else if n == n.trunc() && n.abs() < 1e15 {
        let _ = write!(out, "{}", n as i64);
    } else {
        let _ = write!(out, "{}", n);
    }
}

fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Number(v)
    }
}
impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Number(v as f64)
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Number(v as f64)
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_owned())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}
impl From<Vec<f32>> for Value {
    fn from(v: Vec<f32>) -> Self {
        Value::Float32Array(v)
    }
}
impl From<Vec<Value>> for Value {
    fn from(v: Vec<Value>) -> Self {
        Value::Array(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_shapes() {
        let v = Value::object([
            ("b", Value::Bool(true)),
            ("a", Value::Number(1.5)),
            ("n", Value::Number(440.0)),
            ("s", Value::String("x\"y".into())),
            ("arr", Value::Float32Array(vec![0.5, 1.0])),
            ("u", Value::Undefined),
        ]);
        assert_eq!(
            v.to_json(),
            r#"{"a":1.5,"arr":[0.5,1],"b":true,"n":440,"s":"x\"y","u":null}"#
        );
    }
}
