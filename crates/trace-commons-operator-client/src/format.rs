//! Output formatting helpers for operator binaries.
//!
//! Every operator binary exposes a `--json` flag. When set, the binary
//! emits a uniform envelope:
//!
//! ```json
//! { "request": { "method": "GET", "url": "<sanitized>" }, "data": <server-response> }
//! ```
//!
//! When `--json` is not set, the binary prints a human-readable rendering
//! that varies per command. The helpers here cover the common patterns:
//!
//! - [`emit_json`] — write the wrapper envelope to a writer.
//! - [`print_table`] — render a fixed-column ASCII table for list endpoints.
//! - [`print_kv`] — render a "label: value" block for single-item endpoints.

use std::io::{self, Write};

use serde::Serialize;

/// Wrapper envelope for `--json` output. Generic over the server payload
/// so each command can describe its own response shape statically.
#[derive(Debug, Serialize)]
pub struct JsonEnvelope<'a, T: Serialize> {
    pub request: JsonRequestSummary<'a>,
    pub data: &'a T,
}

#[derive(Debug, Serialize)]
pub struct JsonRequestSummary<'a> {
    pub method: &'a str,
    /// URL with query string + fragment stripped.
    pub url: String,
}

/// Write the JSON envelope to `out`, pretty-printed for human inspection.
pub fn emit_json<T: Serialize, W: Write>(
    out: &mut W,
    method: &str,
    url: &str,
    data: &T,
) -> io::Result<()> {
    let envelope = JsonEnvelope {
        request: JsonRequestSummary {
            method,
            url: crate::error::sanitize_url_for_display(url),
        },
        data,
    };
    serde_json::to_writer_pretty(&mut *out, &envelope).map_err(io::Error::other)?;
    out.write_all(b"\n")
}

/// Render a list of rows as a fixed-column ASCII table. Each row is a
/// `Vec<String>` matching `headers.len()`. Columns are sized to the longest
/// cell (header or data) and separated by two spaces.
///
/// Rows that don't match `headers.len()` are skipped with a warning written
/// to `out`'s sibling stderr-equivalent — callers should validate row shape
/// before calling.
pub fn print_table<W: Write>(
    out: &mut W,
    headers: &[&str],
    rows: &[Vec<String>],
) -> io::Result<()> {
    if headers.is_empty() {
        return Ok(());
    }
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        if row.len() != headers.len() {
            continue;
        }
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
    }

    write_row(out, headers, &widths)?;
    // Underline.
    let underline: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    let underline_refs: Vec<&str> = underline.iter().map(|s| s.as_str()).collect();
    write_row(out, &underline_refs, &widths)?;

    for row in rows {
        if row.len() != headers.len() {
            continue;
        }
        let row_refs: Vec<&str> = row.iter().map(|s| s.as_str()).collect();
        write_row(out, &row_refs, &widths)?;
    }
    Ok(())
}

fn write_row<W: Write>(out: &mut W, cells: &[&str], widths: &[usize]) -> io::Result<()> {
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            out.write_all(b"  ")?;
        }
        write!(out, "{:<width$}", cell, width = widths[i])?;
    }
    out.write_all(b"\n")
}

/// Render a `(label, value)` block where labels are right-padded to a uniform
/// width. Useful for "show single record" commands.
pub fn print_kv<W: Write>(out: &mut W, pairs: &[(&str, &str)]) -> io::Result<()> {
    if pairs.is_empty() {
        return Ok(());
    }
    let width = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (k, v) in pairs {
        writeln!(out, "{:<width$}  {}", k, v, width = width)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    struct Sample {
        id: String,
        approved: bool,
    }

    #[test]
    fn emit_json_writes_envelope_with_sanitized_url() {
        let mut buf = Vec::new();
        emit_json(
            &mut buf,
            "GET",
            "https://api.example/v1/traces?token=secret",
            &Sample {
                id: "sub-1".into(),
                approved: true,
            },
        )
        .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains(r#""method": "GET""#));
        assert!(out.contains(r#""url": "https://api.example/v1/traces""#));
        assert!(!out.contains("token=secret"));
        assert!(out.contains(r#""id": "sub-1""#));
        assert!(out.contains(r#""approved": true"#));
    }

    #[test]
    fn print_table_renders_header_underline_and_rows() {
        let mut buf = Vec::new();
        print_table(
            &mut buf,
            &["id", "status"],
            &[
                vec!["sub-1".into(), "leased".into()],
                vec!["sub-22".into(), "approved".into()],
            ],
        )
        .unwrap();
        let out = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "id      status  ");
        assert_eq!(lines[1], "------  --------");
        assert_eq!(lines[2], "sub-1   leased  ");
        assert_eq!(lines[3], "sub-22  approved");
    }

    #[test]
    fn print_table_skips_rows_with_wrong_arity() {
        let mut buf = Vec::new();
        print_table(
            &mut buf,
            &["id", "status"],
            &[
                vec!["sub-1".into()],                                    // skipped
                vec!["sub-2".into(), "approved".into(), "extra".into()], // skipped
                vec!["sub-3".into(), "leased".into()],
            ],
        )
        .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("sub-3"));
        assert!(!out.contains("sub-1"));
        assert!(!out.contains("sub-2"));
    }

    #[test]
    fn print_kv_pads_labels_uniformly() {
        let mut buf = Vec::new();
        print_kv(
            &mut buf,
            &[("id", "sub-1"), ("submission_status", "approved")],
        )
        .unwrap();
        let out = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "id                 sub-1");
        assert_eq!(lines[1], "submission_status  approved");
    }

    #[test]
    fn print_kv_handles_empty_input() {
        let mut buf = Vec::new();
        print_kv(&mut buf, &[]).unwrap();
        assert!(buf.is_empty());
    }
}
