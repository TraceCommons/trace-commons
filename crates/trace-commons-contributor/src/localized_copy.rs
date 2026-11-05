//! Locale-neutral presentation only. Callers supply facts, not completed
//! sentences. Keys and English templates travel together so an older client
//! cannot apply a translation to a changed disclosure.

use serde_json::{Value, json};

mod disclosures;
mod inference;
pub mod notifications;
mod skills;
mod structured;
mod surfaces;

pub fn text(key: &str, english: &str, parameters: Value) -> Value {
    json!({"key": key, "english": english, "parameters": parameters})
}

pub fn plural(key: &str, one: &str, other: &str, count: u64, mut parameters: Value) -> Value {
    parameters["count"] = json!(count);
    json!({"key": key, "english": {"one": one, "other": other}, "parameters": parameters})
}

/// Single-pass English compatibility rendering. Inserted values are never
/// parsed again; a folder named `{count}` stays that folder's name.
pub fn english(message: &Value) -> String {
    let source = &message["english"];
    let template = source
        .as_str()
        .or_else(|| {
            source[if message["parameters"]["count"].as_u64() == Some(1) {
                "one"
            } else {
                "other"
            }]
            .as_str()
        })
        .unwrap_or_default();
    let mut result = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            rest = &rest[start..];
            break;
        };
        let name = &rest[start + 1..start + end];
        let value = &message["parameters"][name];
        if value.is_null() {
            result.push_str(&rest[start..=start + end]);
        } else if value.get("key").is_some() {
            result.push_str(&english(value));
        } else if let Some(value) = value.as_str().or_else(|| value["english"].as_str()) {
            result.push_str(value);
        } else {
            result.push_str(&value.to_string());
        }
        rest = &rest[start + end + 1..];
    }
    result.push_str(rest);
    result
}

pub fn message(operation: &str, input: &Value) -> Option<Value> {
    inference::message(operation, input)
        .or_else(|| disclosures::message(operation, input))
        .or_else(|| notifications::message(operation, input))
        .or_else(|| skills::message(operation, input))
        .or_else(|| structured::message(operation, input))
        .or_else(|| surfaces::message(operation, input))
}

pub fn templates() -> Vec<Value> {
    let mut messages = inference::templates();
    messages.extend(disclosures::templates());
    messages.extend(notifications::templates());
    messages.extend(skills::templates());
    messages.extend(structured::templates());
    messages.extend(surfaces::templates());
    messages
}
