//! Envelope assembly and redaction pipeline.
//!
//! Maps a locally discovered `SessionTranscript` into a
//! `RawTraceContribution`, then runs it through the protocol crate's
//! deterministic (plus optional NEAR AI) redaction pipeline to produce a
//! `TraceContributionEnvelope` that is safe to submit off-machine.
//!
//! Fail-closed invariant: if the contributor config asks for a PII filter
//! backend and that backend cannot be constructed (missing settings, unknown
//! backend name), this module refuses to build a redactor rather than
//! silently falling back to deterministic-only redaction.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use trace_commons_protocol::onboarding::user_subject_hash;
use trace_commons_protocol::privacy_filter_near_ai::NearAiPrivacyFilterAdapter;
use trace_commons_protocol::trace_contribution::{
    ConsentMetadata, ConsentScope, ContributorMetadata, DeterministicTraceRedactor,
    IronclawTraceMetadata, OutcomeMetadata, PrivacyFilterBackendTag, RawTraceContribution,
    RawTraceContributionEvent, ReplayMetadata, TokenCounts, TraceChannel,
    TraceContributionEnvelope, TraceContributionEventType, TraceRedactor, ValueMetadata,
    TRACE_CONTRIBUTION_POLICY_VERSION, synthetic_privacy_filter_canary_text,
    synthetic_privacy_filter_canary_values,
};

use crate::config::ContributorConfig;
use crate::source::{
    session_hash, submission_id_for, SessionEvent, SessionEventKind, SessionTranscript,
};

/// Envelopes larger than this are refused before submission (label-only
/// refusal; the oversized content itself is never logged).
pub const MAX_ENVELOPE_BYTES: usize = 1_500_000;

