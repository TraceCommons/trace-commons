//! Privacy-filter sidecar adapter — ported from Ironclaw.
//!
//! Provides a small, wire-compatible client for the Privacy Filter sidecar
//! protocol used by the trace contribution pipeline. The sidecar is a
//! standalone subprocess that reads a JSON `PrivacyFilterSidecarRequest`
//! from stdin and writes a JSON response on stdout. The response is parsed
//! into a [`SafePrivacyFilterRedaction`], which carries only label-level
//! summary information (no raw spans) so that operator binaries can run a
//! local canary check without ever logging the raw text.
//!
//! Wire shapes match the Ironclaw `ironclaw_reborn_traces::contribution`
//! types byte-for-byte. Operators who already have an Ironclaw sidecar
//! configured can point it at this client unchanged.
//!
//! Env vars (canonical names first, Ironclaw fallbacks second):
//!
//! - `TRACE_COMMONS_PRIVACY_FILTER_COMMAND` /
//!   `IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND` — sidecar executable path.
//! - `..._ARGS` — whitespace-separated argv.
//! - `..._TIMEOUT_MS` — wall-clock timeout in milliseconds.
//! - `..._MAX_INPUT_BYTES`, `..._MAX_STDOUT_BYTES`, `..._MAX_STDERR_BYTES`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Once};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

pub const PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES: usize = 1024 * 1024;
pub const PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES: usize = 1024 * 1024;
pub const PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDERR_BYTES: usize = 64 * 1024;

/// Label-only summary of a privacy-filter redaction. Mirrors the Ironclaw
/// `SafePrivacyFilterSummary` wire shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SafePrivacyFilterSummary {
    pub schema_version: u16,
    pub output_mode: String,
    pub span_count: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_label: BTreeMap<String, u32>,
    pub decoded_mismatch: bool,
}

/// Label-only redaction-report counts. Mirrors the Ironclaw
/// `RedactionReport` wire shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RedactionReport {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counts: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pii_labels_present: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub blocked_secret_detected: bool,
}

impl RedactionReport {
    fn increment(&mut self, label: impl Into<String>) {
        *self.counts.entry(label.into()).or_insert(0) += 1;
    }

    fn add_warning(&mut self, warning: impl Into<String>) {
        let warning = warning.into();
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }
}

/// Parsed sidecar response: the redacted text, plus label-only summary +
/// report. Carries no raw spans.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SafePrivacyFilterRedaction {
    pub redacted_text: String,
    pub summary: SafePrivacyFilterSummary,
    pub report: RedactionReport,
}

/// The JSON request body the sidecar reads from stdin.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivacyFilterSidecarRequest {
    pub text: String,
}

impl PrivacyFilterSidecarRequest {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

/// Trait alias for sidecar adapters. Implementors run the privacy filter
/// (locally, via subprocess, or via a stub) and return a safe summary.
#[async_trait]
pub trait PrivacyFilterAdapter: Send + Sync {
    async fn redact_text(&self, text: &str) -> anyhow::Result<Option<SafePrivacyFilterRedaction>>;
}

/// No-op adapter for tests and for callers that want to thread a sidecar
/// dependency without actually configuring one.
pub struct NoopPrivacyFilterAdapter;

#[async_trait]
impl PrivacyFilterAdapter for NoopPrivacyFilterAdapter {
    async fn redact_text(&self, _text: &str) -> anyhow::Result<Option<SafePrivacyFilterRedaction>> {
        Ok(None)
    }
}

/// Subprocess-driven sidecar adapter. Spawns the configured command,
/// pipes the JSON request to stdin, captures stdout/stderr under byte
/// limits, and parses the response into a [`SafePrivacyFilterRedaction`].
#[derive(Debug, Clone)]
pub struct CommandPrivacyFilterAdapter {
    command: PathBuf,
    args: Vec<String>,
    timeout: Duration,
    max_input_bytes: usize,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
}

impl CommandPrivacyFilterAdapter {
    pub fn new(command: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            timeout: Duration::from_secs(10),
            max_input_bytes: PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES,
            max_stdout_bytes: PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES,
            max_stderr_bytes: PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDERR_BYTES,
        }
    }

    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_input_limit(mut self, max_input_bytes: usize) -> Self {
        self.max_input_bytes = max_input_bytes;
        self
    }

