//! Submit pipeline: redact-and-upload sessions, then read back submission
//! status. Every outcome reason is a fixed label -- never a response body,
//! trace content, or raw path.

use std::time::Duration as StdDuration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Method;
use uuid::Uuid;

use trace_commons_operator_client::{Client, Error as OcError};
use trace_commons_protocol::trace_contribution::{
    TraceContributionEnvelope, TraceSubmissionReceipt, TraceSubmissionStatusRequest,
    TraceSubmissionStatusUpdate,
};

use crate::config::{ConfigStore, ContributorConfig, Receipt, allowlist_for};
use crate::envelope::{
    apply_granted_scopes, build_raw_contribution, build_redactor_with, canary_self_test_async,
    envelope_has_residual_secret, envelope_size_ok, near_ai_settings_from_env, parse_scope_names,
    parse_use_names, raw_contribution_size_ok, redact_to_envelope,
};
use crate::identity::{
    DeviceIdentity, build_signed_claim_request, build_signed_claim_request_with_scopes,
};
use crate::issuer_client::{ClaimToken, IssuerClient};
use crate::source::{SessionRef, TraceSource};

/// Statuses that mean a session has already been accepted by the server;
/// re-encountering a receipt with one of these statuses short-circuits the
/// per-session flow instead of re-uploading.
pub(crate) const ALREADY_SUBMITTED_STATUSES: [&str; 3] = ["submitted", "accepted", "quarantined"];

#[derive(Debug)]
pub enum SubmitOutcome {
    Submitted { submission_id: Uuid, status: String },
    AlreadySubmitted { submission_id: Uuid },
    SkippedParseFailure { reason_label: String },
    Refused { reason_label: String }, // canary hit, fail-closed PII filter, too large
    Failed { reason_label: String },  // network/auth after retries
}

pub struct SubmitOptions {
    pub dry_run: bool,
    pub pii_filter: Option<String>,
}

/// One entry in a `submit --manifest` file: an envelope id that reached the
/// server, for handing to an external collector (e.g. devfolio).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ManifestEntry {
    pub submission_id: Uuid,
    pub status: String,
}

/// Envelope ids that reached the server, for handing to an external
/// collector (e.g. devfolio). Includes freshly submitted and
/// already-submitted traces; skips refused/failed/skipped outcomes.
pub fn build_manifest(outcomes: &[SubmitOutcome]) -> Vec<ManifestEntry> {
    outcomes
        .iter()
        .filter_map(|o| match o {
            SubmitOutcome::Submitted {
                submission_id,
                status,
            } => Some(ManifestEntry {
                submission_id: *submission_id,
                status: status.clone(),
            }),
            SubmitOutcome::AlreadySubmitted { submission_id } => Some(ManifestEntry {
                submission_id: *submission_id,
                status: "already-submitted".to_string(),
            }),
            SubmitOutcome::SkippedParseFailure { .. }
            | SubmitOutcome::Refused { .. }
            | SubmitOutcome::Failed { .. } => None,
        })
        .collect()
}