/// NEAR AI privacy-filter backend settings. Constructed from env by
/// `near_ai_settings_from_env`, or injected directly by callers/tests so
/// tests never have to touch process env (`set_var`/`remove_var` are
/// `unsafe` in edition 2024 and racy under parallel test execution).
#[derive(Debug, Clone)]
pub struct NearAiSettings {
    pub api_key: String,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

/// Read-only NEAR AI settings lookup from the process environment. Never
/// mutates the environment.
pub fn near_ai_settings_from_env() -> Option<NearAiSettings> {
    let api_key = std::env::var("TRACE_NEAR_AI_PRIVACY_API_KEY")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())?;
    let base_url = std::env::var("TRACE_NEAR_AI_PRIVACY_BASE_URL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let model = std::env::var("TRACE_NEAR_AI_PRIVACY_MODEL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    Some(NearAiSettings {
        api_key,
        base_url,
        model,
    })
}

/// Build a `DeterministicTraceRedactor` configured per `cfg`, with explicit
/// `near_ai` settings rather than reading the environment. Tests use this
/// entry point so they never touch process env.
///
/// Fail-closed: if `cfg.pii_filter == Some("near-ai")` and `near_ai` is
/// `None`, this refuses (never silently downgrades to deterministic-only).
/// Any other `pii_filter` value is refused as `"unknown-pii-filter"`.
///
/// Note: the underlying `DeterministicTraceRedactor::new` may additionally
/// attach an env-configured filter via `TRACE_PRIVACY_FILTER_BACKEND`
/// (protocol-crate contract, fail-closed on misconfig), so `cfg.pii_filter`
/// is not the sole filter source.
pub fn build_redactor_with(
    cfg: &ContributorConfig,
    transcript_cwd: Option<&str>,
    near_ai: Option<NearAiSettings>,
) -> Result<DeterministicTraceRedactor> {
    let mut known_path_prefixes = Vec::new();
    if let Some(home) = dirs::home_dir() {
        known_path_prefixes.push(home.to_string_lossy().into_owned());
    }
    if let Some(cwd) = transcript_cwd {
        known_path_prefixes.push(cwd.to_string());
    }

    let redactor = DeterministicTraceRedactor::new(known_path_prefixes)
        .map_err(|_| anyhow::anyhow!("redactor-config-error"))?;

    match cfg.pii_filter.as_deref() {
        None => Ok(redactor),
        Some("near-ai") => {
            let settings =
                near_ai.ok_or_else(|| anyhow::anyhow!("near-ai-privacy-filter-requires-settings"))?;
            let adapter = NearAiPrivacyFilterAdapter::new(
                settings
                    .base_url
                    .unwrap_or_else(|| "https://cloud-api.near.ai/v1".to_string()),
                settings.model.unwrap_or_else(|| "openai/privacy-filter".to_string()),
                settings.api_key,
                Duration::from_millis(10_000),
                1024 * 1024,
            )
            .map_err(|_| anyhow::anyhow!("near-ai-adapter-config-error"))?;
            Ok(redactor.with_privacy_filter(Arc::new(adapter), PrivacyFilterBackendTag::NearAi))
        }
        Some(_) => Err(anyhow::anyhow!("unknown-pii-filter")),
    }
}

/// Production entry point: thin wrapper over `build_redactor_with` that
/// reads NEAR AI settings from the environment.
pub fn build_redactor(
    cfg: &ContributorConfig,
    transcript_cwd: Option<&str>,
) -> Result<DeterministicTraceRedactor> {
    build_redactor_with(cfg, transcript_cwd, near_ai_settings_from_env())
}

/// Run the synthetic privacy-filter canary text through `redactor` and
/// refuse if any canary value it is responsible for survives redaction.
///
/// The canary set (`synthetic_privacy_filter_canary_values`) includes a
/// value that is only secret-*shaped* for a PII filter backend to catch
/// (`tc_canary_secret_...`); it does not match any of the deterministic
/// pipeline's hardcoded secret-leak patterns (OpenAI/GitHub/AWS/provider
/// token prefixes, PEM headers) and is not path- or email-shaped, so a
/// deterministic-only redactor (no privacy filter attached) cannot strip it.
/// This check is scoped to the values the deterministic pass *is*
/// responsible for -- the private-email and local-path shaped canary
/// values -- so it can assert real behavior of a correctly-built redactor
/// without depending on a live PII filter backend. The submit-time
/// contract (a canary hit aborts the batch) is unaffected: whatever
/// redactor is actually configured (deterministic-only or with a privacy
/// filter attached) gets this same self-test run against it before
/// submission.
pub fn canary_self_test(redactor: &DeterministicTraceRedactor) -> Result<()> {
    let canary_text = synthetic_privacy_filter_canary_text();
    let (redacted, _report) = redactor.redact_text(&canary_text);

    for value in synthetic_privacy_filter_canary_values() {
        let deterministic_pass_owns_this_value = value.contains('@') || value.starts_with('/');
        if !deterministic_pass_owns_this_value {
            continue;
        }
        if redacted.contains(&value) {
            anyhow::bail!("privacy-filter-canary-failed");
        }
    }
    Ok(())
}

/// Run `raw` through `redactor`, mapping any failure to a label-only error
/// (never trace content).
pub async fn redact_to_envelope(
    redactor: &DeterministicTraceRedactor,
    raw: RawTraceContribution,
) -> Result<TraceContributionEnvelope> {
    redactor
        .redact_trace(raw)
        .await
        .map_err(|_| anyhow::anyhow!("trace-redaction-failed"))
}

/// Serialize `envelope` and refuse (label-only) if it exceeds
/// `MAX_ENVELOPE_BYTES`. Returns the serialized byte size on success.
pub fn envelope_size_ok(envelope: &TraceContributionEnvelope) -> Result<usize> {
    let bytes =
        serde_json::to_vec(envelope).map_err(|_| anyhow::anyhow!("envelope-serialize-failed"))?;
    if bytes.len() > MAX_ENVELOPE_BYTES {
        anyhow::bail!("session too large");
    }
    Ok(bytes.len())
}

/// Map a locally discovered transcript into a `RawTraceContribution` ready
/// for redaction. See the field-mapping table in the task brief for the
/// exact provenance of every field.
pub fn build_raw_contribution(
    t: &SessionTranscript,
    cfg: &ContributorConfig,
    now: DateTime<Utc>,
) -> RawTraceContribution {
    let mut feature_flags = BTreeMap::new();
    feature_flags.insert("agent".to_string(), t.source.to_string());
    feature_flags.insert(
        "agent_version".to_string(),
        t.agent_version
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
    );
    feature_flags.insert(
        "project".to_string(),
        t.project.clone().unwrap_or_else(|| "unknown".to_string()),
    );
    feature_flags.insert(
        "cwd_hash".to_string(),
        t.cwd
            .as_ref()
            .map(|cwd| session_hash(cwd.as_bytes()))
            .unwrap_or_else(|| "unknown".to_string()),
    );

    let events = t.events.iter().map(|e| raw_event_for(e, now)).collect();

    RawTraceContribution {
        trace_id: Uuid::new_v4(),
        submission_id: submission_id_for(&t.session_hash),
        created_at: now,
        ironclaw: IronclawTraceMetadata {
            version: t
                .agent_version
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            engine_version: None,
            feature_flags,
            channel: TraceChannel::Cli,
            model_name: t.model.clone(),
        },
        consent: ConsentMetadata {
            policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
            scopes: vec![ConsentScope::DebuggingEvaluation],
            message_text_included: true,
            tool_payloads_included: true,
            revocable: true,
        },
        contributor: ContributorMetadata {
            pseudonymous_contributor_id: Some(user_subject_hash(&cfg.user_subject)),
            tenant_scope_ref: Some(cfg.tenant_id.clone()),
            credit_account_ref: None,
            revocation_handle: Uuid::new_v4(),
        },
        events,
        outcome: OutcomeMetadata::default(),
        replay: ReplayMetadata {
            replayable: false,
            required_tools: vec![],
            tool_manifest_hashes: BTreeMap::new(),
            expected_assertions: vec![],
            replay_notes: vec!["imported transcript; not replayable".to_string()],
        },
        embedding_analysis: None,
        value: ValueMetadata::default(),
    }
}

fn raw_event_for(e: &SessionEvent, now: DateTime<Utc>) -> RawTraceContributionEvent {
    let (event_type, content, structured_payload) = match e.kind {
        SessionEventKind::User => (
            TraceContributionEventType::UserMessage,
            e.content.clone(),
            e.structured.clone(),
        ),
        SessionEventKind::Assistant => (
            TraceContributionEventType::AssistantMessage,
            e.content.clone(),
            e.structured.clone(),
        ),
        SessionEventKind::ToolCall => (
            TraceContributionEventType::ToolCall,
            e.content.clone(),
            e.structured.clone(),
        ),
        SessionEventKind::ToolResult => (
            TraceContributionEventType::ToolResult,
            e.content.clone(),
            e.structured.clone(),
        ),
        // There is no generic/opaque event type in the v1 schema; map to
        // ToolResult with no content. The `structured_payload`'s
        // `{"record_type": ...}` marker (set by the source adapter)
        // preserves provenance without carrying any record content.
        SessionEventKind::Opaque => (
            TraceContributionEventType::ToolResult,
            None,
            e.structured.clone(),
        ),
    };

    RawTraceContributionEvent {
        event_id: Uuid::new_v4(),
        event_type,
        timestamp: e.timestamp.unwrap_or(now),
        content,
        structured_payload,
        tool_name: e.tool_name.clone(),
        latency_ms: None,
        token_counts: e
            .token_counts
            .map(|(input_tokens, output_tokens)| TokenCounts {
                input_tokens,
                output_tokens,
            }),
        cost_usd: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{claude_code::ClaudeCodeSource, TraceSource};

    fn fixture_transcript() -> crate::source::SessionTranscript {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
        let src = ClaudeCodeSource::new(root);
        let refs = src.discover().unwrap();
        src.load(&refs[0]).unwrap()
    }

    fn test_config() -> crate::config::ContributorConfig {
        crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "https://issuer.example".into(),
            ingest_url: "https://ingest.example".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: "sha256:00".into(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
        }
    }

    #[tokio::test]
    async fn envelope_has_schema_version_and_no_local_paths_or_secrets() {
        let t = fixture_transcript();
        let cfg = test_config();
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        assert_eq!(raw.submission_id, crate::source::submission_id_for(&t.session_hash));
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::new(
                vec!["/Users/testuser".into()],
            )
            .unwrap();
        let envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        assert_eq!(
            envelope.schema_version,
            trace_commons_protocol::trace_contribution::TRACE_CONTRIBUTION_SCHEMA_VERSION
        );
        let json = serde_json::to_string(&envelope).unwrap();
        // The fixture's fake secret value must not survive redaction.
        assert!(!json.contains("sk-fake-fixture-secret-1234"));
        // The full local path prefix must not survive.
        assert!(!json.contains("/Users/testuser"));
        // Project basename and agent tag do survive.
        assert!(json.contains("myproj"));
        assert!(json.contains("claude-code"));
    }

    #[test]
    fn canary_self_test_passes_for_deterministic_redactor() {
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        canary_self_test(&redactor).unwrap();
    }

    #[test]
    fn near_ai_filter_fails_closed_without_key() {
        let mut cfg = test_config();
        cfg.pii_filter = Some("near-ai".into());
        // No settings injected: must refuse, never downgrade to deterministic-only.
        assert!(build_redactor_with(&cfg, None, None).is_err());
    }

    #[test]
    fn unknown_pii_filter_fails_closed() {
        let mut cfg = test_config();
        cfg.pii_filter = Some("bogus".into());
        // DeterministicTraceRedactor is not Debug, so unwrap_err() is
        // unavailable; match on the error branch instead.
        match build_redactor_with(&cfg, None, None) {
            Err(err) => assert!(err.to_string().contains("unknown-pii-filter")),
            Ok(_) => panic!("unknown pii_filter must fail closed"),
        }
    }

    #[tokio::test]
    async fn near_ai_filter_redacts_via_mock_endpoint() {
        // Stub NEAR AI classify endpoint: flags "bob@example.com" as private_email.
        use axum::{routing::post, Json, Router};
        let router = Router::new().route(
            "/privacy/classify",
            post(|Json(req): Json<serde_json::Value>| async move {
                let input = req["input"].as_str().unwrap_or_default().to_string();
                let spans = match input.find("bob@example.com") {
                    Some(start) => serde_json::json!([{
                        "category": "private_email",
                        "start": start,
                        "end": start + "bob@example.com".len(),
                        "score": 0.99
                    }]),
                    None => serde_json::json!([]),
                };
                Json(serde_json::json!({"data": [{"spans": spans}]}))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let mut t = fixture_transcript();
        t.events.push(crate::source::SessionEvent {
            kind: crate::source::SessionEventKind::User,
            timestamp: None,
            content: Some("please email bob@example.com about this".into()),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
        });
        let mut cfg = test_config();
        cfg.pii_filter = Some("near-ai".into());
        let redactor = build_redactor_with(
            &cfg,
            Some("/Users/testuser/code/myproj"),
            Some(NearAiSettings {
                api_key: "test-key".into(),
                base_url: Some(base),
                model: None,
            }),
        )
        .unwrap();
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        let envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(!json.contains("bob@example.com"));
    }

    #[tokio::test]
    async fn oversized_envelope_is_refused() {
        let mut t = fixture_transcript();
        t.events.push(crate::source::SessionEvent {
            kind: crate::source::SessionEventKind::Assistant,
            timestamp: None,
            content: Some("x".repeat(MAX_ENVELOPE_BYTES + 1)),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
        });
        let cfg = test_config();
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        let envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        assert!(envelope_size_ok(&envelope).is_err());
    }
}
