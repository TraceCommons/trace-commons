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
    envelope_size_ok, near_ai_settings_from_env, parse_scope_names, parse_use_names,
    redact_to_envelope,
};
use crate::identity::{DeviceIdentity, build_signed_claim_request};
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
                    if e.to_string().contains("consent scopes not permitted") {
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
            .expect("a claim must be minted before applying granted scopes");
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
        apply_granted_scopes(&mut envelope, &granted_scopes, &granted_uses);

        if envelope_size_ok(&envelope).is_err() {
            outcomes.push(SubmitOutcome::Refused {
                reason_label: "session-too-large".to_string(),
            });
            continue;
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
    let issuer = IssuerClient::new(allowlist_for(cfg.allowed_hosts.as_deref()))
        .context("building issuer client")?;
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
}