/// Redact-and-upload every selected session. Sessions are independent: one
/// session's failure never aborts the batch. The one exception is the
/// once-per-batch privacy-filter canary self-test, which is a fail-closed
/// precondition for the whole batch.
pub async fn submit_sessions(
    store: &ConfigStore,
    cfg: &ContributorConfig,
    sessions: Vec<(Box<dyn TraceSource>, SessionRef)>,
    opts: &SubmitOptions,
) -> Result<Vec<SubmitOutcome>> {
    let mut outcomes = Vec::with_capacity(sessions.len());
    let effective_cfg = effective_config(cfg, opts);

    if effective_cfg.pii_filter.as_deref() == Some("near-ai")
        && store
            .ensure_near_ai_notice_shown()
            .context("recording NEAR AI first-use notice")?
    {
        println!(
            "notice: this will send redacted-but-unscrubbed message text to NEAR AI under your \
             API key (one-time notice; see `--pii-filter near-ai` in the README for scope)."
        );
    }

    let device = DeviceIdentity::load_or_generate(store).context("loading device identity")?;
    let issuer = IssuerClient::new(allowlist_for(cfg.allowed_hosts.as_deref()))
        .context("building issuer client")?;

    let mut claim: Option<ClaimToken> = None;
    let mut canary_checked = false;

    for (source, session_ref) in sessions {
        let transcript = match source.load(&session_ref) {
            Ok(t) => t,
            Err(_) => {
                outcomes.push(SubmitOutcome::SkippedParseFailure {
                    reason_label: "parse-failed".to_string(),
                });
                continue;
            }
        };

        let receipts = store.load_receipts().context("loading receipts")?;
        if receipts.iter().any(|r| {
            r.session_hash == transcript.session_hash
                && ALREADY_SUBMITTED_STATUSES.contains(&r.status.as_str())
        }) {
            outcomes.push(SubmitOutcome::AlreadySubmitted {
                submission_id: crate::source::submission_id_for(&transcript.session_hash),
            });
            continue;
        }

        let redactor = match build_redactor_with(
            &effective_cfg,
            transcript.cwd.as_deref(),
            near_ai_settings_from_env(),
        ) {
            Ok(r) => r,
            Err(_) => {
                outcomes.push(SubmitOutcome::Refused {
                    reason_label: "pii-filter-unavailable".to_string(),
                });
                continue;
            }
        };

        if !canary_checked {
            canary_self_test_async(&redactor)
                .await
                .context("privacy-filter-canary-failed")?;
            canary_checked = true;
        }

        let now = Utc::now();
        let raw = build_raw_contribution(&transcript, &effective_cfg, now);
        // Skip sessions that already exceed the envelope limit before the
        // expensive redaction/privacy-filter pass; they would be refused for
        // size after redaction anyway (envelope_size_ok below is the
        // authoritative check).
        if raw_contribution_size_ok(&raw).is_err() {
            outcomes.push(SubmitOutcome::Refused {
                reason_label: "session-too-large".to_string(),
            });
            continue;
        }
        let mut envelope = match redact_to_envelope(&redactor, raw).await {
            Ok(e) => e,
            Err(_) => {
                outcomes.push(SubmitOutcome::Refused {
                    reason_label: "redaction-failed".to_string(),
                });
                continue;
            }
        };

        let size = match envelope_size_ok(&envelope) {
            Ok(s) => s,
            Err(_) => {
                outcomes.push(SubmitOutcome::Refused {
                    reason_label: "session-too-large".to_string(),
                });
                continue;
            }
        };

        if opts.dry_run {
            if let Some(outcome) = residual_secret_refusal(&redactor, &envelope)? {
                outcomes.push(outcome);
                continue;
            }
            println!(
                "dry-run: submission_id={} bytes={size}",
                envelope.submission_id
            );
            outcomes.push(SubmitOutcome::Submitted {
                submission_id: envelope.submission_id,
                status: "dry-run".to_string(),
            });
            continue;
        }

        if !claim.as_ref().map(|c| c.is_fresh(now)).unwrap_or(false) {
            match mint_claim(&issuer, cfg, &device, now).await {
                Ok(token) => claim = Some(token),
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("consent scopes not permitted")
                        || msg.contains("allowed uses not permitted")
                    {
                        println!("hint: re-run login --scopes with a narrower selection");
                        outcomes.push(SubmitOutcome::Refused {
                            reason_label: "scopes-not-permitted".to_string(),
                        });
                    } else {
                        outcomes.push(SubmitOutcome::Failed {
                            reason_label: "claim-mint-failed".to_string(),
                        });
                    }
                    continue;
                }
            }
        }

        let token = claim
            .as_ref()
            .expect("a claim must be minted before applying granted scopes")
            .clone();
        stamp_granted_scopes(&mut envelope, &effective_cfg, &token);

        if let Some(outcome) = residual_secret_refusal(&redactor, &envelope)? {
            outcomes.push(outcome);
            continue;
        }

        if envelope_size_ok(&envelope).is_err() {
            outcomes.push(SubmitOutcome::Refused {
                reason_label: "session-too-large".to_string(),
            });
            continue;
        }

        match upload_with_retry(
            cfg,
            &issuer,
            &device,
            &mut claim,
            &mut envelope,
            &effective_cfg,
        )
        .await
        {
            Ok(receipt) => {
                let r = Receipt {
                    submission_id: envelope.submission_id,
                    session_hash: transcript.session_hash.clone(),
                    source: transcript.source.to_string(),
                    submitted_at: Utc::now(),
                    status: receipt.status.clone(),
                };
                store.append_receipt(&r).context("appending receipt")?;
                outcomes.push(SubmitOutcome::Submitted {
                    submission_id: envelope.submission_id,
                    status: receipt.status,
                });
            }
            Err(reason_label) => {
                outcomes.push(SubmitOutcome::Failed { reason_label });
            }
        }
    }

    Ok(outcomes)
}

/// Read back submission status for every locally recorded receipt. Returns
/// an empty vec (no network calls) when there are no receipts yet.
pub async fn status(
    store: &ConfigStore,
    cfg: &ContributorConfig,
) -> Result<Vec<TraceSubmissionStatusUpdate>> {
    let receipts = store.load_receipts().context("loading receipts")?;
    if receipts.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = receipts.iter().map(|r| r.submission_id).collect();

    let device = DeviceIdentity::load_or_generate(store).context("loading device identity")?;
    let issuer = IssuerClient::new(allowlist_for(cfg.allowed_hosts.as_deref()))
        .context("building issuer client")?;
    // Mint with an empty scopes/uses request rather than the submit path's
    // consent_scopes: the issuer resolves an empty request to the caller's
    // full grant ceiling, so status read-back works regardless of what
    // scopes were narrowed for submission since the last login.
    let token = mint_status_claim(&issuer, cfg, &device, Utc::now())
        .await
        .context("minting upload claim for status lookup")?;
    let client = build_ingest_client(cfg, &token).context("building ingest client")?;

    let mut updates = Vec::new();
    for chunk in ids.chunks(500) {
        let req = TraceSubmissionStatusRequest {
            submission_ids: chunk.to_vec(),
        };
        let mut chunk_updates: Vec<TraceSubmissionStatusUpdate> = client
            .call_json(
                Method::POST,
                "/v1/contributors/me/submission-status",
                &[],
                Some(&req),
            )
            .await
            .context("fetching submission status")?;
        updates.append(&mut chunk_updates);
    }
    Ok(updates)
}

