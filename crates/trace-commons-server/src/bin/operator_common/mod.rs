// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Helpers shared by the operator binaries (`trace-commons-admin`,
//! `trace-commons-review`, `trace-commons-tenant`, `trace-commons-worker`).
//! Brought into each binary via
//! `#[path = "operator_common/mod.rs"] mod operator_common;`.
//!
//! The helpers here render JSON values produced by the trace-commons-server
//! API, build the shared HTTP client, and carry the three request flows that
//! are byte-identical between two binaries apart from the URL path. They never
//! log bearer tokens or query strings. URL sanitization happens inside the
//! foundation crate.
//!
//! Most items are used by every binary. The handful that are not carry an
//! item-level `#[allow(dead_code)]` naming the binaries that do use them;
//! there is deliberately no module-level blanket allow, so a helper that
//! stops being used anywhere still shows up as a warning.

use std::io::{Write, stdout};
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::ValueEnum;
use reqwest::Method;
use serde_json::Value;
use trace_commons_operator_client::host_allowlist::HostAllowlist;
use trace_commons_operator_client::{Client, format};

// --- shared clap value enums ---
//
// These three `ValueEnum`s were duplicated verbatim across the operator
// binaries. The variant identifiers are what clap renders in `--help` and
// accepts on the command line, so they must not be renamed.

/// Privacy-risk filter. Used by all four operator binaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PrivacyRisk {
    Low,
    Medium,
    High,
}

impl PrivacyRisk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Consent-scope filter in kebab-case query form. Used by `trace-commons-admin`
/// and `trace-commons-worker`. `trace-commons-tenant` keeps its own enum: it
/// carries an extra `PublicAttribution` variant and both a snake_case wire form
/// and a kebab-case query form.
#[allow(dead_code)] // admin + worker only
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ConsentScope {
    DebuggingEvaluation,
    BenchmarkOnly,
    RankingTraining,
    ModelTraining,
}

#[allow(dead_code)] // admin + worker only
impl ConsentScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DebuggingEvaluation => "debugging-evaluation",
            Self::BenchmarkOnly => "benchmark-only",
            Self::RankingTraining => "ranking-training",
            Self::ModelTraining => "model-training",
        }
    }
}

/// Corpus-status filter. Used by `trace-commons-admin`,
/// `trace-commons-tenant` and `trace-commons-worker`.
#[allow(dead_code)] // admin + tenant + worker only
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CorpusStatus {
    Accepted,
    Quarantined,
    Rejected,
    Revoked,
    Expired,
    Purged,
}

#[allow(dead_code)] // admin + tenant + worker only
impl CorpusStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Quarantined => "quarantined",
            Self::Rejected => "rejected",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
            Self::Purged => "purged",
        }
    }
}

/// Build the operator HTTP client. `allowed_hosts` is the raw CSV form of
/// `--allowed-hosts` / `TRACE_COMMONS_ALLOWED_HOSTS`; `None` leaves the
/// foundation default allowlist in place.
pub fn build_client(
    endpoint: &str,
    bearer_token_env: &str,
    allowed_hosts: Option<&str>,
) -> Result<Client> {
    let mut builder = Client::builder(endpoint, bearer_token_env);
    if let Some(csv) = allowed_hosts {
        builder = builder.host_allowlist(HostAllowlist::from_csv(csv));
    }
    Ok(builder.build()?)
}

