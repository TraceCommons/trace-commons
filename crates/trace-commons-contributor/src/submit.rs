//! Submit pipeline: redact-and-upload sessions, then read back submission
//! status. Every outcome reason is a fixed label -- never a response body,
//! trace content, or raw path.

use std::time::Duration as StdDuration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Method;
use uuid::Uuid;

use trace_commons_operator_client::{host_allowlist::HostAllowlist, Client, Error as OcError};
use trace_commons_protocol::trace_contribution::{
    TraceContributionEnvelope, TraceSubmissionReceipt, TraceSubmissionStatusRequest,
    TraceSubmissionStatusUpdate,
};

use crate::config::{ConfigStore, ContributorConfig, Receipt};
use crate::envelope::{
    build_raw_contribution, build_redactor_with, canary_self_test, envelope_size_ok,
    near_ai_settings_from_env, redact_to_envelope,
};
use crate::identity::{build_signed_claim_request, DeviceIdentity};
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

    let device = DeviceIdentity::load_or_generate(store).context("loading device identity")?;
    let issuer = IssuerClient::new(allowlist_for(cfg)).context("building issuer client")?;

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
            canary_self_test(&redactor).context("privacy-filter-canary-failed")?;
            canary_checked = true;
        }

        let now = Utc::now();
        let raw = build_raw_contribution(&transcript, &effective_cfg, now);
        let envelope = match redact_to_envelope(&redactor, raw).await {
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
                Err(_) => {
                    outcomes.push(SubmitOutcome::Failed {
                        reason_label: "claim-mint-failed".to_string(),
                    });
                    continue;
                }
            }
        }

        match upload_with_retry(cfg, &issuer, &device, &mut claim, &envelope).await {
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
    let issuer = IssuerClient::new(allowlist_for(cfg)).context("building issuer client")?;
    let token = mint_claim(&issuer, cfg, &device, Utc::now())
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

/// `cfg` with `opts.pii_filter` overriding `cfg.pii_filter` when set.
fn effective_config(cfg: &ContributorConfig, opts: &SubmitOptions) -> ContributorConfig {
    let mut c = cfg.clone();
    if opts.pii_filter.is_some() {
        c.pii_filter = opts.pii_filter.clone();
    }
    c
}

fn allowlist_for(cfg: &ContributorConfig) -> HostAllowlist {
    match cfg.allowed_hosts.as_deref() {
        Some(csv) => HostAllowlist::from_csv(csv),
        None => HostAllowlist::from_env(),
    }
}

async fn mint_claim(
    issuer: &IssuerClient,
    cfg: &ContributorConfig,
    device: &DeviceIdentity,
    now: DateTime<Utc>,
) -> Result<ClaimToken> {
    let signed = build_signed_claim_request(cfg, device, now)
        .context("building signed claim request")?;
    issuer.mint_claim(&cfg.issuer_url, &signed).await
}

fn build_ingest_client(
    cfg: &ContributorConfig,
    token: &ClaimToken,
) -> std::result::Result<Client, OcError> {
    let mut builder =
        Client::builder(&cfg.ingest_url, "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV")
            .bearer_token(&token.access_token);
    if let Some(csv) = cfg.allowed_hosts.as_deref() {
        builder = builder.host_allowlist(HostAllowlist::from_csv(csv));
    }
    builder.build()
}

/// Upload `envelope`, retrying transient transport failures up to 3 attempts
/// total (1s then 4s backoff) and, on a 401/403, re-minting the claim once
/// and retrying once more before giving up.
async fn upload_with_retry(
    cfg: &ContributorConfig,
    issuer: &IssuerClient,
    device: &DeviceIdentity,
    claim: &mut Option<ClaimToken>,
    envelope: &TraceContributionEnvelope,
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
                Some(envelope),
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
                    Ok(new_token) => *claim = Some(new_token),
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
    use axum::{routing::post, Json, Router};
    use std::sync::{Arc, Mutex};

    async fn spawn(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
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
                }))
            }),
        )
    }

    fn stub_ingest(received: Arc<Mutex<Vec<serde_json::Value>>>) -> Router {
        Router::new().route(
            "/v1/traces",
            post(move |headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
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
            }),
        )
    }

    fn fixture_selection() -> Vec<(Box<dyn crate::source::TraceSource>, crate::source::SessionRef)> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
        let src = crate::source::claude_code::ClaudeCodeSource::new(root.clone());
        let r = src.discover().unwrap().remove(0);
        vec![(
            Box::new(crate::source::claude_code::ClaudeCodeSource::new(root)) as Box<dyn crate::source::TraceSource>,
            r,
        )]
    }

    fn cfg_for(issuer: &str, ingest: &str, device_key_id: &str) -> crate::config::ContributorConfig {
        crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: issuer.into(),
            ingest_url: ingest.into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device_key_id.into(),
            consent_scopes: vec!["debugging_evaluation".into()],
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
        let opts = SubmitOptions { dry_run: false, pii_filter: None };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts).await.unwrap();
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
        }

        // Second run: receipt short-circuits, no second upload.
        let outcomes2 = submit_sessions(&store, &cfg, fixture_selection(), &opts).await.unwrap();
        assert!(matches!(outcomes2[0], SubmitOutcome::AlreadySubmitted { .. }));
        assert_eq!(received.lock().unwrap().len(), 1);
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
        let opts = SubmitOptions { dry_run: true, pii_filter: None };
        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts).await.unwrap();
        assert!(matches!(outcomes[0], SubmitOutcome::Submitted { .. }));
        assert_eq!(received.lock().unwrap().len(), 0);
        assert!(store.load_receipts().unwrap().is_empty());
    }
}