/// Re-scan a finished envelope for a residual secret shape. Returns
/// `Ok(Some(Refused))` (emitting the same `refusing session` warn every
/// caller relies on) when the redactor's re-scan still finds a secret shape
/// in the serialized envelope, else `Ok(None)`. This is the single seam both
/// the dry-run and real submit paths route through, so deleting either call
/// site removes the fail-closed guard entirely -- callers must `continue` on
/// `Some(_)`.
fn residual_secret_refusal(
    redactor: &trace_commons_protocol::trace_contribution::DeterministicTraceRedactor,
    envelope: &TraceContributionEnvelope,
) -> Result<Option<SubmitOutcome>> {
    if envelope_has_residual_secret(redactor, envelope)? {
        tracing::warn!("refusing session: secret survived redaction");
        return Ok(Some(SubmitOutcome::Refused {
            reason_label: "secret-leak-detected".to_string(),
        }));
    }
    Ok(None)
}

/// `cfg` with `opts.pii_filter` overriding `cfg.pii_filter` when set.
fn effective_config(cfg: &ContributorConfig, opts: &SubmitOptions) -> ContributorConfig {
    let mut c = cfg.clone();
    if opts.pii_filter.is_some() {
        c.pii_filter = opts.pii_filter.clone();
    }
    c
}

async fn mint_claim(
    issuer: &IssuerClient,
    cfg: &ContributorConfig,
    device: &DeviceIdentity,
    now: DateTime<Utc>,
) -> Result<ClaimToken> {
    let signed =
        build_signed_claim_request(cfg, device, now).context("building signed claim request")?;
    issuer.mint_claim(&cfg.issuer_url, &signed).await
}

/// Mint a claim for a status read-back: an empty consent_scopes/allowed_uses
/// request, which the issuer resolves to the caller's full grant ceiling
/// regardless of what was requested for submission.
async fn mint_status_claim(
    issuer: &IssuerClient,
    cfg: &ContributorConfig,
    device: &DeviceIdentity,
    now: DateTime<Utc>,
) -> Result<ClaimToken> {
    let signed = build_signed_claim_request_with_scopes(cfg, device, now, &[], &[])
        .context("building signed status claim request")?;
    issuer.mint_claim(&cfg.issuer_url, &signed).await
}

/// Stamp `envelope` with the granted consent scopes/uses from `token`,
/// falling back to the requested (`effective_cfg`) scopes/uses when the
/// issuer is old enough not to echo them back (empty `consent_scopes`).
/// Shared between the initial stamp before the first upload attempt and the
/// restamp after a claim re-mint, so both paths derive the grant the same
/// way.
fn stamp_granted_scopes(
    envelope: &mut TraceContributionEnvelope,
    effective_cfg: &ContributorConfig,
    token: &ClaimToken,
) {
    let (granted_scopes, granted_uses) = if token.consent_scopes.is_empty() {
        (
            parse_scope_names(&effective_cfg.consent_scopes),
            parse_use_names(&crate::consent::scopes_to_allowed_uses(
                &effective_cfg.consent_scopes,
            )),
        )
    } else {
        (
            parse_scope_names(&token.consent_scopes),
            parse_use_names(&token.allowed_uses),
        )
    };
    apply_granted_scopes(envelope, &granted_scopes, &granted_uses);
}

fn build_ingest_client(
    cfg: &ContributorConfig,
    token: &ClaimToken,
) -> std::result::Result<Client, OcError> {
    Client::builder(
        &cfg.ingest_url,
        "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
    )
    .bearer_token(&token.access_token)
    .host_allowlist(allowlist_for(cfg.allowed_hosts.as_deref()))
    .build()
}

