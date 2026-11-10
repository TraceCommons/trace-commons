//! End-to-end test: mint an enrollment grant, log in against the real
//! issuer's `/v1/enroll`, mint an upload claim through the real issuer's
//! device-signature verification, and submit a fixture session through the
//! full CLI submit pipeline to a stub ingest server.
//!
//! Only the ingest side (`/v1/traces`, `/v1/contributors/me/submission-
//! status`) is stubbed; the enrollment and claim-minting paths run through
//! the real `trace_upload_claim_issuer_router` from `trace-commons-server`.
//!
//! Non-Windows only: the server's OpenSSL dependency cannot link on the CI
//! Windows runner. This suite lives outside the contributor so ordinary
//! client tests never need to compile the server.
#![cfg(not(windows))]

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, Utc};
use sha2::Digest as _;
use trace_commons_contributor::source::TraceSource as _;
use uuid::Uuid;

use trace_commons_server::db::{
    Database, DeviceKeyRecord, InstanceEnrollmentOutcome, InstanceUserProvision,
};
use trace_commons_server::error::DatabaseError;
use trace_commons_server::trace_corpus_storage::*;

/// In-memory `Database` stub used only by the real issuer router in this
/// test. The issuer's `/v1/enroll` and `/v1/trace-upload-claim` handlers only
/// call `reserve_instance_enrollment`, `enroll_instance_user`,
/// `get_device_key`, and `bind_device_upload_principal`; every other
/// `TraceCorpusStore` / `Database` method is unreachable and panics via `todo!`
/// if hit.
struct InMemoryEnrollDb {
    device_keys: RwLock<HashMap<(String, String), DeviceKeyRecord>>,
    /// Grants captured at `enroll_instance_user` time, keyed by
    /// `(tenant_id, principal_ref)`, where `principal_ref` mirrors the real
    /// issuer's `principal_storage_ref(&format!("device:{tenant_id}:{device_key_id}"))`
    /// (see `device_key_claims_honor_grant_scope_ceiling` in the server
    /// crate, which pins this exact format). Value is
    /// `(allowed_consent_scopes, allowed_uses)`.
    grants: Mutex<HashMap<(String, String), (Vec<String>, Vec<String>)>>,
}

impl InMemoryEnrollDb {
    fn new() -> Self {
        Self {
            device_keys: RwLock::new(HashMap::new()),
            grants: Mutex::new(HashMap::new()),
        }
    }
}

/// `principal_sha256:<hex>` ref for a device principal, matching the private
/// `principal_storage_ref` helper in `trace_upload_claim_issuer.rs`.
fn device_grant_principal_ref(tenant_id: &str, device_key_id: &str) -> String {
    let raw = format!("device:{tenant_id}:{device_key_id}");
    format!(
        "principal_sha256:{}",
        hex::encode(sha2::Sha256::digest(raw.as_bytes()))
    )
}

#[async_trait::async_trait]
impl TraceCorpusStore for InMemoryEnrollDb {
    async fn list_active_trace_tenant_access_grants_for_principal(
        &self,
        tenant_id: &str,
        principal_ref: &str,
        _now: DateTime<Utc>,
    ) -> Result<Vec<TraceTenantAccessGrantRecord>, DatabaseError> {
        let grants = self.grants.lock().unwrap();
        let Some((allowed_consent_scopes, allowed_uses)) =
            grants.get(&(tenant_id.to_string(), principal_ref.to_string()))
        else {
            return Ok(Vec::new());
        };
        let now = Utc::now();
        Ok(vec![TraceTenantAccessGrantRecord {
            tenant_id: tenant_id.to_string(),
            grant_id: Uuid::new_v4(),
            principal_ref: principal_ref.to_string(),
            role: TraceTenantAccessGrantRole::Contributor,
            status: TraceTenantAccessGrantStatus::Active,
            allowed_consent_scopes: allowed_consent_scopes.clone(),
            allowed_uses: allowed_uses.clone(),
            issuer: None,
            audience: None,
            subject: None,
            issued_at: now - chrono::Duration::seconds(60),
            expires_at: None,
            revoked_at: None,
            created_by_principal_ref: None,
            revoked_by_principal_ref: None,
            reason: None,
            metadata: BTreeMap::new(),
            created_at: now,
            updated_at: now,
        }])
    }
}

#[async_trait::async_trait]
impl Database for InMemoryEnrollDb {
    async fn bind_device_upload_principal(
        &self,
        tenant_id: &str,
        device_key_id: &str,
        _actor_ref: &str,
    ) -> Result<(), DatabaseError> {
        self.get_device_key(tenant_id, device_key_id)
            .await?
            .filter(|key| key.revoked_at.is_none())
            .ok_or_else(|| DatabaseError::Pool("unknown or revoked fixture device".into()))?;
        Ok(())
    }

    async fn run_migrations(&self) -> Result<(), DatabaseError> {
        Ok(())
    }