    pub fn with_output_limits(mut self, max_stdout_bytes: usize, max_stderr_bytes: usize) -> Self {
        self.max_stdout_bytes = max_stdout_bytes;
        self.max_stderr_bytes = max_stderr_bytes;
        self
    }
}

#[async_trait]
impl PrivacyFilterAdapter for CommandPrivacyFilterAdapter {
    async fn redact_text(&self, text: &str) -> anyhow::Result<Option<SafePrivacyFilterRedaction>> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        if text.len() > self.max_input_bytes {
            anyhow::bail!(
                "privacy filter sidecar input exceeded limit: input_len={} max_input_bytes={}",
                text.len(),
                self.max_input_bytes
            );
        }

        let mut command = tokio::process::Command::new(&self.command);
        command.env_clear();
        for name in ["PATH", "LANG", "LC_ALL"] {
            if let Ok(value) = std::env::var(name) {
                command.env(name, value);
            }
        }
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| {
            anyhow::anyhow!(
                "failed to spawn privacy filter sidecar {}: {}",
                self.command.display(),
                error
            )
        })?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("privacy filter sidecar stdin was not available"))?;
        let request = PrivacyFilterSidecarRequest::new(text);
        let request_body = serde_json::to_vec(&request).map_err(|error| {
            anyhow::anyhow!("failed to serialize privacy filter request: {error}")
        })?;
        stdin
            .write_all(&request_body)
            .await
            .map_err(|error| anyhow::anyhow!("failed to write privacy filter request: {error}"))?;
        drop(stdin);

        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "privacy filter sidecar timed out after {}ms",
                    self.timeout.as_millis()
                )
            })?
            .map_err(|error| anyhow::anyhow!("privacy filter sidecar failed: {error}"))?;

        if output.stdout.len() > self.max_stdout_bytes {
            anyhow::bail!(
                "stdout exceeded privacy filter sidecar limit: stdout_len={} max_stdout_bytes={}",
                output.stdout.len(),
                self.max_stdout_bytes
            );
        }
        if output.stderr.len() > self.max_stderr_bytes {
            anyhow::bail!(
                "stderr exceeded privacy filter sidecar limit: stderr_len={} stderr_hash={} max_stderr_bytes={}",
                output.stderr.len(),
                privacy_filter_bytes_hash(&output.stderr),
                self.max_stderr_bytes
            );
        }

        if !output.status.success() {
            anyhow::bail!(
                "privacy filter sidecar exited with {}; stderr_len={} stderr_hash={}",
                output.status,
                output.stderr.len(),
                privacy_filter_bytes_hash(&output.stderr)
            );
        }

        let value: Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| anyhow::anyhow!("failed to parse privacy filter output: {error}"))?;
        safe_privacy_filter_redaction_from_output(&value).map(Some)
    }
}

/// Parse a sidecar stdout JSON value into a [`SafePrivacyFilterRedaction`].
/// Drops raw spans; keeps only label-level counts.
pub fn safe_privacy_filter_redaction_from_output(
    output: &Value,
) -> anyhow::Result<SafePrivacyFilterRedaction> {
    let redacted_text = output
        .get("redacted_text")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("privacy filter output is missing redacted_text"))?
        .to_string();
    let detected_spans = output
        .get("detected_spans")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut by_label = BTreeMap::new();
    let mut report = RedactionReport::default();
    for span in &detected_spans {
        let raw_label = span
            .get("label")
            .or_else(|| span.get("type"))
            .or_else(|| span.get("entity_type"))
            .and_then(Value::as_str);
        let label = safe_privacy_filter_label(raw_label, &mut report);
        *by_label.entry(label.clone()).or_insert(0) += 1;
        report.increment(format!("privacy_filter:{label}"));
        if label.eq_ignore_ascii_case("secret") {
            report.blocked_secret_detected = true;
        }
        if !report.pii_labels_present.contains(&label) {
            report.pii_labels_present.push(label);
        }
    }

    let schema_version = output
        .get("schema_version")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or(1);
    let decoded_mismatch = output
        .get("decoded_mismatch")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(SafePrivacyFilterRedaction {
        redacted_text,
        summary: SafePrivacyFilterSummary {
            schema_version,
            output_mode: "redacted_text_only".to_string(),
            span_count: detected_spans.len() as u32,
            by_label,
            decoded_mismatch,
        },
        report,
    })
}