/// Upload `envelope`, retrying transient transport failures up to 3 attempts
/// total (1s then 4s backoff) and, on a 401/403, re-minting the claim once
/// and retrying once more before giving up.
///
/// A re-mint can return narrower (or otherwise different) granted scopes
/// than the claim that was active when `envelope` was first stamped. To
/// avoid resending an envelope stamped with a stale grant, the envelope is
/// restamped with the new token's granted scopes/uses (via
/// `stamp_granted_scopes`, the same helper used before the first attempt)
/// and re-checked for size before the retry.
async fn upload_with_retry(
    cfg: &ContributorConfig,
    issuer: &IssuerClient,
    device: &DeviceIdentity,
    claim: &mut Option<ClaimToken>,
    envelope: &mut TraceContributionEnvelope,
    effective_cfg: &ContributorConfig,
) -> std::result::Result<TraceSubmissionReceipt, String> {
    let mut transport_attempts: u32 = 0;
    let mut remint_attempted = false;

    loop {
        let token = claim
            .as_ref()
            .expect("a claim must be minted before uploading")
            .clone();
        let client = match build_ingest_client(cfg, &token) {
            Ok(c) => c,
            Err(e) => return Err(e.kind().to_string()),
        };

        let result = client
            .call_json::<TraceContributionEnvelope, TraceSubmissionReceipt>(
                Method::POST,
                "/v1/traces",
                &[],
                Some(&*envelope),
            )
            .await;

        match result {
            Ok(receipt) => return Ok(receipt),
            Err(OcError::Transport { .. }) => {
                transport_attempts += 1;
                if transport_attempts >= 3 {
                    return Err("transport".to_string());
                }
                let delay_secs = if transport_attempts == 1 { 1 } else { 4 };
                tokio::time::sleep(StdDuration::from_secs(delay_secs)).await;
            }
            Err(e) if is_auth_failure(&e) => {
                if remint_attempted {
                    return Err("auth-failed".to_string());
                }
                remint_attempted = true;
                match mint_claim(issuer, cfg, device, Utc::now()).await {
                    Ok(new_token) => {
                        stamp_granted_scopes(envelope, effective_cfg, &new_token);
                        if envelope_size_ok(envelope).is_err() {
                            return Err("session-too-large".to_string());
                        }
                        *claim = Some(new_token);
                    }
                    Err(_) => return Err("auth-failed".to_string()),
                }
            }
            Err(e) => return Err(e.kind().to_string()),
        }
    }
}

