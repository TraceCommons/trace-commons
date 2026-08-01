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
    RawTraceContributionEvent, ReplayMetadata, TRACE_CONTRIBUTION_POLICY_VERSION, TokenCounts,
    TraceAllowedUse, TraceChannel, TraceContributionEnvelope, TraceContributionEventType,
    TraceRedactor, ValueMetadata, run_privacy_filter_canary, synthetic_privacy_filter_canary_text,
    synthetic_privacy_filter_canary_values,
};

use crate::config::ContributorConfig;
use crate::source::{
    SessionEvent, SessionEventKind, SessionTranscript, session_hash, submission_id_for,
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
            let settings = near_ai
                .ok_or_else(|| anyhow::anyhow!("near-ai-privacy-filter-requires-settings"))?;
            let adapter = NearAiPrivacyFilterAdapter::new(
                settings
                    .base_url
                    .unwrap_or_else(|| "https://cloud-api.near.ai/v1".to_string()),
                settings
                    .model
                    .unwrap_or_else(|| "openai/privacy-filter".to_string()),
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

/// Once-per-batch precondition, extended to also exercise any privacy-filter
/// backend attached to `redactor` (NEAR AI, sidecar, ...).
///
/// `canary_self_test` only checks the deterministic pass (the email/path
/// shaped canary values it is responsible for). A well-formed no-op filter
/// -- one that returns 200 with an empty span list for everything -- passes
/// that check trivially, because the deterministic pass never depended on
/// the filter to strip anything. This function additionally runs the
/// protocol crate's `run_privacy_filter_canary` directly against the
/// attached adapter (if any) and fails closed when the canary's synthetic
/// secret/email/path values survive redaction through that adapter alone,
/// which is exactly the failure mode a no-op or broken filter exhibits.
pub async fn canary_self_test_async(redactor: &DeterministicTraceRedactor) -> Result<()> {
    canary_self_test(redactor)?;

    if let Some(adapter) = redactor.attached_privacy_filter() {
        let report = run_privacy_filter_canary(adapter.as_ref())
            .await
            .map_err(|_| anyhow::anyhow!("privacy-filter-canary-failed"))?;
        if !report.healthy {
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

/// Re-scan the *finished* envelope with the secret detector and report
/// whether any secret shape survived redaction. Defense-in-depth: a
/// correctly-redacted envelope yields zero detector hits, but a survivor (a
/// detect-then-redact bug, or a non-string payload value the string-leaf
/// pass never visited) is still caught here and fails the session closed.
///
/// Unlike a whole-token diff, this only flags secret *shapes* (pattern
/// matches, PEM blocks, cue-gated entropy) -- ordinary prose and the
/// `{"record_type":...}` markers on Opaque events are not secret-shaped and
/// never trip it, so there are no prose false positives.
pub fn envelope_has_residual_secret(
    redactor: &DeterministicTraceRedactor,
    envelope: &TraceContributionEnvelope,
) -> Result<bool> {
    let json = serde_json::to_string(envelope)
        .map_err(|_| anyhow::anyhow!("envelope-serialize-failed"))?;
    let (_out, report) = redactor.redact_text(&json);
    Ok(report.blocked_secret_detected)
}

/// Serialize `raw` and refuse (label-only) if the pre-redaction payload
/// already exceeds `MAX_ENVELOPE_BYTES`. The finished envelope carries the
/// same event content plus additional metadata (trace card, consent, etc.),
/// so an over-limit raw contribution cannot serialize to an in-limit
/// envelope. Checking here lets `submit` skip the expensive chunked,
/// networked privacy-filter pass on sessions that would be refused for size
/// anyway; `envelope_size_ok` remains the authoritative post-redaction guard.
pub fn raw_contribution_size_ok(raw: &RawTraceContribution) -> Result<usize> {
    let bytes = serde_json::to_vec(raw).map_err(|_| anyhow::anyhow!("raw-serialize-failed"))?;
    if bytes.len() > MAX_ENVELOPE_BYTES {
        anyhow::bail!("session too large");
    }
    Ok(bytes.len())
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

    let events: Vec<RawTraceContributionEvent> = t
        .events
        .iter()
        .map(|e| raw_event_for(e, now, cfg.include_message_text, cfg.include_tool_payloads))
        .collect();
    let message_text_included = events.iter().any(|event| {
        matches!(
            event.event_type,
            TraceContributionEventType::UserMessage
                | TraceContributionEventType::AssistantMessage
                | TraceContributionEventType::Reasoning
        ) && event.content.is_some()
    });
    let tool_payloads_included = events.iter().any(|event| {
        event.tool_name.is_some()
            || !event.structured_payload.is_null()
            || (matches!(
                event.event_type,
                TraceContributionEventType::ToolCall | TraceContributionEventType::ToolResult
            ) && event.content.is_some())
    });

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
            scopes: {
                let parsed = parse_scope_names(&cfg.consent_scopes);
                if parsed.is_empty() {
                    vec![ConsentScope::DebuggingEvaluation]
                } else {
                    parsed
                }
            },
            message_text_included,
            tool_payloads_included,
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

/// Parse wire names to typed scopes; unknown names are skipped (they were
/// validated at login/claim time) — never panics.
pub fn parse_scope_names(names: &[String]) -> Vec<ConsentScope> {
    names
        .iter()
        .filter_map(|name| match name.as_str() {
            "debugging_evaluation" => Some(ConsentScope::DebuggingEvaluation),
            "benchmark_only" => Some(ConsentScope::BenchmarkOnly),
            "ranking_training" => Some(ConsentScope::RankingTraining),
            "model_training" => Some(ConsentScope::ModelTraining),
            "public_attribution" => Some(ConsentScope::PublicAttribution),
            _ => None,
        })
        .collect()
}

/// Same for allowed uses (wire names -> TraceAllowedUse, unknown skipped).
pub fn parse_use_names(names: &[String]) -> Vec<TraceAllowedUse> {
    names
        .iter()
        .filter_map(|name| match name.as_str() {
            "debugging" => Some(TraceAllowedUse::Debugging),
            "evaluation" => Some(TraceAllowedUse::Evaluation),
            "benchmark_generation" => Some(TraceAllowedUse::BenchmarkGeneration),
            "ranking_model_training" => Some(TraceAllowedUse::RankingModelTraining),
            "model_training" => Some(TraceAllowedUse::ModelTraining),
            "aggregate_analytics" => Some(TraceAllowedUse::AggregateAnalytics),
            _ => None,
        })
        .collect()
}

/// Overwrite the envelope's consent metadata and trace card with the
/// claim-granted set. Called after redaction, before size check/upload.
pub fn apply_granted_scopes(
    envelope: &mut TraceContributionEnvelope,
    granted_scopes: &[ConsentScope],
    granted_uses: &[TraceAllowedUse],
) {
    // Upload claims grant consent scopes and allowed uses only. They carry no
    // message-text or tool-payload permission signal, so stamping a grant must
    // leave the contributor's effective content choices unchanged.
    envelope.consent.scopes = granted_scopes.to_vec();
    envelope.trace_card.allowed_uses = granted_uses.to_vec();
    envelope.trace_card.consent_scope = granted_scopes
        .iter()
        .find(|s| **s != ConsentScope::PublicAttribution)
        .copied()
        .unwrap_or(ConsentScope::DebuggingEvaluation);
}

fn raw_event_for(
    e: &SessionEvent,
    now: DateTime<Utc>,
    include_message_text: bool,
    include_tool_payloads: bool,
) -> RawTraceContributionEvent {
    let event_type = match e.kind {
        SessionEventKind::User => TraceContributionEventType::UserMessage,
        SessionEventKind::Assistant => TraceContributionEventType::AssistantMessage,
        SessionEventKind::Reasoning => TraceContributionEventType::Reasoning,
        SessionEventKind::ToolCall => TraceContributionEventType::ToolCall,
        SessionEventKind::ToolResult => TraceContributionEventType::ToolResult,
        // There is no generic/opaque event type in the v1 schema; map to
        // ToolResult with no content. The `structured_payload`'s
        // `{"record_type": ...}` marker (set by the source adapter)
        // preserves provenance without carrying any record content.
        SessionEventKind::Opaque => TraceContributionEventType::ToolResult,
    };

    // User, assistant, and reasoning content is message text. Tool-call and
    // tool-result content is tool payload. Every structured_payload, including
    // opaque provenance markers, is also tool payload.
    let include_content = match e.kind {
        SessionEventKind::User | SessionEventKind::Assistant | SessionEventKind::Reasoning => {
            include_message_text
        }
        SessionEventKind::ToolCall | SessionEventKind::ToolResult => include_tool_payloads,
        SessionEventKind::Opaque => false,
    };
    let content = if include_content {
        e.content.clone()
    } else {
        None
    };
    let structured_payload = if include_tool_payloads {
        e.structured.clone()
    } else {
        serde_json::Value::Null
    };

    RawTraceContributionEvent {
        event_id: Uuid::new_v4(),
        event_type,
        timestamp: e.timestamp.unwrap_or(now),
        content,
        structured_payload,
        tool_name: if include_tool_payloads {
            e.tool_name.clone()
        } else {
            None
        },
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
    use crate::source::{TraceSource, claude_code::ClaudeCodeSource};

    fn fixture_transcript() -> crate::source::SessionTranscript {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
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
            include_message_text: true,
            include_tool_payloads: true,
        }
    }

    fn transcript_with(
        events: Vec<crate::source::SessionEvent>,
    ) -> crate::source::SessionTranscript {
        crate::source::SessionTranscript {
            source: std::borrow::Cow::Borrowed("openhands"),
            agent_version: None,
            model: None,
            project: None,
            cwd: None,
            started_at: None,
            session_hash: crate::source::session_hash(b"content-presence-test"),
            events,
        }
    }

    fn event(
        kind: crate::source::SessionEventKind,
        content: Option<&str>,
        structured: serde_json::Value,
        tool_name: Option<&str>,
    ) -> crate::source::SessionEvent {
        crate::source::SessionEvent {
            kind,
            timestamp: None,
            content: content.map(str::to_string),
            structured,
            tool_name: tool_name.map(str::to_string),
            token_counts: None,
        }
    }

    #[test]
    fn meta_only_trace_declares_both_content_classes_absent() {
        let raw = build_raw_contribution(&transcript_with(vec![]), &test_config(), Utc::now());

        assert!(!raw.consent.message_text_included);
        assert!(!raw.consent.tool_payloads_included);
    }

    #[test]
    fn message_only_trace_declares_only_message_text_present() {
        let transcript = transcript_with(vec![event(
            crate::source::SessionEventKind::User,
            Some("hello"),
            serde_json::Value::Null,
            None,
        )]);
        let raw = build_raw_contribution(&transcript, &test_config(), Utc::now());

        assert!(raw.consent.message_text_included);
        assert!(!raw.consent.tool_payloads_included);
    }

    #[test]
    fn tool_only_trace_declares_only_tool_payloads_present() {
        let transcript = transcript_with(vec![event(
            crate::source::SessionEventKind::ToolCall,
            None,
            serde_json::json!({"path": "src/lib.rs"}),
            Some("read_file"),
        )]);
        let raw = build_raw_contribution(&transcript, &test_config(), Utc::now());

        assert!(!raw.consent.message_text_included);
        assert!(raw.consent.tool_payloads_included);
    }

    #[test]
    fn disabled_content_is_declared_absent_after_gating() {
        let transcript = transcript_with(vec![
            event(
                crate::source::SessionEventKind::Assistant,
                Some("message"),
                serde_json::Value::Null,
                None,
            ),
            event(
                crate::source::SessionEventKind::ToolResult,
                Some("tool output"),
                serde_json::json!({"result": "tool output"}),
                Some("private-tool-name"),
            ),
        ]);
        let mut cfg = test_config();
        cfg.include_message_text = false;
        cfg.include_tool_payloads = false;
        let raw = build_raw_contribution(&transcript, &cfg, Utc::now());

        assert!(!raw.consent.message_text_included);
        assert!(!raw.consent.tool_payloads_included);
    }

    #[tokio::test]
    async fn envelope_has_schema_version_and_no_local_paths_or_secrets() {
        let t = fixture_transcript();
        let cfg = test_config();
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        assert_eq!(
            raw.submission_id,
            crate::source::submission_id_for(&t.session_hash)
        );
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::new(vec![
                "/Users/testuser".into(),
            ])
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

    /// Spans back every occurrence of `bob@example.com` (used by
    /// `near_ai_filter_redacts_via_mock_endpoint`) and every canary email
    /// value the redaction canary sends through (used by the canary tests
    /// below), so one realistic classifier stub covers both. Matches how a
    /// real NEAR AI privacy-filter deployment would flag these categories --
    /// unlike a no-op filter, which always returns an empty span list.
    fn realistic_classify_router() -> axum::Router {
        use axum::{Json, Router, routing::post};
        Router::new().route(
            "/privacy/classify",
            post(|Json(req): Json<serde_json::Value>| async move {
                let input = req["input"].as_str().unwrap_or_default().to_string();
                let targets: &[(&str, &str)] = &[
                    ("bob@example.com", "private_email"),
                    ("trace-canary.person@example.invalid", "private_email"),
                    ("tc_canary_secret_0123456789abcdef", "secret"),
                    ("/tmp/trace_canary_private/path.txt", "private_url"),
                ];
                let mut spans = Vec::new();
                for (needle, category) in targets {
                    if let Some(start) = input.find(needle) {
                        spans.push(serde_json::json!({
                            "category": category,
                            "start": start,
                            "end": start + needle.len(),
                            "score": 0.99
                        }));
                    }
                }
                Json(serde_json::json!({"data": [{"spans": spans}]}))
            }),
        )
    }

    /// Always returns 200 with an empty span list, regardless of input --
    /// the shape a well-formed but non-functional ("no-op") privacy filter
    /// takes. It must not be able to pass the batch canary.
    fn noop_classify_router() -> axum::Router {
        use axum::{Json, Router, routing::post};
        Router::new().route(
            "/privacy/classify",
            post(|Json(_req): Json<serde_json::Value>| async move {
                Json(serde_json::json!({"data": [{"spans": []}]}))
            }),
        )
    }

    async fn spawn_near_ai_mock(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        base
    }

    fn near_ai_redactor(base_url: String) -> DeterministicTraceRedactor {
        let mut cfg = test_config();
        cfg.pii_filter = Some("near-ai".into());
        build_redactor_with(
            &cfg,
            Some("/Users/testuser/code/myproj"),
            Some(NearAiSettings {
                api_key: "test-key".into(),
                base_url: Some(base_url),
                model: None,
            }),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn near_ai_filter_redacts_via_mock_endpoint() {
        let base = spawn_near_ai_mock(realistic_classify_router()).await;

        let mut t = fixture_transcript();
        t.events.push(crate::source::SessionEvent {
            kind: crate::source::SessionEventKind::User,
            timestamp: None,
            content: Some("please email bob@example.com about this".into()),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
        });
        let cfg = test_config();
        let redactor = near_ai_redactor(base);
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        let envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(!json.contains("bob@example.com"));
    }

    #[tokio::test]
    async fn canary_self_test_async_passes_for_realistic_near_ai_filter() {
        let base = spawn_near_ai_mock(realistic_classify_router()).await;
        let redactor = near_ai_redactor(base);
        canary_self_test_async(&redactor).await.unwrap();
    }

    #[tokio::test]
    async fn canary_self_test_async_fails_closed_for_noop_near_ai_filter() {
        // A no-op filter (always empty spans) must not be able to pass the
        // batch canary just because the deterministic pass alone happens to
        // strip the values it is responsible for.
        let base = spawn_near_ai_mock(noop_classify_router()).await;
        let redactor = near_ai_redactor(base);
        let err = canary_self_test_async(&redactor).await.unwrap_err();
        assert!(err.to_string().contains("privacy-filter-canary-failed"));
    }

    #[tokio::test]
    async fn canary_self_test_async_is_noop_without_attached_filter() {
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        canary_self_test_async(&redactor).await.unwrap();
    }

    #[tokio::test]
    async fn granted_scopes_overwrite_consent_and_trace_card() {
        let t = fixture_transcript();
        let mut cfg = test_config();
        cfg.include_message_text = false;
        cfg.include_tool_payloads = false;
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        let mut envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        let scopes = vec![
            ConsentScope::DebuggingEvaluation,
            ConsentScope::ModelTraining,
        ];
        let uses = vec![
            trace_commons_protocol::trace_contribution::TraceAllowedUse::Debugging,
            trace_commons_protocol::trace_contribution::TraceAllowedUse::ModelTraining,
        ];
        apply_granted_scopes(&mut envelope, &scopes, &uses);
        assert_eq!(envelope.consent.scopes, scopes);
        assert_eq!(envelope.trace_card.allowed_uses, uses);
        assert_eq!(
            envelope.trace_card.consent_scope,
            ConsentScope::DebuggingEvaluation
        );
        assert!(!envelope.consent.message_text_included);
        assert!(!envelope.consent.tool_payloads_included);
    }

    #[tokio::test]
    async fn content_free_trace_has_low_residual_risk() {
        let raw = build_raw_contribution(&transcript_with(vec![]), &test_config(), Utc::now());
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        let envelope = redact_to_envelope(&redactor, raw).await.unwrap();

        assert!(envelope.privacy.redaction_counts.is_empty());
        assert_eq!(
            envelope.privacy.residual_pii_risk,
            trace_commons_protocol::trace_contribution::ResidualPiiRisk::Low
        );
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

    #[test]
    fn oversized_raw_is_refused_before_redaction() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events.push(crate::source::SessionEvent {
            kind: crate::source::SessionEventKind::Assistant,
            timestamp: None,
            content: Some("x".repeat(MAX_ENVELOPE_BYTES + 1)),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
        });
        let big = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        assert!(raw_contribution_size_ok(&big).is_err());

        let small = build_raw_contribution(&fixture_transcript(), &cfg, chrono::Utc::now());
        assert!(raw_contribution_size_ok(&small).is_ok());
    }

    #[test]
    fn reasoning_events_map_to_the_reasoning_event_type() {
        let event = crate::source::SessionEvent {
            kind: crate::source::SessionEventKind::Reasoning,
            timestamp: None,
            content: Some("weighing two approaches".to_string()),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
        };
        let raw = super::raw_event_for(&event, chrono::Utc::now(), true, true);
        assert_eq!(
            raw.event_type,
            trace_commons_protocol::trace_contribution::TraceContributionEventType::Reasoning
        );
        assert_eq!(raw.content.as_deref(), Some("weighing two approaches"));
    }

    #[test]
    fn trajectory_session_builds_an_envelope_with_reasoning_and_no_cwd_leak() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");
        let mut f = std::fs::File::create(&path).unwrap();
        // `cwd` is nested one level below the "secretproj" marker: the
        // project field derived from the cwd basename ("workdir") is
        // expected to survive into the raw contribution (same as every
        // other source), but the full cwd path -- and therefore the
        // "secretproj" marker itself -- must never appear in the
        // serialized output. Only its hash (`cwd_hash`) may.
        f.write_all(
            br#"[
              {"role":"meta","source":"openhands","cwd":"/home/dev/secretproj/workdir","model":"gpt-5"},
              {"role":"user","content":"fix the bug","timestamp":"2026-07-10T12:00:00Z"},
              {"role":"reasoning","content":"the guard clause is inverted","timestamp":"2026-07-10T12:00:01Z"},
              {"role":"assistant","content":"Fixed it.","timestamp":"2026-07-10T12:00:02Z"}
            ]"#,
        )
        .unwrap();

        let src = crate::source::trajectory::TrajectorySource::new(path);
        let r = &crate::source::TraceSource::discover(&src).unwrap()[0];
        let t = crate::source::TraceSource::load(&src, r).unwrap();

        assert_eq!(t.source.as_ref(), "openhands");

        let cfg = test_config();
        let raw = super::build_raw_contribution(&t, &cfg, chrono::Utc::now());
        assert!(
            raw.events.iter().any(|e| {
                e.event_type
                == trace_commons_protocol::trace_contribution::TraceContributionEventType::Reasoning
            }),
            "reasoning must survive into the raw contribution"
        );

        let serialized = serde_json::to_string(&raw).unwrap();
        assert!(
            !serialized.contains("secretproj"),
            "cwd must never be serialized"
        );
    }
}