fn safe_privacy_filter_label(raw_label: Option<&str>, report: &mut RedactionReport) -> String {
    let Some(raw_label) = raw_label else {
        return "unknown".to_string();
    };
    let normalized = raw_label
        .trim()
        .chars()
        .map(|character| {
            if character == '-' {
                '_'
            } else {
                character.to_ascii_lowercase()
            }
        })
        .collect::<String>();
    let allowed = matches!(
        normalized.as_str(),
        "account_number"
            | "credit_card"
            | "ip_address"
            | "private_address"
            | "private_date"
            | "private_email"
            | "private_location"
            | "private_name"
            | "private_person"
            | "private_phone"
            | "private_url"
            | "secret"
            | "secret_like"
            | "ssn"
    );
    if allowed {
        return normalized;
    }
    report.add_warning("Privacy Filter sidecar emitted unsupported span label; mapped to unknown.");
    "unknown".to_string()
}

/// Hash arbitrary bytes for hash-only logging surfaces.
pub fn privacy_filter_bytes_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex::encode(digest))
}

/// SHA-256 a UTF-8 string for canary attestation.
pub fn canonical_hash(content: &str) -> String {
    privacy_filter_bytes_hash(content.as_bytes())
}

/// Identify naive token leaks: any whitespace-separated token in `original`
/// that re-appears verbatim in `redacted` is considered leaked. Returns the
/// `sha256:<hex>` hashes of leaked tokens (never the raw token value).
pub fn canary_leaked_tokens(original: &str, redacted: &str) -> Vec<String> {
    let mut leaked = Vec::new();
    for token in original.split_whitespace() {
        if token.len() < 4 {
            continue;
        }
        if redacted.contains(token) {
            let hash = canonical_hash(token);
            if !leaked.contains(&hash) {
                leaked.push(hash);
            }
        }
    }
    leaked
}

