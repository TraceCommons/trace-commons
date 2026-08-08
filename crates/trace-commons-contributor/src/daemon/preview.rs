//! Real preview: redact one session without uploading, and report exactly
//! what would leave the machine.
//!
//! Before this module existed, the IPC `"preview"` arm reported
//! `entry.size_bytes` -- the raw session file's size on disk. Redaction
//! shrinks (and reshapes) the payload, so that number overstated what
//! actually gets sent and was the one figure backing a contributor's consent
//! decision. `build_preview` runs the *same* redaction path `submit_one`
//! uses (`build_redactor_with` + `build_raw_contribution` +
//! `redact_to_envelope`), so preview and upload can never disagree.

use anyhow::Result;
use chrono::Utc;

use crate::config::{ConfigStore, ContributorConfig};
use crate::envelope::{
    NearAiSettings, build_raw_contribution, build_redactor_with, envelope_size, redact_to_envelope,
};
use crate::source::{SessionRef, TraceSource};
use trace_commons_protocol::trace_contribution::TraceContributionEventType;

/// What preview reports to the contributor before they consent to upload.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PreviewSummary {
    pub would_send_bytes: usize,
    pub raw_session_bytes: u64,
    pub event_count: usize,
    pub opening_prompt: String,
    pub redactions: std::collections::BTreeMap<String, u32>,
    pub pii_labels_present: Vec<String>,
    /// The consent scopes this device **requests**, taken from the local
    /// config, which is what `build_raw_contribution` stamps onto the
    /// envelope here.
    ///
    /// These are not necessarily the scopes an upload ends up carrying. An
    /// actual submission mints an upload claim first, and
    /// `submit::stamp_granted_scopes` then overwrites the envelope with the
    /// **granted** set the issuer echoed back -- falling back to the
    /// requested set only when the issuer is old enough not to echo one.
    /// Preview cannot show the granted set without minting a claim, which
    /// it deliberately does not do (preview is a local operation and must
    /// work offline).
    ///
    /// So this field is an upper bound on what an upload will claim, never
    /// an under-statement: the issuer can only narrow the request, never
    /// widen it. A consumer rendering this to a contributor should say
    /// "requested", not "will be sent as". Separately, an entry's approval
    /// is pinned to exactly this requested set
    /// (`QueueEntry::approved_scopes`), so a local widening between preview
    /// and upload revokes the approval rather than riding along with it.
    pub consent_scopes: Vec<String>,
    pub residual_risk: String,
}

/// Redact one session without uploading and describe exactly what would be
/// sent. Same redaction path the uploader uses.
///
/// `store` is accepted for signature parity with the other entry points that
/// build a redactor/envelope (`submit_one`); preview does not itself read or
/// write through it -- everything it needs comes from `cfg` and the already
/// -resolved `source`/`session_ref`.
pub async fn build_preview(
    _store: &ConfigStore,
    cfg: &ContributorConfig,
    near_ai: Option<NearAiSettings>,
    source: &dyn TraceSource,
    session_ref: &SessionRef,
) -> Result<(PreviewSummary, String)> {
    let transcript = source.load(session_ref)?;
    let raw_session_bytes = session_ref.size_bytes;

    let redactor = build_redactor_with(cfg, transcript.cwd.as_deref(), near_ai)
        .map_err(|_| anyhow::anyhow!("pii-filter-unavailable"))?;
    let raw = build_raw_contribution(&transcript, cfg, Utc::now());
    let envelope = redact_to_envelope(&redactor, raw).await?;
    let would_send_bytes = envelope_size(&envelope)?;

    let event_count = envelope.events.len();
    let opening_prompt = envelope
        .events
        .iter()
        .find(|e| e.event_type == TraceContributionEventType::UserMessage)
        .and_then(|e| e.redacted_content.clone())
        .unwrap_or_default();
    let opening_prompt = truncate_chars(&opening_prompt, 200);

    let redactions = envelope.privacy.redaction_counts.clone();
    let pii_labels_present = envelope.privacy.pii_labels_present.clone();
    let consent_scopes = envelope.consent.scopes.iter().map(wire_name).collect();
    let residual_risk = serde_json::to_value(envelope.privacy.residual_pii_risk)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "pattern-based".to_string());

    let body = serde_json::to_string_pretty(&envelope.events)
        .map_err(|_| anyhow::anyhow!("preview-body-serialize-failed"))?;

    Ok((
        PreviewSummary {
            would_send_bytes,
            raw_session_bytes,
            event_count,
            opening_prompt,
            redactions,
            pii_labels_present,
            consent_scopes,
            residual_risk,
        },
        body,
    ))
}