/// Borrow an owned query-parameter list as the `(&str, &str)` pairs the
/// foundation client takes.
pub fn borrow_query<'a>(owned: &'a [(&'a str, String)]) -> Vec<(&'a str, &'a str)> {
    owned.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

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
#[allow(dead_code)] // admin only
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

/// Emit a JSON response with the sanitized request URL, the way every operator
/// binary's `--json` path does.
pub fn emit_json(endpoint: &str, method: &str, path: &str, value: &Value) -> Result<()> {
    let url = sanitized_url(endpoint, path);
    format::emit_json(&mut stdout(), method, &url, value)?;
    Ok(())
}

// --- shared request flows ---
//
// The three flows below were duplicated between two binaries each, differing
// only in the URL path (and, for the ranker export, in which consent-scope
// spelling the caller had already resolved). Each binary keeps its own clap
// `Args` struct, because the worker's variants carry an extra
// `--bearer-token-env` with a per-route default; the caller lowers its struct
// into the plain parameter structs here.

/// Query parameters shared by the replay-dataset and ranker-training exports.
/// `consent_scope`, `status` and `privacy_risk` are already resolved to their
/// wire spellings by the caller.
#[allow(dead_code)] // admin + tenant + worker only
pub struct ExportQuery {
    pub purpose: Option<String>,
    pub consent_scope: Option<String>,
    pub status: Option<String>,
    pub privacy_risk: Option<String>,
    pub limit: Option<usize>,
    pub output: Option<PathBuf>,
}

#[allow(dead_code)] // admin + tenant + worker only
impl ExportQuery {
    /// Build the owned query-parameter list. Parameter order matches the
    /// original per-binary implementations.
    fn owned_pairs(&self) -> Result<Vec<(&'static str, String)>> {
        let mut owned: Vec<(&'static str, String)> = Vec::new();
        if let Some(limit) = self.limit {
            owned.push(("limit", limit.to_string()));
        }
        if let Some(purpose) = self.purpose.as_deref() {
            let trimmed = purpose.trim();
            if trimmed.is_empty() {
                anyhow::bail!("--purpose must be non-empty");
            }
            owned.push(("purpose", trimmed.to_string()));
        }
        if let Some(scope) = self.consent_scope.as_deref() {
            owned.push(("consent_scope", scope.to_string()));
        }
        if let Some(status) = self.status.as_deref() {
            owned.push(("status", status.to_string()));
        }
        if let Some(risk) = self.privacy_risk.as_deref() {
            owned.push(("privacy_risk", risk.to_string()));
        }
        Ok(owned)
    }
}

/// Write a raw export body to `output`, pretty-printing it when it parsed as
/// JSON. `what` names the export in the failure message.
#[allow(dead_code)] // admin + tenant + worker only
fn write_export_file(output: &Path, raw: &str, value: &Value, what: &str) -> Result<()> {
    let pretty = if value.is_null() {
        raw.to_string()
    } else {
        serde_json::to_string_pretty(value)?
    };
    std::fs::write(output, pretty)
        .map_err(|e| anyhow::anyhow!("failed to write {} {}: {}", what, output.display(), e))?;
    Ok(())
}

/// The central replay-dataset export. `path` is `/v1/datasets/replay` for
/// `trace-commons-admin` and `/v1/workers/replay-export` for
/// `trace-commons-worker`.
#[allow(dead_code)] // admin + worker only
pub async fn replay_dataset_export(
    client: &Client,
    endpoint: &str,
    path: &str,
    args: ExportQuery,
    json: bool,
) -> Result<()> {
    let owned = args.owned_pairs()?;
    let query = borrow_query(&owned);
    let raw = client
        .call_raw::<()>(Method::GET, path, &query, None)
        .await?;
    let value: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);

    if let Some(output) = args.output.as_ref() {
        write_export_file(output, &raw, &value, "replay dataset export")?;
        if json {
            emit_json(endpoint, "GET", path, &value)?;
        } else {
            let mut out = stdout();
            writeln!(
                out,
                "Wrote central replay dataset export to {}",
                output.display()
            )?;
            render_kv_fields(
                &mut out,
                &value,
                &[
                    ("  export id", "export_id"),
                    ("  manifest id", "manifest_id"),
                    ("  audit event id", "audit_event_id"),
                    ("  item count", "item_count"),
                ],
            )?;
        }
        return Ok(());
    }

    emit_json(endpoint, "GET", path, &value)
}

/// The ranker-training candidates/pairs export. Used by
/// `trace-commons-tenant` (`/v1/ranker/training-*`) and
/// `trace-commons-worker` (`/v1/workers/ranker/training-*`).
#[allow(dead_code)] // tenant + worker only
pub async fn ranker_training_export(
    client: &Client,
    endpoint: &str,
    path: &str,
    output_label: &str,
    item_field: &str,
    args: ExportQuery,
    json: bool,
) -> Result<()> {
    let owned = args.owned_pairs()?;
    let query = borrow_query(&owned);
    let raw = client
        .call_raw::<()>(Method::GET, path, &query, None)
        .await?;
    let value: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);

    if let Some(output) = args.output.as_ref() {
        write_export_file(output, &raw, &value, "ranker training export")?;
        if json {
            emit_json(endpoint, "GET", path, &value)?;
        } else {
            let mut out = stdout();
            writeln!(
                out,
                "Wrote central {} export to {}",
                output_label,
                output.display()
            )?;
            render_kv_fields(
                &mut out,
                &value,
                &[
                    ("  export id", "export_id"),
                    ("  audit event id", "audit_event_id"),
                    ("  purpose", "purpose"),
                    ("  item count", "item_count"),
                ],
            )?;
        }
        return Ok(());
    }

    if json {
        emit_json(endpoint, "GET", path, &value)
    } else {
        let items = value.get(item_field).cloned().unwrap_or(Value::Null);
        let envelope = serde_json::json!({ "items": items });
        render_items(
            &mut stdout(),
            &format!("Central {output_label}"),
            &envelope,
            &[
                "submission_id",
                "trace_id",
                "ranker_score",
                "preferred_submission_id",
                "rejected_submission_id",
                "reason",
            ],
        )?;
        Ok(())
    }
}

/// Body parameters for the benchmark-conversion request. `consent_scope`,
/// `status` and `privacy_risk` are already resolved to their wire spellings.
#[allow(dead_code)] // admin + worker only
pub struct BenchmarkConvertRequest {
    pub purpose: String,
    pub limit: Option<usize>,
    pub consent_scope: Option<String>,
    pub status: Option<String>,
    pub privacy_risk: Option<String>,
    pub external_ref: Option<String>,
}

/// The benchmark-conversion request. `path` is `/v1/benchmarks/convert` for
/// `trace-commons-admin` and `/v1/workers/benchmark-convert` for
/// `trace-commons-worker`; `heading` is the line printed above the rendered
/// fields in non-`--json` mode.
#[allow(dead_code)] // admin + worker only
pub async fn benchmark_convert(
    client: &Client,
    endpoint: &str,
    path: &str,
    heading: &str,
    args: BenchmarkConvertRequest,
    json: bool,
) -> Result<()> {
    let trimmed = args.purpose.trim();
    if trimmed.is_empty() {
        anyhow::bail!("--purpose must be non-empty");
    }
    let mut body = serde_json::json!({ "purpose": trimmed });
    if let Some(limit) = args.limit {
        body["limit"] = serde_json::json!(limit);
    }
    if let Some(scope) = args.consent_scope {
        body["consent_scope"] = Value::String(scope);
    }
    if let Some(status) = args.status {
        body["status"] = Value::String(status);
    }
    if let Some(risk) = args.privacy_risk {
        body["privacy_risk"] = Value::String(risk);
    }
    if let Some(external_ref) = args.external_ref {
        body["external_ref"] = Value::String(external_ref);
    }
    let value: Value = client
        .call_json::<Value, Value>(Method::POST, path, &[], Some(&body))
        .await?;
    if json {
        emit_json(endpoint, "POST", path, &value)?;
    } else {
        let mut out = stdout();
        writeln!(out, "{heading}")?;
        render_kv_fields(
            &mut out,
            &value,
            &[
                ("  conversion id", "conversion_id"),
                ("  audit event id", "audit_event_id"),
                ("  item count", "item_count"),
                ("  purpose", "purpose"),
            ],
        )?;
    }
    Ok(())
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