fn is_auth_failure(e: &OcError) -> bool {
    match e {
        OcError::ServerLabel { status, .. } | OcError::HttpFailure { status, .. } => {
            status.as_u16() == 401 || status.as_u16() == 403
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use std::sync::{Arc, Mutex};

    async fn spawn(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    /// Same as `spawn`, but returns a URL addressed via `localhost` instead
    /// of the literal `127.0.0.1`, so tests can put the issuer and ingest
    /// endpoints on distinct allowlist-checkable host strings while both
    /// still resolve to the same loopback listener.
    async fn spawn_as_localhost(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://localhost:{port}")
    }

    fn stub_issuer() -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(|| async {
                Json(serde_json::json!({
                    "access_token": "stub-claim-jwt",
                    "token_type": "Bearer",
                    "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                    "expires_in": 300,
                    "consent_scopes": ["debugging_evaluation", "model_training"],
                    "allowed_uses": ["debugging", "evaluation", "model_training", "aggregate_analytics"],
                }))
            }),
        )
    }

    fn stub_issuer_refuses_scopes() -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(|| async {
                (
                    axum::http::StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": "consent scopes not permitted"})),
                )
            }),
        )
    }

    fn stub_issuer_refuses_uses() -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(|| async {
                (
                    axum::http::StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": "allowed uses not permitted"})),
                )
            }),
        )
    }

    fn stub_ingest(received: Arc<Mutex<Vec<serde_json::Value>>>) -> Router {
        Router::new().route(
            "/v1/traces",
            post(
                move |headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
                    let received = received.clone();
                    async move {
                        assert_eq!(
                            headers.get("authorization").unwrap(),
                            "Bearer stub-claim-jwt"
                        );
                        received.lock().unwrap().push(body);
                        Json(serde_json::json!({
                            "status": "accepted",
                            "credit_points_pending": 0.0,
                            "explanation": []
                        }))
                    }
                },
            ),
        )
    }

    fn fixture_selection() -> Vec<(
        Box<dyn crate::source::TraceSource>,
        crate::source::SessionRef,
    )> {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
        let src = crate::source::claude_code::ClaudeCodeSource::new(root.clone());
        let r = src.discover().unwrap().remove(0);
        vec![(
            Box::new(crate::source::claude_code::ClaudeCodeSource::new(root))
                as Box<dyn crate::source::TraceSource>,
            r,
        )]
    }

    fn cfg_for(
        issuer: &str,
        ingest: &str,
        device_key_id: &str,
    ) -> crate::config::ContributorConfig {
        crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: issuer.into(),
            ingest_url: ingest.into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device_key_id.into(),
            consent_scopes: vec!["debugging_evaluation".into(), "model_training".into()],
            pii_filter: None,
            allowed_hosts: None,
        }
    }

    #[tokio::test]
    async fn submits_fixture_session_and_is_idempotent_on_rerun() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(outcomes[0], SubmitOutcome::Submitted { .. }));
        {
            // Scope the guard: `let sent = &received.lock().unwrap()[0]` would
            // extend the MutexGuard to the end of the test and self-deadlock
            // on the re-lock after the second run.
            let received_guard = received.lock().unwrap();
            assert_eq!(received_guard.len(), 1);
            let sent = &received_guard[0];
            assert_eq!(sent["schema_version"], "ironclaw.trace_contribution.v1");
            assert!(
                !serde_json::to_string(sent)
                    .unwrap()
                    .contains("sk-fake-fixture-secret-1234")
            );
            assert_eq!(
                sent["consent"]["scopes"],
                serde_json::json!(["debugging_evaluation", "model_training"])
            );
        }

        // Second run: receipt short-circuits, no second upload.
        let outcomes2 = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(
            outcomes2[0],
            SubmitOutcome::AlreadySubmitted { .. }
        ));
        assert_eq!(received.lock().unwrap().len(), 1);
    }

    /// The residual-secret guard is a re-scan of the finished envelope with
    /// the secret detector. A survivor (a detect-then-redact bug, or a
    /// non-string payload value the string-leaf pass never visited) leaves a
    /// recognizable secret shape in the serialized envelope and trips the
    /// guard; a clean envelope does not. This exercises the helper directly:
    /// forcing a real survivor through the (now-strong) redaction pipeline is
    /// impractical, so we plant a detector-recognized secret shape
    /// (`sk-ant-...`) into a finished envelope and assert the guard catches
    /// it, plus that an unmodified redacted envelope is clean. The full
    /// submit path's clean-session Submitted behavior is covered by
    /// `submits_fixture_session_and_is_idempotent_on_rerun` against the
    /// original fixture (whose Opaque record-type markers and normal prose
    /// are not secret-shaped and never trip the guard).
    #[tokio::test]
    async fn residual_secret_guard_flags_survivor_and_passes_clean_envelope() {
        use crate::envelope::{
            build_raw_contribution, envelope_has_residual_secret, redact_to_envelope,
        };
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
        let src = crate::source::claude_code::ClaudeCodeSource::new(root);
        let r = src.discover().unwrap().remove(0);
        let transcript = src.load(&r).unwrap();

        let cfg = cfg_for(
            "https://issuer.example",
            "https://ingest.example",
            "sha256:00",
        );
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        let raw = build_raw_contribution(&transcript, &cfg, Utc::now());
        let mut envelope = redact_to_envelope(&redactor, raw).await.unwrap();

        // A properly-redacted envelope has no residual secret shape.
        assert!(!envelope_has_residual_secret(&redactor, &envelope).unwrap());

        // Plant a detector-recognized secret shape into the finished
        // envelope, simulating a value that survived redaction. The re-scan
        // must catch it and the session must fail closed.
        if let Some(first) = envelope.events.first_mut() {
            first.redacted_content =
                Some("leftover sk-ant-EXPOSEDsecret0123456789abcdefghij here".to_string());
        }
        assert!(envelope_has_residual_secret(&redactor, &envelope).unwrap());
    }

    /// The `model` field (`IronclawTraceMetadata::model_name`) is copied
    /// verbatim from the transcript into the envelope and is never routed
    /// through the per-field redaction pass (only `content` and
    /// `structured_payload` are). The whole-envelope residual-secret rescan
    /// (`residual_secret_refusal`, called from both submit-path call sites)
    /// is the only thing standing between a secret-shaped literal placed
    /// there and delivery to ingest. This drives the *real* `submit_sessions`
    /// entrypoint end to end with a fixture whose `model` field is a
    /// recognized secret shape (`sk-ant-...`), so it fails if either call
    /// site is ever deleted: without the guard, this session would upload
    /// (`Submitted`, 1 delivery) instead of refusing.
    #[tokio::test]
    async fn submit_sessions_refuses_session_with_secret_in_unredacted_model_field() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
        };

        // A minimal transcript whose assistant message carries a
        // detector-recognized secret shape in `model`, a field the per-field
        // redaction pass never scans.
        let fixture_root = tempfile::tempdir().unwrap();
        let project_dir = fixture_root.path().join("-tmp-secret-model-proj");
        std::fs::create_dir_all(&project_dir).unwrap();
        let jsonl = concat!(
            r#"{"type":"user","message":{"role":"user","content":"hello"},"cwd":"/tmp/secret-model-proj","timestamp":"2026-07-01T10:00:00Z","version":"2.0.1","sessionId":"22222222-2222-2222-2222-222222222222","uuid":"a1"}"#,
            "\n",
            r#"{"type":"assistant","message":{"role":"assistant","model":"sk-ant-EXPOSEDsecret0123456789abcdefghij","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1}},"cwd":"/tmp/secret-model-proj","timestamp":"2026-07-01T10:00:05Z","version":"2.0.1","uuid":"a2"}"#,
            "\n",
        );
        std::fs::write(
            project_dir.join("22222222-2222-2222-2222-222222222222.jsonl"),
            jsonl,
        )
        .unwrap();

        let src =
            crate::source::claude_code::ClaudeCodeSource::new(fixture_root.path().to_path_buf());
        let session_ref = src.discover().unwrap().remove(0);
        let selection: Vec<(
            Box<dyn crate::source::TraceSource>,
            crate::source::SessionRef,
        )> = vec![(
            Box::new(crate::source::claude_code::ClaudeCodeSource::new(
                fixture_root.path().to_path_buf(),
            )) as Box<dyn crate::source::TraceSource>,
            session_ref,
        )];

        let outcomes = submit_sessions(&store, &cfg, selection, &opts)
            .await
            .unwrap();
        match &outcomes[0] {
            SubmitOutcome::Refused { reason_label } => {
                assert_eq!(reason_label, "secret-leak-detected");
            }
            other => panic!("expected Refused(secret-leak-detected), got {other:?}"),
        }
        assert_eq!(
            received.lock().unwrap().len(),
            0,
            "a session with a residual secret must never reach ingest"
        );
        assert!(store.load_receipts().unwrap().is_empty());
    }

    #[tokio::test]
    async fn dry_run_uploads_nothing_and_writes_no_receipt() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: true,
            pii_filter: None,
        };
        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(outcomes[0], SubmitOutcome::Submitted { .. }));
        assert_eq!(received.lock().unwrap().len(), 0);
        assert!(store.load_receipts().unwrap().is_empty());
    }

    #[tokio::test]
    async fn near_ai_batch_creates_first_use_notice_marker() {
        // No TRACE_NEAR_AI_PRIVACY_API_KEY is set in this process, so every
        // session will be refused as pii-filter-unavailable -- but the
        // once-per-batch first-use notice marker must still be created,
        // since it is unconditional on effective_cfg.pii_filter and does
        // not depend on the redactor actually building successfully.
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(Arc::new(Mutex::new(Vec::new())))).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        assert!(!store.dir().join("near-ai-notice-shown").exists());

        let opts = SubmitOptions {
            dry_run: true,
            pii_filter: Some("near-ai".to_string()),
        };
        submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(store.dir().join("near-ai-notice-shown").exists());
    }

    /// Grants strictly less than requested: config asks for
    /// debugging_evaluation + model_training, issuer grants only
    /// debugging_evaluation.
    fn stub_issuer_narrows_grant() -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(|| async {
                Json(serde_json::json!({
                    "access_token": "stub-claim-jwt",
                    "token_type": "Bearer",
                    "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                    "expires_in": 300,
                    "consent_scopes": ["debugging_evaluation"],
                    "allowed_uses": ["debugging", "evaluation", "aggregate_analytics"],
                }))
            }),
        )
    }

    #[tokio::test]
    async fn envelope_is_stamped_with_narrowed_grant_when_server_grants_less() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer_narrows_grant()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        // cfg_for requests debugging_evaluation + model_training; the stub
        // issuer grants only debugging_evaluation. The envelope must carry
        // the granted (narrower) set, never the requested one.
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(outcomes[0], SubmitOutcome::Submitted { .. }));
        let received_guard = received.lock().unwrap();
        assert_eq!(received_guard.len(), 1);
        let sent = &received_guard[0];
        assert_eq!(
            sent["consent"]["scopes"],
            serde_json::json!(["debugging_evaluation"])
        );
        let allowed_uses = sent["trace_card"]["allowed_uses"].as_array().unwrap();
        assert!(
            !allowed_uses
                .iter()
                .any(|u| u == &serde_json::json!("model_training"))
        );
    }

    /// An issuer that predates the consent_scopes/allowed_uses echo: the
    /// claim response omits both fields entirely.
    fn stub_issuer_omits_scope_echo() -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(|| async {
                Json(serde_json::json!({
                    "access_token": "stub-claim-jwt",
                    "token_type": "Bearer",
                    "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                    "expires_in": 300,
                }))
            }),
        )
    }

    #[tokio::test]
    async fn envelope_is_stamped_with_requested_scopes_when_issuer_omits_echo() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer_omits_scope_echo()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        // cfg_for requests debugging_evaluation + model_training; the stub
        // issuer's claim response has no consent_scopes/allowed_uses fields
        // at all, so the fallback must stamp the requested set verbatim.
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(outcomes[0], SubmitOutcome::Submitted { .. }));
        let received_guard = received.lock().unwrap();
        assert_eq!(received_guard.len(), 1);
        let sent = &received_guard[0];
        assert_eq!(
            sent["consent"]["scopes"],
            serde_json::json!(["debugging_evaluation", "model_training"])
        );
    }

    #[tokio::test]
    async fn scope_refusal_from_issuer_yields_refused_outcome_with_no_deliveries() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer_refuses_scopes()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        match &outcomes[0] {
            SubmitOutcome::Refused { reason_label } => {
                assert_eq!(reason_label, "scopes-not-permitted");
            }
            other => panic!("expected Refused(scopes-not-permitted), got {other:?}"),
        }
        assert_eq!(received.lock().unwrap().len(), 0);
        assert!(store.load_receipts().unwrap().is_empty());
    }

    #[tokio::test]
    async fn uses_refusal_from_issuer_yields_refused_outcome_with_no_deliveries() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer_refuses_uses()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        match &outcomes[0] {
            SubmitOutcome::Refused { reason_label } => {
                assert_eq!(reason_label, "scopes-not-permitted");
            }
            other => panic!("expected Refused(scopes-not-permitted), got {other:?}"),
        }
        assert_eq!(received.lock().unwrap().len(), 0);
        assert!(store.load_receipts().unwrap().is_empty());
    }

    /// Mints `["debugging_evaluation", "model_training"]` on the first call
    /// and the narrower `["debugging_evaluation"]` on every call after —
    /// simulating a grant narrowed between the first and second mint.
    fn stub_issuer_narrows_on_remint(mint_calls: Arc<std::sync::atomic::AtomicUsize>) -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(move || {
                let mint_calls = mint_calls.clone();
                async move {
                    let n = mint_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if n == 0 {
                        Json(serde_json::json!({
                            "access_token": "stub-claim-jwt",
                            "token_type": "Bearer",
                            "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                            "expires_in": 300,
                            "consent_scopes": ["debugging_evaluation", "model_training"],
                            "allowed_uses": ["debugging", "evaluation", "model_training", "aggregate_analytics"],
                        }))
                    } else {
                        Json(serde_json::json!({
                            "access_token": "stub-claim-jwt",
                            "token_type": "Bearer",
                            "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                            "expires_in": 300,
                            "consent_scopes": ["debugging_evaluation"],
                            "allowed_uses": ["debugging", "evaluation", "aggregate_analytics"],
                        }))
                    }
                }
            }),
        )
    }

    /// Refuses the first POST with 401 (forcing a claim re-mint + retry) and
    /// accepts every POST after, recording every received body so the test
    /// can inspect what the *retried* request actually carried.
    fn stub_ingest_401_then_200(
        received: Arc<Mutex<Vec<serde_json::Value>>>,
        post_calls: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Router {
        use axum::response::IntoResponse;
        Router::new().route(
            "/v1/traces",
            post(move |Json(body): Json<serde_json::Value>| {
                let received = received.clone();
                let post_calls = post_calls.clone();
                async move {
                    let n = post_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    received.lock().unwrap().push(body);
                    if n == 0 {
                        axum::http::StatusCode::UNAUTHORIZED.into_response()
                    } else {
                        Json(serde_json::json!({
                            "status": "accepted",
                            "credit_points_pending": 0.0,
                            "explanation": []
                        }))
                        .into_response()
                    }
                }
            }),
        )
    }

    #[tokio::test]
    async fn envelope_is_restamped_after_claim_remint_on_auth_failure() {
        use std::sync::atomic::AtomicUsize;

        let mint_calls = Arc::new(AtomicUsize::new(0));
        let post_calls = Arc::new(AtomicUsize::new(0));
        let received = Arc::new(Mutex::new(Vec::new()));

        let issuer = spawn(stub_issuer_narrows_on_remint(mint_calls.clone())).await;
        let ingest = spawn(stub_ingest_401_then_200(
            received.clone(),
            post_calls.clone(),
        ))
        .await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(
            matches!(outcomes[0], SubmitOutcome::Submitted { .. }),
            "expected Submitted after remint+retry, got {:?}",
            outcomes[0]
        );
        assert_eq!(mint_calls.load(std::sync::atomic::Ordering::SeqCst), 2);

        let received_guard = received.lock().unwrap();
        assert_eq!(
            received_guard.len(),
            2,
            "the 401 attempt and the successful retry must both reach ingest"
        );
        // The envelope actually delivered on the second (200) POST must carry
        // the NEW token's narrower grant, not the original wider one it was
        // first stamped with.
        let restamped = &received_guard[1];
        assert_eq!(
            restamped["consent"]["scopes"],
            serde_json::json!(["debugging_evaluation"]),
            "retried envelope must be restamped with the re-minted (narrower) scopes: {restamped}"
        );
        let allowed_uses = restamped["trace_card"]["allowed_uses"].as_array().unwrap();
        assert!(
            !allowed_uses
                .iter()
                .any(|u| u == &serde_json::json!("model_training")),
            "retried envelope must not retain model_training from the stale claim: {restamped}"
        );
    }

    /// Records every claim-request body it receives (as raw JSON) before
    /// responding with a fixed claim, so tests can inspect what scopes/uses
    /// were actually requested.
    fn stub_issuer_recording_requests(received: Arc<Mutex<Vec<serde_json::Value>>>) -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(move |body: String| {
                let received = received.clone();
                async move {
                    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
                    received.lock().unwrap().push(parsed);
                    Json(serde_json::json!({
                        "access_token": "stub-claim-jwt",
                        "token_type": "Bearer",
                        "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                        "expires_in": 300,
                        "consent_scopes": ["debugging_evaluation", "model_training"],
                        "allowed_uses": ["debugging", "evaluation", "model_training", "aggregate_analytics"],
                    }))
                }
            }),
        )
    }

    fn stub_submission_status_ingest() -> Router {
        Router::new().route(
            "/v1/contributors/me/submission-status",
            post(|Json(req): Json<serde_json::Value>| async move {
                let ids = req["submission_ids"].as_array().unwrap();
                let updates: Vec<serde_json::Value> = ids
                    .iter()
                    .map(|id| {
                        serde_json::json!({
                            "submission_id": id,
                            "trace_id": id,
                            "status": "accepted",
                            "credit_points_pending": 0.0,
                        })
                    })
                    .collect();
                Json(updates)
            }),
        )
    }

    #[tokio::test]
    async fn status_mints_claim_with_empty_scopes_and_uses() {
        let claim_requests = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer_recording_requests(claim_requests.clone())).await;
        let ingest = spawn(stub_submission_status_ingest()).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);

        // Seed a receipt so status() actually mints a claim and calls out.
        store
            .append_receipt(&crate::config::Receipt {
                submission_id: Uuid::new_v4(),
                session_hash: "sha256:test".to_string(),
                source: "claude-code".to_string(),
                submitted_at: Utc::now(),
                status: "submitted".to_string(),
            })
            .unwrap();

        let updates = status(&store, &cfg).await.unwrap();
        assert_eq!(updates.len(), 1);

        let requests = claim_requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let req = &requests[0];
        assert_eq!(
            req["consent_scopes"],
            serde_json::json!([]),
            "status claim request must not request the submit-path's scopes: {req}"
        );
        assert_eq!(
            req["allowed_uses"],
            serde_json::json!([]),
            "status claim request must not request the submit-path's uses: {req}"
        );
    }

    #[tokio::test]
    async fn upload_refuses_ingest_host_off_allowlist_before_any_request() {
        let received = Arc::new(Mutex::new(Vec::new()));
        // Issuer stays on the literal `127.0.0.1` host (allowed); ingest is
        // addressed via `localhost` (not on the allowlist), so the claim
        // mints fine but the ingest client must refuse to even build.
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn_as_localhost(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let mut cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        cfg.allowed_hosts = Some("127.0.0.1".to_string());
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        match &outcomes[0] {
            SubmitOutcome::Failed { reason_label } => {
                assert_eq!(reason_label, "host-not-allowed");
            }
            other => panic!("expected Failed(host-not-allowed), got {other:?}"),
        }
        assert_eq!(received.lock().unwrap().len(), 0);
        assert!(store.load_receipts().unwrap().is_empty());
    }

    #[test]
    fn build_manifest_includes_only_delivered_ids() {
        let u1 = Uuid::new_v4();
        let u2 = Uuid::new_v4();
        let outcomes = vec![
            SubmitOutcome::Submitted {
                submission_id: u1,
                status: "submitted".to_string(),
            },
            SubmitOutcome::AlreadySubmitted { submission_id: u2 },
            SubmitOutcome::Refused {
                reason_label: "secret-leak-detected".to_string(),
            },
            SubmitOutcome::Failed {
                reason_label: "claim-mint-failed".to_string(),
            },
            SubmitOutcome::SkippedParseFailure {
                reason_label: "parse-failed".to_string(),
            },
        ];

        let manifest = build_manifest(&outcomes);

        assert_eq!(manifest.len(), 2);
        assert_eq!(manifest[0].submission_id, u1);
        assert_eq!(manifest[0].status, "submitted");
        assert_eq!(manifest[1].submission_id, u2);
        assert_eq!(manifest[1].status, "already-submitted");
    }

    #[tokio::test]
    async fn submit_sessions_outcomes_round_trip_through_manifest_file() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(outcomes[0], SubmitOutcome::Submitted { .. }));

        let entries = build_manifest(&outcomes);
        let manifest_path = tempfile::NamedTempFile::new().unwrap();
        let json = serde_json::to_string_pretty(&entries).unwrap();
        std::fs::write(manifest_path.path(), json).unwrap();

        let read_back = std::fs::read_to_string(manifest_path.path()).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&read_back).unwrap();
        assert_eq!(parsed.len(), 1);
        let SubmitOutcome::Submitted { submission_id, .. } = &outcomes[0] else {
            unreachable!()
        };
        assert_eq!(
            parsed[0]["submission_id"],
            serde_json::Value::String(submission_id.to_string())
        );
        assert_eq!(parsed[0]["status"], "accepted");
    }
}