/// Serde's wire name for a `Serialize` value that serializes to a bare
/// string (every enum used here does, via `#[serde(rename_all =
/// "snake_case")]`).
fn wire_name<T: serde::Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Truncate to at most `max_chars` characters, always on a char boundary.
fn truncate_chars(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::claude_code::ClaudeCodeSource;

    fn sample_cfg(store: &ConfigStore) -> ContributorConfig {
        let device = crate::identity::DeviceIdentity::load_or_generate(store).unwrap();
        ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "http://issuer.invalid".into(),
            ingest_url: "http://ingest.invalid".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id.clone(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
        }
    }

    /// A session with a planted secret, so redaction has something to do.
    fn fixture_session() -> (tempfile::TempDir, ClaudeCodeSource, SessionRef) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        let project = root.join("-Users-testuser-code-myproj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("11111111-1111-1111-1111-111111111111.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\
             \"content\":\"deploy with key sk-fake-fixture-secret-1234\"},\
             \"cwd\":\"/Users/testuser/code/myproj\",\
             \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
             \"sessionId\":\"11111111-1111-1111-1111-111111111111\",\
             \"uuid\":\"a1\"}\n",
        )
        .unwrap();
        let src = ClaudeCodeSource::new(root);
        let r = src.discover().unwrap().remove(0);
        (dir, src, r)
    }

    #[tokio::test]
    async fn preview_reports_the_redacted_size_not_the_raw_size() {
        // The defect this task exists to fix: the old code returned the raw
        // file size. Measured, the redacted envelope is substantially LARGER
        // than the session file it came from -- envelope metadata dominates --
        // so the old number understated what actually leaves the machine.
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (summary, _body) = build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        assert!(summary.raw_session_bytes > 0);
        assert!(summary.would_send_bytes > 0);
        assert_ne!(
            summary.would_send_bytes as u64, summary.raw_session_bytes,
            "a redacted envelope is not the same size as the raw session file"
        );
    }

    #[tokio::test]
    async fn preview_reports_what_redaction_actually_removed() {
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (summary, _body) = build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        let total: u32 = summary.redactions.values().sum();
        assert!(
            total > 0,
            "planted secret should appear in the counts: {:?}",
            summary.redactions
        );
    }

    #[tokio::test]
    async fn preview_body_does_not_contain_the_planted_secret() {
        // The whole point of showing a body is that it is the redacted one.
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (_summary, body) = build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        assert!(
            !body.contains("sk-fake-fixture-secret-1234"),
            "secret survived into the preview body"
        );
    }

    #[tokio::test]
    async fn preview_carries_an_opening_prompt_and_an_event_count() {
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (summary, _body) = build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        assert_eq!(summary.event_count, 1);
        assert!(!summary.opening_prompt.is_empty());
        assert!(
            !summary
                .opening_prompt
                .contains("sk-fake-fixture-secret-1234"),
            "the opening prompt must be the redacted one"
        );
    }

    #[tokio::test]
    async fn preview_opening_prompt_is_truncated() {
        // 200 chars, so a huge first message cannot dominate a queue row.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        let project = root.join("-Users-testuser-code-myproj");
        std::fs::create_dir_all(&project).unwrap();
        let long = "x".repeat(500);
        std::fs::write(
            project.join("22222222-2222-2222-2222-222222222222.jsonl"),
            format!(
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"{long}\"}},\
                 \"cwd\":\"/Users/testuser/code/myproj\",\
                 \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                 \"sessionId\":\"22222222-2222-2222-2222-222222222222\",\"uuid\":\"a1\"}}\n"
            ),
        )
        .unwrap();
        let src = ClaudeCodeSource::new(root);
        let r = src.discover().unwrap().remove(0);
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (summary, _body) = build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        assert!(summary.opening_prompt.chars().count() <= 200);
    }
}