    async fn enroll_instance_user(&self, p: InstanceUserProvision) -> Result<(), DatabaseError> {
        let allowed_consent_scopes: Vec<String> =
            serde_json::from_value(p.allowed_consent_scopes.clone()).unwrap_or_default();
        let allowed_uses: Vec<String> =
            serde_json::from_value(p.allowed_uses.clone()).unwrap_or_default();
        let principal_ref = device_grant_principal_ref(&p.tenant_id, &p.device_key_id);
        self.grants.lock().unwrap().insert(
            (p.tenant_id.clone(), principal_ref),
            (allowed_consent_scopes, allowed_uses),
        );
        self.device_keys.write().unwrap().insert(
            (p.tenant_id.clone(), p.device_key_id.clone()),
            DeviceKeyRecord {
                device_key_id: p.device_key_id,
                tenant_id: p.tenant_id,
                public_key: p.public_key,
                invite_subject_hash: Some(p.instance_subject_hash),
                client_info: p.client_info,
                created_at: Utc::now(),
                revoked_at: None,
            },
        );
        Ok(())
    }

    async fn reserve_instance_enrollment(
        &self,
        _instance_subject_hash: &str,
        _user_subject_hash: &str,
        _tenant_id: &str,
        _max_enrollments: i64,
    ) -> Result<InstanceEnrollmentOutcome, DatabaseError> {
        Ok(InstanceEnrollmentOutcome::NewlyEnrolled)
    }

    async fn instance_ledger_rls_ready(&self) -> Result<bool, DatabaseError> {
        Ok(false)
    }

    async fn get_device_key(
        &self,
        tenant_id: &str,
        device_key_id: &str,
    ) -> Result<Option<DeviceKeyRecord>, DatabaseError> {
        Ok(self
            .device_keys
            .read()
            .unwrap()
            .get(&(tenant_id.to_string(), device_key_id.to_string()))
            .cloned())
    }
}

