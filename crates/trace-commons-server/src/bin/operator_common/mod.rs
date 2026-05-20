//! Helpers shared by the operator binaries (`trace-commons-review`,
//! `trace-commons-admin`). Brought into each binary via
//! `#[path = "operator_common/mod.rs"] mod operator_common;`.
//!
//! The functions here are intentionally small and concern themselves with
//! rendering JSON values produced by the trace-commons-server API. They never
//! log bearer tokens or query strings. URL sanitization happens inside the
//! foundation crate.
//!
//! Each binary uses a subset of these helpers, so we tag the module with
//! `#![allow(dead_code)]` to silence per-bin warnings for the helpers that
//! aren't exercised in one of the two surfaces.

#![allow(dead_code)]

use std::io::Write;

use serde_json::Value;
use trace_commons_operator_client::format;

/// Print an `{"items": [...]}` (or top-level array) list response as a
/// fixed-column table. `columns` is the ordered list of JSON field names; the
/// header row uses the same names verbatim so operators can correlate the
/// rendered output with the server schema.
pub fn render_items<W: Write>(
    out: &mut W,
    label: &str,
    value: &Value,
    columns: &[&str],
) -> std::io::Result<()> {
    let items_value = value.get("items").unwrap_or(value);
    let items = items_value.as_array();
    writeln!(out, "{label}:")?;
    let Some(items) = items else {
        writeln!(out, "  (no items)")?;
        return Ok(());
    };
    if items.is_empty() {
        writeln!(out, "  (no items)")?;
        return Ok(());
    }
    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|item| {
            columns
                .iter()
                .map(|col| stringify_field(item.get(col)))
                .collect()
        })
        .collect();
    format::print_table(out, columns, &rows)
}

/// Print an object's fields as a "label: value" block. Each entry is rendered
/// only if the field is present and non-null. Indentation matches the
/// Ironclaw CLI conventions ("  label").
pub fn render_kv_fields<W: Write>(
    out: &mut W,
    value: &Value,
    pairs: &[(&str, &str)],
) -> std::io::Result<()> {
    let mut rendered: Vec<(String, String)> = Vec::new();
    for (label, field) in pairs {
        if let Some(child) = value.get(field) {
            if !child.is_null() {
                rendered.push(((*label).to_string(), stringify_field(Some(child))));
            }
        }
    }
    if rendered.is_empty() {
        return Ok(());
    }
    let refs: Vec<(&str, &str)> = rendered
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    format::print_kv(out, &refs)
}

/// Render a JSON object's keys as a simple "k=v" map. Used by analytics
/// summary's `by_status`, `by_privacy_risk`, etc.
pub fn render_json_map<W: Write>(
    out: &mut W,
    label: &str,
    value: Option<&Value>,
) -> std::io::Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let Some(obj) = value.as_object() else {
        return Ok(());
    };
    if obj.is_empty() {
        return Ok(());
    }
    writeln!(out, "{label}:")?;
    for (k, v) in obj {
        writeln!(out, "    {k}: {}", stringify_field(Some(v)))?;
    }
    Ok(())
}

fn stringify_field(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => other.to_string(),
    }
}

/// Compose the sanitized URL that `format::emit_json` should record. The
/// foundation `format::emit_json` re-sanitizes via `sanitize_url_for_display`,
/// so we only need to pass the endpoint + path here without a query string.
pub fn sanitized_url(endpoint: &str, path: &str) -> String {
    let endpoint = endpoint.trim_end_matches('/');
    format!("{endpoint}{path}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_items_handles_top_level_array() {
        let mut buf = Vec::new();
        let value = json!([{"id": "a", "status": "ok"}, {"id": "b", "status": "bad"}]);
        render_items(&mut buf, "Items", &value, &["id", "status"]).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("Items:"));
        assert!(out.contains("id"));
        assert!(out.contains("a"));
        assert!(out.contains("bad"));
    }

    #[test]
    fn render_items_handles_items_envelope() {
        let mut buf = Vec::new();
        let value = json!({"items": [{"id": "x", "status": "ok"}]});
        render_items(&mut buf, "Items", &value, &["id", "status"]).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("x"));
    }

    #[test]
    fn render_items_handles_empty() {
        let mut buf = Vec::new();
        let value = json!({"items": []});
        render_items(&mut buf, "Items", &value, &["id"]).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("(no items)"));
    }

    #[test]
    fn render_kv_fields_skips_missing_and_null() {
        let mut buf = Vec::new();
        let value = json!({"a": "alpha", "b": null});
        render_kv_fields(
            &mut buf,
            &value,
            &[("a label", "a"), ("b label", "b"), ("c label", "c")],
        )
        .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("alpha"));
        assert!(!out.contains("b label"));
        assert!(!out.contains("c label"));
    }

    #[test]
    fn render_json_map_writes_entries() {
        let mut buf = Vec::new();
        let value = json!({"approved": 4, "rejected": 1});
        render_json_map(&mut buf, "  by status", Some(&value)).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("by status:"));
        assert!(out.contains("approved: 4"));
        assert!(out.contains("rejected: 1"));
    }

    #[test]
    fn sanitized_url_trims_trailing_slash() {
        assert_eq!(
            sanitized_url("https://api.example/", "/v1/foo"),
            "https://api.example/v1/foo"
        );
    }
}