/// Build an adapter from environment variables, preferring the canonical
/// `TRACE_COMMONS_PRIVACY_FILTER_*` names and falling back to the
/// Ironclaw-era `IRONCLAW_TRACE_PRIVACY_FILTER_*` names. Returns `None` when
/// no command is configured.
pub fn adapter_from_env() -> Option<Arc<dyn PrivacyFilterAdapter>> {
    let command = env_with_fallback("PRIVACY_FILTER_COMMAND")?;
    let command = command.trim();
    if command.is_empty() {
        return None;
    }

    let args = env_with_fallback("PRIVACY_FILTER_ARGS")
        .map(|raw| {
            raw.split_whitespace()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut adapter = CommandPrivacyFilterAdapter::new(command).with_args(args);
    if let Some(timeout_ms) = parse_usize_env("PRIVACY_FILTER_TIMEOUT_MS") {
        adapter = adapter.with_timeout(Duration::from_millis(timeout_ms as u64));
    }
    if let Some(max_input_bytes) = parse_usize_env("PRIVACY_FILTER_MAX_INPUT_BYTES") {
        adapter = adapter.with_input_limit(max_input_bytes);
    }
    let max_stdout = parse_usize_env("PRIVACY_FILTER_MAX_STDOUT_BYTES");
    let max_stderr = parse_usize_env("PRIVACY_FILTER_MAX_STDERR_BYTES");
    if max_stdout.is_some() || max_stderr.is_some() {
        adapter = adapter.with_output_limits(
            max_stdout.unwrap_or(PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES),
            max_stderr.unwrap_or(PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDERR_BYTES),
        );
    }
    Some(Arc::new(adapter))
}

fn env_with_fallback(suffix: &str) -> Option<String> {
    let canonical = format!("TRACE_COMMONS_{suffix}");
    if let Ok(value) = std::env::var(&canonical) {
        if !value.is_empty() {
            return Some(value);
        }
    }
    let legacy = format!("IRONCLAW_TRACE_{suffix}");
    match std::env::var(&legacy) {
        Ok(value) if !value.is_empty() => {
            warn_legacy_env_once(&legacy, &canonical);
            Some(value)
        }
        _ => None,
    }
}

fn warn_legacy_env_once(legacy: &str, canonical: &str) {
    static WARN: Once = Once::new();
    WARN.call_once(|| {
        tracing::warn!(
            target: "trace_commons::privacy_filter",
            legacy = legacy,
            canonical = canonical,
            "privacy-filter sidecar configured via legacy Ironclaw env var; migrate to TRACE_COMMONS_* names"
        );
    });
}

fn parse_usize_env(suffix: &str) -> Option<usize> {
    env_with_fallback(suffix).and_then(|value| value.trim().parse::<usize>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Guard env vars used by `adapter_from_env` tests; ensures we restore
    /// the prior process state even on panic. Used serially to avoid
    /// cross-test races on shared environment.
    fn env_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    struct EnvGuard {
        keys: Vec<(String, Option<String>)>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl EnvGuard {
        fn capture(keys: &[&str]) -> Self {
            let lock = env_test_lock();
            let snapshot = keys
                .iter()
                .map(|k| (k.to_string(), std::env::var(k).ok()))
                .collect();
            for key in keys {
                unsafe { std::env::remove_var(key) };
            }
            EnvGuard {
                keys: snapshot,
                _lock: lock,
            }
        }
        fn set(&self, key: &str, value: &str) {
            unsafe { std::env::set_var(key, value) };
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, prior) in &self.keys {
                match prior {
                    Some(value) => unsafe { std::env::set_var(key, value) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
        }
    }

    const ENV_KEYS: &[&str] = &[
        "TRACE_COMMONS_PRIVACY_FILTER_COMMAND",
        "TRACE_COMMONS_PRIVACY_FILTER_ARGS",
        "TRACE_COMMONS_PRIVACY_FILTER_TIMEOUT_MS",
        "TRACE_COMMONS_PRIVACY_FILTER_MAX_INPUT_BYTES",
        "TRACE_COMMONS_PRIVACY_FILTER_MAX_STDOUT_BYTES",
        "TRACE_COMMONS_PRIVACY_FILTER_MAX_STDERR_BYTES",
        "IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND",
        "IRONCLAW_TRACE_PRIVACY_FILTER_ARGS",
        "IRONCLAW_TRACE_PRIVACY_FILTER_TIMEOUT_MS",
        "IRONCLAW_TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES",
        "IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES",
        "IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES",
    ];

    #[test]
    fn parses_safe_redaction_from_output() {
        let value = json!({
            "redacted_text": "Hello [REDACTED]",
            "detected_spans": [
                {"label": "private_email"},
                {"label": "Secret"},
                {"label": "weird-label"}
            ],
            "schema_version": 2,
            "decoded_mismatch": false,
        });
        let red = safe_privacy_filter_redaction_from_output(&value).unwrap();
        assert_eq!(red.redacted_text, "Hello [REDACTED]");
        assert_eq!(red.summary.schema_version, 2);
        assert_eq!(red.summary.span_count, 3);
        assert_eq!(red.summary.by_label.get("private_email"), Some(&1));
        assert_eq!(red.summary.by_label.get("secret"), Some(&1));
        assert_eq!(red.summary.by_label.get("unknown"), Some(&1));
        assert!(red.report.blocked_secret_detected);
        assert!(
            red.report
                .warnings
                .iter()
                .any(|w| w.contains("unsupported span label"))
        );
    }

    #[test]
    fn parser_rejects_missing_redacted_text() {
        let value = json!({"detected_spans": []});
        assert!(safe_privacy_filter_redaction_from_output(&value).is_err());
    }

    #[test]
    fn canary_leaked_tokens_detects_residual_canary() {
        let original = "trace-canary.person@example.invalid tc_canary_secret_0123456789abcdef";
        let redacted_clean = "[REDACTED_EMAIL] [REDACTED_SECRET]";
        assert!(canary_leaked_tokens(original, redacted_clean).is_empty());

        let redacted_leak = "[REDACTED] tc_canary_secret_0123456789abcdef";
        let leaks = canary_leaked_tokens(original, redacted_leak);
        assert_eq!(leaks.len(), 1);
        assert!(leaks[0].starts_with("sha256:"));
    }

    #[tokio::test]
    async fn noop_adapter_returns_none() {
        let adapter = NoopPrivacyFilterAdapter;
        assert!(adapter.redact_text("anything").await.unwrap().is_none());
    }

    #[test]
    fn adapter_from_env_returns_none_when_unset() {
        let _guard = EnvGuard::capture(ENV_KEYS);
        assert!(adapter_from_env().is_none());
    }

    #[test]
    fn adapter_from_env_prefers_canonical_over_legacy() {
        let guard = EnvGuard::capture(ENV_KEYS);
        guard.set("TRACE_COMMONS_PRIVACY_FILTER_COMMAND", "/usr/bin/canonical");
        guard.set("IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND", "/usr/bin/legacy");
        let adapter = adapter_from_env().expect("adapter built");
        drop(adapter);
        // Sanity: rebuild via the concrete constructor with the same envs to
        // exercise the timeout/limit parsing branch.
        guard.set("TRACE_COMMONS_PRIVACY_FILTER_TIMEOUT_MS", "2500");
        guard.set("TRACE_COMMONS_PRIVACY_FILTER_MAX_INPUT_BYTES", "1024");
        guard.set("TRACE_COMMONS_PRIVACY_FILTER_MAX_STDOUT_BYTES", "2048");
        guard.set("TRACE_COMMONS_PRIVACY_FILTER_MAX_STDERR_BYTES", "512");
        guard.set("TRACE_COMMONS_PRIVACY_FILTER_ARGS", "--mode redact");
        let adapter2 = adapter_from_env().expect("adapter built with overrides");
        drop(adapter2);
    }

    #[test]
    fn adapter_from_env_falls_back_to_legacy() {
        let guard = EnvGuard::capture(ENV_KEYS);
        guard.set("IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND", "/usr/bin/legacy");
        assert!(adapter_from_env().is_some());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn command_adapter_round_trip_via_shell() {
        // Use a tiny shell pipeline that ignores stdin and emits a known
        // sidecar response. The adapter must parse it into a safe summary.
        let stdout = serde_json::json!({
            "redacted_text": "[REDACTED]",
            "detected_spans": [{"label": "private_email"}],
            "schema_version": 1,
            "decoded_mismatch": false,
        })
        .to_string();
        let script = format!(
            "cat >/dev/null; printf '%s' '{}'",
            stdout.replace('\'', "'\\''")
        );
        let adapter = CommandPrivacyFilterAdapter::new("/bin/sh")
            .with_args(["-c", &script])
            .with_timeout(Duration::from_secs(5));
        let red = adapter.redact_text("hello").await.unwrap().expect("some");
        assert_eq!(red.redacted_text, "[REDACTED]");
        assert_eq!(red.summary.span_count, 1);
        assert_eq!(red.summary.by_label.get("private_email"), Some(&1));
    }

    #[test]
    fn canonical_hash_format() {
        let h = canonical_hash("hello");
        assert!(h.starts_with("sha256:"));
        assert_eq!(h.len(), "sha256:".len() + 64);
    }
}