#[tokio::test]
async fn enroll_mint_submit_round_trip() {
    // Instance keypair + allowlist file registering it.
    let instance_doc =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let instance_kp = ring::signature::Ed25519KeyPair::from_pkcs8(instance_doc.as_ref()).unwrap();
    use base64::Engine as _;
    use ring::signature::KeyPair as _;
    let instance_pk_b64 =
        base64::engine::general_purpose::STANDARD.encode(instance_kp.public_key().as_ref());

    let tmp = tempfile::tempdir().unwrap();
    let allowlist_path = tmp.path().join("allowlist.json");
    std::fs::write(
        &allowlist_path,
        serde_json::json!({
            "version": 1,
            "generated_at": chrono::Utc::now(),
            "policy_label": "e2e",
            "entries": [{
                "kind": "instance",
                "instance_id": "instance-e2e",
                "instance_public_key": instance_pk_b64,
                "max_enrollments": 10,
                "rate_per_min": 60,
                "policy_template": {
                    "policy_version": "e2e-v1",
                    "allowed_consent_scopes": ["debugging_evaluation","public_attribution","model_training"],
                    "allowed_uses": ["debugging","evaluation","aggregate_analytics","model_training"]
                }
            }]
        })
        .to_string(),
    )
    .unwrap();

    // Stub ingest first (its URL goes into the issuer config as onboarding_ingest_url).
    let received = Arc::new(std::sync::Mutex::new(Vec::<serde_json::Value>::new()));
    let ingest_url = {
        use axum::{Json, Router, routing::post};
        let sink = received.clone();
        let router = Router::new()
            .route(
                "/v1/traces",
                post(
                    move |headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
                        let sink = sink.clone();
                        async move {
                            // Real EdDSA claim minted by the real issuer rides in here.
                            let auth = headers.get("authorization").unwrap().to_str().unwrap();
                            assert!(auth.starts_with("Bearer "));
                            assert!(auth.len() > "Bearer ".len() + 20);
                            sink.lock().unwrap().push(body);
                            Json(serde_json::json!({
                                "status": "accepted",
                                "credit_points_pending": 0.0,
                                "explanation": []
                            }))
                        }
                    },
                ),
            )
            .route(
                "/v1/contributors/me/submission-status",
                post(|Json(req): Json<serde_json::Value>| async move {
                    let first = req["submission_ids"][0].clone();
                    Json(serde_json::json!([{
                        "submission_id": first,
                        "trace_id": "00000000-0000-0000-0000-000000000000",
                        "status": "accepted",
                        "credit_points_pending": 0.0,
                        "explanation": [],
                        "consent_scopes": ["debugging_evaluation","model_training"]
                    }]))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        url
    };

    // Real issuer router.
    let db = Arc::new(InMemoryEnrollDb::new());
    let keys =
        trace_commons_server::trace_upload_claim_issuer::generate_upload_claim_keypair().unwrap();
    let config = trace_commons_server::trace_upload_claim_issuer::TraceUploadClaimIssuerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        signing_private_key_pem: keys.private_key_pem.clone(),
        signing_public_key_pem: keys.public_key_pem.clone(),
        signing_kid: keys.suggested_kid.clone(),
        issuer: "trace-commons-upload-issuer".into(),
        audience: "trace-commons-upload".into(),
        max_ttl_seconds: 300,
        // The fail-closed defaults documented on the fields: None leaves
        // /v1/admin/invites* unmounted, and false keeps /v1/onboard on the
        // unchanged file-allowlist path. This test exercises the pre-existing
        // onboard/submit flow, so it must keep that behaviour rather than opt
        // into the DB-authoritative registry.
        invite_admin_backend: None,
        invite_admin_registry: None,
        invite_registry_authoritative: false,
        near_ai_signup_policy_template: None,
        workload_public_key_pem: keys.public_key_pem.clone(),
        workload_issuer: None,
        workload_audience: None,
        tenant_access_grant_db: Some(db.clone() as Arc<dyn Database>),
        require_tenant_access_grants: false,
        shutdown_grace_seconds: 30,
        request_timeout_seconds: 10,
        max_request_bytes: 64 * 1024,
        allowlist_source: Some(
            trace_commons_server::trace_upload_claim_allowlist::AllowlistSourceSpec::File(
                allowlist_path.clone(),
            ),
        ),
        allowlist_refresh_interval_seconds: 60,
        allowlist_max_stale_seconds: 3600,
        onboarding_device_key_db: Some(db.clone() as Arc<dyn Database>),
        onboarding_ingest_url: Some(ingest_url.clone()),
        onboarding_community_url: None,
        onboarding_profile_url: None,
        onboarding_leaderboard_url: None,
        admin_bind: None,
    };
    let router =
        trace_commons_server::trace_upload_claim_issuer::trace_upload_claim_issuer_router(config)
            .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    // CLI-side flow, all through lib functions.
    let store =
        trace_commons_contributor::config::ConfigStore::open(tmp.path().join("cfg")).unwrap();
    let device =
        trace_commons_contributor::identity::DeviceIdentity::load_or_generate(&store).unwrap();
    let grant = trace_commons_contributor::identity::mint_grant(
        instance_doc.as_ref(),
        &issuer_url,
        "instance-e2e",
        "alice@example.com",
        "trace-commons-upload",
        &device.device_key_id,
        300,
        chrono::Utc::now(),
    )
    .unwrap();
    trace_commons_contributor::commands::login(
        &store,
        Some(&grant.encode()),
        None,
        None,
        Some("debugging_evaluation,model_training"),
        false,
        false,
    )
    .await
    .unwrap();

    let cfg = store.load_config().unwrap().unwrap();
    assert_eq!(
        cfg.tenant_id,
        trace_commons_protocol::onboarding::derive_user_tenant_id(
            "instance-e2e",
            "alice@example.com"
        )
    );
    // The issuer normalizes a bare origin to the submit endpoint.
    assert_eq!(cfg.ingest_url, format!("{ingest_url}/v1/traces"));

    // Submit the Claude Code fixture through the real claim path.
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../trace-commons-contributor/fixtures/claude-code");
    let src = trace_commons_contributor::source::claude_code::ClaudeCodeSource::new(root.clone());
    let r = src.discover().unwrap().remove(0);
    let outcomes = trace_commons_contributor::submit::submit_sessions(
        &store,
        &cfg,
        vec![(
            Box::new(trace_commons_contributor::source::claude_code::ClaudeCodeSource::new(root))
                as _,
            r,
        )],
        &trace_commons_contributor::submit::SubmitOptions {
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        outcomes[0],
        trace_commons_contributor::submit::SubmitOutcome::Submitted { .. }
    ));
    assert_eq!(received.lock().unwrap().len(), 1);

    // The redaction pipeline must have stripped the fixture's fake secret
    // before it ever left the process.
    let sent = received.lock().unwrap()[0].to_string();
    assert!(!sent.contains("sk-fake-fixture-secret-1234"));

    // The training-consent scopes granted at enrollment must ride through
    // the real issuer's claim into the submitted envelope.
    let envelope = received.lock().unwrap()[0].clone();
    assert_eq!(
        envelope["consent"]["scopes"],
        serde_json::json!(["debugging_evaluation", "model_training"])
    );
    let allowed_uses = envelope["trace_card"]["allowed_uses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(allowed_uses.contains(&"model_training".to_string()));

    assert_eq!(store.load_receipts().unwrap().len(), 1);

    let status = trace_commons_contributor::submit::status(&store, &cfg)
        .await
        .unwrap();
    assert_eq!(status.len(), 1);
    assert_eq!(
        status[0].consent_scopes,
        vec![
            trace_commons_protocol::trace_contribution::ConsentScope::DebuggingEvaluation,
            trace_commons_protocol::trace_contribution::ConsentScope::ModelTraining,
        ]
    );
}
